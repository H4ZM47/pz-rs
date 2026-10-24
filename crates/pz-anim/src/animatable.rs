//! The character-facing surface: what the character code implements
//! (`IAnimatable`, `IAnimEventCallback` / `IAnimEventListener`) and the
//! helpers it uses (`AnimEventBroadcaster`, `AnimationVariableHandles`,
//! the well-known event names).
//!
//! A character (IsoGameCharacter / IsoZombie in the original) owns an
//! [`AnimationPlayer`], an [`AdvancedAnimator`] and its variable state, and
//! implements [`AnimationVariableSource`] over that state. Each frame it
//! calls [`AnimatedCharacter::animate`] (animator update, player update,
//! listener dispatch) and then routes the fired events
//! ([`AdvancedAnimator::events`]) to its [`AnimEventListener`]s, e.g. via an
//! [`AnimEventBroadcaster`].

use std::collections::HashMap;

use crate::animator::{AdvancedAnimator, AnimEventRecord};
use crate::player::AnimationPlayer;
use crate::vars::{AnimationVariableHandle, AnimationVariableSource};

/// `AnimationVariableHandles`: the handles the animation code itself reads.
pub struct AnimationVariableHandles;

impl AnimationVariableHandles {
    pub fn animal_running() -> AnimationVariableHandle {
        AnimationVariableHandle::alloc("animalRunning").unwrap()
    }

    /// Read by `AnimLayer.updateNodeActiveFlags`: while true no node
    /// selection happens.
    pub fn anim_locked() -> AnimationVariableHandle {
        AnimationVariableHandle::alloc("AnimLocked").unwrap()
    }

    /// Written by `AnimLayer.updateMaximumTwist` when the slot exists.
    pub fn max_twist() -> AnimationVariableHandle {
        AnimationVariableHandle::alloc("maxTwist").unwrap()
    }
}

/// Event names fired by the animation system itself (`LocalAnimEvent`,
/// `GlobalAnimEvent`) and the AnimSet / character-code names the original
/// registers listeners for. AnimSet events are data-driven: any
/// `m_EventName` can arrive.
pub mod event_names {
    pub const ACTIVE_ANIM_LOOPED: &str = "ActiveAnimLooped";
    pub const NON_LOOPED_ANIM_FADE_OUT: &str = "NonLoopedAnimFadeOut";
    pub const ACTIVE_ANIM_FINISHING: &str = "ActiveAnimFinishing";
    pub const ACTIVE_ANIM_FINISHED: &str = "ActiveAnimFinished";
    pub const NO_ANIM_CONDITIONS_PASS: &str = "NoAnimConditionsPass";
    pub const TURN_180_STARTED: &str = "Turn180Started";
    pub const TURN_180_TARGET_CHANGED: &str = "Turn180TargetChanged";
    pub const SET_VARIABLE: &str = "SetVariable";
    pub const CLEAR_VARIABLE: &str = "ClearVariable";
    pub const FLAG_WHILE_ALIVE: &str = "FlagWhileAlive";
    pub const SET_STATE: &str = "SetState";
    pub const FOOTSTEP: &str = "Footstep";
    pub const PLAY_SOUND: &str = "PlaySound";
    pub const ATTACK_ANIM: &str = "AttackAnim";
    pub const ATTACK_COLLISION_CHECK: &str = "AttackCollisionCheck";
    pub const ATTACK_CONNECT: &str = "AttackConnect";
    pub const BLOCK_MOVEMENT: &str = "BlockMovement";
    pub const BLOCK_TURN: &str = "BlockTurn";
    pub const SET_ON_FLOOR: &str = "SetOnFloor";
    pub const ON_FLOOR: &str = "OnFloor";
    pub const FALL_ON_FRONT: &str = "FallOnFront";
    pub const SET_COLLIDABLE: &str = "SetCollidable";
    pub const SET_MELEE_DELAY: &str = "SetMeleeDelay";
    pub const SHOVE_ANIM: &str = "ShoveAnim";
    pub const STOMP_ANIM: &str = "StompAnim";
    pub const THUMP_FRAME: &str = "ThumpFrame";
    pub const KNOCK_DOWN: &str = "KnockDown";
    pub const SET_KNOCKED_DOWN: &str = "SetKnockedDown";
    pub const IS_ALMOST_UP: &str = "IsAlmostUp";
    pub const TURN_AROUND: &str = "TurnAround";
    pub const TURN_AROUND_FLIP_SKELETON: &str = "TurnAround_FlipSkeleton";
    pub const PLAYER_VOICE_SOUND: &str = "PlayerVoiceSound";
    pub const PLAY_SWING_SOUND: &str = "PlaySwingSound";
    pub const PLAY_DEATH_SOUND: &str = "PlayDeathSound";
    pub const EMOTE_FINISHING: &str = "EmoteFinishing";
    pub const EMOTE_LOOPED: &str = "EmoteLooped";
    pub const WEAPON_EMPTY_CHECK: &str = "WeaponEmptyCheck";
    pub const VAULT_OVER_STARTED: &str = "VaultOverStarted";
    pub const SIT_GROUND_STARTED: &str = "SitGroundStarted";
    pub const GRAPPLE_GRAB_ANIM: &str = "GrappleGrabAnim";
    pub const GRAPPLE_GRAB_COLLISION_CHECK: &str = "GrappleGrabCollisionCheck";
    pub const GRAPPLER_LET_GO: &str = "GrapplerLetGo";
    pub const SET_SHARED_GRAPPLE_TYPE: &str = "SetSharedGrappleType";
}

/// `IAnimEventListener` / `IAnimEventCallback.OnAnimEvent`: one fired event
/// with its sending layer (state name and depth) and track.
pub trait AnimEventListener<C: ?Sized> {
    fn anim_event(&mut self, owner: &mut C, event: &AnimEventRecord);
}

impl<C: ?Sized, F: FnMut(&mut C, &AnimEventRecord)> AnimEventListener<C> for F {
    fn anim_event(&mut self, owner: &mut C, event: &AnimEventRecord) {
        self(owner, event)
    }
}

/// `AnimEventBroadcaster`: listeners keyed by event name (case-insensitive),
/// called in registration order.
pub struct AnimEventBroadcaster<C: ?Sized> {
    listeners: HashMap<String, Vec<Box<dyn AnimEventListener<C>>>>,
}

impl<C: ?Sized> Default for AnimEventBroadcaster<C> {
    fn default() -> Self {
        AnimEventBroadcaster { listeners: HashMap::new() }
    }
}

impl<C: ?Sized> AnimEventBroadcaster<C> {
    pub fn new() -> Self {
        Self::default()
    }

    /// `addListener(animEventName, listener)`.
    pub fn add_listener(&mut self, event_name: &str, listener: impl AnimEventListener<C> + 'static) {
        self.listeners.entry(event_name.to_lowercase()).or_default().push(Box::new(listener));
    }

    /// `animEvent(owner, layer, track, event)`.
    pub fn anim_event(&mut self, owner: &mut C, event: &AnimEventRecord) {
        if let Some(list) = self.listeners.get_mut(&event.event.event_name.to_lowercase()) {
            for l in list.iter_mut() {
                l.anim_event(owner, event);
            }
        }
    }
}

/// `IAnimatable`: what the animator needs from a character, plus the
/// state-machine calls the character code makes. Implementors provide the
/// three accessors; the rest are provided.
pub trait AnimatedCharacter {
    /// `getAnimationPlayer`, `getAdvancedAnimator` and the character's
    /// variables (`IAnimationVariableSource`), borrowed together.
    fn anim_parts(&mut self) -> (&mut AnimationPlayer, &mut AdvancedAnimator, &mut dyn AnimationVariableSource);

    /// `GetAnimSetName`.
    fn anim_set_name(&self) -> &str;

    /// `IAnimEventCallback.OnAnimEvent`, called by [`Self::animate`] for each
    /// fired event (after the frame's animation work).
    fn on_anim_event(&mut self, _event: &AnimEventRecord) {}

    /// One frame: `AdvancedAnimator.update`, `AnimationPlayer.Update`, the
    /// track listeners, then [`Self::on_anim_event`] for each fired event.
    fn animate(&mut self, dt: f32) {
        let events = {
            let (player, animator, vars) = self.anim_parts();
            animator.animate(player, vars, dt);
            std::mem::take(&mut animator.events)
        };
        for e in &events {
            self.on_anim_event(e);
        }
    }

    /// `AdvancedAnimator.setState(stateName, subStateNames)`.
    fn set_anim_state(&mut self, state: &str, sub_states: &[&str]) {
        let (player, animator, vars) = self.anim_parts();
        animator.set_state(player, vars, state, sub_states);
    }

    /// `AdvancedAnimator.getCurrentStateName`.
    fn current_anim_state_name(&mut self) -> Option<String> {
        let (_, animator, _) = self.anim_parts();
        animator.current_state_name().map(str::to_string)
    }

    /// `AdvancedAnimator.containsState`.
    fn contains_anim_state(&mut self, state: &str) -> bool {
        let (_, animator, _) = self.anim_parts();
        animator.contains_state(state)
    }

    /// `IsoGameCharacter.isAnimationFinished` analogue: no live track of the
    /// root layer is still playing a non-looped clip short of its end.
    fn is_animation_finished(&mut self) -> bool {
        let (player, animator, _) = self.anim_parts();
        animator.root_layer_tracks().iter().all(|&t| player.track(t).is_none_or(|t| t.is_looping() || t.is_finished()))
    }

    /// `getAnimationPlayer().getSkinTransforms(...)` as a renderer palette.
    fn skin_palette(&mut self) -> Vec<pz_model::Mat4> {
        let (player, _, _) = self.anim_parts();
        player.palette()
    }
}
