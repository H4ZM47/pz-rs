//! Replays the frame extras' geometry against a dump of the original
//! classes (`FBORenderAreaHighlights.AreaHighlight.render`,
//! `FBORenderWorldMarkers.Marker.render` with their drawers' ortho,
//! `CorpseFlies.render`, `WorldMarkers.DrawTextureAngle` and
//! `IsoSprite.renderTextureWithDepth`'s depth and position). Runs only when
//! `PZ_EXTRAS_ORACLE` names the dump; the harness that writes it is kept
//! outside the repository.
//!
//! Dump lines, floats as raw IEEE bits in hex (`|` separates inputs from
//! results): `area px py pz ow oh x1 y1 x2 y2 z r g b a | (ndcX ndcY depth
//! u v) x4 r g b a`, `marker px py pz ow oh ox oy z size r g b a | quad |
//! quad`, `flies px py offX offY x y z frame | x y w h u1 v1 .. u4 v4 nextZ
//! nextChunkDepth`, `angle w h cx cy angle mod | x1 y1 .. x4 y4` (doubles as
//! 64-bit hex), `texdepth px py x y z | chunkDepth nextZ screenX screenY`.

use pz_iso_render::area_highlights::AreaHighlight;
use pz_iso_render::flies::CorpseFlies;
use pz_iso_render::markers::{
    draw_texture_angle, render_texture_with_depth, DirectionArrow, GridSquareMarker, MarkersFrame,
};
use pz_iso_render::models::ModelCamera;
use pz_iso_render::world::{DepthMaps, DepthPreset};
use pz_iso_render::{Camera, ChunkRenderer};
use pz_render::{DrawCmd, DrawList, Texture, WorldShader};

fn f(s: &str) -> f32 {
    f32::from_bits(u32::from_str_radix(s, 16).unwrap())
}

fn d(s: &str) -> f64 {
    f64::from_bits(u64::from_str_radix(s, 16).unwrap())
}

fn tex(name: &str, id: u32, w: u32, h: u32) -> Texture {
    let mut t = Texture::empty(name);
    t.id = Some(id);
    t.width = w;
    t.height = h;
    t.width_hw = w as i32;
    t.height_hw = h as i32;
    t.ready = true;
    t
}

fn cam(px: f32, py: f32, pz: f32) -> Camera {
    let mut c = Camera::centred(px, py, pz, 1280, 720, 1.0);
    c.x = px;
    c.y = py;
    c.z = pz;
    c
}

struct NoMaps;
impl DepthMaps for NoMaps {
    fn preset(&self, _: DepthPreset) -> Option<Texture> {
        None
    }
    fn default_texture(&self) -> Option<Texture> {
        None
    }
}

/// `(ndcX, ndcY, depth, u, v)` per corner of a projected depth quad.
fn quad(c: &DrawCmd) -> Vec<f32> {
    let DrawCmd::ProjectedDepthQuad { pos, uv, .. } = c else {
        panic!("{c:?}")
    };
    (0..4)
        .flat_map(|i| [pos[i][0], pos[i][1], pos[i][2], uv[i][0], uv[i][1]])
        .collect()
}

fn check(what: &str, line: &str, want: &[f32], got: &[f32], bad: &mut usize) {
    check_with(what, line, want, got, bad, |_| true);
}

/// Projected quads: the window depths and texture coordinates exactly, the
/// normalised device x and y within 2 ulp-scale (the original projects on
/// the GPU; the dump's projection is the harness's own JOML product).
fn check_quad(what: &str, line: &str, want: &[f32], got: &[f32], bad: &mut usize) {
    check_with(what, line, want, got, bad, |i| i % 5 >= 2);
}

fn check_with(
    what: &str,
    line: &str,
    want: &[f32],
    got: &[f32],
    bad: &mut usize,
    exact: impl Fn(usize) -> bool,
) {
    let differs = want.iter().zip(got).enumerate().any(|(i, (a, b))| {
        if exact(i) {
            a.to_bits() != b.to_bits()
        } else {
            (a - b).abs() > 1e-6 * a.abs().max(1.0)
        }
    });
    if differs || want.len() != got.len() {
        if *bad < 10 {
            eprintln!("{what} mismatch: {line}\n want {want:?}\n  got {got:?}");
        }
        *bad += 1;
    }
}

#[test]
fn extras_match_the_original() {
    let Ok(path) = std::env::var("PZ_EXTRAS_ORACLE") else {
        return;
    };
    let text = std::fs::read_to_string(path).unwrap();
    let white = tex("white", 3, 32, 32);
    let (mut n, mut bad) = (0, 0);
    for line in text.lines() {
        let (ins, outs) = line.split_once('|').unwrap_or((line, ""));
        let i: Vec<&str> = ins.split_whitespace().collect();
        let o: Vec<f32> = outs
            .split(['|', ' '])
            .filter(|s| !s.is_empty())
            .map(f)
            .collect();
        match i[0] {
            "area" => {
                let c = cam(f(i[1]), f(i[2]), f(i[3]));
                let mc = ModelCamera::new(&c, f(i[4]), f(i[5]));
                let p: Vec<i32> = i[6..11].iter().map(|s| s.parse().unwrap()).collect();
                let rgba = [f(i[11]), f(i[12]), f(i[13]), f(i[14])];
                let a = AreaHighlight {
                    x1: p[0],
                    y1: p[1],
                    x2: p[2],
                    y2: p[3],
                    z: p[4],
                    rgba,
                };
                let q = a.quad(&mc, &white);
                check_quad("area", line, &o[..20], &quad(&q), &mut bad);
                let DrawCmd::ProjectedDepthQuad { color, .. } = q else {
                    unreachable!()
                };
                assert_eq!(
                    color,
                    pz_iso_render::color::to_abgr(o[20], o[21], o[22], o[23])
                );
            }
            "marker" => {
                let c = cam(f(i[1]), f(i[2]), f(i[3]));
                let mc = ModelCamera::new(&c, f(i[4]), f(i[5]));
                let z: i32 = i[8].parse().unwrap();
                let m = GridSquareMarker {
                    orig: [f(i[6]), f(i[7]), z as f32],
                    z: z as f32,
                    size: f(i[9]),
                    rgb: [f(i[10]), f(i[11]), f(i[12])],
                    alpha: f(i[13]),
                    active: true,
                    texture_name: "circle_center".into(),
                    overlay_texture_name: None,
                };
                let frame = MarkersFrame {
                    grid_square_markers: vec![m],
                    circle_center: Some(tex("cc", 1, 64, 64)),
                    chunk_map: Some([-1000, -1000, 1000, 1000]),
                    ..Default::default()
                };
                let mut out = Vec::new();
                frame.render_grid_square_markers(&mc, z, &mut out);
                let quads: Vec<Vec<f32>> = out
                    .iter()
                    .filter(|c| matches!(c, DrawCmd::ProjectedDepthQuad { .. }))
                    .map(quad)
                    .collect();
                assert_eq!(quads.len(), 2);
                check_quad("marker", line, &o[..20], &quads[0], &mut bad);
                check_quad("marker 2", line, &o[24..44], &quads[1], &mut bad);
            }
            "flies" => {
                let mut c = cam(f(i[1]), f(i[2]), 0.0);
                c.off_x = i[3].parse().unwrap();
                c.off_y = i[4].parse().unwrap();
                let r = ChunkRenderer::new(c, &NoMaps, (1280, 720));
                let fl = CorpseFlies {
                    texture: Some(tex("flies", 7, 1024, 1024)),
                    counter: 0.0,
                    frame: i[8].parse().unwrap(),
                };
                let mut out = Vec::new();
                fl.render(
                    &r,
                    i[5].parse().unwrap(),
                    i[6].parse().unwrap(),
                    i[7].parse().unwrap(),
                    &mut out,
                );
                let DrawCmd::WorldSprite(s, WorldShader::Default { z }) = &out[0] else {
                    panic!()
                };
                let got = [
                    s.x[0],
                    s.y[0],
                    s.x[1] - s.x[0],
                    s.y[2] - s.y[0],
                    s.u[0],
                    s.v[0],
                    s.u[1],
                    s.v[1],
                    s.u[2],
                    s.v[2],
                    s.u[3],
                    s.v[3],
                    *z,
                    0.0,
                ];
                check("flies", line, &o, &got, &mut bad);
            }
            "angle" => {
                let a = DirectionArrow {
                    texture: tex("arrow", 9, 64, 64),
                    render_width: f(i[1]),
                    render_height: f(i[2]),
                    screen: [0.0; 2],
                    angle: d(i[5]),
                    rgba: [1.0; 4],
                    render_size_mod: f(i[6]),
                };
                // The centre is a double in the original; place it exactly.
                let (cx, cy) = (d(i[3]), d(i[4]));
                let mut dl = DrawList::default();
                draw_texture_angle_at(&a, cx, cy, &mut dl);
                let DrawCmd::Sprite(s) = &dl.cmds[0] else {
                    panic!()
                };
                let got: Vec<f32> = (0..4).flat_map(|k| [s.x[k], s.y[k]]).collect();
                check("angle", line, &o, &got, &mut bad);
            }
            "texdepth" => {
                let c = cam(f(i[1]), f(i[2]), 0.0);
                let mut c = c;
                c.off_x = 0;
                c.off_y = 0;
                let (x, y, z) = (f(i[3]), f(i[4]), f(i[5]));
                let mut out = Vec::new();
                render_texture_with_depth(
                    &c,
                    &tex("m", 4, 64, 64),
                    0.0,
                    0.0,
                    [1.0; 4],
                    x,
                    y,
                    z,
                    &mut out,
                );
                let DrawCmd::WorldSprite(s, WorldShader::Default { z: depth }) = &out[0] else {
                    panic!()
                };
                check(
                    "texdepth",
                    line,
                    &[o[0] + o[1], o[2], o[3]],
                    &[*depth, s.x[0], s.y[0]],
                    &mut bad,
                );
            }
            _ => continue,
        }
        n += 1;
    }
    eprintln!("{n} cases, {bad} mismatches");
    assert!(n > 0 && bad == 0);
}

fn draw_texture_angle_at(a: &DirectionArrow, cx: f64, cy: f64, dl: &mut DrawList) {
    draw_texture_angle(
        &DirectionArrow {
            screen: [cx as f32, cy as f32],
            ..a.clone()
        },
        dl,
    );
}
