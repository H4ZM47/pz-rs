//! Authored actual-allocation fixtures; no installed assets or model result is fabricated.
use super::*;
use pz_entity::RetiredComponents;
use pz_world::sprite::runtime::{
    create_runtime_sprite, ObjectSprite, RuntimeSpriteRef, SpriteManagerRef,
};
use pz_world_render::RuntimeHostError;
use std::{cell::RefCell, rc::Rc};

fn native(manager: &SpriteManagerRef, retired: &mut RetiredComponents) -> RuntimeSpriteRef {
    let tables = manager.borrow().tables.clone();
    create_runtime_sprite(manager, &tables, retired).unwrap()
}
fn object(id: u32, kind: ObjectKind, sprite: &RuntimeSpriteRef) -> IsoObject {
    let mut object = IsoObject::new(id, kind, None);
    object.sprite = ObjectSprite::Native(sprite.clone());
    object
}
fn view<'a>(
    cell: &'a IsoCell,
    meta: &'a IsoMetaGrid,
    manager: &'a IsoSpriteManager,
) -> WorldView<'a> {
    WorldView {
        cell,
        meta,
        sprites: manager,
    }
}

#[test]
fn live_curtain_properties_are_read_from_one_actual_sprite_and_borrow_faults_survive() {
    let manager = Rc::new(RefCell::new(sprites()));
    let mut retired = RetiredComponents::new();
    let sprite = native(&manager, &mut retired);
    let mut square = IsoGridSquare::new(0, 0, 0);
    square.objects.push(pz_world::ObjectSlot::new(object(
        17,
        ObjectKind::Curtain(Default::default()),
        &sprite,
    )));
    square.special_objects.push(17);
    let cell = IsoCell::default();
    let meta = IsoMetaGrid::empty();
    let m = manager.borrow();
    let w = view(&cell, &meta, &m);
    sprite.borrow_mut().data.tile_type = m.tables.ty.curtainN;
    let first = grid_square(&w, &square, &NoWorldHooks, 0, 0).unwrap();
    assert!(
        matches!(&first.special_objects[..],[SpecialObject::Curtain(c)] if c.kind==pz_lighting::jni::CurtainType::N)
    );
    sprite.borrow_mut().data.tile_type = m.tables.ty.curtainW;
    let next = grid_square(&w, &square, &NoWorldHooks, 0, 0).unwrap();
    assert!(
        matches!(&next.special_objects[..],[SpecialObject::Curtain(c)] if c.kind==pz_lighting::jni::CurtainType::W)
    );
    let held = sprite.borrow_mut();
    assert!(matches!(
        grid_square(&w, &square, &NoWorldHooks, 0, 0),
        Err(RuntimeHostError::Borrowed("actual object sprite"))
    ));
    drop(held);
    assert!(Rc::ptr_eq(
        square.objects[0].borrow().sprite.native_slot().unwrap(),
        &sprite
    ));
}

#[test]
fn window_lighting_does_not_read_an_unneeded_sprite_body() {
    let manager = Rc::new(RefCell::new(sprites()));
    let mut retired = RetiredComponents::new();
    let sprite = native(&manager, &mut retired);
    let mut square = IsoGridSquare::new(0, 0, 0);
    square.objects.push(pz_world::ObjectSlot::new(object(
        9,
        ObjectKind::Window(Default::default()),
        &sprite,
    )));
    square.special_objects.push(9);
    let cell = IsoCell::default();
    let meta = IsoMetaGrid::empty();
    let m = manager.borrow();
    let w = view(&cell, &meta, &m);
    let held = sprite.borrow_mut();
    assert!(matches!(
        &grid_square(&w, &square, &NoWorldHooks, 0, 0)
            .unwrap()
            .special_objects[..],
        [SpecialObject::Window(_)]
    ));
    drop(held);
    let held_object = square.objects[0].borrow_mut();
    assert!(matches!(
        grid_square(&w, &square, &NoWorldHooks, 0, 0),
        Err(RuntimeHostError::Borrowed(_))
    ));
    drop(held_object);
}

#[test]
fn native_and_archive_unmanaged_sprites_are_never_plain_cached_tile_draws() {
    let manager = Rc::new(RefCell::new(sprites()));
    let mut retired = RetiredComponents::new();
    let sprite = native(&manager, &mut retired);
    let mut square = IsoGridSquare::new(0, 0, 0);
    square.objects.push(pz_world::ObjectSlot::new(object(
        3,
        ObjectKind::Object,
        &sprite,
    )));
    let cell = IsoCell::default();
    let meta = IsoMetaGrid::empty();
    let m = manager.borrow();
    let w = view(&cell, &meta, &m);
    let mut cache = SpriteCache::new();
    assert!(matches!(
        pz_world_render::render_square(&w, &square, &mut cache, &mut NoTextures),
        Err(RuntimeHostError::Unsupported {
            operation: "native runtime sprite render host",
            ..
        })
    ));
    assert!(Rc::ptr_eq(
        square.objects[0].borrow().sprite.native_slot().unwrap(),
        &sprite
    ));
    square.objects[0].borrow_mut().sprite = ObjectSprite::ArchiveUnmanaged(20_000_000);
    assert!(matches!(
        pz_world_render::render_square(&w, &square, &mut cache, &mut NoTextures),
        Err(RuntimeHostError::Unsupported {
            operation: "archive unmanaged sprite render body",
            ..
        })
    ));
    square.objects[0].borrow_mut().sprite = ObjectSprite::None;
    assert!(
        pz_world_render::render_square(&w, &square, &mut cache, &mut NoTextures)
            .unwrap()
            .objects[0]
            .sprite
            .is_none()
    );
}

#[test]
fn native_world_render_false_returns_before_held_sprite_and_model_host() {
    let manager = Rc::new(RefCell::new(sprites()));
    let mut retired = RetiredComponents::new();
    let sprite = native(&manager, &mut retired);
    use pz_scripts::ScriptObject;
    let sm = pz_scripts::ScriptManager::new();
    sm.enable_texture_capture();
    let mut definition = pz_scripts::objects::items::Item::new();
    definition.load(&sm, "Hidden", "item Hidden {} ").unwrap();
    assert!(definition.texture_capture_complete());
    definition.world_render = false;
    let script = Rc::new(RefCell::new(definition));
    let mut template =
        pz_inventory::ItemTemplate::new(pz_inventory::ItemClass::Normal, "Base", "Hidden");
    template.script = Some(pz_inventory::template::ScriptItemRef(script.clone()));
    struct Env;
    impl pz_inventory::ItemEnv for Env {}
    let item = pz_inventory::env::create_from_template_ref(&Env, Rc::new(template), 1).unwrap();
    assert!(Rc::ptr_eq(
        &pz_inventory::item_texture::get_script_item_ref(&item)
            .unwrap()
            .unwrap()
            .0,
        &script
    ));
    let payload = pz_world::objects::IsoWorldInventoryObject {
        xoff: 0.,
        yoff: 0.,
        zoff: 0.,
        offset_x: 0.,
        offset_y: 0.,
        item: pz_world::floor_item::FloorItem::retain(&item),
        drop_time: 0.,
        ignore_remove_sandbox: false,
        extended_placement: false,
    };
    let mut square = IsoGridSquare::new(0, 0, 0);
    square.objects.push(pz_world::ObjectSlot::new(object(
        4,
        ObjectKind::WorldInventory(pz_world::floor_object::FloorObject::new(payload)),
        &sprite,
    )));
    let cell = IsoCell::default();
    let meta = IsoMetaGrid::empty();
    let m = manager.borrow();
    let w = view(&cell, &meta, &m);
    let mut cache = SpriteCache::new();
    let held = sprite.borrow_mut();
    let render = pz_world_render::render_square(&w, &square, &mut cache, &mut NoTextures).unwrap();
    assert_eq!(
        render.objects[0].kind,
        pz_iso_render::ObjectKind::WorldInventory
    );
    assert!(render.objects[0].sprite.is_none());
    assert!(render.objects[0].attached.is_empty());
    script.borrow_mut().world_render = true;
    assert!(matches!(
        pz_world_render::render_square(&w, &square, &mut cache, &mut NoTextures),
        Err(RuntimeHostError::Unsupported {
            operation: "native world-item render/model host",
            ..
        })
    ));
    for replacement in [
        ObjectSprite::None,
        ObjectSprite::Tile(m.named_map["floor"]),
        ObjectSprite::ArchiveUnmanaged(20_000_000),
    ] {
        square.objects[0].borrow_mut().sprite = replacement;
        assert!(matches!(
            pz_world_render::render_square(&w, &square, &mut cache, &mut NoTextures),
            Err(RuntimeHostError::Unsupported {
                operation: "native world-item render/model host",
                ..
            })
        ));
        script.borrow_mut().world_render = false;
        let hidden =
            pz_world_render::render_square(&w, &square, &mut cache, &mut NoTextures).unwrap();
        assert!(hidden.objects[0].sprite.is_none() && hidden.objects[0].attached.is_empty());
        script.borrow_mut().world_render = true;
    }
    square.objects[0].borrow_mut().sprite = ObjectSprite::Native(sprite.clone());
    drop(held);
    let held_script = script.borrow_mut();
    assert!(matches!(
        pz_world_render::render_square(&w, &square, &mut cache, &mut NoTextures),
        Err(RuntimeHostError::Borrowed("world render item script"))
    ));
    drop(held_script);
    // A completed real string constructor has a true null instance script,
    // even though this receiver's Rust blueprint still references false.
    script.borrow_mut().world_render = false;
    let previous = pz_render::install_texture_ref_loader(Some(Rc::new(|_, _| Ok(None))));
    let result = pz_inventory::item_texture::constructor_string_ref(&item, "Item_NullScript");
    let displaced = pz_render::install_texture_ref_loader(previous);
    drop(displaced);
    result.unwrap();
    assert!(item.borrow().template.script.is_some());
    assert!(pz_inventory::item_texture::get_script_item_ref(&item)
        .unwrap()
        .is_none());
    assert!(matches!(
        pz_world_render::render_square(&w, &square, &mut cache, &mut NoTextures),
        Err(RuntimeHostError::Unsupported {
            operation: "native world render null script diagnostic",
            ..
        })
    ));
}

#[test]
fn hearth_reads_current_actual_sprite_and_preserves_fuel_prefix_on_later_fault() {
    let manager = Rc::new(RefCell::new(sprites()));
    let mut retired = RetiredComponents::new();
    let sprite = native(&manager, &mut retired);
    sprite.borrow_mut().data.name = Some("first".into());
    let fp = pz_world::objects::IsoFireplace {
        fuel_amount_minutes: 100,
        lit: true,
        last_update_time: 0.,
        ..Default::default()
    };
    let mut obj = object(7, ObjectKind::Fireplace(fp.clone()), &sprite);
    let m = manager.borrow();
    let first = pz_world_render::fireplace_input(&obj, &fp, &m).unwrap();
    sprite.borrow_mut().data.name = Some("next".into());
    assert_eq!(first.sprite_name, "first");
    assert_eq!(
        pz_world_render::fireplace_input(&obj, &fp, &m)
            .unwrap()
            .sprite_name,
        "next"
    );
    let held = sprite.borrow_mut();
    let mut link = pz_world_render::HearthLink::new();
    let mut lighting = WorldLighting::new();
    let rand = pz_core::random::Rand::new(1);
    let result = link.update_fireplace(
        &mut lighting,
        &mut obj,
        &m,
        (0, 0, 0),
        1.,
        0.,
        &rand,
        &|_| None,
    );
    assert!(matches!(
        result,
        Err(RuntimeHostError::Borrowed("actual object sprite"))
    ));
    let ObjectKind::Fireplace(fp) = &obj.kind else {
        panic!()
    };
    assert_eq!(fp.fuel_amount_minutes, 40);
    assert_eq!(fp.last_update_time, 1.);
    drop(held);
}

#[test]
fn live_seam_floor_and_picking_read_faults_are_not_false_absence() {
    let manager = Rc::new(RefCell::new(sprites()));
    let mut retired = RetiredComponents::new();
    let sprite = native(&manager, &mut retired);
    let mut cell = IsoCell::default();
    cell.chunks.insert((0, 0), IsoChunk::new(0, 0));
    let mut south = IsoGridSquare::new(0, 1, 0);
    south.objects.push(pz_world::ObjectSlot::new(object(
        8,
        ObjectKind::Object,
        &sprite,
    )));
    cell.chunks
        .get_mut(&(0, 0))
        .unwrap()
        .set_square(0, 1, 0, Some(south.into()));
    let meta = IsoMetaGrid::empty();
    let m = manager.borrow();
    let w = view(&cell, &meta, &m);
    let square = IsoGridSquare::new(0, 0, 0);
    let held = sprite.borrow_mut();
    assert!(matches!(
        seam_neighbours(&w, &square),
        Err(RuntimeHostError::Borrowed("actual object sprite"))
    ));
    let render = RenderWorld::new();
    let infos = pz_iso_render::RenderInfos::new();
    let pick = pz_world_render::WorldPick {
        view: w,
        render: &render,
        infos: &infos,
        player: 0,
        camera: pz_iso_render::Camera::centred(0., 0., 0., 100, 100, 1.),
    };
    assert!(matches!(
        pick.objects(0, 1, 0),
        Err(RuntimeHostError::Borrowed("actual object sprite"))
    ));
    drop(held);
    assert!(matches!(
        pick.objects(0, 1, 0),
        Err(RuntimeHostError::Unsupported {
            operation: "native sprite picking/mask host",
            ..
        })
    ));
    assert!(pick.objects(7, 7, 0).unwrap().is_none());
}

#[test]
fn barbecue_fuel_prefix_precedes_held_actual_sprite_query() {
    let manager = Rc::new(RefCell::new(sprites()));
    let mut retired = RetiredComponents::new();
    let sprite = native(&manager, &mut retired);
    let barbecue = pz_world::objects::IsoBarbecue {
        fuel_amount_minutes: 100,
        lit: true,
        last_update_time: 0.,
        ..Default::default()
    };
    let mut receiver = object(16, ObjectKind::Barbecue(barbecue), &sprite);
    let m = manager.borrow();
    let held = sprite.borrow_mut();
    let mut link = pz_world_render::HearthLink::new();
    assert!(matches!(
        link.update_barbecue(&mut receiver, &m, 1., 0.),
        Err(RuntimeHostError::Borrowed("actual object sprite"))
    ));
    let ObjectKind::Barbecue(barbecue) = &receiver.kind else {
        panic!()
    };
    assert_eq!(barbecue.fuel_amount_minutes, 40);
    assert_eq!(barbecue.last_update_time, 1.);
    assert!(Rc::ptr_eq(receiver.sprite.native_slot().unwrap(), &sprite));
    drop(held);
    // A non-barbecue never requests this class's sprite properties.
    let mut other = object(19, ObjectKind::Object, &sprite);
    let held = sprite.borrow_mut();
    assert!(link.update_barbecue(&mut other, &m, 2., 0.).is_ok());
    drop(held);
}
