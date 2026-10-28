//! Live character action stack and local LuaTimedActionNew runtime. Lua's
//! ISTimedActionQueue remains the owner of pending actions and transitions.
//! Every protected callback runs without character, action, or stack borrows.

use crate::bind_character_sim::{
    character_value, chr_of, CharacterObject, LuaGameCharacterRef, ISO_GAME_CHARACTER_CLASS,
};
use crate::expose::{bool_arg, expose_class, int_arg, num_arg, str_arg, with_this};
use pz_anim::AnimationVariableSource;
use pz_characters::character::timed_action::{ActionRef, TimedAction};
use pz_kahlua::{LuaError, LuaObject, LuaResult, NativeCall, Thread, Value};
use std::any::Any;
use std::cell::RefCell;
use std::rc::Rc;

const BASE: &str = "zombie.characters.CharacterTimedActions.BaseAction";
const LUA: &str = "zombie.characters.CharacterTimedActions.LuaTimedActionNew";
const STACK: &str = "java.util.Stack";
const DELTAS: &str = "zombie.characters.MoveDeltaModifiers";
const ACTION_ANIMS: &str = "zombie.characters.CharacterActionAnims";

/// Load the actual enum names from the installed class, without embedding
/// game-specific declarations or replacing enum instances with strings.
pub(crate) fn register_action_anims(thread: &mut Thread, jar: &pz_classfile::Jar) {
    thread_local! { static NAMES: RefCell<std::collections::HashMap<String, &'static str>> = RefCell::default(); }
    let Ok(names) = pz_classfile::enum_constants(jar, "zombie/characters/CharacterActionAnims")
    else {
        return;
    };
    let names: Vec<_> = names
        .into_iter()
        .map(|name| {
            NAMES.with(|cache| {
                *cache
                    .borrow_mut()
                    .entry(name.clone())
                    .or_insert_with(|| Box::leak(name.into_boxed_str()))
            })
        })
        .collect();
    crate::expose::expose_enum(thread, ACTION_ANIMS, &names);
}

struct DeltaObject(Rc<RefCell<pz_characters::character::state::MoveDeltaModifiers>>);
impl LuaObject for DeltaObject {
    fn class_name(&self) -> &str {
        DELTAS
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}
thread_local! {
    static LIVE_DELTAS: Rc<RefCell<pz_characters::character::state::MoveDeltaModifiers>> = Rc::default();
    static LIVE_DELTAS_VALUE: Value = LIVE_DELTAS.with(|d| Value::Object(Rc::new(DeltaObject(d.clone()))));
}

pub fn movement_modifiers(
    thread: &mut Thread,
    chr: &LuaGameCharacterRef,
    initial: pz_characters::character::state::MoveDeltaModifiers,
) -> pz_characters::character::state::MoveDeltaModifiers {
    let front = chr.borrow().character_actions.borrow().first().cloned();
    let Some(a) = front else { return initial };
    if a.borrow().finished()
        || !a
            .borrow()
            .table
            .as_ref()
            .is_some_and(|t| !t.get_str("getDeltaModifiers").is_nil())
    {
        return initial;
    }
    let deltas = LIVE_DELTAS.with(Rc::clone);
    *deltas.borrow_mut() = initial;
    let value = LIVE_DELTAS_VALUE.with(Clone::clone);
    callback(thread, &a, "getDeltaModifiers", &[value]);
    let value = *deltas.borrow();
    value
}

struct ActionObject(ActionRef);
impl LuaObject for ActionObject {
    fn class_name(&self) -> &str {
        if self.0.borrow().table.is_some() {
            LUA
        } else {
            BASE
        }
    }
    fn super_classes(&self) -> &[&'static str] {
        &[BASE, "java.lang.Object"]
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

fn action_value(action: &ActionRef) -> Value {
    if let Some(handle) = action.borrow().handle.as_ref().and_then(|h| h.upgrade()) {
        return Value::Object(handle);
    }
    let handle: Rc<dyn LuaObject> = Rc::new(ActionObject(action.clone()));
    action.borrow_mut().handle = Some(Rc::downgrade(&handle));
    Value::Object(handle)
}

fn action(v: &Value) -> LuaResult<ActionRef> {
    if let Value::Object(o) = v {
        if let Some(o) = o.as_any().downcast_ref::<ActionObject>() {
            return Ok(o.0.clone());
        }
    }
    Err(LuaError::runtime("Expected BaseAction"))
}
fn this(c: &NativeCall<'_>) -> LuaResult<ActionRef> {
    action(&c.arg(0))
}
fn character(v: &Value) -> LuaResult<LuaGameCharacterRef> {
    if let Value::Object(o) = v {
        if let Some(o) = o.as_any().downcast_ref::<CharacterObject>() {
            return Ok(o.0.clone());
        }
    }
    Err(LuaError::runtime("Expected IsoGameCharacter"))
}
fn owner(action: &ActionRef) -> LuaGameCharacterRef {
    character(
        action
            .borrow()
            .character
            .as_ref()
            .expect("action character installed"),
    )
    .expect("action character")
}
fn callback(
    thread: &mut Thread,
    action: &ActionRef,
    name: &str,
    extra: &[Value],
) -> Option<Vec<Value>> {
    let table = action.borrow().table.clone()?;
    let f = table.rawget(&Value::str(name));
    let mut args = vec![Value::Table(table)];
    args.extend_from_slice(extra);
    match thread.pcall(&f, &args) {
        Ok(values) => Some(values),
        Err((message, trace)) => {
            (thread.log)(&format!(
                "{}\n{trace}",
                pz_kahlua::exec::java_value_of(&message)
            ));
            None
        }
    }
}
fn valid(thread: &mut Thread, a: &ActionRef) -> bool {
    let Some(table) = a.borrow().table.clone() else {
        return true;
    };
    let f = table.get_str("isValid");
    // Original valid() inspects pcall's value slot without inspecting its
    // success flag. A boolean-valued Lua error therefore also reaches here.
    match thread.pcall(&f, &[Value::Table(table)]) {
        Ok(values) => matches!(values.first(), Some(Value::Bool(true))),
        Err((message, trace)) => {
            (thread.log)(&format!(
                "{}\n{trace}",
                pz_kahlua::exec::java_value_of(&message)
            ));
            matches!(message, Value::Bool(true))
        }
    }
}
fn progress(chr: &LuaGameCharacterRef, value: f32) {
    let chr = chr.borrow();
    if chr.is_player() && chr.in_players {
        crate::hud::try_set_progress_bar_value(chr.player.player_index.max(0) as usize, value);
    }
}
fn cleanup(a: &ActionRef) {
    let chr = owner(a);
    let names = {
        let mut a = a.borrow_mut();
        a.override_hand_models = false;
        a.anim_variables.clone()
    };
    let mut chr = chr.borrow_mut();
    for key in names {
        chr.clear_variable(&key);
    }
    chr.set_variable_bool("IsPerformingAnAction", false);
}
fn wait_to_start(thread: &mut Thread, a: &ActionRef) {
    if a.borrow().table.is_some()
        && !matches!(
            callback(thread, a, "waitToStart", &[]).as_deref(),
            Some([Value::Bool(false), ..])
        )
    {
        return;
    }
    let chr = owner(a);
    let waiting = {
        let chr = chr.borrow();
        chr.sit_on_ground
            && !chr.is_current_state_id(pz_characters::character::states::ids::FISHING)
            && chr.wait_to_start_timed_action
    };
    if waiting {
        return;
    }
    {
        let mut chr = chr.borrow_mut();
        if chr.is_current_state_id(pz_characters::character::states::ids::PLAYER_ACTIONS) {
            let weapon = |item: Option<pz_inventory::shared::ItemRef>| {
                item.is_some_and(|i| {
                    matches!(i.borrow().kind, pz_inventory::item::ItemKind::Weapon(_))
                })
            };
            chr.vars.hide_weapon_model =
                !weapon(chr.items.primary_hand_item()) && !weapon(chr.items.secondary_hand_item());
        }
    }
    {
        let mut a = a.borrow_mut();
        a.started = true;
        a.force_complete = false;
        a.force_stop = false;
        if a.table.is_some() {
            a.current_time = 0.0;
        }
    }
    callback(thread, a, "start", &[]);
    sync_flags(&chr);
}
fn stop_action(thread: &mut Thread, a: &ActionRef) {
    progress(&owner(a), 0.0);
    cleanup(a);
    callback(thread, a, "stop", &[]);
}

pub fn sync_flags(chr: &LuaGameCharacterRef) {
    let front = chr.borrow().character_actions.borrow().first().cloned();
    let mut chr = chr.borrow_mut();
    chr.player.has_actions = front.is_some();
    chr.vars.has_timed_actions = front.is_some() || chr.vars.performing_an_action;
    chr.player.action_blocks_movement = front.as_ref().is_some_and(|a| a.borrow().block_movement);
    chr.player.action_overrides_animation = front
        .as_ref()
        .is_some_and(|a| a.borrow().override_animation);
    chr.environment.busy_with_action = front.is_some();
    chr.environment.first_action_calories_modifier =
        front.as_ref().map(|a| a.borrow().calories_modifier);
}

/// Clear only after callbacks, matching StopAllActionQueue's reentrant order.
pub fn cancel(thread: &mut Thread, chr: &LuaGameCharacterRef, reason: &str) {
    let stack = chr.borrow().character_actions.clone();
    let Some(a) = stack.borrow().first().cloned() else {
        return;
    };
    let allowed = {
        let a = a.borrow();
        match reason {
            "Running" => a.stop_on_run,
            "Walking" => a.stop_on_walk,
            "Aiming" => a.stop_on_aim,
            _ => true,
        }
    };
    if !allowed {
        return;
    }
    if a.borrow().started {
        stop_action(thread, &a);
    } else if reason != "Aiming" {
        callback(thread, &a, "interruptWaitToStart", &[]);
    }
    stack.borrow_mut().clear();
    progress(chr, 0.0);
    sync_flags(chr);
}

pub fn force_stop(chr: &LuaGameCharacterRef) {
    if let Some(a) = chr.borrow().character_actions.borrow().first() {
        a.borrow_mut().force_stop = true;
    }
}

/// The original action block in updateInternal, between the two character
/// update phases. Re-read flags after callbacks and remove the old identity.
pub fn update(thread: &mut Thread, chr: &LuaGameCharacterRef) {
    update_with_clock(thread, chr, || {
        crate::bind_climate::shared_game_time()
            .borrow()
            .get_multiplier(&crate::bind_climate::LuaClimateContext::new())
    });
}

/// Deterministic host clock for authored fixtures; the production update
/// resolves the actual GameTime multiplier after validity callbacks return.
pub fn update_with_multiplier(thread: &mut Thread, chr: &LuaGameCharacterRef, multiplier: f32) {
    update_with_clock(thread, chr, || multiplier);
}

fn update_with_clock(
    thread: &mut Thread,
    chr: &LuaGameCharacterRef,
    mut multiplier: impl FnMut() -> f32,
) {
    let stack = chr.borrow().character_actions.clone();
    let Some(a) = stack.borrow().first().cloned() else {
        sync_flags(chr);
        return;
    };
    if chr
        .borrow()
        .timed_action_to_retrigger
        .as_ref()
        .is_some_and(|b| Rc::ptr_eq(&a, b))
    {
        chr.borrow_mut().timed_action_to_retrigger = None;
    }
    let mut is_valid = valid(thread, &a);
    if is_valid && !a.borrow().started {
        wait_to_start(thread, &a);
    } else if is_valid && {
        let a = a.borrow();
        !a.finished() && !a.force_complete && !a.force_stop
    } {
        a.borrow_mut().update_time(multiplier());
        let (show, delta) = {
            let a = a.borrow();
            (
                a.use_progress_bar
                    && (a.force_progress_bar
                        || crate::bind_core::shared_core()
                            .borrow()
                            .is_option_progress_bar()),
                if a.max_time == -1 {
                    f32::INFINITY
                } else {
                    a.delta
                },
            )
        };
        if show {
            progress(chr, delta);
        }
        callback(thread, &a, "update", &[]);
    }
    if !is_valid || {
        let a = a.borrow();
        a.finished() || a.force_complete || a.force_stop
    } {
        if {
            let a = a.borrow();
            a.finished() || a.force_complete
        } {
            progress(chr, 1.0);
            if !a.borrow().loop_action {
                cleanup(&a);
            }
            callback(thread, &a, "perform", &[]);
            callback(thread, &a, "complete", &[]);
            is_valid = true;
        }
        let remove = {
            let a = a.borrow();
            ((a.finished() || a.force_complete) && !a.loop_action) || a.force_stop || !is_valid
        };
        if remove {
            if {
                let a = a.borrow();
                a.started && (a.force_stop || !is_valid)
            } {
                stop_action(thread, &a);
            }
            let index = stack.borrow().iter().position(|b| Rc::ptr_eq(&a, b));
            if let Some(index) = index {
                stack.borrow_mut().remove(index);
            }
            progress(chr, 0.0);
        }
        a.borrow_mut().force_complete = false;
    }
    sync_flags(chr);
}

pub fn anim_event(
    thread: &mut Thread,
    chr: &LuaGameCharacterRef,
    name: &str,
    parameter: Option<&str>,
) {
    let front = chr.borrow().character_actions.borrow().first().cloned();
    if let Some(a) = front {
        if a.borrow()
            .table
            .as_ref()
            .is_some_and(|t| !t.get_str("animEvent").is_nil())
        {
            callback(
                thread,
                &a,
                "animEvent",
                &[
                    Value::str(name),
                    parameter.map(Value::str).unwrap_or_default(),
                ],
            );
        }
    }
}

struct StackObject(Rc<RefCell<Vec<ActionRef>>>);
impl LuaObject for StackObject {
    fn class_name(&self) -> &str {
        STACK
    }
    fn super_classes(&self) -> &[&'static str] {
        &[
            "java.util.Vector",
            "java.util.AbstractList",
            "java.util.List",
            "java.util.Collection",
        ]
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}
fn stack(c: &NativeCall<'_>) -> LuaResult<Rc<RefCell<Vec<ActionRef>>>> {
    with_this::<StackObject, _>(c, |s| s.0.clone())
}

pub fn register(thread: &mut Thread) {
    let d = expose_class(thread, DELTAS, Some("java.lang.Object"));
    for (get, set, max, index) in [
        ("getMoveDelta", "setMoveDelta", "setMaxMoveDelta", 0),
        ("getTurnDelta", "setTurnDelta", "setMaxTurnDelta", 1),
        ("getTwistDelta", "setTwistDelta", "setMaxTwistDelta", 2),
    ] {
        d.method(get, move |c| {
            let value = with_this::<DeltaObject, _>(c, |o| {
                let d = o.0.borrow();
                match index {
                    0 => d.move_delta,
                    1 => d.turn_delta,
                    _ => d.twist_delta,
                }
            })?;
            Ok(c.push(value as f64))
        });
        d.method(set, move |c| {
            let value = num_arg(c, 1) as f32;
            with_this::<DeltaObject, _>(c, |o| {
                let mut d = o.0.borrow_mut();
                match index {
                    0 => d.move_delta = value,
                    1 => d.turn_delta = value,
                    _ => d.twist_delta = value,
                }
            })?;
            Ok(0)
        });
        d.method(max, move |c| {
            let value = num_arg(c, 1) as f32;
            with_this::<DeltaObject, _>(c, |o| {
                let mut d = o.0.borrow_mut();
                match index {
                    0 => d.set_max_move_delta(value),
                    1 => d.set_max_turn_delta(value),
                    _ => d.set_max_twist_delta(value),
                }
            })?;
            Ok(0)
        });
    }
    let b = expose_class(thread, BASE, Some("java.lang.Object"));
    for (name, get) in [
        ("isStarted", 0),
        ("isForceComplete", 1),
        ("hasStalled", 2),
        ("finished", 3),
        ("isPathfinding", 4),
        ("isAllowedWhileDraggingCorpses", 5),
    ] {
        b.method(name, move |c| {
            let a = this(c)?;
            let a = a.borrow();
            Ok(c.push(match get {
                0 => a.started,
                1 => a.force_complete,
                2 => a.has_stalled(),
                3 => a.finished(),
                4 => a.pathfinding,
                _ => a.allowed_while_dragging_corpses,
            }))
        });
    }
    for (name, get) in [("getJobDelta", 0), ("getCurrentTime", 1)] {
        b.method(name, move |c| {
            let a = this(c)?;
            let a = a.borrow();
            Ok(c.push(match get {
                0 => a.delta,
                1 => a.current_time,
                _ => unreachable!(),
            } as f64))
        });
    }
    for (name, which) in [
        ("forceStop", 0),
        ("forceComplete", 1),
        ("setWaitForFinished", 2),
        ("setUseProgressBar", 3),
        ("setBlockMovementEtc", 4),
        ("setOverrideAnimation", 5),
        ("setPathfinding", 6),
        ("setAllowedWhileDraggingCorpses", 7),
        ("setLoopedAction", 8),
    ] {
        b.method(name, move |c| {
            let a = this(c)?;
            let mut a = a.borrow_mut();
            let v = bool_arg(c, 1);
            match which {
                0 => a.force_stop = true,
                1 => a.force_complete = true,
                2 => a.wait_for_finished = v,
                3 => a.use_progress_bar = v,
                4 => a.block_movement = v,
                5 => a.override_animation = v,
                6 => a.pathfinding = v,
                7 => a.allowed_while_dragging_corpses = v,
                _ => a.loop_action = v,
            };
            drop(a);
            sync_flags(&owner(&this(c)?));
            Ok(0)
        });
    }
    b.method("setJobDelta", |c| {
        let a = this(c)?;
        let mut a = a.borrow_mut();
        a.delta = num_arg(c, 1) as f32;
        a.current_time = a.max_time as f32 * a.delta;
        Ok(0)
    });
    b.method("resetJobDelta", |c| {
        let a = this(c)?;
        let mut a = a.borrow_mut();
        a.delta = 0.0;
        a.current_time = 0.0;
        Ok(0)
    });
    b.method("setAnimVariable", |c| {
        let a = this(c)?;
        let key = str_arg(c, 1).unwrap_or_default().to_string();
        let value = match c.arg(2) {
            Value::Bool(v) => v.to_string(),
            v => pz_kahlua::exec::java_value_of(&v),
        };
        {
            let mut a = a.borrow_mut();
            if !a.anim_variables.contains(&key) {
                a.anim_variables.push(key.clone());
            }
        }
        owner(&a).borrow_mut().set_variable_string(&key, &value);
        Ok(0)
    });
    b.method("setActionAnim", |c| {
        let a = this(c)?;
        let anim = match c.arg(1) {
            Value::Str(s) => s.to_string(),
            Value::Object(o) => o
                .as_any()
                .downcast_ref::<crate::expose::JavaEnum>()
                .filter(|e| e.class == ACTION_ANIMS)
                .map(|e| e.name.to_string())
                .ok_or_else(|| LuaError::runtime("Expected String or CharacterActionAnims"))?,
            _ => return Err(LuaError::runtime("Expected String or CharacterActionAnims")),
        };
        {
            let mut a = a.borrow_mut();
            if !a.anim_variables.iter().any(|s| s == "PerformingAction") {
                a.anim_variables.push("PerformingAction".into());
            }
        }
        let chr = owner(&a);
        {
            let mut chr = chr.borrow_mut();
            chr.set_variable_string("PerformingAction", &anim);
            chr.set_variable_bool("IsPerformingAnAction", true);
        }
        sync_flags(&chr);
        Ok(0)
    });
    b.method("stopTimedActionAnim", |c| {
        cleanup(&this(c)?);
        Ok(0)
    });
    for name in [
        "setOverrideHandModels",
        "setOverrideHandModelsObject",
        "setOverrideHandModelsString",
    ] {
        b.method(name, |c| {
            let a = this(c)?;
            let mut a = a.borrow_mut();
            a.override_hand_models = true;
            a.primary_item = crate::bind_items::item_arg_value(&c.arg(1));
            a.secondary_item = crate::bind_items::item_arg_value(&c.arg(2));
            a.primary_model = c
                .arg(1)
                .as_str()
                .filter(|s| !s.trim().is_empty())
                .map(str::to_string);
            a.secondary_model = c
                .arg(2)
                .as_str()
                .filter(|s| !s.trim().is_empty())
                .map(str::to_string);
            Ok(0)
        });
    }
    for (name, secondary) in [("getPrimaryHandMdl", false), ("getSecondaryHandMdl", true)] {
        b.method(name, move |c| {
            let a = this(c)?;
            let a = a.borrow();
            let s = if secondary {
                &a.secondary_model
            } else {
                &a.primary_model
            };
            Ok(c.push(s.as_deref().map(Value::str).unwrap_or_default()))
        });
    }
    let l = expose_class(thread, LUA, Some(BASE));
    l.method("getTime", |c| {
        let value = this(c)?.borrow().max_time;
        Ok(c.push(value))
    });
    l.method("setTime", |c| {
        this(c)?.borrow_mut().max_time = int_arg(c, 1);
        Ok(0)
    });
    l.method("setCurrentTime", |c| {
        let a = this(c)?;
        let mut a = a.borrow_mut();
        let value = num_arg(c, 1) as f32;
        a.current_time = if value < 0.0 {
            0.0
        } else if value > a.max_time as f32 {
            a.max_time as f32
        } else {
            value
        };
        Ok(0)
    });
    l.method("getTable", |c| {
        let table = this(c)?.borrow().table.clone();
        Ok(c.push(table.map(Value::Table).unwrap_or_default()))
    });
    l.ctor(|c| {
        let table = c
            .arg(0)
            .as_table()
            .cloned()
            .ok_or_else(|| LuaError::runtime("Expected KahluaTable"))?;
        if table.metatable().is_none() {
            return Err(LuaError::runtime(
                "LuaTimedActionNew table has no metatable",
            ));
        }
        let chr = character(&c.arg(1))?;
        let max_time = table
            .get_str("maxTime")
            .as_num()
            .ok_or_else(|| LuaError::runtime("LuaTimedActionNew.maxTime must be a number"))?
            as i32;
        let mut a = TimedAction {
            character: Some(character_value(&chr)),
            max_time: max_time,
            ..Default::default()
        };
        for (key, field) in [
            ("stopOnWalk", &mut a.stop_on_walk),
            ("stopOnRun", &mut a.stop_on_run),
            ("stopOnAim", &mut a.stop_on_aim),
            ("useProgressBar", &mut a.use_progress_bar),
            ("forceProgressBar", &mut a.force_progress_bar),
            ("loopedAction", &mut a.loop_action),
            (
                "allowedWhileDraggingCorpses",
                &mut a.allowed_while_dragging_corpses,
            ),
        ] {
            if let Value::Bool(v) = table.get_str(key) {
                *field = v;
            }
        }
        if let Some(v) = table.get_str("caloriesModifier").as_num() {
            a.calories_modifier = v as f32;
        }
        a.table = Some(table);
        let a = Rc::new(RefCell::new(a));
        Ok(c.push(action_value(&a)))
    });
    let s = expose_class(thread, STACK, Some("java.lang.Object"));
    s.method("size", |c| {
        let n = stack(c)?.borrow().len();
        Ok(c.push(n as i32))
    });
    s.method("isEmpty", |c| {
        let empty = stack(c)?.borrow().is_empty();
        Ok(c.push(empty))
    });
    s.method("get", |c| {
        let s = stack(c)?;
        let i = int_arg(c, 1);
        let a = s
            .borrow()
            .get(i as usize)
            .cloned()
            .ok_or_else(|| LuaError::runtime("IndexOutOfBoundsException"))?;
        Ok(c.push(action_value(&a)))
    });
    s.method("contains", |c| {
        let a = action(&c.arg(1)).ok();
        let s = stack(c)?;
        let found = a.is_some_and(|a| s.borrow().iter().any(|b| Rc::ptr_eq(&a, b)));
        Ok(c.push(found))
    });
    s.method("clear", |c| {
        stack(c)?.borrow_mut().clear();
        Ok(0)
    });
    s.method("add", |c| {
        let a = action(&c.arg(if c.nargs() > 2 { 2 } else { 1 }))?;
        let stack = stack(c)?;
        if c.nargs() > 2 {
            let i = int_arg(c, 1) as usize;
            let mut stack = stack.borrow_mut();
            if i > stack.len() {
                return Err(LuaError::runtime("IndexOutOfBoundsException"));
            }
            stack.insert(i, a);
            Ok(0)
        } else {
            stack.borrow_mut().push(a);
            Ok(c.push(true))
        }
    });
    s.method("push", |c| {
        let a = action(&c.arg(1))?;
        stack(c)?.borrow_mut().push(a.clone());
        Ok(c.push(action_value(&a)))
    });
    for name in ["remove", "removeElement"] {
        s.method(name, move |c| {
            let stack = stack(c)?;
            if name == "remove" && matches!(c.arg(1), Value::Num(_)) {
                let i = int_arg(c, 1) as usize;
                let mut stack = stack.borrow_mut();
                if i >= stack.len() {
                    return Err(LuaError::runtime("IndexOutOfBoundsException"));
                }
                let a = stack.remove(i);
                drop(stack);
                Ok(c.push(action_value(&a)))
            } else {
                let a = action(&c.arg(1)).ok();
                let index = a.and_then(|a| stack.borrow().iter().position(|b| Rc::ptr_eq(&a, b)));
                if let Some(i) = index {
                    stack.borrow_mut().remove(i);
                }
                Ok(c.push(index.is_some()))
            }
        });
    }
    for (name, pop) in [("peek", false), ("pop", true)] {
        s.method(name, move |c| {
            let s = stack(c)?;
            let a = if pop {
                s.borrow_mut().pop()
            } else {
                s.borrow().last().cloned()
            }
            .ok_or_else(|| LuaError::runtime("EmptyStackException"))?;
            Ok(c.push(action_value(&a)))
        });
    }
    let ch = expose_class(thread, ISO_GAME_CHARACTER_CLASS, Some("java.lang.Object"));
    ch.method("getCharacterActions", |c| {
        let chr = chr_of(c)?;
        let value = {
            let mut chr = chr.borrow_mut();
            if chr.character_actions_handle.is_none() {
                chr.character_actions_handle = Some(Value::Object(Rc::new(StackObject(
                    chr.character_actions.clone(),
                ))));
            }
            chr.character_actions_handle.clone().unwrap()
        };
        Ok(c.push(value))
    });
    ch.method("StartAction", |c| {
        let chr = chr_of(c)?;
        let a = action(&c.arg(1))?;
        let stack = chr.borrow().character_actions.clone();
        {
            let mut stack = stack.borrow_mut();
            stack.clear();
            stack.push(a.clone());
        }
        sync_flags(&chr);
        if valid(c.thread, &a) {
            wait_to_start(c.thread, &a);
        }
        Ok(0)
    });
    ch.method("QueueAction", |_| Ok(0));
    ch.method("hasTimedActions", |c| {
        let chr = chr_of(c)?;
        sync_flags(&chr);
        let value = chr.borrow().vars.has_timed_actions;
        Ok(c.push(value))
    });
    ch.method("getTimedActionToRetrigger", |c| {
        let a = chr_of(c)?.borrow().timed_action_to_retrigger.clone();
        Ok(c.push(a.as_ref().map(action_value).unwrap_or_default()))
    });
    ch.method("setTimedActionToRetrigger", |c| {
        let a = if c.arg(1).is_nil() {
            None
        } else {
            Some(action(&c.arg(1))?)
        };
        chr_of(c)?.borrow_mut().timed_action_to_retrigger = a;
        Ok(0)
    });
    for (name, reason) in [
        ("StopAllActionQueue", ""),
        ("StopAllActionQueueRunning", "Running"),
        ("StopAllActionQueueWalking", "Walking"),
        ("StopAllActionQueueAiming", "Aiming"),
    ] {
        ch.method(name, move |c| {
            let chr = chr_of(c)?;
            cancel(c.thread, &chr, reason);
            Ok(0)
        });
    }
    let player = expose_class(
        thread,
        crate::bind_character_sim::ISO_PLAYER_CLASS,
        Some(ISO_GAME_CHARACTER_CLASS),
    );
    player.method("isFarming", |c| {
        let value = chr_of(c)?.borrow().player.is_farming;
        Ok(c.push(value))
    });
    player.method("setIsFarming", |c| {
        chr_of(c)?.borrow_mut().player.is_farming = bool_arg(c, 1);
        Ok(0)
    });
}
