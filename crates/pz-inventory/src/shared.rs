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
//! one actual strong field ([`try_container_of`]); [`try_add_item`] and
//! [`try_remove_item`] keep it as `AddItem` and `Remove` do. The weak registry
//! formerly used for this field is no longer authoritative or retained.
//!
//! Records the item codec keeps raw ([`SavedItem::Raw`]) are kept in
//! [`SharedContainer::raw_items`]: they have no item object, so the
//! container methods do not see them (as the weight methods of
//! [`ItemContainer`] skip them), and they are saved after the decoded
//! items.
//!
//! A bag (`InventoryContainer`) holds its inventory as a
//! [`ContainerRef`] ([`crate::item::ContainerData::container`]) whose
//! `containingItem` is the bag: [`new_item`] (the way an item becomes a
//! shared object) links it. Cloning an item copies its bag's contents
//! into a new container (an item value is a copy, not the object).

use std::any::Any;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

use crate::container::ItemContainer;
use crate::context::ItemContext;
use crate::gameplay::UseOutcome;
use crate::item::{InventoryItem, ItemKind};
use crate::saved::SavedItem;
use crate::template::ItemTemplate;

/// A shared item (the original's object identity).
pub type ItemRef = Rc<RefCell<InventoryItem>>;

/// A shared container.
pub type ContainerRef = Rc<RefCell<SharedContainer>>;

/// The sole nullable physical InventoryContainer.container field. Native
/// constructor paths allocate a child; collector detach leaves genuine null.
/// Legacy Deref is restricted to the existing nonnull synchronous profile.
#[derive(Debug, Default)]
pub struct ContainerField(Option<ContainerRef>);
impl ContainerField {
    pub fn slot(&self) -> Option<&ContainerRef> {
        self.0.as_ref()
    }
    pub fn take_for_gc(&mut self) -> Option<ContainerRef> {
        self.0.take()
    }
    pub fn replace(
        &mut self,
        new: Option<&ContainerRef>,
        retired: &mut pz_entity::RetiredComponents,
    ) {
        if let Some(old) = std::mem::replace(&mut self.0, new.cloned()) {
            retired.push_erased(old);
        }
    }
}
impl From<ContainerRef> for ContainerField {
    fn from(value: ContainerRef) -> Self {
        Self(Some(value))
    }
}
impl std::ops::Deref for ContainerField {
    type Target = ContainerRef;
    fn deref(&self) -> &ContainerRef {
        self.0
            .as_ref()
            .expect("legacy nonnull bag container profile")
    }
}

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
    /// Native separate physical nullable list, populated during compressed load.
    pub including_obsolete_items: Vec<Option<ItemRef>>,
    /// Saved records without an item object (see the module docs).
    pub raw_items: Vec<SavedItem>,
    /// `containingItem`: the bag whose inventory this is.
    pub containing_item: Option<ItemRef>,
    /// Distinct Native constructor-owned bag alias, separate from containingItem.
    pub inventory_container: Option<ItemRef>,
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

impl PartialEq for SharedContainer {
    /// The values (the items by value); the owner links are not compared.
    fn eq(&self, o: &Self) -> bool {
        self.kind == o.kind
            && self.capacity == o.capacity
            && self.id == o.id
            && self.active == o.active
            && self.is_device == o.is_device
            && self.age_factor == o.age_factor
            && self.cooking_factor == o.cooking_factor
            && self.explored == o.explored
            && self.has_been_looted == o.has_been_looted
            && self.custom_temperature == o.custom_temperature
            && self.weight_reduction == o.weight_reduction
            && self.open_sound == o.open_sound
            && self.close_sound == o.close_sound
            && self.put_sound == o.put_sound
            && self.take_sound == o.take_sound
            && self.only_accept_category == o.only_accept_category
            && self.accept_item_function == o.accept_item_function
            && self.container_position == o.container_position
            && self.freezer_position == o.freezer_position
            && self.raw_items == o.raw_items
            && self.items.len() == o.items.len()
            && self
                .items
                .iter()
                .zip(&o.items)
                .all(|(a, b)| Rc::ptr_eq(a, b) || *a.borrow() == *b.borrow())
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
            including_obsolete_items: Vec::new(),
            raw_items: Vec::new(),
            containing_item: None,
            inventory_container: None,
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
        let container = Rc::new(RefCell::new(self));
        observe_container(&container);
        container
    }

    /// The shared form of a saved container: its decoded items become
    /// item objects (and this container their `container`), raw records
    /// are kept. A missing `type` reads as `"none"`.
    pub fn from_container(c: ItemContainer) -> ContainerRef {
        let r = SharedContainer::default().into_ref();
        Self::fill_from(&r, c);
        r
    }

    /// Replaces the saved fields and the items of `r` with those of `c`
    /// (the old items lose their `container`; the decoded ones become item
    /// objects whose container is `r`).
    pub fn fill_from(r: &ContainerRef, c: ItemContainer) {
        let old = std::mem::take(&mut r.borrow_mut().items);
        for i in &old {
            if container_of(i).is_some_and(|o| Rc::ptr_eq(&o, r)) {
                set_container(i, None);
            }
        }
        let mut items = Vec::new();
        let mut raw = Vec::new();
        for i in c.items {
            match i {
                SavedItem::Item(b) => items.push(new_item(*b)),
                other => raw.push(other),
            }
        }
        for i in &items {
            set_container(i, Some(r));
        }
        let mut s = r.borrow_mut();
        s.kind = c.kind.unwrap_or_else(|| "none".to_string());
        s.capacity = c.capacity;
        s.id = c.id;
        s.explored = c.explored;
        s.has_been_looted = c.has_been_looted;
        s.container_position = c.container_position;
        s.freezer_position = c.freezer_position;
        s.items = items;
        s.raw_items = raw;
    }

    /// A copy as a new container: the items are copied (bags with their
    /// contents) and are the new container's; the owner links
    /// (`parent`, `sourceGrid`, `containingItem`, `inventoryContainer`) and the
    /// runtime includingObsolete bookkeeping list are not copied.
    pub fn try_deep_clone(&self) -> Result<ContainerRef, crate::archive::ArchiveCopyError> {
        crate::archive::ArchiveCopyCheck::default().shared_container_body(self)?;
        Ok(self.deep_clone())
    }
    pub fn deep_clone(&self) -> ContainerRef {
        let r = SharedContainer {
            kind: self.kind.clone(),
            capacity: self.capacity,
            id: self.id,
            active: self.active,
            dirty: self.dirty,
            draw_dirty: self.draw_dirty,
            is_device: self.is_device,
            age_factor: self.age_factor,
            cooking_factor: self.cooking_factor,
            explored: self.explored,
            has_been_looted: self.has_been_looted,
            custom_temperature: self.custom_temperature,
            weight_reduction: self.weight_reduction,
            open_sound: self.open_sound.clone(),
            close_sound: self.close_sound.clone(),
            put_sound: self.put_sound.clone(),
            take_sound: self.take_sound.clone(),
            only_accept_category: self.only_accept_category.clone(),
            accept_item_function: self.accept_item_function.clone(),
            container_position: self.container_position.clone(),
            freezer_position: self.freezer_position.clone(),
            items: Vec::new(),
            including_obsolete_items: Vec::new(),
            raw_items: self.raw_items.clone(),
            containing_item: None,
            inventory_container: None,
            parent: None,
            source_grid: None,
        }
        .into_ref();
        let items: Vec<ItemRef> = self
            .items
            .iter()
            .map(|i| new_item(i.borrow().clone()))
            .collect();
        for i in &items {
            set_container(i, Some(&r));
        }
        r.borrow_mut().items = items;
        r
    }

    /// `new ItemContainer()` as `InventoryContainer`'s constructor and
    /// `Item.InstanceItem` set it up for a bag of `template`: the type is
    /// the item type, then `setCapacity`, `setWeightReduction` and the
    /// script's sounds and accept rules.
    pub fn new_bag(template: &ItemTemplate) -> Self {
        let discard = |s: &Option<String>| s.clone().filter(|s| !s.trim().is_empty());
        SharedContainer {
            kind: template.type_.clone(),
            capacity: template.capacity,
            weight_reduction: template.weight_reduction.clamp(0, 100),
            put_sound: template.put_in_sound.clone(),
            close_sound: template.close_sound.clone(),
            open_sound: template.open_sound.clone(),
            only_accept_category: discard(&template.only_accept_category),
            accept_item_function: discard(&template.accept_item_function),
            ..Default::default()
        }
    }

    /// `getContentsWeight()`: the items' `getUnequippedWeight()` (raw
    /// records weigh nothing; the items' holder is not known here).
    pub fn get_contents_weight_with(&self, ctx: &dyn ItemContext) -> f32 {
        let owner = crate::context::NoOwner;
        self.items
            .iter()
            .map(|i| i.borrow().get_unequipped_weight(ctx, &owner))
            .fold(0.0, |a, w| a + w)
    }

    /// `isEmpty()` (raw records count as items).
    pub fn is_empty(&self) -> bool {
        self.items.is_empty() && self.raw_items.is_empty()
    }

    /// The saved form (a copy of the items, then the raw records).
    pub fn try_to_container(&self) -> Result<ItemContainer, crate::archive::ArchiveCopyError> {
        crate::archive::ArchiveCopyCheck::default().shared_container_body(self)?;
        Ok(self.to_container())
    }
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

/// The item object of `item`: a bag's inventory gets the new object as
/// its `containingItem`. Every shared item is made with this.
pub fn new_item(item: InventoryItem) -> ItemRef {
    try_new_item(item)
        .unwrap_or_else(|failure| panic!("item allocation: {:?}", failure.into_error_and_retire()))
}

/// Metadata-only host registration, installed before live item creation.
/// The function must not invoke Lua or native lifecycle callbacks. Registry
/// unavailability stays conservative and is retried by the host observer.
pub type ItemObserver = fn(&ItemRef);

thread_local! {
    static ITEM_OBSERVER: Cell<Option<ItemObserver>> = const { Cell::new(None) };
}

pub fn install_item_observer(observer: Option<ItemObserver>) -> Option<ItemObserver> {
    ITEM_OBSERVER.with(|slot| slot.replace(observer))
}

/// End the observer-slot borrow before invoking the metadata callback.
pub fn observe_item(item: &ItemRef) {
    let observer = ITEM_OBSERVER.with(Cell::get);
    if let Some(observer) = observer {
        observer(item);
    }
}

#[derive(Debug)]
#[must_use]
pub struct NewItemFailure {
    pub error: pz_entity::runtime_load::RuntimeHostError,
    pub item: ItemRef,
    pub retired: pz_entity::RetiredComponents,
}

impl NewItemFailure {
    pub fn into_error_and_retire(mut self) -> pz_entity::runtime_load::RuntimeHostError {
        self.retired
            .push_owner(pz_entity::EntityOwner::from_ref(&self.item));
        crate::component_host::retire_components(self.retired);
        self.error
    }
}

/// Construct the actual allocation before installing strong backedges. The
/// observer sees every actual item/component allocation before a cycle can
/// become unreachable. Failure owns the completed prefix for outer retirement.
pub fn try_new_item(item: InventoryItem) -> Result<ItemRef, NewItemFailure> {
    use pz_entity::runtime_load::RuntimeHostError;
    let item = Rc::new(RefCell::new(item));
    item.borrow_mut().components.mark_bound();
    observe_item(&item);
    let refs: Vec<_> = item
        .borrow()
        .components
        .entry_slots()
        .filter_map(|entry| match entry {
            pz_entity::RuntimeEntryView::Live { component, .. } => Some(component.clone()),
            pz_entity::RuntimeEntryView::Encoded(_) => None,
        })
        .collect();
    let mut retired = pz_entity::RetiredComponents::new();
    for component in refs {
        let result = match component.try_borrow_mut() {
            Ok(mut body) => {
                if body.owner_slot().is_some() {
                    Err(RuntimeHostError::Unsupported {
                        operation: "adopt component with existing owner",
                        kind: Some(body.kind()),
                    })
                } else {
                    if let Some(owner) =
                        body.replace_owner(Some(&pz_entity::EntityOwner::from_ref(&item)))
                    {
                        retired.push_owner(owner);
                    }
                    Ok(())
                }
            }
            Err(_) => Err(RuntimeHostError::Borrowed(
                "attach new item component owner",
            )),
        };
        retired.push_component(component);
        if let Err(error) = result {
            return Err(NewItemFailure {
                error,
                item,
                retired,
            });
        }
    }
    if let Err(error) = link_bag_ref(&item) {
        return Err(NewItemFailure { error, item, retired });
    }
    // These are additional pins only: the actual store retains every live
    // component, and no former owner exists on successful unowned adoption.
    drop(retired);
    Ok(item)
}

/// Sets `containingItem` of `item`'s inventory (a bag) to `item`.
pub fn link_bag(item: &ItemRef) {
    link_bag_ref(item).expect("legacy unborrowed bag linking");
}

pub fn link_bag_ref(item: &ItemRef) -> pz_entity::runtime_load::HostResult<()> {
    use pz_entity::runtime_load::RuntimeHostError;
    pin_owner(item);
    let container = {
        let body = item
            .try_borrow()
            .map_err(|_| RuntimeHostError::Borrowed("bag inventory link"))?;
        inventory_of(&body)
    };
    if let Some(container) = container {
        pin_owner(&container);
        let result = container
            .try_borrow_mut()
            .map_err(|_| RuntimeHostError::Borrowed("bag containingItem link"))
            .map(|mut body| {
                if let Some(old) = body.containing_item.replace(item.clone()) {
                    pin_owner(&old);
                }
                if let Some(old) = body.inventory_container.replace(item.clone()) {
                    pin_owner(&old);
                }
            });
        observe_container(&container);
        result?;
    }
    Ok(())
}

/// `InventoryContainer.getInventory()` (`None` for other items).
pub fn inventory_of(item: &InventoryItem) -> Option<ContainerRef> {
    match &item.kind {
        ItemKind::Container(c) => c.container.slot().cloned(),
        _ => None,
    }
}

/// `ItemContainer.containingItem`.
pub fn try_containing_item(c: &ContainerRef) -> pz_entity::runtime_load::HostResult<Option<ItemRef>> {
    let result = c.try_borrow().map(|body| body.containing_item.clone())
        .map_err(|_| pz_entity::runtime_load::RuntimeHostError::Borrowed("container containingItem getter"));
    observe_container(c);
    result
}
pub fn containing_item(c: &ContainerRef) -> Option<ItemRef> {
    try_containing_item(c).expect("legacy unborrowed containingItem getter")
}

/// `getOutermostContainer()` of a container: up through the bags that
/// hold it.
pub fn outermost(c: &ContainerRef) -> ContainerRef {
    let mut c = c.clone();
    while let Some(outer) = containing_item(&c).and_then(|b| container_of(&b)) {
        c = outer;
    }
    c
}

/// `item.getOutermostContainer()`: `None` when the item has no container
/// or lies on the floor; a bag on the floor is the outermost.
pub fn outermost_container_of(item: &ItemRef) -> Option<ContainerRef> {
    let mut outer = container_of(item)?;
    if outer.borrow().kind == "floor" {
        return None;
    }
    while let Some(next) = containing_item(&outer).and_then(|b| container_of(&b)) {
        if next.borrow().kind == "floor" {
            break;
        }
        outer = next;
    }
    Some(outer)
}

/// The one physical runtime backlink. Archive copies clear it and data
/// equality ignores it, without retaining the source container graph.
#[derive(Debug, Default)]
pub(crate) struct ItemContainerSlot(Option<ContainerRef>);
impl Clone for ItemContainerSlot {
    fn clone(&self) -> Self {
        Self::default()
    }
}
impl PartialEq for ItemContainerSlot {
    fn eq(&self, _other: &Self) -> bool {
        true
    }
}
impl InventoryItem {
    pub fn container_slot(&self) -> Option<&ContainerRef> {
        self.container.0.as_ref()
    }
    pub fn take_container_for_gc(&mut self) -> Option<ContainerRef> {
        self.container.0.take()
    }
    pub fn replace_container(
        &mut self,
        container: Option<&ContainerRef>,
        retired: &mut pz_entity::RetiredComponents,
    ) {
        if let Some(old) = std::mem::replace(&mut self.container.0, container.cloned()) {
            retired.push_erased(old);
        }
    }
}

pub(crate) fn pin_owner<T: Any>(owner: &Rc<T>) {
    let mut retired = pz_entity::RetiredComponents::new();
    retired.push_erased(owner.clone());
    crate::component_host::retire_components(retired);
}

pub type ContainerObserver = fn(&ContainerRef);
thread_local! {
    static CONTAINER_OBSERVER: Cell<Option<ContainerObserver>> = const { Cell::new(None) };
}
pub fn install_container_observer(
    observer: Option<ContainerObserver>,
) -> Option<ContainerObserver> {
    CONTAINER_OBSERVER.with(|slot| slot.replace(observer))
}
pub fn observe_container(container: &ContainerRef) {
    if let Some(observer) = CONTAINER_OBSERVER.with(Cell::get) {
        observer(container);
    }
}

/// Pointer-only actual strong field access; no child container body access.
pub fn try_container_of(
    item: &ItemRef,
) -> pz_entity::runtime_load::HostResult<Option<ContainerRef>> {
    use pz_entity::runtime_load::RuntimeHostError;
    let result = item
        .try_borrow()
        .map(|body| body.container_slot().cloned())
        .map_err(|_| RuntimeHostError::Borrowed("item container getter"));
    observe_item(item);
    result
}
/// Legacy synchronous wrapper with an explicit unborrowed item precondition.
pub fn container_of(item: &ItemRef) -> Option<ContainerRef> {
    try_container_of(item).expect("legacy unborrowed item container getter")
}

/// Direct field assignment. It does not remove an item from another list.
pub fn try_set_container(
    item: &ItemRef,
    container: Option<&ContainerRef>,
) -> pz_entity::runtime_load::HostResult<()> {
    use pz_entity::runtime_load::RuntimeHostError;
    pin_owner(item);
    if let Some(container) = container {
        pin_owner(container);
    }
    let mut retired = pz_entity::RetiredComponents::new();
    let result = item
        .try_borrow_mut()
        .map_err(|_| RuntimeHostError::Borrowed("item container setter"))
        .map(|mut body| body.replace_container(container, &mut retired));
    crate::component_host::retire_components(retired);
    observe_item(item);
    result
}
/// Legacy synchronous wrapper; actual typed callers use try_set_container.
pub fn set_container(item: &ItemRef, container: Option<&ContainerRef>) {
    try_set_container(item, container).expect("legacy unborrowed item container setter");
}

/// The list part of `ItemContainer.AddItem(item)` once the checks passed:
/// the item leaves its old container (its `Remove`, without the owner's
/// side effects; [`remove_item`]) and is appended here.
pub fn try_add_item(c: &ContainerRef, item: &ItemRef) -> pz_entity::runtime_load::HostResult<()> {
    use pz_entity::runtime_load::RuntimeHostError;
    pin_owner(c);
    pin_owner(item);
    if let Some(old) = try_container_of(item)? {
        try_remove_item(&old, item)?;
    }
    try_set_container(item, Some(c))?;
    let result = c
        .try_borrow_mut()
        .map_err(|_| RuntimeHostError::Borrowed("container add item"))
        .map(|mut body| body.items.push(item.clone()));
    observe_container(c);
    result
}
pub fn add_item(c: &ContainerRef, item: &ItemRef) {
    try_add_item(c, item).expect("legacy unborrowed container AddItem");
}

/// Native Remove does not change includingObsoleteItems. A later backlink
/// assignment failure preserves the already completed physical removal.
pub fn try_remove_item(
    c: &ContainerRef,
    item: &ItemRef,
) -> pz_entity::runtime_load::HostResult<bool> {
    use pz_entity::runtime_load::RuntimeHostError;
    pin_owner(c);
    pin_owner(item);
    let result = (|| {
        let mut body = c
            .try_borrow_mut()
            .map_err(|_| RuntimeHostError::Borrowed("container Remove"))?;
        if let Some(index) = body.index_of(item) {
            let old = body.items.remove(index);
            pin_owner(&old);
            Ok(true)
        } else {
            Ok(false)
        }
    })();
    observe_container(c);
    let removed = result?;
    if removed {
        try_set_container(item, None)?;
        let result=c.try_borrow_mut().map_err(|_| RuntimeHostError::Borrowed("container Remove flags"))
            .map(|mut body| {body.draw_dirty=true;body.dirty=true;});
        observe_container(c);
        result?;
    }
    Ok(removed)
}
pub fn remove_item(c: &ContainerRef, item: &ItemRef) -> bool {
    try_remove_item(c, item).expect("legacy unborrowed container Remove")
}

/// `ItemContainer.load` into the existing container `c` (the object
/// stays, as the original loads into the same container): its fields and
/// items become those of the saved `saved`; the old items have no
/// container after.
pub fn load_into(c: &ContainerRef, saved: ItemContainer) {
    SharedContainer::fill_from(c, saved);
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
            add_item(c, &new_item(item));
        }
    }
    if out.remove {
        let found = c
            .borrow()
            .items
            .iter()
            .find(|i| i.borrow().id == id)
            .cloned();
        if let Some(i) = found {
            remove_item(c, &i);
        }
    }
    new_id
}
