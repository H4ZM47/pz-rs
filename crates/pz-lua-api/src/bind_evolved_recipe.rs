//! Inventory-backed methods of `zombie.scripting.objects.EvolvedRecipe`
//! (`getItemRecipe`, `needToBeCooked`, `isSpiceAdded`, `hasMinimumWater`,
//! `getItemsCanBeUse`, `isItemUsableInRecipe`, `addItem`), plus
//! `zombie.scripting.objects.Recipe$RequiredSkill:getPerk()` and
//! `InventoryItem`/`Food` `getExtraItems`/`addExtraItem`/`getSpices`/`isSpice`.

use std::any::Any;
use std::cell::RefCell;
use std::rc::Rc;

use pz_characters::Characters;
use pz_inventory::item::ItemKind;
use pz_inventory::shared::ContainerRef;
use pz_inventory::InventoryItem;
use pz_kahlua::{LuaError, LuaResult, NativeCall, Thread, Value};
use pz_scripts::objects::recipes::{EvolvedRecipe, RequiredSkill, UniqueRecipe};
use pz_scripts::{ScriptManager, ScriptType};

use crate::bind_character_sim::{CharacterObject, LuaGameCharacterRef};
use crate::bind_item_container;
use crate::bind_items::{self, ItemRef, INVENTORY_ITEM_CLASS};
use crate::bind_java_util::{list_value, JavaList};
use crate::bind_scripts::recipes::{
    item_recipe_value, EVOLVED_RECIPE_CLASS, REQUIRED_SKILL_CLASS,
};
use crate::bind_scripts::{method, npe, ScriptHandle};
use crate::bind_skills::perk_value;
use crate::expose::{expose_class, int_arg};

const FOOD_CLASS: &str = "zombie.inventory.types.Food";

fn is_item_spice(item: &InventoryItem) -> bool {
    item.template
        .script
        .as_ref()
        .is_some_and(|s| s.0.borrow().spice)
}

fn cooking_level_of(chars: &RefCell<Characters>, chr: &LuaGameCharacterRef) -> i32 {
    let perk = chars.borrow().perks.constant("Cooking");
    chr.borrow().perk_level(perk.as_ref())
}

fn character_arg(v: &Value) -> LuaResult<LuaGameCharacterRef> {
    match v {
        Value::Object(o) => (&**o as &dyn Any)
            .downcast_ref::<CharacterObject>()
            .map(|c| c.0.clone())
            .ok_or_else(|| LuaError::runtime("expected IsoGameCharacter")),
        Value::Nil => Err(npe()),
        _ => Err(LuaError::runtime("expected IsoGameCharacter")),
    }
}

/// Collects `chr`'s main inventory followed by any unique containers in an
/// optional `ArrayList<ItemContainer>` argument.
pub(crate) fn collect_containers(
    chr: &LuaGameCharacterRef,
    containers_val: &Value,
) -> Vec<ContainerRef> {
    let mut out = Vec::new();
    let main = chr.borrow().items.inventory.clone();
    out.push(main);
    if let Value::Object(o) = containers_val {
        if let Some(list) = o.as_any().downcast_ref::<JavaList>() {
            for elem in list.snapshot() {
                if let Some(cont) = bind_item_container::container_of_value(&elem) {
                    if !out.iter().any(|existing| Rc::ptr_eq(existing, &cont)) {
                        out.push(cont);
                    }
                }
            }
        }
    }
    out
}

/// `EvolvedRecipe.needToBeCooked(item)`: raw food that is dangerous uncooked
/// cannot be added to a non-cookable recipe.
pub fn need_to_be_cooked(recipe: &EvolvedRecipe, item: &InventoryItem) -> bool {
    if recipe.is_cookable() {
        return false;
    }
    match &item.kind {
        ItemKind::Food(f) => f.dangerous_uncooked && !f.cooked,
        _ => false,
    }
}

/// `EvolvedRecipe.isSpiceAdded(baseItem, spiceItem)`.
pub fn is_spice_added(base: &InventoryItem, spice: &InventoryItem) -> bool {
    let ItemKind::Food(f) = &base.kind else {
        return false;
    };
    let Some(spices) = f.spices.as_ref() else {
        return false;
    };
    let full = spice.get_full_type();
    let short = spice.get_type();
    spices.iter().any(|s| s == &full || s == short)
}

/// `EvolvedRecipe.hasMinimumWater(item)`.
pub fn has_minimum_water(recipe: &EvolvedRecipe, item: &ItemRef) -> bool {
    let min_w = recipe.minimum_water();
    if min_w <= 0.0 {
        return true;
    }
    if let Ok(Some(fc)) = crate::bind_world_object::fluid_component(&bind_items::item_value(item)) {
        if let Ok(amt) = crate::bind_fluid_container::amount(&fc) {
            return amt >= min_w;
        }
    }
    item.borrow().get_current_uses_float() >= min_w
}

/// `EvolvedRecipe.getItemsCanBeUse(chr, baseItem, containers)`:
/// returns all usable ingredients/spices across `chr`'s inventory and `containers`.
pub fn items_can_be_use(
    recipe: &EvolvedRecipe,
    chars: &RefCell<Characters>,
    chr: &LuaGameCharacterRef,
    base_item: &ItemRef,
    containers_val: &Value,
) -> Vec<ItemRef> {
    let cooking_lvl = cooking_level_of(chars, chr);
    let containers = collect_containers(chr, containers_val);
    let (extra_count, base_is_result) = {
        let b = base_item.borrow();
        let extra = b.extra_items.as_ref().map_or(0, Vec::len) as i32;
        let is_res = recipe
            .is_result_item_type(Some(b.get_type()))
            .unwrap_or(false);
        (extra, is_res)
    };
    let at_max = base_is_result && recipe.max_items() > 0 && extra_count >= recipe.max_items();

    let mut out = Vec::new();
    for cont in containers {
        let items = cont.borrow().items.clone();
        for candidate in items {
            if Rc::ptr_eq(&candidate, base_item) {
                continue;
            }
            if out.iter().any(|existing: &ItemRef| Rc::ptr_eq(existing, &candidate)) {
                continue;
            }
            let usable = {
                let it = candidate.borrow();
                let Some(ir) = recipe.item_recipe(it.get_type()) else {
                    continue;
                };
                let _ = ir;
                let spice = is_item_spice(&it);
                if spice {
                    if !recipe.can_add_spices_empty && extra_count == 0 {
                        false
                    } else if is_spice_added(&base_item.borrow(), &it) {
                        false
                    } else {
                        match &it.kind {
                            ItemKind::Food(f) => {
                                !f.burnt
                                    && (!f.frozen || recipe.is_allow_frozen_item())
                                    && (!it.is_rotten() || cooking_lvl >= 7)
                                    && f.hung_change.abs() > 0.001
                            }
                            _ => it.get_current_uses() > 0 || it.get_current_uses_float() > 0.0,
                        }
                    }
                } else if at_max {
                    false
                } else {
                    match &it.kind {
                        ItemKind::Food(f) => {
                            !f.burnt
                                && (!f.frozen || recipe.is_allow_frozen_item())
                                && (!it.is_rotten() || cooking_lvl >= 7)
                                && !need_to_be_cooked(recipe, &it)
                                && f.hung_change.abs() > 0.001
                        }
                        _ => it.get_current_uses() > 0 || it.get_current_uses_float() > 0.0,
                    }
                }
            };
            if usable {
                out.push(candidate);
            }
        }
    }
    out
}

/// Checks `ScriptManager`'s `UniqueRecipe` definitions and applies any matching
/// recipe name and hunger/happiness/boredom bonuses to `result`.
fn apply_unique_recipe(sm: &ScriptManager, recipe: &EvolvedRecipe, result: &ItemRef) {
    let extra_types: Vec<String> = {
        let r = result.borrow();
        let Some(ids) = r.extra_items.as_ref() else {
            return;
        };
        bind_items::with_factory_env(|env, _| {
            ids.iter()
                .filter_map(|&id| env.full_type(id))
                .collect::<Vec<_>>()
        })
    };
    if extra_types.is_empty() {
        return;
    }
    for script in sm.collection(ScriptType::UniqueRecipe).get_all_scripts() {
        let Some(ur_rc) = script.downcast::<UniqueRecipe>() else {
            continue;
        };
        let ur = ur_rc.borrow();
        if ur.base_recipe() != Some(recipe.untranslated_name()) {
            continue;
        }
        let req = ur.items();
        if req.is_empty() || req.len() != extra_types.len() {
            continue;
        }
        let mut remaining = extra_types.clone();
        let mut matched = true;
        for need in req {
            if let Some(pos) = remaining.iter().position(|t| {
                t == need || t.rsplit('.').next() == Some(need.as_str())
            }) {
                remaining.swap_remove(pos);
            } else {
                matched = false;
                break;
            }
        }
        if matched {
            let mut r = result.borrow_mut();
            if let Some(name) = ur.name() {
                r.name = name.to_string();
                r.custom_name = true;
            }
            if let ItemKind::Food(f) = &mut r.kind {
                f.hung_change -= ur.hunger_bonus() as f32 / 100.0;
                f.base_hunger -= ur.hunger_bonus() as f32 / 100.0;
                f.unhappy_change -= ur.hapiness_bonus() as f32;
                f.boredom_change -= ur.boredom_bonus() as f32;
            }
            break;
        }
    }
}

/// `EvolvedRecipe.addItem(baseItem, usedItem, chr)`:
/// converts `baseItem` to `resultItem` on first addition (moving it in the
/// container), transfers ingredient or spice nutrition/poison/cookable state
/// scaled by `ItemRecipe.use` and `Cooking` level, consumes `usedItem`, and
/// returns the updated dish `ItemRef`.
pub fn add_item(
    thread: &mut Thread,
    sm: &ScriptManager,
    chars: &RefCell<Characters>,
    recipe: &EvolvedRecipe,
    base_item: &ItemRef,
    used_item: &ItemRef,
    chr: &LuaGameCharacterRef,
) -> LuaResult<ItemRef> {
    let cooking_lvl = cooking_level_of(chars, chr);
    let chef_name = chr.borrow().username().unwrap_or("").to_string();
    let base_is_result = recipe
        .is_result_item_type(Some(base_item.borrow().get_type()))
        .unwrap_or(false);

    // Promote `baseItem` -> `resultItem` if this is the first ingredient.
    let result: ItemRef = if base_is_result {
        base_item.clone()
    } else if let Some(full_result) = recipe.full_result_item() {
        if let Some(created) = bind_items::create_item(full_result, 1.0) {
            bind_items::initialise_item(thread, &created);
            let target_container = bind_item_container::item_container_value(base_item)
                .ok()
                .and_then(|v| bind_item_container::container_of_value(&v))
                .unwrap_or_else(|| chr.borrow().items.inventory.clone());
            // Carry over base food properties if base itself was food (e.g. bread slice).
            {
                let base_b = base_item.borrow();
                let mut res_b = created.borrow_mut();
                if let (ItemKind::Food(bf), ItemKind::Food(rf)) = (&base_b.kind, &mut res_b.kind) {
                    rf.hung_change = bf.hung_change;
                    rf.base_hunger = bf.base_hunger;
                    rf.calories = bf.calories;
                    rf.proteins = bf.proteins;
                    rf.lipids = bf.lipids;
                    rf.carbohydrates = bf.carbohydrates;
                    rf.thirst_change = bf.thirst_change;
                    rf.unhappy_change = bf.unhappy_change;
                    rf.boredom_change = bf.boredom_change;
                    rf.heat = bf.heat;
                    rf.age = bf.age;
                }
                if let ItemKind::Food(rf) = &mut res_b.kind {
                    rf.is_cookable = recipe.is_cookable();
                    rf.chef = Some(chef_name.clone());
                }
            }
            let _ = bind_item_container::remove(thread, &target_container, base_item);
            let _ = bind_item_container::add_item(thread, &target_container, Some(created.clone()));
            created
        } else {
            base_item.clone()
        }
    } else {
        base_item.clone()
    };

    let (used_type, used_full_type, used_reg_id, is_spice, use_amount) = {
        let u = used_item.borrow();
        let ty = u.get_type().to_string();
        let full = u.get_full_type();
        let reg_id = bind_items::with_factory_env(|env, _| {
            let id = env.registry_id(&full);
            if id >= 0 { id } else { u.registry_id }
        });
        let spice = is_item_spice(&u);
        let amt = recipe.item_recipe(&ty).map_or(10, |ir| ir.use_());
        (ty, full, reg_id, spice, amt)
    };
    let _ = used_type;

    let cooking_bonus = 1.0 + (cooking_lvl.max(0) as f32) * 0.03;
    let mut remove_used = false;
    let mut use_drainable_count: Option<i32> = None;

    if is_spice {
        let mut add_hung = 0.0f32;
        let mut add_thirst = 0.0f32;
        let mut add_unhappy = 0.0f32;
        let mut add_boredom = 0.0f32;
        let mut add_cal = 0.0f32;
        let mut add_pro = 0.0f32;
        let mut add_lip = 0.0f32;
        let mut add_carb = 0.0f32;
        let mut add_poison_power = 0i32;
        let mut add_poison_det = 0i32;
        {
            let mut u = used_item.borrow_mut();
            match &mut u.kind {
                ItemKind::Food(uf) => {
                    let want = (use_amount as f32 / 100.0).max(0.01);
                    let avail = uf.hung_change.abs();
                    let ratio = if avail > 0.0001 {
                        (want / avail).clamp(0.0, 1.0)
                    } else {
                        1.0
                    };
                    add_hung = uf.hung_change * ratio * cooking_bonus;
                    add_thirst = uf.thirst_change * ratio;
                    add_unhappy = uf.unhappy_change * ratio - (cooking_lvl.max(0) as f32) * 0.5;
                    add_boredom = uf.boredom_change * ratio - (cooking_lvl.max(0) as f32) * 0.5;
                    add_cal = uf.calories * ratio;
                    add_pro = uf.proteins * ratio;
                    add_lip = uf.lipids * ratio;
                    add_carb = uf.carbohydrates * ratio;
                    if uf.poison_power > 0 {
                        add_poison_power = ((uf.poison_power as f32) * ratio).ceil() as i32;
                        add_poison_det = uf.poison_detection_level;
                    }
                    let rem = 1.0 - ratio;
                    if rem <= 0.01 {
                        remove_used = true;
                    } else {
                        uf.hung_change *= rem;
                        uf.base_hunger *= rem;
                        uf.calories *= rem;
                        uf.proteins *= rem;
                        uf.lipids *= rem;
                        uf.carbohydrates *= rem;
                    }
                }
                _ => {
                    use_drainable_count = Some(use_amount.max(1));
                }
            }
        }
        {
            let mut r = result.borrow_mut();
            if let ItemKind::Food(rf) = &mut r.kind {
                rf.hung_change += add_hung;
                rf.base_hunger += add_hung;
                rf.thirst_change += add_thirst;
                rf.unhappy_change += add_unhappy;
                rf.boredom_change += add_boredom;
                rf.calories += add_cal;
                rf.proteins += add_pro;
                rf.lipids += add_lip;
                rf.carbohydrates += add_carb;
                if add_poison_power > 0 {
                    rf.poison_power += add_poison_power;
                    rf.poison_detection_level = rf.poison_detection_level.max(add_poison_det);
                }
                rf.spices.get_or_insert_with(Vec::new).push(used_full_type);
                rf.chef = Some(chef_name);
            }
        }
    } else {
        // Regular ingredient.
        let mut add_hung = 0.0f32;
        let mut add_thirst = 0.0f32;
        let mut add_unhappy = 0.0f32;
        let mut add_boredom = 0.0f32;
        let mut add_cal = 0.0f32;
        let mut add_pro = 0.0f32;
        let mut add_lip = 0.0f32;
        let mut add_carb = 0.0f32;
        let mut add_pain = 0.0f32;
        let mut add_flu = 0i32;
        let mut add_sick = 0i32;
        let mut add_poison_power = 0i32;
        let mut add_poison_det = 0i32;
        let mut ingredient_uncooked = false;
        {
            let mut u = used_item.borrow_mut();
            match &mut u.kind {
                ItemKind::Food(uf) => {
                    let want = (use_amount as f32 / 100.0).max(0.01);
                    let avail = uf.hung_change.abs();
                    let ratio = if avail > 0.0001 {
                        (want / avail).clamp(0.0, 1.0)
                    } else {
                        1.0
                    };
                    add_hung = uf.hung_change * ratio * cooking_bonus;
                    add_thirst = uf.thirst_change * ratio;
                    add_unhappy = uf.unhappy_change * ratio - (cooking_lvl.max(0) as f32);
                    add_boredom = uf.boredom_change * ratio - (cooking_lvl.max(0) as f32);
                    add_cal = uf.calories * ratio;
                    add_pro = uf.proteins * ratio;
                    add_lip = uf.lipids * ratio;
                    add_carb = uf.carbohydrates * ratio;
                    add_pain = uf.pain_reduction * ratio;
                    add_flu = ((uf.flu_reduction as f32) * ratio).round() as i32;
                    add_sick = ((uf.food_sickness_change as f32) * ratio).round() as i32;
                    if uf.poison_power > 0 {
                        add_poison_power = ((uf.poison_power as f32) * ratio).ceil() as i32;
                        add_poison_det = uf.poison_detection_level;
                    }
                    if uf.dangerous_uncooked && !uf.cooked {
                        ingredient_uncooked = true;
                    }
                    let rem = 1.0 - ratio;
                    if rem <= 0.01 {
                        remove_used = true;
                    } else {
                        uf.hung_change *= rem;
                        uf.base_hunger *= rem;
                        uf.calories *= rem;
                        uf.proteins *= rem;
                        uf.lipids *= rem;
                        uf.carbohydrates *= rem;
                    }
                }
                _ => {
                    use_drainable_count = Some(use_amount.max(1));
                }
            }
        }
        {
            let mut r = result.borrow_mut();
            r.extra_items.get_or_insert_with(Vec::new).push(used_reg_id);
            if let ItemKind::Food(rf) = &mut r.kind {
                rf.hung_change += add_hung;
                rf.base_hunger += add_hung;
                rf.thirst_change += add_thirst;
                rf.unhappy_change += add_unhappy;
                rf.boredom_change += add_boredom;
                rf.calories += add_cal;
                rf.proteins += add_pro;
                rf.lipids += add_lip;
                rf.carbohydrates += add_carb;
                rf.pain_reduction += add_pain;
                rf.flu_reduction += add_flu;
                rf.food_sickness_change += add_sick;
                if add_poison_power > 0 {
                    rf.poison_power += add_poison_power;
                    rf.poison_detection_level = rf.poison_detection_level.max(add_poison_det);
                }
                if recipe.is_cookable() {
                    rf.is_cookable = true;
                    if ingredient_uncooked {
                        rf.dangerous_uncooked = true;
                        rf.cooked = false;
                    }
                }
                rf.chef = Some(chef_name);
            }
        }
        apply_unique_recipe(sm, recipe, &result);
    }

    if remove_used {
        let _ = bind_item_container::item_user_remove_item(thread, used_item);
    } else if let Some(uses) = use_drainable_count {
        let _ = bind_item_container::item_user_use_item(thread, used_item, uses, false, true);
    }

    Ok(result)
}

/// Registers inventory-backed methods on `EvolvedRecipe`, `RequiredSkill:getPerk`,
/// and `InventoryItem`/`Food` `extraItems`/`spices` getters/mutators.
pub fn register(thread: &mut Thread, sm: Rc<ScriptManager>, chars: Rc<RefCell<Characters>>) {
    // `Recipe.RequiredSkill:getPerk()`
    let req_skill = expose_class(thread, REQUIRED_SKILL_CLASS, None);
    let chars_perk = chars.clone();
    method(&req_skill, "getPerk", move |_, h| {
        let perk_name = h.read(|s: &RequiredSkill| s.perk().to_string())?;
        let perk = chars_perk.borrow().perks.constant(&perk_name);
        Ok(perk_value(perk.as_ref()))
    });

    // `InventoryItem:getExtraItems()`, `addExtraItem(fullType)`
    let item_cls = expose_class(thread, INVENTORY_ITEM_CLASS, None);
    item_cls.method("getExtraItems", |c| {
        let item = bind_items::item_arg(c, 0)?;
        let extra = item.borrow().extra_items.clone();
        let Some(ids) = extra else {
            return Ok(c.push(Value::Nil));
        };
        let vals: Vec<Value> = bind_items::with_factory_env(|env, _| {
            ids.iter()
                .map(|&id| {
                    env.full_type(id)
                        .map(Value::from)
                        .unwrap_or_else(|| Value::from(format!("Unknown:{id}")))
                })
                .collect()
        });
        Ok(c.push(list_value(vals)))
    });
    item_cls.method("addExtraItem", |c| {
        let item = bind_items::item_arg(c, 0)?;
        let Some(full) = crate::expose::str_arg(c, 1) else {
            return Ok(0);
        };
        let reg_id = bind_items::with_factory_env(|env, _| env.registry_id(&full));
        item.borrow_mut()
            .extra_items
            .get_or_insert_with(Vec::new)
            .push(reg_id);
        Ok(0)
    });

    // `Food:getSpices()`, `setSpices(list)`, `isSpice()`
    let food_cls = expose_class(thread, FOOD_CLASS, Some(INVENTORY_ITEM_CLASS));
    food_cls.method("getSpices", |c| {
        let item = bind_items::item_arg(c, 0)?;
        let spices = match &item.borrow().kind {
            ItemKind::Food(f) => f.spices.clone(),
            _ => None,
        };
        let Some(spices) = spices else {
            return Ok(c.push(Value::Nil));
        };
        Ok(c.push(list_value(spices.into_iter().map(Value::from).collect())))
    });
    food_cls.method("setSpices", |c| {
        let item = bind_items::item_arg(c, 0)?;
        let next = match c.arg(1) {
            Value::Nil => None,
            Value::Object(o) => o.as_any().downcast_ref::<JavaList>().map(|l| {
                l.snapshot()
                    .into_iter()
                    .filter_map(|v| v.as_str().map(str::to_string))
                    .collect::<Vec<_>>()
            }),
            _ => None,
        };
        if let ItemKind::Food(f) = &mut item.borrow_mut().kind {
            f.spices = next;
        }
        Ok(0)
    });
    food_cls.method("isSpice", |c| {
        let item = bind_items::item_arg(c, 0)?;
        let spice = is_item_spice(&item.borrow());
        Ok(c.push(spice))
    });

    // `EvolvedRecipe` inventory methods.
    let b = expose_class(thread, EVOLVED_RECIPE_CLASS, None);

    method(&b, "getItemRecipe", |c, h| {
        let Some(item) = bind_items::item_arg_value(&c.arg(1)) else {
            return Ok(Value::Nil);
        };
        let ty = item.borrow().get_type().to_string();
        let exists = h.read(|e: &EvolvedRecipe| e.item_recipe(&ty).is_some())?;
        Ok(if exists {
            item_recipe_value(h, &ty)
        } else {
            Value::Nil
        })
    });

    method(&b, "needToBeCooked", |c, h| {
        let Some(item) = bind_items::item_arg_value(&c.arg(1)) else {
            return Ok(Value::Bool(false));
        };
        let item = item.borrow();
        let need = h.read(|e: &EvolvedRecipe| need_to_be_cooked(e, &item))?;
        Ok(Value::Bool(need))
    });

    method(&b, "isSpiceAdded", |c, _h| {
        let (Some(base), Some(spice)) = (
            bind_items::item_arg_value(&c.arg(1)),
            bind_items::item_arg_value(&c.arg(2)),
        ) else {
            return Ok(Value::Bool(false));
        };
        let added = is_spice_added(&base.borrow(), &spice.borrow());
        Ok(Value::Bool(added))
    });

    method(&b, "hasMinimumWater", |c, h| {
        let Some(item) = bind_items::item_arg_value(&c.arg(1)) else {
            return Ok(Value::Bool(false));
        };
        let ok = h.read(|e: &EvolvedRecipe| has_minimum_water(e, &item))?;
        Ok(Value::Bool(ok))
    });

    let chars_use = chars.clone();
    method(&b, "getItemsCanBeUse", move |c, h| {
        let chr = character_arg(&c.arg(1))?;
        let Some(base) = bind_items::item_arg_value(&c.arg(2)) else {
            return Ok(list_value(Vec::new()));
        };
        let containers = c.arg(3);
        let items = h.read(|e: &EvolvedRecipe| {
            items_can_be_use(e, &chars_use, &chr, &base, &containers)
        })?;
        let vals: Vec<Value> = items.iter().map(bind_items::item_value).collect();
        Ok(list_value(vals))
    });

    let chars_usable = chars.clone();
    method(&b, "isItemUsableInRecipe", move |c, h| {
        let chr = character_arg(&c.arg(1))?;
        let Some(base) = bind_items::item_arg_value(&c.arg(2)) else {
            return Ok(Value::Bool(false));
        };
        let id = int_arg(c, 3);
        let found = h.read(|e: &EvolvedRecipe| {
            items_can_be_use(e, &chars_usable, &chr, &base, &Value::Nil)
                .into_iter()
                .any(|it| it.borrow().id == id)
        })?;
        Ok(Value::Bool(found))
    });

    let sm_add = sm.clone();
    let chars_add = chars;
    b.method("addItem", move |c: &mut NativeCall<'_>| {
        let h = match c.arg(0) {
            Value::Object(o) => o
                .as_any()
                .downcast_ref::<ScriptHandle>()
                .cloned()
                .ok_or_else(|| LuaError::runtime("expected EvolvedRecipe"))?,
            _ => return Err(npe()),
        };
        let base = bind_items::item_arg(c, 1)?;
        let used = bind_items::item_arg(c, 2)?;
        let chr = character_arg(&c.arg(3))?;
        let result = h.read(|e: &EvolvedRecipe| {
            add_item(c.thread, &sm_add, &chars_add, e, &base, &used, &chr)
        })??;
        Ok(c.push(bind_items::item_value(&result)))
    });
}
