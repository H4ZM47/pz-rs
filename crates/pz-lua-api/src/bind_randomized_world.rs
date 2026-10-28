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

thread_local! {
    /// The static lists, by field name.
    static LISTS: RefCell<HashMap<String, Rc<JavaList>>> = RefCell::default();
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
    let b = expose_class(thread, CLASS, Some("java.lang.Object"));
    for (method, field) in getters {
        let field = field.clone();
        b.method(method, move |c| {
            crate::expose::this::<LuaRandomizedWorldBase>(c)?;
            Ok(c.push(Value::Object(list_of(&field))))
        });
    }
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
}

/// [`register`] with the getters read from `projectzomboid.jar`.
pub fn register_from_jar(thread: &mut Thread, jar: Option<&pz_classfile::Jar>) {
    let getters = jar
        .and_then(|j| pz_classfile::load_class(j, &CLASS.replace('.', "/")).ok())
        .map(|c| clutter_getters_from_class(&c))
        .unwrap_or_default();
    register(thread, &getters);
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
        "#;
        let f = pz_kahlua::stdlib::load_string(src, Some("test"), t.globals().clone()).unwrap();
        t.call(&f, &[]).unwrap();
        assert_eq!(clutter("barnClutter"), ["Base.Hat", "Base.Hat"]);
    }
}
