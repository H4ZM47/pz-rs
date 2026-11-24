//! `zombie.inventory.FixingManager` and the inventory-query methods of
//! `zombie.scripting.objects.Fixing` (`usedInFixer`, `haveGlobalItem`,
//! `haveThisFixer`, `countUses`, `getRequiredFixerItems`, `getRequiredItems`).

use std::cell::RefCell;
use std::rc::Rc;

use pz_inventory::shared::ContainerRef;
use pz_inventory::template::ItemClass;
use pz_kahlua::{LuaError, LuaResult, NativeCall, Thread, Value};
use pz_scripts::objects::recipes::{Fixer, Fixing};
use pz_scripts::{ScriptManager, ScriptType};

use crate::bind_character_sim::CharacterObject;
use crate::bind_items::{item_value, ItemRef, LuaInventoryItem};
use crate::bind_java_util::JavaList;
use crate::bind_scripts::{handle_arg, script_value, ScriptHandle};
use crate::expose::expose_class;

pub const FIXING_MANAGER_CLASS: &str = "zombie.inventory.FixingManager";

thread_local! {
    static FIXING_RAND: RefCell<pz_core::random::Rand> =
        RefCell::new(pz_core::random::Rand::new(0xF141_9601));
}

fn item_arg(c: &NativeCall<'_>, idx: usize) -> LuaResult<ItemRef> {
    match c.arg(idx) {
        Value::Object(o) => o
            .as_any()
            .downcast_ref::<LuaInventoryItem>()
            .map(|l| l.item.clone())
            .ok_or_else(|| LuaError::runtime("expected InventoryItem")),
        _ => Err(LuaError::runtime("expected InventoryItem")),
    }
}

fn opt_item_arg(c: &NativeCall<'_>, idx: usize) -> Option<ItemRef> {
    match c.arg(idx) {
        Value::Object(o) => o
            .as_any()
            .downcast_ref::<LuaInventoryItem>()
            .map(|l| l.item.clone()),
        _ => None,
    }
}

fn fixing_arg(c: &NativeCall<'_>, idx: usize) -> LuaResult<ScriptHandle> {
    handle_arg(c, idx)?.ok_or_else(|| LuaError::runtime("expected Fixing"))
}

fn fixer_arg(c: &NativeCall<'_>, idx: usize) -> LuaResult<Fixer> {
    let h = handle_arg(c, idx)?.ok_or_else(|| LuaError::runtime("expected Fixing$Fixer"))?;
    h.read(|f: &Fixer| f.clone())
        .map_err(|e| LuaError::runtime(format!("Fixing$Fixer: {e:?}")))
}

fn chr_inventory(val: &Value) -> Option<ContainerRef> {
    match val {
        Value::Object(o) => o
            .as_any()
            .downcast_ref::<CharacterObject>()
            .map(|ch| ch.0.borrow().items.inventory.clone()),
        _ => None,
    }
}

fn item_matches_name(item: &ItemRef, expected: &str) -> bool {
    let it = item.borrow();
    it.get_type() == expected || it.get_full_type() == expected
}

/// Collect matching items in `inv` for `fixer_name`, excluding `broken_item` by identity.
fn collect_matching_items(
    inv: &ContainerRef,
    fixer_name: &str,
    broken_item: Option<&ItemRef>,
) -> Vec<ItemRef> {
    inv.borrow()
        .items
        .iter()
        .filter(|it| {
            if let Some(broken) = broken_item {
                if Rc::ptr_eq(it, broken) {
                    return false;
                }
            }
            item_matches_name(it, fixer_name)
        })
        .cloned()
        .collect()
}

/// `Fixing.countUses(chr, fixer, brokenItem)`.
pub fn count_uses(chr: &Value, fixer: &Fixer, broken_item: Option<&ItemRef>) -> i32 {
    let Some(inv) = chr_inventory(chr) else {
        return 0;
    };
    let mut total = 0i32;
    for it in collect_matching_items(&inv, fixer.fixer_name(), broken_item) {
        let b = it.borrow();
        if b.template.class == ItemClass::Drainable {
            total += b.get_current_uses().max(0);
        } else {
            total += 1;
        }
    }
    total
}

/// `FixingManager.useFixer(chr, fixer, brokenItem)`: consume `fixer.number_of_use()`
/// uses across matching items in `chr`'s inventory.
pub fn use_fixer(chr: &Value, fixer: &Fixer, broken_item: Option<&ItemRef>) {
    let Some(inv) = chr_inventory(chr) else {
        return;
    };
    let mut remaining = fixer.number_of_use().max(0);
    if remaining == 0 {
        return;
    }
    let candidates = collect_matching_items(&inv, fixer.fixer_name(), broken_item);
    let mut to_remove: Vec<ItemRef> = Vec::new();
    for it in candidates {
        if remaining <= 0 {
            break;
        }
        let is_drainable = it.borrow().template.class == ItemClass::Drainable;
        if is_drainable {
            let cur = it.borrow().get_current_uses().max(0);
            if cur <= remaining {
                remaining -= cur;
                it.borrow_mut().set_current_uses(0);
                to_remove.push(it);
            } else {
                it.borrow_mut().set_current_uses(cur - remaining);
                remaining = 0;
            }
        } else {
            remaining -= 1;
            to_remove.push(it);
        }
    }
    if !to_remove.is_empty() {
        let mut inv_mut = inv.borrow_mut();
        inv_mut
            .items
            .retain(|it| !to_remove.iter().any(|r| Rc::ptr_eq(it, r)));
    }
}

/// `FixingManager.getFixes(item)`.
pub fn get_fixes(sm: &ScriptManager, item: &ItemRef) -> Vec<Value> {
    let (short_type, full_type) = {
        let it = item.borrow();
        (it.get_type().to_owned(), it.get_full_type())
    };
    let mut out = Vec::new();
    for script in sm.collection(ScriptType::Fixing).get_all_scripts() {
        let matches = script
            .downcast::<Fixing>()
            .is_some_and(|f| {
                f.borrow()
                    .required_item()
                    .is_some_and(|req| req.iter().any(|r| r == &short_type || r == &full_type))
            });
        if matches {
            out.push(script_value(Some(&script)));
        }
    }
    out
}

/// `FixingManager.getChanceOfFail(brokenItem, chr, fixing, fixer)`.
pub fn get_chance_of_fail(
    thread: &mut Thread,
    broken_item: &ItemRef,
    chr: &Value,
    fixer: &Fixer,
) -> LuaResult<f64> {
    let mut result = 3.0f64;
    if let Some(skills) = fixer.fixer_skills() {
        for skill in skills {
            let level = crate::item_tooltip::chr::perk_level(thread, chr, skill.skill_name())?;
            if level < skill.skill_level() {
                result += ((skill.skill_level() - level) * 30) as f64;
            } else {
                result -= ((level - skill.skill_level()) * 5) as f64;
            }
        }
    }
    result += (broken_item.borrow().have_been_repaired * 2) as f64;
    if crate::item_tooltip::chr::has_trait(thread, chr, "Lucky")? {
        result -= 5.0;
    }
    if crate::item_tooltip::chr::has_trait(thread, chr, "Unlucky")? {
        result += 5.0;
    }
    Ok(result.clamp(0.0, 100.0))
}

/// `FixingManager.getCondRepaired(brokenItem, chr, fixing, fixer)`.
pub fn get_cond_repaired(
    thread: &mut Thread,
    broken_item: &ItemRef,
    chr: &Value,
    fixing: &Fixing,
    fixer: &Fixer,
) -> LuaResult<f64> {
    let mut factor = match fixing.fixers().iter().position(|f| f == fixer) {
        Some(0) => 50.0,
        Some(1) => 20.0,
        _ => 10.0,
    };
    let repaired = (broken_item.borrow().have_been_repaired + 1).max(1) as f64;
    factor *= 1.0 / repaired;
    if let Some(skills) = fixer.fixer_skills() {
        for skill in skills {
            let level = crate::item_tooltip::chr::perk_level(thread, chr, skill.skill_name())?;
            if level > skill.skill_level() {
                factor += ((level - skill.skill_level()) * 5).min(25) as f64;
            } else {
                factor -= ((skill.skill_level() - level) * 15) as f64;
            }
        }
    }
    factor *= fixing.condition_modifier() as f64;
    Ok(factor.clamp(0.0, 100.0))
}

/// Expose `zombie.inventory.FixingManager` and inventory methods on `Fixing`.
pub fn register(thread: &mut Thread, sm: &Rc<ScriptManager>) {
    let sm_fixes = Rc::clone(sm);
    let mgr = expose_class(thread, FIXING_MANAGER_CLASS, Some("java.lang.Object"));

    mgr.static_fn("getFixes", move |c| {
        let item = item_arg(c, 0)?;
        let fixes = get_fixes(&sm_fixes, &item);
        Ok(c.push(Value::Object(Rc::new(JavaList::new(fixes)))))
    });

    mgr.static_fn("getChanceOfFail", |c| {
        let broken = item_arg(c, 0)?;
        let chr = c.arg(1);
        let _fixing = fixing_arg(c, 2)?;
        let fixer = fixer_arg(c, 3)?;
        let pct = get_chance_of_fail(c.thread, &broken, &chr, &fixer)?;
        Ok(c.push(pct))
    });

    mgr.static_fn("getCondRepaired", |c| {
        let broken = item_arg(c, 0)?;
        let chr = c.arg(1);
        let fixing_h = fixing_arg(c, 2)?;
        let fixer = fixer_arg(c, 3)?;
        let (fixer_idx, cond_mod) = fixing_h
            .read(|f: &Fixing| {
                let idx = f.fixers().iter().position(|x| x == &fixer);
                (idx, f.condition_modifier())
            })
            .map_err(|e| LuaError::runtime(format!("Fixing: {e:?}")))?;
        let mut factor = match fixer_idx {
            Some(0) => 50.0,
            Some(1) => 20.0,
            _ => 10.0,
        };
        let repaired = (broken.borrow().have_been_repaired + 1).max(1) as f64;
        factor *= 1.0 / repaired;
        if let Some(skills) = fixer.fixer_skills() {
            for skill in skills {
                let level =
                    crate::item_tooltip::chr::perk_level(c.thread, &chr, skill.skill_name())?;
                if level > skill.skill_level() {
                    factor += ((level - skill.skill_level()) * 5).min(25) as f64;
                } else {
                    factor -= ((skill.skill_level() - level) * 15) as f64;
                }
            }
        }
        factor *= cond_mod as f64;
        Ok(c.push(factor.clamp(0.0, 100.0)))
    });

    mgr.static_fn("fixItem", |c| {
        let broken = item_arg(c, 0)?;
        let chr = c.arg(1);
        let fixing_h = fixing_arg(c, 2)?;
        let fixer = fixer_arg(c, 3)?;
        let (fixer_idx, cond_mod, global_item) = fixing_h
            .read(|f: &Fixing| {
                let idx = f.fixers().iter().position(|x| x == &fixer);
                (idx, f.condition_modifier(), f.global_item().cloned())
            })
            .map_err(|e| LuaError::runtime(format!("Fixing: {e:?}")))?;

        let fail_chance = get_chance_of_fail(c.thread, &broken, &chr, &fixer)?;
        let roll = FIXING_RAND.with(|r| r.borrow_mut().next(100));
        if (roll as f64) < fail_chance {
            let mut it = broken.borrow_mut();
            it.condition = (it.condition - 1).max(0);
        } else {
            let mut factor = match fixer_idx {
                Some(0) => 50.0,
                Some(1) => 20.0,
                _ => 10.0,
            };
            let repaired = (broken.borrow().have_been_repaired + 1).max(1) as f64;
            factor *= 1.0 / repaired;
            if let Some(skills) = fixer.fixer_skills() {
                for skill in skills {
                    let level =
                        crate::item_tooltip::chr::perk_level(c.thread, &chr, skill.skill_name())?;
                    if level > skill.skill_level() {
                        factor += ((level - skill.skill_level()) * 5).min(25) as f64;
                    } else {
                        factor -= ((skill.skill_level() - level) * 15) as f64;
                    }
                }
            }
            let pct = (factor * cond_mod as f64).clamp(0.0, 100.0);
            let mut it = broken.borrow_mut();
            let missing = (it.condition_max - it.condition).max(0);
            let cond_add = ((missing as f64) * (pct / 100.0)).round() as i32;
            it.condition = (it.condition + cond_add.max(1)).min(it.condition_max);
            it.have_been_repaired += 1;
        }

        use_fixer(&chr, &fixer, Some(&broken));
        if let Some(global) = global_item.as_ref() {
            use_fixer(&chr, global, Some(&broken));
        }

        Ok(c.push(item_value(&broken)))
    });

    mgr.static_fn("useFixer", |c| {
        let chr = c.arg(0);
        let fixer = fixer_arg(c, 1)?;
        let broken = opt_item_arg(c, 2);
        use_fixer(&chr, &fixer, broken.as_ref());
        Ok(0)
    });

    // Inventory query methods on `zombie.scripting.objects.Fixing`.
    let fix = expose_class(
        thread,
        "zombie.scripting.objects.Fixing",
        Some(crate::bind_scripts::BASE_SCRIPT_OBJECT_CLASS),
    );
    fix.method("usedInFixer", |c| {
        let fixing_h = fixing_arg(c, 0)?;
        let item = item_arg(c, 1)?;
        let used = fixing_h
            .read(|f: &Fixing| {
                f.fixers()
                    .iter()
                    .any(|fx| item_matches_name(&item, fx.fixer_name()))
            })
            .map_err(|e| LuaError::runtime(format!("Fixing: {e:?}")))?;
        Ok(c.push(used))
    });
    fix.method("haveGlobalItem", |c| {
        let fixing_h = fixing_arg(c, 0)?;
        let chr = c.arg(1);
        let global = fixing_h
            .read(|f: &Fixing| f.global_item().cloned())
            .map_err(|e| LuaError::runtime(format!("Fixing: {e:?}")))?;
        let found = global.and_then(|g| {
            let inv = chr_inventory(&chr)?;
            collect_matching_items(&inv, g.fixer_name(), None)
                .into_iter()
                .next()
        });
        Ok(c.push(found.map(|it| item_value(&it)).unwrap_or(Value::Nil)))
    });
    fix.method("haveThisFixer", |c| {
        let _fixing_h = fixing_arg(c, 0)?;
        let chr = c.arg(1);
        let fixer = fixer_arg(c, 2)?;
        let broken = opt_item_arg(c, 3);
        let have = count_uses(&chr, &fixer, broken.as_ref()) >= fixer.number_of_use();
        let found = if have {
            chr_inventory(&chr).and_then(|inv| {
                collect_matching_items(&inv, fixer.fixer_name(), broken.as_ref())
                    .into_iter()
                    .next()
            })
        } else {
            None
        };
        Ok(c.push(found.map(|it| item_value(&it)).unwrap_or(Value::Nil)))
    });
    fix.method("countUses", |c| {
        let _fixing_h = fixing_arg(c, 0)?;
        let chr = c.arg(1);
        let fixer = fixer_arg(c, 2)?;
        let broken = opt_item_arg(c, 3);
        Ok(c.push(count_uses(&chr, &fixer, broken.as_ref())))
    });
    fix.method("getRequiredFixerItems", |c| {
        let _fixing_h = fixing_arg(c, 0)?;
        let chr = c.arg(1);
        let fixer = fixer_arg(c, 2)?;
        let broken = opt_item_arg(c, 3);
        let items: Vec<Value> = chr_inventory(&chr)
            .map(|inv| {
                collect_matching_items(&inv, fixer.fixer_name(), broken.as_ref())
                    .into_iter()
                    .map(|it| item_value(&it))
                    .collect()
            })
            .unwrap_or_default();
        if let Value::Object(o) = c.arg(4) {
            if let Some(list) = o.as_any().downcast_ref::<JavaList>() {
                list.items.borrow_mut().extend(items);
                return Ok(c.push(c.arg(4)));
            }
        }
        Ok(c.push(Value::Object(Rc::new(JavaList::new(items)))))
    });
    fix.method("getRequiredItems", |c| {
        let fixing_h = fixing_arg(c, 0)?;
        let chr = c.arg(1);
        let fixer = fixer_arg(c, 2)?;
        let broken = opt_item_arg(c, 3);
        let global = fixing_h
            .read(|f: &Fixing| f.global_item().cloned())
            .map_err(|e| LuaError::runtime(format!("Fixing: {e:?}")))?;
        let mut items: Vec<Value> = Vec::new();
        if let Some(inv) = chr_inventory(&chr) {
            for it in collect_matching_items(&inv, fixer.fixer_name(), broken.as_ref()) {
                items.push(item_value(&it));
            }
            if let Some(g) = global {
                for it in collect_matching_items(&inv, g.fixer_name(), broken.as_ref()) {
                    items.push(item_value(&it));
                }
            }
        }
        Ok(c.push(Value::Object(Rc::new(JavaList::new(items)))))
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bind_character_sim::LuaGameCharacter;
    use pz_inventory::item::InventoryItem;
    use pz_inventory::template::{ItemClass, ItemTemplate};
    use pz_kahlua::stdlib::load_string;
    use pz_kahlua::TableRef;

    fn run(t: &mut Thread, code: &str) -> Vec<Value> {
        let f = load_string(code, Some("fixing_test.lua"), t.globals().clone()).unwrap();
        t.pcall(&f, &[]).unwrap()
    }

    #[test]
    fn fixing_manager_finds_fixes_computes_chances_and_repairs_item() {
        let sm = Rc::new(ScriptManager::new());
        sm.parse_script(
            pz_scripts::ScriptLoadMode::Init,
            "module Base { fixing FixAxe { Require = Base.Axe, Fixer = Base.DuctTape=2; Woodwork=2, ConditionModifier = 1.0, } }",
        )
        .unwrap();
        sm.get_module("Base")
            .unwrap()
            .bucket(ScriptType::Fixing)
            .load_scripts(&sm, pz_scripts::ScriptLoadMode::Init);

        let mut t = Thread::new(TableRef::new());
        pz_kahlua::stdlib::install(&mut t, None).unwrap();
        crate::bind_java_util::register(&mut t);
        crate::bind_items::register(&mut t);
        crate::bind_scripts::register(&mut t, sm.clone());
        register(&mut t, &sm);

        // Build broken axe (condition 2/10) + character carrying 3 duct tape items.
        let mut axe_tpl = ItemTemplate::new(ItemClass::Weapon, "Base", "Axe");
        axe_tpl.condition_max = 10;
        let axe = Rc::new(RefCell::new(InventoryItem::create(Rc::new(axe_tpl), 1)));
        axe.borrow_mut().condition = 2;

        let tape_tpl = Rc::new(ItemTemplate::new(ItemClass::Normal, "Base", "DuctTape"));
        let chr = Rc::new(RefCell::new(LuaGameCharacter::new(
            &sm,
            pz_characters::CharacterKind::Player,
        )));
        {
            let inv = chr.borrow().items.inventory.clone();
            let mut inv = inv.borrow_mut();
            inv.items.push(axe.clone());
            inv.items.push(Rc::new(RefCell::new(InventoryItem::create(tape_tpl.clone(), 2))));
            inv.items.push(Rc::new(RefCell::new(InventoryItem::create(tape_tpl.clone(), 3))));
            inv.items.push(Rc::new(RefCell::new(InventoryItem::create(tape_tpl, 4))));
        }

        // Wrap real CharacterObject with getPerkLevel/hasTrait on IsoGameCharacter.
        let chr_obj = crate::bind_character_sim::character_value(&chr);
        let chr_class = expose_class(&mut t, "zombie.characters.IsoGameCharacter", None);
        chr_class.method("getPerkLevel", |c| Ok(c.push(4)));
        chr_class.method("hasTrait", |c| Ok(c.push(false)));

        t.globals().set_str("axe", item_value(&axe));
        t.globals().set_str("chr", chr_obj);

        let res = run(
            &mut t,
            r#"
            local fixes = FixingManager.getFixes(axe)
            local f = fixes:get(0)
            local fixer = f:getFixers():get(0)
            local usesBefore = f:countUses(chr, fixer, axe)
            local failChance = FixingManager.getChanceOfFail(axe, chr, f, fixer)
            local condPct = FixingManager.getCondRepaired(axe, chr, f, fixer)
            FixingManager.fixItem(axe, chr, f, fixer)
            local usesAfter = f:countUses(chr, fixer, axe)
            return fixes:size(), usesBefore, failChance, condPct, usesAfter
            "#,
        );
        assert_eq!(res[0].as_num(), Some(1.0));
        assert_eq!(res[1].as_num(), Some(3.0));
        // Base 3 - (4 - 2)*5 = -7 -> clamped to 0.0 -> guaranteed repair!
        assert_eq!(res[2].as_num(), Some(0.0));
        // Base 50 / 1 + (4 - 2)*5 = 60.0%
        assert_eq!(res[3].as_num(), Some(60.0));
        // Consumed 2 of 3 duct tapes.
        assert_eq!(res[4].as_num(), Some(1.0));
        assert!(axe.borrow().condition > 2);
        assert_eq!(axe.borrow().have_been_repaired, 1);
    }
}
