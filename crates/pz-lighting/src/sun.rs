//! Sun/sky seeding and the light flood fills: `CalculateFullLighting`,
//! `PropagateSunlightToNeighbour[s|Fraction]`,
//! `PropagateLightsToNeighbour[s]`.

use std::collections::VecDeque;

use crate::model::{distance_to, flags, open, GROUND_LEVEL};
use crate::model::Transmission;
use crate::state::LightingState;
use crate::world::{edge, nb, SqId};

/// The cardinal neighbours in propagation order with the edge used on
/// each side: (neighbour slot, open-mask bit, own edge, neighbour's edge).
const CARDINALS: [(usize, u32, usize, usize); 4] = [
    (nb::W, open::W, edge::W, edge::E),
    (nb::E, open::E, edge::E, edge::W),
    (nb::N, open::N, edge::N, edge::S),
    (nb::S, open::S, edge::S, edge::N),
];

impl LightingState {
    /// Chooses the two transmissions for a closed edge, or `None` when
    /// neither side transmits any light.
    fn edge_transmissions(&self, from: SqId, to: SqId, own: usize, theirs: usize) -> Option<(Transmission, Transmission)> {
        let a = self.sq(from).trans[own];
        let b = self.sq(to).trans[theirs];
        if a.intensity <= 0.0 {
            if b.intensity <= 0.0 {
                return None;
            }
            Some((b, Transmission::NONE))
        } else {
            let second = if b.intensity > 0.0 { b } else { Transmission::NONE };
            Some((a, second))
        }
    }

    /// `PropagateSunlightToNeighbour`: open edge, -16 per step (-32 across
    /// levels). The neighbour is overwritten when any channel improves.
    fn propagate_sun_open(&mut self, from: SqId, to: SqId, q: &mut VecDeque<SqId>) {
        let (src, sz) = {
            let s = self.sq(from);
            (s.sun, s.z)
        };
        let t = self.sq_mut(to);
        let k = if t.z != sz { 32 } else { 16 };
        let c = [i32::from(src[0]) - k, i32::from(src[1]) - k, i32::from(src[2]) - k];
        if i32::from(t.sun[0]) < c[0] || i32::from(t.sun[1]) < c[1] || i32::from(t.sun[2]) < c[2] {
            t.sun = [c[0].max(0) as u8, c[1].max(0) as u8, c[2].max(0) as u8];
            q.push_back(to);
        }
    }

    /// `PropagateSunlightToNeighbourFraction`: a closed edge that lets a
    /// fraction of the light through.
    fn propagate_sun_fraction(&mut self, from: SqId, to: SqId, q: &mut VecDeque<SqId>, t1: &Transmission, t2: &Transmission) {
        let src = self.sq(from).sun;
        let r = f32::from(src[0]) / 255.0;
        let g = f32::from(src[1]) / 255.0;
        let b = f32::from(src[2]) / 255.0;
        let (r, g, b) = t1.apply(r, g, b);
        let (r, g, b) = t2.apply(r, g, b);
        let (r, g, b) = (255.0 * r, 255.0 * g, b * 255.0);
        let t = self.sq_mut(to);
        let (nr, ng, nb_) = (f32::from(t.sun[0]), f32::from(t.sun[1]), f32::from(t.sun[2]));
        if !(r <= nr && g <= ng && b <= nb_) {
            let old = t.level();
            let ri = r as i32 as u32;
            let gi = g as i32 as u32;
            let bi = b as i32 as u32;
            let v = (old & 0xff00_0000) | ((gi << 8) & 0xff00_ff00) | (ri & 0xff00_00ff) | (bi << 16);
            t.set_level(v);
            q.push_back(to);
        }
    }

    /// `LightingSquare::PropagateSunlightToNeighbours`.
    fn propagate_sun_neighbours(&mut self, id: SqId, q: &mut VecDeque<SqId>) {
        for &(slot, bit, own, theirs) in &CARDINALS {
            let Some(n) = self.sq(id).nb[slot] else { continue };
            if self.sq(id).open & bit != 0 {
                self.propagate_sun_open(id, n, q);
            } else if let Some((t1, t2)) = self.edge_transmissions(id, n, own, theirs) {
                self.propagate_sun_fraction(id, n, q, &t1, &t2);
            }
        }
        let s = self.sq(id);
        if s.flags & flags::DOWN != 0 {
            if let Some(d) = s.nb[nb::DOWN] {
                if i32::from(s.z) == GROUND_LEVEL {
                    let ds = self.sq(d);
                    if ds.flags & flags::OPEN_AIR == 0 || ds.room_id != -1 {
                        self.propagate_sun_open(id, d, q);
                    }
                }
            }
        }
    }

    /// `PropagateLightsToNeighbour`: offers every light of `from` to `to`.
    fn propagate_lights_to(&mut self, from: SqId, to: SqId, q: &mut VecDeque<SqId>, t1: &Transmission, t2: &Transmission) {
        let (w, h) = (self.w, self.h);
        let (fx, fy, fz, fi) = {
            let s = self.sq(from);
            (s.x, s.y, i32::from(s.z), s.idx as usize)
        };
        let (tx, ty, tz, ti) = {
            let s = self.sq(to);
            (s.x, s.y, i32::from(s.z), s.idx as usize)
        };
        let d = distance_to(
            (fx as f32 + 0.5) as i32,
            (fy as f32 + 0.5) as i32,
            fz,
            (tx as f32 + 0.5) as i32,
            (ty as f32 + 0.5) as i32,
            tz,
        );
        let src = self.back[fz as usize & 63].data_mut(w, h).statics[fi];
        if src.count < 1 {
            return;
        }
        let mut changed = false;
        let mut i = 0;
        // The count is re-read after every add, as the source list can be
        // the destination list.
        let mut count = src.count;
        while i < count {
            let e = self.back[fz as usize & 63].data_mut(w, h).statics[fi].lights[i as usize];
            let total = d + e.dist;
            if !(total > f32::from(e.radius) + 0.01) {
                let p = e.packed;
                let r = (p & 0xff) as f32 / 255.0;
                let g = ((p >> 8) & 0xff) as f32 / 255.0;
                let b = ((p >> 16) & 0xff) as f32 / 255.0;
                let (r, g, b) = t1.apply(r, g, b);
                let (r, g, b) = t2.apply(r, g, b);
                let dst = &mut self.back[tz as usize & 63].data_mut(w, h).statics[ti];
                changed |= dst.add_light(e.id, e.x, e.y, e.z, i32::from(e.radius), r, g, b, (p >> 24) as u8, total);
                count = self.back[fz as usize & 63].data_mut(w, h).statics[fi].count;
            }
            i += 1;
        }
        if changed {
            q.push_back(to);
        }
    }

    /// `LightingSquare::PropagateLightsToNeighbours`.
    fn propagate_lights_neighbours(&mut self, id: SqId, q: &mut VecDeque<SqId>) {
        for &(slot, bit, own, theirs) in &CARDINALS {
            let Some(n) = self.sq(id).nb[slot] else { continue };
            if self.sq(id).open & bit != 0 {
                self.propagate_lights_to(id, n, q, &Transmission::OPEN, &Transmission::NONE);
            } else if let Some((t1, t2)) = self.edge_transmissions(id, n, own, theirs) {
                self.propagate_lights_to(id, n, q, &t1, &t2);
            }
        }
        const OTHERS: [(u16, usize); 6] = [
            (flags::NW, nb::NW),
            (flags::NE, nb::NE),
            (flags::SE, nb::SE),
            (flags::SW, nb::SW),
            (flags::UP, nb::UP),
            (flags::DOWN, nb::DOWN),
        ];
        for &(bit, slot) in &OTHERS {
            let s = self.sq(id);
            if s.flags & bit != 0 {
                if let Some(n) = s.nb[slot] {
                    self.propagate_lights_to(id, n, q, &Transmission::OPEN, &Transmission::NONE);
                }
            }
        }
    }

    /// Runs a FIFO flood from one square.
    fn flood(&mut self, seed: SqId, f: fn(&mut LightingState, SqId, &mut VecDeque<SqId>)) {
        let mut q = VecDeque::new();
        f(self, seed, &mut q);
        while let Some(s) = q.pop_front() {
            f(self, s, &mut q);
        }
    }

    /// `LightingState::CalculateFullLighting`: resets the propagated light,
    /// seeds the sky columns and floods sunlight, then floods the point and
    /// room lights from the squares that hold them.
    pub(crate) fn calculate_full_lighting(&mut self) {
        let (w, h) = (self.w, self.h);
        let (mw, mh) = (self.map.w, self.map.h);
        let mut light_seeds: Vec<SqId> = Vec::new();
        for cy in 0..mh {
            for cx in 0..mw {
                let Some(c) = self.map.at(cx, cy) else { continue };
                if c.min > c.max {
                    continue;
                }
                let mut ids = Vec::new();
                for z in (c.min..=c.max).rev() {
                    let l = c.level(z).unwrap();
                    for u in 0..64u32 {
                        if let Some(id) = l[((u & 7) * 8 + (u >> 3)) as usize] {
                            ids.push(id);
                        }
                    }
                }
                for id in ids {
                    let (x, y, z, idx) = {
                        let s = self.sq_mut(id);
                        s.flags &= !flags::SUN_SEEDED;
                        s.set_level(0);
                        (s.x, s.y, i32::from(s.z), s.idx as usize)
                    };
                    let list = &self.back[z as usize & 63].data_mut(w, h).statics[idx];
                    let n = list.count.max(0) as usize;
                    if list.lights[..n.min(5)].iter().any(|e| e.x == x && e.y == y && e.z == z) {
                        light_seeds.push(id);
                    }
                }
            }
        }
        // Sky seeds: every column from the top down while the square is
        // open air outdoors.
        let mut sky_seeds: Vec<SqId> = Vec::new();
        let sky = self.sky;
        for cy in 0..mh {
            for cx in 0..mw {
                let Some(c) = self.map.at(cx, cy) else { continue };
                if c.min > c.max {
                    continue;
                }
                let mut column_seeds = Vec::new();
                for u in 0..64u32 {
                    for z in (c.min..=c.max).rev() {
                        let Some(id) = c.level(z).unwrap()[((u & 7) * 8 + (u >> 3)) as usize] else {
                            continue;
                        };
                        let s = self.sq(id);
                        if s.flags & flags::OPEN_AIR == 0 || s.room_id != -1 {
                            break;
                        }
                        column_seeds.push(id);
                    }
                }
                for id in column_seeds {
                    let s = self.sq_mut(id);
                    s.set_level((sky & 0x00ff_ffff) | (u32::from(s.level_hi) << 24));
                    s.flags |= flags::SUN_SEEDED;
                    sky_seeds.push(id);
                }
            }
        }
        // The native code keeps the seeds in an unordered_set keyed by
        // address; its iteration order cannot be reproduced. Insertion order
        // is used instead (see the crate docs on order dependence), unless an
        // explicit order was supplied for parity testing.
        if let Some(order) = self.seed_order.take() {
            let rank: std::collections::HashMap<(i32, i32, i32), usize> =
                order.into_iter().enumerate().map(|(i, k)| (k, i)).collect();
            let key = |st: &LightingState, id: SqId| {
                let s = st.sq(id);
                rank.get(&(s.x, s.y, i32::from(s.z))).copied().unwrap_or(usize::MAX)
            };
            sky_seeds.sort_by_key(|&id| key(self, id));
        }
        for id in sky_seeds {
            self.flood(id, LightingState::propagate_sun_neighbours);
        }
        for id in light_seeds {
            self.flood(id, LightingState::propagate_lights_neighbours);
        }
    }
}
