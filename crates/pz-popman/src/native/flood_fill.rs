//! `FloodFill<N>`: the library's fixed-size scanline flood fill over an
//! N x N window, recording visited squares in fill order.

/// One `FloodFill<N>` instance (the library keeps a few as statics; their
/// buffers persist between uses, which matters only for the visited list
/// beyond `visited_count`).
#[derive(Clone)]
pub struct FloodFill {
    pub size: i32,
    pub origin: (i32, i32),
    words_per_row: i32,
    bits: Vec<u32>,
    stack: Vec<(i16, i16)>,
    pub visited: Vec<(i16, i16)>,
}

impl FloodFill {
    pub fn new(size: i32) -> FloodFill {
        let words_per_row = (size + 31) / 32;
        FloodFill {
            size,
            origin: (0, 0),
            words_per_row,
            bits: vec![0; (words_per_row * size) as usize],
            stack: Vec::new(),
            visited: Vec::new(),
        }
    }

    fn cap(&self) -> usize {
        (self.size * self.size) as usize
    }

    /// The visited bit for absolute (x, y), with the library's 16-bit
    /// offset arithmetic; `None` when outside the bitmap.
    fn bit_index(&self, x: i32, y: i32) -> Option<(usize, u32)> {
        let dx = x.wrapping_sub(self.origin.0);
        if (dx >> 15) & 1 != 0 {
            return None;
        }
        let sdx = dx as i16 as i32;
        if sdx >= self.size {
            return None;
        }
        let dy = (y as i16).wrapping_sub(self.origin.1 as i16) as i32;
        if dy < 0 || dy >= self.size {
            return None;
        }
        let word = (sdx as u32) >> 5;
        if word as i32 >= self.words_per_row {
            return None;
        }
        Some(((word as i32 + self.words_per_row * dy) as usize, (dx & 31) as u32))
    }

    /// Whether (x, y) was visited.
    pub fn is_visited(&self, x: i32, y: i32) -> bool {
        match self.bit_index(x, y) {
            Some((w, b)) => (self.bits[w] >> b) & 1 != 0,
            None => false,
        }
    }

    /// Marks (x, y) visited.
    pub fn set_visited(&mut self, x: i32, y: i32) {
        if let Some((w, b)) = self.bit_index(x, y) {
            self.bits[w] |= 1 << b;
        }
    }

    /// The base `shouldVisit` test: inside the window and not visited.
    pub fn in_window_unvisited(&self, x: i32, y: i32) -> bool {
        let (ox, oy) = self.origin;
        if x < ox || self.size + ox <= x {
            return false;
        }
        if y < oy || oy + self.size <= y {
            return false;
        }
        !self.is_visited(x, y)
    }

    /// `calculate(sx, sy, ox, oy)`. `init` is `visitInit`, `visit` is
    /// `shouldVisit(fromX, fromY, toX, toY)`.
    #[allow(clippy::too_many_arguments)]
    pub fn calculate<C>(
        &mut self,
        ctx: &mut C,
        sx: i32,
        sy: i32,
        ox: i32,
        oy: i32,
        init: impl FnOnce(&mut FloodFill, &mut C),
        mut visit: impl FnMut(&FloodFill, &mut C, i32, i32, i32, i32) -> bool,
    ) {
        self.bits.fill(0);
        self.origin = (ox, oy);
        self.stack.clear();
        self.visited.clear();
        init(self, ctx);
        let cap = self.cap();
        if self.stack.len() >= cap {
            return;
        }
        self.stack.push(((sx as i16).wrapping_sub(ox as i16), (sy as i16).wrapping_sub(oy as i16)));
        while let Some((px, py)) = self.stack.pop() {
            if px == -1 {
                break;
            }
            let x = ox.wrapping_add(px as i32);
            let mut y = oy.wrapping_add(py as i32);
            while visit(self, ctx, x, y, x, y - 1) {
                y -= 1;
            }
            let mut span_left = false;
            let mut span_right = false;
            loop {
                if self.visited.len() >= cap {
                    return;
                }
                self.visited.push(((x.wrapping_sub(ox)) as i16, (y as i16).wrapping_sub(oy as i16)));
                self.set_visited(x, y);
                let l = visit(self, ctx, x, y, x - 1, y);
                if span_left {
                    if !l {
                        span_left = false;
                    } else if !visit(self, ctx, x - 1, y, x - 1, y - 1) {
                        if self.stack.len() >= cap {
                            return;
                        }
                        self.stack.push(((x - 1).wrapping_sub(ox) as i16, (y as i16).wrapping_sub(oy as i16)));
                    }
                } else if l {
                    if self.stack.len() >= cap {
                        return;
                    }
                    self.stack.push(((x - 1).wrapping_sub(ox) as i16, (y as i16).wrapping_sub(oy as i16)));
                    span_left = true;
                }
                let r = visit(self, ctx, x, y, x + 1, y);
                if span_right {
                    if !r {
                        span_right = false;
                    } else if !visit(self, ctx, x + 1, y, x + 1, y - 1) {
                        if self.stack.len() >= cap {
                            return;
                        }
                        self.stack.push(((x + 1).wrapping_sub(ox) as i16, (y as i16).wrapping_sub(oy as i16)));
                    }
                } else if r {
                    if self.stack.len() >= cap {
                        return;
                    }
                    self.stack.push(((x + 1).wrapping_sub(ox) as i16, (y as i16).wrapping_sub(oy as i16)));
                    span_right = true;
                }
                if !visit(self, ctx, x, y, x, y + 1) {
                    break;
                }
                y += 1;
            }
        }
    }
}
