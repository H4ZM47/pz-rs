//! What the HUD widgets read from other workstreams, as small input traits
//! with no-op defaults. Each trait names the owner that implements it;
//! the engine installs the real ones with [`install`].

use std::cell::RefCell;

/// `IsoPlayer` statics the HUD reads. Owner: characters thread.
pub trait HudPlayers {
    /// `IsoPlayer.getInstance() != null`.
    fn has_player_instance(&self) -> bool {
        false
    }
    /// `IsoPlayer.numPlayers`.
    fn num_players(&self) -> i32 {
        1
    }
    /// `IsoPlayer.players[i] != null`.
    fn player_exists(&self, _i: usize) -> bool {
        false
    }
    /// `IsoPlayer.allPlayersDead()`.
    fn all_players_dead(&self) -> bool {
        false
    }
    /// `IsoPlayer.players[i]`'s position and draw offsets, for HUD
    /// widgets that follow the player on screen; `None` without a player.
    fn player_screen(&self, _i: usize) -> Option<PlayerScreen> {
        None
    }
    /// `IsoPlayer.players[i].isDead()`.
    /// `IsoPlayer.getPlayerIndex()`: the player being updated or
    /// rendered.
    fn player_index(&self) -> i32 {
        0
    }
    /// `IsoPlayer.players[i].getSquare()`'s x, y, z (`None`: no player or
    /// no square).
    fn player_square(&self, _i: usize) -> Option<(i32, i32, i32)> {
        None
    }
    /// `player.playerIndex` of a Lua `IsoPlayer`.
    fn player_index_of(&self, _player: &pz_kahlua::Value) -> Option<usize> {
        None
    }
    fn player_dead(&self, _i: usize) -> bool {
        false
    }
}

/// What `ActionProgressBar.updateScreenPos` reads from a player.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PlayerScreen {
    pub x: f32,
    pub y: f32,
    pub z: f32,
    /// `IsoGameCharacter.offsetX` / `offsetY`.
    pub offset_x: f32,
    pub offset_y: f32,
    /// `getUserNameHeight()`.
    pub user_name_height: i32,
}

/// `IsoCamera` / `Core` zoom as the HUD reads them. Owner: world-render
/// thread.
pub trait HudCamera {
    /// `IsoCamera.getOffX()` / `getOffY()` (of the player being rendered).
    fn off_x(&self) -> f32 {
        0.0
    }
    fn off_y(&self) -> f32 {
        0.0
    }
    /// `Core.getZoom(player)`.
    fn zoom(&self, _player: usize) -> f32 {
        1.0
    }
    /// `IsoCamera.cameras[player].getOffX()` / `getOffY()` (already
    /// truncated to whole pixels, as `PlayerCamera` returns them).
    fn camera_offset(&self, _player: usize) -> (f32, f32) {
        (0.0, 0.0)
    }
    /// `IsoCamera.cameras[player].fixJigglyModelsSquareX/Y`.
    fn fix_jiggly_square(&self, _player: usize) -> (f32, f32) {
        (0.0, 0.0)
    }
    /// `IsoCamera.frameState.playerIndex`: the player whose frame is being
    /// updated or rendered.
    fn frame_player_index(&self) -> usize {
        0
    }
}

/// `GameTime.instance` as the HUD uses it. Owner: characters thread.
pub trait HudGameTime {
    /// `setMultiplier(m)`.
    fn set_multiplier(&mut self, _m: f32) {}
    /// `getMultiplier()` (all factors).
    fn get_multiplier(&self) -> f32 {
        0.8
    }
    /// `getTrueMultiplier()`.
    fn get_true_multiplier(&self) -> f32 {
        1.0
    }
    /// `getRealworldSecondsSinceLastUpdate()`.
    fn realworld_seconds_since_last_update(&self) -> f32 {
        1.0 / 60.0
    }
    /// `getTimeOfDay()`, raw (the clock's AM/PM indicator reads this).
    fn time_of_day(&self) -> f32 {
        9.0
    }
    /// The time the clock's digits show (`Clock.timeDigits`):
    /// `getTimeOfDay()`, or `serverTimeOfDay` for an MP client while the
    /// server fast-forwards (`GameClient.client && GameClient.fastForward`).
    fn digits_time_of_day(&self) -> f32 {
        self.time_of_day()
    }
    /// `getDay()` (0-based).
    fn day(&self) -> i32 {
        0
    }
    /// `getMonth()` (0-based).
    fn month(&self) -> i32 {
        0
    }
}

/// `ClimateManager` as the HUD reads it. Owner: characters thread.
pub trait HudClimate {
    /// `getAirTemperatureForCharacter(players[player], false)` (Celsius).
    fn air_temperature_for_character(&self, _player: usize) -> f32 {
        0.0
    }
    /// `ClimateManager.getInstance().getFogIntensity()`.
    fn fog_intensity(&self) -> f32 {
        0.0
    }
}

/// An `AlarmClock` / `AlarmClockClothing` item as `Clock` reads it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AlarmClockItem {
    /// `AlarmClockClothing` (else `AlarmClock`).
    pub clothing: bool,
    /// `hasTag(ItemTag.DIGITAL)`.
    pub digital_tag: bool,
    /// `isDigital()`.
    pub is_digital: bool,
    pub alarm_set: bool,
    pub ringing: bool,
    /// `isWorn()` / `isEquipped()` (inventory items).
    pub worn: bool,
    pub equipped: bool,
}

/// Where an alarm clock is: in `getWornItems()` or `getInventory().getItems()`,
/// by its index among that list's alarm clocks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AlarmClockRef {
    Worn(usize),
    Inventory(usize),
}

/// A player's alarm clocks. Owner: world thread (items).
pub trait HudAlarmClocks {
    /// The alarm clocks among `players[player].getWornItems()`, in order.
    fn worn_clocks(&self, _player: usize) -> Vec<AlarmClockItem> {
        Vec::new()
    }
    /// The alarm clocks among `players[player].getInventory().getItems()`,
    /// in order.
    fn inventory_clocks(&self, _player: usize) -> Vec<AlarmClockItem> {
        Vec::new()
    }
    /// `stopRinging()`.
    fn stop_ringing(&mut self, _player: usize, _item: AlarmClockRef) {}
    /// `setAlarmSet(set)` then `syncAlarmClock()`.
    fn set_alarm_set(&mut self, _player: usize, _item: AlarmClockRef, _set: bool) {}
}

/// A player's `Moodles` as `MoodlesUI` reads them, by moodle type name
/// (`MoodleType` registry id without namespace, e.g. `Endurance`). Owner:
/// characters thread.
pub trait HudMoodles {
    /// `getMoodles().UI_RefreshNeeded()` (true once per state change; it
    /// clears the flag).
    fn ui_refresh_needed(&mut self, _player: usize) -> bool {
        false
    }
    /// `getMoodles().setMoodlesStateChanged(changed)`.
    fn set_moodles_state_changed(&mut self, _player: usize, _changed: bool) {}
    /// `getMoodleLevel(type)` (0..4).
    fn moodle_level(&self, _player: usize, _moodle: &str) -> i32 {
        0
    }
    /// `getGoodBadNeutral(type)`: 0 neutral, 1 good, 2 bad.
    fn good_bad_neutral(&self, _player: usize, _moodle: &str) -> i32 {
        0
    }
    /// `getMoodleDisplayString(type)`.
    fn display_string(&self, _player: usize, _moodle: &str) -> String {
        String::new()
    }
    /// `getMoodleDescriptionString(type)`.
    fn description_string(&self, _player: usize, _moodle: &str) -> String {
        String::new()
    }
}

/// The object under the mouse (`UIManager.getPicked().tile`). Owner: world
/// thread (picking).
pub trait HudPicking {
    /// The picked tile object, if any.
    fn picked_tile(&self) -> pz_kahlua::LuaResult<Option<pz_ui::tooltip::TooltipObject>> {
        Ok(None)
    }
    /// An `IsoGridSquare` Lua object's x, y, z.
    fn square_coords(&self, _square: &pz_kahlua::Value) -> Option<(i32, i32, i32)> {
        None
    }
}

/// One `BodyPart` as `UI_BodyPart` reads it.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct BodyPartView {
    /// `bandaged()` (also `BodyDamage.IsBandaged(type)`).
    pub bandaged: bool,
    pub bandage_life: f32,
    pub wound_infection_level: f32,
    /// `scratched()`.
    pub scratched: bool,
    pub scratch_time: f32,
    /// `getCutTime()` (`IsCut` is `cutTime > 0`).
    pub cut_time: f32,
    /// `bitten()`.
    pub bitten: bool,
    pub bite_time: f32,
    /// `deepWounded()`.
    pub deep_wounded: bool,
    pub deep_wound_time: f32,
    /// `stitched()`.
    pub stitched: bool,
    pub fracture_time: f32,
    pub splint_factor: f32,
    pub have_glass: bool,
    pub have_bullet: bool,
    pub burn_time: f32,
}

/// A character's `BodyDamage` as the health panel reads it; the character
/// is its Lua object (`IsoGameCharacter`). Implementations pick
/// `getBodyDamageRemote()` for a remote MP player. Owner: characters
/// thread.
pub trait HudBodyDamage {
    /// `isFemale()`.
    fn is_female(&self, _chr: &pz_kahlua::Value) -> bool {
        false
    }
    /// `getBodyDamage().getHealth()`.
    fn health(&self, _chr: &pz_kahlua::Value) -> f32 {
        100.0
    }
    /// `getBodyDamage().getBodyPart(type)` by `BodyPartType.ToIndex`.
    fn body_part(&self, _chr: &pz_kahlua::Value, _index: usize) -> BodyPartView {
        BodyPartView::default()
    }
}

/// `DebugOptions` the HUD reads. Owner: us (debug options).
pub trait HudDebug {
    /// `DebugOptions.instance.cheat.clock.visible`.
    fn cheat_clock_visible(&self) -> bool {
        false
    }
    /// `Core.debug && DebugOptions.instance.uiRenderOutline`.
    fn ui_render_outline(&self) -> bool {
        false
    }
    /// `DebugOptions.instance.uiDebugConsoleStartVisible`.
    fn ui_debug_console_start_visible(&self) -> bool {
        false
    }
    /// `DebugOptions.instance.uiDebugConsoleEchoCommand`.
    fn ui_debug_console_echo_command(&self) -> bool {
        false
    }
    /// `DebugOptions.instance.uiDebugConsoleDebugLog`.
    fn ui_debug_console_debug_log(&self) -> bool {
        false
    }
}

/// [`HudDebug`] over the shared `DebugOptions.instance`
/// (`pz_core::debug::debug_options`).
pub struct DebugOptionsHud;

impl DebugOptionsHud {
    fn option(name: &str) -> bool {
        pz_core::debug::debug_options()
            .try_borrow()
            .is_ok_and(|o| o.get_boolean(name))
    }
}

impl HudDebug for DebugOptionsHud {
    fn cheat_clock_visible(&self) -> bool {
        Self::option("Cheat.Clock.Visible")
    }
    fn ui_render_outline(&self) -> bool {
        let debug = crate::bind_core::shared_core().try_borrow().is_ok_and(|c| c.debug);
        debug && Self::option("UI.Render.Outline")
    }
    fn ui_debug_console_start_visible(&self) -> bool {
        Self::option("UI.DebugConsole.StartVisible")
    }
    fn ui_debug_console_echo_command(&self) -> bool {
        Self::option("UI.DebugConsole.EchoCommand")
    }
    fn ui_debug_console_debug_log(&self) -> bool {
        Self::option("UI.DebugConsole.DebugLog")
    }
}

/// Put [`DebugOptionsHud`] in the HUD host (run by `register_all`).
pub fn install_debug_options_host() {
    with_host(|h| h.debug = Box::new(DebugOptionsHud));
}

/// A Java method as the debug console's suggestion engine sees it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ConsoleMethod {
    pub name: String,
    /// `getReturnType().toString()`: `void`, `int`, `class a.b.C`, ...
    pub return_type: String,
    /// `getParameterTypes().length`.
    pub param_count: usize,
}

/// The reflection the debug console's suggestions use. Owner: us (the
/// Lua API's exposure tables).
pub trait HudConsoleMethods {
    /// `LuaManager.GlobalObject.class.getDeclaredMethods()`, in that order.
    fn global_methods(&self) -> Vec<ConsoleMethod> {
        Vec::new()
    }
    /// The public declared methods of `return_type` (as
    /// [`ConsoleMethod::return_type`]) and each superclass, class first.
    fn public_methods_of(&self, _return_type: &str) -> Vec<ConsoleMethod> {
        Vec::new()
    }
}

/// [`HudConsoleMethods`] backed by `pz-lua-api`'s exposure tables and, when
/// the installed `projectzomboid.jar` is present, enriched with exact return
/// types and parameter counts from `pz-classfile` (lazily on first use).
pub struct ExposedConsoleMethods {
    jar_path: Option<std::path::PathBuf>,
    jar: RefCell<Option<Option<pz_classfile::Jar>>>,
    class_cache: RefCell<std::collections::HashMap<String, Vec<ConsoleMethod>>>,
}

impl ExposedConsoleMethods {
    pub fn new(jar_path: Option<std::path::PathBuf>) -> Self {
        ExposedConsoleMethods {
            jar_path,
            jar: RefCell::new(None),
            class_cache: RefCell::new(std::collections::HashMap::new()),
        }
    }

    fn with_jar<R>(&self, f: impl FnOnce(Option<&pz_classfile::Jar>) -> R) -> R {
        let mut slot = self.jar.borrow_mut();
        if slot.is_none() {
            let opened = self
                .jar_path
                .as_deref()
                .and_then(|p| pz_classfile::Jar::open(p).ok());
            *slot = Some(opened);
        }
        f(slot.as_ref().and_then(|o| o.as_ref()))
    }
}

/// Parse a JVM method descriptor `(<params>)<ret>` into Java's
/// `Class.toString()` return type (`"void"`, `"int"`, `"class a.b.C"`, ...)
/// and parameter count.
fn parse_jvm_method_descriptor(desc: &str) -> Option<(String, usize)> {
    let bytes = desc.as_bytes();
    if *bytes.first()? != b'(' {
        return None;
    }
    let mut i = 1usize;
    let mut params = 0usize;
    while i < bytes.len() && bytes[i] != b')' {
        while i < bytes.len() && bytes[i] == b'[' {
            i += 1;
        }
        match *bytes.get(i)? {
            b'B' | b'C' | b'D' | b'F' | b'I' | b'J' | b'S' | b'Z' => {
                i += 1;
                params += 1;
            }
            b'L' => {
                i += 1;
                while i < bytes.len() && bytes[i] != b';' {
                    i += 1;
                }
                if i >= bytes.len() {
                    return None;
                }
                i += 1;
                params += 1;
            }
            _ => return None,
        }
    }
    if bytes.get(i) != Some(&b')') {
        return None;
    }
    let ret = &desc[i + 1..];
    let return_type = match ret.as_bytes().first()? {
        b'V' => "void".to_string(),
        b'Z' => "boolean".to_string(),
        b'B' => "byte".to_string(),
        b'C' => "char".to_string(),
        b'S' => "short".to_string(),
        b'I' => "int".to_string(),
        b'J' => "long".to_string(),
        b'F' => "float".to_string(),
        b'D' => "double".to_string(),
        b'L' => {
            let inner = ret.strip_prefix('L')?.strip_suffix(';')?;
            format!("class {}", inner.replace('/', "."))
        }
        b'[' => format!("class {}", ret.replace('/', ".")),
        _ => return None,
    };
    Some((return_type, params))
}

fn methods_from_classfile(
    cf: &pz_classfile::ClassFile,
    public_only: bool,
) -> Vec<ConsoleMethod> {
    const ACC_PUBLIC: u16 = 0x0001;
    const ACC_BRIDGE: u16 = 0x0040;
    const ACC_SYNTHETIC: u16 = 0x1000;
    let mut out = Vec::new();
    for m in &cf.methods {
        if m.name == "<init>" || m.name == "<clinit>" {
            continue;
        }
        if m.access & (ACC_SYNTHETIC | ACC_BRIDGE) != 0 {
            continue;
        }
        if public_only && (m.access & ACC_PUBLIC == 0) {
            continue;
        }
        let Some((return_type, param_count)) = parse_jvm_method_descriptor(&m.descriptor) else {
            continue;
        };
        out.push(ConsoleMethod {
            name: m.name.clone(),
            return_type,
            param_count,
        });
    }
    out
}

impl HudConsoleMethods for ExposedConsoleMethods {
    fn global_methods(&self) -> Vec<ConsoleMethod> {
        let mut out = self.with_jar(|jar| {
            jar.and_then(|j| pz_classfile::load_class(j, "zombie/Lua/LuaManager$GlobalObject").ok())
                .map(|cf| methods_from_classfile(&cf, false))
                .unwrap_or_default()
        });
        for name in crate::expose::exposed_globals() {
            if !out.iter().any(|m| m.name == name) {
                out.push(ConsoleMethod {
                    name,
                    return_type: "void".to_string(),
                    param_count: 0,
                });
            }
        }
        out
    }

    fn public_methods_of(&self, return_type: &str) -> Vec<ConsoleMethod> {
        let fqn = match return_type.strip_prefix("class ") {
            Some(s) if !s.starts_with('[') => s,
            None if return_type.contains('.') => return_type,
            _ => return Vec::new(),
        };
        if let Some(cached) = self.class_cache.borrow().get(fqn) {
            return cached.clone();
        }
        let mut out = self.with_jar(|jar| {
            let Some(jar) = jar else {
                return Vec::new();
            };
            let mut methods = Vec::new();
            let mut cur = Some(fqn.replace('.', "/"));
            let mut hops = 0usize;
            while let Some(internal) = cur {
                if hops > 32 || internal == "java/lang/Object" {
                    break;
                }
                hops += 1;
                let Ok(cf) = pz_classfile::load_class(jar, &internal) else {
                    break;
                };
                methods.extend(methods_from_classfile(&cf, true));
                cur = cf.super_class;
            }
            methods
        });
        for name in crate::expose::exposed_class_methods(fqn) {
            if !out.iter().any(|m| m.name == name) {
                out.push(ConsoleMethod {
                    name,
                    return_type: "void".to_string(),
                    param_count: 0,
                });
            }
        }
        self.class_cache
            .borrow_mut()
            .insert(fqn.to_string(), out.clone());
        out
    }
}

/// Put [`ExposedConsoleMethods`] in the HUD host (run by `register_all`).
pub fn install_console_methods_host(jar_path: Option<std::path::PathBuf>) {
    with_host(|h| h.console_methods = Box::new(ExposedConsoleMethods::new(jar_path)));
}

/// `SoundManager.instance` calls the HUD makes. Owner: audio thread.
pub trait HudSound {
    fn pause_sound_and_music(&mut self, _keep_music: bool) {}
    fn resume_sound_and_music(&mut self) {}
    fn set_music_state(&mut self, _state: &str) {}
    fn play_ui_sound(&mut self, _name: &str) {}
}

/// `GameClient.client` / `GameServer.server`. Owner: network thread.
pub trait HudNetwork {
    fn is_client(&self) -> bool {
        false
    }
    fn is_server(&self) -> bool {
        false
    }
    /// The debug console may run commands: `!GameClient.client`, or the
    /// player's role has `Capability.UIManagerProcessCommands`, or the
    /// connection is the coop host.
    fn may_process_console_commands(&self) -> bool {
        !self.is_client()
    }
}

/// `GameClient.client` / `GameServer.server` as Core mirrors them. In this
/// single-player port both stay false unless a network layer sets them.
pub struct CoreNetwork;

impl HudNetwork for CoreNetwork {
    fn is_client(&self) -> bool {
        crate::bind_core::shared_core().borrow().is_client
    }
    fn is_server(&self) -> bool {
        crate::bind_core::shared_core().borrow().is_server
    }
}

/// The no-op defaults (sound defaults to the shared sound system).
pub struct NoHud;
impl HudPlayers for NoHud {}
impl HudCamera for NoHud {}
impl HudClimate for NoHud {}
impl HudAlarmClocks for NoHud {}
impl HudDebug for NoHud {}
impl HudMoodles for NoHud {}
impl HudPicking for NoHud {}
impl HudBodyDamage for NoHud {}
impl HudConsoleMethods for NoHud {}
impl HudGizmoCamera for NoHud {}
impl HudSound for NoHud {}
impl HudNetwork for NoHud {}

/// [`HudGameTime`] over the `GameTime` the Lua bindings share
/// (`bind_climate::shared_game_time`), the default until the engine
/// installs its own.
pub struct SharedGameTime;

impl HudGameTime for SharedGameTime {
    fn set_multiplier(&mut self, m: f32) {
        crate::bind_climate::shared_game_time()
            .borrow_mut()
            .multiplier = m;
    }
    fn get_multiplier(&self) -> f32 {
        let ctx = crate::bind_climate::LuaClimateContext::new();
        crate::bind_climate::shared_game_time()
            .borrow()
            .get_multiplier(&ctx)
    }
    fn get_true_multiplier(&self) -> f32 {
        crate::bind_climate::shared_game_time()
            .borrow()
            .get_true_multiplier()
    }
    fn realworld_seconds_since_last_update(&self) -> f32 {
        crate::bind_climate::shared_game_time()
            .borrow()
            .realworld_seconds_since_last_update()
    }
    fn time_of_day(&self) -> f32 {
        crate::bind_climate::shared_game_time().borrow().time_of_day
    }
    fn digits_time_of_day(&self) -> f32 {
        use pz_core::game_time::GameTimeContext;
        let ctx = crate::bind_climate::LuaClimateContext::new();
        let client = crate::bind_core::shared_core().borrow().is_client;
        let gt = crate::bind_climate::shared_game_time();
        let g = gt.borrow();
        if client && ctx.fast_forward() {
            g.server_time_of_day
        } else {
            g.time_of_day
        }
    }
    fn day(&self) -> i32 {
        crate::bind_climate::shared_game_time().borrow().day
    }
    fn month(&self) -> i32 {
        crate::bind_climate::shared_game_time().borrow().month
    }
}

/// The camera values the gizmo scene reads. Owner: world-render thread.
pub trait HudGizmoCamera {
    /// `IsoCamera.frameState` / `IsoCamera.cameras[0]`, as
    /// [`crate::gizmo::GizmoCamera`] lists them.
    fn gizmo_camera(&self) -> crate::gizmo::GizmoCamera {
        crate::gizmo::GizmoCamera::default()
    }
}

/// The installed inputs.
pub struct HudHost {
    pub players: Box<dyn HudPlayers>,
    pub game_time: Box<dyn HudGameTime>,
    pub sound: Box<dyn HudSound>,
    pub network: Box<dyn HudNetwork>,
    pub camera: Box<dyn HudCamera>,
    pub climate: Box<dyn HudClimate>,
    pub alarm_clocks: Box<dyn HudAlarmClocks>,
    pub debug: Box<dyn HudDebug>,
    pub moodles: Box<dyn HudMoodles>,
    pub picking: Box<dyn HudPicking>,
    pub body_damage: Box<dyn HudBodyDamage>,
    pub console_methods: Box<dyn HudConsoleMethods>,
    pub gizmo_camera: Box<dyn HudGizmoCamera>,
}

impl Default for HudHost {
    fn default() -> Self {
        HudHost {
            players: Box::new(NoHud),
            game_time: Box::new(SharedGameTime),
            sound: Box::new(crate::bind_audio::SharedHudSound),
            network: Box::new(CoreNetwork),
            camera: Box::new(NoHud),
            climate: Box::new(NoHud),
            alarm_clocks: Box::new(NoHud),
            debug: Box::new(NoHud),
            moodles: Box::new(NoHud),
            picking: Box::new(NoHud),
            body_damage: Box::new(NoHud),
            console_methods: Box::new(NoHud),
            gizmo_camera: Box::new(NoHud),
        }
    }
}

thread_local! {
    static HOST: RefCell<HudHost> = RefCell::new(HudHost::default());
}

/// Replace the HUD inputs.
pub fn install(h: HudHost) {
    HOST.with(|s| *s.borrow_mut() = h);
}

/// Run `f` on the installed inputs. Implementations must not call Lua.
pub fn with_host<R>(f: impl FnOnce(&mut HudHost) -> R) -> R {
    HOST.with(|s| f(&mut s.borrow_mut()))
}

/// [`with_host`], or `None` while the host is already borrowed (a log
/// write from inside a host call).
pub fn try_with_host<R>(f: impl FnOnce(&mut HudHost) -> R) -> Option<R> {
    HOST.with(|s| s.try_borrow_mut().ok().map(|mut h| f(&mut h)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_jvm_method_descriptors() {
        assert_eq!(
            parse_jvm_method_descriptor("()Lzombie/characters/IsoPlayer;"),
            Some(("class zombie.characters.IsoPlayer".to_string(), 0))
        );
        assert_eq!(
            parse_jvm_method_descriptor("(IF[Ljava/lang/String;Z)V"),
            Some(("void".to_string(), 4))
        );
        assert_eq!(
            parse_jvm_method_descriptor("(D)[I"),
            Some(("class [I".to_string(), 1))
        );
        assert_eq!(
            parse_jvm_method_descriptor("()I"),
            Some(("int".to_string(), 0))
        );
    }

    #[test]
    fn exposed_console_methods_lists_globals_and_inherited_class_methods() {
        let mut t = pz_kahlua::Thread::new(pz_kahlua::TableRef::new());
        crate::expose::global(&mut t, "testConsoleGlobalFn", |_| Ok(0));
        let base = crate::expose::expose_class(&mut t, "test.console.BaseClass", None);
        base.method("baseMethod", |_| Ok(0));
        let sub = crate::expose::expose_class(
            &mut t,
            "test.console.SubClass",
            Some("test.console.BaseClass"),
        );
        sub.method("subMethod", |_| Ok(0));

        let host = ExposedConsoleMethods::new(None);
        assert!(host
            .global_methods()
            .iter()
            .any(|m| m.name == "testConsoleGlobalFn"));
        let names: Vec<String> = host
            .public_methods_of("class test.console.SubClass")
            .into_iter()
            .map(|m| m.name)
            .collect();
        assert_eq!(names, vec!["subMethod", "baseMethod"]);
        assert!(host.public_methods_of("void").is_empty());
    }
}
