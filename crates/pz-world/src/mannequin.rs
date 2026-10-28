//! `IsoMannequin` constructor state, separate from world activation.
//!
//! Native constructors allocate a visual and worn-item state immediately.
//! The Rust constructor leaves the worn owner pending until the host can
//! attach its actual Human body-location group before activation.
//! `addToWorld` later calls `initOutfit`; ordinary chunk saving neither calls
//! it nor rejects a constructor whose `init` flag is still false.

use crate::objects::IsoMannequin;
use crate::save::visual::HumanVisual;
use crate::IsoDirections;
use pz_inventory::archive::ArchiveCopyCheck;
use pz_inventory::io::{SaveError, SaveResult};
use pz_inventory::shared::{self, ContainerRef};
use pz_inventory::worn_items::InventoryWornItemsRef;
use std::cell::RefCell;
use std::rc::Rc;

/// Previous owners moved out of a mannequin. Drop after releasing object,
/// world and sprite guards; an actual inventory can own opaque host parents.
#[must_use]
pub struct MannequinInventoryRetirement {
    pub container: Option<ContainerRef>,
    pub worn_items: Option<InventoryWornItemsRef>,
    pub saved_container: Option<crate::objects::ItemContainer>,
    pub saved_worn_items: Vec<(String, i16)>,
}

impl IsoMannequin {
    /// Both native constructors use this state. `parts` comes from the real
    /// `BloodBodyPartType.MAX.index()` table (or an authored test table).
    pub fn new(parts: usize) -> Self {
        Self {
            dir: IsoDirections::N as i8,
            init: false,
            female: false,
            zombie: false,
            skeleton: false,
            mannequin_script_name: None,
            pose: None,
            human_visual: HumanVisual::new(parts),
            model_script_name: None,
            texture_name: None,
            anim_set: None,
            anim_state: None,
            outfit: None,
            item_visuals: Vec::new(),
            pending_loaded_skin_resolution: false,
            container: None,
            worn_items: Vec::new(),
            runtime_container: None,
            runtime_worn_items: None,
        }
    }

    /// Installs actual created clothing once. The old value fields cease to
    /// be a second inventory snapshot as soon as this owner is present.
    pub fn install_runtime_inventory(
        &mut self,
        container: ContainerRef,
        worn: InventoryWornItemsRef,
    ) -> MannequinInventoryRetirement {
        MannequinInventoryRetirement {
            container: self.runtime_container.replace(container),
            worn_items: self.runtime_worn_items.replace(worn),
            saved_container: self.container.take(),
            saved_worn_items: std::mem::take(&mut self.worn_items),
        }
    }

    /// A transient codec value, never installed back into the live inventory.
    /// Worn indices name the current live item positions; the writer further
    /// maps them through CompressIdenticalItems' actual saved-item order.
    pub fn save_inventory_snapshot(
        &self,
    ) -> SaveResult<Option<(crate::objects::ItemContainer, Vec<(String, i32)>)>> {
        let Some(actual) = &self.runtime_container else {
            if let Some(container) = &self.container {
                ArchiveCopyCheck::default()
                    .container(container)
                    .map_err(|e| SaveError(format!("IsoMannequin inventory snapshot: {e}")))?;
            }
            return Ok(self.container.clone().map(|c| {
                (
                    c,
                    self.worn_items
                        .iter()
                        .map(|(key, index)| (key.clone(), i32::from(*index)))
                        .collect(),
                )
            }));
        };
        ArchiveCopyCheck::default()
            .container_ref(actual)
            .map_err(|e| SaveError(format!("IsoMannequin inventory snapshot: {e}")))?;
        let container = actual
            .try_borrow()
            .map_err(|_| SaveError("IsoMannequin inventory snapshot: BorrowedContainer".into()))?;
        let worn = self
            .runtime_worn_items
            .as_ref()
            .ok_or_else(|| SaveError("IsoMannequin runtime inventory has no worn owner".into()))?;
        let worn = worn
            .try_borrow()
            .map_err(|_| SaveError("IsoMannequin inventory snapshot: BorrowedWornItems".into()))?;
        let worn = worn
            .items()
            .iter()
            .map(|entry| {
                let location = entry.location.as_ref().ok_or_else(|| {
                    SaveError("IsoMannequin worn location: java.lang.NullPointerException".into())
                })?;
                let index = container
                    .items
                    .iter()
                    .position(|item| Rc::ptr_eq(item, &entry.item));
                Ok((location.location.id(), index.map_or(-1, |i| i as i32)))
            })
            .collect::<SaveResult<Vec<_>>>()?;
        Ok(Some((container.to_container(), worn)))
    }
}

impl Clone for IsoMannequin {
    /// Archive/value copying retains no opaque parent/source-grid owners.
    /// Runtime worn references are mapped into the copied actual inventory.
    fn clone(&self) -> Self {
        let runtime_container = self
            .runtime_container
            .as_ref()
            .map(|container| container.borrow().deep_clone());
        let runtime_worn_items = self.runtime_worn_items.as_ref().map(|worn| {
            let copied = worn
                .borrow()
                .try_map_items(|item| {
                    let index = self.runtime_container.as_ref().and_then(|container| {
                        container
                            .borrow()
                            .items
                            .iter()
                            .position(|entry| Rc::ptr_eq(entry, item))
                    });
                    let copied = index
                        .and_then(|i| {
                            runtime_container
                                .as_ref()
                                .map(|c| c.borrow().items[i].clone())
                        })
                        .unwrap_or_else(|| shared::new_item(item.borrow().clone()));
                    Ok::<_, std::convert::Infallible>(copied)
                })
                .unwrap_or_else(|never| match never {});
            Rc::new(RefCell::new(copied))
        });
        Self {
            dir: self.dir,
            init: self.init,
            female: self.female,
            zombie: self.zombie,
            skeleton: self.skeleton,
            mannequin_script_name: self.mannequin_script_name.clone(),
            pose: self.pose.clone(),
            human_visual: self.human_visual.clone(),
            model_script_name: self.model_script_name.clone(),
            texture_name: self.texture_name.clone(),
            anim_set: self.anim_set.clone(),
            anim_state: self.anim_state.clone(),
            outfit: self.outfit.clone(),
            item_visuals: self.item_visuals.clone(),
            pending_loaded_skin_resolution: self.pending_loaded_skin_resolution,
            container: if runtime_container.is_some() {
                None
            } else {
                self.container.clone()
            },
            worn_items: if runtime_container.is_some() {
                Vec::new()
            } else {
                self.worn_items.clone()
            },
            runtime_container,
            runtime_worn_items,
        }
    }
}

impl PartialEq for IsoMannequin {
    /// Value/archive equality. The canonical body-location group is a shared
    /// registry dependency; worn membership and inventory state compare by
    /// value, rather than making a deep archive unequal just for new owners.
    fn eq(&self, other: &Self) -> bool {
        let worn_equal = match (&self.runtime_worn_items, &other.runtime_worn_items) {
            (None, None) => true,
            (Some(a), Some(b)) => {
                let a = a.borrow();
                let b = b.borrow();
                Rc::ptr_eq(a.body_location_group(), b.body_location_group())
                    && a.size() == b.size()
                    && a.items()
                        .iter()
                        .zip(b.items())
                        .all(|(a, b)| a.location == b.location && a.item == b.item)
            }
            _ => false,
        };
        worn_equal
            && self.dir == other.dir
            && self.init == other.init
            && self.female == other.female
            && self.zombie == other.zombie
            && self.skeleton == other.skeleton
            && self.mannequin_script_name == other.mannequin_script_name
            && self.pose == other.pose
            && self.human_visual == other.human_visual
            && self.model_script_name == other.model_script_name
            && self.texture_name == other.texture_name
            && self.anim_set == other.anim_set
            && self.anim_state == other.anim_state
            && self.outfit == other.outfit
            && self.item_visuals == other.item_visuals
            && self.pending_loaded_skin_resolution == other.pending_loaded_skin_resolution
            && self.container == other.container
            && self.worn_items == other.worn_items
            && self.runtime_container == other.runtime_container
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pz_inventory::shared::ItemRef;
    use pz_inventory::worn_items::{BodyLocationGroup, WornItems};
    use pz_scripts::registry::RegistryKey;

    fn worn_items(entries: Vec<(Rc<RegistryKey>, ItemRef)>) -> InventoryWornItemsRef {
        let group = Rc::new(RefCell::new(BodyLocationGroup::new("Human").unwrap()));
        for (location, _) in &entries {
            group
                .borrow_mut()
                .get_or_create_location(Some(location.clone()))
                .unwrap();
        }
        let mut worn = WornItems::new(group);
        for (location, item) in entries {
            worn.set_item(Some(location), Some(item)).unwrap();
        }
        Rc::new(RefCell::new(worn))
    }

    #[test]
    fn constructor_allocates_real_visual_but_does_not_initialize_an_outfit() {
        let mannequin = IsoMannequin::new(7);
        // The installed IsoDirections.N constant has ordinal zero, and the
        // IsoObject constructor assigns it before the subclass constructor.
        assert_eq!(mannequin.dir, 0);
        assert!(!mannequin.init && !mannequin.female);
        assert!(!mannequin.zombie && !mannequin.skeleton);
        assert_eq!(mannequin.human_visual, HumanVisual::new(7));
        assert_eq!(mannequin.human_visual.blood.len(), 7);
        assert_eq!(mannequin.human_visual.dirt.len(), 7);
        assert_eq!(mannequin.human_visual.holes.len(), 7);
        assert!(mannequin.mannequin_script_name.is_none());
        assert!(mannequin.pose.is_none());
        assert!(mannequin.model_script_name.is_none());
        assert!(mannequin.texture_name.is_none());
        assert!(mannequin.anim_set.is_none() && mannequin.anim_state.is_none());
        assert!(mannequin.outfit.is_none());
        assert!(mannequin.item_visuals.is_empty());
        assert!(mannequin.container.is_none());
        assert!(mannequin.worn_items.is_empty());
        assert!(mannequin.runtime_worn_items.is_none());
    }

    #[test]
    fn archive_clone_rebinds_worn_owners_and_copies_nested_inventory() {
        use pz_inventory::{InventoryItem, ItemClass, ItemTemplate};
        use pz_scripts::registry::ResourceLocation;
        let container = shared::SharedContainer::new(4, "mannequin").into_ref();
        let bag = shared::new_item(InventoryItem::create(
            Rc::new(ItemTemplate::new(ItemClass::Container, "Base", "Bag")),
            1,
        ));
        let child = shared::new_item(InventoryItem::create(
            Rc::new(ItemTemplate::new(ItemClass::Normal, "Base", "Child")),
            2,
        ));
        shared::add_item(&shared::inventory_of(&bag.borrow()).unwrap(), &child);
        shared::add_item(&container, &bag);
        let location = Rc::new(RegistryKey {
            location: ResourceLocation::of("base:back").unwrap(),
            name: "Back".into(),
            item_key: None,
        });
        let mut original = IsoMannequin::new(7);
        drop(original.install_runtime_inventory(
            container.clone(),
            worn_items(vec![(location, bag.clone())]),
        ));
        let copy = original.clone();
        let copied = copy.runtime_container.as_ref().unwrap();
        assert!(!Rc::ptr_eq(&container, copied));
        let copied_bag = copied.borrow().items[0].clone();
        assert!(!Rc::ptr_eq(&bag, &copied_bag));
        let original_worn = original.runtime_worn_items.as_ref().unwrap();
        let copied_worn = copy.runtime_worn_items.as_ref().unwrap();
        assert!(!Rc::ptr_eq(original_worn, copied_worn));
        assert!(Rc::ptr_eq(
            original_worn.borrow().body_location_group(),
            copied_worn.borrow().body_location_group()
        ));
        assert!(Rc::ptr_eq(
            &copied_worn.borrow().items()[0].item,
            &copied_bag
        ));
        assert_eq!(original, copy);
        let copied_contents = shared::inventory_of(&copied_bag.borrow()).unwrap();
        assert!(!Rc::ptr_eq(&child, &copied_contents.borrow().items[0]));
        child.borrow_mut().favorite = true;
        assert!(!copied_contents.borrow().items[0].borrow().favorite);
        assert!(copy.container.is_none() && copy.worn_items.is_empty());
    }

    #[test]
    fn replaced_inventory_owners_drop_after_mannequin_and_host_guards() {
        use std::cell::{Cell, RefCell};
        struct Probe {
            host: Rc<RefCell<()>>,
            drops: Rc<Cell<usize>>,
        }
        impl Drop for Probe {
            fn drop(&mut self) {
                let _guard = self
                    .host
                    .try_borrow_mut()
                    .expect("inventory retired under host guard");
                self.drops.set(self.drops.get() + 1);
            }
        }
        let host = Rc::new(RefCell::new(()));
        let drops = Rc::new(Cell::new(0));
        let container = shared::SharedContainer::new(4, "mannequin").into_ref();
        container.borrow_mut().parent = Some(Rc::new(Probe {
            host: host.clone(),
            drops: drops.clone(),
        }));
        let mannequin = RefCell::new(IsoMannequin::new(7));
        drop(
            mannequin
                .borrow_mut()
                .install_runtime_inventory(container, worn_items(Vec::new())),
        );
        let host_guard = host.borrow_mut();
        let mut guard = mannequin.borrow_mut();
        let retired = guard.install_runtime_inventory(
            shared::SharedContainer::new(5, "mannequin").into_ref(),
            worn_items(Vec::new()),
        );
        assert_eq!(drops.get(), 0);
        drop(guard);
        drop(host_guard);
        drop(retired);
        assert_eq!(drops.get(), 1);
    }

    #[test]
    fn square_archive_rejects_borrowed_actual_worn_collection_before_copying() {
        use crate::archive::{ArchiveLocation, SquareArchiveError};
        use crate::{IsoGridSquare, IsoObject, ObjectKind, ObjectSlot, SquareSlot};
        let worn = worn_items(Vec::new());
        let mut mannequin = IsoMannequin::new(7);
        drop(mannequin.install_runtime_inventory(
            shared::SharedContainer::new(4, "mannequin").into_ref(),
            worn.clone(),
        ));
        let mut square = IsoGridSquare::new(1, 2, 0);
        square.objects.push(ObjectSlot::new(IsoObject::new(
            1,
            ObjectKind::Mannequin(Box::new(mannequin)),
            None,
        )));
        let square = SquareSlot::new(square);
        let guard = worn.borrow_mut();
        assert_eq!(
            square.try_archive_copy().unwrap_err(),
            SquareArchiveError::BorrowedMannequinWorn(ArchiveLocation::Object(0))
        );
        drop(guard);
        assert!(square.try_archive_copy().is_ok());
    }
}
