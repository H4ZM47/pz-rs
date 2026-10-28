//! `CombatManager`'s firearm hit chance: the distance, aim-delay, move,
//! moodle, weather and pain penalties and `calculateHitChanceData`.

use pz_scripts::ScriptManager;

use super::weapon::FirearmStats;
use crate::character::combat::config;
use crate::character::state::{StateCx, StateWorld};
use crate::character::{GameCharacter, SimContext};
use crate::Characters;

/// `HitChanceData`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct HitChanceData {
    pub hit_chance: f32,
    pub aim_penalty: f32,
}

/// What the hit chance reads of the target (`hitInfo.getObject()`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TargetInfo {
    pub x: f32,
    pub y: f32,
    pub prone: bool,
    /// The target is an `IsoPlayer`.
    pub player: bool,
    /// `getVehicle() != null` and its `getCurrentSpeedKmHour()`.
    pub in_vehicle: bool,
    pub vehicle_speed_kmh: f32,
    pub sprinting: bool,
    pub running: bool,
    pub player_moving: bool,
    /// `getSquare()`'s position, when the target is on a square.
    pub square: Option<(i32, i32, i32)>,
}

impl TargetInfo {
    /// The fields of `t`.
    pub fn of<I: 'static>(sm: &ScriptManager, ctx: &dyn SimContext, t: &GameCharacter<I>, on_square: bool) -> TargetInfo {
        TargetInfo {
            x: t.vars.x,
            y: t.vars.y,
            prone: t.is_prone(),
            player: t.is_player(),
            in_vehicle: t.environment.in_vehicle,
            vehicle_speed_kmh: t.environment.vehicle_speed_kmh,
            sprinting: t.is_sprinting(sm, ctx),
            running: t.is_running(sm),
            player_moving: t.player_moving,
            square: on_square.then(|| t.square_xyz()),
        }
    }
}

/// What the drive-by penalty reads of the shooter's vehicle.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct VehicleAim {
    /// `getForwardVector()`'s `(x, z)`.
    pub forward_x: f32,
    pub forward_z: f32,
    /// `isPointLeftOfCenter(owner.x, owner.y)`.
    pub point_left_of_center: bool,
    /// `isDriver(owner)`.
    pub driver: bool,
    /// `getPartForSeatContainer(seat).getItemContainer().getCapacityWeight()`.
    pub seat_container_weight: Option<f32>,
    /// `owner.getInventory().getCapacityWeight()`.
    pub inventory_weight: f32,
    /// `getCurrentSpeedKmHour()`.
    pub speed_kmh: f32,
}

/// `getDistanceModifierSightless(dist, prone)`.
pub fn distance_modifier_sightless(dist: f32, prone: bool) -> f32 {
    let pbd = config::POINT_BLANK_DISTANCE;
    if dist > pbd {
        return (pbd - dist) * (config::SIGHTLESS_TO_HIT_BASE_DISTANCE + (pbd - dist) * -config::POINT_BLANK_DROP_OFF_TO_HIT_PENALTY);
    }
    if dist < pbd {
        return (pbd - dist) / pbd
            * config::POINT_BLANK_TO_HIT_MAXIMUM_BONUS
            * if prone { config::SIGHTLESS_TO_HIT_PRONE_MODIFIER } else { 1.0 };
    }
    0.0
}

/// `getAimDelayPenaltySightless(aimDelay, dist)`.
pub fn aim_delay_penalty_sightless(mut aim_delay: f32, dist: f32) -> f32 {
    let pbd = config::POINT_BLANK_DISTANCE;
    if dist < pbd {
        aim_delay *= dist / pbd;
    } else if dist > pbd {
        aim_delay *= 1.0 + (dist - pbd * config::SIGHTLESS_AIM_DELAY_TO_HIT_DISTANCE_MODIFIER);
    }
    aim_delay
}

/// `PZMath.pow(a, b)`: `(float) Math.pow(a, b)`.
fn powf(a: f32, b: f32) -> f32 {
    (a as f64).powf(b as f64) as f32
}

/// `getDistanceModifier(dist, min, max, prone)`.
pub fn distance_modifier(dist: f32, min: f32, max: f32, _prone: bool) -> f32 {
    let inc = config::OPTIMAL_RANGE_DROP_OFF_TO_HIT_PENALTY_INCREMENT;
    if dist < min {
        if dist > config::POINT_BLANK_DISTANCE {
            return (dist - min) * (config::OPTIMAL_RANGE_DROP_OFF_TO_HIT_PENALTY + (dist - min) * -inc);
        }
        return 0.0;
    }
    if dist > max {
        return -((dist - max) * (config::OPTIMAL_RANGE_DROP_OFF_TO_HIT_PENALTY + (dist - max) * inc));
    }
    let scale = (max - min) * 0.5;
    let e = -powf(dist - (min + scale), 2.0) / powf(2.0 * ((max - min) / 7.0), 2.0);
    (config::OPTIMAL_RANGE_TO_HIT_MAXIMUM_BONUS as f64 * (e as f64).exp()) as f32
}

/// `getAimDelayPenalty(delay, dist, min, max)`.
pub fn aim_delay_penalty(mut delay: f32, dist: f32, min: f32, max: f32) -> f32 {
    if min > -1.0 && dist >= min && dist <= max {
        let scale = (max - min) * 0.5;
        delay *= 1.0 - (1.0 - (dist - (min + scale)).abs() / scale) * 0.25;
    } else if dist > max {
        delay *= 1.0 + (dist - max) * 0.1;
    }
    let pbd = config::POINT_BLANK_DISTANCE;
    delay * if dist < pbd { dist / pbd } else { 1.0 }
}

impl<I: 'static> GameCharacter<I> {
    fn moodle_of(&self, sm: &ScriptManager, field: &str) -> i32 {
        self.moodles.moodle_level_of(sm, field).unwrap_or(0)
    }

    /// `CombatManager.getMovePenalty(character, dist)`.
    pub fn move_penalty(&self, chars: &Characters, dist: f32) -> f32 {
        let lv = self.perk_level(chars.perks.constant("Aiming").as_ref()) + self.perk_level(chars.perks.constant("Nimble").as_ref());
        let mut penalty = self.vars.been_moving_for * (1.0 - lv as f32 / 40.0);
        if dist < 10.0 {
            penalty *= dist / 10.0;
        } else {
            penalty *= 1.0 + (dist - 10.0) * 0.07;
        }
        penalty
    }

    /// `CombatManager.getMoodlesPenalty(character, distance)`.
    pub fn moodles_penalty(&self, sm: &ScriptManager, ctx: &dyn SimContext, distance: f32) -> f32 {
        (self.moodle_of(sm, "PANIC") as f32 * (config::PANIC_TO_HIT_BASE_PENALTY + distance * config::PANIC_TO_HIT_DISTANCE_MODIFIER)
            + self.moodle_of(sm, "STRESS") as f32 * (config::STRESS_TO_HIT_BASE_PENALTY + distance * config::STRESS_TO_HIT_DISTANCE_MODIFIER)
            + self.moodle_of(sm, "TIRED") as f32 * config::TIRED_TO_HIT_BASE_PENALTY
            + self.moodle_of(sm, "ENDURANCE") as f32 * config::ENDURANCE_TO_HIT_BASE_PENALTY
            + self.moodle_of(sm, "DRUNK") as f32 * (config::DRUNK_TO_HIT_BASE_PENALTY + distance * config::DRUNK_TO_HIT_DISTANCE_MODIFIER))
            * ctx.firearm_moodle_multiplier() as f32
    }

    /// `CombatManager.getPainPenalty(character)`.
    pub fn pain_penalty(&self, chars: &Characters, ctx: &dyn SimContext) -> f32 {
        let types = &chars.sim.body_part_types;
        let (Ok(from), Ok(to)) = (types.index("Hand_L"), types.index("UpperArm_R")) else { return 0.0 };
        let mut pain = 0.0f32;
        for x in from..=to {
            pain += self.body_damage.parts[x].pain(ctx);
        }
        pain * config::ARM_PAIN_TO_HIT_MODIFIER
    }

    /// `CombatManager.getWeatherPenalty(character, weapon, square,
    /// distance)`; `square` is `None` for a null square.
    #[allow(clippy::too_many_arguments)]
    pub fn weather_penalty(
        &self,
        sm: &ScriptManager,
        chars: &Characters,
        ctx: &dyn SimContext,
        world: Option<&dyn StateWorld<I>>,
        w: &FirearmStats,
        square: Option<(i32, i32, i32)>,
        distance: f32,
    ) -> f32 {
        let mut penalty = 0.0f32;
        let (Some(sq), Some(world)) = (square, world) else { return penalty };
        let thermal = w.thermal_sight;
        if world.square_is_outside(sq) {
            let aiming = self.perk_level(chars.perks.constant("Aiming").as_ref()) as f32;
            let marksman = if self.has_trait(sm, "MARKSMAN").unwrap_or(false) {
                config::WIND_INTENSITY_TO_HIT_MINIMUM_MARKSMAN_MODIFIER
            } else {
                config::WIND_INTENSITY_TO_HIT_MAXIMUM_MARKSMAN_MODIFIER
            };
            penalty += ctx.wind_intensity()
                * (config::WIND_INTENSITY_TO_HIT_PENALTY - aiming * config::WIND_INTENSITY_TO_HIT_AIMING_MODIFIER)
                * distance
                * marksman;
            penalty += ctx.rain_intensity() * distance * config::RAIN_INTENSITY_TO_HIT_DISTANCE_MODIFIER;
            penalty *= self.character_traits.trait_weather_penalty_modifier(sm).unwrap_or(1.0);
            if !thermal {
                penalty += ctx.fog_intensity() * config::FOG_INTENSITY_DISTANCE_MODIFIER * distance;
            }
            penalty *= (distance / config::POINT_BLANK_DISTANCE).min(config::POINT_BLANK_MAXIMUM_DISTANCE_MODIFIER);
            penalty *= ctx.firearm_weather_multiplier() as f32;
        }
        if thermal {
            return penalty;
        }
        let n = if self.is_player() { self.seen.player_index } else { -1 };
        let light = world.square_light_level(sq, n);
        if light < config::LOW_LIGHT_THRESHOLD {
            let light_penalty = 0.0f32.max(config::LOW_LIGHT_TO_HIT_MAXIMUM_PENALTY * (1.0 - light / config::LOW_LIGHT_THRESHOLD));
            penalty += light_penalty - w.low_light_bonus;
        }
        penalty
    }

    /// `CombatManager.calculateHitChanceData(owner, weapon, hitInfo)`:
    /// `hit` is the hit info's distance squared and target, `None` for a
    /// reticle without a target. `vehicle` is the shooter's vehicle.
    pub fn calculate_hit_chance_data(
        &self,
        cx: &StateCx<'_, I>,
        w: &FirearmStats,
        hit: Option<(f32, &TargetInfo)>,
        vehicle: Option<&VehicleAim>,
    ) -> HitChanceData {
        let (sm, chars) = (cx.sm, cx.chars);
        let mut data = HitChanceData::default();
        let target = hit.map(|h| h.1);
        let mut hit_chance = (w.hit_chance as f32).min(config::MAXIMUM_START_TO_HIT_CHANCE);
        hit_chance += w.aiming_perk_hit_chance_modifier * self.perk_level(chars.perks.constant("Aiming").as_ref()) as f32;
        if let (Some(v), Some(t)) = (vehicle, target) {
            let (mut ox, mut oy) = crate::character::zombie::math::normalize(v.forward_x, v.forward_z);
            let (zx, zy) = crate::character::zombie::math::normalize(t.x - self.vars.x, t.y - self.vars.y);
            let backwards = zx * ox + zy * oy < 0.0;
            let rotation: i32 = if v.point_left_of_center { 90 } else { -90 };
            (ox, oy) = rotate_vec((ox, oy), (rotation as f64).to_radians() as f32);
            (ox, oy) = crate::character::zombie::math::normalize(ox, oy);
            let facing = zx * ox + zy * oy;
            if facing > -0.6 && !w.ranged {
                return data;
            }
            if facing > config::DRIVEBY_DOT_MAXIMUM_ANGLE {
                return data;
            }
            if !w.ranged {
                data.hit_chance = config::MAXIMUM_TO_HIT_CHANCE;
                return data;
            }
            if v.driver && w.two_hand {
                return data;
            }
            if facing > config::DRIVEBY_DOT_OPTIMAL_ANGLE {
                let mut max_facing = config::DRIVEBY_DOT_MAXIMUM_ANGLE;
                if v.driver {
                    max_facing -= 0.15;
                }
                if let Some(cw) = v.seat_container_weight {
                    max_facing -= cw * if w.two_hand { 0.05 } else { 0.025 };
                }
                max_facing -= v.inventory_weight * 0.01;
                if backwards {
                    max_facing -= if w.two_hand { 0.15 } else { 0.1 };
                }
                let p = config::DRIVEBY_DOT_TO_HIT_MAXIMUM_PENALTY
                    * (1.0 - (facing - 0.1) / (config::DRIVEBY_DOT_OPTIMAL_ANGLE - 0.1));
                let penalty = clamp(p, 0.0, config::DRIVEBY_DOT_TO_HIT_MAXIMUM_PENALTY)
                    * if w.two_hand { 1.5 } else { 1.0 }
                    * if backwards { 1.5 } else { 1.0 }
                    * if facing <= max_facing { 1.0 } else { 3.0 };
                hit_chance -= penalty;
                data.aim_penalty += penalty;
            }
            let speed_penalty = v.speed_kmh.abs() * if v.driver { 3.0 } else { 2.0 };
            hit_chance -= speed_penalty;
            data.aim_penalty += speed_penalty;
        }
        if !w.ranged {
            data.hit_chance = config::MAXIMUM_TO_HIT_CHANCE;
            return data;
        }
        let max = self.firearm_max_sight_range(sm, chars, w);
        let min = self.firearm_min_sight_range(chars, w);
        let dist = match hit {
            Some((dist_sq, _)) => (dist_sq as f64).sqrt() as f32,
            None => max,
        };
        let prone = target.is_some_and(|t| t.prone);
        let delay = 0.0f32.max(self.vars.aiming_delay);
        let delay_sightless = aim_delay_penalty_sightless(delay, dist);
        let delay_sights = aim_delay_penalty(delay, dist, min, max);
        let dm_sightless = distance_modifier_sightless(dist, prone);
        let dm_sights = distance_modifier(dist, min, max, prone);
        let mut combined = 0.0f32;
        let sights = (dm_sightless - delay_sightless).max(dm_sights - delay_sights);
        hit_chance += sights;
        data.aim_penalty -= sights;
        combined += self.move_penalty(chars, dist);
        if let Some(t) = target.filter(|t| t.player) {
            let p = if t.in_vehicle {
                t.vehicle_speed_kmh.abs() * 2.0
            } else if t.sprinting {
                config::SPRINTING_TO_HIT_PENALTY
            } else if t.running {
                config::RUNNING_TO_HIT_PENALTY
            } else if t.player_moving {
                config::MOVING_TO_HIT_PENALTY
            } else {
                0.0
            };
            if t.in_vehicle || t.sprinting || t.running || t.player_moving {
                hit_chance -= p;
                data.aim_penalty += p;
            }
        }
        if self.has_trait(sm, "MARKSMAN").unwrap_or(false) {
            hit_chance += config::MARKSMAN_TRAIT_TO_HIT_BONUS;
        }
        combined += self.pain_penalty(chars, cx.ctx);
        let square = match target {
            Some(t) => t.square,
            None => cx.world.square(self.square_xyz().0, self.square_xyz().1, self.square_xyz().2).map(|_| self.square_xyz()),
        };
        combined += self.weather_penalty(sm, chars, cx.ctx, Some(&*cx.world), w, square, dist);
        combined += self.moodles_penalty(sm, cx.ctx, dist);
        if cx.ctx.firearm_head_gear_effect() {
            combined += config::MAXIMUM_TO_HIT_CHANCE - config::MAXIMUM_TO_HIT_CHANCE / self.zombie.worn_items_vision_modifier;
        }
        if dist < config::POINT_BLANK_DISTANCE {
            let base = dist / config::POINT_BLANK_DISTANCE;
            combined *= base / 5.0;
            hit_chance /= base * 1.1;
        }
        data.aim_penalty += combined;
        hit_chance -= combined;
        data.hit_chance = clamp(hit_chance as i32 as f32, config::MINIMUM_TO_HIT_CHANCE, config::MAXIMUM_TO_HIT_CHANCE);
        data
    }
}

/// `PZMath.clamp(float, float, float)`.
pub(crate) fn clamp(v: f32, min: f32, max: f32) -> f32 {
    if v < min {
        return min;
    }
    if v > max {
        return max;
    }
    v
}

/// `Vector2.rotate(rad)`.
fn rotate_vec((x, y): (f32, f32), rad: f32) -> (f32, f32) {
    let c = (rad as f64).cos();
    let s = (rad as f64).sin();
    ((x as f64 * c - y as f64 * s) as f32, (x as f64 * s + y as f64 * c) as f32)
}
