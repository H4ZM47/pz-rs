//! Parity of the atlas render jobs against the original classes:
//! `WorldItemAtlas.RenderJob` (`calcMatrices`, `calcModelBounds`,
//! `calcModelOffset`, `calcEntrySize`) for boxes with assorted transforms,
//! rotations and zoom, and `DeadBodyAtlas.RenderJob` (`calcModelOffset`,
//! `calcEntrySize`) on a posed body's bones.
//!
//! The oracle harness (outside the repo) writes `atlas.txt`; point
//! `PZ_ATLAS_ORACLE_DIR` at it. Without it this test does nothing.

use std::rc::Rc;

use pz_iso_render::body_atlas::placement;
use pz_iso_render::item_atlas::{entry_offset, entry_size, ItemModel, ItemParams};
use pz_iso_render::joml::Matrix4f;
use pz_render::model::{MeshData, MeshKind};

fn f(s: &str) -> f32 {
    s.parse().unwrap()
}

fn close(a: f32, b: f32, tol: f32) -> bool {
    (a - b).abs() <= tol * (1.0 + b.abs())
}

#[test]
fn atlas_jobs_match_the_original() {
    let Some(dir) = std::env::var_os("PZ_ATLAS_ORACLE_DIR") else {
        eprintln!("PZ_ATLAS_ORACLE_DIR not set; skipping");
        return;
    };
    let text = std::fs::read_to_string(std::path::Path::new(&dir).join("atlas.txt")).unwrap();
    let lines: Vec<Vec<&str>> = text
        .lines()
        .map(|l| l.split_whitespace().collect())
        .collect();
    let m16 = |v: &[&str]| {
        Matrix4f::from_cols(std::array::from_fn(|c| {
            std::array::from_fn(|r| f(v[c * 4 + r]))
        }))
    };
    let check_m = |what: &str, a: &Matrix4f, b: &Matrix4f| {
        for c in 0..4 {
            for r in 0..4 {
                assert!(
                    close(a.m[c][r], b.m[c][r], 1e-6),
                    "{what} m{c}{r}: {} vs {}",
                    a.m[c][r],
                    b.m[c][r]
                );
            }
        }
    };
    let (mut items, mut bodies) = (0, 0);
    let mut i = 0;
    while i < lines.len() {
        let l = &lines[i];
        match l[0] {
            "item" => {
                let v: Vec<f32> = l[2..].iter().map(|s| f(s)).collect();
                let t = Matrix4f::identity()
                    .scale1(v[10])
                    .rotate_y(v[11] * std::f32::consts::PI / 180.0)
                    .translate(v[12], 0.0, 0.0);
                let model = ItemModel {
                    name: "box".into(),
                    mesh: Rc::new(MeshData {
                        id: 0,
                        kind: MeshKind::Static,
                        vertices: vec![],
                        indices: vec![],
                    }),
                    min: [v[0], v[1], v[2]],
                    max: [v[3], v[4], v[5]],
                    texture: None,
                };
                let mut p = ItemParams::new(model, t);
                p.angle = [v[6], v[7], v[8]];
                p.max_zoom_is_one = v[9] == 1.0;
                check_m("T", &t, &m16(&lines[i + 1][2..]));
                let (pm, mv) = p.matrices(0.0, 0.0);
                check_m("P", &pm, &m16(&lines[i + 2][2..]));
                check_m("MV", &mv, &m16(&lines[i + 3][2..]));
                let r: Vec<f32> = lines[i + 4][2..].iter().map(|s| f(s)).collect();
                let b = p.bounds();
                for k in 0..4 {
                    assert!(
                        close(b[k], r[k], 1e-5),
                        "item {} bounds {b:?} vs {r:?}",
                        l[1]
                    );
                }
                let (ox, oy) = entry_offset(b);
                assert!(
                    (ox - r[4]).abs() < 1e-3 && (oy - r[5]).abs() < 1e-3,
                    "item {} offset {ox} {oy} vs {r:?}",
                    l[1]
                );
                assert_eq!(
                    entry_size(b),
                    (r[6] as i32, r[7] as i32),
                    "item {} size",
                    l[1]
                );
                items += 1;
                i += 5;
            }
            "body" => {
                let (angle, scale, n) = (f(l[2]), f(l[3]), l[4].parse::<usize>().unwrap());
                let bones: Vec<[f32; 3]> = lines[i + 1..i + 1 + n]
                    .iter()
                    .map(|b| [f(b[1]), f(b[2]), f(b[3])])
                    .collect();
                let r: Vec<f32> = lines[i + 1 + n][2..].iter().map(|s| f(s)).collect();
                let pl = placement(angle, scale, &bones);
                for k in 0..3 {
                    assert!(
                        (pl.offset[k] - r[k]).abs() < 1e-4,
                        "body {} offset {:?} vs {r:?}",
                        l[1],
                        pl.offset
                    );
                }
                assert!(
                    (pl.entry_offset[0] - r[3]).abs() < 1e-2
                        && (pl.entry_offset[1] - r[4]).abs() < 1e-2,
                    "body {} entry offset {:?} vs {r:?}",
                    l[1],
                    pl.entry_offset
                );
                assert_eq!(
                    (pl.entry_w, pl.entry_h),
                    (r[5] as i32, r[6] as i32),
                    "body {} size",
                    l[1]
                );
                bodies += 1;
                i += n + 2;
            }
            _ => i += 1,
        }
    }
    assert!(items >= 5 && bodies >= 4, "{items} items, {bodies} bodies");
}
