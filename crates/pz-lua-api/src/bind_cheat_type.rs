//! `zombie.characters.CheatType` and the `getCheatTypes()` global
//! (`LuaManager.GlobalObject.getCheatTypes`, which returns
//! `CheatType.getList()`: `List.copyOf(Arrays.asList(values()))`).
//!
//! The constants and their tooltips (`new CheatType(name, ordinal,
//! tooltip)`) are read at start-up from the installed `projectzomboid.jar`.

use crate::bind_java_util::JavaList;
use crate::expose::{enum_value, expose_enum, global, with_this, JavaEnum};
use pz_classfile::{Jar, Value as ClassValue};
use pz_kahlua::{Thread, Value};
use std::rc::Rc;

pub const CHEAT_TYPE_CLASS: &str = "zombie.characters.CheatType";
const CHEAT_TYPE_PATH: &str = "zombie/characters/CheatType";

/// A constant's name and its `tooltip`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CheatType {
    pub name: String,
    pub tooltip: String,
}

/// The constants in `values()` order.
pub fn read(jar: &Jar) -> Result<Vec<CheatType>, String> {
    let names = pz_classfile::enum_constants(jar, CHEAT_TYPE_PATH).map_err(|e| e.to_string())?;
    let ctors = pz_classfile::self_constructed_constants(jar, CHEAT_TYPE_PATH).map_err(|e| e.to_string())?;
    names
        .into_iter()
        .enumerate()
        .map(|(ordinal, name)| match ctors.iter().find(|(f, _)| *f == name).map(|(_, a)| a.as_slice()) {
            Some([ClassValue::Str(n), ClassValue::Int(o), ClassValue::Str(tooltip)]) if *n == name && *o == ordinal as i64 => {
                Ok(CheatType { tooltip: tooltip.clone(), name })
            }
            args => Err(format!("{name}: unexpected constructor arguments {args:?}")),
        })
        .collect()
}

/// Register the enum (with `getTooltip`) and `getCheatTypes()`.
pub fn register(thread: &mut Thread, types: &[CheatType]) {
    let names: Vec<&'static str> = types.iter().map(|t| &*Box::leak(t.name.clone().into_boxed_str())).collect();
    let b = expose_enum(thread, CHEAT_TYPE_CLASS, &names);
    let tooltips: Rc<Vec<String>> = Rc::new(types.iter().map(|t| t.tooltip.clone()).collect());
    b.method("getTooltip", move |c| {
        let o = with_this::<JavaEnum, _>(c, |e| e.ordinal)?;
        Ok(c.push(tooltips.get(o as usize).map_or(Value::Nil, |t| Value::from(t.as_str()))))
    });
    let values: Rc<Vec<Value>> = Rc::new(names.iter().map(|n| enum_value(CHEAT_TYPE_CLASS, n)).collect());
    // `List.copyOf`: an unmodifiable list.
    let list = {
        let values = values.clone();
        Value::Object(Rc::new(JavaList::live(move || values.to_vec(), None)))
    };
    b.static_fn("getList", {
        let list = list.clone();
        move |c| Ok(c.push(list.clone()))
    });
    global(thread, "getCheatTypes", move |c| Ok(c.push(list.clone())));
}

/// Read the constants from `jar` and register them; a jar that cannot be
/// read leaves both unregistered and logs why.
pub fn register_from_jar(thread: &mut Thread, jar: Option<&Jar>) {
    match jar.ok_or_else(|| "no projectzomboid.jar".to_string()).and_then(read) {
        Ok(types) => register(thread, &types),
        Err(e) => (thread.log)(&format!("bind_cheat_type: {e}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pz_kahlua::TableRef;

    #[test]
    fn cheat_types_list_the_constants_with_their_tooltips() {
        let mut t = Thread::new(TableRef::new());
        pz_kahlua::stdlib::install(&mut t, None).unwrap();
        crate::bind_java_util::register(&mut t);
        let types = [("GOD_MODE", "GodMod"), ("INVISIBLE", "Invisible")]
            .map(|(n, tip)| CheatType { name: n.into(), tooltip: tip.into() });
        register(&mut t, &types);
        let src = r#"
            local function assert(v) if not v then error("assertion failed") end end
            local list = getCheatTypes()
            assert(list == getCheatTypes() and list:size() == 2)
            assert(list:get(0) == CheatType.GOD_MODE and list:get(1):getTooltip() == "Invisible")
            assert(not pcall(function() list:add(CheatType.GOD_MODE) end))
            return CheatType.getList():get(0):getTooltip()
        "#;
        let p = pz_kahlua::compiler::compile(src, Some("t"), None, None).unwrap();
        let f = Value::Closure(pz_kahlua::vm::LuaClosure::new(Rc::new(p), t.globals().clone()));
        let r = t.call(&f, &[]).unwrap();
        assert!(matches!(&r[..], [Value::Str(s)] if &**s == "GodMod"));
    }

    /// The game's constants, with `PZ_GAME_DIR` set.
    #[test]
    fn game_jar_cheat_types() {
        let Some(game) = std::env::var_os("PZ_GAME_DIR") else { return };
        let jar = Jar::open(std::path::PathBuf::from(game).join("projectzomboid.jar")).unwrap();
        let types = read(&jar).unwrap();
        assert_eq!(types[0], CheatType { name: "GOD_MODE".into(), tooltip: "GodMod".into() });
        assert_eq!(types.len(), 25);
    }
}
