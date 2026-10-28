use super::*;
use pz_kahlua::gc::{self, CollectionResult};
use pz_kahlua::{stdlib, LuaObject};
use std::any::Any;
use std::cell::Cell;
use std::rc::Weak;

fn collect() {
    assert!(matches!(
        gc::collect_tables(),
        CollectionResult::Collected(_)
    ));
}

struct Fixture {
    thread: Thread,
    events: EventManager,
    event: Rc<RefCell<Event>>,
    callback: Rc<LuaClosure>,
}

impl Fixture {
    fn new(copies: usize) -> Self {
        let env = TableRef::new();
        let mut thread = Thread::new(env.clone());
        let events = EventManager::register(&env, "Events", &["Pulse"]);
        let event = events.add_event("Pulse");
        let Value::Closure(callback) =
            stdlib::load_string("calls = (calls or 0) + 1; return calls", None, env).unwrap()
        else {
            panic!("expected callback")
        };
        let add = events
            .table
            .get_str("Pulse")
            .as_table()
            .unwrap()
            .get_str("Add");
        for _ in 0..copies {
            thread
                .call(&add, &[Value::Closure(callback.clone())])
                .unwrap();
        }
        Self {
            thread,
            events,
            event,
            callback,
        }
    }

    fn method(&self, name: &str) -> Value {
        self.events
            .table
            .get_str("Pulse")
            .as_table()
            .unwrap()
            .get_str(name)
    }
}

#[test]
fn retired_event_native_capture_callback_environment_cycle_is_reclaimed() {
    let fixture = Fixture::new(1);
    let event = Rc::downgrade(&fixture.event);
    let callback = Rc::downgrade(&fixture.callback);
    let Value::Native(add) = fixture.method("Add") else {
        unreachable!()
    };
    let Value::Native(remove) = fixture.method("Remove") else {
        unreachable!()
    };
    let add_weak = Rc::downgrade(&add);
    let remove_weak = Rc::downgrade(&remove);
    drop(add);
    drop(remove);
    drop(fixture);
    collect();
    assert!(event.upgrade().is_none());
    assert!(callback.upgrade().is_none());
    assert!(add_weak.upgrade().is_none());
    assert!(remove_weak.upgrade().is_none());
}

#[test]
fn retained_event_manager_preserves_callbacks_and_old_environment() {
    let fixture = Fixture::new(1);
    let events = fixture.events.clone();
    let callback = Rc::downgrade(&fixture.callback);
    let event = Rc::downgrade(&fixture.event);
    drop(fixture);
    collect();
    assert_eq!(events.callbacks("Pulse"), 1);
    let mut thread = Thread::new(TableRef::new());
    assert!(events.trigger(&mut thread, "Pulse", &[]));
    assert_eq!(
        callback
            .upgrade()
            .unwrap()
            .env
            .borrow()
            .get_str("calls")
            .as_num(),
        Some(1.0)
    );
    drop(events);
    collect();
    assert!(callback.upgrade().is_none());
    assert!(event.upgrade().is_none());
}

#[test]
fn retained_event_owner_and_duplicate_slots_survive_then_release() {
    let mut fixture = Fixture::new(3);
    assert_eq!(fixture.event.borrow().callbacks.len(), 3);
    let remove = fixture.method("Remove");
    fixture
        .thread
        .call(&remove, &[Value::Closure(fixture.callback.clone())])
        .unwrap();
    drop(remove);
    assert_eq!(fixture.event.borrow().callbacks.len(), 2);
    let event = fixture.event.clone();
    let callback = Rc::downgrade(&fixture.callback);
    drop(fixture);
    collect();
    assert_eq!(event.borrow().callbacks.len(), 2);
    let mut thread = Thread::new(TableRef::new());
    let function = Value::Closure(event.borrow().callbacks[0].clone());
    assert_eq!(thread.call(&function, &[]).unwrap()[0].as_num(), Some(1.0));
    drop(function);
    drop(event);
    collect();
    assert!(callback.upgrade().is_none());
}

#[test]
fn retained_add_native_is_a_real_root_and_remains_callable() {
    let fixture = Fixture::new(1);
    let add = fixture.method("Add");
    let event = Rc::downgrade(&fixture.event);
    let callback = Rc::downgrade(&fixture.callback);
    drop(fixture);
    collect();
    let mut thread = Thread::new(TableRef::new());
    thread
        .call(&add, &[Value::Closure(callback.upgrade().unwrap())])
        .unwrap();
    assert_eq!(event.upgrade().unwrap().borrow().callbacks.len(), 2);
    drop(add);
    collect();
    assert!(event.upgrade().is_none());
    assert!(callback.upgrade().is_none());
}

#[test]
fn retained_remove_native_can_remove_the_last_callback_without_retaining_its_environment() {
    let fixture = Fixture::new(1);
    let remove = fixture.method("Remove");
    let event = Rc::downgrade(&fixture.event);
    let callback = Rc::downgrade(&fixture.callback);
    drop(fixture);
    collect();
    let mut thread = Thread::new(TableRef::new());
    thread
        .call(&remove, &[Value::Closure(callback.upgrade().unwrap())])
        .unwrap();
    collect();
    assert_eq!(event.upgrade().unwrap().borrow().callbacks.len(), 0);
    assert!(callback.upgrade().is_none());
    drop(remove);
    assert!(event.upgrade().is_none());
}

#[test]
fn event_borrows_defer_the_whole_pass_without_clearing_other_retired_callbacks() {
    let rooted = Fixture::new(1);
    let retired = Fixture::new(1);
    let callback = Rc::downgrade(&retired.callback);
    drop(retired);
    let borrow = rooted.event.borrow();
    assert_eq!(gc::collect_tables(), CollectionResult::Deferred);
    assert!(callback.upgrade().is_some());
    drop(borrow);
    let borrow = rooted.event.borrow_mut();
    assert_eq!(gc::collect_pending(), CollectionResult::Deferred);
    assert!(callback.upgrade().is_some());
    drop(borrow);
    assert!(matches!(
        gc::collect_pending(),
        CollectionResult::Collected(_)
    ));
    assert!(callback.upgrade().is_none());
    assert_eq!(rooted.event.borrow().callbacks.len(), 1);
}

#[test]
fn callback_self_removal_and_addition_during_dispatch_preserve_index_semantics() {
    let env = TableRef::new();
    let mut thread = Thread::new(env.clone());
    env.set_str(
        "collectgarbage",
        Value::Native(NativeFunction::new("collectgarbage", |_| {
            gc::request_collection();
            Ok(0)
        })),
    );
    let events = EventManager::register(&env, "Events", &["Pulse"]);
    let setup = stdlib::load_string(
        "calls=0; local function second() calls=calls+10 end; local first; first=function() calls=calls+1; Events.Pulse.Remove(first); Events.Pulse.Add(second); collectgarbage() end; Events.Pulse.Add(first)",
        None, env.clone(),
    ).unwrap();
    thread.call(&setup, &[]).unwrap();
    assert!(events.trigger(&mut thread, "Pulse", &[]));
    assert_eq!(env.get_str("calls").as_num(), Some(11.0));
    assert_eq!(events.callbacks("Pulse"), 1);
    assert!(events.trigger(&mut thread, "Pulse", &[]));
    assert_eq!(env.get_str("calls").as_num(), Some(21.0));
}

struct ReentrantDrop {
    event: Weak<RefCell<Event>>,
    callback: Weak<LuaClosure>,
    dropped: Rc<Cell<bool>>,
}

impl LuaObject for ReentrantDrop {
    fn class_name(&self) -> &str {
        "test.EventReentrantDrop"
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

impl Drop for ReentrantDrop {
    fn drop(&mut self) {
        assert!(self.event.upgrade().unwrap().try_borrow_mut().is_ok());
        assert!(self
            .callback
            .upgrade()
            .unwrap()
            .env
            .try_borrow_mut()
            .is_ok());
        let env = TableRef::new();
        let _events = EventManager::register(&env, "Events", &["CreatedByDrop"]);
        gc::request_collection();
        assert_eq!(gc::collect_tables(), CollectionResult::Deferred);
        self.dropped.set(true);
    }
}

#[test]
fn event_and_capture_guards_are_released_before_reentrant_userdata_destruction() {
    let fixture = Fixture::new(1);
    let event = Rc::downgrade(&fixture.event);
    let callback = Rc::downgrade(&fixture.callback);
    let dropped = Rc::new(Cell::new(false));
    fixture.thread.globals().set_str(
        "probe",
        Value::Object(Rc::new(ReentrantDrop {
            event: event.clone(),
            callback: callback.clone(),
            dropped: dropped.clone(),
        })),
    );
    drop(fixture);
    collect();
    assert!(dropped.get());
    assert!(event.upgrade().is_none());
    assert!(callback.upgrade().is_none());
    assert!(matches!(
        gc::collect_pending(),
        CollectionResult::Collected(_)
    ));
}
