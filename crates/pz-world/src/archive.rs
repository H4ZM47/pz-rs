//! Explicit internal archive copies. These are not runtime aliases or native
//! save/reload operations. Copying does not include pending host retirements.
use std::collections::HashMap;

use pz_inventory::archive::ArchiveCopyCheck;

use crate::{IsoGridSquare, IsoObjectRef, ObjectKind, ObjectSlot, SquareRef, SquareSlot};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ArchiveLocation {
    Object(usize),
    Corpse(usize),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SquareArchiveError {
    BorrowedSquare,
    BorrowedObject(usize),
    BorrowedFloor(usize),
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

/// Copies a retained square allocation into an independent owning slot.
pub fn try_archive_copy_square(square: &SquareRef) -> Result<SquareSlot, SquareArchiveError> {
    let square = square
        .try_borrow()
        .map_err(|_| SquareArchiveError::BorrowedSquare)?;
    square.try_archive_copy().map(SquareSlot::new)
}

pub(crate) fn copy_objects(square: &IsoGridSquare) -> Result<Vec<ObjectSlot>, SquareArchiveError> {
    let mut check = ArchiveCopyCheck::default();
    for (index, slot) in square.objects.iter().enumerate() {
        let object = slot.ref_handle();
        let object = object
            .try_borrow()
            .map_err(|_| SquareArchiveError::BorrowedObject(index))?;
        let location = ArchiveLocation::Object(index);
        let inventory_error = |cause| SquareArchiveError::Inventory { location, cause };
        for container in object.containers() {
            check.container(container).map_err(inventory_error)?;
        }
        check
            .optional_table(&object.mod_data)
            .map_err(inventory_error)?;
        match &object.kind {
            ObjectKind::WorldInventory(floor) => {
                let floor = floor.ref_handle();
                let floor = floor
                    .try_borrow()
                    .map_err(|_| SquareArchiveError::BorrowedFloor(index))?;
                if let Some(item) = floor.item.item_ref() {
                    check.item_ref(&item).map_err(inventory_error)?;
                }
            }
            ObjectKind::Mannequin(Some(mannequin)) => {
                if let Some(container) = &mannequin.container {
                    check.container(container).map_err(inventory_error)?;
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
            | ObjectKind::Mannequin(None)
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

    // No callbacks, source writes, or thread handoff occur between validation
    // and Clone. All nested shared owners are therefore still readable. The
    // leaf clones omit opaque parent/source-grid links and invoke no user code.
    // Each distinct whole object is copied once; equal bodies/IDs do not merge.
    let mut objects: HashMap<usize, IsoObjectRef> = HashMap::new();
    let mut result = Vec::with_capacity(square.objects.len());
    for slot in &square.objects {
        let source = slot.ref_handle();
        let key = std::rc::Rc::as_ptr(&source) as usize;
        let copied = objects
            .entry(key)
            .or_insert_with(|| ObjectSlot::new(source.borrow().clone()).ref_handle());
        result.push(ObjectSlot::retain(copied));
    }
    Ok(result)
}

#[cfg(test)]
mod tests;
