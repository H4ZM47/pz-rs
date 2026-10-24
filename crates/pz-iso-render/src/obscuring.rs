//! `FBORenderCell.checkObjectsObscuringPlayer`: solid objects between the
//! camera and the player (furniture, stairs, wall-attached items) are drawn
//! translucent while they hide the player, and keep fading back in on
//! later frames until their alpha returns to 1.

use pz_render::iso::{x_to_screen, y_to_screen, TILE_SCALE};

use crate::cutaways::CutawayWorld;
use crate::world::{ObjectKind, RenderObject, RenderSquare, StairPart};

type Loc = (i32, i32, i32);

/// The player's screen bounds (`calculatePlayerRenderBounds`).
fn player_bounds(px: f32, py: f32, pz: f32) -> [f32; 4] {
    let x = x_to_screen(px, py) - (32 * TILE_SCALE) as f32;
    let y = y_to_screen(px, py, pz, 0) - (112 * TILE_SCALE) as f32;
    [x, y, (64 * TILE_SCALE) as f32, (128 * TILE_SCALE) as f32]
}

/// `isPotentiallyObscuringObject` (sitting and beds not modelled).
pub fn is_potentially_obscuring(sq: &RenderSquare, obj: &RenderObject, player: Loc, player_on_stairs: bool) -> bool {
    if obj.kind == ObjectKind::WorldInventory {
        return false;
    }
    let Some(s) = obj.sprite.as_ref() else { return false };
    let f = &s.flags;
    if f.water {
        return false;
    }
    let square_solid = sq.objects.iter().any(|o| o.sprite.as_ref().is_some_and(|s| s.solid || s.solid_trans));
    if f.attached_surface && square_solid {
        return true;
    }
    if f.attached_e || f.attached_s || f.attached_ceiling {
        return true;
    }
    if obj.container_count > 0 {
        return true;
    }
    if let Some(st) = s.stairs {
        if player_on_stairs {
            return false;
        }
        return if st.north() { sq.x > player.0 } else { sq.y > player.1 };
    }
    s.solid || s.solid_trans
}

/// The obscuring state carried across frames (`PerPlayerData`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Obscuring {
    /// `squaresObscuringPlayer`.
    pub squares: Vec<Loc>,
    /// `fadingInSquares`.
    pub fading_in: Vec<Loc>,
}

impl Obscuring {
    pub fn is_obscuring(&self, x: i32, y: i32, z: i32) -> bool {
        self.squares.contains(&(x, y, z))
    }

    pub fn is_fading_in(&self, x: i32, y: i32, z: i32) -> bool {
        self.fading_in.contains(&(x, y, z))
    }

    /// `checkObjectsObscuringPlayer` and `checkFadingInObjectsObscuringPlayer`
    /// for a player at `(px, py, pz)`. Returns the squares whose chunk level
    /// must be re-recorded (`DIRTY_OBSCURING`). Sprite-grid objects are not
    /// expanded to their other squares.
    pub fn update(&mut self, world: &dyn CutawayWorld, px: f32, py: f32, pz: f32) -> Vec<Loc> {
        let mut dirty = Vec::new();
        let player = (pz_render::iso::fastfloor(px), pz_render::iso::fastfloor(py), pz_render::iso::fastfloor(pz));
        let on_stairs = world.square(player.0, player.1, player.2).is_some_and(|s| s.objects.iter().any(|o| o.sprite.as_ref().is_some_and(|s| s.stairs.is_some())));
        // checkFadingInObjectsObscuringPlayer.
        self.fading_in.retain(|&(x, y, z)| {
            let keep = world.square(x, y, z).is_some_and(|sq| has_fading_in(sq, player, on_stairs));
            if !keep {
                dirty.push((x, y, z));
            }
            keep
        });
        let bounds = player_bounds(px, py, pz);
        let mut locs: Vec<Loc> = Vec::new();
        let mut test = |x: i32, y: i32| {
            if let Some(sq) = world.square(x, y, player.2) {
                if is_square_obscuring(sq, bounds, player, on_stairs) && !locs.contains(&(x, y, player.2)) {
                    locs.push((x, y, player.2));
                }
            }
        };
        let (sx, sy) = (player.0, player.1);
        test(sx, sy);
        for i in 1..=3 {
            test(sx + i, sy + i);
            test(sx - 1 + i, sy + i);
            test(sx + i, sy - 1 + i);
            test(sx - 1 + i, sy + 1 + i);
            test(sx + 1 + i, sy - 1 + i);
        }
        // addObscuringStairObjects.
        let mut i = 0;
        while i < locs.len() {
            let (x, y, z) = locs[i];
            if let Some(sq) = world.square(x, y, z) {
                for o in &sq.objects {
                    if !is_potentially_obscuring(sq, o, player, on_stairs) {
                        continue;
                    }
                    let Some(st) = o.sprite.as_ref().and_then(|s| s.stairs) else { continue };
                    let (d1, d2) = match st {
                        StairPart::TopN | StairPart::TopW => (0, 2),
                        StairPart::MiddleN | StairPart::MiddleW => (-1, 1),
                        StairPart::BottomN | StairPart::BottomW => (-2, 0),
                    };
                    for d in d1..=d2 {
                        let l = if st.north() { (x, y + d, z) } else { (x + d, y, z) };
                        if world.square(l.0, l.1, l.2).is_some() && !locs.contains(&l) {
                            locs.push(l);
                        }
                    }
                }
            }
            i += 1;
        }
        for &l in &self.squares {
            if !locs.contains(&l) && !self.fading_in.contains(&l) && world.square(l.0, l.1, l.2).is_some_and(|sq| has_fading_in(sq, player, on_stairs)) {
                self.fading_in.push(l);
            }
        }
        if locs != self.squares {
            for l in self.squares.iter().chain(locs.iter()) {
                if !dirty.contains(l) {
                    dirty.push(*l);
                }
            }
            self.squares = locs;
        }
        dirty
    }
}

/// `squareHasFadingInObjects`.
fn has_fading_in(sq: &RenderSquare, player: Loc, on_stairs: bool) -> bool {
    sq.objects.iter().any(|o| is_potentially_obscuring(sq, o, player, on_stairs) && o.alpha < 1.0)
}

/// `isSquareObscuringPlayer` with `isObjectObscuringPlayer`.
fn is_square_obscuring(sq: &RenderSquare, b: [f32; 4], player: Loc, on_stairs: bool) -> bool {
    sq.objects.iter().any(|o| {
        if !is_potentially_obscuring(sq, o, player, on_stairs) {
            return false;
        }
        let Some(tex) = o.sprite.as_ref().and_then(|s| s.texture.as_ref()) else { return false };
        let (x, y) = (sq.x as f32, sq.y as f32);
        let tx = x_to_screen(x, y) - o.offset_x + tex.offset_x;
        let ty = y_to_screen(x, y, sq.z as f32, 0) - (o.offset_y + o.render_y_offset * TILE_SCALE as f32) + tex.offset_y;
        tx < b[0] + b[2] && tx + tex.width as f32 > b[0] && ty < b[1] + b[3] && ty + tex.height as f32 > b[1]
    })
}

#[cfg(test)]
mod tests {
    use std::rc::Rc;

    use super::*;
    use crate::cutaways::GridWorld;
    use crate::world::SpriteInfo;

    fn solid_object() -> RenderObject {
        let mut tex = pz_render::Texture::empty("fridge");
        tex.width = 128;
        tex.height = 256;
        tex.ready = true;
        let s = SpriteInfo { texture: Some(tex), solid: true, tint_mod: [1.0; 3], ..Default::default() };
        RenderObject::new(ObjectKind::Plain, Some(Rc::new(s)))
    }

    #[test]
    fn solid_object_in_front_hides_the_player_then_fades_in() {
        let mut w = GridWorld::default();
        w.insert(RenderSquare::new(10, 10, 0));
        let mut front = RenderSquare::new(11, 11, 0);
        front.objects.push(solid_object());
        w.insert(front);
        let mut o = Obscuring::default();
        let dirty = o.update(&w, 10.5, 10.5, 0.0);
        assert_eq!(o.squares, vec![(11, 11, 0)]);
        assert_eq!(dirty, vec![(11, 11, 0)]);
        // The player walks away while the object is still at alpha 0.66:
        // the square fades in until the object is opaque again.
        w.square_mut(11, 11, 0).objects[0].alpha = 0.66;
        o.update(&w, 2.5, 2.5, 0.0);
        assert!(o.squares.is_empty());
        assert_eq!(o.fading_in, vec![(11, 11, 0)]);
        w.square_mut(11, 11, 0).objects[0].alpha = 1.0;
        let dirty = o.update(&w, 2.5, 2.5, 0.0);
        assert!(o.fading_in.is_empty());
        assert_eq!(dirty, vec![(11, 11, 0)]);
    }
}
