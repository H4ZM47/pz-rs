//! `zombie.core.ColorMixer.LerpLCH` and `Color.interp`, which fluid colour
//! mixing uses. Colours are `[r, g, b, a]`; the float and double steps
//! follow the original's expression order.

/// An `[r, g, b, a]` colour (`zombie.core.Color`).
pub type Rgba = [f32; 4];

/// `Color.interp(to, delta, dest)`.
pub fn interp(from: Rgba, to: Rgba, delta: f32) -> Rgba {
    let mut d = [0.0; 4];
    for i in 0..4 {
        let diff = to[i] - from[i];
        d[i] = from[i] + diff * delta;
    }
    d
}

fn to_linear(v: f32) -> f32 {
    if v > 0.04045 {
        ((v as f64 + 0.055) / 1.055).powf(2.4) as f32
    } else {
        v / 12.92
    }
}

/// `ColorMixer.ColorToXYZ`.
fn color_to_xyz(c: Rgba) -> [f32; 3] {
    let r = to_linear(c[0]) * 100.0;
    let g = to_linear(c[1]) * 100.0;
    let b = to_linear(c[2]) * 100.0;
    [
        r * 0.412453 + g * 0.35758 + b * 0.180423,
        r * 0.212671 + g * 0.71516 + b * 0.072169,
        r * 0.019334 + g * 0.119193 + b * 0.950227,
    ]
}

/// `ColorMixer.XYZToLab`.
fn xyz_to_lab(xyz: [f32; 3]) -> [f32; 3] {
    let f = |v: f32| {
        if v > 0.008856 {
            (v as f64).powf(0.3333333333333333) as f32
        } else {
            v * 7.787 + 0.13793103
        }
    };
    let x = f(xyz[0] / 95.047);
    let y = f(xyz[1] / 100.0);
    let z = f(xyz[2] / 108.883);
    [116.0 * y - 16.0, 500.0 * (x - y), 200.0 * (y - z)]
}

/// `ColorMixer.LabToLCH`.
fn lab_to_lch(lab: [f32; 3]) -> [f32; 3] {
    let c = ((lab[1] * lab[1] + lab[2] * lab[2]) as f64).sqrt() as f32;
    let h = (lab[2] as f64).atan2(lab[1] as f64) as f32;
    let mut h = (h as f64 * 57.29577951308232) as f32;
    if h < 0.0 {
        h += 360.0;
    }
    [lab[0], c, h]
}

/// `ColorMixer.LCHToLab`.
fn lch_to_lab(lch: [f32; 3]) -> [f32; 3] {
    let rad = lch[2] as f64 * (std::f64::consts::PI / 180.0);
    [lch[0], lch[1] * rad.cos() as f32, lch[1] * rad.sin() as f32]
}

/// `ColorMixer.LabToXYZ`.
fn lab_to_xyz(lab: [f32; 3]) -> [f32; 3] {
    let y = (lab[0] + 16.0) / 116.0;
    let mut v = [lab[1] / 500.0 + y, y, y - lab[2] / 200.0];
    for x in &mut v {
        let pow = *x * *x * *x;
        *x = if *x > 0.20689656 {
            pow
        } else {
            0.12841856 * (*x - 0.13793103)
        };
    }
    [v[0] * 95.047, v[1] * 100.0, v[2] * 108.883]
}

/// `ColorMixer.XYZToRGB` (alpha 1).
fn xyz_to_rgb(xyz: [f32; 3]) -> Rgba {
    let [x, y, z] = xyz.map(|v| v / 100.0);
    let mut rgb = [
        x * 3.240479 + y * -1.53715 + z * -0.498535,
        x * -0.969256 + y * 1.875992 + z * 0.041556,
        x * 0.055648 + y * -0.204043 + z * 1.057311,
    ];
    for v in &mut rgb {
        *v = if *v > 0.0031308 {
            1.055 * (*v as f64).powf(0.4166666567325592) as f32 - 0.055
        } else {
            *v * 12.92
        };
    }
    let [r, g, b] = rgb.map(|v| (v * 255.0).clamp(0.0, 255.0));
    [r / 255.0, g / 255.0, b / 255.0, 1.0]
}

/// `ColorMixer.LerpLCH(col1, col2, delta, dest)`: grey colours interpolate
/// in RGB, others along the shorter hue arc in LCH.
pub fn lerp_lch(col1: Rgba, col2: Rgba, delta: f32) -> Rgba {
    let grey = |c: Rgba| c[0] == c[1] && c[0] == c[2];
    if grey(col1) || grey(col2) {
        return interp(col1, col2, delta);
    }
    let mut l1 = lab_to_lch(xyz_to_lab(color_to_xyz(col1)));
    let mut l2 = lab_to_lch(xyz_to_lab(color_to_xyz(col2)));
    if l2[2] > l1[2] {
        if l2[2] - l1[2] > l1[2] + 360.0 - l2[2] {
            l1[2] += 360.0;
        }
    } else if l1[2] - l2[2] > l2[2] + 360.0 - l1[2] {
        l2[2] += 360.0;
    }
    let mut lch = [0.0; 3];
    for i in 0..3 {
        lch[i] = l2[i] * delta + l1[i] * (1.0 - delta);
    }
    if lch[2] > 360.0 {
        lch[2] -= 360.0;
    }
    xyz_to_rgb(lab_to_xyz(lch_to_lab(lch)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grey_interpolates_in_rgb() {
        let c = lerp_lch([0.5, 0.5, 0.5, 1.0], [1.0, 0.0, 0.0, 1.0], 0.5);
        assert_eq!(c, [0.75, 0.25, 0.25, 1.0]);
    }

    #[test]
    fn same_colour_stays_close() {
        let c = [0.8, 0.2, 0.1, 1.0];
        let m = lerp_lch(c, c, 0.5);
        for i in 0..3 {
            assert!((m[i] - c[i]).abs() < 1.0e-3, "{m:?}");
        }
        assert_eq!(m[3], 1.0);
    }

    #[test]
    fn hue_takes_the_short_way() {
        // Red and magenta-ish blue: the mix stays on the red/purple side.
        let m = lerp_lch([1.0, 0.0, 0.1, 1.0], [0.6, 0.0, 1.0, 1.0], 0.5);
        assert!(m[1] < 0.2 && m[0] > 0.5 && m[2] > 0.3, "{m:?}");
    }
}
