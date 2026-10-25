//! The climb and movement hooks of [`StateWorld`]: what
//! `ClimbThroughWindowState` and `ClimbOverFenceState` read and change of
//! windows and fences, the stairs, slopes and walls of
//! `IsoMovingObject.postupdate` (`doStairs`, `handleSlopedSurface`,
//! `checkHitWall` with `checkVaultOver`).
//!
//! [`StateWorld`]: pz_characters::character::state::StateWorld

use pz_characters::character::state::{
    ClimbStatus, FenceClimb, FenceInfo, RopeClimbOutcome, RopeSquare, SquareInfo, StairsPieces, ThumpKind, WindowClimb,
};
use pz_pathfind::mover::ObjectRef;
use pz_world::objects::{IsoObject, ObjectKind, ObjectName};
use pz_world::{IsoDirections, IsoGridSquare};

use crate::{StatesWorldAdapter, Thumper};

fn oref(sq: &IsoGridSquare, o: &IsoObject) -> ObjectRef {
    ObjectRef {
        x: sq.x,
        y: sq.y,
        z: sq.z,
        id: o.id,
    }
}

impl StatesWorldAdapter<'_> {
    /// What the states read about a square.
    pub fn square_info(&self, sq: &IsoGridSquare) -> SquareInfo {
        let m = &*self.sprites;
        let ty = &m.tables.ty;
        SquareInfo {
            x: sq.x,
            y: sq.y,
            z: sq.z,
            in_room: sq.room_id != -1,
            solid_floor: sq.treat_as_solid_floor(m),
            has_stairs: sq.has_stairs(m),
            has_stairs_north: sq.has_stairs_north(m),
            has_stairs_west: sq.has_stairs_west(m),
            stairs: StairsPieces {
                tn: sq.has_type(ty.stairsTN),
                tw: sq.has_type(ty.stairsTW),
                mn: sq.has_type(ty.stairsMN),
                mw: sq.has_type(ty.stairsMW),
                bn: sq.has_type(ty.stairsBN),
                bw: sq.has_type(ty.stairsBW),
            },
            wall_n: self.sq_flag(sq, "WallN"),
            wall_w: self.sq_flag(sq, "WallW"),
            sloped_surface: sq.has_sloped_surface(),
            sloped_surface_height_max: sq.get_sloped_surface_height_max(),
        }
    }

    /// `square.has(flag)` for a flag the tables may lack.
    fn sq_flag(&self, sq: &IsoGridSquare, name: &str) -> bool {
        self.sprites.tables.flag_from_string(name).is_some_and(|f| sq.has(f))
    }

    /// `square.getWallType()`.
    pub fn wall_type_of(&self, sq: &IsoGridSquare) -> i32 {
        let mut t = 0;
        if self.sq_flag(sq, "WallN") {
            t |= 1;
        }
        if self.sq_flag(sq, "WallW") {
            t |= 4;
        }
        if self.sq_flag(sq, "WallNW") {
            t |= 5;
        }
        if let Some(e) = sq.get_adjacent_square(IsoDirections::E, self.cell) {
            if self.sq_flag(e, "WallW") || self.sq_flag(e, "WallNW") {
                t |= 8;
            }
        }
        if let Some(s) = sq.get_adjacent_square(IsoDirections::S, self.cell) {
            if self.sq_flag(s, "WallN") || self.sq_flag(s, "WallNW") {
                t |= 2;
            }
        }
        t
    }

    /// What `ClimbOverFenceState.setParams` reads of the square beyond a
    /// fence.
    pub fn fence_opposite(&self, opp: Option<&IsoGridSquare>) -> FenceClimb {
        let m = &*self.sprites;
        FenceClimb {
            counter: opp.is_some_and(|o| o.has(m.tables.flag.solidtrans)),
            solid_floor: opp.is_some_and(|o| o.treat_as_solid_floor(m)),
            sheet_rope: opp.is_some_and(|o| self.env().can_climb_down_sheet_rope(Some(o))),
        }
    }

    /// `IsoPlayer.isSafeToClimbOver(dir)` from the square `at`.
    fn is_safe_to_climb_over(&self, at: (i32, i32, i32), dir: IsoDirections) -> bool {
        let (dx, dy) = match dir {
            IsoDirections::N => (0, -1),
            IsoDirections::S => (0, 1),
            IsoDirections::W => (-1, 0),
            IsoDirections::E => (1, 0),
            _ => return false,
        };
        let Some(sq) = self.cell.get_grid_square(at.0 + dx, at.1 + dy, at.2) else { return false };
        let m = &*self.sprites;
        if sq.has(m.tables.flag.water) {
            return false;
        }
        if !sq.treat_as_solid_floor(m) {
            return self.env().has_stairs_below(sq);
        }
        true
    }

    /// `checkVaultOver()`'s squares: the vault direction of a player on
    /// `current` (standing on `at`) facing `dir`, and the square beyond.
    pub fn vault_over_for(
        &self,
        current: (i32, i32, i32),
        at: (i32, i32, i32),
        dir: IsoDirections,
        (cn, cs, cw, ce): (bool, bool, bool, bool),
    ) -> Option<(IsoDirections, FenceClimb)> {
        use IsoDirections as D;
        let cur = self.cell.get_grid_square(current.0, current.1, current.2)?;
        let f = &self.sprites.tables.flag;
        if dir == D::SE {
            if let Some(se) = cur.get_adjacent_square(D::SE, self.cell) {
                if se.has(f.HoppableN) && se.has(f.HoppableW) {
                    return None;
                }
            }
        }
        let feeler = if cs {
            cur.get_adjacent_square(D::S, self.cell)
        } else if ce {
            cur.get_adjacent_square(D::E, self.cell)
        } else {
            Some(cur)
        }?;
        let mut dir = dir;
        let mut vault = false;
        if cur.has(f.HoppableN) && cn && !cw && !ce && matches!(dir, D::NW | D::N | D::NE) {
            dir = D::N;
            vault = true;
        }
        if feeler.has(f.HoppableN) && cs && !cw && !ce && matches!(dir, D::SW | D::S | D::SE) {
            dir = D::S;
            vault = true;
        }
        if cur.has(f.HoppableW) && cw && !cn && !cs && matches!(dir, D::NW | D::W | D::SW) {
            dir = D::W;
            vault = true;
        }
        if feeler.has(f.HoppableW) && ce && !cn && !cs && matches!(dir, D::NE | D::E | D::SE) {
            dir = D::E;
            vault = true;
        }
        if !vault || !self.is_player_able_to_hop_wall_to(cur, dir, feeler) || !self.is_safe_to_climb_over(at, dir) {
            return None;
        }
        Some((dir, self.fence_opposite(cur.get_adjacent_square(dir, self.cell))))
    }

    // ---- windows ----

    /// `isFreeSquare(square)`.
    fn is_free_square(&self, sq: Option<&IsoGridSquare>) -> bool {
        let m = &*self.sprites;
        let f = &m.tables.flag;
        sq.is_some_and(|s| s.treat_as_solid_floor(m) && !s.has(f.solid) && !s.has(f.solidtrans))
    }

    /// `isObstacleSquare(square)`.
    fn is_obstacle_square(&self, sq: Option<&IsoGridSquare>) -> bool {
        let m = &*self.sprites;
        let f = &m.tables.flag;
        sq.is_some_and(|s| s.treat_as_solid_floor(m) && !s.has(f.solid) && s.has(f.solidtrans) && !s.has(f.water))
    }

    /// Nothing between `a` and `b` stops a counter climb: no wall, window or
    /// door (`isSomethingTo`), window frame or window thumpable.
    fn open_between(&self, a: &IsoGridSquare, b: &IsoGridSquare) -> bool {
        let hooks = self.env().hooks();
        !self.env().is_something_to(a, b)
            && a.get_window_frame_to(Some(b), self.cell).is_none()
            && a.get_window_thumpable_to(Some(b), self.sprites, self.cell, &hooks).is_none()
    }

    /// `getFreeSquareAfterObstacles(square, dir)`.
    fn free_square_after_obstacles<'s>(&'s self, mut sq: &'s IsoGridSquare, dir: IsoDirections) -> Option<&'s IsoGridSquare> {
        loop {
            let next = sq.get_adjacent_square(dir, self.cell)?;
            if !self.open_between(sq, next) {
                return None;
            }
            if self.is_free_square(Some(next)) {
                return Some(next);
            }
            if !self.is_obstacle_square(Some(next)) {
                return None;
            }
            sq = next;
        }
    }

    /// `IsoObject.getName()`.
    fn object_name(&self, o: &IsoObject) -> Option<String> {
        match &o.name {
            None => None,
            Some(ObjectName::Grass) => Some("Grass".into()),
            Some(ObjectName::Str(s)) => Some(s.clone()),
            Some(ObjectName::Dict(i)) => self.engine.dict_object_name(*i),
        }
    }

    /// `getClimbThroughWindowPositioningParams(chr, obj, params)`'s world
    /// part.
    pub fn window_climb_for(&self, t: &Thumper, r: ObjectRef) -> Option<WindowClimb> {
        let (sq, o) = self.find(r)?;
        let (portal, can_climb, portal_can_climb, north, broken_glass, barbed) = match &o.kind {
            ObjectKind::Window(w) => {
                let c = self.window_can_climb_through(sq, o, Some(t));
                (ThumpKind::Window, c, c, w.north, w.destroyed && !w.glass_removed, false)
            }
            ObjectKind::Thumpable(th) => {
                let c = self.thumpable_can_climb_through(sq, o, Some(t));
                let barbed = self.object_name(o).as_deref() == Some("Barbed Fence");
                (ThumpKind::Thumpable, c, c, th.north, false, barbed)
            }
            ObjectKind::WindowFrame { north } => {
                let c = self.frame_can_climb_through(sq, *north, Some(t));
                (ThumpKind::WindowFrame, true, c, *north, false, false)
            }
            _ => return None,
        };
        let (x, y, z) = (sq.x, sq.y, sq.z);
        let (mut start, mut opp) = ((x, y), (x, y));
        let dir = if north {
            if (y as f32) < t.y {
                opp.1 -= 1;
                IsoDirections::N
            } else {
                start.1 -= 1;
                IsoDirections::S
            }
        } else if (x as f32) < t.x {
            opp.0 -= 1;
            IsoDirections::W
        } else {
            start.0 -= 1;
            IsoDirections::E
        };
        let m = &*self.sprites;
        let opposite = self.cell.get_grid_square(opp.0, opp.1, z);
        let mut counter = opposite.is_some_and(|s| s.has(m.tables.flag.solidtrans));
        let floor = opposite.is_some_and(|s| s.treat_as_solid_floor(m));
        let sheet_rope = opposite.is_some_and(|s| self.env().can_climb_down_sheet_rope(Some(s)));
        let mut end = opp;
        if let Some(os) = opposite.filter(|_| counter) {
            if t.zombie {
                let next = os.get_adjacent_square(dir, self.cell);
                match next {
                    Some(n) if self.is_free_square(Some(n)) && self.open_between(os, n) => end = (n.x, n.y),
                    _ => counter = false,
                }
            } else {
                match self.free_square_after_obstacles(os, dir) {
                    Some(n) => end = (n.x, n.y),
                    None => counter = false,
                }
            }
        }
        Some(WindowClimb {
            portal,
            can_climb,
            portal_can_climb,
            broken_glass,
            barbed,
            north,
            square: (x, y, z),
            climb_dir: dir,
            start,
            opposite: opp,
            end,
            z,
            counter,
            floor,
            sheet_rope,
        })
    }

    /// `startSq.getWindowTo(endSq)`, `getWindowThumpableTo`, then
    /// `getHoppableTo`.
    pub fn window_between_squares(&self, start: (i32, i32, i32), end: (i32, i32, i32)) -> Option<(ObjectRef, ThumpKind)> {
        let a = self.cell.get_grid_square(start.0, start.1, start.2)?;
        let b = self.cell.get_grid_square(end.0, end.1, end.2)?;
        let hooks = self.env().hooks();
        let in_a = |o: &IsoObject| a.objects.iter().any(|x| std::ptr::eq(x, o));
        let found = a
            .get_window_to(Some(b), self.cell)
            .map(|o| (if in_a(o) { a } else { b }, o))
            .or_else(|| a.get_window_thumpable_to(Some(b), self.sprites, self.cell, &hooks))
            .map(|(s, o)| oref(s, o))
            .or_else(|| {
                self.collide_env()
                    .get_hoppable_to(a, Some(b))
                    .map(|(s, id)| ObjectRef { x: s.x, y: s.y, z: s.z, id })
            })?;
        let (_, o) = self.find(found)?;
        Some((found, Self::kind_of(o)))
    }

    /// `square.getBrokenGlass() != null`.
    pub fn broken_glass_at(&self, p: (i32, i32, i32)) -> bool {
        self.cell
            .get_grid_square(p.0, p.1, p.2)
            .is_some_and(|s| s.get_special_objects().any(|o| matches!(o.kind, ObjectKind::BrokenGlass)))
    }

    /// `IsoThumpable.health -= damage`, `destroy()` at 0 or below.
    pub fn damage_thumpable_at(&mut self, r: ObjectRef, damage: i32) {
        let Some(o) = self.find_mut(r) else { return };
        let ObjectKind::Thumpable(t) = &mut o.kind else { return };
        t.health -= damage;
        if t.health <= 0 {
            self.destroy_thumpable(r);
        }
    }

    // ---- fences ----

    /// `startSq.getHoppableTo(endSq)`.
    pub fn hoppable_between_squares(&self, start: (i32, i32, i32), end: (i32, i32, i32)) -> Option<ObjectRef> {
        let a = self.cell.get_grid_square(start.0, start.1, start.2)?;
        let b = self.cell.get_grid_square(end.0, end.1, end.2)?;
        self.collide_env()
            .get_hoppable_to(a, Some(b))
            .map(|(s, id)| ObjectRef { x: s.x, y: s.y, z: s.z, id })
    }

    /// `ClimbOverFenceState.isMetalFence(fence)`.
    fn is_metal_fence(&self, o: &IsoObject) -> bool {
        let env = self.env();
        let mats = ["Material", "Material2", "Material3"].map(|k| env.obj_prop(o, k));
        if mats.iter().any(|m| matches!(m, Some("MetalBars") | Some("MetalWire"))) {
            return true;
        }
        if let ObjectKind::Thumpable(t) = &o.kind {
            if let Some(table) = t.mod_data.as_ref().or(o.mod_data.as_ref()) {
                return table
                    .entries
                    .iter()
                    .any(|(k, _)| matches!(k, pz_world::save::lua_table::LuaKey::Str(s) if s.contains("MetalPipe")));
            }
        }
        false
    }

    /// `getFenceType(fence)` and `getTripType(fence)` as parameter values.
    fn fence_sound_types(&self, o: &IsoObject) -> (i32, i32) {
        // ParameterFenceTypeLow.FenceType / ParameterTripObstacleType.ObstacleType.
        const WOOD: i32 = 0;
        let env = self.env();
        if o.sprite.is_none() {
            return (WOOD, WOOD);
        }
        if let Some(low) = env.obj_prop(o, "FenceTypeLow") {
            if low == "Sandbag" && self.object_name(o).is_some_and(|n| n.to_lowercase().contains("gravel")) {
                return (3, 3);
            }
            let fence = match low {
                "Wood" => 0,
                "Metal" => 1,
                "Sandbag" => 2,
                "Gravelbag" => 3,
                "Barbwire" => 4,
                "RoadBlock" => 5,
                "MetalGate" => 6,
                _ => WOOD,
            };
            // WOOD, METAL, SANDBAG, GRAVELBAG, BARBWIRE, ROADBLOCK -> WOOD,
            // METAL_GATE -> METAL_BARS (8).
            let trip = match fence {
                5 => 0,
                6 => 8,
                v => v,
            };
            return (fence, trip);
        }
        if let Some(high) = env.obj_prop(o, "FenceTypeHigh") {
            return match high {
                "Metal" => (1, 1),
                "MetalGate" => (6, 8),
                _ => (WOOD, WOOD),
            };
        }
        (WOOD, WOOD)
    }

    /// What `ClimbOverFenceState` reads of a hoppable.
    pub fn fence_info_of(&self, r: ObjectRef) -> Option<FenceInfo> {
        let (sq, o) = self.find(r)?;
        let (fence_type_low, trip_type) = self.fence_sound_types(o);
        Some(FenceInfo {
            square: (sq.x, sq.y, sq.z),
            hoppable_n: self.env().obj_has_flag(o, "HoppableN"),
            metal: self.is_metal_fence(o),
            fence_type_low,
            trip_type,
        })
    }

    /// `fence.damage = (short)(fence.damage - damage)`, then
    /// `destroyFence(dir)` (`BrokenFences`, the engine's) at 0 or below.
    pub fn damage_fence_at(&mut self, r: ObjectRef, damage: i32, dir: IsoDirections) {
        if self.find(r).is_none() {
            return;
        }
        let d = (self.object_damage(r) - damage) as i16 as i32;
        self.runtime.object_damage.insert(r, d);
        if d <= 0 {
            self.engine.destroy_fence(self.cell, r, dir);
        }
    }
}

// ---- walls and sheet ropes ----

impl StatesWorldAdapter<'_> {
    /// `IsoGridSquare.HasSlopedRoof()`: `HasSlopedRoofWest()` or
    /// `HasSlopedRoofNorth()`, which in the original also test the west
    /// roof types.
    fn has_sloped_roof(&self, sq: &IsoGridSquare) -> bool {
        let t = &self.sprites.tables;
        ["WestRoofB", "WestRoofM", "WestRoofT"].into_iter().any(|n| {
            let ty = t.object_type_from_string(n);
            ty != t.object_type_max() && sq.has_type(ty)
        })
    }

    /// `HasSlopedRoof() && !HasEave()` of the square above `sq`.
    fn sloped_roof_above(&self, sq: &IsoGridSquare) -> bool {
        self.cell
            .get_grid_square(sq.x, sq.y, sq.z + 1)
            .is_some_and(|a| self.has_sloped_roof(a) && !self.sq_flag(a, "isEave"))
    }

    /// The world's part of `IsoPlayer.canClimbOverWall(dir)` for a player
    /// on `at`.
    pub fn can_climb_over_wall_at(&self, at: (i32, i32, i32), dir: IsoDirections) -> bool {
        if !self.is_safe_to_climb_over(at, dir) {
            return false;
        }
        let Some(cur) = self.cell.get_grid_square(at.0, at.1, at.2) else { return false };
        if cur.have_roof || self.square_building_idx(cur).is_some() || self.sloped_roof_above(cur) {
            return false;
        }
        // The original dereferences the adjacent square.
        let Some(square) = cur.get_adjacent_square(dir, self.cell) else { return false };
        let m = &*self.sprites;
        if square.have_roof || square.is_solid(m) || square.is_solid_trans(m) || self.square_building_idx(square).is_some() {
            return false;
        }
        if self.sloped_roof_above(square) {
            return false;
        }
        let above = |s: &IsoGridSquare| self.cell.get_grid_square(s.x, s.y, s.z + 1);
        let wall = |s: &IsoGridSquare, collide: &str, hoppable: &str| {
            !self.sq_flag(s, "CantClimb")
                && s.has_type(m.tables.ty.wall)
                && self.sq_flag(s, collide)
                && !self.sq_flag(s, hoppable)
                && !above(s).is_some_and(|a| self.sq_flag(a, collide))
        };
        match dir {
            IsoDirections::N => wall(cur, "collideN", "HoppableN"),
            IsoDirections::S => wall(square, "collideN", "HoppableN"),
            IsoDirections::W => wall(cur, "collideW", "HoppableW"),
            IsoDirections::E => wall(square, "collideW", "HoppableW"),
            _ => false,
        }
    }

    /// `ClimbOverWallState.getClimbableWallN/W(square)`: the first wall
    /// object along that edge that can be climbed.
    fn climbable_wall<'s>(&self, sq: &'s IsoGridSquare, north: bool) -> Option<&'s IsoObject> {
        let env = self.env();
        let (collide, hoppable) = if north { ("collideN", "HoppableN") } else { ("collideW", "HoppableW") };
        sq.objects.iter().find(|o| {
            let Some(s) = o.sprite else { return false };
            !env.obj_has_flag(o, "CantClimb")
                && self.sprites.sprites[s].tile_type == self.sprites.tables.ty.wall
                && env.obj_has_flag(o, collide)
                && !env.obj_has_flag(o, hoppable)
        })
    }

    /// `ClimbOverWallState.getFence(owner)` and `getFenceType(fence)`.
    pub fn climbable_wall_fence_type_at(&self, start: (i32, i32, i32), end: (i32, i32, i32), dir: IsoDirections) -> Option<i32> {
        let s = self.cell.get_grid_square(start.0, start.1, start.2)?;
        let e = self.cell.get_grid_square(end.0, end.1, end.2)?;
        let wall = match dir {
            IsoDirections::N => self.climbable_wall(s, true),
            IsoDirections::E => self.climbable_wall(e, false),
            IsoDirections::W => self.climbable_wall(s, false),
            IsoDirections::S => self.climbable_wall(e, true),
            _ => None,
        }?;
        // ParameterFenceTypeHigh: WOOD 0, METAL 1, METAL_BARS 2.
        Some(match self.env().obj_prop(wall, "FenceTypeHigh") {
            Some("Metal") => 1,
            Some("MetalGate") => 2,
            _ => 0,
        })
    }

    /// What the sheet-rope climbs read of the square `(x, y, z)`.
    pub fn rope_square_at(&self, x: i32, y: i32, z: i32) -> Option<RopeSquare> {
        let sq = self.cell.get_grid_square(x, y, z)?;
        let env = self.env();
        let f = |n: &str| self.sq_flag(sq, n);
        let chunk = self.cell.chunks.get(&(x.div_euclid(8), y.div_euclid(8)));
        Some(RopeSquare {
            sheet_rope: env.is_sheet_rope_here(sq),
            top_of_sheet_rope: f("climbSheetTopN") || f("climbSheetTopS") || f("climbSheetTopW") || f("climbSheetTopE"),
            can_climb_here: env.can_climb_here(sq),
            solid_floor: sq.treat_as_solid_floor(self.sprites),
            solid_trans: sq.has(self.sprites.tables.flag.solidtrans),
            sheet_n: f("climbSheetN") || f("climbSheetTopN"),
            sheet_s: f("climbSheetS") || f("climbSheetTopS"),
            sheet_w: f("climbSheetW") || f("climbSheetTopW"),
            sheet_e: f("climbSheetE") || f("climbSheetTopE"),
            chunk_min_level: chunk.map_or(0, |c| c.min_level),
            chunk_max_level: chunk.map_or(0, |c| c.max_level),
        })
    }

    /// `ClimbSheetRopeState.calculateClimbOutcome(chr)` from the top of
    /// the rope `top`, facing `dir`.
    pub fn rope_climb_outcome_for(&mut self, t: &Thumper, top: (i32, i32, i32), dir: IsoDirections) -> RopeClimbOutcome {
        let blocked = RopeClimbOutcome { status: ClimbStatus::Blocked, target: None };
        let Some(sq) = self.cell.get_grid_square(top.0, top.1, top.2) else { return RopeClimbOutcome::default() };
        // The original dereferences the exit square.
        let Some(exit) = sq.get_adjacent_square(dir, self.cell) else { return RopeClimbOutcome::default() };
        let exit_p = (exit.x, exit.y, exit.z);
        if !exit.treat_as_solid_floor(self.sprites) {
            return blocked;
        }
        let in_sq = |o: &IsoObject| sq.objects.iter().any(|x| std::ptr::eq(x, o));
        let window = sq.get_window_to(Some(exit), self.cell).map(|o| oref(if in_sq(o) { sq } else { exit }, o));
        if let Some(w) = window {
            let open = self.find(w).is_some_and(|(_, o)| matches!(&o.kind, ObjectKind::Window(win) if win.open));
            if !open {
                self.engine.toggle_window(self.cell, w, t.id);
            }
            let can = self.find(w).is_some_and(|(s, o)| self.window_can_climb_through(s, o, Some(t)));
            if !can {
                return blocked;
            }
            return RopeClimbOutcome { status: ClimbStatus::OpenWindow, target: Some(w) };
        }
        let sq = self.cell.get_grid_square(top.0, top.1, top.2).expect("top square");
        let exit = self.cell.get_grid_square(exit_p.0, exit_p.1, exit_p.2);
        let hooks = self.env().hooks();
        if let Some((s, o)) = sq.get_window_thumpable_to(exit, self.sprites, self.cell, &hooks) {
            if !self.thumpable_can_climb_through(s, o, Some(t)) {
                return blocked;
            }
            return RopeClimbOutcome { status: ClimbStatus::OpenWindow, target: Some(oref(s, o)) };
        }
        let north = matches!(dir, IsoDirections::N | IsoDirections::S);
        let m = &*self.sprites;
        let hoppable_thumpable = sq.get_edge_element_to(exit, self.cell, &|s: &IsoGridSquare, facing| {
            s.get_special_objects()
                .find(|o| matches!(&o.kind, ObjectKind::Thumpable(th) if o.is_hoppable(m) && pz_world::square::EdgeFacing::of_north(th.north) == facing))
                .map(|_| ())
        });
        if hoppable_thumpable.is_some() {
            if !self.can_climb_through_helper(t, sq, exit, north) {
                return blocked;
            }
            return RopeClimbOutcome { status: ClimbStatus::Fence, target: None };
        }
        if let Some((s, o)) = sq.get_window_frame_to(exit, self.cell) {
            let ObjectKind::WindowFrame { north: frame_north } = o.kind else { return RopeClimbOutcome::default() };
            if !self.frame_can_climb_through(s, frame_north, Some(t)) {
                return blocked;
            }
            return RopeClimbOutcome { status: ClimbStatus::WindowFrame, target: Some(oref(s, o)) };
        }
        let wall_hoppable = sq.get_edge_element_to(exit, self.cell, &|s: &IsoGridSquare, facing| {
            s.objects.iter().find(|o| o.get_hoppable_direction(m) == Some(facing)).map(|_| ())
        });
        if wall_hoppable.is_some() {
            if !self.can_climb_through_helper(t, sq, exit, north) {
                return blocked;
            }
            return RopeClimbOutcome { status: ClimbStatus::Fence, target: None };
        }
        RopeClimbOutcome::default()
    }

    /// `sq.getFloor().getSprite().getName()`: the first object whose
    /// sprite has `solidfloor`.
    pub fn floor_sprite_name_at(&self, p: (i32, i32, i32)) -> Option<String> {
        let sq = self.cell.get_grid_square(p.0, p.1, p.2)?;
        let floor = sq.objects.iter().find(|o| o.sprite.is_some_and(|s| self.sprites.sprites[s].properties.has(self.sprites.tables.flag.solidfloor)))?;
        self.env().sprite_name(floor).map(str::to_string)
    }
}
