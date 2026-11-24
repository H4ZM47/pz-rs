//! Live views of the current cell's actual shared-item process queues.
//! Retained collection values keep their original backing after cell replacement.

use crate::bind_items::{item_arg_value, item_value};
use crate::bind_java_util::{JavaList, JavaMap};
use pz_inventory::shared::ItemRef;
use pz_kahlua::value::LuaObject;
use pz_kahlua::{LuaError, LuaResult, Value};
use pz_world::process_items::{same_item, ProcessItemEntries};
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::{Rc, Weak};

thread_local! {
    static VIEWS: RefCell<HashMap<(usize, bool), Weak<dyn LuaObject>>> = RefCell::default();
}

pub(crate) fn decode(v: &Value) -> LuaResult<Option<ItemRef>> {
    if v.is_nil() {
        Ok(None)
    } else {
        item_arg_value(v)
            .map(Some)
            .ok_or_else(|| LuaError::Runtime {
                message: format!(
                    "class {} cannot be cast to class zombie.inventory.InventoryItem",
                    pz_kahlua::stdlib::java_class_name(v)
                ),
                class: "java.lang.ClassCastException",
            })
    }
}

/// The item and ArrayList overloads; both original methods accept null.
pub(crate) fn argument_values(v: &Value) -> Vec<Value> {
    if let Value::Object(o) = v {
        if let Some(list) = o.as_any().downcast_ref::<JavaList>() {
            return list.snapshot();
        }
    }
    vec![v.clone()]
}

pub(crate) fn queue_value(entries: &ProcessItemEntries, set: bool) -> Value {
    let key = (Rc::as_ptr(entries) as usize, set);
    VIEWS.with(|views| {
        let mut views = views.borrow_mut();
        if let Some(view) = views.get(&key).and_then(Weak::upgrade) {
            return Value::Object(view);
        }
        let reader = entries.clone();
        let read = move || {
            reader
                .borrow()
                .iter()
                .map(|v| v.as_ref().map_or(Value::Nil, item_value))
                .collect()
        };
        let writer = entries.clone();
        let write = Box::new(move |values: Vec<Value>| {
            let mut next = Vec::new();
            for v in &values {
                let item = decode(v)?;
                if !set || !next.iter().any(|i| same_item(i, &item)) {
                    next.push(item);
                }
            }
            let old = std::mem::replace(&mut *writer.borrow_mut(), next);
            drop(old);
            Ok(())
        });
        let view: Rc<dyn LuaObject> = if set {
            Rc::new(JavaMap::live_set(read, Some(write)).with_transient_snapshots())
        } else {
            Rc::new(JavaList::live(read, Some(write)).with_transient_snapshots())
        };
        views.retain(|_, v| v.strong_count() > 0);
        views.insert(key, Rc::downgrade(&view));
        Value::Object(view)
    })
}

/// `ItemContainer.addItemsToProcessItems`, also used by IngameState.enter.
pub fn add_container_items(container: &pz_inventory::shared::ContainerRef) -> LuaResult<()> {
    let items = container.borrow().items.clone();
    let client = crate::bind_core::shared_core().borrow().is_client;
    crate::bind_world::with_world(|world| -> LuaResult<()> {
        let cell = world
            .cell
            .as_ref()
            .ok_or_else(|| LuaError::runtime("java.lang.NullPointerException"))?;
        cell.process_items.add_all(items, client);
        Ok(())
    })
}

/// Both container branches queue the same live items for cell removal.
/// Only non-floor containers remove their item-sound ownership: the
/// floor inventory is a transient UI proxy over actual world items.
pub fn remove_container_items(container: &pz_inventory::shared::ContainerRef) -> LuaResult<()> {
    let (items, floor) = {
        let container = container.borrow();
        (container.items.clone(), container.kind == "floor")
    };
    crate::bind_world::with_world(|world| -> LuaResult<()> {
        let cell = world
            .cell
            .as_ref()
            .ok_or_else(|| LuaError::runtime("java.lang.NullPointerException"))?;
        cell.process_items.queue_remove_all(items.iter().cloned());
        Ok(())
    })?;
    if !floor {
        crate::item_sound::remove_items(items);
    }
    Ok(())
}

/// Moves `current` toward `target` by `step`, snapping to `target` within one step.
fn step_heat(current: f32, target: f32, step: f32) -> f32 {
    if !current.is_finite() || !target.is_finite() || step <= 0.0 {
        return current;
    }
    let next = if current < target {
        (current + step).min(target)
    } else if current > target {
        (current - step).max(target)
    } else {
        target
    };
    next.clamp(0.02, 3.0)
}

/// `IsoCell.ProcessItems()`: steps real runtime `InventoryItem` updates
/// (Food aging/freezing/heat/cooking via [`crate::bind_item_age`], Drainable
/// and base item heat convergence) and queues items whose `finishupdate()`
/// returns `true` for removal by the trailing `ProcessRemoveItems`.
pub fn process_items(thread: &mut pz_kahlua::Thread, multiplier: f32) -> Vec<String> {
    let queues = crate::bind_world::with_world(|w| w.cell.as_ref().map(|c| c.process_items.clone()));
    let Some(queues) = queues else {
        return Vec::new();
    };
    let size = queues.items.borrow().len();
    if size == 0 {
        return Vec::new();
    }
    let world_age_hours = crate::bind_climate::shared_game_time()
        .borrow()
        .world_age_hours() as f32;
    let cur_minute = (world_age_hours * 60.0) as i32;
    let heat_step = 0.001 * multiplier.max(0.0);
    let mut errors = Vec::new();

    for index in 0..size {
        let entry = queues.items.borrow().get(index).cloned().flatten();
        let Some(item) = entry else { continue };

        let class = match item.try_borrow() {
            Ok(it) => it.template.class,
            Err(_) => continue,
        };

        // Alarm clocks are handled by the alarm clock updater; keep them
        // intact here.
        if matches!(
            class,
            pz_inventory::ItemClass::AlarmClock | pz_inventory::ItemClass::AlarmClockClothing
        ) {
            continue;
        }

        let container = pz_inventory::shared::outermost_container_of(&item);
        let target_temp = container
            .as_ref()
            .and_then(|c| crate::bind_item_container::temperature(thread, c).ok())
            .unwrap_or(1.0);

        let finished = match class {
            pz_inventory::ItemClass::Food => {
                // Full Build 42.22 Food.updateAge / updateFreezing when
                // SandboxOptions is available (minimal headless fixtures
                // without projectzomboid.jar skip sandbox-backed aging).
                if crate::bind_sandbox::instance(thread).is_ok() {
                    if let Err(err) = crate::bind_item_age::update_age(thread, &item, None) {
                        let is_npe = matches!(
                            &err,
                            LuaError::Runtime {
                                class: "java.lang.NullPointerException",
                                ..
                            }
                        );
                        if !is_npe {
                            errors.push(format!("Food.updateAge: {err}"));
                        }
                    }
                }
                match item.try_borrow_mut() {
                    Ok(mut it) => {
                        let minutes_to_burn = it
                            .template
                            .script
                            .as_ref()
                            .map_or(120.0, |s| s.0.borrow().minutes_to_burn as f32);
                        if let pz_inventory::ItemKind::Food(food) = &mut it.kind {
                            food.heat = step_heat(food.heat, target_temp, heat_step);
                            if food.last_cook_minute != cur_minute {
                                if food.last_cook_minute >= 0 && cur_minute > food.last_cook_minute {
                                    let mins = (cur_minute - food.last_cook_minute) as f32;
                                    if food.is_cookable && food.heat > 1.6 {
                                        food.cooking_time += mins;
                                        if !food.cooked
                                            && food.minutes_to_cook > 0.0
                                            && food.cooking_time >= food.minutes_to_cook
                                        {
                                            food.cooked = true;
                                        }
                                        if !food.burnt
                                            && minutes_to_burn > 0.0
                                            && food.cooking_time >= minutes_to_burn
                                        {
                                            food.burnt = true;
                                            food.cooked = false;
                                        }
                                    }
                                }
                                food.last_cook_minute = cur_minute;
                            }
                            container.is_none()
                                && (food.heat - 1.0).abs() <= 1e-5
                                && !food.frozen
                                && food.freezing_time <= 0.0
                                && food.off_age_max >= 1_000_000_000
                        } else {
                            false
                        }
                    }
                    Err(_) => false,
                }
            }
            pz_inventory::ItemClass::Drainable => match item.try_borrow_mut() {
                Ok(mut it) => {
                    let heat = step_heat(it.drainable_heat.unwrap_or(1.0), target_temp, heat_step);
                    it.drainable_heat = Some(heat);
                    !it.activated
                        && (heat - target_temp).abs() <= 1e-5
                        && (target_temp - 1.0).abs() <= 1e-5
                }
                Err(_) => false,
            },
            pz_inventory::ItemClass::Radio => false,
            _ => match item.try_borrow_mut() {
                Ok(mut it) => {
                    it.item_heat = step_heat(it.item_heat, target_temp, heat_step);
                    !it.activated
                        && (it.item_heat - target_temp).abs() <= 1e-5
                        && (target_temp - 1.0).abs() <= 1e-5
                }
                Err(_) => false,
            },
        };

        if finished {
            queues.queue_remove(Some(item));
        }
    }
    errors
}

#[cfg(test)]
mod tests;
