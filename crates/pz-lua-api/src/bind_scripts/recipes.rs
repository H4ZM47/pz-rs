//! `Recipe` (with `Recipe.Source`, `Recipe.Result`, `Recipe.RequiredSkill`),
//! `EvolvedRecipe` and `ItemRecipe`, `Fixing` (with `Fixing.Fixer`,
//! `Fixing.FixerSkill`) and `UniqueRecipe`.
//!
//! Not exposed yet: members taking characters or inventory items
//! (`EvolvedRecipe.addItem`, `getItemsCanBeUse`, `getItemRecipe`,
//! `hasMinimumWater`, `isResultItem`, `isSpiceAdded`, `needToBeCooked`,
//! `isItemUsableInRecipe`; `Fixing.usedInFixer`, `haveGlobalItem`,
//! `haveThisFixer`, `countUses`, `getRequiredItems`, `getRequiredFixerItems`)
//! and `RequiredSkill.getPerk` (TODO(inventory, IsoGameCharacter,
//! PerkFactory)).

use super::{
    get, get_obj, method, opt_str, opt_string_list, set, string_list, void, IntoLua, ScriptHandle,
};
use crate::bind_registry::java_error;
use crate::expose::{expose_class, int_arg};
use pz_kahlua::{Thread, Value};
use pz_scripts::objects::recipes::{
    EvolvedRecipe, Fixer, FixerSkill, Fixing, ItemRecipe, Recipe, RecipeResult, RequiredSkill,
    Source, UniqueRecipe,
};
use pz_scripts::ScriptManager;
use std::rc::Rc;

pub const RECIPE_CLASS: &str = "zombie.scripting.objects.Recipe";
pub const SOURCE_CLASS: &str = "zombie.scripting.objects.Recipe$Source";
pub const RESULT_CLASS: &str = "zombie.scripting.objects.Recipe$Result";
pub const REQUIRED_SKILL_CLASS: &str = "zombie.scripting.objects.Recipe$RequiredSkill";
pub const EVOLVED_RECIPE_CLASS: &str = "zombie.scripting.objects.EvolvedRecipe";
pub const ITEM_RECIPE_CLASS: &str = "zombie.scripting.objects.ItemRecipe";
pub const FIXING_CLASS: &str = "zombie.scripting.objects.Fixing";
pub const FIXER_CLASS: &str = "zombie.scripting.objects.Fixing$Fixer";
pub const FIXER_SKILL_CLASS: &str = "zombie.scripting.objects.Fixing$FixerSkill";
pub const UNIQUE_RECIPE_CLASS: &str = "zombie.scripting.objects.UniqueRecipe";

fn list(values: Vec<Value>) -> Value {
    crate::bind_java_util::list_value(values)
}

fn source_value(h: &ScriptHandle, i: usize) -> Value {
    h.child_mut(
        SOURCE_CLASS,
        &format!("source{i}"),
        move |r: &Recipe| r.source.get(i),
        move |r: &mut Recipe| r.source.get_mut(i),
    )
}

fn item_recipe_value(h: &ScriptHandle, name: &str) -> Value {
    let (n, nm) = (name.to_string(), name.to_string());
    h.child_mut(
        ITEM_RECIPE_CLASS,
        &format!("item:{name}"),
        move |e: &EvolvedRecipe| e.items_list.get(&n),
        move |e: &mut EvolvedRecipe| e.items_list.get_mut(&nm),
    )
}

fn fixer_value(h: &ScriptHandle, i: Option<usize>) -> Value {
    match i {
        Some(i) => h.child(FIXER_CLASS, &format!("fixer{i}"), move |f: &Fixing| {
            f.fixers().get(i)
        }),
        None => h.child(FIXER_CLASS, "globalItem", |f: &Fixing| f.global_item()),
    }
}

pub fn register(thread: &mut Thread, _sm: &Rc<ScriptManager>) {
    let b = expose_class(thread, RECIPE_CLASS, Some(super::BASE_SCRIPT_OBJECT_CLASS));
    get(&b, "getName", |r: &Recipe| r.name().to_string());
    get(&b, "getOriginalname", |r: &Recipe| {
        r.originalname().map(str::to_string)
    });
    set(
        &b,
        "setOriginalname",
        |r: &mut Recipe, v: Option<String>| r.set_originalname(v),
    );
    method(&b, "getFullType", |_, h| {
        h.read(|r: &Recipe| r.full_type().ok_or_else(super::npe))?
            .map(IntoLua::into_lua)
    });
    get(&b, "getHeat", |r: &Recipe| r.heat());
    get(&b, "getNearItem", |r: &Recipe| {
        r.near_item().map(str::to_string)
    });
    set(&b, "setNearItem", |r: &mut Recipe, v: Option<String>| {
        r.set_near_item(v)
    });
    get(&b, "isRequiresWorkstation", |r: &Recipe| {
        r.is_requires_workstation()
    });
    get(&b, "getStationMultiplier", |r: &Recipe| {
        r.station_multiplier()
    });
    get(&b, "getNumberOfNeededItem", |r: &Recipe| {
        r.number_of_needed_item()
    });
    get(&b, "getRequiredSkillCount", |r: &Recipe| {
        r.required_skill_count()
    });
    get(&b, "getRequiredSkills", |r: &Recipe| {
        opt_string_list(r.required_skills())
    });
    void(&b, "clearRequiredSkills", |_, h| {
        h.write(|r: &mut Recipe| r.clear_required_skills())
    });
    void(&b, "addRequiredSkill", |c, h| {
        let perk = match c.arg(1) {
            Value::Object(o) => o.to_lua_string(),
            _ => opt_str(c, 1).ok_or_else(super::npe)?,
        };
        let level = int_arg(c, 2);
        h.write(|r: &mut Recipe| r.add_required_skill(&perk, level))
    });
    method(&b, "getRequiredSkill", |c, h| {
        let i = int_arg(c, 1);
        let n = h.read(|r: &Recipe| r.required_skill_count())?;
        if i < 0 || i as usize >= n {
            return Ok(Value::Nil);
        }
        let i = i as usize;
        Ok(h.child(
            REQUIRED_SKILL_CLASS,
            &format!("skill{i}"),
            move |r: &Recipe| r.skill_required.as_ref()?.get(i),
        ))
    });
    get_obj(&b, "getSource", |h, r: &Recipe| {
        list((0..r.source.len()).map(|i| source_value(h, i)).collect())
    });
    method(&b, "findSource", |c, h| {
        let t = opt_str(c, 1).unwrap_or_default();
        let i = h.read(|r: &Recipe| r.source.iter().position(|s| s.items().contains(&t)))?;
        Ok(i.map(|i| source_value(h, i)).unwrap_or_default())
    });
    get_obj(&b, "getResult", |h, r: &Recipe| match r.result() {
        Some(_) => h.child_mut(
            RESULT_CLASS,
            "result",
            |r: &Recipe| r.result.as_ref(),
            |r: &mut Recipe| r.result.as_mut(),
        ),
        None => Value::Nil,
    });
    get_obj(&b, "getResults", |h, r: &Recipe| {
        list(
            (0..r.results().len())
                .map(|i| {
                    h.child_mut(
                        RESULT_CLASS,
                        &format!("results{i}"),
                        move |r: &Recipe| r.results.get(i),
                        move |r: &mut Recipe| r.results.get_mut(i),
                    )
                })
                .collect(),
        )
    });
    void(&b, "DoSource", |c, h| {
        let s = opt_str(c, 1).ok_or_else(super::npe)?;
        h.write(|r: &mut Recipe| r.do_source(&s))?
            .map_err(java_error)
    });
    void(&b, "DoResult", |c, h| {
        let s = opt_str(c, 1).ok_or_else(super::npe)?;
        h.write(|r: &mut Recipe| r.do_result(&s))?
            .map_err(java_error)
    });

    let b = expose_class(thread, SOURCE_CLASS, None);
    get(&b, "isDestroy", |s: &Source| s.destroy);
    set(&b, "setDestroy", |s: &mut Source, v: bool| s.destroy = v);
    get(&b, "isKeep", |s: &Source| s.keep);
    set(&b, "setKeep", |s: &mut Source, v: bool| s.keep = v);
    get(&b, "getCount", |s: &Source| s.count);
    set(&b, "setCount", |s: &mut Source, v: f32| s.count = v);
    get(&b, "getUse", |s: &Source| s.use_);
    set(&b, "setUse", |s: &mut Source, v: f32| s.use_ = v);
    get(&b, "getItems", |s: &Source| string_list(s.items()));
    get(&b, "getOriginalItems", |s: &Source| {
        string_list(s.original_items())
    });
    method(&b, "getOnlyItem", |_, h| {
        h.read(|s: &Source| s.only_item().map(str::to_string).map_err(java_error))?
            .map(IntoLua::into_lua)
    });

    let b = expose_class(thread, RESULT_CLASS, None);
    get(&b, "getType", |r: &RecipeResult| r.type_.clone());
    set(&b, "setType", |r: &mut RecipeResult, v: Option<String>| {
        r.type_ = v
    });
    get(&b, "getCount", |r: &RecipeResult| r.count);
    set(&b, "setCount", |r: &mut RecipeResult, v: i32| r.count = v);
    get(&b, "getDrainableCount", |r: &RecipeResult| {
        r.drainable_count
    });
    set(&b, "setDrainableCount", |r: &mut RecipeResult, v: i32| {
        r.drainable_count = v
    });
    get(&b, "getFullType", |r: &RecipeResult| r.full_type());

    let b = expose_class(thread, REQUIRED_SKILL_CLASS, None);
    get(&b, "getLevel", |s: &RequiredSkill| s.level());

    let b = expose_class(
        thread,
        EVOLVED_RECIPE_CLASS,
        Some(super::BASE_SCRIPT_OBJECT_CLASS),
    );
    get(&b, "getName", |e: &EvolvedRecipe| {
        e.display_name().map(str::to_string)
    });
    get(&b, "getOriginalname", |e: &EvolvedRecipe| {
        e.originalname().map(str::to_string)
    });
    get(&b, "getUntranslatedName", |e: &EvolvedRecipe| {
        e.untranslated_name().to_string()
    });
    get(&b, "isAllowFrozenItem", |e: &EvolvedRecipe| {
        e.is_allow_frozen_item()
    });
    set(
        &b,
        "setAllowFrozenItem",
        |e: &mut EvolvedRecipe, v: bool| e.set_allow_frozen_item(v),
    );
    get(&b, "isHidden", |e: &EvolvedRecipe| e.is_hidden());
    set(&b, "setIsHidden", |e: &mut EvolvedRecipe, v: bool| {
        e.set_is_hidden(v)
    });
    get(&b, "isCookable", |e: &EvolvedRecipe| e.is_cookable());
    get(&b, "getMinimumWater", |e: &EvolvedRecipe| e.minimum_water());
    get(&b, "getBaseItem", |e: &EvolvedRecipe| {
        e.base_item().map(str::to_string)
    });
    get(&b, "getFullResultItem", |e: &EvolvedRecipe| {
        e.full_result_item().map(str::to_string)
    });
    get(&b, "getMaxItems", |e: &EvolvedRecipe| e.max_items());
    get(&b, "getAddIngredientSound", |e: &EvolvedRecipe| {
        e.add_ingredient_sound().map(str::to_string)
    });
    method(&b, "getResultItem", |_, h| {
        h.read(|e: &EvolvedRecipe| e.result_item().map(str::to_string).map_err(java_error))?
            .map(IntoLua::into_lua)
    });
    get_obj(&b, "getPossibleItems", |h, e: &EvolvedRecipe| {
        list(
            e.possible_items()
                .iter()
                .map(|r| item_recipe_value(h, r.name()))
                .collect(),
        )
    });
    // The live `HashMap<String, ItemRecipe>`, in its own (Java) order.
    method(&b, "getItemsList", |_, h| {
        let h = h.clone();
        let read = move || {
            h.read(|e: &EvolvedRecipe| {
                e.items_list
                    .iter()
                    .map(|(k, _)| (Value::from(k), item_recipe_value(&h, k)))
                    .collect()
            })
            .unwrap_or_default()
        };
        Ok(Value::Object(Rc::new(
            crate::bind_java_util::JavaMap::live(read, None),
        )))
    });

    let b = expose_class(thread, ITEM_RECIPE_CLASS, None);
    get(&b, "getName", |r: &ItemRecipe| r.name().to_string());
    get(&b, "getUse", |r: &ItemRecipe| r.use_());
    get(&b, "getModule", |r: &ItemRecipe| r.module().to_string());
    get(&b, "getFullType", |r: &ItemRecipe| r.full_type());
    void(&b, "setModule", |c, h| {
        let m = opt_str(c, 1).unwrap_or_else(|| "null".into());
        h.write(|r: &mut ItemRecipe| r.set_module(&m))
    });

    let b = expose_class(thread, FIXING_CLASS, Some(super::BASE_SCRIPT_OBJECT_CLASS));
    get(&b, "getName", |f: &Fixing| f.name().map(str::to_string));
    set(&b, "setName", |f: &mut Fixing, v: Option<String>| {
        f.set_name(v)
    });
    get(&b, "getRequiredItem", |f: &Fixing| {
        opt_string_list(f.required_item())
    });
    void(&b, "addRequiredItem", |c, h| {
        let r = opt_str(c, 1).unwrap_or_else(|| "null".into());
        h.write(|f: &mut Fixing| f.add_required_item(&r))
    });
    get(&b, "getConditionModifier", |f: &Fixing| {
        f.condition_modifier()
    });
    set(&b, "setConditionModifier", |f: &mut Fixing, v: f32| {
        f.set_condition_modifier(v)
    });
    get_obj(&b, "getGlobalItem", |h, f: &Fixing| match f.global_item() {
        Some(_) => fixer_value(h, None),
        None => Value::Nil,
    });
    void(&b, "setGlobalItem", |c, h| {
        let fixer = match super::handle_arg(c, 1)? {
            Some(o) => Some(o.read(|x: &Fixer| x.clone())?),
            None => None,
        };
        h.write(|f: &mut Fixing| f.set_global_item(fixer))
    });
    get_obj(&b, "getFixers", |h, f: &Fixing| {
        list(
            (0..f.fixers().len())
                .map(|i| fixer_value(h, Some(i)))
                .collect(),
        )
    });

    let b = expose_class(thread, FIXER_CLASS, None);
    get(&b, "getFixerName", |f: &Fixer| f.fixer_name().to_string());
    get(&b, "getNumberOfUse", |f: &Fixer| f.number_of_use());
    get_obj(&b, "getFixerSkills", |h, f: &Fixer| {
        match f.fixer_skills() {
            Some(skills) => list(
                (0..skills.len())
                    .map(|i| {
                        h.child(FIXER_SKILL_CLASS, &format!("skill{i}"), move |f: &Fixer| {
                            f.fixer_skills()?.get(i)
                        })
                    })
                    .collect(),
            ),
            None => Value::Nil,
        }
    });

    let b = expose_class(thread, FIXER_SKILL_CLASS, None);
    get(&b, "getSkillName", |s: &FixerSkill| {
        s.skill_name().to_string()
    });
    get(&b, "getSkillLevel", |s: &FixerSkill| s.skill_level());

    let b = expose_class(
        thread,
        UNIQUE_RECIPE_CLASS,
        Some(super::BASE_SCRIPT_OBJECT_CLASS),
    );
    get(&b, "getName", |r: &UniqueRecipe| {
        r.name().map(str::to_string)
    });
    set(&b, "setName", |r: &mut UniqueRecipe, v: Option<String>| {
        r.set_name(v)
    });
    get(&b, "getBaseRecipe", |r: &UniqueRecipe| {
        r.base_recipe().map(str::to_string)
    });
    set(
        &b,
        "setBaseRecipe",
        |r: &mut UniqueRecipe, v: Option<String>| r.set_base_recipe(v),
    );
    get(&b, "getItems", |r: &UniqueRecipe| string_list(r.items()));
    get(&b, "getHungerBonus", |r: &UniqueRecipe| r.hunger_bonus());
    set(&b, "setHungerBonus", |r: &mut UniqueRecipe, v: i32| {
        r.set_hunger_bonus(v)
    });
    get(&b, "getHapinessBonus", |r: &UniqueRecipe| {
        r.hapiness_bonus()
    });
    set(&b, "setHapinessBonus", |r: &mut UniqueRecipe, v: i32| {
        r.set_hapiness_bonus(v)
    });
    get(&b, "getBoredomBonus", |r: &UniqueRecipe| r.boredom_bonus());
    set(&b, "setBoredomBonus", |r: &mut UniqueRecipe, v: i32| {
        r.set_boredom_bonus(v)
    });
}
