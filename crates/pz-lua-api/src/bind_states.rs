//! The `zombie.ai.states` singletons (`ClimbThroughWindowState.instance()`
//! and the other states with a static `instance()`, as the jar declares
//! them) and the character's state-machine and animation-variable methods.
//!
//! Lua compares a character's current state with the singletons and uses
//! them as table keys, so each class has one canonical object. Behind each
//! object is the ported state of `pz_characters::character::states` (or a
//! base-class stand-in for states not ported yet), which `changeState`
//! enters on the character.
//!
//! State calls run with the installed world ([`install_state_world`];
//! nothing but the character itself by default) and the simulation context
//! of [`crate::bind_character_sim`].

use crate::bind_character_sim::{chr_of, with_ctx, LuaGameCharacter, ISO_GAME_CHARACTER_CLASS};
use crate::bind_characters::{shared_characters, LuaItem};
use crate::bind_registry::shared_script_manager;
use crate::expose::{bool_arg, expose_class, num_arg, str_arg, with_this};
use pz_anim::{AnimationVariableSource, VariableValue};
use pz_characters::character::state::{NullWorld, StateCx, StateRef, StateRegistry, StateWorld};
use pz_kahlua::value::LuaObject;
use pz_kahlua::{LuaResult, NativeCall, Thread, Value};
use pz_scripts::ScriptManager;
use std::any::Any;
use std::cell::RefCell;
use std::collections::HashMap;
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

type SharedWorld = Rc<RefCell<dyn StateWorld<LuaItem>>>;

thread_local! {
    static REGISTRY: RefCell<StateRegistry<LuaItem>> = RefCell::new(StateRegistry::with_ported());
    static OBJECTS: RefCell<HashMap<&'static str, Value>> = RefCell::new(HashMap::new());
    static WORLD: RefCell<SharedWorld> = RefCell::new(Rc::new(RefCell::new(NullWorld)));
}

/// Makes `world` the world state calls from Lua see.
pub fn install_state_world(world: SharedWorld) {
    WORLD.with(|w| *w.borrow_mut() = world);
}

/// Runs `f` with the state singletons the bound characters use, for the
/// engine's character updates (`IsoGameCharacter.update` enters the same
/// `instance()` objects Lua compares against). `f` must not call into Lua.
pub fn with_state_registry<R>(f: impl FnOnce(&mut StateRegistry<LuaItem>) -> R) -> R {
    REGISTRY.with(|r| f(&mut r.borrow_mut()))
}

/// The canonical Lua object of the state class `class`.
fn object_of(class: &'static str) -> Value {
    OBJECTS.with(|o| o.borrow_mut().entry(class).or_insert_with(|| Value::Object(Rc::new(StateObject { class }))).clone())
}

/// The Lua object of a state (`nil` for none).
pub fn state_value(s: Option<StateRef<LuaItem>>) -> Value {
    match s {
        Some(s) => object_of(s.id().0),
        None => Value::Nil,
    }
}

/// The state behind a Lua state object (`nil` is none).
pub fn state_arg(c: &NativeCall<'_>, i: usize) -> LuaResult<Option<StateRef<LuaItem>>> {
    let v = c.arg(i);
    if v.is_nil() {
        return Ok(None);
    }
    let class = match &v {
        Value::Object(o) => o.as_any().downcast_ref::<StateObject>().map(|s| s.class),
        _ => None,
    };
    match class {
        Some(class) => Ok(Some(REGISTRY.with(|r| r.borrow_mut().get_or_opaque(class)))),
        None => Err(crate::bind_java_util::class_cast(&v, STATE_CLASS)),
    }
}

/// Runs `f` on the character with a state context.
pub(crate) fn with_state_cx<R>(
    c: &mut NativeCall<'_>,
    f: impl FnOnce(&mut LuaGameCharacter, &mut StateCx<'_, LuaItem>) -> R,
) -> LuaResult<R> {
    let chr = chr_of(c)?;
    let this = c.arg(0);
    let sm = shared_script_manager();
    let chars = shared_characters();
    let world = WORLD.with(|w| w.borrow().clone());
    with_ctx(c, &this, |ctx| {
        let chars = chars.borrow();
        let mut world = world.borrow_mut();
        let mut cx = StateCx::new(&sm, &chars, ctx, &mut *world);
        let mut chr = chr.borrow_mut();
        Ok(f(&mut chr, &mut cx))
    })
}

/// A Lua value as a variable value (`setVariable` overloads).
fn variable_value(v: &Value) -> VariableValue {
    match v {
        Value::Bool(b) => VariableValue::Bool(*b),
        Value::Num(n) => VariableValue::Float(*n as f32),
        Value::Str(s) => VariableValue::String(s.to_string()),
        _ => VariableValue::String(String::new()),
    }
}

fn key_arg(c: &NativeCall<'_>, i: usize) -> String {
    str_arg(c, i).map(|s| s.to_string()).unwrap_or_default()
}

/// Register the singletons `sm`'s built-in tables list (and the ported
/// states) and the character methods.
pub fn register(thread: &mut Thread, sm: &Rc<ScriptManager>) {
    let st = expose_class(thread, STATE_CLASS, None);
    st.method("getName", |c| {
        let class = with_this::<StateObject, _>(c, |o| o.class)?;
        Ok(c.push(class.rsplit(['.', '$']).next().unwrap_or(class)))
    });
    let mut classes = sm.builtin_tables.borrow().state_singletons.clone();
    for s in pz_characters::character::states::all::<LuaItem>() {
        let name = s.id().0.replace('.', "/");
        if !classes.contains(&name) {
            classes.push(name);
        }
    }
    for class in classes {
        let fqn: &'static str = Box::leak(class.replace('/', ".").into_boxed_str());
        let b = expose_class(thread, fqn, Some(STATE_CLASS));
        b.static_fn("instance", move |c| Ok(c.push(object_of(fqn))));
    }
    register_character_methods(thread);
}

fn register_character_methods(thread: &mut Thread) {
    let b = expose_class(thread, ISO_GAME_CHARACTER_CLASS, None);
    b.method("changeState", |c| {
        let s = state_arg(c, 1)?;
        with_state_cx(c, |chr, cx| chr.change_state(s, cx))?;
        Ok(0)
    });
    b.method("getCurrentState", |c| {
        let s = chr_of(c)?.borrow().current_state();
        Ok(c.push(state_value(s)))
    });
    b.method("isCurrentState", |c| {
        let s = state_arg(c, 1)?;
        let chr = chr_of(c)?;
        let r = s.is_some_and(|s| chr.borrow().is_current_state(s));
        Ok(c.push(r))
    });
    b.method("getCurrentStateName", |c| {
        let n = chr_of(c)?.borrow().current_state_name();
        Ok(c.push(n.map_or(Value::Nil, Value::str)))
    });
    b.method("getPreviousStateName", |c| {
        let n = chr_of(c)?.borrow().previous_state_name();
        Ok(c.push(n.map_or(Value::Nil, Value::str)))
    });
    b.method("getDefaultState", |c| {
        let s = chr_of(c)?.borrow().default_state();
        Ok(c.push(state_value(s)))
    });
    b.method("setDefaultState", |c| {
        if c.nargs() >= 2 {
            let s = state_arg(c, 1)?;
            chr_of(c)?.borrow_mut().set_default_state_to(s);
        } else {
            with_state_cx(c, |chr, cx| chr.set_default_state(cx))?;
        }
        Ok(0)
    });
    b.method("setStateMachineLocked", |c| {
        let v = bool_arg(c, 1);
        chr_of(c)?.borrow_mut().set_state_machine_locked(v);
        Ok(0)
    });
    b.method("reportEvent", |c| {
        let e = key_arg(c, 1);
        chr_of(c)?.borrow_mut().report_event(&e);
        Ok(0)
    });
    b.method("getActionStateName", |c| {
        let n = chr_of(c)?.borrow().action_state_name();
        Ok(c.push(n))
    });
    b.method("getCurrentActionContextStateName", |c| {
        let n = chr_of(c)?.borrow().current_action_context_state_name();
        Ok(c.push(n))
    });
    b.method("getPreviousActionContextStateName", |c| {
        let n = chr_of(c)?.borrow().previous_action_context_state_name();
        Ok(c.push(n))
    });
    for name in ["setVariable", "SetVariable"] {
        b.method(name, |c| {
            let key = key_arg(c, 1);
            let v = variable_value(&c.arg(2));
            chr_of(c)?.borrow_mut().set_variable(&key, v);
            Ok(0)
        });
    }
    for name in ["clearVariable", "ClearVariable"] {
        b.method(name, |c| {
            let key = key_arg(c, 1);
            chr_of(c)?.borrow_mut().clear_variable(&key);
            Ok(0)
        });
    }
    b.method("clearVariables", |c| {
        chr_of(c)?.borrow_mut().clear_variables();
        Ok(0)
    });
    for name in ["getVariableString", "GetVariable"] {
        b.method(name, |c| {
            let key = key_arg(c, 1);
            let v = chr_of(c)?.borrow().get_variable_string(&key);
            Ok(c.push(v))
        });
    }
    b.method("getVariableFloat", |c| {
        let key = key_arg(c, 1);
        let d = num_arg(c, 2) as f32;
        let v = chr_of(c)?.borrow().get_variable_float(&key, d);
        Ok(c.push(Value::Num(f64::from(v))))
    });
    b.method("getVariableBoolean", |c| {
        let key = key_arg(c, 1);
        let chr = chr_of(c)?;
        let v = if c.nargs() >= 3 {
            chr.borrow().variable_boolean_or(&key, bool_arg(c, 2))
        } else {
            chr.borrow().get_variable_boolean(&key)
        };
        Ok(c.push(v))
    });
    b.method("isVariable", |c| {
        let key = key_arg(c, 1);
        let val = key_arg(c, 2);
        let v = chr_of(c)?.borrow().is_variable(&key, &val);
        Ok(c.push(v))
    });
    b.method("containsVariable", |c| {
        let key = key_arg(c, 1);
        let v = chr_of(c)?.borrow().contains_variable(&key);
        Ok(c.push(v))
    });
    crate::bind_combat::register_character_methods(&b);
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
            assert(IdleState.instance():getName() == "IdleState")
        "#;
        let p = pz_kahlua::compiler::compile(src, Some("t"), Some("t"), None).unwrap();
        let f = Value::Closure(pz_kahlua::vm::LuaClosure::new(Rc::new(p), t.globals().clone()));
        t.pcall(&f, &[]).map_err(|(e, tr)| format!("{}\n{tr}", pz_kahlua::exec::java_value_of(&e))).unwrap();
    }
}
