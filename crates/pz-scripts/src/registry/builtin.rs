//! The game's built-in registry keys (`registerBase` calls in each key
//! class's static initializer), read from the installed
//! `projectzomboid.jar` with `pz-classfile`. Nothing is shipped: the ids come
//! from the user's own copy of the game at runtime.

use std::collections::HashMap;

use pz_classfile::{load_class, static_inits, Jar, StaticInit, Value};

use super::Registries;
use crate::base::{ScriptError, ScriptResult};

/// Registry name -> key class whose initializer registers its base keys.
pub const KEY_CLASSES: &[(&str, &str)] = &[
    ("ammo_type", "zombie/scripting/objects/AmmoType"),
    ("book_subject", "zombie/scripting/objects/BookSubject"),
    ("character_profession", "zombie/scripting/objects/CharacterProfession"),
    ("character_trait", "zombie/scripting/objects/CharacterTrait"),
    ("game_mode", "zombie/core/GameMode"),
    ("item_body_location", "zombie/scripting/objects/ItemBodyLocation"),
    ("item_tag", "zombie/scripting/objects/ItemTag"),
    ("item_type", "zombie/scripting/objects/ItemType"),
    ("magazine_subject", "zombie/scripting/objects/MagazineSubject"),
    ("weapon_category", "zombie/scripting/objects/WeaponCategory"),
];

fn class_error(e: pz_classfile::Error) -> ScriptError {
    ScriptError::new(format!("projectzomboid.jar: {e}"))
}

/// Resolves `ItemKey.X.CONST` to `ItemKey.toString()` (`Base.<id>`).
struct ItemKeys<'a> {
    jar: &'a Jar,
    classes: HashMap<String, Vec<StaticInit>>,
}

impl ItemKeys<'_> {
    fn resolve(&mut self, class: &str, field: &str) -> ScriptResult<Option<String>> {
        if !self.classes.contains_key(class) {
            let cf = load_class(self.jar, class).map_err(class_error)?;
            self.classes.insert(class.to_string(), static_inits(&cf).map_err(class_error)?);
        }
        let inits = &self.classes[class];
        Ok(inits.iter().find(|s| s.field == field).and_then(|s| match &s.value {
            Value::Call { args, .. } | Value::New { args, .. } => args.first().and_then(Value::as_str).map(|id| format!("Base.{id}")),
            _ => None,
        }))
    }
}

/// Registers every built-in key of [`KEY_CLASSES`] into `registries`, in the
/// order the original's static initializers run.
pub fn register_builtin(registries: &Registries, jar: &Jar) -> ScriptResult {
    let mut item_keys = ItemKeys { jar, classes: HashMap::new() };
    for (registry, class) in KEY_CLASSES {
        let cf = load_class(jar, class).map_err(class_error)?;
        let mut reg = registries.get(registry).expect("known registry").borrow_mut();
        for init in static_inits(&cf).map_err(class_error)? {
            let Value::Call { name, args, .. } = &init.value else { continue };
            if name != "registerBase" {
                continue;
            }
            let Some(id) = args.first().and_then(Value::as_str) else {
                return Err(ScriptError::new(format!("{class}.{}: unreadable registerBase id", init.field)));
            };
            let item_key = match args.get(1) {
                Some(Value::Static { class, name }) => item_keys.resolve(class, name)?,
                _ => None,
            };
            let loc = super::create_location(id, true)?;
            reg.register(loc, id, item_key.as_deref())?;
        }
    }
    Ok(())
}

/// Enums the vehicle scripts name parts, areas and seats with, as exposed to
/// Lua: (class, [(constant, id)]) where `id` is the constructor's string.
pub const VEHICLE_ENUM_CLASSES: &[&str] = &[
    "zombie/scripting/objects/VehicleArea",
    "zombie/scripting/objects/VehicleAttachment",
    "zombie/scripting/objects/VehicleCategory",
    "zombie/scripting/objects/VehicleEngineRpmType",
    "zombie/scripting/objects/VehicleItemEquip",
    "zombie/scripting/objects/VehicleMechanicArea",
    "zombie/scripting/objects/VehiclePart",
    "zombie/scripting/objects/VehiclePassenger",
    "zombie/scripting/objects/VehiclePosition",
    "zombie/scripting/objects/VehicleSwitchSeat",
    "zombie/scripting/objects/VehicleTable",
    "zombie/scripting/objects/VehicleWheel",
];

const PERKS_CLASS: &str = "zombie/characters/skills/PerkFactory$Perks";
const PERK_CLASS: &str = "zombie/characters/skills/PerkFactory$Perk";
const RAGDOLL_CLASS: &str = "zombie/scripting/objects/RagdollScript";
pub const METABOLICS_CLASS: &str = "zombie/characters/BodyDamage/Metabolics";
pub const BODY_PART_TYPE_CLASS: &str = "zombie/characters/BodyDamage/BodyPartType";

/// One `BodyPartType` constant: its constructor arguments.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BodyPartDecl {
    /// The enum constant (`name()`).
    pub field: String,
    /// The `name` field (`ToString`, `FromString`).
    pub name: String,
    pub display_name_key: Option<String>,
    pub bandage_model: Option<String>,
    pub bite_wound_model: Option<String>,
    pub scratch_wound_model: Option<String>,
    pub cut_wound_model: Option<String>,
    pub pain_modifier: f32,
    pub damage_modifier: f32,
    pub bleeding_time_modifier: f32,
    pub skin_surface: f32,
    pub dist_to_core: f32,
    pub umbrella_mod: f32,
    pub max_action_penalty: f32,
    pub max_movement_penalty: f32,
}

fn opt_str(v: Option<&Value>) -> Option<String> {
    v.and_then(Value::as_str).map(str::to_string)
}

fn float(v: Option<&Value>) -> Option<f32> {
    match v? {
        Value::Float(f) => Some(*f as f32),
        Value::Int(i) => Some(*i as f32),
        _ => None,
    }
}

/// `Metabolics` constants and their `met`.
fn metabolics(jar: &Jar) -> ScriptResult<Vec<(String, f32)>> {
    pz_classfile::self_constructed_constants(jar, METABOLICS_CLASS)
        .map_err(class_error)?
        .into_iter()
        // (name, ordinal, met)
        .map(|(field, args)| {
            let met = float(args.get(2)).ok_or_else(|| ScriptError::new(format!("Metabolics.{field}: met")))?;
            Ok((field, met))
        })
        .collect()
}

/// `BodyPartType` constants with their constructor arguments.
fn body_parts(jar: &Jar) -> ScriptResult<Vec<BodyPartDecl>> {
    pz_classfile::self_constructed_constants(jar, BODY_PART_TYPE_CLASS)
        .map_err(class_error)?
        .into_iter()
        .map(|(field, args)| {
            // (enum name, ordinal, name, displayNameKey, bandageModel,
            // biteWoundModel, scratchWoundModel, cutWoundModel, 8 floats)
            let bad = || ScriptError::new(format!("BodyPartType.{field}: constructor arguments"));
            let f = |i: usize| float(args.get(i)).ok_or_else(bad);
            Ok(BodyPartDecl {
                name: opt_str(args.get(2)).ok_or_else(bad)?,
                display_name_key: opt_str(args.get(3)),
                bandage_model: opt_str(args.get(4)),
                bite_wound_model: opt_str(args.get(5)),
                scratch_wound_model: opt_str(args.get(6)),
                cut_wound_model: opt_str(args.get(7)),
                pain_modifier: f(8)?,
                damage_modifier: f(9)?,
                bleeding_time_modifier: f(10)?,
                skin_surface: f(11)?,
                dist_to_core: f(12)?,
                umbrella_mod: f(13)?,
                max_action_penalty: f(14)?,
                max_movement_penalty: f(15)?,
                field,
            })
        })
        .collect()
}

/// Built-in name tables the original defines in code, read from the jar.
#[derive(Clone, Debug, Default)]
pub struct BuiltinTables {
    /// `PerkFactory.Perks` ids in declaration order.
    pub perk_ids: Vec<String>,
    /// `RagdollScript.boneNameArray`.
    pub ragdoll_bone_names: Vec<String>,
    /// [`VEHICLE_ENUM_CLASSES`] constants: (class, [(constant, id)]).
    pub vehicle_enums: Vec<(String, Vec<(String, String)>)>,
    /// `Metabolics` constants in declaration order, with `getMet()`.
    pub metabolics: Vec<(String, f32)>,
    /// `BodyPartType` constants in declaration order.
    pub body_parts: Vec<BodyPartDecl>,
}

impl BuiltinTables {
    /// `Metabolics.valueOf`: `name` if it is a constant.
    pub fn metabolics_value_of(&self, name: &str) -> ScriptResult<String> {
        if self.metabolics.iter().any(|(n, _)| n == name) {
            return Ok(name.to_string());
        }
        Err(ScriptError::new(format!(
            "java.lang.IllegalArgumentException: No enum constant zombie.characters.BodyDamage.Metabolics.{name}"
        )))
    }

    /// `BodyPartType.FromString`: the constant whose `name` equals `s`
    /// ignoring case, else `MAX`.
    pub fn body_part_from_string(&self, s: &str) -> String {
        self.body_parts
            .iter()
            .find(|p| crate::java::eq_ignore_case(&p.name, s))
            .map_or_else(|| "MAX".to_string(), |p| p.field.clone())
    }

    /// `PerkFactory.Perks.FromString`: the perk id, or `"MAX"` when unknown.
    pub fn perk_from_string(&self, id: &str) -> String {
        if self.perk_ids.iter().any(|p| p == id) {
            id.to_string()
        } else {
            "MAX".to_string()
        }
    }

    pub fn load(jar: &Jar) -> ScriptResult<Self> {
        let perks = load_class(jar, PERKS_CLASS).map_err(class_error)?;
        let perk_ids = static_inits(&perks)
            .map_err(class_error)?
            .into_iter()
            .filter_map(|i| match i.value {
                Value::New { class, args } if class == PERK_CLASS => args.first().and_then(Value::as_str).map(str::to_string),
                _ => None,
            })
            .collect();
        let bones = pz_classfile::static_value(jar, RAGDOLL_CLASS, "boneNameArray").map_err(class_error)?;
        let ragdoll_bone_names = bones
            .as_ref()
            .and_then(Value::as_str_array)
            .map(|v| v.into_iter().map(str::to_string).collect())
            .unwrap_or_default();
        let mut vehicle_enums = Vec::new();
        for class in VEHICLE_ENUM_CLASSES {
            let consts = pz_classfile::self_constructed_constants(jar, class)
                .map_err(class_error)?
                .into_iter()
                .filter_map(|(name, args)| Some((name, args.get(2)?.as_str()?.to_string())))
                .collect();
            vehicle_enums.push((class.rsplit('/').next().unwrap().to_string(), consts));
        }
        Ok(BuiltinTables {
            perk_ids,
            ragdoll_bone_names,
            vehicle_enums,
            metabolics: metabolics(jar)?,
            body_parts: body_parts(jar)?,
        })
    }
}
