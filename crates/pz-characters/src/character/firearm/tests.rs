//! Firearm tests with invented weapons, characters and a synthetic physics
//! skeleton (the field names are the original's code symbols; every number
//! is made up).

use std::cell::RefCell;

use pz_anim::AnimationVariableSource;
use pz_bullet::PzBullet;
use pz_inventory::ItemClass;

use super::hit_chance::*;
use super::physics::{PhysicsWorld, RagdollBodyPart};
use super::reaction::{body_part_for, calculate_shot_direction, resolve_hit_reaction, HitReaction, ShotDirection};
use super::{FirearmStats, MuzzleAttachment, TargetSkeleton};
use crate::character::state::{SquareInfo, StateCx, StateWorld};
use crate::character::tests::{give_item, player};
use crate::character::thermoregulator::body_tests::body_fixture;
use crate::character::*;

/// Draws from a Java-compatible generator, recording the events.
struct Seeded {
    rand: pz_core::random::Rand,
    events: RefCell<Vec<String>>,
}

impl SimContext for Seeded {
    fn rand_next(&self, max: i32) -> i32 {
        self.rand.next(max)
    }
    fn rand_next_range(&self, min: i32, max: i32) -> i32 {
        self.rand.next_range(min, max)
    }
    fn rand_next_float(&self, min: f32, max: f32) -> f32 {
        self.rand.next_float(min, max)
    }
    fn event(&self, e: SimEvent) {
        self.events.borrow_mut().push(format!("{e:?}"));
    }
    fn debug_always_hit_target(&self) -> bool {
        true
    }
}

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
    let it = c.items.item(id).unwrap();
    let mut it = it.borrow_mut();
    if let Some(w) = it.weapon_data_mut() {
        w.min_damage = 0.8;
        w.max_damage = 1.0;
        w.min_sight_range = 2.0;
        w.max_sight_range = 9.0;
    }
    it.set_current_ammo_count(ammo);
    it.set_round_chambered(true, true);
    drop(it);
    c.items.primary_hand_item = Some(id);
    c.vars.use_hand_weapon = Some(id);
}

#[test]
fn perk_scaled_recoil_ammo_and_jams() {
    let (sm, mut chars) = body_fixture();
    let mut p = player(&sm, &mut chars);
    pistol(&sm, &mut p, 7, 2);
    let f = FirearmStats::of(&sm, &p.items.item(7).unwrap().borrow()).unwrap();
    assert!(f.ranged && f.aimed_firearm && f.have_chamber);
    // Aiming 0, Strength 0: 12 * 1 * (1 - (-10 / 40)) = 15.
    assert_eq!(p.firearm_recoil_delay(&chars, &f), 15);
    assert_eq!(p.firearm_max_sight_range(&sm, &chars, &f), 9.0);
    // One shot: the chamber refills from the magazine; jams are off by
    // default (multiplier 0).
    p.on_shoot_ammo(&sm, &chars, &High, 7);
    let w = p.items.item(7).unwrap();
    let w = w.borrow();
    assert_eq!(w.get_current_ammo_count(), 1);
    assert!(w.is_round_chambered() && !w.is_spent_round_chambered() && !w.is_jammed());
    drop(w);
    // A roll under the chance jams the gun; the unjam roll at 0 clears it.
    assert!(p.check_jam(&sm, &chars, &Low, 7, false));
    assert!(!p.check_unjam(&sm, &chars, &Low, 7));
    // The post-shot aiming delay is capped by the aiming time.
    p.vars.aiming_delay = 19.0;
    p.set_aiming_delay_after_shot(&chars, &f);
    assert_eq!(p.vars.aiming_delay, 20.0);
}

/// A shooter's world: the zombies, the synthetic physics and the
/// animation inputs.
struct Range {
    chars: Vec<GameCharacter<()>>,
    bullet: PzBullet,
    muzzle: MuzzleAttachment,
}

impl StateWorld<()> for Range {
    fn character_mut(&mut self, id: i32) -> Option<&mut GameCharacter<()>> {
        self.chars.iter_mut().find(|c| c.id == id)
    }
    fn take_character(&mut self, id: i32) -> Option<GameCharacter<()>> {
        let i = self.chars.iter().position(|c| c.id == id)?;
        Some(self.chars.remove(i))
    }
    fn put_character(&mut self, chr: GameCharacter<()>) {
        self.chars.push(chr);
    }
    fn characters_near(&self, _x: f32, _y: f32, _z: f32, _r: f32) -> Vec<i32> {
        self.chars.iter().map(|c| c.id).collect()
    }
    fn square(&self, x: i32, y: i32, z: i32) -> Option<SquareInfo> {
        Some(SquareInfo { x, y, z, ..Default::default() })
    }
    fn physics(&mut self) -> Option<&mut dyn PhysicsWorld> {
        Some(&mut self.bullet)
    }
    fn muzzle_attachment(&self, _chr: i32) -> Option<MuzzleAttachment> {
        Some(self.muzzle)
    }
    fn aim_reticle_iso(&self, _chr: i32) -> (f32, f32) {
        (4.0, 0.0)
    }
    fn ballistics_skeleton(&self, _chr: i32) -> Option<TargetSkeleton> {
        let mut packed = Vec::new();
        for i in 0..35 {
            let y = if i == 0 { 0.0 } else { 0.05 };
            packed.extend_from_slice(&[0.0, y, 0.0, 0.0, 0.0, 0.0, 1.0]);
        }
        Some(TargetSkeleton { bone_count: 35, angle: 0.0, packed })
    }
}

/// A 35-bone chain standing up the physics y axis with 11 capsule parts.
fn synthetic_bullet() -> PzBullet {
    let mut b = PzBullet::new();
    b.init_pz_bullet();
    b.init_world(-2, -2, 2, 2, 0, 0, false).unwrap();
    let mut info = Vec::new();
    for part in 0..11 {
        info.extend_from_slice(&[part as f32, 1.0, 0.08, 0.3, 0.02, 0.0, 0.09, 0.0, 0.0, 0.0]);
    }
    b.define_ragdoll_body_part_info(Some(&info), false).unwrap();
    let mut joints = Vec::new();
    for j in 0..10 {
        joints.extend_from_slice(&[j as f32, 5.0, j as f32, (j + 1) as f32]);
        joints.extend_from_slice(&[0.0; 12]);
        joints.extend_from_slice(&[0.5; 6]);
    }
    b.define_ragdoll_constraints(Some(&joints), false).unwrap();
    let mut dynamics = Vec::new();
    for part in 0..11 {
        dynamics.extend_from_slice(&[part as f32, 0.05, 0.9, 0.8, 1.6, 2.5, 1.5, 0.5]);
    }
    b.define_ragdoll_body_dynamics(Some(&dynamics), false).unwrap();
    let bones = [2, 4, 6, 20, 21, 24, 25, 8, 9, 14, 15];
    let mut anchors = Vec::new();
    for bone in 0..35 {
        let part = bones.iter().position(|&x| x == bone);
        anchors.extend_from_slice(&[bone as f32, part.unwrap_or(0) as f32, 0.0, 0.0, if part.is_some() { 1.0 } else { 0.0 }]);
    }
    b.define_ragdoll_anchors(Some(&anchors), false).unwrap();
    let hierarchy: Vec<i32> = (0..35).map(|i| i - 1).collect();
    b.initialize_ragdoll_skeleton(35, &hierarchy);
    let mut pose = Vec::new();
    for i in 0..35 {
        let y = if i == 0 { 0.0 } else { 0.05 };
        pose.extend_from_slice(&[0.0, y, 0.0, 0.0, 0.0, 0.0, 1.0]);
    }
    b.initialize_ragdoll_pose(35, &pose, 0.0, 0.0, 0.0, 1.0);
    b
}

#[test]
fn a_shot_hits_the_zombie_in_front() {
    let (sm, mut chars) = body_fixture();
    let mut p = player(&sm, &mut chars);
    p.id = 1;
    p.set_forward_direction(1.0, 0.0);
    pistol(&sm, &mut p, 7, 6);
    p.set_variable_string("ZombieHitReaction", "Shot");
    let mut z: GameCharacter<()> = GameCharacter::new(&sm, CharacterKind::Zombie);
    z.id = 2;
    z.vars.x = 4.0;
    z.vars.shootable = true;
    z.health = 50.0;
    z.set_forward_direction(-1.0, 0.0);
    // The muzzle at physics height 0.6 (iso z 0.6 / 2.44949), level.
    let muzzle = MuzzleAttachment { position: [0.4, 0.0, 0.6 / 2.44949], direction: [1.0, 0.0, 0.0] };
    let mut world = Range { chars: vec![z], bullet: synthetic_bullet(), muzzle };
    let ctx = Seeded { rand: pz_core::random::Rand::new(11), events: RefCell::new(Vec::new()) };
    let item = p.items.item(7).unwrap().borrow().clone();
    let f = FirearmStats::of(&sm, &item).unwrap();
    // Aiming frames (the reticle's hit list adds the target), then the shot.
    for _ in 0..3 {
        let mut t = world.take_character(2).unwrap();
        t.update_ballistics_target(&ctx, &mut world);
        world.put_character(t);
        p.update_ballistics(&sm, &chars, &ctx, &mut world);
        p.calculate_ballistics(&mut chars.combat.borrow_mut(), Some(&muzzle), 12.0);
        let mut cx = StateCx::new(&sm, &chars, &ctx, &mut world);
        p.calculate_ranged_hit_info_list(&mut cx, &f).unwrap();
        world.bullet.step_simulation(1.0 / 60.0, 1, 1.0 / 60.0, &mut pz_bullet::NoCallbacks).unwrap();
        if let Some(c) = p.ballistics_controller.as_mut() {
            c.post_update();
        }
    }
    assert!(world.chars[0].ballistics_target.is_some(), "the reticle query added the target");
    let mut cx = StateCx::new(&sm, &chars, &ctx, &mut world);
    p.ranged_attack_collision_check(&mut cx, &item).unwrap();
    assert_eq!(p.hit_info_list.len(), 1);
    assert_eq!(p.hit_info_list[0].target, 2);
    assert_eq!(p.hit_info_list[0].chance, 100);
    assert_eq!(p.vars.last_hit_count, 1);
    assert!(crate::character::states::swipe::ATTACKED.get(&mut p.state.params));
    let z = &world.chars[0];
    assert!(z.health < 50.0, "damage applied");
    assert!(z.vars.hit_reaction.starts_with("Shot"), "{}", z.vars.hit_reaction);
    assert_eq!(z.vars.attacked_by, Some(1));
    let ev = ctx.events.borrow();
    assert!(ev.iter().any(|e| e.starts_with("BulletTracer")), "{ev:?}");
    assert!(ev.iter().any(|e| e.contains("Shots Fired")));
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
