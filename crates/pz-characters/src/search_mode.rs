//! `zombie.iso.SearchMode`: the per-player search-mode screen effect (the
//! foraging overlay). Each player has a [`PlayerSearchMode`] with an
//! enabled flag, a fade in or out, and five [`SearchModeFloat`]s (blur,
//! desaturation, radius, darkness, gradient width), each with an exterior
//! and an interior value and target.
//!
//! `IngameState.UpdateStuff` calls [`SearchMode::update`] once per frame
//! with `GameTime.getTimeDelta()`; `WeatherShader.startMainThread` reads
//! [`PlayerSearchMode::shader`] for the player being drawn; Lua
//! (`ISSearchManager`) sets the targets and the enabled flag through
//! `getSearchMode()`.
//!
//! The original reads `IsoPlayer.players[plrIndex]`'s square for
//! `isPlayerExterior`; here the caller passes it ([`is_player_exterior`]).

use crate::character::GameCharacter;

/// `IsoPlayer.MAX`: `SearchMode.plrModes`' length.
pub const MAX_PLAYERS: usize = 4;

/// `PZMath.clamp(float, float, float)`: `min` when below it, else `max`
/// when above it, else `v` (no ordering check, NaN passes through).
fn clamp(v: f32, min: f32, max: f32) -> f32 {
    if v < min {
        min
    } else if v > max {
        max
    } else {
        v
    }
}

/// `PZMath.equal(a, b, delta)`.
fn equal(a: f32, b: f32, delta: f32) -> bool {
    (b - a).abs() < delta
}

/// `PZMath.lerp(a, b, t)`.
fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

/// `SearchMode.SearchModeFloat`: an exterior and an interior value, each
/// moving toward its target, with the `min`/`max`/`stepsize` the debug UI
/// uses for its sliders.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SearchModeFloat {
    min: f32,
    max: f32,
    stepsize: f32,
    exterior: f32,
    target_exterior: f32,
    interior: f32,
    target_interior: f32,
}

impl SearchModeFloat {
    /// The private constructor: the values and targets are zero.
    fn new(min: f32, max: f32, stepsize: f32) -> Self {
        SearchModeFloat { min, max, stepsize, exterior: 0.0, target_exterior: 0.0, interior: 0.0, target_interior: 0.0 }
    }

    /// `set(exterior, targetExterior, interior, targetInterior)`.
    pub fn set(&mut self, exterior: f32, target_exterior: f32, interior: f32, target_interior: f32) {
        self.set_exterior(exterior);
        self.set_target_exterior(target_exterior);
        self.set_interior(interior);
        self.set_target_interior(target_interior);
    }

    /// `setAll(v)`.
    pub fn set_all(&mut self, v: f32) {
        self.set(v, v, v, v);
    }

    /// `setTargets(exterior, interior)`.
    pub fn set_targets(&mut self, exterior: f32, interior: f32) {
        self.set_target_exterior(exterior);
        self.set_target_interior(interior);
    }

    /// `getExterior()`.
    pub fn exterior(&self) -> f32 {
        self.exterior
    }
    /// `setExterior(v)`.
    pub fn set_exterior(&mut self, v: f32) {
        self.exterior = v;
    }
    /// `getTargetExterior()`.
    pub fn target_exterior(&self) -> f32 {
        self.target_exterior
    }
    /// `setTargetExterior(v)`.
    pub fn set_target_exterior(&mut self, v: f32) {
        self.target_exterior = v;
    }
    /// `getInterior()`.
    pub fn interior(&self) -> f32 {
        self.interior
    }
    /// `setInterior(v)`.
    pub fn set_interior(&mut self, v: f32) {
        self.interior = v;
    }
    /// `getTargetInterior()`.
    pub fn target_interior(&self) -> f32 {
        self.target_interior
    }
    /// `setTargetInterior(v)`.
    pub fn set_target_interior(&mut self, v: f32) {
        self.target_interior = v;
    }

    /// `update(delta)`: both values become `delta` times their targets
    /// (the fade).
    pub fn update(&mut self, delta: f32) {
        self.exterior = delta * self.target_exterior;
        self.interior = delta * self.target_interior;
    }

    /// `equalise()`: each value lerps a hundredth of the way to its target,
    /// or snaps to it once within 0.001.
    pub fn equalise(&mut self) {
        self.exterior = if !equal(self.exterior, self.target_exterior, 0.001) {
            lerp(self.exterior, self.target_exterior, 0.01)
        } else {
            self.target_exterior
        };
        self.interior = if !equal(self.interior, self.target_interior, 0.001) {
            lerp(self.interior, self.target_interior, 0.01)
        } else {
            self.target_interior
        };
    }

    /// `reset()`: the values to zero (the targets stay).
    pub fn reset(&mut self) {
        self.exterior = 0.0;
        self.interior = 0.0;
    }

    /// `resetAll()`: the values and targets to zero.
    pub fn reset_all(&mut self) {
        self.exterior = 0.0;
        self.interior = 0.0;
        self.target_interior = 0.0;
        self.target_exterior = 0.0;
    }

    /// `getMin()`.
    pub fn min(&self) -> f32 {
        self.min
    }
    /// `getMax()`.
    pub fn max(&self) -> f32 {
        self.max
    }
    /// `getStepsize()`.
    pub fn stepsize(&self) -> f32 {
        self.stepsize
    }
}

/// Which [`SearchModeFloat`] of a [`PlayerSearchMode`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SearchModeParam {
    Radius,
    GradientWidth,
    Blur,
    Desat,
    Darkness,
}

impl SearchModeParam {
    /// Every parameter, in the original's field order.
    pub const ALL: [SearchModeParam; 5] = [
        SearchModeParam::Radius,
        SearchModeParam::GradientWidth,
        SearchModeParam::Blur,
        SearchModeParam::Desat,
        SearchModeParam::Darkness,
    ];
}

/// What `WeatherShader.startMainThread` reads of a player's search mode:
/// each `getShader*` value is the exterior or interior one by
/// `isPlayerExterior()`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SearchModeShaderInputs {
    /// `isShaderEnabled()`.
    pub enabled: bool,
    /// `getShaderBlur()`.
    pub blur: f32,
    /// `getShaderRadius()`.
    pub radius: f32,
    /// `getShaderGradientWidth()`.
    pub gradient_width: f32,
    /// `getShaderDesat()`.
    pub desat: f32,
    /// `getShaderDarkness()`.
    pub darkness: f32,
}

/// `SearchMode.PlayerSearchMode`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PlayerSearchMode {
    plr_index: i32,
    /// `override`: [`PlayerSearchMode::update`] does nothing while set (the
    /// debug UI drives the values).
    pub override_: bool,
    /// `overrideSearchManager`: read by `ISSearchManager`.
    pub override_search_manager: bool,
    enabled: bool,
    pub radius: SearchModeFloat,
    pub gradient_width: SearchModeFloat,
    pub blur: SearchModeFloat,
    pub desat: SearchModeFloat,
    pub darkness: SearchModeFloat,
    timer: f32,
    do_fade_out: bool,
    do_fade_in: bool,
}

impl PlayerSearchMode {
    /// `new PlayerSearchMode(plrIndex, parent)`: every value and target
    /// zero, the floats' ranges `radius` 0..50 step 1, `gradientWidth`
    /// 0..20 step 1, the rest 0..1 step 0.01.
    pub fn new(plr_index: i32) -> Self {
        PlayerSearchMode {
            plr_index,
            override_: false,
            override_search_manager: false,
            enabled: false,
            radius: SearchModeFloat::new(0.0, 50.0, 1.0),
            gradient_width: SearchModeFloat::new(0.0, 20.0, 1.0),
            blur: SearchModeFloat::new(0.0, 1.0, 0.01),
            desat: SearchModeFloat::new(0.0, 1.0, 0.01),
            darkness: SearchModeFloat::new(0.0, 1.0, 0.01),
            timer: 0.0,
            do_fade_out: false,
            do_fade_in: false,
        }
    }

    /// `plrIndex`.
    pub fn plr_index(&self) -> i32 {
        self.plr_index
    }

    /// `enabled` (`SearchMode.isEnabled(plrIndex)`).
    pub fn enabled(&self) -> bool {
        self.enabled
    }

    /// `timer`, `doFadeIn`, `doFadeOut`.
    pub fn fade_state(&self) -> (f32, bool, bool) {
        (self.timer, self.do_fade_in, self.do_fade_out)
    }

    /// The [`SearchModeFloat`] `p` (`getRadius()` and so on).
    pub fn param(&self, p: SearchModeParam) -> &SearchModeFloat {
        match p {
            SearchModeParam::Radius => &self.radius,
            SearchModeParam::GradientWidth => &self.gradient_width,
            SearchModeParam::Blur => &self.blur,
            SearchModeParam::Desat => &self.desat,
            SearchModeParam::Darkness => &self.darkness,
        }
    }

    /// The [`SearchModeFloat`] `p`, mutably.
    pub fn param_mut(&mut self, p: SearchModeParam) -> &mut SearchModeFloat {
        match p {
            SearchModeParam::Radius => &mut self.radius,
            SearchModeParam::GradientWidth => &mut self.gradient_width,
            SearchModeParam::Blur => &mut self.blur,
            SearchModeParam::Desat => &mut self.desat,
            SearchModeParam::Darkness => &mut self.darkness,
        }
    }

    /// `isShaderEnabled()`: enabled, or fading in or out.
    pub fn is_shader_enabled(&self) -> bool {
        self.enabled || self.do_fade_in || self.do_fade_out
    }

    /// `getShader<P>()`: the exterior value when `exterior`
    /// (`isPlayerExterior()`), else the interior one.
    pub fn shader_value(&self, p: SearchModeParam, exterior: bool) -> f32 {
        let f = self.param(p);
        if exterior {
            f.exterior()
        } else {
            f.interior()
        }
    }

    /// Everything `WeatherShader.startMainThread` reads, `exterior` being
    /// `isPlayerExterior()` ([`is_player_exterior`]).
    pub fn shader(&self, exterior: bool) -> SearchModeShaderInputs {
        SearchModeShaderInputs {
            enabled: self.is_shader_enabled(),
            blur: self.shader_value(SearchModeParam::Blur, exterior),
            radius: self.shader_value(SearchModeParam::Radius, exterior),
            gradient_width: self.shader_value(SearchModeParam::GradientWidth, exterior),
            desat: self.shader_value(SearchModeParam::Desat, exterior),
            darkness: self.shader_value(SearchModeParam::Darkness, exterior),
        }
    }

    /// The fade step's floats: blur, desat, radius and darkness follow
    /// `delta`, the gradient width equalises.
    fn fade(&mut self, delta: f32) {
        self.blur.update(delta);
        self.desat.update(delta);
        self.radius.update(delta);
        self.darkness.update(delta);
        self.gradient_width.equalise();
    }

    /// The private `update()`, `parent.fadeTime` being `fade_time` and
    /// `GameTime.getTimeDelta()` `time_delta`.
    pub fn update(&mut self, fade_time: f32, time_delta: f32) {
        if self.override_ {
            return;
        }
        if self.do_fade_in {
            self.timer += time_delta;
            self.timer = clamp(self.timer, 0.0, fade_time);
            let delta = clamp(self.timer / fade_time, 0.0, 1.0);
            self.fade(delta);
            if self.timer >= fade_time {
                self.do_fade_in = false;
            }
            return;
        }
        if self.do_fade_out {
            self.timer -= time_delta;
            self.timer = clamp(self.timer, 0.0, fade_time);
            let delta = clamp(self.timer / fade_time, 0.0, 1.0);
            self.fade(delta);
            // `timer <= 0`: `fcmpg` makes a NaN timer greater, so it stays.
            if self.timer <= 0.0 {
                self.do_fade_out = false;
            }
            return;
        }
        if self.enabled {
            self.blur.equalise();
            self.desat.equalise();
            self.radius.equalise();
            self.darkness.equalise();
            self.gradient_width.equalise();
        } else {
            self.blur.reset();
            self.desat.reset();
            self.radius.reset();
            self.darkness.reset();
            self.gradient_width.equalise();
        }
    }
}

/// `SearchMode` (the `getInstance()` singleton's state).
#[derive(Clone, Debug, PartialEq)]
pub struct SearchMode {
    /// `fadeTime` (1 from the constructor).
    pub fade_time: f32,
    plr_modes: [PlayerSearchMode; MAX_PLAYERS],
}

impl Default for SearchMode {
    fn default() -> Self {
        Self::new()
    }
}

impl SearchMode {
    /// The private constructor: `fadeTime` 1 and, for each player, the
    /// targets blur 1, desat 0.85, radius 4, darkness 0, gradient width 4
    /// (exterior and interior alike).
    pub fn new() -> Self {
        let plr_modes = std::array::from_fn(|i| {
            let mut m = PlayerSearchMode::new(i as i32);
            m.blur.set_targets(1.0, 1.0);
            m.desat.set_targets(0.85, 0.85);
            m.radius.set_targets(4.0, 4.0);
            m.darkness.set_targets(0.0, 0.0);
            m.gradient_width.set_targets(4.0, 4.0);
            m
        });
        SearchMode { fade_time: 1.0, plr_modes }
    }

    /// `getSearchModeForPlayer(i)`; `None` where the original's array
    /// access throws.
    pub fn player(&self, i: usize) -> Option<&PlayerSearchMode> {
        self.plr_modes.get(i)
    }

    /// `getSearchModeForPlayer(i)`, mutably.
    pub fn player_mut(&mut self, i: usize) -> Option<&mut PlayerSearchMode> {
        self.plr_modes.get_mut(i)
    }

    /// `getFadeTime()`.
    pub fn fade_time(&self) -> f32 {
        self.fade_time
    }

    /// `setFadeTime(v)`.
    pub fn set_fade_time(&mut self, v: f32) {
        self.fade_time = v;
    }

    /// `isEnabled(i)`.
    pub fn is_enabled(&self, i: usize) -> Option<bool> {
        self.player(i).map(|m| m.enabled)
    }

    /// `setEnabled(i, b)`: turning on fades in (`timer = max(timer, 0)`),
    /// turning off fades out (`timer = min(timer, fadeTime)`); setting the
    /// current value does nothing. `None` for a bad index.
    pub fn set_enabled(&mut self, i: usize, b: bool) -> Option<()> {
        let fade_time = self.fade_time;
        let m = self.plr_modes.get_mut(i)?;
        if b && !m.enabled {
            m.enabled = true;
            // FadeIn: Math.max(timer, 0f).
            m.timer = java_max(m.timer, 0.0);
            m.do_fade_in = true;
            m.do_fade_out = false;
        } else if !b && m.enabled {
            m.enabled = false;
            // FadeOut: Math.min(timer, fadeTime).
            m.timer = java_min(m.timer, fade_time);
            m.do_fade_in = false;
            m.do_fade_out = true;
        }
        Some(())
    }

    /// `update()`: every player's update with `GameTime.getTimeDelta()`.
    pub fn update(&mut self, time_delta: f32) {
        let fade_time = self.fade_time;
        for m in &mut self.plr_modes {
            m.update(fade_time, time_delta);
        }
    }
}

/// `Math.max(float, float)`: NaN if either is NaN.
fn java_max(a: f32, b: f32) -> f32 {
    if a.is_nan() || b.is_nan() {
        f32::NAN
    } else {
        a.max(b)
    }
}

/// `Math.min(float, float)`: NaN if either is NaN.
fn java_min(a: f32, b: f32) -> f32 {
    if a.is_nan() || b.is_nan() {
        f32::NAN
    } else {
        a.min(b)
    }
}

/// `PlayerSearchMode.isPlayerExterior()`: `IsoPlayer.players[plrIndex]` is
/// set, has a current square, and that square is not in a room (the
/// character's [`environment`](GameCharacter::environment) as the world
/// filled it).
pub fn is_player_exterior<I: 'static>(player: Option<&GameCharacter<I>>) -> bool {
    player.is_some_and(|p| p.environment.has_current_square && !p.environment.square_in_room)
}

#[cfg(test)]
mod tests;
