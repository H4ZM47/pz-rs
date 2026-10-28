use super::*;
use pz_kahlua::{LuaError, TableRef, Value};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Phase { Enter, Update, Exit, Yield, Redirect, Reenter, Render }
struct Probe {
    name: &'static str,
    fail: Option<Phase>,
    action: StateAction,
    log: Rc<RefCell<Vec<String>>>,
    payload: TableRef,
    redirect: Option<StateRef>,
}
impl Probe {
    fn phase(&self, cx: &mut StateCx<'_>, phase: Phase, label: &str) {
        self.log.borrow_mut().push(format!("{} {label}", self.name));
        if self.fail == Some(phase) {
            cx.record_lua_failure("state callback", LuaError::Kahlua(Value::Table(self.payload.clone())));
        }
    }
}
impl GameState for Probe {
    fn class_name(&self) -> &'static str { self.name }
    fn enter(&mut self, cx: &mut StateCx<'_>) { self.phase(cx, Phase::Enter, "enter"); }
    fn update(&mut self, cx: &mut StateCx<'_>) -> StateAction { self.phase(cx, Phase::Update, "update"); self.action }
    fn exit(&mut self, cx: &mut StateCx<'_>) { self.phase(cx, Phase::Exit, "exit"); }
    fn yield_state(&mut self, cx: &mut StateCx<'_>) { self.phase(cx, Phase::Yield, "yield"); }
    fn redirect_state(&mut self, cx: &mut StateCx<'_>) -> Option<StateRef> { self.phase(cx, Phase::Redirect, "redirect"); self.redirect.clone() }
    fn reenter(&mut self, cx: &mut StateCx<'_>) { self.phase(cx, Phase::Reenter, "reenter"); }
    fn render(&mut self, cx: &mut StateCx<'_>, _: &mut DrawList) { self.phase(cx, Phase::Render, "render"); }
}
fn probe(name: &'static str, fail: Option<Phase>, action: StateAction, log: &Rc<RefCell<Vec<String>>>, payload: &TableRef, redirect: Option<StateRef>) -> StateRef {
    state(Probe { name, fail, action, log: log.clone(), payload: payload.clone(), redirect })
}
fn with_cx(f: impl FnOnce(&mut StateCx<'_>)) {
    let deferral = pz_kahlua::gc::defer_collection();
    let fs = Rc::new(RefCell::new(pz_fs::GameFs::new(std::env::temp_dir())));
    let mut lua = LuaManager::init(fs).unwrap();
    let launch = Launch { game_dir: std::env::temp_dir(), cache_dir: std::env::temp_dir(), start: Default::default() };
    let mut world = None;
    let mut retired_models = Vec::new();
    let mut error = None;
    f(&mut StateCx { lua: &mut lua, launch: &launch, screen_w: 1, screen_h: 1, multiplier: 1.0, time_delta: 1.0 / 30.0, world: &mut world, shutdown: false, error: &mut error, retired_models: &mut retired_models });
    drop(error);
    drop(deferral);
    for _ in 0..16 { if pz_kahlua::native_list::drain_retired() == 0 { break; } }
}
fn stopped(machine: &mut GameStateMachine, cx: &mut StateCx<'_>, payload: &TableRef, log: &Rc<RefCell<Vec<String>>>, expected: &[&str]) {
    assert!(machine.current.is_none() && machine.force_next.is_none() && machine.yield_stack.is_empty() && !machine.looping);
    match cx.error.as_ref().unwrap().lua_error() {
        Some(LuaError::Kahlua(Value::Table(actual))) => assert!(actual.ptr_eq(payload)),
        _ => panic!("original state error lost"),
    }
    assert_eq!(&*log.borrow(), expected);
    machine.update(cx);
    machine.render(cx, &mut DrawList::default());
    assert_eq!(&*log.borrow(), expected);
}

#[test]
fn enter_failure_prevents_same_tick_update_and_all_later_render() {
    with_cx(|cx| {
        let log = Rc::new(RefCell::new(Vec::new())); let payload = TableRef::new();
        let current = probe("current", Some(Phase::Enter), StateAction::Continue, &log, &payload, None);
        let mut machine = GameStateMachine { states: vec![current], ..Default::default() };
        machine.update(cx);
        stopped(&mut machine, cx, &payload, &log, &["current enter"]);
    });
}

#[test]
fn update_failure_prevents_exit_yield_and_redirect_for_every_returned_action() {
    for action in [StateAction::Continue, StateAction::Yield, StateAction::Remain] {
        with_cx(|cx| {
            let log = Rc::new(RefCell::new(Vec::new())); let payload = TableRef::new();
            let current = probe("current", Some(Phase::Update), action, &log, &payload, None);
            let mut machine = GameStateMachine { firstrun: false, states: vec![current.clone()], current: Some(current), ..Default::default() };
            machine.update(cx);
            stopped(&mut machine, cx, &payload, &log, &["current update"]);
        });
    }
}

#[test]
fn exit_and_yield_failures_prevent_redirect_and_next_state_entry() {
    for (phase,action,expected) in [(Phase::Exit, StateAction::Continue, "current exit"), (Phase::Yield, StateAction::Yield, "current yield")] {
        with_cx(|cx| {
            let log = Rc::new(RefCell::new(Vec::new())); let payload = TableRef::new();
            let next = probe("next", None, StateAction::Remain, &log, &payload, None);
            let current = probe("current", Some(phase), action, &log, &payload, Some(next));
            let mut machine = GameStateMachine { firstrun: false, states: vec![current.clone()], current: Some(current), ..Default::default() };
            machine.update(cx);
            stopped(&mut machine, cx, &payload, &log, &["current update", expected]);
        });
    }
}

#[test]
fn redirect_failure_prevents_target_entry_for_continue_and_yield() {
    for (action,phase) in [(StateAction::Continue,"current exit"),(StateAction::Yield,"current yield")] {
        with_cx(|cx| {
            let log = Rc::new(RefCell::new(Vec::new())); let payload = TableRef::new();
            let next = probe("next", None, StateAction::Remain, &log, &payload, None);
            let current = probe("current", Some(Phase::Redirect), action, &log, &payload, Some(next));
            let mut machine = GameStateMachine { firstrun: false, states: vec![current.clone()], current: Some(current), ..Default::default() };
            machine.update(cx);
            stopped(&mut machine, cx, &payload, &log, &["current update",phase,"current redirect"]);
        });
    }
}

#[test]
fn redirected_entry_and_suspended_reentry_failure_clear_the_current_state() {
    for reenter in [false,true] {
        with_cx(|cx| {
            let log = Rc::new(RefCell::new(Vec::new())); let payload = TableRef::new();
            let next = probe("next", Some(if reenter { Phase::Reenter } else { Phase::Enter }), StateAction::Remain, &log, &payload, None);
            let current = probe("current", None, StateAction::Continue, &log, &payload, if reenter { None } else { Some(next.clone()) });
            let mut machine = GameStateMachine { firstrun: false, states: vec![current.clone()], current: Some(current), yield_stack: if reenter { vec![next] } else { Vec::new() }, ..Default::default() };
            machine.update(cx);
            let expected = if reenter { vec!["current update","current exit","next reenter"] } else { vec!["current update","current exit","current redirect","next enter"] };
            stopped(&mut machine, cx, &payload, &log, &expected);
        });
    }
}

#[test]
fn forced_exit_failure_does_not_enter_its_pending_target() {
    with_cx(|cx| {
        let log = Rc::new(RefCell::new(Vec::new())); let payload = TableRef::new();
        let next = probe("next", None, StateAction::Remain, &log, &payload, None);
        let current = probe("current", Some(Phase::Exit), StateAction::Remain, &log, &payload, None);
        let mut machine = GameStateMachine { firstrun: false, states: vec![current.clone()], current: Some(current), force_next: Some(next), ..Default::default() };
        machine.update(cx);
        stopped(&mut machine, cx, &payload, &log, &["current exit"]);
    });
}

#[test]
fn render_failure_stops_immediately_and_a_preexisting_failure_skips_render() {
    with_cx(|cx| {
        let log = Rc::new(RefCell::new(Vec::new())); let payload = TableRef::new();
        let current = probe("current", Some(Phase::Render), StateAction::Remain, &log, &payload, None);
        let mut machine = GameStateMachine { firstrun: false, states: vec![current.clone()], current: Some(current), ..Default::default() };
        machine.render(cx, &mut DrawList::default());
        stopped(&mut machine, cx, &payload, &log, &["current render"]);
    });
}
