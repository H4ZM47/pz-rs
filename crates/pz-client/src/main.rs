//! Desktop client entry point (`zombie.gameStates.MainScreenState.main` /
//! `GameWindow`).
//!
//! `--headless-frames N` runs the boot sequence and N main-menu frames
//! without a window and prints Lua errors and UI statistics; it is the
//! development harness until the GPU backend lands.
//!
//! `--new-game <name>` / `--load-game <mode>/<name>` skip the main menu:
//! they set `Core.gameMode` / `gameSaveWorld` (and for a new game
//! `createWorld` and the player setup) as the menu's Lua does, then go
//! through `LoadingQueueState` and `GameLoadingState` into `IngameState`.
//! The save lives under `<cache dir>/Saves`. `--game-mode` (default
//! `Sandbox`), `--map` (default `Muldraugh, KY`), `--spawn x,y,z` (default:
//! a point of the map's `spawnpoints.lua`) and `--seed` shape a new game.
//! With `--headless-frames N` (and optionally `--png`) the frames run
//! without a window.

mod install;
mod main_screen;

use std::path::PathBuf;

fn main() {
    let mut args = std::env::args().skip(1);
    let mut game_dir = None;
    let mut headless_frames = None;
    let mut cache_dir = None;
    let mut png_out: Option<PathBuf> = None;
    let mut new_game: Option<String> = None;
    let mut load_game: Option<String> = None;
    let mut game_mode = "Sandbox".to_string();
    let mut map = "Muldraugh, KY".to_string();
    let mut spawn = None;
    let mut seed = String::new();
    while let Some(a) = args.next() {
        match a.as_str() {
            "--new-game" => new_game = args.next(),
            "--load-game" => load_game = args.next(),
            "--game-mode" => game_mode = args.next().unwrap_or(game_mode),
            "--map" => map = args.next().unwrap_or(map),
            "--seed" => seed = args.next().unwrap_or_default(),
            "--spawn" => {
                let v: Vec<i32> = args.next().unwrap_or_default().split(',').filter_map(|n| n.trim().parse().ok()).collect();
                if v.len() != 3 {
                    eprintln!("--spawn takes x,y,z");
                    std::process::exit(2);
                }
                spawn = Some((v[0], v[1], v[2]));
            }
            "--game-dir" => game_dir = args.next().map(PathBuf::from),
            "--png" => png_out = args.next().map(PathBuf::from),
            "--cache-dir" => cache_dir = args.next().map(PathBuf::from),
            "--headless-frames" => headless_frames = args.next().and_then(|n| n.parse::<u32>().ok()),
            "--help" | "-h" => {
                println!(
                    "pz-client [--game-dir DIR] [--cache-dir DIR] [--headless-frames N] [--png OUT] \
                     [--new-game NAME [--game-mode MODE] [--map MAP] [--spawn X,Y,Z] [--seed S] | --load-game MODE/NAME]"
                );
                return;
            }
            other => {
                eprintln!("unknown argument {other}");
                std::process::exit(2);
            }
        }
    }
    let Some(dir) = install::find_game_dir(game_dir) else {
        eprintln!("Cannot find the Project Zomboid install (a directory containing media/) in the Steam libraries. Use --game-dir or PZ_GAME_DIR.");
        std::process::exit(2);
    };
    // Development cache dir: ~/Zomboid-rs unless --cache-dir is given. The
    // original game's ~/Zomboid is never written.
    let cache_dir = cache_dir.unwrap_or_else(|| {
        install::home_dir().join("Zomboid-rs")
    });
    let _ = std::fs::create_dir_all(&cache_dir);
    use pz_engine::game_window::{DevGame, Launch, Start};
    let start = match (new_game, load_game) {
        (Some(_), Some(_)) => {
            eprintln!("--new-game and --load-game exclude each other");
            std::process::exit(2);
        }
        (Some(world), None) => Start::Game(DevGame {
            game_mode,
            world,
            new_game: Some(pz_engine::iso_world::NewGameSetup { spawn, seed }),
            map,
        }),
        (None, Some(path)) => {
            let Some((mode, world)) = path.trim_end_matches('/').rsplit_once('/') else {
                eprintln!("--load-game takes <game mode>/<save name> (under <cache dir>/Saves)");
                std::process::exit(2);
            };
            let mode = mode.rsplit('/').next().unwrap_or(mode).to_string();
            if !cache_dir.join("Saves").join(&mode).join(world).is_dir() {
                eprintln!("no save {mode}/{world} under {}", cache_dir.join("Saves").display());
                std::process::exit(2);
            }
            Start::Game(DevGame { game_mode: mode, world: world.to_string(), new_game: None, map })
        }
        (None, None) => Start::MainMenu,
    };
    let in_game = matches!(start, Start::Game(_));
    if let (Some(out), Some(frames)) = (png_out.clone(), headless_frames) {
        let cache = cache_dir.clone();
        match pz_engine::game_window::render_headless(Launch { game_dir: dir, cache_dir: cache, start }, frames) {
            Ok((w, h, px)) => {
                let f = std::fs::File::create(&out).expect("png");
                let mut e = png::Encoder::new(std::io::BufWriter::new(f), w, h);
                e.set_color(png::ColorType::Rgba);
                e.set_depth(png::BitDepth::Eight);
                e.write_header().unwrap().write_image_data(&px).unwrap();
                println!("wrote {} ({w}x{h})", out.display());
            }
            Err(e) => {
                eprintln!("{e}");
                std::process::exit(1);
            }
        }
        return;
    }
    if let (true, Some(frames)) = (in_game, headless_frames) {
        match pz_engine::game_window::run_headless(Launch { game_dir: dir, cache_dir, start }, frames) {
            Ok(state) => println!("state after {frames} frames: {}", state.unwrap_or("none")),
            Err(e) => {
                eprintln!("{e}");
                std::process::exit(1);
            }
        }
        return;
    }
    if let Some(frames) = headless_frames {
        if let Err(e) = main_screen::run_headless(&dir, &cache_dir, frames) {
            eprintln!("{e}");
            std::process::exit(1);
        }
        return;
    }
    if let Err(e) = pz_engine::game_window::run(Launch { game_dir: dir, cache_dir, start }) {
        eprintln!("{e}");
        std::process::exit(1);
    }
}
