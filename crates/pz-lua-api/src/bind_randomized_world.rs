//! `zombie.randomizedWorld.RandomizedWorldBase` as
//! `media/lua/server/RandomizedWorldContent/StoryClutter/StoryClutter_Initialization.lua`
//! uses it: `getWorld():getRandomizedWorldBase()` and its `get*Clutter()`
//! getters, which return the class's static clutter `ArrayList`s for
//! `StoryTables.initClutterArray` to fill.
//!
//! The getters are read from the game's class at runtime: each
//! `()Ljava/util/ArrayList;` method whose body is `getstatic <own field>;
//! areturn` returns that field's list ([`clutter_getters_from_class`]), so
//! two getters of one field share it as in Java. [`clutter`] gives the
//! story code a list's contents.

use crate::bind_java_util::JavaList;
use crate::expose::expose_class;
use pz_classfile::class::ClassFile;
use pz_kahlua::value::LuaObject;
use pz_kahlua::{Thread, Value};
use std::any::Any;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

pub const CLASS: &str = "zombie.randomizedWorld.RandomizedWorldBase";
pub const RBBASIC_CLASS: &str = "zombie.randomizedWorld.randomizedBuilding.RBBasic";

thread_local! {
    /// The static lists, by field name.
    static LISTS: RefCell<HashMap<String, Rc<JavaList>>> = RefCell::default();
    /// `RBBasic.uniqueRDSSpawned`: live static `ArrayList<String>` saved in `map_meta.bin`.
    static UNIQUE_RDS_SPAWNED: Rc<JavaList> = Rc::new(JavaList::new(Vec::new()));
    /// Whether `IsoWorld` has initialized `RBBasic.uniqueRDSSpawned` for the active world.
    static UNIQUE_RDS_ACTIVE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// `IsoWorld.instance.getRandomizedWorldBase()`.
pub struct LuaRandomizedWorldBase;

impl LuaObject for LuaRandomizedWorldBase {
    fn class_name(&self) -> &str {
        CLASS
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// `IsoWorld.instance.getRBBasic()` (`zombie.randomizedWorld.randomizedBuilding.RBBasic`).
pub struct LuaRBBasic;

impl LuaObject for LuaRBBasic {
    fn class_name(&self) -> &str {
        RBBASIC_CLASS
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// `RBBasic.getUniqueRDSSpawned().clear()` on `IsoWorld.init` / world exit.
pub fn clear_unique_rds_spawned() {
    UNIQUE_RDS_SPAWNED.with(|l| l.items.borrow_mut().clear());
    UNIQUE_RDS_ACTIVE.with(|a| a.set(true));
}

/// Release `RBBasic.uniqueRDSSpawned` tracking on `IngameState.exit`.
pub fn reset_unique_rds_spawned() {
    UNIQUE_RDS_SPAWNED.with(|l| l.items.borrow_mut().clear());
    UNIQUE_RDS_ACTIVE.with(|a| a.set(false));
}

/// Replace `RBBasic.uniqueRDSSpawned` in-place from `map_meta.bin`.
pub fn install_unique_rds_spawned(names: &[String]) {
    UNIQUE_RDS_SPAWNED.with(|l| {
        let mut items = l.items.borrow_mut();
        items.clear();
        items.extend(names.iter().map(|s| Value::str(s)));
    });
    UNIQUE_RDS_ACTIVE.with(|a| a.set(true));
}

/// Snapshot `RBBasic.uniqueRDSSpawned` for `map_meta.bin` (`None` if inactive).
pub fn snapshot_unique_rds_spawned() -> Option<Vec<String>> {
    if !UNIQUE_RDS_ACTIVE.with(|a| a.get()) {
        return None;
    }
    Some(UNIQUE_RDS_SPAWNED.with(|l| {
        l.snapshot()
            .iter()
            .map(pz_kahlua::exec::java_value_of)
            .collect()
    }))
}

/// The `(method, static field)` pairs of the class's list getters, in
/// method order.
pub fn clutter_getters_from_class(class: &ClassFile) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for m in &class.methods {
        if m.descriptor != "()Ljava/util/ArrayList;" {
            continue;
        }
        let Some(code) = &m.code else { continue };
        // getstatic #idx; areturn
        if code.len() != 4 || code[0] != 0xb2 || code[3] != 0xb0 {
            continue;
        }
        let idx = u16::from_be_bytes([code[1], code[2]]);
        let Ok(r) = class.member_ref(idx) else { continue };
        if r.class != class.this_class || r.descriptor != "Ljava/util/ArrayList;" {
            continue;
        }
        out.push((m.name.clone(), r.name));
    }
    out
}

/// The list in static field `field` (empty when nothing filled it).
pub fn clutter(field: &str) -> Vec<String> {
    LISTS.with(|l| {
        l.borrow().get(field).map_or_else(Vec::new, |list| {
            list.snapshot().iter().map(pz_kahlua::exec::java_value_of).collect()
        })
    })
}

/// Pick one random item from clutter static field `field` using the shared
/// `RandStandard` generator (`RandomizedWorldBase.get*ClutterItem()`).
pub fn clutter_item(field: &str) -> Option<String> {
    let items = clutter(field);
    if items.is_empty() {
        return None;
    }
    let idx = pz_core::random::with_standard(|r| r.next(items.len() as i32)) as usize;
    items.into_iter().nth(idx)
}

/// The `(method, static field)` pairs of `get*ClutterItem()` methods on
/// `RandomizedWorldBase` (`()Ljava/lang/String;` calling a clutter list getter
/// or reading a static `ArrayList` field), plus fallback `<getter>Item` names
/// for every `(getter, field)` pair in `getters`.
pub fn clutter_item_getters_from_class(
    class: &ClassFile,
    getters: &[(String, String)],
) -> Vec<(String, String)> {
    let by_getter: HashMap<&str, &str> = getters
        .iter()
        .map(|(m, f)| (m.as_str(), f.as_str()))
        .collect();
    let mut out = Vec::new();
    for m in &class.methods {
        if m.descriptor != "()Ljava/lang/String;" {
            continue;
        }
        let Some(code) = &m.code else { continue };
        // Scan for getstatic (0xb2) of a static ArrayList field or
        // invokevirtual/invokestatic (0xb6/0xb8) calling one of the clutter
        // list getters.
        let mut i = 0usize;
        let mut found: Option<String> = None;
        while i + 2 < code.len() {
            let op = code[i];
            if op == 0xb2 || op == 0xb6 || op == 0xb8 {
                let idx = u16::from_be_bytes([code[i + 1], code[i + 2]]);
                if let Ok(r) = class.member_ref(idx) {
                    if r.class == class.this_class {
                        if op == 0xb2 && r.descriptor == "Ljava/util/ArrayList;" {
                            found = Some(r.name);
                            break;
                        }
                        if (op == 0xb6 || op == 0xb8) && r.descriptor == "()Ljava/util/ArrayList;" {
                            if let Some(&field) = by_getter.get(r.name.as_str()) {
                                found = Some(field.to_string());
                                break;
                            }
                        }
                    }
                }
            }
            i += 1;
        }
        if let Some(field) = found {
            out.push((m.name.clone(), field));
        }
    }
    out
}

fn list_of(field: &str) -> Rc<JavaList> {
    LISTS.with(|l| {
        l.borrow_mut()
            .entry(field.to_string())
            .or_insert_with(|| Rc::new(JavaList::new(Vec::new())))
            .clone()
    })
}

/// The class and its getters (`getters` from
/// [`clutter_getters_from_class`]); `IsoWorld.getRandomizedWorldBase`.
pub fn register(thread: &mut Thread, getters: &[(String, String)]) {
    register_with_items(thread, getters, &[]);
}

/// [`register`] with explicit `get*ClutterItem()` mappings read from
/// `projectzomboid.jar` (falling back to `<getter>Item` for every clutter list
/// getter not already listed).
pub fn register_with_items(
    thread: &mut Thread,
    getters: &[(String, String)],
    item_getters: &[(String, String)],
) {
    let b = expose_class(thread, CLASS, Some("java.lang.Object"));
    for (method, field) in getters {
        let field = field.clone();
        b.method(method, move |c| {
            crate::expose::this::<LuaRandomizedWorldBase>(c)?;
            Ok(c.push(Value::Object(list_of(&field))))
        });
    }
    let mut registered_items: Vec<(String, String)> = item_getters.to_vec();
    for (method, field) in getters {
        let item_method = format!("{method}Item");
        if !registered_items.iter().any(|(m, _)| m == &item_method) {
            registered_items.push((item_method, field.clone()));
        }
    }
    for (method, field) in registered_items {
        b.method(&method, move |c| {
            crate::expose::this::<LuaRandomizedWorldBase>(c)?;
            let v = match clutter_item(&field) {
                Some(s) => Value::str(&s),
                None => Value::Nil,
            };
            Ok(c.push(v))
        });
    }
    let rb = expose_class(thread, RBBASIC_CLASS, Some(CLASS));
    let get_unique = |c: &mut pz_kahlua::NativeCall<'_>| {
        let list = UNIQUE_RDS_SPAWNED.with(|l| Value::Object(l.clone()));
        Ok(c.push(list))
    };
    rb.static_fn("getUniqueRDSSpawned", get_unique);
    rb.method("getUniqueRDSSpawned", get_unique);
    rb.method("getName", |c| {
        crate::expose::this::<LuaRBBasic>(c)?;
        Ok(c.push(Value::str("RBBasic")))
    });
    rb.method("getSurvivorStories", |c| {
        crate::expose::this::<LuaRBBasic>(c)?;
        Ok(c.push(Value::Object(Rc::new(JavaList::new(Vec::new())))))
    });
    rb.method("getSurvivorProfession", |c| {
        crate::expose::this::<LuaRBBasic>(c)?;
        Ok(c.push(Value::Object(Rc::new(JavaList::new(Vec::new())))))
    });

    let w = expose_class(thread, "zombie.iso.IsoWorld", Some("java.lang.Object"));
    let rwb = Value::Object(Rc::new(LuaRandomizedWorldBase));
    w.method("getRandomizedWorldBase", move |c| {
        match c.arg(0) {
            Value::Object(o) if o.class_name() == "zombie.iso.IsoWorld" => {}
            other => {
                return Err(pz_kahlua::LuaError::runtime(format!(
                    "Expected zombie.iso.IsoWorld but got {}",
                    pz_kahlua::stdlib::java_class_name(&other)
                )))
            }
        }
        Ok(c.push(rwb.clone()))
    });
    let rb_val = Value::Object(Rc::new(LuaRBBasic));
    w.method("getRBBasic", move |c| {
        match c.arg(0) {
            Value::Object(o) if o.class_name() == "zombie.iso.IsoWorld" => {}
            other => {
                return Err(pz_kahlua::LuaError::runtime(format!(
                    "Expected zombie.iso.IsoWorld but got {}",
                    pz_kahlua::stdlib::java_class_name(&other)
                )))
            }
        }
        Ok(c.push(rb_val.clone()))
    });
}

/// [`register`] with the getters read from `projectzomboid.jar`.
pub fn register_from_jar(thread: &mut Thread, jar: Option<&pz_classfile::Jar>) {
    let class = jar.and_then(|j| pz_classfile::load_class(j, &CLASS.replace('.', "/")).ok());
    let getters = class
        .as_ref()
        .map(clutter_getters_from_class)
        .unwrap_or_default();
    let item_getters = class
        .as_ref()
        .map(|c| clutter_item_getters_from_class(c, &getters))
        .unwrap_or_default();
    register_with_items(thread, &getters, &item_getters);
}

#[cfg(test)]
mod tests {
    use super::*;
    use pz_kahlua::TableRef;

    #[test]
    fn getters_share_lists() {
        let mut t = Thread::new(TableRef::new());
        pz_kahlua::stdlib::install(&mut t, None).unwrap();
        crate::bind_java_util::register(&mut t);
        let getters = vec![
            ("getBarnClutter".to_string(), "barnClutter".to_string()),
            ("getBBQClutter".to_string(), "bbqClutter".to_string()),
            ("getBarnClutterAgain".to_string(), "barnClutter".to_string()),
        ];
        register(&mut t, &getters);
        t.globals().set_str("rwb", Value::Object(Rc::new(LuaRandomizedWorldBase)));
        let src = r#"
            -- assert normally comes from the installed stdlib.lua.
            local function assert(c) if not c then error("assertion failed") end end
            local l = rwb:getBarnClutter()
            l:clear()
            l:add("Base.Hat")
            l:add("Base.Hat")
            assert(rwb:getBarnClutterAgain():size() == 2)
            assert(rwb:getBBQClutter():size() == 0)
            assert(rwb:getBarnClutterItem() == "Base.Hat")
            assert(rwb:getBBQClutterItem() == nil)
        "#;
        let f = pz_kahlua::stdlib::load_string(src, Some("test"), t.globals().clone()).unwrap();
        t.call(&f, &[]).unwrap();
        assert_eq!(clutter("barnClutter"), ["Base.Hat", "Base.Hat"]);
        assert_eq!(clutter_item("barnClutter").as_deref(), Some("Base.Hat"));
        assert_eq!(clutter_item("bbqClutter"), None);
    }

    #[test]
    fn rbbasic_unique_rds_spawned_roundtrip_and_clear() {
        reset_unique_rds_spawned();
        assert!(snapshot_unique_rds_spawned().is_none());

        let mut t = Thread::new(TableRef::new());
        pz_kahlua::stdlib::install(&mut t, None).unwrap();
        crate::bind_java_util::register(&mut t);
        register(&mut t, &[]);

        clear_unique_rds_spawned();
        let src = r#"
            local l = RBBasic.getUniqueRDSSpawned()
            l:add("RDSPoliceAtHouse")
            l:add("RDSBedroom")
            return l:size()
        "#;
        let f = pz_kahlua::stdlib::load_string(src, Some("test"), t.globals().clone()).unwrap();
        let out = t.call(&f, &[]).unwrap();
        assert_eq!(out[0].as_num(), Some(2.0));
        assert_eq!(
            snapshot_unique_rds_spawned(),
            Some(vec!["RDSPoliceAtHouse".into(), "RDSBedroom".into()])
        );

        install_unique_rds_spawned(&["RestoredStory".into()]);
        assert_eq!(
            snapshot_unique_rds_spawned(),
            Some(vec!["RestoredStory".into()])
        );

        reset_unique_rds_spawned();
        assert!(snapshot_unique_rds_spawned().is_none());
    }
}
