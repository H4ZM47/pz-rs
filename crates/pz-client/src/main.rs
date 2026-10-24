//! Desktop client entry point (`zombie.gameStates.MainScreenState.main` /
//! `GameWindow`).
//!
//! `--headless-frames N` runs the boot sequence and N main-menu frames
//! without a window and prints Lua errors and UI statistics; it is the
//! development harness until the GPU backend lands.

mod main_screen;

use std::path::PathBuf;

/// Default install locations of the game's `Contents/Java` (macOS) or
/// game directory (Windows/Linux).
fn default_game_dirs() -> Vec<PathBuf> {
    let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")).map(PathBuf::from).unwrap_or_default();
    vec![
        home.join("Library/Application Support/Steam/steamapps/common/ProjectZomboid/Project Zomboid.app/Contents/Java"),
        home.join(".steam/steam/steamapps/common/ProjectZomboid/projectzomboid"),
        home.join(".local/share/Steam/steamapps/common/ProjectZomboid/projectzomboid"),
        PathBuf::from("C:/Program Files (x86)/Steam/steamapps/common/ProjectZomboid"),
    ]
}

fn find_game_dir(arg: Option<PathBuf>) -> Option<PathBuf> {
    let candidates = arg.into_iter().chain(std::env::var_os("PZ_GAME_DIR").map(PathBuf::from)).chain(default_game_dirs());
    candidates.into_iter().find(|p| p.join("media").is_dir())
}

fn main() {
    let mut args = std::env::args().skip(1);
    let mut game_dir = None;
    let mut headless_frames = None;
    while let Some(a) = args.next() {
        match a.as_str() {
            "--game-dir" => game_dir = args.next().map(PathBuf::from),
            "--headless-frames" => headless_frames = args.next().and_then(|n| n.parse::<u32>().ok()),
            "--help" | "-h" => {
                println!("pz-client [--game-dir DIR] [--headless-frames N]");
                return;
            }
            other => {
                eprintln!("unknown argument {other}");
                std::process::exit(2);
            }
        }
    }
    let Some(dir) = find_game_dir(game_dir) else {
        eprintln!("Cannot find the Project Zomboid install (a directory containing media/). Use --game-dir or PZ_GAME_DIR.");
        std::process::exit(2);
    };
    let frames = headless_frames.unwrap_or(1);
    if let Err(e) = main_screen::run_headless(&dir, frames) {
        eprintln!("{e}");
        std::process::exit(1);
    }
}
