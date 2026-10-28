use super::*;
use std::rc::Rc;

#[derive(Default)]
struct Body {
    failed: bool,
    starts: usize,
}
impl RagdollCharacter for Body {
    fn host_action_failed(&self) -> bool {
        self.failed
    }
    fn id(&self) -> i32 {
        7
    }
    fn position(&self) -> Vec3 {
        Vec3::default()
    }
    fn forward_direction_angle(&self) -> f32 {
        0.0
    }
    fn can_ragdoll(&self) -> bool {
        true
    }
    fn on_ragdoll_simulation_started(&mut self) {
        self.starts += 1;
        self.failed = true;
    }
    fn set_ragdoll_fall(&mut self, _value: bool) {
        panic!("later character callback after host fault")
    }
    fn set_use_physic_hit_reaction(&mut self, _value: bool) {
        panic!("later reset callback after host fault")
    }
    fn is_fall_on_front(&self) -> bool {
        false
    }
    fn set_fall_on_front(&mut self, _value: bool) {
        panic!("later front callback after host fault")
    }
    fn set_on_floor(&mut self, _value: bool) {
        panic!("later floor callback after host fault")
    }
    fn do_deferred_movement_from_ragdoll(&mut self, _player: &AnimationPlayer, _m: Vec3) {
        panic!("later movement callback after host fault")
    }
}

#[test]
fn allocation_callback_fault_preserves_controller_prefix_and_blocks_frame_and_controller_physics() {
    let mut bullet = PzBullet::new();
    // This fixture never starts Bullet: crossing the abort seam must prevent
    // controller initialization and every later physics/character callback.
    let builder = RagdollBuilder {
        bone_hierarchy: Vec::new(),
        bone_transform_data: Vec::new(),
        pelvis_bone: None,
        head_bone: None,
        left_shoulder: None,
        right_shoulder: None,
        mass: 0.0,
        vehicle_body_dynamics_params: [0.0; 8],
    };
    let mut body = Body::default();
    let mut globals = RagdollGlobals::default();
    let mut slot = None;
    let mut player = AnimationPlayer::new(Rc::new(pz_model::SkinningData::default()));
    let mut frame = RagdollFrame {
        env: RagdollEnv {
            bullet: &mut bullet,
            globals: &mut globals,
            builder: &builder,
            character: &mut body,
            bullet_frame_no: 0,
            time_delta: 0.25,
            debug: RagdollDebugOptions::default(),
        },
        controller: &mut slot,
        multiplier: 1.0,
    };
    frame.alloc_controller();
    assert!(frame.host_action_failed());
    assert!(
        frame.has_controller(),
        "native allocation prefix remains visible"
    );
    frame.update(&mut player, 0.25);
    frame.post_update(&mut player, 0.25);
    frame.release_controller();
    assert!(
        frame.has_controller(),
        "host abort does not roll back allocation"
    );
    // Direct controller entry points obey the same actual character carrier.
    let controller = frame.controller.as_mut().unwrap();
    controller.update(&mut player, &mut frame.env, 0.25);
    controller.post_update(&player, &mut frame.env, 0.25);
    controller.reinitialize(&mut player, &mut frame.env);
    assert!(!controller.is_initialized());
    assert_eq!(frame.env.globals.number_of_active_simulations, 0);
    drop(frame);
    assert_eq!(body.starts, 1);
    assert!(body.failed);
}
