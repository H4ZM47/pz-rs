//! `zombie.characters.Safety`: the PvP safety state of a character.
//!
//! Not ported: `isToggleAllowed` and `toggleSafety` (they read
//! `ServerOptions` and `NonPvpZone` and send network packets), and
//! `load`/`save` (network packets).

/// `Safety`. Each `IsoGameCharacter` constructor makes one for its
/// character.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Safety {
    pub enabled: bool,
    pub last: bool,
    pub cooldown: f32,
    pub toggle: f32,
}

impl Default for Safety {
    /// `Safety(IsoGameCharacter)`: enabled and last true, both timers 0.
    fn default() -> Self {
        Self { enabled: true, last: true, cooldown: 0.0, toggle: 0.0 }
    }
}

impl Safety {
    /// `getDescription()`.
    pub fn description(&self) -> String {
        format!(
            "enabled={} last={} cooldown={} toggle={}",
            self.enabled,
            self.last,
            pz_core::debug::java_float_to_string(self.cooldown),
            pz_core::debug::java_float_to_string(self.toggle)
        )
    }
}
