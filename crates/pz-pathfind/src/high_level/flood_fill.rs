//! `FloodFill`: grows an `HLChunkRegion` from a seed square with a scanline
//! fill limited to the chunk, using `canMoveBetween` for a plain player
//! mover.

use super::hl_chunk_level::{can_walk_on_square, is_can_path_transition};
use super::RegionId;
use crate::request::MoverKind;
use crate::square::bits::*;
use crate::world::{MoverSel, SquareId, World};

struct Fill {
    region: RegionId,
    squares: [[Option<SquareId>; 8]; 8],
    min_x: i32,
    min_y: i32,
    level: i32,
    thumpable: bool,
    /// `visited[y]` bit `x`.
    visited: [u8; 8],
}

impl Fill {
    fn is_visited(&self, gx: i32, gy: i32) -> bool {
        self.visited[gy as usize] & (1 << gx) != 0
    }

    fn set_visited(&mut self, gx: i32, gy: i32) {
        if (0..8).contains(&gx) && (0..8).contains(&gy) {
            self.visited[gy as usize] |= 1 << gx;
        }
    }

    fn square(&self, x: i32, y: i32) -> Option<SquareId> {
        let (gx, gy) = (x - self.min_x, y - self.min_y);
        assert!(
            (0..8).contains(&gx) && (0..8).contains(&gy),
            "invalid ChunkSquares x,y"
        );
        self.squares[gx as usize][gy as usize]
    }
}

impl World {
    /// `FloodFill::shouldVisit`.
    fn ff_should_visit(&self, f: &Fill, x1: i32, y1: i32, x2: i32, y2: i32) -> bool {
        let gx = x2 - f.min_x;
        let gy = y2 - f.min_y;
        if !(0..8).contains(&gx) || !(0..8).contains(&gy) || f.is_visited(gx, gy) {
            return false;
        }
        let sq2 = f.squares[gx as usize][gy as usize];
        if f.thumpable {
            match sq2 {
                Some(s) if self.sq(s).has(THUMPABLE) && self.sq(s).treat_as_solid_floor() => {}
                _ => return false,
            }
        } else {
            if !can_walk_on_square(self, sq2) {
                return false;
            }
            let sq1 = f.square(x1, y1);
            if is_can_path_transition(self, sq1, sq2) {
                return false;
            }
        }
        self.can_move_between_with(MoverSel::FloodFill, x1, y1, f.level, x2, y2, f.level)
    }

    /// `FloodFill::initVisibilityGraphs`: squares under vehicle clusters
    /// (other than stairs and slopes) are pre-marked as visited.
    fn ff_init_visibility_graphs(&self, f: &mut Fill) {
        let chunk = self.region(f.region).level.chunk;
        let mut graphs = Vec::new();
        self.get_visibility_graphs_overlapping_chunk(chunk, f.level, &mut graphs);
        for g in graphs {
            let cluster = self.vis.graphs_pool[g].cluster;
            for rect in &self.vis.clusters_pool[cluster].rects {
                for y in rect.y - 1..rect.y + rect.h + 1 {
                    for x in rect.x - 1..rect.x + rect.w + 1 {
                        if !self.chunk(chunk).contains(x, y) {
                            continue;
                        }
                        if let Some(s) = f.square(x, y) {
                            let sq = self.sq(s);
                            if sq.has(ALL_STAIRS) || sq.has_sloped_surface() {
                                continue;
                            }
                        }
                        f.set_visited(x - f.min_x, y - f.min_y);
                    }
                }
            }
        }
    }

    /// `HLChunkLevel::floodFill` / `FloodFill::calculate`.
    pub(crate) fn flood_fill(&mut self, region: RegionId, seed: SquareId) {
        let level_ref = self.region(region).level;
        let squares = self.chunk_level(level_ref).squares;
        let (min_x, min_y) = (
            self.chunk(level_ref.chunk).min_x(),
            self.chunk(level_ref.chunk).min_y(),
        );
        let level = level_ref.z;
        self.flood_mover.kind = MoverKind::Player;
        self.flood_mover.min_level = level;
        self.flood_mover.max_level = level;
        let mut f = Fill {
            region,
            squares,
            min_x,
            min_y,
            level,
            thumpable: self.sq(seed).has(THUMPABLE),
            visited: [0; 8],
        };
        self.ff_init_visibility_graphs(&mut f);

        let mut stack: Vec<Option<SquareId>> = Vec::new();
        let mut choices: Vec<SquareId> = Vec::new();
        let seed_sq = self.sq(seed);
        stack.push(f.square(seed_sq.x, seed_sq.y));
        while let Some(top) = stack.pop() {
            let sq = match top {
                Some(s) => s,
                None => break,
            };
            let x = self.sq(sq).x;
            let mut y = self.sq(sq).y;
            while self.ff_should_visit(&f, x, y, x, y - 1) {
                y -= 1;
            }
            let mut span_left = false;
            let mut span_right = false;
            loop {
                f.set_visited(x - f.min_x, y - f.min_y);
                if let Some(s2) = f.square(x, y) {
                    choices.push(s2);
                }
                let left = self.ff_should_visit(&f, x, y, x - 1, y);
                if span_left {
                    if !left {
                        span_left = false;
                    } else if !self.ff_should_visit(&f, x - 1, y, x - 1, y - 1) {
                        stack.push(f.square(x - 1, y));
                    }
                } else if left {
                    stack.push(f.square(x - 1, y));
                    span_left = true;
                }
                let right = self.ff_should_visit(&f, x, y, x + 1, y);
                if span_right {
                    if !right {
                        span_right = false;
                    } else if !self.ff_should_visit(&f, x + 1, y, x + 1, y - 1) {
                        stack.push(f.square(x + 1, y));
                    }
                } else if right {
                    stack.push(f.square(x + 1, y));
                    span_right = true;
                }
                if !self.ff_should_visit(&f, x, y, x, y + 1) {
                    break;
                }
                y += 1;
            }
        }

        let (min_x, min_y) = (f.min_x, f.min_y);
        let mut reg = self.hl.regions[region].clone();
        reg.min_x = i32::MAX;
        reg.min_y = i32::MAX;
        reg.max_x = i32::MIN;
        reg.max_y = i32::MIN;
        for s in choices {
            let sq = self.sq(s);
            reg.set_local(sq.x - min_x, sq.y - min_y);
            reg.min_x = reg.min_x.min(sq.x);
            reg.min_y = reg.min_y.min(sq.y);
            reg.max_x = reg.max_x.max(sq.x);
            reg.max_y = reg.max_y.max(sq.y);
        }
        self.hl.regions[region] = reg;
    }
}
