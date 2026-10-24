//! The character simulation exposed to Lua under the original names:
//! `IsoGameCharacter`'s stat, moodle, trait and perk members (on character
//! objects, whose classes are `IsoPlayer`, `IsoSurvivor`, `IsoZombie` or
//! `IsoAnimal` under `IsoGameCharacter`), `IsoGameCharacter$XP`,
//! `IsoGameCharacter$PerkInfo`, `Stats`, `CharacterStat`, `Moodles`,
//! `Moodle` and `Moodle$MoodleLevel`, over [`pz_characters::character`].
//!
//! Characters are created by the engine and handed to Lua with
//! [`character_value`]. Engine singletons come from the installed
//! [`SimContext`] ([`install_sim_context`], defaults otherwise). The Lua
//! events the simulation fires (`LevelPerk`, `AddXP`, `LogLevelPerk`) are
//! triggered through the event sink ([`install_event_sink`]) once the call
//! that raised them returns, in the order the original raises them; a
//! listener therefore sees the state at the end of the call (the original
//! calls it mid-update). Sounds, halo texts and UI wiggles are dropped.
//! TODO(sounds, HaloTextHelper, MoodlesUI).
//!
//! Not exposed yet: `getCharacterTraits` (the character's `CharacterTraits`
//! is not shared with the Lua object of that class), `getBodyDamage`,
//! `getNutrition`, `Stats.load/save`, `CharacterStat.REGISTRY` and
//! `ORDERED_STATS`, `XP.load/save`. TODO(BodyDamage, Nutrition, saves).

use crate::bind_characters::{
    adopt_survivor_desc_value, desc_arg, shared_characters, survivor_desc_value, trait_def_arg, LuaItem,
};
use crate::bind_java_util::{class_cast, JavaList, JavaMap};
use crate::bind_registry::{java_error, key_arg, key_value, shared_script_manager, MOODLE_TYPE_CLASS};
use crate::bind_skills::{perk_arg, perk_value};
use crate::expose::{bool_arg, expose_class, expose_enum, int_arg, num_arg, str_arg, with_this};
use pz_characters::character::{CharacterStatRef, CharacterStats};
use pz_characters::character::{
    ChevronColor, DefaultSimContext, GameCharacter, MoodleLevel, Moodles, SimContext, SimEvent, Stats,
};
use pz_characters::{CharacterKind, Characters, Color, PerkRef};
use pz_kahlua::value::LuaObject;
use pz_kahlua::{LuaError, LuaResult, NativeCall, Thread, Value};
use pz_scripts::{ScriptError, ScriptManager};
use pz_ui::ColorObject;
use std::any::Any;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

pub const ISO_GAME_CHARACTER_CLASS: &str = "zombie.characters.IsoGameCharacter";
pub const ISO_PLAYER_CLASS: &str = "zombie.characters.IsoPlayer";
pub const ISO_SURVIVOR_CLASS: &str = "zombie.characters.IsoSurvivor";
pub const ISO_ZOMBIE_CLASS: &str = "zombie.characters.IsoZombie";
pub const ISO_ANIMAL_CLASS: &str = "zombie.characters.animals.IsoAnimal";
pub const XP_CLASS: &str = "zombie.characters.IsoGameCharacter$XP";
pub const PERK_INFO_CLASS: &str = "zombie.characters.IsoGameCharacter$PerkInfo";
pub const XP_MULTIPLIER_CLASS: &str = "zombie.characters.IsoGameCharacter$XPMultiplier";
pub const STATS_CLASS: &str = "zombie.characters.Stats";
pub const CHARACTER_STAT_CLASS: &str = "zombie.characters.CharacterStat";
pub const MOODLES_CLASS: &str = "zombie.characters.Moodles.Moodles";
pub const MOODLE_CLASS: &str = "zombie.characters.Moodles.Moodle";
pub const MOODLE_LEVEL_CLASS: &str = "zombie.characters.Moodles.Moodle$MoodleLevel";

/// A character as Lua sees it.
pub type LuaGameCharacter = GameCharacter<LuaItem>;
pub type LuaGameCharacterRef = Rc<RefCell<LuaGameCharacter>>;

type EventSink = Rc<dyn Fn(&mut Thread, &str, &[Value])>;

thread_local! {
    static CONTEXT: RefCell<Rc<dyn SimContext>> = RefCell::new(Rc::new(DefaultSimContext));
    static SINK: RefCell<Option<EventSink>> = const { RefCell::new(None) };
    static HANDLES: RefCell<HashMap<(Handle, usize, usize), Value>> = RefCell::new(HashMap::new());
    static COLORS: RefCell<HashMap<usize, (Rc<Color>, Value)>> = RefCell::new(HashMap::new());
}

/// Makes `ctx` the singletons the bound characters read.
pub fn install_sim_context(ctx: Rc<dyn SimContext>) {
    CONTEXT.with(|c| *c.borrow_mut() = ctx);
}

/// Where the simulation's Lua events go (`LuaEventManager.triggerEvent`).
pub fn install_event_sink(sink: EventSink) {
    SINK.with(|s| *s.borrow_mut() = Some(sink));
}

/// The installed context, collecting events for one call.
struct CallContext {
    base: Rc<dyn SimContext>,
    events: RefCell<Vec<SimEvent>>,
}

impl SimContext for CallContext {
    fn zomboid_globals(&self) -> &pz_characters::character::ZomboidGlobals {
        self.base.zomboid_globals()
    }
    fn is_server(&self) -> bool {
        self.base.is_server()
    }
    fn is_client(&self) -> bool {
        self.base.is_client()
    }
    fn is_debug(&self) -> bool {
        self.base.is_debug()
    }
    fn game_mode(&self) -> &str {
        self.base.game_mode()
    }
    fn multiplier(&self) -> f32 {
        self.base.multiplier()
    }
    fn minutes_per_day(&self) -> f32 {
        self.base.minutes_per_day()
    }
    fn delta_minutes_per_day(&self) -> f32 {
        self.base.delta_minutes_per_day()
    }
    fn thirty_fps_multiplier(&self) -> f32 {
        self.base.thirty_fps_multiplier()
    }
    fn world_age_hours(&self) -> f64 {
        self.base.world_age_hours()
    }
    fn all_players_asleep(&self) -> bool {
        self.base.all_players_asleep()
    }
    fn stats_decrease_multiplier(&self) -> f64 {
        self.base.stats_decrease_multiplier()
    }
    fn endurance_regen_multiplier(&self) -> f64 {
        self.base.endurance_regen_multiplier()
    }
    fn decaying_corpse_health_impact(&self) -> i32 {
        self.base.decaying_corpse_health_impact()
    }
    fn xp_multiplier_global_toggle(&self) -> bool {
        self.base.xp_multiplier_global_toggle()
    }
    fn xp_multiplier_global(&self) -> f64 {
        self.base.xp_multiplier_global()
    }
    fn xp_multiplier_for(&self, perk_id: &str) -> f32 {
        self.base.xp_multiplier_for(perk_id)
    }
    fn server_sleep_allowed_and_needed(&self) -> bool {
        self.base.server_sleep_allowed_and_needed()
    }
    fn trigger_hook(&self, name: &str) -> bool {
        self.base.trigger_hook(name)
    }
    fn get_text(&self, key: &str) -> String {
        self.base.get_text(key)
    }
    fn event(&self, e: SimEvent) {
        self.events.borrow_mut().push(e);
    }
}

/// Runs `f` with the installed context, then fires the Lua events it
/// raised with `chr` as the character.
fn with_ctx<R>(
    c: &mut NativeCall<'_>,
    chr: &Value,
    f: impl FnOnce(&dyn SimContext) -> Result<R, ScriptError>,
) -> LuaResult<R> {
    let ctx = CallContext { base: CONTEXT.with(|c| c.borrow().clone()), events: RefCell::default() };
    let r = f(&ctx);
    let events = ctx.events.into_inner();
    if let Some(sink) = SINK.with(|s| s.borrow().clone()) {
        for e in events {
            let (name, args) = match e {
                SimEvent::LevelPerk { perk, level, increased } => (
                    "LevelPerk",
                    vec![chr.clone(), perk_value(Some(&perk)), Value::from(level), Value::Bool(increased)],
                ),
                SimEvent::LogLevelPerk { perk, level } => {
                    ("LogLevelPerk", vec![chr.clone(), perk_value(Some(&perk)), Value::from(level), Value::Bool(false)])
                }
                SimEvent::AddXp { perk, amount } => {
                    ("AddXP", vec![chr.clone(), perk_value(Some(&perk)), Value::Num(f64::from(amount))])
                }
                _ => continue,
            };
            sink(c.thread, name, &args);
        }
    }
    r.map_err(java_error)
}

// ---- objects ------------------------------------------------------------

/// A character (`IsoGameCharacter` and its subclasses).
pub struct CharacterObject(pub LuaGameCharacterRef);

impl LuaObject for CharacterObject {
    fn class_name(&self) -> &str {
        match self.0.borrow().kind {
            CharacterKind::Player => ISO_PLAYER_CLASS,
            CharacterKind::Survivor => ISO_SURVIVOR_CLASS,
            CharacterKind::Zombie => ISO_ZOMBIE_CLASS,
            CharacterKind::Animal => ISO_ANIMAL_CLASS,
        }
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// What a canonical Lua object stands for.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum Handle {
    Character,
    Stat,
    Stats,
    Moodles,
    Moodle,
    Xp,
    PerkInfo,
}

/// One Lua object per (kind, owner, member), so Lua `==` is identity.
fn canonical(id: (Handle, usize, usize), make: impl FnOnce() -> Value) -> Value {
    HANDLES.with(|h| h.borrow_mut().entry(id).or_insert_with(make).clone())
}

/// The canonical Lua object of `chr`.
pub fn character_value(chr: &LuaGameCharacterRef) -> Value {
    canonical((Handle::Character, Rc::as_ptr(chr) as usize, 0), || Value::Object(Rc::new(CharacterObject(chr.clone()))))
}

/// A character's `Stats`, or a free-standing `new Stats()`.
pub enum StatsObject {
    Of(LuaGameCharacterRef),
    Owned(RefCell<Stats>),
}

impl LuaObject for StatsObject {
    fn class_name(&self) -> &str {
        STATS_CLASS
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

impl StatsObject {
    fn with<R>(&self, f: impl FnOnce(&mut Stats) -> R) -> R {
        match self {
            StatsObject::Of(chr) => f(&mut chr.borrow_mut().stats),
            StatsObject::Owned(s) => f(&mut s.borrow_mut()),
        }
    }
}

/// `CharacterStat`.
pub struct CharacterStatObject(pub CharacterStatRef);

impl LuaObject for CharacterStatObject {
    fn class_name(&self) -> &str {
        CHARACTER_STAT_CLASS
    }
    fn to_lua_string(&self) -> String {
        self.0.to_java_string()
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

fn stat_value(s: Option<&CharacterStatRef>) -> Value {
    match s {
        Some(s) => canonical((Handle::Stat, Rc::as_ptr(s) as usize, 0), || {
            Value::Object(Rc::new(CharacterStatObject(s.clone())))
        }),
        None => Value::Nil,
    }
}

fn npe() -> LuaError {
    LuaError::Runtime { message: String::new(), class: "java.lang.NullPointerException" }
}

fn stat_arg(c: &NativeCall<'_>, i: usize) -> LuaResult<CharacterStatRef> {
    match c.arg(i) {
        Value::Object(o) => match o.as_any().downcast_ref::<CharacterStatObject>() {
            Some(s) => Ok(s.0.clone()),
            None => Err(class_cast(&c.arg(i), CHARACTER_STAT_CLASS)),
        },
        Value::Nil => Err(npe()),
        other => Err(class_cast(&other, CHARACTER_STAT_CLASS)),
    }
}

/// A character's `Moodles`.
pub struct MoodlesObject(pub LuaGameCharacterRef);

/// One `Moodle` of a character.
pub struct MoodleObject {
    pub chr: LuaGameCharacterRef,
    pub moodle_type: Rc<pz_scripts::registry::RegistryKey>,
}

/// A character's `XP`.
pub struct XpObject(pub LuaGameCharacterRef);

/// A `PerkInfo` of a character.
pub struct PerkInfoObject {
    pub chr: LuaGameCharacterRef,
    pub perk: PerkRef,
}

/// An `XPMultiplier` (a snapshot; it has public fields only).
pub struct XpMultiplierObject(pub pz_characters::character::XPMultiplier);

macro_rules! lua_object {
    ($t:ty, $class:expr) => {
        impl LuaObject for $t {
            fn class_name(&self) -> &str {
                $class
            }
            fn as_any(&self) -> &dyn Any {
                self
            }
        }
    };
}

lua_object!(MoodlesObject, MOODLES_CLASS);
lua_object!(MoodleObject, MOODLE_CLASS);
lua_object!(XpObject, XP_CLASS);
lua_object!(PerkInfoObject, PERK_INFO_CLASS);
lua_object!(XpMultiplierObject, XP_MULTIPLIER_CLASS);

fn chr_of(c: &NativeCall<'_>) -> LuaResult<LuaGameCharacterRef> {
    with_this::<CharacterObject, _>(c, |o| o.0.clone())
}

fn float_arg(c: &NativeCall<'_>, i: usize) -> f32 {
    num_arg(c, i) as f32
}

fn float_value(f: f32) -> Value {
    Value::Num(f64::from(f))
}

fn color_value(col: &ChevronColor) -> Value {
    thread_local! {
        static FIXED: [Value; 3] = [ChevronColor::White, ChevronColor::Negative, ChevronColor::Positive].map(|c| {
            let k = c.color();
            Value::Object(Rc::new(ColorObject::new(k.r, k.g, k.b, k.a)))
        });
    }
    match col {
        ChevronColor::White => FIXED.with(|f| f[0].clone()),
        ChevronColor::Negative => FIXED.with(|f| f[1].clone()),
        ChevronColor::Positive => FIXED.with(|f| f[2].clone()),
        ChevronColor::Other(rc) => COLORS
            .with(|m| m.borrow().values().find(|(k, _)| Rc::ptr_eq(k, rc)).map(|(_, v)| v.clone()).unwrap_or_default()),
    }
}

/// A `Color` argument: the three fixed colours by identity, any other
/// colour object as itself (its components when passed).
fn color_arg(c: &NativeCall<'_>, i: usize) -> LuaResult<Option<ChevronColor>> {
    let v = c.arg(i);
    let Value::Object(o) = &v else {
        return if v.is_nil() { Ok(None) } else { Err(class_cast(&v, "zombie.core.Color")) };
    };
    for fixed in [ChevronColor::White, ChevronColor::Negative, ChevronColor::Positive] {
        if color_value(&fixed).raw_equals(&v) {
            return Ok(Some(fixed));
        }
    }
    let Some(col) = o.as_any().downcast_ref::<ColorObject>() else {
        return Err(class_cast(&v, "zombie.core.Color"));
    };
    let pz_render::Rgba(r, g, b, a) = col.rgba.get();
    let id = Rc::as_ptr(o) as *const () as usize;
    let rc = COLORS
        .with(|m| m.borrow_mut().entry(id).or_insert_with(|| (Rc::new(Color { r, g, b, a }), v.clone())).0.clone());
    Ok(Some(ChevronColor::Other(rc)))
}

// ---- registration ---------------------------------------------------------

/// Register the classes, bound to `sm` and `chars`.
pub fn register(thread: &mut Thread, sm: Rc<ScriptManager>, chars: Rc<RefCell<Characters>>) {
    register_character_stat(thread, &chars);
    register_stats(thread, &chars);
    register_moodles(thread, &sm, &chars);
    register_xp(thread, &sm, &chars);
    register_character(thread, &sm, &chars);
}

/// `register` on the shared ScriptManager and Characters.
pub fn register_shared(thread: &mut Thread) {
    register(thread, shared_script_manager(), shared_characters());
}

fn register_character_stat(thread: &mut Thread, chars: &Rc<RefCell<Characters>>) {
    let b = expose_class(thread, CHARACTER_STAT_CLASS, None);
    for (field, stat) in chars.borrow().sim.character_stats.constants() {
        b.field(field, stat_value(Some(stat)));
    }
    let ch = chars.clone();
    b.static_fn("register", move |c| {
        let id = str_arg(c, 0).map(|s| s.to_string()).unwrap_or_default();
        let s = ch.borrow_mut().sim.character_stats.register(&id, float_arg(c, 1), float_arg(c, 2), float_arg(c, 3));
        Ok(c.push(stat_value(Some(&s))))
    });
    let ch = chars.clone();
    b.static_fn("getById", move |c| {
        let id = str_arg(c, 0).map(|s| s.to_string()).unwrap_or_default();
        let s = ch.borrow().sim.character_stats.get_by_id(&id);
        Ok(c.push(stat_value(s.as_ref())))
    });
    let this = |c: &NativeCall<'_>| with_this::<CharacterStatObject, _>(c, |o| o.0.clone());
    b.method("getId", move |c| {
        let s = this(c)?;
        Ok(c.push(s.id()))
    });
    b.method("getMinimumValue", move |c| {
        let s = this(c)?;
        Ok(c.push(float_value(s.minimum_value())))
    });
    b.method("getMaximumValue", move |c| {
        let s = this(c)?;
        Ok(c.push(float_value(s.maximum_value())))
    });
    b.method("getDefaultValue", move |c| {
        let s = this(c)?;
        Ok(c.push(float_value(s.default_value())))
    });
    b.method("clamp", move |c| {
        let s = this(c)?;
        Ok(c.push(float_value(s.clamp(float_arg(c, 1)))))
    });
    b.method("isAtMinimum", move |c| {
        let s = this(c)?;
        Ok(c.push(s.is_at_minimum(float_arg(c, 1))))
    });
    b.method("isAtMaximum", move |c| {
        let s = this(c)?;
        Ok(c.push(s.is_at_maximum(float_arg(c, 1))))
    });
    b.method("toString", move |c| {
        let s = this(c)?;
        Ok(c.push(s.to_java_string()))
    });
}

fn stats_this(c: &NativeCall<'_>) -> LuaResult<Rc<dyn LuaObject>> {
    crate::expose::this::<StatsObject>(c)
}

fn with_stats<R>(c: &NativeCall<'_>, f: impl FnOnce(&mut Stats) -> R) -> LuaResult<R> {
    let o = stats_this(c)?;
    Ok(o.as_any().downcast_ref::<StatsObject>().unwrap().with(f))
}

fn register_stats(thread: &mut Thread, chars: &Rc<RefCell<Characters>>) {
    let b = expose_class(thread, STATS_CLASS, None);
    b.ctor(|c| Ok(c.push(Value::Object(Rc::new(StatsObject::Owned(RefCell::new(Stats::default())))))));
    type StatFn = fn(&mut Stats, &CharacterStatRef) -> Value;
    let by_stat: [(&str, StatFn); 6] = [
        ("get", |s, k| float_value(s.get(k))),
        ("reset", |s, k| Value::Bool(s.reset(k))),
        ("isAtMinimum", |s, k| Value::Bool(s.is_at_minimum(k))),
        ("isAtMaximum", |s, k| Value::Bool(s.is_at_maximum(k))),
        ("isAboveMinimum", |s, k| Value::Bool(s.is_above_minimum(k))),
        ("toString", |_, _| Value::Nil),
    ];
    for (name, f) in by_stat.into_iter().take(5) {
        b.method(name, move |c| {
            let k = stat_arg(c, 1)?;
            let v = with_stats(c, |s| f(s, &k))?;
            Ok(c.push(v))
        });
    }
    type AmountFn = fn(&mut Stats, &CharacterStatRef, f32) -> bool;
    let by_amount: [(&str, AmountFn); 3] =
        [("set", |s, k, v| s.set(k, v)), ("add", |s, k, v| s.add(k, v)), ("remove", |s, k, v| s.remove(k, v))];
    for (name, f) in by_amount {
        b.method(name, move |c| {
            let k = stat_arg(c, 1)?;
            let v = float_arg(c, 2);
            let r = with_stats(c, |s| f(s, &k, v))?;
            Ok(c.push(r))
        });
    }
    let ch = chars.clone();
    b.method("resetStats", move |c| {
        let defs: CharacterStats = ch.borrow().sim.character_stats.clone();
        with_stats(c, |s| s.reset_stats(&defs))?;
        Ok(0)
    });
    let ch = chars.clone();
    b.method("getNicotineStress", move |c| {
        let defs = ch.borrow().sim.character_stats.clone();
        let v = with_stats(c, |s| s.nicotine_stress(&defs))?.map_err(java_error)?;
        Ok(c.push(float_value(v)))
    });
    b.method("toString", |c| {
        let v = with_stats(c, |s| s.to_java_string())?;
        Ok(c.push(v))
    });
    type IntGet = fn(&Stats) -> i32;
    let ints: [(&str, IntGet); 4] = [
        ("getNumVisibleZombies", Stats::num_visible_zombies),
        ("getVisibleZombies", Stats::num_visible_zombies),
        ("getNumChasingZombies", Stats::num_chasing_zombies),
        ("getNumVeryCloseZombies", Stats::num_very_close_zombies),
    ];
    for (name, f) in ints {
        b.method(name, move |c| {
            let v = with_stats(c, |s| f(s))?;
            Ok(c.push(v))
        });
    }
    b.method("setNumVisibleZombies", |c| {
        let n = int_arg(c, 1);
        with_stats(c, |s| s.set_num_visible_zombies(n))?;
        Ok(0)
    });
    b.method("setLastNumberChasingZombies", |c| {
        let n = int_arg(c, 1);
        with_stats(c, |s| s.set_last_number_chasing_zombies(n))?;
        Ok(0)
    });
    type FloatGet = fn(&Stats) -> f32;
    let floats: [(&str, FloatGet); 4] = [
        ("getLastEndurance", Stats::last_endurance),
        ("getTrippingRotAngle", Stats::tripping_rot_angle),
        ("getEnduranceDangerWarning", |_| Stats::ENDURANCE_DANGER_WARNING),
        ("getEnduranceWarning", |_| Stats::ENDURANCE_WARNING),
    ];
    for (name, f) in floats {
        b.method(name, move |c| {
            let v = with_stats(c, |s| f(s))?;
            Ok(c.push(float_value(v)))
        });
    }
    type FloatSet = fn(&mut Stats, f32);
    let setters: [(&str, FloatSet); 3] = [
        ("setLastEndurance", Stats::set_last_endurance),
        ("setTrippingRotAngle", Stats::set_tripping_rot_angle),
        ("addTrippingRotAngle", Stats::add_tripping_rot_angle),
    ];
    for (name, f) in setters {
        b.method(name, move |c| {
            let v = float_arg(c, 1);
            with_stats(c, |s| f(s, v))?;
            Ok(0)
        });
    }
    b.method("isEnduranceRecharging", |c| {
        let v = with_stats(c, |s| s.is_endurance_recharging())?;
        Ok(c.push(v))
    });
    b.method("isTripping", |c| {
        let v = with_stats(c, |s| s.is_tripping())?;
        Ok(c.push(v))
    });
    b.method("setTripping", |c| {
        let v = bool_arg(c, 1);
        with_stats(c, |s| s.set_tripping(v))?;
        Ok(0)
    });
}

fn moodles_chr(c: &NativeCall<'_>) -> LuaResult<LuaGameCharacterRef> {
    with_this::<MoodlesObject, _>(c, |o| o.0.clone())
}

fn moodle_this(c: &NativeCall<'_>) -> LuaResult<(LuaGameCharacterRef, Rc<pz_scripts::registry::RegistryKey>)> {
    with_this::<MoodleObject, _>(c, |o| (o.chr.clone(), o.moodle_type.clone()))
}

/// The canonical Lua object of `chr`'s moodle of type `t` (the original
/// hands `Moodle`s to its UI, not to scripts).
pub fn moodle_value(chr: &LuaGameCharacterRef, t: &Rc<pz_scripts::registry::RegistryKey>) -> Value {
    let id = (Handle::Moodle, Rc::as_ptr(chr) as usize, Rc::as_ptr(t) as usize);
    canonical(id, || Value::Object(Rc::new(MoodleObject { chr: chr.clone(), moodle_type: t.clone() })))
}

fn register_moodles(thread: &mut Thread, sm: &Rc<ScriptManager>, chars: &Rc<RefCell<Characters>>) {
    expose_enum(thread, MOODLE_LEVEL_CLASS, &MoodleLevel::NAMES);
    let b = expose_class(thread, MOODLES_CLASS, None);
    b.field("NeutralMoodleType", Moodles::NEUTRAL_MOODLE_TYPE);
    b.field("GoodMoodleType", Moodles::GOOD_MOODLE_TYPE);
    b.field("BadMoodleType", Moodles::BAD_MOODLE_TYPE);
    let s = sm.clone();
    b.method("getGoodBadNeutral", move |c| {
        let t = key_arg(c, 1)?;
        let chr = moodles_chr(c)?;
        let v = chr.borrow().moodles.good_bad_neutral(&s, t.as_deref());
        Ok(c.push(v))
    });
    b.method("getMoodleLevel", |c| {
        let t = key_arg(c, 1)?;
        let chr = moodles_chr(c)?;
        let v = chr.borrow().moodles.moodle_level(t.as_deref()).map_err(java_error)?;
        Ok(c.push(v))
    });
    b.method("isMaxMoodleLevel", |c| {
        let t = key_arg(c, 1)?;
        let chr = moodles_chr(c)?;
        let v = chr.borrow().moodles.is_max_moodle_level(t.as_deref()).map_err(java_error)?;
        Ok(c.push(v))
    });
    let get_text = || {
        let ctx = CONTEXT.with(|c| c.borrow().clone());
        move |k: &str| ctx.get_text(k)
    };
    b.method("getMoodleDisplayString", move |c| {
        let t = key_arg(c, 1)?;
        let chr = moodles_chr(c)?;
        let v = chr.borrow().moodles.moodle_display_string(t.as_deref(), &get_text()).map_err(java_error)?;
        Ok(c.push(v))
    });
    b.method("getMoodleDescriptionString", move |c| {
        let t = key_arg(c, 1)?;
        let chr = moodles_chr(c)?;
        let v = chr.borrow().moodles.moodle_description_string(t.as_deref(), &get_text()).map_err(java_error)?;
        Ok(c.push(v))
    });
    b.method("UI_RefreshNeeded", |c| {
        let chr = moodles_chr(c)?;
        let v = chr.borrow_mut().moodles.ui_refresh_needed();
        Ok(c.push(v))
    });
    b.method("setMoodlesStateChanged", |c| {
        let v = bool_arg(c, 1);
        moodles_chr(c)?.borrow_mut().moodles.set_moodles_state_changed(v);
        Ok(0)
    });
    let ch = chars.clone();
    b.method("Update", move |c| {
        let chr = moodles_chr(c)?;
        let v = character_value(&chr);
        let chars = ch.borrow();
        with_ctx(c, &v, |ctx| chr.borrow_mut().update_moodles(&chars, ctx))?;
        Ok(0)
    });

    let b = expose_class(thread, MOODLE_CLASS, None);
    b.method("getMoodleType", |c| {
        let (_, t) = moodle_this(c)?;
        Ok(c.push(key_value(MOODLE_TYPE_CLASS, Some(&t))))
    });
    fn with_moodle<R>(c: &NativeCall<'_>, f: impl FnOnce(&mut pz_characters::character::Moodle) -> R) -> LuaResult<R> {
        let (chr, t) = moodle_this(c)?;
        let mut chr = chr.borrow_mut();
        chr.moodles.moodle_mut(&t).map(f).ok_or_else(npe)
    }
    b.method("getLevel", |c| {
        let v = with_moodle(c, |m| m.level())?;
        Ok(c.push(v))
    });
    b.method("getChevronCount", |c| {
        let v = with_moodle(c, |m| m.chevron_count())?;
        Ok(c.push(v))
    });
    b.method("isChevronIsUp", |c| {
        let v = with_moodle(c, |m| m.is_chevron_is_up())?;
        Ok(c.push(v))
    });
    b.method("getChevronColor", |c| {
        let v = with_moodle(c, |m| color_value(m.chevron_color()))?;
        Ok(c.push(v))
    });
    b.method("chevronDifference", |c| {
        let col = color_arg(c, 3)?;
        let (n, up) = (int_arg(c, 1), bool_arg(c, 2));
        // A `null` colour differs from any stored one.
        let v = with_moodle(c, |m| match &col {
            Some(col) => m.chevron_difference(n, up, col),
            None => true,
        })?;
        Ok(c.push(v))
    });
    b.method("setChevron", |c| {
        let col = color_arg(c, 3)?;
        let (n, up) = (int_arg(c, 1), bool_arg(c, 2));
        with_moodle(c, |m| m.set_chevron(n, up, col))?;
        Ok(0)
    });
    let ch = chars.clone();
    b.method("Update", move |c| {
        let (chr, t) = moodle_this(c)?;
        let v = character_value(&chr);
        let chars = ch.borrow();
        let changed = with_ctx(c, &v, |ctx| {
            let mut g = chr.borrow_mut();
            let Some(i) = g.moodles.all().iter().position(|m| m.moodle_type().location == t.location) else {
                return Err(ScriptError::new("java.lang.NullPointerException"));
            };
            let mut m = g.moodles.all()[i].clone();
            let r = m.update(&mut g, &chars, ctx);
            *g.moodles.moodle_mut(&t).expect("present") = m;
            r
        })?;
        Ok(c.push(changed))
    });
}

fn xp_chr(c: &NativeCall<'_>) -> LuaResult<LuaGameCharacterRef> {
    with_this::<XpObject, _>(c, |o| o.0.clone())
}

fn register_xp(thread: &mut Thread, sm: &Rc<ScriptManager>, chars: &Rc<RefCell<Characters>>) {
    expose_class(thread, XP_MULTIPLIER_CLASS, None);
    let b = expose_class(thread, XP_CLASS, None);
    let (s, ch) = (sm.clone(), chars.clone());
    b.method("AddXP", move |c| {
        let chr = xp_chr(c)?;
        let v = character_value(&chr);
        let perk = perk_arg(c, 1)?;
        let amount = float_arg(c, 2);
        let n = c.nargs() - 1;
        let flags: Vec<bool> = (3..=n).map(|i| bool_arg(c, i)).collect();
        let chars = ch.borrow();
        with_ctx(c, &v, |ctx| {
            let mut g = chr.borrow_mut();
            let p = perk.as_ref();
            match n {
                // AddXP(perk, amount)
                2 => g.add_xp(&s, &chars, p, amount, ctx),
                // AddXP(perk, amount, noMultiplier[, haloText])
                3 => g.add_xp_no_multiplier_flag(&s, &chars, p, amount, flags[0], false, ctx),
                4 => g.add_xp_no_multiplier_flag(&s, &chars, p, amount, flags[0], flags[1], ctx),
                // AddXP(perk, amount, callLua, doXPBoost, remote[, haloText])
                5 => g.add_xp_full(&s, &chars, p, amount, flags[0], flags[1], flags[2], false, ctx),
                _ => g.add_xp_full(&s, &chars, p, amount, flags[0], flags[1], flags[2], flags[3], ctx),
            }
        })?;
        Ok(0)
    });
    let (s, ch) = (sm.clone(), chars.clone());
    b.method("AddXPHaloText", move |c| {
        let chr = xp_chr(c)?;
        let v = character_value(&chr);
        let perk = perk_arg(c, 1)?;
        let amount = float_arg(c, 2);
        let chars = ch.borrow();
        with_ctx(c, &v, |ctx| chr.borrow_mut().add_xp_halo_text(&s, &chars, perk.as_ref(), amount, ctx))?;
        Ok(0)
    });
    let (s, ch) = (sm.clone(), chars.clone());
    b.method("AddXPNoMultiplier", move |c| {
        let chr = xp_chr(c)?;
        let v = character_value(&chr);
        let perk = perk_arg(c, 1)?;
        let amount = float_arg(c, 2);
        let chars = ch.borrow();
        with_ctx(c, &v, |ctx| chr.borrow_mut().add_xp_no_multiplier(&s, &chars, perk.as_ref(), amount, ctx))?;
        Ok(0)
    });
    b.method("addXpMultiplier", |c| {
        let chr = xp_chr(c)?;
        let perk = perk_arg(c, 1)?.ok_or_else(npe)?;
        chr.borrow_mut().xp.add_xp_multiplier(&perk, float_arg(c, 2), int_arg(c, 3), int_arg(c, 4));
        Ok(0)
    });
    b.method("getMultiplier", |c| {
        let chr = xp_chr(c)?;
        let v = perk_arg(c, 1)?.map_or(0.0, |p| chr.borrow().xp.multiplier(&p));
        Ok(c.push(float_value(v)))
    });
    b.method("getMultiplierMap", |c| {
        let chr = xp_chr(c)?;
        let read = move || {
            chr.borrow()
                .xp
                .xp_map_multiplier
                .iter()
                .map(|(p, m)| (perk_value(Some(p)), Value::Object(Rc::new(XpMultiplierObject(*m)))))
                .collect()
        };
        Ok(c.push(Value::Object(Rc::new(JavaMap::live(read, None)))))
    });
    b.method("getPerkBoost", |c| {
        let chr = xp_chr(c)?;
        let perk = perk_arg(c, 1)?;
        let v = match perk {
            Some(p) => chr.borrow().xp_perk_boost(&p).map_err(java_error)?,
            // `getXPBoostMap().get(null)`: no entry.
            None => 0,
        };
        Ok(c.push(v))
    });
    let ch = chars.clone();
    b.method("setPerkBoost", move |c| {
        let chr = xp_chr(c)?;
        let perk = perk_arg(c, 1)?;
        let chars = ch.borrow();
        chr.borrow_mut().xp_set_perk_boost(&chars, perk.as_ref(), int_arg(c, 2)).map_err(java_error)?;
        Ok(0)
    });
    b.method("getLevel", |c| {
        let v = xp_chr(c)?.borrow().xp.level();
        Ok(c.push(v))
    });
    b.method("setLevel", |c| {
        let l = int_arg(c, 1);
        xp_chr(c)?.borrow_mut().xp.set_level(l);
        Ok(0)
    });
    b.method("getTotalXp", |c| {
        let v = xp_chr(c)?.borrow().xp.total_xp();
        Ok(c.push(float_value(v)))
    });
    b.method("setTotalXP", |c| {
        let v = float_arg(c, 1);
        xp_chr(c)?.borrow_mut().xp.set_total_xp(v);
        Ok(0)
    });
    b.method("getXP", |c| {
        let chr = xp_chr(c)?;
        let v = perk_arg(c, 1)?.map_or(0.0, |p| chr.borrow().xp.xp(&p));
        Ok(c.push(float_value(v)))
    });
    let ch = chars.clone();
    b.method("setXPToLevel", move |c| {
        let chr = xp_chr(c)?;
        let v = character_value(&chr);
        let Some(perk) = perk_arg(c, 1)? else { return Ok(0) };
        let level = int_arg(c, 2);
        let chars = ch.borrow();
        with_ctx(c, &v, |ctx| chr.borrow_mut().xp_set_xp_to_level(&chars, &perk, level, ctx))?;
        Ok(0)
    });

    let b = expose_class(thread, PERK_INFO_CLASS, None);
    b.method("getLevel", |c| {
        let (chr, perk) = with_this::<PerkInfoObject, _>(c, |o| (o.chr.clone(), o.perk.clone()))?;
        let v = chr.borrow().perk_level(Some(&perk));
        Ok(c.push(v))
    });
}

fn perk_info_value(chr: &LuaGameCharacterRef, perk: &PerkRef) -> Value {
    let id = (Handle::PerkInfo, Rc::as_ptr(chr) as usize, Rc::as_ptr(perk) as usize);
    canonical(id, || Value::Object(Rc::new(PerkInfoObject { chr: chr.clone(), perk: perk.clone() })))
}

fn register_character(thread: &mut Thread, sm: &Rc<ScriptManager>, chars: &Rc<RefCell<Characters>>) {
    let b = expose_class(thread, ISO_GAME_CHARACTER_CLASS, None);
    for class in [ISO_PLAYER_CLASS, ISO_SURVIVOR_CLASS, ISO_ZOMBIE_CLASS, ISO_ANIMAL_CLASS] {
        expose_class(thread, class, Some(ISO_GAME_CHARACTER_CLASS));
    }
    b.method("getStats", |c| {
        let chr = chr_of(c)?;
        let v = canonical((Handle::Stats, Rc::as_ptr(&chr) as usize, 0), || {
            Value::Object(Rc::new(StatsObject::Of(chr.clone())))
        });
        Ok(c.push(v))
    });
    b.method("getMoodles", |c| {
        let chr = chr_of(c)?;
        let v = canonical((Handle::Moodles, Rc::as_ptr(&chr) as usize, 0), || {
            Value::Object(Rc::new(MoodlesObject(chr.clone())))
        });
        Ok(c.push(v))
    });
    b.method("getXp", |c| {
        let chr = chr_of(c)?;
        let v = canonical((Handle::Xp, Rc::as_ptr(&chr) as usize, 0), || Value::Object(Rc::new(XpObject(chr.clone()))));
        Ok(c.push(v))
    });
    b.method("getPerkLevel", |c| {
        let perk = perk_arg(c, 1)?;
        let v = chr_of(c)?.borrow().perk_level(perk.as_ref());
        Ok(c.push(v))
    });
    b.method("setPerkLevelDebug", |c| {
        let chr = chr_of(c)?;
        let v = character_value(&chr);
        let perk = perk_arg(c, 1)?;
        let level = int_arg(c, 2);
        with_ctx(c, &v, |ctx| {
            // `null` is stored as a PerkInfo with no perk in the original;
            // nothing can read it back.
            if let Some(p) = &perk {
                chr.borrow_mut().set_perk_level_debug(p, level, ctx);
            }
            Ok(())
        })?;
        Ok(0)
    });
    let ch = chars.clone();
    b.method("LevelPerk", move |c| {
        let chr = chr_of(c)?;
        let v = character_value(&chr);
        let perk = perk_arg(c, 1)?;
        let chars = ch.borrow();
        with_ctx(c, &v, |ctx| chr.borrow_mut().level_perk(&chars, perk.as_ref(), ctx))?;
        Ok(0)
    });
    let ch = chars.clone();
    b.method("LoseLevel", move |c| {
        let chr = chr_of(c)?;
        let v = character_value(&chr);
        let perk = perk_arg(c, 1)?;
        let chars = ch.borrow();
        with_ctx(c, &v, |ctx| {
            // `null` finds no PerkInfo: the level-0 event with a nil perk is
            // not raised here.
            if let Some(p) = &perk {
                chr.borrow_mut().lose_level(&chars, p, ctx);
            }
            Ok(())
        })?;
        Ok(0)
    });
    b.method("level0", |c| {
        let perk = perk_arg(c, 1)?;
        if let Some(p) = perk {
            chr_of(c)?.borrow_mut().level0(&p);
        }
        Ok(0)
    });
    b.method("getPerkInfo", |c| {
        let chr = chr_of(c)?;
        let perk = perk_arg(c, 1)?;
        let found = chr.borrow().perk_info(perk.as_ref()).map(|i| i.perk.clone());
        Ok(c.push(found.map_or(Value::Nil, |p| perk_info_value(&chr, &p))))
    });
    b.method("getPerkList", |c| {
        let chr = chr_of(c)?;
        let read = move || chr.borrow().perk_list.iter().map(|i| perk_info_value(&chr, &i.perk)).collect();
        Ok(c.push(Value::Object(Rc::new(JavaList::live(read, None)))))
    });
    b.method("getPerkToUnit", |c| {
        let perk = perk_arg(c, 1)?.ok_or_else(npe)?;
        let v = chr_of(c)?.borrow().perk_to_unit(&perk);
        Ok(c.push(float_value(v)))
    });
    let ch = chars.clone();
    b.method("getXpForLevel", move |c| {
        let v = chr_of(c)?.borrow().xp_for_level(&ch.borrow(), int_arg(c, 1)).map_err(java_error)?;
        Ok(c.push(v))
    });
    let ch = chars.clone();
    b.method("getLevelUpLevels", move |c| {
        let v = chr_of(c)?.borrow().level_up_levels(&ch.borrow(), int_arg(c, 1)).map_err(java_error)?;
        Ok(c.push(v))
    });
    let ch = chars.clone();
    b.method("getLevelMaxForXp", move |c| {
        let v = chr_of(c)?.borrow().level_max_for_xp(&ch.borrow());
        Ok(c.push(v))
    });
    b.method("hasTrait", |c| {
        let chr = chr_of(c)?;
        // `hasTrait(CharacterTrait...)` with several arguments: any of them.
        let mut any = false;
        for i in 1..c.nargs().max(2) {
            let t = key_arg(c, i)?;
            if chr.borrow().character_traits.get(t.as_ref()).map_err(java_error)? {
                any = true;
                break;
            }
        }
        Ok(c.push(any))
    });
    b.method("isUnlimitedEndurance", |c| {
        let ctx = CONTEXT.with(|c| c.borrow().clone());
        let v = chr_of(c)?.borrow().is_unlimited_endurance(&*ctx);
        Ok(c.push(v))
    });
    b.method("isGodMod", |c| {
        let ctx = CONTEXT.with(|c| c.borrow().clone());
        let v = chr_of(c)?.borrow().is_god_mod(&*ctx);
        Ok(c.push(v))
    });
    b.method("isAsleep", |c| {
        let v = chr_of(c)?.borrow().is_asleep();
        Ok(c.push(v))
    });
    b.method("setAsleep", |c| {
        let v = bool_arg(c, 1);
        chr_of(c)?.borrow_mut().asleep = v;
        Ok(0)
    });
    b.method("isDead", |c| {
        let v = chr_of(c)?.borrow().is_dead();
        Ok(c.push(v))
    });
    b.method("getMaxWeight", |c| {
        let v = chr_of(c)?.borrow().max_weight();
        Ok(c.push(v))
    });
    b.method("setMaxWeight", |c| {
        let v = int_arg(c, 1);
        chr_of(c)?.borrow_mut().max_weight = v;
        Ok(0)
    });
    b.method("getBedType", |c| {
        let v = chr_of(c)?.borrow().bed_type.clone();
        Ok(c.push(v))
    });
    b.method("setBedType", |c| {
        let v = str_arg(c, 1).map(|s| s.to_string()).unwrap_or_default();
        chr_of(c)?.borrow_mut().bed_type = v;
        Ok(0)
    });
    b.method("getHoursSurvived", |c| {
        let ctx = CONTEXT.with(|c| c.borrow().clone());
        let v = chr_of(c)?.borrow().hours_survived(&*ctx);
        Ok(c.push(v))
    });
    let (s, ch) = (sm.clone(), chars.clone());
    b.method("getRecoveryMod", move |c| {
        let v = chr_of(c)?.borrow().recovery_mod(&s, &ch.borrow()).map_err(java_error)?;
        Ok(c.push(float_value(v)))
    });
    let (s, ch) = (sm.clone(), chars.clone());
    b.method("updateEnduranceWhileSitting", move |c| {
        let chr = chr_of(c)?;
        let v = character_value(&chr);
        let chars = ch.borrow();
        with_ctx(c, &v, |ctx| chr.borrow_mut().update_endurance_while_sitting(&s, &chars, ctx))?;
        Ok(0)
    });
    b.method("getDescriptor", |c| {
        let d = chr_of(c)?.borrow().descriptor.clone();
        Ok(c.push(d.as_ref().map_or(Value::Nil, survivor_desc_value)))
    });
    b.method("setDescriptor", |c| {
        let d = if c.arg(1).is_nil() { None } else { Some(desc_arg(c, 1)?) };
        adopt_survivor_desc_value(&c.arg(1));
        chr_of(c)?.borrow_mut().descriptor = d;
        Ok(0)
    });
    b.method("modifyTraitXPBoost", |c| {
        let def = trait_def_arg(c, 1)?;
        let removing = bool_arg(c, 2);
        chr_of(c)?.borrow_mut().modify_trait_xp_boost(def.as_ref(), removing).map_err(java_error)?;
        Ok(0)
    });
    let (s, ch) = (sm.clone(), chars.clone());
    b.method("applyTraits", move |c| {
        let chr = chr_of(c)?;
        let v = character_value(&chr);
        let traits: Option<Vec<Option<Rc<pz_scripts::registry::RegistryKey>>>> = match c.arg(1) {
            Value::Nil => None,
            Value::Object(o) => {
                let list =
                    o.as_any().downcast_ref::<JavaList>().ok_or_else(|| class_cast(&c.arg(1), "java.util.List"))?;
                let mut out = Vec::new();
                for item in list.snapshot() {
                    out.push(match &item {
                        Value::Nil => None,
                        Value::Object(k) => Some(
                            k.as_any()
                                .downcast_ref::<crate::bind_registry::KeyObject>()
                                .map(|k| k.key.clone())
                                .ok_or_else(|| class_cast(&item, crate::bind_registry::CHARACTER_TRAIT_CLASS))?,
                        ),
                        other => return Err(class_cast(other, crate::bind_registry::CHARACTER_TRAIT_CLASS)),
                    });
                }
                Some(out)
            }
            other => return Err(class_cast(&other, "java.util.List")),
        };
        let chars = ch.borrow();
        with_ctx(c, &v, |ctx| chr.borrow_mut().apply_traits(&s, &chars, traits.as_deref(), ctx))?;
        Ok(0)
    });
    let s = sm.clone();
    b.method("applyProfessionRecipes", move |c| {
        chr_of(c)?.borrow_mut().apply_profession_recipes(&s).map_err(java_error)?;
        Ok(0)
    });
    let s = sm.clone();
    b.method("applyCharacterTraitsRecipes", move |c| {
        chr_of(c)?.borrow_mut().apply_character_traits_recipes(&s);
        Ok(0)
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use pz_characters::SurvivorDesc;
    use pz_kahlua::TableRef;
    use pz_scripts::registry::builtin::{BuiltinTables, CharacterStatDecl, MoodleStatDecl, PerkDecl, PerkInit};

    fn run(t: &mut Thread, src: &str) -> Result<(), String> {
        let p = pz_kahlua::compiler::compile(src, Some("test"), Some("test"), None).unwrap();
        let f = Value::Closure(pz_kahlua::vm::LuaClosure::new(Rc::new(p), t.globals().clone()));
        t.pcall(&f, &[]).map(|_| ()).map_err(|(e, tr)| format!("{}\n{tr}", pz_kahlua::exec::java_value_of(&e)))
    }

    /// Invented stats, moodle types, traits and perks.
    fn setup() -> (Thread, LuaGameCharacterRef, Rc<RefCell<Vec<String>>>) {
        let mut t = Thread::new(TableRef::new());
        pz_kahlua::stdlib::install(&mut t, None).unwrap();
        run(&mut t, "function assert(v, msg) if not v then error(msg or 'assertion failed!', 2) end return v end")
            .unwrap();
        crate::bind_java_util::register(&mut t);
        let sm = Rc::new(ScriptManager::new());
        {
            let mut st = sm.statics.items.borrow_mut();
            for (reg, field, id) in [
                ("character_trait", "FAST_LEARNER", "t:quick"),
                ("character_trait", "STRONG", "t:strong"),
                ("character_trait", "SLOW_LEARNER", "t:slow"),
                ("character_trait", "PACIFIST", "t:calm"),
                ("character_trait", "CRAFTY", "t:handy"),
                ("moodle_type", "PANIC", "t:Scared"),
                ("moodle_type", "FOOD_EATEN", "t:Fed"),
            ] {
                sm.registries.get(reg).unwrap().borrow_mut().register_id(id, false).unwrap();
                st.constants.insert(reg, field, id);
            }
        }
        let mut tables = BuiltinTables::default();
        for f in ["PANIC", "FITNESS", "STRESS", "NICOTINE_WITHDRAWAL", "TEMPERATURE", "ZOMBIE_INFECTION"] {
            let max = if f == "PANIC" { 100.0 } else { 1.0 };
            tables.character_stats.push(CharacterStatDecl {
                field: f.into(),
                id: f.to_lowercase(),
                minimum: if f == "FITNESS" { -1.0 } else { 0.0 },
                maximum: max,
                default: 0.0,
            });
        }
        tables.moodle_stats = vec![MoodleStatDecl {
            field: "PANIC".into(),
            moodle_type: "PANIC".into(),
            thresholds: [0.0, 5.0, 10.0, 20.0, 40.0],
        }];
        tables.level_up_levels = vec![5, 10];
        let decl = |f: &str| PerkDecl { field: f.into(), id: f.into(), parent: None };
        tables.perks = ["None", "Fitness", "Strength", "Sprinting", "Knit", "MAX"].map(decl).to_vec();
        tables.perk_ids = tables.perks.iter().map(|p| p.id.clone()).collect();
        let xp = [10, 20, 30, 40, 50, 60, 70, 80, 90, 100];
        tables.perk_init = vec![PerkInit::SetParent { perk: "None".into(), parent: "None".into() }];
        for p in ["Fitness", "Strength", "Sprinting", "Knit"] {
            tables.perk_init.push(PerkInit::AddPerk {
                perk: p.into(),
                translation: p.into(),
                parent: None,
                xp,
                passiv: false,
            });
        }
        *sm.builtin_tables.borrow_mut() = tables.clone();
        let mut chars =
            Characters { perks: pz_characters::PerkFactory::from_tables(&tables).unwrap(), ..Default::default() };
        chars.perks.init(&tables, &|k| k.to_string()).unwrap();
        chars.sim = pz_characters::SimStatics::from_tables(&tables).unwrap();
        let desc = SurvivorDesc::<LuaItem>::new_unregistered(&mut chars, &sm).unwrap();
        let chars = Rc::new(RefCell::new(chars));
        crate::bind_registry::register(&mut t, sm.clone());
        crate::bind_characters::register(&mut t, sm.clone(), chars.clone());
        crate::bind_skills::register(&mut t, sm.clone(), chars.clone());
        register(&mut t, sm.clone(), chars);
        let mut chr = GameCharacter::new(&sm, CharacterKind::Player);
        chr.in_players = true;
        chr.descriptor = Some(Rc::new(RefCell::new(desc)));
        let chr = Rc::new(RefCell::new(chr));
        t.globals().set_str("player", character_value(&chr));
        let log: Rc<RefCell<Vec<String>>> = Rc::default();
        let l = log.clone();
        install_event_sink(Rc::new(move |_t: &mut Thread, name: &str, args: &[Value]| {
            let rest: Vec<String> = args[1..].iter().map(pz_kahlua::exec::java_value_of).collect();
            l.borrow_mut().push(format!("{name} {}", rest.join(" ")));
        }));
        (t, chr, log)
    }

    #[test]
    fn xp_and_perks_from_lua() {
        let (mut t, chr, log) = setup();
        run(
            &mut t,
            r#"
            local xp = player:getXp()
            assert(xp == player:getXp() and player:getStats() == player:getStats())
            xp:setPerkBoost(Perks.Knit, 2)
            assert(xp:getPerkBoost(Perks.Knit) == 2 and player:getDescriptor():getXPBoostMap():size() == 1)
            xp:AddXP(Perks.Knit, 20)
            assert(math.abs(xp:getXP(Perks.Knit) - 26.6) < 1e-4, tostring(xp:getXP(Perks.Knit)))
            assert(player:getPerkLevel(Perks.Knit) == 1 and player:getPerkInfo(Perks.Knit):getLevel() == 1)
            assert(player:getPerkList():size() == 1 and player:getPerkInfo(Perks.Strength) == nil)
            xp:AddXP(Perks.Knit, 10, true)
            assert(math.abs(xp:getXP(Perks.Knit) - 36.6) < 1e-4)
            xp:addXpMultiplier(Perks.Strength, 3, 1, 10)
            assert(xp:getMultiplier(Perks.Strength) == 3 and xp:getMultiplierMap():containsKey(Perks.Strength))
            player:LevelPerk(Perks.Sprinting)
            player:LoseLevel(Perks.Sprinting)
            assert(player:getPerkLevel(Perks.Sprinting) == 0)
            assert(player:getXpForLevel(1) == 25 and player:getLevelUpLevels(9) == 10)
            assert(not pcall(function() player:LevelPerk(Perks.MAX) end))
            player:setPerkLevelDebug(Perks.Fitness, 7)
            assert(player:getPerkLevel(Perks.Fitness) == 7)
            "#,
        )
        .unwrap();
        let log = log.borrow();
        assert_eq!(log[0], "LevelPerk Knit 1.0 true");
        assert!(log[1].starts_with("AddXP Knit 26.6"), "{}", log[1]);
        assert!(log.contains(&"LevelPerk Sprinting 0.0 false".to_string()));
        assert_eq!(chr.borrow().perk_list.len(), 3);
    }

    #[test]
    fn stats_and_moodles_from_lua() {
        let (mut t, chr, _) = setup();
        run(
            &mut t,
            r#"
            local s = player:getStats()
            assert(CharacterStat.PANIC:getId() == "panic" and CharacterStat.getById("panic") == CharacterStat.PANIC)
            assert(s:set(CharacterStat.PANIC, 150) and s:get(CharacterStat.PANIC) == 100)
            assert(s:isAtMaximum(CharacterStat.PANIC) and not s:isAtMinimum(CharacterStat.PANIC))
            s:remove(CharacterStat.PANIC, 85)
            assert(s:get(CharacterStat.PANIC) == 15)
            local free = Stats.new()
            assert(free:get(CharacterStat.PANIC) == 0 and free:add(CharacterStat.STRESS, 2) and free:getNicotineStress() == 1)
            local m = player:getMoodles()
            m:Update()
            assert(m:getMoodleLevel(MoodleType.PANIC) == 2, tostring(m:getMoodleLevel(MoodleType.PANIC)))
            assert(m:UI_RefreshNeeded(), "refresh")
            assert(not m:UI_RefreshNeeded(), "refreshed")
            assert(m:getMoodleDisplayString(MoodleType.PANIC) == "Moodles_t:Scared_lvl2")
            assert(m:getGoodBadNeutral(MoodleType.FOOD_EATEN) == Moodles.GoodMoodleType)
            assert(MoodleType.PANIC:getTranslationName() == "t:Scared")
            assert(Moodle.MoodleLevel.MaxMoodleLevel:ordinal() == 4)
            local r = CharacterStat.register("custom", 0, 5, 2)
            assert(s:get(r) == 2 and tostring(r) == "CharacterStat{id='custom', min=0.0, max=5.0, default=2.0}")
            "#,
        )
        .unwrap();
        assert_eq!(chr.borrow().moodles.all().iter().map(|m| m.level()).max(), Some(2));
    }
}
