//! The window, sheet-rope and wall hooks of [`StateWorld`]: what
//! `OpenWindowState`, `CloseWindowState`, `SmashWindowState`,
//! `ClimbSheetRopeState`, `ClimbDownSheetRopeState` and
//! `ClimbOverWallState` read and change of the cell (`IsoWindow`'s
//! state, `setPermaLocked`, `WeaponHit` without a weapon,
//! `IsoBarricade.WeaponHit`, the sheet-rope squares,
//! `calculateClimbOutcome`, `IsoPlayer.canClimbOverWall` and
//! `isSafeToClimbOver`, and the wall's `FenceTypeHigh`).
//!
//! [`StateWorld`]: pz_characters::character::state::StateWorld

use pz_characters::character::state::{SheetRopeExit, SheetRopeSquare, WindowInfo};
use pz_pathfind::mover::ObjectRef;
use pz_world::java::fastfloor;
use pz_world::objects::{IsoObject, ObjectKind};
use pz_world::{IsoDirections, IsoGridSquare};

use crate::{StatesWorldAdapter, Thumper, WorldSoundRequest};

fn oref(sq: &IsoGridSquare, o: &IsoObject) -> ObjectRef {
    ObjectRef {
        x: sq.x,
        y: sq.y,
        z: sq.z,
        id: o.id,
    }
}

impl StatesWorldAdapter<'_> {
    /// An `IsoWindow`'s state.
    pub fn window_info_of(&self, r: ObjectRef) -> Option<WindowInfo> {
        let (sq, o) = self.find(r)?;
        let ObjectKind::Window(w) = &o.kind else {
            return None;
        };
        Some(WindowInfo {
            square: (sq.x, sq.y, sq.z),
            north: w.north,
            in_world: true,
            locked: w.locked,
            perma_locked: w.perma_locked,
            open: w.open,
            destroyed: w.destroyed,
            invincible: self.window_invincible(sq, o, w.north),
        })
    }

    /// `window.setPermaLocked(perma)` (`sync()` is the network's).
    pub fn set_window_perma_locked_at(&mut self, r: ObjectRef, perma: bool) {
        if let Some(ObjectKind::Window(w)) = self.find_mut(r).map(|o| &mut o.kind) {
            w.perma_locked = perma;
        }
    }

    /// `WorldSoundManager.instance.addSound(chr, x, y, z, 20, 20, false,
    /// 0.0f, 15.0f)` of a weapon hit.
    fn weapon_hit_sound(&mut self, chr: i32, r: ObjectRef) {
        self.engine.add_world_sound(WorldSoundRequest {
            source: Some(chr),
            x: r.x,
            y: r.y,
            z: r.z,
            radius: 20,
            volume: 20,
            stress_humans: false,
            zombie_ignore_dist: 0.0,
            stress_mod: 15.0,
        });
    }

    /// `IsoWindow.WeaponHit(owner, null)` off the client: the thumpable
    /// (a barricade takes the hit), the damage, the sound, and a player's
    /// hit breaks the pane.
    pub fn window_weapon_hit_by(&mut self, r: ObjectRef, t: &Thumper, player: bool) {
        let Some(target) = self.thumpable_for(r, Some(t)) else {
            return;
        };
        if target != r {
            self.barricade_weapon_hit(target, t);
            return;
        }
        self.engine.weapon_hit_thumpable(t.id, r);
        // weapon == null: `damage(100.0f, owner)`.
        self.window_damage(r, 100, t);
        self.weapon_hit_sound(t.id, r);
        let Some((sq, o)) = self.find(r) else { return };
        let ObjectKind::Window(w) = &o.kind else {
            return;
        };
        let invincible = self.window_invincible(sq, o, w.north);
        if player && !invincible {
            if let Some(ObjectKind::Window(w)) = self.find_mut(r).map(|o| &mut o.kind) {
                w.health = 0;
            }
        }
        let Some(ObjectKind::Window(w)) = self.find(r).map(|(_, o)| &o.kind) else {
            return;
        };
        if !w.destroyed && w.health <= 0 {
            self.smash_window(r, true);
            self.add_broken_glass(r, t);
        }
    }

    /// `IsoBarricade.WeaponHit(owner, null)` off the client.
    fn barricade_weapon_hit(&mut self, r: ObjectRef, t: &Thumper) {
        let Some(ObjectKind::Barricade(b)) = self.find(r).map(|(_, o)| &o.kind) else {
            return;
        };
        if b.is_destroyed() {
            return;
        }
        let metal = b.metal_health > 0 || b.metal_bar_health > 0;
        self.engine.weapon_hit_thumpable(t.id, r);
        let sound = if metal {
            "HitBarricadeMetal"
        } else {
            "HitBarricadePlank"
        };
        self.engine.play_world_sound(sound, (r.x, r.y, r.z));
        self.barricade_damage(r, 100.0);
        self.weapon_hit_sound(t.id, r);
        let destroyed = matches!(self.find(r).map(|(_, o)| &o.kind), Some(ObjectKind::Barricade(b)) if b.is_destroyed());
        if destroyed {
            let sound2 = if metal {
                "BreakBarricadeMetal"
            } else {
                "BreakBarricadePlank"
            };
            self.engine.play_sound(t.id, sound2, None);
            self.remove_object(r, false);
        }
        // Not destroyed: `setRenderEffect(Hit_Door, true)`, TODO(render
        // effects): it depends on `Core.optionDoDoorSpriteEffects`.
    }

    /// The sheet-rope view of a square.
    pub fn sheet_rope_square_at(&self, p: (i32, i32, i32)) -> Option<SheetRopeSquare> {
        let env = self.env();
        let sq = env.at(p)?;
        let has = |n: &str| env.sq_has(sq, n);
        Some(SheetRopeSquare {
            sheet_rope: env.is_sheet_rope_here(sq),
            can_climb_here: env.can_climb_here(sq),
            top: has("climbSheetTopN")
                || has("climbSheetTopS")
                || has("climbSheetTopW")
                || has("climbSheetTopE"),
            solid_floor: sq.treat_as_solid_floor(self.sprites),
            solidtrans: sq.has(self.sprites.tables.flag.solidtrans),
            n: has("climbSheetN") || has("climbSheetTopN"),
            s: has("climbSheetS") || has("climbSheetTopS"),
            w: has("climbSheetW") || has("climbSheetTopW"),
            e: has("climbSheetE") || has("climbSheetTopE"),
        })
    }

    /// `getChunk().getMinLevel()` / `getMaxLevel()` of a square's chunk.
    pub fn chunk_levels_at(&self, p: (i32, i32, i32)) -> (i32, i32) {
        self.cell
            .chunks
            .get(&(p.0.div_euclid(8), p.1.div_euclid(8)))
            .map_or((0, 0), |c| (c.min_level, c.max_level))
    }

    /// `ClimbSheetRopeState.calculateClimbOutcome(chr)` at the top square
    /// `target`, facing `dir`.
    pub fn sheet_rope_exit_for(
        &mut self,
        t: &Thumper,
        target: (i32, i32, i32),
        dir: IsoDirections,
    ) -> SheetRopeExit {
        let Some(sq) = self.at(target) else {
            return SheetRopeExit::Blocked;
        };
        // The original dereferences the square beyond.
        let Some(exit) = sq.get_adjacent_square(dir, self.cell) else {
            return SheetRopeExit::Blocked;
        };
        if !exit.treat_as_solid_floor(self.sprites) {
            return SheetRopeExit::Blocked;
        }
        let north = matches!(dir, IsoDirections::N | IsoDirections::S);
        let window = sq.get_window_to(Some(exit), self.cell).map(|o| {
            let in_sq = sq.objects.iter().any(|x| std::ptr::eq(x, o));
            (
                oref(if in_sq { sq } else { exit }, o),
                matches!(&o.kind, ObjectKind::Window(w) if w.open),
            )
        });
        if let Some((r, open)) = window {
            if !open {
                self.engine.toggle_window(self.cell, r, t.id);
            }
            let climb = self
                .find(r)
                .is_some_and(|(s, o)| self.window_can_climb_through(s, o, Some(t)));
            if !climb {
                return SheetRopeExit::Blocked;
            }
            return SheetRopeExit::OpenWindow(r);
        }
        let hooks = self.env().hooks();
        let found = sq
            .get_window_thumpable_to(Some(exit), self.sprites, self.cell, &hooks)
            .map(|(s, o)| (oref(s, o), self.thumpable_can_climb_through(s, o, Some(t))));
        if let Some((r, climb)) = found {
            return if climb {
                SheetRopeExit::OpenWindow(r)
            } else {
                SheetRopeExit::Blocked
            };
        }
        let fence = |a: &Self| a.can_climb_through_helper(t, sq, Some(exit), north);
        if sq
            .get_hoppable_thumpable_to(Some(exit), self.cell, &hooks)
            .is_some()
        {
            return if fence(self) {
                SheetRopeExit::Fence
            } else {
                SheetRopeExit::Blocked
            };
        }
        let frame = sq
            .get_window_frame_to(Some(exit), self.cell)
            .and_then(|(s, o)| match &o.kind {
                ObjectKind::WindowFrame { north } => {
                    Some((oref(s, o), self.frame_can_climb_through(s, *north, Some(t))))
                }
                _ => None,
            });
        if let Some((r, climb)) = frame {
            return if climb {
                SheetRopeExit::WindowFrame(r)
            } else {
                SheetRopeExit::Blocked
            };
        }
        if sq
            .get_wall_hoppable_to(Some(exit), self.sprites, self.cell)
            .is_some()
        {
            return if fence(self) {
                SheetRopeExit::Fence
            } else {
                SheetRopeExit::Blocked
            };
        }
        SheetRopeExit::Undefined
    }

    /// `HasSlopedRoof() && !HasEave()`.
    fn sloped_roof_without_eave(&self, sq: Option<&IsoGridSquare>) -> bool {
        let Some(sq) = sq else { return false };
        let ty = |n: &str| self.sprites.tables.object_type_from_string(n);
        let sloped = ["WestRoofB", "WestRoofM", "WestRoofT"]
            .into_iter()
            .any(|n| sq.has_type(ty(n)));
        sloped && !self.env().sq_has(sq, "isEave")
    }

    /// `IsoPlayer.canClimbOverWall(dir)` without the sprint test.
    pub fn can_climb_over_wall_for(&self, t: &Thumper, dir: IsoDirections) -> bool {
        let at = (fastfloor(t.x), fastfloor(t.y), fastfloor(t.z));
        if !self.is_safe_to_climb_over(at, dir) {
            return false;
        }
        let Some(cur) = t.current_square.and_then(|p| self.at(p)) else {
            return false;
        };
        if cur.have_roof || self.square_building_idx(cur).is_some() {
            return false;
        }
        let above = self.cell.get_grid_square(cur.x, cur.y, cur.z + 1);
        if self.sloped_roof_without_eave(above) {
            return false;
        }
        // The original dereferences the square beyond.
        let Some(sq) = cur.get_adjacent_square(dir, self.cell) else {
            return false;
        };
        if sq.have_roof
            || sq.is_solid(self.sprites)
            || sq.is_solid_trans(self.sprites)
            || self.square_building_idx(sq).is_some()
        {
            return false;
        }
        let above2 = self.cell.get_grid_square(sq.x, sq.y, sq.z + 1);
        if self.sloped_roof_without_eave(above2) {
            return false;
        }
        let env = self.env();
        let wall = self.sprites.tables.ty.wall;
        let (s, up, collide, hoppable) = match dir {
            IsoDirections::N => (cur, above, "collideN", "HoppableN"),
            IsoDirections::S => (sq, above2, "collideN", "HoppableN"),
            IsoDirections::W => (cur, above, "collideW", "HoppableW"),
            IsoDirections::E => (sq, above2, "collideW", "HoppableW"),
            _ => return false,
        };
        if env.sq_has(s, "CantClimb")
            || !s.has_type(wall)
            || !env.sq_has(s, collide)
            || env.sq_has(s, hoppable)
        {
            return false;
        }
        if up.is_some_and(|u| env.sq_has(u, collide)) {
            return false;
        }
        self.can_climb_through_helper(
            t,
            cur,
            Some(sq),
            matches!(dir, IsoDirections::N | IsoDirections::S),
        )
    }

    /// `ClimbOverWallState.getClimbableWallN` / `getClimbableWallW`.
    fn climbable_wall<'s>(&'s self, sq: &'s IsoGridSquare, north: bool) -> Option<&'s IsoObject> {
        let env = self.env();
        let wall = self.sprites.tables.ty.wall;
        let (collide, hoppable) = if north {
            ("collideN", "HoppableN")
        } else {
            ("collideW", "HoppableW")
        };
        sq.objects.iter().find(|o| {
            let Some(s) = o.sprite else { return false };
            self.sprites.sprites[s].tile_type == wall
                && !env.obj_has_flag(o, "CantClimb")
                && env.obj_has_flag(o, collide)
                && !env.obj_has_flag(o, hoppable)
        })
    }

    /// `getFenceType(getFence(owner))` of a wall climb.
    pub fn climb_wall_fence_type_at(
        &self,
        start: (i32, i32, i32),
        end: (i32, i32),
        dir: IsoDirections,
    ) -> Option<i32> {
        let s = self.cell.get_grid_square(start.0, start.1, start.2)?;
        let e = self.cell.get_grid_square(end.0, end.1, start.2)?;
        let fence = match dir {
            IsoDirections::N => self.climbable_wall(s, true),
            IsoDirections::E => self.climbable_wall(e, false),
            IsoDirections::W => self.climbable_wall(s, false),
            IsoDirections::S => self.climbable_wall(e, true),
            _ => None,
        }?;
        Some(match self.env().obj_prop(fence, "FenceTypeHigh") {
            Some("Metal") => 1,
            Some("MetalGate") => 2,
            _ => 0,
        })
    }
}
