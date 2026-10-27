use super::*;
use pz_render::{LuaTexture, Texture};

struct Restore {
    pool: VecDeque<SpriteInstanceRef>,
    loader: Option<pz_render::TextureRefLoader>,
}
impl Restore {
    fn new() -> Self {
        Self {
            pool: INSTANCE_POOL.with(|pool| std::mem::take(&mut *pool.borrow_mut())),
            loader: pz_render::install_texture_ref_loader(None),
        }
    }
}
impl Drop for Restore {
    fn drop(&mut self) {
        let pool = INSTANCE_POOL.with(|pool| {
            std::mem::replace(&mut *pool.borrow_mut(), std::mem::take(&mut self.pool))
        });
        let loader = pz_render::install_texture_ref_loader(self.loader.take());
        drop(pool);
        drop(loader);
    }
}
fn manager() -> SpriteManagerRef {
    Rc::new(RefCell::new(IsoSpriteManager::new(
        super::super::test_tables(),
    )))
}

#[test]
fn unmanaged_create_retains_actual_manager_def_and_distinct_same_id_sprites() {
    let _restore = Restore::new();
    let manager = manager();
    let before = (
        manager.borrow().sprites.len(),
        manager.borrow().named_map.len(),
        manager.borrow().int_map.len(),
    );
    let mut retired = RetiredComponents::new();
    let manager_guard = manager.borrow_mut();
    let first =
        create_runtime_sprite(&manager, &super::super::test_tables(), &mut retired).unwrap();
    let second =
        create_runtime_sprite(&manager, &super::super::test_tables(), &mut retired).unwrap();
    drop(manager_guard);
    assert!(!Rc::ptr_eq(&first, &second));
    assert_eq!(
        (first.borrow().data.id, second.borrow().data.id),
        (20_000_000, 20_000_000)
    );
    assert_eq!(
        before,
        (
            manager.borrow().sprites.len(),
            manager.borrow().named_map.len(),
            manager.borrow().int_map.len()
        )
    );
    assert!(Rc::ptr_eq(
        first.borrow().parent_manager_slot().unwrap(),
        &manager
    ));
    let def = first.borrow().def_slot().unwrap().clone();
    assert!(Rc::ptr_eq(
        def.borrow().parent_sprite_slot().unwrap(),
        &first
    ));
    assert_eq!(
        (
            def.borrow().scale_x,
            def.borrow().scale_y,
            def.borrow().frame
        ),
        (1.0, 1.0, 0.0)
    );
    assert!(first.borrow().texture_slot().is_none());
    assert!(first.borrow().data.properties.is_empty());
}

#[test]
fn actual_instance_pool_is_fifo_reset_retains_parent_and_full_pool_still_resets() {
    let _restore = Restore::new();
    let mut retired = RetiredComponents::new();
    let sprite =
        create_runtime_sprite(&manager(), &super::super::test_tables(), &mut retired).unwrap();
    let a = get_instance(&sprite, &mut retired).unwrap();
    let b = get_instance(&sprite, &mut retired).unwrap();
    a.borrow_mut().scale_x = 7.0;
    add_instance(Some(&a), &mut retired).unwrap();
    add_instance(Some(&b), &mut retired).unwrap();
    assert_eq!(a.borrow().scale_x, 1.0);
    assert!(Rc::ptr_eq(
        a.borrow().parent_sprite_slot().unwrap(),
        &sprite
    ));
    assert!(Rc::ptr_eq(
        &get_instance(&sprite, &mut retired).unwrap(),
        &a
    ));
    assert!(Rc::ptr_eq(
        &get_instance(&sprite, &mut retired).unwrap(),
        &b
    ));
    for _ in 0..2000 {
        let value = Rc::new(RefCell::new(RuntimeSpriteInstance::new()));
        add_instance(Some(&value), &mut retired).unwrap();
    }
    a.borrow_mut().scale_y = 12.0;
    add_instance(Some(&a), &mut retired).unwrap();
    assert_eq!(a.borrow().scale_y, 1.0);
    assert_eq!(INSTANCE_POOL.with(|pool| pool.borrow().len()), 2000);
    let held = b.borrow_mut();
    assert!(matches!(
        add_instance(Some(&b), &mut retired),
        Err(RuntimeHostError::Borrowed("sprite instance reset"))
    ));
    assert_eq!(INSTANCE_POOL.with(|pool| pool.borrow().len()), 2000);
    drop(held);
}

#[test]
fn load_single_stores_actual_nullable_texture_without_replacing_def_or_tint() {
    let _restore = Restore::new();
    let mut retired = RetiredComponents::new();
    let sprite =
        create_runtime_sprite(&manager(), &super::super::test_tables(), &mut retired).unwrap();
    let def = sprite.borrow().def_slot().unwrap().clone();
    let tint = sprite.borrow().tint_slot().unwrap().clone();
    let texture = Rc::new(LuaTexture::new(Texture::empty("actual")));
    let supplied = texture.clone();
    pz_render::install_texture_ref_loader(Some(Rc::new(move |_, name| {
        Ok((name == "actual").then(|| supplied.clone()))
    })));
    let held_texture = texture.0.borrow_mut();
    let returned = load_single_texture(&sprite, "actual", &mut retired)
        .unwrap()
        .unwrap();
    assert!(Rc::ptr_eq(&returned, &texture));
    assert!(Rc::ptr_eq(
        sprite.borrow().texture_slot().unwrap(),
        &texture
    ));
    assert!(Rc::ptr_eq(sprite.borrow().def_slot().unwrap(), &def));
    assert!(Rc::ptr_eq(sprite.borrow().tint_slot().unwrap(), &tint));
    drop(held_texture);
    assert!(load_single_texture(&sprite, "missing", &mut retired)
        .unwrap()
        .is_none());
    assert!(sprite.borrow().has_no_textures());
    assert!(Rc::ptr_eq(sprite.borrow().def_slot().unwrap(), &def));
}

#[test]
fn archive_sprite_bodies_are_independent_while_actual_assets_stay_shared() {
    let _restore = Restore::new();
    let mut retired = RetiredComponents::new();
    let manager = manager();
    let source =
        create_runtime_sprite(&manager, &super::super::test_tables(), &mut retired).unwrap();
    let copy = archive_copy_sprite(&source).unwrap();
    let first_def = source.borrow().def_slot().unwrap().clone();
    let second_def = copy.borrow().def_slot().unwrap().clone();
    assert!(!Rc::ptr_eq(&first_def, &second_def));
    assert!(Rc::ptr_eq(
        second_def.borrow().parent_sprite_slot().unwrap(),
        &copy
    ));
    assert!(Rc::ptr_eq(
        copy.borrow().parent_manager_slot().unwrap(),
        &manager
    ));
    set_scale(&copy, 2.0, 3.0).unwrap();
    assert_eq!(
        (first_def.borrow().scale_x, first_def.borrow().scale_y),
        (1.0, 1.0)
    );
    let held = source.borrow_mut();
    assert!(matches!(
        archive_copy_sprite(&source),
        Err(RuntimeHostError::Borrowed("archive sprite"))
    ));
    drop(held);
}

#[test]
fn scale_aspect_preserves_native_input_gates_and_multiply_divide_order() {
    let mut instance = RuntimeSpriteInstance::new();
    instance.scale_aspect(80.0, 40.0, 16.0, 16.0);
    assert_eq!((instance.scale_x, instance.scale_y), (0.2, 0.2));
    instance.scale_aspect(f32::NAN, 40.0, 16.0, 16.0);
    assert_eq!((instance.scale_x, instance.scale_y), (0.2, 0.2));
    instance.dispose();
    assert_eq!((instance.scale_x, instance.scale_y), (0.2, 0.2));
}

#[test]
fn archive_copy_rejects_cross_sprite_parent_after_real_pool_reuse_instead_of_rebinding_it() {
    let _restore = Restore::new();
    let manager = manager();
    let tables = super::super::test_tables();
    let mut retired = RetiredComponents::new();
    let first = create_runtime_sprite(&manager, &tables, &mut retired).unwrap();
    let def = first.borrow().def_slot().unwrap().clone();
    add_instance(Some(&def), &mut retired).unwrap();
    let second = create_runtime_sprite(&manager, &tables, &mut retired).unwrap();
    assert!(Rc::ptr_eq(second.borrow().def_slot().unwrap(), &def));
    assert!(Rc::ptr_eq(
        def.borrow().parent_sprite_slot().unwrap(),
        &second
    ));
    assert!(matches!(
        archive_copy_sprite(&first),
        Err(RuntimeHostError::Unsupported {
            operation: "archive copy of reused pooled sprite instance parent graph",
            ..
        })
    ));
    assert!(Rc::ptr_eq(
        def.borrow().parent_sprite_slot().unwrap(),
        &second
    ));
}

#[test]
fn native_metadata_queries_read_the_same_mutable_sprite_and_held_body_is_not_absence() {
    let _restore = Restore::new();
    let manager = manager();
    let tables = super::super::test_tables();
    let mut retired = RetiredComponents::new();
    let sprite = create_runtime_sprite(&manager, &tables, &mut retired).unwrap();
    sprite
        .borrow_mut()
        .data
        .properties
        .set_flag(tables.flag.solidfloor);
    sprite
        .borrow_mut()
        .data
        .properties
        .set_flag(tables.flag.WindowN);
    let mut object = crate::IsoObject::new(1, crate::ObjectKind::Object, None);
    object.sprite = ObjectSprite::Native(sprite.clone());
    let slot = crate::ObjectSlot::new(object);
    let actual = slot.ref_handle();
    let mut square = crate::IsoGridSquare::new(1, 2, 0);
    square.objects.push(slot);
    assert_eq!(square.try_floor_index(&manager.borrow()).unwrap(), Some(0));
    assert!(actual.borrow().is_window(&manager.borrow()));
    sprite
        .borrow_mut()
        .data
        .properties
        .unset_flag(tables.flag.solidfloor);
    sprite
        .borrow_mut()
        .data
        .properties
        .unset_flag(tables.flag.WindowN);
    assert_eq!(square.try_floor_index(&manager.borrow()).unwrap(), None);
    assert!(!actual.borrow().is_window(&manager.borrow()));
    let _held = sprite.borrow_mut();
    assert!(matches!(
        square.try_floor_index(&manager.borrow()),
        Err(RuntimeHostError::Borrowed("actual object sprite"))
    ));
}

#[test]
fn native_thumpable_nonnull_properties_gate_reads_ishigh_instead_of_arena_membership() {
    let _restore = Restore::new();
    let manager = manager();
    let tables = super::super::test_tables();
    let mut values = super::super::PropertyValueMap::default();
    values.put("IsHigh", vec!["true".into()]);
    manager.borrow_mut().alias = super::super::TilePropertyAliasMap::generate(&values, &[]);
    let mut retired = RetiredComponents::new();
    let sprite = create_runtime_sprite(&manager, &tables, &mut retired).unwrap();
    sprite
        .borrow_mut()
        .data
        .properties
        .set_raw(&manager.borrow().alias, "IsHigh", "true");
    let mut thumpable = crate::objects::IsoThumpable::default();
    thumpable.north = true;
    thumpable.is_thumpable = true;
    thumpable.can_pass_through = true;
    let mut object =
        crate::IsoObject::new(8, crate::ObjectKind::Thumpable(Box::new(thumpable)), None);
    object.sprite = ObjectSprite::Native(sprite.clone());
    object.container = Some(crate::objects::ItemContainer::new(Some("fixture")));
    let slot = crate::ObjectSlot::new(object);
    let actual = slot.ref_handle();
    let mut square = crate::IsoGridSquare::new(1, 2, 0);
    square.special_objects.push(8);
    square.objects.push(slot);
    let cell = crate::IsoCell::default();
    let meta = crate::IsoMetaGrid::empty();
    let manager_guard = manager.borrow();
    let world = crate::square::CollideEnv::new(&cell, &manager_guard, &meta);
    assert!(actual.borrow().sprite.tile_projection().is_none());
    assert_eq!(
        world.get_special_wall(&square, true),
        None,
        "actual nonnull PropertyContainer IsHigh skips pass-through container"
    );
    sprite
        .borrow_mut()
        .data
        .properties
        .unset(&manager_guard.alias, "IsHigh");
    assert_eq!(world.get_special_wall(&square, true), Some(8));
    assert!(Rc::ptr_eq(
        actual.borrow().sprite.native_slot().unwrap(),
        &sprite
    ));
}

#[test]
fn native_stack_surface_queries_use_current_actual_metadata_without_arena_membership() {
    let _restore = Restore::new();
    let manager = manager();
    let tables = super::super::test_tables();
    let mut values = super::super::PropertyValueMap::default();
    values.put("Surface", vec!["20".into(), "40".into()]);
    values.put("IsTable", vec!["".into()]);
    values.put("IsTableTop", vec!["".into()]);
    manager.borrow_mut().alias = super::super::TilePropertyAliasMap::generate(&values, &[]);
    let mut retired = RetiredComponents::new();
    let table = create_runtime_sprite(&manager, &tables, &mut retired).unwrap();
    let top = create_runtime_sprite(&manager, &tables, &mut retired).unwrap();
    {
        let manager = manager.borrow();
        table
            .borrow_mut()
            .data
            .properties
            .set_raw(&manager.alias, "IsTable", "");
        table
            .borrow_mut()
            .data
            .properties
            .set_raw(&manager.alias, "Surface", "20");
        top.borrow_mut()
            .data
            .properties
            .set_raw(&manager.alias, "IsTableTop", "");
    }
    let mut square = crate::IsoGridSquare::new(1, 2, 0);
    for (id, sprite) in [(1, &table), (2, &top)] {
        let mut object = crate::IsoObject::new(id, crate::ObjectKind::Object, None);
        object.sprite = ObjectSprite::Native(sprite.clone());
        square.objects.push(crate::ObjectSlot::new(object));
    }
    square.fix_stackable_objects(&manager.borrow());
    assert_eq!(square.objects[1].borrow().render_y_offset, 20.0);
    table
        .borrow_mut()
        .data
        .properties
        .set_raw(&manager.borrow().alias, "Surface", "40");
    square.fix_stackable_objects(&manager.borrow());
    assert_eq!(square.objects[1].borrow().render_y_offset, 40.0);
    assert!(Rc::ptr_eq(
        square.objects[0].borrow().sprite.native_slot().unwrap(),
        &table
    ));
    assert!(Rc::ptr_eq(
        square.objects[1].borrow().sprite.native_slot().unwrap(),
        &top
    ));
}
