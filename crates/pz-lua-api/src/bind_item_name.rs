//! `InventoryItem.getName()` / `getDisplayName()` and their overrides
//! (`Clothing`, `Food`, `Moveable`/`Radio`), the item name Lua shows with
//! its state prefixes ("Broken", "Worn", "Empty", "Fresh, Uncooked", ...)
//! and a fluid container's `FluidContainer.getUiName()`, plus the virtual
//! `getBloodLevel()` they read.
//!
//! Not modelled: the world item's fluid container (`getWorldItem()`; items
//! reached from Lua here have none), `getSharpness()`'s write back of the
//! clamped sharpness (`applyMaxSharpness`), and the
//! `EnableTaintedWaterText` sandbox option (its default, on).

use crate::bind_items::ItemLuaEnv;
use pz_entity::components::attributes::AttributeValue;
use pz_entity::{EntityContext, FluidContainer, FluidInstance, FluidRef};
use pz_inventory::item::ItemKind;
use pz_inventory::{InventoryItem, ItemAssets, ItemClass};
use pz_scripts::objects::definitions::{
    FluidCategory, FluidDefinitionScript, FluidType, PoisonEffect,
};
use pz_scripts::ScriptType;
use pz_text::jfmt::JavaArg;

/// `Translator.getText(key, args...)` (the key without a translator).
fn text(key: &str, args: &[&str]) -> String {
    let a: Vec<JavaArg> = args.iter().map(|s| JavaArg::Str(s.to_string())).collect();
    crate::bind_text::with_translator(|t| t.get_text(key, &a)).unwrap_or_else(|| key.to_string())
}

/// `Translator.getFluidText(key)`.
fn fluid_text(key: &str) -> String {
    crate::bind_text::with_translator(|t| t.get_fluid_text(key)).unwrap_or_else(|| key.to_string())
}

/// `getBloodLevel()`: `Clothing`'s and `HandWeapon`'s field,
/// `InventoryContainer`'s sum of `getBlood(part)` over its blood clothing
/// type's covered parts, 0 for the rest.
pub fn blood_level(item: &InventoryItem, assets: Option<&ItemAssets>) -> f32 {
    match &item.kind {
        ItemKind::Weapon(w) => w.blood_level,
        ItemKind::Clothing(c) | ItemKind::AlarmClockClothing(c, _) => c.blood_level,
        ItemKind::Container(_) => {
            let (Some(names), Some(a)) = (item.get_blood_clothing_type(), assets) else {
                return 0.0;
            };
            let parts = a.blood_clothing_types.covered_parts_of(names);
            parts.iter().fold(0.0f32, |t, &p| {
                t + item.visual.as_ref().map_or(0.0, |v| v.get_blood(p))
            })
        }
        _ => 0.0,
    }
}

fn attribute_f32(v: &AttributeValue) -> Option<f32> {
    Some(match v {
        AttributeValue::Float(f) => *f,
        AttributeValue::Double(d) => *d as f32,
        AttributeValue::Int(i) => *i as f32,
        AttributeValue::Short(i) => *i as f32,
        AttributeValue::Byte(i) => *i as f32,
        AttributeValue::Long(i) => *i as f32,
        _ => return None,
    })
}

/// `getSharpness()` when `hasSharpness()`: the attribute, at most
/// `getMaxSharpness()`.
fn sharpness(item: &InventoryItem, ctx: &dyn EntityContext) -> Option<f32> {
    let v = attribute_f32(&item.get_attribute(ctx, "Sharpness")?)?;
    let max = match item.get_attribute(ctx, "HeadCondition") {
        Some(head) => {
            let head = attribute_f32(&head).unwrap_or(0.0) as i32;
            let head_max = item
                .get_attribute(ctx, "HeadConditionMax")
                .and_then(|m| attribute_f32(&m))
                .map_or(item.condition_max, |m| m as i32);
            head as f32 / head_max as f32
        }
        None => item.condition as f32 / item.condition_max as f32,
    };
    Some(if v > max { max } else { v })
}

/// The fluid's definition script (`Fluid.script`: the last definition
/// registered for it).
fn fluid_script(f: &FluidRef) -> Option<std::rc::Rc<std::cell::RefCell<FluidDefinitionScript>>> {
    let sm = crate::bind_registry::shared_script_manager();
    let all = sm.get_all::<FluidDefinitionScript>(ScriptType::FluidDefinition);
    all.into_iter().rev().find(|d| {
        let d = d.borrow();
        match f {
            FluidRef::Type(id) => d.fluid_type() != FluidType::Modded && d.fluid_type().id() == *id,
            FluidRef::Modded(s) => {
                d.fluid_type() == FluidType::Modded && d.fluid_type_string() == Some(s.as_str())
            }
        }
    })
}

struct FluidView {
    name: String,
    amount: f32,
    categories: Vec<FluidCategory>,
    poisonous: bool,
    tainted_water: bool,
}

fn view(i: &FluidInstance) -> FluidView {
    let script = i.fluid.as_ref().and_then(fluid_script);
    let s = script.as_ref().map(|s| s.borrow());
    FluidView {
        // `Fluid.getDisplayName`: the script's (translated) display name.
        name: s.as_ref().map_or("<unknown_fluid>".to_string(), |s| {
            fluid_text(s.display_name())
        }),
        amount: i.amount,
        categories: s
            .as_ref()
            .map_or(Vec::new(), |s| s.categories().iter().copied().collect()),
        poisonous: s
            .as_ref()
            .is_some_and(|s| s.poison_max_effect() != PoisonEffect::None),
        tainted_water: matches!(i.fluid, Some(FluidRef::Type(id)) if id == FluidType::TaintedWater.id()),
    }
}

/// `FluidContainer.getUiName()` (with `EnableTaintedWaterText` on).
pub fn fluid_ui_name(fc: &FluidContainer, item: &InventoryItem) -> String {
    let cname = fluid_text(&format!("Fluid_Container_{}", fc.container_name));
    let fluids: Vec<FluidView> = fc.fluids.iter().map(view).collect();
    let poisonous = fluids.iter().any(|f| f.poisonous);
    let tainted = fluids.iter().any(|f| f.tainted_water);
    let one = |f: &FluidView| text("Fluid_HoldingOneType", &[&cname, &f.name]);
    match fluids.len() {
        0 => {
            if item.has_tag("base:omitemptyfromname") {
                cname
            } else {
                text("Fluid_HoldingNone", &[&cname])
            }
        }
        1 => {
            let mut n = one(&fluids[0]);
            if fluids[0].tainted_water {
                n = format!("{n} {}", fluid_text("Fluid_Tainted"));
            }
            if !fc.can_player_empty {
                n = format!("{n} {}", fluid_text("Fluid_Sealed"));
            }
            n
        }
        2 => {
            let (a, b) = (&fluids[0], &fluids[1]);
            let mut n = if poisonous {
                one(if !a.categories.contains(&FluidCategory::Poisons) {
                    a
                } else {
                    b
                })
            } else {
                let (less, more) = if a.amount < b.amount { (a, b) } else { (b, a) };
                if a.name != b.name {
                    text("Fluid_HoldingTwoTypes", &[&cname, &less.name, &more.name])
                } else {
                    one(less)
                }
            };
            // `contains(TaintedWater) && getPoisonEffect() != None`.
            if tainted && poisonous {
                n = format!("{n} {}", fluid_text("Fluid_Tainted"));
            }
            n
        }
        _ => {
            let mut all_beverages = true;
            let mut alcoholics = 0;
            for f in &fluids {
                let beverage = f.categories.contains(&FluidCategory::Beverage);
                if !beverage {
                    all_beverages = f.categories.contains(&FluidCategory::Poisons);
                }
                if beverage && f.categories.contains(&FluidCategory::Alcoholic) {
                    alcoholics += 1;
                }
            }
            let what = if !all_beverages {
                "Fluid_Mixed_Fluids"
            } else if alcoholics >= 2 {
                "Fluid_Cocktail"
            } else {
                "Fluid_Mixed_Beverages"
            };
            text("Fluid_HoldingOneType", &[&cname, &fluid_text(what)])
        }
    }
}

/// Joins the `", "`-terminated prefix the way the originals do (drop the
/// last two characters, trim) and names the item with `naming`.
fn with_prefix(prefix: String, name: &str, naming: &str) -> String {
    let mut p = prefix;
    if p.len() > 2 {
        p.truncate(p.len() - 2);
    }
    let p = p.trim();
    if p.is_empty() {
        name.to_string()
    } else {
        text(naming, &[p, name])
    }
}

/// `getName()` with the item's class's override.
pub fn item_name(item: &InventoryItem, env: &dyn ItemLuaEnv) -> String {
    let ctx = env.context();
    let entity = env.entity();
    let fluid = || entity.and_then(|e| item.get_fluid_container(e));
    match &item.kind {
        ItemKind::Clothing(c) | ItemKind::AlarmClockClothing(c, _) => {
            let mut p = String::new();
            if c.dirtyness > 15.0 {
                p += &(text("IGUI_ClothingName_Dirty", &[]) + ", ");
            }
            if c.blood_level > 25.0 {
                p += &(text("IGUI_ClothingName_Bloody", &[]) + ", ");
            }
            if c.wetness >= 100.0 {
                p += &(text("IGUI_ClothingName_Soaked", &[]) + ", ");
            } else if c.wetness > 25.0 {
                p += &(text("IGUI_ClothingName_Wet", &[]) + ", ");
            }
            if item.is_broken() {
                p += &(text("Tooltip_Clothing_broken", &[]) + ", ");
            } else if (item.condition as f32) < item.condition_max as f32 / 3.0 {
                p += &(text("IGUI_ClothingName_Worn", &[]) + ", ");
            }
            if let Some(fc) = fluid() {
                return fluid_ui_name(&fc, item);
            }
            with_prefix(p, &item.name, "IGUI_ClothingNaming")
        }
        ItemKind::Food(f) => {
            const NEVER: i32 = 1_000_000_000;
            let mut p = String::new();
            let mut add = |key: &str| p += &(text(key, &[]) + ", ");
            if f.fertilized {
                add("Tooltip_food_Fresh");
            }
            if f.burnt {
                add("Tooltip_food_Burnt");
            } else if !f.fertilized && f.off_age < NEVER && f.age < f.off_age as f32 {
                add("Tooltip_food_Fresh");
            } else if !f.fertilized && f.off_age_max < NEVER && f.age >= f.off_age_max as f32 {
                add("Tooltip_food_Rotten");
            } else if !f.fertilized && f.off_age_max < NEVER && f.age >= f.off_age as f32 {
                add("Tooltip_food_Stale");
            }
            let hide_cooked = item.has_tag("base:hidecooked");
            if f.cooked && !f.burnt && !hide_cooked {
                if item.has_tag("base:grilled") {
                    add("Tooltip_food_Grilled");
                } else if item.has_tag("base:toastable") {
                    add("Tooltip_food_Toasted");
                } else {
                    add("Tooltip_food_Cooked");
                }
            } else if f.is_cookable
                && !f.burnt
                && !hide_cooked
                && !item.has_tag("base:hideuncooked")
            {
                add("Tooltip_food_Uncooked");
            }
            if f.frozen {
                add("Tooltip_food_Frozen");
            }
            with_prefix(p, &item.name, "IGUI_FoodNaming")
        }
        ItemKind::Moveable(m) | ItemKind::Radio(m, _) => {
            if let ItemKind::Radio(_, d) = &item.kind {
                if d.is_turned_on {
                    return text(
                        "IGUI_ClothingNaming",
                        &[&text("Tooltip_activated", &[]), &item.name],
                    );
                }
            }
            if item.has_tag("base:usedisplayname") {
                let full = item.get_full_type();
                if let Some(n) = crate::bind_items::script_display_name(&full) {
                    return n;
                }
            }
            if m.movable_full_name == pz_inventory::item::MOVEABLE_OBJECT {
                return item.name.clone();
            }
            let tr = |n: &str| {
                crate::bind_text::with_translator(|t| t.get_moveable_display_name(n))
                    .unwrap_or_else(|| n.to_string())
            };
            if m.movable_full_name == item.name {
                return tr(&m.custom_name_full);
            }
            let rest = m
                .custom_name_full
                .get(m.movable_full_name.len()..)
                .unwrap_or("");
            tr(&m.movable_full_name) + rest
        }
        _ => {
            if let Some(fc) = fluid() {
                return fluid_ui_name(&fc, item);
            }
            if item.remote_control_id != -1 {
                return text("IGUI_ItemNameControllerLinked", &[&item.name]);
            }
            let mut fake = item.name.clone();
            let mech = crate::bind_items::script_vehicle_type(&item.get_full_type());
            if mech > 0 {
                let t = text(&format!("IGUI_VehicleType_{mech}"), &[]);
                fake = text("IGUI_ItemNameMechanicalType", &[&fake, &t]);
            }
            let mut p = String::new();
            if blood_level(item, ctx.assets()) > 0.25 {
                p += &(text("IGUI_ClothingName_Bloody", &[]) + ", ");
            }
            if item.is_broken() {
                p += &(text("Tooltip_broken", &[]) + ", ");
            } else if (item.condition as f32) < item.condition_max as f32 / 3.0 {
                p += &(text("IGUI_Name_Worn", &[]) + ", ");
            }
            if !item.is_broken() {
                if let Some(s) = entity.and_then(|e| sharpness(item, e)) {
                    if s < 0.333_333_34 {
                        let key = if s <= 0.0 {
                            "Tooltip_blunt"
                        } else {
                            "Tooltip_dull"
                        };
                        p += &(text(key, &[]) + ", ");
                    }
                }
            }
            if matches!(
                item.template.class,
                ItemClass::Drainable | ItemClass::WeaponPart
            ) && item.get_current_uses_float() <= 0.0
            {
                p += &(text("ContextMenu_Empty", &[]) + ", ");
            }
            if item.can_be_activated() && item.is_activated() {
                p += &(text("Tooltip_activated", &[]) + ", ");
            }
            with_prefix(p, &fake, "IGUI_ClothingNaming")
        }
    }
}

/// `getDisplayName()`: the name field, `Moveable`'s (and `Radio`'s)
/// `getName()`.
pub fn display_name(item: &InventoryItem, env: &dyn ItemLuaEnv) -> String {
    match item.kind {
        ItemKind::Moveable(_) | ItemKind::Radio(..) => item_name(item, env),
        _ => item.name.clone(),
    }
}
