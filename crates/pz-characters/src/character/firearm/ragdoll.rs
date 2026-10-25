//! The character side of ragdolls. The ragdoll itself (`RagdollBuilder`,
//! `RagdollController`) is pz-ragdoll's and lives beside the character's
//! animation player; this module is the character side it needs:
//! `canRagdoll` ([`GameCharacter::can_ragdoll_uncapped`]), the tracked
//! ragdolling state, and the shot it reacts to with the impulse
//! `PhysicsHitReactionScript` gives for the weapon
//! ([`GameCharacter::take_ragdoll_hit`], `simulateHitReaction`'s first
//! half). TODO(#102): the `pz_ragdoll::RagdollCharacter` impl over these
//! (`CharacterRagdoll`) lands with pz-ragdoll.
//!
//! The ragdoll starts the way the original's does: the animation state
//! machine picks a ragdoll node (death and hit-reaction nodes test
//! `canRagdoll`, `usePhysicHitReaction` and the hit reaction, which
//! [`GameCharacter::update_ragdoll_variables`] and `processHit` set), and
//! the animation player then asks for a controller.

use pz_scripts::ScriptManager;

use super::physics::RagdollBodyPart;
use crate::character::{GameCharacter, SimContext};

/// What the character tracks of its ragdoll (written by the ragdoll
/// driver; read by the animation variables and the combat code).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RagdollTracking {
    /// `getRagdollController() != null`.
    pub has_controller: bool,
    /// `isRagdoll()` (`AnimationPlayer.isRagdolling()`).
    pub is_ragdolling: bool,
    /// `isUpright()` / `isOnBack()` of the controller.
    pub upright: bool,
    pub on_back: bool,
    /// `canRagdoll()` as last computed (the `canRagdoll` variable).
    pub can_ragdoll: bool,
}

/// The impulse of a shot (`PhysicsHitReactionScript.getImpulse` /
/// `getUpwardImpulse` for the weapon's ammo type or, for an explosive, its
/// physics object). `None` for other weapons.
pub fn hit_impulses(sm: &ScriptManager, part: RagdollBodyPart, data: &super::CombatDamageData) -> Option<(f32, f32)> {
    use pz_scripts::objects::items as it;
    let p = part.ordinal() as usize;
    if data.weapon_ammo_type.is_some() {
        let a = data.weapon_ammo_type.as_ref();
        return Some((it::impulse_for_ammo(sm, p, a), it::upward_impulse_for_ammo(sm, p, a)));
    }
    if data.weapon_explosive {
        let o = data.weapon_physics_object.as_deref();
        return Some((it::impulse_for_object(sm, p, o), it::upward_impulse_for_object(sm, p, o)));
    }
    None
}

/// A shot for the ragdoll to react to.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HitImpulse {
    pub body_part: i32,
    pub impulse: f32,
    pub upward_impulse: f32,
    pub target: [f32; 3],
    pub attacker: [f32; 3],
}

impl<I: 'static> GameCharacter<I> {
    /// `canRagdoll()` without the cap on active simulations (pz-ragdoll's
    /// frame applies that one).
    pub fn can_ragdoll_uncapped(&self, ctx: &dyn SimContext) -> bool {
        if ctx.is_client() || ctx.is_server() || ctx.debug_disable_ragdolls() || !ctx.option_use_physics_hit_reaction() {
            return false;
        }
        if !self.worn_clothing_can_ragdoll {
            return false;
        }
        self.current_state().is_some_and(|s| s.can_ragdoll(self))
    }

    /// The ragdoll animation variables (`canRagdoll`), for the frame's
    /// animation update.
    pub fn update_ragdoll_variables(&mut self, ctx: &dyn SimContext) {
        self.ragdoll.can_ragdoll = self.can_ragdoll(ctx);
    }

    /// `simulateHitReaction`'s character part: the ballistics target's
    /// unprocessed hit is marked processed and the hit reaction cleared
    /// (not for a hit without a body part, which stays pending); the
    /// impulse comes back for a firearm or an explosive. `attacker` is the
    /// attacker's position now.
    pub fn take_ragdoll_hit(&mut self, sm: &ScriptManager, attacker: Option<[f32; 3]>) -> Option<HitImpulse> {
        let t = self.ballistics_target.as_mut()?;
        if t.combat_damage_data_processed {
            return None;
        }
        let data = t.combat_damage_data.clone();
        if data.body_part == RagdollBodyPart::Count {
            return None;
        }
        t.combat_damage_data_processed = true;
        self.vars.hit_reaction = String::new();
        let (impulse, upward_impulse) = hit_impulses(sm, data.body_part, &data)?;
        Some(HitImpulse {
            body_part: data.body_part.ordinal(),
            impulse,
            upward_impulse,
            target: [self.vars.x, self.vars.y, self.vars.z],
            attacker: attacker.unwrap_or(data.attacker_position),
        })
    }
}
