//! `zombie.scripting.itemConfig.script.BucketRootScript`: a root block of an
//! item config (`Attribute <id> { ... }`, `FluidContainer <id>`, `LuaFunc`).

use super::item_config::item_config_exception;
use super::{RootType, SelectorBucketScript, SelectorType, SituatedType};
use crate::base::ScriptResult;
use crate::java;
use crate::manager::ScriptManager;
use crate::objects::entity::{null_pointer, parse_int};
use crate::parser::Block;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BucketRootScript {
    type_: RootType,
    id: Option<String>,
    default_bucket: SelectorBucketScript,
    on_create_bucket: Option<SelectorBucketScript>,
}

impl BucketRootScript {
    /// `getType`.
    pub fn root_type(&self) -> RootType {
        self.type_
    }

    /// `getId`.
    pub fn id(&self) -> Option<&str> {
        self.id.as_deref()
    }

    /// `setId`.
    pub fn set_id(&mut self, id: Option<String>) {
        self.id = id;
    }

    /// `idIsVariable`.
    pub fn id_is_variable(&self) -> bool {
        self.id.as_deref().is_some_and(|id| id.starts_with('$'))
    }

    /// `getDefaultBucket`.
    pub fn default_bucket(&self) -> &SelectorBucketScript {
        &self.default_bucket
    }

    /// `getOnCreateBucket`.
    pub fn on_create_bucket(&self) -> Option<&SelectorBucketScript> {
        self.on_create_bucket.as_ref()
    }

    /// `copy`.
    pub fn copy(&self) -> BucketRootScript {
        self.clone()
    }

    /// `TryLoad` (`load`). The original keeps the id only when it is blank
    /// (inverted test), so roots that require an id fail to load.
    pub fn try_load(sm: &ScriptManager, block: &Block) -> ScriptResult<BucketRootScript> {
        sm.statics.entity.borrow_mut().item_config_error_root = block.id.clone();
        let type_ = RootType::value_of(java::trim(block.type_str()))?;
        let id = if java::is_null_or_whitespace(block.id.as_deref()) {
            Some(java::trim(block.id.as_deref().ok_or_else(null_pointer)?).to_string())
        } else {
            None
        };
        if type_.is_requires_id() && id.is_none() {
            return Err(item_config_exception(sm, &format!("Root node with type '{}' requires id.", type_.name())));
        }
        let mut default_bucket = None;
        let mut on_create_bucket = None;
        for child in &block.children {
            if java::eq_ignore_case(child.type_str(), "default") {
                default_bucket = Some(load_selector_bucket(sm, child, false)?);
            } else if java::eq_ignore_case(child.type_str(), "oncreate") {
                on_create_bucket = Some(load_selector_bucket(sm, child, false)?);
            }
        }
        let mut default_bucket = default_bucket.unwrap_or_else(|| SelectorBucketScript::new(SelectorType::Default));
        for child in &block.children {
            if java::eq_ignore_case(child.type_str(), "default") || java::eq_ignore_case(child.type_str(), "oncreate") {
                continue;
            }
            let bucket = load_selector_bucket(sm, child, true)?;
            default_bucket.children.push(bucket);
        }
        sm.statics.entity.borrow_mut().item_config_error_root = None;
        Ok(BucketRootScript { type_, id, default_bucket, on_create_bucket })
    }
}

/// `loadSelectorBucket`; `has_parent` stands for a non-null `parent`.
pub(crate) fn load_selector_bucket(
    sm: &ScriptManager,
    block: &Block,
    has_parent: bool,
) -> ScriptResult<SelectorBucketScript> {
    let t = block.type_.as_deref();
    sm.statics.entity.borrow_mut().item_config_error_bucket =
        Some(format!("{} {}", t.unwrap_or("null"), block.id.as_deref().unwrap_or("")));
    let err = |msg: &str| Err(item_config_exception(sm, msg));
    let t = t.ok_or_else(null_pointer)?;
    let id = block.id.as_deref();
    let blank_id = java::is_null_or_whitespace(id);
    let eq = |s: &str| java::eq_ignore_case(t, s);
    let mut situated = SituatedType::None;
    let mut world_age = 0;
    let mut selector_string = None;
    let selector_type = if eq("default") {
        if has_parent {
            return err("Default block may not be nested or defined twice.");
        }
        SelectorType::Default
    } else if eq("oncreate") {
        if has_parent {
            return err("OnCreate block may not be nested or defined twice.");
        }
        SelectorType::OnCreate
    } else if eq("situated") {
        if blank_id {
            return err("Block 'Situated' requires a parameter.");
        }
        let id = id.unwrap();
        situated = if java::eq_ignore_case(id, "interior") {
            SituatedType::Interior
        } else if java::eq_ignore_case(id, "exterior") {
            SituatedType::Exterior
        } else if java::eq_ignore_case(id, "shop") {
            SituatedType::Shop
        } else if java::eq_ignore_case(id, "junk") {
            SituatedType::Junk
        } else {
            return err("Block 'Situated' requires a valid parameter.");
        };
        SelectorType::Situated
    } else if let Some((st, label)) = [
        ("zone", (SelectorType::Zone, "Zone")),
        ("vehicle", (SelectorType::Vehicle, "Vehicle")),
        ("room", (SelectorType::Room, "Room")),
        ("container", (SelectorType::Container, "Container")),
        ("tile", (SelectorType::Tile, "Tile")),
    ]
    .iter()
    .find(|(k, _)| eq(k))
    .map(|(_, v)| *v)
    {
        selector_string = id.map(str::to_string);
        if blank_id {
            return err(&format!("Block '{label}' requires a parameter."));
        }
        st
    } else if eq("worldagedays") {
        if blank_id {
            return err("Block 'WorldAgeDays' requires a parameter.");
        }
        world_age = parse_int(id.unwrap())?;
        if world_age < 0 {
            return err("Block 'WorldAgeDays' requires a value greater than zero.");
        }
        SelectorType::WorldAge
    } else {
        if !blank_id {
            sm.warn(&format!(
                "A custom block should not have a parameter, typo in block identifier? Block: {t} {}",
                id.unwrap()
            ));
        }
        SelectorType::None
    };
    if selector_type == SelectorType::Default && !block.children.is_empty() {
        return err("Default block may not have any nested children.");
    }
    if selector_type == SelectorType::OnCreate && !block.children.is_empty() {
        return err("OnCreate block may not have any nested children.");
    }
    let mut bucket = SelectorBucketScript::new(selector_type);
    bucket.selector_situated = Some(situated);
    bucket.selector_world_age = world_age;
    bucket.selector_string = selector_string;
    for value in &block.values {
        if java::is_null_or_whitespace(Some(&value.string)) {
            continue;
        }
        bucket.randomizers.push(java::trim(&value.string).to_string());
        sm.statics.entity.borrow_mut().item_config_error_line = None;
    }
    if selector_type == SelectorType::OnCreate && bucket.randomizers.is_empty() {
        return err("OnCreate block needs at least one randomizer parameter defined.");
    }
    if selector_type == SelectorType::None && !bucket.randomizers.is_empty() {
        return err("A custom container bucket may not have any randomizer parameters.");
    }
    for child in &block.children {
        let child_bucket = load_selector_bucket(sm, child, true)?;
        bucket.children.push(child_bucket);
    }
    Ok(bucket)
}
