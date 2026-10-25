//! The `IsoAnimal` methods `BaseAnimalBehavior`, the animal states and
//! `updateInternal` call: path finding (`pathToLocation`,
//! `pathToCharacter`, `pathToTrough`, `pathFailed`, `canGoThere`), stopping
//! (`stopAllMovementNow`), luring, the tree, the mother, the hook, sounds
//! (`respondToSound`), thumping (`tryThump`), vehicle collisions, death
//! (`killed`, `HitByAnimal`), `alertOtherAnimals`, `climbOverFence` and
//! the queries (`isAnimalMoving`, `isAnimalEating`, ...).
//!
//! Other characters (the mother, the leash holder, the lure, opponents)
//! are reached through [`StateWorld::character_mut`]; when a call needs
//! the other character and its own world at once it is taken out with
//! [`StateWorld::take_character`] and put back.
//!
//! [`StateWorld::character_mut`]: crate::character::state::StateWorld::character_mut
//! [`StateWorld::take_character`]: crate::character::state::StateWorld::take_character

use std::rc::Rc;

use pz_animals::{
    character_ref, character_ref_id, square_ref, square_ref_xyz, Animal, AnimalDefinitions,
    AnimalEnv, BehaviorObject,
};
use pz_world::square::IsoDirections;

use super::{AnimalFields, CtxClock};
use crate::character::state::{StateCx, ThumpKind, ThumpTarget};
use crate::character::states::{animal as states, ids};
use crate::character::zombie::math::{
    distance_manhatten, distance_to, fastfloor, length, normalize, set_length,
};
use crate::character::GameCharacter;

/// `IsoGridSquare.DistToProper(IsoGridSquare)`: centre to centre.
pub(crate) fn sq_dist_proper(a: (i32, i32, i32), b: (i32, i32, i32)) -> f32 {
    distance_to(
        a.0 as f32 + 0.5,
        a.1 as f32 + 0.5,
        b.0 as f32 + 0.5,
        b.1 as f32 + 0.5,
    )
}

/// `IsoGridSquare.DistToProper(int x, int y)` (Manhattan, as the
/// original).
pub(crate) fn sq_dist_proper_xy(sq: (i32, i32, i32), x: i32, y: i32) -> f32 {
    distance_manhatten(
        x as f32 + 0.5,
        y as f32 + 0.5,
        sq.0 as f32 + 0.5,
        sq.1 as f32 + 0.5,
    )
}

/// `IsoGridSquare.DistToProper(IsoMovingObject)`.
pub(crate) fn sq_dist_proper_to(sq: (i32, i32, i32), x: f32, y: f32) -> f32 {
    distance_to(sq.0 as f32 + 0.5, sq.1 as f32 + 0.5, x, y)
}

/// `IsoGridSquare.DistTo(IsoGridSquare)` (Manhattan).
pub(crate) fn sq_dist_to(a: (i32, i32, i32), b: (i32, i32, i32)) -> f32 {
    distance_manhatten(
        a.0 as f32 + 0.5,
        a.1 as f32 + 0.5,
        b.0 as f32 + 0.5,
        b.1 as f32 + 0.5,
    )
}

/// `IsoMovingObject.DistToProper(other)`: `sqrt(DistToSquared)`.
pub(crate) fn dist_proper(a: (f32, f32), b: (f32, f32)) -> f32 {
    let dx = (b.0 - a.0) as f64;
    let dy = (b.1 - a.1) as f64;
    ((dx * dx + dy * dy) as f32 as f64).sqrt() as f32
}

/// `IsoDirections.fromAngle(v)`.
pub(crate) fn dir_from_angle(x: f32, y: f32) -> IsoDirections {
    crate::character::state::vars::iso_direction_from_angle(x, y)
}

/// A snapshot of another character, as the animal code reads it.
#[derive(Clone, Debug)]
pub(crate) struct Other {
    pub id: i32,
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub square: Option<(i32, i32, i32)>,
    pub dead: bool,
    pub vehicle: bool,
    pub exists: bool,
    pub zombie: bool,
    pub animal: bool,
    /// `instanceof IsoPlayer` (players and animals).
    pub iso_player: bool,
    pub online_id: i16,
}

impl<I: 'static> GameCharacter<I> {
    /// The animal's fields (it is an animal).
    pub(crate) fn af(&self) -> &AnimalFields {
        self.animal.as_deref().expect("an animal")
    }

    pub(crate) fn af_mut(&mut self) -> &mut AnimalFields {
        self.animal.as_deref_mut().expect("an animal")
    }

    /// `adef`.
    pub(crate) fn adef(&self) -> Rc<AnimalDefinitions> {
        self.af()
            .def
            .as_ref()
            .map(|d| d.adef.clone())
            .unwrap_or_default()
    }

    /// `getOnlineID()` of any character.
    pub(crate) fn online_id_of(&self) -> i16 {
        if let Some(f) = self.animal.as_ref() {
            return f.online_id;
        }
        if self.is_zombie() {
            return self.zombie.online_id;
        }
        self.player.online_id
    }

    /// `isExistInTheWorld()` of any character: an animal's flag, else on a
    /// square.
    pub(crate) fn exists_in_the_world(&self) -> bool {
        match self.animal.as_ref() {
            Some(f) => f.exists_in_world,
            None => self.path.current_square.is_some(),
        }
    }

    pub(crate) fn animal_view(&self) -> Other {
        Other {
            id: self.id,
            x: self.vars.x,
            y: self.vars.y,
            z: self.vars.z,
            square: self.path.current_square,
            dead: self.is_dead(),
            vehicle: self.path.vehicle.is_some(),
            exists: self.exists_in_the_world(),
            zombie: self.is_zombie(),
            animal: self.is_animal(),
            iso_player: self.is_player() || self.is_animal(),
            online_id: self.online_id_of(),
        }
    }

    /// Another character's snapshot.
    pub(crate) fn other(&self, id: i32, cx: &mut StateCx<'_, I>) -> Option<Other> {
        if id == self.id {
            return Some(self.animal_view());
        }
        cx.world.character_mut(id).map(|c| c.animal_view())
    }

    /// Runs `f` on another character (or on this one when `id` is its
    /// own) taken out of the world, with the context.
    pub(crate) fn with_other<R>(
        &mut self,
        id: i32,
        cx: &mut StateCx<'_, I>,
        f: impl FnOnce(&mut GameCharacter<I>, &mut StateCx<'_, I>) -> R,
    ) -> Option<R> {
        if id == self.id {
            return Some(f(self, cx));
        }
        let mut o = cx.world.take_character(id)?;
        let r = f(&mut o, cx);
        cx.world.put_character(o);
        Some(r)
    }

    /// Runs `f` with the animal logic's environment (the world's
    /// `AnimalHost`), then does a `stopAllMovementNow()` the logic asked
    /// for. `None` without a host.
    pub(crate) fn animal_env<R>(
        &mut self,
        cx: &mut StateCx<'_, I>,
        f: impl FnOnce(&mut Animal, &mut Self, &mut AnimalEnv<'_>) -> R,
    ) -> Option<R> {
        let ctx = cx.ctx;
        let r = {
            let mut host = cx.world.animal_host(self.id)?;
            self.with_animal_env(ctx, &mut host, f)
        };
        if self
            .animal
            .as_ref()
            .is_some_and(|a| a.pending_stop_all_movement)
        {
            self.af_mut().pending_stop_all_movement = false;
            self.animal_stop_all_movement_now(cx);
        }
        r
    }

    /// `getX()` as `getXi()` etc.
    pub(crate) fn xi(&self) -> i32 {
        fastfloor(self.vars.x)
    }
    pub(crate) fn yi(&self) -> i32 {
        fastfloor(self.vars.y)
    }
    pub(crate) fn zi(&self) -> i32 {
        fastfloor(self.vars.z)
    }

    // -------------------------------------------------------------------
    // queries

    /// `isAnimalMoving()`: walking, path finding or following a wall.
    pub fn animal_is_moving(&self) -> bool {
        [
            ids::ANIMAL_WALK,
            ids::ANIMAL_PATH_FIND,
            ids::ANIMAL_FOLLOW_WALL,
        ]
        .iter()
        .any(|s| self.current_state_id() == Some(*s))
    }

    /// `isAnimalEating()`.
    pub fn animal_is_eating(&self) -> bool {
        self.variable_string("idleAction") == "eat"
    }

    /// `isAnimalSitting()`.
    pub fn animal_is_sitting(&self) -> bool {
        self.variable_string("idleAction") == "sit"
    }

    /// `isAnimalAttacking()`.
    pub fn animal_is_attacking(&self, cx: &StateCx<'_, I>) -> bool {
        let f = self.af();
        if cx.ctx.is_client() {
            return f.attacking_on_client;
        }
        f.atk_target.is_some() || self.adef().can_thump && f.thump_target.is_some()
    }

    /// `canDoAction()`.
    pub fn animal_can_do_action(&self, cx: &StateCx<'_, I>) -> bool {
        !self.animal_is_attacking(cx)
            && !self.animal_is_sitting()
            && !self.af().animal.behavior.block_movement
            && !self.animal_is_moving()
    }

    /// `hasHitReaction()`.
    pub(crate) fn has_hit_reaction(&self) -> bool {
        !self.vars.hit_reaction.is_empty()
    }

    /// `shouldStartFollowWall()`.
    pub fn animal_should_start_follow_wall(&self) -> bool {
        let f = self.af();
        f.animal.data.attached_player.is_some()
            || f.animal.stress_level > 20.0 && self.variable_boolean("animalRunning")
            || f.animal.wild
    }

    // -------------------------------------------------------------------
    // path finding

    /// The leash and the tree: a target further than the rope's 15 squares
    /// from the holder or the tree is not walked to.
    fn leash_allows(&self, x: f32, y: f32, holder: impl FnOnce(i32) -> Option<f32>) -> bool {
        let d = &self.af().animal.data;
        if let Some(p) = d.attached_player.and_then(character_ref_id) {
            if holder(p).is_some_and(|dist| dist > 15.0) {
                return false;
            }
        }
        if d.attached_tree.is_some() {
            let (tx, ty) = (
                d.attached_tree_x as f32 + 0.5,
                d.attached_tree_y as f32 + 0.5,
            );
            if distance_to(tx, ty, x, y) > 15.0 {
                return false;
            }
        }
        true
    }

    /// `IsoAnimal.pathToLocation(x, y, z)`.
    pub fn animal_path_to_location(&mut self, x: i32, y: i32, z: i32, cx: &mut StateCx<'_, I>) {
        let holder_sq = self
            .af()
            .animal
            .data
            .attached_player
            .and_then(character_ref_id)
            .and_then(|p| self.other(p, cx))
            .and_then(|o| o.square);
        let ok = self.leash_allows(x as f32 + 0.5, y as f32 + 0.5, |_| {
            holder_sq.map(|sq| sq_dist_proper_xy(sq, x, y))
        });
        if !ok {
            return;
        }
        cx.world.path_to_location(self, x, y, z);
        self.path_to_aux(x as f32, y as f32, z as f32, cx);
    }

    /// `IsoAnimal.pathToCharacter(target)`.
    pub fn animal_path_to_character(&mut self, target: i32, cx: &mut StateCx<'_, I>) {
        let Some(t) = self.other(target, cx) else {
            return;
        };
        let holder = self
            .af()
            .animal
            .data
            .attached_player
            .and_then(character_ref_id)
            .and_then(|p| self.other(p, cx));
        if !self.leash_allows(t.x, t.y, |_| {
            holder.map(|h| dist_proper((h.x, h.y), (t.x, t.y)))
        }) {
            return;
        }
        let mid = if target == self.id {
            self.mover_id_of()
        } else {
            match cx.world.character_mut(target) {
                Some(c) => c.mover_id_of(),
                None => return,
            }
        };
        cx.world.path_to_character(self, mid);
        self.path_to_aux(t.x, t.y, t.z, cx);
    }

    /// `pathToTrough(trough)`.
    pub fn animal_path_to_trough(
        &mut self,
        trough: pz_animals::ObjectRef,
        cx: &mut StateCx<'_, I>,
    ) {
        let Some(pos) = cx
            .world
            .animal_host(self.id)
            .and_then(|h| h.world.trough_position(trough))
        else {
            return;
        };
        let tsq = cx
            .world
            .animal_host(self.id)
            .and_then(|h| h.world.trough_square(trough))
            .and_then(square_ref_xyz);
        let (tx, ty, tz, north) = pos;
        // getGridSquare(double, double, double) floors.
        let sq = |cx: &StateCx<'_, I>, x: f32, y: f32| {
            let (x, y, z) = (fastfloor(x), fastfloor(y), fastfloor(tz));
            cx.world.square(x, y, z).map(|_| (x, y, z))
        };
        let (Some(mut s1), Some(mut s2), Some(mut s3), Some(mut s4)) = (
            sq(cx, tx, ty - 1.0),
            sq(cx, tx, ty + 1.0),
            sq(cx, tx - 1.0, ty),
            sq(cx, tx + 1.0, ty),
        ) else {
            return;
        };
        if !north {
            s3 = s1;
            s4 = s2;
            match (sq(cx, tx + 1.0, ty), sq(cx, tx - 1.0, ty)) {
                (Some(a), Some(b)) => {
                    s1 = a;
                    s2 = b;
                }
                // getGridSquare can't be null here: s3/s4 were these squares.
                _ => return,
            }
        }
        let free = |cx: &StateCx<'_, I>, s: (i32, i32, i32)| {
            cx.world.square_is_free_ignoring_characters(s)
                && !tsq.is_some_and(|t| cx.world.is_wall_to(s, t))
                && !tsq.is_some_and(|t| cx.world.is_window_to(s, t))
        };
        let (f1, f2, f3, f4) = (free(cx, s1), free(cx, s2), free(cx, s3), free(cx, s4));
        let (x, y) = (self.vars.x, self.vars.y);
        let d = |s: (i32, i32, i32)| sq_dist_proper_to(s, x, y);
        let mut chosen = None;
        if f1 && (d(s1) < d(s2) || !f2) {
            chosen = Some(s1);
        }
        if f2 && (d(s2) < d(s1) || !f1) {
            chosen = Some(s2);
        }
        if chosen.is_none() {
            if f3 && (d(s3) < d(s4) || !f4) {
                chosen = Some(s3);
            }
            if f4 && (d(s4) < d(s3) || !f3) {
                chosen = Some(s4);
            }
        }
        let Some(c) = chosen else {
            return;
        };
        if !cx.world.square_is_free_ignoring_characters(c) {
            let f = &mut self.af_mut().animal;
            // The original adds the trough again only when it is already
            // ignored.
            if f.ignored_trough.contains(&trough) {
                f.ignored_trough.push(trough);
            }
            return;
        }
        if self.adef().dist_to_eat < 1.0 {
            self.animal_path_to_location(fastfloor(tx), fastfloor(ty), fastfloor(tz), cx);
        } else {
            self.animal_path_to_location(c.0, c.1, c.2, cx);
        }
    }

    /// `pathFailed()`.
    pub fn animal_path_failed(&mut self, cx: &mut StateCx<'_, I>) {
        let f = &mut self.af_mut().animal;
        if let Some(t) = f.eat_from_trough {
            f.ignored_trough.push(t);
        }
        if let Some(t) = f.drink_from_trough {
            f.ignored_trough.push(t);
        }
        if let Some(BehaviorObject::Trough(t)) = f.behavior.behavior_object {
            f.ignored_trough.push(t);
        }
        self.animal_stop_all_movement_now(cx);
    }

    /// `canGoThere(sq)`.
    pub fn animal_can_go_there(&mut self, sq: (i32, i32, i32), cx: &mut StateCx<'_, I>) -> bool {
        let Some(cur) = self.path.current_square else {
            return false;
        };
        if cx.world.is_blocked_to(sq, cur) || cx.world.is_window_to(sq, cur) {
            return false;
        }
        if let Some(p) = self
            .af()
            .animal
            .data
            .attached_player
            .and_then(character_ref_id)
        {
            if let Some(psq) = self.other(p, cx).and_then(|o| o.square) {
                if sq_dist_to(sq, psq) > 15.0 {
                    return false;
                }
            }
        }
        let d = &self.af().animal.data;
        d.attached_tree.is_none()
            || distance_to(
                d.attached_tree_x as f32 + 0.5,
                d.attached_tree_y as f32 + 0.5,
                sq.0 as f32 + 0.5,
                sq.1 as f32 + 0.5,
            ) <= 15.0
    }

    /// `stopAllMovementNow()`.
    pub fn animal_stop_all_movement_now(&mut self, cx: &mut StateCx<'_, I>) {
        self.vars.moving = false;
        cx.world.path_reset(self);
        self.path.path2 = None;
        let d = self.default_state();
        self.change_state(d, cx);
        self.with_animal(|a, c| a.data_reset_eating_check(c));
        if let Some(f) = self.animal.as_mut() {
            f.pending_stop_all_movement = false;
        }
    }

    // -------------------------------------------------------------------
    // luring

    /// `cancelLuring()`.
    pub fn animal_cancel_luring(&mut self, cx: &mut StateCx<'_, I>) {
        self.animal_stop_all_movement_now(cx);
        let id = self.id;
        if let Some(p) = self.af().lured_by {
            if let Some(pc) = cx.world.character_mut(p) {
                pc.player.lured_animals.retain(|a| *a != id);
            }
        }
        let f = self.af_mut();
        f.lured_by = None;
        f.walk_to_char_luring = false;
        f.lured_start_timer = -1.0;
    }

    /// `updateLured()`.
    pub fn animal_update_lured(&mut self, cx: &mut StateCx<'_, I>) {
        let Some(p) = self.af().lured_by else {
            return;
        };
        let luring = cx
            .world
            .character_mut(p)
            .is_some_and(|pc| pc.player.luring_animals);
        if !luring {
            self.animal_cancel_luring(cx);
            return;
        }
        if self.af().walk_to_char_luring {
            if !self.animal_is_moving() {
                let id = self.id;
                if let Some(pc) = cx.world.character_mut(p) {
                    if !pc.player.lured_animals.contains(&id) {
                        pc.player.lured_animals.push(id);
                    }
                }
                self.set_var_bool("animalRunning", false);
                self.animal_path_to_character(p, cx);
            }
            return;
        }
        let f = self.af_mut();
        if f.lured_start_timer > -1.0 {
            f.lured_start_timer -= 1.0;
            if f.lured_start_timer == 0.0 {
                f.walk_to_char_luring = true;
            }
        }
    }

    // -------------------------------------------------------------------
    // tree, mother, hook

    /// `getCanAttachAnimalObject(square)`.
    fn attach_object_at(
        &mut self,
        sq: (i32, i32, i32),
        cx: &mut StateCx<'_, I>,
    ) -> Option<pz_animals::ObjectRef> {
        cx.world
            .animal_host(self.id)
            .and_then(|h| h.world.attach_object_at(square_ref(sq.0, sq.1, sq.2)))
    }

    /// `checkTreeExists()`.
    pub fn animal_check_tree_exists(&mut self, cx: &mut StateCx<'_, I>) {
        let d = &self.af().animal.data;
        if d.attached_tree.is_none() {
            return;
        }
        let sq = (d.attached_tree_x, d.attached_tree_y, fastfloor(self.vars.z));
        if cx.world.square(sq.0, sq.1, sq.2).is_none() {
            return;
        }
        match self.attach_object_at(sq, cx) {
            Some(t) => self
                .af_mut()
                .animal
                .data_set_attached_tree(Some(t), Some((sq.0, sq.1))),
            None => {
                let a = &mut self.af_mut().animal;
                a.data_set_attached_tree(None, None);
                a.attach_back_to_tree_x = i32::MAX;
                a.attach_back_to_tree_y = i32::MAX;
            }
        }
    }

    /// `reattachToTree()`.
    pub fn animal_reattach_to_tree(&mut self, cx: &mut StateCx<'_, I>) {
        let a = &self.af().animal;
        if a.attach_back_to_tree_x == i32::MAX || a.attach_back_to_tree_y == i32::MAX {
            return;
        }
        let Some(cur) = self.path.current_square else {
            return;
        };
        let sq = (a.attach_back_to_tree_x, a.attach_back_to_tree_y, cur.2);
        if cx.world.square(sq.0, sq.1, sq.2).is_none() {
            return;
        }
        let Some(t) = self.attach_object_at(sq, cx) else {
            return;
        };
        let a = &mut self.af_mut().animal;
        a.data_set_attached_tree(Some(t), Some((sq.0, sq.1)));
        a.attach_back_to_tree_x = i32::MAX;
        a.attach_back_to_tree_y = i32::MAX;
    }

    /// `reattachBackToMom()`.
    pub fn animal_reattach_back_to_mom(&mut self, cx: &mut StateCx<'_, I>) {
        let want = self.af().animal.attach_back_to_mother;
        if want > 0 {
            if let Some(v) = self.path.vehicle {
                for a in cx.world.vehicle_animals(v) {
                    if cx
                        .world
                        .character_mut(a)
                        .and_then(|c| c.animal.as_ref())
                        .is_some_and(|f| f.animal.animal_id == want)
                    {
                        self.animal_set_mother(a, cx);
                        break;
                    }
                }
            }
        }
        if !self.af().exists_in_world {
            return;
        }
        let mother_in_world = self
            .af()
            .animal
            .mother
            .and_then(character_ref_id)
            .and_then(|m| self.other(m, cx))
            .is_some_and(|o| o.exists);
        let a = &mut self.af_mut().animal;
        if a.attach_back_to_mother == 0 && !mother_in_world && a.mother_id > 0 {
            a.attach_back_to_mother = a.mother_id;
        }
        if a.attach_back_to_mother > 0 && self.path.vehicle.is_none() {
            let m = cx.ctx.multiplier();
            let a = &mut self.af_mut().animal;
            if a.attach_back_to_mother_timer < 50.0 {
                a.attach_back_to_mother_timer += m;
                return;
            }
            a.attach_back_to_mother_timer = 0.0;
            let zones = a.connected_d_zone.clone();
            for z in zones {
                let animals = cx
                    .world
                    .animal_host(self.id)
                    .map(|h| h.world.zone_animals(z))
                    .unwrap_or_default();
                let ids: Vec<i32> = animals.into_iter().filter_map(character_ref_id).collect();
                if self.find_mother_and_attach(&ids, cx) {
                    return;
                }
            }
            let all = cx.world.cell_animals();
            self.find_mother_and_attach(&all, cx);
        }
    }

    /// `findMotherAndAttach(animals)`.
    fn find_mother_and_attach(&mut self, animals: &[i32], cx: &mut StateCx<'_, I>) -> bool {
        let want = self.af().animal.attach_back_to_mother;
        for &a in animals {
            let id = if a == self.id {
                Some(self.af().animal.animal_id)
            } else {
                cx.world
                    .character_mut(a)
                    .and_then(|c| c.animal.as_ref())
                    .map(|f| f.animal.animal_id)
            };
            if id != Some(want) {
                continue;
            }
            self.animal_set_mother(a, cx);
            let f = &mut self.af_mut().animal;
            f.attach_back_to_mother = 0;
            f.mother_id = want;
            return true;
        }
        false
    }

    /// `setMother(mom)`.
    pub fn animal_set_mother(&mut self, mom: i32, cx: &mut StateCx<'_, I>) {
        let (me, my_id) = (character_ref(self.id), self.af().animal.animal_id);
        self.af_mut().animal.mother = Some(character_ref(mom));
        let id = self.id;
        let mut other_ids = Vec::new();
        if let Some(m) = cx.world.character_mut(mom) {
            if let Some(mf) = m.animal.as_ref() {
                for b in mf.animal.babies.clone().unwrap_or_default() {
                    other_ids.push(b);
                }
            }
        }
        // removeBaby of the first entry that is this animal or has its id.
        let mut remove = None;
        for b in &other_ids {
            let same = *b == me
                || character_ref_id(*b).is_some_and(|bid| {
                    bid != id
                        && cx
                            .world
                            .character_mut(bid)
                            .and_then(|c| c.animal.as_ref())
                            .is_some_and(|f| f.animal.animal_id == my_id)
                });
            if same {
                remove = Some(*b);
                break;
            }
        }
        if let Some(m) = cx.world.character_mut(mom).and_then(|c| c.animal.as_mut()) {
            let babies = m.animal.babies.get_or_insert_with(Vec::new);
            if let Some(r) = remove {
                if let Some(i) = babies.iter().position(|b| *b == r) {
                    babies.remove(i);
                }
            }
            babies.push(me);
        }
    }

    /// `reattachBackToHook()`.
    pub fn animal_reattach_back_to_hook(&mut self, cx: &mut StateCx<'_, I>) {
        let a = &self.af().animal;
        if a.attach_back_to_hook_x == 0 && a.attach_back_to_hook_y == 0 {
            return;
        }
        if self.path.current_square.is_none() {
            return;
        }
        let sq = square_ref(
            a.attach_back_to_hook_x,
            a.attach_back_to_hook_y,
            a.attach_back_to_hook_z,
        );
        let done = cx
            .world
            .animal_host(self.id)
            .is_some_and(|h| h.world.reattach_to_hook(sq));
        if done {
            let a = &mut self.af_mut().animal;
            a.attach_back_to_hook_x = 0;
            a.attach_back_to_hook_y = 0;
            a.attach_back_to_hook_z = 0;
        }
    }

    // -------------------------------------------------------------------
    // sound

    /// `respondToSound()`.
    pub fn animal_respond_to_sound(&mut self, cx: &mut StateCx<'_, I>) {
        if self.af().last_sound_responded_to.is_some() {
            let m = cx.ctx.multiplier();
            let f = self.af_mut();
            f.time_since_flee_from_sound -= m;
            if f.time_since_flee_from_sound <= 0.0 || !self.animal_is_moving() {
                self.vars.moving = false;
                let src = self
                    .af()
                    .last_sound_responded_to
                    .as_ref()
                    .and_then(|s| s.source);
                if cx.ctx.is_server() {
                    if let Some(p) = src.and_then(|s| cx.world.sound_source_player(&s)) {
                        if let Some(pc) = cx.world.character_mut(p) {
                            pc.environment.call_out = false;
                        }
                    }
                }
                self.af_mut().last_sound_responded_to = None;
            } else {
                return;
            }
        }
        let wild = self.af().animal.wild;
        let listener = pz_audio::world_sound::Listener {
            x: self.vars.x,
            y: self.vars.y,
            z: self.vars.z,
            square: match self.path.current_square {
                None => pz_audio::world_sound::ListenerSquare::None,
                Some(_) if !self.environment.square_has_chunk => {
                    pz_audio::world_sound::ListenerSquare::NoChunk
                }
                Some((x, y, _)) => {
                    pz_audio::world_sound::ListenerSquare::Chunk(x.div_euclid(8), y.div_euclid(8))
                }
            },
            hearing: 2,
            hearing_scale: 1.0,
            sound_source_target: None,
            wild,
        };
        let Some((sound, attract)) = cx.world.world_sounds().map(|(mgr, sw)| {
            let id = mgr.get_sound_animal(sw, &listener);
            let attract = mgr.get_sound_attract_animal(sw, id, &listener);
            (id.map(|i| mgr.sound(i).clone()), attract)
        }) else {
            return;
        };
        let Some(sound) = sound else {
            return;
        };
        if attract <= 0.0 {
            return;
        }
        if sound
            .source
            .is_some_and(|s| cx.world.sound_source_is_radio_or_television(&s))
        {
            return;
        }
        if !wild && sound.source_is_vehicle() {
            return;
        }
        if let Some(p) = sound.source.and_then(|s| cx.world.sound_source_player(&s)) {
            let info = cx.world.character_mut(p).map(|pc| {
                (
                    pc.environment.call_out,
                    pc.online_id_of(),
                    pc.xi(),
                    pc.yi(),
                    pc.zi(),
                )
            });
            if let Some((call_out, oid, px, py, pz)) = info {
                if call_out {
                    let acc = {
                        let mut r = crate::character::fire::CtxRand(cx.ctx);
                        self.af_mut().animal.player_acceptance_of(oid, &mut r)
                    };
                    if acc >= 40.0 {
                        self.animal_path_to_location(px, py, pz, cx);
                        return;
                    }
                }
            }
        }
        let radius = sound.radius;
        let (sx, sy) = (sound.x, sound.y);
        {
            let f = self.af_mut();
            f.last_sound_responded_to = Some(sound);
            f.time_since_flee_from_sound = 2000.0 * radius as f32;
        }
        let (vx, vy) = (self.vars.x - sx as f32, self.vars.y - sy as f32);
        let mut flee = (radius + 10) as f32;
        if wild {
            flee = radius as f32 * 3.0 + 20.0;
            let f = self.af_mut();
            f.animal.alerted = false;
            f.animal.behavior.last_alerted = 1000.0;
        }
        let (gx, gy) = set_length(vx, vy, flee - length(vx, vy));
        let (go_x, go_y) = (fastfloor(gx), fastfloor(gy));
        self.af_mut().animal.change_stress(radius as f32 * 0.05);
        self.set_var_bool("animalRunning", true);
        let (x, y, z) = (self.xi() + go_x, self.yi() + go_y, self.zi());
        self.animal_path_to_location(x, y, z, cx);
    }

    // -------------------------------------------------------------------
    // thumping

    /// `animalShouldThump()`.
    pub fn animal_should_thump(&mut self, cx: &StateCx<'_, I>) -> bool {
        if !self.adef().can_thump {
            return false;
        }
        if self.vars.attacked_by.is_some() {
            return true;
        }
        let (h, t) = self.hunger_thirst();
        let m = cx.ctx.multiplier();
        let a = &mut self.af_mut().animal;
        if t >= 0.9 || h >= 0.9 {
            if a.thump_delay == 0.0 {
                return true;
            }
            a.thump_delay -= m;
            if a.thump_delay < 0.0 {
                a.thump_delay = 0.0;
            }
        } else {
            a.thump_delay = 20000.0;
        }
        a.stress_level >= 80.0
    }

    /// `getFeelerTile(dist)`'s coordinates.
    fn feeler_tile_at(&self, dist: f32) -> (i32, i32, i32) {
        let (vx, vy) = set_length(
            self.vars.next_x - self.vars.last_x,
            self.vars.next_y - self.vars.last_y,
            dist,
        );
        (
            fastfloor(self.vars.x + vx),
            fastfloor(self.vars.y + vy),
            fastfloor(self.vars.z),
        )
    }

    /// `tryThump(square)`.
    pub fn animal_try_thump(
        &mut self,
        square: Option<(i32, i32, i32)>,
        cx: &mut StateCx<'_, I>,
    ) -> bool {
        if self.animal_is_attacking(cx) || !self.animal_should_thump(cx) {
            return false;
        }
        if !self.animal_is_moving() {
            return false;
        }
        let feeler = match square {
            Some(s) => s,
            None => self.feeler_tile_at(0.8),
        };
        if cx.world.square(feeler.0, feeler.1, feeler.2).is_none() {
            return false;
        }
        let Some(cur) = self.path.current_square else {
            return false;
        };
        let obj = match cx.world.special_object_between(cur, feeler) {
            Some(t) => Some(t),
            None => cx
                .world
                .hoppable_between(cur, feeler)
                .map(ThumpTarget::Object),
        };
        let Some(ThumpTarget::Object(o)) = obj else {
            return false;
        };
        let kind = cx
            .world
            .thump_info(ThumpTarget::Object(o), self.id)
            .map(|i| i.kind);
        if matches!(kind, Some(ThumpKind::Thumpable) | Some(ThumpKind::Door)) {
            self.af_mut().thump_target = Some(o);
            self.path.path2 = None;
            return true;
        }
        false
    }

    // -------------------------------------------------------------------
    // vehicles, death, alerting

    /// `testCollideWithVehicles(vehicle, null)`.
    pub fn animal_test_collide_with_vehicles(
        &mut self,
        vehicle: i16,
        cx: &mut StateCx<'_, I>,
    ) -> bool {
        if self.is_dead() {
            return false;
        }
        cx.world.vehicle_test_collide_animal(vehicle, self)
    }

    /// `killed(chr)`: when dead in a designation zone, the zone's animals
    /// are stressed and those within 10 of the killer flee from it.
    pub fn animal_killed(
        &mut self,
        chr: Option<i32>,
        chr_pos: (f32, f32),
        cx: &mut StateCx<'_, I>,
    ) {
        if self.health > 0.0 {
            return;
        }
        let Some(z) = self.af().animal.d_zone else {
            return;
        };
        let animals: Vec<i32> = cx
            .world
            .animal_host(self.id)
            .map(|h| h.world.zone_animals_connected(z))
            .unwrap_or_default()
            .into_iter()
            .filter_map(character_ref_id)
            .collect();
        for a in animals {
            let s = cx.ctx.rand_next_float(10.0, 30.0);
            self.with_other(a, cx, |o, cx| {
                if let Some(f) = o.animal.as_mut() {
                    f.animal.change_stress(s);
                }
                if let Some(c) = chr {
                    if dist_proper((o.vars.x, o.vars.y), chr_pos) < 10.0 {
                        o.behavior_force_flee_from_chr(c, chr_pos, cx);
                    }
                }
            });
        }
    }

    /// `HitByAnimal(animal, ignoreDamage)`: `attacker_dmg` is the
    /// attacker's `adef.baseDmg` times its `strength` gene.
    pub fn animal_hit_by_animal(
        &mut self,
        attacker_dmg: f32,
        ignore_damage: bool,
        cx: &mut StateCx<'_, I>,
    ) {
        self.vars.hit_reaction = "default".to_string();
        if !ignore_damage {
            let ctx = cx.ctx;
            let loss = cx.world.animal_host(self.id).map(|h| {
                let clock = CtxClock(ctx);
                let sandbox = ctx.animal_sandbox();
                let actx = pz_animals::AnimalCtx {
                    defs: h.defs,
                    clock: &clock,
                    sandbox: &sandbox,
                };
                self.af().animal.data_health_loss(Some(0.04), actx)
            });
            if let Some(loss) = loss {
                self.animal_set_health(self.health - attacker_dmg * loss);
            }
            if let Some(sq) = self.path.current_square {
                if cx.world.square(sq.0, sq.1, sq.2).is_some() {
                    for _ in 0..10 {
                        let x = sq.0 as f32 + cx.ctx.rand_next_float(-0.8, 0.8);
                        let y = sq.1 as f32 + cx.ctx.rand_next_float(-0.8, 0.8);
                        let k = cx.ctx.rand_next(8);
                        if let Some(h) = cx.world.animal_host(self.id) {
                            h.world.add_blood_splat(
                                square_ref(sq.0, sq.1, sq.2),
                                x,
                                y,
                                sq.2 as f32,
                                k,
                            );
                        }
                    }
                }
            }
        }
        if self.is_dead() {
            if let Some(o) = self.af().fighting_opponent {
                if let Some(f) = cx.world.character_mut(o).and_then(|c| c.animal.as_mut()) {
                    f.fighting_opponent = None;
                }
            }
            self.af_mut().fighting_opponent = None;
        }
    }

    /// `calcDamage()` of this animal (0 without a definition).
    pub(crate) fn animal_damage(&self) -> f32 {
        self.animal_calc_damage().unwrap_or(0.0)
    }

    /// `alertOtherAnimals(chr, alert)`: the animals on the squares within
    /// five of this one are alerted (`canBeAlerted` and not alerted lately)
    /// or take this animal's spotted character.
    pub fn animal_alert_other_animals(&mut self, alert: bool, cx: &mut StateCx<'_, I>) {
        let (x0, y0, z) = (self.vars.x as i32, self.vars.y as i32, self.vars.z as i32);
        let (alerted, spotted) = (self.af().alerted_chr, self.af().spotted_chr);
        for x in x0 - 5..x0 + 5 {
            for y in y0 - 5..y0 + 5 {
                if cx.world.square(x, y, z).is_none() {
                    continue;
                }
                for a in cx.world.characters_on_square((x, y, z)) {
                    if a == self.id {
                        continue;
                    }
                    let Some(o) = cx.world.character_mut(a) else {
                        continue;
                    };
                    let can = o
                        .animal
                        .as_ref()
                        .and_then(|f| f.def.as_ref())
                        .is_some_and(|d| d.adef.can_be_alerted);
                    let Some(f) = o.animal.as_mut() else {
                        continue;
                    };
                    if can && f.animal.behavior.last_alerted <= 0.0 && alert {
                        f.animal.alerted = true;
                        f.alerted_chr = alerted;
                        continue;
                    }
                    f.spotted_chr = spotted;
                }
            }
        }
    }

    /// `IsoAnimal.climbOverFence(dir)`: the climb's parameters
    /// (`AnimalClimbOverFenceState.setParams`) and `ClimbFence`.
    pub fn animal_climb_over_fence(&mut self, dir: IsoDirections, cx: &mut StateCx<'_, I>) {
        if self.path.current_square.is_none() || self.variable_boolean("ClimbFence") {
            return;
        }
        states::set_climb_params(self, dir, cx);
        self.set_var_bool("ClimbFence", true);
    }

    /// Hunger and thirst.
    pub(crate) fn hunger_thirst(&self) -> (f32, f32) {
        let f = self.af();
        (self.stats.get(&f.hunger), self.stats.get(&f.thirst))
    }

    /// `faceThisObject(chr)` for a character (`faceThisObjectAlt`).
    pub(crate) fn face_character(&mut self, x: f32, y: f32) {
        let (dx, dy) = (x - self.vars.x, y - self.vars.y);
        if length(dx, dy) > 0.0 {
            self.set_forward_direction(dx, dy);
        }
    }

    /// `faceThisObject(obj)` for a world object at facing position `p`
    /// (`None`: not in the world).
    pub(crate) fn face_position(&mut self, p: Option<(f32, f32)>) {
        let Some((px, py)) = p else {
            return;
        };
        let (nx, ny) = normalize(px - self.vars.x, py - self.vars.y);
        self.set_forward_iso_direction(dir_from_angle(nx, ny));
        if length(nx, ny) > 0.0 {
            self.set_forward_direction(nx, ny);
        }
    }

    /// `faceThisObject(obj)` for an `IsoDoor` or `IsoThumpable`
    /// (`BarricadeAble`: facing across it from either of its squares).
    pub(crate) fn face_thump_target(
        &mut self,
        obj: pz_pathfind::mover::ObjectRef,
        cx: &mut StateCx<'_, I>,
    ) {
        let Some(info) = cx.world.thump_info(ThumpTarget::Object(obj), self.id) else {
            return;
        };
        if !info.in_world {
            return;
        }
        let cur = self.path.current_square;
        if cur.is_some() && cur == info.square {
            self.set_forward_iso_direction(if info.north {
                IsoDirections::N
            } else {
                IsoDirections::W
            });
        } else if cur.is_some() && cur == info.opposite_square {
            self.set_forward_iso_direction(if info.north {
                IsoDirections::S
            } else {
                IsoDirections::E
            });
        } else {
            self.face_position(Some(info.facing_position));
        }
    }

    /// `faceThisObject(obj)` for an animal-world object (a trough, a world
    /// inventory object).
    pub(crate) fn face_animal_object(
        &mut self,
        obj: pz_animals::ObjectRef,
        cx: &mut StateCx<'_, I>,
    ) {
        let (x, y) = (self.vars.x, self.vars.y);
        let p = cx
            .world
            .animal_host(self.id)
            .and_then(|h| h.world.facing_position(obj, x, y));
        self.face_position(p);
    }

    /// `faceThisObject(square.getFloor())`.
    pub(crate) fn face_floor(&mut self, square: pz_animals::ObjectRef, cx: &mut StateCx<'_, I>) {
        let (x, y) = (self.vars.x, self.vars.y);
        let p = cx
            .world
            .animal_host(self.id)
            .and_then(|h| h.world.floor_facing_position(square, x, y));
        self.face_position(p);
    }
}
