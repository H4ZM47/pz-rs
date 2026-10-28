use super::*;
use pz_world::sprite::runtime as sprites;

struct Restore {
    square: Option<pz_world::square_slot::SquareObserver>,
    sprite: Option<sprites::SpriteObserver>,
    instance: Option<sprites::SpriteInstanceObserver>,
    object: Option<pz_world::object_slot::ObjectObserver>,
    texture: Option<pz_render::TextureObserver>,
    loader: Option<pz_render::TextureRefLoader>,
}
impl Restore {
    fn new() -> Self {
        pz_inventory::component_host::drain_retired_components();
        let _ = gc::collect_tables();
        Self {
            square: pz_world::square_slot::install_square_observer(Some(observe_square)),
            sprite: sprites::install_sprite_observer(Some(observe_sprite)),
            instance: sprites::install_instance_observer(Some(observe_sprite_instance)),
            object: pz_world::object_slot::install_object_observer(Some(observe_object)),
            texture: pz_render::install_texture_observer(Some(observe_texture)),
            loader: pz_render::install_texture_ref_loader(None),
        }
    }
}
impl Drop for Restore {
    fn drop(&mut self) {
        pz_world::square_slot::install_square_observer(self.square);
        sprites::install_sprite_observer(self.sprite);
        sprites::install_instance_observer(self.instance);
        pz_world::object_slot::install_object_observer(self.object);
        pz_render::install_texture_observer(self.texture);
        drop(pz_render::install_texture_ref_loader(self.loader.take()));
        pz_inventory::component_host::drain_retired_components();
        retry_pending();
        let _ = gc::collect_tables();
    }
}

#[test]
fn actual_def_roots_actual_sprite_but_texture_alone_does_not_and_collection_never_resets() {
    let _restore = Restore::new();
    let tables = pz_world::sprite::test_tables();
    let manager = Rc::new(RefCell::new(pz_world::sprite::IsoSpriteManager::new(
        tables.clone(),
    )));
    let mut retired = RetiredComponents::new();
    let sprite = sprites::create_runtime_sprite(&manager, &tables, &mut retired).unwrap();
    let def = sprite.borrow().def_slot().unwrap().clone();
    sprites::set_scale(&sprite, 2.5, 3.5).unwrap();
    let texture = Rc::new(pz_render::LuaTexture::new(pz_render::Texture::empty(
        "actual",
    )));
    let captured = texture.clone();
    pz_render::install_texture_ref_loader(Some(Rc::new(move |_, _| Ok(Some(captured.clone())))));
    sprites::load_single_texture(&sprite, "actual", &mut retired).unwrap();
    let sprite_weak = Rc::downgrade(&sprite);
    let def_weak = Rc::downgrade(&def);
    drop(retired);
    drop(sprite);
    let _ = gc::collect_tables();
    assert!(sprite_weak.upgrade().is_some());
    assert_eq!((def.borrow().scale_x, def.borrow().scale_y), (2.5, 3.5));
    assert!(Rc::ptr_eq(
        def.borrow().parent_sprite_slot().unwrap(),
        &sprite_weak.upgrade().unwrap()
    ));
    drop(def);
    let _ = gc::collect_tables();
    assert!(sprite_weak.upgrade().is_none());
    assert!(def_weak.upgrade().is_none());
    assert_eq!(texture.0.borrow().name.as_ref(), "actual");
}

#[test]
fn real_square_main_and_world_lists_and_object_backlink_collect_without_native_removal() {
    let _restore = Restore::new();
    let square = pz_world::SquareSlot::new(IsoGridSquare::new(1, 2, 0)).ref_handle();
    let slot = pz_world::ObjectSlot::new(IsoObject::new(1, ObjectKind::Object, None));
    let object = slot.ref_handle();
    let mut retired = RetiredComponents::new();
    object
        .borrow_mut()
        .replace_square(Some(&square), &mut retired);
    object.borrow_mut().added_to_world_or_equipped = true;
    square.borrow_mut().objects.push(slot);
    square
        .borrow()
        .world_objects
        .borrow_mut()
        .push_object(&object);
    observe_object(&object, "zombie.iso.IsoObject");
    drop(retired);
    let square_weak = Rc::downgrade(&square);
    let object_weak = Rc::downgrade(&object);
    drop(square);
    let _ = gc::collect_tables();
    assert!(square_weak.upgrade().is_some());
    assert!(
        object.borrow().added_to_world_or_equipped,
        "collector must not run removeFromWorld/reset"
    );
    let retained_square = object.borrow().square_slot().unwrap().clone();
    assert!(Rc::ptr_eq(
        retained_square.borrow().objects[0].owner_slot(),
        &object
    ));
    assert_eq!(retained_square.borrow().world_objects.borrow().len(), 1);
    drop(retained_square);
    drop(object);
    let _ = gc::collect_tables();
    assert!(square_weak.upgrade().is_none());
    assert!(object_weak.upgrade().is_none());
}

#[test]
fn held_sprite_body_defers_collection_then_exact_cycle_collects_after_release() {
    let _restore = Restore::new();
    let tables = pz_world::sprite::test_tables();
    let manager = Rc::new(RefCell::new(pz_world::sprite::IsoSpriteManager::new(
        tables.clone(),
    )));
    let mut retired = RetiredComponents::new();
    let sprite = sprites::create_runtime_sprite(&manager, &tables, &mut retired).unwrap();
    let weak = Rc::downgrade(&sprite);
    drop(retired);
    {
        let _held = sprite.borrow_mut();
        assert_eq!(
            register_owner_graph(OwnerNode::Sprite(sprite.clone())).unwrap(),
            MetadataStatus::Deferred
        );
        assert_eq!(gc::collect_tables(), gc::CollectionResult::Deferred);
        assert!(weak.upgrade().is_some());
    }
    retry_pending();
    assert_eq!(
        register_owner_graph(OwnerNode::Sprite(sprite.clone())).unwrap(),
        MetadataStatus::Complete
    );
    drop(sprite);
    let _ = gc::collect_tables();
    assert!(weak.upgrade().is_none());
}
