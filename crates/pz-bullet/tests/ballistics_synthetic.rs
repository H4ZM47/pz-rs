//! Synthetic checks of the ballistics natives: the C `rand()` sequence, a
//! shooter hitting a target with a made-up skeleton, camera targets with body
//! parts, the pools and the JNI error cases.

use pz_bullet::pz::ballistics::CRand;
use pz_bullet::{JniError, PzBullet};

#[test]
fn c_rand_is_the_minimal_standard_generator() {
    let mut r = CRand::default();
    let first: Vec<i32> = (0..5).map(|_| r.rand()).collect();
    assert_eq!(first, [16807, 282475249, 1622650073, 984943658, 1144108930]);
    // 16807^n mod (2^31 - 1) after 10000 draws
    let mut x: u64 = 1;
    for _ in 0..10000 {
        x = x * 16807 % 0x7fff_ffff;
    }
    let mut r = CRand::default();
    let mut v = 0;
    for _ in 0..10000 {
        v = r.rand();
    }
    assert_eq!(v as u64, x);
}

/// A 35-bone chain standing up the y axis, 11 capsule body parts each
/// anchored to its bone (after `initWorld`, which sets up the bone pairs).
fn define_ragdoll(b: &mut PzBullet) {
    let mut info = Vec::new();
    for part in 0..11 {
        // part, calculateLength, radius, height, gap, shape, mass, offset
        info.extend_from_slice(&[part as f32, 1.0, 0.08, 0.3, 0.02, 0.0, 0.09, 0.0, 0.0, 0.0]);
    }
    assert_eq!(
        b.define_ragdoll_body_part_info(Some(&info), false),
        Ok(true)
    );
    // ten cone-twist joints chaining the parts
    let mut joints = Vec::new();
    for j in 0..10 {
        joints.extend_from_slice(&[j as f32, 5.0, j as f32, (j + 1) as f32]);
        joints.extend_from_slice(&[0.0; 12]);
        joints.extend_from_slice(&[0.5, 0.5, 0.5, 0.5, 0.5, 0.5]);
    }
    assert_eq!(b.define_ragdoll_constraints(Some(&joints), false), Ok(true));
    let mut dynamics = Vec::new();
    for part in 0..11 {
        dynamics.extend_from_slice(&[part as f32, 0.05, 0.9, 0.8, 1.6, 2.5, 1.5, 0.5]);
    }
    assert_eq!(
        b.define_ragdoll_body_dynamics(Some(&dynamics), false),
        Ok(true)
    );
    let bones = [2, 4, 6, 20, 21, 24, 25, 8, 9, 14, 15];
    let mut anchors = Vec::new();
    for bone in 0..35 {
        let part = bones.iter().position(|&x| x == bone);
        anchors.extend_from_slice(&[
            bone as f32,
            part.unwrap_or(0) as f32,
            0.0,
            0.0,
            if part.is_some() { 1.0 } else { 0.0 },
        ]);
    }
    assert_eq!(b.define_ragdoll_anchors(Some(&anchors), false), Ok(true));
    let hierarchy: Vec<i32> = (0..35).map(|i| i - 1).collect();
    b.initialize_ragdoll_skeleton(35, &hierarchy);
    let mut pose = Vec::new();
    for i in 0..35 {
        let y = if i == 0 { 0.0 } else { 0.05 };
        pose.extend_from_slice(&[0.0, y, 0.0, 0.0, 0.0, 0.0, 1.0]);
    }
    b.initialize_ragdoll_pose(35, &pose, 0.0, 0.0, 0.0, 1.0);
}

fn world() -> PzBullet {
    let mut b = PzBullet::new();
    b.init_pz_bullet();
    b.init_world(-2, -2, 2, 2, 0, 0, false).unwrap();
    define_ragdoll(&mut b);
    b
}

/// One frame of `BallisticsTarget.update`.
fn update_target(b: &mut PzBullet, id: i32, x: f32, y: f32) {
    b.set_ballistics_target_axis(id, 0.0, 0.0, 0.0);
    b.update_ballistics_target(id, x, 0.0, y, 0.0, 0.0, 0.0, 1.0, false)
        .unwrap();
    let mut data = Vec::new();
    for i in 0..35 {
        let y = if i == 0 { 0.0 } else { 0.05 };
        data.extend_from_slice(&[0.0, y, 0.0, 0.0, 0.0, 0.0, 1.0]);
    }
    b.update_ballistics_target_skeleton(id, 35, &data);
}

#[test]
fn muzzle_ray_hits_the_target_box() {
    let mut b = world();
    assert_eq!(b.add_ballistics_target(7), Ok(0));
    assert_eq!(b.add_ballistics_target(7), Ok(1));
    update_target(&mut b, 7, 5.0, 0.0);
    // the box's broadphase AABB is refreshed by the next step
    update_target(&mut b, 7, 5.0, 0.0);
    b.step_simulation(1.0 / 60.0, 1, 1.0 / 60.0, &mut pz_bullet::NoCallbacks)
        .unwrap();
    b.update_ballistics(1, 0.0, 0.6, 0.0); // range 0: the muzzle stays put
    b.set_ballistics_range(1, 20.0);
    b.update_ballistics(1, 0.0, 0.6, 0.0);
    b.update_ballistics_muzzle_aim_direction(1, 1.0, 0.0, 0.0);
    let mut out = [0f32; 80];
    let n = b.get_ballistics_targets(1, 20.0, 20, &mut out).unwrap();
    assert_eq!(n, 1);
    assert_eq!(out[0], 7.0);
    assert!(out[1] > 4.5 && out[1] < 5.0, "hit x {}", out[1]);
    // nothing in the way when aiming away
    b.update_ballistics_muzzle_aim_direction(1, -1.0, 0.0, 0.0);
    let mut out = [0f32; 80];
    assert_eq!(b.get_ballistics_targets(1, 20.0, 20, &mut out).unwrap(), 0);
    // nine spread rays, all reported
    b.update_ballistics_muzzle_aim_direction(1, 1.0, 0.0, 0.0);
    let mut out = [0f32; 36];
    assert_eq!(
        b.get_ballistics_targets_spread_data(1, 20.0, 0.5, 1.0, 9, 9, &mut out)
            .unwrap(),
        9
    );
    assert!(out.chunks(4).any(|c| c[0] == 7.0));
    assert!(out.chunks(4).all(|c| c[0] == 7.0 || c[0] == 0.0));
}

#[test]
fn camera_targets_and_body_parts() {
    let mut b = world();
    b.add_ballistics_target(3).unwrap();
    // the box follows the bodies one update late, and the broadphase one step late
    update_target(&mut b, 3, 4.0, 0.0);
    update_target(&mut b, 3, 4.0, 0.0);
    b.step_simulation(1.0 / 60.0, 1, 1.0 / 60.0, &mut pz_bullet::NoCallbacks)
        .unwrap();
    b.update_ballistics(9, 0.0, 0.0, 0.0);
    // looking along +x: -z of the reticle basis is +x
    let s = std::f32::consts::FRAC_1_SQRT_2;
    b.update_ballistics_aim_reticle_quaternion(9, 0.0, -s, 0.0, s);
    b.update_ballistics_aim_reticle_position(9, 4.0, 0.6, 0.0);
    b.set_ballistics_range(9, 10.0);
    b.update_ballistics(9, 0.0, 0.6, 0.0);
    assert_eq!(b.get_targeted_body_part(3), 11);
    let mut out = [-1f32; 50];
    let n = b
        .get_ballistics_camera_targets(9, 10.5, 10, true, &mut out)
        .unwrap();
    assert_eq!(n, 1);
    assert_eq!(out[0], 3.0);
    assert!(out[4] >= 0.0 && out[4] <= 11.0);
    assert_eq!(out[5], -1.0, "entries past the count are left alone");
    // sweep the reticle up the body until a camera ray hits a body part
    let mut part = b.get_targeted_body_part(3);
    let mut h = 0.1;
    while part == 11 && h < 1.5 {
        b.update_ballistics_aim_reticle_position(9, 4.0, h, 0.0);
        let mut out = [0f32; 50];
        let n = b
            .get_ballistics_camera_targets(9, 10.5, 10, true, &mut out)
            .unwrap();
        part = b.get_targeted_body_part(3);
        assert_eq!(n, 1);
        h += 0.05;
    }
    assert!(part < 11, "a body part was hit");
    // out of range of the muzzle
    let mut out = [0f32; 50];
    assert_eq!(
        b.get_ballistics_camera_targets(9, 1.0, 10, false, &mut out)
            .unwrap(),
        0
    );
}

#[test]
fn pools_and_errors() {
    let mut b = world();
    let mut empty: [f32; 0] = [];
    assert_eq!(
        b.get_ballistics_targets(1, 1.0, 1, &mut empty),
        Err(JniError::IllegalArgument("output array is too small"))
    );
    let mut out = [0f32; 4];
    assert_eq!(
        b.get_ballistics_targets(1, 1.0, 1, &mut out),
        Ok(0),
        "unknown shooter"
    );
    assert_eq!(b.remove_ballistics_target(5), 0, "unknown target");
    b.add_ballistics_target(5).unwrap();
    assert_eq!(b.remove_ballistics_target(5), 0);
    // the released object is reused for the next id, out of the world
    update_target(&mut b, 6, 1.0, 1.0);
    assert_eq!(b.remove_ballistics_target(6), 1);
    assert_eq!(b.get_targeted_body_part(6), 11);
    b.remove_ballistics(4).unwrap();
    b.destroy_world();
    assert_eq!(
        b.remove_ballistics(4),
        Err(JniError::NullPointer("WorldSimulation::instance is null"))
    );
    assert!(b.add_ballistics_target(1).is_err());
}
