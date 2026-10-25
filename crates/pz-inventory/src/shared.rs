//! `ItemContainer` as a shared object: the original's containers and
//! items are Java objects that Lua and the engine hold at the same time.
//!
//! A [`SharedContainer`] holds its decoded items as [`ItemRef`]s (one
//! `Rc` per item, the original's object identity), so a Lua item object
//! and the engine see the same item. [`SharedContainer::from_container`]
//! and [`SharedContainer::to_container`] convert from and to the saved
//! [`ItemContainer`] (the form `load`/`save` use).
//!
//! `InventoryItem.container` (the item's current container) is kept in a
//! per-thread registry ([`container_of`]); [`add_item`] and
//! [`remove_item`] keep it as `AddItem` and `Remove` do.
//!
//! Records the item codec keeps raw ([`SavedItem::Raw`]) are kept in
//! [`SharedContainer::raw_items`]: they have no item object, so the
//! container methods do not see them (as the weight methods of
//! [`ItemContainer`] skip them), and they are saved after the decoded
//! items.
//!
//! Bags (`InventoryContainer`) keep their contents in the item
//! ([`crate::item::ItemKind::Container`]) as saved items; they are not
//! shared objects yet.

use std::any::Any;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::{Rc, Weak};

use crate::container::ItemContainer;
use crate::gameplay::UseOutcome;
use crate::item::{InventoryItem, ItemKind};
use crate::saved::SavedItem;

/// A shared item (the original's object identity).
pub type ItemRef = Rc<RefCell<InventoryItem>>;

/// A shared container.
pub type ContainerRef = Rc<RefCell<SharedContainer>>;

/// `zombie.inventory.ItemContainer`'s fields.
pub struct SharedContainer {
    /// `type` (`"none"` by default).
    pub kind: String,
    pub capacity: i32,
    /// `ID` (`-1` for `new ItemContainer()`).
    pub id: i32,
    pub active: bool,
    pub dirty: bool,
    pub draw_dirty: bool,
    /// `isdevice`.
    pub is_device: bool,
    pub age_factor: f32,
    pub cooking_factor: f32,
    pub explored: bool,
    pub has_been_looted: bool,
    pub custom_temperature: f32,
    pub weight_reduction: i32,
    pub open_sound: Option<String>,
    pub close_sound: Option<String>,
    pub put_sound: Option<String>,
    pub take_sound: Option<String>,
    pub only_accept_category: Option<String>,
    pub accept_item_function: Option<String>,
    pub container_position: Option<String>,
    pub freezer_position: Option<String>,
    /// `items`, in list order.
    pub items: Vec<ItemRef>,
    /// Saved records without an item object (see the module docs).
    pub raw_items: Vec<SavedItem>,
    /// `containingItem`: the bag whose inventory this is.
    pub containing_item: Option<Weak<RefCell<InventoryItem>>>,
    /// `parent`, opaque here: the holder of the container as its owner
    /// stores it (pz-lua-api keeps the parent's Lua object).
    pub parent: Option<Rc<dyn Any>>,
    /// `sourceGrid`, opaque like [`Self::parent`].
    pub source_grid: Option<Rc<dyn Any>>,
}

impl std::fmt::Debug for SharedContainer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SharedContainer")
            .field("kind", &self.kind)
            .field("id", &self.id)
            .field("items", &self.items.len())
            .finish()
    }
}

impl Default for SharedContainer {
    /// `new ItemContainer()`.
    fn default() -> Self {
        SharedContainer {
            kind: "none".to_string(),
            capacity: 50,
            id: -1,
            active: false,
            dirty: true,
            draw_dirty: true,
            is_device: false,
            age_factor: 1.0,
            cooking_factor: 1.0,
            explored: false,
            has_been_looted: false,
            custom_temperature: 0.0,
            weight_reduction: 0,
            open_sound: None,
            close_sound: None,
            put_sound: None,
            take_sound: None,
            only_accept_category: None,
            accept_item_function: None,
            container_position: None,
            freezer_position: None,
            items: Vec::new(),
            raw_items: Vec::new(),
            containing_item: None,
            parent: None,
            source_grid: None,
        }
    }
}

impl SharedContainer {
    /// `new ItemContainer(id, containerName, square, parent)` (the square
    /// and parent are set by the caller): a fridge ages food at 0.02 and
    /// does not cook.
    pub fn new(id: i32, kind: &str) -> Self {
        let mut c = SharedContainer {
            id,
            kind: kind.to_string(),
            ..Default::default()
        };
        if c.is_fridge_type() {
            c.age_factor = 0.02;
            c.cooking_factor = 0.0;
        }
        c
    }

    /// Wraps it as a [`ContainerRef`].
    pub fn into_ref(self) -> ContainerRef {
        Rc::new(RefCell::new(self))
    }

    /// The shared form of a saved container: its decoded items become
    /// item objects (and this container their `container`), raw records
    /// are kept. A missing `type` reads as `"none"`.
    pub fn from_container(c: ItemContainer) -> ContainerRef {
        let mut s = SharedContainer {
            kind: c.kind.unwrap_or_else(|| "none".to_string()),
            capacity: c.capacity,
            id: c.id,
            explored: c.explored,
            has_been_looted: c.has_been_looted,
            container_position: c.container_position,
            freezer_position: c.freezer_position,
            ..Default::default()
        };
        let mut items = Vec::new();
        for i in c.items {
            match i {
                SavedItem::Item(b) => items.push(Rc::new(RefCell::new(*b))),
                raw => s.raw_items.push(raw),
            }
        }
        let r = s.into_ref();
        for i in &items {
            set_container(i, Some(&r));
        }
        r.borrow_mut().items = items;
        r
    }

    /// The saved form (a copy of the items, then the raw records).
    pub fn to_container(&self) -> ItemContainer {
        ItemContainer {
            kind: Some(self.kind.clone()),
            capacity: self.capacity,
            container_position: self.container_position.clone(),
            freezer_position: self.freezer_position.clone(),
            id: self.id,
            explored: self.explored,
            has_been_looted: self.has_been_looted,
            items: self
                .items
                .iter()
                .map(|i| SavedItem::Item(Box::new(i.borrow().clone())))
                .chain(self.raw_items.iter().cloned())
                .collect(),
        }
    }

    /// `isFreezer()`: the type is `ContainerType.FREEZER` (`base:freezer`)
    /// or `freezer`.
    pub fn is_freezer(&self) -> bool {
        self.kind != "none" && (self.kind == "base:freezer" || self.kind == "freezer")
    }

    /// `isFridge()` without a parent object: the type is
    /// `ContainerType.FRIDGE` (`base:fridge`) or `fridge` (a parent's
    /// `IsFridge` property is the caller's).
    pub fn is_fridge_type(&self) -> bool {
        self.kind != "none"
            && !self.is_freezer()
            && (self.kind == "base:fridge" || self.kind == "fridge")
    }

    /// The index of `item` in `items` (by identity).
    pub fn index_of(&self, item: &ItemRef) -> Option<usize> {
        self.items.iter().position(|i| Rc::ptr_eq(i, item))
    }
}

type ContainerMap = HashMap<usize, (Weak<RefCell<InventoryItem>>, Weak<RefCell<SharedContainer>>)>;

thread_local! {
    static ITEM_CONTAINERS: RefCell<ContainerMap> = RefCell::new(HashMap::new());
}

fn key(item: &ItemRef) -> usize {
    Rc::as_ptr(item) as *const () as usize
}

/// `item.getContainer()`.
pub fn container_of(item: &ItemRef) -> Option<ContainerRef> {
    ITEM_CONTAINERS.with(|m| {
        let m = m.borrow();
        let (i, c) = m.get(&key(item))?;
        if !std::ptr::eq(i.as_ptr(), Rc::as_ptr(item)) || i.strong_count() == 0 {
            return None;
        }
        c.upgrade()
    })
}

/// `item.container = container`.
pub fn set_container(item: &ItemRef, container: Option<&ContainerRef>) {
    ITEM_CONTAINERS.with(|m| {
        let mut m = m.borrow_mut();
        match container {
            Some(c) => {
                if m.len() > 1024 && m.len().is_power_of_two() {
                    m.retain(|_, (i, c)| i.strong_count() > 0 && c.strong_count() > 0);
                }
                m.insert(key(item), (Rc::downgrade(item), Rc::downgrade(c)));
            }
            None => {
                m.remove(&key(item));
            }
        }
    });
}

/// The list part of `ItemContainer.AddItem(item)` once the checks passed:
/// the item leaves its old container (its `Remove`, without the owner's
/// side effects; [`remove_item`]) and is appended here.
pub fn add_item(c: &ContainerRef, item: &ItemRef) {
    if let Some(old) = container_of(item) {
        remove_item(&old, item);
    }
    set_container(item, Some(c));
    c.borrow_mut().items.push(item.clone());
}

/// The list part of `ItemContainer.Remove(item)`: the first entry that is
/// `item` is removed and the item has no container. Returns whether it
/// was there.
pub fn remove_item(c: &ContainerRef, item: &ItemRef) -> bool {
    let i = c.borrow().index_of(item);
    match i {
        Some(i) => {
            {
                let mut s = c.borrow_mut();
                s.items.remove(i);
                s.draw_dirty = true;
                s.dirty = true;
            }
            set_container(item, None);
            true
        }
        None => false,
    }
}

/// `ItemContainer.load` into the existing container `c` (the object
/// stays, as the original loads into the same container): its fields and
/// items become those of the saved `saved`; the old items have no
/// container after.
pub fn load_into(c: &ContainerRef, saved: ItemContainer) {
    let old = std::mem::take(&mut c.borrow_mut().items);
    for i in &old {
        set_container(i, None);
    }
    let mut items = Vec::new();
    let mut raw_items = Vec::new();
    for i in saved.items {
        match i {
            SavedItem::Item(b) => items.push(Rc::new(RefCell::new(*b))),
            raw => raw_items.push(raw),
        }
    }
    for i in &items {
        set_container(i, Some(c));
    }
    let mut s = c.borrow_mut();
    s.kind = saved.kind.unwrap_or_else(|| "none".to_string());
    s.capacity = saved.capacity;
    s.id = saved.id;
    s.explored = saved.explored;
    s.has_been_looted = saved.has_been_looted;
    s.container_position = saved.container_position;
    s.freezer_position = saved.freezer_position;
    s.items = items;
    s.raw_items = raw_items;
}

/// [`ItemContainer::apply_use`] on a shared container: the
/// `ReplaceOnUse`/`ReplaceOnDeplete` item is added (food at
/// `temperature`), then the used item `id` removed when the outcome says
/// so. Returns the new item's id.
pub fn apply_use(c: &ContainerRef, id: i32, out: &mut UseOutcome, temperature: f32) -> Option<i32> {
    let mut new_id = None;
    if let Some(a) = out.added.as_mut() {
        if let Some(mut item) = a.item.take() {
            if let ItemKind::Food(f) = &mut item.kind {
                f.heat = temperature;
            }
            new_id = Some(item.id);
            add_item(c, &Rc::new(RefCell::new(item)));
        }
    }
    if out.remove {
        let found = c.borrow().items.iter().find(|i| i.borrow().id == id).cloned();
        if let Some(i) = found {
            remove_item(c, &i);
        }
    }
    new_id
}
