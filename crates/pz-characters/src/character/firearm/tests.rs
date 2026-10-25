//! Firearm tests with invented weapons, characters and a synthetic physics
//! skeleton (the field names are the original's code symbols; every number
//! is made up).

use pz_inventory::ItemClass;

use super::hit_chance::*;
use super::physics::RagdollBodyPart;
use super::reaction::{body_part_for, calculate_shot_direction, resolve_hit_reaction, HitReaction, ShotDirection};
use super::FirearmStats;
use crate::character::tests::{give_item, player};
use crate::character::thermoregulator::body_tests::body_fixture;
use crate::character::*;

/// Draws `n - 1` (never a one-in-n) and the top of a float range.
struct High;
impl SimContext for High {
    fn rand_next(&self, max: i32) -> i32 {
        (max - 1).max(0)
    }
    fn rand_next_float(&self, _min: f32, max: f32) -> f32 {
        max
    }
}

/// Draws the bottom of a float range; jams on.
struct Low;
impl SimContext for Low {
    fn rand_next_float(&self, min: f32, _max: f32) -> f32 {
        min
    }
    fn firearm_jam_multiplier(&self) -> f64 {
        1.0
    }
}

#[test]
fn distance_and_delay_penalties() {
    // Point blank (3.5): the sightless bonus peaks at 0, prone doubles it.
    assert_eq!(distance_modifier_sightless(0.0, false), 40.0);
    assert_eq!(distance_modifier_sightless(0.0, true), 80.0);
    assert_eq!(distance_modifier_sightless(3.5, false), 0.0);
    assert!(distance_modifier_sightless(10.0, false) < 0.0);
    // Inside the sight range the bonus is a bell around its middle.
    let mid = distance_modifier(6.0, 2.0, 10.0, false);
    assert!(mid > distance_modifier(3.0, 2.0, 10.0, false) && mid <= 15.0);
    assert!(distance_modifier(14.0, 2.0, 10.0, false) < 0.0);
    // Aim delay counts less at point blank.
    assert_eq!(aim_delay_penalty_sightless(10.0, 1.75), 5.0);
    assert_eq!(aim_delay_penalty(10.0, 1.75, 2.0, 10.0), 5.0);
    // In the sight range: least at its middle.
    assert_eq!(aim_delay_penalty(10.0, 6.0, 2.0, 10.0), 7.5);
}

#[test]
fn ragdoll_parts_map_to_body_parts() {
    assert_eq!(RagdollBodyPart::body_part_type_field(RagdollBodyPart::Head.ordinal(), true), "Head");
    assert_eq!(RagdollBodyPart::body_part_type_field(RagdollBodyPart::Head.ordinal(), false), "Neck");
    assert_eq!(RagdollBodyPart::body_part_type_field(RagdollBodyPart::LeftUpperArm.ordinal(), false), "UpperArm_L");
    assert_eq!(RagdollBodyPart::body_part_type_field(11, true), "MAX");
    assert!(RagdollBodyPart::is_leg(4) && RagdollBodyPart::is_arm(9) && !RagdollBodyPart::is_arm(2));
    assert_eq!(HitReaction::from_string("shotheadbwd"), HitReaction::ShotHeadBwd);
    assert_eq!(HitReaction::from_string("nope"), HitReaction::None);
    assert_eq!(HitReaction::HitSpear1.value(), "HitSpearDeath1");
}

#[test]
fn shot_direction_and_reaction() {
    // Facing the same way (0 degrees): Next(9) = 8 > 6 is LEFT.
    assert_eq!(calculate_shot_direction(&High, (1.0, 0.0), (1.0, 0.0)), ShotDirection::Left);
    // A quarter turn and a bit (90..135 degrees): always RIGHT.
    assert_eq!(calculate_shot_direction(&High, (1.0, 0.0), (-0.6, 0.8)), ShotDirection::Right);
    // A critical shot from the south is a head shot.
    assert_eq!(resolve_hit_reaction(&High, true, false, ShotDirection::South), HitReaction::ShotHeadFwd02);
    assert_eq!(resolve_hit_reaction(&High, false, true, ShotDirection::North), HitReaction::ShotBellyStepBehind);
    // Next(5) = 4 from the side: the shoulder step.
    assert_eq!(resolve_hit_reaction(&High, false, false, ShotDirection::Right), HitReaction::ShotShoulderStepR);
    assert_eq!(body_part_for(&High, HitReaction::ShotShoulderStepR, ShotDirection::Right), RagdollBodyPart::RightUpperArm);
    assert_eq!(body_part_for(&High, HitReaction::ShotBelly, ShotDirection::Right), RagdollBodyPart::Pelvis);
}

/// An invented pistol in the primary hand, chambered, with `ammo` rounds.
fn pistol(sm: &pz_scripts::ScriptManager, c: &mut GameCharacter<()>, id: i32, ammo: i32) {
    give_item(
        sm,
        c,
        ItemClass::Weapon,
        "Popgun",
        id,
        |t| {
            t.ranged = true;
            t.is_aimed_firearm = true;
            t.min_damage = 0.8;
            t.max_damage = 1.0;
        },
        |s| {
            s.ranged = true;
            s.is_aimed_firearm = true;
            s.max_range = 12.0;
            s.hit_chance = 60;
            s.recoil_delay = 12;
            s.aiming_time = 20;
            s.max_hit_count = 1;
            s.jam_gun_chance = 50.0;
            s.have_chamber = true;
            s.max_ammo = 6;
            s.hit_sound = Some("PopHit".into());
        },
    );
    let it = c.items.item_mut(id).unwrap();
    if let Some(w) = it.weapon_data_mut() {
        w.min_damage = 0.8;
        w.max_damage = 1.0;
        w.min_sight_range = 2.0;
        w.max_sight_range = 9.0;
    }
    it.set_current_ammo_count(ammo);
    it.set_round_chambered(true, true);
    c.items.primary_hand_item = Some(id);
    c.vars.use_hand_weapon = Some(id);
}

#[test]
fn perk_scaled_recoil_ammo_and_jams() {
    let (sm, mut chars) = body_fixture();
    let mut p = player(&sm, &mut chars);
    pistol(&sm, &mut p, 7, 2);
    let f = FirearmStats::of(&sm, p.items.item(7).unwrap()).unwrap();
    assert!(f.ranged && f.aimed_firearm && f.have_chamber);
    // Aiming 0, Strength 0: 12 * 1 * (1 - (-10 / 40)) = 15.
    assert_eq!(p.firearm_recoil_delay(&chars, &f), 15);
    assert_eq!(p.firearm_max_sight_range(&sm, &chars, &f), 9.0);
    // One shot: the chamber refills from the magazine; jams are off by
    // default (multiplier 0).
    p.on_shoot_ammo(&sm, &chars, &High, 7);
    let w = p.items.item(7).unwrap();
    assert_eq!(w.get_current_ammo_count(), 1);
    assert!(w.is_round_chambered() && !w.is_spent_round_chambered() && !w.is_jammed());
    // A roll under the chance jams the gun; the unjam roll at 0 clears it.
    assert!(p.check_jam(&sm, &chars, &Low, 7, false));
    assert!(!p.check_unjam(&sm, &chars, &Low, 7));
    // The post-shot aiming delay is capped by the aiming time.
    p.vars.aiming_delay = 19.0;
    p.set_aiming_delay_after_shot(&chars, &f);
    assert_eq!(p.vars.aiming_delay, 20.0);
}

#[test]
fn the_ragdoll_takes_a_pending_shot_once() {
    let (sm, _chars) = body_fixture();
    let mut z: GameCharacter<()> = GameCharacter::new(&sm, CharacterKind::Zombie);
    z.vars.x = 2.0;
    z.vars.hit_reaction = "ShotChest".into();
    // Nothing pending: no impulse.
    assert!(z.take_ragdoll_hit(&sm, None).is_none());
    let t = z.ensure_ballistics_target();
    t.combat_damage_data = super::CombatDamageData {
        body_part: RagdollBodyPart::Spine,
        weapon_explosive: true,
        attacker_position: [0.0, 0.0, 0.0],
        ..Default::default()
    };
    t.combat_damage_data_processed = false;
    // No hit reactions scripted: the defaults (80 forward, 40 up).
    let h = z.take_ragdoll_hit(&sm, Some([1.0, 0.0, 0.0])).unwrap();
    assert_eq!((h.body_part, h.impulse, h.upward_impulse), (1, 80.0, 40.0));
    assert_eq!(h.attacker, [1.0, 0.0, 0.0]);
    assert_eq!(z.vars.hit_reaction, "");
    assert!(z.take_ragdoll_hit(&sm, None).is_none(), "processed");
    // A melee hit (no ammo, not explosive) is consumed without an impulse.
    let t = z.ballistics_target.as_mut().unwrap();
    t.combat_damage_data.weapon_explosive = false;
    t.combat_damage_data_processed = false;
    assert!(z.take_ragdoll_hit(&sm, None).is_none());
    assert!(z.ballistics_target.as_ref().unwrap().combat_damage_data_processed);
}
