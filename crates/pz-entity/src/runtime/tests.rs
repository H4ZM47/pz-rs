use super::*;
use crate::context::BasicContext;
use crate::fluids::{FluidInstance, FluidRef};
use std::cell::Cell;
use std::rc::Weak;

const VERSION: i32 = 249;

fn fluid_ref() -> ComponentRef {
    RuntimeComponent::new_unowned(Component::FluidContainer(FluidContainer::default())).unwrap()
}

fn empty_ref(kind: ComponentType) -> ComponentRef {
    RuntimeComponent::new_unowned(Component::Empty(kind)).unwrap()
}

#[test]
fn owned_retired_payloads_append_without_dropping_under_the_callers_guard() {
    struct Probe {
        guard: Rc<RefCell<()>>,
        drops: Rc<Cell<usize>>,
        dropped_under_guard: Rc<Cell<bool>>,
    }
    impl Drop for Probe {
        fn drop(&mut self) {
            if self.guard.try_borrow_mut().is_err() {
                self.dropped_under_guard.set(true);
            }
            self.drops.set(self.drops.get() + 1);
        }
    }

    let guard = Rc::new(RefCell::new(()));
    let drops = Rc::new(Cell::new(0));
    let dropped_under_guard = Rc::new(Cell::new(false));
    let mut outer = RetiredComponents::new();
    assert!(outer.is_empty());
    let held = guard.borrow_mut();
    for _ in 0..2 {
        let mut inner = RetiredComponents::new();
        inner.push_payload(Box::new(vec![Probe {
            guard: Rc::clone(&guard),
            drops: Rc::clone(&drops),
            dropped_under_guard: Rc::clone(&dropped_under_guard),
        }]));
        assert!(!inner.is_empty());
        assert_eq!(inner.len(), 1);
        outer.append(inner);
        assert_eq!(drops.get(), 0);
    }
    assert_eq!(outer.len(), 2);
    assert!(format!("{outer:?}").contains("payloads: 2"));
    assert_eq!(drops.get(), 0);
    drop(held);
    drop(outer);
    assert_eq!(drops.get(), 2);
    assert!(!dropped_under_guard.get());
}

#[test]
fn erased_owner_is_direct_actual_allocation_and_borrowed_slot_is_not_a_clone() {
    let entity = Rc::new(RefCell::new(31i32));
    let owner = EntityOwner::from_ref(&entity);
    assert_eq!(Rc::strong_count(&entity), 2);
    assert!(std::ptr::eq(
        owner.downcast_ref::<i32>().unwrap(),
        entity.as_ref()
    ));
    assert!(owner.is::<i32>());
    assert!(!owner.is::<u32>());
    assert!(owner.try_handle::<u32>().is_none());
    assert_eq!(Rc::strong_count(&entity), 2);
    let same = owner.try_handle::<i32>().unwrap();
    assert!(Rc::ptr_eq(&same, &entity));
    *same.borrow_mut() = 52;
    assert_eq!(*entity.borrow(), 52);
    drop(same);
    let before = Rc::strong_count(&entity);
    let slot = owner.erased_slot();
    assert!(slot.is::<RefCell<i32>>());
    assert_eq!(Rc::strong_count(&entity), before);
    let alias = owner.clone();
    assert!(owner.ptr_eq(&alias));
    assert!(!owner.ptr_eq(&EntityOwner::from_ref(&Rc::new(RefCell::new(52i32)))));
    drop(alias);
    drop(owner);
    assert_eq!(Rc::strong_count(&entity), 1);
}

#[test]
fn factory_values_move_without_quantizing_or_discarding_unsaved_fields() {
    let mut fluid = FluidContainer {
        capacity: f32::from_bits(0x3f80_0001),
        rain_catcher: f32::from_bits(0x8000_0000),
        custom_drink_sound: Some("FactoryOnlyDrink".into()),
        fills_with_clean_water: true,
        amount_cache: 0.31415927,
        cache_invalidated: true,
        ..Default::default()
    };
    fluid.fluids.push(FluidInstance {
        fluid: Some(FluidRef::Type(0)),
        custom_color: Some([0.12345679, 0.2345679, 0.3456789]),
        amount: f32::from_bits(0x3e00_0001),
    });
    let moved_allocation = fluid.fluids.as_ptr();
    let expected_color = fluid.fluids[0].custom_color.unwrap();
    let expected_amount = fluid.fluids[0].amount;
    let store = RuntimeComponents::adopt_values_unowned(Components {
        list: vec![Component::FluidContainer(fluid)],
    })
    .unwrap();
    let component = store
        .get_ref(ComponentType::FluidContainer)
        .unwrap()
        .unwrap();
    let borrowed = component.borrow();
    let live = borrowed.fluid().unwrap();
    assert_eq!(
        live.fluids.as_ptr(),
        moved_allocation,
        "factory body was moved, not roundtripped/copied"
    );
    assert_eq!(live.capacity.to_bits(), 0x3f80_0001);
    assert_eq!(live.rain_catcher.to_bits(), 0x8000_0000);
    assert_eq!(live.fluids[0].amount.to_bits(), expected_amount.to_bits());
    assert_eq!(
        live.fluids[0].custom_color.unwrap().map(f32::to_bits),
        expected_color.map(f32::to_bits)
    );
    assert_eq!(live.custom_drink_sound.as_deref(), Some("FactoryOnlyDrink"));
    assert!(live.fills_with_clean_water && live.cache_invalidated);
    let cache_bits = live.amount_cache.to_bits();
    drop(borrowed);
    store.save(VERSION).unwrap();
    let borrowed = component.borrow();
    assert_eq!(borrowed.fluid().unwrap().amount_cache.to_bits(), cache_bits);
    assert_eq!(
        borrowed.fluid().unwrap().custom_drink_sound.as_deref(),
        Some("FactoryOnlyDrink")
    );
}

#[test]
fn typed_mutation_getters_and_physical_slots_retain_the_same_component() {
    let mut store = RuntimeComponents::new();
    let component = fluid_ref();
    store.push_ref_if_absent(Rc::clone(&component)).unwrap();
    let before = Rc::strong_count(&component);
    for slot in store.entry_slots() {
        match slot {
            RuntimeEntryView::Live {
                kind,
                component: actual,
            } => {
                assert_eq!(kind, ComponentType::FluidContainer);
                assert!(Rc::ptr_eq(actual, &component));
            }
            _ => panic!("unexpected encoded entry"),
        }
    }
    assert_eq!(
        Rc::strong_count(&component),
        before,
        "tracing views must not clone physical edges"
    );
    let mut guard = component.borrow_mut();
    match guard.typed_mut() {
        ComponentBodyMut::FluidContainer(fluid) => fluid.capacity = 23.25,
        _ => panic!("wrong typed view"),
    }
    let second = store
        .get_ref(ComponentType::FluidContainer)
        .unwrap()
        .unwrap();
    assert!(
        Rc::ptr_eq(&component, &second),
        "lookup does not borrow or copy the body"
    );
    assert!(Rc::ptr_eq(
        &store.get_ref_at(0).unwrap().unwrap(),
        &component
    ));
    assert_eq!(guard.kind(), ComponentType::FluidContainer);
    drop(guard);
    assert_eq!(second.borrow().fluid().unwrap().capacity, 23.25);
}

#[test]
fn storage_remove_is_pointer_based_unordered_and_leaves_owner_for_host() {
    let mut store = RuntimeComponents::new();
    let first = empty_ref(ComponentType::Lua);
    let middle = fluid_ref();
    let last = empty_ref(ComponentType::Parts);
    let actual_entity = Rc::new(RefCell::new(7u8));
    let owner = EntityOwner::from_ref(&actual_entity);
    assert!(middle.borrow_mut().replace_owner(Some(&owner)).is_none());
    for item in [&first, &middle, &last] {
        store.push_ref_if_absent(Rc::clone(item)).unwrap();
    }
    let same_type_foreign = fluid_ref();
    assert!(store.remove_exact_unordered(&same_type_foreign).is_none());
    let rejected = store
        .push_ref_if_absent(Rc::clone(&same_type_foreign))
        .unwrap_err();
    assert_eq!(
        rejected.error,
        RuntimeError::DuplicateType(ComponentType::FluidContainer.id())
    );
    assert!(Rc::ptr_eq(&rejected.component, &same_type_foreign));
    let removed = store.remove_exact_unordered(&middle).unwrap();
    assert!(Rc::ptr_eq(&removed, &middle));
    assert!(
        Rc::ptr_eq(&store.get_ref_at(1).unwrap().unwrap(), &last),
        "native unordered remove moves the final slot"
    );
    assert!(
        removed.borrow().owner_slot().unwrap().ptr_eq(&owner),
        "membership alone cannot invoke lifecycle/clear owner"
    );
    let mut retirement = store.take_entries();
    retirement.push_owner(removed.borrow_mut().replace_owner(None).unwrap());
    drop(retirement);
}

struct ReenterStore {
    store: Weak<RefCell<RuntimeComponents>>,
    component: Option<Weak<RefCell<RuntimeComponent>>>,
    successful_borrows: Rc<Cell<usize>>,
}

impl Drop for ReenterStore {
    fn drop(&mut self) {
        let store = self.store.upgrade().expect("probe store retained");
        assert!(
            store.try_borrow_mut().is_ok(),
            "payload dropped under store guard"
        );
        if let Some(component) = self.component.as_ref().and_then(Weak::upgrade) {
            assert!(
                component.try_borrow_mut().is_ok(),
                "owner dropped under component guard"
            );
        }
        self.successful_borrows
            .set(self.successful_borrows.get() + 1);
    }
}

#[test]
fn detached_owner_and_entries_retire_after_both_refcell_guards() {
    let store = Rc::new(RefCell::new(RuntimeComponents::new()));
    let component = fluid_ref();
    let observed = Rc::new(Cell::new(0));
    let entity = Rc::new(RefCell::new(ReenterStore {
        store: Rc::downgrade(&store),
        component: Some(Rc::downgrade(&component)),
        successful_borrows: Rc::clone(&observed),
    }));
    let owner = EntityOwner::from_ref(&entity);
    assert!(component.borrow_mut().replace_owner(Some(&owner)).is_none());
    store
        .borrow_mut()
        .push_ref_if_absent(Rc::clone(&component))
        .unwrap();
    drop(owner);
    drop(entity);
    let mut store_guard = store.borrow_mut();
    let mut component_guard = component.borrow_mut();
    let mut retirement = store_guard.take_entries();
    retirement.push_owner(component_guard.replace_owner(None).unwrap());
    assert_eq!(observed.get(), 0);
    assert!(store_guard.is_empty());
    drop(component_guard);
    drop(store_guard);
    drop(retirement);
    assert_eq!(observed.get(), 1);
    assert!(component.borrow().owner_slot().is_none());
}

#[test]
fn rejected_last_owner_is_returned_until_caller_releases_store_guard() {
    let store = Rc::new(RefCell::new(RuntimeComponents::new()));
    store.borrow_mut().push_ref_if_absent(fluid_ref()).unwrap();
    let observed = Rc::new(Cell::new(0));
    let incoming = fluid_ref();
    let entity = Rc::new(RefCell::new(ReenterStore {
        store: Rc::downgrade(&store),
        component: None,
        successful_borrows: Rc::clone(&observed),
    }));
    let owner = EntityOwner::from_ref(&entity);
    assert!(incoming.borrow_mut().replace_owner(Some(&owner)).is_none());
    drop(owner);
    drop(entity);
    let mut guard = store.borrow_mut();
    let rejected = guard.push_ref_if_absent(incoming).unwrap_err();
    assert_eq!(observed.get(), 0);
    assert_eq!(
        rejected.error,
        RuntimeError::DuplicateType(ComponentType::FluidContainer.id())
    );
    drop(guard);
    drop(rejected);
    assert_eq!(observed.get(), 1);
}

#[test]
fn borrowed_incoming_ref_is_returned_unchanged_without_borrow_panic() {
    let mut store = RuntimeComponents::new();
    let component = fluid_ref();
    let guard = component.borrow_mut();
    let rejected = store.push_ref_if_absent(Rc::clone(&component)).unwrap_err();
    assert!(matches!(
        rejected.error,
        RuntimeError::BorrowedComponent { .. }
    ));
    assert!(Rc::ptr_eq(&rejected.component, &component));
    assert!(store.is_empty());
    drop(guard);
    store.push_ref_if_absent(rejected.component).unwrap();
    assert!(Rc::ptr_eq(
        &store.get_ref_at(0).unwrap().unwrap(),
        &component
    ));
}

#[test]
fn live_load_keeps_allocation_partial_effects_and_original_absolute_reader() {
    let component = fluid_ref();
    {
        let mut state = component.borrow_mut();
        let fluid = state.fluid_mut().unwrap();
        fluid.capacity = 99.0;
        fluid.rain_catcher = 0.875;
        fluid.custom_drink_sound = Some("KeepUnsaved".into());
    }
    let mut store = RuntimeComponents::new();
    store.push_ref_if_absent(Rc::clone(&component)).unwrap();
    // Prefix + capacity header + truncated float: load resets several fields
    // before it fails. No transaction restores the old capacity.
    let bytes = [0xee, 0x00, 0x01, 0x12, 0x34];
    let mut reader = Reader::new(&bytes);
    reader.seek(1).unwrap();
    let mut retired = RetiredComponents::new();
    let error = component
        .borrow_mut()
        .load_payload_in_place(&mut reader, VERSION, &BasicContext::default(), &mut retired)
        .unwrap_err();
    assert!(matches!(error, RuntimeError::Codec(_)));
    assert_eq!(
        reader.pos(),
        3,
        "same caller reader retains failed cursor position"
    );
    assert_eq!(component.borrow().fluid().unwrap().capacity, 1.0);
    assert_eq!(component.borrow().fluid().unwrap().rain_catcher, 0.875);
    assert_eq!(
        component
            .borrow()
            .fluid()
            .unwrap()
            .custom_drink_sound
            .as_deref(),
        Some("KeepUnsaved")
    );
    assert!(Rc::ptr_eq(
        &store.get_ref_at(0).unwrap().unwrap(),
        &component
    ));
    // A valid payload remains at its absolute position; following caller
    // bytes were never hidden in a record-sized subreader.
    let mut input = Writer::new();
    input.i8(0x22);
    input.i16(1);
    input.f32(13.5);
    input.i8(0x33);
    let mut reader = Reader::new(&input.buf);
    reader.seek(1).unwrap();
    component
        .borrow_mut()
        .load_payload_in_place(&mut reader, VERSION, &BasicContext::default(), &mut retired)
        .unwrap();
    assert_eq!(reader.pos(), 7);
    assert_eq!(reader.i8().unwrap(), 0x33);
    assert_eq!(component.borrow().fluid().unwrap().capacity, 13.5);
    assert_eq!(component.borrow().fluid().unwrap().rain_catcher, 0.875);
}

fn encoded_fluid() -> EncodedComponent {
    let mut writer = Writer::new();
    FluidContainer {
        capacity: 7.125,
        rain_catcher: 0.25,
        ..Default::default()
    }
    .save(&mut writer);
    EncodedComponent::new(ComponentType::FluidContainer.id(), writer.buf, VERSION)
}

#[test]
fn encoded_import_promotes_once_in_the_same_slot_and_then_uses_live_state() {
    let mut store = RuntimeComponents::from_encoded_unowned(vec![encoded_fluid()]).unwrap();
    assert_eq!(
        store.presence(ComponentType::FluidContainer),
        EntryPresence::Encoded {
            world_version: VERSION
        }
    );
    assert!(matches!(
        store.get_ref(ComponentType::FluidContainer),
        Err(RuntimeError::NeedsPromotion { .. })
    ));
    assert!(matches!(
        store.try_values_snapshot(),
        Err(RuntimeError::NeedsPromotion { .. })
    ));
    let first = store
        .promote_encoded(ComponentType::FluidContainer, &BasicContext::default())
        .unwrap();
    assert!(first.promoted);
    assert_eq!(first.retired.len(), 1);
    assert_eq!(first.component.borrow().fluid().unwrap().capacity, 7.125);
    first.component.borrow_mut().fluid_mut().unwrap().capacity = 16.0;
    let second = store
        .promote_encoded(ComponentType::FluidContainer, &BasicContext::default())
        .unwrap();
    assert!(!second.promoted);
    assert!(second.retired.is_empty());
    assert!(Rc::ptr_eq(&first.component, &second.component));
    assert_eq!(
        second.component.borrow().fluid().unwrap().capacity,
        16.0,
        "repeat promotion cannot reload a shadow blob"
    );
    let mut expected = Writer::new();
    expected.i8(1);
    let start = expected.block_start();
    expected.i16(ComponentType::FluidContainer.id());
    first
        .component
        .borrow()
        .write_payload(&mut expected)
        .unwrap();
    expected.block_end(start);
    assert_eq!(store.save(VERSION).unwrap(), expected.buf);
}

#[test]
fn failed_promotion_preserves_original_encoded_authority_and_version() {
    let encoded = EncodedComponent::new(
        ComponentType::FluidContainer.id(),
        vec![0, 1, 0x12],
        VERSION,
    );
    let expected = encoded.clone();
    let mut store = RuntimeComponents::from_encoded_unowned(vec![encoded]).unwrap();
    let before = store.save(VERSION).unwrap();
    for _ in 0..2 {
        assert!(matches!(
            store.promote_encoded(ComponentType::FluidContainer, &BasicContext::default()),
            Err(RuntimeError::Codec(_))
        ));
        let slot = store.entry_slots().next().unwrap();
        match slot {
            RuntimeEntryView::Encoded(actual) => assert_eq!(actual, &expected),
            _ => panic!("failed decode published a live body"),
        }
        assert_eq!(store.save(VERSION).unwrap(), before);
    }
    assert!(matches!(
        store.save(VERSION + 1),
        Err(RuntimeError::EncodedVersionMismatch {
            stored: VERSION,
            requested: 250,
            ..
        })
    ));
    assert!(matches!(
        store.remove_type_unordered(ComponentType::FluidContainer),
        Err(RuntimeError::NeedsPromotion { .. })
    ));
    assert_eq!(store.len(), 1);
}

#[test]
fn unknown_raw_entries_copy_and_encode_exactly_without_fake_live_components() {
    let entries = vec![
        EncodedComponent::new(-7, vec![0xff, 0x11, 0, 0x22], VERSION),
        EncodedComponent::new(999, vec![1, 2, 3], VERSION),
    ];
    let store = RuntimeComponents::from_encoded_unowned(entries.clone()).unwrap();
    let copy = store.try_archive_copy_unowned().unwrap();
    assert!(store.try_data_eq(&copy).unwrap());
    let mut expected = Writer::new();
    expected.i8(2);
    for entry in &entries {
        let start = expected.block_start();
        expected.i16(entry.type_id());
        expected.bytes(entry.payload());
        expected.block_end(start);
    }
    assert_eq!(store.save(VERSION).unwrap(), expected.buf);
    assert_eq!(copy.save(VERSION).unwrap(), expected.buf);
    assert!(matches!(
        store.get_ref_at(0),
        Err(RuntimeError::NeedsPromotion { .. })
    ));
    let original_payload = match store.entry_slots().next().unwrap() {
        RuntimeEntryView::Encoded(e) => e.payload().as_ptr(),
        _ => unreachable!(),
    };
    let copied_payload = match copy.entry_slots().next().unwrap() {
        RuntimeEntryView::Encoded(e) => e.payload().as_ptr(),
        _ => unreachable!(),
    };
    assert_ne!(original_payload, copied_payload);
}

#[test]
fn adoption_rejection_returns_all_values_and_cannot_change_runtime_class() {
    let values = Components {
        list: vec![
            Component::FluidContainer(FluidContainer {
                capacity: 7.0,
                ..Default::default()
            }),
            Component::FluidContainer(FluidContainer {
                capacity: 9.0,
                ..Default::default()
            }),
        ],
    };
    let rejected = RuntimeComponents::adopt_values_unowned(values).unwrap_err();
    assert_eq!(rejected.error, RuntimeError::DuplicateType(2));
    assert_eq!(rejected.values.len(), 2);
    assert_eq!(
        rejected.values.list[1],
        Component::FluidContainer(FluidContainer {
            capacity: 9.0,
            ..Default::default()
        })
    );
    let invalid =
        RuntimeComponent::new_unowned(Component::Empty(ComponentType::FluidContainer)).unwrap_err();
    assert_eq!(
        invalid.error,
        RuntimeError::InvalidBody(ComponentType::FluidContainer)
    );
    assert!(matches!(
        invalid.component,
        Component::Empty(ComponentType::FluidContainer)
    ));
    let raw = vec![4, 7, 9];
    let allocation = raw.as_ptr();
    let opaque = RuntimeComponent::new_unowned(Component::Opaque {
        kind: ComponentType::FurnaceLogic,
        data: raw,
    })
    .unwrap_err();
    assert_eq!(
        opaque.error,
        RuntimeError::UnsupportedLive(ComponentType::FurnaceLogic)
    );
    match opaque.component {
        Component::Opaque { data, .. } => assert_eq!(data.as_ptr(), allocation),
        _ => unreachable!(),
    }
    let too_many: Vec<_> = (100..228)
        .map(|id| EncodedComponent::new(id, vec![1], VERSION))
        .collect();
    let rejected = RuntimeComponents::from_encoded_unowned(too_many).unwrap_err();
    assert_eq!(rejected.error, RuntimeError::TooManyComponents);
    assert_eq!(rejected.entries.len(), 128);
    let duplicate = RuntimeComponents::from_encoded_unowned(vec![encoded_fluid(), encoded_fluid()])
        .unwrap_err();
    assert_eq!(duplicate.error, RuntimeError::DuplicateType(2));
    assert_eq!(duplicate.entries.len(), 2);
}

#[test]
fn archive_copy_is_data_independent_unowned_and_fallible_on_body_borrows() {
    let component = fluid_ref();
    let actual_entity = Rc::new(RefCell::new(44u32));
    let owner = EntityOwner::from_ref(&actual_entity);
    assert!(component.borrow_mut().replace_owner(Some(&owner)).is_none());
    let mut store = RuntimeComponents::new();
    store.push_ref_if_absent(Rc::clone(&component)).unwrap();
    let copy = store.try_archive_copy_unowned().unwrap();
    let copied = copy.get_ref_at(0).unwrap().unwrap();
    assert!(!Rc::ptr_eq(&component, &copied));
    assert!(copied.borrow().owner_slot().is_none());
    assert!(
        store.try_data_eq(&copy).unwrap(),
        "owner reference is not archive data"
    );
    copied.borrow_mut().fluid_mut().unwrap().capacity = 123.0;
    assert_eq!(component.borrow().fluid().unwrap().capacity, 1.0);
    assert!(!store.try_data_eq(&copy).unwrap());
    let guard = component.borrow_mut();
    assert!(matches!(
        store.try_archive_copy_unowned(),
        Err(RuntimeError::BorrowedComponent { index: Some(0), .. })
    ));
    assert!(matches!(
        store.try_values_snapshot(),
        Err(RuntimeError::BorrowedComponent { index: Some(0), .. })
    ));
    assert!(matches!(
        store.try_data_eq(&store),
        Err(RuntimeError::BorrowedComponent { index: Some(0), .. })
    ));
    drop(guard);
    assert!(component.borrow_mut().replace_owner(None).is_some());
}

#[test]
fn data_equality_does_not_substitute_pointer_identity_for_nan_semantics() {
    let component = fluid_ref();
    component.borrow_mut().fluid_mut().unwrap().capacity = f32::NAN;
    let mut store = RuntimeComponents::new();
    store.push_ref_if_absent(component).unwrap();
    assert!(!store.try_data_eq(&store).unwrap());
}

#[test]
fn fallible_write_retains_completed_prefix_and_does_not_mutate_live_body() {
    let first = empty_ref(ComponentType::Lua);
    let second = fluid_ref();
    let mut store = RuntimeComponents::new();
    store.push_ref_if_absent(first).unwrap();
    store.push_ref_if_absent(Rc::clone(&second)).unwrap();
    let body_guard = second.borrow_mut();
    let mut writer = Writer::new();
    let error = store.write_to(&mut writer, VERSION).unwrap_err();
    assert!(matches!(
        error,
        RuntimeError::BorrowedComponent {
            index: Some(1),
            kind: ComponentType::FluidContainer
        }
    ));
    assert_eq!(
        &writer.buf[..7],
        &[2, 0, 0, 0, 2, 0, ComponentType::Lua.id() as u8]
    );
    assert_eq!(
        writer.buf.len(),
        13,
        "second block framing was written before body borrow failure"
    );
    assert_eq!(body_guard.fluid().unwrap().capacity, 1.0);
    drop(body_guard);
    assert!(store.save(VERSION).is_ok());
}
