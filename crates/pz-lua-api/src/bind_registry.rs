//! `zombie.scripting.objects.ResourceLocation` and the registry key classes
//! `CharacterTrait`, `CharacterProfession` and `zombie.core.GameMode` exposed
//! to Lua, over the shared
//! `ScriptManager`'s registries.
//!
//! # Providing the ScriptManager
//!
//! Like [`crate::bind_core::shared_core`], [`shared_script_manager`] stands
//! in for `ScriptManager.instance`: call [`install_script_manager`] with the
//! engine's instance (built-in registries and scripts loaded) *before*
//! `LuaManager::init`, since the key classes' constants are read from it
//! when they are exposed.
//!
//! Key objects are canonical (one Lua object per registry key) so Lua `==`
//! and table keys behave like the original's singletons.

use crate::expose::{expose_class, str_arg, this};
use pz_kahlua::value::LuaObject;
use pz_kahlua::{LuaError, LuaResult, NativeCall, Thread, Value};
use pz_scripts::objects::items::GameMode;
use pz_scripts::registry::{RegistryKey, ResourceLocation, DEFAULT_NAMESPACE};
use pz_scripts::{ScriptError, ScriptManager};
use std::any::Any;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

pub const RESOURCE_LOCATION_CLASS: &str = "zombie.scripting.objects.ResourceLocation";
pub const CHARACTER_TRAIT_CLASS: &str = "zombie.scripting.objects.CharacterTrait";
pub const CHARACTER_PROFESSION_CLASS: &str = "zombie.scripting.objects.CharacterProfession";
pub const GAME_MODE_CLASS: &str = "zombie.core.GameMode";

/// Key classes exposed here: `(Java class, registry)`.
pub const KEY_CLASSES: &[(&str, &str)] = &[
    (CHARACTER_PROFESSION_CLASS, "character_profession"),
    (CHARACTER_TRAIT_CLASS, "character_trait"),
    (GAME_MODE_CLASS, "game_mode"),
];

thread_local! {
    static SHARED: RefCell<Option<Rc<ScriptManager>>> = const { RefCell::new(None) };
    static KEYS: RefCell<HashMap<*const RegistryKey, Value>> = RefCell::new(HashMap::new());
}

/// Make `sm` the instance `register_all` binds.
pub fn install_script_manager(sm: Rc<ScriptManager>) {
    SHARED.with(|s| *s.borrow_mut() = Some(sm));
}

/// The shared ScriptManager (`ScriptManager.instance`), empty if none was
/// installed.
pub fn shared_script_manager() -> Rc<ScriptManager> {
    SHARED.with(|s| s.borrow_mut().get_or_insert_with(|| Rc::new(ScriptManager::new())).clone())
}

/// A Java exception from the ported code (`"java.lang.X: message"`) as the
/// Lua error Kahlua raises for it.
pub fn java_error(e: ScriptError) -> LuaError {
    const CLASSES: &[&str] = &[
        "java.lang.NullPointerException",
        "java.lang.IllegalArgumentException",
        "java.lang.IndexOutOfBoundsException",
        "java.lang.RuntimeException",
    ];
    let (class, message) = match e.0.split_once(": ") {
        Some((c, m)) => (c, m.to_string()),
        None => (e.0.as_str(), String::new()),
    };
    match CLASSES.iter().find(|c| **c == class) {
        Some(c) => LuaError::Runtime { message, class: c },
        None => LuaError::runtime(e.0),
    }
}

/// A `CharacterTrait`/`CharacterProfession` instance.
pub struct KeyObject {
    pub class: &'static str,
    pub key: Rc<RegistryKey>,
}

impl LuaObject for KeyObject {
    fn class_name(&self) -> &str {
        self.class
    }
    /// `toString`: the location, except `GameMode`'s registered id.
    fn to_lua_string(&self) -> String {
        if self.class == GAME_MODE_CLASS {
            self.key.name.clone()
        } else {
            self.key.location.id()
        }
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// The canonical Lua object for `key` (nil for `None`).
pub fn key_value(class: &'static str, key: Option<&Rc<RegistryKey>>) -> Value {
    let Some(key) = key else { return Value::Nil };
    KEYS.with(|k| {
        k.borrow_mut()
            .entry(Rc::as_ptr(key))
            .or_insert_with(|| Value::Object(Rc::new(KeyObject { class, key: key.clone() })))
            .clone()
    })
}

/// A key argument: nil is `null`; other values must be key objects.
pub fn key_arg(c: &NativeCall<'_>, i: usize) -> LuaResult<Option<Rc<RegistryKey>>> {
    match c.arg(i) {
        Value::Nil => Ok(None),
        Value::Object(o) => match o.as_any().downcast_ref::<KeyObject>() {
            Some(k) => Ok(Some(k.key.clone())),
            None => Err(LuaError::runtime(format!("Expected a registry key but got {}", o.class_name()))),
        },
        other => Err(LuaError::runtime(format!(
            "Expected a registry key but got {}",
            pz_kahlua::stdlib::java_class_name(&other)
        ))),
    }
}

/// `zombie.scripting.objects.ResourceLocation`.
pub struct ResourceLocationObject(pub ResourceLocation);

impl LuaObject for ResourceLocationObject {
    fn class_name(&self) -> &str {
        RESOURCE_LOCATION_CLASS
    }
    fn to_lua_string(&self) -> String {
        self.0.id()
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// A `ResourceLocation` argument (nil is `null`).
pub fn location_arg(c: &NativeCall<'_>, i: usize) -> LuaResult<Option<ResourceLocation>> {
    match c.arg(i) {
        Value::Nil => Ok(None),
        Value::Object(o) => match o.as_any().downcast_ref::<ResourceLocationObject>() {
            Some(l) => Ok(Some(l.0.clone())),
            None => Err(LuaError::runtime(format!("Expected ResourceLocation but got {}", o.class_name()))),
        },
        other => Err(LuaError::runtime(format!(
            "Expected ResourceLocation but got {}",
            pz_kahlua::stdlib::java_class_name(&other)
        ))),
    }
}

fn str_or_empty(c: &NativeCall<'_>, i: usize) -> String {
    str_arg(c, i).map(|s| s.to_string()).unwrap_or_default()
}

fn this_location(c: &NativeCall<'_>) -> LuaResult<ResourceLocation> {
    let o = this::<ResourceLocationObject>(c)?;
    Ok(o.as_any().downcast_ref::<ResourceLocationObject>().unwrap().0.clone())
}

fn this_key(c: &NativeCall<'_>) -> LuaResult<Rc<RegistryKey>> {
    let o = this::<KeyObject>(c)?;
    Ok(o.as_any().downcast_ref::<KeyObject>().unwrap().key.clone())
}

/// Register `ResourceLocation`, `CharacterTrait` and `CharacterProfession`.
pub fn register(thread: &mut Thread, sm: Rc<ScriptManager>) {
    let b = expose_class(thread, RESOURCE_LOCATION_CLASS, None);
    b.field("DEFAULT_NAMESPACE", DEFAULT_NAMESPACE);
    b.static_fn("of", |c| {
        // `null` and "" are both rejected by `ResourceLocation.of`.
        let l = ResourceLocation::of(&str_or_empty(c, 0)).map_err(java_error)?;
        Ok(c.push(Value::Object(Rc::new(ResourceLocationObject(l)))))
    });
    b.ctor(|c| {
        let l = ResourceLocation::new(&str_or_empty(c, 0), &str_or_empty(c, 1)).map_err(java_error)?;
        Ok(c.push(Value::Object(Rc::new(ResourceLocationObject(l)))))
    });
    b.method("getNamespace", |c| {
        let l = this_location(c)?;
        Ok(c.push(l.namespace()))
    });
    b.method("getPath", |c| {
        let l = this_location(c)?;
        Ok(c.push(l.path()))
    });
    b.method("toString", |c| {
        let l = this_location(c)?;
        Ok(c.push(l.id()))
    });
    b.method("hashCode", |c| {
        let l = this_location(c)?;
        Ok(c.push(l.java_hash()))
    });
    b.method("equals", |c| {
        let l = this_location(c)?;
        let eq = match c.arg(1) {
            Value::Object(o) => o.as_any().downcast_ref::<ResourceLocationObject>().is_some_and(|o| o.0 == l),
            _ => false,
        };
        Ok(c.push(eq))
    });

    for &(class, registry) in KEY_CLASSES {
        let b = expose_class(thread, class, None);
        let constants: Vec<(String, String)> = sm
            .statics
            .items
            .borrow()
            .constants
            .constants(registry)
            .into_iter()
            .map(|(f, id)| (f.to_string(), id.to_string()))
            .collect();
        for (field, id) in constants {
            let key = ResourceLocation::of(&id)
                .ok()
                .and_then(|l| sm.registries.get(registry).expect("known registry").borrow().get(&l));
            if let Some(key) = key {
                b.field(&field, key_value(class, Some(&key)));
            }
        }
        let s = sm.clone();
        b.static_fn("get", move |c| {
            let key = match location_arg(c, 0)? {
                Some(l) => s.registries.get(registry).expect("known registry").borrow().get(&l),
                None => None,
            };
            Ok(c.push(key_value(class, key.as_ref())))
        });
        let s = sm.clone();
        b.static_fn("register", move |c| {
            let id = str_or_empty(c, 0);
            let key = s.registries.get(registry).expect("known registry").borrow_mut().register_id(&id, false);
            Ok(c.push(key_value(class, Some(&key.map_err(java_error)?))))
        });
        if class == GAME_MODE_CLASS {
            type Text = fn(&RegistryKey) -> String;
            let texts: [(&str, Text); 4] = [
                ("getTitle", GameMode::title),
                ("getDescription", GameMode::description),
                ("getThumbnail", GameMode::thumbnail),
                ("getVideo", GameMode::video),
            ];
            for (name, f) in texts {
                b.method(name, move |c| {
                    let k = this_key(c)?;
                    Ok(c.push(f(&k)))
                });
            }
        } else {
            b.method("getName", |c| {
                let k = this_key(c)?;
                Ok(c.push(k.path()))
            });
        }
        b.method("toString", |c| {
            let o = this::<KeyObject>(c)?;
            Ok(c.push(o.to_lua_string()))
        });
        b.method("equals", |c| {
            let r = c.arg(0).raw_equals(&c.arg(1));
            Ok(c.push(r))
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pz_kahlua::TableRef;

    pub(crate) fn sm_with_keys() -> Rc<ScriptManager> {
        let sm = Rc::new(ScriptManager::new());
        for (reg, ids) in [("character_trait", ["t:brave", "t:timid"]), ("character_profession", ["t:baker", "t:smith"])] {
            for id in ids {
                sm.registries.get(reg).unwrap().borrow_mut().register_id(id, false).unwrap();
            }
        }
        sm.registries.get("game_mode").unwrap().borrow_mut().register_id("t:Quick", false).unwrap();
        let mut st = sm.statics.items.borrow_mut();
        st.constants.insert("game_mode", "QUICK", "t:Quick");
        st.constants.insert("character_trait", "BRAVE", "t:brave");
        st.constants.insert("character_profession", "BAKER", "t:baker");
        drop(st);
        sm
    }

    fn run(t: &mut Thread, src: &str) -> Result<(), String> {
        let p = pz_kahlua::compiler::compile(src, Some("test"), Some("test"), None).unwrap();
        let f = Value::Closure(pz_kahlua::vm::LuaClosure::new(Rc::new(p), t.globals().clone()));
        t.pcall(&f, &[]).map(|_| ()).map_err(|(e, _)| pz_kahlua::exec::java_value_of(&e))
    }

    #[test]
    fn keys_and_locations() {
        let mut t = Thread::new(TableRef::new());
        pz_kahlua::stdlib::install(&mut t, None).unwrap();
        run(&mut t, "function assert(v, msg) if not v then error(msg or 'assertion failed!', 2) end return v end").unwrap();
        register(&mut t, sm_with_keys());
        run(
            &mut t,
            r#"
            local l = ResourceLocation.of("T:Brave")
            assert(l:getNamespace() == "t" and l:getPath() == "brave" and l:toString() == "t:brave")
            assert(ResourceLocation.of("x"):toString() == "base:x")
            assert(ResourceLocation.DEFAULT_NAMESPACE == "base")
            local b = CharacterTrait.get(l)
            assert(b == CharacterTrait.BRAVE and b:getName() == "brave" and tostring(b) == "t:brave")
            assert(CharacterTrait.get(ResourceLocation.of("t:nope")) == nil)
            assert(CharacterProfession.BAKER:toString() == "t:baker")
            assert(CharacterTrait.TIMID == nil)
            local t = {}
            t[CharacterTrait.BRAVE] = 1
            assert(t[CharacterTrait.get(ResourceLocation.new("t", "brave"))] == 1)
            local m = CharacterTrait.register("mod:new")
            assert(CharacterTrait.get(ResourceLocation.of("mod:new")) == m)
            local q = GameMode.QUICK
            assert(GameMode.get(ResourceLocation.of("t:quick")) == q and tostring(q) == "t:Quick")
            assert(q:getTitle() == "UI_NewGame_t:Quick" and q:getDescription() == "UI_NewGame_t:Quick_desc")
            assert(q:getThumbnail() == "media/ui/playstyleIcons/t:Quick.png" and q:getVideo() == "t:Quick.bik")
            assert(q.getName == nil)
            "#,
        )
        .unwrap();
        let e = run(&mut t, "ResourceLocation.of('')").unwrap_err();
        assert!(e.contains("Identifier cannot be null or empty"), "{e}");
        let e = run(&mut t, "CharacterTrait.register('plain')").unwrap_err();
        assert!(e.contains("Default namespace"), "{e}");
    }
}
