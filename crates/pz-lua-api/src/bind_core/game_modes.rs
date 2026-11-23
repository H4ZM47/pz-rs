//! Core's actual nullable GameMode field and identity query.
use super::{game_mode_error, CoreObject};
use crate::expose::ClassBuilder;
use pz_entity::RetiredComponents;
use pz_kahlua::{LuaResult, NativeCall, Value};
use std::any::Any;
use std::cell::RefCell;
use std::rc::Rc;

struct Inputs(RetiredComponents);
impl Inputs {
    fn new(call: &NativeCall<'_>) -> Self {
        let mut pins = RetiredComponents::new();
        for index in 0..call.nargs() {
            pins.push_payload(Box::new(call.arg(index)));
        }
        Self(pins)
    }
}
impl Drop for Inputs {
    fn drop(&mut self) {
        pz_inventory::component_host::retire_components(std::mem::take(&mut self.0));
    }
}
fn receiver(call: &NativeCall<'_>) -> LuaResult<Rc<RefCell<pz_core::core::Core>>> {
    if let Value::Object(object) = call.arg(0) {
        if let Some(core) = (&*object as &dyn Any).downcast_ref::<CoreObject>() {
            return Ok(core.core.clone());
        }
    }
    Err(crate::bind_world_object::unsupported(
        "Core GameMode receiver conversion",
    ))
}
fn arity(call: &NativeCall<'_>, expected: usize) -> LuaResult<()> {
    if call.nargs() == expected {
        Ok(())
    } else {
        Err(crate::bind_world_object::unsupported(
            "Core GameMode argument count conversion",
        ))
    }
}
fn borrowed() -> pz_kahlua::LuaError {
    pz_kahlua::LuaError::Runtime {
        class: "pz.host.Borrowed",
        message: "Core GameMode field".into(),
    }
}
pub(super) fn register(builder: &ClassBuilder) {
    builder.method("getGameMode", |call| {
        let _pins = Inputs::new(call);
        let core = receiver(call)?;
        arity(call, 1)?;
        let mode = core
            .try_borrow()
            .map_err(|_| borrowed())?
            .get_game_mode_ref();
        Ok(call.push(crate::bind_registry::key_value(
            crate::bind_registry::GAME_MODE_CLASS,
            mode.as_ref(),
        )))
    });
    builder.method("isTutorial", |call| {
        let _pins = Inputs::new(call);
        let core = receiver(call)?;
        arity(call, 1)?;
        let value = core
            .try_borrow()
            .map_err(|_| borrowed())?
            .is_tutorial()
            .map_err(game_mode_error)?;
        Ok(call.push(value))
    });
    builder.method("setGameMode", |call| {
        let _pins = Inputs::new(call);
        let core = receiver(call)?;
        arity(call, 2)?;
        match call.arg(1) {
            Value::Str(id) => core
                .try_borrow_mut()
                .map_err(|_| borrowed())?
                .set_game_mode(&id)
                .map_err(game_mode_error)?,
            Value::Nil => {
                return Err(crate::bind_world_object::unsupported(
                    "Core.setGameMode null overload selection",
                ))
            }
            value => {
                let mode = crate::bind_registry::game_mode_arg(&value)?;
                core.try_borrow_mut()
                    .map_err(|_| borrowed())?
                    .set_game_mode_ref(mode)
                    .map_err(game_mode_error)?;
            }
        }
        Ok(0)
    });
}

#[cfg(test)]
mod tests;
