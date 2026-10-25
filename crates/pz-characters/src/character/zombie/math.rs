//! The `Vector2`, `IsoUtils` and `PZMath` helpers the zombie code uses,
//! with the original's float/double steps.

/// `PZMath.fastfloor(float)`.
pub fn fastfloor(v: f32) -> i32 {
    let xi = v as i32;
    if v < xi as f32 {
        xi - 1
    } else {
        xi
    }
}

/// `Vector2.getLength()`.
pub fn length(x: f32, y: f32) -> f32 {
    ((x * x + y * y) as f64).sqrt() as f32
}

/// `Vector2.normalize()`: unit length (a length within 1e-5 of 1 is left
/// alone; zero stays zero).
pub fn normalize(x: f32, y: f32) -> (f32, f32) {
    let len_sq = x * x + y * y;
    if (1.0 - len_sq).abs() < 1.0e-5 {
        return (x, y);
    }
    if len_sq == 0.0 {
        return (0.0, 0.0);
    }
    let len = (len_sq as f64).sqrt() as f32;
    (x / len, y / len)
}

/// `Vector2.setLength(len)`.
pub fn set_length(x: f32, y: f32, len: f32) -> (f32, f32) {
    let (x, y) = normalize(x, y);
    (x * len, y * len)
}

/// `Vector2.setLengthAndDirection(dir, len)`.
pub fn from_direction(dir: f32, len: f32) -> (f32, f32) {
    (((dir as f64).cos() * len as f64) as f32, ((dir as f64).sin() * len as f64) as f32)
}

/// `Vector2.getDirection()` (`atan2` wrapped to `[-pi, pi]`).
pub fn direction(x: f32, y: f32) -> f32 {
    let a = (y as f64).atan2(x as f64) as f32;
    wrap(a, -std::f32::consts::PI, std::f32::consts::PI)
}

/// `Vector2.getDirectionNeg()`.
pub fn direction_neg(x: f32, y: f32) -> f32 {
    (x as f64).atan2(y as f64) as f32
}

/// `PZMath.frac`.
fn frac(v: f32) -> f32 {
    v - v.floor()
}

/// `PZMath.wrap(val, range)`.
pub fn wrap_range(val: f32, range: f32) -> f32 {
    if range <= 0.0 {
        return 0.0;
    }
    if val < 0.0 {
        let m = -val / range;
        let up = 1.0 - frac(m);
        return up * range;
    }
    let m = val / range;
    frac(m) * range
}

/// `PZMath.wrap(val, min, max)`.
pub fn wrap(val: f32, min: f32, max: f32) -> f32 {
    let real_max = if max > min { max } else { min };
    let real_min = if max < min { max } else { min };
    let range = real_max - real_min;
    real_min + wrap_range(val - real_min, range)
}

/// `IsoUtils.DistanceManhatten(fx, fy, tx, ty)`.
pub fn distance_manhatten(fx: f32, fy: f32, tx: f32, ty: f32) -> f32 {
    (tx - fx).abs() + (ty - fy).abs()
}

/// `IsoUtils.DistanceToSquared(fx, fy, tx, ty)`.
pub fn distance_to_squared(fx: f32, fy: f32, tx: f32, ty: f32) -> f32 {
    let dx = (tx - fx) as f64;
    let dy = (ty - fy) as f64;
    (dx * dx + dy * dy) as f32
}

/// `IsoUtils.DistanceTo(fx, fy, tx, ty)`.
pub fn distance_to(fx: f32, fy: f32, tx: f32, ty: f32) -> f32 {
    let dx = (tx - fx) as f64;
    let dy = (ty - fy) as f64;
    (dx * dx + dy * dy).sqrt() as f32
}

/// `PZMath.sqrt(float)`.
pub fn sqrt(v: f32) -> f32 {
    (v as f64).sqrt() as f32
}

/// `IsoMovingObject.getVectorFromDirection(dir)`.
pub fn vector_from_direction(dir: pz_world::square::IsoDirections) -> (f32, f32) {
    let (dx, dy) = dir.delta();
    normalize(dx as f32, dy as f32)
}

/// `IsoDirections.ToVector()` (the normalized `(dx, dy)`).
pub fn dir_vector(dir: pz_world::square::IsoDirections) -> (f32, f32) {
    vector_from_direction(dir)
}

/// `IsoDirections.RotLeft()`.
pub fn rot_left(dir: pz_world::square::IsoDirections) -> pz_world::square::IsoDirections {
    pz_world::square::IsoDirections::ALL[(dir as usize + 1) & 7]
}

/// `IsoDirections.RotRight()`.
pub fn rot_right(dir: pz_world::square::IsoDirections) -> pz_world::square::IsoDirections {
    pz_world::square::IsoDirections::ALL[(dir as usize).wrapping_sub(1) & 7]
}

/// JOML `Vector2f.normalize()` / `length()` as `IsoUtils.isSimilarDirection`
/// uses them.
fn joml_normalize(x: f32, y: f32) -> (f32, f32) {
    let inv = 1.0 / ((x * x + y * y) as f64).sqrt() as f32;
    (x * inv, y * inv)
}

/// `IsoUtils.isSimilarDirection(chr, xA, yA, xB, yB, similar)` with the
/// character at `(cx, cy)`.
pub fn is_similar_direction(cx: f32, cy: f32, xa: f32, ya: f32, xb: f32, yb: f32, similar: f32) -> bool {
    let (ax, ay) = joml_normalize(xa - cx, ya - cy);
    let (bx, by) = joml_normalize(cx - xb, cy - yb);
    let (sx, sy) = (ax + bx, ay + by);
    (((sx * sx + sy * sy) as f64).sqrt() as f32) < similar
}

/// `PZMath.lerp(a, b, t)`.
pub fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

/// `PZMath.lerp(a, b, t, LerpType.EaseOutQuad)`.
pub fn lerp_ease_out_quad(a: f32, b: f32, t: f32) -> f32 {
    let t = lerp_func_ease_out_quad(t);
    lerp(a, b, t)
}

/// `PZMath.lerpFunc_EaseOutQuad(t)` (the original's name; it squares).
pub fn lerp_func_ease_out_quad(t: f32) -> f32 {
    t * t
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vector_helpers() {
        assert_eq!(fastfloor(-0.5), -1);
        assert_eq!(fastfloor(2.9), 2);
        assert_eq!(normalize(3.0, 4.0), (0.6, 0.8));
        assert_eq!(normalize(0.0, 0.0), (0.0, 0.0));
        assert_eq!(rot_left(pz_world::square::IsoDirections::NE), pz_world::square::IsoDirections::N);
        assert_eq!(rot_right(pz_world::square::IsoDirections::N), pz_world::square::IsoDirections::NE);
        assert!(is_similar_direction(0.0, 0.0, 1.0, 0.0, 2.0, 0.1, 0.5));
        assert!(!is_similar_direction(0.0, 0.0, 1.0, 0.0, -2.0, 0.0, 0.5));
        assert!((wrap(4.0, -std::f32::consts::PI, std::f32::consts::PI) - (4.0 - 2.0 * std::f32::consts::PI)).abs() < 1e-6);
    }
}
