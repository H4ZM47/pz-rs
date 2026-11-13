//! Inputs the world map reads from other workstreams' state, with no-op
//! defaults until their owners install real ones.

use std::cell::RefCell;

/// A local player as the map draws it (`IsoPlayer.players[i]`).
#[derive(Clone, Debug, Default)]
pub struct MapPlayer {
    /// The player's position, or its vehicle's when it is in one.
    pub x: f32,
    pub y: f32,
    pub username: String,
}

/// `IsoPlayer.players` for the map's player dots. Owner: the characters /
/// player workstream (pz-characters).
pub trait WorldMapPlayers {
    /// The live (non-null, not dead) local players, in index order.
    fn live_players(&self) -> Vec<MapPlayer> {
        Vec::new()
    }
}

/// `IsoWorld.instance.getMetaGrid()`'s cell bounds for
/// `setBoundsFromWorld`. Owner: the world workstream (pz-world).
pub trait WorldMapMetaGrid {
    /// `(minX, minY, maxX, maxY)` in 256-square cells; `None` when there is
    /// no meta grid (the original NPEs).
    fn cell_bounds(&self) -> Option<(i32, i32, i32, i32)>;
}

struct NoHost;
impl WorldMapPlayers for NoHost {}

/// The meta grid Lua sees through `IsoWorld` (`bind_world`).
pub struct WorldMetaGrid;
impl WorldMapMetaGrid for WorldMetaGrid {
    fn cell_bounds(&self) -> Option<(i32, i32, i32, i32)> {
        crate::bind_world::meta_grid_cell_bounds()
    }
}

/// `MapFiles.getCurrentMapFiles()`: the loaded world's `IsoLot.MapFiles`
/// (the meta grid's) when there are any, else the standalone list loaded
/// from `Core.gameMap`'s lot directories and kept until the map changes or
/// [`reset_current_map_files`] (`MapFiles.Reset`).
pub struct CurrentMapFiles;

thread_local! {
    /// `MapFiles.currentMap` and `MapFiles.currentMapFiles`.
    static CURRENT: RefCell<(Option<String>, Vec<pz_world::MapFiles>)> =
        const { RefCell::new((None, Vec::new())) };
}

/// `MapFiles.Reset()`.
pub fn reset_current_map_files() {
    CURRENT.with(|c| *c.borrow_mut() = (None, Vec::new()));
}

/// Run `f` on `MapFiles.getCurrentMapFiles()`.
pub fn with_current_map_files<R>(f: impl FnOnce(&[pz_world::MapFiles]) -> R) -> R {
    let mut f = Some(f);
    if let Some(r) = crate::bind_world::with_world_map_files(|files| {
        (!files.is_empty()).then(|| (f.take().unwrap())(files))
    })
    .flatten()
    {
        return r;
    }
    let f = f.take().unwrap();
    CURRENT.with(|c| {
        let game_map = crate::bind_core::shared_core().borrow().game_map.clone();
        let stale = {
            let c = c.borrow();
            c.1.is_empty() || c.0.as_deref() != Some(game_map.as_str())
        };
        if stale {
            reset_current_map_files();
            let files = super::with_fs(|fs| {
                // `getLotDirectories()`: may switch a DEFAULT game map to the
                // only map group's world first, so `currentMap` is read after.
                let dirs = crate::bind_world::lua_world::lot_directories(fs).unwrap_or_default();
                pz_world::map_files::load_current_map_files(fs, &dirs)
            })
            .unwrap_or_default();
            let map = crate::bind_core::shared_core().borrow().game_map.clone();
            *c.borrow_mut() = (Some(map), files);
        }
        f(&c.borrow().1)
    })
}

impl pz_worldmap::triangulate::MapFilesInfo for CurrentMapFiles {
    fn map_directory_names(&self) -> Vec<String> {
        with_current_map_files(|m| m.iter().map(|f| f.map_directory_name.clone()).collect())
    }
    fn has_cell300(&self, index: usize, x: i32, y: i32) -> bool {
        with_current_map_files(|m| m.get(index).is_some_and(|f| f.has_cell300(x, y)))
    }
}

/// The installed inputs.
pub struct WorldMapHost {
    pub players: Box<dyn WorldMapPlayers>,
    pub meta_grid: Box<dyn WorldMapMetaGrid>,
    /// `MapFiles.getCurrentMapFiles()` (cell priority and the clipping of
    /// lower-priority cells). Owner: the world workstream (pz-world
    /// `MapFiles`).
    pub map_files: Box<dyn pz_worldmap::triangulate::MapFilesInfo>,
}

impl Default for WorldMapHost {
    fn default() -> Self {
        WorldMapHost {
            players: Box::new(NoHost),
            meta_grid: Box::new(WorldMetaGrid),
            map_files: Box::new(CurrentMapFiles),
        }
    }
}

thread_local! {
    static HOST: RefCell<WorldMapHost> = RefCell::new(WorldMapHost::default());
}

/// Replace the world-map inputs.
pub fn install(h: WorldMapHost) {
    HOST.with(|s| *s.borrow_mut() = h);
}

/// Run `f` on the installed inputs. Implementations must not call Lua.
pub fn with_host<R>(f: impl FnOnce(&mut WorldMapHost) -> R) -> R {
    HOST.with(|s| f(&mut s.borrow_mut()))
}
