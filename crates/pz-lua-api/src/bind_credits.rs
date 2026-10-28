//! `zombie.core.CreditsName`, `CreditsRole` and `CreditsRoleGroup`: the
//! credits roll `CreditsScreen.lua` builds, and the translator credits read
//! from each language's `Credits_Translator.json`.
//!
//! The constants (names, roles, groups) are read at run time from the
//! enums' static initializers in the installed `projectzomboid.jar`
//! ([`CreditsData::load`]); nothing of them is shipped.

use std::cell::RefCell;
use std::cmp::Ordering;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use pz_classfile::{Jar, Value as JValue};
use pz_core::debug::DebugType;
use pz_kahlua::{LuaError, LuaResult, NativeCall, TableRef, Value};
use pz_text::translator::Language;

use crate::bind_java_util::list_value;
use crate::expose::{enum_value, expose_enum, JavaEnum};
use crate::LuaManager;

pub const CREDITS_NAME: &str = "zombie.core.CreditsName";
pub const CREDITS_ROLE: &str = "zombie.core.CreditsRole";
pub const CREDITS_ROLE_GROUP: &str = "zombie.core.CreditsRoleGroup";

/// `CreditsRole.TRANSLATION_FOLDER`, relative to the working directory
/// (the game directory).
pub const TRANSLATION_FOLDER: &str = "media/lua/shared/Translate";
/// `CreditsRole.CREDITS_TRANSLATOR_JSON`.
pub const CREDITS_TRANSLATOR_JSON: &str = "Credits_Translator.json";
/// `CreditsRole.TRANSLATOR`.
pub const TRANSLATOR: &str = "Translator";

/// `String.compareTo`: UTF-16 code unit order.
fn java_compare(a: &str, b: &str) -> Ordering {
    a.encode_utf16().cmp(b.encode_utf16())
}

/// The three enums' constants in declaration order.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CreditsData {
    /// `CreditsName`: constant, display name, explicit sort word.
    pub names: Vec<(String, String, Option<String>)>,
    /// `CreditsRole`: constant, sorted by `getSort`, names (indices).
    pub roles: Vec<(String, bool, Vec<usize>)>,
    /// `CreditsRoleGroup`: constant, logo, roles (indices).
    pub groups: Vec<(String, Option<String>, Vec<usize>)>,
}

const NAME_CLASS: &str = "zombie/core/CreditsName";
const ROLE_CLASS: &str = "zombie/core/CreditsRole";
const GROUP_CLASS: &str = "zombie/core/CreditsRoleGroup";

/// The `getstatic owner.X` elements of a varargs array argument, as
/// indices into `consts`.
fn refs(args: &[JValue], owner: &str, consts: &[String]) -> Result<Vec<usize>, String> {
    let Some(JValue::Array(items)) = args.iter().find(|a| matches!(a, JValue::Array(_))) else {
        return Err(format!("{owner}: no constant array"));
    };
    items
        .iter()
        .map(|v| match v {
            JValue::Static { class, name } if class == owner => {
                consts.iter().position(|c| c == name).ok_or_else(|| format!("unknown {owner}.{name}"))
            }
            other => Err(format!("{owner}: unexpected element {other:?}")),
        })
        .collect()
}

impl CreditsData {
    /// Reads the constants from the enums' `<clinit>` in `jar`. Enum
    /// constructor arguments start with the implicit name and ordinal.
    pub fn load(jar: &Jar) -> Result<CreditsData, String> {
        let e = |e: pz_classfile::Error| format!("{e:?}");
        let mut d = CreditsData::default();
        for (field, args) in pz_classfile::self_constructed_constants(jar, NAME_CLASS).map_err(e)? {
            let strs: Vec<&str> = args.iter().skip(2).filter_map(JValue::as_str).collect();
            let name = strs.first().ok_or_else(|| format!("CreditsName.{field}: no name"))?.to_string();
            d.names.push((field, name, strs.get(1).map(|s| s.to_string())));
        }
        let names: Vec<String> = d.names.iter().map(|n| n.0.clone()).collect();
        for (field, args) in pz_classfile::self_constructed_constants(jar, ROLE_CLASS).map_err(e)? {
            // `CreditsRole(boolean sort, CreditsName...)`; the one-argument
            // constructor sorts.
            let sorted = !matches!(args.get(2), Some(JValue::Int(0)));
            let list = refs(&args[2..], NAME_CLASS, &names)?;
            d.roles.push((field, sorted, list));
        }
        let roles: Vec<String> = d.roles.iter().map(|r| r.0.clone()).collect();
        for (field, args) in pz_classfile::self_constructed_constants(jar, GROUP_CLASS).map_err(e)? {
            let logo = args.get(2).and_then(JValue::as_str).map(str::to_string);
            let list = refs(&args[2..], ROLE_CLASS, &roles)?;
            d.groups.push((field, logo, list));
        }
        if d.names.is_empty() || d.roles.is_empty() || d.groups.is_empty() {
            return Err("credits enums are empty".into());
        }
        Ok(d)
    }

    /// `CreditsName.getSort()`: the explicit sort word, else the last word
    /// of the name (`split(" ")`, which drops trailing empty strings),
    /// upper-cased.
    pub fn name_sort(&self, i: usize) -> String {
        let (_, name, sort) = &self.names[i];
        let word = sort.as_deref().unwrap_or_else(|| name.split(' ').rev().find(|s| !s.is_empty()).unwrap_or(""));
        word.to_uppercase()
    }

    /// `CreditsRole.getNames()`: sorted (stably) by sort key unless the
    /// role keeps its order.
    pub fn role_names(&self, role: usize) -> Vec<usize> {
        let (_, sorted, names) = &self.roles[role];
        let mut v = names.clone();
        if *sorted {
            v.sort_by(|&a, &b| java_compare(&self.name_sort(a), &self.name_sort(b)));
        }
        v
    }

    /// `CreditsRole.getTitle()`: `credits_role.<constant in lower case>`.
    pub fn role_title(&self, role: usize) -> String {
        format!("credits_role.{}", self.roles[role].0.to_lowercase())
    }

    /// `CreditsRoleGroup.getTitle()`.
    pub fn group_title(&self, group: usize) -> String {
        format!("credits_role_group.{}", self.groups[group].0.to_lowercase())
    }
}

/// An error a Java method throws.
#[derive(Debug, PartialEq, Eq)]
pub struct JavaError {
    pub class: &'static str,
    pub message: String,
}

impl From<JavaError> for LuaError {
    fn from(e: JavaError) -> Self {
        LuaError::Runtime { message: e.message, class: e.class }
    }
}

/// `CreditsRole.getTranslatorCreditsList(language)` for the language
/// directory `name` under `game_dir`: the lines of the file's `Translator`
/// string (none when the file is missing or the string is empty).
pub fn translator_credits_list(game_dir: &Path, name: &str) -> Result<Vec<String>, JavaError> {
    let path = game_dir.join(TRANSLATION_FOLDER).join(name).join(CREDITS_TRANSLATOR_JSON);
    if !path.exists() {
        return Ok(Vec::new());
    }
    // `Files.readString` decodes strict UTF-8; its IOException is rethrown
    // wrapped in a RuntimeException.
    let bytes = std::fs::read(&path)
        .map_err(|e| JavaError { class: "java.lang.RuntimeException", message: format!("java.io.IOException: {e}") })?;
    let text = String::from_utf8(bytes).map_err(|_| JavaError {
        class: "java.lang.RuntimeException",
        message: "java.nio.charset.MalformedInputException: Input length = 1".into(),
    })?;
    // Core.IS_DEV is false: lenient parsing.
    let obj = pz_text::json::parse_object(&text, false)
        .map_err(|e| JavaError { class: "org.json.JSONException", message: e.0 })?;
    let translator = match obj.map.get(TRANSLATOR) {
        Some(pz_text::json::JsonValue::String(s)) => s.clone(),
        Some(_) => {
            return Err(JavaError {
                class: "org.json.JSONException",
                message: format!("JSONObject[\"{TRANSLATOR}\"] is not a string (class java.lang.Object)."),
            })
        }
        None => {
            return Err(JavaError {
                class: "org.json.JSONException",
                message: format!("JSONObject[\"{TRANSLATOR}\"] not found."),
            })
        }
    };
    if translator.is_empty() {
        return Ok(Vec::new());
    }
    let mut lines: Vec<String> = translator.split('\n').map(str::to_string).collect();
    while lines.last().is_some_and(String::is_empty) {
        lines.pop();
    }
    Ok(lines)
}

/// `CreditsRole.getTranslatorCredits()`: for every language directory (in
/// directory order) that names a known language other than the default,
/// the language's display text and its translator lines. The first error
/// stops the walk and is logged; what was read so far is kept.
pub fn translator_credits(
    game_dir: &Path,
    language: impl Fn(&str) -> Option<Rc<Language>>,
    default_language: &str,
) -> Vec<(String, Vec<String>)> {
    let mut out = Vec::new();
    let folder = game_dir.join(TRANSLATION_FOLDER);
    if !folder.exists() {
        debug_translation(&format!("Translations folder not found at {TRANSLATION_FOLDER}"));
        return out;
    }
    let Ok(rd) = std::fs::read_dir(&folder) else {
        debug_translation("Failed to load translator credits");
        return out;
    };
    let dirs: Vec<PathBuf> = rd.filter_map(|e| e.ok().map(|e| e.path())).filter(|p| p.is_dir()).collect();
    for dir in dirs {
        let Some(file_name) = dir.file_name().and_then(|n| n.to_str()) else { continue };
        let Some(lang) = language(file_name) else { continue };
        if lang.name == default_language {
            continue;
        }
        match translator_credits_list(game_dir, &lang.name) {
            Ok(list) if !list.is_empty() => out.push((lang.text.clone(), list)),
            Ok(_) => {}
            Err(_) => {
                debug_translation("Failed to load translator credits");
                break;
            }
        }
    }
    out
}

fn debug_translation(msg: &str) {
    pz_core::debug::with(|d| d.log(DebugType::Translation, msg));
}

fn strings(v: Vec<String>) -> Value {
    list_value(v.into_iter().map(Value::from).collect())
}

thread_local! {
    /// The lists `getNames()` / `getRoles()` return: one object per
    /// constant, as the Java fields are.
    static LISTS: RefCell<std::collections::HashMap<(&'static str, usize), Value>> = RefCell::default();
}

fn cached_list(class: &'static str, i: usize, make: impl FnOnce() -> Vec<Value>) -> Value {
    LISTS.with(|l| l.borrow_mut().entry((class, i)).or_insert_with(|| list_value(make())).clone())
}

fn this_ordinal(c: &NativeCall<'_>, class: &str) -> LuaResult<usize> {
    match c.arg(0) {
        Value::Object(o) => o
            .as_any()
            .downcast_ref::<JavaEnum>()
            .filter(|e| e.class == class)
            .map(|e| e.ordinal as usize)
            .ok_or_else(|| LuaError::runtime(format!("Expected {class}"))),
        _ => Err(LuaError::runtime(format!("Expected {class}"))),
    }
}

fn leak(s: &str) -> &'static str {
    Box::leak(s.to_string().into_boxed_str())
}

/// Exposes the enums with the constants read from the game's jar (nothing
/// is exposed when the jar cannot be read).
pub fn register(mgr: &mut LuaManager) {
    let fs = mgr.fs.clone();
    let jar = Jar::open(fs.borrow().base.join("projectzomboid.jar"));
    let data = match jar.map_err(|e| format!("{e:?}")).and_then(|j| CreditsData::load(&j)) {
        Ok(d) => Rc::new(d),
        Err(e) => {
            pz_core::debug::with(|d| d.log(DebugType::General, &format!("credits enums not loaded: {e}")));
            return;
        }
    };
    register_data(mgr, data);
}

/// Exposes the enums over `data`.
pub fn register_data(mgr: &mut LuaManager, data: Rc<CreditsData>) {
    let fs = mgr.fs.clone();
    let thread = &mut mgr.thread;
    let d = data.clone();
    let name_value = move |i: usize| enum_value(CREDITS_NAME, &d.names[i].0);
    let d = data.clone();
    let role_value = move |i: usize| enum_value(CREDITS_ROLE, &d.roles[i].0);
    let d = data.clone();
    let group_value = move |i: usize| enum_value(CREDITS_ROLE_GROUP, &d.groups[i].0);

    let names: Vec<&'static str> = data.names.iter().map(|n| leak(&n.0)).collect();
    let b = expose_enum(thread, CREDITS_NAME, &names);
    let d = data.clone();
    b.method("getName", move |c| {
        let i = this_ordinal(c, CREDITS_NAME)?;
        Ok(c.push(d.names[i].1.clone()))
    });
    let d = data.clone();
    b.method("getSort", move |c| {
        let i = this_ordinal(c, CREDITS_NAME)?;
        Ok(c.push(d.name_sort(i)))
    });

    let roles: Vec<&'static str> = data.roles.iter().map(|r| leak(&r.0)).collect();
    let b = expose_enum(thread, CREDITS_ROLE, &roles);
    let d = data.clone();
    b.method("getTitle", move |c| {
        let i = this_ordinal(c, CREDITS_ROLE)?;
        Ok(c.push(d.role_title(i)))
    });
    let d = data.clone();
    b.method("getNames", move |c| {
        let i = this_ordinal(c, CREDITS_ROLE)?;
        let v = cached_list(CREDITS_ROLE, i, || d.role_names(i).into_iter().map(&name_value).collect());
        Ok(c.push(v))
    });
    b.field("TRANSLATOR", TRANSLATOR);
    let f = fs.clone();
    b.static_fn("getTranslatorCredits", move |c| {
        let base = f.borrow().base.clone();
        let default = crate::bind_text::with_translator(|t| t.languages.default_language().name.clone())
            .unwrap_or_else(|| "EN".into());
        let lookup = |n: &str| crate::bind_text::with_translator(|t| t.languages.by_name(n)).flatten();
        let map = TableRef::new();
        for (text, list) in translator_credits(&base, lookup, &default) {
            map.rawset(Value::from(text), strings(list));
        }
        Ok(c.push(map))
    });
    let f = fs;
    b.static_fn("getTranslatorCreditsList", move |c| {
        let Some(lang) = crate::bind_text::language_of(&c.arg(0)) else {
            return Err(LuaError::Runtime {
                message: "language is null".into(),
                class: "java.lang.NullPointerException",
            });
        };
        let base = f.borrow().base.clone();
        let list = translator_credits_list(&base, &lang.name)?;
        Ok(c.push(strings(list)))
    });

    let groups: Vec<&'static str> = data.groups.iter().map(|g| leak(&g.0)).collect();
    let b = expose_enum(thread, CREDITS_ROLE_GROUP, &groups);
    let n = data.groups.len();
    b.static_fn("getAll", move |c| {
        let all = (0..n).map(&group_value).collect();
        Ok(c.push(list_value(all)))
    });
    let d = data.clone();
    b.method("getTitle", move |c| {
        let i = this_ordinal(c, CREDITS_ROLE_GROUP)?;
        Ok(c.push(d.group_title(i)))
    });
    let d = data.clone();
    b.method("getLogo", move |c| {
        let i = this_ordinal(c, CREDITS_ROLE_GROUP)?;
        Ok(c.push(d.groups[i].1.clone().map_or(Value::Nil, Value::from)))
    });
    let d = data;
    b.method("getRoles", move |c| {
        let i = this_ordinal(c, CREDITS_ROLE_GROUP)?;
        let v = cached_list(CREDITS_ROLE_GROUP, i, || d.groups[i].2.iter().map(|&r| role_value(r)).collect());
        Ok(c.push(v))
    });
}

#[cfg(test)]
mod tests;
