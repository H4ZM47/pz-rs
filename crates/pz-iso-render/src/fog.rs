//! `FBORenderCell.renderFog`: `ImprovedFog` over levels 0 and 1, drawn
//! after the per-level translucent passes (distinct from `IsoWeatherFX`'s
//! fog layer).

use pz_render::DrawCmd;

use crate::cell::ChunkRenderer;
use crate::frame::{ChunkLevels, FrameSize};

/// One frame's `ImprovedFog` inputs.
#[derive(Clone, Debug, Default)]
pub struct FogFrame {}

/// `renderFog(playerIndex)`.
pub fn render_fog(
    _r: &ChunkRenderer<'_>,
    _fog: &FogFrame,
    _chunks: &[ChunkLevels<'_>],
    _size: FrameSize,
    _out: &mut Vec<DrawCmd>,
) {
}
