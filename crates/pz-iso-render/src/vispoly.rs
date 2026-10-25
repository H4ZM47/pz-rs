//! `VisibilityPolygon2`: the vision cone's darkening polygon, drawn on the
//! camera character's level after its shadows.

use pz_render::DrawCmd;

use crate::cell::ChunkRenderer;
use crate::frame::FrameSize;

/// One frame's vision-cone inputs.
#[derive(Clone, Debug, Default)]
pub struct VisionFrame {}

/// `VisibilityPolygon2.renderMain(playerIndex)`.
pub fn render_main(
    _r: &ChunkRenderer<'_>,
    _v: &VisionFrame,
    _size: FrameSize,
    _out: &mut Vec<DrawCmd>,
) {
}
