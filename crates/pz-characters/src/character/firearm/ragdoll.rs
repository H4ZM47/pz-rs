//! The character side of ragdolls. The ragdoll itself (`RagdollBuilder`,
//! `RagdollController`) is pz-ragdoll's and lives beside the character's
//! animation player; this module gives it the character
//! (`pz_ragdoll::RagdollCharacter`, behind the `bullet` feature):
//! `canRagdoll`, the fall flags and position it writes, and the shot it
//! reacts to, with the impulse `PhysicsHitReactionScript` gives for the
//! weapon (`RagdollController.simulateHitReaction`'s first half).
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

/// A character as pz-ragdoll's `RagdollCharacter`.
#[cfg(feature = "bullet")]
pub struct CharacterRagdoll<'a, I: 'static> {
    pub chr: &'a mut GameCharacter<I>,
    pub sm: &'a ScriptManager,
    pub ctx: &'a dyn SimContext,
    /// The attacker's position by character id (for the shot impulse).
    pub attacker_position: &'a dyn Fn(i32) -> Option<[f32; 3]>,
}

#[cfg(feature = "bullet")]
impl<I: 'static> pz_ragdoll::RagdollCharacter for CharacterRagdoll<'_, I> {
    fn id(&self) -> i32 {
        self.chr.id
    }
    fn position(&self) -> pz_ragdoll::Vec3 {
        pz_ragdoll::Vec3::new(self.chr.vars.x, self.chr.vars.y, self.chr.vars.z)
    }
    fn forward_direction_angle(&self) -> f32 {
        self.chr.direction_angle_radians()
    }
    fn can_ragdoll(&self) -> bool {
        self.chr.can_ragdoll_uncapped(self.ctx)
    }
    fn set_ragdoll_fall(&mut self, v: bool) {
        self.chr.vars.ragdoll_fall = v;
    }
    fn set_use_physic_hit_reaction(&mut self, v: bool) {
        self.chr.vars.use_physic_hit_reaction = v;
    }
    fn is_fall_on_front(&self) -> bool {
        self.chr.environment.fall_on_front
    }
    fn set_fall_on_front(&mut self, v: bool) {
        self.chr.environment.fall_on_front = v;
    }
    fn set_on_floor(&mut self, v: bool) {
        self.chr.vars.on_floor = v;
    }
    /// `doDeferredMovementFromRagdoll(dMovement)`.
    fn do_deferred_movement_from_ragdoll(&mut self, player: &pz_anim::AnimationPlayer, m: pz_ragdoll::Vec3) {
        if !player.is_ragdolling() {
            return;
        }
        let c = &mut *self.chr;
        // `moveUnmoddedInternal`, then the position follows.
        c.vars.next_x += m.x;
        c.vars.next_y += m.y;
        c.vars.req_movement = (m.x, m.y);
        c.vars.x = c.vars.next_x;
        c.vars.y = c.vars.next_y;
        c.vars.z += m.z;
    }
    fn take_ragdoll_hit(&mut self) -> Option<pz_ragdoll::RagdollHit> {
        let attacker = self.chr.ballistics_target.as_ref().and_then(|t| (self.attacker_position)(t.combat_damage_data.attacker));
        let h = self.chr.take_ragdoll_hit(self.sm, attacker)?;
        let v = |p: [f32; 3]| pz_ragdoll::Vec3::new(p[0], p[1], p[2]);
        Some(pz_ragdoll::RagdollHit {
            body_part: h.body_part,
            impulse: h.impulse,
            upward_impulse: h.upward_impulse,
            target: v(h.target),
            attacker: v(h.attacker),
        })
    }
}

#[cfg(feature = "bullet")]
impl<I: 'static> CharacterRagdoll<'_, I> {
    /// After the animation update: what the character tracks of the
    /// controller (`isRagdoll`, `isRagdollSimulationActive`, `isUpright`,
    /// `isOnBack`).
    pub fn track(chr: &mut GameCharacter<I>, player: &pz_anim::AnimationPlayer, controller: Option<&pz_ragdoll::RagdollController>) {
        chr.ragdoll.has_controller = controller.is_some();
        chr.ragdoll.is_ragdolling = player.is_ragdolling();
        chr.ragdoll.upright = controller.is_some_and(|c| c.is_upright());
        chr.ragdoll.on_back = controller.is_some_and(|c| c.is_on_back());
        chr.vars.ragdoll_simulation_active = controller.is_some_and(|c| c.is_simulation_active()) && chr.ragdoll.can_ragdoll;
    }
}
