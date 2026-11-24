//! Build 42.22 scripting/entity enums and registries exposed to Lua:
//! - `zombie.scripting.objects.WeaponReloadType`
//! - `zombie.entity.GameEntityType`
//! - `zombie.network.fields.ContainerID$ContainerType` (global `ContainerType`)
//! - `zombie.entity.components.attributes.AttributeValueType`
//! - `zombie.scripting.objects.DigType`
//! - `zombie.scripting.ui.TextAlign`
//! - `zombie.scripting.objects.Registries` & `zombie.scripting.objects.Registry`
//! - `zombie.scripting.objects.Brochure`, `Flier`, `Newspaper`, `ModelKey`

use crate::bind_items::on_create_data::{self, Entry};
use crate::bind_java_util::JavaList;
use crate::expose::{expose_class, expose_enum, int_arg, str_arg, JavaEnum};
use pz_classfile::{Effect, Jar, Value as JVal};
use pz_kahlua::value::LuaObject;
use pz_kahlua::{Thread, Value};
use std::any::Any;
use std::rc::Rc;

pub const WEAPON_RELOAD_TYPE_CLASS: &str = "zombie.scripting.objects.WeaponReloadType";
pub const GAME_ENTITY_TYPE_CLASS: &str = "zombie.entity.GameEntityType";
pub const CONTAINER_TYPE_CLASS: &str = "zombie.network.fields.ContainerID$ContainerType";
pub const ATTRIBUTE_VALUE_TYPE_CLASS: &str =
    "zombie.entity.components.attributes.AttributeValueType";
pub const DIG_TYPE_CLASS: &str = "zombie.scripting.objects.DigType";
pub const TEXT_ALIGN_CLASS: &str = "zombie.scripting.ui.TextAlign";
pub const REGISTRIES_CLASS: &str = "zombie.scripting.objects.Registries";
pub const REGISTRY_CLASS: &str = "zombie.scripting.objects.Registry";
pub const BROCHURE_CLASS: &str = "zombie.scripting.objects.Brochure";
pub const FLIER_CLASS: &str = "zombie.scripting.objects.Flier";
pub const NEWSPAPER_CLASS: &str = "zombie.scripting.objects.Newspaper";
pub const MODEL_KEY_CLASS: &str = "zombie.scripting.objects.ModelKey";

const WEAPON_RELOAD_TYPES: &[(&str, &str)] = &[
    ("NONE", ""),
    ("BOLT_ACTION", "boltaction"),
    ("BOLT_ACTION_NO_MAG", "boltactionnomag"),
    ("DOUBLE_BARREL_SHOTGUN", "doublebarrelshotgun"),
    ("DOUBLE_BARREL_SHOTGUN_SAWN", "doublebarrelshotgunsawn"),
    ("HANDGUN", "handgun"),
    ("LEVER_ACTION", "leveraction"),
    ("REVOLVER", "revolver"),
    ("SHOTGUN", "shotgun"),
];

const GAME_ENTITY_TYPES: &[&str] = &[
    "IsoObject",
    "InventoryItem",
    "VehiclePart",
    "IsoMovingObject",
    "Template",
    "MetaEntity",
];

const CONTAINER_TYPES: &[&str] = &[
    "Undefined",
    "DeadBody",
    "WorldObject",
    "IsoObject",
    "ObjectContainer",
    "ObjectInVehicle",
    "Vehicle",
    "PlayerInventory",
    "InventoryContainer",
    "Floor",
];

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

const DIG_TYPES: &[(&str, &str)] = &[
    ("HOE", "Hoe"),
    ("PICK_AXE", "PickAxe"),
    ("SHOVEL", "Shovel"),
    ("TROWEL", "Trowel"),
];

const TEXT_ALIGNS: &[&str] = &["Left", "Center", "Right"];

/// Mapping from `Registries.<FIELD>` static name to the `on_create_data`
/// registry key and Java entry class name.
const DATA_REGISTRIES: &[(&str, &str, &str)] = &[
    ("BROCHURE", "Brochure", BROCHURE_CLASS),
    ("FLIER", "Flier", FLIER_CLASS),
    ("NEWSPAPER", "Newspaper", NEWSPAPER_CLASS),
    ("OLD_NEWSPAPER", "OldNewspaper", "zombie.scripting.objects.OldNewspaper"),
    ("BOOK", "Book", "zombie.scripting.objects.Book"),
    ("BOOK_SUBJECT", "BookSubject", "zombie.scripting.objects.BookSubject"),
    ("MAGAZINE", "Magazine", "zombie.scripting.objects.Magazine"),
    ("MAGAZINE_SUBJECT", "MagazineSubject", "zombie.scripting.objects.MagazineSubject"),
    ("COMIC_BOOK", "ComicBook", "zombie.scripting.objects.ComicBook"),
    ("CATALOGUE", "Catalogue", "zombie.scripting.objects.Catalogue"),
    ("RPG_MANUAL", "RpgManual", "zombie.scripting.objects.RpgManual"),
    ("LOCKET", "Locket", "zombie.scripting.objects.Locket"),
    ("DOODLE", "Doodle", "zombie.scripting.objects.Doodle"),
    ("DOODLE_KIDS", "DoodleKids", "zombie.scripting.objects.DoodleKids"),
    ("POSTCARD", "Postcard", "zombie.scripting.objects.Postcard"),
    ("JOB", "Job", "zombie.scripting.objects.Job"),
    ("LETTER", "Letter", "zombie.scripting.objects.Letter"),
    ("BUSINESS", "Business", "zombie.scripting.objects.Business"),
    ("PET_NAME", "PetName", "zombie.scripting.objects.PetName"),
    ("PHOTO", "Photo", "zombie.scripting.objects.Photo"),
];

struct LuaRegistry {
    data_key: &'static str,
    entry_class: &'static str,
}

impl LuaObject for LuaRegistry {
    fn class_name(&self) -> &str {
        REGISTRY_CLASS
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

struct LuaPrintMediaEntry {
    class: &'static str,
    name: String,
    translation_key: String,
    translation_info_key: String,
    translation_text_key: String,
    issues: Vec<String>,
}

impl LuaPrintMediaEntry {
    fn from_entry(class: &'static str, e: &Entry) -> Self {
        let tk = e.key("translationKey");
        let tik = e.key("translationInfoKey");
        let ttk = e.key("translationTextKey");
        Self {
            class,
            name: e.name.clone(),
            translation_key: if tk.is_empty() {
                format!("Print_Media_{}_title", e.name)
            } else {
                tk.to_owned()
            },
            translation_info_key: if tik.is_empty() {
                format!("Print_Media_{}_info", e.name)
            } else {
                tik.to_owned()
            },
            translation_text_key: if ttk.is_empty() {
                format!("Print_Text_{}_info", e.name)
            } else {
                ttk.to_owned()
            },
            issues: e.issues.clone(),
        }
    }
}

impl LuaObject for LuaPrintMediaEntry {
    fn class_name(&self) -> &str {
        self.class
    }
    fn to_lua_string(&self) -> String {
        self.name.clone()
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

struct LuaModelKey(String);

impl LuaObject for LuaModelKey {
    fn class_name(&self) -> &str {
        MODEL_KEY_CLASS
    }
    fn to_lua_string(&self) -> String {
        self.0.clone()
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

fn loc_path_from_value(v: &Value) -> Option<String> {
    let raw = match v {
        Value::Str(s) => s.to_string(),
        Value::Object(o) => o.to_lua_string(),
        _ => return None,
    };
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    let path = match trimmed.split_once(':') {
        Some((_, p)) => p,
        None => trimmed,
    };
    Some(path.to_ascii_lowercase())
}

fn lookup_media_entry(data_key: &'static str, entry_class: &'static str, v: &Value) -> Value {
    let Some(path) = loc_path_from_value(v) else {
        return Value::Nil;
    };
    let Some(tables) = on_create_data::maybe_tables() else {
        return Value::Nil;
    };
    let Ok(entries) = tables.values(data_key) else {
        return Value::Nil;
    };
    entries
        .iter()
        .find(|e| e.path() == path || e.name.eq_ignore_ascii_case(&path))
        .map(|e| Value::Object(Rc::new(LuaPrintMediaEntry::from_entry(entry_class, e))))
        .unwrap_or(Value::Nil)
}

fn registry_values(data_key: &'static str, entry_class: &'static str) -> Vec<Value> {
    let Some(tables) = on_create_data::maybe_tables() else {
        return Vec::new();
    };
    let Ok(entries) = tables.values(data_key) else {
        return Vec::new();
    };
    entries
        .iter()
        .map(|e| Value::Object(Rc::new(LuaPrintMediaEntry::from_entry(entry_class, e))))
        .collect()
}

pub fn register(thread: &mut Thread, jar: Option<&Jar>) {
    register_weapon_reload_type(thread);
    register_game_entity_type(thread);
    register_container_type(thread);
    register_attribute_value_type(thread);
    register_dig_type(thread);
    register_text_align(thread);
    register_registries(thread);
    register_model_key(thread, jar);
}

fn register_weapon_reload_type(thread: &mut Thread) {
    let names: Vec<&'static str> = WEAPON_RELOAD_TYPES.iter().map(|(n, _)| *n).collect();
    let b = expose_enum(thread, WEAPON_RELOAD_TYPE_CLASS, &names);
    b.method("toString", |c| {
        let s = match c.arg(0) {
            Value::Object(o) => o
                .as_any()
                .downcast_ref::<JavaEnum>()
                .and_then(|e| {
                    WEAPON_RELOAD_TYPES
                        .iter()
                        .find(|(n, _)| *n == e.name)
                        .map(|(_, s)| *s)
                })
                .unwrap_or(""),
            _ => "",
        };
        Ok(c.push(Value::str(s)))
    });
    b.static_fn("fromValue", |c| {
        let raw = str_arg(c, 0).map(|s| s.to_string()).unwrap_or_default();
        let target = WEAPON_RELOAD_TYPES
            .iter()
            .find(|(n, v)| v.eq_ignore_ascii_case(&raw) || n.eq_ignore_ascii_case(&raw))
            .map(|(n, _)| *n);
        let out = target
            .and_then(|name| {
                c.thread
                    .globals()
                    .get_str("WeaponReloadType")
                    .as_table()
                    .map(|t| t.get_str(name))
            })
            .unwrap_or(Value::Nil);
        Ok(c.push(out))
    });
}

fn register_game_entity_type(thread: &mut Thread) {
    let b = expose_enum(thread, GAME_ENTITY_TYPE_CLASS, GAME_ENTITY_TYPES);
    b.method("getId", |c| {
        let id = match c.arg(0) {
            Value::Object(o) => o
                .as_any()
                .downcast_ref::<JavaEnum>()
                .map(|e| e.ordinal + 1)
                .unwrap_or(0),
            _ => 0,
        };
        Ok(c.push(id))
    });
    b.method("getByteId", |c| {
        let id = match c.arg(0) {
            Value::Object(o) => o
                .as_any()
                .downcast_ref::<JavaEnum>()
                .map(|e| e.ordinal + 1)
                .unwrap_or(0),
            _ => 0,
        };
        Ok(c.push(id))
    });
    b.method("getBits", |c| {
        let bits = match c.arg(0) {
            Value::Object(o) => o
                .as_any()
                .downcast_ref::<JavaEnum>()
                .map(|e| 1i32 << e.ordinal)
                .unwrap_or(0),
            _ => 0,
        };
        Ok(c.push(bits))
    });
    b.static_fn("FromID", |c| {
        let id = int_arg(c, 0);
        let name = if id >= 1 {
            GAME_ENTITY_TYPES.get((id - 1) as usize).copied()
        } else {
            None
        };
        let out = name
            .and_then(|n| {
                c.thread
                    .globals()
                    .get_str("GameEntityType")
                    .as_table()
                    .map(|t| t.get_str(n))
            })
            .unwrap_or(Value::Nil);
        Ok(c.push(out))
    });
}

fn register_container_type(thread: &mut Thread) {
    let _b = expose_enum(thread, CONTAINER_TYPE_CLASS, CONTAINER_TYPES);
    // Also alias under global `ContainerType` (nested class simple name).
    if let Value::Table(tbl) = thread.globals().get_str("ContainerType") {
        let _ = tbl;
    }
}

fn register_attribute_value_type(thread: &mut Thread) {
    let b = expose_enum(thread, ATTRIBUTE_VALUE_TYPE_CLASS, ATTRIBUTE_VALUE_TYPES);
    b.static_fn("IsNumeric", |c| {
        let is_num = match c.arg(0) {
            Value::Object(o) => o
                .as_any()
                .downcast_ref::<JavaEnum>()
                .is_some_and(|e| {
                    matches!(
                        e.name,
                        "Float" | "Double" | "Byte" | "Short" | "Int" | "Long"
                    )
                }),
            _ => false,
        };
        Ok(c.push(is_num))
    });
    b.static_fn("IsDecimal", |c| {
        let is_dec = match c.arg(0) {
            Value::Object(o) => o
                .as_any()
                .downcast_ref::<JavaEnum>()
                .is_some_and(|e| matches!(e.name, "Float" | "Double")),
            _ => false,
        };
        Ok(c.push(is_dec))
    });
}

fn register_dig_type(thread: &mut Thread) {
    let names: Vec<&'static str> = DIG_TYPES.iter().map(|(n, _)| *n).collect();
    let b = expose_enum(thread, DIG_TYPE_CLASS, &names);
    b.method("toString", |c| {
        let s = match c.arg(0) {
            Value::Object(o) => o
                .as_any()
                .downcast_ref::<JavaEnum>()
                .and_then(|e| DIG_TYPES.iter().find(|(n, _)| *n == e.name).map(|(_, s)| *s))
                .unwrap_or(""),
            _ => "",
        };
        Ok(c.push(Value::str(s)))
    });
}

fn register_text_align(thread: &mut Thread) {
    let _b = expose_enum(thread, TEXT_ALIGN_CLASS, TEXT_ALIGNS);
}

fn register_media_class(thread: &mut Thread, class: &'static str, data_key: &'static str) {
    let cls = expose_class(thread, class, Some("java.lang.Object"));
    cls.static_fn("get", move |c| {
        let out = lookup_media_entry(data_key, class, &c.arg(0));
        Ok(c.push(out))
    });
    cls.method("getTranslationKey", |c| {
        let s = match c.arg(0) {
            Value::Object(o) => o
                .as_any()
                .downcast_ref::<LuaPrintMediaEntry>()
                .map(|e| e.translation_key.clone())
                .unwrap_or_default(),
            _ => String::new(),
        };
        Ok(c.push(Value::str(&s)))
    });
    cls.method("getTranslationInfoKey", |c| {
        let s = match c.arg(0) {
            Value::Object(o) => o
                .as_any()
                .downcast_ref::<LuaPrintMediaEntry>()
                .map(|e| e.translation_info_key.clone())
                .unwrap_or_default(),
            _ => String::new(),
        };
        Ok(c.push(Value::str(&s)))
    });
    cls.method("getTranslationTextKey", |c| {
        let s = match c.arg(0) {
            Value::Object(o) => o
                .as_any()
                .downcast_ref::<LuaPrintMediaEntry>()
                .map(|e| e.translation_text_key.clone())
                .unwrap_or_default(),
            _ => String::new(),
        };
        Ok(c.push(Value::str(&s)))
    });
    cls.method("getTitle", |c| {
        let s = match c.arg(0) {
            Value::Object(o) => o
                .as_any()
                .downcast_ref::<LuaPrintMediaEntry>()
                .map(|e| e.name.clone())
                .unwrap_or_default(),
            _ => String::new(),
        };
        Ok(c.push(Value::str(&s)))
    });
    cls.method("getIssues", |c| {
        let items: Vec<Value> = match c.arg(0) {
            Value::Object(o) => o
                .as_any()
                .downcast_ref::<LuaPrintMediaEntry>()
                .map(|e| e.issues.iter().map(|i| Value::str(i.as_str())).collect())
                .unwrap_or_default(),
            _ => Vec::new(),
        };
        Ok(c.push(Value::Object(Rc::new(JavaList::new(items)))))
    });
    cls.method("toString", |c| {
        let s = match c.arg(0) {
            Value::Object(o) => o
                .as_any()
                .downcast_ref::<LuaPrintMediaEntry>()
                .map(|e| e.name.clone())
                .unwrap_or_default(),
            _ => String::new(),
        };
        Ok(c.push(Value::str(&s)))
    });
}

fn register_registries(thread: &mut Thread) {
    let reg = expose_class(thread, REGISTRY_CLASS, Some("java.lang.Object"));
    reg.method("values", |c| {
        let items = match c.arg(0) {
            Value::Object(o) => o
                .as_any()
                .downcast_ref::<LuaRegistry>()
                .map(|r| registry_values(r.data_key, r.entry_class))
                .unwrap_or_default(),
            _ => Vec::new(),
        };
        Ok(c.push(Value::Object(Rc::new(JavaList::new(items)))))
    });
    reg.method("get", |c| {
        let out = match c.arg(0) {
            Value::Object(o) => o
                .as_any()
                .downcast_ref::<LuaRegistry>()
                .map(|r| lookup_media_entry(r.data_key, r.entry_class, &c.arg(1)))
                .unwrap_or(Value::Nil),
            _ => Value::Nil,
        };
        Ok(c.push(out))
    });
    reg.method("contains", |c| {
        let found = match c.arg(0) {
            Value::Object(o) => o
                .as_any()
                .downcast_ref::<LuaRegistry>()
                .is_some_and(|r| !lookup_media_entry(r.data_key, r.entry_class, &c.arg(1)).is_nil()),
            _ => false,
        };
        Ok(c.push(found))
    });

    for &(_, data_key, entry_class) in DATA_REGISTRIES {
        register_media_class(thread, entry_class, data_key);
    }

    let regs = expose_class(thread, REGISTRIES_CLASS, Some("java.lang.Object"));
    for &(field, data_key, entry_class) in DATA_REGISTRIES {
        regs.field(
            field,
            Value::Object(Rc::new(LuaRegistry {
                data_key,
                entry_class,
            })),
        );
    }
}

fn register_model_key(thread: &mut Thread, jar: Option<&Jar>) {
    let mk = expose_class(thread, MODEL_KEY_CLASS, Some("java.lang.Object"));
    mk.method("toString", |c| {
        let s = match c.arg(0) {
            Value::Object(o) => o
                .as_any()
                .downcast_ref::<LuaModelKey>()
                .map(|k| k.0.clone())
                .unwrap_or_default(),
            _ => String::new(),
        };
        Ok(c.push(Value::str(&s)))
    });
    mk.static_fn("fromString", |c| {
        let s = str_arg(c, 0).map(|s| s.to_string()).unwrap_or_default();
        Ok(c.push(Value::Object(Rc::new(LuaModelKey(s)))))
    });

    // Populate ModelKey static fields dynamically from the installed jar's
    // `zombie/scripting/objects/ModelKey.<clinit>` without hardcoding assets.
    if let Some(jar) = jar {
        if let Ok(cf) = pz_classfile::load_class(jar, "zombie/scripting/objects/ModelKey") {
            if let Ok(effects) = pz_classfile::method_effects(&cf, "<clinit>") {
                for effect in effects {
                    let Effect::PutStatic(init) = effect else {
                        continue;
                    };
                    if init.class != "zombie/scripting/objects/ModelKey" {
                        continue;
                    }
                    let id = match &init.value {
                        JVal::Call { args, .. } | JVal::New { args, .. } => {
                            args.first().and_then(JVal::as_str).map(str::to_owned)
                        }
                        other => other.as_str().map(str::to_owned),
                    };
                    if let Some(id) = id {
                        mk.field(&init.field, Value::Object(Rc::new(LuaModelKey(id))));
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pz_kahlua::stdlib;

    #[test]
    fn scripting_enums_and_registries_lua() {
        let mut t = Thread::new(pz_kahlua::TableRef::new());
        stdlib::install(&mut t, None).unwrap();
        crate::bind_java_util::register(&mut t);
        register(&mut t, None);

        let src = r#"
            local function assert(v, msg)
                if not v then error(msg or "assertion failed") end
            end

            -- WeaponReloadType
            assert(WeaponReloadType.SHOTGUN ~= nil)
            assert(WeaponReloadType.SHOTGUN:toString() == "shotgun")
            assert(WeaponReloadType.DOUBLE_BARREL_SHOTGUN:toString() == "doublebarrelshotgun")
            assert(WeaponReloadType.fromValue("revolver") == WeaponReloadType.REVOLVER)
            assert(WeaponReloadType.fromValue("boltactionnomag") == WeaponReloadType.BOLT_ACTION_NO_MAG)

            -- GameEntityType
            assert(GameEntityType.IsoObject:getId() == 1)
            assert(GameEntityType.VehiclePart:getId() == 3)
            assert(GameEntityType.FromID(1) == GameEntityType.IsoObject)
            assert(GameEntityType.IsoObject:getBits() == 1)
            assert(GameEntityType.VehiclePart:getBits() == 4)

            -- ContainerType
            assert(ContainerType.IsoObject ~= nil)
            assert(ContainerType.IsoObject ~= ContainerType.Floor)
            assert(ContainerType.Vehicle ~= nil)

            -- AttributeValueType
            assert(AttributeValueType.IsNumeric(AttributeValueType.Int) == true)
            assert(AttributeValueType.IsNumeric(AttributeValueType.Float) == true)
            assert(AttributeValueType.IsNumeric(AttributeValueType.String) == false)
            assert(AttributeValueType.IsNumeric(AttributeValueType.Boolean) == false)
            assert(AttributeValueType.IsDecimal(AttributeValueType.Double) == true)
            assert(AttributeValueType.IsDecimal(AttributeValueType.Int) == false)

            -- DigType
            assert(DigType.TROWEL:toString() == "Trowel")
            assert(DigType.SHOVEL:toString() == "Shovel")
            assert(DigType.HOE:toString() == "Hoe")
            assert(DigType.PICK_AXE:toString() == "PickAxe")

            -- TextAlign
            assert(TextAlign.Left ~= TextAlign.Center)
            assert(TextAlign.Right ~= TextAlign.Center)

            -- Registries & Brochure/Flier/Newspaper & ModelKey
            assert(Registries.FLIER ~= nil)
            assert(Registries.BROCHURE ~= nil)
            assert(Registries.NEWSPAPER ~= nil)
            assert(Registries.FLIER:values():size() == 0)
            assert(Brochure.get("base:missing") == nil)
            assert(Flier.get("base:missing") == nil)
            local mk = ModelKey.fromString("TinCanEmpty")
            assert(mk:toString() == "TinCanEmpty")
        "#;
        let f = stdlib::load_string(src, Some("scripting_enums_registries"), t.globals().clone())
            .unwrap();
        t.pcall(&f, &[]).unwrap();
    }
}
