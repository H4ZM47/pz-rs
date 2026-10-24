//! The `zombie.ai.states` singletons (`ClimbThroughWindowState.instance()`
//! and the other states with a static `instance()`, as the jar declares
//! them): Lua compares a character's current state with them and uses them
//! as table keys, so each is one canonical object. Only identity is ported;
//! the states' behaviour belongs to the character state machine.
//! TODO(State): `enter`/`execute`/`exit` and the state machine.

use crate::bind_registry::shared_script_manager;
use crate::expose::expose_class;
use pz_kahlua::value::LuaObject;
use pz_kahlua::{Thread, Value};
use pz_scripts::ScriptManager;
use std::any::Any;
use std::rc::Rc;

pub const STATE_CLASS: &str = "zombie.ai.State";

/// A state singleton.
pub struct StateObject {
    pub class: &'static str,
}

impl LuaObject for StateObject {
    fn class_name(&self) -> &str {
        self.class
    }
    /// `Object.toString`.
    fn to_lua_string(&self) -> String {
        format!("{}@{:x}", self.class, self as *const Self as usize)
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn super_classes(&self) -> &[&'static str] {
        &[STATE_CLASS]
    }
}

/// Register the singletons `sm`'s built-in tables list.
pub fn register(thread: &mut Thread, sm: &Rc<ScriptManager>) {
    expose_class(thread, STATE_CLASS, None);
    let classes = sm.builtin_tables.borrow().state_singletons.clone();
    for class in classes {
        let fqn: &'static str = Box::leak(class.replace('/', ".").into_boxed_str());
        let b = expose_class(thread, fqn, Some(STATE_CLASS));
        let instance = Value::Object(Rc::new(StateObject { class: fqn }));
        b.static_fn("instance", move |c| Ok(c.push(instance.clone())));
    }
}

/// `register` on the shared ScriptManager.
pub fn register_shared(thread: &mut Thread) {
    register(thread, &shared_script_manager());
}

#[cfg(test)]
mod tests {
    use super::*;
    use pz_kahlua::TableRef;

    #[test]
    fn singletons_are_canonical() {
        let mut t = Thread::new(TableRef::new());
        pz_kahlua::stdlib::install(&mut t, None).unwrap();
        let sm = Rc::new(ScriptManager::new());
        sm.builtin_tables.borrow_mut().state_singletons = vec!["t/states/HopState".into(), "t/states/SkipState".into()];
        register(&mut t, &sm);
        let src = r#"
            local function assert(v) if not v then error("assertion failed!", 2) end end
            local s = {}
            s[HopState.instance()] = true
            assert(s[HopState.instance()] and not s[SkipState:instance()])
            assert(HopState.instance() ~= SkipState.instance())
        "#;
        let p = pz_kahlua::compiler::compile(src, Some("t"), Some("t"), None).unwrap();
        let f = Value::Closure(pz_kahlua::vm::LuaClosure::new(Rc::new(p), t.globals().clone()));
        t.pcall(&f, &[]).map_err(|(e, tr)| format!("{}\n{tr}", pz_kahlua::exec::java_value_of(&e))).unwrap();
    }
}
