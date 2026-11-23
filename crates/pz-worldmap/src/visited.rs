//! `zombie.worldMap.WorldMapVisited`: the fog of war. Two bits per
//! 32x32-square unit (eight units per cell): known (`BIT_KNOWN`, e.g. from
//! a map item) and visited (`BIT_VISITED`, within 25 squares of a
//! player), saved to `map_visited.bin`.

use crate::vbo::{Prim, Vbo};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::Path;

pub const RADIUS: i32 = 25;
pub const UNITS_PER_CELL: i32 = 8;
pub const SQUARES_PER_UNIT: i32 = 32;
pub const BIT_VISITED: i32 = 1;
pub const BIT_KNOWN: i32 = 2;
pub const HEADER_BYTES: usize = 28;
/// `VERSION`.
pub const VERSION: i32 = 2;
/// The world version written into the header.
pub const WORLD_VERSION: i32 = 249;

/// `IsoMetaGrid`'s cell bounds, which the original reads statically.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MetaBounds {
    pub min_x: i32,
    pub min_y: i32,
    pub max_x: i32,
    pub max_y: i32,
}

impl MetaBounds {
    /// `IsoMetaGrid`'s bounds before any map is loaded.
    pub const UNLOADED: MetaBounds = MetaBounds { min_x: 10000000, min_y: 10000000, max_x: -10000000, max_y: -10000000 };

    fn width(&self) -> i32 {
        self.max_x.wrapping_sub(self.min_x).wrapping_add(1)
    }
    fn height(&self) -> i32 {
        self.max_y.wrapping_sub(self.min_y).wrapping_add(1)
    }
    /// `getSpan()`: bytes per row.
    fn span(&self) -> i32 {
        self.width().wrapping_mul(8) / 4
    }
    /// `getVisitedLength()`.
    pub fn visited_length(&self) -> i32 {
        self.span().wrapping_mul(self.height()).wrapping_mul(8)
    }
}

/// `zombie.worldMap.WorldMapVisited`.
pub struct WorldMapVisited {
    meta: MetaBounds,
    min_x: i32,
    min_y: i32,
    max_x: i32,
    max_y: i32,
    pub visited: Vec<u8>,
    span: i32,
    changed: bool,
    change: [i32; 4],
    save_changed: bool,
    save_change: [i32; 4],
    update_min_x: [i32; 4],
    update_min_y: [i32; 4],
    update_max_x: [i32; 4],
    update_max_y: [i32; 4],
    /// `color`: the unvisited colour.
    pub color: [f32; 4],
    pub grid_color: [f32; 4],
    pub main_menu: bool,
    pub client: bool,
}

const NO_CHANGE: [i32; 4] = [i32::MAX, i32::MAX, i32::MIN, i32::MIN];

impl WorldMapVisited {
    /// `new WorldMapVisited()` then `setBounds` with the meta grid's
    /// bounds. An unloaded grid makes `setBounds` throw
    /// (`NegativeArraySizeException`) after the object exists; the error
    /// is returned with it.
    pub fn new(meta: MetaBounds) -> (Self, Option<&'static str>) {
        let c = [0.858_823_54, 0.843_137_26, 0.752_941_2, 1.0];
        let mut v = WorldMapVisited {
            meta,
            min_x: 0,
            min_y: 0,
            max_x: 0,
            max_y: 0,
            visited: Vec::new(),
            span: 0,
            changed: false,
            change: NO_CHANGE,
            save_changed: false,
            save_change: NO_CHANGE,
            update_min_x: [-1; 4],
            update_min_y: [-1; 4],
            update_max_x: [-1; 4],
            update_max_y: [-1; 4],
            color: c,
            grid_color: [c[0] * 0.85, c[1] * 0.85, c[2] * 0.85, 1.0],
            main_menu: false,
            client: false,
        };
        let err = v.set_bounds(meta.min_x, meta.min_y, meta.max_x, meta.max_y);
        (v, err)
    }

    /// `setBounds(minX, minY, maxX, maxY)`.
    pub fn set_bounds(&mut self, mut min_x: i32, mut min_y: i32, mut max_x: i32, mut max_y: i32) -> Option<&'static str> {
        if min_x > max_x || min_y > max_y {
            (min_x, min_y, max_x, max_y) = (0, 0, 0, 0);
            self.main_menu = true;
        }
        (self.min_x, self.min_y, self.max_x, self.max_y) = (min_x, min_y, max_x, max_y);
        self.changed = true;
        self.change = [0, 0, self.meta.width().wrapping_mul(8) - 1, self.meta.height().wrapping_mul(8) - 1];
        self.save_changed = false;
        self.save_change = NO_CHANGE;
        self.span = self.meta.span();
        let len = self.meta.visited_length();
        if len < 0 {
            return Some("java.lang.NegativeArraySizeException");
        }
        self.visited = vec![0; len as usize];
        None
    }

    pub fn get_min_x(&self) -> i32 {
        self.min_x
    }
    pub fn get_min_y(&self) -> i32 {
        self.min_y
    }

    fn width_in_cells(&self) -> i32 {
        self.meta.width()
    }
    fn height_in_cells(&self) -> i32 {
        self.meta.height()
    }

    /// `getFlags(x, y)` of a unit.
    pub fn get_flags(&self, x: i32, y: i32) -> i32 {
        let b = self.visited[(x / 4 + y * self.span) as usize] as i8 as i32;
        (b >> (x % 4 * 2)) & 3
    }

    fn put_flags(&mut self, x: i32, y: i32, flags: i32) {
        let i = (x / 4 + y * self.span) as usize;
        let shift = x % 4 * 2;
        let b = (self.visited[i] as i32 & !(3 << shift)) | flags << shift;
        self.visited[i] = b as u8;
    }

    fn normalize_x(&self, x: i32) -> i32 {
        (x - self.meta.min_x * 256).clamp(0, (self.width_in_cells() * 256 - 1).max(0))
    }
    fn normalize_y(&self, y: i32) -> i32 {
        (y - self.meta.min_y * 256).clamp(0, (self.height_in_cells() * 256 - 1).max(0))
    }

    /// `coordToUnit(coord, isMax)`: the unit of a square; for a maximum,
    /// one less when the unit number is a multiple of 32 (the original
    /// tests the unit, not the square).
    fn coord_to_unit(coord: i32, is_max: bool) -> i32 {
        let c = coord / 32;
        if is_max && c % 32 == 0 {
            c - 1
        } else {
            c
        }
    }

    fn mark(&mut self, xmin: i32, ymin: i32, xmax: i32, ymax: i32) {
        self.changed = true;
        self.change = [self.change[0].min(xmin), self.change[1].min(ymin), self.change[2].max(xmax), self.change[3].max(ymax)];
        self.save_changed = true;
        self.save_change = [self.save_change[0].min(xmin), self.save_change[1].min(ymin), self.save_change[2].max(xmax), self.save_change[3].max(ymax)];
    }

    /// `setFlags(xMin, yMin, xMax, yMax, flags)` in squares.
    pub fn set_flags(&mut self, x_min: i32, y_min: i32, x_max: i32, y_max: i32, flags: i32) {
        if self.visited.is_empty() {
            return;
        }
        let (x0, y0, x1, y1) = (self.normalize_x(x_min), self.normalize_y(y_min), self.normalize_x(x_max), self.normalize_y(y_max));
        if x0 == x1 || y0 == y1 {
            return;
        }
        let (ux0, ux1, uy0, uy1) = (Self::coord_to_unit(x0, false), Self::coord_to_unit(x1, true), Self::coord_to_unit(y0, false), Self::coord_to_unit(y1, true));
        let mut changed = false;
        for y in uy0..=uy1 {
            for x in ux0..=ux1 {
                if !self.in_range(x, y) {
                    return;
                }
                let it = self.get_flags(x, y);
                if it & flags == flags {
                    continue;
                }
                self.put_flags(x, y, it | flags);
                changed = true;
            }
        }
        if changed {
            let (cx0, cx1) = (Self::coord_to_unit(self.normalize_x(x_min), false), Self::coord_to_unit(self.normalize_x(x_max), true));
            let (cy0, cy1) = (Self::coord_to_unit(self.normalize_y(y_min), false), Self::coord_to_unit(self.normalize_y(y_max), true));
            self.mark(cx0, cy0, cx1, cy1);
        }
    }

    /// The original indexes past the array here and throws; stop instead.
    fn in_range(&self, x: i32, y: i32) -> bool {
        x >= 0 && y >= 0 && ((x / 4 + y * self.span) as usize) < self.visited.len()
    }

    /// `clearFlags(xMin, yMin, xMax, yMax, flags)`.
    pub fn clear_flags(&mut self, x_min: i32, y_min: i32, x_max: i32, y_max: i32, flags: i32) {
        if self.visited.is_empty() {
            return;
        }
        let (x0, y0, x1, y1) = (self.normalize_x(x_min), self.normalize_y(y_min), self.normalize_x(x_max), self.normalize_y(y_max));
        if x0 == x1 || y0 == y1 {
            return;
        }
        let (ux0, ux1, uy0, uy1) = (Self::coord_to_unit(x0, false), Self::coord_to_unit(x1, true), Self::coord_to_unit(y0, false), Self::coord_to_unit(y1, true));
        let mut changed = false;
        for y in uy0..=uy1 {
            for x in ux0..=ux1 {
                if !self.in_range(x, y) {
                    return;
                }
                let it = self.get_flags(x, y);
                if it & flags == 0 {
                    continue;
                }
                self.put_flags(x, y, it & !flags);
                changed = true;
            }
        }
        if changed {
            self.mark(ux0, uy0, ux1, uy1);
        }
    }

    pub fn set_known_in_cells(&mut self, x0: i32, y0: i32, x1: i32, y1: i32) {
        self.set_flags(x0 * 256, y0 * 256, (x1 + 1) * 256, (y1 + 1) * 256, BIT_KNOWN);
    }
    pub fn clear_known_in_cells(&mut self, x0: i32, y0: i32, x1: i32, y1: i32) {
        self.clear_flags(x0 * 256, y0 * 256, (x1 + 1) * 256, (y1 + 1) * 256, BIT_KNOWN);
    }
    pub fn set_visited_in_cells(&mut self, x0: i32, y0: i32, x1: i32, y1: i32) {
        self.set_flags(x0 * 256, y0 * 256, x1 * 256, y1 * 256, BIT_VISITED);
    }
    pub fn clear_visited_in_cells(&mut self, x0: i32, y0: i32, x1: i32, y1: i32) {
        self.clear_flags(x0 * 256, y0 * 256, x1 * 256, y1 * 256, BIT_VISITED);
    }

    /// `hasFlags(xMin, yMin, xMax, yMax, flags, bAnyOK)`.
    pub fn has_flags(&self, x_min: i32, y_min: i32, x_max: i32, y_max: i32, flags: i32, any_ok: bool) -> bool {
        if self.visited.is_empty() {
            return false;
        }
        let (w, h) = (self.width_in_cells() * 256 - 1, self.height_in_cells() * 256 - 1);
        let x0 = (x_min - self.min_x * 256).clamp(0, w.max(0));
        let y0 = (y_min - self.min_y * 256).clamp(0, h.max(0));
        let x1 = (x_max - self.min_x * 256).clamp(0, w.max(0));
        let y1 = (y_max - self.min_y * 256).clamp(0, h.max(0));
        if x0 == x1 || y0 == y1 {
            return false;
        }
        let (ux0, uy0) = (x0 / 32, y0 / 32);
        let mut ux1 = x1 / 32;
        let mut uy1 = y1 / 32;
        if x1 % 32 == 0 {
            ux1 -= 1;
        }
        if y1 % 32 == 0 {
            uy1 -= 1;
        }
        for y in uy0..=uy1 {
            for x in ux0..=ux1 {
                let it = self.get_flags(x, y);
                if any_ok {
                    if it & flags != 0 {
                        return true;
                    }
                } else if it & flags != flags {
                    return false;
                }
            }
        }
        !any_ok
    }

    /// `isCellVisible(cellX, cellY)`.
    pub fn is_cell_visible(&self, cx: i32, cy: i32) -> bool {
        self.has_flags(cx * 256 - 1, cy * 256 - 1, (cx + 1) * 256 + 1, (cy + 1) * 256 + 1, 3, true)
    }
    pub fn is_known(&self, x1: i32, y1: i32, x2: i32, y2: i32) -> bool {
        self.has_flags(x1 - 1, y1 - 1, x2 + 1, y2 + 1, BIT_KNOWN, true)
    }
    pub fn is_visited(&self, x1: i32, y1: i32, x2: i32, y2: i32) -> bool {
        self.has_flags(x1 - 1, y1 - 1, x2 + 1, y2 + 1, BIT_VISITED, true)
    }

    /// `update()`: marks the 25-square radius around each live player
    /// when the units it covers change. `players[i]` is player i's
    /// position.
    pub fn update(&mut self, players: &[Option<(f32, f32)>]) {
        for (i, p) in players.iter().enumerate().take(4) {
            let Some((px, py)) = *p else { continue };
            let (x, y) = (px as i32, py as i32);
            let mut ux1 = (x + RADIUS) / 32;
            let mut uy1 = (y + RADIUS) / 32;
            let (ux0, uy0) = ((x - RADIUS) / 32, (y - RADIUS) / 32);
            if (x + RADIUS) % 32 == 0 {
                ux1 -= 1;
            }
            if (y + RADIUS) % 32 == 0 {
                uy1 -= 1;
            }
            if [ux0, uy0, ux1, uy1] == [self.update_min_x[i], self.update_min_y[i], self.update_max_x[i], self.update_max_y[i]] {
                continue;
            }
            self.update_min_x[i] = ux0;
            self.update_min_y[i] = uy0;
            self.update_max_x[i] = ux1;
            self.update_max_y[i] = uy1;
            self.set_flags(x - RADIUS, y - RADIUS, x + RADIUS, y + RADIUS, 3);
        }
    }

    /// `forget()`.
    pub fn forget(&mut self) {
        let (a, b, c, d) = (self.min_x, self.min_y, self.max_x, self.max_y);
        self.clear_known_in_cells(a, b, c, d);
        self.clear_visited_in_cells(a, b, c, d);
        self.update_min_x = [-1; 4];
        self.update_min_y = [-1; 4];
        self.update_max_x = [-1; 4];
        self.update_max_y = [-1; 4];
    }

    fn header(&self) -> Vec<u8> {
        let mut h = Vec::with_capacity(HEADER_BYTES);
        for v in [WORLD_VERSION, VERSION, self.min_x, self.min_y, self.max_x, self.max_y, UNITS_PER_CELL] {
            h.extend(v.to_be_bytes());
        }
        h
    }

    /// `save()`: the whole file.
    pub fn save(&self, path: &Path) -> io::Result<()> {
        let mut f = io::BufWriter::new(std::fs::File::create(path)?);
        f.write_all(&self.header())?;
        f.write_all(&self.visited)?;
        f.flush()
    }

    /// `saveToBufferMap` + `writeBufferMap`: a new file is written whole;
    /// an existing one gets only the changed rows of units rewritten.
    pub fn save_changes(&mut self, path: &Path) -> io::Result<()> {
        if !path.exists() {
            return self.save(path);
        }
        if !self.save_changed {
            return Ok(());
        }
        let [x1, y1, x2, y2] = self.save_change;
        let (b1, b2) = (x1 / 4, x2 / 4);
        let span = (b2 - b1 + 1) as usize;
        let mut f = std::fs::OpenOptions::new().write(true).open(path)?;
        for y in y1..=y2 {
            let at = (b1 + y * self.span) as usize;
            f.seek(SeekFrom::Start((HEADER_BYTES + b1 as usize) as u64 + y as u64 * self.span as u64))?;
            f.write_all(&self.visited[at..at + span])?;
        }
        self.save_changed = false;
        self.save_change = NO_CHANGE;
        Ok(())
    }

    /// `load()`: a file with other bounds or units (or version 1) is
    /// replayed unit by unit; a missing file leaves everything unset.
    pub fn load(&mut self, path: &Path) -> io::Result<()> {
        let mut f = match std::fs::File::open(path) {
            Ok(f) => io::BufReader::new(f),
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(()),
            Err(e) => return Err(e),
        };
        let mut head = [0u8; 24];
        f.read_exact(&mut head)?;
        let int = |b: &[u8], i: usize| i32::from_be_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]]);
        let world_version = int(&head, 0);
        let mut ints: Vec<i32> = (1..6).map(|i| int(&head, i * 4)).collect();
        let mut version = 1;
        if world_version >= 234 {
            version = ints.remove(0);
            if !(1..=2).contains(&version) {
                return Err(io::Error::new(io::ErrorKind::InvalidData, format!("unknown file version {version}")));
            }
            let mut more = [0u8; 4];
            f.read_exact(&mut more)?;
            ints.push(i32::from_be_bytes(more));
        }
        let (min_x, min_y, max_x, max_y, upc) = (ints[0], ints[1], ints[2], ints[3], ints[4]);
        if version < 2 || min_x != self.min_x || min_y != self.min_y || max_x != self.max_x || max_y != self.max_y || upc != 8 {
            let mut row = vec![0u8; ((max_x - min_x + 1) * upc).max(0) as usize];
            let spu = 256 / upc.max(1);
            for y in min_y * upc..=max_y * upc {
                let n = f.read(&mut row)?;
                let _ = n;
                for x in min_x * upc..=max_x * upc {
                    let xx = (x - min_x * upc) as usize;
                    let flags = row.get(xx).copied().unwrap_or(0) as i8 as i32;
                    self.set_flags(x * spu, y * spu, x * spu + spu - 1, y * spu + spu - 1, flags);
                }
            }
            return Ok(());
        }
        let mut buf = Vec::new();
        f.read_to_end(&mut buf)?;
        let n = buf.len().min(self.visited.len());
        self.visited[..n].copy_from_slice(&buf[..n]);
        Ok(())
    }

    /// The `worldMapVisited` shader for one unit: (r, g, b, a) over the
    /// unvisited colour.
    fn unit_color(&self, x: i32, y: i32) -> [f32; 4] {
        let w = self.width_in_cells() * 8;
        let h = self.height_in_cells() * 8;
        // Outside the bounds the texture holds its initial texels.
        let flags = if x < 0 || y < 0 || x >= w || y >= h { 0 } else { self.get_flags(x, y) };
        let tr = if flags & BIT_KNOWN != 0 { 0.0 } else { 1.0 };
        let tg = if flags & BIT_VISITED != 0 { 0.0 } else { 1.0 };
        let c = self.color;
        let hidden = [c[0], c[1], c[2], tg * tr];
        let known = [0.5, 0.5, 0.5, 0.5 * tg];
        let t = 1.0 - tr;
        std::array::from_fn(|i| hidden[i] + (known[i] - hidden[i]) * t)
    }

    /// `render(...)`: the overlay over cells `min..=max` as per-unit quads
    /// (rows of equal units merged), with the shader's colour per unit.
    /// The original samples the unit texture in a fragment shader (linear
    /// when blurred); this draws the unblurred result.
    #[allow(clippy::too_many_arguments)]
    pub fn render(&self, vbo: &mut Vbo, render_x: f32, render_y: f32, min_x: i32, min_y: i32, max_x: i32, max_y: i32, world_scale: f32) {
        if self.main_menu || self.visited.is_empty() {
            return;
        }
        let ux0 = (min_x - self.min_x) * 8;
        let uy0 = (min_y - self.min_y) * 8;
        let ux1 = (max_x + 1 - self.min_x) * 8;
        let uy1 = (max_y + 1 - self.min_y) * 8;
        vbo.start_run(Prim::Quads);
        let s = 32.0 * world_scale;
        for y in uy0..uy1 {
            let mut x = ux0;
            while x < ux1 {
                let c = self.unit_color(x, y);
                let mut x2 = x + 1;
                while x2 < ux1 && self.unit_color(x2, y) == c {
                    x2 += 1;
                }
                if c[3] > 0.0 {
                    vbo.quad(render_x + x as f32 * s, render_y + y as f32 * s, render_x + x2 as f32 * s, render_y + (y + 1) as f32 * s, c);
                }
                x = x2;
            }
        }
        vbo.end_run();
    }

    /// `renderGrid(...)`: the unit grid over hidden units (and the area
    /// outside the bounds), every `mult` units by zoom.
    #[allow(clippy::too_many_arguments)]
    pub fn render_grid(&self, vbo: &mut Vbo, render_x: f32, render_y: f32, min_x: i32, min_y: i32, max_x: i32, max_y: i32, world_scale: f32, zoom_f: f32) {
        if zoom_f < 11.0 || self.visited.is_empty() {
            return;
        }
        let mult = if zoom_f < 13.0 {
            8
        } else if zoom_f < 14.0 {
            4
        } else if zoom_f < 15.0 {
            2
        } else {
            1
        };
        let g = self.grid_color;
        let hidden = |x: i32, y: i32| {
            let c = self.unit_color(x, y);
            if c[3] > 0.0 && c[0] == self.color[0] {
                c[3]
            } else {
                0.0
            }
        };
        let (bx0, by0) = (min_x.clamp(self.min_x, self.max_x), min_y.clamp(self.min_y, self.max_y));
        let (bx1, by1) = (max_x.clamp(self.min_x, self.max_x), max_y.clamp(self.min_y, self.max_y));
        let s = 32.0 * world_scale;
        vbo.start_run(Prim::Lines);
        vbo.set_line_width(0.5);
        let (ux0, uy0, ux1, uy1) = ((bx0 - self.min_x) * 8, (by0 - self.min_y) * 8, (bx1 + 1 - self.min_x) * 8, (by1 + 1 - self.min_y) * 8);
        let mut x = ux0;
        while x <= ux1 {
            for y in uy0..uy1 {
                let a = hidden(x - 1, y).max(hidden(x, y));
                if a > 0.0 {
                    vbo.line(render_x + x as f32 * s, render_y + y as f32 * s, render_x + x as f32 * s, render_y + (y + 1) as f32 * s, [g[0], g[1], g[2], g[3] * a]);
                }
            }
            x += mult;
        }
        let mut y = uy0;
        while y <= uy1 {
            for x in ux0..ux1 {
                let a = hidden(x, y - 1).max(hidden(x, y));
                if a > 0.0 {
                    vbo.line(render_x + x as f32 * s, render_y + y as f32 * s, render_x + (x + 1) as f32 * s, render_y + y as f32 * s, [g[0], g[1], g[2], g[3] * a]);
                }
            }
            y += mult;
        }
        vbo.end_run();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn meta() -> MetaBounds {
        MetaBounds { min_x: 0, min_y: 0, max_x: 9, max_y: 9 }
    }

    #[test]
    fn unloaded_grid_throws_after_creating() {
        let (v, err) = WorldMapVisited::new(MetaBounds::UNLOADED);
        assert_eq!(err, Some("java.lang.NegativeArraySizeException"));
        assert!(v.main_menu);
        assert!(!v.is_cell_visible(0, 0));
    }

    #[test]
    fn flags_update_and_save() {
        let (mut v, err) = WorldMapVisited::new(meta());
        assert!(err.is_none());
        assert_eq!(v.visited.len(), 20 * 80);
        assert!(!v.is_cell_visible(3, 3));
        v.update(&[Some((900.5, 900.5)), None, None, None]);
        assert!(v.is_visited(900, 900, 900, 900));
        assert!(v.is_cell_visible(3, 3));
        assert!(!v.is_known(2000, 2000, 2000, 2000));
        v.set_known_in_cells(7, 7, 7, 7);
        assert!(v.is_known(1900, 1900, 1900, 1900));
        let n = std::env::temp_dir().join(format!("pz-visited-{}-{}.bin", std::process::id(), line!()));
        v.save(&n).unwrap();
        let (mut w, _) = WorldMapVisited::new(meta());
        w.load(&n).unwrap();
        assert_eq!(v.visited, w.visited);
        w.clear_known_in_cells(7, 7, 7, 7);
        w.save_changes(&n).unwrap();
        let (mut z, _) = WorldMapVisited::new(meta());
        z.load(&n).unwrap();
        assert!(!z.is_known(1900, 1900, 1900, 1900));
        assert!(z.is_visited(900, 900, 900, 900));
        let _ = std::fs::remove_file(&n);
        v.forget();
        assert!(!v.is_cell_visible(3, 3));
    }

    /// The flag operations, change rectangle, cell visibility and the
    /// known/visited queries against the original (oracle `visited.txt`):
    /// a 10x9-cell grid at (3, 2).
    #[test]
    fn matches_visited_oracle() {
        let Some(dir) = std::env::var_os("PZ_WORLDMAP_ORACLE_DIR") else {
            eprintln!("PZ_WORLDMAP_ORACLE_DIR not set; skipping");
            return;
        };
        let want = std::fs::read_to_string(std::path::Path::new(&dir).join("visited.txt")).unwrap();
        let (mut v, err) = WorldMapVisited::new(MetaBounds { min_x: 3, min_y: 2, max_x: 12, max_y: 10 });
        assert!(err.is_none());
        let ops = [
            [0, 1000, 700, 1050, 750, 3],
            [0, 3000, 2400, 3100, 2600, 1],
            [0, 767, 511, 800, 600, 3],
            [1, 1010, 710, 1020, 720, 1],
            [0, 0, 0, 200, 200, 2],
            [0, 3200, 2700, 3400, 2900, 2],
            [2, 5, 4, 7, 6, 0],
            [3, 5, 4, 7, 6, 0],
            [4, 6, 5, 8, 7, 0],
            [1, 3300, 2600, 3330, 2690, 2],
            [0, 1999, 1599, 2001, 1601, 3],
            [0, 2048, 1536, 2049, 1537, 1],
        ];
        let mut out = Vec::new();
        for o in ops {
            match o[0] {
                0 => v.set_flags(o[1], o[2], o[3], o[4], o[5]),
                1 => v.clear_flags(o[1], o[2], o[3], o[4], o[5]),
                2 => v.set_known_in_cells(o[1], o[2], o[3], o[4]),
                3 => v.set_visited_in_cells(o[1], o[2], o[3], o[4]),
                _ => v.clear_known_in_cells(o[1], o[2], o[3], o[4]),
            }
            let mut h: u64 = 0xcbf29ce484222325;
            for &b in &v.visited {
                h = (h ^ b as u64).wrapping_mul(0x100000001b3);
            }
            let c = v.save_change;
            out.push(format!("op {} {:x} {} {} {} {}", o[0], h, c[0], c[1], c[2], c[3]));
        }
        let mut s = String::from("cells");
        for y in 0..14 {
            for x in 0..15 {
                s.push(if v.is_cell_visible(x, y) { '1' } else { '0' });
            }
        }
        out.push(s);
        let mut s = String::from("known");
        for y in (500..2800).step_by(37) {
            for x in (700..3400).step_by(41) {
                s.push(if v.is_known(x, y, x, y) { '1' } else { '0' });
                s.push(if v.is_visited(x, y, x, y) { '1' } else { '0' });
            }
        }
        out.push(s);
        assert_eq!(out, want.lines().collect::<Vec<_>>());
    }
}
