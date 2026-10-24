//! `AnimationClip` and `Keyframe` as the sampler uses them: keyframes
//! grouped per bone, with the duplicate last frame dropped unless the clip
//! keeps it (`keepLastFrame`).

use crate::math::{pzmath, Quaternion, Vector2, Vector3f};
use crate::BoneAxis;

/// The original sizes its per-bone tables to 60 bones.
pub const MAX_BONES: usize = 60;

/// `Keyframe`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Keyframe {
    pub bone: i32,
    pub time: f32,
    pub position: Vector3f,
    pub rotation: Quaternion,
    pub scale: Vector3f,
}

impl Keyframe {
    pub fn identity() -> Keyframe {
        Keyframe { bone: 0, time: -1.0, position: Vector3f::ZERO, rotation: Quaternion::IDENTITY, scale: Vector3f::ONE }
    }

    pub fn from_model(k: &pz_model::Keyframe) -> Keyframe {
        Keyframe {
            bone: k.bone,
            time: k.time,
            position: Vector3f::from_array(k.position),
            rotation: Quaternion::from_model(k.rotation),
            scale: Vector3f::from_array(k.scale),
        }
    }

    /// `Keyframe.lerp(a, b, time, pos, rot, scale)`.
    pub fn lerp(a: &Keyframe, b: &Keyframe, time: f32) -> (Vector3f, Quaternion, Vector3f) {
        if b.time == a.time {
            return (b.position, b.rotation, b.scale);
        }
        let del = (time - a.time) / (b.time - a.time);
        (
            pzmath::lerp_v3(a.position, b.position, del),
            pzmath::slerp(a.rotation, b.rotation, del),
            pzmath::lerp_v3(a.scale, b.scale, del),
        )
    }
}

/// `AnimationClip`.
#[derive(Clone, Debug)]
pub struct AnimClip {
    pub name: String,
    pub is_ragdoll: bool,
    pub keep_last_frame: bool,
    pub duration: f32,
    pub keyframes: Vec<Keyframe>,
    /// Bone name per keyframe (parallel to `keyframes`).
    pub keyframe_bone_names: Vec<String>,
    by_bone: Vec<Vec<Keyframe>>,
}

impl AnimClip {
    pub fn new(name: &str, duration: f32, keyframes: Vec<Keyframe>, names: Vec<String>, keep_last_frame: bool, is_ragdoll: bool) -> AnimClip {
        let mut clip = AnimClip {
            name: name.to_string(),
            is_ragdoll,
            keep_last_frame,
            duration,
            keyframes,
            keyframe_bone_names: names,
            by_bone: Vec::new(),
        };
        clip.recalculate_keyframes_by_bone_index();
        clip
    }

    pub fn from_model(c: &pz_model::AnimationClip) -> AnimClip {
        AnimClip::new(
            &c.name,
            c.duration,
            c.keyframes.iter().map(Keyframe::from_model).collect(),
            c.keyframes.iter().map(|k| k.bone_name.clone()).collect(),
            c.keep_last_frame,
            c.is_ragdoll,
        )
    }

    /// `recalculateKeyframesByBoneIndex`.
    pub fn recalculate_keyframes_by_bone_index(&mut self) {
        self.by_bone = (0..MAX_BONES as i32)
            .map(|bone| {
                let mut v: Vec<Keyframe> = self.keyframes.iter().filter(|k| k.bone == bone).copied().collect();
                if !self.keep_last_frame && v.len() > 1 {
                    v.pop();
                }
                v
            })
            .collect();
    }

    /// `getBoneFramesAt`.
    pub fn bone_frames(&self, bone: usize) -> &[Keyframe] {
        self.by_bone.get(bone).map(|v| v.as_slice()).unwrap_or(&[])
    }

    fn frames_for_bone_name(&self, bone_name: &str) -> Vec<Keyframe> {
        let mut v: Vec<Keyframe> = self
            .keyframes
            .iter()
            .zip(&self.keyframe_bone_names)
            .filter(|(_, n)| n.eq_ignore_ascii_case(bone_name))
            .map(|(k, _)| *k)
            .collect();
        if !self.keep_last_frame && v.len() > 1 {
            v.pop();
        }
        v
    }

    /// `getTotalTranslation`.
    pub fn total_translation(&self, bone_name: &str, axis: BoneAxis) -> Vector2 {
        let f = self.frames_for_bone_name(bone_name);
        let (Some(first), Some(last)) = (f.first(), f.last()) else {
            return Vector2::default();
        };
        let x = last.position.x - first.position.x;
        let y = if axis == BoneAxis::Y {
            -last.position.z + first.position.z
        } else {
            last.position.y - first.position.y
        };
        Vector2::new(x, y)
    }

    /// `getTranslationLength`.
    pub fn translation_length(&self, bone_name: &str, axis: BoneAxis) -> f32 {
        let v = self.total_translation(bone_name, axis);
        ((v.x * v.x + v.y * v.y) as f64).sqrt() as f32
    }
}
