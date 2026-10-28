//! Guard-free virtual `InventoryItem.updateAge` dispatch.
//!
//! Containers retain their actual inventory and walk its live indexed list.
//! Food mutations occur at the native phase boundaries, with no item,
//! container or world borrow across a potentially reentrant host call.

use crate::events::EventManager;
use pz_inventory::food_age;
use pz_inventory::item::FoodData;
use pz_inventory::shared::{self, ContainerRef, ItemRef};
use pz_inventory::ItemKind;
use pz_kahlua::{LuaError, LuaResult, Thread, Value};
use std::cell::RefCell;
use std::rc::Rc;

#[cfg(test)]
mod tests;

thread_local! {
    static EVENTS: RefCell<Option<EventManager>> = const { RefCell::new(None) };
}

pub(crate) fn install_events(events: EventManager) {
    let retired = EVENTS.with(|e| e.replace(Some(events)));
    drop(retired);
}

fn missing(what: &str) -> LuaError {
    LuaError::Runtime {
        class: "java.lang.NullPointerException",
        message: what.into(),
    }
}

fn food<R>(item: &ItemRef, f: impl FnOnce(&FoodData) -> R) -> LuaResult<R> {
    let item = item
        .try_borrow()
        .map_err(|_| LuaError::runtime("borrowed food receiver"))?;
    match &item.kind {
        ItemKind::Food(food) => Ok(f(food)),
        _ => Err(LuaError::runtime("food receiver changed its native class")),
    }
}

fn food_mut<R>(item: &ItemRef, f: impl FnOnce(&mut FoodData) -> R) -> LuaResult<R> {
    let mut item = item
        .try_borrow_mut()
        .map_err(|_| LuaError::runtime("borrowed food receiver"))?;
    match &mut item.kind {
        ItemKind::Food(food) => Ok(f(food)),
        _ => Err(LuaError::runtime("food receiver changed its native class")),
    }
}

/// The optional owner is used by authored fixtures. Production resolves the
/// actual singleton at each native option read, including after host calls.
#[derive(Default)]
pub(crate) struct Runtime {
    pub options: Option<Rc<RefCell<pz_sandbox::SandboxOptions>>>,
    pub events: Option<EventManager>,
}

impl Runtime {
    fn options(&self, thread: &mut Thread) -> LuaResult<Rc<RefCell<pz_sandbox::SandboxOptions>>> {
        if let Some(options) = &self.options {
            return Ok(options.clone());
        }
        let value = crate::bind_sandbox::instance(thread)?;
        match value {
            Value::Object(o) => o
                .as_any()
                .downcast_ref::<crate::bind_sandbox::SandboxObj>()
                .map(|o| o.0.clone())
                .ok_or_else(|| missing("SandboxOptions.instance")),
            _ => Err(missing("SandboxOptions.instance")),
        }
    }

    fn option(&self, thread: &mut Thread, name: &str) -> LuaResult<i32> {
        let options = self.options(thread)?;
        let options = options.borrow();
        options
            .option_by_name(name)
            .map(|o| o.config.get_int())
            .ok_or_else(|| missing(name))
    }

    fn event(&self, thread: &mut Thread, item: &ItemRef) -> LuaResult<()> {
        let events = self
            .events
            .clone()
            .or_else(|| EVENTS.with(|e| e.borrow().clone()))
            .ok_or_else(|| missing("LuaEventManager"))?;
        events.trigger(
            thread,
            "OnContainerUpdate",
            &[crate::bind_items::item_value(item)],
        );
        Ok(())
    }
}

fn in_fridge(thread: &mut Thread, container: Option<&ContainerRef>) -> LuaResult<bool> {
    match container {
        Some(c) => crate::bind_item_container::is_fridge(thread, c),
        None => Ok(false),
    }
}

fn in_freezer(container: Option<&ContainerRef>) -> bool {
    container.is_some_and(|c| c.borrow().is_freezer())
}

pub(crate) fn is_freezing(thread: &mut Thread, item: &ItemRef) -> LuaResult<bool> {
    let outer = shared::outermost_container_of(item);
    let excluded = food(item, |f| !f.can_be_frozen || f.freezing_time >= 100.0)?;
    if excluded || !in_freezer(outer.as_ref()) {
        return Ok(false);
    }
    crate::bind_item_container::is_powered(thread, outer.as_ref().unwrap())
}

pub(crate) fn is_thawing(thread: &mut Thread, item: &ItemRef) -> LuaResult<bool> {
    let outer = shared::outermost_container_of(item);
    let excluded = food(item, |f| !f.can_be_frozen || f.freezing_time <= 0.0)?;
    if excluded {
        return Ok(false);
    }
    if !in_freezer(outer.as_ref()) {
        return Ok(true);
    }
    Ok(!crate::bind_item_container::is_powered(
        thread,
        outer.as_ref().unwrap(),
    )?)
}

fn process_contains(item: &ItemRef) -> LuaResult<bool> {
    crate::bind_world::with_world(|w| {
        let cell = w
            .cell
            .as_ref()
            .ok_or_else(|| missing("IsoWorld.currentCell"))?;
        let found = cell
            .process_items
            .items
            .borrow()
            .iter()
            .any(|entry| entry.as_ref().is_some_and(|entry| Rc::ptr_eq(entry, item)));
        Ok(found)
    })
}

fn process_add(item: &ItemRef) -> LuaResult<()> {
    let client = crate::bind_core::shared_core().borrow().is_client;
    crate::bind_world::with_world(|w| {
        let cell = w
            .cell
            .as_ref()
            .ok_or_else(|| missing("IsoWorld.currentCell"))?;
        cell.process_items.add(Some(item.clone()), client);
        Ok(())
    })
}

fn source_electricity(thread: &mut Thread, container: &ContainerRef) -> LuaResult<bool> {
    let grid = crate::bind_item_container::source_grid(container);
    if grid.is_nil() {
        return Ok(false);
    }
    let method = thread.table_get(&grid, &Value::str("haveElectricity"))?;
    if method.is_nil() {
        return Err(LuaError::runtime(
            "source grid haveElectricity is not bound",
        ));
    }
    Ok(thread.call1(&method, &[grid])?.truthy())
}

fn update_food(
    thread: &mut Thread,
    item: &ItemRef,
    send_stats: bool,
    runtime: &Runtime,
) -> LuaResult<()> {
    let now = crate::bind_climate::shared_game_time()
        .borrow()
        .world_age_hours() as f32;
    let container = shared::outermost_container_of(item);
    if let Some(dt) = food_mut(item, |f| f.begin_freezing_update(now))? {
        if is_freezing(thread, item)? {
            food_mut(item, |f| f.apply_freezing(dt))?;
        }
        // A separate live query, after the first branch's mutation.
        if is_thawing(thread, item)? {
            let powered_fridge = in_fridge(thread, container.as_ref())?
                && crate::bind_item_container::is_powered(thread, container.as_ref().unwrap())?;
            let hot = match &container {
                Some(c) => crate::bind_item_container::temperature(thread, c)? > 1.0,
                None => false,
            };
            food_mut(item, |f| {
                f.apply_thawing(dt, food_age::thaw_hours(powered_fridge, hot))
            })?;
        }
        food_mut(item, |f| f.last_frozen_update = now)?;
    }

    let Some(mut elapsed) = food_mut(item, |f| f.begin_age_update(now))? else {
        return Ok(());
    };
    if let Some(c) = &container {
        let heat = food(item, |f| f.heat)?;
        if heat != crate::bind_item_container::temperature(thread, c)? {
            if elapsed < 0.3333333432674408f64 {
                if !process_contains(item)? {
                    let heat = food(item, |f| f.heat)?;
                    let temperature = crate::bind_item_container::temperature(thread, c)?;
                    let heat =
                        food_age::lerp_heat(heat, temperature, elapsed as f32 / 0.33333334f32);
                    food_mut(item, |f| f.heat = heat)?;
                    process_add(item)?;
                }
            } else {
                let temperature = crate::bind_item_container::temperature(thread, c)?;
                food_mut(item, |f| f.heat = temperature)?;
            }
        }
    }

    if food(item, |f| f.frozen)? {
        elapsed = food_age::frozen_age_hours(elapsed);
    } else if in_fridge(thread, container.as_ref())? || in_freezer(container.as_ref()) {
        let c = container.as_ref().unwrap();
        if source_electricity(thread, c)? {
            elapsed *= food_age::fridge_factor(runtime.option(thread, "FridgeFactor")?) as f64;
        } else {
            let modifier = runtime.option(thread, "ElecShutModifier")?;
            let last = food(item, |f| f.last_aged)?;
            if modifier > -1 && last < modifier.wrapping_mul(24) as f32 {
                let factor = food_age::fridge_factor(runtime.option(thread, "FridgeFactor")?);
                elapsed = food_age::outage_hours(last, now, modifier, factor);
            }
        }
    }
    let rot_speed = food_age::food_rot_speed(runtime.option(thread, "FoodRotSpeed")?);
    let transition = food_mut(item, |f| f.apply_age_delta(now, elapsed, rot_speed))?;
    let server = crate::bind_core::shared_core().borrow().is_server;
    if !server && transition.changed() {
        runtime.event(thread, item)?;
    }
    if send_stats && crate::bind_core::shared_core().borrow().is_server {
        // Preserve completed aging. Actual server transport is a separate port;
        // a successful local mutation is not proof of a sent network packet.
        return Err(LuaError::runtime("GameServer.sendItemStats is not ported"));
    }
    Ok(())
}

pub(crate) fn update_age_with(
    thread: &mut Thread,
    item: &ItemRef,
    send_stats: Option<bool>,
    runtime: &Runtime,
) -> LuaResult<()> {
    let (is_food, inventory) = {
        let item = item.borrow();
        (
            matches!(item.kind, ItemKind::Food(_)),
            shared::inventory_of(&item),
        )
    };
    if is_food {
        let send_stats = send_stats.unwrap_or_else(food_age::check_update_rate);
        return update_food(thread, item, send_stats, runtime);
    }
    if send_stats.is_some() {
        return Err(LuaError::runtime("updateAge(boolean) requires Food"));
    }
    if let Some(inventory) = inventory {
        let mut index = 0;
        loop {
            let child = inventory.borrow().items.get(index).cloned();
            let Some(child) = child else { break };
            update_age_with(thread, &child, None, runtime)?;
            index += 1;
        }
    }
    // Native InventoryItem.updateAge is a genuine return-only base method.
    Ok(())
}

pub(crate) fn update_age(
    thread: &mut Thread,
    item: &ItemRef,
    send_stats: Option<bool>,
) -> LuaResult<()> {
    update_age_with(thread, item, send_stats, &Runtime::default())
}
