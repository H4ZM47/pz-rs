//! `zombie.scripting.objects.ResourceLocation` and the registry key classes
//! (`CharacterTrait`, `CharacterProfession`, `zombie.core.GameMode`,
//! `ItemType`, `ItemTag`, `ItemBodyLocation`, `WeaponCategory`, `AmmoType`)
//! exposed to Lua, over the shared `ScriptManager`'s registries.
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
//!
//! `zombie.scripting.objects.ItemKey` and its nested holder classes
//! (`ItemKey.Container.KEY_RING`, ...) are exposed with the constants the
//! jar declares ([`pz_scripts::registry::builtin::BuiltinTables::item_keys`]).
//! TODO(ItemKey.BY_NAME): `getByName`/`getByItemKeyValue` read a map the
//! holders fill as the JVM initializes them lazily; not exposed yet.

use crate::expose::{expose_class, str_arg, this};
use pz_kahlua::value::LuaObject;
use pz_kahlua::{LuaError, LuaResult, NativeCall, Thread, Value};
use pz_scripts::objects::items::{AmmoType, GameMode, ItemBodyLocation, ItemType};
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
pub const ITEM_TYPE_CLASS: &str = "zombie.scripting.objects.ItemType";
pub const ITEM_TAG_CLASS: &str = "zombie.scripting.objects.ItemTag";
pub const ITEM_BODY_LOCATION_CLASS: &str = "zombie.scripting.objects.ItemBodyLocation";
pub const WEAPON_CATEGORY_CLASS: &str = "zombie.scripting.objects.WeaponCategory";
pub const AMMO_TYPE_CLASS: &str = "zombie.scripting.objects.AmmoType";

/// Key classes exposed here: `(Java class, registry)`.
pub const KEY_CLASSES: &[(&str, &str)] = &[
    (CHARACTER_PROFESSION_CLASS, "character_profession"),
    (CHARACTER_TRAIT_CLASS, "character_trait"),
    (GAME_MODE_CLASS, "game_mode"),
    (ITEM_TYPE_CLASS, "item_type"),
    (ITEM_TAG_CLASS, "item_tag"),
    (ITEM_BODY_LOCATION_CLASS, "item_body_location"),
    (WEAPON_CATEGORY_CLASS, "weapon_category"),
    (AMMO_TYPE_CLASS, "ammo_type"),
];

/// The key class exposed for registry `registry`.
pub fn key_class(registry: &str) -> Option<&'static str> {
    KEY_CLASSES.iter().find(|(_, r)| *r == registry).map(|(c, _)| *c)
}

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

pub const ITEM_KEY_CLASS: &str = "zombie.scripting.objects.ItemKey";

/// An `ItemKey` record: `id` and `itemType`.
pub struct ItemKeyObject {
    pub id: String,
    pub item_type: Option<Rc<RegistryKey>>,
}

impl LuaObject for ItemKeyObject {
    fn class_name(&self) -> &str {
        ITEM_KEY_CLASS
    }
    /// `toString`: `Base.<id>`.
    fn to_lua_string(&self) -> String {
        format!("Base.{}", self.id)
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn super_classes(&self) -> &[&'static str] {
        &["java.lang.Record"]
    }
}

/// `ItemKey` and its holder classes, one canonical object per constant.
fn register_item_keys(thread: &mut Thread, sm: &Rc<ScriptManager>) {
    let b = expose_class(thread, ITEM_KEY_CLASS, None);
    fn this_item_key<R>(c: &NativeCall<'_>, f: impl FnOnce(&ItemKeyObject) -> R) -> LuaResult<R> {
        crate::expose::with_this::<ItemKeyObject, R>(c, f)
    }
    b.method("id", |c| {
        let v = this_item_key(c, |k| k.id.clone())?;
        Ok(c.push(v))
    });
    b.method("itemType", |c| {
        let v = this_item_key(c, |k| key_value(ITEM_TYPE_CLASS, k.item_type.as_ref()))?;
        Ok(c.push(v))
    });
    b.method("toString", |c| {
        let v = this_item_key(c, |k| k.to_lua_string())?;
        Ok(c.push(v))
    });
    // The record's `equals`: same `id` and `itemType`.
    b.method("equals", |c| {
        let (id, ty) = this_item_key(c, |k| (k.id.clone(), k.item_type.clone()))?;
        let eq = match c.arg(1) {
            Value::Object(o) => o.as_any().downcast_ref::<ItemKeyObject>().is_some_and(|k| {
                k.id == id
                    && match (&k.item_type, &ty) {
                        (Some(a), Some(b)) => Rc::ptr_eq(a, b),
                        (None, None) => true,
                        _ => false,
                    }
            }),
            _ => false,
        };
        Ok(c.push(eq))
    });
    let decls = sm.builtin_tables.borrow().item_keys.clone();
    let types = sm.registries.get("item_type").expect("known registry");
    let mut holder: Option<(String, crate::expose::ClassBuilder)> = None;
    for d in decls {
        if holder.as_ref().is_none_or(|(h, _)| *h != d.holder) {
            let class: &'static str = Box::leak(format!("{ITEM_KEY_CLASS}${}", d.holder).into_boxed_str());
            holder = Some((d.holder.clone(), expose_class(thread, class, None)));
        }
        let item_type = sm
            .statics
            .items
            .borrow()
            .constants
            .id("item_type", &d.item_type)
            .and_then(|id| ResourceLocation::of(id).ok())
            .and_then(|l| types.borrow().get(&l));
        let v = Value::Object(Rc::new(ItemKeyObject { id: d.id, item_type }));
        holder.as_ref().unwrap().1.field(&d.field, v);
    }
}

/// Register `ResourceLocation`, the key classes of [`KEY_CLASSES`] and
/// `ItemKey`.
pub fn register(thread: &mut Thread, sm: Rc<ScriptManager>) {
    register_item_keys(thread, &sm);
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
        if class == AMMO_TYPE_CLASS {
            b.static_fn("register", move |c| {
                let key = AmmoType::register_with_item(&s, &str_or_empty(c, 0), &str_or_empty(c, 1));
                Ok(c.push(key_value(class, Some(&key.map_err(java_error)?))))
            });
            let s = sm.clone();
            b.static_fn("getByItemKey", move |c| {
                let key = AmmoType::get_by_item_key(&s, &str_or_empty(c, 0));
                Ok(c.push(key_value(class, key.as_ref())))
            });
        } else {
            b.static_fn("register", move |c| {
                let id = str_or_empty(c, 0);
                let key = s.registries.get(registry).expect("known registry").borrow_mut().register_id(&id, false);
                Ok(c.push(key_value(class, Some(&key.map_err(java_error)?))))
            });
        }
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
        } else if class == CHARACTER_TRAIT_CLASS || class == CHARACTER_PROFESSION_CLASS {
            b.method("getName", |c| {
                let k = this_key(c)?;
                Ok(c.push(k.path()))
            });
        } else {
            b.method("getTranslationName", move |c| {
                let k = this_key(c)?;
                let name = match class {
                    ITEM_TYPE_CLASS => ItemType::translation_name(&k),
                    ITEM_BODY_LOCATION_CLASS => ItemBodyLocation::translation_name(&k).to_string(),
                    _ => k.name.clone(),
                };
                Ok(c.push(name))
            });
            if class == AMMO_TYPE_CLASS {
                b.method("getItemKey", |c| {
                    let k = this_key(c)?;
                    Ok(c.push(AmmoType::item_key(&k).map(Value::from).unwrap_or_default()))
                });
            }
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

    #[test]
    fn item_keys() {
        use pz_scripts::registry::builtin::ItemKeyDecl;
        let mut t = Thread::new(TableRef::new());
        pz_kahlua::stdlib::install(&mut t, None).unwrap();
        run(&mut t, "function assert(v, msg) if not v then error(msg or 'assertion failed!', 2) end return v end").unwrap();
        let sm = sm_with_keys();
        sm.registries.get("item_type").unwrap().borrow_mut().register_id("t:boxy", false).unwrap();
        sm.statics.items.borrow_mut().constants.insert("item_type", "BOXY", "t:boxy");
        let decl = |holder: &str, field: &str, id: &str| ItemKeyDecl {
            holder: holder.into(),
            field: field.into(),
            id: id.into(),
            item_type: "BOXY".into(),
        };
        sm.builtin_tables.borrow_mut().item_keys =
            vec![decl("Crate", "SMALL_CRATE", "SmallCrate"), decl("Crate", "BIG_CRATE", "BigCrate"), decl("Odd", "THING", "Thing")];
        register(&mut t, sm);
        run(
            &mut t,
            r#"
            local k = ItemKey.Crate.SMALL_CRATE
            assert(k == ItemKey.Crate.SMALL_CRATE and k ~= ItemKey.Crate.BIG_CRATE)
            assert(k:id() == "SmallCrate" and tostring(k) == "Base.SmallCrate" and k:toString() == "Base.SmallCrate")
            assert(k:itemType() == ItemType.BOXY and ItemKey.Odd.THING:itemType() == ItemType.BOXY)
            assert(k:equals(ItemKey.Crate.SMALL_CRATE) and not k:equals(ItemKey.Odd.THING))
            "#,
        )
        .unwrap();
    }
}
