//! Saving a single-player game: `GameWindow.save(true)` with
//! `IsoCell.save`, the quit sequence of `IngameState` (`Core.exiting`, then
//! `IngameState.exit`), and the main-thread part of the chunk save worker's
//! hot save (`ChunkSaveWorker.HotsaveAncilliarySystems`).
//!
//! The files this module writes itself, in the original's order:
//!
//! | step | file |
//! |---|---|
//! | `GameWindow.save` | `map_ver.bin`, `map_sand.bin` |
//! | `IsoCell.save` | chunk files and `vehicles.db` rows (through [`SaveGameHooks::save_chunks`]), `map.bin`, `map_t.bin`, `map_meta.bin`, `map_zone.bin`, `reanimated.bin` |
//! | `SGlobalObjects.save` | `gos_*.bin` |
//!
//! The systems this crate cannot reach (the chunk streamer's borrows, Lua
//! events, the zombie and animal populations, the radio and the world map)
//! run through [`SaveGameHooks`] at their place in the order.
//!
//! The multiplayer branches (`GameClient.clientSave`, `loadedAsClient`) are
//! not ported: this is the local game's save.

use std::io;
use std::path::Path;

use pz_core::core::Core;
use pz_core::game_time::GameTime;
use pz_core::game_time_save;
use pz_kahlua::java_io::JavaWrite;
use pz_world::meta_save::{save_map_meta, MetaWorld, NetRole};
use pz_world::zone_save::{SpawnedZombieZones, UuidSource};
use pz_world::IsoMetaGrid;

use crate::global_objects_save;
use crate::player_db::{PlayerData, PlayerDb, VehicleStore};

/// `IsoWorld.getWorldVersion()` in this build.
pub const WORLD_VERSION: i32 = 249;

/// `new IsoCell(256, 256)` (`CellLoader.LoadCellBinaryChunk`) and
/// `IsoCell.maxHeight`: the three ints of `map.bin`.
pub const CELL_SIZE: (i32, i32, i32) = (256, 256, 32);

/// `map_ver.bin`: `writeInt(249)` then `GameWindow.WriteString(Core.gameMap)`
/// (an int length and the UTF-16 code units; `null` is a zero length).
pub fn map_ver_bytes(game_map: Option<&str>) -> Vec<u8> {
    let mut out = Vec::new();
    out.put_i32(WORLD_VERSION);
    out.write_string_chars(game_map);
    out
}

/// `map.bin`: `IsoCell.width`, `height` and `maxHeight`.
pub fn map_bin_bytes(size: (i32, i32, i32)) -> Vec<u8> {
    let mut out = Vec::new();
    out.put_i32(size.0);
    out.put_i32(size.1);
    out.put_i32(size.2);
    out
}

/// `ReanimatedPlayers.saveReanimatedPlayers`: the world version, the count
/// and each zombie's `IsoZombie.save` bytes (the reanimated players the
/// manager tracks, then those in the cell's zombie list that are not dead
/// and not tracked yet).
pub fn reanimated_bytes(zombies: &[Vec<u8>]) -> Vec<u8> {
    let mut out = Vec::new();
    out.put_i32(WORLD_VERSION);
    out.put_i32(zombies.len() as i32);
    for z in zombies {
        out.extend_from_slice(z);
    }
    out
}

/// The world state [`save_game`] saves.
pub struct SaveGame<'a> {
    /// The current save folder (`ZomboidFileSystem.getCurrentSaveDir`).
    pub save_dir: &'a Path,
    /// `Core.getInstance().isNoSave()`: nothing is saved.
    pub no_save: bool,
    /// `IsoWorld.instance.currentCell != null`.
    pub has_cell: bool,
    /// `Core.getInstance().getGameMode()`: `LastStand` and `Tutorial` are
    /// not saved.
    pub game_mode: &'a str,
    /// `Core.gameMap` (the `;`-separated map folders).
    pub game_map: Option<&'a str>,
    /// `SandboxOptions.instance.save(..)`: the bytes of `map_sand.bin`
    /// (`pz_sandbox::SandboxOptions::save_bin`).
    pub sandbox: &'a [u8],
    pub game_time: &'a GameTime,
    /// For the berry and mushroom `GameTime.save` writes.
    pub core: &'a Core,
    pub meta: &'a mut IsoMetaGrid,
    /// The world-wide part of `map_meta.bin`.
    pub meta_world: &'a MetaWorld,
    /// `IsoMetaGrid.spawnedZombieZone` for `map_zone.bin`.
    pub spawned_zones: &'a SpawnedZombieZones,
    /// Ids for zones that have none yet (their constructor's UUID).
    pub uuids: &'a mut dyn UuidSource,
    /// `PlayerDB.getInstance()` (`PlayerDB.isAllow()`).
    pub players: Option<&'a mut PlayerDb>,
    /// `PlayerDB.canSavePlayers`.
    pub can_save_players: bool,
    /// The `IsoZombie.save` bytes of the reanimated players
    /// ([`reanimated_bytes`]).
    pub reanimated: &'a [Vec<u8>],
}

/// The steps of the save that belong to systems outside this crate, called
/// at their place in `GameWindow.save`.
pub trait SaveGameHooks {
    /// After `map_sand.bin`: `WorldGenParams.save`, `InstanceTracker.save`,
    /// `MetaTracker.save`, `StatisticsManager.save`, then
    /// `LuaEventManager.triggerEvent("OnSave")` and the thumbnail
    /// (`SavefileThumbnail.create`, on the game thread).
    fn on_save(&mut self) {}
    /// `IsoCell.save` before `map.bin` gets its header: wait while the
    /// chunk save worker is saving
    /// (`pz_world::streamer::ChunkStreamer::wait_for_saves`), then
    /// `chunkMap[n].Save()` for every player
    /// (`ChunkStreamer::save_chunk_maps`), which writes the chunk files and
    /// their vehicles (`VehiclesDB2.unloadChunk`). `meta` is the grid the
    /// streamer context borrows.
    fn save_chunks(&mut self, meta: &IsoMetaGrid);
    /// `IsoMetaGrid.save` after `map_zone.bin`: `map_animals.bin`
    /// (`saveAnimalZones`) and `metagrid/metacell_*.bin` (the cells queued
    /// with `addCellToSave`).
    fn save_meta_extra(&mut self) {}
    /// `AnimalPopulationManager.getInstance().save()`.
    fn save_animals(&mut self) {}
    /// `MapCollisionData.instance.save()`: `ZombiePopulationManager`
    /// updates its real zombies and saves (`WorldPopulation::save`).
    fn save_population(&mut self) {}
    /// After `gos_*.bin`: `ZomboidRadio.Save`, `GlobalModData.save`,
    /// `MapItem.SaveWorldMap`, `WorldMapVisited.SaveAll`,
    /// `FishSchoolManager.save`, `GameEntityManager.Save` and
    /// `VirtualVehicleManager.save`.
    fn save_other_systems(&mut self) {}
    /// `LuaEventManager.triggerEvent("OnPostSave")` (quitting only).
    fn on_post_save(&mut self) {}
}

/// What [`save_game`] did besides writing files.
#[derive(Debug, Default, PartialEq)]
pub struct SaveReport {
    /// Whether anything was saved (`isNoSave`, no cell, `LastStand` and
    /// `Tutorial` save nothing).
    pub saved: bool,
    /// `PlayerDB.savePlayers()` set `forceSavePlayers`: the next
    /// `PlayerDB.updateMain` saves the local players.
    pub force_save_players: bool,
    /// The exceptions the original logs and goes on after.
    pub errors: Vec<String>,
}

fn write(dir: &Path, name: &str, bytes: &[u8]) -> io::Result<()> {
    std::fs::write(dir.join(name), bytes)
}

/// `GameWindow.save(true)` in a local game. `map_ver.bin` and
/// `map_sand.bin` failures end it (`IOException`); the rest is logged in
/// [`SaveReport::errors`] as the original logs it.
pub fn save_game(g: &mut SaveGame<'_>, hooks: &mut dyn SaveGameHooks) -> io::Result<SaveReport> {
    let mut report = SaveReport::default();
    if g.no_save || !g.has_cell || g.game_mode == "LastStand" || g.game_mode == "Tutorial" {
        return Ok(report);
    }
    report.saved = true;
    let dir = g.save_dir;
    write(dir, "map_ver.bin", &map_ver_bytes(g.game_map))?;
    write(dir, "map_sand.bin", g.sandbox)?;
    hooks.on_save();
    // `map.bin` is opened here; `IsoCell.save` writes its header after the
    // chunks.
    hooks.save_chunks(g.meta);
    let log = |r: &mut SaveReport, what: &str, e: io::Result<()>| {
        if let Err(e) = e {
            eprintln!("ERROR: {what}: {e}");
            r.errors.push(format!("{what}: {e}"));
        }
    };
    let header = write(dir, "map.bin", &map_bin_bytes(CELL_SIZE));
    let header_ok = header.is_ok();
    log(&mut report, "map.bin", header);
    // An exception in `IsoCell.save` skips the rest of it.
    if header_ok {
        let gt = game_time_save::save_map_t_file(g.game_time, g.core, dir);
        let gt_ok = gt.is_ok();
        log(&mut report, "map_t.bin", gt);
        if gt_ok {
            save_meta_grid(g, &mut report, hooks);
            if g.players.is_some() && g.can_save_players {
                report.force_save_players = true;
            }
            let r = write(dir, "reanimated.bin", &reanimated_bytes(g.reanimated));
            log(&mut report, "reanimated.bin", r);
        }
    }
    hooks.save_animals();
    hooks.save_population();
    let r = global_objects_save::save_all(dir, g.no_save, false);
    log(&mut report, "SGlobalObjects.save", r);
    hooks.save_other_systems();
    Ok(report)
}

/// `IsoMetaGrid.save()`: `map_meta.bin`, `map_zone.bin`, then the parts the
/// hooks write; the first failure ends it (logged).
fn save_meta_grid(g: &mut SaveGame<'_>, report: &mut SaveReport, hooks: &mut dyn SaveGameHooks) {
    let dir = g.save_dir;
    let meta = save_map_meta(g.meta, g.meta_world, NetRole::SinglePlayer);
    let r = write(dir, "map_meta.bin", &meta)
        .and_then(|_| g.meta.save_zones_file(dir, g.spawned_zones, &mut *g.uuids));
    if let Err(e) = r {
        eprintln!("ERROR: IsoMetaGrid.save: {e}");
        report.errors.push(format!("IsoMetaGrid.save: {e}"));
        return;
    }
    hooks.save_meta_extra();
}

/// The `Core.exiting` branch of `IngameState.updateInternal` (local game):
/// `PlayerDB.saveLocalPlayersForce()` for `local_players` (each one's
/// `IsoPlayer.save` bytes in [`PlayerData::bytes`]; a player without an
/// id gets one), `canSavePlayers = false`, [`save_game`] (a failure is
/// logged and returned in the report), then `OnPostSave`.
///
/// `IngameState.exit` follows: the engine stops the streamer
/// (`pz_world::streamer::ChunkStreamer::stop`: `ChunkSaveWorker.SaveNow`
/// and every chunk's `removeFromWorld`), then calls [`close_stores`].
pub fn save_on_quit(
    g: &mut SaveGame<'_>,
    local_players: &mut [PlayerData],
    hooks: &mut dyn SaveGameHooks,
) -> SaveReport {
    if let Some(db) = g.players.as_deref_mut() {
        // `savePlayersAsync` queues them; the streamer thread stores them
        // (`updateWorldStreamer`) on its next loop, or at once when it has
        // stopped. Each player is its own row, so the order does not show.
        for p in local_players.iter_mut() {
            db.save_player(p);
        }
        g.can_save_players = false;
    }
    let report = match save_game(g, hooks) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("ERROR: GameWindow.save: {e}");
            SaveReport {
                saved: true,
                errors: vec![format!("GameWindow.save: {e}")],
                ..Default::default()
            }
        }
    };
    hooks.on_post_save();
    report
}

/// The end of `IngameState.exit`: `PlayerDB.getInstance().close()`, then
/// `VehiclesDB2.instance.Reset()` (the queued vehicle updates were written
/// by the streamer already; the connection closes).
pub fn close_stores(players: Option<&mut PlayerDb>, vehicles: &mut VehicleStore) {
    if let Some(db) = players {
        db.close();
    }
    vehicles.reset();
}

/// The world state the hot save reads.
pub struct HotSave<'a> {
    pub save_dir: &'a Path,
    pub no_save: bool,
    pub meta: &'a mut IsoMetaGrid,
    pub meta_world: &'a MetaWorld,
    pub spawned_zones: &'a SpawnedZombieZones,
    pub uuids: &'a mut dyn UuidSource,
    pub game_time: &'a GameTime,
    pub core: &'a Core,
}

/// The part of `ChunkSaveWorker.HotsaveAncilliarySystems` this crate can
/// write, for `pz_world::streamer::StreamEvent::HotSave`: the main thread
/// fills the save buffer map with `IsoMetaGrid.saveToBufferMap`
/// (`map_meta.bin`, `map_zone.bin`; `map_animals.bin` and the queued
/// `metacell_*.bin` are not ported), the animal population's buffers (not
/// ported), `GameTime.saveToBufferMap` (`map_t.bin`) and the world map's
/// (not ported); `PlayerDB.savePlayers()` follows, and the buffers are
/// written unless saving is off. The buffers hold the same bytes as the
/// `save()` methods. Failures are logged and returned; the other files
/// are still written.
pub fn hot_save_ancillary(h: &mut HotSave<'_>) -> Vec<String> {
    let mut errors = Vec::new();
    let mut files: Vec<(&str, Vec<u8>)> = Vec::new();
    files.push((
        "map_meta.bin",
        save_map_meta(h.meta, h.meta_world, NetRole::SinglePlayer),
    ));
    match h.meta.save_zones(h.spawned_zones, &mut *h.uuids) {
        Ok(z) => files.push(("map_zone.bin", z)),
        Err(e) => errors.push(format!("IsoMetaGrid.saveToBufferMap: {e}")),
    }
    match game_time_save::save_map_t(h.game_time, h.core, false) {
        Ok(t) => files.push(("map_t.bin", t)),
        Err(e) => errors.push(format!("GameTime.saveToBufferMap: {e}")),
    }
    if !h.no_save {
        for (name, bytes) in files {
            if let Err(e) = write(h.save_dir, name, &bytes) {
                errors.push(format!("{name}: {e}"));
            }
        }
    }
    for e in &errors {
        eprintln!("ERROR: {e}");
    }
    errors
}

#[cfg(test)]
mod tests;
