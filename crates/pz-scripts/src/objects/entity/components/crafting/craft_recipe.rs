//! `zombie.scripting.entity.components.crafting.CraftRecipe` and its nested
//! `IOScript`, `LuaCall`, `RequiredSkill` and `XpAward`.

use super::{
    craft_recipe_tag, CraftRecipeGroup, InputFlag, InputScript, ItemApplyMode, OutputMapper, OutputMapperRef,
    OutputScript, OverlayMapper,
};
use crate::base::{index_error, BaseScriptObject, ScriptObject, ScriptRef, ScriptResult};
use crate::java::{self, JHashMap};
use crate::manager::ScriptManager;
use crate::objects::entity::components::resources::ResourceType;
use crate::objects::entity::{
    array_get, contains_whitespace, exception, index_of, io_exception, null_pointer, parse_int, perk_from_string,
    try_parse_int,
};
use crate::parser::Block;
use crate::script_type::{ScriptLoadMode, ScriptType};

/// `CraftRecipe.IOScript`: an entry of `ioLines`. Lines refer to each
/// other (and mappers refer to inputs) by their index in `ioLines`.
#[derive(Clone, Debug)]
pub enum IoScript {
    Input(InputScript),
    Output(OutputScript),
}

impl IoScript {
    pub fn as_input(&self) -> Option<&InputScript> {
        match self {
            IoScript::Input(i) => Some(i),
            IoScript::Output(_) => None,
        }
    }

    pub fn as_output(&self) -> Option<&OutputScript> {
        match self {
            IoScript::Output(o) => Some(o),
            IoScript::Input(_) => None,
        }
    }

    pub fn resource_type(&self) -> ResourceType {
        match self {
            IoScript::Input(i) => i.resource_type(),
            IoScript::Output(o) => o.resource_type(),
        }
    }
}

/// `CraftRecipe.LuaCall`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum LuaCall {
    OnTest,
    OnStart,
    OnUpdate,
    OnCreate,
    OnFailed,
}

/// `CraftRecipe.RequiredSkill`. The perk is kept by its `PerkFactory.Perk` id.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RequiredSkill {
    perk: String,
    level: i32,
}

impl RequiredSkill {
    pub fn new(perk: impl Into<String>, level: i32) -> Self {
        RequiredSkill { perk: perk.into(), level }
    }

    /// `getPerk`.
    pub fn perk(&self) -> &str {
        &self.perk
    }

    /// `getLevel`.
    pub fn level(&self) -> i32 {
        self.level
    }
}

/// `CraftRecipe.XpAward`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct XpAward {
    perk: String,
    amount: i32,
}

impl XpAward {
    /// `getPerk`.
    pub fn perk(&self) -> &str {
        &self.perk
    }

    /// `getAmount`.
    pub fn amount(&self) -> i32 {
        self.amount
    }
}

#[derive(Debug)]
pub struct CraftRecipe {
    pub base: BaseScriptObject,
    name: Option<String>,
    translation_name: Option<String>,
    icon_name: Option<String>,
    recipe_group: Option<CraftRecipeGroup>,
    need_to_be_learn: bool,
    pub skill_required: Option<Vec<RequiredSkill>>,
    pub auto_learn_any: Option<Vec<RequiredSkill>>,
    pub auto_learn_all: Option<Vec<RequiredSkill>>,
    pub xp_award: Option<Vec<XpAward>>,
    pub meta_recipe: Option<String>,
    has_on_tick_inputs: bool,
    has_on_tick_outputs: bool,
    time: i32,
    loaded_timed_action_script: Option<String>,
    timed_action_script: Option<ScriptRef>,
    /// `luaCalls`, indexed by `LuaCall` ordinal.
    lua_calls: [Option<String>; 5],
    io_lines: Vec<IoScript>,
    /// Indices into `io_lines`.
    inputs: Vec<usize>,
    outputs: Vec<usize>,
    category: Option<String>,
    category_tags: Vec<String>,
    prop1: Option<usize>,
    prop2: Option<usize>,
    animation: Option<String>,
    tool_left: Option<usize>,
    tool_right: Option<usize>,
    exists_as_vanilla: bool,
    mod_id: Option<String>,
    output_mappers: JHashMap<OutputMapper>,
    pub(crate) overlay_mapper: OverlayMapper,
    uses_tools: bool,
    tooltip: Option<String>,
    allow_batch_craft: bool,
    can_walk: bool,
    on_add_to_menu: Option<String>,
    pub research_skill_level: i32,
    research_all: Vec<String>,
    research_any: Vec<String>,
}

impl Default for CraftRecipe {
    fn default() -> Self {
        Self::new()
    }
}

impl CraftRecipe {
    pub fn new() -> Self {
        CraftRecipe {
            base: BaseScriptObject::new(ScriptType::CraftRecipe),
            name: None,
            translation_name: None,
            icon_name: None,
            recipe_group: None,
            need_to_be_learn: false,
            skill_required: None,
            auto_learn_any: None,
            auto_learn_all: None,
            xp_award: None,
            meta_recipe: None,
            has_on_tick_inputs: false,
            has_on_tick_outputs: false,
            time: 50,
            loaded_timed_action_script: None,
            timed_action_script: None,
            lua_calls: Default::default(),
            io_lines: Vec::new(),
            inputs: Vec::new(),
            outputs: Vec::new(),
            category: None,
            category_tags: Vec::new(),
            prop1: None,
            prop2: None,
            animation: None,
            tool_left: None,
            tool_right: None,
            exists_as_vanilla: false,
            mod_id: None,
            output_mappers: JHashMap::new(),
            overlay_mapper: OverlayMapper::default(),
            uses_tools: false,
            tooltip: None,
            allow_batch_craft: true,
            can_walk: false,
            on_add_to_menu: None,
            research_skill_level: -1,
            research_all: Vec::new(),
            research_any: Vec::new(),
        }
    }

    /// `getOutputMapper`.
    pub fn output_mapper(&self, name: &str) -> Option<&OutputMapper> {
        self.output_mappers.get(name)
    }

    /// Every `itemMapper`, in `HashMap` order.
    pub fn output_mappers(&self) -> &JHashMap<OutputMapper> {
        &self.output_mappers
    }

    /// `getOrCreateOutputMapper`.
    pub(crate) fn get_or_create_output_mapper(&mut self, name: &str) -> &mut OutputMapper {
        if !self.output_mappers.contains_key(name) {
            self.output_mappers.insert(name.to_string(), OutputMapper::new(name));
        }
        self.output_mappers.get_mut(name).unwrap()
    }

    /// The mapper an output line uses (`OutputScript.getOutputMapper`).
    pub fn output_mapper_for<'a>(&'a self, output: &'a OutputScript) -> Option<&'a OutputMapper> {
        match output.output_mapper_ref()? {
            OutputMapperRef::Named(n) => self.output_mappers.get(n),
            OutputMapperRef::Own(m) => Some(m),
        }
    }

    /// `getOverlayMapper`.
    pub fn overlay_mapper(&self) -> &OverlayMapper {
        &self.overlay_mapper
    }

    /// `getExistsAsVanilla`.
    pub fn exists_as_vanilla(&self) -> bool {
        self.exists_as_vanilla
    }

    /// `isVanilla`.
    pub fn is_vanilla(&self) -> bool {
        self.mod_id.as_deref() == Some("pz-vanilla")
    }

    /// `getModID`. TODO(ChooseGameInfo): `getModName` reads the mod info.
    pub fn mod_id(&self) -> Option<&str> {
        self.mod_id.as_deref()
    }

    /// `getName`.
    pub fn name(&self) -> Option<&str> {
        self.name.as_deref()
    }

    /// `getTranslationName`.
    pub fn translation_name(&self) -> Option<&str> {
        self.translation_name.as_deref()
    }

    /// `overrideTranslationName`.
    pub fn override_translation_name(&mut self, name: Option<String>) {
        self.translation_name = name;
    }

    /// `getIconName`. TODO(Texture): `getIconTexture`.
    pub fn icon_name(&self) -> Option<&str> {
        self.icon_name.as_deref()
    }

    /// `needToBeLearn`.
    pub fn need_to_be_learn(&self) -> bool {
        self.need_to_be_learn
    }

    /// `canBeResearched`.
    pub fn can_be_researched(&mut self) -> bool {
        if self.cannot_be_researched() {
            return false;
        }
        self.can_always_be_researched() || self.need_to_be_learn
    }

    /// `canAlwaysBeResearched`.
    pub fn can_always_be_researched(&mut self) -> bool {
        if self.cannot_be_researched() {
            return false;
        }
        self.need_to_be_learn
            && (self.has_tag(craft_recipe_tag::CAN_ALWAYS_BE_RESEARCHED) || self.get_research_skill_level() == 0)
    }

    /// `cannotBeResearched`.
    pub fn cannot_be_researched(&self) -> bool {
        self.has_tag(craft_recipe_tag::CANNOT_BE_RESEARCHED)
    }

    /// `getTime()`.
    pub fn time(&self) -> i32 {
        self.time
    }

    /// `getTimedActionScript`.
    pub fn timed_action_script(&self) -> Option<&ScriptRef> {
        self.timed_action_script.as_ref()
    }

    /// The `timedAction` value as loaded.
    pub fn loaded_timed_action_script(&self) -> Option<&str> {
        self.loaded_timed_action_script.as_deref()
    }

    /// `getRecipeGroup`.
    pub fn recipe_group(&self) -> Option<CraftRecipeGroup> {
        self.recipe_group
    }

    /// `getCategory`: `"Miscellaneous"` when unset.
    pub fn category(&self) -> &str {
        match self.category.as_deref() {
            Some(c) if !java::is_null_or_whitespace(Some(c)) => c,
            _ => "Miscellaneous",
        }
    }

    /// The `category` value as loaded (`getCategory` defaults it).
    pub fn raw_category(&self) -> Option<&str> {
        self.category.as_deref()
    }

    /// `getTags` / `getModTags`. TODO(CraftRecipeManager): `getTagBits`.
    pub fn tags(&self) -> &[String] {
        &self.category_tags
    }

    /// `getInputCount`.
    pub fn input_count(&self) -> usize {
        self.inputs.len()
    }

    /// `getOutputCount`.
    pub fn output_count(&self) -> usize {
        self.outputs.len()
    }

    /// `getInputs`.
    pub fn inputs(&self) -> impl Iterator<Item = &InputScript> {
        self.inputs.iter().filter_map(|&i| self.io_lines[i].as_input())
    }

    /// `getOutputs`.
    pub fn outputs(&self) -> impl Iterator<Item = &OutputScript> {
        self.outputs.iter().filter_map(|&i| self.io_lines[i].as_output())
    }

    /// The `ioLines` indices of `getInputs`.
    pub fn input_indices(&self) -> &[usize] {
        &self.inputs
    }

    /// The `ioLines` indices of `getOutputs`.
    pub fn output_indices(&self) -> &[usize] {
        &self.outputs
    }

    /// `getIoLines`.
    pub fn io_lines(&self) -> &[IoScript] {
        &self.io_lines
    }

    /// `getIOForIndex`.
    pub fn io_for_index(&self, index: i32) -> Option<&IoScript> {
        usize::try_from(index).ok().and_then(|i| self.io_lines.get(i))
    }

    /// `isUsesTools`.
    pub fn is_uses_tools(&self) -> bool {
        self.uses_tools
    }

    fn input_at(&self, i: Option<usize>) -> Option<&InputScript> {
        self.io_lines.get(i?)?.as_input()
    }

    /// `getToolLeft`.
    pub fn tool_left(&self) -> Option<&InputScript> {
        self.input_at(self.tool_left)
    }

    /// `getToolRight`.
    pub fn tool_right(&self) -> Option<&InputScript> {
        self.input_at(self.tool_right)
    }

    /// `getProp1`.
    pub fn prop1(&self) -> Option<&InputScript> {
        self.input_at(self.prop1)
    }

    /// `getProp2`.
    pub fn prop2(&self) -> Option<&InputScript> {
        self.input_at(self.prop2)
    }

    /// The `ioLines` indices of the tool and prop inputs.
    pub fn tool_left_index(&self) -> Option<usize> {
        self.tool_left
    }

    pub fn tool_right_index(&self) -> Option<usize> {
        self.tool_right
    }

    pub fn prop1_index(&self) -> Option<usize> {
        self.prop1
    }

    pub fn prop2_index(&self) -> Option<usize> {
        self.prop2
    }

    /// `getAnimation`.
    pub fn animation(&self) -> Option<&str> {
        self.animation.as_deref()
    }

    /// `setAnimation`.
    pub fn set_animation(&mut self, animation: Option<String>) {
        self.animation = animation;
    }

    /// `hasOnTickInputs`.
    pub fn has_on_tick_inputs(&self) -> bool {
        self.has_on_tick_inputs
    }

    /// `hasOnTickOutputs`.
    pub fn has_on_tick_outputs(&self) -> bool {
        self.has_on_tick_outputs
    }

    /// `getLuaCallString`.
    pub fn lua_call_string(&self, call: LuaCall) -> Option<&str> {
        self.lua_calls[call as usize].as_deref()
    }

    /// `hasLuaCall`.
    pub fn has_lua_call(&self, call: LuaCall) -> bool {
        self.lua_call_string(call).is_some()
    }

    /// `getTooltip`.
    pub fn tooltip(&self) -> Option<&str> {
        self.tooltip.as_deref()
    }

    /// `isAllowBatchCraft`.
    pub fn is_allow_batch_craft(&self) -> bool {
        self.allow_batch_craft
    }

    /// `isCanWalk`.
    pub fn is_can_walk(&self) -> bool {
        self.can_walk
    }

    /// `getOnAddToMenu`.
    pub fn on_add_to_menu(&self) -> Option<&str> {
        self.on_add_to_menu.as_deref()
    }

    /// `getMetaRecipe`.
    pub fn meta_recipe(&self) -> Option<&str> {
        self.meta_recipe.as_deref()
    }

    /// The `ResearchAll` perk ids.
    pub fn research_all(&self) -> &[String] {
        &self.research_all
    }

    /// The `ResearchAny` perk ids.
    pub fn research_any(&self) -> &[String] {
        &self.research_any
    }

    /// `isResearchAll`.
    pub fn is_research_all(&self) -> bool {
        !self.research_all.is_empty()
    }

    /// `hasTag`.
    pub fn has_tag(&self, tag: &str) -> bool {
        self.category_tags.iter().any(|t| t == tag)
    }

    /// `isCanBeDoneFromFloor`.
    pub fn is_can_be_done_from_floor(&self) -> bool {
        self.has_tag(craft_recipe_tag::CAN_BE_DONE_FROM_FLOOR)
    }

    /// `canBeDoneInDark`.
    pub fn can_be_done_in_dark(&self) -> bool {
        self.has_tag(craft_recipe_tag::CAN_BE_DONE_IN_DARK)
    }

    /// `isAnySurfaceCraft`.
    pub fn is_any_surface_craft(&self) -> bool {
        self.has_tag(craft_recipe_tag::ANY_SURFACE_CRAFT)
    }

    /// `isInHandCraftCraft`.
    pub fn is_in_hand_craft(&self) -> bool {
        self.has_tag(craft_recipe_tag::IN_HAND_CRAFT)
    }

    /// `isAutoRotate`.
    pub fn is_auto_rotate(&self) -> bool {
        self.has_tag(craft_recipe_tag::AUTO_ROTATE)
    }

    /// `requiresSpecificWorkstation`.
    pub fn requires_specific_workstation(&self) -> bool {
        !self.category_tags.is_empty()
            && !self.has_tag(craft_recipe_tag::IN_HAND_CRAFT)
            && !self.has_tag(craft_recipe_tag::ANY_SURFACE_CRAFT)
            && !self.has_tag(craft_recipe_tag::ENTITY_RECIPE)
            && !self.has_tag(craft_recipe_tag::OUTDOORS)
    }

    /// `isBuildableRecipe`.
    pub fn is_buildable_recipe(&self) -> bool {
        self.has_tag(craft_recipe_tag::ENTITY_RECIPE)
    }

    /// `isSmithing`.
    pub fn is_smithing(&self) -> bool {
        self.involves_skill("Blacksmith", false) || self.category() == "Blacksmithing"
    }

    /// `involvesSkill`. The original tests the any-list's presence for the
    /// all-list too.
    pub fn involves_skill(&self, perk: &str, include_auto_learn: bool) -> bool {
        let any = |l: &Option<Vec<RequiredSkill>>| l.iter().flatten().any(|s| s.perk == perk);
        if any(&self.skill_required) || self.xp_award.iter().flatten().any(|x| x.perk == perk) {
            return true;
        }
        include_auto_learn && self.auto_learn_any.is_some() && (any(&self.auto_learn_any) || any(&self.auto_learn_all))
    }

    /// `getRequiredSkills`: `"<perk> <level>"` per skill. TODO(PerkFactory):
    /// the original prints the perk's (translatable) name.
    pub fn required_skills(&self) -> Vec<String> {
        self.skill_required.iter().flatten().map(|s| format!("{} {}", s.perk, s.level)).collect()
    }

    /// `getRequiredSkillCount`.
    pub fn required_skill_count(&self) -> usize {
        self.skill_required.as_ref().map_or(0, Vec::len)
    }

    /// `getXPAwardCount`.
    pub fn xp_award_count(&self) -> usize {
        self.xp_award.as_ref().map_or(0, Vec::len)
    }

    /// `getHighestSkillRequirement(includeAutoLearn)`. The original tests the
    /// any-list's presence for the all-list too.
    pub fn highest_skill_requirement(&self, include_auto_learn: bool) -> i32 {
        let mut highest = 0;
        let mut take = |l: &Option<Vec<RequiredSkill>>| {
            for s in l.iter().flatten() {
                if s.level > highest {
                    highest = s.level;
                }
            }
        };
        take(&self.skill_required);
        if include_auto_learn && self.auto_learn_any.is_some() {
            take(&self.auto_learn_any);
            take(&self.auto_learn_all);
        }
        highest
    }

    /// `normalizeSkillLevel`: clamps to 0..=10.
    pub fn normalize_skill_level(level: i32) -> i32 {
        level.clamp(0, 10)
    }

    /// `getResearchSkillLevel()`: derives and stores a level from the skill
    /// requirements on first use. TODO(IsoGameCharacter): the per-character
    /// overload.
    pub fn get_research_skill_level(&mut self) -> i32 {
        if self.research_skill_level == -1 {
            let highest = match self.highest_skill_requirement(false) {
                0 => self.highest_skill_requirement(true),
                h => h,
            };
            if highest > 0 {
                // Math.round(float): floor(x + 0.5).
                let research_level = (highest as f32 * 2.0 / 3.0 + 0.5).floor();
                self.research_skill_level = Self::normalize_skill_level(research_level.max(1.0) as i32);
            }
        }
        self.research_skill_level = Self::normalize_skill_level(self.research_skill_level);
        self.research_skill_level
    }

    /// `setResearchSkillLevel`.
    pub fn set_research_skill_level(&mut self, level: i32) {
        self.research_skill_level = Self::normalize_skill_level(level);
    }

    /// `addRequiredSkill`.
    pub fn add_required_skill(&mut self, perk: &str, level: i32) {
        self.skill_required.get_or_insert_with(Vec::new).push(RequiredSkill::new(perk, level));
    }

    /// `clearRequiredSkills`.
    pub fn clear_required_skills(&mut self) {
        if let Some(s) = &mut self.skill_required {
            s.clear();
        }
    }

    /// Parses `perk:level;perk:level` (`SkillRequired`, `AutoLearnAny`,
    /// `AutoLearnAll`, `xpAward`), skipping unknown perks.
    fn parse_skills(sm: &ScriptManager, val: &str, recipe: &str) -> ScriptResult<Vec<(String, i32)>> {
        let mut out = Vec::new();
        for entry in java::split_char(val, ';') {
            let split3 = java::split_char(entry, ':');
            let perk = perk_from_string(array_get(&split3, 0)?);
            if perk == "MAX" {
                sm.warn(&format!("Unknown skill \"{}\" in recipe \"{recipe}\"", split3[0]));
                continue;
            }
            out.push((perk.to_string(), try_parse_int(array_get(&split3, 1)?, 1)));
        }
        Ok(out)
    }

    /// Parses `perk;perk` (`ResearchAll`, `ResearchAny`).
    fn parse_perks(sm: &ScriptManager, val: &str, recipe: &str, list: &mut Vec<String>) {
        for entry in java::split_char(val, ';') {
            let perk = perk_from_string(entry);
            if perk == "MAX" {
                sm.warn(&format!("Unknown skill \"{entry}\" in recipe \"{recipe}\""));
                continue;
            }
            list.push(perk.to_string());
        }
    }

    /// `Load(String, Block)`.
    pub fn load_block(&mut self, sm: &ScriptManager, name: &str, block: &Block) -> ScriptResult {
        self.name = Some(name.to_string());
        // TODO(Translator): `Translator.getRecipeName(name)`, which falls
        // back to the name when there is no translation.
        self.translation_name = Some(name.to_string());
        self.base.load_common_block(block);
        let recipe = self.name.clone().unwrap_or_default();
        for value in &block.values {
            let key = java::trim(value.get_key());
            let val = java::trim(value.get_value());
            if key.is_empty() || val.is_empty() || java::eq_ignore_case(key, "consumeOnFinish") {
                continue;
            }
            let eq = |k: &str| java::eq_ignore_case(key, k);
            let v = Some(val.to_string());
            if eq("recipeGroup") {
                self.recipe_group = CraftRecipeGroup::from_string(val);
            } else if eq("icon") {
                self.icon_name = v;
            } else if eq("time") {
                self.time = parse_int(val)?;
            } else if eq("ResearchSkillLevel") {
                self.research_skill_level = parse_int(val)?;
            } else if eq("OnTest") {
                self.lua_calls[LuaCall::OnTest as usize] = v;
            } else if eq("OnAddToMenu") {
                self.on_add_to_menu = v;
            } else if eq("OnStart") {
                self.lua_calls[LuaCall::OnStart as usize] = v;
            } else if eq("OnUpdate") {
                self.lua_calls[LuaCall::OnUpdate as usize] = v;
            } else if eq("OnCreate") {
                self.lua_calls[LuaCall::OnCreate as usize] = v;
            } else if eq("OnFailed") {
                self.lua_calls[LuaCall::OnFailed as usize] = v;
            } else if eq("requiresPlayer") {
            } else if eq("needToBeLearn") {
                self.need_to_be_learn = java::eq_ignore_case(val, "true");
            } else if eq("SkillRequired") {
                self.skill_required = Some(Vec::new());
                for (perk, level) in Self::parse_skills(sm, val, &recipe)? {
                    if level > 0 {
                        self.skill_required.as_mut().unwrap().push(RequiredSkill::new(perk, level));
                    }
                }
            } else if eq("AutoLearnAny") {
                self.auto_learn_any = Some(Vec::new());
                let skills = Self::parse_skills(sm, val, &recipe)?;
                self.auto_learn_any = Some(skills.into_iter().map(|(p, l)| RequiredSkill::new(p, l)).collect());
            } else if eq("AutoLearnAll") {
                self.auto_learn_all = Some(Vec::new());
                let skills = Self::parse_skills(sm, val, &recipe)?;
                self.auto_learn_all = Some(skills.into_iter().map(|(p, l)| RequiredSkill::new(p, l)).collect());
            } else if eq("ResearchAll") {
                Self::parse_perks(sm, val, &recipe, &mut self.research_all);
            } else if eq("ResearchAny") {
                Self::parse_perks(sm, val, &recipe, &mut self.research_any);
            } else if eq("xpAward") {
                self.xp_award = Some(Vec::new());
                let skills = Self::parse_skills(sm, val, &recipe)?;
                self.xp_award = Some(skills.into_iter().map(|(perk, amount)| XpAward { perk, amount }).collect());
            } else if eq("MetaRecipe") {
                self.meta_recipe = v;
            } else if eq("Animation") {
                self.animation = v;
            } else if eq("timedAction") {
                self.loaded_timed_action_script = v;
            } else if eq("category") {
                self.category = v;
            } else if eq("Tags") {
                self.category_tags.extend(java::split_char(val, ';').iter().map(|t| java::trim(t).to_string()));
            } else if eq("Tooltip") {
                self.tooltip = v;
            } else if eq("AllowBatchCraft") {
                self.allow_batch_craft = java::eq_ignore_case(val, "true");
            } else if eq("CanWalk") {
                self.can_walk = java::eq_ignore_case(val, "true");
            } else if eq("overlayStyle") {
                self.overlay_mapper.set_default_overlay_style(val);
            } else {
                sm.script_error(&format!("Unknown key '{key}' val({val}) in craft recipe: {name}"));
                if sm.debug {
                    return Err(exception(format!("CraftRecipe error in {name}")));
                }
            }
        }
        for child in &block.children {
            let t = child.type_str();
            if java::eq_ignore_case("inputs", t) || java::eq_ignore_case("outputs", t) {
                self.load_io(sm, child, java::eq_ignore_case("inputs", t))?;
            } else if java::eq_ignore_case("itemMapper", t) {
                self.load_output_mapper(child)?;
            } else if java::eq_ignore_case("fluidMapper", t) || java::eq_ignore_case("energyMapper", t) {
                return Err(exception("Not yet implemented."));
            } else if java::eq_ignore_case("overlayMapper", t) {
                self.load_overlay_mapper(sm, child)?;
            } else {
                sm.script_error(&format!("Unknown block '{t}' in craft recipe: {name}"));
                if sm.debug {
                    return Err(exception(format!(
                        "CraftRecipe error in {name}, Unknown block '{t}' in craft recipe: {name}"
                    )));
                }
            }
        }
        Ok(())
    }

    /// `LoadIO`: the lines of an `inputs` or `outputs` block. `+` lines add
    /// an item-create output to the previous line, `-` lines a consume input.
    fn load_io(&mut self, sm: &ScriptManager, block: &Block, is_input: bool) -> ScriptResult {
        let name = self.name.clone().unwrap_or_else(|| "null".to_string());
        let mut last_input: Option<usize> = None;
        let mut last_output: Option<usize> = None;
        for value in &block.values {
            let s = java::trim(&value.string);
            if java::is_null_or_whitespace(Some(s)) {
                continue;
            }
            if !contains_whitespace(s) {
                sm.warn(&format!("Cannot load: {}", value.string));
            }
            let index = self.io_lines.len();
            if is_input {
                if java::to_lower(s).starts_with("replace") {
                    sm.script_error("Replace inputs have been deprecated in CraftRecipes.");
                    continue;
                }
                if let Some(rest) = s.strip_prefix('+') {
                    let last =
                        last_input.ok_or_else(|| io_exception(format!("Previous input is null [{name}] line: {s}")))?;
                    if self.input_mut(last).create_to_item_script.is_some() {
                        return Err(io_exception(format!("Previous input already has '+' output [{name}] line: {s}")));
                    }
                    let script = OutputScript::load(self, sm, rest, true)?.ok_or_else(null_pointer)?;
                    self.input_mut(last).create_to_item_script = Some(index);
                    self.io_lines.push(IoScript::Output(script));
                    continue;
                }
                if let Some(rest) = s.strip_prefix('-') {
                    let last =
                        last_input.ok_or_else(|| io_exception(format!("Previous input is null [{name}] line: {s}")))?;
                    if self.input_mut(last).consume_from_item_script.is_some() {
                        return Err(io_exception(format!("Previous input already has '-' input [{name}] line: {s}")));
                    }
                    let mut script = InputScript::load(self, sm, rest, true, index)?.ok_or_else(null_pointer)?;
                    self.input_mut(last).consume_from_item_script = Some(index);
                    script.parent_script = Some(last);
                    self.io_lines.push(IoScript::Input(script));
                    continue;
                }
                let input = InputScript::load(self, sm, &value.string, false, index)?.ok_or_else(null_pointer)?;
                self.io_lines.push(IoScript::Input(input));
                self.inputs.push(index);
                last_input = Some(index);
                continue;
            }
            if let Some(rest) = s.strip_prefix('+') {
                let last =
                    last_output.ok_or_else(|| io_exception(format!("Previous input is null [{name}] line: {s}")))?;
                let script = OutputScript::load(self, sm, rest, true)?.ok_or_else(null_pointer)?;
                if let IoScript::Output(o) = &mut self.io_lines[last] {
                    o.create_to_item_script = Some(index);
                }
                self.io_lines.push(IoScript::Output(script));
                last_output = None;
                continue;
            }
            if s.starts_with('-') {
                return Err(io_exception(format!("Cannot add '-' line to output, [{name}] line: {s}")));
            }
            let output = OutputScript::load(self, sm, &value.string, false)?.ok_or_else(null_pointer)?;
            self.io_lines.push(IoScript::Output(output));
            self.outputs.push(index);
            last_output = Some(index);
        }
        Ok(())
    }

    fn input_mut(&mut self, index: usize) -> &mut InputScript {
        match &mut self.io_lines[index] {
            IoScript::Input(i) => i,
            IoScript::Output(_) => unreachable!("index of an input line"),
        }
    }

    /// `LoadOutputMapper`: `result = a;b` patterns and a `default`.
    fn load_output_mapper(&mut self, block: &Block) -> ScriptResult {
        let id = block.id.as_deref().ok_or_else(null_pointer)?;
        let mapper_name = java::trim(id).to_string();
        self.get_or_create_output_mapper(&mapper_name);
        for value in &block.values {
            let key = java::trim(value.get_key());
            let val = java::trim(value.get_value());
            if key.is_empty() || val.is_empty() {
                continue;
            }
            let mapper = self.output_mappers.get_mut(&mapper_name).unwrap();
            if java::eq_ignore_case(key, "default") {
                mapper.set_default_output_entree(val);
                continue;
            }
            let mut split: Vec<&str> = java::split_char(val, ';');
            for entry in split.iter_mut() {
                let s = *entry;
                if !s.contains('$') {
                    continue;
                }
                let dollar = index_of(s, "$") as usize;
                if matches!(java::to_lower(&s[..dollar]).as_str(), "fluid" | "energy") {
                    return Err(exception("Not yet implemented"));
                }
                *entry = &s[dollar + 1..];
            }
            mapper.add_output_entree(key, &split);
        }
        if self.output_mappers.get(&mapper_name).unwrap().is_empty() {
            return Err(exception(format!("Failed to load contents for output mapper: {id}")));
        }
        Ok(())
    }

    /// `LoadOverlayMapper`: `style = Item;Item` entries and a `default`.
    fn load_overlay_mapper(&mut self, sm: &ScriptManager, block: &Block) -> ScriptResult {
        for value in &block.values {
            let key = java::trim(value.get_key());
            let val = java::trim(value.get_value());
            if key.is_empty() || val.is_empty() {
                continue;
            }
            if java::eq_ignore_case(key, "default") {
                self.overlay_mapper.set_default_overlay_style(val);
                continue;
            }
            let split = java::split_char(key, ';');
            self.overlay_mapper.add_overlay_style_entry(sm, val, &split)?;
        }
        if self.overlay_mapper.is_empty() {
            return Err(exception("Failed to load contents for overlay mapper."));
        }
        Ok(())
    }

    /// `OnPostWorldDictionaryInit` of each input line (and its `+`/`-` lines).
    fn input_post_world_dictionary_init(&mut self, sm: &ScriptManager, index: usize) -> ScriptResult {
        let recipe_name = self.name.clone();
        let (create, consume) = {
            let i = self.input_mut(index);
            (i.create_to_item_script, i.consume_from_item_script)
        };
        let line = self.input_mut(index).original_line.clone();
        if let (Some(c), Some(d)) = (create, consume) {
            if self.io_lines[c].resource_type() == self.io_lines[d].resource_type() {
                return Err(exception(format!(
                    "Input line cannot have a '-' and '+' line of the same ResourceType. line: {line}"
                )));
            }
        }
        for (other, sign) in [(create, '+'), (consume, '-')] {
            let Some(other) = other else { continue };
            match &self.io_lines[other] {
                IoScript::Output(_) => self.output_post_world_dictionary_init(sm, other)?,
                IoScript::Input(_) => self.input_post_world_dictionary_init(sm, other)?,
            }
            let input = self.input_mut(index);
            if input.int_amount() != 1 || input.is_variable_amount() {
                return Err(exception(format!(
                    "Lines prior to a '{sign}' line should have 1 item amount. line: {line}"
                )));
            }
            if input.type_ != ResourceType::Item {
                return Err(exception(format!(
                    "Lines prior to a '{sign}' line should be of resource type Item. line: {line}"
                )));
            }
            if input.apply_on_tick {
                return Err(exception(format!(
                    "Lines prior to a '{sign}' line should not be apply on tick. line: {line}"
                )));
            }
            if sign == '+' || input.item_apply_mode != ItemApplyMode::Destroy {
                input.item_apply_mode = ItemApplyMode::Keep;
            }
        }
        let linked =
            |t: ResourceType| [create, consume].iter().flatten().any(|&i| self.io_lines[i].resource_type() == t);
        let filtered = linked(ResourceType::Fluid) || linked(ResourceType::Energy);
        let input = self.input_mut(index);
        input.resolve_items(sm, recipe_name.as_deref(), filtered)?;
        // TODO(Fluid): fluids and categories go into the sealed fluid filter;
        // the original throws for unknown fluids. TODO(Energy): likewise for energies.
        input.fluid_filter_sealed = true;
        if !input.is_valid() {
            sm.script_error(&format!("Invalid input. line: {line}"));
        }
        Ok(())
    }

    /// `OutputScript.OnPostWorldDictionaryInit`.
    fn output_post_world_dictionary_init(&mut self, sm: &ScriptManager, index: usize) -> ScriptResult {
        let recipe_name = self.name.clone();
        let IoScript::Output(output) = &self.io_lines[index] else { unreachable!("index of an output line") };
        let line = output.original_line.clone();
        if let Some(c) = output.create_to_item_script {
            self.output_post_world_dictionary_init(sm, c)?;
            let IoScript::Output(output) = &mut self.io_lines[index] else { unreachable!() };
            if output.int_amount() != 1 || output.is_variable_amount() {
                return Err(exception(format!("Lines prior to a '+' line should have 1 item amount. line: {line}")));
            }
            if output.type_ != ResourceType::Item {
                return Err(exception(format!(
                    "Lines prior to a '+' line should be of resource type Item. line: {line}"
                )));
            }
            if output.apply_on_tick {
                return Err(exception(format!("Lines prior to a '+' line should not be apply on tick. line: {line}")));
            }
            output.item_apply_mode = ItemApplyMode::Normal;
        }
        let IoScript::Output(output) = &mut self.io_lines[index] else { unreachable!() };
        match output.type_ {
            ResourceType::Item => {
                let mapper = match &mut output.output_mapper {
                    None => return Err(exception(format!("No outputMapper set. line: {line}"))),
                    Some(OutputMapperRef::Own(m)) => m,
                    Some(OutputMapperRef::Named(n)) => {
                        let n = n.clone();
                        self.output_mappers.get_mut(&n).ok_or_else(null_pointer)?
                    }
                };
                mapper.on_post_world_dictionary_init(sm, recipe_name.as_deref())?;
                if mapper.result_items().is_empty() {
                    return Err(exception(format!("Invalid output. line: {line}")));
                }
            }
            // TODO(Fluid), TODO(Energy): resolve `Fluid.Get` / `Energy.Get`
            // (the original throws when not found, and when the line is
            // invalid).
            _ => output.resolved = true,
        }
        Ok(())
    }
}

impl ScriptObject for CraftRecipe {
    fn base(&self) -> &BaseScriptObject {
        &self.base
    }

    fn base_mut(&mut self) -> &mut BaseScriptObject {
        &mut self.base
    }

    fn class_name(&self) -> &'static str {
        "zombie.scripting.entity.components.crafting.CraftRecipe"
    }

    /// `InitLoadPP`. TODO(ChooseGameInfo): non-vanilla recipes look up
    /// their mod details.
    fn init_load_pp(&mut self, sm: &ScriptManager, name: &str) {
        self.base.init_load_pp(name);
        self.mod_id = sm.current_load_file_mod();
        if self.mod_id.as_deref() == Some("pz-vanilla") {
            self.exists_as_vanilla = true;
        }
    }

    fn load(&mut self, sm: &ScriptManager, name: &str, body: &str) -> ScriptResult {
        let root = sm.parse(body);
        let block = root.children.first().ok_or_else(index_error)?;
        self.load_block(sm, name, block)
    }

    fn pre_reload(&mut self) {
        self.name = None;
        self.translation_name = None;
        self.base.enabled = true;
        self.base.debug_only = false;
        self.icon_name = None;
        self.recipe_group = None;
        self.need_to_be_learn = false;
        self.skill_required = None;
        self.auto_learn_any = None;
        self.auto_learn_all = None;
        self.has_on_tick_inputs = false;
        self.has_on_tick_outputs = false;
        self.time = 50;
        self.loaded_timed_action_script = None;
        self.timed_action_script = None;
        self.lua_calls = Default::default();
        self.inputs.clear();
        self.outputs.clear();
        self.io_lines.clear();
        self.category = None;
        self.category_tags.clear();
        self.prop1 = None;
        self.prop2 = None;
        self.animation = None;
        self.overlay_mapper.clear();
    }

    /// `OnScriptsLoaded`. TODO(Texture): the icon texture lookup.
    fn on_scripts_loaded(&mut self, sm: &ScriptManager, _mode: ScriptLoadMode) -> ScriptResult {
        if let Some(icon) = &self.icon_name {
            if !icon.starts_with("Item_") {
                self.icon_name = Some(format!("Item_{icon}"));
            }
        }
        let name = self.name.clone().unwrap_or_else(|| "null".to_string());
        for k in 0..self.inputs.len() {
            let index = self.inputs[k];
            let input = self.input_mut(index);
            let (on_tick, tool, left, right, p1, p2) = (
                input.is_apply_on_tick(),
                input.is_tool(),
                input.has_flag(InputFlag::ToolLeft),
                input.has_flag(InputFlag::ToolRight),
                input.is_prop1(),
                input.is_prop2(),
            );
            if on_tick {
                self.has_on_tick_inputs = true;
            }
            if tool {
                self.uses_tools = true;
                if left {
                    if self.tool_left.is_some() {
                        return Err(exception(format!("Duplicate tool left in recipe {name}")));
                    }
                    self.tool_left = Some(index);
                }
                if right {
                    if self.tool_right.is_some() {
                        return Err(exception(format!("Duplicate tool left in recipe {name}")));
                    }
                    self.tool_right = Some(index);
                }
            }
            if p1 {
                if self.prop1.is_some() {
                    return Err(io_exception(format!("Duplicate Prop1 in recipe {name}")));
                }
                self.prop1 = Some(index);
            }
            if p2 {
                if self.prop2.is_some() {
                    return Err(io_exception(format!("Duplicate Prop2 in recipe {name}")));
                }
                self.prop2 = Some(index);
            }
        }
        if self.outputs().any(OutputScript::is_apply_on_tick) {
            self.has_on_tick_outputs = true;
        }
        if let Some(t) = &self.loaded_timed_action_script {
            self.timed_action_script = sm.get_script(ScriptType::TimedAction, t);
            if self.timed_action_script.is_none() {
                return Err(exception(format!("TimedActionScript '{t}' could not be found in recipe {name}")));
            }
        }
        Ok(())
    }

    /// `OnPostWorldDictionaryInit`. TODO(Texture): the icon fallback to the
    /// first input or output item's texture.
    fn on_post_world_dictionary_init(&mut self, sm: &ScriptManager) -> ScriptResult {
        for k in 0..self.inputs.len() {
            self.input_post_world_dictionary_init(sm, self.inputs[k])?;
        }
        for k in 0..self.outputs.len() {
            self.output_post_world_dictionary_init(sm, self.outputs[k])?;
        }
        Ok(())
    }
}
