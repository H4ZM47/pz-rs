use super::*;
use std::path::PathBuf;

/// A fresh, empty directory under the system temp dir.
fn temp_dir(tag: &str) -> PathBuf {
    use std::sync::atomic::{AtomicU32, Ordering};
    static N: AtomicU32 = AtomicU32::new(0);
    let n = N.fetch_add(1, Ordering::Relaxed);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let d = std::env::temp_dir().join(format!(
        "pz-engine-core-{tag}-{}-{nanos}-{n}",
        std::process::id()
    ));
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// Hand-written sample in the format the game writes (sorted
/// case-insensitively, `version=` first, no trailing newline).
const SAMPLE: &str = "version=8
3DGroundItem=false
ambientVolume=3
autozoom=true,false,false,false
badHighlightColor=1.0,0.0,0.0
bPerfReflections=false
chatFadeTime=2.5
codeFontSize=Large
controller=030000004c050000e60c000000010000
controllerButtonStyle=2
fontSize=3
frameRate=144
fullScreen=true
gamepadBindingPreset=Original
height=1080
language=EN
lightFPS=30
minChatOpaque=0.699999988079071
mpTextColor=0.6980392336845398,0.6549019813537598,0.6627451181411743
objHighlightColor=0.25,0.5,0.75
perfSkybox=2
screenFilter=linear
singleContextMenu=false,true,false,false
textureCompression=true
uncappedFPS=false
unknownOption=whatever
voiceRecordDeviceName=Some Mic
width=1920
zoomLevels1x=50;75;125;150;175;200
zoomLevels2x=250;225;200;175;150;125;75;50;25";

fn core_in(dir: &Path) -> Core {
    let mut c = Core::new();
    c.set_cache_dir(dir);
    c.system_language = "EN".into();
    c
}

#[test]
fn defaults() {
    let mut c = Core::new();
    assert_eq!(c.get_screen_width(), 1280);
    assert_eq!(c.get_screen_height(), 720);
    assert_eq!(c.get_game_mode(), "Sandbox");
    assert!(!c.get_debug());
    assert_eq!(c.get_tile_scale(), 2);
    assert_eq!(c.get_version(), "42.21.0 4a0e9546ec");
    assert_eq!(c.get_version_number(), "42.21");
    assert_eq!(c.get_git_sha(), "4a0e9546ec");
    assert_eq!(c.get_option_count(), 136);
    assert_eq!(c.fake_options().len(), 22);
    assert_eq!(c.get_option_font_size(), 6);
    assert_eq!(c.get_option_moodle_size(), 7);
    assert_eq!(c.get_option_ambient_volume(), 5);
    assert_eq!(c.get_option_context_menu_font(), "Medium");
    assert_eq!(c.get_option_tooltip_font(), "Small");
    assert_eq!(c.get_option_chat_font_size(), "medium");
    assert_eq!(c.get_option_gamepad_binding_preset(), "Original");
    assert_eq!(c.get_option_controller_button_style_string(), "XBOX");
    assert_eq!(c.get_option_min_chat_opaque(), 1.0);
    assert_eq!(c.get_option_chat_fade_time(), 0.0);
    assert_eq!(c.get_option_precipitation_speed_multiplier(), 1.0);
    assert!(c.get_option_zoom());
    assert!(c.is_zoom_enabled());
    assert!(c.is_toggle_to_sprint());
    assert!(!c.is_toggle_to_run());
    assert!(c.is_option3_d_ground_item());
    assert_eq!(c.get_iso_cursor_visibility(), 5);
    assert_eq!(c.get_iso_cursor_alpha(), 0.5);
    assert_eq!(c.get_terms_of_service_version(), -1);
    assert_eq!(c.get_option_zoom_levels1x(), "");
    assert_eq!(
        c.get_default_zoom_levels(),
        vec![250, 225, 200, 175, 150, 125, 100, 75, 50, 25]
    );
    assert_eq!(
        c.get_object_highlited_color(),
        ColorInfo::new(0.98, 0.56, 0.11, 1.0)
    );
    assert_eq!(
        c.get_world_item_highlight_color(),
        ColorInfo::new(0.5, 1.0, 1.0, 1.0)
    );
    assert_eq!(c.get_option_single_context_menu(3), Some(false));
    assert_eq!(c.get_option_single_context_menu(4), None);
    assert_eq!(c.get_auto_zoom(0), Some(false));
    assert_eq!(c.get_max_texture_size(), 256);
    assert_eq!(c.get_max_vehicle_texture_size(), 512);
    assert_eq!(c.get_option_reload_difficulty(), 2);
    assert_eq!(c.get_option_font_size_real(), 1);
    assert_eq!(c.get_key("Forward"), 0);
    assert!(!c.is_key("Forward", 0));
    assert!(c.is_default_options());
    assert_eq!(
        c.option("worldItemHighlightColor")
            .unwrap()
            .get_value_as_string(),
        "0.5,1.0,1.0"
    );
    assert_eq!(
        c.option("autozoom").unwrap().get_value_as_string(),
        "false,false,false,false"
    );
}

#[test]
fn no_options_path_writes_nothing() {
    let mut c = Core::new();
    assert!(!c.load_options().unwrap());
    c.save_options().unwrap();
    assert_eq!(c.get_option_zoom_levels2x(), "50;75;125;150;175;200");
}

#[test]
fn first_run_creates_options_ini() {
    let dir = temp_dir("first");
    let mut c = core_in(&dir);
    assert!(!c.load_options().unwrap());
    let text = std::fs::read_to_string(dir.join("options.ini")).unwrap();
    assert!(text.starts_with("version=8\n3DGroundItem=true\n"), "{text}");
    assert!(text.contains("\nzoomLevels1x=50;75;125;150;175;200\n"));
    assert!(text.contains("\nlanguage=EN\n"));
    assert!(text.contains("\nwidth=1280\n"));
    assert!(text.ends_with("zoomLevels2x=50;75;125;150;175;200"));
    assert_eq!(
        text.contains("macosIgnoreMouseWheelAcceleration"),
        cfg!(target_os = "macos")
    );
    // No multi-line `controller=` entry without active controllers.
    assert!(!text.contains("controller="));
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn round_trip_sample() {
    let dir = temp_dir("roundtrip");
    std::fs::write(dir.join("options.ini"), SAMPLE).unwrap();
    let mut c = core_in(&dir);
    assert!(c.load_options().unwrap());
    assert!(!c.is_default_options());
    assert_eq!(c.version, 8);
    assert_eq!((c.get_screen_width(), c.get_screen_height()), (1920, 1080));
    assert!(!c.is_option3_d_ground_item());
    assert_eq!(c.get_option_ambient_volume(), 3);
    assert_eq!(c.get_auto_zoom(0), Some(true));
    assert!(c.offscreen_buffer.auto_zoom[0]);
    assert!(!c.get_perf_reflections());
    assert!(!c.get_perf_reflections_on_load());
    assert_eq!(c.get_perf_skybox_on_load(), 2);
    assert_eq!(c.get_option_chat_fade_time(), 2.5);
    assert_eq!(c.get_option_min_chat_opaque(), 0.7);
    assert_eq!(c.get_option_code_font_size(), "Large");
    assert_eq!(c.current_code_font, "CodeLarge");
    assert_eq!(
        c.active_controller_guids,
        vec!["030000004c050000e60c000000010000".to_string()]
    );
    assert_eq!(c.get_option_controller_button_style(), 2);
    assert_eq!(c.get_option_controller_button_style_string(), "PS4");
    assert_eq!(c.get_option_font_size(), 3);
    assert_eq!(c.performance.lock_fps, 144);
    assert_eq!(c.performance.lighting_fps, 30);
    assert!(c.is_full_screen());
    assert_eq!(c.get_option_screen_filter(), "linear");
    assert_eq!(c.get_option_single_context_menu(1), Some(true));
    assert!(c.get_option_texture_compression() || !c.allow_option_texture_compression());
    assert_eq!(c.get_option_voice_record_device_name(), "Some Mic");
    assert_eq!(
        c.get_object_highlited_color(),
        ColorInfo::new(0.25, 0.5, 0.75, 1.0)
    );
    assert_eq!(
        c.get_option_zoom_levels2x(),
        "250;225;200;175;150;125;75;50;25"
    );
    // controllerButtonStyle changed on load: its on-change event fired.
    let events = c.take_events();
    assert!(events.contains(&CoreEvent {
        name: "OptionControllerButtonStyleChanged",
        args: vec![EventArg::Str("PS4".into())]
    }));

    // loadOptions saved; the result reloads to the same state and re-saves
    // identically.
    let first = std::fs::read_to_string(dir.join("options.ini")).unwrap();
    assert!(first.starts_with("version=8\n3DGroundItem=false\n"));
    assert!(!first.contains("unknownOption"));
    assert!(first.contains("\nminChatOpaque=0.699999988079071\n"));
    assert!(first.contains("\nchatFadeTime=2.5\n"));
    assert!(
        first.contains("\nmpTextColor=0.6980392336845398,0.6549019813537598,0.6627451181411743\n")
    );
    assert!(first.contains("\ncontroller=030000004c050000e60c000000010000\n"));
    assert!(first.contains("\ndoneNewSaveFolder=true\n"));
    let mut c2 = core_in(&dir);
    assert!(c2.load_options().unwrap());
    let second = std::fs::read_to_string(dir.join("options.ini")).unwrap();
    assert_eq!(first, second);
    for o in c.options().iter().chain(c.fake_options()) {
        assert_eq!(
            o.get_value_as_string(),
            c2.option(o.name()).unwrap().get_value_as_string(),
            "option {}",
            o.name()
        );
    }

    // Setters are saved.
    c2.set_option_ambient_volume(9);
    c2.set_option_min_chat_opaque(0.25);
    c2.set_option_language_name(Some("FR"));
    c2.set_screen_size(1600, 900);
    c2.save_options().unwrap();
    let mut c3 = core_in(&dir);
    c3.load_options().unwrap();
    assert_eq!(c3.get_option_ambient_volume(), 9);
    assert_eq!(c3.get_option_min_chat_opaque(), 0.25);
    assert_eq!(c3.get_option_language_name(), "FR");
    assert_eq!((c3.get_screen_width(), c3.get_screen_height()), (1600, 900));
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn version_migration() {
    let dir = temp_dir("migrate");
    std::fs::write(
        dir.join("options.ini"),
        "version=7\r\ntextureCompression=true\r\ndoWindSpriteEffects=true\r\nzoom=false\r\n",
    )
    .unwrap();
    let mut c = core_in(&dir);
    c.load_options().unwrap();
    assert_eq!(c.version, 7);
    assert!(!c.option("textureCompression").unwrap().get_bool());
    assert!(!c.get_option_do_wind_sprite_effects());
    assert!(!c.get_option_zoom());
    let text = std::fs::read_to_string(dir.join("options.ini")).unwrap();
    assert!(text.starts_with("version=8\n"));
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn uncapped_fps_is_reset_on_load() {
    let dir = temp_dir("uncapped");
    std::fs::write(
        dir.join("options.ini"),
        "version=8\nuncappedFPS=true\nframeRate=244\n",
    )
    .unwrap();
    let mut c = core_in(&dir);
    c.load_options().unwrap();
    assert!(!c.performance.uncapped_fps);
    assert_eq!(c.performance.lock_fps, 60);
    std::fs::remove_dir_all(&dir).unwrap();
}

/// Load and re-save a copy of the real game's `options.ini` (if present on
/// this machine) in a temp dir: the game's own output must round-trip
/// byte for byte. The original file is only read.
#[test]
fn real_options_ini_round_trips() {
    let Some(home) = std::env::var_os("HOME") else {
        return;
    };
    let real = Path::new(&home).join("Zomboid").join("options.ini");
    let Ok(original) = std::fs::read_to_string(&real) else {
        return;
    };
    let dir = temp_dir("real");
    std::fs::write(dir.join("options.ini"), &original).unwrap();
    let mut c = core_in(&dir);
    if let Some(lang) = original.lines().find_map(|l| l.strip_prefix("language=")) {
        c.system_language = lang.to_string();
    }
    c.load_options().unwrap();
    let saved = std::fs::read_to_string(dir.join("options.ini")).unwrap();
    if cfg!(target_os = "macos") {
        assert_eq!(saved, original);
    }
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn key_bindings() {
    let mut c = Core::new();
    c.add_key_binding("Forward", 17, 200, false, false, false);
    c.add_key_binding("Shout", 44, 0, false, false, false);
    c.add_key_binding("Crouch", 44, 0, false, true, false);
    assert_eq!(c.get_key("Forward"), 17);
    assert_eq!(c.get_alt_key("Forward"), 200);
    assert!(c.is_key("Forward", 17));
    assert!(c.is_key("Forward", 200));
    assert!(!c.is_key("Forward", 18));
    assert!(c.is_key("Shout", 44));
    // Crouch needs ctrl.
    assert!(!c.is_key("Crouch", 44));
    c.keys_down_raw.insert(29);
    assert!(c.is_key("Crouch", 44));
    // Shout's modifiers don't match while Crouch's do.
    assert!(!c.is_key("Shout", 44));
    c.keys_down_raw.clear();
    // Rebinding moves the key.
    c.add_key_binding("Forward", 18, 0, false, false, false);
    assert_eq!(c.get_key_binding_by_key(17), KeyBinding::empty());
    assert_eq!(c.get_key_binding_by_key(18).name, "Forward");
    assert_eq!(c.get_key_binding_by_key(44).name, "Shout");
    c.reinit_key_maps();
    assert_eq!(c.get_key("Forward"), 0);
    assert_eq!(
        KeyBinding::empty().to_string(),
        "KeyBinding[name=empty, keyValue=0, altKey=0, shift=false, ctrl=false, alt=false]"
    );
}

#[test]
fn zoom_and_screen() {
    let mut c = Core::new();
    c.set_option_zoom_levels2x(Some("50;75;125;300;abc;75"));
    assert_eq!(c.get_option_zoom_levels2x(), "");
    c.set_option_zoom_levels2x(Some("50;75;125"));
    c.init_fbos();
    assert!(c.offscreen_buffer.created);
    assert_eq!(
        c.offscreen_buffer.zoom_levels.as_deref(),
        Some(&[1.25f32, 1.0, 0.75, 0.5][..])
    );
    assert_eq!(c.get_max_zoom(), 1.25);
    assert_eq!(c.get_min_zoom(), 0.5);
    assert_eq!(c.get_next_zoom(0, 1), 1.25);
    assert_eq!(c.get_next_zoom(0, -1), 0.75);
    c.do_zoom_scroll(0, -1);
    assert_eq!(c.offscreen_buffer.target_zoom[0], 0.75);
    assert_eq!(c.get_zoom(0), 1.0);
    c.set_screen_size(1920, 1080);
    assert_eq!(
        c.take_events(),
        vec![CoreEvent {
            name: "OnResolutionChange",
            args: vec![
                EventArg::Num(1280.0),
                EventArg::Num(720.0),
                EventArg::Num(1920.0),
                EventArg::Num(1080.0)
            ]
        }]
    );
    c.set_screen_size(1920, 1080);
    assert!(c.take_events().is_empty());
    assert_eq!(c.get_option_font_size_real(), 2);
    c.set_screen_size(1920, 1600);
    assert_eq!(c.get_option_font_size_real(), 4);
    assert_eq!(c.get_x_angle(100, 0.0), 0);
    assert_eq!(c.get_y_angle(100, 90.0), 0);
}

#[test]
fn setters_validate_like_java() {
    let mut c = Core::new();
    c.set_option_ambient_volume(11);
    assert_eq!(c.get_option_ambient_volume(), 5);
    c.set_option_context_menu_font(Some("Huge"));
    assert_eq!(c.get_option_context_menu_font(), "Medium");
    c.set_option_min_chat_opaque(0.7);
    assert_eq!(
        c.option("minChatOpaque").unwrap().get_value_as_string(),
        "0.699999988079071"
    );
    c.set_option_gamepad_binding_preset(Some("Custom"));
    assert_eq!(
        c.take_events(),
        vec![CoreEvent {
            name: "OptionGamepadBindingPresetChanged",
            args: vec![EventArg::Str("Custom".into())]
        }]
    );
    c.set_option_gamepad_binding_preset(Some("Custom"));
    assert!(c.take_events().is_empty());
    c.set_mp_text_color(ColorInfo::new(0.5, 0.25, 1.0, 1.0));
    assert_eq!(c.get_mp_text_color(), ColorInfo::new(0.5, 0.25, 1.0, 1.0));
    assert_eq!(
        c.option("mpTextColor").unwrap().get_value_as_string(),
        "0.5,0.25,1.0"
    );
    c.set_game_mode("LastStand");
    assert!(!c.add_zombie_on_cell_load);
    c.set_console_dot_txt_size_kb(1);
    assert_eq!(c.get_console_dot_txt_size_kb(), 512);
    c.init_poisonous_berry();
    assert!(c.get_poisonous_berry().unwrap().starts_with("Base.Berry"));
    c.set_framerate(5);
    assert_eq!(c.performance.lock_fps, 144);
    assert!(c.take_full_screenshot(Some("../x.png")).is_err());
}
