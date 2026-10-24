//! Per-square colours: `LightingSquare::CalcLightInfo`,
//! `CalcMaxLightInfo`, `LightingState::CalculateMaxVertLight1/2`,
//! `CalculateVertColoursForTile` and `CalculateColor`.

use crate::model::{fastfloor, flags, open, Character};
use crate::state::LightingState;
use crate::world::{nb, SqId};

/// Arguments of one `CalculateColor` call: the diagonal and the two
/// cardinal neighbours sharing the corner, and the four corner indices.
type ColorCall = (Option<SqId>, Option<SqId>, Option<SqId>, [usize; 4]);

/// `0xFF000000 | b << 16 | g << 8 | r`.
#[inline]
fn pack(c: [u32; 3]) -> u32 {
    c[0] | (c[1] << 8) | (c[2] << 16) | 0xff00_0000
}

#[inline]
fn unpack(v: u32) -> [u32; 3] {
    [v & 0xff, (v >> 8) & 0xff, (v >> 16) & 0xff]
}

#[inline]
fn max3(a: [u32; 3], b: [u32; 3]) -> [u32; 3] {
    [a[0].max(b[0]), a[1].max(b[1]), a[2].max(b[2])]
}

impl LightingState {
    /// `LightingSquare::CalcLightInfo`: darkMulti from the target values of
    /// the square and its visible cardinal neighbours, and the light colour
    /// from sun, sky, light lists and torches.
    pub(crate) fn calc_light_info(&mut self, id: SqId, chr: &Character) {
        let (w, h) = (self.w, self.h);
        let (z, idx, f, level, nbs) = {
            let s = self.sq(id);
            (i32::from(s.z), s.idx as usize, s.flags, s.level(), s.nb)
        };
        let zi = z as usize & 63;
        let nb_idx = |o: Option<SqId>| o.map(|o| self.sq(o).idx as usize);
        let (iw, inn, ie, is) = (nb_idx(nbs[nb::W]), nb_idx(nbs[nb::N]), nb_idx(nbs[nb::E]), nb_idx(nbs[nb::S]));
        let sky = self.sky;
        let torch = self.torch_cells.get(idx).map(|c| c.rgb).unwrap_or([0; 3]);
        let d = self.back[zi].data_mut(w, h);
        let t = d.target[idx];
        let mut sum;
        let mut n: u32;
        match iw {
            Some(j) if f & flags::W != 0 && t < d.target[j] => {
                sum = if d.vis[j] & 1 != 0 { t + d.target[j] } else { t };
                n = 2;
            }
            _ => {
                sum = t;
                n = 1;
            }
        }
        for (bit, j) in [(flags::N, inn), (flags::E, ie), (flags::S, is)] {
            let Some(j) = j else { continue };
            if f & bit == 0 {
                continue;
            }
            let tn = d.target[j];
            if t < tn {
                if d.vis[j] & 1 != 0 {
                    sum += tn;
                }
                n += 1;
            }
        }
        let mut cr = (level & 0xff) as i32;
        let mut cg = ((level >> 8) & 0xff) as i32;
        let mut cb = ((level >> 16) & 0xff) as i32;
        if f & flags::OPEN_AIR != 0 {
            cr = cr.max((sky & 0xff) as i32);
            cg = cg.max(((sky >> 8) & 0xff) as i32);
            cb = cb.max(((sky >> 16) & 0xff) as i32);
        }
        let list = d.statics[idx];
        let count = list.count;
        if count > 0 {
            for e in &list.lights[..(count as usize).min(5)] {
                let k = 1.0 - e.dist / f32::from(e.radius);
                let p = e.packed;
                cr = cr.max((k * (p & 0xff) as f32) as i32);
                cg = cg.max((((p >> 8) & 0xff) as f32 * k) as i32);
                cb = cb.max((((p >> 16) & 0xff) as f32 * k) as i32);
            }
        }
        let dark = sum / n as f32;
        let light_count = d.torches[idx].count.wrapping_add(count);
        if fastfloor(chr.z) == z && (d.vis[idx] >> 1) & 1 != 0 {
            cr += i32::from(torch[0]);
            cg += i32::from(torch[1]);
            cb += i32::from(torch[2]);
        }
        let cr = if cr < 0xff { cr } else { 0xff };
        let cg = cg.min(0xff);
        let cb = cb.min(0xff);
        let vr = dark * (cr as u32 as f32 / 255.0);
        let vg = (cg as u32 as f32 / 255.0) * dark;
        let vb = (cb as u32 as f32 / 255.0) * dark;
        let vr = if vr > 1.0 { 1.0 } else { vr };
        let vg = if vg > 1.0 { 1.0 } else { vg };
        let vb = if vb > 1.0 { 1.0 } else { vb };
        let br = (vr * 255.0) as i32 as u8;
        let bg = (vg * 255.0) as i32 as u8;
        let bb = (vb * 255.0) as i32 as u8;
        let d1000 = (dark * 1000.0) as i32;
        let s = &mut self.squares[id as usize];
        if s.prev_rgb != [br, bg, bb]
            || s.prev_level != level
            || s.prev_dark != d1000
            || s.prev_light_count != light_count
        {
            d.set_dirty(w, idx);
        }
        s.prev_rgb = [br, bg, bb];
        d.r[idx] = br;
        d.g[idx] = bg;
        d.b[idx] = bb;
        s.prev_level = level;
        d.level[idx] = level;
        s.prev_light_count = light_count;
        d.dark[idx] = dark;
        s.prev_dark = d1000;
    }

    /// `LightingSquare::CalcMaxLightInfo`.
    pub(crate) fn calc_max_light_info(&mut self, id: SqId) {
        let (w, h) = (self.w, self.h);
        let rgb_of = |st: &mut LightingState, o: SqId| -> [u8; 3] {
            let (z, i) = {
                let s = st.sq(o);
                (s.z as usize & 63, s.idx as usize)
            };
            let d = st.back[z].data_mut(w, h);
            [d.r[i], d.g[i], d.b[i]]
        };
        let own = rgb_of(self, id);
        let (f, down, up) = {
            let s = self.sq(id);
            (s.flags, s.nb[nb::DOWN], s.nb[nb::UP])
        };
        let mut lo = own;
        let mut hi = own;
        if f & flags::DOWN != 0 {
            if let Some(o) = down {
                let c = rgb_of(self, o);
                lo = [own[0].max(c[0]), own[1].max(c[1]), own[2].max(c[2])];
            }
        }
        if f & flags::UP != 0 {
            if let Some(o) = up {
                let c = rgb_of(self, o);
                hi = [own[0].max(c[0]), own[1].max(c[1]), own[2].max(c[2])];
            }
        }
        let s = self.sq_mut(id);
        s.max_down = lo;
        s.max_up = hi;
    }

    /// The eight neighbours used by the vertex stages, in slot order,
    /// filtered by the vision flags (diagonals) and open edges (cardinals).
    fn vert_neighbours(&self, id: SqId) -> [Option<SqId>; 8] {
        let s = self.sq(id);
        let f = s.flags;
        let o = s.open;
        let pick = |cond: bool, slot: usize| if cond { s.nb[slot] } else { None };
        [
            pick(f & flags::NW != 0, nb::NW),
            pick(o & open::N != 0, nb::N),
            pick(f & flags::NE != 0, nb::NE),
            pick(o & open::E != 0, nb::E),
            pick(f & flags::SE != 0, nb::SE),
            pick(o & open::S != 0, nb::S),
            pick(f & flags::SW != 0, nb::SW),
            pick(o & open::W != 0, nb::W),
        ]
    }

    /// `LightingState::CalculateMaxVertLight1`: corner maxima of the
    /// below/above maxima of the four squares sharing each corner.
    pub(crate) fn calculate_max_vert_light1(&mut self, id: SqId) {
        let (w, h) = (self.w, self.h);
        let [nw, n, ne, e, se, s, sw, wst] = self.vert_neighbours(id);
        let lo = |o: Option<SqId>| o.map(|o| self.sq(o).max_down.map(u32::from));
        let hi = |o: Option<SqId>| o.map(|o| self.sq(o).max_up.map(u32::from));
        let me = self.sq(id);
        let own_lo = me.max_down.map(u32::from);
        let own_hi = me.max_up.map(u32::from);
        let corners: [[Option<SqId>; 3]; 4] = [[nw, n, wst], [ne, n, e], [se, s, e], [sw, s, wst]];
        let mut out = [0u32; 8];
        for (k, c) in corners.iter().enumerate() {
            let mut acc = [0u32; 3];
            for v in c.iter().filter_map(|&o| lo(o)) {
                acc = max3(acc, v);
            }
            out[k] = pack(max3(acc, own_lo));
            let mut acc = [0u32; 3];
            for v in c.iter().filter_map(|&o| hi(o)) {
                acc = max3(acc, v);
            }
            out[k + 4] = pack(max3(acc, own_hi));
        }
        let (zi, i) = (me.z as usize & 63, me.idx as usize);
        let d = self.back[zi].data_mut(w, h);
        for (arr, v) in d.inter.iter_mut().zip(out) {
            arr[i] = v;
        }
    }

    /// `LightingState::CalculateMaxVertLight2`: blends each corner with the
    /// matching corners of the neighbours, in place (so the pass order
    /// matters).
    pub(crate) fn calculate_max_vert_light2(&mut self, id: SqId) {
        let (w, h) = (self.w, self.h);
        let [nw, n, ne, e, se, s, sw, wst] = self.vert_neighbours(id);
        // (diagonal, cardinal a, cardinal b) and the corner each of them
        // contributes, for corners 0..3; the upper set is the same + 4.
        let plan: [[(Option<SqId>, usize); 3]; 4] = [
            [(nw, 2), (n, 3), (wst, 1)],
            [(ne, 3), (n, 2), (e, 0)],
            [(se, 0), (s, 1), (e, 3)],
            [(sw, 1), (s, 0), (wst, 2)],
        ];
        let me = self.sq(id);
        let (zi, i) = (me.z as usize & 63, me.idx as usize);
        for upper in [0usize, 4] {
            for (k, row) in plan.iter().enumerate() {
                let mut acc = unpack(self.back[zi].data_mut(w, h).inter[k + upper][i]);
                for &(o, corner) in row {
                    let Some(o) = o else { continue };
                    let (oz, oi) = {
                        let os = self.sq(o);
                        (os.z as usize & 63, os.idx as usize)
                    };
                    let v = self.back[oz].data_mut(w, h).inter[corner + upper][oi];
                    acc = max3(acc, unpack(v));
                }
                self.back[zi].data_mut(w, h).inter[k + upper][i] = pack(acc);
            }
        }
    }

    /// `LightingState::CalculateVertColoursForTile`.
    pub(crate) fn calculate_vert_colours_for_tile(&mut self, id: SqId) {
        let [nw, n, ne, e, se, s, sw, wst] = self.vert_neighbours(id);
        let calls: [ColorCall; 4] = [
            (nw, n, wst, [2, 3, 1, 0]),
            (ne, n, e, [3, 2, 0, 1]),
            (se, s, e, [0, 1, 3, 2]),
            (sw, s, wst, [1, 0, 2, 3]),
        ];
        for upper in [0usize, 4] {
            for &(a, b, c, k) in &calls {
                self.calculate_color(a, b, c, id, k.map(|v| v + upper));
            }
        }
    }

    /// `LightingState::CalculateColor`: the final colour of one vertex.
    fn calculate_color(&mut self, a: Option<SqId>, b: Option<SqId>, c: Option<SqId>, id: SqId, k: [usize; 4]) {
        let (w, h) = (self.w, self.h);
        let mut acc = [0u32; 3];
        for (o, corner) in [(a, k[0]), (b, k[1]), (c, k[2])] {
            let Some(o) = o else { continue };
            let (oz, oi) = {
                let os = self.sq(o);
                (os.z as usize & 63, os.idx as usize)
            };
            acc = max3(acc, unpack(self.back[oz].data_mut(w, h).inter[corner][oi]));
        }
        let (zi, i) = {
            let s = self.sq(id);
            (s.z as usize & 63, s.idx as usize)
        };
        let d = self.back[zi].data_mut(w, h);
        let v = pack(max3(acc, unpack(d.inter[k[3]][i])));
        let s = &mut self.squares[id as usize];
        if s.prev_vert[k[3]] != v {
            d.set_dirty(w, i);
        }
        s.prev_vert[k[3]] = v;
        d.vert[k[3]][i] = v;
    }
}
