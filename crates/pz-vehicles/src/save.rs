//! The runtime vehicle from and to pz-world's saved `BaseVehicle`
//! (`BaseVehicle.load` / `save` applied to the runtime state).

use crate::engine::VehicleEngine;
use crate::env;
use crate::jmath::{Quaternionf, Vector3f};
use crate::part::{PartItem, VehicleDoor, VehicleLight, VehiclePart, VehicleWindow};
use crate::vehicle::{BaseVehicle, VehicleRef};
use pz_inventory::lua_table::{LuaKey, LuaTable, LuaValue};
use pz_inventory::SavedItem;
use pz_kahlua::{TableRef, Value};
use pz_world::save::vehicle as saved;
use std::cell::RefCell;
use std::rc::Rc;

/// A saved mod-data table as a Lua table.
pub fn mod_data_to_lua(t: &LuaTable) -> TableRef {
    let out = TableRef::new();
    for (k, v) in &t.entries {
        let key = match k {
            LuaKey::Str(s) => Value::str(s),
            LuaKey::Num(n) => Value::Num(*n),
        };
        let val = match v {
            LuaValue::Str(s) => Value::str(s),
            LuaValue::Num(n) => Value::Num(*n),
            LuaValue::Bool(b) => Value::Bool(*b),
            LuaValue::Table(t) => Value::Table(mod_data_to_lua(t)),
            LuaValue::Unsaveable => continue,
        };
        out.rawset(key, val);
    }
    out
}

/// A Lua table as saved mod data (values the format cannot hold are
/// `Unsaveable`).
pub fn lua_to_mod_data(t: &TableRef) -> LuaTable {
    let mut out = LuaTable::default();
    for (k, v) in t.entries() {
        let key = match &k {
            Value::Str(s) => LuaKey::Str(s.to_string()),
            Value::Num(n) => LuaKey::Num(*n),
            _ => continue,
        };
        let val = match &v {
            Value::Str(s) => LuaValue::Str(s.to_string()),
            Value::Num(n) => LuaValue::Num(*n),
            Value::Bool(b) => LuaValue::Bool(*b),
            Value::Table(t) => LuaValue::Table(lua_to_mod_data(t)),
            _ => LuaValue::Unsaveable,
        };
        out.entries.push((key, val));
    }
    out
}

fn part_item(i: &SavedItem) -> PartItem {
    match i {
        SavedItem::Item(item) => PartItem::Item(Rc::new(RefCell::new((**item).clone()))),
        raw => PartItem::Raw(raw.clone()),
    }
}

/// `new BaseVehicle(cell)` + `load`: a runtime vehicle with the saved
/// state (not yet in the world; `addToWorld` attaches the script).
pub fn from_save(s: &saved::BaseVehicle) -> VehicleRef {
    from_save_with(s, || {})
}

/// [`from_save`] running `loading` between the constructor and the load
/// (where the original's `InventoryItem.loadItem` creates the saved items,
/// drawing their ids from `Rand`).
pub fn from_save_with(s: &saved::BaseVehicle, loading: impl FnOnce()) -> VehicleRef {
    let v = BaseVehicle::new();
    loading();
    {
        let (ox, oy) = env::offset();
        let mut st = v.st_mut();
        st.x = s.base.x;
        st.y = s.base.y;
        st.z = s.base.z;
        st.saved_physics_z = s.saved_physics_z;
        let q = s.saved_rot;
        st.saved_rot = Quaternionf::new(q[0], q[1], q[2], q[3]);
        let py = if st.saved_physics_z.is_nan() {
            st.z
        } else {
            st.saved_physics_z
        };
        st.jni_transform.origin = Vector3f::new(st.x - ox, py, st.y - oy);
        let r = st.saved_rot;
        st.jni_transform.set_rotation(&r);
        st.script_name = s.script_name.clone();
        st.skin_index = s.skin_index;
        st.front_end_durability = s.front_end_durability;
        st.rear_end_durability = s.rear_end_durability;
        st.current_front_end_durability = s.current_front_end_durability;
        st.current_rear_end_durability = s.current_rear_end_durability;
        st.key_id = s.key_id;
        st.key_spawned = s.key_spawned;
        st.headlights_on = s.headlights_on;
        st.created = s.created;
        st.sound_horn_on = s.sound_horn_on;
        st.sound_back_move_on = s.sound_back_move_on;
        st.lightbar_lights_mode.set(s.lightbar_lights_mode);
        st.lightbar_siren_mode = s.lightbar_siren_mode;
        st.iso_object = Some(s.base.clone());
    }
    for p in &s.parts {
        let part = VehiclePart::new(&v);
        {
            let mut ps = part.st_mut();
            ps.part_id = Some(p.part_id.clone());
            ps.created = p.created;
            ps.last_updated = p.last_updated;
            ps.item = p.item.as_ref().map(part_item);
            if let Some(c) = &p.container {
                let mut c = c.clone();
                c.id = 0;
                ps.container = Some(c);
            }
            ps.mod_data = p.mod_data.as_ref().map(mod_data_to_lua);
            ps.device_data = p.device_data.clone();
            ps.light = p.light.as_ref().map(|l| VehicleLight {
                active: l.active,
                offset: Vector3f::new(l.offset_x, l.offset_y, 0.0),
                intensity: l.intensity,
                dist: l.dist,
                focusing: l.focusing,
                ..Default::default()
            });
            ps.door = p.door.as_ref().map(|d| VehicleDoor {
                open: d.open,
                locked: d.locked,
                lock_broken: d.lock_broken,
            });
            if p.part_id == "Engine" {
                ps.engine = Some(VehicleEngine::default());
            }
            ps.window = p.window.as_ref().map(|w| VehicleWindow {
                openable: false,
                open: w.open,
                open_delta: w.open_delta,
            });
            ps.condition = p.condition;
            ps.wheel_friction = p.wheel_friction;
            ps.mechanic_skill_installer = p.mechanic_skill_installer;
            ps.suspension_compression = p.suspension_compression;
            ps.suspension_damping = p.suspension_damping;
            ps.components = p.components.clone();
        }
        v.add_part(part);
    }
    {
        let mut st = v.st_mut();
        st.key_is_on_door = s.key_is_on_door;
        st.hotwired = s.hotwired;
        st.hotwired_broken = s.hotwired_broken;
        st.keys_in_ignition = s.keys_in_ignition;
        st.rust = s.rust;
        st.color_hue = s.color_hue;
        st.color_saturation = s.color_saturation;
        st.color_value = s.color_value;
    }
    if let Some(e) = &s.engine {
        if v.engine_part().is_some() {
            v.with_engine_mut(|en| en.load(e.running, e.loudness, e.quality, e.power));
        }
    }
    {
        let mut st = v.st_mut();
        st.mechanical_id = s.mechanical_id;
        st.alarmed = s.alarmed;
        st.alarm.start_time = s.alarm_start_time;
        st.alarm.chosen_sound = s.alarm_chosen_sound.clone();
        st.siren_start_time = s.siren_start_time;
    }
    if let Some(SavedItem::Item(k)) = &s.current_key {
        v.set_current_key(Rc::new(RefCell::new((**k).clone())));
    }
    {
        let mut st = v.st_mut();
        st.blood_intensity = s.blood_intensity.clone();
        if let Some(t) = &s.towing {
            st.vehicle_towing_id = t.vehicle_id;
            st.tow_attachment_self = Some(t.attachment_self.clone());
            st.tow_attachment_other = Some(t.attachment_other.clone());
            st.row_constraint_z_offset = t.z_offset;
        }
        st.regulator_speed = s.regulator_speed;
        st.previously_entered = s.previously_entered;
        st.previously_moved = s.previously_moved;
        st.animals = s.animals.clone();
        st.loaded = true;
    }
    v
}

/// The saved state of a runtime part (`VehiclePart.save`'s fields).
pub fn part_to_save(p: &VehiclePart) -> saved::VehiclePart {
    let ps = p.st();
    let mut sp = saved::VehiclePart::new();
    sp.part_id = p.id().unwrap_or_default();
    sp.created = ps.created;
    sp.last_updated = ps.last_updated;
    sp.item = ps.item.as_ref().map(|i| match i {
        PartItem::Item(i) => SavedItem::Item(Box::new(i.borrow().clone())),
        PartItem::Raw(r) => r.clone(),
    });
    sp.container = ps.container.clone();
    sp.mod_data = ps.mod_data.as_ref().map(lua_to_mod_data);
    sp.device_data = ps.device_data.clone();
    sp.light = ps.light.as_ref().map(|l| saved::VehicleLight {
        active: l.active,
        offset_x: l.offset.x,
        offset_y: l.offset.y,
        intensity: l.intensity,
        dist: l.dist,
        focusing: l.focusing,
    });
    sp.door = ps.door.map(|d| saved::VehicleDoor {
        open: d.open,
        locked: d.locked,
        lock_broken: d.lock_broken,
    });
    sp.window = ps.window.map(|w| saved::VehicleWindow {
        open: w.open,
        open_delta: w.open_delta,
    });
    sp.condition = ps.condition;
    sp.wheel_friction = ps.wheel_friction;
    sp.mechanic_skill_installer = ps.mechanic_skill_installer;
    sp.suspension_compression = ps.suspension_compression;
    sp.suspension_damping = ps.suspension_damping;
    sp.components = ps.components.clone();
    sp
}

/// The saved state of a runtime vehicle (`BaseVehicle.save`'s fields).
pub fn to_save(v: &BaseVehicle, base: &pz_world::save::visual::MovingBase) -> saved::BaseVehicle {
    let parts: Vec<saved::VehiclePart> = v.parts().iter().map(|p| part_to_save(p)).collect();
    let engine = v.engine_part().map(|_| {
        v.with_engine(|e| saved::VehicleEngine {
            running: e.is_running(),
            loudness: e.loudness,
            quality: e.quality,
            power: e.power,
        })
    });
    let st = v.st();
    let q = st.jni_transform.rotation();
    let mut base = base.clone();
    base.x = st.x;
    base.y = st.y;
    base.z = st.z;
    saved::BaseVehicle {
        base,
        saved_physics_z: st.jni_transform.origin.y,
        saved_rot: [q.x, q.y, q.z, q.w],
        script_name: st.script_name.clone(),
        skin_index: st.skin_index,
        engine,
        front_end_durability: st.front_end_durability,
        rear_end_durability: st.rear_end_durability,
        current_front_end_durability: st.current_front_end_durability,
        current_rear_end_durability: st.current_rear_end_durability,
        key_id: st.key_id,
        key_spawned: st.key_spawned,
        headlights_on: st.headlights_on,
        created: st.created,
        sound_horn_on: st.sound_horn_on,
        sound_back_move_on: st.sound_back_move_on,
        lightbar_lights_mode: st.lightbar_lights_mode.mode,
        lightbar_siren_mode: st.lightbar_siren_mode,
        parts,
        key_is_on_door: st.key_is_on_door,
        hotwired: st.hotwired,
        hotwired_broken: st.hotwired_broken,
        keys_in_ignition: st.keys_in_ignition,
        rust: st.rust,
        color_hue: st.color_hue,
        color_saturation: st.color_saturation,
        color_value: st.color_value,
        vehicle_id: st.vehicle_id,
        loaded_vehicle_id: st.vehicle_id,
        mechanical_id: st.mechanical_id,
        alarmed: st.alarmed,
        alarm_start_time: st.alarm.start_time,
        alarm_chosen_sound: st.alarm.chosen_sound.clone(),
        siren_start_time: st.siren_start_time,
        current_key: st
            .ignition_switch
            .first()
            .map(|k| SavedItem::Item(Box::new(k.borrow().clone()))),
        blood_intensity: st.blood_intensity.clone(),
        towing: (st.vehicle_towing_id != -1).then(|| saved::VehicleTowing {
            vehicle_id: st.vehicle_towing_id,
            attachment_self: st.tow_attachment_self.clone().unwrap_or_default(),
            attachment_other: st.tow_attachment_other.clone().unwrap_or_default(),
            z_offset: st.row_constraint_z_offset,
        }),
        regulator_speed: st.regulator_speed,
        previously_entered: st.previously_entered,
        previously_moved: st.previously_moved,
        animals: st.animals.clone(),
    }
}
