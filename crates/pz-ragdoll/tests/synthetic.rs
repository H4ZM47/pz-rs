//! Synthetic end-to-end check: a made-up 35-bone skeleton and ragdoll
//! definitions (not the game's) fall, settle into the player's pose, and
//! the frozen pose replays as the corpse's `DeathPose`.

use std::rc::Rc;

use pz_anim::{AnimationPlayer, RagdollHost};
use pz_bullet::{NoCallbacks, PzBullet};
use pz_model::{AnimationClip, Keyframe, Mat4, Quat, SkinningData};

const IDENTITY: Mat4 = [
    [1.0, 0.0, 0.0, 0.0],
    [0.0, 1.0, 0.0, 0.0],
    [0.0, 0.0, 1.0, 0.0],
    [0.0, 0.0, 0.0, 1.0],
];
use pz_ragdoll::controller::RagdollController;
use pz_ragdoll::script::{
    RagdollAnchor, RagdollBodyDynamics, RagdollBodyPartInfo, RagdollConstraint,
};
use pz_ragdoll::skeleton_bone::SKELETON_BONES;
use pz_ragdoll::{
    corpse, RagdollBuilder, RagdollCharacter, RagdollDebugOptions, RagdollEnv, RagdollFrame,
    RagdollGlobals, RagdollScripts, Vec3,
};

fn skinning() -> SkinningData {
    let n = SKELETON_BONES.len();
    let mut sd = SkinningData {
        bind_pose: vec![IDENTITY; n],
        inverse_bind_pose: vec![IDENTITY; n],
        bone_offset: vec![IDENTITY; n],
        skeleton_hierarchy: (0..n as i32)
            .map(|i| if i == 0 { -1 } else { (i - 1) / 2 })
            .collect(),
        ..Default::default()
    };
    for (i, b) in SKELETON_BONES.iter().enumerate() {
        sd.bone_indices.insert(b.to_string(), i as i32);
    }
    let mut keyframes = Vec::new();
    for (i, b) in SKELETON_BONES.iter().enumerate() {
        for t in [0.0, 0.5, 1.0] {
            keyframes.push(Keyframe {
                bone: i as i32,
                time: t,
                position: [0.0, if i == 0 { 0.0 } else { 0.1 }, 0.02 * (i % 3) as f32],
                rotation: Quat {
                    x: 0.0,
                    y: 0.0,
                    z: 0.0,
                    w: 1.0,
                },
                scale: [1.0; 3],
                bone_name: b.to_string(),
            });
        }
    }
    for name in ["Bob_TPose", "Stand"] {
        let clip = AnimationClip {
            name: name.into(),
            duration: 1.0,
            keep_last_frame: false,
            keyframes: keyframes.clone(),
            ..Default::default()
        };
        sd.animation_clips.insert(name.into(), clip);
    }
    sd
}

fn scripts() -> RagdollScripts {
    let mut s = RagdollScripts::default();
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
        s.anchors.push(RagdollAnchor {
            bone,
            body_part: bone % 11,
            reverse: bone % 2 == 1,
            original: bone % 5 == 0,
            enabled: true,
        });
    }
    s
}

#[derive(Default)]
struct Body {
    pos: Vec3,
    fall_on_front: bool,
    ragdoll_fall: bool,
    on_floor: bool,
}

impl RagdollCharacter for Body {
    fn id(&self) -> i32 {
        7
    }
    fn position(&self) -> Vec3 {
        self.pos
    }
    fn forward_direction_angle(&self) -> f32 {
        0.0
    }
    fn can_ragdoll(&self) -> bool {
        true
    }
    fn set_ragdoll_fall(&mut self, v: bool) {
        self.ragdoll_fall = v;
    }
    fn set_use_physic_hit_reaction(&mut self, _v: bool) {}
    fn is_fall_on_front(&self) -> bool {
        self.fall_on_front
    }
    fn set_fall_on_front(&mut self, v: bool) {
        self.fall_on_front = v;
    }
    fn set_on_floor(&mut self, v: bool) {
        self.on_floor = v;
    }
    fn do_deferred_movement_from_ragdoll(&mut self, _p: &AnimationPlayer, m: Vec3) {
        self.pos = Vec3::new(self.pos.x + m.x, self.pos.y + m.y, self.pos.z + m.z);
    }
}

#[test]
fn falls_and_freezes_into_the_corpse_pose() {
    let sd = Rc::new(skinning());
    let s = scripts();
    let mut bullet = PzBullet::new();
    bullet.init_pz_bullet();
    s.to_bullet(&mut bullet, false);
    bullet.init_world(-1, -1, 1, 1, -256, -256, false).unwrap();
    let mut player = AnimationPlayer::new(sd.clone());
    let tpose = (*player.clip("Bob_TPose").unwrap()).clone();
    let builder =
        RagdollBuilder::initialize(&mut bullet, player.skeleton(), &tpose, &s.body_dynamics);
    assert_eq!(builder.bone_hierarchy[..3], [-1, 0, 0]);
    assert_eq!(builder.pelvis_bone, Some(2));

    let mut body = Body {
        pos: Vec3::new(10.0, 12.0, 3.0),
        ..Default::default()
    };
    let mut globals = RagdollGlobals::default();
    let mut slot: Option<RagdollController> = None;
    let stand = player.play("Stand", true, false, -1.0).unwrap();
    player.track_mut(stand).unwrap().set_blend_weight(1.0);
    let mut frame_no = -1;
    let mut pelvis = Vec::new();
    let mut standing = Vec::new();
    for f in 0..120 {
        if f == 2 {
            let r = player.play("Ragdoll", false, true, 5.0).unwrap();
            player.track_mut(r).unwrap().set_blend_weight(1.0);
            player.track_mut(stand).unwrap().set_blend_weight(0.0);
        }
        if f > 0 {
            bullet
                .step_simulation(0.01, 0, 0.0, &mut NoCallbacks)
                .unwrap();
            bullet
                .step_simulation(0.01, 0, 0.0, &mut NoCallbacks)
                .unwrap();
            frame_no += 1;
        }
        let mut host = RagdollFrame {
            env: RagdollEnv {
                bullet: &mut bullet,
                globals: &mut globals,
                builder: &builder,
                character: &mut body,
                bullet_frame_no: frame_no,
                time_delta: 1.0 / 60.0,
                debug: RagdollDebugOptions::default(),
            },
            controller: &mut slot,
            multiplier: 0.8,
        };
        player.update_with_ragdoll(1.0 / 60.0, None, &mut host);
        if f == 1 {
            standing = (0..35).map(|b| player.bone_transform_at(b).prs()).collect();
        }
        if let Some(c) = &slot {
            if c.state().is_calculated {
                pelvis.push(c.pelvis_position());
            }
        }
    }
    // The controller was added once, initialised, and wrote the simulation
    // into the ragdoll clip: the pose is the simulation's now, and the
    // pelvis moves with it.
    assert_eq!(globals.number_of_active_simulations, 1);
    assert!(player.is_ragdolling());
    assert!(player
        .ragdoll_simulation_animation_clip()
        .unwrap()
        .is_ragdoll_simulation_active());
    assert!(body.ragdoll_fall);
    assert!(pelvis.len() > 50);
    assert!(
        pelvis[0].distance_to(pelvis.last().unwrap()) > 0.05,
        "pelvis {:?} .. {:?}",
        pelvis[0],
        pelvis.last()
    );
    let moved = (0..35).filter(|&b| {
        let (p, r, _) = player.bone_transform_at(b).prs();
        let (p0, r0, _) = standing[b];
        (p.x - p0.x).abs()
            + (p.y - p0.y).abs()
            + (p.z - p0.z).abs()
            + (r.x - r0.x).abs()
            + (r.y - r0.y).abs()
            + (r.z - r0.z).abs()
            > 1e-3
    });
    assert!(moved.count() > 10);

    // The corpse: the frozen pose replays exactly on a fresh player.
    let died = corpse::died_bone_transforms(
        &player,
        body.ragdoll_fall,
        corpse::can_use_current_pose_for_corpse(&player, false, true),
    );
    assert_eq!(died.len(), 35);
    let mut atlas = AnimationPlayer::new(sd.clone());
    atlas.set_target_and_current_direction(player.angle().cos(), player.angle().sin());
    corpse::set_death_pose(&mut atlas, &died);
    atlas.update(0.0, None);
    assert_eq!(atlas.track_count(), 1);
    assert!(corpse::use_ragdoll_pose(true, &died, &atlas));
    for b in 0..35 {
        assert_eq!(
            atlas.bone_transform_at(b).prs(),
            player.bone_transform_at(b).prs(),
            "bone {b}"
        );
    }

    // Releasing the controller takes the ragdoll out of the world.
    let mut host = RagdollFrame {
        env: RagdollEnv {
            bullet: &mut bullet,
            globals: &mut globals,
            builder: &builder,
            character: &mut body,
            bullet_frame_no: frame_no,
            time_delta: 1.0 / 60.0,
            debug: RagdollDebugOptions::default(),
        },
        controller: &mut slot,
        multiplier: 0.8,
    };
    player.release_ragdoll_controller(Some(&mut host as &mut dyn RagdollHost));
    assert!(slot.is_none());
    assert_eq!(globals.number_of_active_simulations, 0);
    assert!(!body.ragdoll_fall);
    assert!(!player.is_ragdolling());
}
