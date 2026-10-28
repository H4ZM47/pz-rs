//! `zombie.characters.ClothingWetness`: moves wetness between the body
//! parts and the worn clothing covering them.
//!
//! The original's `Clothing.updateWetness(true)` for clothing without a
//! blood clothing type is `pz-inventory`'s STUB `update_wetness`.

use pz_inventory::InventoryItem;
use pz_scripts::ScriptManager;

use super::body_part::{BodyPart, BodyPartTypes};
use super::context::SimContext;
use super::items::{
    blood_part, covered_parts, flush_wetness, has_tag, is_clothing, is_protect_from_rain_while_equipped,
    set_wetness, visual, visual_hole, water_resistance, CharacterItems,
};
use super::thermoregulator::Thermoregulator;

/// `ClothingWetness`.
#[derive(Clone, Debug, PartialEq)]
pub struct ClothingWetness {
    /// `clothingList` by `BloodBodyPartType` index: item ids, outermost
    /// first.
    clothing_list: Vec<Vec<i32>>,
    perspiring_parts: Vec<i32>,
    /// `changed`: recompute the exposed items on the next update (set it
    /// when the worn items change).
    pub changed: bool,
}

impl Default for ClothingWetness {
    fn default() -> Self {
        ClothingWetness {
            clothing_list: vec![Vec::new(); blood_part::MAX],
            perspiring_parts: vec![0; blood_part::MAX],
            changed: true,
        }
    }
}

fn wetness(item: &InventoryItem) -> f32 {
    item.get_wetness()
}

impl ClothingWetness {
    /// `calculateExposedItems`.
    pub fn calculate_exposed_items(&mut self, sm: &ScriptManager, items: &CharacterItems) {
        for l in &mut self.clothing_list {
            l.clear();
        }
        for item in items.item_visuals().into_iter().rev() {
            let item = &*item.borrow();
            let Some(parts) = covered_parts(sm, item) else { continue };
            for p in parts {
                self.clothing_list[p].push(item.id);
            }
        }
    }

    /// `updateWetness(outerWetnessInc, outerWetnessDec)`.
    // `!(x <= y)` keeps the original's handling of NaN.
    #[allow(clippy::too_many_arguments, clippy::neg_cmp_op_on_partial_ord)]
    pub fn update_wetness(
        &mut self,
        sm: &ScriptManager,
        ctx: &dyn SimContext,
        types: &BodyPartTypes,
        items: &mut CharacterItems,
        parts: &mut [BodyPart],
        thermo: &Thermoregulator,
        outer_inc: f32,
        outer_dec: f32,
    ) {
        let mut umbrella = false;
        if items.primary_hand_item().is_some_and(|i| is_protect_from_rain_while_equipped(sm, &i.borrow())) {
            umbrella = true;
        }
        if items.secondary_hand_item().is_some_and(|i| is_protect_from_rain_while_equipped(sm, &i.borrow())) {
            umbrella = true;
        }
        if self.changed {
            self.changed = false;
            self.calculate_exposed_items(sm, items);
        }
        for id in items.item_visual_ids() {
            let has_types = items.item(id).is_some_and(|i| covered_parts(sm, &i.borrow()).is_some());
            let Some(item) = items.item(id) else { continue };
            let item = &mut *item.borrow_mut();
            if !is_clothing(item) {
                continue;
            }
            if has_tag(sm, item, "BREAK_WHEN_WET") && item.get_wetness() >= 100.0 {
                // STUB: `setCondition(0)` without its sound and sync.
                item.condition = 0;
                // `onWornItemsChanged()`.
                self.changed = true;
            }
            if !has_types {
                item.update_wetness();
                continue;
            }
            flush_wetness(item);
        }
        let g = ctx.zomboid_globals();
        let base_increase = g.wetness_increase as f32 * ctx.multiplier();
        let base_decrease = g.wetness_decrease as f32 * ctx.multiplier();
        let max = types.max_index();
        'parts: for i in 0..self.clothing_list.len() {
            if i >= max {
                continue;
            }
            let Some(node) = thermo.node_index_for_blood_type(i).and_then(|n| thermo.node(n)) else { continue };
            let Some(body_part) = parts.get_mut(i) else { continue };
            let mut base_delta = 0.0f32;
            let mut perspiration = node.secondary_delta().clamp(0.0, 1.0);
            perspiration *= perspiration;
            perspiration *= 0.2 + 0.8 * (1.0 - node.dist_to_core());
            if perspiration > 0.1 {
                base_delta += perspiration;
            } else {
                let mut body_heat = (node.skin_celcius() - 20.0) / 22.0;
                body_heat *= body_heat;
                body_heat -= outer_inc;
                body_heat = body_heat.max(0.0);
                base_delta -= body_heat;
                if outer_inc > 0.0 {
                    base_delta = 0.0;
                }
            }
            self.perspiring_parts[i] = if base_delta > 0.0 { 1 } else { 0 };
            if base_delta == 0.0 {
                continue;
            }
            if base_delta > 0.0 {
                base_delta *= base_increase;
            } else {
                base_delta *= base_decrease;
            }
            body_part.set_wetness(body_part.wetness + base_delta);
            if base_delta > 0.0 && body_part.wetness < 25.0 || base_delta < 0.0 && body_part.wetness > 50.0 {
                continue;
            }
            if base_delta > 0.0 {
                let mut ext = thermo.external_air_temperature();
                ext += 10.0;
                ext = ext.clamp(0.0, 20.0) / 20.0;
                base_delta *= 0.4 + 0.6 * ext;
            }
            let (mut hole_before, mut soak_through, mut dry_before) = (false, false, false);
            for j in (0..self.clothing_list[i].len()).rev() {
                if base_delta > 0.0 {
                    self.perspiring_parts[i] += 1;
                }
                let Some(item) = items.item(self.clothing_list[i][j]) else { continue };
                let item = &mut *item.borrow_mut();
                if !is_clothing(item) {
                    continue;
                }
                let mut increase_mod = 1.0f32;
                if visual(item).is_some() {
                    if visual_hole(item, i) > 0.0 {
                        hole_before = true;
                        continue;
                    }
                    if base_delta > 0.0 && wetness(item) >= 100.0 {
                        soak_through = true;
                        continue;
                    }
                    if base_delta < 0.0 && wetness(item) <= 0.0 {
                        dry_before = true;
                        continue;
                    }
                    if base_delta > 0.0 && water_resistance(item) > 0.0 {
                        increase_mod = 0.0f32.max(1.0 - water_resistance(item));
                        if increase_mod <= 0.0 {
                            self.perspiring_parts[i] -= 1;
                            continue 'parts;
                        }
                    }
                }
                let mut delta = base_delta;
                if delta > 0.0 {
                    delta *= increase_mod;
                }
                if hole_before || soak_through || dry_before {
                    delta /= 2.0;
                }
                let w = wetness(item) + delta;
                set_wetness(item, w);
                continue 'parts;
            }
        }
        for i in 0..self.clothing_list.len() {
            if i >= max {
                continue;
            }
            if thermo.node_index_for_blood_type(i).is_none() {
                continue;
            }
            let Some(body_part) = parts.get_mut(i) else { continue };
            let mut max_wetness = 100.0f32;
            if umbrella {
                max_wetness = 100.0 * types.decls[i].umbrella_mod;
            }
            let base_delta = if outer_inc > 0.0 { outer_inc * base_increase } else { 0.0 - outer_dec * base_decrease };
            let (mut hole_above, mut soak_through, mut dry_before) = (false, false, false);
            let mut layer_divider = 2.0f32;
            let n = self.clothing_list[i].len();
            for j in 0..n {
                let part_count = 1 + n as i32 - j as i32;
                let mut increase_mod = 1.0f32;
                let id = self.clothing_list[i][j];
                let Some(num_parts) = items.item(id).map(|it| covered_parts(sm, &it.borrow()).map_or(0, |p| p.len())) else {
                    continue;
                };
                let Some(item) = items.item(id) else { continue };
                let item = &mut *item.borrow_mut();
                if !is_clothing(item) {
                    continue;
                }
                if visual(item).is_some() {
                    if visual_hole(item, i) > 0.0 {
                        hole_above = true;
                        continue;
                    }
                    if base_delta > 0.0 && wetness(item) >= 100.0 {
                        soak_through = true;
                        continue;
                    }
                    if base_delta < 0.0 && wetness(item) <= 0.0 {
                        dry_before = true;
                        continue;
                    }
                    if base_delta > 0.0 && water_resistance(item) > 0.0 {
                        increase_mod = 0.0f32.max(1.0 - water_resistance(item));
                        if increase_mod <= 0.0 {
                            break;
                        }
                    }
                }
                let mut delta = base_delta;
                if delta > 0.0 {
                    delta *= increase_mod;
                }
                delta /= num_parts as f32;
                if hole_above || soak_through || dry_before {
                    delta /= layer_divider;
                }
                if base_delta < 0.0 && part_count > self.perspiring_parts[i]
                    || base_delta > 0.0 && wetness(item) <= max_wetness
                {
                    let w = wetness(item) + delta;
                    set_wetness(item, w);
                }
                if base_delta > 0.0 {
                    break;
                }
                if !dry_before {
                    continue;
                }
                layer_divider *= 2.0;
            }
            if n == 0 {
                if !(base_delta < 0.0 && self.perspiring_parts[i] == 0) && !(body_part.wetness <= max_wetness) {
                    continue;
                }
                body_part.set_wetness(body_part.wetness + base_delta);
                continue;
            }
            let Some(bottom) = items.item(self.clothing_list[i][n - 1]) else { continue };
            let bottom = &*bottom.borrow();
            if !is_clothing(bottom) {
                continue;
            }
            let bottom_wetness = wetness(bottom);
            if base_delta > 0.0 && self.perspiring_parts[i] == 0 && bottom_wetness >= 50.0 && body_part.wetness <= max_wetness {
                body_part.set_wetness(body_part.wetness + base_delta / 2.0);
            }
            if !(base_delta < 0.0) || self.perspiring_parts[i] != 0 || !(bottom_wetness <= 50.0) {
                continue;
            }
            body_part.set_wetness(body_part.wetness + base_delta / 2.0);
        }
    }
}
