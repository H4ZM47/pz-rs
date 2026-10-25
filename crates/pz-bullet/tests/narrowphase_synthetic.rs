//! Synthetic checks of the shape and narrowphase port (no oracle data).

use pz_bullet::bt::linear_math::*;
use pz_bullet::bt::narrowphase::box_box::box_box_get_closest_points;
use pz_bullet::bt::narrowphase::convex_cast::*;
use pz_bullet::bt::narrowphase::gjk_epa::*;
use pz_bullet::bt::narrowphase::gjk_pair_detector::*;
use pz_bullet::bt::narrowphase::simplex::VoronoiSimplexSolver;
use pz_bullet::bt::shapes::*;

struct Rec(Vec<(Vec3, Vec3, f64)>);
impl DiscreteResult for Rec {
    fn set_shape_identifiers_a(&mut self, _: i32, _: i32) {}
    fn set_shape_identifiers_b(&mut self, _: i32, _: i32) {}
    fn add_contact_point(&mut self, n: &Vec3, p: &Vec3, d: f64) {
        self.0.push((*n, *p, d));
    }
}

fn at(x: f64, y: f64, z: f64) -> Transform {
    Transform::new(Mat3::IDENTITY, Vec3::new(x, y, z))
}

#[test]
fn box_keeps_margin_inside_half_extents() {
    let b = Shape::new_box(&Vec3::new(1.0, 0.5, 2.0));
    assert_eq!(b.get_margin(), 0.04);
    let h = b.box_half_extents_with_margin();
    assert!((h.x - 1.0).abs() < 1e-15 && (h.z - 2.0).abs() < 1e-15);
    // thin box: safe margin is 0.1f * min dimension
    let t = Shape::new_box(&Vec3::new(1.0, 0.1, 1.0));
    assert_eq!(t.get_margin(), flt(0.1) * 0.1);
}

#[test]
fn sphere_gjk_distance_is_analytic() {
    let a = Shape::new_sphere(1.0);
    let b = Shape::new_sphere(0.5);
    let mut ss = VoronoiSimplexSolver::new();
    let mut g = GjkPairDetector::new(&a, &b, &mut ss, Some(PenetrationDepthSolver::GjkEpa));
    let input = ClosestPointInput {
        transform_a: at(0.0, 0.0, 0.0),
        transform_b: at(3.0, 0.0, 0.0),
        ..Default::default()
    };
    let mut pc = PointCollector::default();
    g.get_closest_points(&input, &mut pc);
    assert!(pc.has_result);
    assert!((pc.distance - 1.5).abs() < 1e-9, "{}", pc.distance);
    assert!((pc.normal_on_b_in_world.x + 1.0).abs() < 1e-9);
}

#[test]
fn overlapping_boxes_penetrate() {
    let a = Shape::new_box(&Vec3::new(1.0, 1.0, 1.0));
    let b = Shape::new_box(&Vec3::new(1.0, 1.0, 1.0));
    let mut res = GjkEpaResults::default();
    assert!(penetration(
        &a,
        &at(0.0, 0.0, 0.0),
        &b,
        &at(1.5, 0.2, 0.1),
        &Vec3::new(1.0, 0.0, 0.0),
        &mut res,
        true
    ));
    assert_eq!(res.status, GjkEpaStatus::Penetrating);
    assert!((res.distance + 0.5).abs() < 1e-4, "{}", res.distance);
}

#[test]
fn box_box_face_contact_reports_four_points() {
    let a = Shape::new_box(&Vec3::new(2.0, 0.5, 2.0));
    let b = Shape::new_box(&Vec3::new(0.5, 0.5, 0.5));
    let input = ClosestPointInput {
        transform_a: at(0.0, 0.0, 0.0),
        transform_b: at(0.1, 0.95, -0.2),
        ..Default::default()
    };
    let mut rec = Rec(vec![]);
    box_box_get_closest_points(&a, &b, &input, &mut rec);
    assert_eq!(rec.0.len(), 4);
    for (n, _, d) in &rec.0 {
        assert!((n.y - -1.0).abs() < 1e-12 || (n.y - 1.0).abs() < 1e-12);
        assert!((d + 0.05).abs() < 1e-9, "{d}");
    }
}

#[test]
fn ray_hits_box_with_gjk_cast() {
    let mut point = Shape::new_sphere(0.0);
    point.set_margin(0.0);
    let b = Shape::new_box(&Vec3::new(1.0, 1.0, 1.0));
    let mut ss = VoronoiSimplexSolver::new();
    let mut cr = CastResult {
        fraction: 1.0,
        ..Default::default()
    };
    let hit = gjk_convex_cast(
        &point,
        &b,
        &mut ss,
        &at(0.0, 5.0, 0.0),
        &at(0.0, -5.0, 0.0),
        &at(0.0, 0.0, 0.0),
        &at(0.0, 0.0, 0.0),
        &mut cr,
    );
    assert!(hit);
    assert!((cr.fraction - 0.4).abs() < 1e-3, "{}", cr.fraction);
}

#[test]
fn compound_aabb_covers_children() {
    let mut shapes = Shapes::new();
    let c = shapes.add(Shape::new_compound(true));
    let b = shapes.add(Shape::new_box(&Vec3::new(1.0, 0.5, 2.0)));
    let s = shapes.add(Shape::new_sphere(0.3));
    shapes.compound_add_child_shape(c, &at(0.0, 1.0, 0.0), b);
    shapes.compound_add_child_shape(c, &at(0.0, 0.0, 3.0), s);
    let (mn, mx) = shapes.get(c).get_aabb(&Transform::IDENTITY);
    assert!(mn.y <= 0.5 - 1e-12 + 0.0 && mx.z >= 3.3 - 1e-12);
    assert_eq!(shapes.get(c).compound().update_revision, 3);
}

#[test]
fn convex_hull_support_picks_extreme_point() {
    let pts = [0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 2.0, 0.0, 0.0, 0.0, 3.0];
    let h = Shape::new_convex_hull(&pts, 4, 3);
    let s = h.local_get_supporting_vertex_without_margin(&Vec3::new(0.0, 0.0, 1.0));
    assert_eq!(s, Vec3::new(0.0, 0.0, 3.0));
    let (mn, mx) = h.get_aabb(&Transform::IDENTITY);
    assert!((mx.y - 2.08).abs() < 1e-12 && (mn.x + 0.08).abs() < 1e-12);
}
