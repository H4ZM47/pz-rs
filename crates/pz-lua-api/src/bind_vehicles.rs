//! `BaseVehicle`, `VehiclePart`, `VehicleDoor`, `VehicleWindow`,
//! `VehicleLight`, `VehicleEngine` and `VehicleScript` exposed to Lua over
//! the [`pz_vehicles`] runtime, with the methods the game's Lua calls on
//! them (the vehicle part functions in `Vehicles.lua`, the mechanics UI and
//! timed actions), plus the parts of `InventoryItem`, `DeviceData`,
//! `ArrayList` and `HashMap` those functions use on part items.
//!
//! Every method runs the vehicle code with the calling thread installed
//! ([`pz_vehicles::lua::with_thread`]), so vehicle code that calls back
//! into Lua (`repair`, `isEngineWorking`, ...) re-enters on it.
//!
//! The item methods here are the vehicle subset; pz-lua-api's full item
//! binding (`bind_items` on the items branch) uses the same item type
//! (`Rc<RefCell<InventoryItem>>`) and should replace them when merged.

use crate::expose::{bool_arg, expose_class, int_arg, num_arg, str_arg, with_this};
use pz_inventory::device::DeviceData;
use pz_inventory::item::ItemKind;
use pz_kahlua::value::LuaObject;
use pz_kahlua::{LuaResult, NativeCall, TableRef, Thread, Value};
use pz_vehicles::items::{self, ItemRef};
use pz_vehicles::lua::{
    self as vlua, LuaDoor, LuaEngine, LuaLight, LuaPart, LuaVehicle, LuaVehicleScript, LuaWindow,
};
use pz_vehicles::part::{PartItem, PartRef};
use pz_vehicles::vehicle::{BaseVehicle, VehicleRef};
use std::any::Any;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::{Rc, Weak};

pub const INVENTORY_ITEM_CLASS: &str = "zombie.inventory.InventoryItem";
pub const DRAINABLE_CLASS: &str = "zombie.inventory.types.DrainableComboItem";
pub const DEVICE_DATA_CLASS: &str = "zombie.radio.devices.DeviceData";
pub const ARRAY_LIST_CLASS: &str = "java.util.ArrayList";
pub const HASH_MAP_CLASS: &str = "java.util.HashMap";

fn f(v: f32) -> Value {
    Value::Num(v as f64)
}

fn opt(v: Option<String>) -> Value {
    v.map_or(Value::Nil, |s| Value::str(&s))
}

/// An item handed to Lua by the vehicle code.
pub struct LuaVehItem {
    pub item: ItemRef,
    class: &'static str,
}

impl LuaObject for LuaVehItem {
    fn class_name(&self) -> &str {
        self.class
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn super_classes(&self) -> &[&'static str] {
        &[INVENTORY_ITEM_CLASS, "zombie.entity.GameEntity"]
    }
}

thread_local! {
    static ITEMS: RefCell<HashMap<usize, Weak<dyn LuaObject>>> = RefCell::new(HashMap::new());
}

/// The Lua object of an item (one per item while it lives).
pub fn item_value(i: &ItemRef) -> Value {
    let key = Rc::as_ptr(i) as usize;
    if let Some(o) = ITEMS.with(|m| m.borrow().get(&key).and_then(Weak::upgrade)) {
        return Value::Object(o);
    }
    let class = if items::is_drainable(&i.borrow()) {
        DRAINABLE_CLASS
    } else {
        INVENTORY_ITEM_CLASS
    };
    let o: Rc<dyn LuaObject> = Rc::new(LuaVehItem {
        item: i.clone(),
        class,
    });
    ITEMS.with(|m| {
        let mut m = m.borrow_mut();
        m.retain(|_, w| w.strong_count() > 0);
        m.insert(key, Rc::downgrade(&o));
    });
    Value::Object(o)
}

fn item_of(v: &Value) -> Option<ItemRef> {
    match v {
        Value::Object(o) => o
            .as_any()
            .downcast_ref::<LuaVehItem>()
            .map(|i| i.item.clone()),
        _ => None,
    }
}

/// Where a `DeviceData` lives.
#[derive(Clone)]
enum DeviceOwner {
    Part(PartRef),
    Item(ItemRef),
}

/// `DeviceData` of a vehicle part (radio) or of a radio item.
pub struct LuaDeviceData(DeviceOwner);

impl LuaObject for LuaDeviceData {
    fn class_name(&self) -> &str {
        DEVICE_DATA_CLASS
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

impl LuaDeviceData {
    fn with<R>(&self, f: impl FnOnce(&mut DeviceData) -> R) -> Option<R> {
        match &self.0 {
            DeviceOwner::Part(p) => p.st_mut().device_data.as_mut().map(f),
            DeviceOwner::Item(i) => match &mut i.borrow_mut().kind {
                ItemKind::Radio(_, d) => Some(f(d)),
                _ => None,
            },
        }
    }
}

/// A Java `ArrayList<String>` (`part:getItemType()`).
pub struct LuaStringList(pub Vec<String>);

impl LuaObject for LuaStringList {
    fn class_name(&self) -> &str {
        ARRAY_LIST_CLASS
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// `vehicle:getChoosenParts()` (a `HashMap<String, String>` of the vehicle).
pub struct LuaChoosenParts(pub VehicleRef);

impl LuaObject for LuaChoosenParts {
    fn class_name(&self) -> &str {
        HASH_MAP_CLASS
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// Runs vehicle code with the call's thread installed.
fn run<R>(c: &mut NativeCall<'_>, f: impl FnOnce() -> R) -> R {
    let t: &mut Thread = c.thread;
    vlua::with_thread(t, f)
}

fn this_vehicle(c: &NativeCall<'_>) -> LuaResult<VehicleRef> {
    with_this::<LuaVehicle, _>(c, |v| v.0.clone())
}

fn this_part(c: &NativeCall<'_>) -> LuaResult<PartRef> {
    with_this::<LuaPart, _>(c, |p| p.0.clone())
}

fn part_arg(c: &NativeCall<'_>, i: usize) -> Option<PartRef> {
    vlua::part_from_value(&c.arg(i))
}

macro_rules! vmethod {
    ($b:expr, $name:literal, |$v:ident, $c:ident| $body:expr) => {
        $b.method($name, |$c| {
            let $v = this_vehicle($c)?;
            let r: Value = $body;
            Ok($c.push(r))
        });
    };
}

macro_rules! pmethod {
    ($b:expr, $name:literal, |$p:ident, $c:ident| $body:expr) => {
        $b.method($name, |$c| {
            let $p = this_part($c)?;
            let r: Value = $body;
            Ok($c.push(r))
        });
    };
}

fn register_vehicle(thread: &mut Thread) {
    let b = expose_class(thread, vlua::BASE_VEHICLE_CLASS, None);
    vmethod!(b, "getId", |v, c| Value::Num(v.id() as f64));
    vmethod!(b, "getX", |v, c| f(v.x()));
    vmethod!(b, "getY", |v, c| f(v.y()));
    vmethod!(b, "getZ", |v, c| f(v.z()));
    vmethod!(b, "getScriptName", |v, c| Value::str(&v.script_name()));
    vmethod!(b, "getScript", |v, c| v.script().map_or(Value::Nil, |s| {
        Value::Object(Rc::new(LuaVehicleScript(s)))
    }));
    vmethod!(b, "getPartById", |v, c| {
        let id = str_arg(c, 1);
        id.and_then(|id| v.part_by_id(&id))
            .map_or(Value::Nil, |p| vlua::part_value(&p))
    });
    vmethod!(b, "getPartByIndex", |v, c| v
        .part_by_index(int_arg(c, 1))
        .map_or(Value::Nil, |p| vlua::part_value(&p)));
    vmethod!(b, "getPartCount", |v, c| Value::Num(v.part_count() as f64));
    vmethod!(b, "getBattery", |v, c| v
        .battery()
        .map_or(Value::Nil, |p| vlua::part_value(&p)));
    vmethod!(b, "getHeater", |v, c| v
        .heater()
        .map_or(Value::Nil, |p| vlua::part_value(&p)));
    vmethod!(b, "getGasTank", |v, c| v
        .gas_tank()
        .map_or(Value::Nil, |p| vlua::part_value(&p)));
    vmethod!(b, "getBatteryCharge", |v, c| f(v.battery_charge()));
    vmethod!(b, "isEngineRunning", |v, c| Value::Bool(
        v.is_engine_running()
    ));
    vmethod!(b, "isEngineStarted", |v, c| Value::Bool(
        v.is_engine_started()
    ));
    vmethod!(b, "isEngineWorking", |v, c| Value::Bool(
        run(c, || v.is_engine_working())
    ));
    vmethod!(b, "hasEnoughGasToRun", |v, c| Value::Bool(
        v.has_enough_gas_to_run()
    ));
    vmethod!(b, "getEngineQuality", |v, c| Value::Num(
        v.engine_quality() as f64
    ));
    vmethod!(b, "getEngineLoudness", |v, c| Value::Num(
        v.engine_loudness() as f64
    ));
    vmethod!(b, "getEnginePower", |v, c| Value::Num(
        v.engine_power() as f64
    ));
    vmethod!(b, "getEngineSpeed", |v, c| Value::Num(v.engine_speed()));
    vmethod!(b, "getMaxSpeed", |v, c| f(v.max_speed()));
    vmethod!(b, "getMass", |v, c| f(v.mass()));
    vmethod!(b, "getTransmissionNumber", |v, c| Value::Num(
        v.transmission_number() as f64
    ));
    vmethod!(b, "getCurrentSpeedKmHour", |v, c| f(v.current_speed_kmh()));
    vmethod!(b, "getCurrentSteering", |v, c| f(v.current_steering()));
    vmethod!(b, "getBrakeSpeedBetweenUpdate", |v, c| f(
        v.brake_speed_between_update()
    ));
    vmethod!(b, "isDoingOffroad", |v, c| Value::Bool(
        v.is_doing_offroad()
    ));
    vmethod!(b, "getOffroadEfficiency", |v, c| f(v.offroad_efficiency()));
    vmethod!(b, "getHeadlightsOn", |v, c| Value::Bool(v.headlights_on()));
    vmethod!(b, "setHeadlightsOn", |v, c| {
        v.set_headlights_on(bool_arg(c, 1));
        Value::Nil
    });
    vmethod!(b, "hasLightbar", |v, c| Value::Bool(v.has_lightbar()));
    vmethod!(b, "getLightbarLightsMode", |v, c| Value::Num(
        v.lightbar_lights_mode() as f64
    ));
    vmethod!(b, "setLightbarLightsMode", |v, c| {
        v.set_lightbar_lights_mode(int_arg(c, 1));
        Value::Nil
    });
    vmethod!(b, "getLightbarSirenMode", |v, c| Value::Num(
        v.lightbar_siren_mode() as f64
    ));
    vmethod!(b, "setLightbarSirenMode", |v, c| {
        v.set_lightbar_siren_mode(int_arg(c, 1));
        Value::Nil
    });
    vmethod!(b, "sirenShutoffTimeExpired", |v, c| Value::Bool(
        v.siren_shutoff_time_expired()
    ));
    vmethod!(b, "getDriver", |v, c| {
        // Characters are bound by the character port; the driver is only
        // tested for presence here.
        if v.driver().is_some() {
            Value::Bool(true)
        } else {
            Value::Nil
        }
    });
    vmethod!(b, "setNeedPartsUpdate", |v, c| {
        v.set_need_parts_update(bool_arg(c, 1));
        Value::Nil
    });
    vmethod!(b, "needPartsUpdate", |v, c| Value::Bool(
        v.need_parts_update()
    ));
    for name in [
        "transmitPartModData",
        "transmitPartUsedDelta",
        "transmitPartCondition",
        "transmitPartItem",
        "transmitPartDoor",
        "transmitPartWindow",
        "transmitEngine",
    ] {
        b.method(name, |c| Ok(c.push(Value::Nil)));
    }
    vmethod!(b, "updatePartStats", |v, c| {
        v.update_part_stats();
        Value::Nil
    });
    vmethod!(b, "updateBulletStats", |v, c| {
        v.update_bullet_stats();
        Value::Nil
    });
    vmethod!(b, "updateTotalMass", |v, c| {
        v.update_total_mass();
        Value::Nil
    });
    vmethod!(b, "getSquare", |v, c| Value::Nil);
    vmethod!(b, "windowsOpen", |v, c| Value::Num(v.windows_open() as f64));
    vmethod!(b, "setTireRemoved", |v, c| {
        v.set_tire_removed(int_arg(c, 1), bool_arg(c, 2));
        Value::Nil
    });
    vmethod!(b, "setTireInflation", |v, c| {
        v.set_tire_inflation(int_arg(c, 1), num_arg(c, 2) as f32);
        Value::Nil
    });
    vmethod!(b, "getAnimals", |v, c| Value::Object(Rc::new(
        LuaStringList(Vec::new())
    )));
    vmethod!(b, "isGoodCar", |v, c| Value::Bool(v.is_good_car()));
    vmethod!(b, "setGoodCar", |v, c| {
        v.set_good_car(bool_arg(c, 1));
        Value::Nil
    });
    vmethod!(b, "setEngineFeature", |v, c| {
        v.set_engine_feature(int_arg(c, 1), int_arg(c, 2), int_arg(c, 3));
        Value::Nil
    });
    vmethod!(b, "getVehicleType", |v, c| Value::str(&v.vehicle_type()));
    vmethod!(b, "getChoosenParts", |v, c| Value::Object(Rc::new(
        LuaChoosenParts(v.clone())
    )));
    vmethod!(b, "setAlarmed", |v, c| {
        v.set_alarmed(bool_arg(c, 1));
        Value::Nil
    });
    vmethod!(b, "isAlarmed", |v, c| Value::Bool(v.is_alarmed()));
    vmethod!(b, "getMaxPassengers", |v, c| Value::Num(
        v.max_passengers() as f64
    ));
    vmethod!(b, "getPassengerDoor", |v, c| v
        .passenger_door(int_arg(c, 1))
        .map_or(Value::Nil, |p| vlua::part_value(&p)));
    vmethod!(b, "getPassengerDoor2", |v, c| v
        .passenger_door2(int_arg(c, 1))
        .map_or(Value::Nil, |p| vlua::part_value(&p)));
    vmethod!(b, "getPassengerArea", |v, c| opt(
        v.passenger_area(int_arg(c, 1))
    ));
    vmethod!(b, "getKeyId", |v, c| Value::Num(v.key_id() as f64));
    vmethod!(b, "getMechanicalID", |v, c| Value::Num(
        v.st().mechanical_id as f64
    ));
    vmethod!(b, "isHotwired", |v, c| Value::Bool(v.is_hotwired()));
    vmethod!(b, "isKeysInIgnition", |v, c| Value::Bool(
        v.is_keys_in_ignition()
    ));
    vmethod!(b, "getAreaCenter", |v, c| {
        // Vector2 is bound by the iso port; a {x, y} table stands in.
        match str_arg(c, 1).and_then(|a| v.area_center(&a)) {
            Some(p) => {
                let t = TableRef::new();
                t.set_str("x", f(p.x));
                t.set_str("y", f(p.y));
                Value::Table(t)
            }
            None => Value::Nil,
        }
    });
    vmethod!(
        b,
        "getLightCount",
        |v, c| Value::Num(v.light_count() as f64)
    );
    vmethod!(b, "getLightByIndex", |v, c| v
        .light_by_index(int_arg(c, 1))
        .map_or(Value::Nil, |p| vlua::part_value(&p)));
    vmethod!(b, "getRust", |v, c| f(v.st().rust));
    vmethod!(b, "getColorHue", |v, c| f(v.st().color_hue));
    vmethod!(b, "getColorSaturation", |v, c| f(v.st().color_saturation));
    vmethod!(b, "getColorValue", |v, c| f(v.st().color_value));
    vmethod!(b, "getSkinIndex", |v, c| Value::Num(
        v.st().skin_index as f64
    ));
    vmethod!(b, "isBurnt", |v, c| Value::Bool(v.is_burnt()));
    vmethod!(b, "isSmashed", |v, c| Value::Bool(v.is_smashed()));
    vmethod!(b, "getRemainingFuelPercentage", |v, c| {
        f(v.gas_tank().map_or(0.0, |g| {
            g.container_content_amount() / g.container_capacity() as f32 * 100.0
        }))
    });
    vmethod!(b, "getPartForSeatContainer", |v, c| v
        .part_for_seat_container(int_arg(c, 1))
        .map_or(Value::Nil, |p| vlua::part_value(&p)));
    vmethod!(b, "isInArea", |v, c| {
        // isInArea(areaId, chr): characters are not bound here; a table
        // with x/y is accepted.
        let area = str_arg(c, 1);
        let pos = match c.arg(2) {
            Value::Table(t) => t.get_str("x").as_num().zip(t.get_str("y").as_num()),
            _ => None,
        };
        match (area, pos) {
            (Some(a), Some((x, y))) => Value::Bool(v.is_in_area(&a, x as f32, y as f32)),
            _ => Value::Bool(false),
        }
    });
    vmethod!(b, "repair", |v, c| {
        let parts = v.parts();
        run(c, || {
            for p in parts {
                p.repair();
            }
        });
        Value::Nil
    });
    vmethod!(b, "tryStartEngine", |v, c| {
        let k = bool_arg(c, 1);
        run(c, || v.try_start_engine(k));
        Value::Nil
    });
    vmethod!(b, "shutOff", |v, c| {
        v.shut_off(pz_vehicles::engine::EngineReason::TurnedOff);
        Value::Nil
    });
    vmethod!(b, "getUpVectorDot", |v, c| f(v.up_vector_dot()));
}

fn register_part(thread: &mut Thread) {
    let b = expose_class(thread, vlua::VEHICLE_PART_CLASS, None);
    pmethod!(b, "getVehicle", |p, c| vlua::vehicle_value(&p.vehicle()));
    pmethod!(b, "getId", |p, c| opt(p.id()));
    pmethod!(b, "getIndex", |p, c| {
        let v = p.vehicle();
        Value::Num(
            v.parts()
                .iter()
                .position(|x| Rc::ptr_eq(x, &p))
                .map_or(-1, |i| i as i32) as f64,
        )
    });
    pmethod!(b, "getArea", |p, c| opt(p.area()));
    pmethod!(b, "getCategory", |p, c| opt(p.st().category.clone()));
    pmethod!(b, "getItemType", |p, c| p
        .item_type()
        .map_or(Value::Nil, |t| Value::Object(Rc::new(LuaStringList(t)))));
    pmethod!(b, "getInventoryItem", |p, c| p
        .item()
        .map_or(Value::Nil, |i| vlua::item_value(&i)));
    pmethod!(b, "setInventoryItem", |p, c| {
        let item = vlua::item_from_value(&c.arg(1)).map(PartItem::Item);
        let skill = int_arg(c, 2);
        run(c, || p.set_inventory_item(item, skill));
        Value::Nil
    });
    pmethod!(b, "getCondition", |p, c| Value::Num(p.condition() as f64));
    pmethod!(b, "setCondition", |p, c| {
        let n = int_arg(c, 1);
        p.set_condition(n);
        Value::Nil
    });
    pmethod!(b, "damage", |p, c| {
        let n = int_arg(c, 1);
        p.damage(n);
        Value::Nil
    });
    pmethod!(b, "getModData", |p, c| Value::Table(p.mod_data()));
    pmethod!(b, "hasModData", |p, c| Value::Bool(p.has_mod_data()));
    pmethod!(b, "getLastUpdated", |p, c| f(p.st().last_updated));
    pmethod!(b, "isContainer", |p, c| Value::Bool(p.is_container()));
    pmethod!(b, "getContainerCapacity", |p, c| Value::Num(
        p.container_capacity() as f64
    ));
    pmethod!(b, "setContainerCapacity", |p, c| {
        p.set_container_capacity(int_arg(c, 1));
        Value::Nil
    });
    pmethod!(b, "getContainerContentAmount", |p, c| f(
        p.container_content_amount()
    ));
    pmethod!(b, "setContainerContentAmount", |p, c| {
        let amount = num_arg(c, 1) as f32;
        let (force, no_mass) = if c.nargs() > 2 {
            (bool_arg(c, 2), bool_arg(c, 3))
        } else {
            (false, false)
        };
        p.set_container_content_amount(amount, force, no_mass);
        Value::Nil
    });
    pmethod!(b, "getContainerContentType", |p, c| opt(
        p.container_content_type()
    ));
    pmethod!(b, "getContainerSeatNumber", |p, c| Value::Num(
        p.container_seat_number() as f64
    ));
    pmethod!(b, "isSeat", |p, c| Value::Bool(p.is_seat()));
    pmethod!(b, "isInventoryItemUninstalled", |p, c| Value::Bool(
        p.is_inventory_item_uninstalled()
    ));
    pmethod!(b, "getLuaFunction", |p, c| opt(
        str_arg(c, 1).and_then(|n| p.lua_function(&n))
    ));
    pmethod!(b, "getTable", |p, c| str_arg(c, 1)
        .and_then(|n| p.table(&n))
        .map_or(Value::Nil, Value::Table));
    pmethod!(b, "getDoor", |p, c| vlua::door_value(&p));
    pmethod!(b, "getWindow", |p, c| vlua::window_value(&p));
    pmethod!(b, "getLight", |p, c| vlua::light_value(&p));
    pmethod!(b, "getVehicleEngine", |p, c| vlua::engine_value(&p));
    pmethod!(b, "getChildWindow", |p, c| p
        .child_window()
        .map_or(Value::Nil, |w| vlua::part_value(&w)));
    pmethod!(b, "getParent", |p, c| p
        .parent()
        .map_or(Value::Nil, |w| vlua::part_value(&w)));
    pmethod!(
        b,
        "getChildCount",
        |p, c| Value::Num(p.child_count() as f64)
    );
    pmethod!(b, "getChild", |p, c| p
        .child(int_arg(c, 1).max(0) as usize)
        .map_or(Value::Nil, |w| vlua::part_value(&w)));
    pmethod!(
        b,
        "getWheelIndex",
        |p, c| Value::Num(p.wheel_index() as f64)
    );
    pmethod!(b, "createSpotLight", |p, c| {
        p.create_spot_light(
            num_arg(c, 1) as f32,
            num_arg(c, 2) as f32,
            num_arg(c, 3) as f32,
            num_arg(c, 4) as f32,
            num_arg(c, 5) as f32,
            int_arg(c, 6),
        );
        Value::Nil
    });
    pmethod!(b, "createSpotLightColor", |p, c| {
        p.create_spot_light_color(
            num_arg(c, 1) as f32,
            num_arg(c, 2) as f32,
            num_arg(c, 3) as f32,
            num_arg(c, 4) as f32,
            num_arg(c, 5) as f32,
            int_arg(c, 6),
            num_arg(c, 7) as f32,
            num_arg(c, 8) as f32,
            num_arg(c, 9) as f32,
        );
        Value::Nil
    });
    pmethod!(b, "setLightActive", |p, c| {
        p.set_light_active(bool_arg(c, 1));
        Value::Nil
    });
    pmethod!(b, "getLightDistance", |p, c| f(p.light_distance()));
    pmethod!(b, "getLightIntensity", |p, c| f(p.light_intensity()));
    pmethod!(b, "getLightFocusing", |p, c| f(p.light_focusing()));
    pmethod!(b, "getDeviceData", |p, c| {
        if p.st().device_data.is_some() {
            Value::Object(Rc::new(LuaDeviceData(DeviceOwner::Part(p.clone()))))
        } else {
            Value::Nil
        }
    });
    pmethod!(b, "createSignalDevice", |p, c| {
        p.create_signal_device();
        Value::Object(Rc::new(LuaDeviceData(DeviceOwner::Part(p.clone()))))
    });
    pmethod!(b, "setRandomCondition", |p, c| {
        let item = vlua::item_from_value(&c.arg(1));
        run(c, || p.set_random_condition(item.as_ref()));
        Value::Nil
    });
    pmethod!(b, "setGeneralCondition", |p, c| {
        let item = vlua::item_from_value(&c.arg(1));
        let (q, ch) = (num_arg(c, 2) as f32, num_arg(c, 3) as f32);
        p.set_general_condition(item.as_ref(), q, ch);
        Value::Nil
    });
    pmethod!(b, "isSpecificItem", |p, c| Value::Bool(
        p.st().specific_item
    ));
    pmethod!(b, "getWheelFriction", |p, c| f(p.st().wheel_friction));
    pmethod!(b, "getSuspensionDamping", |p, c| f(p
        .st()
        .suspension_damping));
    pmethod!(b, "getSuspensionCompression", |p, c| f(p
        .st()
        .suspension_compression));
    pmethod!(b, "getEngineLoudness", |p, c| f(p.st().engine_loudness));
    pmethod!(b, "getMechanicSkillInstaller", |p, c| Value::Num(
        p.st().mechanic_skill_installer as f64
    ));
    pmethod!(b, "getDurability", |p, c| f(p.st().durability));
    pmethod!(b, "setModelVisible", |p, c| {
        if let Some(id) = str_arg(c, 1) {
            p.set_model_visible(&id, bool_arg(c, 2));
        }
        Value::Nil
    });
    pmethod!(b, "repair", |p, c| {
        run(c, || p.repair());
        Value::Nil
    });
    pmethod!(b, "getItemContainer", |p, c| {
        // Containers are bound by the inventory port; a {isEmpty} view.
        match &p.st().container {
            Some(cont) => Value::Object(Rc::new(LuaStringList(
                cont.items.iter().map(|_| String::new()).collect(),
            ))),
            None => Value::Nil,
        }
    });
    let _ = part_arg;
}

fn register_door_window_light(thread: &mut Thread) {
    let b = expose_class(thread, vlua::VEHICLE_DOOR_CLASS, None);
    macro_rules! door {
        ($name:literal, |$d:ident, $c:ident| $body:expr) => {
            b.method($name, |$c| {
                let p = with_this::<LuaDoor, _>($c, |d| d.0.clone())?;
                let r: Value = {
                    let mut st = p.st_mut();
                    let $d = st.door.as_mut().unwrap();
                    $body
                };
                Ok($c.push(r))
            });
        };
    }
    door!("isOpen", |d, c| Value::Bool(d.open));
    door!("isLocked", |d, c| Value::Bool(d.locked));
    door!("isLockBroken", |d, c| Value::Bool(d.lock_broken));
    door!("setOpen", |d, c| {
        d.open = bool_arg(c, 1);
        Value::Nil
    });
    door!("setLocked", |d, c| {
        d.locked = bool_arg(c, 1);
        Value::Nil
    });
    door!("setLockBroken", |d, c| {
        d.lock_broken = bool_arg(c, 1);
        Value::Nil
    });

    let b = expose_class(thread, vlua::VEHICLE_WINDOW_CLASS, None);
    b.method("isOpen", |c| {
        let p = with_this::<LuaWindow, _>(c, |d| d.0.clone())?;
        let r = p.st().window.is_some_and(|w| w.open);
        Ok(c.push(r))
    });
    b.method("isOpenable", |c| {
        let p = with_this::<LuaWindow, _>(c, |d| d.0.clone())?;
        let r = p.st().window.is_some_and(|w| w.openable);
        Ok(c.push(r))
    });
    b.method("setOpen", |c| {
        let p = with_this::<LuaWindow, _>(c, |d| d.0.clone())?;
        p.set_window_open(bool_arg(c, 1));
        Ok(0)
    });
    b.method("getHealth", |c| {
        let p = with_this::<LuaWindow, _>(c, |d| d.0.clone())?;
        Ok(c.push(p.condition()))
    });
    b.method("isDestroyed", |c| {
        let p = with_this::<LuaWindow, _>(c, |d| d.0.clone())?;
        Ok(c.push(p.condition() == 0))
    });
    b.method("isHittable", |c| {
        let p = with_this::<LuaWindow, _>(c, |d| d.0.clone())?;
        Ok(c.push(p.window_is_hittable()))
    });
    b.method("damage", |c| {
        let p = with_this::<LuaWindow, _>(c, |d| d.0.clone())?;
        p.window_damage(int_arg(c, 1));
        Ok(0)
    });
    b.method("getPart", |c| {
        let p = with_this::<LuaWindow, _>(c, |d| d.0.clone())?;
        Ok(c.push(vlua::part_value(&p)))
    });

    let b = expose_class(thread, vlua::VEHICLE_LIGHT_CLASS, None);
    b.method("getActive", |c| {
        let p = with_this::<LuaLight, _>(c, |d| d.0.clone())?;
        let r = p.st().light.as_ref().is_some_and(|l| l.active);
        Ok(c.push(r))
    });
    b.method("setActive", |c| {
        let p = with_this::<LuaLight, _>(c, |d| d.0.clone())?;
        if let Some(l) = p.st_mut().light.as_mut() {
            l.active = bool_arg(c, 1);
        }
        Ok(0)
    });
    b.method("getIntensity", |c| {
        let p = with_this::<LuaLight, _>(c, |d| d.0.clone())?;
        let r = p.st().light.as_ref().map_or(0.0, |l| l.intensity);
        Ok(c.push(r as f64))
    });
    b.method("getFocusing", |c| {
        let p = with_this::<LuaLight, _>(c, |d| d.0.clone())?;
        let r = p.st().light.as_ref().map_or(0, |l| l.focusing);
        Ok(c.push(r))
    });

    let b = expose_class(thread, vlua::VEHICLE_ENGINE_CLASS, None);
    b.method("getState", |c| {
        let p = with_this::<LuaEngine, _>(c, |d| d.0.clone())?;
        let r = p
            .st()
            .engine
            .as_ref()
            .map(|e| e.state.name())
            .unwrap_or("Idle");
        Ok(c.push(r))
    });
    b.method("isRunning", |c| {
        let p = with_this::<LuaEngine, _>(c, |d| d.0.clone())?;
        let r = p.st().engine.as_ref().is_some_and(|e| e.is_running());
        Ok(c.push(r))
    });
}

fn register_script(thread: &mut Thread) {
    let b = expose_class(thread, vlua::VEHICLE_SCRIPT_CLASS, None);
    macro_rules! smethod {
        ($name:literal, |$s:ident| $body:expr) => {
            b.method($name, |c| {
                let s = with_this::<LuaVehicleScript, _>(c, |s| s.0.clone())?;
                let $s = s.borrow();
                let r: Value = $body;
                Ok(c.push(r))
            });
        };
    }
    smethod!("getName", |s| opt(s.name().map(str::to_string)));
    smethod!("getFullName", |s| Value::str(&s.full_name()));
    smethod!("getMass", |s| f(s.mass()));
    smethod!("getGearRatioCount", |s| Value::Num(
        s.gear_ratio_count() as f64
    ));
    smethod!("getEngineForce", |s| f(s.engine_force()));
    smethod!("getEngineLoudness", |s| Value::Num(
        s.engine_loudness() as f64
    ));
    smethod!(
        "getEngineQuality",
        |s| Value::Num(s.engine_quality() as f64)
    );
    smethod!("getEngineIdleSpeed", |s| f(s.engine_idle_speed()));
    smethod!("getMaxSpeed", |s| f(s.max_speed));
    smethod!("getOffroadEfficiency", |s| f(s.offroad_efficiency()));
    smethod!("getPartCount", |s| Value::Num(s.part_count() as f64));
    smethod!("getPassengerCount", |s| Value::Num(
        s.passenger_count() as f64
    ));
    smethod!("getWheelCount", |s| Value::Num(s.wheel_count() as f64));
    smethod!("getAreaCount", |s| Value::Num(s.area_count() as f64));
    smethod!("getSeats", |s| Value::Num(s.seats() as f64));
    smethod!("getMechanicType", |s| Value::Num(s.mechanic_type() as f64));
    smethod!("getEngineRepairLevel", |s| Value::Num(
        s.engine_repair_level() as f64
    ));
    smethod!("getPlayerDamageProtection", |s| f(
        s.player_damage_protection()
    ));
    smethod!("getCarModelName", |s| opt(s
        .car_model_name()
        .map(str::to_string)));
    smethod!("getCarMechanicsOverlay", |s| opt(s
        .car_mechanics_overlay()
        .map(str::to_string)));
    smethod!("getStorageCapacity", |s| Value::Num(
        s.storage_capacity() as f64
    ));
    smethod!("getFrontEndHealth", |s| Value::Num(
        s.front_end_health() as f64
    ));
    smethod!("getRearEndHealth", |s| Value::Num(
        s.rear_end_health() as f64
    ));
    smethod!("getHeadlightConfigLevel", |s| Value::Num(
        s.headlight_config_level() as f64
    ));
}

fn register_items(thread: &mut Thread) {
    let b = expose_class(thread, INVENTORY_ITEM_CLASS, None);
    macro_rules! imethod {
        ($name:literal, |$i:ident, $c:ident| $body:expr) => {
            b.method($name, |$c| {
                let item = with_this::<LuaVehItem, _>($c, |i| i.item.clone())?;
                let $i = &item;
                let r: Value = $body;
                Ok($c.push(r))
            });
        };
    }
    imethod!("getCondition", |i, c| Value::Num(
        i.borrow().condition as f64
    ));
    imethod!("getConditionMax", |i, c| Value::Num(
        i.borrow().condition_max as f64
    ));
    imethod!("setCondition", |i, c| {
        items::set_condition(&mut i.borrow_mut(), int_arg(c, 1));
        Value::Nil
    });
    imethod!("setConditionNoSound", |i, c| {
        items::set_condition(&mut i.borrow_mut(), int_arg(c, 1));
        Value::Nil
    });
    imethod!("setConditionMax", |i, c| {
        i.borrow_mut().condition_max = int_arg(c, 1);
        Value::Nil
    });
    imethod!("isBroken", |i, c| Value::Bool(i.borrow().condition <= 0));
    imethod!("getMaxCapacity", |i, c| Value::Num(
        i.borrow().max_capacity as f64
    ));
    imethod!("setMaxCapacity", |i, c| {
        i.borrow_mut().max_capacity = int_arg(c, 1);
        Value::Nil
    });
    imethod!("getItemCapacity", |i, c| f(i.borrow().item_capacity));
    imethod!("setItemCapacity", |i, c| {
        i.borrow_mut().item_capacity = num_arg(c, 1) as f32;
        Value::Nil
    });
    imethod!("getCurrentUsesFloat", |i, c| f(items::current_uses_float(
        &i.borrow()
    )));
    imethod!("setCurrentUsesFloat", |i, c| {
        items::set_current_uses_float(&mut i.borrow_mut(), num_arg(c, 1) as f32);
        Value::Nil
    });
    imethod!("setUsedDelta", |i, c| {
        items::set_current_uses_float(&mut i.borrow_mut(), num_arg(c, 1) as f32);
        Value::Nil
    });
    imethod!("getUseDelta", |i, c| f(items::use_delta(&i.borrow())));
    imethod!("getConditionLowerNormal", |i, c| f(items::stats(
        &i.borrow()
    )
    .condition_lower_normal));
    imethod!("getConditionLowerOffroad", |i, c| f(items::stats(
        &i.borrow()
    )
    .condition_lower_offroad));
    imethod!("getBrakeForce", |i, c| f(
        items::stats(&i.borrow()).brake_force
    ));
    imethod!("getWheelFriction", |i, c| f(
        items::stats(&i.borrow()).wheel_friction
    ));
    imethod!("getChanceToSpawnDamaged", |i, c| Value::Num(
        items::stats(&i.borrow()).chance_to_spawn_damaged as f64
    ));
    imethod!("getWeight", |i, c| f(items::weight(&i.borrow())));
    imethod!("getFullType", |i, c| Value::str(
        &i.borrow().get_full_type()
    ));
    imethod!("getType", |i, c| Value::str(i.borrow().get_type()));
    imethod!("getName", |i, c| Value::str(i.borrow().get_name()));
    imethod!("getID", |i, c| Value::Num(i.borrow().id as f64));
    imethod!("getDeviceData", |i, c| {
        if matches!(i.borrow().kind, ItemKind::Radio(..)) {
            Value::Object(Rc::new(LuaDeviceData(DeviceOwner::Item(i.clone()))))
        } else {
            Value::Nil
        }
    });
    let d = expose_class(thread, DRAINABLE_CLASS, Some(INVENTORY_ITEM_CLASS));
    let _ = d;

    let b = expose_class(thread, DEVICE_DATA_CLASS, None);
    macro_rules! dget {
        ($name:literal, |$d:ident| $body:expr) => {
            b.method($name, |c| {
                let r = with_this::<LuaDeviceData, _>(c, |dd| dd.with(|$d| -> Value { $body }))?;
                Ok(c.push(r.unwrap_or_default()))
            });
        };
    }
    macro_rules! dset {
        ($name:literal, |$d:ident, $c:ident| $body:expr) => {
            b.method($name, |$c| {
                let owner = with_this::<LuaDeviceData, _>($c, |dd| match &dd.0 {
                    DeviceOwner::Part(p) => DeviceOwner::Part(p.clone()),
                    DeviceOwner::Item(i) => DeviceOwner::Item(i.clone()),
                })?;
                let a = $c.arg(1);
                let dd = LuaDeviceData(owner);
                dd.with(|$d| {
                    let $c = &a;
                    $body
                });
                Ok(0)
            });
        };
    }
    let n = |v: &Value| pz_kahlua::exec::raw_tonumber(v).unwrap_or(0.0);
    let _ = n;
    dget!("getDeviceName", |d| Value::str(&d.device_name));
    dget!("getIsTwoWay", |d| Value::Bool(d.two_way));
    dget!("getTransmitRange", |d| Value::Num(d.transmit_range as f64));
    dget!("getMicRange", |d| Value::Num(d.mic_range as f64));
    dget!("getBaseVolumeRange", |d| f(d.base_volume_range));
    dget!("getIsTelevision", |d| Value::Bool(d.is_television));
    dget!("getMinChannelRange", |d| Value::Num(
        d.min_channel_range as f64
    ));
    dget!("getMaxChannelRange", |d| Value::Num(
        d.max_channel_range as f64
    ));
    dget!("getIsHighTier", |d| Value::Bool(d.is_high_tier));
    dget!("getUseDelta", |d| f(d.use_delta));
    dget!("getMediaType", |d| Value::Num(d.media_type as f64));
    dget!("getIsTurnedOn", |d| Value::Bool(d.is_turned_on));
    dget!("getChannel", |d| Value::Num(d.channel as f64));
    dget!("getIsPortable", |d| Value::Bool(d.is_portable));
    dget!("getIsBatteryPowered", |d| Value::Bool(d.is_battery_powered));
    let num = |v: &Value| pz_kahlua::exec::raw_tonumber(v).unwrap_or(0.0);
    let _ = num;
    dset!("setDeviceName", |d, a| d.device_name = match a {
        Value::Str(s) => s.to_string(),
        _ => String::new(),
    });
    dset!("setIsTwoWay", |d, a| d.two_way =
        matches!(a, Value::Bool(true)));
    dset!("setTransmitRange", |d, a| {
        let r = pz_kahlua::exec::raw_tonumber(a).unwrap_or(0.0) as i32;
        d.transmit_range = if r > 0 { r } else { 0 };
    });
    dset!("setMicRange", |d, a| d.mic_range =
        pz_kahlua::exec::raw_tonumber(a).unwrap_or(0.0) as i32);
    dset!("setBaseVolumeRange", |d, a| d.base_volume_range =
        pz_kahlua::exec::raw_tonumber(a).unwrap_or(0.0) as f32);
    dset!("setIsPortable", |d, a| d.is_portable =
        matches!(a, Value::Bool(true)));
    dset!("setIsTelevision", |d, a| d.is_television =
        matches!(a, Value::Bool(true)));
    dset!("setMinChannelRange", |d, a| {
        let i = pz_kahlua::exec::raw_tonumber(a).unwrap_or(0.0) as i32;
        d.min_channel_range = if (200..=1_000_000).contains(&i) {
            i
        } else {
            200
        };
    });
    dset!("setMaxChannelRange", |d, a| {
        let i = pz_kahlua::exec::raw_tonumber(a).unwrap_or(0.0) as i32;
        d.max_channel_range = if (200..=1_000_000).contains(&i) {
            i
        } else {
            1_000_000
        };
    });
    dset!("setIsBatteryPowered", |d, a| d.is_battery_powered =
        matches!(a, Value::Bool(true)));
    dset!("setIsHighTier", |d, a| d.is_high_tier =
        matches!(a, Value::Bool(true)));
    dset!("setUseDelta", |d, a| d.use_delta =
        pz_kahlua::exec::raw_tonumber(a).unwrap_or(0.0) as f32 / 60.0);
    dset!("setMediaType", |d, a| d.media_type =
        pz_kahlua::exec::raw_tonumber(a).unwrap_or(0.0) as i8);
    dset!("setIsTurnedOn", |d, a| d.is_turned_on =
        matches!(a, Value::Bool(true)));
    dset!("setChannel", |d, a| d.channel =
        pz_kahlua::exec::raw_tonumber(a).unwrap_or(0.0) as i32);
}

fn register_collections(thread: &mut Thread) {
    let b = expose_class(thread, ARRAY_LIST_CLASS, None);
    b.method("size", |c| {
        let n = with_this::<LuaStringList, _>(c, |l| l.0.len())?;
        Ok(c.push(n as f64))
    });
    b.method("isEmpty", |c| {
        let n = with_this::<LuaStringList, _>(c, |l| l.0.is_empty())?;
        Ok(c.push(n))
    });
    b.method("get", |c| {
        let i = int_arg(c, 1);
        let s = with_this::<LuaStringList, _>(c, |l| l.0.get(i.max(0) as usize).cloned())?;
        Ok(c.push(opt(s)))
    });
    b.method("contains", |c| {
        let x = str_arg(c, 1);
        let r =
            with_this::<LuaStringList, _>(c, |l| x.is_some_and(|x| l.0.iter().any(|s| *s == *x)))?;
        Ok(c.push(r))
    });

    let b = expose_class(thread, HASH_MAP_CLASS, None);
    b.method("get", |c| {
        let k = str_arg(c, 1);
        let v = with_this::<LuaChoosenParts, _>(c, |m| m.0.clone())?;
        let r = k.and_then(|k| {
            v.st()
                .choosen_parts
                .iter()
                .find(|(a, _)| *a == *k)
                .map(|(_, b)| b.clone())
        });
        Ok(c.push(opt(r)))
    });
    b.method("put", |c| {
        let (k, val) = (str_arg(c, 1), str_arg(c, 2));
        let v = with_this::<LuaChoosenParts, _>(c, |m| m.0.clone())?;
        if let (Some(k), Some(val)) = (k, val) {
            let mut st = v.st_mut();
            match st.choosen_parts.iter_mut().find(|(a, _)| *a == *k) {
                Some(e) => e.1 = val.to_string(),
                None => st.choosen_parts.push((k.to_string(), val.to_string())),
            }
        }
        Ok(0)
    });
}

/// Registers the vehicle classes and installs the item conversions the
/// vehicle runtime uses.
pub fn register(thread: &mut Thread) {
    register_vehicle(thread);
    register_part(thread);
    register_door_window_light(thread);
    register_script(thread);
    register_items(thread);
    register_collections(thread);
    vlua::install_item_conversions(
        Rc::new(|i: &ItemRef| item_value(i)),
        Rc::new(|v: &Value| item_of(v)),
    );
    let _: Option<&BaseVehicle> = None;
}
