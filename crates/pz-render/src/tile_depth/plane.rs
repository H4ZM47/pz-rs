//! Recovering a roof sprite's slope from its depth map.
//!
//! A depth pixel is the normalized iso depth of the surface under it, so the
//! pixel position (two ortho-camera coordinates) plus that depth (the third)
//! fixes a point in tile scene space. Roof tiles are planar, so a least
//! squares plane through those points gives the slope.
//!
//! Scene space: `x` east and `z` south (both tile-centred, the NW floor
//! corner is (-0.5, 0, -0.5)), `y` up with one level = [`UPPER_FLOOR_Y`]. The
//! offsets within the square are `dx = x + 0.5`, `dy = z + 0.5` and
//! `dz = y / UPPER_FLOOR_Y` (levels).

use super::clamp::{matrices_for_square, raw_depth_of_normalized, Vec3, UPPER_FLOOR_Y};

/// Fewest depth pixels a fit needs.
pub const MIN_FIT_PIXELS: usize = 32;
/// Largest RMS residual (levels) of an accepted fit.
pub const MAX_FIT_RMS: f32 = 0.05;

/// Heights, in levels above the square's floor, of a roof plane at the
/// square's corners, in the order NW (dx 0, dy 0), NE (1, 0), SE (1, 1),
/// SW (0, 1).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DepthPlane {
    pub corners: [f32; 4],
}

/// The scene point shown at tile pixel (`px`, `py`) (pixel coordinates, e.g.
/// centre = index + 0.5) of a `width` x `height` tile with normalized depth
/// `normalized`. Pixel to NDC exactly as `getDepthOnPlaneAt`: x spans the
/// tile width and y its height (the viewport is 1 x 2 tiles).
pub(super) fn scene_point_of_pixel(
    width: i32,
    height: i32,
    px: f32,
    py: f32,
    normalized: f32,
) -> Vec3 {
    let (projection, model_view) = matrices_for_square();
    let inverse = projection.mul_affine(&model_view).invert_affine();
    let tile_x = px / width as f32;
    let tile_y = py * 2.0 / height as f32;
    let win_z = (raw_depth_of_normalized(normalized) + 1.0) * 0.5;
    inverse.unproject_inv(tile_x, 2.0 - tile_y, win_z, [0, 0, 1, 2])
}

/// Least-squares plane `dz = a + b*dx + c*dy` through the depth pixels, as
/// corner heights. `None` with fewer than [`MIN_FIT_PIXELS`] pixels of depth
/// (>= 0), a degenerate layout, or an RMS residual above [`MAX_FIT_RMS`].
pub fn fit_depth_plane(width: i32, height: i32, pixels: &[f32]) -> Option<DepthPlane> {
    if width <= 0 || height <= 0 || pixels.len() < (width * height) as usize {
        return None;
    }
    let (projection, model_view) = matrices_for_square();
    let inverse = projection.mul_affine(&model_view).invert_affine();
    let mut pts: Vec<[f64; 3]> = Vec::new();
    for y in 0..height {
        for x in 0..width {
            let d = pixels[(x + y * width) as usize];
            if d < 0.0 {
                continue;
            }
            let tile_x = (x as f32 + 0.5) / width as f32;
            let tile_y = (y as f32 + 0.5) * 2.0 / height as f32;
            let win_z = (raw_depth_of_normalized(d) + 1.0) * 0.5;
            let p = inverse.unproject_inv(tile_x, 2.0 - tile_y, win_z, [0, 0, 1, 2]);
            pts.push([
                (p.x + 0.5) as f64,
                (p.z + 0.5) as f64,
                (p.y / UPPER_FLOOR_Y) as f64,
            ]);
        }
    }
    if pts.len() < MIN_FIT_PIXELS {
        return None;
    }
    // Fit, then refit once without outliers: real roof pieces carry a few
    // non-planar pixels (eaves, fascia, ridge caps) that would otherwise
    // reject or tilt the plane.
    let all = pts.len();
    let (mut plane, mut rms) = fit(&pts)?;
    let cut = (2.5 * rms).max(OUTLIER_LEVELS);
    pts.retain(|p| residual(plane, p).abs() <= cut);
    if pts.len() >= MIN_FIT_PIXELS && pts.len() * 10 >= all * 7 {
        (plane, rms) = fit(&pts)?;
    }
    if !rms.is_finite() || rms > MAX_FIT_RMS as f64 {
        return None;
    }
    let [a, b, c] = plane;
    Some(DepthPlane {
        corners: [a as f32, (a + b) as f32, (a + b + c) as f32, (a + c) as f32],
    })
}

/// A residual (levels) always kept by the outlier pass.
const OUTLIER_LEVELS: f64 = 0.03;

fn residual([a, b, c]: [f64; 3], p: &[f64; 3]) -> f64 {
    p[2] - (a + b * p[0] + c * p[1])
}

/// Least-squares `[a, b, c]` and its RMS residual.
fn fit(pts: &[[f64; 3]]) -> Option<([f64; 3], f64)> {
    // Normal equations for [1, dx, dy] . [a, b, c] = dz.
    let mut ata = [[0.0f64; 3]; 3];
    let mut atz = [0.0f64; 3];
    for p in pts {
        let r = [1.0, p[0], p[1]];
        for i in 0..3 {
            for j in 0..3 {
                ata[i][j] += r[i] * r[j];
            }
            atz[i] += r[i] * p[2];
        }
    }
    let plane = solve3(ata, atz)?;
    let ss: f64 = pts.iter().map(|p| residual(plane, p).powi(2)).sum();
    Some((plane, (ss / pts.len() as f64).sqrt()))
}

/// Solves a 3x3 system by Gaussian elimination with partial pivoting.
fn solve3(mut m: [[f64; 3]; 3], mut v: [f64; 3]) -> Option<[f64; 3]> {
    for col in 0..3 {
        let piv = (col..3).max_by(|&i, &j| m[i][col].abs().total_cmp(&m[j][col].abs()))?;
        if m[piv][col].abs() < 1e-12 {
            return None;
        }
        m.swap(col, piv);
        v.swap(col, piv);
        for r in col + 1..3 {
            let f = m[r][col] / m[col][col];
            for k in col..3 {
                m[r][k] -= f * m[col][k];
            }
            v[r] -= f * v[col];
        }
    }
    let mut x = [0.0; 3];
    for i in (0..3).rev() {
        let s: f64 = (i + 1..3).map(|k| m[i][k] * x[k]).sum();
        x[i] = (v[i] - s) / m[i][i];
    }
    Some(x)
}

#[cfg(test)]
mod tests {
    use super::super::clamp::{depth_of_scene_point, depth_on_plane_at, normalized_depth};
    use super::*;

    const W: i32 = 128;
    const H: i32 = 256;

    /// Scene point to (pixel x, pixel y, normalized depth) of a 128x256 tile.
    fn forward(p: Vec3) -> (f32, f32, f32) {
        let (proj, mv) = matrices_for_square();
        let m = proj.mul_affine(&mv).m;
        let ndc_x = m[0][0] * p.x + m[1][0] * p.y + m[2][0] * p.z + m[3][0];
        let ndc_y = m[0][1] * p.x + m[1][1] * p.y + m[2][1] * p.z + m[3][1];
        let px = (ndc_x + 1.0) * 0.5 * W as f32;
        let py = (1.0 - ndc_y) * H as f32 * 0.5;
        (px, py, normalized_depth(depth_of_scene_point(p)))
    }

    /// A tile of the plane `dz = a + b*dx + c*dy`, cast pixel by pixel; only
    /// pixels whose hit lies over the square get depth.
    fn synth(a: f32, b: f32, c: f32, noise: f32) -> Vec<f32> {
        let level = |dx: f32, dy: f32| UPPER_FLOOR_Y * (a + b * dx + c * dy);
        let point = Vec3::new(0.0, level(0.5, 0.5), 0.0);
        let normal = Vec3::new(-UPPER_FLOOR_Y * b, 1.0, -UPPER_FLOOR_Y * c);
        let mut out = vec![-1.0f32; (W * H) as usize];
        for y in 0..H {
            for x in 0..W {
                let d = depth_on_plane_at(x as f32 + 0.5, y as f32 + 0.5, point, normal);
                if d == 666.0 {
                    continue;
                }
                let n = normalized_depth(d);
                let hit = scene_point_of_pixel(W, H, x as f32 + 0.5, y as f32 + 0.5, n);
                let (dx, dy) = (hit.x + 0.5, hit.z + 0.5);
                if (0.0..=1.0).contains(&dx) && (0.0..=1.0).contains(&dy) {
                    let jitter = noise * (((x * 31 + y * 17) % 7) as f32 / 3.0 - 1.0);
                    out[(x + y * W) as usize] = (n + jitter).max(0.0);
                }
            }
        }
        out
    }

    fn assert_corners(plane: Option<DepthPlane>, want: [f32; 4]) {
        let got = plane.expect("fit").corners;
        for i in 0..4 {
            assert!(
                (got[i] - want[i]).abs() < 0.01,
                "corner {i}: {got:?} vs {want:?}"
            );
        }
    }

    #[test]
    fn pixel_depth_round_trips_to_scene_point() {
        for p in [
            (0.0, 0.0, 0.0),
            (-0.5, 0.0, -0.5),
            (0.5, 0.3, 0.5),
            (0.2, 1.0, -0.3),
            (-0.4, 2.0, 0.1),
        ] {
            let p = Vec3::new(p.0, p.1, p.2);
            let (px, py, n) = forward(p);
            let q = scene_point_of_pixel(W, H, px, py, n);
            assert!(
                (q.x - p.x).abs() < 1e-3 && (q.y - p.y).abs() < 1e-3 && (q.z - p.z).abs() < 1e-3,
                "{p:?} -> {q:?}"
            );
        }
    }

    #[test]
    fn flat_floor() {
        assert_corners(fit_depth_plane(W, H, &synth(0.0, 0.0, 0.0, 0.0)), [0.0; 4]);
    }

    #[test]
    fn ramp_rising_west() {
        // 1/3 level at dx = 0, 0 at dx = 1.
        let third = 1.0 / 3.0;
        assert_corners(
            fit_depth_plane(W, H, &synth(third, -third, 0.0, 0.0)),
            [third, 0.0, 0.0, third],
        );
    }

    #[test]
    fn ramp_rising_north() {
        let third = 1.0 / 3.0;
        assert_corners(
            fit_depth_plane(W, H, &synth(third, 0.0, -third, 0.0)),
            [third, third, 0.0, 0.0],
        );
    }

    #[test]
    fn quantized_depth_still_fits() {
        let third = 1.0 / 3.0;
        let mut px = synth(third, -third, 0.0, 0.0);
        for v in px.iter_mut().filter(|v| **v >= 0.0) {
            *v = (*v * 255.0).floor() / 255.0;
        }
        assert_corners(fit_depth_plane(W, H, &px), [third, 0.0, 0.0, third]);
    }

    #[test]
    fn a_few_off_plane_pixels_are_ignored() {
        // About one pixel in twelve is off the plane (an eave or fascia).
        let third = 1.0 / 3.0;
        let mut px = synth(third, -third, 0.0, 0.0);
        for (i, v) in px.iter_mut().filter(|v| **v >= 0.0).enumerate() {
            if i % 12 == 0 {
                *v = (*v + 0.2).min(1.0);
            }
        }
        assert_corners(fit_depth_plane(W, H, &px), [third, 0.0, 0.0, third]);
    }

    #[test]
    fn noisy_or_sparse_tiles_have_no_plane() {
        // Heavy per-pixel noise: not a plane.
        assert_eq!(fit_depth_plane(W, H, &synth(0.0, 0.0, 0.0, 1.0)), None);
        // Too few pixels.
        let mut few = vec![-1.0f32; (W * H) as usize];
        for v in few.iter_mut().take(31) {
            *v = 0.7;
        }
        assert_eq!(fit_depth_plane(W, H, &few), None);
        // Wrong buffer size.
        assert_eq!(fit_depth_plane(W, H, &[0.5; 10]), None);
    }
}
