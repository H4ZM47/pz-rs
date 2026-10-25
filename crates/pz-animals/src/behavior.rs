//! The state of `zombie.characters.animals.behavior.BaseAnimalBehavior`
//! (`IsoAnimal.getBehavior()`) and the parts of it that need no character
//! runtime: the hutch call (`callToHutch`, `canGoToHutch`, which
//! `AnimalData.checkEggs` reaches), the attack timer and
//! `setDoingBehavior`.
//!
//! The rest of the behaviour (wandering, eating and drinking lookups,
//! following, fleeing, `spotted`, attacking) moves the animal and changes
//! its states, so it is the character side's (`pz-characters`'
//! `character/animal/behavior.rs`), working on these fields.

use crate::animal::Animal;
use crate::env::{AnimalCharacter, AnimalEnv, ObjectRef};

/// `BehaviorAction`, in ordinal order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BehaviorAction {
    EatTrough,
    EatGround,
    DrinkGround,
    EatMom,
    Drink,
    Fertilize,
    EnterHutch,
    FightAnimal,
    DrinkFromRiver,
    DrinkFromPuddle,
    EatGrass,
}

impl BehaviorAction {
    /// `name()`.
    pub fn name(self) -> &'static str {
        match self {
            BehaviorAction::EatTrough => "EATTROUGH",
            BehaviorAction::EatGround => "EATGROUND",
            BehaviorAction::DrinkGround => "DRINKGROUND",
            BehaviorAction::EatMom => "EATMOM",
            BehaviorAction::Drink => "DRINK",
            BehaviorAction::Fertilize => "FERTILIZE",
            BehaviorAction::EnterHutch => "ENTERHUTCH",
            BehaviorAction::FightAnimal => "FIGHTANIMAL",
            BehaviorAction::DrinkFromRiver => "DRINKFROMRIVER",
            BehaviorAction::DrinkFromPuddle => "DRINKFROMPUDDLE",
            BehaviorAction::EatGrass => "EATGRASS",
        }
    }
}

/// `behaviorObject`: what the current action is about. The handles are
/// the [`AnimalWorld`](crate::env::AnimalWorld)'s (troughs, world
/// inventory objects, hutches, squares as [`crate::env::square_ref`]) or
/// characters as [`crate::env::character_ref`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BehaviorObject {
    /// An `IsoFeedingTrough`.
    Trough(ObjectRef),
    /// An `IsoWorldInventoryObject`.
    WorldItem(ObjectRef),
    /// An `IsoGridSquare` (river, puddle or grass floor).
    Square(ObjectRef),
    /// An `IsoHutch`.
    Hutch(ObjectRef),
    /// An `IsoAnimal` (the female to fertilize, the mother).
    Animal(ObjectRef),
    /// Another `IsoGameCharacter` (the fighting opponent).
    Character(ObjectRef),
}

/// `BaseAnimalBehavior`'s fields (the `parent` is the animal holding
/// them).
#[derive(Clone, Debug, PartialEq)]
pub struct AnimalBehavior {
    pub wander_mul_mod: f32,
    pub block_movement: bool,
    pub go_to_mom_timer: f32,
    pub sit_in_time: i32,
    pub sit_out_time: i32,
    pub blocked_for: f32,
    pub attack_animal_timer: f32,
    pub follow_chr_timer: f32,
    pub last_alerted: f32,
    pub idle_anim_timer: f32,
    pub behavior_check_timer: f32,
    pub behavior_action: Option<BehaviorAction>,
    pub behavior_object: Option<BehaviorObject>,
    pub is_doing_behavior: bool,
    pub behavior_max_time: f32,
    pub behavior_failsafe: f32,
    pub hutch_path_timer: f32,
    pub enter_hutch_timer_after_destroy: f32,
    pub forced_outside_hutch: i64,
    pub timer_flee_again: f32,
    pub wild_and_hurt: bool,
    pub wild_drop_dead_timer: f32,
}

impl Default for AnimalBehavior {
    /// `new BaseAnimalBehavior(parent)`.
    fn default() -> Self {
        AnimalBehavior {
            wander_mul_mod: 0.0,
            block_movement: false,
            go_to_mom_timer: 0.0,
            sit_in_time: 0,
            sit_out_time: 0,
            blocked_for: 0.0,
            attack_animal_timer: 0.0,
            follow_chr_timer: 0.0,
            last_alerted: 0.0,
            idle_anim_timer: 0.0,
            behavior_check_timer: 0.0,
            behavior_action: None,
            behavior_object: None,
            is_doing_behavior: false,
            behavior_max_time: 5000.0,
            behavior_failsafe: 0.0,
            hutch_path_timer: -1.0,
            enter_hutch_timer_after_destroy: 0.0,
            forced_outside_hutch: 0,
            timer_flee_again: 0.0,
            wild_and_hurt: false,
            wild_drop_dead_timer: 0.0,
        }
    }
}

impl AnimalBehavior {
    /// `setDoingBehavior(b)`: never while in a vehicle.
    pub fn set_doing_behavior(&mut self, mut doing: bool, in_vehicle: bool) {
        if in_vehicle {
            doing = false;
        }
        self.is_doing_behavior = doing;
    }

    /// `updateAttackTimer()`.
    pub fn update_attack_timer(&mut self, multiplier: f32) {
        if self.attack_animal_timer > 0.0 {
            self.attack_animal_timer -= multiplier;
            if self.attack_animal_timer < 0.0 {
                self.attack_animal_timer = 0.0;
            }
        }
    }

    /// `setHourBeforeLeavingHutch(hours)`.
    pub fn set_hour_before_leaving_hutch(&mut self, hours: i32, cal_millis: i64) {
        self.forced_outside_hutch = cal_millis + 3_600_000 * hours as i64 + 1;
    }
}

impl Animal {
    /// `getBehavior().callToHutch(hutch, force)`.
    pub fn behavior_call_to_hutch(
        &mut self,
        hutch: Option<ObjectRef>,
        force: bool,
        chr: &mut dyn AnimalCharacter,
        env: &mut AnimalEnv<'_>,
    ) -> bool {
        let ctx = env.ctx;
        let b = &mut self.behavior;
        if b.enter_hutch_timer_after_destroy > 0.0 {
            b.enter_hutch_timer_after_destroy -= ctx.clock.multiplier();
            if b.enter_hutch_timer_after_destroy < 0.0 {
                b.enter_hutch_timer_after_destroy = 0.0;
            }
            return false;
        }
        if !force {
            let adef = self.adef(ctx);
            let ok = chr.exists_in_world()
                && adef.enter_hutch_time != 0
                && adef.exit_hutch_time != 0
                && adef.hutches.is_some()
                && adef.is_inside_hutch_time(ctx.clock.hour())
                && self.hutch.is_none();
            if !ok {
                return false;
            }
        }
        let mut list = false;
        let hutch = match hutch {
            Some(h) => Some(h),
            None => {
                list = true;
                self.data_region_hutch(ctx, env)
            }
        };
        let Some(hutch) = hutch else {
            return false;
        };
        if force {
            self.behavior.forced_outside_hutch = ctx.clock.calendar_millis() + 7_200_001;
        }
        if !list {
            return self.behavior_can_go_to_hutch(hutch, force, chr, env);
        }
        for zone in self.connected_d_zone.clone() {
            for (h, _) in env.world.zone_hutches(zone) {
                if self.behavior_can_go_to_hutch(h, force, chr, env) {
                    return true;
                }
            }
        }
        false
    }

    /// `getBehavior().canGoToHutch(hutch, force)`.
    pub fn behavior_can_go_to_hutch(
        &mut self,
        hutch: ObjectRef,
        force: bool,
        chr: &mut dyn AnimalCharacter,
        env: &mut AnimalEnv<'_>,
    ) -> bool {
        if env.world.hutch_inside_count(hutch) >= env.world.hutch_max_animals(hutch) {
            return false;
        }
        if force && env.world.hutch_door_closed(hutch) {
            env.world.hutch_toggle_door(hutch);
        }
        if env.world.hutch_door_closed(hutch) {
            return false;
        }
        if self.behavior.hutch_path_timer <= -1.0 {
            self.behavior.hutch_path_timer = if self.is_baby().unwrap_or(false) {
                env.rand.next_float_range(10.0, 30.0)
            } else if chr.is_female() {
                env.rand.next_float_range(350.0, 500.0)
            } else {
                env.rand.next_float_range(950.0, 1000.0)
            };
        }
        if force {
            self.behavior.hutch_path_timer = 1.0;
        }
        self.behavior
            .set_doing_behavior(true, chr.vehicle().is_some());
        self.behavior.behavior_action = Some(BehaviorAction::EnterHutch);
        self.behavior.behavior_object = Some(BehaviorObject::Hutch(hutch));
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_and_timers() {
        let mut b = AnimalBehavior::default();
        assert_eq!(b.behavior_max_time, 5000.0);
        assert_eq!(b.hutch_path_timer, -1.0);
        b.attack_animal_timer = 3.0;
        b.update_attack_timer(2.0);
        assert_eq!(b.attack_animal_timer, 1.0);
        b.update_attack_timer(2.0);
        assert_eq!(b.attack_animal_timer, 0.0);
        b.set_doing_behavior(true, true);
        assert!(!b.is_doing_behavior);
        b.set_doing_behavior(true, false);
        assert!(b.is_doing_behavior);
        assert_eq!(BehaviorAction::EatGrass.name(), "EATGRASS");
    }
}

#[cfg(test)]
mod ref_tests {
    use crate::env::{character_ref, character_ref_id, square_ref, square_ref_xyz};

    #[test]
    fn square_and_character_refs_round_trip() {
        for (x, y, z) in [
            (0, 0, 0),
            (12345, 6789, 1),
            (-5, -7, -32),
            (8_000_000, 20000, 31),
        ] {
            let r = square_ref(x, y, z);
            assert_eq!(square_ref_xyz(r), Some((x, y, z)));
            assert_eq!(character_ref_id(r), None);
        }
        for id in [0, 1, 77, i32::MAX, -1] {
            let r = character_ref(id);
            assert_eq!(character_ref_id(r), Some(id));
            assert_eq!(square_ref_xyz(r), None);
        }
    }
}
