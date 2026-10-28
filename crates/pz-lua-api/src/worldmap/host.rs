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
            map_files: Box::new(pz_worldmap::triangulate::NoMapFiles),
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
