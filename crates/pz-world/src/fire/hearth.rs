//! The gameplay of `IsoFireplace` and `IsoBarbecue`: fuel, lighting and
//! putting out, and the fuel their `update()` burns (or smoulders for ten
//! minutes after being put out) as world time passes.
//!
//! Their flames, smoke, fuel logs and light are `pz-iso-render`'s
//! `FireplaceState` / `BarbecueState`, which read the lit flag and fuel
//! kept here; `pz-world-render`'s `HearthLink` runs both halves of
//! `update()` in the original's order. Heat sources and sounds are not
//! modelled.

use crate::objects::{IsoBarbecue, IsoFireplace};

/// `IsoFireplace.SmoulderMinutes` (the barbecue uses the same 10).
pub const SMOULDER_MINUTES: i32 = 10;

/// The fields the fuel step reads and writes.
struct Fuel<'a> {
    lit: &'a mut bool,
    fuel: &'a mut i32,
    last_update_time: &'a mut f32,
    accumulator: &'a mut f32,
    since_out: &'a mut i32,
    smouldering: &'a mut bool,
}

/// The fuel step both `update()`s share: whole minutes of world time since
/// the last update burn fuel while lit (going out without fuel), or
/// smoulder it for a while after being put out.
fn burn_fuel(f: Fuel<'_>, world_age_hours: f64) {
    let elapsed_hours = world_age_hours as f32;
    if *f.last_update_time < 0.0 || *f.last_update_time > elapsed_hours {
        *f.last_update_time = elapsed_hours;
    }
    if elapsed_hours > *f.last_update_time {
        *f.accumulator += (elapsed_hours - *f.last_update_time) * 60.0;
        let minutes = f.accumulator.floor() as i32;
        if minutes > 0 {
            let use_fuel = |fuel: &mut i32, amount: i32| {
                let used = (*fuel).min(amount);
                *fuel = (*fuel - used).max(0);
            };
            if *f.lit {
                use_fuel(f.fuel, minutes);
                if *f.fuel <= 0 {
                    // extinguish() with no fuel left: no smouldering.
                    *f.lit = false;
                }
            } else if *f.since_out != -1 {
                let smoulder = minutes.min(SMOULDER_MINUTES - *f.since_out);
                *f.since_out += minutes;
                *f.smouldering = true;
                use_fuel(f.fuel, smoulder);
                if *f.fuel <= 0 || *f.since_out >= SMOULDER_MINUTES {
                    *f.since_out = -1;
                    *f.smouldering = false;
                }
            }
            *f.accumulator -= minutes as f32;
        }
    }
    *f.last_update_time = elapsed_hours;
}

impl IsoFireplace {
    /// `getFuelAmount()`.
    pub fn fuel_amount(&self) -> i32 {
        self.fuel_amount_minutes
    }

    /// `setFuelAmount(units)` (never below 0).
    pub fn set_fuel_amount(&mut self, units: i32) {
        self.fuel_amount_minutes = units.max(0);
    }

    /// `addFuel(units)`.
    pub fn add_fuel(&mut self, units: i32) {
        self.set_fuel_amount(self.fuel_amount() + units);
    }

    /// `useFuel(amount)`: what was actually used.
    pub fn use_fuel(&mut self, amount: i32) -> i32 {
        let avail = self.fuel_amount();
        let used = avail.min(amount);
        self.set_fuel_amount(avail - used);
        used
    }

    /// `hasFuel()`.
    pub fn has_fuel(&self) -> bool {
        self.fuel_amount() > 0
    }

    /// `turnOn()`: true when it was lit now (the caller then runs
    /// `getContainer().addItemsToProcessItems()`).
    pub fn turn_on(&mut self) -> bool {
        if self.lit {
            return false;
        }
        self.lit = true;
        true
    }

    /// `extinguish()`: put out, smouldering if fuel is left.
    pub fn extinguish(&mut self) {
        if self.lit {
            self.lit = false;
            if self.has_fuel() {
                self.minutes_since_extinguished = 0;
            }
        }
    }

    /// `getTemperature()`.
    pub fn temperature(&self) -> f32 {
        if self.lit {
            1.8
        } else {
            1.0
        }
    }

    /// `calcLightRadius()`: 1 to 8 squares over the first hour of fuel
    /// (the heat source's radius too).
    pub fn calc_light_radius(&self) -> i32 {
        let delta = (self.fuel_amount() as f32).min(60.0) / 60.0;
        (1.0 + 7.0 * delta.clamp(0.0, 1.0)) as i32
    }

    /// The fuel part of `update()` (before the sprites and light), at world
    /// age `world_age_hours` (`GameTime.getWorldAgeHours()`).
    pub fn update_fuel(&mut self, world_age_hours: f64) {
        burn_fuel(
            Fuel {
                lit: &mut self.lit,
                fuel: &mut self.fuel_amount_minutes,
                last_update_time: &mut self.last_update_time,
                accumulator: &mut self.minute_accumulator,
                since_out: &mut self.minutes_since_extinguished,
                smouldering: &mut self.smouldering,
            },
            world_age_hours,
        );
    }
}

impl IsoBarbecue {
    /// `getFuelAmount()`.
    pub fn fuel_amount(&self) -> i32 {
        self.fuel_amount_minutes
    }

    /// `setFuelAmount(units)`.
    pub fn set_fuel_amount(&mut self, units: i32) {
        self.fuel_amount_minutes = units.max(0);
    }

    /// `addFuel(units)`.
    pub fn add_fuel(&mut self, units: i32) {
        self.set_fuel_amount(self.fuel_amount() + units);
    }

    /// `useFuel(amount)`.
    pub fn use_fuel(&mut self, amount: i32) -> i32 {
        let avail = self.fuel_amount();
        let used = avail.min(amount);
        self.set_fuel_amount(avail - used);
        used
    }

    /// `hasFuel()`.
    pub fn has_fuel(&self) -> bool {
        self.fuel_amount() > 0
    }

    /// `hasPropaneTank()`; `propane_bbq` is `isPropaneBBQ()` (the sprite
    /// has the `PropaneTank` property).
    pub fn has_propane_tank(&self, propane_bbq: bool) -> bool {
        propane_bbq && self.has_propane_tank
    }

    /// `setPropaneTank(tank)` for a `Base.PropaneTank` with `uses` left:
    /// 0.24 minutes of fuel per use.
    pub fn set_propane_tank(&mut self, uses: i32) {
        self.has_propane_tank = true;
        self.fuel_amount_minutes = (uses as f32 * 0.24).round() as i32;
    }

    /// `turnOn()`: true when it was lit now.
    pub fn turn_on(&mut self) -> bool {
        if self.lit {
            return false;
        }
        self.lit = true;
        true
    }

    /// `turnOff()`.
    pub fn turn_off(&mut self) {
        self.lit = false;
    }

    /// `extinguish()`: charcoal smoulders, propane does not.
    pub fn extinguish(&mut self, propane_bbq: bool) {
        if self.lit {
            self.lit = false;
            if self.has_fuel() && !propane_bbq {
                self.minutes_since_extinguished = 0;
            }
        }
    }

    /// `getTemperature()`.
    pub fn temperature(&self) -> f32 {
        if self.lit {
            1.8
        } else {
            1.0
        }
    }

    /// The fuel part of `update()`.
    pub fn update_fuel(&mut self, world_age_hours: f64) {
        burn_fuel(
            Fuel {
                lit: &mut self.lit,
                fuel: &mut self.fuel_amount_minutes,
                last_update_time: &mut self.last_update_time,
                accumulator: &mut self.minute_accumulator,
                since_out: &mut self.minutes_since_extinguished,
                smouldering: &mut self.smouldering,
            },
            world_age_hours,
        );
    }

    /// `updateSprite()`: a propane barbecue shows its tank or not.
    pub fn sprite(&self, current: Option<usize>, propane_bbq: bool) -> Option<usize> {
        if !propane_bbq {
            return current;
        }
        if self.has_propane_tank(propane_bbq) {
            self.normal_sprite
        } else {
            self.no_tank_sprite
        }
    }
}
