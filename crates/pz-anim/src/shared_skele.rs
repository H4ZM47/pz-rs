//! `sharedskele`: model-space bone matrices of a single unblended track
//! baked at a fixed rate and shared between characters
//! (`SharedSkeleAnimationTrack`, `SharedSkeleAnimationRepository`,
//! `ModelTransformSampler`, `AnimTrackSampler`).

use std::collections::HashMap;
use std::rc::Rc;

use crate::bone_transform::BoneTransform;
use crate::clip::AnimClip;
use crate::math::{helpers, pzmath, Matrix4f, Vector3f, PI_F};
use crate::skeleton::Skeleton;
use crate::track::AnimationTrack;

/// `AnimTrackSampler`.
pub trait AnimTrackSampler {
    fn total_time(&self) -> f32;
    fn is_looped(&self) -> bool;
    fn move_to_time(&mut self, time: f32);
    fn current_time(&self) -> f32;
    fn bone_matrix(&mut self, bone: usize) -> Matrix4f;
    fn num_bones(&self) -> usize;
}

/// `SharedSkeleAnimationRepository`, keyed by clip identity.
#[derive(Default)]
pub struct SharedSkeleRepository {
    tracks: HashMap<*const AnimClip, (Rc<AnimClip>, Rc<SharedSkeleAnimationTrack>)>,
}

impl SharedSkeleRepository {
    pub fn get(&self, clip: &Rc<AnimClip>) -> Option<Rc<SharedSkeleAnimationTrack>> {
        self.tracks.get(&Rc::as_ptr(clip)).map(|(_, t)| t.clone())
    }

    pub fn set(&mut self, clip: &Rc<AnimClip>, track: Rc<SharedSkeleAnimationTrack>) {
        self.tracks.insert(Rc::as_ptr(clip), (clip.clone(), track));
    }
}

/// `SharedSkeleAnimationTrack`.
#[derive(Clone, Debug)]
pub struct SharedSkeleAnimationTrack {
    num_frames: usize,
    total_time: f32,
    is_looped: bool,
    /// Per bone, `num_frames` matrices in `Matrix4f.store` order.
    bone_tracks: Vec<Vec<[f32; 16]>>,
    /// `DebugOptions...sharedSkeles.allowLerping`.
    pub allow_lerping: bool,
}

impl Default for SharedSkeleAnimationTrack {
    fn default() -> Self {
        SharedSkeleAnimationTrack { num_frames: 0, total_time: 0.0, is_looped: false, bone_tracks: Vec::new(), allow_lerping: true }
    }
}

impl SharedSkeleAnimationTrack {
    /// `set(sampler, fps)`.
    pub fn set(&mut self, sampler: &mut dyn AnimTrackSampler, fps: f32) {
        let total = sampler.total_time();
        self.total_time = total;
        self.num_frames = ((total * fps + 0.99) as i32).max(1) as usize;
        self.is_looped = sampler.is_looped();
        let nb = sampler.num_bones();
        self.bone_tracks = vec![vec![[0.0; 16]; self.num_frames]; nb];
        let inc = total / (self.num_frames - 1) as f32;
        for f in 0..self.num_frames {
            sampler.move_to_time(inc * f as f32);
            for b in 0..nb {
                self.bone_tracks[b][f] = sampler.bone_matrix(b).store();
            }
        }
    }

    pub fn num_bones(&self) -> usize {
        self.bone_tracks.len()
    }

    /// `moveToTime(t)` + `getBoneMatrix(bone)`.
    pub fn bone_matrix_at(&self, bone: usize, t: f32) -> Matrix4f {
        let Some(track) = self.bone_tracks.get(bone) else { return Matrix4f::IDENTITY };
        let alpha = t / self.total_time;
        let fi = alpha * (self.num_frames - 1) as f32;
        let raw = fi as i32;
        let a = fi - raw as f32;
        let n = self.num_frames as i32;
        let (frame, next) = if self.is_looped {
            let f = raw % n;
            (f, (f + 1) % n)
        } else {
            let f = pzmath::clamp_i(raw, 0, n - 1);
            (f, pzmath::clamp_i(f + 1, 0, n - 1))
        };
        let cur = &track[frame.max(0) as usize];
        if frame != next && self.allow_lerping {
            let nx = &track[next.max(0) as usize];
            let mut v = [0.0; 16];
            for i in 0..16 {
                v[i] = pzmath::lerp(cur[i], nx[i], a);
            }
            Matrix4f::load(&v)
        } else {
            Matrix4f::load(cur)
        }
    }
}

/// `ModelTransformSampler`: samples one track (without blending) into
/// model-space transforms.
pub struct ModelTransformSampler {
    skeleton: Rc<Skeleton>,
    track: AnimationTrack,
    current_time: f32,
    counter_rotation_bone: Option<usize>,
    zero_counter_rotation_bone: bool,
    bone_transforms: Vec<BoneTransform>,
    bone_model_transforms: Vec<Matrix4f>,
}

impl ModelTransformSampler {
    pub fn new(skeleton: Rc<Skeleton>, track: &AnimationTrack, counter_rotation_bone: Option<usize>, zero_counter_rotation_bone: bool) -> Self {
        let n = skeleton.num_bones();
        ModelTransformSampler {
            skeleton,
            track: track.create_clone(0),
            current_time: 0.0,
            counter_rotation_bone,
            zero_counter_rotation_bone,
            bone_transforms: vec![BoneTransform::default(); n],
            bone_model_transforms: vec![Matrix4f::IDENTITY; n],
        }
    }

    fn update_bone_animation_transform(&mut self, bone: usize) {
        let (mut pos, mut rot, scale) = self.track.get(bone);
        if self.counter_rotation_bone == Some(bone) && self.track.use_deferred_rotation() {
            if self.zero_counter_rotation_bone {
                let mut m = Matrix4f::IDENTITY;
                m.rotate(-PI_F / 2.0, Vector3f::new(0.0, 1.0, 0.0));
                m.rotate(-PI_F / 2.0, Vector3f::new(1.0, 0.0, 0.0));
                rot = helpers::get_rotation(&m);
            } else {
                let e = helpers::to_euler_angles(&rot);
                rot = helpers::to_quaternion(e.x as f64, e.y as f64, 1.570_796_370_506_286_6);
            }
        }
        if self.track.deferred_movement_bone_idx() == bone as i32 {
            let d = self.track.current_deferred_counter_position();
            pos.x += d.x;
            pos.y += d.y;
            pos.z += d.z;
        }
        self.bone_transforms[bone].set_prs(pos, rot, scale);
    }
}

impl AnimTrackSampler for ModelTransformSampler {
    fn total_time(&self) -> f32 {
        self.track.get_duration()
    }

    fn is_looped(&self) -> bool {
        self.track.is_looping()
    }

    fn move_to_time(&mut self, time: f32) {
        self.current_time = time;
        self.track.set_current_time_value(time);
        self.track.update(0.0, None, &mut Vec::new());
        for b in 0..self.bone_transforms.len() {
            self.update_bone_animation_transform(b);
        }
    }

    fn current_time(&self) -> f32 {
        self.current_time
    }

    fn bone_matrix(&mut self, bone: usize) -> Matrix4f {
        if bone == 0 {
            self.bone_model_transforms[0] = self.bone_transforms[0].matrix();
            return self.bone_model_transforms[0];
        }
        let local = self.bone_transforms[bone].matrix();
        let m = match self.skeleton.parent(bone) {
            Some(p) => Matrix4f::mul(&local, &self.bone_model_transforms[p]),
            None => local,
        };
        self.bone_model_transforms[bone] = m;
        m
    }

    fn num_bones(&self) -> usize {
        self.skeleton.num_bones()
    }
}
