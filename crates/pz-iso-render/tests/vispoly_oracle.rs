//! Replays a dump of the original `VisibilityPolygon2.Drawer` (the new
//! visibility path) and compares the shadow triangles
//! (`calculateVisibilityPolygonNew`: `addViewShadow`, `isInViewCone`,
//! `addWallShadow`, `addSquareShadow`) and the vertices `PolygonData`
//! uploads (`updatePolygons` / `addVertex`) bit for bit. Runs only when
//! `PZ_VISPOLY_ORACLE` names the dump; the harness that produces it is
//! kept outside the repository.
//!
//! Dump format, floats as raw IEEE bits in hex: per case
//! `case px py pz visionCone lookAngle lookX lookY xMin yMin xMax yMax`,
//! `wall x1 y1 x2 y2` and `solid x y` lines (one chunk's walls then solid
//! squares, in loop order), `verts n f...`, `cam offX offY`, `poly n f...`.

use pz_iso_render::vispoly::{
    polygon_vertices, shadow_verts, ChunkLevelData, VisibilityWall, VisionFrame,
};
use pz_iso_render::Camera;

fn f(s: &str) -> f32 {
    f32::from_bits(u32::from_str_radix(s, 16).expect("hex"))
}

fn i(s: &str) -> i32 {
    s.parse().expect("int")
}

#[test]
fn shadow_triangles_match_the_original() {
    let Ok(path) = std::env::var("PZ_VISPOLY_ORACLE") else {
        return;
    };
    let text = std::fs::read_to_string(path).expect("dump");
    let mut lines = text.lines().peekable();
    let (mut cases, mut vert_count) = (0, 0);
    while let Some(line) = lines.next() {
        let t: Vec<&str> = line.split(' ').collect();
        assert_eq!(t[0], "case", "{line}");
        let (px, py, pz) = (f(t[1]), f(t[2]), i(t[3]));
        let mut v = VisionFrame {
            vision_cone: f(t[4]),
            look_angle: f(t[5]),
            look_vector: [f(t[6]), f(t[7])],
            chunk_map: [i(t[8]), i(t[9]), i(t[10]), i(t[11])],
            ..Default::default()
        };
        let mut chunk = ChunkLevelData::default();
        while let Some(l) = lines.peek().copied() {
            let t: Vec<&str> = l.split(' ').collect();
            match t[0] {
                "wall" => chunk.all_walls.push(VisibilityWall {
                    x1: i(t[1]),
                    y1: i(t[2]),
                    x2: i(t[3]),
                    y2: i(t[4]),
                }),
                "solid" => chunk.solid_squares.push((i(t[1]), i(t[2]))),
                _ => break,
            }
            lines.next();
        }
        v.chunks.push(chunk);
        let verts: Vec<f32> = lines
            .next()
            .expect("verts")
            .split(' ')
            .skip(2)
            .map(f)
            .collect();
        let cam_line: Vec<&str> = lines.next().expect("cam").split(' ').collect();
        let poly: Vec<f32> = lines
            .next()
            .expect("poly")
            .split(' ')
            .skip(2)
            .map(f)
            .collect();
        let cam = Camera {
            off_x: f(cam_line[1]) as i32,
            off_y: f(cam_line[2]) as i32,
            zoom: 1.0,
            x: px,
            y: py,
            z: pz as f32,
            fix_jiggly_x: 0.0,
            fix_jiggly_y: 0.0,
            fix_jiggly_square_x: 0.0,
            fix_jiggly_square_y: 0.0,
        };
        let ours = shadow_verts(&cam, &v);
        let bits = |s: &[f32]| s.iter().map(|x| x.to_bits()).collect::<Vec<_>>();
        assert_eq!(bits(&ours), bits(&verts), "case {cases}: shadow vertices");
        let ours_poly: Vec<f32> = polygon_vertices(&cam, &ours)
            .into_iter()
            .flatten()
            .collect();
        assert_eq!(bits(&ours_poly), bits(&poly), "case {cases}: polygon data");
        cases += 1;
        vert_count += verts.len() / 3;
    }
    assert!(cases > 0);
    eprintln!("vispoly oracle: {cases} cases, {vert_count} vertices identical");
}
