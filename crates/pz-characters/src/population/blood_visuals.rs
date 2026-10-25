//! `BloodClothingType.addBlood` / `addDirt` / `addHole` over a character's
//! `HumanVisual` and clothing `ItemVisual`s (the visuals a zombie wears
//! carry no `InventoryItem`, so the `Clothing` branches never apply).

use pz_core::random::{self as rand, outfit_rng};
use pz_inventory::visual::{HumanVisual, ItemVisual};
use pz_scripts::objects::items::get_item;
use pz_scripts::ScriptManager;

use crate::character::items::{blood_clothing_covered_parts, blood_part};

/// `BloodBodyPartType.MAX.index()`.
const MAX: usize = blood_part::MAX;

/// What the item visual's script says: the parts its blood clothing types
/// cover (`None` without a script or types) and `canHaveHoles`.
fn script_parts(sm: &ScriptManager, v: &ItemVisual) -> Option<(Vec<usize>, bool)> {
    if v.full_type.trim().is_empty() {
        return None;
    }
    let s = get_item(sm, &v.full_type)?;
    let s = s.borrow();
    let types = s.blood_clothing_type.as_ref()?;
    let parts = types.iter().filter_map(|t| blood_clothing_covered_parts(t)).flatten().collect();
    Some((parts, s.can_have_holes))
}

/// An item visual covers `part` and has no hole there.
fn covers(sm: &ScriptManager, v: &ItemVisual, part: usize) -> bool {
    script_parts(sm, v).is_some_and(|(p, _)| p.contains(&part)) && v.get_hole(part) == 0.0
}

/// The intensity `addBlood(part, humanVisual, itemVisuals, allLayers)` and
/// `addDirt(...)` draw from `OutfitRNG` by `SandboxOptions.clothingDegradation`.
pub fn degradation_intensity(clothing_degradation: i32) -> f32 {
    match clothing_degradation {
        2 => outfit_rng::next_float(0.001, 0.01),
        3 => outfit_rng::next_float(0.05, 0.1),
        4 => outfit_rng::next_float(0.01, 0.05),
        _ => 0.0,
    }
}

fn add(
    sm: &ScriptManager,
    part: usize,
    intensity: f32,
    hv: &mut HumanVisual,
    visuals: &mut [ItemVisual],
    all_layers: bool,
    blood: bool,
) {
    type Get = fn(&HumanVisual, usize) -> f32;
    type Set = fn(&mut HumanVisual, usize, f32);
    let (get_hv, set_hv): (Get, Set) = if blood {
        (HumanVisual::get_blood, HumanVisual::set_blood)
    } else {
        (HumanVisual::get_dirt, HumanVisual::set_dirt)
    };
    let get_v = |v: &ItemVisual| if blood { v.get_blood(part) } else { v.get_dirt(part) };
    let set_v = |v: &mut ItemVisual, a: f32| if blood { v.set_blood(part, a, MAX) } else { v.set_dirt(part, a, MAX) };
    // `addBlood` rolls on `OutfitRNG`, `addDirt` on `Rand`.
    let next_bool = |n: i32| if blood { outfit_rng::next_bool(n) } else { rand::next(n) == 0 };
    if !all_layers {
        match visuals.iter().rposition(|v| covers(sm, v, part)) {
            Some(i) => {
                if intensity > 0.0 {
                    let a = get_v(&visuals[i]) + intensity;
                    set_v(&mut visuals[i], a);
                }
            }
            None => {
                let a = get_hv(hv, part) + 0.05;
                set_hv(hv, part, a);
            }
        }
        return;
    }
    let a = get_hv(hv, part) + 0.05;
    set_hv(hv, part, a);
    let mut current = get_hv(hv, part);
    if next_bool(((current * 100.0) as i32 - 100).abs()) {
        return;
    }
    for v in visuals.iter_mut() {
        if !covers(sm, v, part) {
            continue;
        }
        if intensity > 0.0 {
            let a = get_v(v) + intensity;
            set_v(v, a);
            current = get_v(v);
        }
        if next_bool(((current * 100.0) as i32 - 100).abs()) {
            break;
        }
    }
}

/// `BloodClothingType.addBlood(part, intensity, humanVisual, itemVisuals,
/// allLayers)`.
pub fn add_blood(sm: &ScriptManager, part: usize, intensity: f32, hv: &mut HumanVisual, visuals: &mut [ItemVisual], all_layers: bool) {
    add(sm, part, intensity, hv, visuals, all_layers, true);
}

/// `BloodClothingType.addDirt(part, intensity, humanVisual, itemVisuals,
/// allLayers)`.
pub fn add_dirt(sm: &ScriptManager, part: usize, intensity: f32, hv: &mut HumanVisual, visuals: &mut [ItemVisual], all_layers: bool) {
    add(sm, part, intensity, hv, visuals, all_layers, false);
}

/// `BloodClothingType.addHole(part, humanVisual, itemVisuals, allLayers)`
/// for visuals without inventory items: the outermost covering item that
/// can have holes gets one (every layer with `all_layers`); the skin gets
/// one when nothing covered the part, or always with `all_layers`.
pub fn add_hole(sm: &ScriptManager, part: usize, hv: &mut HumanVisual, visuals: &mut [ItemVisual], all_layers: bool) -> bool {
    let mut hit = false;
    let mut added = false;
    for i in (0..visuals.len()).rev() {
        let Some((parts, can_have_holes)) = script_parts(sm, &visuals[i]) else {
            continue;
        };
        if !(parts.contains(&part) && visuals[i].get_hole(part) == 0.0) {
            continue;
        }
        hit = true;
        if can_have_holes {
            visuals[i].set_hole(part, MAX);
            added = true;
        }
        if !all_layers {
            break;
        }
        hit = false;
    }
    if !hit || all_layers {
        hv.set_hole(part);
    }
    added
}
