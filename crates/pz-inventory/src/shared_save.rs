//! Actual shared-owner save phases. Archive/value codecs remain separate.
//!
//! The returned Vec is a Rust projection of native saved-item order; it does
//! not claim the identity/reuse of Native's pooled returned ArrayList.
use std::any::Any;
use std::cell::RefCell;
use std::panic::{catch_unwind, resume_unwind, AssertUnwindSafe};
use std::rc::Rc;

use pz_entity::components::attributes::AttributeValue;
use pz_entity::context::AttributeKind;
use pz_entity::runtime_load::{HostResult, RuntimeHostError};
use pz_entity::{ComponentRef, ComponentType, RetiredComponents};

use crate::io::{self, NativeSaveBufferRef, NativeWriterFault, Writer};
use crate::item_texture::ItemPersistenceError;
use crate::shared::{ContainerRef, ItemRef};
use crate::{ItemEnv, ItemKind};

type Result<T> = std::result::Result<T, ItemPersistenceError>;

thread_local! {
    // Native owns ONE comparison buffer per thread, shared across nested
    // calls. Each acquired local buffer handle keeps its original identity
    // even if a nested overflow replaces the thread's current buffer.
    static COMPARE: RefCell<Option<NativeSaveBufferRef>> = const { RefCell::new(None) };
}
fn unsupported(operation: &'static str) -> RuntimeHostError {
    RuntimeHostError::Unsupported {
        operation,
        kind: None,
    }
}
fn pin<T: Any>(owner: &Rc<T>) {
    let mut retired = RetiredComponents::new();
    retired.push_erased(owner.clone());
    crate::component_host::retire_components(retired);
}
fn current_buffer() -> NativeSaveBufferRef {
    COMPARE.with(|slot| {
        slot.borrow_mut()
            .get_or_insert_with(io::new_native_save_buffer)
            .clone()
    })
}
fn replace_buffer(buffer: &NativeSaveBufferRef) {
    COMPARE.with(|slot| {
        *slot.borrow_mut() = Some(buffer.clone());
    });
}

#[derive(Default)]
pub(crate) struct SaveContext {
    active_items: Vec<usize>,
    active_containers: Vec<usize>,
}
impl SaveContext {
    fn item(&mut self, item: &ItemRef, w: &mut Writer, env: &dyn ItemEnv) -> Result<()> {
        pin(item);
        let key = Rc::as_ptr(item) as usize;
        if self.active_items.contains(&key) || self.active_items.len() >= 128 {
            return Err(unsupported("actual save recursive item graph/depth").into());
        }
        self.active_items.push(key);
        let result = catch_unwind(AssertUnwindSafe(|| {
            crate::item_texture::save_item_ref_in_context(item, w, env, self)
        }));
        self.active_items.pop();
        match result {
            Ok(result) => result,
            Err(payload) => resume_unwind(payload),
        }
    }
    fn container(
        &mut self,
        container: &ContainerRef,
        w: &mut Writer,
        env: &dyn ItemEnv,
    ) -> Result<Vec<ItemRef>> {
        pin(container);
        let key = Rc::as_ptr(container) as usize;
        if self.active_containers.contains(&key) || self.active_containers.len() >= 128 {
            return Err(unsupported("actual save recursive container graph/depth").into());
        }
        self.active_containers.push(key);
        let result = catch_unwind(AssertUnwindSafe(|| {
            save_container_inner(container, w, env, self)
        }));
        self.active_containers.pop();
        match result {
            Ok(result) => result,
            Err(payload) => resume_unwind(payload),
        }
    }
}

/// `ItemContainer.save`: Native ignores its character argument and passes
/// literal null to compression. Caller must release enclosing body guards.
pub fn save_container_ref(
    container: &ContainerRef,
    w: &mut Writer,
    env: &dyn ItemEnv,
) -> Result<Vec<ItemRef>> {
    SaveContext::default().container(container, w, env)
}

/// Native item save, including guard-free nested bag continuations.
pub fn save_item_ref(item: &ItemRef, w: &mut Writer, env: &dyn ItemEnv) -> Result<()> {
    SaveContext::default().item(item, w, env)
}

/// Direct CompressIdenticalItems entry. The optional callback represents the
/// actual character's isEquipped query; ItemContainer itself passes None.
pub fn save_items_ref(
    items: &[ItemRef],
    w: &mut Writer,
    env: &dyn ItemEnv,
    no_compress: Option<&dyn Fn(&ItemRef) -> HostResult<bool>>,
) -> Result<Vec<ItemRef>> {
    compress(items, w, env, no_compress, &mut SaveContext::default())
}

fn save_container_inner(
    container: &ContainerRef,
    w: &mut Writer,
    env: &dyn ItemEnv,
    context: &mut SaveContext,
) -> Result<Vec<ItemRef>> {
    let kind = container
        .try_borrow()
        .map_err(|_| RuntimeHostError::Borrowed("actual container save type"))?
        .kind
        .clone();
    w.string(&kind);
    let explored = container
        .try_borrow()
        .map_err(|_| RuntimeHostError::Borrowed("actual container save explored"))?
        .explored;
    w.bool(explored);
    // Capture only native list membership/actual owner pins, not item data
    // or pre-serialized records. These are the groups' stable input owners
    // after a provider removes/reorders/replaces the original list field.
    let items = {
        let body = container
            .try_borrow()
            .map_err(|_| RuntimeHostError::Borrowed("actual container save items"))?;
        if !body.raw_items.is_empty() {
            return Err(unsupported("actual container save selected raw archive records").into());
        }
        body.items.clone()
    };
    for item in &items {
        pin(item);
    }
    let saved = compress(&items, w, env, None, context)?;
    let looted = container
        .try_borrow()
        .map_err(|_| RuntimeHostError::Borrowed("actual container save looted"))?
        .has_been_looted;
    w.bool(looted);
    let capacity = container
        .try_borrow()
        .map_err(|_| RuntimeHostError::Borrowed("actual container save capacity"))?
        .capacity;
    w.i32(capacity);
    Ok(saved)
}

pub(crate) fn save_bag_tail(
    item: &ItemRef,
    w: &mut Writer,
    env: &dyn ItemEnv,
    context: &mut SaveContext,
) -> Result<()> {
    let inventory = bag_inventory(item)?;
    let id = inventory
        .try_borrow()
        .map_err(|_| RuntimeHostError::Borrowed("actual bag inventory ID"))?
        .id;
    w.i32(id);
    let reduction = {
        let body = item
            .try_borrow()
            .map_err(|_| RuntimeHostError::Borrowed("actual bag weightReduction"))?;
        let ItemKind::Container(bag) = &body.kind else {
            return Err(unsupported("actual bag save class changed").into());
        };
        bag.weight_reduction
    };
    w.i32(reduction);
    // Native rereads the current field after ID and reduction, then invokes
    // that exact receiver. No parent/container borrow spans the child call.
    let inventory = bag_inventory(item)?;
    context.container(&inventory, w, env)?;
    Ok(())
}
fn bag_inventory(item: &ItemRef) -> Result<ContainerRef> {
    let inventory = {
        let body = item
            .try_borrow()
            .map_err(|_| RuntimeHostError::Borrowed("actual bag inventory field"))?;
        let ItemKind::Container(bag) = &body.kind else {
            return Err(unsupported("InventoryContainer cast diagnostic").into());
        };
        bag.container.clone()
    };
    pin(&inventory);
    Ok(inventory)
}

fn item_id(item: &ItemRef) -> Result<i32> {
    Ok(item
        .try_borrow()
        .map_err(|_| RuntimeHostError::Borrowed("actual save item ID"))?
        .id)
}
fn set_id(item: &ItemRef, id: i32) -> Result<()> {
    item.try_borrow_mut()
        .map_err(|_| RuntimeHostError::Borrowed("actual save item ID write"))?
        .id = id;
    Ok(())
}
fn zero_id<T>(item: &ItemRef, body: impl FnOnce() -> Result<T>) -> Result<T> {
    pin(item);
    let original = item_id(item)?;
    set_id(item, 0)?;
    let result = catch_unwind(AssertUnwindSafe(body));
    let restored = set_id(item, original);
    match result {
        Ok(result) => {
            restored?;
            result
        }
        Err(payload) => {
            // Attempt finally even during a control unwind. A failed ID
            // restore supersedes native writer control with a typed host
            // fault; unrelated programmer panics still resume unchanged.
            if let Err(error) = restored {
                if io::is_native_writer_control(&*payload) {
                    return Err(error);
                }
            }
            resume_unwind(payload)
        }
    }
}

fn compare_first(first: &ItemRef, env: &dyn ItemEnv, context: &mut SaveContext) -> Result<()> {
    // Captured ONCE, unlike candidate retries which read current TLS again.
    let initial = current_buffer();
    io::native_buffer_clear(&initial)?;
    zero_id(first, || {
        let mut buffer = initial;
        loop {
            let attempt = io::catch_native_overflow(|| -> Result<()> {
                let mut scratch = Writer::for_native_buffer(&buffer);
                scratch.i32(0);
                context.item(first, &mut scratch, env)?;
                let end = scratch.pos();
                scratch.patch_i32(0, end as i32);
                Ok(())
            });
            match attempt {
                Ok(result) => return result,
                Err(NativeWriterFault::Host(error)) => return Err(error.into()),
                Err(NativeWriterFault::Overflow) => {
                    buffer = io::native_buffer_ensure_capacity(&buffer)?;
                    io::native_buffer_clear(&buffer)?;
                    replace_buffer(&buffer);
                }
            }
        }
    })
}

fn candidate_identical(
    first: &ItemRef,
    candidate: &ItemRef,
    env: &dyn ItemEnv,
    context: &mut SaveContext,
) -> Result<bool> {
    if !comparison_prechecks(first, candidate)? {
        return Ok(false);
    }
    zero_id(candidate, || loop {
        let buffer = current_buffer();
        let attempt = io::catch_native_overflow(|| -> Result<bool> {
            io::native_buffer_set_position(&buffer, 0)?;
            let end = io::native_buffer_i32_at(&buffer, 0)?;
            if end < 4 {
                return Err(unsupported("native comparison buffer first record bounds").into());
            }
            io::native_buffer_set_position(&buffer, 4)?;
            let start = io::native_buffer_position(&buffer)?;
            io::native_buffer_set_position(&buffer, end as usize)?;
            let candidate_start = io::native_buffer_position(&buffer)?;
            let mut scratch = Writer::for_native_buffer(&buffer);
            context.item(candidate, &mut scratch, env)?;
            let candidate_end = scratch.pos();
            if candidate_end.checked_sub(candidate_start) != Some(end as usize - start) {
                return Ok(false);
            }
            let a = io::native_buffer_bytes(&buffer, start, end as usize)?;
            let b = io::native_buffer_bytes(&buffer, candidate_start, candidate_end)?;
            Ok(a == b)
        });
        match attempt {
            Ok(result) => return result,
            Err(NativeWriterFault::Host(error)) => return Err(error.into()),
            Err(NativeWriterFault::Overflow) => {
                let grown = io::native_buffer_ensure_capacity(&buffer)?;
                io::native_buffer_clear(&grown)?;
                replace_buffer(&grown);
                compare_first(first, env, context)?;
            }
        }
    })
}

fn compress(
    items: &[ItemRef],
    w: &mut Writer,
    env: &dyn ItemEnv,
    no_compress: Option<&dyn Fn(&ItemRef) -> HostResult<bool>>,
    context: &mut SaveContext,
) -> Result<Vec<ItemRef>> {
    // Borrowed input may itself be a caller's temporary list. Pin every
    // physical owner before the first fallible read so even an early error
    // cannot final-drop an unread later item under that caller's guard.
    for item in items {
        pin(item);
    }
    let mut groups: Vec<(String, Vec<ItemRef>)> = Vec::new();
    for item in items {
        let full_type = item
            .try_borrow()
            .map_err(|_| RuntimeHostError::Borrowed("actual compression full type"))?
            .get_full_type();
        if let Some((_, group)) = groups.iter_mut().find(|(name, _)| *name == full_type) {
            group.push(item.clone());
        } else {
            groups.push((full_type, vec![item.clone()]));
        }
    }
    let at = w.pos();
    w.i16(0);
    let mut runs = 0i16;
    let mut saved = Vec::new();
    for (_, group) in groups {
        let mut start = 0;
        while start < group.len() {
            let first = &group[start];
            saved.push(first.clone());
            let mut end = start + 1;
            if !no_compress
                .map(|query| query(first))
                .transpose()?
                .unwrap_or(false)
            {
                // Required even for singleton/nonempty-bag runs.
                compare_first(first, env, context)?;
                while end < group.len() && candidate_identical(first, &group[end], env, context)? {
                    saved.push(group[end].clone());
                    end += 1;
                }
            }
            w.i32((end - start) as i32);
            let record_start = w.pos();
            w.i32(0);
            context.item(first, w, env)?;
            w.patch_i32(
                record_start,
                (w.pos() as i32)
                    .wrapping_sub(record_start as i32)
                    .wrapping_sub(4),
            );
            for item in &group[start + 1..end] {
                w.i32(item_id(item)?);
            }
            runs = runs.wrapping_add(1);
            start = end;
        }
    }
    w.patch_i16(at, runs);
    Ok(saved)
}

fn attributes(item: &ItemRef) -> Result<Option<ComponentRef>> {
    let component = item
        .try_borrow()
        .map_err(|_| RuntimeHostError::Borrowed("actual comparison attributes field"))?
        .components
        .get_ref(ComponentType::Attributes)
        .map_err(RuntimeHostError::Storage)?;
    if let Some(component) = &component {
        pin(component);
    }
    Ok(component)
}
fn comparison_prechecks(first: &ItemRef, candidate: &ItemRef) -> Result<bool> {
    let first_is_bag = matches!(
        first
            .try_borrow()
            .map_err(|_| RuntimeHostError::Borrowed("actual comparison first class"))?
            .kind,
        ItemKind::Container(_)
    );
    if first_is_bag {
        let a = bag_inventory(first)?;
        let b = bag_inventory(candidate)?; // Native casts before emptiness.
        let a_empty = a
            .try_borrow()
            .map_err(|_| RuntimeHostError::Borrowed("actual comparison first bag"))?
            .items
            .is_empty();
        if !a_empty {
            return Ok(false);
        }
        let b_empty = b
            .try_borrow()
            .map_err(|_| RuntimeHostError::Borrowed("actual comparison second bag"))?
            .items
            .is_empty();
        if !b_empty {
            return Ok(false);
        }
    }
    match (attributes(first)?, attributes(candidate)?) {
        (Some(a), Some(b)) => {
            if !attributes_identical(&a, &b)? {
                return Ok(false);
            }
        }
        (None, None) => {}
        _ => return Ok(false),
    }
    let a = first
        .try_borrow()
        .map_err(|_| RuntimeHostError::Borrowed("actual comparison first ByteData"))?
        .byte_data
        .is_some();
    let b = candidate
        .try_borrow()
        .map_err(|_| RuntimeHostError::Borrowed("actual comparison second ByteData"))?
        .byte_data
        .is_some();
    if a {
        return Err(unsupported("actual ByteData ByteBuffer position/limit comparison").into());
    }
    Ok(!b)
}
fn attributes_identical(a: &ComponentRef, b: &ComponentRef) -> Result<bool> {
    let first = {
        let body = a
            .try_borrow()
            .map_err(|_| RuntimeHostError::Borrowed("actual first AttributeContainer"))?;
        let attributes = body
            .attributes()
            .ok_or_else(|| unsupported("actual Attributes container body"))?;
        if !attributes.is_resolved() {
            return Err(unsupported("unresolved first AttributeContainer").into());
        }
        attributes
            .entry_slots()
            .map(|slot| slot.instance_slot().clone())
            .collect::<Vec<_>>()
    };
    for instance in &first {
        pin(instance);
    }
    let second_count = {
        let body = b
            .try_borrow()
            .map_err(|_| RuntimeHostError::Borrowed("actual second AttributeContainer"))?;
        let attributes = body
            .attributes()
            .ok_or_else(|| unsupported("actual Attributes container body"))?;
        if !attributes.is_resolved() {
            return Err(unsupported("unresolved second AttributeContainer").into());
        }
        attributes.entry_slots().count()
    };
    if first.len() != second_count {
        return Ok(false);
    }
    for instance in first {
        let (kind, type_ref, value) = {
            let body = instance
                .try_borrow()
                .map_err(|_| RuntimeHostError::Borrowed("actual first AttributeInstance"))?;
            (
                body.kind(),
                body.type_slot().cloned(),
                body.value_snapshot(),
            )
        };
        let type_ref =
            type_ref.ok_or_else(|| unsupported("null attribute type comparison diagnostic"))?;
        pin(&type_ref);
        let other = {
            let body = b
                .try_borrow()
                .map_err(|_| RuntimeHostError::Borrowed("actual second AttributeContainer"))?;
            body.attributes()
                .ok_or_else(|| unsupported("actual Attributes container body"))?
                .get_ref_by_type(&type_ref)
                .map_err(RuntimeHostError::Storage)?
                .cloned()
        };
        let Some(other) = other else {
            return Ok(false);
        };
        pin(&other);
        if !matches!(kind, AttributeKind::Float | AttributeKind::Int) {
            return Err(unsupported("native attribute equalTo subclass").into());
        }
        let body = other
            .try_borrow()
            .map_err(|_| RuntimeHostError::Borrowed("actual second AttributeInstance"))?;
        // Native numeric equalTo returns true for different type pointers.
        if body.kind() != kind {
            return Err(unsupported("native attribute equalTo class cast diagnostic").into());
        }
        if !body
            .type_slot()
            .is_some_and(|other_type| Rc::ptr_eq(other_type, &type_ref))
        {
            continue;
        }
        let same = match (&value, body.value()) {
            (AttributeValue::Float(a), AttributeValue::Float(b))
                if kind == AttributeKind::Float && body.kind() == kind =>
            {
                a == b
            }
            (AttributeValue::Int(a), AttributeValue::Int(b))
                if kind == AttributeKind::Int && body.kind() == kind =>
            {
                a == b
            }
            _ => return Err(unsupported("native attribute equalTo class cast diagnostic").into()),
        };
        if !same {
            return Ok(false);
        }
    }
    Ok(true)
}

#[cfg(test)]
mod tests;
