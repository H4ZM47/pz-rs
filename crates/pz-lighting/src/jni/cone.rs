//! `LightingJNI.calculateVisionCone`, `getLumaInverted` and the two
//! character values `updatePlayer` sends.

use super::decode::ColorInfo;

/// What `calculateVisionCone` reads from the player and the climate.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct VisionConeInput {
    /// `ClimateManager.getDayLightStrength()`.
    pub day_light_strength: f32,
    /// `getVehicle() != null`.
    pub in_vehicle: bool,
    /// `getStats().get(FATIGUE)`.
    pub fatigue: f32,
    /// `getStats().isAtMaximum(FATIGUE)`.
    pub fatigue_at_max: bool,
    /// `getMoodles().getMoodleLevel(DRUNK)`.
    pub drunk_level: i32,
    /// `getStats().get(INTOXICATION)`.
    pub intoxication: f32,
    /// `getMoodles().getMoodleLevel(PANIC)`.
    pub panic_level: i32,
    /// `getLumaInverted(player)`, see [`luma_inverted`].
    pub luma_inverted: f32,
    /// `isInARoom()`.
    pub in_room: bool,
    /// `hasTrait(EAGLE_EYED)`.
    pub eagle_eyed: bool,
    /// `hasTrait(NIGHT_VISION)`.
    pub night_vision: bool,
    /// In a vehicle: `getHeadlightsOn() && getHeadlightCanEmmitLight()`.
    pub headlights: bool,
    /// `getWornItemsVisionMultiplier()`.
    pub worn_items_vision_multiplier: f32,
}

/// `PZMath.equal(a, b, delta)`.
fn near(a: f32, b: f32, delta: f32) -> bool {
    (b - a).abs() < delta
}

/// `PZMath.lerp`.
fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

/// The smoothed state of `calculateVisionCone` (`visionConeLerp`,
/// `lumaInvertedLerp`; static in Java, so shared by all players).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct VisionCone {
    pub cone_lerp: f32,
    pub luma_inverted_lerp: f32,
}

impl VisionCone {
    /// `calculateVisionCone(player)`: the `cone` argument of `playerSet`,
    /// `(degrees - 180) / 180`.
    pub fn calculate(&mut self, p: &VisionConeInput) -> f32 {
        let dls = p.day_light_strength;
        let inv = 1.0 - dls;
        let mut cone;
        if !p.in_vehicle {
            cone = 144.0f32;
            cone -= 72.0 * p.fatigue;
            if cone > 144.0 {
                cone = 144.0;
            }
            if p.fatigue_at_max {
                cone -= 36.0;
            }
            if p.drunk_level >= 2 {
                cone -= 0.36 * p.intoxication;
            }
            if p.panic_level == 4 {
                cone -= 36.0;
            }
            if !near(self.luma_inverted_lerp, p.luma_inverted, 0.1) {
                self.luma_inverted_lerp = lerp(self.luma_inverted_lerp, p.luma_inverted, 0.1);
            }
            cone -= if p.in_room { 126.0 * self.luma_inverted_lerp } else { 126.0 * inv.min(self.luma_inverted_lerp) };
            cone = cone.clamp(18.0, 180.0);
            if p.eagle_eyed {
                cone += 36.0 * dls;
            }
            if p.night_vision {
                cone += 36.0 * inv;
            }
        } else {
            cone = 324.0;
            cone -= 540.0 * inv;
            if p.fatigue_at_max {
                cone -= 36.0;
            }
            if p.drunk_level >= 2 {
                cone -= 0.36 * p.intoxication;
            }
            if p.panic_level == 4 {
                cone -= 36.0;
            }
            if p.night_vision {
                cone += 36.0 * inv;
            }
            if p.headlights && cone < 36.0 {
                cone = 36.0;
            }
        }
        cone *= p.worn_items_vision_multiplier;
        cone = cone.clamp(18.0, 360.0);
        if !near(self.cone_lerp, cone, 0.033) {
            self.cone_lerp = lerp(self.cone_lerp, cone, 0.033);
        }
        (self.cone_lerp - 180.0) / 180.0
    }
}

/// `getLumaInverted(player)`: one minus the brighter of the light colours
/// of the player's square and the square ahead (`None` when missing).
pub fn luma_inverted(here: Option<ColorInfo>, ahead: Option<ColorInfo>) -> f32 {
    let luma = |c: Option<ColorInfo>| c.map_or(0.0, |c| c.r * 0.299 + c.g * 0.587 + c.b * 0.114);
    1.0 - luma(here).max(luma(ahead))
}

/// `IsoGameCharacter.getEffectiveFatigue()`.
pub fn effective_fatigue(fatigue: f32) -> f32 {
    (fatigue - 0.6).max(0.0) * 2.5
}

/// `IsoGameCharacter.getDetectionRange()`.
pub fn detection_range(
    deaf: bool,
    keen_hearing: bool,
    hard_of_hearing: bool,
    worn_items_hearing_multiplier: f32,
    fatigue: f32,
    intoxication: f32,
) -> f32 {
    let mut range = if deaf { 2.0f32 } else { 3.5 };
    range += if keen_hearing { 3.0 } else { 0.0 };
    range -= if hard_of_hearing { 1.0 } else { 0.0 };
    let adjustable = range.max(2.0) - 2.0;
    let mut range = 2.0 + adjustable * worn_items_hearing_multiplier;
    range -= fatigue;
    range - intoxication * 0.01
}
