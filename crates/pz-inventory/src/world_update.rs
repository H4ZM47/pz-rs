//! Native virtual world-update predicates over actual inventory state.
//!
//! World reads use a fallible, callback-free context so inventory never owns
//! a second world body or depends on the world crate. Each read occurs only
//! at its native branch. A missing required JVM receiver is an explicit
//! diagnostic capability boundary, rather than a fabricated false result.

use pz_entity::runtime_load::{HostResult, RuntimeHostError};
use pz_entity::{ComponentType, EntityOwner, RetiredComponents};

use crate::shared::ItemRef;
use crate::{InventoryItem, ItemClass, ItemKind};

/// Actual role/sandbox and complete-world-allocation reads. Implementations
/// must be side-effect-free and release each world/square guard on return.
/// None from world_square_outside means an actual null square, not an absent
/// service, held body, wrong erased type, unknown profile, or invalid locator.
pub trait WorldItemUpdateContext {
    fn is_server(&self) -> HostResult<bool>;
    fn is_client(&self) -> HostResult<bool>;
    fn world_object_index(&self, owner: &EntityOwner) -> HostResult<i32>;
    fn world_square_outside(&self, owner: &EntityOwner) -> HostResult<Option<bool>>;
    fn rotten_food_removal_days(&self) -> HostResult<i32>;
}

fn read<T>(item: &ItemRef, f: impl FnOnce(&InventoryItem) -> HostResult<T>) -> HostResult<T> {
    let body = item
        .try_borrow()
        .map_err(|_| RuntimeHostError::Borrowed("inventory world-update item"))?;
    f(&body)
}

fn unsupported(operation: &'static str) -> RuntimeHostError {
    RuntimeHostError::Unsupported {
        operation,
        kind: None,
    }
}

/// Own the real field only for the native world-reference phases. A queued
/// extra pin keeps even an outer caller's last graph owner alive until its
/// guards end; context reads never execute with an inventory guard held.
fn with_world<T>(
    item: &ItemRef,
    f: impl FnOnce(Option<&EntityOwner>) -> HostResult<T>,
) -> HostResult<T> {
    let owner = crate::world_item::get_world_item_ref(item)?;
    let mut retired = RetiredComponents::new();
    if let Some(owner) = &owner {
        retired.push_owner(owner.clone());
    }
    let result = f(owner.as_ref());
    crate::component_host::retire_components(retired);
    result
}

impl InventoryItem {
    pub fn get_item_heat(&self) -> f32 {
        self.item_heat
    }

    /// Native ordered comparisons preserve NaN payloads and negative zero.
    pub fn set_item_heat(&mut self, mut heat: f32) {
        if heat > 3.0 {
            heat = 3.0;
        }
        if heat < 0.0 {
            heat = 0.0;
        }
        self.item_heat = heat;
    }
}

/// Virtual dispatch matters: Food, the two alarms, AnimalInventoryItem and
/// DrainableComboItem all override the base implementation in the installed
/// class set. These branches must not accidentally invoke base world reads.
pub fn should_update_in_world_ref(
    item: &ItemRef,
    cx: &dyn WorldItemUpdateContext,
) -> HostResult<bool> {
    match read(item, |body| Ok(body.template.class))? {
        ItemClass::AlarmClock | ItemClass::AlarmClockClothing => {
            read(item, |body| match &body.kind {
                ItemKind::AlarmClock(alarm) | ItemKind::AlarmClockClothing(_, alarm) => {
                    Ok(alarm.alarm_set)
                }
                _ => Err(RuntimeHostError::InvalidHostState(
                    "alarm native class/body mismatch",
                )),
            })
        }
        ItemClass::Animal => Ok(true),
        ItemClass::Food => food_world(item, cx),
        ItemClass::Drainable => {
            if !cx.is_server()?
                && read(item, |body| {
                    body.drainable_heat
                        .ok_or(RuntimeHostError::InvalidHostState(
                            "DrainableComboItem missing actual heat",
                        ))
                })? != 1.0
            {
                return Ok(true);
            }
            rain(item, cx, true)
        }
        _ => {
            if !cx.is_server()? && read(item, |body| Ok(body.item_heat))? != 1.0 {
                return Ok(true);
            }
            if !cx.is_client()? {
                let fluid = read(item, |body| {
                    body.components
                        .get_ref(ComponentType::FluidContainer)
                        .map(|component| component.is_some())
                        .map_err(RuntimeHostError::Storage)
                })?;
                if fluid {
                    return with_world(item, |owner| {
                        let owner = owner.ok_or_else(|| {
                            unsupported(
                                "InventoryItem.shouldUpdateInWorld null world-item diagnostic",
                            )
                        })?;
                        // This branch checks the square for null before isOutside.
                        Ok(cx.world_square_outside(owner)?.unwrap_or(false))
                    });
                }
            }
            rain(item, cx, false)
        }
    }
}

fn food_world(item: &ItemRef, cx: &dyn WorldItemUpdateContext) -> HostResult<bool> {
    if !cx.is_client()? {
        let (age_max, replacement) = read(item, |body| match &body.kind {
            ItemKind::Food(food) => Ok((
                food.off_age_max,
                food.replace_on_rotten
                    .as_ref()
                    .is_some_and(|name| !name.is_empty()),
            )),
            _ => Err(RuntimeHostError::InvalidHostState(
                "Food native class/body mismatch",
            )),
        })?;
        // Native compares the int converted to double with 1_000_000_000d.
        if age_max != 1_000_000_000 {
            if replacement || cx.rotten_food_removal_days()? != -1 {
                return Ok(true);
            }
        }
    }
    if read(item, |body| match &body.kind {
        ItemKind::Food(food) => Ok(food.heat),
        _ => Err(RuntimeHostError::InvalidHostState(
            "Food native class/body mismatch",
        )),
    })? != 1.0
    {
        return Ok(true);
    }
    if !cx.is_client()? {
        return read(item, |body| match &body.kind {
            ItemKind::Food(food) => Ok(food.fertilized),
            _ => Err(RuntimeHostError::InvalidHostState(
                "Food native class/body mismatch",
            )),
        });
    }
    Ok(false)
}

/// InventoryItem.shouldUpdateInRain is inherited by every represented class.
/// Drainable's world-update override instead calls canBecomeWetInRain, whose
/// final dry-towel keys differ; its front-end world checks are otherwise equal.
pub fn should_update_in_rain_ref(
    item: &ItemRef,
    cx: &dyn WorldItemUpdateContext,
) -> HostResult<bool> {
    rain(item, cx, false)
}

fn rain(item: &ItemRef, cx: &dyn WorldItemUpdateContext, drainable: bool) -> HostResult<bool> {
    if cx.is_client()? {
        return Ok(false);
    }
    with_world(item, |owner| {
        let Some(owner) = owner else {
            return Ok(false);
        };
        if cx.world_object_index(owner)? == -1 {
            return Ok(false);
        }
        let outside = cx
            .world_square_outside(owner)?
            .ok_or_else(|| unsupported("InventoryItem rain predicate null square diagnostic"))?;
        if !outside {
            return Ok(false);
        }
        read(item, |body| {
            let full_type = body.get_full_type();
            if drainable {
                Ok(matches!(
                    full_type.as_str(),
                    "Base.BathTowel" | "Base.DishCloth"
                ))
            } else {
                let untainted_food = matches!(&body.kind, ItemKind::Food(food) if !food.tainted);
                // Native still evaluates is(ItemKey[]) even when Food is true.
                let wet_type = matches!(
                    full_type.as_str(),
                    "Base.BathTowelWet" | "Base.DishClothWet"
                );
                Ok(untainted_food || wet_type)
            }
        })
    })
}

#[cfg(test)]
mod tests;
