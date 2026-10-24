//! What item loading and saving need from the rest of the game.

use std::rc::Rc;

use crate::component::{ComponentCodec, OpaqueComponents};
use crate::item::InventoryItem;
use crate::template::{ItemClass, ItemTemplate};

/// Game state and lookups used by item `load`/`save`.
///
/// The defaults describe a game where every item id is creatable (with
/// an unknown script, so items stay raw records) and no load-time lookup
/// changes anything.
pub trait ItemEnv {
    /// `InventoryItemFactory.CreateItem(registryID)`'s script, or `None`
    /// when the item cannot be created (the original then skips it).
    fn template(&self, _registry_id: i16) -> Option<Rc<ItemTemplate>> {
        None
    }

    /// Whether `CreateItem(registryID)` returns an item. True by default
    /// so unknown items are kept as raw records.
    fn exists(&self, registry_id: i16) -> bool {
        let _ = registry_id;
        true
    }

    /// `WorldDictionary.getItemTypeFromID`.
    fn full_type(&self, _registry_id: i16) -> Option<String> {
        None
    }

    /// `ScriptManager.FindItem(fullType).getObsolete()`.
    fn obsolete(&self, _full_type: &str) -> bool {
        false
    }

    /// `GameTime.getWorldAgeHours()`.
    fn world_age_hours(&self) -> f64 {
        0.0
    }

    /// `BloodBodyPartType.MAX.index()` (length of `ItemVisual` arrays).
    fn blood_body_parts(&self) -> usize {
        // LOADER STUB: runtime value from the game (see pz-world SaveTables).
        0
    }

    /// The component codec (see [`crate::component`]).
    fn components(&self) -> &dyn ComponentCodec {
        &OpaqueComponents
    }

    /// Whether items of this script are decoded into fields; others stay
    /// raw records. `MapItem` (world map symbols) and
    /// `AnimalInventoryItem` (a whole `IsoAnimal`) are not ported.
    fn decodes(&self, t: &ItemTemplate) -> bool {
        !matches!(t.class, ItemClass::Map | ItemClass::Animal)
            && self.components().knows_factory_components(t)
    }

    /// `InventoryItemFactory.CreateItem(registryID)`.
    fn create_item(&self, registry_id: i16) -> Option<InventoryItem> {
        let t = self.template(registry_id)?;
        let mut item = InventoryItem::create(t.clone(), registry_id);
        item.components = self.components().factory_components(&t);
        Some(item)
    }

    /// `extraItems` load and save: id to full type and back through the
    /// `WorldDictionary` (unchanged for known items).
    fn extra_item_round_trip(&self, id: i16) -> i16 {
        id
    }

    /// `setTexture(Texture.getSharedTexture(name))` followed by the save
    /// test: the name saved again, or `None` when the texture is missing
    /// or matches the script icon.
    fn texture_round_trip(&self, _t: &ItemTemplate, name: &str) -> Option<String> {
        Some(name.to_string())
    }

    /// `RecordedMedia.getMediaDataFromIndex(index)`: `None` when unknown
    /// (the index is reset to -1), `Some(name)` with the translated
    /// display name the item takes (`Some(None)` keeps the name).
    fn recorded_media_name(&self, _index: i16) -> Option<Option<String>> {
        Some(None)
    }

    /// `Moveable.customItem != null` after `ReadFromWorldSprite`: custom
    /// moveables keep their type. True by default (no change).
    fn moveable_is_custom_item(&self, _world_sprite: &str) -> bool {
        true
    }

    /// Called when an item is kept as raw bytes, with the reason.
    fn note_raw(&self, _registry_id: i16, _reason: &str) {}
}

/// [`ItemEnv`] with every default.
pub struct NoItemEnv;
impl ItemEnv for NoItemEnv {}
