//! Actual character persistence. Factory/asset and nested-container phases run
//! after releasing both character and character-manager body guards.
use std::cell::{Cell, RefCell};
use std::rc::Rc;

use pz_entity::runtime_load::RuntimeHostError;
use pz_entity::RetiredComponents;
use pz_inventory::io::{Reader, Writer};
use pz_inventory::item_texture::ItemPersistenceError;
use pz_inventory::shared::{ContainerRef, ItemRef};
use pz_scripts::{ScriptError, ScriptManager};

use super::{GameCharacter, SaveEnv};
mod moving;
use crate::character::{GameCharacterRef, SimContext};
use crate::worn_items::ItemIdentity;
use crate::Characters;

pub type SavedInventoryListRef = Rc<RefCell<Vec<Option<ItemRef>>>>;
type Observer = fn(&SavedInventoryListRef);
thread_local! { static OBSERVER: Cell<Option<Observer>> = const { Cell::new(None) }; }

/// Metadata registration only. Never calls a Native item/list lifecycle hook.
pub fn install_saved_inventory_observer(observer: Option<Observer>) -> Option<Observer> {
    OBSERVER.with(|slot| slot.replace(observer))
}
fn observe(list: &SavedInventoryListRef) {
    if let Some(observer) = OBSERVER.with(Cell::get) {
        observer(list);
    }
}

/// Exclusive authority: actual runtime aliases must never be shadowed by IDs.
#[derive(Debug)]
pub enum SavedInventoryItems {
    ArchiveIds(Vec<Option<i32>>),
    ActualRefs(SavedInventoryListRef),
}
impl SavedInventoryItems {
    pub fn actual_empty() -> Self {
        let list = Rc::new(RefCell::new(Vec::new()));
        observe(&list);
        Self::ActualRefs(list)
    }
    pub fn archive_ids(&self) -> Option<&[Option<i32>]> {
        match self {
            Self::ArchiveIds(ids) => Some(ids),
            Self::ActualRefs(_) => None,
        }
    }
    pub fn actual_refs(&self) -> Option<&SavedInventoryListRef> {
        match self {
            Self::ActualRefs(list) => Some(list),
            Self::ArchiveIds(_) => None,
        }
    }
}

#[derive(Debug)]
pub enum CharacterPersistenceError {
    Script(ScriptError),
    Item(ItemPersistenceError),
    Host(RuntimeHostError),
}
pub type CharacterPersistenceResult<T> = Result<T, CharacterPersistenceError>;
impl CharacterPersistenceError {
    pub fn is_host_fault(&self) -> bool {
        matches!(self, Self::Host(error) | Self::Item(ItemPersistenceError::Host(error))
            if !matches!(error, RuntimeHostError::NativeException(_)))
    }
    /// Only the explicitly owned/archive facade uses this conversion.
    pub(super) fn into_archive_error(self) -> ScriptError {
        match self {
            Self::Script(error) => error,
            other => ScriptError::new(other.to_string()),
        }
    }
}
impl std::fmt::Display for CharacterPersistenceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Script(error) => write!(f, "{error}"),
            Self::Item(error) => write!(f, "{error}"),
            Self::Host(error) => write!(f, "{error:?}"),
        }
    }
}
impl std::error::Error for CharacterPersistenceError {}
impl From<ScriptError> for CharacterPersistenceError {
    fn from(e: ScriptError) -> Self {
        Self::Script(e)
    }
}
impl From<RuntimeHostError> for CharacterPersistenceError {
    fn from(e: RuntimeHostError) -> Self {
        Self::Host(e)
    }
}
impl From<ItemPersistenceError> for CharacterPersistenceError {
    fn from(e: ItemPersistenceError) -> Self {
        Self::Item(e)
    }
}
pub(super) fn unsupported(operation: &'static str) -> RuntimeHostError {
    RuntimeHostError::Unsupported {
        operation,
        kind: None,
    }
}

fn replace_saved<I: ItemIdentity + 'static>(
    character: &GameCharacterRef<I>,
    entries: Vec<Option<ItemRef>>,
) -> CharacterPersistenceResult<()> {
    // Native reads savedInventoryItems after inventory.load/save has returned.
    let list = character
        .try_borrow()
        .map_err(|_| RuntimeHostError::Borrowed("character savedInventoryItems field"))?
        .saved_inventory_items
        .actual_refs()
        .cloned()
        .ok_or_else(|| unsupported("actual character savedInventoryItems archive profile"))?;
    // Retire displaced owners only after ending both list and character guards.
    let old = {
        let mut body = list.try_borrow_mut().map_err(|_| {
            RuntimeHostError::Borrowed("character savedInventoryItems clear/addAll")
        })?;
        let old = std::mem::take(&mut *body);
        body.extend(entries);
        old
    };
    let mut retired = RetiredComponents::new();
    retired.push_payload(Box::new(old));
    pz_inventory::component_host::retire_components(retired);
    observe(&list);
    Ok(())
}

fn pin_receivers<I: ItemIdentity + 'static>(
    character: &GameCharacterRef<I>,
    chars: &Rc<RefCell<Characters>>,
) {
    let mut retired = RetiredComponents::new();
    retired.push_erased(character.clone());
    retired.push_erased(chars.clone());
    pz_inventory::component_host::retire_components(retired);
}

/// The actual receiver path, with no character/manager guard across inventory
/// factories, current asset capture, or a nested bag continuation.
#[allow(clippy::too_many_arguments)]
pub fn load_player_ref<I: ItemIdentity + 'static>(
    character: &GameCharacterRef<I>,
    sm: &ScriptManager,
    chars: &Rc<RefCell<Characters>>,
    ctx: &dyn SimContext,
    env: &SaveEnv<'_>,
    reader: &mut Reader<'_>,
    version: i32,
    set_on_fire: &mut dyn FnMut(&mut GameCharacter<I>),
) -> CharacterPersistenceResult<()> {
    pin_receivers(character, chars);
    super::e(reader.i8())?;
    super::e(reader.i8())?;
    moving::load(character, reader, version)?;
    let inventory = {
        let mut body = character
            .try_borrow_mut()
            .map_err(|_| RuntimeHostError::Borrowed("character load prefix"))?;
        let mut manager = chars
            .try_borrow_mut()
            .map_err(|_| RuntimeHostError::Borrowed("character load prefix manager"))?;
        body.load_game_visual_prefix(sm, &mut manager, env, reader, version)?;
        body.items.inventory.clone()
    };
    let saved =
        pz_inventory::shared_load::load_container_ref(&inventory, reader, version, env.items)
            .map_err(|failure| failure.into_error_and_retire())?;
    replace_saved(character, saved)?;
    {
        let mut body = character
            .try_borrow_mut()
            .map_err(|_| RuntimeHostError::Borrowed("character load continuation"))?;
        let mut manager = chars
            .try_borrow_mut()
            .map_err(|_| RuntimeHostError::Borrowed("character load continuation manager"))?;
        body.load_game_tail(sm, &mut manager, ctx, reader, version, set_on_fire, true)?;
        body.load_player_tail(sm, &mut manager, ctx, env, reader, version, true)?;
    }
    Ok(())
}

pub fn save_player_ref<I: ItemIdentity + 'static>(
    character: &GameCharacterRef<I>,
    _sm: &ScriptManager,
    chars: &Rc<RefCell<Characters>>,
    ctx: &dyn SimContext,
    env: &SaveEnv<'_>,
    writer: &mut Writer,
) -> CharacterPersistenceResult<()> {
    pin_receivers(character, chars);
    moving::save(character, env, writer)?;
    let inventory = {
        let body = character
            .try_borrow()
            .map_err(|_| RuntimeHostError::Borrowed("character save prefix"))?;
        body.save_game_visual_prefix(env, writer)?;
        body.items.inventory.clone()
    };
    let saved = pz_inventory::shared_save::save_container_ref(&inventory, writer, env.items)?;
    replace_saved(character, saved.into_iter().map(Some).collect())?;
    {
        let body = character
            .try_borrow()
            .map_err(|_| RuntimeHostError::Borrowed("character save continuation"))?;
        let manager = chars
            .try_borrow()
            .map_err(|_| RuntimeHostError::Borrowed("character save continuation manager"))?;
        body.save_game_tail(&manager, ctx, writer, true)?;
        body.save_player_tail(ctx, env, writer, true)?;
    }
    Ok(())
}

fn main_items(container: &ContainerRef) -> CharacterPersistenceResult<Vec<ItemRef>> {
    Ok(container
        .try_borrow()
        .map_err(|_| RuntimeHostError::Borrowed("character current inventory items"))?
        .items
        .clone())
}
fn unique_id(item: &ItemRef, inventory: &ContainerRef) -> CharacterPersistenceResult<i32> {
    let id = item
        .try_borrow()
        .map_err(|_| RuntimeHostError::Borrowed("character saved item ID bridge"))?
        .id;
    let items = main_items(inventory)?;
    let mut same_id = 0;
    let mut actual_member = false;
    for member in items {
        if member
            .try_borrow()
            .map_err(|_| RuntimeHostError::Borrowed("character current member ID bridge"))?
            .id
            == id
        {
            same_id += 1;
        }
        actual_member |= Rc::ptr_eq(&member, item);
    }
    if same_id != 1 || !actual_member {
        return Err(unsupported(
            "actual character item reference outside unique main-inventory ID profile",
        )
        .into());
    }
    Ok(id)
}
fn item_for_id(id: i32, inventory: &ContainerRef) -> CharacterPersistenceResult<ItemRef> {
    let mut found = None;
    for member in main_items(inventory)? {
        if member
            .try_borrow()
            .map_err(|_| RuntimeHostError::Borrowed("character hand/worn current item"))?
            .id
            == id
        {
            if found.is_some() {
                return Err(unsupported("actual character duplicate-ID hand/worn storage").into());
            }
            found = Some(member);
        }
    }
    found.ok_or_else(|| {
        unsupported("actual character hand/worn reference outside main inventory").into()
    })
}

impl<I: ItemIdentity + 'static> GameCharacter<I> {
    pub(super) fn persistence_saved_item(
        &self,
        index: i32,
        actual: bool,
    ) -> CharacterPersistenceResult<Option<Option<i32>>> {
        if !actual {
            return Ok(self.saved_item(index));
        }
        let Some(index) = usize::try_from(index).ok() else {
            return Ok(None);
        };
        let list = self
            .saved_inventory_items
            .actual_refs()
            .ok_or_else(|| unsupported("actual savedInventoryItems field"))?;
        let entry = list
            .try_borrow()
            .map_err(|_| RuntimeHostError::Borrowed("character savedInventoryItems get"))?
            .get(index)
            .cloned();
        entry
            .map(|item| {
                item.map(|item| unique_id(&item, &self.items.inventory))
                    .transpose()
            })
            .transpose()
    }
    pub(super) fn persistence_base_item(
        &self,
        index: i32,
        actual: bool,
    ) -> CharacterPersistenceResult<Option<Option<i32>>> {
        if !actual {
            return Ok(self.saved_item(index));
        }
        let Some(index) = usize::try_from(index).ok() else {
            return Ok(None);
        };
        let entry = self
            .items
            .inventory
            .try_borrow()
            .map_err(|_| RuntimeHostError::Borrowed("character current includingObsoleteItems"))?
            .including_obsolete_items
            .get(index)
            .cloned();
        entry
            .map(|item| {
                item.map(|item| unique_id(&item, &self.items.inventory))
                    .transpose()
            })
            .transpose()
    }
    pub(super) fn persistence_saved_index(
        &self,
        id: Option<i32>,
        actual: bool,
    ) -> CharacterPersistenceResult<i16> {
        if !actual {
            return Ok(self.saved_index(id));
        }
        let item = id
            .map(|id| item_for_id(id, &self.items.inventory))
            .transpose()?;
        let list = self
            .saved_inventory_items
            .actual_refs()
            .ok_or_else(|| unsupported("actual savedInventoryItems field"))?;
        let body = list
            .try_borrow()
            .map_err(|_| RuntimeHostError::Borrowed("character savedInventoryItems indexOf"))?;
        Ok(body
            .iter()
            .position(|entry| match (entry, &item) {
                (None, None) => true,
                (Some(a), Some(b)) => Rc::ptr_eq(a, b),
                _ => false,
            })
            .map_or(-1, |index| index as i16))
    }
    pub(super) fn persistence_main_index(
        &self,
        id: Option<i32>,
        actual: bool,
    ) -> CharacterPersistenceResult<i32> {
        let Some(id) = id else { return Ok(-1) };
        if !actual {
            return Ok(self
                .items
                .inventory
                .borrow()
                .to_container()
                .items
                .iter()
                .position(|i| i.id() == id)
                .map_or(-1, |i| i as i32));
        }
        let item = item_for_id(id, &self.items.inventory)?;
        Ok(main_items(&self.items.inventory)?
            .iter()
            .position(|i| Rc::ptr_eq(i, &item))
            .map_or(-1, |i| i as i32))
    }
    pub(super) fn load_worn_refs(
        &mut self,
        sm: &ScriptManager,
        _chars: &mut Characters,
        r: &mut Reader<'_>,
    ) -> CharacterPersistenceResult<()> {
        use pz_scripts::{objects::items::ItemBodyLocation, registry::ResourceLocation};
        let count = super::e(r.i8())?;
        // Native does not clear the existing WornItems at this point.
        for _ in 0..count {
            let name = super::e(r.string())?;
            let location = ItemBodyLocation::get(sm, &ResourceLocation::of(&name)?);
            let index = super::e(r.i16())?;
            let list = self
                .saved_inventory_items
                .actual_refs()
                .ok_or_else(|| unsupported("actual savedInventoryItems field"))?;
            if index < 0
                || index as usize
                    >= list
                        .try_borrow()
                        .map_err(|_| {
                            RuntimeHostError::Borrowed("character worn saved-list bounds")
                        })?
                        .len()
            {
                continue;
            }
            let worn = self.items.worn_items.as_ref().ok_or_else(crate::npe)?;
            if worn
                .body_location_group()
                .try_borrow()
                .map_err(|_| RuntimeHostError::Borrowed("character worn location group"))?
                .location(location.as_ref())?
                .is_none()
            {
                continue;
            }
            let item = self
                .persistence_saved_item(index as i32, true)?
                .ok_or_else(|| {
                    unsupported("character saved-list changed during location lookup")
                })?;
            self.items
                .worn_items
                .as_mut()
                .ok_or_else(crate::npe)?
                .set_item(location, item)?;
        }
        Ok(())
    }
    pub(super) fn save_worn_refs(&self, w: &mut Writer) -> CharacterPersistenceResult<()> {
        let worn = self.items.worn_items.as_ref().ok_or_else(crate::npe)?;
        if worn.size() > 127 {
            return Err(ScriptError::new("java.lang.RuntimeException: too many worn items").into());
        }
        w.i8(worn.size() as i8);
        for entry in worn.items() {
            let location = entry.location.as_ref().ok_or_else(crate::npe)?;
            w.string(&location.location.id());
            w.i16(self.persistence_saved_index(Some(entry.item), true)?);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
