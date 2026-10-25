//! `FBORenderChunkCamera`: the camera that draws a model straight into a
//! chunk texture (`FBORenderCorpses`' mannequins and, with the debug option,
//! corpses), and `FBORenderCorpses.ChunkCamera.Begin`'s target depth.
//!
//! The projection is an orthographic box in 1920ths of the chunk texture's
//! size, with the chunk's floor at its `bottom`; the model-view puts the
//! model at its offset from the chunk's centre square (`wx * 8 + 4`) in the
//! isometric view, mirrored on x and scaled 1.5 as characters are.

// The original writes the angles as float literals.
#![allow(clippy::approx_constant)]

use pz_render::iso::{
    calculate_depth, chunk_depth_data, fastfloor, square_depth, PIXELS_PER_LEVEL, TILE_SCALE,
};

use crate::chunk::{extra_height_for_jumbo_trees, ChunkTexture, FLOOR_HEIGHT};
use crate::joml::Matrix4f;

/// `FBORenderChunkCamera.set(renderChunk, x, y, z, angle)` for a chunk
/// texture (never high-res here).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ChunkCamera {
    pub texture: ChunkTexture,
    pub origin: [f32; 3],
    pub bottom: i32,
    pub width: i32,
    pub height: i32,
}

impl ChunkCamera {
    pub fn new(texture: ChunkTexture) -> Self {
        let span = FLOOR_HEIGHT * 8;
        let levels = texture.top_level - texture.min_level + 1;
        let bottom = span
            + levels * PIXELS_PER_LEVEL
            + extra_height_for_jumbo_trees(texture.min_level, texture.top_level);
        ChunkCamera {
            texture,
            origin: [
                (texture.wx * 8) as f32 + 4.0,
                (texture.wy * 8) as f32 + 4.0,
                texture.min_level as f32,
            ],
            bottom,
            width: texture.width(),
            height: texture.height(),
        }
    }

    /// `pushProjectionMatrix`.
    pub fn projection(&self) -> Matrix4f {
        let screen_width = self.width as f32 / 1920.0;
        let span = FLOOR_HEIGHT * 8;
        let bottom = self.bottom as f32 - span as f32 / 2.0;
        let top = self.height as f32 - bottom;
        Matrix4f::set_ortho(
            -screen_width / 2.0,
            screen_width / 2.0,
            bottom / 1920.0,
            -top / 1920.0,
            -10.0,
            10.0,
        )
    }

    /// `pushModelViewMatrix(x, y, z, angle, false)`.
    pub fn model_view(&self, x: f32, y: f32, z: f32, angle: f32) -> Matrix4f {
        let [cx, cy, cz] = self.origin;
        Matrix4f::identity()
            .scale1(crate::models::core_scale())
            .scale1(TILE_SCALE as f32 / 2.0)
            .rotate(0.523_598_8, 1.0, 0.0, 0.0)
            .rotate(2.356_194_5, 0.0, 1.0, 0.0)
            .translate(-(x - cx), (z - cz) * 2.449_49, -(y - cy))
            .scale(-1.5, 1.5, 1.5)
            .rotate(angle + std::f32::consts::PI, 0.0, 1.0, 0.0)
    }

    /// `ChunkCamera.Begin`'s `targetDepth` for a model at `(x, y, z)` seen
    /// by a camera character on square `(cam_x, cam_y)`: its depth inside
    /// the chunk (relative to the chunk's depth when it stands outside the
    /// chunk), moved so the model's origin lands on it.
    pub fn target_depth(
        &self,
        cam_x: f32,
        cam_y: f32,
        x: f32,
        y: f32,
        z: f32,
        mvp: &Matrix4f,
    ) -> f32 {
        let t = self.texture;
        let inside = fastfloor(x / 8.0) == t.wx && fastfloor(y / 8.0) == t.wy;
        let mut depth = calculate_depth(x, y, z);
        if !inside {
            depth = square_depth(fastfloor(cam_x), fastfloor(cam_y), x, y, z);
            let chunk = chunk_depth_data(
                fastfloor(cam_x / 8.0),
                fastfloor(cam_y / 8.0),
                t.wx,
                t.wy,
                fastfloor(z),
            )
            .depth_start;
            depth -= chunk;
        }
        // VertexBufferObject.getDepthValueAt(0, 0, 0).
        let origin = mvp.transform([0.0, 0.0, 0.0, 1.0])[2];
        depth - (origin + 1.0) / 2.0 + 0.5
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn centre_square_projects_to_the_chunk_floor_centre() {
        let t = ChunkTexture::new(10, 12, 0, 0, 0);
        let c = ChunkCamera::new(t);
        let mvp = c.projection().mul(&c.model_view(84.0, 100.0, 0.0, 0.0));
        let p = mvp.transform([0.0, 0.0, 0.0, 1.0]);
        // x centred; y at the floor centre: `bottom - span / 2` up from the
        // texture's bottom edge in a box `height` tall.
        assert!(p[0].abs() < 1e-5, "{p:?}");
        let span = (FLOOR_HEIGHT * 8) as f32;
        let floor = c.bottom as f32 - span / 2.0;
        let want = 2.0 * floor / c.height as f32 - 1.0;
        assert!((p[1] - want).abs() < 1e-4, "{p:?} vs {want}");
        // One square east moves right and down by a tile's half extents.
        let q = mvp.transform([0.0, 0.0, 0.0, 1.0]);
        let m2 = c
            .projection()
            .mul(&c.model_view(85.0, 100.0, 0.0, 0.0))
            .transform([0.0, 0.0, 0.0, 1.0]);
        let px = (m2[0] - q[0]) * c.width as f32 / 2.0;
        let py = (m2[1] - q[1]) * c.height as f32 / 2.0;
        assert!((px - 32.0 * TILE_SCALE as f32).abs() < 0.05, "{px}");
        assert!((py.abs() - 16.0 * TILE_SCALE as f32).abs() < 0.05, "{py}");
    }
}
