//! `zombie.gameStates.GameState` and `GameStateMachine`: the game's top
//! level states (`MainScreenState`, `LoadingQueueState`,
//! `GameLoadingState`, `IngameState`) and how `GameWindow.logic`/`render`
//! step them.

use crate::game_window::Launch;
use crate::iso_world::IsoWorld;
use pz_lua_api::LuaManager;
use pz_render::DrawList;
use std::cell::RefCell;
use std::rc::Rc;

/// `GameStateMachine.StateAction`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StateAction {
    /// Leave the state (`exit`), then re-enter the yielded one or go to
    /// `redirectState()` / the next listed state.
    Continue,
    /// Stay in the state.
    Remain,
    /// Suspend the state (`yield`) and go to `redirectState()`; it is
    /// re-entered (`reenter`) when that one continues.
    Yield,
}

/// What a state's methods reach: the engine's singletons the original's
/// states read as statics.
pub struct StateCx<'a> {
    pub lua: &'a mut LuaManager,
    pub launch: &'a Launch,
    /// `Core.getInstance().getScreenWidth()` / `getScreenHeight()`.
    pub screen_w: i32,
    pub screen_h: i32,
    /// `GameTime.getInstance().getMultiplierInMenu()` (30 FPS frames).
    pub multiplier: f32,
    /// `GameTime.instance.getTimeDeltaInMenu()`: real seconds since the
    /// last frame.
    pub time_delta: f32,
    /// `IsoWorld.instance` (`None` until a game loads).
    pub world: &'a mut Option<IsoWorld>,
    /// A native window close disposes the session without rebuilding the
    /// script/Lua environment for a return to the main menu.
    pub shutdown: bool,
    /// A failed environment rebuild stops state transitions. The window
    /// or headless caller reports the error after owners are retired.
    pub error: &'a mut Option<String>,
    /// Models retired at the actual state-exit boundary. The window
    /// releases their GPU resources after returning from the state call.
    pub retired_models: &'a mut Vec<crate::world_models::WorldModels>,
}

/// `zombie.gameStates.GameState`; every method defaults to the base
/// class's (empty, `update` continues at once).
pub trait GameState {
    /// The Java class name `GameStateMachine` prints (`STATE: exit ...`).
    fn class_name(&self) -> &'static str;
    fn enter(&mut self, _cx: &mut StateCx<'_>) {}
    fn exit(&mut self, _cx: &mut StateCx<'_>) {}
    fn render(&mut self, _cx: &mut StateCx<'_>, _out: &mut DrawList) {}
    fn redirect_state(&mut self, _cx: &mut StateCx<'_>) -> Option<StateRef> {
        None
    }
    fn update(&mut self, _cx: &mut StateCx<'_>) -> StateAction {
        StateAction::Continue
    }
    fn yield_state(&mut self, _cx: &mut StateCx<'_>) {}
    fn reenter(&mut self, _cx: &mut StateCx<'_>) {}
}

/// A state object; states are compared by identity, as in Java.
pub type StateRef = Rc<RefCell<dyn GameState>>;

/// Wraps a state for the machine.
pub fn state(s: impl GameState + 'static) -> StateRef {
    Rc::new(RefCell::new(s))
}

/// `zombie.gameStates.GameStateMachine` (`GameWindow.states`).
pub struct GameStateMachine {
    pub firstrun: bool,
    /// `loop`: start again at `loop_to_state` after the last state.
    pub looping: bool,
    pub state_index: usize,
    pub loop_to_state: usize,
    pub states: Vec<StateRef>,
    yield_stack: Vec<StateRef>,
    pub current: Option<StateRef>,
    pub force_next: Option<StateRef>,
}

impl Default for GameStateMachine {
    fn default() -> Self {
        GameStateMachine {
            firstrun: true,
            looping: true,
            state_index: 0,
            loop_to_state: 0,
            states: Vec::new(),
            yield_stack: Vec::new(),
            current: None,
            force_next: None,
        }
    }
}

impl GameStateMachine {
    /// `render()`.
    pub fn render(&mut self, cx: &mut StateCx<'_>, out: &mut DrawList) {
        if let Some(c) = self.current.clone() {
            c.borrow_mut().render(cx, out);
        }
    }

    /// `forceNextState(state)` (Lua's `forceChangeState`).
    pub fn force_next_state(&mut self, s: StateRef) {
        self.force_next = Some(s);
    }

    /// Whether `current` is `s`.
    pub fn is_current(&self, s: &StateRef) -> bool {
        self.current.as_ref().is_some_and(|c| Rc::ptr_eq(c, s))
    }

    /// The current state's class name.
    pub fn current_name(&self) -> Option<&'static str> {
        self.current.as_ref().map(|c| c.borrow().class_name())
    }

    fn enter(s: &StateRef, cx: &mut StateCx<'_>) {
        s.borrow_mut().enter(cx);
    }

    /// `update()`.
    pub fn update(&mut self, cx: &mut StateCx<'_>) {
        if self.stop_on_error(cx) { return; }
        if self.states.is_empty() {
            let Some(f) = self.force_next.take() else { return };
            self.states.push(f);
        }
        if self.firstrun {
            if self.current.is_none() {
                self.current = self.states.get(self.state_index).cloned();
            }
            if let Some(c) = self.current.clone() {
                Self::enter(&c, cx);
            }
            self.firstrun = false;
        }
        if self.current.is_none() {
            if !self.looping {
                return;
            }
            self.state_index = self.loop_to_state;
            if self.states.is_empty() {
                return;
            }
            self.current = self.states.get(self.state_index).cloned();
            if let Some(c) = self.current.clone() {
                Self::enter(&c, cx);
            }
        }
        let Some(current) = self.current.clone() else { return };
        let next;
        if let Some(f) = self.force_next.take() {
            println!("STATE: exit {}", current.borrow().class_name());
            current.borrow_mut().exit(cx);
            if self.stop_on_error(cx) { return; }
            next = Some(f);
        } else {
            let action = current.borrow_mut().update(cx);
            match action {
                StateAction::Continue => {
                    println!("STATE: exit {}", current.borrow().class_name());
                    current.borrow_mut().exit(cx);
                    if self.stop_on_error(cx) { return; }
                    if let Some(y) = self.yield_stack.pop() {
                        println!("STATE: reenter {}", y.borrow().class_name());
                        self.current = Some(y.clone());
                        y.borrow_mut().reenter(cx);
                        return;
                    }
                    next = current.borrow_mut().redirect_state(cx);
                }
                StateAction::Yield => {
                    println!("STATE: yield {}", current.borrow().class_name());
                    current.borrow_mut().yield_state(cx);
                    self.yield_stack.push(current.clone());
                    next = current.borrow_mut().redirect_state(cx);
                }
                StateAction::Remain => return,
            }
        }
        if let Some(n) = next {
            Self::enter(&n, cx);
            self.current = Some(n);
            return;
        }
        self.state_index += 1;
        if self.state_index < self.states.len() {
            let c = self.states[self.state_index].clone();
            self.current = Some(c.clone());
            Self::enter(&c, cx);
            return;
        }
        self.current = None;
    }

    fn stop_on_error(&mut self, cx: &StateCx<'_>) -> bool {
        if cx.error.is_none() { return false; }
        self.current = None;
        self.force_next = None;
        self.yield_stack.clear();
        self.looping = false;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A state that records its calls and answers `update` from a script.
    struct Probe {
        name: &'static str,
        log: Rc<RefCell<Vec<String>>>,
        actions: Vec<StateAction>,
        redirect: Option<StateRef>,
    }

    impl GameState for Probe {
        fn class_name(&self) -> &'static str {
            self.name
        }
        fn enter(&mut self, _cx: &mut StateCx<'_>) {
            self.log.borrow_mut().push(format!("{} enter", self.name));
        }
        fn exit(&mut self, _cx: &mut StateCx<'_>) {
            self.log.borrow_mut().push(format!("{} exit", self.name));
        }
        fn reenter(&mut self, _cx: &mut StateCx<'_>) {
            self.log.borrow_mut().push(format!("{} reenter", self.name));
        }
        fn yield_state(&mut self, _cx: &mut StateCx<'_>) {
            self.log.borrow_mut().push(format!("{} yield", self.name));
        }
        fn update(&mut self, _cx: &mut StateCx<'_>) -> StateAction {
            self.log.borrow_mut().push(format!("{} update", self.name));
            if self.actions.is_empty() {
                StateAction::Remain
            } else {
                self.actions.remove(0)
            }
        }
        fn redirect_state(&mut self, _cx: &mut StateCx<'_>) -> Option<StateRef> {
            self.redirect.take()
        }
    }

    fn probe(name: &'static str, log: &Rc<RefCell<Vec<String>>>, actions: &[StateAction], redirect: Option<StateRef>) -> StateRef {
        state(Probe { name, log: log.clone(), actions: actions.to_vec(), redirect })
    }

    fn with_cx(f: impl FnOnce(&mut StateCx<'_>)) {
        let fs = Rc::new(RefCell::new(pz_fs::GameFs::new(std::env::temp_dir())));
        let mut lua = LuaManager::init(fs).unwrap();
        let launch = Launch { game_dir: std::env::temp_dir(), cache_dir: std::env::temp_dir(), start: Default::default() };
        let mut world = None;
        let mut retired_models = Vec::new();
        let mut error = None;
        let mut cx = StateCx { lua: &mut lua, launch: &launch, screen_w: 1, screen_h: 1, multiplier: 1.0, time_delta: 1.0 / 30.0, world: &mut world, shutdown: false, error: &mut error, retired_models: &mut retired_models };
        f(&mut cx);
    }

    #[test]
    fn a_quit_requested_during_the_tick_retires_the_actual_world_models() {
        with_cx(|cx| {
            let mut world = IsoWorld::new(&cx.launch.game_dir, &cx.launch.cache_dir, cx.launch.cache_dir.join("pz-rs-retire-test"));
            world.models = Some(crate::world_models::WorldModels::new(
                &cx.launch.game_dir, cx.lua.fs.clone(), pz_lua_api::bind_render::textures(),
            ));
            *cx.world = Some(world);
            let f = pz_kahlua::stdlib::load_string(
                "Events.OnTickEvenPaused.Add(function() getCore():exitToMenu() end)",
                Some("quit-during-tick"), cx.lua.thread.globals().clone(),
            ).unwrap();
            cx.lua.thread.call(&f, &[]).unwrap();
            let ingame = state(crate::ingame::IngameState::default());
            let mut states = GameStateMachine {
                firstrun: false, states: vec![ingame.clone()], current: Some(ingame),
                ..Default::default()
            };
            states.update(cx);
            assert!(cx.world.is_none());
            assert_eq!(cx.retired_models.len(), 1);
            // This authored fixture has no installed scripts to rebuild.
            // Retirement must still complete before the failure is reported.
            assert_eq!(states.current_name(), None);
            assert!(cx.error.is_some());
        });
    }

    #[test]
    fn listed_states_redirects_yields_and_force_next() {
        use StateAction::*;
        let log: Rc<RefCell<Vec<String>>> = Rc::default();
        with_cx(|cx| {
            let mut m = GameStateMachine::default();
            let c = probe("C", &log, &[Continue], None);
            let b = probe("B", &log, &[Yield, Continue], Some(c));
            let a = probe("A", &log, &[Continue], None);
            m.states.push(a);
            m.states.push(b);
            for _ in 0..3 {
                m.update(cx);
            }
            assert_eq!(m.current_name(), Some("B"));
            let f = probe("F", &log, &[], None);
            m.force_next_state(f.clone());
            m.update(cx);
            assert!(m.is_current(&f));
        });
        let got = log.borrow().join(", ");
        assert_eq!(
            got,
            "A enter, A update, A exit, B enter, B update, B yield, C enter, C update, C exit, B reenter, \
             B exit, F enter"
        );
    }

    #[test]
    fn empty_machine_takes_the_forced_state() {
        let log: Rc<RefCell<Vec<String>>> = Rc::default();
        with_cx(|cx| {
            let mut m = GameStateMachine::default();
            m.update(cx);
            assert!(m.current.is_none());
            m.force_next_state(probe("Q", &log, &[], None));
            m.update(cx);
            assert_eq!(m.current_name(), Some("Q"));
        });
        assert_eq!(log.borrow().join(", "), "Q enter, Q update");
    }
}
