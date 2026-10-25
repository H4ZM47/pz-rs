//! Script classes of the `items` group: `Item` and the classes it stores,
//! the registry keys items refer to, character trait/profession
//! definitions, item filters and physics hit reactions, plus the item steps
//! at the end of `ScriptManager.Load`.

mod character_definitions;
mod item;
mod item_filter;
pub mod keys;
mod parts;
mod physics_hit_reaction;

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

pub use character_definitions::{
    add_character_profession_definition, add_character_trait_definition, add_granted_trait,
    character_profession_definition, character_profession_definitions, character_trait_definition,
    character_trait_definitions, is_mutually_exclusive, reset_character_profession_definitions,
    reset_character_trait_definitions, set_mutual_exclusive, CharacterProfessionDefinition,
    CharacterProfessionDefinitionRef, CharacterProfessionDefinitionScript,
    CharacterTraitDefinition, CharacterTraitDefinitionRef, CharacterTraitDefinitionScript,
};
pub use item::{Item, KeySet, ModDataValue};
pub use item_filter::{resolve_get_item_types, FilterTypeInfo, ItemFilterScript};
pub use keys::{
    load_builtin_constants, AmmoType, BookSubject, BuiltinConstants, CharacterProfession,
    CharacterTrait, GameMode, ItemBodyLocation, ItemTag, MoodleType,
    ItemType, MagazineSubject, WeaponCategory,
};
pub use parts::{ItemReplacement, ModelWeaponPart, VehiclePartModel};
pub use physics_hit_reaction::{
    impulse_for_ammo, impulse_for_object, upward_impulse_for_ammo, upward_impulse_for_object,
    PhysicsHitReaction, PhysicsHitReactionScript, BODYPART_COUNT,
};

use crate::base::{ScriptError, ScriptRef, ScriptResult};
use crate::java::{self, JHashMap};
use crate::manager::{ScriptManager, BASE};
use crate::module::ScriptModule;
use crate::objects::models::{self, ItemModelRefs};
use crate::registry::RegistryKey;
use crate::script_type::ScriptType;

/// `createInstance` for the script types this group ports; `None` for
/// types it does not (yet) handle.
pub fn create_instance(
    type_: ScriptType,
    _module: &Rc<ScriptModule>,
    _name: &str,
    _token: &str,
) -> Option<ScriptRef> {
    Some(match type_ {
        ScriptType::Item => ScriptRef::new(Item::new()),
        ScriptType::ItemFilter => ScriptRef::new(ItemFilterScript::new()),
        ScriptType::CharacterTraitDefinition => {
            ScriptRef::new(CharacterTraitDefinitionScript::new())
        }
        ScriptType::CharacterProfessionDefinition => {
            ScriptRef::new(CharacterProfessionDefinitionScript::new())
        }
        ScriptType::PhysicsHitReaction => ScriptRef::new(PhysicsHitReactionScript::new()),
        _ => return None,
    })
}

/// Java static fields of this group's classes (reset by `ScriptManager.Reset`).
#[derive(Default)]
pub struct Statics {
    /// `Item.netIdToItem`.
    pub net_id_to_item: HashMap<i32, String>,
    /// `Item.netItemToId`.
    pub net_item_to_id: JHashMap<i32>,
    /// `Item.idMax`.
    pub id_max: i32,
    /// `ScriptManager.clothingToItemMap`.
    pub clothing_to_item_map: JHashMap<String>,
    /// `ScriptManager.visualDamagesList`.
    pub visual_damages_list: Vec<String>,
    /// `ScriptManager.tagToItemMap`, keyed by the tag's location id.
    pub tag_to_item_map: HashMap<Option<String>, Vec<Rc<RefCell<Item>>>>,
    /// `Translator.itemEvolvedRecipeName`, the part an item's
    /// `EvolvedRecipeName` writes. TODO(Translator): translated names.
    pub item_evolved_recipe_name: HashMap<String, String>,
    /// `CharacterTraitDefinition.characterTraitDefinitions` (a
    /// `LinkedHashMap` keyed by trait).
    pub character_trait_definitions: Vec<CharacterTraitDefinitionRef>,
    /// `CharacterProfessionDefinition.characterProfessionDefinitions`.
    pub character_profession_definitions: Vec<CharacterProfessionDefinitionRef>,
    /// `PhysicsHitReactionScript.physicsHitReactionList`.
    pub physics_hit_reaction_list: Vec<PhysicsHitReaction>,
    /// `PhysicsHitReactionScript.originalFilename`.
    pub physics_hit_reaction_original_filename: Option<String>,
    /// Ids of the key classes' built-in constants (see [`keys`]).
    pub constants: BuiltinConstants,
}

/// Group steps run at the end of `ScriptManager.Load`: `debugItems`,
/// `resolveItemTypes`, `ModelScript.ScriptsLoaded` (with
/// `Item.resolveModelScripts`), `createClothingItemMap`, `createZedDmgMap`.
pub fn post_load(sm: &ScriptManager) {
    let items = get_all_items(sm);
    debug_items(sm, &items);
    for item in &items {
        item.borrow_mut().resolve_item_types(sm);
    }
    let mut refs = Vec::with_capacity(items.len());
    for item in &items {
        let mut item = item.borrow_mut();
        item.resolve_model_scripts(sm);
        refs.push(ItemModelRefs {
            full_name: item.full_name(),
            static_model: item.static_model.clone(),
            weapon_sprite: item.weapon_sprite.clone(),
            world_static_model: item.world_static_model.clone(),
            clothing_item: item.clothing_item.clone(),
            is_food: item.is_item_type(sm, "FOOD"),
        });
    }
    models::model_script::scripts_loaded(sm, &refs, &|rel| sm.active_file_exists(rel));
    create_clothing_item_map(sm, &items);
    create_zed_dmg_map(sm);
}

/// `ScriptManager.getAllItems`.
pub fn get_all_items(sm: &ScriptManager) -> Vec<Rc<RefCell<Item>>> {
    sm.get_all(ScriptType::Item)
}

/// `ScriptManager.getItem`.
pub fn get_item(sm: &ScriptManager, name: &str) -> Option<Rc<RefCell<Item>>> {
    sm.get(ScriptType::Item, name)
}

/// `ScriptManager.FindItem(name)`.
pub fn find_item(sm: &ScriptManager, name: &str) -> Option<Rc<RefCell<Item>>> {
    sm.find_item(name, true)?.downcast::<Item>()
}

/// `ScriptManager.debugItems`: script warnings only.
fn debug_items(sm: &ScriptManager, items: &[Rc<RefCell<Item>>]) {
    for item in items {
        let item = item.borrow();
        if item.is_item_type(sm, "DRAINABLE") && item.replace_on_use.is_some() {
            sm.script_warn(&format!(
                "{} ReplaceOnUse instead of ReplaceOnDeplete",
                item.full_name()
            ));
        }
    }
}

/// `ScriptManager.createClothingItemMap`.
fn create_clothing_item_map(sm: &ScriptManager, items: &[Rc<RefCell<Item>>]) {
    let mut st = sm.statics.items.borrow_mut();
    for item in items {
        let item = item.borrow();
        let Some(ci) = item.clothing_item.as_deref() else {
            continue;
        };
        if java::is_null_or_whitespace(Some(ci)) {
            continue;
        }
        st.clothing_to_item_map
            .insert(ci.to_string(), item.full_name());
    }
}

/// `ScriptManager.createZedDmgMap`: the `Base` items worn at the built-in
/// `ItemBodyLocation.ZED_DMG`, in the module's map order.
fn create_zed_dmg_map(sm: &ScriptManager) {
    let mut list = Vec::new();
    if let Some(module) = sm.get_module(BASE) {
        for s in module.bucket(ScriptType::Item).script_map_values() {
            let Some(item) = s.downcast::<Item>() else {
                continue;
            };
            let item = item.borrow();
            if keys::is_constant(
                sm,
                item.body_location.as_ref(),
                ItemBodyLocation::REGISTRY,
                "ZED_DMG",
            ) {
                list.push(item.name.clone());
            }
        }
    }
    sm.statics.items.borrow_mut().visual_damages_list = list;
}

/// `ScriptManager.getItemTypeForClothingItem`.
pub fn get_item_type_for_clothing_item(sm: &ScriptManager, clothing_item: &str) -> Option<String> {
    sm.statics
        .items
        .borrow()
        .clothing_to_item_map
        .get(clothing_item)
        .cloned()
}

/// `ScriptManager.getItemForClothingItem`.
pub fn get_item_for_clothing_item(
    sm: &ScriptManager,
    clothing_name: &str,
) -> Option<Rc<RefCell<Item>>> {
    let item_type = get_item_type_for_clothing_item(sm, clothing_name)?;
    find_item(sm, &item_type)
}

/// `ScriptManager.getZedDmgMap`.
pub fn get_zed_dmg_map(sm: &ScriptManager) -> Vec<String> {
    sm.statics.items.borrow().visual_damages_list.clone()
}

/// `ScriptManager.getItemsTag` / `getAllItemsWithTag`: cached per tag.
pub fn get_items_tag(sm: &ScriptManager, tag: Option<&Rc<RegistryKey>>) -> Vec<Rc<RefCell<Item>>> {
    let key = tag.map(|t| t.location.id());
    if let Some(items) = sm.statics.items.borrow().tag_to_item_map.get(&key) {
        return items.clone();
    }
    let items: Vec<_> = get_all_items(sm)
        .into_iter()
        .filter(|i| i.borrow().has_tag(tag))
        .collect();
    sm.statics
        .items
        .borrow_mut()
        .tag_to_item_map
        .insert(key, items.clone());
    items
}

/// `ScriptManager.isDrainableItemType`.
pub fn is_drainable_item_type(sm: &ScriptManager, item_type: &str) -> bool {
    find_item(sm, item_type).is_some_and(|i| i.borrow().is_item_type(sm, "DRAINABLE"))
}

/// `Item.netIdToItem.get(id)`.
pub fn net_id_to_item(sm: &ScriptManager, id: i32) -> Option<String> {
    sm.statics.items.borrow().net_id_to_item.get(&id).cloned()
}

/// `Item.netItemToId.get(fullType)`.
pub fn net_item_to_id(sm: &ScriptManager, full_type: &str) -> Option<i32> {
    sm.statics
        .items
        .borrow()
        .net_item_to_id
        .get(full_type)
        .copied()
}

/// `Texture.trygetTexture(name)` as far as it goes without the texture
/// system: a loose `media/textures/<name>.png` is found (the running game
/// loads it), names that only exist in texture packs are not.
/// TODO(Texture): texture packs and loading.
pub(crate) fn try_get_texture(sm: &ScriptManager, name: &str) -> ScriptResult<bool> {
    if name.contains("..") {
        return Ok(false);
    }
    let mut file = format!("media/textures/{name}");
    if !name.ends_with(".png") {
        file.push_str(".png");
    }
    Ok(sm.active_file_exists(&java::to_lower(&file)))
}

/// `Float.parseFloat`, with its `NumberFormatException` text.
pub(crate) fn parse_float(s: &str) -> ScriptResult<f32> {
    java::parse_f32(s).ok_or_else(|| {
        let t = java::trim(s);
        ScriptError::new(if t.is_empty() {
            "java.lang.NumberFormatException: empty String".to_string()
        } else {
            format!("java.lang.NumberFormatException: For input string: \"{t}\"")
        })
    })
}

/// `Integer.parseInt`, with its `NumberFormatException` text.
pub(crate) fn parse_int(s: &str) -> ScriptResult<i32> {
    java::parse_i32(s).ok_or_else(|| {
        ScriptError::new(format!(
            "java.lang.NumberFormatException: For input string: \"{s}\""
        ))
    })
}

/// `Byte.parseByte`.
pub(crate) fn parse_byte(s: &str) -> ScriptResult<i8> {
    let v = parse_int(s)?;
    i8::try_from(v).map_err(|_| {
        ScriptError::new(format!(
            "java.lang.NumberFormatException: Value out of range. Value:\"{s}\" Radix:10"
        ))
    })
}

/// `array[index]` on a `String.split` result.
pub(crate) fn at<'a>(arr: &[&'a str], index: usize) -> ScriptResult<&'a str> {
    arr.get(index).copied().ok_or_else(|| {
        ScriptError::new(format!(
            "java.lang.ArrayIndexOutOfBoundsException: Index {index} out of bounds for length {}",
            arr.len()
        ))
    })
}

/// `new InvalidParameterException(msg)`.
pub(crate) fn invalid_parameter(msg: Option<&str>) -> ScriptError {
    match msg {
        Some(m) => ScriptError::new(format!("java.security.InvalidParameterException: {m}")),
        None => ScriptError::new("java.security.InvalidParameterException"),
    }
}

/// `Throwable.getMessage()` of an exception held as `class: message`.
pub(crate) fn exception_message(e: &ScriptError) -> Option<&str> {
    match e.0.split_once(": ") {
        Some((class, msg)) if !class.contains(' ') => Some(msg),
        Some(_) => Some(&e.0),
        None if !e.0.contains(' ') => None,
        None => Some(&e.0),
    }
}

/// `StringUtils.discardNullOrWhitespace`.
pub(crate) fn discard_null_or_whitespace(s: &str) -> Option<String> {
    (!java::is_null_or_whitespace(Some(s))).then(|| s.to_string())
}

#[cfg(test)]
mod tests;
