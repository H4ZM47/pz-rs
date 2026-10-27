//! Actual Literature fields and the null-only learned-recipes capability.
//!
//! Native evidence: runtime-native-literature-fields-producers-load-save-current-jar-
//! 20261027.json (installed JAR e1a69eb7). Runtime trained fields are instance
//! storage, never a view of a mutable script/template. Java list ownership is
//! not implemented here; its first required allocation/return stops explicitly.

use std::rc::Rc;

use pz_entity::runtime_load::{HostResult, RuntimeHostError};
use pz_entity::RetiredComponents;
use pz_kahlua::Value;

use crate::item::{ItemKind, LiteratureData};
use crate::shared::ItemRef;
use crate::{ItemClass, ItemTemplate, ModDataError};

fn unsupported(operation: &'static str) -> RuntimeHostError {
    RuntimeHostError::Unsupported {
        operation,
        kind: None,
    }
}

fn read<T>(item: &ItemRef, f: impl FnOnce(&LiteratureData) -> T) -> HostResult<T> {
    let body = item
        .try_borrow()
        .map_err(|_| RuntimeHostError::Borrowed("Literature field"))?;
    let ItemKind::Literature(literature) = &body.kind else {
        return Err(unsupported("Literature getter receiver class"));
    };
    Ok(f(literature))
}

pub fn get_skill_trained_ref(item: &ItemRef) -> HostResult<Option<String>> {
    read(item, |v| v.skill_trained.clone())
}
pub fn get_lvl_skill_trained_ref(item: &ItemRef) -> HostResult<i32> {
    read(item, |v| v.lvl_skill_trained)
}
/// Internal naming alias; the native exported method is getLvlSkillTrained.
pub fn get_min_level_trained_ref(item: &ItemRef) -> HostResult<i32> {
    get_lvl_skill_trained_ref(item)
}
pub fn get_num_levels_trained_ref(item: &ItemRef) -> HostResult<i32> {
    read(item, |v| v.num_levels_trained)
}
pub fn get_max_level_trained_ref(item: &ItemRef) -> HostResult<i32> {
    read(item, |v| {
        v.lvl_skill_trained
            .wrapping_add(v.num_levels_trained)
            .wrapping_sub(1)
    })
}
pub fn get_number_of_pages_ref(item: &ItemRef) -> HostResult<i32> {
    read(item, |v| v.number_of_pages)
}

/// The only completed native result this bounded implementation can expose.
/// A nonnull native List must never be substituted with a detached Vec or nil.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LearnedRecipesResult {
    Null,
}

pub fn get_learned_recipes_ref(item: &ItemRef) -> HostResult<LearnedRecipesResult> {
    let mut retired = RetiredComponents::new();
    retired.push_payload(Box::new(Rc::clone(item)));
    let result = (|| {
        // Native PC1 calls the lazy getter even when an override already exists.
        let table = crate::moddata::get_mod_data_ref(item).map_err(ModDataError::into_host)?;
        retired.push_payload(Box::new(table.clone()));
        let value = table
            .try_rawget(&Value::str("learnedRecipe"))
            .map_err(ModDataError::from)
            .map_err(ModDataError::into_host)?;
        let present = !matches!(value, Value::Nil);
        retired.push_payload(Box::new(value));
        if present {
            // PC17 allocates ArrayList BEFORE rereading/casting the entry,
            // publishing the list, or removing the ModData key. Keep that prefix.
            return Err(unsupported(
                "Literature.getLearnedRecipes ArrayList allocation",
            ));
        }
        if read(item, |v| v.learned_recipes.is_some())? {
            return Err(unsupported(
                "Literature.getLearnedRecipes actual override List return",
            ));
        }
        let script = crate::item_texture::get_script_item_ref(item)?
            .ok_or_else(|| unsupported("Literature.getLearnedRecipes null scriptItem receiver"))?;
        retired.push_payload(Box::new(script.clone()));
        let script = script
            .0
            .try_borrow()
            .map_err(|_| RuntimeHostError::Borrowed("Literature.getLearnedRecipes script"))?;
        if script.learned_recipes.is_some() {
            return Err(unsupported(
                "Literature.getLearnedRecipes actual script List return",
            ));
        }
        Ok(LearnedRecipesResult::Null)
    })();
    crate::component_host::retire_components(retired);
    result
}

/// Allocation defaults precede InventoryItem's superclass texture work.
pub(crate) fn initialize_java_fields(item: &ItemRef) -> HostResult<()> {
    let mut body = item
        .try_borrow_mut()
        .map_err(|_| RuntimeHostError::Borrowed("Literature allocation fields"))?;
    initialize_java_fields_value(&mut body);
    Ok(())
}

/// Initialize before the actual Rc is published to its allocation observer.
pub(crate) fn initialize_java_fields_value(body: &mut crate::InventoryItem) {
    if let ItemKind::Literature(v) = &mut body.kind {
        v.number_of_pages = 0;
        v.lvl_skill_trained = 0;
        v.num_levels_trained = 0;
        v.skill_trained = None;
        v.already_read_pages = 0;
        v.can_be_write = false;
        v.page_to_write = 0;
        v.custom_pages = None;
        v.locked_by = None;
        v.learned_recipes = None;
    }
}

/// Both native Literature constructors execute these initializers after super.
pub(crate) fn finish_super_fields(item: &ItemRef) -> HostResult<()> {
    let mut body = item
        .try_borrow_mut()
        .map_err(|_| RuntimeHostError::Borrowed("Literature constructor fields"))?;
    if let ItemKind::Literature(v) = &mut body.kind {
        v.number_of_pages = -1;
        v.lvl_skill_trained = -1;
        v.skill_trained = Some("None".to_owned());
    }
    Ok(())
}

/// Native InstanceItem PC907..961 reads its CURRENT argument separately for
/// every setter. With no script, this is an explicit captured data profile.
pub(crate) fn copy_instance_fields(item: &ItemRef, template: &ItemTemplate) -> HostResult<()> {
    if template.class != ItemClass::Literature {
        return Ok(());
    }
    macro_rules! copy_field {
        ($field:ident) => {{
            let value = if let Some(script) = &template.script {
                script
                    .0
                    .try_borrow()
                    .map_err(|_| {
                        RuntimeHostError::Borrowed(concat!(
                            "InstanceItem Literature script ",
                            stringify!($field)
                        ))
                    })?
                    .$field
                    .clone()
            } else {
                template.$field.clone()
            };
            let mut body = item.try_borrow_mut().map_err(|_| {
                RuntimeHostError::Borrowed(concat!(
                    "InstanceItem Literature receiver ",
                    stringify!($field)
                ))
            })?;
            let ItemKind::Literature(v) = &mut body.kind else {
                return Err(RuntimeHostError::InvalidHostState(
                    "Literature receiver class",
                ));
            };
            v.$field = value;
        }};
    }
    copy_field!(number_of_pages);
    {
        let mut body = item
            .try_borrow_mut()
            .map_err(|_| RuntimeHostError::Borrowed("InstanceItem Literature alreadyReadPages"))?;
        let ItemKind::Literature(v) = &mut body.kind else {
            return Err(RuntimeHostError::InvalidHostState(
                "Literature receiver class",
            ));
        };
        v.already_read_pages = 0;
    }
    copy_field!(skill_trained);
    copy_field!(lvl_skill_trained);
    copy_field!(num_levels_trained);
    copy_field!(can_be_write);
    copy_field!(page_to_write);
    Ok(())
}

#[cfg(test)]
mod tests;
