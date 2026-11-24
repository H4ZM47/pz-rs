//! Build 42.22 enum & direction-set bindings:
//! - `zombie.characterTextures.BloodBodyPartType`
//! - `zombie.characterTextures.BloodClothingType`
//! - `zombie.core.properties.IsoObjectChange`
//! - `zombie.iso.IsoDirectionSet`
//! - `zombie.entity.components.resources.{ResourceChannel, ResourceIO}`
//! - `zombie.entity.components.crafting.StartMode`
//! - `zombie.entity.components.attributes.AttributeValueType`
//! - `zombie.entity.GameEntityType`
//! - `zombie.scripting.objects.{DigType, WeaponReloadType}`
//! - `zombie.scripting.ui.TextAlign`

use std::any::Any;
use std::cell::Cell;
use std::rc::Rc;

use pz_characters::character::items::blood_clothing_covered_parts;
use pz_kahlua::value::LuaObject;
use pz_kahlua::{Thread, Value};

use crate::bind_java_util::{list_value, JavaList};
use crate::expose::{enum_value, expose_class, expose_enum, int_arg, str_arg, with_this, JavaEnum};

pub const BLOOD_BODY_PART_TYPE_CLASS: &str = "zombie.characterTextures.BloodBodyPartType";
pub const BLOOD_CLOTHING_TYPE_CLASS: &str = "zombie.characterTextures.BloodClothingType";
pub const ISO_OBJECT_CHANGE_CLASS: &str = "zombie.core.properties.IsoObjectChange";
pub const ISO_DIRECTION_SET_CLASS: &str = "zombie.iso.IsoDirectionSet";
pub const RESOURCE_CHANNEL_CLASS: &str = "zombie.entity.components.resources.ResourceChannel";
pub const RESOURCE_IO_CLASS: &str = "zombie.entity.components.resources.ResourceIO";
pub const START_MODE_CLASS: &str = "zombie.entity.components.crafting.StartMode";
pub const ATTRIBUTE_VALUE_TYPE_CLASS: &str =
    "zombie.entity.components.attributes.AttributeValueType";
pub const GAME_ENTITY_TYPE_CLASS: &str = "zombie.entity.GameEntityType";
pub const DIG_TYPE_CLASS: &str = "zombie.scripting.objects.DigType";
pub const WEAPON_RELOAD_TYPE_CLASS: &str = "zombie.scripting.objects.WeaponReloadType";
pub const TEXT_ALIGN_CLASS: &str = "zombie.scripting.ui.TextAlign";

const BLOOD_BODY_PARTS: &[(&str, &str)] = &[
    ("Hand_L", "IGUI_health_Left_Hand"),
    ("Hand_R", "IGUI_health_Right_Hand"),
    ("ForeArm_L", "IGUI_health_Left_Forearm"),
    ("ForeArm_R", "IGUI_health_Right_Forearm"),
    ("UpperArm_L", "IGUI_health_Left_Upper_Arm"),
    ("UpperArm_R", "IGUI_health_Right_Upper_Arm"),
    ("Torso_Upper", "IGUI_health_Upper_Torso"),
    ("Torso_Lower", "IGUI_health_Lower_Torso"),
    ("Head", "IGUI_health_Head"),
    ("Neck", "IGUI_health_Neck"),
    ("Groin", "IGUI_health_Groin"),
    ("UpperLeg_L", "IGUI_health_Left_Thigh"),
    ("UpperLeg_R", "IGUI_health_Right_Thigh"),
    ("LowerLeg_L", "IGUI_health_Left_Shin"),
    ("LowerLeg_R", "IGUI_health_Right_Shin"),
    ("Foot_L", "IGUI_health_Left_Foot"),
    ("Foot_R", "IGUI_health_Right_Foot"),
    ("Back", "IGUI_health_Back"),
    ("MAX", "IGUI_health_Unknown_Body_Part"),
];

const BLOOD_CLOTHING_TYPES: &[&str] = &[
    "Apron",
    "ShirtNoSleeves",
    "JumperNoSleeves",
    "Shirt",
    "ShirtLongSleeves",
    "Jumper",
    "Jacket",
    "LongJacket",
    "ShortsShort",
    "Trousers",
    "Shoes",
    "FullHelmet",
    "Bag",
    "Hands",
    "Head",
    "Neck",
    "Groin",
    "UpperBody",
    "LowerBody",
    "LowerLegs",
    "UpperLegs",
    "LowerArms",
    "UpperArms",
    "Hand_L",
    "Hand_R",
    "ForeArm_L",
    "ForeArm_R",
    "UpperArm_L",
    "UpperArm_R",
    "UpperLeg_L",
    "UpperLeg_R",
    "LowerLeg_L",
    "LowerLeg_R",
    "Foot_L",
    "Foot_R",
];

const ISO_OBJECT_CHANGES: &[(&str, &str)] = &[
    ("ENERGY", "Energy"),
    ("LIGHT_RADIUS", "LightRadius"),
    ("CONTAINERS", "containers"),
    ("CONTAINER", "container"),
    ("CONTAINER_CUSTOM_TEMPERATURE", "container.customTemperature"),
    ("NAME", "name"),
    ("REPLACE_WITH", "replaceWith"),
    ("USES_EXTERNAL_WATER_SOURCE", "usesExternalWaterSource"),
    ("EMPTY_TRASH", "emptyTrash"),
    ("SPRITE", "sprite"),
    ("STATE", "state"),
    ("WASHER_STATE", "washer.state"),
    ("DRYER_STATE", "dryer.state"),
    ("MODE", "mode"),
    ("BECOME_SKELETON", "becomeSkeleton"),
    ("ANIMAL_ROT_STAGE", "animalRotStage"),
    ("ZOMBIE_ROT_STAGE", "zombieRotStage"),
    ("OBJECT_ID", "objectID"),
    ("ADD_SHEET", "addSheet"),
    ("REMOVE_SHEET", "removeSheet"),
    ("SET_CURTAIN_OPEN", "setCurtainOpen"),
    ("ADD_ITEM", "addItem"),
    ("ADD_ITEM_OF_TYPE", "addItemOfType"),
    ("ADD_RANDOM_DAMAGE_FROM_ZOMBIE", "AddRandomDamageFromZombie"),
    ("ADD_ZOMBIE_KILL", "AddZombieKill"),
    ("REMOVE_ITEM", "removeItem"),
    ("REMOVE_ITEM_ID", "removeItemID"),
    ("REMOVE_ITEM_TYPE", "removeItemType"),
    ("REMOVE_ONE_OF", "removeOneOf"),
    ("REANIMATED_ID", "reanimatedID"),
    ("SHOVE", "Shove"),
    ("WAKE_UP", "wakeUp"),
    ("MECHANIC_ACTION_DONE", "mechanicActionDone"),
    ("EXIT_VEHICLE", "exitVehicle"),
    ("STOP_BURNING", "StopBurning"),
    ("VEHICLE_NO_KEY", "vehicleNoKey"),
    ("ROTATE", "rotate"),
    ("LIGHT_SOURCE", "lightSource"),
    ("PAINTABLE", "paintable"),
    ("SWAP_ITEM", "swapItem"),
    ("PLAY_GAIN_EXPERIENCE_LEVEL_SOUND", "playGainExperienceLevelSound"),
    ("SET_HALO_NOTE", "setHaloNote"),
    ("COUGH", "Cough"),
    ("VEHICLE_HIT_OBJECT", "VehicleHitObject"),
];

const RESOURCE_CHANNELS: &[&str] = &[
    "NO_CHANNEL",
    "Channel_Red",
    "Channel_Yellow",
    "Channel_Blue",
    "Channel_Orange",
    "Channel_Green",
    "Channel_Purple",
    "Channel_Cyan",
    "Channel_Magenta",
];

const RESOURCE_IOS: &[&str] = &["Input", "Output", "Any"];
const START_MODES: &[&str] = &["Manual", "Automatic", "Passive"];
const ATTRIBUTE_VALUE_TYPES: &[&str] = &[
    "Boolean",
    "String",
    "Float",
    "Double",
    "Byte",
    "Short",
    "Int",
    "Long",
    "Enum",
    "EnumSet",
    "EnumStringSet",
];
const GAME_ENTITY_TYPES: &[&str] = &[
    "IsoObject",
    "InventoryItem",
    "VehiclePart",
    "IsoMovingObject",
    "Template",
    "MetaEntity",
];
const DIG_TYPES: &[&str] = &["HOE", "PICK_AXE", "SHOVEL", "TROWEL"];
const WEAPON_RELOAD_TYPES: &[(&str, &str)] = &[
    ("NONE", "none"),
    ("BOLT_ACTION", "boltaction"),
    ("BOLT_ACTION_NO_MAG", "boltactionnomag"),
    ("DOUBLE_BARREL_SHOTGUN", "doublebarrelshotgun"),
    ("DOUBLE_BARREL_SHOTGUN_SAWN", "doublebarrelshotgunsawn"),
    ("HANDGUN", "handgun"),
    ("LEVER_ACTION", "leveraction"),
    ("REVOLVER", "revolver"),
    ("SHOTGUN", "shotgun"),
];
const TEXT_ALIGNS: &[&str] = &["Left", "Center", "Right"];

const ISO_DIRECTIONS_CLASS: &str = "zombie.iso.IsoDirections";
const ISO_DIR_NAMES: &[&str] = &["N", "NW", "W", "SW", "S", "SE", "E", "NE"];

struct LuaIsoDirectionSet {
    bits: Cell<i32>,
}

impl LuaObject for LuaIsoDirectionSet {
    fn class_name(&self) -> &str {
        ISO_DIRECTION_SET_CLASS
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

fn dir_index(v: &Value) -> Option<usize> {
    match v {
        Value::Object(o) => o
            .as_any()
            .downcast_ref::<JavaEnum>()
            .and_then(|e| ISO_DIR_NAMES.iter().position(|&n| n == e.name)),
        Value::Str(s) => ISO_DIR_NAMES.iter().position(|&n| n == s.as_ref()),
        _ => None,
    }
}

fn extract_clothing_type_names(arg: &Value) -> Vec<String> {
    let Value::Object(o) = arg else {
        return Vec::new();
    };
    let Some(list) = o.as_any().downcast_ref::<JavaList>() else {
        return Vec::new();
    };
    list.items
        .borrow()
        .iter()
        .filter_map(|v| match v {
            Value::Object(o) => o
                .as_any()
                .downcast_ref::<JavaEnum>()
                .map(|e| e.name.to_string()),
            Value::Str(s) => Some(s.to_string()),
            _ => None,
        })
        .collect()
}

pub fn register(thread: &mut Thread) {
    // BloodBodyPartType
    let bp_names: Vec<&'static str> = BLOOD_BODY_PARTS.iter().map(|&(n, _)| n).collect();
    let bp = expose_enum(thread, BLOOD_BODY_PART_TYPE_CLASS, &bp_names);
    bp.method("index", |c| {
        let ord = with_this::<JavaEnum, _>(c, |e| e.ordinal)?;
        Ok(c.push(ord as f64))
    });
    bp.method("getDisplayName", |c| {
        let ord = with_this::<JavaEnum, _>(c, |e| e.ordinal)?;
        let key = BLOOD_BODY_PARTS
            .get(ord as usize)
            .map_or("IGUI_health_Unknown_Body_Part", |&(_, k)| k);
        let text = crate::bind_text::with_translator_mut(|t| t.get_text(key, &[]))
            .unwrap_or_else(|| key.to_string());
        Ok(c.push(text))
    });
    bp.static_fn("FromIndex", |c| {
        let idx = int_arg(c, 0);
        let name = BLOOD_BODY_PARTS
            .get(idx as usize)
            .map_or("MAX", |&(n, _)| n);
        Ok(c.push(enum_value(BLOOD_BODY_PART_TYPE_CLASS, name)))
    });
    bp.static_fn("ToIndex", |c| {
        let idx = match c.arg(0) {
            Value::Object(o) => o
                .as_any()
                .downcast_ref::<JavaEnum>()
                .map_or(BLOOD_BODY_PARTS.len() as i32 - 1, |e| e.ordinal),
            _ => BLOOD_BODY_PARTS.len() as i32 - 1,
        };
        Ok(c.push(idx as f64))
    });
    bp.static_fn("FromString", |c| {
        let name = str_arg(c, 0);
        let found = name
            .as_deref()
            .and_then(|s| BLOOD_BODY_PARTS.iter().find(|&&(n, _)| n == s).map(|&(n, _)| n))
            .unwrap_or("MAX");
        Ok(c.push(enum_value(BLOOD_BODY_PART_TYPE_CLASS, found)))
    });

    // BloodClothingType
    let bc = expose_enum(thread, BLOOD_CLOTHING_TYPE_CLASS, BLOOD_CLOTHING_TYPES);
    bc.static_fn("fromString", |c| {
        let found = str_arg(c, 0).and_then(|s| {
            BLOOD_CLOTHING_TYPES
                .iter()
                .copied()
                .find(|&n| n.eq_ignore_ascii_case(&s))
        });
        Ok(c.push(
            found.map_or(Value::Nil, |n| enum_value(BLOOD_CLOTHING_TYPE_CLASS, n)),
        ))
    });
    bc.static_fn("getCoveredParts", |c| {
        let names = extract_clothing_type_names(&c.arg(0));
        let mut parts: Vec<Value> = Vec::new();
        for name in &names {
            if let Some(indices) = blood_clothing_covered_parts(name) {
                for idx in indices {
                    if let Some(&(bp_name, _)) = BLOOD_BODY_PARTS.get(idx) {
                        parts.push(enum_value(BLOOD_BODY_PART_TYPE_CLASS, bp_name));
                    }
                }
            }
        }
        if c.nargs() >= 2 {
            if let Value::Object(out_obj) = c.arg(1) {
                if let Some(out_list) = out_obj.as_any().downcast_ref::<JavaList>() {
                    out_list.items.borrow_mut().extend(parts);
                    return Ok(c.push(Value::Object(out_obj)));
                }
            }
        }
        Ok(c.push(list_value(parts)))
    });
    bc.static_fn("getCoveredPartCount", |c| {
        let names = extract_clothing_type_names(&c.arg(0));
        let count: usize = names
            .iter()
            .filter_map(|n| blood_clothing_covered_parts(n))
            .map(|v| v.len())
            .sum();
        Ok(c.push(count as f64))
    });

    // IsoObjectChange
    let ioc_names: Vec<&'static str> = ISO_OBJECT_CHANGES.iter().map(|&(n, _)| n).collect();
    let ioc = expose_enum(thread, ISO_OBJECT_CHANGE_CLASS, &ioc_names);
    ioc.method("getName", |c| {
        let ord = with_this::<JavaEnum, _>(c, |e| e.ordinal)?;
        let s = ISO_OBJECT_CHANGES
            .get(ord as usize)
            .map_or("", |&(_, s)| s);
        Ok(c.push(s))
    });
    ioc.method("toString", |c| {
        let ord = with_this::<JavaEnum, _>(c, |e| e.ordinal)?;
        let s = ISO_OBJECT_CHANGES
            .get(ord as usize)
            .map_or("", |&(_, s)| s);
        Ok(c.push(s))
    });
    ioc.method("isProperty", |c| {
        let ord = with_this::<JavaEnum, _>(c, |e| e.ordinal)?;
        let prop = ISO_OBJECT_CHANGES
            .get(ord as usize)
            .map_or("", |&(_, s)| s);
        let ok = str_arg(c, 1).is_some_and(|s| s.as_ref() == prop);
        Ok(c.push(ok))
    });
    ioc.static_fn("lookup", |c| {
        let found = match c.arg(0) {
            Value::Str(s) => ISO_OBJECT_CHANGES
                .iter()
                .find(|&&(en, pn)| pn == s.as_ref() || en == s.as_ref())
                .map(|&(en, _)| enum_value(ISO_OBJECT_CHANGE_CLASS, en)),
            Value::Object(o) if o.class_name() == ISO_OBJECT_CHANGE_CLASS => {
                Some(Value::Object(o))
            }
            _ => None,
        };
        Ok(c.push(found.unwrap_or(Value::Nil)))
    });

    // IsoDirectionSet
    let ids = expose_class(thread, ISO_DIRECTION_SET_CLASS, Some("java.lang.Object"));
    ids.static_fn("new", |c| {
        Ok(c.push(Value::Object(Rc::new(LuaIsoDirectionSet {
            bits: Cell::new(0),
        }))))
    });
    ids.static_fn("rotate", |c| {
        let Some(idx) = dir_index(&c.arg(0)) else {
            return Ok(c.push(Value::Nil));
        };
        let amount = int_arg(c, 1);
        let next = ((idx as i32 + amount).rem_euclid(8)) as usize;
        Ok(c.push(enum_value(ISO_DIRECTIONS_CLASS, ISO_DIR_NAMES[next])))
    });
    ids.method("getNext", |c| {
        let next = with_this::<LuaIsoDirectionSet, _>(c, |s| {
            let mut b = s.bits.get();
            for i in 0..8 {
                let mask = 1 << i;
                if (b & mask) != 0 {
                    b &= !mask;
                    s.bits.set(b);
                    return Some(ISO_DIR_NAMES[i]);
                }
            }
            None
        })?;
        Ok(c.push(
            next.map_or(Value::Nil, |n| enum_value(ISO_DIRECTIONS_CLASS, n)),
        ))
    });

    // ResourceChannel
    let rc = expose_enum(thread, RESOURCE_CHANNEL_CLASS, RESOURCE_CHANNELS);
    rc.method("getByteId", |c| {
        let ord = with_this::<JavaEnum, _>(c, |e| e.ordinal)?;
        let id = if ord == 0 { -1 } else { ord - 1 };
        Ok(c.push(id as f64))
    });
    rc.method("getBits", |c| {
        let ord = with_this::<JavaEnum, _>(c, |e| e.ordinal)?;
        let bits = if ord == 0 { 0 } else { 1i32 << (ord - 1) };
        Ok(c.push(bits as f64))
    });
    rc.static_fn("fromId", |c| {
        let id = int_arg(c, 0);
        let name = if id < 0 {
            "NO_CHANNEL"
        } else {
            RESOURCE_CHANNELS
                .get((id + 1) as usize)
                .copied()
                .unwrap_or("NO_CHANNEL")
        };
        Ok(c.push(enum_value(RESOURCE_CHANNEL_CLASS, name)))
    });

    // ResourceIO
    let rio = expose_enum(thread, RESOURCE_IO_CLASS, RESOURCE_IOS);
    rio.method("getId", |c| {
        let ord = with_this::<JavaEnum, _>(c, |e| e.ordinal)?;
        Ok(c.push(ord as f64))
    });
    rio.static_fn("fromId", |c| {
        let id = int_arg(c, 0);
        let name = RESOURCE_IOS.get(id as usize).copied().unwrap_or("Any");
        Ok(c.push(enum_value(RESOURCE_IO_CLASS, name)))
    });

    // StartMode
    let sm = expose_enum(thread, START_MODE_CLASS, START_MODES);
    sm.method("getByteId", |c| {
        let ord = with_this::<JavaEnum, _>(c, |e| e.ordinal)?;
        Ok(c.push((ord + 1) as f64))
    });
    sm.method("getBits", |c| {
        let ord = with_this::<JavaEnum, _>(c, |e| e.ordinal)?;
        Ok(c.push((1i32 << ord) as f64))
    });
    sm.static_fn("fromByteId", |c| {
        let id = int_arg(c, 0);
        let name = if id > 0 {
            START_MODES.get((id - 1) as usize).copied()
        } else {
            None
        };
        Ok(c.push(
            name.map_or(Value::Nil, |n| enum_value(START_MODE_CLASS, n)),
        ))
    });

    // AttributeValueType
    let avt = expose_enum(thread, ATTRIBUTE_VALUE_TYPE_CLASS, ATTRIBUTE_VALUE_TYPES);
    avt.method("getByteIndex", |c| {
        let ord = with_this::<JavaEnum, _>(c, |e| e.ordinal)?;
        Ok(c.push(ord as f64))
    });
    avt.static_fn("fromByteIndex", |c| {
        let idx = int_arg(c, 0);
        let name = ATTRIBUTE_VALUE_TYPES.get(idx as usize).copied();
        Ok(c.push(
            name.map_or(Value::Nil, |n| enum_value(ATTRIBUTE_VALUE_TYPE_CLASS, n)),
        ))
    });
    avt.static_fn("IsNumeric", |c| {
        let ok = match c.arg(0) {
            Value::Object(o) => o.as_any().downcast_ref::<JavaEnum>().is_some_and(|e| {
                matches!(e.name, "Float" | "Double" | "Byte" | "Short" | "Int" | "Long")
            }),
            _ => false,
        };
        Ok(c.push(ok))
    });
    avt.static_fn("IsDecimal", |c| {
        let ok = match c.arg(0) {
            Value::Object(o) => o
                .as_any()
                .downcast_ref::<JavaEnum>()
                .is_some_and(|e| matches!(e.name, "Float" | "Double")),
            _ => false,
        };
        Ok(c.push(ok))
    });
    avt.static_fn("valueOfIgnoreCase", |c| {
        let found = str_arg(c, 0).and_then(|s| {
            ATTRIBUTE_VALUE_TYPES
                .iter()
                .copied()
                .find(|&n| n.eq_ignore_ascii_case(&s))
        });
        Ok(c.push(
            found.map_or(Value::Nil, |n| enum_value(ATTRIBUTE_VALUE_TYPE_CLASS, n)),
        ))
    });

    // GameEntityType
    let get = expose_enum(thread, GAME_ENTITY_TYPE_CLASS, GAME_ENTITY_TYPES);
    for method_name in ["getId", "getByteId"] {
        get.method(method_name, |c| {
            let ord = with_this::<JavaEnum, _>(c, |e| e.ordinal)?;
            Ok(c.push((ord + 1) as f64))
        });
    }
    get.method("getBits", |c| {
        let ord = with_this::<JavaEnum, _>(c, |e| e.ordinal)?;
        Ok(c.push((1i32 << ord) as f64))
    });
    get.static_fn("FromID", |c| {
        let id = int_arg(c, 0);
        let name = if id > 0 {
            GAME_ENTITY_TYPES.get((id - 1) as usize).copied()
        } else {
            None
        };
        Ok(c.push(
            name.map_or(Value::Nil, |n| enum_value(GAME_ENTITY_TYPE_CLASS, n)),
        ))
    });

    // DigType
    let dig = expose_enum(thread, DIG_TYPE_CLASS, DIG_TYPES);
    dig.method("toString", |c| {
        let name = with_this::<JavaEnum, _>(c, |e| e.name)?;
        Ok(c.push(name.to_ascii_lowercase()))
    });

    // WeaponReloadType
    let wrt_names: Vec<&'static str> = WEAPON_RELOAD_TYPES.iter().map(|&(n, _)| n).collect();
    let wrt = expose_enum(thread, WEAPON_RELOAD_TYPE_CLASS, &wrt_names);
    wrt.method("toString", |c| {
        let ord = with_this::<JavaEnum, _>(c, |e| e.ordinal)?;
        let s = WEAPON_RELOAD_TYPES
            .get(ord as usize)
            .map_or("none", |&(_, s)| s);
        Ok(c.push(s))
    });
    wrt.static_fn("fromValue", |c| {
        let found = str_arg(c, 0).and_then(|s| {
            WEAPON_RELOAD_TYPES
                .iter()
                .find(|&&(en, val)| val.eq_ignore_ascii_case(&s) || en.eq_ignore_ascii_case(&s))
                .map(|&(en, _)| en)
        });
        Ok(c.push(
            found.map_or(Value::Nil, |n| enum_value(WEAPON_RELOAD_TYPE_CLASS, n)),
        ))
    });

    // TextAlign
    expose_enum(thread, TEXT_ALIGN_CLASS, TEXT_ALIGNS);
}

#[cfg(test)]
mod tests {
    use super::*;
    use pz_kahlua::TableRef;

    #[test]
    fn blood_object_change_and_entity_enums_work_from_lua() {
        let mut t = Thread::new(TableRef::new());
        pz_kahlua::stdlib::install(&mut t, None).unwrap();
        crate::bind_java_util::register(&mut t);
        crate::expose::expose_enum(&mut t, ISO_DIRECTIONS_CLASS, ISO_DIR_NAMES);
        register(&mut t);

        let src = r#"
            local function assert(v, msg) if not v then error(msg or "assertion failed") end end

            assert(BloodBodyPartType.Head:index() == 8)
            assert(BloodBodyPartType.FromIndex(8) == BloodBodyPartType.Head)
            assert(BloodBodyPartType.ToIndex(BloodBodyPartType.Head) == 8)
            assert(BloodBodyPartType.FromString("Foot_R") == BloodBodyPartType.Foot_R)
            assert(BloodBodyPartType.FromString("Nope") == BloodBodyPartType.MAX)
            assert(type(BloodBodyPartType.Head:getDisplayName()) == "string")

            local bct = ArrayList.new()
            bct:add(BloodClothingType.fromString("Shirt"))
            local parts = BloodClothingType.getCoveredParts(bct)
            assert(parts:size() > 0)
            assert(BloodClothingType.getCoveredPartCount(bct) == parts:size())

            assert(IsoObjectChange.CONTAINER:getName() == "container")
            assert(IsoObjectChange.CONTAINER:isProperty("container") == true)
            assert(IsoObjectChange.lookup("container") == IsoObjectChange.CONTAINER)
            assert(IsoObjectChange.lookup("unknown") == nil)

            assert(IsoDirectionSet.rotate(IsoDirections.N, 2) == IsoDirections.W)
            assert(IsoDirectionSet.rotate(IsoDirections.N, -1) == IsoDirections.NE)

            assert(ResourceChannel.NO_CHANNEL:getByteId() == -1)
            assert(ResourceChannel.Channel_Red:getByteId() == 0)
            assert(ResourceChannel.fromId(0) == ResourceChannel.Channel_Red)
            assert(ResourceIO.fromId(0) == ResourceIO.Input)
            assert(StartMode.fromByteId(1) == StartMode.Manual)
            assert(AttributeValueType.IsNumeric(AttributeValueType.Float) == true)
            assert(AttributeValueType.IsDecimal(AttributeValueType.Int) == false)
            assert(AttributeValueType.valueOfIgnoreCase("string") == AttributeValueType.String)
            assert(GameEntityType.FromID(1) == GameEntityType.IsoObject)
            assert(WeaponReloadType.fromValue("shotgun") == WeaponReloadType.SHOTGUN)
            assert(TextAlign.Center ~= nil)
            return true
        "#;
        let cl = pz_kahlua::stdlib::load_string(src, Some("bind_blood_enums_test"), t.globals().clone())
            .unwrap();
        let res = t
            .pcall(&cl, &[])
            .unwrap_or_else(|e| panic!("lua error: {e:?}"));
        assert!(matches!(res.first(), Some(Value::Bool(true))));
    }
}
