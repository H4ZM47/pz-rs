//! Parity of the iso character pass against the original classes: the
//! model camera (`Core.DoPushIsoStuff`) and `targetDepth` for characters
//! and vehicles at known squares and cameras, `calculateShadowParams` on a
//! walking body, and `FBORenderShadows` shadows (`addShadow` corners,
//! depths and colour; `render`'s projection and model-view).
//!
//! The oracle harness (outside the repo) writes `skeleton.txt` and
//! `charrender.txt`; point `PZ_CHARRENDER_ORACLE_DIR` at them. Without it
//! this test does nothing.

use std::path::PathBuf;
use std::rc::Rc;

use pz_anim::math::Matrix4f;
use pz_anim::AnimationPlayer;
use pz_character_model::scene::shadow_params;
use pz_iso_render::models::{ModelBasis, ModelCamera};
use pz_iso_render::shadows::{ShadowParams, Shadows};
use pz_iso_render::trees::Mat4;
use pz_iso_render::Camera;
use pz_model::{AnimationClip, Keyframe, Quat, SkinningData};
use pz_render::iso::{fastfloor, square_depth};

fn f(s: &str) -> f32 {
    s.parse().unwrap()
}

fn load_skinning(path: &std::path::Path) -> SkinningData {
    let text = std::fs::read_to_string(path).unwrap();
    let mut sd = SkinningData::default();
    let mut clip: Option<AnimationClip> = None;
    let m16 = |v: &[&str]| {
        let a: [f32; 16] = std::array::from_fn(|i| f(v[i]));
        Matrix4f::load(&a).to_mat4()
    };
    for line in text.lines() {
        let p: Vec<&str> = line.split_whitespace().collect();
        let idx = |i: usize| p[i].parse::<usize>().unwrap();
        match p[0] {
            "bones" => {
                let n = idx(1);
                sd.bind_pose = vec![Default::default(); n];
                sd.inverse_bind_pose = vec![Default::default(); n];
                sd.bone_offset = vec![Default::default(); n];
                sd.skeleton_hierarchy = vec![-1; n];
            }
            "name" => {
                sd.bone_indices
                    .insert(p[2].to_string(), p[1].parse().unwrap());
            }
            "parent" => sd.skeleton_hierarchy[idx(1)] = p[2].parse().unwrap(),
            "bind" => sd.bind_pose[idx(1)] = m16(&p[2..]),
            "invbind" => sd.inverse_bind_pose[idx(1)] = m16(&p[2..]),
            "offset" => sd.bone_offset[idx(1)] = m16(&p[2..]),
            "clip" => {
                if let Some(c) = clip.take() {
                    sd.animation_clips.insert(c.name.clone(), c);
                }
                clip = Some(AnimationClip {
                    name: p[1].into(),
                    duration: f(p[2]),
                    keep_last_frame: p[3] == "true",
                    ..Default::default()
                });
            }
            "k" => clip.as_mut().unwrap().keyframes.push(Keyframe {
                bone: p[1].parse().unwrap(),
                time: f(p[2]),
                position: [f(p[3]), f(p[4]), f(p[5])],
                rotation: Quat {
                    x: f(p[6]),
                    y: f(p[7]),
                    z: f(p[8]),
                    w: f(p[9]),
                },
                scale: [f(p[10]), f(p[11]), f(p[12])],
                bone_name: p.get(13).unwrap_or(&"").to_string(),
            }),
            _ => {}
        }
    }
    if let Some(c) = clip.take() {
        sd.animation_clips.insert(c.name.clone(), c);
    }
    sd
}

fn floats(p: &[&str]) -> Vec<f32> {
    p.iter().map(|s| f(s)).collect()
}

/// Close within `tol` relative to the larger magnitude (absolute near 0).
fn close(a: f32, b: f32, tol: f32) -> bool {
    (a - b).abs() <= tol * a.abs().max(b.abs()).max(1.0)
}

fn assert_mat(what: &str, got: &Mat4, want: &[f32], tol: f32) {
    let g: Vec<f32> = got.0.iter().flatten().copied().collect();
    for i in 0..16 {
        assert!(
            close(g[i], want[i], tol),
            "{what}[{i}]: {} vs {}\n got {g:?}\nwant {want:?}",
            g[i],
            want[i]
        );
    }
}

#[test]
fn matches_charrender_oracle() {
    let Ok(dir) = std::env::var("PZ_CHARRENDER_ORACLE_DIR") else {
        eprintln!("PZ_CHARRENDER_ORACLE_DIR not set; skipping");
        return;
    };
    let dir = PathBuf::from(dir);
    let body = Rc::new(load_skinning(&dir.join("skeleton.txt")));
    let text = std::fs::read_to_string(dir.join("charrender.txt")).unwrap();
    let lines: Vec<Vec<&str>> = text
        .lines()
        .map(|l| l.split_whitespace().collect())
        .collect();

    let mut player = AnimationPlayer::new(body);
    player.set_target_and_current_direction(0.6, 0.8);
    let t = player.play("Bob_Walk", true, false, -1.0).unwrap();
    player.track_mut(t).unwrap().set_blend_weight(1.0);

    let (mut n_sp, mut n_cam, mut n_sh) = (0, 0, 0);
    let mut worst_sp = 0.0f32;
    let cam0 = Camera::centred(10.5, 10.5, 0.0, 1920, 1080, 1.0);
    let mc0 = ModelCamera::new(&cam0, 1920.0, 1080.0);
    let mut i = 0;
    while i < lines.len() {
        let p = &lines[i];
        match p[0] {
            "sp" => {
                player.update(0.05, None);
                let v = floats(&p[2..]);
                assert!(
                    close(player.angle(), v[0], 1e-6) && close(player.rendered_angle(), v[1], 1e-6)
                );
                let sp = shadow_params(Some(&player), 1.0, false);
                let r = floats(&lines[i + 1][2..]);
                let spr = shadow_params(Some(&player), 0.7, true);
                for (got, want) in [(sp, &v[2..5]), (spr, &r[..])] {
                    let g = [got.w, got.fm, got.bm];
                    for k in 0..3 {
                        worst_sp = worst_sp.max((g[k] - want[k]).abs());
                        assert!(
                            (g[k] - want[k]).abs() < 2e-5,
                            "frame {}: shadow params {g:?} vs {want:?}",
                            p[1]
                        );
                    }
                }
                n_sp += 1;
                i += 2;
                continue;
            }
            "cam" => {
                let vehicle = p[2] == "1";
                let v = floats(&p[3..]);
                let (cx, cy, cz, ow, oh, ox, oy, oz, angle) =
                    (v[0], v[1], v[2], v[3], v[4], v[5], v[6], v[7], v[8]);
                let cam = Camera::centred(cx, cy, cz, ow as i32, oh as i32, 1.0);
                let mc = ModelCamera::new(&cam, ow, oh);
                let basis = if vehicle {
                    ModelBasis::Vehicle
                } else {
                    ModelBasis::Character { angle }
                };
                let mv = mc.model_view(ox, oy, oz, basis);
                assert_mat(
                    "projection",
                    &mc.projection,
                    &floats(&lines[i + 1][1..]),
                    1e-6,
                );
                assert_mat("model-view", &mv, &floats(&lines[i + 2][1..]), 2e-6);
                let d = floats(&lines[i + 3][1..]);
                let mvp = mc.mvp(&mv);
                let d0 = mvp.transform([0.0, 0.0, 0.0])[2];
                assert!((d0 - d[0]).abs() < 1e-7, "depth value {d0} vs {}", d[0]);
                let sq = square_depth(fastfloor(cx), fastfloor(cy), ox, oy, oz);
                assert_eq!(sq, d[1], "square depth");
                let td = mc.target_depth(&mvp, ox, oy, oz, if vehicle { 0.0 } else { 1.0e-4 });
                assert!((td - d[2]).abs() < 1e-7, "targetDepth {td} vs {}", d[2]);
                n_cam += 1;
                i += 5;
                continue;
            }
            "sh" => {
                let v = floats(&p[2..14]);
                let animal = p[14] == "true";
                let want = floats(&p[15..]);
                let mut shadows = Shadows::default();
                let mut tex = pz_render::Texture::empty("NewShadow");
                tex.id = Some(1);
                shadows.clear(Some(tex));
                let params = ShadowParams {
                    w: v[5],
                    fm: v[6],
                    bm: v[7],
                };
                shadows.add_character_shadow(
                    &mc0,
                    [v[0], v[1], v[2]],
                    [v[3], v[4]],
                    params,
                    [v[8], v[9], v[10]],
                    v[11],
                    animal,
                    None,
                );
                let s = &shadows.shadows[0];
                for k in 0..4 {
                    for c in 0..2 {
                        assert!(
                            close(s.corners[k][c], want[k * 2 + c], 1e-6),
                            "shadow {} corner {k}: {:?} vs {:?}",
                            p[1],
                            s.corners,
                            &want[..8]
                        );
                    }
                    assert_eq!(s.depths[k], want[8 + k], "shadow {} depth {k}", p[1]);
                }
                for k in 0..4 {
                    assert!(
                        close(s.rgba[k], want[12 + k], 1e-6),
                        "shadow {} colour {:?} vs {:?}",
                        p[1],
                        s.rgba,
                        &want[12..]
                    );
                }
                assert_mat(
                    "shadow projection",
                    &mc0.projection,
                    &floats(&lines[i + 1][2..]),
                    1e-6,
                );
                assert_mat(
                    "shadow model-view",
                    &s.model_view(&mc0),
                    &floats(&lines[i + 2][2..]),
                    2e-6,
                );
                // The quad's corners and per-corner depths as submitted.
                let q = floats(&lines[i + 3][2..]);
                let pz_render::DrawCmd::ProjectedDepthQuad { pos, uv, color, .. } = s.quad(&mc0)
                else {
                    panic!()
                };
                let jp = Mat4(std::array::from_fn(|c| {
                    std::array::from_fn(|r| f(lines[i + 1][2 + c * 4 + r]))
                }));
                let jmv = Mat4(std::array::from_fn(|c| {
                    std::array::from_fn(|r| f(lines[i + 2][2 + c * 4 + r]))
                }));
                let jmvp = jp.mul_mat(&jmv);
                for k in 0..4 {
                    let b = k * 6;
                    let ndc = jmvp.transform([q[b], q[b + 1], q[b + 2]]);
                    assert!(
                        (pos[k][0] - ndc[0]).abs() < 1e-5 && (pos[k][1] - ndc[1]).abs() < 1e-5,
                        "shadow {} corner {k} ndc",
                        p[1]
                    );
                    assert_eq!(pos[k][2], q[b + 5]);
                    assert_eq!(uv[k], [q[b + 3], q[b + 4]]);
                }
                let c = pz_iso_render::color::to_abgr(q[24], q[25], q[26], q[27]);
                assert_eq!(color, c);
                assert_eq!(lines[i + 4][2], "glEnable", "{:?}", lines[i + 4]);
                n_sh += 1;
                i += 5;
                continue;
            }
            _ => {}
        }
        i += 1;
    }
    eprintln!("shadow params {n_sp} frames (worst {worst_sp:e}), cameras {n_cam}, shadows {n_sh}");
    assert!(n_sp == 12 && n_cam == 12 && n_sh == 6);
}
