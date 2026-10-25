//! The game's built-in registry keys (`registerBase` calls in each key
//! class's static initializer), read from the installed
//! `projectzomboid.jar` with `pz-classfile`. Nothing is shipped: the ids come
//! from the user's own copy of the game at runtime.

use std::collections::HashMap;

use pz_classfile::{load_class, method_effects, method_return, static_inits, Effect, Jar, StaticInit, Value};

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
    ("moodle_type", "zombie/scripting/objects/MoodleType"),
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
pub const METABOLICS_CLASS: &str = "zombie/characters/BodyDamage/Metabolics";
pub const BODY_PART_TYPE_CLASS: &str = "zombie/characters/BodyDamage/BodyPartType";
pub const ITEM_KEY_CLASS: &str = "zombie/scripting/objects/ItemKey";
const ITEM_TYPE_CLASS: &str = "zombie/scripting/objects/ItemType";

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

/// One `ItemKey` constant of a nested holder class (`ItemKey.Container.X`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ItemKeyDecl {
    /// The holder's simple name (`Container`).
    pub holder: String,
    pub field: String,
    /// `ItemKey.id()`.
    pub id: String,
    /// The `ItemType` constant (field name) of `ItemKey.itemType()`.
    pub item_type: String,
}

const STATES_PACKAGE: &str = "zombie/ai/states/";

/// Classes of the `zombie.ai.states` packages with a singleton accessor
/// `public static <Self> instance()`, in jar order.
fn state_singletons(jar: &Jar) -> ScriptResult<Vec<String>> {
    let names: Vec<String> = jar
        .names()
        .filter_map(|n| n.strip_suffix(".class"))
        .filter(|n| n.starts_with(STATES_PACKAGE) && !n.contains('$'))
        .map(str::to_string)
        .collect();
    let mut out = Vec::new();
    for class in names {
        let cf = load_class(jar, &class).map_err(class_error)?;
        let accessor = format!("()L{class};");
        let public_static = pz_classfile::class::ACC_STATIC | 0x0001;
        if cf
            .methods
            .iter()
            .any(|m| m.name == "instance" && m.descriptor == accessor && m.access & public_static == public_static)
        {
            out.push(class);
        }
    }
    Ok(out)
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

const FALLING_WHILE_INJURED_CLASS: &str = "zombie/characters/FallingWhileInjured";

/// `FallingWhileInjured` constants and their damage multipliers
/// (`injured`, `deepInjured`, `fractured`).
fn falling_while_injured(jar: &Jar) -> ScriptResult<Vec<(String, [f32; 3])>> {
    pz_classfile::self_constructed_constants(jar, FALLING_WHILE_INJURED_CLASS)
        .map_err(class_error)?
        .into_iter()
        // (name, ordinal, injured, deepInjured, fractured, isBodyPart)
        .map(|(field, args)| {
            let f = |i: usize| float(args.get(i)).ok_or_else(|| ScriptError::new(format!("FallingWhileInjured.{field}: multiplier")));
            let m = [f(2)?, f(3)?, f(4)?];
            Ok((field, m))
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

/// The `ItemKey` constants of every nested holder class of `ItemKey`
/// (`ItemKey.Container.KEY_RING = container("KeyRing")`): each factory call
/// is evaluated to its `new ItemKey(id, ItemType.X)`.
fn item_keys(jar: &Jar) -> ScriptResult<Vec<ItemKeyDecl>> {
    let prefix = format!("{ITEM_KEY_CLASS}$");
    let holders: Vec<String> = jar
        .names()
        .filter_map(|n| n.strip_suffix(".class")?.strip_prefix(&prefix).map(str::to_string))
        .filter(|h| !h.contains('$') && !h.chars().next().is_some_and(|c| c.is_ascii_digit()))
        .collect();
    let mut out = Vec::new();
    for holder in holders {
        let class = format!("{prefix}{holder}");
        let cf = load_class(jar, &class).map_err(class_error)?;
        for init in static_inits(&cf).map_err(class_error)? {
            if init.class != class {
                continue;
            }
            let made = match &init.value {
                Value::Call { class: c, name, args, .. } if *c == class => method_return(&cf, name, args).map_err(class_error)?,
                v @ Value::New { .. } => Some(v.clone()),
                _ => None,
            };
            let Some(Value::New { class: c, args }) = made else { continue };
            if c != ITEM_KEY_CLASS {
                continue;
            }
            let (Some(id), Some(Value::Static { class: tc, name: item_type })) = (opt_str(args.first()), args.get(1)) else {
                continue;
            };
            if tc == ITEM_TYPE_CLASS {
                out.push(ItemKeyDecl { holder: holder.clone(), field: init.field, id, item_type: item_type.clone() });
            }
        }
    }
    Ok(out)
}

pub const CHARACTER_STAT_CLASS: &str = "zombie/characters/CharacterStat";
pub const MOODLE_STAT_CLASS: &str = "zombie/characters/Moodles/MoodleStat";
pub const MOODLE_TYPE_CLASS: &str = "zombie/scripting/objects/MoodleType";
const ISO_GAME_CHARACTER_CLASS: &str = "zombie/characters/IsoGameCharacter";
const CHEAT_TYPE_CLASS: &str = "zombie/characters/CheatType";

/// One `public static final CharacterStat <field> = register(id, min, max,
/// default)`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CharacterStatDecl {
    pub field: String,
    pub id: String,
    pub minimum: f32,
    pub maximum: f32,
    pub default: f32,
}

/// One `public static final MoodleStat <field> = register(MoodleType.<type>,
/// minimum, lowest, moderate, highest, maximum)`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MoodleStatDecl {
    pub field: String,
    /// The `MoodleType` field passed as the type.
    pub moodle_type: String,
    /// minimum, lowest, moderate, highest, maximum.
    pub thresholds: [f32; 5],
}

/// `CharacterStat`'s registrations and `ORDERED_STATS`.
fn character_stats(jar: &Jar) -> ScriptResult<(Vec<CharacterStatDecl>, Vec<String>)> {
    let cf = load_class(jar, CHARACTER_STAT_CLASS).map_err(class_error)?;
    let mut decls = Vec::new();
    let mut ordered = Vec::new();
    for init in static_inits(&cf).map_err(class_error)? {
        if init.class != CHARACTER_STAT_CLASS {
            continue;
        }
        match &init.value {
            Value::Call { class, name, args, .. } if class == CHARACTER_STAT_CLASS && name == "register" => {
                let bad = || ScriptError::new(format!("CharacterStat.{}: register arguments", init.field));
                decls.push(CharacterStatDecl {
                    id: opt_str(args.first()).ok_or_else(bad)?,
                    minimum: float(args.get(1)).ok_or_else(bad)?,
                    maximum: float(args.get(2)).ok_or_else(bad)?,
                    default: float(args.get(3)).ok_or_else(bad)?,
                    field: init.field.clone(),
                });
            }
            Value::Array(items) if init.field == "ORDERED_STATS" => {
                ordered = items
                    .iter()
                    .map(|v| match v {
                        Value::Static { class, name } if class == CHARACTER_STAT_CLASS => Some(name.clone()),
                        _ => None,
                    })
                    .collect::<Option<_>>()
                    .ok_or_else(|| ScriptError::new("CharacterStat.ORDERED_STATS: unreadable"))?;
            }
            _ => {}
        }
    }
    Ok((decls, ordered))
}

/// `MoodleStat`'s registrations.
fn moodle_stats(jar: &Jar) -> ScriptResult<Vec<MoodleStatDecl>> {
    let cf = load_class(jar, MOODLE_STAT_CLASS).map_err(class_error)?;
    let mut out = Vec::new();
    for init in static_inits(&cf).map_err(class_error)? {
        let Value::Call { class, name, args, .. } = &init.value else { continue };
        if init.class != MOODLE_STAT_CLASS || class != MOODLE_STAT_CLASS || name != "register" {
            continue;
        }
        let bad = || ScriptError::new(format!("MoodleStat.{}: register arguments", init.field));
        let moodle_type = match args.first() {
            Some(Value::Static { class, name }) if class == MOODLE_TYPE_CLASS => name.clone(),
            _ => return Err(bad()),
        };
        let mut thresholds = [0.0; 5];
        for (i, t) in thresholds.iter_mut().enumerate() {
            *t = float(args.get(i + 1)).ok_or_else(bad)?;
        }
        out.push(MoodleStatDecl { field: init.field, moodle_type, thresholds });
    }
    Ok(out)
}

/// `IsoGameCharacter.LevelUpLevels`.
fn level_up_levels(jar: &Jar) -> ScriptResult<Vec<i32>> {
    let bad = || ScriptError::new("IsoGameCharacter.LevelUpLevels: unreadable");
    match pz_classfile::static_value(jar, ISO_GAME_CHARACTER_CLASS, "LevelUpLevels").map_err(class_error)? {
        Some(Value::Array(items)) => items
            .iter()
            .map(|v| match v {
                Value::Int(i) => Some(*i as i32),
                _ => None,
            })
            .collect::<Option<_>>()
            .ok_or_else(bad),
        _ => Err(bad()),
    }
}

const ISO_ZOMBIE_CLASS: &str = "zombie/characters/IsoZombie";

/// `IsoZombie.temporaryMapCloseSneakBonusDir` and
/// `temporaryMapCloseSneakBonusValue`: (sprite, directions, value), in
/// the `Map.ofEntries` order of the directions map.
fn zombie_close_sneak_bonus(jar: &Jar) -> ScriptResult<Vec<(String, String, f32)>> {
    fn entries(v: Option<Value>) -> Option<Vec<(String, Value)>> {
        let Value::Call { name, args, .. } = v? else { return None };
        if name != "ofEntries" {
            return None;
        }
        let Some(Value::Array(items)) = args.into_iter().next() else { return None };
        items
            .into_iter()
            .map(|e| match e {
                Value::Call { name, mut args, .. } if name == "entry" && args.len() == 2 => {
                    let v = args.pop()?;
                    match args.pop()? {
                        Value::Str(k) => Some((k, v)),
                        _ => None,
                    }
                }
                _ => None,
            })
            .collect()
    }
    let bad = || ScriptError::new("IsoZombie.temporaryMapCloseSneakBonus: unreadable");
    let dirs = entries(pz_classfile::static_value(jar, ISO_ZOMBIE_CLASS, "temporaryMapCloseSneakBonusDir").map_err(class_error)?)
        .ok_or_else(bad)?;
    let values =
        entries(pz_classfile::static_value(jar, ISO_ZOMBIE_CLASS, "temporaryMapCloseSneakBonusValue").map_err(class_error)?)
            .ok_or_else(bad)?;
    let value_of = |k: &str| -> Option<f32> {
        let (_, v) = values.iter().find(|(n, _)| n == k)?;
        match v {
            Value::Float(f) => Some(*f as f32),
            Value::Call { args, .. } => match args.first() {
                Some(Value::Float(f)) => Some(*f as f32),
                Some(Value::Int(i)) => Some(*i as f32),
                _ => None,
            },
            _ => None,
        }
    };
    dirs.into_iter()
        .map(|(k, d)| match d {
            Value::Str(d) => value_of(&k).map(|v| (k, d, v)),
            _ => None,
        })
        .collect::<Option<_>>()
        .ok_or_else(bad)
}

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
    /// `Metabolics` constants in declaration order, with `getMet()`.
    pub metabolics: Vec<(String, f32)>,
    /// `BodyPartType` constants in declaration order.
    pub body_parts: Vec<BodyPartDecl>,
    /// `ItemKey`'s nested holder constants, by holder in jar order.
    pub item_keys: Vec<ItemKeyDecl>,
    /// `zombie.ai.states` classes with a static `instance()` singleton
    /// (`a/b/C` form).
    pub state_singletons: Vec<String>,
    /// `CharacterStat.register` calls of the static initializer, in order.
    pub character_stats: Vec<CharacterStatDecl>,
    /// `CharacterStat.ORDERED_STATS`: the `CharacterStat` fields, in order.
    pub ordered_stats: Vec<String>,
    /// `MoodleStat.register` calls of the static initializer, in order.
    pub moodle_stats: Vec<MoodleStatDecl>,
    /// `IsoGameCharacter.LevelUpLevels`.
    pub level_up_levels: Vec<i32>,
    /// `CheatType` constants in declaration order.
    pub cheat_types: Vec<String>,
    /// `IsoZombie`'s close-sneak-bonus sprites: (sprite, directions, value).
    pub zombie_close_sneak_bonus: Vec<(String, String, f32)>,
    /// `FallingWhileInjured` constants in declaration order, with their
    /// `injured`, `deepInjured` and `fractured` multipliers.
    pub falling_while_injured: Vec<(String, [f32; 3])>,
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
        let (character_stats, ordered_stats) = character_stats(jar)?;
        Ok(BuiltinTables {
            perk_ids,
            perks,
            perk_init,
            ragdoll_bone_names,
            vehicle_enums,
            metabolics: metabolics(jar)?,
            body_parts: body_parts(jar)?,
            item_keys: item_keys(jar)?,
            state_singletons: state_singletons(jar)?,
            character_stats,
            ordered_stats,
            moodle_stats: moodle_stats(jar)?,
            level_up_levels: level_up_levels(jar)?,
            cheat_types: pz_classfile::enum_constants(jar, CHEAT_TYPE_CLASS).map_err(class_error)?,
            zombie_close_sneak_bonus: zombie_close_sneak_bonus(jar)?,
            falling_while_injured: falling_while_injured(jar)?,
        })
    }
}
