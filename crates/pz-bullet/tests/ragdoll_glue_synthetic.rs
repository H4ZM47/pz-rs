//! Synthetic checks of the ragdoll glue (`api_ragdoll`, `pz::ragdoll*`):
//! a made-up 35-bone skeleton and ragdoll definitions (not the game's).

use pz_bullet::pz::ragdoll::{BODY_PART_COUNT, JOINT_COUNT};
use pz_bullet::pz::ragdoll_script::{round_index, RagdollScript};
use pz_bullet::{NoCallbacks, PzBullet, SkeletonBoneCallbacks};

const BONES: usize = 35;

/// A chain-ish hierarchy: every bone hangs off an earlier one.
fn hierarchy() -> Vec<i32> {
    (0..BONES as i32)
        .map(|i| if i == 0 { -1 } else { (i - 1) / 2 })
        .collect()
}

/// Bind pose: each bone 0.1 below its parent, no rotation.
fn tpose() -> Vec<f32> {
    let mut p = Vec::new();
    for i in 0..BONES {
        let y = if i == 0 { 0.0 } else { -0.1 };
        p.extend_from_slice(&[0.0, y, 0.02 * (i % 3) as f32, 0.0, 0.0, 0.0, 1.0]);
    }
    p
}

fn define(b: &mut PzBullet) {
    let mut parts = Vec::new();
    for part in 0..BODY_PART_COUNT {
        let shape = (part % 3) as f32; // capsule, box, sphere
        let calc = if part % 2 == 0 { 1.0 } else { 0.0 };
        parts.extend_from_slice(&[
            part as f32,
            calc,
            0.05,
            0.2,
            0.01,
            shape,
            0.09,
            0.0,
            0.01,
            0.0,
        ]);
    }
    let mut dynamics = Vec::new();
    for part in 0..BODY_PART_COUNT {
        dynamics.extend_from_slice(&[part as f32, 0.05, 0.8, 0.8, 1.6, 2.5, 1.5, 0.5]);
    }
    let mut cons = Vec::new();
    for j in 0..JOINT_COUNT {
        let kind = if j % 3 == 0 { 4.0 } else { 5.0 };
        let (a, c) = (j as f32 / 2.0, (j + 1) as f32);
        cons.extend_from_slice(&[j as f32, kind, a.floor(), c, 0.0, 1.5, 0.0, 0.0, 1.5, 0.0]);
        cons.extend_from_slice(&[
            0.0, 0.05, 0.0, 0.0, -0.05, 0.0, 0.7, 0.5, 0.3, 0.0, 0.0, 0.0,
        ]);
    }
    let mut anchors = Vec::new();
    for bone in 0..BONES {
        let part = (bone % BODY_PART_COUNT) as f32;
        anchors.extend_from_slice(&[
            bone as f32,
            part,
            (bone % 2) as f32,
            (bone % 5 == 0) as u8 as f32,
            1.0,
        ]);
    }
    assert_eq!(
        b.define_ragdoll_body_dynamics(Some(&dynamics), false),
        Ok(true)
    );
    assert_eq!(b.define_ragdoll_constraints(Some(&cons), false), Ok(true));
    assert_eq!(b.define_ragdoll_anchors(Some(&anchors), false), Ok(true));
    assert_eq!(
        b.define_ragdoll_body_part_info(Some(&parts), false),
        Ok(true)
    );
}

fn world_with_ragdoll() -> PzBullet {
    let mut b = PzBullet::new();
    b.init_pz_bullet();
    define(&mut b);
    b.init_world(-1, -1, 1, 1, -256, -256, false).unwrap();
    b.initialize_ragdoll_skeleton(BONES as i32, &hierarchy());
    b.initialize_ragdoll_pose(BONES as i32, &tpose(), 0.0, 0.0, 0.0, 1.0);
    b.add_ragdoll(7, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0).unwrap();
    b.set_ragdoll_local_transform_rotation(7, 0.0, 0.0, 0.0, 1.0);
    b.update_ragdoll(7, 10.0, 3.0, 12.0, 0.0, 0.0, 0.0, 1.0);
    b.update_ragdoll_skeleton_transforms(7, BONES as i32, &tpose());
    b
}

#[test]
fn parsers_round_and_index_by_key() {
    assert_eq!(round_index(2.4), 2);
    assert_eq!(round_index(2.5), 3);
    assert_eq!(round_index(-1.5), -2);
    let mut s = RagdollScript::new();
    // A record for part 3 lands at index 3; part 1 stays undefined.
    s.body_part_info_from_java(&[3.0, 1.0, 0.1, 0.2, 0.3, 1.0, 0.5, 0.0, 0.0, 0.0]);
    assert_eq!(s.get_body_part_info(3).unwrap().shape, 1);
    assert!(s.get_body_part_info(1).is_none());
    // Records that do not fit are not written.
    s.body_dynamics_from_java(&[4.0, 0.1, 0.2]);
    assert!(s.get_ragdoll_body_dynamics(4).is_none());
}

#[test]
fn bodies_fall_and_skeleton_follows() {
    let mut b = world_with_ragdoll();
    let mut skel = vec![0f32; BONES * 7];
    let mut rb = vec![0f32; BODY_PART_COUNT * 7];
    assert_eq!(
        b.simulate_ragdoll_with_rigid_body_output(7, &mut skel, &mut rb),
        Ok(BONES as i32)
    );
    let y0 = rb[1];
    b.set_ragdoll_active(7, true);
    {
        let ws = b.world.as_deref().unwrap();
        assert_eq!(ws.dw.constraint_list.len(), JOINT_COUNT);
    }
    for _ in 0..30 {
        b.step_simulation(0.01, 0, 0.0, &mut NoCallbacks).unwrap();
    }
    b.simulate_ragdoll_with_rigid_body_output(7, &mut skel, &mut rb)
        .unwrap();
    assert!(rb[1] < y0 - 0.02, "pelvis fell: {} -> {}", y0, rb[1]);
    assert!(skel.iter().all(|v| v.is_finite()));
    assert_eq!(b.get_ragdoll_simulation_state(7), 1);
    assert_eq!(b.get_ragdoll_simulation_state(8), -1);
    let mut c = [0f32; 3];
    b.get_corrected_world_space(7, &mut c).unwrap();
    assert_eq!(c, [10.0 + 256.0, 3.0, 12.0 + 256.0]);
}

#[test]
fn removed_ragdolls_are_pooled_with_their_mass() {
    let mut b = world_with_ragdoll();
    b.set_ragdoll_active(7, true);
    b.set_ragdoll_mass(50.0);
    b.remove_ragdoll(7).unwrap();
    {
        let ws = b.world.as_deref().unwrap();
        assert!(ws.ragdoll.ragdolls.is_empty());
        assert_eq!(ws.ragdoll.pool.len(), 1);
        assert!(ws.dw.constraint_list.is_empty());
        assert_eq!(ws.ragdoll.pool[0].mass, 50.0);
    }
    b.add_ragdoll(9, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0).unwrap();
    let ws = b.world.as_deref().unwrap();
    assert!(ws.ragdoll.pool.is_empty());
    let r = &ws.ragdoll.ragdolls[&9];
    assert_eq!((r.id, r.mass, r.active), (9, 50.0, false));
}

#[test]
fn argument_errors_and_first_call_quirk() {
    let mut b = PzBullet::new();
    let mut skel = vec![0f32; 7];
    // The first call's log clears the exception: Java sees -1.
    assert_eq!(b.simulate_ragdoll(1, &mut skel), Ok(-1));
    assert!(b.simulate_ragdoll(1, &mut skel).is_err());
    assert!(b.add_ragdoll(1, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0).is_err());
    assert!(b.define_ragdoll_anchors(None, false).is_err());
    let mut b = world_with_ragdoll();
    assert!(b.simulate_ragdoll(7, &mut []).is_err());
    assert_eq!(b.simulate_ragdoll(99, &mut skel), Ok(0));
}

struct Names;
impl SkeletonBoneCallbacks for Names {
    fn get_bone_name(&mut self, ordinal: i32) -> Option<String> {
        Some(format!("bone{ordinal}"))
    }
    fn get_bone_ordinal(&mut self, name: &str) -> i32 {
        name[4..].parse().unwrap_or(-1)
    }
}

#[test]
fn bone_validation_uses_the_callbacks() {
    let mut b = PzBullet::new();
    assert!(b.init_pz_bullet_with_bones(&mut Names));
}
