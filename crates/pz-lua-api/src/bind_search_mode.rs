//! `zombie.iso.SearchMode` as Lua and the engine see it: the
//! `getInstance()` singleton ([`instance`], [`reset`], [`update`]), the
//! `LuaManager.GlobalObject.getSearchMode` global, the three exposed
//! classes (`SearchMode`, `SearchMode$PlayerSearchMode`,
//! `SearchMode$SearchModeFloat`) with their public methods, and what
//! `WeatherShader.startMainThread` gives the `screen` shader per player
//! ([`apply_screen_inputs`]).
//!
//! The state is pz-characters' [`SearchMode`]; `isPlayerExterior` reads
//! [`crate::bind_players::player`]'s environment.

use crate::bind_characters::shared_characters;
use crate::bind_players::{self, MAX_PLAYERS};
use crate::expose::{bool_arg, expose_class, global, int_arg, num_arg, with_this};
use pz_characters::character::player::PlayerRenderInputs;
use pz_characters::search_mode::{
    is_player_exterior, PlayerSearchMode, SearchMode, SearchModeParam, SearchModeShaderInputs,
};
use pz_kahlua::value::LuaObject;
use pz_kahlua::{LuaError, LuaResult, NativeCall, Thread, Value};
use pz_render::{ScreenParams, SearchModeShader};
use pz_scripts::ScriptResult;
use std::any::Any;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

pub const SEARCH_MODE_CLASS: &str = "zombie.iso.SearchMode";
pub const PLAYER_SEARCH_MODE_CLASS: &str = "zombie.iso.SearchMode$PlayerSearchMode";
pub const SEARCH_MODE_FLOAT_CLASS: &str = "zombie.iso.SearchMode$SearchModeFloat";

/// The shared `SearchMode`.
pub type SearchModeRef = Rc<RefCell<SearchMode>>;

thread_local! {
    /// `SearchMode.instance`.
    static INSTANCE: RefCell<Option<SearchModeRef>> = const { RefCell::new(None) };
    /// One Lua object per Java object, so Lua `==` is identity.
    static HANDLES: RefCell<HashMap<(usize, usize, usize), Value>> = RefCell::new(HashMap::new());
}

/// `SearchMode.getInstance()`: makes the instance on first use.
pub fn instance() -> SearchModeRef {
    INSTANCE.with(|s| s.borrow_mut().get_or_insert_with(Default::default).clone())
}

/// `SearchMode.reset()` (`IngameState.exit`): the next
/// [`instance`] is a new one. Lua objects of the old one keep it.
pub fn reset() {
    INSTANCE.with(|s| *s.borrow_mut() = None);
}

/// `SearchMode.getInstance().update()` (`IngameState.UpdateStuff`), with
/// `GameTime.getTimeDelta()`.
pub fn update(time_delta: f32) {
    instance().borrow_mut().update(time_delta);
}

/// `isPlayerExterior()` of player slot `i`.
fn player_exterior(i: usize) -> bool {
    bind_players::player(i).is_some_and(|p| is_player_exterior(Some(&*p.borrow())))
}

/// What `WeatherShader.startMainThread` reads of player `i`'s search mode
/// (`getSearchModeForPlayer(i)`'s `isShaderEnabled` and `getShader*`);
/// `None` for `i >= 4`.
pub fn shader_inputs(i: usize) -> Option<SearchModeShaderInputs> {
    let exterior = player_exterior(i);
    instance().borrow().player(i).map(|m| m.shader(exterior))
}

/// [`SearchModeShaderInputs`] as pz-render's [`SearchModeShader`].
pub fn search_mode_shader(s: &SearchModeShaderInputs) -> SearchModeShader {
    SearchModeShader {
        enabled: s.enabled,
        blur: s.blur,
        radius: s.radius,
        gradient_width: s.gradient_width,
        desat: s.desat,
        darkness: s.darkness,
    }
}

/// The player part of `PlayerRenderSettings.updateRenderSettings(i,
/// IsoPlayer.players[i])` on `inputs` (one per player slot, kept across
/// frames).
pub fn update_render_inputs(i: usize, inputs: &mut PlayerRenderInputs) -> ScriptResult<()> {
    let chars = shared_characters();
    let chars = chars.borrow();
    match bind_players::player(i) {
        Some(p) => inputs.update(&chars, Some(&*p.borrow())),
        None => inputs.update::<crate::bind_characters::LuaItem>(&chars, None),
    }
}

/// `params` with the characters' inputs of player `i`'s `WeatherShader`
/// pass: `NightVisionGoggles`, `DrunkFactor`, `BlurFactor` from `inputs`
/// and the search-mode group from [`shader_inputs`].
pub fn apply_screen_inputs(params: ScreenParams, i: usize, inputs: &PlayerRenderInputs) -> ScreenParams {
    ScreenParams {
        night_vision_goggles: inputs.night_vision_goggles,
        drunk_factor: inputs.drunk_factor,
        blur_factor: inputs.blur_factor,
        search: shader_inputs(i).map(|s| search_mode_shader(&s)).unwrap_or_default(),
        ..params
    }
}

/// Where a `PlayerSearchMode` lives: a slot of a `SearchMode`, or one Lua
/// made with `new PlayerSearchMode(plrIndex, parent)` (its `parent` is
/// only read by the private `update`, so it is not kept).
#[derive(Clone)]
enum PlayerSrc {
    Slot(SearchModeRef, usize),
    Own(Rc<RefCell<PlayerSearchMode>>),
}

impl PlayerSrc {
    fn with<R>(&self, f: impl FnOnce(&mut PlayerSearchMode) -> R) -> R {
        match self {
            PlayerSrc::Slot(s, i) => f(s.borrow_mut().player_mut(*i).expect("slot index checked")),
            PlayerSrc::Own(m) => f(&mut m.borrow_mut()),
        }
    }
    fn key(&self) -> (usize, usize) {
        match self {
            PlayerSrc::Slot(s, i) => (Rc::as_ptr(s) as *const () as usize, *i),
            PlayerSrc::Own(m) => (Rc::as_ptr(m) as *const () as usize, usize::MAX),
        }
    }
}

struct SearchModeObject(SearchModeRef);
struct PlayerSearchModeObject(PlayerSrc);
struct SearchModeFloatObject(PlayerSrc, SearchModeParam);

impl LuaObject for SearchModeObject {
    fn class_name(&self) -> &str {
        SEARCH_MODE_CLASS
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

impl LuaObject for PlayerSearchModeObject {
    fn class_name(&self) -> &str {
        PLAYER_SEARCH_MODE_CLASS
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

impl LuaObject for SearchModeFloatObject {
    fn class_name(&self) -> &str {
        SEARCH_MODE_FLOAT_CLASS
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

fn canonical(key: (usize, usize, usize), make: impl FnOnce() -> Value) -> Value {
    HANDLES.with(|h| h.borrow_mut().entry(key).or_insert_with(make).clone())
}

fn search_mode_value(s: &SearchModeRef) -> Value {
    canonical((Rc::as_ptr(s) as *const () as usize, usize::MAX, usize::MAX), || {
        Value::Object(Rc::new(SearchModeObject(s.clone())))
    })
}

fn player_value(src: PlayerSrc) -> Value {
    let (a, b) = src.key();
    canonical((a, b, usize::MAX), || Value::Object(Rc::new(PlayerSearchModeObject(src))))
}

fn float_value_of(src: PlayerSrc, p: SearchModeParam) -> Value {
    let (a, b) = src.key();
    let idx = SearchModeParam::ALL.iter().position(|q| *q == p).unwrap_or(0);
    canonical((a, b, idx), || Value::Object(Rc::new(SearchModeFloatObject(src, p))))
}

fn float(f: f32) -> Value {
    Value::Num(f64::from(f))
}

fn float_arg(c: &NativeCall<'_>, i: usize) -> f32 {
    num_arg(c, i) as f32
}

fn out_of_bounds(i: i32, len: usize) -> LuaError {
    LuaError::Runtime {
        message: format!("Index {i} out of bounds for length {len}"),
        class: "java.lang.ArrayIndexOutOfBoundsException",
    }
}

/// The `plrModes[i]` index of argument `arg`, or the original's
/// `ArrayIndexOutOfBoundsException`.
fn slot_arg(c: &NativeCall<'_>, arg: usize) -> LuaResult<usize> {
    let i = int_arg(c, arg);
    usize::try_from(i).ok().filter(|&i| i < MAX_PLAYERS).ok_or_else(|| out_of_bounds(i, MAX_PLAYERS))
}

fn this_mode(c: &NativeCall<'_>) -> LuaResult<SearchModeRef> {
    with_this::<SearchModeObject, _>(c, |o| o.0.clone())
}

fn this_player(c: &NativeCall<'_>) -> LuaResult<PlayerSrc> {
    with_this::<PlayerSearchModeObject, _>(c, |o| o.0.clone())
}

fn this_float(c: &NativeCall<'_>) -> LuaResult<(PlayerSrc, SearchModeParam)> {
    with_this::<SearchModeFloatObject, _>(c, |o| (o.0.clone(), o.1))
}

const PARAM_GETTERS: [(&str, SearchModeParam); 5] = [
    ("getRadius", SearchModeParam::Radius),
    ("getGradientWidth", SearchModeParam::GradientWidth),
    ("getBlur", SearchModeParam::Blur),
    ("getDesat", SearchModeParam::Desat),
    ("getDarkness", SearchModeParam::Darkness),
];

const SHADER_GETTERS: [(&str, SearchModeParam); 5] = [
    ("getShaderBlur", SearchModeParam::Blur),
    ("getShaderDesat", SearchModeParam::Desat),
    ("getShaderRadius", SearchModeParam::Radius),
    ("getShaderGradientWidth", SearchModeParam::GradientWidth),
    ("getShaderDarkness", SearchModeParam::Darkness),
];

/// The global, the three classes and their public methods.
pub fn register(thread: &mut Thread) {
    global(thread, "getSearchMode", |c| Ok(c.push(search_mode_value(&instance()))));
    register_search_mode(thread);
    register_player_search_mode(thread);
    register_search_mode_float(thread);
}

fn register_search_mode(thread: &mut Thread) {
    let b = expose_class(thread, SEARCH_MODE_CLASS, None);
    b.static_fn("getInstance", |c| Ok(c.push(search_mode_value(&instance()))));
    b.static_fn("reset", |_| {
        reset();
        Ok(0)
    });
    b.method("getSearchModeForPlayer", |c| {
        let s = this_mode(c)?;
        let i = slot_arg(c, 1)?;
        Ok(c.push(player_value(PlayerSrc::Slot(s, i))))
    });
    b.method("getFadeTime", |c| {
        let v = this_mode(c)?.borrow().fade_time();
        Ok(c.push(float(v)))
    });
    b.method("setFadeTime", |c| {
        let v = float_arg(c, 1);
        this_mode(c)?.borrow_mut().set_fade_time(v);
        Ok(0)
    });
    b.method("isOverride", |c| {
        let (s, i) = (this_mode(c)?, slot_arg(c, 1)?);
        let v = s.borrow().player(i).is_some_and(|m| m.override_);
        Ok(c.push(v))
    });
    b.method("setOverride", |c| {
        let (s, i, v) = (this_mode(c)?, slot_arg(c, 1)?, bool_arg(c, 2));
        if let Some(m) = s.borrow_mut().player_mut(i) {
            m.override_ = v;
        }
        Ok(0)
    });
    b.method("isOverrideSearchManager", |c| {
        let (s, i) = (this_mode(c)?, slot_arg(c, 1)?);
        let v = s.borrow().player(i).is_some_and(|m| m.override_search_manager);
        Ok(c.push(v))
    });
    b.method("setOverrideSearchManager", |c| {
        let (s, i, v) = (this_mode(c)?, slot_arg(c, 1)?, bool_arg(c, 2));
        if let Some(m) = s.borrow_mut().player_mut(i) {
            m.override_search_manager = v;
        }
        Ok(0)
    });
    for (name, p) in PARAM_GETTERS {
        b.method(name, move |c| {
            let (s, i) = (this_mode(c)?, slot_arg(c, 1)?);
            Ok(c.push(float_value_of(PlayerSrc::Slot(s, i), p)))
        });
    }
    b.method("isEnabled", |c| {
        let (s, i) = (this_mode(c)?, slot_arg(c, 1)?);
        let v = s.borrow().is_enabled(i).unwrap_or(false);
        Ok(c.push(v))
    });
    b.method("setEnabled", |c| {
        let (s, i, v) = (this_mode(c)?, slot_arg(c, 1)?, bool_arg(c, 2));
        s.borrow_mut().set_enabled(i, v);
        Ok(0)
    });
    b.method("update", |c| {
        let s = this_mode(c)?;
        let dt = crate::bind_climate::game_time_delta();
        s.borrow_mut().update(dt);
        Ok(0)
    });
}

fn register_player_search_mode(thread: &mut Thread) {
    let b = expose_class(thread, PLAYER_SEARCH_MODE_CLASS, None);
    b.ctor(|c| {
        let i = int_arg(c, 0);
        Ok(c.push(player_value(PlayerSrc::Own(Rc::new(RefCell::new(PlayerSearchMode::new(i)))))))
    });
    b.method("isShaderEnabled", |c| {
        let v = this_player(c)?.with(|m| m.is_shader_enabled());
        Ok(c.push(v))
    });
    for (name, p) in SHADER_GETTERS {
        b.method(name, move |c| {
            let src = this_player(c)?;
            // isPlayerExterior: IsoPlayer.players[plrIndex].
            let i = src.with(|m| m.plr_index());
            let slot = usize::try_from(i).ok().filter(|&i| i < MAX_PLAYERS).ok_or_else(|| out_of_bounds(i, MAX_PLAYERS))?;
            let exterior = player_exterior(slot);
            let v = src.with(|m| m.shader_value(p, exterior));
            Ok(c.push(float(v)))
        });
    }
    for (name, p) in PARAM_GETTERS {
        b.method(name, move |c| {
            let src = this_player(c)?;
            Ok(c.push(float_value_of(src, p)))
        });
    }
}

fn register_search_mode_float(thread: &mut Thread) {
    let b = expose_class(thread, SEARCH_MODE_FLOAT_CLASS, None);
    fn get(b: &crate::expose::ClassBuilder, name: &str, f: fn(&pz_characters::search_mode::SearchModeFloat) -> f32) {
        b.method(name, move |c| {
            let (src, p) = this_float(c)?;
            let v = src.with(|m| f(m.param(p)));
            Ok(c.push(float(v)))
        });
    }
    fn set(b: &crate::expose::ClassBuilder, name: &str, f: fn(&mut pz_characters::search_mode::SearchModeFloat, f32)) {
        b.method(name, move |c| {
            let (src, p) = this_float(c)?;
            let v = float_arg(c, 1);
            src.with(|m| f(m.param_mut(p), v));
            Ok(0)
        });
    }
    b.method("set", |c| {
        let (src, p) = this_float(c)?;
        let a = [float_arg(c, 1), float_arg(c, 2), float_arg(c, 3), float_arg(c, 4)];
        src.with(|m| m.param_mut(p).set(a[0], a[1], a[2], a[3]));
        Ok(0)
    });
    set(&b, "setAll", |f, v| f.set_all(v));
    b.method("setTargets", |c| {
        let (src, p) = this_float(c)?;
        let (e, i) = (float_arg(c, 1), float_arg(c, 2));
        src.with(|m| m.param_mut(p).set_targets(e, i));
        Ok(0)
    });
    get(&b, "getExterior", |f| f.exterior());
    set(&b, "setExterior", |f, v| f.set_exterior(v));
    get(&b, "getTargetExterior", |f| f.target_exterior());
    set(&b, "setTargetExterior", |f, v| f.set_target_exterior(v));
    get(&b, "getInterior", |f| f.interior());
    set(&b, "setInterior", |f, v| f.set_interior(v));
    get(&b, "getTargetInterior", |f| f.target_interior());
    set(&b, "setTargetInterior", |f, v| f.set_target_interior(v));
    set(&b, "update", |f, v| f.update(v));
    for (name, op) in [
        ("equalise", (|f| f.equalise()) as fn(&mut pz_characters::search_mode::SearchModeFloat)),
        ("reset", |f| f.reset()),
        ("resetAll", |f| f.reset_all()),
    ] {
        b.method(name, move |c| {
            let (src, p) = this_float(c)?;
            src.with(|m| op(m.param_mut(p)));
            Ok(0)
        });
    }
    get(&b, "getMin", |f| f.min());
    get(&b, "getMax", |f| f.max());
    get(&b, "getStepsize", |f| f.stepsize());
}

#[cfg(test)]
mod tests;
