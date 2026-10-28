use super::*;

#[derive(Default)]
struct Host {
    failed: bool,
    controller: bool,
    fail_allocation: bool,
    fail_movement: bool,
    calls: Vec<&'static str>,
}
impl RagdollHost for Host {
    fn host_action_failed(&self) -> bool {
        self.failed
    }
    fn has_character(&self) -> bool {
        true
    }
    fn can_ragdoll(&self) -> bool {
        true
    }
    fn has_controller(&self) -> bool {
        self.controller
    }
    fn alloc_controller(&mut self) {
        self.calls.push("alloc/on_started");
        self.controller = true;
        self.failed = self.fail_allocation;
    }
    fn release_controller(&mut self) {
        self.calls.push("release");
        self.controller = false;
    }
    fn is_initialized(&self) -> bool {
        true
    }
    fn is_first_frame(&self) -> bool {
        false
    }
    fn is_simulation_active(&self) -> bool {
        true
    }
    fn is_simulation_direction_calculated(&self) -> bool {
        true
    }
    fn desired_character_position(&self) -> Vector3f {
        Vector3f::ZERO
    }
    fn calculated_simulation_direction_angle(&self) -> f32 {
        0.0
    }
    fn update(&mut self, _player: &mut AnimationPlayer, _dt: f32) {
        self.calls.push("later physics");
    }
    fn post_update(&mut self, _player: &mut AnimationPlayer, _dt: f32) {
        self.calls.push("later post physics");
    }
    fn character_position(&self) -> Vector3f {
        Vector3f::ZERO
    }
    fn do_deferred_movement_from_ragdoll(&mut self, _player: &AnimationPlayer, _m: Vector3f) {
        self.calls.push("movement");
        self.failed = self.fail_movement;
    }
    fn game_time_multiplier(&self) -> f32 {
        1.0
    }
}

fn player() -> (AnimationPlayer, TrackId) {
    let identity = Matrix4f::IDENTITY.to_mat4();
    let mut skinning = SkinningData {
        bind_pose: vec![identity; 3],
        inverse_bind_pose: vec![identity; 3],
        bone_offset: vec![identity; 3],
        skeleton_hierarchy: vec![-1, 0, 1],
        ..Default::default()
    };
    for (i, name) in ["Root", "Bip01", "Tip"].iter().enumerate() {
        skinning.bone_indices.insert((*name).into(), i as i32);
    }
    let mut player = AnimationPlayer::new(Rc::new(skinning));
    let track = player.play("Ragdoll", false, true, 5.0).unwrap();
    player.track_mut(track).unwrap().set_blend_weight(1.0);
    (player, track)
}

#[test]
fn on_started_host_fault_preserves_allocated_controller_and_stops_physics_and_track_time() {
    let (mut player, track) = player();
    let mut host = Host {
        fail_allocation: true,
        ..Default::default()
    };
    player.update_with_ragdoll(0.25, None, &mut host);
    assert!(host.failed && host.controller);
    assert_eq!(host.calls, ["alloc/on_started"]);
    assert_eq!(player.track(track).unwrap().current_time_value(), 0.0);
    assert!(player.bone_transforms_need_first_frame);
    player.update_with_ragdoll(0.25, None, &mut host);
    assert_eq!(
        host.calls,
        ["alloc/on_started"],
        "failed action is never resumed implicitly"
    );
}

#[test]
fn deferred_movement_host_fault_stops_later_physics_and_animation_steps() {
    let (mut player, track) = player();
    player.bone_transforms_need_first_frame = false;
    player.live_entries.entries = vec![LiveEntry {
        track: 0,
        id: track,
        weight: 1.0,
        priority: 0,
        layer: 0,
    }];
    let mut host = Host {
        controller: true,
        fail_movement: true,
        ..Default::default()
    };
    player.update_with_ragdoll(0.25, None, &mut host);
    assert!(host.failed && host.controller);
    assert_eq!(host.calls, ["movement"]);
    assert_eq!(player.track(track).unwrap().current_time_value(), 0.0);
}
