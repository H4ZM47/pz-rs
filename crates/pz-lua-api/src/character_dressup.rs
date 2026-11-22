//! `IsoGameCharacter.Dressup(desc)`: the character wears new copies of the
//! descriptor's worn items, which then go into its inventory.
use std::cell::RefCell;
use std::rc::Rc;

use pz_inventory::shared::ItemRef;
use pz_inventory::visual::ItemVisual;
use pz_inventory::worn_items::{
    add_items_to_item_container, set_from_item_visuals, InventoryWornItemsRef, WornInventoryResult,
};
use pz_kahlua::Thread;
use pz_scripts::{ScriptError, ScriptManager};

use crate::bind_character_sim::LuaGameCharacterRef;
use crate::bind_characters::LuaSurvivorDesc;
use crate::world_mannequins::InventoryHost;

fn borrowed(what: &str) -> ScriptError {
    ScriptError::new(format!("host borrow conflict: {what}"))
}

/// `SurvivorDesc.getItemVisuals(itemVisuals)` (`WornItems.getItemVisuals`):
/// the visual of each worn item in order, skipping a null `getVisual()`.
fn desc_item_visuals(desc: &Rc<RefCell<LuaSurvivorDesc>>) -> WornInventoryResult<Vec<ItemVisual>> {
    let items: Vec<ItemRef> = desc
        .try_borrow()
        .map_err(|_| borrowed("SurvivorDesc"))?
        .worn_items
        .items()
        .iter()
        .map(|w| {
            crate::bind_items::item_arg_value(&w.item.0)
                .ok_or_else(|| ScriptError::new("java.lang.ClassCastException: worn item is not an InventoryItem"))
        })
        .collect::<Result<_, _>>()?;
    let env = crate::bind_items::item_env();
    let mut out = Vec::new();
    for item in &items {
        let owner = env.owner(item);
        if env.context().assets().is_some() {
            pz_inventory::moddata::prepare_clothing_mod_data_ref(item)?;
        }
        let mut body = item.try_borrow_mut().map_err(|_| borrowed("InventoryItem"))?;
        if let Some(visual) = body.try_get_visual(env.context().assets(), owner.as_ref(), env.visual_rng())? {
            out.push(visual.clone());
        }
    }
    Ok(out)
}

/// `IsoGameCharacter.Dressup(desc)`: nothing for a zombie or without
/// `wornItems`; else `wornItems.setFromItemVisuals` with the descriptor's
/// item visuals, `wornItems.addItemsToItemContainer(inventory)`, the
/// descriptor's worn items cleared and `onWornItemsChanged()`.
///
/// The character keeps its worn items as inventory ids, so the new items are
/// worn on a list of the same group first and then mapped to their ids.
/// `ItemVisual.setInventoryItem` on the descriptor's visuals is not modelled:
/// the descriptor's items are dropped right after.
pub fn dressup(
    thread: &mut Thread,
    sm: &ScriptManager,
    chr: &LuaGameCharacterRef,
    desc: &Rc<RefCell<LuaSurvivorDesc>>,
) -> WornInventoryResult {
    let (group, inventory) = {
        let c = chr.try_borrow().map_err(|_| borrowed("IsoGameCharacter"))?;
        if c.is_zombie() {
            return Ok(());
        }
        let Some(worn) = c.items.worn_items.as_ref() else {
            return Ok(());
        };
        (worn.body_location_group().clone(), c.items.inventory.clone())
    };
    let visuals = desc_item_visuals(desc)?;
    let worn: InventoryWornItemsRef = Rc::new(RefCell::new(pz_inventory::worn_items::WornItems::new(group)));
    let mut host = InventoryHost(thread);
    set_from_item_visuals(&worn, &visuals, &mut host)?;
    let ids = worn
        .try_borrow()
        .map_err(|_| borrowed("WornItems"))?
        .try_map_items(|item| item.try_borrow().map(|i| i.id).map_err(|_| borrowed("InventoryItem")))?;
    chr.try_borrow_mut().map_err(|_| borrowed("IsoGameCharacter"))?.items.worn_items = Some(ids);
    add_items_to_item_container(&worn, &inventory, &mut host)?;
    let retired = desc
        .try_borrow_mut()
        .map_err(|_| borrowed("SurvivorDesc"))?
        .worn_items
        .take_items_for_retirement();
    if !retired.is_empty() {
        let mut pins = pz_entity::RetiredComponents::new();
        pins.push_payload(Box::new(retired));
        pz_inventory::component_host::retire_components(pins);
    }
    chr.try_borrow_mut().map_err(|_| borrowed("IsoGameCharacter"))?.on_worn_items_changed(sm);
    Ok(())
}
