//! Regression fixtures for dispatch on an actual retained receiver. The
//! registry hooks here are explicit traces, not a native registry substitute.
use super::*;
use crate::objects::{HutchMaster, IsoHutch, IsoMannequin};
use pz_inventory::shared::{ContainerRef, SharedContainer};
use pz_inventory::worn_items::{BodyLocationGroup, InventoryWornItemsRef, WornItems};

#[derive(Default)]
struct DispatchHooks {
    calls: Vec<WorldCall>,
    receivers: Vec<IsoObjectRef>,
    hutch_links_after_base: Option<(i32, i32)>,
}
impl SquareMutationHooks for DispatchHooks {
    fn world_call(
        &mut self,
        _: ObjectRef,
        object: IsoObjectRef,
        _: &mut dyn GameRand,
        call: WorldCall,
    ) {
        self.calls.push(call);
        self.receivers.push(object.clone());
        if call == WorldCall::FlagForHotSave {
            if let Some((x, y)) = self.hutch_links_after_base {
                let mut body = object.borrow_mut();
                let ObjectKind::Hutch(hutch) = &mut body.kind else {
                    panic!("hutch-only callback fixture");
                };
                hutch.linked_x = x;
                hutch.linked_y = y;
            }
        }
    }

    fn try_world_call(
        &mut self,
        r: ObjectRef,
        object: IsoObjectRef,
        rand: &mut dyn GameRand,
        call: WorldCall,
    ) -> pz_entity::runtime_load::HostResult<()> {
        self.world_call(r, object, rand, call);
        Ok(())
    }
}

fn mannequin() -> (IsoObjectRef, ContainerRef, InventoryWornItemsRef) {
    let container = SharedContainer::new(4, "mannequin").into_ref();
    let group = Rc::new(RefCell::new(BodyLocationGroup::new("Human").unwrap()));
    let worn = Rc::new(RefCell::new(WornItems::new(group)));
    let mut mannequin = IsoMannequin::new(7);
    drop(mannequin.install_runtime_inventory(container.clone(), worn.clone()));
    let object = ObjectSlot::new(IsoObject::new(
        31,
        ObjectKind::Mannequin(Box::new(mannequin)),
        None,
    ))
    .ref_handle();
    (object, container, worn)
}

fn run(world: &mut World, hooks: &mut DispatchHooks, object: &IsoObjectRef, add: bool) {
    let location = ObjectRef {
        pos: (2, 2, 0),
        id: object.borrow().id,
    };
    let mut env = world.env();
    env.world = hooks;
    if add {
        env.object_add_to_world(location, object).unwrap();
    } else {
        env.object_remove_from_world_detached(location, object)
            .unwrap();
    }
}

fn assert_same_mannequin(
    object: &IsoObjectRef,
    container: &ContainerRef,
    worn: &InventoryWornItemsRef,
    hooks: &DispatchHooks,
    add: bool,
) {
    let body = object.borrow();
    let ObjectKind::Mannequin(mannequin) = &body.kind else {
        panic!("receiver class changed");
    };
    assert!(Rc::ptr_eq(
        mannequin.runtime_container.as_ref().unwrap(),
        container
    ));
    assert!(Rc::ptr_eq(
        mannequin.runtime_worn_items.as_ref().unwrap(),
        worn
    ));
    assert_eq!(body.added_to_world_or_equipped, add);
    assert!(!hooks.receivers.is_empty());
    assert!(hooks.receivers.iter().all(|r| Rc::ptr_eq(r, object)));
    assert_eq!(
        hooks.calls.last(),
        Some(if add {
            &WorldCall::MannequinInit
        } else {
            &WorldCall::MannequinListRemove
        })
    );
}

#[test]
fn add_mannequin_dispatch_does_not_copy_borrowed_inventory() {
    let mut world = World::new();
    let (object, container, worn) = mannequin();
    let held = container.borrow_mut();
    let mut hooks = DispatchHooks::default();
    run(&mut world, &mut hooks, &object, true);
    assert_same_mannequin(&object, &container, &worn, &hooks, true);
    assert!(held.items.is_empty());
}

#[test]
fn remove_mannequin_dispatch_does_not_copy_borrowed_inventory() {
    let mut world = World::new();
    let (object, container, worn) = mannequin();
    object.borrow_mut().added_to_world_or_equipped = true;
    let held = container.borrow_mut();
    let mut hooks = DispatchHooks::default();
    run(&mut world, &mut hooks, &object, false);
    assert_same_mannequin(&object, &container, &worn, &hooks, false);
    assert!(held.items.is_empty());
}

#[test]
fn add_mannequin_dispatch_does_not_copy_borrowed_worn_items() {
    let mut world = World::new();
    let (object, container, worn) = mannequin();
    let held = worn.borrow_mut();
    let mut hooks = DispatchHooks::default();
    run(&mut world, &mut hooks, &object, true);
    assert_same_mannequin(&object, &container, &worn, &hooks, true);
    assert!(held.items().is_empty());
}

#[test]
fn remove_mannequin_dispatch_does_not_copy_borrowed_worn_items() {
    let mut world = World::new();
    let (object, container, worn) = mannequin();
    object.borrow_mut().added_to_world_or_equipped = true;
    let held = worn.borrow_mut();
    let mut hooks = DispatchHooks::default();
    run(&mut world, &mut hooks, &object, false);
    assert_same_mannequin(&object, &container, &worn, &hooks, false);
    assert!(held.items().is_empty());
}

fn hutch(master: bool, links: (i32, i32)) -> IsoObjectRef {
    let master = master.then(|| HutchMaster {
        sprite_name: "fixture".into(),
        open: false,
        open_egg_hatch: false,
        saved_x: 0,
        saved_y: 0,
        saved_z: 0,
        animals: Vec::new(),
        hutch_dirt: 0.0,
        nest_box_dirt: 0.0,
        nest_boxes: Vec::new(),
    });
    ObjectSlot::new(IsoObject::new(
        32,
        ObjectKind::Hutch(Box::new(IsoHutch {
            linked_x: links.0,
            linked_y: links.1,
            linked_z: 0,
            master,
        })),
        None,
    ))
    .ref_handle()
}

#[test]
fn hutch_registration_uses_link_coordinates_instead_of_archive_payload_presence() {
    for master in [false, true] {
        for (links, register) in [
            ((0, 0), true),
            ((1, 0), true),
            ((0, 1), true),
            ((1, 1), false),
        ] {
            let mut world = World::new();
            let object = hutch(master, links);
            let mut hooks = DispatchHooks::default();
            run(&mut world, &mut hooks, &object, true);
            assert_eq!(
                hooks.calls.contains(&WorldCall::HutchManager { add: true }),
                register
            );
            assert!(hooks.receivers.iter().all(|r| Rc::ptr_eq(r, &object)));
        }
    }
}

#[test]
fn hutch_registration_reads_current_links_after_base_callback() {
    for (initial, current, register) in [((1, 1), (0, 0), true), ((0, 0), (1, 1), false)] {
        let mut world = World::new();
        let object = hutch(initial == (0, 0), initial);
        let mut hooks = DispatchHooks {
            hutch_links_after_base: Some(current),
            ..Default::default()
        };
        run(&mut world, &mut hooks, &object, true);
        assert_eq!(
            hooks.calls.contains(&WorldCall::HutchManager { add: true }),
            register
        );
        let base_tail = hooks
            .calls
            .iter()
            .position(|c| *c == WorldCall::FlagForHotSave)
            .unwrap();
        if register {
            assert_eq!(
                hooks.calls[base_tail + 1],
                WorldCall::HutchManager { add: true }
            );
        } else {
            assert_eq!(base_tail, hooks.calls.len() - 1);
        }
    }
}

#[test]
fn fire_dispatch_preserves_perm_branch_and_receiver_across_add_and_remove() {
    for perm in [false, true] {
        let mut world = World::new();
        let mut fire = crate::fire::IsoFire::permanent();
        fire.perm = perm;
        let object = ObjectSlot::new(IsoObject::new(33, ObjectKind::Fire(fire), None)).ref_handle();
        let mut hooks = DispatchHooks::default();
        run(&mut world, &mut hooks, &object, true);
        assert_eq!(
            hooks.calls,
            vec![if perm {
                WorldCall::StaticUpdater { add: true }
            } else {
                WorldCall::FireManager { add: true }
            }]
        );
        hooks.calls.clear();
        hooks.receivers.clear();
        run(&mut world, &mut hooks, &object, false);
        assert_eq!(
            hooks.calls.contains(&WorldCall::FireManager { add: false }),
            !perm
        );
        let sound = hooks
            .calls
            .iter()
            .position(|c| *c == WorldCall::FireStopSound)
            .unwrap();
        let unregister = hooks
            .calls
            .iter()
            .position(|c| *c == WorldCall::UnregisterEntity)
            .unwrap();
        assert!(sound < unregister);
        assert_eq!(sound, usize::from(!perm));
        assert!(hooks.receivers.iter().all(|r| Rc::ptr_eq(r, &object)));
    }
}
