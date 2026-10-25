//! `HumanVisual.dressInNamedOutfit` / `dressInOutfit` / `addClothingItem`:
//! an outfit's clothing as `ItemVisual`s, placed by body location the way
//! the original does (replacing a single-item location, removing exclusive
//! ones, ordered by the `Human` group's location order).
//!
//! The default pants and top are picked and added (with their own
//! `ClothingItemReference.randomize`) in the original's interleaved order,
//! so `OutfitRNG` draws match.

use super::outfit::{ClothingItemReference, OutfitContext};
use super::OutfitManager;
use crate::worn_items::BodyLocationGroupRef;
use pz_fs::java::is_null_or_whitespace;
use pz_inventory::visual::{HumanVisual, ItemVisual};
use pz_inventory::{ClothingItem, ClothingItems};
use pz_scripts::objects::items::{find_item, get_item_for_clothing_item};
use pz_scripts::registry::RegistryKey;
use pz_scripts::ScriptManager;
use std::rc::Rc;

/// What dressing reads besides the outfit context.
pub struct DressContext<'a> {
    pub outfit: &'a OutfitContext<'a>,
    pub outfits: &'a OutfitManager,
    /// `Item.getClothingItemAsset()` by clothing item name.
    pub clothing: &'a ClothingItems,
    pub scripts: &'a ScriptManager,
    /// `BodyLocations.getGroup("Human")`.
    pub group: &'a BodyLocationGroupRef,
}

type Key = Option<Rc<RegistryKey>>;

fn same(a: Option<&Rc<RegistryKey>>, b: Option<&Rc<RegistryKey>>) -> bool {
    match (a, b) {
        (Some(a), Some(b)) => Rc::ptr_eq(a, b) || a.location == b.location,
        (None, None) => true,
        _ => false,
    }
}

/// `ScriptItem.getBodyLocation()`, else its `canBeEquipped` field.
fn script_location(item: &pz_scripts::objects::items::Item) -> Key {
    item.body_location.clone().or_else(|| item.can_be_equipped.clone())
}

/// `getItemVisualLocations`: each visual's script location (`null` without
/// a script item).
fn item_visual_locations(ctx: &DressContext<'_>, visuals: &[ItemVisual]) -> Vec<Key> {
    visuals
        .iter()
        .map(|v| {
            find_item(ctx.scripts, &v.full_type).and_then(|i| script_location(&i.borrow()))
        })
        .collect()
}

/// `ItemVisual.synchWithOutfit(itemRef)` with the reference's clothing item.
pub fn synch_with_outfit(v: &mut ItemVisual, r: &ClothingItemReference, c: &ClothingItem) {
    let d = &r.random_data;
    v.clothing_item_name = c.name.clone();
    v.hue = d.hue;
    v.tint = Some(d.tint);
    let index = |list: &[String], s: &Option<String>| {
        s.as_ref().and_then(|s| list.iter().position(|x| x == s)).map_or(-1, |i| i as i8)
    };
    v.base_texture = index(&c.base_textures, &d.base_texture);
    v.texture_choice = index(&c.texture_choices, &d.texture_choice);
    v.decal = d.decal.clone();
}

/// `ItemVisual.copyVisualFrom(visual)`.
pub fn copy_visual_from(v: &mut ItemVisual, from: &ItemVisual) {
    v.set_hue(from.hue);
    v.tint = from.tint;
    v.base_texture = from.base_texture;
    v.texture_choice = from.texture_choice;
    if from.decal.is_some() {
        v.decal = from.decal.clone();
    }
}

/// The private `addClothingItem(itemVisuals, locations, clothingName,
/// itemRef)`: the index of the added visual.
fn add_clothing_item(
    ctx: &DressContext<'_>,
    visuals: &mut Vec<ItemVisual>,
    locations: &mut Vec<Key>,
    clothing_name: Option<&str>,
    item_ref: Option<&ClothingItemReference>,
) -> Result<Option<usize>, String> {
    if item_ref.is_some_and(|r| !r.random_data.active) {
        return Ok(None);
    }
    let Some(name) = clothing_name.filter(|n| !is_null_or_whitespace(n)) else {
        return Ok(None);
    };
    let Some(script) = get_item_for_clothing_item(ctx.scripts, name) else {
        return Ok(None);
    };
    let script = script.borrow();
    let Some(clothing) = script.clothing_item.as_deref().and_then(|n| ctx.clothing.get(n)) else {
        return Ok(None);
    };
    let Some(location) = script_location(&script) else {
        return Ok(None);
    };
    let fresh;
    let item_ref = match item_ref {
        Some(r) => r,
        None => {
            let mut r = ClothingItemReference { item_guid: Some(clothing.guid.clone()), ..Default::default() };
            r.randomize(ctx.outfit)?;
            fresh = r;
            &fresh
        }
    };
    if !item_ref.random_data.active {
        return Ok(None);
    }
    let group = ctx.group.borrow();
    let err = |e: pz_scripts::ScriptError| e.to_string();
    let Some(loc) = group.location(Some(&location)).map_err(err)? else {
        return Ok(None);
    };
    if !loc.borrow().multi_item {
        if let Some(i) = locations.iter().position(|l| same(l.as_ref(), Some(&location))) {
            visuals.remove(i);
            locations.remove(i);
        }
    }
    let mut i = 0;
    while i < visuals.len() {
        if group.is_exclusive(Some(&location), locations[i].as_ref()).map_err(err)? {
            visuals.remove(i);
            locations.remove(i);
        } else {
            i += 1;
        }
    }
    let location_index = group.index_of(Some(&location)).map_err(err)?;
    let mut insert_at = visuals.len();
    for (i, l) in locations.iter().enumerate() {
        if group.index_of(l.as_ref()).map_err(err)? > location_index {
            insert_at = i;
            break;
        }
    }
    let mut v = ItemVisual::new(&script.full_name());
    // `itemRef.getClothingItem()`: the picked item's asset.
    let picked = ctx
        .outfit
        .clothing_items
        .clothing_item(item_ref.picked_guid().unwrap_or_default())
        .flatten()
        .unwrap_or(clothing);
    synch_with_outfit(&mut v, item_ref, picked);
    visuals.insert(insert_at, v);
    locations.insert(insert_at, Some(location));
    Ok(Some(insert_at))
}

/// `HumanVisual.dressInNamedOutfit(outfitName, itemVisuals)` (`clear`
/// true): `visuals` is cleared, then the named outfit (by the owner's sex)
/// is cloned, randomized and dressed into it.
pub fn dress_in_named_outfit(
    human: &mut HumanVisual,
    female: bool,
    outfit_name: Option<&str>,
    visuals: &mut Vec<ItemVisual>,
    ctx: &DressContext<'_>,
) -> Result<(), String> {
    visuals.clear();
    let Some(name) = outfit_name.filter(|n| !is_null_or_whitespace(n)) else {
        return Ok(());
    };
    let source = if female { ctx.outfits.find_female_outfit(name) } else { ctx.outfits.find_male_outfit(name) };
    let Some(source) = source else {
        return Ok(());
    };
    let mut outfit = source.clone_outfit();
    outfit.randomize(ctx.outfit)?;
    // `dressInOutfit`.
    human.outfit = Some(outfit.name.clone());
    let mut locations = item_visual_locations(ctx, visuals);
    let defaults = &ctx.outfit.population.default_clothing;
    if outfit.pants {
        let name = if outfit.allow_pants_hue {
            defaults.pick_pants_hue()
        } else if outfit.allow_pants_tint {
            defaults.pick_pants_tint()
        } else {
            defaults.pick_pants_texture()
        };
        add_clothing_item(ctx, visuals, &mut locations, name.as_deref(), None)?;
    }
    if outfit.top && outfit.random_data.has_top {
        let rd = &outfit.random_data;
        let name = if rd.has_tshirt {
            if rd.has_tshirt_decal && outfit.is_torso_visible(ctx.outfit) && outfit.allow_tshirt_decal {
                if outfit.allow_top_tint {
                    defaults.pick_tshirt_decal_tint()
                } else {
                    defaults.pick_tshirt_decal_texture()
                }
            } else if outfit.allow_top_tint {
                defaults.pick_tshirt_tint()
            } else {
                defaults.pick_tshirt_texture()
            }
        } else if outfit.allow_top_tint {
            defaults.pick_vest_tint()
        } else {
            defaults.pick_vest_texture()
        };
        add_clothing_item(ctx, visuals, &mut locations, name.as_deref(), None)?;
    }
    let mod_id = outfit.mod_id.clone().unwrap_or_else(|| "game".to_string());
    for r in &outfit.items {
        let Some(Some(item)) = ctx.outfit.clothing_items.clothing_item(r.picked_guid().unwrap_or_default()) else {
            continue;
        };
        let Some(at) = add_clothing_item(ctx, visuals, &mut locations, Some(&item.name), Some(r))? else {
            continue;
        };
        let source = visuals[at].clone();
        for s in &item.spawn_with {
            let Some(Some(w)) = ctx.outfit.clothing_items.clothing_item(&format!("{mod_id}-{s}")) else {
                continue;
            };
            if let Some(j) = add_clothing_item(ctx, visuals, &mut locations, Some(&w.name), None)? {
                copy_visual_from(&mut visuals[j], &source);
            }
        }
    }
    Ok(())
}
