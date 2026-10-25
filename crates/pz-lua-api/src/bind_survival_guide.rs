//! `zombie.core.SurvivalGuideEntry`: the survival guide's entries (the
//! built-ins read from the game jar's static initializer, plus any a mod
//! registers), backed by the `survival_guide_entry` registry.

use crate::bind_registry::{java_error, location_arg, shared_script_manager};
use crate::expose::{bool_arg, expose_class, str_arg, with_this};
use pz_kahlua::value::LuaObject;
use pz_kahlua::{LuaError, NativeCall, Thread, Value};
use pz_scripts::registry::{create_location, ResourceLocation};
use pz_text::JavaArg;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

pub const SURVIVAL_GUIDE_ENTRY: &str = "zombie.core.SurvivalGuideEntry";
const REGISTRY: &str = "survival_guide_entry";
const CLASS_FILE: &str = "zombie/core/SurvivalGuideEntry";

/// One entry (`new SurvivalGuideEntry(id, subCategory, keys, joypadKeys)`).
#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    pub id: String,
    pub sub_category: Option<String>,
    pub keys: Vec<String>,
    pub joypad_keys: Vec<String>,
}

impl Entry {
    pub fn title(&self) -> String {
        format!("SurvivalGuide_{}_title", self.id)
    }
    pub fn description(&self) -> String {
        format!("SurvivalGuide_{}_description", self.id)
    }
    fn has_sub(&self) -> bool {
        self.sub_category.as_deref().is_some_and(|s| !s.is_empty())
    }
    /// `media/ui/playstyleIcons/<id>.png` for a sub-entry, else "".
    pub fn thumbnail(&self) -> String {
        if self.has_sub() {
            format!("media/ui/playstyleIcons/{}.png", self.id)
        } else {
            String::new()
        }
    }
    /// `getVideo()`: the Bink path for a sub-entry (`ISRichTextPanel`
    /// opens it with the global `getVideo`), else "".
    pub fn video(&self) -> String {
        if self.has_sub() {
            format!("survival_guide/sg_{}.bik", self.id)
        } else {
            String::new()
        }
    }
    /// `getCategoryImage()`: only categories have one.
    pub fn category_image(&self) -> Option<String> {
        (!self.has_sub()).then(|| format!("SurvivalGuide_{}_category_image", self.id))
    }
}

/// An entry as Lua holds it.
pub struct EntryObject(pub Rc<Entry>);

impl LuaObject for EntryObject {
    fn class_name(&self) -> &str {
        SURVIVAL_GUIDE_ENTRY
    }
    fn to_lua_string(&self) -> String {
        self.0.id.clone()
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

thread_local! {
    /// Every registered entry's Lua object, by location id.
    static ENTRIES: RefCell<HashMap<String, Value>> = RefCell::default();
}

/// The `registerBase(...)` calls of the class's static initializer:
/// (field, entry), in declaration order.
pub fn builtin_entries(jar: &pz_classfile::Jar) -> pz_classfile::Result<Vec<(String, Entry)>> {
    use pz_classfile::Value as V;
    let cf = pz_classfile::load_class(jar, CLASS_FILE)?;
    let strs = |v: &V| -> Vec<String> {
        match v {
            V::Call {
                class, name, args, ..
            } if class == "java/util/List" && name == "of" => {
                // List.of(E...) with an array, or up to ten elements.
                let items: Vec<&V> = match args.as_slice() {
                    [V::Array(a)] => a.iter().collect(),
                    a => a.iter().collect(),
                };
                items
                    .into_iter()
                    .filter_map(|i| i.as_str().map(str::to_string))
                    .collect()
            }
            _ => Vec::new(),
        }
    };
    Ok(pz_classfile::static_init::static_inits(&cf)?
        .into_iter()
        .filter(|i| i.class == cf.this_class)
        .filter_map(|i| match i.value {
            V::Call { name, args, .. } if name == "registerBase" => {
                let id = args.first()?.as_str()?.to_string();
                let sub = args.get(1).and_then(|v| v.as_str().map(str::to_string));
                let keys = args.get(2).map(strs).unwrap_or_default();
                let joypad = args.get(3).map(strs).unwrap_or_default();
                Some((
                    i.field,
                    Entry {
                        id,
                        sub_category: sub,
                        keys,
                        joypad_keys: joypad,
                    },
                ))
            }
            _ => None,
        })
        .collect())
}

/// `register(allowDefaultNamespace, id, subCategory, keys, joypadKeys)`.
pub fn register_entry(allow_default: bool, e: Entry) -> Result<Value, LuaError> {
    let loc = create_location(&e.id, allow_default).map_err(java_error)?;
    let sm = shared_script_manager();
    let reg = sm
        .registries
        .get(REGISTRY)
        .expect("survival_guide_entry registry");
    reg.borrow_mut()
        .register(loc.clone(), &e.id, None)
        .map_err(java_error)?;
    let v = Value::Object(Rc::new(EntryObject(Rc::new(e))));
    ENTRIES.with(|m| m.borrow_mut().insert(loc.id(), v.clone()));
    Ok(v)
}

/// `SurvivalGuideEntry.get(id)`.
pub fn get(loc: &ResourceLocation) -> Value {
    let sm = shared_script_manager();
    let reg = sm
        .registries
        .get(REGISTRY)
        .expect("survival_guide_entry registry");
    if !reg.borrow().contains(loc) {
        return Value::Nil;
    }
    ENTRIES
        .with(|m| m.borrow().get(&loc.id()).cloned())
        .unwrap_or_default()
}

/// `SurvivalGuideEntry.getAll()`: the registry's entries in order (mods'
/// entries leave with `RegistryReset`).
pub fn all() -> Vec<Value> {
    let sm = shared_script_manager();
    let reg = sm
        .registries
        .get(REGISTRY)
        .expect("survival_guide_entry registry");
    let ids: Vec<String> = reg
        .borrow()
        .values()
        .iter()
        .map(|k| k.location.id())
        .collect();
    ENTRIES.with(|m| {
        let m = m.borrow();
        ids.iter().filter_map(|i| m.get(i).cloned()).collect()
    })
}

fn call_global(t: &mut Thread, name: &str, args: &[Value]) -> Result<Value, LuaError> {
    let f = t.globals().get_str(name);
    t.call1(&f, args)
}

fn tr(key: &str) -> String {
    crate::bind_text::with_translator(|t| t.get_text(key, &[])).unwrap_or_else(|| key.to_string())
}

/// `getKeyName(key)`: a translated `IGUI_` key, a keybinding's key name
/// (and its alternative), or the joypad/image markup as is.
fn key_name(t: &mut Thread, key: &str) -> Result<String, LuaError> {
    if key.contains("IGUI_") {
        return Ok(tr(key));
    }
    if !key.contains("<JOYPAD") && !key.contains("<IMAGE") {
        let (k, alt) = {
            let core = crate::bind_core::shared_core();
            let c = core.borrow();
            (c.get_key(key), c.get_alt_key(key))
        };
        // Input.getKeyName, as the Lua global does it.
        let name = |t: &mut Thread, code: i32| -> Result<String, LuaError> {
            Ok(pz_kahlua::exec::java_value_of(&call_global(
                t,
                "getKeyName",
                &[Value::from(code)],
            )?))
        };
        let mut txt = name(t, k)?;
        if alt > 0 {
            txt = format!("{txt} {} {}", tr("ContextMenu_or"), name(t, alt)?);
        }
        return Ok(txt);
    }
    Ok(key.to_string())
}

/// `getText(hasJoystick)`: the description with the key names filled in
/// (the `_joypad` variant may be missing), or nil for `<IGNORE>`.
pub fn text(t: &mut Thread, e: &Entry, has_joystick: bool) -> Result<Option<String>, LuaError> {
    let keys = if has_joystick {
        &e.joypad_keys
    } else {
        &e.keys
    };
    let mut args = Vec::with_capacity(keys.len());
    for k in keys {
        args.push(JavaArg::Str(key_name(t, k)?));
    }
    let desc = e.description();
    let s = if has_joystick {
        crate::bind_text::with_translator(|t| t.get_text_or_null(&format!("{desc}_joypad"), &args))
            .flatten()
    } else {
        Some(crate::bind_text::with_translator(|t| t.get_text(&desc, &args)).unwrap_or(desc))
    };
    Ok(s.filter(|s| s != "<IGNORE>"))
}

fn this(c: &NativeCall<'_>) -> Result<Rc<Entry>, LuaError> {
    with_this::<EntryObject, _>(c, |o| o.0.clone())
}

fn str_list(v: &[String]) -> Value {
    crate::bind_java_util::list_value(v.iter().map(|s| Value::str(s)).collect())
}

/// A `List<String>` argument: a Java list or a Lua array.
fn list_arg(c: &NativeCall<'_>, i: usize) -> Vec<String> {
    match c.arg(i) {
        Value::Object(o) => o
            .as_any()
            .downcast_ref::<crate::bind_java_util::JavaList>()
            .map(|l| {
                l.snapshot()
                    .iter()
                    .map(pz_kahlua::exec::java_value_of)
                    .collect()
            })
            .unwrap_or_default(),
        Value::Table(t) => (1..=t.len())
            .map(|k| pz_kahlua::exec::java_value_of(&t.get_int(k)))
            .collect(),
        _ => Vec::new(),
    }
}

/// Register the class, its built-in entries (from `jar`) and their
/// constants.
pub fn register(thread: &mut Thread, jar: Option<&pz_classfile::Jar>) {
    let b = expose_class(thread, SURVIVAL_GUIDE_ENTRY, None);
    let builtins = match jar.map(builtin_entries) {
        Some(Ok(v)) => v,
        Some(Err(e)) => {
            eprintln!("{SURVIVAL_GUIDE_ENTRY}: {e}");
            Vec::new()
        }
        None => Vec::new(),
    };
    for (field, e) in builtins {
        // The base registrations are already there after a reload.
        let existing = ResourceLocation::of(&e.id)
            .ok()
            .map(|l| get(&l))
            .filter(|v| !v.is_nil());
        let v = match existing {
            Some(v) => v,
            None => match register_entry(true, e) {
                Ok(v) => v,
                Err(err) => {
                    eprintln!("{SURVIVAL_GUIDE_ENTRY}.{field}: {err:?}");
                    continue;
                }
            },
        };
        b.field(&field, v);
    }
    b.static_fn("get", |c| {
        let l = location_arg(c, 0)?;
        Ok(c.push(l.map_or(Value::Nil, |l| get(&l))))
    });
    b.static_fn("getAll", |c| {
        Ok(c.push(Value::Object(Rc::new(
            crate::bind_java_util::JavaList::live(all, None),
        ))))
    });
    b.static_fn("register", |c| {
        let id = str_arg(c, 0)
            .map(|s| s.to_string())
            .ok_or(LuaError::Runtime {
                message: String::new(),
                class: "java.lang.NullPointerException",
            })?;
        let sub = str_arg(c, 1).map(|s| s.to_string());
        let e = Entry {
            id,
            sub_category: sub,
            keys: list_arg(c, 2),
            joypad_keys: list_arg(c, 3),
        };
        let v = register_entry(false, e)?;
        Ok(c.push(v))
    });
    b.method("getTitle", |c| {
        let e = this(c)?;
        Ok(c.push(Value::str(&e.title())))
    });
    b.method("getDescription", |c| {
        let e = this(c)?;
        Ok(c.push(Value::str(&e.description())))
    });
    b.method("getThumbnail", |c| {
        let e = this(c)?;
        Ok(c.push(Value::str(&e.thumbnail())))
    });
    b.method("getVideo", |c| {
        let e = this(c)?;
        Ok(c.push(Value::str(&e.video())))
    });
    b.method("getSubCategory", |c| {
        let e = this(c)?;
        Ok(c.push(e.sub_category.as_deref().map_or(Value::Nil, Value::str)))
    });
    b.method("getCategoryImage", |c| {
        let e = this(c)?;
        Ok(c.push(e.category_image().as_deref().map_or(Value::Nil, Value::str)))
    });
    b.method("getKeys", |c| {
        let e = this(c)?;
        Ok(c.push(str_list(&e.keys)))
    });
    b.method("getJoypadKeys", |c| {
        let e = this(c)?;
        Ok(c.push(str_list(&e.joypad_keys)))
    });
    b.method("getText", |c| {
        let e = this(c)?;
        let joy = bool_arg(c, 1);
        let s = text(c.thread, &e, joy)?;
        Ok(c.push(s.as_deref().map_or(Value::Nil, Value::str)))
    });
    b.method("toString", |c| {
        let e = this(c)?;
        Ok(c.push(Value::str(&e.id)))
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use pz_scripts::ScriptManager;

    fn setup() -> Thread {
        crate::bind_registry::install_script_manager(Rc::new(ScriptManager::new()));
        ENTRIES.with(|m| m.borrow_mut().clear());
        let mut t = Thread::new(pz_kahlua::TableRef::new());
        pz_kahlua::stdlib::install(&mut t, None).unwrap();
        crate::bind_java_util::register(&mut t);
        crate::bind_registry::register(&mut t, shared_script_manager());
        t
    }

    fn run(t: &mut Thread, src: &str) {
        let p = pz_kahlua::compiler::compile(src, Some("test"), Some("test"), None).unwrap();
        let f = Value::Closure(pz_kahlua::vm::LuaClosure::new(
            Rc::new(p),
            t.globals().clone(),
        ));
        if let Err((e, tr)) = t.pcall(&f, &[]) {
            panic!("lua error: {e:?}\n{tr}");
        }
    }

    fn list(v: &[String]) -> String {
        format!("[{}]", v.join(", "))
    }

    fn line(e: &Entry) -> String {
        format!(
            "entry {} | {} | {} | {} | {} | {} | {} | {} | {} | {}",
            e.id,
            ResourceLocation::of(&e.id).unwrap(),
            e.title(),
            e.description(),
            e.thumbnail(),
            e.video(),
            e.sub_category.as_deref().unwrap_or("null"),
            e.category_image().as_deref().unwrap_or("null"),
            list(&e.keys),
            list(&e.joypad_keys)
        )
    }

    fn entry(v: &Value) -> Rc<Entry> {
        match v {
            Value::Object(o) => o.as_any().downcast_ref::<EntryObject>().unwrap().0.clone(),
            _ => panic!("not an entry"),
        }
    }

    /// Parity with the original class (oracle `guide.txt`, `PZ_JAR` the
    /// game jar): every built-in entry read from the jar, in registry
    /// order, with its field, derived strings and key lists; `get`; a
    /// mod's `register` and its errors.
    #[test]
    fn matches_guide_oracle() {
        let (Ok(dir), Ok(jar)) = (std::env::var("PZ_HUD_ORACLE_DIR"), std::env::var("PZ_JAR"))
        else {
            eprintln!("PZ_HUD_ORACLE_DIR / PZ_JAR not set; skipping");
            return;
        };
        let text = std::fs::read_to_string(std::path::Path::new(&dir).join("guide.txt")).unwrap();
        let jar = pz_classfile::Jar::open(jar).unwrap();
        let mut t = setup();
        register(&mut t, Some(&jar));
        let mut got = Vec::new();
        for (f, e) in builtin_entries(&jar).unwrap() {
            got.push(format!("field {f} {}", e.id));
        }
        for v in all() {
            got.push(line(&entry(&v)));
        }
        let name = |v: Value| {
            if v.is_nil() {
                "null".to_string()
            } else {
                entry(&v).id.clone()
            }
        };
        got.push(format!(
            "get {} {}",
            name(get(&ResourceLocation::of("foraging").unwrap())),
            name(get(&ResourceLocation::of("nope").unwrap()))
        ));
        let err = |e: LuaError| match e {
            LuaError::Runtime { message, class } => format!("{class}: {message}"),
            e => format!("{e:?}"),
        };
        let bad = Entry {
            id: "bad".into(),
            sub_category: None,
            keys: vec![],
            joypad_keys: vec![],
        };
        got.push(format!(
            "register base {}",
            err(register_entry(false, bad).unwrap_err())
        ));
        let m = register_entry(
            false,
            Entry {
                id: "mymod:thing".into(),
                sub_category: Some("combat".into()),
                keys: vec!["Aim".into()],
                joypad_keys: vec![],
            },
        )
        .unwrap();
        let m = entry(&m);
        got.push(format!(
            "mod {} | {} | {} | {} | {}",
            m.id,
            m.title(),
            m.video(),
            m.thumbnail(),
            all().len()
        ));
        let dup = Entry {
            id: "mymod:thing".into(),
            sub_category: None,
            keys: vec![],
            joypad_keys: vec![],
        };
        got.push(format!(
            "dup {}",
            err(register_entry(false, dup).unwrap_err())
        ));
        let want: Vec<&str> = text.lines().collect();
        assert_eq!(got, want);
    }

    /// The Lua side `SurvivalGuide.lua` uses: `getAll()` as a list of the
    /// same objects, the constants, `get`, the getters, `register` from a
    /// table, `getText` filling keys (IGUI keys translated, joypad markup
    /// kept, keybindings by name) and the registry reset dropping mods'
    /// entries.
    #[test]
    fn lua_api() {
        let mut t = setup();
        // No jar: register two built-ins by hand, as the jar would.
        register(&mut t, None);
        register_entry(
            true,
            Entry {
                id: "combat".into(),
                sub_category: None,
                keys: vec![],
                joypad_keys: vec![],
            },
        )
        .unwrap();
        register_entry(
            true,
            Entry {
                id: "shove".into(),
                sub_category: Some("combat".into()),
                keys: vec!["IGUI_mouse_btn_1".into(), "<IMAGE:x.png,28,28>".into()],
                joypad_keys: vec!["<JOYPAD:LTrigger,28,28>".into()],
            },
        )
        .unwrap();
        run(
            &mut t,
            "local all = SurvivalGuideEntry.getAll()
             n = all:size(); first = all:get(0); same = SurvivalGuideEntry.getAll():get(0) == first
             cat = first:getSubCategory(); img = first:getCategoryImage(); fv = first:getVideo()
             sub = all:get(1); s = sub:getSubCategory(); v = sub:getVideo(); th = sub:getThumbnail()
             id = tostring(sub); k = sub:getKeys():size(); jk = sub:getJoypadKeys():get(0)
             got = SurvivalGuideEntry.get(ResourceLocation.of('shove')) == sub
             txt = sub:getText(false); jtxt = sub:getText(true)
             m = SurvivalGuideEntry.register('mod:x', 'combat', {'Aim'}, {})
             n2 = SurvivalGuideEntry.getAll():size()
             ok, err = pcall(function() SurvivalGuideEntry.register('plain', nil, {}, {}) end)",
        );
        let g = |t: &mut Thread, k: &str| t.globals().get_str(k);
        assert_eq!(g(&mut t, "n").as_num(), Some(2.0));
        assert!(g(&mut t, "same").truthy());
        assert!(g(&mut t, "cat").is_nil());
        assert_eq!(
            g(&mut t, "img").as_str(),
            Some("SurvivalGuide_combat_category_image")
        );
        assert_eq!(g(&mut t, "fv").as_str(), Some(""));
        assert_eq!(g(&mut t, "s").as_str(), Some("combat"));
        assert_eq!(g(&mut t, "v").as_str(), Some("survival_guide/sg_shove.bik"));
        assert_eq!(
            g(&mut t, "th").as_str(),
            Some("media/ui/playstyleIcons/shove.png")
        );
        assert_eq!(g(&mut t, "id").as_str(), Some("shove"));
        assert_eq!(g(&mut t, "k").as_num(), Some(2.0));
        assert_eq!(g(&mut t, "jk").as_str(), Some("<JOYPAD:LTrigger,28,28>"));
        assert!(g(&mut t, "got").truthy());
        // No translator: the description key itself; the joypad variant
        // is missing (null).
        assert_eq!(
            g(&mut t, "txt").as_str(),
            Some("SurvivalGuide_shove_description")
        );
        assert!(g(&mut t, "jtxt").is_nil());
        assert_eq!(g(&mut t, "n2").as_num(), Some(3.0));
        assert!(!g(&mut t, "ok").truthy());
        // Key names: IGUI keys translate, markup stays, keybindings go
        // through getKeyName.
        run(&mut t, "function getKeyName(code) return 'K' .. code end");
        assert_eq!(
            key_name(&mut t, "IGUI_mouse_btn_1").unwrap(),
            "IGUI_mouse_btn_1"
        );
        assert_eq!(
            key_name(&mut t, "<JOYPAD:AButton,28,28>").unwrap(),
            "<JOYPAD:AButton,28,28>"
        );
        let code = crate::bind_core::shared_core().borrow().get_key("Aim");
        assert_eq!(key_name(&mut t, "Aim").unwrap(), format!("K{code}"));
        // RegistryReset drops the mod's entry.
        shared_script_manager().registries.reset_all();
        assert_eq!(all().len(), 2);
    }
}
