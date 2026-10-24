//! `zombie.scripting.itemConfig.script.SelectorBucketScript`: one selector
//! block of an item config root, with its randomizer lines and children.

use super::{SelectorType, SituatedType};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SelectorBucketScript {
    pub(crate) children: Vec<SelectorBucketScript>,
    selector_type: SelectorType,
    pub(crate) selector_string: Option<String>,
    /// `null` until set by the loader.
    pub(crate) selector_situated: Option<SituatedType>,
    pub(crate) selector_world_age: i32,
    pub(crate) randomizers: Vec<String>,
}

impl SelectorBucketScript {
    pub(crate) fn new(selector_type: SelectorType) -> Self {
        SelectorBucketScript {
            children: Vec::new(),
            selector_type,
            selector_string: None,
            selector_situated: None,
            selector_world_age: 0,
            randomizers: Vec::new(),
        }
    }

    /// `getSelectorType`.
    pub fn selector_type(&self) -> SelectorType {
        self.selector_type
    }

    /// `getSelectorString`.
    pub fn selector_string(&self) -> Option<&str> {
        self.selector_string.as_deref()
    }

    /// `getSelectorSituated`.
    pub fn selector_situated(&self) -> Option<SituatedType> {
        self.selector_situated
    }

    /// `getSelectorWorldAge`.
    pub fn selector_world_age(&self) -> i32 {
        self.selector_world_age
    }

    /// `getRandomizers`.
    pub fn randomizers(&self) -> &[String] {
        &self.randomizers
    }

    /// `getChildren`.
    pub fn children(&self) -> &[SelectorBucketScript] {
        &self.children
    }

    /// `copy`: a deep copy.
    pub fn copy(&self) -> SelectorBucketScript {
        self.clone()
    }
}
