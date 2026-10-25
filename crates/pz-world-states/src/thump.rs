//! The thump code of the special objects: `getThumpableFor`,
//! `getThumpCondition` and `Thump` of `IsoDoor`, `IsoThumpable`,
//! `IsoWindow`, `IsoWindowFrame`, `IsoBarricade` and plain `IsoObject`s,
//! with the damage and destruction they do to the cell (`Damage`,
//! `destroy`, `destroyDoubleDoor`, `destroyGarageDoor`, `smashWindow`,
//! `addBrokenGlass`), and the zombie's sheet-rope damage.

use pz_characters::character::state::{ThumpInfo, ThumpKind, ThumpTarget};
use pz_pathfind::mover::ObjectRef;
use pz_world::java::parse_int;
use pz_world::object_slot::{IsoObjectRef, ObjectSlot};
use pz_world::objects::{IsoObject, ObjectKind};
use pz_world::square::EdgeFacing;
use pz_world::{IsoDirections, IsoGridSquare, SquareRef};

use crate::{StatesWorldAdapter, Thumper, WorldSoundRequest};

const DD_NORTH_CLOSED_X: [i32; 4] = [0, 1, 2, 3];
const DD_NORTH_OPEN_X: [i32; 4] = [0, 0, 3, 3];
const DD_NORTH_CLOSED_Y: [i32; 4] = [0, 0, 0, 0];
const DD_NORTH_OPEN_Y: [i32; 4] = [0, 1, 1, 0];
const DD_WEST_CLOSED_X: [i32; 4] = [0, 0, 0, 0];
const DD_WEST_OPEN_X: [i32; 4] = [0, 1, 1, 0];
const DD_WEST_CLOSED_Y: [i32; 4] = [0, -1, -2, -3];
const DD_WEST_OPEN_Y: [i32; 4] = [0, 0, -3, -3];

/// The new thump target `Thump` leaves on the thumper: `None` keeps it.
pub(crate) type TargetOut = Option<Option<ThumpTarget>>;

fn oref(sq: &IsoGridSquare, o: &IsoObject) -> ObjectRef {
    ObjectRef {
        x: sq.x,
        y: sq.y,
        z: sq.z,
        id: o.id,
    }
}

/// `door.north` / `door.open` of an `IsoDoor` or `IsoThumpable`.
fn door_like(o: &IsoObject) -> Option<(bool, bool)> {
    match &o.kind {
        ObjectKind::Door(d) => Some((d.north, d.open)),
        ObjectKind::Thumpable(t) => Some((t.north, t.open)),
        _ => None,
    }
}

impl StatesWorldAdapter<'_> {
    /// The square and object an [`ObjectRef`] names.
    pub fn find(&self, r: ObjectRef) -> Option<(SquareRef, IsoObjectRef)> {
        let square = self.cell.get_grid_square(r.x, r.y, r.z)?;
        let object = {
            let sq = square.borrow();
            sq.objects[sq.object_index(r.id)?].ref_handle()
        };
        Some((square, object))
    }

    /// Pin the actual object allocation independently of its square's guard.
    pub(crate) fn object_ref(&self, r: ObjectRef) -> Option<IsoObjectRef> {
        self.find(r).map(|(_, object)| object)
    }

    /// Mutation phases end before their caller performs another lookup or callback.
    pub(crate) fn with_object_mut<R>(
        &self,
        r: ObjectRef,
        f: impl FnOnce(&mut IsoObject) -> R,
    ) -> Option<R> {
        let object = self.object_ref(r)?;
        let result = f(&mut object.borrow_mut());
        Some(result)
    }

    fn sound(
        &mut self,
        source: Option<i32>,
        r: ObjectRef,
        radius: i32,
        volume: i32,
        stress_mod: f32,
    ) {
        self.engine.add_world_sound(WorldSoundRequest {
            source,
            x: r.x,
            y: r.y,
            z: r.z,
            radius,
            volume,
            stress_humans: true,
            zombie_ignore_dist: 4.0,
            stress_mod,
        });
    }

    fn force_awake_at(&mut self, r: ObjectRef) {
        let b = self
            .at((r.x, r.y, r.z))
            .and_then(|s| self.square_building_idx(&s.borrow()));
        if let Some(b) = b {
            self.engine.force_awake(b);
        }
    }

    // ---- barricades ----

    /// `IsoBarricade.GetBarricadeOnSquare(square, dir)`.
    fn barricade_on(&self, sq: Option<&IsoGridSquare>, dir: IsoDirections) -> Option<ObjectRef> {
        let sq = sq?;
        sq.special_objects.iter().find_map(|&id| {
            let o = sq.objects[sq.object_index(id)?].borrow();
            match &o.kind {
                ObjectKind::Barricade(b) if b.forward_dir() == dir => Some(oref(sq, &o)),
                _ => None,
            }
        })
    }

    fn opposite(&self, sq: &IsoGridSquare, north: bool) -> Option<SquareRef> {
        if north {
            self.cell.get_grid_square(sq.x, sq.y - 1, sq.z)
        } else {
            self.cell.get_grid_square(sq.x - 1, sq.y, sq.z)
        }
    }

    /// `chr` stands on the far side of an edge element (`north ? chr.y <
    /// sq.y : chr.x < sq.x`).
    fn beyond(sq: &IsoGridSquare, north: bool, chr: Option<&Thumper>) -> bool {
        chr.is_some_and(|c| {
            if north {
                c.y < sq.y as f32
            } else {
                c.x < sq.x as f32
            }
        })
    }

    fn same_side(&self, sq: &IsoGridSquare, north: bool) -> Option<ObjectRef> {
        self.barricade_on(
            Some(sq),
            if north {
                IsoDirections::N
            } else {
                IsoDirections::W
            },
        )
    }

    fn opposite_side(&self, sq: &IsoGridSquare, north: bool) -> Option<ObjectRef> {
        self.barricade_on(
            self.opposite(sq, north)
                .as_ref()
                .map(|s| s.borrow())
                .as_deref(),
            if north {
                IsoDirections::S
            } else {
                IsoDirections::E
            },
        )
    }

    /// `IsoBarricade.GetBarricadeForCharacter(obj, chr)`.
    fn barricade_for_character(
        &self,
        sq: &IsoGridSquare,
        north: bool,
        chr: Option<&Thumper>,
    ) -> Option<ObjectRef> {
        if Self::beyond(sq, north, chr) {
            self.opposite_side(sq, north)
        } else {
            self.same_side(sq, north)
        }
    }

    /// `IsoBarricade.GetBarricadeOppositeCharacter(obj, chr)`.
    fn barricade_opposite_character(
        &self,
        sq: &IsoGridSquare,
        north: bool,
        chr: Option<&Thumper>,
    ) -> Option<ObjectRef> {
        if Self::beyond(sq, north, chr) {
            self.same_side(sq, north)
        } else {
            self.opposite_side(sq, north)
        }
    }

    // ---- window and frame queries ----

    /// `IsoWindow.isInvincible()`.
    pub(crate) fn window_invincible(&self, sq: &IsoGridSquare, o: &IsoObject, north: bool) -> bool {
        let env = self.env();
        if !env.sq_has(sq, "makeWindowInvincible") {
            return false;
        }
        let cut = if north { "cutN" } else { "cutW" };
        if sq.objects.iter().any(|x| {
            let x = x.borrow();
            x.id != o.id
                && env.obj_has_flag(&x, cut)
                && env.obj_has_flag(&x, "makeWindowInvincible")
        }) {
            return true;
        }
        env.obj_has_flag(&o, "makeWindowInvincible")
    }

    /// `IsoWindow.canClimbThroughHelper(chr, sq, oppositeSq, north)`.
    pub(crate) fn can_climb_through_helper(
        &self,
        chr: &Thumper,
        sq: &IsoGridSquare,
        opp: Option<&IsoGridSquare>,
        north: bool,
    ) -> bool {
        if chr.animal {
            return false;
        }
        let env = self.env();
        let (mut test, mut dx, mut dy) = (Some(sq), 0.5f32, 0.5f32);
        if north {
            if chr.y >= sq.y as f32 {
                test = opp;
                dy = 0.7;
            } else {
                dy = 0.3;
            }
        } else if chr.x >= sq.x as f32 {
            test = opp;
            dx = 0.7;
        } else {
            dx = 0.3;
        }
        let Some(t) = test else { return false };
        if t.is_solid(self.sprites) || t.has(self.sprites.tables.flag.water) {
            return false;
        }
        if !(env.can_climb_down_sheet_rope(Some(t))
            || env.has_stairs_below(t)
            || self
                .engine
                .can_stand_at(t.x as f32 + dx, t.y as f32 + dy, t.z, 19))
        {
            return !t.treat_as_solid_floor(self.sprites);
        }
        true
    }

    pub(crate) fn window_can_climb_through(
        &self,
        sq: &IsoGridSquare,
        o: &IsoObject,
        chr: Option<&Thumper>,
    ) -> bool {
        let ObjectKind::Window(w) = &o.kind else {
            return false;
        };
        if self.window_invincible(sq, &o, w.north) || self.env().hooks().object_barricaded(sq, o) {
            return false;
        }
        if let Some(c) = chr {
            if !self.can_climb_through_helper(
                c,
                sq,
                self.opposite(sq, w.north)
                    .as_ref()
                    .map(|s| s.borrow())
                    .as_deref(),
                w.north,
            ) {
                return false;
            }
        }
        if self.engine.window_closing_success(oref(sq, &o)) {
            return false;
        }
        if w.health <= 0 || w.destroyed {
            return true;
        }
        w.open
    }

    /// `IsoWindowFrame.getWindow()`.
    fn frame_window(&self, sq: &IsoGridSquare, north: bool) -> Option<IsoObjectRef> {
        sq.get_window_facing(EdgeFacing::of_north(north))
    }

    pub(crate) fn frame_can_climb_through(
        &self,
        sq: &IsoGridSquare,
        north: bool,
        chr: Option<&Thumper>,
    ) -> bool {
        let hooks = self.env().hooks();
        if let Some(w) = self.frame_window(sq, north) {
            if hooks.object_barricaded(sq, &w.borrow()) {
                return false;
            }
        } else {
            let (a, b) = hooks.barricades_for(sq, north);
            if a.is_some() || b.is_some() {
                return false;
            }
        }
        if let Some(c) = chr {
            let opp = if north {
                sq.get_adjacent_square(IsoDirections::N, self.cell)
            } else {
                sq.get_adjacent_square(IsoDirections::W, self.cell)
            };
            if !self.can_climb_through_helper(
                c,
                sq,
                opp.as_ref().map(|s| s.borrow()).as_deref(),
                north,
            ) {
                return false;
            }
        }
        true
    }

    pub(crate) fn thumpable_can_climb_through(
        &self,
        sq: &IsoGridSquare,
        o: &IsoObject,
        chr: Option<&Thumper>,
    ) -> bool {
        let ObjectKind::Thumpable(t) = &o.kind else {
            return false;
        };
        let env = self.env();
        if !env.is_window(o) || env.hooks().object_barricaded(sq, o) {
            return false;
        }
        chr.is_none_or(|c| {
            self.can_climb_through_helper(
                c,
                sq,
                self.opposite(sq, t.north)
                    .as_ref()
                    .map(|s| s.borrow())
                    .as_deref(),
                t.north,
            )
        })
    }

    fn breakable(&self, o: &IsoObject) -> bool {
        self.env()
            .sprite_name(&o)
            .is_some_and(|n| self.engine.is_breakable_fence(n))
    }

    /// `IsoThumpable.isTallHoppable()` / `IsoObject.isTallHoppable()`.
    fn tall_hoppable(&self, o: &IsoObject) -> bool {
        let env = self.env();
        let tall = |s: Option<pz_world::sprite::SpriteIdx>| {
            s.is_some_and(|s| {
                let p = &self.sprites.sprites[s].properties;
                ["TallHoppableN", "TallHoppableW"]
                    .into_iter()
                    .any(|n| env.flag(n).is_some_and(|f| p.has(f)))
            })
        };
        if let ObjectKind::Thumpable(t) = &o.kind {
            if t.is_door && !t.open {
                if let Some(c) = t.closed_sprite {
                    return tall(Some(c));
                }
            }
        }
        tall(o.sprite)
    }

    /// `IsoDoor.isHoppable()`.
    fn door_hoppable(&self, o: &IsoObject) -> bool {
        let ObjectKind::Door(d) = &o.kind else {
            return false;
        };
        if d.open {
            return false;
        }
        let f = &self.sprites.tables.flag;
        d.closed_sprite.is_some_and(|s| {
            let p = &self.sprites.sprites[s].properties;
            p.has(f.HoppableN) || p.has(f.HoppableW)
        })
    }

    /// `IsoObject.damage` of a plain object.
    pub(crate) fn object_damage(&self, r: ObjectRef) -> i32 {
        *self.runtime.object_damage.get(&r).unwrap_or(&100)
    }

    // ---- getThumpableFor / getThumpCondition ----

    /// `obj.getThumpableFor(chr)`.
    pub fn thumpable_for(&self, r: ObjectRef, chr: Option<&Thumper>) -> Option<ObjectRef> {
        let (sq_owner, o_owner) = self.find(r)?;
        let sq = sq_owner.borrow();
        let o = o_owner.borrow();
        let env = self.env();
        match &o.kind {
            ObjectKind::Door(d) => self
                .barricade_for_character(&sq, d.north, chr)
                .or_else(|| self.barricade_opposite_character(&sq, d.north, chr))
                .or((!d.open).then_some(r)),
            ObjectKind::Thumpable(t) => {
                if t.is_door || env.is_window(&o) {
                    if let Some(b) = self
                        .barricade_for_character(&sq, t.north, chr)
                        .or_else(|| self.barricade_opposite_character(&sq, t.north, chr))
                    {
                        return Some(b);
                    }
                }
                let crawling = chr.is_some_and(|c| c.zombie && c.crawling);
                let hoppable = env.hooks().thumpable_is_hoppable(&o, t);
                let mut thumpable = t.is_thumpable;
                if !thumpable && crawling && self.breakable(&o) {
                    thumpable = true;
                }
                if !thumpable && crawling && hoppable {
                    thumpable = true;
                }
                if !thumpable {
                    return None;
                }
                if t.is_door && t.open || env.is_window(&o) {
                    return None;
                }
                if !crawling && hoppable && !self.tall_hoppable(&o) {
                    return None;
                }
                Some(r)
            }
            ObjectKind::Window(w) => {
                if let Some(b) = self.barricade_for_character(&sq, w.north, chr) {
                    return Some(b);
                }
                if w.destroyed || w.open {
                    return self.barricade_opposite_character(&sq, w.north, chr);
                }
                Some(r)
            }
            ObjectKind::WindowFrame { north } => {
                if let Some(w) = self.frame_window(&sq, *north) {
                    return self.thumpable_for(oref(&sq, &w.borrow()), chr);
                }
                self.barricade_for_character(&sq, *north, chr)
                    .or_else(|| self.barricade_opposite_character(&sq, *north, chr))
            }
            ObjectKind::Barricade(b) => (!b.is_destroyed()).then_some(r),
            ObjectKind::Object => {
                if self.object_damage(r) <= 0 {
                    return None;
                }
                if o.moved_thumpable {
                    return Some(r);
                }
                // BentFences (unbent fences thumped by a hunting zombie) is
                // not ported: Lua registers no bent fences in vanilla
                // B42 maps' default `BentFences.isEnabled()` path here.
                if self.breakable(&o) {
                    return chr.is_some_and(|c| c.zombie && c.crawling).then_some(r);
                }
                None
            }
            _ => None,
        }
    }

    /// `thumpable.getThumpCondition()`.
    pub fn thump_condition(&self, r: ObjectRef) -> f32 {
        let Some((_, o_owner)) = self.find(r) else {
            return 0.0;
        };
        let o = o_owner.borrow();
        let ratio = |h: i32, max: i32| h.clamp(0, max) as f32 / max as f32;
        match &o.kind {
            ObjectKind::Door(d) => {
                if d.max_health <= 0 {
                    0.0
                } else {
                    ratio(d.health, d.max_health)
                }
            }
            ObjectKind::Thumpable(t) => ratio(t.health, t.max_health),
            ObjectKind::Window(w) => ratio(w.health, w.max_health),
            ObjectKind::Barricade(b) => {
                if b.metal_health > 0 {
                    return ratio(b.metal_health, 5000);
                }
                if b.metal_bar_health > 0 {
                    return ratio(b.metal_bar_health, 3000);
                }
                for i in (0..4).rev() {
                    if b.plank_health[i] > 0 {
                        return ratio(b.plank_health[i], 1000);
                    }
                }
                0.0
            }
            _ => ratio(self.object_damage(r), 100),
        }
    }

    pub(crate) fn kind_of(o: &IsoObject) -> ThumpKind {
        match &o.kind {
            ObjectKind::Door(_) => ThumpKind::Door,
            ObjectKind::Window(_) => ThumpKind::Window,
            ObjectKind::WindowFrame { .. } => ThumpKind::WindowFrame,
            ObjectKind::Thumpable(_) => ThumpKind::Thumpable,
            ObjectKind::Barricade(_) => ThumpKind::Barricade,
            _ => ThumpKind::Other,
        }
    }

    /// `getSoundPrefix()` of a door or thumpable: the closed sprite's
    /// `DoorSound`, else `WoodDoor`.
    fn sound_prefix(&self, o: &IsoObject) -> String {
        let closed = match &o.kind {
            ObjectKind::Door(d) => d.closed_sprite,
            ObjectKind::Thumpable(t) => t.closed_sprite,
            _ => return String::new(),
        };
        closed
            .and_then(|s| {
                self.sprites.sprites[s]
                    .properties
                    .get(&self.sprites.alias, "DoorSound")
            })
            .unwrap_or("WoodDoor")
            .to_string()
    }

    /// `IsoDoor.getThumpSound()` / `IsoThumpable.getThumpSound()`.
    fn thump_sound(&self, o: &IsoObject) -> String {
        let env = self.env();
        let by_prefix = |p: &str| match p {
            "MetalGate" => Some("ZombieThumpChainlinkFence"),
            "MetalPoleGate" | "MetalPoleGateDouble" => Some("ZombieThumpMetalPoleFence"),
            "GarageDoor" => Some("ZombieThumpGarageDoor"),
            "MetalDoor" | "PrisonMetalDoor" => Some("ZombieThumpMetal"),
            "SlidingGlassDoor" => Some("ZombieThumpWindow"),
            _ => None,
        };
        let own = env
            .obj_prop(&o, "ThumpSound")
            .filter(|s| !s.trim().is_empty());
        match &o.kind {
            ObjectKind::Door(_) => own.map(str::to_string).unwrap_or_else(|| {
                by_prefix(&self.sound_prefix(o))
                    .unwrap_or("ZombieThumpGeneric")
                    .to_string()
            }),
            ObjectKind::Thumpable(t) => {
                if let Some(s) = own {
                    return s.to_string();
                }
                if t.is_door {
                    if let Some(s) = by_prefix(&self.sound_prefix(o)) {
                        return s.to_string();
                    }
                }
                t.thump_sound.clone()
            }
            _ => "ZombieThumpGeneric".into(),
        }
    }

    fn facing_position(&self, sq: &IsoGridSquare, o: &IsoObject) -> (f32, f32) {
        let (x, y) = (sq.x as f32, sq.y as f32);
        let edge = |north: bool| if north { (x + 0.5, y) } else { (x, y + 0.5) };
        match &o.kind {
            ObjectKind::Door(d) => edge(d.north),
            ObjectKind::Window(w) => edge(w.north),
            ObjectKind::WindowFrame { north } => edge(*north),
            ObjectKind::Thumpable(t) => {
                let env = self.env();
                let f = &self.sprites.tables.flag;
                let collide = env
                    .props(o)
                    .is_some_and(|p| p.has(f.collideN) || p.has(f.collideW));
                if t.is_door || t.is_door_frame || env.is_window(o) || t.is_hoppable || collide {
                    edge(t.north)
                } else {
                    (x + 0.5, y + 0.5)
                }
            }
            ObjectKind::Barricade(b) => match b.forward_dir() {
                IsoDirections::N => (x + 0.5, y),
                IsoDirections::S => (x + 0.5, y + 1.0),
                IsoDirections::W => (x, y + 0.5),
                IsoDirections::E => (x + 1.0, y + 0.5),
                _ => (x, y + 0.5),
            },
            // `IsoObject.getFacingPosition`: the square's centre.
            _ => (x + 0.5, y + 0.5),
        }
    }

    /// What the zombie code reads about `r` as `chr` sees it.
    pub fn object_thump_info(&self, r: ObjectRef, chr: Option<&Thumper>) -> Option<ThumpInfo> {
        let (sq_owner, o_owner) = self.find(r)?;
        let sq = sq_owner.borrow();
        let o = o_owner.borrow();
        let env = self.env();
        let mut i = ThumpInfo {
            kind: Self::kind_of(&o),
            square: Some((sq.x, sq.y, sq.z)),
            in_world: true,
            facing_position: self.facing_position(&sq, &o),
            ..Default::default()
        };
        let north = match &o.kind {
            ObjectKind::Door(d) => {
                i.open = d.open;
                i.hoppable = self.door_hoppable(&o);
                i.tall_hoppable = self.tall_hoppable(&o);
                Some(d.north)
            }
            ObjectKind::Thumpable(t) => {
                i.open = t.open;
                i.is_door = t.is_door;
                i.hoppable = env.hooks().thumpable_is_hoppable(&o, t);
                i.tall_hoppable = self.tall_hoppable(&o);
                i.can_climb_through = self.thumpable_can_climb_through(&sq, &o, chr);
                i.block_all_the_square = t.block_all_the_square;
                Some(t.north)
            }
            ObjectKind::Window(w) => {
                i.open = w.open;
                i.destroyed = w.destroyed;
                i.can_climb_through = self.window_can_climb_through(&sq, &o, chr);
                Some(w.north)
            }
            ObjectKind::WindowFrame { north } => {
                i.can_climb_through = self.frame_can_climb_through(&sq, *north, chr);
                Some(*north)
            }
            ObjectKind::Barricade(b) => {
                i.destroyed = b.is_destroyed();
                i.north = matches!(b.forward_dir(), IsoDirections::N | IsoDirections::S);
                None
            }
            _ => {
                i.destroyed = self.object_damage(r) <= 0;
                None
            }
        };
        if let Some(n) = north {
            i.north = n;
            i.opposite_square = self.opposite(&sq, n).map(|s| {
                let s = s.borrow();
                (s.x, s.y, s.z)
            });
        }
        if let Some(tf) = self.thumpable_for(r, chr) {
            let (tsq_owner, to_owner) = self.find(tf)?;
            let tsq = tsq_owner.borrow();
            let to = to_owner.borrow();
            i.thumpable_for = Some(ThumpTarget::Object(tf));
            i.thumpable_for_kind = Self::kind_of(&to);
            i.thump_condition = self.thump_condition(tf);
            i.thump_sound = self.thump_sound(&to);
            i.sound_prefix = self.sound_prefix(&to);
            if let ObjectKind::Barricade(b) = &to.kind {
                i.barricade_metal = b.is_metal() || b.is_metal_bar();
                i.barricade_planks = b.num_planks();
                if let Some(n) = north {
                    i.barricade_for_character =
                        self.barricade_for_character(&sq, n, chr) == Some(tf);
                }
            }
            let _ = tsq;
        }
        Some(i)
    }

    // ---- Thump ----

    /// `obj.Thump(chr)` (one thump event).
    pub(crate) fn thump_object(&mut self, r: ObjectRef, t: &Thumper, out: &mut TargetOut) {
        let Some((_, o_owner)) = self.find(r) else {
            return;
        };
        let o = o_owner.borrow();
        let kind = Self::kind_of(&o);
        drop(o);
        if kind != ThumpKind::Barricade {
            if kind == ThumpKind::Thumpable && !self.settings.thump_on_construction {
                return;
            }
            if kind == ThumpKind::Window
                || kind == ThumpKind::Thumpable
                || kind == ThumpKind::Door
                || kind == ThumpKind::Other
            {
                let Some(tf) = self.thumpable_for(r, Some(t)) else {
                    return;
                };
                if tf != r {
                    self.thump_object(tf, t, out);
                    return;
                }
            }
        }
        match kind {
            ThumpKind::Door => self.thump_door(r, t, out),
            ThumpKind::Thumpable => self.thump_thumpable(r, t, out),
            ThumpKind::Window => self.thump_window(r, t, out),
            ThumpKind::Barricade => self.thump_barricade(r, t),
            ThumpKind::Other => self.thump_plain(r, t, out),
            // `IsoWindowFrame` is not a `Thumpable`.
            _ => {}
        }
    }

    fn locked_out(&self, t: &Thumper, locked: bool) -> bool {
        let exterior = |p: (i32, i32, i32)| {
            self.at(p)
                .is_some_and(|s| s.borrow().is_exterior(self.sprites))
        };
        locked && t.current_square.is_none_or(exterior)
    }

    fn door_damage(&mut self, r: ObjectRef, amount: i32) {
        if self.settings.tutorial {
            return;
        }
        self.with_object_mut(r, |o| {
            if let ObjectKind::Door(d) = &mut o.kind {
                d.health -= amount;
            }
        });
    }

    fn thump_door(&mut self, r: ObjectRef, t: &Thumper, out: &mut TargetOut) {
        let door = |s: &Self| {
            s.object_ref(r).and_then(|o| match &o.borrow().kind {
                ObjectKind::Door(d) => Some((d.open, d.locked, d.health)),
                _ => None,
            })
        };
        let Some((open, locked, _)) = door(self) else {
            return;
        };
        if t.zombie {
            if t.cognition == 1 && !open && !self.locked_out(t, locked) {
                self.engine.toggle_door(self.cell, r, t.id);
                match door(self) {
                    Some((true, _, _)) | None => return,
                    _ => {}
                }
            }
            if self.settings.client {
                self.engine.client_thump_hit(t.id, ThumpTarget::Object(r));
                return;
            }
            let tot = self.engine.surrounding_thumpers(t);
            if tot >= 2 {
                self.door_damage(r, t.strength);
                if self.settings.lore_strength == 1 {
                    self.door_damage(r, tot * 2);
                }
            }
            if self.settings.last_stand {
                self.door_damage(r, 1);
            }
            self.sound(Some(t.id), r, 20, 20, 15.0);
        }
        let Some((_, _, health)) = door(self) else {
            return;
        };
        if health <= 0 {
            self.force_awake_at(r);
            let prefix = self
                .find(r)
                .map(|(_, o)| self.sound_prefix(&o.borrow()))
                .unwrap_or_default();
            self.engine
                .play_sound(t.id, &format!("{prefix}Break"), Some(r));
            self.sound(None, r, 10, 20, 15.0);
            *out = Some(None);
            if self.destroy_double_door(r) || self.destroy_garage_door(r) {
                return;
            }
            self.destroy_door(r);
        }
    }

    fn thump_thumpable(&mut self, r: ObjectRef, t: &Thumper, out: &mut TargetOut) {
        let Some((_, o_owner)) = self.find(r) else {
            return;
        };
        let o = o_owner.borrow();
        let ObjectKind::Thumpable(th) = &o.kind else {
            return;
        };
        let (is_door, open, locked, thump_dmg) = (th.is_door, th.open, th.locked, th.thump_dmg);
        let breakable = self.breakable(&o);
        let stress = self.stress_mod_from_thumping(&o);
        drop(o);
        if t.zombie {
            if t.cognition == 1 && is_door && !open && !locked {
                self.engine.toggle_door(self.cell, r, t.id);
                return;
            }
            if self.settings.client {
                self.engine.client_thump_hit(t.id, ThumpTarget::Object(r));
                self.sound(Some(t.id), r, 20, 20, 15.0);
                return;
            }
            let total = self.engine.surrounding_thumpers(t);
            let amount = if total >= thump_dmg {
                1
            } else {
                let p = self.runtime.partial_thump_dmg.entry(r).or_insert(0.0);
                *p += total as f32 / thump_dmg as f32;
                let a = *p as i32;
                if a > 0 {
                    *p -= a as f32;
                }
                a.max(0)
            };
            if amount > 0 {
                self.with_object_mut(r, |o| {
                    if let ObjectKind::Thumpable(th) = &mut o.kind {
                        th.health -= amount;
                    }
                });
            }
            self.sound(Some(t.id), r, 20, 20, stress);
        }
        let Some((_, o_owner)) = self.find(r) else {
            return;
        };
        let o = o_owner.borrow();
        let ObjectKind::Thumpable(th) = &o.kind else {
            return;
        };
        let destroyed = th.health <= 0;
        let break_sound = (destroyed && !breakable).then(|| self.engine.thumpable_break_sound(&o));
        drop(o);
        if destroyed {
            if let Some(s) = break_sound {
                self.engine.play_sound(t.id, &s, Some(r));
            }
            self.sound(None, r, 10, 20, 15.0);
            *out = Some(None);
            if self.destroy_double_door(r) || self.destroy_garage_door(r) {
                return;
            }
            if breakable {
                let dir = self.break_dir(r, t);
                self.engine.destroy_fence(self.cell, r, dir);
                return;
            }
            self.destroy_thumpable(r);
        }
    }

    /// `IsoObject.getStressModFromThumping()`.
    fn stress_mod_from_thumping(&self, o: &IsoObject) -> f32 {
        let env = self.env();
        let mut m = if matches!(o.kind, ObjectKind::Thumpable(_)) {
            15.0
        } else {
            25.0
        };
        if env.obj_has_prop(&o, "FenceTypeHigh") {
            m = 7.0;
            if ["WallNTrans", "WallWTrans", "DoorTrans"]
                .into_iter()
                .any(|p| env.obj_has_prop(&o, p))
            {
                m = 0.0;
            }
        }
        m
    }

    /// The `dirBreak` of a broken fence.
    fn break_dir(&self, r: ObjectRef, t: &Thumper) -> IsoDirections {
        let env = self.env();
        let f = &self.sprites.tables.flag;
        let (n, w) = self.find(r).map_or((false, false), |(_, o)| {
            env.props(&o.borrow())
                .map_or((false, false), |p| (p.has(f.collideN), p.has(f.collideW)))
        });
        let ns = if t.y >= r.y as f32 {
            IsoDirections::N
        } else {
            IsoDirections::S
        };
        if n && w || n {
            ns
        } else if t.x >= r.x as f32 {
            IsoDirections::W
        } else {
            IsoDirections::E
        }
    }

    fn thump_window(&mut self, r: ObjectRef, t: &Thumper, out: &mut TargetOut) {
        if t.zombie {
            let Some((sq_owner, o_owner)) = self.find(r) else {
                return;
            };
            let sq = sq_owner.borrow();
            let o = o_owner.borrow();
            let ObjectKind::Window(w) = &o.kind else {
                return;
            };
            let climb = self.window_can_climb_through(&sq, &o, Some(t));
            let invincible = self.window_invincible(&sq, &o, w.north);
            let can_toggle =
                t.cognition == 1 && !climb && !invincible && !self.locked_out(t, w.locked);
            drop(o);
            drop(sq);
            if can_toggle {
                self.engine.toggle_window(self.cell, r, t.id);
                let Some((sq_owner, o_owner)) = self.find(r) else {
                    return;
                };
                let sq = sq_owner.borrow();
                let o = o_owner.borrow();
                if self.window_can_climb_through(&sq, &o, Some(t)) {
                    return;
                }
            }
            self.window_damage(r, t.strength, t);
            self.sound(Some(t.id), r, 20, 20, 15.0);
        }
        let Some(w_owner) = self.object_ref(r) else {
            return;
        };
        let Ok(w) = std::cell::Ref::filter_map(w_owner.borrow(), |o| match &o.kind {
            ObjectKind::Window(value) => Some(value),
            _ => None,
        }) else {
            return;
        };
        let should_smash = !w.destroyed && w.health <= 0;
        let smashed = w.smashed_sprite;
        drop(w);
        if should_smash {
            self.force_awake_at(r);
            self.engine.play_sound(t.id, "SmashWindow", Some(r));
            self.sound(None, r, 10, 20, 15.0);
            *out = Some(None);
            if let Some(o_owner) = self.object_ref(r) {
                let mut o = o_owner.borrow_mut();
                if let ObjectKind::Window(w) = &mut o.kind {
                    w.destroyed = true;
                }
                o.sprite = smashed;
            }
            self.engine.square_changed((r.x, r.y, r.z));
            self.add_broken_glass(r, t);
            if t.zombie {
                if let Some(tf) = self.thumpable_for(r, Some(t)) {
                    *out = Some(Some(ThumpTarget::Object(tf)));
                }
            }
        }
    }

    /// `IsoWindow.damage(amount, chr)`.
    pub(crate) fn window_damage(&mut self, r: ObjectRef, amount: i32, chr: &Thumper) {
        let Some((sq_owner, o_owner)) = self.find(r) else {
            return;
        };
        let sq = sq_owner.borrow();
        let o = o_owner.borrow();
        let ObjectKind::Window(w) = &o.kind else {
            return;
        };
        if self.window_invincible(&sq, &o, w.north) || self.settings.tutorial {
            return;
        }
        drop(o);
        drop(sq);
        let Some(w_owner) = self.object_ref(r) else {
            return;
        };
        let Ok(mut w) =
            std::cell::RefMut::filter_map(w_owner.borrow_mut(), |o| match &mut o.kind {
                ObjectKind::Window(value) => Some(value),
                _ => None,
            })
        else {
            return;
        };
        w.health = (w.health - amount).max(0);
        let should_smash = !w.destroyed && w.health == 0;
        drop(w);
        if should_smash {
            let alarm = !chr.zombie || self.settings.trigger_house_alarm;
            self.smash_window(r, alarm);
            self.add_broken_glass(r, chr);
        }
    }

    /// `IsoWindow.smashWindow(false, doAlarm)`.
    pub(crate) fn smash_window(&mut self, r: ObjectRef, do_alarm: bool) {
        let Some((sq_owner, o_owner)) = self.find(r) else {
            return;
        };
        let sq = sq_owner.borrow();
        let o = o_owner.borrow();
        let ObjectKind::Window(w) = &o.kind else {
            return;
        };
        if w.destroyed {
            return;
        }
        let (north, smashed) = (w.north, w.smashed_sprite);
        // `getIndoorSquare()`'s building for `handleAlarm`.
        let indoor = if sq.room_id != -1 {
            Some(sq_owner.clone())
        } else {
            self.opposite(&sq, north)
                .filter(|s| s.borrow().room_id != -1)
        };
        let alarm_building = indoor
            .and_then(|s| self.square_building_idx(&s.borrow()))
            .filter(|b| {
                self.meta
                    .building_defs
                    .get(b.0 as usize)
                    .is_some_and(|d| d.alarmed)
            });
        let sqp = (r.x, r.y, r.z);
        let attached_to_glass: Vec<bool> = o
            .attached_anim_sprite
            .iter()
            .flatten()
            .map(|i| {
                self.sprites.sprites[i.sprite]
                    .properties
                    .has_name(&self.sprites.alias, "AttachedToGlass")
            })
            .collect();
        drop(o);
        drop(sq);
        self.engine.play_world_sound("SmashWindow", sqp);
        self.sound(None, r, 10, 20, 15.0);
        if let Some(o_owner) = self.object_ref(r) {
            let mut o = o_owner.borrow_mut();
            if let ObjectKind::Window(w) = &mut o.kind {
                w.destroyed = true;
            }
            o.sprite = smashed;
            if let Some(list) = o.attached_anim_sprite.as_mut() {
                let mut k = 0;
                list.retain(|_| {
                    k += 1;
                    !attached_to_glass[k - 1]
                });
            }
        }
        if do_alarm {
            if let Some(b) = alarm_building {
                self.engine.do_alarm(b);
            }
        }
        self.engine.square_changed(sqp);
    }

    /// `IsoWindow.addBrokenGlass(chr)`.
    pub(crate) fn add_broken_glass(&mut self, r: ObjectRef, chr: &Thumper) {
        let Some((sq_owner, o_owner)) = self.find(r) else {
            return;
        };
        let sq = sq_owner.borrow();
        let o = o_owner.borrow();
        let ObjectKind::Window(w) = &o.kind else {
            return;
        };
        let opposite = if w.north {
            chr.y >= sq.y as f32
        } else {
            chr.x >= sq.x as f32
        };
        let target = if opposite {
            self.opposite(&sq, w.north)
        } else {
            Some(sq_owner.clone())
        };
        let Some(t) = target.map(|s| {
            let s = s.borrow();
            (s.x, s.y, s.z)
        }) else {
            return;
        };
        drop(o);
        drop(sq);
        self.square_add_broken_glass(t);
    }

    /// `IsoGridSquare.addBrokenGlass()`.
    pub fn square_add_broken_glass(&mut self, p: (i32, i32, i32)) {
        let Some(sq) = self.at(p) else { return };
        if !self.env().is_free(&sq.borrow()) {
            return;
        }
        let has = sq
            .borrow()
            .get_special_objects()
            .any(|o| matches!(o.borrow().kind, ObjectKind::BrokenGlass));
        if has {
            return;
        }
        let n = self.engine.rand().next(4);
        let sprite = self
            .sprites
            .named_map
            .get(&format!("brokenglass_1_{n}"))
            .copied();
        let sprites = &*self.sprites;
        let Some(sq_owner) = self.cell.get_grid_square_mut(p.0, p.1, p.2) else {
            return;
        };
        let mut sq = sq_owner.borrow_mut();
        let id = sq.new_object_id();
        let obj = IsoObject::new(id, ObjectKind::BrokenGlass, sprite);
        let index = sq.place_wall_and_door_check(sprites, &obj, -1);
        if index >= 0 && (index as usize) <= sq.objects.len() {
            sq.objects.insert(index as usize, ObjectSlot::new(obj));
        } else {
            sq.objects.push(ObjectSlot::new(obj));
        }
        sq.special_objects.push(id);
        sq.burnt_out = false;
        drop(sq);
        self.cell
            .recalc_all_with_neighbours(p.0, p.1, p.2, self.meta, sprites);
        self.engine.square_changed(p);
    }

    fn thump_barricade(&mut self, r: ObjectRef, t: &Thumper) {
        let Some(b_owner) = self.object_ref(r) else {
            return;
        };
        let Ok(b) = std::cell::Ref::filter_map(b_owner.borrow(), |o| match &o.kind {
            ObjectKind::Barricade(value) => Some(value),
            _ => None,
        }) else {
            return;
        };
        if b.is_destroyed() || !t.zombie {
            return;
        }
        let planks = b.num_planks();
        drop(b);
        self.barricade_damage(r, t.strength as f32);
        let Some(b_owner) = self.object_ref(r) else {
            return;
        };
        let Ok(b) = std::cell::Ref::filter_map(b_owner.borrow(), |o| match &o.kind {
            ObjectKind::Barricade(value) => Some(value),
            _ => None,
        }) else {
            return;
        };
        let (now, destroyed) = (b.num_planks(), b.is_destroyed());
        drop(b);
        if planks != now {
            self.engine.play_sound(t.id, "BreakBarricadePlank", None);
        }
        if destroyed {
            self.force_awake_at(r);
            self.remove_object(r, false);
        }
        self.sound(Some(t.id), r, 20, 20, 15.0);
    }

    /// `IsoBarricade.Damage(amount)`.
    pub fn barricade_damage(&mut self, r: ObjectRef, amount: f32) {
        if self.settings.tutorial {
            return;
        }
        let Some(o_owner) = self.object_ref(r) else {
            return;
        };
        let mut o = o_owner.borrow_mut();
        let ObjectKind::Barricade(b) = &mut o.kind else {
            return;
        };
        let mut changed = false;
        if b.metal_health > 0 {
            b.metal_health = (b.metal_health as f32 - amount) as i32;
            if b.metal_health <= 0 {
                b.metal_health = 0;
                changed = true;
            }
        } else if b.metal_bar_health > 0 {
            b.metal_bar_health = (b.metal_bar_health as f32 - amount) as i32;
            if b.metal_bar_health <= 0 {
                b.metal_bar_health = 0;
                changed = true;
            }
        } else {
            for i in (0..4).rev() {
                if b.plank_health[i] <= 0 {
                    continue;
                }
                b.plank_health[i] = (b.plank_health[i] as f32 - amount) as i32;
                if b.plank_health[i] <= 0 {
                    b.plank_health[i] = 0;
                    changed = true;
                }
                break;
            }
        }
        drop(o);
        if changed {
            self.barricade_choose_sprite(r);
        }
    }

    /// `IsoBarricade.chooseSprite()`.
    fn barricade_choose_sprite(&mut self, r: ObjectRef) {
        let Some(b_owner) = self.object_ref(r) else {
            return;
        };
        let Ok(b) = std::cell::Ref::filter_map(b_owner.borrow(), |o| match &o.kind {
            ObjectKind::Barricade(value) => Some(value),
            _ => None,
        }) else {
            return;
        };
        let Some(name) = b.sprite_name() else {
            return;
        };
        let s = self.sprites.named_map.get(&name).copied();
        drop(b);
        if let Some(o_owner) = self.object_ref(r) {
            let mut o = o_owner.borrow_mut();
            o.sprite = s;
        }
    }

    /// `IsoObject.Thump` without the bent-fence branch.
    fn thump_plain(&mut self, r: ObjectRef, t: &Thumper, out: &mut TargetOut) {
        let Some((_, o_owner)) = self.find(r) else {
            return;
        };
        let o = o_owner.borrow();
        let breakable = self.breakable(&o);
        drop(o);
        if t.zombie {
            let total = self.engine.surrounding_thumpers(t);
            let amount = if total >= 8 {
                1
            } else {
                let p = self.runtime.partial_thump_dmg.entry(r).or_insert(0.0);
                *p += total as f32 / 8.0;
                let a = *p as i32;
                if a > 0 {
                    *p -= a as f32;
                }
                a.max(0)
            };
            if amount > 0 {
                *self.runtime.object_damage.entry(r).or_insert(100) -= amount;
            }
            self.sound(Some(t.id), r, 20, 20, 15.0);
        }
        if self.object_damage(r) <= 0 {
            self.sound(None, r, 10, 20, 15.0);
            *out = Some(None);
            if breakable {
                let dir = self.break_dir(r, t);
                self.engine.destroy_fence(self.cell, r, dir);
                return;
            }
            self.engine.play_sound(t.id, "BreakObject", Some(r));
            if let Some(o) = self.object_ref(r) {
                self.engine.dump_contents((r.x, r.y, r.z), &o);
            }
            self.remove_object(r, false);
        }
    }

    // ---- destruction ----

    /// `square.transmitRemoveItemFromSquare(obj)` offline
    /// (`RemoveTileObject`): out of `objects` and `specialObjects`, then
    /// `RecalcAllWithNeighbours(true)`.
    pub fn remove_object(&mut self, r: ObjectRef, destroyed: bool) -> Option<ObjectSlot> {
        let sq_owner = self.cell.get_grid_square_mut(r.x, r.y, r.z)?;
        let mut sq = sq_owner.borrow_mut();
        let i = sq.object_index(r.id)?;
        let obj = sq.remove_object_slot(i);
        sq.special_objects.retain(|&id| id != r.id);
        drop(sq);
        self.runtime.forget(r);
        if !matches!(obj.borrow().kind, ObjectKind::WorldInventory(_)) {
            self.cell
                .recalc_all_with_neighbours(r.x, r.y, r.z, self.meta, self.sprites);
        }
        self.engine
            .object_removed((r.x, r.y, r.z), &obj.ref_handle(), destroyed);
        self.engine.square_changed((r.x, r.y, r.z));
        Some(obj)
    }

    /// `IsoDoor.destroy()`.
    fn destroy_door(&mut self, r: ObjectRef) {
        let Some((sq_owner, o_owner)) = self.find(r) else {
            return;
        };
        let sq = sq_owner.borrow();
        let o = o_owner.borrow();
        let ObjectKind::Door(d) = &o.kind else { return };
        let env = self.env();
        if env.obj_has_prop(&o, "GarageDoor") {
            drop(o);
            drop(sq);
            self.remove_object(r, true);
            return;
        }
        if env.props(&o).is_none() {
            return;
        }
        let mats: Vec<String> = ["Material", "Material2", "Material3"]
            .into_iter()
            .filter_map(|k| env.obj_prop(&o, k).map(str::to_string))
            .collect();
        let has_curtain = d.has_curtain;
        let key = self.check_key_id(&sq, &o, d.north, d.locked);
        drop(o);
        drop(sq);
        let p = (r.x, r.y, r.z);
        if mats.iter().any(|m| !m.is_empty()) {
            self.add_items_from_properties(p, &mats);
        } else {
            let n = self.engine.rand().next(2) + 1;
            for _ in 0..n {
                self.engine.add_world_item(p, "Base.Plank", 0.0, 0.0, None);
            }
        }
        self.engine
            .add_world_item(p, "Base.Doorknob", 0.0, 0.0, Some(key));
        let hinges = self.engine.rand().next(3);
        for _ in 0..hinges {
            self.engine.add_world_item(p, "Base.Hinge", 0.0, 0.0, None);
        }
        if has_curtain {
            self.engine.add_world_item(p, "Base.Sheet", 0.0, 0.0, None);
        }
        self.remove_object(r, true);
    }

    /// `IsoDoor.checkKeyId()` (without storing the found id).
    fn check_key_id(&self, sq: &IsoGridSquare, o: &IsoObject, north: bool, _locked: bool) -> i32 {
        if o.key_id != -1 {
            return o.key_id;
        }
        let Some(other) = self.opposite(sq, north) else {
            return -1;
        };
        let key = |s: &IsoGridSquare| {
            self.square_building_idx(s)
                .and_then(|b| self.meta.building_defs.get(b.0 as usize))
                .map(|d| d.key_id)
        };
        let other = other.borrow();
        match (key(sq), key(&other)) {
            (None, Some(k)) => k,
            (Some(k), None) => k,
            (Some(k), Some(_)) if self.env().obj_has_prop(&o, "forceLocked") => k,
            _ => o.key_id,
        }
    }

    /// `IsoObject.addItemsFromProperties()`.
    fn add_items_from_properties(&mut self, p: (i32, i32, i32), mats: &[String]) {
        let has = |m: &str| mats.iter().any(|x| x == m);
        let item = |s: &mut Self, name: &str| {
            let x = s.engine.rand().next_float_range(0.0, 0.5);
            let y = s.engine.rand().next_float_range(0.0, 0.5);
            s.engine.add_world_item(p, name, x, y, None);
        };
        if has("Wood") {
            item(self, "Base.UnusableWood");
            if self.engine.rand().next(5) == 0 {
                item(self, "Base.UnusableWood");
            }
        }
        for (m, n, it) in [
            ("MetalBars", 2, "Base.MetalBar"),
            ("MetalPlates", 2, "Base.SheetMetal"),
            ("MetalPipe", 2, "Base.MetalPipe"),
            ("MetalWire", 3, "Base.Wire"),
            ("Nails", 2, "Base.Nails"),
            ("Screws", 2, "Base.Screws"),
        ] {
            if has(m) && self.engine.rand().next(n) == 0 {
                item(self, it);
            }
        }
    }

    /// `IsoThumpable.destroy()`.
    pub(crate) fn destroy_thumpable(&mut self, r: ObjectRef) {
        let Some((_, o_owner)) = self.find(r) else {
            return;
        };
        let o = o_owner.borrow();
        drop(o);
        let o = self.object_ref(r).expect("located object");
        self.engine.dump_contents((r.x, r.y, r.z), &o);
        self.with_object_mut(r, |o| {
            if let ObjectKind::Thumpable(t) = &mut o.kind {
                t.health = 0;
            }
        });
        self.remove_object(r, true);
    }

    fn destroy_any(&mut self, r: ObjectRef) {
        match self.find(r).map(|(_, o)| Self::kind_of(&o.borrow())) {
            Some(ThumpKind::Door) => self.destroy_door(r),
            Some(ThumpKind::Thumpable) => self.destroy_thumpable(r),
            _ => {}
        }
    }

    /// `IsoDoor.getDoubleDoorIndex(obj)` / `getGarageDoorIndex(obj)`.
    fn multi_door_index(&self, r: ObjectRef, prop: &str, max: i32, open_shift: i32) -> i32 {
        let Some((_, o_owner)) = self.find(r) else {
            return -1;
        };
        let o = o_owner.borrow();
        let Some(v) = self.env().obj_prop(&o, prop) else {
            return -1;
        };
        let Some(i) = parse_int(v) else { return -1 };
        if !(1..=max).contains(&i) {
            return -1;
        }
        let Some((_, open)) = door_like(&o) else {
            return -1;
        };
        if open {
            if i > open_shift {
                i - open_shift
            } else {
                -1
            }
        } else {
            i
        }
    }

    fn double_door_index(&self, r: ObjectRef) -> i32 {
        self.multi_door_index(r, "DoubleDoor", 8, 4)
    }

    fn garage_door_index(&self, r: ObjectRef) -> i32 {
        self.multi_door_index(r, "GarageDoor", 6, 3)
    }

    /// The object of the same class (`IsoDoor` / `IsoThumpable`) facing
    /// `north` on `p` whose index passes `pick`.
    fn same_class_at(
        &self,
        r: ObjectRef,
        p: (i32, i32, i32),
        north: bool,
        pick: &dyn Fn(&Self, ObjectRef) -> bool,
    ) -> Option<ObjectRef> {
        let (_, me_owner) = self.find(r)?;
        let me = me_owner.borrow();
        let door = matches!(me.kind, ObjectKind::Door(_));
        let sq = self.at(p)?;
        let sq = sq.borrow();
        let found = sq.get_special_objects().find_map(|o| {
            let o = o.borrow();
            let same = match &o.kind {
                ObjectKind::Door(d) => door && d.north == north,
                ObjectKind::Thumpable(t) => !door && t.north == north,
                _ => false,
            };
            let c = oref(&sq, &o);
            (same && pick(self, c)).then_some(c)
        });
        found
    }

    /// `IsoDoor.getDoubleDoorObject(obj, index)`.
    fn double_door_object(&self, r: ObjectRef, index: i32) -> Option<ObjectRef> {
        let me = self.double_door_index(r);
        if me == -1 || index <= 0 {
            return None;
        }
        let (north, open) = door_like(&self.find(r)?.1.borrow())?;
        let (xo, yo) = match (north, open) {
            (true, true) => (DD_NORTH_OPEN_X, DD_NORTH_OPEN_Y),
            (true, false) => (DD_NORTH_CLOSED_X, DD_NORTH_CLOSED_Y),
            (false, true) => (DD_WEST_OPEN_X, DD_WEST_OPEN_Y),
            (false, false) => (DD_WEST_CLOSED_X, DD_WEST_CLOSED_Y),
        };
        let (fx, fy) = (r.x - xo[me as usize - 1], r.y - yo[me as usize - 1]);
        let p = (
            fx + xo[index as usize - 1],
            fy + yo[index as usize - 1],
            r.z,
        );
        self.same_class_at(r, p, north, &|s, c| s.double_door_index(c) == index)
    }

    /// `IsoDoor.destroyDoubleDoor(obj)`.
    fn destroy_double_door(&mut self, r: ObjectRef) -> bool {
        let i = self.double_door_index(r);
        if i == -1 {
            return false;
        }
        if i == 1 || i == 4 {
            if let Some(a) = self.double_door_object(r, if i == 1 { 2 } else { 3 }) {
                self.destroy_any(a);
            }
        }
        self.destroy_any(r);
        self.engine.on_container_update();
        true
    }

    /// `IsoDoor.getGarageDoorPrev(obj)` / `getGarageDoorNext(obj)`.
    fn garage_neighbour(&self, r: ObjectRef, next: bool) -> Option<ObjectRef> {
        let me = self.garage_door_index(r);
        if me == -1 || (!next && me == 1) || (next && me == 3) {
            return None;
        }
        let (north, _) = door_like(&self.find(r)?.1.borrow())?;
        let p = if next {
            (r.x + north as i32, r.y - (!north) as i32, r.z)
        } else {
            (r.x - north as i32, r.y + (!north) as i32, r.z)
        };
        self.same_class_at(r, p, north, &|s, c| {
            let j = s.garage_door_index(c);
            if next {
                j >= me
            } else {
                j <= me
            }
        })
    }

    /// `IsoDoor.destroyGarageDoor(obj)`.
    fn destroy_garage_door(&mut self, r: ObjectRef) -> bool {
        if self.garage_door_index(r) == -1 {
            return false;
        }
        for next in [false, true] {
            let mut cur = self.garage_neighbour(r, next);
            while let Some(c) = cur {
                let after = self.garage_neighbour(c, next);
                self.destroy_any(c);
                cur = after;
            }
        }
        self.destroy_any(r);
        self.engine.on_container_update();
        true
    }

    // ---- sheet ropes ----

    /// `IsoZombie.damageSheetRope`'s effect: `sheetRope.sheetRopeHealth -=
    /// damage`, then `damageSpriteSheetRopeFromBottom()`.
    pub fn damage_sheet_rope_at(&mut self, p: (i32, i32, i32), damage: f32) {
        let Some(sq_owner) = self.cell.get_grid_square_mut(p.0, p.1, p.2) else {
            return;
        };
        let sq = sq_owner.borrow();
        let Some(o) = sq.objects.iter().find(|o| o.borrow().sheet_rope) else {
            return;
        };
        o.borrow_mut().sheet_rope_health -= damage;
        drop(sq);
        self.damage_sprite_sheet_rope_from_bottom(p);
    }

    fn rope_flag(
        &self,
        sq: &IsoGridSquare,
        top: bool,
    ) -> Option<(&'static str, &'static str, bool)> {
        let env = self.env();
        let north = env.sq_has(sq, "climbSheetN") || env.sq_has(sq, "climbSheetS");
        let pick = |a: &'static str, ta: &'static str| if top { (ta, a) } else { (a, a) };
        let (t1, t2) = if north {
            if env.sq_has(sq, "climbSheetN") {
                pick("climbSheetN", "climbSheetTopN")
            } else if env.sq_has(sq, "climbSheetS") {
                pick("climbSheetS", "climbSheetTopS")
            } else {
                return None;
            }
        } else if env.sq_has(sq, "climbSheetW") {
            pick("climbSheetW", "climbSheetTopW")
        } else if env.sq_has(sq, "climbSheetE") {
            pick("climbSheetE", "climbSheetTopE")
        } else {
            return None;
        };
        Some((t1, t2, north))
    }

    /// `IsoGridSquare.damageSpriteSheetRopeFromBottom()`.
    fn damage_sprite_sheet_rope_from_bottom(&mut self, p: (i32, i32, i32)) {
        let Some(square) = self.at(p) else { return };
        let (health, rope) = {
            let sq = square.borrow();
            let Some(i) = self.env().sheet_rope_index(&sq) else {
                return;
            };
            let health = sq.objects[i].borrow().sheet_rope_health;
            (health, self.rope_flag(&sq, false))
        };
        if health >= 40.0 {
            return;
        }
        if health <= 0.0 {
            self.remove_sheet_rope_from_bottom(p);
            return;
        }
        let Some((_, ty, _)) = rope else { return };
        let mut z = p.2;
        while let Some(square) = self.at((p.0, p.1, z)) {
            let hit = {
                let sq = square.borrow();
                let env = self.env();
                sq.objects
                    .iter()
                    .find(|o| env.obj_has_flag(&o.borrow(), ty))
                    .map(|o| {
                        (
                            o.ref_handle(),
                            env.sprite_name(&o.borrow()).unwrap_or("").to_string(),
                        )
                    })
            };
            if let Some((object, name)) = hit {
                let parts: Vec<&str> = name.split('_').collect();
                let index = parts.get(2).and_then(|s| parse_int(s)).unwrap_or(0);
                if index > 14 {
                    return;
                }
                let new = format!(
                    "{}_{}_{}",
                    parts.first().unwrap_or(&""),
                    parts.get(1).unwrap_or(&""),
                    index + 40
                );
                object.borrow_mut().sprite = self.sprites.named_map.get(&new).copied();
            }
            if z == 7 {
                break;
            }
            z += 1;
        }
        if let Some(square) = self.at(p) {
            square.borrow_mut().recalc_properties(self.sprites);
        }
    }

    /// `IsoGridSquare.removeSheetRopeFromBottom()`.
    fn remove_sheet_rope_from_bottom(&mut self, p: (i32, i32, i32)) {
        let Some(square) = self.at(p) else { return };
        let Some((t1, t2, north)) = self.rope_flag(&square.borrow(), true) else {
            return;
        };
        let tile = match (t2, north) {
            ("climbSheetS", true) => Some("crafted_01_4"),
            ("climbSheetE", false) => Some("crafted_01_3"),
            _ => None,
        };
        if let Some(tile) = tile {
            let receiver = {
                let sq = square.borrow();
                let env = self.env();
                sq.objects
                    .iter()
                    .find(|o| env.sprite_name(&o.borrow()) == Some(tile))
                    .map(|o| oref(&sq, &o.borrow()))
            };
            if let Some(r) = receiver {
                self.remove_object(r, false);
            }
        }
        let mut find = false;
        let mut previous = None;
        let mut z = p.2;
        while let Some(square) = self.at((p.0, p.1, z)) {
            let hit = {
                let sq = square.borrow();
                let env = self.env();
                sq.objects
                    .iter()
                    .find(|o| {
                        let o = o.borrow();
                        env.obj_has_flag(&o, t1) || env.obj_has_flag(&o, t2)
                    })
                    .map(|o| oref(&sq, &o.borrow()))
            };
            if let Some(r) = hit {
                previous = Some(z);
                find = true;
                self.remove_object(r, false);
            }
            if z == 7 {
                break;
            }
            z += 1;
            find = false;
        }
        if find {
            return;
        }
        let Some(pz) = previous else { return };
        let top = if north {
            (p.0, p.1 + 1, pz)
        } else {
            (p.0 + 1, p.1, pz)
        };
        let Some(square) = self.at(top) else { return };
        let receiver = {
            let sq = square.borrow();
            let env = self.env();
            sq.objects
                .iter()
                .find(|o| {
                    let o = o.borrow();
                    env.obj_has_flag(&o, t1) || env.obj_has_flag(&o, t2)
                })
                .map(|o| oref(&sq, &o.borrow()))
        };
        if let Some(r) = receiver {
            self.remove_object(r, false);
        }
    }
}
