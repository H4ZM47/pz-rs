//! Per-tick simulation updaters for `IsoCell.ProcessIsoObject` and
//! `IsoCell.ProcessStaticUpdaters` (`IsoFireplace`, `IsoBarbecue`,
//! `IsoGenerator`, `IsoCompost`, `IsoStove`, `IsoCarBatteryCharger`).
//!
//! `IsoRadio` / `IsoTelevision` / `IsoFeedingTrough` have their own dedicated
//! update systems; this module handles the remaining world-owned `ObjectKind`
//! simulation ticks cleanly without borrowing the cell across Lua callbacks.

use crate::objects::{IsoObject, ObjectKind};

/// Inputs from the current world frame needed by `IsoObject::update()`.
#[derive(Clone, Copy, Debug)]
pub struct ObjectUpdateContext {
    /// `GameTime.getWorldAgeHours()`.
    pub world_age_hours: f64,
    /// `SandboxOptions.GeneratorFuelConsumption` multiplier (default `1.0`).
    pub generator_fuel_mult: f32,
    /// `SandboxOptions.compost_hours()` (default `336`, i.e. 2 weeks).
    pub compost_hours: i32,
    /// Whole game seconds elapsed since the previous tick (for `IsoStove.secondsTimer`).
    pub delta_game_seconds: i32,
}

impl Default for ObjectUpdateContext {
    fn default() -> Self {
        Self {
            world_age_hours: 0.0,
            generator_fuel_mult: 1.0,
            compost_hours: 336,
            delta_game_seconds: 0,
        }
    }
}

/// Significant state transitions observed while updating one `IsoObject`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ObjectUpdateOutcome {
    /// A running `IsoGenerator` ran out of fuel or condition and shut off.
    pub generator_shut_off: bool,
    /// An `IsoCompost` gained compost progress.
    pub compost_changed: bool,
    /// An `IsoStove` timer reached zero and shut off.
    pub stove_turned_off: bool,
    /// An `IsoFireplace` or `IsoBarbecue` ran out of fuel and extinguished.
    pub hearth_extinguished: bool,
}

fn has_container_items(obj: &IsoObject) -> bool {
    if let Some(rc) = obj.runtime_containers.as_ref() {
        if let Some(main) = rc.main.as_ref() {
            return !main.borrow().items.is_empty();
        }
        return false;
    }
    obj.container
        .as_ref()
        .is_some_and(|c| !c.items.is_empty())
}

/// Run the world-simulation portion of `IsoObject.update()` for supported
/// `ObjectKind` variants.
pub fn update_iso_object(obj: &mut IsoObject, ctx: &ObjectUpdateContext) -> ObjectUpdateOutcome {
    let mut out = ObjectUpdateOutcome::default();
    let has_items = matches!(obj.kind, ObjectKind::Compost(_)) && has_container_items(obj);

    match &mut obj.kind {
        ObjectKind::Fireplace(fp) => {
            let was_lit = fp.lit;
            fp.update_fuel(ctx.world_age_hours);
            if was_lit && !fp.lit {
                out.hearth_extinguished = true;
            }
        }
        ObjectKind::Barbecue(bbq) => {
            let was_lit = bbq.lit;
            bbq.update_fuel(ctx.world_age_hours);
            if was_lit && !bbq.lit {
                out.hearth_extinguished = true;
            }
        }
        ObjectKind::Generator(g) => {
            let hour = ctx.world_age_hours.floor() as i32;
            if g.last_hour < 0 || hour < g.last_hour {
                g.last_hour = hour;
            }
            if hour > g.last_hour {
                if g.activated && g.connected {
                    let dt_hours = (hour - g.last_hour) as f32;
                    let burn = g.total_power_using.max(0.02)
                        * ctx.generator_fuel_mult.max(0.0)
                        * dt_hours;
                    g.fuel = (g.fuel - burn).max(0.0);
                    // Average ~1 condition decay every 2 running hours.
                    let wear = (hour / 2) - (g.last_hour / 2);
                    if wear > 0 {
                        g.condition = (g.condition - wear).max(0);
                    }
                    if g.fuel <= 0.0 || g.condition <= 0 {
                        g.activated = false;
                        out.generator_shut_off = true;
                    }
                }
                g.last_hour = hour;
            }
        }
        ObjectKind::Compost(c) => {
            let now = ctx.world_age_hours as f32;
            if c.last_updated < 0.0 || c.last_updated > now {
                c.last_updated = now;
            } else if now > c.last_updated {
                let dt_hours = now - c.last_updated;
                c.last_updated = now;
                if has_items && ctx.compost_hours > 0 && c.compost < 100.0 {
                    let delta = (dt_hours / ctx.compost_hours as f32) * 100.0;
                    let next = (c.compost + delta).min(100.0);
                    if next > c.compost {
                        c.compost = next;
                        out.compost_changed = true;
                    }
                }
            }
        }
        ObjectKind::Stove(s) => {
            if s.activated && s.seconds_timer > 0 && ctx.delta_game_seconds > 0 {
                s.seconds_timer -= ctx.delta_game_seconds;
                if s.seconds_timer <= 0 {
                    s.seconds_timer = -1;
                    s.activated = false;
                    out.stove_turned_off = true;
                }
            }
        }
        ObjectKind::CarBatteryCharger(ch) => {
            if ch.activated {
                ch.last_update = ctx.world_age_hours as f32;
            }
        }
        _ => {}
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::objects::{
        IsoBarbecue, IsoCompost, IsoFireplace, IsoGenerator, IsoStove, ItemContainer,
    };

    #[test]
    fn fireplace_and_barbecue_burn_fuel_and_extinguish() {
        let mut fp = IsoFireplace::default();
        fp.add_fuel(15);
        fp.turn_on();
        let mut obj = IsoObject::new(1, ObjectKind::Fireplace(fp), None);

        let r0 = update_iso_object(
            &mut obj,
            &ObjectUpdateContext {
                world_age_hours: 10.0,
                ..Default::default()
            },
        );
        assert!(!r0.hearth_extinguished);

        let r1 = update_iso_object(
            &mut obj,
            &ObjectUpdateContext {
                world_age_hours: 10.5,
                ..Default::default()
            },
        );
        assert!(r1.hearth_extinguished);
        let ObjectKind::Fireplace(fp) = &obj.kind else {
            panic!("expected fireplace");
        };
        assert!(!fp.lit);
        assert_eq!(fp.fuel_amount(), 0);

        let mut bbq = IsoBarbecue::default();
        bbq.add_fuel(10);
        bbq.turn_on();
        let mut bbq_obj = IsoObject::new(2, ObjectKind::Barbecue(bbq), None);
        update_iso_object(
            &mut bbq_obj,
            &ObjectUpdateContext {
                world_age_hours: 2.0,
                ..Default::default()
            },
        );
        let r2 = update_iso_object(
            &mut bbq_obj,
            &ObjectUpdateContext {
                world_age_hours: 2.5,
                ..Default::default()
            },
        );
        assert!(r2.hearth_extinguished);
    }

    #[test]
    fn generator_consumes_fuel_decays_condition_and_shuts_off() {
        let g = IsoGenerator {
            connected: true,
            activated: true,
            fuel: 1.0,
            condition: 100,
            last_hour: 10,
            total_power_using: 0.5,
        };
        let mut obj = IsoObject::new(3, ObjectKind::Generator(g), None);

        let r1 = update_iso_object(
            &mut obj,
            &ObjectUpdateContext {
                world_age_hours: 11.2,
                generator_fuel_mult: 1.0,
                ..Default::default()
            },
        );
        assert!(!r1.generator_shut_off);
        {
            let ObjectKind::Generator(g) = &obj.kind else {
                panic!("expected generator");
            };
            assert!((g.fuel - 0.5).abs() < 1e-4);
            assert!(g.activated);
            assert_eq!(g.last_hour, 11);
        }

        let r2 = update_iso_object(
            &mut obj,
            &ObjectUpdateContext {
                world_age_hours: 12.5,
                generator_fuel_mult: 1.0,
                ..Default::default()
            },
        );
        assert!(r2.generator_shut_off);
        let ObjectKind::Generator(g) = &obj.kind else {
            panic!("expected generator");
        };
        assert_eq!(g.fuel, 0.0);
        assert!(!g.activated);
        assert_eq!(g.condition, 99);
    }

    #[test]
    fn compost_advances_with_container_items_over_compost_hours() {
        let c = IsoCompost {
            compost: 0.0,
            last_updated: 0.0,
            health: 100,
            max_health: 100,
        };
        let mut obj = IsoObject::new(4, ObjectKind::Compost(c), None);
        let mut cont = ItemContainer::new(Some("composter"));
        cont.items.push(pz_inventory::SavedItem::Raw(vec![1, 2, 3]));
        obj.container = Some(cont);

        let r = update_iso_object(
            &mut obj,
            &ObjectUpdateContext {
                world_age_hours: 168.0,
                compost_hours: 336,
                ..Default::default()
            },
        );
        assert!(r.compost_changed);
        let ObjectKind::Compost(c) = &obj.kind else {
            panic!("expected compost");
        };
        assert!((c.compost - 50.0).abs() < 1e-3);
    }

    #[test]
    fn stove_timer_counts_down_and_turns_off() {
        let s = IsoStove {
            activated: true,
            seconds_timer: 10,
            max_temperature: 200.0,
            first_turn_on: false,
            broken: false,
        };
        let mut obj = IsoObject::new(5, ObjectKind::Stove(s), None);

        let r1 = update_iso_object(
            &mut obj,
            &ObjectUpdateContext {
                delta_game_seconds: 4,
                ..Default::default()
            },
        );
        assert!(!r1.stove_turned_off);

        let r2 = update_iso_object(
            &mut obj,
            &ObjectUpdateContext {
                delta_game_seconds: 6,
                ..Default::default()
            },
        );
        assert!(r2.stove_turned_off);
        let ObjectKind::Stove(s) = &obj.kind else {
            panic!("expected stove");
        };
        assert!(!s.activated);
        assert_eq!(s.seconds_timer, -1);
    }
}
