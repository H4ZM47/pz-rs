//! `zombie.scripting.logic.RecipeCodeOnTest`: the Java `OnTest` functions
//! that recipe scripts name (`OnTest = RecipeCodeOnTest.<name>`). Only
//! `genericPacking` is ported; the other names stay unbound.

use crate::bind_items::ItemRef;
use crate::expose::expose_class;
use pz_inventory::item::ItemKind;
use pz_inventory::InventoryItem;
use pz_kahlua::{LuaError, LuaResult, Thread, Value};
use pz_scripts::objects::items::Item;
use pz_scripts::ScriptManager;
use std::rc::Rc;

pub const RECIPE_CODE_ON_TEST_CLASS: &str = "zombie.scripting.logic.RecipeCodeOnTest";

pub(super) fn register(thread: &mut Thread, sm: Rc<ScriptManager>) {
    let b = expose_class(thread, RECIPE_CODE_ON_TEST_CLASS, None);
    b.static_fn("genericPacking", move |c| {
        let item = crate::bind_world_object::item_argument(&c.arg(0))?;
        let v = generic_packing(c.thread, &sm, &item)?;
        Ok(c.push(v))
    });
    // `haveFilter` / `haveOxygenTank`: `((Clothing) item).hasFilter()` /
    // `hasTank()`. `noFilter` / `noOxygenTank`: true unless a `Clothing`
    // has one.
    for (name, method, have) in [
        ("haveFilter", "hasFilter", true),
        ("noFilter", "hasFilter", false),
        ("haveOxygenTank", "hasTank", true),
        ("noOxygenTank", "hasTank", false),
    ] {
        b.static_fn(name, move |c| {
            let item = crate::bind_world_object::item_argument(&c.arg(0))?;
            let v = clothing_has(c.thread, &item, method, have)?;
            Ok(c.push(v))
        });
    }
    // `cutFish` / `cutFillet`: food heavier than 0.6 / 1.0; any other item.
    for (name, min) in [("cutFish", 0.6f32), ("cutFillet", 1.0)] {
        b.static_fn(name, move |c| {
            let item = crate::bind_world_object::item_argument(&c.arg(0))?;
            if !matches!(item.borrow().kind, ItemKind::Food(_)) {
                return Ok(c.push(true));
            }
            let weight = number(&call(c.thread, &item, "getActualWeight", &[])?) as f32;
            Ok(c.push(weight > min))
        });
    }
    // `breakGlass`: no fluid container, or an empty one.
    b.static_fn("breakGlass", |c| {
        let item = crate::bind_world_object::item_argument(&c.arg(0))?;
        let fluid = call(c.thread, &item, "getFluidContainer", &[])?;
        let v = fluid.is_nil() || {
            let f = c.thread.table_get(&fluid, &Value::str("isEmpty"))?;
            c.thread.call1(&f, &[fluid.clone()])?.truthy()
        };
        Ok(c.push(v))
    });
    // `hotFluidContainer`: no `FluidContainer` component, or hotter than 1.6.
    b.static_fn("hotFluidContainer", |c| {
        let value = c.arg(0);
        let item = crate::bind_world_object::item_argument(&value)?;
        let has = crate::bind_world_object::fluid_component(&value)?.is_some();
        let v = !has || item.borrow().get_item_heat() > 1.6;
        Ok(c.push(v))
    });
    // `canAddToPack`: a `DrainableComboItem` with the `Packed` tag needs
    // less than one use left.
    b.static_fn("canAddToPack", |c| {
        let item = crate::bind_world_object::item_argument(&c.arg(0))?;
        let i = item.borrow();
        let v = i.template.class != pz_inventory::ItemClass::Drainable
            || !i.has_tag("base:packed")
            || i.get_current_uses_float() < 1.0;
        Ok(c.push(v))
    });
    // `purifyWater`: a `FluidContainer` component that contains
    // `Fluid.TaintedWater`; any other item.
    b.static_fn("purifyWater", |c| {
        let value = c.arg(0);
        crate::bind_world_object::item_argument(&value)?;
        let Some(component) = crate::bind_world_object::fluid_component(&value)? else {
            return Ok(c.push(true));
        };
        let body = component.try_borrow().map_err(|_| LuaError::runtime("FluidContainer is in use"))?;
        let fluid = body.fluid().ok_or_else(|| LuaError::runtime("FluidContainer allocation has an invalid class"))?;
        let tainted = pz_entity::FluidRef::Type(pz_scripts::objects::definitions::FluidType::TaintedWater.id());
        let v = fluid.fluids.iter().any(|f| f.fluid.as_ref() == Some(&tainted));
        Ok(c.push(v))
    });
    // `scratchTicket`: `scratched` is not true in the mod data.
    b.static_fn("scratchTicket", |c| {
        let item = crate::bind_world_object::item_argument(&c.arg(0))?;
        let data = call(c.thread, &item, "getModData", &[])?;
        let Value::Table(t) = data else { return Err(npe()) };
        Ok(c.push(!t.get_str("scratched").raw_equals(&Value::Bool(true))))
    });
    // `copyKey`: not a building key, or a key with an id.
    b.static_fn("copyKey", |c| {
        let item = crate::bind_world_object::item_argument(&c.arg(0))?;
        let i = item.borrow();
        let key_id = match &i.kind {
            ItemKind::Key { key_id, .. } => *key_id,
            _ => i.key_id,
        };
        Ok(c.push(!i.has_tag("base:buildingkey") || key_id != -1))
    });
}

/// `item.<name>(args)` through the item's binding.
fn call(thread: &mut Thread, item: &ItemRef, name: &str, args: &[Value]) -> LuaResult<Value> {
    let value = crate::bind_items::try_item_value(item)?;
    let f = thread.table_get(&value, &Value::str(name))?;
    let mut all = vec![value];
    all.extend_from_slice(args);
    thread.call1(&f, &all)
}

fn number(v: &Value) -> f64 {
    pz_kahlua::exec::raw_tonumber(v).unwrap_or(0.0)
}

/// `((Clothing) item).<method>()` when `have`, else `!(item instanceof
/// Clothing) || !item.<method>()`.
fn clothing_has(thread: &mut Thread, item: &ItemRef, method: &str, have: bool) -> LuaResult<bool> {
    let clothing = matches!(item.borrow().kind, ItemKind::Clothing(_) | ItemKind::AlarmClockClothing(..));
    if !clothing {
        if have {
            return Err(LuaError::runtime("java.lang.ClassCastException: not a Clothing"));
        }
        return Ok(true);
    }
    // Through the binding, which prepares the item's mod data.
    let value = crate::bind_items::try_item_value(item)?;
    let f = thread.table_get(&value, &Value::str(method))?;
    let has = thread.call1(&f, &[value])?.truthy();
    Ok(has == have)
}

fn npe() -> LuaError {
    LuaError::Runtime { message: String::new(), class: "java.lang.NullPointerException" }
}

/// `genericPacking(item, chr)`: an item as it comes out of a fresh pack.
/// Food must be `isNormalAndFullFood`; no blood, no dirt on clothing, an
/// empty container, full condition, and the script's colour.
fn generic_packing(thread: &mut Thread, sm: &ScriptManager, item: &ItemRef) -> LuaResult<bool> {
    if matches!(item.borrow().kind, ItemKind::Food(_)) && !is_normal_and_full_food(sm, item)? {
        return Ok(false);
    }
    // `getBloodLevel()` through its binding, which reads the item assets.
    let value = crate::bind_items::try_item_value(item)?;
    let f = thread.table_get(&value, &Value::str("getBloodLevel"))?;
    let blood = pz_kahlua::exec::raw_tonumber(&thread.call1(&f, &[value.clone()])?).unwrap_or(0.0) as f32;
    if blood > 0.0 {
        return Ok(false);
    }
    let i = item.borrow();
    match &i.kind {
        ItemKind::Clothing(_) | ItemKind::AlarmClockClothing(..) if i.get_dirtyness() > 0.0 => return Ok(false),
        ItemKind::Container(c) if c.container.slot().is_some_and(|c| !c.borrow().is_empty()) => return Ok(false),
        _ => {}
    }
    if i.condition != i.condition_max {
        return Ok(false);
    }
    let script = pz_characters::character::items::script_item(sm, &i).ok_or_else(npe)?;
    let s = script.borrow();
    Ok(i.color_red == s.r() && i.color_green == s.g() && i.color_blue == s.b())
}

/// `Food.isNormalAndFullFood()`.
fn is_normal_and_full_food(sm: &ScriptManager, item: &ItemRef) -> LuaResult<bool> {
    let (script, full_type) = {
        let i = item.borrow();
        let ItemKind::Food(f) = &i.kind else { return Ok(false) };
        if i.is_tainted() || i.is_rotten() || f.fertilized || f.spices.is_some() || f.compost_time != 0.0 {
            return Ok(false);
        }
        let Some(script) = pz_characters::character::items::script_item(sm, &i) else {
            return Ok(false);
        };
        if i.get_poison_power() as f32 != script.borrow().poison_power as f32 {
            return Ok(false);
        }
        (script, i.get_full_type())
    };
    Ok(is_whole_food_item(item, &script.borrow(), &full_type)? && is_uncooked(&item.borrow()))
}

/// `Food.isUncooked()`.
fn is_uncooked(i: &InventoryItem) -> bool {
    !i.is_cooked() && !i.is_burnt()
}

/// `Food.isWholeFoodItem()`: the script's hunger, unhappiness, boredom and
/// thirst, and the values of a new item of the same type.
fn is_whole_food_item(item: &ItemRef, script: &Item, full_type: &str) -> LuaResult<bool> {
    {
        let i = item.borrow();
        if i.get_hung_change() * 100.0 != script.hunger_change
            || i.get_unhappy_change() != script.unhappy_change
            || i.get_boredom_change() != script.boredom_change
            || thirst_change(&i) * 100.0 != script.thirst_change
        {
            return Ok(false);
        }
    }
    // `(Food) InventoryItemFactory.CreateItem(getFullType())`.
    let Some(fresh) = crate::bind_items::try_create_item(full_type, 1.0)? else {
        return Ok(false);
    };
    let (i, n) = (item.borrow(), fresh.borrow());
    let (ItemKind::Food(a), ItemKind::Food(b)) = (&i.kind, &n.kind) else {
        return Err(LuaError::runtime("java.lang.ClassCastException: not a Food"));
    };
    Ok(a.calories == b.calories
        && a.proteins == b.proteins
        && a.lipids == b.lipids
        && a.carbohydrates == b.carbohydrates
        && a.pain_reduction == b.pain_reduction
        && a.flu_reduction == b.flu_reduction
        && food_sickness_change(&i) == food_sickness_change(&n)
        && i.get_fatigue_change() == n.get_fatigue_change())
}

/// `Food.getThirstChange()`: a fifth when burnt, half when cooked.
fn thirst_change(i: &InventoryItem) -> f32 {
    let ItemKind::Food(f) = &i.kind else { return 0.0 };
    if f.burnt {
        f.thirst_change / 5.0
    } else if i.is_cooked() {
        f.thirst_change / 2.0
    } else {
        f.thirst_change
    }
}

/// `Food.getFoodSicknessChange()`: less when burnt or stale, more when
/// cooked.
fn food_sickness_change(i: &InventoryItem) -> i32 {
    let ItemKind::Food(f) = &i.kind else { return 0 };
    let s = f.food_sickness_change as f32;
    if f.burnt {
        (s / 3.0) as i32
    } else if f.age >= f.off_age as f32 && f.age < f.off_age_max as f32 {
        (s / 1.3) as i32
    } else if f.age >= f.off_age_max as f32 {
        (s / 2.2) as i32
    } else if i.is_cooked() {
        (s * 1.3) as i32
    } else {
        f.food_sickness_change
    }
}
