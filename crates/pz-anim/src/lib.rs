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
//! Ragdolls: the player runs the original's `updateRagdoll` /
//! `postUpdateRagdoll` and blends the ragdoll clip like any other track;
//! the `RagdollController` itself (Bullet physics) lives outside this crate
//! and is reached through [`RagdollHost`] (`pz-ragdoll` implements it).
//! Without a host the player behaves as the original does for a character
//! that cannot ragdoll. Grappling is not ported: [`Grappleable`] is the
//! seam, and the animator treats every character as not grappling.
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

/// What `AnimationPlayer`'s ragdoll code reaches outside the player: its
/// `IsoGameCharacter` and the `RagdollController` the player holds (which
/// lives with the host here, since it drives the physics world).
pub trait RagdollHost {
    /// `getIsoGameCharacter() != null`.
    fn has_character(&self) -> bool;
    /// `character.canRagdoll()`.
    fn can_ragdoll(&self) -> bool;
    /// `ragdollController != null`.
    fn has_controller(&self) -> bool;
    /// `initRagdollController` once its checks passed:
    /// `RagdollController.alloc()`, `setGameCharacterObject` and
    /// `character.onRagdollSimulationStarted()`.
    fn alloc_controller(&mut self);
    /// `Pool.tryRelease(ragdollController)` (`onReleased` → `reset`).
    fn release_controller(&mut self);
    /// `ragdollController.isInitialized()`.
    fn is_initialized(&self) -> bool;
    /// `ragdollController.isFirstFrame()`.
    fn is_first_frame(&self) -> bool;
    /// `ragdollController.isSimulationActive()`.
    fn is_simulation_active(&self) -> bool;
    /// `ragdollController.isSimulationDirectionCalculated()`.
    fn is_simulation_direction_calculated(&self) -> bool;
    /// `getDesiredCharacterPosition`.
    fn desired_character_position(&self) -> math::Vector3f;
    /// `getCalculatedSimulationDirectionAngle`.
    fn calculated_simulation_direction_angle(&self) -> f32;
    /// `ragdollController.update(deltaT, ...)`.
    fn update(&mut self, player: &mut AnimationPlayer, dt: f32);
    /// `ragdollController.postUpdate(deltaT)`.
    fn post_update(&mut self, player: &mut AnimationPlayer, dt: f32);
    /// The character's `getX/getY/getZ`.
    fn character_position(&self) -> math::Vector3f;
    /// `character.doDeferredMovementFromRagdoll(dMovement)`.
    fn do_deferred_movement_from_ragdoll(&mut self, player: &AnimationPlayer, movement: math::Vector3f);
    /// `GameTime.getInstance().getMultiplier()`.
    fn game_time_multiplier(&self) -> f32;
}

/// The seam for `IGrappleable`. Not ported: the animator treats every
/// character as neither grappling nor grappled.
pub trait Grappleable {
    fn is_grappling(&self) -> bool;
    fn is_being_grappled(&self) -> bool;
}
