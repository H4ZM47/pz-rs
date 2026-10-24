//! `CraftRecipe` (a script, or embedded in an entity's
//! `CraftRecipeComponentScript`) with its `InputScript`/`OutputScript` lines,
//! `CraftRecipe.RequiredSkill` and `CraftRecipe.XpAward`, and the crafting
//! enums they return.
//!
//! Not exposed yet: members taking characters, players or inventory items
//! (`canUseItem`, `canOutputItem`, `OnTestItem`, `hasRecipeAtHand`,
//! `isFavourite`, `addXP`, `hasPlayerLearned`, `canResearch`,
//! `checkMetaRecipe`, `checkAutoLearn*Skills`, `characterHasRequiredSkills`,
//! `getHighestRelevantSkill*`, `*BenefitFromRecipeAtHand`,
//! `generateDebugText`, `getFavouriteModDataString`, `getTime(character)`,
//! the input item tests, `isFluidMatch`, `isEnergyMatch`; TODO(inventory,
//! IsoGameCharacter)), perks (`getPerk`, `involvesSkill`,
//! `getHighestPerkRequirement`; TODO(PerkFactory)), `getIconTexture`
//! (TODO(Texture)), fluids and energies (`getPossibleInputFluids`,
//! `getFluid`, ...; TODO(Fluid, Energy)), `getOverlayMapper`,
//! `getOutputMapper`, `getLuaCallString`/`hasLuaCall` (enum not exposed),
//! `getRecipeGroup` (TODO(CraftRecipeGroup)), `setTags`, `setProp1`/`setProp2`, `getTagBits`/`getModTags`, and a mod's
//! name in `getModName` (TODO(ChooseGameInfo)).

use super::{
    get, handle_arg, method, opt_str, script_list, script_value, set, string_list, void, IntoLua, ScriptHandle,
    CRAFT_RECIPE_CLASS,
};
use crate::bind_registry::{key_value, ITEM_TAG_CLASS};
use crate::expose::{bool_arg, enum_value, expose_class, expose_enum, int_arg};
use pz_kahlua::{LuaResult, NativeCall, Thread, Value};
use pz_scripts::objects::entity::components::crafting::{
    CraftRecipe, FluidMatchMode, InputFlag, InputScript, ItemApplyMode, OutputFlag, OutputScript, RequiredSkill,
    XpAward,
};
use pz_scripts::objects::entity::components::resources::ResourceType;
use pz_scripts::objects::entity::ComponentType;
use pz_scripts::objects::items::Item;
use pz_scripts::ScriptManager;
use std::rc::Rc;

pub const INPUT_SCRIPT_CLASS: &str = "zombie.scripting.entity.components.crafting.InputScript";
pub const OUTPUT_SCRIPT_CLASS: &str = "zombie.scripting.entity.components.crafting.OutputScript";
pub const IO_SCRIPT_CLASS: &str = "zombie.scripting.entity.components.crafting.CraftRecipe$IOScript";
pub const REQUIRED_SKILL_CLASS: &str = "zombie.scripting.entity.components.crafting.CraftRecipe$RequiredSkill";
pub const XP_AWARD_CLASS: &str = "zombie.scripting.entity.components.crafting.CraftRecipe$XpAward";
pub const INPUT_FLAG_CLASS: &str = "zombie.entity.components.crafting.InputFlag";
pub const OUTPUT_FLAG_CLASS: &str = "zombie.entity.components.crafting.OutputFlag";
pub const ITEM_APPLY_MODE_CLASS: &str = "zombie.entity.components.crafting.ItemApplyMode";
pub const FLUID_MATCH_MODE_CLASS: &str = "zombie.entity.components.crafting.FluidMatchMode";
pub const RESOURCE_TYPE_CLASS: &str = "zombie.entity.components.resources.ResourceType";

fn list(values: Vec<Value>) -> Value {
    crate::bind_java_util::list_value(values)
}

/// Line `i` of a recipe (an `InputScript` or `OutputScript`).
fn io_value(recipe: &ScriptHandle, i: usize) -> LuaResult<Value> {
    let input = recipe.read(|r: &CraftRecipe| r.io_lines().get(i).map(|io| io.as_input().is_some()))?;
    Ok(match input {
        Some(true) => {
            recipe.child(INPUT_SCRIPT_CLASS, &format!("io{i}"), move |r: &CraftRecipe| r.io_lines().get(i)?.as_input())
        }
        Some(false) => recipe
            .child(OUTPUT_SCRIPT_CLASS, &format!("io{i}"), move |r: &CraftRecipe| r.io_lines().get(i)?.as_output()),
        None => Value::Nil,
    })
}

fn opt_io(recipe: &ScriptHandle, i: Option<usize>) -> LuaResult<Value> {
    match i {
        Some(i) => io_value(recipe, i),
        None => Ok(Value::Nil),
    }
}

/// The index of an input/output handle among its recipe's lines.
fn line_index(h: &ScriptHandle) -> LuaResult<Option<(ScriptHandle, usize)>> {
    let Some(recipe) = h.parent().cloned() else { return Ok(None) };
    let ptr = h.target_ptr()?;
    let i = recipe.read(|r: &CraftRecipe| {
        r.io_lines().iter().position(|io| {
            let p = match (io.as_input(), io.as_output()) {
                (Some(i), _) => i as *const InputScript as *const () as usize,
                (_, Some(o)) => o as *const OutputScript as *const () as usize,
                _ => 0,
            };
            p == ptr
        })
    })?;
    Ok(i.map(|i| (recipe, i)))
}

fn skill_list(
    h: &ScriptHandle,
    field: &'static str,
    f: fn(&CraftRecipe) -> Option<&Vec<RequiredSkill>>,
) -> LuaResult<Value> {
    let n = h.read(|r: &CraftRecipe| f(r).map_or(0, Vec::len))?;
    Ok(list((0..n).map(|i| skill_value(h, field, f, i)).collect()))
}

fn skill_value(
    h: &ScriptHandle,
    field: &'static str,
    f: fn(&CraftRecipe) -> Option<&Vec<RequiredSkill>>,
    i: usize,
) -> Value {
    h.child(REQUIRED_SKILL_CLASS, &format!("{field}{i}"), move |r: &CraftRecipe| f(r)?.get(i))
}

fn skill_at(
    c: &NativeCall<'_>,
    h: &ScriptHandle,
    field: &'static str,
    f: fn(&CraftRecipe) -> Option<&Vec<RequiredSkill>>,
) -> LuaResult<Value> {
    let i = int_arg(c, 1);
    let n = h.read(|r: &CraftRecipe| f(r).map_or(0, Vec::len))?;
    match usize::try_from(i).ok().filter(|i| *i < n) {
        Some(i) => Ok(skill_value(h, field, f, i)),
        None => Ok(Value::Nil),
    }
}

/// An `Item` argument's full type, or a string argument.
fn item_name_arg(c: &NativeCall<'_>, i: usize) -> LuaResult<Option<String>> {
    if let Value::Object(_) = c.arg(i) {
        if let Some(h) = handle_arg(c, i)? {
            return h.read(|it: &Item| it.full_name()).map(Some);
        }
    }
    Ok(opt_str(c, i))
}

fn enum_once(thread: &mut Thread, class: &'static str, names: Vec<&'static str>) {
    if thread.class_metatables.get_str(class).is_nil() {
        expose_enum(thread, class, &names);
    }
}

pub fn register(thread: &mut Thread, sm: &Rc<ScriptManager>) {
    enum_once(thread, INPUT_FLAG_CLASS, InputFlag::ALL.iter().map(|v| v.name()).collect());
    enum_once(thread, OUTPUT_FLAG_CLASS, OutputFlag::ALL.iter().map(|v| v.name()).collect());
    enum_once(thread, ITEM_APPLY_MODE_CLASS, ItemApplyMode::ALL.iter().map(|v| v.name()).collect());
    enum_once(thread, FLUID_MATCH_MODE_CLASS, FluidMatchMode::ALL.iter().map(|v| v.name()).collect());
    enum_once(thread, RESOURCE_TYPE_CLASS, ResourceType::ALL.iter().map(|v| v.name()).collect());
    enum_once(thread, super::COMPONENT_TYPE_CLASS, ComponentType::ALL.iter().map(|v| v.name()).collect());

    let b = expose_class(thread, CRAFT_RECIPE_CLASS, Some(super::BASE_SCRIPT_OBJECT_CLASS));
    get(&b, "getName", |r: &CraftRecipe| r.name().map(str::to_string));
    get(&b, "getTranslationName", |r: &CraftRecipe| r.translation_name().map(str::to_string));
    set(&b, "overrideTranslationName", |r: &mut CraftRecipe, v: Option<String>| r.override_translation_name(v));
    get(&b, "getIconName", |r: &CraftRecipe| r.icon_name().map(str::to_string));
    get(&b, "getModID", |r: &CraftRecipe| r.mod_id().map(str::to_string));
    get(&b, "getModName", |r: &CraftRecipe| if r.is_vanilla() { "Project Zomboid" } else { "<unknown_mod>" });
    get(&b, "isVanilla", |r: &CraftRecipe| r.is_vanilla());
    get(&b, "getExistsAsVanilla", |r: &CraftRecipe| r.exists_as_vanilla());
    get(&b, "needToBeLearn", |r: &CraftRecipe| r.need_to_be_learn());
    get(&b, "getMetaRecipe", |r: &CraftRecipe| r.meta_recipe().map(str::to_string));
    method(&b, "hasTag", |c, h| {
        let t = opt_str(c, 1).unwrap_or_default();
        h.read(|r: &CraftRecipe| r.has_tag(&t).into_lua())
    });
    get(&b, "getTags", |r: &CraftRecipe| string_list(r.tags()));
    get(&b, "getCategory", |r: &CraftRecipe| r.category().to_string());
    get(&b, "getTooltip", |r: &CraftRecipe| r.tooltip().map(str::to_string));
    get(&b, "isCanWalk", |r: &CraftRecipe| r.is_can_walk());
    get(&b, "hasOnTickInputs", |r: &CraftRecipe| r.has_on_tick_inputs());
    get(&b, "hasOnTickOutputs", |r: &CraftRecipe| r.has_on_tick_outputs());
    get(&b, "canBeDoneInDark", |r: &CraftRecipe| r.can_be_done_in_dark());
    get(&b, "isSmithing", |r: &CraftRecipe| r.is_smithing());
    get(&b, "isResearchAll", |r: &CraftRecipe| r.is_research_all());
    get(&b, "isShapeless", |_: &CraftRecipe| true);
    get(&b, "isConsumeOnFinish", |_: &CraftRecipe| false);
    get(&b, "isRequiresPlayer", |_: &CraftRecipe| false);
    get(&b, "isUsesTools", |r: &CraftRecipe| r.is_uses_tools());
    get(&b, "getAnimation", |r: &CraftRecipe| r.animation().map(str::to_string));
    set(&b, "setAnimation", |r: &mut CraftRecipe, v: Option<String>| r.set_animation(v));
    get(&b, "isAllowBatchCraft", |r: &CraftRecipe| r.is_allow_batch_craft());
    get(&b, "isInHandCraftCraft", |r: &CraftRecipe| r.is_in_hand_craft());
    get(&b, "isAutoRotate", |r: &CraftRecipe| r.is_auto_rotate());
    get(&b, "getOnAddToMenu", |r: &CraftRecipe| r.on_add_to_menu().map(str::to_string));
    get(&b, "isBuildableRecipe", |r: &CraftRecipe| r.is_buildable_recipe());
    get(&b, "requiresSpecificWorkstation", |r: &CraftRecipe| r.requires_specific_workstation());
    get(&b, "isCanBeDoneFromFloor", |r: &CraftRecipe| r.is_can_be_done_from_floor());
    get(&b, "isAnySurfaceCraft", |r: &CraftRecipe| r.is_any_surface_craft());
    get(&b, "cannotBeResearched", |r: &CraftRecipe| r.cannot_be_researched());
    method(&b, "canBeResearched", |_, h| h.write(|r: &mut CraftRecipe| r.can_be_researched().into_lua()));
    method(&b, "canAlwaysBeResearched", |_, h| h.write(|r: &mut CraftRecipe| r.can_always_be_researched().into_lua()));
    method(&b, "getResearchSkillLevel", |_, h| h.write(|r: &mut CraftRecipe| r.get_research_skill_level().into_lua()));
    set(&b, "setResearchSkillLevel", |r: &mut CraftRecipe, v: i32| r.set_research_skill_level(v));
    method(&b, "normalizeSkillLevel", |c, _| Ok(CraftRecipe::normalize_skill_level(int_arg(c, 1)).into_lua()));
    method(&b, "getTime", |c, h| {
        if !c.arg(1).is_nil() {
            return Err(pz_kahlua::LuaError::runtime("TODO(IsoGameCharacter): getTime(character)"));
        }
        h.read(|r: &CraftRecipe| r.time().into_lua())
    });
    get(&b, "getTimedActionScript", |r: &CraftRecipe| script_value(r.timed_action_script()));
    method(&b, "getHighestSkillRequirement", |c, h| {
        let include = c.nargs() > 1 && bool_arg(c, 1);
        h.read(|r: &CraftRecipe| r.highest_skill_requirement(include).into_lua())
    });
    get(&b, "getRequiredSkillCount", |r: &CraftRecipe| r.required_skill_count());
    get(&b, "getRequiredSkills", |r: &CraftRecipe| string_list(r.required_skills()));
    method(&b, "getRequiredSkill", |c, h| skill_at(c, h, "skill", |r| r.skill_required.as_ref()));
    get(&b, "getAutoLearnAnySkillCount", |r: &CraftRecipe| r.auto_learn_any.as_ref().map_or(0, Vec::len));
    get(&b, "getAutoLearnAllSkillCount", |r: &CraftRecipe| r.auto_learn_all.as_ref().map_or(0, Vec::len));
    method(&b, "getAutoLearnAnySkills", |_, h| skill_list(h, "autoLearnAny", |r| r.auto_learn_any.as_ref()));
    method(&b, "getAutoLearnAllSkills", |_, h| skill_list(h, "autoLearnAll", |r| r.auto_learn_all.as_ref()));
    method(&b, "getAutoLearnAnySkill", |c, h| skill_at(c, h, "autoLearnAny", |r| r.auto_learn_any.as_ref()));
    method(&b, "getAutoLearnAllSkill", |c, h| skill_at(c, h, "autoLearnAll", |r| r.auto_learn_all.as_ref()));
    void(&b, "addRequiredSkill", |c, h| {
        let perk = match c.arg(1) {
            Value::Object(o) => o.to_lua_string(),
            _ => opt_str(c, 1).ok_or_else(super::npe)?,
        };
        let level = int_arg(c, 2);
        h.write(|r: &mut CraftRecipe| r.add_required_skill(&perk, level))
    });
    void(&b, "clearRequiredSkills", |_, h| h.write(|r: &mut CraftRecipe| r.clear_required_skills()));
    get(&b, "getXPAwardCount", |r: &CraftRecipe| r.xp_award_count());
    method(&b, "getXPAward", |c, h| {
        let i = int_arg(c, 1);
        let n = h.read(|r: &CraftRecipe| r.xp_award_count())?;
        Ok(match usize::try_from(i).ok().filter(|i| *i < n) {
            Some(i) => h.child(XP_AWARD_CLASS, &format!("xp{i}"), move |r: &CraftRecipe| r.xp_award.as_ref()?.get(i)),
            None => Value::Nil,
        })
    });
    get(&b, "getInputCount", |r: &CraftRecipe| r.input_count());
    get(&b, "getOutputCount", |r: &CraftRecipe| r.output_count());
    method(&b, "getInputs", |_, h| {
        let idx = h.read(|r: &CraftRecipe| r.input_indices().to_vec())?;
        Ok(list(idx.into_iter().map(|i| io_value(h, i)).collect::<LuaResult<_>>()?))
    });
    method(&b, "getOutputs", |_, h| {
        let idx = h.read(|r: &CraftRecipe| r.output_indices().to_vec())?;
        Ok(list(idx.into_iter().map(|i| io_value(h, i)).collect::<LuaResult<_>>()?))
    });
    method(&b, "getIoLines", |_, h| {
        let n = h.read(|r: &CraftRecipe| r.io_lines().len())?;
        Ok(list((0..n).map(|i| io_value(h, i)).collect::<LuaResult<_>>()?))
    });
    method(&b, "getIOForIndex", |c, h| {
        let i = int_arg(c, 1);
        opt_io(h, usize::try_from(i).ok())
    });
    method(&b, "getIndexForIO", |c, _| {
        let Some(io) = handle_arg(c, 1)? else { return Ok((-1).into_lua()) };
        Ok(line_index(&io)?.map_or(-1, |(_, i)| i as i32).into_lua())
    });
    method(&b, "containsIO", |c, h| {
        let Some(io) = handle_arg(c, 1)? else { return Ok(false.into_lua()) };
        Ok(line_index(&io)?.is_some_and(|(r, _)| r.root.ptr_eq(&h.root)).into_lua())
    });
    method(&b, "getToolLeft", |_, h| opt_io(h, h.read(|r: &CraftRecipe| r.tool_left_index())?));
    method(&b, "getToolRight", |_, h| opt_io(h, h.read(|r: &CraftRecipe| r.tool_right_index())?));
    method(&b, "getToolBoth", |_, _| Ok(Value::Nil));
    method(&b, "getProp1", |_, h| opt_io(h, h.read(|r: &CraftRecipe| r.prop1_index())?));
    method(&b, "getProp2", |_, h| opt_io(h, h.read(|r: &CraftRecipe| r.prop2_index())?));

    let b = expose_class(thread, REQUIRED_SKILL_CLASS, None);
    get(&b, "getLevel", |s: &RequiredSkill| s.level());
    let b = expose_class(thread, XP_AWARD_CLASS, None);
    get(&b, "getAmount", |x: &XpAward| x.amount());

    register_input(thread, sm);
    register_output(thread);
}

/// `InputScript` boolean getters that test one flag.
const INPUT_FLAG_GETTERS: &[(&str, &str)] = &[
    ("isExclusive", "IsExclusive"),
    ("isItemCount", "ItemCount"),
    ("isToolLeft", "ToolLeft"),
    ("isToolRight", "ToolRight"),
    ("isWorn", "IsWorn"),
    ("isNotWorn", "IsNotWorn"),
    ("isFull", "IsFull"),
    ("isEmpty", "IsEmpty"),
    ("notFull", "NotFull"),
    ("notEmpty", "NotEmpty"),
    ("isDamaged", "IsDamaged"),
    ("isUndamaged", "IsUndamaged"),
    ("allowFrozenItem", "AllowFrozenItem"),
    ("allowRottenItem", "AllowRottenItem"),
    ("allowDestroyedItem", "AllowDestroyedItem"),
    ("isEmptyContainer", "IsEmptyContainer"),
    ("isWholeFoodItem", "IsWholeFoodItem"),
    ("isUncookedFoodItem", "IsUncookedFoodItem"),
    ("isCookedFoodItem", "IsCookedFoodItem"),
    ("isHeadPart", "IsHeadPart"),
    ("isSharpenable", "IsSharpenable"),
    ("dontPutBack", "DontPutBack"),
    ("inheritColor", "InheritColor"),
    ("inheritCondition", "InheritCondition"),
    ("inheritHeadCondition", "InheritHeadCondition"),
    ("inheritSharpness", "InheritSharpness"),
    ("inheritUses", "InheritUses"),
    ("isNotDull", "IsNotDull"),
    ("mayDegrade", "MayDegrade"),
    ("mayDegradeLight", "MayDegradeLight"),
    ("mayDegradeVeryLight", "MayDegradeVeryLight"),
    ("mayDegradeHeavy", "MayDegradeHeavy"),
    ("sharpnessCheck", "SharpnessCheck"),
];

fn flag_arg<F: Copy>(c: &NativeCall<'_>, all: &[F], name: fn(F) -> &'static str) -> Option<F> {
    match c.arg(1) {
        Value::Object(o) => {
            let n = o.to_lua_string();
            all.iter().copied().find(|f| name(*f) == n)
        }
        _ => None,
    }
}

/// `hasPossibleFrozenFoodInputItems`: a possible input is food that can freeze.
fn frozen_food(sm: &ScriptManager, i: &InputScript) -> bool {
    i.possible_input_items().iter().any(|s| {
        s.downcast::<Item>()
            .and_then(|it| it.try_borrow().ok().map(|it| it.is_item_type(sm, "FOOD") && !it.cant_be_frozen))
            .unwrap_or(false)
    })
}

fn register_input(thread: &mut Thread, sm: &Rc<ScriptManager>) {
    let b = expose_class(thread, INPUT_SCRIPT_CLASS, Some(IO_SCRIPT_CLASS));
    for &(name, flag) in INPUT_FLAG_GETTERS {
        let flag = InputFlag::value_of(flag).expect("InputFlag constant");
        get(&b, name, move |i: &InputScript| i.has_flag(flag));
    }
    method(&b, "hasFlag", |c, h| {
        let f = flag_arg(c, InputFlag::ALL, InputFlag::name);
        h.read(|i: &InputScript| f.is_some_and(|f| i.has_flag(f)).into_lua())
    });
    get(&b, "isDestroy", |i: &InputScript| i.is_destroy());
    get(&b, "isKeep", |i: &InputScript| i.is_keep());
    get(&b, "isTool", |i: &InputScript| i.is_tool());
    get(&b, "isProp1", |i: &InputScript| i.is_prop1());
    get(&b, "isProp2", |i: &InputScript| i.is_prop2());
    get(&b, "isApplyOnTick", |i: &InputScript| i.is_apply_on_tick());
    get(&b, "isAcceptsAnyItem", |i: &InputScript| i.is_accepts_any_item());
    get(&b, "isAcceptsAnyFluid", |i: &InputScript| i.is_accepts_any_fluid());
    get(&b, "isAcceptsAnyEnergy", |i: &InputScript| i.is_accepts_any_energy());
    get(&b, "isHandcraftOnly", |i: &InputScript| i.is_handcraft_only());
    get(&b, "isAutomationOnly", |i: &InputScript| i.is_automation_only());
    get(&b, "allowFavorites", |i: &InputScript| i.allow_favorites());
    get(&b, "isReplace", |_: &InputScript| false);
    get(&b, "isFluidExact", |i: &InputScript| i.is_fluid_exact());
    get(&b, "isFluidPrimary", |i: &InputScript| i.is_fluid_primary());
    get(&b, "isFluidMixture", |i: &InputScript| i.is_fluid_mixture());
    get(&b, "isFluidAnything", |i: &InputScript| i.is_fluid_anything());
    get(&b, "isVariableAmount", |i: &InputScript| i.is_variable_amount());
    get(&b, "getOriginalLine", |i: &InputScript| i.original_line().to_string());
    get(&b, "getShapedIndex", |i: &InputScript| i.shaped_index());
    get(&b, "getResourceType", |i: &InputScript| enum_value(RESOURCE_TYPE_CLASS, i.resource_type().name()));
    get(&b, "getItemApplyMode", |i: &InputScript| enum_value(ITEM_APPLY_MODE_CLASS, i.item_apply_mode().name()));
    get(&b, "getFluidMatchMode", |i: &InputScript| enum_value(FLUID_MATCH_MODE_CLASS, i.fluid_match_mode().name()));
    get(&b, "getItems", |i: &InputScript| string_list(i.loaded_items()));
    get(&b, "getItemTags", |i: &InputScript| {
        list(i.item_tags().iter().map(|k| key_value(ITEM_TAG_CLASS, k.as_ref())).collect())
    });
    get(&b, "getPossibleInputItems", |i: &InputScript| script_list(i.possible_input_items()));
    let s = sm.clone();
    method(&b, "hasPossibleFrozenFoodInputItems", move |_, h| h.read(|i: &InputScript| frozen_food(&s, i).into_lua()));
    let s = sm.clone();
    method(&b, "dontAllowFrozenItem", move |_, h| {
        h.read(|i: &InputScript| {
            (i.resource_type() == ResourceType::Item && frozen_food(&s, i) && !i.has_flag(InputFlag::AllowFrozenItem))
                .into_lua()
        })
    });
    get(&b, "hasCreateToItem", |i: &InputScript| i.has_create_to_item());
    get(&b, "hasConsumeFromItem", |i: &InputScript| i.has_consume_from_item());
    get(&b, "hasParentScript", |i: &InputScript| i.has_parent_script());
    method(&b, "getCreateToItemScript", |_, h| sibling(h, |i: &InputScript| i.create_to_item_script()));
    method(&b, "getConsumeFromItemScript", |_, h| sibling(h, |i: &InputScript| i.consume_from_item_script()));
    method(&b, "getParentScript", |_, h| sibling(h, |i: &InputScript| i.parent_script()));
    method(&b, "getReplaceOutputScript", |_, _| Ok(Value::Nil));
    method(&b, "getParentRecipe", |_, h| Ok(parent_value(h)));
    method(&b, "getRecipeLineIndex", |_, h| Ok(line_index(h)?.map_or(-1, |(_, i)| i as i32).into_lua()));
    method(&b, "containsItem", |c, h| {
        let Some(item) = handle_arg(c, 1)? else { return Ok(false.into_lua()) };
        h.read(|i: &InputScript| i.possible_input_items().iter().any(|s| s.ptr_eq(&item.root)).into_lua())
    });
    amounts(&b, "getAmount", |i, a| match a {
        Amount::None => i.amount(),
        Amount::Index(n) => i.amount_at(n),
        Amount::Item(s) => i.amount_for(&s),
    });
    amounts(&b, "getIntAmount", |i, a| match a {
        Amount::None => i.int_amount() as f32,
        Amount::Index(n) => i.amount_at(n) as i32 as f32,
        Amount::Item(s) => i.amount_for(&s) as i32 as f32,
    });
    amounts(&b, "getMaxAmount", |i, a| match a {
        Amount::None => i.max_amount(),
        Amount::Index(n) => i.max_amount_at(n),
        Amount::Item(s) => i.max_amount_for(&s),
    });
    amounts(&b, "getIntMaxAmount", |i, a| match a {
        Amount::None => i.int_max_amount() as f32,
        Amount::Index(n) => i.max_amount_at(n) as i32 as f32,
        Amount::Item(s) => i.max_amount_for(&s) as i32 as f32,
    });
    method(&b, "getRelativeScale", |c, h| {
        let item = opt_str(c, 1).unwrap_or_default();
        h.read(|i: &InputScript| i.relative_scale(&item).into_lua())
    });
    method(&b, "isCanBeDoneFromFloor", |_, h| {
        let own = h.read(|i: &InputScript| i.has_flag(InputFlag::CanBeDoneFromFloor))?;
        let recipe = match h.parent() {
            Some(p) => p.read(|r: &CraftRecipe| r.is_can_be_done_from_floor())?,
            None => false,
        };
        Ok((own || recipe).into_lua())
    });
}

enum Amount {
    None,
    Index(usize),
    Item(String),
}

/// `getAmount()`, `getAmount(int)`, `getAmount(String/Item)` and the like.
fn amounts(b: &crate::expose::ClassBuilder, name: &str, f: fn(&InputScript, Amount) -> f32) {
    let int = name.starts_with("getInt");
    method(b, name, move |c, h| {
        let a = match c.arg(1) {
            Value::Nil => Amount::None,
            Value::Num(n) => Amount::Index(n.max(0.0) as usize),
            _ => match item_name_arg(c, 1)? {
                Some(s) => Amount::Item(s),
                None => Amount::None,
            },
        };
        let v = h.read(|i: &InputScript| f(i, a))?;
        Ok(if int { (v as i32).into_lua() } else { v.into_lua() })
    });
}

fn parent_value(h: &ScriptHandle) -> Value {
    match h.parent() {
        Some(p) if p.is_root() => script_value(Some(&p.root)),
        Some(p) => super::canonical(p.clone()),
        None => Value::Nil,
    }
}

/// Another line of the same recipe, by the index `f` gives.
fn sibling<T: 'static>(h: &ScriptHandle, f: fn(&T) -> Option<usize>) -> LuaResult<Value> {
    let i = h.read(f)?;
    match (h.parent(), i) {
        (Some(p), Some(i)) => io_value(p, i),
        _ => Ok(Value::Nil),
    }
}

fn register_output(thread: &mut Thread) {
    let b = expose_class(thread, OUTPUT_SCRIPT_CLASS, Some(IO_SCRIPT_CLASS));
    method(&b, "hasFlag", |c, h| {
        let f = flag_arg(c, OutputFlag::ALL, OutputFlag::name);
        h.read(|o: &OutputScript| f.is_some_and(|f| o.has_flag(f)).into_lua())
    });
    get(&b, "getAmount", |o: &OutputScript| o.amount());
    get(&b, "getIntAmount", |o: &OutputScript| o.int_amount());
    get(&b, "getMaxAmount", |o: &OutputScript| o.max_amount());
    get(&b, "getIntMaxAmount", |o: &OutputScript| o.int_max_amount());
    get(&b, "getChance", |o: &OutputScript| o.chance());
    get(&b, "isVariableAmount", |o: &OutputScript| o.is_variable_amount());
    get(&b, "getShapedIndex", |o: &OutputScript| o.shaped_index());
    get(&b, "isApplyOnTick", |o: &OutputScript| o.is_apply_on_tick());
    get(&b, "isHandcraftOnly", |o: &OutputScript| o.is_handcraft_only());
    get(&b, "isAutomationOnly", |o: &OutputScript| o.is_automation_only());
    get(&b, "getOriginalLine", |o: &OutputScript| o.original_line().to_string());
    get(&b, "isFluidExact", |o: &OutputScript| o.is_fluid_exact());
    get(&b, "isFluidPrimary", |o: &OutputScript| o.is_fluid_primary());
    get(&b, "isFluidAnything", |o: &OutputScript| o.is_fluid_anything());
    get(&b, "hasCreateToItem", |o: &OutputScript| o.has_create_to_item());
    get(&b, "getResourceType", |o: &OutputScript| enum_value(RESOURCE_TYPE_CLASS, o.resource_type().name()));
    get(&b, "getItemApplyMode", |o: &OutputScript| enum_value(ITEM_APPLY_MODE_CLASS, o.item_apply_mode().name()));
    get(&b, "getFluidMatchMode", |o: &OutputScript| enum_value(FLUID_MATCH_MODE_CLASS, o.fluid_match_mode().name()));
    method(&b, "getCreateToItemScript", |_, h| sibling(h, |o: &OutputScript| o.create_to_item_script()));
    method(&b, "getParentRecipe", |_, h| Ok(parent_value(h)));
    method(&b, "getRecipeLineIndex", |_, h| Ok(line_index(h)?.map_or(-1, |(_, i)| i as i32).into_lua()));
    method(&b, "getPossibleResultItems", |_, h| {
        let Some((recipe, i)) = line_index(h)? else { return Ok(list(Vec::new())) };
        recipe.read(|r: &CraftRecipe| {
            let out = r.io_lines()[i].as_output();
            match out.and_then(|o| r.output_mapper_for(o)) {
                Some(m) => script_list(m.result_items()),
                None => list(Vec::new()),
            }
        })
    });
    method(&b, "containsItem", |c, h| {
        let Some(item) = handle_arg(c, 1)? else { return Ok(false.into_lua()) };
        let Some((recipe, i)) = line_index(h)? else { return Ok(false.into_lua()) };
        recipe.read(|r: &CraftRecipe| {
            let out = r.io_lines()[i].as_output();
            out.and_then(|o| r.output_mapper_for(o))
                .is_some_and(|m| m.result_items().iter().any(|s| s.ptr_eq(&item.root)))
                .into_lua()
        })
    });
}
