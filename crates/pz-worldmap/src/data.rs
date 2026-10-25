//! `zombie.worldMap.WorldMapData`: one map data file (`worldmap.xml` or
//! its `.bin`), shared by every map UI that shows it.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

/// `Asset.State`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AssetState {
    Empty,
    Ready,
    Failure,
}

/// `zombie.worldMap.WorldMapData`.
pub struct WorldMapData {
    /// The absolute path the asset was created for.
    pub path: Rc<str>,
    /// `relativeFileName`: the name the last `addData` used.
    pub relative_file_name: RefCell<Rc<str>>,
    pub state: Cell<AssetState>,
    /// `minX, minY, maxX, maxY` in cells.
    pub bounds: Cell<[i32; 4]>,
}

impl WorldMapData {
    pub fn new(path: &str) -> Self {
        WorldMapData {
            path: path.into(),
            relative_file_name: RefCell::new("".into()),
            state: Cell::new(AssetState::Empty),
            bounds: Cell::new([0; 4]),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.state.get() == AssetState::Empty
    }

    pub fn is_ready(&self) -> bool {
        self.state.get() == AssetState::Ready
    }

    pub fn is_failure(&self) -> bool {
        self.state.get() == AssetState::Failure
    }
}
