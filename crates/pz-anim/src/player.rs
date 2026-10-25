//! `AnimationPlayer` (+ `AnimationMultiTrack`, `LiveAnimationTrackEntries`):
//! blends the playing tracks into per-bone local transforms, applies
//! twist bones, bone reparenting and the counter-rotation bone, computes
//! the model-space transforms and the skin palette, and accumulates
//! deferred (root) motion.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use pz_model::{Mat4, SkinningData};

use crate::bone_transform::AnimatorsBoneTransform;
use crate::clip::{AnimClip, Keyframe};
use crate::math::{helpers, pzmath, Matrix4f, Quaternion, Vector2, Vector3f, PI_F};
use crate::shared_skele::{ModelTransformSampler, SharedSkeleAnimationTrack, SharedSkeleRepository};
use crate::skeleton::Skeleton;
use crate::track::{AnimationTrack, TrackEvent, TrackId, TrackListener};
use crate::vars::AnimationVariableSource;
use crate::{AnimBoneWeight, BoneAxis};

/// The debug switches the animation code reads, with the original's
/// defaults.
#[derive(Clone, Debug)]
pub struct AnimDebugOptions {
    pub no_bone_masks: bool,
    pub no_bone_twists: bool,
    pub zero_counter_rotation_bone: bool,
    pub deferred_rotations_only: bool,
    pub shared_skeles_enabled: bool,
    pub shared_skeles_allow_lerping: bool,
    pub allow_early_transition_out: bool,
    pub disable_animation_blends: bool,
    pub allow_anim_node_override: bool,
}

impl Default for AnimDebugOptions {
    fn default() -> Self {
        AnimDebugOptions {
            no_bone_masks: false,
            no_bone_twists: false,
            zero_counter_rotation_bone: false,
            deferred_rotations_only: false,
            shared_skeles_enabled: true,
            shared_skeles_allow_lerping: true,
            allow_early_transition_out: true,
            disable_animation_blends: false,
            allow_anim_node_override: false,
        }
    }
}

/// `StartAnimTrackParameters`.
#[derive(Clone, Debug)]
pub struct StartAnimTrackParameters {
    pub anim_name: String,
    pub priority: i32,
    pub is_primary: bool,
    pub is_ragdoll: bool,
    pub ragdoll_start_time: f32,
    pub ragdoll_max_time: f32,
    pub sub_layer_bone_weights: Vec<AnimBoneWeight>,
    pub sync_tracking_enabled: bool,
    pub track_time_to_variable: Option<String>,
    pub speed_scale: f32,
    pub initial_weight: f32,
    pub is_looped: bool,
    pub is_reversed: bool,
    pub deferred_bone_name: String,
    pub deferred_bone_axis: BoneAxis,
    pub use_deferred_movement: bool,
    pub use_deferred_rotation: bool,
    pub deferred_rotation_scale: f32,
    pub matching_grappled_anim_node: String,
}

impl Default for StartAnimTrackParameters {
    fn default() -> Self {
        StartAnimTrackParameters {
            anim_name: String::new(),
            priority: 0,
            is_primary: false,
            is_ragdoll: false,
            ragdoll_start_time: -1.0,
            ragdoll_max_time: -1.0,
            sub_layer_bone_weights: Vec::new(),
            sync_tracking_enabled: false,
            track_time_to_variable: None,
            speed_scale: 1.0,
            initial_weight: 0.0,
            is_looped: false,
            is_reversed: false,
            deferred_bone_name: String::new(),
            deferred_bone_axis: BoneAxis::Y,
            use_deferred_movement: true,
            use_deferred_rotation: false,
            deferred_rotation_scale: 1.0,
            matching_grappled_anim_node: String::new(),
        }
    }
}

/// The `AnimLayer` a track is started for.
#[derive(Clone, Copy, Debug)]
pub struct LayerBinding {
    pub id: u32,
    pub depth: i32,
    pub is_sub_layer: bool,
}

/// `LiveAnimationTrackEntry`.
#[derive(Clone, Copy, Debug)]
struct LiveEntry {
    track: usize,
    weight: f32,
    priority: i32,
    layer: i32,
}

/// `LiveAnimationTrackEntries`.
#[derive(Clone, Debug, Default)]
struct LiveAnimationTrackEntries {
    entries: Vec<LiveEntry>,
}

const LIVE_ENTRY_CACHE: usize = 32;
const MAX_LAYERS: usize = 16;

impl LiveAnimationTrackEntries {
    fn set_tracks(&mut self, tracks: &[AnimationTrack], min_weight: f32, normalize_first_layer: bool) {
        self.entries.clear();
        for (i, t) in tracks.iter().enumerate() {
            let w = t.blend_weight();
            let layer = t.layer_depth;
            let prio = t.priority();
            if !(0..MAX_LAYERS as i32).contains(&layer) {
                continue;
            }
            if w < min_weight || layer > 0 && t.is_finished() {
                continue;
            }
            let mut insert_at = None;
            for j in 0..LIVE_ENTRY_CACHE {
                let Some(e) = self.entries.get(j) else {
                    insert_at = Some(j);
                    break;
                };
                if layer > e.layer {
                    continue;
                }
                if layer < e.layer {
                    insert_at = Some(j);
                    break;
                }
                if prio > e.priority {
                    continue;
                }
                if prio < e.priority {
                    insert_at = Some(j);
                    break;
                }
                if !(w < e.weight) {
                    continue;
                }
                insert_at = Some(j);
                break;
            }
            let Some(at) = insert_at else { continue };
            self.entries.insert(at, LiveEntry { track: i, weight: w, priority: prio, layer });
            self.entries.truncate(LIVE_ENTRY_CACHE);
        }
        if self.entries.is_empty() {
            return;
        }
        let mut counts = [0usize; MAX_LAYERS];
        let mut totals = [0f32; MAX_LAYERS];
        for e in &self.entries {
            totals[e.layer as usize] += e.weight;
            counts[e.layer as usize] += 1;
        }
        if normalize_first_layer {
            let layer_no = self.entries[0].layer;
            let count = counts[0];
            let total = totals[0];
            if total < 1.0 {
                for e in self.entries.iter_mut() {
                    if e.layer != layer_no {
                        break;
                    }
                    if total > 0.0 {
                        e.weight /= total;
                    } else {
                        e.weight = 1.0 / count as f32;
                    }
                }
            }
        }
    }
}

/// `AnimationPlayer`.
pub struct AnimationPlayer {
    skinning: Rc<SkinningData>,
    skeleton: Rc<Skeleton>,
    clip_cache: HashMap<String, Rc<AnimClip>>,
    bone_transforms_need_first_frame: bool,
    bone_transforms_time_delta: f32,
    pub bone_transforms: Vec<AnimatorsBoneTransform>,
    model_transforms: Vec<Matrix4f>,
    ragdoll_animation_clip: Option<Rc<AnimClip>>,
    shared_skele_repo: Option<Rc<RefCell<SharedSkeleRepository>>>,
    current_shared_track: Option<Rc<SharedSkeleAnimationTrack>>,
    current_shared_track_clip: Option<*const AnimClip>,
    angle: f32,
    target_angle: f32,
    character_allows_twist: bool,
    twist_angle: f32,
    shoulder_twist_angle: f32,
    shoulder_twist_weight: f32,
    target_twist_angle: f32,
    max_twist_angle: f32,
    excess_twist: f32,
    pub angle_step_delta: f32,
    pub angle_twist_delta: f32,
    pub do_blending: bool,
    pub update_bones: bool,
    target_dir: Vector2,
    reparented_bone_bindings: Vec<(String, String)>,
    twist_bones: Vec<String>,
    counter_rotation_bone: Option<String>,
    pub dismembered: Vec<i32>,
    live_entries: LiveAnimationTrackEntries,
    deferred_movement: Vector2,
    deferred_movement_accum: Vector2,
    deferred_movement_from_ragdoll: Vector3f,
    deferred_rotation_weight: f32,
    deferred_angle_delta: f32,
    target_grapple_pos: Vector3f,
    target_grapple_rotation: Vector2,
    grapple_offset: Vector3f,
    tracks: Vec<AnimationTrack>,
    next_track_id: TrackId,
    /// Listener callbacks queued by the last [`AnimationPlayer::update`],
    /// for the owner (the animator) to dispatch.
    pub events: Vec<TrackEvent>,
    /// `IsoGameCharacter.isAiming()` for the twist code.
    pub is_aiming: bool,
    pub debug: AnimDebugOptions,
    /// `PerformanceSettings.interpolateAnims`.
    pub interpolate_anims: bool,
    /// `GameTime.multiplierBias`.
    pub multiplier_bias: f32,
    /// `AdvancedAnimator.motionScale` (`Defaults.xml`).
    pub motion_scale: f32,
    /// `AdvancedAnimator.rotationScale` (`Defaults.xml`).
    pub rotation_scale: f32,
}

impl AnimationPlayer {
    /// `AnimationPlayer.alloc(model)` for a model whose `tag` is `skinning`.
    pub fn new(skinning: Rc<SkinningData>) -> AnimationPlayer {
        let skeleton = Rc::new(Skeleton::from_skinning_data(&skinning));
        let n = skeleton.num_bones();
        AnimationPlayer {
            skinning,
            skeleton,
            clip_cache: HashMap::new(),
            bone_transforms_need_first_frame: true,
            bone_transforms_time_delta: -1.0,
            bone_transforms: vec![AnimatorsBoneTransform::default(); n],
            model_transforms: vec![Matrix4f::IDENTITY; n],
            ragdoll_animation_clip: None,
            shared_skele_repo: None,
            current_shared_track: None,
            current_shared_track_clip: None,
            angle: 0.0,
            target_angle: 0.0,
            character_allows_twist: true,
            twist_angle: 0.0,
            shoulder_twist_angle: 0.0,
            shoulder_twist_weight: 1.0,
            target_twist_angle: 0.0,
            max_twist_angle: pzmath::deg_to_rad(70.0),
            excess_twist: 0.0,
            angle_step_delta: 1.0,
            angle_twist_delta: 1.0,
            do_blending: true,
            update_bones: true,
            target_dir: Vector2::default(),
            reparented_bone_bindings: Vec::new(),
            twist_bones: Vec::new(),
            counter_rotation_bone: None,
            dismembered: Vec::new(),
            live_entries: LiveAnimationTrackEntries::default(),
            deferred_movement: Vector2::default(),
            deferred_movement_accum: Vector2::default(),
            deferred_movement_from_ragdoll: Vector3f::ZERO,
            deferred_rotation_weight: 0.0,
            deferred_angle_delta: 0.0,
            target_grapple_pos: Vector3f::ZERO,
            target_grapple_rotation: Vector2::new(1.0, 0.0),
            grapple_offset: Vector3f::ZERO,
            tracks: Vec::new(),
            next_track_id: 1,
            events: Vec::new(),
            is_aiming: false,
            debug: AnimDebugOptions::default(),
            interpolate_anims: true,
            multiplier_bias: 1.0,
            motion_scale: 0.76,
            rotation_scale: 0.76,
        }
    }

    /// `setModel` with a different skinning data (`initSkinningData`).
    pub fn set_skinning_data(&mut self, skinning: Rc<SkinningData>) {
        if Rc::ptr_eq(&skinning, &self.skinning) {
            return;
        }
        self.reset();
        self.skeleton = Rc::new(Skeleton::from_skinning_data(&skinning));
        self.skinning = skinning;
        self.clip_cache.clear();
        let n = self.skeleton.num_bones();
        self.model_transforms.resize(n, Matrix4f::IDENTITY);
        self.bone_transforms.resize(n, AnimatorsBoneTransform::default());
        for b in &mut self.bone_transforms {
            b.set_identity();
        }
        self.bone_transforms_need_first_frame = true;
    }

    pub fn skinning_data(&self) -> &Rc<SkinningData> {
        &self.skinning
    }

    pub fn skeleton(&self) -> &Rc<Skeleton> {
        &self.skeleton
    }

    pub fn num_bones(&self) -> usize {
        self.bone_transforms.len()
    }

    /// `getSkinningBoneIndex`.
    pub fn skinning_bone_index(&self, name: &str, default: i32) -> i32 {
        self.skinning.bone_indices.get(name).copied().unwrap_or(default)
    }

    /// Adds (or replaces) a clip the player can play by name, alongside the
    /// skinning data's own clips.
    pub fn add_clip(&mut self, clip: Rc<AnimClip>) {
        self.clip_cache.insert(clip.name.clone(), clip);
    }

    /// `skinningData.animationClips.get(name)`.
    pub fn clip(&mut self, name: &str) -> Option<Rc<AnimClip>> {
        if let Some(c) = self.clip_cache.get(name) {
            return Some(c.clone());
        }
        let c = Rc::new(AnimClip::from_model(self.skinning.animation_clips.get(name)?));
        self.clip_cache.insert(name.to_string(), c.clone());
        Some(c)
    }

    // ---- set-up ----

    /// `addBoneReparent`.
    pub fn add_bone_reparent(&mut self, bone: &str, new_parent: &str) {
        if self
            .reparented_bone_bindings
            .iter()
            .any(|(a, b)| a.eq_ignore_ascii_case(bone) && b.eq_ignore_ascii_case(new_parent))
        {
            return;
        }
        self.reparented_bone_bindings.push((bone.to_string(), new_parent.to_string()));
    }

    /// `setTwistBones`.
    pub fn set_twist_bones(&mut self, bones: &[&str]) {
        self.twist_bones = bones.iter().map(|s| s.to_string()).collect();
    }

    pub fn num_twist_bones(&self) -> usize {
        self.twist_bones.len()
    }

    /// `setCounterRotationBone`.
    pub fn set_counter_rotation_bone(&mut self, bone: &str) {
        self.counter_rotation_bone = Some(bone.to_string());
    }

    fn counter_rotation_bone_index(&self) -> Option<usize> {
        self.counter_rotation_bone.as_deref().and_then(|n| self.skeleton.bone_index(n))
    }

    pub fn set_shared_anim_repo(&mut self, repo: Option<Rc<RefCell<SharedSkeleRepository>>>) {
        self.shared_skele_repo = repo;
    }

    // ---- tracks ----

    pub fn tracks(&self) -> &[AnimationTrack] {
        &self.tracks
    }

    pub fn track_count(&self) -> usize {
        self.tracks.len()
    }

    pub fn track(&self, id: TrackId) -> Option<&AnimationTrack> {
        self.tracks.iter().find(|t| t.id == id)
    }

    pub fn track_mut(&mut self, id: TrackId) -> Option<&mut AnimationTrack> {
        self.tracks.iter_mut().find(|t| t.id == id)
    }

    /// `startClip`.
    pub fn start_clip(&mut self, clip: Rc<AnimClip>, looped: bool, ragdoll_max_time: f32) -> TrackId {
        let id = self.next_track_id;
        self.next_track_id += 1;
        let mut track = AnimationTrack::new(id);
        track.interpolate = self.interpolate_anims;
        track.no_bone_masks = self.debug.no_bone_masks;
        let name = clip.name.clone();
        track.start_clip(clip, looped, ragdoll_max_time, None);
        track.set_name(&name);
        track.is_playing = true;
        self.tracks.push(track);
        id
    }

    /// `play(animName, looped, isRagdoll, ragdollMaxTime)`.
    pub fn play(&mut self, anim_name: &str, looped: bool, is_ragdoll: bool, ragdoll_max_time: f32) -> Option<TrackId> {
        let clip = if is_ragdoll { self.get_or_create_ragdoll_animation_clip() } else { self.clip(anim_name) }?;
        Some(self.start_clip(clip, looped, ragdoll_max_time))
    }

    /// `play(StartAnimTrackParameters, AnimLayer)`.
    pub fn play_params(&mut self, p: &StartAnimTrackParameters, layer: Option<LayerBinding>) -> Option<TrackId> {
        let id = self.play(&p.anim_name, p.is_looped, p.is_ragdoll, p.ragdoll_max_time)?;
        let skeleton = self.skeleton.clone();
        let deferred_bone = skeleton.bone_index(&p.deferred_bone_name);
        let t = self.track_mut(id).unwrap();
        t.is_primary = p.is_primary;
        if layer.is_some_and(|l| l.is_sub_layer) {
            t.set_bone_weights(Some(&p.sub_layer_bone_weights));
            t.init_bone_weights(&skeleton);
        } else {
            t.set_bone_weights(None);
        }
        t.set_speed_delta(p.speed_scale);
        t.sync_tracking_enabled = p.sync_tracking_enabled;
        t.track_time_to_variable = p.track_time_to_variable.clone();
        t.set_deferred_bone(deferred_bone, p.deferred_bone_axis);
        t.set_use_deferred_rotation(p.use_deferred_rotation);
        t.set_deferred_rotation_scale(p.deferred_rotation_scale);
        t.set_blend_weight(p.initial_weight);
        t.reverse = p.is_reversed;
        t.priority = p.priority;
        t.ragdoll_start_time = p.ragdoll_start_time;
        t.set_matching_grappled_anim_node(&p.matching_grappled_anim_node);
        match layer {
            Some(l) => t.set_anim_layer(Some(l.id), l.depth),
            None => t.set_anim_layer(None, 0),
        }
        Some(id)
    }

    /// `getOrCreateRagdollAnimationClip`. The ragdoll simulation itself is
    /// not ported (see [`crate::RagdollController`]); the placeholder clip
    /// is.
    pub fn get_or_create_ragdoll_animation_clip(&mut self) -> Option<Rc<AnimClip>> {
        if self.ragdoll_animation_clip.is_none() {
            let root = self.skeleton.find_bone_ignore_case("Dummy01")?;
            let mut bones = Vec::new();
            fn walk(s: &Skeleton, b: usize, out: &mut Vec<usize>) {
                out.push(b);
                for &c in &s.bones[b].children {
                    walk(s, c, out);
                }
            }
            walk(&self.skeleton, root, &mut bones);
            let mut keys = Vec::new();
            for b in bones {
                for t in [0.0, 1.0] {
                    keys.push(Keyframe { bone: b as i32, time: t, position: Vector3f::ZERO, rotation: Quaternion::IDENTITY, scale: Vector3f::ZERO });
                }
            }
            let names = vec![String::new(); keys.len()];
            self.ragdoll_animation_clip = Some(Rc::new(AnimClip::new("RagdollAnimationClip", 1.0, keys, names, true, true)));
        }
        self.ragdoll_animation_clip.clone()
    }

    /// `AnimationMultiTrack.removeTrack` + release: returns the listeners
    /// the original would notify with `onTrackDestroyed`.
    pub fn remove_track(&mut self, id: TrackId) -> Option<Vec<TrackListener>> {
        let i = self.tracks.iter().position(|t| t.id == id)?;
        let t = self.tracks.remove(i);
        Some(t.listeners)
    }

    /// `reset()` / `stopAll()`: drops every track without notifying.
    pub fn reset(&mut self) {
        self.tracks.clear();
    }

    pub fn stop_all(&mut self) {
        self.reset();
    }

    // ---- update ----

    /// `Update(deltaT)`. `vars` serves tracks driven by a variable
    /// (`m_TrackTimeToVariable`).
    pub fn update(&mut self, dt: f32, vars: Option<&dyn AnimationVariableSource>) {
        self.update_internal(dt, vars, None);
    }

    /// `Update(deltaT)` for a player whose `parentPlayer` is `parent`
    /// (attached models copy the parent's bone transforms).
    pub fn update_attached(&mut self, dt: f32, vars: Option<&dyn AnimationVariableSource>, parent: &AnimationPlayer) {
        self.update_internal(dt, vars, Some(parent));
    }

    fn update_internal(&mut self, dt: f32, vars: Option<&dyn AnimationVariableSource>, parent: Option<&AnimationPlayer>) {
        // updateRagdoll: no ragdoll controller without physics.
        self.multi_track_update(dt, vars);
        if !self.update_bones {
            self.update_animation_non_visual_only(dt, parent.is_some());
            return;
        }
        if self.tracks.is_empty() {
            return;
        }
        if let Some(shared) = self.determine_current_shared_skele_track(parent.is_some()) {
            let track_time = self.tracks[0].current_track_time();
            self.update_animation_shared_skele_track(&shared, dt, track_time, parent.is_some());
            return;
        }
        self.update_animation_standard(dt, parent);
    }

    /// `AnimationMultiTrack.Update`.
    fn multi_track_update(&mut self, dt: f32, vars: Option<&dyn AnimationVariableSource>) {
        let mut i = 0;
        while i < self.tracks.len() {
            let t = &mut self.tracks[i];
            t.update(dt, vars, &mut self.events);
            if t.current_clip.is_none() {
                self.tracks.remove(i);
                continue;
            }
            i += 1;
        }
    }

    fn determine_current_shared_skele_track(&mut self, has_parent: bool) -> Option<Rc<SharedSkeleAnimationTrack>> {
        let repo = self.shared_skele_repo.clone()?;
        if self.do_blending || !self.debug.shared_skeles_enabled || self.tracks.len() != 1 {
            return None;
        }
        if !pzmath::equal(self.twist_angle, 0.0, 114.59155) || has_parent {
            return None;
        }
        let t = &self.tracks[0];
        if t.is_ragdoll() || !pzmath::equal(t.blend_field_weight(), 0.0, 0.1) {
            return None;
        }
        let clip = t.current_clip.clone().unwrap();
        if self.current_shared_track_clip == Some(Rc::as_ptr(&clip)) {
            return self.current_shared_track.clone();
        }
        let existing = repo.borrow().get(&clip);
        let shared = match existing {
            Some(s) => s,
            None => {
                let mut sampler = ModelTransformSampler::new(self.skeleton.clone(), &self.tracks[0], self.counter_rotation_bone_index(), self.debug.zero_counter_rotation_bone);
                let mut s = SharedSkeleAnimationTrack::default();
                s.allow_lerping = self.debug.shared_skeles_allow_lerping;
                s.set(&mut sampler, 5.0);
                let s = Rc::new(s);
                repo.borrow_mut().set(&clip, s.clone());
                s
            }
        };
        self.current_shared_track_clip = Some(Rc::as_ptr(&clip));
        self.current_shared_track = Some(shared.clone());
        Some(shared)
    }

    fn update_animation_non_visual_only(&mut self, dt: f32, has_parent: bool) {
        self.update_multi_track_bone_transforms_deferred_movement_only(has_parent);
        self.do_angles(dt);
        self.calculate_deferred_movement();
    }

    fn update_animation_shared_skele_track(&mut self, shared: &SharedSkeleAnimationTrack, dt: f32, track_time: f32, has_parent: bool) {
        self.update_multi_track_bone_transforms_deferred_movement_only(has_parent);
        self.do_angles(dt);
        self.calculate_deferred_movement();
        for (i, m) in self.model_transforms.iter_mut().enumerate() {
            *m = shared.bone_matrix_at(i, track_time);
        }
    }

    fn update_animation_standard(&mut self, dt: f32, parent: Option<&AnimationPlayer>) {
        match parent {
            None => self.update_multi_track_bone_transforms(dt),
            Some(p) => {
                self.bone_transforms_need_first_frame = false;
                for (dst, src) in self.bone_transforms.iter_mut().zip(&p.bone_transforms) {
                    *dst = src.clone();
                }
            }
        }
        self.do_angles(dt);
        self.calculate_deferred_movement();
        self.update_twist_bone();
        self.apply_bone_reparenting();
        self.update_model_transforms();
    }

    fn update_layer_blend_weightings(&mut self) {
        self.live_entries.set_tracks(&self.tracks, 0.001, self.bone_transforms_need_first_frame);
    }

    fn update_multi_track_bone_transforms(&mut self, dt: f32) {
        self.bone_transforms_time_delta = dt;
        for b in &mut self.bone_transforms {
            b.next_frame(dt);
        }
        for m in &mut self.model_transforms {
            m.set_identity();
        }
        self.update_layer_blend_weightings();
        if self.live_entries.entries.is_empty() {
            return;
        }
        for bone in 0..self.bone_transforms.len() {
            if self.is_bone_reparented(bone) {
                continue;
            }
            self.update_bone_animation_transform(bone, None);
        }
        self.bone_transforms_need_first_frame = false;
    }

    /// `updateMultiTrackBoneTransforms_DeferredMovementOnly`.
    pub fn update_multi_track_bone_transforms_deferred_movement_only(&mut self, has_parent: bool) {
        self.deferred_movement_from_ragdoll = Vector3f::ZERO;
        if has_parent {
            return;
        }
        self.update_layer_blend_weightings();
        if self.live_entries.entries.is_empty() {
            return;
        }
        let mut bones: Vec<usize> = Vec::new();
        for t in &self.tracks {
            let b = t.deferred_movement_bone_idx();
            if b == -1 || bones.contains(&(b as usize)) {
                continue;
            }
            bones.push(b as usize);
        }
        for b in bones {
            self.update_bone_animation_transform(b, None);
        }
    }

    fn is_bone_reparented(&self, bone: usize) -> bool {
        self.reparented_bone_bindings.iter().any(|(a, _)| self.skeleton.bone_index(a) == Some(bone))
    }

    /// `updateBoneAnimationTransform_Internal`.
    fn update_bone_animation_transform(&mut self, bone: usize, reparent: Option<(usize, usize)>) {
        let is_cr_bone = self.counter_rotation_bone_index() == Some(bone);
        let mut key = Keyframe::identity();
        let mut total_weight = 0.0f32;
        let mut is_first = true;
        let mut remaining = 1.0f32;
        let count = self.live_entries.entries.len();
        let mut idx = count as isize - 1;
        while idx >= 0 && remaining > 0.0 && !(remaining <= 0.001) {
            let e = self.live_entries.entries[idx as usize];
            idx -= 1;
            let bw = self.tracks[e.track].bone_weight(bone);
            if bw <= 0.001 {
                continue;
            }
            let raw = e.weight * bw;
            if raw <= 0.001 {
                continue;
            }
            let anim_weight = pzmath::clamp(raw, 0.0, remaining);
            remaining -= raw;
            remaining = remaining.max(0.0);
            let (mut pos, mut rot, _scale) = self.get_track_transform(bone, e.track, reparent);
            let t = &self.tracks[e.track];
            if is_cr_bone && !t.is_ragdoll() && t.use_deferred_rotation() {
                if self.debug.zero_counter_rotation_bone {
                    let mut m = Matrix4f::IDENTITY;
                    m.rotate(-PI_F / 2.0, Vector3f::new(0.0, 1.0, 0.0));
                    m.rotate(-PI_F / 2.0, Vector3f::new(1.0, 0.0, 0.0));
                    rot = helpers::get_rotation(&m);
                } else {
                    let e = helpers::to_euler_angles(&rot);
                    rot = helpers::to_quaternion(e.x as f64, e.y as f64, 1.570_796_370_506_286_6, );
                }
            }
            if !t.is_ragdoll() && t.deferred_movement_bone_idx() == bone as i32 {
                let d = t.current_deferred_counter_position();
                pos.x += d.x;
                pos.y += d.y;
                pos.z += d.z;
            }
            if is_first {
                key.position = Vector3f::new(pos.x * anim_weight, pos.y * anim_weight, pos.z * anim_weight);
                key.rotation = rot;
                total_weight = anim_weight;
                is_first = false;
                continue;
            }
            let rot_weight = anim_weight / (anim_weight + total_weight);
            total_weight += anim_weight;
            key.position = Vector3f::new(
                key.position.x + pos.x * anim_weight,
                key.position.y + pos.y * anim_weight,
                key.position.z + pos.z * anim_weight,
            );
            key.rotation = pzmath::slerp(key.rotation, rot, rot_weight);
        }
        if remaining > 0.0 && !self.bone_transforms_need_first_frame {
            let (pos, rot, scale) = self.bone_transforms[bone].transform.prs();
            key.position = Vector3f::new(
                key.position.x + pos.x * remaining,
                key.position.y + pos.y * remaining,
                key.position.z + pos.z * remaining,
            );
            key.rotation = pzmath::slerp(rot, key.rotation, total_weight);
            key.scale = pzmath::lerp_v3(scale, key.scale, total_weight);
        }
        let bt = &mut self.bone_transforms[bone];
        bt.transform.set_prs(key.position, key.rotation, key.scale);
        bt.blend_weight = total_weight;
    }

    /// `getTrackTransform`.
    fn get_track_transform(&mut self, bone: usize, track: usize, reparent: Option<(usize, usize)>) -> (Vector3f, Quaternion, Vector3f) {
        // SkeletonBone.Bip01.index() is the enum ordinal, 1.
        if bone == 1 && !self.tracks[track].is_ragdoll() {
            let t = &mut self.tracks[track];
            if !t.is_initial_adjustment_calculated {
                // Only non-zero while ragdolling, which is not ported.
                t.initial_adjustment = Vector3f::ZERO;
                t.is_initial_adjustment_calculated = true;
            }
            let (mut pos, rot, scale) = t.get(bone);
            pos.x += t.initial_adjustment.x;
            pos.y -= t.initial_adjustment.z;
            return (pos, rot, scale);
        }
        let Some((bone_a, bone_b)) = reparent else {
            return self.tracks[track].get(bone);
        };
        let pa = Self::unweighted_bone_transform(&mut self.tracks[track], bone_a);
        let parent_a = self.skeleton.parent(bone_a).expect("reparented bone has no parent");
        let m_a = self.bone_model_transform(parent_a);
        let m_a_inv = m_a.invert().unwrap_or(Matrix4f::IDENTITY);
        let m_b = self.bone_model_transform(bone_b);
        let um_a = self.unweighted_model_transform(track, parent_a);
        let um_b = self.unweighted_model_transform(track, bone_b);
        let um_b_inv = um_b.invert().unwrap_or(Matrix4f::IDENTITY);
        let mut r = Matrix4f::mul(&pa, &um_a);
        r = Matrix4f::mul(&r, &um_b_inv);
        r = Matrix4f::mul(&r, &m_b);
        r = Matrix4f::mul(&r, &m_a_inv);
        (helpers::get_position(&r), helpers::get_rotation(&r), Vector3f::ONE)
    }

    fn unweighted_bone_transform(track: &mut AnimationTrack, bone: usize) -> Matrix4f {
        let (pos, mut rot, scale) = track.get(bone);
        helpers::create_from_qps(pos, &mut rot, scale)
    }

    /// `getUnweightedModelTransform`.
    fn unweighted_model_transform(&mut self, track: usize, bone: usize) -> Matrix4f {
        let mut m = Matrix4f::IDENTITY;
        let mut cur = Some(bone);
        while let Some(c) = cur {
            let b = Self::unweighted_bone_transform(&mut self.tracks[track], c);
            m = Matrix4f::mul(&m, &b);
            cur = self.skeleton.parent(c);
        }
        m
    }

    /// `getBoneModelTransform`.
    pub fn bone_model_transform(&mut self, bone: usize) -> Matrix4f {
        let mut m = Matrix4f::IDENTITY;
        let mut cur = Some(bone);
        while let Some(c) = cur {
            let b = self.bone_transforms[c].transform.matrix();
            m = Matrix4f::mul(&m, &b);
            cur = self.skeleton.parent(c);
        }
        m
    }

    /// `getBindPoseBoneModelTransform`.
    pub fn bind_pose_bone_model_transform(&self, bone: usize) -> Matrix4f {
        let mut m = Matrix4f::IDENTITY;
        let mut cur = Some(bone);
        while let Some(c) = cur {
            m = Matrix4f::mul(&m, &self.skeleton.bind_pose[c]);
            cur = self.skeleton.parent(c);
        }
        m
    }

    fn apply_bone_reparenting(&mut self) {
        for i in 0..self.reparented_bone_bindings.len() {
            let (a, b) = &self.reparented_bone_bindings[i];
            let (Some(a), Some(b)) = (self.skeleton.bone_index(a), self.skeleton.bone_index(b)) else { continue };
            self.update_bone_animation_transform(a, Some((a, b)));
        }
    }

    fn update_model_transforms(&mut self) {
        if self.model_transforms.is_empty() {
            return;
        }
        self.model_transforms[0] = self.bone_transforms[0].transform.matrix();
        for i in 1..self.model_transforms.len() {
            let local = self.bone_transforms[i].transform.matrix();
            self.model_transforms[i] = match self.skeleton.parent(i) {
                Some(p) => Matrix4f::mul(&local, &self.model_transforms[p]),
                None => local,
            };
        }
    }

    /// `transformRootChildBones`.
    pub fn transform_root_child_bones(&mut self, bone_name: &str, mut rotation: Quaternion) {
        let rm = helpers::create_from_quaternion(&mut rotation);
        if let Some(i) = (0..self.bone_transforms.len()).find(|&i| self.skeleton.bones[i].name.as_deref().is_some_and(|n| n.eq_ignore_ascii_case(bone_name))) {
            let b = self.bone_transforms[i].transform.matrix();
            self.bone_transforms[i].transform.mul_into(&rm, &b);
        }
        self.update_model_transforms();
    }

    /// `resetBoneModelTransforms`.
    pub fn reset_bone_model_transforms(&mut self) {
        self.bone_transforms_need_first_frame = true;
        self.bone_transforms_time_delta = -1.0;
        for b in &mut self.bone_transforms {
            b.reset();
        }
        for m in &mut self.model_transforms {
            m.set_identity();
        }
    }

    pub fn bone_transforms_need_first_frame(&self) -> bool {
        self.bone_transforms_need_first_frame
    }

    pub fn bone_transforms_time_delta(&self) -> f32 {
        self.bone_transforms_time_delta
    }

    // ---- angles and twist ----

    pub fn calculate_anim_player_angle(dx: f32, dy: f32) -> f32 {
        Vector2::direction(dx, dy)
    }

    /// `setTargetDirection`.
    pub fn set_target_direction(&mut self, dx: f32, dy: f32) {
        if self.target_dir.x != dx || self.target_dir.y != dy {
            self.target_angle = Self::calculate_anim_player_angle(dx, dy);
            self.target_twist_angle = pzmath::get_closest_angle(self.angle, self.target_angle);
            let clamped = pzmath::clamp(self.target_twist_angle, -self.max_twist_angle, self.max_twist_angle);
            self.excess_twist = pzmath::get_closest_angle(clamped, self.target_twist_angle);
            self.target_dir = Vector2::new(dx, dy);
        }
    }

    /// `setTargetAndCurrentDirection`.
    pub fn set_target_and_current_direction(&mut self, dx: f32, dy: f32) {
        self.target_angle = Self::calculate_anim_player_angle(dx, dy);
        self.angle = self.target_angle;
        self.target_twist_angle = 0.0;
        self.target_dir = Vector2::new(dx, dy);
    }

    /// `updateForwardDirection(character)` with the character's values.
    pub fn update_forward_direction(&mut self, dx: f32, dy: f32, allows_twist: bool, shoulder_twist_weight: f32) {
        self.set_target_direction(dx, dy);
        self.character_allows_twist = allows_twist;
        self.shoulder_twist_weight = shoulder_twist_weight;
    }

    /// `DoAngles`.
    pub fn do_angles(&mut self, dt: f32) {
        let step = 0.15 * (dt * 0.8 * self.multiplier_bias * 60.0);
        let step = pzmath::min(step, PI_F);
        self.interpolate_body_angle(step);
        self.interpolate_body_twist(step);
        self.interpolate_shoulder_twist(step);
    }

    fn interpolate_body_angle(&mut self, base: f32) {
        let target = self.target_angle;
        let diff = pzmath::get_closest_angle(self.angle, target);
        if pzmath::equal(diff, 0.0, 0.001) {
            self.angle = self.target_angle;
            self.target_twist_angle = 0.0;
            return;
        }
        let diff_sign = pzmath::sign(diff);
        let undeferred = base * diff_sign as f32 * self.angle_step_delta;
        let step = if self.debug.deferred_rotations_only {
            self.deferred_angle_delta
        } else if self.deferred_rotation_weight > 0.0 {
            self.deferred_angle_delta * self.deferred_rotation_weight + undeferred * (1.0 - self.deferred_rotation_weight)
        } else {
            undeferred
        };
        let step_sign = pzmath::sign(step);
        let next = self.angle + step;
        let new_diff = pzmath::get_closest_angle(next, target);
        let new_sign = pzmath::sign(new_diff);
        if new_sign == diff_sign || step_sign != diff_sign {
            self.angle = next;
            self.target_twist_angle = new_diff;
        } else {
            self.angle = self.target_angle;
            self.target_twist_angle = 0.0;
        }
    }

    fn interpolate_body_twist(&mut self, base: f32) {
        let unclamped = pzmath::wrap_range(self.target_twist_angle, -PI_F, PI_F);
        let target = pzmath::clamp(unclamped, -self.max_twist_angle, self.max_twist_angle);
        self.excess_twist = pzmath::get_closest_angle(target, unclamped);
        let diff = pzmath::get_closest_angle(self.twist_angle, target);
        if pzmath::equal(diff, 0.0, 0.001) {
            self.twist_angle = target;
            return;
        }
        let sign = pzmath::sign(diff);
        let step = base * sign as f32 * pzmath::abs(self.angle_twist_delta);
        let next = self.twist_angle + step;
        let nd = pzmath::get_closest_angle(next, target);
        self.twist_angle = if pzmath::sign(nd) == sign { next } else { target };
    }

    fn interpolate_shoulder_twist(&mut self, base: f32) {
        let target = pzmath::wrap_range(self.twist_angle, -PI_F, PI_F);
        let diff = pzmath::get_closest_angle(self.shoulder_twist_angle, target);
        if pzmath::equal(diff, 0.0, 0.001) {
            self.shoulder_twist_angle = target;
            return;
        }
        let sign = pzmath::sign(diff);
        let step = base * sign as f32 * pzmath::abs(self.angle_twist_delta) * 0.55;
        let next = self.shoulder_twist_angle + step;
        let nd = pzmath::get_closest_angle(next, target);
        self.shoulder_twist_angle = if pzmath::sign(nd) == sign { next } else { target };
    }

    fn update_twist_bone(&mut self) {
        if self.twist_bones.is_empty() || self.debug.no_bone_twists || !self.character_allows_twist {
            return;
        }
        let count = self.twist_bones.len();
        let head_idx = count - 1;
        let mut shoulder_idx = self.twist_bones.iter().position(|b| b.eq_ignore_ascii_case("Bip01_Spine1")).map(|i| i as isize).unwrap_or(-1);
        if shoulder_idx < 0 {
            shoulder_idx = head_idx as isize - 2;
        }
        if shoulder_idx < 0 {
            shoulder_idx = head_idx as isize;
        }
        let head_twist = self.shoulder_twist_angle;
        let resolve = |s: &Self, i: usize| s.skeleton.bone_index(&s.twist_bones[i]);
        let head_bone = resolve(self, head_idx);
        let adjust = self.calculate_desired_twist(head_bone, head_twist);
        let weight_delta = self.shoulder_twist_weight / (count - 1) as f32;
        let step = pzmath::slerp(Quaternion::IDENTITY, adjust, weight_delta);
        for i in 0..head_idx {
            let b = resolve(self, i);
            self.apply_twist_bone(b, step);
        }
        if self.is_aiming {
            let b = resolve(self, shoulder_idx as usize);
            self.apply_twist_bone(b, step);
        } else {
            self.apply_twist_bone(head_bone, step);
        }
    }

    fn apply_twist_bone(&mut self, bone: Option<usize>, twist_rot: Quaternion) {
        let Some(bone) = bone else { return };
        let parent = self.skeleton.parent(bone).expect("twist bone has no parent");
        let pt = self.bone_model_transform(parent);
        let Some(pt_inv) = pt.invert() else { return };
        let bt = self.bone_model_transform(bone);
        let pos = helpers::get_position(&bt);
        let mut nt = bt;
        helpers::set_position(&mut nt, Vector3f::ZERO);
        let mut q = twist_rot;
        let adj = helpers::create_from_quaternion(&mut q);
        nt = Matrix4f::mul(&nt, &adj);
        helpers::set_position(&mut nt, pos);
        let b = &mut self.bone_transforms[bone];
        b.twist = pzmath::wrap_range(helpers::get_rotation_y_matrix(&nt) - PI_F, -PI_F, PI_F);
        b.transform.mul_into(&nt, &pt_inv);
    }

    fn calculate_desired_twist(&mut self, bone: Option<usize>, twist_angle: f32) -> Quaternion {
        let Some(bone) = bone else { return Quaternion::IDENTITY };
        let parent = self.skeleton.parent(bone).expect("twist bone has no parent");
        let pt = self.bone_model_transform(parent);
        if pt.invert().is_none() {
            return Quaternion::IDENTITY;
        }
        let nt = self.bone_model_transform(bone);
        let mut turn = helpers::set_from_axis_angle(0.0, 1.0, 0.0, twist_angle);
        let desired = helpers::transform(&mut turn, Vector3f::new(0.0, 0.0, 1.0));
        let mut current = helpers::transform_vector(&nt, Vector3f::new(0.0, 0.0, -1.0));
        current.y = 0.0;
        current.normalise();
        let mut axis = Vector3f::cross(desired, current);
        if pzmath::equal(axis.length_squared(), 0.0, 1.0e-7) {
            return Quaternion::IDENTITY;
        }
        axis.normalise();
        let dot = Vector3f::dot(desired, current);
        let angle = (pzmath::clamp(dot, -1.0, 1.0) as f64).acos() as f32;
        helpers::set_from_axis_angle(axis.x, axis.y, axis.z, -angle)
    }

    // ---- deferred movement ----

    fn calculate_deferred_movement(&mut self) {
        self.deferred_angle_delta = 0.0;
        self.deferred_rotation_weight = 0.0;
        let mut accum = self.deferred_movement_accum;
        let mut remaining = 1.0f32;
        let mut idx = self.live_entries.entries.len() as isize - 1;
        while idx >= 0 && !(remaining <= 0.001) {
            let e = self.live_entries.entries[idx as usize];
            idx -= 1;
            let t = &self.tracks[e.track];
            if t.is_finished() {
                continue;
            }
            let bw = t.deferred_bone_weight();
            if bw <= 0.001 {
                continue;
            }
            let raw = e.weight * bw;
            if raw <= 0.001 {
                continue;
            }
            let w = pzmath::clamp(raw, 0.0, remaining);
            remaining -= raw;
            remaining = remaining.max(0.0);
            if t.is_ragdoll() {
                continue;
            }
            if t.use_deferred_movement() {
                accum = Vector2::add_scaled(accum, t.deferred_movement_diff(), w);
            }
            if !t.use_deferred_rotation() {
                continue;
            }
            self.deferred_angle_delta += t.deferred_rotation_diff() * w;
            self.deferred_rotation_weight += w;
        }
        // applyRotationToDeferredMovement
        let angle = self.rendered_angle();
        let len = accum.normalize();
        accum.rotate(angle);
        accum.set_length(-len);
        accum.x *= self.motion_scale;
        accum.y *= self.motion_scale;
        self.deferred_angle_delta *= self.rotation_scale;
        self.target_grapple_pos.x += accum.x;
        self.target_grapple_pos.y += accum.y;
        self.deferred_movement_accum = accum;
        self.deferred_movement = accum;
    }

    /// `getDeferredMovement(result, reset)`.
    pub fn deferred_movement(&mut self, reset: bool) -> Vector2 {
        let r = self.deferred_movement;
        if reset {
            self.deferred_movement_accum = Vector2::default();
        }
        r
    }

    pub fn reset_deferred_movement_accum(&mut self) {
        self.deferred_movement_accum = Vector2::default();
    }

    pub fn deferred_angle_delta(&self) -> f32 {
        self.deferred_angle_delta
    }

    pub fn deferred_rotation_weight(&self) -> f32 {
        self.deferred_rotation_weight
    }

    pub fn deferred_movement_from_ragdoll(&self) -> Vector3f {
        self.deferred_movement_from_ragdoll
    }

    pub fn target_grapple_pos(&self) -> Vector3f {
        self.target_grapple_pos
    }

    pub fn set_target_grapple_pos(&mut self, p: Vector3f) {
        self.target_grapple_pos = p;
    }

    pub fn target_grapple_rotation(&self) -> Vector2 {
        self.target_grapple_rotation
    }

    pub fn set_target_grapple_rotation(&mut self, x: f32, y: f32) {
        self.target_grapple_rotation = Vector2::new(x, y);
    }

    pub fn grapple_offset(&self) -> Vector3f {
        self.grapple_offset
    }

    pub fn set_grapple_offset(&mut self, p: Vector3f) {
        self.grapple_offset = p;
    }

    // ---- results ----

    pub fn rendered_angle(&self) -> f32 {
        self.angle + PI_F / 2.0
    }

    pub fn angle(&self) -> f32 {
        self.angle
    }

    pub fn set_angle(&mut self, a: f32) {
        self.angle = a;
    }

    pub fn set_angle_to_target(&mut self) {
        self.angle = self.target_angle;
    }

    pub fn set_target_to_angle(&mut self) {
        self.target_angle = self.angle;
    }

    pub fn target_angle(&self) -> f32 {
        self.target_angle
    }

    pub fn set_target_angle(&mut self, a: f32) {
        self.target_angle = a;
    }

    pub fn max_twist_angle(&self) -> f32 {
        self.max_twist_angle
    }

    pub fn set_max_twist_angle(&mut self, r: f32) {
        self.max_twist_angle = r;
    }

    pub fn excess_twist_angle(&self) -> f32 {
        self.excess_twist
    }

    pub fn twist_angle(&self) -> f32 {
        self.twist_angle
    }

    pub fn shoulder_twist_angle(&self) -> f32 {
        self.shoulder_twist_angle
    }

    pub fn target_twist_angle(&self) -> f32 {
        self.target_twist_angle
    }

    /// `getModelTransformAt` in the original layout.
    pub fn model_transform(&self, bone: usize) -> Matrix4f {
        self.model_transforms[bone]
    }

    pub fn model_transforms(&self) -> &[Matrix4f] {
        &self.model_transforms
    }

    /// `GetPropBoneMatrix` in pz-render's column-major layout.
    pub fn prop_bone_matrix(&self, bone: usize) -> Mat4 {
        self.model_transforms[bone].to_mat4()
    }

    /// `getSkinTransforms(skinnedTo)` in the original layout: for each bone
    /// of `skinned_to` (or of this player's skeleton) `boneOffset ×
    /// modelTransform` (LWJGL product order).
    pub fn skin_transforms(&self, skinned_to: Option<&Skeleton>) -> Vec<Matrix4f> {
        let target = skinned_to.unwrap_or(&self.skeleton);
        let mut out = vec![Matrix4f::IDENTITY; target.num_bones()];
        for (bone, model) in self.model_transforms.iter().enumerate() {
            let Some(name) = self.skeleton.bones[bone].name.as_deref() else { continue };
            let Some(&to) = target.bone_indices.get(name) else { continue };
            let to = to as usize;
            if to >= out.len() {
                continue;
            }
            out[to] = match target.bone_offset.get(to) {
                Some(off) => Matrix4f::mul(off, model),
                None => Matrix4f::IDENTITY,
            };
        }
        out
    }

    /// The skin palette for `pz_render::model::ModelDraw::palette`
    /// (column-major, `global × invBind`).
    pub fn palette(&self) -> Vec<Mat4> {
        self.skin_transforms(None).iter().map(Matrix4f::to_mat4).collect()
    }

    /// The palette for another mesh skinned to this skeleton's bone names.
    pub fn palette_for(&self, skinned_to: &Skeleton) -> Vec<Mat4> {
        self.skin_transforms(Some(skinned_to)).iter().map(Matrix4f::to_mat4).collect()
    }
}
