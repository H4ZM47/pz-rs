//! Guard-free native phases over the actual complete world receiver.
//! The supported Fluid hooks inherit Component's authenticated empty bodies;
//! no callbacks are replaced by archive values or copied component bodies.
#[cfg(test)]
use std::rc::Rc;

use pz_entity::runtime_load::{HostResult, RuntimeHostError};
use pz_entity::{ComponentRef, ComponentType, EntityOwner, RetiredComponents};
use pz_inventory::shared::ItemRef;

use crate::object_slot::IsoObjectRef;
use crate::objects::ObjectKind;

fn borrowed(name: &'static str) -> RuntimeHostError {
    RuntimeHostError::Borrowed(name)
}

pub fn floor_ref(object: &IsoObjectRef) -> HostResult<super::FloorObjectRef> {
    let object = object
        .try_borrow()
        .map_err(|_| borrowed("world item receiver"))?;
    let ObjectKind::WorldInventory(floor) = &object.kind else {
        return Err(RuntimeHostError::InvalidHostState(
            "receiver is not IsoWorldInventoryObject",
        ));
    };
    floor
        .payload_slot()
        .cloned()
        .ok_or(RuntimeHostError::InvalidHostState(
            "collected floor payload",
        ))
}

pub fn item_ref(object: &IsoObjectRef) -> HostResult<Option<ItemRef>> {
    let floor = floor_ref(object)?;
    let floor = floor
        .try_borrow()
        .map_err(|_| borrowed("world item payload"))?;
    if floor.item.is_raw() {
        return Err(RuntimeHostError::Unsupported {
            operation: "raw archive floor item as live receiver",
            kind: None,
        });
    }
    Ok(floor.item.item_slot().cloned())
}

/// InventoryItem.setWorldItem only. The caller chooses the authentic phase.
/// Neither allocation nor raw list membership implies this field assignment.
pub fn publish_item_backlink(
    object: &IsoObjectRef,
    item: &ItemRef,
    retired: &mut RetiredComponents,
) -> HostResult<()> {
    let owner = EntityOwner::from_ref(object);
    // Protect both inputs on every error, even under an enclosing WORLD guard.
    retired.push_owner(owner.clone());
    retired.push_owner(EntityOwner::from_ref(item));
    item.try_borrow_mut()
        .map_err(|_| borrowed("set item world owner"))?
        .replace_world_item(Some(&owner), retired);
    pz_inventory::shared::observe_item(item);
    Ok(())
}

fn supported_hook(kind: ComponentType) -> HostResult<()> {
    match kind {
        ComponentType::FluidContainer
        | ComponentType::Script
        | ComponentType::Attributes
        | ComponentType::Durability
        | ComponentType::ContextMenuConfig => Ok(()),
        _ => Err(RuntimeHostError::Unsupported {
            operation: "actual component inherited owner/connect hook",
            kind: Some(kind),
        }),
    }
}

pub fn connect_world(object: &IsoObjectRef) -> HostResult<()> {
    let mut index = 0;
    loop {
        let component = {
            let body = object
                .try_borrow()
                .map_err(|_| borrowed("world component connect"))?;
            if index >= body.components.len() {
                break;
            }
            body.components
                .get_ref_at(index)
                .map_err(RuntimeHostError::Storage)?
                .ok_or(RuntimeHostError::InvalidHostState(
                    "missing world component index",
                ))?
        };
        let kind = component
            .try_borrow()
            .map_err(|_| borrowed("world component connect body"))?
            .kind();
        supported_hook(kind)?;
        index += 1;
    }
    Ok(())
}

pub fn remove_world_component(
    object: &IsoObjectRef,
    component: &ComponentRef,
    retired: &mut RetiredComponents,
) -> HostResult<Option<ComponentRef>> {
    retired.push_owner(EntityOwner::from_ref(object));
    retired.push_component(component.clone());
    let removed = {
        let mut body = object
            .try_borrow_mut()
            .map_err(|_| borrowed("remove world component membership"))?;
        let removed = body
            .components
            .runtime_mut()
            .remove_exact_unordered(component);
        body.has_components = !body.components.is_empty();
        removed
    };
    if let Some(removed) = removed.as_ref() {
        retired.push_component(removed.clone());
        let kind = removed
            .try_borrow()
            .map_err(|_| borrowed("removed world component kind"))?
            .kind();
        supported_hook(kind)?;
        let old = removed
            .try_borrow_mut()
            .map_err(|_| borrowed("clear world component owner"))?
            .replace_owner(None);
        if let Some(old) = old {
            retired.push_owner(old);
        }
    }
    Ok(removed)
}

pub fn release_detached(
    component: &ComponentRef,
    retired: &mut RetiredComponents,
) -> HostResult<()> {
    use pz_entity::runtime_pool as pool;
    let kind = component
        .try_borrow()
        .map_err(|_| borrowed("released world component kind"))?
        .kind();
    if !pool::component_pool_exists(kind).map_err(pz_inventory::component_host::pool_error)? {
        return Ok(());
    }
    let debug = pz_inventory::component_host::component_debug_enabled()?;
    pool::release_detached_component_checked(
        component,
        retired,
        pool::PoolReleaseOptions {
            assertions_enabled: false,
            debug,
        },
    )
    .map(|_| ())
    .map_err(pz_inventory::component_host::pool_error)
}

/// Native addComponent while the complete destination is still before
/// addToWorld. This is deliberately not a substitute for manager registration
/// on an already active world entity.
pub fn add_before_world(
    object: &IsoObjectRef,
    component: &ComponentRef,
    retired: &mut RetiredComponents,
) -> HostResult<bool> {
    retired.push_owner(EntityOwner::from_ref(object));
    retired.push_component(component.clone());
    let kind = component
        .try_borrow()
        .map_err(|_| borrowed("incoming world component kind"))?
        .kind();
    {
        let body = object
            .try_borrow()
            .map_err(|_| borrowed("world add prechecks"))?;
        if body.components.len() >= 127 {
            return Ok(false);
        }
        if body.components.runtime().presence(kind) != pz_entity::EntryPresence::Absent {
            return Ok(false);
        }
    }
    if component
        .try_borrow()
        .map_err(|_| borrowed("incoming world component owner"))?
        .owner_slot()
        .is_some()
    {
        return Err(RuntimeHostError::InvalidHostState(
            "world add needs the authentic prior-owner removal phase",
        ));
    }
    if let Err(rejected) = object
        .try_borrow_mut()
        .map_err(|_| borrowed("world component add membership"))?
        .components
        .runtime_mut()
        .push_ref_if_absent(component.clone())
    {
        retired.push_component(rejected.component);
        return Err(RuntimeHostError::Storage(rejected.error));
    }
    let native_class = object
        .try_borrow()
        .map_err(|_| borrowed("world component allocation class"))?
        .kind
        .native_class_name();
    crate::object_slot::observe_object(object, native_class);
    let owner = EntityOwner::from_ref(object);
    let previous = component
        .try_borrow_mut()
        .map_err(|_| borrowed("assign world component owner"))?
        .replace_owner(Some(&owner));
    if let Some(previous) = previous {
        retired.push_owner(previous);
    }
    supported_hook(kind)?;
    object
        .try_borrow_mut()
        .map_err(|_| borrowed("world component flag"))?
        .has_components = true;
    Ok(true)
}

/// Constructor transfer keeps the exact FluidContainer and its final Color.
pub fn fluid_from_item_before_world(
    item: &ItemRef,
    object: &IsoObjectRef,
    retired: &mut RetiredComponents,
) -> HostResult<()> {
    protected_transfer(fluid_from_item_data(item, object, retired))
}

fn protected_transfer(result: HostResult<()>) -> HostResult<()> {
    let exception = match result {
        Err(RuntimeHostError::NativeException(error))
        | Err(RuntimeHostError::Storage(pz_entity::RuntimeError::Codec(error))) => error,
        result => return result,
    };
    pz_inventory::component_host::emit_native_notices(&[
        pz_entity::runtime_fluid::RuntimeNativeNotice::ExceptionLogger(exception.to_string()),
    ])
}

fn fluid_from_item_data(
    item: &ItemRef,
    object: &IsoObjectRef,
    retired: &mut RetiredComponents,
) -> HostResult<()> {
    retired.push_owner(EntityOwner::from_ref(item));
    retired.push_owner(EntityOwner::from_ref(object));
    let incoming = pz_inventory::component_host::detach_component_for_transfer(
        item,
        ComponentType::FluidContainer,
        retired,
    )?;
    let Some(incoming) = incoming else {
        return Ok(());
    };
    let old = object
        .try_borrow()
        .map_err(|_| borrowed("world transfer destination"))?
        .components
        .get_ref(ComponentType::FluidContainer)
        .map_err(RuntimeHostError::Storage)?;
    if let Some(old) = old {
        if let Some(detached) = remove_world_component(object, &old, retired)? {
            release_detached(&detached, retired)?;
        }
    }
    add_before_world(object, &incoming, retired)?;
    connect_world(object)
}

/// removeFromSquare transfer: the item still retains this world owner until
/// a separate explicit native setWorldItem(null) caller actually clears it.
pub fn fluid_to_item(
    object: &IsoObjectRef,
    item: &ItemRef,
    retired: &mut RetiredComponents,
) -> HostResult<()> {
    protected_transfer(fluid_to_item_data(object, item, retired))
}

fn fluid_to_item_data(
    object: &IsoObjectRef,
    item: &ItemRef,
    retired: &mut RetiredComponents,
) -> HostResult<()> {
    retired.push_owner(EntityOwner::from_ref(object));
    retired.push_owner(EntityOwner::from_ref(item));
    let incoming = object
        .try_borrow()
        .map_err(|_| borrowed("world transfer source"))?
        .components
        .get_ref(ComponentType::FluidContainer)
        .map_err(RuntimeHostError::Storage)?;
    let Some(incoming) = incoming else {
        return Ok(());
    };
    remove_world_component(object, &incoming, retired)?;
    let old = pz_inventory::component_host::detach_component_for_transfer(
        item,
        ComponentType::FluidContainer,
        retired,
    )?;
    if let Some(old) = old {
        release_detached(&old, retired)?;
    }
    // The floor item's native GameEntity hook is the authenticated empty base
    // implementation. It has not become equipped merely by returning fluid.
    let insertion = item
        .try_borrow_mut()
        .map_err(|_| borrowed("floor item fluid membership"))?
        .components
        .runtime_mut()
        .push_ref_if_absent(incoming.clone());
    if let Err(rejected) = insertion {
        retired.push_component(rejected.component);
        return Err(RuntimeHostError::Storage(rejected.error));
    }
    let prior = incoming
        .try_borrow_mut()
        .map_err(|_| borrowed("floor item fluid owner"))?
        .replace_owner(Some(&EntityOwner::from_ref(item)));
    if let Some(prior) = prior {
        retired.push_owner(prior);
    }
    pz_inventory::shared::observe_item(item);
    let mut index = 0;
    loop {
        let component = {
            let body = item
                .try_borrow()
                .map_err(|_| borrowed("floor item connect"))?;
            if index >= body.components.len() {
                break;
            }
            body.components
                .get_ref_at(index)
                .map_err(RuntimeHostError::Storage)?
                .ok_or(RuntimeHostError::InvalidHostState(
                    "missing floor item component index",
                ))?
        };
        supported_hook(
            component
                .try_borrow()
                .map_err(|_| borrowed("floor item connect body"))?
                .kind(),
        )?;
        index += 1;
    }
    Ok(())
}

/// The real external phases that precede swapItem's backlink assignments.
/// No default implementation silently replaces process/sound systems.
pub trait SwapPrefixHost {
    fn remove_process_item(&mut self, item: &ItemRef) -> HostResult<()>;
    fn remove_item_sound(&mut self, item: &ItemRef) -> HostResult<()>;
    fn item_name(&mut self, item: &ItemRef) -> HostResult<String>;
}

/// Owned actual receiver continuation. New-fluid transfer, process registries,
/// updateSprite and OnContainerUpdate remain explicit required tail phases;
/// returning this prefix is not a successful complete native swapItem.
#[derive(Debug)]
#[must_use]
pub struct SwapContinuation {
    pub object: IsoObjectRef,
    pub item: ItemRef,
}

pub fn swap_item_prefix(
    object: &IsoObjectRef,
    incoming: Option<&ItemRef>,
    host: &mut impl SwapPrefixHost,
    retired: &mut RetiredComponents,
) -> HostResult<Option<SwapContinuation>> {
    let Some(incoming) = incoming else {
        return Ok(None);
    };
    retired.push_owner(EntityOwner::from_ref(object));
    retired.push_owner(EntityOwner::from_ref(incoming));
    let old = item_ref(object)?;
    if let Some(old) = old {
        retired.push_owner(EntityOwner::from_ref(&old));
        if object
            .try_borrow()
            .map_err(|_| borrowed("swap old world fluid check"))?
            .components
            .runtime()
            .presence(ComponentType::FluidContainer)
            != pz_entity::EntryPresence::Absent
        {
            fluid_to_item(object, &old, retired)?;
        }
        host.remove_process_item(&old)?;
        host.remove_item_sound(&old)?;
        old.try_borrow_mut()
            .map_err(|_| borrowed("swap old world link clear"))?
            .replace_world_item(None, retired);
        pz_inventory::shared::observe_item(&old);
        let values = {
            let old = old
                .try_borrow()
                .map_err(|_| borrowed("swap old copied scalars"))?;
            (
                old.id,
                old.world_scale,
                old.world_z_rotation,
                old.world_y_rotation,
                old.world_x_rotation,
            )
        };
        let mut new = incoming
            .try_borrow_mut()
            .map_err(|_| borrowed("swap incoming copied scalars"))?;
        new.id = values.0;
        new.world_scale = values.1;
        new.world_z_rotation = values.2;
        new.world_y_rotation = values.3;
        new.world_x_rotation = values.4;
    }
    let floor = floor_ref(object)?;
    let displaced = floor
        .try_borrow_mut()
        .map_err(|_| borrowed("swap native item assignment"))?
        .item
        .replace_item(Some(incoming));
    retired.push_payload(Box::new(displaced));
    crate::object_slot::observe_object(object, "zombie.iso.objects.IsoWorldInventoryObject");
    // Native checks only AFTER the actual item field has changed.
    if incoming
        .try_borrow()
        .map_err(|_| borrowed("swap incoming world link check"))?
        .world_item_slot()
        .is_some()
    {
        return Err(RuntimeHostError::NativeException(pz_entity::EntityError(
            "IllegalArgumentException: newItem.getWorldItem() != null".into(),
        )));
    }
    publish_item_backlink(object, incoming, retired)?;
    let key = incoming
        .try_borrow()
        .map_err(|_| borrowed("swap new key id"))?
        .get_key_id();
    object
        .try_borrow_mut()
        .map_err(|_| borrowed("swap world key id"))?
        .key_id = key;
    let name = host.item_name(incoming)?;
    object
        .try_borrow_mut()
        .map_err(|_| borrowed("swap world name"))?
        .name = Some(crate::objects::ObjectName::Str(name));
    Ok(Some(SwapContinuation {
        object: object.clone(),
        item: incoming.clone(),
    }))
}

#[cfg(test)]
mod tests;
