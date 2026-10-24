//! The game's built-in registry keys (`registerBase` calls in each key
//! class's static initializer), read from the installed
//! `projectzomboid.jar` with `pz-classfile`. Nothing is shipped: the ids come
//! from the user's own copy of the game at runtime.

use std::collections::HashMap;

use pz_classfile::{load_class, method_effects, static_inits, Effect, Jar, StaticInit, Value};

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
const PERK_FACTORY_CLASS: &str = "zombie/characters/skills/PerkFactory";

/// A `PerkFactory.Perks` constant: `public static final Perk <field> = new
/// Perk(id[, parent])`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PerkDecl {
    /// The `Perks` field name.
    pub field: String,
    /// The constructor's `id`.
    pub id: String,
    /// The `Perks` field passed as `parent`, for `Perk(String, Perk)`.
    pub parent: Option<String>,
}

/// One statement of `PerkFactory.init()`. Perks are named by their
/// `PerkFactory.Perks` field.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PerkInit {
    /// `Perks.<perk>.parent = Perks.<parent>`.
    SetParent { perk: String, parent: String },
    /// `AddPerk(perk, translation, [parent,] xp1..xp10[, passiv])`; `parent`
    /// is `None` for the overloads without one (they pass `Perks.None`).
    AddPerk { perk: String, translation: String, parent: Option<String>, xp: [i32; 10], passiv: bool },
}

fn perks_field(v: &Value) -> Option<String> {
    match v {
        Value::Static { class, name } if class == PERKS_CLASS => Some(name.clone()),
        _ => None,
    }
}

/// Decodes the effects of `PerkFactory.init()`.
fn perk_init(effects: Vec<Effect>) -> ScriptResult<Vec<PerkInit>> {
    let bad = |what: &str| ScriptError::new(format!("{PERK_FACTORY_CLASS}.init: unreadable {what}"));
    let mut out = Vec::new();
    for e in effects {
        match e {
            Effect::PutField { object, class, field, value } if class == PERK_CLASS && field == "parent" => {
                let perk = perks_field(&object).ok_or_else(|| bad("parent assignment"))?;
                let parent = perks_field(&value).ok_or_else(|| bad("parent assignment"))?;
                out.push(PerkInit::SetParent { perk, parent });
            }
            Effect::Call(Value::Call { class, name, args, .. }) if class == PERK_FACTORY_CLASS && name == "AddPerk" => {
                let mut args = args.iter();
                let perk = args.next().and_then(perks_field).ok_or_else(|| bad("AddPerk perk"))?;
                let translation = args.next().and_then(Value::as_str).ok_or_else(|| bad("AddPerk translation"))?.to_string();
                let rest: Vec<&Value> = args.collect();
                let (parent, rest) = match rest.first() {
                    Some(v @ Value::Static { .. }) => (Some(perks_field(v).ok_or_else(|| bad("AddPerk parent"))?), &rest[1..]),
                    _ => (None, &rest[..]),
                };
                let ints: Vec<i64> = rest
                    .iter()
                    .map(|v| match v {
                        Value::Int(i) => Some(*i),
                        _ => None,
                    })
                    .collect::<Option<_>>()
                    .ok_or_else(|| bad("AddPerk xp"))?;
                let (xp, passiv) = match ints.len() {
                    10 => (&ints[..], false),
                    11 => (&ints[..10], ints[10] != 0),
                    _ => return Err(bad("AddPerk arguments")),
                };
                let mut table = [0; 10];
                for (t, v) in table.iter_mut().zip(xp) {
                    *t = *v as i32;
                }
                out.push(PerkInit::AddPerk { perk, translation, parent, xp: table, passiv });
            }
            _ => {}
        }
    }
    Ok(out)
}
const RAGDOLL_CLASS: &str = "zombie/scripting/objects/RagdollScript";

/// Built-in name tables the original defines in code, read from the jar.
#[derive(Clone, Debug, Default)]
pub struct BuiltinTables {
    /// `PerkFactory.Perks` ids in declaration order.
    pub perk_ids: Vec<String>,
    /// `PerkFactory.Perks` constants in declaration (construction) order.
    pub perks: Vec<PerkDecl>,
    /// `PerkFactory.init()`, statement by statement.
    pub perk_init: Vec<PerkInit>,
    /// `RagdollScript.boneNameArray`.
    pub ragdoll_bone_names: Vec<String>,
    /// [`VEHICLE_ENUM_CLASSES`] constants: (class, [(constant, id)]).
    pub vehicle_enums: Vec<(String, Vec<(String, String)>)>,
}

impl BuiltinTables {
    /// `PerkFactory.Perks.FromString`: the perk id, or `"MAX"` when unknown.
    pub fn perk_from_string(&self, id: &str) -> String {
        if self.perk_ids.iter().any(|p| p == id) {
            id.to_string()
        } else {
            "MAX".to_string()
        }
    }

    pub fn load(jar: &Jar) -> ScriptResult<Self> {
        let perks_class = load_class(jar, PERKS_CLASS).map_err(class_error)?;
        let perks: Vec<PerkDecl> = static_inits(&perks_class)
            .map_err(class_error)?
            .into_iter()
            .filter_map(|i| match i.value {
                Value::New { class, args } if class == PERK_CLASS && i.class == PERKS_CLASS => Some(PerkDecl {
                    field: i.field,
                    id: args.first().and_then(Value::as_str)?.to_string(),
                    parent: args.get(1).and_then(perks_field),
                }),
                _ => None,
            })
            .collect();
        let perk_ids = perks.iter().map(|p| p.id.clone()).collect();
        let factory = load_class(jar, PERK_FACTORY_CLASS).map_err(class_error)?;
        let perk_init = perk_init(method_effects(&factory, "init").map_err(class_error)?)?;
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
        Ok(BuiltinTables { perk_ids, perks, perk_init, ragdoll_bone_names, vehicle_enums })
    }
}
