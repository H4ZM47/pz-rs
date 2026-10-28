//! Actual designation class roots and their single-player host. Class-list
//! allocations survive world/menu resets; native Reset clears their contents.
use crate::iso_world::IsoWorld;
use pz_entity::runtime_load::{HostResult, RuntimeHostError};
use pz_inventory::component_host::retire_components;
use pz_lua_api::bind_designation::{self, DesignationContext, DesignationHostRef};
use pz_world::designation::{
    self, DesignationCorner, DesignationHost, DesignationRegistryRef, DesignationZoneRef,
};
use std::cell::RefCell;
use std::rc::Rc;

struct NativeClassRoots {
    registry: DesignationRegistryRef,
    host: DesignationHostRef,
}
thread_local! {
    // These are the actual native class/static owners, not a lookup cache or
    // a projection of MetaWorld's saved DTOs. Neither load nor exit replaces it.
    static CLASSES: RefCell<Option<NativeClassRoots>> = const { RefCell::new(None) };
}
fn unsupported(operation: &'static str) -> RuntimeHostError {
    RuntimeHostError::Unsupported {
        operation,
        kind: None,
    }
}

pub(crate) struct ActualDesignationHost;
impl DesignationHost for ActualDesignationHost {
    fn rand_next(&mut self, bound: i32) -> HostResult<i32> {
        Ok(pz_core::random::next(bound))
    }
    fn current_cell_present(&mut self) -> HostResult<bool> {
        pz_lua_api::bind_world::try_with_world_read(|world| Ok(world.cell.is_some()))
    }
    fn corner_square(
        &mut self,
        corner: DesignationCorner<'_>,
    ) -> HostResult<Option<pz_world::SquareRef>> {
        pz_lua_api::bind_world::try_with_world_read(|world| {
            // Native gets currentCell BEFORE this corner's zone field reads.
            let cell = world.cell.as_ref().ok_or_else(|| {
                unsupported("native designation corner null currentCell diagnostic")
            })?;
            let (x, y, z) = corner.read_coordinates()?;
            Ok(cell.get_grid_square(x, y, z))
        })
    }
    fn sync_zone(&mut self, zone: &DesignationZoneRef) -> HostResult<()> {
        // SyncZonePacket's pure constructor precedes ArrayList.contains.
        // Its Designation setData branch retains this same zone and the
        // membership flag, without reading any zone body fields.
        let mut packet = (None, false);
        let (registry, _) = actual_class_roots()?;
        let list = registry
            .try_borrow()
            .map_err(|_| RuntimeHostError::Borrowed("designation sync registry"))?
            .all_zones_slot()
            .cloned()
            .ok_or_else(|| unsupported("designation sync null allZones"))?;
        let contains = {
            let list = list
                .try_borrow()
                .map_err(|_| RuntimeHostError::Borrowed("designation sync allZones.contains"))?;
            let designation::DesignationList::Zones(zones) = &*list else {
                return Err(unsupported("designation sync allZones list type"));
            };
            zones.iter().any(|entry| Rc::ptr_eq(entry, zone))
        };
        packet.0 = Some(zone.clone());
        packet.1 = contains;
        let core = pz_lua_api::bind_core::shared_core();
        if core
            .try_borrow()
            .map_err(|_| RuntimeHostError::Borrowed("designation sync client role"))?
            .is_client
        {
            return Err(unsupported("actual designation client synchronization"));
        }
        if core
            .try_borrow()
            .map_err(|_| RuntimeHostError::Borrowed("designation sync server role"))?
            .is_server
        {
            return Err(unsupported("actual designation server synchronization"));
        }
        drop(packet);
        Ok(())
    }
}
fn actual_class_roots() -> HostResult<(DesignationRegistryRef, DesignationHostRef)> {
    let existing = CLASSES.with(|slot| {
        slot.try_borrow()
            .map_err(|_| RuntimeHostError::Borrowed("designation class roots"))
            .map(|roots| roots.as_ref().map(|r| (r.registry.clone(), r.host.clone())))
    })?;
    if let Some(roots) = existing {
        return Ok(roots);
    }
    pz_lua_api::component_gc::install_designation_observer();
    let registry = designation::new_registry();
    let host: DesignationHostRef = Rc::new(RefCell::new(Box::new(ActualDesignationHost)));
    CLASSES.with(|slot| {
        let mut roots = slot
            .try_borrow_mut()
            .map_err(|_| RuntimeHostError::Borrowed("designation class roots publication"))?;
        if roots.is_some() {
            return Err(RuntimeHostError::InvalidHostState(
                "reentrant designation class initialization",
            ));
        }
        *roots = Some(NativeClassRoots {
            registry: registry.clone(),
            host: host.clone(),
        });
        Ok(())
    })?;
    Ok((registry, host))
}

/// Make native class statics available before shipped menu Lua can call them.
/// This also reinstalls weak access after Lua bootstrap without resetting data.
pub(crate) fn install_actual_designation_classes() -> HostResult<DesignationRegistryRef> {
    let (registry, host) = actual_class_roots()?;
    let previous =
        bind_designation::install_context(Some(DesignationContext::new(&registry, &host)));
    drop(previous);
    Ok(registry)
}

impl IsoWorld {
    pub(crate) fn init_designation_registry(&mut self) -> Result<(), String> {
        let registry = install_actual_designation_classes()
            .map_err(|e| format!("designation class initialization: {e:?}"))?;
        self.meta_world.designation_zones = pz_world::meta_save::DesignationState::Live(registry);
        Ok(())
    }
    pub(crate) fn reset_designation_registry(&mut self) -> HostResult<()> {
        let registry = self
            .meta_world
            .designation_zones
            .live_slot()
            .cloned()
            .ok_or_else(|| unsupported("actual designation reset archive profile"))?;
        let mut retired = pz_entity::RetiredComponents::new();
        let result = (|| {
            designation::try_reset_base(&registry, &mut retired)?;
            designation::try_reset_animal(&registry, &mut retired)
        })();
        retire_components(retired);
        result
    }
}

#[cfg(test)]
mod tests;
