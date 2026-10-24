//! `zombie.core.Core` exposed to Lua, plus the `getCore()` global
//! (`LuaManager.GlobalObject.getCore`).
//!
//! Every public `Core` method is callable under its Java name with Kahlua's
//! argument conversions (`int` = `Double.intValue`, `float` = narrowing,
//! `boolean` only from `true`, Java `null` = nil) and return conversions
//! (numbers become Lua numbers, `float` widened to double, `null` = nil).
//! Overloads are resolved on argument count/type like Kahlua's
//! `MultiLuaJavaInvoker`.
//!
//! # Providing the Core
//!
//! The binding works on a shared `Rc<RefCell<Core>>`. `register_all` (which
//! takes no `Core`) uses [`shared_core`], a thread-local singleton standing
//! in for `Core.getInstance()`: call [`install_core`] with the engine's own
//! instance *before* `LuaManager::init` to share it, or call
//! [`shared_core`] afterwards to get the one that was created. Code that
//! builds a Lua state by hand can call [`register`] directly with any core.
//!
//! # Events
//!
//! Lua events Java `Core` triggers (`OnResolutionChange`,
//! `OptionControllerButtonStyleChanged`, `OptionGamepadBindingPresetChanged`)
//! are queued on the `Core` ([`Core::take_events`]) and triggered after each
//! Lua call into Core. Engine-side requests (`quit`, `ResetLua`, display
//! mode, vsync, screenshots) stay queued in [`Core::take_requests`] for the
//! game loop.
//!
//! # Helper classes
//!
//! Classes Core's methods return that are not ported elsewhere yet get a
//! minimal `LuaObject` here, registered only if no other binding has
//! registered that class first: `zombie.core.textures.ColorInfo`
//! ([`LuaColorInfo`]), `zombie.core.GameVersion`, `zombie.core.Core$KeyBinding`,
//! `zombie.input.KeybindId` (constants for `getCore():getKey(KeybindId.X)`),
//! `java.util.ArrayList` (read-only, for `getDefaultZoomLevels`) and a live
//! view of `zombie.config.ConfigOption` (for `getOptionByIndex`).

use crate::events::EventManager;
use crate::expose::{bool_arg, expose_class, global, int_arg, num_arg, str_arg, ClassBuilder};
use pz_core::core::{
    ColorInfo, Core, EventArg, GameVersion, KeyBinding, SimpleAccessor, SIMPLE_ACCESSORS,
};
use pz_kahlua::value::LuaObject;
use pz_kahlua::{LuaError, LuaResult, NativeCall, TableRef, Thread, Value};
use std::any::Any;
use std::cell::{Cell, Ref, RefCell, RefMut};
use std::collections::HashMap;
use std::rc::Rc;

pub const CORE_CLASS: &str = "zombie.core.Core";
pub const COLOR_INFO_CLASS: &str = "zombie.core.textures.ColorInfo";
pub const GAME_VERSION_CLASS: &str = "zombie.core.GameVersion";
pub const KEY_BINDING_CLASS: &str = "zombie.core.Core$KeyBinding";
pub const KEYBIND_ID_CLASS: &str = "zombie.input.KeybindId";
pub const ARRAY_LIST_CLASS: &str = "java.util.ArrayList";
pub const CONFIG_OPTION_CLASS: &str = "zombie.config.ConfigOption";

thread_local! {
    static SHARED: RefCell<Option<Rc<RefCell<Core>>>> = const { RefCell::new(None) };
}

/// Make `core` the instance `register_all` binds (call before
/// `LuaManager::init`).
pub fn install_core(core: Rc<RefCell<Core>>) {
    SHARED.with(|s| *s.borrow_mut() = Some(core));
}

/// The shared Core (`Core.getInstance()`), created with defaults on first
/// use.
pub fn shared_core() -> Rc<RefCell<Core>> {
    SHARED.with(|s| {
        s.borrow_mut()
            .get_or_insert_with(|| Rc::new(RefCell::new(Core::new())))
            .clone()
    })
}

/// The Lua object for the Core singleton.
pub struct CoreObject {
    pub core: Rc<RefCell<Core>>,
}

impl LuaObject for CoreObject {
    fn class_name(&self) -> &str {
        CORE_CLASS
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// `zombie.core.textures.ColorInfo`.
pub struct LuaColorInfo {
    pub c: Cell<ColorInfo>,
}

impl LuaColorInfo {
    pub fn new(c: ColorInfo) -> Rc<Self> {
        Rc::new(LuaColorInfo { c: Cell::new(c) })
    }
}

impl LuaObject for LuaColorInfo {
    fn class_name(&self) -> &str {
        COLOR_INFO_CLASS
    }
    fn to_lua_string(&self) -> String {
        self.c.get().to_string()
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// `zombie.core.GameVersion`.
pub struct LuaGameVersion(pub GameVersion);

impl LuaObject for LuaGameVersion {
    fn class_name(&self) -> &str {
        GAME_VERSION_CLASS
    }
    fn to_lua_string(&self) -> String {
        self.0.to_string()
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// `Core.KeyBinding` record.
pub struct LuaKeyBinding(pub KeyBinding);

impl LuaObject for LuaKeyBinding {
    fn class_name(&self) -> &str {
        KEY_BINDING_CLASS
    }
    fn to_lua_string(&self) -> String {
        self.0.to_string()
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn super_classes(&self) -> &[&'static str] {
        &["java.lang.Record"]
    }
}

/// `zombie.input.KeybindId`; `toString()` is the id.
pub struct KeybindId {
    pub id: String,
}

impl LuaObject for KeybindId {
    fn class_name(&self) -> &str {
        KEYBIND_ID_CLASS
    }
    fn to_lua_string(&self) -> String {
        self.id.clone()
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// The base-game `KeybindId` constants (field name, id).
pub const KEYBIND_IDS: &[(&str, &str)] = &[
    ("FORWARD", "Forward"),
    ("BACKWARD", "Backward"),
    ("LEFT", "Left"),
    ("RIGHT", "Right"),
    ("RUN", "Run"),
    ("INTERACT", "Interact"),
    ("SPRINT", "Sprint"),
    ("ROTATE_BUILDING", "Rotate building"),
    ("TOGGLE_MODE", "Toggle mode"),
    ("TOGGLE_SEARCH_MODE", "Toggle Search Mode"),
    ("CANCEL_ACTION", "CancelAction"),
    ("WALK_TO", "WalkTo"),
    ("RELEASE_ROPE", "ReleaseRope"),
    ("SIT_ON_GROUND", "SitOnGround"),
    ("ATTACK_CLICK", "Attack/Click"),
    ("CROUCH", "Crouch"),
    ("AIM", "Aim"),
    ("MELEE", "Melee"),
    ("RACK_FIREARM", "Rack Firearm"),
    ("RELOAD_WEAPON", "ReloadWeapon"),
    ("SHARPEN_WEAPON", "SharpenWeapon"),
    ("MANUAL_FLOOR_ATTACK", "ManualFloorAtk"),
    ("START_VEHICLE_ENGINE", "StartVehicleEngine"),
    ("BRAKE", "Brake"),
    ("CRUISE_CONTROL", "CruiseControl"),
    ("TOGGLE_VEHICLE_HEADLIGHTS", "ToggleVehicleHeadlights"),
    ("VEHICLE_HEATER", "VehicleHeater"),
    ("VEHICLE_MECHANICS", "VehicleMechanics"),
    ("VEHICLE_HORN", "VehicleHorn"),
    ("VEHICLE_RADIAL_MENU", "VehicleRadialMenu"),
    ("VEHICLE_SWITCH_SEAT", "VehicleSwitchSeat"),
    ("TOGGLE_UI", "Toggle UI"),
    ("CRAFTING_UI", "Crafting UI"),
    ("BUILDING_UI", "Building UI"),
    ("MAIN_MENU", "Main Menu"),
    ("TOGGLE_INVENTORY", "Toggle Inventory"),
    ("TOGGLE_SKILL_PANEL", "Toggle Skill Panel"),
    ("TOGGLE_HEALTH_PANEL", "Toggle Health Panel"),
    ("TOGGLE_INFO_PANEL", "Toggle Info Panel"),
    (
        "TOGGLE_CLOTHING_PROTECTION_PANEL",
        "Toggle Clothing Protection Panel",
    ),
    ("TOGGLE_MOVABLE_PANEL_MODE", "Toggle Moveable Panel Mode"),
    ("MAP", "Map"),
    ("ANIMAL_RADIAL_MENU", "AnimalRadialMenu"),
    ("TAKE_SCREENSHOT", "Take screenshot"),
    ("TOGGLE_SURVIVAL_GUIDE", "Toggle Survival Guide"),
    ("DISPLAY_FPS", "Display FPS"),
    ("PAUSE", "Pause"),
    ("NORMAL_SPEED", "Normal Speed"),
    ("FAST_FORWARD_X1", "Fast Forward x1"),
    ("FAST_FORWARD_X2", "Fast Forward x2"),
    ("FAST_FORWARD_X3", "Fast Forward x3"),
    ("PAN_CAMERA", "PanCamera"),
    ("ZOOM_IN", "Zoom in"),
    ("ZOOM_OUT", "Zoom out"),
    ("HOTBAR_1", "Hotbar 1"),
    ("HOTBAR_2", "Hotbar 2"),
    ("HOTBAR_3", "Hotbar 3"),
    ("HOTBAR_4", "Hotbar 4"),
    ("HOTBAR_5", "Hotbar 5"),
    ("HOTBAR_6", "Hotbar 6"),
    ("HOTBAR_7", "Hotbar 7"),
    ("HOTBAR_8", "Hotbar 8"),
    ("LIGHT_SOURCE", "Equip/Turn On/Off Light Source"),
    ("DROP_BOTH_HELD_ITEMS", "DropBothHeldItems"),
    ("DROP_PRIMARY_HELD_ITEM", "DropPrimaryHeldItem"),
    ("DROP_SECONDARY_HELD_ITEM", "DropSecondaryHeldItem"),
    ("DROP_WORN_BAG", "DropWornBag"),
    (
        "DROP_BOTH_HELD_ITEMS_AND_WORN_BAG",
        "DropBothHeldItemsAndWornBag",
    ),
    ("GRAB_CORPSE", "GrabCorpse"),
    ("TOGGLE_SAFETY", "Toggle Safety"),
    ("TOGGLE_CHAT", "Toggle chat"),
    ("ALT_TOGGLE_CHAT", "Alt toggle chat"),
    ("SWITCH_CHAT_STREAM", "Switch chat stream"),
    ("ENABLE_VOICE_TRANSMIT", "Enable voice transmit"),
    ("SHOUT", "Shout"),
    ("EMOTE", "Emote"),
    ("TOGGLE_MUSIC", "Toggle Music"),
    ("TOGGLE_LUA_DEBUGGER", "Toggle Lua Debugger"),
    ("TOGGLE_LUA_CONSOLE", "ToggleLuaConsole"),
    ("TOGGLE_GOD_MODE", "ToggleGodModeInvisible"),
    ("TOGGLE_MODELS_ENABLED", "ToggleModelsEnabled"),
    ("TOGGLE_ANIMATION_TEXT", "ToggleAnimationText"),
    ("TOGGLE_OLD_RENDERER", "ToggleOldRenderer"),
];

/// A read-only `java.util.ArrayList`.
pub struct JavaList(pub Vec<Value>);

impl LuaObject for JavaList {
    fn class_name(&self) -> &str {
        ARRAY_LIST_CLASS
    }
    fn to_lua_string(&self) -> String {
        let parts: Vec<String> = self.0.iter().map(pz_kahlua::exec::java_value_of).collect();
        format!("[{}]", parts.join(", "))
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// Live view of one of Core's options (`getOptionByIndex`).
pub struct LuaConfigOption {
    pub core: Rc<RefCell<Core>>,
    pub index: usize,
    class: &'static str,
}

impl LuaObject for LuaConfigOption {
    fn class_name(&self) -> &str {
        self.class
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn super_classes(&self) -> &[&'static str] {
        &[CONFIG_OPTION_CLASS]
    }
}

/// An opaque Lua value stored in Core (`setAccountUsed`,
/// `setOptionOnStartup`).
struct StoredValue(Value);

/// Return value of a bound method.
pub enum Ret {
    Void,
    Val(Value),
}

impl From<bool> for Ret {
    fn from(v: bool) -> Self {
        Ret::Val(Value::Bool(v))
    }
}
impl From<i32> for Ret {
    fn from(v: i32) -> Self {
        Ret::Val(Value::Num(v as f64))
    }
}
impl From<f32> for Ret {
    fn from(v: f32) -> Self {
        Ret::Val(Value::Num(v as f64))
    }
}
impl From<f64> for Ret {
    fn from(v: f64) -> Self {
        Ret::Val(Value::Num(v))
    }
}
impl From<String> for Ret {
    fn from(v: String) -> Self {
        Ret::Val(Value::from(v))
    }
}
impl From<Option<String>> for Ret {
    fn from(v: Option<String>) -> Self {
        Ret::Val(v.map(Value::from).unwrap_or(Value::Nil))
    }
}
impl From<Value> for Ret {
    fn from(v: Value) -> Self {
        Ret::Val(v)
    }
}

/// Shared state of the binding.
pub struct Ctx {
    pub core: Rc<RefCell<Core>>,
    pub events: Option<EventManager>,
    pub object: Value,
    /// One `ColorInfo` object per Core colour, like the Java fields.
    colors: RefCell<HashMap<&'static str, Rc<LuaColorInfo>>>,
}

impl Ctx {
    fn core(&self) -> Ref<'_, Core> {
        self.core.borrow()
    }
    fn core_mut(&self) -> RefMut<'_, Core> {
        self.core.borrow_mut()
    }
    /// Trigger the Lua events Core queued.
    fn flush(&self, thread: &mut Thread) {
        let events = self.core.borrow_mut().take_events();
        let Some(em) = &self.events else { return };
        for e in events {
            let args: Vec<Value> = e
                .args
                .into_iter()
                .map(|a| match a {
                    EventArg::Num(n) => Value::Num(n),
                    EventArg::Str(s) => Value::from(s),
                })
                .collect();
            em.trigger(thread, e.name, &args);
        }
    }
    fn color(&self, slot: &'static str, c: ColorInfo) -> Ret {
        let obj = self
            .colors
            .borrow_mut()
            .entry(slot)
            .or_insert_with(|| LuaColorInfo::new(c))
            .clone();
        obj.c.set(c);
        Ret::Val(Value::Object(obj))
    }
}

fn def(
    b: &ClassBuilder,
    cx: &Rc<Ctx>,
    name: &str,
    f: impl Fn(&Ctx, &mut NativeCall<'_>) -> LuaResult<Ret> + 'static,
) {
    let cx = cx.clone();
    b.method(name, move |c| {
        let r = f(&cx, c)?;
        cx.flush(c.thread);
        Ok(match r {
            Ret::Void => 0,
            Ret::Val(v) => c.push(v),
        })
    });
}

fn sdef(
    b: &ClassBuilder,
    cx: &Rc<Ctx>,
    name: &str,
    f: impl Fn(&Ctx, &mut NativeCall<'_>) -> LuaResult<Ret> + 'static,
) {
    let cx = cx.clone();
    b.static_fn(name, move |c| {
        let r = f(&cx, c)?;
        cx.flush(c.thread);
        Ok(match r {
            Ret::Void => 0,
            Ret::Val(v) => c.push(v),
        })
    });
}

fn java_error(message: impl Into<String>, class: &'static str) -> LuaError {
    LuaError::Runtime {
        message: message.into(),
        class,
    }
}

fn index_error(i: i32, size: usize) -> LuaError {
    java_error(
        format!("Index {i} out of bounds for length {size}"),
        "java.lang.IndexOutOfBoundsException",
    )
}

fn io_error(e: std::io::Error) -> LuaError {
    java_error(e.to_string(), "java.io.IOException")
}

fn opt_str(c: &NativeCall<'_>, i: usize) -> Option<String> {
    str_arg(c, i).map(|s| s.to_string())
}

fn float_arg(c: &NativeCall<'_>, i: usize) -> f32 {
    num_arg(c, i) as f32
}

/// A key name argument: a string or a `KeybindId` (whose `getId()` is its
/// `toString()`); `None` for Java `null`.
fn key_name_arg(c: &NativeCall<'_>, i: usize) -> Option<String> {
    match c.arg(i) {
        Value::Object(o) if o.class_name() == KEYBIND_ID_CLASS => Some(o.to_lua_string()),
        _ => opt_str(c, i),
    }
}

fn key_binding_for(core: &Core, name: Option<String>) -> KeyBinding {
    match name {
        Some(n) => core.get_key_binding(&n),
        None => KeyBinding::empty(),
    }
}

fn color_arg(c: &NativeCall<'_>, i: usize) -> LuaResult<ColorInfo> {
    match c.arg(i) {
        Value::Object(o) => match o.as_any().downcast_ref::<LuaColorInfo>() {
            Some(ci) => Ok(ci.c.get()),
            None => Err(java_error(
                format!(
                    "class {} cannot be cast to class {COLOR_INFO_CLASS}",
                    o.class_name()
                ),
                "java.lang.ClassCastException",
            )),
        },
        Value::Nil => Err(java_error(
            "Cannot read field \"r\" because \"colorInfo\" is null",
            "java.lang.NullPointerException",
        )),
        other => Err(java_error(
            format!(
                "{} cannot be cast to {COLOR_INFO_CLASS}",
                pz_kahlua::stdlib::java_class_name(&other)
            ),
            "java.lang.ClassCastException",
        )),
    }
}

fn store(v: Value) -> Option<Rc<dyn Any>> {
    if v.is_nil() {
        None
    } else {
        Some(Rc::new(StoredValue(v)))
    }
}

fn load(v: Option<Rc<dyn Any>>) -> Value {
    v.and_then(|a| a.downcast_ref::<StoredValue>().map(|s| s.0.clone()))
        .unwrap_or(Value::Nil)
}

fn class_free(thread: &Thread, fqn: &str) -> bool {
    thread.class_metatables.get_str(fqn).is_nil()
}

/// Register `zombie.core.Core`, its helper classes and the `getCore()`
/// global on `thread`, bound to `core`. `events` receives the Lua events
/// Core triggers (pass the `LuaManager`'s `events`).
pub fn register(
    thread: &mut Thread,
    events: Option<EventManager>,
    core: Rc<RefCell<Core>>,
) -> Rc<Ctx> {
    let object = Value::Object(Rc::new(CoreObject { core: core.clone() }));
    let cx = Rc::new(Ctx {
        core,
        events,
        object: object.clone(),
        colors: RefCell::new(HashMap::new()),
    });
    register_helpers(thread, &cx);
    let b = expose_class(thread, CORE_CLASS, None);
    register_statics(&b, &cx);
    register_simple(&b, &cx);
    register_methods(&b, &cx);
    let obj = object;
    global(thread, "getCore", move |c| Ok(c.push(obj.clone())));
    cx
}

fn register_simple(b: &ClassBuilder, cx: &Rc<Ctx>) {
    for acc in SIMPLE_ACCESSORS {
        let name = acc.java_name();
        match *acc {
            SimpleAccessor::GetBool(_, f) => {
                def(b, cx, name, move |cx, _| Ok(f(&cx.core()).into()))
            }
            SimpleAccessor::GetInt(_, f) => def(b, cx, name, move |cx, _| Ok(f(&cx.core()).into())),
            SimpleAccessor::GetDouble(_, f) => {
                def(b, cx, name, move |cx, _| Ok(f(&cx.core()).into()))
            }
            SimpleAccessor::GetFloat(_, f) => {
                def(b, cx, name, move |cx, _| Ok(f(&cx.core()).into()))
            }
            SimpleAccessor::GetString(_, f) => {
                def(b, cx, name, move |cx, _| Ok(f(&cx.core()).into()))
            }
            SimpleAccessor::SetBool(_, f) => def(b, cx, name, move |cx, c| {
                f(&mut cx.core_mut(), bool_arg(c, 1));
                Ok(Ret::Void)
            }),
            SimpleAccessor::SetInt(_, f) => def(b, cx, name, move |cx, c| {
                f(&mut cx.core_mut(), int_arg(c, 1));
                Ok(Ret::Void)
            }),
            SimpleAccessor::SetDouble(_, f) => def(b, cx, name, move |cx, c| {
                f(&mut cx.core_mut(), num_arg(c, 1));
                Ok(Ret::Void)
            }),
            SimpleAccessor::SetFloat(_, f) => def(b, cx, name, move |cx, c| {
                f(&mut cx.core_mut(), float_arg(c, 1));
                Ok(Ret::Void)
            }),
            SimpleAccessor::SetString(_, f) => def(b, cx, name, move |cx, c| {
                let s = opt_str(c, 1);
                f(&mut cx.core_mut(), s.as_deref());
                Ok(Ret::Void)
            }),
        }
    }
}

/// Public static fields (copied at exposure time, like Kahlua) and static
/// methods.
fn register_statics(b: &ClassBuilder, cx: &Rc<Ctx>) {
    {
        let core = cx.core();
        b.field("IS_DEV", core.is_dev);
        b.field("PZWorldToBulletZScale", 2.44949f32 as f64);
        b.field("characterHeight", 0.6f32 as f64);
        b.field("characterRangedAimPointHeight", 0.468f32 as f64);
        b.field("characterMeleeAimPointHeight", 0.57f32 as f64);
        b.field("bDemo", false);
        b.field("tutorial", core.tutorial);
        b.field("dirtyGlobalLightsCount", core.dirty_global_lights_count);
        b.field("altMoveMethod", core.alt_move_method);
        b.field("optionModsEnabled", core.option_mods_enabled);
        b.field("tileScale", core.tile_scale);
        b.field("blinkAlpha", core.blink_alpha as f64);
        b.field("blinkAlphaIncrease", core.blink_alpha_increase);
        b.field("width", core.width);
        b.field("height", core.height);
        b.field("initialWidth", core.initial_width as f64);
        b.field("initialHeight", core.initial_height as f64);
        b.field("maxJukeBoxesActive", core.max_juke_boxes_active);
        b.field("numJukeBoxesActive", core.num_juke_boxes_active);
        b.field("gameMode", core.game_mode.as_str());
        b.field("addZombieOnCellLoad", core.add_zombie_on_cell_load);
        b.field("preset", core.preset.as_str());
        // `debug` is @HiddenFromLua.
        b.field("antiCheats", core.anti_cheats);
        b.field("useViewports", core.use_viewports);
        b.field("useGameViewport", core.use_game_viewport);
        b.field("imGui", core.im_gui);
        b.field("iPerfSkybox_High", 0);
        b.field("iPerfSkybox_Medium", 1);
        b.field("iPerfSkybox_Static", 2);
        b.field("iPerfPuddles_None", 3);
        b.field("iPerfPuddles_GroundOnly", 2);
        b.field("iPerfPuddles_GroundWithRuts", 1);
        b.field("iPerfPuddles_All", 0);
        b.field("gameMap", core.game_map.as_str());
        b.field("gameSaveWorld", core.game_save_world.as_str());
        b.field("safeMode", core.safe_mode);
        b.field("safeModeForced", core.safe_mode_forced);
        b.field("soundDisabled", core.sound_disabled);
        b.field("xx", core.xx);
        b.field("yy", core.yy);
        b.field("zz", core.zz);
        b.field("ModelScale", 1.5);
        b.field("scale", ((2f64.sqrt() / 2.0 / 10.0) as f32 / 1.5f32) as f64);
        b.field("lastStand", core.last_stand);
        b.field("exiting", core.exiting);
        b.field(
            "KEYBINDING_EMPTY",
            Value::Object(Rc::new(LuaKeyBinding(KeyBinding::empty()))),
        );
    }
    sdef(b, cx, "getInstance", |cx, _| Ok(cx.object.clone().into()));
    sdef(b, cx, "getTileScale", |cx, _| {
        Ok(cx.core().get_tile_scale().into())
    });
    sdef(b, cx, "isLastStand", |cx, _| {
        Ok(cx.core().is_last_stand().into())
    });
    sdef(b, cx, "isDevMode", |cx, _| {
        Ok(cx.core().is_dev_mode().into())
    });
    sdef(b, cx, "getMyDocumentFolder", |cx, _| {
        Ok(cx.core().get_my_document_folder().into())
    });
    sdef(b, cx, "UnfocusActiveTextEntryBox", |cx, _| {
        cx.core_mut().unfocus_active_text_entry_box();
        Ok(Ret::Void)
    });
    sdef(b, cx, "setFullScreen", |cx, c| {
        cx.core_mut().set_full_screen(bool_arg(c, 0));
        Ok(Ret::Void)
    });
    sdef(b, cx, "isUseGameViewport", |cx, _| {
        Ok(cx.core().is_use_game_viewport().into())
    });
    sdef(b, cx, "isImGui", |cx, _| Ok(cx.core().is_im_gui().into()));
    sdef(b, cx, "isUseViewports", |cx, _| {
        Ok(cx.core().is_use_viewports().into())
    });
    sdef(b, cx, "getGLMajorVersion", |cx, _| {
        Ok(cx.core().get_gl_major_version().into())
    });
    sdef(b, cx, "getGLVersion", |cx, _| {
        Ok(cx.core().get_gl_version().into())
    });
    // TODO(render): queried from GL by the renderer (`Core::set_open_gl_version`).
    sdef(b, cx, "getOpenGLVersions", |_, _| Ok(Ret::Void));
    // `flipPixels(int[], w, h)`: Lua cannot build a Java int[]; nil like a null input.
    sdef(b, cx, "flipPixels", |_, _| Ok(Value::Nil.into()));
    sdef(b, cx, "supportNPTTexture", |_, _| Ok(false.into()));
    sdef(b, cx, "setDisplayMode", |cx, c| {
        cx.core_mut()
            .set_display_mode(int_arg(c, 0), int_arg(c, 1), bool_arg(c, 2));
        Ok(Ret::Void)
    });
    sdef(b, cx, "setInitialSize", |cx, _| {
        cx.core_mut().set_initial_size();
        Ok(Ret::Void)
    });
    sdef(b, cx, "supportCompressedTextures", |cx, _| {
        Ok(cx.core().support_compressed_textures().into())
    });
    sdef(b, cx, "getGitRevisionString", |cx, _| {
        Ok(cx.core().get_git_revision_string().into())
    });
}

/// Methods that are not a single option `getValue()` / `setValue()`.
fn register_methods(b: &ClassBuilder, cx: &Rc<Ctx>) {
    macro_rules! get {
        ($name:literal, $f:ident) => {
            def(b, cx, $name, |cx, _| Ok(cx.core().$f().into()))
        };
    }
    macro_rules! get_mut {
        ($name:literal, $f:ident) => {
            def(b, cx, $name, |cx, _| Ok(cx.core_mut().$f().into()))
        };
    }
    macro_rules! void {
        ($name:literal, $f:ident) => {
            def(b, cx, $name, |cx, _| {
                cx.core_mut().$f();
                Ok(Ret::Void)
            })
        };
    }
    macro_rules! set_bool {
        ($name:literal, $f:ident) => {
            def(b, cx, $name, |cx, c| {
                cx.core_mut().$f(bool_arg(c, 1));
                Ok(Ret::Void)
            })
        };
    }
    macro_rules! set_int {
        ($name:literal, $f:ident) => {
            def(b, cx, $name, |cx, c| {
                cx.core_mut().$f(int_arg(c, 1));
                Ok(Ret::Void)
            })
        };
    }
    macro_rules! set_str {
        ($name:literal, $f:ident) => {
            def(b, cx, $name, |cx, c| {
                let s = opt_str(c, 1);
                cx.core_mut().$f(s.as_deref());
                Ok(Ret::Void)
            })
        };
    }
    macro_rules! color {
        ($get:literal, $set:literal, $slot:literal, $gf:ident, $sf:ident) => {
            def(b, cx, $get, |cx, _| {
                let col = cx.core_mut().$gf();
                Ok(cx.color($slot, col))
            });
            def(b, cx, $set, |cx, c| {
                let col = color_arg(c, 1)?;
                cx.core_mut().$sf(col);
                Ok(Ret::Void)
            });
        };
    }

    // ---- options list ----
    get!("getOptionCount", get_option_count);
    def(b, cx, "getOptionByIndex", |cx, c| {
        let i = int_arg(c, 1);
        let core = cx.core();
        let n = core.options().len();
        let Some(o) = usize::try_from(i)
            .ok()
            .and_then(|i| core.get_option_by_index(i))
        else {
            return Err(index_error(i, n));
        };
        let class = config_option_class(o.get_type());
        Ok(Value::Object(Rc::new(LuaConfigOption {
            core: cx.core.clone(),
            index: i as usize,
            class,
        }))
        .into())
    });
    get!("isMultiThread", is_multi_thread);
    set_bool!("setMultiThread", set_multi_thread);
    set_bool!("setChallenge", set_challenge);
    get!("isChallenge", is_challenge);
    get!("getChallengeID", get_challenge_id);
    set_bool!(
        "setOptionTieredZombieUpdates",
        set_option_tiered_zombie_updates
    );
    set_int!("setFramerate", set_framerate);
    get!("loadedShader", loaded_shader);
    get!("getUseShaders", get_use_shaders);
    set_bool!("setUseShaders", set_use_shaders);
    get!("getPerfSkyboxOnLoad", get_perf_skybox_on_load);
    get!("getPerfReflectionsOnLoad", get_perf_reflections_on_load);
    get!("getPerfPuddlesOnLoad", get_perf_puddles_on_load);
    get!("getUseOpenGL21", get_use_open_gl21);
    get!("getVidMem", get_vid_mem);
    set_int!("setVidMem", set_vid_mem);
    void!("shadersOptionChanged", shaders_option_changed);
    void!("initGlobalShader", init_global_shader);
    void!("initShaders", init_shaders);
    get!("getGameMode", get_game_mode);
    def(b, cx, "setGameMode", |cx, c| {
        let Some(m) = opt_str(c, 1) else {
            return Err(java_error(
                "gameMode is null",
                "java.lang.NullPointerException",
            ));
        };
        cx.core_mut().set_game_mode(&m);
        Ok(Ret::Void)
    });
    get!("getDebug", get_debug);

    // ---- screenshots / frames (TODO(render)) ----
    def(b, cx, "TakeScreenshot", |cx, c| {
        let mut core = cx.core_mut();
        match c.nargs() {
            0 | 1 => core.take_screenshot(),
            2..=4 => core.take_screenshot_sized(int_arg(c, 1), int_arg(c, 2), int_arg(c, 3)),
            _ => core.take_screenshot_at(
                int_arg(c, 1),
                int_arg(c, 2),
                int_arg(c, 3),
                int_arg(c, 4),
                int_arg(c, 5),
            ),
        }
        Ok(Ret::Void)
    });
    def(b, cx, "TakeFullScreenshot", |cx, c| {
        let f = opt_str(c, 1);
        cx.core_mut()
            .take_full_screenshot(f.as_deref())
            .map_err(|m| java_error(m, "java.lang.IllegalArgumentException"))?;
        Ok(Ret::Void)
    });
    get_mut!("supportsFBO", supports_fbo);
    void!("MoveMethodToggle", move_method_toggle);
    set_int!("EndFrameText", end_frame_text);
    def(b, cx, "EndFrame", |cx, c| {
        if c.nargs() >= 2 {
            cx.core_mut().end_frame_player(int_arg(c, 1));
        } else {
            cx.core_mut().end_frame();
        }
        Ok(Ret::Void)
    });
    // TODO(render): GameTime.getThirtyFPSMultiplier() (1.0 at 30 FPS).
    def(b, cx, "EndFrameUI", |cx, _| {
        cx.core_mut().end_frame_ui(1.0);
        Ok(Ret::Void)
    });
    def(b, cx, "StartFrame", |cx, c| {
        if c.nargs() >= 3 {
            cx.core_mut()
                .start_frame_player(int_arg(c, 1), bool_arg(c, 2));
        } else {
            cx.core_mut().start_frame();
        }
        Ok(Ret::Void)
    });
    set_int!("StartFrameText", start_frame_text);
    def(b, cx, "StartFrameUI", |cx, _| {
        Ok(cx.core_mut().start_frame_ui(1.0).into())
    });
    for name in [
        "RenderOffScreenBuffer",
        "DoStartFrameStuff",
        "DoEndFrameStuffFx",
        "DoStartFrameStuffSmartTextureFx",
        "ChangeWorldViewport",
        "StartFrameFlipY",
        "DoStartFrameNoZoom",
        "DoPushIsoStuff",
        "DoPushIsoStuff2D",
        "DoPushIsoParticleStuff",
        "DoPopIsoStuff",
        "DoEndFrameStuff",
        "setLastRenderedFBO",
    ] {
        // TODO(render): GL state for the iso/world passes.
        def(b, cx, name, |_, _| Ok(Ret::Void));
    }
    // TODO(render): no TextureFBO objects yet.
    def(b, cx, "getOffscreenBuffer", |_, _| Ok(Value::Nil.into()));
    def(b, cx, "getOffscreenWidth", |cx, c| {
        Ok(cx.core().get_offscreen_width(int_arg(c, 1)).into())
    });
    def(b, cx, "getOffscreenHeight", |cx, c| {
        Ok(cx.core().get_offscreen_height(int_arg(c, 1)).into())
    });
    get!("getOffscreenTrueWidth", get_offscreen_true_width);
    get!("getOffscreenTrueHeight", get_offscreen_true_height);
    get!("getScreenHeight", get_screen_height);
    get!("getScreenWidth", get_screen_width);
    def(b, cx, "setResolutionAndFullScreen", |cx, c| {
        cx.core_mut()
            .set_resolution_and_full_screen(int_arg(c, 1), int_arg(c, 2), bool_arg(c, 3));
        Ok(Ret::Void)
    });
    def(b, cx, "setResolution", |cx, c| {
        let r = opt_str(c, 1).unwrap_or_default();
        cx.core_mut()
            .set_resolution(&r)
            .map_err(|m| java_error(m, "java.lang.NumberFormatException"))?;
        Ok(Ret::Void)
    });

    // ---- options.ini ----
    def(b, cx, "loadOptions", |cx, _| {
        Ok(cx.core_mut().load_options().map_err(io_error)?.into())
    });
    def(b, cx, "loadOptions_OLD", |cx, _| {
        Ok(cx.core_mut().load_options().map_err(io_error)?.into())
    });
    def(b, cx, "saveOptions", |cx, _| {
        cx.core_mut().save_options().map_err(io_error)?;
        Ok(Ret::Void)
    });
    def(b, cx, "saveOptions_OLD", |cx, _| {
        cx.core_mut().save_options().map_err(io_error)?;
        Ok(Ret::Void)
    });
    get!("isDefaultOptions", is_default_options);
    get!("isDedicated", is_dedicated);
    set_bool!("setWindowed", set_windowed);
    def(b, cx, "getScreenModes", |cx, _| {
        let t = TableRef::new();
        for (i, m) in cx.core().get_screen_modes().into_iter().enumerate() {
            t.set_int(i as i64 + 1, m);
        }
        Ok(Value::Table(t).into())
    });
    get!("isDoingTextEntry", is_doing_text_entry);
    void!("updateKeyboard", update_keyboard);
    void!("quit", quit);
    void!("exitToMenu", exit_to_menu);
    void!("quitToDesktop", quit_to_desktop);
    def(b, cx, "supportRes", |cx, c| {
        Ok(cx.core().support_res(int_arg(c, 1), int_arg(c, 2)).into())
    });
    def(b, cx, "init", |cx, c| {
        cx.core_mut().init(int_arg(c, 1), int_arg(c, 2));
        Ok(Ret::Void)
    });
    def(b, cx, "setScreenSize", |cx, c| {
        cx.core_mut().set_screen_size(int_arg(c, 1), int_arg(c, 2));
        Ok(Ret::Void)
    });

    // ---- key bindings ----
    void!("reinitKeyMaps", reinit_key_maps);
    def(b, cx, "invalidBindingShiftCtrl", |cx, c| {
        let kb = match c.arg(1) {
            Value::Object(o) => o
                .as_any()
                .downcast_ref::<LuaKeyBinding>()
                .map(|k| k.0.clone()),
            _ => None,
        };
        let Some(kb) = kb else {
            return Err(java_error("keyB is null", "java.lang.NullPointerException"));
        };
        Ok(cx.core().invalid_binding_shift_ctrl(&kb).into())
    });
    def(b, cx, "isKey", |cx, c| {
        let name = key_name_arg(c, 1);
        if c.arg(2).is_nil() {
            return Err(java_error("key is null", "java.lang.NullPointerException"));
        }
        let key = int_arg(c, 2);
        let core = cx.core();
        let kb = key_binding_for(&core, name);
        if core.invalid_binding_shift_ctrl(&kb) {
            return Ok(false.into());
        }
        Ok((kb.key_value == key || kb.alt_key == key).into())
    });
    def(b, cx, "getKey", |cx, c| {
        let core = cx.core();
        Ok(key_binding_for(&core, key_name_arg(c, 1)).key_value.into())
    });
    def(b, cx, "getAltKey", |cx, c| {
        let core = cx.core();
        Ok(key_binding_for(&core, key_name_arg(c, 1)).alt_key.into())
    });
    def(b, cx, "getKeyBinding", |cx, c| {
        let core = cx.core();
        let kb = match c.arg(1) {
            Value::Num(n) => core.get_key_binding_by_key(n as i32),
            _ => key_binding_for(&core, key_name_arg(c, 1)),
        };
        Ok(Value::Object(Rc::new(LuaKeyBinding(kb))).into())
    });
    def(b, cx, "addKeyBinding", |cx, c| {
        let name = key_name_arg(c, 1).unwrap_or_else(|| "null".into());
        cx.core_mut().add_key_binding(
            &name,
            int_arg(c, 2),
            int_arg(c, 3),
            bool_arg(c, 4),
            bool_arg(c, 5),
            bool_arg(c, 6),
        );
        Ok(Ret::Void)
    });

    // ---- version ----
    get!("getVersion", get_version);
    get!("getGameAndBuildVersion", get_game_and_build_version);
    get!("getBulletVersion", get_bullet_version);
    get_mut!("getGitSha", get_git_sha);
    get_mut!("getGitRevision", get_git_revision);
    def(b, cx, "getGameVersion", |cx, _| {
        Ok(Value::Object(Rc::new(LuaGameVersion(cx.core().get_game_version()))).into())
    });
    def(b, cx, "getBreakModGameVersion", |cx, _| {
        Ok(Value::Object(Rc::new(LuaGameVersion(
            cx.core().get_break_mod_game_version(),
        )))
        .into())
    });
    get!("getSteamServerVersion", get_steam_server_version);
    void!("DoFrameReady", do_frame_ready);
    get!("getVersionNumber", get_version_number);

    // ---- zoom ----
    get!("getCurrentPlayerZoom", get_current_player_zoom);
    def(b, cx, "getZoom", |cx, c| {
        Ok(cx.core().get_zoom(int_arg(c, 1)).into())
    });
    def(b, cx, "getNextZoom", |cx, c| {
        Ok(cx.core().get_next_zoom(int_arg(c, 1), int_arg(c, 2)).into())
    });
    get!("getMinZoom", get_min_zoom);
    get!("getMaxZoom", get_max_zoom);
    def(b, cx, "doZoomScroll", |cx, c| {
        cx.core_mut().do_zoom_scroll(int_arg(c, 1), int_arg(c, 2));
        Ok(Ret::Void)
    });
    get!("getSaveFolder", get_save_folder);
    set_bool!("zoomOptionChanged", zoom_option_changed);
    void!("zoomLevelsChanged", zoom_levels_changed);
    get!("isZoomEnabled", is_zoom_enabled);
    set_bool!("setZoomEnalbed", set_zoom_enalbed);
    void!("initFBOs", init_fbos);
    def(b, cx, "getAutoZoom", |cx, c| {
        let i = int_arg(c, 1);
        cx.core()
            .get_auto_zoom(i)
            .map(Ret::from)
            .ok_or_else(|| index_error(i, 4))
    });
    def(b, cx, "setAutoZoom", |cx, c| {
        let i = int_arg(c, 1);
        cx.core_mut()
            .set_auto_zoom(i, bool_arg(c, 2))
            .ok_or_else(|| index_error(i, 4))?;
        Ok(Ret::Void)
    });
    def(b, cx, "getDefaultZoomLevels", |cx, _| {
        let v = cx
            .core()
            .get_default_zoom_levels()
            .into_iter()
            .map(|p| Value::Num(p as f64))
            .collect();
        Ok(Value::Object(Rc::new(JavaList(v))).into())
    });

    // ---- options with side effects ----
    set_bool!("setOptionVSync", set_option_vsync);
    get!("getRealOptionSoundVolume", get_real_option_sound_volume);
    set_int!("setOptionSoundVolume", set_option_sound_volume);
    set_int!("setOptionMusicVolume", set_option_music_volume);
    set_int!("setOptionAmbientVolume", set_option_ambient_volume);
    set_int!(
        "setOptionVehicleEngineVolume",
        set_option_vehicle_engine_volume
    );
    def(b, cx, "setOptionVoiceEnable", |cx, c| {
        if c.nargs() >= 3 {
            cx.core_mut()
                .set_option_voice_enable_restart(bool_arg(c, 1), bool_arg(c, 2));
        } else {
            cx.core_mut().set_option_voice_enable(bool_arg(c, 1));
        }
        Ok(Ret::Void)
    });
    set_int!("setOptionVoiceMode", set_option_voice_mode);
    set_int!("setOptionVoiceVADMode", set_option_voice_vad_mode);
    set_int!("setOptionVoiceAGCMode", set_option_voice_agc_mode);
    set_int!("setOptionVoiceVolumeMic", set_option_voice_volume_mic);
    set_int!(
        "setOptionVoiceVolumePlayers",
        set_option_voice_volume_players
    );
    set_str!(
        "setOptionVoiceRecordDeviceName",
        set_option_voice_record_device_name
    );
    get!("getOptionVoiceRecordDevice", get_option_voice_record_device);
    set_int!("setOptionVoiceRecordDevice", set_option_voice_record_device);
    get!("getMicVolumeIndicator", get_mic_volume_indicator);
    get!("getMicVolumeError", get_mic_volume_error);
    get!("getServerVOIPEnable", get_server_voip_enable);
    set_bool!("setTestingMicrophone", set_testing_microphone);
    get!("getOptionReloadDifficulty", get_option_reload_difficulty);
    get!("getOptionFontSizeReal", get_option_font_size_real);
    set_str!("setOptionCodeFontSize", set_option_code_font_size);
    set_str!("setOptionTooltipFont", set_option_tooltip_font);
    get!("getOptionModsEnabled", get_option_mods_enabled);
    set_bool!("setOptionModsEnabled", set_option_mods_enabled);
    get!(
        "allowOptionTextureCompression",
        allow_option_texture_compression
    );
    get!(
        "getOptionTextureCompression",
        get_option_texture_compression
    );
    get!("getOptionTexture2x", get_option_texture2x);
    set_bool!("setOptionTexture2x", set_option_texture2x);
    def(b, cx, "getMaxTextureSizeFromFlags", |cx, c| {
        Ok(cx
            .core()
            .get_max_texture_size_from_flags(int_arg(c, 1))
            .into())
    });
    def(b, cx, "getMaxTextureSizeFromOption", |cx, c| {
        let o = int_arg(c, 1);
        cx.core()
            .get_max_texture_size_from_option(o)
            .map(Ret::from)
            .ok_or_else(|| {
                java_error(
                    format!("Unexpected value: {o}"),
                    "java.lang.IllegalStateException",
                )
            })
    });
    get!("getMaxTextureSize", get_max_texture_size);
    get!("getMaxVehicleTextureSize", get_max_vehicle_texture_size);
    get!("getOptionZoomLevels1x", get_option_zoom_levels1x);
    set_str!("setOptionZoomLevels1x", set_option_zoom_levels1x);
    get!("getOptionZoomLevels2x", get_option_zoom_levels2x);
    set_str!("setOptionZoomLevels2x", set_option_zoom_levels2x);
    set_str!("setOptionScreenFilter", set_option_screen_filter);
    get!("getScreenFilter", get_screen_filter);
    def(b, cx, "setOptionActiveController", |cx, c| {
        cx.core_mut()
            .set_option_active_controller(int_arg(c, 1), bool_arg(c, 2));
        Ok(Ret::Void)
    });
    def(b, cx, "getOptionActiveController", |cx, c| {
        let g = opt_str(c, 1);
        Ok(
            g.is_some_and(|g| cx.core().get_option_active_controller(&g))
                .into(),
        )
    });
    set_bool!("setOptionUIFBO", set_option_uifbo);
    def(b, cx, "onOptionControllerButtonStyleChanged", |cx, _| {
        cx.core_mut().on_option_controller_button_style_changed();
        Ok(Ret::Void)
    });
    get!(
        "getOptionControllerButtonStyleString",
        get_option_controller_button_style_string
    );
    def(b, cx, "onOptionGamepadBindingPresetChanged", |cx, _| {
        cx.core_mut().on_option_gamepad_binding_preset_changed();
        Ok(Ret::Void)
    });
    def(b, cx, "getOptionSingleContextMenu", |cx, c| {
        let i = int_arg(c, 1);
        cx.core()
            .get_option_single_context_menu(i)
            .map(Ret::from)
            .ok_or_else(|| index_error(i, 4))
    });
    def(b, cx, "setOptionSingleContextMenu", |cx, c| {
        let i = int_arg(c, 1);
        cx.core_mut()
            .set_option_single_context_menu(i, bool_arg(c, 2))
            .ok_or_else(|| index_error(i, 4))?;
        Ok(Ret::Void)
    });
    def(b, cx, "isOptionSimpleClothingTextures", |cx, c| {
        Ok(cx
            .core()
            .is_option_simple_clothing_textures(bool_arg(c, 1))
            .into())
    });
    get!("getIgnoreProneZombieRange", get_ignore_prone_zombie_range);

    // ---- Lua reset (queued for the engine) ----
    def(b, cx, "ResetLua", |cx, c| {
        let mods = match c.arg(1) {
            Value::Bool(_) => "default".to_string(),
            _ => opt_str(c, 1).unwrap_or_else(|| "null".into()),
        };
        let reason = opt_str(c, 2).unwrap_or_else(|| "null".into());
        cx.core_mut().reset_lua(&mods, &reason);
        Ok(Ret::Void)
    });
    def(b, cx, "DelayResetLua", |cx, c| {
        let mods = opt_str(c, 1).unwrap_or_else(|| "null".into());
        let reason = opt_str(c, 2).unwrap_or_else(|| "null".into());
        cx.core_mut().delay_reset_lua(&mods, &reason);
        Ok(Ret::Void)
    });
    void!("CheckDelayResetLua", check_delay_reset_lua);

    // ---- plain state ----
    get!("isShowPing", is_show_ping);
    set_bool!("setShowPing", set_show_ping);
    get!("isForceSnow", is_force_snow);
    set_bool!("setForceSnow", set_force_snow);
    get!("isZombieGroupSound", is_zombie_group_sound);
    set_bool!("setZombieGroupSound", set_zombie_group_sound);
    get!("getBlinkingMoodle", get_blinking_moodle);
    set_str!("setBlinkingMoodle", set_blinking_moodle);
    void!("initPoisonousBerry", init_poisonous_berry);
    void!("initPoisonousMushroom", init_poisonous_mushroom);
    get!("getPoisonousBerry", get_poisonous_berry);
    set_str!("setPoisonousBerry", set_poisonous_berry);
    get!("getPoisonousMushroom", get_poisonous_mushroom);
    set_str!("setPoisonousMushroom", set_poisonous_mushroom);
    get!("isSelectingAll", is_selecting_all);
    set_bool!("setIsSelectingAll", set_is_selecting_all);
    color!(
        "getMpTextColor",
        "setMpTextColor",
        "mpTextColor",
        get_mp_text_color,
        set_mp_text_color
    );
    get!("isAzerty", is_azerty);
    set_bool!("setAzerty", set_azerty);
    color!(
        "getObjectHighlitedColor",
        "setObjectHighlitedColor",
        "objectHighlitedColor",
        get_object_highlited_color,
        set_object_highlited_color
    );
    color!(
        "getWorldItemHighlightColor",
        "setWorldItemHighlightColor",
        "worldItemHighlightColor",
        get_world_item_highlight_color,
        set_world_item_highlight_color
    );
    color!(
        "getGoodHighlitedColor",
        "setGoodHighlitedColor",
        "goodHighlitedColor",
        get_good_highlited_color,
        set_good_highlited_color
    );
    color!(
        "getBadHighlitedColor",
        "setBadHighlitedColor",
        "badHighlitedColor",
        get_bad_highlited_color,
        set_bad_highlited_color
    );
    color!(
        "getTargetColor",
        "setTargetColor",
        "targetColor",
        get_target_color,
        set_target_color
    );
    color!(
        "getNoTargetColor",
        "setNoTargetColor",
        "noTargetColor",
        get_no_target_color,
        set_no_target_color
    );
    def(b, cx, "getXAngle", |cx, c| {
        Ok(cx.core().get_x_angle(int_arg(c, 1), float_arg(c, 2)).into())
    });
    def(b, cx, "getYAngle", |cx, c| {
        Ok(cx.core().get_y_angle(int_arg(c, 1), float_arg(c, 2)).into())
    });
    get!("isInDebug", is_in_debug);
    get!("isNoSave", is_no_save);
    set_bool!("setNoSave", set_no_save);
    get!(
        "isShowFirstTimeVehicleTutorial",
        is_show_first_time_vehicle_tutorial
    );
    set_bool!(
        "setShowFirstTimeVehicleTutorial",
        set_show_first_time_vehicle_tutorial
    );
    get!(
        "isShowFirstTimeWeatherTutorial",
        is_show_first_time_weather_tutorial
    );
    set_bool!(
        "setShowFirstTimeWeatherTutorial",
        set_show_first_time_weather_tutorial
    );
    set_bool!("setAnimPopupDone", set_anim_popup_done);
    get!("isAnimPopupDone", is_anim_popup_done);
    set_bool!("setModsPopupDone", set_mods_popup_done);
    get!("isModsPopupDone", is_mods_popup_done);
    get!("isCollideZombies", is_collide_zombies);
    set_bool!("setCollideZombies", set_collide_zombies);
    get!("isFlashIsoCursor", is_flash_iso_cursor);
    set_bool!("setFlashIsoCursor", set_flash_iso_cursor);
    get!("isOptionProgressBar", is_option_progress_bar);
    def(b, cx, "getOptionOnStartup", |cx, c| {
        let name = opt_str(c, 1).unwrap_or_else(|| "null".into());
        Ok(load(cx.core().get_option_on_startup(&name)).into())
    });
    def(b, cx, "setOptionOnStartup", |cx, c| {
        let name = opt_str(c, 1).unwrap_or_else(|| "null".into());
        cx.core_mut().set_option_on_startup(&name, store(c.arg(2)));
        Ok(Ret::Void)
    });
    def(b, cx, "countMissing3DItems", |cx, _| {
        // System.out.println in Java.
        println!("{}", cx.core().count_missing3d_items());
        Ok(Ret::Void)
    });
    set_bool!("setAnimalCheat", set_animal_cheat);
    set_bool!("setDisplayPlayerModel", set_display_player_model);
    get!("isDisplayPlayerModel", is_display_player_model);
    set_bool!("setDisplayCursor", set_display_cursor);
    get!("isDisplayCursor", is_display_cursor);
    get!("getIsoCursorAlpha", get_iso_cursor_alpha);
    def(b, cx, "debugOutputMissingItemSpawn", |cx, _| {
        Ok(cx
            .core()
            .debug_output_missing_item_spawn()
            .map_err(io_error)?
            .into())
    });
    def(b, cx, "debugOutputMissingCLothingSpawn", |cx, _| {
        Ok(cx
            .core()
            .debug_output_missing_clothing_spawn()
            .map_err(io_error)?
            .into())
    });
    def(b, cx, "debugOutputMissingSpawn", |cx, c| {
        let d = opt_str(c, 1).unwrap_or_default();
        let k = opt_str(c, 2).unwrap_or_else(|| "null".into());
        Ok(cx
            .core()
            .debug_output_missing_spawn(&d, &k)
            .map_err(io_error)?
            .into())
    });
    get!("getSelectedMap", get_selected_map);
    set_str!("setSelectedMap", set_selected_map);
    get!("getConsoleDotTxtSizeKB", get_console_dot_txt_size_kb);
    def(b, cx, "setConsoleDotTxtSizeKB", |cx, c| {
        match c.arg(1) {
            Value::Str(s) => cx.core_mut().set_console_dot_txt_size_kb_str(Some(&s)),
            _ => cx.core_mut().set_console_dot_txt_size_kb(int_arg(c, 1)),
        }
        Ok(Ret::Void)
    });
    get!(
        "getOptionUsePhysicsHitReaction",
        get_option_use_physics_hit_reaction
    );
    def(b, cx, "getAccountUsed", |cx, _| {
        Ok(load(cx.core().get_account_used()).into())
    });
    def(b, cx, "setAccountUsed", |cx, c| {
        cx.core_mut().set_account_used(store(c.arg(1)));
        Ok(Ret::Void)
    });
}

fn config_option_class(kind: &str) -> &'static str {
    match kind {
        "boolean" => "zombie.config.BooleanConfigOption",
        "integer" => "zombie.config.IntegerConfigOption",
        "double" => "zombie.config.DoubleConfigOption",
        "string" => "zombie.config.StringConfigOption",
        _ => "zombie.config.ArrayConfigOption",
    }
}

fn with_color<R>(c: &NativeCall<'_>, f: impl FnOnce(&LuaColorInfo) -> R) -> LuaResult<R> {
    match c.arg(0) {
        Value::Object(o) => match o.as_any().downcast_ref::<LuaColorInfo>() {
            Some(ci) => Ok(f(ci)),
            None => Err(LuaError::runtime(format!(
                "Expected ColorInfo but got {}",
                o.class_name()
            ))),
        },
        other => Err(LuaError::runtime(format!(
            "Expected ColorInfo but got {}",
            pz_kahlua::stdlib::java_class_name(&other)
        ))),
    }
}

fn register_helpers(thread: &mut Thread, cx: &Rc<Ctx>) {
    if class_free(thread, COLOR_INFO_CLASS) {
        let b = expose_class(thread, COLOR_INFO_CLASS, None);
        b.ctor(|c| {
            let ci = if c.nargs() >= 4 {
                ColorInfo::new(
                    float_arg(c, 0),
                    float_arg(c, 1),
                    float_arg(c, 2),
                    float_arg(c, 3),
                )
            } else {
                ColorInfo::default()
            };
            Ok(c.push(Value::Object(LuaColorInfo::new(ci))))
        });
        b.method("getR", |c| {
            let v = with_color(c, |ci| ci.c.get().r)?;
            Ok(c.push(v as f64))
        });
        b.method("getG", |c| {
            let v = with_color(c, |ci| ci.c.get().g)?;
            Ok(c.push(v as f64))
        });
        b.method("getB", |c| {
            let v = with_color(c, |ci| ci.c.get().b)?;
            Ok(c.push(v as f64))
        });
        b.method("getA", |c| {
            let v = with_color(c, |ci| ci.c.get().a)?;
            Ok(c.push(v as f64))
        });
        b.method("set", |c| {
            let new = if c.nargs() >= 5 {
                ColorInfo::new(
                    float_arg(c, 1),
                    float_arg(c, 2),
                    float_arg(c, 3),
                    float_arg(c, 4),
                )
            } else {
                color_arg(c, 1)?
            };
            with_color(c, |ci| ci.c.set(new))?;
            Ok(c.push(c.arg(0)))
        });
        b.method("setRGB", |c| {
            let (r, g, bl) = if c.nargs() >= 4 {
                (float_arg(c, 1), float_arg(c, 2), float_arg(c, 3))
            } else {
                let v = float_arg(c, 1);
                (v, v, v)
            };
            with_color(c, |ci| {
                let a = ci.c.get().a;
                ci.c.set(ColorInfo::new(r, g, bl, a))
            })?;
            Ok(c.push(c.arg(0)))
        });
        b.method("setABGR", |c| {
            let abgr = int_arg(c, 1) as u32;
            let ch = |shift: u32| ((abgr >> shift) & 0xFF) as f32 / 255.0;
            with_color(c, |ci| {
                ci.c.set(ColorInfo::new(ch(0), ch(8), ch(16), ch(24)))
            })?;
            Ok(c.push(c.arg(0)))
        });
        b.method("min", |c| {
            let (r, g, bl, a) = (
                float_arg(c, 1),
                float_arg(c, 2),
                float_arg(c, 3),
                float_arg(c, 4),
            );
            with_color(c, |ci| {
                let o = ci.c.get();
                ci.c.set(ColorInfo::new(
                    o.r.min(r),
                    o.g.min(g),
                    o.b.min(bl),
                    o.a.min(a),
                ))
            })?;
            Ok(c.push(c.arg(0)))
        });
        b.method("minRGB", |c| {
            let (r, g, bl) = if c.nargs() >= 4 {
                (float_arg(c, 1), float_arg(c, 2), float_arg(c, 3))
            } else {
                let v = float_arg(c, 1);
                (v, v, v)
            };
            with_color(c, |ci| {
                let o = ci.c.get();
                ci.c.set(ColorInfo::new(
                    o.r.min(r),
                    o.g.min(g),
                    o.b.min(bl),
                    o.a.min(1.0),
                ))
            })?;
            Ok(c.push(c.arg(0)))
        });
        b.method("desaturate", |c| {
            let s = float_arg(c, 1);
            with_color(c, |ci| {
                let o = ci.c.get();
                let gray = o.r * 0.3086 + o.g * 0.6094 + o.b * 0.082;
                ci.c.set(ColorInfo::new(
                    gray * s + o.r * (1.0 - s),
                    gray * s + o.g * (1.0 - s),
                    gray * s + o.b * (1.0 - s),
                    o.a,
                ))
            })?;
            Ok(0)
        });
        b.method("interp", |c| {
            let to = color_arg(c, 1)?;
            let delta = float_arg(c, 2);
            let Value::Object(dest) = c.arg(3) else {
                return Err(java_error("dest is null", "java.lang.NullPointerException"));
            };
            let Some(dest) = dest.as_any().downcast_ref::<LuaColorInfo>() else {
                return Err(java_error(
                    "dest is not a ColorInfo",
                    "java.lang.ClassCastException",
                ));
            };
            let from = with_color(c, |ci| ci.c.get())?;
            dest.c.set(ColorInfo::new(
                from.r + (to.r - from.r) * delta,
                from.g + (to.g - from.g) * delta,
                from.b + (to.b - from.b) * delta,
                from.a + (to.a - from.a) * delta,
            ));
            Ok(0)
        });
        b.method("equals", |c| {
            let a = with_color(c, |ci| ci.c.get())?;
            let r = match c.arg(1) {
                Value::Object(o) => o
                    .as_any()
                    .downcast_ref::<LuaColorInfo>()
                    .is_some_and(|o| o.c.get() == a),
                _ => false,
            };
            Ok(c.push(r))
        });
        b.method("toString", |c| {
            let s = with_color(c, |ci| ci.c.get().to_string())?;
            Ok(c.push(s))
        });
    }

    if class_free(thread, GAME_VERSION_CLASS) {
        let b = expose_class(thread, GAME_VERSION_CLASS, None);
        fn gv(c: &NativeCall<'_>, i: usize) -> LuaResult<GameVersion> {
            match c.arg(i) {
                Value::Object(o) => o
                    .as_any()
                    .downcast_ref::<LuaGameVersion>()
                    .map(|g| g.0.clone())
                    .ok_or_else(|| {
                        LuaError::runtime(format!(
                            "Expected GameVersion but got {}",
                            o.class_name()
                        ))
                    }),
                _ => Err(java_error(
                    "GameVersion is null",
                    "java.lang.NullPointerException",
                )),
            }
        }
        b.ctor(|c| {
            let v = GameVersion::new(
                int_arg(c, 0).max(0),
                int_arg(c, 1).clamp(0, 999),
                &opt_str(c, 2).unwrap_or_default(),
            );
            Ok(c.push(Value::Object(Rc::new(LuaGameVersion(v)))))
        });
        b.static_fn("parse", |c| {
            let s = opt_str(c, 0).unwrap_or_default();
            match GameVersion::parse(&s) {
                Some(v) => Ok(c.push(Value::Object(Rc::new(LuaGameVersion(v))))),
                None => Err(java_error(
                    format!("invalid game version \"{s}\""),
                    "java.lang.IllegalArgumentException",
                )),
            }
        });
        b.method("getMajor", |c| {
            let v = gv(c, 0)?;
            Ok(c.push(v.major))
        });
        b.method("getMinor", |c| {
            let v = gv(c, 0)?;
            Ok(c.push(v.minor))
        });
        b.method("getSuffix", |c| {
            let v = gv(c, 0)?;
            Ok(c.push(v.suffix))
        });
        b.method("getInt", |c| {
            let v = gv(c, 0)?;
            Ok(c.push(v.get_int()))
        });
        b.method("isGreaterThan", |c| {
            let r = gv(c, 0)?.is_greater_than(&gv(c, 1)?);
            Ok(c.push(r))
        });
        b.method("isGreaterThanOrEqualTo", |c| {
            let r = gv(c, 0)?.is_greater_than_or_equal_to(&gv(c, 1)?);
            Ok(c.push(r))
        });
        b.method("isLessThan", |c| {
            let r = gv(c, 0)?.is_less_than(&gv(c, 1)?);
            Ok(c.push(r))
        });
        b.method("isLessThanOrEqualTo", |c| {
            let r = gv(c, 0)?.is_less_than_or_equal_to(&gv(c, 1)?);
            Ok(c.push(r))
        });
        b.method("equals", |c| {
            let a = gv(c, 0)?;
            let r = gv(c, 1).is_ok_and(|o| o.major == a.major && o.minor == a.minor);
            Ok(c.push(r))
        });
        b.method("toString", |c| {
            let v = gv(c, 0)?;
            Ok(c.push(v.to_string()))
        });
    }

    if class_free(thread, KEY_BINDING_CLASS) {
        let b = expose_class(thread, KEY_BINDING_CLASS, None);
        fn kb(c: &NativeCall<'_>) -> LuaResult<KeyBinding> {
            match c.arg(0) {
                Value::Object(o) => o
                    .as_any()
                    .downcast_ref::<LuaKeyBinding>()
                    .map(|k| k.0.clone())
                    .ok_or_else(|| {
                        LuaError::runtime(format!("Expected KeyBinding but got {}", o.class_name()))
                    }),
                _ => Err(LuaError::runtime("Expected KeyBinding")),
            }
        }
        b.method("name", |c| {
            let k = kb(c)?;
            Ok(c.push(k.name))
        });
        b.method("keyValue", |c| {
            let k = kb(c)?;
            Ok(c.push(k.key_value))
        });
        b.method("altKey", |c| {
            let k = kb(c)?;
            Ok(c.push(k.alt_key))
        });
        b.method("shift", |c| {
            let k = kb(c)?;
            Ok(c.push(k.shift))
        });
        b.method("ctrl", |c| {
            let k = kb(c)?;
            Ok(c.push(k.ctrl))
        });
        b.method("alt", |c| {
            let k = kb(c)?;
            Ok(c.push(k.alt))
        });
        b.method("toString", |c| {
            let k = kb(c)?;
            Ok(c.push(k.to_string()))
        });
        b.method("equals", |c| {
            let k = kb(c)?;
            let r = match c.arg(1) {
                Value::Object(o) => o
                    .as_any()
                    .downcast_ref::<LuaKeyBinding>()
                    .is_some_and(|o| o.0 == k),
                _ => false,
            };
            Ok(c.push(r))
        });
    }

    if class_free(thread, KEYBIND_ID_CLASS) {
        let b = expose_class(thread, KEYBIND_ID_CLASS, None);
        for (field, id) in KEYBIND_IDS {
            b.field(
                field,
                Value::Object(Rc::new(KeybindId { id: id.to_string() })),
            );
        }
        b.static_fn("register", |c| {
            let id = opt_str(c, 0).unwrap_or_default();
            Ok(c.push(Value::Object(Rc::new(KeybindId { id }))))
        });
        b.method("getId", |c| {
            let s = match c.arg(0) {
                Value::Object(o) => o.to_lua_string(),
                _ => String::new(),
            };
            Ok(c.push(s))
        });
        b.method("toString", |c| {
            let s = match c.arg(0) {
                Value::Object(o) => o.to_lua_string(),
                _ => String::new(),
            };
            Ok(c.push(s))
        });
    }

    if class_free(thread, ARRAY_LIST_CLASS) {
        let b = expose_class(thread, ARRAY_LIST_CLASS, None);
        fn list<R>(c: &NativeCall<'_>, f: impl FnOnce(&[Value]) -> R) -> LuaResult<R> {
            match c.arg(0) {
                Value::Object(o) => match o.as_any().downcast_ref::<JavaList>() {
                    Some(l) => Ok(f(&l.0)),
                    None => Err(LuaError::runtime(format!(
                        "Expected ArrayList but got {}",
                        o.class_name()
                    ))),
                },
                _ => Err(LuaError::runtime("Expected ArrayList")),
            }
        }
        b.method("size", |c| {
            let n = list(c, |l| l.len())?;
            Ok(c.push(n as f64))
        });
        b.method("isEmpty", |c| {
            let e = list(c, |l| l.is_empty())?;
            Ok(c.push(e))
        });
        b.method("get", |c| {
            let i = int_arg(c, 1);
            let (v, n) = list(c, |l| {
                (
                    usize::try_from(i).ok().and_then(|i| l.get(i)).cloned(),
                    l.len(),
                )
            })?;
            match v {
                Some(v) => Ok(c.push(v)),
                None => Err(index_error(i, n)),
            }
        });
        b.method("contains", |c| {
            let x = c.arg(1);
            let r = list(c, |l| l.iter().any(|v| v.raw_equals(&x)))?;
            Ok(c.push(r))
        });
        b.method("indexOf", |c| {
            let x = c.arg(1);
            let r = list(c, |l| {
                l.iter()
                    .position(|v| v.raw_equals(&x))
                    .map_or(-1, |i| i as i32)
            })?;
            Ok(c.push(r))
        });
        b.method("toString", |c| {
            let s = match c.arg(0) {
                Value::Object(o) => o.to_lua_string(),
                _ => String::new(),
            };
            Ok(c.push(s))
        });
    }

    if class_free(thread, CONFIG_OPTION_CLASS) {
        let b = expose_class(thread, CONFIG_OPTION_CLASS, None);
        fn opt<R>(c: &NativeCall<'_>, f: impl FnOnce(&mut Core, usize) -> R) -> LuaResult<R> {
            match c.arg(0) {
                Value::Object(o) => match o.as_any().downcast_ref::<LuaConfigOption>() {
                    Some(r) => Ok(f(&mut r.core.borrow_mut(), r.index)),
                    None => Err(LuaError::runtime(format!(
                        "Expected ConfigOption but got {}",
                        o.class_name()
                    ))),
                },
                _ => Err(LuaError::runtime("Expected ConfigOption")),
            }
        }
        fn with_opt<R>(
            core: &mut Core,
            i: usize,
            f: impl FnOnce(&mut pz_core::config::ConfigOption) -> R,
        ) -> R {
            let name = core.options()[i].name().to_string();
            f(core.option_mut(&name).unwrap())
        }
        b.method("getName", |c| {
            let s = opt(c, |core, i| core.options()[i].name().to_string())?;
            Ok(c.push(s))
        });
        b.method("getType", |c| {
            let s = opt(c, |core, i| core.options()[i].get_type())?;
            Ok(c.push(s))
        });
        b.method("getValueAsString", |c| {
            let s = opt(c, |core, i| core.options()[i].get_value_as_string())?;
            Ok(c.push(s))
        });
        b.method("getValueAsLuaString", |c| {
            let s = opt(c, |core, i| core.options()[i].get_value_as_lua_string())?;
            Ok(c.push(s))
        });
        b.method("getTooltip", |c| {
            let s = opt(c, |core, i| core.options()[i].get_tooltip())?;
            Ok(c.push(s))
        });
        b.method("getValueAsObject", |c| {
            use pz_core::config::ConfigObject;
            let v = opt(c, |core, i| match core.options()[i].get_value_as_object() {
                ConfigObject::Bool(b) => Value::Bool(b),
                ConfigObject::Double(d) => Value::Num(d),
                ConfigObject::Str(s) | ConfigObject::Other(s) => Value::from(s),
                ConfigObject::Null => Value::Nil,
            })?;
            Ok(c.push(v))
        });
        b.method("isValidString", |c| {
            let s = opt_str(c, 1);
            let r = opt(c, |core, i| {
                s.is_some_and(|s| core.options()[i].is_valid_string(&s))
            })?;
            Ok(c.push(r))
        });
        b.method("parse", |c| {
            let s = opt_str(c, 1).unwrap_or_default();
            opt(c, |core, i| with_opt(core, i, |o| o.parse(&s)))?;
            Ok(0)
        });
        b.method("setValueFromObject", |c| {
            use pz_core::config::ConfigObject;
            let v = match c.arg(1) {
                Value::Nil => ConfigObject::Null,
                Value::Bool(b) => ConfigObject::Bool(b),
                Value::Num(n) => ConfigObject::Double(n),
                Value::Str(s) => ConfigObject::Str(s.to_string()),
                other => ConfigObject::Other(pz_kahlua::exec::java_value_of(&other)),
            };
            opt(c, |core, i| {
                with_opt(core, i, |o| o.set_value_from_object(&v))
            })?;
            Ok(0)
        });
        b.method("resetToDefault", |c| {
            opt(c, |core, i| with_opt(core, i, |o| o.reset_to_default()))?;
            Ok(0)
        });
        b.method("setDefaultToCurrentValue", |c| {
            opt(c, |core, i| {
                with_opt(core, i, |o| o.set_default_to_current_value())
            })?;
            Ok(0)
        });
    }
    let _ = cx;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::{EventManager, EVENT_NAMES};

    /// The distinct `getCore():<method>` calls in the vanilla Lua
    /// (`media/lua`), all of which must exist.
    const LUA_CALLED: &str = "
    DelayResetLua ResetLua addKeyBinding allowOptionTextureCompression countMissing3DItems
    debugOutputMissingCLothingSpawn debugOutputMissingItemSpawn doZoomScroll exitToMenu
    getAccountUsed getAltKey getAutoZoom getBadHighlitedColor getBulletVersion getChallengeID
    getContentTranslationsEnabled getDebug getDefaultZoomLevels getGameMode getGitSha
    getGoodHighlitedColor getIsoCursorVisibility getKey getMaxActiveRagdolls getMaxZoom
    getMicVolumeError getMicVolumeIndicator getMinZoom getMpTextColor getNextZoom getNoTargetColor
    getObjectHighlitedColor getOptionActionProgressBarSize getOptionActiveController
    getOptionAimTextureIndex getOptionAmbientVolume getOptionAutoDrink
    getOptionAutoRevealPrintMediaMapLocations getOptionAutoWalkContainer getOptionBloodDecals
    getOptionBorderlessWindow getOptionChatFadeTime getOptionChatFontSize
    getOptionChatOpaqueOnFocus getOptionClock24Hour getOptionClockFormat getOptionClockSize
    getOptionCodeFontSize getOptionColorblindPatterns getOptionContextMenuFont
    getOptionControllerButtonStyle getOptionControllerButtonStyleString getOptionCorpseShadows
    getOptionCrosshairTextureIndex getOptionCycleContainerKey getOptionDisableLightningDuringStorms
    getOptionDisplayAsCelsius getOptionDoContainerOutline getOptionDoDoorSpriteEffects
    getOptionDoVideoEffects getOptionDoWindSpriteEffects getOptionDropItemsOnSquareCenter
    getOptionEnableDyslexicFont getOptionEnableLeftJoystickRadialMenu getOptionFontSize
    getOptionFontSizeReal getOptionGamepadBindingPreset getOptionHighResPlacedItems
    getOptionInventoryContainerSize getOptionInventoryFont getOptionJumpScareVolume
    getOptionLeaveKeyInIgnition getOptionLockCursorToWindow
    getOptionMacOSIgnoreMouseWheelAcceleration getOptionMacOSMapHorizontalMouseWheelToVertical
    getOptionMapViewPause getOptionMaxChatOpaque getOptionMaxCrosshairOffset
    getOptionMaxTextureSize getOptionMaxVehicleTextureSize getOptionMeleeOutline
    getOptionMinChatOpaque getOptionModsEnabled getOptionMoodleSize getOptionMusicActionStyle
    getOptionMusicLibrary getOptionMusicVolume getOptionOnStartup getOptionPanCameraWhileAiming
    getOptionPanCameraWhileDriving getOptionPauseOnFocusloss getOptionPrecipitationSpeedMultiplier
    getOptionRadialMenuKeyToggle getOptionReloadRadialInstant getOptionRenderPrecipitation
    getOptionReticleCameraZoom getOptionReticleMode getOptionReticleTextureIndex
    getOptionScreenFilter getOptionSearchModeOverlayEffect getOptionShoulderButtonContainerSwitch
    getOptionShowAimTexture getOptionShowCraftingXP getOptionShowCursorWhileAiming
    getOptionShowFirstAnimalZoneInfo getOptionShowItemModInfo getOptionShowReticleTexture
    getOptionShowSurvivalGuide getOptionShowValidTargetReticleTexture getOptionSidebarSize
    getOptionSimpleClothingTextures getOptionSimpleWeaponTextures getOptionSingleContextMenu
    getOptionSoundVolume getOptionStreamerMode getOptionTextureCompression
    getOptionTieredZombieUpdates getOptionTimedActionGameSpeedReset getOptionTooltipFont
    getOptionUIFBO getOptionUIRenderFPS getOptionUpdateSneakButton getOptionUsePhysicsHitReaction
    getOptionVSync getOptionValidTargetReticleTextureIndex getOptionVehicleEngineVolume
    getOptionVoiceAGCMode getOptionVoiceEnable getOptionVoiceMode getOptionVoiceRecordDevice
    getOptionVoiceVADMode getOptionVoiceVolumeMic getOptionVoiceVolumePlayers
    getOptionWorldMapBrightness getOptionZoom getOptionZoomLevels1x getOptionZoomLevels2x
    getPerfPuddles getPerfPuddlesOnLoad getPerfReflections getPerfReflectionsOnLoad getPerfSkybox
    getPerfSkyboxOnLoad getScreenHeight getScreenModes getScreenWidth getSelectedMap
    getServerVOIPEnable getTargetColor getTermsOfServiceVersion getVersion
    getWorldItemHighlightColor getZoom gotNewBelt isAzerty isCelsius isChallenge isDedicated
    isDisplayCursor isDisplayPlayerModel isFullScreen isKey isModsPopupDone isOption3DGroundItem
    isOptionAutoProneAtk isOptionShowChatTimestamp isOptionShowChatTitle isOptiondblTapJogToSprint
    isPopulateServerListOnStart isShowFirstTimeSearchTutorial isShowYourUsername isToggleToAim
    isToggleToRun isToggleToSprint isTutorialDone isZoomEnabled quit quitToDesktop reinitKeyMaps
    saveOptions setAccountUsed setAnimalCheat setAutoZoom setBadHighlitedColor setBlinkingMoodle
    setCelsius setCollideZombies setContentTranslationsEnabled setDisplayCursor
    setDisplayPlayerModel setFlashIsoCursor setForceSnow setFramerate setGoodHighlitedColor
    setGotNewBelt setIsSelectingAll setIsoCursorVisibility setMaxActiveRagdolls setModsPopupDone
    setMpTextColor setNoSave setNoTargetColor setObjectHighlitedColor setOption3DGroundItem
    setOptionActionProgressBarSize setOptionActiveController setOptionAimTextureIndex
    setOptionAutoDrink setOptionAutoProneAtk setOptionAutoRevealPrintMediaMapLocations
    setOptionAutoWalkContainer setOptionBloodDecals setOptionBorderlessWindow setOptionChatFadeTime
    setOptionChatOpaqueOnFocus setOptionClock24Hour setOptionClockFormat setOptionClockSize
    setOptionCodeFontSize setOptionColorblindPatterns setOptionContextMenuFont
    setOptionControllerButtonStyle setOptionCorpseShadows setOptionCrosshairTextureIndex
    setOptionCycleContainerKey setOptionDisableLightningDuringStorms setOptionDisplayAsCelsius
    setOptionDoContainerOutline setOptionDoDoorSpriteEffects setOptionDoVideoEffects
    setOptionDoWindSpriteEffects setOptionDropItemsOnSquareCenter setOptionEnableDyslexicFont
    setOptionEnableLeftJoystickRadialMenu setOptionFontSize setOptionGamepadBindingPreset
    setOptionHighResPlacedItems setOptionInventoryContainerSize setOptionInventoryFont
    setOptionJumpScareVolume setOptionLanguageName setOptionLeaveKeyInIgnition
    setOptionLockCursorToWindow setOptionMacOSIgnoreMouseWheelAcceleration
    setOptionMacOSMapHorizontalMouseWheelToVertical setOptionMapViewPause setOptionMaxChatOpaque
    setOptionMaxCrosshairOffset setOptionMaxTextureSize setOptionMaxVehicleTextureSize
    setOptionMeleeOutline setOptionMinChatOpaque setOptionMoodleSize setOptionMusicActionStyle
    setOptionMusicLibrary setOptionMusicVolume setOptionOnStartup setOptionPanCameraWhileAiming
    setOptionPanCameraWhileDriving setOptionPauseOnFocusloss setOptionPrecipitationSpeedMultiplier
    setOptionRadialMenuKeyToggle setOptionReloadRadialInstant setOptionRenderPrecipitation
    setOptionReticleCameraZoom setOptionReticleMode setOptionReticleTextureIndex
    setOptionScreenFilter setOptionSearchModeOverlayEffect setOptionShoulderButtonContainerSwitch
    setOptionShowAimTexture setOptionShowCraftingXP setOptionShowCursorWhileAiming
    setOptionShowFirstAnimalZoneInfo setOptionShowItemModInfo setOptionShowReticleTexture
    setOptionShowSurvivalGuide setOptionShowValidTargetReticleTexture setOptionSidebarSize
    setOptionSimpleClothingTextures setOptionSimpleWeaponTextures setOptionSingleContextMenu
    setOptionSoundVolume setOptionStreamerMode setOptionTextureCompression
    setOptionTieredZombieUpdates setOptionTimedActionGameSpeedReset setOptionTooltipFont
    setOptionUIFBO setOptionUIRenderFPS setOptionUpdateSneakButton setOptionUsePhysicsHitReaction
    setOptionVSync setOptionValidTargetReticleTextureIndex setOptionVehicleEngineVolume
    setOptionVoiceAGCMode setOptionVoiceEnable setOptionVoiceMode setOptionVoiceRecordDevice
    setOptionVoiceVADMode setOptionVoiceVolumeMic setOptionVoiceVolumePlayers
    setOptionWorldMapBrightness setOptionZoom setOptionZoomLevels1x setOptionZoomLevels2x
    setOptiondblTapJogToSprint setPerfPuddles setPerfReflections setPerfSkybox
    setPopulateServerListOnStart setResolutionAndFullScreen setSelectedMap
    setShowFirstTimeSearchTutorial setShowYourUsername setTargetColor setTermsOfServiceVersion
    setTestingMicrophone setToggleToAim setToggleToRun setToggleToSprint setTutorialDone
    setWorldItemHighlightColor zoomLevelsChanged zoomOptionChanged
";

    fn setup(core: Rc<RefCell<Core>>) -> Thread {
        let mut t = Thread::new(TableRef::new());
        pz_kahlua::stdlib::install(&mut t, None).unwrap();
        let env = t.globals().clone();
        let events = EventManager::register(&env, "Events", EVENT_NAMES);
        register(&mut t, Some(events), core);
        // `assert` lives in the game's stdlib.lua, which these tests don't load.
        run(&mut t, "function assert(v, msg) if not v then error(msg or 'assertion failed!', 2) end return v end");
        t
    }

    fn run(t: &mut Thread, src: &str) {
        let p = pz_kahlua::compiler::compile(src, Some("test"), Some("test"), None).unwrap();
        let f = Value::Closure(pz_kahlua::vm::LuaClosure::new(
            Rc::new(p),
            t.globals().clone(),
        ));
        if let Err((e, tr)) = t.pcall(&f, &[]) {
            panic!("lua error: {e:?}\n{tr}");
        }
    }

    fn run_err(t: &mut Thread, src: &str) -> String {
        let p = pz_kahlua::compiler::compile(src, Some("test"), Some("test"), None).unwrap();
        let f = Value::Closure(pz_kahlua::vm::LuaClosure::new(
            Rc::new(p),
            t.globals().clone(),
        ));
        match t.pcall(&f, &[]) {
            Ok(_) => panic!("expected an error"),
            Err((e, _)) => pz_kahlua::exec::java_value_of(&e),
        }
    }

    #[test]
    fn every_lua_called_method_exists() {
        let mut t = setup(Rc::new(RefCell::new(Core::new())));
        let list = TableRef::new();
        for (i, n) in LUA_CALLED.split_whitespace().enumerate() {
            list.set_int(i as i64 + 1, n);
        }
        assert_eq!(list.len(), 325);
        t.globals().set_str("names", list);
        run(
            &mut t,
            r#"
            local core = getCore()
            for i = 1, #names do
                assert(type(core[names[i]]) == "function", "missing Core method " .. names[i])
            end
            assert(Core.getInstance() == core)
            assert(getCore() == core)
            "#,
        );
    }

    #[test]
    fn getters_and_setters_through_get_core() {
        let core = Rc::new(RefCell::new(Core::new()));
        let mut t = setup(core.clone());
        run(
            &mut t,
            r#"
            local core = getCore()
            assert(core:getScreenWidth() == 1280 and core:getScreenHeight() == 720)
            assert(core:getGameMode() == "Sandbox")
            assert(core:getDebug() == false)
            assert(core:getVersion() == "42.21.0 4a0e9546ec")
            assert(Core.getTileScale() == 2)
            assert(Core.isLastStand() == false)

            assert(core:getOptionAmbientVolume() == 5)
            core:setOptionAmbientVolume(8)
            assert(core:getOptionAmbientVolume() == 8)
            core:setOptionAmbientVolume(11) -- out of range: ignored
            assert(core:getOptionAmbientVolume() == 8)
            core:setOptionAmbientVolume(3.9) -- Double.intValue
            assert(core:getOptionAmbientVolume() == 3)

            assert(core:isToggleToSprint() == true)
            core:setToggleToSprint(false)
            assert(core:isToggleToSprint() == false)

            assert(core:getOptionContextMenuFont() == "Medium")
            core:setOptionContextMenuFont("Large")
            assert(core:getOptionContextMenuFont() == "Large")
            core:setOptionContextMenuFont("Bogus")
            assert(core:getOptionContextMenuFont() == "Large")

            core:setOptionMinChatOpaque(0.5)
            assert(core:getOptionMinChatOpaque() == 0.5)
            core:setOptionMaxChatOpaque(0.7)
            assert(math.abs(core:getOptionMaxChatOpaque() - 0.7) < 1e-6)
            assert(core:getOptionMaxChatOpaque() ~= 0.7) -- float precision, like Java

            assert(core:getOptionWorldMapBrightness() == 1)
            core:setOptionWorldMapBrightness(0.25)
            assert(core:getOptionWorldMapBrightness() == 0.25)

            assert(core:getAutoZoom(0) == false)
            core:setAutoZoom(2, true)
            assert(core:getAutoZoom(2) == true)
            core:setOptionSingleContextMenu(1, true)
            assert(core:getOptionSingleContextMenu(1) == true)

            core:setOptionZoomLevels2x("50;75;125")
            assert(core:getOptionZoomLevels2x() == "50;75;125")
            local levels = core:getDefaultZoomLevels()
            assert(levels:size() == 10 and levels:get(0) == 250 and levels:get(9) == 25)

            assert(core:getOptionFontSizeReal() == 1)
            core:setResolutionAndFullScreen(1920, 1080, false)
            assert(core:getScreenWidth() == 1920)
            assert(core:getOptionFontSizeReal() == 2)

            assert(core:getOptionControllerButtonStyleString() == "XBOX")
            core:setOptionControllerButtonStyle(3)
            assert(core:getOptionControllerButtonStyleString() == "STEAMDECK")

            core:setGameMode("LastStand")
            assert(core:getGameMode() == "LastStand")

            core:setSelectedMap("Muldraugh, KY")
            assert(core:getSelectedMap() == "Muldraugh, KY")
            assert(core:getBlinkingMoodle() == nil)

            local t = {}
            assert(core:getOptionOnStartup("x") == nil)
            core:setOptionOnStartup("x", t)
            assert(core:getOptionOnStartup("x") == t)
            core:setOptionOnStartup("y", 5)
            assert(core:getOptionOnStartup("y") == 5)

            local modes = core:getScreenModes()
            assert(type(modes) == "table" and #modes == 0)
            assert(core:isZoomEnabled() == true)
            assert(core:getMaxZoom() == 1 and core:getMinZoom() == 1)
            core:initFBOs()
            assert(core:getMaxZoom() == 1.25 and core:getMinZoom() == 0.5)
            "#,
        );
        let c = core.borrow();
        assert_eq!(c.get_option_ambient_volume(), 3);
        assert!(!c.is_toggle_to_sprint());
        assert_eq!(c.get_auto_zoom(2), Some(true));
        assert_eq!(c.get_game_mode(), "LastStand");
        assert!(!c.add_zombie_on_cell_load);
        assert_eq!(c.width, 1920);
        assert!(c.get_option_on_startup("x").is_some());
    }

    #[test]
    fn colors_keys_and_objects() {
        let core = Rc::new(RefCell::new(Core::new()));
        let mut t = setup(core.clone());
        run(
            &mut t,
            r#"
            local core = getCore()
            local c = core:getObjectHighlitedColor()
            assert(math.abs(c:getR() - 0.98) < 1e-6 and math.abs(c:getB() - 0.11) < 1e-6 and c:getA() == 1)
            assert(core:getObjectHighlitedColor() == c) -- same object, like the Java field
            core:setObjectHighlitedColor(ColorInfo.new(0.25, 0.5, 0.75, 1))
            local c2 = core:getObjectHighlitedColor()
            assert(c2:getR() == 0.25 and c2:getG() == 0.5 and c2:getB() == 0.75)
            assert(tostring(c2) == "Color (0.25,0.5,0.75,1.0)")
            core:setMpTextColor(ColorInfo.new(0.5, 0.25, 1, 1))
            assert(core:getMpTextColor():getG() == 0.25)
            local ok = pcall(function() core:setTargetColor(nil) end)
            assert(not ok)

            core:addKeyBinding("Forward", 17, 200, false, false, false)
            core:addKeyBinding(KeybindId.SPRINT, 42, 0, false, false, false)
            assert(core:getKey("Forward") == 17)
            assert(core:getKey(KeybindId.FORWARD) == 17)
            assert(core:getAltKey(KeybindId.FORWARD) == 200)
            assert(core:isKey(KeybindId.FORWARD, 17))
            assert(core:isKey("Forward", 200))
            assert(not core:isKey("Forward", 18))
            assert(core:getKey("Sprint") == 42)
            assert(KeybindId.FORWARD:getId() == "Forward")
            assert(core:getKey("Nope") == 0)
            local kb = core:getKeyBinding("Forward")
            assert(kb:keyValue() == 17 and kb:name() == "Forward" and kb:altKey() == 200)
            assert(core:getKeyBinding(17):name() == "Forward")
            core:reinitKeyMaps()
            assert(core:getKey("Forward") == 0)

            local v = core:getGameVersion()
            assert(v:getMajor() == 42 and v:getMinor() == 21 and tostring(v) == "42.21")
            assert(v:isGreaterThan(core:getBreakModGameVersion()))

            local o = core:getOptionByIndex(0)
            assert(o:getName() == "reticleMode" and o:getType() == "integer")
            o:parse("1")
            assert(core:getOptionReticleMode() == 1)
            assert(core:getOptionCount() == 136)
            "#,
        );
        let msg = run_err(&mut t, "getCore():getAutoZoom(7)");
        assert!(msg.contains("IndexOutOfBoundsException"), "{msg}");
        assert_eq!(
            core.borrow_mut().get_object_highlited_color(),
            ColorInfo::new(0.25, 0.5, 0.75, 1.0)
        );
    }

    #[test]
    fn option_events_reach_lua() {
        let core = Rc::new(RefCell::new(Core::new()));
        let mut t = setup(core);
        run(
            &mut t,
            r#"
            log = {}
            Events.OptionGamepadBindingPresetChanged.Add(function(p) table.insert(log, "preset " .. p) end)
            Events.OptionControllerButtonStyleChanged.Add(function(s) table.insert(log, "style " .. s) end)
            Events.OnResolutionChange.Add(function(ow, oh, w, h)
                table.insert(log, "res " .. ow .. "x" .. oh .. " " .. w .. "x" .. h)
                -- re-entering Core from an event handler is fine
                assert(getCore():getScreenWidth() == w)
            end)
            local core = getCore()
            core:setOptionGamepadBindingPreset("Modern")
            core:setOptionGamepadBindingPreset("Modern") -- unchanged: no event
            core:setOptionControllerButtonStyle(2)
            core:setScreenSize(1600, 900)
            assert(#log == 3, #log)
            assert(log[1] == "preset Modern", log[1])
            assert(log[2] == "style PS4", log[2])
            assert(log[3] == "res 1280x720 1600x900", log[3])
            "#,
        );
    }

    #[test]
    fn save_options_from_lua() {
        let dir = std::env::temp_dir().join(format!(
            "pz-lua-api-core-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let mut c = Core::new();
        c.set_cache_dir(&dir);
        let core = Rc::new(RefCell::new(c));
        let mut t = setup(core.clone());
        run(
            &mut t,
            r#"
            local core = getCore()
            core:setOptionSoundVolume(4)
            core:setOptionCodeFontSize("Small")
            core:saveOptions()
            "#,
        );
        let text = std::fs::read_to_string(dir.join("options.ini")).unwrap();
        assert!(text.contains("\nsoundVolume=4\n"));
        assert!(text.contains("\ncodeFontSize=Small\n"));
        let mut c2 = Core::new();
        c2.set_cache_dir(&dir);
        assert!(c2.load_options().unwrap());
        assert_eq!(c2.get_option_sound_volume(), 4);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn shared_core_singleton() {
        let a = Rc::new(RefCell::new(Core::new()));
        install_core(a.clone());
        assert!(Rc::ptr_eq(&shared_core(), &a));
    }
}
