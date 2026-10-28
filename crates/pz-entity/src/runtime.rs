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
use crate::runtime_attributes::RuntimeAttributes;
use crate::runtime_durability::RuntimeDurability;
use crate::runtime_fluid::{
    FluidColorRef, FluidPropertiesRef, FluidRecalcOutcome, RuntimeFluidState,
};
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
    BorrowedTransient(&'static str),
    UnsupportedRuntime(&'static str),
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
            Self::BorrowedTransient(name) => write!(f, "{name} is borrowed"),
            Self::UnsupportedRuntime(message) => f.write_str(message),
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
    Attributes(&'a mut RuntimeAttributes),
    FluidContainer(&'a mut FluidContainer),
    SpriteConfig(&'a mut SpriteConfig),
    Script(&'a mut EntityScriptInfo),
    UiConfig(&'a mut UiConfig),
    CraftLogic(&'a mut CraftLogic),
    DryingCraftLogic(&'a mut DryingCraftLogic),
    MetaTag(&'a mut MetaTag),
    Resources(&'a mut Resources),
    CraftBench(&'a mut CraftBench),
    Durability(&'a mut RuntimeDurability),
    SpriteOverlayConfig(&'a mut SpriteOverlayConfig),
    Empty,
}

/// The actual body has one authority. Attributes owns real instance slots;
/// Durability owns its nullable native fields. Neither live allocation keeps
/// an editable legacy DTO alongside its actual state.
#[derive(Debug)]
pub enum RuntimeBody {
    Attributes(RuntimeAttributes),
    Durability(RuntimeDurability),
    Data(Component),
}

/// Actual mutable component state. The kind and body variant stay fixed for
/// the lifetime of this allocation; the owner is a real, strong backedge.
#[derive(Debug)]
pub struct RuntimeComponent {
    kind: ComponentType,
    body: RuntimeBody,
    owner: Option<EntityOwner>,
    fluid_runtime: Option<RuntimeFluidState>,
    script_cache: Option<EntityOwner>,
}

impl RuntimeComponent {
    pub fn new_unowned(body: Component) -> Result<ComponentRef, RejectedComponent> {
        if let Err(error) = validate_live(&body) {
            return Err(RejectedComponent {
                error,
                component: body,
            });
        }
        let default_color = if matches!(body, Component::FluidContainer(_)) {
            match crate::runtime_fluid::copy_default_color() {
                Ok(color) => Some(color),
                Err(error) => {
                    return Err(RejectedComponent {
                        error,
                        component: body,
                    })
                }
            }
        } else {
            None
        };
        Ok(Rc::new(RefCell::new(Self::from_validated_body(
            body,
            default_color,
        ))))
    }

    fn from_validated_body(body: Component, default_color: Option<crate::color::Rgba>) -> Self {
        let kind = body.kind();
        let fluid_runtime = match &body {
            Component::FluidContainer(fluid) => Some(RuntimeFluidState::from_value(
                fluid,
                default_color.expect("fluid constructor default color"),
            )),
            _ => None,
        };
        let body = match body {
            Component::Attributes(value) => {
                RuntimeBody::Attributes(RuntimeAttributes::from_archive(value))
            }
            Component::Durability(value) => {
                RuntimeBody::Durability(RuntimeDurability::from_archive(value))
            }
            value => RuntimeBody::Data(value),
        };
        Self {
            kind,
            body,
            owner: None,
            fluid_runtime,
            script_cache: None,
        }
    }

    pub fn kind(&self) -> ComponentType {
        self.kind
    }

    pub fn body(&self) -> &RuntimeBody {
        &self.body
    }

    pub fn value_snapshot(&self) -> Result<Component, RuntimeError> {
        match &self.body {
            RuntimeBody::Attributes(attributes) => {
                Ok(Component::Attributes(attributes.value_snapshot()?))
            }
            RuntimeBody::Durability(value) => Ok(Component::Durability(value.value_snapshot()?)),
            RuntimeBody::Data(value) => Ok(value.clone()),
        }
    }

    pub fn script_info(&self) -> Option<&EntityScriptInfo> {
        match &self.body {
            RuntimeBody::Data(Component::Script(value)) => Some(value),
            _ => None,
        }
    }

    pub fn attributes(&self) -> Option<&RuntimeAttributes> {
        match &self.body {
            RuntimeBody::Attributes(value) => Some(value),
            _ => None,
        }
    }
    pub fn attributes_mut(&mut self) -> Option<&mut RuntimeAttributes> {
        match &mut self.body {
            RuntimeBody::Attributes(value) => Some(value),
            _ => None,
        }
    }

    pub fn durability(&self) -> Option<&RuntimeDurability> {
        match &self.body {
            RuntimeBody::Durability(value) => Some(value),
            _ => None,
        }
    }
    pub fn durability_mut(&mut self) -> Option<&mut RuntimeDurability> {
        match &mut self.body {
            RuntimeBody::Durability(value) => Some(value),
            _ => None,
        }
    }
    pub fn read_durability_script(
        &mut self,
        script: &pz_scripts::objects::entity::components::combat::DurabilityScript,
    ) -> Result<(), RuntimeError> {
        let kind = self.kind;
        self.durability_mut()
            .ok_or(RuntimeError::InvalidBody(kind))?
            .read_from_script(script);
        Ok(())
    }

    pub fn typed_mut(&mut self) -> ComponentBodyMut<'_> {
        match &mut self.body {
            RuntimeBody::Attributes(c) => ComponentBodyMut::Attributes(c),
            RuntimeBody::Durability(c) => ComponentBodyMut::Durability(c),
            RuntimeBody::Data(Component::FluidContainer(c)) => {
                c.cache_invalidated = true;
                ComponentBodyMut::FluidContainer(c)
            }
            RuntimeBody::Data(Component::SpriteConfig(c)) => ComponentBodyMut::SpriteConfig(c),
            RuntimeBody::Data(Component::Script(c)) => ComponentBodyMut::Script(c),
            RuntimeBody::Data(Component::UiConfig(c)) => ComponentBodyMut::UiConfig(c),
            RuntimeBody::Data(Component::CraftLogic(c)) => ComponentBodyMut::CraftLogic(c),
            RuntimeBody::Data(Component::DryingCraftLogic(c)) => {
                ComponentBodyMut::DryingCraftLogic(c)
            }
            RuntimeBody::Data(Component::MetaTag(c)) => ComponentBodyMut::MetaTag(c),
            RuntimeBody::Data(Component::Resources(c)) => ComponentBodyMut::Resources(c),
            RuntimeBody::Data(Component::CraftBench(c)) => ComponentBodyMut::CraftBench(c),
            RuntimeBody::Data(Component::SpriteOverlayConfig(c)) => {
                ComponentBodyMut::SpriteOverlayConfig(c)
            }
            RuntimeBody::Data(Component::Empty(_)) => ComponentBodyMut::Empty,
            RuntimeBody::Data(
                Component::Opaque { .. } | Component::Attributes(_) | Component::Durability(_),
            ) => {
                unreachable!("invalid actual data body")
            }
        }
    }

    pub fn fluid(&self) -> Option<&FluidContainer> {
        match &self.body {
            RuntimeBody::Data(Component::FluidContainer(c)) => Some(c),
            _ => None,
        }
    }

    pub fn fluid_mut(&mut self) -> Option<&mut FluidContainer> {
        match &mut self.body {
            RuntimeBody::Data(Component::FluidContainer(c)) => {
                c.cache_invalidated = true;
                Some(c)
            }
            _ => None,
        }
    }

    pub fn fluid_runtime(&self) -> Option<&RuntimeFluidState> {
        self.fluid_runtime.as_ref()
    }
    pub fn fluid_runtime_mut(&mut self) -> Option<&mut RuntimeFluidState> {
        self.fluid_runtime.as_mut()
    }
    /// Borrowed physical final cache allocations; never cloned trace views.
    pub fn fluid_color_slot(&self) -> Option<&FluidColorRef> {
        self.fluid_runtime
            .as_ref()
            .map(RuntimeFluidState::color_slot)
    }
    pub fn fluid_properties_slot(&self) -> Option<&FluidPropertiesRef> {
        self.fluid_runtime
            .as_ref()
            .map(RuntimeFluidState::properties_slot)
    }

    /// Native cache data phase only. Dispatch the returned actual owner's
    /// dirty/notification phase after releasing every component/entity guard.
    /// Pure lookup contexts only; no Lua callback is permitted here.
    pub fn fluid_recalculate_data(
        &mut self,
        ctx: &dyn EntityContext,
        force: bool,
    ) -> Result<FluidRecalcOutcome, RuntimeError> {
        let RuntimeBody::Data(Component::FluidContainer(body)) = &mut self.body else {
            return Err(RuntimeError::InvalidBody(self.kind));
        };
        self.fluid_runtime
            .as_mut()
            .expect("fluid runtime matches immutable kind")
            .recalculate(body, ctx, force, self.owner.as_ref())
    }

    pub fn fluid_empty_data(
        &mut self,
        ctx: &dyn EntityContext,
    ) -> Result<FluidRecalcOutcome, RuntimeError> {
        let RuntimeBody::Data(Component::FluidContainer(body)) = &mut self.body else {
            return Err(RuntimeError::InvalidBody(self.kind));
        };
        self.fluid_runtime
            .as_mut()
            .expect("fluid runtime matches immutable kind")
            .empty(body, ctx, self.owner.as_ref())
    }

    /// A factory/data import initializes actual caches before publishing the
    /// owner. Native loads use fluid_recalculate_data(force=true) after their
    /// successful body read and dispatch its real owner phase separately.
    pub fn initialize_fluid_data(&mut self, ctx: &dyn EntityContext) -> Result<(), RuntimeError> {
        if self.owner.is_some() {
            return Err(RuntimeError::UnsupportedRuntime(
                "FluidContainer initialization requires an unowned component",
            ));
        }
        let outcome = self.fluid_recalculate_data(ctx, true)?;
        debug_assert!(outcome.owner.is_none());
        Ok(())
    }

    /// Actual FluidContainer.readFromScript on a pooled, detached receiver.
    /// It deliberately does NOT replace the body or empty existing fluids:
    /// retained handles can mutate a component while it is held by the pool.
    /// The context and RNG must be reviewed callback-free implementations.
    /// `script_name` is the actual ComponentScript.getName result supplied by
    /// the host, rather than an item type or display name. The host emits all
    /// notices outside its guards on both success and failure.
    pub fn read_fluid_script(
        &mut self,
        script: &pz_scripts::objects::entity::components::fluids::FluidContainerScript,
        ctx: &dyn EntityContext,
        rng: &mut dyn crate::factory::FactoryRng,
        script_name: &str,
        native_notices: &mut Vec<crate::runtime_fluid::RuntimeNativeNotice>,
    ) -> Result<(), RuntimeError> {
        if self.owner.is_some() {
            return Err(RuntimeError::UnsupportedRuntime(
                "Fluid script factory requires a detached component",
            ));
        }
        let RuntimeBody::Data(Component::FluidContainer(body)) = &mut self.body else {
            return Err(RuntimeError::InvalidBody(self.kind));
        };
        body.whitelist = script
            .whitelist()
            .map(|filter| crate::factory::filter_from_script(filter, ctx));
        body.blacklist = script
            .blacklist()
            .map(|filter| crate::factory::filter_from_script(filter, ctx));
        body.container_name = script.container_name().to_string();
        body.custom_drink_sound = Some(script.custom_drink_sound().to_string());
        body.rain_catcher = script.rain_catcher();
        body.fills_with_clean_water = script.is_filled_with_clean_water();
        body.input_locked = false;
        body.can_player_empty = true;
        // Native setCapacity writes capacity BEFORE its getAmount call.
        let old_capacity = body.capacity;
        body.capacity = script.capacity().max(0.05);
        let _unowned_update = self.fluid_recalculate_data(ctx, false)?;
        let body = match &mut self.body {
            RuntimeBody::Data(Component::FluidContainer(body)) => body,
            _ => unreachable!(),
        };
        if body.amount_cache > body.capacity {
            for index in (0..body.fluids.len()).rev() {
                let percentage = body.fluids[index].amount / old_capacity;
                body.fluids[index].amount = body.capacity * percentage;
                if body.fluids[index].amount.abs() < 1.0e-4 {
                    body.fluids.remove(index);
                }
            }
        }
        body.cache_invalidated = true;
        let initial_amount = |rng: &mut dyn crate::factory::FactoryRng| {
            if !script.initial_amount_set() {
                script.capacity()
            } else {
                let (min, max) = script.initial_amount_range();
                if min == max {
                    min
                } else {
                    rng.next_float(min, max)
                }
            }
        };
        if let Some(initial) = script.initial_fluids().filter(|fluids| !fluids.is_empty()) {
            if initial_amount(rng) > 0.0 {
                let amount = initial_amount(rng);
                let indices: Vec<usize> = if initial.len() == 1 {
                    vec![0]
                } else if script.is_initial_fluids_is_random() {
                    vec![rng.next_int(initial.len() as i32) as usize]
                } else {
                    (0..initial.len()).collect()
                };
                for index in indices {
                    let initial = &initial[index];
                    let name = initial.fluid_type();
                    match ctx.fluid_by_name(name) {
                        crate::Lookup::Missing => {
                            native_notices.push(
                                crate::runtime_fluid::RuntimeNativeNotice::GeneralLog(format!(
                                    "FluidContainer -> initial Fluid '{name}' not found! [{script_name}]"
                                )),
                            );
                            continue;
                        }
                        crate::Lookup::Unknown => {
                            return Err(RuntimeError::UnsupportedRuntime(
                                "Fluid script factory registry is unavailable",
                            ))
                        }
                        crate::Lookup::Found(_) => {}
                    }
                    use pz_scripts::objects::definitions::FluidType;
                    let fluid = match FluidType::value_of(name) {
                        Ok(kind) if kind != FluidType::Modded && kind != FluidType::None => {
                            crate::fluids::FluidRef::Type(kind.id())
                        }
                        _ => crate::fluids::FluidRef::Modded(name.to_string()),
                    };
                    let add = amount * initial.percentage();
                    if add.max(0.0) > 0.0 {
                        let _unowned_update = self.fluid_recalculate_data(ctx, false)?;
                        let body = match &mut self.body {
                            RuntimeBody::Data(Component::FluidContainer(body)) => body,
                            _ => unreachable!(),
                        };
                        let color = initial
                            .custom_color()
                            .map(|color| [color.r, color.g, color.b]);
                        body.add_fluid(fluid, color, add, ctx);
                    }
                }
            }
        }
        let body = match &mut self.body {
            RuntimeBody::Data(Component::FluidContainer(body)) => body,
            _ => unreachable!(),
        };
        body.input_locked = script.input_locked();
        body.can_player_empty = script.can_empty();
        body.hidden_amount = script.is_hidden_amount();
        Ok(())
    }

    /// Pure-data ScriptInfo setter. The native cached script-object reference
    /// has no exposed getter here; adding one requires its real retained owner.
    pub fn set_original_script_data(
        &mut self,
        is_item: bool,
        full_type: String,
    ) -> Result<(), RuntimeError> {
        let RuntimeBody::Data(Component::Script(script)) = &mut self.body else {
            return Err(RuntimeError::InvalidBody(self.kind));
        };
        script.original_is_item = is_item;
        script.original_script = Some(full_type);
        Ok(())
    }

    /// Cache the same actual script allocation. The replaced cache pin is
    /// returned for outer retirement; no script body is copied or traversed.
    #[must_use]
    pub fn set_original_script_actual(
        &mut self,
        is_item: bool,
        full_type: String,
        script: &EntityOwner,
    ) -> Result<Option<EntityOwner>, RuntimeError> {
        self.set_original_script_data(is_item, full_type)?;
        Ok(std::mem::replace(
            &mut self.script_cache,
            Some(script.clone()),
        ))
    }

    pub fn script_cache_slot(&self) -> Option<&EntityOwner> {
        self.script_cache.as_ref()
    }

    /// The normal Script.load lookup phase uses this only when the loaded
    /// originalScript was nonnull. A null original name preserves the cache.
    #[must_use]
    pub fn replace_script_cache(
        &mut self,
        script: Option<&EntityOwner>,
    ) -> Result<Option<EntityOwner>, RuntimeError> {
        if self.kind != ComponentType::Script {
            return Err(RuntimeError::InvalidBody(self.kind));
        }
        Ok(std::mem::replace(&mut self.script_cache, script.cloned()))
    }

    pub(crate) fn reset_detached_for_pool(
        &mut self,
        retired: &mut RetiredComponents,
    ) -> Result<(), RuntimeError> {
        if self.owner.is_some() {
            return Err(RuntimeError::UnsupportedRuntime(
                "Component pool reset requires detached owner",
            ));
        }
        match &mut self.body {
            RuntimeBody::Attributes(attributes) => attributes.clear(retired),
            RuntimeBody::Durability(_) => Ok(()), // actual inherited Component.reset
            RuntimeBody::Data(Component::Script(_)) => Ok(()), // actual inherited Component.reset
            RuntimeBody::Data(Component::Empty(ComponentType::ContextMenuConfig)) => Ok(()),
            RuntimeBody::Data(Component::FluidContainer(body)) => self
                .fluid_runtime
                .as_mut()
                .expect("fluid runtime matches immutable kind")
                .reset_detached(body),
            _ => Err(RuntimeError::UnsupportedLive(self.kind)),
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
    /// block subreader, rollback, owner mutation or lifecycle/pool call occurs.
    /// Fluid field loading includes its authentic Empty(false) cache prefix;
    /// the host finishes its successful force-recalc and owner phases.
    /// `ctx` must be a reviewed, callback-free lookup implementation while a
    /// component RefCell guard is held. Errors preserve partial body effects.
    pub fn load_payload_in_place(
        &mut self,
        reader: &mut Reader<'_>,
        world_version: i32,
        ctx: &dyn EntityContext,
        retired: &mut RetiredComponents,
    ) -> Result<(), RuntimeError> {
        match &mut self.body {
            RuntimeBody::Attributes(attributes) => attributes.load(reader, ctx, retired)?,
            RuntimeBody::Durability(value) => value.load(reader)?,
            // Native field decoding precedes the real cache/properties and
            // guard-free owner phases driven by RuntimeLoadHost. The legacy
            // DTO loader must not publish its amount-only cache prematurely.
            RuntimeBody::Data(Component::FluidContainer(fluid)) => {
                // Fluid.load reaches Empty(false) before even reading its
                // header. All final cache contents are therefore cleared on
                // a partial header/body failure, with no owner notification.
                fluid.amount_cache = 0.0;
                fluid.capacity = 1.0;
                fluid.whitelist = None;
                fluid.blacklist = None;
                fluid.input_locked = false;
                fluid.can_player_empty = false;
                fluid.hidden_amount = false;
                self.fluid_runtime
                    .as_mut()
                    .expect("fluid runtime matches immutable kind")
                    .empty_without_recalculate(fluid)?;
                fluid.load_data_without_cache(reader, ctx)?;
            }
            RuntimeBody::Data(body) => body.load(reader, world_version, ctx)?,
        }
        if let Some(fluid) = &mut self.fluid_runtime {
            fluid.mark_uninitialized();
        }
        Ok(())
    }

    pub fn write_payload(&self, writer: &mut Writer) -> Result<(), RuntimeError> {
        match &self.body {
            RuntimeBody::Attributes(attributes) => attributes.write(writer)?,
            RuntimeBody::Durability(value) => value.write(writer)?,
            RuntimeBody::Data(body) => body.save(writer),
        }
        Ok(())
    }

    /// Named archive copy: saved/script body data and final cache contents
    /// copy into fresh allocations, shared actual script definitions retain
    /// their reference, and the actual entity-owner backedge is never followed.
    pub fn try_archive_copy_unowned(&self) -> Result<ComponentRef, RuntimeError> {
        let copy = Self {
            kind: self.kind,
            body: match &self.body {
                RuntimeBody::Attributes(attributes) => {
                    RuntimeBody::Attributes(attributes.archive_copy()?)
                }
                RuntimeBody::Durability(value) => RuntimeBody::Durability(value.clone()),
                RuntimeBody::Data(body) => RuntimeBody::Data(body.clone()),
            },
            owner: None,
            fluid_runtime: self
                .fluid_runtime
                .as_ref()
                .map(RuntimeFluidState::archive_copy)
                .transpose()?,
            script_cache: self.script_cache.clone(),
        };
        Ok(Rc::new(RefCell::new(copy)))
    }

    /// Borrow-only archive preflight. This does not allocate/cache/clone any
    /// body or recursively traverse the actual owner backedge.
    pub fn check_archive_copy(&self) -> Result<(), RuntimeError> {
        if let Some(fluid) = &self.fluid_runtime {
            fluid.check_archive_copy()?;
        }
        if let Some(attributes) = self.attributes() {
            attributes.check_archive_copy()?;
        }
        Ok(())
    }

    /// Native AttributeContainer.readFromScript data phase on this SAME
    /// receiver. A missing script template logs and leaves current data alone.
    pub fn read_attributes_template(
        &mut self,
        template: Option<&ComponentRef>,
        script_name: &str,
        notices: &mut Vec<crate::runtime_fluid::RuntimeNativeNotice>,
        retired: &mut RetiredComponents,
    ) -> Result<(), RuntimeError> {
        let Some(template) = template else {
            if self.kind != ComponentType::Attributes {
                return Err(RuntimeError::InvalidBody(self.kind));
            }
            notices.push(crate::runtime_fluid::RuntimeNativeNotice::GeneralError(
                format!("Unable to create AttributeContainer from script: {script_name}"),
            ));
            return Ok(());
        };
        let same = std::ptr::eq(template.as_ref().as_ptr().cast_const(), self as *const Self);
        let kind = self.kind;
        let destination = self
            .attributes_mut()
            .ok_or(RuntimeError::InvalidBody(kind))?;
        if same {
            return destination.clear(retired);
        }
        let source = template
            .try_borrow()
            .map_err(|_| RuntimeError::BorrowedTransient("Attribute template component"))?;
        let source = source
            .attributes()
            .ok_or(RuntimeError::InvalidBody(source.kind()))?;
        destination.copy_from(source, retired)
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
#[derive(Default)]
#[must_use]
pub struct RetiredComponents {
    entries: Vec<RuntimeEntry>,
    owners: Vec<EntityOwner>,
    components: Vec<ComponentRef>,
    payloads: Vec<Box<dyn Any>>,
}

impl fmt::Debug for RetiredComponents {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RetiredComponents")
            .field("entries", &self.entries.len())
            .field("owners", &self.owners.len())
            .field("components", &self.components.len())
            .field("payloads", &self.payloads.len())
            .finish()
    }
}

impl RetiredComponents {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
            && self.owners.is_empty()
            && self.components.is_empty()
            && self.payloads.is_empty()
    }
    pub fn len(&self) -> usize {
        self.entries.len() + self.owners.len() + self.components.len() + self.payloads.len()
    }
    pub fn push_owner(&mut self, owner: EntityOwner) {
        self.owners.push(owner);
    }
    pub fn push_erased(&mut self, allocation: Rc<dyn Any>) {
        self.owners.push(EntityOwner { allocation });
    }
    pub fn push_component(&mut self, component: ComponentRef) {
        self.components.push(component);
    }
    /// Retain owned host bookkeeping until its outer guard-safe drain.
    /// This payload is not a native entity/component or lifecycle action.
    pub fn push_payload(&mut self, payload: Box<dyn Any>) {
        self.payloads.push(payload);
    }
    pub fn append(&mut self, mut other: Self) {
        self.entries.append(&mut other.entries);
        self.owners.append(&mut other.owners);
        self.components.append(&mut other.components);
        self.payloads.append(&mut other.payloads);
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
        let mut default_color = None;
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
            if values
                .list
                .iter()
                .any(|body| matches!(body, Component::FluidContainer(_)))
            {
                default_color = Some(crate::runtime_fluid::copy_default_color()?);
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
                    component: Rc::new(RefCell::new(RuntimeComponent::from_validated_body(
                        body,
                        default_color,
                    ))),
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

    /// Explicit encoded import append; never rebuilds/replaces unrelated
    /// live allocations. Rejected input is returned intact for outer drop.
    pub fn push_encoded_if_absent(
        &mut self,
        entry: EncodedComponent,
    ) -> Result<(), RejectedEncoded> {
        let error = if self.entries.len() >= 127 {
            Some(RuntimeError::TooManyComponents)
        } else if self
            .entries
            .iter()
            .any(|current| current.stored_type_id() == entry.type_id())
        {
            Some(RuntimeError::DuplicateType(entry.type_id()))
        } else {
            None
        };
        if let Some(error) = error {
            return Err(RejectedEncoded {
                error,
                entries: vec![entry],
            });
        }
        self.entries.push(RuntimeEntry::Encoded(entry));
        Ok(())
    }

    /// Remove only an explicit encoded carrier. Live actual components are
    /// never silently removed through a blob/DTO migration API.
    pub fn remove_encoded_type_unordered(&mut self, type_id: i16) -> Option<EncodedComponent> {
        let index = self.entries.iter().position(
            |entry| matches!(entry, RuntimeEntry::Encoded(encoded) if encoded.type_id() == type_id),
        )?;
        match self.entries.swap_remove(index) {
            RuntimeEntry::Encoded(encoded) => Some(encoded),
            RuntimeEntry::Live { .. } => unreachable!(),
        }
    }

    /// Replace only the sole encoded carrier at its existing physical index.
    /// Actual live components cannot be edited through this archive API.
    pub fn replace_encoded(
        &mut self,
        existing_type_id: i16,
        incoming: EncodedComponent,
    ) -> Result<EncodedComponent, RejectedEncoded> {
        if incoming.type_id() != existing_type_id {
            return Err(RejectedEncoded {
                error: RuntimeError::UnsupportedRuntime(
                    "Encoded replacement cannot change component type",
                ),
                entries: vec![incoming],
            });
        }
        let Some(index) = self
            .entries
            .iter()
            .position(|entry| entry.stored_type_id() == existing_type_id)
        else {
            return Err(RejectedEncoded {
                error: RuntimeError::UnsupportedRuntime(
                    "Encoded replacement requires an existing carrier",
                ),
                entries: vec![incoming],
            });
        };
        if matches!(self.entries[index], RuntimeEntry::Live { .. }) {
            return Err(RejectedEncoded {
                error: RuntimeError::UnsupportedRuntime(
                    "Live component cannot be replaced through encoded archive API",
                ),
                entries: vec![incoming],
            });
        }
        match std::mem::replace(&mut self.entries[index], RuntimeEntry::Encoded(incoming)) {
            RuntimeEntry::Encoded(old) => Ok(old),
            RuntimeEntry::Live { .. } => unreachable!(),
        }
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
        let default_color = if kind == ComponentType::FluidContainer {
            Some(crate::runtime_fluid::copy_default_color()?)
        } else {
            None
        };
        let mut runtime = RuntimeComponent::from_validated_body(body, default_color);
        if kind == ComponentType::FluidContainer {
            runtime.initialize_fluid_data(ctx)?;
        }
        let component = Rc::new(RefCell::new(runtime));
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
                        .value_snapshot()?,
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
                    if left.value_snapshot()? != right.value_snapshot()? {
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
