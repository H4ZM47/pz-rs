//! The player overloads of IsoGameCharacter.DoFootstepSound.
use super::super::{
    player::update::nimble_mod, zombie::senses::sneak_spot_mod, GameCharacter, SimContext, SimEvent,
};
use crate::Characters;
use pz_scripts::{ScriptManager, ScriptResult};

impl<I: 'static> GameCharacter<I> {
    pub fn player_footstep_gait(&mut self, gait: &str) -> f32 {
        let (speed, volume) = match gait {
            "sneak_walk" => (0, 0.25),
            "sneak_run" => (1, 0.25),
            "strafe" => (2, 0.5),
            "run" => (4, 0.75),
            "sprint" => (5, 1.0),
            _ => (3, 0.5),
        };
        self.player.sound_movement_speed = speed;
        volume
    }

    pub fn player_footstep(&self, volume: f32, in_room: bool, sm: &ScriptManager,
        chars: &Characters, ctx: &dyn SimContext) -> ScriptResult {
        let invisible = self.is_ghost_mode(ctx);
        if invisible && !pz_core::debug::debug_options().borrow().get_boolean("Character.Debug.PlaySoundWhenInvisible") {
            return Ok(());
        }
        if self.path.current_square.is_none() || volume <= 0.0 {
            return Ok(());
        }
        let mut loudness = volume * 1.4;
        if self.has_trait(sm, "GRACEFUL")? { loudness *= 0.6; }
        if self.has_trait(sm, "CLUMSY")? { loudness *= 1.2; }
        if self.clothing_item(sm, "SHOES")?.is_none() { loudness *= 0.5; }
        let perk = |name| self.perk_level(chars.perks.constant(name).as_ref());
        let lightfoot = perk("Lightfoot");
        let lightfoot_mod = [0.99, 0.9, 0.79, 0.71, 0.65, 0.59, 0.52, 0.45, 0.37, 0.3, 0.2];
        loudness *= lightfoot_mod.get(lightfoot as usize).copied().unwrap_or(0.99);
        loudness *= 2.0 - nimble_mod(perk("Nimble"));
        if self.is_sneaking() { loudness *= sneak_spot_mod(perk("Sneak")); }
        if loudness > 0.0 {
            ctx.event(SimEvent::PlayerFootstep);
            if invisible { return Ok(()); }
            let mut radius = (loudness * 10.0).ceil() as i32;
            let odds = if self.is_sneaking() {
                radius = radius.max(1);
                (4 + lightfoot).min(12)
            } else { 2 };
            if in_room { radius /= 2; }
            if ctx.rand_next(odds) == 0 && radius > 0 {
                ctx.event(SimEvent::FootstepNoise(self.world_sound_request(radius, radius, false)));
            }
        }
        Ok(())
    }
}
