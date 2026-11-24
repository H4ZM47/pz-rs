//! Build 42.22 `zombie.ui.TutorialManager`:
//! exposes `TutorialManager.instance`, its public flags, and its lifecycle
//! methods (`init`, `CreateQuests`, `update`, `AllowUse`, `CheckWake`), wired
//! into `GameLoadingState::run_inner` and `session_reset`.

use crate::expose::expose_class;
use pz_kahlua::value::LuaObject;
use pz_kahlua::{Thread, Value};
use std::any::Any;
use std::cell::RefCell;
use std::rc::Rc;

pub const TUTORIAL_MANAGER_CLASS: &str = "zombie.ui.TutorialManager";

#[derive(Clone, Debug)]
pub struct TutorialManagerState {
    pub debug: bool,
    pub active: bool,
    pub active_control_zombies: bool,
    pub target_zombies: f32,
    pub doors_locked: bool,
    pub barricade_count: i32,
    pub steal_control: bool,
    pub alarm_time: i32,
    pub profanity_filter: bool,
    pub timer: i32,
    pub alarm_tick_time: i32,
    pub done_first_sleep: bool,
    pub wife_killed_by_earl: bool,
    pub warned_hammer: bool,
    pub trigger_fire: bool,
    pub can_drag_wife: bool,
    pub allow_sleep: bool,
    pub skipped: bool,
    pub initialized: bool,
    pub quests_created: bool,
}

impl Default for TutorialManagerState {
    fn default() -> Self {
        Self {
            debug: false,
            active: false,
            active_control_zombies: false,
            target_zombies: 60.0,
            doors_locked: true,
            barricade_count: 0,
            steal_control: false,
            alarm_time: 0,
            profanity_filter: false,
            timer: 0,
            alarm_tick_time: 160,
            done_first_sleep: false,
            wife_killed_by_earl: false,
            warned_hammer: false,
            trigger_fire: false,
            can_drag_wife: false,
            allow_sleep: false,
            skipped: false,
            initialized: false,
            quests_created: false,
        }
    }
}

pub struct TutorialManagerObject;

impl LuaObject for TutorialManagerObject {
    fn class_name(&self) -> &str {
        TUTORIAL_MANAGER_CLASS
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

thread_local! {
    static STATE: RefCell<TutorialManagerState> = RefCell::new(TutorialManagerState::default());
    static INSTANCE: RefCell<Option<Rc<TutorialManagerObject>>> = const { RefCell::new(None) };
}

fn instance() -> Rc<TutorialManagerObject> {
    INSTANCE.with(|slot| {
        slot.borrow_mut()
            .get_or_insert_with(|| Rc::new(TutorialManagerObject))
            .clone()
    })
}

/// `TutorialManager.instance.init()`.
pub fn init() {
    let tutorial = crate::bind_core::shared_core().borrow().tutorial;
    STATE.with(|s| {
        let mut st = s.borrow_mut();
        st.active = tutorial;
        st.initialized = true;
    });
}

/// `TutorialManager.instance.CreateQuests()`.
pub fn create_quests() {
    STATE.with(|s| {
        s.borrow_mut().quests_created = true;
    });
}

/// `TutorialManager.instance.update()`.
pub fn update() {
    STATE.with(|s| {
        let mut st = s.borrow_mut();
        if st.active {
            st.timer = st.timer.saturating_add(1);
        }
    });
}

pub fn snapshot() -> TutorialManagerState {
    STATE.with(|s| s.borrow().clone())
}

pub(crate) fn reset_session_handles() {
    STATE.with(|s| *s.borrow_mut() = TutorialManagerState::default());
}

pub fn register(thread: &mut Thread) {
    let b = expose_class(thread, TUTORIAL_MANAGER_CLASS, Some("java.lang.Object"));
    b.field("instance", Value::Object(instance()));
    b.field("debug", false);
    b.method("init", |_c| {
        init();
        Ok(0)
    });
    b.method("CreateQuests", |_c| {
        create_quests();
        Ok(0)
    });
    b.method("update", |_c| {
        update();
        Ok(0)
    });
    b.method("AllowUse", |c| {
        let allow = STATE.with(|s| !s.borrow().active || !s.borrow().steal_control);
        Ok(c.push(allow))
    });
    b.method("CheckWake", |_c| Ok(0));
}

#[cfg(test)]
mod tests {
    use super::*;
    use pz_kahlua::{stdlib, TableRef};

    #[test]
    fn tutorial_manager_lifecycle_lua() {
        reset_session_handles();
        let mut t = Thread::new(TableRef::new());
        stdlib::install(&mut t, None).unwrap();
        register(&mut t);

        let f = stdlib::load_string(
            r#"
            local tm = TutorialManager.instance
            tm:init()
            tm:CreateQuests()
            tm:update()
            tm:CheckWake()
            return tm:AllowUse(nil)
            "#,
            Some("tutorial_mgr"),
            t.globals().clone(),
        )
        .unwrap();
        let r = t.pcall(&f, &[]).unwrap();
        assert!(r[0].truthy());
        let st = snapshot();
        assert!(st.initialized);
        assert!(st.quests_created);
    }
}
