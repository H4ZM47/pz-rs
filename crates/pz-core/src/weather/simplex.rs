//! `zombie.iso.weather.SimplexNoise` (Stefan Gustavson's simplex noise in
//! doubles) and the permutation it shares with `erosion.utils.Noise2D`.

/// Ken Perlin's reference permutation (`SimplexNoise.p`, `Noise2D.perm`).
pub(crate) const PERM: [u8; 256] = [
    151, 160, 137, 91, 90, 15, 131, 13, 201, 95, 96, 53, 194, 233, 7, 225, 140, 36, 103, 30, 69, 142, 8, 99, 37, 240,
    21, 10, 23, 190, 6, 148, 247, 120, 234, 75, 0, 26, 197, 62, 94, 252, 219, 203, 117, 35, 11, 32, 57, 177, 33, 88,
    237, 149, 56, 87, 174, 20, 125, 136, 171, 168, 68, 175, 74, 165, 71, 134, 139, 48, 27, 166, 77, 146, 158, 231, 83,
    111, 229, 122, 60, 211, 133, 230, 220, 105, 92, 41, 55, 46, 245, 40, 244, 102, 143, 54, 65, 25, 63, 161, 1, 216,
    80, 73, 209, 76, 132, 187, 208, 89, 18, 169, 200, 196, 135, 130, 116, 188, 159, 86, 164, 100, 109, 198, 173, 186,
    3, 64, 52, 217, 226, 250, 124, 123, 5, 202, 38, 147, 118, 126, 255, 82, 85, 212, 207, 206, 59, 227, 47, 16, 58, 17,
    182, 189, 28, 42, 223, 183, 170, 213, 119, 248, 152, 2, 44, 154, 163, 70, 221, 153, 101, 155, 167, 43, 172, 9, 129,
    22, 39, 253, 19, 98, 108, 110, 79, 113, 224, 232, 178, 185, 112, 104, 218, 246, 97, 228, 251, 34, 242, 193, 238,
    210, 144, 12, 191, 179, 162, 241, 81, 51, 145, 235, 249, 14, 239, 107, 49, 192, 214, 31, 181, 199, 106, 157, 184,
    84, 204, 176, 115, 121, 50, 45, 127, 4, 150, 254, 138, 236, 205, 93, 222, 114, 67, 29, 24, 72, 243, 141, 128, 195,
    78, 66, 215, 61, 156, 180,
];

const GRAD3: [[f64; 3]; 12] = [
    [1.0, 1.0, 0.0],
    [-1.0, 1.0, 0.0],
    [1.0, -1.0, 0.0],
    [-1.0, -1.0, 0.0],
    [1.0, 0.0, 1.0],
    [-1.0, 0.0, 1.0],
    [1.0, 0.0, -1.0],
    [-1.0, 0.0, -1.0],
    [0.0, 1.0, 1.0],
    [0.0, -1.0, 1.0],
    [0.0, 1.0, -1.0],
    [0.0, -1.0, -1.0],
];

fn grad4(i: usize) -> [f64; 4] {
    // The 32 edges of a tesseract, in the original's order.
    const G: [[f64; 4]; 32] = [
        [0.0, 1.0, 1.0, 1.0],
        [0.0, 1.0, 1.0, -1.0],
        [0.0, 1.0, -1.0, 1.0],
        [0.0, 1.0, -1.0, -1.0],
        [0.0, -1.0, 1.0, 1.0],
        [0.0, -1.0, 1.0, -1.0],
        [0.0, -1.0, -1.0, 1.0],
        [0.0, -1.0, -1.0, -1.0],
        [1.0, 0.0, 1.0, 1.0],
        [1.0, 0.0, 1.0, -1.0],
        [1.0, 0.0, -1.0, 1.0],
        [1.0, 0.0, -1.0, -1.0],
        [-1.0, 0.0, 1.0, 1.0],
        [-1.0, 0.0, 1.0, -1.0],
        [-1.0, 0.0, -1.0, 1.0],
        [-1.0, 0.0, -1.0, -1.0],
        [1.0, 1.0, 0.0, 1.0],
        [1.0, 1.0, 0.0, -1.0],
        [1.0, -1.0, 0.0, 1.0],
        [1.0, -1.0, 0.0, -1.0],
        [-1.0, 1.0, 0.0, 1.0],
        [-1.0, 1.0, 0.0, -1.0],
        [-1.0, -1.0, 0.0, 1.0],
        [-1.0, -1.0, 0.0, -1.0],
        [1.0, 1.0, 1.0, 0.0],
        [1.0, 1.0, -1.0, 0.0],
        [1.0, -1.0, 1.0, 0.0],
        [1.0, -1.0, -1.0, 0.0],
        [-1.0, 1.0, 1.0, 0.0],
        [-1.0, 1.0, -1.0, 0.0],
        [-1.0, -1.0, 1.0, 0.0],
        [-1.0, -1.0, -1.0, 0.0],
    ];
    G[i]
}

fn perm(i: usize) -> usize {
    PERM[i & 0xFF] as usize
}

fn perm_mod12(i: usize) -> usize {
    perm(i) % 12
}

fn f2() -> f64 {
    0.5 * (3.0f64.sqrt() - 1.0)
}

fn g2() -> f64 {
    (3.0 - 3.0f64.sqrt()) / 6.0
}

fn f4() -> f64 {
    (5.0f64.sqrt() - 1.0) / 4.0
}

fn g4() -> f64 {
    (5.0 - 5.0f64.sqrt()) / 20.0
}

fn fastfloor(x: f64) -> i32 {
    let xi = x as i32;
    if x < xi as f64 {
        xi - 1
    } else {
        xi
    }
}

fn corner2(t: f64, g: usize, x: f64, y: f64) -> f64 {
    if t < 0.0 {
        0.0
    } else {
        let t = t * t;
        t * t * (GRAD3[g][0] * x + GRAD3[g][1] * y)
    }
}

fn corner3(t: f64, g: usize, x: f64, y: f64, z: f64) -> f64 {
    if t < 0.0 {
        0.0
    } else {
        let t = t * t;
        t * t * (GRAD3[g][0] * x + GRAD3[g][1] * y + GRAD3[g][2] * z)
    }
}

fn corner4(t: f64, g: usize, x: f64, y: f64, z: f64, w: f64) -> f64 {
    if t < 0.0 {
        0.0
    } else {
        let t = t * t;
        let g = grad4(g);
        t * t * (g[0] * x + g[1] * y + g[2] * z + g[3] * w)
    }
}

/// `SimplexNoise.noise(xin, yin)`.
pub fn noise2(xin: f64, yin: f64) -> f64 {
    let (f2, g2) = (f2(), g2());
    let s = (xin + yin) * f2;
    let i = fastfloor(xin + s);
    let j = fastfloor(yin + s);
    let t = (i.wrapping_add(j)) as f64 * g2;
    let x0 = xin - (i as f64 - t);
    let y0 = yin - (j as f64 - t);
    let (i1, j1) = if x0 > y0 { (1, 0) } else { (0, 1) };
    let x1 = x0 - i1 as f64 + g2;
    let y1 = y0 - j1 as f64 + g2;
    let x2 = x0 - 1.0 + 2.0 * g2;
    let y2 = y0 - 1.0 + 2.0 * g2;
    let ii = (i & 0xFF) as usize;
    let jj = (j & 0xFF) as usize;
    let gi0 = perm_mod12(ii + perm(jj));
    let gi1 = perm_mod12(ii + i1 + perm(jj + j1));
    let gi2 = perm_mod12(ii + 1 + perm(jj + 1));
    let n0 = corner2(0.5 - x0 * x0 - y0 * y0, gi0, x0, y0);
    let n1 = corner2(0.5 - x1 * x1 - y1 * y1, gi1, x1, y1);
    let n2 = corner2(0.5 - x2 * x2 - y2 * y2, gi2, x2, y2);
    70.0 * (n0 + n1 + n2)
}

/// `SimplexNoise.noise(xin, yin, zin)`.
pub fn noise3(xin: f64, yin: f64, zin: f64) -> f64 {
    const F3: f64 = 0.3333333333333333;
    const G3: f64 = 0.16666666666666666;
    let s = (xin + yin + zin) * F3;
    let i = fastfloor(xin + s);
    let j = fastfloor(yin + s);
    let k = fastfloor(zin + s);
    let t = (i.wrapping_add(j).wrapping_add(k)) as f64 * G3;
    let x0 = xin - (i as f64 - t);
    let y0 = yin - (j as f64 - t);
    let z0 = zin - (k as f64 - t);
    let (i1, j1, k1, i2, j2, k2) = if x0 >= y0 {
        if y0 >= z0 {
            (1, 0, 0, 1, 1, 0)
        } else if x0 >= z0 {
            (1, 0, 0, 1, 0, 1)
        } else {
            (0, 0, 1, 1, 0, 1)
        }
    } else if y0 < z0 {
        (0, 0, 1, 0, 1, 1)
    } else if x0 < z0 {
        (0, 1, 0, 0, 1, 1)
    } else {
        (0, 1, 0, 1, 1, 0)
    };
    let x1 = x0 - i1 as f64 + G3;
    let y1 = y0 - j1 as f64 + G3;
    let z1 = z0 - k1 as f64 + G3;
    let x2 = x0 - i2 as f64 + F3;
    let y2 = y0 - j2 as f64 + F3;
    let z2 = z0 - k2 as f64 + F3;
    let x3 = x0 - 1.0 + 0.5;
    let y3 = y0 - 1.0 + 0.5;
    let z3 = z0 - 1.0 + 0.5;
    let ii = (i & 0xFF) as usize;
    let jj = (j & 0xFF) as usize;
    let kk = (k & 0xFF) as usize;
    let gi0 = perm_mod12(ii + perm(jj + perm(kk)));
    let gi1 = perm_mod12(ii + i1 + perm(jj + j1 + perm(kk + k1)));
    let gi2 = perm_mod12(ii + i2 + perm(jj + j2 + perm(kk + k2)));
    let gi3 = perm_mod12(ii + 1 + perm(jj + 1 + perm(kk + 1)));
    let n0 = corner3(0.6 - x0 * x0 - y0 * y0 - z0 * z0, gi0, x0, y0, z0);
    let n1 = corner3(0.6 - x1 * x1 - y1 * y1 - z1 * z1, gi1, x1, y1, z1);
    let n2 = corner3(0.6 - x2 * x2 - y2 * y2 - z2 * z2, gi2, x2, y2, z2);
    let n3 = corner3(0.6 - x3 * x3 - y3 * y3 - z3 * z3, gi3, x3, y3, z3);
    32.0 * (n0 + n1 + n2 + n3)
}

/// `SimplexNoise.noise(x, y, z, w)`.
pub fn noise4(x: f64, y: f64, z: f64, w: f64) -> f64 {
    let (f4, g4) = (f4(), g4());
    let s = (x + y + z + w) * f4;
    let i = fastfloor(x + s);
    let j = fastfloor(y + s);
    let k = fastfloor(z + s);
    let l = fastfloor(w + s);
    let t = (i.wrapping_add(j).wrapping_add(k).wrapping_add(l)) as f64 * g4;
    let x0 = x - (i as f64 - t);
    let y0 = y - (j as f64 - t);
    let z0 = z - (k as f64 - t);
    let w0 = w - (l as f64 - t);
    let (mut rx, mut ry, mut rz, mut rw) = (0, 0, 0, 0);
    if x0 > y0 { rx += 1 } else { ry += 1 }
    if x0 > z0 { rx += 1 } else { rz += 1 }
    if x0 > w0 { rx += 1 } else { rw += 1 }
    if y0 > z0 { ry += 1 } else { rz += 1 }
    if y0 > w0 { ry += 1 } else { rw += 1 }
    if z0 > w0 { rz += 1 } else { rw += 1 }
    let b = |r: i32, n: i32| usize::from(r >= n);
    let (i1, j1, k1, l1) = (b(rx, 3), b(ry, 3), b(rz, 3), b(rw, 3));
    let (i2, j2, k2, l2) = (b(rx, 2), b(ry, 2), b(rz, 2), b(rw, 2));
    let (i3, j3, k3, l3) = (b(rx, 1), b(ry, 1), b(rz, 1), b(rw, 1));
    let x1 = x0 - i1 as f64 + g4;
    let y1 = y0 - j1 as f64 + g4;
    let z1 = z0 - k1 as f64 + g4;
    let w1 = w0 - l1 as f64 + g4;
    let x2 = x0 - i2 as f64 + 2.0 * g4;
    let y2 = y0 - j2 as f64 + 2.0 * g4;
    let z2 = z0 - k2 as f64 + 2.0 * g4;
    let w2 = w0 - l2 as f64 + 2.0 * g4;
    let x3 = x0 - i3 as f64 + 3.0 * g4;
    let y3 = y0 - j3 as f64 + 3.0 * g4;
    let z3 = z0 - k3 as f64 + 3.0 * g4;
    let w3 = w0 - l3 as f64 + 3.0 * g4;
    let x4 = x0 - 1.0 + 4.0 * g4;
    let y4 = y0 - 1.0 + 4.0 * g4;
    let z4 = z0 - 1.0 + 4.0 * g4;
    let w4 = w0 - 1.0 + 4.0 * g4;
    let ii = (i & 0xFF) as usize;
    let jj = (j & 0xFF) as usize;
    let kk = (k & 0xFF) as usize;
    let ll = (l & 0xFF) as usize;
    let g = |a: usize, bb: usize, c: usize, d: usize| perm(ii + a + perm(jj + bb + perm(kk + c + perm(ll + d)))) % 32;
    let gi0 = g(0, 0, 0, 0);
    let gi1 = g(i1, j1, k1, l1);
    let gi2 = g(i2, j2, k2, l2);
    let gi3 = g(i3, j3, k3, l3);
    let gi4 = g(1, 1, 1, 1);
    let n0 = corner4(0.6 - x0 * x0 - y0 * y0 - z0 * z0 - w0 * w0, gi0, x0, y0, z0, w0);
    let n1 = corner4(0.6 - x1 * x1 - y1 * y1 - z1 * z1 - w1 * w1, gi1, x1, y1, z1, w1);
    let n2 = corner4(0.6 - x2 * x2 - y2 * y2 - z2 * z2 - w2 * w2, gi2, x2, y2, z2, w2);
    let n3 = corner4(0.6 - x3 * x3 - y3 * y3 - z3 * z3 - w3 * w3, gi3, x3, y3, z3, w3);
    let n4 = corner4(0.6 - x4 * x4 - y4 * y4 - z4 * z4 - w4 * w4, gi4, x4, y4, z4, w4);
    27.0 * (n0 + n1 + n2 + n3 + n4)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn noise_is_bounded_and_zero_at_lattice() {
        assert_eq!(noise2(0.0, 0.0), 0.0);
        for i in 0..200 {
            let x = i as f64 * 0.37 - 30.0;
            let v = noise2(x, 1234.5);
            assert!((-1.0..=1.0).contains(&v));
            assert!((-1.0..=1.0).contains(&noise3(x, -x, 0.5)));
            assert!((-1.0..=1.0).contains(&noise4(x, 0.25, -x, 7.0)));
        }
        assert_ne!(noise2(0.3, 0.7), noise2(0.7, 0.3));
    }
}
