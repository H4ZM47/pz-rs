//! `zombie.core.Core`: the global settings/options object.
//!
//! Holds every `options.ini` option (with its original default, range and
//! name), the screen size, game mode, debug flags, version strings, key
//! bindings and the rest of the state the Java singleton keeps outside the
//! in-game world. The Java static fields (`Core.width`, `Core.gameMode`,
//! `Core.debug`, ...) are plain fields here: the engine owns one `Core`
//! (shared with Lua as `Rc<RefCell<Core>>`, see `pz_lua_api::bind_core`).
//!
//! Loading and saving `options.ini` (`loadOptions` / `saveOptions`) follow
//! the original byte for byte: `ConfigFile` reading, version migration
//! (`upgradeOptionName` / `upgradeOptionValue`), fake options, the
//! case-insensitive name sort, `macos*` options only written on macOS, and
//! Java number formatting. The file location is a parameter: set it with
//! [`Core::set_cache_dir`] (the Zomboid user dir, `options.ini` inside it,
//! like `ZomboidFileSystem.getCacheDirSub("options.ini")`) or
//! [`Core::set_options_path`]. Without one, `loadOptions` behaves as if the
//! file does not exist and `saveOptions` writes nothing.
//!
//! Rendering (OpenGL, FBOs, shaders, screenshots, display modes) is left out
//! and marked `TODO(render)`. Calls into other singletons the Java code makes
//! (SoundManager, VoiceManager, JoypadManager, PerformanceSettings,
//! TextManager, ...) are either kept as state on `Core` for those systems to
//! read (`performance`, `active_controller_guids`, `voice`, ...) or queued as
//! [`CoreEvent`]s for the Lua events Java triggers from here.

use crate::config::{
    case_insensitive_cmp, debug_log, float_to_string, java_lines, java_split, java_trim,
    ConfigFile, ConfigObject, ConfigOption, OnChange,
};
use pz_kahlua::number::java_parse_int;
use std::any::Any;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::rc::Rc;

/// `zombie.core.GameVersion`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GameVersion {
    pub major: i32,
    pub minor: i32,
    pub suffix: String,
}

impl GameVersion {
    pub fn new(major: i32, minor: i32, suffix: &str) -> Self {
        assert!(major >= 0, "major version must be greater than zero");
        assert!(
            (0..=999).contains(&minor),
            "minor version must be from 0 to 999"
        );
        GameVersion {
            major,
            minor,
            suffix: suffix.to_string(),
        }
    }
    pub fn get_int(&self) -> i32 {
        self.major * 1000 + self.minor
    }
    pub fn is_greater_than(&self, rhs: &GameVersion) -> bool {
        self.get_int() > rhs.get_int()
    }
    pub fn is_greater_than_or_equal_to(&self, rhs: &GameVersion) -> bool {
        self.get_int() >= rhs.get_int()
    }
    pub fn is_less_than(&self, rhs: &GameVersion) -> bool {
        self.get_int() < rhs.get_int()
    }
    pub fn is_less_than_or_equal_to(&self, rhs: &GameVersion) -> bool {
        self.get_int() <= rhs.get_int()
    }
    /// `GameVersion.parse`: `([0-9]+)\.([0-9]+)(.*)`.
    pub fn parse(s: &str) -> Option<GameVersion> {
        let (maj, rest) = s.split_once('.')?;
        if maj.is_empty() || !maj.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        let n = rest.bytes().take_while(|b| b.is_ascii_digit()).count();
        if n == 0 {
            return None;
        }
        let major = java_parse_int(maj, 10).unwrap_or(0);
        let minor = java_parse_int(&rest[..n], 10).unwrap_or(0);
        if !(0..=999).contains(&minor) {
            return None;
        }
        Some(GameVersion::new(major, minor, &rest[n..]))
    }
}

impl std::fmt::Display for GameVersion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{}{}", self.major, self.minor, self.suffix)
    }
}

/// `zombie.core.textures.ColorInfo` (the value part).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ColorInfo {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

impl Default for ColorInfo {
    fn default() -> Self {
        ColorInfo {
            r: 1.0,
            g: 1.0,
            b: 1.0,
            a: 1.0,
        }
    }
}

impl ColorInfo {
    pub fn new(r: f32, g: f32, b: f32, a: f32) -> Self {
        ColorInfo { r, g, b, a }
    }
    fn from_array(c: [f32; 4]) -> Self {
        ColorInfo {
            r: c[0],
            g: c[1],
            b: c[2],
            a: c[3],
        }
    }
}

impl std::fmt::Display for ColorInfo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Color ({},{},{},{})",
            float_to_string(self.r),
            float_to_string(self.g),
            float_to_string(self.b),
            float_to_string(self.a)
        )
    }
}

/// `Core.KeyBinding` record.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeyBinding {
    pub name: String,
    pub key_value: i32,
    pub alt_key: i32,
    pub shift: bool,
    pub ctrl: bool,
    pub alt: bool,
}

impl KeyBinding {
    /// `Core.KEYBINDING_EMPTY`.
    pub fn empty() -> Self {
        KeyBinding {
            name: "empty".into(),
            key_value: 0,
            alt_key: 0,
            shift: false,
            ctrl: false,
            alt: false,
        }
    }
}

impl std::fmt::Display for KeyBinding {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "KeyBinding[name={}, keyValue={}, altKey={}, shift={}, ctrl={}, alt={}]",
            self.name, self.key_value, self.alt_key, self.shift, self.ctrl, self.alt
        )
    }
}

/// The parts of `zombie.core.PerformanceSettings` `Core` reads and writes
/// when loading/saving options.
#[derive(Clone, Debug)]
pub struct PerformanceSettings {
    pub lock_fps: i32,
    pub uncapped_fps: bool,
    pub water_quality: i32,
    pub puddles_quality: i32,
    pub lighting_fps: i32,
    pub fog_quality: i32,
    pub view_cone_opacity: i32,
}

impl Default for PerformanceSettings {
    fn default() -> Self {
        PerformanceSettings {
            lock_fps: 60,
            uncapped_fps: false,
            water_quality: 0,
            puddles_quality: 0,
            lighting_fps: 15,
            fog_quality: 0,
            view_cone_opacity: 3,
        }
    }
}

impl PerformanceSettings {
    /// `setLightingFPS`: clamped to 1..=120.
    pub fn set_lighting_fps(&mut self, fps: i32) {
        self.lighting_fps = fps.clamp(1, 120);
        debug_log(&format!("LightingFPS set to {}", self.lighting_fps));
    }
}

/// `zombie.core.textures.MultiTextureFBO2` without the GL parts: zoom
/// levels and per-player zoom state. `created` stands in for
/// `current != null` (the FBO exists).
#[derive(Clone, Debug)]
pub struct OffscreenBuffer {
    pub zoom_levels: Option<Vec<f32>>,
    pub zoom: [f32; 4],
    pub target_zoom: [f32; 4],
    pub start_zoom: [f32; 4],
    pub zoomed_in_level: f32,
    pub zoomed_out_level: f32,
    pub auto_zoom: [bool; 4],
    pub zoom_enabled: bool,
    /// TODO(render): stands in for the `TextureFBO current`.
    pub created: bool,
}

/// `MultiTextureFBO2.zoomLevelsDefault`.
pub const ZOOM_LEVELS_DEFAULT: [f32; 10] = [2.5, 2.25, 2.0, 1.75, 1.5, 1.25, 1.0, 0.75, 0.5, 0.25];

/// Java `Math.round(float)`.
fn round_f32(f: f32) -> i32 {
    (f + 0.5).floor() as i32
}

impl Default for OffscreenBuffer {
    fn default() -> Self {
        OffscreenBuffer {
            zoom_levels: None,
            zoom: [1.0; 4],
            target_zoom: [1.0; 4],
            start_zoom: [1.0; 4],
            zoomed_in_level: 0.0,
            zoomed_out_level: 0.0,
            auto_zoom: [false; 4],
            zoom_enabled: true,
            created: false,
        }
    }
}

impl OffscreenBuffer {
    pub fn get_display_zoom(&self, player: usize, width: i32, initial_width: f32) -> f32 {
        if width as f32 > initial_width {
            return self.zoom[player] * (initial_width / width as f32);
        }
        self.zoom[player]
    }

    pub fn set_target_zoom(&mut self, player: usize, target: f32) {
        if self.target_zoom[player] != target {
            self.target_zoom[player] = target;
            self.start_zoom[player] = self.zoom[player];
        }
    }

    pub fn get_default_zoom_levels(&self) -> Vec<i32> {
        ZOOM_LEVELS_DEFAULT
            .iter()
            .map(|l| round_f32(l * 100.0))
            .collect()
    }

    /// `setZoomLevelsFromOption`.
    pub fn set_zoom_levels_from_option(&mut self, levels: Option<&str>) {
        self.zoom_levels = Some(ZOOM_LEVELS_DEFAULT.to_vec());
        let Some(levels) = levels else { return };
        if levels.is_empty() {
            return;
        }
        let ss = java_split(levels, ";");
        if ss.is_empty() {
            return;
        }
        let mut percents: Vec<i32> = Vec::new();
        for s in ss {
            if s.is_empty() {
                continue;
            }
            let Some(percent) = java_parse_int(s, 10) else {
                continue;
            };
            for known in ZOOM_LEVELS_DEFAULT {
                if round_f32(known * 100.0) != percent {
                    continue;
                }
                if !percents.contains(&percent) {
                    percents.push(percent);
                }
                break;
            }
        }
        if !percents.contains(&100) {
            percents.push(100);
        }
        percents.sort_by(|a, b| b.cmp(a));
        self.zoom_levels = Some(percents.iter().map(|p| *p as f32 / 100.0).collect());
    }

    /// `destroy()` (GL part TODO(render)).
    pub fn destroy(&mut self) {
        if !self.created {
            return;
        }
        self.created = false;
        self.target_zoom = [1.0; 4];
        self.zoom = [1.0; 4];
    }

    /// `create(xres, yres)` without the texture (TODO(render)).
    pub fn create(&mut self) {
        let levels = self
            .zoom_levels
            .get_or_insert_with(|| ZOOM_LEVELS_DEFAULT.to_vec());
        self.zoomed_in_level = *levels.last().unwrap_or(&1.0);
        self.zoomed_out_level = *levels.first().unwrap_or(&1.0);
        self.created = true;
    }

    pub fn do_zoom_scroll(&mut self, player: usize, del: i32) {
        self.target_zoom[player] = self.get_next_zoom(player, del);
    }

    pub fn get_next_zoom(&self, player: usize, del: i32) -> f32 {
        let Some(levels) = self.zoom_levels.as_ref().filter(|_| self.zoom_enabled) else {
            return 1.0;
        };
        if del > 0 {
            for i in (1..levels.len()).rev() {
                if self.target_zoom[player] == levels[i] {
                    return levels[i - 1];
                }
            }
        } else if del < 0 {
            for i in 0..levels.len().saturating_sub(1) {
                if self.target_zoom[player] == levels[i] {
                    return levels[i + 1];
                }
            }
        }
        self.target_zoom[player]
    }

    pub fn get_min_zoom(&self) -> f32 {
        match &self.zoom_levels {
            Some(l) if self.zoom_enabled && !l.is_empty() => l[l.len() - 1],
            _ => 1.0,
        }
    }

    pub fn get_max_zoom(&self) -> f32 {
        match &self.zoom_levels {
            Some(l) if self.zoom_enabled && !l.is_empty() => l[0],
            _ => 1.0,
        }
    }
}

/// The parts of `VoiceManager` / FMOD that `Core`'s voice getters read.
#[derive(Clone, Debug)]
pub struct VoiceState {
    /// `VoiceManager.voipDisabled`.
    pub voip_disabled: bool,
    pub mic_volume_indicator: i32,
    pub mic_volume_error: bool,
    pub server_voip_enable: bool,
    pub testing_microphone: bool,
    /// FMOD record driver names, in driver order.
    pub record_driver_names: Vec<String>,
}

impl Default for VoiceState {
    fn default() -> Self {
        // No FMOD receive buffer: indicator 0, error true.
        VoiceState {
            voip_disabled: false,
            mic_volume_indicator: 0,
            mic_volume_error: true,
            server_voip_enable: true,
            testing_microphone: false,
            record_driver_names: Vec::new(),
        }
    }
}

/// `Core.currentTextEntryBox` as far as `Core` looks at it.
#[derive(Clone, Debug, Default)]
pub struct TextEntryInfo {
    pub ui_name: String,
    pub editable: bool,
    pub doing_text_entry: bool,
}

/// An argument of a queued [`CoreEvent`].
#[derive(Clone, Debug, PartialEq)]
pub enum EventArg {
    Num(f64),
    Str(String),
}

/// A Lua event Java `Core` triggers through `LuaEventManager.triggerEvent`.
/// The Lua binding drains and triggers them after each call.
#[derive(Clone, Debug, PartialEq)]
pub struct CoreEvent {
    pub name: &'static str,
    pub args: Vec<EventArg>,
}

/// Something the engine has to do on behalf of a `Core` call that cannot be
/// done from inside it (process exit, Lua reset, display mode change).
#[derive(Clone, Debug, PartialEq)]
pub enum CoreRequest {
    /// `Core.quit()` with no player: options were saved; `System.exit(0)`.
    Exit,
    /// `ResetLua(activeMods, reason)`.
    ResetLua { active_mods: String, reason: String },
    /// `setDisplayMode(width, height, fullscreen)`. TODO(render).
    DisplayMode {
        width: i32,
        height: i32,
        fullscreen: bool,
    },
    /// `setVSyncEnabled`. TODO(render).
    VSync(bool),
    /// `TakeFullScreenshot(filename)`. TODO(render).
    FullScreenshot(Option<String>),
    /// `TakeScreenshot(...)` of the current save's thumbnail. TODO(render).
    Screenshot {
        x: i32,
        y: i32,
        width: i32,
        height: i32,
        read_buffer: i32,
    },
}

/// The game version this port reports (`Core.gameVersion`).
pub const GAME_VERSION: (i32, i32, &str) = (42, 21, "");
/// The release git revision string baked into the build.
pub const GIT_REVISION: &str = "4a0e9546ec";

/// `zombie.core.Core`.
pub struct Core {
    options: Vec<ConfigOption>,
    fake_options: Vec<ConfigOption>,
    option_by_name: HashMap<String, usize>,
    fake_option_by_name: HashMap<String, usize>,

    // ---- file locations (ZomboidFileSystem stand-ins) ----
    cache_dir: Option<PathBuf>,
    options_path: Option<PathBuf>,
    /// Directory `initOptionsINI` copies `mods` from (the Java CWD).
    pub game_dir: Option<PathBuf>,

    // ---- Java statics ----
    pub is_dev: bool,
    pub tutorial: bool,
    pub dirty_global_lights_count: i32,
    pub alt_move_method: bool,
    pub option_mods_enabled: bool,
    pub difficulty: String,
    pub tile_scale: i32,
    pub blink_alpha: f32,
    pub blink_alpha_increase: bool,
    pub width: i32,
    pub height: i32,
    pub initial_width: f32,
    pub initial_height: f32,
    pub max_juke_boxes_active: i32,
    pub num_juke_boxes_active: i32,
    pub game_mode: String,
    pub add_zombie_on_cell_load: bool,
    pub preset: String,
    pub gl_version: Option<String>,
    pub gl_major_version: i32,
    pub debug: bool,
    pub anti_cheats: bool,
    pub use_viewports: bool,
    pub use_game_viewport: bool,
    pub im_gui: bool,
    pub game_map: String,
    pub game_save_world: String,
    pub safe_mode: bool,
    pub safe_mode_forced: bool,
    pub sound_disabled: bool,
    pub xx: i32,
    pub yy: i32,
    pub zz: i32,
    pub last_stand: bool,
    pub challenge_id: Option<String>,
    pub exiting: bool,
    pub current_text_entry_box: Option<TextEntryInfo>,

    // ---- instance state ----
    git_revision_string: Option<String>,
    pub steam_server_version: String,
    console_dot_txt_size_kb: i32,
    object_highlited_color: ColorInfo,
    world_item_highlight_color: ColorInfo,
    workstation_highlited_color: ColorInfo,
    good_highlited_color: ColorInfo,
    bad_highlited_color: ColorInfo,
    flash_iso_cursor: bool,
    target_color: ColorInfo,
    no_target_color: ColorInfo,
    account_used: Option<Rc<dyn Any>>,
    selected_map: Option<String>,
    git_sha: Option<String>,
    collide_zombies: bool,
    pub offscreen_buffer: OffscreenBuffer,
    save_folder: Option<String>,
    show_ping: bool,
    force_snow: bool,
    zombie_group_sound: bool,
    blinking_moodle: Option<String>,
    poisonous_berry: Option<String>,
    poisonous_mushroom: Option<String>,
    is_selecting_all: bool,
    mp_text_color: Option<ColorInfo>,
    is_azerty: bool,
    no_save: bool,
    show_first_time_vehicle_tutorial: bool,
    show_first_time_weather_tutorial: bool,
    anim_popup_done: bool,
    mods_popup_done: bool,
    loaded_options: bool,
    options_on_startup: HashMap<String, Rc<dyn Any>>,
    pub animal_cheat: bool,
    pub display_player_model: bool,
    pub display_cursor: bool,
    screen_filter: i32,
    challenge: bool,
    key_maps: HashMap<String, KeyBinding>,
    key_binding_by_key_value: HashMap<i32, Vec<KeyBinding>>,
    i_perf_skybox: i32,
    i_perf_puddles: i32,
    perf_reflections: bool,
    pub vid_mem: i32,
    supports_fbo: bool,
    pub ui_render_accumulator: f32,
    pub ui_render_this_frame: bool,
    /// Version read from `options.ini`.
    pub version: i32,
    /// Version written to `options.ini`.
    pub fileversion: i32,
    pub frame_stage: i32,
    delay_reset_lua: Option<(String, String)>,

    // ---- state of other systems Core reads or writes ----
    pub performance: PerformanceSettings,
    /// `JoypadManager.activeControllerGuids` (insertion-ordered; the Java
    /// `HashSet` order is unspecified).
    pub active_controller_guids: Vec<String>,
    /// GUIDs of connected controllers by index (`GameInput.getController`).
    pub connected_controller_guids: Vec<String>,
    /// `MovingObjectUpdateScheduler.instance.setEnabled`.
    pub moving_object_update_scheduler_enabled: bool,
    /// `GameServer.server`.
    pub is_server: bool,
    /// `GameClient.client`.
    pub is_client: bool,
    pub voice: VoiceState,
    /// `Display.getAvailableDisplayModes()` (width, height). TODO(render).
    pub available_display_modes: Vec<(i32, i32)>,
    /// Primary monitor work area (width, height). TODO(render).
    pub monitor_work_area: Option<(i32, i32)>,
    /// `Display.capabilities.OpenGL33`. TODO(render).
    pub gl33_supported: bool,
    /// Raw key state for `invalidBindingShiftCtrl` (`GameKeyboard.isKeyDownRaw`).
    pub keys_down_raw: HashSet<i32>,
    /// Mouse buttons down (`Mouse.isButtonDownUICheck`).
    pub mouse_buttons_down: HashSet<i32>,
    /// `IsoPlayer.getInstance() != null`.
    pub player_exists: bool,
    /// `IsoPlayer.numPlayers`.
    pub num_players: i32,
    /// `GameWindow.closeRequested`.
    pub close_requested: bool,
    /// `GameWindow.states.current == IngameState.instance`.
    pub in_game_state: bool,
    /// `UIManager.useUiFbo`.
    pub use_ui_fbo: bool,
    /// `TextManager.instance.currentCodeFont` (UIFont name).
    pub current_code_font: &'static str,
    /// `System.getProperty("user.language").toUpperCase()`.
    pub system_language: String,
    /// `Bullet.getPZBulletVersion()` (native; no physics port yet).
    pub bullet_version: String,
    pending_events: Vec<CoreEvent>,
    pending_requests: Vec<CoreRequest>,
    rng: u64,
}

impl Default for Core {
    fn default() -> Self {
        Self::new()
    }
}

macro_rules! simple_accessors {
    ($( $kind:ident $ty:ident $rust:ident $java:literal $opt:literal; )*) => {
        impl Core { $( simple_accessors!(@m $kind $ty $rust $opt); )* }
        /// Option getters/setters whose Java body is a single
        /// `getValue()` / `setValue(v)` on one option.
        pub const SIMPLE_ACCESSORS: &[SimpleAccessor] = &[ $( simple_accessors!(@t $kind $ty $rust $java) ),* ];
    };
    (@m get bool $rust:ident $opt:literal) => { pub fn $rust(&self) -> bool { self.opt($opt).get_bool() } };
    (@m get i32 $rust:ident $opt:literal) => { pub fn $rust(&self) -> i32 { self.opt($opt).get_int() } };
    (@m get f64 $rust:ident $opt:literal) => { pub fn $rust(&self) -> f64 { self.opt($opt).get_double() } };
    (@m get f32 $rust:ident $opt:literal) => { pub fn $rust(&self) -> f32 { self.opt($opt).get_double() as f32 } };
    (@m get String $rust:ident $opt:literal) => { pub fn $rust(&self) -> String { self.opt($opt).get_string().to_string() } };
    (@m set bool $rust:ident $opt:literal) => { pub fn $rust(&mut self, v: bool) { self.opt_mut($opt).set_bool(v) } };
    (@m set i32 $rust:ident $opt:literal) => { pub fn $rust(&mut self, v: i32) { self.opt_mut($opt).set_int(v) } };
    (@m set f64 $rust:ident $opt:literal) => { pub fn $rust(&mut self, v: f64) { self.opt_mut($opt).set_double(v) } };
    (@m set f32 $rust:ident $opt:literal) => { pub fn $rust(&mut self, v: f32) { self.opt_mut($opt).set_double(v as f64) } };
    (@m set String $rust:ident $opt:literal) => { pub fn $rust(&mut self, v: Option<&str>) { self.opt_mut($opt).set_string(v) } };
    (@t get bool $rust:ident $java:literal) => { SimpleAccessor::GetBool($java, Core::$rust) };
    (@t get i32 $rust:ident $java:literal) => { SimpleAccessor::GetInt($java, Core::$rust) };
    (@t get f64 $rust:ident $java:literal) => { SimpleAccessor::GetDouble($java, Core::$rust) };
    (@t get f32 $rust:ident $java:literal) => { SimpleAccessor::GetFloat($java, Core::$rust) };
    (@t get String $rust:ident $java:literal) => { SimpleAccessor::GetString($java, Core::$rust) };
    (@t set bool $rust:ident $java:literal) => { SimpleAccessor::SetBool($java, Core::$rust) };
    (@t set i32 $rust:ident $java:literal) => { SimpleAccessor::SetInt($java, Core::$rust) };
    (@t set f64 $rust:ident $java:literal) => { SimpleAccessor::SetDouble($java, Core::$rust) };
    (@t set f32 $rust:ident $java:literal) => { SimpleAccessor::SetFloat($java, Core::$rust) };
    (@t set String $rust:ident $java:literal) => { SimpleAccessor::SetString($java, Core::$rust) };
}

/// A simple option accessor with its Java method name.
#[derive(Clone, Copy)]
pub enum SimpleAccessor {
    GetBool(&'static str, fn(&Core) -> bool),
    GetInt(&'static str, fn(&Core) -> i32),
    GetDouble(&'static str, fn(&Core) -> f64),
    GetFloat(&'static str, fn(&Core) -> f32),
    GetString(&'static str, fn(&Core) -> String),
    SetBool(&'static str, fn(&mut Core, bool)),
    SetInt(&'static str, fn(&mut Core, i32)),
    SetDouble(&'static str, fn(&mut Core, f64)),
    SetFloat(&'static str, fn(&mut Core, f32)),
    SetString(&'static str, fn(&mut Core, Option<&str>)),
}

impl SimpleAccessor {
    pub fn java_name(&self) -> &'static str {
        match *self {
            SimpleAccessor::GetBool(n, _)
            | SimpleAccessor::GetInt(n, _)
            | SimpleAccessor::GetDouble(n, _)
            | SimpleAccessor::GetFloat(n, _)
            | SimpleAccessor::GetString(n, _)
            | SimpleAccessor::SetBool(n, _)
            | SimpleAccessor::SetInt(n, _)
            | SimpleAccessor::SetDouble(n, _)
            | SimpleAccessor::SetFloat(n, _)
            | SimpleAccessor::SetString(n, _) => n,
        }
    }
}

simple_accessors! {
    get bool get_option_tiered_zombie_updates "getOptionTieredZombieUpdates" "tieredZombieUpdates";
    get i32 get_perf_skybox "getPerfSkybox" "perfSkybox";
    get bool get_perf_reflections "getPerfReflections" "bPerfReflections";
    get i32 get_perf_puddles "getPerfPuddles" "perfPuddles";
    get bool is_full_screen "isFullScreen" "fullScreen";
    get bool get_option_zoom "getOptionZoom" "zoom";
    get bool get_option_v_sync "getOptionVSync" "vsync";
    get i32 get_option_sound_volume "getOptionSoundVolume" "soundVolume";
    get i32 get_option_music_volume "getOptionMusicVolume" "musicVolume";
    get i32 get_option_ambient_volume "getOptionAmbientVolume" "ambientVolume";
    get i32 get_option_jump_scare_volume "getOptionJumpScareVolume" "jumpScareVolume";
    get i32 get_option_music_action_style "getOptionMusicActionStyle" "musicActionStyle";
    get i32 get_option_music_library "getOptionMusicLibrary" "musicLibrary";
    get i32 get_option_vehicle_engine_volume "getOptionVehicleEngineVolume" "vehicleEngineVolume";
    get bool get_option_streamer_mode "getOptionStreamerMode" "vehicleStreamerMode";
    get bool get_option_voice_enable "getOptionVoiceEnable" "voiceEnable";
    get i32 get_option_voice_mode "getOptionVoiceMode" "voiceMode";
    get i32 get_option_voice_vad_mode "getOptionVoiceVADMode" "voiceVADMode";
    get i32 get_option_voice_agc_mode "getOptionVoiceAGCMode" "voiceAGCMode";
    get i32 get_option_voice_volume_mic "getOptionVoiceVolumeMic" "voiceVolumeMic";
    get i32 get_option_voice_volume_players "getOptionVoiceVolumePlayers" "voiceVolumePlayers";
    get String get_option_voice_record_device_name "getOptionVoiceRecordDeviceName" "voiceRecordDeviceName";
    get bool get_option_rack_progress "getOptionRackProgress" "rackProgress";
    get i32 get_option_font_size "getOptionFontSize" "fontSize";
    get i32 get_option_moodle_size "getOptionMoodleSize" "moodleSize";
    get i32 get_option_sidebar_size "getOptionSidebarSize" "sidebarSize";
    get i32 get_option_action_progress_bar_size "getOptionActionProgressBarSize" "actionProgressBarSize";
    get String get_option_context_menu_font "getOptionContextMenuFont" "contextMenuFont";
    get String get_option_code_font_size "getOptionCodeFontSize" "codeFontSize";
    get String get_option_inventory_font "getOptionInventoryFont" "inventoryFont";
    get i32 get_option_inventory_container_size "getOptionInventoryContainerSize" "inventoryContainerSize";
    get String get_option_tooltip_font "getOptionTooltipFont" "tooltipFont";
    get String get_option_measurement_format "getOptionMeasurementFormat" "measurementsFormat";
    get i32 get_option_clock_format "getOptionClockFormat" "clockFormat";
    get i32 get_option_clock_size "getOptionClockSize" "clockSize";
    get bool get_option_clock24_hour "getOptionClock24Hour" "clock24Hour";
    get i32 get_option_blood_decals "getOptionBloodDecals" "bloodDecals";
    get bool get_option_pause_on_focusloss "getOptionPauseOnFocusloss" "focusloss";
    get bool get_option_map_view_pause "getOptionMapViewPause" "mapViewPause";
    get bool get_option_borderless_window "getOptionBorderlessWindow" "borderless";
    get bool get_option_lock_cursor_to_window "getOptionLockCursorToWindow" "lockCursorToWindow";
    get bool get_option_high_res_placed_items "getOptionHighResPlacedItems" "highResPlacedItems";
    get i32 get_option_max_texture_size "getOptionMaxTextureSize" "maxTextureSize";
    get i32 get_option_max_vehicle_texture_size "getOptionMaxVehicleTextureSize" "maxVehicleTextureSize";
    get bool get_option_model_texture_mipmaps "getOptionModelTextureMipmaps" "modelTextureMipmaps";
    get String get_option_screen_filter "getOptionScreenFilter" "screenFilter";
    get bool is_option_show_chat_timestamp "isOptionShowChatTimestamp" "showChatTimestamp";
    get bool is_option_show_chat_title "isOptionShowChatTitle" "showChatTitle";
    get String get_option_chat_font_size "getOptionChatFontSize" "chatFontSize";
    get f32 get_option_min_chat_opaque "getOptionMinChatOpaque" "minChatOpaque";
    get f32 get_option_max_chat_opaque "getOptionMaxChatOpaque" "maxChatOpaque";
    get f32 get_option_chat_fade_time "getOptionChatFadeTime" "chatFadeTime";
    get bool get_option_chat_opaque_on_focus "getOptionChatOpaqueOnFocus" "chatOpaqueOnFocus";
    get bool get_option_temperature_display_celsius "getOptionTemperatureDisplayCelsius" "temperatureDisplayCelsius";
    get bool get_option_uifbo "getOptionUIFBO" "uiRenderOffscreen";
    get bool get_option_melee_outline "getOptionMeleeOutline" "meleeOutline";
    get i32 get_option_ui_render_fps "getOptionUIRenderFPS" "uiRenderFPS";
    get bool get_option_radial_menu_key_toggle "getOptionRadialMenuKeyToggle" "radialMenuKeyToggle";
    get bool get_option_reload_radial_instant "getOptionReloadRadialInstant" "reloadRadialInstant";
    get bool get_option_pan_camera_while_aiming "getOptionPanCameraWhileAiming" "panCameraWhileAiming";
    get bool get_option_pan_camera_while_driving "getOptionPanCameraWhileDriving" "panCameraWhileDriving";
    get String get_option_cycle_container_key "getOptionCycleContainerKey" "cycleContainerKey";
    get bool get_option_drop_items_on_square_center "getOptionDropItemsOnSquareCenter" "dropItemsOnSquareCenter";
    get bool get_option_timed_action_game_speed_reset "getOptionTimedActionGameSpeedReset" "timedActionGameSpeedReset";
    get i32 get_option_shoulder_button_container_switch "getOptionShoulderButtonContainerSwitch" "shoulderButtonContainerSwitch";
    get i32 get_option_controller_button_style "getOptionControllerButtonStyle" "controllerButtonStyle";
    get String get_option_gamepad_binding_preset "getOptionGamepadBindingPreset" "gamepadBindingPreset";
    get bool get_option_auto_drink "getOptionAutoDrink" "autoDrink";
    get bool get_option_auto_reveal_print_media_map_locations "getOptionAutoRevealPrintMediaMapLocations" "autoRevealPrintMediaMapLocations";
    get bool get_option_auto_walk_container "getOptionAutoWalkContainer" "autoWalkContainer";
    get bool get_option_corpse_shadows "getOptionCorpseShadows" "corpseShadows";
    get bool get_option_leave_key_in_ignition "getOptionLeaveKeyInIgnition" "leaveKeyInIgnition";
    get i32 get_option_search_mode_overlay_effect "getOptionSearchModeOverlayEffect" "searchModeOverlayEffect";
    get i32 get_option_simple_clothing_textures "getOptionSimpleClothingTextures" "simpleClothingTextures";
    get bool get_option_simple_weapon_textures "getOptionSimpleWeaponTextures" "simpleWeaponTextures";
    get i32 get_option_ignore_prone_zombie_range "getOptionIgnoreProneZombieRange" "ignoreProneZombieRange";
    get bool is_tutorial_done "isTutorialDone" "tutorialDone";
    get bool is_vehicles_warning_show "isVehiclesWarningShow" "vehiclesWarningShow";
    get bool is_done_new_save_folder "isDoneNewSaveFolder" "doneNewSaveFolder";
    get bool get_content_translations_enabled "getContentTranslationsEnabled" "contentTranslationsEnabled";
    get bool is_show_your_username "isShowYourUsername" "showYourUsername";
    get bool is_populate_server_list_on_start "isPopulateServerListOnStart" "populateServerListOnStart";
    get bool get_option_colorblind_patterns "getOptionColorblindPatterns" "colorblindPatterns";
    get bool get_option_enable_dyslexic_font "getOptionEnableDyslexicFont" "enableDyslexicFont";
    get bool get_option_disable_lightning_during_storms "getOptionDisableLightningDuringStorms" "disableLightningDuringStorms";
    get String get_seen_update_text "getSeenUpdateText" "seenNews";
    get bool is_toggle_to_aim "isToggleToAim" "toggleToAim";
    get bool is_toggle_to_run "isToggleToRun" "toggleToRun";
    get bool is_celsius "isCelsius" "celsius";
    get bool is_riverside_done "isRiversideDone" "riversideDone";
    get bool get_option_display_as_celsius "getOptionDisplayAsCelsius" "temperatureDisplayCelsius";
    get bool get_option_do_video_effects "getOptionDoVideoEffects" "doVideoEffects";
    get bool get_option_do_wind_sprite_effects "getOptionDoWindSpriteEffects" "doWindSpriteEffects";
    get bool get_option_do_door_sprite_effects "getOptionDoDoorSpriteEffects" "doDoorSpriteEffects";
    get bool get_option_do_container_outline "getOptionDoContainerOutline" "doContainerOutline";
    get bool get_option_update_sneak_button "getOptionUpdateSneakButton" "updateSneakButton";
    get bool is_show_first_time_sneak_tutorial "isShowFirstTimeSneakTutorial" "showFirstTimeSneakTutorial";
    get f64 get_shown_welcome_message_version "getShownWelcomeMessageVersion" "shownWelcomeMessageVersion";
    get bool is_show_first_time_search_tutorial "isShowFirstTimeSearchTutorial" "showFirstTimeSearchTutorial";
    get i32 get_terms_of_service_version "getTermsOfServiceVersion" "termsOfServiceVersion";
    get bool is_optiondbl_tap_jog_to_sprint "isOptiondblTapJogToSprint" "dblTapJogToSprint";
    get bool is_toggle_to_sprint "isToggleToSprint" "toggleToSprint";
    get i32 get_iso_cursor_visibility "getIsoCursorVisibility" "iso_cursor";
    get bool get_option_show_cursor_while_aiming "getOptionShowCursorWhileAiming" "showCursorWhileAiming";
    get bool got_new_belt "gotNewBelt" "gotNewBelt";
    get bool is_render_precip_indoors "isRenderPrecipIndoors" "renderPrecipIndoors";
    get f32 get_option_precipitation_speed_multiplier "getOptionPrecipitationSpeedMultiplier" "precipitationSpeedMultiplier";
    get String get_option_language_name "getOptionLanguageName" "language";
    get i32 get_option_render_precipitation "getOptionRenderPrecipitation" "renderPrecipitation";
    get bool is_option_auto_prone_atk "isOptionAutoProneAtk" "autoProneAtk";
    get bool is_option3_d_ground_item "isOption3DGroundItem" "3DGroundItem";
    get bool get_option_show_item_mod_info "getOptionShowItemModInfo" "showItemModInfo";
    get bool get_option_show_crafting_xp "getOptionShowCraftingXP" "showCraftingXP";
    get bool get_option_show_survival_guide "getOptionShowSurvivalGuide" "showSurvivalGuide";
    get bool get_option_show_first_animal_zone_info "getOptionShowFirstAnimalZoneInfo" "showFirstAnimalZoneInfo";
    get bool get_option_enable_left_joystick_radial_menu "getOptionEnableLeftJoystickRadialMenu" "enableLeftJoystickRadialMenu";
    get bool get_option_mac_os_ignore_mouse_wheel_acceleration "getOptionMacOSIgnoreMouseWheelAcceleration" "macosIgnoreMouseWheelAcceleration";
    get bool get_option_mac_os_map_horizontal_mouse_wheel_to_vertical "getOptionMacOSMapHorizontalMouseWheelToVertical" "macosMapHorizontalMouseWheelToVertical";
    get bool get_option_show_aim_texture "getOptionShowAimTexture" "showAimTexture";
    get bool get_option_show_reticle_texture "getOptionShowReticleTexture" "showReticleTexture";
    get bool get_option_show_valid_target_reticle_texture "getOptionShowValidTargetReticleTexture" "showValidTargetReticleTexture";
    get i32 get_option_reticle_mode "getOptionReticleMode" "reticleMode";
    get i32 get_option_aim_texture_index "getOptionAimTextureIndex" "aimTextureIndex";
    get i32 get_option_reticle_texture_index "getOptionReticleTextureIndex" "reticleTextureIndex";
    get i32 get_option_valid_target_reticle_texture_index "getOptionValidTargetReticleTextureIndex" "validTargetReticleTextureIndex";
    get i32 get_option_crosshair_texture_index "getOptionCrosshairTextureIndex" "crosshairTextureIndex";
    get i32 get_option_max_crosshair_offset "getOptionMaxCrosshairOffset" "maxCrosshairOffset";
    get bool get_option_reticle_camera_zoom "getOptionReticleCameraZoom" "reticleCameraZoom";
    get i32 get_max_active_ragdolls "getMaxActiveRagdolls" "maxActiveRagdolls";
    get f64 get_option_world_map_brightness "getOptionWorldMapBrightness" "worldMapBrightness";
    get bool get_option_show_welcome_message "getOptionShowWelcomeMessage" "showWelcomeMessage2";
    set i32 set_perf_skybox "setPerfSkybox" "perfSkybox";
    set bool set_perf_reflections "setPerfReflections" "bPerfReflections";
    set i32 set_perf_puddles "setPerfPuddles" "perfPuddles";
    set bool set_option_zoom "setOptionZoom" "zoom";
    set i32 set_option_jump_scare_volume "setOptionJumpScareVolume" "jumpScareVolume";
    set i32 set_option_music_action_style "setOptionMusicActionStyle" "musicActionStyle";
    set i32 set_option_music_library "setOptionMusicLibrary" "musicLibrary";
    set bool set_option_streamer_mode "setOptionStreamerMode" "vehicleStreamerMode";
    set i32 set_option_reload_difficulty "setOptionReloadDifficulty" "reloadDifficulty";
    set bool set_option_rack_progress "setOptionRackProgress" "rackProgress";
    set i32 set_option_font_size "setOptionFontSize" "fontSize";
    set i32 set_option_moodle_size "setOptionMoodleSize" "moodleSize";
    set i32 set_option_sidebar_size "setOptionSidebarSize" "sidebarSize";
    set i32 set_option_action_progress_bar_size "setOptionActionProgressBarSize" "actionProgressBarSize";
    set String set_option_context_menu_font "setOptionContextMenuFont" "contextMenuFont";
    set String set_option_inventory_font "setOptionInventoryFont" "inventoryFont";
    set i32 set_option_inventory_container_size "setOptionInventoryContainerSize" "inventoryContainerSize";
    set String set_option_measurement_format "setOptionMeasurementFormat" "measurementsFormat";
    set i32 set_option_clock_format "setOptionClockFormat" "clockFormat";
    set i32 set_option_clock_size "setOptionClockSize" "clockSize";
    set bool set_option_clock24_hour "setOptionClock24Hour" "clock24Hour";
    set i32 set_option_blood_decals "setOptionBloodDecals" "bloodDecals";
    set bool set_option_pause_on_focusloss "setOptionPauseOnFocusloss" "focusloss";
    set bool set_option_map_view_pause "setOptionMapViewPause" "mapViewPause";
    set bool set_option_borderless_window "setOptionBorderlessWindow" "borderless";
    set bool set_option_lock_cursor_to_window "setOptionLockCursorToWindow" "lockCursorToWindow";
    set bool set_option_texture_compression "setOptionTextureCompression" "textureCompression";
    set bool set_option_high_res_placed_items "setOptionHighResPlacedItems" "highResPlacedItems";
    set i32 set_option_max_texture_size "setOptionMaxTextureSize" "maxTextureSize";
    set i32 set_option_max_vehicle_texture_size "setOptionMaxVehicleTextureSize" "maxVehicleTextureSize";
    set bool set_option_model_texture_mipmaps "setOptionModelTextureMipmaps" "modelTextureMipmaps";
    set bool set_option_show_chat_timestamp "setOptionShowChatTimestamp" "showChatTimestamp";
    set String set_option_chat_font_size "setOptionChatFontSize" "chatFontSize";
    set bool set_option_show_chat_title "setOptionShowChatTitle" "showChatTitle";
    set f32 set_option_min_chat_opaque "setOptionMinChatOpaque" "minChatOpaque";
    set f32 set_option_max_chat_opaque "setOptionMaxChatOpaque" "maxChatOpaque";
    set f32 set_option_chat_fade_time "setOptionChatFadeTime" "chatFadeTime";
    set bool set_option_chat_opaque_on_focus "setOptionChatOpaqueOnFocus" "chatOpaqueOnFocus";
    set bool set_option_melee_outline "setOptionMeleeOutline" "meleeOutline";
    set i32 set_option_ui_render_fps "setOptionUIRenderFPS" "uiRenderFPS";
    set bool set_option_radial_menu_key_toggle "setOptionRadialMenuKeyToggle" "radialMenuKeyToggle";
    set bool set_option_reload_radial_instant "setOptionReloadRadialInstant" "reloadRadialInstant";
    set bool set_option_pan_camera_while_aiming "setOptionPanCameraWhileAiming" "panCameraWhileAiming";
    set bool set_option_pan_camera_while_driving "setOptionPanCameraWhileDriving" "panCameraWhileDriving";
    set String set_option_cycle_container_key "setOptionCycleContainerKey" "cycleContainerKey";
    set bool set_option_drop_items_on_square_center "setOptionDropItemsOnSquareCenter" "dropItemsOnSquareCenter";
    set bool set_option_timed_action_game_speed_reset "setOptionTimedActionGameSpeedReset" "timedActionGameSpeedReset";
    set i32 set_option_shoulder_button_container_switch "setOptionShoulderButtonContainerSwitch" "shoulderButtonContainerSwitch";
    set i32 set_option_controller_button_style "setOptionControllerButtonStyle" "controllerButtonStyle";
    set String set_option_gamepad_binding_preset "setOptionGamepadBindingPreset" "gamepadBindingPreset";
    set bool set_option_auto_drink "setOptionAutoDrink" "autoDrink";
    set bool set_option_auto_reveal_print_media_map_locations "setOptionAutoRevealPrintMediaMapLocations" "autoRevealPrintMediaMapLocations";
    set bool set_option_auto_walk_container "setOptionAutoWalkContainer" "autoWalkContainer";
    set bool set_option_corpse_shadows "setOptionCorpseShadows" "corpseShadows";
    set bool set_option_leave_key_in_ignition "setOptionLeaveKeyInIgnition" "leaveKeyInIgnition";
    set i32 set_option_search_mode_overlay_effect "setOptionSearchModeOverlayEffect" "searchModeOverlayEffect";
    set i32 set_option_simple_clothing_textures "setOptionSimpleClothingTextures" "simpleClothingTextures";
    set bool set_option_simple_weapon_textures "setOptionSimpleWeaponTextures" "simpleWeaponTextures";
    set i32 set_option_ignore_prone_zombie_range "setOptionIgnoreProneZombieRange" "ignoreProneZombieRange";
    set bool set_tutorial_done "setTutorialDone" "tutorialDone";
    set bool set_vehicles_warning_show "setVehiclesWarningShow" "vehiclesWarningShow";
    set bool set_done_new_save_folder "setDoneNewSaveFolder" "doneNewSaveFolder";
    set bool set_content_translations_enabled "setContentTranslationsEnabled" "contentTranslationsEnabled";
    set bool set_show_your_username "setShowYourUsername" "showYourUsername";
    set bool set_populate_server_list_on_start "setPopulateServerListOnStart" "populateServerListOnStart";
    set bool set_option_colorblind_patterns "setOptionColorblindPatterns" "colorblindPatterns";
    set bool set_option_enable_dyslexic_font "setOptionEnableDyslexicFont" "enableDyslexicFont";
    set bool set_option_disable_lightning_during_storms "setOptionDisableLightningDuringStorms" "disableLightningDuringStorms";
    set String set_seen_update_text "setSeenUpdateText" "seenNews";
    set bool set_toggle_to_aim "setToggleToAim" "toggleToAim";
    set bool set_toggle_to_run "setToggleToRun" "toggleToRun";
    set bool set_celsius "setCelsius" "celsius";
    set bool set_riverside_done "setRiversideDone" "riversideDone";
    set bool set_option_display_as_celsius "setOptionDisplayAsCelsius" "temperatureDisplayCelsius";
    set bool set_option_do_video_effects "setOptionDoVideoEffects" "doVideoEffects";
    set bool set_option_do_wind_sprite_effects "setOptionDoWindSpriteEffects" "doWindSpriteEffects";
    set bool set_option_do_door_sprite_effects "setOptionDoDoorSpriteEffects" "doDoorSpriteEffects";
    set bool set_option_do_container_outline "setOptionDoContainerOutline" "doContainerOutline";
    set bool set_option_update_sneak_button "setOptionUpdateSneakButton" "updateSneakButton";
    set bool set_show_first_time_sneak_tutorial "setShowFirstTimeSneakTutorial" "showFirstTimeSneakTutorial";
    set f64 set_shown_welcome_message_version "setShownWelcomeMessageVersion" "shownWelcomeMessageVersion";
    set bool set_show_first_time_search_tutorial "setShowFirstTimeSearchTutorial" "showFirstTimeSearchTutorial";
    set i32 set_terms_of_service_version "setTermsOfServiceVersion" "termsOfServiceVersion";
    set bool set_optiondbl_tap_jog_to_sprint "setOptiondblTapJogToSprint" "dblTapJogToSprint";
    set bool set_toggle_to_sprint "setToggleToSprint" "toggleToSprint";
    set i32 set_iso_cursor_visibility "setIsoCursorVisibility" "iso_cursor";
    set bool set_option_show_cursor_while_aiming "setOptionShowCursorWhileAiming" "showCursorWhileAiming";
    set bool set_got_new_belt "setGotNewBelt" "gotNewBelt";
    set bool set_render_precip_indoors "setRenderPrecipIndoors" "renderPrecipIndoors";
    set f32 set_option_precipitation_speed_multiplier "setOptionPrecipitationSpeedMultiplier" "precipitationSpeedMultiplier";
    set bool set_option_progress_bar "setOptionProgressBar" "progressBar";
    set String set_option_language_name "setOptionLanguageName" "language";
    set i32 set_option_render_precipitation "setOptionRenderPrecipitation" "renderPrecipitation";
    set bool set_option_auto_prone_atk "setOptionAutoProneAtk" "autoProneAtk";
    set bool set_option3_d_ground_item "setOption3DGroundItem" "3DGroundItem";
    set bool set_option_show_item_mod_info "setOptionShowItemModInfo" "showItemModInfo";
    set bool set_option_show_crafting_xp "setOptionShowCraftingXP" "showCraftingXP";
    set bool set_option_show_survival_guide "setOptionShowSurvivalGuide" "showSurvivalGuide";
    set bool set_option_show_first_animal_zone_info "setOptionShowFirstAnimalZoneInfo" "showFirstAnimalZoneInfo";
    set bool set_option_enable_left_joystick_radial_menu "setOptionEnableLeftJoystickRadialMenu" "enableLeftJoystickRadialMenu";
    set bool set_option_mac_os_ignore_mouse_wheel_acceleration "setOptionMacOSIgnoreMouseWheelAcceleration" "macosIgnoreMouseWheelAcceleration";
    set bool set_option_mac_os_map_horizontal_mouse_wheel_to_vertical "setOptionMacOSMapHorizontalMouseWheelToVertical" "macosMapHorizontalMouseWheelToVertical";
    set bool set_option_show_aim_texture "setOptionShowAimTexture" "showAimTexture";
    set bool set_option_show_reticle_texture "setOptionShowReticleTexture" "showReticleTexture";
    set bool set_option_show_valid_target_reticle_texture "setOptionShowValidTargetReticleTexture" "showValidTargetReticleTexture";
    set i32 set_option_reticle_mode "setOptionReticleMode" "reticleMode";
    set i32 set_option_aim_texture_index "setOptionAimTextureIndex" "aimTextureIndex";
    set i32 set_option_reticle_texture_index "setOptionReticleTextureIndex" "reticleTextureIndex";
    set i32 set_option_valid_target_reticle_texture_index "setOptionValidTargetReticleTextureIndex" "validTargetReticleTextureIndex";
    set i32 set_option_crosshair_texture_index "setOptionCrosshairTextureIndex" "crosshairTextureIndex";
    set i32 set_option_max_crosshair_offset "setOptionMaxCrosshairOffset" "maxCrosshairOffset";
    set bool set_option_reticle_camera_zoom "setOptionReticleCameraZoom" "reticleCameraZoom";
    set bool set_option_use_physics_hit_reaction "setOptionUsePhysicsHitReaction" "usePhysicsHitReaction";
    set i32 set_max_active_ragdolls "setMaxActiveRagdolls" "maxActiveRagdolls";
    set f64 set_option_world_map_brightness "setOptionWorldMapBrightness" "worldMapBrightness";
    set bool set_option_show_welcome_message "setOptionShowWelcomeMessage" "showWelcomeMessage2";
}

impl Core {
    /// `new Core()` plus the static initialiser's values.
    pub fn new() -> Self {
        let mut core = Core {
            options: Vec::new(),
            fake_options: Vec::new(),
            option_by_name: HashMap::new(),
            fake_option_by_name: HashMap::new(),
            cache_dir: None,
            options_path: None,
            game_dir: None,
            is_dev: false,
            tutorial: false,
            dirty_global_lights_count: 0,
            alt_move_method: false,
            option_mods_enabled: true,
            difficulty: "Hardcore".into(),
            tile_scale: 2,
            blink_alpha: 1.0,
            blink_alpha_increase: false,
            width: 1280,
            height: 720,
            initial_width: 1280.0,
            initial_height: 720.0,
            max_juke_boxes_active: 10,
            num_juke_boxes_active: 0,
            game_mode: "Sandbox".into(),
            add_zombie_on_cell_load: true,
            preset: "Apocalypse".into(),
            gl_version: None,
            gl_major_version: -1,
            debug: false,
            anti_cheats: false,
            use_viewports: false,
            use_game_viewport: true,
            im_gui: false,
            game_map: "DEFAULT".into(),
            game_save_world: String::new(),
            safe_mode: false,
            safe_mode_forced: false,
            sound_disabled: false,
            xx: 0,
            yy: 0,
            zz: 0,
            last_stand: false,
            challenge_id: None,
            exiting: false,
            current_text_entry_box: None,
            git_revision_string: None,
            steam_server_version: "1.0.0.0".into(),
            console_dot_txt_size_kb: 5120,
            object_highlited_color: ColorInfo::new(0.98, 0.56, 0.11, 1.0),
            world_item_highlight_color: ColorInfo::new(0.5, 1.0, 1.0, 1.0),
            workstation_highlited_color: ColorInfo::new(0.56, 0.98, 0.11, 1.0),
            good_highlited_color: ColorInfo::new(0.0, 1.0, 0.0, 1.0),
            bad_highlited_color: ColorInfo::new(1.0, 0.0, 0.0, 1.0),
            flash_iso_cursor: false,
            target_color: ColorInfo::new(1.0, 1.0, 1.0, 1.0),
            no_target_color: ColorInfo::new(1.0, 1.0, 1.0, 1.0),
            account_used: None,
            selected_map: None,
            git_sha: None,
            collide_zombies: true,
            offscreen_buffer: OffscreenBuffer::default(),
            save_folder: None,
            show_ping: true,
            force_snow: false,
            zombie_group_sound: true,
            blinking_moodle: None,
            poisonous_berry: None,
            poisonous_mushroom: None,
            is_selecting_all: false,
            mp_text_color: None,
            is_azerty: false,
            no_save: false,
            show_first_time_vehicle_tutorial: false,
            show_first_time_weather_tutorial: false,
            anim_popup_done: false,
            mods_popup_done: false,
            loaded_options: false,
            options_on_startup: HashMap::new(),
            animal_cheat: false,
            display_player_model: true,
            display_cursor: true,
            screen_filter: 9728,
            challenge: false,
            key_maps: HashMap::new(),
            key_binding_by_key_value: HashMap::new(),
            i_perf_skybox: 1,
            i_perf_puddles: 0,
            perf_reflections: true,
            vid_mem: 3,
            supports_fbo: true,
            ui_render_accumulator: 0.0,
            ui_render_this_frame: true,
            version: 1,
            fileversion: 8,
            frame_stage: 0,
            delay_reset_lua: None,
            performance: PerformanceSettings::default(),
            active_controller_guids: Vec::new(),
            connected_controller_guids: Vec::new(),
            moving_object_update_scheduler_enabled: true,
            is_server: false,
            is_client: false,
            voice: VoiceState::default(),
            available_display_modes: Vec::new(),
            monitor_work_area: None,
            gl33_supported: true,
            keys_down_raw: HashSet::new(),
            mouse_buttons_down: HashSet::new(),
            player_exists: false,
            num_players: 1,
            close_requested: false,
            in_game_state: false,
            use_ui_fbo: false,
            current_code_font: "CodeSmall",
            system_language: detect_system_language(),
            bullet_version: String::new(),
            pending_events: Vec::new(),
            pending_requests: Vec::new(),
            rng: seed_rng(),
        };
        core.register_options();
        core
    }

    fn add(&mut self, fake: bool, option: ConfigOption) {
        let name = option.name().to_string();
        if fake {
            self.fake_option_by_name
                .insert(name, self.fake_options.len());
            self.fake_options.push(option);
        } else {
            self.option_by_name.insert(name, self.options.len());
            self.options.push(option);
        }
    }

    /// The `newOption` / `newFakeOption` field initialisers, in order.
    fn register_options(&mut self) {
        self.add(false, ConfigOption::integer("reticleMode", 0, 1, 0));
        self.add(false, ConfigOption::boolean("showAimTexture", false));
        self.add(false, ConfigOption::boolean("showReticleTexture", true));
        self.add(
            false,
            ConfigOption::boolean("showValidTargetReticleTexture", true),
        );
        self.add(false, ConfigOption::integer("aimTextureIndex", 0, 17, 0));
        self.add(false, ConfigOption::integer("reticleTextureIndex", 0, 6, 0));
        self.add(
            false,
            ConfigOption::integer("validTargetReticleTextureIndex", 0, 6, 3),
        );
        self.add(
            false,
            ConfigOption::integer("crosshairTextureIndex", 0, 2, 0),
        );
        self.add(false, ConfigOption::integer("maxCrosshairOffset", 0, 11, 5));
        self.add(false, ConfigOption::boolean("reticleCameraZoom", false));
        self.add(
            false,
            ConfigOption::array(
                "targetColor",
                ConfigOption::double("element", 0.0, 1.0, 1.0),
                ",",
                "1.0,1.0,1.0",
            )
            .fixed_size(3),
        );
        self.add(
            false,
            ConfigOption::array(
                "noTargetColor",
                ConfigOption::double("element", 0.0, 1.0, 1.0),
                ",",
                "1.0,1.0,1.0",
            )
            .fixed_size(3),
        );
        self.add(
            false,
            ConfigOption::array(
                "objHighlightColor",
                ConfigOption::double("element", 0.0, 1.0, 1.0),
                ",",
                "0.98,0.56,0.11",
            )
            .fixed_size(3),
        );
        self.add(
            false,
            ConfigOption::array(
                "worldItemHighlightColor",
                ConfigOption::double("element", 0.0, 1.0, 1.0),
                ",",
                "0.5,1.0, 1.0",
            )
            .fixed_size(3),
        );
        self.add(
            false,
            ConfigOption::array(
                "workstationHighlightColor",
                ConfigOption::double("element", 0.0, 1.0, 1.0),
                ",",
                "0.56,0.98,0.11",
            )
            .fixed_size(3),
        );
        self.add(
            false,
            ConfigOption::array(
                "goodHighlightColor",
                ConfigOption::double("element", 0.0, 1.0, 1.0),
                ",",
                "0.0,1.0,0.0",
            )
            .fixed_size(3),
        );
        self.add(
            false,
            ConfigOption::array(
                "badHighlightColor",
                ConfigOption::double("element", 0.0, 1.0, 1.0),
                ",",
                "1.0,0.0,0.0",
            )
            .fixed_size(3),
        );
        self.add(false, ConfigOption::integer("iso_cursor", 0, 6, 5));
        self.add(false, ConfigOption::boolean("showCursorWhileAiming", false));
        self.add(false, ConfigOption::boolean("zoom", true));
        self.add(false, ConfigOption::integer("fontSize", 1, 6, 6));
        self.add(false, ConfigOption::integer("moodleSize", 1, 7, 7));
        self.add(false, ConfigOption::integer("sidebarSize", 1, 6, 6));
        self.add(
            false,
            ConfigOption::integer("actionProgressBarSize", 1, 4, 1),
        );
        self.add(
            false,
            ConfigOption::string_values("contextMenuFont", "Medium", &["Small", "Medium", "Large"]),
        );
        self.add(
            false,
            ConfigOption::string_values("codeFontSize", "Medium", &["Small", "Medium", "Large"]),
        );
        self.add(
            false,
            ConfigOption::string_values("inventoryFont", "Medium", &["Small", "Medium", "Large"]),
        );
        self.add(
            false,
            ConfigOption::integer("inventoryContainerSize", 1, 3, 1),
        );
        self.add(
            false,
            ConfigOption::string_values("tooltipFont", "Small", &["Small", "Medium", "Large"]),
        );
        self.add(false, ConfigOption::boolean("colorblindPatterns", false));
        self.add(false, ConfigOption::boolean("enableDyslexicFont", false));
        self.add(
            false,
            ConfigOption::boolean("disableLightningDuringStorms", false),
        );
        self.add(
            false,
            ConfigOption::string_values("measurementsFormat", "Metric", &["Imperial", "Metric"]),
        );
        self.add(false, ConfigOption::integer("clockFormat", 1, 2, 1));
        self.add(false, ConfigOption::integer("clockSize", 1, 2, 2));
        self.add(false, ConfigOption::boolean("clock24Hour", true));
        self.add(false, ConfigOption::boolean("vsync", false));
        self.add(false, ConfigOption::integer("soundVolume", 0, 10, 10));
        self.add(false, ConfigOption::integer("musicVolume", 0, 10, 10));
        self.add(false, ConfigOption::integer("ambientVolume", 0, 10, 5));
        self.add(false, ConfigOption::integer("jumpScareVolume", 0, 10, 10));
        self.add(false, ConfigOption::integer("musicActionStyle", 1, 2, 1));
        self.add(false, ConfigOption::integer("musicLibrary", 1, 3, 1));
        self.add(false, ConfigOption::boolean("voiceEnable", true));
        self.add(false, ConfigOption::integer("voiceMode", 1, 3, 3));
        self.add(false, ConfigOption::integer("voiceVADMode", 1, 4, 3));
        self.add(false, ConfigOption::integer("voiceAGCMode", 1, 3, 2));
        self.add(
            false,
            ConfigOption::string_max("voiceRecordDeviceName", "", 256),
        );
        self.add(false, ConfigOption::integer("voiceVolumeMic", 0, 10, 10));
        self.add(false, ConfigOption::integer("voiceVolumePlayers", 0, 10, 5));
        self.add(
            false,
            ConfigOption::integer("vehicleEngineVolume", 0, 10, 10),
        );
        self.add(false, ConfigOption::boolean("vehicleStreamerMode", false));
        self.add(false, ConfigOption::integer("reloadDifficulty", 1, 3, 2));
        self.add(false, ConfigOption::boolean("rackProgress", true));
        self.add(false, ConfigOption::integer("bloodDecals", 0, 10, 10));
        self.add(false, ConfigOption::boolean("focusloss", true));
        self.add(false, ConfigOption::boolean("mapViewPause", true));
        self.add(false, ConfigOption::boolean("borderless", false));
        self.add(false, ConfigOption::boolean("lockCursorToWindow", false));
        self.add(false, ConfigOption::boolean("textureCompression", false));
        self.add(false, ConfigOption::boolean("modelTextureMipmaps", false));
        self.add(false, ConfigOption::boolean("texture2x", true));
        self.add(false, ConfigOption::boolean("highResPlacedItems", true));
        self.add(false, ConfigOption::integer("maxTextureSize", 1, 4, 1));
        self.add(
            false,
            ConfigOption::integer("maxVehicleTextureSize", 1, 4, 2),
        );
        self.add(
            false,
            ConfigOption::string_values("screenFilter", "nearest", &["nearest", "linear"]),
        );
        self.add(
            false,
            ConfigOption::array(
                "zoomLevels1x",
                ConfigOption::integer("element", 25, 250, 100),
                ";",
                "",
            ),
        );
        self.add(
            false,
            ConfigOption::array(
                "zoomLevels2x",
                ConfigOption::integer("element", 25, 250, 100),
                ";",
                "",
            ),
        );
        self.add(
            false,
            ConfigOption::boolean("contentTranslationsEnabled", true),
        );
        self.add(false, ConfigOption::boolean("uiRenderOffscreen", true));
        self.add(false, ConfigOption::integer("uiRenderFPS", 10, 120, 60));
        self.add(false, ConfigOption::boolean("radialMenuKeyToggle", true));
        self.add(false, ConfigOption::boolean("reloadRadialInstant", false));
        self.add(false, ConfigOption::boolean("panCameraWhileAiming", true));
        self.add(false, ConfigOption::boolean("panCameraWhileDriving", false));
        self.add(false, ConfigOption::boolean("showChatTimestamp", false));
        self.add(false, ConfigOption::boolean("showChatTitle", false));
        self.add(
            false,
            ConfigOption::string_values("chatFontSize", "medium", &["small", "medium", "large"]),
        );
        self.add(false, ConfigOption::double("minChatOpaque", 0.0, 1.0, 1.0));
        self.add(false, ConfigOption::double("maxChatOpaque", 0.0, 1.0, 1.0));
        self.add(false, ConfigOption::double("chatFadeTime", 0.0, 10.0, 0.0));
        self.add(false, ConfigOption::boolean("chatOpaqueOnFocus", true));
        self.add(
            false,
            ConfigOption::boolean("temperatureDisplayCelsius", false),
        );
        self.add(false, ConfigOption::boolean("doVideoEffects", true));
        self.add(false, ConfigOption::boolean("doWindSpriteEffects", false));
        self.add(false, ConfigOption::boolean("doDoorSpriteEffects", true));
        self.add(false, ConfigOption::boolean("doContainerOutline", true));
        self.add(false, ConfigOption::boolean("renderPrecipIndoors", true));
        self.add(
            false,
            ConfigOption::double("precipitationSpeedMultiplier", 0.01, 1.0, 1.0),
        );
        self.add(false, ConfigOption::boolean("autoProneAtk", true));
        self.add(false, ConfigOption::boolean("3DGroundItem", true));
        self.add(false, ConfigOption::integer("renderPrecipitation", 1, 3, 1));
        self.add(false, ConfigOption::boolean("dblTapJogToSprint", false));
        self.add(false, ConfigOption::boolean("meleeOutline", false));
        self.add(
            false,
            ConfigOption::string_values(
                "cycleContainerKey",
                "shift",
                &[
                    "control",
                    "shift",
                    "control+shift",
                    "command",
                    "command+shift",
                ],
            ),
        );
        self.add(
            false,
            ConfigOption::boolean("dropItemsOnSquareCenter", false),
        );
        self.add(
            false,
            ConfigOption::boolean("timedActionGameSpeedReset", true),
        );
        self.add(
            false,
            ConfigOption::integer("shoulderButtonContainerSwitch", 1, 3, 1),
        );
        self.add(
            false,
            ConfigOption::integer("controllerButtonStyle", 1, 3, 1)
                .on_change(OnChange::ControllerButtonStyle),
        );
        self.add(
            false,
            ConfigOption::string_max("gamepadBindingPreset", "Original", 0)
                .on_change(OnChange::GamepadBindingPreset),
        );
        self.add(false, ConfigOption::boolean("progressBar", false));
        self.add(false, ConfigOption::string_max("language", "", 64));
        self.add(
            false,
            ConfigOption::array(
                "singleContextMenu",
                ConfigOption::boolean("element", false),
                ",",
                "",
            )
            .fixed_size(4),
        );
        self.add(false, ConfigOption::boolean("corpseShadows", true));
        self.add(
            false,
            ConfigOption::integer("simpleClothingTextures", 1, 3, 1),
        );
        self.add(false, ConfigOption::boolean("simpleWeaponTextures", false));
        self.add(false, ConfigOption::boolean("autoDrink", true));
        self.add(
            false,
            ConfigOption::boolean("autoRevealPrintMediaMapLocations", false),
        );
        self.add(false, ConfigOption::boolean("leaveKeyInIgnition", false));
        self.add(false, ConfigOption::boolean("autoWalkContainer", false));
        self.add(
            false,
            ConfigOption::integer("searchModeOverlayEffect", 1, 4, 1),
        );
        self.add(
            false,
            ConfigOption::integer("ignoreProneZombieRange", 1, 5, 2),
        );
        self.add(false, ConfigOption::boolean("showItemModInfo", true));
        self.add(false, ConfigOption::boolean("showCraftingXP", true));
        self.add(false, ConfigOption::boolean("showSurvivalGuide", true));
        self.add(
            false,
            ConfigOption::boolean("showFirstAnimalZoneInfo", true),
        );
        self.add(
            false,
            ConfigOption::boolean("enableLeftJoystickRadialMenu", true),
        );
        self.add(
            false,
            ConfigOption::boolean("macosIgnoreMouseWheelAcceleration", true),
        );
        self.add(
            false,
            ConfigOption::boolean("macosMapHorizontalMouseWheelToVertical", true),
        );
        self.add(false, ConfigOption::boolean("usePhysicsHitReaction", true));
        self.add(false, ConfigOption::integer("maxActiveRagdolls", 5, 60, 20));
        self.add(
            false,
            ConfigOption::double("worldMapBrightness", 0.0, 1.0, 1.0),
        );
        self.add(false, ConfigOption::boolean("showWelcomeMessage2", true));
        self.add(false, ConfigOption::boolean("showYourUsername", true));
        self.add(
            false,
            ConfigOption::boolean("populateServerListOnStart", false),
        );
        self.add(
            false,
            ConfigOption::array(
                "mpTextColor",
                ConfigOption::double("element", 0.0, 1.0, 1.0),
                ",",
                "",
            )
            .fixed_size(3),
        );
        self.add(false, ConfigOption::string_max("seenNews", "", 64));
        self.add(false, ConfigOption::boolean("toggleToAim", false));
        self.add(false, ConfigOption::boolean("toggleToRun", false));
        self.add(false, ConfigOption::boolean("toggleToSprint", true));
        self.add(false, ConfigOption::boolean("celsius", false));
        self.add(false, ConfigOption::integer("perfSkybox", 0, 2, 1));
        self.add(false, ConfigOption::integer("perfPuddles", 0, 3, 0));
        self.add(false, ConfigOption::boolean("bPerfReflections", true));
        self.add(
            true,
            ConfigOption::array(
                "controller",
                ConfigOption::string_max("element", "", 256),
                ",",
                "",
            )
            .multi_line(true),
        );
        self.add(true, ConfigOption::boolean("doneNewSaveFolder", false));
        self.add(true, ConfigOption::integer("fogQuality", 0, 2, 0));
        self.add(true, ConfigOption::integer("viewConeOpacity", 0, 5, 3));
        self.add(true, ConfigOption::boolean("gotNewBelt", false));
        self.add(true, ConfigOption::integer("lightFPS", 5, 60, 15));
        self.add(true, ConfigOption::integer("frameRate", 24, 244, 60));
        self.add(true, ConfigOption::integer("puddles", 0, 2, 0));
        self.add(true, ConfigOption::boolean("riversideDone", false));
        self.add(true, ConfigOption::boolean("rosewoodSpawnDone", false));
        self.add(true, ConfigOption::integer("height", 0, 16384, 720));
        self.add(true, ConfigOption::integer("width", 0, 16384, 1280));
        self.add(
            true,
            ConfigOption::boolean("showFirstTimeSearchTutorial", true),
        );
        self.add(
            true,
            ConfigOption::boolean("showFirstTimeSneakTutorial", true),
        );
        self.add(
            true,
            ConfigOption::double("shownWelcomeMessageVersion", 0.0, 1000.0, 0.0),
        );
        self.add(
            true,
            ConfigOption::integer("termsOfServiceVersion", -1, 1000, -1),
        );
        self.add(true, ConfigOption::boolean("tieredZombieUpdates", true));
        self.add(true, ConfigOption::boolean("tutorialDone", false));
        self.add(true, ConfigOption::boolean("updateSneakButton", true));
        self.add(true, ConfigOption::boolean("uncappedFPS", true));
        self.add(true, ConfigOption::boolean("vehiclesWarningShow", false));
        self.add(true, ConfigOption::integer("water", 0, 2, 0));
        self.add(false, ConfigOption::boolean("fullScreen", false));
        self.add(
            false,
            ConfigOption::array("autozoom", ConfigOption::boolean("element", false), ",", "")
                .fixed_size(4),
        );
    }

    /// Look up a real or fake option by its `options.ini` name.
    pub fn option(&self, name: &str) -> Option<&ConfigOption> {
        if let Some(&i) = self.option_by_name.get(name) {
            return Some(&self.options[i]);
        }
        self.fake_option_by_name
            .get(name)
            .map(|&i| &self.fake_options[i])
    }

    pub fn option_mut(&mut self, name: &str) -> Option<&mut ConfigOption> {
        if let Some(&i) = self.option_by_name.get(name) {
            return Some(&mut self.options[i]);
        }
        match self.fake_option_by_name.get(name) {
            Some(&i) => Some(&mut self.fake_options[i]),
            None => None,
        }
    }

    fn opt(&self, name: &str) -> &ConfigOption {
        self.option(name)
            .unwrap_or_else(|| panic!("unknown Core option {name}"))
    }

    fn opt_mut(&mut self, name: &str) -> &mut ConfigOption {
        self.option_mut(name)
            .unwrap_or_else(|| panic!("unknown Core option {name}"))
    }

    /// The real (saved, non-fake) options in declaration order.
    pub fn options(&self) -> &[ConfigOption] {
        &self.options
    }

    /// The fake options (also saved; Java keeps them in a separate list).
    pub fn fake_options(&self) -> &[ConfigOption] {
        &self.fake_options
    }

    // ---- events / requests ------------------------------------------------

    fn collect_option_changes(&mut self) {
        let mut fired = Vec::new();
        for o in self.options.iter_mut().chain(self.fake_options.iter_mut()) {
            if o.take_changed() {
                if let Some(cb) = o.on_change_callback() {
                    fired.push(cb);
                }
            }
        }
        for cb in fired {
            match cb {
                OnChange::ControllerButtonStyle => self.on_option_controller_button_style_changed(),
                OnChange::GamepadBindingPreset => self.on_option_gamepad_binding_preset_changed(),
            }
        }
    }

    /// Drain the Lua events queued since the last call (including those of
    /// option on-change callbacks), in the order Java triggers them.
    pub fn take_events(&mut self) -> Vec<CoreEvent> {
        self.collect_option_changes();
        std::mem::take(&mut self.pending_events)
    }

    /// Drain the engine requests queued since the last call.
    pub fn take_requests(&mut self) -> Vec<CoreRequest> {
        std::mem::take(&mut self.pending_requests)
    }

    fn trigger_event(&mut self, name: &'static str, args: Vec<EventArg>) {
        self.collect_option_changes();
        self.pending_events.push(CoreEvent { name, args });
    }

    fn rand_next(&mut self, max: i32) -> i32 {
        // xorshift64*; `Rand.Next(max)` is [0, max).
        if max <= 0 {
            return 0;
        }
        let mut x = self.rng;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.rng = x;
        (x.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 33) as i32 % max
    }

    fn rand_next_range(&mut self, min: i32, max: i32) -> i32 {
        if max <= min {
            return min;
        }
        min + self.rand_next(max - min)
    }

    // ---- file locations ---------------------------------------------------

    /// Set the Zomboid user directory (`ZomboidFileSystem.getCacheDir()`);
    /// `options.ini` is read from and written to it.
    pub fn set_cache_dir(&mut self, dir: impl Into<PathBuf>) {
        self.cache_dir = Some(dir.into());
    }

    pub fn cache_dir(&self) -> Option<&Path> {
        self.cache_dir.as_deref()
    }

    /// Override the `options.ini` path (defaults to `<cache dir>/options.ini`).
    pub fn set_options_path(&mut self, path: impl Into<PathBuf>) {
        self.options_path = Some(path.into());
    }

    /// `ZomboidFileSystem.getCacheDirSub("options.ini")`.
    pub fn options_path(&self) -> Option<PathBuf> {
        self.options_path
            .clone()
            .or_else(|| self.cache_dir.as_ref().map(|d| d.join("options.ini")))
    }

    // ---- misc state -------------------------------------------------------

    pub fn get_option_count(&self) -> i32 {
        self.options.len() as i32
    }

    pub fn get_option_by_index(&self, index: usize) -> Option<&ConfigOption> {
        self.options.get(index)
    }

    pub fn is_multi_thread(&self) -> bool {
        true
    }

    pub fn set_multi_thread(&mut self, _val: bool) {}

    pub fn set_challenge(&mut self, b: bool) {
        self.challenge = b;
    }

    pub fn is_challenge(&self) -> bool {
        self.challenge
    }

    pub fn get_challenge_id(&self) -> Option<String> {
        self.challenge_id.clone()
    }

    pub fn set_option_tiered_zombie_updates(&mut self, val: bool) {
        self.opt_mut("tieredZombieUpdates").set_bool(val);
        self.moving_object_update_scheduler_enabled = val;
    }

    /// `setFramerate(index)`: the options-screen framerate choice.
    pub fn set_framerate(&mut self, index: i32) {
        self.performance.uncapped_fps = index == 1;
        let fps = match index {
            1 => 60,
            2 => 244,
            3 => 240,
            4 => 165,
            5 => 144,
            6 => 120,
            7 => 95,
            8 => 90,
            9 => 75,
            10 => 60,
            11 => 55,
            12 => 45,
            13 => 30,
            14 => 24,
            _ => return,
        };
        self.performance.lock_fps = fps;
    }

    pub fn is_use_game_viewport(&self) -> bool {
        self.use_game_viewport && self.debug && self.im_gui
    }

    pub fn is_im_gui(&self) -> bool {
        self.debug && self.im_gui
    }

    pub fn is_use_viewports(&self) -> bool {
        self.debug && self.im_gui && self.use_viewports
    }

    /// TODO(render): `SceneShaderStore.weatherShader != null`.
    pub fn loaded_shader(&self) -> bool {
        false
    }

    /// TODO(render): queried from GL; `-1` until the renderer sets it.
    pub fn get_gl_major_version(&self) -> i32 {
        self.gl_major_version
    }

    pub fn get_gl_version(&self) -> Option<String> {
        self.gl_version.clone()
    }

    /// `getOpenGLVersions()` given `GL_VERSION`. TODO(render): called by
    /// the renderer.
    pub fn set_open_gl_version(&mut self, version: &str) {
        self.gl_version = Some(version.to_string());
        self.gl_major_version = version.chars().next().map_or(-1, |c| c as i32 - 48);
    }

    pub fn get_use_shaders(&self) -> bool {
        true
    }

    pub fn set_use_shaders(&mut self, _b: bool) {}

    pub fn get_perf_skybox_on_load(&self) -> i32 {
        self.i_perf_skybox
    }

    pub fn get_perf_reflections_on_load(&self) -> bool {
        self.perf_reflections
    }

    pub fn get_perf_puddles_on_load(&self) -> i32 {
        self.i_perf_puddles
    }

    /// TODO(render): `!Display.capabilities.OpenGL33`.
    pub fn get_use_open_gl21(&self) -> bool {
        !self.gl33_supported
    }

    pub fn get_vid_mem(&self) -> i32 {
        if self.safe_mode {
            return 5;
        }
        self.vid_mem
    }

    pub fn set_vid_mem(&mut self, mem: i32) {
        self.vid_mem = mem;
        if let Err(e) = self.save_options() {
            debug_log(&format!("{e}"));
        }
    }

    /// TODO(render): `SceneShaderStore.shaderOptionsChanged()`.
    pub fn shaders_option_changed(&mut self) {}

    /// TODO(render): `SceneShaderStore.initGlobalShader()`.
    pub fn init_global_shader(&mut self) {}

    /// TODO(render): `SceneShaderStore.initShaders()`, puddles, water.
    pub fn init_shaders(&mut self) {}

    pub fn get_game_mode(&self) -> String {
        self.game_mode.clone()
    }

    pub fn set_game_mode(&mut self, game_mode: &str) {
        self.game_mode = game_mode.to_string();
        self.add_zombie_on_cell_load = game_mode != "Tutorial" && game_mode != "LastStand";
    }

    pub fn get_debug(&self) -> bool {
        self.debug
    }

    /// Static `Core.setFullScreen`.
    pub fn set_full_screen(&mut self, b: bool) {
        self.opt_mut("fullScreen").set_bool(b);
    }

    /// `TakeScreenshot()`.
    pub fn take_screenshot(&mut self) {
        self.take_screenshot_sized(256, 256, 1028);
    }

    /// `TakeScreenshot(width, height, readBuffer)`: centred on player 0's
    /// view. TODO(render): `IsoCamera` per-player viewports.
    pub fn take_screenshot_sized(&mut self, width: i32, height: i32, read_buffer: i32) {
        let screen_width = self.width;
        let screen_height = self.height;
        let w = width.min(screen_width);
        let h = height.min(screen_height);
        let x = screen_width / 2 - w / 2;
        let y = screen_height / 2 - h / 2;
        self.take_screenshot_at(x, y, w, h, read_buffer);
    }

    /// `TakeScreenshot(x, y, width, height, readBuffer)`. TODO(render).
    pub fn take_screenshot_at(
        &mut self,
        x: i32,
        y: i32,
        width: i32,
        height: i32,
        read_buffer: i32,
    ) {
        self.pending_requests.push(CoreRequest::Screenshot {
            x,
            y,
            width,
            height,
            read_buffer,
        });
    }

    /// `TakeFullScreenshot(filename)`. Errs like the Java
    /// `IllegalArgumentException` for names containing `..`.
    pub fn take_full_screenshot(&mut self, filename: Option<&str>) -> Result<(), String> {
        if let Some(f) = filename {
            if f.contains("..") {
                return Err(format!("Unable to save options to filename: {f}"));
            }
        }
        self.pending_requests
            .push(CoreRequest::FullScreenshot(filename.map(str::to_string)));
        Ok(())
    }

    /// `supportsFBO()`. TODO(render): FBO support is assumed; creating the
    /// offscreen buffer only sets up its zoom levels.
    pub fn supports_fbo(&mut self) -> bool {
        if self.safe_mode {
            self.offscreen_buffer.zoom_enabled = false;
            return false;
        }
        if !self.supports_fbo {
            return false;
        }
        if self.offscreen_buffer.created {
            return true;
        }
        self.setup_multi_fbo();
        true
    }

    fn setup_multi_fbo(&mut self) {
        let levels = self.zoom_levels_option_string();
        self.offscreen_buffer
            .set_zoom_levels_from_option(Some(&levels));
        self.offscreen_buffer.create();
    }

    fn zoom_levels_option_string(&self) -> String {
        if self.tile_scale == 2 {
            self.opt("zoomLevels2x").get_value_as_string()
        } else {
            self.opt("zoomLevels1x").get_value_as_string()
        }
    }

    pub fn move_method_toggle(&mut self) {
        self.alt_move_method = !self.alt_move_method;
    }

    /// TODO(render): `EndFrameText`.
    pub fn end_frame_text(&mut self, _player: i32) {
        self.frame_stage = 2;
    }

    /// TODO(render): `EndFrame(nPlayer)`.
    pub fn end_frame_player(&mut self, _player: i32) {
        self.frame_stage = 2;
    }

    /// TODO(render): `EndFrame()`.
    pub fn end_frame(&mut self) {}

    /// `EndFrameUI` minus rendering: the moodle blink alpha animation.
    /// TODO(render): UI FBO, cursor, imgui.
    pub fn end_frame_ui(&mut self, thirty_fps_multiplier: f32) {
        if !self.blink_alpha_increase {
            self.blink_alpha -= 0.07 * thirty_fps_multiplier;
            if self.blink_alpha < 0.15 {
                self.blink_alpha = 0.15;
                self.blink_alpha_increase = true;
            }
        } else {
            self.blink_alpha += 0.07 * thirty_fps_multiplier;
            if self.blink_alpha > 1.0 {
                self.blink_alpha = 1.0;
                self.blink_alpha_increase = false;
            }
        }
        if self.use_ui_fbo && self.ui_render_this_frame {
            self.ui_render_accumulator %= 30.0 / self.get_option_ui_render_fps() as f32;
        }
        self.frame_stage = 0;
    }

    /// Static `UnfocusActiveTextEntryBox()`.
    pub fn unfocus_active_text_entry_box(&mut self) {
        if let Some(b) = &self.current_text_entry_box {
            if !b.ui_name.contains("chat text entry") {
                self.current_text_entry_box = None;
            }
        }
    }

    /// `IsoCamera.getScreenWidth(player)` for the split-screen layout.
    fn camera_screen_width(&self, _player: i32) -> i32 {
        // TODO(render): IsoCamera split-screen viewports.
        if self.num_players > 1 {
            self.width / 2
        } else {
            self.width
        }
    }

    fn camera_screen_height(&self, _player: i32) -> i32 {
        if self.num_players > 2 {
            self.height / 2
        } else {
            self.height
        }
    }

    /// `getOffscreenWidth(playerIndex)` (`MultiTextureFBO2.getWidth`).
    pub fn get_offscreen_width(&self, player: i32) -> i32 {
        let p = player.clamp(0, 3) as usize;
        (self.camera_screen_width(player) as f32
            * self
                .offscreen_buffer
                .get_display_zoom(p, self.width, self.initial_width)
            * (self.tile_scale as f32 / 2.0)) as i32
    }

    pub fn get_offscreen_height(&self, player: i32) -> i32 {
        let p = player.clamp(0, 3) as usize;
        (self.camera_screen_height(player) as f32
            * self
                .offscreen_buffer
                .get_display_zoom(p, self.width, self.initial_width)
            * (self.tile_scale as f32 / 2.0)) as i32
    }

    /// TODO(render): the FBO texture size when one exists.
    pub fn get_offscreen_true_width(&self) -> i32 {
        self.width
    }

    pub fn get_offscreen_true_height(&self) -> i32 {
        self.height
    }

    pub fn get_screen_height(&self) -> i32 {
        self.height
    }

    pub fn get_screen_width(&self) -> i32 {
        self.width
    }

    /// `setResolutionAndFullScreen`. TODO(render): the display mode change
    /// is queued; the new size is assumed to take.
    pub fn set_resolution_and_full_screen(&mut self, w: i32, h: i32, full_screen: bool) {
        if !self.is_use_game_viewport() {
            self.set_display_mode(w, h, full_screen);
        }
        self.set_screen_size(w, h);
    }

    /// `setResolution("W x H")`.
    pub fn set_resolution(&mut self, res: &str) -> Result<(), String> {
        let bits = java_split(res, "x");
        let parse = |s: Option<&&str>| s.and_then(|s| java_parse_int(java_trim(s), 10));
        let (Some(w), Some(h)) = (parse(bits.first()), parse(bits.get(1))) else {
            return Err(format!("For input string: \"{res}\""));
        };
        if !self.is_use_game_viewport() {
            let fs = self.opt("fullScreen").get_bool();
            self.set_display_mode(w, h, fs);
        }
        self.set_screen_size(w, h);
        if let Err(e) = self.save_options() {
            debug_log(&format!("{e}"));
        }
        Ok(())
    }

    /// Static `setDisplayMode`. TODO(render).
    pub fn set_display_mode(&mut self, width: i32, height: i32, fullscreen: bool) {
        self.pending_requests.push(CoreRequest::DisplayMode {
            width,
            height,
            fullscreen,
        });
    }

    pub fn is_default_options(&self) -> bool {
        !self.loaded_options
    }

    pub fn is_dedicated(&self) -> bool {
        self.is_server
    }

    /// Static `getMyDocumentFolder()`: the cache dir.
    pub fn get_my_document_folder(&self) -> Option<String> {
        self.cache_dir
            .as_ref()
            .map(|d| d.to_string_lossy().into_owned())
    }

    /// `setWindowed(b)` (which, as in Java, sets `fullScreen = b`).
    pub fn set_windowed(&mut self, b: bool) {
        if self.is_use_game_viewport() {
            return;
        }
        if b != self.opt("fullScreen").get_bool() {
            let (w, h) = (self.width, self.height);
            self.set_display_mode(w, h, b);
        }
        self.opt_mut("fullScreen").set_bool(b);
        if let Err(e) = self.save_options() {
            debug_log(&format!("{e}"));
        }
    }

    /// `getScreenModes()`: lines of `<cache>/Lua/screenresolution.ini`,
    /// created from the display modes on first use.
    pub fn get_screen_modes(&self) -> Vec<String> {
        let mut out = Vec::new();
        let Some(dir) = &self.cache_dir else {
            // No cache dir: what Java would write to the file.
            for (w, h) in &self.available_display_modes {
                let r = format!("{w} x {h}");
                if !out.contains(&r) {
                    out.push(r);
                }
            }
            return out;
        };
        let lua_dir = dir.join("Lua");
        let _ = std::fs::create_dir_all(&lua_dir);
        let file = lua_dir.join("screenresolution.ini");
        if !file.exists() {
            let mut text = String::new();
            for (w, h) in &self.available_display_modes {
                let r = format!("{w} x {h}");
                if out.contains(&r) {
                    continue;
                }
                text.push_str(&r);
                text.push_str(" \r\n");
                out.push(r);
            }
            if let Err(e) = std::fs::write(&file, text) {
                debug_log(&format!("{e}"));
            }
        } else if let Ok(bytes) = std::fs::read(&file) {
            let text = String::from_utf8_lossy(&bytes);
            for line in java_lines(&text) {
                out.push(java_trim(line).to_string());
            }
        }
        out
    }

    /// `isDoingTextEntry()`.
    pub fn is_doing_text_entry(&self) -> bool {
        match &self.current_text_entry_box {
            Some(b) => b.editable && b.doing_text_entry,
            None => false,
        }
    }

    /// TODO(input): `updateKeyboard()` feeds key events to the focused
    /// text entry box; the UI port does that itself.
    pub fn update_keyboard(&mut self) {}

    /// `quit()`: in game, flags `exiting`; otherwise saves the options and
    /// requests process exit ([`CoreRequest::Exit`]).
    pub fn quit(&mut self) {
        if self.player_exists {
            self.exiting = true;
        } else {
            if let Err(e) = self.save_options() {
                debug_log(&format!("{e}"));
            }
            // TODO(net): GameClient.instance.Shutdown(), SteamUtils.shutdown().
            self.pending_requests.push(CoreRequest::Exit);
        }
    }

    pub fn exit_to_menu(&mut self) {
        self.exiting = true;
    }

    pub fn quit_to_desktop(&mut self) {
        self.close_requested = true;
    }

    /// `supportRes(width, height)`. TODO(render): fullscreen capability.
    pub fn support_res(&self, width: i32, height: i32) -> bool {
        self.available_display_modes
            .iter()
            .any(|&(w, h)| w == width && h == height)
    }

    /// `init(width, height)` minus display creation (TODO(render)).
    pub fn init(&mut self, width: i32, height: i32) {
        self.width = width;
        self.height = height;
    }

    /// Static `setInitialSize()`.
    pub fn set_initial_size(&mut self) {
        self.initial_height = self.height as f32;
        self.initial_width = self.width as f32;
    }

    /// `setScreenSize(width, height)`: queues `OnResolutionChange`.
    pub fn set_screen_size(&mut self, width: i32, height: i32) {
        if self.width != width || self.height != height {
            let old_width = self.width;
            let old_height = self.height;
            debug_log(&format!(
                "Screen resolution changed from {old_width}x{old_height} to {width}x{height} fullScreen={}",
                self.opt("fullScreen").get_value_as_string()
            ));
            self.width = width;
            self.height = height;
            if self.offscreen_buffer.created {
                self.offscreen_buffer.destroy();
                self.setup_multi_fbo();
            }
            self.trigger_event(
                "OnResolutionChange",
                vec![
                    EventArg::Num(old_width as f64),
                    EventArg::Num(old_height as f64),
                    EventArg::Num(width as f64),
                    EventArg::Num(height as f64),
                ],
            );
            // TODO(world): IsoPlayer.players[i].dirtyRecalcGridStackTime = 2.
        }
    }

    /// TODO(render): `GL_EXT_texture_compression_latc`.
    pub fn support_compressed_textures(&self) -> bool {
        false
    }

    /// TODO(render): `StartFrame()`.
    pub fn start_frame(&mut self) {
        self.frame_stage = 1;
    }

    /// TODO(render): `StartFrame(nPlayer, clear)`.
    pub fn start_frame_player(&mut self, _player: i32, _clear: bool) {
        self.frame_stage = 1;
    }

    /// TODO(render): `StartFrameText`.
    pub fn start_frame_text(&mut self, _player: i32) {
        self.frame_stage = 2;
    }

    /// `StartFrameUI()` minus rendering: whether the UI is redrawn this
    /// frame. TODO(render).
    pub fn start_frame_ui(&mut self, thirty_fps_multiplier: f32) -> bool {
        let mut render_this_frame = true;
        if self.use_ui_fbo {
            self.ui_render_accumulator += thirty_fps_multiplier;
            self.ui_render_this_frame =
                self.ui_render_accumulator >= 30.0 / self.get_option_ui_render_fps() as f32;
            if !self.ui_render_this_frame {
                render_this_frame = false;
            }
        }
        self.frame_stage = 3;
        render_this_frame
    }

    // ---- key bindings -----------------------------------------------------

    pub fn reinit_key_maps(&mut self) {
        self.key_maps = HashMap::new();
        self.key_binding_by_key_value.clear();
    }

    fn key_down_raw(&self, k: i32) -> bool {
        self.keys_down_raw.contains(&k)
    }

    /// `invalidBindingShiftCtrl(keyB)`.
    pub fn invalid_binding_shift_ctrl(&self, key_b: &KeyBinding) -> bool {
        let shift_down = self.key_down_raw(42) || self.key_down_raw(54);
        let ctrl_down = self.key_down_raw(29) || self.key_down_raw(157);
        let alt_down = self.key_down_raw(56) || self.key_down_raw(184);
        if key_b.alt_key >= 10000 && self.mouse_buttons_down.contains(&(key_b.alt_key - 10000)) {
            return false;
        }
        if key_b.key_value >= 10000 && self.mouse_buttons_down.contains(&(key_b.key_value - 10000))
        {
            return false;
        }
        if key_b.key_value == 0 && key_b.alt_key == 0 {
            return true;
        }
        if key_b.key_value == 42 || key_b.key_value == 54 {
            return !shift_down;
        }
        if key_b.key_value == 29 || key_b.key_value == 157 {
            return !ctrl_down;
        }
        if key_b.key_value == 56 || key_b.key_value == 184 {
            return !alt_down;
        }
        if key_b.shift && !shift_down {
            return true;
        }
        if key_b.ctrl && !ctrl_down {
            return true;
        }
        if key_b.alt && !alt_down {
            return true;
        }
        let Some(list) = self.key_binding_by_key_value.get(&key_b.key_value) else {
            return false;
        };
        for other in list {
            if other == key_b
                || (other.shift == key_b.shift
                    && other.ctrl == key_b.ctrl
                    && other.alt == key_b.alt)
            {
                continue;
            }
            let matches_input =
                key_b.shift == shift_down && key_b.ctrl == ctrl_down && key_b.alt == alt_down;
            let other_matches_input =
                other.shift == shift_down && other.ctrl == ctrl_down && other.alt == alt_down;
            if matches_input || !other_matches_input {
                continue;
            }
            return true;
        }
        false
    }

    /// `isKey(keyName, key)`.
    pub fn is_key(&self, key_name: &str, key: i32) -> bool {
        let key_b = self.get_key_binding(key_name);
        if self.invalid_binding_shift_ctrl(&key_b) {
            return false;
        }
        key_b.key_value == key || key_b.alt_key == key
    }

    pub fn get_key(&self, key_name: &str) -> i32 {
        self.get_key_binding(key_name).key_value
    }

    pub fn get_key_binding(&self, key_name: &str) -> KeyBinding {
        self.key_maps
            .get(key_name)
            .cloned()
            .unwrap_or_else(KeyBinding::empty)
    }

    /// `getKeyBinding(int keyId)`: the first binding on that key.
    pub fn get_key_binding_by_key(&self, key_id: i32) -> KeyBinding {
        match self.key_binding_by_key_value.get(&key_id) {
            Some(l) if !l.is_empty() => l[0].clone(),
            _ => KeyBinding::empty(),
        }
    }

    pub fn add_key_binding(
        &mut self,
        key_name: &str,
        key: i32,
        alt_key: i32,
        shift: bool,
        ctrl: bool,
        alt: bool,
    ) {
        let binding = KeyBinding {
            name: key_name.to_string(),
            key_value: key,
            alt_key,
            shift,
            ctrl,
            alt,
        };
        if let Some(previous) = self.key_maps.insert(key_name.to_string(), binding.clone()) {
            if let Some(list) = self.key_binding_by_key_value.get_mut(&previous.key_value) {
                if let Some(i) = list.iter().position(|b| *b == previous) {
                    list.remove(i);
                }
            }
        }
        let list = self.key_binding_by_key_value.entry(key).or_default();
        if list.contains(&binding) {
            return;
        }
        list.push(binding);
    }

    pub fn get_alt_key(&self, key_name: &str) -> i32 {
        self.get_key_binding(key_name).alt_key
    }

    /// All current bindings by name.
    pub fn key_maps(&self) -> &HashMap<String, KeyBinding> {
        &self.key_maps
    }

    // ---- version ----------------------------------------------------------

    pub fn is_last_stand(&self) -> bool {
        self.last_stand
    }

    pub fn get_version(&self) -> String {
        format!(
            "{} {}",
            self.get_game_and_build_version(),
            self.get_git_revision_string()
        )
    }

    pub fn get_game_and_build_version(&self) -> String {
        format!("{}.0", self.get_game_version())
    }

    pub fn get_bullet_version(&self) -> String {
        self.bullet_version.clone()
    }

    /// TODO(dev): `DevMainScreenState.getDevSha()` in dev builds.
    pub fn get_git_sha(&mut self) -> String {
        self.git_sha
            .get_or_insert_with(|| GIT_REVISION.to_string())
            .clone()
    }

    pub fn get_git_revision(&mut self) -> String {
        self.git_revision_string
            .get_or_insert_with(|| GIT_REVISION[..GIT_REVISION.len().min(20)].to_string())
            .clone()
    }

    /// Static `getGitRevisionString()`.
    pub fn get_git_revision_string(&self) -> String {
        if self.is_dev {
            if let Some(s) = &self.git_revision_string {
                return s.clone();
            }
        }
        GIT_REVISION.to_string()
    }

    pub fn get_game_version(&self) -> GameVersion {
        GameVersion::new(GAME_VERSION.0, GAME_VERSION.1, GAME_VERSION.2)
    }

    pub fn get_break_mod_game_version(&self) -> GameVersion {
        GameVersion::new(42, 0, "")
    }

    pub fn get_steam_server_version(&self) -> String {
        self.steam_server_version.clone()
    }

    pub fn get_version_number(&self) -> String {
        self.get_game_version().to_string()
    }

    /// TODO(input): `DoFrameReady()` = `updateKeyboard()`.
    pub fn do_frame_ready(&mut self) {
        self.update_keyboard();
    }

    // ---- zoom -------------------------------------------------------------

    /// TODO(render): `IsoCamera.frameState.playerIndex`; player 0.
    pub fn get_current_player_zoom(&self) -> f32 {
        self.get_zoom(0)
    }

    pub fn get_zoom(&self, player: i32) -> f32 {
        let p = player.clamp(0, 3) as usize;
        self.offscreen_buffer
            .get_display_zoom(p, self.width, self.initial_width)
            * (self.tile_scale as f32 / 2.0)
    }

    pub fn get_next_zoom(&self, player: i32, del: i32) -> f32 {
        self.offscreen_buffer
            .get_next_zoom(player.clamp(0, 3) as usize, del)
    }

    pub fn get_min_zoom(&self) -> f32 {
        self.offscreen_buffer.get_min_zoom() * (self.tile_scale as f32 / 2.0)
    }

    pub fn get_max_zoom(&self) -> f32 {
        self.offscreen_buffer.get_max_zoom() * (self.tile_scale as f32 / 2.0)
    }

    pub fn do_zoom_scroll(&mut self, player: i32, del: i32) {
        self.offscreen_buffer
            .do_zoom_scroll(player.clamp(0, 3) as usize, del);
    }

    pub fn get_save_folder(&self) -> Option<String> {
        self.save_folder.clone()
    }

    /// `zoomOptionChanged(inGame)`. TODO(render): the in-game branch runs
    /// on the render thread in Java.
    pub fn zoom_option_changed(&mut self, in_game: bool) {
        if !in_game {
            self.safe_mode = self.safe_mode_forced;
            self.offscreen_buffer.zoom_enabled = self.get_option_zoom() && !self.safe_mode_forced;
            return;
        }
        self.offscreen_buffer.zoom_enabled = self.get_option_zoom();
        if self.safe_mode_forced {
            self.offscreen_buffer.destroy();
            self.safe_mode = true;
            self.supports_fbo = false;
        } else {
            self.safe_mode = false;
            self.supports_fbo = true;
            self.supports_fbo();
        }
        debug_log(&format!(
            "SafeMode is {}",
            if self.safe_mode { "on" } else { "off" }
        ));
    }

    pub fn zoom_levels_changed(&mut self) {
        if self.offscreen_buffer.created {
            self.offscreen_buffer.destroy();
            self.zoom_option_changed(true);
        }
    }

    pub fn is_zoom_enabled(&self) -> bool {
        self.offscreen_buffer.zoom_enabled
    }

    /// `setZoomEnalbed` (sic).
    pub fn set_zoom_enalbed(&mut self, val: bool) {
        self.offscreen_buffer.zoom_enabled = val;
    }

    pub fn init_fbos(&mut self) {
        self.offscreen_buffer.zoom_enabled = self.get_option_zoom();
        if self.safe_mode_forced {
            self.safe_mode = true;
        } else {
            self.supports_fbo();
        }
        debug_log(&format!(
            "SafeMode is {}",
            if self.safe_mode { "on" } else { "off" }
        ));
    }

    /// `getAutoZoom(playerIndex)`; `None` where Java throws
    /// `IndexOutOfBoundsException`.
    pub fn get_auto_zoom(&self, player: i32) -> Option<bool> {
        let a = self.opt("autozoom").as_array();
        usize::try_from(player)
            .ok()
            .filter(|&i| i < a.size())
            .map(|i| a.get_element(i).get_bool())
    }

    pub fn set_auto_zoom(&mut self, player: i32, auto: bool) -> Option<()> {
        let i = usize::try_from(player).ok()?;
        let a = self.opt_mut("autozoom").as_array_mut();
        if i >= a.size() {
            return None;
        }
        a.get_element_mut(i).set_bool(auto);
        if i < 4 {
            self.offscreen_buffer.auto_zoom[i] = auto;
        }
        Some(())
    }

    pub fn set_option_vsync(&mut self, sync: bool) {
        self.opt_mut("vsync").set_bool(sync);
        self.pending_requests.push(CoreRequest::VSync(sync));
    }

    pub fn get_real_option_sound_volume(&self) -> f32 {
        self.get_option_sound_volume() as f32 / 10.0
    }

    /// TODO(sound): `SoundManager.setSoundVolume(v / 10)`.
    pub fn set_option_sound_volume(&mut self, volume: i32) {
        self.opt_mut("soundVolume").set_int(volume);
    }

    /// TODO(sound): `SoundManager.setMusicVolume(v / 10)`.
    pub fn set_option_music_volume(&mut self, volume: i32) {
        self.opt_mut("musicVolume").set_int(volume);
    }

    /// TODO(sound): `SoundManager.setAmbientVolume(v / 10)`.
    pub fn set_option_ambient_volume(&mut self, volume: i32) {
        self.opt_mut("ambientVolume").set_int(volume);
    }

    /// TODO(sound): `SoundManager.setVehicleEngineVolume(v / 10)`.
    pub fn set_option_vehicle_engine_volume(&mut self, volume: i32) {
        self.opt_mut("vehicleEngineVolume").set_int(volume);
    }

    pub fn set_option_voice_enable(&mut self, option: bool) {
        self.set_option_voice_enable_restart(option, true);
    }

    /// `setOptionVoiceEnable(option, bRestartClient)`. TODO(voice):
    /// `VoiceManager.VoiceRestartClient`.
    pub fn set_option_voice_enable_restart(&mut self, option: bool, _restart_client: bool) {
        if self.get_option_voice_enable() != option {
            self.opt_mut("voiceEnable").set_bool(option);
        }
    }

    /// TODO(voice): `VoiceManager.setMode`.
    pub fn set_option_voice_mode(&mut self, option: i32) {
        self.opt_mut("voiceMode").set_int(option);
    }

    /// TODO(voice): `VoiceManager.setVADMode`.
    pub fn set_option_voice_vad_mode(&mut self, option: i32) {
        self.opt_mut("voiceVADMode").set_int(option);
    }

    /// TODO(voice): `VoiceManager.setAGCMode`.
    pub fn set_option_voice_agc_mode(&mut self, option: i32) {
        self.opt_mut("voiceAGCMode").set_int(option);
    }

    /// TODO(voice): `VoiceManager.setVolumeMic`.
    pub fn set_option_voice_volume_mic(&mut self, option: i32) {
        self.opt_mut("voiceVolumeMic").set_int(option);
    }

    /// TODO(voice): `VoiceManager.setVolumePlayers`.
    pub fn set_option_voice_volume_players(&mut self, option: i32) {
        self.opt_mut("voiceVolumePlayers").set_int(option);
    }

    /// TODO(voice): `VoiceManager.UpdateRecordDevice`.
    pub fn set_option_voice_record_device_name(&mut self, option: Option<&str>) {
        self.opt_mut("voiceRecordDeviceName").set_string(option);
    }

    /// `getOptionVoiceRecordDevice()`: 1-based index of the selected FMOD
    /// record driver, 0 if none.
    pub fn get_option_voice_record_device(&self) -> i32 {
        if self.sound_disabled || self.voice.voip_disabled {
            return 0;
        }
        let name = self.get_option_voice_record_device_name();
        for (i, d) in self.voice.record_driver_names.iter().enumerate() {
            if *d == name {
                return i as i32 + 1;
            }
        }
        0
    }

    pub fn set_option_voice_record_device(&mut self, option: i32) {
        if self.sound_disabled || self.voice.voip_disabled {
            return;
        }
        if option < 1 {
            return;
        }
        let name = self
            .voice
            .record_driver_names
            .get(option as usize - 1)
            .cloned()
            .unwrap_or_default();
        self.opt_mut("voiceRecordDeviceName")
            .set_string(Some(&name));
    }

    pub fn get_mic_volume_indicator(&self) -> i32 {
        self.voice.mic_volume_indicator
    }

    pub fn get_mic_volume_error(&self) -> bool {
        self.voice.mic_volume_error
    }

    pub fn get_server_voip_enable(&self) -> bool {
        self.voice.server_voip_enable
    }

    pub fn set_testing_microphone(&mut self, testing: bool) {
        self.voice.testing_microphone = testing;
    }

    pub fn get_option_reload_difficulty(&self) -> i32 {
        2
    }

    /// `getOptionFontSizeReal()`: font size 6 ("auto") from screen height.
    pub fn get_option_font_size_real(&self) -> i32 {
        let mut size = self.get_option_font_size();
        if size == 6 {
            let scale = ((self.get_screen_height() - 1080) as f32 / 132.0).floor() as i32 + 1;
            size = match scale.clamp(0, 10) {
                0 => 1,
                1 => 2,
                2 | 3 => 3,
                4..=6 => 4,
                _ => 5,
            };
        }
        size
    }

    pub fn set_option_code_font_size(&mut self, font: Option<&str>) {
        self.opt_mut("codeFontSize").set_string(font);
        self.current_code_font = match font {
            Some("Small") => "CodeSmall",
            Some("Medium") => "CodeMedium",
            Some("Large") => "CodeLarge",
            _ => "CodeSmall",
        };
    }

    /// TODO(ui): `ObjectTooltip.checkFont()`.
    pub fn set_option_tooltip_font(&mut self, font: Option<&str>) {
        self.opt_mut("tooltipFont").set_string(font);
    }

    pub fn get_option_mods_enabled(&self) -> bool {
        self.option_mods_enabled
    }

    pub fn set_option_mods_enabled(&mut self, enabled: bool) {
        self.option_mods_enabled = enabled;
    }

    /// TODO(render): `Display.capabilities.OpenGL33 || !macOS`.
    pub fn allow_option_texture_compression(&self) -> bool {
        self.gl33_supported || !cfg!(target_os = "macos")
    }

    pub fn get_option_texture_compression(&self) -> bool {
        self.allow_option_texture_compression() && self.opt("textureCompression").get_bool()
    }

    pub fn get_option_texture2x(&self) -> bool {
        true
    }

    pub fn set_option_texture2x(&mut self, b: bool) {
        self.opt_mut("texture2x").set_bool(b);
        debug_log("1x textures are disabled.");
    }

    pub fn get_max_texture_size_from_flags(&self, flags: i32) -> i32 {
        if flags & 0x80 != 0 {
            return self.get_max_texture_size();
        }
        if flags & 0x100 != 0 {
            return self.get_max_vehicle_texture_size();
        }
        32768
    }

    /// `getMaxTextureSizeFromOption`; `None` where Java throws.
    pub fn get_max_texture_size_from_option(&self, option: i32) -> Option<i32> {
        match option {
            1 => Some(256),
            2 => Some(512),
            3 => Some(1024),
            4 => Some(2048),
            _ => None,
        }
    }

    pub fn get_max_texture_size(&self) -> i32 {
        self.get_max_texture_size_from_option(self.get_option_max_texture_size())
            .unwrap_or(256)
    }

    pub fn get_max_vehicle_texture_size(&self) -> i32 {
        self.get_max_texture_size_from_option(self.get_option_max_vehicle_texture_size())
            .unwrap_or(256)
    }

    pub fn get_option_zoom_levels1x(&self) -> String {
        self.opt("zoomLevels1x").get_value_as_string()
    }

    pub fn set_option_zoom_levels1x(&mut self, levels: Option<&str>) {
        self.opt_mut("zoomLevels1x").parse(levels.unwrap_or(""));
    }

    pub fn get_option_zoom_levels2x(&self) -> String {
        self.opt("zoomLevels2x").get_value_as_string()
    }

    pub fn set_option_zoom_levels2x(&mut self, levels: Option<&str>) {
        self.opt_mut("zoomLevels2x").parse(levels.unwrap_or(""));
    }

    pub fn get_default_zoom_levels(&self) -> Vec<i32> {
        self.offscreen_buffer.get_default_zoom_levels()
    }

    pub fn set_option_screen_filter(&mut self, value: Option<&str>) {
        self.opt_mut("screenFilter").parse(value.unwrap_or(""));
        self.screen_filter = if self
            .get_option_screen_filter()
            .eq_ignore_ascii_case("linear")
        {
            9729
        } else {
            9728
        };
    }

    pub fn get_screen_filter(&self) -> i32 {
        self.screen_filter
    }

    /// `setOptionActiveController(controllerIndex, active)`.
    pub fn set_option_active_controller(&mut self, controller_index: i32, active: bool) {
        if controller_index < 0
            || controller_index as usize >= self.connected_controller_guids.len()
        {
            return;
        }
        let guid = self.connected_controller_guids[controller_index as usize].clone();
        self.set_controller_active(&guid, active);
    }

    /// `JoypadManager.setControllerActive(guid, active)`.
    pub fn set_controller_active(&mut self, guid: &str, active: bool) {
        if active {
            if !self.active_controller_guids.iter().any(|g| g == guid) {
                self.active_controller_guids.push(guid.to_string());
            }
        } else {
            self.active_controller_guids.retain(|g| g != guid);
        }
    }

    pub fn get_option_active_controller(&self, guid: &str) -> bool {
        self.active_controller_guids.iter().any(|g| g == guid)
    }

    pub fn set_option_uifbo(&mut self, use_: bool) {
        self.opt_mut("uiRenderOffscreen").set_bool(use_);
        if self.in_game_state {
            self.use_ui_fbo = self.supports_fbo() && self.get_option_uifbo();
        }
    }

    pub fn on_option_controller_button_style_changed(&mut self) {
        let s = self.get_option_controller_button_style_string();
        self.pending_events.push(CoreEvent {
            name: "OptionControllerButtonStyleChanged",
            args: vec![EventArg::Str(s)],
        });
    }

    /// `ControllerButtonStyle.fromOptionCode(...).getLabel()`.
    pub fn get_option_controller_button_style_string(&self) -> String {
        match self.get_option_controller_button_style() {
            1 => "XBOX",
            3 => "STEAMDECK",
            _ => "PS4",
        }
        .to_string()
    }

    pub fn on_option_gamepad_binding_preset_changed(&mut self) {
        let s = self.get_option_gamepad_binding_preset();
        self.pending_events.push(CoreEvent {
            name: "OptionGamepadBindingPresetChanged",
            args: vec![EventArg::Str(s)],
        });
    }

    pub fn get_option_single_context_menu(&self, player: i32) -> Option<bool> {
        let a = self.opt("singleContextMenu").as_array();
        usize::try_from(player)
            .ok()
            .filter(|&i| i < a.size())
            .map(|i| a.get_element(i).get_bool())
    }

    pub fn set_option_single_context_menu(&mut self, player: i32, b: bool) -> Option<()> {
        let i = usize::try_from(player).ok()?;
        let a = self.opt_mut("singleContextMenu").as_array_mut();
        if i >= a.size() {
            return None;
        }
        a.get_element_mut(i).set_bool(b);
        Some(())
    }

    pub fn is_option_simple_clothing_textures(&self, zombie: bool) -> bool {
        match self.get_option_simple_clothing_textures() {
            1 => false,
            2 => zombie,
            _ => true,
        }
    }

    pub fn get_ignore_prone_zombie_range(&self) -> f32 {
        match self.get_option_ignore_prone_zombie_range() {
            2 => 1.5,
            3 => 2.0,
            4 => 2.5,
            5 => 3.0,
            _ => -1.0,
        }
    }

    // ---- Lua reset ----------------------------------------------------------

    /// `ResetLua(activeMods, reason)`: queued for the engine, which tears
    /// down and re-creates the Lua state ([`CoreRequest::ResetLua`]).
    pub fn reset_lua(&mut self, active_mods: &str, reason: &str) {
        self.pending_requests.push(CoreRequest::ResetLua {
            active_mods: active_mods.into(),
            reason: reason.into(),
        });
    }

    pub fn delay_reset_lua(&mut self, active_mods: &str, reason: &str) {
        self.delay_reset_lua = Some((active_mods.to_string(), reason.to_string()));
    }

    pub fn check_delay_reset_lua(&mut self) {
        if let Some((mods, reason)) = self.delay_reset_lua.take() {
            self.reset_lua(&mods, &reason);
        }
    }

    // ---- plain flags --------------------------------------------------------

    pub fn is_show_ping(&self) -> bool {
        self.show_ping
    }
    pub fn set_show_ping(&mut self, b: bool) {
        self.show_ping = b;
    }
    pub fn is_force_snow(&self) -> bool {
        self.force_snow
    }
    pub fn set_force_snow(&mut self, b: bool) {
        self.force_snow = b;
    }
    pub fn is_zombie_group_sound(&self) -> bool {
        self.zombie_group_sound
    }
    pub fn set_zombie_group_sound(&mut self, b: bool) {
        self.zombie_group_sound = b;
    }
    pub fn get_blinking_moodle(&self) -> Option<String> {
        self.blinking_moodle.clone()
    }
    pub fn set_blinking_moodle(&mut self, s: Option<&str>) {
        self.blinking_moodle = s.map(str::to_string);
    }

    pub fn init_poisonous_berry(&mut self) {
        let list = [
            "Base.BerryGeneric1",
            "Base.BerryGeneric2",
            "Base.BerryGeneric3",
            "Base.BerryGeneric4",
            "Base.BerryGeneric5",
            "Base.BerryPoisonIvy",
        ];
        let i = self.rand_next_range(0, list.len() as i32 - 1);
        self.poisonous_berry = Some(list[i as usize].to_string());
    }

    pub fn init_poisonous_mushroom(&mut self) {
        let list = [
            "Base.MushroomGeneric1",
            "Base.MushroomGeneric2",
            "Base.MushroomGeneric3",
            "Base.MushroomGeneric4",
            "Base.MushroomGeneric5",
            "Base.MushroomGeneric6",
            "Base.MushroomGeneric7",
        ];
        let i = self.rand_next_range(0, list.len() as i32 - 1);
        self.poisonous_mushroom = Some(list[i as usize].to_string());
    }

    pub fn get_poisonous_berry(&self) -> Option<String> {
        self.poisonous_berry.clone()
    }
    pub fn set_poisonous_berry(&mut self, s: Option<&str>) {
        self.poisonous_berry = s.map(str::to_string);
    }
    pub fn get_poisonous_mushroom(&self) -> Option<String> {
        self.poisonous_mushroom.clone()
    }
    pub fn set_poisonous_mushroom(&mut self, s: Option<&str>) {
        self.poisonous_mushroom = s.map(str::to_string);
    }

    /// Static `getTileScale()`.
    pub fn get_tile_scale(&self) -> i32 {
        self.tile_scale
    }
    pub fn is_selecting_all(&self) -> bool {
        self.is_selecting_all
    }
    pub fn set_is_selecting_all(&mut self, b: bool) {
        self.is_selecting_all = b;
    }

    /// `getMpTextColor()`: a random light colour is picked (and stored in
    /// the option) the first time it is asked for in a session.
    pub fn get_mp_text_color(&mut self) -> ColorInfo {
        if self.mp_text_color.is_none() {
            let r = (self.rand_next(135) + 120) as f32 / 255.0;
            let g = (self.rand_next(135) + 120) as f32 / 255.0;
            let b = (self.rand_next(135) + 120) as f32 / 255.0;
            let c = ColorInfo::new(r, g, b, 1.0);
            self.mp_text_color = Some(c);
            self.set_color_option("mpTextColor", &c);
        }
        let c = ColorInfo::from_array(self.opt("mpTextColor").as_array().get_value_as_color());
        self.mp_text_color = Some(c);
        c
    }

    pub fn set_mp_text_color(&mut self, c: ColorInfo) {
        self.mp_text_color = Some(c);
        self.set_color_option("mpTextColor", &c);
    }

    /// `setValueVarArgs(c.r, c.g, c.b)` (floats widened to double).
    fn set_color_option(&mut self, name: &str, c: &ColorInfo) {
        let args = [
            ConfigObject::Double(c.r as f64),
            ConfigObject::Double(c.g as f64),
            ConfigObject::Double(c.b as f64),
        ];
        self.opt_mut(name).as_array_mut().set_value_var_args(&args);
    }

    fn color_option(&self, name: &str) -> ColorInfo {
        ColorInfo::from_array(self.opt(name).as_array().get_value_as_color())
    }

    pub fn is_azerty(&self) -> bool {
        self.is_azerty
    }
    pub fn set_azerty(&mut self, b: bool) {
        self.is_azerty = b;
    }

    pub fn get_object_highlited_color(&mut self) -> ColorInfo {
        self.object_highlited_color = self.color_option("objHighlightColor");
        self.object_highlited_color
    }
    pub fn set_object_highlited_color(&mut self, c: ColorInfo) {
        self.set_color_option("objHighlightColor", &c);
        self.object_highlited_color = c;
    }
    pub fn get_world_item_highlight_color(&mut self) -> ColorInfo {
        self.world_item_highlight_color = self.color_option("worldItemHighlightColor");
        self.world_item_highlight_color
    }
    pub fn set_world_item_highlight_color(&mut self, c: ColorInfo) {
        self.set_color_option("worldItemHighlightColor", &c);
        self.world_item_highlight_color = c;
    }
    /// Not exposed by Java (no getter); kept for the renderer.
    pub fn workstation_highlight_color(&mut self) -> ColorInfo {
        self.workstation_highlited_color = self.color_option("workstationHighlightColor");
        self.workstation_highlited_color
    }
    pub fn get_good_highlited_color(&mut self) -> ColorInfo {
        self.good_highlited_color = self.color_option("goodHighlightColor");
        self.good_highlited_color
    }
    pub fn set_good_highlited_color(&mut self, c: ColorInfo) {
        self.set_color_option("goodHighlightColor", &c);
        self.good_highlited_color = c;
    }
    pub fn get_bad_highlited_color(&mut self) -> ColorInfo {
        self.bad_highlited_color = self.color_option("badHighlightColor");
        self.bad_highlited_color
    }
    pub fn set_bad_highlited_color(&mut self, c: ColorInfo) {
        self.set_color_option("badHighlightColor", &c);
        self.bad_highlited_color = c;
    }
    pub fn get_target_color(&mut self) -> ColorInfo {
        self.target_color = self.color_option("targetColor");
        self.target_color
    }
    pub fn set_target_color(&mut self, c: ColorInfo) {
        self.set_color_option("targetColor", &c);
        self.target_color = c;
    }
    pub fn get_no_target_color(&mut self) -> ColorInfo {
        self.no_target_color = self.color_option("noTargetColor");
        self.no_target_color
    }
    pub fn set_no_target_color(&mut self, c: ColorInfo) {
        self.set_color_option("noTargetColor", &c);
        self.no_target_color = c;
    }

    /// `getXAngle(width, angle)`.
    pub fn get_x_angle(&self, width: i32, angle: f32) -> i32 {
        let radian = ((225.0f32 + angle) as f64).to_radians();
        java_round((2f64.sqrt() * radian.cos() + 1.0) * (width / 2) as f64) as i32
    }

    /// `getYAngle(width, angle)`.
    pub fn get_y_angle(&self, width: i32, angle: f32) -> i32 {
        let radian = ((225.0f32 + angle) as f64).to_radians();
        java_round((2f64.sqrt() * radian.sin() + 1.0) * (width / 2) as f64) as i32
    }

    pub fn is_in_debug(&self) -> bool {
        self.debug
    }
    pub fn is_no_save(&self) -> bool {
        self.no_save
    }
    pub fn set_no_save(&mut self, b: bool) {
        self.no_save = b;
    }
    pub fn is_show_first_time_vehicle_tutorial(&self) -> bool {
        self.show_first_time_vehicle_tutorial
    }
    pub fn set_show_first_time_vehicle_tutorial(&mut self, b: bool) {
        self.show_first_time_vehicle_tutorial = b;
    }
    pub fn is_show_first_time_weather_tutorial(&self) -> bool {
        self.show_first_time_weather_tutorial
    }
    pub fn set_show_first_time_weather_tutorial(&mut self, b: bool) {
        self.show_first_time_weather_tutorial = b;
    }
    pub fn set_anim_popup_done(&mut self, b: bool) {
        self.anim_popup_done = b;
    }
    pub fn is_anim_popup_done(&self) -> bool {
        self.anim_popup_done
    }
    pub fn set_mods_popup_done(&mut self, b: bool) {
        self.mods_popup_done = b;
    }
    pub fn is_mods_popup_done(&self) -> bool {
        self.mods_popup_done
    }
    pub fn is_collide_zombies(&self) -> bool {
        self.collide_zombies
    }
    pub fn set_collide_zombies(&mut self, b: bool) {
        self.collide_zombies = b;
    }
    pub fn is_flash_iso_cursor(&self) -> bool {
        self.flash_iso_cursor
    }
    pub fn set_flash_iso_cursor(&mut self, b: bool) {
        self.flash_iso_cursor = b;
    }
    pub fn is_option_progress_bar(&self) -> bool {
        true
    }

    /// `getOptionOnStartup(name)` (opaque values stored by Lua).
    pub fn get_option_on_startup(&self, name: &str) -> Option<Rc<dyn Any>> {
        self.options_on_startup.get(name).cloned()
    }

    pub fn set_option_on_startup(&mut self, name: &str, value: Option<Rc<dyn Any>>) {
        match value {
            Some(v) => {
                self.options_on_startup.insert(name.to_string(), v);
            }
            // HashMap.put(name, null): get() returns null afterwards.
            None => {
                self.options_on_startup.remove(name);
            }
        }
    }

    /// TODO(scripts): needs `ScriptManager.getAllItems()`.
    pub fn count_missing3d_items(&self) -> String {
        "total missing: 0/0".to_string()
    }

    pub fn set_animal_cheat(&mut self, b: bool) {
        self.animal_cheat = b;
    }
    pub fn set_display_player_model(&mut self, b: bool) {
        self.display_player_model = b;
    }
    pub fn is_display_player_model(&self) -> bool {
        self.display_player_model
    }
    pub fn set_display_cursor(&mut self, b: bool) {
        self.display_cursor = b;
    }
    pub fn is_display_cursor(&self) -> bool {
        self.display_cursor
    }

    pub fn get_iso_cursor_alpha(&self) -> f32 {
        match self.get_iso_cursor_visibility() {
            0 => 0.0,
            1 => 0.05,
            2 => 0.1,
            3 => 0.15,
            4 => 0.3,
            5 => 0.5,
            6 => 0.75,
            _ => 0.05,
        }
    }

    pub fn debug_output_missing_item_spawn(&self) -> std::io::Result<String> {
        self.debug_output_missing_spawn("../workdir/media/scripts/items/", "item")
    }

    pub fn debug_output_missing_clothing_spawn(&self) -> std::io::Result<String> {
        self.debug_output_missing_spawn("../workdir/media/scripts/clothing/", "clothing")
    }

    /// `debugOutputMissingSpawn(directory, category)`: dev tool listing
    /// script items that no distribution spawns (paths relative to the CWD,
    /// as in Java).
    pub fn debug_output_missing_spawn(
        &self,
        directory: &str,
        category: &str,
    ) -> std::io::Result<String> {
        let mut sb = format!("{category} items that don't spawn:\r\n");
        let script_folder = Path::new(directory);
        if !script_folder.is_dir() {
            sb.push_str(&format!("Couldn't find {category} dir."));
            return Ok(sb);
        }
        if std::fs::read_dir(script_folder)?.next().is_none() {
            sb.push_str(&format!("No {category} script found."));
            return Ok(sb);
        }
        let items = clothing_strings(script_folder)?;
        sb.push_str(&format!("Found {} {category}s \r\n", items.len()));
        let spawn_folder = Path::new("../workdir/media/lua/server/Items/");
        if !spawn_folder.is_dir() {
            sb.push_str("Couldn't find Items dir.");
            return Ok(sb);
        }
        if std::fs::read_dir(spawn_folder)?.next().is_none() {
            sb.push_str("No spawn script found.");
            return Ok(sb);
        }
        let spawned = clothing_spawn_strings(spawn_folder)?;
        let missing: Vec<&String> = items.iter().filter(|i| !spawned.contains(i)).collect();
        let vehicle_folder = Path::new("../workdir/media/lua/server/Vehicles");
        if !vehicle_folder.is_dir() {
            sb.push_str("Couldn't find vehicle distribution dir.");
            return Ok(sb);
        }
        if std::fs::read_dir(vehicle_folder)?.next().is_none() {
            sb.push_str("No vehicle distribution script found.");
            return Ok(sb);
        }
        let vehicle = clothing_spawn_strings(vehicle_folder)?;
        for name in missing {
            if vehicle.contains(name) {
                sb.push_str(&format!("{name} only spawn in vehicle \r\n"));
            } else {
                sb.push_str(&format!("{name} dont spawn \r\n"));
            }
        }
        println!("{sb}");
        std::fs::write(format!("{category}ItemsNotSpawning.txt"), &sb)?;
        Ok(sb)
    }

    pub fn get_selected_map(&self) -> Option<String> {
        self.selected_map.clone()
    }
    pub fn set_selected_map(&mut self, s: Option<&str>) {
        self.selected_map = s.map(str::to_string);
    }
    pub fn get_console_dot_txt_size_kb(&self) -> i32 {
        self.console_dot_txt_size_kb
    }
    pub fn set_console_dot_txt_size_kb(&mut self, kilobytes: i32) {
        self.console_dot_txt_size_kb = kilobytes.clamp(512, 102400);
    }
    /// `setConsoleDotTxtSizeKB(String)`.
    pub fn set_console_dot_txt_size_kb_str(&mut self, s: Option<&str>) {
        let kb = s.and_then(|s| java_parse_int(s, 10)).unwrap_or(-1);
        if kb > 0 {
            self.set_console_dot_txt_size_kb(kb);
        }
    }

    pub fn get_option_use_physics_hit_reaction(&self) -> bool {
        if self.is_client || self.is_server {
            return false;
        }
        self.opt("usePhysicsHitReaction").get_bool()
    }

    pub fn get_account_used(&self) -> Option<Rc<dyn Any>> {
        self.account_used.clone()
    }
    pub fn set_account_used(&mut self, a: Option<Rc<dyn Any>>) {
        self.account_used = a;
    }

    /// Static `isDevMode()`.
    pub fn is_dev_mode(&self) -> bool {
        self.is_dev
    }

    // ---- options.ini --------------------------------------------------------

    /// `loadOptions()`. Returns `Ok(false)` when there was no file (defaults
    /// are initialised and written, as `initOptionsINI` does).
    pub fn load_options(&mut self) -> std::io::Result<bool> {
        self.loaded_options = false;
        let path = self.options_path();
        let exists = path.as_ref().is_some_and(|p| p.exists());
        if !exists {
            self.init_options_ini()?;
            return Ok(false);
        }
        let path = path.unwrap();
        self.loaded_options = true;
        for n in 0..4 {
            self.set_auto_zoom(n, false);
        }
        self.opt_mut("language").set_string(Some(""));
        for o in self.options.iter_mut() {
            if o.is_multi_line() {
                o.as_array_mut().clear();
            }
        }
        let mut config_file = ConfigFile::new();
        config_file.set_version_string("version");
        if config_file.read(&path) {
            let version = config_file.get_version();
            self.version = version;
            for o in config_file.get_options() {
                let name = self.upgrade_option_name(o.name(), version);
                let value = self.upgrade_option_value(&name, &o.get_value_as_string(), version);
                if let Some(core_option) = self.option_mut(&name) {
                    core_option.parse(&value);
                }
            }
            self.width = self.opt("width").get_int();
            self.height = self.opt("height").get_int();
            self.performance.fog_quality = self.opt("fogQuality").get_int();
            self.performance.view_cone_opacity = self.opt("viewConeOpacity").get_int();
            self.performance.lock_fps = self.opt("frameRate").get_int();
            self.performance.uncapped_fps = self.opt("uncappedFPS").get_bool();
            self.performance.water_quality = self.opt("water").get_int();
            self.performance.puddles_quality = self.opt("puddles").get_int();
            let lighting = self.opt("lightFPS").get_int();
            self.performance.set_lighting_fps(lighting);
            if self.opt("uncappedFPS").get_bool() {
                self.opt_mut("uncappedFPS").set_bool(false);
                self.opt_mut("frameRate").set_int(60);
                self.performance.lock_fps = self.opt("frameRate").get_int();
                self.performance.uncapped_fps = self.opt("uncappedFPS").get_bool();
            }
            self.i_perf_skybox = self.opt("perfSkybox").get_int();
            self.i_perf_puddles = self.opt("perfPuddles").get_int();
            self.perf_reflections = self.opt("bPerfReflections").get_bool();
            for p in 0..4 {
                self.offscreen_buffer.auto_zoom[p] = self.get_auto_zoom(p as i32).unwrap_or(false);
            }
            let guids: Vec<String> = {
                let a = self.opt("controller").as_array();
                (0..a.size())
                    .map(|i| java_trim(&a.get_element(i).get_value_as_string()).to_string())
                    .collect()
            };
            for guid in guids {
                if guid.is_empty() {
                    continue;
                }
                self.set_controller_active(&guid, true);
            }
            self.moving_object_update_scheduler_enabled =
                self.opt("tieredZombieUpdates").get_bool();
            let code_font = self.get_option_code_font_size();
            self.set_option_code_font_size(Some(&code_font));
        }
        if self.opt("language").get_string().is_empty() {
            let lang = self.system_language.clone();
            self.opt_mut("language").set_string(Some(&lang));
        }
        if !self.opt("doneNewSaveFolder").get_bool() {
            self.handle_new_save_folder_format();
            self.opt_mut("doneNewSaveFolder").set_bool(true);
        }
        self.collect_option_changes();
        self.save_options()?;
        Ok(true)
    }

    fn upgrade_option_name(&self, name: &str, _version: i32) -> String {
        name.to_string()
    }

    /// Version migration: before version 8 `textureCompression` and
    /// `doWindSpriteEffects` are reset to false.
    fn upgrade_option_value(&self, name: &str, value: &str, version: i32) -> String {
        if version < 8 && (name == "textureCompression" || name == "doWindSpriteEffects") {
            return "false".into();
        }
        value.to_string()
    }

    /// `initOptionsINI()`: first run, no `options.ini`.
    fn init_options_ini(&mut self) -> std::io::Result<()> {
        self.save_folder = self.get_my_document_folder();
        if let Some(dir) = &self.cache_dir {
            let _ = std::fs::create_dir(dir);
            if let (Some(src), Some(dst)) = (self.game_dir.clone(), self.cache_dir.clone()) {
                copy_paste_folders(&src.join("mods"), &dst.join("mods"));
            }
        }
        let lang = self.system_language.clone();
        self.set_option_language_name(Some(&lang));
        if self.get_option_language_name() == "FR" {
            self.set_azerty(true);
        }
        // TODO(render): Display modes / monitor work area come from the
        // renderer; with none supplied the default 1280x720 is kept.
        if !self.is_server && !self.available_display_modes.is_empty() {
            let (mw, mh) = self.monitor_work_area.unwrap_or((i32::MAX, i32::MAX));
            let mut w = 0;
            let mut h = 0;
            for &(mode_w, mode_h) in &self.available_display_modes {
                if mode_w <= w || mode_w >= mw || mode_h >= mh {
                    continue;
                }
                w = mode_w;
                h = mode_h;
            }
            self.width = w;
            self.height = h;
        }
        self.set_option_zoom_levels2x(Some("50;75;125;150;175;200"));
        self.set_option_zoom_levels1x(Some("50;75;125;150;175;200"));
        self.save_options()
    }

    /// `handleNewSaveFolderFormat()`: move pre-41 saves into `Saves/`.
    fn handle_new_save_folder_format(&mut self) {
        let Some(cache) = self.cache_dir.clone() else {
            return;
        };
        let save_dir = cache.join("Saves");
        let _ = std::fs::create_dir(&save_dir);
        for mode in [
            "Beginner",
            "Survival",
            "A Really CD DA",
            "LastStand",
            "Opening Hours",
            "Sandbox",
            "Tutorial",
            "Winter is Coming",
            "You Have One Day",
        ] {
            let previous = cache.join(mode);
            let new = save_dir.join(mode);
            if !previous.exists() {
                continue;
            }
            let _ = std::fs::create_dir(&new);
            let _ = std::fs::remove_dir(&new);
            if std::fs::rename(&previous, &new).is_err() {
                break;
            }
        }
    }

    /// `saveOptions()`. Writes nothing (and succeeds) when no options path
    /// is configured.
    pub fn save_options(&mut self) -> std::io::Result<()> {
        // TODO(server): ZipBackup.waitFinish() on dedicated servers.
        let Some(path) = self.options_path() else {
            return Ok(());
        };
        self.add_fake_options_for_writing();
        let mut config_file = ConfigFile::new();
        config_file.set_version_string("version");
        config_file.set_write_tooltips(false);
        let mut options: Vec<&ConfigOption> = self.options.iter().collect();
        if !cfg!(target_os = "macos") {
            options.retain(|o| {
                o.name() != "macosIgnoreMouseWheelAcceleration"
                    && o.name() != "macosMapHorizontalMouseWheelToVertical"
            });
        }
        options.extend(self.fake_options.iter());
        options.sort_by(|a, b| case_insensitive_cmp(a.name(), b.name()));
        if config_file.write(&path, self.fileversion, &options) {
            Ok(())
        } else {
            // Java logs the exception inside ConfigFile.write.
            debug_log(&format!("failed to write {}", path.display()));
            Ok(())
        }
    }

    fn add_fake_options_for_writing(&mut self) {
        let p = self.performance.clone();
        self.opt_mut("fogQuality").set_int(p.fog_quality);
        self.opt_mut("viewConeOpacity").set_int(p.view_cone_opacity);
        self.opt_mut("lightFPS").set_int(p.lighting_fps);
        self.opt_mut("frameRate").set_int(p.lock_fps);
        self.opt_mut("puddles").set_int(p.puddles_quality);
        let (w, h) = (self.get_screen_width(), self.get_screen_height());
        self.opt_mut("height").set_int(h);
        self.opt_mut("width").set_int(w);
        let tiered = self.get_option_tiered_zombie_updates();
        self.opt_mut("tieredZombieUpdates").set_bool(tiered);
        self.opt_mut("uncappedFPS").set_bool(p.uncapped_fps);
        self.opt_mut("water").set_int(p.water_quality);
        let guids = self.active_controller_guids.clone();
        let c = self.opt_mut("controller");
        c.as_array_mut().clear();
        for g in guids {
            c.parse(&g);
        }
    }
}

/// Java `Math.round(double)`.
fn java_round(d: f64) -> i64 {
    (d + 0.5).floor() as i64
}

fn detect_system_language() -> String {
    for var in ["LC_ALL", "LC_MESSAGES", "LANG"] {
        if let Ok(v) = std::env::var(var) {
            let lang: String = v.chars().take_while(|c| c.is_ascii_alphabetic()).collect();
            if !lang.is_empty() && lang != "C" && lang != "POSIX" {
                return lang.to_uppercase();
            }
        }
    }
    "EN".into()
}

fn seed_rng() -> u64 {
    let t = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0x9E37_79B9_7F4A_7C15);
    t | 1
}

/// `copyPasteFolders` / `searchFolders` / `copyPasteFile`.
fn copy_paste_folders(src: &Path, dst: &Path) {
    if !src.exists() {
        return;
    }
    if src.is_dir() {
        let _ = std::fs::create_dir(dst);
        if let Ok(rd) = std::fs::read_dir(src) {
            for e in rd.flatten() {
                copy_paste_folders(&e.path(), &dst.join(e.file_name()));
            }
        }
    } else if let Err(e) = std::fs::copy(src, dst) {
        debug_log(&format!("{e}"));
    }
}

fn sorted_files(dir: &Path) -> std::io::Result<Vec<PathBuf>> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(dir)?
        .flatten()
        .map(|e| e.path())
        .collect();
    v.sort();
    Ok(v)
}

/// `getClothingSpawnString(scriptFolder)`.
fn clothing_spawn_strings(folder: &Path) -> std::io::Result<Vec<String>> {
    let mut out = Vec::new();
    let mut meh = 500;
    for file in sorted_files(folder)? {
        let mut is_distribution = false;
        let Ok(bytes) = std::fs::read(&file) else {
            continue;
        };
        let text = String::from_utf8_lossy(&bytes);
        let file_name = file
            .file_name()
            .map(|f| f.to_string_lossy().into_owned())
            .unwrap_or_default();
        for line in java_lines(&text) {
            if !is_distribution {
                meh -= 1;
                if meh == 0 {
                    break;
                }
            }
            let t = java_trim(line);
            if !t.starts_with("items") && !is_distribution && !file_name.contains("Junk") {
                continue;
            }
            is_distribution = true;
            meh = 500;
            if !t.starts_with('"') || !t.contains(',') {
                continue;
            }
            if let Some(s) = line.split('"').nth(1) {
                out.push(s.to_string());
            }
        }
    }
    Ok(out)
}

/// `getClothingStrings(scriptFolder)`.
fn clothing_strings(folder: &Path) -> std::io::Result<Vec<String>> {
    let mut out = Vec::new();
    for file in sorted_files(folder)? {
        let Ok(bytes) = std::fs::read(&file) else {
            continue;
        };
        let text = String::from_utf8_lossy(&bytes);
        for line in java_lines(&text) {
            let t = java_trim(line);
            if !t.starts_with("item") {
                continue;
            }
            let Some(n) = t.split("item ").nth(1) else {
                continue;
            };
            let starts = [
                "Wound",
                "Bandage",
                "ZedDmg",
                "MakeUp",
                "Ring_Right",
                "Ring_Left_MiddleFinger_",
                "Animal_",
                "WristWatch_Right_",
                "Bracelet_Right",
                "Berry",
                "MushroomGeneric",
                "Umbrella",
            ];
            let ends = [
                "DOWN",
                "Reverse",
                "Back",
                "Right",
                "_R",
                "_nofilter",
                "_Stubble",
                "Open",
                "Wet",
                "Lit",
                "Set",
                "BulletsMold",
                "ShellsMold",
            ];
            let contains = ["_R_", "Debug", "DEBUG", "Dev", "GloveBox", "Trunk"];
            if starts.iter().any(|s| n.starts_with(s))
                || ends.iter().any(|s| n.ends_with(s))
                || contains.iter().any(|s| n.contains(s))
                || ((n.contains("Tent") || n.contains("SleepingBag")) && !n.contains("Packed"))
            {
                continue;
            }
            out.push(n.to_string());
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests;
