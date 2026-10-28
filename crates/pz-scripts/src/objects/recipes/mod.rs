//! Script classes of the `recipes` group.

mod evolved_recipe;
mod fixing;
mod item_recipe;
mod recipe;
mod unique_recipe;

use std::cell::RefCell;
use std::rc::Rc;

pub use evolved_recipe::EvolvedRecipe;
pub use fixing::{Fixer, FixerSkill, Fixing};
pub use item_recipe::ItemRecipe;
pub use recipe::{LuaCall, Recipe, RecipeResult, RequiredSkill, Source};
pub use unique_recipe::UniqueRecipe;

use crate::base::{ScriptError, ScriptRef, ScriptResult};
use crate::java;
use crate::manager::ScriptManager;
use crate::module::ScriptModule;
use crate::script_type::ScriptType;

/// `createInstance` for the script types this group ports; `None` for
/// types it does not (yet) handle.
pub fn create_instance(type_: ScriptType, _module: &Rc<ScriptModule>, name: &str, _token: &str) -> Option<ScriptRef> {
    match type_ {
        ScriptType::Recipe => Some(ScriptRef::new(Recipe::new())),
        ScriptType::UniqueRecipe => Some(ScriptRef::new(UniqueRecipe::new(name))),
        ScriptType::EvolvedRecipe => Some(ScriptRef::new(EvolvedRecipe::new(name))),
        ScriptType::Fixing => Some(ScriptRef::new(Fixing::new())),
        _ => None,
    }
}

/// Java static fields of this group's classes (reset by `ScriptManager.Reset`).
/// The only statics (`Fixing`'s predicate scratch, `EvolvedRecipe`'s
/// `DecimalFormat`) are runtime helpers, not script state.
#[derive(Default)]
pub struct Statics {}

/// Group steps run at the end of `ScriptManager.Load`.
pub fn post_load(_sm: &ScriptManager) {}

/// `ScriptManager.getRecipe`.
pub fn get_recipe(sm: &ScriptManager, name: &str) -> Option<Rc<RefCell<Recipe>>> {
    sm.get(ScriptType::Recipe, name)
}

/// Actual recipe lookup preserving borrowed-source errors separately from
/// genuine absence. No recipe body borrow is needed to return its identity.
pub fn try_get_recipe(sm: &ScriptManager, name: &str) -> Result<Option<Rc<RefCell<Recipe>>>, crate::native_item_keys::ScriptAccessError> {
    match sm.try_get_script(ScriptType::Recipe, name)? {
        None => Ok(None),
        Some(script) => script.downcast::<Recipe>().map(Some).ok_or(crate::native_item_keys::ScriptAccessError::Unsupported("wrong actual Recipe class")),
    }
}

/// `ScriptManager.getAllRecipes`.
pub fn get_all_recipes(sm: &ScriptManager) -> Vec<Rc<RefCell<Recipe>>> {
    sm.get_all(ScriptType::Recipe)
}

/// `ScriptManager.getUniqueRecipe`.
pub fn get_unique_recipe(sm: &ScriptManager, name: &str) -> Option<Rc<RefCell<UniqueRecipe>>> {
    sm.get(ScriptType::UniqueRecipe, name)
}

/// `ScriptManager.getAllUniqueRecipes`.
pub fn get_all_unique_recipes(sm: &ScriptManager) -> Vec<Rc<RefCell<UniqueRecipe>>> {
    sm.get_all(ScriptType::UniqueRecipe)
}

/// `ScriptManager.getEvolvedRecipe`.
pub fn get_evolved_recipe(sm: &ScriptManager, name: &str) -> Option<Rc<RefCell<EvolvedRecipe>>> {
    sm.get(ScriptType::EvolvedRecipe, name)
}

/// `ScriptManager.getAllEvolvedRecipesList` / `getAllEvolvedRecipes`.
pub fn get_all_evolved_recipes(sm: &ScriptManager) -> Vec<Rc<RefCell<EvolvedRecipe>>> {
    sm.get_all(ScriptType::EvolvedRecipe)
}

/// `ScriptManager.getFixing`.
pub fn get_fixing(sm: &ScriptManager, name: &str) -> Option<Rc<RefCell<Fixing>>> {
    sm.get(ScriptType::Fixing, name)
}

/// `ScriptManager.getAllFixing`.
pub fn get_all_fixing(sm: &ScriptManager) -> Vec<Rc<RefCell<Fixing>>> {
    sm.get_all(ScriptType::Fixing)
}

/// `token.split("[{}]")[1].split(",")`: the comma-separated values of a
/// script's first brace segment, as the line-based loaders read them.
fn split_body(token: &str) -> ScriptResult<Vec<&str>> {
    let waypoint = java::split(token, |c| c == '{' || c == '}', false);
    Ok(java::split_char(at(&waypoint, 1)?, ','))
}

/// `arr[i]` on a `String.split` result.
fn at<'a>(arr: &[&'a str], i: usize) -> ScriptResult<&'a str> {
    arr.get(i).copied().ok_or_else(|| {
        ScriptError::new(format!(
            "java.lang.ArrayIndexOutOfBoundsException: Index {i} out of bounds for length {}",
            arr.len()
        ))
    })
}

/// `Float.parseFloat` with the original's exception text.
fn parse_float(s: &str) -> ScriptResult<f32> {
    java::parse_f32(s).ok_or_else(|| {
        let t = java::trim(s);
        ScriptError::new(if t.is_empty() {
            "java.lang.NumberFormatException: empty String".to_string()
        } else {
            format!("java.lang.NumberFormatException: For input string: \"{t}\"")
        })
    })
}

/// `Integer.parseInt` with the original's exception text.
fn parse_int(s: &str) -> ScriptResult<i32> {
    java::parse_i32(s)
        .ok_or_else(|| ScriptError::new(format!("java.lang.NumberFormatException: For input string: \"{s}\"")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn number_errors_match_java() {
        assert_eq!(parse_float("  ").unwrap_err().0, "java.lang.NumberFormatException: empty String");
        assert_eq!(parse_float(" 1.5f ").unwrap(), 1.5);
        assert_eq!(parse_int("").unwrap_err().0, "java.lang.NumberFormatException: For input string: \"\"");
        assert_eq!(parse_int("-7").unwrap(), -7);
    }

    #[test]
    fn create_instance_covers_group_types() {
        let m = ScriptModule::new();
        for t in [ScriptType::Recipe, ScriptType::UniqueRecipe, ScriptType::EvolvedRecipe, ScriptType::Fixing] {
            assert!(create_instance(t, &m, "x", "").is_some());
        }
        assert!(create_instance(ScriptType::Item, &m, "x", "").is_none());
    }
}
