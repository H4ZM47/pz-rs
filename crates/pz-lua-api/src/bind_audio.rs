//! Audio classes exposed to Lua: `zombie.audio.MusicIntensityConfig`,
//! `MusicIntensityEvent`, `MusicIntensityEvents`, `MusicThreatConfig`,
//! `MusicThreatStatus` and `MusicThreatStatuses`.
//!
//! The configs are singletons (`getInstance()`), filled from Lua tables by
//! `initEvents`/`initStatuses` exactly as the original reads them through
//! `KahluaTableImpl`: entries in table order, the `VERSION` key skipped,
//! `rawgetStr` as a cast (a non-string raises `ClassCastException`),
//! `rawgetFloat`/`rawgetInt` returning -1 for non-numbers, and a later entry
//! with the same id replacing the earlier one at the end of the list.
//!
//! `zombie.SoundManager` is exposed through `getSoundManager()` over a
//! shared [`pz_sound::system::SoundSystem`]: call [`install_sound_system`]
//! with the engine's before Lua starts; without one a silent system is used
//! (the original's `DummySoundManager` when sound is off). Methods that need
//! the world (`PlaySound`, `PlayWorldSound*`) return nil until it is ported.
//! `getUIEmitter()` returns the manager's `fmod.fmod.FMODSoundEmitter`, the
//! same one `playUISound` plays on, with the `BaseSoundEmitter` methods.
//!
//! Character-dependent parts (`checkHealthPanelVisible`,
//! `restoreToFullHealth`, `MusicThreatStatuses.update`) only act on an
//! `IsoPlayer`, which is not ported yet, so they do nothing for now.

use crate::expose::{expose_class, num_arg, str_arg};
use pz_kahlua::value::LuaObject;
use pz_kahlua::{LuaError, LuaResult, NativeCall, TableRef, Thread, Value};
use std::any::Any;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

pub const INTENSITY_CONFIG_CLASS: &str = "zombie.audio.MusicIntensityConfig";
pub const INTENSITY_EVENT_CLASS: &str = "zombie.audio.MusicIntensityEvent";
pub const INTENSITY_EVENTS_CLASS: &str = "zombie.audio.MusicIntensityEvents";
pub const THREAT_CONFIG_CLASS: &str = "zombie.audio.MusicThreatConfig";
pub const THREAT_STATUS_CLASS: &str = "zombie.audio.MusicThreatStatus";
pub const THREAT_STATUSES_CLASS: &str = "zombie.audio.MusicThreatStatuses";
pub const SOUND_MANAGER_CLASS: &str = "zombie.SoundManager";
pub const BASE_SOUND_MANAGER_CLASS: &str = "zombie.BaseSoundManager";
pub const BASE_SOUND_EMITTER_CLASS: &str = "zombie.audio.BaseSoundEmitter";
pub const FMOD_SOUND_EMITTER_CLASS: &str = "fmod.fmod.FMODSoundEmitter";

thread_local! {
    static SOUND: RefCell<Option<Rc<RefCell<pz_sound::system::SoundSystem>>>> = const { RefCell::new(None) };
    static SOUND_HANDLE: Rc<SoundManagerObject> = Rc::new(SoundManagerObject);
    static UI_EMITTER_HANDLE: Rc<SoundEmitterObject> = Rc::new(SoundEmitterObject::Ui);
}

/// Make `system` the one `getSoundManager()` drives.
pub fn install_sound_system(system: Rc<RefCell<pz_sound::system::SoundSystem>>) {
    SOUND.with(|s| *s.borrow_mut() = Some(system));
}

/// The shared sound system, created silent on first use.
pub fn sound_system() -> Rc<RefCell<pz_sound::system::SoundSystem>> {
    SOUND.with(|s| {
        s.borrow_mut().get_or_insert_with(|| Rc::new(RefCell::new(pz_sound::system::SoundSystem::silent()))).clone()
    })
}

/// The Lua handle of `SoundManager.instance`.
pub struct SoundManagerObject;

impl LuaObject for SoundManagerObject {
    fn class_name(&self) -> &str {
        SOUND_MANAGER_CLASS
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// [`crate::hud::host::HudSound`] over the shared sound system, the same
/// `SoundManager.instance` that `getSoundManager()` returns.
pub struct SharedHudSound;

impl crate::hud::host::HudSound for SharedHudSound {
    fn pause_sound_and_music(&mut self, keep_music: bool) {
        sound_system().borrow_mut().with(|m, ctx| m.pause_sound_and_music(keep_music, false, ctx));
    }
    fn resume_sound_and_music(&mut self) {
        sound_system().borrow_mut().with(|m, ctx| m.resume_sound_and_music(ctx));
    }
    fn set_music_state(&mut self, state: &str) {
        sound_system().borrow().manager.set_music_state(state);
    }
    fn play_ui_sound(&mut self, name: &str) {
        sound_system().borrow_mut().with(|m, ctx| m.play_ui_sound(name, None, ctx));
    }
}

/// A Lua handle to an `FMODSoundEmitter` owned by the sound system.
pub enum SoundEmitterObject {
    /// `SoundManager.getUIEmitter()`.
    Ui,
}

impl LuaObject for SoundEmitterObject {
    fn class_name(&self) -> &str {
        FMOD_SOUND_EMITTER_CLASS
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

fn java_error(message: String, class: &'static str) -> LuaError {
    LuaError::Runtime { message, class }
}

fn npe(what: &str) -> LuaError {
    java_error(format!("Cannot invoke \"{what}\" because value is null"), "java.lang.NullPointerException")
}

/// `StringUtils.discardNullOrWhitespace`: Java `trim()` drops chars <= ' '.
fn discard_null_or_whitespace(s: Option<Rc<str>>) -> Option<Rc<str>> {
    s.filter(|s| !s.trim_matches(|c: char| c <= ' ').is_empty())
}

/// `KahluaTableImpl.rawgetStr`: `(String) rawget(key)`.
fn rawget_str(t: &TableRef, key: &str) -> LuaResult<Option<Rc<str>>> {
    match t.rawget(&Value::str(key)) {
        Value::Nil => Ok(None),
        Value::Str(s) => Ok(Some(s)),
        other => Err(java_error(
            format!("class {} cannot be cast to class java.lang.String", pz_kahlua::stdlib::java_class_name(&other)),
            "java.lang.ClassCastException",
        )),
    }
}

/// `KahluaTableImpl.rawgetFloat`.
fn rawget_float(t: &TableRef, key: &str) -> f32 {
    match t.rawget(&Value::str(key)) {
        Value::Num(n) => n as f32,
        _ => -1.0,
    }
}

/// `KahluaTableImpl.rawgetInt`.
fn rawget_int(t: &TableRef, key: &str) -> i32 {
    match t.rawget(&Value::str(key)) {
        Value::Num(n) => n as i32,
        _ => -1,
    }
}

/// `(KahluaTableImpl) value`.
fn table_cast(v: Value) -> LuaResult<TableRef> {
    match v {
        Value::Table(t) => Ok(t),
        other => Err(java_error(
            format!(
                "class {} cannot be cast to class se.krka.kahlua.j2se.KahluaTableImpl",
                pz_kahlua::stdlib::java_class_name(&other)
            ),
            "java.lang.ClassCastException",
        )),
    }
}

/// `it.getKey().toString()` for the `VERSION` check.
fn key_string(k: &Value) -> String {
    pz_kahlua::exec::java_value_of(k)
}

/// Java `String.equalsIgnoreCase` (`StringUtils.equalsIgnoreCase` is
/// null-safe; both null compare equal).
fn eq_ignore_case(a: Option<&str>, b: Option<&str>) -> bool {
    match (a, b) {
        (Some(a), Some(b)) => a.chars().count() == b.chars().count() && a.to_lowercase() == b.to_lowercase(),
        (None, None) => true,
        _ => false,
    }
}

// ---------------------------------------------------------------------------
// MusicIntensityConfig
// ---------------------------------------------------------------------------

#[derive(Clone)]
struct IntensityEventDef {
    id: Rc<str>,
    intensity: f32,
    duration: i64,
    multiple: bool,
}

#[derive(Default)]
pub struct MusicIntensityConfig {
    events: Vec<IntensityEventDef>,
}

impl MusicIntensityConfig {
    fn by_id(&self, id: Option<&str>) -> Option<&IntensityEventDef> {
        let id = id?;
        // HashMap lookup: exact match; a later duplicate replaced the earlier.
        self.events.iter().find(|e| &*e.id == id)
    }

    pub fn init_events(&mut self, table: &TableRef) -> LuaResult<()> {
        self.events.clear();
        for (k, v) in table.entries() {
            if key_string(&k).eq_ignore_ascii_case("VERSION") {
                continue;
            }
            let t = table_cast(v)?;
            let id = discard_null_or_whitespace(rawget_str(&t, "id")?);
            let intensity = rawget_float(&t, "intensity");
            let duration = rawget_int(&t, "duration") as i64;
            let multiple = match t.rawget(&Value::str("multiple")) {
                Value::Bool(b) => b,
                _ => true,
            };
            let Some(id) = id else { continue };
            self.events.retain(|e| e.id != id);
            self.events.push(IntensityEventDef { id, intensity, duration, multiple });
        }
        Ok(())
    }
}

thread_local! {
    static INTENSITY_CONFIG: Rc<RefCell<MusicIntensityConfig>> = Rc::new(RefCell::new(MusicIntensityConfig::default()));
    static THREAT_CONFIG: Rc<RefCell<MusicThreatConfig>> = Rc::new(RefCell::new(MusicThreatConfig::default()));
    // One Lua handle per singleton so `getInstance() == getInstance()`.
    static INTENSITY_HANDLE: Rc<IntensityConfigObject> = Rc::new(IntensityConfigObject);
    static THREAT_HANDLE: Rc<ThreatConfigObject> = Rc::new(ThreatConfigObject);
}

/// `MusicIntensityConfig.getInstance()`.
pub fn intensity_config() -> Rc<RefCell<MusicIntensityConfig>> {
    INTENSITY_CONFIG.with(Rc::clone)
}

/// `MusicThreatConfig.getInstance()`.
pub fn threat_config() -> Rc<RefCell<MusicThreatConfig>> {
    THREAT_CONFIG.with(Rc::clone)
}

struct IntensityConfigObject;

impl LuaObject for IntensityConfigObject {
    fn class_name(&self) -> &str {
        INTENSITY_CONFIG_CLASS
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

// ---------------------------------------------------------------------------
// MusicIntensityEvent(s)
// ---------------------------------------------------------------------------

pub struct MusicIntensityEvent {
    id: Option<Rc<str>>,
    intensity: f32,
    duration_ms: i64,
    elapsed_ms: Cell<i64>,
}

impl LuaObject for MusicIntensityEvent {
    fn class_name(&self) -> &str {
        INTENSITY_EVENT_CLASS
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

#[derive(Default)]
pub struct MusicIntensityEvents {
    events: RefCell<Vec<Rc<MusicIntensityEvent>>>,
    update_time_ms: Cell<i64>,
    intensity: Cell<f32>,
}

impl MusicIntensityEvents {
    pub fn new() -> Self {
        MusicIntensityEvents { update_time_ms: Cell::new(-1), ..Default::default() }
    }

    pub fn add_event(
        &self,
        id: Option<Rc<str>>,
        intensity: f32,
        duration_ms: i64,
        multiple: bool,
    ) -> Rc<MusicIntensityEvent> {
        if !multiple {
            if let Some(e) = self.find_event_by_id(id.as_deref()) {
                e.elapsed_ms.set(0);
                return e;
            }
        }
        let e = Rc::new(MusicIntensityEvent { id, intensity, duration_ms, elapsed_ms: Cell::new(0) });
        self.events.borrow_mut().push(e.clone());
        e
    }

    pub fn find_event_by_id(&self, id: Option<&str>) -> Option<Rc<MusicIntensityEvent>> {
        self.events.borrow().iter().find(|e| eq_ignore_case(e.id.as_deref(), id)).cloned()
    }

    fn calculate_intensity(&self) -> f32 {
        let mut i = 50.0f32;
        for e in self.events.borrow().iter() {
            i += e.intensity;
        }
        i.clamp(0.0, 100.0)
    }

    /// `update()` with `System.currentTimeMillis()` passed in.
    pub fn update_at(&self, now_ms: i64) {
        let elapsed = now_ms - self.update_time_ms.get();
        self.update_time_ms.set(now_ms);
        self.events.borrow_mut().retain(|e| {
            if e.duration_ms <= 0 && e.elapsed_ms.get() > 0 {
                return false;
            }
            e.elapsed_ms.set(e.elapsed_ms.get() + elapsed);
            !(e.duration_ms > 0 && e.elapsed_ms.get() >= e.duration_ms)
        });
        self.intensity.set(self.calculate_intensity());
    }
}

impl LuaObject for MusicIntensityEvents {
    fn class_name(&self) -> &str {
        INTENSITY_EVENTS_CLASS
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

fn now_ms() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis() as i64)
}

// ---------------------------------------------------------------------------
// MusicThreatConfig, MusicThreatStatus(es)
// ---------------------------------------------------------------------------

struct ThreatStatusDef {
    id: Rc<str>,
    intensity: f32,
    intensity_override: f32,
}

#[derive(Default)]
pub struct MusicThreatConfig {
    statuses: Vec<ThreatStatusDef>,
}

impl MusicThreatConfig {
    fn by_id(&self, id: Option<&str>) -> Option<&ThreatStatusDef> {
        let id = id?;
        self.statuses.iter().find(|s| &*s.id == id)
    }

    pub fn init_statuses(&mut self, table: &TableRef) -> LuaResult<()> {
        self.statuses.clear();
        for (k, v) in table.entries() {
            if key_string(&k).eq_ignore_ascii_case("VERSION") {
                continue;
            }
            let t = table_cast(v)?;
            let id = discard_null_or_whitespace(rawget_str(&t, "id")?);
            let intensity = rawget_float(&t, "intensity");
            let Some(id) = id else { continue };
            if intensity <= 0.0 || intensity.is_nan() {
                continue;
            }
            self.statuses.retain(|s| s.id != id);
            self.statuses.push(ThreatStatusDef { id, intensity, intensity_override: f32::NAN });
        }
        Ok(())
    }

    pub fn status_intensity(&self, id: Option<&str>) -> f32 {
        self.by_id(id).map_or(0.0, |s| s.intensity)
    }

    pub fn is_overridden(&self, id: Option<&str>) -> bool {
        self.by_id(id).is_some_and(|s| !s.intensity_override.is_nan())
    }

    pub fn status_intensity_override(&self, id: Option<&str>) -> f32 {
        self.by_id(id).map_or(0.0, |s| s.intensity_override)
    }
}

struct ThreatConfigObject;

impl LuaObject for ThreatConfigObject {
    fn class_name(&self) -> &str {
        THREAT_CONFIG_CLASS
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

pub struct MusicThreatStatus {
    id: Option<Rc<str>>,
    intensity: Cell<f32>,
}

impl MusicThreatStatus {
    pub fn intensity(&self) -> f32 {
        let cfg = threat_config();
        let cfg = cfg.borrow();
        if cfg.is_overridden(self.id.as_deref()) {
            cfg.status_intensity_override(self.id.as_deref())
        } else {
            self.intensity.get()
        }
    }
}

impl LuaObject for MusicThreatStatus {
    fn class_name(&self) -> &str {
        THREAT_STATUS_CLASS
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

#[derive(Default)]
pub struct MusicThreatStatuses {
    statuses: RefCell<Vec<Rc<MusicThreatStatus>>>,
    intensity: Cell<f32>,
}

impl MusicThreatStatuses {
    pub fn set_status(&self, id: Option<Rc<str>>, intensity: f32) -> Rc<MusicThreatStatus> {
        if let Some(s) = self.find_status_by_id(id.as_deref()) {
            s.intensity.set(intensity);
            return s;
        }
        let s = Rc::new(MusicThreatStatus { id, intensity: Cell::new(intensity) });
        self.statuses.borrow_mut().push(s.clone());
        s
    }

    pub fn find_status_by_id(&self, id: Option<&str>) -> Option<Rc<MusicThreatStatus>> {
        self.statuses.borrow().iter().find(|s| eq_ignore_case(s.id.as_deref(), id)).cloned()
    }

    fn calculate_intensity(&self) -> f32 {
        let cfg = threat_config();
        let mut total = 0.0f32;
        for s in self.statuses.borrow().iter() {
            let weight = cfg.borrow().status_intensity(s.id.as_deref());
            total += s.intensity() * weight;
        }
        total
    }
}

impl LuaObject for MusicThreatStatuses {
    fn class_name(&self) -> &str {
        THREAT_STATUSES_CLASS
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

// ---------------------------------------------------------------------------
// Lua glue
// ---------------------------------------------------------------------------

fn obj<T: 'static>(c: &NativeCall<'_>, i: usize) -> LuaResult<Rc<dyn LuaObject>> {
    match c.arg(i) {
        Value::Object(o) if o.as_any().is::<T>() => Ok(o),
        Value::Nil => Err(npe("object")),
        other => Err(java_error(
            format!(
                "class {} cannot be cast to class {}",
                pz_kahlua::stdlib::java_class_name(&other),
                std::any::type_name::<T>()
            ),
            "java.lang.ClassCastException",
        )),
    }
}

fn down<T: 'static>(o: &Rc<dyn LuaObject>) -> &T {
    o.as_any().downcast_ref::<T>().unwrap()
}

fn push_obj<T: LuaObject + 'static>(c: &mut NativeCall<'_>, v: Option<Rc<T>>) -> usize {
    match v {
        Some(v) => c.push(Value::Object(v)),
        None => c.push(Value::Nil),
    }
}

fn idx_arg(c: &NativeCall<'_>, i: usize) -> i32 {
    crate::expose::int_arg(c, i)
}

fn out_of_bounds(index: i32, len: usize) -> LuaError {
    java_error(format!("Index {index} out of bounds for length {len}"), "java.lang.IndexOutOfBoundsException")
}

fn sm_this(c: &NativeCall<'_>) -> LuaResult<()> {
    obj::<SoundManagerObject>(c, 0).map(|_| ())
}

/// Java `long` and `float` returns become Lua numbers.
fn num(c: &mut NativeCall<'_>, v: f64) -> usize {
    c.push(v)
}

fn register_sound_manager(thread: &mut Thread) {
    expose_class(thread, BASE_SOUND_MANAGER_CLASS, None);
    let b = expose_class(thread, SOUND_MANAGER_CLASS, Some(BASE_SOUND_MANAGER_CLASS));
    crate::expose::global(thread, "getSoundManager", |c| {
        let h: Rc<dyn LuaObject> = SOUND_HANDLE.with(|h| h.clone());
        Ok(c.push(Value::Object(h)))
    });
    b.method("getUIEmitter", |c| {
        sm_this(c)?;
        let h: Rc<dyn LuaObject> = UI_EMITTER_HANDLE.with(|h| h.clone());
        Ok(c.push(Value::Object(h)))
    });
    b.method("playUISound", |c| {
        sm_this(c)?;
        let name = str_arg(c, 1).unwrap_or_else(|| Rc::from(""));
        let r = sound_system().borrow_mut().with(|m, ctx| m.play_ui_sound(&name, None, ctx));
        Ok(num(c, r as f64))
    });
    b.method("stopUISound", |c| {
        sm_this(c)?;
        let inst = num_arg(c, 1) as i64;
        sound_system().borrow_mut().with(|m, ctx| m.stop_ui_sound(inst, ctx));
        Ok(0)
    });
    b.method("isPlayingUISound", |c| {
        sm_this(c)?;
        let v = match c.arg(1) {
            Value::Str(s) => sound_system().borrow_mut().with(|m, _| m.is_playing_ui_sound(&s)),
            _ => {
                let inst = num_arg(c, 1) as i64;
                sound_system().borrow_mut().with(|m, _| m.is_playing_ui_sound_instance(inst))
            }
        };
        Ok(c.push(v))
    });
    b.method("setUiSoundMuted", |c| {
        sm_this(c)?;
        let v = matches!(c.arg(1), Value::Bool(true));
        sound_system().borrow_mut().manager.set_ui_sound_muted(v);
        Ok(0)
    });
    b.method("isUiSoundMuted", |c| {
        sm_this(c)?;
        let v = sound_system().borrow().manager.is_ui_sound_muted();
        Ok(c.push(v))
    });
    b.method("playMusic", |c| {
        sm_this(c)?;
        let name = str_arg(c, 1).unwrap_or_else(|| Rc::from(""));
        sound_system().borrow_mut().with(|m, ctx| m.play_music(&name, ctx));
        Ok(0)
    });
    b.method("stopMusic", |c| {
        sm_this(c)?;
        let name = str_arg(c, 1);
        sound_system().borrow_mut().with(|m, ctx| m.stop_music(name.as_deref(), ctx.backend));
        Ok(0)
    });
    b.method("StopMusic", |c| {
        sm_this(c)?;
        sound_system().borrow_mut().with(|m, ctx| m.stop_music_all(ctx.backend));
        Ok(0)
    });
    b.method("isPlayingMusic", |c| {
        sm_this(c)?;
        let v = sound_system().borrow_mut().with(|m, ctx| m.is_playing_music(ctx.backend));
        Ok(c.push(v))
    });
    b.method("IsMusicPlaying", |c| {
        sm_this(c)?;
        Ok(c.push(false))
    });
    b.method("getMusicPosition", |c| {
        sm_this(c)?;
        let v = sound_system().borrow_mut().with(|m, ctx| m.music_position(ctx.backend));
        Ok(num(c, v as f64))
    });
    b.method("getCurrentMusicName", |c| {
        sm_this(c)?;
        let v = sound_system().borrow_mut().with(|m, ctx| m.current_music_name(ctx.backend));
        Ok(c.push(v.map_or(Value::Nil, |s| Value::str(&s))))
    });
    b.method("getCurrentMusicLibrary", |c| {
        sm_this(c)?;
        let v = sound_system().borrow_mut().with(|m, ctx| m.current_music_library(ctx.backend));
        Ok(c.push(v.map_or(Value::Nil, |s| Value::str(&s))))
    });
    b.method("setMusicState", |c| {
        sm_this(c)?;
        let name = str_arg(c, 1).ok_or_else(|| npe("String.hashCode()"))?;
        sound_system().borrow().manager.set_music_state(&name);
        Ok(0)
    });
    b.method("isRemastered", |c| {
        sm_this(c)?;
        let sys = sound_system();
        let mut sys = sys.borrow_mut();
        let pz_sound::system::SoundSystem { manager, rand_next, .. } = &mut *sys;
        let v = manager.is_remastered(rand_next.as_mut());
        drop(sys);
        Ok(c.push(v))
    });
    b.method("pauseSoundAndMusic", |c| {
        sm_this(c)?;
        let keep = matches!(c.arg(1), Value::Bool(true));
        sound_system().borrow_mut().with(|m, ctx| m.pause_sound_and_music(keep, false, ctx));
        Ok(0)
    });
    b.method("resumeSoundAndMusic", |c| {
        sm_this(c)?;
        sound_system().borrow_mut().with(|m, ctx| m.resume_sound_and_music(ctx));
        Ok(0)
    });
    macro_rules! volume {
        ($get:literal, $set:literal, $field:ident, $setter:ident) => {
            b.method($get, |c| {
                sm_this(c)?;
                let v = sound_system().borrow().manager.$field;
                Ok(num(c, v as f64))
            });
            b.method($set, |c| {
                sm_this(c)?;
                let v = num_arg(c, 1) as f32;
                sound_system().borrow_mut().with(|m, ctx| m.$setter(v, ctx.backend));
                Ok(0)
            });
        };
    }
    volume!("getSoundVolume", "setSoundVolume", sound_volume, set_sound_volume);
    volume!("getMusicVolume", "setMusicVolume", music_volume, set_music_volume);
    volume!("getVehicleEngineVolume", "setVehicleEngineVolume", vehicle_engine_volume, set_vehicle_engine_volume);
    b.method("setAmbientVolume", |c| {
        sm_this(c)?;
        let v = num_arg(c, 1) as f32;
        sound_system().borrow_mut().with(|m, ctx| m.set_ambient_volume(v, ctx.backend));
        Ok(0)
    });
    b.method("getAmbientVolume", |c| {
        sm_this(c)?;
        Ok(num(c, 1.0))
    });
    b.method("getAmbientPieces", |c| {
        sm_this(c)?;
        Ok(c.push(crate::bind_java_util::list_value(Vec::new())))
    });
    // World sounds need IsoWorld's free emitters (not ported yet): the
    // original returns null when there is no world.
    for name in ["PlaySound", "PlayWorldSound", "PlayWorldSoundImpl", "PlayWorldSoundWav", "PlayWorldSoundWavImpl"] {
        b.method(name, |c| {
            sm_this(c)?;
            Ok(c.push(Value::Nil))
        });
    }
}

/// Run `f` on the emitter `self` (argument 0) refers to.
fn with_emitter<R>(
    c: &NativeCall<'_>,
    f: impl FnOnce(&mut pz_sound::emitter::FmodSoundEmitter, &mut pz_sound::emitter::Ctx) -> R,
) -> LuaResult<R> {
    let o = obj::<SoundEmitterObject>(c, 0)?;
    let target = down::<SoundEmitterObject>(&o);
    Ok(sound_system().borrow_mut().with(|m, ctx| match target {
        SoundEmitterObject::Ui => f(m.ui_emitter(), ctx),
    }))
}

/// A sound name argument; nil finds no sound.
fn name_arg(c: &NativeCall<'_>, i: usize) -> LuaResult<Rc<str>> {
    Ok(str_arg(c, i).unwrap_or_else(|| Rc::from("")))
}

fn register_sound_emitter(thread: &mut Thread) {
    use pz_sound::emitter::ParentObject;
    expose_class(thread, BASE_SOUND_EMITTER_CLASS, None);
    let b = expose_class(thread, FMOD_SOUND_EMITTER_CLASS, Some(BASE_SOUND_EMITTER_CLASS));
    // `void` methods.
    macro_rules! void {
        ($name:literal, |$c:ident, $e:ident, $ctx:ident| $body:expr) => {
            b.method($name, |$c| {
                with_emitter($c, |$e, $ctx| {
                    let _ = &$ctx;
                    $body;
                })?;
                Ok(0)
            });
        };
    }
    macro_rules! boolean {
        ($name:literal, |$c:ident, $e:ident, $ctx:ident| $body:expr) => {
            b.method($name, |$c| {
                let v = with_emitter($c, |$e, $ctx| {
                    let _ = &$ctx;
                    $body
                })?;
                Ok($c.push(v))
            });
        };
    }
    fn reference(c: &NativeCall<'_>) -> i64 {
        num_arg(c, 1) as i64
    }
    fn flag(c: &NativeCall<'_>, i: usize) -> bool {
        matches!(c.arg(i), Value::Bool(true))
    }

    // Empty in FMODSoundEmitter.
    b.method("randomStart", |c| {
        obj::<SoundEmitterObject>(c, 0)?;
        Ok(0)
    });
    b.method("setPos", |c| {
        let (x, y, z) = (num_arg(c, 1) as f32, num_arg(c, 2) as f32, num_arg(c, 3) as f32);
        with_emitter(c, |e, _| e.set_pos(x, y, z))?;
        Ok(0)
    });
    b.method("stopSound", |c| {
        let r = reference(c);
        let v = with_emitter(c, |e, ctx| e.stop_sound(r, ctx))?;
        Ok(num(c, v as f64))
    });
    b.method("stopSoundDelayRelease", |c| {
        let r = reference(c);
        let v = with_emitter(c, |e, ctx| e.stop_sound_delay_release(r, ctx))?;
        Ok(num(c, v as f64))
    });
    b.method("stopSoundLocal", |c| {
        let r = reference(c);
        with_emitter(c, |e, ctx| e.stop_sound_local(r, ctx))?;
        Ok(0)
    });
    b.method("stopOrTriggerSoundLocal", |c| {
        let r = reference(c);
        with_emitter(c, |e, ctx| e.stop_or_trigger_sound_local(r, ctx))?;
        Ok(0)
    });
    b.method("stopOrTriggerSound", |c| {
        let r = reference(c);
        with_emitter(c, |e, ctx| e.stop_or_trigger_sound(r, ctx))?;
        Ok(0)
    });
    b.method("stopSoundByName", |c| {
        let name = name_arg(c, 1)?;
        let v = with_emitter(c, |e, ctx| e.stop_sound_by_name(&name, ctx))?;
        Ok(num(c, v as f64))
    });
    b.method("stopOrTriggerSoundByName", |c| {
        let name = name_arg(c, 1)?;
        with_emitter(c, |e, ctx| e.stop_or_trigger_sound_by_name(&name, ctx))?;
        Ok(0)
    });
    b.method("setVolume", |c| {
        let (r, v) = (reference(c), num_arg(c, 2) as f32);
        with_emitter(c, |e, _| e.set_volume(r, v))?;
        Ok(0)
    });
    b.method("setPitch", |c| {
        let (r, v) = (reference(c), num_arg(c, 2) as f32);
        with_emitter(c, |e, _| e.set_pitch(r, v))?;
        Ok(0)
    });
    boolean!("hasSustainPoints", |c, e, ctx| e.has_sustain_points(reference(c), ctx.sounds));
    b.method("setParameterValueByName", |c| {
        let r = reference(c);
        let name = str_arg(c, 2).unwrap_or_else(|| Rc::from(""));
        let v = num_arg(c, 3) as f32;
        with_emitter(c, |e, ctx| e.set_parameter_value_by_name(r, &name, v, ctx))?;
        Ok(0)
    });
    b.method("isUsingParameter", |c| {
        let r = reference(c);
        let name = str_arg(c, 2).unwrap_or_else(|| Rc::from(""));
        let v = with_emitter(c, |e, ctx| e.is_using_parameter(r, &name, ctx))?;
        Ok(c.push(v))
    });
    b.method("setTimelinePosition", |c| {
        let r = reference(c);
        let name = str_arg(c, 2).unwrap_or_else(|| Rc::from(""));
        with_emitter(c, |e, ctx| e.set_timeline_position(r, &name, ctx))?;
        Ok(0)
    });
    void!("triggerCue", |c, e, ctx| e.trigger_cue(reference(c), ctx));
    b.method("setVolumeAll", |c| {
        let v = num_arg(c, 1) as f32;
        with_emitter(c, |e, _| e.set_volume_all(v))?;
        Ok(0)
    });
    void!("stopAll", |c, e, ctx| e.stop_all(ctx));
    // The IsoGameCharacter/IsoGridSquare/IsoObject overloads have no parent
    // object to follow until those classes are bound.
    b.method("playSound", |c| {
        let name = name_arg(c, 1)?;
        let v = match (c.arg(2), c.arg(3), c.arg(4)) {
            (Value::Num(x), Value::Num(y), Value::Num(z)) => {
                let (x, y, z) = (x as i32, y as i32, z as i32);
                with_emitter(c, |e, ctx| e.play_sound_at(&name, x, y, z, ctx))?
            }
            (Value::Bool(_), _, _) => with_emitter(c, |e, ctx| e.play_sound(&name, ctx))?,
            _ => with_emitter(c, |e, ctx| e.play_sound(&name, ctx))?,
        };
        Ok(num(c, v as f64))
    });
    b.method("playSoundImpl", |c| {
        let name = name_arg(c, 1)?;
        let v = with_emitter(c, |e, ctx| e.play_sound_impl(&name, ParentObject::None, ctx))?;
        Ok(num(c, v as f64))
    });
    for m in ["playSoundLooped", "playSoundLoopedImpl"] {
        b.method(m, |c| {
            let name = name_arg(c, 1)?;
            let v = with_emitter(c, |e, ctx| e.play_sound_looped_impl(&name, ctx))?;
            Ok(num(c, v as f64))
        });
    }
    b.method("playAmbientSound", |c| {
        let name = name_arg(c, 1)?;
        let v = with_emitter(c, |e, ctx| e.play_ambient_sound(&name, ctx))?;
        Ok(num(c, v as f64))
    });
    b.method("playAmbientLoopedImpl", |c| {
        let name = name_arg(c, 1)?;
        let v = with_emitter(c, |e, ctx| e.play_ambient_looped_impl(&name, ctx))?;
        Ok(num(c, v as f64))
    });
    b.method("set3D", |c| {
        let (r, on) = (reference(c), flag(c, 2));
        with_emitter(c, |e, ctx| e.set_3d(r, on, ctx))?;
        Ok(0)
    });
    void!("tick", |c, e, ctx| e.tick(ctx));
    boolean!("hasSoundsToStart", |c, e, ctx| e.has_sounds_to_start());
    boolean!("isEmpty", |c, e, ctx| e.is_empty());
    b.method("isPlaying", |c| {
        let v = match c.arg(1) {
            Value::Str(s) => with_emitter(c, |e, _| e.is_playing_name(&s))?,
            _ => {
                let r = reference(c);
                with_emitter(c, |e, _| e.is_playing(r))?
            }
        };
        Ok(c.push(v))
    });
    b.method("restart", |c| {
        let r = reference(c);
        let v = with_emitter(c, |e, ctx| e.restart(r, ctx))?;
        Ok(c.push(v))
    });
    b.method("setPlayRemoteEvents", |c| {
        let on = flag(c, 1);
        with_emitter(c, |e, _| e.set_play_remote_events(on))?;
        Ok(0)
    });
}

pub fn register(thread: &mut Thread) {
    register_sound_manager(thread);
    register_sound_emitter(thread);
    let b = expose_class(thread, INTENSITY_CONFIG_CLASS, None);
    b.static_fn("getInstance", |c| {
        let h: Rc<dyn LuaObject> = INTENSITY_HANDLE.with(|h| h.clone());
        Ok(c.push(Value::Object(h)))
    });
    b.method("initEvents", |c| {
        obj::<IntensityConfigObject>(c, 0)?;
        let t = table_cast(c.arg(1))?;
        intensity_config().borrow_mut().init_events(&t)?;
        Ok(0)
    });
    b.method("triggerEvent", |c| {
        obj::<IntensityConfigObject>(c, 0)?;
        let id = str_arg(c, 1);
        let def = intensity_config().borrow().by_id(id.as_deref()).cloned();
        let Some(def) = def else {
            return Ok(c.push(Value::Nil));
        };
        let mie = match c.arg(2) {
            Value::Nil => return Err(npe("zombie.audio.MusicIntensityEvents.addEvent(String, float, long, boolean)")),
            _ => obj::<MusicIntensityEvents>(c, 2)?,
        };
        let e = down::<MusicIntensityEvents>(&mie).add_event(Some(def.id), def.intensity, def.duration, def.multiple);
        Ok(c.push(Value::Object(e)))
    });
    // Both only act on an IsoPlayer (not ported yet).
    b.method("checkHealthPanelVisible", |c| {
        obj::<IntensityConfigObject>(c, 0)?;
        Ok(0)
    });
    b.method("restoreToFullHealth", |c| {
        obj::<IntensityConfigObject>(c, 0)?;
        Ok(0)
    });

    let b = expose_class(thread, INTENSITY_EVENT_CLASS, None);
    b.ctor(|c| {
        let e = MusicIntensityEvent {
            id: str_arg(c, 0),
            intensity: num_arg(c, 1) as f32,
            duration_ms: num_arg(c, 2) as i64,
            elapsed_ms: Cell::new(0),
        };
        Ok(c.push(Value::Object(Rc::new(e))))
    });
    b.method("getId", |c| {
        let o = obj::<MusicIntensityEvent>(c, 0)?;
        let id = down::<MusicIntensityEvent>(&o).id.clone();
        Ok(c.push(id.map_or(Value::Nil, Value::Str)))
    });
    b.method("getIntensity", |c| {
        let o = obj::<MusicIntensityEvent>(c, 0)?;
        Ok(c.push(down::<MusicIntensityEvent>(&o).intensity as f64))
    });
    b.method("getDuration", |c| {
        let o = obj::<MusicIntensityEvent>(c, 0)?;
        Ok(c.push(down::<MusicIntensityEvent>(&o).duration_ms as f64))
    });
    b.method("getElapsedTime", |c| {
        let o = obj::<MusicIntensityEvent>(c, 0)?;
        Ok(c.push(down::<MusicIntensityEvent>(&o).elapsed_ms.get() as f64))
    });
    b.method("setElapsedTime", |c| {
        let o = obj::<MusicIntensityEvent>(c, 0)?;
        down::<MusicIntensityEvent>(&o).elapsed_ms.set(num_arg(c, 1) as i64);
        Ok(0)
    });

    let b = expose_class(thread, INTENSITY_EVENTS_CLASS, None);
    b.ctor(|c| Ok(c.push(Value::Object(Rc::new(MusicIntensityEvents::new())))));
    b.method("addEvent", |c| {
        let o = obj::<MusicIntensityEvents>(c, 0)?;
        let e = down::<MusicIntensityEvents>(&o).add_event(
            str_arg(c, 1),
            num_arg(c, 2) as f32,
            num_arg(c, 3) as i64,
            matches!(c.arg(4), Value::Bool(true)),
        );
        Ok(c.push(Value::Object(e)))
    });
    b.method("clear", |c| {
        let o = obj::<MusicIntensityEvents>(c, 0)?;
        down::<MusicIntensityEvents>(&o).events.borrow_mut().clear();
        Ok(0)
    });
    b.method("getEventCount", |c| {
        let o = obj::<MusicIntensityEvents>(c, 0)?;
        let n = down::<MusicIntensityEvents>(&o).events.borrow().len();
        Ok(c.push(n as f64))
    });
    b.method("getEventByIndex", |c| {
        let o = obj::<MusicIntensityEvents>(c, 0)?;
        let i = idx_arg(c, 1);
        let events = down::<MusicIntensityEvents>(&o).events.borrow();
        let e = usize::try_from(i)
            .ok()
            .and_then(|i| events.get(i).cloned())
            .ok_or_else(|| out_of_bounds(i, events.len()))?;
        drop(events);
        Ok(c.push(Value::Object(e)))
    });
    b.method("findEventById", |c| {
        let o = obj::<MusicIntensityEvents>(c, 0)?;
        let e = down::<MusicIntensityEvents>(&o).find_event_by_id(str_arg(c, 1).as_deref());
        Ok(push_obj(c, e))
    });
    b.method("update", |c| {
        let o = obj::<MusicIntensityEvents>(c, 0)?;
        down::<MusicIntensityEvents>(&o).update_at(now_ms());
        Ok(0)
    });
    b.method("getIntensity", |c| {
        let o = obj::<MusicIntensityEvents>(c, 0)?;
        Ok(c.push(down::<MusicIntensityEvents>(&o).intensity.get() as f64))
    });

    let b = expose_class(thread, THREAT_CONFIG_CLASS, None);
    b.static_fn("getInstance", |c| {
        let h: Rc<dyn LuaObject> = THREAT_HANDLE.with(|h| h.clone());
        Ok(c.push(Value::Object(h)))
    });
    b.method("initStatuses", |c| {
        obj::<ThreatConfigObject>(c, 0)?;
        let t = table_cast(c.arg(1))?;
        threat_config().borrow_mut().init_statuses(&t)?;
        Ok(0)
    });
    b.method("getStatusCount", |c| {
        obj::<ThreatConfigObject>(c, 0)?;
        Ok(c.push(threat_config().borrow().statuses.len() as f64))
    });
    b.method("getStatusIdByIndex", |c| {
        obj::<ThreatConfigObject>(c, 0)?;
        let i = idx_arg(c, 1);
        let cfg = threat_config();
        let cfg = cfg.borrow();
        let s = usize::try_from(i)
            .ok()
            .and_then(|i| cfg.statuses.get(i))
            .ok_or_else(|| out_of_bounds(i, cfg.statuses.len()))?;
        let id = s.id.clone();
        drop(cfg);
        Ok(c.push(Value::Str(id)))
    });
    b.method("getStatusIntensityByIndex", |c| {
        obj::<ThreatConfigObject>(c, 0)?;
        let i = idx_arg(c, 1);
        let cfg = threat_config();
        let cfg = cfg.borrow();
        let s = usize::try_from(i)
            .ok()
            .and_then(|i| cfg.statuses.get(i))
            .ok_or_else(|| out_of_bounds(i, cfg.statuses.len()))?;
        let v = s.intensity as f64;
        drop(cfg);
        Ok(c.push(v))
    });
    b.method("getStatusIntensity", |c| {
        obj::<ThreatConfigObject>(c, 0)?;
        let v = threat_config().borrow().status_intensity(str_arg(c, 1).as_deref());
        Ok(c.push(v as f64))
    });
    b.method("setStatusIntensityOverride", |c| {
        obj::<ThreatConfigObject>(c, 0)?;
        let id = str_arg(c, 1);
        let v = num_arg(c, 2) as f32;
        let cfg = threat_config();
        let mut cfg = cfg.borrow_mut();
        if let Some(id) = id {
            if let Some(s) = cfg.statuses.iter_mut().find(|s| *s.id == *id) {
                s.intensity_override = if v < 0.0 { f32::NAN } else { v.clamp(0.0, 1.0) };
            }
        }
        Ok(0)
    });
    b.method("getStatusIntensityOverride", |c| {
        obj::<ThreatConfigObject>(c, 0)?;
        let v = threat_config().borrow().status_intensity_override(str_arg(c, 1).as_deref());
        Ok(c.push(v as f64))
    });
    b.method("isStatusIntensityOverridden", |c| {
        obj::<ThreatConfigObject>(c, 0)?;
        let v = threat_config().borrow().is_overridden(str_arg(c, 1).as_deref());
        Ok(c.push(v))
    });

    let b = expose_class(thread, THREAT_STATUS_CLASS, None);
    b.ctor(|c| {
        let s = MusicThreatStatus { id: str_arg(c, 0), intensity: Cell::new(num_arg(c, 1) as f32) };
        Ok(c.push(Value::Object(Rc::new(s))))
    });
    b.method("getId", |c| {
        let o = obj::<MusicThreatStatus>(c, 0)?;
        let id = down::<MusicThreatStatus>(&o).id.clone();
        Ok(c.push(id.map_or(Value::Nil, Value::Str)))
    });
    b.method("getIntensity", |c| {
        let o = obj::<MusicThreatStatus>(c, 0)?;
        Ok(c.push(down::<MusicThreatStatus>(&o).intensity() as f64))
    });
    b.method("setIntensity", |c| {
        let o = obj::<MusicThreatStatus>(c, 0)?;
        down::<MusicThreatStatus>(&o).intensity.set(num_arg(c, 1) as f32);
        Ok(0)
    });

    let b = expose_class(thread, THREAT_STATUSES_CLASS, None);
    // The constructor takes an IsoPlayer; only `update` reads it.
    b.ctor(|c| Ok(c.push(Value::Object(Rc::new(MusicThreatStatuses::default())))));
    b.method("setStatus", |c| {
        let o = obj::<MusicThreatStatuses>(c, 0)?;
        let s = down::<MusicThreatStatuses>(&o).set_status(str_arg(c, 1), num_arg(c, 2) as f32);
        Ok(c.push(Value::Object(s)))
    });
    b.method("clear", |c| {
        let o = obj::<MusicThreatStatuses>(c, 0)?;
        down::<MusicThreatStatuses>(&o).statuses.borrow_mut().clear();
        Ok(0)
    });
    b.method("getStatusCount", |c| {
        let o = obj::<MusicThreatStatuses>(c, 0)?;
        let n = down::<MusicThreatStatuses>(&o).statuses.borrow().len();
        Ok(c.push(n as f64))
    });
    b.method("getStatusByIndex", |c| {
        let o = obj::<MusicThreatStatuses>(c, 0)?;
        let i = idx_arg(c, 1);
        let list = down::<MusicThreatStatuses>(&o).statuses.borrow();
        let s =
            usize::try_from(i).ok().and_then(|i| list.get(i).cloned()).ok_or_else(|| out_of_bounds(i, list.len()))?;
        drop(list);
        Ok(c.push(Value::Object(s)))
    });
    b.method("findStatusById", |c| {
        let o = obj::<MusicThreatStatuses>(c, 0)?;
        let s = down::<MusicThreatStatuses>(&o).find_status_by_id(str_arg(c, 1).as_deref());
        Ok(push_obj(c, s))
    });
    b.method("update", |c| {
        // Reads the player's moodles and stats (IsoPlayer is not ported yet);
        // the intensity is still recomputed from the current statuses.
        let o = obj::<MusicThreatStatuses>(c, 0)?;
        let s = down::<MusicThreatStatuses>(&o);
        s.intensity.set(s.calculate_intensity());
        Ok(0)
    });
    b.method("getIntensity", |c| {
        let o = obj::<MusicThreatStatuses>(c, 0)?;
        Ok(c.push(down::<MusicThreatStatuses>(&o).intensity.get() as f64))
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use pz_kahlua::exec::java_value_of;

    fn run(t: &mut Thread, src: &str) -> Result<Vec<Value>, String> {
        let f = pz_kahlua::stdlib::load_string(src, Some("test"), t.globals().clone()).map_err(|e| e.to_string())?;
        t.pcall(&f, &[]).map_err(|(m, _)| java_value_of(&m))
    }

    fn setup() -> Thread {
        let mut t = Thread::new(TableRef::new());
        pz_kahlua::stdlib::install(&mut t, None).unwrap();
        register(&mut t);
        t
    }

    fn strs(v: &[Value]) -> Vec<String> {
        v.iter().map(java_value_of).collect()
    }

    #[test]
    fn sound_manager_global() {
        let mut t = setup();
        let r = run(
            &mut t,
            r#"
            local sm = getSoundManager()
            local same = sm == getSoundManager()
            local inst = sm:playUISound("UIHighlightMainMenuItem")
            sm:setUiSoundMuted(true)
            local muted = sm:isUiSoundMuted()
            sm:setSoundVolume(0.5)
            sm:setMusicState("MainMenu")
            return same, inst, muted, sm:getSoundVolume(), sm:getAmbientVolume(), sm:isPlayingMusic(),
                type(sm:getAmbientPieces()), sm:PlayWorldSound("x", nil, 0, 0, 0, false)
            "#,
        )
        .unwrap();
        // The default system is silent: nothing plays.
        assert_eq!(strs(&r), ["true", "0.0", "true", "0.5", "1.0", "false", "userdata", "null"]);
    }

    /// Records the engine calls of the character-creation voice preview.
    struct VoiceBackend(Rc<RefCell<Vec<String>>>);

    impl pz_sound::backend::AudioBackend for VoiceBackend {
        fn event_description(&mut self, path: &str) -> Option<Rc<pz_sound::backend::EventDescription>> {
            (path == "event:/VoiceMaleShoutHey").then(|| {
                Rc::new(pz_sound::backend::EventDescription {
                    address: 1,
                    path: path.into(),
                    length_ms: 900,
                    is_3d: false,
                    is_oneshot: true,
                    has_sustain_points: false,
                    max_distance: 20.0,
                    parameters: vec![self.parameter_description("CharacterVoicePitch").unwrap()],
                })
            })
        }
        fn parameter_description(&mut self, name: &str) -> Option<pz_sound::backend::ParameterDescription> {
            (name == "CharacterVoicePitch").then(|| pz_sound::backend::ParameterDescription {
                name: name.into(),
                id: (7, 9),
                minimum: 0.0,
                maximum: 2.0,
                default: 1.0,
                kind: 0,
                flags: 0,
                global_index: 0,
            })
        }
        fn create_event_instance(&mut self, _desc: &pz_sound::backend::EventDescription) -> i64 {
            self.0.borrow_mut().push("create".into());
            42
        }
        fn instance_set_volume(&mut self, _inst: i64, _volume: f32) {}
        fn instance_set_parameter_by_name(&mut self, _inst: i64, _name: &str, _value: f32) {}
        fn instance_set_parameter_by_id(&mut self, inst: i64, id: (u32, u32), value: f32) {
            self.0.borrow_mut().push(format!("param {inst} {id:?} {value}"));
        }
        fn instance_start(&mut self, inst: i64) {
            self.0.borrow_mut().push(format!("start {inst}"));
        }
        fn instance_stop(&mut self, inst: i64, _immediate: bool) {
            self.0.borrow_mut().push(format!("stop {inst}"));
        }
        fn instance_release(&mut self, _inst: i64) {}
        fn instance_playback_state(&mut self, _inst: i64) -> pz_sound::backend::PlaybackState {
            pz_sound::backend::PlaybackState::Playing
        }
        fn listener_3d(&mut self, _i: i32, _p: [f32; 3], _v: [f32; 3], _f: [f32; 3], _u: [f32; 3]) {}
        fn load_sound(&mut self, _path: &str, _looped: bool) -> u64 {
            0
        }
        fn play_sound(&mut self, _sound: u64, _paused: bool) -> u64 {
            0
        }
        fn channel_set_volume(&mut self, _ch: u64, _volume: f32) {}
        fn channel_set_pitch(&mut self, _ch: u64, _pitch: f32) {}
        fn channel_set_mode(&mut self, _ch: u64, _mode: u64) {}
        fn channel_set_paused(&mut self, _ch: u64, _paused: bool) {}
        fn channel_is_playing(&mut self, _ch: u64) -> bool {
            false
        }
        fn channel_stop(&mut self, _ch: u64) {}
        fn master_channel_group_set_volume(&mut self, _volume: f32) {}
    }

    #[test]
    fn ui_emitter_drives_the_voice_preview() {
        let log = Rc::new(RefCell::new(Vec::new()));
        let system = pz_sound::system::SoundSystem::new(
            Box::new(VoiceBackend(log.clone())),
            pz_sound::game_sounds::SoundEnv::default(),
            1,
        );
        install_sound_system(Rc::new(RefCell::new(system)));
        let mut t = setup();
        // CharacterCreationMain's PLAYDEMOVOICE handler.
        let r = run(
            &mut t,
            r#"
            local sm = getSoundManager()
            local player = sm:getUIEmitter()
            local ref = sm:playUISound("VoiceMaleShoutHey")
            player:setPos(0, 0, 0)
            player:setParameterValueByName(ref, "CharacterVoicePitch", 1.25)
            player:tick()
            local playing = player:isPlaying(ref)
            local same = player == sm:getUIEmitter()
            local stopped = player:stopSound(ref)
            return ref, playing, same, player:isPlaying(ref), type(player), sm:getUIEmitter():playSound("Missing")
            "#,
        )
        .unwrap();
        assert_eq!(strs(&r), ["42.0", "true", "true", "false", "userdata", "0.0"]);
        let log = log.borrow();
        assert_eq!(log[..2], ["create", "start 42"], "{log:?}");
        assert!(log.contains(&"param 42 (7, 9) 1.25".to_string()), "{log:?}");
        assert!(log.contains(&"stop 42".to_string()), "{log:?}");
    }

    #[test]
    fn hud_sound_defaults_to_the_shared_sound_manager() {
        crate::hud::host::with_host(|h| h.sound.set_music_state("InGame"));
        assert_eq!(sound_system().borrow().manager.music_state(), pz_sound::sound_manager::MusicState::InGame);
        crate::hud::host::with_host(|h| {
            h.sound.pause_sound_and_music(true);
            h.sound.resume_sound_and_music();
            h.sound.play_ui_sound("UIPauseMenuEnter");
        });
    }

    #[test]
    fn intensity_config_and_events() {
        let mut t = setup();
        let r = run(
            &mut t,
            r#"
            local cfg = MusicIntensityConfig.getInstance()
            cfg:initEvents({ VERSION = 1,
                { id = "A", intensity = 10, duration = 1000 },
                { id = "B", intensity = -70, duration = 0, multiple = false },
                { id = "  ", intensity = 5 },
                { id = "A", intensity = 20, duration = 500 } })
            local mie = MusicIntensityEvents.new()
            local a = cfg:triggerEvent("A", mie)
            local b1 = cfg:triggerEvent("B", mie)
            local b2 = cfg:triggerEvent("B", mie)
            return cfg == MusicIntensityConfig.getInstance(), mie:getEventCount(), a:getIntensity(),
                a:getDuration(), b1 == b2, cfg:triggerEvent("missing", mie), mie:findEventById("a"):getId()
            "#,
        )
        .unwrap();
        assert_eq!(strs(&r), ["true", "2.0", "20.0", "500.0", "true", "null", "A"]);
        let err = run(&mut t, "MusicIntensityConfig.getInstance():initEvents({ { id = 5 } })").unwrap_err();
        assert!(err.contains("ClassCastException") || err.contains("cannot be cast"), "{err}");
    }

    #[test]
    fn intensity_update_expires_and_clamps() {
        let ev = MusicIntensityEvents::new();
        ev.add_event(Some("x".into()), 80.0, 100, true);
        ev.add_event(Some("y".into()), 5.0, 0, true);
        ev.update_at(1_000);
        // First update: elapsed is huge (from -1), "x" expires; "y" gets elapsed time.
        assert_eq!(ev.events.borrow().len(), 1);
        assert_eq!(ev.intensity.get(), 55.0);
        ev.update_at(1_010);
        // A zero-duration event lives for exactly one update.
        assert_eq!(ev.events.borrow().len(), 0);
        assert_eq!(ev.intensity.get(), 50.0);
    }

    #[test]
    fn threat_config_and_statuses() {
        let mut t = setup();
        let r = run(
            &mut t,
            r#"
            local cfg = MusicThreatConfig.getInstance()
            cfg:initStatuses({ VERSION = 2,
                { id = "Panic", intensity = 0.5 },
                { id = "Zero", intensity = 0 },
                { id = "Health", intensity = 2 } })
            local s = MusicThreatStatuses.new(nil)
            s:setStatus("Panic", 1.0)
            s:setStatus("health", 0.25)
            s:update()
            local before = s:getIntensity()
            cfg:setStatusIntensityOverride("Panic", 3)
            local over = cfg:getStatusIntensityOverride("Panic")
            s:update()
            local after = s:getIntensity()
            cfg:setStatusIntensityOverride("Panic", -1)
            return cfg:getStatusCount(), cfg:getStatusIdByIndex(1), before, over, after,
                cfg:isStatusIntensityOverridden("Panic"), cfg:getStatusIntensity("Zero")
            "#,
        )
        .unwrap();
        // "health" does not match "Health" in the config's HashMap, so only
        // Panic counts; the override (clamped to 1) replaces its 1.0.
        assert_eq!(strs(&r), ["2.0", "Health", "0.5", "1.0", "0.5", "false", "0.0"]);
    }
}
