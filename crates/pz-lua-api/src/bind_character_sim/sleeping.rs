//! `SleepingEvent`: `getSleepingEvent():setPlayerFallAsleep` from the sleep
//! menu, `SleepingEvent.update` from `GameTime.update`, and the UI and game
//! part of `SleepingEvent.wakeUp`. The character part is
//! [`pz_characters::character::sleeping_event`].

use std::cell::RefCell;
use std::rc::Rc;

use pz_characters::character::sleeping_event::SleepingUpdate;
use pz_characters::character::SimEvent;
use pz_kahlua::{LuaError, LuaResult, Thread, Value};

use super::{CallContext, CharacterObject, LuaGameCharacterRef, CONTEXT};
use crate::bind_world_object::unsupported;
use crate::expose::{bool_arg, int_arg};

const SLEEPING_EVENT: &str = "zombie.ai.sadisticAIDirector.SleepingEvent";

/// `SleepingEvent.instance`.
struct SleepingEvent;

impl pz_kahlua::value::LuaObject for SleepingEvent {
    fn class_name(&self) -> &str {
        SLEEPING_EVENT
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

pub(super) fn register_globals(thread: &mut Thread) {
    let event = crate::expose::expose_class(thread, SLEEPING_EVENT, Some("java.lang.Object"));
    // setPlayerFallAsleep(player, sleepFor) is
    // setPlayerFallAsleep(player, sleepFor, false, false).
    event.method("setPlayerFallAsleep", |c| {
        let player = c.arg(1);
        let chr = chr_of_value(&player)?;
        let sleep_for = int_arg(c, 2);
        let (force_intruders, force_nightmare) = if c.nargs() >= 5 { (bool_arg(c, 3), bool_arg(c, 4)) } else { (false, false) };
        let rain_exposed = is_raining() && is_exposed_to_precipitation(c.thread, &chr, &player)?;
        let sm = crate::bind_registry::shared_script_manager();
        let intruders = super::with_ctx(c, &player, |ctx| {
            chr.borrow_mut().set_player_fall_asleep(sleep_for, force_intruders, force_nightmare, rain_exposed, &sm, ctx)
        })?;
        if intruders {
            return Err(unsupported("SleepingEvent.setPlayerFallAsleep: the zombie intruders (forceIntruders)"));
        }
        Ok(0)
    });
    crate::expose::global(thread, "getSleepingEvent", |c| Ok(c.push(Value::Object(Rc::new(SleepingEvent)))));
    // save(doCharacter): `GameWindow.save(doCharacter)`, whose exceptions
    // `ExceptionLogger.logException` logs. `doCharacter` is not read on the
    // way to the files (`IsoCell.save` ignores it).
    crate::expose::global(thread, "save", |_| {
        crate::bind_game_window::request_save();
        Ok(0)
    });
}

/// The `IsoPlayer` argument.
fn chr_of_value(v: &Value) -> LuaResult<LuaGameCharacterRef> {
    let chr = match v {
        Value::Object(o) => o.as_any().downcast_ref::<CharacterObject>().map(|o| o.0.clone()),
        _ => None,
    };
    chr.ok_or_else(|| crate::bind_java_util::class_cast(v, super::ISO_PLAYER_CLASS))
}

/// `ClimateManager.getInstance().isRaining()`.
fn is_raining() -> bool {
    super::with_sim_context(|ctx| ctx.is_raining())
}

/// `SleepingEvent.isExposedToPrecipitation(chr)`.
fn is_exposed_to_precipitation(t: &mut Thread, chr: &LuaGameCharacterRef, value: &Value) -> LuaResult<bool> {
    let Some((x, y, z)) = chr.borrow().path.current_square else {
        return Ok(false);
    };
    let square = crate::bind_world::square_value(x, y, z);
    if call(t, &square, "isInARoom", &[])?.truthy() || crate::bind_world::square_queries::square_have_roof(x, y, z)? {
        return Ok(false);
    }
    let bed = chr.borrow().bed.clone();
    if let Some(bed) = bed {
        let tent = call(t, &bed, "isTent", &[])?.truthy()
            || call(t, &bed, "getName", &[])?.as_str().is_some_and(|name| name == "Tent");
        if tent {
            return Ok(false);
        }
    }
    let vehicle = call(t, value, "getVehicle", &[])?;
    if !vehicle.is_nil() {
        let seat = call(t, &vehicle, "getSeat", &[value.clone()])?;
        if call(t, &vehicle, "hasRoof", &[seat])?.truthy() {
            return Ok(false);
        }
    }
    Ok(true)
}

/// `obj.name(args)` through the object's Lua binding.
fn call(t: &mut Thread, obj: &Value, name: &str, args: &[Value]) -> LuaResult<Value> {
    let f = t.table_get(obj, &Value::str(name))?;
    if f.is_nil() {
        return Err(LuaError::runtime(format!(
            "SleepingEvent: {}.{name} is not bound",
            pz_kahlua::stdlib::java_class_name(obj)
        )));
    }
    let mut all = vec![obj.clone()];
    all.extend_from_slice(args);
    t.call1(&f, &all)
}

/// `SleepingEvent.instance.update(player)` from `GameTime.update`, for an
/// asleep player after its `asleepTime` grew. An error is logged as the
/// game loop would.
pub(crate) fn update_asleep_player(t: &mut Thread, chr: &LuaGameCharacterRef) {
    if let Err(e) = try_update_asleep_player(t, chr) {
        pz_core::debug::log_exception(&pz_core::debug::LogThrowable::new(format!("SleepingEvent.update: {e}")), None);
    }
}

fn try_update_asleep_player(t: &mut Thread, chr: &LuaGameCharacterRef) -> LuaResult<()> {
    let raining = is_raining();
    let value = super::character_value(chr);
    let mut exposed = None;
    if raining {
        exposed = Some(is_exposed_to_precipitation(t, chr, &value)?);
    }
    let chars = crate::bind_characters::shared_characters();
    let chars = chars.try_borrow().map_err(|_| LuaError::runtime("Characters is in use"))?;
    let ctx = CallContext {
        base: CONTEXT.with(|c| c.borrow().clone()),
        events: RefCell::default(),
        game_time: crate::bind_climate::shared_game_time(),
    };
    let r = chr
        .borrow_mut()
        .sleeping_event_update(|| Ok(exposed.unwrap_or(false)), raining, &chars, &ctx)
        .map_err(super::java_error)?;
    for e in ctx.events.into_inner() {
        if let SimEvent::SleepingEventWakeUp { fast_wakeup } = e {
            wake_up_ui(chr, fast_wakeup);
        }
        // SimEvent::AddSound: WorldSoundManager, TODO(sound).
    }
    if r == SleepingUpdate::ZombieIntruders {
        return Err(unsupported("SleepingEvent.update: spawnZombieIntruders"));
    }
    Ok(())
}

/// The UI and game part of `SleepingEvent.wakeUp(chr, remote)`
/// ([`SimEvent::SleepingEventWakeUp`]), run after the character part woke
/// `chr` up: a local player's fade-in; then, not on a client and with all
/// players asleep before `chr` woke, game speed 1 and
/// `GameWindow.save(true)`, which the engine runs after this update.
pub fn wake_up_ui(chr: &LuaGameCharacterRef, fast_wakeup: bool) {
    let (index, local) = {
        let c = chr.borrow();
        (c.player.player_index, super::with_sim_context(|ctx| c.is_local_player(ctx)))
    };
    if local {
        let seconds = if fast_wakeup { 0.5 } else { 2.0 } as f64 as i32;
        pz_ui::with_ui(|m| {
            let fps = m.lock_fps;
            if let Some(f) = m.player_fade_info.get_mut(index.max(0) as usize) {
                f.fade_before_ui = true;
                f.fade_in(seconds, fps);
            }
        });
    }
    if super::with_sim_context(|ctx| ctx.is_client()) || !all_players_asleep_before(chr) {
        return;
    }
    crate::hud::set_game_speed(1);
    crate::bind_game_window::request_save();
}

/// `IsoPlayer.allPlayersAsleep()` as it was before `chr`, then asleep,
/// woke up.
fn all_players_asleep_before(chr: &LuaGameCharacterRef) -> bool {
    let (mut living, mut asleep) = (0, 0);
    for i in 0..crate::bind_players::num_players().max(0) as usize {
        let Some(p) = crate::bind_players::player(i) else { continue };
        let p_ref = p.borrow();
        if p_ref.is_dead() {
            continue;
        }
        living += 1;
        if Rc::ptr_eq(&p, chr) || p_ref.is_asleep() {
            asleep += 1;
        }
    }
    living > 0 && living == asleep
}

#[cfg(test)]
mod tests;
