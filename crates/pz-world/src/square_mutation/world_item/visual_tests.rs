use super::*;
use pz_render::{LuaTexture, Texture, TextureRef, TextureRefLoader};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

struct Restore(Option<TextureRefLoader>);
impl Restore {
    fn new() -> Self {
        Self(pz_render::install_texture_ref_loader(None))
    }
}
impl Drop for Restore {
    fn drop(&mut self) {
        drop(pz_render::install_texture_ref_loader(self.0.take()));
        pz_inventory::component_host::drain_retired_components();
    }
}
struct Env;
impl pz_inventory::ItemEnv for Env {}

fn actual_item(texture: &TextureRef) -> ItemRef {
    use pz_scripts::ScriptObject;
    let captured = texture.clone();
    drop(pz_render::install_texture_ref_loader(Some(Rc::new(
        move |_, _| Ok(Some(captured.clone())),
    ))));
    let scripts = pz_scripts::ScriptManager::new();
    scripts
        .parse_script(pz_scripts::ScriptLoadMode::Init, "module Base {} ")
        .unwrap();
    scripts.enable_texture_capture();
    let module = scripts.get_module("Base").unwrap();
    let mut definition = pz_scripts::objects::items::Item::new();
    definition.set_module(&module);
    definition.init_load_pp(&scripts, "WorldVisual");
    definition
        .load(
            &scripts,
            "WorldVisual",
            "item WorldVisual { Icon = WorldVisual, }",
        )
        .unwrap();
    definition.use_world_item = false;
    let mut template = pz_inventory::ItemTemplate::new(ItemClass::Normal, "Base", "WorldVisual");
    template.script = Some(pz_inventory::template::ScriptItemRef(Rc::new(
        RefCell::new(definition),
    )));
    pz_inventory::env::create_from_template_ref(&Env, Rc::new(template), 1).unwrap()
}
fn texture() -> TextureRef {
    let mut texture = Texture::empty("Item_WorldVisual");
    texture.width_orig = 80;
    texture.height_orig = 40;
    Rc::new(LuaTexture::new(texture))
}
struct Host {
    manager: SpriteManagerRef,
    tables: crate::sprite::SpriteTables,
    fail_age: bool,
    fail_manager: bool,
}
impl Host {
    fn new() -> Self {
        let tables = crate::sprite::test_tables();
        Self {
            manager: Rc::new(RefCell::new(IsoSpriteManager::new(tables.clone()))),
            tables,
            fail_age: false,
            fail_manager: false,
        }
    }
}
impl NativeFloorHost for Host {
    fn sprite_manager(&self) -> HostResult<SpriteManagerRef> {
        if self.fail_manager {
            Err(RuntimeHostError::Borrowed("actual sprite owner provider"))
        } else {
            Ok(self.manager.clone())
        }
    }
    fn sprite_tables(&self) -> HostResult<crate::sprite::SpriteTables> {
        Ok(self.tables.clone())
    }
    fn world_age_hours(&self) -> HostResult<f64> {
        if self.fail_age {
            Err(RuntimeHostError::Borrowed("actual world age"))
        } else {
            Ok(42.0)
        }
    }
    fn tile_scale(&self) -> HostResult<i32> {
        Ok(2)
    }
    fn world_texture_enabled(&self) -> HostResult<bool> {
        Ok(true)
    }
    fn item_display_name(&self, _: &ItemRef) -> HostResult<String> {
        Ok("explicit fixture name".into())
    }
    fn add_to_world(
        &mut self,
        _: &crate::IsoObjectRef,
        _: &crate::SquareRef,
        _: &mut pz_entity::RetiredComponents,
    ) -> HostResult<()> {
        Err(RuntimeHostError::Unsupported {
            operation: "fixture native addWorld boundary",
            kind: None,
        })
    }
    fn designation_item_on_ground(
        &mut self,
        _: &crate::IsoObjectRef,
        _: &crate::SquareRef,
    ) -> HostResult<()> {
        panic!("addWorld failure must stop before designation")
    }
}
struct Render(Vec<u64>);
struct Rand;
impl crate::GameRand for Rand {
    fn next(&mut self, _: i32) -> i32 {
        17
    }
}
impl NativeFloorRender for Render {
    fn invalidate_render_chunk_level(
        &mut self,
        _: i32,
        _: i32,
        _: i32,
        flags: u64,
    ) -> HostResult<()> {
        self.0.push(flags);
        Ok(())
    }
}
fn object(
    item: &ItemRef,
    host: &Host,
    retired: &mut pz_entity::RetiredComponents,
) -> crate::IsoObjectRef {
    let sprite = runtime::create_runtime_sprite(&host.manager, &host.tables, retired).unwrap();
    let slot = ObjectSlot::new(IsoObject::new(
        1,
        ObjectKind::WorldInventory(crate::floor_object::FloorObject::new(
            IsoWorldInventoryObject {
                xoff: 0.2,
                yoff: 0.3,
                zoff: 0.0,
                offset_x: 0.0,
                offset_y: 0.0,
                item: FloorItem::retain(item),
                drop_time: -1.0,
                ignore_remove_sandbox: false,
                extended_placement: false,
            },
        )),
        None,
    ));
    let object = slot.ref_handle();
    object.borrow_mut().sprite = ObjectSprite::Native(sprite);
    object
}

#[test]
fn native_update_captures_tint_before_provider_and_reads_script_scale_after_same_actual_texture() {
    let _restore = Restore::new();
    let texture = texture();
    let item = actual_item(&texture);
    item.borrow_mut().col = [0.2, 0.3, 0.4, 0.5];
    let host = Host::new();
    let mut retired = pz_entity::RetiredComponents::new();
    let object = object(&item, &host, &mut retired);
    let sprite = actual_sprite(&object).unwrap();
    let tint = sprite.borrow().tint_slot().unwrap().clone();
    let def = sprite.borrow().def_slot().unwrap().clone();
    let script = pz_inventory::item_texture::get_script_item_ref(&item)
        .unwrap()
        .unwrap();
    let calls = Rc::new(Cell::new(0));
    let count = calls.clone();
    let weak_item = Rc::downgrade(&item);
    let supplied = texture.clone();
    drop(pz_render::install_texture_ref_loader(Some(Rc::new(
        move |_, _| {
            count.set(count.get() + 1);
            weak_item.upgrade().unwrap().borrow_mut().col = [0.9; 4];
            script.0.borrow_mut().scale_world_icon = 3.0;
            Ok(Some(supplied.clone()))
        },
    ))));
    update_floor_sprite(&object, &host, &mut retired).unwrap();
    assert_eq!(
        calls.get(),
        2,
        "prelookup then real LoadSingleTexture lookup"
    );
    assert_eq!(*tint.borrow(), [0.2, 0.3, 0.4, 0.5]);
    assert!(Rc::ptr_eq(
        sprite.borrow().texture_slot().unwrap(),
        &texture
    ));
    assert!(Rc::ptr_eq(sprite.borrow().tint_slot().unwrap(), &tint));
    assert!(Rc::ptr_eq(sprite.borrow().def_slot().unwrap(), &def));
    assert_eq!((def.borrow().scale_x, def.borrow().scale_y), (3.0, 3.0));
}

#[test]
fn native_update_null_gettex_is_uncaught_but_load_uses_sack_and_nullable_return() {
    let _restore = Restore::new();
    let texture = texture();
    let item = actual_item(&texture);
    pz_inventory::item_texture::set_texture_ref(&item, None).unwrap();
    let host = Host::new();
    let mut retired = pz_entity::RetiredComponents::new();
    let object = object(&item, &host, &mut retired);
    let calls = Rc::new(RefCell::new(Vec::new()));
    let log = calls.clone();
    drop(pz_render::install_texture_ref_loader(Some(Rc::new(
        move |_, name| {
            log.borrow_mut().push(name.to_string());
            Ok(None)
        },
    ))));
    assert!(matches!(
        update_floor_sprite(&object, &host, &mut retired),
        Err(RuntimeHostError::NativeException(_))
    ));
    assert!(
        calls.borrow().is_empty(),
        "updateSprite74 must not enter caught fallback"
    );
    assert!(!load_floor_sprite_visual(&object, &host, &mut retired).unwrap());
    assert_eq!(
        &*calls.borrow(),
        &[
            "media/inventory/world/WItem_Sack.png",
            "media/inventory/world/WItem_Sack.png"
        ]
    );
    assert!(actual_sprite(&object)
        .unwrap()
        .borrow()
        .texture_slot()
        .is_none());
}

#[test]
fn retained_native_target_survives_same_coordinate_replacement_and_late_addworld_fault() {
    let _restore = Restore::new();
    let texture = texture();
    let item = actual_item(&texture);
    let mut host = Host::new();
    let target = crate::SquareSlot::new(IsoGridSquare::new(1, 2, 0)).ref_handle();
    let replacement = crate::SquareSlot::new(IsoGridSquare::new(1, 2, 0));
    let replacement_ref = replacement.ref_handle();
    let mut chunk = crate::IsoChunk::new(0, 0);
    chunk.set_square(1, 2, 0, Some(replacement));
    let mut cell = IsoCell::default();
    cell.chunks.insert((0, 0), chunk);
    let mut rand = Rand;
    let mut render = Render(Vec::new());
    let result = NativeFloorPlacement {
        cell: &mut cell,
        rand: &mut rand,
        render: &mut render,
    }
    .place_retained(&target, &item, 0.2, 0.3, 0.0, &mut host);
    assert!(matches!(
        result,
        Err(WorldItemPlacementError::Component(
            RuntimeHostError::Unsupported {
                operation: "fixture native addWorld boundary",
                ..
            }
        ))
    ));
    assert!(replacement_ref.borrow().objects.is_empty());
    assert_eq!(target.borrow().objects.len(), 1);
    assert_eq!(target.borrow().world_objects.borrow().len(), 1);
    let actual = target.borrow().objects[0].ref_handle();
    assert!(Rc::ptr_eq(actual.borrow().square_slot().unwrap(), &target));
    assert!(item
        .borrow()
        .world_item_slot()
        .unwrap()
        .ptr_eq(&pz_entity::EntityOwner::from_ref(&actual)));
    assert_eq!(
        crate::floor_object::runtime::floor_ref(&actual)
            .unwrap()
            .borrow()
            .drop_time,
        42.0
    );
    assert!(actual.borrow().pending_floor_visual_tail.is_none());
    assert_eq!(
        render.0,
        [WORLD_ITEM_ADDED],
        "later failure stops final invalidation"
    );
    drop(cell.take_retired_owners());
}

#[test]
fn constructor_world_age_fault_preserves_same_sprite_and_square_before_time_transfer_or_lists() {
    let _restore = Restore::new();
    let texture = texture();
    let item = actual_item(&texture);
    let mut host = Host::new();
    host.fail_age = true;
    let square = crate::SquareSlot::new(IsoGridSquare::new(1, 2, 0)).ref_handle();
    let mut cell = IsoCell::default();
    let mut rand = Rand;
    let mut render = Render(Vec::new());
    let result = NativeFloorPlacement {
        cell: &mut cell,
        rand: &mut rand,
        render: &mut render,
    }
    .place_retained(&square, &item, 0.2, 0.3, 0.0, &mut host);
    assert!(matches!(
        result,
        Err(WorldItemPlacementError::Component(
            RuntimeHostError::Borrowed("actual world age")
        ))
    ));
    let receivers = cell.take_retired_objects();
    assert_eq!(receivers.len(), 1);
    let object = receivers[0].ref_handle();
    assert!(Rc::ptr_eq(object.borrow().square_slot().unwrap(), &square));
    let sprite = actual_sprite(&object).unwrap();
    assert!(Rc::ptr_eq(
        sprite.borrow().texture_slot().unwrap(),
        &texture
    ));
    assert_eq!(
        crate::floor_object::runtime::floor_ref(&object)
            .unwrap()
            .borrow()
            .drop_time,
        -1.0
    );
    assert!(square.borrow().objects.is_empty());
    assert!(item.borrow().world_item_slot().is_none());
    assert!(object.borrow().pending_floor_visual_tail.is_none());
    drop(cell.take_retired_owners());
}

#[test]
fn texture_host_fault_is_never_caught_as_sack_and_preserves_published_native_sprite_prefix() {
    let _restore = Restore::new();
    let texture = texture();
    let item = actual_item(&texture);
    item.borrow_mut().col = [0.2, 0.3, 0.4, 0.5];
    let mut host = Host::new();
    let names = Rc::new(RefCell::new(Vec::new()));
    let log = names.clone();
    drop(pz_render::install_texture_ref_loader(Some(Rc::new(
        move |_, name| {
            log.borrow_mut().push(name.to_string());
            Err(pz_render::TextureHostError::Borrowed(
                "actual image provider",
            ))
        },
    ))));
    let square = crate::SquareSlot::new(IsoGridSquare::new(1, 2, 0)).ref_handle();
    let mut cell = IsoCell::default();
    let mut rand = Rand;
    let mut render = Render(Vec::new());
    let result = NativeFloorPlacement {
        cell: &mut cell,
        rand: &mut rand,
        render: &mut render,
    }
    .place_retained(&square, &item, 0.2, 0.3, 0.0, &mut host);
    assert!(matches!(
        result,
        Err(WorldItemPlacementError::Component(
            RuntimeHostError::Borrowed("actual image provider")
        ))
    ));
    assert_eq!(&*names.borrow(), &["Item_WorldVisual"]);
    let receivers = cell.take_retired_objects();
    assert_eq!(receivers.len(), 1);
    let object = receivers[0].ref_handle();
    assert!(object.borrow().square_slot().is_none());
    let sprite = actual_sprite(&object).unwrap();
    assert_eq!(
        *sprite.borrow().tint_slot().unwrap().borrow(),
        [0.2, 0.3, 0.4, 0.5]
    );
    assert!(sprite.borrow().texture_slot().is_none());
    assert_eq!(
        object.borrow().pending_floor_visual_tail,
        Some(crate::objects::FloorVisualTail::ConstructorUpdateSprite)
    );
    assert!(square.borrow().objects.is_empty());
    assert!(item.borrow().world_item_slot().is_none());
    drop(cell.take_retired_owners());
}

#[test]
fn native_load_manager_fault_retains_read_offsets_and_cursor_before_item_record() {
    let _restore = Restore::new();
    let mut host = Host::new();
    host.fail_manager = true;
    let tables = crate::save::SaveTables::for_tests();
    let settings = crate::save::SaveSettings::default();
    let mut bytes = crate::save::io::Writer::new();
    bytes.i8(1);
    bytes.i8(tables.class_id("WorldInventoryItem"));
    for number in [0.25, 0.5, 0.75, 111.0, 222.0] {
        bytes.f32(number);
    }
    let mut reader = crate::save::io::Reader::new(&bytes.buf);
    let manager = host.manager.clone();
    let manager_guard = manager.borrow();
    let context = crate::save::SaveCtx {
        sprites: &manager_guard,
        tables: &tables,
        settings: &settings,
        items: &Env,
        hooks: &crate::save::NoSaveHooks,
        meta: None,
    };
    let failure = crate::save::object_io::load_object_slot_with_visual_host(
        &mut reader,
        &context,
        249,
        5,
        &mut host,
    )
    .unwrap_err();
    assert_eq!(failure.phase, "CreateSprite");
    assert_eq!(failure.position, bytes.buf.len());
    assert_eq!(reader.pos, bytes.buf.len());
    assert_eq!(
        failure.error,
        RuntimeHostError::Borrowed("actual sprite owner provider")
    );
    let object = failure.object.as_ref().unwrap();
    assert!(object.borrow().sprite.is_none());
    let floor = crate::floor_object::runtime::floor_ref(object).unwrap();
    assert_eq!(
        (
            floor.borrow().xoff,
            floor.borrow().yoff,
            floor.borrow().zoff
        ),
        (0.25, 0.5, 0.75)
    );
    assert!(floor.borrow().item.item_slot().is_none());
    drop(manager_guard);
    drop(failure);
}

#[test]
fn native_name_publication_precedes_later_key_borrow_fault_and_never_publishes_lists() {
    let _restore = Restore::new();
    let texture = texture();
    let item = actual_item(&texture);
    let mut host = Host::new();
    let square = crate::SquareSlot::new(IsoGridSquare::new(1, 2, 0)).ref_handle();
    let mut retired = pz_entity::RetiredComponents::new();
    let object = object(&item, &host, &mut retired);
    object.borrow_mut().key_id = 321;
    object
        .borrow_mut()
        .replace_square(Some(&square), &mut retired);
    let held = item.borrow_mut();
    // The explicit fixture name capability supplies the already-completed
    // getName result; it is not a generic production getName substitute.
    let result = finish_native_supplied(
        &square,
        ObjectSlot::retain(&object),
        &item,
        &mut host,
        &mut retired,
    );
    assert!(matches!(
        result,
        Err(RuntimeHostError::Borrowed("floor native key"))
    ));
    assert_eq!(
        object.borrow().name,
        Some(ObjectName::Str("explicit fixture name".into()))
    );
    assert_eq!(object.borrow().key_id, 321);
    assert!(square.borrow().objects.is_empty());
    assert!(square.borrow().world_objects.borrow().is_empty());
    assert!(held.world_item_slot().is_none());
    drop(held);
    drop(retired);
}
