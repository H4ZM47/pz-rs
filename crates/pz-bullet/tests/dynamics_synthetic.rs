//! Synthetic checks of the rigid-body integrator and the constraint solver.

use pz_bullet::bt::collision_object::*;
use pz_bullet::bt::dynamics::point2point::Point2PointConstraint;
use pz_bullet::bt::dynamics::rigid_body::RigidBodyConstructionInfo;
use pz_bullet::bt::dynamics::sequential_impulse::SequentialImpulseConstraintSolver;
use pz_bullet::bt::dynamics::solver_body::ContactSolverInfo;
use pz_bullet::bt::dynamics::typed_constraint::ConstraintArena;
use pz_bullet::bt::linear_math::*;
use pz_bullet::bt::manifold::{ManifoldPoint, PersistentManifold};

fn body(mass: f64, pos: Vec3) -> CollisionObject {
    let mut ci =
        RigidBodyConstructionInfo::new(mass, None, Some(0), Vec3::new(1.0, 1.0, 1.0) * mass);
    ci.start_world_transform.origin = pos;
    let mut o = CollisionObject::new_rigid_body(&ci);
    o.set_gravity(&Vec3::new(0.0, -10.0, 0.0));
    o
}

#[test]
fn free_fall_integrates_gravity() {
    let mut o = body(2.0, Vec3::new(0.0, 10.0, 0.0));
    o.apply_gravity();
    o.integrate_velocities(0.01);
    assert!((o.rb().linear_velocity.y + 0.1).abs() < 1e-12);
    let t = o.predict_integrated_transform(0.01);
    o.proceed_to_transform(&t);
    assert!(o.world_transform.origin.y < 10.0);
}

#[test]
fn contact_stops_approaching_body() {
    let mut objs = vec![body(0.0, Vec3::ZERO), body(1.0, Vec3::new(0.0, 1.0, 0.0))];
    objs[1].set_linear_velocity(&Vec3::new(0.0, -2.0, 0.0));
    let mut m = PersistentManifold::new(0, 1, 0.02, BT_LARGE_FLOAT);
    let mut p = ManifoldPoint::new(
        Vec3::new(0.0, 0.5, 0.0),
        Vec3::new(0.0, -0.5, 0.0),
        Vec3::new(0.0, 1.0, 0.0),
        -0.001,
    );
    p.position_world_on_a = Vec3::new(0.0, 0.5, 0.0);
    p.position_world_on_b = Vec3::new(0.0, 0.5, 0.0);
    // body0 = static ground (A), body1 = falling box (B): normal on B points up.
    p.normal_world_on_b = Vec3::new(0.0, -1.0, 0.0);
    m.add_manifold_point(&p);
    let mut mans = vec![m];
    let mut cons = ConstraintArena::default();
    let mut solver = SequentialImpulseConstraintSolver::new();
    let info = ContactSolverInfo {
        time_step: 0.01,
        ..Default::default()
    };
    solver.solve_group(&mut objs, &[1], &mut mans, &[0], &mut cons, &[], &info);
    assert!(
        objs[1].rb().linear_velocity.y > -0.1,
        "{:?}",
        objs[1].rb().linear_velocity
    );
    assert!(mans[0].get_contact_point(0).applied_impulse > 0.0);
}

#[test]
fn point_to_point_pulls_bodies_together() {
    let mut objs = vec![body(1.0, Vec3::ZERO), body(1.0, Vec3::new(3.0, 0.0, 0.0))];
    let mut cons = ConstraintArena::default();
    let c = cons.insert(Point2PointConstraint::new_typed(
        0,
        1,
        Vec3::new(1.0, 0.0, 0.0),
        Vec3::new(-1.0, 0.0, 0.0),
    ));
    let mut solver = SequentialImpulseConstraintSolver::new();
    let info = ContactSolverInfo {
        time_step: 0.01,
        ..Default::default()
    };
    let mut mans: Vec<PersistentManifold> = Vec::new();
    solver.solve_group(&mut objs, &[0, 1], &mut mans, &[], &mut cons, &[c], &info);
    assert!(objs[0].rb().linear_velocity.x > 0.0);
    assert!(objs[1].rb().linear_velocity.x < 0.0);
}
