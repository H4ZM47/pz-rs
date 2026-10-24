//! `CharacterTraitDefinitionScript` and `CharacterProfessionDefinitionScript`,
//! which register `CharacterTraitDefinition`/`CharacterProfessionDefinition`
//! entries (kept in this group's statics).

use std::rc::Rc;

use super::{parse_int, try_get_texture};
use crate::base::{index_error, BaseScriptObject, ScriptError, ScriptObject, ScriptResult};
use crate::java;
use crate::manager::ScriptManager;
use crate::objects::entity::{perk_from_string, try_parse_int};
use crate::parser::{Block, BlockElement};
use crate::registry::{RegistryKey, ResourceLocation};
use crate::script_type::ScriptType;

fn key_eq(a: Option<&Rc<RegistryKey>>, b: Option<&Rc<RegistryKey>>) -> bool {
    match (a, b) {
        (Some(a), Some(b)) => a.location == b.location,
        (None, None) => true,
        _ => false,
    }
}

fn list_contains(list: &[Option<Rc<RegistryKey>>], key: Option<&Rc<RegistryKey>>) -> bool {
    list.iter().any(|k| key_eq(k.as_ref(), key))
}

/// `HashMap.put` on a perk-keyed map held as a list.
fn put_xp_boost(boosts: &mut Vec<(String, i32)>, perk: &str, level: i32) {
    match boosts.iter_mut().find(|(p, _)| p == perk) {
        Some(e) => e.1 = level,
        None => boosts.push((perk.to_string(), level)),
    }
}

/// `Registries.CHARACTER_TRAIT.get(ResourceLocation.of(id))`.
fn character_trait(sm: &ScriptManager, id: &str) -> ScriptResult<Option<Rc<RegistryKey>>> {
    Ok(sm
        .registries
        .get("character_trait")
        .expect("known registry")
        .borrow()
        .get(&ResourceLocation::of(id)?))
}

/// The `key = value` pairs of a definition block: `split("=", 2)`, both
/// sides trimmed, pairs with an empty side skipped.
fn pairs(block: &Block) -> impl Iterator<Item = (&str, &str)> {
    block.elements.iter().filter_map(move |e| match e {
        BlockElement::Value(i) => {
            let (k, v) = block.values[*i].string.split_once('=')?;
            let (k, v) = (java::trim(k), java::trim(v));
            (!k.is_empty() && !v.is_empty()).then_some((k, v))
        }
        BlockElement::Block(_) => None,
    })
}

/// The `XPBoosts` value: `Perk=level;...`; unknown perks are skipped and
/// a bad level counts as 1.
fn xp_boosts(v: &str, limit_two: bool) -> Vec<(String, i32)> {
    let mut out = Vec::new();
    for s in java::split_char(v, ';') {
        let pair: Vec<&str> = if limit_two {
            match s.split_once('=') {
                Some((a, b)) => vec![a, b],
                None => vec![s],
            }
        } else {
            java::split_char(s, '=')
        };
        if pair.len() < 2 {
            continue;
        }
        let perk = perk_from_string(java::trim(pair[0]));
        if perk == "MAX" {
            continue;
        }
        out.push((perk.to_string(), try_parse_int(pair[1], 1)));
    }
    out
}

/// An unknown key: logged, and an error in debug builds.
fn unknown_key(sm: &ScriptManager, class: &str, k: &str, block: &Block) -> ScriptResult {
    let id = block.id.as_deref().unwrap_or("null");
    sm.script_error(&format!("Unknown key '{k}' in {class} '{id}'"));
    if sm.debug {
        return Err(ScriptError::new(format!(
            "java.lang.Exception: {class} error in {id}"
        )));
    }
    Ok(())
}

/// `zombie.characters.traits.CharacterTraitDefinition`.
#[derive(Clone, Debug)]
pub struct CharacterTraitDefinition {
    pub character_trait_type: Option<Rc<RegistryKey>>,
    /// `Translator.getText(UIName)`. TODO(Translator): translated; this is
    /// the untranslated key.
    pub display_name: Option<String>,
    pub cost: i32,
    pub description: Option<String>,
    pub is_profession_trait: bool,
    /// The texture's path. TODO(Texture): the texture, falling back to
    /// `trait_generic.png`.
    pub texture_name: String,
    pub disabled_in_multiplayer: bool,
    pub granted_traits: Vec<Option<Rc<RegistryKey>>>,
    pub granted_recipes: Vec<String>,
    pub mutually_exclusive_traits: Vec<Option<Rc<RegistryKey>>>,
    /// `xpBoosts` (perk id -> level).
    pub xp_boosts: Vec<(String, i32)>,
}

impl CharacterTraitDefinition {
    /// The constructor: dereferences the trait type for its texture name.
    fn new(
        trait_type: Option<Rc<RegistryKey>>,
        name: Option<&str>,
        cost: i32,
        description: Option<&str>,
        is_profession_trait: bool,
        disabled_in_multiplayer: bool,
    ) -> ScriptResult<Self> {
        let path = trait_type
            .as_ref()
            .ok_or_else(|| ScriptError::new("java.lang.NullPointerException"))?
            .path();
        Ok(CharacterTraitDefinition {
            display_name: name.map(str::to_string),
            cost,
            description: description.filter(|d| !d.is_empty()).map(str::to_string),
            is_profession_trait,
            texture_name: format!("media/ui/Traits/trait_{}.png", java::to_lower(path)),
            character_trait_type: trait_type,
            disabled_in_multiplayer,
            granted_traits: Vec::new(),
            granted_recipes: Vec::new(),
            mutually_exclusive_traits: Vec::new(),
            xp_boosts: Vec::new(),
        })
    }

    /// `getRightLabel`.
    pub fn right_label(&self) -> String {
        cost_label(self.cost)
    }
}

/// `getRightLabel` of both definition classes: a positive cost is shown
/// as `-cost`, a negative one as `+|cost|`.
fn cost_label(cost: i32) -> String {
    if cost == 0 {
        return String::new();
    }
    let before = if cost > 0 { "-" } else { "+" };
    format!(
        "{before}{}",
        if cost < 0 { cost.wrapping_neg() } else { cost }
    )
}

/// `isMutuallyExclusive`, on definitions held in `defs`.
fn is_mutually_exclusive(defs: &[CharacterTraitDefinition], a: usize, b: usize) -> bool {
    let other = defs[b].character_trait_type.as_ref();
    if list_contains(&defs[a].mutually_exclusive_traits, other) {
        return true;
    }
    defs[a].granted_traits.iter().any(|g| {
        defs.iter()
            .find(|d| key_eq(d.character_trait_type.as_ref(), g.as_ref()))
            .is_some_and(|d| list_contains(&d.mutually_exclusive_traits, other))
    })
}

/// `updateMutualExclusiveTraits`.
fn update_mutual_exclusive_traits(defs: &mut [CharacterTraitDefinition], i: usize) {
    for j in 0..defs.len() {
        if i == j || (!is_mutually_exclusive(defs, i, j) && !is_mutually_exclusive(defs, j, i)) {
            continue;
        }
        let tj = defs[j].character_trait_type.clone();
        if !list_contains(&defs[i].mutually_exclusive_traits, tj.as_ref()) {
            defs[i].mutually_exclusive_traits.push(tj);
        }
        let ti = defs[i].character_trait_type.clone();
        if !list_contains(&defs[j].mutually_exclusive_traits, ti.as_ref()) {
            defs[j].mutually_exclusive_traits.push(ti);
        }
    }
}

/// `CharacterTraitDefinition.getTraits()`, in registration order.
pub fn character_trait_definitions(sm: &ScriptManager) -> Vec<CharacterTraitDefinition> {
    sm.statics
        .items
        .borrow()
        .character_trait_definitions
        .clone()
}

/// `CharacterTraitDefinition.getCharacterTraitDefinition`.
pub fn character_trait_definition(
    sm: &ScriptManager,
    t: Option<&Rc<RegistryKey>>,
) -> Option<CharacterTraitDefinition> {
    let st = sm.statics.items.borrow();
    st.character_trait_definitions
        .iter()
        .find(|d| key_eq(d.character_trait_type.as_ref(), t))
        .cloned()
}

/// `zombie.scripting.objects.CharacterTraitDefinitionScript`.
#[derive(Debug)]
pub struct CharacterTraitDefinitionScript {
    pub base: BaseScriptObject,
}

impl Default for CharacterTraitDefinitionScript {
    fn default() -> Self {
        Self::new()
    }
}

impl CharacterTraitDefinitionScript {
    pub fn new() -> Self {
        CharacterTraitDefinitionScript {
            base: BaseScriptObject::new(ScriptType::CharacterTraitDefinition),
        }
    }

    /// `loadCharacterTraitDefinition`.
    fn load_definition(&mut self, sm: &ScriptManager, block: &Block) -> ScriptResult {
        let mut trait_type = None;
        let mut ui_name = None;
        let mut cost = 0;
        let mut ui_description = None;
        let mut profession = false;
        let mut disabled_in_multiplayer = false;
        let mut texture_name = None;
        let mut xp = Vec::new();
        let mut mutually_exclusive = Vec::new();
        let mut granted_recipes = Vec::new();
        let mut granted_traits = Vec::new();
        for (k, v) in pairs(block) {
            let is = |key: &str| java::eq_ignore_case(k, key);
            if is("CharacterTrait") {
                trait_type = character_trait(sm, v)?;
            } else if is("UIName") {
                ui_name = Some(v);
            } else if is("Cost") {
                cost = parse_int(v)?;
            } else if is("UIDescription") {
                ui_description = Some(v);
            } else if is("IsProfessionTrait") {
                profession = java::parse_bool(v);
            } else if is("DisabledInMultiplayer") {
                disabled_in_multiplayer = java::parse_bool(v);
            } else if is("GrantedRecipes") {
                granted_recipes.extend(java::split_char(v, ';').into_iter().map(str::to_string));
            } else if is("GrantedTraits") {
                for s in java::split_char(v, ';') {
                    granted_traits.push(character_trait(sm, java::trim(s))?);
                }
            } else if is("XPBoosts") {
                xp.extend(xp_boosts(v, false));
            } else if is("MutuallyExclusiveTraits") {
                for s in java::split_char(v, ';') {
                    mutually_exclusive.push(character_trait(sm, java::trim(s))?);
                }
            } else if is("Texture") {
                texture_name = Some(v);
            } else {
                unknown_key(sm, "CharacterTraitDefinitionScript", k, block)?;
            }
        }
        let def = CharacterTraitDefinition::new(
            trait_type.clone(),
            ui_name,
            cost,
            ui_description,
            profession,
            disabled_in_multiplayer,
        )?;
        let mut st = sm.statics.items.borrow_mut();
        let defs = &mut st.character_trait_definitions;
        let i = match defs
            .iter()
            .position(|d| key_eq(d.character_trait_type.as_ref(), trait_type.as_ref()))
        {
            Some(i) => {
                defs[i] = def;
                i
            }
            None => {
                defs.push(def);
                defs.len() - 1
            }
        };
        for (perk, level) in xp {
            put_xp_boost(&mut defs[i].xp_boosts, &perk, level);
        }
        defs[i].mutually_exclusive_traits.extend(mutually_exclusive);
        defs[i].granted_recipes.extend(granted_recipes);
        for t in granted_traits {
            defs[i].granted_traits.push(t);
            update_mutual_exclusive_traits(defs, i);
        }
        if let Some(t) = texture_name {
            defs[i].texture_name = t.to_string();
        }
        Ok(())
    }
}

impl ScriptObject for CharacterTraitDefinitionScript {
    fn base(&self) -> &BaseScriptObject {
        &self.base
    }

    fn base_mut(&mut self) -> &mut BaseScriptObject {
        &mut self.base
    }

    fn class_name(&self) -> &'static str {
        "zombie.scripting.objects.CharacterTraitDefinitionScript"
    }

    fn load(&mut self, sm: &ScriptManager, _name: &str, body: &str) -> ScriptResult {
        let root = sm.parse(body);
        let block = root.children.first().ok_or_else(index_error)?;
        self.base.load_common_block(block);
        self.load_definition(sm, block)
    }
}

/// `zombie.characters.professions.CharacterProfessionDefinition`.
#[derive(Clone, Debug)]
pub struct CharacterProfessionDefinition {
    pub character_profession_type: Option<Rc<RegistryKey>>,
    /// `Translator.getText(UIName)`. TODO(Translator): translated; this is
    /// the untranslated key.
    pub display_name: Option<String>,
    pub cost: i32,
    pub description: String,
    /// TODO(Texture): the icon texture.
    pub icon_path_name: Option<String>,
    pub granted_traits: Vec<Option<Rc<RegistryKey>>>,
    pub granted_recipes: Vec<String>,
    /// `xpBoosts` (perk id -> level).
    pub xp_boosts: Vec<(String, i32)>,
}

impl CharacterProfessionDefinition {
    /// `getRightLabel`.
    pub fn right_label(&self) -> String {
        cost_label(self.cost)
    }

    /// `isGrantedRecipe`.
    pub fn is_granted_recipe(&self, recipe: &str) -> bool {
        self.granted_recipes.iter().any(|r| r == recipe)
    }
}

/// `CharacterProfessionDefinition.getProfessions()`, in registration order.
pub fn character_profession_definitions(sm: &ScriptManager) -> Vec<CharacterProfessionDefinition> {
    sm.statics
        .items
        .borrow()
        .character_profession_definitions
        .clone()
}

/// `zombie.scripting.objects.CharacterProfessionDefinitionScript`.
#[derive(Debug)]
pub struct CharacterProfessionDefinitionScript {
    pub base: BaseScriptObject,
}

impl Default for CharacterProfessionDefinitionScript {
    fn default() -> Self {
        Self::new()
    }
}

impl CharacterProfessionDefinitionScript {
    pub fn new() -> Self {
        CharacterProfessionDefinitionScript {
            base: BaseScriptObject::new(ScriptType::CharacterProfessionDefinition),
        }
    }

    /// `loadCharacterProfessionDefinition`.
    fn load_definition(&mut self, sm: &ScriptManager, block: &Block) -> ScriptResult {
        let mut profession = None;
        let mut ui_name = None;
        let mut cost = 0;
        let mut ui_description = None;
        let mut icon_path_name: Option<&str> = None;
        let mut granted_traits = Vec::new();
        let mut granted_recipes = Vec::new();
        let mut xp = Vec::new();
        for (k, v) in pairs(block) {
            let is = |key: &str| java::eq_ignore_case(k, key);
            if is("CharacterProfession") {
                let reg = sm
                    .registries
                    .get("character_profession")
                    .expect("known registry");
                profession = reg.borrow().get(&ResourceLocation::of(v)?);
            } else if is("UIName") {
                ui_name = Some(v);
            } else if is("Cost") {
                cost = parse_int(v)?;
            } else if is("UIDescription") {
                ui_description = Some(v);
            } else if is("GrantedRecipes") {
                granted_recipes.extend(java::split_char(v, ';').into_iter().map(str::to_string));
            } else if is("GrantedTraits") {
                for s in java::split_char(v, ';') {
                    granted_traits.push(character_trait(sm, java::trim(s))?);
                }
            } else if is("XPBoosts") {
                xp.extend(xp_boosts(v, true));
            } else if is("IconPathName") {
                icon_path_name = Some(v);
            } else {
                unknown_key(sm, "CharacterProfessionDefinitionScript", k, block)?;
            }
        }
        if let Some(icon) = icon_path_name.filter(|i| !i.is_empty()) {
            try_get_texture(sm, icon)?;
        }
        let mut def = CharacterProfessionDefinition {
            character_profession_type: profession,
            display_name: ui_name.map(str::to_string),
            cost,
            description: ui_description
                .filter(|d| !d.is_empty())
                .unwrap_or("")
                .to_string(),
            icon_path_name: icon_path_name.map(str::to_string),
            granted_traits: Vec::new(),
            granted_recipes: Vec::new(),
            xp_boosts: Vec::new(),
        };
        for (perk, level) in xp {
            put_xp_boost(&mut def.xp_boosts, &perk, level);
        }
        def.granted_recipes = granted_recipes;
        def.granted_traits = granted_traits;
        let mut st = sm.statics.items.borrow_mut();
        let defs = &mut st.character_profession_definitions;
        match defs.iter().position(|d| {
            key_eq(
                d.character_profession_type.as_ref(),
                def.character_profession_type.as_ref(),
            )
        }) {
            Some(i) => defs[i] = def,
            None => defs.push(def),
        }
        Ok(())
    }
}

impl ScriptObject for CharacterProfessionDefinitionScript {
    fn base(&self) -> &BaseScriptObject {
        &self.base
    }

    fn base_mut(&mut self) -> &mut BaseScriptObject {
        &mut self.base
    }

    fn class_name(&self) -> &'static str {
        "zombie.scripting.objects.CharacterProfessionDefinitionScript"
    }

    fn load(&mut self, sm: &ScriptManager, _name: &str, body: &str) -> ScriptResult {
        let root = sm.parse(body);
        let block = root.children.first().ok_or_else(index_error)?;
        self.base.load_common_block(block);
        self.load_definition(sm, block)
    }
}
