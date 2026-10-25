//! The `org.joml` operations `BaseVehicle.updateTransform` uses that
//! pz-iso-render's [`Matrix4f`] does not have, with JOML's float
//! arithmetic.

use pz_iso_render::joml::Matrix4f;
use pz_vehicles::jmath::Quaternionf;

/// `Matrix4f.translationRotateScale(tx, ty, tz, qx, qy, qz, qw, sx, sy, sz)`.
#[allow(clippy::too_many_arguments)]
pub fn translation_rotate_scale(
    t: [f32; 3],
    q: Quaternionf,
    sx: f32,
    sy: f32,
    sz: f32,
) -> Matrix4f {
    let (qx, qy, qz, qw) = (q.x, q.y, q.z, q.w);
    let dqx = qx + qx;
    let dqy = qy + qy;
    let dqz = qz + qz;
    let q00 = dqx * qx;
    let q11 = dqy * qy;
    let q22 = dqz * qz;
    let q01 = dqx * qy;
    let q02 = dqx * qz;
    let q03 = dqx * qw;
    let q12 = dqy * qz;
    let q13 = dqy * qw;
    let q23 = dqz * qw;
    Matrix4f::from_cols([
        [
            sx - (q11 + q22) * sx,
            (q01 + q23) * sx,
            (q02 - q13) * sx,
            0.0,
        ],
        [
            (q01 - q23) * sy,
            sy - (q22 + q00) * sy,
            (q12 + q03) * sy,
            0.0,
        ],
        [
            (q02 + q13) * sz,
            (q12 - q03) * sz,
            sz - (q11 + q00) * sz,
            0.0,
        ],
        [t[0], t[1], t[2], 1.0],
    ])
}

/// `Quaternionf.get(Matrix4f)` (`Matrix4f.rotation(q)`).
pub fn rotation(q: Quaternionf) -> Matrix4f {
    let w2 = q.w * q.w;
    let x2 = q.x * q.x;
    let y2 = q.y * q.y;
    let z2 = q.z * q.z;
    let zw = q.z * q.w;
    let dzw = zw + zw;
    let xy = q.x * q.y;
    let dxy = xy + xy;
    let xz = q.x * q.z;
    let dxz = xz + xz;
    let yw = q.y * q.w;
    let dyw = yw + yw;
    let yz = q.y * q.z;
    let dyz = yz + yz;
    let xw = q.x * q.w;
    let dxw = xw + xw;
    Matrix4f::from_cols([
        [w2 + x2 - z2 - y2, dxy + dzw, dxz - dyw, 0.0],
        [-dzw + dxy, y2 - z2 + w2 - x2, dyz + dxw, 0.0],
        [dyw + dxz, dyz - dxw, z2 - y2 - x2 + w2, 0.0],
        [0.0, 0.0, 0.0, 1.0],
    ])
}

/// `ModelInstanceRenderData.makeAttachmentTransform`:
/// `translation(offset).rotateXYZ(rotate°).scale(scale)`.
pub fn attachment_transform(offset: [f32; 3], rotate: [f32; 3], scale: f32) -> Matrix4f {
    let d = std::f32::consts::PI / 180.0;
    Matrix4f::translation(offset[0], offset[1], offset[2])
        .rotate_xyz(rotate[0] * d, rotate[1] * d, rotate[2] * d)
        .scale1(scale)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_rotation_and_scale() {
        let m = translation_rotate_scale([1.0, 2.0, 3.0], Quaternionf::default(), 2.0, 2.0, 2.0);
        assert_eq!(m.m[0], [2.0, 0.0, 0.0, 0.0]);
        assert_eq!(m.m[3], [1.0, 2.0, 3.0, 1.0]);
        assert_eq!(rotation(Quaternionf::default()), Matrix4f::identity());
    }

    #[test]
    fn quarter_turn_about_y() {
        let q = Quaternionf::rotation_xyz(0.0, std::f32::consts::FRAC_PI_2, 0.0);
        let m = rotation(q);
        // x goes to -z.
        assert!((m.m[0][2] + 1.0).abs() < 1e-6, "{:?}", m.m);
    }
}
