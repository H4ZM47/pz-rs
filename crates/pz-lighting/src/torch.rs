//! Torches (flashlights, headlights): `LightingState::AddTorchLighting`,
//! `AddTorchLight`, `PropagateTorchToNeigbours` and `CalculateTorchRGB`.

use std::collections::VecDeque;

use crate::model::{distance_squared, fastfloor, flags, open, Torch};
use crate::session::Globals;
use crate::state::{LightingState, TorchCell};
use crate::world::nb;

impl LightingState {
    /// `LightingState::CalculateTorchRGB`: the colour a torch adds to the
    /// square at `(x, y)` reached over a path of length `path`.
    fn calculate_torch_rgb(&self, t: &Torch, path: f32, x: i32, y: i32, level: i32) -> Option<[i32; 3]> {
        let fx = x as f32;
        let fy = y as f32;
        let dx = t.x - (fx + 0.5);
        let dy = t.y - (fy + 0.5);
        let len = dx.mul_add(dx, dy * dy).sqrt();
        let (dirx, diry) = if len == 0.0 { (0.0, 0.0) } else { (dx / len, dy / len) };
        let ll = t.angle_x.mul_add(t.angle_x, t.angle_y * t.angle_y).sqrt();
        let (lx, ly) = if ll == 0.0 { (0.0, 0.0) } else { (t.angle_x / ll, t.angle_y / ll) };
        let dot0 = dirx.mul_add(lx, diry * ly);
        let dot = if t.focusing == 0 {
            dot0
        } else if t.focusing > 3 {
            dot0 - t.dist / (path * t.focusing as u32 as f32)
        } else {
            (dot0 * t.dist) / path
        };
        if !(distance_squared(fx, fy, t.x, t.y) < t.dist * t.dist) {
            return None;
        }
        let limit = if t.cone { -t.dot } else { 0.8 };
        if !(dot < limit || dot == -1.0) {
            return None;
        }
        if !(path < t.dist) {
            return None;
        }
        let chr_z = self.chr.as_ref().map(|c| c.z).unwrap_or(0.0);
        if fastfloor(chr_z) != level {
            return None;
        }
        let f = (path / t.dist).clamp(0.0, 1.0);
        let k = (-f).mul_add(f, 1.0) * 1.15;
        let mut cone = 1.0f32;
        if t.cone {
            let c = -dot - t.dot;
            let c = c + c;
            cone = if c > 1.0 { 1.0 } else { c };
        }
        let k = k * t.strength;
        let r = (cone * ((k * t.r) * 255.0)) as i32;
        let g = (cone * ((k * t.g) * 255.0)) as i32;
        let b = (cone * ((k * t.b) * 255.0)) as i32;
        Some([r, g, b])
    }

    /// `LightingState::AddTorchLight`.
    fn add_torch_light(&mut self, tid: usize, t: &Torch, x: i32, y: i32, level: i32, path: f32) -> bool {
        let (w, h) = (self.w, self.h);
        let ci = ((x - self.ox) + w * (y - self.oy)) as usize;
        let mut cell = self.torch_cells[ci];
        if cell.torch == Some(tid) {
            if cell.dist <= path {
                return false;
            }
        } else {
            cell.torch = Some(tid);
        }
        cell.dist = path;
        let rgb = self.calculate_torch_rgb(t, path, x, y, level);
        if let Some([r, g, b]) = rgb {
            let clamp = |v: i32| if v > 0xfe { 0xff } else { v };
            let r = (clamp(r) & 0xff) as u8;
            let g = (clamp(g) & 0xff) as u8;
            let b = (clamp(b) & 0xff) as u8;
            cell.rgb = [cell.rgb[0].max(r), cell.rgb[1].max(g), cell.rgb[2].max(b)];
            self.torch_cells[ci] = cell;
            let list = &mut self.back[level as usize & 63].data_mut(w, h).torches[ci];
            list.add_light(
                t.id,
                t.x as i32,
                t.y as i32,
                t.z as i32,
                t.dist as i32,
                f32::from(cell.rgb[0]) / 255.0,
                f32::from(cell.rgb[1]) / 255.0,
                f32::from(cell.rgb[2]) / 255.0,
                2,
                path,
            );
        } else {
            self.torch_cells[ci] = cell;
        }
        true
    }

    #[inline]
    fn in_grid(&self, x: i32, y: i32) -> bool {
        let lx = x - self.ox;
        let ly = y - self.oy;
        lx >= 0 && lx < self.w && ly >= 0 && ly < self.h
    }

    /// `LightingState::PropagateTorchToNeigbours`.
    #[allow(clippy::too_many_arguments)]
    fn propagate_torch(&mut self, tid: usize, t: &Torch, x: i32, y: i32, z: i32, path: f32, q: &mut VecDeque<[i32; 3]>) {
        let Some(id) = self.get_square(x, y, z) else { return };
        let (o, f, n) = {
            let s = self.sq(id);
            (s.open, s.flags, s.nb)
        };
        let r2 = t.dist * t.dist;
        // (taken, neighbour slot, dx, dy, step)
        let moves: [(bool, usize, i32, i32, f32); 8] = [
            (o & open::W != 0, nb::W, -1, 0, 1.0),
            (o & open::E != 0, nb::E, 1, 0, 1.0),
            (o & open::N != 0, nb::N, 0, -1, 1.0),
            (o & open::S != 0, nb::S, 0, 1, 1.0),
            (f & flags::NW != 0, nb::NW, -1, -1, std::f32::consts::SQRT_2),
            (f & flags::SW != 0, nb::SW, -1, 1, std::f32::consts::SQRT_2),
            (f & flags::NE != 0, nb::NE, 1, -1, std::f32::consts::SQRT_2),
            (f & flags::SE != 0, nb::SE, 1, 1, std::f32::consts::SQRT_2),
        ];
        for (taken, slot, dx, dy, step) in moves {
            if !taken {
                continue;
            }
            let Some(nid) = n[slot] else { continue };
            let (nx, ny) = (x + dx, y + dy);
            // The cardinal moves check the unchanged axis too.
            if !self.in_grid(nx, ny) {
                continue;
            }
            if !(distance_squared(nx as f32 + 0.5, ny as f32 + 0.5, t.x, t.y) < r2) {
                continue;
            }
            if self.add_torch_light(tid, t, nx, ny, z, path + step) {
                let s = self.sq(nid);
                q.push_back([s.x, s.y, i32::from(s.z)]);
            }
        }
    }

    /// `LightingState::AddTorchLighting`: floods each torch on the
    /// character's level.
    pub(crate) fn add_torch_lighting(&mut self, g: &Globals) {
        self.torch_cells.fill(TorchCell { torch: None, dist: 0.0, rgb: [0; 3] });
        let chr_z = self.chr.as_ref().map(|c| c.z).unwrap_or(0.0);
        let torches = self.torches.clone();
        for tid in torches {
            let t = g.torches[tid];
            if t.id == 0 || fastfloor(t.z) != fastfloor(chr_z) {
                continue;
            }
            let mut q: VecDeque<[i32; 3]> = VecDeque::new();
            let (tx, ty, tz) = (t.x as i32, t.y as i32, t.z as i32);
            let lx = tx - self.ox;
            if lx >= 0 {
                let ly = ty - self.oy;
                if ly >= 0 && lx < self.w && ly < self.h {
                    self.add_torch_light(tid, &t, tx, ty, tz, 0.0);
                    q.push_back([tx, ty, tz]);
                }
            }
            while let Some([x, y, z]) = q.pop_front() {
                let ci = ((x - self.ox) + (y - self.oy) * self.w) as usize;
                let path = self.torch_cells.get(ci).map(|c| c.dist).unwrap_or(0.0);
                self.propagate_torch(tid, &t, x, y, z, path, &mut q);
            }
        }
    }
}
