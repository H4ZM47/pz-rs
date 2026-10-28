//! The player's vision effects (`IsoGameCharacter.updateVisionEffects`,
//! `updateVisionEffectTargets`, `getBlurFactor`, `IsoPlayer`'s night-vision
//! goggles flag) and what `RenderSettings.PlayerRenderSettings` copies from
//! the player for the screen shader ([`PlayerRenderInputs`]).

use pz_scripts::{ScriptManager, ScriptResult};

use crate::character::items::script_item;
use crate::character::zombie::math::lerp;
use crate::character::GameCharacter;
use crate::Characters;

impl<I: 'static> GameCharacter<I> {
    /// `IsoPlayer.isWearingNightVisionGoggles()`.
    pub fn is_wearing_night_vision_goggles(&self) -> bool {
        self.player.wearing_night_vision_goggles
    }

    /// `IsoPlayer.setWearingNightVisionGoggles(b)`.
    pub fn set_wearing_night_vision_goggles(&mut self, b: bool) {
        self.player.wearing_night_vision_goggles = b;
    }

    /// `getBlurFactor()`.
    pub fn blur_factor(&self) -> f32 {
        self.player.blur_factor
    }

    /// `isWearingVisualAid()`: a worn item whose script item is a visual
    /// aid (the `EYES` item, checked again after the loop, is one of the
    /// worn items).
    pub fn is_wearing_visual_aid(&self, sm: &ScriptManager) -> bool {
        self.items.worn().any(|i| script_item(sm, &i.borrow()).is_some_and(|s| s.borrow().visual_aid))
    }

    /// `updateVisionEffects()` (from `OnClothingUpdated`, players only):
    /// the blur target is 1 when the glasses and `SHORT_SIGHTED` disagree
    /// (short-sighted without glasses, or glasses without the trait), else 0.
    pub fn update_vision_effects(&mut self, sm: &ScriptManager) -> ScriptResult<()> {
        let short_sighted = self.has_trait(sm, "SHORT_SIGHTED")?;
        let glasses = self.is_wearing_glasses(sm);
        self.player.blur_factor_target = if (!glasses && short_sighted) || (glasses && !short_sighted) { 1.0 } else { 0.0 };
        Ok(())
    }

    /// `updateVisionEffectTargets()` (from `IsoPlayer.updateInternal1`,
    /// non-animals): the blur factor lerps toward its target, by 0.1 when
    /// rising and 0.01 when falling.
    pub fn update_vision_effect_targets(&mut self) {
        let p = &mut self.player;
        let r = if p.blur_factor < p.blur_factor_target { 0.1 } else { 0.01 };
        p.blur_factor = lerp(p.blur_factor, p.blur_factor_target, r);
    }
}

/// The player inputs of one `RenderSettings.PlayerRenderSettings` that the
/// `WeatherShader` reads (`isApplyNightVisionGoggles`, `getDrunkFactor`,
/// `getBlurFactor`). Keep one per player slot: like the original's fields,
/// the drunk and blur factors keep their last values while the slot has no
/// living player.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PlayerRenderInputs {
    /// `applyNightVisionGoggles`.
    pub night_vision_goggles: bool,
    /// `drunkFactor`.
    pub drunk_factor: f32,
    /// `blurFactor`.
    pub blur_factor: f32,
}

impl PlayerRenderInputs {
    /// The player part of `updateRenderSettings(plrIndex, player)`,
    /// `player` being `IsoPlayer.players[plrIndex]`:
    /// `applyNightVisionGoggles = player != null &&
    /// player.isWearingNightVisionGoggles()`, then, for a living player,
    /// `drunkFactor = stats.get(INTOXICATION) / 100` and
    /// `blurFactor = player.getBlurFactor()`.
    pub fn update<I: 'static>(&mut self, chars: &Characters, player: Option<&GameCharacter<I>>) -> ScriptResult<()> {
        self.night_vision_goggles = player.is_some_and(|p| p.is_wearing_night_vision_goggles());
        if let Some(p) = player.filter(|p| !p.is_dead()) {
            self.drunk_factor = p.stat(chars, "INTOXICATION")? / 100.0;
            self.blur_factor = p.blur_factor();
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::character::tests::{fixture, player};
    use pz_scripts::objects::items::CharacterTrait;

    #[test]
    fn short_sighted_without_glasses_blurs() {
        let (sm, mut chars) = fixture();
        let mut c = player(&sm, &mut chars);
        c.update_vision_effects(&sm).unwrap();
        assert_eq!(c.player.blur_factor_target, 0.0);
        c.character_traits.add(CharacterTrait::constant(&sm, "SHORT_SIGHTED").as_ref()).unwrap();
        c.update_vision_effects(&sm).unwrap();
        assert_eq!(c.player.blur_factor_target, 1.0);
        // Rises by a tenth of the gap per update, falls by a hundredth.
        c.update_vision_effect_targets();
        assert_eq!(c.blur_factor(), 0.1);
        c.update_vision_effect_targets();
        assert_eq!(c.blur_factor(), 0.1 + (1.0 - 0.1) * 0.1);
        c.player.blur_factor_target = 0.0;
        let b = c.blur_factor();
        c.update_vision_effect_targets();
        assert_eq!(c.blur_factor(), b + (0.0 - b) * 0.01);
    }

    #[test]
    fn render_inputs_follow_the_living_player() {
        let (sm, mut chars) = fixture();
        let mut c = player(&sm, &mut chars);
        let intox = chars.sim.character_stats.field("INTOXICATION").unwrap();
        c.stats.set(&intox, 0.5);
        c.player.blur_factor = 0.25;
        c.set_wearing_night_vision_goggles(true);
        let mut r = PlayerRenderInputs::default();
        r.update(&chars, Some(&c)).unwrap();
        assert_eq!(r, PlayerRenderInputs { night_vision_goggles: true, drunk_factor: 0.5 / 100.0, blur_factor: 0.25 });
        // No player: the goggles flag clears, the factors stay.
        r.update::<()>(&chars, None).unwrap();
        assert_eq!(r, PlayerRenderInputs { night_vision_goggles: false, drunk_factor: 0.005, blur_factor: 0.25 });
        // A dead player: the goggles flag follows, the factors stay.
        c.player.blur_factor = 0.75;
        c.health = 0.0;
        r.update(&chars, Some(&c)).unwrap();
        assert_eq!(r, PlayerRenderInputs { night_vision_goggles: true, drunk_factor: 0.005, blur_factor: 0.25 });
    }
}
