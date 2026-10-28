//! A character against the squares' special objects: pz-world's
//! [`CollideMover`] over a [`GameCharacter`], the mover-aware
//! `testCollideAdjacent` with its side effects (`collideWith(obj)`,
//! `setCollidedWithDoor`, `setCollidedObject`), `IsoZombie.collideWith`
//! (climb through a window, thump), the hoppable actions after a
//! square-edge collision (`checkBreakHoppable`, `checkHitHoppable`,
//! `checkBreakBendableFence`) and `climbOverFence(dir)`.
//!
//! The squares are the world's: [`StateWorld::test_collide_adjacent_mover`],
//! [`StateWorld::hoppable_actions`], [`StateWorld::thump_info`] and
//! [`StateWorld::fence_climb`].
//!
//! [`StateWorld::test_collide_adjacent_mover`]: super::state::StateWorld::test_collide_adjacent_mover
//! [`StateWorld::hoppable_actions`]: super::state::StateWorld::hoppable_actions
//! [`StateWorld::thump_info`]: super::state::StateWorld::thump_info
//! [`StateWorld::fence_climb`]: super::state::StateWorld::fence_climb

use pz_pathfind::mover::{ObjectRef, PathMover};
use pz_world::square::{CollideMover, CollidedFlags, HoppableAction, IsoDirections, MoverKind, MoverStates};

use super::state::{StateCx, ThumpKind, ThumpTarget};
use super::states::{climb_fence, ids};
use super::{CharacterKind, GameCharacter};

/// A character as the collision code reads it. `path_ignore` is the square
/// of its path's sit-on-furniture goal while it has a path
/// (`hasPath() && getPathFindBehavior2().shouldIgnoreCollisionWithSquare`).
/// TODO(furniture): `getSitOnFurnitureObject().getSquare()`.
pub struct CollideView<'a, I: 'static> {
    pub chr: &'a GameCharacter<I>,
    pub path_ignore: Option<(i32, i32, i32)>,
}

/// The mover kind of a character kind (`IsoSurvivor` is no `IsoPlayer`).
pub fn mover_kind(kind: CharacterKind) -> MoverKind {
    match kind {
        CharacterKind::Zombie => MoverKind::Zombie,
        CharacterKind::Player => MoverKind::Player,
        CharacterKind::Animal => MoverKind::Animal,
        CharacterKind::Survivor => MoverKind::Other,
    }
}

impl<I: 'static> CollideMover for CollideView<'_, I> {
    fn mover_kind(&self) -> MoverKind {
        mover_kind(self.chr.kind)
    }
    fn is_no_clip(&self) -> bool {
        self.chr.vars.no_clip
    }
    fn should_ignore_collision_with_square(&self, sq: Option<(i32, i32, i32)>) -> bool {
        sq.is_some() && sq == self.path_ignore
    }
    fn is_crawling(&self) -> bool {
        self.chr.vars.crawling
    }
    fn collided(&self) -> CollidedFlags {
        let v = &self.chr.vars;
        CollidedFlags { n: v.collided_n, s: v.collided_s, w: v.collided_w, e: v.collided_e }
    }
    fn states(&self) -> MoverStates {
        let st = |id| self.chr.is_current_state_id(id);
        MoverStates {
            attack: st(ids::ATTACK),
            stagger_back: st(ids::STAGGER_BACK),
            climb_over_fence: st(ids::CLIMB_OVER_FENCE),
            climb_through_window: st(ids::CLIMB_THROUGH_WINDOW),
            crawling_zombie_turn: st(ids::CRAWLING_ZOMBIE_TURN),
            animal_attack: st(ids::ANIMAL_ATTACK),
            animal_climb_over_fence: st(ids::ANIMAL_CLIMB_OVER_FENCE),
        }
    }
}

impl<I: 'static> GameCharacter<I> {
    /// This character's [`CollideView`], with its path's ignored square.
    pub fn collide_view<'a>(&'a self, cx: &StateCx<'_, I>) -> CollideView<'a, I> {
        let path_ignore = if self.path.path2.is_some() {
            cx.world.path_behavior(self.id).and_then(|b| {
                b.goal_sit_on_furniture_object().map(|o| (o.x, o.y, o.z)).filter(|&sq| b.should_ignore_collision_with_square(sq))
            })
        } else {
            None
        };
        CollideView { chr: self, path_ignore }
    }

    /// `square(from).testCollideAdjacent(this, dx, dy, dz)` with its side
    /// effects: a special object hit is `collideWith(obj)`,
    /// `setCollidedWithDoor(true)` for a door and `setCollidedObject(obj)`.
    pub fn test_collide_adjacent_with_mover(&mut self, from: (i32, i32, i32), d: (i32, i32, i32), cx: &mut StateCx<'_, I>) -> bool {
        if cx.has_host_error() { return false; }
        let r = {
            let view = self.collide_view(cx);
            cx.world.test_collide_adjacent_mover(&view, PathMover::mover_id(self), from, d.0, d.1, d.2)
        };
        if let Some(sp) = r.special {
            let o = sp.object;
            let obj = ObjectRef { x: o.x, y: o.y, z: o.z, id: o.id };
            self.collide_with(obj, cx);
            if cx.has_host_error() { return false; }
            if sp.door {
                self.vars.collided_with_door = true;
            }
            self.vars.collided_object = Some(obj);
        }
        r.blocked
    }

    /// `collideWith(obj)`: `IsoZombie`'s, then `IsoMovingObject`'s (the
    /// `OnObjectCollide` Lua event, not raised here).
    pub fn collide_with(&mut self, obj: ObjectRef, cx: &mut StateCx<'_, I>) {
        if cx.has_host_error() { return; }
        if self.is_zombie() {
            self.zombie_collide_with(obj, cx);
        }
    }

    /// `IsoZombie.collideWith(obj)`: climb through an open window (or a
    /// window frame, or a climbable thumpable) the zombie is moving into,
    /// else take a thumpable in the way as the thump target while chasing.
    /// An object with a `rerouteCollide` (a wall a window was built on)
    /// becomes the zombie's own `rerouteCollide`, which is null: nothing
    /// happens.
    fn zombie_collide_with(&mut self, obj: ObjectRef, cx: &mut StateCx<'_, I>) {
        if self.zombie.ghost {
            return;
        }
        if cx.world.reroute_collide(obj) {
            return;
        }
        let Some(i) = cx.world.thump_info(ThumpTarget::Object(obj), self.id) else { return };
        let path_find = self.is_current_state_id(ids::PATH_FIND);
        let moving = [ids::PATH_FIND, ids::LUNGE, ids::LUNGE_NETWORK, ids::WALK_TOWARD, ids::WALK_TOWARD_NETWORK]
            .iter()
            .any(|s| self.is_current_state_id(*s));
        let crawling = self.vars.crawling;
        if i.kind == ThumpKind::Window && i.can_climb_through && moving {
            if !self.is_facing_location(i.facing_position.0, i.facing_position.1, 0.8) {
                return;
            }
            if !path_find && !crawling {
                self.climb_through_window(obj, cx);
            }
            return;
        }
        if moving && !crawling && i.kind == ThumpKind::WindowFrame && i.can_climb_through {
            if !self.is_facing_location(i.facing_position.0, i.facing_position.1, 0.8) {
                return;
            }
            if !path_find {
                self.climb_through_window_frame(obj, cx);
            }
            return;
        }
        if i.kind == ThumpKind::Thumpable && i.can_climb_through && moving {
            if !path_find && !crawling {
                self.climb_through_window(obj, cx);
            }
            return;
        }
        if i.kind == ThumpKind::Door && i.hoppable || i.thumpable_for.is_none() || !moving {
            return;
        }
        let chasing_sound = [ids::PATH_FIND, ids::WALK_TOWARD, ids::WALK_TOWARD_NETWORK].iter().any(|s| self.is_current_state_id(*s))
            && self.path_goal_sound(cx);
        if cx.ctx.lore_thump_no_chasing() || self.vars.target.is_some() || chasing_sound {
            if i.kind == ThumpKind::Thumpable && !cx.ctx.lore_thump_on_construction() {
                return;
            }
            let mut t = ThumpTarget::Object(obj);
            if i.kind == ThumpKind::Window && i.destroyed {
                if let Some(tf) = i.thumpable_for {
                    t = tf;
                }
            }
            self.zombie.thump_target = Some(t);
        } else {
            self.set_var_bool("bPathfind", false);
            self.set_var_bool("bMoving", false);
        }
        self.clear_path2();
    }

    /// `checkBreakHoppable()`, `checkHitHoppable()` and
    /// `checkBreakBendableFence(current)` after a square-edge collision
    /// from `last`.
    pub fn check_hoppables(&mut self, last: (i32, i32, i32), cx: &mut StateCx<'_, I>) {
        let actions = {
            let view = self.collide_view(cx);
            cx.world.hoppable_actions(&view, self.id, last)
        };
        for a in actions {
            match a {
                HoppableAction::ThumpTarget(o) => {
                    self.zombie.thump_target = Some(ThumpTarget::Object(ObjectRef { x: o.x, y: o.y, z: o.z, id: o.id }));
                }
                HoppableAction::ClimbOverFence(dir) => self.do_climb_over_fence(dir, cx),
            }
        }
    }

    /// `climbOverFence(dir)`: the world's checks, then
    /// `ClimbOverFenceState.setParams` and the `EventClimbFence` event.
    pub fn do_climb_over_fence(&mut self, dir: IsoDirections, cx: &mut StateCx<'_, I>) {
        if self.is_animal() && self.animal.is_some() {
            // IsoAnimal.climbOverFence(dir).
            self.animal_climb_over_fence(dir, cx);
            return;
        }
        let Some(cur) = self.path.current_square else { return };
        let Some(f) = cx.world.fence_climb(&*self, self.is_zombie(), cur, dir) else { return };
        climb_fence::set_params(self, dir, f, cx);
        self.report_event("EventClimbFence");
    }
}
