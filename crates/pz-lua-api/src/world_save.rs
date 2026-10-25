//! Saving a single-player game: `GameWindow.save(true)` with
//! `IsoCell.save`, the quit sequence of `IngameState` (`Core.exiting`, then
//! `IngameState.exit`), and the main-thread part of the chunk save worker's
//! hot save (`ChunkSaveWorker.HotsaveAncilliarySystems`).
//!
//! The files this module writes itself, in the original's order:
//!
//! | step | file |
//! |---|---|
//! | `GameWindow.save` | `map_ver.bin`, `map_sand.bin`, `map_worldgen.bin`, `iTrack.bin` (and `ItemTracker.log` in the cache folder), `metadata.bin`, `statistics.bin`, then `thumb.png` (pixels from [`SaveGameHooks::capture_thumbnail`]) |
//! | `IsoCell.save` | chunk files and `vehicles.db` rows (through [`SaveGameHooks::save_chunks`]), `map.bin`, `map_t.bin`, `map_meta.bin`, `map_zone.bin`, `map_animals.bin`, `metagrid/metacell_*.bin`, `reanimated.bin` |
//! | `SGlobalObjects.save` | `gos_*.bin` |
//!
//! The systems this crate cannot reach (the chunk streamer's borrows, Lua
//! events, the zombie and animal populations, the radio and the world map)
//! run through [`SaveGameHooks`] at their place in the order.
//!
//! The multiplayer branches (`GameClient.clientSave`, `loadedAsClient`) are
//! not ported: this is the local game's save.

use std::collections::VecDeque;
use std::io;
use std::path::Path;

use pz_core::core::Core;
use pz_core::game_time::GameTime;
use pz_core::game_time_save;
use pz_core::save_trackers::{InstanceTracker, MetaTracker, Statistics};
use pz_core::thumbnail::{self, Thumbnail};
use pz_kahlua::java_io::JavaWrite;
use pz_world::meta_save::{save_map_meta, MetaWorld, NetRole};
use pz_world::save::vehicle::VehicleRow;
use pz_world::save::vehicles_db::{self, ChunkVehiclesUnloaded};
use pz_world::zone_save::{SpawnedZombieZones, UuidSource};
use pz_world::IsoMetaGrid;
use pz_worldgen::params::WorldGenParams;

use pz_characters::character::save::SaveEnv;
use pz_characters::character::{GameCharacter, SimContext};
use pz_characters::worn_items::ItemIdentity;
use pz_characters::Characters;
use pz_inventory::io::Writer;
use pz_scripts::{ScriptError, ScriptManager, ScriptResult};

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

/// `PlayerDB.PlayerData.set(player)` for [`save_on_quit`]'s local players:
/// the player's id (`-1` gets one from [`PlayerDb::save_player`], which the
/// engine keeps as the player's `sqlId`), position, `isDead()`, the
/// descriptor's `forename + " " + surname`, the world version and the
/// `IsoPlayer.save` bytes (`GameCharacter::save_player`).
pub fn local_player_data<I: ItemIdentity + 'static>(
    player: &mut GameCharacter<I>,
    sql_id: i32,
    sm: &ScriptManager,
    chars: &Characters,
    ctx: &dyn SimContext,
    env: &SaveEnv<'_>,
) -> ScriptResult<PlayerData> {
    let name = {
        let d = player
            .descriptor
            .as_ref()
            .ok_or_else(|| ScriptError::new("java.lang.NullPointerException"))?;
        let d = d.borrow();
        format!(
            "{} {}",
            d.forename.as_deref().unwrap_or("null"),
            d.surname.as_deref().unwrap_or("null")
        )
    };
    let mut w = Writer::new();
    player.save_player(sm, chars, ctx, env, &mut w)?;
    Ok(PlayerData {
        sql_id,
        x: player.vars.x,
        y: player.vars.y,
        z: player.vars.z,
        is_dead: player.is_dead(),
        name: Some(name),
        world_version: WORLD_VERSION,
        bytes: w.buf,
    })
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
    /// ([`reanimated_bytes`];
    /// `pz_characters::character::save::reanimated_players_to_save`).
    /// `None`: `saveReanimatedPlayers` threw (logged), `reanimated.bin` is
    /// not written.
    pub reanimated: Option<&'a [Vec<u8>]>,
    /// `WorldGenParams.INSTANCE` (`map_worldgen.bin`).
    pub world_gen: &'a WorldGenParams,
    /// `InstanceTracker` (`iTrack.bin`).
    pub instance_tracker: &'a InstanceTracker,
    /// `ZomboidFileSystem.getCacheDir()`, where `InstanceTracker.save`
    /// writes `ItemTracker.log`; `None` skips the log.
    pub cache_dir: Option<&'a Path>,
    /// `IsoWorld.helicopter` and `AmbientStreamManager`'s alarms
    /// (`metadata.bin`).
    pub meta_tracker: &'a MetaTracker,
    /// `StatisticsManager` (`statistics.bin`).
    pub statistics: &'a Statistics,
    /// `IsoPlayer.players[n] != null` for each local player slot
    /// (`SavefileThumbnail.create` uses the first).
    pub local_players: &'a [bool],
    /// `Thread.currentThread() == GameWindow.gameThread`: only then is the
    /// thumbnail made.
    pub on_game_thread: bool,
}

/// The steps of the save that belong to systems outside this crate, called
/// at their place in `GameWindow.save`.
pub trait SaveGameHooks {
    /// After `map_worldgen.bin`, `iTrack.bin`, `metadata.bin` and
    /// `statistics.bin`: `LuaEventManager.triggerEvent("OnSave")`.
    fn on_save(&mut self) {}
    /// `SavefileThumbnail.create(player)` on the game thread, after
    /// `OnSave`: the renderer sets player `player`'s zoom to 1, centres the
    /// camera, renders the world and reads back the 256x256 square in the
    /// middle of the player's view (see [`pz_core::thumbnail`]), then
    /// restores zoom and screen. `None` writes no thumbnail (no renderer,
    /// or the capture failed). [`save_game`] writes `thumb.png`; the
    /// renderer reloads the texture (`Texture.reload`).
    fn capture_thumbnail(&mut self, _player: usize) -> Option<Thumbnail> {
        None
    }
    /// `IsoCell.save` before `map.bin` gets its header: wait while the
    /// chunk save worker is saving
    /// (`pz_world::streamer::ChunkStreamer::wait_for_saves`), then
    /// `chunkMap[n].Save()` for every player
    /// (`ChunkStreamer::save_chunk_maps`), which writes the chunk files and
    /// their vehicles (`VehiclesDB2.unloadChunk`). `meta` is the grid the
    /// streamer context borrows.
    fn save_chunks(&mut self, meta: &IsoMetaGrid);
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
    let log = |r: &mut SaveReport, what: &str, e: io::Result<()>| {
        if let Err(e) = e {
            eprintln!("ERROR: {what}: {e}");
            r.errors.push(format!("{what}: {e}"));
        }
    };
    log(
        &mut report,
        "WorldGenParams.save",
        g.world_gen.save_file(dir, g.no_save),
    );
    log(
        &mut report,
        "InstanceTracker.save",
        g.instance_tracker.save_file(dir, g.no_save),
    );
    // `exportFile(.., "ItemTracker.log", ..)` runs even when the save
    // failed.
    if let Some(cache) = g.cache_dir {
        let r = std::fs::write(
            cache.join("ItemTracker.log"),
            g.instance_tracker.item_tracker_log(),
        );
        log(&mut report, "InstanceTracker.exportFile", r);
    }
    log(
        &mut report,
        "MetaTracker.save",
        g.meta_tracker.save_file(dir, g.no_save),
    );
    log(
        &mut report,
        "StatisticsManager.save",
        g.statistics.save_file(dir, g.no_save),
    );
    hooks.on_save();
    if g.on_game_thread {
        if let Some(player) = thumbnail::thumbnail_player(g.local_players) {
            if let Some(t) = hooks.capture_thumbnail(player) {
                log(
                    &mut report,
                    "SavefileThumbnail.create",
                    thumbnail::save_thumbnail(dir, &t),
                );
            }
        }
    }
    // `map.bin` is opened here; `IsoCell.save` writes its header after the
    // chunks.
    hooks.save_chunks(g.meta);
    let header = write(dir, "map.bin", &map_bin_bytes(CELL_SIZE));
    let header_ok = header.is_ok();
    log(&mut report, "map.bin", header);
    // An exception in `IsoCell.save` skips the rest of it.
    if header_ok {
        let gt = game_time_save::save_map_t_file(g.game_time, g.core, dir);
        let gt_ok = gt.is_ok();
        log(&mut report, "map_t.bin", gt);
        if gt_ok {
            save_meta_grid(g, &mut report);
            if g.players.is_some() && g.can_save_players {
                report.force_save_players = true;
            }
            if let Some(zombies) = g.reanimated {
                let r = write(dir, "reanimated.bin", &reanimated_bytes(zombies));
                log(&mut report, "reanimated.bin", r);
            }
        }
    }
    hooks.save_animals();
    hooks.save_population();
    let r = global_objects_save::save_all(dir, g.no_save, false);
    log(&mut report, "SGlobalObjects.save", r);
    hooks.save_other_systems();
    Ok(report)
}

/// `IsoMetaGrid.save()`: `map_meta.bin`, `map_zone.bin`, `map_animals.bin`,
/// then `metagrid/metacell_*.bin` for the queued cells; the first failure
/// ends it (logged).
fn save_meta_grid(g: &mut SaveGame<'_>, report: &mut SaveReport) {
    let dir = g.save_dir;
    let meta = save_map_meta(g.meta, g.meta_world, NetRole::SinglePlayer);
    let r = write(dir, "map_meta.bin", &meta)
        .and_then(|_| g.meta.save_zones_file(dir, g.spawned_zones, &mut *g.uuids))
        .and_then(|_| g.meta.save_animal_zones_file(dir, &mut *g.uuids))
        .and_then(|_| g.meta.save_cells_files(dir));
    if let Err(e) = r {
        eprintln!("ERROR: IsoMetaGrid.save: {e}");
        report.errors.push(format!("IsoMetaGrid.save: {e}"));
    }
}

/// The meta-grid loads of `IsoWorld.init` after `map_zone.bin`:
/// `loadCells("metagrid", ..)` (each cell's `animalZonesGenerated`), then
/// `load("map_animals.bin", loadAnimalZones)` (the saved animal zones
/// replace the map's). Failures are logged and returned; the second runs
/// after the first failed.
pub fn load_meta_grid_extras(meta: &mut IsoMetaGrid, save_dir: &Path) -> Vec<String> {
    let mut errors = Vec::new();
    if let Err(e) = meta.load_cells_files(save_dir) {
        errors.push(format!("IsoMetaGrid.loadCells: {e}"));
    }
    if let Err(e) = meta.load_animal_zones_file(save_dir) {
        errors.push(format!("IsoMetaGrid.loadAnimalZones: {e}"));
    }
    for e in &errors {
        eprintln!("ERROR: {e}");
    }
    errors
}

/// `SavefileNaming.ensureSubdirectoriesExist(saveDir)` (`GameLoadingState`,
/// unless saving is off): `apop`, `chunkdata`, `map`, `zpop` and
/// `metagrid`.
pub fn ensure_subdirectories_exist(save_dir: &Path) -> io::Result<()> {
    for d in ["apop", "chunkdata", "map", "zpop", "metagrid"] {
        std::fs::create_dir_all(save_dir.join(d))?;
    }
    Ok(())
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
/// `VehiclesDB2.instance.Reset()`: `updateWorldStreamer()` writes the rows
/// still queued for the stopped streamer thread (`pending`, from
/// `pz_world::streamer::ChunkStreamer::take_vehicle_queue`: the vehicles
/// the quit's `removeFromWorld` sent to the meta), then the connection
/// closes. Returns the ids written and the failures (logged).
pub fn close_stores(
    players: Option<&mut PlayerDb>,
    vehicles: &mut VehicleStore,
    mut pending: VecDeque<VehicleRow>,
) -> ChunkVehiclesUnloaded {
    if let Some(db) = players {
        db.close();
    }
    let r = vehicles_db::update_world_streamer(vehicles.conn.as_mut(), &mut pending);
    vehicles.reset();
    r
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
/// (`map_meta.bin`, `map_zone.bin`, `map_animals.bin` and the queued
/// `metagrid/metacell_*.bin`, which leave the queue even with saving off),
/// the animal population's buffers (not ported),
/// `GameTime.saveToBufferMap` (`map_t.bin`) and the world map's (not
/// ported); `PlayerDB.savePlayers()` follows, and the buffers are
/// written unless saving is off. The buffers hold the same bytes as the
/// `save()` methods. Failures are logged and returned; the other files
/// are still written.
pub fn hot_save_ancillary(h: &mut HotSave<'_>) -> Vec<String> {
    let mut errors = Vec::new();
    let mut files: Vec<(String, Vec<u8>)> = Vec::new();
    files.push((
        "map_meta.bin".into(),
        save_map_meta(h.meta, h.meta_world, NetRole::SinglePlayer),
    ));
    // `saveToBufferMap`: the first failure ends the meta grid's part.
    let meta = h
        .meta
        .save_zones(h.spawned_zones, &mut *h.uuids)
        .and_then(|z| {
            files.push(("map_zone.bin".into(), z));
            let a = h.meta.save_animal_zones(&mut *h.uuids)?;
            files.push(("map_animals.bin".into(), a));
            files.extend(h.meta.save_cells());
            Ok(())
        });
    if let Err(e) = meta {
        errors.push(format!("IsoMetaGrid.saveToBufferMap: {e}"));
    }
    match game_time_save::save_map_t(h.game_time, h.core, false) {
        Ok(t) => files.push(("map_t.bin".into(), t)),
        Err(e) => errors.push(format!("GameTime.saveToBufferMap: {e}")),
    }
    if !h.no_save {
        for (name, bytes) in files {
            if let Err(e) = write(h.save_dir, &name, &bytes) {
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
