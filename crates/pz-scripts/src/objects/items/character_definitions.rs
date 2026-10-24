//! `CharacterTraitDefinitionScript` and `CharacterProfessionDefinitionScript`,
//! which register `CharacterTraitDefinition`/`CharacterProfessionDefinition`
//! entries (kept in this group's statics).

use std::cell::RefCell;
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
fn xp_boosts(sm: &ScriptManager, v: &str, limit_two: bool) -> Vec<(String, i32)> {
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
        let perk = perk_from_string(sm, java::trim(pair[0]));
        if perk == "MAX" {
            continue;
        }
        out.push((perk, try_parse_int(pair[1], 1)));
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

/// A definition as the original shares it (`characterTraitDefinitions`
/// values, Lua handles).
pub type CharacterTraitDefinitionRef = Rc<RefCell<CharacterTraitDefinition>>;

impl CharacterTraitDefinition {
    /// The constructor: dereferences the trait type for its texture name.
    pub fn new(
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

    /// `isGrantedRecipe`.
    pub fn is_granted_recipe(&self, recipe: &str) -> bool {
        self.granted_recipes.iter().any(|r| r == recipe)
    }

    /// `hasMutuallyExclusiveTraits`.
    pub fn has_mutually_exclusive_traits(&self) -> bool {
        !self.mutually_exclusive_traits.is_empty()
    }

    /// `addXPBoost`.
    pub fn add_xp_boost(&mut self, perk: &str, level: i32) {
        put_xp_boost(&mut self.xp_boosts, perk, level);
    }

    /// `addMutuallyExclusive` (no duplicate check).
    pub fn add_mutually_exclusive(&mut self, t: Option<Rc<RegistryKey>>) {
        self.mutually_exclusive_traits.push(t);
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

/// `isMutuallyExclusive`: `a` excludes `b`'s type directly or through one
/// of its granted traits' definitions.
pub fn is_mutually_exclusive(
    sm: &ScriptManager,
    a: &CharacterTraitDefinitionRef,
    b: &CharacterTraitDefinitionRef,
) -> bool {
    let other = b.borrow().character_trait_type.clone();
    let (direct, granted) = {
        let a = a.borrow();
        (
            list_contains(&a.mutually_exclusive_traits, other.as_ref()),
            a.granted_traits.clone(),
        )
    };
    if direct {
        return true;
    }
    granted.iter().any(|g| {
        character_trait_definition(sm, g.as_ref()).is_some_and(|d| {
            list_contains(&d.borrow().mutually_exclusive_traits, other.as_ref())
        })
    })
}

/// `updateMutualExclusiveTraits`.
fn update_mutual_exclusive_traits(sm: &ScriptManager, this: &CharacterTraitDefinitionRef) {
    for other in character_trait_definitions(sm) {
        if Rc::ptr_eq(this, &other)
            || (!is_mutually_exclusive(sm, this, &other) && !is_mutually_exclusive(sm, &other, this))
        {
            continue;
        }
        let other_type = other.borrow().character_trait_type.clone();
        let this_type = this.borrow().character_trait_type.clone();
        let mut t = this.borrow_mut();
        if !list_contains(&t.mutually_exclusive_traits, other_type.as_ref()) {
            t.mutually_exclusive_traits.push(other_type);
        }
        drop(t);
        let mut o = other.borrow_mut();
        if !list_contains(&o.mutually_exclusive_traits, this_type.as_ref()) {
            o.mutually_exclusive_traits.push(this_type);
        }
    }
}

/// `addGrantedTrait`.
pub fn add_granted_trait(
    sm: &ScriptManager,
    def: &CharacterTraitDefinitionRef,
    t: Option<Rc<RegistryKey>>,
) {
    def.borrow_mut().granted_traits.push(t);
    update_mutual_exclusive_traits(sm, def);
}

/// `CharacterTraitDefinition.getTraits()`, in registration order.
pub fn character_trait_definitions(sm: &ScriptManager) -> Vec<CharacterTraitDefinitionRef> {
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
) -> Option<CharacterTraitDefinitionRef> {
    let st = sm.statics.items.borrow();
    st.character_trait_definitions
        .iter()
        .find(|d| key_eq(d.borrow().character_trait_type.as_ref(), t))
        .cloned()
}

/// `CharacterTraitDefinition.addCharacterTraitDefinition`: a new definition
/// replacing any for the same trait in place (`LinkedHashMap.put`).
pub fn add_character_trait_definition(
    sm: &ScriptManager,
    trait_type: Option<Rc<RegistryKey>>,
    name: Option<&str>,
    cost: i32,
    description: Option<&str>,
    is_profession_trait: bool,
    disabled_in_multiplayer: bool,
) -> ScriptResult<CharacterTraitDefinitionRef> {
    let def = Rc::new(RefCell::new(CharacterTraitDefinition::new(
        trait_type.clone(),
        name,
        cost,
        description,
        is_profession_trait,
        disabled_in_multiplayer,
    )?));
    let mut st = sm.statics.items.borrow_mut();
    let defs = &mut st.character_trait_definitions;
    match defs
        .iter()
        .position(|d| key_eq(d.borrow().character_trait_type.as_ref(), trait_type.as_ref()))
    {
        Some(i) => defs[i] = def.clone(),
        None => defs.push(def.clone()),
    }
    Ok(def)
}

/// `CharacterTraitDefinition.reset`.
pub fn reset_character_trait_definitions(sm: &ScriptManager) {
    sm.statics.items.borrow_mut().character_trait_definitions.clear();
}

/// `CharacterTraitDefinition.setMutualExclusive`; a trait without a
/// definition is a `NullPointerException`.
pub fn set_mutual_exclusive(
    sm: &ScriptManager,
    a: Option<&Rc<RegistryKey>>,
    b: Option<&Rc<RegistryKey>>,
) -> ScriptResult {
    let npe = || ScriptError::new("java.lang.NullPointerException");
    let da = character_trait_definition(sm, a).ok_or_else(npe)?;
    {
        let mut d = da.borrow_mut();
        if !list_contains(&d.mutually_exclusive_traits, b) {
            d.mutually_exclusive_traits.push(b.cloned());
        }
    }
    let db = character_trait_definition(sm, b).ok_or_else(npe)?;
    {
        let mut d = db.borrow_mut();
        if !list_contains(&d.mutually_exclusive_traits, a) {
            d.mutually_exclusive_traits.push(a.cloned());
        }
    }
    update_mutual_exclusive_traits(sm, &da);
    update_mutual_exclusive_traits(sm, &db);
    Ok(())
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
                xp.extend(xp_boosts(sm, v, false));
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
        let def = add_character_trait_definition(
            sm,
            trait_type,
            ui_name,
            cost,
            ui_description,
            profession,
            disabled_in_multiplayer,
        )?;
        {
            let mut d = def.borrow_mut();
            for (perk, level) in xp {
                d.add_xp_boost(&perk, level);
            }
            for t in mutually_exclusive {
                d.add_mutually_exclusive(t);
            }
            d.granted_recipes.extend(granted_recipes);
        }
        for t in granted_traits {
            add_granted_trait(sm, &def, t);
        }
        if let Some(t) = texture_name {
            def.borrow_mut().texture_name = t.to_string();
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
    pub description: Option<String>,
    /// TODO(Texture): the icon texture.
    pub icon_path_name: Option<String>,
    pub granted_traits: Vec<Option<Rc<RegistryKey>>>,
    pub granted_recipes: Vec<String>,
    /// `xpBoosts` (perk id -> level).
    pub xp_boosts: Vec<(String, i32)>,
}

/// A shared profession definition.
pub type CharacterProfessionDefinitionRef = Rc<RefCell<CharacterProfessionDefinition>>;

impl CharacterProfessionDefinition {
    /// The constructor (`Texture.trygetTexture` on a non-empty icon path).
    pub fn new(
        sm: &ScriptManager,
        profession_type: Option<Rc<RegistryKey>>,
        name: Option<&str>,
        cost: i32,
        description: Option<&str>,
        icon_path_name: Option<&str>,
    ) -> ScriptResult<Self> {
        if let Some(icon) = icon_path_name.filter(|i| !i.is_empty()) {
            try_get_texture(sm, icon)?;
        }
        Ok(CharacterProfessionDefinition {
            character_profession_type: profession_type,
            display_name: name.map(str::to_string),
            cost,
            description: Some(
                description
                    .filter(|d| !d.is_empty())
                    .unwrap_or("")
                    .to_string(),
            ),
            icon_path_name: icon_path_name.map(str::to_string),
            granted_traits: Vec::new(),
            granted_recipes: Vec::new(),
            xp_boosts: Vec::new(),
        })
    }

    /// `getRightLabel`.
    pub fn right_label(&self) -> String {
        cost_label(self.cost)
    }

    /// `isGrantedRecipe`.
    pub fn is_granted_recipe(&self, recipe: &str) -> bool {
        self.granted_recipes.iter().any(|r| r == recipe)
    }

    /// `addXPBoost`.
    pub fn add_xp_boost(&mut self, perk: &str, level: i32) {
        put_xp_boost(&mut self.xp_boosts, perk, level);
    }
}

/// `CharacterProfessionDefinition.getProfessions()`, in registration order.
pub fn character_profession_definitions(
    sm: &ScriptManager,
) -> Vec<CharacterProfessionDefinitionRef> {
    sm.statics
        .items
        .borrow()
        .character_profession_definitions
        .clone()
}

/// `CharacterProfessionDefinition.getCharacterProfessionDefinition`.
pub fn character_profession_definition(
    sm: &ScriptManager,
    p: Option<&Rc<RegistryKey>>,
) -> Option<CharacterProfessionDefinitionRef> {
    let st = sm.statics.items.borrow();
    st.character_profession_definitions
        .iter()
        .find(|d| key_eq(d.borrow().character_profession_type.as_ref(), p))
        .cloned()
}

/// `CharacterProfessionDefinition.addCharacterProfessionDefinition`.
pub fn add_character_profession_definition(
    sm: &ScriptManager,
    profession_type: Option<Rc<RegistryKey>>,
    name: Option<&str>,
    cost: i32,
    description: Option<&str>,
    icon_path_name: Option<&str>,
) -> ScriptResult<CharacterProfessionDefinitionRef> {
    let def = Rc::new(RefCell::new(CharacterProfessionDefinition::new(
        sm,
        profession_type.clone(),
        name,
        cost,
        description,
        icon_path_name,
    )?));
    let mut st = sm.statics.items.borrow_mut();
    let defs = &mut st.character_profession_definitions;
    match defs.iter().position(|d| {
        key_eq(
            d.borrow().character_profession_type.as_ref(),
            profession_type.as_ref(),
        )
    }) {
        Some(i) => defs[i] = def.clone(),
        None => defs.push(def.clone()),
    }
    Ok(def)
}

/// `CharacterProfessionDefinition.reset`.
pub fn reset_character_profession_definitions(sm: &ScriptManager) {
    sm.statics
        .items
        .borrow_mut()
        .character_profession_definitions
        .clear();
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
                xp.extend(xp_boosts(sm, v, true));
            } else if is("IconPathName") {
                icon_path_name = Some(v);
            } else {
                unknown_key(sm, "CharacterProfessionDefinitionScript", k, block)?;
            }
        }
        let def = add_character_profession_definition(
            sm,
            profession,
            ui_name,
            cost,
            ui_description,
            icon_path_name,
        )?;
        let mut d = def.borrow_mut();
        for (perk, level) in xp {
            d.add_xp_boost(&perk, level);
        }
        d.granted_recipes.extend(granted_recipes);
        d.granted_traits.extend(granted_traits);
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
