//! Project Zomboid's skeletal animation system
//! (`zombie.core.skinnedmodel.animation` and `.advancedanimation`).
//!
//! * [`AnimationPlayer`] samples and blends [`AnimationTrack`]s (by layer,
//!   priority and weight, with per-bone masks), applies twist bones, bone
//!   reparenting, the counter-rotation bone and deferred root motion, and
//!   produces the skin palette ([`AnimationPlayer::palette`]) in
//!   pz-render's column-major convention for
//!   `pz_render::model::ModelDraw::palette`.
//! * [`AnimationSet`] / [`AnimState`] / [`AnimNode`] load the
//!   `media/AnimSets` XML (with the original's `x_extends` / `x_include`
//!   merging).
//! * [`AdvancedAnimator`] runs the state machine: per layer it picks the
//!   nodes whose conditions pass, starts their tracks, blends nodes in and
//!   out, plays transition animations, weights 2D blend fields and fires
//!   [`AnimEvent`]s.
//! * [`vars`] is the animation-variable system the conditions read, with a
//!   map-backed source ([`MapVariableSource`]) for tests.
//!
//! Ragdoll physics and grappling are not ported: [`RagdollController`] and
//! [`Grappleable`] are the seams, and the player/animator behave as the
//! original does for a character that cannot ragdoll and is not grappling.
//!
//! Matrices: the internal math keeps the original's LWJGL `Matrix4f`
//! fields so float operations match ([`math`]); everything handed out to
//! the renderer is a column-major [`pz_model::Mat4`].

// The negated comparisons mirror the original's NaN behaviour.
#![allow(clippy::neg_cmp_op_on_partial_ord)]

pub mod animatable;
pub mod animator;
pub mod animset;
pub mod bone_transform;
pub mod clip;
pub mod interp;
pub mod math;
pub mod player;
pub mod rand;
pub mod shared_skele;
pub mod skeleton;
pub mod track;
pub mod vars;
pub mod xml;

pub use animatable::{event_names, AnimEventBroadcaster, AnimEventListener, AnimatedCharacter, AnimationVariableHandles};
pub use animator::{AdvancedAnimator, AnimEventRecord};
pub use rand::{AnimRandom, JavaRandom};
pub use animset::{
    Anim2DBlend, Anim2DBlendPicker, Anim2DBlendTriangle, AnimCondition, AnimConditionType, AnimEvent, AnimEventKind, AnimEventTime,
    AnimNode, AnimState, AnimTransition, AnimationSet, GrappleOffsetBehaviour,
};
pub use clip::{AnimClip, Keyframe};
pub use player::{AnimDebugOptions, AnimationPlayer, LayerBinding, StartAnimTrackParameters};
pub use track::{AnimationTrack, TrackEvent, TrackEventKind, TrackId, TrackListener};
pub use vars::{AnimationVariableHandle, AnimationVariableReference, AnimationVariableSource, MapVariableSource, VariableValue};

/// `BoneAxis`: which axis a deferred (root-motion) bone moves along.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum BoneAxis {
    #[default]
    Y,
    Z,
}

/// `AnimBoneWeight`: one entry of a sub-layer bone mask.
#[derive(Clone, Debug, PartialEq)]
pub struct AnimBoneWeight {
    pub bone_name: String,
    pub weight: f32,
    pub include_descendants: bool,
}

impl AnimBoneWeight {
    pub fn new(bone_name: &str, weight: f32) -> Self {
        AnimBoneWeight { bone_name: bone_name.to_string(), weight, include_descendants: true }
    }
}

/// The seam for `RagdollController` (Bullet physics in the original). Not
/// ported: the player never creates one, so ragdoll tracks only play their
/// placeholder clip.
pub trait RagdollController {
    fn is_initialized(&self) -> bool;
    fn is_simulation_active(&self) -> bool;
}

/// The seam for `IGrappleable`. Not ported: the animator treats every
/// character as neither grappling nor grappled.
pub trait Grappleable {
    fn is_grappling(&self) -> bool;
    fn is_being_grappled(&self) -> bool;
}
