//! Native row read-state prefixes over actual item fields. Selected radio and
//! nullable MapID phases remain explicit capabilities, never fabricated misses.
use crate::shared::ItemRef;
use crate::ItemKind;
use pz_entity::runtime_load::{HostResult, RuntimeHostError};
use pz_entity::{EntityOwner, RetiredComponents};

struct Pins(RetiredComponents);
impl Pins {
    fn new(item: &ItemRef) -> Self {
        let mut pins = RetiredComponents::new();
        pins.push_owner(EntityOwner::from_ref(item));
        Self(pins)
    }
}
impl Drop for Pins {
    fn drop(&mut self) {
        crate::component_host::retire_components(std::mem::take(&mut self.0));
    }
}
fn unsupported(operation: &'static str) -> RuntimeHostError {
    RuntimeHostError::Unsupported {
        operation,
        kind: None,
    }
}
fn category(item: &ItemRef, pins: &mut Pins) -> HostResult<Option<String>> {
    let script = crate::item_texture::get_script_item_ref(item)?
        .ok_or_else(|| unsupported("InventoryItem media read null scriptItem receiver"))?;
    pins.0.push_payload(Box::new(script.clone()));
    let value = script
        .0
        .try_borrow()
        .map_err(|_| RuntimeHostError::Borrowed("InventoryItem media read script category"))?
        .recorded_media_cat
        .clone();
    Ok(value)
}
fn media_read(item: &ItemRef, seen: bool) -> HostResult<bool> {
    let mut pins = Pins::new(item);
    let index = item
        .try_borrow()
        .map_err(|_| RuntimeHostError::Borrowed("InventoryItem media read recordedMediaIndex"))?
        .recorded_media_index;
    if index < 0 {
        return Ok(false);
    }
    if category(item, &mut pins)?.is_none() {
        return Ok(false);
    }
    // Native re-reads the current field for each Objects.equals argument. No
    // script/template/category cache may replace these separate field phases.
    let retail = category(item, &mut pins)?.as_deref() == Some("Retail-VHS");
    let video = retail || category(item, &mut pins)?.as_deref() == Some("Home-VHS");
    if video != seen {
        return Ok(false);
    }
    // PC55 requires the genuine class's nullable actual singleton; absence of
    // the host service is not evidence of a null Radio or empty media store.
    Err(unsupported(if seen {
        "InventoryItem.hasBeenSeen actual ZomboidRadio context"
    } else {
        "InventoryItem.hasBeenHeard actual ZomboidRadio context"
    }))
}
/// The nullable player argument is first converted by the actual frontend.
/// This admitted prefix never reads it before the selected radio capability.
pub fn has_been_seen_ref(item: &ItemRef) -> HostResult<bool> {
    media_read(item, true)
}
pub fn has_been_heard_ref(item: &ItemRef) -> HostResult<bool> {
    media_read(item, false)
}

/// MapItem.getMediaId's supported actual stash prefix. The caller must first
/// select the immutable actual Map class. Archive map_id is not nullable native
/// field authority and is never used as a fallback.
pub fn map_media_id_ref(item: &ItemRef) -> HostResult<Option<String>> {
    let _pins = Pins::new(item);
    let present = {
        let body = item
            .try_borrow()
            .map_err(|_| RuntimeHostError::Borrowed("MapItem media-id stash field"))?;
        if !matches!(&body.kind, ItemKind::Map { .. }) {
            return Err(RuntimeHostError::InvalidHostState(
                "MapItem media-id receiver kind",
            ));
        }
        body.stash_map.is_some()
    };
    if present {
        let value = item
            .try_borrow()
            .map_err(|_| RuntimeHostError::Borrowed("MapItem media-id stash reread"))?
            .stash_map
            .clone();
        return Ok(value);
    }
    Err(unsupported("MapItem.getMapID actual nullable authority"))
}
#[cfg(test)]
mod tests;
