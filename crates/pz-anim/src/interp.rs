//! `IInterpolator` and its implementations (`zombie.core.math.interpolators`),
//! used as an AnimNode's `m_BlendCurve`.

use crate::math::pzmath;

/// `LerpType`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LerpType {
    Linear,
    EaseOutQuad,
    EaseInQuad,
    EaseOutInQuad,
}

impl LerpType {
    pub fn parse(s: &str) -> Option<LerpType> {
        Some(match s {
            "Linear" => LerpType::Linear,
            "EaseOutQuad" => LerpType::EaseOutQuad,
            "EaseInQuad" => LerpType::EaseInQuad,
            "EaseOutInQuad" => LerpType::EaseOutInQuad,
            _ => return None,
        })
    }

    /// `PZMath.lerp(src, dest, alpha, lerpType)`.
    pub fn lerp(self, src: f32, dest: f32, alpha: f32) -> f32 {
        match self {
            LerpType::Linear => pzmath::lerp(src, dest, alpha),
            LerpType::EaseOutQuad => pzmath::lerp(src, dest, pzmath::lerp_func_ease_out_quad(alpha)),
            LerpType::EaseInQuad => pzmath::lerp(src, dest, pzmath::lerp_func_ease_in_quad(alpha)),
            LerpType::EaseOutInQuad => pzmath::lerp(src, dest, pzmath::lerp_func_ease_out_in_quad(alpha)),
        }
    }
}

/// `IInterpolator`.
#[derive(Clone, Debug, PartialEq)]
pub enum Interpolator {
    EaseInQuad { y0: f32, y1: f32 },
    EaseOutInQuad { y0: f32, y1: f32 },
    EaseOutQuad { y0: f32, y1: f32 },
    Linear { y0: f32, y1: f32 },
    One,
    Zero,
    /// `LerpPolyline`: points sorted by x.
    Polyline { points: Vec<(f32, f32)>, min_x: f32, max_x: f32 },
    /// `LerpSequence`: (x, y, lerp type into this point), sorted by x.
    Sequence { entries: Vec<(f32, f32, LerpType)>, min_x: f32, max_x: f32 },
}

impl Interpolator {
    /// Builds a polyline the way `LerpPolyline.parse` sorts it.
    pub fn polyline(raw: &[(f32, f32)]) -> Interpolator {
        let mut pts = raw.to_vec();
        let (min_x, max_x) = selection_sort(&mut pts, |p| p.0);
        Interpolator::Polyline { points: pts, min_x, max_x }
    }

    /// Builds a sequence the way `LerpSequence.parse` sorts it.
    pub fn sequence(raw: &[(f32, f32, LerpType)]) -> Interpolator {
        let mut e = raw.to_vec();
        let (min_x, max_x) = selection_sort(&mut e, |p| p.0);
        Interpolator::Sequence { entries: e, min_x, max_x }
    }

    pub fn lerp(&self, alpha: f32) -> f32 {
        match self {
            Interpolator::EaseInQuad { y0, y1 } => pzmath::lerp(*y0, *y1, pzmath::lerp_func_ease_in_quad(alpha)),
            Interpolator::EaseOutInQuad { y0, y1 } => pzmath::lerp(*y0, *y1, pzmath::lerp_func_ease_out_in_quad(alpha)),
            Interpolator::EaseOutQuad { y0, y1 } => pzmath::lerp(*y0, *y1, pzmath::lerp_func_ease_out_quad(alpha)),
            Interpolator::Linear { y0, y1 } => pzmath::lerp(*y0, *y1, alpha),
            Interpolator::One => 1.0,
            Interpolator::Zero => 0.0,
            Interpolator::Polyline { points, min_x, max_x } => {
                let n = points.len();
                if n == 0 {
                    return 0.0;
                }
                if alpha <= *min_x {
                    return points[0].1;
                }
                if alpha >= *max_x {
                    return points[n - 1].1;
                }
                for i in 1..n {
                    let px = points[i].0;
                    if px < alpha {
                        continue;
                    }
                    let prev_x = points[i - 1].0;
                    if prev_x == px {
                        continue;
                    }
                    let la = (alpha - prev_x) / (px - prev_x);
                    return pzmath::lerp(points[i - 1].1, points[i].1, la);
                }
                points[n - 1].1
            }
            Interpolator::Sequence { entries, min_x, max_x } => {
                let n = entries.len();
                if n == 0 {
                    return 0.0;
                }
                if alpha <= *min_x {
                    return entries[0].1;
                }
                if alpha >= *max_x {
                    return entries[n - 1].1;
                }
                for i in 1..n {
                    let (px, py, ty) = entries[i];
                    if px < alpha {
                        continue;
                    }
                    let (prev_x, prev_y, _) = entries[i - 1];
                    if prev_x == px {
                        continue;
                    }
                    let la = (alpha - prev_x) / (px - prev_x);
                    return ty.lerp(prev_y, py, la);
                }
                entries[n - 1].1
            }
        }
    }
}

/// The original's selection pass (it swaps with the last smaller element
/// found, not the smallest) and the min/max it tracks along the way.
fn selection_sort<T: Copy>(v: &mut [T], key: impl Fn(&T) -> f32) -> (f32, f32) {
    let mut min_x = f32::MAX;
    // Java's Float.MIN_VALUE: the smallest positive subnormal.
    let mut max_x = f32::from_bits(1);
    for i in 0..v.len() {
        let mut smallest = i;
        for o in i + 1..v.len() {
            if key(&v[o]) < key(&v[i]) {
                smallest = o;
            }
        }
        if smallest != i {
            v.swap(i, smallest);
        }
        min_x = pzmath::min(key(&v[i]), min_x);
        max_x = pzmath::max(key(&v[i]), max_x);
    }
    (min_x, max_x)
}
