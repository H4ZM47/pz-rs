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
use super::{CharacterRagdoll, FirearmStats, MuzzleAttachment, TargetSkeleton};
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
    let item = p.items.item(7).unwrap().clone();
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

/// Invented ragdoll definitions over pz-ragdoll's 35 skeleton bones, and a
/// made-up skeleton with a T-pose and a standing clip.
fn ragdoll_setup() -> (std::rc::Rc<pz_model::SkinningData>, pz_ragdoll::RagdollScripts) {
    use pz_model::{AnimationClip, Keyframe, Quat, SkinningData};
    use pz_ragdoll::script::{RagdollAnchor, RagdollBodyDynamics, RagdollBodyPartInfo, RagdollConstraint};
    use pz_ragdoll::skeleton_bone::SKELETON_BONES;
    let identity = [[1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0], [0.0, 0.0, 1.0, 0.0], [0.0, 0.0, 0.0, 1.0]];
    let n = SKELETON_BONES.len();
    let mut sd = SkinningData {
        bind_pose: vec![identity; n],
        inverse_bind_pose: vec![identity; n],
        bone_offset: vec![identity; n],
        skeleton_hierarchy: (0..n as i32).map(|i| if i == 0 { -1 } else { (i - 1) / 2 }).collect(),
        ..Default::default()
    };
    let mut keyframes = Vec::new();
    for (i, b) in SKELETON_BONES.iter().enumerate() {
        sd.bone_indices.insert(b.to_string(), i as i32);
        for t in [0.0, 0.5, 1.0] {
            keyframes.push(Keyframe {
                bone: i as i32,
                time: t,
                position: [0.0, if i == 0 { 0.0 } else { 0.1 }, 0.02 * (i % 3) as f32],
                rotation: Quat { x: 0.0, y: 0.0, z: 0.0, w: 1.0 },
                scale: [1.0; 3],
                bone_name: b.to_string(),
            });
        }
    }
    for name in ["Bob_TPose", "Stand"] {
        let clip = AnimationClip { name: name.into(), duration: 1.0, keyframes: keyframes.clone(), ..Default::default() };
        sd.animation_clips.insert(name.into(), clip);
    }
    let mut s = pz_ragdoll::RagdollScripts::default();
    for part in 0..11 {
        s.body_part_infos.push(RagdollBodyPartInfo {
            part,
            calculate_length: part % 2 == 0,
            radius: 0.05,
            height: 0.2,
            gap: 0.01,
            shape: part % 3,
            mass: 0.09,
            offset: [0.0, 0.01, 0.0],
        });
        s.body_dynamics.push(RagdollBodyDynamics {
            part,
            linear_damping: 0.05,
            angular_damping: 0.8,
            deactivation_time: 0.8,
            linear_sleeping_threshold: 1.6,
            angular_sleeping_threshold: 2.5,
            friction: 1.5,
            rolling_friction: 0.5,
        });
    }
    for j in 0..10 {
        s.constraints.push(RagdollConstraint {
            joint: j,
            constraint_type: if j % 3 == 0 { 4 } else { 5 },
            part_a: j / 2,
            part_b: j + 1,
            axis_a: [0.0, 1.5, 0.0],
            axis_b: [0.0, 1.5, 0.0],
            position_offset_a: [0.0, 0.05, 0.0],
            position_offset_b: [0.0, -0.05, 0.0],
            limit: [0.7, 0.5, 0.3],
            limit_extended: [0.0; 3],
        });
    }
    for bone in 0..35 {
        s.anchors.push(RagdollAnchor { bone, body_part: bone % 11, reverse: bone % 2 == 1, original: bone % 5 == 0, enabled: true });
    }
    (std::rc::Rc::new(sd), s)
}

#[test]
fn the_character_drives_its_ragdoll() {
    use pz_anim::{AnimationPlayer, RagdollHost};
    use pz_ragdoll::{RagdollBuilder, RagdollController, RagdollDebugOptions, RagdollEnv, RagdollFrame, RagdollGlobals};
    let (sm, chars) = body_fixture();
    let (sd, scripts) = ragdoll_setup();
    let mut bullet = PzBullet::new();
    bullet.init_pz_bullet();
    scripts.to_bullet(&mut bullet, false);
    bullet.init_world(-1, -1, 1, 1, -256, -256, false).unwrap();
    let mut player = AnimationPlayer::new(sd.clone());
    let tpose = (*player.clip("Bob_TPose").unwrap()).clone();
    let builder = RagdollBuilder::initialize(&mut bullet, player.skeleton(), &tpose, &scripts.body_dynamics);

    // The shooter (in the world) and the zombie it shot with an explosive
    // (no hit reaction scripts: 80 forward, 40 up).
    let mut shooter: GameCharacter<()> = GameCharacter::new(&sm, CharacterKind::Player);
    shooter.id = 1;
    shooter.vars.x = 8.0;
    shooter.vars.y = 12.0;
    let muzzle = MuzzleAttachment { position: [0.0; 3], direction: [1.0, 0.0, 0.0] };
    let mut world = Range { chars: vec![shooter], bullet: synthetic_bullet(), muzzle };
    let ctx = Seeded { rand: pz_core::random::Rand::new(3), events: RefCell::new(Vec::new()) };
    let mut z: GameCharacter<()> = GameCharacter::new(&sm, CharacterKind::Zombie);
    z.id = 7;
    z.set_x(10.0);
    z.set_y(12.0);
    z.set_z(3.0);
    z.set_forward_direction(-1.0, 0.0);
    {
        let mut cx = StateCx::new(&sm, &chars, &ctx, &mut world);
        z.change_state(Some(crate::character::states::zombie::idle()), &mut cx);
    }
    z.vars.hit_reaction = "ShotChest".into();
    let t = z.ensure_ballistics_target();
    t.combat_damage_data = super::CombatDamageData {
        attacker: 1,
        body_part: RagdollBodyPart::Spine,
        weapon_explosive: true,
        ..Default::default()
    };
    t.combat_damage_data_processed = false;
    assert!(z.can_ragdoll_uncapped(&ctx));

    // Not ragdolling yet: no deferred movement.
    {
        use pz_ragdoll::RagdollCharacter;
        let cx = StateCx::new(&sm, &chars, &ctx, &mut world);
        let mut character = CharacterRagdoll::new(&mut z, cx);
        character.do_deferred_movement_from_ragdoll(&player, pz_ragdoll::Vec3::new(1.0, 1.0, 0.0));
        // The shot's impulse: from the shooter's position now towards the
        // zombie (taken by the controller below, so put back).
        let h = character.take_ragdoll_hit().unwrap();
        assert_eq!((h.attacker, h.target), (pz_ragdoll::Vec3::new(8.0, 12.0, 0.0), pz_ragdoll::Vec3::new(10.0, 12.0, 3.0)));
        assert_eq!((h.body_part, h.impulse, h.upward_impulse), (1, 80.0, 40.0));
    }
    assert_eq!((z.vars.x, z.vars.y), (10.0, 12.0));
    z.vars.hit_reaction = "ShotChest".into();
    z.ballistics_target.as_mut().unwrap().combat_damage_data_processed = false;
    let mut globals = RagdollGlobals::default();
    let mut slot: Option<RagdollController> = None;
    let stand = player.play("Stand", true, false, -1.0).unwrap();
    player.track_mut(stand).unwrap().set_blend_weight(1.0);
    let mut frame_no = -1;
    for f in 0..120 {
        if f == 2 {
            let r = player.play("Ragdoll", false, true, 5.0).unwrap();
            player.track_mut(r).unwrap().set_blend_weight(1.0);
            player.track_mut(stand).unwrap().set_blend_weight(0.0);
        }
        if f > 0 {
            bullet.step_simulation(0.01, 0, 0.0, &mut pz_bullet::NoCallbacks).unwrap();
            bullet.step_simulation(0.01, 0, 0.0, &mut pz_bullet::NoCallbacks).unwrap();
            frame_no += 1;
        }
        z.update_ragdoll_variables(&ctx);
        {
            let cx = StateCx::new(&sm, &chars, &ctx, &mut world);
            let mut character = CharacterRagdoll::new(&mut z, cx);
            let mut host = RagdollFrame {
                env: RagdollEnv {
                    bullet: &mut bullet,
                    globals: &mut globals,
                    builder: &builder,
                    character: &mut character,
                    bullet_frame_no: frame_no,
                    time_delta: 1.0 / 60.0,
                    debug: RagdollDebugOptions::default(),
                },
                controller: &mut slot,
                multiplier: 0.8,
            };
            player.update_with_ragdoll(1.0 / 60.0, None, &mut host);
        }
        CharacterRagdoll::track(&mut z, &player, slot.as_ref());
    }
    // The ragdoll took the pending shot once: processed, hit reaction
    // cleared.
    let t = z.ballistics_target.as_ref().unwrap();
    assert!(t.combat_damage_data_processed);
    assert_eq!(z.vars.hit_reaction, "");
    // The controller's state is the character's.
    assert_eq!(globals.number_of_active_simulations, 1);
    assert!(z.ragdoll.has_controller && z.ragdoll.is_ragdolling);
    assert!(z.vars.ragdoll_fall, "the simulation direction was calculated");
    assert_eq!(z.vars.ragdoll_simulation_active, player.is_ragdoll_simulation_active());
    assert_eq!(z.vars.on_floor, !z.ragdoll.upright);
    assert_eq!(z.environment.fall_on_front, !z.ragdoll.on_back);
    // `doDeferredMovementFromRagdoll` while ragdolling: `moveUnmoddedInternal`,
    // then the position follows the next position.
    let (x, y, zz) = (z.vars.x, z.vars.y, z.vars.z);
    {
        use pz_ragdoll::RagdollCharacter;
        let cx = StateCx::new(&sm, &chars, &ctx, &mut world);
        let mut character = CharacterRagdoll::new(&mut z, cx);
        character.do_deferred_movement_from_ragdoll(&player, pz_ragdoll::Vec3::new(0.25, -0.5, 0.125));
        assert_eq!(character.take_ragdoll_hit(), None, "the shot was taken once");
    }
    assert_eq!((z.vars.x, z.vars.y, z.vars.z), (x + 0.25, y - 0.5, zz + 0.125));
    assert_eq!((z.vars.next_x, z.vars.next_y), (z.vars.x, z.vars.y));
    assert_eq!(z.vars.req_movement, (0.25, -0.5));

    // Releasing the controller takes it out of the world and clears the
    // fall.
    {
        let cx = StateCx::new(&sm, &chars, &ctx, &mut world);
        let mut character = CharacterRagdoll::new(&mut z, cx);
        let mut host = RagdollFrame {
            env: RagdollEnv {
                bullet: &mut bullet,
                globals: &mut globals,
                builder: &builder,
                character: &mut character,
                bullet_frame_no: frame_no,
                time_delta: 1.0 / 60.0,
                debug: RagdollDebugOptions::default(),
            },
            controller: &mut slot,
            multiplier: 0.8,
        };
        player.release_ragdoll_controller(Some(&mut host as &mut dyn RagdollHost));
    }
    CharacterRagdoll::track(&mut z, &player, slot.as_ref());
    assert!(slot.is_none() && !z.ragdoll.has_controller);
    assert_eq!(globals.number_of_active_simulations, 0);
    assert!(!z.vars.ragdoll_fall && !z.vars.use_physic_hit_reaction);
}
