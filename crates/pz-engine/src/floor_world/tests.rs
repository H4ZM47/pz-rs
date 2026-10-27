use super::*;
use pz_inventory::{InventoryItem, ItemClass, ItemTemplate};
use pz_lua_api::bind_world::{set_cell, set_meta_grid, with_world, WorldState};
use pz_world::{IsoChunk, IsoGridSquare, IsoObject, ObjectKind, ObjectSlot, SquareSlot};
use std::path::Path;

struct Restore {
    world: Option<WorldState>,
}
impl Drop for Restore {
    fn drop(&mut self) {
        let retired_floor = pz_lua_api::bind_character_sim::reset_floor_world();
        let retired = with_world(|world| std::mem::replace(world, self.world.take().unwrap()));
        drop((retired_floor, retired));
        pz_inventory::component_host::drain_retired_components();
    }
}
fn fixture() -> (Restore, IsoWorld, pz_world::SquareRef) {
    let restore = Restore {
        world: Some(with_world(std::mem::take)),
    };
    let mut world = IsoWorld::new(
        Path::new("unused-floor-service"),
        Path::new("unused-floor-service"),
        "unused-floor-service".into(),
    );
    let mut sprites = IsoSpriteManager::new(pz_world::sprite::test_tables());
    let floor = sprites.add_sprite("floor", 1);
    let (tables, alias) = (sprites.tables.clone(), sprites.alias.clone());
    sprites.sprites[floor]
        .properties
        .set(&tables, &alias, "solidfloor", "", true);
    let mut square = IsoGridSquare::new(2, 2, 0);
    square.objects.push(ObjectSlot::new(IsoObject::new(
        1,
        ObjectKind::Object,
        Some(floor),
    )));
    square.recalc_properties(&sprites);
    let square = SquareSlot::new(square);
    let actual_square = square.ref_handle();
    let mut chunk = IsoChunk::new(0, 0);
    chunk.set_square(2, 2, 0, Some(square));
    let mut cell = IsoCell::default();
    cell.chunks.insert((0, 0), chunk);
    set_meta_grid(Some(IsoMetaGrid::with_bounds(0, 0, 1, 1)));
    set_cell(Some(cell));
    world.sprites = Some(Rc::new(RefCell::new(sprites)));
    world.regions = Some(Rc::new(RefCell::new(IsoRegions::new(0))));
    world.door_tables = Some(Rc::new(DoorTables {
        x_offset: [[[0; 4]; 2]; 2],
        y_offset: [[[0; 4]; 2]; 2],
    }));
    world.install_floor_world().unwrap();
    (restore, world, actual_square)
}
fn item() -> ItemRef {
    pz_inventory::shared::new_item(InventoryItem::create(
        Rc::new(ItemTemplate::new(ItemClass::Normal, "Base", "Bandage")),
        5,
    ))
}

#[test]
fn installed_service_uses_same_resources_and_typed_actual_world_and_square_queries() {
    let (_restore, world, square) = fixture();
    let service = world.floor_world.as_ref().unwrap();
    assert!(Rc::ptr_eq(
        &service.borrow().session_owner.upgrade().unwrap(),
        world.floor_session.as_ref().unwrap()
    ));
    assert!(Rc::ptr_eq(
        &service.borrow().sprites.upgrade().unwrap(),
        world.sprites.as_ref().unwrap()
    ));
    assert!(Rc::ptr_eq(
        &service.borrow().regions.upgrade().unwrap(),
        world.regions.as_ref().unwrap()
    ));
    assert!(Rc::ptr_eq(
        &service.borrow().render.upgrade().unwrap(),
        &world.render
    ));
    assert_eq!(
        service.borrow().solid_floor_at(2, 2, 1),
        Ok(Some((2, 2, 0)))
    );
    assert_eq!(service.borrow().solid_floor_at(5, 5, 0), Ok(None));
    {
        let _square = square.borrow_mut();
        assert!(matches!(
            service.borrow().solid_floor_at(2, 2, 0),
            Err(RuntimeHostError::Borrowed("solid floor square"))
        ));
        assert!(matches!(
            service.borrow().apparent_z((2, 2, 0), 0.2, 0.3),
            Err(RuntimeHostError::Borrowed("apparent floor square"))
        ));
    }
    with_world(|_| {
        assert!(matches!(
            service.borrow().solid_floor_at(2, 2, 0),
            Err(RuntimeHostError::Borrowed(_))
        ))
    });
}

#[test]
fn reset_invalidates_retained_service_before_cell_replacement_even_with_resources_pinned() {
    let (_restore, world, _) = fixture();
    let service = world.floor_world.as_ref().unwrap().clone();
    let actual_cell_generation = cell_generation();
    let old = pz_lua_api::bind_character_sim::reset_floor_world();
    assert_eq!(cell_generation(), actual_cell_generation);
    assert!(matches!(
        service.borrow().solid_floor_at(2, 2, 0),
        Err(RuntimeHostError::Unsupported {
            operation: "stale actual floor world session",
            ..
        })
    ));
    assert!(service.borrow_mut().rand_float(0.1, 0.9).is_err());
    assert!(service
        .borrow_mut()
        .add_world_inventory_item((2, 2, 0), &item(), 0.2, 0.3, 0.0)
        .is_err());
    drop(old);
}

#[test]
fn actual_cell_replacement_invalidates_old_service_while_old_resources_are_retained() {
    let (_restore, world, _) = fixture();
    let service = world.floor_world.as_ref().unwrap().clone();
    let provider_generation = floor_world_generation();
    set_cell(Some(IsoCell::default()));
    assert_eq!(floor_world_generation(), provider_generation);
    assert!(matches!(
        service.borrow().solid_floor_at(2, 2, 0),
        Err(RuntimeHostError::Unsupported {
            operation: "stale actual floor world session",
            ..
        })
    ));
    assert!(service.borrow_mut().rand_float(0.1, 0.9).is_err());
}

#[test]
fn native_service_stops_at_visual_constructor_before_time_transfer_lists_and_backlink() {
    let (_restore, world, square) = fixture();
    let item = item();
    item.borrow_mut().world_x_rotation = 22.0;
    item.borrow_mut().world_y_rotation = 33.0;
    item.borrow_mut().world_z_rotation = 9.0;
    let before: Vec<_> = square
        .borrow()
        .objects
        .iter()
        .map(ObjectSlot::ref_handle)
        .collect();
    let result = world
        .floor_world
        .as_ref()
        .unwrap()
        .borrow_mut()
        .add_world_inventory_item((2, 2, 0), &item, 0.2, 0.3, 0.0);
    assert!(matches!(
        result,
        Err(RuntimeHostError::Unsupported {
            operation: "native IsoSprite.CreateSprite/LoadSingleTexture constructor",
            ..
        })
    ));
    assert!(item.borrow().world_item_slot().is_none());
    assert_eq!(
        (
            item.borrow().world_x_rotation,
            item.borrow().world_y_rotation,
            item.borrow().world_z_rotation
        ),
        (0.0, 0.0, 9.0)
    );
    let square_body = square.borrow();
    assert_eq!(square_body.objects.len(), before.len());
    for (after, before) in square_body.objects.iter().zip(&before) {
        assert!(after.ptr_eq_ref(before));
    }
    assert!(square_body.world_objects.borrow().is_empty());
    drop(square_body);
    let retired = with_world(|world| world.cell.as_mut().unwrap().take_retired_objects());
    assert_eq!(retired.len(), 1);
    let receiver = retired[0].ref_handle();
    let body = receiver.borrow();
    assert_eq!(
        body.pending_floor_visual_tail,
        Some(pz_world::objects::FloorVisualTail::ConstructorCreateSprite)
    );
    assert!(body.sprite.is_none() && body.sprite_id_override.is_none());
    assert!(!body.added_to_world_or_equipped);
    let ObjectKind::WorldInventory(floor) = &body.kind else {
        panic!("actual floor receiver")
    };
    let floor = floor.payload_slot().unwrap().borrow();
    assert_eq!(floor.drop_time, -1.0);
    assert_eq!((floor.xoff, floor.yoff, floor.zoff), (0.2, 0.3, 0.0));
    assert!(Rc::ptr_eq(floor.item.item_slot().unwrap(), &item));
}

#[test]
fn actual_state_adapter_uses_borrowed_entry_without_reentering_world_or_sprite_guards() {
    let (_restore, mut world, _) = fixture();
    let item = item();
    let sprites = world.sprites.as_ref().unwrap().clone();
    let sim = crate::sim::FrameSim::capture();
    let result = crate::world_update::with_states(
        &sprites,
        &mut world.states_runtime,
        None,
        &sim,
        world.floor_world.as_ref(),
        |adapter| {
            let floor = pz_characters::character::state::StateWorld::<
                pz_lua_api::bind_characters::LuaItem,
            >::floor(adapter)
            .unwrap();
            assert_eq!(floor.solid_floor_at(2, 2, 0), Ok(Some((2, 2, 0))));
            floor.add_world_inventory_item((2, 2, 0), &item, 0.2, 0.3, 0.0)
        },
    )
    .unwrap();
    assert!(matches!(result,Err(RuntimeHostError::Unsupported { operation:"native IsoSprite.CreateSprite/LoadSingleTexture constructor", .. })),"must reach strict native constructor rather than fail reentrant WORLD/sprite borrow: {result:?}");
}

#[test]
fn real_lua_drop_reaches_actual_cell_constructor_and_returns_typed_visual_failure() {
    let (_restore, mut world, square) = fixture();
    let fs = Rc::new(RefCell::new(pz_fs::GameFs::new("unused-floor-lua")));
    let mut lua = pz_lua_api::LuaManager::init(fs).unwrap();
    // The real game installs after registration and cell creation too.
    world.install_floor_world().unwrap();
    struct RestoreLocalPlayer(Option<pz_lua_api::bind_character_sim::LuaGameCharacterRef>);
    impl Drop for RestoreLocalPlayer {
        fn drop(&mut self) {
            pz_lua_api::bind_players::set_player(0, self.0.take());
        }
    }
    let _local_player = RestoreLocalPlayer(pz_lua_api::bind_players::player(0));
    let sm = pz_lua_api::bind_registry::shared_script_manager();
    let player = Rc::new(RefCell::new(
        pz_lua_api::bind_character_sim::LuaGameCharacter::new(
            &sm,
            pz_characters::CharacterKind::Player,
        ),
    ));
    let item = item();
    {
        let mut player = player.borrow_mut();
        player.path.current_square = Some((2, 2, 0));
        pz_inventory::shared::add_item(&player.items.inventory, &item);
        player.set_primary_hand_item(&sm, Some(item.borrow().id));
    }
    lua.thread.globals().set_str(
        "actualFloorPlayer",
        pz_lua_api::bind_character_sim::character_value(&player),
    );
    let call = pz_kahlua::stdlib::load_string(
        "actualFloorPlayer:dropHandItems()",
        Some("actual-floor-service"),
        lua.thread.globals().clone(),
    )
    .unwrap();
    // Native non-local-player guard is a genuine no-op; creating a Player
    // alone does not register it in IsoPlayer.players.
    assert!(lua.thread.call(&call, &[]).unwrap().is_empty());
    assert_eq!(
        player.borrow().items.primary_hand_item,
        Some(item.borrow().id)
    );
    assert!(pz_inventory::shared::container_of(&item).is_some());
    assert!(with_world(|world| world.cell.as_mut().unwrap().take_retired_objects()).is_empty());
    // Use the same real registry as production to enter the local action.
    pz_lua_api::bind_players::set_player(0, Some(player.clone()));
    let result = lua.thread.call(&call, &[]);
    assert!(
        matches!(
            result,
            Err(pz_kahlua::LuaError::Runtime {
                class: "pz.host.Unsupported",
                ..
            })
        ),
        "must surface genuine constructor capability: {result:?}"
    );
    assert!(player.borrow().items.primary_hand_item.is_none());
    assert!(player.borrow().items.inventory.borrow().items.is_empty());
    assert!(pz_inventory::shared::container_of(&item).is_none());
    assert!(item.borrow().world_item_slot().is_none());
    assert!(square.borrow().world_objects.borrow().is_empty());
    let retired = with_world(|world| world.cell.as_mut().unwrap().take_retired_objects());
    assert_eq!(
        retired.len(),
        1,
        "actual constructor prefix receiver is retained by actual cell"
    );
}

#[test]
fn held_regions_do_not_precede_native_constructor_but_renderer_borrow_is_at_pc20() {
    let (_restore, world, square) = fixture();
    let item = item();
    {
        let mut body = item.borrow_mut();
        body.world_x_rotation = 22.0;
        body.world_y_rotation = 33.0;
        body.world_z_rotation = 9.0;
    }
    let _held_regions = world.regions.as_ref().unwrap().borrow_mut();
    let result = world
        .floor_world
        .as_ref()
        .unwrap()
        .borrow_mut()
        .add_world_inventory_item((2, 2, 0), &item, 0.2, 0.3, 0.0);
    assert!(
        matches!(
            result,
            Err(RuntimeHostError::Unsupported {
                operation: "native IsoSprite.CreateSprite/LoadSingleTexture constructor",
                ..
            })
        ),
        "unrelated region body must not mask native prefix: {result:?}"
    );
    assert_eq!(
        (
            item.borrow().world_x_rotation,
            item.borrow().world_y_rotation
        ),
        (0.0, 0.0)
    );
    let retired = with_world(|world| world.cell.as_mut().unwrap().take_retired_objects());
    assert_eq!(retired.len(), 1);
    assert!(square.borrow().world_objects.borrow().is_empty());
    drop(retired);

    item.borrow_mut().world_x_rotation = 44.0;
    item.borrow_mut().world_y_rotation = 55.0;
    let _held_render = world.render.borrow_mut();
    let result = world
        .floor_world
        .as_ref()
        .unwrap()
        .borrow_mut()
        .add_world_inventory_item((2, 2, 0), &item, 0.2, 0.3, 0.0);
    assert!(matches!(
        result,
        Err(RuntimeHostError::Borrowed("actual floor renderer"))
    ));
    assert_eq!(
        (
            item.borrow().world_x_rotation,
            item.borrow().world_y_rotation
        ),
        (44.0, 55.0),
        "native render invalidation PC20 precedes ordinary constructor rotations"
    );
    assert!(with_world(|world| world.cell.as_mut().unwrap().take_retired_objects()).is_empty());
}

#[test]
fn corpse_branch_precedes_actual_renderer_borrow() {
    let (_restore, world, _) = fixture();
    let corpse = pz_inventory::shared::new_item(InventoryItem::create(
        Rc::new(ItemTemplate::new(ItemClass::Normal, "Base", "CorpseMale")),
        8,
    ));
    corpse.borrow_mut().world_x_rotation = 27.0;
    let _held_render = world.render.borrow_mut();
    let result = world
        .floor_world
        .as_ref()
        .unwrap()
        .borrow_mut()
        .add_world_inventory_item((2, 2, 0), &corpse, 0.2, 0.3, 0.0);
    assert!(
        matches!(
            result,
            Err(RuntimeHostError::Unsupported {
                operation: "native corpse floor placement",
                ..
            })
        ),
        "corpse attempt is before supplied-drop render invalidation: {result:?}"
    );
    assert_eq!(corpse.borrow().world_x_rotation, 27.0);
    assert!(with_world(|world| world.cell.as_mut().unwrap().take_retired_objects()).is_empty());
}

#[test]
fn expired_actual_session_owner_rejects_every_operation_even_with_all_resources_pinned() {
    let (_restore, world, _) = fixture();
    let service = world.floor_world.as_ref().unwrap().clone();
    let sprites = world.sprites.as_ref().unwrap().clone();
    let regions = world.regions.as_ref().unwrap().clone();
    let doors = world.door_tables.as_ref().unwrap().clone();
    let render = world.render.clone();
    let generations = (cell_generation(), floor_world_generation());
    drop(world); // No stop/reset: all resource owners above intentionally survive.
    assert_eq!((cell_generation(), floor_world_generation()), generations);
    let _sprites = sprites.borrow_mut();
    let _regions = regions.borrow_mut();
    let _render = render.borrow_mut();
    let expected = RuntimeHostError::Unsupported {
        operation: "expired actual floor world owner",
        kind: None,
    };
    assert_eq!(service.borrow().validate().unwrap_err(), expected);
    assert_eq!(service.borrow().fbo_render_chunk().unwrap_err(), expected);
    assert_eq!(
        service.borrow().solid_floor_at(2, 2, 0).unwrap_err(),
        expected
    );
    assert_eq!(
        service
            .borrow()
            .apparent_z((2, 2, 0), 0.2, 0.3)
            .unwrap_err(),
        expected
    );
    assert_eq!(
        service.borrow_mut().rand_float(0.1, 0.9).unwrap_err(),
        expected
    );
    assert_eq!(
        service
            .borrow_mut()
            .add_world_inventory_item((2, 2, 0), &item(), 0.2, 0.3, 0.0)
            .unwrap_err(),
        expected
    );
    assert!(Rc::strong_count(&doors) > 0);
    assert!(with_world(|world| world.cell.as_mut().unwrap().take_retired_objects()).is_empty());
}
