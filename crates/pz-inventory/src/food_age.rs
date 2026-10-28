//! The numerical phases of `Food.updateAge` and `updateFreezing`.
//!
//! The caller retains the actual item and container, reads their live
//! state between phases, and performs process-list and Lua event calls
//! without holding an item borrow. These helpers do not sample a clock,
//! resolve power, recurse through bags, or dispatch events.

use crate::item::FoodData;
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

/// The two predicates compared before and after aging.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AgeState {
    pub fresh: bool,
    pub rotten: bool,
}

/// A local `OnContainerUpdate` is required if either predicate changed.
/// The server branch instead uses the shared update-rate result.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AgeTransition {
    pub before: AgeState,
    pub after: AgeState,
}

impl AgeTransition {
    pub fn changed(self) -> bool {
        self.before != self.after
    }
}

/// `GameTime.checkHours(float, float)`, whose min is `PZMath.min`.
/// Ordered comparisons deliberately preserve a NaN or signed-zero last
/// value, unlike `f32::min`.
fn check_hours(last: f32, now: f32) -> f32 {
    if last < 0.0 || last > now {
        now
    } else {
        last
    }
}

impl FoodData {
    /// Apply the native checked-time prefix before any freezing queries.
    /// The host commits `last_frozen_update = now` only after all later
    /// freezing/thawing host calls and mutations have succeeded.
    pub fn begin_freezing_update(&mut self, now: f32) -> Option<f32> {
        self.last_frozen_update = check_hours(self.last_frozen_update, now);
        if now > self.last_frozen_update {
            Some(now - self.last_frozen_update)
        } else {
            None
        }
    }

    /// The body of the true `isFreezing()` branch. The caller evaluates
    /// that live predicate before borrowing the food for this phase.
    pub fn apply_freezing(&mut self, elapsed_hours: f32) {
        self.fertilized = false;
        self.set_freezing_time(self.freezing_time + elapsed_hours / 4.0f32 * 100.0f32);
    }

    /// The body of the independently evaluated `isThawing()` branch.
    /// `hours_to_thaw` comes from the original retained container's live
    /// fridge/power/temperature queries after the freezing phase.
    pub fn apply_thawing(&mut self, elapsed_hours: f32, hours_to_thaw: f32) {
        self.set_freezing_time(self.freezing_time - elapsed_hours / hours_to_thaw * 100.0f32);
    }

    /// `setFreezingTime`: the interior range preserves the previous
    /// frozen flag. NaN also preserves that flag and remains NaN.
    pub fn set_freezing_time(&mut self, mut value: f32) {
        if value >= 100.0 {
            self.frozen = true;
            value = 100.0;
        } else if value <= 0.0 {
            self.frozen = false;
            value = 0.0;
        }
        self.freezing_time = value;
    }

    /// Apply the native checked-time prefix after freezing finishes.
    /// The subtraction is float before its result is promoted to double.
    pub fn begin_age_update(&mut self, now: f32) -> Option<f64> {
        self.last_aged = check_hours(self.last_aged, now);
        if now > self.last_aged {
            Some((now - self.last_aged) as f64)
        } else {
            None
        }
    }

    pub fn age_state(&self) -> AgeState {
        AgeState {
            fresh: !self.burnt && self.off_age < 1_000_000_000 && self.age < self.off_age as f32,
            rotten: !self.burnt
                && self.off_age_max < 1_000_000_000
                && self.age >= self.off_age_max as f32,
        }
    }

    /// Commit aging after heat, process-list and refrigeration phases.
    /// The host releases the item borrow before dispatching any event.
    pub fn apply_age_delta(
        &mut self,
        now: f32,
        elapsed_hours: f64,
        rot_speed: f32,
    ) -> AgeTransition {
        let before = self.age_state();
        self.age = (self.age as f64 + elapsed_hours * rot_speed as f64 / 24.0f64) as f32;
        self.last_aged = now;
        AgeTransition {
            before,
            after: self.age_state(),
        }
    }
}

/// The enum-valued `SandboxOptions.fridgeFactor` mapping.
pub fn fridge_factor(option: i32) -> f32 {
    match option {
        1 => 0.4,
        2 => 0.3,
        4 => 0.1,
        5 => 0.03,
        6 => 0.0,
        _ => 0.2,
    }
}

/// The enum-valued `SandboxOptions.foodRotSpeed` mapping.
pub fn food_rot_speed(option: i32) -> f32 {
    match option {
        1 => 1.7,
        2 => 1.4,
        4 => 0.7,
        5 => 0.4,
        _ => 1.0,
    }
}

/// `GameTime.Lerp`: clamp with ordered branches, then float arithmetic.
pub fn lerp_heat(a: f32, b: f32, mut factor: f32) -> f32 {
    if factor < 0.0 {
        factor = 0.0;
    }
    if factor >= 1.0 {
        factor = 1.0;
    }
    let delta = b - a;
    let scaled = delta * factor;
    a + scaled
}

/// Thawing takes twice as long in a powered fridge, and one sixth as
/// long when the retained container's temperature is greater than one.
pub fn thaw_hours(powered_fridge: bool, hot_container: bool) -> f32 {
    let mut hours = 1.5f32;
    if powered_fridge {
        hours *= 2.0f32;
    }
    if hot_container {
        hours /= 6.0f32;
    }
    hours
}

/// The current installed version multiplies frozen age hours by zero.
/// Keep the multiplication: infinity and NaN must not become finite zero.
pub fn frozen_age_hours(elapsed_hours: f64) -> f64 {
    elapsed_hours * 0.0f64
}

/// `java.lang.Math.min(float,float)` in the power-shutoff calculation.
fn java_min(a: f32, b: f32) -> f32 {
    if a.is_nan() {
        a
    } else if a == 0.0 && b == 0.0 && b.is_sign_negative() {
        b
    } else if a <= b {
        a
    } else {
        b
    }
}

/// The fridge/freezer branch without current generator power. Before
/// shutoff it uses the fridge factor; crossing shutoff adds ambient time.
/// The native integer product wraps, and the cooled part stays float
/// until its promotion to double.
pub fn outage_hours(last_aged: f32, now: f32, shut_modifier: i32, factor: f32) -> f64 {
    let mut elapsed = (now - last_aged) as f64;
    let shut = shut_modifier.wrapping_mul(24) as f32;
    if shut_modifier > -1 && last_aged < shut {
        let until = java_min(shut, now);
        elapsed = ((until - last_aged) * factor) as f64;
        if now > shut {
            elapsed += (now - shut) as f64;
        }
    }
    elapsed
}

/// Deterministic state for Food's shared `new UpdateLimit(1000)`.
/// Its owner supplies real wall-clock milliseconds and retains one
/// instance for the class lifetime. A false result only suppresses the
/// server-stats tail; it never suppresses aging or local events.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FoodAgeRate {
    last: i64,
}

impl FoodAgeRate {
    pub fn new_at(now_ms: i64) -> Self {
        Self { last: now_ms }
    }

    pub fn last(&self) -> i64 {
        self.last
    }

    pub fn check_at(&mut self, now_ms: i64) -> bool {
        let elapsed = now_ms.wrapping_sub(self.last);
        if elapsed > 1000 {
            self.last = if elapsed > 3000 {
                now_ms
            } else {
                self.last.wrapping_add(1000)
            };
            true
        } else {
            false
        }
    }
}

static UPDATE_RATE: OnceLock<Mutex<FoodAgeRate>> = OnceLock::new();

fn now_ms() -> i64 {
    match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(duration) => duration.as_millis() as i64,
        Err(before_epoch) => (before_epoch.duration().as_millis() as i64).wrapping_neg(),
    }
}

fn shared_update_rate() -> &'static Mutex<FoodAgeRate> {
    UPDATE_RATE.get_or_init(|| Mutex::new(FoodAgeRate::new_at(now_ms())))
}

/// Called on Food construction, before its instance field initialization,
/// corresponding to the native class initializer's shared timer.
pub(crate) fn initialize_update_rate() {
    let _ = shared_update_rate();
}

/// `Food.updateAgeRate.Check()` over actual wall-clock milliseconds.
/// The single shared timer survives menu/world replacement, as does the
/// native static. This result only gates server item-stat transmission.
pub fn check_update_rate() -> bool {
    let mut rate = shared_update_rate()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    rate.check_at(now_ms())
}

#[cfg(test)]
mod tests;
