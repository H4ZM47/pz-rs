use super::*;
use pz_kahlua::{LuaObject, TableRef};
use std::{any::Any, cell::{Cell, RefCell}, rc::Rc};

fn drain() {
    for _ in 0..16 {
        if pz_kahlua::native_list::drain_retired() == 0 { break; }
    }
}
struct Tail;
impl Drop for Tail { fn drop(&mut self) { drain(); } }

#[test]
fn first_failure_keeps_original_payload_and_rejects_later_report() {
    let _tail = Tail;
    let payload = TableRef::new();
    let mut slot = None;
    record_first(&mut slot, EngineError::lua("OnTick", LuaError::Kahlua(Value::Table(payload.clone()))));
    record_first(&mut slot, "later teardown failure".into());
    match slot.as_ref().unwrap().lua_error() {
        Some(LuaError::Kahlua(Value::Table(actual))) => assert!(actual.ptr_eq(&payload)),
        _ => panic!("original error lost"),
    }
    assert!(slot.as_ref().unwrap().report_text().is_none());
    let plain = EngineError::from("existing report");
    assert_eq!(plain.report_text(), Some("existing report"));
    assert_eq!(plain.to_string(), "existing report");
}

#[test]
fn diagnostic_rendering_never_calls_virtual_payload_formatting_or_metadata() {
    let _tail = Tail;
    struct Hostile;
    impl LuaObject for Hostile {
        fn as_any(&self) -> &dyn Any { self }
        fn class_name(&self) -> &str { panic!("virtual class metadata") }
        fn to_lua_string(&self) -> String { panic!("infallible formatter") }
    }
    let fault = EngineError::lua("OnTick", LuaError::Kahlua(Value::Object(Rc::new(Hostile))));
    assert_eq!(fault.to_string(), "OnTick: Lua error (userdata payload)");
    assert_eq!(format!("{fault:?}"), "OnTick: Lua error (userdata payload)");
    let native = EngineError::lua("callback", LuaError::Runtime { class: "test.UnknownNativeClass", message: "original".into() });
    assert!(matches!(native.lua_error(), Some(LuaError::Runtime { class: "test.UnknownNativeClass", message }) if message == "original"));
}

#[test]
fn rejected_or_dropped_error_payload_waits_for_the_outer_safe_drain() {
    let _tail = Tail;
    drain();
    struct Probe { outer: Rc<RefCell<()>>, safe: Rc<Cell<bool>> }
    impl LuaObject for Probe {
        fn as_any(&self) -> &dyn Any { self }
        fn class_name(&self) -> &str { "test.EngineErrorTail" }
    }
    impl Drop for Probe { fn drop(&mut self) { self.safe.set(self.outer.try_borrow_mut().is_ok()); } }
    let outer = Rc::new(RefCell::new(()));
    let safe = Rc::new(Cell::new(false));
    let value = Rc::new(Probe { outer: outer.clone(), safe: safe.clone() });
    let weak = Rc::downgrade(&value);
    let deferral = pz_kahlua::gc::defer_collection();
    let held = outer.borrow_mut();
    let mut slot = Some(EngineError::from("first"));
    record_first(&mut slot, EngineError::lua("later", LuaError::Kahlua(Value::Object(value))));
    assert!(weak.upgrade().is_some() && !safe.get());
    assert_eq!(pz_kahlua::native_list::drain_retired(), 0);
    drop(held);
    drop(deferral);
    drain();
    assert!(weak.upgrade().is_none() && safe.get());
}


#[test]
fn engine_state_boundary_keeps_actual_animal_cause_and_first_error_domain() {
    use pz_characters::character::{animal::AnimalDataError, state::StateFailure};
    let original = Rc::new(AnimalDataError("NullPointerException: currentStage".into()));
    let mut slot = None;
    record_first(&mut slot, EngineError::state("IsoWorld.update", StateFailure::Animal(original.clone())));
    record_first(&mut slot, EngineError::from("later diagnostic"));
    let error = slot.as_ref().unwrap();
    assert!(Rc::ptr_eq(error.state_failure().unwrap().animal().unwrap(), &original));
    assert_eq!(error.to_string(), "IsoWorld.update: java.lang.NullPointerException: currentStage");
    assert!(error.lua_error().is_none() && error.report_text().is_none());
}

#[test]
fn retained_diagnostic_payload_uses_same_guard_free_engine_retirement() {
    let _tail = Tail;
    drain();
    #[derive(Debug)] struct Cause;
    impl std::fmt::Display for Cause { fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { f.write_str("original") } }
    impl std::error::Error for Cause {}
    struct Probe { outer: Rc<RefCell<()>>, safe: Rc<Cell<bool>> }
    impl LuaObject for Probe {
        fn as_any(&self) -> &dyn Any { self }
        fn class_name(&self) -> &str { "test.RetainedEngineDiagnostic" }
    }
    impl Drop for Probe { fn drop(&mut self) { self.safe.set(self.outer.try_borrow_mut().is_ok()); } }
    let outer = Rc::new(RefCell::new(())); let safe = Rc::new(Cell::new(false));
    let value = Rc::new(Probe { outer: outer.clone(), safe: safe.clone() }); let weak = Rc::downgrade(&value);
    let fault = LuaError::with_cause(LuaError::Kahlua(Value::Object(value)), Rc::new(Cause));
    let deferral = pz_kahlua::gc::defer_collection(); let held = outer.borrow_mut();
    drop(EngineError::lua("retained callback", fault));
    assert!(weak.upgrade().is_some() && !safe.get());
    assert_eq!(pz_kahlua::native_list::drain_retired(), 0);
    drop(held); drop(deferral); drain();
    assert!(weak.upgrade().is_none() && safe.get());
}


#[test]
fn weak_diagnostic_observer_does_not_skip_actual_payload_retirement() {
    let _tail = Tail; drain();
    #[derive(Debug)] struct Cause;
    impl std::fmt::Display for Cause { fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { f.write_str("original") } }
    impl std::error::Error for Cause {}
    struct Probe { outer: Rc<RefCell<()>>, safe: Rc<Cell<bool>> }
    impl LuaObject for Probe {
        fn as_any(&self) -> &dyn Any { self }
        fn class_name(&self) -> &str { "test.WeakRetainedDiagnostic" }
    }
    impl Drop for Probe { fn drop(&mut self) { self.safe.set(self.outer.try_borrow_mut().is_ok()); } }
    let outer = Rc::new(RefCell::new(())); let safe = Rc::new(Cell::new(false));
    let value = Rc::new(Probe { outer: outer.clone(), safe: safe.clone() }); let weak_value = Rc::downgrade(&value);
    let diagnostic = Rc::new(LuaError::Kahlua(Value::Object(value)));
    let weak_diagnostic = Rc::downgrade(&diagnostic);
    let fault = LuaError::WithCause { diagnostic, cause: Rc::new(Cause) };
    let deferral = pz_kahlua::gc::defer_collection(); let held = outer.borrow_mut();
    drop(EngineError::lua("weak retained callback", fault));
    assert!(weak_diagnostic.upgrade().is_none(), "the actual last strong diagnostic owner was dropped");
    assert!(weak_value.upgrade().is_some() && !safe.get(), "Value must remain pinned despite Weak diagnostic observer");
    assert_eq!(pz_kahlua::native_list::drain_retired(), 0);
    drop(held); drop(deferral); drain();
    assert!(weak_value.upgrade().is_none() && safe.get());
}
