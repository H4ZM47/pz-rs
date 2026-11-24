//! Lua bindings for Build 42.22 household appliances and traps:
//! - `zombie.iso.objects.IsoStove`
//! - `zombie.iso.objects.IsoLightSwitch`
//! - `zombie.iso.objects.IsoCarBatteryCharger`
//! - `zombie.iso.objects.IsoTrap`

use pz_kahlua::{LuaError, LuaResult, NativeCall, Thread, Value};
use pz_world::objects::{
    DryerLogic, IsoCarBatteryCharger, IsoLightSwitch, IsoObject, IsoStove, IsoTrap, IsoTree,
    ObjectKind, WasherDryer, WasherLogic,
};

use super::*;
use crate::expose::{bool_arg, expose_class, int_arg, num_arg, str_arg};

pub const ISO_STOVE: &str = "zombie.iso.objects.IsoStove";
pub const ISO_LIGHT_SWITCH: &str = "zombie.iso.objects.IsoLightSwitch";
pub const ISO_CAR_BATTERY_CHARGER: &str = "zombie.iso.objects.IsoCarBatteryCharger";
pub const ISO_TRAP: &str = "zombie.iso.objects.IsoTrap";
pub const ISO_CLOTHING_WASHER: &str = "zombie.iso.objects.IsoClothingWasher";
pub const ISO_CLOTHING_DRYER: &str = "zombie.iso.objects.IsoClothingDryer";
pub const ISO_COMBO_WASHER_DRYER: &str = "zombie.iso.objects.IsoCombinationWasherDryer";
pub const ISO_STACKED_WASHER_DRYER: &str = "zombie.iso.objects.IsoStackedWasherDryer";
pub const ISO_TREE: &str = "zombie.iso.objects.IsoTree";
pub const ISO_JUKEBOX: &str = "zombie.iso.objects.IsoJukebox";

fn with_stove<R>(c: &NativeCall<'_>, f: impl FnOnce(&IsoObject, &IsoStove) -> R) -> LuaResult<R> {
    let obj = receiver_object(c)?;
    let body = obj.try_borrow().map_err(|_| borrowed("IsoStove"))?;
    match &body.kind {
        ObjectKind::Stove(s) => Ok(f(&body, s)),
        _ => Err(LuaError::runtime("expected IsoStove")),
    }
}

fn with_stove_mut<R>(
    c: &NativeCall<'_>,
    f: impl FnOnce(&mut IsoStove) -> R,
) -> LuaResult<R> {
    let obj = receiver_object(c)?;
    let mut body = obj.try_borrow_mut().map_err(|_| borrowed("IsoStove"))?;
    match &mut body.kind {
        ObjectKind::Stove(s) => Ok(f(s)),
        _ => Err(LuaError::runtime("expected IsoStove")),
    }
}

fn with_light<R>(
    c: &NativeCall<'_>,
    f: impl FnOnce(&IsoLightSwitch) -> R,
) -> LuaResult<R> {
    let obj = receiver_object(c)?;
    let body = obj.try_borrow().map_err(|_| borrowed("IsoLightSwitch"))?;
    match &body.kind {
        ObjectKind::LightSwitch(ls) => Ok(f(ls)),
        _ => Err(LuaError::runtime("expected IsoLightSwitch")),
    }
}

fn with_light_mut<R>(
    c: &NativeCall<'_>,
    f: impl FnOnce(&mut IsoLightSwitch) -> R,
) -> LuaResult<R> {
    let obj = receiver_object(c)?;
    let mut body = obj.try_borrow_mut().map_err(|_| borrowed("IsoLightSwitch"))?;
    match &mut body.kind {
        ObjectKind::LightSwitch(ls) => Ok(f(ls)),
        _ => Err(LuaError::runtime("expected IsoLightSwitch")),
    }
}

fn with_charger<R>(
    c: &NativeCall<'_>,
    f: impl FnOnce(&IsoCarBatteryCharger) -> R,
) -> LuaResult<R> {
    let obj = receiver_object(c)?;
    let body = obj
        .try_borrow()
        .map_err(|_| borrowed("IsoCarBatteryCharger"))?;
    match &body.kind {
        ObjectKind::CarBatteryCharger(ch) => Ok(f(ch)),
        _ => Err(LuaError::runtime("expected IsoCarBatteryCharger")),
    }
}

fn with_charger_mut<R>(
    c: &NativeCall<'_>,
    f: impl FnOnce(&mut IsoCarBatteryCharger) -> R,
) -> LuaResult<R> {
    let obj = receiver_object(c)?;
    let mut body = obj
        .try_borrow_mut()
        .map_err(|_| borrowed("IsoCarBatteryCharger"))?;
    match &mut body.kind {
        ObjectKind::CarBatteryCharger(ch) => Ok(f(ch)),
        _ => Err(LuaError::runtime("expected IsoCarBatteryCharger")),
    }
}

fn with_trap<R>(c: &NativeCall<'_>, f: impl FnOnce(&IsoTrap) -> R) -> LuaResult<R> {
    let obj = receiver_object(c)?;
    let body = obj.try_borrow().map_err(|_| borrowed("IsoTrap"))?;
    match &body.kind {
        ObjectKind::Trap(tr) => Ok(f(tr)),
        _ => Err(LuaError::runtime("expected IsoTrap")),
    }
}

fn with_trap_mut<R>(c: &NativeCall<'_>, f: impl FnOnce(&mut IsoTrap) -> R) -> LuaResult<R> {
    let obj = receiver_object(c)?;
    let mut body = obj.try_borrow_mut().map_err(|_| borrowed("IsoTrap"))?;
    match &mut body.kind {
        ObjectKind::Trap(tr) => Ok(f(tr)),
        _ => Err(LuaError::runtime("expected IsoTrap")),
    }
}

fn stove_is_microwave(obj: &IsoObject) -> bool {
    obj.container
        .as_ref()
        .is_some_and(|c| c.kind.as_deref() == Some("microwave"))
}

fn register_stove(t: &mut Thread) {
    let b = expose_class(t, ISO_STOVE, Some(ISO_OBJECT));
    b.static_fn("new", |c| {
        let mut body = IsoObject::new(0, ObjectKind::Stove(IsoStove::default()), None);
        if c.nargs() >= 2 {
            if let Ok((_, sq)) = construction::get_square(&c.arg(1)) {
                body.replace_square(Some(&sq), &mut pz_entity::RetiredComponents::new());
            }
        }
        Ok(c.push(object_value(pz_world::ObjectSlot::new(body).owner_slot())?))
    });
    b.method("getObjectName", |c| Ok(c.push(Value::str("Stove"))));
    b.method("getActivatableType", |c| Ok(c.push(Value::str("stove"))));
    b.method("Activated", |c| Ok(c.push(Value::Bool(with_stove(c, |_, s| s.activated)?))));
    b.method("setActivated", |c| {
        let v = bool_arg(c, 1);
        with_stove_mut(c, |s| {
            s.activated = v;
            if v {
                s.first_turn_on = false;
            }
        })?;
        Ok(0)
    });
    b.method("Toggle", |c| {
        with_stove_mut(c, |s| {
            if !s.broken {
                s.activated = !s.activated;
                if s.activated {
                    s.first_turn_on = false;
                }
            }
        })?;
        Ok(0)
    });
    b.method("PlayToggleSound", |_c| Ok(0));
    b.method("getTimer", |c| Ok(c.push(Value::Num(with_stove(c, |_, s| s.seconds_timer)? as f64))));
    b.method("setTimer", |c| {
        let v = int_arg(c, 1);
        with_stove_mut(c, |s| s.seconds_timer = v)?;
        Ok(0)
    });
    b.method("getMaxTemperature", |c| {
        let v = with_stove(c, |_, s| {
            if s.max_temperature > 0.0 { s.max_temperature } else { 200.0 }
        })?;
        Ok(c.push(Value::Num(v as f64)))
    });
    b.method("setMaxTemperature", |c| {
        let v = num_arg(c, 1) as f32;
        with_stove_mut(c, |s| s.max_temperature = v)?;
        Ok(0)
    });
    b.method("getCurrentTemperature", |c| {
        let v = with_stove(c, |_, s| {
            if s.activated {
                if s.max_temperature > 0.0 { s.max_temperature } else { 200.0 }
            } else {
                20.0
            }
        })?;
        Ok(c.push(Value::Num(v as f64)))
    });
    b.method("isMicrowave", |c| {
        Ok(c.push(Value::Bool(with_stove(c, |o, _| stove_is_microwave(o))?)))
    });
    b.method("isTemperatureChanging", |c| {
        Ok(c.push(Value::Bool(with_stove(c, |_, s| s.activated)?)))
    });
    b.method("isRunningFor", |c| {
        Ok(c.push(Value::Num(with_stove(c, |_, s| s.seconds_timer.max(0))? as f64)))
    });
    b.method("isBroken", |c| Ok(c.push(Value::Bool(with_stove(c, |_, s| s.broken)?))));
    b.method("setBroken", |c| {
        let v = bool_arg(c, 1);
        with_stove_mut(c, |s| {
            s.broken = v;
            if v {
                s.activated = false;
            }
        })?;
        Ok(0)
    });
    b.method("couldBePoweredByGenerator", |c| Ok(c.push(Value::Bool(true))));
    b.method("getGeneratorPowerConsumption", |c| Ok(c.push(Value::Num(0.09))));
    b.method("shouldShowOnOverlay", |c| {
        Ok(c.push(Value::Bool(with_stove(c, |_, s| s.activated)?)))
    });
    b.method("sync", |_c| Ok(0));
    b.method("syncSpriteGridObjects", |_c| Ok(0));
}

fn register_light_switch(t: &mut Thread) {
    let b = expose_class(t, ISO_LIGHT_SWITCH, Some(ISO_OBJECT));
    b.static_fn("new", |c| {
        let ls = IsoLightSwitch {
            activated: false,
            light_room: false,
            room_id: -1,
            street_light: false,
            can_be_modified: false,
            primary_r: 1.0,
            primary_g: 1.0,
            primary_b: 1.0,
            lights: Vec::new(),
            use_battery: false,
            has_battery: false,
            bulb_item: Some("Base.LightBulb".to_string()),
            power: 0.002,
            delta: 0.0,
            last_minute_stamp: -1,
            bulb_burn_minutes: -1,
        };
        let mut body = IsoObject::new(0, ObjectKind::LightSwitch(ls), None);
        if c.nargs() >= 2 {
            if let Ok((_, sq)) = construction::get_square(&c.arg(1)) {
                body.replace_square(Some(&sq), &mut pz_entity::RetiredComponents::new());
            }
        }
        Ok(c.push(object_value(pz_world::ObjectSlot::new(body).owner_slot())?))
    });
    b.method("getObjectName", |c| Ok(c.push(Value::str("LightSwitch"))));
    b.method("isActivated", |c| {
        Ok(c.push(Value::Bool(with_light(c, |ls| ls.activated)?)))
    });
    b.method("setActivated", |c| {
        let v = bool_arg(c, 1);
        with_light_mut(c, |ls| ls.activated = v)?;
        Ok(0)
    });
    b.method("setActive", |c| {
        let v = bool_arg(c, 1);
        let ok = with_light_mut(c, |ls| {
            if !v || ls.bulb_item.is_some() {
                ls.activated = v;
                true
            } else {
                false
            }
        })?;
        Ok(c.push(Value::Bool(ok)))
    });
    b.method("toggle", |c| {
        let ok = with_light_mut(c, |ls| {
            let next = !ls.activated;
            if !next || ls.bulb_item.is_some() {
                ls.activated = next;
                true
            } else {
                false
            }
        })?;
        Ok(c.push(Value::Bool(ok)))
    });
    b.method("switchLight", |c| {
        let v = bool_arg(c, 1);
        with_light_mut(c, |ls| ls.activated = v)?;
        Ok(0)
    });
    b.method("canSwitchLight", |c| {
        let ok = with_light(c, |ls| {
            if ls.bulb_item.is_none() {
                return false;
            }
            if ls.use_battery {
                ls.has_battery && ls.delta > 0.0
            } else {
                true
            }
        })?;
        Ok(c.push(Value::Bool(ok)))
    });
    b.method("getCanBeModified", |c| {
        Ok(c.push(Value::Bool(with_light(c, |ls| ls.can_be_modified)?)))
    });
    b.method("setCanBeModified", |c| {
        let v = bool_arg(c, 1);
        with_light_mut(c, |ls| ls.can_be_modified = v)?;
        Ok(0)
    });
    b.method("getPower", |c| Ok(c.push(Value::Num(with_light(c, |ls| ls.power)? as f64))));
    b.method("setPower", |c| {
        let v = num_arg(c, 1) as f32;
        with_light_mut(c, |ls| ls.power = v)?;
        Ok(0)
    });
    b.method("getDelta", |c| Ok(c.push(Value::Num(with_light(c, |ls| ls.delta)? as f64))));
    b.method("setDelta", |c| {
        let v = num_arg(c, 1) as f32;
        with_light_mut(c, |ls| ls.delta = v)?;
        Ok(0)
    });
    b.method("getUseBattery", |c| {
        Ok(c.push(Value::Bool(with_light(c, |ls| ls.use_battery)?)))
    });
    b.method("setUseBattery", |c| {
        let v = bool_arg(c, 1);
        with_light_mut(c, |ls| ls.use_battery = v)?;
        Ok(0)
    });
    b.method("setUseBatteryDirect", |c| {
        let v = bool_arg(c, 1);
        with_light_mut(c, |ls| ls.use_battery = v)?;
        Ok(0)
    });
    b.method("getHasBattery", |c| {
        Ok(c.push(Value::Bool(with_light(c, |ls| ls.has_battery)?)))
    });
    b.method("setHasBattery", |c| {
        let v = bool_arg(c, 1);
        with_light_mut(c, |ls| ls.has_battery = v)?;
        Ok(0)
    });
    b.method("setHasBatteryRaw", |c| {
        let v = bool_arg(c, 1);
        with_light_mut(c, |ls| ls.has_battery = v)?;
        Ok(0)
    });
    b.method("hasLightBulb", |c| {
        Ok(c.push(Value::Bool(with_light(c, |ls| ls.bulb_item.is_some())?)))
    });
    b.method("getBulbItem", |c| {
        let v = with_light(c, |ls| ls.bulb_item.clone())?;
        Ok(c.push(v.map(|s| Value::str(s.as_str())).unwrap_or(Value::Nil)))
    });
    b.method("setBulbItemRaw", |c| {
        let v = str_arg(c, 1).map(|s| s.to_string());
        with_light_mut(c, |ls| ls.bulb_item = v)?;
        Ok(0)
    });
    b.method("getPrimaryR", |c| Ok(c.push(Value::Num(with_light(c, |ls| ls.primary_r)? as f64))));
    b.method("getPrimaryG", |c| Ok(c.push(Value::Num(with_light(c, |ls| ls.primary_g)? as f64))));
    b.method("getPrimaryB", |c| Ok(c.push(Value::Num(with_light(c, |ls| ls.primary_b)? as f64))));
    b.method("setPrimaryR", |c| {
        let v = num_arg(c, 1) as f32;
        with_light_mut(c, |ls| ls.primary_r = v)?;
        Ok(0)
    });
    b.method("setPrimaryG", |c| {
        let v = num_arg(c, 1) as f32;
        with_light_mut(c, |ls| ls.primary_g = v)?;
        Ok(0)
    });
    b.method("setPrimaryB", |c| {
        let v = num_arg(c, 1) as f32;
        with_light_mut(c, |ls| ls.primary_b = v)?;
        Ok(0)
    });
    b.method("couldBePoweredByGenerator", |c| {
        Ok(c.push(Value::Bool(with_light(c, |ls| !ls.use_battery)?)))
    });
    b.method("getGeneratorPowerConsumption", |c| Ok(c.push(Value::Num(0.002))));
    b.method("shouldShowOnOverlay", |c| {
        Ok(c.push(Value::Bool(with_light(c, |ls| ls.activated)?)))
    });
}

fn register_car_battery_charger(t: &mut Thread) {
    let b = expose_class(t, ISO_CAR_BATTERY_CHARGER, Some(ISO_OBJECT));
    b.static_fn("new", |c| {
        let ch = IsoCarBatteryCharger {
            item: None,
            battery: None,
            activated: false,
            last_update: -1.0,
            charge_rate: 0.16666667,
        };
        let mut body = IsoObject::new(0, ObjectKind::CarBatteryCharger(Box::new(ch)), None);
        if c.nargs() >= 3 {
            if let Ok((_, sq)) = construction::get_square(&c.arg(2)) {
                body.replace_square(Some(&sq), &mut pz_entity::RetiredComponents::new());
            }
        }
        Ok(c.push(object_value(pz_world::ObjectSlot::new(body).owner_slot())?))
    });
    b.method("getObjectName", |c| {
        Ok(c.push(Value::str("IsoCarBatteryCharger")))
    });
    b.method("isActivated", |c| {
        Ok(c.push(Value::Bool(with_charger(c, |ch| ch.activated)?)))
    });
    b.method("setActivated", |c| {
        let v = bool_arg(c, 1);
        with_charger_mut(c, |ch| ch.activated = v)?;
        Ok(0)
    });
    b.method("getChargeRate", |c| {
        Ok(c.push(Value::Num(with_charger(c, |ch| ch.charge_rate)? as f64)))
    });
    b.method("setChargeRate", |c| {
        let v = num_arg(c, 1) as f32;
        with_charger_mut(c, |ch| ch.charge_rate = v)?;
        Ok(0)
    });
    b.method("couldBePoweredByGenerator", |c| Ok(c.push(Value::Bool(true))));
    b.method("getGeneratorPowerConsumption", |c| Ok(c.push(Value::Num(0.05))));
}

fn register_trap(t: &mut Thread) {
    let b = expose_class(t, ISO_TRAP, Some(ISO_OBJECT));
    b.static_fn("new", |c| {
        let tr = IsoTrap {
            sensor_range: 0,
            fire_starting_chance: 0,
            fire_starting_energy: 0,
            fire_range: 0,
            explosion_power: 0,
            explosion_range: 0,
            explosion_duration: 0,
            explosion_start_time: 0.0,
            smoke_range: 0,
            noise_range: 0,
            noise_duration: 0,
            noise_start_time: 0.0,
            extra_damage: 0.0,
            remote_control_id: -1,
            timer_before_explosion: 0,
            count_down_sound: String::new(),
            explosion_sound: String::new(),
            weapon: None,
        };
        let mut body = IsoObject::new(0, ObjectKind::Trap(Box::new(tr)), None);
        if c.nargs() >= 3 {
            let sq_arg = if c.nargs() >= 4 { c.arg(3) } else { c.arg(2) };
            if let Ok((_, sq)) = construction::get_square(&sq_arg) {
                body.replace_square(Some(&sq), &mut pz_entity::RetiredComponents::new());
            }
        }
        Ok(c.push(object_value(pz_world::ObjectSlot::new(body).owner_slot())?))
    });
    b.method("getObjectName", |c| Ok(c.push(Value::str("IsoTrap"))));
    b.method("getTimerBeforeExplosion", |c| {
        Ok(c.push(Value::Num(with_trap(c, |tr| tr.timer_before_explosion)? as f64)))
    });
    b.method("setTimerBeforeExplosion", |c| {
        let v = int_arg(c, 1);
        with_trap_mut(c, |tr| tr.timer_before_explosion = v)?;
        Ok(0)
    });
    b.method("getSensorRange", |c| {
        Ok(c.push(Value::Num(with_trap(c, |tr| tr.sensor_range)? as f64)))
    });
    b.method("setSensorRange", |c| {
        let v = int_arg(c, 1);
        with_trap_mut(c, |tr| tr.sensor_range = v)?;
        Ok(0)
    });
    b.method("getFireRange", |c| {
        Ok(c.push(Value::Num(with_trap(c, |tr| tr.fire_range)? as f64)))
    });
    b.method("setFireRange", |c| {
        let v = int_arg(c, 1);
        with_trap_mut(c, |tr| tr.fire_range = v)?;
        Ok(0)
    });
    b.method("getFireStartingEnergy", |c| {
        Ok(c.push(Value::Num(with_trap(c, |tr| tr.fire_starting_energy)? as f64)))
    });
    b.method("setFireStartingEnergy", |c| {
        let v = int_arg(c, 1);
        with_trap_mut(c, |tr| tr.fire_starting_energy = v)?;
        Ok(0)
    });
    b.method("getFireStartingChance", |c| {
        Ok(c.push(Value::Num(with_trap(c, |tr| tr.fire_starting_chance)? as f64)))
    });
    b.method("setFireStartingChance", |c| {
        let v = int_arg(c, 1);
        with_trap_mut(c, |tr| tr.fire_starting_chance = v)?;
        Ok(0)
    });
    b.method("getExplosionPower", |c| {
        Ok(c.push(Value::Num(with_trap(c, |tr| tr.explosion_power)? as f64)))
    });
    b.method("setExplosionPower", |c| {
        let v = int_arg(c, 1);
        with_trap_mut(c, |tr| tr.explosion_power = v)?;
        Ok(0)
    });
    b.method("getExplosionRange", |c| {
        Ok(c.push(Value::Num(with_trap(c, |tr| tr.explosion_range)? as f64)))
    });
    b.method("setExplosionRange", |c| {
        let v = int_arg(c, 1);
        with_trap_mut(c, |tr| tr.explosion_range = v)?;
        Ok(0)
    });
    b.method("getExplosionDuration", |c| {
        Ok(c.push(Value::Num(with_trap(c, |tr| tr.explosion_duration)? as f64)))
    });
    b.method("setExplosionDuration", |c| {
        let v = int_arg(c, 1);
        with_trap_mut(c, |tr| tr.explosion_duration = v)?;
        Ok(0)
    });
    b.method("getSmokeRange", |c| {
        Ok(c.push(Value::Num(with_trap(c, |tr| tr.smoke_range)? as f64)))
    });
    b.method("setSmokeRange", |c| {
        let v = int_arg(c, 1);
        with_trap_mut(c, |tr| tr.smoke_range = v)?;
        Ok(0)
    });
    b.method("getNoiseRange", |c| {
        Ok(c.push(Value::Num(with_trap(c, |tr| tr.noise_range)? as f64)))
    });
    b.method("setNoiseRange", |c| {
        let v = int_arg(c, 1);
        with_trap_mut(c, |tr| tr.noise_range = v)?;
        Ok(0)
    });
    b.method("getNoiseDuration", |c| {
        Ok(c.push(Value::Num(with_trap(c, |tr| tr.noise_duration)? as f64)))
    });
    b.method("setNoiseDuration", |c| {
        let v = int_arg(c, 1);
        with_trap_mut(c, |tr| tr.noise_duration = v)?;
        Ok(0)
    });
    b.method("getExtraDamage", |c| {
        Ok(c.push(Value::Num(with_trap(c, |tr| tr.extra_damage)? as f64)))
    });
    b.method("setExtraDamage", |c| {
        let v = num_arg(c, 1) as f32;
        with_trap_mut(c, |tr| tr.extra_damage = v)?;
        Ok(0)
    });
    b.method("getRemoteControlID", |c| {
        Ok(c.push(Value::Num(with_trap(c, |tr| tr.remote_control_id)? as f64)))
    });
    b.method("setRemoteControlID", |c| {
        let v = int_arg(c, 1);
        with_trap_mut(c, |tr| tr.remote_control_id = v)?;
        Ok(0)
    });
    b.method("getCountDownSound", |c| {
        let s = with_trap(c, |tr| tr.count_down_sound.clone())?;
        Ok(c.push(Value::str(s.as_str())))
    });
    b.method("setCountDownSound", |c| {
        let s = str_arg(c, 1).map(|v| v.to_string()).unwrap_or_default();
        with_trap_mut(c, |tr| tr.count_down_sound = s)?;
        Ok(0)
    });
    b.method("getExplosionSound", |c| {
        let s = with_trap(c, |tr| tr.explosion_sound.clone())?;
        Ok(c.push(Value::str(s.as_str())))
    });
    b.method("setExplosionSound", |c| {
        let s = str_arg(c, 1).map(|v| v.to_string()).unwrap_or_default();
        with_trap_mut(c, |tr| tr.explosion_sound = s)?;
        Ok(0)
    });
    b.method("isInstantExplosion", |c| {
        Ok(c.push(Value::Bool(with_trap(c, |tr| tr.timer_before_explosion <= 0)?)))
    });
    b.method("setInstantExplosion", |c| {
        let v = bool_arg(c, 1);
        with_trap_mut(c, |tr| {
            if v {
                tr.timer_before_explosion = 0;
            }
        })?;
        Ok(0)
    });
    b.method("isExploding", |c| {
        Ok(c.push(Value::Bool(with_trap(c, |tr| tr.explosion_start_time > 0.0)?)))
    });
    b.method("triggerExplosion", |c| {
        with_trap_mut(c, |tr| {
            tr.timer_before_explosion = 0;
            tr.explosion_start_time = 1.0;
        })?;
        Ok(0)
    });
}

fn register_laundry(t: &mut Thread) {
    // IsoClothingWasher
    let w = expose_class(t, ISO_CLOTHING_WASHER, Some(ISO_OBJECT));
    w.static_fn("new", |c| {
        let mut body = IsoObject::new(0, ObjectKind::ClothingWasher(WasherLogic::default()), None);
        if c.nargs() >= 2 {
            if let Ok((_, sq)) = construction::get_square(&c.arg(1)) {
                body.replace_square(Some(&sq), &mut pz_entity::RetiredComponents::new());
            }
        }
        Ok(c.push(object_value(pz_world::ObjectSlot::new(body).owner_slot())?))
    });
    w.method("getObjectName", |c| Ok(c.push(Value::str("ClothingWasher"))));
    w.method("isActivated", |c| {
        let obj = receiver_object(c)?;
        let body = obj.try_borrow().map_err(|_| borrowed("IsoClothingWasher"))?;
        let v = matches!(&body.kind, ObjectKind::ClothingWasher(l) if l.activated);
        Ok(c.push(Value::Bool(v)))
    });
    w.method("setActivated", |c| {
        let v = bool_arg(c, 1);
        let obj = receiver_object(c)?;
        let mut body = obj.try_borrow_mut().map_err(|_| borrowed("IsoClothingWasher"))?;
        if let ObjectKind::ClothingWasher(l) = &mut body.kind {
            l.activated = v;
        }
        Ok(0)
    });
    w.method("couldBePoweredByGenerator", |c| Ok(c.push(Value::Bool(true))));
    w.method("getGeneratorPowerConsumption", |c| Ok(c.push(Value::Num(0.09))));

    // IsoClothingDryer
    let d = expose_class(t, ISO_CLOTHING_DRYER, Some(ISO_OBJECT));
    d.static_fn("new", |c| {
        let mut body = IsoObject::new(0, ObjectKind::ClothingDryer(DryerLogic::default()), None);
        if c.nargs() >= 2 {
            if let Ok((_, sq)) = construction::get_square(&c.arg(1)) {
                body.replace_square(Some(&sq), &mut pz_entity::RetiredComponents::new());
            }
        }
        Ok(c.push(object_value(pz_world::ObjectSlot::new(body).owner_slot())?))
    });
    d.method("getObjectName", |c| Ok(c.push(Value::str("ClothingDryer"))));
    d.method("isActivated", |c| {
        let obj = receiver_object(c)?;
        let body = obj.try_borrow().map_err(|_| borrowed("IsoClothingDryer"))?;
        let v = matches!(&body.kind, ObjectKind::ClothingDryer(l) if l.activated);
        Ok(c.push(Value::Bool(v)))
    });
    d.method("setActivated", |c| {
        let v = bool_arg(c, 1);
        let obj = receiver_object(c)?;
        let mut body = obj.try_borrow_mut().map_err(|_| borrowed("IsoClothingDryer"))?;
        if let ObjectKind::ClothingDryer(l) = &mut body.kind {
            l.activated = v;
        }
        Ok(0)
    });
    d.method("couldBePoweredByGenerator", |c| Ok(c.push(Value::Bool(true))));
    d.method("getGeneratorPowerConsumption", |c| Ok(c.push(Value::Num(0.09))));

    // IsoCombinationWasherDryer
    let cb = expose_class(t, ISO_COMBO_WASHER_DRYER, Some(ISO_OBJECT));
    cb.static_fn("new", |c| {
        let mut body = IsoObject::new(
            0,
            ObjectKind::CombinationWasherDryer(WasherDryer::default()),
            None,
        );
        if c.nargs() >= 2 {
            if let Ok((_, sq)) = construction::get_square(&c.arg(1)) {
                body.replace_square(Some(&sq), &mut pz_entity::RetiredComponents::new());
            }
        }
        Ok(c.push(object_value(pz_world::ObjectSlot::new(body).owner_slot())?))
    });
    cb.method("getObjectName", |c| Ok(c.push(Value::str("CombinationWasherDryer"))));
    cb.method("isModeWasher", |c| {
        let obj = receiver_object(c)?;
        let body = obj.try_borrow().map_err(|_| borrowed("IsoCombinationWasherDryer"))?;
        let v = matches!(&body.kind, ObjectKind::CombinationWasherDryer(wd) if !wd.dryer_mode);
        Ok(c.push(Value::Bool(v)))
    });
    cb.method("isModeDryer", |c| {
        let obj = receiver_object(c)?;
        let body = obj.try_borrow().map_err(|_| borrowed("IsoCombinationWasherDryer"))?;
        let v = matches!(&body.kind, ObjectKind::CombinationWasherDryer(wd) if wd.dryer_mode);
        Ok(c.push(Value::Bool(v)))
    });
    cb.method("setModeWasher", |c| {
        let obj = receiver_object(c)?;
        let mut body = obj.try_borrow_mut().map_err(|_| borrowed("IsoCombinationWasherDryer"))?;
        if let ObjectKind::CombinationWasherDryer(wd) = &mut body.kind {
            wd.dryer_mode = false;
            wd.dryer.activated = false;
        }
        Ok(0)
    });
    cb.method("setModeDryer", |c| {
        let obj = receiver_object(c)?;
        let mut body = obj.try_borrow_mut().map_err(|_| borrowed("IsoCombinationWasherDryer"))?;
        if let ObjectKind::CombinationWasherDryer(wd) = &mut body.kind {
            wd.dryer_mode = true;
            wd.washer.activated = false;
        }
        Ok(0)
    });
    cb.method("isActivated", |c| {
        let obj = receiver_object(c)?;
        let body = obj.try_borrow().map_err(|_| borrowed("IsoCombinationWasherDryer"))?;
        let v = match &body.kind {
            ObjectKind::CombinationWasherDryer(wd) => {
                if wd.dryer_mode { wd.dryer.activated } else { wd.washer.activated }
            }
            _ => false,
        };
        Ok(c.push(Value::Bool(v)))
    });
    cb.method("setActivated", |c| {
        let v = bool_arg(c, 1);
        let obj = receiver_object(c)?;
        let mut body = obj.try_borrow_mut().map_err(|_| borrowed("IsoCombinationWasherDryer"))?;
        if let ObjectKind::CombinationWasherDryer(wd) = &mut body.kind {
            if wd.dryer_mode {
                wd.dryer.activated = v;
            } else {
                wd.washer.activated = v;
            }
        }
        Ok(0)
    });
    cb.method("couldBePoweredByGenerator", |c| Ok(c.push(Value::Bool(true))));
    cb.method("getGeneratorPowerConsumption", |c| Ok(c.push(Value::Num(0.09))));

    // IsoStackedWasherDryer
    let st = expose_class(t, ISO_STACKED_WASHER_DRYER, Some(ISO_OBJECT));
    st.static_fn("new", |c| {
        let mut body = IsoObject::new(
            0,
            ObjectKind::StackedWasherDryer(WasherDryer::default()),
            None,
        );
        if c.nargs() >= 2 {
            if let Ok((_, sq)) = construction::get_square(&c.arg(1)) {
                body.replace_square(Some(&sq), &mut pz_entity::RetiredComponents::new());
            }
        }
        Ok(c.push(object_value(pz_world::ObjectSlot::new(body).owner_slot())?))
    });
    st.method("getObjectName", |c| Ok(c.push(Value::str("StackedWasherDryer"))));
    st.method("isWasherActivated", |c| {
        let obj = receiver_object(c)?;
        let body = obj.try_borrow().map_err(|_| borrowed("IsoStackedWasherDryer"))?;
        let v = matches!(&body.kind, ObjectKind::StackedWasherDryer(wd) if wd.washer.activated);
        Ok(c.push(Value::Bool(v)))
    });
    st.method("setWasherActivated", |c| {
        let v = bool_arg(c, 1);
        let obj = receiver_object(c)?;
        let mut body = obj.try_borrow_mut().map_err(|_| borrowed("IsoStackedWasherDryer"))?;
        if let ObjectKind::StackedWasherDryer(wd) = &mut body.kind {
            wd.washer.activated = v;
        }
        Ok(0)
    });
    st.method("isDryerActivated", |c| {
        let obj = receiver_object(c)?;
        let body = obj.try_borrow().map_err(|_| borrowed("IsoStackedWasherDryer"))?;
        let v = matches!(&body.kind, ObjectKind::StackedWasherDryer(wd) if wd.dryer.activated);
        Ok(c.push(Value::Bool(v)))
    });
    st.method("setDryerActivated", |c| {
        let v = bool_arg(c, 1);
        let obj = receiver_object(c)?;
        let mut body = obj.try_borrow_mut().map_err(|_| borrowed("IsoStackedWasherDryer"))?;
        if let ObjectKind::StackedWasherDryer(wd) = &mut body.kind {
            wd.dryer.activated = v;
        }
        Ok(0)
    });
    st.method("couldBePoweredByGenerator", |c| Ok(c.push(Value::Bool(true))));
    st.method("getGeneratorPowerConsumption", |c| Ok(c.push(Value::Num(0.09))));
}

fn register_tree_and_jukebox(t: &mut Thread) {
    let tr = expose_class(t, ISO_TREE, Some(ISO_OBJECT));
    let make_tree = |c: &mut NativeCall<'_>| -> LuaResult<usize> {
        let tree = IsoTree {
            size: 4,
            log_yield: 3,
            damage: 160,
        };
        let mut body = IsoObject::new(0, ObjectKind::Tree(tree), None);
        if c.nargs() >= 2 {
            if let Ok((_, sq)) = construction::get_square(&c.arg(0)) {
                body.replace_square(Some(&sq), &mut pz_entity::RetiredComponents::new());
            }
            if let Some(sp) = str_arg(c, 1) {
                body.sprite_name = Some(pz_world::objects::SpriteName::Str(sp.to_string()));
            }
        }
        Ok(c.push(object_value(pz_world::ObjectSlot::new(body).owner_slot())?))
    };
    tr.static_fn("new", make_tree);
    tr.static_fn("getNew", |c| {
        let body = IsoObject::new(
            0,
            ObjectKind::Tree(IsoTree {
                size: 4,
                log_yield: 3,
                damage: 160,
            }),
            None,
        );
        Ok(c.push(object_value(pz_world::ObjectSlot::new(body).owner_slot())?))
    });
    tr.static_fn("setChopTreeCursorLocation", |_c| Ok(0));
    tr.static_fn("checkChopTreeIndicators", |_c| Ok(0));
    tr.static_fn("renderChopTreeIndicators", |_c| Ok(0));
    tr.method("getObjectName", |c| Ok(c.push(Value::str("Tree"))));
    tr.method("initTree", |c| {
        let obj = receiver_object(c)?;
        let mut body = obj.try_borrow_mut().map_err(|_| borrowed("IsoTree"))?;
        if let ObjectKind::Tree(t) = &mut body.kind {
            t.size = t.size.clamp(1, 8);
            t.damage = ((t.log_yield - 1) * 80).max(40);
        }
        Ok(0)
    });
    tr.method("getSize", |c| {
        let obj = receiver_object(c)?;
        let body = obj.try_borrow().map_err(|_| borrowed("IsoTree"))?;
        let v = match &body.kind {
            ObjectKind::Tree(t) => t.size,
            _ => 0,
        };
        Ok(c.push(Value::Num(v as f64)))
    });
    tr.method("getLogYield", |c| {
        let obj = receiver_object(c)?;
        let body = obj.try_borrow().map_err(|_| borrowed("IsoTree"))?;
        let v = match &body.kind {
            ObjectKind::Tree(t) => t.log_yield,
            _ => 0,
        };
        Ok(c.push(Value::Num(v as f64)))
    });
    tr.method("getHealth", |c| {
        let obj = receiver_object(c)?;
        let body = obj.try_borrow().map_err(|_| borrowed("IsoTree"))?;
        let v = match &body.kind {
            ObjectKind::Tree(t) => t.damage,
            _ => 0,
        };
        Ok(c.push(Value::Num(v as f64)))
    });
    tr.method("setHealth", |c| {
        let v = int_arg(c, 1);
        let obj = receiver_object(c)?;
        let mut body = obj.try_borrow_mut().map_err(|_| borrowed("IsoTree"))?;
        if let ObjectKind::Tree(t) = &mut body.kind {
            t.damage = v;
        }
        Ok(0)
    });
    tr.method("getMaxHealth", |c| {
        let obj = receiver_object(c)?;
        let body = obj.try_borrow().map_err(|_| borrowed("IsoTree"))?;
        let v = match &body.kind {
            ObjectKind::Tree(t) => ((t.log_yield - 1) * 80).max(40),
            _ => 0,
        };
        Ok(c.push(Value::Num(v as f64)))
    });
    tr.method("Damage", |c| {
        let amount = num_arg(c, 1) as i32;
        let obj = receiver_object(c)?;
        let mut body = obj.try_borrow_mut().map_err(|_| borrowed("IsoTree"))?;
        if let ObjectKind::Tree(t) = &mut body.kind {
            t.damage = (t.damage - amount).max(0);
        }
        Ok(0)
    });
    tr.method("getSlowFactor", |c| {
        let obj = receiver_object(c)?;
        let body = obj.try_borrow().map_err(|_| borrowed("IsoTree"))?;
        let slow = match &body.kind {
            ObjectKind::Tree(t) => match t.size {
                1 | 2 => 0.8,
                3 | 4 => 0.5,
                _ => 0.3,
            },
            _ => 1.0,
        };
        Ok(c.push(Value::Num(slow)))
    });
    tr.method("checkChopTreeIndicator", |_c| Ok(0));
    tr.method("dropWood", |_c| Ok(0));
    tr.method("toppleTree", |_c| Ok(0));
    tr.method("reset", |_c| Ok(0));

    let jb = expose_class(t, ISO_JUKEBOX, Some(ISO_OBJECT));
    jb.static_fn("new", |c| {
        let mut body = IsoObject::new(0, ObjectKind::Jukebox, None);
        if c.nargs() >= 2 {
            if let Ok((_, sq)) = construction::get_square(&c.arg(1)) {
                body.replace_square(Some(&sq), &mut pz_entity::RetiredComponents::new());
            }
        }
        Ok(c.push(object_value(pz_world::ObjectSlot::new(body).owner_slot())?))
    });
    jb.method("getObjectName", |c| Ok(c.push(Value::str("Jukebox"))));
    jb.method("SetPlaying", |_c| Ok(0));
}

pub(super) fn register(t: &mut Thread) {
    register_stove(t);
    register_light_switch(t);
    register_car_battery_charger(t);
    register_trap(t);
    register_laundry(t);
    register_tree_and_jukebox(t);
}

#[cfg(test)]
mod tests {
    use super::*;
    use pz_kahlua::TableRef;

    #[test]
    fn appliances_and_traps_round_trip_from_lua() {
        pz_world::object_slot::install_object_observer(Some(crate::component_gc::observe_object));
        let mut t = Thread::new(TableRef::new());
        pz_kahlua::stdlib::install(&mut t, None).unwrap();
        crate::bind_world_object::register_classes(&mut t);

        let src = r#"
            local function assert(v, msg) if not v then error(msg or "assertion failed") end end

            local stove = IsoStove.new(nil)
            assert(stove:getObjectName() == "Stove")
            assert(stove:Activated() == false)
            stove:setMaxTemperature(250)
            stove:setTimer(45)
            stove:Toggle()
            assert(stove:Activated() == true and stove:getCurrentTemperature() == 250 and stove:getTimer() == 45)
            stove:setBroken(true)
            assert(stove:isBroken() == true and stove:Activated() == false)

            local ls = IsoLightSwitch.new(nil)
            assert(ls:getObjectName() == "LightSwitch" and ls:hasLightBulb() == true)
            assert(ls:canSwitchLight() == true)
            assert(ls:toggle() == true and ls:isActivated() == true)
            ls:setUseBattery(true)
            ls:setHasBattery(true)
            ls:setDelta(0.75)
            assert(ls:getUseBattery() == true and ls:getHasBattery() == true and ls:getDelta() == 0.75)
            ls:setPrimaryR(0.8)
            assert(math.abs(ls:getPrimaryR() - 0.8) < 0.001)
            ls:setBulbItemRaw(nil)
            assert(ls:hasLightBulb() == false and ls:canSwitchLight() == false)

            local ch = IsoCarBatteryCharger.new(nil)
            assert(ch:getObjectName() == "IsoCarBatteryCharger" and ch:isActivated() == false)
            ch:setActivated(true)
            ch:setChargeRate(0.25)
            assert(ch:isActivated() == true and ch:getChargeRate() == 0.25)

            local tr = IsoTrap.new(nil)
            assert(tr:getObjectName() == "IsoTrap")
            tr:setTimerBeforeExplosion(10)
            tr:setExplosionPower(90)
            tr:setExplosionRange(5)
            tr:setSmokeRange(7)
            tr:setNoiseRange(20)
            tr:setRemoteControlID(42)
            tr:setExplosionSound("PipeBombExplode")
            assert(tr:getTimerBeforeExplosion() == 10 and tr:getExplosionPower() == 90)
            assert(tr:getExplosionRange() == 5 and tr:getSmokeRange() == 7 and tr:getNoiseRange() == 20)
            assert(tr:getRemoteControlID() == 42 and tr:getExplosionSound() == "PipeBombExplode")
            tr:triggerExplosion(false)
            assert(tr:isExploding() == true and tr:isInstantExplosion() == true)

            local w = IsoClothingWasher.new(nil)
            w:setActivated(true)
            assert(w:getObjectName() == "ClothingWasher" and w:isActivated() == true)

            local d = IsoClothingDryer.new(nil)
            d:setActivated(true)
            assert(d:getObjectName() == "ClothingDryer" and d:isActivated() == true)

            local cb = IsoCombinationWasherDryer.new(nil)
            assert(cb:isModeWasher() == true)
            cb:setActivated(true)
            assert(cb:isActivated() == true)
            cb:setModeDryer()
            assert(cb:isModeDryer() == true and cb:isActivated() == false)
            cb:setActivated(true)
            assert(cb:isActivated() == true)

            local st = IsoStackedWasherDryer.new(nil)
            st:setWasherActivated(true)
            st:setDryerActivated(true)
            assert(st:isWasherActivated() == true and st:isDryerActivated() == true)

            local tree = IsoTree.getNew()
            assert(tree:getObjectName() == "Tree" and tree:getSize() == 4 and tree:getLogYield() == 3)
            assert(tree:getHealth() == 160 and tree:getMaxHealth() == 160)
            tree:Damage(40)
            assert(tree:getHealth() == 120)
            assert(tree:getSlowFactor() == 0.5)

            local jb = IsoJukebox.new(nil)
            assert(jb:getObjectName() == "Jukebox")
            jb:SetPlaying(true)
            return true
        "#;
        let cl = pz_kahlua::stdlib::load_string(src, Some("appliances_test"), t.globals().clone())
            .unwrap();
        let res = t
            .pcall(&cl, &[])
            .unwrap_or_else(|e| panic!("lua error: {e:?}"));
        assert!(matches!(res.first(), Some(Value::Bool(true))));
    }
}
