//! The subclasses' `DoTooltip(tooltipUI, layout)` overrides: `Food`,
//! `Clothing`, `HandWeapon`, `Literature`, `WeaponPart`, `AlarmClock`,
//! `AlarmClockClothing`, `InventoryContainer` and `AnimalInventoryItem`
//! (`ComboItem`, `DrainableComboItem`, `Key`, `KeyRing`, `MapItem`,
//! `Moveable` and `Radio` add nothing), and `InventoryContainer`'s icons
//! of its contents.

use super::chr;
use super::{
    created_display_name, fstr, int_str, item_name, item_texture, mod_has, mod_num, mod_set,
    mod_str, mod_value, npe, script, script_display_name, sv, tr, tr_args, trc, Cx, Row, Tip, LABEL, WHITE,
};
use crate::bind_items::{item_env, ItemRef};
use pz_inventory::item::{ClothingData, FoodData, ItemKind};
use pz_inventory::{InventoryItem, ItemClass};
use pz_kahlua::{LuaResult, Value};
use pz_ui::tooltip::{Layout, ObjectTooltip};
use std::collections::HashSet;

#[cfg(test)]
#[path = "kinds/moddata_tests.rs"]
mod moddata_tests;

/// The subclass's `DoTooltip(tooltipUI, layout)`.
pub fn do_tooltip(
    cx: &mut Cx<'_>,
    item: &ItemRef,
    tip: &Tip<'_>,
    layout: &mut Layout,
) -> LuaResult<()> {
    let class = item.borrow().template.class;
    let kind = std::mem::discriminant(&item.borrow().kind);
    let _ = kind;
    match class {
        ItemClass::Food => food(cx, item, layout),
        ItemClass::Clothing => clothing(cx, item, tip, layout),
        ItemClass::Weapon => weapon(cx, item, layout),
        ItemClass::Literature => literature(cx, item, layout),
        ItemClass::WeaponPart => {
            weapon_part(cx, &item.borrow(), layout);
            Ok(())
        }
        ItemClass::AlarmClock | ItemClass::AlarmClockClothing => {
            alarm_clock(&item.borrow(), layout);
            Ok(())
        }
        ItemClass::Container => container(cx, item, layout),
        ItemClass::Animal => animal(cx, item, layout),
        _ => Ok(()),
    }
}

fn good(cx: &Cx<'_>) -> [f32; 4] {
    [cx.good[0], cx.good[1], cx.good[2], 1.0]
}

fn bad(cx: &Cx<'_>) -> [f32; 4] {
    [cx.bad[0], cx.bad[1], cx.bad[2], 1.0]
}

fn rgb1(c: [f32; 4]) -> [f32; 4] {
    [c[0], c[1], c[2], 1.0]
}

/// `Food.getThirstChange()`.
fn thirst_change(it: &InventoryItem, f: &FoodData) -> f32 {
    if f.burnt {
        return f.thirst_change / 5.0;
    }
    if it.is_cooked() {
        return f.thirst_change / 2.0;
    }
    f.thirst_change
}

/// `Food.getEnduranceChange()`.
fn endurance_change(it: &InventoryItem, f: &FoodData) -> f32 {
    if f.burnt {
        return f.end_change / 3.0;
    }
    if f.age >= f.off_age as f32 && f.age < f.off_age_max as f32 {
        return f.end_change / 2.0;
    }
    if it.is_cooked() {
        return f.end_change * 2.0;
    }
    f.end_change
}

/// `Food.DoTooltip`.
fn food(cx: &mut Cx<'_>, item: &ItemRef, layout: &mut Layout) -> LuaResult<()> {
    {
        let it = item.borrow();
        let ItemKind::Food(f) = &it.kind else {
            return Ok(());
        };
        let hunger = it.get_hunger_change();
        if hunger != 0.0 && !it.has_tag("base:hidehungerchange") {
            layout
                .add_item()
                .label(&trc("Tooltip_food_Hunger"), LABEL)
                .set_value_right((hunger * 100.0) as i32, false);
        }
        let thirst = thirst_change(&it, f);
        if thirst != 0.0 {
            let row = layout.add_item();
            row.label(&trc("Tooltip_food_Thirst"), LABEL);
            let v = thirst * -2.0;
            if v > 0.0 {
                row.progress(v, good(cx));
            } else {
                row.progress(-v, bad(cx));
            }
        }
        let end = endurance_change(&it, f);
        if end != 0.0 {
            let row = layout.add_item();
            let v = (end * 100.0) as i32;
            row.label(&trc("Tooltip_food_Endurance"), LABEL);
            if v > 0 {
                row.progress(v as f32 / 10.0, good(cx));
            } else {
                row.progress((-v) as f32 / 10.0, bad(cx));
            }
        }
        let stress = it.get_stress_change();
        if stress != 0.0 {
            let row = layout.add_item();
            let v = stress * 2.0;
            row.label(&trc("Tooltip_food_Stress"), LABEL);
            if stress < 0.0 {
                row.progress(-v, good(cx));
            } else {
                row.progress(v, bad(cx));
            }
        }
        let boredom = it.get_boredom_change();
        if boredom != 0.0 {
            let row = layout.add_item();
            let v = boredom * -0.02;
            row.label(&trc("Tooltip_food_Boredom"), LABEL);
            if v > 0.0 {
                row.progress(v, good(cx));
            } else {
                row.progress(-v, bad(cx));
            }
        }
        let unhappy = it.get_unhappy_change();
        if unhappy != 0.0 {
            let row = layout.add_item();
            let v = unhappy * -0.02;
            row.label(&trc("Tooltip_food_Unhappiness"), LABEL);
            if v > 0.0 {
                row.progress(v, good(cx));
            } else {
                row.progress(-v, bad(cx));
            }
        }
        let mtc = sv(&it, |s| s.minutes_to_cook) as f32;
        let mtb = sv(&it, |s| s.minutes_to_burn) as f32;
        if f.is_cookable && !f.frozen && !f.burnt && (f.heat as f64) > 1.6 {
            let ct = f.cooking_time;
            let mut fr = ct / mtc;
            let mut h = cx.good;
            let mut s = tr("IGUI_invpanel_Cooking");
            if ct > mtc {
                h = cx.bad;
                s = tr("IGUI_invpanel_Burning");
                fr = (ct - mtc) / (mtb - mtc);
            }
            layout
                .add_item()
                .label(&format!("{s}: "), rgb1(h))
                .progress(fr, rgb1(h));
        }
        if f.freezing_time < 100.0 && f.freezing_time > 0.0 {
            layout
                .add_item()
                .label(&format!("{}: ", tr("IGUI_invpanel_FreezingTime")), LABEL)
                .progress(f.freezing_time / 100.0, [0.0, 0.6, 0.0, 0.7]);
        }
        let (fert_time, hatch) = f
            .egg
            .as_ref()
            .map_or((0, 0), |e| (e.fertilized_time, e.time_to_hatch));
        if cx.debug && f.fertilized {
            layout
                .add_item()
                .label("Fertilized :", LABEL)
                .value(&format!("{fert_time}/{hatch}"), WHITE);
        } else if f.fertilized && (hatch as f64) / (fert_time as f64) < 4.0 {
            layout
                .add_item()
                .label(&tr("Tooltip_food_fertilized"), [0.5, 1.0, 0.4, 1.0]);
        }
    }
    // `character != null & character.getPerkLevel(...)`: the non-short-
    // circuit `&` dereferences a null character.
    let chr = cx.chr.clone();
    if chr.is_nil() {
        return Err(npe());
    }
    let pro_cook = chr::perk_level(cx.t, &chr, "Cooking")? > 4;
    let illiterate = chr::has_trait(cx.t, &chr, "ILLITERATE")?;
    let player = cx.player.clone();
    let too_dark = !player.is_nil() && chr::too_dark_to_read(cx.t, &player)?;
    let nutritionist =
        chr::has_trait(cx.t, &chr, "NUTRITIONIST")? || chr::has_trait(cx.t, &chr, "NUTRITIONIST2")?;
    let no_label = mod_has(item, "NoLabel")?;
    let it = item.borrow();
    let ItemKind::Food(f) = &it.kind else {
        return Ok(());
    };
    let packaged = sv(&it, |s| s.packaged);
    let can_read_package = packaged && !illiterate && !too_dark && !no_label;
    if pro_cook && f.is_cookable {
        layout
            .add_item()
            .label(&trc("Tooltip_food_MinutesToCook"), LABEL)
            .set_value_right_no_plus_float(sv(&it, |s| s.minutes_to_cook) as f32);
    }
    if cx.tooltip_info() || can_read_package || nutritionist {
        layout
            .add_item()
            .label(&trc("Tooltip_food_Nutrition"), LABEL);
        for (k, v) in [
            ("Tooltip_food_Calories", f.calories),
            ("Tooltip_food_Carbs", f.carbohydrates),
            ("Tooltip_food_Prots", f.proteins),
            ("Tooltip_food_Fat", f.lipids),
        ] {
            layout
                .add_item()
                .label(&trc(k), LABEL)
                .set_value_right_no_plus_float(v);
        }
    } else if packaged && illiterate {
        layout
            .add_item()
            .label(&trc("Tooltip_food_Nutrition"), LABEL);
        layout
            .add_item()
            .label(&tr("ContextMenu_Illiterate"), LABEL);
    } else if packaged && too_dark {
        layout
            .add_item()
            .label(&trc("Tooltip_food_Nutrition"), LABEL);
        layout.add_item().label(&tr("ContextMenu_TooDark"), LABEL);
    }
    if f.dangerous_uncooked && !f.cooked && !f.burnt {
        let row = layout.add_item();
        row.label(&tr("Tooltip_food_Dangerous_uncooked"), bad(cx));
        if it.has_tag("base:egg") {
            row.label(
                &tr("Tooltip_food_SlightDanger_uncooked"),
                [1.0, 0.0, 0.0, 1.0],
            );
        }
    }
    if sv(&it, |s| s.remove_unhappiness_when_cooked) && !f.cooked && !f.burnt {
        layout
            .add_item()
            .label(&tr("Tooltip_food_CookToRemoveUnhappiness"), bad(cx));
    }
    if (it.template.good_hot || it.template.bad_cold) && f.heat < 1.3 {
        layout
            .add_item()
            .label(&tr("Tooltip_food_BetterHot"), [1.0, 0.9, 0.9, 1.0]);
    }
    if f.cooked_in_microwave {
        layout
            .add_item()
            .label(&tr("Tooltip_food_CookedInMicrowave"), [1.0, 0.9, 0.9, 1.0]);
    }
    if let Some(m) = f.milk_type.as_deref().filter(|m| !m.is_empty()) {
        layout
            .add_item()
            .label(
                &format!("{}: ", tr("Tooltip_food_MilkType")),
                [1.0, 0.9, 0.9, 1.0],
            )
            .value(&tr(&format!("Tooltip_food_{m}")), [1.0, 0.9, 0.9, 1.0]);
    }
    if cx.tooltip_info() {
        let dbg = [0.0, 1.0, 0.0, 1.0];
        layout
            .add_item()
            .label("DBG: BaseHunger", dbg)
            .set_value_right((f.base_hunger * 100.0) as i32, false);
        layout
            .add_item()
            .label("DBG: Age", dbg)
            .set_value_right_no_plus_float(f.age * 24.0);
        if f.off_age_max as f64 != 1.0e9 {
            layout
                .add_item()
                .label("DBG: Age Fresh", dbg)
                .set_value_right_no_plus_float(f.off_age as f32 * 24.0);
            layout
                .add_item()
                .label("DBG: Age Rotten", dbg)
                .set_value_right_no_plus_int(f.off_age_max * 24);
        }
        layout
            .add_item()
            .label("DBG: Heat", dbg)
            .set_value_right_no_plus_float(f.heat);
        layout
            .add_item()
            .label("DBG: Freeze Time", dbg)
            .set_value_right_no_plus_float(f.freezing_time);
        layout
            .add_item()
            .label("DBG: Compost Time", dbg)
            .set_value_right_no_plus_float(f.compost_time);
    }
    Ok(())
}

fn clothing_data(it: &InventoryItem) -> Option<&ClothingData> {
    match &it.kind {
        ItemKind::Clothing(c) | ItemKind::AlarmClockClothing(c, _) => Some(c),
        _ => None,
    }
}

/// One defence's rows when the item is not worn: the new value against
/// the worn items at the same (or an exclusive) location.
fn defence_rows(cx: &Cx<'_>, layout: &mut Layout, key: &str, new: f32, previous: f32) {
    if new != previous {
        let row = layout.add_item();
        if new > 0.0 || previous > 0.0 {
            row.label(&trc(key), LABEL);
            if new > previous {
                row.value(
                    &format!("{} (+{})", new as i32, (new - previous) as i32),
                    good(cx),
                );
            } else {
                row.value(
                    &format!("{} (-{})", new as i32, (previous - new) as i32),
                    bad(cx),
                );
            }
        }
    } else if new != 0.0 {
        layout
            .add_item()
            .label(&trc(key), LABEL)
            .set_value_right_no_plus_int(new as i32);
    }
}

/// `Clothing.DoTooltip`.
fn clothing(cx: &mut Cx<'_>, item: &ItemRef, tip: &Tip<'_>, layout: &mut Layout) -> LuaResult<()> {
    let e = item_env();
    let owner = e.owner(item);
    {
        let it = item.borrow();
        let Some(c) = clothing_data(&it) else {
            return Ok(());
        };
        if !it.template.cosmetic {
            let f = it.condition as f32 / it.condition_max as f32;
            layout
                .add_item()
                .label(&trc("Tooltip_weapon_Condition"), LABEL)
                .progress(f, rgb1(cx.bad_to_good(f)));
            let f = it.get_insulation();
            layout
                .add_item()
                .label(&format!("{}: ", tr("Tooltip_item_Insulation")), LABEL)
                .progress(f, rgb1(cx.bad_to_good(f)));
            let f = it.get_windresistance();
            if f > 0.0 {
                layout
                    .add_item()
                    .label(&format!("{}: ", tr("Tooltip_item_Windresist")), LABEL)
                    .progress(f, rgb1(cx.bad_to_good(f)));
            }
            let f = it.get_water_resistance();
            if f > 0.0 {
                layout
                    .add_item()
                    .label(&format!("{}: ", tr("Tooltip_item_Waterresist")), LABEL)
                    .progress(f, rgb1(cx.bad_to_good(f)));
            }
        }
        if c.blood_level != 0.0 {
            let f = c.blood_level / 100.0;
            layout
                .add_item()
                .label(&trc("Tooltip_clothing_bloody"), LABEL)
                .progress(f, rgb1(cx.good_to_bad(f)));
        }
        if c.dirtyness >= 1.0 {
            let f = c.dirtyness / 100.0;
            layout
                .add_item()
                .label(&trc("Tooltip_clothing_dirty"), LABEL)
                .progress(f, rgb1(cx.good_to_bad(f)));
        }
        if c.wetness != 0.0 {
            let f = c.wetness / 100.0;
            layout
                .add_item()
                .label(&trc("Tooltip_clothing_wet"), LABEL)
                .progress(f, rgb1(cx.good_to_bad(f)));
        }
    }
    // getVisual(): null without a ready ClothingItem, and getHole throws.
    if e.context().assets().is_some() { crate::bind_items::ensure_clothing_mod_data(item)?; }
    let visual_result = (|| -> LuaResult<i32> {
        let mut it = item.try_borrow_mut().map_err(|_| pz_inventory::moddata::ModDataError::Borrowed("item tooltip body").to_lua_error())?;
        let assets = e.context().assets();
        match it.try_get_visual(assets, owner.as_ref(), e.visual_rng()).map_err(|error| error.to_lua_error())? {
            Some(v) => Ok(v.get_holes_number()),
            None => return Err(npe()),
        }
    })();
    pz_inventory::shared::observe_item(item);
    let holes = visual_result?;
    if holes > 0 {
        layout
            .add_item()
            .label(&trc("Tooltip_clothing_holes"), LABEL)
            .set_value_right_no_plus_int(holes);
    }
    let equipped = item.borrow().is_equipped(owner.as_ref());
    let chr = cx.chr.clone();
    let (bite, scratch, bullet) = {
        let it = item.borrow();
        (
            it.get_bite_defense(),
            it.get_scratch_defense(),
            it.get_bullet_defense(),
        )
    };
    if !equipped && !chr.is_nil() {
        let (worn, worn_v) = chr::worn_items(cx.t, &chr)?;
        let loc = item.borrow().get_body_location().map(str::to_string);
        let (mut pb, mut ps, mut pu) = (0.0f32, 0.0f32, 0.0f32);
        for w in worn {
            let Some(wl) = w.location.as_deref() else {
                continue;
            };
            if !super::is_clothing(&w.item.borrow()) {
                continue;
            }
            // this.getBodyLocation().equals(...): a null location throws.
            let Some(l) = loc.as_deref() else {
                return Err(npe());
            };
            if l != wl && !chr::is_exclusive(cx.t, &worn_v, l, wl)? {
                continue;
            }
            let wi = w.item.borrow();
            pb += wi.get_bite_defense();
            ps += wi.get_scratch_defense();
            pu += wi.get_bullet_defense();
        }
        defence_rows(cx, layout, "Tooltip_BiteDefense", bite, pb);
        defence_rows(cx, layout, "Tooltip_ScratchDefense", scratch, ps);
        defence_rows(cx, layout, "Tooltip_BulletDefense", bullet, pu);
    } else {
        for (k, v) in [
            ("Tooltip_BiteDefense", bite),
            ("Tooltip_ScratchDefense", scratch),
            ("Tooltip_BulletDefense", bullet),
        ] {
            if v != 0.0 {
                layout
                    .add_item()
                    .label(&trc(k), LABEL)
                    .set_value_right_no_plus_int(v as i32);
            }
        }
    }
    let _ = tip;
    crate::bind_items::ensure_clothing_mod_data(item)?;
    let result = (|| -> LuaResult<()> {
    let mut it = item.try_borrow_mut().map_err(|_| pz_inventory::moddata::ModDataError::Borrowed("item tooltip body").to_lua_error())?;
    if it.has_tag("base:gasmask")
        || it.has_tag("base:respirator")
        || it.has_tag("base:improvisedgasmask")
    {
        if it.try_has_filter().map_err(|error| error.to_lua_error())? {
            let name = it
                .try_get_filter_type().map_err(|error| error.to_lua_error())?
                .as_deref().and_then(find_script_display_name)
                .ok_or_else(npe)?;
            let v = it.try_get_used_delta().map_err(|error| error.to_lua_error())?;
            layout
                .add_item()
                .label(&format!("{}:", tr(&name)), LABEL)
                .progress(v, rgb1(cx.bad_to_good(v)));
        } else {
            layout.add_item().label(&tr("Tooltip_NoFilter"), LABEL);
        }
    } else if it.has_tag("base:gasmasknofilter") || it.has_tag("base:respiratornofilter") {
        layout.add_item().label(&tr("Tooltip_NoFilter"), LABEL);
    }
    if it.has_tag("base:scba") {
        if it.try_has_tank().map_err(|error| error.to_lua_error())? {
            let name = it
                .try_get_tank_type().map_err(|error| error.to_lua_error())?
                .as_deref().and_then(find_script_display_name)
                .ok_or_else(npe)?;
            let v = it.try_get_used_delta().map_err(|error| error.to_lua_error())?;
            layout
                .add_item()
                .label(&format!("{}:", tr(&name)), LABEL)
                .progress(v, rgb1(cx.bad_to_good(v)));
        } else {
            layout.add_item().label(&tr("Tooltip_NoTank"), LABEL);
        }
    } else if it.has_tag("base:scbanotank") {
        layout.add_item().label(&tr("Tooltip_NoTank"), LABEL);
    }
    let run = sv(&it, |s| s.run_speed_modifier);
    let combat = sv(&it, |s| s.combat_speed_modifier);
    if run != 1.0 {
        let row = layout.add_item();
        row.label(&trc("Tooltip_RunSpeedModifier"), LABEL);
        if run > 1.0 {
            row.progress(run - 1.0, good(cx));
        } else {
            row.progress(1.0 - run, bad(cx));
        }
    }
    if combat != 1.0 {
        let row = layout.add_item();
        row.label(&trc("Tooltip_CombatSpeedModifier"), LABEL);
        // The original tests the run speed modifier here.
        if run > 1.0 {
            row.progress(combat - 1.0, good(cx));
        } else {
            row.progress(1.0 - combat, bad(cx));
        }
    }
    if cx.tooltip_info() {
        let c = clothing_data(&it).cloned();
        if let Some(c) = c {
            for (l, v) in [
                ("DBG: bloodLevel:", c.blood_level),
                ("DBG: dirtyness:", c.dirtyness),
                ("DBG: wetness:", c.wetness),
            ] {
                if v != 0.0 {
                    layout
                        .add_item()
                        .label(l, LABEL)
                        .set_value_right(v.ceil() as i32, false);
                }
            }
        }
    }
    Ok(())
    })();
    pz_inventory::shared::observe_item(item);
    result
}

/// `ScriptManager.instance.getItem(name).getDisplayName()`.
fn find_script_display_name(name: &str) -> Option<String> {
    let s = super::find_script(name)?;
    let s = s.borrow();
    Some(script_display_name(&s))
}

/// `HandWeapon.DoTooltip`.
fn weapon(cx: &mut Cx<'_>, item: &ItemRef, layout: &mut Layout) -> LuaResult<()> {
    if super::components::has_sharpness_ref(item)
        .map_err(|error| pz_kahlua::LuaError::runtime(format!("{error:?}")))?
    {
        let f = super::components::sharpness_mut(item)
            .map_err(|error| pz_kahlua::LuaError::runtime(format!("{error:?}")))?;
        layout
            .add_item()
            .label(&trc("Tooltip_weapon_Sharpness"), LABEL)
            .progress(f, rgb1(cx.bad_to_good(f)));
    }
    {
        let it = item.borrow();
        let head = super::components::has_head_condition(&it);
        let key = if head {
            "Tooltip_weapon_HandleCondition"
        } else {
            "Tooltip_weapon_Condition"
        };
        let f = it.condition as f32 / it.condition_max as f32;
        layout
            .add_item()
            .label(&trc(key), LABEL)
            .progress(f, rgb1(cx.bad_to_good(f)));
        if head {
            let f = super::components::head_condition(&it) as f32
                / super::components::head_condition_max(&it) as f32;
            layout
                .add_item()
                .label(&trc("Tooltip_weapon_HeadCondition"), LABEL)
                .progress(f, rgb1(cx.bad_to_good(f)));
        }
        let max = max_damage(&it);
        if max > 0.0 {
            let f = (max + it.get_min_damage()) / 5.0;
            layout
                .add_item()
                .label(&trc("Tooltip_weapon_Damage"), LABEL)
                .progress(f, rgb1(cx.bad_to_good(f)));
        }
        if let ItemKind::Weapon(w) = &it.kind {
            if w.blood_level != 0.0 {
                let f = w.blood_level;
                layout
                    .add_item()
                    .label(&trc("Tooltip_clothing_bloody"), LABEL)
                    .progress(f, rgb1(cx.good_to_bad(f)));
            }
        }
    }
    let fishing_rod = item.borrow().has_tag("base:fishingrod");
    if fishing_rod {
        fishing_rows(cx, item, layout)?;
    }
    let ranged = item.borrow().is_ranged();
    if ranged {
        let owner = chr::player_instance();
        if owner.is_nil() {
            // getMaxRange(IsoPlayer.getInstance()) on a null owner, after
            // the row's label.
            layout.add_item().label(&trc("Tooltip_weapon_Range"), LABEL);
            return Err(npe());
        }
        let aiming = chr::perk_level(cx.t, &owner, "Aiming")?;
        let it = item.borrow();
        let max_range = match &it.kind {
            ItemKind::Weapon(w) => w.max_range,
            _ => 0.0,
        };
        let range = max_range + sv(&it, |s| s.aiming_perk_range_modifier) * (aiming as f32 / 2.0);
        let f = range / 40.0;
        layout
            .add_item()
            .label(&trc("Tooltip_weapon_Range"), LABEL)
            .progress(f, rgb1(cx.bad_to_good(f)));
    }
    let it = item.borrow();
    let ItemKind::Weapon(w) = &it.kind else {
        return Ok(());
    };
    if it.is_two_hand_weapon() && !sv(&it, |s| s.requires_equipped_both_hands) {
        layout
            .add_item()
            .label(&tr("Tooltip_item_TwoHandWeapon"), LABEL);
    }
    if let Some(m) = w.fire_mode.as_deref().filter(|m| !m.is_empty()) {
        layout
            .add_item()
            .label(&trc("Tooltip_item_FireMode"), LABEL)
            .value(&tr(&format!("ContextMenu_FireMode_{m}")), WHITE);
    }
    if sv(&it, |s| s.cant_attack_with_lowest_endurance) {
        layout
            .add_item()
            .label(&tr("Tooltip_weapon_Unusable_at_max_exertion"), bad(cx));
    }
    let max_ammo = sv(&it, |s| s.max_ammo);
    let magazine = it.template.magazine_type.clone();
    if max_ammo > 0 {
        let mut current = it.current_ammo_count.to_string();
        if w.round_chambered {
            current += "+1";
        }
        let bullet = match magazine.as_deref() {
            Some(m) => created_display_name(m),
            None => sv(&it, |s| {
                s.ammo_type.as_ref().and_then(|k| k.item_key.clone())
            })
            .and_then(|k| created_display_name(&k)),
        }
        .ok_or_else(npe)?;
        layout
            .add_item()
            .label(&format!("{bullet}:"), LABEL)
            .value(&format!("{current} / {max_ammo}"), WHITE);
    }
    if w.is_jammed {
        layout
            .add_item()
            .label(&tr("Tooltip_weapon_Jammed"), bad(cx));
    } else if it.template.have_chamber && !w.round_chambered && it.current_ammo_count > 0 {
        let key = if w.spent_round_chambered {
            "Tooltip_weapon_SpentRoundChambered"
        } else {
            "Tooltip_weapon_NoRoundChambered"
        };
        layout.add_item().label(&tr(key), bad(cx));
    } else if w.spent_round_count > 0 {
        layout
            .add_item()
            .label(&trc("Tooltip_weapon_SpentRounds"), bad(cx))
            .value(&format!("{} / {}", w.spent_round_count, max_ammo), WHITE);
    }
    if magazine.as_deref().is_some_and(|m| !m.is_empty()) {
        let key = if w.contains_clip {
            "Tooltip_weapon_ContainsClip"
        } else {
            "Tooltip_weapon_NoClip"
        };
        layout.add_item().label(&tr(key), LABEL);
    }
    for part in &w.attachments {
        let part_type = sv(part, |s| s.part_type.clone()).unwrap_or_else(|| "null".into());
        let name = item_name(&mut part.clone());
        layout
            .add_item()
            .label(&trc(&format!("Tooltip_weapon_{part_type}")), LABEL)
            .value(&name, WHITE);
        battery_rows(cx, part, layout);
    }
    if it.has_tag("base:nomaintenancexp") {
        layout
            .add_item()
            .label(&tr("Tooltip_weapon_NoMaintenanceXp"), LABEL);
    }
    Ok(())
}

/// `HandWeapon.getMaxDamage()`: a sharp weapon's damage above the
/// minimum scales with `getSharpnessMultiplier()`.
fn max_damage(it: &InventoryItem) -> f32 {
    let max = it.get_max_damage();
    let min = it.get_min_damage();
    match super::components::raw_sharpness(it) {
        Some(s) if max > min => (max - min) * ((s + 1.0) / 2.0) + min,
        _ => max,
    }
}

/// The fishing rod's line, hook and bait rows (setting the defaults in
/// the mod data as the original does).
fn fishing_rows(cx: &Cx<'_>, item: &ItemRef, layout: &mut Layout) -> LuaResult<()> {
    let row = layout.add_item();
    row.label(&trc("Tooltip_fishing_line_Condition"), LABEL);
    let f = match mod_num(item, "fishing_LineCondition")? {
        Some(v) => v as f32,
        None => {
            mod_set(item, "fishing_LineCondition", Some(Value::Num(1.0)))?;
            1.0
        }
    };
    row.progress(f, rgb1(cx.bad_to_good(f)));
    let row = layout.add_item();
    row.label(&trc("Tooltip_fishing_line"), LABEL);
    match mod_value(item, "fishing_LineType")? {
        None => {
            mod_set(item, "fishing_LineType", Some(Value::str("FishingLine")))?;
            let n = find_script_display_name("Base.FishingLine").ok_or_else(npe)?;
            row.value(&tr(&n), WHITE);
        }
        Some(line) => {
            if super::find_script("FishingLine").is_none() {
                row.value(&tr("Tooltip_fishing_line"), WHITE);
            } else {
                let line = match line {
                    Value::Str(value) => value.to_string(),
                    value => return Err(crate::bind_java_util::class_cast(&value, "java.lang.String")),
                };
                match find_script_display_name(&line) {
                    Some(name) => row.value(&tr(&name), WHITE),
                    None => row.value(&tr("Tooltip_fishing_line"), WHITE),
                };
            }
        }
    }
    let row = layout.add_item();
    row.label(&trc("Tooltip_fishing_hook"), LABEL);
    match mod_str(item, "fishing_HookType")? {
        None => {
            mod_set(item, "fishing_HookType", Some(Value::str("FishingHook")))?;
            let n = find_script_display_name("FishingHook").ok_or_else(npe)?;
            row.value(&tr(&n), WHITE);
        }
        Some(hook) => {
            if super::find_script(&hook).is_none() || super::find_script("FishingHook").is_none() {
                row.value(&tr("Tooltip_fishing_hook"), WHITE);
            } else {
                row.value(
                    &tr(&find_script_display_name(&hook).unwrap_or_default()),
                    WHITE,
                );
            }
        }
    }
    let row = layout.add_item();
    row.label(&trc("Tooltip_fishing_bait"), LABEL);
    match mod_str(item, "fishing_Lure")? {
        None => {
            row.value(&tr("UI_None"), [1.0, 0.0, 0.0, 1.0]);
        }
        Some(lure) => {
            if super::find_script(&lure).is_none() {
                row.value(&tr("Tooltip_fishing_bait"), WHITE);
            } else {
                row.value(
                    &tr(&find_script_display_name(&lure).unwrap_or_default()),
                    WHITE,
                );
            }
        }
    }
    Ok(())
}

/// `WeaponPart.DoBatteryTooltip`.
fn battery_rows(cx: &Cx<'_>, it: &InventoryItem, layout: &mut Layout) {
    if it.has_tag("base:usesbattery") && !it.has_tag("base:hideremaining") {
        let f = it.get_current_uses_float();
        layout
            .add_item()
            .label(&format!("{}: ", tr("IGUI_invpanel_Remaining")), LABEL)
            .progress(f, rgb1(cx.bad_to_good(f)));
    }
}

/// `WeaponPart.DoTooltip`.
fn weapon_part(cx: &Cx<'_>, it: &InventoryItem, layout: &mut Layout) {
    let part_type = sv(it, |s| s.part_type.clone()).unwrap_or_else(|| "null".into());
    layout
        .add_item()
        .label(&format!("{}: ", tr("Tooltip_weapon_Type")), LABEL)
        .value(&tr(&format!("Tooltip_weapon_{part_type}")), LABEL);
    // setMountOn: the display names of the mount scripts that exist.
    let module = it.get_module().to_string();
    let names: Vec<String> = sv(it, |s| s.mount_on.clone())
        .unwrap_or_default()
        .iter()
        .filter_map(|m| {
            let full = if m.contains('.') {
                m.clone()
            } else {
                format!("{module}.{m}")
            };
            let s = crate::bind_registry::shared_script_manager()
                .get_script(pz_scripts::ScriptType::Item, &full)?
                .downcast::<pz_scripts::objects::items::Item>()?;
            let s = s.borrow();
            Some(script_display_name(&s))
        })
        .collect();
    let mount = format!(
        "{}: {}",
        tr("Tooltip_weapon_CanBeMountOn"),
        names.join(", ").replace(['[', ']'], "")
    );
    layout.add_item().label(&mount, LABEL);
    battery_rows(cx, it, layout);
}

/// `AlarmClock.DoTooltip` / `AlarmClockClothing.DoTooltip`.
fn alarm_clock(it: &InventoryItem, layout: &mut Layout) {
    let tod = crate::bind_climate::shared_game_time().borrow().time_of_day;
    // GameTime.getHour() / getMinutes().
    let hour = ((tod * 3600.0).floor() as f64 / 3600.0).floor() as i32;
    let minute = ((tod - tod as i32 as f32) * 60.0) as i32;
    let minutes = minute / 10 * 10;
    let mm = |m: i32| if m == 0 { "00".to_string() } else { int_str(m) };
    layout
        .add_item()
        .label(&tr("IGUI_CurrentTime"), LABEL)
        .value(&format!("{hour}:{}", mm(minutes)), LABEL);
    let alarm = match &it.kind {
        ItemKind::AlarmClock(a) | ItemKind::AlarmClockClothing(_, a) => a,
        _ => return,
    };
    if alarm.alarm_set {
        layout
            .add_item()
            .label(&tr("IGUI_AlarmIsSetFor"), LABEL)
            .value(
                &format!("{}:{}", alarm.alarm_hour, mm(alarm.alarm_minutes)),
                LABEL,
            );
    }
}

/// `InventoryContainer.getEffectiveCapacity(chr)`.
fn effective_capacity(cx: &mut Cx<'_>, it: &InventoryItem) -> LuaResult<i32> {
    let ItemKind::Container(c) = &it.kind else {
        return Ok(0);
    };
    // ItemContainer.getEffectiveCapacity: getCapacity() (an item's
    // container caps at 50) with the organized/disorganized traits for a
    // character's bag that is not the character's own inventory.
    let cap = c.container.borrow().capacity.min(50);
    let chr = cx.chr.clone();
    let mut eff = cap;
    if !chr.is_nil() {
        if chr::has_trait(cx.t, &chr, "ORGANIZED")? {
            eff = (cap as f32 * 1.3).max((cap + 1) as f32) as i32;
        } else if chr::has_trait(cx.t, &chr, "DISORGANIZED")? {
            eff = (cap as f32 * 0.7).max(1.0) as i32;
        }
    }
    if eff as f32 > 50.0 - it.actual_weight {
        eff = (50.0 - it.actual_weight) as i32;
    }
    Ok(eff)
}

/// `InventoryContainer.DoTooltip(tooltipUI, layout)`.
fn container(cx: &mut Cx<'_>, item: &ItemRef, layout: &mut Layout) -> LuaResult<()> {
    let cap = effective_capacity(cx, &item.borrow())?;
    let it = item.borrow();
    if cap != 0 {
        layout
            .add_item()
            .label(&trc("Tooltip_container_Capacity"), WHITE)
            .set_value_right_no_plus_int(cap);
    }
    let reduction = match &it.kind {
        ItemKind::Container(c) => c.weight_reduction,
        _ => 0,
    };
    if reduction != 0 {
        layout
            .add_item()
            .label(&trc("Tooltip_container_Weight_Reduction"), WHITE)
            .set_value_right_no_plus_int(reduction);
    }
    let max_size = sv(&it, |s| s.max_item_size);
    if max_size != 0.0 {
        layout
            .add_item()
            .label(&trc("Tooltip_container_Max_Item_Size"), WHITE)
            .set_value_right_no_plus_float(max_size);
    }
    if it.get_blood_clothing_type().is_some() {
        let total = super::components::container_blood_level(&it);
        if total != 0.0 {
            layout
                .add_item()
                .label(&trc("Tooltip_clothing_bloody"), LABEL)
                .progress(total, [0.0, 0.6, 0.0, 0.7]);
        }
    }
    Ok(())
}

/// The rest of `InventoryContainer.DoTooltip(tooltipUI)`: one icon per
/// item name of the contents (last first), in a row below the layout.
pub fn container_icons(tip: &Tip<'_>, item: &ItemRef) -> LuaResult<()> {
    let el = tip.el;
    let mut y = el.get_height() as i32;
    y -= tip.tt.pad_bottom.get();
    if el.get_width() < 160.0 {
        el.set_width(160.0);
    }
    let items: Vec<ItemRef> = {
        let body = item
            .try_borrow()
            .map_err(|_| crate::bind_world_object::borrowed("tooltip container item"))?;
        match &body.kind {
            ItemKind::Container(c) => {
                let contents = c.container.try_borrow().map_err(|_| {
                    crate::bind_world_object::borrowed("tooltip contained item list")
                })?;
                contents.items.clone()
            }
            _ => return Ok(()),
        }
    };
    if !items.is_empty() {
        let mut x = tip.tt.pad_left.get();
        y += 4;
        let mut names = HashSet::new();
        let icon = 16.max(pz_ui::draw::font_height(Some(ObjectTooltip::font())));
        for i in items.iter().rev() {
            let name = i
                .try_borrow()
                .map_err(|_| crate::bind_world_object::borrowed("tooltip contained item name"))?
                .name
                .clone();
            if !names.insert(name) {
                continue;
            }
            let tex = item_texture(i)?;
            tip.tt.draw_texture_scaled_aspect(
                el,
                tex.as_ref(),
                x as f64,
                y as f64,
                icon as f64,
                icon as f64,
                1.0,
                1.0,
                1.0,
                1.0,
            );
            x += icon + 1;
            if (x + icon) as f32 > el.get_width() as f32 - tip.tt.pad_right.get() as f32 {
                break;
            }
        }
        y += icon;
    }
    y += tip.tt.pad_bottom.get();
    el.set_height(y as f64);
    Ok(())
}

/// `Literature.DoTooltip`.
fn literature(cx: &mut Cx<'_>, item: &ItemRef, layout: &mut Layout) -> LuaResult<()> {
    let chr = cx.chr.clone();
    if chr.is_nil() {
        return Ok(());
    }
    let (pages, full, lvl, levels, picture) = {
        let it = item.borrow();
        let ItemKind::Literature(l) = &it.kind else {
            return Ok(());
        };
        (
            l.number_of_pages,
            it.get_full_type(),
            sv(&it, |s| s.lvl_skill_trained),
            sv(&it, |s| s.num_levels_trained),
            it.has_tag("base:picture"),
        )
    };
    let title = mod_str(item, "literatureTitle")?;
    let title_read = match &title {
        Some(t) => chr::is_literature_read(cx.t, &chr, t)?,
        None => false,
    };
    if title.is_none() || !title_read {
        let it = item.borrow();
        let boredom = it.get_boredom_change();
        if boredom != 0.0 {
            let row = layout.add_item();
            let v = boredom * -0.02;
            row.label(&trc("Tooltip_food_Boredom"), LABEL);
            if v > 0.0 {
                row.progress(v, good(cx));
            } else {
                row.progress(-v, bad(cx));
            }
        }
        let stress = it.get_stress_change();
        if stress != 0.0 {
            let row = layout.add_item();
            let v = (stress * 100.0) as i32 as f32 * -0.02;
            row.label(&trc("Tooltip_literature_Stress_Reduction"), LABEL);
            if v > 0.0 {
                row.progress(v, good(cx));
            } else {
                row.progress(-v, bad(cx));
            }
        }
        let unhappy = it.get_unhappy_change();
        if unhappy != 0.0 {
            let row = layout.add_item();
            let v = unhappy * -0.02;
            row.label(&trc("Tooltip_food_Unhappiness"), LABEL);
            if v > 0.0 {
                row.progress(v, good(cx));
            } else {
                row.progress(-v, bad(cx));
            }
        }
    }
    if pages != -1 {
        let read = chr::already_read_pages(cx.t, &chr, &full)?;
        layout
            .add_item()
            .label(&trc("Tooltip_literature_Number_of_Pages"), LABEL)
            .value(&format!("{read} / {pages}"), WHITE);
    }
    let illiterate = chr::has_trait(cx.t, &chr, "ILLITERATE")?;
    // getLearnedRecipes() runs before the `learnedRecipe` test and moves
    // that mod data value into the item's list.
    let learned = learned_recipes(item)?;
    let learned_recipe = mod_str(item, "learnedRecipe")?;
    if lvl != -1 {
        let max = lvl + levels - 1;
        let mut l = lvl.to_string();
        if lvl != max {
            l = format!("{l}-{max}");
        }
        layout
            .add_item()
            .label(&tr_args("Tooltip_Literature_XpMultiplier", &[&l]), LABEL);
    }
    match (&learned, illiterate) {
        (Some(recipes), false) => {
            for r in recipes {
                let n = super::recipe_name(r);
                layout
                    .add_item()
                    .label(&tr_args("Tooltip_Literature_LearnedRecipes", &[&n]), LABEL);
            }
            let known = chr::knows_recipes(cx.t, &chr, recipes)?;
            let l = if known {
                tr("Tooltip_literature_HasBeenRead")
            } else {
                tr("Tooltip_literature_NotBeenRead")
            };
            layout.add_item().label(&l, LABEL);
            if known {
                let (p, t) = chr::granted_recipe_counts(cx.t, &chr, recipes)?;
                if p > 0 || t > 0 {
                    layout
                        .add_item()
                        .label(&tr("Tooltip_literature_AlreadyKnown"), [0.0, 1.0, 0.8, 1.0]);
                }
            }
        }
        (Some(_), true) => {
            let l = tr("ContextMenu_Illiterate");
            layout
                .add_item()
                .label(&tr_args("Tooltip_Literature_LearnedRecipes", &[&l]), LABEL);
        }
        (None, _) => {}
    }
    match (&learned_recipe, illiterate) {
        (Some(r), false) => {
            let n = super::recipe_name(r);
            layout
                .add_item()
                .label(&tr_args("Tooltip_Literature_LearnedRecipes", &[&n]), LABEL);
            let one = [r.clone()];
            let known = chr::knows_recipes(cx.t, &chr, &one)?;
            let l = if known {
                tr("Tooltip_literature_HasBeenRead")
            } else {
                tr("Tooltip_literature_NotBeenRead")
            };
            layout.add_item().label(&l, LABEL);
            if known {
                let (p, t) = chr::granted_recipe_counts(cx.t, &chr, &one)?;
                if p > 0 || t > 0 {
                    layout
                        .add_item()
                        .label(&tr("Tooltip_literature_AlreadyKnown"), [0.0, 1.0, 0.8, 1.0]);
                }
            }
        }
        (Some(_), true) => {
            let l = tr("ContextMenu_Illiterate");
            layout
                .add_item()
                .label(&tr_args("Tooltip_Literature_LearnedRecipes", &[&l]), LABEL);
        }
        (None, _) => {}
    }
    if title.is_some() && title_read {
        let l = if picture {
            tr("ContextMenu_RecentlySeen")
        } else {
            tr("ContextMenu_RecentlyRead")
        };
        layout.add_item().label(&l, LABEL);
    }
    Ok(())
}

/// `Literature.getLearnedRecipes()`: a `learnedRecipe` mod data value
/// becomes the item's own list (and is cleared); else the item's list or
/// the script's.
fn learned_recipes(item: &ItemRef) -> LuaResult<Option<Vec<String>>> {
    if let Some(r) = mod_str(item, "learnedRecipe")? {
        {
            let mut it = item.try_borrow_mut().map_err(|_| pz_inventory::moddata::ModDataError::Borrowed("item tooltip body").to_lua_error())?;
            if let ItemKind::Literature(l) = &mut it.kind {
            l.learned_recipes = Some(vec![r]);
            }
        }
        mod_set(item, "learnedRecipe", None)?;
    }
    let it = item.try_borrow().map_err(|_| pz_inventory::moddata::ModDataError::Borrowed("item tooltip body").to_lua_error())?;
    if let ItemKind::Literature(l) = &it.kind {
        if let Some(r) = &l.learned_recipes {
            return Ok(Some(r.clone()));
        }
    }
    Ok(sv(&it, |s| s.learned_recipes.clone()))
}

/// `AnimalInventoryItem.DoTooltip`. TODO(animals): `IsoAnimal` is not
/// bound to items here; the rows come from the item's animal through its
/// Lua binding (`getAnimal()`), and are skipped without one.
fn animal(cx: &mut Cx<'_>, item: &ItemRef, layout: &mut Layout) -> LuaResult<()> {
    let v = crate::bind_items::item_value(item);
    let animal = chr::call_or(cx.t, &v, "getAnimal", &[], Value::Nil)?;
    if animal.is_nil() {
        // this.animal.getAnimalType() on a null animal, after the label.
        layout.add_item().label(&trc("IGUI_AnimalType"), LABEL);
        return Err(npe());
    }
    let s = |v: Value| pz_kahlua::exec::java_value_of(&v);
    let ty = s(chr::call_or(
        cx.t,
        &animal,
        "getAnimalType",
        &[],
        Value::Nil,
    )?);
    layout
        .add_item()
        .label(&trc("IGUI_AnimalType"), LABEL)
        .value(&tr(&format!("IGUI_AnimalType_{ty}")), WHITE);
    let female = chr::call_or(cx.t, &animal, "isFemale", &[], Value::Bool(false))?.truthy();
    let g = if female {
        tr("IGUI_Animal_Female")
    } else {
        tr("IGUI_Animal_Male")
    };
    layout
        .add_item()
        .label(&trc("UI_characreation_gender"), LABEL)
        .value(&g, WHITE);
    let player = chr::player_instance();
    if player.is_nil() {
        return Err(npe());
    }
    let husbandry = chr::perk_level(cx.t, &player, "Husbandry")?;
    let cheat = Value::Bool(cx.animal_cheat);
    let age = s(chr::call_or(
        cx.t,
        &animal,
        "getAgeText",
        &[cheat.clone(), Value::from(husbandry)],
        Value::str(""),
    )?);
    layout
        .add_item()
        .label(&trc("IGUI_char_Age"), LABEL)
        .value(&age, WHITE);
    let app = s(chr::call_or(
        cx.t,
        &animal,
        "getAppearanceText",
        std::slice::from_ref(&cheat),
        Value::str(""),
    )?);
    layout
        .add_item()
        .label(&trc("IGUI_Animal_Appearance"), LABEL)
        .value(&app, WHITE);
    let health = s(chr::call_or(
        cx.t,
        &animal,
        "getHealthText",
        &[cheat, Value::from(husbandry)],
        Value::str(""),
    )?);
    layout
        .add_item()
        .label(&trc("IGUI_XP_Health"), LABEL)
        .value(&health, WHITE);
    if cx.animal_cheat {
        let stress = pz_kahlua::exec::raw_tonumber(&chr::call_or(
            cx.t,
            &animal,
            "getStress",
            &[],
            Value::from(0),
        )?)
        .unwrap_or(0.0);
        layout
            .add_item()
            .label("[DEBUG] Stress:", LABEL)
            .value(&format!("{}", (stress as f32).round() as i32), WHITE);
    }
    let _ = (fstr, script);
    Ok(())
}
