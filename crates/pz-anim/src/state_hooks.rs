//! What the character states read from, and do to, a character's
//! animation, over [`AdvancedAnimator`] and [`AnimationPlayer`]:
//!
//! * the thump track (`ThumpState.execute`, [`crate::thump`]);
//! * `ZombieTurnAlerted.enter`: `getAnimationPlayer().setTargetAngle(TARGET_ANGLE)`;
//! * the root motion that becomes the next position
//!   (`IsoGameCharacter.doDeferredMovement`, whose result goes to
//!   `moveUnmodded(dx, dy)`: `nextX += dx`, `nextY += dy`).
//!
//! [`StateAnimHooks`] puts them on every [`AnimatedCharacter`], so the
//! world's character wrapper (which owns the animator and player) gets
//! them by implementing `anim_parts`.

use crate::animatable::AnimatedCharacter;
use crate::animator::AdvancedAnimator;
use crate::math::Vector2;
use crate::player::AnimationPlayer;
use crate::thump;

/// What `doDeferredMovement` reads about the character besides its
/// animation player (single player and server; the `GameClient` branches
/// for remote characters and network hit reactions are not modelled).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DeferredMovementInput {
    /// `GameServer.server`.
    pub server: bool,
    /// `isAnimationUpdatingThisFrame()`.
    pub animation_updating_this_frame: bool,
    /// `getPath2() != null` while not in `ClimbOverFenceState` or
    /// `ClimbThroughWindowState`: the movement is consumed and dropped
    /// (the caller also clears `path2` when in `WalkTowardState`).
    pub path2_blocks: bool,
    /// In `WalkTowardState`: `(pathTargetX - x, pathTargetY - y)`; the step
    /// is shortened to reach the target, not pass it.
    pub walk_toward: Option<(f32, f32)>,
    /// `isGrappling() || isBeingGrappled()`: the grappleable's
    /// `getGrappleOffset` x and y, added to the step.
    pub grapple_offset: Option<(f32, f32)>,
    /// `isDeferredMovementEnabled()`.
    pub deferred_movement_enabled: bool,
}

impl Default for DeferredMovementInput {
    fn default() -> Self {
        DeferredMovementInput {
            server: false,
            animation_updating_this_frame: true,
            path2_blocks: false,
            walk_toward: None,
            grapple_offset: None,
            deferred_movement_enabled: true,
        }
    }
}

/// `IsoGameCharacter.doDeferredMovement()` on the animation player: takes
/// (and resets) the accumulated root motion and returns the step for
/// `moveUnmodded`, or `None` when nothing moves. The accumulator is left
/// alone when the animation did not update this frame (on a client or in
/// single player).
pub fn do_deferred_movement(
    player: &mut AnimationPlayer,
    input: &DeferredMovementInput,
) -> Option<(f32, f32)> {
    if !input.server && !input.animation_updating_this_frame {
        return None;
    }
    let mut d = player.deferred_movement(true);
    if input.path2_blocks {
        return None;
    }
    if let Some((tx, ty)) = input.walk_toward {
        let target = Vector2 { x: tx, y: ty };
        if target.length_squared() < d.length_squared() {
            d.set_length(target.length());
        }
    }
    if let Some((gx, gy)) = input.grapple_offset {
        d.x += gx;
        d.y += gy;
    }
    input.deferred_movement_enabled.then_some((d.x, d.y))
}

/// `ZombieTurnAlerted.enter`: `getAnimationPlayer().setTargetAngle(TARGET_ANGLE)`.
pub fn turn_alerted_enter(player: &mut AnimationPlayer, target_angle: f32) {
    player.set_target_angle(target_angle);
}

/// The state hooks on a character's animation.
pub trait StateAnimHooks: AnimatedCharacter {
    /// `ThumpState.execute`'s thump count for this frame
    /// ([`thump::thumps_this_frame`]).
    fn thumps_this_frame(
        &mut self,
        animation_updating_this_frame: bool,
        animation_time_delta: f32,
    ) -> i32 {
        let (player, animator, _) = self.anim_parts();
        thump::thumps_this_frame(
            animator,
            player,
            animation_updating_this_frame,
            animation_time_delta,
        )
    }

    /// `ThumpAnimations.countThumpEventsBetween(chr, start, end)`.
    fn count_thump_events_between(&mut self, start_time: f32, end_time: f32) -> i32 {
        let (player, animator, _) = self.anim_parts();
        thump::count_thump_events_between(animator, player, start_time, end_time)
    }

    /// `ThumpAnimations.getDuration(chr)`.
    fn thump_duration(&mut self) -> f32 {
        let (player, animator, _) = self.anim_parts();
        thump::duration(animator, player)
    }

    /// `ZombieTurnAlerted.enter` with `TARGET_ANGLE`.
    fn turn_alerted_enter(&mut self, target_angle: f32) {
        turn_alerted_enter(self.anim_parts().0, target_angle);
    }

    /// `doDeferredMovement`'s step for `moveUnmodded`.
    fn deferred_movement_step(&mut self, input: &DeferredMovementInput) -> Option<(f32, f32)> {
        do_deferred_movement(self.anim_parts().0, input)
    }
}

impl<T: AnimatedCharacter + ?Sized> StateAnimHooks for T {}

/// The same hooks on borrowed parts, for a world that keeps animators and
/// players apart from its characters.
pub struct CharacterAnim<'a> {
    pub animator: &'a AdvancedAnimator,
    pub player: &'a mut AnimationPlayer,
}

impl CharacterAnim<'_> {
    pub fn thumps_this_frame(
        &self,
        animation_updating_this_frame: bool,
        animation_time_delta: f32,
    ) -> i32 {
        thump::thumps_this_frame(
            self.animator,
            self.player,
            animation_updating_this_frame,
            animation_time_delta,
        )
    }

    pub fn turn_alerted_enter(&mut self, target_angle: f32) {
        turn_alerted_enter(self.player, target_angle);
    }

    pub fn deferred_movement_step(&mut self, input: &DeferredMovementInput) -> Option<(f32, f32)> {
        do_deferred_movement(self.player, input)
    }
}
