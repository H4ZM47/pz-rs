use super::*;
use crate::native_list::{self as lists, NativeListRetired, ObservationStatus};
use crate::LuaObject;
use std::any::Any;

fn drain_all() {
    for _ in 0..12 {
        if lists::drain_retired() == 0 {
            break;
        }
    }
}

#[test]
fn native_list_registry_unavailable_is_deferred_and_retried_not_native_failure() {
    let mut retired = NativeListRetired::new();
    let list = COLLECTOR.with(|collector| {
        let _held = collector.borrow_mut();
        let list = lists::new_capacity(1, &mut retired)
            .expect("registry availability is not constructor failure");
        assert_eq!(lists::observe_list(&list), Ok(ObservationStatus::Deferred));
        assert_eq!(lists::retry_observation(), Ok(ObservationStatus::Deferred));
        list
    });
    assert_eq!(lists::retry_observation(), Ok(ObservationStatus::Complete));
    assert_eq!(lists::observe_list(&list), Ok(ObservationStatus::Complete));
    struct Conflict;
    impl TraceDescriptor<lists::NativeArrayList> for Conflict {
        fn trace(_: &lists::NativeArrayList, _: &mut TraceVisitor) {
            panic!("must not replace intrinsic descriptor");
        }
    }
    assert_eq!(
        register_existing::<_, Conflict>(&list),
        Err(RegistrationError::DescriptorConflict)
    );
    assert_eq!(lists::size(&list, &mut retired).unwrap(), 0);
}

struct Probe {
    drops: Rc<Cell<usize>>,
    recursive: bool,
}
impl LuaObject for Probe {
    fn class_name(&self) -> &str {
        "test.NativeListDrainProbe"
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}
impl Drop for Probe {
    fn drop(&mut self) {
        self.drops.set(self.drops.get() + 1);
        if self.recursive {
            assert_eq!(lists::drain_retired(), 0);
            assert_eq!(drain_retired_table_values(), 0);
            assert_eq!(collect_tables(), CollectionResult::Deferred);
            let mut next = NativeListRetired::new();
            let value = Value::Object(Rc::new(Probe {
                drops: self.drops.clone(),
                recursive: false,
            }));
            next.keep_value(&value);
        }
    }
}
#[test]
fn native_list_drain_uses_shared_guard_and_drops_one_generation_only() {
    drain_all();
    drain_retired_table_values();
    let drops = Rc::new(Cell::new(0));
    {
        let mut retired = NativeListRetired::new();
        let value = Value::Object(Rc::new(Probe {
            drops: drops.clone(),
            recursive: true,
        }));
        retired.keep_value(&value);
    }
    let outer = defer_collection();
    assert_eq!(lists::drain_retired(), 0);
    assert_eq!(drops.get(), 0);
    drop(outer);
    assert_eq!(lists::drain_retired(), 1);
    assert_eq!(drops.get(), 1);
    assert_eq!(lists::drain_retired(), 1);
    assert_eq!(drops.get(), 2);
    assert!(matches!(collect_pending(), CollectionResult::Collected(_)));
}

#[test]
fn shared_table_drain_blocks_native_release_and_native_hook_runs_without_request() {
    drain_all();
    drain_retired_table_values();
    let drops = Rc::new(Cell::new(0));
    let value = Value::Object(Rc::new(Probe {
        drops: drops.clone(),
        recursive: true,
    }));
    let mut retired = RetiredTableValues::new();
    retired.push(value);
    retire_table_values(retired);
    assert_eq!(drain_retired_table_values(), 1);
    assert_eq!(drops.get(), 1);
    // Even without a new GC request this safe tail drains the native queue.
    let _ = collect_pending();
    assert_eq!(drops.get(), 2);
    let drops = Rc::new(Cell::new(0));
    {
        let mut retired = NativeListRetired::new();
        retired.keep_value(&Value::Object(Rc::new(Probe {
            drops: drops.clone(),
            recursive: false,
        })));
    }
    COLLECTOR.with(|collector| collector.borrow_mut().active = true);
    assert_eq!(lists::drain_retired(), 0);
    assert_eq!(drops.get(), 0);
    COLLECTOR.with(|collector| collector.borrow_mut().active = false);
    let _ = collect_pending();
    assert_eq!(drops.get(), 1);
}
