//! Explicit internal archive copies. These are not runtime aliases or native
//! save/reload operations. Copying does not include pending host retirements.
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use pz_inventory::archive::ArchiveCopyCheck;

use crate::{IsoGridSquare, IsoObjectRef, ObjectKind, ObjectSlot, SquareRef, SquareSlot};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ArchiveLocation {
    Object(usize),
    Corpse(usize),
    WorldObject(usize),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SquareArchiveError {
    BorrowedSquare,
    BorrowedWorldObjects,
    BorrowedWorldObject(usize),
    BorrowedWorldFloor(usize),
    UnsupportedForeign {
        index: usize,
        kind: &'static str,
    },
    BorrowedObject(usize),
    BorrowedFloor(usize),
    BorrowedMannequinWorn(ArchiveLocation),
    Components {
        location: ArchiveLocation,
        cause: pz_entity::RuntimeError,
    },
    Metadata(pz_inventory::archive::ArchiveCopyError),
    Inventory {
        location: ArchiveLocation,
        cause: pz_inventory::archive::ArchiveCopyError,
    },
}

impl std::fmt::Display for SquareArchiveError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "square archive copy: {self:?}")
    }
}
impl std::error::Error for SquareArchiveError {}

/// Foreign VM values cannot be generically deep-cloned. Default copies reject
/// them. Explicit retention shares their exact original referents, while the
/// square, list, and known whole-object allocations remain independent.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ForeignArchivePolicy {
    #[default]
    RejectForeign,
    RetainForeignHandles,
}

/// Copies a retained square allocation using the default typed-only policy.
pub fn try_archive_copy_square(square: &SquareRef) -> Result<SquareSlot, SquareArchiveError> {
    try_archive_copy_square_with_policy(square, ForeignArchivePolicy::RejectForeign)
}
pub fn try_archive_copy_square_with_policy(
    square: &SquareRef,
    policy: ForeignArchivePolicy,
) -> Result<SquareSlot, SquareArchiveError> {
    let square = square
        .try_borrow()
        .map_err(|_| SquareArchiveError::BorrowedSquare)?;
    square
        .try_archive_copy_with_policy(policy)
        .map(SquareSlot::new)
}

#[derive(Clone, Copy)]
enum ObjectOrigin {
    Main(usize),
    World(usize),
}
impl ObjectOrigin {
    fn location(self) -> ArchiveLocation {
        match self {
            Self::Main(i) => ArchiveLocation::Object(i),
            Self::World(i) => ArchiveLocation::WorldObject(i),
        }
    }
    fn borrowed_object(self) -> SquareArchiveError {
        match self {
            Self::Main(i) => SquareArchiveError::BorrowedObject(i),
            Self::World(i) => SquareArchiveError::BorrowedWorldObject(i),
        }
    }
    fn borrowed_floor(self) -> SquareArchiveError {
        match self {
            Self::Main(i) => SquareArchiveError::BorrowedFloor(i),
            Self::World(i) => SquareArchiveError::BorrowedWorldFloor(i),
        }
    }
}

fn check_object(
    object: &IsoObjectRef,
    origin: ObjectOrigin,
    check: &mut ArchiveCopyCheck,
) -> Result<(), SquareArchiveError> {
    let object = object.try_borrow().map_err(|_| origin.borrowed_object())?;
    let location = origin.location();
    let inventory_error = |cause| SquareArchiveError::Inventory { location, cause };
    for (index, entry) in object.components.entry_slots().enumerate() {
        if let pz_entity::RuntimeEntryView::Live { kind, component } = entry {
            let body = component.try_borrow().map_err(|_| SquareArchiveError::Components {
                location,
                cause: pz_entity::RuntimeError::BorrowedComponent { index: Some(index), kind },
            })?;
            body.check_archive_copy().map_err(|cause| SquareArchiveError::Components { location, cause })?;
        }
    }
    for container in object.containers() {
        check.container(container).map_err(inventory_error)?;
    }
    check
        .optional_table(&object.mod_data)
        .map_err(inventory_error)?;
    match &object.kind {
        ObjectKind::WorldInventory(floor) => {
            let floor = floor.ref_handle();
            let floor = floor.try_borrow().map_err(|_| origin.borrowed_floor())?;
            if let Some(item) = floor.item.item_ref() {
                check.item_ref(&item).map_err(inventory_error)?;
            }
        }
        ObjectKind::Mannequin(mannequin) => {
            if let Some(container) = &mannequin.runtime_container {
                check.container_ref(container).map_err(inventory_error)?;
            } else if let Some(container) = &mannequin.container {
                check.container(container).map_err(inventory_error)?;
            }
            if let Some(worn) = &mannequin.runtime_worn_items {
                let worn = worn
                    .try_borrow()
                    .map_err(|_| SquareArchiveError::BorrowedMannequinWorn(location))?;
                for entry in worn.items() {
                    check.item_ref(&entry.item).map_err(inventory_error)?;
                }
            }
        }
        ObjectKind::Trap(trap) => {
            if let Some(item) = &trap.weapon {
                check.saved_item(item).map_err(inventory_error)?;
            }
        }
        ObjectKind::CarBatteryCharger(charger) => {
            for item in charger.item.iter().chain(charger.battery.iter()) {
                check.saved_item(item).map_err(inventory_error)?;
            }
        }
        ObjectKind::Hutch(hutch) => {
            if let Some(master) = &hutch.master {
                for (_, animal) in &master.animals {
                    check.table(&animal.mod_data).map_err(inventory_error)?;
                }
                for nest in &master.nest_boxes {
                    for egg in &nest.eggs {
                        check.saved_item(egg).map_err(inventory_error)?;
                    }
                    if let Some(animal) = &nest.animal {
                        check.table(&animal.mod_data).map_err(inventory_error)?;
                    }
                }
            }
        }
        ObjectKind::Thumpable(thumpable) => {
            for table in [
                &thumpable.table,
                &thumpable.mod_data,
                &thumpable.build_materials,
            ] {
                check.optional_table(table).map_err(inventory_error)?;
            }
        }
        ObjectKind::Pushable(pushable) => {
            check
                .optional_table(&pushable.base.mod_data)
                .map_err(inventory_error)?;
        }
        // These variants contain values/codec records, not live item or
        // container RefCells. Keep the match exhaustive when adding kinds.
        ObjectKind::Object
        | ObjectKind::Door(_)
        | ObjectKind::LightSwitch(_)
        | ObjectKind::Curtain(_)
        | ObjectKind::Window(_)
        | ObjectKind::WindowFrame { .. }
        | ObjectKind::Barbecue(_)
        | ObjectKind::Fireplace(_)
        | ObjectKind::CombinationWasherDryer(_)
        | ObjectKind::StackedWasherDryer(_)
        | ObjectKind::ClothingDryer(_)
        | ObjectKind::ClothingWasher(_)
        | ObjectKind::Stove(_)
        | ObjectKind::Jukebox
        | ObjectKind::Radio(_)
        | ObjectKind::Television(_)
        | ObjectKind::Tree(_)
        | ObjectKind::BrokenGlass
        | ObjectKind::ButcherHook
        | ObjectKind::Compost(_)
        | ObjectKind::FeedingTrough(_)
        | ObjectKind::Barricade(_)
        | ObjectKind::Generator(_)
        | ObjectKind::Fire(_)
        | ObjectKind::AnimalTrack(_) => {}
    }
    Ok(())
}

pub(crate) struct CopiedSquareObjects {
    pub objects: Vec<ObjectSlot>,
    pub world_objects: crate::WorldObjectsRef,
}

pub(crate) fn copy_objects(
    square: &IsoGridSquare,
    policy: ForeignArchivePolicy,
) -> Result<CopiedSquareObjects, SquareArchiveError> {
    // This temporary pin set is not a second authoritative list. No callbacks,
    // source writes or handoff occur before all validation and cloning finish.
    let entries = square
        .world_objects
        .try_borrow()
        .map_err(|_| SquareArchiveError::BorrowedWorldObjects)?
        .snapshot_entries();
    if policy == ForeignArchivePolicy::RejectForeign {
        for (index, entry) in entries.iter().enumerate() {
            if let crate::WorldObjectEntry::Foreign(value) = entry {
                return Err(SquareArchiveError::UnsupportedForeign {
                    index,
                    kind: value.type_name(),
                });
            }
        }
    }
    let mut known = Vec::new();
    let mut seen = HashSet::new();
    for (index, slot) in square.objects.iter().enumerate() {
        let owner = slot.ref_handle();
        if seen.insert(Rc::as_ptr(&owner) as usize) {
            known.push((owner, ObjectOrigin::Main(index)));
        }
    }
    for (index, entry) in entries.iter().enumerate() {
        if let crate::WorldObjectEntry::Object(owner) = entry {
            if seen.insert(Rc::as_ptr(owner) as usize) {
                known.push((Rc::clone(owner), ObjectOrigin::World(index)));
            }
        }
    }
    let mut check = ArchiveCopyCheck::default();
    for (owner, origin) in &known {
        check_object(owner, *origin, &mut check)?;
    }
    for (index, corpse) in square.static_moving.iter().enumerate() {
        check
            .optional_table(&corpse.base.mod_data)
            .map_err(|cause| SquareArchiveError::Inventory {
                location: ArchiveLocation::Corpse(index),
                cause,
            })?;
        if let Some(container) = &corpse.container {
            check
                .container(container)
                .map_err(|cause| SquareArchiveError::Inventory {
                    location: ArchiveLocation::Corpse(index),
                    cause,
                })?;
        }
    }
    check
        .optional_table(&square.mod_data)
        .map_err(SquareArchiveError::Metadata)?;

    // Each known actual owner is copied once across BOTH membership domains.
    // Existing per-object item/container value-tree semantics remain unchanged;
    // no generic VM clone or cross-object child-graph remapping is implied.
    let objects: HashMap<usize, IsoObjectRef> = known
        .into_iter()
        .map(|(source, _)| {
            let key = Rc::as_ptr(&source) as usize;
            (key, ObjectSlot::new(source.borrow().clone()).ref_handle())
        })
        .collect();
    let main = square
        .objects
        .iter()
        .map(|slot| ObjectSlot::retain(&objects[&(Rc::as_ptr(&slot.ref_handle()) as usize)]))
        .collect();
    let entries = entries
        .into_iter()
        .map(|entry| match entry {
            crate::WorldObjectEntry::Object(owner) => {
                crate::WorldObjectEntry::Object(Rc::clone(&objects[&(Rc::as_ptr(&owner) as usize)]))
            }
            // Default rejection was checked before allocation. Under the explicit
            // retention policy, opaque values still point into the source graph.
            foreign @ crate::WorldObjectEntry::Foreign(_) => foreign,
        })
        .collect();
    Ok(CopiedSquareObjects {
        objects: main,
        world_objects: Rc::new(std::cell::RefCell::new(crate::WorldObjects::from_entries(
            entries,
        ))),
    })
}

#[cfg(test)]
mod tests;
