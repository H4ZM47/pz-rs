//! The optional 3D view of the world (first person, third person and
//! free camera). This is a port extension, not part of the original game:
//! with [`ViewMode::Iso`] (the default) nothing here runs and the frame is
//! the original isometric one.
//!
//! # Space
//!
//! Everything here is in *model space*, the space
//! [`pz_iso_render::models::ModelCamera::model_view`] places character and
//! vehicle models in before its isometric rotation, relative to an origin
//! (normally the camera character's position `(cx, cy, cz)`):
//!
//! ```text
//! model = ( -(x - cx),  (z - cz) * LEVEL_HEIGHT,  -(y - cy) )
//! ```
//!
//! with world `x` east, `y` south and `z` the level, one square = 1 unit and
//! one level = [`LEVEL_HEIGHT`] (√6) units. Model-space `+Y` is up. The map
//! from world to model space is a reflection, so triangle winding is not
//! meaningful; draw with culling off.
//!
//! Matrices are [`Mat4`] (column-major, JOML post-multiplying), and
//! projections use GL clip space (depth -1..1), as `ModelDraw::mvp` does.

pub mod camera;
pub mod frame;
pub mod mesh;
pub mod view;

pub use pz_iso_render::models::LEVEL_HEIGHT;
pub use pz_iso_render::trees::Mat4;

/// World `(x, y, z)` to model space around `origin` (world coordinates).
pub fn to_model(p: [f32; 3], origin: [f32; 3]) -> [f32; 3] {
    [
        -(p[0] - origin[0]),
        (p[2] - origin[2]) * LEVEL_HEIGHT,
        -(p[1] - origin[1]),
    ]
}

/// Which view the world is drawn from.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ViewMode {
    /// The original isometric view.
    #[default]
    Iso,
    /// From the player's eyes.
    FirstPerson,
    /// Behind and above the player.
    ThirdPerson,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn to_model_axes() {
        let o = [10.0, 20.0, 0.0];
        assert_eq!(to_model([11.0, 20.0, 0.0], o), [-1.0, 0.0, 0.0]);
        assert_eq!(to_model([10.0, 21.0, 0.0], o), [0.0, 0.0, -1.0]);
        assert_eq!(to_model([10.0, 20.0, 1.0], o), [0.0, LEVEL_HEIGHT, 0.0]);
    }
}
