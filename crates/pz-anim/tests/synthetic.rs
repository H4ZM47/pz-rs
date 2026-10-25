//! Behaviour tests on skeletons, clips and AnimSets built in code.

use std::path::Path;
use std::rc::Rc;

use pz_anim::math::{Matrix4f, Quaternion};
use pz_anim::vars::java_float_to_string;
use pz_anim::xml;
use pz_anim::{
    AdvancedAnimator, AnimBoneWeight, AnimClip, AnimNode, AnimState, AnimationPlayer, AnimationSet, AnimationVariableSource, LayerBinding,
    MapVariableSource, StartAnimTrackParameters,
};
use pz_model::{AnimationClip, Keyframe, Mat4, Quat, SkinningData};

const IDENT: Mat4 = [[1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0], [0.0, 0.0, 1.0, 0.0], [0.0, 0.0, 0.0, 1.0]];

/// Three bones in a chain: root(0) -> Bip01(1) -> tip(2).
fn skeleton() -> SkinningData {
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
    sd
}

fn key(bone: i32, time: f32, pos: [f32; 3]) -> Keyframe {
    Keyframe { bone, bone_name: String::new(), time, position: pos, rotation: Quat { x: 0.0, y: 0.0, z: 0.0, w: 1.0 }, scale: [1.0; 3] }
}

/// A clip moving `bone` along x from 0 to `dist` over one second.
fn slide(name: &str, bone: i32, dist: f32) -> AnimationClip {
    let mut keyframes = Vec::new();
    for b in 0..3 {
        let end = if b == bone { dist } else { 0.0 };
        keyframes.push(key(b, 0.0, [0.0; 3]));
        keyframes.push(key(b, 1.0, [end, 0.0, 0.0]));
        keyframes.push(key(b, 1.0, [end, 0.0, 0.0]));
    }
    AnimationClip { name: name.into(), duration: 1.0, keep_last_frame: false, is_ragdoll: false, keyframes }
}

fn player_with(clips: &[AnimationClip]) -> AnimationPlayer {
    let mut sd = skeleton();
    for c in clips {
        sd.animation_clips.insert(c.name.clone(), c.clone());
    }
    AnimationPlayer::new(Rc::new(sd))
}

fn translation(p: &AnimationPlayer, bone: usize) -> [f32; 3] {
    let m = p.palette()[bone];
    [m[3][0], m[3][1], m[3][2]]
}

#[test]
fn clip_drops_duplicate_last_frame_unless_kept() {
    let c = AnimClip::from_model(&slide("a", 0, 1.0));
    assert_eq!(c.bone_frames(0).len(), 2);
    let mut m = slide("a", 0, 1.0);
    m.keep_last_frame = true;
    assert_eq!(AnimClip::from_model(&m).bone_frames(0).len(), 3);
}

#[test]
fn single_track_samples_and_palette_is_column_major() {
    let mut p = player_with(&[slide("slide", 0, 2.0)]);
    let t = p.play("slide", true, false, -1.0).unwrap();
    p.track_mut(t).unwrap().set_blend_weight(1.0);
    p.update(0.25, None);
    let tr = translation(&p, 0);
    assert!((tr[0] - 0.5).abs() < 1e-6, "{tr:?}");
    // Children inherit the parent's transform.
    assert!((translation(&p, 2)[0] - 0.5).abs() < 1e-6);
    // A column-major matrix maps the origin to the translation column.
    let m = p.palette()[0];
    assert_eq!(m[0], [1.0, 0.0, 0.0, 0.0]);
}

#[test]
fn looping_track_wraps_and_non_looping_clamps() {
    let mut p = player_with(&[slide("slide", 0, 1.0)]);
    let a = p.play("slide", true, false, -1.0).unwrap();
    p.update(1.25, None);
    assert!((p.track(a).unwrap().current_time_value() - 0.25).abs() < 1e-6);
    let mut q = player_with(&[slide("slide", 0, 1.0)]);
    let b = q.play("slide", false, false, -1.0).unwrap();
    q.update(1.25, None);
    let t = q.track(b).unwrap();
    assert_eq!(t.current_time_value(), 1.0);
    assert!(t.is_finished());
}

#[test]
fn tracks_blend_by_weight() {
    let mut p = player_with(&[slide("a", 0, 1.0), slide("b", 0, 3.0)]);
    let a = p.play("a", true, false, -1.0).unwrap();
    let b = p.play("b", true, false, -1.0).unwrap();
    p.track_mut(a).unwrap().set_blend_weight(0.5);
    p.track_mut(b).unwrap().set_blend_weight(0.5);
    p.update(0.5, None);
    // 0.5 * 0.5 + 0.5 * 1.5 = 1.0
    assert!((translation(&p, 0)[0] - 1.0).abs() < 1e-5, "{:?}", translation(&p, 0));
}

#[test]
fn sub_layer_bone_mask_limits_tracks_to_masked_bones() {
    let mut p = player_with(&[slide("base", 2, 1.0), slide("upper", 2, 5.0)]);
    let base = p
        .play_params(&StartAnimTrackParameters { anim_name: "base".into(), is_looped: true, initial_weight: 1.0, ..Default::default() }, Some(LayerBinding { id: 0, depth: 0, is_sub_layer: false }))
        .unwrap();
    let weights = vec![AnimBoneWeight { bone_name: "Root".into(), weight: 1.0, include_descendants: false }];
    let upper = p
        .play_params(
            &StartAnimTrackParameters { anim_name: "upper".into(), is_looped: true, initial_weight: 1.0, sub_layer_bone_weights: weights, ..Default::default() },
            Some(LayerBinding { id: 1, depth: 1, is_sub_layer: true }),
        )
        .unwrap();
    assert_eq!(p.track(upper).unwrap().bone_weight(0), 1.0);
    assert_eq!(p.track(upper).unwrap().bone_weight(2), 0.0);
    assert_eq!(p.track(base).unwrap().bone_weight(2), 1.0);
    p.update(0.5, None);
    // The tip only follows the base layer: 0.5, not the sub-layer's 2.5.
    assert!((translation(&p, 2)[0] - 0.5).abs() < 1e-5, "{:?}", translation(&p, 2));
}

#[test]
fn matrix_conversions_round_trip() {
    let mut m = Matrix4f::IDENTITY;
    m.m03 = 1.0;
    m.m13 = 2.0;
    m.m23 = 3.0;
    m.m01 = 0.5;
    assert_eq!(Matrix4f::from_mat4(&m.to_mat4()), m);
    assert_eq!(m.to_mat4()[3], [1.0, 2.0, 3.0, 1.0]);
    assert_eq!(m.to_mat4()[1][0], 0.5);
    let q = Quaternion::from_matrix(&Matrix4f::IDENTITY);
    assert_eq!(q, Quaternion::IDENTITY);
}

#[test]
fn java_float_strings() {
    assert_eq!(java_float_to_string(1.0), "1.0");
    assert_eq!(java_float_to_string(0.25), "0.25");
    assert_eq!(java_float_to_string(1.0e7), "1.0E7");
    assert_eq!(java_float_to_string(0.0001), "1.0E-4");
    assert_eq!(java_float_to_string(-12.5), "-12.5");
}

#[test]
fn variable_slots_convert_like_the_original() {
    let mut v = MapVariableSource::new();
    v.set_variable_string("Weapon", "rifle");
    v.set_variable_float("speed", 2.0);
    v.set_variable_bool("isMoving", true);
    assert_eq!(v.get_variable_string("weapon"), "rifle");
    assert_eq!(v.get_variable_string("SPEED"), "2.0");
    assert_eq!(v.get_variable_float("isMoving", 0.0), 1.0);
    // Setting a string on a float slot parses it.
    v.set_variable_string("speed", " 3.5 ");
    assert_eq!(v.get_variable_float("speed", 0.0), 3.5);
    assert_eq!(v.get_variable_float("missing", 7.0), 7.0);
}

fn write(dir: &Path, rel: &str, body: &str) {
    let p = dir.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, body).unwrap();
}

fn temp_dir(name: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("pz-anim-test-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

#[test]
fn x_extends_merges_by_name_and_position() {
    let d = temp_dir("xml");
    write(
        &d,
        "base.xml",
        r#"<animNode><m_Name>base</m_Name><m_AnimName>A</m_AnimName><m_BlendTime>0.2</m_BlendTime>
           <m_Conditions x_name="c1"><m_Name>v</m_Name><m_Type>BOOL</m_Type><m_Value>true</m_Value></m_Conditions>
           <m_Events><m_EventName>E1</m_EventName><m_TimePc>0.5</m_TimePc></m_Events></animNode>"#,
    );
    write(
        &d,
        "child.xml",
        r#"<animNode x_extends="base.xml"><m_Name>child</m_Name>
           <m_Conditions x_name="c1"><m_Value>false</m_Value></m_Conditions>
           <m_Conditions><m_Name>w</m_Name><m_Type>STRING</m_Type><m_Value>x</m_Value></m_Conditions>
           <m_Events><m_TimePc>0.25</m_TimePc></m_Events></animNode>"#,
    );
    let n = AnimNode::from_element(&xml::parse_xml_file(&d.join("child.xml")).unwrap());
    assert_eq!(n.name, "child");
    assert_eq!(n.anim_name, "A");
    assert_eq!(n.blend_time, 0.2);
    assert_eq!(n.conditions.len(), 2);
    assert_eq!(n.conditions[0].name, "v");
    assert!(!n.conditions[0].bool_value);
    // The unnamed event overrides the parent's first event by position.
    assert_eq!(n.events.len(), 1);
    assert_eq!(n.events[0].event_name, "E1");
    assert_eq!(n.events[0].time_pc, 0.25);
}

#[test]
fn state_picks_best_passing_nodes() {
    let mut st = AnimState::new("idle");
    let node = |xml: &str| AnimNode::from_element(&xml::parse(xml).unwrap());
    st.add_node(node("<animNode><m_Name>plain</m_Name><m_AnimName>A</m_AnimName></animNode>"));
    st.add_node(node(
        "<animNode><m_Name>armed</m_Name><m_AnimName>B</m_AnimName><m_Conditions><m_Name>Weapon</m_Name><m_Type>STRING</m_Type><m_Value>rifle</m_Value></m_Conditions></animNode>",
    ));
    let mut v = MapVariableSource::new();
    let names = |v: &MapVariableSource| st.get_anim_nodes(v, false).iter().map(|n| n.name.clone()).collect::<Vec<_>>();
    assert_eq!(names(&v), ["plain"]);
    v.set_variable_string("Weapon", "Rifle");
    assert_eq!(names(&v), ["armed"]);
}

#[test]
fn blend_picker_weights_inside_triangle() {
    let n = AnimNode::from_element(
        &xml::parse(
            r#"<animNode><m_Name>w</m_Name><m_AnimName>A</m_AnimName>
            <m_2DBlends referenceID="1"><m_AnimName>A</m_AnimName><m_XPos>0</m_XPos><m_YPos>0</m_YPos></m_2DBlends>
            <m_2DBlends referenceID="2"><m_AnimName>B</m_AnimName><m_XPos>1</m_XPos><m_YPos>0</m_YPos></m_2DBlends>
            <m_2DBlends referenceID="3"><m_AnimName>C</m_AnimName><m_XPos>0</m_XPos><m_YPos>1</m_YPos></m_2DBlends>
            <m_2DBlendTri><node1>1</node1><node2>2</node2><node3>3</node3></m_2DBlendTri></animNode>"#,
        )
        .unwrap(),
    );
    let picker = n.blend2d_picker.as_ref().unwrap();
    let r = picker.pick(&n.blends2d, 0.25, 0.25);
    assert_eq!(r.num_nodes, 3);
    assert!((r.scale1 - 0.5).abs() < 1e-6 && (r.scale2 - 0.25).abs() < 1e-6 && (r.scale3 - 0.25).abs() < 1e-6);
    // Outside the hull it projects onto the nearest edge.
    let r = picker.pick(&n.blends2d, 2.0, 0.0);
    assert_eq!(r.num_nodes, 1);
}

#[test]
fn animator_selects_nodes_fires_events_and_blends_between_states() {
    let d = temp_dir("animset");
    write(
        &d,
        "idle/Idle.xml",
        r#"<animNode><m_Name>Idle</m_Name><m_AnimName>idleclip</m_AnimName><m_BlendTime>0.5</m_BlendTime>
           <m_Events><m_EventName>Half</m_EventName><m_TimePc>0.5</m_TimePc><m_ParameterValue>p</m_ParameterValue></m_Events></animNode>"#,
    );
    write(&d, "walk/Walk.xml", r#"<animNode><m_Name>Walk</m_Name><m_AnimName>walkclip</m_AnimName><m_BlendTime>0.5</m_BlendTime></animNode>"#);
    let set = Rc::new(AnimationSet::load("test", &d));
    assert!(set.contains_state("IDLE") && set.contains_state("walk"));

    let mut player = player_with(&[slide("idleclip", 0, 1.0), slide("walkclip", 0, 3.0)]);
    let mut anim = AdvancedAnimator::new();
    anim.set_anim_set(Some(set));
    let mut vars = MapVariableSource::new();
    anim.set_state(&mut player, &mut vars, "idle", &[]);
    let dt = 0.1;
    let mut fired = Vec::new();
    for _ in 0..10 {
        anim.animate(&mut player, &mut vars, dt);
        fired.extend(anim.events.drain(..).map(|e| (e.state_name.clone().unwrap_or_default(), e.event.event_name.clone())));
    }
    assert!(fired.contains(&("idle".into(), "Half".into())), "{fired:?}");
    assert!(fired.contains(&("idle".into(), "ActiveAnimLooped".into())), "{fired:?}");
    assert_eq!(anim.current_state_name(), Some("idle"));
    assert_eq!(player.track_count(), 1);

    anim.set_state(&mut player, &mut vars, "walk", &[]);
    anim.animate(&mut player, &mut vars, dt);
    // Both nodes live while the walk node blends in.
    assert_eq!(player.track_count(), 2);
    for _ in 0..10 {
        anim.animate(&mut player, &mut vars, dt);
    }
    // After the blend the idle node is gone.
    anim.animate(&mut player, &mut vars, dt);
    assert_eq!(player.track_count(), 1);
    assert_eq!(player.tracks()[0].name(), "walkclip");
}
