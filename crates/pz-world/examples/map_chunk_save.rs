//! Load chunks from map lots with the full object creation path and save
//! each with the chunk save port, for checking that the original game loads
//! (and re-saves) them unchanged on a machine with the game installed.
//! Results are recorded as notes only.
//!
//! ```text
//! map_chunk_save <game dir> <fs root> <map> <sprite tables> <object tables>
//!                <seed> <locked houses> <save tables> <out dir>
//!                <wx0> <wy0> <wx1> <wy1>
//! ```
//!
//! Writes `<out dir>/map/<wx>/<wy>.bin`. Map loading is set up as in
//! `objects_digest`.

use pz_fs::GameFs;
use pz_world::chunk::{ChunkLoadEnv, IsoCell};
use pz_world::fix2x::Fix2xTable;
use pz_world::{
    CreateParams, GameRand, IsoMetaGrid, IsoSpriteManager, ObjectLoadHooks, ObjectLoadSettings,
    ObjectTables, SpriteTables, WorldGenBounds,
};

/// `java.util.Random`.
struct JavaRandom(u64);

impl JavaRandom {
    fn new(seed: i64) -> Self {
        JavaRandom((seed as u64 ^ 0x5DEECE66D) & ((1 << 48) - 1))
    }
    fn bits(&mut self, bits: u32) -> i32 {
        self.0 = (self.0.wrapping_mul(0x5DEECE66D).wrapping_add(0xB)) & ((1 << 48) - 1);
        (self.0 >> (48 - bits)) as i64 as i32
    }
}

impl GameRand for JavaRandom {
    fn next(&mut self, bound: i32) -> i32 {
        if bound <= 0 {
            return 0;
        }
        let mut r = self.bits(31);
        let m = bound - 1;
        if bound & m == 0 {
            return ((bound as i64 * r as i64) >> 31) as i32;
        }
        let mut u = r;
        loop {
            r = u % bound;
            if u.wrapping_sub(r).wrapping_add(m) >= 0 {
                return r;
            }
            u = self.bits(31);
        }
    }
    fn next_float(&mut self) -> f32 {
        self.bits(24) as f32 / (1 << 24) as f32
    }
}

/// A fresh world with no zones or generators: the power grid is on except
/// in derelict and user-defined rooms (`IsoGridSquare.isNoPower`).
struct GridOn;
impl ObjectLoadHooks for GridOn {
    fn square_powered(&mut self, _sq: &pz_world::IsoGridSquare, room: &pz_world::RoomDef) -> bool {
        !room.name.to_lowercase().contains("derelict") && !room.user_defined
    }
}

fn grid(fs_root: &str, map: &str) -> IsoMetaGrid {
    IsoMetaGrid::create(
        &GameFs::new(fs_root),
        CreateParams {
            game_map: map,
            world_gen: WorldGenBounds::default(),
            rand: &mut JavaRandom::new(0),
            voronois: Vec::new(),
        },
    )
    .expect("create")
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let game = std::path::Path::new(&a[1]);
    let mut g = grid(&a[2], &a[3]);
    g.assign_alarms(&mut JavaRandom::new(0), false, false, 1, || 0);
    for r in &g.room_defs {
        r.lights_active.set((r.x + r.y + r.level) & 1 == 0);
    }
    let tables = SpriteTables::from_text(&std::fs::read_to_string(&a[4]).unwrap()).unwrap();
    let objects = ObjectTables::from_text(&std::fs::read_to_string(&a[5]).unwrap()).unwrap();
    let mut sprites =
        IsoSpriteManager::load_tile_definitions_from(tables, &game.join("media"), &[]).unwrap();
    sprites.loaded_tile_definitions(&|_| false).unwrap();
    let mut rand = JavaRandom::new(a[6].parse().unwrap());
    let settings = ObjectLoadSettings {
        locked_houses: a[7].parse().unwrap(),
        ..Default::default()
    };
    let fix2x = Fix2xTable::default();
    let save_tables =
        pz_world::save::SaveTables::from_text(&std::fs::read_to_string(&a[8]).unwrap()).unwrap();
    let out_dir = std::path::Path::new(&a[9]);
    let (wx0, wy0, wx1, wy1): (i32, i32, i32, i32) = (
        a[10].parse().unwrap(),
        a[11].parse().unwrap(),
        a[12].parse().unwrap(),
        a[13].parse().unwrap(),
    );
    let mut cell = IsoCell::default();
    let mut counts = std::collections::BTreeMap::<&str, usize>::new();
    for wx in wx0..=wx1 {
        for wy in wy0..=wy1 {
            let mut env = ChunkLoadEnv {
                sprites: &mut sprites,
                fix2x: &fix2x,
                rand: &mut rand,
                tables: &objects,
                settings,
                hooks: &mut GridOn,
            };
            cell.load_chunk(&g, wx, wy, &mut env);
            let ch = cell.chunks.remove(&(wx, wy)).unwrap();
            let settings = pz_world::save::SaveSettings::default();
            let ctx = pz_world::save::SaveCtx {
                sprites: &sprites,
                tables: &save_tables,
                settings: &settings,
                items: &pz_world::save::items::AllItems,
                hooks: &pz_world::save::NoSaveHooks,
                meta: None,
            };
            match pz_world::save::chunk_file::save_chunk(&ch, &ctx) {
                Ok(bytes) => {
                    let p = pz_world::save::chunk_file::chunk_path(out_dir, wx, wy);
                    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
                    std::fs::write(p, bytes).unwrap();
                    *counts.entry("saved").or_default() += 1;
                }
                Err(e) => {
                    eprintln!("chunk {wx},{wy}: {e}");
                    *counts.entry("save errors").or_default() += 1;
                }
            }
        }
    }
    println!("{counts:?}");
}
