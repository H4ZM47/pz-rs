//! `zombie.characters.AttachedItems`: `AttachedLocations`,
//! `AttachedLocationGroup`, `AttachedLocation`, `AttachedItem` and
//! `AttachedItems`.
//!
//! Locations are plain string ids that the Lua scripts declare. Attached
//! items are any [`ItemIdentity`] (`InventoryItem` is not ported).

use std::cell::RefCell;
use std::rc::Rc;

use pz_scripts::{ScriptError, ScriptResult};

use crate::worn_items::ItemIdentity;
use crate::{index_out_of_bounds, npe};

fn npe_msg(m: &str) -> ScriptError {
    ScriptError::new(format!("java.lang.NullPointerException: {m}"))
}

fn illegal(m: &str) -> ScriptError {
    ScriptError::new(format!("java.lang.IllegalArgumentException: {m}"))
}

/// The constructors' `id` checks: null, then empty.
fn check_id(id: Option<&str>, what: &str) -> ScriptResult<String> {
    match id {
        None => Err(npe_msg(&format!("{what} is null"))),
        Some("") => Err(illegal(&format!("{what} is empty"))),
        Some(id) => Ok(id.to_string()),
    }
}

/// `AttachedLocation` (the original's unused `group` back-reference is not
/// kept).
#[derive(Clone, Debug)]
pub struct AttachedLocation {
    id: String,
    attachment_name: Option<String>,
}

pub type AttachedLocationRef = Rc<RefCell<AttachedLocation>>;

impl AttachedLocation {
    /// `new AttachedLocation(group, id)`.
    pub fn new(id: Option<&str>) -> ScriptResult<Self> {
        Ok(AttachedLocation { id: check_id(id, "id")?, attachment_name: None })
    }

    /// `getId`.
    pub fn id(&self) -> &str {
        &self.id
    }

    /// `setAttachmentName`. The original validates `id` (never null or
    /// empty) rather than the argument, so any name, `null` included, is
    /// stored.
    pub fn set_attachment_name(&mut self, name: Option<&str>) {
        self.attachment_name = name.map(str::to_string);
    }

    /// `getAttachmentName`.
    pub fn attachment_name(&self) -> Option<&str> {
        self.attachment_name.as_deref()
    }
}

/// `AttachedLocationGroup`.
#[derive(Clone, Debug)]
pub struct AttachedLocationGroup {
    id: String,
    locations: Vec<AttachedLocationRef>,
}

pub type AttachedLocationGroupRef = Rc<RefCell<AttachedLocationGroup>>;

impl AttachedLocationGroup {
    /// `new AttachedLocationGroup(id)`.
    pub fn new(id: Option<&str>) -> ScriptResult<Self> {
        Ok(AttachedLocationGroup { id: check_id(id, "id")?, locations: Vec::new() })
    }

    /// The `id` field.
    pub fn id(&self) -> &str {
        &self.id
    }

    /// The `locations` field, in declaration order.
    pub fn locations(&self) -> &[AttachedLocationRef] {
        &self.locations
    }

    /// `getLocation` (`null` matches nothing).
    pub fn location(&self, id: Option<&str>) -> Option<AttachedLocationRef> {
        let id = id?;
        self.locations.iter().find(|l| l.borrow().id == id).cloned()
    }

    /// `getOrCreateLocation`.
    pub fn get_or_create_location(&mut self, id: Option<&str>) -> ScriptResult<AttachedLocationRef> {
        if let Some(l) = self.location(id) {
            return Ok(l);
        }
        let l = Rc::new(RefCell::new(AttachedLocation::new(id)?));
        self.locations.push(l.clone());
        Ok(l)
    }

    /// `getLocationByIndex`.
    pub fn location_by_index(&self, index: i32) -> Option<AttachedLocationRef> {
        usize::try_from(index).ok().and_then(|i| self.locations.get(i)).cloned()
    }

    /// `size`.
    pub fn size(&self) -> usize {
        self.locations.len()
    }

    /// `indexOf`.
    pub fn index_of(&self, id: Option<&str>) -> i32 {
        id.and_then(|id| self.locations.iter().position(|l| l.borrow().id == id)).map_or(-1, |i| i as i32)
    }

    /// `checkValid`.
    pub fn check_valid(&self, id: Option<&str>) -> ScriptResult {
        match id {
            None => Err(npe_msg("locationId is null")),
            Some("") => Err(illegal("locationId is empty")),
            Some(l) if self.index_of(id) == -1 => {
                Err(ScriptError::new(format!("java.lang.RuntimeException: no such location \"{l}\"")))
            }
            Some(_) => Ok(()),
        }
    }
}

/// `AttachedLocations`: the groups, created on first request.
#[derive(Default)]
pub struct AttachedLocations {
    groups: Vec<AttachedLocationGroupRef>,
}

impl AttachedLocations {
    /// `getGroup`.
    pub fn get_group(&mut self, id: Option<&str>) -> ScriptResult<AttachedLocationGroupRef> {
        if let Some(g) = self.groups.iter().find(|g| Some(g.borrow().id.as_str()) == id) {
            return Ok(g.clone());
        }
        let g = Rc::new(RefCell::new(AttachedLocationGroup::new(id)?));
        self.groups.push(g.clone());
        Ok(g)
    }

    /// `Reset`.
    pub fn reset(&mut self) {
        self.groups.clear();
    }

    /// The `groups` field.
    pub fn groups(&self) -> &[AttachedLocationGroupRef] {
        &self.groups
    }
}

/// `AttachedItem`.
#[derive(Clone, Debug)]
pub struct AttachedItem<I> {
    location: String,
    item: I,
}

impl<I> AttachedItem<I> {
    /// `new AttachedItem(location, item)` (`item` must not be null).
    pub fn new(location: Option<&str>, item: Option<I>) -> ScriptResult<Self> {
        let location = check_id(location, "location")?;
        let item = item.ok_or_else(|| npe_msg("item is null"))?;
        Ok(AttachedItem { location, item })
    }

    /// `getLocation`.
    pub fn location(&self) -> &str {
        &self.location
    }

    /// `getItem`.
    pub fn item(&self) -> &I {
        &self.item
    }
}

/// `AttachedItems`.
#[derive(Clone, Debug)]
pub struct AttachedItems<I> {
    group: AttachedLocationGroupRef,
    items: Vec<AttachedItem<I>>,
}

impl<I: ItemIdentity> AttachedItems<I> {
    /// `new AttachedItems(group)`.
    pub fn new(group: AttachedLocationGroupRef) -> Self {
        AttachedItems { group, items: Vec::new() }
    }

    /// `copyFrom`: both must use the same group.
    pub fn copy_from(&mut self, other: &AttachedItems<I>) -> ScriptResult {
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

    /// `getGroup`.
    pub fn group(&self) -> &AttachedLocationGroupRef {
        &self.group
    }

    /// `get(index)`.
    pub fn get(&self, index: i32) -> ScriptResult<&AttachedItem<I>> {
        usize::try_from(index)
            .ok()
            .and_then(|i| self.items.get(i))
            .ok_or_else(|| index_out_of_bounds(index as i64, self.items.len()))
    }

    fn index_of_location(&self, location: &str) -> Option<usize> {
        self.items.iter().position(|a| a.location == location)
    }

    fn index_of_item(&self, item: &I) -> Option<usize> {
        self.items.iter().position(|a| a.item.same(item))
    }

    /// `setItem`: replaces the item at `location`, then inserts in the
    /// group's location order.
    pub fn set_item(&mut self, location: Option<&str>, item: Option<I>) -> ScriptResult {
        let group = self.group.clone();
        let group = group.borrow();
        group.check_valid(location)?;
        let location = location.ok_or_else(npe)?;
        if let Some(i) = self.index_of_location(location) {
            self.items.remove(i);
        }
        let Some(item) = item else {
            return Ok(());
        };
        self.remove(&item);
        let at = group.index_of(Some(location));
        let insert_at = self
            .items
            .iter()
            .position(|a| group.index_of(Some(&a.location)) > at)
            .unwrap_or(self.items.len());
        self.items.insert(insert_at, AttachedItem { location: location.to_string(), item });
        Ok(())
    }

    /// `getItem(location)`.
    pub fn item(&self, location: Option<&str>) -> ScriptResult<Option<I>> {
        self.group.borrow().check_valid(location)?;
        let location = location.ok_or_else(npe)?;
        Ok(self.index_of_location(location).map(|i| self.items[i].item.clone()))
    }

    /// `getItemByIndex`.
    pub fn item_by_index(&self, index: i32) -> Option<I> {
        usize::try_from(index).ok().and_then(|i| self.items.get(i)).map(|a| a.item.clone())
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
    pub fn location(&self, item: &I) -> Option<&str> {
        self.index_of_item(item).map(|i| self.items[i].location.as_str())
    }

    /// `contains`.
    pub fn contains(&self, item: &I) -> bool {
        self.index_of_item(item).is_some()
    }

    /// `size`.
    pub fn size(&self) -> usize {
        self.items.len()
    }

    /// `isEmpty`.
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// The items, in order (`forEach`).
    pub fn items(&self) -> &[AttachedItem<I>] {
        &self.items
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Clone, Debug, PartialEq)]
    struct Item(u32);

    impl ItemIdentity for Item {
        fn same(&self, other: &Self) -> bool {
            self == other
        }
    }

    #[test]
    fn groups_and_locations() {
        let mut locs = AttachedLocations::default();
        let g = locs.get_group(Some("Critter")).unwrap();
        assert!(Rc::ptr_eq(&g, &locs.get_group(Some("Critter")).unwrap()));
        assert!(locs.get_group(None).unwrap_err().0.contains("NullPointerException: id is null"));
        assert!(locs.get_group(Some("")).unwrap_err().0.contains("id is empty"));
        let mut g = g.borrow_mut();
        let tail = g.get_or_create_location(Some("tail")).unwrap();
        assert!(Rc::ptr_eq(&tail, &g.get_or_create_location(Some("tail")).unwrap()));
        tail.borrow_mut().set_attachment_name(Some("tail_bone"));
        assert_eq!(tail.borrow().attachment_name(), Some("tail_bone"));
        g.get_or_create_location(Some("ear")).unwrap();
        assert_eq!((g.size(), g.index_of(Some("ear")), g.index_of(None)), (2, 1, -1));
        assert!(g.location(None).is_none() && g.location_by_index(2).is_none());
        assert!(g.check_valid(Some("nose")).unwrap_err().0.contains("no such location \"nose\""));
        assert!(g.check_valid(Some("")).unwrap_err().0.contains("locationId is empty"));
        assert!(g.get_or_create_location(None).is_err());
    }

    #[test]
    fn attached_items_order() {
        let g = Rc::new(RefCell::new(AttachedLocationGroup::new(Some("Critter")).unwrap()));
        for id in ["back", "belt", "ankle"] {
            g.borrow_mut().get_or_create_location(Some(id)).unwrap();
        }
        let mut a = AttachedItems::new(g.clone());
        a.set_item(Some("ankle"), Some(Item(1))).unwrap();
        a.set_item(Some("back"), Some(Item(2))).unwrap();
        a.set_item(Some("belt"), Some(Item(3))).unwrap();
        let order: Vec<u32> = a.items().iter().map(|i| i.item().0).collect();
        assert_eq!(order, [2, 3, 1]);
        // Moving an item removes it from its old location.
        a.set_item(Some("ankle"), Some(Item(2))).unwrap();
        assert_eq!(a.location(&Item(2)), Some("ankle"));
        assert_eq!(a.size(), 2);
        assert_eq!(a.item(Some("back")).unwrap(), None);
        a.set_item(Some("belt"), None).unwrap();
        assert!(!a.contains(&Item(3)));
        assert!(a.set_item(Some("nope"), Some(Item(9))).is_err());
        assert!(a.get(5).is_err());
        let mut b = AttachedItems::new(g);
        b.copy_from(&a).unwrap();
        assert_eq!(b.item_by_index(0), Some(Item(2)));
        let other = AttachedItems::<Item>::new(Rc::new(RefCell::new(AttachedLocationGroup::new(Some("X")).unwrap())));
        assert!(b.copy_from(&other).is_err());
        assert!(AttachedItem::<Item>::new(Some("back"), None).is_err());
    }
}
