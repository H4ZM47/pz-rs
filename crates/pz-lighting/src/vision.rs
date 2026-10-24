//! Line of sight and visibility: `LightingObject::TestVision`,
//! `LightingSquare::testVisionAdjacent`, `LightingState::lineClearCached`
//! and `LightingSquare::CalcVisibility`.

use crate::model::{fastfloor, flags, Character, LightingObject, ObjectKind, VisionResult};
use crate::state::LightingState;
use crate::world::SqId;

const CLEAR: u8 = 0;
const BLOCKED: u8 = 3;

/// `LightingObject::TestVision(from, to)`.
fn test_vision(st: &LightingState, obj: &LightingObject, from: SqId, to: SqId) -> VisionResult {
    use VisionResult::*;
    let f = obj.flags;
    let a = st.sq(from);
    let b = st.sq(to);
    if b.z != a.z {
        return NoEffect;
    }
    // Doors and thumpables swing: when open, the blocking edge flips.
    let swung = if f & 1 == 0 { (f & 2) >> 1 } else { u16::from(f & 2 == 0) };
    match obj.kind {
        ObjectKind::Curtain => {
            let m = if obj.owner == from { f & 0xc } else { f & 0x30 };
            let no = if m == 0 {
                if f & 2 == 0 {
                    b.x <= a.x
                } else {
                    b.y <= a.y
                }
            } else if f & 2 == 0 {
                a.x <= b.x
            } else {
                a.y <= b.y
            };
            if no || f & 1 != 0 {
                NoEffect
            } else {
                Blocked
            }
        }
        ObjectKind::Door => {
            let r = if f & 4 != 0 { Unblocked } else { Blocked };
            let hit = if obj.owner == from {
                if swung == 0 {
                    b.x < a.x
                } else {
                    b.y < a.y
                }
            } else if swung == 0 {
                a.x < b.x
            } else {
                a.y < b.y
            };
            if hit {
                r
            } else {
                NoEffect
            }
        }
        ObjectKind::Thumpable => {
            if f & 8 != 0 {
                return NoEffect;
            }
            let hit = if obj.owner == from {
                if swung == 0 {
                    b.x < a.x
                } else {
                    b.y < a.y
                }
            } else if swung == 0 {
                a.x < b.x
            } else {
                a.y < b.y
            };
            if hit {
                Blocked
            } else {
                NoEffect
            }
        }
        ObjectKind::Window => {
            let r = if f & 4 == 0 { Unblocked } else { Blocked };
            let hit = if obj.owner == from {
                if f & 2 != 0 {
                    b.y < a.y
                } else {
                    b.x < a.x
                }
            } else if f & 2 != 0 {
                a.y < b.y
            } else {
                a.x < b.x
            };
            if hit {
                r
            } else {
                NoEffect
            }
        }
    }
}

impl LightingState {
    /// `LightingSquare::testVisionAdjacent(dx, dy, dz, specialDiag,
    /// ignoreDoors)`, returning a `TestResult` ordinal.
    pub(crate) fn test_vision_adjacent(
        &self,
        mut this: SqId,
        mut dx: i32,
        mut dy: i32,
        mut dz: i32,
        mut special: bool,
        ignore_doors: bool,
    ) -> u8 {
        let unit = |v: i32| (-1..=1).contains(&v);
        loop {
            if !unit(dx) || !unit(dy) || !unit(dz) {
                return BLOCKED;
            }
            if dx != 0 || dy != 0 {
                let s = self.sq(this);
                if dz == 1 && s.flags & flags::ELEVATED != 0 {
                    if let Some(above) = self.get_square(s.x, s.y, i32::from(s.z) + 1) {
                        this = above;
                        dz = 0;
                    }
                } else if dz == -1 {
                    if let Some(below) = self.get_square(s.x + dx, s.y + dy, i32::from(s.z) - 1) {
                        if self.sq(below).flags & flags::ELEVATED != 0 {
                            dz = 0;
                        }
                    }
                }
            }
            let diag = dx != 0 && dy != 0;
            if !(diag && special) {
                return self.test_vision_single(this, dx, dy, dz, ignore_doors);
            }
            let r1 = self.test_vision_adjacent(this, dx, 0, dz, false, ignore_doors);
            if r1 == BLOCKED {
                return BLOCKED;
            }
            let r2 = self.test_vision_adjacent(this, 0, dy, dz, false, ignore_doors);
            if r2 == BLOCKED {
                return BLOCKED;
            }
            let mut res = 2;
            if r1 != 2 && r2 != 2 {
                res = self.test_vision_adjacent(this, dx, dy, dz, false, ignore_doors);
                if res > 4 || (0x17u32 >> res) & 1 == 0 {
                    return res;
                }
            }
            let s = self.sq(this);
            let Some(next) = self.get_square(s.x + dx, s.y + dy, i32::from(s.z) + dz) else {
                return res;
            };
            this = next;
            dx = -dx;
            dz = -dz;
            let r1 = self.test_vision_adjacent(this, dx, 0, dz, false, ignore_doors);
            if r1 == BLOCKED {
                return BLOCKED;
            }
            dy = -dy;
            let r2 = self.test_vision_adjacent(this, 0, dy, dz, false, ignore_doors);
            if r2 == BLOCKED {
                return BLOCKED;
            }
            if r1 == 2 {
                return 2;
            }
            special = false;
            if r2 == 2 {
                return 2;
            }
        }
    }

    /// The non-diagonal part of `testVisionAdjacent`.
    fn test_vision_single(&self, this: SqId, dx: i32, dy: i32, dz: i32, ignore_doors: bool) -> u8 {
        use VisionResult::*;
        let s = self.sq(this);
        let mut res = CLEAR;
        if let Some(other) = self.get_square(s.x + dx, s.y + dy, i32::from(s.z) + dz) {
            let o = self.sq(other);
            if o.z == s.z {
                for obj in &s.objects {
                    match test_vision(self, obj, this, other) {
                        Blocked => match obj.kind {
                            ObjectKind::Curtain | ObjectKind::Window => return BLOCKED,
                            ObjectKind::Door if !ignore_doors => return BLOCKED,
                            ObjectKind::Thumpable if !ignore_doors && obj.flags & 4 != 0 => return BLOCKED,
                            _ => {}
                        },
                        Unblocked => match obj.kind {
                            ObjectKind::Door => res = if obj.flags & 1 != 0 { 1 } else { 4 },
                            ObjectKind::Window => res = 2,
                            ObjectKind::Thumpable if obj.flags & 4 != 0 => res = 1,
                            _ => {}
                        },
                        NoEffect => {}
                    }
                }
                for obj in &o.objects {
                    match test_vision(self, obj, this, other) {
                        Blocked => match obj.kind {
                            ObjectKind::Curtain | ObjectKind::Window => return BLOCKED,
                            ObjectKind::Door if !ignore_doors => return BLOCKED,
                            ObjectKind::Thumpable if !ignore_doors && obj.flags & 4 != 0 => return BLOCKED,
                            _ => {}
                        },
                        Unblocked => match obj.kind {
                            ObjectKind::Door => res = if obj.flags & 1 != 0 { 1 } else { 4 },
                            ObjectKind::Window => res = 2,
                            ObjectKind::Thumpable if obj.flags & 4 != 0 => res = 1,
                            _ => {}
                        },
                        NoEffect => {}
                    }
                }
            }
        }
        let bit = (dx + dy * 3 + dz * 9 + 13) as u32 & 31;
        if (s.vision_matrix >> bit) & 1 != 0 {
            res = BLOCKED;
        }
        res
    }

    /// `LightingState::lineClearCached(x1, y1, z1, x0, y0, z0,
    /// ignoreDoors)`: line of sight from `(x0, y0, z0)` to `(x1, y1, z1)`,
    /// memoised in the per-state cache relative to the start square.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn line_clear_cached(
        &mut self,
        x1: i32,
        y1: i32,
        mut z1: i32,
        x0: i32,
        y0: i32,
        z0: i32,
        ignore_doors: bool,
    ) -> u8 {
        if z0.wrapping_sub(1) == z1 {
            if let Some(s) = self.get_square(x1, y1, z1) {
                if self.sq(s).flags & flags::ELEVATED != 0 {
                    z1 = z0;
                }
            }
        }
        let dx = x1.wrapping_sub(x0);
        let dy = y1.wrapping_sub(y0);
        let dz = z1.wrapping_sub(z0);
        if !(-100..=99).contains(&dx) || !(-100..=99).contains(&dy) || !(-16..=15).contains(&dz) {
            return BLOCKED;
        }
        let cache_index = |cx: i32, cy: i32, cz: i32| -> usize {
            ((cz + 16) + (cy + 100) * 32 + (cx + 100) * 6400) as usize
        };
        let ci = cache_index(dx, dy, dz);
        let cached = self.los[ci];
        if cached != 0 {
            return match cached {
                2 => 1,
                3 => 2,
                4 => 3,
                5 => 4,
                _ => 0,
            };
        }
        let mut b = self.get_square(x0, y0, z0);
        let fx = dx as f32;
        let fy = dy as f32;
        let fz = dz as f32;
        let (ax, ay, az) = (fx.abs(), fy.abs(), fz.abs());
        let mut state = 1u8;
        let step_once = |st: &mut LightingState, x: i32, y: i32, z: i32, state: &mut u8, b: &mut Option<SqId>| {
            let a = st.get_square(x, y, z);
            if let (Some(a), Some(bb)) = (a, *b) {
                if *state != 4 {
                    let (sa, sb) = (st.sq(a), st.sq(bb));
                    let r = st.test_vision_adjacent(
                        a,
                        sb.x - sa.x,
                        sb.y - sa.y,
                        i32::from(sb.z) - i32::from(sa.z),
                        true,
                        ignore_doors,
                    );
                    if r == BLOCKED {
                        *state = 4;
                    }
                }
            }
            let idx = cache_index(x - x0, y - y0, z - z0);
            if let Some(c) = st.los.get_mut(idx) {
                if *c == 0 {
                    *c = *state;
                }
            }
            *b = a;
        };
        if ay < ax && ax > az {
            // x-major
            if x0 != x1 {
                let step = if dx < 0 { -1 } else { 1 };
                let sf = step as f32;
                let mut ty = y0 as f32 + 0.5;
                let mut tz = z0 as f32 + 0.5;
                let mut off = 0;
                loop {
                    ty += (fy / fx) * sf;
                    tz += (fz / fx) * sf;
                    let (yy, zz) = (fastfloor(ty), fastfloor(tz));
                    step_once(self, x0 + step + off, yy, zz, &mut state, &mut b);
                    off += step;
                    if off == dx {
                        break;
                    }
                }
            }
        } else if ax <= ay && ay > az {
            // y-major
            if y0 != y1 {
                let step = if dy < 0 { -1 } else { 1 };
                let sf = step as f32;
                let mut tx = x0 as f32 + 0.5;
                let mut tz = z0 as f32 + 0.5;
                let mut cy = y0 + step;
                let mut count = y0 - y1;
                loop {
                    tx += (fx / fy) * sf;
                    tz += (fz / fy) * sf;
                    let (xx, zz) = (fastfloor(tx), fastfloor(tz));
                    step_once(self, xx, cy, zz, &mut state, &mut b);
                    cy += step;
                    count += step;
                    if count == 0 {
                        break;
                    }
                }
            }
        } else if z0 != z1 {
            // z-major
            let step = if dz < 0 { -1 } else { 1 };
            let sf = step as f32;
            let mut tx = x0 as f32 + 0.5;
            let mut ty = y0 as f32 + 0.5;
            let mut cz = z0 + step;
            let mut count = z0 - z1;
            loop {
                tx += (fx / fz) * sf;
                ty += (fy / fz) * sf;
                let (xx, yy) = (fastfloor(tx), fastfloor(ty));
                step_once(self, xx, yy, cz, &mut state, &mut b);
                cz += step;
                count += step;
                if count == 0 {
                    break;
                }
            }
        }
        if state == 1 {
            self.los[ci] = 1;
            CLEAR
        } else {
            self.los[ci] = 4;
            BLOCKED
        }
    }

    /// `LightingSquare::CalcVisibility`.
    pub(crate) fn calc_visibility(&mut self, id: SqId, chr: &Character) {
        let (w, h) = (self.w, self.h);
        let (sx, sy, sz, idx, room) = {
            let s = self.sq(id);
            (s.x, s.y, i32::from(s.z), s.idx as usize, s.room_id)
        };
        let zi = sz as usize & 63;
        if chr.see_all && !chr.reanimated {
            let d = self.back[zi].data_mut(w, h);
            d.vis[idx] |= 1 | 2 | 4;
            // The native code leaves the target untouched here and so reads
            // uninitialised memory; a fully visible square has target 1.
            d.target[idx] = 1.0;
            d.set_dirty(w, idx);
            return;
        }
        let Some(csq) = self.chr_square else { return };
        let dx = chr.x - (sx as f32 + 0.5);
        let dy = chr.y - (sy as f32 + 0.5);
        let len = dx.mul_add(dx, dy * dy).sqrt();
        let (dirx, diry) = if len != 0.0 { (dx / len, dy / len) } else { (0.0, 0.0) };
        let vis_byte = self.back[zi].data_mut(w, h).vis[idx];
        let ll = chr.look_x.mul_add(chr.look_x, chr.look_y * chr.look_y).sqrt();
        let (lx, ly) = if ll == 0.0 { (0.0, 0.0) } else { (chr.look_x / ll, chr.look_y / ll) };
        let thr = chr.perception * (1.0 - chr.fatigue);
        let dot = if len < thr || csq == id { -1.0 } else { dirx.mul_add(lx, diry * ly) };
        let res = self.line_clear_cached(
            sx,
            sy,
            sz,
            fastfloor(chr.x),
            fastfloor(chr.y),
            fastfloor(chr.z),
            false,
        );
        let can_see = dot <= chr.cone && res != BLOCKED;
        let could_see = res != BLOCKED;
        let mut seen = vis_byte & 1;
        let target = if can_see {
            seen = 1;
            1.0f32
        } else if seen != 0 {
            1.0
        } else {
            0.0
        };
        if could_see && room != -1 {
            self.visible_rooms_work.insert(room as u64);
        }
        let t1000 = (target * 1000.0) as i32;
        let s = &mut self.squares[id as usize];
        let d = self.back[zi].data_mut(w, h);
        if s.prev_seen != seen
            || s.prev_can_see != u8::from(can_see)
            || s.prev_could_see != u8::from(could_see)
            || s.prev_target != t1000
        {
            d.set_dirty(w, idx);
        }
        s.prev_seen = seen;
        s.prev_can_see = u8::from(can_see);
        s.prev_could_see = u8::from(could_see);
        let mut v = (d.vis[idx] & 0xfe) | seen;
        v = (v & 0xfd) | if can_see { 2 } else { 0 };
        v = (v & 0xfb) | if could_see { 4 } else { 0 };
        d.vis[idx] = v;
        d.target[idx] = target;
        s.prev_target = t1000;
    }
}
