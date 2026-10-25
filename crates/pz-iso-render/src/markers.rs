//! `WorldMarkers`, `FBORenderWorldMarkers` and `IsoMarkers`, and
//! `FBORenderCell.renderOpaqueObjectsEvent`.

use pz_render::DrawCmd;

use crate::area_highlights::Line;
use crate::cell::ChunkRenderer;
use crate::models::ModelCamera;
use crate::Camera;

/// `renderOpaqueObjectsEvent`'s Lua event.
pub struct OpaqueObjectsEvent<'a> {
    pub handler: &'a mut dyn FnMut(i32, i32, i32, &mut Vec<DrawCmd>),
}

impl OpaqueObjectsEvent<'_> {
    pub fn trigger(&mut self, _cam: &Camera, _out: &mut Vec<DrawCmd>) {}
}

/// One frame's markers.
#[derive(Clone, Debug, Default)]
pub struct MarkersFrame {
    /// `LineDrawer` lines the markers add.
    pub lines: Vec<Line>,
}

impl MarkersFrame {
    /// The levels holding markers.
    pub fn levels(&self) -> Vec<i32> {
        Vec::new()
    }

    pub fn render_level(
        &mut self,
        _r: &ChunkRenderer<'_>,
        _cam: &ModelCamera,
        _z: i32,
        _out: &mut Vec<DrawCmd>,
    ) {
    }
}
