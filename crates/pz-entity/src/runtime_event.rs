//! Actual `EntityEvent` references and its unbounded FIFO native pool.
//!
//! Reset clears only the entity slot. Detached owners are retired by the
//! caller after foreign guards; neither Rust Drop nor GC invokes release.
use crate::runtime_load::{HostResult, RuntimeHostError};
use crate::{EntityOwner, RetiredComponents};
use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;

/// Only the authenticated event needed by the current live floor path.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EntityEventType {
    AddedToWorld,
}

#[derive(Debug, Default)]
pub struct EntityEvent {
    event_type: Option<EntityEventType>,
    entity: Option<EntityOwner>,
}
pub type EntityEventRef = Rc<RefCell<EntityEvent>>;

impl EntityEvent {
    pub fn event_type(&self) -> Option<EntityEventType> {
        self.event_type
    }
    /// The physical slot is exposed by reference for tracing, without a clone.
    pub fn entity_slot(&self) -> Option<&EntityOwner> {
        self.entity.as_ref()
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum EventReleaseError {
    Host(RuntimeHostError),
    /// Java AssertionError, distinct from an Exception caught by native code.
    AlreadyInPool,
}
impl From<RuntimeHostError> for EventReleaseError {
    fn from(error: RuntimeHostError) -> Self {
        Self::Host(error)
    }
}

#[derive(Default)]
pub struct EntityEventPool {
    values: RefCell<VecDeque<EntityEventRef>>,
}

impl EntityEventPool {
    pub fn len(&self) -> HostResult<usize> {
        Ok(self
            .values
            .try_borrow()
            .map_err(|_| RuntimeHostError::Borrowed("EntityEvent.pool"))?
            .len())
    }
    pub fn is_empty(&self) -> HostResult<bool> {
        self.len().map(|n| n == 0)
    }

    /// poll/new, then eventType, then entity, in native order. A popped
    /// reference is pinned even if a host borrow rejects the following write.
    pub fn alloc(
        &self,
        event_type: EntityEventType,
        entity: &EntityOwner,
        retired: &mut RetiredComponents,
    ) -> HostResult<EntityEventRef> {
        let event = self
            .values
            .try_borrow_mut()
            .map_err(|_| RuntimeHostError::Borrowed("EntityEvent.pool poll"))?
            .pop_front()
            .unwrap_or_else(|| Rc::new(RefCell::new(EntityEvent::default())));
        retired.push_payload(Box::new(event.clone()));
        {
            let mut body = event
                .try_borrow_mut()
                .map_err(|_| RuntimeHostError::Borrowed("EntityEvent.Alloc receiver"))?;
            body.event_type = Some(event_type);
            if let Some(old) = body.entity.replace(entity.clone()) {
                retired.push_owner(old);
            }
        }
        Ok(event)
    }

    /// reset precedes the optional assertion, including when that assertion
    /// fails. Read Core.debug at its real phase, outside event/pool guards.
    pub fn release(
        &self,
        event: &EntityEventRef,
        retired: &mut RetiredComponents,
        assertions_enabled: bool,
        debug_enabled: impl FnOnce() -> HostResult<bool>,
    ) -> Result<(), EventReleaseError> {
        {
            let mut body = event
                .try_borrow_mut()
                .map_err(|_| RuntimeHostError::Borrowed("EntityEvent.reset receiver"))?;
            if let Some(entity) = body.entity.take() {
                retired.push_owner(entity);
            }
        }
        if assertions_enabled && debug_enabled()? {
            let pool = self
                .values
                .try_borrow()
                .map_err(|_| RuntimeHostError::Borrowed("EntityEvent.pool contains"))?;
            if pool.iter().any(|current| Rc::ptr_eq(current, event)) {
                return Err(EventReleaseError::AlreadyInPool);
            }
        }
        self.values
            .try_borrow_mut()
            .map_err(|_| RuntimeHostError::Borrowed("EntityEvent.pool offer"))?
            .push_back(event.clone());
        Ok(())
    }
}

thread_local! {
    static POOL: Rc<EntityEventPool> = Rc::new(EntityEventPool::default());
}

/// Clone the actual pool root before invoking host code, without a TLS guard.
pub fn entity_event_pool() -> Rc<EntityEventPool> {
    POOL.with(Rc::clone)
}

/// The original rereads component size each iteration; a dispatcher must not
/// snapshot the component list or claim an unknown onEntityEvent is a no-op.
pub trait EntityEventHost {
    fn entity_owner(&self) -> HostResult<EntityOwner>;
    fn has_component_container(&self) -> HostResult<bool>;
    fn component_count(&self) -> HostResult<usize>;
    fn component_event(
        &mut self,
        index: usize,
        event: &EntityEventRef,
        retired: &mut RetiredComponents,
    ) -> HostResult<()>;
    fn assertions_enabled(&self) -> bool;
    fn debug_enabled(&self) -> HostResult<bool>;
}

/// Allocates and releases even with zero components. There is no native
/// finally: a component/host failure leaves the allocated event unreleased.
pub fn send_entity_event(
    host: &mut dyn EntityEventHost,
    event_type: EntityEventType,
    retired: &mut RetiredComponents,
) -> HostResult<()> {
    let pool = entity_event_pool();
    let event = pool.alloc(event_type, &host.entity_owner()?, retired)?;
    // Native performs one null gate, then rereads size for every iteration.
    // A callback clearing a previously nonnull container must not become an
    // empty-success branch: its JVM null diagnostic is still unimplemented.
    if host.has_component_container()? {
        let mut index = 0;
        loop {
            let size = host.component_count()?;
            if index >= size {
                break;
            }
            host.component_event(index, &event, retired)?;
            index += 1;
        }
    }
    pool.release(&event, retired, host.assertions_enabled(), || {
        host.debug_enabled()
    })
    .map_err(|error| match error {
        EventReleaseError::Host(error) => error,
        EventReleaseError::AlreadyInPool => RuntimeHostError::InvalidHostState(
            "EntityEvent.release AssertionError: Object already in pool.",
        ),
    })
}

#[cfg(test)]
mod tests;
