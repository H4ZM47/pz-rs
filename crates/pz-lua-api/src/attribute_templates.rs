//! Early native AttributesScript support. Script loading runs before Lua
//! registration, so it needs the same actual type registry at this phase.

use std::cell::RefCell;
use std::rc::Rc;

use pz_entity::runtime_fluid::RuntimeNativeNotice;
use pz_entity::runtime_load::{HostResult, RuntimeHostError};
use pz_entity::{AttributeTypes, BasicContext, RetiredComponents};
use pz_scripts::objects::entity::components::attributes::{
    install_attribute_template_creator, AttributesScript,
};

thread_local! {
    // Registry capability only. The actual template field stays on its
    // AttributesScript; this is never another template lookup/store.
    static CONTEXT: RefCell<Option<Rc<BasicContext>>> = const { RefCell::new(None) };
    static NOTICES: RefCell<Vec<RuntimeNativeNotice>> = const { RefCell::new(Vec::new()) };
    // ScriptManager can catch ScriptError when debug is false. Rust host
    // capability/borrow faults must still stop bootstrap at the outer phase.
    static FAULTS: RefCell<Vec<RuntimeHostError>> = const { RefCell::new(Vec::new()) };
}

/// Install before ScriptManager.Load, including OnScriptsLoaded. The caller
/// must use these SAME type allocations in the later complete item context.
pub fn install(types: AttributeTypes) {
    CONTEXT.with(|slot| {
        *slot.borrow_mut() = Some(Rc::new(BasicContext {
            attributes: types,
            ..Default::default()
        }));
    });
    crate::component_gc::install_observers();
    pz_inventory::component_host::install_component_debug_query(Some(crate::component_debug_query));
    pz_inventory::component_host::install_native_notice_logger(Some(
        crate::component_native_notice,
    ));
    install_attribute_template_creator(Some(create_template));
}

fn create_template(script: &AttributesScript) -> pz_scripts::ScriptResult<()> {
    let context = CONTEXT.with(|slot| slot.borrow().clone()).ok_or_else(|| {
        script_error(RuntimeHostError::Unsupported {
            operation: "actual attribute type registry",
            kind: Some(pz_entity::ComponentType::Attributes),
        })
    })?;
    let mut notices = Vec::new();
    let mut retired = RetiredComponents::new();
    let result = pz_entity::runtime_attributes::get_or_create_attributes_template(
        script,
        context.as_ref(),
        &crate::component_debug_query,
        &mut notices,
        &mut retired,
    );
    // The script loader still owns its parent definition guard. Keep native
    // notices and possibly final old owners until the enclosing host ends it.
    NOTICES.with(|queue| queue.borrow_mut().extend(notices));
    pz_inventory::component_host::retire_components(retired);
    // Register even a partially created actual template retained after error.
    // Metadata registration cannot invoke Lua or run collection.
    let metadata = register_template(script);
    let result = result.map_err(script_error);
    let metadata = metadata.map_err(script_error);
    result?;
    metadata
}

fn script_error(error: RuntimeHostError) -> pz_scripts::ScriptError {
    let message = format!("attribute template: {error:?}");
    if !matches!(error, RuntimeHostError::NativeException(_)) {
        FAULTS.with(|queue| queue.borrow_mut().push(error));
    }
    pz_scripts::ScriptError::new(message)
}

fn register_template(script: &AttributesScript) -> HostResult<()> {
    let template = script
        .template_slot()
        .map_err(|_| RuntimeHostError::Borrowed("AttributesScript template metadata"))?
        .as_ref()
        .cloned();
    if let Some(template) = template {
        let component = template
            .downcast::<RefCell<pz_entity::RuntimeComponent>>()
            .map_err(|_| {
                RuntimeHostError::InvalidHostState(
                    "AttributesScript cache has a foreign allocation",
                )
            })?;
        crate::component_gc::register_component(&component).map_err(|_| {
            RuntimeHostError::InvalidHostState("attribute template metadata conflict")
        })?;
    }
    Ok(())
}

/// Call on both successful and failed ScriptManager.Load after it releases
/// every parent definition guard. Never lose a completed diagnostic prefix.
pub fn flush_notices() -> HostResult<()> {
    let notices = NOTICES.with(|queue| std::mem::take(&mut *queue.borrow_mut()));
    let faults = FAULTS.with(|queue| std::mem::take(&mut *queue.borrow_mut()));
    pz_inventory::component_host::emit_native_notices(&notices)?;
    if let Some(error) = faults.into_iter().next() {
        return Err(error);
    }
    Ok(())
}

#[cfg(test)]
mod tests;
