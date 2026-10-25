//! `FBORenderObjectHighlight`.

use pz_render::DrawCmd;

use crate::cell::ChunkRenderer;
use crate::world::RenderSquare;

/// One frame's highlight pass inputs.
#[derive(Clone, Debug, Default)]
pub struct HighlightFrame {}

/// `FBORenderObjectHighlight.render(playerIndex)`.
pub fn render(
    _r: &ChunkRenderer<'_>,
    _h: &HighlightFrame,
    _squares: &[&RenderSquare],
    _out: &mut Vec<DrawCmd>,
) {
}
