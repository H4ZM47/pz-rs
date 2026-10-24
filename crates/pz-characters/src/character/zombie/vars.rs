//! `IsoZombie.registerVariableCallbacks`: the zombie's animation variable
//! slots (`battack`, `blunge`, `bthump`, `turndirection`, ...).
//!
//! The original's getters read the target and the squares directly; here
//! [`GameCharacter::refresh_zombie_view`] snapshots what they read
//! ([`TargetView`]) before the action context evaluates the transitions,
//! and the getters' side effects (`setTarget(null)` from `bHasTarget`,
//! `battack` and `blunge`, `lungeTimer = 0` from `blunge`) are recorded and
//! applied after the evaluation ([`GameCharacter::apply_zombie_var_effects`]).

use pz_anim::VariableValue;

use super::math::*;
use super::GameCharacter;
use crate::character::state::{CharVar, StateCx};
use crate::character::states::ids;

/// What the slots read about the target and the squares.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TargetView {
    /// The target exists in the world.
    pub present: bool,
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub on_floor: bool,
    pub in_vehicle: bool,
    pub reanimated_corpse: bool,
    pub ghost: bool,
    pub zombies_dont_attack: bool,
    pub in_bumped_state: bool,
    pub climbing_wall: bool,
    pub width: f32,
    /// The target's vehicle is next to the zombie
    /// (`vehicle.isCharacterAdjacentTo(this)`).
    pub vehicle_adjacent: bool,
    /// `getVehicle().getScript().isSmallVehicle`.
    pub small_vehicle: bool,
    /// `currentSquare.isSomethingTo(targetSquare)` (and the reverse).
    pub something_between: bool,
    pub something_between_rev: bool,
    /// `BentFences.isUnbentObject(target)`.
    pub unbent: bool,
    /// `isSideOfStaircaseBetweenSelfAndTarget()`.
    pub side_of_staircase: bool,
    /// `lineClearCollide(..., target, false, true)` for the lunge: blocked.
    pub lunge_blocked: bool,
    /// The thump target is an object farther than 3 squares or off the map.
    pub thump_target_gone: bool,
    /// `shouldGetUpFromCrawl()`.
    pub get_up_from_crawl: bool,
    /// The `turndirection` slot's answer.
    pub turn_direction: String,
    /// `isUnderVehicle()`.
    pub under_vehicle: bool,
    /// `AttackVehicleState.isPassengerExposed(this)`.
    pub passenger_exposed: bool,
    /// `bPathfind` is set.
    pub pathfind: bool,
    /// `SystemDisabler.zombiesDontAttack`.
    pub system_zombies_dont_attack: bool,
}

/// `lineClearCollide` flags: ignore doors.
pub const LCC_IGNORE_DOORS: i32 = 1;
/// Close to walls.
pub const LCC_CLOSE_TO_WALLS: i32 = 2;
/// Check characters' vehicles (non-zombies' `pathToAux`).
pub const LCC_CHECK_VEHICLES: i32 = 4;
/// The 6-argument overload (`ignoreDoors = closeToWalls = true`).
pub const LCC_DEFAULT: i32 = LCC_IGNORE_DOORS | LCC_CLOSE_TO_WALLS;

/// The zombie slots' getters; `None` for other slots.
pub fn get<I: 'static>(var: CharVar, c: &GameCharacter<I>) -> Option<VariableValue> {
    use VariableValue::{Bool as B, Float as F, Int, String as S};
    let z = &c.zombie;
    let t = &z.target_view;
    let target = c.vars.target.filter(|_| !z.clear_target_requested.get());
    let has = target.is_some() && t.present;
    let vlen = length(z.vector_to_target.0, z.vector_to_target.1);
    Some(match var {
        CharVar::ZClient => B(false),
        CharVar::ZMultiplayer => B(false),
        CharVar::ZMovingNetwork => B(c.vars.moving),
        CharVar::ZHitHeadType => Int(c.vars.hit_head_while_on_floor),
        CharVar::ZRealState => S("Idle".into()),
        CharVar::ZReanimatedForGrappleOnly => B(z.reanimated_for_grapple_only),
        CharVar::ZAttack => B(should_attack(c)),
        CharVar::ZIsFacingTarget => B(has && c.is_facing_target_at(Some((t.x, t.y)))),
        CharVar::ZTargetSeenTime => F(z.target_seen_time),
        CharVar::ZAttackVehicle => {
            let v = !t.pathfind && !c.vars.moving && has && (t.z - c.vars.z).abs() < 0.8 && !t.ghost && t.in_vehicle && t.vehicle_adjacent;
            B(v)
        }
        CharVar::ZEatBodyTarget => B(z.force_eating_animation || z.eat_body_target.is_some()),
        CharVar::ZBecomeCrawler => B(c.vars.become_crawler),
        CharVar::ZFakeDead => B(z.fake_dead),
        CharVar::ZHasTarget => {
            if has && t.reanimated_corpse {
                z.clear_target_requested.set(true);
                return Some(B(false));
            }
            B(target.is_some())
        }
        CharVar::ZCanSeeTarget => B(z.can_see_target),
        CharVar::ZShouldSprint => {
            if has && t.reanimated_corpse {
                z.clear_target_requested.set(true);
            }
            let tgt = c.vars.target.filter(|_| !z.clear_target_requested.get());
            let sound = z.sound_source_target.is_some() && !z.sound_source_target_is_zombie;
            B(tgt.is_some() || sound)
        }
        CharVar::ZKnockedDown => B(c.vars.knocked_down),
        CharVar::ZLunge => B(lunge(c)),
        CharVar::ZStopLunging => {
            if !has {
                return Some(B(true));
            }
            if z.lunge_timer > 0.0 {
                return Some(B(false));
            }
            let d = vlen - z.width + t.width;
            B(d >= 0.8 || t.side_of_staircase)
        }
        CharVar::ZPassengerExposed => B(t.passenger_exposed),
        CharVar::ZTargetIsSmallVehicle => B(!(has && t.in_vehicle) || t.small_vehicle),
        CharVar::ZReanimate => B(z.reanimate),
        CharVar::ZStaggerBack => B(c.vars.stagger_back),
        CharVar::ZUnbalancedLevel => F(z.unbalanced_level),
        CharVar::ZThump => B(z.thump_target.is_some() && !t.thump_target_gone && z.thump_timer <= 0),
        CharVar::ZUnderVehicle => B(t.under_vehicle),
        CharVar::ZBeingSteppedOn => B(z.being_stepped_on),
        CharVar::ZDistanceToTarget => F(if has { vlen - z.width + t.width } else { -1.0 }),
        CharVar::ZLastTargetSeen => B(z.last_target_seen_x != -1),
        CharVar::ZLungeTimer => F(z.lunge_timer),
        CharVar::ZReanimateTimer => F(c.vars.reanimate_timer),
        CharVar::ZTurnDirection => S(t.turn_direction.clone()),
        CharVar::ZAlerted => B(z.alerted),
        CharVar::ZWalkType => S(z.walk_type.clone().unwrap_or_default()),
        CharVar::ZCrawlerType => Int(z.crawler_type),
        CharVar::ZGetUpFromCrawl => B(t.get_up_from_crawl),
        CharVar::ZPlayerAttackPosition => S(c.vars.player_attack_position.clone().unwrap_or_default()),
        CharVar::ZEatSpeed => F(z.eat_speed),
        CharVar::ZIsSitting => B(c.vars.sit_against_wall),
        CharVar::ZKnifeDeath => B(z.knife_death),
        CharVar::ZJawStabAttach => B(z.jaw_stab_attach),
        CharVar::ZPathFindPrediction => B(false),
        CharVar::ZCrawling => B(c.vars.crawling),
        CharVar::ZAttackDidDamage => B(z.attack_did_damage),
        CharVar::ZAttackOutcome => S(z.attack_outcome.clone()),
        _ => return None,
    })
}

/// The writable zombie slots' setters; `None` for other slots,
/// `Some(false)` for a read-only zombie slot.
pub fn set<I: 'static>(var: CharVar, c: &mut GameCharacter<I>, value: &VariableValue) -> Option<bool> {
    let b = value.as_bool();
    match var {
        CharVar::ZBecomeCrawler => c.vars.become_crawler = b,
        CharVar::ZReanimate => c.zombie.reanimate = b,
        CharVar::ZUnbalancedLevel => c.zombie.unbalanced_level = value.as_float(),
        CharVar::ZKnifeDeath => c.zombie.knife_death = b,
        CharVar::ZJawStabAttach => c.zombie.jaw_stab_attach = b,
        CharVar::ZCrawling => c.vars.crawling = b,
        CharVar::ZAttackDidDamage => c.zombie.attack_did_damage = b,
        CharVar::ZAttackOutcome => c.zombie.attack_outcome = value.as_string(),
        v if is_read_only(v).is_some() => return Some(false),
        _ => return None,
    }
    Some(true)
}

/// Whether a zombie slot has no setter; `None` for other slots.
pub fn is_read_only(var: CharVar) -> Option<bool> {
    let name = format!("{var:?}");
    if !name.starts_with('Z') {
        return None;
    }
    Some(!matches!(
        var,
        CharVar::ZBecomeCrawler
            | CharVar::ZReanimate
            | CharVar::ZUnbalancedLevel
            | CharVar::ZKnifeDeath
            | CharVar::ZJawStabAttach
            | CharVar::ZCrawling
            | CharVar::ZAttackDidDamage
            | CharVar::ZAttackOutcome
    ))
}

/// `getShouldAttack()`.
fn should_attack<I: 'static>(c: &GameCharacter<I>) -> bool {
    let z = &c.zombie;
    let t = &z.target_view;
    if t.system_zombies_dont_attack || z.reanimated_for_grapple_only {
        return false;
    }
    let target = c.vars.target.filter(|_| !z.clear_target_requested.get());
    if target.is_none() || !t.present || t.zombies_dont_attack {
        return false;
    }
    if t.on_floor && !t.in_bumped_state {
        z.clear_target_requested.set(true);
        return false;
    }
    if t.in_vehicle || t.reanimated_corpse || t.climbing_wall {
        return false;
    }
    if z.reanimate {
        return false;
    }
    if (t.z - c.vars.z).abs() >= 0.2 {
        return false;
    }
    if t.ghost {
        return false;
    }
    if z.fake_dead || c.vars.crawling {
        return !t.under_vehicle && distance_manhatten(c.vars.x, c.vars.y, t.x, t.y) < 1.3;
    }
    if t.something_between && !t.unbent {
        return false;
    }
    let range = if c.vars.crawling { 1.4 } else { 0.72 };
    length(z.vector_to_target.0, z.vector_to_target.1) <= range
}

/// The `blunge` slot.
fn lunge<I: 'static>(c: &GameCharacter<I>) -> bool {
    let z = &c.zombie;
    let t = &z.target_view;
    let target = c.vars.target.filter(|_| !z.clear_target_requested.get());
    if target.is_none() || !t.present {
        return false;
    }
    if fastfloor(c.vars.z) != fastfloor(t.z) {
        return false;
    }
    if t.in_vehicle || t.reanimated_corpse {
        return false;
    }
    if t.ghost {
        z.clear_target_requested.set(true);
        return false;
    }
    if t.something_between_rev && z.thump_target.is_some() && !t.unbent {
        return false;
    }
    if t.side_of_staircase {
        z.lunge_timer_reset_requested.set(true);
        return false;
    }
    if c.is_current_state_id(ids::ZOMBIE_TURN_ALERTED) && !c.is_facing_target_at(Some((t.x, t.y))) {
        return false;
    }
    let len = length(z.vector_to_target.0, z.vector_to_target.1);
    if len > 3.5 && !(len <= 4.0 && t.in_vehicle) {
        return false;
    }
    !t.lunge_blocked
}

/// `CrawlingZombieTurnState.calculateDir(owner, targetDir)`: true turns
/// left.
pub fn calculate_dir(forward: pz_world::square::IsoDirections, target: pz_world::square::IsoDirections) -> bool {
    let (t, f) = (target as i32, forward as i32);
    if t > f {
        return t - f <= 4;
    }
    t - f < -4
}

impl<I: 'static> GameCharacter<I> {
    /// Snapshots what the zombie slots read (run before
    /// `update_action_context`).
    pub fn refresh_zombie_view(&mut self, cx: &mut StateCx<'_, I>) {
        if !self.is_zombie() {
            return;
        }
        let mut t = TargetView {
            pathfind: self.variable_boolean("bPathfind"),
            system_zombies_dont_attack: cx.ctx.zombies_dont_attack(),
            ..TargetView::default()
        };
        let my_sq = self.current_square_xyz();
        if let Some(id) = self.vars.target {
            if let Some(o) = cx.world.character_mut(id) {
                t.present = true;
                t.x = o.vars.x;
                t.y = o.vars.y;
                t.z = o.vars.z;
                t.on_floor = o.vars.on_floor;
                t.in_vehicle = o.path.vehicle.is_some();
                t.reanimated_corpse = o.seen.reanimated_corpse.is_some();
                t.ghost = o.is_player() && o.is_ghost_mode(cx.ctx);
                t.zombies_dont_attack = o.cheats.is_set(cx.ctx, "ZOMBIES_DONT_ATTACK");
                t.in_bumped_state = o.is_current_state_id(ids::BUMPED);
                t.climbing_wall = o.is_current_state_id(ids::CLIMB_OVER_WALL);
                t.width = o.zombie.width;
                let o_sq = o.path.current_square;
                if let (Some(a), Some(b)) = (my_sq, o_sq) {
                    t.something_between = cx.world.is_something_to(a, b);
                    t.something_between_rev = cx.world.is_something_to(b, a);
                    let stairs_n = |s: (i32, i32, i32)| cx.world.square(s.0, s.1, s.2).is_some_and(|q| q.has_stairs_north);
                    let stairs_w = |s: (i32, i32, i32)| cx.world.square(s.0, s.1, s.2).is_some_and(|q| q.has_stairs_west);
                    t.side_of_staircase = ((a.0 == b.0 - 1 || a.0 == b.0 + 1) && (stairs_n(a) || stairs_n(b)))
                        || ((a.1 == b.1 - 1 || a.1 == b.1 + 1) && (stairs_w(a) || stairs_w(b)));
                }
                t.lunge_blocked =
                    cx.world.line_clear_collide(self.vars.x, self.vars.y, t.x, t.y, fastfloor(self.vars.z), Some(id), LCC_CLOSE_TO_WALLS);
            }
        }
        if let Some(crate::character::state::ThumpTarget::Object(o)) = self.zombie.thump_target {
            let info = cx.world.thump_info(crate::character::state::ThumpTarget::Object(o), self.id);
            t.thump_target_gone = match info.and_then(|i| i.square) {
                None => true,
                Some(sq) => self.dist_to_squared_xy(sq.0 as f32 + 0.5, sq.1 as f32 + 0.5) > 9.0,
            };
        }
        t.get_up_from_crawl = self.should_get_up_from_crawl(cx);
        t.turn_direction = self.turn_direction(&t, cx);
        self.zombie.target_view = t;
        self.zombie.clear_target_requested.set(false);
        self.zombie.lunge_timer_reset_requested.set(false);
        // The `bthump` slot also drops the target it finds gone.
        if self.zombie.target_view.thump_target_gone || self.zombie.thump_timer > 0 {
            self.zombie.thump_target = None;
            self.zombie.target_view.thump_target_gone = false;
        }
    }

    /// Applies the getters' recorded side effects.
    pub fn apply_zombie_var_effects(&mut self) {
        if self.zombie.clear_target_requested.replace(false) {
            self.set_zombie_target(None);
        }
        if self.zombie.lunge_timer_reset_requested.replace(false) {
            self.zombie.lunge_timer = 0.0;
        }
    }

    /// The `turndirection` slot.
    fn turn_direction(&self, t: &TargetView, cx: &StateCx<'_, I>) -> String {
        if self.path.path2.is_some() {
            return String::new();
        }
        let vlen = length(self.zombie.vector_to_target.0, self.zombie.vector_to_target.1);
        let (dx, dy) = if self.vars.target.is_none() || !t.present || vlen == 0.0 {
            if !self.is_current_state_id(ids::WALK_TOWARD) {
                return String::new();
            }
            let (lx, ly) = crate::character::states::zombie::walk_toward_target_location(self, cx);
            (lx - self.vars.x, ly - self.vars.y)
        } else {
            self.zombie.vector_to_target
        };
        let target_dir = crate::character::state::vars::iso_direction_from_angle(dx, dy);
        if self.vars.dir == target_dir {
            return String::new();
        }
        if calculate_dir(self.vars.dir, target_dir) { "left" } else { "right" }.to_string()
    }

    /// `shouldGetUpFromCrawl()`.
    pub fn should_get_up_from_crawl(&self, cx: &StateCx<'_, I>) -> bool {
        if self.is_current_state_id(ids::ZOMBIE_GET_UP_FROM_CRAWL) {
            return true;
        }
        if self.is_current_state_id(ids::ZOMBIE_GET_UP) {
            return self.state.machine.previous().is_some_and(|p| p.id() == ids::ZOMBIE_GET_UP_FROM_CRAWL);
        }
        if !self.vars.crawling || !self.zombie.can_walk {
            return false;
        }
        if self.is_current_state_id(ids::PATH_FIND) {
            if self.state.machine.previous().is_some_and(|p| p.id() == ids::ZOMBIE_GET_DOWN)
                && crate::character::states::zombie::get_down_is_near_start_xy(self)
            {
                return false;
            }
            return cx.world.path_behavior(self.id).is_some_and(|b| b.should_get_up_from_crawl(self));
        }
        if self.is_current_state_id(ids::WALK_TOWARD) {
            let (tx, ty, _) = self.path_target_f(cx);
            if self.dist_to_squared_xy(tx, ty) > 0.010_000_001
                && cx.world.line_clear_collide(self.vars.x, self.vars.y, tx, ty, fastfloor(self.vars.z), None, LCC_DEFAULT)
            {
                return false;
            }
        }
        if self.is_current_state_id(ids::ZOMBIE_GET_DOWN) {
            return false;
        }
        cx.world.can_stand_at(self.vars.x, self.vars.y, fastfloor(self.vars.z))
    }
}
