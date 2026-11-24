//! Build 42.22 `zombie.entity.components.fluids.Fluid` and
//! `zombie.iso.objects.IsoBarricade`.

use crate::bind_java_util::JavaList;
use crate::bind_misc_classes::color_value_of;
use crate::bind_world_object::{object_from_value, object_value, ISO_OBJECT};
use crate::expose::{expose_class, JavaEnum};
use pz_entity::Lookup;
use pz_kahlua::value::LuaObject;
use pz_kahlua::{LuaError, LuaResult, Thread, Value};
use pz_scripts::objects::definitions::{FluidCategory, FluidType};
use pz_world::objects::{IsoBarricade as WorldBarricade, IsoObject, ObjectKind};
use pz_world::ObjectSlot;
use std::any::Any;
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;

pub const FLUID_CLASS: &str = "zombie.entity.components.fluids.Fluid";
const FLUID_TYPE_CLASS: &str = "zombie.entity.components.fluids.FluidType";
const FLUID_CATEGORY_CLASS: &str = "zombie.entity.components.fluids.FluidCategory";
pub const ISO_BARRICADE_CLASS: &str = "zombie.iso.objects.IsoBarricade";

pub const MAX_PLANKS: i32 = 4;
pub const PLANK_HEALTH: i32 = 1000;
pub const METAL_BAR_HEALTH: i32 = 3000;
pub const METAL_HEALTH: i32 = 5000;
pub const METAL_HEALTH_DAMAGED: i32 = 2500;

/// The 27 public static `Fluid` fields declared on Build 42.22 `Fluid.class`.
const STATIC_FLUIDS: &[&str] = &[
    "Water",
    "TaintedWater",
    "Petrol",
    "Alcohol",
    "PoisonPotent",
    "Beer",
    "Whiskey",
    "SodaPop",
    "Coffee",
    "Tea",
    "Wine",
    "Bleach",
    "Blood",
    "Honey",
    "Mead",
    "Acid",
    "SpiffoJuice",
    "SecretFlavoring",
    "CarbonatedWater",
    "CleaningLiquid",
    "CowMilk",
    "SheepMilk",
    "AnimalBlood",
    "AnimalGrease",
    "Dye",
    "HairDye",
    "AnimalMilk",
];

#[derive(Clone, Debug)]
struct FluidEntry {
    name: String,
    fluid_type: FluidType,
    vanilla: bool,
    color: [f32; 4],
    categories: Vec<FluidCategory>,
    poisonous: bool,
}

impl FluidEntry {
    fn builtin(name: &str) -> Self {
        let fluid_type = if name == "Alcohol" {
            FluidType::RubbingAlcohol
        } else {
            FluidType::from_name_lower(name).unwrap_or(FluidType::Modded)
        };
        let (color, categories, poisonous) = match name {
            "Water" => (
                [0.0, 0.5, 1.0, 1.0],
                vec![FluidCategory::Water, FluidCategory::Beverage],
                false,
            ),
            "TaintedWater" => ([0.35, 0.55, 0.35, 1.0], vec![FluidCategory::Water], true),
            "Petrol" => (
                [0.85, 0.72, 0.25, 1.0],
                vec![FluidCategory::Fuel, FluidCategory::Hazardous],
                true,
            ),
            "Alcohol" => (
                [0.9, 0.9, 0.95, 1.0],
                vec![FluidCategory::Alcoholic, FluidCategory::Medical],
                false,
            ),
            "PoisonPotent" => (
                [0.25, 0.75, 0.2, 1.0],
                vec![FluidCategory::Poisons, FluidCategory::Hazardous],
                true,
            ),
            "Bleach" => (
                [0.92, 0.95, 0.85, 1.0],
                vec![FluidCategory::Hazardous, FluidCategory::Poisons],
                true,
            ),
            "CleaningLiquid" => (
                [0.4, 0.8, 0.9, 1.0],
                vec![FluidCategory::Hazardous],
                true,
            ),
            "Acid" => (
                [0.6, 0.9, 0.1, 1.0],
                vec![FluidCategory::Hazardous, FluidCategory::Poisons],
                true,
            ),
            "Blood" | "AnimalBlood" => (
                [0.65, 0.05, 0.05, 1.0],
                vec![FluidCategory::Hazardous],
                false,
            ),
            "CowMilk" | "SheepMilk" | "AnimalMilk" => (
                [0.97, 0.97, 0.95, 1.0],
                vec![FluidCategory::Beverage],
                false,
            ),
            "Beer" | "Whiskey" | "Wine" | "Mead" => (
                [0.75, 0.45, 0.15, 1.0],
                vec![FluidCategory::Alcoholic, FluidCategory::Beverage],
                false,
            ),
            "Coffee" | "Tea" | "SodaPop" | "SpiffoJuice" | "CarbonatedWater" => (
                [0.45, 0.28, 0.15, 1.0],
                vec![FluidCategory::Beverage],
                false,
            ),
            "Honey" => ([0.9, 0.7, 0.1, 1.0], vec![FluidCategory::Beverage], false),
            _ => ([1.0, 1.0, 1.0, 1.0], Vec::new(), false),
        };
        Self {
            name: name.to_owned(),
            fluid_type,
            vanilla: fluid_type != FluidType::Modded,
            color,
            categories,
            poisonous,
        }
    }
}

struct LuaFluid(FluidEntry);

impl LuaObject for LuaFluid {
    fn class_name(&self) -> &str {
        FLUID_CLASS
    }
    fn to_lua_string(&self) -> String {
        self.0.name.clone()
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

thread_local! {
    static FLUID_CACHE: RefCell<BTreeMap<String, Value>> = RefCell::new(BTreeMap::new());
}

fn ensure_default_fluids() {
    FLUID_CACHE.with(|cache| {
        let mut cache = cache.borrow_mut();
        if !cache.is_empty() {
            return;
        }
        for &name in STATIC_FLUIDS {
            let entry = FluidEntry::builtin(name);
            cache.insert(name.to_owned(), Value::Object(Rc::new(LuaFluid(entry))));
        }
    });
}

fn lookup_fluid_value(name: &str) -> Option<Value> {
    ensure_default_fluids();
    if let Some(found) = FLUID_CACHE.with(|c| {
        let c = c.borrow();
        if let Some(v) = c.get(name) {
            return Some(v.clone());
        }
        let lower = name.to_ascii_lowercase();
        c.iter()
            .find(|(k, _)| k.to_ascii_lowercase() == lower)
            .map(|(_, v)| v.clone())
    }) {
        return Some(found);
    }
    let env = crate::bind_items::item_env();
    let info = env.entity().and_then(|ctx| match ctx.fluid_by_name(name) {
        Lookup::Found(info) => Some(info),
        _ => None,
    });
    if let Some(info) = info {
        let entry = FluidEntry {
            name: info.name.clone(),
            fluid_type: FluidType::from_name_lower(&info.name).unwrap_or(FluidType::Modded),
            vanilla: FluidType::from_name_lower(&info.name).is_some(),
            color: info.color,
            categories: info.categories,
            poisonous: false,
        };
        let val = Value::Object(Rc::new(LuaFluid(entry)));
        FLUID_CACHE.with(|c| c.borrow_mut().insert(info.name, val.clone()));
        return Some(val);
    }
    if let Some(ft) = FluidType::from_name_lower(name) {
        let entry = FluidEntry::builtin(ft.name());
        let val = Value::Object(Rc::new(LuaFluid(entry)));
        FLUID_CACHE.with(|c| c.borrow_mut().insert(ft.name().to_owned(), val.clone()));
        return Some(val);
    }
    None
}

fn with_fluid<R>(v: &Value, f: impl FnOnce(&FluidEntry) -> R) -> Option<R> {
    if let Value::Object(o) = v {
        if let Some(fl) = o.as_any().downcast_ref::<LuaFluid>() {
            return Some(f(&fl.0));
        }
    }
    None
}

pub(crate) fn reset_session_handles() {}

pub fn register(thread: &mut Thread) {
    register_fluid(thread);
    register_iso_barricade(thread);
}

fn register_fluid(thread: &mut Thread) {
    ensure_default_fluids();
    let fl = expose_class(thread, FLUID_CLASS, Some("java.lang.Object"));

    for &name in STATIC_FLUIDS {
        if let Some(v) = lookup_fluid_value(name) {
            fl.field(name, v);
        }
    }

    fl.static_fn("Get", |c| {
        let arg = c.arg(0);
        let key: Option<String> = match &arg {
            Value::Str(s) => Some(s.to_string()),
            Value::Object(o) => o
                .as_any()
                .downcast_ref::<JavaEnum>()
                .filter(|e| e.class == FLUID_TYPE_CLASS)
                .map(|e| e.name.to_owned()),
            _ => None,
        };
        let out = key
            .as_deref()
            .and_then(lookup_fluid_value)
            .unwrap_or(Value::Nil);
        Ok(c.push(out))
    });
    fl.static_fn("getAllFluids", |c| {
        ensure_default_fluids();
        let items: Vec<Value> =
            FLUID_CACHE.with(|cache| cache.borrow().values().cloned().collect());
        Ok(c.push(Value::Object(Rc::new(JavaList::new(items)))))
    });
    fl.static_fn("getAllFluidItemsDebug", |c| {
        Ok(c.push(Value::Object(Rc::new(JavaList::new(Vec::new())))))
    });
    fl.static_fn("FluidsInitialized", |c| Ok(c.push(true)));
    fl.static_fn("Reset", |_c| Ok(0));

    fl.method("isVanilla", |c| {
        let v = with_fluid(&c.arg(0), |f| f.vanilla).unwrap_or(false);
        Ok(c.push(v))
    });
    fl.method("toString", |c| {
        let s = with_fluid(&c.arg(0), |f| f.name.clone()).unwrap_or_default();
        Ok(c.push(Value::str(&s)))
    });
    fl.method("getFluidTypeString", |c| {
        let s = with_fluid(&c.arg(0), |f| f.name.clone()).unwrap_or_default();
        Ok(c.push(Value::str(&s)))
    });
    fl.method("getFluidType", |c| {
        let ft = with_fluid(&c.arg(0), |f| f.fluid_type).unwrap_or(FluidType::None);
        let val = c
            .thread
            .globals()
            .get_str("FluidType")
            .as_table()
            .map(|tbl| tbl.get_str(ft.name()))
            .unwrap_or_else(|| {
                Value::Object(Rc::new(JavaEnum {
                    class: FLUID_TYPE_CLASS,
                    name: ft.name(),
                    ordinal: ft.id() as i32,
                }))
            });
        Ok(c.push(val))
    });
    fl.method("getColor", |c| {
        let [r, g, b, a] = with_fluid(&c.arg(0), |f| f.color).unwrap_or([1.0, 1.0, 1.0, 1.0]);
        Ok(c.push(color_value_of(r, g, b, a)))
    });
    fl.method("isCategory", |c| {
        let arg = c.arg(1);
        let target: Option<FluidCategory> = match &arg {
            Value::Str(s) => FluidCategory::ALL
                .iter()
                .copied()
                .find(|cat| cat.name().eq_ignore_ascii_case(s)),
            Value::Object(o) => o
                .as_any()
                .downcast_ref::<JavaEnum>()
                .filter(|e| e.class == FLUID_CATEGORY_CLASS)
                .and_then(|e| {
                    FluidCategory::ALL
                        .iter()
                        .copied()
                        .find(|cat| cat.name() == e.name)
                }),
            _ => None,
        };
        let matched = target.is_some_and(|cat| {
            with_fluid(&c.arg(0), |f| f.categories.contains(&cat)).unwrap_or(false)
        });
        Ok(c.push(matched))
    });
    fl.method("getDisplayName", |c| {
        let name = with_fluid(&c.arg(0), |f| f.name.clone()).unwrap_or_default();
        let key = format!("Fluid_Name_{name}");
        let translated = crate::item_tooltip::tr(&key);
        let out = if translated == key { name } else { translated };
        Ok(c.push(Value::str(&out)))
    });
    fl.method("getTranslatedName", |c| {
        let name = with_fluid(&c.arg(0), |f| f.name.clone()).unwrap_or_default();
        let key = format!("Fluid_Name_{name}");
        let translated = crate::item_tooltip::tr(&key);
        let out = if translated == key { name } else { translated };
        Ok(c.push(Value::str(&out)))
    });
    fl.method("getTranslatedNameLower", |c| {
        let name = with_fluid(&c.arg(0), |f| f.name.clone()).unwrap_or_default();
        let key = format!("Fluid_Name_{name}");
        let translated = crate::item_tooltip::tr(&key);
        let out = if translated == key { name } else { translated };
        let lower = out.to_ascii_lowercase();
        Ok(c.push(Value::str(&lower)))
    });
    fl.method("canBlendWith", |c| {
        let ok = with_fluid(&c.arg(0), |_| true).unwrap_or(false)
            && with_fluid(&c.arg(1), |_| true).unwrap_or(false);
        Ok(c.push(ok))
    });
    fl.method("isPoisonous", |c| {
        let p = with_fluid(&c.arg(0), |f| f.poisonous).unwrap_or(false);
        Ok(c.push(p))
    });
}

fn with_barricade<R>(v: &Value, f: impl FnOnce(&WorldBarricade) -> R) -> LuaResult<R> {
    let obj = object_from_value(v)
        .ok_or_else(|| LuaError::runtime("expected IsoBarricade receiver"))?;
    let body = obj
        .object
        .try_borrow()
        .map_err(|_| LuaError::runtime("IsoBarricade borrowed"))?;
    match &body.kind {
        ObjectKind::Barricade(b) => Ok(f(b)),
        _ => Err(LuaError::runtime("receiver is not an IsoBarricade")),
    }
}

fn with_barricade_mut<R>(v: &Value, f: impl FnOnce(&mut WorldBarricade) -> R) -> LuaResult<R> {
    let obj = object_from_value(v)
        .ok_or_else(|| LuaError::runtime("expected IsoBarricade receiver"))?;
    let mut body = obj
        .object
        .try_borrow_mut()
        .map_err(|_| LuaError::runtime("IsoBarricade borrowed"))?;
    match &mut body.kind {
        ObjectKind::Barricade(b) => Ok(f(b)),
        _ => Err(LuaError::runtime("receiver is not an IsoBarricade")),
    }
}

fn register_iso_barricade(thread: &mut Thread) {
    let bar = expose_class(thread, ISO_BARRICADE_CLASS, Some(ISO_OBJECT));

    bar.field("MAX_PLANKS", Value::Num(MAX_PLANKS as f64));
    bar.field("PLANK_HEALTH", Value::Num(PLANK_HEALTH as f64));
    bar.field("METAL_BAR_HEALTH", Value::Num(METAL_BAR_HEALTH as f64));
    bar.field("METAL_HEALTH", Value::Num(METAL_HEALTH as f64));
    bar.field("METAL_HEALTH_DAMAGED", Value::Num(METAL_HEALTH_DAMAGED as f64));

    bar.static_fn("AddBarricadeToObject", |c| {
        let to = c.arg(0);
        let opposite = matches!(c.arg(1), Value::Bool(true));
        let (sq_slot, dir) = if let Some(target) = object_from_value(&to) {
            let body = target
                .object
                .try_borrow()
                .map_err(|_| LuaError::runtime("IsoBarricade target borrowed"))?;
            let north = match &body.kind {
                ObjectKind::Door(d) => d.north,
                ObjectKind::Window(w) => w.north,
                ObjectKind::Thumpable(t) => t.north,
                _ => true,
            };
            let dir: i8 = match (north, opposite) {
                (true, false) => 0,
                (true, true) => 4,
                (false, false) => 2,
                (false, true) => 6,
            };
            (body.square_slot().cloned(), dir)
        } else {
            (None, if opposite { 4 } else { 0 })
        };

        let mut obj = IsoObject::new(
            0,
            ObjectKind::Barricade(WorldBarricade {
                dir,
                plank_health: [0; 4],
                metal_health: 0,
                metal_bar_health: 0,
            }),
            None,
        );
        if let Some(sq) = sq_slot.as_ref() {
            obj.replace_square(Some(sq), &mut pz_entity::RetiredComponents::default());
        }
        let slot = ObjectSlot::new(obj);
        let obj_ref = slot.ref_handle();
        crate::bind_world_object::ensure_class(&obj_ref, ISO_BARRICADE_CLASS)?;
        if let Some(sq) = sq_slot {
            sq.borrow_mut().objects.push(slot);
        }
        let val = object_value(&obj_ref)?;
        Ok(c.push(val))
    });

    bar.method("getObjectName", |c| Ok(c.push("Barricade")));
    bar.method("getNumPlanks", |c| {
        let n = with_barricade(&c.arg(0), |b| {
            b.plank_health.iter().filter(|&&h| h > 0).count() as i32
        })?;
        Ok(c.push(n))
    });
    bar.method("canAddPlank", |c| {
        let ok = with_barricade(&c.arg(0), |b| {
            b.metal_health <= 0
                && b.metal_bar_health <= 0
                && b.plank_health.iter().any(|&h| h <= 0)
        })?;
        Ok(c.push(ok))
    });
    bar.method("isMetal", |c| {
        let v = with_barricade(&c.arg(0), |b| b.metal_health > 0)?;
        Ok(c.push(v))
    });
    bar.method("isMetalBar", |c| {
        let v = with_barricade(&c.arg(0), |b| b.metal_bar_health > 0)?;
        Ok(c.push(v))
    });
    bar.method("isBlockVision", |c| {
        let v = with_barricade(&c.arg(0), |b| {
            b.metal_health > 0 || b.plank_health.iter().filter(|&&h| h > 0).count() >= 3
        })?;
        Ok(c.push(v))
    });
    bar.method("isDestroyed", |c| {
        let v = with_barricade(&c.arg(0), |b| {
            b.metal_health <= 0
                && b.metal_bar_health <= 0
                && b.plank_health.iter().all(|&h| h <= 0)
        })?;
        Ok(c.push(v))
    });
    bar.method("addPlank", |c| {
        with_barricade_mut(&c.arg(0), |b| {
            if b.metal_health <= 0 && b.metal_bar_health <= 0 {
                if let Some(slot) = b.plank_health.iter_mut().find(|h| **h <= 0) {
                    *slot = PLANK_HEALTH;
                }
            }
        })?;
        Ok(0)
    });
    bar.method("removePlank", |c| {
        with_barricade_mut(&c.arg(0), |b| {
            if let Some(slot) = b.plank_health.iter_mut().rev().find(|h| **h > 0) {
                *slot = 0;
            }
        })?;
        Ok(c.push(Value::Nil))
    });
    bar.method("addMetal", |c| {
        with_barricade_mut(&c.arg(0), |b| {
            b.plank_health = [0; 4];
            b.metal_bar_health = 0;
            b.metal_health = METAL_HEALTH;
        })?;
        Ok(0)
    });
    bar.method("removeMetal", |c| {
        with_barricade_mut(&c.arg(0), |b| {
            b.metal_health = 0;
        })?;
        Ok(c.push(Value::Nil))
    });
    bar.method("addMetalBar", |c| {
        with_barricade_mut(&c.arg(0), |b| {
            b.plank_health = [0; 4];
            b.metal_health = 0;
            b.metal_bar_health = METAL_BAR_HEALTH;
        })?;
        Ok(0)
    });
    bar.method("removeMetalBar", |c| {
        with_barricade_mut(&c.arg(0), |b| {
            b.metal_bar_health = 0;
        })?;
        Ok(c.push(Value::Nil))
    });
    bar.method("getHealth", |c| {
        let h = with_barricade(&c.arg(0), |b| {
            if b.metal_health > 0 {
                b.metal_health
            } else if b.metal_bar_health > 0 {
                b.metal_bar_health
            } else {
                b.plank_health.iter().copied().sum()
            }
        })?;
        Ok(c.push(h))
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use pz_kahlua::{stdlib, TableRef};
    use pz_world::objects::IsoWindow;
    use pz_world::{IsoGridSquare, SquareSlot};

    #[test]
    fn fluid_and_iso_barricade_lua() {
        reset_session_handles();
        let mut t = Thread::new(TableRef::new());
        stdlib::install(&mut t, None).unwrap();
        crate::bind_java_util::register(&mut t);
        crate::bind_misc_classes::register(&mut t, None);
        crate::bind_world_object::register_classes(&mut t);
        register(&mut t);

        let sq = SquareSlot::new(IsoGridSquare::new(10, 10, 0)).ref_handle();
        let mut win = IsoObject::new(
            1,
            ObjectKind::Window(IsoWindow {
                north: true,
                locked: false,
                perma_locked: false,
                open: false,
                destroyed: false,
                glass_removed: false,
                health: 100,
                max_health: 100,
                open_sprite: None,
                closed_sprite: None,
                smashed_sprite: None,
                glass_removed_sprite: None,
            }),
            None,
        );
        win.replace_square(Some(&sq), &mut pz_entity::RetiredComponents::default());
        let win_ref = ObjectSlot::new(win).ref_handle();
        crate::bind_world_object::ensure_class(&win_ref, "zombie.iso.objects.IsoWindow").unwrap();
        t.globals()
            .set_str("testWindow", object_value(&win_ref).unwrap());

        let src = r#"
            local function assert(v, msg)
                if not v then error(msg or "assertion failed") end
            end

            assert(Fluid.Water ~= nil)
            assert(Fluid.TaintedWater ~= nil)
            assert(Fluid.Petrol ~= nil)
            assert(Fluid.Bleach ~= nil)
            assert(Fluid.AnimalMilk ~= nil)
            assert(Fluid.Get("Water") == Fluid.Water)
            assert(Fluid.Get("petrol") == Fluid.Petrol)
            assert(Fluid.Water:getFluidTypeString() == "Water")
            assert(Fluid.Water:isVanilla() == true)
            assert(Fluid.Water:isPoisonous() == false)
            assert(Fluid.Bleach:isPoisonous() == true)
            assert(Fluid.Water:getColor():getB() > 0.5)
            assert(Fluid.getAllFluids():size() >= 27)
            assert(Fluid.FluidsInitialized() == true)

            assert(IsoBarricade.MAX_PLANKS == 4)
            assert(IsoBarricade.PLANK_HEALTH == 1000)
            assert(IsoBarricade.METAL_HEALTH == 5000)
            local bar = IsoBarricade.AddBarricadeToObject(testWindow, false)
            assert(bar ~= nil)
            assert(bar:getObjectName() == "Barricade")
            assert(bar:isDestroyed() == true)
            assert(bar:canAddPlank() == true)
            bar:addPlank(nil)
            bar:addPlank(nil)
            bar:addPlank(nil)
            assert(bar:getNumPlanks() == 3)
            assert(bar:isBlockVision() == true)
            assert(bar:isDestroyed() == false)
            bar:removePlank(nil)
            assert(bar:getNumPlanks() == 2)
            assert(bar:isBlockVision() == false)
            bar:addMetal(nil)
            assert(bar:isMetal() == true)
            assert(bar:canAddPlank() == false)
            bar:removeMetal(nil)
            assert(bar:isMetal() == false)
            bar:addMetalBar(nil)
            assert(bar:isMetalBar() == true)
        "#;
        let f = stdlib::load_string(src, Some("fluid_barricade"), t.globals().clone()).unwrap();
        t.pcall(&f, &[]).unwrap();
    }
}
