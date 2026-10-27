//! IsoGameCharacter.isKnownPoison's two genuinely different overloads.
//! All owning arguments are pinned before fallible access; host callbacks run
//! between short actual-body reads and preserve the native short-circuit order.
use std::cell::RefCell;
use std::rc::Rc;

use super::GameCharacter;
use crate::PerkRef;
use pz_entity::runtime_load::{HostResult, RuntimeHostError};
use pz_entity::RetiredComponents;
use pz_inventory::shared::ItemRef;
use pz_inventory::template::ScriptItemRef;
use pz_inventory::ItemKind;
use pz_kahlua::Value;
use pz_scripts::objects::recipes::Recipe;
use pz_scripts::registry::RegistryKey;

/// `is_debug`, `is_client`, and `is_server` MUST be callback-free reads of
/// independent actual static fields. They may not reenter/mutate a character.
/// The current inline, non-null PlayerCheats representation retains a pure
/// character field lease across ONLY those three reads, not a copied set or
/// fabricated PlayerCheats Rc. Arbitrary reentrant providers, nullable/public
/// cheats-reference replacement and retained CheatRef APIs need a real owner
/// representation and remain unsupported. Sandbox may run Lua only after the
/// field lease has ended.
pub trait KnownPoisonHost {
    fn item_tag(&self, field: &'static str) -> HostResult<Rc<RegistryKey>>;
    fn food_item_type(&self) -> HostResult<Rc<RegistryKey>>;
    fn recipe(&self, name: &str) -> HostResult<Option<Rc<RefCell<Recipe>>>>;
    fn is_debug(&self) -> HostResult<bool>;
    fn is_client(&self) -> HostResult<bool>;
    fn is_server(&self) -> HostResult<bool>;
    fn see_not_learnt_recipe(&self) -> HostResult<bool>;
    fn cooking_perk(&self) -> HostResult<PerkRef>;
}
struct Pins(RetiredComponents);
impl Pins {
    fn new() -> Self {
        Self(RetiredComponents::new())
    }
    fn keep<T: Clone + 'static>(&mut self, value: &T) {
        self.0.push_payload(Box::new(value.clone()));
    }
}
impl Drop for Pins {
    fn drop(&mut self) {
        pz_inventory::component_host::retire_components(std::mem::take(&mut self.0));
    }
}
fn borrowed(phase: &'static str) -> RuntimeHostError {
    RuntimeHostError::Borrowed(phase)
}

fn know_all<I: 'static>(
    character: &Rc<RefCell<GameCharacter<I>>>,
    host: &dyn KnownPoisonHost,
) -> HostResult<bool> {
    // Native getCheats precedes PlayerCheats.isSet/isCheatAllowed. Keep the
    // actual field lease through pure globals so even a replaced public
    // reference cannot silently retarget within the admitted host contract.
    let character = character
        .try_borrow()
        .map_err(|_| borrowed("character cheats field"))?;
    let cheats = &character.cheats;
    if !(host.is_debug()? || host.is_client()? || host.is_server()?) {
        return Ok(false);
    }
    Ok(cheats.names().iter().any(|name| name == "KNOW_ALL_RECIPES"))
}
fn recipe_actually_known<I: 'static>(
    character: &Rc<RefCell<GameCharacter<I>>>,
    name: &str,
    host: &dyn KnownPoisonHost,
    pins: &mut Pins,
) -> HostResult<bool> {
    // Even forced knowledge queries resolve the actual legacy recipe first.
    let recipe = host.recipe(name)?;
    if let Some(recipe) = recipe {
        pins.keep(&recipe);
        if know_all(character, host)? {
            return Ok(true);
        }
        // The forced flag is not forwarded to the found-Recipe overload.
        if host.see_not_learnt_recipe()? {
            return Ok(true);
        }
        // Native obtains the actual knownRecipes field at PC22 before
        // Recipe.getOriginalname26. This is the same inline list, no snapshot;
        // only the pure recipe getter runs while its field lease is held.
        let character = character
            .try_borrow()
            .map_err(|_| borrowed("character known recipes"))?;
        let original = recipe
            .try_borrow()
            .map_err(|_| borrowed("recipe originalname"))?
            .originalname()
            .map(str::to_owned);
        return Ok(
            original.is_some_and(|name| character.known_recipes.iter().any(|known| known == &name))
        );
    }
    if know_all(character, host)? {
        return Ok(true);
    }
    Ok(character
        .try_borrow()
        .map_err(|_| borrowed("character known recipes"))?
        .known_recipes
        .iter()
        .any(|known| known == name))
}
fn cooking_level<I: 'static>(
    character: &Rc<RefCell<GameCharacter<I>>>,
    host: &dyn KnownPoisonHost,
    pins: &mut Pins,
) -> HostResult<i32> {
    let perk = host.cooking_perk()?;
    pins.keep(&perk);
    // getPerkInfo compares the actual static Perk pointer; its body is unused.
    Ok(character
        .try_borrow()
        .map_err(|_| borrowed("character Cooking level"))?
        .perk_level(Some(&perk)))
}
fn full_name<I: 'static>(
    character: &Rc<RefCell<GameCharacter<I>>>,
    pins: &mut Pins,
) -> HostResult<String> {
    let descriptor = {
        let character = character
            .try_borrow()
            .map_err(|_| borrowed("character full name"))?;
        if character.kind == crate::CharacterKind::Animal {
            return Err(RuntimeHostError::Unsupported {
                operation: "IsoAnimal.getFullName actual animal data/translation",
                kind: None,
            });
        }
        character.descriptor.clone()
    };
    let Some(descriptor) = descriptor else {
        return Ok("Bob Smith".to_owned());
    };
    pins.keep(&descriptor);
    let name = descriptor
        .try_borrow()
        .map_err(|_| borrowed("character descriptor full name"))?
        .full_name();
    Ok(name)
}

pub fn is_known_poison_item<I: 'static>(
    character: &Rc<RefCell<GameCharacter<I>>>,
    item: &ItemRef,
    host: &dyn KnownPoisonHost,
) -> HostResult<bool> {
    let mut pins = Pins::new();
    pins.keep(character);
    pins.keep(item);
    let no_detect = host.item_tag("NO_DETECT")?;
    if pz_inventory::item_texture::has_tag_ref(item, Some(&no_detect))? {
        return Ok(false);
    }
    let show = host.item_tag("SHOW_POISON")?;
    if pz_inventory::item_texture::has_tag_ref(item, Some(&show))? {
        return Ok(true);
    }
    {
        let item = item
            .try_borrow()
            .map_err(|_| borrowed("known poison Food power"))?;
        let ItemKind::Food(food) = &item.kind else {
            return Ok(false);
        };
        if food.poison_power <= 0 {
            return Ok(false);
        }
    }
    let herbalist = {
        let item = item
            .try_borrow()
            .map_err(|_| borrowed("known poison herbalist type"))?;
        let ItemKind::Food(food) = &item.kind else {
            return Err(RuntimeHostError::InvalidHostState("Food changed class"));
        };
        food.herbalist_type.clone()
    };
    if herbalist.as_deref().is_some_and(|value| !value.is_empty()) {
        return recipe_actually_known(character, "Herbalist", host, &mut pins);
    }
    let detection = || -> HostResult<i32> {
        let item = item
            .try_borrow()
            .map_err(|_| borrowed("known poison detection level"))?;
        let ItemKind::Food(food) = &item.kind else {
            return Err(RuntimeHostError::InvalidHostState("Food changed class"));
        };
        Ok(food.poison_detection_level)
    };
    if detection()? >= 0 {
        let level = cooking_level(character, host, &mut pins)?;
        if level >= 10_i32.wrapping_sub(detection()?) {
            return Ok(true);
        }
    }
    // Java evaluates String receiver BEFORE item.getModData().rawget().
    let name = full_name(character, &mut pins)?;
    let table = pz_inventory::moddata::get_mod_data_ref(item)
        .map_err(pz_inventory::ModDataError::into_host)?;
    pins.keep(&table);
    let value = table
        .try_rawget(&Value::str("addedPoisonBy"))
        .map_err(|error| match error {
            pz_kahlua::table::TableAccessError::Borrowed => {
                borrowed("known poison addedPoisonBy table")
            }
            pz_kahlua::table::TableAccessError::TraversalLimit => RuntimeHostError::Unsupported {
                operation: "known poison metatable traversal boundary",
                kind: None,
            },
        })?;
    pins.keep(&value);
    Ok(matches!(value, Value::Str(value) if value.as_ref() == name))
}

pub fn is_known_poison_script<I: 'static>(
    character: &Rc<RefCell<GameCharacter<I>>>,
    item: &ScriptItemRef,
    host: &dyn KnownPoisonHost,
) -> HostResult<bool> {
    let mut pins = Pins::new();
    pins.keep(character);
    pins.keep(item);
    let show = host.item_tag("SHOW_POISON")?;
    if item
        .0
        .try_borrow()
        .map_err(|_| borrowed("script known poison SHOW_POISON"))?
        .has_tag(Some(&show))
    {
        return Ok(true);
    }
    let food = host.food_item_type()?;
    {
        let item = item
            .0
            .try_borrow()
            .map_err(|_| borrowed("script known poison type/power"))?;
        if !item.is_item_type_key(Some(&food)) || (item.poison_power as f32) <= 0.0 {
            return Ok(false);
        }
    }
    let herbalist = item
        .0
        .try_borrow()
        .map_err(|_| borrowed("script known poison herbalist"))?
        .herbalist_type
        .clone();
    if herbalist.as_deref().is_some_and(|value| !value.is_empty()) {
        return recipe_actually_known(character, "Herbalist", host, &mut pins);
    }
    let detection = || -> HostResult<i32> {
        Ok(item
            .0
            .try_borrow()
            .map_err(|_| borrowed("script known poison detection"))?
            .poison_detection_level)
    };
    if detection()? >= 0 {
        let level = cooking_level(character, host, &mut pins)?;
        if level >= 10_i32.wrapping_sub(detection()?) {
            return Ok(true);
        }
    }
    // Unlike the actual-item overload this returns detection>=0 even when
    // Cooking was too low. There is no runtime item/ModData/full-name call.
    Ok(detection()? >= 0)
}

#[cfg(test)]
mod tests;
