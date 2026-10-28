//! Actual erased Rc slots retain the same ownership as their typed allocation.
use super::*;
use crate::{LuaObject, NativeFunction, TableRef};
use std::any::Any;
use std::cell::Cell;
use std::marker::PhantomData;

struct Target {
    values: Vec<Value>,
    number: i32,
    drops: Rc<Cell<usize>>,
}

impl Drop for Target {
    fn drop(&mut self) {
        self.drops.set(self.drops.get() + 1);
    }
}

struct TargetTrace;
impl TraceDescriptor<Target> for TargetTrace {
    fn trace(target: &Target, visitor: &mut TraceVisitor) {
        for value in &target.values {
            visitor.value(value);
        }
    }
    fn detach(target: &mut Target) -> Option<Box<dyn Any>> {
        Some(Box::new(std::mem::take(&mut target.values)))
    }
}

struct Carrier<T> {
    // Each element is an actual independent owning slot, including duplicates.
    slots: Vec<Rc<dyn Any>>,
    marker: PhantomData<fn() -> T>,
}
impl<T: 'static> TracePayload for Carrier<T> {
    fn trace(&self, visitor: &mut TraceVisitor) {
        for slot in &self.slots {
            visitor.owner_erased::<T>(slot);
        }
    }
    fn detach(&mut self) -> Option<Box<dyn Any>> {
        Some(Box::new(std::mem::take(&mut self.slots)))
    }
}

fn callable<T: 'static>(slots: Vec<Rc<dyn Any>>) -> Rc<NativeFunction> {
    NativeFunction::new_traced_capture(
        "erased-owner",
        Carrier::<T> {
            slots,
            marker: PhantomData,
        },
        |_, _| Ok(0),
    )
}

struct Fixture {
    target: Rc<RefCell<Target>>,
    table: Weak<RefCell<Table>>,
    native: Weak<NativeFunction>,
    drops: Rc<Cell<usize>>,
}

fn cycle<T: 'static>(registered: bool) -> Fixture {
    let table = TableRef::new();
    table.set_str("kept", 401);
    let drops = Rc::new(Cell::new(0));
    let target = Rc::new(RefCell::new(Target {
        values: vec![table.clone().into()],
        number: 409,
        drops: drops.clone(),
    }));
    if registered {
        assert_eq!(register_existing::<_, TargetTrace>(&target), Ok(true));
    }
    let native = callable::<T>(vec![target.clone(), target.clone()]);
    table.set_str("callback", Value::Native(native.clone()));
    Fixture {
        target,
        table: Rc::downgrade(&table.0),
        native: Rc::downgrade(&native),
        drops,
    }
}

fn collect() {
    assert!(matches!(collect_tables(), CollectionResult::Collected(_)));
}
fn assert_live(target: &RefCell<Target>, table: &Weak<RefCell<Table>>) {
    let target = target.borrow();
    assert_eq!(target.number, 409);
    assert_eq!(target.values.len(), 1);
    assert_eq!(
        TableRef(table.upgrade().unwrap()).get_str("kept").as_num(),
        Some(401.0)
    );
}

#[test]
fn duplicate_erased_slots_keep_typed_and_erased_roots_then_collect_actual_cycle() {
    let f = cycle::<Target>(true);
    let weak = Rc::downgrade(&f.target);
    let count = Rc::strong_count(&f.target);
    collect();
    assert_eq!(Rc::strong_count(&f.target), count);
    assert_live(&f.target, &f.table);
    let erased: Rc<dyn Any> = f.target.clone();
    drop(f.target);
    collect();
    assert_live(
        erased.as_ref().downcast_ref::<RefCell<Target>>().unwrap(),
        &f.table,
    );
    drop(erased);
    collect();
    assert!(weak.upgrade().is_none());
    assert!(f.native.upgrade().is_none());
    assert!(f.table.upgrade().is_none());
    assert_eq!(f.drops.get(), 1);
}

#[test]
fn equal_targets_are_distinct_and_missing_actual_registration_can_retry() {
    let f = cycle::<Target>(false);
    let weak = Rc::downgrade(&f.target);
    drop(f.target);
    collect();
    let target = weak.upgrade().expect("raw erased target remains a root");
    assert_live(&target, &f.table);
    assert_eq!(register_existing::<_, TargetTrace>(&target), Ok(true));
    // A separate equal-valued target must remain its own allocation/node.
    let second = Rc::new(RefCell::new(Target {
        values: vec![TableRef(f.table.upgrade().unwrap()).into()],
        number: 409,
        drops: f.drops.clone(),
    }));
    assert_eq!(register_existing::<_, TargetTrace>(&second), Ok(true));
    TableRef(f.table.upgrade().unwrap()).set_str(
        "other",
        Value::Native(callable::<Target>(vec![second.clone()])),
    );
    let second_weak = Rc::downgrade(&second);
    drop(target);
    collect();
    assert_live(&second, &f.table);
    assert!(weak.upgrade().is_some());
    drop(second);
    collect();
    assert!(weak.upgrade().is_none());
    assert!(second_weak.upgrade().is_none());
    assert!(f.table.upgrade().is_none());
    assert_eq!(f.drops.get(), 2);
}

#[test]
fn wrong_payload_type_and_owning_wrappers_stay_opaque() {
    struct Other;
    let f = cycle::<Other>(true);
    let weak = Rc::downgrade(&f.target);
    drop(f.target);
    collect();
    let target = weak
        .upgrade()
        .expect("wrong descriptor target is conservative");
    assert_live(&target, &f.table);
    target.borrow_mut().values.clear();
    drop(target);
    collect();
    assert!(weak.upgrade().is_none());
    assert_eq!(f.drops.get(), 1);

    struct Wrapper(Rc<RefCell<Target>>);
    let f = cycle::<Target>(true);
    let wrapper = Rc::new(Wrapper(f.target.clone()));
    let wrapper_weak = Rc::downgrade(&wrapper);
    // Replace the existing callable with one owning a genuine wrapper,
    // whose internal target is not this erased slot's physical allocation.
    TableRef(f.table.upgrade().unwrap())
        .set_str("callback", Value::Native(callable::<Target>(vec![wrapper])));
    drop(f.target);
    collect();
    let wrapper = wrapper_weak
        .upgrade()
        .expect("untraced wrapper edge remains a root");
    assert_live(&wrapper.0, &f.table);
    wrapper.0.borrow_mut().values.clear();
    drop(wrapper);
    collect();
    assert!(wrapper_weak.upgrade().is_none());
    assert_eq!(f.drops.get(), 1);
}

struct NoTrace;
impl<T: 'static> TraceDescriptor<T> for NoTrace {
    fn trace(_: &T, _: &mut TraceVisitor) {}
}

#[test]
fn erased_vm_allocations_do_not_become_host_edges() {
    let table = TableRef::new();
    table.set_str("kept", 419);
    assert_eq!(
        register_existing::<_, NoTrace>(&table.0),
        Err(RegistrationError::VmPayloadReserved)
    );
    let native = callable::<Table>(vec![table.0.clone(), table.0.clone()]);
    table.set_str("callback", Value::Native(native));
    let weak = Rc::downgrade(&table.0);
    drop(table);
    collect();
    let table = TableRef(weak.upgrade().expect("erased VM slot is an opaque root"));
    assert_eq!(table.get_str("kept").as_num(), Some(419.0));
    table.set_str("callback", Value::Nil);
    drop(table);
    collect();
    assert!(weak.upgrade().is_none());

    let table = TableRef::new();
    table.set_str("kept", 421);
    let slot = UpValue::closed(table.clone().into());
    assert_eq!(
        register_existing::<_, NoTrace>(&slot),
        Err(RegistrationError::VmPayloadReserved)
    );
    table.set_str(
        "callback",
        Value::Native(callable::<UpValue>(vec![slot.clone(), slot.clone()])),
    );
    let slot_weak = Rc::downgrade(&slot);
    let table_weak = Rc::downgrade(&table.0);
    drop(slot);
    drop(table);
    collect();
    let slot = slot_weak
        .upgrade()
        .expect("erased upvalue slot remains external");
    assert!(
        matches!(&*slot.borrow(), UpValue::Closed(Value::Table(t)) if t.get_str("kept").as_num()==Some(421.0))
    );
    *slot.borrow_mut() = UpValue::Closed(Value::Nil);
    drop(slot);
    collect();
    assert!(slot_weak.upgrade().is_none());
    assert!(table_weak.upgrade().is_none());
}

impl LuaObject for RefCell<Target> {
    fn class_name(&self) -> &str {
        panic!("GC called LuaObject::class_name")
    }
    fn to_lua_string(&self) -> String {
        panic!("GC called LuaObject::to_lua_string")
    }
    fn as_any(&self) -> &dyn Any {
        panic!("GC called LuaObject::as_any")
    }
    fn super_classes(&self) -> &[&'static str] {
        panic!("GC called LuaObject::super_classes")
    }
}
struct TargetUserdataTrace;
impl UserdataDescriptor<RefCell<Target>> for TargetUserdataTrace {
    fn trace(payload: &RefCell<Target>, visitor: &mut TraceVisitor) -> Result<(), TraceDeferred> {
        let payload = payload.try_borrow().map_err(|_| TraceDeferred)?;
        TargetTrace::trace(&payload, visitor);
        Ok(())
    }
}

#[test]
fn erased_userdata_allocation_does_not_map_to_a_registered_userdata_node() {
    let f = cycle::<Target>(false);
    assert_eq!(
        register_userdata::<_, TargetUserdataTrace>(&f.target),
        Ok(true)
    );
    assert_eq!(
        register_existing::<_, TargetTrace>(&f.target),
        Err(RegistrationError::AllocationKindConflict)
    );
    let weak = Rc::downgrade(&f.target);
    drop(f.target);
    collect();
    let target = weak
        .upgrade()
        .expect("host-only erased slot cannot infer userdata edge");
    assert_live(&target, &f.table);
    target.borrow_mut().values.clear();
    drop(target);
    collect();
    assert!(weak.upgrade().is_none());
    assert!(f.table.upgrade().is_none());
    assert_eq!(f.drops.get(), 1);
}

struct Retired {
    target: Weak<RefCell<Target>>,
    checked: Rc<Cell<bool>>,
}
impl LuaObject for Retired {
    fn class_name(&self) -> &str {
        "AuthoredErasedDropProbe"
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}
impl Drop for Retired {
    fn drop(&mut self) {
        let target = self
            .target
            .upgrade()
            .expect("actual collector pin still exists");
        target
            .try_borrow_mut()
            .expect("all target guards ended")
            .number = 431;
        assert_eq!(collect_tables(), CollectionResult::Deferred);
        let fresh = TableRef::new();
        fresh.set_str("self", fresh.clone());
        self.checked.set(true);
    }
}

#[test]
fn held_actual_target_borrow_defers_whole_pass_and_retired_values_reenter_without_guards() {
    let f = cycle::<Target>(true);
    let checked = Rc::new(Cell::new(false));
    f.target
        .borrow_mut()
        .values
        .push(Value::Object(Rc::new(Retired {
            target: Rc::downgrade(&f.target),
            checked: checked.clone(),
        })));
    let unrelated = TableRef::new();
    unrelated.set_str("self", unrelated.clone());
    let unrelated_weak = Rc::downgrade(&unrelated.0);
    drop(unrelated);
    let guard = f.target.borrow_mut();
    assert_eq!(collect_tables(), CollectionResult::Deferred);
    assert!(unrelated_weak.upgrade().is_some());
    assert!(!checked.get());
    drop(guard);
    drop(f.target);
    collect();
    assert!(unrelated_weak.upgrade().is_none());
    assert!(checked.get());
    assert_eq!(f.drops.get(), 1);
    assert!(matches!(collect_pending(), CollectionResult::Collected(_)));
    assert_eq!(collect_pending(), CollectionResult::NotRequested);
}
