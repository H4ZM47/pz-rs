//! `IsoPlayer.initializeStates()`: the action-state names the player's
//! AI states are registered under (on foot and in a vehicle).

use crate::character::state::StateRegistry;
use crate::character::GameCharacter;

/// On foot (`getVehicle() == null`).
pub const ON_FOOT: &[(&str, &str)] = &[
    ("actions", "zombie.ai.states.PlayerActionsState"),
    ("aim", "zombie.ai.states.PlayerAimState"),
    ("aim-sneak", "zombie.ai.states.PlayerAimState"),
    ("aim-strafe", "zombie.ai.states.PlayerStrafeState"),
    ("bumped", "zombie.ai.states.BumpedState"),
    ("bumped-bump", "zombie.ai.states.BumpedState"),
    ("climbdownrope", "zombie.ai.states.ClimbDownSheetRopeState"),
    ("climbfence", "zombie.ai.states.ClimbOverFenceState"),
    ("climbrope", "zombie.ai.states.ClimbSheetRopeState"),
    ("climbwall", "zombie.ai.states.ClimbOverWallState"),
    ("climbwindow", "zombie.ai.states.ClimbThroughWindowState"),
    ("closewindow", "zombie.ai.states.CloseWindowState"),
    ("collide", "zombie.ai.states.CollideWithWallState"),
    ("emote", "zombie.ai.states.PlayerEmoteState"),
    ("ext", "zombie.ai.states.PlayerExtState"),
    ("falldown", "zombie.ai.states.PlayerFallDownState"),
    ("falling", "zombie.ai.states.PlayerFallingState"),
    ("fishing", "zombie.ai.states.FishingState"),
    ("fitness", "zombie.ai.states.FitnessState"),
    ("getup", "zombie.ai.states.PlayerGetUpState"),
    ("hitreaction", "zombie.ai.states.PlayerHitReactionState"),
    ("hitreaction-bite", "zombie.ai.states.PlayerHitReactionState"),
    ("hitreaction-biteGeneric", "zombie.ai.states.PlayerHitReactionState"),
    ("hitreaction-crawlerBite", "zombie.ai.states.PlayerHitReactionState"),
    ("hitreaction-endDeath", "zombie.ai.states.PlayerHitReactionState"),
    ("hitreaction-shot", "zombie.ai.states.PlayerHitReactionState"),
    ("hitreaction-hit", "zombie.ai.states.PlayerHitReactionPVPState"),
    ("hitreactionpvp", "zombie.ai.states.PlayerHitReactionPVPState"),
    ("idle", "zombie.ai.states.IdleState"),
    ("knockeddown", "zombie.ai.states.PlayerKnockedDown"),
    ("melee", "zombie.ai.states.SwipeStatePlayer"),
    ("milkanimal", "zombie.ai.states.player.PlayerMilkAnimalState"),
    ("movement", "zombie.ai.states.player.PlayerMovementState"),
    ("onbed", "zombie.ai.states.PlayerOnBedState"),
    ("onground", "zombie.ai.states.PlayerOnGroundState"),
    ("openwindow", "zombie.ai.states.OpenWindowState"),
    ("petanimal", "zombie.ai.states.player.PlayerPetAnimalState"),
    ("ranged", "zombie.ai.states.SwipeStatePlayer"),
    ("run", "zombie.ai.states.player.PlayerMovementState"),
    ("shearanimal", "zombie.ai.states.player.PlayerShearAnimalState"),
    ("shove", "zombie.ai.states.SwipeStatePlayer"),
    ("shoveWithFirearm", "zombie.ai.states.SwipeStatePlayer"),
    ("shoveWithHandgun", "zombie.ai.states.SwipeStatePlayer"),
    ("shoveAim", "zombie.ai.states.SwipeStatePlayer"),
    ("stomp", "zombie.ai.states.SwipeStatePlayer"),
    ("sitext", "zombie.ai.states.PlayerExtState"),
    ("sitonfurniture", "zombie.ai.states.PlayerSitOnFurnitureState"),
    ("sitonground", "zombie.ai.states.PlayerSitOnGroundState"),
    ("sitonground-sitting", "zombie.ai.states.PlayerSitOnGroundState"),
    ("sitonground-sitting-warmhands", "zombie.ai.states.PlayerSitOnGroundState"),
    ("smashwindow", "zombie.ai.states.SmashWindowState"),
    ("sprint", "zombie.ai.states.player.PlayerMovementState"),
    ("strafe", "zombie.ai.states.PlayerStrafeState"),
    ("grappleGrab", "zombie.ai.states.SwipeStatePlayer"),
    ("pickUpBody", "zombie.ai.states.SwipeStatePlayer"),
    ("grabCorpseFromContainer", "zombie.ai.states.PlayerDraggingCorpse"),
    ("grabCorpseFromContainer-HeadEnd-onBack", "zombie.ai.states.PlayerDraggingCorpse"),
    ("pickUpBody-HeadEnd-onBack", "zombie.ai.states.PlayerDraggingCorpse"),
    ("pickUpBody-HeadEnd-onFront", "zombie.ai.states.PlayerDraggingCorpse"),
    ("pickUpBody-LegsEnd-onBack", "zombie.ai.states.PlayerDraggingCorpse"),
    ("pickUpBody-LegsEnd-onFront", "zombie.ai.states.PlayerDraggingCorpse"),
    ("draggingBody-HeadEnd-onBack", "zombie.ai.states.PlayerDraggingCorpse"),
    ("draggingBody-HeadEnd-onFront", "zombie.ai.states.PlayerDraggingCorpse"),
    ("draggingBody-LegsEnd-onBack", "zombie.ai.states.PlayerDraggingCorpse"),
    ("draggingBody-LegsEnd-onFront", "zombie.ai.states.PlayerDraggingCorpse"),
    ("layDownBody-HeadEnd-onBack", "zombie.ai.states.PlayerDraggingCorpse"),
    ("layDownBody-HeadEnd-onFront", "zombie.ai.states.PlayerDraggingCorpse"),
    ("layDownBody-LegsEnd-onBack", "zombie.ai.states.PlayerDraggingCorpse"),
    ("layDownBody-LegsEnd-onFront", "zombie.ai.states.PlayerDraggingCorpse"),
    ("throwBodyOutWindow-HeadEnd-onBack", "zombie.ai.states.PlayerDraggingCorpse"),
    ("throwBodyOutWindow-HeadEnd-onFront", "zombie.ai.states.PlayerDraggingCorpse"),
    ("throwBodyOutWindow-LegsEnd-onBack", "zombie.ai.states.PlayerDraggingCorpse"),
    ("throwBodyOutWindow-LegsEnd-onFront", "zombie.ai.states.PlayerDraggingCorpse"),
    ("throwBodyOverFence-HeadEnd-onBack", "zombie.ai.states.PlayerDraggingCorpse"),
    ("throwBodyOverFence-HeadEnd-onFront", "zombie.ai.states.PlayerDraggingCorpse"),
    ("throwBodyOverFence-LegsEnd-onBack", "zombie.ai.states.PlayerDraggingCorpse"),
    ("throwBodyOverFence-LegsEnd-onFront", "zombie.ai.states.PlayerDraggingCorpse"),
    ("throwBodyIntoContainer-HeadEnd-onBack", "zombie.ai.states.PlayerDraggingCorpse"),
    ("throwBodyIntoContainer-HeadEnd-onFront", "zombie.ai.states.PlayerDraggingCorpse"),
    ("throwBodyIntoContainer-LegsEnd-onBack", "zombie.ai.states.PlayerDraggingCorpse"),
];

/// In a vehicle.
pub const IN_VEHICLE: &[(&str, &str)] = &[
    ("throwBodyIntoContainer-LegsEnd-onFront", "zombie.ai.states.PlayerDraggingCorpse"),
    ("aim", "zombie.ai.states.PlayerAimState"),
    ("idle", "zombie.ai.states.IdleState"),
    ("melee", "zombie.ai.states.SwipeStatePlayer"),
    ("shove", "zombie.ai.states.SwipeStatePlayer"),
    ("shoveWithFirearm", "zombie.ai.states.SwipeStatePlayer"),
    ("shoveWithHandgun", "zombie.ai.states.SwipeStatePlayer"),
    ("shoveAim", "zombie.ai.states.SwipeStatePlayer"),
    ("stomp", "zombie.ai.states.SwipeStatePlayer"),
];

impl<I: 'static> GameCharacter<I> {
    /// `IsoPlayer.initializeStates()`: clears the map and registers the
    /// player's states (from `registry`, stand-ins for unported ones).
    pub fn initialize_player_states(&mut self, registry: &mut StateRegistry<I>, in_vehicle: bool) {
        self.clear_ai_state_map();
        for (name, class) in if in_vehicle { IN_VEHICLE } else { ON_FOOT } {
            let s = registry.get_or_opaque(class);
            self.register_ai_state(name, s);
        }
    }
}
