//! `AnimationTrack`: one playing clip with its time, blend weights, bone
//! mask and deferred (root) motion.
//!
//! The original calls its `IAnimListener`s synchronously while ticking.
//! Here a tick appends [`TrackEvent`]s (with the listeners registered at
//! that moment) to a queue the owner dispatches afterwards.

use std::collections::HashMap;
use std::rc::Rc;

use crate::bone_transform::BoneTransform;
use crate::clip::{AnimClip, Keyframe, MAX_BONES};
use crate::interp::Interpolator;
use crate::math::{helpers, pzmath, Quaternion, Vector2, Vector3f, PI_F};
use crate::skeleton::Skeleton;
use crate::vars::AnimationVariableSource;
use crate::{AnimBoneWeight, BoneAxis};

/// A track handle, unique within its [`crate::AnimationPlayer`].
pub type TrackId = u32;

/// Who listens to a track. The ids belong to whoever registered them (the
/// animator uses them for its layers and live nodes).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TrackListener {
    Layer(u32),
    Node(u32),
}

/// `IAnimListener` callbacks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrackEventKind {
    AnimStarted,
    LoopedAnim,
    NonLoopedAnimFadeOut,
    NonLoopedAnimFinished,
}

/// One queued listener callback.
#[derive(Clone, Debug, PartialEq)]
pub struct TrackEvent {
    pub track: TrackId,
    pub kind: TrackEventKind,
    pub listeners: Vec<TrackListener>,
}

/// `AnimationTrack.KeyframeSpan`: indices into the clip's frames for one
/// bone.
#[derive(Clone, Copy, Debug, Default)]
struct KeyframeSpan {
    bone: usize,
    from: Option<usize>,
    to: Option<usize>,
}

impl KeyframeSpan {
    fn clear(&mut self) {
        self.from = None;
        self.to = None;
    }

    fn is_span(&self) -> bool {
        self.from.is_some() && self.to.is_some()
    }

    fn is_bone(&self, bone: usize) -> bool {
        (self.from.is_some() || self.to.is_some()) && self.bone == bone
    }

    fn contains_time(&self, frames: &[Keyframe], time: f32) -> bool {
        match (self.from, self.to) {
            (Some(f), Some(t)) => frames[f].time <= time && time <= frames[t].time,
            _ => false,
        }
    }

    /// `lerp(time, pos, rot, scale)`.
    fn lerp(&self, frames: &[Keyframe], time: f32, interpolate: bool) -> (Vector3f, Quaternion, Vector3f) {
        let get = |i: usize| (frames[i].position, frames[i].rotation, frames[i].scale);
        match (self.from, self.to) {
            (None, None) => (Vector3f::ZERO, Quaternion::IDENTITY, Vector3f::ONE),
            (Some(f), None) => get(f),
            (None, Some(t)) => get(t),
            (Some(f), Some(t)) => {
                if f == t || !interpolate {
                    get(t)
                } else {
                    Keyframe::lerp(&frames[f], &frames[t], time)
                }
            }
        }
    }
}

/// `AnimationTrack.DeferredMotionData`.
#[derive(Clone, Copy, Debug, Default)]
struct DeferredMotionData {
    target_deferred_rotation: f32,
    prev_deferred_rotation: f32,
    target_deferred_rotation_q: Quaternion,
    target_deferred_position: Vector3f,
    prev_deferred_movement: Vector2,
    target_deferred_movement: Vector2,
    deferred_rotation_diff: f32,
    deferred_movement_diff: Vector2,
    counter_rotated_movement_diff: Vector2,
}

/// `AnimationTrack`.
#[derive(Clone, Debug)]
pub struct AnimationTrack {
    pub id: TrackId,
    pub is_playing: bool,
    pub is_primary: bool,
    pub current_clip: Option<Rc<AnimClip>>,
    pub priority: i32,
    is_ragdoll_first_frame: bool,
    pub ragdoll_start_time: f32,
    pub ragdoll_max_time: f32,
    current_time_value: f32,
    previous_time_value: f32,
    accumulated_time_value: f32,
    pub sync_tracking_enabled: bool,
    pub track_time_to_variable: Option<String>,
    pub reverse: bool,
    pub looping: bool,
    pose: Vec<KeyframeSpan>,
    deferred_pose_span: KeyframeSpan,
    pose_adjustments: HashMap<usize, BoneTransform>,
    speed_delta: f32,
    blend_weight: f32,
    blend_field_weight: f32,
    pub blend_curve: Option<Rc<Interpolator>>,
    name: String,
    matching_grappled_anim_node: String,
    is_grappler: bool,
    pub early_blend_out_time: f32,
    pub trigger_on_non_looped_anim_fade_out_event: bool,
    /// `animLayer`: the layer id of the animator that owns this track.
    pub anim_layer: Option<u32>,
    /// `getLayerIdx()`: the depth of `anim_layer`, kept in sync by the
    /// animator.
    pub layer_depth: i32,
    bone_weight_bindings: Option<Vec<AnimBoneWeight>>,
    bone_weights: Option<Vec<f32>>,
    pub(crate) listeners: Vec<TrackListener>,
    deferred_bone: Option<usize>,
    deferred_bone_axis: BoneAxis,
    use_deferred_movement: bool,
    use_deferred_rotation: bool,
    deferred_rotation_scale: f32,
    deferred_motion: DeferredMotionData,
    pub is_initial_adjustment_calculated: bool,
    pub initial_adjustment: Vector3f,
    /// `PerformanceSettings.interpolateAnims`.
    pub interpolate: bool,
    /// `DebugOptions...noBoneMasks`.
    pub no_bone_masks: bool,
}

impl AnimationTrack {
    pub fn new(id: TrackId) -> AnimationTrack {
        AnimationTrack {
            id,
            is_playing: false,
            is_primary: false,
            current_clip: None,
            priority: 0,
            is_ragdoll_first_frame: false,
            ragdoll_start_time: 0.0,
            ragdoll_max_time: 5.0,
            current_time_value: 0.0,
            previous_time_value: 0.0,
            accumulated_time_value: 0.0,
            sync_tracking_enabled: true,
            track_time_to_variable: None,
            reverse: false,
            looping: false,
            pose: vec![KeyframeSpan::default(); MAX_BONES],
            deferred_pose_span: KeyframeSpan::default(),
            pose_adjustments: HashMap::new(),
            speed_delta: 1.0,
            blend_weight: 0.0,
            blend_field_weight: 1.0,
            blend_curve: None,
            name: "!Empty!".into(),
            matching_grappled_anim_node: String::new(),
            is_grappler: false,
            early_blend_out_time: 0.0,
            trigger_on_non_looped_anim_fade_out_event: false,
            anim_layer: None,
            layer_depth: 0,
            bone_weight_bindings: None,
            bone_weights: None,
            listeners: Vec::new(),
            deferred_bone: None,
            deferred_bone_axis: BoneAxis::Y,
            use_deferred_movement: true,
            use_deferred_rotation: false,
            deferred_rotation_scale: 1.0,
            deferred_motion: DeferredMotionData::default(),
            is_initial_adjustment_calculated: false,
            initial_adjustment: Vector3f::ZERO,
            interpolate: true,
            no_bone_masks: false,
        }
    }

    /// `reset()`.
    pub fn reset(&mut self) {
        let (id, interpolate, no_bone_masks) = (self.id, self.interpolate, self.no_bone_masks);
        *self = AnimationTrack::new(id);
        self.interpolate = interpolate;
        self.no_bone_masks = no_bone_masks;
    }

    /// `startClip`.
    pub fn start_clip(&mut self, clip: Rc<AnimClip>, looped: bool, ragdoll_max_time: f32, vars: Option<&dyn AnimationVariableSource>) {
        self.reset();
        if self.track_time_to_variable.as_deref().is_some_and(|s| !s.is_empty()) {
            let t = self.track_time_to_variable_value(self.current_time_value, vars);
            self.current_time_value = t;
            self.previous_time_value = t;
        }
        self.is_playing = true;
        self.looping = looped;
        self.current_clip = Some(clip);
        self.is_ragdoll_first_frame = self.is_ragdoll();
        self.ragdoll_max_time = ragdoll_max_time;
    }

    /// `createClone`: the playback settings without listeners or sampled
    /// state.
    pub fn create_clone(&self, id: TrackId) -> AnimationTrack {
        let mut t = AnimationTrack::new(id);
        t.interpolate = self.interpolate;
        t.no_bone_masks = self.no_bone_masks;
        t.is_playing = self.is_playing;
        t.current_clip = self.current_clip.clone();
        t.priority = self.priority;
        t.is_ragdoll_first_frame = self.is_ragdoll_first_frame;
        t.current_time_value = self.current_time_value;
        t.previous_time_value = self.previous_time_value;
        t.accumulated_time_value = self.accumulated_time_value;
        t.sync_tracking_enabled = self.sync_tracking_enabled;
        t.reverse = self.reverse;
        t.looping = self.looping;
        t.speed_delta = self.speed_delta;
        t.blend_weight = self.blend_weight;
        t.blend_field_weight = self.blend_field_weight;
        t.name = self.name.clone();
        t.early_blend_out_time = self.early_blend_out_time;
        t.trigger_on_non_looped_anim_fade_out_event = self.trigger_on_non_looped_anim_fade_out_event;
        t.anim_layer = self.anim_layer;
        t.layer_depth = self.layer_depth;
        t.bone_weight_bindings = self.bone_weight_bindings.clone();
        t.bone_weights = self.bone_weights.clone();
        t.deferred_bone = self.deferred_bone;
        t.deferred_bone_axis = self.deferred_bone_axis;
        t.use_deferred_movement = self.use_deferred_movement;
        t.use_deferred_rotation = self.use_deferred_rotation;
        t.deferred_rotation_scale = self.deferred_rotation_scale;
        t.matching_grappled_anim_node = self.matching_grappled_anim_node.clone();
        t.is_grappler = self.is_grappler;
        t
    }

    pub fn add_listener(&mut self, l: TrackListener) {
        self.listeners.push(l);
    }

    pub fn remove_listener(&mut self, l: TrackListener) {
        if let Some(i) = self.listeners.iter().position(|&x| x == l) {
            self.listeners.remove(i);
        }
    }

    pub fn listeners(&self) -> &[TrackListener] {
        &self.listeners
    }

    /// `setAnimLayer`: swaps the layer listener.
    pub fn set_anim_layer(&mut self, layer: Option<u32>, depth: i32) {
        self.layer_depth = depth;
        if self.anim_layer == layer {
            return;
        }
        if let Some(old) = self.anim_layer {
            self.remove_listener(TrackListener::Layer(old));
        }
        self.anim_layer = layer;
        if let Some(new) = layer {
            self.add_listener(TrackListener::Layer(new));
        }
    }

    pub fn has_clip(&self) -> bool {
        self.current_clip.is_some()
    }

    pub fn is_ragdoll(&self) -> bool {
        self.current_clip.as_ref().is_some_and(|c| c.is_ragdoll)
    }

    /// Ragdoll simulation is not ported; a ragdoll clip is never simulating.
    pub fn is_ragdoll_simulation_active(&self) -> bool {
        false
    }

    pub fn is_ragdoll_first_frame(&self) -> bool {
        self.is_ragdoll_first_frame
    }

    pub fn name(&self) -> &str {
        self.current_clip.as_ref().map(|c| c.name.as_str()).unwrap_or("!Empty!")
    }

    pub fn set_name(&mut self, name: &str) {
        self.name = name.to_string();
    }

    pub fn get_duration(&self) -> f32 {
        if self.is_ragdoll() {
            return self.ragdoll_max_time;
        }
        self.current_clip.as_ref().map(|c| c.duration).unwrap_or(0.0)
    }

    fn reversible(&self, t: f32) -> f32 {
        if self.reverse {
            self.get_duration() - t
        } else {
            t
        }
    }

    pub fn current_track_time(&self) -> f32 {
        self.reversible(self.current_time_value)
    }

    pub fn previous_track_time(&self) -> f32 {
        self.reversible(self.previous_time_value)
    }

    pub fn current_animation_time(&self) -> f32 {
        if self.is_ragdoll() {
            return self.current_clip.as_ref().unwrap().duration;
        }
        self.current_track_time()
    }

    pub fn previous_animation_time(&self) -> f32 {
        if self.is_ragdoll() {
            return 0.0;
        }
        self.previous_track_time()
    }

    pub fn current_time_value(&self) -> f32 {
        self.current_time_value
    }

    pub fn set_current_time_value(&mut self, v: f32) {
        self.current_time_value = v;
    }

    pub fn previous_time_value(&self) -> f32 {
        self.previous_time_value
    }

    pub fn set_previous_time_value(&mut self, v: f32) {
        self.previous_time_value = v;
    }

    pub fn accumulated_time_value(&self) -> f32 {
        self.accumulated_time_value
    }

    pub fn speed_delta(&self) -> f32 {
        self.speed_delta
    }

    pub fn set_speed_delta(&mut self, v: f32) {
        self.speed_delta = v;
    }

    /// `getBlendWeight`: weight × blend curve × blend-field weight.
    pub fn blend_weight(&self) -> f32 {
        let curve_weight = match &self.blend_curve {
            Some(c) => c.lerp(self.current_track_time() / self.get_duration()),
            None => 1.0,
        };
        self.blend_weight * curve_weight * self.blend_field_weight
    }

    pub fn set_blend_weight(&mut self, w: f32) {
        self.blend_weight = w;
    }

    pub fn blend_field_weight(&self) -> f32 {
        self.blend_field_weight
    }

    pub fn set_blend_field_weight(&mut self, w: f32) {
        self.blend_field_weight = w;
    }

    pub fn is_looping(&self) -> bool {
        self.looping
    }

    /// `isFinished`.
    pub fn is_finished(&self) -> bool {
        if self.is_ragdoll() {
            return !self.is_ragdoll_simulation_active();
        }
        !self.looping && self.get_duration() > 0.0 && self.current_time_value >= self.get_duration()
    }

    pub fn current_time_fraction(&self) -> f32 {
        if self.has_clip() {
            self.current_time_value / self.get_duration()
        } else {
            0.0
        }
    }

    /// `advance`.
    pub fn advance(&mut self, amount: f32) {
        let d = self.get_duration();
        self.current_time_value = pzmath::wrap_range(self.current_time_value + amount, 0.0, d);
        self.previous_time_value = pzmath::wrap_range(self.previous_time_value + amount, 0.0, d);
        self.accumulated_time_value += amount;
    }

    pub fn rewind(&mut self, amount: f32) {
        self.advance(-amount);
    }

    pub fn scaled_advance(&mut self, amount: f32) {
        self.advance(amount * self.speed_delta);
    }

    pub fn scaled_rewind(&mut self, amount: f32) {
        self.scaled_advance(-amount);
    }

    pub fn advance_fraction(&mut self, f: f32) {
        self.advance(self.get_duration() * f);
    }

    pub fn move_current_time_value_to(&mut self, target: f32) {
        let diff = target - self.current_time_value;
        self.advance(diff);
    }

    pub fn move_current_time_value_to_fraction(&mut self, fraction: f32) {
        let target = self.get_duration() * fraction;
        self.move_current_time_value_to(target);
    }

    pub fn priority(&self) -> i32 {
        self.priority
    }

    pub fn matching_grappled_anim_node(&self) -> &str {
        &self.matching_grappled_anim_node
    }

    pub fn set_matching_grappled_anim_node(&mut self, name: &str) {
        self.matching_grappled_anim_node = name.to_string();
        self.is_grappler = !crate::vars::is_null_or_whitespace(name);
    }

    pub fn is_grappler(&self) -> bool {
        self.is_grappler
    }

    // ---- bone masks ----

    /// `setBoneWeights`.
    pub fn set_bone_weights(&mut self, weights: Option<&[AnimBoneWeight]>) {
        self.bone_weight_bindings = weights.map(|w| w.to_vec());
        self.bone_weights = None;
    }

    /// `initBoneWeights`.
    pub fn init_bone_weights(&mut self, skeleton: &Skeleton) {
        if self.has_bone_mask() {
            return;
        }
        let Some(bindings) = self.bone_weight_bindings.clone() else { return };
        if bindings.is_empty() {
            self.bone_weights = Some(Vec::new());
            return;
        }
        self.bone_weights = Some(vec![0.0; skeleton.num_bones()]);
        for b in &bindings {
            if crate::vars::is_null_or_whitespace(&b.bone_name) {
                continue;
            }
            let Some(bone) = skeleton.bone_index(&b.bone_name) else { continue };
            self.assign_bone_weight(b.weight, bone);
            if b.include_descendants {
                let mut desc = Vec::new();
                skeleton.for_each_descendant(bone, &mut |d| desc.push(d));
                for d in desc {
                    self.assign_bone_weight(b.weight, d);
                }
            }
        }
    }

    fn assign_bone_weight(&mut self, weight: f32, bone: usize) {
        let w = self.bone_weights.as_mut().expect("bone weights not initialised");
        if let Some(slot) = w.get_mut(bone) {
            *slot = weight.max(*slot);
        }
    }

    pub fn has_bone_mask(&self) -> bool {
        self.bone_weights.is_some()
    }

    /// `getBoneWeight`.
    pub fn bone_weight(&self, bone: usize) -> f32 {
        match &self.bone_weights {
            None => 1.0,
            Some(_) if self.no_bone_masks => 1.0,
            Some(w) => w.get(bone).copied().unwrap_or(0.0),
        }
    }

    pub fn deferred_bone_weight(&self) -> f32 {
        match self.deferred_bone {
            None => 0.0,
            Some(b) => self.bone_weight(b),
        }
    }

    // ---- deferred motion settings ----

    pub fn set_deferred_bone(&mut self, bone: Option<usize>, axis: BoneAxis) {
        self.deferred_bone = bone;
        self.deferred_bone_axis = axis;
    }

    pub fn deferred_movement_bone_idx(&self) -> i32 {
        self.deferred_bone.map(|b| b as i32).unwrap_or(-1)
    }

    pub fn set_use_deferred_movement(&mut self, v: bool) {
        self.use_deferred_movement = v;
    }

    pub fn use_deferred_movement(&self) -> bool {
        self.use_deferred_movement
    }

    pub fn set_use_deferred_rotation(&mut self, v: bool) {
        self.use_deferred_rotation = v;
    }

    pub fn use_deferred_rotation(&self) -> bool {
        self.use_deferred_rotation
    }

    pub fn set_deferred_rotation_scale(&mut self, v: f32) {
        self.deferred_rotation_scale = v;
    }

    pub fn deferred_rotation_scale(&self) -> f32 {
        self.deferred_rotation_scale
    }

    /// `getDeferredMovementDiff` (the counter-rotated diff, like the original).
    #[allow(clippy::misnamed_getters)]
    pub fn deferred_movement_diff(&self) -> Vector2 {
        self.deferred_motion.counter_rotated_movement_diff
    }

    pub fn deferred_rotation_diff(&self) -> f32 {
        self.deferred_motion.deferred_rotation_diff
    }

    pub fn current_deferred_rotation(&self) -> f32 {
        self.deferred_motion.target_deferred_rotation
    }

    pub fn current_deferred_position(&self) -> Vector3f {
        self.deferred_motion.target_deferred_position
    }

    /// `getCurrentDeferredCounterPosition`.
    pub fn current_deferred_counter_position(&self) -> Vector3f {
        let r = self.current_deferred_position();
        if self.deferred_bone_axis == BoneAxis::Y {
            Vector3f::new(-r.x, 0.0, r.z)
        } else {
            Vector3f::new(-r.x, -r.y, 0.0)
        }
    }

    // ---- pose ----

    /// `setBonePoseAdjustment`.
    pub fn set_bone_pose_adjustment(&mut self, bone: usize, pos: Vector3f, rot: Quaternion, scale: Vector3f) {
        let mut t = BoneTransform::default();
        t.set_prs(pos, rot, scale);
        self.pose_adjustments.insert(bone, t);
    }

    /// `get(bone, pos, rot, scale)`: the sampled local transform.
    pub fn get(&mut self, bone: usize) -> (Vector3f, Quaternion, Vector3f) {
        let time = self.current_animation_time();
        let (mut pos, mut rot, mut scale) = match (&self.current_clip, self.pose.get(bone)) {
            (Some(clip), Some(span)) => span.lerp(clip.bone_frames(bone), time, self.interpolate),
            _ => (Vector3f::ZERO, Quaternion::IDENTITY, Vector3f::ONE),
        };
        if let Some(adj) = self.pose_adjustments.get_mut(&bone) {
            let mut bt = BoneTransform::default();
            bt.set_prs(pos, rot, scale);
            let a = adj.matrix();
            let b = bt.matrix();
            bt.mul_into(&a, &b);
            (pos, rot, scale) = bt.prs();
        }
        (pos, rot, scale)
    }

    fn get_keyframe_span(clip: &AnimClip, bone: usize, time: f32, result: &mut KeyframeSpan) {
        if !result.is_bone(bone) {
            result.clear();
        }
        result.bone = bone;
        let frames = clip.bone_frames(bone);
        let n = frames.len();
        if n == 0 {
            result.clear();
            return;
        }
        if result.contains_time(frames, time) {
            return;
        }
        if time >= frames[n - 1].time {
            result.from = Some(if n > 1 { n - 2 } else { 0 });
            result.to = Some(n - 1);
            return;
        }
        if time <= frames[0].time {
            result.clear();
            result.to = Some(0);
            return;
        }
        let mut start = 0;
        if let (true, Some(t)) = (result.is_span(), result.to) {
            if frames[t].time <= time {
                start = t;
            }
        }
        result.clear();
        for idx in start..n - 1 {
            if frames[idx].time <= time && time <= frames[idx + 1].time {
                result.from = Some(idx);
                result.to = Some(idx + 1);
                break;
            }
        }
    }

    /// `Update(time)` / `UpdateKeyframes`.
    pub fn update(&mut self, dt: f32, vars: Option<&dyn AnimationVariableSource>, events: &mut Vec<TrackEvent>) {
        if self.current_clip.is_none() {
            return;
        }
        if dt > 0.0 {
            self.tick_current_time(dt, vars, events);
        }
        self.update_pose();
        self.update_deferred_values();
    }

    fn update_pose(&mut self) {
        let t = self.current_animation_time();
        let clip = self.current_clip.clone().unwrap();
        for n in 0..MAX_BONES {
            Self::get_keyframe_span(&clip, n, t, &mut self.pose[n]);
        }
    }

    fn push(&self, kind: TrackEventKind, events: &mut Vec<TrackEvent>) {
        if !self.listeners.is_empty() {
            events.push(TrackEvent { track: self.id, kind, listeners: self.listeners.clone() });
        }
    }

    fn track_time_to_variable_value(&self, default_time: f32, vars: Option<&dyn AnimationVariableSource>) -> f32 {
        if self.anim_layer.is_none() {
            return default_time;
        }
        let Some(vars) = vars else { return default_time };
        let len = self.get_duration();
        let frac = vars.get_variable_float(self.track_time_to_variable.as_deref().unwrap_or(""), default_time / len);
        len * frac
    }

    fn tick_current_time(&mut self, time: f32, vars: Option<&dyn AnimationVariableSource>, events: &mut Vec<TrackEvent>) {
        let mut time = time * self.speed_delta;
        if !self.is_playing {
            time = 0.0;
        }
        let end = self.get_duration();
        self.previous_time_value = self.current_time_value;
        if self.track_time_to_variable.as_deref().is_none_or(str::is_empty) {
            self.current_time_value += time;
            self.accumulated_time_value += time;
        } else {
            self.current_time_value = self.track_time_to_variable_value(self.current_time_value + time, vars);
        }
        if self.looping {
            if self.previous_time_value == 0.0 && self.current_time_value > 0.0 {
                self.push(TrackEventKind::AnimStarted, events);
            }
            if self.previous_time_value > self.current_time_value || self.current_time_value >= end {
                self.push(TrackEventKind::LoopedAnim, events);
                self.current_time_value %= end;
                self.push(TrackEventKind::AnimStarted, events);
            }
            return;
        }
        if self.current_time_value < 0.0 {
            self.current_time_value = 0.0;
        }
        if self.previous_time_value == 0.0 && self.current_time_value > 0.0 {
            self.push(TrackEventKind::AnimStarted, events);
        }
        if self.trigger_on_non_looped_anim_fade_out_event {
            let at = end - self.early_blend_out_time * self.speed_delta;
            if self.previous_time_value < at && at <= self.current_time_value {
                self.push(TrackEventKind::NonLoopedAnimFadeOut, events);
            }
        }
        if self.current_time_value > end {
            self.current_time_value = end;
        }
        if !self.is_ragdoll() && self.previous_time_value < end && self.current_time_value >= end {
            if self.looping {
                self.push(TrackEventKind::LoopedAnim, events);
            }
            self.push(TrackEventKind::NonLoopedAnimFinished, events);
        }
    }

    fn update_deferred_values(&mut self) {
        if self.deferred_bone.is_none() {
            return;
        }
        let mut dm = self.deferred_motion;
        dm.deferred_rotation_diff = 0.0;
        dm.deferred_movement_diff = Vector2::default();
        dm.counter_rotated_movement_diff = Vector2::default();
        let mut prev = self.previous_animation_time();
        let cur = self.current_animation_time();
        if self.is_looping() && prev > cur {
            let end = self.get_duration();
            self.append_deferred_values(&mut dm, prev, end);
            prev = 0.0;
        }
        self.append_deferred_values(&mut dm, prev, cur);
        self.deferred_motion = dm;
    }

    /// `getDeferredMovementFrameAt` (always interpolated).
    fn deferred_frame_at(&mut self, bone: usize, time: f32) -> Keyframe {
        let clip = self.current_clip.clone().unwrap();
        Self::get_keyframe_span(&clip, bone, time, &mut self.deferred_pose_span);
        let frames = clip.bone_frames(bone);
        let s = self.deferred_pose_span;
        let mut k = Keyframe::identity();
        let (p, r, sc) = s.lerp(frames, time, true);
        k.position = p;
        k.rotation = r;
        k.scale = sc;
        k
    }

    fn deferred_twist_rotation(&self, mut q: Quaternion) -> f32 {
        match self.deferred_bone_axis {
            BoneAxis::Z => helpers::get_rotation_z(&q),
            BoneAxis::Y => helpers::get_rotation_y(&mut q),
        }
    }

    fn deferred_movement(&self, p: Vector3f) -> Vector2 {
        if self.deferred_bone_axis == BoneAxis::Y {
            Vector2::new(p.x, -p.z)
        } else {
            Vector2::new(p.x, p.y)
        }
    }

    fn append_deferred_values(&mut self, dm: &mut DeferredMotionData, prev_time: f32, cur_time: f32) {
        let bone = self.deferred_bone.unwrap();
        let prev_k = self.deferred_frame_at(bone, prev_time);
        let k = self.deferred_frame_at(bone, cur_time);
        dm.prev_deferred_rotation = self.deferred_twist_rotation(prev_k.rotation);
        dm.target_deferred_rotation_q = k.rotation;
        dm.target_deferred_rotation = self.deferred_twist_rotation(k.rotation);
        let angle_diff = pzmath::get_closest_angle(dm.prev_deferred_rotation, dm.target_deferred_rotation);
        dm.deferred_rotation_diff += angle_diff * self.deferred_rotation_scale;
        dm.prev_deferred_movement = self.deferred_movement(prev_k.position);
        dm.target_deferred_position = k.position;
        dm.target_deferred_movement = self.deferred_movement(k.position);
        let diff = Vector2::new(
            dm.target_deferred_movement.x - dm.prev_deferred_movement.x,
            dm.target_deferred_movement.y - dm.prev_deferred_movement.y,
        );
        let mut cr = diff;
        if self.use_deferred_rotation && !self.is_ragdoll() {
            let len = cr.normalize();
            cr.rotate(-(dm.target_deferred_rotation + PI_F / 2.0));
            cr.scale(-len);
        }
        dm.deferred_movement_diff.x += diff.x;
        dm.deferred_movement_diff.y += diff.y;
        dm.counter_rotated_movement_diff.x += cr.x;
        dm.counter_rotated_movement_diff.y += cr.y;
    }
}
