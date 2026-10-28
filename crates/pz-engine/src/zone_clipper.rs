//! The meta grid's native clipper (`IsoMetaGrid.clipperOffset` and
//! `Zone`'s shared `Clipper`): [`pz_clipper::ZoneClipper`] behind
//! [`pz_world::zones::ClipperOps`].

use pz_clipper::ZoneClipper;
use pz_world::zones::ClipperOps;

#[derive(Default)]
pub struct ZoneClipperOps(pub ZoneClipper);

impl ClipperOps for ZoneClipperOps {
    fn offset_polyline(&mut self, points: &[f32], delta: f64) -> Option<Vec<f32>> {
        self.0.offset_polyline(points, delta)
    }

    fn clip_polygon_aabb(
        &mut self,
        points: &[f32],
        x1: f32,
        y1: f32,
        x2: f32,
        y2: f32,
    ) -> Vec<Vec<f32>> {
        self.0.clip_polygon_aabb(points, x1, y1, x2, y2)
    }
}
