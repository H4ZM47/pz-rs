//! Opt-in regression through the actual Rust session owners. Installed
//! media is read at runtime; saves always go into a fresh test cache.

use super::{headless_app, run_frames, DevGame, Launch, NewGameSetup, Start};
use pz_core::core::Core;
use pz_lua_api::{bind_core, bind_players, player_db::PlayerDb, LuaManager};
use pz_characters::character::player::PlayerInput;
use pz_anim::AnimationVariableSource;
use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::{SystemTime, UNIX_EPOCH};

const INGAME: &str = "zombie.gameStates.IngameState";

struct RestoreCore(Rc<RefCell<Core>>);

impl Drop for RestoreCore {
    fn drop(&mut self) {
        bind_core::install_core(self.0.clone());
    }
}

fn new_cache(game_dir: &Path) -> PathBuf {
    let base = std::env::var_os("PZ_SESSION_CHECK_CACHE")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    std::fs::create_dir_all(&base).expect("create session-check cache parent");
    let base = base.canonicalize().expect("resolve session-check cache parent");
    assert!(
        !base.starts_with(game_dir),
        "session-check output must be outside the installed game directory"
    );
    let stamp = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
    let cache = base.join(format!("pz-rs-session-check-{}-{stamp}", std::process::id()));
    // create_dir deliberately refuses to reuse any existing cache/save.
    std::fs::create_dir(&cache).expect("create fresh session-check cache");
    cache
}

fn lua_check(lua: &mut LuaManager, source: &str) {
    let f = pz_kahlua::stdlib::load_string(
        source,
        Some("rust-session-regression"),
        lua.thread.globals().clone(),
    )
    .expect("compile session-check Lua");
    lua.thread.pcall(&f, &[]).unwrap_or_else(|(error, trace)| {
        panic!("session-check Lua failed: {error:?}\n{trace}")
    });
}

fn save_counts(lua: &LuaManager) -> (Option<f64>, Option<f64>) {
    let globals = lua.thread.globals();
    (
        globals.get_str("rustSessionSaveCount").as_num(),
        globals.get_str("rustSessionPostSaveCount").as_num(),
    )
}

fn movement_diagnostics(app: &mut super::App, label: &str) {
    let input = pz_player_input::EnginePlayerInput::capture();
    let binding = app.core.borrow().get_key_binding("Forward");
    let raw = pz_lua_api::bind_input::is_key_down_raw(17);
    eprintln!("movement {label}: Forward={binding:?}, rawW={raw}, namedForward={}, textEntry={}, luaKeys={}",
        input.is_key_down("Forward"), input.is_doing_text_entry(), input.do_lua_key_pressed());
    let Some(player) = bind_players::player(0) else { return };
    let p = player.borrow();
    eprintln!("movement {label}: id={}, position=({},{},{}), action={}, children={:?}, moving={}, pressedMovement={}, isMoving={}, blockMovement={}, ignoreMovement={}, deferredEnabled={}, moveDelta={}",
        p.id, p.vars.x, p.vars.y, p.vars.z, p.state.action_context.current_state_name(),
        p.state.action_context.child_states().iter().map(|s| s.name()).collect::<Vec<_>>(),
        p.player_moving, p.get_variable_boolean("pressedMovement"), p.get_variable_boolean("isMoving"),
        p.vars.block_movement, p.vars.ignore_movement, p.vars.deferred_movement_enabled, p.vars.move_delta);
    if let Some(models) = app.world.as_mut().and_then(|w| w.models.as_mut()) {
        if let Some((animation, animator)) = models.animation_mut(p.id) {
            let root = animation.deferred_movement(false);
            let tracks: Vec<_> = animation.tracks().iter().map(|t| (t.name(), t.current_track_time(), t.blend_weight(), t.deferred_movement_bone_idx(), t.use_deferred_movement(), t.deferred_movement_diff())).collect();
            eprintln!("movement {label}: animState={:?}, root=({},{}), tracks={tracks:?}", animator.current_state_name(), root.x, root.y);
        } else {
            eprintln!("movement {label}: no live character animation");
        }
    }
}

#[test]
#[ignore = "requires PZ_GAME_DIR; boots installed media in the Rust engine and writes a fresh test save"]
fn installed_assets_new_game_moves_animates_and_saves_on_close() {
    let game_dir = PathBuf::from(std::env::var_os("PZ_GAME_DIR").expect("set PZ_GAME_DIR to installed game assets"))
        .canonicalize()
        .expect("resolve installed game directory");
    assert!(game_dir.join("projectzomboid.jar").is_file(), "PZ_GAME_DIR must contain projectzomboid.jar");
    assert!(game_dir.join("media").is_dir(), "PZ_GAME_DIR must contain media");
    let cache_dir = new_cache(&game_dir);
    eprintln!("session-check cache: {}", cache_dir.display());

    let _restore_core = RestoreCore(bind_core::shared_core());
    bind_core::install_core(Rc::new(RefCell::new(Core::default())));
    let mut app = headless_app(Launch {
        game_dir,
        cache_dir: cache_dir.clone(),
        start: Start::Game(DevGame {
            game_mode: "Sandbox".into(),
            world: "rust-session-regression".into(),
            new_game: Some(NewGameSetup { spawn: None, seed: "RustSessionRegression".into() }),
            map: "Muldraugh, KY".into(),
        }),
    })
    .expect("create Rust session");
    app.core.borrow_mut().set_no_save(false);
    app.boot().expect("boot Rust session from installed media");
    movement_diagnostics(&mut app, "boot");

    // Use the real loading/state-machine path, including population startup.
    for _ in 0..1100 {
        run_frames(&mut app, 1);
        if app.current_state() == Some(INGAME) {
            break;
        }
    }
    assert_eq!(app.current_state(), Some(INGAME), "session never entered gameplay; cache: {}", cache_dir.display());
    run_frames(&mut app, 4);
    movement_diagnostics(&mut app, "before W");
    let player = bind_players::player(0).expect("new session created local player 0");
    let (player_id, before) = {
        let p = player.borrow();
        (p.id, (p.vars.x, p.vars.y, p.vars.z))
    };

    // Keyboard scan code 17 is W. It enters the same input queue as the
    // native window; no character coordinate or movement flag is set here.
    pz_input::with_input(|i| i.key(17, 1));
    let mut late_start = None;
    for frame in 1..=60 {
        run_frames(&mut app, 1);
        if frame <= 5 || frame >= 56 || [10, 20, 40].contains(&frame) {
            movement_diagnostics(&mut app, &format!("W frame {frame}"));
        }
        if frame == 10 || frame == 40 {
            let p = player.borrow();
            let position = (p.vars.x, p.vars.y, p.vars.z);
            if frame == 10 {
                assert!(
                    (position.0 - before.0).abs() + (position.1 - before.1).abs() > 0.0001,
                    "real W input did not move the player during its first ten frames: {before:?} -> {position:?}; cache: {}",
                    cache_dir.display()
                );
            } else {
                late_start = Some(position);
            }
        }
    }
    let late_start = late_start.expect("captured held-W frame 40");
    let held_end = {
        let p = player.borrow();
        (p.vars.x, p.vars.y, p.vars.z)
    };
    assert!(
        (held_end.0 - late_start.0).abs() + (held_end.1 - late_start.1).abs() > 0.0001,
        "held W stopped moving the player during frames 41-60: {late_start:?} -> {held_end:?}; cache: {}",
        cache_dir.display()
    );
    pz_input::with_input(|i| i.key(17, 0));
    run_frames(&mut app, 1);
    let after = {
        let p = player.borrow();
        (p.vars.x, p.vars.y, p.vars.z)
    };
    assert!(after.0.is_finite() && after.1.is_finite() && after.2.is_finite(), "player position became non-finite");
    assert!(
        (after.0 - before.0).abs() + (after.1 - before.1).abs() > 0.0001,
        "real W input did not move the player: {before:?} -> {after:?}; cache: {}",
        cache_dir.display()
    );
    let world = app.world.as_ref().expect("live gameplay world");
    let models = world.models.as_ref().expect("world model owner exists");
    let (animation, _) = models.animation(player_id).expect("live player has an animation player");
    assert!(animation.num_bones() > 0, "player animation has no loaded rig");
    assert!(animation.track_count() > 0, "player animation has no live clip tracks");
    let save_dir = world.save_dir.clone();
    let sql_id = world.player_sql_ids[0];
    assert!(save_dir.starts_with(&cache_dir), "session save escaped its fresh cache");
    assert!(sql_id > 0, "local player has no persistent database id");

    lua_check(app.lua.as_mut().unwrap(), r#"
        rustSessionSaveCount, rustSessionPostSaveCount = 0, 0
        Events.OnSave.Add(function() rustSessionSaveCount = rustSessionSaveCount + 1 end)
        Events.OnPostSave.Add(function() rustSessionPostSaveCount = rustSessionPostSaveCount + 1 end)
    "#);
    // This is the native CloseRequested path, not a direct save helper.
    app.close();
    assert!(app.world.is_none(), "native close retained the gameplay world");
    assert_eq!(app.current_state(), None);
    assert_eq!(save_counts(app.lua.as_ref().unwrap()), (Some(1.0), Some(1.0)));
    app.close();
    assert_eq!(save_counts(app.lua.as_ref().unwrap()), (Some(1.0), Some(1.0)), "second close saved twice");

    let db_path = save_dir.join("players.db");
    assert!(db_path.is_file(), "native close did not create players.db");
    let mut db = PlayerDb::new(false, &save_dir.to_string_lossy());
    let saved = db.load_player(sql_id).expect("native close persisted the local player");
    assert!(!saved.bytes.is_empty(), "saved player has no serialized gameplay data");
    assert!(saved.world_version > 0);
    assert!((saved.x - after.0).abs() < 0.0001, "saved x differs: {} versus {}", saved.x, after.0);
    assert!((saved.y - after.1).abs() < 0.0001, "saved y differs: {} versus {}", saved.y, after.1);
    assert!((saved.z - after.2).abs() < 0.0001, "saved z differs: {} versus {}", saved.z, after.2);
    eprintln!("session-check passed: {before:?} -> {after:?}; save: {}", db_path.display());
}
