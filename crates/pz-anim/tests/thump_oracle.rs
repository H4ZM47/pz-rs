//! `ThumpAnimations` parity against the original classes.
//!
//! The harness (kept outside the repo) runs the game's `AdvancedAnimator`
//! over a script on the zombie AnimSet and, before each frame's update,
//! writes what `ThumpState.execute` computes through `ThumpAnimations`
//! (`thump <count> <prev> <cur> <duration> <speedDelta>`, or
//! `thump none <duration>`) to `frames.txt`, with the skeleton and clips in
//! `skeleton.txt`. Point `PZ_THUMP_ORACLE_DIR` at that directory (holding
//! `script.txt` too) and `PZ_ANIMSETS_DIR` at `media/AnimSets/zombie`;
//! without them this test does nothing.

use std::path::PathBuf;
use std::rc::Rc;

use pz_anim::math::Matrix4f;
use pz_anim::{
    thump, AdvancedAnimator, AnimationPlayer, AnimationSet, AnimationVariableSource, JavaRandom,
    MapVariableSource,
};
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
                sd.bone_indices
                    .insert(p[2].to_string(), p[1].parse().unwrap());
            }
            "parent" => {
                sd.skeleton_hierarchy[p[1].parse::<usize>().unwrap()] = p[2].parse().unwrap()
            }
            "bind" => sd.bind_pose[p[1].parse::<usize>().unwrap()] = m16(&p[2..]),
            "invbind" => sd.inverse_bind_pose[p[1].parse::<usize>().unwrap()] = m16(&p[2..]),
            "offset" => sd.bone_offset[p[1].parse::<usize>().unwrap()] = m16(&p[2..]),
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

#[test]
fn thump_counts_match_java_oracle() {
    let (Ok(dir), Ok(sets)) = (
        std::env::var("PZ_THUMP_ORACLE_DIR"),
        std::env::var("PZ_ANIMSETS_DIR"),
    ) else {
        eprintln!("PZ_THUMP_ORACLE_DIR / PZ_ANIMSETS_DIR not set; skipping");
        return;
    };
    let dir = PathBuf::from(dir);
    let sd = Rc::new(load_skinning(&dir.join("skeleton.txt")));
    let java: Vec<Vec<String>> = std::fs::read_to_string(dir.join("frames.txt"))
        .unwrap()
        .lines()
        .filter(|l| l.starts_with("thump "))
        .map(|l| l.split_whitespace().skip(1).map(str::to_string).collect())
        .collect();
    let script = std::fs::read_to_string(dir.join("script.txt")).unwrap();
    let lines: Vec<Vec<String>> = script
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(|l| l.split_whitespace().map(str::to_string).collect())
        .collect();
    let mut player = AnimationPlayer::new(sd.clone());
    let mut animator = AdvancedAnimator::new();
    let seed = lines
        .iter()
        .find(|l| l[0] == "seed")
        .map(|l| l[1].parse().unwrap())
        .unwrap_or(12345i64);
    animator.rng = Box::new(JavaRandom::new(seed));
    let set_name = lines
        .iter()
        .find(|l| l[0] == "set")
        .map(|l| l[1].clone())
        .unwrap_or_else(|| "player".into());
    let states: Vec<&str> = lines.iter().find(|l| l[0] == "states").unwrap()[1..]
        .iter()
        .map(String::as_str)
        .collect();
    animator.set_anim_set(Some(Rc::new(AnimationSet::load_states(
        &set_name,
        &PathBuf::from(sets),
        &states,
    ))));
    let mut vars = MapVariableSource::new();
    let mut frame = 0usize;
    let (mut thumps, mut mismatches) = (0, 0);
    for l in &lines {
        match l[0].as_str() {
            "dirnow" => player.set_target_and_current_direction(f(&l[1]), f(&l[2])),
            "dir" => player.set_target_direction(f(&l[1]), f(&l[2])),
            "var" => {
                match l[1].as_str() {
                    "f" => vars.set_variable_float(&l[2], f(&l[3])),
                    "b" => vars.set_variable_bool(&l[2], l[3].eq_ignore_ascii_case("true")),
                    _ => {
                        vars.set_variable_string(&l[2], l.get(3).map(String::as_str).unwrap_or(""))
                    }
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
                    let j = &java[frame];
                    let ok = match thump::animation_track(&animator, &player) {
                        None => j[0] == "none" && f(&j[1]) == thump::duration(&animator, &player),
                        Some(t) => {
                            let (prev, speed) = (t.accumulated_time_value(), t.speed_delta());
                            let cur = prev + dt * speed;
                            let count = thump::thumps_this_frame(&animator, &player, true, dt);
                            thumps += count;
                            j[0] != "none"
                                && count == j[0].parse::<i32>().unwrap()
                                && (prev - f(&j[1])).abs() < 1e-4
                                && (cur - f(&j[2])).abs() < 1e-4
                                && thump::duration(&animator, &player) == f(&j[3])
                                && speed == f(&j[4])
                        }
                    };
                    if !ok {
                        mismatches += 1;
                        if mismatches <= 5 {
                            let t = thump::animation_track(&animator, &player);
                            eprintln!(
                                "frame {frame}: java {j:?} ours {:?}",
                                t.map(|t| (
                                    t.accumulated_time_value(),
                                    t.speed_delta(),
                                    t.get_duration()
                                ))
                            );
                        }
                    }
                    animator.animate(&mut player, &mut vars, dt);
                    frame += 1;
                }
            }
            _ => {}
        }
    }
    eprintln!("thump oracle: {frame} frames, {thumps} thumps, {mismatches} mismatches");
    assert_eq!(frame, java.len());
    assert_eq!(mismatches, 0);
}
