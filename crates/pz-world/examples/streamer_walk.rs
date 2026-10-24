//! Walk a player over a save with the chunk streamer and print what it
//! does, one line per [`StreamEvent`], for comparing against the original
//! `IsoChunkMap`/`WorldStreamer` driven by a local harness on a machine with
//! the game installed. Results are recorded as notes only.
//!
//! ```text
//! streamer_walk <game dir> <save dir> <work dir> <script> <out file>
//! ```
//!
//! `<save dir>/map` is copied to `<work dir>/map` first (`<work dir>` must
//! not exist): chunks unloaded during the walk are saved there, never into
//! `<save dir>`. Item names come from `<save dir>/../items.txt` when the
//! harness wrote one, otherwise every item is known. The map has no lot
//! files, so brand-new chunks are empty, as in the harness.
//!
//! The script's first line is `width <chunkGridWidth>`; then
//! `load <wx> <wy>` loads the world around a chunk with the player in its
//! middle, and `frame <x> <y> <all|n>` moves the player to tile `(x, y)` and
//! runs one frame, serving all jobs or `n` streamer iterations.

use std::path::Path;

use pz_world::chunk::IsoCell;
use pz_world::fix2x::Fix2xTable;
use pz_world::save::items::AllItems;
use pz_world::save::{NoSaveHooks, SaveSettings, SaveTables};
use pz_world::streamer::{ChunkSource, ChunkStreamer, PlayerPos, Serve, StreamCtx, StreamEvent};
use pz_world::{
    GameRand, IsoMetaGrid, IsoSpriteManager, NoHooks, ObjectLoadSettings, ObjectTables,
    SpriteTables,
};

struct NoRand;
impl GameRand for NoRand {
    fn next(&mut self, _max: i32) -> i32 {
        0
    }
}

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for e in std::fs::read_dir(from).unwrap() {
        let e = e.unwrap();
        let p = e.path();
        if p.is_dir() {
            copy_dir(&p, &to.join(e.file_name()));
        } else {
            std::fs::copy(&p, to.join(e.file_name())).unwrap();
        }
    }
}

fn line(e: &StreamEvent) -> String {
    match e {
        StreamEvent::Requested { wx, wy, .. } => format!("R {wx},{wy}"),
        StreamEvent::Served { wx, wy, source } => format!(
            "S {wx},{wy} {}",
            match source {
                ChunkSource::Disk => "d".to_string(),
                ChunkSource::BrandNew => "n".into(),
                ChunkSource::BrandNewAfterError(e) => format!("e {e}"),
            }
        ),
        StreamEvent::Cancelled { wx, wy } => format!("C {wx},{wy}"),
        StreamEvent::HandedOff { wx, wy } => format!("H {wx},{wy}"),
        StreamEvent::Discarded { wx, wy } => format!("D {wx},{wy}"),
        StreamEvent::Unloaded { wx, wy } => format!("U {wx},{wy}"),
        StreamEvent::Saved { wx, wy, .. } => format!("W {wx},{wy}"),
        StreamEvent::SaveFailed { wx, wy, error } => format!("W {wx},{wy} failed: {error}"),
    }
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    if a.len() != 6 {
        eprintln!("usage: streamer_walk <game dir> <save dir> <work dir> <script> <out file>");
        std::process::exit(2);
    }
    let game = Path::new(&a[1]);
    let (save, work) = (Path::new(&a[2]), Path::new(&a[3]));
    assert!(!work.exists(), "work dir must not exist");
    copy_dir(&save.join("map"), &work.join("map"));
    let jar = game.join("projectzomboid.jar");
    let sprite_tables = SpriteTables::from_game_jar(&jar).unwrap();
    let object_tables = ObjectTables::from_game_jar(&jar).unwrap();
    let save_tables = SaveTables::from_game_jar(&jar).unwrap();
    let mut sprites =
        IsoSpriteManager::load_tile_definitions_from(sprite_tables, &game.join("media"), &[])
            .unwrap();
    sprites.loaded_tile_definitions(&|_| false).unwrap();
    let mut meta = IsoMetaGrid::empty();
    (meta.max_x, meta.max_y) = (99, 99);
    let fix2x = Fix2xTable::default();
    let settings = SaveSettings::default();
    let script = std::fs::read_to_string(&a[4]).unwrap();
    let mut lines = script.lines();
    let width: i32 = lines
        .next()
        .unwrap()
        .split_whitespace()
        .nth(1)
        .unwrap()
        .parse()
        .unwrap();
    let mut s = ChunkStreamer::new(1, width);
    let mut cell = IsoCell::default();
    let (mut rand, mut object_hooks) = (NoRand, NoHooks);
    let mut out = String::new();
    let mut last = (0.0f32, 0.0f32);
    for l in lines {
        let l = l.trim();
        if l.is_empty() || l.starts_with('#') {
            continue;
        }
        out.push_str(&format!("## {l}\n"));
        let t: Vec<&str> = l.split_whitespace().collect();
        let mut ctx = StreamCtx {
            save_dir: work,
            meta: &meta,
            sprites: &mut sprites,
            fix2x: &fix2x,
            rand: &mut rand,
            object_tables: &object_tables,
            object_settings: ObjectLoadSettings::default(),
            object_hooks: &mut object_hooks,
            save_tables: &save_tables,
            save_settings: &settings,
            items: &AllItems,
            save_hooks: &NoSaveHooks,
            brand_new: None,
            chunk_hook: None,
            no_save: false,
        };
        match t[0] {
            "load" => {
                let (wx, wy) = (t[1].parse().unwrap(), t[2].parse().unwrap());
                s.load_world(wx, wy, &mut cell, &mut ctx);
                last = ((wx * 8 + 4) as f32, (wy * 8 + 4) as f32);
            }
            "frame" => {
                let (x, y): (f32, f32) = (t[1].parse().unwrap(), t[2].parse().unwrap());
                let serve = match t[3] {
                    "all" => Serve::UntilIdle,
                    n => Serve::Jobs(n.parse().unwrap()),
                };
                let p = PlayerPos::moved(last.0, last.1, x, y);
                s.update(&[Some(p)], serve, &mut cell, &mut ctx);
                last = (x, y);
            }
            _ => panic!("bad script line {l}"),
        }
        for e in s.take_events() {
            out.push_str(&line(&e));
            out.push('\n');
        }
    }
    std::fs::write(&a[5], out).unwrap();
}
