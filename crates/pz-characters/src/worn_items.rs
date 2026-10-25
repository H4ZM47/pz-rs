//! `zombie.characters.WornItems`: `BodyLocations`, `BodyLocationGroup`,
//! `BodyLocation`, `WornItem` and `WornItems`.
//!
//! Locations are `ItemBodyLocation` registry keys (compared by location).
//! Worn items are any [`ItemIdentity`] (`InventoryItem` is not ported);
//! TODO(InventoryItem): `setFromItemVisuals`, `getItemVisuals`,
//! `addItemsToItemContainer`, `getItemById`, `save`/`load`.

use std::cell::RefCell;
use std::rc::Rc;

use pz_scripts::registry::RegistryKey;
use pz_scripts::{ScriptError, ScriptResult};

use crate::{index_out_of_bounds, npe};

type Key = Option<Rc<RegistryKey>>;

fn same_key(a: Option<&Rc<RegistryKey>>, b: Option<&Rc<RegistryKey>>) -> bool {
    match (a, b) {
        (Some(a), Some(b)) => a.location == b.location,
        (None, None) => true,
        _ => false,
    }
}

fn contains(list: &[Key], k: Option<&Rc<RegistryKey>>) -> bool {
    list.iter().any(|e| same_key(e.as_ref(), k))
}

/// `BodyLocation` (the original's unused `group` back-reference is not kept).
#[derive(Clone, Debug)]
pub struct BodyLocation {
    pub id: Key,
    pub exclusive: Vec<Key>,
    pub hide_model: Vec<Key>,
    pub alt_model: Vec<Key>,
    pub multi_item: bool,
}

impl BodyLocation {
    pub fn new(id: Key) -> Self {
        BodyLocation { id, exclusive: Vec::new(), hide_model: Vec::new(), alt_model: Vec::new(), multi_item: false }
    }

    /// `setExclusive`.
    pub fn set_exclusive(&mut self, k: Key) {
        if !contains(&self.exclusive, k.as_ref()) {
            self.exclusive.push(k);
        }
    }

    /// `setHideModel`.
    pub fn set_hide_model(&mut self, k: Key) {
        if !contains(&self.hide_model, k.as_ref()) {
            self.hide_model.push(k);
        }
    }

    /// `setAltModel`.
    pub fn set_alt_model(&mut self, k: Key) {
        if !contains(&self.alt_model, k.as_ref()) {
            self.alt_model.push(k);
        }
    }

    /// `isExclusive`.
    pub fn is_exclusive(&self, k: Option<&Rc<RegistryKey>>) -> bool {
        contains(&self.exclusive, k)
    }

    /// `isHideModel`.
    pub fn is_hide_model(&self, k: Option<&Rc<RegistryKey>>) -> bool {
        contains(&self.hide_model, k)
    }

    /// `isAltModel`.
    pub fn is_alt_model(&self, k: Option<&Rc<RegistryKey>>) -> bool {
        contains(&self.alt_model, k)
    }

    /// `isId` (`id.equals(other)`; a null id is a `NullPointerException`).
    pub fn is_id(&self, k: Option<&Rc<RegistryKey>>) -> ScriptResult<bool> {
        let id = self.id.as_ref().ok_or_else(npe)?;
        Ok(k.is_some_and(|k| k.location == id.location))
    }
}

/// `BodyLocationGroup`.
#[derive(Clone, Debug)]
pub struct BodyLocationGroup {
    id: String,
    locations: Vec<BodyLocationRef>,
}

pub type BodyLocationGroupRef = Rc<RefCell<BodyLocationGroup>>;
pub type BodyLocationRef = Rc<RefCell<BodyLocation>>;

impl BodyLocationGroup {
    pub fn new(id: &str) -> ScriptResult<Self> {
        if id.is_empty() {
            return Err(ScriptError::new("java.lang.IllegalArgumentException: id is empty"));
        }
        Ok(BodyLocationGroup { id: id.to_string(), locations: Vec::new() })
    }

    /// `getId`.
    pub fn id(&self) -> &str {
        &self.id
    }

    /// `getAllLocations`.
    pub fn all_locations(&self) -> &[BodyLocationRef] {
        &self.locations
    }

    /// `size`.
    pub fn size(&self) -> usize {
        self.locations.len()
    }

    /// `indexOf`.
    pub fn index_of(&self, k: Option<&Rc<RegistryKey>>) -> ScriptResult<i32> {
        for (i, l) in self.locations.iter().enumerate() {
            if l.borrow().is_id(k)? {
                return Ok(i as i32);
            }
        }
        Ok(-1)
    }

    /// `getLocation`.
    pub fn location(&self, k: Option<&Rc<RegistryKey>>) -> ScriptResult<Option<BodyLocationRef>> {
        Ok(match self.index_of(k)? {
            -1 => None,
            i => Some(self.locations[i as usize].clone()),
        })
    }

    /// `getLocation`, where the original then dereferences the result.
    fn existing(&self, k: Option<&Rc<RegistryKey>>) -> ScriptResult<BodyLocationRef> {
        self.location(k)?.ok_or_else(npe)
    }

    /// `getLocationByIndex`.
    pub fn location_by_index(&self, index: i32) -> Option<BodyLocationRef> {
        usize::try_from(index).ok().and_then(|i| self.locations.get(i)).cloned()
    }

    /// `getOrCreateLocation`.
    pub fn get_or_create_location(&mut self, k: Key) -> ScriptResult<BodyLocationRef> {
        if let Some(l) = self.location(k.as_ref())? {
            return Ok(l);
        }
        let l = Rc::new(RefCell::new(BodyLocation::new(k)));
        self.locations.push(l.clone());
        Ok(l)
    }

    /// `moveLocationToIndex`.
    pub fn move_location_to_index(&mut self, k: Option<&Rc<RegistryKey>>, index: i32) -> ScriptResult {
        if index < 0 || index as usize >= self.locations.len() {
            return Ok(());
        }
        for i in 0..self.locations.len() {
            if self.locations[i].borrow().is_id(k)? {
                let l = self.locations.remove(i);
                self.locations.insert(index as usize, l);
            }
        }
        Ok(())
    }

    /// `setExclusive`.
    pub fn set_exclusive(&mut self, a: Key, b: Key) -> ScriptResult {
        let first = self.location(a.as_ref())?;
        let second = self.location(b.as_ref())?;
        first.ok_or_else(npe)?.borrow_mut().set_exclusive(b);
        second.ok_or_else(npe)?.borrow_mut().set_exclusive(a);
        Ok(())
    }

    /// `isExclusive`.
    pub fn is_exclusive(&self, a: Option<&Rc<RegistryKey>>, b: Option<&Rc<RegistryKey>>) -> ScriptResult<bool> {
        Ok(self.existing(a)?.borrow().is_exclusive(b))
    }

    /// `setHideModel`.
    pub fn set_hide_model(&mut self, a: Option<&Rc<RegistryKey>>, b: Key) -> ScriptResult {
        self.existing(a)?.borrow_mut().set_hide_model(b);
        Ok(())
    }

    /// `isHideModel`.
    pub fn is_hide_model(&self, a: Option<&Rc<RegistryKey>>, b: Option<&Rc<RegistryKey>>) -> ScriptResult<bool> {
        Ok(self.existing(a)?.borrow().is_hide_model(b))
    }

    /// `setAltModel`.
    pub fn set_alt_model(&mut self, a: Option<&Rc<RegistryKey>>, b: Key) -> ScriptResult {
        self.existing(a)?.borrow_mut().set_alt_model(b);
        Ok(())
    }

    /// `isAltModel`.
    pub fn is_alt_model(&self, a: Option<&Rc<RegistryKey>>, b: Option<&Rc<RegistryKey>>) -> ScriptResult<bool> {
        Ok(self.existing(a)?.borrow().is_alt_model(b))
    }

    /// `setMultiItem`.
    pub fn set_multi_item(&mut self, k: Option<&Rc<RegistryKey>>, multi: bool) -> ScriptResult {
        self.existing(k)?.borrow_mut().multi_item = multi;
        Ok(())
    }

    /// `isMultiItem`.
    pub fn is_multi_item(&self, k: Option<&Rc<RegistryKey>>) -> ScriptResult<bool> {
        Ok(self.existing(k)?.borrow().multi_item)
    }
}

/// `BodyLocations`: the groups, created on first request.
#[derive(Default)]
pub struct BodyLocations {
    groups: Vec<BodyLocationGroupRef>,
}

impl BodyLocations {
    /// `getGroup`.
    pub fn get_group(&mut self, id: &str) -> ScriptResult<BodyLocationGroupRef> {
        if let Some(g) = self.groups.iter().find(|g| g.borrow().id == id) {
            return Ok(g.clone());
        }
        let g = Rc::new(RefCell::new(BodyLocationGroup::new(id)?));
        self.groups.push(g.clone());
        Ok(g)
    }

    /// `reset`.
    pub fn reset(&mut self) {
        self.groups.clear();
    }

    /// `getAllGroups`.
    pub fn all_groups(&self) -> &[BodyLocationGroupRef] {
        &self.groups
    }
}

/// Identity of a worn item (Java compares items with `==`).
pub trait ItemIdentity: Clone {
    fn same(&self, other: &Self) -> bool;
}

/// Item ids (`InventoryItem.id`): the character code refers to the items
/// of an inventory by id.
impl ItemIdentity for i32 {
    fn same(&self, other: &Self) -> bool {
        self == other
    }
}

/// `WornItem`.
#[derive(Clone, Debug)]
pub struct WornItem<I> {
    pub location: Key,
    pub item: I,
}

/// `WornItems`.
#[derive(Clone, Debug)]
pub struct WornItems<I> {
    group: BodyLocationGroupRef,
    items: Vec<WornItem<I>>,
}

impl<I: ItemIdentity> WornItems<I> {
    pub fn new(group: BodyLocationGroupRef) -> Self {
        WornItems { group, items: Vec::new() }
    }

    /// `getBodyLocationGroup`.
    pub fn body_location_group(&self) -> &BodyLocationGroupRef {
        &self.group
    }

    /// `copyFrom`: both must use the same group.
    pub fn copy_from(&mut self, other: &WornItems<I>) -> ScriptResult {
        if !Rc::ptr_eq(&self.group, &other.group) {
            return Err(ScriptError::new(format!(
                "java.lang.RuntimeException: group={} other.group={}",
                self.group.borrow().id,
                other.group.borrow().id
            )));
        }
        self.items = other.items.clone();
        Ok(())
    }

    /// `get(index)`.
    pub fn get(&self, index: i32) -> ScriptResult<&WornItem<I>> {
        usize::try_from(index)
            .ok()
            .and_then(|i| self.items.get(i))
            .ok_or_else(|| index_out_of_bounds(index as i64, self.items.len()))
    }

    fn index_of_location(&self, k: Option<&Rc<RegistryKey>>) -> ScriptResult<Option<usize>> {
        for (i, w) in self.items.iter().enumerate() {
            let l = w.location.as_ref().ok_or_else(npe)?;
            if k.is_some_and(|k| k.location == l.location) {
                return Ok(Some(i));
            }
        }
        Ok(None)
    }

    fn index_of_item(&self, item: &I) -> Option<usize> {
        self.items.iter().position(|w| w.item.same(item))
    }

    /// `setItem`: replaces the item at a single-item location, drops items
    /// at exclusive locations, then inserts in the group's location order.
    pub fn set_item(&mut self, location: Key, item: Option<I>) -> ScriptResult {
        let group = self.group.clone();
        let group = group.borrow();
        if !group.is_multi_item(location.as_ref())? {
            if let Some(i) = self.index_of_location(location.as_ref())? {
                self.items.remove(i);
            }
        }
        let mut i = 0;
        while i < self.items.len() {
            if group.is_exclusive(location.as_ref(), self.items[i].location.as_ref())? {
                self.items.remove(i);
            } else {
                i += 1;
            }
        }
        let Some(item) = item else {
            return Ok(());
        };
        self.remove(&item);
        let at = group.index_of(location.as_ref())?;
        let mut insert_at = self.items.len();
        for (i, w) in self.items.iter().enumerate() {
            if group.index_of(w.location.as_ref())? > at {
                insert_at = i;
                break;
            }
        }
        self.items.insert(insert_at, WornItem { location, item });
        Ok(())
    }

    /// `getItem(location)`.
    pub fn item(&self, location: Option<&Rc<RegistryKey>>) -> ScriptResult<Option<I>> {
        Ok(self.index_of_location(location)?.map(|i| self.items[i].item.clone()))
    }

    /// `getItemByIndex`.
    pub fn item_by_index(&self, index: i32) -> Option<I> {
        usize::try_from(index).ok().and_then(|i| self.items.get(i)).map(|w| w.item.clone())
    }

    /// `remove(item)`.
    pub fn remove(&mut self, item: &I) {
        if let Some(i) = self.index_of_item(item) {
            self.items.remove(i);
        }
    }

    /// `clear`.
    pub fn clear(&mut self) {
        self.items.clear();
    }

    /// `getLocation(item)`.
    pub fn location(&self, item: &I) -> Key {
        self.index_of_item(item).and_then(|i| self.items[i].location.clone())
    }

    /// `contains`.
    pub fn contains(&self, item: &I) -> bool {
        self.index_of_item(item).is_some()
    }

    pub fn size(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// `getItems`.
    pub fn items(&self) -> &[WornItem<I>] {
        &self.items
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pz_scripts::registry::Registry;

    impl ItemIdentity for u32 {
        fn same(&self, other: &Self) -> bool {
            self == other
        }
    }

    #[test]
    fn worn_items_order_and_exclusion() {
        let mut reg = Registry::new("item_body_location");
        let [hat, shirt, coat, ring] = ["t:hat", "t:shirt", "t:coat", "t:ring"].map(|id| Some(reg.register_id(id, false).unwrap()));
        let mut locs = BodyLocations::default();
        let g = locs.get_group("Human").unwrap();
        assert!(Rc::ptr_eq(&g, &locs.get_group("Human").unwrap()));
        {
            let mut g = g.borrow_mut();
            for k in [&hat, &shirt, &coat, &ring] {
                g.get_or_create_location(k.clone()).unwrap();
            }
            g.set_exclusive(shirt.clone(), coat.clone()).unwrap();
            g.set_multi_item(ring.as_ref(), true).unwrap();
            assert!(g.is_exclusive(coat.as_ref(), shirt.as_ref()).unwrap());
        }
        let mut w = WornItems::new(g.clone());
        w.set_item(ring.clone(), Some(1)).unwrap();
        w.set_item(hat.clone(), Some(2)).unwrap();
        w.set_item(ring.clone(), Some(3)).unwrap();
        w.set_item(shirt.clone(), Some(4)).unwrap();
        let order: Vec<u32> = w.items().iter().map(|i| i.item).collect();
        assert_eq!(order, [2, 4, 1, 3]);
        w.set_item(coat.clone(), Some(5)).unwrap();
        assert!(!w.contains(&4) && w.contains(&5));
        w.set_item(hat.clone(), Some(5)).unwrap();
        assert_eq!(w.item(hat.as_ref()).unwrap(), Some(5));
        assert_eq!(w.item(coat.as_ref()).unwrap(), None);
        let unknown = Some(reg.register_id("t:unknown", false).unwrap());
        assert!(w.set_item(unknown, Some(9)).is_err());
        assert!(w.get(10).is_err());
        assert!(locs.get_group("").is_err());
    }
}
