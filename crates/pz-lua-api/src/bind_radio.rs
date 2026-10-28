//! `zombie.radio` exposed to Lua over [`pz_radio`]: `getZomboidRadio()`,
//! `getRadioAPI()`, `ZomboidRadio`, `RadioAPI`, `RecordedMedia`,
//! `MediaData`, `MediaData.MediaLineData`, `DevicePresets` and
//! `PresetEntry`, plus the `DeviceData` methods that run the device
//! simulation (`pz_inventory::device_sim`) on top of bind_vehicles'
//! `DeviceData` getters and setters.
//!
//! The engine installs the world's radio ([`install_radio`], with
//! [`radio_api_created`] once `ZomboidRadio.Init` made the `RadioAPI`),
//! the translator ([`install_text`]) and the world hooks
//! ([`install_hooks`]: device environments, the remote-control test, the
//! media lines a player knows). While the `OnLoadRadioScripts` and
//! `OnInitRecordedMedia` events run, the radio or media being loaded is
//! lent to Lua with [`with_scoped_radio`] / [`with_scoped_media`].

use crate::bind_vehicles::{DeviceOwner, LuaDeviceData, DEVICE_DATA_CLASS};
use crate::expose::{bool_arg, expose_class, int_arg, num_arg, str_arg, with_this};
use pz_inventory::device::{
    DeviceData, DevicePresetsRef, PresetEntry, PresetEntryRef, PresetListRef,
};
use pz_inventory::device_sim::{
    DeviceEnv, DeviceEvent, IsoObjectPos, MainsPower, MediaLine, NetMode, ParentKind,
};
use pz_kahlua::value::LuaObject;
use pz_kahlua::{LuaError, LuaResult, NativeCall, TableRef, Thread, Value};
use pz_radio::{RecordedMedia, ZomboidRadio};
use std::any::Any;
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::{Rc, Weak};

pub const ZOMBOID_RADIO_CLASS: &str = "zombie.radio.ZomboidRadio";
pub const RADIO_API_CLASS: &str = "zombie.radio.RadioAPI";
pub const RECORDED_MEDIA_CLASS: &str = "zombie.radio.media.RecordedMedia";
pub const MEDIA_DATA_CLASS: &str = "zombie.radio.media.MediaData";
pub const MEDIA_LINE_CLASS: &str = "zombie.radio.media.MediaData$MediaLineData";
pub const DEVICE_PRESETS_CLASS: &str = "zombie.radio.devices.DevicePresets";
pub const PRESET_ENTRY_CLASS: &str = "zombie.radio.devices.PresetEntry";

/// A world object's `DeviceData` (`IsoRadio` / `IsoTelevision`).
pub trait WorldDevice {
    /// Runs `f` on the object's device data (not at all when it is gone).
    fn with_device(&self, f: &mut dyn FnMut(&mut DeviceData));
    /// The object's Lua value (`getParent()`).
    fn lua_value(&self) -> Value {
        Value::Nil
    }
}

/// `Translator.getText` and `getTextMediaEN` for the radio classes.
pub trait LuaRadioText {
    fn get_text(&self, key: &str) -> String;
    fn get_text_media_en(&self, key: &str) -> String {
        self.get_text(key)
    }
}

/// The world side of the Lua radio classes.
pub trait RadioLuaHooks {
    /// The device environment of a `DeviceData` (time, RNG, power,
    /// sounds). Existing simulation calls use a minimal single-player
    /// fallback. Battery/media insertion requires a provided environment
    /// at its network and final transmission phases.
    fn device_env(&self, _owner: &DeviceOwner) -> Option<Box<dyn DeviceEnv>> {
        None
    }
    /// `canPlayerRemoteInteract(character)`: the character sees the
    /// television and holds a TV remote.
    fn can_player_remote_interact(&self, _owner: &DeviceOwner, _character: &Value) -> bool {
        false
    }
    /// `player.isKnownMediaLine(guid)`.
    fn knows_media_line(&self, _player: &Value, _guid: &str) -> bool {
        false
    }
}

struct NoHooks;
impl RadioLuaHooks for NoHooks {}

struct KeyText;
impl LuaRadioText for KeyText {
    fn get_text(&self, key: &str) -> String {
        key.to_string()
    }
}

/// Canonical userdata for the actual allocations; keys never encode a
/// device slot or list index. Live userdata pins the corresponding owner.
type EntryCache = HashMap<usize, Weak<LuaPresetEntry>>;

thread_local! {
    static RADIO: RefCell<Option<Rc<RefCell<ZomboidRadio>>>> = const { RefCell::new(None) };
    static API_CREATED: Cell<bool> = const { Cell::new(false) };
    static SCOPED_RADIO: RefCell<Option<ZomboidRadio>> = const { RefCell::new(None) };
    static SCOPED_MEDIA: RefCell<Option<RecordedMedia>> = const { RefCell::new(None) };
    static TEXT: RefCell<Rc<dyn LuaRadioText>> = RefCell::new(Rc::new(KeyText));
    static HOOKS: RefCell<Rc<dyn RadioLuaHooks>> = RefCell::new(Rc::new(NoHooks));
    static PRESET_ENTRIES: RefCell<EntryCache> = RefCell::new(HashMap::new());
    static PRESET_OWNERS: RefCell<HashMap<usize, Weak<LuaDevicePresets>>> = RefCell::new(HashMap::new());
    static PRESET_LISTS: RefCell<HashMap<usize, Weak<crate::bind_java_util::JavaList>>> = RefCell::new(HashMap::new());
}

/// Makes `radio` the `ZomboidRadio.getInstance()` Lua sees (`None`:
/// `hasInstance()` is false).
pub fn install_radio(radio: Option<Rc<RefCell<ZomboidRadio>>>) {
    RADIO.with(|r| *r.borrow_mut() = radio);
}

/// `RadioAPI.getInstance()` ran (in `ZomboidRadio.Init`); the instance is
/// never dropped.
pub fn radio_api_created() {
    API_CREATED.with(|c| c.set(true));
}

pub fn install_text(text: Rc<dyn LuaRadioText>) {
    TEXT.with(|t| *t.borrow_mut() = text);
}

pub fn install_hooks(hooks: Rc<dyn RadioLuaHooks>) {
    HOOKS.with(|h| *h.borrow_mut() = hooks);
}

fn text() -> Rc<dyn LuaRadioText> {
    TEXT.with(|t| t.borrow().clone())
}

fn hooks() -> Rc<dyn RadioLuaHooks> {
    HOOKS.with(|h| h.borrow().clone())
}

/// Lends `radio` to Lua while `f` runs (the `OnLoadRadioScripts` event,
/// when the radio is not yet the installed one).
pub fn with_scoped_radio<R>(radio: &mut ZomboidRadio, f: impl FnOnce() -> R) -> R {
    let lent = std::mem::replace(radio, ZomboidRadio::new(0));
    let prev = SCOPED_RADIO.with(|s| s.borrow_mut().replace(lent));
    let r = f();
    *radio = SCOPED_RADIO
        .with(|s| std::mem::replace(&mut *s.borrow_mut(), prev))
        .expect("scoped radio");
    r
}

/// Lends `media` to Lua while `f` runs (the `OnInitRecordedMedia` event).
pub fn with_scoped_media<R>(media: &mut RecordedMedia, f: impl FnOnce() -> R) -> R {
    let lent = std::mem::take(media);
    let prev = SCOPED_MEDIA.with(|s| s.borrow_mut().replace(lent));
    let r = f();
    *media = SCOPED_MEDIA
        .with(|s| std::mem::replace(&mut *s.borrow_mut(), prev))
        .expect("scoped media");
    r
}

fn has_radio() -> bool {
    SCOPED_RADIO.with(|s| s.borrow().is_some()) || RADIO.with(|r| r.borrow().is_some())
}

/// Runs `f` on the radio Lua sees (the lent one first).
pub fn with_radio<R>(f: impl FnOnce(&mut ZomboidRadio) -> R) -> Option<R> {
    let mut f = Some(f);
    let scoped = SCOPED_RADIO.with(|s| s.borrow_mut().as_mut().map(|r| (f.take().unwrap())(r)));
    if scoped.is_some() {
        return scoped;
    }
    let radio = RADIO.with(|r| r.borrow().clone())?;
    let mut radio = radio.borrow_mut();
    Some((f.take().unwrap())(&mut radio))
}

/// Runs `f` on the recorded media Lua sees (the lent media first, then
/// the radio's).
pub fn with_media<R>(f: impl FnOnce(&mut RecordedMedia) -> R) -> Option<R> {
    let mut f = Some(f);
    let scoped = SCOPED_MEDIA.with(|s| s.borrow_mut().as_mut().map(|m| (f.take().unwrap())(m)));
    if scoped.is_some() {
        return scoped;
    }
    with_radio(|r| (f.take().unwrap())(r.recorded_media_mut()))
}

macro_rules! unit_object {
    ($name:ident, $class:expr) => {
        pub struct $name;
        impl LuaObject for $name {
            fn class_name(&self) -> &str {
                $class
            }
            fn as_any(&self) -> &dyn Any {
                self
            }
        }
    };
}

unit_object!(LuaZomboidRadio, ZOMBOID_RADIO_CLASS);
unit_object!(LuaRadioApi, RADIO_API_CLASS);
unit_object!(LuaRecordedMedia, RECORDED_MEDIA_CLASS);

thread_local! {
    static RADIO_OBJ: Value = Value::Object(Rc::new(LuaZomboidRadio));
    static API_OBJ: Value = Value::Object(Rc::new(LuaRadioApi));
    static MEDIA_OBJ: Value = Value::Object(Rc::new(LuaRecordedMedia));
    static MEDIA_DATA_OBJS: RefCell<HashMap<String, Value>> = RefCell::new(HashMap::new());
}

/// A `MediaData`, by id.
pub struct LuaMediaData(pub String);
impl LuaObject for LuaMediaData {
    fn class_name(&self) -> &str {
        MEDIA_DATA_CLASS
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// A `MediaLineData`: the media id and the line index.
pub struct LuaMediaLine(pub String, pub i32);
impl LuaObject for LuaMediaLine {
    fn class_name(&self) -> &str {
        MEDIA_LINE_CLASS
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// The one Lua object of each media id (Java hands out the same
/// `MediaData`).
fn media_value(id: Option<&str>) -> Value {
    match id {
        None => Value::Nil,
        Some(id) => MEDIA_DATA_OBJS.with(|m| {
            m.borrow_mut()
                .entry(id.to_string())
                .or_insert_with(|| Value::Object(Rc::new(LuaMediaData(id.to_string()))))
                .clone()
        }),
    }
}

fn opt_str(s: Option<String>) -> Value {
    s.map_or(Value::Nil, |s| Value::str(&s))
}

fn list(items: Vec<Value>) -> Value {
    crate::bind_java_util::list_value(items)
}

fn radio_text_fn() -> impl Fn(&str) -> String {
    let t = text();
    move |k: &str| t.get_text(k)
}

fn register_globals(thread: &mut Thread) {
    crate::expose::global(thread, "getZomboidRadio", |c| {
        let v = if has_radio() {
            RADIO_OBJ.with(|v| v.clone())
        } else {
            Value::Nil
        };
        Ok(c.push(v))
    });
    crate::expose::global(thread, "getRadioAPI", |c| {
        let v = if API_CREATED.with(|a| a.get()) {
            API_OBJ.with(|v| v.clone())
        } else {
            Value::Nil
        };
        Ok(c.push(v))
    });
}

fn channels_table(list: Option<Vec<(i32, String)>>) -> Value {
    let t = TableRef::new();
    for (k, v) in list.unwrap_or_default() {
        t.rawset(Value::Num(k as f64), Value::str(&v));
    }
    Value::Table(t)
}

fn register_radio_api(thread: &mut Thread) {
    let b = expose_class(thread, RADIO_API_CLASS, None);
    // `days * 24`, as in the original.
    b.static_fn("timeToTimeStamp", |c| {
        let (d, h, m) = (int_arg(c, 0), int_arg(c, 1), int_arg(c, 2));
        let v = d
            .wrapping_mul(24)
            .wrapping_add(h.wrapping_mul(60))
            .wrapping_add(m);
        Ok(c.push(v as f64))
    });
    b.static_fn("timeStampToDays", |c| {
        Ok(c.push(java_div(int_arg(c, 0), 1440) as f64))
    });
    b.static_fn("timeStampToHours", |c| {
        Ok(c.push(java_div(int_arg(c, 0), 60).wrapping_rem(24) as f64))
    });
    b.static_fn("timeStampToMinutes", |c| {
        Ok(c.push(int_arg(c, 0).wrapping_rem(60) as f64))
    });
    b.static_fn("hasInstance", |c| Ok(c.push(API_CREATED.with(|a| a.get()))));
    b.static_fn("getInstance", |c| {
        API_CREATED.with(|a| a.set(true));
        Ok(c.push(API_OBJ.with(|v| v.clone())))
    });
    b.method("getChannels", |c| {
        let cat = str_arg(c, 1).map(|s| s.to_string()).unwrap_or_default();
        // `ZomboidRadio.getInstance()` makes an empty radio when there is
        // none, which has no channels.
        let l = with_radio(|r| r.channel_list(&cat)).flatten();
        Ok(c.push(channels_table(l)))
    });
}

fn java_div(a: i32, b: i32) -> i32 {
    a.wrapping_div(b)
}

fn register_zomboid_radio(thread: &mut Thread) {
    let b = expose_class(thread, ZOMBOID_RADIO_CLASS, None);
    b.static_fn("hasInstance", |c| Ok(c.push(has_radio())));
    b.static_fn("isStaticSound", |c| {
        let s = str_arg(c, 0);
        Ok(c.push(s.is_some_and(|s| pz_radio::is_static_sound(&s))))
    });
    b.method("getRecordedMedia", |c| {
        Ok(c.push(MEDIA_OBJ.with(|v| v.clone())))
    });
    b.method("getDisableBroadcasting", |c| {
        Ok(c.push(with_radio(|r| r.disable_broadcasting).unwrap_or(false)))
    });
    b.method("setDisableBroadcasting", |c| {
        let v = bool_arg(c, 1);
        with_radio(|r| r.disable_broadcasting = v);
        Ok(0)
    });
    b.method("getDisableMediaLineLearning", |c| {
        Ok(c.push(with_media(|m| m.disable_line_learning).unwrap_or(false)))
    });
    b.method("setDisableMediaLineLearning", |c| {
        let v = bool_arg(c, 1);
        with_media(|m| m.disable_line_learning = v);
        Ok(0)
    });
    b.method("getDaysSinceStart", |c| {
        Ok(c.push(with_radio(|r| r.days_since_start()).unwrap_or(0) as f64))
    });
    b.method("getChannelName", |c| {
        let f = int_arg(c, 1);
        let n = with_radio(|r| r.channel_name(f).map(str::to_string)).flatten();
        Ok(c.push(opt_str(n)))
    });
    b.method("addChannelName", |c| {
        let name = str_arg(c, 1).map(|s| s.to_string()).unwrap_or_default();
        let (freq, cat) = (
            int_arg(c, 2),
            str_arg(c, 3).map(|s| s.to_string()).unwrap_or_default(),
        );
        let over = match c.arg(4) {
            Value::Nil => false,
            v => matches!(v, Value::Bool(true)),
        };
        with_radio(|r| r.add_channel_name(&name, freq, &cat, over));
        Ok(0)
    });
    b.method("removeChannelName", |c| {
        let f = int_arg(c, 1);
        with_radio(|r| r.remove_channel_name(f));
        Ok(0)
    });
    b.method("GetChannelList", |c| {
        let cat = str_arg(c, 1).map(|s| s.to_string()).unwrap_or_default();
        let l = with_radio(|r| r.channel_list(&cat)).flatten();
        let Some(l) = l else {
            return Ok(c.push(Value::Nil));
        };
        let m = crate::bind_java_util::JavaMap::hash_map();
        for (k, v) in l {
            m.put(crate::bind_java_util::integer_value(k), Value::str(&v));
        }
        Ok(c.push(Value::Object(Rc::new(m))))
    });
    b.method("getRandomFrequency", |c| {
        let (a, b2) = (c.arg(1), c.arg(2));
        let v = with_radio(|r| {
            let mut rng = |n: i32| pz_vehicles::rand::next(n);
            if a.is_nil() {
                r.random_frequency(&mut rng)
            } else {
                let (lo, hi) = (
                    pz_kahlua::exec::raw_tonumber(&a).unwrap_or(0.0) as i32,
                    pz_kahlua::exec::raw_tonumber(&b2).unwrap_or(0.0) as i32,
                );
                r.random_frequency_in(lo, hi, &mut rng)
            }
        })
        .unwrap_or(0);
        Ok(c.push(v as f64))
    });
    b.method("getRandomBzztFzzt", |c| {
        let s = pz_radio::Airwaves::random_bzzt(&mut |n| pz_vehicles::rand::next(n));
        Ok(c.push(Value::str(s)))
    });
    b.method("computerize", |c| {
        let s = str_arg(c, 1).map(|s| s.to_string()).unwrap_or_default();
        Ok(c.push(Value::str(&ZomboidRadio::computerize(&s, &mut |n| {
            pz_vehicles::rand::next(n)
        }))))
    });
    b.method("scrambleString", |c| {
        let msg = str_arg(c, 1).map(|s| s.to_string()).unwrap_or_default();
        let intensity = int_arg(c, 2);
        let ignore = bool_arg(c, 3);
        let custom = str_arg(c, 4).map(|s| s.to_string());
        let v = with_radio(|r| {
            r.scramble_string(&msg, intensity, ignore, custom.as_deref(), &mut |n| {
                pz_vehicles::rand::next(n)
            })
        })
        .unwrap_or(msg);
        Ok(c.push(Value::str(&v)))
    });
    b.method("getFullChannelList", |c| {
        let l = with_radio(|r| r.full_channel_list()).unwrap_or_default();
        let m = crate::bind_java_util::JavaMap::hash_map();
        for (cat, chans) in l {
            let inner = crate::bind_java_util::JavaMap::hash_map();
            for (k, v) in chans {
                inner.put(crate::bind_java_util::integer_value(k), Value::str(&v));
            }
            m.put(Value::str(&cat), Value::Object(Rc::new(inner)));
        }
        Ok(c.push(Value::Object(Rc::new(m))))
    });
}

fn self_media_id(c: &NativeCall<'_>) -> LuaResult<String> {
    with_this::<LuaMediaData, _>(c, |m| m.0.clone())
}

fn with_media_data<R>(id: &str, f: impl FnOnce(&pz_radio::MediaData) -> R) -> Option<R> {
    with_media(|m| m.media_data(id).map(f)).flatten()
}

fn register_media(thread: &mut Thread) {
    let b = expose_class(thread, RECORDED_MEDIA_CLASS, None);
    b.static_fn("getMediaTypeForCategory", |c| {
        let cat = str_arg(c, 0);
        Ok(c.push(pz_radio::media::media_type_for_category(cat.as_deref()) as f64))
    });
    b.method("getCategories", |c| {
        let v = with_media(|m| m.categories().to_vec()).unwrap_or_default();
        Ok(c.push(list(v.iter().map(|s| Value::str(s)).collect())))
    });
    b.method("getAllMediaForType", |c| {
        let t = int_arg(c, 1) as i8;
        let tf = radio_text_fn();
        let ids = with_media(|m| {
            m.all_media_for_type(t, &tf)
                .iter()
                .map(|d| d.id().to_string())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
        Ok(c.push(list(ids.iter().map(|i| media_value(Some(i))).collect())))
    });
    b.method("getAllMediaForCategory", |c| {
        let cat = str_arg(c, 1).map(|s| s.to_string()).unwrap_or_default();
        let tf = radio_text_fn();
        let ids = with_media(|m| {
            m.all_media_for_category(&cat, &tf)
                .iter()
                .map(|d| d.id().to_string())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
        Ok(c.push(list(ids.iter().map(|i| media_value(Some(i))).collect())))
    });
    b.method("register", |c| {
        let s = |i| str_arg(c, i).map(|s| s.to_string()).unwrap_or_default();
        let (cat, id, name) = (s(1), s(2), s(3));
        let spawning = int_arg(c, 4);
        let ok = with_media(|m| m.register(&cat, &id, &name, spawning).is_some()).unwrap_or(false);
        Ok(c.push(if ok {
            MEDIA_DATA_OBJS.with(|m| m.borrow_mut().remove(&id));
            media_value(Some(&id))
        } else {
            Value::Nil
        }))
    });
    b.method("getMediaDataFromIndex", |c| {
        let i = int_arg(c, 1) as i16;
        let id = with_media(|m| m.media_data_from_index(i).map(|d| d.id().to_string())).flatten();
        Ok(c.push(media_value(id.as_deref())))
    });
    b.method("getMediaData", |c| {
        let id = str_arg(c, 1).map(|s| s.to_string()).unwrap_or_default();
        let ok = with_media(|m| m.media_data(&id).is_some()).unwrap_or(false);
        Ok(c.push(if ok {
            media_value(Some(&id))
        } else {
            Value::Nil
        }))
    });
    b.method("getIndexForMediaData", |c| {
        let id = match c.arg(1) {
            Value::Object(o) => o
                .as_any()
                .downcast_ref::<LuaMediaData>()
                .map(|m| m.0.clone()),
            _ => None,
        };
        let i = id
            .and_then(|id| {
                with_media(|m| m.media_data(&id).map(|d| m.index_for_media_data(d))).flatten()
            })
            .unwrap_or(-1);
        Ok(c.push(i as f64))
    });
    b.method("getRandomFromCategory", |c| {
        let cat = str_arg(c, 1).map(|s| s.to_string()).unwrap_or_default();
        let id = with_media(|m| {
            m.random_from_category(&cat, &mut |n| pz_vehicles::rand::next(n))
                .map(|d| d.id().to_string())
        })
        .flatten();
        Ok(c.push(media_value(id.as_deref())))
    });
    b.method("hasListenedToLine", |c| {
        let player = c.arg(1);
        let guid = str_arg(c, 2).map(|s| s.to_string()).unwrap_or_default();
        Ok(c.push(hooks().knows_media_line(&player, &guid)))
    });
    b.method("hasListenedToAll", |c| {
        let player = c.arg(1);
        let id = match c.arg(2) {
            Value::Object(o) => o
                .as_any()
                .downcast_ref::<LuaMediaData>()
                .map(|m| m.0.clone()),
            _ => None,
        };
        let h = hooks();
        let guids = id
            .and_then(|id| {
                with_media_data(&id, |d| {
                    (0..d.line_count() as i32)
                        .filter_map(|i| d.line(i).map(|l| l.text_guid().to_string()))
                        .collect::<Vec<_>>()
                })
            })
            .unwrap_or_default();
        let all = !guids.is_empty() && guids.iter().all(|g| h.knows_media_line(&player, g));
        Ok(c.push(all))
    });

    let b = expose_class(thread, MEDIA_DATA_CLASS, None);
    macro_rules! mget {
        ($name:literal, |$d:ident| $body:expr) => {
            b.method($name, |c| {
                let id = self_media_id(c)?;
                let v = with_media_data(&id, |$d| -> Value { $body }).unwrap_or(Value::Nil);
                Ok(c.push(v))
            });
        };
    }
    mget!("getId", |d| Value::str(d.id()));
    mget!("getIndex", |d| Value::Num(d.index() as f64));
    mget!("getIndexForLua", |d| Value::Num(d.index() as f64));
    mget!("getCategory", |d| opt_str(d.category().map(str::to_string)));
    mget!("getSpawning", |d| Value::Num(d.spawning() as f64));
    mget!("getMediaType", |d| Value::Num(d.media_type() as f64));
    mget!("getLineCount", |d| Value::Num(d.line_count() as f64));
    mget!("hasTitle", |d| Value::Bool(d.title.is_some()));
    mget!("hasSubTitle", |d| Value::Bool(d.subtitle.is_some()));
    mget!("hasAuthor", |d| Value::Bool(d.author.is_some()));
    mget!("hasExtra", |d| Value::Bool(d.extra.is_some()));
    mget!("getTranslatedItemDisplayName", |d| Value::str(
        &text().get_text(d.item_display_name())
    ));
    mget!("getTranslatedTitle", |d| opt_str(
        d.title.as_deref().map(|s| text().get_text(s))
    ));
    mget!("getTranslatedSubTitle", |d| opt_str(
        d.subtitle.as_deref().map(|s| text().get_text(s))
    ));
    mget!("getTranslatedAuthor", |d| opt_str(
        d.author.as_deref().map(|s| text().get_text(s))
    ));
    mget!("getTranslatedExtra", |d| opt_str(
        d.extra.as_deref().map(|s| text().get_text(s))
    ));
    mget!("getTitleEN", |d| opt_str(
        d.title.as_deref().map(|s| text().get_text_media_en(s))
    ));
    mget!("getSubtitleEN", |d| opt_str(
        d.subtitle.as_deref().map(|s| text().get_text_media_en(s))
    ));
    mget!("getAuthorEN", |d| opt_str(
        d.author.as_deref().map(|s| text().get_text_media_en(s))
    ));
    mget!("getExtraEN", |d| opt_str(
        d.extra.as_deref().map(|s| text().get_text_media_en(s))
    ));
    macro_rules! mset {
        ($name:literal, $field:ident) => {
            b.method($name, |c| {
                let id = self_media_id(c)?;
                let v = str_arg(c, 1).map(|s| s.to_string());
                with_media(|m| {
                    if let Some(d) = m.media_data_mut(&id) {
                        d.$field = v;
                    }
                });
                Ok(0)
            });
        };
    }
    mset!("setTitle", title);
    mset!("setSubtitle", subtitle);
    mset!("setAuthor", author);
    mset!("setExtra", extra);
    b.method("addLine", |c| {
        let id = self_media_id(c)?;
        let t = str_arg(c, 1).map(|s| s.to_string()).unwrap_or_default();
        let (r, g, bl) = (
            num_arg(c, 2) as f32,
            num_arg(c, 3) as f32,
            num_arg(c, 4) as f32,
        );
        let codes = str_arg(c, 5).map(|s| s.to_string());
        with_media(|m| {
            if let Some(d) = m.media_data_mut(&id) {
                d.add_line(&t, r, g, bl, codes.as_deref());
            }
        });
        Ok(0)
    });
    b.method("getLine", |c| {
        let id = self_media_id(c)?;
        let i = int_arg(c, 1);
        let ok = with_media_data(&id, |d| d.line(i).is_some()).unwrap_or(false);
        Ok(c.push(if ok {
            Value::Object(Rc::new(LuaMediaLine(id, i)))
        } else {
            Value::Nil
        }))
    });

    let b = expose_class(thread, MEDIA_LINE_CLASS, None);
    macro_rules! lget {
        ($name:literal, |$l:ident| $body:expr) => {
            b.method($name, |c| {
                let (id, i) = with_this::<LuaMediaLine, _>(c, |l| (l.0.clone(), l.1))?;
                let v = with_media_data(&id, |d| d.line(i).map(|$l| -> Value { $body }))
                    .flatten()
                    .unwrap_or(Value::Nil);
                Ok(c.push(v))
            });
        };
    }
    lget!("getTranslatedText", |l| Value::str(
        &text().get_text(l.text_guid())
    ));
    lget!("getTextGuid", |l| Value::str(l.text_guid()));
    lget!("getCodes", |l| opt_str(l.codes().map(str::to_string)));
    lget!("getR", |l| Value::Num(l.r() as f64));
    lget!("getG", |l| Value::Num(l.g() as f64));
    lget!("getB", |l| Value::Num(l.b() as f64));
}

/// Lua's reference to the actual PresetEntry, retained after a remove,
/// clear, replacement, or insertion of the same entry at several slots.
pub struct LuaPresetEntry(pub PresetEntryRef);
impl LuaObject for LuaPresetEntry {
    fn class_name(&self) -> &str {
        PRESET_ENTRY_CLASS
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// Lua's reference to the actual DevicePresets allocation.
pub struct LuaDevicePresets(pub DevicePresetsRef);
impl LuaObject for LuaDevicePresets {
    fn class_name(&self) -> &str {
        DEVICE_PRESETS_CLASS
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

fn cached_owner<T>(
    cache: &RefCell<HashMap<usize, Weak<T>>>,
    key: usize,
    make: impl FnOnce() -> T,
) -> Rc<T> {
    if let Some(v) = cache.borrow().get(&key).and_then(Weak::upgrade) {
        return v;
    }
    let value = Rc::new(make());
    let mut cache = cache.borrow_mut();
    if cache.len() == cache.capacity() {
        cache.retain(|_, value| value.strong_count() != 0);
    }
    cache.insert(key, Rc::downgrade(&value));
    value
}

fn entry_value(entry: &PresetEntryRef) -> Value {
    let key = Rc::as_ptr(entry) as usize;
    PRESET_ENTRIES
        .with(|cache| Value::Object(cached_owner::<LuaPresetEntry>(cache, key, || LuaPresetEntry(entry.clone()))))
}

fn preset_value(presets: &DevicePresetsRef) -> Value {
    let key = Rc::as_ptr(presets.owner()) as usize;
    PRESET_OWNERS.with(|cache| {
        Value::Object(cached_owner::<LuaDevicePresets>(cache, key, || {
            LuaDevicePresets(presets.clone())
        }))
    })
}

fn presets_of(o: &DeviceOwner) -> Option<DevicePresetsRef> {
    LuaDeviceData(o.clone()).with(|d| d.presets.clone())
}

fn entry_of(v: &Value) -> Option<PresetEntryRef> {
    match v {
        Value::Object(o) => Some(o.as_any().downcast_ref::<LuaPresetEntry>()?.0.clone()),
        _ => None,
    }
}

/// The existing collection operation facade over this actual list. Each
/// outward entry is the same owner; the facade's legacy refresh/iterator
/// semantics are not a claim of full NativeArrayList equivalence.
fn presets_list(list: &PresetListRef) -> Value {
    let key = Rc::as_ptr(list) as usize;
    PRESET_LISTS.with(|cache| {
        Value::Object(cached_owner::<crate::bind_java_util::JavaList>(cache, key, || {
            let (r, w) = (list.clone(), list.clone());
            let read = move || {
                let entries = r.borrow().clone();
                entries.iter().map(entry_value).collect()
            };
            let write: crate::bind_java_util::ListWrite = Box::new(move |items: Vec<Value>| {
                let mut entries = Vec::with_capacity(items.len());
                for v in &items {
                    match entry_of(v) {
                        Some(e) => entries.push(e),
                        None => {
                            return Err(crate::bind_java_util::class_cast(v, PRESET_ENTRY_CLASS))
                        }
                    }
                }
                *w.borrow_mut() = entries;
                Ok(())
            });
            crate::bind_java_util::JavaList::live(read, Some(write))
        }))
    })
}

fn register_presets(thread: &mut Thread) {
    let b = expose_class(thread, PRESET_ENTRY_CLASS, None);
    b.ctor(|c| {
        let name = str_arg(c, 0).map(|s| s.to_string()).unwrap_or_default();
        let frequency = int_arg(c, 1);
        let entry = Rc::new(RefCell::new(PresetEntry { name, frequency }));
        Ok(c.push(entry_value(&entry)))
    });
    b.method("getName", |c| {
        let name = with_this::<LuaPresetEntry, _>(c, |e| e.0.borrow().name.clone())?;
        Ok(c.push(Value::str(&name)))
    });
    b.method("getFrequency", |c| {
        let frequency = with_this::<LuaPresetEntry, _>(c, |e| e.0.borrow().frequency)?;
        Ok(c.push(frequency as f64))
    });
    b.method("setName", |c| {
        let name = str_arg(c, 1).map(|s| s.to_string()).unwrap_or_default();
        with_this::<LuaPresetEntry, _>(c, |e| e.0.borrow_mut().name = name)?;
        Ok(0)
    });
    b.method("setFrequency", |c| {
        let frequency = int_arg(c, 1);
        with_this::<LuaPresetEntry, _>(c, |e| e.0.borrow_mut().frequency = frequency)?;
        Ok(0)
    });

    let b = expose_class(thread, DEVICE_PRESETS_CLASS, None);
    fn owner(c: &NativeCall<'_>) -> LuaResult<DevicePresetsRef> {
        with_this::<LuaDevicePresets, _>(c, |p| p.0.clone())
    }
    b.method("getPresets", |c| {
        let p = owner(c)?;
        Ok(c.push(presets_list(&p.list())))
    });
    b.method("getMaxPresets", |c| {
        Ok(c.push(owner(c)?.max_presets() as f64))
    });
    b.method("setMaxPresets", |c| {
        owner(c)?.set_max_presets(int_arg(c, 1));
        Ok(0)
    });
    b.method("addPreset", |c| {
        let name = str_arg(c, 1).map(|s| s.to_string()).unwrap_or_default();
        owner(c)?.add_preset(&name, int_arg(c, 2));
        Ok(0)
    });
    b.method("removePreset", |c| {
        owner(c)?.remove_preset(int_arg(c, 1));
        Ok(0)
    });
    b.method("getPresetName", |c| {
        let name = owner(c)?.preset_name(int_arg(c, 1));
        Ok(c.push(Value::str(&name)))
    });
    b.method("getPresetFreq", |c| {
        Ok(c.push(owner(c)?.preset_freq(int_arg(c, 1)) as f64))
    });
    b.method("setPresetName", |c| {
        let name = str_arg(c, 2).map(|s| s.to_string());
        owner(c)?.set_preset_name(int_arg(c, 1), name.as_deref());
        Ok(0)
    });
    b.method("setPresetFreq", |c| {
        owner(c)?.set_preset_freq(int_arg(c, 1), int_arg(c, 2));
        Ok(0)
    });
    b.method("setPreset", |c| {
        let name = str_arg(c, 2).map(|s| s.to_string());
        owner(c)?.set_preset(int_arg(c, 1), name.as_deref(), int_arg(c, 3));
        Ok(0)
    });
    b.method("clearPresets", |c| {
        owner(c)?.clear_presets();
        Ok(0)
    });
    b.method("getPresetsLua", |c| {
        let list = owner(c)?.list();
        let entries = list.borrow().clone();
        let t = TableRef::new();
        for (i, entry) in entries.iter().enumerate() {
            let entry = entry.borrow();
            let row = TableRef::new();
            row.set_str("name", Value::str(&entry.name));
            row.set_str("frequency", Value::Num(entry.frequency as f64));
            t.rawset(Value::Num(i as f64), Value::Table(row));
        }
        Ok(c.push(Value::Table(t)))
    });
}

/// The environment used when the engine has no device environment for a
/// device: single player, no sound, the vehicle code's `Rand` and game
/// time, the radio's channels and media; events are dropped.
struct FallbackEnv {
    parent: ParentKind,
    mains: MainsPower,
    pos: [f32; 3],
}

impl FallbackEnv {
    fn new(o: &DeviceOwner) -> Self {
        match o {
            DeviceOwner::Part(p) => {
                let v = p.vehicle();
                FallbackEnv {
                    parent: ParentKind::Vehicle,
                    mains: MainsPower::Vehicle {
                        uninstalled: p.is_inventory_item_uninstalled(),
                        has_power: v.battery_charge() > 0.0,
                    },
                    pos: [v.x(), v.y(), v.z()],
                }
            }
            DeviceOwner::Item(_) => FallbackEnv {
                parent: ParentKind::Item,
                mains: MainsPower::Square(None),
                pos: [0.0; 3],
            },
            DeviceOwner::World(_) => FallbackEnv {
                parent: ParentKind::Iso,
                mains: MainsPower::Square(None),
                pos: [0.0; 3],
            },
        }
    }
}

impl DeviceEnv for FallbackEnv {
    fn net(&self) -> NetMode {
        NetMode::SinglePlayer
    }
    fn minutes_stamp(&self) -> i64 {
        (pz_vehicles::env::time().world_age_hours * 60.0) as i64
    }
    fn multiplier(&self) -> f32 {
        pz_vehicles::env::time().multiplier
    }
    fn parent(&self) -> ParentKind {
        self.parent
    }
    fn next_int(&mut self, bound: i32) -> i32 {
        pz_vehicles::rand::next(bound)
    }
    fn next_float(&mut self) -> f32 {
        pz_vehicles::rand::next_float(0.0, 1.0)
    }
    fn mains_power(&self) -> MainsPower {
        self.mains
    }
    fn channel_list(&self, category: &str) -> Option<Vec<(i32, String)>> {
        with_radio(|r| r.channel_list(category)).unwrap_or_else(|| {
            pz_vehicles::env::items()
                .channel_lists()
                .channel_list(category)
        })
    }
    fn media_exists(&self, index: i16) -> bool {
        with_media(|m| m.media_data_from_index(index).is_some()).unwrap_or(false)
    }
    fn media_line(&self, index: i16, line: i32) -> Option<MediaLine> {
        let tf = radio_text_fn();
        with_media(|m| pz_radio::media_line(m, &tf, index, line)).flatten()
    }
    fn iso_object(&self) -> Option<IsoObjectPos> {
        None
    }
    fn closest_listener_distance_sq(&self, _x: f32, _y: f32, _z: f32) -> f32 {
        f32::MAX
    }
    fn parent_pos(&self) -> [f32; 3] {
        self.pos
    }
    fn event(&mut self, _e: DeviceEvent) {}
}

fn this_owner(c: &NativeCall<'_>) -> LuaResult<DeviceOwner> {
    with_this::<LuaDeviceData, _>(c, |d| d.0.clone())
}

/// Runs `f` on the device data of `self` with its environment.
fn with_env<R>(
    c: &NativeCall<'_>,
    f: impl FnOnce(&mut DeviceData, &mut dyn DeviceEnv) -> R,
) -> LuaResult<Option<R>> {
    let o = this_owner(c)?;
    let mut env: Box<dyn DeviceEnv> = hooks()
        .device_env(&o)
        .unwrap_or_else(|| Box::new(FallbackEnv::new(&o)));
    Ok(LuaDeviceData(o).with(|d| f(d, &mut *env)))
}

fn call(t: &mut Thread, obj: &Value, name: &str, args: &[Value]) -> LuaResult<Value> {
    let f = t.table_get(obj, &Value::str(name))?;
    if f.is_nil() {
        return Err(LuaError::runtime(format!(
            "DeviceData: {}.{name} is not bound",
            pz_kahlua::stdlib::java_class_name(obj)
        )));
    }
    let mut all = vec![obj.clone()];
    all.extend_from_slice(args);
    t.call1(&f, &all)
}

fn lua_str(v: Value) -> Option<String> {
    match v {
        Value::Str(s) => Some(s.to_string()),
        _ => None,
    }
}

fn lua_num(v: &Value) -> f64 {
    pz_kahlua::exec::raw_tonumber(v).unwrap_or(0.0)
}

/// Public-port capability failure for a positional DeviceOwner whose
/// DeviceData disappeared. This is not a fabricated native NPE/default.
fn missing_item_call_owner() -> LuaError {
    LuaError::runtime("port capability unavailable: actual radio DeviceData owner for item call")
}

fn required_item_call_device<R>(
    owner: &DeviceOwner,
    f: impl FnOnce(&mut DeviceData) -> R,
) -> LuaResult<R> {
    LuaDeviceData(owner.clone())
        .with(f)
        .ok_or_else(missing_item_call_owner)
}

fn required_item_call_env<R>(
    owner: &DeviceOwner,
    f: impl FnOnce(&mut DeviceData, &mut dyn DeviceEnv) -> R,
) -> LuaResult<R> {
    // End this guard before invoking the environment factory, which can
    // reenter. Check again when entering the actual environment operation.
    required_item_call_device(owner, |_| ())?;
    let mut env: Box<dyn DeviceEnv> = hooks().device_env(owner).ok_or_else(|| {
        LuaError::runtime("port capability unavailable: actual radio DeviceEnv for item call")
    })?;
    required_item_call_device(owner, |device| f(device, &mut *env))
}

fn required_item_call_method(
    thread: &mut Thread,
    owner: &DeviceOwner,
    object: &Value,
    name: &str,
    args: &[Value],
) -> LuaResult<Value> {
    required_item_call_device(owner, |_| ())?;
    let function = thread.table_get(object, &Value::str(name))?;
    required_item_call_device(owner, |_| ())?;
    if function.is_nil() {
        return Err(LuaError::runtime(format!(
            "DeviceData: {}.{name} is not bound",
            pz_kahlua::stdlib::java_class_name(object)
        )));
    }
    let mut all = vec![object.clone()];
    all.extend_from_slice(args);
    let result = thread.call1(&function, &all)?;
    required_item_call_device(owner, |_| ())?;
    Ok(result)
}

/// `item:getContainer():DoRemoveItem(item)` (nothing without a
/// container); `true` when there was a container.
fn remove_from_container(
    c: &mut NativeCall<'_>,
    owner: &DeviceOwner,
    item: &Value,
) -> LuaResult<bool> {
    // Both native callers obtain the container before the client gate.
    let cont = required_item_call_method(c.thread, owner, item, "getContainer", &[])?;
    if cont.is_nil() {
        return Ok(false);
    }
    let client = required_item_call_env(owner, |_, env| env.net() == NetMode::Client)?;
    if !client {
        required_item_call_method(
            c.thread,
            owner,
            &cont,
            "DoRemoveItem",
            std::slice::from_ref(item),
        )?;
    }
    Ok(true)
}

fn register_device_data(thread: &mut Thread) {
    let b = expose_class(thread, DEVICE_DATA_CLASS, None);
    macro_rules! dget {
        ($name:literal, |$d:ident| $body:expr) => {
            b.method($name, |c| {
                let o = this_owner(c)?;
                let v = LuaDeviceData(o).with(|$d| -> Value { $body });
                Ok(c.push(v.unwrap_or_default()))
            });
        };
    }
    macro_rules! denv {
        ($name:literal, |$d:ident, $e:ident, $c:ident| $body:expr) => {
            b.method($name, |$c| {
                let r = with_env($c, |$d, $e| -> Value { $body })?;
                Ok($c.push(r.unwrap_or_default()))
            });
        };
    }
    dget!("hasMedia", |d| Value::Bool(d.has_media()));
    dget!("getPower", |d| Value::Num(d.power_delta as f64));
    dget!("getHeadphoneType", |d| Value::Num(d.headphone_type as f64));
    dget!("isPlayingMedia", |d| Value::Bool(d.is_playing_media()));
    dget!("getDeviceVolume", |d| Value::Num(d.device_volume as f64));
    dget!("getMicIsMuted", |d| Value::Bool(d.mic_is_muted));
    dget!("getHasBattery", |d| Value::Bool(d.has_battery));
    dget!("isNoTransmit", |d| Value::Bool(d.no_transmit));
    dget!("isReceivingSignal", |d| Value::Bool(
        d.is_receiving_signal()
    ));
    dget!("getMediaIndex", |d| Value::Num(d.media_index as f64));
    dget!("getLastRecordedDistance", |d| Value::Num(
        d.runtime.last_recorded_distance as f64
    ));
    dget!("getDeviceVolumeRange", |d| Value::Num(
        d.device_volume_range() as f64
    ));
    dget!("isTelevision", |d| Value::Bool(d.is_television));
    b.method("getDevicePresets", |c| {
        let o = this_owner(c)?;
        Ok(c.push(presets_of(&o).map_or(Value::Nil, |p| preset_value(&p))))
    });
    b.method("isVehicleDevice", |c| {
        Ok(c.push(matches!(this_owner(c)?, DeviceOwner::Part(_))))
    });
    b.method("isIsoDevice", |c| {
        Ok(c.push(matches!(this_owner(c)?, DeviceOwner::World(_))))
    });
    b.method("isInventoryDevice", |c| {
        Ok(c.push(matches!(this_owner(c)?, DeviceOwner::Item(_))))
    });
    b.method("getParent", |c| {
        let v = match this_owner(c)? {
            DeviceOwner::Part(p) => pz_vehicles::lua::part_value(&p),
            DeviceOwner::Item(i) => crate::bind_items::item_value(&i),
            DeviceOwner::World(w) => w.lua_value(),
        };
        Ok(c.push(v))
    });
    b.method("getMediaData", |c| {
        let o = this_owner(c)?;
        let idx = LuaDeviceData(o).with(|d| d.media_index).unwrap_or(-1);
        let id = if idx >= 0 {
            with_media(|m| m.media_data_from_index(idx).map(|d| d.id().to_string())).flatten()
        } else {
            None
        };
        Ok(c.push(media_value(id.as_deref())))
    });
    b.method("canPlayerRemoteInteract", |c| {
        let o = this_owner(c)?;
        let tv = LuaDeviceData(o.clone())
            .with(|d| d.is_television)
            .unwrap_or(false);
        let ch = c.arg(1);
        Ok(c.push(tv && !ch.is_nil() && hooks().can_player_remote_interact(&o, &ch)))
    });
    macro_rules! dsetv {
        ($name:literal, |$d:ident, $c:ident| $body:expr) => {
            b.method($name, |$c| {
                let o = this_owner($c)?;
                let a = $c.arg(1);
                LuaDeviceData(o).with(|$d| {
                    let $c = &a;
                    $body
                });
                Ok(0)
            });
        };
    }
    dsetv!("setPower", |d, a| d.set_power(lua_num(a) as f32));
    dsetv!("setHeadphoneType", |d, a| d.headphone_type =
        lua_num(a) as i32);
    dsetv!("setHasBattery", |d, a| d.has_battery =
        matches!(a, Value::Bool(true)));
    dsetv!("setNoTransmit", |d, a| d.no_transmit =
        matches!(a, Value::Bool(true)));
    dsetv!("setMediaIndex", |d, a| d.media_index = lua_num(a) as i16);
    dsetv!("setDeviceVolumeRaw", |d, a| d
        .set_device_volume_raw(lua_num(a) as f32));
    dsetv!("setChannelRaw", |d, a| d.set_channel_raw(lua_num(a) as i32));
    dsetv!("setMicRange", |d, a| d.mic_range = lua_num(a) as i32);
    b.method("cloneDevicePresets", |c| {
        let src = match c.arg(1) {
            Value::Object(o) => o
                .as_any()
                .downcast_ref::<LuaDevicePresets>()
                .map(|p| p.0.clone()),
            _ => None,
        };
        let o = this_owner(c)?;
        LuaDeviceData(o).with(|d| d.clone_device_presets(src.as_ref()));
        Ok(0)
    });
    b.method("setDevicePresets", |c| {
        let src = match c.arg(1) {
            Value::Object(o) => o
                .as_any()
                .downcast_ref::<LuaDevicePresets>()
                .map(|p| p.0.clone()),
            _ => None,
        };
        let o = this_owner(c)?;
        LuaDeviceData(o).with(|d| d.presets = src.unwrap_or_default());
        Ok(0)
    });
    denv!("setIsTurnedOn", |d, e, c| {
        d.set_is_turned_on(e, matches!(c.arg(1), Value::Bool(true)));
        Value::Nil
    });
    denv!("setTurnedOnRaw", |d, e, c| {
        d.set_turned_on_raw_env(e, matches!(c.arg(1), Value::Bool(true)));
        Value::Nil
    });
    denv!("setMicIsMuted", |d, e, c| {
        d.set_mic_is_muted(e, matches!(c.arg(1), Value::Bool(true)));
        Value::Nil
    });
    denv!("setDeviceVolume", |d, e, c| {
        d.set_device_volume_env(e, lua_num(&c.arg(1)) as f32);
        Value::Nil
    });
    denv!("setChannel", |d, e, c| {
        let set_listening = !matches!(c.arg(2), Value::Bool(false));
        d.set_channel(e, lua_num(&c.arg(1)) as i32, set_listening);
        Value::Nil
    });
    denv!("canBePoweredHere", |d, e, _c| Value::Bool(
        d.can_be_powered_here(e)
    ));
    denv!("getDeviceSoundVolumeRange", |d, e, _c| Value::Num(
        d.device_sound_volume_range(e) as f64
    ));
    denv!("isEmergencyBroadcast", |d, e, _c| Value::Bool(
        d.is_emergency_broadcast(e)
    ));
    denv!("TriggerPlayerListening", |d, e, _c| {
        d.trigger_player_listening(e);
        Value::Nil
    });
    denv!("setInitialPower", |d, e, _c| {
        d.set_initial_power(e.minutes_stamp());
        Value::Nil
    });
    denv!("playSoundSend", |d, e, c| {
        let n = lua_str(c.arg(1)).unwrap_or_default();
        d.play_sound_send(e, &n, bool_arg(c, 2));
        Value::Nil
    });
    denv!("playSoundLocal", |d, e, c| {
        let n = lua_str(c.arg(1)).unwrap_or_default();
        d.play_sound_local(e, &n, bool_arg(c, 2));
        Value::Nil
    });
    denv!("stopOrTriggerSoundByName", |d, e, c| {
        let n = lua_str(c.arg(1)).unwrap_or_default();
        d.stop_or_trigger_sound_by_name(e, &n);
        Value::Nil
    });
    denv!("cleanSoundsAndEmitter", |d, e, _c| {
        d.clean_sounds_and_emitter(e);
        Value::Nil
    });
    denv!("StartPlayMedia", |d, e, _c| {
        d.start_play_media(e);
        Value::Nil
    });
    denv!("StopPlayMedia", |d, e, _c| {
        d.stop_play_media(e);
        Value::Nil
    });
    denv!("transmitPresets", |_d, _e, _c| Value::Nil);
    denv!("transmitBatteryChange", |_d, _e, _c| Value::Nil);
    denv!("addEmergencyChannel", |d, e, _c| {
        if !d.is_television {
            if let Some(list) = e.channel_list("Emergency") {
                for (k, v) in list {
                    d.presets.add_preset(&v, k);
                }
            }
        }
        Value::Nil
    });
    denv!("generatePresets", |d, e, _c| {
        let lists = EnvLists(&*e);
        let mut next = |b: i32| pz_vehicles::rand::next(b);
        d.generate_presets(&lists, &mut next);
        Value::Nil
    });
    // Items moving in and out of the device go through the items' own
    // bindings (`getFullType`, `getContainer():DoRemoveItem`,
    // `inventory:AddItem`).
    b.method("addBattery", |c| {
        let owner = this_owner(c)?;
        if required_item_call_device(&owner, |d| d.has_battery)? {
            return Ok(0);
        }
        let bat = c.arg(1);
        if bat.is_nil() {
            return Ok(0);
        }
        let full_type = lua_str(required_item_call_method(
            c.thread,
            &owner,
            &bat,
            "getFullType",
            &[],
        )?)
        .unwrap_or_default();
        if full_type != "Base.Battery" {
            return Ok(0);
        }
        remove_from_container(c, &owner, &bat)?;
        let uses = lua_num(&required_item_call_method(
            c.thread,
            &owner,
            &bat,
            "getCurrentUsesFloat",
            &[],
        )?) as f32;
        // Native writes after removal; it does not repeat hasBattery's
        // initial gate after the item/container calls.
        required_item_call_device(&owner, |d| {
            d.power_delta = uses;
            d.has_battery = true;
        })?;
        required_item_call_env(&owner, |d, env| d.transmit_battery_state(env))?;
        Ok(0)
    });
    b.method("getBattery", |c| {
        let inv = c.arg(1);
        let p = with_env(c, |d, e| d.take_battery(e))?.flatten();
        if let (Some(p), false) = (p, inv.is_nil()) {
            let bat = call(c.thread, &inv, "AddItem", &[Value::str("Base.Battery")])?;
            if !bat.is_nil() {
                let max = lua_num(&call(c.thread, &bat, "getMaxUses", &[])?) as f32;
                let uses = (max * p) as i32;
                call(c.thread, &bat, "setCurrentUses", &[Value::Num(uses as f64)])?;
            }
        }
        Ok(0)
    });
    b.method("addHeadphones", |c| {
        let (item, cont) = (c.arg(1), c.arg(2));
        if item.is_nil() {
            return Ok(0);
        }
        let ft = lua_str(call(c.thread, &item, "getFullType", &[])?).unwrap_or_default();
        let ok = with_env(c, |d, e| d.add_headphones(e, &ft, !cont.is_nil()))?.unwrap_or(false);
        if ok {
            call(c.thread, &cont, "DoRemoveItem", std::slice::from_ref(&item))?;
        }
        Ok(0)
    });
    b.method("addHeadphonesToInventory", |c| {
        let inv = c.arg(1);
        let t = with_env(c, |d, e| d.take_headphones(e))?.flatten();
        if let (Some(t), false) = (t, inv.is_nil()) {
            call(c.thread, &inv, "AddItem", &[Value::str(t)])?;
        }
        Ok(0)
    });
    b.method("addMediaItem", |c| {
        let owner = this_owner(c)?;
        if required_item_call_device(&owner, |d| d.media_index >= 0)? {
            return Ok(0);
        }
        let media = c.arg(1);
        if media.is_nil() {
            return Ok(0);
        }
        if !matches!(
            required_item_call_method(c.thread, &owner, &media, "isRecordedMedia", &[])?,
            Value::Bool(true)
        ) {
            return Ok(0);
        }
        let media_type = lua_num(&required_item_call_method(
            c.thread,
            &owner,
            &media,
            "getMediaType",
            &[],
        )?) as i8;
        if !required_item_call_device(&owner, |d| d.media_type == media_type)? {
            return Ok(0);
        }
        remove_from_container(c, &owner, &media)?;
        let index = lua_num(&required_item_call_method(
            c.thread,
            &owner,
            &media,
            "getRecordedMediaIndex",
            &[],
        )?) as i16;
        required_item_call_device(&owner, |d| d.media_index = index)?;
        // Native stores mediaIndex before obtaining/storing mediaItem;
        // an error in this later read must retain the earlier write.
        let full_type = lua_str(required_item_call_method(
            c.thread,
            &owner,
            &media,
            "getFullType",
            &[],
        )?)
        .unwrap_or_default();
        required_item_call_device(&owner, |d| d.media_item = Some(full_type))?;
        required_item_call_env(&owner, |d, env| d.transmit_media_item_state(env))?;
        Ok(0)
    });
    b.method("removeMediaItem", |c| {
        let inv = c.arg(1);
        let out = with_env(c, |d, e| d.remove_media_item(e))?.flatten();
        if let (Some((Some(ft), idx)), false) = (out, inv.is_nil()) {
            let item = call(c.thread, &inv, "AddItem", &[Value::str(&ft)])?;
            if !item.is_nil() {
                call(
                    c.thread,
                    &item,
                    "setRecordedMediaIndex",
                    &[Value::Num(idx as f64)],
                )?;
            }
        }
        Ok(0)
    });
}

/// `ZomboidRadio.GetChannelList` through a device environment.
struct EnvLists<'a>(&'a dyn DeviceEnv);
impl pz_inventory::device::ChannelLists for EnvLists<'_> {
    fn channel_list(&self, category: &str) -> Option<Vec<(i32, String)>> {
        self.0.channel_list(category)
    }
}

/// Registers the radio classes (after bind_vehicles, whose `DeviceData`
/// methods this extends and partly replaces).
pub fn register(thread: &mut Thread) {
    register_globals(thread);
    register_radio_api(thread);
    register_zomboid_radio(thread);
    register_media(thread);
    register_presets(thread);
    register_device_data(thread);
}

#[cfg(test)]
mod tests;
