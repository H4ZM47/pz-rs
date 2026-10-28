//! Genuine same-allocation, FIFO native class pools for supported components.
//!
//! Owner diagnostics/removal are a host phase. This detached entry point never
//! substitutes Drop for that phase and never resets when the class has no pool.
//! Pool references are real strong native roots, intentionally not GC weak refs.

use std::cell::RefCell;
use std::collections::{HashMap, VecDeque};
use std::fmt;
use std::rc::Rc;

use crate::{
    Component, ComponentRef, ComponentType, RetiredComponents, RuntimeComponent, RuntimeError,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PoolError {
    UnsupportedClass(ComponentType),
    BorrowedComponent,
    BorrowedFactory,
    OwnerPresent,
    AlreadyInPool,
    Runtime(RuntimeError),
}

impl fmt::Display for PoolError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedClass(kind) => write!(
                f,
                "Native component pool for {} is unsupported",
                kind.name()
            ),
            Self::BorrowedComponent => f.write_str("Component is borrowed"),
            Self::BorrowedFactory => f.write_str("Component factory is borrowed"),
            Self::OwnerPresent => f.write_str("Owner not removed"),
            Self::AlreadyInPool => f.write_str("Object already in pool."),
            Self::Runtime(error) => error.fmt(f),
        }
    }
}
impl std::error::Error for PoolError {}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReleaseOutcome {
    MissingPool,
    Pooled,
    Full,
}

/// Native assertions use desiredAssertionStatus, not Rust debug_assertions.
/// Core.debug is read by the host at the actual factory release phase.
#[derive(Clone, Copy, Debug, Default)]
pub struct PoolReleaseOptions {
    pub assertions_enabled: bool,
    pub debug: bool,
}

#[derive(Debug, Default)]
struct ClassPool {
    values: VecDeque<ComponentRef>,
    peak: usize,
}

#[derive(Debug)]
pub struct RuntimeComponentFactory {
    initial_size: i32,
    max_size: i32,
    pools: HashMap<ComponentType, ClassPool>,
}

impl Default for RuntimeComponentFactory {
    fn default() -> Self {
        Self::new()
    }
}

impl RuntimeComponentFactory {
    pub fn new() -> Self {
        Self::with_limits(1024, i32::MAX)
    }

    /// The installed pool constructor stores max, ignores initial for
    /// allocation, and starts with an empty ConcurrentLinkedDeque.
    pub fn with_limits(initial_size: i32, max_size: i32) -> Self {
        Self {
            initial_size,
            max_size,
            pools: HashMap::new(),
        }
    }
    pub fn initial_size(&self) -> i32 {
        self.initial_size
    }
    pub fn max_size(&self) -> i32 {
        self.max_size
    }
    pub fn has_pool(&self, kind: ComponentType) -> bool {
        self.pools.contains_key(&kind)
    }
    pub fn pool_len(&self, kind: ComponentType) -> Option<usize> {
        self.pools.get(&kind).map(|pool| pool.values.len())
    }
    pub fn peak_size(&self, kind: ComponentType) -> Option<usize> {
        self.pools.get(&kind).map(|pool| pool.peak)
    }

    pub fn ensure_pool(&mut self, kind: ComponentType) -> Result<(), PoolError> {
        if !matches!(
            kind,
            ComponentType::Attributes
                | ComponentType::FluidContainer
                | ComponentType::Script
                | ComponentType::ContextMenuConfig
                | ComponentType::Durability
        ) {
            return Err(PoolError::UnsupportedClass(kind));
        }
        self.pools.entry(kind).or_default();
        Ok(())
    }

    pub fn obtain(&mut self, kind: ComponentType) -> Result<ComponentRef, PoolError> {
        self.ensure_pool(kind)?;
        if let Some(component) = self
            .pools
            .get_mut(&kind)
            .and_then(|pool| pool.values.pop_front())
        {
            return Ok(component);
        }
        let body = Component::create(kind).ok_or(PoolError::UnsupportedClass(kind))?;
        RuntimeComponent::new_unowned(body).map_err(|rejected| PoolError::Runtime(rejected.error))
    }

    /// Normal nonasserting profile. Production hosts with enabled Java
    /// assertions/Core.debug use release_detached_checked instead.
    pub fn release_detached(
        &mut self,
        component: &ComponentRef,
        retired: &mut RetiredComponents,
    ) -> Result<ReleaseOutcome, PoolError> {
        self.release_detached_checked(component, retired, PoolReleaseOptions::default())
    }

    pub fn release_detached_checked(
        &mut self,
        component: &ComponentRef,
        retired: &mut RetiredComponents,
        options: PoolReleaseOptions,
    ) -> Result<ReleaseOutcome, PoolError> {
        let kind = component
            .try_borrow()
            .map_err(|_| PoolError::BorrowedComponent)?
            .kind();
        let Some(pool) = self.pools.get_mut(&kind) else {
            return Ok(ReleaseOutcome::MissingPool);
        };
        if options.assertions_enabled
            && options.debug
            && pool
                .values
                .iter()
                .any(|current| Rc::ptr_eq(current, component))
        {
            return Err(PoolError::AlreadyInPool);
        }
        {
            let mut body = component
                .try_borrow_mut()
                .map_err(|_| PoolError::BorrowedComponent)?;
            if body.owner_slot().is_some() {
                return Err(PoolError::OwnerPresent);
            }
            // Current supported reset bodies and final cache cells are pure
            // data. No old-owner callback, Lua call or arbitrary payload Drop
            // is allowed while this factory queue is borrowed.
            body.reset_detached_for_pool(retired)
                .map_err(PoolError::Runtime)?;
        }
        if pool.values.len() as i64 >= self.max_size as i64 {
            return Ok(ReleaseOutcome::Full);
        }
        pool.values.push_back(Rc::clone(component));
        pool.peak = pool.peak.max(pool.values.len());
        Ok(ReleaseOutcome::Pooled)
    }

    /// Physical pool roots for reviewed tracing/diagnostics. These owning
    /// slots must never be weakened or detached by cycle collection.
    pub fn pool_slots(&self) -> impl Iterator<Item = &ComponentRef> {
        self.pools.values().flat_map(|pool| pool.values.iter())
    }
}

thread_local! {
    static COMPONENT_FACTORY: RefCell<RuntimeComponentFactory> = RefCell::new(RuntimeComponentFactory::new());
}

pub fn ensure_component_pool(kind: ComponentType) -> Result<(), PoolError> {
    COMPONENT_FACTORY.with(|factory| {
        factory
            .try_borrow_mut()
            .map_err(|_| PoolError::BorrowedFactory)?
            .ensure_pool(kind)
    })
}
pub fn component_pool_exists(kind: ComponentType) -> Result<bool, PoolError> {
    COMPONENT_FACTORY.with(|factory| {
        Ok(factory
            .try_borrow()
            .map_err(|_| PoolError::BorrowedFactory)?
            .has_pool(kind))
    })
}
pub fn obtain_component(kind: ComponentType) -> Result<ComponentRef, PoolError> {
    COMPONENT_FACTORY.with(|factory| {
        factory
            .try_borrow_mut()
            .map_err(|_| PoolError::BorrowedFactory)?
            .obtain(kind)
    })
}
pub fn release_detached_component(
    component: &ComponentRef,
    retired: &mut RetiredComponents,
) -> Result<ReleaseOutcome, PoolError> {
    COMPONENT_FACTORY.with(|factory| {
        factory
            .try_borrow_mut()
            .map_err(|_| PoolError::BorrowedFactory)?
            .release_detached(component, retired)
    })
}
pub fn release_detached_component_checked(
    component: &ComponentRef,
    retired: &mut RetiredComponents,
    options: PoolReleaseOptions,
) -> Result<ReleaseOutcome, PoolError> {
    COMPONENT_FACTORY.with(|factory| {
        factory
            .try_borrow_mut()
            .map_err(|_| PoolError::BorrowedFactory)?
            .release_detached_checked(component, retired, options)
    })
}

#[cfg(test)]
mod tests;
