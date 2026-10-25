//! Synthetic checks for the raycast vehicle (no recorded data).

use pz_bullet::bt::collision_object::CollisionObject;
use pz_bullet::bt::dynamics::raycast_vehicle::*;
use pz_bullet::bt::dynamics::rigid_body::RigidBodyConstructionInfo;
use pz_bullet::bt::linear_math::*;

/// Flat ground at y = 0.
struct Flat;
impl VehicleRaycaster for Flat {
    fn cast_ray(&mut self, from: &Vec3, to: &Vec3) -> Option<VehicleRaycasterResult> {
        if from.y > 0.0 && to.y <= 0.0 {
            let t = from.y / (from.y - to.y);
            Some(VehicleRaycasterResult {
                hit_point_in_world: *from + (*to - *from) * t,
                hit_normal_in_world: Vec3::new(0.0, 1.0, 0.0),
                dist_fraction: t,
                hit_object: 1,
            })
        } else {
            None
        }
    }
}

fn car() -> (Vec<CollisionObject>, RaycastVehicle) {
    let mut chassis = CollisionObject::new_rigid_body(&RigidBodyConstructionInfo::new(
        1000.0,
        None,
        None,
        Vec3::new(400.0, 500.0, 300.0),
    ));
    let mut t = Transform::IDENTITY;
    t.origin = Vec3::new(0.0, 0.8, 0.0);
    chassis.set_center_of_mass_transform(&t);
    let objs = vec![chassis];
    let tuning = VehicleTuning::default();
    let mut v = RaycastVehicle::new(&tuning, 0);
    v.set_coordinate_system(0, 1, 2);
    for (i, (x, z)) in [(0.8, 1.4), (-0.8, 1.4), (0.8, -1.4), (-0.8, -1.4)]
        .iter()
        .enumerate()
    {
        v.add_wheel(
            &objs[..],
            &Vec3::new(*x, -0.2, *z),
            &Vec3::new(0.0, -1.0, 0.0),
            &Vec3::new(-1.0, 0.0, 0.0),
            0.3,
            0.35,
            &tuning,
            i < 2,
        );
    }
    v.reset_suspension();
    (objs, v)
}

#[test]
fn wheels_touch_flat_ground_and_push_up() {
    let (mut objs, mut v) = car();
    v.update_vehicle(&mut objs[..], 0.01, &mut Flat);
    for w in &v.wheel_info {
        assert!(w.raycast_info.is_in_contact && w.raycast_info.ground_object);
        assert!(w.wheels_suspension_force > 0.0);
    }
    assert!(objs[0].rb().linear_velocity.y > 0.0);
}

#[test]
fn engine_force_accelerates_forward() {
    let (mut objs, mut v) = car();
    for i in 0..4 {
        v.apply_engine_force(2000.0, i);
    }
    for _ in 0..5 {
        v.update_vehicle(&mut objs[..], 0.01, &mut Flat);
    }
    assert!(objs[0].rb().linear_velocity.z > 0.0);
    assert!(v.current_vehicle_speed_km_hour > 0.0);
}

#[test]
fn no_ground_means_rest_length_and_no_force() {
    struct Miss;
    impl VehicleRaycaster for Miss {
        fn cast_ray(&mut self, _: &Vec3, _: &Vec3) -> Option<VehicleRaycasterResult> {
            None
        }
    }
    let (mut objs, mut v) = car();
    v.update_vehicle(&mut objs[..], 0.01, &mut Miss);
    for w in &v.wheel_info {
        assert!(!w.raycast_info.is_in_contact);
        assert_eq!(w.raycast_info.suspension_length, 0.3);
        assert_eq!(w.wheels_suspension_force, 0.0);
        assert_eq!(w.skid_info, 1.0);
    }
}
