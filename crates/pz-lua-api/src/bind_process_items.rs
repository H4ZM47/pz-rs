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

#[cfg(test)]
mod tests;
