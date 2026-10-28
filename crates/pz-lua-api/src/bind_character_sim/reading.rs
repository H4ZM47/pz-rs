//! Native scalar reading-state methods. Collection-return APIs require their
//! own physical Java collection backend and are not projected here.
use crate::expose::ClassBuilder;
use pz_characters::character::reading::{self, ReadingHost};
use pz_entity::runtime_load::{HostResult, RuntimeHostError};
use pz_kahlua::{LuaError, LuaResult, NativeCall, Thread, Value};
use std::any::Any;
use std::cell::RefCell;

fn unsupported(operation: &'static str) -> RuntimeHostError {
    RuntimeHostError::Unsupported {
        operation,
        kind: None,
    }
}
struct InputPins(pz_entity::RetiredComponents);
impl InputPins {
    fn new(call: &NativeCall<'_>) -> Self {
        let mut retired = pz_entity::RetiredComponents::new();
        for index in 0..call.nargs() {
            retired.push_payload(Box::new(call.arg(index)));
        }
        Self(retired)
    }
}
impl Drop for InputPins {
    fn drop(&mut self) {
        pz_inventory::component_host::retire_components(std::mem::take(&mut self.0));
    }
}
struct Host<'a> {
    thread: RefCell<&'a mut Thread>,
    lua_fault: RefCell<Option<LuaError>>,
}
impl ReadingHost for Host<'_> {
    fn nights_survived(&self) -> HostResult<i32> {
        let time = crate::bind_climate::shared_game_time();
        let nights = time
            .try_borrow()
            .map_err(|_| RuntimeHostError::Borrowed("reading GameTime.nightsSurvived"))?
            .nights_survived;
        Ok(nights)
    }
    fn literature_cooldown(&self) -> HostResult<i32> {
        let value = {
            let mut thread = self
                .thread
                .try_borrow_mut()
                .map_err(|_| RuntimeHostError::Borrowed("reading Sandbox initializer thread"))?;
            crate::bind_sandbox::instance(&mut **thread)
        };
        let value = match value {
            Ok(value) => value,
            Err(error) => {
                *self.lua_fault.borrow_mut() = Some(error);
                return Err(unsupported("reading Sandbox initializer failed"));
            }
        };
        let Value::Object(object) = value else {
            return Err(unsupported("reading actual SandboxOptions instance"));
        };
        let object = (&*object as &dyn Any)
            .downcast_ref::<crate::bind_sandbox::SandboxObj>()
            .ok_or_else(|| unsupported("reading actual SandboxOptions class"))?;
        let options = object
            .0
            .try_borrow()
            .map_err(|_| RuntimeHostError::Borrowed("reading SandboxOptions"))?;
        let option = options
            .option_by_name("LiteratureCooldown")
            .ok_or_else(|| unsupported("SandboxOptions.LiteratureCooldown definition"))?;
        match &option.config.kind {
            pz_core::config::OptionKind::Integer(value) => Ok(value.value),
            _ => Err(unsupported(
                "SandboxOptions.LiteratureCooldown integer type",
            )),
        }
    }
}

pub(super) fn register(builder: &ClassBuilder) {
    for name in [
        "isLiteratureRead",
        "isPrintMediaRead",
        "getAlreadyReadPages",
    ] {
        builder.method(name, move |call| {
            let _inputs = InputPins::new(call);
            if call.nargs() != 2 {
                return Err(crate::bind_world_object::host_error(unsupported(
                    "reading method argument count conversion",
                )));
            }
            let character = super::chr_of(call)?;
            let title = match call.arg(1) {
                Value::Nil => None,
                Value::Str(value) => Some(value.to_string()),
                _ => {
                    return Err(crate::bind_world_object::host_error(unsupported(
                        "reading nullable String argument conversion",
                    )))
                }
            };
            let result: LuaResult<Value> = match name {
                "isLiteratureRead" => {
                    let host = Host {
                        thread: RefCell::new(&mut *call.thread),
                        lua_fault: RefCell::new(None),
                    };
                    let result = reading::is_literature_read(&character, title.as_deref(), &host);
                    match host.lua_fault.into_inner() {
                        Some(error) => Err(error),
                        None => result
                            .map(Value::from)
                            .map_err(crate::bind_world_object::host_error),
                    }
                }
                "isPrintMediaRead" => reading::is_print_media_read(&character, title.as_deref())
                    .map(Value::from)
                    .map_err(crate::bind_world_object::host_error),
                _ => reading::already_read_pages(&character, title.as_deref())
                    .map(Value::from)
                    .map_err(crate::bind_world_object::host_error),
            };
            Ok(call.push(result?))
        });
    }
}

#[cfg(test)]
mod tests;
