//! `FBORenderAreaHighlights` and `LineDrawer.render`.

use pz_render::DrawCmd;

use crate::models::ModelCamera;
use crate::Camera;

/// A `LineDrawer.DrawableLine`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Line {}

/// One frame's area highlights.
#[derive(Clone, Debug, Default)]
pub struct AreaHighlightsFrame {}

impl AreaHighlightsFrame {
    pub fn render(
        &self,
        _cam: &Camera,
        _mc: &ModelCamera,
        _lines: &mut Vec<Line>,
        _out: &mut Vec<DrawCmd>,
    ) {
    }
}

/// `LineDrawer.render`.
pub fn render_lines(_cam: &Camera, _lines: &[Line], _out: &mut Vec<DrawCmd>) {}
