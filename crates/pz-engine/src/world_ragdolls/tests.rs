use super::*;

use pz_anim::clip::{AnimClip, Keyframe};
use pz_anim::math::{Quaternion, Vector3f};
use pz_anim::skeleton::Skeleton;
use pz_characters::character::firearm::PhysicsWorld;
use pz_characters::character::state::StateWorld;
use pz_characters::character::{CharacterKind, GameCharacter};
use pz_fs::GameFs;
use pz_lua_api::bind_characters::LuaItem;
use pz_model::SkinningData;
use pz_render::TextureManager;
use pz_scripts::ScriptManager;
use pz_world::{GameRand, IsoCell, IsoMetaGrid, IsoSpriteManager};

use crate::iso_world::StandardRand;
use pz_world_states::{StatesEngine, StatesRuntime, StatesWorldAdapter, ThumpSettings};

use crate::sim::FrameSim;
use crate::world_vehicles::SharedBullet;

fn synthetic_ragdoll_scripts() -> RagdollScripts {
    let mut text = String::from("module Base {\n");
    for part in 0..11 {
        text.push_str(&format!(
            "ragdoll part{part}_bodypartinfo {{\n\
                part = {part},\n\
                calculateLength = false,\n\
                radius = 0.12,\n\
                height = 0.20,\n\
                gap = 0.01,\n\
                shape = 0,\n\
                mass = 6.0,\n\
                offset = 0.0 0.0 0.0,\n\
            }}\n\
            ragdoll part{part}_bodydynamics {{\n\
                part = {part},\n\
                linearDamping = 0.1,\n\
                angularDamping = 0.2,\n\
                deactivationTime = 0.8,\n\
                linearSleepingThreshold = 0.8,\n\
                angularSleepingThreshold = 1.0,\n\
                friction = 0.6,\n\
                rollingFriction = 0.1,\n\
            }}\n"
        ));
    }
    text.push_str(
        r#"
            ragdoll pelvis_spine_constraint {
                joint = 0,
                constraintType = 0,
                constraintPartA = 0,
                constraintPartB = 1,
                constraintAxisA = 0.0 0.0 0.0,
                constraintAxisB = 0.0 0.0 0.0,
                constraintPositionOffsetA = 0.0 0.1 0.0,
                constraintPositionOffsetB = 0.0 -0.1 0.0,
                constraintLimit = 0.4 0.4 0.4,
                constraintLimitExtended = 0.6 0.6 0.6,
            }
            ragdoll pelvis_anchor {
                bone = 2,
                bodyPart = 0,
                reverse = false,
                original = true,
                enabled = true,
            }
        }
        "#,
    );
    let mut s = RagdollScripts::default();
    s.load_file(&text);
    s
}

fn synthetic_tpose_skeleton() -> (Rc<SkinningData>, Skeleton, AnimClip) {
    let bone_names = &pz_ragdoll::skeleton_bone::SKELETON_BONES;
    let n = bone_names.len();
    let parents: Vec<i32> = (0..n).map(|i| if i == 0 { -1 } else { (i - 1) as i32 }).collect();
    let ident = [
        [1.0, 0.0, 0.0, 0.0],
        [0.0, 1.0, 0.0, 0.0],
        [0.0, 0.0, 1.0, 0.0],
        [0.0, 0.0, 0.0, 1.0],
    ];
    let mut sd = SkinningData {
        bind_pose: vec![ident; n],
        inverse_bind_pose: vec![ident; n],
        bone_offset: vec![ident; n],
        skeleton_hierarchy: parents,
        ..Default::default()
    };
    for (i, &name) in bone_names.iter().enumerate() {
        sd.bone_indices.insert(name.into(), i as i32);
    }
    let mut kfs = Vec::new();
    let mut model_kfs = Vec::new();
    let mut names = Vec::new();
    for (i, &name) in bone_names.iter().enumerate() {
        for &t in &[0.0f32, 0.5f32] {
            kfs.push(Keyframe {
                bone: i as i32,
                time: t,
                position: Vector3f::new(0.0, i as f32 * 0.05, 0.0),
                rotation: Quaternion::IDENTITY,
                scale: Vector3f::ONE,
            });
            model_kfs.push(pz_model::Keyframe {
                bone: i as i32,
                bone_name: name.into(),
                time: t,
                position: [0.0, i as f32 * 0.05, 0.0],
                rotation: pz_model::Quat {
                    x: 0.0,
                    y: 0.0,
                    z: 0.0,
                    w: 1.0,
                },
                scale: [1.0; 3],
            });
            names.push(name.into());
        }
    }
    sd.animation_clips.insert(
        "Bob_TPose".into(),
        pz_model::AnimationClip {
            name: "Bob_TPose".into(),
            duration: 0.5,
            keep_last_frame: true,
            is_ragdoll: false,
            keyframes: model_kfs,
        },
    );
    let clip = AnimClip::new("Bob_TPose", 0.5, kfs, names, true, false);
    let sd_rc = Rc::new(sd);
    let skel = Skeleton::from_skinning_data(&sd_rc);
    (sd_rc, skel, clip)
}

#[test]
fn shared_bullet_drives_ragdoll_builder_and_ballistics_through_states_world() {
    let bullet = SharedBullet::new();
    bullet
        .0
        .borrow_mut()
        .init_world(0, 0, 10, 10, 0, 0, false)
        .unwrap();

    let scripts = synthetic_ragdoll_scripts();
    assert_eq!(scripts.constraints.len(), 1);
    assert_eq!(scripts.anchors.len(), 1);
    assert_eq!(scripts.body_part_infos.len(), 11);
    assert_eq!(scripts.body_dynamics.len(), 11);
    scripts.to_bullet(&mut bullet.0.borrow_mut(), false);

    let (sd, skel, tpose) = synthetic_tpose_skeleton();
    let mut wr = WorldRagdolls {
        bullet: bullet.0.clone(),
        scripts,
        builder: None,
        builder_attempted: false,
        globals: RagdollGlobals::default(),
        debug: RagdollDebugOptions::default(),
    };
    wr.init_builder_with(&skel, &tpose);
    assert!(wr.has_builder());

    // Build an AnimationPlayer and verify pack_target_skeleton + freeze_corpse_ragdoll.
    let mut player = AnimationPlayer::new(sd);
    if let Some(tid) = player.play("Bob_TPose", true, false, -1.0) {
        player.track_mut(tid).unwrap().set_blend_weight(1.0);
    }
    player.update(0.016, None);
    let target_skel = pack_target_skeleton(&player);
    assert_eq!(target_skel.bone_count as usize, player.num_bones());
    assert_eq!(target_skel.packed.len(), pz_ragdoll::skeleton_bone::COUNT * 7);

    let bones = freeze_corpse_ragdoll(&player, true);
    assert!(bones.is_some());
    assert_eq!(bones.unwrap().len(), player.num_bones());
    assert!(freeze_corpse_ragdoll(&player, false).is_none());

    // Verify SharedBullet works as StateWorld::physics() through StatesWorldAdapter.
    struct TestEngine {
        bullet: SharedBullet,
        rand: StandardRand,
        skel: TargetSkeleton,
    }
    impl StatesEngine for TestEngine {
        fn rand(&mut self) -> &mut dyn GameRand {
            &mut self.rand
        }
        fn physics(&mut self) -> Option<&mut dyn PhysicsWorld> {
            Some(&mut self.bullet)
        }
        fn ballistics_skeleton(&self, _chr: i32) -> Option<TargetSkeleton> {
            Some(self.skel.clone())
        }
        fn ragdoll_pelvis(&self, _chr: i32) -> Option<[f32; 3]> {
            Some([10.5, 12.5, 0.0])
        }
    }

    let mut cell = IsoCell::default();
    let meta = IsoMetaGrid::empty();
    let mut sprites = IsoSpriteManager::new(pz_world::sprite::test_tables());
    let mut runtime = StatesRuntime::default();
    let mut engine = TestEngine {
        bullet: bullet.clone(),
        rand: StandardRand,
        skel: target_skel,
    };
    let mut world = StatesWorldAdapter::new(
        &mut cell,
        &meta,
        &mut sprites,
        ThumpSettings::default(),
        &mut runtime,
        &mut engine,
    );

    assert_eq!(
        StateWorld::<LuaItem>::ragdoll_pelvis(&world, 7),
        Some([10.5, 12.5, 0.0])
    );
    assert!(StateWorld::<LuaItem>::ballistics_skeleton(&world, 7).is_some());
    {
        let phys = StateWorld::<LuaItem>::physics(&mut world).expect("physics installed");
        let added = phys.add_ballistics_target(7);
        assert_eq!(added, 0);
        let removed = phys.remove_ballistics_target(7);
        assert_eq!(removed, 0);
    }

    // Verify GameCharacter::update_ballistics_target uses the shared Bullet world cleanly.
    let sm = ScriptManager::new();
    let mut zombie = GameCharacter::<LuaItem>::new(&sm, CharacterKind::Zombie);
    zombie.id = 42;
    zombie.vars.x = 10.5;
    zombie.vars.y = 12.5;
    zombie.vars.z = 0.0;
    zombie.ensure_ballistics_target();
    let sim = FrameSim::capture();
    zombie.update_ballistics_target(&sim, &mut world);
    zombie.release_ballistics_target(&mut world);

    wr.reset();
    assert_eq!(active_ragdoll_simulations(), 0);
    bullet.0.borrow_mut().destroy_world();
}

#[test]
#[ignore = "reads installed Build 42.22 ragdoll scripts and Bob_TPose via PZ_GAME_DIR"]
fn installed_build42_ragdoll_scripts_and_tpose_init_in_bullet() {
    let game_dir = PathBuf::from(
        std::env::var("PZ_GAME_DIR")
            .expect("set PZ_GAME_DIR to the installed Project Zomboid Java/media root"),
    );
    let bullet = SharedBullet::new();
    bullet
        .0
        .borrow_mut()
        .init_world(0, 0, 20, 20, 0, 0, false)
        .unwrap();

    let mut wr = WorldRagdolls::init(&game_dir, bullet.0.clone());
    println!(
        "Installed Build 42.22 ragdoll scripts: constraints={}, anchors={}, body_part_infos={}, body_dynamics={}",
        wr.scripts.constraints.len(),
        wr.scripts.anchors.len(),
        wr.scripts.body_part_infos.len(),
        wr.scripts.body_dynamics.len(),
    );
    assert!(!wr.scripts.constraints.is_empty());
    assert!(!wr.scripts.anchors.is_empty());
    assert!(!wr.scripts.body_part_infos.is_empty());
    assert!(!wr.scripts.body_dynamics.is_empty());

    let fs = Rc::new(RefCell::new(GameFs::new(&game_dir)));
    let tm = Rc::new(RefCell::new(TextureManager::new()));
    let assets = Assets::load(&game_dir, &fs, &tm).expect("load installed character media index");
    assert!(
        wr.ensure_builder(&assets),
        "expected Skinned/MaleBody + Bob/Bob_TPose to initialize RagdollBuilder"
    );
    let builder = wr.builder.as_ref().unwrap();
    assert!(builder.pelvis_bone.is_some());
    assert!(builder.head_bone.is_some());
    assert!(builder.left_shoulder.is_some());
    assert!(builder.right_shoulder.is_some());

    wr.reset();
    bullet.0.borrow_mut().destroy_world();
}
