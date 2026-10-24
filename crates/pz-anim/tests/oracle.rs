//! Parity against the original Java classes.
//!
//! The oracle harness (kept outside the repo) runs the game's
//! `AnimationPlayer` / `AdvancedAnimator` headless over a script and writes
//! `skeleton.txt` (skeleton and clips it loaded) and `frames.txt` (skin
//! transforms, deferred movement and events per frame). Point
//! `PZ_ANIM_ORACLE_DIR` at a directory holding those plus `script.txt`, and
//! `PZ_ANIMSETS_DIR` at `media/AnimSets/player`; without them this test
//! does nothing.

#![allow(clippy::needless_range_loop)]

use std::path::PathBuf;
use std::rc::Rc;

use pz_anim::math::Matrix4f;
use pz_anim::{JavaRandom, AdvancedAnimator, AnimationPlayer, AnimationSet, AnimationVariableSource, MapVariableSource};
use pz_model::{AnimationClip, Keyframe, Quat, SkinningData};

fn f(s: &str) -> f32 {
    s.parse().unwrap()
}

fn m16(v: &[&str]) -> pz_model::Mat4 {
    let mut a = [0f32; 16];
    for (i, s) in v.iter().take(16).enumerate() {
        a[i] = f(s);
    }
    Matrix4f::load(&a).to_mat4()
}

fn load_skinning(path: &PathBuf) -> SkinningData {
    let text = std::fs::read_to_string(path).unwrap();
    let mut sd = SkinningData::default();
    let mut clip: Option<AnimationClip> = None;
    for line in text.lines() {
        let p: Vec<&str> = line.split_whitespace().collect();
        match p[0] {
            "bones" => {
                let n: usize = p[1].parse().unwrap();
                sd.bind_pose = vec![Default::default(); n];
                sd.inverse_bind_pose = vec![Default::default(); n];
                sd.bone_offset = vec![Default::default(); n];
                sd.skeleton_hierarchy = vec![-1; n];
            }
            "name" => {
                sd.bone_indices.insert(p[2].to_string(), p[1].parse().unwrap());
            }
            "parent" => sd.skeleton_hierarchy[p[1].parse::<usize>().unwrap()] = p[2].parse().unwrap(),
            "bind" => sd.bind_pose[p[1].parse::<usize>().unwrap()] = m16(&p[2..]),
            "invbind" => sd.inverse_bind_pose[p[1].parse::<usize>().unwrap()] = m16(&p[2..]),
            "offset" => sd.bone_offset[p[1].parse::<usize>().unwrap()] = m16(&p[2..]),
            "clip" => {
                if let Some(c) = clip.take() {
                    sd.animation_clips.insert(c.name.clone(), c);
                }
                clip = Some(AnimationClip { name: p[1].into(), duration: f(p[2]), keep_last_frame: p[3] == "true", ..Default::default() });
            }
            "k" => clip.as_mut().unwrap().keyframes.push(Keyframe {
                bone: p[1].parse().unwrap(),
                time: f(p[2]),
                position: [f(p[3]), f(p[4]), f(p[5])],
                rotation: Quat { x: f(p[6]), y: f(p[7]), z: f(p[8]), w: f(p[9]) },
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

struct JavaFrame {
    events: Vec<String>,
    dm: [f32; 5],
    mats: Vec<[f32; 16]>,
    attached: Vec<[f32; 16]>,
    shared: Vec<[f32; 16]>,
}

fn load_frames(path: &PathBuf) -> Vec<JavaFrame> {
    let text = std::fs::read_to_string(path).unwrap();
    let mut out: Vec<JavaFrame> = Vec::new();
    for line in text.lines() {
        let p: Vec<&str> = line.split_whitespace().collect();
        match p[0] {
            "frame" => out.push(JavaFrame { events: vec![], dm: [0.0; 5], mats: vec![], attached: vec![], shared: vec![] }),
            "ev" => out.last_mut().unwrap().events.push(p[1..].join(" ")),
            "dm" => {
                let d = &mut out.last_mut().unwrap().dm;
                for i in 0..5 {
                    d[i] = f(p[1 + i]);
                }
            }
            "m" | "c" | "s" => {
                let mut a = [0f32; 16];
                for i in 0..16 {
                    a[i] = f(p[2 + i]);
                }
                let fr = out.last_mut().unwrap();
                match p[0] {
                    "m" => fr.mats.push(a),
                    "c" => fr.attached.push(a),
                    _ => fr.shared.push(a),
                }
            }
            _ => {}
        }
    }
    out
}

#[test]
fn matches_java_oracle() {
    let (Ok(dir), Ok(sets)) = (std::env::var("PZ_ANIM_ORACLE_DIR"), std::env::var("PZ_ANIMSETS_DIR")) else {
        eprintln!("PZ_ANIM_ORACLE_DIR / PZ_ANIMSETS_DIR not set; skipping");
        return;
    };
    let dir = PathBuf::from(dir);
    let sd = Rc::new(load_skinning(&dir.join("skeleton.txt")));
    let frames = load_frames(&dir.join("frames.txt"));
    let script = std::fs::read_to_string(dir.join("script.txt")).unwrap();
    let lines: Vec<Vec<String>> = script
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(|l| l.split_whitespace().map(str::to_string).collect())
        .collect();

    let mut player = AnimationPlayer::new(sd.clone());
    player.motion_scale = 1.6;
    player.rotation_scale = 1.1;
    let mut animator = AdvancedAnimator::new();
    let seed = lines.iter().find(|l| l[0] == "seed").map(|l| l[1].parse().unwrap()).unwrap_or(12345i64);
    animator.rng = Box::new(JavaRandom::new(seed));
    let mut attached: Option<AnimationPlayer> = None;
    let mut shared: Option<AnimationPlayer> = None;
    let mut side_err = [0f32; 2];
    let mut vars = MapVariableSource::new();
    let states: Vec<&str> = lines.iter().find(|l| l[0] == "states").unwrap()[1..].iter().map(String::as_str).collect();
    animator.set_anim_set(Some(Rc::new(AnimationSet::load_states("player", &PathBuf::from(sets), &states))));

    let mut frame = 0usize;
    let mut max_err = 0f32;
    let mut max_err_frame = 0;
    let mut max_dm_err = 0f32;
    let mut event_mismatches = 0;
    let mut first_bad: Option<usize> = None;
    for l in &lines {
        match l[0].as_str() {
            "setup" => match l[1].as_str() {
                "twist" => {
                    let v: Vec<&str> = l[2..].iter().map(String::as_str).collect();
                    player.set_twist_bones(&v);
                }
                "counter" => player.set_counter_rotation_bone(&l[2]),
                "reparent" => player.add_bone_reparent(&l[2], &l[3]),
                _ => panic!(),
            },
            "attached" => {
                let mut p = AnimationPlayer::new(sd.clone());
                let t = p.play(&l[1], true, false, -1.0).unwrap();
                p.track_mut(t).unwrap().set_blend_weight(1.0);
                attached = Some(p);
            }
            "shared" => {
                let mut p = AnimationPlayer::new(sd.clone());
                p.do_blending = false;
                p.set_shared_anim_repo(Some(Default::default()));
                let t = p.play(&l[1], true, false, -1.0).unwrap();
                let t = p.track_mut(t).unwrap();
                t.set_blend_weight(1.0);
                t.set_blend_field_weight(0.0);
                shared = Some(p);
            }
            "dirnow" => player.set_target_and_current_direction(f(&l[1]), f(&l[2])),
            "dir" => player.set_target_direction(f(&l[1]), f(&l[2])),
            "var" => {
                match l[1].as_str() {
                    "f" => vars.set_variable_float(&l[2], f(&l[3])),
                    "b" => vars.set_variable_bool(&l[2], l[3].eq_ignore_ascii_case("true")),
                    _ => vars.set_variable_string(&l[2], l.get(3).map(String::as_str).unwrap_or("")),
                };
            }
            "state" => {
                let subs: Vec<&str> = l[2..].iter().map(String::as_str).collect();
                animator.set_state(&mut player, &mut vars, &l[1], &subs);
            }
            "run" => {
                let n: usize = l[1].parse().unwrap();
                let dt = f(&l[2]);
                for _ in 0..n {
                    animator.animate(&mut player, &mut vars, dt);
                    let dm = player.deferred_movement(true);
                    let ours = [dm.x, dm.y, player.angle(), player.twist_angle(), player.shoulder_twist_angle()];
                    let j = &frames[frame];
                    for i in 0..5 {
                        max_dm_err = max_dm_err.max((ours[i] - j.dm[i]).abs());
                    }
                    for (k, side) in [attached.as_mut(), shared.as_mut()].into_iter().enumerate() {
                        let Some(sp) = side else { continue };
                        match k {
                            0 => sp.update_attached(dt, None, &player),
                            _ => sp.update(dt, None),
                        }
                        let want = if k == 0 { &j.attached } else { &j.shared };
                        for (b, m) in sp.skin_transforms(None).iter().enumerate() {
                            for (a, c) in m.store().iter().zip(&want[b]) {
                                side_err[k] = side_err[k].max((a - c).abs());
                            }
                        }
                    }
                    let evs: Vec<String> = animator
                        .events
                        .drain(..)
                        .map(|e| {
                            format!(
                                "{} {} {} {}",
                                e.state_name.as_deref().unwrap_or("null"),
                                e.layer_depth,
                                e.event.event_name,
                                e.event.parameter_value.as_deref().unwrap_or("-")
                            )
                        })
                        .map(|s| s.trim_end().to_string())
                        .collect();
                    if evs != j.events {
                        event_mismatches += 1;
                        if event_mismatches <= 5 {
                            eprintln!("frame {frame}: events ours {evs:?} java {:?}", j.events);
                        }
                    }
                    let st = player.skin_transforms(None);
                    let mut fe = 0f32;
                    for (b, m) in st.iter().enumerate() {
                        let s = m.store();
                        for i in 0..16 {
                            fe = fe.max((s[i] - j.mats[b][i]).abs());
                        }
                    }
                    if std::env::var("PZ_ANIM_ORACLE_VERBOSE").is_ok() && fe > 1e-5 {
                        let worst: Vec<usize> = st.iter().enumerate().filter(|(b, m)| m.store().iter().zip(&j.mats[*b]).any(|(a, c)| (a - c).abs() > 1e-4)).map(|(b, _)| b).collect();
                        eprintln!("frame {frame}: err {fe:e} bones {worst:?}");
                    }
                    if fe > 1e-3 && first_bad.is_none() {
                        first_bad = Some(frame);
                    }
                    if fe > max_err {
                        max_err = fe;
                        max_err_frame = frame;
                    }
                    frame += 1;
                }
            }
            _ => {}
        }
    }
    eprintln!("attached player max err = {:e}, shared-skeleton player max err = {:e}", side_err[0], side_err[1]);
    eprintln!(
        "oracle parity: {frame} frames, {} bones; max |skin - java| = {max_err:e} (frame {max_err_frame}); max deferred/angle err = {max_dm_err:e}; event mismatches = {event_mismatches}; first frame > 1e-3: {first_bad:?}",
        sd.num_bones()
    );
    assert_eq!(frame, frames.len());
    assert!(max_err < 1e-3, "skin transforms diverge");
    assert_eq!(event_mismatches, 0);
    assert!(side_err[0] < 1e-3 && side_err[1] < 1e-3, "side players diverge");
}
