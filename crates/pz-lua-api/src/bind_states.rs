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

use crate::bind_character_sim::{chr_of, with_ctx_state, LuaGameCharacter, ISO_GAME_CHARACTER_CLASS};
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

/// Retire the installed world and the old environment's state wrappers.
/// The actual state registry remains the engine's existing singleton set.
pub(crate) fn reset_session_handles() {
    WORLD.with(|w| *w.borrow_mut() = Rc::new(RefCell::new(NullWorld)));
    OBJECTS.with(|o| o.borrow_mut().clear());
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
    with_ctx_state(c, &this, |ctx| {
        use pz_entity::runtime_load::RuntimeHostError;
        let chars = chars.try_borrow().map_err(|_| RuntimeHostError::Borrowed("state characters"))?;
        let mut world = world.try_borrow_mut().map_err(|_| RuntimeHostError::Borrowed("state world"))?;
        let mut cx = StateCx::new(&sm, &chars, ctx, &mut *world);
        let mut chr = chr.try_borrow_mut().map_err(|_| RuntimeHostError::Borrowed("state character"))?;
        let result = f(&mut chr, &mut cx);
        cx.state_result()?;
        Ok(result)
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
    fn state_context_surfaces_first_host_fault_after_guards_and_completed_prefix_event() {
        use pz_entity::runtime_load::RuntimeHostError;
        let (mut t, player, _)=crate::bind_character_sim::tests::setup_with(ScriptManager::new());
        let world=WORLD.with(|world|world.borrow().clone());
        let observed=Rc::new(std::cell::Cell::new(0));
        let sink_count=observed.clone();
        let sink_player=player.clone();
        let sink_world=world.clone();
        crate::bind_character_sim::install_event_sink(Rc::new(move |_: &mut Thread, name: &str, _: &[Value]| {
            assert!(sink_player.try_borrow_mut().is_ok());
            assert!(sink_world.try_borrow_mut().is_ok());
            assert_eq!(name,"OnContainerUpdate");
            sink_count.set(sink_count.get()+1);
        }));
        let later=Rc::new(std::cell::Cell::new(false));
        let ran=later.clone();
        t.globals().set_str("stateProbe",Value::Native(pz_kahlua::vm::NativeFunction::new("stateProbe",move |c| {
            with_state_cx(c,|_,cx| {
                cx.ctx.event(pz_characters::character::SimEvent::ContainerUpdate);
                cx.record_host_error(RuntimeHostError::Borrowed("first state phase"));
                cx.record_host_error(RuntimeHostError::Unsupported {operation:"later state phase",kind:None});
            })?;
            ran.set(true);
            Ok(0)
        })));
        let error=crate::bind_character_sim::tests::run(&mut t,"stateProbe(player)").unwrap_err();
        assert!(error.contains("first state phase") && error.contains("pz.host.Borrowed"));
        assert!(!error.contains("later state phase"));
        assert_eq!(observed.get(),1);
        assert!(!later.get());
    }

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


#[cfg(test)]
mod retained_animal_cause_tests {
    use super::*;
    use pz_characters::character::state::StateFailure;
    use pz_kahlua::{LuaError, vm::NativeFunction};

    fn install_probe(t: &mut Thread, cause: Rc<pz_characters::character::animal::AnimalDataError>, later: Rc<std::cell::Cell<bool>>) -> Value {
        let native = Value::Native(NativeFunction::new("retainedState", move |c| {
            with_state_cx(c, |_, cx| {
                cx.record_state_error(StateFailure::Animal(cause.clone()));
                cx.record_host_error(pz_entity::runtime_load::RuntimeHostError::Borrowed("later fault"));
            })?;
            later.set(true);
            Ok(0)
        }));
        t.globals().set_str("retainedState", native.clone());
        native
    }

    fn assert_original(error: &LuaError, original: &Rc<pz_characters::character::animal::AnimalDataError>) {
        let erased: Rc<dyn std::error::Error> = original.clone();
        assert!(Rc::ptr_eq(error.retained_cause().expect("retained original cause"), &erased));
        let source = std::error::Error::source(error).unwrap().downcast_ref::<pz_characters::character::animal::AnimalDataError>().unwrap();
        assert!(std::ptr::eq(source, original.as_ref()));
    }

    #[test]
    fn real_state_native_call_and_pcall_ex_keep_same_animal_cause_and_legacy_diagnostic() {
        let (mut t, player, _) = crate::bind_character_sim::tests::setup_with(ScriptManager::new());
        let original = Rc::new(pz_characters::character::animal::AnimalDataError("NullPointerException: item".into()));
        let later = Rc::new(std::cell::Cell::new(false));
        let native = install_probe(&mut t, original.clone(), later.clone());
        let arg = crate::bind_character_sim::character_value(&player);
        let diagnostic = crate::bind_registry::java_error(pz_scripts::ScriptError::new("java.lang.NullPointerException: item"));
        let plain = Value::Native(NativeFunction::new("retainedState", move |_| Err(diagnostic.clone())));
        let plain_call = t.call(&plain, std::slice::from_ref(&arg)).unwrap_err();
        let error = t.call(&native, std::slice::from_ref(&arg)).unwrap_err();
        assert_original(&error, &original);
        assert_eq!(error.pcall_message().as_str(), plain_call.pcall_message().as_str());
        assert_eq!(error.to_string(), plain_call.to_string());
        let (message, _, error) = t.pcall_ex(&native, std::slice::from_ref(&arg)).unwrap_err();
        assert_original(&error, &original);
        assert_eq!(message.as_str(), plain_call.pcall_message().as_str());
        assert!(!later.get());
        assert!(player.try_borrow_mut().is_ok(), "conversion must occur after actual character guard");
        assert!(WORLD.with(|world| world.borrow().try_borrow_mut().is_ok()));
    }

    #[test]
    fn actual_lua_pcall_presents_the_existing_message_for_retained_animal_failure() {
        let (mut t, _player, _) = crate::bind_character_sim::tests::setup_with(ScriptManager::new());
        let original = Rc::new(pz_characters::character::animal::AnimalDataError("NullPointerException: item".into()));
        let later = Rc::new(std::cell::Cell::new(false));
        install_probe(&mut t, original, later.clone());
        let diagnostic = crate::bind_registry::java_error(pz_scripts::ScriptError::new("java.lang.NullPointerException: item"));
        t.globals().set_str("plainState", Value::Native(NativeFunction::new("retainedState", move |_| Err(diagnostic.clone()))));
        let f = pz_kahlua::stdlib::load_string("local a,b=pcall(retainedState,player); local c,d=pcall(plainState,player); return a,b,c,d", Some("retained-state-pcall"), t.globals().clone()).unwrap();
        let values = t.call(&f, &[]).unwrap();
        assert!(matches!(values[0], Value::Bool(false)) && matches!(values[2], Value::Bool(false)));
        assert_eq!(values[1].as_str(), values[3].as_str());
        assert!(!later.get());
    }
}
