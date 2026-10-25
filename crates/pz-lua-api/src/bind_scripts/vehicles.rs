//! `VehicleScript` and the objects inside it Lua reaches (`Part`, `Model`,
//! `Area`, `PhysicsShape`, `Passenger`, `Position`, `Wheel`, the model
//! attachments).
//!
//! Not exposed yet: `getSkin`/`getTextures`, `getSounds`, `getLightbar`,
//! `Part.getAnimById`, `Passenger.getSwitchSeatById`/`makeCopy` (classes not
//! exposed), `getCrawlThroughWheel`, `globMatch`, `getRandomSpecialKeyRing`
//! (TODO(ItemKey)). Attachments added from Lua are copied into the script.

use super::models::{attachment_arg, vec2, vec3, MODEL_ATTACHMENT_CLASS};
use super::{get, get_obj, method, opt_str, opt_string_list, set, void, IntoLua, ScriptHandle};
use crate::bind_registry::java_error;
use crate::expose::{expose_class, int_arg, num_arg};
use pz_kahlua::{LuaError, NativeCall, Thread, Value};
use pz_scripts::objects::vehicles::{
    Area, Model, Part, Passenger, PhysicsShape, Position, VehicleScript, Wheel,
};
use pz_scripts::ScriptManager;
use std::rc::Rc;

pub const VEHICLE_SCRIPT_CLASS: &str = "zombie.scripting.objects.VehicleScript";
pub const PART_CLASS: &str = "zombie.scripting.objects.VehicleScript$Part";
pub const MODEL_CLASS: &str = "zombie.scripting.objects.VehicleScript$Model";
pub const AREA_CLASS: &str = "zombie.scripting.objects.VehicleScript$Area";
pub const PHYSICS_SHAPE_CLASS: &str = "zombie.scripting.objects.VehicleScript$PhysicsShape";
pub const PASSENGER_CLASS: &str = "zombie.scripting.objects.VehicleScript$Passenger";
pub const POSITION_CLASS: &str = "zombie.scripting.objects.VehicleScript$Position";
pub const WHEEL_CLASS: &str = "zombie.scripting.objects.VehicleScript$Wheel";

fn some_if(found: bool, v: impl FnOnce() -> Value) -> Value {
    if found {
        v()
    } else {
        Value::Nil
    }
}

fn index(i: i32) -> Option<usize> {
    usize::try_from(i).ok()
}

fn out_of_bounds(i: i32, n: usize) -> LuaError {
    LuaError::Runtime {
        message: format!("Index {i} out of bounds for length {n}"),
        class: "java.lang.IndexOutOfBoundsException",
    }
}

/// `get(i)` on a list of `n`: the value, or Java's exception.
fn checked(i: i32, n: usize, v: impl FnOnce(usize) -> Value) -> Result<Value, LuaError> {
    match index(i).filter(|i| *i < n) {
        Some(i) => Ok(v(i)),
        None => Err(out_of_bounds(i, n)),
    }
}

fn model_value(h: &ScriptHandle, i: usize) -> Value {
    h.child_mut(
        MODEL_CLASS,
        &format!("model{i}"),
        move |v: &VehicleScript| v.models().get(i),
        move |v: &mut VehicleScript| v.models_mut().get_mut(i),
    )
}

fn part_value(h: &ScriptHandle, i: usize) -> Value {
    h.child_mut(
        PART_CLASS,
        &format!("part{i}"),
        move |v: &VehicleScript| v.parts().get(i),
        move |v: &mut VehicleScript| v.parts_mut().get_mut(i),
    )
}

fn area_value(h: &ScriptHandle, i: usize) -> Value {
    h.child_mut(
        AREA_CLASS,
        &format!("area{i}"),
        move |v: &VehicleScript| v.areas().get(i),
        move |v: &mut VehicleScript| v.areas_mut().get_mut(i),
    )
}

fn shape_value(h: &ScriptHandle, i: usize) -> Value {
    h.child_mut(
        PHYSICS_SHAPE_CLASS,
        &format!("shape{i}"),
        move |v: &VehicleScript| v.physics_shapes().get(i),
        move |v: &mut VehicleScript| v.physics_shapes_mut().get_mut(i),
    )
}

fn passenger_value(h: &ScriptHandle, i: usize) -> Value {
    h.child(
        PASSENGER_CLASS,
        &format!("passenger{i}"),
        move |v: &VehicleScript| v.passengers().get(i),
    )
}

fn wheel_value(h: &ScriptHandle, i: usize) -> Value {
    h.child(
        WHEEL_CLASS,
        &format!("wheel{i}"),
        move |v: &VehicleScript| v.wheels().get(i),
    )
}

fn attachment_value(h: &ScriptHandle, i: usize) -> Value {
    h.child_mut(
        MODEL_ATTACHMENT_CLASS,
        &format!("attachment{i}"),
        move |v: &VehicleScript| v.attachments.get(i),
        move |v: &mut VehicleScript| v.attachments.get_mut(i),
    )
}

fn id_arg(c: &NativeCall<'_>) -> Option<String> {
    opt_str(c, 1)
}

/// `getX(int)`/`getXById(String)` and `getXCount()` for one list.
fn indexed(
    b: &crate::expose::ClassBuilder,
    get_name: &str,
    by_id: Option<&str>,
    count: fn(&VehicleScript) -> usize,
    find: fn(&VehicleScript, Option<&str>) -> Option<usize>,
    value: fn(&ScriptHandle, usize) -> Value,
) {
    method(b, get_name, move |c, h| {
        let i = int_arg(c, 1);
        let n = h.read(count)?;
        checked(i, n, |i| value(h, i))
    });
    if let Some(by_id) = by_id {
        method(b, by_id, move |c, h| {
            let id = id_arg(c);
            let i = h.read(|v: &VehicleScript| find(v, id.as_deref()))?;
            Ok(i.map(|i| value(h, i)).unwrap_or_default())
        });
    }
}

fn copy_from(
    b: &crate::expose::ClassBuilder,
    name: &str,
    sm: &Rc<ScriptManager>,
    f: fn(&mut VehicleScript, &ScriptManager, &VehicleScript, &str),
) {
    let s = sm.clone();
    void(b, name, move |c, h| {
        let other = super::handle_arg(c, 1)?.ok_or_else(super::npe)?;
        let spec = opt_str(c, 2).ok_or_else(super::npe)?;
        let r = other.read(|o: &VehicleScript| h.write(|v: &mut VehicleScript| f(v, &s, o, &spec)));
        match r {
            Ok(inner) => inner,
            Err(e) => Err(e),
        }
    });
}

pub fn register(thread: &mut Thread, sm: &Rc<ScriptManager>) {
    let b = expose_class(
        thread,
        VEHICLE_SCRIPT_CLASS,
        Some(super::BASE_SCRIPT_OBJECT_CLASS),
    );
    get(&b, "getName", |v: &VehicleScript| {
        v.name().map(str::to_string)
    });
    get(&b, "getFullName", |v: &VehicleScript| v.full_name());
    get(&b, "getFullType", |v: &VehicleScript| v.full_name());
    get(&b, "getFileName", |v: &VehicleScript| {
        v.file_name().map(str::to_string)
    });
    get(&b, "getMass", |v: &VehicleScript| v.mass());
    get(&b, "getEngineIdleSpeed", |v: &VehicleScript| {
        v.engine_idle_speed()
    });
    get(&b, "getEngineForce", |v: &VehicleScript| v.engine_force());
    get(&b, "getEngineQuality", |v: &VehicleScript| {
        v.engine_quality()
    });
    get(&b, "getEngineLoudness", |v: &VehicleScript| {
        v.engine_loudness()
    });
    get(&b, "getRollInfluence", |v: &VehicleScript| {
        v.roll_influence()
    });
    get(&b, "getSteeringIncrement", |v: &VehicleScript| {
        v.steering_increment()
    });
    method(&b, "getSteeringClamp", |c, h| {
        let speed = num_arg(c, 1) as f32;
        h.read(|v: &VehicleScript| v.steering_clamp(speed).into_lua())
    });
    get(&b, "getSuspensionStiffness", |v: &VehicleScript| {
        v.suspension_stiffness()
    });
    get(&b, "getSuspensionDamping", |v: &VehicleScript| {
        v.suspension_damping()
    });
    get(&b, "getSuspensionCompression", |v: &VehicleScript| {
        v.suspension_compression()
    });
    get(&b, "getSuspensionRestLength", |v: &VehicleScript| {
        v.suspension_rest_length()
    });
    get(&b, "getSuspensionTravel", |v: &VehicleScript| {
        v.suspension_travel()
    });
    get(&b, "getWheelFriction", |v: &VehicleScript| {
        v.wheel_friction()
    });
    get(&b, "getFrontEndHealth", |v: &VehicleScript| {
        v.front_end_health()
    });
    get(&b, "getRearEndHealth", |v: &VehicleScript| {
        v.rear_end_health()
    });
    get(&b, "getStorageCapacity", |v: &VehicleScript| {
        v.storage_capacity()
    });
    get(&b, "getGearRatioCount", |v: &VehicleScript| {
        v.gear_ratio_count()
    });
    get(&b, "getSeats", |v: &VehicleScript| v.seats());
    set(&b, "setSeats", |v: &mut VehicleScript, n: i32| {
        v.set_seats(n)
    });
    get(&b, "getMechanicType", |v: &VehicleScript| v.mechanic_type());
    set(&b, "setMechanicType", |v: &mut VehicleScript, n: i32| {
        v.set_mechanic_type(n)
    });
    get(&b, "getEngineRepairLevel", |v: &VehicleScript| {
        v.engine_repair_level()
    });
    set(
        &b,
        "setEngineRepairLevel",
        |v: &mut VehicleScript, n: i32| v.set_engine_repair_level(n),
    );
    get(&b, "getHeadlightConfigLevel", |v: &VehicleScript| {
        v.headlight_config_level()
    });
    get(&b, "getPlayerDamageProtection", |v: &VehicleScript| {
        v.player_damage_protection()
    });
    set(
        &b,
        "setPlayerDamageProtection",
        |v: &mut VehicleScript, n: f32| v.set_player_damage_protection(n),
    );
    get(&b, "getForcedHue", |v: &VehicleScript| v.forced_hue());
    set(&b, "setForcedHue", |v: &mut VehicleScript, n: f32| {
        v.set_forced_hue(n)
    });
    get(&b, "getForcedSat", |v: &VehicleScript| v.forced_sat());
    set(&b, "setForcedSat", |v: &mut VehicleScript, n: f32| {
        v.set_forced_sat(n)
    });
    get(&b, "getForcedVal", |v: &VehicleScript| v.forced_val());
    set(&b, "setForcedVal", |v: &mut VehicleScript, n: f32| {
        v.set_forced_val(n)
    });
    get(&b, "getEngineRPMType", |v: &VehicleScript| {
        v.engine_rpm_type().to_string()
    });
    set(
        &b,
        "setEngineRPMType",
        |v: &mut VehicleScript, t: Option<String>| {
            v.set_engine_rpm_type(t.as_deref().unwrap_or("null"))
        },
    );
    get(&b, "getOffroadEfficiency", |v: &VehicleScript| {
        v.offroad_efficiency()
    });
    set(
        &b,
        "setOffroadEfficiency",
        |v: &mut VehicleScript, n: f32| v.set_offroad_efficiency(n),
    );
    get(&b, "getCrawlOffsets", |v: &VehicleScript| {
        crate::bind_java_util::list_value(
            v.crawl_offsets()
                .iter()
                .map(|f| Value::Num(*f as f64))
                .collect(),
        )
    });
    get(&b, "getAnimalTrailerSize", |v: &VehicleScript| {
        v.animal_trailer_size()
    });
    get(&b, "getZombieType", |v: &VehicleScript| {
        opt_string_list(v.zombie_type())
    });
    get(&b, "getFirstZombieType", |v: &VehicleScript| {
        v.first_zombie_type().map(str::to_string)
    });
    method(&b, "hasZombieType", |c, h| {
        let t = opt_str(c, 1).unwrap_or_default();
        h.read(|v: &VehicleScript| v.has_zombie_type(&t).into_lua())
    });
    method(&b, "getRandomZombieType", |_, h| {
        let types = h
            .read(|v: &VehicleScript| v.zombie_type().map(<[String]>::to_vec))?
            .ok_or_else(super::npe)?;
        if types.is_empty() {
            return Ok(Value::Nil);
        }
        let i = pz_core::random::next(types.len() as i32) as usize;
        Ok(types[i].as_str().into_lua())
    });
    get(&b, "getSpecialKeyRing", |v: &VehicleScript| {
        opt_string_list(v.special_key_ring())
    });
    get(&b, "hasSpecialKeyRing", |v: &VehicleScript| {
        v.has_special_key_ring()
    });
    get(&b, "notKillCrops", |v: &VehicleScript| v.not_kill_crops());
    get(&b, "hasLighter", |v: &VehicleScript| v.has_lighter());
    get(&b, "neverSpawnKey", |v: &VehicleScript| v.never_spawn_key());
    get(&b, "getHasSiren", |v: &VehicleScript| v.has_siren());
    get(&b, "getCarMechanicsOverlay", |v: &VehicleScript| {
        v.car_mechanics_overlay().map(str::to_string)
    });
    set(
        &b,
        "setCarMechanicsOverlay",
        |v: &mut VehicleScript, s: Option<String>| v.set_car_mechanics_overlay(s),
    );
    get(&b, "getCarModelName", |v: &VehicleScript| {
        v.car_model_name().map(str::to_string)
    });
    set(
        &b,
        "setCarModelName",
        |v: &mut VehicleScript, s: Option<String>| v.set_car_model_name(s),
    );
    get(&b, "getSpecialLootChance", |v: &VehicleScript| {
        v.special_loot_chance()
    });
    get(&b, "getSpecialKeyRingChance", |v: &VehicleScript| {
        v.special_key_ring_chance()
    });
    get(&b, "useChassisPhysicsCollision", |v: &VehicleScript| {
        v.use_chassis_physics_collision()
    });
    get(&b, "hasPhysicsChassisShape", |v: &VehicleScript| {
        v.has_physics_chassis_shape()
    });
    get(&b, "getModelScale", |v: &VehicleScript| v.model_scale());
    let s = sm.clone();
    void(&b, "setModelScale", move |c, h| {
        let scale = num_arg(c, 1) as f32;
        h.write(|v: &mut VehicleScript| v.set_model_scale(&s, scale))?
            .map_err(java_error)
    });
    void(&b, "toBullet", |_, h| {
        h.write(|v: &mut VehicleScript| v.to_bullet())?
            .map_err(java_error)
    });
    let s = sm.clone();
    void(&b, "Loaded", move |_, h| {
        h.write(|v: &mut VehicleScript| v.loaded(&s))?
            .map_err(java_error)
    });
    copy_from(&b, "copyAreasFrom", sm, |v, s, o, spec| {
        v.copy_areas_from(s, o, spec)
    });
    copy_from(&b, "copyPartsFrom", sm, |v, s, o, spec| {
        v.copy_parts_from(s, o, spec)
    });
    copy_from(&b, "copyPassengersFrom", sm, |v, s, o, spec| {
        v.copy_passengers_from(s, o, spec)
    });
    copy_from(&b, "copyWheelsFrom", sm, |v, s, o, spec| {
        v.copy_wheels_from(s, o, spec)
    });
    copy_from(&b, "copyPhysicsFrom", sm, |v, _, o, spec| {
        v.copy_physics_from(o, spec)
    });
    copy_from(&b, "copySoundFrom", sm, |v, _, o, spec| {
        v.copy_sound_from(o, spec)
    });

    get_obj(&b, "getExtents", |h, _: &VehicleScript| {
        vec3(h, VehicleScript::extents, None)
    });
    get_obj(&b, "getPhysicsChassisShape", |h, _: &VehicleScript| {
        vec3(h, VehicleScript::physics_chassis_shape, None)
    });
    get_obj(&b, "getCenterOfMassOffset", |h, _: &VehicleScript| {
        vec3(h, VehicleScript::center_of_mass_offset, None)
    });
    get_obj(&b, "getShadowExtents", |h, _: &VehicleScript| {
        vec2(h, VehicleScript::shadow_extents)
    });
    get_obj(&b, "getShadowOffset", |h, _: &VehicleScript| {
        vec2(h, VehicleScript::shadow_offset)
    });
    get_obj(&b, "getExtentsOffset", |h, _: &VehicleScript| {
        vec2(h, VehicleScript::extents_offset)
    });
    get_obj(&b, "getModelOffset", |h, v: &VehicleScript| {
        some_if(v.model().is_some(), || {
            vec3(
                h,
                |v: &VehicleScript| v.model_offset().unwrap_or_default(),
                None,
            )
        })
    });

    get_obj(&b, "getModel", |h, v: &VehicleScript| {
        some_if(v.model().is_some(), || model_value(h, 0))
    });
    get(&b, "getModelCount", |v: &VehicleScript| v.model_count());
    indexed(
        &b,
        "getModelByIndex",
        None,
        VehicleScript::model_count,
        |_, _| None,
        model_value,
    );
    method(&b, "getModelById", |c, h| {
        let id = id_arg(c);
        let blank = pz_scripts::java::is_null_or_whitespace(id.as_deref());
        let i = h.read(|v: &VehicleScript| {
            v.models().iter().position(|m| {
                (blank && pz_scripts::java::is_null_or_whitespace(m.id.as_deref()))
                    || (m.id.is_some() && m.id == id)
            })
        })?;
        Ok(i.map(|i| model_value(h, i)).unwrap_or_default())
    });
    get(&b, "getPartCount", |v: &VehicleScript| v.part_count());
    indexed(
        &b,
        "getPart",
        Some("getPartById"),
        VehicleScript::part_count,
        |v, id| index(v.index_of_part_by_id(id)),
        part_value,
    );
    method(&b, "getIndexOfPartById", |c, h| {
        let id = id_arg(c);
        h.read(|v: &VehicleScript| v.index_of_part_by_id(id.as_deref()).into_lua())
    });
    get(&b, "getAreaCount", |v: &VehicleScript| v.area_count());
    indexed(
        &b,
        "getArea",
        Some("getAreaById"),
        VehicleScript::area_count,
        |v, id| index(v.index_of_area_by_id(id)),
        area_value,
    );
    method(&b, "getIndexOfAreaById", |c, h| {
        let id = id_arg(c);
        h.read(|v: &VehicleScript| v.index_of_area_by_id(id.as_deref()).into_lua())
    });
    get(&b, "getWheelCount", |v: &VehicleScript| v.wheel_count());
    indexed(
        &b,
        "getWheel",
        Some("getWheelById"),
        VehicleScript::wheel_count,
        |v, id| index(v.index_of_wheel_by_id(id)),
        wheel_value,
    );
    method(&b, "getIndexOfWheelById", |c, h| {
        let id = id_arg(c);
        h.read(|v: &VehicleScript| v.index_of_wheel_by_id(id.as_deref()).into_lua())
    });
    get(&b, "getPassengerCount", |v: &VehicleScript| {
        v.passenger_count()
    });
    indexed(
        &b,
        "getPassenger",
        Some("getPassengerById"),
        VehicleScript::passenger_count,
        |v, id| index(v.passenger_index(id)),
        passenger_value,
    );
    method(&b, "getPassengerIndex", |c, h| {
        let id = id_arg(c);
        h.read(|v: &VehicleScript| v.passenger_index(id.as_deref()).into_lua())
    });
    get(&b, "getPhysicsShapeCount", |v: &VehicleScript| {
        v.physics_shape_count()
    });
    method(&b, "getPhysicsShape", |c, h| {
        let i = int_arg(c, 1);
        let n = h.read(|v: &VehicleScript| v.physics_shape_count())?;
        Ok(index(i)
            .filter(|i| *i < n)
            .map(|i| shape_value(h, i))
            .unwrap_or_default())
    });
    method(&b, "addPhysicsShape", |c, h| {
        let t = opt_str(c, 1).ok_or_else(super::npe)?;
        h.write(|v: &mut VehicleScript| v.add_physics_shape(&t).map(|_| ()))?
            .map_err(java_error)?;
        let n = h.read(|v: &VehicleScript| v.physics_shape_count())?;
        Ok(shape_value(h, n - 1))
    });
    void(&b, "removePhysicsShape", |c, h| {
        let i = int_arg(c, 1);
        let n = h.read(|v: &VehicleScript| v.physics_shape_count())?;
        let i = index(i)
            .filter(|i| *i < n)
            .ok_or_else(|| out_of_bounds(i, n))?;
        h.write(|v: &mut VehicleScript| {
            v.remove_physics_shape(i);
        })
    });
    get(&b, "getAttachmentCount", |v: &VehicleScript| {
        v.attachment_count()
    });
    indexed(
        &b,
        "getAttachment",
        Some("getAttachmentById"),
        VehicleScript::attachment_count,
        |v, id| v.attachments.iter().position(|a| Some(a.id()) == id),
        attachment_value,
    );
    method(&b, "addAttachment", |c, h| {
        let a = attachment_arg(c, 1)?.borrow().clone();
        h.write(|v: &mut VehicleScript| {
            v.add_attachment(a);
        })?;
        let n = h.read(|v: &VehicleScript| v.attachment_count())?;
        Ok(attachment_value(h, n - 1))
    });
    method(&b, "addAttachmentAt", |c, h| {
        let i = int_arg(c, 1);
        let a = attachment_arg(c, 2)?.borrow().clone();
        let n = h.read(|v: &VehicleScript| v.attachment_count())?;
        let i = index(i)
            .filter(|i| *i <= n)
            .ok_or_else(|| out_of_bounds(i, n))?;
        h.write(|v: &mut VehicleScript| {
            v.add_attachment_at(i, a);
        })?;
        Ok(attachment_value(h, i))
    });
    void(&b, "removeAttachment", |c, h| {
        let n = h.read(|v: &VehicleScript| v.attachment_count())?;
        let i = match c.arg(1) {
            Value::Num(_) => {
                let i = int_arg(c, 1);
                index(i)
                    .filter(|i| *i < n)
                    .ok_or_else(|| out_of_bounds(i, n))?
            }
            v => {
                let id = super::handle_arg(c, 1)?.and_then(|a| {
                    a.read(|a: &pz_scripts::objects::models::ModelAttachment| a.id().to_string())
                        .ok()
                });
                let _ = v;
                match id.and_then(|id| {
                    h.read(|v: &VehicleScript| v.attachments.iter().position(|a| a.id() == id))
                        .ok()
                        .flatten()
                }) {
                    Some(i) => i,
                    None => return Ok(()),
                }
            }
        };
        h.write(|v: &mut VehicleScript| {
            v.remove_attachment(i);
        })
    });
    void(&b, "beforeRenameAttachment", |_, _| Ok(()));
    void(&b, "afterRenameAttachment", |_, _| Ok(()));

    let b = expose_class(thread, PART_CLASS, None);
    get(&b, "getId", |p: &Part| p.id().map(str::to_string));
    get(&b, "getModelCount", |p: &Part| p.model_count());
    get(&b, "getDurability", |p: &Part| p.durability());
    get(&b, "getMechanicArea", |p: &Part| {
        p.mechanic_area().map(str::to_string)
    });
    get(&b, "isRepairMechanic", |p: &Part| p.repair_mechanic);
    set(&b, "setRepairMechanic", |p: &mut Part, v: bool| {
        p.repair_mechanic = v
    });
    get(&b, "isMechanicRequireKey", |p: &Part| {
        p.mechanic_require_key
    });
    set(&b, "setMechanicRequireKey", |p: &mut Part, v: bool| {
        p.mechanic_require_key = v
    });
    method(&b, "getModel", |c, h| {
        let i = int_arg(c, 1);
        let n = h.read(|p: &Part| p.model_count())?;
        checked(i, n, |i| {
            h.child(MODEL_CLASS, &format!("model{i}"), move |p: &Part| {
                p.models.as_ref()?.get(i)
            })
        })
    });
    method(&b, "getModelById", |c, h| {
        let id = opt_str(c, 1);
        let i = h.read(|p: &Part| {
            p.models
                .as_ref()
                .and_then(|m| m.iter().position(|m| m.id == id))
        })?;
        Ok(i.map(|i| {
            h.child(MODEL_CLASS, &format!("model{i}"), move |p: &Part| {
                p.models.as_ref()?.get(i)
            })
        })
        .unwrap_or_default())
    });

    let b = expose_class(thread, MODEL_CLASS, None);
    get(&b, "getId", |m: &Model| m.id.clone());
    get(&b, "getFile", |m: &Model| m.file.clone());
    get(&b, "getScale", |m: &Model| m.scale);
    get(&b, "getAttachmentNameParent", |m: &Model| {
        m.attachment_name_parent.clone()
    });
    get(&b, "getAttachmentNameSelf", |m: &Model| {
        m.attachment_name_self.clone()
    });
    get_obj(&b, "getOffset", |h, _: &Model| {
        vec3(
            h,
            |m: &Model| m.offset,
            Some(|m: &mut Model, v| m.offset = v),
        )
    });
    get_obj(&b, "getRotate", |h, _: &Model| {
        vec3(
            h,
            |m: &Model| m.rotate,
            Some(|m: &mut Model, v| m.rotate = v),
        )
    });

    let b = expose_class(thread, AREA_CLASS, None);
    get(&b, "getId", |a: &Area| a.id.clone());
    get(&b, "getX", |a: &Area| a.x);
    get(&b, "getY", |a: &Area| a.y);
    get(&b, "getW", |a: &Area| a.w);
    get(&b, "getH", |a: &Area| a.h);
    set(&b, "setX", |a: &mut Area, v: f64| a.x = v as f32);
    set(&b, "setY", |a: &mut Area, v: f64| a.y = v as f32);
    set(&b, "setW", |a: &mut Area, v: f64| a.w = v as f32);
    set(&b, "setH", |a: &mut Area, v: f64| a.h = v as f32);

    let b = expose_class(thread, PHYSICS_SHAPE_CLASS, None);
    get(&b, "getRadius", |s: &PhysicsShape| s.radius);
    set(&b, "setRadius", |s: &mut PhysicsShape, v: f32| {
        s.set_radius(v)
    });
    get(&b, "getTypeString", |s: &PhysicsShape| s.type_string());
    get(&b, "getPhysicsShapeScript", |s: &PhysicsShape| {
        s.physics_shape_script.clone()
    });
    set(
        &b,
        "setPhysicsShapeScript",
        |s: &mut PhysicsShape, v: Option<String>| s.physics_shape_script = v,
    );
    get_obj(&b, "getOffset", |h, _: &PhysicsShape| {
        vec3(
            h,
            |s: &PhysicsShape| s.offset,
            Some(|s: &mut PhysicsShape, v| s.offset = v),
        )
    });
    get_obj(&b, "getRotate", |h, _: &PhysicsShape| {
        vec3(
            h,
            |s: &PhysicsShape| s.rotate,
            Some(|s: &mut PhysicsShape, v| s.rotate = v),
        )
    });
    get_obj(&b, "getExtents", |h, _: &PhysicsShape| {
        vec3(
            h,
            |s: &PhysicsShape| s.extents,
            Some(|s: &mut PhysicsShape, v| s.extents = v),
        )
    });

    let b = expose_class(thread, PASSENGER_CLASS, None);
    get(&b, "getId", |p: &Passenger| p.id.clone());
    get(&b, "getPositionCount", |p: &Passenger| p.positions.len());
    method(&b, "getPosition", |c, h| {
        let i = int_arg(c, 1);
        let n = h.read(|p: &Passenger| p.positions.len())?;
        checked(i, n, |i| {
            h.child(
                POSITION_CLASS,
                &format!("position{i}"),
                move |p: &Passenger| p.positions.get(i),
            )
        })
    });
    method(&b, "getPositionById", |c, h| {
        let id = opt_str(c, 1);
        let i = h.read(|p: &Passenger| {
            p.positions
                .iter()
                .position(|pos| pos.id.is_some() && pos.id == id)
        })?;
        Ok(i.map(|i| {
            h.child(
                POSITION_CLASS,
                &format!("position{i}"),
                move |p: &Passenger| p.positions.get(i),
            )
        })
        .unwrap_or_default())
    });

    let b = expose_class(thread, POSITION_CLASS, None);
    get(&b, "getId", |p: &Position| p.id.clone());
    get(&b, "getArea", |p: &Position| p.area.clone());
    get_obj(&b, "getOffset", |h, _: &Position| {
        vec3(h, |p: &Position| p.offset, None)
    });
    get_obj(&b, "getRotate", |h, _: &Position| {
        vec3(h, |p: &Position| p.rotate, None)
    });

    let b = expose_class(thread, WHEEL_CLASS, None);
    get(&b, "getId", |w: &Wheel| w.id.clone());
    get_obj(&b, "getOffset", |h, _: &Wheel| {
        vec3(h, |w: &Wheel| w.offset, None)
    });
}
