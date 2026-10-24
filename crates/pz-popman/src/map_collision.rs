//! `zombie.MapCollisionData`: the Java side of the native collision grid.
//! It hands the native library the meta grid (map `chunkdata` files and
//! per-chunk zombie intensity), the game-state strings, each loaded chunk's
//! square bits and single-square changes, and drives the worker thread's
//! update/save/stop sequence.

use crate::native::Native;
use pz_world::map_files::zombie_intensity_for_chunk;
use pz_world::IsoMetaGrid;
use std::path::PathBuf;

pub use crate::native::mcd::{BIT_ROOM, BIT_SOLID, BIT_WALLN, BIT_WALLW, BIT_WATER};

/// `SQUARE_UPDATE_SIZE`: bytes per queued square change.
pub const SQUARE_UPDATE_SIZE: usize = 9;

/// What `MapCollisionData.init` passes for one cell.
#[derive(Clone, Debug)]
pub struct MetaCellInit {
    pub cx: i32,
    pub cy: i32,
    /// `infoFileNames.get("chunkdata_X_Y.bin")` of every map, in map order.
    pub chunkdata: Vec<Option<PathBuf>>,
    /// `max(LotHeader.getZombieIntensityForChunk(...), 0)` per chunk
    /// (`wx + wy * 32`), when the meta cell exists.
    pub intensity: Option<Vec<u8>>,
}

/// The meta-grid part of `MapCollisionData.init(metaGrid)`.
#[derive(Clone, Debug)]
pub struct MetaInit {
    pub min_x: i32,
    pub min_y: i32,
    pub width: i32,
    pub height: i32,
    pub cells: Vec<MetaCellInit>,
}

/// Collects the `n_initMetaGrid/Cell/Chunk` arguments from a meta grid.
pub fn meta_init(grid: &IsoMetaGrid) -> MetaInit {
    let mut cells = Vec::new();
    for cy in grid.min_y..grid.min_y + grid.height {
        for cx in grid.min_x..grid.min_x + grid.width {
            let name = format!("chunkdata_{cx}_{cy}.bin");
            let chunkdata = grid.map_files.iter().map(|mf| mf.info_file_names.get(&name).cloned()).collect();
            let intensity = grid.get_cell_data(cx, cy).map(|cell| {
                let header = cell.info.as_ref().and_then(|(mf, n)| grid.map_files[*mf].info_headers.get(n));
                let mut v = vec![0u8; 1024];
                for wy in 0..32 {
                    for wx in 0..32 {
                        let i = zombie_intensity_for_chunk(&grid.map_files, header, wx, wy);
                        v[(wx + wy * 32) as usize] = i.max(0) as u8;
                    }
                }
                v
            });
            cells.push(MetaCellInit { cx, cy, chunkdata, intensity });
        }
    }
    MetaInit { min_x: grid.min_x, min_y: grid.min_y, width: grid.width, height: grid.height, cells }
}

/// The game state `MapCollisionData.init` sends before the meta grid.
#[derive(Clone, Debug, Default)]
pub struct McdGameState {
    pub game_mode: String,
    pub game_save_world: String,
    pub last_stand: bool,
    pub no_save: bool,
    pub cache_dir: String,
    /// `getGameModeCacheDir() + File.separator`.
    pub game_mode_cache_dir: String,
    pub save_dir: String,
    pub distribution: i32,
    pub zombies: i32,
    pub zombies_disabled: bool,
}

/// One square's `BIT_*` flags, from what the square holds
/// (`isSolid`/`isBlockedN`/`isBlockedW`/`isWater`/`isRoom`).
#[derive(Clone, Copy, Debug, Default)]
pub struct SquareFlags {
    /// `isSolid() || isSolidTrans()`.
    pub solid: bool,
    pub has_stairs: bool,
    pub water: bool,
    pub tree: bool,
    pub hoppable_n: bool,
    /// `collideN`, `doorFrN`, `DoorWallN`, `windowFN`, `windowN`, `WindowN`.
    pub blocks_n: bool,
    pub hoppable_w: bool,
    pub blocks_w: bool,
    pub room: bool,
}

impl SquareFlags {
    /// The byte `addChunkToWorld` / `squareChanged` send.
    pub fn bits(&self) -> u8 {
        let mut solid = self.solid;
        if self.has_stairs {
            solid = true;
        }
        if self.water || self.tree {
            solid = false;
        }
        let mut b = 0;
        if solid {
            b |= BIT_SOLID;
        }
        if !self.hoppable_n && self.blocks_n {
            b |= BIT_WALLN;
        }
        if !self.hoppable_w && self.blocks_w {
            b |= BIT_WALLW;
        }
        if self.water {
            b |= BIT_WATER;
        }
        if self.room {
            b |= BIT_ROOM;
        }
        b
    }
}

/// `zombie.MapCollisionData` (single player: the worker thread's steps are
/// run by whoever drives it; see [`MapCollisionData::thread_step`]).
#[derive(Default)]
pub struct MapCollisionData {
    pub client: bool,
    pub paused: bool,
    pub no_save: bool,
    /// Square changes not yet handed over (`squareUpdateBuffer`).
    pending_squares: Vec<(i32, i32, u8)>,
    last_update: i64,
    /// Whether the worker thread has started (`thread.isAlive()`).
    pub thread_alive: bool,
}

impl MapCollisionData {
    /// `init(metaGrid)`.
    pub fn init(&mut self, n: &mut Native, gs: &McdGameState, meta: &MetaInit, client: bool) {
        self.client = client;
        if client {
            return;
        }
        let _ = n.mcd_set_game_state_string("Core.GameMode", &gs.game_mode);
        let _ = n.mcd_set_game_state_string("Core.GameSaveWorld", &gs.game_save_world);
        let _ = n.mcd_set_game_state_bool("Core.bLastStand", gs.last_stand);
        self.no_save = gs.no_save;
        let _ = n.mcd_set_game_state_bool("Core.noSave", gs.no_save);
        let _ = n.mcd_set_game_state_string("GameWindow.CacheDir", &gs.cache_dir);
        let _ = n.mcd_set_game_state_string("GameWindow.GameModeCacheDir", &gs.game_mode_cache_dir);
        let _ = n.mcd_set_game_state_string("GameWindow.SaveDir", &gs.save_dir);
        let _ = n.mcd_set_game_state_int("SandboxOptions.Distribution", gs.distribution);
        let _ = n.mcd_set_game_state_int("SandboxOptions.Zombies", gs.zombies);
        let _ = n.mcd_set_game_state_string("SavefileNaming.SUBDIR_CHUNKDATA", "chunkdata");
        let _ = n.mcd_set_game_state_string("SavefileNaming.SUBDIR_ZPOP", "zpop");
        let _ = n.mcd_set_game_state_bool("World.ZombiesDisabled", gs.zombies_disabled);
        self.paused = true;
        let _ = n.mcd_set_game_state_bool("PAUSED", true);
        n.mcd_init_meta_grid(meta.min_x, meta.min_y, meta.width, meta.height);
        for c in &meta.cells {
            for f in &c.chunkdata {
                n.mcd_init_meta_cell(c.cx, c.cy, f.clone());
            }
            if let Some(v) = &c.intensity {
                for wy in 0..32 {
                    for wx in 0..32 {
                        n.mcd_init_meta_chunk(c.cx, c.cy, wx, wy, v[(wx + wy * 32) as usize]);
                    }
                }
            }
        }
        n.mcd_init(meta.min_x, meta.min_y, meta.width, meta.height);
    }

    /// `addChunkToWorld`: `bits[x + y * 8]` of the chunk's level-0 squares
    /// (`BIT_SOLID` for missing squares).
    pub fn add_chunk_to_world(&mut self, n: &mut Native, wx: i32, wy: i32, bits: &[u8; 64]) {
        if self.client {
            return;
        }
        let _ = n.mcd_chunk_update_task(wx, wy, bits);
    }

    /// `squareChanged`.
    pub fn square_changed(&mut self, n: &mut Native, x: i32, y: i32, flags: SquareFlags) {
        if self.client {
            return;
        }
        self.pending_squares.push((x, y, flags.bits()));
        if (1024 - self.pending_squares.len() * SQUARE_UPDATE_SIZE) < SQUARE_UPDATE_SIZE {
            self.flush_squares(n);
        }
    }

    fn flush_squares(&mut self, n: &mut Native) {
        for (x, y, b) in self.pending_squares.drain(..) {
            n.mcd_square_update_task(x, y, b);
        }
    }

    /// `hasDataForThread`.
    pub fn has_data_for_thread(&mut self, n: &mut Native) -> bool {
        if !self.pending_squares.is_empty() {
            self.flush_squares(n);
        }
        n.mcd_has_data_for_thread()
    }

    /// `updateGameState(noSave, paused)`.
    pub fn update_game_state(&mut self, n: &mut Native, no_save: bool, paused: bool) {
        if self.no_save != no_save {
            self.no_save = no_save;
            let _ = n.mcd_set_game_state_bool("Core.noSave", no_save);
        }
        if paused != self.paused {
            self.paused = paused;
            let _ = n.mcd_set_game_state_bool("PAUSED", paused);
        }
    }

    /// `updateMain`: returns whether the worker should be woken.
    pub fn update_main(&mut self, now_ms: i64) -> bool {
        if self.client {
            return false;
        }
        if now_ms - self.last_update > 10_000 {
            self.last_update = now_ms;
            return true;
        }
        false
    }

    /// One pass of `MCDThread.runInner` (no path tasks are queued from
    /// Java in single player): `n_update`, the population's
    /// `updateThread` and pending cell saves are run by the caller with
    /// [`crate::ZombiePopulationManager::thread_step`].
    pub fn thread_update(&mut self, n: &mut Native) {
        n.mcd_n_update();
    }

    /// `save()` when the thread is not running.
    pub fn save(&mut self, n: &mut Native) {
        if self.client {
            return;
        }
        n.mcd_save();
    }

    /// `stop()`.
    pub fn stop(&mut self, n: &mut Native) {
        if self.client {
            return;
        }
        self.thread_alive = false;
        n.mcd_stop();
        self.pending_squares.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn square_bits() {
        let f = SquareFlags { solid: true, tree: true, blocks_n: true, room: true, ..Default::default() };
        assert_eq!(f.bits(), BIT_WALLN | BIT_ROOM);
        let f = SquareFlags { has_stairs: true, blocks_w: true, hoppable_w: true, ..Default::default() };
        assert_eq!(f.bits(), BIT_SOLID);
        let f = SquareFlags { water: true, solid: true, ..Default::default() };
        assert_eq!(f.bits(), BIT_WATER);
    }
}
