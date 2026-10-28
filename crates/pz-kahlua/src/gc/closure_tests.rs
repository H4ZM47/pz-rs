use super::*;
use crate::vm::{Coroutine, LuaError, Prototype};
use crate::{stdlib, LuaObject, NativeFunction, TableRef, Thread};
use std::any::Any;
use std::cell::Cell;

fn collected() -> CollectionStats {
    match collect_tables() {
        CollectionResult::Collected(stats) => stats,
        other => panic!("collection did not complete: {other:?}"),
    }
}

fn closure(env: TableRef, upvalues: Vec<UpValueRef>) -> Rc<LuaClosure> {
    LuaClosure::with_upvalues(Rc::new(Prototype::default()), env, upvalues)
}

fn environment_cycle() -> (Weak<RefCell<Table>>, Weak<LuaClosure>) {
    let env = TableRef::new();
    let function = LuaClosure::new(Rc::new(Prototype::default()), env.clone());
    env.set_str("function", Value::Closure(function.clone()));
    (Rc::downgrade(&env.0), Rc::downgrade(&function))
}

#[test]
fn environment_and_closure_cycles_are_reclaimed() {
    let (env, function) = environment_cycle();
    assert!(env.upgrade().is_some());
    assert!(function.upgrade().is_some());
    assert_eq!(collected().tables_cleared, 1);
    assert!(env.upgrade().is_none());
    assert!(function.upgrade().is_none());
    COLLECTOR.with(|collector| {
        let collector = collector.borrow();
        assert!(collector.tables.is_empty());
        assert!(collector.closures.is_empty());
        assert!(collector.upvalues.is_empty());
    });
}

#[test]
fn interpreted_recursive_closure_and_closed_upvalue_cycle_is_reclaimed() {
    let env = TableRef::new();
    let mut thread = Thread::new(env.clone());
    let factory = stdlib::load_string(
        "local f; f = function() return f end; return f",
        Some("recursive-gc"),
        env.clone(),
    )
    .unwrap();
    let result = thread.call(&factory, &[]).unwrap();
    let Value::Closure(function) = &result[0] else {
        panic!("expected closure")
    };
    let function_weak = Rc::downgrade(function);
    let upvalue_weak = Rc::downgrade(&function.upvalues[0]);
    assert!(
        matches!(&*function.upvalues[0].borrow(), UpValue::Closed(Value::Closure(value)) if Rc::ptr_eq(value, function))
    );
    assert!(thread.call(&result[0], &[]).unwrap()[0].raw_equals(&result[0]));
    let env_weak = Rc::downgrade(&env.0);
    drop(result);
    drop(factory);
    drop(thread);
    drop(env);
    collected();
    assert!(function_weak.upgrade().is_none());
    assert!(upvalue_weak.upgrade().is_none());
    assert!(env_weak.upgrade().is_none());
}

#[test]
fn repeated_closure_keys_values_and_shared_upvalue_slots_count_exactly_once_per_slot() {
    let env = TableRef::new();
    env.set_str("kept", 61);
    let slot = UpValue::closed(env.clone().into());
    let first = closure(env.clone(), vec![slot.clone(), slot.clone()]);
    let second = closure(env.clone(), vec![slot.clone()]);
    env.rawset(Value::Closure(first.clone()), Value::Closure(first.clone()));
    env.set_str("second", Value::Closure(second.clone()));
    let first_weak = Rc::downgrade(&first);
    let second_weak = Rc::downgrade(&second);
    let slot_weak = Rc::downgrade(&slot);
    let env_weak = Rc::downgrade(&env.0);
    drop(env);
    drop(second);
    assert_eq!(collected().tables_cleared, 0);
    assert_eq!(first.env.borrow().get_str("kept").as_num(), Some(61.0));
    assert!(Rc::ptr_eq(&first.upvalues[0], &first.upvalues[1]));
    drop(first);
    // An externally retained shared upvalue is itself a root, even after
    // every external closure handle has been released.
    assert_eq!(collected().tables_cleared, 0);
    assert!(first_weak.upgrade().is_some());
    assert!(second_weak.upgrade().is_some());
    assert!(
        matches!(&*slot.borrow(), UpValue::Closed(Value::Table(value)) if value.get_str("kept").as_num() == Some(61.0))
    );
    drop(slot);
    assert_eq!(collected().tables_cleared, 1);
    assert!(first_weak.upgrade().is_none());
    assert!(second_weak.upgrade().is_none());
    assert!(slot_weak.upgrade().is_none());
    assert!(env_weak.upgrade().is_none());
}

#[test]
fn raw_unregistered_closure_and_upvalue_slots_are_conservative_roots() {
    let env = TableRef::new();
    env.set_str("kept", 73);
    let raw = Rc::new(LuaClosure {
        prototype: Rc::new(Prototype::default()),
        env: RefCell::new(env.clone()),
        upvalues: Vec::new(),
        debug_name: RefCell::new(None),
    });
    env.set_str("raw", Value::Closure(raw.clone()));
    let weak = Rc::downgrade(&raw);
    drop(raw);
    drop(env);
    assert_eq!(collected().tables_cleared, 0);
    let raw = weak.upgrade().unwrap();
    assert_eq!(raw.env.borrow().get_str("kept").as_num(), Some(73.0));
    raw.env.borrow().set_str("raw", Value::Nil);
    drop(raw);
    assert!(weak.upgrade().is_none());

    let env = TableRef::new();
    let slot = Rc::new(RefCell::new(UpValue::Closed(env.clone().into())));
    let function = closure(env.clone(), vec![slot.clone()]);
    env.set_str("function", Value::Closure(function.clone()));
    let weak = Rc::downgrade(&env.0);
    let slot_weak = Rc::downgrade(&slot);
    drop(env);
    drop(function);
    drop(slot);
    assert_eq!(collected().tables_cleared, 0);
    // The raw slot remains opaque, rather than having its outgoing TableRef
    // subtracted as if the collector owned a registered UpValue allocation.
    let slot = slot_weak.upgrade().unwrap();
    assert!(matches!(&*slot.borrow(), UpValue::Closed(Value::Table(value)) if value.size() == 1));
    *slot.borrow_mut() = UpValue::Closed(Value::Nil);
    drop(slot);
    collected();
    assert!(weak.upgrade().is_none());
}

#[test]
fn native_capture_preserves_closure_environment_and_closed_upvalue() {
    let env = TableRef::new();
    let slot = UpValue::closed(env.clone().into());
    let function = closure(env.clone(), vec![slot]);
    env.set_str("function", Value::Closure(function.clone()));
    env.set_str("kept", 83);
    let weak = Rc::downgrade(&function);
    let native = NativeFunction::new("captured-closure", move |call| {
        Ok(call.push(Value::Closure(function.clone())))
    });
    drop(env);
    assert_eq!(collected().tables_cleared, 0);
    assert_eq!(
        weak.upgrade()
            .unwrap()
            .env
            .borrow()
            .get_str("kept")
            .as_num(),
        Some(83.0)
    );
    drop(native);
    assert_eq!(collected().tables_cleared, 1);
    assert!(weak.upgrade().is_none());
}

#[test]
fn shared_prototype_constant_slots_are_opaque_roots_not_edges_per_closure() {
    let env = TableRef::new();
    let target_env = TableRef::new();
    target_env.set_str("kept", 89);
    let target = closure(target_env.clone(), Vec::new());
    let slot = UpValue::closed(env.clone().into());
    let prototype = Rc::new(Prototype {
        constants: vec![target_env.clone().into(), Value::Closure(target.clone())],
        ..Prototype::default()
    });
    let first = LuaClosure::with_upvalues(prototype.clone(), env.clone(), vec![slot.clone()]);
    let second = LuaClosure::with_upvalues(prototype.clone(), env.clone(), vec![slot]);
    env.set_str("first", Value::Closure(first.clone()));
    env.set_str("second", Value::Closure(second.clone()));
    let env_weak = Rc::downgrade(&env.0);
    let target_weak = Rc::downgrade(&target);
    drop(first);
    drop(second);
    drop(env);
    drop(target);
    drop(target_env);
    // Only the Prototype owns its constants. Counting those once per
    // closure would produce an invalid internal count or erase live data.
    assert_eq!(collected().tables_cleared, 1);
    assert!(env_weak.upgrade().is_none());
    assert_eq!(
        target_weak
            .upgrade()
            .unwrap()
            .env
            .borrow()
            .get_str("kept")
            .as_num(),
        Some(89.0)
    );
    drop(prototype);
    assert!(target_weak.upgrade().is_none());
}

#[test]
fn prototype_mediated_cycles_are_retained_conservatively() {
    let env = TableRef::new();
    env.set_str("kept", 97);
    let prototype = Rc::new(Prototype {
        constants: vec![env.clone().into()],
        ..Prototype::default()
    });
    let function = LuaClosure::new(prototype, env.clone());
    let weak = Rc::downgrade(&function);
    env.set_str("function", Value::Closure(function.clone()));
    drop(function);
    drop(env);
    assert_eq!(collected().tables_cleared, 0);
    let function = weak.upgrade().unwrap();
    assert_eq!(function.env.borrow().get_str("kept").as_num(), Some(97.0));
    // Explicit host mutation only breaks this fixture's deliberately opaque
    // cycle; the collector must not rewrite a live prototype or environment.
    function.env.borrow().set_str("function", Value::Nil);
    drop(function);
    assert!(weak.upgrade().is_none());
}

#[test]
fn closure_environment_and_upvalue_borrows_defer_all_mutations() {
    let env = TableRef::new();
    let slot = UpValue::closed(env.clone().into());
    let function = closure(env, vec![slot.clone()]);
    let (dead_env, dead_function) = environment_cycle();
    for mutable in [false, true] {
        if mutable {
            let _borrow = function.env.borrow_mut();
            assert_eq!(collect_tables(), CollectionResult::Deferred);
        } else {
            let _borrow = function.env.borrow();
            assert_eq!(collect_tables(), CollectionResult::Deferred);
        }
        assert!(dead_function.upgrade().is_some());
        assert_eq!(dead_env.upgrade().unwrap().borrow().map.len(), 1);
        if mutable {
            let _borrow = slot.borrow_mut();
            assert_eq!(collect_pending(), CollectionResult::Deferred);
        } else {
            let _borrow = slot.borrow();
            assert_eq!(collect_pending(), CollectionResult::Deferred);
        }
        assert!(dead_function.upgrade().is_some());
    }
    assert!(matches!(collect_pending(), CollectionResult::Collected(_)));
    assert!(dead_env.upgrade().is_none());
    assert!(dead_function.upgrade().is_none());
    assert_eq!(function.upvalues.len(), 1);
    assert!(matches!(&*slot.borrow(), UpValue::Closed(Value::Table(_))));
}

#[test]
fn returned_and_error_valued_closures_survive_safe_point_collection() {
    let mut thread = Thread::new(TableRef::new());
    stdlib::install(&mut thread, None).unwrap();
    for raises_error in [false, true] {
        let (dead_env, dead_function) = environment_cycle();
        let source = if raises_error {
            "local n = 100; local f; f = function() n=n+1; return n, f end; collectgarbage(); error(f)"
        } else {
            "local n = 100; local f; f = function() n=n+1; return n, f end; collectgarbage(); return f"
        };
        let factory = stdlib::load_string(source, None, thread.globals().clone()).unwrap();
        let function = if raises_error {
            let (message, _, error) = thread.pcall_ex(&factory, &[]).unwrap_err();
            let LuaError::Kahlua(value) = error else {
                panic!("expected Lua-valued error")
            };
            assert!(message.raw_equals(&value));
            message
        } else {
            thread.call(&factory, &[]).unwrap().remove(0)
        };
        assert!(dead_env.upgrade().is_none());
        assert!(dead_function.upgrade().is_none());
        assert_eq!(collect_pending(), CollectionResult::NotRequested);
        let result = thread.call(&function, &[]).unwrap();
        assert_eq!(result[0].as_num(), Some(101.0));
        assert!(result[1].raw_equals(&function));
    }
}

#[test]
fn suspended_coroutine_state_and_open_upvalues_remain_opaque_roots() {
    let mut thread = Thread::new(TableRef::new());
    stdlib::install(&mut thread, None).unwrap();
    let factory = stdlib::load_string(
        "local t = {value = 107}; local function read() return t.value end; coroutine.yield(read); t.value = t.value + 1; return read()",
        None,
        thread.globals().clone(),
    )
    .unwrap();
    let Value::Closure(function) = factory else {
        panic!("expected closure")
    };
    let co = thread.create_coroutine(function);
    let yielded = thread.resume(&co, &[]).unwrap().unwrap();
    let Value::Closure(read) = &yielded[0] else {
        panic!("expected yielded closure")
    };
    let read_weak = Rc::downgrade(read);
    assert!(matches!(&*read.upvalues[0].borrow(), UpValue::Open { .. }));
    drop(yielded);
    collected();
    let read = Value::Closure(read_weak.upgrade().unwrap());
    assert_eq!(thread.call(&read, &[]).unwrap()[0].as_num(), Some(107.0));
    let resumed = thread.resume(&co, &[]).unwrap().unwrap();
    assert_eq!(resumed[0].as_num(), Some(108.0));
    let Value::Closure(read) = &read else {
        unreachable!()
    };
    assert!(matches!(&*read.upvalues[0].borrow(), UpValue::Closed(_)));
}

#[test]
fn active_vm_frame_and_nested_native_call_remain_roots() {
    let mut thread = Thread::new(TableRef::new());
    let checked = Rc::new(Cell::new(false));
    let observed = checked.clone();
    let (dead_env, _) = environment_cycle();
    thread.globals().set_str(
        "probe",
        Value::Native(NativeFunction::new("probe", move |call| {
            // Immediate host collection here is deliberately more adversarial
            // than the public queued Lua API. Running frame/stack Rc slots are
            // not subtracted, and therefore protect active closure/upvalue state.
            assert!(matches!(collect_tables(), CollectionResult::Collected(_)));
            assert!(dead_env.upgrade().is_none());
            observed.set(true);
            Ok(call.push(1))
        })),
    );
    let factory = stdlib::load_string(
        "local t = {value = 109}; local function inner() probe(); return t.value end; return inner()",
        None,
        thread.globals().clone(),
    )
    .unwrap();
    assert_eq!(thread.call(&factory, &[]).unwrap()[0].as_num(), Some(109.0));
    assert!(checked.get());
}

struct ReentrantPayload {
    dropped: Rc<Cell<bool>>,
    env: Weak<RefCell<Table>>,
    slot: Weak<RefCell<UpValue>>,
    function: Weak<LuaClosure>,
}

impl LuaObject for ReentrantPayload {
    fn class_name(&self) -> &str {
        "test.GcClosureReentrantPayload"
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

impl Drop for ReentrantPayload {
    fn drop(&mut self) {
        assert!(self.env.upgrade().unwrap().try_borrow_mut().is_ok());
        assert!(self.slot.upgrade().unwrap().try_borrow_mut().is_ok());
        assert!(self
            .function
            .upgrade()
            .unwrap()
            .env
            .try_borrow_mut()
            .is_ok());
        // Register all participating kinds while the pass is dropping an
        // old payload. The registry and every heap borrow must be released.
        let env = TableRef::new();
        let slot = UpValue::closed(env.clone().into());
        let function = closure(env.clone(), vec![slot]);
        env.set_str("function", Value::Closure(function));
        request_collection();
        assert_eq!(collect_tables(), CollectionResult::Deferred);
        self.dropped.set(true);
    }
}

#[test]
fn white_payloads_detach_before_reentrant_drop_and_new_registration() {
    let env = TableRef::new();
    let slot = UpValue::closed(Value::Nil);
    let function = closure(env.clone(), vec![slot.clone()]);
    let env_weak = Rc::downgrade(&env.0);
    let function_weak = Rc::downgrade(&function);
    let slot_weak = Rc::downgrade(&slot);
    env.set_str("function", Value::Closure(function.clone()));
    let dropped = Rc::new(Cell::new(false));
    *slot.borrow_mut() = UpValue::Closed(Value::Object(Rc::new(ReentrantPayload {
        dropped: dropped.clone(),
        env: env_weak.clone(),
        slot: slot_weak.clone(),
        function: function_weak.clone(),
    })));
    drop(env);
    drop(function);
    drop(slot);
    assert_eq!(collected().tables_cleared, 1);
    assert!(dropped.get());
    assert!(env_weak.upgrade().is_none());
    assert!(function_weak.upgrade().is_none());
    assert!(slot_weak.upgrade().is_none());
    assert!(matches!(
        collect_pending(),
        CollectionResult::Collected(CollectionStats {
            tables_cleared: 1,
            ..
        })
    ));
    assert_eq!(collect_pending(), CollectionResult::NotRequested);
}

#[test]
fn closure_and_upvalue_registry_headers_are_bounded_without_gc_requests() {
    let env = TableRef::new();
    let prototype = Rc::new(Prototype::default());
    for _ in 0..10_000 {
        drop(LuaClosure::new(prototype.clone(), env.clone()));
        drop(UpValue::closed(Value::Num(113.0)));
    }
    COLLECTOR.with(|collector| {
        let collector = collector.borrow();
        assert!(collector.closures.len() <= 4);
        assert!(collector.upvalues.len() <= 4);
    });
}

#[test]
fn mostly_live_closure_and_upvalue_registries_reserve_pruning_headroom() {
    let env = TableRef::new();
    let prototype = Rc::new(Prototype::default());
    let mut closures: Vec<_> = (0..1024)
        .map(|_| LuaClosure::new(prototype.clone(), env.clone()))
        .collect();
    let capacity = COLLECTOR.with(|collector| collector.borrow().closures.capacity());
    while closures.len() < capacity {
        closures.push(LuaClosure::new(prototype.clone(), env.clone()));
    }
    drop(closures.pop());
    drop(LuaClosure::new(prototype.clone(), env.clone()));
    COLLECTOR.with(|collector| {
        assert!(collector.borrow().closures.capacity() >= capacity + closures.len())
    });

    let mut upvalues: Vec<_> = (0..1024)
        .map(|_| UpValue::closed(Value::Num(127.0)))
        .collect();
    let capacity = COLLECTOR.with(|collector| collector.borrow().upvalues.capacity());
    while upvalues.len() < capacity {
        upvalues.push(UpValue::closed(Value::Num(127.0)));
    }
    drop(upvalues.pop());
    drop(UpValue::closed(Value::Num(127.0)));
    COLLECTOR.with(|collector| {
        assert!(collector.borrow().upvalues.capacity() >= capacity + upvalues.len())
    });
    collected();
    assert!(closures
        .iter()
        .all(|function| function.env.borrow().ptr_eq(&env)));
    assert!(upvalues
        .iter()
        .all(|slot| matches!(&*slot.borrow(), UpValue::Closed(Value::Num(127.0)))));
}

#[test]
fn opaque_coroutine_reference_is_not_subtracted_from_external_roots() {
    let env = TableRef::new();
    let function = closure(env.clone(), Vec::new());
    let co = Rc::new(RefCell::new(Coroutine {
        stack: vec![Value::Closure(function.clone())],
        env: Some(env.clone()),
        ..Coroutine::default()
    }));
    let slot = UpValue::open(co.clone(), 0);
    let outer = closure(env.clone(), vec![slot]);
    env.set_str("outer", Value::Closure(outer));
    let weak = Rc::downgrade(&function);
    drop(function);
    drop(env);
    assert_eq!(collected().tables_cleared, 0);
    assert!(weak.upgrade().is_some());
    // This slice deliberately leaves the coroutine-mediated cycle retained,
    // even after its external CoroutineRef has been dropped.
    drop(co);
    assert_eq!(collected().tables_cleared, 0);
    let function = weak.upgrade().unwrap();
    function.env.borrow().set_str("outer", Value::Nil);
    drop(function);
    assert!(weak.upgrade().is_none());
}
