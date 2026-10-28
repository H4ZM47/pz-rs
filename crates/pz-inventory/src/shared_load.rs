//! Actual receiver loading over the original caller buffer.
//!
//! `items` and `including_obsolete_items` are distinct physical list contents.
//! This bounded adapter does not implement replacement/retention of Java
//! ArrayList allocations (`setItems`); callbacks may mutate the inline list
//! contents. The returned Vec projects saved order, not pooled-list identity.
use std::cell::Cell;
use std::rc::Rc;

use pz_entity::runtime_load::{HostResult, RuntimeHostError};

use crate::io::Reader;
use crate::shared::{self, ContainerRef, ItemRef};
use crate::{ItemEnv, ItemKind};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ContainerLoadPhase {
    Type,
    Explored,
    ClearMain,
    ClearObsolete,
    GroupCount,
    RunCount,
    Record,
    PublishMain,
    PublishObsolete,
    DuplicateIds,
    LinkItems,
    Looted,
    Capacity,
    Dirty,
}

#[derive(Debug)]
#[must_use]
pub struct ContainerLoadFailure {
    pub error: RuntimeHostError,
    pub phase: ContainerLoadPhase,
    pub position: usize,
}
impl ContainerLoadFailure {
    /// All removed/created owners were already queued before any callback or
    /// unwind. Consumption never rolls back the actual target or input cursor.
    pub fn into_error_and_retire(self) -> RuntimeHostError {
        self.error
    }
}

thread_local! { static DEPTH: Cell<usize> = const { Cell::new(0) }; }
struct Depth;
impl Depth {
    fn enter() -> HostResult<Self> {
        DEPTH.with(|slot| {
            if slot.get() >= 128 {
                return Err(unsupported("actual container load nesting limit"));
            }
            slot.set(slot.get() + 1);
            Ok(Self)
        })
    }
}
impl Drop for Depth {
    fn drop(&mut self) {
        DEPTH.with(|slot| slot.set(slot.get() - 1));
    }
}
fn unsupported(operation: &'static str) -> RuntimeHostError {
    RuntimeHostError::Unsupported {
        operation,
        kind: None,
    }
}
fn native(error: crate::io::SaveError) -> RuntimeHostError {
    RuntimeHostError::NativeException(pz_entity::EntityError(error.0))
}
fn edit<T>(
    container: &ContainerRef,
    operation: &'static str,
    f: impl FnOnce(&mut shared::SharedContainer) -> T,
) -> HostResult<T> {
    let result = container
        .try_borrow_mut()
        .map_err(|_| RuntimeHostError::Borrowed(operation))
        .map(|mut body| f(&mut body));
    shared::observe_container(container);
    result
}

/// StringUTF over the original initialized slice. Reader's explicit profile
/// has limit == capacity == slice length, not an invented larger Java array.
fn read_type(reader: &mut Reader<'_>) -> HostResult<String> {
    let mut java = pz_kahlua::java_io::JavaReader::new(reader.buf);
    java.set_position(reader.pos).map_err(|error| {
        RuntimeHostError::NativeException(pz_entity::EntityError(error.to_string()))
    })?;
    let result = java.read_string_utf();
    reader.pos = java.position();
    let notices = java.take_notices();
    // Logger host failure is preserved even when the StringUTF operation failed.
    crate::component_host::emit_mod_data_notices(&notices)?;
    result.map_err(|error| {
        RuntimeHostError::NativeException(pz_entity::EntityError(error.to_string()))
    })
}
fn seek(reader: &mut Reader<'_>, position: i32) -> HostResult<()> {
    let mut java = pz_entity::io::Reader::borrow_cursor(reader.buf, &mut reader.pos)
        .map_err(RuntimeHostError::NativeException)?;
    java.seek_java(position)
        .map_err(RuntimeHostError::NativeException)
}
fn record(
    reader: &mut Reader<'_>,
    world_version: i32,
    env: &dyn ItemEnv,
) -> HostResult<Option<ItemRef>> {
    match crate::saved::load_item_record(reader, world_version, env, None) {
        Ok(loaded) => {
            crate::component_host::retire_components(loaded.retired);
            if let Some(item) = &loaded.item {
                shared::pin_owner(item);
            }
            Ok(loaded.item)
        }
        Err(failure) => Err(failure.into_error_and_retire()),
    }
}
fn clear_main(container: &ContainerRef, dirty: bool) -> HostResult<()> {
    edit(container, "actual container main clear", |body| {
        for item in body.items.drain(..) {
            shared::pin_owner(&item);
        }
        // Raw records belong to the explicit archive codec, never a native
        // item allocation. Retire their former carrier outside outer guards.
        if !body.raw_items.is_empty() {
            shared::pin_owner(&Rc::new(std::mem::take(&mut body.raw_items)));
        }
        if dirty {
            body.dirty = true;
            body.draw_dirty = true;
        }
    })
}

/// Native ItemContainer.load into the SAME actual container. Ctor capture
/// precedes every virtual item body load; repeated records load fresh receivers
/// from the original record start. Failure leaves completed lists/fields/cursor.
pub fn load_container_ref(
    container: &ContainerRef,
    reader: &mut Reader<'_>,
    world_version: i32,
    env: &dyn ItemEnv,
) -> Result<Vec<Option<ItemRef>>, ContainerLoadFailure> {
    shared::pin_owner(container);
    let mut phase = ContainerLoadPhase::Type;
    let result = (|| {
        let _depth = Depth::enter()?;
        let kind = read_type(reader)?;
        edit(container, "actual container type load", |body| {
            body.kind = kind
        })?;
        phase = ContainerLoadPhase::Explored;
        let explored = reader.bool().map_err(native)?;
        edit(container, "actual container explored load", |body| {
            body.explored = explored
        })?;
        let saved = compressed(container, reader, world_version, env, &mut phase)?;
        phase = ContainerLoadPhase::LinkItems;
        let mut index = 0;
        loop {
            // Native re-reads current items size/get for each backlink.
            let item = container
                .try_borrow()
                .map_err(|_| RuntimeHostError::Borrowed("actual load container backlinks"))?
                .items
                .get(index)
                .cloned();
            let Some(item) = item else { break };
            shared::pin_owner(&item);
            shared::try_set_container(&item, Some(container))?;
            index += 1;
        }
        phase = ContainerLoadPhase::Looted;
        let looted = reader.bool().map_err(native)?;
        edit(container, "actual container looted load", |body| {
            body.has_been_looted = looted
        })?;
        phase = ContainerLoadPhase::Capacity;
        let capacity = reader.i32().map_err(native)?;
        edit(container, "actual container capacity load", |body| {
            body.capacity = capacity
        })?;
        phase = ContainerLoadPhase::Dirty;
        edit(container, "actual container dirty load", |body| {
            body.dirty = false
        })?;
        Ok(saved)
    })();
    result.map_err(|error| ContainerLoadFailure {
        error,
        phase,
        position: reader.pos,
    })
}

fn compressed(
    container: &ContainerRef,
    reader: &mut Reader<'_>,
    world_version: i32,
    env: &dyn ItemEnv,
    phase: &mut ContainerLoadPhase,
) -> HostResult<Vec<Option<ItemRef>>> {
    *phase = ContainerLoadPhase::ClearMain;
    clear_main(container, false)?;
    *phase = ContainerLoadPhase::ClearObsolete;
    edit(container, "actual obsolete list clear", |body| {
        for item in body.including_obsolete_items.drain(..).flatten() {
            shared::pin_owner(&item);
        }
    })?;
    *phase = ContainerLoadPhase::GroupCount;
    let groups = reader.i16().map_err(native)?;
    let mut saved = Vec::new();
    for _ in 0..groups.max(0) {
        *phase = ContainerLoadPhase::RunCount;
        let count = reader.i32().map_err(native)?;
        let record_start = i32::try_from(reader.pos)
            .map_err(|_| unsupported("actual load original buffer int position"))?;
        *phase = ContainerLoadPhase::Record;
        // Even zero and negative run counts perform this first actual load.
        let first = record(reader, world_version, env)?;
        if first.is_none() {
            *phase = ContainerLoadPhase::DuplicateIds;
            let skip = if count > 1 {
                count.wrapping_sub(1).wrapping_mul(4)
            } else {
                0
            };
            let end = (reader.pos as i32).wrapping_add(skip);
            seek(reader, end)?;
            for _ in 0..count.max(0) {
                if saved.len() >= 1_000_000 {
                    return Err(unsupported("actual saved list host size limit"));
                }
                *phase = ContainerLoadPhase::PublishObsolete;
                edit(container, "actual obsolete null publication", |body| {
                    body.including_obsolete_items.push(None)
                })?;
                saved.push(None);
            }
            continue;
        }
        let run_start = saved.len();
        let mut current = first;
        for index in 0..count.max(0) {
            if saved.len() >= 1_000_000 {
                return Err(unsupported("actual saved list host size limit"));
            }
            if index > 0 {
                *phase = ContainerLoadPhase::Record;
                seek(reader, record_start)?;
                current = record(reader, world_version, env)?;
            }
            *phase = ContainerLoadPhase::PublishMain;
            let Some(item) = current.as_ref() else {
                // Native publishes null into main here. Vec<ItemRef> cannot
                // represent it; never silently skip or manufacture an item.
                return Err(unsupported("actual main item list null publication"));
            };
            edit(container, "actual main item publication", |body| {
                body.items.push(item.clone())
            })?;
            *phase = ContainerLoadPhase::PublishObsolete;
            edit(container, "actual obsolete item publication", |body| {
                body.including_obsolete_items.push(Some(item.clone()))
            })?;
            saved.push(Some(item.clone()));
        }
        *phase = ContainerLoadPhase::DuplicateIds;
        for index in 1..count.max(0) {
            let id = reader.i32().map_err(native)?;
            if let Some(item) = &saved[run_start + index as usize] {
                item.try_borrow_mut()
                    .map_err(|_| RuntimeHostError::Borrowed("actual duplicate item ID load"))?
                    .id = id;
                shared::observe_item(item);
            }
        }
    }
    Ok(saved)
}

fn bag_child(item: &ItemRef) -> HostResult<ContainerRef> {
    let body = item
        .try_borrow()
        .map_err(|_| RuntimeHostError::Borrowed("actual bag child load"))?;
    match &body.kind {
        ItemKind::Container(bag) => {
            let container = bag
                .container
                .slot()
                .ok_or_else(|| unsupported("native null bag inventory load diagnostic"))?;
            shared::pin_owner(container);
            Ok(container.clone())
        }
        _ => Err(RuntimeHostError::InvalidHostState(
            "actual bag load receiver",
        )),
    }
}

/// InventoryContainer.load after the complete base body (including its first
/// synchWithVisual). Each phase re-reads the actual receiver; no body guard
/// crosses a child load or provider callback.
pub(crate) fn load_bag_tail_ref(
    item: &ItemRef,
    reader: &mut Reader<'_>,
    world_version: i32,
    env: &dyn ItemEnv,
) -> HostResult<()> {
    shared::pin_owner(item);
    let id = reader.i32().map_err(native)?;
    let reduction = reader.i32().map_err(native)?.clamp(0, 100);
    {
        let mut body = item
            .try_borrow_mut()
            .map_err(|_| RuntimeHostError::Borrowed("actual bag weight load"))?;
        let ItemKind::Container(bag) = &mut body.kind else {
            return Err(RuntimeHostError::InvalidHostState(
                "actual bag load receiver",
            ));
        };
        bag.weight_reduction = reduction;
    }
    shared::observe_item(item);
    let child = bag_child(item)?;
    edit(&child, "actual bag setWeightReduction child", |body| {
        body.weight_reduction = reduction
    })?;
    // Child is nonnullable in the supported actual constructor profile. Native
    // setWeightReduction dereferences it BEFORE load's fallback allocation.
    clear_main(&bag_child(item)?, true)?;
    let child = bag_child(item)?;
    edit(&child, "actual bag containingItem load", |body| {
        if let Some(old) = body.containing_item.replace(item.clone()) {
            shared::pin_owner(&old);
        }
    })?;
    let reduction = {
        let body = item
            .try_borrow()
            .map_err(|_| RuntimeHostError::Borrowed("actual bag current weight load"))?;
        let ItemKind::Container(bag) = &body.kind else {
            return Err(RuntimeHostError::InvalidHostState(
                "actual bag load receiver",
            ));
        };
        bag.weight_reduction
    };
    edit(
        &bag_child(item)?,
        "actual bag child weight reload",
        |body| body.weight_reduction = reduction.clamp(0, 100),
    )?;
    let capacity = {
        let body = item
            .try_borrow()
            .map_err(|_| RuntimeHostError::Borrowed("actual bag raw capacity load"))?;
        let ItemKind::Container(bag) = &body.kind else {
            return Err(RuntimeHostError::InvalidHostState(
                "actual bag load receiver",
            ));
        };
        bag.capacity
    };
    edit(
        &bag_child(item)?,
        "actual bag child capacity load",
        |body| body.capacity = capacity,
    )?;
    edit(&bag_child(item)?, "actual bag child ID load", |body| {
        body.id = id
    })?;
    load_container_ref(&bag_child(item)?, reader, world_version, env)
        .map_err(ContainerLoadFailure::into_error_and_retire)?;
    crate::item_texture::synch_with_visual_ref(
        item,
        env.assets(),
        &crate::context::NoOwner,
        env.visual_rng(),
        &mut |m| env.rand_next(m),
    )
}

#[cfg(test)]
mod tests;
