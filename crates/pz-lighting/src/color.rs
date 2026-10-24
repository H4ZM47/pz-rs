//! Per-square colours: `LightingSquare::CalcLightInfo`,
//! `CalcMaxLightInfo`, `LightingState::CalculateMaxVertLight1/2`,
//! `CalculateVertColoursForTile` and `CalculateColor`.

use crate::model::{fastfloor, flags, open, Character};
use crate::state::LightingState;
use crate::world::{nb, SqId};

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

    /// The eight neighbours used by the vertex stages, in slot order
    /// (NW, N, NE, E, SE, S, SW, W), filtered by the vision flags
    /// (diagonals) and open edges (cardinals).
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

    /// Own (level, index) and the vertex neighbours' (level, index).
    fn vert_slots(&self, id: SqId) -> (Slot, [Option<Slot>; 8]) {
        let at = |o: SqId| {
            let s = self.sq(o);
            (s.z as usize & 63, s.idx as usize)
        };
        (at(id), self.vert_neighbours(id).map(|o| o.map(at)))
    }

    /// `LightingState::CalculateMaxVertLight1`: corner maxima of the
    /// below/above maxima of the four squares sharing each corner.
    pub(crate) fn calculate_max_vert_light1(&mut self, id: SqId) {
        let (w, h) = (self.w, self.h);
        let nbs = self.vert_neighbours(id);
        let me = self.sq(id);
        let mut out = [0u32; 8];
        for (k, c) in CORNERS.iter().enumerate() {
            let mut lo = bytes(me.max_down);
            let mut hi = bytes(me.max_up);
            for &slot in c {
                if let Some(o) = nbs[slot] {
                    let os = self.sq(o);
                    lo = bmax(lo, bytes(os.max_down));
                    hi = bmax(hi, bytes(os.max_up));
                }
            }
            out[k] = lo | 0xff00_0000;
            out[k + 4] = hi | 0xff00_0000;
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
        let ((zi, i), nbs) = self.vert_slots(id);
        for upper in [0usize, 4] {
            for (k, row) in BLEND.iter().enumerate() {
                let mut acc = self.back[zi].data_mut(w, h).inter[k + upper][i];
                for &(slot, corner) in row {
                    if let Some((oz, oi)) = nbs[slot] {
                        acc = bmax(acc, self.back[oz].data_mut(w, h).inter[corner + upper][oi]);
                    }
                }
                self.back[zi].data_mut(w, h).inter[k + upper][i] = acc | 0xff00_0000;
            }
        }
    }

    /// `LightingState::CalculateVertColoursForTile`: eight
    /// `CalculateColor` calls, the floor corners then the wall tops.
    pub(crate) fn calculate_vert_colours_for_tile(&mut self, id: SqId) {
        let (w, h) = (self.w, self.h);
        let ((zi, i), nbs) = self.vert_slots(id);
        for upper in [0usize, 4] {
            for (slots, k) in COLOR_CALLS {
                // `CalculateColor`: the final colour of one vertex.
                let mut acc = 0u32;
                for (slot, corner) in slots.into_iter().zip(k) {
                    if let Some((oz, oi)) = nbs[slot] {
                        acc = bmax(acc, self.back[oz].data_mut(w, h).inter[corner + upper][oi]);
                    }
                }
                let kk = k[3] + upper;
                let d = self.back[zi].data_mut(w, h);
                let v = bmax(acc, d.inter[kk][i]) | 0xff00_0000;
                let s = &mut self.squares[id as usize];
                if s.prev_vert[kk] != v {
                    d.set_dirty(w, i);
                }
                s.prev_vert[kk] = v;
                d.vert[kk][i] = v;
            }
        }
    }
}

/// A square's result arrays: (level, index).
type Slot = (usize, usize);

/// Neighbour slots sharing each floor corner (NW, NE, SE, SW).
const CORNERS: [[usize; 3]; 4] = [[0, 1, 7], [2, 1, 3], [4, 5, 3], [6, 5, 7]];

/// For each corner: the neighbour slots and the corner of theirs that
/// coincides with it (`CalculateMaxVertLight2`).
const BLEND: [[(usize, usize); 3]; 4] = [
    [(0, 2), (1, 3), (7, 1)],
    [(2, 3), (1, 2), (3, 0)],
    [(4, 0), (5, 1), (3, 3)],
    [(6, 1), (5, 0), (7, 2)],
];

/// `CalculateColor` arguments: diagonal, cardinal, cardinal (slots), and
/// the corners of those three and of the square itself.
const COLOR_CALLS: [([usize; 3], [usize; 4]); 4] = [
    ([0, 1, 7], [2, 3, 1, 0]),
    ([2, 1, 3], [3, 2, 0, 1]),
    ([4, 5, 3], [0, 1, 3, 2]),
    ([6, 5, 7], [1, 0, 2, 3]),
];

/// RGB bytes as `r | g << 8 | b << 16`.
#[inline]
fn bytes(c: [u8; 3]) -> u32 {
    u32::from(c[0]) | (u32::from(c[1]) << 8) | (u32::from(c[2]) << 16)
}

/// Channel-wise maximum of the low three bytes (the alpha byte is dropped).
#[inline]
fn bmax(a: u32, b: u32) -> u32 {
    let r = (a & 0xff).max(b & 0xff);
    let g = (a & 0xff00).max(b & 0xff00);
    let bl = (a & 0xff_0000).max(b & 0xff_0000);
    r | g | bl
}
