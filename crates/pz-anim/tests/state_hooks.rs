//! The character-state hooks (`ThumpAnimations`, `ZombieTurnAlerted`,
//! `doDeferredMovement`) on synthetic AnimSets and clips, against values
//! worked out by hand from the original's logic.

use std::path::{Path, PathBuf};
use std::rc::Rc;

use pz_anim::state_hooks::{do_deferred_movement, turn_alerted_enter};
use pz_anim::{
    thump, AdvancedAnimator, AnimationPlayer, AnimationSet, BoneAxis, DeferredMovementInput,
    MapVariableSource,
};
use pz_model::{AnimationClip, Keyframe, Mat4, Quat, SkinningData};

const IDENT: Mat4 = [
    [1.0, 0.0, 0.0, 0.0],
    [0.0, 1.0, 0.0, 0.0],
    [0.0, 0.0, 1.0, 0.0],
    [0.0, 0.0, 0.0, 1.0],
];

fn key(bone: i32, time: f32, pos: [f32; 3]) -> Keyframe {
    Keyframe {
        bone,
        bone_name: String::new(),
        time,
        position: pos,
        rotation: Quat {
            x: 0.0,
            y: 0.0,
            z: 0.0,
            w: 1.0,
        },
        scale: [1.0; 3],
    }
}

/// A clip of `duration` seconds moving `bone` by `delta`.
fn clip(name: &str, duration: f32, bone: i32, delta: [f32; 3]) -> AnimationClip {
    let mut keyframes = Vec::new();
    for b in 0..3 {
        let end = if b == bone { delta } else { [0.0; 3] };
        keyframes.push(key(b, 0.0, [0.0; 3]));
        keyframes.push(key(b, duration, end));
    }
    AnimationClip {
        name: name.into(),
        duration,
        keep_last_frame: true,
        is_ragdoll: false,
        keyframes,
    }
}

fn player_with(clips: &[AnimationClip]) -> AnimationPlayer {
    let mut sd = SkinningData {
        bind_pose: vec![IDENT; 3],
        inverse_bind_pose: vec![IDENT; 3],
        bone_offset: vec![IDENT; 3],
        skeleton_hierarchy: vec![-1, 0, 1],
        ..Default::default()
    };
    for (i, n) in ["Root", "Bip01", "Tip"].iter().enumerate() {
        sd.bone_indices.insert(n.to_string(), i as i32);
    }
    for c in clips {
        sd.animation_clips.insert(c.name.clone(), c.clone());
    }
    AnimationPlayer::new(Rc::new(sd))
}

fn temp_dir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("pz-anim-hooks-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn write(dir: &Path, rel: &str, body: &str) {
    let p = dir.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, body).unwrap();
}

/// A `thump` state whose node claws twice per loop (25% and 75%) and an
/// `idle` state without thump events.
fn thump_set() -> Rc<AnimationSet> {
    let d = temp_dir("thump");
    write(
        &d,
        "thump/Claw.xml",
        r#"<animNode><m_Name>Claw</m_Name><m_AnimName>claw</m_AnimName><m_Looped>true</m_Looped>
           <m_Events><m_EventName>ThumpFrame</m_EventName><m_TimePc>0.25</m_TimePc></m_Events>
           <m_Events><m_EventName>thumpframe</m_EventName><m_TimePc>0.75</m_TimePc></m_Events>
           <m_Events><m_EventName>Other</m_EventName><m_TimePc>0.5</m_TimePc></m_Events></animNode>"#,
    );
    write(
        &d,
        "idle/Idle.xml",
        r#"<animNode><m_Name>Idle</m_Name><m_AnimName>idle</m_AnimName></animNode>"#,
    );
    let set = Rc::new(AnimationSet::load("zombie", &d));
    let _ = std::fs::remove_dir_all(&d);
    set
}

/// The thump times `k + t` (k whole loops) in `(start, end]`, counted one
/// by one.
fn crossings(times_s: &[f32], duration: f32, start: f32, end: f32) -> i32 {
    let mut n = 0;
    for k in -2..20 {
        for &t in times_s {
            let at = k as f32 * duration + t;
            if at > start && at <= end {
                n += 1;
            }
        }
    }
    n
}

#[test]
fn thump_count_follows_the_track_time() {
    let mut player = player_with(&[
        clip("claw", 2.0, 0, [0.0; 3]),
        clip("idle", 1.0, 0, [0.0; 3]),
    ]);
    let mut anim = AdvancedAnimator::new();
    anim.set_anim_set(Some(thump_set()));
    let mut vars = MapVariableSource::new();

    // No thump node: no track, duration -1, nothing counted.
    anim.set_state(&mut player, &mut vars, "idle", &[]);
    anim.animate(&mut player, &mut vars, 0.1);
    assert!(thump::live_anim_node(&anim).is_none());
    assert_eq!(thump::duration(&anim, &player), -1.0);
    assert_eq!(thump::thumps_this_frame(&anim, &player, true, 0.1), 0);

    anim.set_state(&mut player, &mut vars, "thump", &[]);
    anim.animate(&mut player, &mut vars, 0.1);
    let node = thump::live_anim_node(&anim).expect("thump node");
    assert_eq!(node.name(), "Claw");
    // Event names match ignoring case; the other event is not a thump.
    assert_eq!(thump::thump_times(node), [0.25, 0.75]);
    assert_eq!(thump::duration(&anim, &player), 2.0);

    // Thumps at 0.5 s and 1.5 s of each 2 s loop.
    let at = [0.5f32, 1.5];
    let dt = 0.07;
    let mut total = 0;
    for frame in 0..80 {
        let track = thump::animation_track(&anim, &player).unwrap();
        let (prev, speed) = (track.accumulated_time_value(), track.speed_delta());
        assert_eq!(speed, 1.0);
        let n = thump::thumps_this_frame(&anim, &player, true, dt);
        assert_eq!(
            n,
            crossings(&at, 2.0, prev, prev + dt * speed),
            "frame {frame} at {prev}"
        );
        // Not updating this frame: no thumps.
        assert_eq!(thump::thumps_this_frame(&anim, &player, false, dt), 0);
        total += n;
        anim.animate(&mut player, &mut vars, dt);
    }
    // 80 frames of 0.07 s from about 0.1 s: past 0.5, 1.5, ..., 5.5.
    assert_eq!(total, 6);
    // A span over a whole loop and a half from the start: 0.5, 1.5, 2.5.
    assert_eq!(
        thump::count_thump_events_between(&anim, &player, 0.0, 3.0),
        3
    );
}

#[test]
fn thump_event_times_start_end_and_percentage() {
    let d = temp_dir("times");
    write(
        &d,
        "thump/T.xml",
        r#"<animNode><m_Name>T</m_Name><m_AnimName>claw</m_AnimName>
           <m_Events><m_EventName>ThumpFrame</m_EventName><m_Time>Start</m_Time></m_Events>
           <m_Events><m_EventName>ThumpFrame</m_EventName><m_Time>End</m_Time></m_Events>
           <m_Events><m_EventName>ThumpFrame</m_EventName><m_Time>Percentage</m_Time><m_TimePc>0.4</m_TimePc></m_Events></animNode>"#,
    );
    let set = Rc::new(AnimationSet::load("zombie", &d));
    let _ = std::fs::remove_dir_all(&d);
    let mut player = player_with(&[clip("claw", 1.0, 0, [0.0; 3])]);
    let mut anim = AdvancedAnimator::new();
    anim.set_anim_set(Some(set));
    let mut vars = MapVariableSource::new();
    anim.set_state(&mut player, &mut vars, "thump", &[]);
    anim.animate(&mut player, &mut vars, 0.05);
    let node = thump::live_anim_node(&anim).unwrap();
    assert_eq!(thump::thump_times(node), [0.0, 1.0, 0.4]);
    // (0.1, 1.1]: 0.4, 1.0 (Start of the second loop and End of the
    // first fall on the same instant and both count), 1.0 again.
    assert_eq!(
        thump::count_thump_events_between(&anim, &player, 0.1, 1.1),
        3
    );
}

#[test]
fn turn_alerted_sets_the_target_angle_only() {
    let mut player = player_with(&[]);
    player.set_angle(0.5);
    turn_alerted_enter(&mut player, -2.25);
    assert_eq!(player.target_angle(), -2.25);
    assert_eq!(player.angle(), 0.5);
}

#[test]
fn deferred_movement_step() {
    let mut player = player_with(&[clip("walk", 1.0, 1, [2.0, 0.0, 0.5])]);
    let t = player.play("walk", true, false, -1.0).unwrap();
    {
        let tr = player.track_mut(t).unwrap();
        tr.set_blend_weight(1.0);
        tr.set_deferred_bone(Some(1), BoneAxis::Y);
        tr.set_use_deferred_movement(true);
    }
    player.update(0.25, None);
    let raw = player.deferred_movement(false);
    let len = raw.length();
    assert!(len > 0.0, "{raw:?}");

    // Not animating this frame (and not a server): untouched, no step.
    let idle = DeferredMovementInput {
        animation_updating_this_frame: false,
        ..Default::default()
    };
    assert_eq!(do_deferred_movement(&mut player, &idle), None);
    assert_eq!(player.deferred_movement(false), raw);

    // The plain step is the root motion.
    assert_eq!(
        do_deferred_movement(&mut player, &DeferredMovementInput::default()),
        Some((raw.x, raw.y))
    );

    // WalkTowardState: a closer target shortens the step to its distance
    // (normalize, then scale), a farther one leaves it.
    let near = DeferredMovementInput {
        walk_toward: Some((0.0, len * 0.5)),
        ..Default::default()
    };
    let (sx, sy) = do_deferred_movement(&mut player, &near).unwrap();
    let k = (len * 0.5) / len;
    assert!(
        (sx - raw.x * k).abs() < 1e-6 && (sy - raw.y * k).abs() < 1e-6,
        "{sx} {sy} {raw:?}"
    );
    let far = DeferredMovementInput {
        walk_toward: Some((len * 3.0, 0.0)),
        ..Default::default()
    };
    assert_eq!(
        do_deferred_movement(&mut player, &far),
        Some((raw.x, raw.y))
    );

    // Grappling adds the grapple offset.
    let grapple = DeferredMovementInput {
        grapple_offset: Some((0.25, -0.5)),
        ..Default::default()
    };
    assert_eq!(
        do_deferred_movement(&mut player, &grapple),
        Some((raw.x + 0.25, raw.y - 0.5))
    );

    // A path (path2) or disabled deferred movement: consumed, no step.
    let path2 = DeferredMovementInput {
        path2_blocks: true,
        ..Default::default()
    };
    assert_eq!(do_deferred_movement(&mut player, &path2), None);
    let off = DeferredMovementInput {
        deferred_movement_enabled: false,
        ..Default::default()
    };
    assert_eq!(do_deferred_movement(&mut player, &off), None);
    // On a server the step is taken even when the animation did not update.
    let server = DeferredMovementInput {
        server: true,
        animation_updating_this_frame: false,
        ..Default::default()
    };
    assert_eq!(
        do_deferred_movement(&mut player, &server),
        Some((raw.x, raw.y))
    );
}

struct Zed {
    player: AnimationPlayer,
    animator: AdvancedAnimator,
    vars: MapVariableSource,
}

impl pz_anim::AnimatedCharacter for Zed {
    fn anim_parts(
        &mut self,
    ) -> (
        &mut AnimationPlayer,
        &mut AdvancedAnimator,
        &mut dyn pz_anim::AnimationVariableSource,
    ) {
        (&mut self.player, &mut self.animator, &mut self.vars)
    }

    fn anim_set_name(&self) -> &str {
        "zombie"
    }
}

#[test]
fn hooks_on_an_animated_character() {
    use pz_anim::{AnimatedCharacter, CharacterAnim, StateAnimHooks};
    let mut z = Zed {
        player: player_with(&[clip("claw", 2.0, 0, [0.0; 3])]),
        animator: AdvancedAnimator::new(),
        vars: MapVariableSource::new(),
    };
    z.animator.set_anim_set(Some(thump_set()));
    {
        let (p, a, v) = z.anim_parts();
        a.set_state(p, v, "thump", &[]);
    }
    z.animate(0.45);
    assert_eq!(z.thump_duration(), 2.0);
    // The track sits at 0.45 s: the 0.5 s thump falls in the next 0.1 s.
    assert_eq!(z.thumps_this_frame(true, 0.1), 1);
    assert_eq!(z.count_thump_events_between(0.0, 4.0), 4);
    z.turn_alerted_enter(1.5);
    assert_eq!(z.player.target_angle(), 1.5);
    assert_eq!(
        z.deferred_movement_step(&DeferredMovementInput::default()),
        Some((0.0, 0.0))
    );
    let mut ca = CharacterAnim {
        animator: &z.animator,
        player: &mut z.player,
    };
    assert_eq!(ca.thumps_this_frame(true, 0.1), 1);
    ca.turn_alerted_enter(-1.0);
    assert_eq!(z.player.target_angle(), -1.0);
}
