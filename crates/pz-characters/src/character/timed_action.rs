//! State of the character's running `BaseAction` objects. The Lua queue of
//! future actions is separate; the host calls Lua only outside character borrows.

use pz_inventory::shared::ItemRef;
use pz_kahlua::{LuaObject, TableRef};
use std::cell::RefCell;
use std::rc::{Rc, Weak};

pub type ActionRef = Rc<RefCell<TimedAction>>;

pub struct TimedAction {
    /// BaseAction.chr retains its real character while an action is retained.
    pub character: Option<pz_kahlua::Value>,
    pub table: Option<TableRef>,
    pub handle: Option<Weak<dyn LuaObject>>,
    pub current_time: f32,
    pub last_time: f32,
    pub prev_last_time: f32,
    pub max_time: i32,
    pub delta: f32,
    pub started: bool,
    pub force_stop: bool,
    pub force_complete: bool,
    pub wait_for_finished: bool,
    pub loop_action: bool,
    pub stop_on_walk: bool,
    pub stop_on_run: bool,
    pub stop_on_aim: bool,
    pub use_progress_bar: bool,
    pub force_progress_bar: bool,
    pub block_movement: bool,
    pub override_animation: bool,
    pub calories_modifier: f32,
    pub pathfinding: bool,
    pub allowed_while_dragging_corpses: bool,
    pub anim_variables: Vec<String>,
    pub override_hand_models: bool,
    pub primary_item: Option<ItemRef>,
    pub secondary_item: Option<ItemRef>,
    pub primary_model: Option<String>,
    pub secondary_model: Option<String>,
}

impl Default for TimedAction {
    fn default() -> Self {
        Self {
            character: None,
            table: None,
            handle: None,
            current_time: -2.0,
            last_time: -1.0,
            prev_last_time: 0.0,
            max_time: 60,
            delta: 0.0,
            started: false,
            force_stop: false,
            force_complete: false,
            wait_for_finished: false,
            loop_action: false,
            stop_on_walk: true,
            stop_on_run: true,
            stop_on_aim: false,
            use_progress_bar: true,
            force_progress_bar: false,
            block_movement: false,
            override_animation: false,
            calories_modifier: 1.0,
            pathfinding: false,
            allowed_while_dragging_corpses: false,
            anim_variables: Vec::new(),
            override_hand_models: false,
            primary_item: None,
            secondary_item: None,
            primary_model: None,
            secondary_model: None,
        }
    }
}

impl TimedAction {
    pub fn finished(&self) -> bool {
        !self.wait_for_finished && self.max_time != -1 && self.current_time >= self.max_time as f32
    }

    pub fn has_stalled(&self) -> bool {
        self.started
            && ((self.last_time == self.current_time
                && self.last_time == self.prev_last_time
                && self.last_time < 0.0)
                || self.current_time < 0.0)
    }

    /// Base update precedes the Lua update callback. Infinite actions keep
    /// their current delta and use the indeterminate progress-bar value.
    pub fn update_time(&mut self, multiplier: f32) {
        self.prev_last_time = self.last_time;
        self.last_time = self.current_time;
        self.current_time += multiplier;
        if self.current_time < 0.0 {
            self.current_time = 0.0;
        }
        if self.max_time != -1 {
            self.delta = if self.max_time == 0 {
                0.0
            } else {
                let ratio = self.current_time / self.max_time as f32;
                if ratio.is_nan() {
                    ratio
                } else {
                    ratio.min(1.0)
                }
            };
        }
    }
}
