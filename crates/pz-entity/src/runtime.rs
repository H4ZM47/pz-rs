//! Canonical component data owners, separate from the legacy value codecs.
//!
//! A live component has one stable allocation and one authoritative body.
//! Membership operations below are deliberately storage operations: they do
//! not transfer a component from its previous owner, invoke lifecycle hooks,
//! register entities, reset components, or implement native pools. Those
//! operations require a host coordinator with no entity/component/world
//! borrow held across callbacks.
//!
//! Encoded entries are an explicit raw-import boundary. Native `loadEntity`
//! skips/logs certain bad blocks; it does **not** retain them as raw entries.
//! The legacy bounded value decoder is therefore not used as a live loader.

use std::any::Any;
use std::cell::RefCell;
use std::fmt;
use std::rc::Rc;

use crate::components::*;
use crate::context::EntityContext;
use crate::entity::{Component, Components};
use crate::fluids::FluidContainer;
use crate::io::{EntityError, Reader, Writer};
use crate::ComponentType;

/// The actual component allocation. Cloning this reference aliases the same
/// body; it is never an archive copy.
pub type ComponentRef = Rc<RefCell<RuntimeComponent>>;

/// One strong reference directly to an actual entity allocation.
///
/// The erased allocation is `RefCell<T>` itself, not an owning adapter,
/// userdata wrapper, locator, or weak substitute. This avoids a dependency
/// from `pz-entity` back to inventory/world while preserving physical edges.
pub struct EntityOwner {
    allocation: Rc<dyn Any>,
}

impl EntityOwner {
    pub fn from_ref<T: 'static>(owner: &Rc<RefCell<T>>) -> Self {
        let typed: Rc<RefCell<T>> = Rc::clone(owner);
        let allocation: Rc<dyn Any> = typed;
        Self { allocation }
    }

    pub fn is<T: 'static>(&self) -> bool {
        self.allocation.is::<RefCell<T>>()
    }

    pub fn downcast_ref<T: 'static>(&self) -> Option<&RefCell<T>> {
        self.allocation.downcast_ref::<RefCell<T>>()
    }

    /// Ordinary access may clone the actual owner reference. GC tracing must
    /// instead borrow [`Self::erased_slot`] without cloning it.
    pub fn try_handle<T: 'static>(&self) -> Option<Rc<RefCell<T>>> {
        Rc::clone(&self.allocation).downcast::<RefCell<T>>().ok()
    }

    pub fn erased_slot(&self) -> &Rc<dyn Any> {
        &self.allocation
    }

    pub fn ptr_eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.allocation, &other.allocation)
    }
}

impl Clone for EntityOwner {
    /// Copies the strong reference to the same entity, not its body.
    fn clone(&self) -> Self {
        Self {
            allocation: Rc::clone(&self.allocation),
        }
    }
}

impl fmt::Debug for EntityOwner {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("EntityOwner")
            .field("allocation", &(Rc::as_ptr(&self.allocation) as *const ()))
            .finish()
    }
}

/// Failures of this data-owner API. Host borrow/unsupported-state failures
/// must not be mistaken for a native, caught per-block codec exception.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RuntimeError {
    TooManyComponents,
    DuplicateType(i16),
    UnsupportedLive(ComponentType),
    InvalidBody(ComponentType),
    BorrowedComponent {
        index: Option<usize>,
        kind: ComponentType,
    },
    NeedsPromotion {
        kind: ComponentType,
        world_version: i32,
    },
    UnknownEncodedType(i16),
    EncodedVersionMismatch {
        type_id: i16,
        stored: i32,
        requested: i32,
    },
    UnreadEncodedBytes {
        kind: ComponentType,
        remaining: usize,
    },
    Codec(EntityError),
}

impl fmt::Display for RuntimeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooManyComponents => f.write_str("Maximum component size '127' reached"),
            Self::DuplicateType(id) => write!(f, "Duplicate component type id {id}"),
            Self::UnsupportedLive(k) => {
                write!(f, "{} requires an explicit encoded entry", k.name())
            }
            Self::InvalidBody(k) => write!(f, "Invalid live body for {}", k.name()),
            Self::BorrowedComponent { index, kind } => {
                write!(f, "{} component at {index:?} is borrowed", kind.name())
            }
            Self::NeedsPromotion {
                kind,
                world_version,
            } => write!(f, "{} is encoded for version {world_version}", kind.name()),
            Self::UnknownEncodedType(id) => write!(f, "Unknown encoded component type id {id}"),
            Self::EncodedVersionMismatch {
                type_id,
                stored,
                requested,
            } => write!(
                f,
                "Encoded component {type_id} version {stored} cannot be written as {requested}"
            ),
            Self::UnreadEncodedBytes { kind, remaining } => write!(
                f,
                "{} encoded payload has {remaining} unread bytes",
                kind.name()
            ),
            Self::Codec(e) => e.fmt(f),
        }
    }
}

impl std::error::Error for RuntimeError {}

impl From<EntityError> for RuntimeError {
    fn from(value: EntityError) -> Self {
        Self::Codec(value)
    }
}

/// A rejected value is returned intact; callers must drop it after their
/// enclosing guards. None of these failures runs lifecycle or resets.
#[derive(Debug)]
#[must_use]
pub struct RejectedComponent {
    pub error: RuntimeError,
    pub component: Component,
}

#[derive(Debug)]
#[must_use]
pub struct RejectedComponentRef {
    pub error: RuntimeError,
    pub component: ComponentRef,
}

#[derive(Debug)]
#[must_use]
pub struct RejectedValues {
    pub error: RuntimeError,
    pub values: Components,
}

#[derive(Debug)]
#[must_use]
pub struct RejectedEncoded {
    pub error: RuntimeError,
    pub entries: Vec<EncodedComponent>,
}

/// A mutable view cannot replace the enum variant or alter class identity.
pub enum ComponentBodyMut<'a> {
    Attributes(&'a mut AttributeContainer),
    FluidContainer(&'a mut FluidContainer),
    SpriteConfig(&'a mut SpriteConfig),
    Script(&'a mut EntityScriptInfo),
    UiConfig(&'a mut UiConfig),
    CraftLogic(&'a mut CraftLogic),
    DryingCraftLogic(&'a mut DryingCraftLogic),
    MetaTag(&'a mut MetaTag),
    Resources(&'a mut Resources),
    CraftBench(&'a mut CraftBench),
    Durability(&'a mut Durability),
    SpriteOverlayConfig(&'a mut SpriteOverlayConfig),
    Empty,
}

/// Actual mutable component state. The kind and body variant stay fixed for
/// the lifetime of this allocation; the owner is a real, strong backedge.
#[derive(Debug)]
pub struct RuntimeComponent {
    kind: ComponentType,
    body: Component,
    owner: Option<EntityOwner>,
}

impl RuntimeComponent {
    pub fn new_unowned(body: Component) -> Result<ComponentRef, RejectedComponent> {
        if let Err(error) = validate_live(&body) {
            return Err(RejectedComponent {
                error,
                component: body,
            });
        }
        Ok(Rc::new(RefCell::new(Self {
            kind: body.kind(),
            body,
            owner: None,
        })))
    }

    pub fn kind(&self) -> ComponentType {
        self.kind
    }

    pub fn body(&self) -> &Component {
        &self.body
    }

    pub fn typed_mut(&mut self) -> ComponentBodyMut<'_> {
        match &mut self.body {
            Component::Attributes(c) => ComponentBodyMut::Attributes(c),
            Component::FluidContainer(c) => ComponentBodyMut::FluidContainer(c),
            Component::SpriteConfig(c) => ComponentBodyMut::SpriteConfig(c),
            Component::Script(c) => ComponentBodyMut::Script(c),
            Component::UiConfig(c) => ComponentBodyMut::UiConfig(c),
            Component::CraftLogic(c) => ComponentBodyMut::CraftLogic(c),
            Component::DryingCraftLogic(c) => ComponentBodyMut::DryingCraftLogic(c),
            Component::MetaTag(c) => ComponentBodyMut::MetaTag(c),
            Component::Resources(c) => ComponentBodyMut::Resources(c),
            Component::CraftBench(c) => ComponentBodyMut::CraftBench(c),
            Component::Durability(c) => ComponentBodyMut::Durability(c),
            Component::SpriteOverlayConfig(c) => ComponentBodyMut::SpriteOverlayConfig(c),
            Component::Empty(_) => ComponentBodyMut::Empty,
            Component::Opaque { .. } => unreachable!("opaque bodies are never live"),
        }
    }

    pub fn fluid(&self) -> Option<&FluidContainer> {
        match &self.body {
            Component::FluidContainer(c) => Some(c),
            _ => None,
        }
    }

    pub fn fluid_mut(&mut self) -> Option<&mut FluidContainer> {
        match &mut self.body {
            Component::FluidContainer(c) => Some(c),
            _ => None,
        }
    }

    pub fn owner_slot(&self) -> Option<&EntityOwner> {
        self.owner.as_ref()
    }

    /// Storage-only owner assignment. The returned former owner MUST remain
    /// alive until enclosing entity/component/world/sprite guards end.
    /// Native logging, old-owner removal and hooks belong to the host.
    #[must_use]
    pub fn replace_owner(&mut self, new: Option<&EntityOwner>) -> Option<EntityOwner> {
        std::mem::replace(&mut self.owner, new.cloned())
    }

    /// Decode into the SAME body using the caller's SAME reader. No bounded
    /// block subreader, rollback, owner mutation, hooks or reset is performed.
    /// `ctx` must be a reviewed, callback-free lookup implementation while a
    /// component RefCell guard is held. Errors preserve partial body effects.
    pub fn load_payload_in_place(
        &mut self,
        reader: &mut Reader<'_>,
        world_version: i32,
        ctx: &dyn EntityContext,
    ) -> Result<(), RuntimeError> {
        self.body
            .load(reader, world_version, ctx)
            .map_err(Into::into)
    }

    pub fn write_payload(&self, writer: &mut Writer) -> Result<(), RuntimeError> {
        self.body.save(writer);
        Ok(())
    }

    /// Named value copy. Current DTO bodies contain only owned data, not
    /// Rc/native/Lua payloads; the actual owner backedge is never followed.
    pub fn try_archive_copy_unowned(&self) -> Result<ComponentRef, RuntimeError> {
        Self::new_unowned(self.body.clone()).map_err(|rejected| rejected.error)
    }
}

fn validate_live(body: &Component) -> Result<(), RuntimeError> {
    match body {
        Component::Opaque { kind, .. } => Err(RuntimeError::UnsupportedLive(*kind)),
        Component::Empty(kind)
            if !matches!(
                kind,
                ComponentType::Lua
                    | ComponentType::Parts
                    | ComponentType::Signals
                    | ComponentType::TestComponent
                    | ComponentType::CraftRecipe
                    | ComponentType::ContextMenuConfig
                    | ComponentType::CraftBenchSounds
                    | ComponentType::WallCoveringConfig
            ) =>
        {
            Err(RuntimeError::InvalidBody(*kind))
        }
        // Exhaustive matching forces future typed bodies to revisit this
        // data-only archive/destructor contract.
        Component::Attributes(_)
        | Component::FluidContainer(_)
        | Component::SpriteConfig(_)
        | Component::Script(_)
        | Component::UiConfig(_)
        | Component::CraftLogic(_)
        | Component::DryingCraftLogic(_)
        | Component::MetaTag(_)
        | Component::Resources(_)
        | Component::CraftBench(_)
        | Component::Durability(_)
        | Component::SpriteOverlayConfig(_)
        | Component::Empty(_) => Ok(()),
    }
}

/// Sole unpromoted payload, excluding the type-id and block-length framing.
/// This is an explicit carrier, not native unknown-block recovery. It has no
/// editable live/cache counterpart. Unknown ids can remain encoded.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EncodedComponent {
    type_id: i16,
    payload: Vec<u8>,
    world_version: i32,
}

impl EncodedComponent {
    pub fn new(type_id: i16, payload: Vec<u8>, world_version: i32) -> Self {
        Self {
            type_id,
            payload,
            world_version,
        }
    }
    pub fn type_id(&self) -> i16 {
        self.type_id
    }
    pub fn payload(&self) -> &[u8] {
        &self.payload
    }
    pub fn world_version(&self) -> i32 {
        self.world_version
    }
}

#[derive(Debug)]
enum RuntimeEntry {
    Live {
        kind: ComponentType,
        component: ComponentRef,
    },
    Encoded(EncodedComponent),
}

impl RuntimeEntry {
    fn stored_type_id(&self) -> i16 {
        match self {
            Self::Live { kind, .. } => kind.id(),
            Self::Encoded(c) => c.type_id,
        }
    }
}

/// Borrowed PHYSICAL slots for tracing. Do not call a cloning getter or
/// report transitive owner edges in place of these real fields.
#[derive(Clone, Copy, Debug)]
pub enum RuntimeEntryView<'a> {
    Live {
        kind: ComponentType,
        component: &'a ComponentRef,
    },
    Encoded(&'a EncodedComponent),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EntryPresence {
    Absent,
    Live,
    Encoded { world_version: i32 },
}

/// Detached owning slots. Keep this value in an outer retirement sink and
/// drop it only after all relevant RefCell/WORLD/sprite guards end. No Drop
/// implementation invokes hooks, clears another live object, or pools data.
#[derive(Debug, Default)]
#[must_use]
pub struct RetiredComponents {
    entries: Vec<RuntimeEntry>,
    owners: Vec<EntityOwner>,
    components: Vec<ComponentRef>,
}

impl RetiredComponents {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty() && self.owners.is_empty() && self.components.is_empty()
    }
    pub fn len(&self) -> usize {
        self.entries.len() + self.owners.len() + self.components.len()
    }
    pub fn push_owner(&mut self, owner: EntityOwner) {
        self.owners.push(owner);
    }
    pub fn push_component(&mut self, component: ComponentRef) {
        self.components.push(component);
    }
    pub fn append(&mut self, mut other: Self) {
        self.entries.append(&mut other.entries);
        self.owners.append(&mut other.owners);
        self.components.append(&mut other.components);
    }
}

#[derive(Debug)]
#[must_use]
pub struct Promotion {
    pub component: ComponentRef,
    pub promoted: bool,
    pub retired: RetiredComponents,
}

/// Sole inline component authority on an entity. There is intentionally no
/// store-level owner reference, shadow bytes cache, Clone, or PartialEq.
#[derive(Debug, Default)]
pub struct RuntimeComponents {
    entries: Vec<RuntimeEntry>,
}

impl RuntimeComponents {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn len(&self) -> usize {
        self.entries.len()
    }
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
    pub fn requires_save(&self) -> bool {
        !self.is_empty()
    }

    /// Moves factory values directly, preserving unsaved/script-only fields
    /// and exact numeric data. Validate ALL entries before consuming any.
    pub fn adopt_values_unowned(values: Components) -> Result<Self, RejectedValues> {
        let validation = (|| {
            if values.len() > 127 {
                return Err(RuntimeError::TooManyComponents);
            }
            let mut seen = Vec::with_capacity(values.len());
            for component in &values.list {
                validate_live(component)?;
                let id = component.kind().id();
                if seen.contains(&id) {
                    return Err(RuntimeError::DuplicateType(id));
                }
                seen.push(id);
            }
            Ok(())
        })();
        if let Err(error) = validation {
            return Err(RejectedValues { error, values });
        }
        let entries = values
            .list
            .into_iter()
            .map(|body| {
                let kind = body.kind();
                RuntimeEntry::Live {
                    kind,
                    component: Rc::new(RefCell::new(RuntimeComponent {
                        kind,
                        body,
                        owner: None,
                    })),
                }
            })
            .collect();
        Ok(Self { entries })
    }

    /// Explicit import, with duplicate ids rejected before consuming input.
    /// Import does not decode, invoke native load, or invent unknown classes.
    pub fn from_encoded_unowned(entries: Vec<EncodedComponent>) -> Result<Self, RejectedEncoded> {
        let error = if entries.len() > 127 {
            Some(RuntimeError::TooManyComponents)
        } else {
            let mut seen = Vec::with_capacity(entries.len());
            entries.iter().find_map(|c| {
                if seen.contains(&c.type_id) {
                    Some(RuntimeError::DuplicateType(c.type_id))
                } else {
                    seen.push(c.type_id);
                    None
                }
            })
        };
        if let Some(error) = error {
            return Err(RejectedEncoded { error, entries });
        }
        Ok(Self {
            entries: entries.into_iter().map(RuntimeEntry::Encoded).collect(),
        })
    }

    pub fn entry_slots(&self) -> impl Iterator<Item = RuntimeEntryView<'_>> {
        self.entries.iter().map(|entry| match entry {
            RuntimeEntry::Live { kind, component } => RuntimeEntryView::Live {
                kind: *kind,
                component,
            },
            RuntimeEntry::Encoded(encoded) => RuntimeEntryView::Encoded(encoded),
        })
    }

    pub fn presence(&self, kind: ComponentType) -> EntryPresence {
        match self
            .entries
            .iter()
            .find(|entry| entry.stored_type_id() == kind.id())
        {
            Some(RuntimeEntry::Live { .. }) => EntryPresence::Live,
            Some(RuntimeEntry::Encoded(c)) => EntryPresence::Encoded {
                world_version: c.world_version,
            },
            None => EntryPresence::Absent,
        }
    }

    pub fn get_ref(&self, kind: ComponentType) -> Result<Option<ComponentRef>, RuntimeError> {
        match self
            .entries
            .iter()
            .position(|entry| entry.stored_type_id() == kind.id())
        {
            Some(index) => self.get_ref_at(index),
            None => Ok(None),
        }
    }

    /// Returning the actual handle does not borrow its body. Retained handles
    /// therefore work while a caller has a body guard, unlike fake snapshots.
    pub fn get_ref_at(&self, index: usize) -> Result<Option<ComponentRef>, RuntimeError> {
        match self.entries.get(index) {
            Some(RuntimeEntry::Live { component, .. }) => Ok(Some(Rc::clone(component))),
            Some(RuntimeEntry::Encoded(c)) => Err(RuntimeError::NeedsPromotion {
                kind: ComponentType::from_id(c.type_id),
                world_version: c.world_version,
            }),
            None => Ok(None),
        }
    }

    /// Membership only; incoming ref is returned intact on rejection. Owner,
    /// old-owner membership, callbacks, pool and entity registration untouched.
    pub fn push_ref_if_absent(
        &mut self,
        component: ComponentRef,
    ) -> Result<(), RejectedComponentRef> {
        let kind = match component.try_borrow().map(|body| body.kind) {
            Ok(kind) => kind,
            Err(_) => {
                return Err(RejectedComponentRef {
                    error: RuntimeError::BorrowedComponent {
                        index: None,
                        kind: ComponentType::Undefined,
                    },
                    component,
                })
            }
        };
        let error = if self.entries.len() >= 127 {
            Some(RuntimeError::TooManyComponents)
        } else if self
            .entries
            .iter()
            .any(|entry| entry.stored_type_id() == kind.id())
        {
            Some(RuntimeError::DuplicateType(kind.id()))
        } else {
            None
        };
        if let Some(error) = error {
            return Err(RejectedComponentRef { error, component });
        }
        self.entries.push(RuntimeEntry::Live { kind, component });
        Ok(())
    }

    /// Native storage is unordered. Return the actual removed slot and leave
    /// its owner untouched for the host's subsequent onRemoved phase.
    pub fn remove_type_unordered(
        &mut self,
        kind: ComponentType,
    ) -> Result<Option<ComponentRef>, RuntimeError> {
        let Some(index) = self
            .entries
            .iter()
            .position(|entry| entry.stored_type_id() == kind.id())
        else {
            return Ok(None);
        };
        if let RuntimeEntry::Encoded(c) = &self.entries[index] {
            return Err(RuntimeError::NeedsPromotion {
                kind,
                world_version: c.world_version,
            });
        }
        match self.entries.swap_remove(index) {
            RuntimeEntry::Live { component, .. } => Ok(Some(component)),
            RuntimeEntry::Encoded(_) => unreachable!(),
        }
    }

    pub fn remove_exact_unordered(&mut self, component: &ComponentRef) -> Option<ComponentRef> {
        let index = self.entries.iter().position(|entry| matches!(entry, RuntimeEntry::Live { component: current, .. } if Rc::ptr_eq(current, component)))?;
        match self.entries.swap_remove(index) {
            RuntimeEntry::Live { component, .. } => Some(component),
            RuntimeEntry::Encoded(_) => unreachable!(),
        }
    }

    pub fn take_entries(&mut self) -> RetiredComponents {
        RetiredComponents {
            entries: std::mem::take(&mut self.entries),
            ..Default::default()
        }
    }

    /// Explicit raw import promotion only, with an intentionally bounded
    /// payload reader. This is NOT the native shared-ByteBuffer load path.
    /// Failure leaves the sole encoded entry exactly unchanged. The current
    /// temporary DTO body is pure owned data and has no owner/host payload.
    pub fn promote_encoded(
        &mut self,
        kind: ComponentType,
        ctx: &dyn EntityContext,
    ) -> Result<Promotion, RuntimeError> {
        let Some(index) = self
            .entries
            .iter()
            .position(|entry| entry.stored_type_id() == kind.id())
        else {
            return Err(RuntimeError::UnknownEncodedType(kind.id()));
        };
        let encoded = match &self.entries[index] {
            RuntimeEntry::Live { component, .. } => {
                return Ok(Promotion {
                    component: Rc::clone(component),
                    promoted: false,
                    retired: RetiredComponents::default(),
                })
            }
            RuntimeEntry::Encoded(c) => c,
        };
        let mut body =
            Component::create(kind).ok_or(RuntimeError::UnknownEncodedType(encoded.type_id))?;
        validate_live(&body)?;
        let mut reader = Reader::new(&encoded.payload);
        body.load(&mut reader, encoded.world_version, ctx)?;
        if reader.remaining() != 0 {
            return Err(RuntimeError::UnreadEncodedBytes {
                kind,
                remaining: reader.remaining(),
            });
        }
        let component = Rc::new(RefCell::new(RuntimeComponent {
            kind,
            body,
            owner: None,
        }));
        let old = std::mem::replace(
            &mut self.entries[index],
            RuntimeEntry::Live {
                kind,
                component: Rc::clone(&component),
            },
        );
        Ok(Promotion {
            component,
            promoted: true,
            retired: RetiredComponents {
                entries: vec![old],
                ..Default::default()
            },
        })
    }

    pub fn try_archive_copy_unowned(&self) -> Result<Self, RuntimeError> {
        let mut entries = Vec::with_capacity(self.len());
        for (index, entry) in self.entries.iter().enumerate() {
            entries.push(match entry {
                RuntimeEntry::Live { kind, component } => {
                    let body =
                        component
                            .try_borrow()
                            .map_err(|_| RuntimeError::BorrowedComponent {
                                index: Some(index),
                                kind: *kind,
                            })?;
                    RuntimeEntry::Live {
                        kind: *kind,
                        component: body.try_archive_copy_unowned()?,
                    }
                }
                RuntimeEntry::Encoded(c) => RuntimeEntry::Encoded(c.clone()),
            });
        }
        Ok(Self { entries })
    }

    /// Explicit detached DTO snapshot for legacy archive consumers. It is
    /// not a getter implementation or a second editable component authority.
    pub fn try_values_snapshot(&self) -> Result<Components, RuntimeError> {
        let mut values = Components::new();
        for (index, entry) in self.entries.iter().enumerate() {
            match entry {
                RuntimeEntry::Live { kind, component } => values.list.push(
                    component
                        .try_borrow()
                        .map_err(|_| RuntimeError::BorrowedComponent {
                            index: Some(index),
                            kind: *kind,
                        })?
                        .body
                        .clone(),
                ),
                RuntimeEntry::Encoded(c) => {
                    return Err(RuntimeError::NeedsPromotion {
                        kind: ComponentType::from_id(c.type_id),
                        world_version: c.world_version,
                    })
                }
            }
        }
        Ok(values)
    }

    /// Compare detached data semantics, excluding actual owner links.
    /// Encoded entries compare id/bytes/version without promotion. A live
    /// entry and an encoded entry are distinct representations; this does
    /// not decode one speculatively or substitute pointer equality for data
    /// equality (including the existing floating-point NaN semantics).
    pub fn try_data_eq(&self, other: &Self) -> Result<bool, RuntimeError> {
        if self.len() != other.len() {
            return Ok(false);
        }
        for (index, (left, right)) in self.entries.iter().zip(&other.entries).enumerate() {
            match (left, right) {
                (
                    RuntimeEntry::Live {
                        kind: left_kind,
                        component: left,
                    },
                    RuntimeEntry::Live {
                        kind: right_kind,
                        component: right,
                    },
                ) => {
                    if left_kind != right_kind {
                        return Ok(false);
                    }
                    let left = left
                        .try_borrow()
                        .map_err(|_| RuntimeError::BorrowedComponent {
                            index: Some(index),
                            kind: *left_kind,
                        })?;
                    let right =
                        right
                            .try_borrow()
                            .map_err(|_| RuntimeError::BorrowedComponent {
                                index: Some(index),
                                kind: *right_kind,
                            })?;
                    if left.body != right.body {
                        return Ok(false);
                    }
                }
                (RuntimeEntry::Encoded(left), RuntimeEntry::Encoded(right)) => {
                    if left != right {
                        return Ok(false);
                    }
                }
                _ => return Ok(false),
            }
        }
        Ok(true)
    }

    /// Encode current live bodies directly, preserving framing and the
    /// writer's completed prefix on failure. Encoded bytes may be forwarded
    /// only to their explicitly recorded world version. No state is reloaded.
    pub fn write_to(&self, writer: &mut Writer, world_version: i32) -> Result<(), RuntimeError> {
        writer.i8(self.len() as i8);
        for (index, entry) in self.entries.iter().enumerate() {
            let start = writer.block_start();
            writer.i16(entry.stored_type_id());
            match entry {
                RuntimeEntry::Live { kind, component } => component
                    .try_borrow()
                    .map_err(|_| RuntimeError::BorrowedComponent {
                        index: Some(index),
                        kind: *kind,
                    })?
                    .write_payload(writer)?,
                RuntimeEntry::Encoded(c) => {
                    if c.world_version != world_version {
                        return Err(RuntimeError::EncodedVersionMismatch {
                            type_id: c.type_id,
                            stored: c.world_version,
                            requested: world_version,
                        });
                    }
                    writer.bytes(&c.payload);
                }
            }
            writer.block_end(start);
        }
        Ok(())
    }

    pub fn save(&self, world_version: i32) -> Result<Vec<u8>, RuntimeError> {
        let mut writer = Writer::new();
        self.write_to(&mut writer, world_version)?;
        Ok(writer.buf)
    }
}

#[cfg(test)]
mod tests;
