//! Script object types (`zombie.scripting.objects.*` and the other classes
//! `ScriptModule` instantiates). Types not yet ported load as
//! [`GenericScript`], which keeps the parsed body for later consumers.

pub mod definitions;
pub mod entity;
pub mod items;
mod generic;
pub mod models;
pub mod recipes;
mod string_list;
pub mod vehicles;

use std::rc::Rc;

pub use generic::GenericScript;
pub use string_list::StringListScript;

use crate::base::ScriptRef;
use crate::manager::ScriptManager;
use crate::module::ScriptModule;
use crate::script_type::ScriptType;

/// State the original keeps in static fields of script classes.
#[derive(Default)]
pub struct Statics {
    pub recipes: std::cell::RefCell<recipes::Statics>,
    pub models: std::cell::RefCell<models::Statics>,
    pub vehicles: std::cell::RefCell<vehicles::Statics>,
    pub definitions: std::cell::RefCell<definitions::Statics>,
    pub entity: std::cell::RefCell<entity::Statics>,
    pub items: std::cell::RefCell<items::Statics>,
}

/// `ScriptBucket.createInstance` for each module bucket.
pub fn create_instance(type_: ScriptType, module: &Rc<ScriptModule>, name: &str, token: &str) -> ScriptRef {
    let ported = recipes::create_instance(type_, module, name, token)
        .or_else(|| models::create_instance(type_, module, name, token))
        .or_else(|| vehicles::create_instance(type_, module, name, token))
        .or_else(|| definitions::create_instance(type_, module, name, token))
        .or_else(|| entity::create_instance(type_, module, name, token))
        .or_else(|| items::create_instance(type_, module, name, token));
    if let Some(s) = ported {
        return s;
    }
    match type_ {
        ScriptType::StringList => ScriptRef::new(StringListScript::new()),
        _ => ScriptRef::new(GenericScript::new(type_)),
    }
}

/// The type-specific steps at the end of `ScriptManager.Load` that depend on
/// ported script classes (`debugItems`, `resolveItemTypes`,
/// `createClothingItemMap`, `createZedDmgMap`).
pub fn post_load(sm: &ScriptManager) {
    recipes::post_load(sm);
    models::post_load(sm);
    vehicles::post_load(sm);
    definitions::post_load(sm);
    entity::post_load(sm);
    items::post_load(sm);
}

/// Static state cleared by `ScriptManager.Reset`.
pub fn reset_statics(sm: &ScriptManager) {
    *sm.statics.recipes.borrow_mut() = Default::default();
    // ScriptManager.Reset leaves the Ragdoll lists and ModelScript.reported alone.
    *sm.statics.vehicles.borrow_mut() = Default::default();
    *sm.statics.definitions.borrow_mut() = Default::default();
    *sm.statics.entity.borrow_mut() = Default::default();
    {
        // Built-in constants and Item.idMax survive ScriptManager.Reset.
        let mut items = sm.statics.items.borrow_mut();
        let constants = std::mem::take(&mut items.constants);
        let id_max = items.id_max;
        *items = Default::default();
        items.constants = constants;
        items.id_max = id_max;
    }
}
