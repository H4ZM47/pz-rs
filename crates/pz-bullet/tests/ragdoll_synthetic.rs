//! Synthetic checks of the capsule shape, the capsule-capsule collider, the
//! cone-twist constraint, the frame hinge constructor and the all-hits ray
//! callback.

use pz_bullet::bt::collision_object::*;
use pz_bullet::bt::collision_world::AllHitsRayResultCallback;
use pz_bullet::bt::dynamics::cone_twist::ConeTwistConstraint;
use pz_bullet::bt::dynamics::hinge::HingeConstraint;
use pz_bullet::bt::dynamics::rigid_body::RigidBodyConstructionInfo;
use pz_bullet::bt::dynamics::solver_body::SolverConstraint;
use pz_bullet::bt::dynamics::typed_constraint::*;
use pz_bullet::bt::dynamics::world::{DiscreteDynamicsWorld, WorldHooks};
use pz_bullet::bt::linear_math::*;
use pz_bullet::bt::shapes::*;

#[test]
fn capsule_dimensions_support_and_aabb() {
    let s = Shape::new_capsule(0.5, 2.0, 1);
    assert_eq!(s.shape_type(), CAPSULE_SHAPE_PROXYTYPE);
    let c = s.capsule();
    assert_eq!(c.get_radius(), 0.5);
    assert_eq!(c.get_half_height(), 1.0);
    // Without margin the support point sits `margin` inside the cap.
    let up = s.local_get_supporting_vertex_without_margin(&Vec3::new(0.0, 3.0, 0.0));
    assert!((up.y - (1.0 + 0.5 - CONVEX_DISTANCE_MARGIN)).abs() < 1e-15);
    let full = s.local_get_supporting_vertex(&Vec3::new(0.0, -1.0, 0.0));
    assert!((full.y + 1.5).abs() < 1e-15);
    let (mn, mx) = s.get_aabb(&Transform::IDENTITY);
    assert!((mx.y - (1.5 + CONVEX_DISTANCE_MARGIN)).abs() < 1e-15);
    assert!((mn.x + (0.5 + CONVEX_DISTANCE_MARGIN)).abs() < 1e-15);
    // X and Z capsules lie along their axis.
    let x = Shape::new_capsule(0.25, 1.0, 0);
    let (_, mx) = x.get_aabb(&Transform::IDENTITY);
    assert!(mx.x > mx.y && mx.x > mx.z);
    let z = Shape::new_capsule(0.25, 1.0, 2);
    let (_, mx) = z.get_aabb(&Transform::IDENTITY);
    assert!(mx.z > mx.x && mx.z > mx.y);
    // The inertia approximates the bounding box, so it is largest off-axis.
    let i = s.calculate_local_inertia(3.0);
    assert!(i.x > i.y && (i.x - i.z).abs() < 1e-12);
}

#[test]
fn capsule_margin_and_scaling_keep_the_outer_size() {
    let mut s = Shape::new_capsule(0.5, 2.0, 1);
    let before = s.get_aabb(&Transform::IDENTITY);
    s.set_margin(0.1);
    let after = s.get_aabb(&Transform::IDENTITY);
    // The radius absorbs the margin change (so does the half height).
    assert!((s.capsule().get_radius() + s.get_margin() - 0.54).abs() < 1e-12);
    assert!((before.1.x - after.1.x).abs() < 1e-12);
    s.set_local_scaling_convex(&Vec3::new(2.0, 2.0, 2.0));
    let scaled = s.get_aabb(&Transform::IDENTITY);
    assert!(scaled.1.y > after.1.y * 1.9);
}

struct NoHooks;
impl pz_bullet::bt::dispatch::ContactAddedHook for NoHooks {}
impl WorldHooks for NoHooks {}

fn add_body(w: &mut DiscreteDynamicsWorld, shape: ShapeId, mass: f64, t: Transform) -> ObjId {
    let inertia = if mass != 0.0 {
        w.cw.shapes.get(shape).calculate_local_inertia(mass)
    } else {
        Vec3::ZERO
    };
    let mut ci = RigidBodyConstructionInfo::new(mass, None, Some(shape), inertia);
    ci.start_world_transform = t;
    let id = w.create_rigid_body(&ci);
    w.add_rigid_body(id);
    id
}

#[test]
fn crossing_capsules_get_one_contact() {
    let mut w = DiscreteDynamicsWorld::new();
    let a = w.cw.shapes.add(Shape::new_capsule(0.2, 1.0, 1));
    let b = w.cw.shapes.add(Shape::new_capsule(0.2, 1.0, 0));
    let oa = add_body(&mut w, a, 0.0, Transform::IDENTITY);
    let mut tb = Transform::IDENTITY;
    tb.origin = Vec3::new(0.0, 0.0, 0.35);
    let ob = add_body(&mut w, b, 1.0, tb);
    w.step_simulation(0.01, 0, 0.0, &mut NoHooks);
    let d = &w.cw.dispatcher;
    let m = d
        .manifolds_ptr
        .iter()
        .map(|&m| d.manifold(m))
        .find(|m| m.cached_points > 0)
        .expect("a contact");
    assert_eq!(m.cached_points, 1);
    assert!(
        (m.body0 == oa && m.body1 == ob) || (m.body0 == ob && m.body1 == oa),
        "manifold bodies"
    );
    let p = m.get_contact_point(0);
    assert!(p.distance1 < 0.0 && p.distance1 > -0.1, "{}", p.distance1);
    assert!(p.normal_world_on_b.z.abs() > 0.99);
}

#[test]
fn all_hits_ray_reports_every_body() {
    let mut w = DiscreteDynamicsWorld::new();
    let b = w.cw.shapes.add(Shape::new_box(&Vec3::new(0.5, 0.5, 0.5)));
    for x in [-2.0, 0.0, 2.0] {
        let mut t = Transform::IDENTITY;
        t.origin = Vec3::new(x, 0.0, 0.0);
        add_body(&mut w, b, 0.0, t);
    }
    w.step_simulation(0.01, 0, 0.0, &mut NoHooks);
    let from = Vec3::new(-5.0, 0.1, 0.0);
    let to = Vec3::new(5.0, 0.1, 0.0);
    let mut cb = AllHitsRayResultCallback::new(from, to);
    w.cw.ray_test(&from, &to, &mut cb);
    assert_eq!(cb.collision_objects.len(), 3);
    assert_eq!(cb.base.closest_hit_fraction, 1.0);
    for (f, p) in cb.hit_fractions.iter().zip(&cb.hit_point_world) {
        assert!(*f > 0.0 && *f < 1.0);
        assert!(
            (p.x.abs() - 1.5).abs() < 1e-6
                || (p.x.abs() - 2.5).abs() < 1e-6
                || (p.x + 0.5).abs() < 1e-6
        );
    }
}

fn body(mass: f64, rot: Quat) -> CollisionObject {
    let mut ci = RigidBodyConstructionInfo::new(mass, None, None, Vec3::new(1.0, 1.0, 1.0));
    ci.start_world_transform = Transform::from_quat(&rot, Vec3::ZERO);
    CollisionObject::new_rigid_body(&ci)
}

fn rows(c: &mut TypedConstraint, objects: &Vec<Option<CollisionObject>>) -> Vec<SolverConstraint> {
    let mut info1 = ConstraintInfo1::default();
    c.get_info1(objects, &mut info1);
    let mut rows = vec![SolverConstraint::default(); info1.num_constraint_rows as usize];
    let mut info2 = ConstraintInfo2 {
        fps: 60.0,
        erp: 0.2,
        rows: &mut rows,
        damping: 1.0,
        num_iterations: 10,
    };
    c.get_info2(objects, &mut info2);
    rows
}

#[test]
fn cone_twist_adds_rows_past_its_limits() {
    let frames = (Transform::IDENTITY, Transform::IDENTITY);
    // Inside the cone: only the three point-to-point rows.
    let objects = vec![
        Some(body(1.0, Quat::IDENTITY)),
        Some(body(
            1.0,
            Quat::from_axis_angle(&Vec3::new(0.0, 1.0, 0.0), 0.2),
        )),
    ];
    let mut c = ConeTwistConstraint::new_typed(0, 1, frames.0, frames.1);
    if let ConstraintKind::ConeTwist(ct) = &mut c.kind {
        ct.set_limit(0.5, 0.5, 0.5, 1.0, flt(0.3), 1.0);
    }
    assert_eq!(rows(&mut c, &objects).len(), 3);
    // Swung past the cone and twisted past the twist span: five rows.
    let q = Quat::from_axis_angle(&Vec3::new(0.0, 1.0, 0.0), 0.9)
        * Quat::from_axis_angle(&Vec3::new(1.0, 0.0, 0.0), 0.8);
    let objects = vec![Some(body(1.0, Quat::IDENTITY)), Some(body(1.0, q))];
    let r = rows(&mut c, &objects);
    assert_eq!(r.len(), 5);
    let ConstraintKind::ConeTwist(ct) = &c.kind else {
        unreachable!()
    };
    assert!(ct.solve_swing_limit && ct.solve_twist_limit);
    assert!(ct.swing_correction > 0.0);
    // The swing row only pushes back (lower limit 0).
    assert_eq!(r[3].lower_limit, 0.0);
    assert_eq!(c.get_param(BT_CONSTRAINT_STOP_ERP, 5), flt(0.3));
}

#[test]
fn frame_hinge_uses_the_given_frames() {
    let mut fa = Transform::IDENTITY;
    fa.origin = Vec3::new(0.0, 0.5, 0.0);
    let c = HingeConstraint::new_typed_frames(0, 1, fa, Transform::IDENTITY, true);
    let ConstraintKind::Hinge(h) = &c.kind else {
        unreachable!()
    };
    assert_eq!(h.rb_a_frame.origin.y, 0.5);
    assert_eq!(h.reference_sign, -1.0);
    assert_eq!(c.object_type, HINGE_CONSTRAINT_TYPE);
}
