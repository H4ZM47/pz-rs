//! The independent `IsoGridSquare.worldObjects` allocation.
//!
//! Membership owns actual whole objects, independently of the main/special
//! lists and item backlinks. Raw ArrayList writes may also store null or other
//! Lua values. Bindings normalize genuine world-object userdata to `Object`;
//! this module never discovers native owners through opaque userdata methods.
use std::cell::RefCell;
use std::rc::Rc;

use pz_kahlua::gc::{RegistrationError, TraceDescriptor, TraceVisitor};
use pz_kahlua::Value;

use crate::IsoObjectRef;

pub type WorldObjectsRef = Rc<RefCell<WorldObjects>>;

/// Cloning an entry retains its runtime referent. It is never an archive copy.
#[derive(Clone)]
pub enum WorldObjectEntry {
    Object(IsoObjectRef),
    /// `Nil` is a Java null list member. Other values remain unmodified.
    Foreign(Value),
}

impl std::fmt::Debug for WorldObjectEntry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Object(owner) => write!(f, "Object@{:p}", Rc::as_ptr(owner)),
            Self::Foreign(value) => f.debug_tuple("Foreign").field(&value.type_name()).finish(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WorldObjectsIndexError {
    pub index: usize,
    pub len: usize,
}

/// Native cast/dereference errors and distinct host borrow conflicts. The latter
/// are not Java null/cast failures and must not be converted into those errors.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WorldObjectsReadError {
    NullEntry { index: usize },
    WrongType { index: usize },
    BorrowedList { index: usize },
    BorrowedObject { index: usize },
    BorrowedFloor { index: usize },
    BorrowedItem { index: usize },
}

impl std::fmt::Display for WorldObjectsReadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NullEntry { index } => write!(f, "NullPointerException: worldObjects[{index}]"),
            Self::WrongType { index } => write!(f, "ClassCastException: worldObjects[{index}]"),
            conflict => write!(f, "worldObjects host borrow conflict: {conflict:?}"),
        }
    }
}
impl std::error::Error for WorldObjectsReadError {}

/// One live backing; no `Clone` or value equality is provided. Returned entries
/// retain runtime owners, while archive copies use their own object map.
#[derive(Debug, Default)]
pub struct WorldObjects {
    entries: Vec<WorldObjectEntry>,
    structural_version: u32,
}

impl WorldObjects {
    pub(crate) fn from_entries(entries: Vec<WorldObjectEntry>) -> Self {
        Self {
            entries,
            structural_version: 0,
        }
    }
    /// Ordinary Rust ownership is conservative until registration succeeds.
    pub fn new_ref() -> WorldObjectsRef {
        Rc::new(RefCell::new(Self::default()))
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
    pub fn structural_version(&self) -> u32 {
        self.structural_version
    }

    /// A temporary runtime pin, not authoritative parallel storage.
    pub fn entry(&self, index: usize) -> Option<WorldObjectEntry> {
        self.entries.get(index).cloned()
    }
    pub fn snapshot_entries(&self) -> Vec<WorldObjectEntry> {
        self.entries.clone()
    }
    pub fn entry_slots(&self) -> impl Iterator<Item = &WorldObjectEntry> {
        self.entries.iter()
    }

    pub fn push_object(&mut self, owner: &IsoObjectRef) {
        self.push_entry(WorldObjectEntry::Object(Rc::clone(owner)));
    }
    pub fn push_entry(&mut self, entry: WorldObjectEntry) {
        self.entries.push(entry);
        self.structural_version = self.structural_version.wrapping_add(1);
    }

    /// Rejected input is returned too, so an owning foreign value cannot be
    /// destroyed inside the caller's list guard on an index error.
    pub fn insert_entry(
        &mut self,
        index: usize,
        entry: WorldObjectEntry,
    ) -> Result<(), (WorldObjectsIndexError, WorldObjectEntry)> {
        if index > self.entries.len() {
            return Err((
                WorldObjectsIndexError {
                    index,
                    len: self.entries.len(),
                },
                entry,
            ));
        }
        self.entries.insert(index, entry);
        self.structural_version = self.structural_version.wrapping_add(1);
        Ok(())
    }

    /// `set` does not change ArrayList's structural version. Move the old (or
    /// rejected incoming) owner outside every enclosing host guard before drop.
    pub fn replace_entry(
        &mut self,
        index: usize,
        entry: WorldObjectEntry,
    ) -> Result<WorldObjectEntry, (WorldObjectsIndexError, WorldObjectEntry)> {
        let len = self.entries.len();
        let Some(slot) = self.entries.get_mut(index) else {
            return Err((WorldObjectsIndexError { index, len }, entry));
        };
        Ok(std::mem::replace(slot, entry))
    }

    pub fn remove_entry(&mut self, index: usize) -> Option<WorldObjectEntry> {
        if index >= self.entries.len() {
            return None;
        }
        self.structural_version = self.structural_version.wrapping_add(1);
        Some(self.entries.remove(index))
    }
    pub fn contains_object(&self, owner: &IsoObjectRef) -> bool {
        self.entries.iter().any(|entry| {
            matches!(entry,
            WorldObjectEntry::Object(candidate) if Rc::ptr_eq(candidate, owner))
        })
    }
    pub fn remove_first_object(&mut self, owner: &IsoObjectRef) -> Option<WorldObjectEntry> {
        let index = self.entries.iter().position(|entry| {
            matches!(entry,
            WorldObjectEntry::Object(candidate) if Rc::ptr_eq(candidate, owner))
        })?;
        self.remove_entry(index)
    }

    /// `clear` changes the structural version even for an empty list. The
    /// caller owns destruction of the returned entries after all host guards.
    pub fn take_entries(&mut self) -> Vec<WorldObjectEntry> {
        self.structural_version = self.structural_version.wrapping_add(1);
        std::mem::take(&mut self.entries)
    }
}

/// Capture-free graph description of this actual list allocation. It never
/// invokes LuaObject methods or reports temporary entry snapshots.
pub struct WorldObjectsDescriptor;
impl TraceDescriptor<WorldObjects> for WorldObjectsDescriptor {
    fn trace(payload: &WorldObjects, visitor: &mut TraceVisitor) {
        for entry in &payload.entries {
            match entry {
                WorldObjectEntry::Object(owner) => visitor.owner(owner),
                WorldObjectEntry::Foreign(value) => visitor.value(value),
            }
        }
    }
    fn detach(payload: &mut WorldObjects) -> Option<Box<dyn std::any::Any>> {
        // Collector detachment is host cleanup, not a native clear/reuse phase.
        Some(Box::new(std::mem::take(&mut payload.entries)))
    }
}

/// Retry on RegistryUnavailable at a later host boundary/cache hit. Conflicts
/// are implementation errors, not reasons to replace or duplicate the owner.
pub fn register_world_objects(owner: &WorldObjectsRef) -> Result<bool, RegistrationError> {
    pz_kahlua::gc::register_existing::<WorldObjects, WorldObjectsDescriptor>(owner)
}

#[cfg(test)]
mod tests;
