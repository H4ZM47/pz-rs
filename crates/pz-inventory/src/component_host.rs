//! Native item component phases over the actual ItemRef.
//!
//! No entity/component guard spans lifecycle or owner notification. Supported
//! FluidContainer, AttributeContainer, Durability, ContextMenuConfig and
//! EntityScriptInfo inherit the
//! authenticated empty owner hooks. Other classes require their actual hook
//! implementations.
//! The current add/connect factory path covers fresh, unequipped receivers.
//! Inherited equip/entity-manager transitions still need their actual state
//! and call sites before this host covers supplied equipped receivers.

use pz_entity::runtime_fluid::FluidRecalcOutcome;
use pz_entity::runtime_load::{
    HostResult, NativeComponentIssue, RuntimeHostError, RuntimeLoadHost,
};
use pz_entity::runtime_pool::{self, PoolError};
use pz_entity::{
    ComponentRef, ComponentType, EntityContext, EntityError, EntityOwner, RetiredComponents,
    RuntimeError,
};
use std::cell::{Cell, RefCell};

use crate::shared::{self, ItemRef};

pub type ItemFluidObserver = fn(&ItemRef) -> HostResult<()>;
pub type FluidOwnerObserver = fn(&EntityOwner) -> HostResult<()>;
pub type ComponentDebugQuery = fn() -> HostResult<bool>;
pub type ScriptCacheResolver = fn(bool, &str) -> HostResult<Option<EntityOwner>>;
pub type NativeNoticeLogger = fn(&pz_entity::runtime_fluid::RuntimeNativeNotice) -> HostResult<()>;

thread_local! {
    static ITEM_FLUID_OBSERVER: Cell<Option<ItemFluidObserver>> = const { Cell::new(None) };
    static FLUID_OWNER_OBSERVER: Cell<Option<FluidOwnerObserver>> = const { Cell::new(None) };
    static COMPONENT_DEBUG: Cell<Option<ComponentDebugQuery>> = const { Cell::new(None) };
    static SCRIPT_CACHE_RESOLVER: Cell<Option<ScriptCacheResolver>> = const { Cell::new(None) };
    static NATIVE_NOTICE_LOGGER: Cell<Option<NativeNoticeLogger>> = const { Cell::new(None) };
    static RETIRED: RefCell<Vec<RetiredComponents>> = const { RefCell::new(Vec::new()) };
}

pub fn install_item_fluid_observer(
    observer: Option<ItemFluidObserver>,
) -> Option<ItemFluidObserver> {
    ITEM_FLUID_OBSERVER.with(|slot| slot.replace(observer))
}

pub fn install_component_debug_query(
    query: Option<ComponentDebugQuery>,
) -> Option<ComponentDebugQuery> {
    COMPONENT_DEBUG.with(|slot| slot.replace(query))
}

pub fn install_fluid_owner_observer(
    observer: Option<FluidOwnerObserver>,
) -> Option<FluidOwnerObserver> {
    FLUID_OWNER_OBSERVER.with(|slot| slot.replace(observer))
}

pub fn component_debug_enabled() -> HostResult<bool> {
    match COMPONENT_DEBUG.with(Cell::get) {
        Some(query) => query(),
        None => Ok(false),
    }
}

pub fn install_script_cache_resolver(
    resolver: Option<ScriptCacheResolver>,
) -> Option<ScriptCacheResolver> {
    SCRIPT_CACHE_RESOLVER.with(|slot| slot.replace(resolver))
}

/// Install the real callback-free native diagnostic sink, with no captured
/// owner graph. It must not invoke Lua or mutate world/entity/script state.
pub fn install_native_notice_logger(
    logger: Option<NativeNoticeLogger>,
) -> Option<NativeNoticeLogger> {
    NATIVE_NOTICE_LOGGER.with(|slot| slot.replace(logger))
}

/// Forward the owned native StringUTF diagnostic prefix after all item/table
/// guards, including failed table loads.
pub fn emit_mod_data_notices(notices: &[pz_kahlua::java_io::JavaIoNotice]) -> HostResult<()> {
    let notices: Vec<_> = notices
        .iter()
        .map(|notice| {
            pz_entity::runtime_fluid::RuntimeNativeNotice::GeneralError(notice.native_message())
        })
        .collect();
    emit_native_notices(&notices)
}

pub fn emit_native_notices(
    notices: &[pz_entity::runtime_fluid::RuntimeNativeNotice],
) -> HostResult<()> {
    let logger = NATIVE_NOTICE_LOGGER.with(Cell::get);
    for notice in notices {
        if let Some(logger) = logger {
            logger(notice)?;
        } else {
            match notice {
                pz_entity::runtime_fluid::RuntimeNativeNotice::GeneralLog(message)
                | pz_entity::runtime_fluid::RuntimeNativeNotice::GeneralError(message)
                | pz_entity::runtime_fluid::RuntimeNativeNotice::GeneralException(message)
                | pz_entity::runtime_fluid::RuntimeNativeNotice::ExceptionLogger(message) => {
                    eprintln!("{message}")
                }
            }
        }
    }
    Ok(())
}

/// A host failure translated to a language error must first move every
/// potentially final owner pin here. This performs no native remove/reset.
pub fn retire_components(retired: RetiredComponents) {
    if !retired.is_empty() {
        RETIRED.with(|queue| queue.borrow_mut().push(retired));
    }
}

/// Engine-only safe boundary, after WORLD/sprite/entity/component guards.
pub fn drain_retired_components() {
    let retired = RETIRED.with(|queue| std::mem::take(&mut *queue.borrow_mut()));
    drop(retired);
}

pub fn notify_fluid_update(outcome: &FluidRecalcOutcome) -> HostResult<()> {
    if !outcome.recalculated {
        return Ok(());
    }
    let Some(owner) = &outcome.owner else {
        return Ok(());
    };
    if let Some(observer) = FLUID_OWNER_OBSERVER.with(Cell::get) {
        return observer(owner);
    }
    let Some(item) = owner.try_handle::<crate::InventoryItem>() else {
        return Err(RuntimeHostError::Unsupported {
            operation: "world entity onFluidContainerUpdate",
            kind: Some(ComponentType::FluidContainer),
        });
    };
    let observer = ITEM_FLUID_OBSERVER.with(Cell::get);
    if let Some(observer) = observer {
        return observer(&item);
    }
    // InventoryItem and every current item subclass inherit the exact empty
    // GameEntity callback. The dirty predicate still needs actual parent type.
    let mut container = shared::try_container_of(&item)?;
    let mut outer = None;
    while let Some(current) = container {
        let containing = {
            let body = current
                .try_borrow()
                .map_err(|_| RuntimeHostError::Borrowed("fluid outermost container"))?;
            body.containing_item.clone()
        };
        outer = Some(current);
        container = match containing {
            Some(bag) => shared::try_container_of(&bag)?,
            None => None,
        };
    }
    if let Some(outer) = outer {
        if outer
            .try_borrow()
            .map_err(|_| RuntimeHostError::Borrowed("fluid outermost parent"))?
            .parent
            .is_some()
        {
            return Err(RuntimeHostError::Unsupported {
                operation: "actual IsoPlayer fluid-container parent predicate",
                kind: Some(ComponentType::FluidContainer),
            });
        }
    }
    Ok(())
}

pub fn pool_error(error: PoolError) -> RuntimeHostError {
    match error {
        PoolError::UnsupportedClass(kind) => RuntimeHostError::Unsupported {
            operation: "native component class pool",
            kind: Some(kind),
        },
        PoolError::BorrowedComponent => RuntimeHostError::Borrowed("native component pool body"),
        PoolError::BorrowedFactory => RuntimeHostError::Borrowed("native component factory"),
        PoolError::OwnerPresent => {
            RuntimeHostError::InvalidHostState("native component pool still has owner")
        }
        PoolError::AlreadyInPool => {
            RuntimeHostError::InvalidHostState("AssertionError: Object already in pool.")
        }
        PoolError::Runtime(error) => RuntimeHostError::Storage(error),
    }
}

fn kind_of(component: &ComponentRef) -> HostResult<ComponentType> {
    component
        .try_borrow()
        .map(|body| body.kind())
        .map_err(|_| RuntimeHostError::Borrowed("component immutable class"))
}

pub(crate) fn native_component_error(error: RuntimeError) -> RuntimeHostError {
    match error {
        RuntimeError::Codec(error) => RuntimeHostError::NativeException(error),
        error => RuntimeHostError::Storage(error),
    }
}

fn supported_owner_hook(kind: ComponentType, operation: &'static str) -> HostResult<()> {
    match kind {
        ComponentType::FluidContainer
        | ComponentType::Attributes
        | ComponentType::Durability
        | ComponentType::ContextMenuConfig
        | ComponentType::Script => Ok(()),
        _ => Err(RuntimeHostError::Unsupported {
            operation,
            kind: Some(kind),
        }),
    }
}

/// Native source detach only: exact membership -> inherited onRemoved ->
/// owner None. The SAME allocation is returned; no release/reset occurs.
pub fn detach_component_for_transfer(
    item: &ItemRef,
    kind: ComponentType,
    retired: &mut RetiredComponents,
) -> HostResult<Option<ComponentRef>> {
    let component = item
        .try_borrow()
        .map_err(|_| RuntimeHostError::Borrowed("transfer source item"))?
        .components
        .get_ref(kind)
        .map_err(RuntimeHostError::Storage)?;
    let Some(component) = component else {
        return Ok(None);
    };
    let removed = item
        .try_borrow_mut()
        .map_err(|_| RuntimeHostError::Borrowed("transfer source membership"))?
        .components
        .runtime_mut()
        .remove_exact_unordered(&component);
    let Some(removed) = removed else {
        retired.push_component(component);
        return Ok(None);
    };
    retired.push_component(component);
    retired.push_component(removed.clone());
    supported_owner_hook(kind, "onRemovedFromOwner")?;
    let old_owner = removed
        .try_borrow_mut()
        .map_err(|_| RuntimeHostError::Borrowed("transfer source owner clear"))?
        .replace_owner(None);
    if let Some(old_owner) = old_owner {
        retired.push_owner(old_owner);
    }
    Ok(Some(removed))
}

pub struct ItemComponentHost<'a> {
    pub item: ItemRef,
    pub context: &'a dyn EntityContext,
}

impl<'a> ItemComponentHost<'a> {
    pub fn new(item: &ItemRef, context: &'a dyn EntityContext) -> Self {
        Self {
            item: item.clone(),
            context,
        }
    }

    /// Native add prechecks precede removal from the incoming actual owner.
    pub fn add(
        &mut self,
        component: &ComponentRef,
        retired: &mut RetiredComponents,
    ) -> HostResult<bool> {
        let kind = kind_of(component)?;
        {
            let item = self
                .item
                .try_borrow()
                .map_err(|_| RuntimeHostError::Borrowed("add target item"))?;
            if item.components.len() >= 127 {
                drop(item);
                eprintln!("Maximum component size '127' for entity reached.");
                return Ok(false);
            }
            if item.components.runtime().presence(kind) != pz_entity::EntryPresence::Absent {
                return Ok(false);
            }
        }
        let old_owner = component
            .try_borrow()
            .map_err(|_| RuntimeHostError::Borrowed("incoming component owner"))?
            .owner_slot()
            .cloned();
        if let Some(old_owner) = old_owner {
            if let Some(old_item) = old_owner.try_handle::<crate::InventoryItem>() {
                let mut old_host = ItemComponentHost::new(&old_item, self.context);
                if let Some(removed) = old_host.remove(component, retired)? {
                    retired.push_component(removed);
                }
            } else {
                retired.push_owner(old_owner);
                return Err(RuntimeHostError::Unsupported {
                    operation: "remove component from actual non-item owner",
                    kind: Some(kind),
                });
            }
            retired.push_owner(old_owner);
        }
        let inserted = self
            .item
            .try_borrow_mut()
            .map_err(|_| RuntimeHostError::Borrowed("add item membership"))?
            .components
            .runtime_mut()
            .push_ref_if_absent(component.clone());
        if let Err(rejected) = inserted {
            retired.push_component(rejected.component);
            return Err(RuntimeHostError::Storage(rejected.error));
        }
        // Metadata registration has no entity/component guard and precedes
        // the newly formed actual owner cycle, including native loaded adds.
        shared::observe_item(&self.item);
        let old = component
            .try_borrow_mut()
            .map_err(|_| RuntimeHostError::Borrowed("assign item component owner"))?
            .replace_owner(Some(&EntityOwner::from_ref(&self.item)));
        if let Some(old) = old {
            retired.push_owner(old);
        }
        supported_owner_hook(kind, "onAddedToOwner")?;
        // Native InventoryItem.addToWorld returns before manager/flags/event.
        Ok(true)
    }

    pub fn remove(
        &mut self,
        component: &ComponentRef,
        retired: &mut RetiredComponents,
    ) -> HostResult<Option<ComponentRef>> {
        let removed = self
            .item
            .try_borrow_mut()
            .map_err(|_| RuntimeHostError::Borrowed("remove item component membership"))?
            .components
            .runtime_mut()
            .remove_exact_unordered(component);
        let Some(removed) = removed else {
            return Ok(None);
        };
        retired.push_component(removed.clone());
        let kind = kind_of(&removed)?;
        supported_owner_hook(kind, "onRemovedFromOwner")?;
        let owner = removed
            .try_borrow_mut()
            .map_err(|_| RuntimeHostError::Borrowed("clear item component owner"))?
            .replace_owner(None);
        if let Some(owner) = owner {
            retired.push_owner(owner);
        }
        Ok(Some(removed))
    }

    pub fn connect(&mut self, _retired: &mut RetiredComponents) -> HostResult<()> {
        let mut index = 0;
        loop {
            let component = {
                let item = self
                    .item
                    .try_borrow()
                    .map_err(|_| RuntimeHostError::Borrowed("connect item current index"))?;
                if index >= item.components.len() {
                    break;
                }
                item.components
                    .get_ref_at(index)
                    .map_err(RuntimeHostError::Storage)?
                    .ok_or(RuntimeHostError::InvalidHostState(
                        "missing current component index",
                    ))?
            };
            supported_owner_hook(kind_of(&component)?, "onConnectComponents")?;
            index += 1;
        }
        Ok(())
    }
}

impl RuntimeLoadHost for ItemComponentHost<'_> {
    fn debug_enabled(&self) -> HostResult<bool> {
        component_debug_enabled()
    }
    fn resolve_type(&mut self, id: i16) -> HostResult<ComponentType> {
        // The live set currently supports actual Fluid+Script constructors.
        // Undefined reflection policy is an explicit unported boundary.
        if id < 0 || id > 26 {
            return Err(RuntimeHostError::NativeException(EntityError(
                "ArrayIndexOutOfBoundsException".into(),
            )));
        }
        Ok(ComponentType::from_id(id))
    }
    fn get_component(&mut self, kind: ComponentType) -> HostResult<Option<ComponentRef>> {
        self.item
            .try_borrow()
            .map_err(|_| RuntimeHostError::Borrowed("get item component"))?
            .components
            .get_ref(kind)
            .map_err(RuntimeHostError::Storage)
    }
    fn component_size(&mut self) -> HostResult<usize> {
        Ok(self
            .item
            .try_borrow()
            .map_err(|_| RuntimeHostError::Borrowed("item component size"))?
            .components
            .len())
    }
    fn component_at(&mut self, index: usize) -> HostResult<Option<ComponentRef>> {
        self.item
            .try_borrow()
            .map_err(|_| RuntimeHostError::Borrowed("item component index"))?
            .components
            .get_ref_at(index)
            .map_err(RuntimeHostError::Storage)
    }
    fn obtain_component(
        &mut self,
        kind: ComponentType,
        _retired: &mut RetiredComponents,
    ) -> HostResult<Option<ComponentRef>> {
        runtime_pool::obtain_component(kind)
            .map(Some)
            .map_err(pool_error)
    }
    fn load_component(
        &mut self,
        component: &ComponentRef,
        reader: &mut pz_entity::io::Reader<'_>,
        world_version: i32,
        retired: &mut RetiredComponents,
    ) -> HostResult<()> {
        let loaded: HostResult<Option<FluidRecalcOutcome>> = (|| {
            let (outcome, script_lookup) = {
                let mut body = component
                    .try_borrow_mut()
                    .map_err(|_| RuntimeHostError::Borrowed("load actual item component"))?;
                body.load_payload_in_place(reader, world_version, self.context, retired)
                    .map_err(native_component_error)?;
                let outcome = if body.kind() == ComponentType::FluidContainer {
                    Some(
                        body.fluid_recalculate_data(self.context, true)
                            .map_err(native_component_error)?,
                    )
                } else {
                    None
                };
                let script_lookup = body.script_info().and_then(|script| {
                    script
                        .original_script
                        .as_ref()
                        .map(|name| (script.original_is_item, name.clone()))
                });
                (outcome, script_lookup)
            };
            if let Some((is_item, name)) = script_lookup {
                let template = self
                    .item
                    .try_borrow()
                    .map_err(|_| RuntimeHostError::Borrowed("script cache item template"))?
                    .template
                    .clone();
                let own_script = template
                    .script
                    .as_ref()
                    .filter(|_| is_item)
                    .map(|script| script.0.clone());
                let matching = if let Some(script) = own_script.as_ref() {
                    script
                        .try_borrow()
                        .map_err(|_| RuntimeHostError::Borrowed("actual original item script"))?
                        .full_name()
                        == name
                } else {
                    false
                };
                let cache = if matching {
                    own_script.as_ref().map(EntityOwner::from_ref)
                } else if let Some(resolve) = SCRIPT_CACHE_RESOLVER.with(Cell::get) {
                    resolve(is_item, &name)?
                } else {
                    return Err(RuntimeHostError::Unsupported {
                        operation: "actual ScriptManager original-script cache lookup",
                        kind: Some(ComponentType::Script),
                    });
                };
                let old = component
                    .try_borrow_mut()
                    .map_err(|_| RuntimeHostError::Borrowed("assign actual script cache"))?
                    .replace_script_cache(cache.as_ref())
                    .map_err(RuntimeHostError::Storage)?;
                if let Some(old) = old {
                    retired.push_owner(old);
                }
            }
            Ok(outcome)
        })();
        // Existing owned components can acquire new actual instance/type
        // slots, and Script can replace its actual definition cache, even
        // before a later load failure. Adopt that completed physical prefix
        // after every data guard ends and before any owner notification.
        shared::observe_item(&self.item);
        let outcome = loaded?;
        if let Some(outcome) = outcome {
            notify_fluid_update(&outcome)?;
        }
        Ok(())
    }
    fn add_component(
        &mut self,
        component: &ComponentRef,
        retired: &mut RetiredComponents,
    ) -> HostResult<bool> {
        self.add(component, retired)
    }
    fn connect_components(&mut self, retired: &mut RetiredComponents) -> HostResult<()> {
        self.connect(retired)
    }
    fn remove_component_exact(
        &mut self,
        component: &ComponentRef,
        retired: &mut RetiredComponents,
    ) -> HostResult<Option<ComponentRef>> {
        self.remove(component, retired)
    }
    fn release_component(
        &mut self,
        component: &ComponentRef,
        retired: &mut RetiredComponents,
    ) -> HostResult<()> {
        if !runtime_pool::component_pool_exists(kind_of(component)?).map_err(pool_error)? {
            return Ok(());
        }
        runtime_pool::release_detached_component_checked(
            component,
            retired,
            runtime_pool::PoolReleaseOptions {
                assertions_enabled: false,
                debug: component_debug_enabled()?,
            },
        )
        .map(|_| ())
        .map_err(pool_error)
    }
    fn log_exception(&mut self, issue: &NativeComponentIssue) -> HostResult<()> {
        eprintln!("GameEntity.loadEntity: {}", issue.exception);
        Ok(())
    }
}

#[derive(Debug)]
#[must_use]
pub struct ItemLoadFailure {
    pub error: RuntimeHostError,
    pub item: ItemRef,
    pub retired: RetiredComponents,
    pub position: usize,
    pub component_phase: Option<pz_entity::runtime_load::RuntimeLoadPhase>,
}

impl ItemLoadFailure {
    pub fn into_error_and_retire(mut self) -> RuntimeHostError {
        self.retired.push_owner(EntityOwner::from_ref(&self.item));
        retire_components(self.retired);
        self.error
    }
}

/// Native actual-receiver load. The original caller buffer and cursor are
/// shared directly with the entity decoder on both success and failure.
/// Nested subclass loaders need their own resumable actual-owner coordinator;
/// an archive subreader cannot substitute for that path.
pub fn load_item_ref(
    item: &ItemRef,
    reader: &mut crate::io::Reader<'_>,
    world_version: i32,
    env: &dyn crate::ItemEnv,
) -> Result<RetiredComponents, ItemLoadFailure> {
    let mut retired = RetiredComponents::new();
    let fail = |error, retired, position, component_phase| ItemLoadFailure {
        error,
        item: item.clone(),
        retired,
        position,
        component_phase,
    };
    let mut notices = Vec::new();
    let prefix = item
        .try_borrow_mut()
        .map_err(|_| RuntimeHostError::Borrowed("item load prefix"))
        .and_then(|mut body| {
            body.load_prefix(reader, world_version, env, &mut retired, &mut notices)
                .map_err(crate::ModDataError::into_host)
        });
    shared::observe_item(item);
    let diagnostics = emit_mod_data_notices(&notices);
    if let Err(error) = diagnostics {
        return Err(fail(error, retired, reader.pos, None));
    }
    let continuation = match prefix {
        Ok(continuation) => continuation,
        Err(error) => return Err(fail(error, retired, reader.pos, None)),
    };
    if continuation.entity_present {
        let Some(context) = env.components().entity_context() else {
            return Err(fail(
                RuntimeHostError::Unsupported {
                    operation: "actual item load component context",
                    kind: None,
                },
                retired,
                reader.pos,
                None,
            ));
        };
        let mut host = ItemComponentHost::new(item, context);
        let report = {
            let original_position = reader.pos;
            let mut input = pz_entity::io::Reader::borrow_cursor(reader.buf, &mut reader.pos)
                .map_err(|error| {
                    fail(
                        RuntimeHostError::NativeException(error),
                        std::mem::take(&mut retired),
                        original_position,
                        None,
                    )
                })?;
            pz_entity::runtime_load::load_native_with(
                &mut input,
                world_version,
                &mut host,
                pz_entity::runtime_load::RuntimeLoadOptions {
                    collect_loaded: true,
                },
            )
        };
        let report = match report {
            Ok(report) => report,
            Err(mut failure) => {
                for component in failure.report.loaded.drain(..) {
                    failure.report.retired.push_component(component);
                }
                retired.append(failure.report.retired);
                return Err(fail(
                    failure.error,
                    retired,
                    reader.pos,
                    Some(failure.phase),
                ));
            }
        };
        match pz_entity::runtime_load::prune_item_after_entity_load(&mut host, report) {
            Ok(report) => retired.append(report.retired),
            Err(mut failure) => {
                for component in failure.report.loaded.drain(..) {
                    failure.report.retired.push_component(component);
                }
                retired.append(failure.report.retired);
                return Err(fail(
                    failure.error,
                    retired,
                    reader.pos,
                    Some(failure.phase),
                ));
            }
        }
    }
    let pending_texture = item
        .try_borrow_mut()
        .map_err(|_| RuntimeHostError::Borrowed("item load pre-texture suffix"))
        .and_then(|mut body| {
            body.load_suffix_before_texture(continuation, reader)
                .map_err(|error| RuntimeHostError::NativeException(EntityError(error.0)))
        });
    shared::observe_item(item);
    let pending_texture = match pending_texture {
        Ok(name) => name,
        Err(error) => return Err(fail(error, retired, reader.pos, None)),
    };
    if let Some(name) = pending_texture {
        if let Err(error) = crate::item_texture::load_texture_ref(item, &name) {
            return Err(fail(error, retired, reader.pos, None));
        }
    }
    let suffix = item
        .try_borrow_mut()
        .map_err(|_| RuntimeHostError::Borrowed("item load post-texture suffix"))
        .and_then(|mut body| {
            body.load_suffix_after_texture(continuation, reader, world_version)
                .map_err(|error| RuntimeHostError::NativeException(EntityError(error.0)))
        });
    if let Err(error) = suffix {
        return Err(fail(error, retired, reader.pos, None));
    }
    if let Err(error) = crate::item_texture::synch_with_visual_ref(
        item,
        env.assets(),
        &crate::context::NoOwner,
        env.visual_rng(),
        &mut |m| env.rand_next(m),
    ) {
        return Err(fail(error, retired, reader.pos, None));
    }
    let subclass = match item.try_borrow() {
        Ok(body) => match body.kind {
            crate::ItemKind::Container(_) => 1,
            crate::ItemKind::Weapon(_) | crate::ItemKind::Map { .. } => 2,
            _ => 0,
        },
        Err(_) => {
            return Err(fail(
                RuntimeHostError::Borrowed("item subclass load dispatch"),
                retired,
                reader.pos,
                None,
            ))
        }
    };
    if subclass == 1 {
        if let Err(error) = crate::shared_load::load_bag_tail_ref(item, reader, world_version, env)
        {
            return Err(fail(error, retired, reader.pos, None));
        }
        return Ok(retired);
    }
    if subclass == 2 {
        return Err(fail(
            RuntimeHostError::Unsupported {
                operation: "nested/original-buffer item subclass load continuation",
                kind: None,
            },
            retired,
            reader.pos,
            None,
        ));
    }
    let loaded = item
        .try_borrow_mut()
        .map_err(|_| RuntimeHostError::Borrowed("item subclass body load"))
        .and_then(|mut body| {
            body.load_kind_native(reader, world_version, env)
                .map_err(|error| RuntimeHostError::NativeException(EntityError(error.0)))
        });
    if let Err(error) = loaded {
        return Err(fail(error, retired, reader.pos, None));
    }
    Ok(retired)
}

#[cfg(test)]
mod tests;
