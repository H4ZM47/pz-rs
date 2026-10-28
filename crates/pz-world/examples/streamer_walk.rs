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
//! When the save has a `vehicles.db` it is copied to `<work dir>` and
//! every served chunk loads its vehicles from the copy; each handed-off
//! chunk's vehicles are printed after its `H` line. When it has an
//! `isoregiondata` directory that is copied too and loaded with
//! [`IsoRegions::init_world`] before the script runs (`IsoWorld.init`).
//!
//! The script's first line is `width <chunkGridWidth>`; then
//! `load <wx> <wy>` loads the world around a chunk with the player in its
//! middle, `frame <x> <y> <all|n>` moves the player to tile `(x, y)` and
//! runs one frame, serving all jobs or `n` streamer iterations, and
//! `regions` prints the region dump of the cell's chunks, runs one
//! [`IsoRegions::update`] (the first frame's `IsoWorld.updateWorld`) and
//! prints it again.

use std::path::Path;

use pz_world::chunk::IsoCell;
use pz_world::chunk_vehicles::VehicleOrigin;
use pz_world::fix2x::Fix2xTable;
use pz_world::isoregion::IsoRegions;
use pz_world::save::items::AllItems;
use pz_world::save::{NoSaveHooks, SaveSettings, SaveTables};
use pz_world::streamer::{ChunkSource, ChunkStreamer, PlayerPos, Serve, StreamCtx, StreamEvent};
use pz_world::{
    GameRand, IsoMetaGrid, IsoSpriteManager, NoHooks, ObjectLoadSettings, ObjectTables,
    SpriteTables,
};

#[path = "common/region_dump.rs"]
mod region_dump;

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
        StreamEvent::Removed { wx, wy } => format!("X {wx},{wy}"),
        StreamEvent::HotSave => "# hot save".into(),
        StreamEvent::VehiclesUnloaded {
            wx,
            wy,
            saved,
            errors,
        } => format!("# vehicles unloaded {wx},{wy} saved={saved:?} errors={errors:?}"),
        StreamEvent::VehiclesLoaded {
            wx,
            wy,
            removed,
            error,
            ..
        } => format!("# vehicles {wx},{wy} removed={removed:?} error={error:?}"),
        StreamEvent::VehiclesToMeta {
            wx,
            wy,
            ids,
            errors,
        } => format!("# vehicles to meta {wx},{wy} ids={ids:?} errors={errors:?}"),
        StreamEvent::VehiclesUpdated { saved, errors } => {
            format!("# vehicle rows saved={saved:?} errors={errors:?}")
        }
        StreamEvent::GenAnimalsPath { wx, wy } => format!("# animal paths {wx},{wy}"),
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
    let mut vehicles_db = None;
    if save.join("vehicles.db").exists() {
        std::fs::copy(save.join("vehicles.db"), work.join("vehicles.db")).unwrap();
        // `SQLStore.create` on an existing file: auto-commit off.
        let mut c = pz_sqlite::Connection::open(work.join("vehicles.db")).unwrap();
        c.set_autocommit(false).unwrap();
        vehicles_db = Some(c);
    }
    let mut regions = None;
    if save.join("isoregiondata").is_dir() {
        copy_dir(&save.join("isoregiondata"), &work.join("isoregiondata"));
        regions = Some(IsoRegions::init_world(work, width, false));
    }
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
            vehicles_db: vehicles_db.as_mut(),
            vehicle_saver: None,
            vehicle_meta: None,
            vehicle_ids: None,
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
            "regions" => {
                let r = regions.as_mut().expect("the save has no isoregiondata");
                let order: Vec<(i32, i32)> = {
                    let mut v: Vec<_> = cell.chunks.keys().copied().collect();
                    v.sort();
                    v
                };
                out.push_str("# regions before update\n");
                out.push_str(&region_dump::dump(r, &order));
                r.update(&cell, ctx.sprites, None);
                out.push_str("# regions after update\n");
                out.push_str(&region_dump::dump(r, &order));
            }
            _ => panic!("bad script line {l}"),
        }
        for e in s.take_events() {
            let t = line(&e);
            if t.starts_with("# vehicles")
                && (t.ends_with("removed=[] error=None") || t.ends_with("saved=[] errors=[]"))
            {
                continue;
            }
            out.push_str(&t);
            out.push('\n');
            let StreamEvent::HandedOff { wx, wy } = e else {
                continue;
            };
            for v in cell.chunks[&(wx, wy)].vehicles.loaded() {
                let VehicleOrigin::Database { sql_id } = v.origin else {
                    continue;
                };
                let sq = v
                    .square
                    .map_or("null".to_string(), |(x, y, z)| format!("{x},{y},{z}"));
                let b = &v.vehicle.base;
                out.push_str(&format!(
                    " v {sql_id} sq={sq} script={} x={} y={} z={} parts={}\n",
                    v.vehicle.script_name,
                    b.x.to_bits() as i32,
                    b.y.to_bits() as i32,
                    b.z.to_bits() as i32,
                    v.vehicle.parts.len()
                ));
            }
        }
    }
    std::fs::write(&a[5], out).unwrap();
}
