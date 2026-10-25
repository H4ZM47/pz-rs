//! The light the character states read (`IsoZombie.spottedNew`,
//! `spottedOld`, `getDarknessPenalty`): a square's decoded lighting for a
//! player and that player's render-settings ambient.
//!
//! [`StateLighting`] answers `pz_characters`' `StateWorld::square_light`,
//! `square_light_level` and `ambient_for_player` with the same signatures,
//! so a world's `StateWorld` impl forwards to it. This crate does not
//! depend on pz-characters; the world crate that implements `StateWorld`
//! holds both.

use pz_lighting::jni::{light_level2, PlayerRenderSettings, SquareLight};

use crate::lighting::WorldLighting;
use crate::settings::RenderSettingsSource;

/// `RenderSettings`: the per-player settings as of the last
/// [`RenderSettings::update`]. `getAmbientForPlayer` reads these cached
/// values, not the climate's current ones.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct RenderSettings {
    /// `playerSettings[0..4]`: every field starts at 0 (a player never
    /// updated has ambient 0).
    pub players: [PlayerRenderSettings; 4],
}

impl RenderSettings {
    pub fn new() -> Self {
        Self::default()
    }

    /// `RenderSettings.update()`: nothing on a server; otherwise each slot
    /// with a player (`IsoPlayer.players[i] != null`) is recomputed
    /// (`updateRenderSettings(i, player)`), the others keep their values.
    pub fn update(
        &mut self,
        source: &mut dyn RenderSettingsSource,
        players_present: [bool; 4],
        server: bool,
    ) {
        if server {
            return;
        }
        for (i, present) in players_present.into_iter().enumerate() {
            if present {
                self.players[i] = source.player_settings(i as i32);
            }
        }
    }

    /// `getPlayerSettings(playerIndex)`; `None` where Java throws
    /// (an index outside 0..4).
    pub fn player_settings(&self, player_index: i32) -> Option<&PlayerRenderSettings> {
        usize::try_from(player_index)
            .ok()
            .and_then(|i| self.players.get(i))
    }

    /// `getAmbientForPlayer(playerIndex)`. The slots always exist in Java,
    /// so the `null` branch (0) is only reached here for an index Java
    /// would reject with an `ArrayIndexOutOfBoundsException`.
    pub fn ambient_for_player(&self, player_index: i32) -> f32 {
        self.player_settings(player_index)
            .map_or(0.0, |s| s.ambient)
    }
}

/// The lighting hooks of `StateWorld` over the world's decoded lighting
/// and render settings.
#[derive(Clone, Copy)]
pub struct StateLighting<'a> {
    pub lighting: &'a WorldLighting,
    pub settings: &'a RenderSettings,
    /// `LightingJNI.lightInts[1]` as the last `getSquareLighting` left it
    /// (what `getLightLevel(-1)` decodes); 0 until the world tracks it.
    pub shared_light_ints_1: i32,
}

impl<'a> StateLighting<'a> {
    pub fn new(lighting: &'a WorldLighting, settings: &'a RenderSettings) -> Self {
        StateLighting {
            lighting,
            settings,
            shared_light_ints_1: 0,
        }
    }

    /// `square.lighting[playerIndex]`: the decoded cache, or the
    /// constructor's (white) for a square no frame has decoded yet.
    fn cache(&self, sq: (i32, i32, i32), player_index: i32) -> Option<&SquareLight> {
        self.lighting.square_light(player_index, sq.0, sq.1, sq.2)
    }

    /// `StateWorld::square_light`: `(r + g + b) / 3` of
    /// `square.lighting[playerIndex].lightInfo()`.
    pub fn square_light(&self, sq: (i32, i32, i32), player_index: i32) -> f32 {
        self.cache(sq, player_index)
            .map_or(1.0, SquareLight::light_mean)
    }

    /// `StateWorld::square_light_level`: `square.getLightLevel(playerIndex)`
    /// (`getLightLevel2()` for -1).
    pub fn square_light_level(&self, sq: (i32, i32, i32), player_index: i32) -> f32 {
        if player_index == -1 {
            return light_level2(self.shared_light_ints_1);
        }
        self.cache(sq, player_index)
            .map_or(1.0, SquareLight::light_level_value)
    }

    /// `StateWorld::ambient_for_player`:
    /// `RenderSettings.getInstance().getAmbientForPlayer(playerIndex)`.
    pub fn ambient_for_player(&self, player_index: i32) -> f32 {
        self.settings.ambient_for_player(player_index)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::{ClimateInput, ClimateSettings, PlayerRenderInput};

    fn climate(ambient: f32) -> ClimateSettings {
        ClimateSettings {
            climate: ClimateInput {
                global_light_exterior: [1.0; 4],
                global_light_interior: [1.0; 4],
                ambient,
                day_light_strength: 0.0,
                night_strength: 1.0,
                desaturation: 0.0,
                view_distance: 30.0,
                fog_intensity: 0.0,
                moon_float: 0.0,
                game_night: 1.0,
                night_darkness: 3,
                weather_shader: true,
                last_stand: false,
            },
            players: [
                PlayerRenderInput::default(),
                PlayerRenderInput {
                    night_vision_trait: true,
                    ..Default::default()
                },
                PlayerRenderInput::default(),
                PlayerRenderInput::default(),
            ],
        }
    }

    #[test]
    fn ambient_is_the_last_update_per_player() {
        let mut rs = RenderSettings::new();
        // Never updated: the fields' default 0.
        assert_eq!(rs.ambient_for_player(0), 0.0);
        let mut src = climate(0.1);
        rs.update(&mut src, [true, true, false, false], false);
        // ambientMin 0.15 (night darkness 3, no moon): 0.15 + 0.85 * 0.1.
        assert_eq!(rs.ambient_for_player(0), 0.15 + (1.0 - 0.15) * 0.1);
        // Night vision lifts ambient below 0.2 to 0.2 first: 0.15 + 0.85 * 0.2.
        assert_eq!(rs.ambient_for_player(1), 0.15 + (1.0 - 0.15) * 0.2);
        // No player in slot 2: untouched.
        assert_eq!(rs.ambient_for_player(2), 0.0);
        // A server never updates; later climate changes are not seen until
        // the next update.
        let mut brighter = climate(0.9);
        rs.update(&mut brighter, [true; 4], true);
        assert_eq!(rs.ambient_for_player(0), 0.15 + (1.0 - 0.15) * 0.1);
        rs.update(&mut brighter, [true; 4], false);
        assert_eq!(rs.ambient_for_player(2), 0.15 + (1.0 - 0.15) * 0.9);
        // Outside 0..4 (Java throws).
        assert_eq!(rs.ambient_for_player(-1), 0.0);
        assert_eq!(rs.ambient_for_player(4), 0.0);
    }

    #[test]
    fn empty_lighting_reads_like_fresh_caches() {
        let lighting = WorldLighting::new();
        let rs = RenderSettings::new();
        let mut h = StateLighting::new(&lighting, &rs);
        assert_eq!(h.square_light((1, 2, 0), 0), 1.0);
        assert_eq!(h.square_light_level((1, 2, 0), 3), 1.0);
        assert_eq!(h.square_light_level((1, 2, 0), -1), 0.0);
        h.shared_light_ints_1 = 0x0000_8000;
        assert_eq!(h.square_light_level((1, 2, 0), -1), 128.0 / 255.0);
    }
}
