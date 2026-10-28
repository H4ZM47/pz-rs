use super::*;
use pz_lua_api::bind_world::{set_cell, with_world, WorldState};
use pz_world::designation::{new_base_default, try_is_fully_streamed, DesignationZoneKind};
use pz_world::{IsoCell, IsoChunk, IsoGridSquare, SquareSlot};
use std::path::Path;

struct Restore {
    roots: Option<NativeClassRoots>,
    context: Option<DesignationContext>,
    world: Option<WorldState>,
}
impl Restore {
    fn new() -> Self {
        Self {
            roots: CLASSES.with(|slot| slot.borrow_mut().take()),
            context: bind_designation::install_context(None),
            world: Some(with_world(std::mem::take)),
        }
    }
}
impl Drop for Restore {
    fn drop(&mut self) {
        let context = bind_designation::install_context(self.context.take());
        let roots =
            CLASSES.with(|slot| std::mem::replace(&mut *slot.borrow_mut(), self.roots.take()));
        let world = with_world(|world| std::mem::replace(world, self.world.take().unwrap()));
        drop((context, roots, world));
        pz_inventory::component_host::drain_retired_components();
    }
}
fn world() -> IsoWorld {
    IsoWorld::new(
        Path::new("unused-designation"),
        Path::new("unused-designation"),
        "unused-designation".into(),
    )
}

#[test]
fn class_lists_survive_world_replacement_and_reset_without_replacing_retained_owners() {
    let _restore = Restore::new();
    let boot_registry = install_actual_designation_classes().unwrap();
    assert!(boot_registry
        .borrow()
        .animal_zones_slot()
        .unwrap()
        .borrow()
        .is_empty());
    let mut first = world();
    first.init_designation_registry().unwrap();
    let registry = first
        .meta_world
        .designation_zones
        .live_slot()
        .unwrap()
        .clone();
    assert!(Rc::ptr_eq(&boot_registry, &registry));
    let list = registry.borrow().all_zones_slot().unwrap().clone();
    let mut retired = pz_entity::RetiredComponents::new();
    let zone = designation::try_new_zone(
        &registry,
        DesignationZoneKind::Base,
        Some("probe"),
        Some("retained"),
        2,
        3,
        0,
        8,
        9,
        false,
        &mut ActualDesignationHost,
        &mut retired,
    )
    .unwrap();
    retire_components(retired);
    let mut second = world();
    second.init_designation_registry().unwrap();
    assert!(Rc::ptr_eq(
        &registry,
        second.meta_world.designation_zones.live_slot().unwrap()
    ));
    assert!(Rc::ptr_eq(
        &list,
        registry.borrow().all_zones_slot().unwrap()
    ));
    assert_eq!(list.borrow().len(), 1);
    second.reset_designation_registry().unwrap();
    assert!(Rc::ptr_eq(
        &list,
        registry.borrow().all_zones_slot().unwrap()
    ));
    assert!(list.borrow().is_empty());
    assert_eq!(zone.borrow().name.as_deref(), Some("retained"));
    assert_eq!((zone.borrow().x, zone.borrow().w), (2, 6));
}

#[test]
fn actual_current_cell_is_selected_before_corner_zone_fields() {
    let _restore = Restore::new();
    let zone = new_base_default();
    let mut host = ActualDesignationHost;
    let held = zone.borrow_mut();
    // Native's initial null cell branch never reads this held zone.
    assert!(!try_is_fully_streamed(&zone, &mut host).unwrap());
    set_cell(Some(IsoCell::default()));
    assert!(matches!(
        try_is_fully_streamed(&zone, &mut host),
        Err(RuntimeHostError::Borrowed("Designation corner coordinates"))
    ));
    with_world(|_| {
        assert!(matches!(
            try_is_fully_streamed(&zone, &mut host),
            Err(RuntimeHostError::Borrowed("Lua actual world read"))
        ));
    });
    drop(held);
}

#[test]
fn streaming_query_uses_current_cell_after_replacement() {
    let _restore = Restore::new();
    let zone = new_base_default();
    {
        let mut zone = zone.borrow_mut();
        zone.x = 2;
        zone.y = 3;
        zone.w = 1;
        zone.h = 1;
    }
    let square = SquareSlot::new(IsoGridSquare::new(2, 3, 0));
    let retained = square.ref_handle();
    let mut chunk = IsoChunk::new(0, 0);
    chunk.set_square(2, 3, 0, Some(square));
    let mut cell = IsoCell::default();
    cell.chunks.insert((0, 0), chunk);
    set_cell(Some(cell));
    assert!(try_is_fully_streamed(&zone, &mut ActualDesignationHost).unwrap());
    set_cell(Some(IsoCell::default()));
    assert!(!try_is_fully_streamed(&zone, &mut ActualDesignationHost).unwrap());
    assert_eq!(
        retained.borrow().x,
        2,
        "retained old square remains an independent owner"
    );
}

#[test]
fn sync_reads_actual_membership_before_roles_and_never_reads_zone_body() {
    let _restore = Restore::new();
    let registry = install_actual_designation_classes().unwrap();
    let list = registry.borrow().all_zones_slot().unwrap().clone();
    let zone = new_base_default();
    let held_zone = zone.borrow_mut();
    let core = pz_lua_api::bind_core::shared_core();
    let old_roles = {
        let core = core.borrow();
        (core.is_client, core.is_server)
    };
    struct Roles(bool, bool);
    impl Drop for Roles {
        fn drop(&mut self) {
            let core = pz_lua_api::bind_core::shared_core();
            let mut core = core.borrow_mut();
            core.is_client = self.0;
            core.is_server = self.1;
        }
    }
    let _roles = Roles(old_roles.0, old_roles.1);
    {
        let mut core = core.borrow_mut();
        core.is_client = false;
        core.is_server = false;
    }
    ActualDesignationHost.sync_zone(&zone).unwrap();
    let held_core = core.borrow_mut();
    let held_list = list.borrow_mut();
    assert_eq!(
        ActualDesignationHost.sync_zone(&zone),
        Err(RuntimeHostError::Borrowed(
            "designation sync allZones.contains"
        ))
    );
    drop(held_list);
    assert_eq!(
        ActualDesignationHost.sync_zone(&zone),
        Err(RuntimeHostError::Borrowed("designation sync client role"))
    );
    drop(held_core);
    core.borrow_mut().is_server = true;
    assert_eq!(
        ActualDesignationHost.sync_zone(&zone),
        Err(unsupported("actual designation server synchronization"))
    );
    core.borrow_mut().is_client = true;
    assert_eq!(
        ActualDesignationHost.sync_zone(&zone),
        Err(unsupported("actual designation client synchronization"))
    );
    drop(held_zone);
}
