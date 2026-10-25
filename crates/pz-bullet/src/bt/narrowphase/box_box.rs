// Box-Box collision detection re-distributed under the ZLib license with permission from Russell L. Smith
// Original version is from Open Dynamics Engine, Copyright (C) 2001,2002 Russell L. Smith.
// All rights reserved.  Email: russ@q12.org   Web: www.q12.org
// Bullet Continuous Collision Detection and Physics Library
// Bullet is Copyright (c) 2003-2006 Erwin Coumans  http://continuousphysics.com/Bullet/
//
// This software is provided 'as-is', without any express or implied warranty.
// In no event will the authors be held liable for any damages arising from the use of this software.
// Permission is granted to anyone to use this software for any purpose,
// including commercial applications, and to alter it and redistribute it freely,
// subject to the following restrictions:
//
// 1. The origin of this software must not be misrepresented; you must not claim that you wrote the original software. If you use this software in a product, an acknowledgment in the product documentation would be appreciated but is not required.
// 2. Altered source versions must be plainly marked as such, and must not be misrepresented as being the original software.
// 3. This notice may not be removed or altered from any source distribution.
//
// Altered for pz-rs: Rust translation of btBoxBoxDetector (dBoxBox2,
// intersectRectQuad2, cullPoints2, dLineClosestApproach) from Bullet 2.82,
// double precision.

//! `btBoxBoxDetector`.

use super::gjk_pair_detector::{ClosestPointInput, DiscreteResult};
use crate::bt::linear_math::*;
use crate::bt::shapes::Shape;

/// `dDOTpq(a, b, p, q)`: `a[0]*b[0] + a[p]*b[q] + a[2p]*b[2q]`, fused.
#[inline(always)]
fn ddotpq(a: &[f64], b: &[f64], p: usize, q: usize) -> f64 {
    fma(a[2 * p], b[2 * q], fma(a[0], b[0], a[p] * b[q]))
}
#[inline(always)]
fn ddot(a: &[f64], b: &[f64]) -> f64 {
    ddotpq(a, b, 1, 1)
}
#[inline(always)]
fn ddot44(a: &[f64], b: &[f64]) -> f64 {
    ddotpq(a, b, 4, 4)
}
#[inline(always)]
fn ddot41(a: &[f64], b: &[f64]) -> f64 {
    ddotpq(a, b, 4, 1)
}
#[inline(always)]
fn ddot14(a: &[f64], b: &[f64]) -> f64 {
    ddotpq(a, b, 1, 4)
}

#[inline(always)]
fn v3(v: &Vec3) -> [f64; 3] {
    [v.x, v.y, v.z]
}

/// `dMULTIPLY1_331(A, B, C)`.
#[inline(always)]
fn dmultiply1_331(b: &[f64], c: &[f64]) -> [f64; 3] {
    [ddot41(b, c), ddot41(&b[1..], c), ddot41(&b[2..], c)]
}
/// `dMULTIPLY0_331(A, B, C)`.
#[inline(always)]
fn dmultiply0_331(b: &[f64], c: &[f64]) -> [f64; 3] {
    [ddot(b, c), ddot(&b[4..], c), ddot(&b[8..], c)]
}

/// `dLineClosestApproach`.
fn d_line_closest_approach(
    pa: &[f64; 3],
    ua: &[f64; 3],
    pb: &[f64; 3],
    ub: &[f64; 3],
) -> (f64, f64) {
    let p = [pb[0] - pa[0], pb[1] - pa[1], pb[2] - pa[2]];
    let uaub = ddot(ua, ub);
    let q1 = ddot(ua, &p);
    let q2 = -ddot(ub, &p);
    let mut d = fma(-uaub, uaub, 1.0);
    if d <= flt(0.0001) {
        (0.0, 0.0)
    } else {
        d = 1.0 / d;
        (fma(uaub, q2, q1) * d, fma(uaub, q1, q2) * d)
    }
}

/// `intersectRectQuad2(h, p, ret)`.
fn intersect_rect_quad2(h: &[f64; 2], p: &[f64; 8], ret: &mut [f64; 16]) -> usize {
    // q and r alternate between the input quad, `ret` and `buffer`. Model
    // them as three named storages: 0 = p (input), 1 = ret, 2 = buffer.
    let mut buffer = [0.0f64; 16];
    let mut input = [0.0f64; 16];
    input[..8].copy_from_slice(p);
    let mut nq: usize = 4;
    let mut nr: usize = 0;
    let mut q_store: u8 = 0;
    let mut r_store: u8 = 1;

    fn store<'a>(
        s: u8,
        input: &'a mut [f64; 16],
        ret: &'a mut [f64; 16],
        buffer: &'a mut [f64; 16],
    ) -> &'a mut [f64; 16] {
        match s {
            0 => input,
            1 => ret,
            _ => buffer,
        }
    }

    'outer: for dir in 0..=1usize {
        let mut sign: i32 = -1;
        while sign <= 1 {
            let q: [f64; 16] = *store(q_store, &mut input, ret, &mut buffer);
            let mut r: [f64; 16] = *store(r_store, &mut input, ret, &mut buffer);
            let mut pq = 0usize;
            let mut pr = 0usize;
            nr = 0;
            let s = sign as f64;
            let mut done = false;
            let mut i = nq;
            while i > 0 {
                if s * q[pq + dir] < h[dir] {
                    r[pr] = q[pq];
                    r[pr + 1] = q[pq + 1];
                    pr += 2;
                    nr += 1;
                    if nr & 8 != 0 {
                        done = true;
                        break;
                    }
                }
                let nextq = if i > 1 { pq + 2 } else { 0 };
                if (s * q[pq + dir] < h[dir]) ^ (s * q[nextq + dir] < h[dir]) {
                    r[pr + 1 - dir] = fma(
                        (q[nextq + 1 - dir] - q[pq + 1 - dir]) / (q[nextq + dir] - q[pq + dir]),
                        fma(s, h[dir], -q[pq + dir]),
                        q[pq + 1 - dir],
                    );
                    r[pr + dir] = s * h[dir];
                    pr += 2;
                    nr += 1;
                    if nr & 8 != 0 {
                        done = true;
                        break;
                    }
                }
                pq += 2;
                i -= 1;
            }
            *store(r_store, &mut input, ret, &mut buffer) = r;
            q_store = r_store;
            if done {
                break 'outer;
            }
            r_store = if q_store == 1 { 2 } else { 1 };
            nq = nr;
            sign += 2;
        }
    }
    if q_store != 1 {
        let q = *store(q_store, &mut input, ret, &mut buffer);
        ret[..nr * 2].copy_from_slice(&q[..nr * 2]);
    }
    nr
}

/// `M__PI` (`3.14159265f`).
#[allow(clippy::approx_constant, clippy::excessive_precision)]
const M_PI_F: f32 = 3.14159265;

/// `cullPoints2(n, p, m, i0, iret)`.
#[allow(clippy::needless_late_init, clippy::explicit_counter_loop)]
fn cull_points2(n: usize, p: &[f64], m: usize, i0: usize, iret: &mut [usize; 8]) {
    let mut a;
    let cx;
    let cy;
    if n == 1 {
        cx = p[0];
        cy = p[1];
    } else if n == 2 {
        cx = 0.5 * (p[0] + p[2]);
        cy = 0.5 * (p[1] + p[3]);
    } else {
        a = 0.0;
        let mut cxs = 0.0;
        let mut cys = 0.0;
        for i in 0..n - 1 {
            let q = fma(p[i * 2], p[i * 2 + 3], -(p[i * 2 + 2] * p[i * 2 + 1]));
            a += q;
            cxs = fma(q, p[i * 2] + p[i * 2 + 2], cxs);
            cys = fma(q, p[i * 2 + 1] + p[i * 2 + 3], cys);
        }
        let q = fma(p[n * 2 - 2], p[1], -(p[0] * p[n * 2 - 1]));
        if (a + q).abs() > SIMD_EPSILON {
            a = 1.0 / (3.0 * (a + q));
        } else {
            a = BT_LARGE_FLOAT;
        }
        cx = a * fma(q, p[n * 2 - 2] + p[0], cxs);
        cy = a * fma(q, p[n * 2 - 1] + p[1], cys);
    }
    let mut aa = [0.0f64; 8];
    for i in 0..n {
        aa[i] = bt_atan2(p[i * 2 + 1] - cy, p[i * 2] - cx);
    }
    let mut avail = [true; 8];
    avail[i0] = false;
    iret[0] = i0;
    let mut k = 1;
    let two_pi_f = 2.0f32 * M_PI_F;
    for j in 1..m {
        // btScalar(j)*(2*M__PI/m) + A[i0]; 2*M__PI/m is float arithmetic.
        let step = (two_pi_f / m as f32) as f64;
        let mut a = fma(j as f64, step, aa[i0]);
        if a > M_PI_F as f64 {
            a -= two_pi_f as f64;
        }
        let mut maxdiff = 1e9;
        iret[k] = i0;
        for i in 0..n {
            if avail[i] {
                let mut diff = (aa[i] - a).abs();
                if diff > M_PI_F as f64 {
                    diff = two_pi_f as f64 - diff;
                }
                if diff < maxdiff {
                    maxdiff = diff;
                    iret[k] = i;
                }
            }
        }
        avail[iret[k]] = false;
        k += 1;
    }
}

/// `dBoxBox2`. `r1`/`r2` are `dMatrix3` (row-major 3x4). Returns the
/// number of contacts reported to `output`.
#[allow(clippy::too_many_arguments)]
fn d_box_box2(
    p1: &Vec3,
    r1: &[f64; 12],
    side1: &Vec3,
    p2: &Vec3,
    r2: &[f64; 12],
    side2: &Vec3,
    maxc_in: usize,
    output: &mut dyn DiscreteResult,
) -> usize {
    let fudge_factor = 1.05;
    let mut normal_c = [0.0f64; 3];
    let mut normal_r: Option<usize> = None; // column index into r1 or r2 (0..3 => r1, 3..6 => r2)
    let p = v3(&(*p2 - *p1));
    let pp = dmultiply1_331(r1, &p);
    let a = [side1.x * 0.5, side1.y * 0.5, side1.z * 0.5];
    let b = [side2.x * 0.5, side2.y * 0.5, side2.z * 0.5];

    let r11 = ddot44(&r1[0..], &r2[0..]);
    let r12 = ddot44(&r1[0..], &r2[1..]);
    let r13 = ddot44(&r1[0..], &r2[2..]);
    let r21 = ddot44(&r1[1..], &r2[0..]);
    let r22 = ddot44(&r1[1..], &r2[1..]);
    let r23 = ddot44(&r1[1..], &r2[2..]);
    let r31 = ddot44(&r1[2..], &r2[0..]);
    let r32 = ddot44(&r1[2..], &r2[1..]);
    let r33 = ddot44(&r1[2..], &r2[2..]);

    let mut q11 = r11.abs();
    let mut q12 = r12.abs();
    let mut q13 = r13.abs();
    let mut q21 = r21.abs();
    let mut q22 = r22.abs();
    let mut q23 = r23.abs();
    let mut q31 = r31.abs();
    let mut q32 = r32.abs();
    let mut q33 = r33.abs();

    let mut s = -(f32::MAX as f64);
    let mut invert_normal = false;
    let mut code = 0;

    macro_rules! tst {
        ($e1:expr, $e2:expr, $norm:expr, $cc:expr) => {{
            let e1: f64 = $e1;
            let s2 = e1.abs() - ($e2);
            if s2 > 0.0 {
                return 0;
            }
            if s2 > s {
                s = s2;
                normal_r = Some($norm);
                invert_normal = e1 < 0.0;
                code = $cc;
            }
        }};
    }

    // separating axis = u1, u2, u3
    tst!(
        pp[0],
        fma(b[2], q13, fma(b[1], q12, fma(b[0], q11, a[0]))),
        0,
        1
    );
    tst!(
        pp[1],
        fma(b[2], q23, fma(b[1], q22, fma(b[0], q21, a[1]))),
        1,
        2
    );
    tst!(
        pp[2],
        fma(b[2], q33, fma(b[1], q32, fma(b[0], q31, a[2]))),
        2,
        3
    );
    // separating axis = v1, v2, v3
    tst!(
        ddot41(&r2[0..], &p),
        fma(a[2], q31, fma(a[0], q11, a[1] * q21)) + b[0],
        3,
        4
    );
    tst!(
        ddot41(&r2[1..], &p),
        fma(a[2], q32, fma(a[0], q12, a[1] * q22)) + b[1],
        4,
        5
    );
    tst!(
        ddot41(&r2[2..], &p),
        fma(a[2], q33, fma(a[0], q13, a[1] * q23)) + b[2],
        5,
        6
    );

    macro_rules! tst2 {
        ($e1:expr, $e2:expr, $n1:expr, $n2:expr, $n3:expr, $cc:expr) => {{
            let e1: f64 = $e1;
            let mut s2 = e1.abs() - ($e2);
            if s2 > SIMD_EPSILON {
                return 0;
            }
            let (n1, n2, n3): (f64, f64, f64) = ($n1, $n2, $n3);
            let l = fma(n3, n3, fma(n1, n1, n2 * n2)).sqrt();
            if l > SIMD_EPSILON {
                s2 /= l;
                if s2 * fudge_factor > s {
                    s = s2;
                    normal_r = None;
                    normal_c = [n1 / l, n2 / l, n3 / l];
                    invert_normal = e1 < 0.0;
                    code = $cc;
                }
            }
        }};
    }

    let fudge2 = flt(1.0e-5);
    q11 += fudge2;
    q12 += fudge2;
    q13 += fudge2;
    q21 += fudge2;
    q22 += fudge2;
    q23 += fudge2;
    q31 += fudge2;
    q32 += fudge2;
    q33 += fudge2;

    // separating axis = u1 x (v1,v2,v3)
    tst2!(
        fma(pp[2], r21, -(pp[1] * r31)),
        fma(b[2], q12, fma(b[1], q13, fma(a[1], q31, a[2] * q21))),
        0.0,
        -r31,
        r21,
        7
    );
    tst2!(
        fma(pp[2], r22, -(pp[1] * r32)),
        fma(b[2], q11, fma(b[0], q13, fma(a[1], q32, a[2] * q22))),
        0.0,
        -r32,
        r22,
        8
    );
    tst2!(
        fma(pp[2], r23, -(pp[1] * r33)),
        fma(b[1], q11, fma(b[0], q12, fma(a[1], q33, a[2] * q23))),
        0.0,
        -r33,
        r23,
        9
    );
    // separating axis = u2 x (v1,v2,v3)
    tst2!(
        fma(pp[0], r31, -(pp[2] * r11)),
        fma(b[2], q22, fma(b[1], q23, fma(a[0], q31, a[2] * q11))),
        r31,
        0.0,
        -r11,
        10
    );
    tst2!(
        fma(pp[0], r32, -(pp[2] * r12)),
        fma(b[2], q21, fma(b[0], q23, fma(a[0], q32, a[2] * q12))),
        r32,
        0.0,
        -r12,
        11
    );
    tst2!(
        fma(pp[0], r33, -(pp[2] * r13)),
        fma(b[1], q21, fma(b[0], q22, fma(a[0], q33, a[2] * q13))),
        r33,
        0.0,
        -r13,
        12
    );
    // separating axis = u3 x (v1,v2,v3)
    tst2!(
        fma(pp[1], r11, -(pp[0] * r21)),
        fma(b[2], q32, fma(b[1], q33, fma(a[0], q21, a[1] * q11))),
        -r21,
        r11,
        0.0,
        13
    );
    tst2!(
        fma(pp[1], r12, -(pp[0] * r22)),
        fma(b[2], q31, fma(b[0], q33, fma(a[0], q22, a[1] * q12))),
        -r22,
        r12,
        0.0,
        14
    );
    tst2!(
        fma(pp[1], r13, -(pp[0] * r23)),
        fma(b[1], q31, fma(b[0], q32, fma(a[0], q23, a[1] * q13))),
        -r23,
        r13,
        0.0,
        15
    );

    if code == 0 {
        return 0;
    }

    let mut normal = match normal_r {
        Some(col) => {
            let (m, c) = if col < 3 { (r1, col) } else { (r2, col - 3) };
            [m[c], m[c + 4], m[c + 8]]
        }
        None => dmultiply0_331(r1, &normal_c),
    };
    if invert_normal {
        normal = [-normal[0], -normal[1], -normal[2]];
    }
    let depth = -s;
    let neg_normal = Vec3::new(-normal[0], -normal[1], -normal[2]);

    // compute contact point(s)
    if code > 6 {
        // an edge from box 1 touches an edge from box 2.
        let mut pa = v3(p1);
        for j in 0..3 {
            let sign = if ddot14(&normal, &r1[j..]) > 0.0 {
                1.0
            } else {
                -1.0
            };
            for i in 0..3 {
                pa[i] = fma(sign * a[j], r1[i * 4 + j], pa[i]);
            }
        }
        let mut pb = v3(p2);
        for j in 0..3 {
            let sign = if ddot14(&normal, &r2[j..]) > 0.0 {
                -1.0
            } else {
                1.0
            };
            for i in 0..3 {
                pb[i] = fma(sign * b[j], r2[i * 4 + j], pb[i]);
            }
        }
        let mut ua = [0.0; 3];
        let mut ub = [0.0; 3];
        for i in 0..3 {
            ua[i] = r1[(code - 7) / 3 + i * 4];
        }
        for i in 0..3 {
            ub[i] = r2[(code - 7) % 3 + i * 4];
        }
        let (alpha, beta) = d_line_closest_approach(&pa, &ua, &pb, &ub);
        for i in 0..3 {
            pa[i] = fma(ua[i], alpha, pa[i]);
        }
        for i in 0..3 {
            pb[i] = fma(ub[i], beta, pb[i]);
        }
        output.add_contact_point(&neg_normal, &Vec3::new(pb[0], pb[1], pb[2]), -depth);
        return 1;
    }

    // face contact
    let (ra, rb, pa, pb, sa, sb): (
        &[f64; 12],
        &[f64; 12],
        [f64; 3],
        [f64; 3],
        [f64; 3],
        [f64; 3],
    ) = if code <= 3 {
        (r1, r2, v3(p1), v3(p2), a, b)
    } else {
        (r2, r1, v3(p2), v3(p1), b, a)
    };

    let normal2 = if code <= 3 {
        normal
    } else {
        [-normal[0], -normal[1], -normal[2]]
    };
    let nr = dmultiply1_331(rb, &normal2);
    let anr = [nr[0].abs(), nr[1].abs(), nr[2].abs()];

    let (lanr, a1, a2);
    if anr[1] > anr[0] {
        if anr[1] > anr[2] {
            a1 = 0;
            lanr = 1;
            a2 = 2;
        } else {
            a1 = 0;
            a2 = 1;
            lanr = 2;
        }
    } else if anr[0] > anr[2] {
        lanr = 0;
        a1 = 1;
        a2 = 2;
    } else {
        a1 = 0;
        a2 = 1;
        lanr = 2;
    }

    let mut center = [0.0f64; 3];
    if nr[lanr] < 0.0 {
        for i in 0..3 {
            center[i] = fma(sb[lanr], rb[i * 4 + lanr], pb[i] - pa[i]);
        }
    } else {
        for i in 0..3 {
            center[i] = fma(-sb[lanr], rb[i * 4 + lanr], pb[i] - pa[i]);
        }
    }

    let code_n = if code <= 3 { code - 1 } else { code - 4 };
    let (code1, code2) = match code_n {
        0 => (1, 2),
        1 => (0, 2),
        _ => (0, 1),
    };

    let c1 = ddot14(&center, &ra[code1..]);
    let c2 = ddot14(&center, &ra[code2..]);
    let mut m11 = ddot44(&ra[code1..], &rb[a1..]);
    let mut m12 = ddot44(&ra[code1..], &rb[a2..]);
    let mut m21 = ddot44(&ra[code2..], &rb[a1..]);
    let mut m22 = ddot44(&ra[code2..], &rb[a2..]);
    let mut quad = [0.0f64; 8];
    {
        let k1 = m11 * sb[a1];
        let k2 = m21 * sb[a1];
        let k3 = m12 * sb[a2];
        let k4 = m22 * sb[a2];
        quad[0] = c1 - k1 - k3;
        quad[1] = c2 - k2 - k4;
        quad[2] = c1 - k1 + k3;
        quad[3] = c2 - k2 + k4;
        quad[4] = c1 + k1 + k3;
        quad[5] = c2 + k2 + k4;
        quad[6] = c1 + k1 - k3;
        quad[7] = c2 + k2 - k4;
    }

    let rect = [sa[code1], sa[code2]];
    let mut ret = [0.0f64; 16];
    let n = intersect_rect_quad2(&rect, &quad, &mut ret);
    if n < 1 {
        return 0;
    }

    let mut point = [0.0f64; 3 * 8];
    let mut dep = [0.0f64; 8];
    let det1 = 1.0 / fma(m11, m22, -(m12 * m21));
    m11 *= det1;
    m12 *= det1;
    m21 *= det1;
    m22 *= det1;
    let mut cnum = 0usize;
    for j in 0..n {
        let k1 = fma(m22, ret[j * 2] - c1, -(m12 * (ret[j * 2 + 1] - c2)));
        let k2 = fma(-m21, ret[j * 2] - c1, m11 * (ret[j * 2 + 1] - c2));
        for i in 0..3 {
            point[cnum * 3 + i] = fma(k2, rb[i * 4 + a2], fma(k1, rb[i * 4 + a1], center[i]));
        }
        dep[cnum] = sa[code_n] - ddot(&normal2, &point[cnum * 3..]);
        if dep[cnum] >= 0.0 {
            ret[cnum * 2] = ret[j * 2];
            ret[cnum * 2 + 1] = ret[j * 2 + 1];
            cnum += 1;
        }
    }
    if cnum < 1 {
        return 0;
    }

    let mut maxc = maxc_in;
    if maxc > cnum {
        maxc = cnum;
    }
    if maxc < 1 {
        maxc = 1;
    }

    if cnum <= maxc {
        if code < 4 {
            for j in 0..cnum {
                let mut piw = Vec3::ZERO;
                for i in 0..3 {
                    piw[i] = point[j * 3 + i] + pa[i];
                }
                output.add_contact_point(&neg_normal, &piw, -dep[j]);
            }
        } else {
            for j in 0..cnum {
                let mut piw = Vec3::ZERO;
                for i in 0..3 {
                    piw[i] = fma(-normal[i], dep[j], point[j * 3 + i] + pa[i]);
                }
                output.add_contact_point(&neg_normal, &piw, -dep[j]);
            }
        }
    } else {
        let mut i1 = 0;
        let mut maxdepth = dep[0];
        for i in 1..cnum {
            if dep[i] > maxdepth {
                maxdepth = dep[i];
                i1 = i;
            }
        }
        let mut iret = [0usize; 8];
        cull_points2(cnum, &ret, maxc, i1, &mut iret);
        for &ij in iret.iter().take(maxc) {
            let mut pos_in_world = Vec3::ZERO;
            for i in 0..3 {
                pos_in_world[i] = point[ij * 3 + i] + pa[i];
            }
            if code < 4 {
                output.add_contact_point(&neg_normal, &pos_in_world, -dep[ij]);
            } else {
                let nv = Vec3::new(normal[0], normal[1], normal[2]);
                output.add_contact_point(&neg_normal, &(pos_in_world - nv * dep[ij]), -dep[ij]);
            }
        }
        cnum = maxc;
    }
    cnum
}

/// `btBoxBoxDetector::getClosestPoints`.
pub fn box_box_get_closest_points(
    box1: &Shape,
    box2: &Shape,
    input: &ClosestPointInput,
    output: &mut dyn DiscreteResult,
) {
    let transform_a = &input.transform_a;
    let transform_b = &input.transform_b;
    let mut r1 = [0.0f64; 12];
    let mut r2 = [0.0f64; 12];
    for j in 0..3 {
        r1[4 * j] = transform_a.basis.el[j].x;
        r2[4 * j] = transform_b.basis.el[j].x;
        r1[1 + 4 * j] = transform_a.basis.el[j].y;
        r2[1 + 4 * j] = transform_b.basis.el[j].y;
        r1[2 + 4 * j] = transform_a.basis.el[j].z;
        r2[2 + 4 * j] = transform_b.basis.el[j].z;
    }
    let maxc = 4;
    d_box_box2(
        &transform_a.origin,
        &r1,
        &(2.0 * box1.box_half_extents_with_margin()),
        &transform_b.origin,
        &r2,
        &(2.0 * box2.box_half_extents_with_margin()),
        maxc,
        output,
    );
}
