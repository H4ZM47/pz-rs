//! `FBORenderCell.renderFlies` and `CorpseFlies`.

use pz_render::DrawCmd;

use crate::cell::ChunkRenderer;
use crate::frame::ChunkLevels;

/// `CorpseFlies`' state.
#[derive(Clone, Debug, Default)]
pub struct CorpseFlies {}

/// `renderFlies(playerIndex)`.
pub fn render_flies(
    _r: &ChunkRenderer<'_>,
    _f: &CorpseFlies,
    _chunks: &[ChunkLevels<'_>],
    _out: &mut Vec<DrawCmd>,
) {
}
