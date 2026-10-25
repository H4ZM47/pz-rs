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
/// discarded, `U` unloaded, `W x,y` saved (`-`: saving is off).
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
            "W 8,8", "S 8,8 d", "H 8,10", "H 8,11", "H 8,9", "H 8,12",
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
        ["U 10,10", "R 11,10", "W 10,10 -", "S 11,10 n", "H 11,10"]
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
