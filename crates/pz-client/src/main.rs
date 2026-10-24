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
    let mut cache_dir = None;
    let mut png_out: Option<PathBuf> = None;
    while let Some(a) = args.next() {
        match a.as_str() {
            "--game-dir" => game_dir = args.next().map(PathBuf::from),
            "--png" => png_out = args.next().map(PathBuf::from),
            "--cache-dir" => cache_dir = args.next().map(PathBuf::from),
            "--headless-frames" => headless_frames = args.next().and_then(|n| n.parse::<u32>().ok()),
            "--help" | "-h" => {
                println!("pz-client [--game-dir DIR] [--cache-dir DIR] [--headless-frames N]");
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
    if let (Some(out), Some(frames)) = (png_out.clone(), headless_frames) {
        let cache = cache_dir.clone().unwrap_or_else(|| std::env::temp_dir().join("pz-rs-cache"));
        let _ = std::fs::create_dir_all(&cache);
        match pz_engine::game_window::render_headless(pz_engine::game_window::Launch { game_dir: dir, cache_dir: cache }, frames) {
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
    if let Some(frames) = headless_frames {
        if let Err(e) = main_screen::run_headless(&dir, frames) {
            eprintln!("{e}");
            std::process::exit(1);
        }
        return;
    }
    // Development default keeps the original game's ~/Zomboid untouched.
    let cache_dir = cache_dir.unwrap_or_else(|| {
        let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")).map(PathBuf::from).unwrap_or_default();
        home.join("Zomboid-rs")
    });
    let _ = std::fs::create_dir_all(&cache_dir);
    if let Err(e) = pz_engine::game_window::run(pz_engine::game_window::Launch { game_dir: dir, cache_dir }) {
        eprintln!("{e}");
        std::process::exit(1);
    }
}
