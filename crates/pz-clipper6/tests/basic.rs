//! Hand-authored checks with synthetic inputs. Exact equality with the
//! original library is verified separately by a local differential run (see
//! the crate README); the exact point orders asserted here were confirmed
//! the same way.

use pz_clipper6::clipper::{self, *};
use pz_clipper6::pz::{from_fixed, to_fixed};
use pz_clipper6::{earcut, PZClipper, Triangle2f};

fn p(v: &[i64]) -> Path {
    v.chunks(2).map(|c| IntPoint::new(c[0], c[1])).collect()
}

fn tri_area(t: &Triangle2f) -> f64 {
    let (ax, ay, bx, by, cx, cy) = (
        t.x0 as f64,
        t.y0 as f64,
        t.x1 as f64,
        t.y1 as f64,
        t.x2 as f64,
        t.y2 as f64,
    );
    ((bx - ax) * (cy - ay) - (cx - ax) * (by - ay)).abs() / 2.0
}

#[test]
fn fixed_point_conversion() {
    assert_eq!(to_fixed(1.0), 1000);
    assert_eq!(to_fixed(-0.3), -300);
    assert_eq!(to_fixed(9.7), 9700);
    assert_eq!(to_fixed(0.0004), 0);
    assert_eq!(to_fixed(0.0005), 1);
    assert_eq!(to_fixed(f32::NAN), 0);
    assert_eq!(to_fixed(1e12), i32::MAX as i64);
    assert_eq!(to_fixed(-1e12), i32::MIN as i64);
    assert_eq!(from_fixed(1500), 1.5);
    assert_eq!(from_fixed(-300), -0.3);
}

#[test]
fn union_of_two_squares() {
    let mut c = Clipper::new(0);
    assert_eq!(
        c.add_path(
            &p(&[0, 0, 100, 0, 100, 100, 0, 100]),
            PolyType::Subject,
            true
        ),
        Ok(true)
    );
    assert_eq!(
        c.add_path(
            &p(&[50, 50, 150, 50, 150, 150, 50, 150]),
            PolyType::Clip,
            true
        ),
        Ok(true)
    );
    let mut sol = Paths::new();
    assert_eq!(
        c.execute(
            ClipType::Union,
            &mut sol,
            PolyFillType::EvenOdd,
            PolyFillType::EvenOdd
        ),
        Ok(true)
    );
    assert_eq!(
        sol,
        vec![p(&[
            100, 50, 150, 50, 150, 150, 50, 150, 50, 100, 0, 100, 0, 0, 100, 0
        ])]
    );
    assert_eq!(clipper::area(&sol[0]), 17500.0);
    assert!(clipper::orientation(&sol[0]));
}

#[test]
fn difference_makes_a_hole() {
    let mut c = Clipper::new(0);
    c.add_path(
        &p(&[0, 0, 100, 0, 100, 100, 0, 100]),
        PolyType::Subject,
        true,
    )
    .unwrap();
    c.add_path(&p(&[25, 25, 75, 25, 75, 75, 25, 75]), PolyType::Clip, true)
        .unwrap();
    let mut tree = PolyTree::new();
    assert!(c.execute_tree(
        ClipType::Difference,
        &mut tree,
        PolyFillType::NonZero,
        PolyFillType::NonZero
    ));
    assert_eq!(tree.total(), 2);
    let outer = tree.get_first().unwrap();
    assert!(!tree.is_hole(outer));
    assert_eq!(tree.contour(outer), &p(&[100, 100, 0, 100, 0, 0, 100, 0]));
    let hole = tree.get_next(outer).unwrap();
    assert!(tree.is_hole(hole));
    assert_eq!(tree.parent(hole), Some(outer));
    assert_eq!(tree.contour(hole), &p(&[25, 25, 25, 75, 75, 75, 75, 25]));
    assert_eq!(tree.get_next(hole), None);
}

#[test]
fn open_paths_need_a_tree() {
    let mut c = Clipper::new(0);
    assert!(c
        .add_path(&p(&[0, 0, 10, 10]), PolyType::Clip, false)
        .is_err());
    assert_eq!(
        c.add_path(&p(&[0, 5, 20, 5]), PolyType::Subject, false),
        Ok(true)
    );
    c.add_path(&p(&[5, 0, 15, 0, 15, 10, 5, 10]), PolyType::Clip, true)
        .unwrap();
    let mut sol = Paths::new();
    assert!(c
        .execute(
            ClipType::Intersection,
            &mut sol,
            PolyFillType::EvenOdd,
            PolyFillType::EvenOdd
        )
        .is_err());
    let mut tree = PolyTree::new();
    assert!(c.execute_tree(
        ClipType::Intersection,
        &mut tree,
        PolyFillType::EvenOdd,
        PolyFillType::EvenOdd
    ));
    let open = clipper::open_paths_from_poly_tree(&tree);
    assert_eq!(open.len(), 1);
    let mut xs: Vec<i64> = open[0].iter().map(|q| q.x).collect();
    xs.sort();
    assert_eq!(xs, vec![5, 15]);
}

#[test]
fn coordinates_out_of_range() {
    let mut c = Clipper::new(0);
    let big = HI_RANGE + 1;
    assert!(c
        .add_path(&p(&[0, 0, big, 0, 0, 10]), PolyType::Subject, true)
        .is_err());
    // Full-range (128-bit) slope tests above LO_RANGE.
    let b = LO_RANGE * 4;
    assert_eq!(
        c.add_path(&p(&[0, 0, b, 0, b, b, 0, b]), PolyType::Subject, true),
        Ok(true)
    );
}

#[test]
fn offset_square() {
    let mut co = ClipperOffset::new(2.0, 0.25);
    co.add_path(
        &p(&[0, 0, 100, 0, 100, 100, 0, 100]),
        JoinType::Miter,
        EndType::ClosedPolygon,
    );
    let mut sol = Paths::new();
    co.execute(&mut sol, 10.0).unwrap();
    assert_eq!(sol.len(), 1);
    let mut pts = sol[0].clone();
    pts.sort_by_key(|q| (q.x, q.y));
    assert_eq!(pts, p(&[-10, -10, -10, 110, 110, -10, 110, 110]));

    co.execute(&mut sol, -10.0).unwrap();
    assert_eq!(clipper::area(&sol[0]).abs(), 6400.0);

    // Round joins add arc vertices.
    co.clear();
    co.add_path(
        &p(&[0, 0, 100, 0, 100, 100, 0, 100]),
        JoinType::Round,
        EndType::ClosedPolygon,
    );
    co.execute(&mut sol, 10.0).unwrap();
    assert!(sol[0].len() > 8);
}

#[test]
fn earcut_square_with_hole() {
    let rings = vec![
        p(&[0, 0, 10, 0, 10, 10, 0, 10]),
        p(&[2, 2, 2, 8, 8, 8, 8, 2]),
    ];
    let idx = earcut::earcut(&rings);
    assert_eq!(idx.len(), 8 * 3);
    let pts: Vec<IntPoint> = rings.concat();
    let area: i64 = idx
        .chunks(3)
        .map(|t| {
            let (a, b, c) = (pts[t[0] as usize], pts[t[1] as usize], pts[t[2] as usize]);
            ((b.x - a.x) * (c.y - a.y) - (c.x - a.x) * (b.y - a.y)).abs()
        })
        .sum();
    assert_eq!(area, 2 * (100 - 36));
}

/// A 2x2 block of solid squares at (10, 20), radius 0.3, as the visibility
/// graph adds them.
fn block(pz: &mut PZClipper, squares: &[(i32, i32)]) {
    let r = 0.3f32;
    for &(x, y) in squares {
        let (fx, fy) = (x as f32, y as f32);
        pz.add_aabb(fx - r, fy - r, r + (x + 1) as f32, r + (y + 1) as f32);
    }
}

#[test]
fn pz_generate_polygons_merges_squares() {
    let mut pz = PZClipper::new();
    pz.clear();
    block(&mut pz, &[(10, 20), (11, 20), (10, 21), (11, 21)]);
    assert_eq!(pz.count(), 4);
    pz.generate_polygons(0.0, JoinType::Miter);
    assert_eq!(pz.polys().len(), 1);
    assert_eq!(
        pz.polys()[0].outer,
        p(&[9700, 22300, 9700, 19700, 12300, 19700, 12300, 22300])
    );
    assert!(pz.polys()[0].holes.is_empty());

    let mut tris = Vec::new();
    pz.triangulate(0, &mut tris);
    assert_eq!(tris.len(), 2);
    assert_eq!(
        tris[0],
        Triangle2f {
            x0: 12.3,
            y0: 19.7,
            x1: 12.3,
            y1: 22.3,
            x2: 9.7,
            y2: 22.3
        }
    );
    let area: f64 = tris.iter().map(tri_area).sum();
    assert!((area - 2.6 * 2.6).abs() < 1e-4);

    pz.clear_polys();
    assert!(pz.polys().is_empty());
}

#[test]
fn pz_bevel_is_a_no_op() {
    let mut pz = PZClipper::new();
    pz.add_aabb_bevel(0.0, 0.0, 1.0, 1.0, 0.198);
    assert_eq!(pz.count(), 0);
    assert_eq!(pz.generate_obstacle_polygons(), Ok(()));
    assert!(pz.polys().is_empty());
}

#[test]
fn pz_offset_grows_the_union() {
    let mut pz = PZClipper::new();
    pz.add_aabb(0.0, 0.0, 2.0, 1.0);
    pz.generate_polygons(0.5, JoinType::Miter);
    assert_eq!(
        pz.polys()[0].outer,
        p(&[2500, 1500, -500, 1500, -500, -500, 2500, -500])
    );
}

#[test]
fn pz_walkable_chunk_has_obstacle_holes() {
    let mut pz = PZClipper::new();
    block(&mut pz, &[(9, 17), (10, 17), (12, 19)]);
    pz.generate_obstacle_polygons().unwrap();
    assert_eq!(pz.polys().len(), 2);
    pz.generate_walkable_polygons(1, 2).unwrap();
    assert_eq!(pz.count(), 1);
    assert_eq!(pz.polys().len(), 1);
    let w = &pz.polys()[0];
    assert_eq!(
        w.outer,
        p(&[16000, 24000, 8000, 24000, 8000, 16000, 16000, 16000])
    );
    assert_eq!(
        w.holes,
        vec![
            p(&[11700, 18700, 11700, 20300, 13300, 20300, 13300, 18700]),
            p(&[8700, 16700, 8700, 18300, 11300, 18300, 11300, 16700]),
        ]
    );
    let mut tris = Vec::new();
    pz.triangulate(0, &mut tris);
    assert_eq!(tris.len(), 14);
    let area: f64 = tris.iter().map(tri_area).sum();
    let expected = 64.0 - 1.6 * 1.6 - 2.6 * 1.6;
    assert!((area - expected).abs() < 1e-3, "{area} vs {expected}");
}
