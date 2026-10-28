use super::*;
use crate::gc::{
    collect_pending, collect_tables, register_existing, CollectionResult, TraceDescriptor,
};
use crate::table::Table;
use crate::vm::{Coroutine, LuaClosure, NativeFunction, Prototype, UpValue};
use crate::{LuaError, TableRef, Thread, Value};
use std::any::Any;
use std::cell::Cell;

struct Backing {
    values: Vec<Value>,
    stamp: i32,
    drops: Rc<Cell<usize>>,
}

impl Drop for Backing {
    fn drop(&mut self) {
        self.drops.set(self.drops.get() + 1);
    }
}

struct BackingTrace;
impl TraceDescriptor<Backing> for BackingTrace {
    fn trace(payload: &Backing, visitor: &mut TraceVisitor) {
        for value in &payload.values {
            visitor.value(value);
        }
    }

    fn detach(payload: &mut Backing) -> Option<Box<dyn Any>> {
        Some(Box::new(std::mem::take(&mut payload.values)))
    }
}

struct Object {
    first: Rc<RefCell<Backing>>,
    second: Rc<RefCell<Backing>>,
    inspect: RefCell<()>,
    drops: Rc<Cell<usize>>,
}

// None of these virtual methods is safe to call from collector bookkeeping.
macro_rules! opaque_methods {
    () => {
        fn class_name(&self) -> &str {
            panic!("GC called class_name")
        }
        fn to_lua_string(&self) -> String {
            panic!("GC called to_lua_string")
        }
        fn as_any(&self) -> &dyn Any {
            panic!("GC called as_any")
        }
        fn super_classes(&self) -> &[&'static str] {
            panic!("GC called super_classes")
        }
    };
}

impl LuaObject for Object {
    opaque_methods!();
}
impl Drop for Object {
    fn drop(&mut self) {
        self.drops.set(self.drops.get() + 1);
    }
}

struct ObjectTrace;
impl UserdataDescriptor<Object> for ObjectTrace {
    fn trace(payload: &Object, visitor: &mut TraceVisitor) -> Result<(), TraceDeferred> {
        let _inspect = payload.inspect.try_borrow().map_err(|_| TraceDeferred)?;
        visitor.owner(&payload.first);
        visitor.owner(&payload.second);
        Ok(())
    }
}

struct ConflictingTrace;
impl UserdataDescriptor<Object> for ConflictingTrace {
    fn trace(_: &Object, _: &mut TraceVisitor) -> Result<(), TraceDeferred> {
        panic!("rejected descriptor replaced actual description");
    }
}

struct Fixture {
    object: Rc<Object>,
    table: Weak<RefCell<Table>>,
    backing: Weak<RefCell<Backing>>,
    object_drops: Rc<Cell<usize>>,
    backing_drops: Rc<Cell<usize>>,
}

fn cycle(host_registered: bool, object_registered: bool) -> Fixture {
    let table = TableRef::new();
    table.set_str("kept", 307);
    let object_drops = Rc::new(Cell::new(0));
    let backing_drops = Rc::new(Cell::new(0));
    let backing = Rc::new(RefCell::new(Backing {
        values: vec![table.clone().into()],
        stamp: 311,
        drops: backing_drops.clone(),
    }));
    if host_registered {
        assert_eq!(register_existing::<_, BackingTrace>(&backing), Ok(true));
    }
    let object = Rc::new(Object {
        first: backing.clone(),
        second: backing.clone(),
        inspect: RefCell::new(()),
        drops: object_drops.clone(),
    });
    if object_registered {
        assert_eq!(register_userdata::<_, ObjectTrace>(&object), Ok(true));
    }
    // Three real owning slots, including a key, target the same allocation.
    table.rawset(Value::Object(object.clone()), Value::Object(object.clone()));
    table.set_str("object", Value::Object(object.clone()));
    Fixture {
        object,
        table: Rc::downgrade(&table.0),
        backing: Rc::downgrade(&backing),
        object_drops,
        backing_drops,
    }
}

fn collect() {
    assert!(matches!(collect_tables(), CollectionResult::Collected(_)));
}

fn assert_live(object: &Object, table: &Weak<RefCell<Table>>) {
    assert_eq!(object.first.borrow().stamp, 311);
    assert!(Rc::ptr_eq(&object.first, &object.second));
    assert_eq!(object.first.borrow().values.len(), 1);
    assert_eq!(
        TableRef(table.upgrade().unwrap()).get_str("kept").as_num(),
        Some(307.0)
    );
}

#[test]
fn actual_typed_and_erased_roots_preserve_slots_then_collect_shared_backing_cycle() {
    let f = cycle(true, true);
    let weak = Rc::downgrade(&f.object);
    let strong_before = Rc::strong_count(&f.object);
    for _ in 0..100 {
        assert_eq!(register_userdata::<_, ObjectTrace>(&f.object), Ok(false));
    }
    assert_eq!(Rc::strong_count(&f.object), strong_before);
    assert_eq!(
        register_userdata::<_, ConflictingTrace>(&f.object),
        Err(RegistrationError::DescriptorConflict)
    );
    collect();
    assert_live(&f.object, &f.table);
    let erased: Rc<dyn LuaObject> = f.object.clone();
    drop(f.object);
    collect();
    assert_eq!(f.object_drops.get(), 0);
    let value = Value::Object(erased);
    collect();
    assert_live(&weak.upgrade().unwrap(), &f.table);
    drop(value);
    collect();
    assert!(weak.upgrade().is_none());
    assert!(f.table.upgrade().is_none());
    assert!(f.backing.upgrade().is_none());
    assert_eq!(f.object_drops.get(), 1);
    assert_eq!(f.backing_drops.get(), 1);
}

#[test]
fn distinct_userdata_wrappers_of_one_backing_are_distinct_physical_nodes() {
    let f = cycle(true, true);
    let second = Rc::new(Object {
        first: f.object.first.clone(),
        second: f.object.second.clone(),
        inspect: RefCell::new(()),
        drops: f.object_drops.clone(),
    });
    assert_eq!(register_userdata::<_, ObjectTrace>(&second), Ok(true));
    let second_weak = Rc::downgrade(&second);
    TableRef(f.table.upgrade().unwrap()).set_str("second", Value::Object(second.clone()));
    drop(f.object);
    collect();
    assert_live(&second, &f.table);
    drop(second);
    collect();
    assert!(second_weak.upgrade().is_none());
    assert_eq!(f.object_drops.get(), 2);
    assert_eq!(f.backing_drops.get(), 1);
}

#[test]
fn either_missing_registration_stays_conservative_until_typed_retry() {
    for (host, userdata) in [(true, false), (false, true)] {
        let f = cycle(host, userdata);
        let weak = Rc::downgrade(&f.object);
        drop(f.object);
        collect();
        let object = weak.upgrade().expect("partial graph must remain retained");
        assert_live(&object, &f.table);
        if !host {
            assert_eq!(
                register_existing::<_, BackingTrace>(&object.first),
                Ok(true)
            );
        }
        if !userdata {
            assert_eq!(register_userdata::<_, ObjectTrace>(&object), Ok(true));
        }
        drop(object);
        collect();
        assert!(weak.upgrade().is_none());
        assert!(f.table.upgrade().is_none());
        assert_eq!(f.object_drops.get(), 1);
    }
}

#[test]
fn registry_borrow_rejects_registration_conservatively_and_retry_is_idempotent() {
    let f = cycle(true, false);
    COLLECTOR.with(|registry| {
        let _borrow = registry.borrow();
        assert_eq!(
            register_userdata::<_, ObjectTrace>(&f.object),
            Err(RegistrationError::RegistryUnavailable)
        );
    });
    collect();
    assert_live(&f.object, &f.table);
    assert_eq!(register_userdata::<_, ObjectTrace>(&f.object), Ok(true));
    assert_eq!(register_userdata::<_, ObjectTrace>(&f.object), Ok(false));
    drop(f.object);
    collect();
    assert_eq!(f.object_drops.get(), 1);
}

#[test]
fn interior_and_backing_borrows_defer_entire_pass_without_clearing_unrelated_cycle() {
    let f = cycle(true, true);
    let unrelated = TableRef::new();
    unrelated.set_str("self", unrelated.clone());
    let unrelated_weak = Rc::downgrade(&unrelated.0);
    drop(unrelated);
    let borrow = f.object.inspect.borrow_mut();
    assert_eq!(collect_tables(), CollectionResult::Deferred);
    assert!(unrelated_weak.upgrade().is_some());
    assert_live(&f.object, &f.table);
    drop(borrow);
    let borrow = f.object.first.borrow();
    assert_eq!(collect_pending(), CollectionResult::Deferred);
    assert!(unrelated_weak.upgrade().is_some());
    drop(borrow);
    collect();
    assert!(unrelated_weak.upgrade().is_none());
    assert_live(&f.object, &f.table);
    drop(f.object);
    collect();
    assert_eq!(f.object_drops.get(), 1);
}

impl LuaObject for RefCell<Backing> {
    opaque_methods!();
}
struct BackingAsUserdata;
impl UserdataDescriptor<RefCell<Backing>> for BackingAsUserdata {
    fn trace(payload: &RefCell<Backing>, visitor: &mut TraceVisitor) -> Result<(), TraceDeferred> {
        let payload = payload.try_borrow().map_err(|_| TraceDeferred)?;
        BackingTrace::trace(&payload, visitor);
        Ok(())
    }
}

#[test]
fn one_physical_allocation_cannot_be_both_host_and_userdata_in_either_order() {
    for host_first in [true, false] {
        let backing = Rc::new(RefCell::new(Backing {
            values: vec![],
            stamp: 331,
            drops: Rc::new(Cell::new(0)),
        }));
        if host_first {
            assert_eq!(register_existing::<_, BackingTrace>(&backing), Ok(true));
            assert_eq!(
                register_userdata::<_, BackingAsUserdata>(&backing),
                Err(RegistrationError::AllocationKindConflict)
            );
            // A rejected Object alias must not be mapped to Host by address.
            let table = TableRef::new();
            table.set_str("alias", Value::Object(backing.clone()));
            backing.borrow_mut().values.push(table.clone().into());
            let weak = Rc::downgrade(&backing);
            drop(table);
            drop(backing);
            collect();
            let backing = weak
                .upgrade()
                .expect("rejected alias must remain an external root");
            assert_eq!(backing.borrow().values.len(), 1);
            backing.borrow_mut().values.clear();
            assert_eq!(backing.borrow().stamp, 331);
        } else {
            assert_eq!(
                register_userdata::<_, BackingAsUserdata>(&backing),
                Ok(true)
            );
            assert_eq!(
                register_existing::<_, BackingTrace>(&backing),
                Err(RegistrationError::AllocationKindConflict)
            );
            collect();
            assert_eq!(backing.borrow().stamp, 331);
        }
        collect();
    }
}

struct Children(Rc<Object>, Rc<Object>);
impl LuaObject for Children {
    opaque_methods!();
}
struct ChildrenTrace;
impl UserdataDescriptor<Children> for ChildrenTrace {
    fn trace(payload: &Children, visitor: &mut TraceVisitor) -> Result<(), TraceDeferred> {
        visitor.userdata(&payload.0);
        // The erased handle still identifies exactly the same physical slot.
        visitor.userdata(&payload.1);
        Ok(())
    }
}

#[test]
fn explicit_userdata_slots_count_multiplicity_without_transitive_edges() {
    let f = cycle(true, true);
    let children = Rc::new(Children(f.object.clone(), f.object.clone()));
    assert_eq!(register_userdata::<_, ChildrenTrace>(&children), Ok(true));
    let weak = Rc::downgrade(&children);
    TableRef(f.table.upgrade().unwrap()).set_str("children", Value::Object(children.clone()));
    drop(f.object);
    collect();
    assert_live(&children.0, &f.table);
    drop(children);
    collect();
    assert!(weak.upgrade().is_none());
    assert_eq!(f.object_drops.get(), 1);
    assert_eq!(f.backing_drops.get(), 1);
}

struct Retired {
    owner: Rc<RefCell<Backing>>,
    checked: Rc<Cell<bool>>,
}
impl LuaObject for Retired {
    opaque_methods!();
}
struct RetiredTrace;
impl UserdataDescriptor<Retired> for RetiredTrace {
    fn trace(payload: &Retired, visitor: &mut TraceVisitor) -> Result<(), TraceDeferred> {
        visitor.owner(&payload.owner);
        Ok(())
    }
}
impl Drop for Retired {
    fn drop(&mut self) {
        self.owner
            .try_borrow_mut()
            .expect("host guards end before userdata Drop")
            .stamp = 337;
        assert_eq!(collect_tables(), CollectionResult::Deferred);
        let fresh = TableRef::new();
        fresh.set_str("self", fresh.clone());
        self.checked.set(true);
    }
}

#[test]
fn retired_userdata_drop_reenters_after_guards_end_and_leaves_next_request_pending() {
    let f = cycle(true, true);
    let checked = Rc::new(Cell::new(false));
    let probe = Rc::new(Retired {
        owner: f.object.first.clone(),
        checked: checked.clone(),
    });
    assert_eq!(register_userdata::<_, RetiredTrace>(&probe), Ok(true));
    TableRef(f.table.upgrade().unwrap()).set_str("probe", Value::Object(probe.clone()));
    drop(probe);
    drop(f.object);
    collect();
    assert!(checked.get());
    assert!(f.backing.upgrade().is_none());
    assert!(matches!(collect_pending(), CollectionResult::Collected(_)));
    assert_eq!(collect_pending(), CollectionResult::NotRequested);
}

struct ReservedTrace;
macro_rules! reserve_vm_shape {
    ($ty:ty) => {
        impl LuaObject for $ty {
            opaque_methods!();
        }
        impl UserdataDescriptor<$ty> for ReservedTrace {
            fn trace(_: &$ty, _: &mut TraceVisitor) -> Result<(), TraceDeferred> {
                panic!("reserved VM shape was traced as userdata");
            }
        }
    };
}
reserve_vm_shape!(RefCell<Table>);
reserve_vm_shape!(RefCell<UpValue>);
reserve_vm_shape!(LuaClosure);
reserve_vm_shape!(NativeFunction);

#[test]
fn standard_and_manual_vm_shapes_are_reserved_before_type_erasure() {
    let table = TableRef::new();
    let raw_table = Rc::new(RefCell::new(Table::default()));
    for table in [&table.0, &raw_table] {
        assert_eq!(
            register_userdata::<_, ReservedTrace>(table),
            Err(RegistrationError::VmPayloadReserved)
        );
    }
    let upvalue = UpValue::closed(Value::Nil);
    let raw_upvalue = Rc::new(RefCell::new(UpValue::Closed(Value::Nil)));
    for upvalue in [&upvalue, &raw_upvalue] {
        assert_eq!(
            register_userdata::<_, ReservedTrace>(upvalue),
            Err(RegistrationError::VmPayloadReserved)
        );
    }
    let closure = LuaClosure::new(Rc::new(Prototype::default()), table.clone());
    let raw_closure = Rc::new(LuaClosure {
        prototype: Rc::new(Prototype::default()),
        env: RefCell::new(table),
        upvalues: vec![],
        debug_name: RefCell::new(None),
    });
    for closure in [&closure, &raw_closure] {
        assert_eq!(
            register_userdata::<_, ReservedTrace>(closure),
            Err(RegistrationError::VmPayloadReserved)
        );
    }
    let native = NativeFunction::new("reserved", |_| Ok(0));
    let raw_native = Rc::new(NativeFunction {
        name: Rc::from("raw"),
        f: Box::new(|_| Ok(0)),
    });
    for native in [&native, &raw_native] {
        assert_eq!(
            register_userdata::<_, ReservedTrace>(native),
            Err(RegistrationError::VmPayloadReserved)
        );
    }
    collect();
}

#[test]
fn successful_and_error_values_survive_outer_call_pending_collection() {
    for error in [false, true] {
        let f = cycle(true, true);
        let weak = Rc::downgrade(&f.object);
        let callback_weak = weak.clone();
        let native = NativeFunction::new("return-userdata", move |call| {
            let object = callback_weak.upgrade().unwrap();
            crate::gc::request_collection();
            let value = Value::Object(object);
            if error {
                Err(LuaError::Kahlua(value))
            } else {
                Ok(call.push(value))
            }
        });
        drop(f.object);
        let mut thread = Thread::new(TableRef::new());
        if error {
            // The direct-native entry deliberately wraps exceptions as Java
            // strings. An interpreted call preserves the Kahlua Value error.
            thread
                .globals()
                .set_str("return_userdata", Value::Native(native));
            let function = crate::stdlib::load_string(
                "return return_userdata()",
                None,
                thread.globals().clone(),
            )
            .unwrap();
            let Err((message, _, error)) = thread.pcall_ex(&function, &[]) else {
                panic!("expected Kahlua error");
            };
            assert!(matches!(message, Value::Object(_)));
            assert_live(&weak.upgrade().unwrap(), &f.table);
            assert_eq!(collect_pending(), CollectionResult::NotRequested);
            drop(message);
            drop(error);
        } else {
            let Ok(values) = thread.call(&Value::Native(native), &[]) else {
                panic!("call failed");
            };
            assert!(matches!(values[0], Value::Object(_)));
            assert_live(&weak.upgrade().unwrap(), &f.table);
            assert_eq!(collect_pending(), CollectionResult::NotRequested);
            drop(values);
        }
        drop(thread);
        collect();
        assert!(weak.upgrade().is_none());
        assert_eq!(f.object_drops.get(), 1);
    }
}

#[test]
fn opaque_native_prototype_and_suspended_coroutine_slots_remain_roots() {
    for kind in 0..3 {
        let f = cycle(true, true);
        let object = f.object;
        let weak = Rc::downgrade(&object);
        let root: Box<dyn Any> = match kind {
            0 => Box::new(NativeFunction::new("opaque", move |_| {
                let _ = &object;
                Ok(0)
            })),
            1 => Box::new(Rc::new(Prototype {
                constants: vec![Value::Object(object)],
                ..Prototype::default()
            })),
            _ => Box::new(Rc::new(RefCell::new(Coroutine {
                stack: vec![Value::Object(object)],
                top: 1,
                ..Coroutine::default()
            }))),
        };
        collect();
        assert_live(&weak.upgrade().unwrap(), &f.table);
        drop(root);
        collect();
        assert!(weak.upgrade().is_none());
        assert_eq!(f.object_drops.get(), 1);
    }
}

#[test]
fn registry_growth_is_amortized_duplicates_are_free_and_dead_headers_are_bounded() {
    std::thread::spawn(|| {
        let mut roots: Vec<_> = (0..1024).map(|_| cycle(true, true).object).collect();
        loop {
            let full = COLLECTOR.with(|registry| {
                let registry = registry.borrow();
                registry.userdata.len() == registry.userdata.capacity()
            });
            if full {
                break;
            }
            roots.push(cycle(true, true).object);
        }
        let capacity = COLLECTOR.with(|r| r.borrow().userdata.capacity());
        // Break one cycle while retaining every other live allocation.
        let retired = roots.pop().unwrap();
        retired.first.borrow_mut().values.clear();
        drop(retired);
        let temporary = cycle(true, true);
        COLLECTOR.with(|r| {
            let registry = r.borrow();
            assert!(registry.userdata.capacity() >= capacity + roots.len());
            assert_eq!(registry.userdata.len(), capacity);
        });
        let before = COLLECTOR.with(|r| r.borrow().userdata.len());
        for _ in 0..1000 {
            assert_eq!(register_userdata::<_, ObjectTrace>(&roots[0]), Ok(false));
        }
        assert_eq!(COLLECTOR.with(|r| r.borrow().userdata.len()), before);
        drop(temporary.object);
        drop(roots);
        collect();
        COLLECTOR.with(|r| {
            let registry = r.borrow();
            assert!(registry.userdata.is_empty());
            assert!(registry.userdata_indices.is_empty());
        });
    })
    .join()
    .unwrap();
    // Fresh TLS avoids making a small-header bound depend on a prior large capacity.
    std::thread::spawn(|| {
        for _ in 0..10_000 {
            let f = cycle(true, true);
            f.object.first.borrow_mut().values.clear();
            drop(f.object);
        }
        COLLECTOR.with(|r| {
            let registry = r.borrow();
            assert!(registry.userdata.len() <= 4);
            assert!(registry.userdata_indices.len() <= 4);
        });
    })
    .join()
    .unwrap();
}

#[test]
fn userdata_registration_during_registry_tls_teardown_is_unavailable_without_callbacks() {
    use std::sync::atomic::{AtomicBool, Ordering};
    static CHECKED: AtomicBool = AtomicBool::new(false);
    struct Late;
    impl Drop for Late {
        fn drop(&mut self) {
            let backing = Rc::new(RefCell::new(Backing {
                values: vec![],
                stamp: 347,
                drops: Rc::new(Cell::new(0)),
            }));
            assert_eq!(
                register_userdata::<_, BackingAsUserdata>(&backing),
                Err(RegistrationError::RegistryUnavailable)
            );
            CHECKED.store(true, Ordering::SeqCst);
        }
    }
    std::thread_local! { static LATE: Late = const { Late }; }
    std::thread::spawn(|| {
        // TLS destructors run in reverse initialization order.
        LATE.with(|_| {});
        let f = cycle(true, true);
        f.object.first.borrow_mut().values.clear();
    })
    .join()
    .unwrap();
    assert!(CHECKED.load(Ordering::SeqCst));
}
