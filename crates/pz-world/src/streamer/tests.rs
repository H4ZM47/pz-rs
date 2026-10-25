//! A player walking over hand-made save directories.

use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

use super::*;
use crate::chunk::ChunksCache;
use crate::save::chunk_file::HookCtx;
use crate::save::items::AllItems;
use crate::save::NoSaveHooks;
use crate::sprite::test_tables;
use crate::{IsoGridSquare, NoHooks};

struct NoRand;
impl GameRand for NoRand {
    fn next(&mut self, _max: i32) -> i32 {
        0
    }
}

/// Records hook calls as strings.
#[derive(Default)]
struct Log(Vec<String>);

impl ChunkLoadHook for Log {
    fn load_grid_square(&mut self, _: &mut IsoChunk, _: i32, _: i32, _: i32, _: &mut HookCtx<'_>) {}
    fn chunk_loaded(&mut self, chunk: &mut IsoChunk, _: &mut HookCtx<'_>) {
        self.0.push(format!("loaded {},{}", chunk.wx, chunk.wy));
    }
    fn chunk_unloaded(&mut self, wx: i32, wy: i32) {
        self.0.push(format!("unloaded {wx},{wy}"));
    }
}

#[derive(Default)]
struct Gen(Vec<String>);

impl BrandNewHook for Gen {
    fn generate_chunks(
        &mut self,
        chunks: &mut ChunksCache,
        _: &IsoMetaGrid,
        _: &mut ChunkLoadEnv<'_>,
    ) {
        self.0.push(format!("generate {:?}", chunks.coords()[0]));
    }
    fn on_new_chunk_loaded(
        &mut self,
        chunk: &mut IsoChunk,
        _: &IsoMetaGrid,
        _: &mut ChunkLoadEnv<'_>,
    ) {
        // Mark brand-new chunks so a later save shows where they came from.
        chunk.saved.loot_respawn_hour = 7;
        self.0.push(format!("basements {},{}", chunk.wx, chunk.wy));
    }
}

/// A save directory and everything a [`StreamCtx`] borrows.
struct Fixture {
    dir: PathBuf,
    meta: IsoMetaGrid,
    sprites: IsoSpriteManager,
    fix2x: Fix2xTable,
    rand: NoRand,
    object_tables: ObjectTables,
    object_hooks: NoHooks,
    save_tables: SaveTables,
    save_settings: SaveSettings,
    gen: Gen,
    log: Log,
    no_save: bool,
    /// `vehicles.db`, when a test made one.
    vehicles: Option<pz_sqlite::Connection>,
}

static NEXT_DIR: AtomicUsize = AtomicUsize::new(0);

impl Fixture {
    /// A world of map cells `(0, 0)..=(1, 1)` (chunks `0..64`).
    fn new() -> Self {
        let dir = std::env::temp_dir().join(format!(
            "pz-streamer-{}-{}",
            std::process::id(),
            NEXT_DIR.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut meta = IsoMetaGrid::empty();
        (meta.max_x, meta.max_y) = (1, 1);
        Fixture {
            dir,
            meta,
            sprites: IsoSpriteManager::new(test_tables()),
            fix2x: Fix2xTable::default(),
            rand: NoRand,
            object_tables: ObjectTables::new(vec![1; 8], vec![(0, 0)]).unwrap(),
            object_hooks: NoHooks,
            save_tables: SaveTables::for_tests(),
            save_settings: SaveSettings::default(),
            gen: Gen::default(),
            log: Log::default(),
            no_save: false,
            vehicles: None,
        }
    }

    fn ctx(&mut self) -> StreamCtx<'_> {
        StreamCtx {
            save_dir: &self.dir,
            meta: &self.meta,
            sprites: &mut self.sprites,
            fix2x: &self.fix2x,
            rand: &mut self.rand,
            object_tables: &self.object_tables,
            object_settings: ObjectLoadSettings::default(),
            object_hooks: &mut self.object_hooks,
            save_tables: &self.save_tables,
            save_settings: &self.save_settings,
            items: &AllItems,
            save_hooks: &NoSaveHooks,
            brand_new: Some(&mut self.gen),
            chunk_hook: Some(&mut self.log),
            no_save: self.no_save,
            vehicles_db: self.vehicles.as_mut(),
            vehicle_saver: None,
            vehicle_meta: None,
            vehicle_ids: None,
        }
    }

    fn save_ctx(&self) -> SaveCtx<'_> {
        SaveCtx {
            sprites: &self.sprites,
            tables: &self.save_tables,
            settings: &self.save_settings,
            items: &AllItems,
            hooks: &NoSaveHooks,
            meta: None,
        }
    }

    /// Write a save file for chunk `(wx, wy)` whose loot respawn hour is
    /// `mark`.
    fn write_chunk(&self, wx: i32, wy: i32, mark: i32) {
        let mut ch = IsoChunk::new(wx, wy);
        for x in 0..8 {
            for y in 0..8 {
                ch.set_square(x, y, 0, Some(IsoGridSquare::new(wx * 8 + x, wy * 8 + y, 0)));
            }
        }
        ch.saved.loot_respawn_hour = mark;
        let path = chunk_path(&self.dir, wx, wy);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, save_chunk(&ch, &self.save_ctx()).unwrap()).unwrap();
    }

    /// The loot respawn hour saved for chunk `(wx, wy)`, if it has a file.
    fn saved_mark(&self, wx: i32, wy: i32) -> Option<i32> {
        let bytes = std::fs::read(chunk_path(&self.dir, wx, wy)).ok()?;
        let ch = load_chunk(&bytes, wx, wy, &self.save_ctx()).unwrap().chunk;
        Some(ch.saved.loot_respawn_hour)
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// Events as short strings: `R x,y` requested, `S x,y d|n|e` served from
/// disk / brand new / after an error, `C` cancelled, `H` handed off, `D`
/// discarded, `U` unloaded, `W x,y` saved (`-`: saving is off), `V`/`VU`
/// vehicles loaded/unloaded, `HOT` the hot save, `X` removed at exit.
fn short(events: &[StreamEvent]) -> Vec<String> {
    events
        .iter()
        .map(|e| match e {
            StreamEvent::Requested { wx, wy, .. } => format!("R {wx},{wy}"),
            StreamEvent::Served { wx, wy, source } => format!(
                "S {wx},{wy} {}",
                match source {
                    ChunkSource::Disk => "d",
                    ChunkSource::BrandNew => "n",
                    ChunkSource::BrandNewAfterError(_) => "e",
                }
            ),
            StreamEvent::Cancelled { wx, wy } => format!("C {wx},{wy}"),
            StreamEvent::HandedOff { wx, wy } => format!("H {wx},{wy}"),
            StreamEvent::Discarded { wx, wy } => format!("D {wx},{wy}"),
            StreamEvent::Unloaded { wx, wy } => format!("U {wx},{wy}"),
            StreamEvent::Saved { wx, wy, written } => {
                format!("W {wx},{wy}{}", if *written { "" } else { " -" })
            }
            StreamEvent::SaveFailed { wx, wy, error } => format!("F {wx},{wy} {error}"),
            StreamEvent::VehiclesLoaded {
                wx,
                wy,
                added,
                removed,
                error,
            } => format!("V {wx},{wy} {added} -{} {error:?}", removed.len()),
            StreamEvent::VehiclesUnloaded {
                wx,
                wy,
                saved,
                errors,
            } => format!(
                "VU {wx},{wy} {saved:?} {:?}",
                errors.iter().map(|e| e.1.as_str()).collect::<Vec<_>>()
            ),
            StreamEvent::HotSave => "HOT".into(),
            StreamEvent::Removed { wx, wy } => format!("X {wx},{wy}"),
            StreamEvent::VehiclesToMeta {
                wx,
                wy,
                ids,
                errors,
            } => format!(
                "VM {wx},{wy} {ids:?} {:?}",
                errors.iter().map(|e| e.1.as_str()).collect::<Vec<_>>()
            ),
            StreamEvent::GenAnimalsPath { wx, wy } => format!("G {wx},{wy}"),
            StreamEvent::VehiclesUpdated { saved, errors } => format!(
                "VQ {saved:?} {:?}",
                errors.iter().map(|e| e.1.as_str()).collect::<Vec<_>>()
            ),
        })
        .collect()
}

fn only<'a>(ev: &'a [String], prefix: &str) -> Vec<&'a str> {
    ev.iter()
        .filter(|e| e.starts_with(prefix))
        .map(|e| &e[prefix.len()..])
        .collect()
}

/// The chunks of a player's grid, row by row.
fn grid(s: &ChunkStreamer, player: usize) -> Vec<Vec<Option<(i32, i32)>>> {
    (0..s.width)
        .map(|y| (0..s.width).map(|x| s.grid_chunk(player, x, y)).collect())
        .collect()
}

#[test]
fn chunk_width_from_screen() {
    assert_eq!(calc_chunk_width(1920, 1080), 19);
    assert_eq!(calc_chunk_width(3840, 2160), 19);
    assert_eq!(calc_chunk_width(1280, 720), 13);
    assert_eq!(calc_chunk_width(800, 600), 11);
}

#[test]
fn loading_the_world_serves_the_last_request_first() {
    let mut f = Fixture::new();
    f.write_chunk(9, 10, 100);
    let mut s = ChunkStreamer::new(1, 3);
    let mut cell = IsoCell::default();
    s.load_world(10, 10, &mut cell, &mut f.ctx());
    let ev = short(&s.take_events());
    assert_eq!(
        only(&ev, "R "),
        ["9,9", "9,10", "9,11", "10,9", "10,10", "10,11", "11,9", "11,10", "11,11"]
    );
    // No player exists while loading, so the comparator ties everything.
    assert_eq!(
        only(&ev, "S "),
        [
            "11,11 n", "11,10 n", "11,9 n", "10,11 n", "10,10 n", "10,9 n", "9,11 n", "9,10 d",
            "9,9 n"
        ]
    );
    assert_eq!(only(&ev, "H ").len(), 9);
    assert_eq!(cell.chunks.len(), 9);
    assert_eq!(cell.chunks[&(9, 10)].saved.loot_respawn_hour, 100);
    assert_eq!(cell.chunks[&(10, 10)].saved.loot_respawn_hour, 7);
    assert_eq!(grid(&s, 0)[0], [Some((9, 9)), Some((10, 9)), Some((11, 9))]);
    // Erosion's `ChunkLoaded` runs at hand-off, in serve order.
    assert_eq!(f.log.0[0], "loaded 11,11");
    // World generation runs for each brand-new chunk's 2x2 block.
    assert_eq!(f.gen.0[0], "generate (10, 10)");
    assert_eq!(s.tile_bounds(0), (72, 72, 96, 96));
}

#[test]
fn invalid_chunks_at_the_world_edge_are_not_requested() {
    let mut f = Fixture::new();
    let mut s = ChunkStreamer::new(1, 3);
    let mut cell = IsoCell::default();
    s.load_world(0, 0, &mut cell, &mut f.ctx());
    let ev = short(&s.take_events());
    assert_eq!(only(&ev, "R "), ["0,0", "0,1", "1,0", "1,1"]);
    assert_eq!(grid(&s, 0)[0], [None, None, None]);
    assert_eq!(grid(&s, 0)[1], [None, Some((0, 0)), Some((1, 0))]);
}

#[test]
fn walking_east_unloads_saves_and_loads_the_closest_first() {
    let mut f = Fixture::new();
    let mut s = ChunkStreamer::new(1, 5);
    let mut cell = IsoCell::default();
    s.load_world(10, 10, &mut cell, &mut f.ctx());
    s.take_events();
    f.log.0.clear();

    // Step from tile 87 into chunk 11.
    let p = PlayerPos::moved(87.0, 84.0, 88.0, 84.0);
    s.update(&[Some(p)], Serve::UntilIdle, &mut cell, &mut f.ctx());
    let ev = short(&s.take_events());
    assert_eq!(
        ev,
        [
            // ProcessChunkPos: the west column falls off, the east column
            // is requested.
            "U 8,8",
            "U 8,9",
            "U 8,10",
            "U 8,11",
            "U 8,12",
            "R 13,8",
            "R 13,9",
            "R 13,10",
            "R 13,11",
            "R 13,12",
            // Streamer: the closest to 10 tiles ahead first (ties keep the
            // later request last, so it goes first), one queued save each.
            "W 8,8",
            "S 13,10 n",
            "W 8,9",
            "S 13,11 n",
            "W 8,10",
            "S 13,9 n",
            "W 8,11",
            "S 13,12 n",
            "W 8,12",
            // The queue is empty: `HotsaveAncilliarySystems`.
            "HOT",
            "S 13,8 n",
            // Hand-off: 1 + 5 * 3 / 5 = 4 this frame.
            "H 13,10",
            "H 13,11",
            "H 13,9",
            "H 13,12",
        ]
    );
    assert_eq!(
        f.log.0[..2],
        ["unloaded 8,8".to_string(), "unloaded 8,9".into()]
    );
    assert_eq!(s.pending_hand_offs(), 1);
    assert_eq!(s.chunk_map(0).world_x, 11);
    assert_eq!(f.saved_mark(8, 10), Some(7));
    assert!(!cell.chunks.contains_key(&(8, 10)));
    assert!(!cell.chunks.contains_key(&(13, 8)));

    // Next frame: no movement, the last chunk is handed off.
    let p = PlayerPos::at(88.0, 84.0);
    s.update(&[Some(p)], Serve::UntilIdle, &mut cell, &mut f.ctx());
    assert_eq!(short(&s.take_events()), ["H 13,8"]);
    assert_eq!(cell.chunks.len(), 25);
    assert_eq!(grid(&s, 0)[0][4], Some((13, 8)));
    assert_eq!(grid(&s, 0)[2][0], Some((9, 10)));
}

#[test]
fn walking_back_cancels_and_reloads_from_the_save_written_first() {
    let mut f = Fixture::new();
    let mut s = ChunkStreamer::new(1, 5);
    let mut cell = IsoCell::default();
    s.load_world(10, 10, &mut cell, &mut f.ctx());
    s.take_events();

    // East, but the streamer does not get to run this frame.
    let p = PlayerPos::moved(87.0, 84.0, 88.0, 84.0);
    s.update(&[Some(p)], Serve::Jobs(0), &mut cell, &mut f.ctx());
    s.take_events();
    assert_eq!(s.pending_saves(), 5);
    assert!(f.saved_mark(8, 10).is_none());

    // Straight back west.
    let p = PlayerPos::moved(88.0, 84.0, 87.0, 84.0);
    s.update(&[Some(p)], Serve::UntilIdle, &mut cell, &mut f.ctx());
    let ev = short(&s.take_events());
    assert_eq!(
        ev,
        [
            // The east column was never loaded: it only loses its ref.
            "R 8,8", "R 8,9", "R 8,10", "R 8,11", "R 8,12",
            // Jobs nobody wants go back to the chunk store, last first.
            "C 13,12", "C 13,11", "C 13,10", "C 13,9", "C 13,8",
            // The chunk about to load has its queued save written first.
            "W 8,10", "S 8,10 d", "W 8,11", "S 8,11 d", "W 8,9", "S 8,9 d", "W 8,12", "S 8,12 d",
            "W 8,8", "HOT", "S 8,8 d", "H 8,10", "H 8,11", "H 8,9", "H 8,12",
        ]
    );
}

#[test]
fn a_long_jump_reloads_the_whole_grid() {
    let mut f = Fixture::new();
    let mut s = ChunkStreamer::new(1, 3);
    let mut cell = IsoCell::default();
    s.load_world(10, 10, &mut cell, &mut f.ctx());
    s.take_events();
    let p = PlayerPos::moved(84.0, 84.0, 30.0 * 8.0 + 1.0, 84.0);
    s.update(&[Some(p)], Serve::Jobs(0), &mut cell, &mut f.ctx());
    let ev = short(&s.take_events());
    assert_eq!(
        ev,
        [
            "U 9,9", "U 10,9", "U 11,9", "U 9,10", "U 10,10", "U 11,10", "U 9,11", "U 10,11",
            "U 11,11", "R 29,9", "R 29,10", "R 29,11", "R 30,9", "R 30,10", "R 30,11", "R 31,9",
            "R 31,10", "R 31,11",
        ]
    );
    assert!(cell.chunks.is_empty());
    assert_eq!(grid(&s, 0), vec![vec![None; 3]; 3]);
    let p = PlayerPos::at(241.0, 84.0);
    s.update(&[Some(p)], Serve::UntilIdle, &mut cell, &mut f.ctx());
    let ev = short(&s.take_events());
    assert_eq!(only(&ev, "W ").len(), 9);
    assert_eq!(only(&ev, "S ")[0], "30,10 n");
    // 1 + 9 * 3 / 3 = 10: all nine.
    assert_eq!(only(&ev, "H ").len(), 9);
}

#[test]
fn split_screen_players_share_chunks() {
    let mut f = Fixture::new();
    let mut s = ChunkStreamer::new(2, 3);
    let mut cell = IsoCell::default();
    s.load_world(10, 10, &mut cell, &mut f.ctx());
    s.take_events();
    // Player 1 joins a chunk east of player 0: its map starts far away, so
    // the first update loads its whole grid, sharing six loaded chunks.
    let p0 = Some(PlayerPos::at(84.0, 84.0));
    let p1 = Some(PlayerPos::at(92.0, 84.0));
    s.update(&[p0, p1], Serve::UntilIdle, &mut cell, &mut f.ctx());
    let ev = short(&s.take_events());
    assert_eq!(only(&ev, "R "), ["12,9", "12,10", "12,11"]);
    assert_eq!(
        grid(&s, 1)[1],
        [Some((10, 10)), Some((11, 10)), Some((12, 10))]
    );
    // Player 0 steps west: column 11 is still player 1's and stays.
    let p0 = Some(PlayerPos::moved(80.0, 84.0, 79.0, 84.0));
    s.update(&[p0, p1], Serve::UntilIdle, &mut cell, &mut f.ctx());
    let ev = short(&s.take_events());
    assert!(only(&ev, "U ").is_empty());
    assert_eq!(only(&ev, "R "), ["8,9", "8,10", "8,11"]);
    assert!(cell.chunks.contains_key(&(11, 10)));
    // Player 1 steps east: column 10 is still player 0's.
    let p1 = Some(PlayerPos::moved(95.0, 84.0, 96.0, 84.0));
    s.update(&[p0, p1], Serve::UntilIdle, &mut cell, &mut f.ctx());
    let ev = short(&s.take_events());
    assert!(only(&ev, "U ").is_empty());
    // Player 0 steps west again: now column 10 has nobody.
    let p0 = Some(PlayerPos::moved(79.0, 84.0, 71.0, 84.0));
    s.update(&[p0, p1], Serve::UntilIdle, &mut cell, &mut f.ctx());
    let ev = short(&s.take_events());
    assert_eq!(only(&ev, "U "), ["10,9", "10,10", "10,11"]);
}

#[test]
fn a_bad_save_file_is_backed_up_and_replaced() {
    let mut f = Fixture::new();
    let path = chunk_path(&f.dir, 10, 10);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, [0u8; 3]).unwrap();
    let mut s = ChunkStreamer::new(1, 1);
    let mut cell = IsoCell::default();
    s.load_world(10, 10, &mut cell, &mut f.ctx());
    let ev = short(&s.take_events());
    assert_eq!(ev, ["R 10,10", "S 10,10 e", "H 10,10"]);
    assert_eq!(
        std::fs::read(f.dir.join("blam/10/10.bin")).unwrap(),
        [0u8; 3]
    );
    assert!(f.dir.join("blam/10/10_error.txt").exists());
}

#[test]
fn no_save_drops_unloaded_chunks() {
    let mut f = Fixture::new();
    f.no_save = true;
    let mut s = ChunkStreamer::new(1, 1);
    let mut cell = IsoCell::default();
    s.load_world(10, 10, &mut cell, &mut f.ctx());
    s.take_events();
    let p = PlayerPos::moved(87.0, 84.0, 88.0, 84.0);
    s.update(&[Some(p)], Serve::UntilIdle, &mut cell, &mut f.ctx());
    let ev = short(&s.take_events());
    assert_eq!(
        ev,
        [
            "U 10,10",
            "R 11,10",
            "W 10,10 -",
            "HOT",
            "S 11,10 n",
            "H 11,10"
        ]
    );
    assert!(!chunk_path(&f.dir, 10, 10).exists());
}

#[test]
fn comparator_ties_and_direction() {
    let p = PlayerPos::moved(0.0, 0.0, 3.0, 4.0);
    let t = ChunkStreamer::comparator_targets(&[Some(p)]);
    assert_eq!(t[0], (9.0, 12.0));
    assert_eq!(t[1], (-1.0, -1.0));
    use std::cmp::Ordering::*;
    assert_eq!(compare_chunks(&t, (0, 0), (1, 1)), Less);
    assert_eq!(compare_chunks(&t, (1, 1), (0, 0)), Greater);
    let none = [(-1.0, -1.0); 4];
    assert_eq!(compare_chunks(&none, (0, 0), (5, 5)), Equal);
}

/// A `vehicles.db` in the fixture's save directory with one good vehicle
/// row per `(id, x, y)`, in the chunk the position is in.
fn vehicles_db(f: &mut Fixture, rows: &[(i64, f64, f64)]) {
    use pz_sqlite::Value;
    let mut c = pz_sqlite::Connection::open(f.dir.join("vehicles.db")).unwrap();
    c.execute("CREATE TABLE vehicles (id   INTEGER PRIMARY KEY NOT NULL,wx    INTEGER,wy    INTEGER,x    FLOAT,y    FLOAT,inMeta BOOLEAN NULL DEFAULT FALSE,worldversion    INTEGER,data BLOB);").unwrap();
    let mut w = crate::save::io::Writer::new();
    crate::save::movers_tests::vehicle()
        .save(
            &mut w,
            f.save_tables.class_id("Vehicle"),
            None,
            None,
            &AllItems,
        )
        .unwrap();
    for &(id, x, y) in rows {
        c.insert(
            "vehicles",
            &[
                ("id", Value::Integer(id)),
                ("wx", Value::Integer((x / 8.0) as i64)),
                ("wy", Value::Integer((y / 8.0) as i64)),
                ("x", Value::Real(x)),
                ("y", Value::Real(y)),
                ("inMeta", Value::Integer(0)),
                (
                    "worldversion",
                    Value::Integer(i64::from(crate::save::WORLD_VERSION)),
                ),
                ("data", Value::Blob(w.buf.clone())),
            ],
        )
        .unwrap();
    }
    c.set_autocommit(false).unwrap();
    f.vehicles = Some(c);
}

/// A loaded vehicle's row id and square.
type VehicleAt = (i32, Option<(i32, i32, i32)>);

fn vehicle_ids(cell: &IsoCell, wx: i32, wy: i32) -> Vec<VehicleAt> {
    cell.chunks[&(wx, wy)]
        .vehicles
        .loaded()
        .map(|v| match v.origin {
            crate::chunk_vehicles::VehicleOrigin::Database { sql_id } => (sql_id, v.square),
            _ => (-1, v.square),
        })
        .collect()
}

#[test]
fn served_chunks_load_their_vehicles_before_the_missing_squares_exist() {
    let mut f = Fixture::new();
    f.write_chunk(9, 10, 100);
    // Two in the saved chunk (9,10), one in the brand-new chunk (11,11),
    // one outside the grid.
    vehicles_db(
        &mut f,
        &[
            (4, 75.5, 82.5),
            (2, 73.0, 87.9),
            (7, 90.5, 91.0),
            (5, 200.0, 200.0),
        ],
    );
    let mut s = ChunkStreamer::new(1, 3);
    let mut cell = IsoCell::default();
    s.load_world(10, 10, &mut cell, &mut f.ctx());
    let ev = short(&s.take_events());
    // `VehiclesDB2.loadChunk` right after each `LoadChunk`, before the
    // chunk is queued for hand-off.
    let i = ev.iter().position(|e| e == "S 9,10 d").unwrap();
    assert_eq!(ev[i + 1], "V 9,10 2 -0 None");
    assert_eq!(only(&ev, "V ").len(), 9);
    assert!(ev.contains(&"V 11,11 1 -0 None".to_string()));
    assert!(ev.contains(&"V 10,10 0 -0 None".to_string()));
    // Rows come in id order, on the saved squares they are over.
    assert_eq!(
        vehicle_ids(&cell, 9, 10),
        [(2, Some((73, 87, 0))), (4, Some((75, 82, 0)))]
    );
    // A brand-new chunk without lots has no squares yet when its vehicles
    // load (`loadInWorldStreamerThread` makes them later).
    assert_eq!(vehicle_ids(&cell, 11, 11), [(7, None)]);
    assert!(cell.chunks[&(11, 11)].get_grid_square(2, 3, 0).is_some());
    assert!(vehicle_ids(&cell, 10, 10).is_empty());
}

#[test]
fn without_a_vehicle_store_no_vehicles_load() {
    let mut f = Fixture::new();
    f.write_chunk(9, 10, 100);
    vehicles_db(&mut f, &[(4, 75.5, 82.5)]);
    let conn = f.vehicles.take();
    let mut s = ChunkStreamer::new(1, 3);
    let mut cell = IsoCell::default();
    s.load_world(10, 10, &mut cell, &mut f.ctx());
    let ev = short(&s.take_events());
    assert!(only(&ev, "V ").is_empty());
    assert!(vehicle_ids(&cell, 9, 10).is_empty());
    drop(conn);
}

/// Every vehicle has a script without alarm or lightbar attached.
struct PlainScripts;

impl vehicles_db::VehicleScripts for PlainScripts {
    fn script(&self, _: &str) -> Option<vehicles_db::VehicleScriptFacts<'_>> {
        Some(vehicles_db::VehicleScriptFacts {
            sounds: crate::save::vehicle::AlarmSounds {
                alarm_enable: false,
                alarm: &[],
                alarm_loop: &[],
            },
            lightbar_enable: false,
        })
    }
}

/// Loaded vehicles that were never updated in the world: on no square's
/// moving objects, so `removeFromWorld` leaves them to `unloadChunk`.
struct NotInWorld<'a>(vehicles_db::LoadedVehicleSaver<'a>);

impl vehicles_db::ChunkVehicleSaver for NotInWorld<'_> {
    fn vehicle_buffer(
        &self,
        vehicle: &crate::chunk_vehicles::VehicleHandle,
        chunk: &crate::chunk::IsoChunk,
        ctx: &crate::save::SaveCtx<'_>,
    ) -> Result<crate::save::vehicle::VehicleRow, String> {
        self.0.vehicle_buffer(vehicle, chunk, ctx)
    }
}

/// Records `VirtualVehicleManager.addToMeta`.
#[derive(Default)]
struct Meta(Vec<i32>);

impl vehicles_db::VehicleMetaHook for Meta {
    fn add_to_meta(&mut self, vehicle: crate::chunk_vehicles::VehicleHandle) {
        let lv = vehicle
            .downcast::<crate::chunk_vehicles::LoadedVehicle>()
            .unwrap();
        match lv.origin {
            crate::chunk_vehicles::VehicleOrigin::Database { sql_id } => self.0.push(sql_id),
            _ => self.0.push(-1),
        }
    }
}

/// `(id, wx, wy, x, y, worldversion, inMeta, data)` of every row.
type DbRow = (i64, i64, i64, f64, f64, i64, i64, Vec<u8>);

fn db_rows(c: &mut pz_sqlite::Connection) -> Vec<DbRow> {
    let t = c.table("vehicles").unwrap();
    let col = |n: &str| t.column(n).unwrap();
    let real = |v: pz_sqlite::Value| match v {
        pz_sqlite::Value::Real(r) => r,
        v => v.as_i64() as f64,
    };
    c.rows(&t)
        .unwrap()
        .iter()
        .map(|r| {
            let data = match r.get(&t, col("data")) {
                pz_sqlite::Value::Blob(b) => b,
                _ => Vec::new(),
            };
            (
                r.get(&t, col("id")).as_i64(),
                r.get(&t, col("wx")).as_i64(),
                r.get(&t, col("wy")).as_i64(),
                real(r.get(&t, col("x"))),
                real(r.get(&t, col("y"))),
                r.get(&t, col("worldversion")).as_i64(),
                r.get(&t, col("inMeta")).as_i64(),
                data,
            )
        })
        .collect()
}

/// The blob `BaseVehicle.save` writes for the test vehicle on square
/// `(x, y)`.
fn vehicle_blob(f: &Fixture, square: (i32, i32)) -> Vec<u8> {
    vehicle_blob_on(f, Some(square))
}

/// [`vehicle_blob`] for a vehicle on no square (no clamp).
fn vehicle_blob_on(f: &Fixture, square: Option<(i32, i32)>) -> Vec<u8> {
    let mut row = crate::save::vehicle::VehicleRow {
        id: 0,
        wx: 0,
        wy: 0,
        x: 0.0,
        y: 0.0,
        world_version: crate::save::WORLD_VERSION,
        in_meta: false,
        data: Vec::new(),
    };
    let mut w = crate::save::io::Writer::new();
    crate::save::movers_tests::vehicle()
        .save(
            &mut w,
            f.save_tables.class_id("Vehicle"),
            None,
            None,
            &AllItems,
        )
        .unwrap();
    row.data = w.buf;
    let class = f.save_tables.class_id("Vehicle");
    let ts = f.save_settings.tile_scale as f32;
    let v = row
        .load(class, (32.0 * ts, 96.0 * ts), &AllItems)
        .unwrap()
        .unwrap();
    let mut w = crate::save::io::Writer::new();
    v.save(&mut w, class, square, None, &AllItems).unwrap();
    w.buf
}

#[test]
fn unloaded_chunks_write_their_vehicles_before_the_chunk_file() {
    let mut f = Fixture::new();
    f.write_chunk(9, 10, 100);
    vehicles_db(&mut f, &[(4, 75.5, 82.5), (2, 73.0, 87.9)]);
    let mut s = ChunkStreamer::new(1, 3);
    let mut cell = IsoCell::default();
    s.load_world(10, 10, &mut cell, &mut f.ctx());
    s.take_events();
    // Row 4 is gone from the table: it is inserted again.
    {
        let c = f.vehicles.as_mut().unwrap();
        c.delete("vehicles", 4).unwrap();
        c.commit().unwrap();
    }
    let p = PlayerPos::moved(87.0, 84.0, 88.0, 84.0);
    let scripts = PlainScripts;
    let saver = NotInWorld(vehicles_db::LoadedVehicleSaver { scripts: &scripts });
    let mut ctx = f.ctx();
    ctx.vehicle_saver = Some(&saver);
    s.update(&[Some(p)], Serve::UntilIdle, &mut cell, &mut ctx);
    let ev = short(&s.take_events());
    assert!(only(&ev, "VM ").is_empty());
    let i = ev.iter().position(|e| e == "W 9,10").unwrap();
    assert_eq!(ev[i - 1], "VU 9,10 [2, 4] []");
    assert!(ev.contains(&"VU 9,9 [] []".to_string()));
    let blob2 = vehicle_blob(&f, (73, 87));
    let blob4 = vehicle_blob(&f, (75, 82));
    // The row's position is the vehicle's own (`getX()`, not the row it
    // came from); the blob clamps it into the square. The test vehicle's
    // engine runs, so it is `inMeta`.
    let rows = db_rows(f.vehicles.as_mut().unwrap());
    assert_eq!(
        rows,
        [
            (2, 9, 10, 100.25, 200.75, 249, 1, blob2),
            (4, 9, 10, 100.25, 200.75, 249, 1, blob4),
        ]
    );
}

#[test]
fn vehicles_without_a_script_are_not_written() {
    let mut f = Fixture::new();
    f.write_chunk(9, 10, 100);
    vehicles_db(&mut f, &[(4, 75.5, 82.5)]);
    let before = db_rows(f.vehicles.as_mut().unwrap());
    let mut s = ChunkStreamer::new(1, 3);
    let mut cell = IsoCell::default();
    s.load_world(10, 10, &mut cell, &mut f.ctx());
    let p = PlayerPos::moved(87.0, 84.0, 88.0, 84.0);
    s.update(&[Some(p)], Serve::UntilIdle, &mut cell, &mut f.ctx());
    let ev = short(&s.take_events());
    // `shouldUpdateInMeta` → `hasAlarm` on the null script.
    assert!(ev.contains(&"VU 9,10 [] [\"java.lang.NullPointerException\"]".to_string()));
    assert_eq!(f.saved_mark(9, 10), Some(100));
    assert_eq!(db_rows(f.vehicles.as_mut().unwrap()), before);
}

#[test]
fn saving_the_cell_writes_every_grid_chunk_and_stop_removes_them() {
    let mut f = Fixture::new();
    f.write_chunk(9, 10, 100);
    vehicles_db(&mut f, &[(4, 75.5, 82.5)]);
    let mut s = ChunkStreamer::new(1, 3);
    let mut cell = IsoCell::default();
    s.load_world(10, 10, &mut cell, &mut f.ctx());
    // One chunk unloaded and still queued when the game is saved.
    let p = PlayerPos::moved(87.0, 84.0, 88.0, 84.0);
    s.update(&[Some(p)], Serve::Jobs(0), &mut cell, &mut f.ctx());
    s.take_events();
    assert_eq!(s.pending_saves(), 3);
    cell.chunks
        .get_mut(&(10, 10))
        .unwrap()
        .saved
        .loot_respawn_hour = 55;
    let scripts = PlainScripts;
    let saver = vehicles_db::LoadedVehicleSaver { scripts: &scripts };
    {
        let mut ctx = f.ctx();
        ctx.vehicle_saver = Some(&saver);
        s.wait_for_saves(&mut ctx);
        s.save_chunk_maps(&cell, &mut ctx);
    }
    let ev = short(&s.take_events());
    assert_eq!(
        only(&ev, "W "),
        [
            // The queued saves, then the hot save; then the grid, x outer.
            "9,9", "9,10", "9,11", "10,9", "10,10", "10,11", "11,9", "11,10", "11,11"
        ]
    );
    assert_eq!(
        ev[ev.iter().position(|e| e == "W 9,11").unwrap() + 1],
        "HOT"
    );
    assert_eq!(ev.iter().filter(|e| *e == "HOT").count(), 1);
    // The grid has moved east: its last column holds no chunks yet.
    assert_eq!(grid(&s, 0)[0], [Some((10, 9)), Some((11, 9)), None]);
    assert_eq!(f.saved_mark(10, 10), Some(55));
    assert!(chunk_path(&f.dir, 11, 11).exists());
    assert!(cell.chunks.contains_key(&(10, 10)));

    // Quitting: nothing is saved again; every chunk leaves the world.
    std::fs::remove_file(chunk_path(&f.dir, 10, 10)).unwrap();
    s.stop(&mut cell, &mut f.ctx());
    let ev = short(&s.take_events());
    assert_eq!(
        only(&ev, "X "),
        ["10,9", "11,9", "10,10", "11,10", "10,11", "11,11"]
    );
    assert!(only(&ev, "W ").is_empty());
    assert!(cell.chunks.is_empty());
    assert!(!chunk_path(&f.dir, 10, 10).exists());
    assert!(!s.is_busy());
}

#[test]
fn running_vehicles_leave_with_their_chunk_for_the_meta() {
    let mut f = Fixture::new();
    f.write_chunk(9, 10, 100);
    vehicles_db(&mut f, &[(4, 75.5, 82.5), (2, 73.0, 87.9)]);
    let mut s = ChunkStreamer::new(1, 3);
    let mut cell = IsoCell::default();
    s.load_world(10, 10, &mut cell, &mut f.ctx());
    s.take_events();
    let p = PlayerPos::moved(87.0, 84.0, 88.0, 84.0);
    let scripts = PlainScripts;
    let saver = vehicles_db::LoadedVehicleSaver { scripts: &scripts };
    let mut meta = Meta::default();
    let mut ids = vehicles_db::VehicleIds::init(f.vehicles.as_mut().unwrap()).unwrap();
    assert_eq!(ids.used.iter().copied().collect::<Vec<_>>(), [2, 4]);
    ids.set_vehicle_loaded(2);
    ids.set_vehicle_loaded(4);
    {
        let mut ctx = f.ctx();
        ctx.vehicle_saver = Some(&saver);
        ctx.vehicle_meta = Some(&mut meta);
        ctx.vehicle_ids = Some(&mut ids);
        s.update(&[Some(p)], Serve::UntilIdle, &mut cell, &mut ctx);
    }
    // `BaseVehicle.removeFromWorld` unloaded both ids.
    assert!(ids.loaded.is_empty());
    assert_eq!(ids.allocate_id(), 1);
    assert_eq!(ids.set_vehicle_loaded(-1), 3);
    let ev = short(&s.take_events());
    // The test vehicle's engine runs: both leave the chunk at
    // `removeFromWorld`, square by square (row 4 stands on square 3 + 2 * 8,
    // row 2 on 1 + 7 * 8), so the chunk save has none left to write.
    let u = ev.iter().position(|e| e == "U 9,10").unwrap();
    assert_eq!(ev[u + 1], "VM 9,10 [4, 2] []");
    let w = ev.iter().position(|e| e == "W 9,10").unwrap();
    assert_eq!(ev[w - 1], "VU 9,10 [] []");
    // The streamer writes the queued rows once it has nothing else to do.
    let q = ev.iter().position(|e| e.starts_with("VQ")).unwrap();
    assert_eq!(ev[q], "VQ [4, 2] []");
    assert!(q > w && ev[q + 1..].iter().all(|e| e.starts_with("H ")));
    assert_eq!(only(&ev, "VQ").len(), 1);
    assert_eq!(meta.0, [4, 2]);
    // Off its square the blob is not clamped.
    let blob = vehicle_blob_on(&f, None);
    let rows = db_rows(f.vehicles.as_mut().unwrap());
    assert_eq!(
        rows,
        [
            (2, 9, 10, 100.25, 200.75, 249, 1, blob.clone()),
            (4, 9, 10, 100.25, 200.75, 249, 1, blob),
        ]
    );
}

#[test]
fn quitting_queues_the_running_vehicles_for_the_store_reset() {
    let mut f = Fixture::new();
    f.write_chunk(10, 10, 100);
    vehicles_db(&mut f, &[(4, 83.5, 82.5)]);
    let mut s = ChunkStreamer::new(1, 3);
    let mut cell = IsoCell::default();
    s.load_world(10, 10, &mut cell, &mut f.ctx());
    s.take_events();
    let scripts = PlainScripts;
    let saver = vehicles_db::LoadedVehicleSaver { scripts: &scripts };
    let before = db_rows(f.vehicles.as_mut().unwrap());
    {
        let mut ctx = f.ctx();
        ctx.vehicle_saver = Some(&saver);
        s.stop(&mut cell, &mut ctx);
    }
    let ev = short(&s.take_events());
    let x = ev.iter().position(|e| e == "X 10,10").unwrap();
    assert_eq!(ev[x + 1], "VM 10,10 [4] []");
    // Nothing is written until `VehiclesDB2.Reset`.
    assert!(only(&ev, "VQ").is_empty());
    assert_eq!(db_rows(f.vehicles.as_mut().unwrap()), before);
    let mut q = s.take_vehicle_queue();
    assert_eq!(q.len(), 1);
    let r = vehicles_db::update_world_streamer(f.vehicles.as_mut(), &mut q);
    assert_eq!(r.saved, [4]);
    let rows = db_rows(f.vehicles.as_mut().unwrap());
    assert_eq!(rows[0].6, 1);
    assert_eq!(rows[0].7, vehicle_blob_on(&f, None));
}

#[test]
fn vehicles_without_a_script_stay_in_the_chunk() {
    let mut f = Fixture::new();
    f.write_chunk(9, 10, 100);
    vehicles_db(&mut f, &[(4, 75.5, 82.5)]);
    let mut s = ChunkStreamer::new(1, 3);
    let mut cell = IsoCell::default();
    s.load_world(10, 10, &mut cell, &mut f.ctx());
    let p = PlayerPos::moved(87.0, 84.0, 88.0, 84.0);
    s.update(&[Some(p)], Serve::UntilIdle, &mut cell, &mut f.ctx());
    let ev = short(&s.take_events());
    assert!(ev.contains(&"VM 9,10 [] [\"java.lang.NullPointerException\"]".to_string()));
    assert!(only(&ev, "VQ").is_empty());
}
