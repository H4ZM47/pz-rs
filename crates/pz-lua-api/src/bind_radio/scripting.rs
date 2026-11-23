//! Lua bindings for `zombie.radio.ChannelCategory`,
//! `zombie.radio.scripting.{RadioScriptManager, RadioChannel,
//! DynamicRadioChannel, RadioBroadCast, RadioLine}` and the world-object
//! `IsoWaveSignal` / `IsoRadio` / `IsoTelevision` device getters.
//!
//! `ISDynamicRadio.lua` and `ISWeatherChannel.lua` call these during
//! `OnLoadRadioScripts` and `EveryHours` to register and update the
//! Automated Emergency Broadcast System and modded radio channels.

use std::any::Any;
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};

use pz_inventory::device::DeviceData;
use pz_kahlua::value::LuaObject;
use pz_kahlua::{LuaResult, NativeCall, Thread, Value};
use pz_radio::{ChannelCategory, RadioBroadCast, RadioChannel, RadioLine};
use pz_world::{IsoObjectRef, ObjectKind};

use crate::bind_world_object::{object_from_value, object_value, ISO_OBJECT};
use crate::expose::{bool_arg, enum_value, expose_class, expose_enum, int_arg, num_arg, str_arg, with_this, JavaEnum};

use super::{list, opt_str, with_radio, DeviceOwner, LuaDeviceData, WorldDevice};

pub const CHANNEL_CATEGORY_CLASS: &str = "zombie.radio.ChannelCategory";
pub const RADIO_SCRIPT_MANAGER_CLASS: &str = "zombie.radio.scripting.RadioScriptManager";
pub const RADIO_CHANNEL_CLASS: &str = "zombie.radio.scripting.RadioChannel";
pub const DYNAMIC_RADIO_CHANNEL_CLASS: &str = "zombie.radio.scripting.DynamicRadioChannel";
pub const RADIO_BROADCAST_CLASS: &str = "zombie.radio.scripting.RadioBroadCast";
pub const RADIO_LINE_CLASS: &str = "zombie.radio.scripting.RadioLine";
pub const ISO_WAVE_SIGNAL_CLASS: &str = "zombie.iso.objects.IsoWaveSignal";
pub const ISO_RADIO_CLASS: &str = "zombie.iso.objects.IsoRadio";
pub const ISO_TELEVISION_CLASS: &str = "zombie.iso.objects.IsoTelevision";

struct LuaRadioScriptManager;
impl LuaObject for LuaRadioScriptManager {
    fn class_name(&self) -> &str {
        RADIO_SCRIPT_MANAGER_CLASS
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

thread_local! {
    static SCRIPT_MANAGER_OBJ: Value = Value::Object(Rc::new(LuaRadioScriptManager));
}

/// The singleton `RadioScriptManager` userdata passed to `OnLoadRadioScripts`
/// and returned by `ZomboidRadio:getScriptManager()`.
pub fn script_manager_value() -> Value {
    SCRIPT_MANAGER_OBJ.with(|v| v.clone())
}

/// `zombie.radio.scripting.RadioLine` userdata.
pub struct LuaRadioLine(pub RefCell<RadioLine>);
impl LuaObject for LuaRadioLine {
    fn class_name(&self) -> &str {
        RADIO_LINE_CLASS
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// `zombie.radio.scripting.RadioBroadCast` userdata.
pub struct LuaRadioBroadCast(pub RefCell<RadioBroadCast>);
impl LuaObject for LuaRadioBroadCast {
    fn class_name(&self) -> &str {
        RADIO_BROADCAST_CLASS
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// Backing storage of a Lua `RadioChannel` / `DynamicRadioChannel`.
///
/// Before `RadioScriptManager:AddChannel`, the channel lives inside the Lua
/// userdata so scripts can configure it (e.g. `setAirCounterMultiplier`); once
/// registered, the handle forwards reads and writes to the live channel in
/// `ZomboidRadio`'s `RadioScriptManager`.
enum ChannelSlot {
    Pending(Option<RadioChannel>),
    Registered { frequency: i32 },
}

/// `zombie.radio.scripting.RadioChannel` / `DynamicRadioChannel` userdata.
pub struct LuaRadioChannel {
    slot: RefCell<ChannelSlot>,
    dynamic: bool,
}

impl LuaRadioChannel {
    fn pending(channel: RadioChannel) -> Value {
        let dynamic = channel.is_dynamic();
        Value::Object(Rc::new(LuaRadioChannel {
            slot: RefCell::new(ChannelSlot::Pending(Some(channel))),
            dynamic,
        }))
    }

    fn registered(frequency: i32, dynamic: bool) -> Value {
        Value::Object(Rc::new(LuaRadioChannel {
            slot: RefCell::new(ChannelSlot::Registered { frequency }),
            dynamic,
        }))
    }

    fn with_channel<R>(&self, f: impl FnOnce(&RadioChannel) -> R) -> Option<R> {
        let slot = self.slot.borrow();
        match &*slot {
            ChannelSlot::Pending(Some(c)) => Some(f(c)),
            ChannelSlot::Pending(None) => None,
            ChannelSlot::Registered { frequency } => {
                let freq = *frequency;
                with_radio(|r| r.script_manager().and_then(|m| m.channels().get(&freq)).map(f))
                    .flatten()
            }
        }
    }

    fn with_channel_mut<R>(&self, f: impl FnOnce(&mut RadioChannel) -> R) -> Option<R> {
        let mut slot = self.slot.borrow_mut();
        match &mut *slot {
            ChannelSlot::Pending(Some(c)) => Some(f(c)),
            ChannelSlot::Pending(None) => None,
            ChannelSlot::Registered { frequency } => {
                let freq = *frequency;
                with_radio(|r| r.script_manager_mut().and_then(|m| m.channel_mut(freq)).map(f))
                    .flatten()
            }
        }
    }
}

impl LuaObject for LuaRadioChannel {
    fn class_name(&self) -> &str {
        if self.dynamic {
            DYNAMIC_RADIO_CHANNEL_CLASS
        } else {
            RADIO_CHANNEL_CLASS
        }
    }
    fn super_classes(&self) -> &[&'static str] {
        if self.dynamic {
            &[RADIO_CHANNEL_CLASS]
        } else {
            &[]
        }
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

fn category_arg(v: &Value) -> ChannelCategory {
    if let Value::Object(o) = v {
        if let Some(e) = o.as_any().downcast_ref::<JavaEnum>() {
            if e.class == CHANNEL_CATEGORY_CLASS {
                return ChannelCategory::ALL
                    .get(e.ordinal as usize)
                    .copied()
                    .unwrap_or(ChannelCategory::Undefined);
            }
        }
    }
    if let Value::Str(s) = v {
        return ChannelCategory::value_of(s).unwrap_or(ChannelCategory::Undefined);
    }
    ChannelCategory::Undefined
}

fn category_value(cat: ChannelCategory) -> Value {
    enum_value(CHANNEL_CATEGORY_CLASS, cat.name())
}

fn generated_guid() -> String {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    format!("pz-radio-chan-{n:016x}")
}

fn channel_ctor(c: &mut NativeCall<'_>, dynamic: bool) -> LuaResult<usize> {
    let name = str_arg(c, 0).map(|s| s.to_string()).unwrap_or_default();
    let freq = int_arg(c, 1);
    let cat = category_arg(&c.arg(2));
    let guid = str_arg(c, 3)
        .map(|s| s.to_string())
        .unwrap_or_else(generated_guid);
    let channel = if dynamic {
        RadioChannel::new_dynamic(&name, freq, cat, &guid)
    } else {
        RadioChannel::new(&name, freq, cat, &guid)
    };
    Ok(c.push(LuaRadioChannel::pending(channel)))
}

fn kind_device_data_mut(kind: &mut ObjectKind) -> Option<&mut DeviceData> {
    match kind {
        ObjectKind::Radio(d) | ObjectKind::Television(d) => Some(d),
        _ => None,
    }
}

/// [`WorldDevice`] adapter over an actual world `IsoRadio` / `IsoTelevision`.
pub struct IsoObjectWorldDevice(pub IsoObjectRef);

impl WorldDevice for IsoObjectWorldDevice {
    fn with_device(&self, f: &mut dyn FnMut(&mut DeviceData)) {
        if let Ok(mut obj) = self.0.try_borrow_mut() {
            if let Some(d) = kind_device_data_mut(&mut obj.kind) {
                f(d);
            }
        }
    }

    fn lua_value(&self) -> Value {
        object_value(&self.0).unwrap_or(Value::Nil)
    }

    fn world_object(&self) -> Option<IsoObjectRef> {
        Some(self.0.clone())
    }
}

fn wave_signal_object(c: &NativeCall<'_>) -> Option<IsoObjectRef> {
    let this = c.arg(0);
    object_from_value(&this).map(|o| o.object.clone())
}

pub(super) fn register(thread: &mut Thread) {
    let names: Vec<&str> = ChannelCategory::ALL.iter().map(|c| c.name()).collect();
    expose_enum(thread, CHANNEL_CATEGORY_CLASS, &names);

    // `zombie.radio.scripting.RadioLine`
    let b = expose_class(thread, RADIO_LINE_CLASS, None);
    b.ctor(|c| {
        let text = str_arg(c, 0).map(|s| s.to_string());
        let r = num_arg(c, 1) as f32;
        let g = num_arg(c, 2) as f32;
        let bl = num_arg(c, 3) as f32;
        let fx = str_arg(c, 4).map(|s| s.to_string());
        let line = RadioLine::new(text.as_deref(), r, g, bl, fx.as_deref());
        Ok(c.push(Value::Object(Rc::new(LuaRadioLine(RefCell::new(line))))))
    });
    b.method("getText", |c| {
        let s = with_this::<LuaRadioLine, _>(c, |l| l.0.borrow().text().to_string())?;
        Ok(c.push(Value::str(&s)))
    });
    b.method("getEffectsString", |c| {
        let s = with_this::<LuaRadioLine, _>(c, |l| l.0.borrow().effects().to_string())?;
        Ok(c.push(Value::str(&s)))
    });
    b.method("getR", |c| {
        Ok(c.push(with_this::<LuaRadioLine, _>(c, |l| l.0.borrow().r() as f64)?))
    });
    b.method("getG", |c| {
        Ok(c.push(with_this::<LuaRadioLine, _>(c, |l| l.0.borrow().g() as f64)?))
    });
    b.method("getB", |c| {
        Ok(c.push(with_this::<LuaRadioLine, _>(c, |l| l.0.borrow().b() as f64)?))
    });
    b.method("getAirTime", |c| {
        Ok(c.push(with_this::<LuaRadioLine, _>(c, |l| l.0.borrow().air_time() as f64)?))
    });
    b.method("setAirTime", |c| {
        let f = num_arg(c, 1) as f32;
        with_this::<LuaRadioLine, _>(c, |l| l.0.borrow_mut().set_air_time(f))?;
        Ok(0)
    });
    b.method("isCustomAirTime", |c| {
        Ok(c.push(with_this::<LuaRadioLine, _>(c, |l| {
            l.0.borrow().is_custom_air_time()
        })?))
    });

    // `zombie.radio.scripting.RadioBroadCast`
    let b = expose_class(thread, RADIO_BROADCAST_CLASS, None);
    b.ctor(|c| {
        let id = str_arg(c, 0).map(|s| s.to_string()).unwrap_or_default();
        let start = int_arg(c, 1);
        let end = int_arg(c, 2);
        let bc = RadioBroadCast::new(&id, start, end);
        Ok(c.push(Value::Object(Rc::new(LuaRadioBroadCast(RefCell::new(bc))))))
    });
    b.method("AddRadioLine", |c| {
        let line = match c.arg(1) {
            Value::Object(o) => o
                .as_any()
                .downcast_ref::<LuaRadioLine>()
                .map(|l| l.0.borrow().clone()),
            _ => None,
        };
        if let Some(line) = line {
            with_this::<LuaRadioBroadCast, _>(c, |bc| bc.0.borrow_mut().add_line(line))?;
        }
        Ok(0)
    });
    b.method("getID", |c| {
        let id = with_this::<LuaRadioBroadCast, _>(c, |bc| bc.0.borrow().id().to_string())?;
        Ok(c.push(Value::str(&id)))
    });
    b.method("getStartStamp", |c| {
        Ok(c.push(with_this::<LuaRadioBroadCast, _>(c, |bc| {
            bc.0.borrow().start_stamp() as f64
        })?))
    });
    b.method("getEndStamp", |c| {
        Ok(c.push(with_this::<LuaRadioBroadCast, _>(c, |bc| {
            bc.0.borrow().end_stamp() as f64
        })?))
    });
    b.method("getCurrentLineNumber", |c| {
        Ok(c.push(with_this::<LuaRadioBroadCast, _>(c, |bc| {
            bc.0.borrow().current_line_number() as f64
        })?))
    });
    b.method("setCurrentLineNumber", |c| {
        let n = int_arg(c, 1);
        with_this::<LuaRadioBroadCast, _>(c, |bc| bc.0.borrow_mut().set_current_line_number(n))?;
        Ok(0)
    });
    b.method("getLines", |c| {
        let lines = with_this::<LuaRadioBroadCast, _>(c, |bc| {
            bc.0.borrow()
                .lines()
                .iter()
                .cloned()
                .map(|l| Value::Object(Rc::new(LuaRadioLine(RefCell::new(l)))))
                .collect::<Vec<_>>()
        })?;
        Ok(c.push(list(lines)))
    });

    // `zombie.radio.scripting.RadioChannel`
    let b = expose_class(thread, RADIO_CHANNEL_CLASS, None);
    b.ctor(|c| channel_ctor(c, false));
    b.method("GetName", |c| {
        let s = with_this::<LuaRadioChannel, _>(c, |ch| {
            ch.with_channel(|r| r.name().to_string())
        })?;
        Ok(c.push(opt_str(s)))
    });
    b.method("GetFrequency", |c| {
        let f = with_this::<LuaRadioChannel, _>(c, |ch| ch.with_channel(|r| r.frequency()))?;
        Ok(c.push(f.unwrap_or(0) as f64))
    });
    b.method("getGUID", |c| {
        let s = with_this::<LuaRadioChannel, _>(c, |ch| {
            ch.with_channel(|r| r.guid().to_string())
        })?;
        Ok(c.push(opt_str(s)))
    });
    b.method("IsTv", |c| {
        let v = with_this::<LuaRadioChannel, _>(c, |ch| ch.with_channel(|r| r.is_tv()))?;
        Ok(c.push(v.unwrap_or(false)))
    });
    b.method("GetCategory", |c| {
        let v = with_this::<LuaRadioChannel, _>(c, |ch| ch.with_channel(|r| r.category()))?;
        Ok(c.push(category_value(v.unwrap_or(ChannelCategory::Undefined))))
    });
    b.method("GetPlayerIsListening", |c| {
        let v = with_this::<LuaRadioChannel, _>(c, |ch| {
            ch.with_channel(|r| r.player_is_listening())
        })?;
        Ok(c.push(v.unwrap_or(false)))
    });
    b.method("getLastAiredLine", |c| {
        let s = with_this::<LuaRadioChannel, _>(c, |ch| {
            ch.with_channel(|r| r.last_aired_line().to_string())
        })?;
        Ok(c.push(opt_str(s)))
    });
    b.method("getLastBroadcastID", |c| {
        let s = with_this::<LuaRadioChannel, _>(c, |ch| {
            ch.with_channel(|r| r.last_broadcast_id().map(str::to_string))
        })?
        .flatten();
        Ok(c.push(opt_str(s)))
    });
    b.method("getAirCounterMultiplier", |c| {
        let m = with_this::<LuaRadioChannel, _>(c, |ch| {
            ch.with_channel(|r| r.air_counter_multiplier())
        })?;
        Ok(c.push(m.unwrap_or(1.0) as f64))
    });
    b.method("setAirCounterMultiplier", |c| {
        let m = num_arg(c, 1) as f32;
        with_this::<LuaRadioChannel, _>(c, |ch| {
            ch.with_channel_mut(|r| r.set_air_counter_multiplier(m));
        })?;
        Ok(0)
    });
    b.method("getCurrentScriptLoop", |c| {
        let v = with_this::<LuaRadioChannel, _>(c, |ch| {
            ch.with_channel(|r| r.current_script_loop())
        })?;
        Ok(c.push(v.unwrap_or(0) as f64))
    });
    b.method("getCurrentScriptMaxLoops", |c| {
        let v = with_this::<LuaRadioChannel, _>(c, |ch| {
            ch.with_channel(|r| r.current_script_max_loops())
        })?;
        Ok(c.push(v.unwrap_or(0) as f64))
    });
    b.method("isTimeSynced", |c| {
        let v = with_this::<LuaRadioChannel, _>(c, |ch| ch.with_channel(|r| r.is_time_synced()))?;
        Ok(c.push(v.unwrap_or(false)))
    });
    b.method("setTimeSynced", |c| {
        let v = bool_arg(c, 1);
        with_this::<LuaRadioChannel, _>(c, |ch| {
            ch.with_channel_mut(|r| r.set_time_synced(v));
        })?;
        Ok(0)
    });
    b.method("isVanilla", |c| {
        let v = with_this::<LuaRadioChannel, _>(c, |ch| ch.with_channel(|r| r.is_vanilla()))?;
        Ok(c.push(v.unwrap_or(true)))
    });
    b.method("getLouisvilleObfuscate", |c| {
        let v = with_this::<LuaRadioChannel, _>(c, |ch| {
            ch.with_channel(|r| r.louisville_obfuscate())
        })?;
        Ok(c.push(v.unwrap_or(false)))
    });
    b.method("setLouisvilleObfuscate", |c| {
        let v = bool_arg(c, 1);
        with_this::<LuaRadioChannel, _>(c, |ch| {
            ch.with_channel_mut(|r| r.set_louisville_obfuscate(v));
        })?;
        Ok(0)
    });
    b.method("setAiringBroadcast", |c| {
        let bc = match c.arg(1) {
            Value::Nil => None,
            Value::Object(o) => o
                .as_any()
                .downcast_ref::<LuaRadioBroadCast>()
                .map(|b| b.0.borrow().clone()),
            _ => None,
        };
        with_this::<LuaRadioChannel, _>(c, |ch| {
            let slot = ch.slot.borrow();
            if let ChannelSlot::Registered { frequency } = &*slot {
                let freq = *frequency;
                with_radio(|r| {
                    if let Some(m) = r.script_manager_mut() {
                        let id = bc.map(|b| m.broadcasts.add(b));
                        if let Some(chan) = m.channel_mut(freq) {
                            chan.set_airing_broadcast(id);
                        }
                    }
                });
            }
        })?;
        Ok(0)
    });

    // `zombie.radio.scripting.DynamicRadioChannel` (subclass of `RadioChannel`)
    let b = expose_class(thread, DYNAMIC_RADIO_CHANNEL_CLASS, Some(RADIO_CHANNEL_CLASS));
    b.ctor(|c| channel_ctor(c, true));

    // `zombie.radio.scripting.RadioScriptManager`
    let b = expose_class(thread, RADIO_SCRIPT_MANAGER_CLASS, None);
    b.method("AddChannel", |c| {
        let overwrite = bool_arg(c, 2);
        let added = match c.arg(1) {
            Value::Object(o) => {
                if let Some(ch) = o.as_any().downcast_ref::<LuaRadioChannel>() {
                    let pending = {
                        let mut slot = ch.slot.borrow_mut();
                        match &mut *slot {
                            ChannelSlot::Pending(opt) => opt.take(),
                            ChannelSlot::Registered { .. } => None,
                        }
                    };
                    if let Some(channel) = pending {
                        let freq = channel.frequency();
                        let ok = with_radio(|r| r.add_channel(channel, overwrite)).unwrap_or(false);
                        if ok {
                            *ch.slot.borrow_mut() = ChannelSlot::Registered { frequency: freq };
                        }
                        ok
                    } else {
                        false
                    }
                } else {
                    false
                }
            }
            _ => false,
        };
        let _ = added;
        Ok(0)
    });
    b.method("RemoveChannel", |c| {
        let freq = int_arg(c, 1);
        with_radio(|r| r.remove_channel(freq));
        Ok(0)
    });
    b.method("getRadioChannel", |c| {
        let uuid = str_arg(c, 1).map(|s| s.to_string()).unwrap_or_default();
        let found = with_radio(|r| {
            r.script_manager()
                .and_then(|m| m.radio_channel(&uuid))
                .map(|ch| (ch.frequency(), ch.is_dynamic()))
        })
        .flatten();
        let v = found.map_or(Value::Nil, |(f, dyn_)| LuaRadioChannel::registered(f, dyn_));
        Ok(c.push(v))
    });
    b.method("getCurrentTimeStamp", |c| {
        let ts = with_radio(|r| r.script_manager().map(|m| m.current_time_stamp()))
            .flatten()
            .unwrap_or(0);
        Ok(c.push(ts as f64))
    });
    b.method("getChannelsList", |c| {
        let items: Vec<Value> = with_radio(|r| {
            r.script_manager()
                .map(|m| {
                    m.channels()
                        .values()
                        .map(|ch| LuaRadioChannel::registered(ch.frequency(), ch.is_dynamic()))
                        .collect()
                })
                .unwrap_or_default()
        })
        .unwrap_or_default();
        Ok(c.push(list(items)))
    });

    // `zombie.iso.objects.IsoWaveSignal` / `IsoRadio` / `IsoTelevision`
    let b = expose_class(thread, ISO_WAVE_SIGNAL_CLASS, Some(ISO_OBJECT));
    b.method("getDeviceData", |c| {
        let Some(obj) = wave_signal_object(c) else {
            return Ok(c.push(Value::Nil));
        };
        let has = obj
            .try_borrow()
            .ok()
            .is_some_and(|o| o.kind.device_data().is_some());
        if !has {
            return Ok(c.push(Value::Nil));
        }
        let dev: Rc<dyn WorldDevice> = Rc::new(IsoObjectWorldDevice(obj));
        Ok(c.push(Value::Object(Rc::new(LuaDeviceData(DeviceOwner::World(dev))))))
    });
    b.method("HasPlayerInRange", |c| {
        let v = wave_signal_object(c)
            .and_then(|o| {
                o.try_borrow()
                    .ok()
                    .and_then(|o| o.kind.device_data().map(|d| d.runtime.wave.has_player_in_range))
            })
            .unwrap_or(false);
        Ok(c.push(v))
    });
    b.method("getDelta", |c| {
        let v = wave_signal_object(c)
            .and_then(|o| {
                o.try_borrow()
                    .ok()
                    .and_then(|o| o.kind.device_data().map(|d| d.power_delta))
            })
            .unwrap_or(0.0);
        Ok(c.push(v as f64))
    });
    b.method("setDelta", |c| {
        let v = num_arg(c, 1) as f32;
        if let Some(o) = wave_signal_object(c) {
            if let Ok(mut o) = o.try_borrow_mut() {
                if let Some(d) = kind_device_data_mut(&mut o.kind) {
                    d.set_power(v);
                }
            }
        }
        Ok(0)
    });
    b.method("getTalkerType", |c| Ok(c.push(Value::str("device"))));

    expose_class(thread, ISO_RADIO_CLASS, Some(ISO_WAVE_SIGNAL_CLASS));
    expose_class(thread, ISO_TELEVISION_CLASS, Some(ISO_WAVE_SIGNAL_CLASS));
}

#[cfg(test)]
mod tests {
    use super::*;
    use pz_core::game_time::GameTimeEvent;
    use pz_kahlua::{TableRef, Thread};
    use pz_radio::ZomboidRadio;
    use std::cell::Cell;

    #[test]
    fn dynamic_radio_channel_and_broadcast_scripting_works() {
        let mut t = Thread::new(TableRef::new());
        pz_kahlua::stdlib::install(&mut t, None).unwrap();
        crate::bind_java_util::register(&mut t);
        crate::bind_world_object::register_classes(&mut t);
        crate::bind_vehicles::register(&mut t);
        super::super::register(&mut t);

        let mut radio = ZomboidRadio::new(9);
        // Ensure script manager exists before AddChannel (as ZomboidRadio.init does).
        let _ = radio.init(
            -1,
            &pz_radio::RadioSource::default(),
            &|k: &str| k.to_string(),
            &mut EmptyRadioWorld,
            &mut |_| 0,
        );
        let radio = Rc::new(RefCell::new(radio));
        super::super::install_radio(Some(radio.clone()));

        let f = pz_kahlua::stdlib::load_string(
            r#"
            local sm = getZomboidRadio():getScriptManager()
            if sm == nil then return "no_sm" end
            local ch = DynamicRadioChannel.new("Automated Emergency Broadcast System", 93200, ChannelCategory.Emergency, "EMRG-711984")
            ch:setAirCounterMultiplier(1.5)
            sm:AddChannel(ch, false)
            local found = sm:getRadioChannel("EMRG-711984")
            if found == nil then return "not_found" end
            local bc = RadioBroadCast.new("GEN-123456", -1, -1)
            local line = RadioLine.new("All clear.", 0.2, 0.8, 0.3)
            bc:AddRadioLine(line)
            ch:setAiringBroadcast(bc)
            return found:GetName(), found:GetFrequency(), found:getAirCounterMultiplier()
            "#,
            Some("radio_scripting_test"),
            t.globals().clone(),
        )
        .unwrap();
        let res = t.call(&f, &[]).unwrap();
        let strs: Vec<String> = res.iter().map(pz_kahlua::exec::java_value_of).collect();
        assert_eq!(
            strs,
            vec!["Automated Emergency Broadcast System", "93200.0", "1.5"]
        );
        {
            let r = radio.borrow();
            let ch = r.script_manager().unwrap().radio_channel("EMRG-711984").unwrap();
            assert!(ch.is_dynamic());
            assert!(ch.airing_broadcast().is_some());
            assert_eq!(r.channel_name(93200), Some("Automated Emergency Broadcast System"));
        }
        super::super::install_radio(None);
    }

    #[test]
    fn climate_fire_events_dispatches_radio_update_scripts() {
        let seen = Rc::new(Cell::new((-1, -1)));
        let seen_hook = seen.clone();
        super::super::install_update_scripts_hook(Some(Rc::new(move |h, m| {
            seen_hook.set((h, m));
        })));
        let mut t = Thread::new(TableRef::new());
        let mut ctx = crate::bind_climate::LuaClimateContext::new();
        ctx.clock_events
            .push(GameTimeEvent::RadioUpdateScripts { hour: 14, minutes: 30 });
        crate::bind_climate::fire_events(&mut t, ctx);
        assert_eq!(seen.get(), (14, 30));
        super::super::install_update_scripts_hook(None);
    }

    struct EmptyRadioWorld;
    impl pz_radio::RadioWorld for EmptyRadioWorld {
        fn hour(&self) -> i32 {
            9
        }
        fn nights_survived(&self) -> i32 {
            0
        }
        fn multiplier(&self) -> f32 {
            1.0
        }
        fn time_since_apo(&self) -> i32 {
            1
        }
        fn core_game_mode(&self) -> String {
            "Apocalypse".into()
        }
        fn current_save_dir(&self) -> Option<std::path::PathBuf> {
            None
        }
    }
}
