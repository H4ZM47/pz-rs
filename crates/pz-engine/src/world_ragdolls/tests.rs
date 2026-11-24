use super::*;

use pz_anim::animator::AdvancedAnimator;
use pz_anim::clip::{AnimClip, Keyframe};
use pz_anim::math::{Quaternion, Vector3f};
use pz_anim::skeleton::Skeleton;
use pz_anim::{AnimationVariableSource, VariableValue};
use pz_characters::character::firearm::PhysicsWorld;
use pz_characters::character::state::{StateCx, StateWorld};
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
use crate::world_models::WorldModels;
use crate::world_update::{EngineStates, PlayerFirearmSnapshot};
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

fn synthetic_tpose_skeleton_with_extra(extra_bones: &[&str]) -> (Rc<SkinningData>, Skeleton, AnimClip) {
    let mut bone_names: Vec<&str> = pz_ragdoll::skeleton_bone::SKELETON_BONES.to_vec();
    bone_names.extend_from_slice(extra_bones);
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

    let (sd, skel, tpose) = synthetic_tpose_skeleton_with_extra(&["ExtraAttach1", "ExtraAttach2"]);
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

    // Build an AnimationPlayer with > COUNT skinning bones and verify
    // pack_target_skeleton reports `COUNT` (matching `packed` / Bullet) rather
    // than raw `player.num_bones()`, and freeze_corpse_ragdoll works.
    let mut player = AnimationPlayer::new(sd.clone());
    assert_eq!(player.num_bones(), pz_ragdoll::skeleton_bone::COUNT + 2);
    if let Some(tid) = player.play("Bob_TPose", true, false, -1.0) {
        let track = player.track_mut(tid).unwrap();
        track.set_blend_weight(1.0);
        track.anim_layer = Some(0);
        track.track_time_to_variable = Some("CustomTrackFrac".into());
    }
    player.update(0.016, None);
    let target_skel = pack_target_skeleton(&player);
    assert_eq!(target_skel.bone_count as usize, pz_ragdoll::skeleton_bone::COUNT);
    assert_eq!(target_skel.packed.len(), pz_ragdoll::skeleton_bone::COUNT * 7);

    let bones = freeze_corpse_ragdoll(&player, true);
    assert!(bones.is_some());
    assert_eq!(bones.unwrap().len(), player.num_bones());
    assert!(freeze_corpse_ragdoll(&player, false).is_none());

    // Exercise `EngineStates` directly with a mutably-borrowed local player
    // (mirroring `update_characters` -> `advance_player_update`), verifying
    // `muzzle_attachment`, `aim_reticle_iso`, `ballistics_skeleton`,
    // `update_ballistics_target`, and `update_player_with_ragdoll`
    // (including `m_TrackTimeToVariable` preservation through `ChrVarProxy`).
    let sm = ScriptManager::new();
    let chars = pz_characters::Characters::default();
    let sim = FrameSim::capture();

    let shooter_rc = Rc::new(RefCell::new(GameCharacter::<LuaItem>::new(
        &sm,
        CharacterKind::Player,
    )));
    {
        let mut s = shooter_rc.borrow_mut();
        s.id = 77;
        s.vars.x = 123.5;
        s.vars.y = 245.5;
        s.vars.z = 0.0;
    }
    pz_lua_api::bind_players::set_player(0, Some(shooter_rc.clone()));
    let mut models = WorldModels::new(
        Path::new("unused-synthetic-root"),
        Rc::new(RefCell::new(GameFs::new("unused-synthetic-root"))),
        Rc::new(RefCell::new(TextureManager::new())),
    );
    let mut shooter_player = AnimationPlayer::new(sd.clone());
    if let Some(tid) = shooter_player.play("Bob_TPose", true, false, -1.0) {
        shooter_player.track_mut(tid).unwrap().set_blend_weight(1.0);
    }
    shooter_player.update(0.016, None);
    models.insert_test_animation(77, shooter_player, AdvancedAnimator::new());
    models.set_test_primary_hand_muzzle(77, [0.0, 0.35, 0.0]);

    let snap = PlayerFirearmSnapshot {
        id: 77,
        pos: [123.5, 245.5, 0.0],
        reticle_iso: (130.25, 250.75),
    };
    let mut cell = IsoCell::default();
    let meta = IsoMetaGrid::empty();
    let mut sprites = IsoSpriteManager::new(pz_world::sprite::test_tables());
    let mut runtime = StatesRuntime::default();
    {
        let mut engine = EngineStates {
            geometry: None,
            sim: Some(&sim),
            closing_windows: Vec::new(),
            rand: StandardRand,
            movers: Vec::new(),
            models: RefCell::new(Some(&mut models)),
            animation_time_delta: 0.016,
            floor_world: None,
            bullet: Some(bullet.clone()),
            player_firearms: vec![snap],
        };
        let mut world = StatesWorldAdapter::new(
            &mut cell,
            &meta,
            &mut sprites,
            ThumpSettings::default(),
            &mut runtime,
            &mut engine,
        );
        // Hold a live `borrow_mut` on the shooter just like `update_characters`.
        let _borrowed_shooter = shooter_rc.borrow_mut();
        let muzzle = StateWorld::<LuaItem>::muzzle_attachment(&world, 77)
            .expect("EngineStates::muzzle_attachment should resolve while shooter is borrowed");
        assert!(
            (muzzle.position[0] - 123.5).abs() < 2.0 && (muzzle.position[1] - 245.5).abs() < 2.0,
            "muzzle position should come from shooter snapshot, got {:?}",
            muzzle.position
        );
        assert_eq!(
            StateWorld::<LuaItem>::aim_reticle_iso(&world, 77),
            (130.25, 250.75),
            "aim_reticle_iso should use snapshot while shooter is borrowed"
        );
        let skel_from_engine = StateWorld::<LuaItem>::ballistics_skeleton(&world, 77)
            .expect("EngineStates::ballistics_skeleton should pack character skeleton");
        assert_eq!(
            skel_from_engine.bone_count as usize,
            pz_ragdoll::skeleton_bone::COUNT
        );
        drop(_borrowed_shooter);

        // Verify `update_player_with_ragdoll` preserves custom `m_TrackTimeToVariable`
        // values through `ChrVarProxy` even when the character is not ragdolling.
        let mut zombie = GameCharacter::<LuaItem>::new(&sm, CharacterKind::Zombie);
        zombie.id = 42;
        zombie.vars.x = 10.5;
        zombie.vars.y = 12.5;
        zombie.vars.z = 0.0;
        zombie.set_variable("CustomTrackFrac", VariableValue::Float(0.6));
        let mut controller = None;
        let cx = StateCx::new(&sm, &chars, &sim, &mut world);
        assert!(wr.update_player_with_ragdoll(
            &mut player,
            &mut controller,
            &mut zombie,
            cx,
            0.016,
            1.0,
        ));
        let track_time = player.tracks()[0].current_time_value();
        assert!(
            (track_time - 0.3).abs() < 1e-4,
            "expected track_time_to_variable (0.6 * 0.5 = 0.3), got {track_time}"
        );

        // Verify `GameCharacter::update_ballistics_target` initializes, registers
        // in Bullet, updates the 35-bone skeleton, and cleanly releases.
        zombie.ensure_ballistics_target();
        // Frame 1: initializes bone count from `target_skel` (via TestEngine below
        // or via `models` for id 77). Let's test on `shooter_rc` (which has a
        // model in `models` -> real `EngineStates::ballistics_skeleton` + Bullet).
        let mut shooter_for_target = shooter_rc.borrow_mut();
        shooter_for_target.ensure_ballistics_target().release_frame = 2;
        shooter_for_target.update_ballistics_target(&sim, &mut world);
        {
            let bt = shooter_for_target.ballistics_target.as_ref().unwrap();
            assert!(bt.is_initialized, "first update initializes BallisticsTarget");
            assert_eq!(bt.number_of_bones as usize, pz_ragdoll::skeleton_bone::COUNT);
            assert!(!bt.added_to_world);
        }
        shooter_for_target.update_ballistics_target(&sim, &mut world);
        {
            let bt = shooter_for_target.ballistics_target.as_ref().unwrap();
            assert!(bt.added_to_world, "second update adds target to Bullet and updates skeleton");
        }
        shooter_for_target.release_ballistics_target(&mut world);
        assert!(
            shooter_for_target.ballistics_target.is_none(),
            "release_ballistics_target should remove and clear BallisticsTarget"
        );
    }
    pz_lua_api::bind_players::set_player(0, None);

    // Also verify SharedBullet directly as StateWorld::physics() via TestEngine.
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
