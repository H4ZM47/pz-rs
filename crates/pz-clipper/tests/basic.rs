//! Hand-authored checks of the Clipper / ClipperOffset JNI behaviour. Exact
//! equality with the original library is verified separately by a local
//! differential run (see the crate README).

use pz_clipper::pz::{to_fixed, to_float};
use pz_clipper::{Clipper, ClipperError, ClipperOffset, ZoneClipper};

fn square(x0: f32, y0: f32, x1: f32, y1: f32) -> Vec<f32> {
    vec![x0, y0, x1, y0, x1, y1, x0, y1]
}

fn ring_area(flat: &[f32]) -> f64 {
    let n = flat.len() / 2;
    let mut a = 0.0f64;
    for i in 0..n {
        let j = (i + 1) % n;
        a += flat[2 * i] as f64 * flat[2 * j + 1] as f64
            - flat[2 * j] as f64 * flat[2 * i + 1] as f64;
    }
    a / 2.0
}

fn sorted_points(flat: &[f32]) -> Vec<(f32, f32)> {
    let mut v: Vec<(f32, f32)> = flat.chunks(2).map(|c| (c[0], c[1])).collect();
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    v
}

#[test]
fn fixed_point_conversion() {
    assert_eq!(to_fixed(1.0), 1000);
    assert_eq!(to_fixed(-0.3), -300);
    assert_eq!(to_fixed(0.0004), 0);
    assert_eq!(to_fixed(0.0005), 1);
    // -0.0005f32 is slightly below -0.0005, and the product is not rounded.
    assert_eq!(to_fixed(-0.0005), -1);
    assert_eq!(to_fixed(f32::NAN), 0);
    assert_eq!(to_fixed(1e12), i32::MAX as i64);
    assert_eq!(to_float(1500), 1.5);
}

#[test]
fn clip_square_by_rect() {
    let mut c = Clipper::new();
    c.add_path(4, &square(0.0, 0.0, 10.0, 10.0), false, true)
        .unwrap();
    c.clip_aabb(5.0, 5.0, 15.0, 15.0);
    // ctDifference: the part of the square outside the clip rectangle.
    assert_eq!(c.generate_polygons(), 1);
    let p = c.get_polygon(0).unwrap();
    assert!(p.holes.is_empty());
    let pts = p.points_f32();
    assert_eq!(pts.len(), 12);
    assert_eq!(ring_area(&pts).abs(), 75.0);
    assert_eq!(
        sorted_points(&pts),
        vec![
            (0.0, 0.0),
            (0.0, 10.0),
            (5.0, 5.0),
            (5.0, 10.0),
            (10.0, 0.0),
            (10.0, 5.0)
        ]
    );
}

#[test]
fn union_and_intersection() {
    let mut c = Clipper::new();
    c.add_path(4, &square(0.0, 0.0, 2.0, 2.0), false, true)
        .unwrap();
    c.add_path(4, &square(1.0, 1.0, 3.0, 3.0), false, true)
        .unwrap();
    assert_eq!(c.generate_polygons_full(Clipper::CT_UNION, 0.0, 0), 1);
    assert_eq!(
        ring_area(&c.get_polygon(0).unwrap().points_f32()).abs(),
        7.0
    );

    let mut c = Clipper::new();
    c.add_path(4, &square(0.0, 0.0, 2.0, 2.0), false, true)
        .unwrap();
    c.clip_aabb(1.0, 1.0, 3.0, 3.0);
    assert_eq!(
        c.generate_polygons_full(Clipper::CT_INTERSECTION, 0.0, 0),
        1
    );
    assert_eq!(
        sorted_points(&c.get_polygon(0).unwrap().points_f32()),
        vec![(1.0, 1.0), (1.0, 2.0), (2.0, 1.0), (2.0, 2.0)]
    );
}

#[test]
fn holes_and_islands() {
    // Difference of two rectangles: one polygon with one hole.
    let mut c2 = Clipper::new();
    c2.add_aabb(0.0, 0.0, 10.0, 10.0);
    c2.clip_aabb(2.0, 2.0, 8.0, 8.0);
    assert_eq!(c2.generate_polygons(), 1);
    let p = c2.get_polygon(0).unwrap();
    assert_eq!(p.holes.len(), 1);
    assert_eq!(ring_area(&p.points_f32()).abs(), 100.0);
    assert_eq!(ring_area(&p.holes_f32()[0]).abs(), 36.0);

    // Union with Positive fill of an outer square, a reversed (negative)
    // square and an island: the island becomes its own polygon after its
    // parent.
    let mut c3 = Clipper::new();
    c3.add_path(4, &square(0.0, 0.0, 10.0, 10.0), false, true)
        .unwrap();
    c3.add_path(4, &square(2.0, 8.0, 8.0, 2.0), false, true)
        .unwrap();
    c3.add_path(4, &square(4.0, 4.0, 6.0, 6.0), false, true)
        .unwrap();
    assert_eq!(c3.generate_polygons_full(Clipper::CT_UNION, 0.0, 0), 2);
    assert_eq!(c3.get_polygon(0).unwrap().holes.len(), 1);
    assert_eq!(
        ring_area(&c3.get_polygon(1).unwrap().points_f32()).abs(),
        4.0
    );
}

#[test]
fn get_polygon_byte_layout() {
    let mut c = Clipper::new();
    c.add_aabb(0.0, 0.0, 1.0, 1.0);
    assert_eq!(c.generate_polygons(), 1);
    let mut buf = vec![0u8; 64];
    assert_eq!(c.get_polygon_into(0, &mut buf), Ok(0));
    assert_eq!(&buf[0..2], &4u16.to_be_bytes());
    let floats: Vec<f32> = (0..8)
        .map(|i| f32::from_be_bytes(buf[2 + 4 * i..6 + 4 * i].try_into().unwrap()))
        .collect();
    assert_eq!(
        sorted_points(&floats),
        vec![(0.0, 0.0), (0.0, 1.0), (1.0, 0.0), (1.0, 1.0)]
    );
    assert_eq!(&buf[34..36], &0u16.to_be_bytes());

    // Too small: the bytes that fit are written, then the overflow.
    let mut small = vec![0u8; 10];
    assert_eq!(
        c.get_polygon_into(0, &mut small),
        Err(ClipperError::BufferOverflow)
    );
    assert_eq!(&small[0..6], &buf[0..6]);
    assert_eq!(
        c.get_polygon_into(1, &mut buf),
        Err(ClipperError::IndexOutOfRange)
    );
}

#[test]
fn add_path_reads_the_whole_buffer() {
    let mut c = Clipper::new();
    assert_eq!(
        c.add_path(3, &[0.0, 0.0, 1.0, 0.0], false, true),
        Err(ClipperError::BufferUnderflow)
    );
    assert_eq!(
        c.add_path(-1, &[0.0; 8], false, true),
        Err(ClipperError::LengthError)
    );
    // Reading only the first n points of a larger buffer is fine.
    let mut buf = square(0.0, 0.0, 3.0, 3.0);
    buf.extend([99.0; 6]);
    c.add_path(4, &buf, false, true).unwrap();
    assert_eq!(c.generate_polygons(), 1);
    assert_eq!(
        ring_area(&c.get_polygon(0).unwrap().points_f32()).abs(),
        9.0
    );
}

#[test]
fn triangulation() {
    let mut c = Clipper::new();
    c.add_aabb(0.0, 0.0, 4.0, 4.0);
    c.clip_aabb(1.0, 1.0, 3.0, 3.0);
    assert_eq!(c.generate_polygons(), 1);
    let tris = c.triangulate(0).unwrap();
    // 8 vertices with one hole -> 8 triangles.
    assert_eq!(tris.len(), 8 * 3 * 2);
    let area: f64 = tris.chunks(6).map(|t| ring_area(t).abs()).sum();
    assert_eq!(area, 12.0);

    let (verts, idx) = c.triangulate2(0).unwrap();
    assert_eq!(verts.len(), 16);
    assert_eq!(idx.len(), 24);
    let mut buf = vec![0u8; 256];
    assert_eq!(c.triangulate2_into(0, &mut buf), Ok(24));
    assert_eq!(&buf[0..2], &8u16.to_be_bytes());
    let mut buf = vec![0u8; 256];
    assert_eq!(c.triangulate_into(0, &mut buf), Ok(24));
}

#[test]
fn generate_triangulate_polygons_subtracts_from_the_cell() {
    let mut c = Clipper::new();
    c.add_aabb(2.0, 2.0, 4.0, 4.0);
    assert_eq!(c.generate_polygons(), 1);
    assert_eq!(c.generate_triangulate_polygons(0, 0), 1);
    let p = c.get_polygon(0).unwrap();
    assert_eq!(ring_area(&p.points_f32()).abs(), 64.0);
    assert_eq!(p.holes.len(), 1);
    assert_eq!(ring_area(&p.holes_f32()[0]).abs(), 4.0);
}

#[test]
fn generate_with_offset() {
    let mut c = Clipper::new();
    c.add_aabb(0.0, 0.0, 10.0, 10.0);
    assert_eq!(c.generate_polygons_delta(-1.0), 1);
    assert_eq!(
        sorted_points(&c.get_polygon(0).unwrap().points_f32()),
        vec![(1.0, 1.0), (1.0, 9.0), (9.0, 1.0), (9.0, 9.0)]
    );
    // Square join on the convex corners of a grown square.
    assert_eq!(c.generate_polygons_delta_join(1.0, Clipper::JT_MITER), 1);
    assert_eq!(
        sorted_points(&c.get_polygon(0).unwrap().points_f32()),
        vec![(-1.0, -1.0), (-1.0, 11.0), (11.0, -1.0), (11.0, 11.0)]
    );
}

#[test]
fn offset_polyline_miter_butt() {
    let mut o = ClipperOffset::new();
    o.add_path(
        2,
        &[0.0, 0.0, 10.0, 0.0],
        ClipperOffset::JOIN_MITER,
        ClipperOffset::END_BUTT,
    )
    .unwrap();
    o.execute(0.5);
    assert_eq!(o.get_polygon_count(), 1);
    assert_eq!(
        sorted_points(&o.get_polygon(0).unwrap().points_f32()),
        vec![(0.0, -0.5), (0.0, 0.5), (10.0, -0.5), (10.0, 0.5)]
    );

    let mut z = ZoneClipper::new();
    let outline = z
        .offset_polyline(&[0.0, 0.0, 10.0, 0.0, 10.0, 10.0], 1.0)
        .unwrap();
    assert_eq!(ring_area(&outline).abs(), 40.0);
    assert!(z.offset_polyline(&[1.0, 1.0], 1.0).is_none() || z.offset.get_polygon_count() == 1);

    let pieces = z.clip_polygon_aabb(&square(0.0, 0.0, 10.0, 10.0), 0.0, 4.0, 10.0, 6.0);
    assert_eq!(pieces.len(), 2);
}
