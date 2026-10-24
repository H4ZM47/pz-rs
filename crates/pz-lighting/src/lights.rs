//! Light sources: `LightingState::updateLightSource`, `addInfluence`,
//! `touchedByLight` and `RoomLight::flagLitSquares/addInfluence`.

use crate::model::{Light, RoomLight};
use crate::session::Globals;
use crate::state::LightingState;

/// Packed colour of a room light entry: r 229, g 216, b 209, flags 1.
const ROOM_LIGHT_PACKED: u32 = 0x01d1_d8e5;
/// Room lights fall off over this many squares outside their rectangle.
const ROOM_LIGHT_RADIUS: i32 = 7;

impl LightingState {
    /// `LightingState::updateLightSource`: rebuilds the light lists' seeds
    /// from the room lights and point lights handed to this state.
    pub(crate) fn update_light_source(&mut self, g: &Globals) {
        let (x0, y0) = (self.ox, self.oy);
        let x1 = self.w + x0;
        let y1 = self.h + y0;
        let (w, h) = (self.w, self.h);
        let (mw, mh) = (self.map.w, self.map.h);
        for cy in 0..mh {
            for cx in 0..mw {
                let Some(c) = self.map.at(cx, cy) else { continue };
                if !c.light_touched {
                    continue;
                }
                let base = ((c.wx - self.map.x) + w * (c.wy - self.map.y)) * 8;
                for z in self.min_level..self.max_level {
                    let d = self.back[z as usize & 63].data_mut(w, h);
                    let mut row = base;
                    for _ in 0..8 {
                        for k in 0..8 {
                            if let Some(v) = d.lit.get_mut((row + k) as usize) {
                                *v = 0;
                            }
                        }
                        row += w;
                    }
                }
                if let Some(c) = self.map.slot(cx, cy).as_deref_mut() {
                    c.light_touched = false;
                }
            }
        }
        self.temp_lights_changed = false;
        self.in_update_light_source = true;
        let room_lights = self.room_lights.clone();
        for &rl in &room_lights {
            self.flag_lit_squares(&g.room_lights[rl], x0, y0, x1, y1);
        }
        for &rl in &room_lights {
            self.room_add_influence(&g.room_lights[rl], x0, y0, x1, y1);
        }
        let lights = self.lights.clone();
        for &l in &lights {
            let light = &g.lights[l];
            if light.life_max > 0 && light.life > 0 {
                self.temp_lights_changed = true;
            }
            let r = light.radius;
            if x0 <= r + light.x && light.x - r < x1 && y0 <= light.y + r && light.y - r < y1 {
                self.add_influence(light);
            }
        }
        self.in_update_light_source = false;
    }

    /// `LightingState::addInfluence`: puts an active point light into the
    /// light list of its own square.
    fn add_influence(&mut self, l: &Light) {
        let p = self.player as usize & 3;
        if !((l.life_max < 1 || l.life > 0) && l.active_snap[p]) {
            return;
        }
        let lx = l.x.wrapping_sub(self.ox);
        let ly = l.y.wrapping_sub(self.oy);
        if lx < 0 || lx >= self.w || ly < 0 || ly >= self.h {
            return;
        }
        let Some(id) = self.map.at(lx >> 3, ly >> 3).and_then(|c| c.square(lx & 7, ly & 7, l.z)) else {
            return;
        };
        let (w, h) = (self.w, self.h);
        let idx = {
            let s = self.sq(id);
            (s.x - self.ox) + (s.y - self.oy) * w
        };
        self.sq_mut(id).idx = idx as u16;
        let list = &mut self.back[l.z as usize & 63].data_mut(w, h).statics[idx as usize & 0xffff];
        list.add_light(l.id, l.x, l.y, l.z, l.radius, l.r, l.g, l.b, 0, 0.0);
    }

    /// `LightingState::touchedByLight`: flags the chunks under a rectangle.
    fn touched_by_light(&mut self, x0: i32, y0: i32, x1: i32, y1: i32) {
        let cx0 = x0 / 8;
        let cy0 = y0 / 8;
        let cx1 = x1 / 8 - i32::from(x1 & 7 == 0);
        let cy1 = y1 / 8 - i32::from(y1 & 7 == 0);
        if cy0 > cy1 || cx0 > cx1 {
            return;
        }
        for cy in cy0..=cy1 {
            for cx in cx0..=cx1 {
                if let Some(c) = self.map.get_chunk_mut(cx, cy) {
                    c.light_touched = true;
                }
            }
        }
    }

    /// Clips a room light's rectangle to `[x0, x1) x [y0, y1)`.
    fn clip(rl: &RoomLight, x0: i32, y0: i32, x1: i32, y1: i32) -> (i32, i32, i32, i32) {
        let ax = if rl.x <= x0 { x0 } else { rl.x };
        let bx = if x1 <= rl.w + rl.x { x1 } else { rl.w + rl.x };
        let ay = if rl.y <= y0 { y0 } else { rl.y };
        let by = if y1 <= rl.h + rl.y { y1 } else { rl.h + rl.y };
        (ax, ay, bx, by)
    }

    /// `RoomLight::flagLitSquares`.
    fn flag_lit_squares(&mut self, rl: &RoomLight, x0: i32, y0: i32, x1: i32, y1: i32) {
        if !rl.active_snap[self.player as usize & 3] {
            return;
        }
        let (ax, ay, bx, by) = Self::clip(rl, x0, y0, x1, y1);
        if !(ay < by && ax < bx) {
            return;
        }
        let (w, h) = (self.w, self.h);
        let zi = rl.z as usize & 63;
        let (lox, loy) = (self.back[zi].ox, self.back[zi].oy);
        for y in ay..by {
            for x in ax..bx {
                if let Some(id) = self.get_square(x, y, rl.z) {
                    let idx = x + (w * (y - loy) - lox);
                    self.sq_mut(id).idx = idx as u16;
                    let d = self.back[zi].data_mut(w, h);
                    d.lit[idx as usize & 0xffff] |= 1;
                }
            }
        }
    }

    /// `RoomLight::addInfluence`: every square of an active room light's
    /// rectangle gets the room light as a radius-7 entry at distance 0.
    fn room_add_influence(&mut self, rl: &RoomLight, x0: i32, y0: i32, x1: i32, y1: i32) {
        if !(rl.x - 7 < x1 && x0 < rl.w + rl.x + 7 && rl.y - 7 < y1 && y0 < rl.h + rl.y + 7) {
            return;
        }
        if !rl.active_snap[self.player as usize & 3] {
            if self.in_update_light_source {
                self.touched_by_light(rl.x, rl.y, rl.w + rl.x, rl.h + rl.y);
            }
            return;
        }
        let (ax, ay, bx, by) = Self::clip(rl, x0, y0, x1, y1);
        if !(ay < by && ax < bx) {
            return;
        }
        let (w, h) = (self.w, self.h);
        let zi = rl.z as usize & 63;
        for y in ay..by {
            for x in ax..bx {
                let Some(id) = self.get_square(x, y, rl.z) else { continue };
                let (sx, sy, sz) = {
                    let s = self.sq(id);
                    (s.x, s.y, i32::from(s.z))
                };
                let idx = (sx - self.ox) + (sy - self.oy) * w;
                self.sq_mut(id).idx = idx as u16;
                let list = &mut self.back[zi].data_mut(w, h).statics[idx as usize & 0xffff];
                list.add_packed(rl.id, sx, sy, sz, ROOM_LIGHT_RADIUS, ROOM_LIGHT_PACKED, 0.0);
            }
        }
    }
}
