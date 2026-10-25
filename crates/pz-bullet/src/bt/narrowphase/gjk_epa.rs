// Bullet Continuous Collision Detection and Physics Library
// Copyright (c) 2003-2008 Erwin Coumans  http://continuousphysics.com/Bullet/
//
// This software is provided 'as-is', without any express or implied warranty.
// In no event will the authors be held liable for any damages arising from the
// use of this software.
// Permission is granted to anyone to use this software for any purpose,
// including commercial applications, and to alter it and redistribute it
// freely, subject to the following restrictions:
//
// 1. The origin of this software must not be misrepresented; you must not
// claim that you wrote the original software. If you use this software in a
// product, an acknowledgment in the product documentation would be appreciated
// but is not required.
// 2. Altered source versions must be plainly marked as such, and must not be
// misrepresented as being the original software.
// 3. This notice may not be removed or altered from any source distribution.
//
// GJK-EPA collision solver by Nathanael Presson, 2008
//
// Altered for pz-rs: Rust translation of btGjkEpa2 (gjkepa2_impl) and
// btGjkEpaPenetrationDepthSolver from Bullet 2.82, double precision. The
// pointer graph of support vertices and faces is kept in index arenas.

//! `btGjkEpaSolver2` and `btGjkEpaPenetrationDepthSolver`.

use crate::bt::linear_math::*;
use crate::bt::shapes::Shape;

const GJK_MAX_ITERATIONS: u32 = 128;
const GJK_ACCURARY: f64 = 0.0001;
const GJK_MIN_DISTANCE: f64 = 0.0001;
const GJK_DUPLICATED_EPS: f64 = 0.0001;
const GJK_SIMPLEX2_EPS: f64 = 0.0;
const GJK_SIMPLEX3_EPS: f64 = 0.0;
const GJK_SIMPLEX4_EPS: f64 = 0.0;
const EPA_MAX_VERTICES: usize = 64;
const EPA_MAX_FACES: usize = EPA_MAX_VERTICES * 2;
const EPA_MAX_ITERATIONS: u32 = 255;
const EPA_ACCURACY: f64 = 0.0001;
const EPA_PLANE_EPS: f64 = 0.00001;

/// `btGjkEpaSolver2::sResults::eStatus`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GjkEpaStatus {
    Separated,
    Penetrating,
    GjkFailed,
    EpaFailed,
}

/// `btGjkEpaSolver2::sResults`.
#[derive(Clone, Copy, Debug)]
pub struct GjkEpaResults {
    pub status: GjkEpaStatus,
    pub witnesses: [Vec3; 2],
    pub normal: Vec3,
    pub distance: f64,
}

impl Default for GjkEpaResults {
    fn default() -> Self {
        GjkEpaResults {
            status: GjkEpaStatus::Separated,
            witnesses: [Vec3::ZERO; 2],
            normal: Vec3::ZERO,
            distance: 0.0,
        }
    }
}

/// `MinkowskiDiff`.
struct MinkowskiDiff<'a> {
    shapes: [&'a Shape; 2],
    toshape1: Mat3,
    toshape0: Transform,
    with_margin: bool,
}

impl MinkowskiDiff<'_> {
    #[inline(always)]
    fn ls(&self, s: &Shape, d: &Vec3) -> Vec3 {
        if self.with_margin {
            s.local_get_support_vertex_non_virtual(d)
        } else {
            s.local_get_support_vertex_without_margin_non_virtual(d)
        }
    }
    #[inline(always)]
    fn support0(&self, d: &Vec3) -> Vec3 {
        self.ls(self.shapes[0], d)
    }
    #[inline(always)]
    fn support1(&self, d: &Vec3) -> Vec3 {
        self.toshape0
            .apply(&self.ls(self.shapes[1], &(self.toshape1 * *d)))
    }
    #[inline(always)]
    fn support(&self, d: &Vec3) -> Vec3 {
        self.support0(d) - self.support1(&-*d)
    }
    #[inline(always)]
    fn support_index(&self, d: &Vec3, index: u32) -> Vec3 {
        if index != 0 {
            self.support1(d)
        } else {
            self.support0(d)
        }
    }
}

/// `GJK::sSV`.
#[derive(Clone, Copy, Debug, Default)]
struct Sv {
    d: Vec3,
    w: Vec3,
}

/// Support vertices of GJK (0..4) and EPA (4..68) share one arena so EPA
/// results may point into either, as the C++ pointers do.
const SV_GJK: usize = 0;
const SV_EPA: usize = 4;

/// `GJK::sSimplex`.
#[derive(Clone, Copy, Debug, Default)]
struct Simplex {
    c: [usize; 4],
    p: [f64; 4],
    rank: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum GjkStatus {
    Valid,
    Inside,
    Failed,
}

/// `det(a, b, c)`.
#[inline(always)]
fn det(a: &Vec3, b: &Vec3, c: &Vec3) -> f64 {
    let s = fma(a.y * b.z, c.x, (a.z * b.x) * c.y);
    let s = fma(-(a.x * b.z), c.y, s);
    let s = fma(-(a.y * b.x), c.z, s);
    let s = fma(a.x * b.y, c.z, s);
    fma(-(a.z * b.y), c.x, s)
}

struct Gjk<'a> {
    shape: MinkowskiDiff<'a>,
    ray: Vec3,
    distance: f64,
    simplices: [Simplex; 2],
    free: [usize; 4],
    nfree: u32,
    current: u32,
    simplex: u32,
    status: GjkStatus,
}

impl<'a> Gjk<'a> {
    fn new(shape: MinkowskiDiff<'a>) -> Gjk<'a> {
        Gjk {
            shape,
            ray: Vec3::ZERO,
            distance: 0.0,
            simplices: [Simplex::default(); 2],
            free: [0; 4],
            nfree: 0,
            current: 0,
            simplex: 0,
            status: GjkStatus::Failed,
        }
    }

    fn getsupport(&self, svs: &mut [Sv], d: &Vec3, sv: usize) {
        let dd = *d / d.length();
        svs[sv].d = dd;
        svs[sv].w = self.shape.support(&dd);
    }
    fn removevertice(&mut self, si: usize) {
        let s = &mut self.simplices[si];
        s.rank -= 1;
        self.free[self.nfree as usize] = s.c[s.rank as usize];
        self.nfree += 1;
    }
    fn appendvertice(&mut self, svs: &mut [Sv], si: usize, v: &Vec3) {
        let r = self.simplices[si].rank as usize;
        self.simplices[si].p[r] = 0.0;
        self.nfree -= 1;
        let sv = self.free[self.nfree as usize];
        self.simplices[si].c[r] = sv;
        self.simplices[si].rank += 1;
        self.getsupport(svs, v, sv);
    }

    fn evaluate(&mut self, svs: &mut [Sv], guess: &Vec3) -> GjkStatus {
        let mut iterations = 0u32;
        let mut sqdist;
        let mut alpha = 0.0;
        let mut lastw = [Vec3::ZERO; 4];
        let mut clastw = 0u32;
        for i in 0..4 {
            self.free[i] = SV_GJK + i;
        }
        self.nfree = 4;
        self.current = 0;
        self.status = GjkStatus::Valid;
        self.distance = 0.0;
        self.simplices[0].rank = 0;
        self.ray = *guess;
        let sqrl = self.ray.length2();
        let first = if sqrl > 0.0 {
            -self.ray
        } else {
            Vec3::new(1.0, 0.0, 0.0)
        };
        self.appendvertice(svs, 0, &first);
        self.simplices[0].p[0] = 1.0;
        self.ray = svs[self.simplices[0].c[0]].w;
        sqdist = sqrl;
        let _ = sqdist;
        lastw[0] = self.ray;
        lastw[1] = self.ray;
        lastw[2] = self.ray;
        lastw[3] = self.ray;
        loop {
            let next = 1 - self.current;
            let cur = self.current as usize;
            let rl = self.ray.length();
            if rl < GJK_MIN_DISTANCE {
                self.status = GjkStatus::Inside;
                break;
            }
            let neg_ray = -self.ray;
            self.appendvertice(svs, cur, &neg_ray);
            let w = svs[self.simplices[cur].c[self.simplices[cur].rank as usize - 1]].w;
            let mut found = false;
            for lw in &lastw {
                if (w - *lw).length2() < GJK_DUPLICATED_EPS {
                    found = true;
                    break;
                }
            }
            if found {
                self.removevertice(cur);
                break;
            } else {
                clastw = (clastw + 1) & 3;
                lastw[clastw as usize] = w;
            }
            let omega = self.ray.dot(&w) / rl;
            alpha = bt_max(omega, alpha);
            if fma(-GJK_ACCURARY, rl, rl - alpha) <= 0.0 {
                self.removevertice(cur);
                break;
            }
            let mut weights = [0.0f64; 4];
            let mut mask = 0u32;
            let cs = self.simplices[cur];
            sqdist = match cs.rank {
                2 => projectorigin2(&svs[cs.c[0]].w, &svs[cs.c[1]].w, &mut weights, &mut mask),
                3 => projectorigin3(
                    &svs[cs.c[0]].w,
                    &svs[cs.c[1]].w,
                    &svs[cs.c[2]].w,
                    &mut weights,
                    &mut mask,
                ),
                4 => projectorigin4(
                    &svs[cs.c[0]].w,
                    &svs[cs.c[1]].w,
                    &svs[cs.c[2]].w,
                    &svs[cs.c[3]].w,
                    &mut weights,
                    &mut mask,
                ),
                _ => 0.0,
            };
            if sqdist >= 0.0 {
                let ni = next as usize;
                self.simplices[ni].rank = 0;
                self.ray = Vec3::ZERO;
                self.current = next;
                for i in 0..cs.rank as usize {
                    if mask & (1 << i) != 0 {
                        let r = self.simplices[ni].rank as usize;
                        self.simplices[ni].c[r] = cs.c[i];
                        self.simplices[ni].p[r] = weights[i];
                        self.simplices[ni].rank += 1;
                        self.ray += svs[cs.c[i]].w * weights[i];
                    } else {
                        self.free[self.nfree as usize] = cs.c[i];
                        self.nfree += 1;
                    }
                }
                if mask == 15 {
                    self.status = GjkStatus::Inside;
                }
            } else {
                self.removevertice(cur);
                break;
            }
            iterations += 1;
            if iterations >= GJK_MAX_ITERATIONS {
                self.status = GjkStatus::Failed;
            }
            if self.status != GjkStatus::Valid {
                break;
            }
        }
        self.simplex = self.current;
        match self.status {
            GjkStatus::Valid => self.distance = self.ray.length(),
            GjkStatus::Inside => self.distance = 0.0,
            _ => {}
        }
        self.status
    }

    fn encloseorigin(&mut self, svs: &mut [Sv]) -> bool {
        let si = self.simplex as usize;
        match self.simplices[si].rank {
            1 => {
                for i in 0..3 {
                    let mut axis = Vec3::ZERO;
                    axis[i] = 1.0;
                    self.appendvertice(svs, si, &axis);
                    if self.encloseorigin(svs) {
                        return true;
                    }
                    self.removevertice(si);
                    self.appendvertice(svs, si, &-axis);
                    if self.encloseorigin(svs) {
                        return true;
                    }
                    self.removevertice(si);
                }
            }
            2 => {
                let s = self.simplices[si];
                let d = svs[s.c[1]].w - svs[s.c[0]].w;
                for i in 0..3 {
                    let mut axis = Vec3::ZERO;
                    axis[i] = 1.0;
                    let p = d.cross(&axis);
                    if p.length2() > 0.0 {
                        self.appendvertice(svs, si, &p);
                        if self.encloseorigin(svs) {
                            return true;
                        }
                        self.removevertice(si);
                        self.appendvertice(svs, si, &-p);
                        if self.encloseorigin(svs) {
                            return true;
                        }
                        self.removevertice(si);
                    }
                }
            }
            3 => {
                let s = self.simplices[si];
                let n = (svs[s.c[1]].w - svs[s.c[0]].w).cross(&(svs[s.c[2]].w - svs[s.c[0]].w));
                if n.length2() > 0.0 {
                    self.appendvertice(svs, si, &n);
                    if self.encloseorigin(svs) {
                        return true;
                    }
                    self.removevertice(si);
                    self.appendvertice(svs, si, &-n);
                    if self.encloseorigin(svs) {
                        return true;
                    }
                    self.removevertice(si);
                }
            }
            4 => {
                let s = self.simplices[si];
                if det(
                    &(svs[s.c[0]].w - svs[s.c[3]].w),
                    &(svs[s.c[1]].w - svs[s.c[3]].w),
                    &(svs[s.c[2]].w - svs[s.c[3]].w),
                )
                .abs()
                    > 0.0
                {
                    return true;
                }
            }
            _ => {}
        }
        false
    }
}

/// `GJK::projectorigin(a, b, w, m)`.
fn projectorigin2(a: &Vec3, b: &Vec3, w: &mut [f64], m: &mut u32) -> f64 {
    let d = *b - *a;
    let l = d.length2();
    if l > GJK_SIMPLEX2_EPS {
        let t = if l > 0.0 { -a.dot(&d) / l } else { 0.0 };
        if t >= 1.0 {
            w[0] = 0.0;
            w[1] = 1.0;
            *m = 2;
            return b.length2();
        } else if t <= 0.0 {
            w[0] = 1.0;
            w[1] = 0.0;
            *m = 1;
            return a.length2();
        } else {
            w[1] = t;
            w[0] = 1.0 - t;
            *m = 3;
            return (*a + d * t).length2();
        }
    }
    -1.0
}

const IMD3: [usize; 3] = [1, 2, 0];

/// `GJK::projectorigin(a, b, c, w, m)`.
fn projectorigin3(a: &Vec3, b: &Vec3, c: &Vec3, w: &mut [f64], m: &mut u32) -> f64 {
    let vt = [a, b, c];
    let dl = [*a - *b, *b - *c, *c - *a];
    let n = dl[0].cross(&dl[1]);
    let l = n.length2();
    if l > GJK_SIMPLEX3_EPS {
        let mut mindist = -1.0;
        let mut subw = [0.0f64; 2];
        let mut subm = 0u32;
        for i in 0..3 {
            if vt[i].dot(&dl[i].cross(&n)) > 0.0 {
                let j = IMD3[i];
                let subd = projectorigin2(vt[i], vt[j], &mut subw, &mut subm);
                if mindist < 0.0 || subd < mindist {
                    mindist = subd;
                    *m = (if subm & 1 != 0 { 1 << i } else { 0 })
                        + (if subm & 2 != 0 { 1 << j } else { 0 });
                    w[i] = subw[0];
                    w[j] = subw[1];
                    w[IMD3[j]] = 0.0;
                }
            }
        }
        if mindist < 0.0 {
            let d = a.dot(&n);
            let s = l.sqrt();
            let p = n * (d / l);
            mindist = p.length2();
            *m = 7;
            w[0] = dl[1].cross(&(*b - p)).length() / s;
            w[1] = dl[2].cross(&(*c - p)).length() / s;
            w[2] = 1.0 - (w[0] + w[1]);
        }
        return mindist;
    }
    -1.0
}

/// `GJK::projectorigin(a, b, c, d, w, m)`.
fn projectorigin4(a: &Vec3, b: &Vec3, c: &Vec3, d: &Vec3, w: &mut [f64], m: &mut u32) -> f64 {
    let vt = [a, b, c, d];
    let dl = [*a - *d, *b - *d, *c - *d];
    let vl = det(&dl[0], &dl[1], &dl[2]);
    let ng = (vl * a.dot(&(*b - *c).cross(&(*a - *b)))) <= 0.0;
    if ng && vl.abs() > GJK_SIMPLEX4_EPS {
        let mut mindist = -1.0;
        let mut subw = [0.0f64; 3];
        let mut subm = 0u32;
        for i in 0..3 {
            let j = IMD3[i];
            let s = vl * d.dot(&dl[i].cross(&dl[j]));
            if s > 0.0 {
                let subd = projectorigin3(vt[i], vt[j], d, &mut subw, &mut subm);
                if mindist < 0.0 || subd < mindist {
                    mindist = subd;
                    *m = (if subm & 1 != 0 { 1 << i } else { 0 })
                        + (if subm & 2 != 0 { 1 << j } else { 0 })
                        + (if subm & 4 != 0 { 8 } else { 0 });
                    w[i] = subw[0];
                    w[j] = subw[1];
                    w[IMD3[j]] = 0.0;
                    w[3] = subw[2];
                }
            }
        }
        if mindist < 0.0 {
            mindist = 0.0;
            *m = 15;
            w[0] = det(c, b, d) / vl;
            w[1] = det(a, c, d) / vl;
            w[2] = det(b, a, d) / vl;
            w[3] = 1.0 - (w[0] + w[1] + w[2]);
        }
        return mindist;
    }
    -1.0
}

// ---------------------------------------------------------------------------
// EPA
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, Default)]
struct Face {
    n: Vec3,
    d: f64,
    c: [usize; 3],
    f: [usize; 3],
    l: [Option<usize>; 2],
    e: [u8; 3],
    pass: u8,
}

#[derive(Clone, Copy, Debug, Default)]
struct List {
    root: Option<usize>,
    count: u32,
}

#[derive(Clone, Copy, Debug, Default)]
struct Horizon {
    cf: Option<usize>,
    ff: Option<usize>,
    nf: u32,
}

#[allow(dead_code)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum EpaStatus {
    Valid,
    Touching,
    Degenerated,
    NonConvex,
    InvalidHull,
    OutOfFaces,
    OutOfVertices,
    AccuraryReached,
    FallBack,
    Failed,
}

struct Epa {
    status: EpaStatus,
    result: Simplex,
    normal: Vec3,
    depth: f64,
    fc_store: Vec<Face>,
    nextsv: usize,
    hull: List,
    stock: List,
}

impl Epa {
    fn new() -> Epa {
        let mut e = Epa {
            status: EpaStatus::Failed,
            result: Simplex::default(),
            normal: Vec3::ZERO,
            depth: 0.0,
            fc_store: vec![Face::default(); EPA_MAX_FACES],
            nextsv: 0,
            hull: List::default(),
            stock: List::default(),
        };
        for i in 0..EPA_MAX_FACES {
            let f = EPA_MAX_FACES - i - 1;
            let mut stock = e.stock;
            Self::append(&mut e.fc_store, &mut stock, f);
            e.stock = stock;
        }
        e
    }

    fn bind(fc: &mut [Face], fa: usize, ea: usize, fb: usize, eb: usize) {
        fc[fa].e[ea] = eb as u8;
        fc[fa].f[ea] = fb;
        fc[fb].e[eb] = ea as u8;
        fc[fb].f[eb] = fa;
    }
    fn append(fc: &mut [Face], list: &mut List, face: usize) {
        fc[face].l[0] = None;
        fc[face].l[1] = list.root;
        if let Some(r) = list.root {
            fc[r].l[0] = Some(face);
        }
        list.root = Some(face);
        list.count += 1;
    }
    fn remove(fc: &mut [Face], list: &mut List, face: usize) {
        if let Some(l1) = fc[face].l[1] {
            fc[l1].l[0] = fc[face].l[0];
        }
        if let Some(l0) = fc[face].l[0] {
            fc[l0].l[1] = fc[face].l[1];
        }
        if Some(face) == list.root {
            list.root = fc[face].l[1];
        }
        list.count -= 1;
    }
    fn hull_remove(&mut self, face: usize) {
        let mut h = self.hull;
        Self::remove(&mut self.fc_store, &mut h, face);
        self.hull = h;
    }
    fn hull_append(&mut self, face: usize) {
        let mut h = self.hull;
        Self::append(&mut self.fc_store, &mut h, face);
        self.hull = h;
    }
    fn stock_remove(&mut self, face: usize) {
        let mut s = self.stock;
        Self::remove(&mut self.fc_store, &mut s, face);
        self.stock = s;
    }
    fn stock_append(&mut self, face: usize) {
        let mut s = self.stock;
        Self::append(&mut self.fc_store, &mut s, face);
        self.stock = s;
    }

    fn evaluate(&mut self, gjk: &mut Gjk, svs: &mut [Sv], guess: &Vec3) -> EpaStatus {
        let si = gjk.simplex as usize;
        if gjk.simplices[si].rank > 1 && gjk.encloseorigin(svs) {
            while let Some(f) = self.hull.root {
                self.hull_remove(f);
                self.stock_append(f);
            }
            self.status = EpaStatus::Valid;
            self.nextsv = 0;
            {
                let s = &mut gjk.simplices[si];
                if det(
                    &(svs[s.c[0]].w - svs[s.c[3]].w),
                    &(svs[s.c[1]].w - svs[s.c[3]].w),
                    &(svs[s.c[2]].w - svs[s.c[3]].w),
                ) < 0.0
                {
                    s.c.swap(0, 1);
                    s.p.swap(0, 1);
                }
            }
            let s = gjk.simplices[si];
            let tetra = [
                self.newface(svs, s.c[0], s.c[1], s.c[2], true),
                self.newface(svs, s.c[1], s.c[0], s.c[3], true),
                self.newface(svs, s.c[2], s.c[1], s.c[3], true),
                self.newface(svs, s.c[0], s.c[2], s.c[3], true),
            ];
            if self.hull.count == 4 {
                let tetra = tetra.map(|t| t.expect("tetra face"));
                let mut best = self.findbest();
                let mut outer = self.fc_store[best];
                let mut pass: u32 = 0;
                let mut iterations = 0u32;
                Self::bind(&mut self.fc_store, tetra[0], 0, tetra[1], 0);
                Self::bind(&mut self.fc_store, tetra[0], 1, tetra[2], 0);
                Self::bind(&mut self.fc_store, tetra[0], 2, tetra[3], 0);
                Self::bind(&mut self.fc_store, tetra[1], 1, tetra[3], 2);
                Self::bind(&mut self.fc_store, tetra[1], 2, tetra[2], 1);
                Self::bind(&mut self.fc_store, tetra[2], 2, tetra[3], 1);
                self.status = EpaStatus::Valid;
                while iterations < EPA_MAX_ITERATIONS {
                    if self.nextsv < EPA_MAX_VERTICES {
                        let mut horizon = Horizon::default();
                        let w = SV_EPA + self.nextsv;
                        self.nextsv += 1;
                        let mut valid = true;
                        pass += 1;
                        self.fc_store[best].pass = pass as u8;
                        let bn = self.fc_store[best].n;
                        gjk.getsupport(svs, &bn, w);
                        let wdist = self.fc_store[best].n.dot(&svs[w].w) - self.fc_store[best].d;
                        if wdist > EPA_ACCURACY {
                            let mut j = 0;
                            while j < 3 && valid {
                                let (bf, be) = (self.fc_store[best].f[j], self.fc_store[best].e[j]);
                                valid &=
                                    self.expand(svs, pass as u8, w, bf, be as usize, &mut horizon);
                                j += 1;
                            }
                            if valid && horizon.nf >= 3 {
                                Self::bind(
                                    &mut self.fc_store,
                                    horizon.cf.expect("horizon"),
                                    1,
                                    horizon.ff.expect("horizon"),
                                    2,
                                );
                                self.hull_remove(best);
                                self.stock_append(best);
                                best = self.findbest();
                                outer = self.fc_store[best];
                            } else {
                                self.status = EpaStatus::InvalidHull;
                                break;
                            }
                        } else {
                            self.status = EpaStatus::AccuraryReached;
                            break;
                        }
                    } else {
                        self.status = EpaStatus::OutOfVertices;
                        break;
                    }
                    iterations += 1;
                }
                let projection = outer.n * outer.d;
                self.normal = outer.n;
                self.depth = outer.d;
                self.result.rank = 3;
                self.result.c[0] = outer.c[0];
                self.result.c[1] = outer.c[1];
                self.result.c[2] = outer.c[2];
                self.result.p[0] = (svs[outer.c[1]].w - projection)
                    .cross(&(svs[outer.c[2]].w - projection))
                    .length();
                self.result.p[1] = (svs[outer.c[2]].w - projection)
                    .cross(&(svs[outer.c[0]].w - projection))
                    .length();
                self.result.p[2] = (svs[outer.c[0]].w - projection)
                    .cross(&(svs[outer.c[1]].w - projection))
                    .length();
                let sum = self.result.p[0] + self.result.p[1] + self.result.p[2];
                self.result.p[0] /= sum;
                self.result.p[1] /= sum;
                self.result.p[2] /= sum;
                return self.status;
            }
        }
        self.status = EpaStatus::FallBack;
        self.normal = -*guess;
        let nl = self.normal.length();
        if nl > 0.0 {
            self.normal = self.normal / nl;
        } else {
            self.normal = Vec3::new(1.0, 0.0, 0.0);
        }
        self.depth = 0.0;
        self.result.rank = 1;
        self.result.c[0] = gjk.simplices[si].c[0];
        self.result.p[0] = 1.0;
        self.status
    }

    fn getedgedist(&self, svs: &[Sv], face: usize, a: usize, b: usize, dist: &mut f64) -> bool {
        let aw = svs[a].w;
        let bw = svs[b].w;
        let ba = bw - aw;
        let n_ab = ba.cross(&self.fc_store[face].n);
        let a_dot_nab = aw.dot(&n_ab);
        if a_dot_nab < 0.0 {
            let ba_l2 = ba.length2();
            let a_dot_ba = aw.dot(&ba);
            let b_dot_ba = bw.dot(&ba);
            if a_dot_ba > 0.0 {
                *dist = aw.length();
            } else if b_dot_ba < 0.0 {
                *dist = bw.length();
            } else {
                let a_dot_b = aw.dot(&bw);
                *dist = bt_max(
                    fma(aw.length2(), bw.length2(), -(a_dot_b * a_dot_b)) / ba_l2,
                    0.0,
                )
                .sqrt();
            }
            return true;
        }
        false
    }

    fn newface(&mut self, svs: &[Sv], a: usize, b: usize, c: usize, forced: bool) -> Option<usize> {
        if let Some(face) = self.stock.root {
            self.stock_remove(face);
            self.hull_append(face);
            {
                let f = &mut self.fc_store[face];
                f.pass = 0;
                f.c = [a, b, c];
                f.n = (svs[b].w - svs[a].w).cross(&(svs[c].w - svs[a].w));
            }
            let l = self.fc_store[face].n.length();
            let v = l > EPA_ACCURACY;
            if v {
                let mut d = self.fc_store[face].d;
                if !(self.getedgedist(svs, face, a, b, &mut d)
                    || self.getedgedist(svs, face, b, c, &mut d)
                    || self.getedgedist(svs, face, c, a, &mut d))
                {
                    d = svs[a].w.dot(&self.fc_store[face].n) / l;
                }
                let f = &mut self.fc_store[face];
                f.d = d;
                f.n.div_assign_scalar(l);
                if forced || f.d >= -EPA_PLANE_EPS {
                    return Some(face);
                } else {
                    self.status = EpaStatus::NonConvex;
                }
            } else {
                self.status = EpaStatus::Degenerated;
            }
            self.hull_remove(face);
            self.stock_append(face);
            return None;
        }
        self.status = if self.stock.root.is_some() {
            EpaStatus::OutOfVertices
        } else {
            EpaStatus::OutOfFaces
        };
        None
    }

    fn findbest(&self) -> usize {
        let mut minf = self.hull.root.expect("empty hull");
        let mut mind = self.fc_store[minf].d * self.fc_store[minf].d;
        let mut f = self.fc_store[minf].l[1];
        while let Some(fi) = f {
            let sqd = self.fc_store[fi].d * self.fc_store[fi].d;
            if sqd < mind {
                minf = fi;
                mind = sqd;
            }
            f = self.fc_store[fi].l[1];
        }
        minf
    }

    fn expand(
        &mut self,
        svs: &[Sv],
        pass: u8,
        w: usize,
        f: usize,
        e: usize,
        horizon: &mut Horizon,
    ) -> bool {
        const I1M3: [usize; 3] = [1, 2, 0];
        const I2M3: [usize; 3] = [2, 0, 1];
        if self.fc_store[f].pass != pass {
            let e1 = I1M3[e];
            if (self.fc_store[f].n.dot(&svs[w].w) - self.fc_store[f].d) < -EPA_PLANE_EPS {
                let (c_e1, c_e) = (self.fc_store[f].c[e1], self.fc_store[f].c[e]);
                if let Some(nf) = self.newface(svs, c_e1, c_e, w, false) {
                    Self::bind(&mut self.fc_store, nf, 0, f, e);
                    if let Some(cf) = horizon.cf {
                        Self::bind(&mut self.fc_store, cf, 1, nf, 2);
                    } else {
                        horizon.ff = Some(nf);
                    }
                    horizon.cf = Some(nf);
                    horizon.nf += 1;
                    return true;
                }
            } else {
                let e2 = I2M3[e];
                self.fc_store[f].pass = pass;
                let (f1, ee1) = (self.fc_store[f].f[e1], self.fc_store[f].e[e1] as usize);
                if self.expand(svs, pass, w, f1, ee1, horizon) {
                    let (f2, ee2) = (self.fc_store[f].f[e2], self.fc_store[f].e[e2] as usize);
                    if self.expand(svs, pass, w, f2, ee2, horizon) {
                        self.hull_remove(f);
                        self.stock_append(f);
                        return true;
                    }
                }
            }
        }
        false
    }
}

fn initialize<'a>(
    shape0: &'a Shape,
    wtrs0: &Transform,
    shape1: &'a Shape,
    wtrs1: &Transform,
    results: &mut GjkEpaResults,
    withmargins: bool,
) -> MinkowskiDiff<'a> {
    results.witnesses[0] = Vec3::ZERO;
    results.witnesses[1] = Vec3::ZERO;
    results.status = GjkEpaStatus::Separated;
    MinkowskiDiff {
        shapes: [shape0, shape1],
        toshape1: wtrs1.basis.transpose_times(&wtrs0.basis),
        toshape0: wtrs0.inverse_times(wtrs1),
        with_margin: withmargins,
    }
}

fn new_svs() -> Vec<Sv> {
    vec![Sv::default(); SV_EPA + EPA_MAX_VERTICES]
}

/// `btGjkEpaSolver2::Distance`.
pub fn distance(
    shape0: &Shape,
    wtrs0: &Transform,
    shape1: &Shape,
    wtrs1: &Transform,
    guess: &Vec3,
    results: &mut GjkEpaResults,
) -> bool {
    let shape = initialize(shape0, wtrs0, shape1, wtrs1, results, false);
    let mut svs = new_svs();
    let mut gjk = Gjk::new(shape);
    let gjk_status = gjk.evaluate(&mut svs, guess);
    if gjk_status == GjkStatus::Valid {
        let mut w0 = Vec3::ZERO;
        let mut w1 = Vec3::ZERO;
        let s = gjk.simplices[gjk.simplex as usize];
        for i in 0..s.rank as usize {
            let p = s.p[i];
            w0 += gjk.shape.support_index(&svs[s.c[i]].d, 0) * p;
            w1 += gjk.shape.support_index(&-svs[s.c[i]].d, 1) * p;
        }
        results.witnesses[0] = wtrs0.apply(&w0);
        results.witnesses[1] = wtrs0.apply(&w1);
        results.normal = w0 - w1;
        results.distance = results.normal.length();
        let div = if results.distance > GJK_MIN_DISTANCE {
            results.distance
        } else {
            1.0
        };
        results.normal.div_assign_scalar(div);
        true
    } else {
        results.status = if gjk_status == GjkStatus::Inside {
            GjkEpaStatus::Penetrating
        } else {
            GjkEpaStatus::GjkFailed
        };
        false
    }
}

/// `btGjkEpaSolver2::Penetration`.
pub fn penetration(
    shape0: &Shape,
    wtrs0: &Transform,
    shape1: &Shape,
    wtrs1: &Transform,
    guess: &Vec3,
    results: &mut GjkEpaResults,
    usemargins: bool,
) -> bool {
    let shape = initialize(shape0, wtrs0, shape1, wtrs1, results, usemargins);
    let mut svs = new_svs();
    let mut gjk = Gjk::new(shape);
    let gjk_status = gjk.evaluate(&mut svs, &-*guess);
    match gjk_status {
        GjkStatus::Inside => {
            let mut epa = Epa::new();
            let epa_status = epa.evaluate(&mut gjk, &mut svs, &-*guess);
            if epa_status != EpaStatus::Failed {
                let mut w0 = Vec3::ZERO;
                for i in 0..epa.result.rank as usize {
                    w0 += gjk.shape.support_index(&svs[epa.result.c[i]].d, 0) * epa.result.p[i];
                }
                results.status = GjkEpaStatus::Penetrating;
                results.witnesses[0] = wtrs0.apply(&w0);
                results.witnesses[1] = wtrs0.apply(&(w0 - epa.normal * epa.depth));
                results.normal = -epa.normal;
                results.distance = -epa.depth;
                return true;
            } else {
                results.status = GjkEpaStatus::EpaFailed;
            }
        }
        GjkStatus::Failed => results.status = GjkEpaStatus::GjkFailed,
        _ => {}
    }
    false
}

/// `btGjkEpaSolver2::SignedDistance(position, margin, shape, wtrs, results)`.
pub fn signed_distance_point(
    position: &Vec3,
    margin: f64,
    shape0: &Shape,
    wtrs0: &Transform,
    results: &mut GjkEpaResults,
) -> f64 {
    let shape1 = Shape::new_sphere(margin);
    let wtrs1 = Transform::from_quat(&Quat::new(0.0, 0.0, 0.0, 1.0), *position);
    let shape = initialize(shape0, wtrs0, &shape1, &wtrs1, results, false);
    let mut svs = new_svs();
    let mut gjk = Gjk::new(shape);
    let gjk_status = gjk.evaluate(&mut svs, &Vec3::new(1.0, 1.0, 1.0));
    if gjk_status == GjkStatus::Valid {
        let mut w0 = Vec3::ZERO;
        let mut w1 = Vec3::ZERO;
        let s = gjk.simplices[gjk.simplex as usize];
        for i in 0..s.rank as usize {
            let p = s.p[i];
            w0 += gjk.shape.support_index(&svs[s.c[i]].d, 0) * p;
            w1 += gjk.shape.support_index(&-svs[s.c[i]].d, 1) * p;
        }
        results.witnesses[0] = wtrs0.apply(&w0);
        results.witnesses[1] = wtrs0.apply(&w1);
        let delta = results.witnesses[1] - results.witnesses[0];
        let margin = shape0.get_margin_non_virtual() + shape1.get_margin_non_virtual();
        let length = delta.length();
        results.normal = delta / length;
        results.witnesses[0] += results.normal * margin;
        length - margin
    } else {
        if gjk_status == GjkStatus::Inside {
            let ray = gjk.ray;
            if penetration(shape0, wtrs0, &shape1, &wtrs1, &ray, results, true) {
                let delta = results.witnesses[0] - results.witnesses[1];
                let length = delta.length();
                if length >= SIMD_EPSILON {
                    results.normal = delta / length;
                }
                return -length;
            }
        }
        SIMD_INFINITY
    }
}

/// `btGjkEpaSolver2::SignedDistance(shape0, wtrs0, shape1, wtrs1, guess, results)`.
pub fn signed_distance(
    shape0: &Shape,
    wtrs0: &Transform,
    shape1: &Shape,
    wtrs1: &Transform,
    guess: &Vec3,
    results: &mut GjkEpaResults,
) -> bool {
    if !distance(shape0, wtrs0, shape1, wtrs1, guess, results) {
        penetration(shape0, wtrs0, shape1, wtrs1, guess, results, false)
    } else {
        true
    }
}

/// `btGjkEpaPenetrationDepthSolver::calcPenDepth`. Returns `isValid` and
/// writes `v`, `wWitnessOnA`, `wWitnessOnB`.
pub fn gjk_epa_calc_pen_depth(
    convex_a: &Shape,
    convex_b: &Shape,
    transform_a: &Transform,
    transform_b: &Transform,
    v: &mut Vec3,
    w_witness_on_a: &mut Vec3,
    w_witness_on_b: &mut Vec3,
) -> bool {
    let guess_vector = transform_b.origin - transform_a.origin;
    let mut results = GjkEpaResults::default();
    if penetration(
        convex_a,
        transform_a,
        convex_b,
        transform_b,
        &guess_vector,
        &mut results,
        true,
    ) {
        *w_witness_on_a = results.witnesses[0];
        *w_witness_on_b = results.witnesses[1];
        *v = results.normal;
        return true;
    } else if distance(
        convex_a,
        transform_a,
        convex_b,
        transform_b,
        &guess_vector,
        &mut results,
    ) {
        *w_witness_on_a = results.witnesses[0];
        *w_witness_on_b = results.witnesses[1];
        *v = results.normal;
        return false;
    }
    false
}
