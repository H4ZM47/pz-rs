use super::*;
use std::cell::Cell;

#[test]
fn fifo_reuses_same_event_and_reset_retains_type_but_retires_owner() {
    let pool = EntityEventPool::default();
    let owner = Rc::new(RefCell::new(123));
    let weak = Rc::downgrade(&owner);
    let mut retired = RetiredComponents::new();
    let first = pool
        .alloc(
            EntityEventType::AddedToWorld,
            &EntityOwner::from_ref(&owner),
            &mut retired,
        )
        .unwrap();
    let second = pool
        .alloc(
            EntityEventType::AddedToWorld,
            &EntityOwner::from_ref(&owner),
            &mut retired,
        )
        .unwrap();
    assert!(!Rc::ptr_eq(&first, &second));
    assert!(first
        .borrow()
        .entity_slot()
        .unwrap()
        .ptr_eq(&EntityOwner::from_ref(&owner)));
    pool.release(&first, &mut retired, false, || {
        panic!("assertions disabled")
    })
    .unwrap();
    pool.release(&second, &mut retired, false, || {
        panic!("assertions disabled")
    })
    .unwrap();
    assert_eq!(
        first.borrow().event_type(),
        Some(EntityEventType::AddedToWorld)
    );
    assert!(first.borrow().entity_slot().is_none());
    drop(owner);
    assert!(weak.upgrade().is_some());
    drop(retired);
    assert!(weak.upgrade().is_none());
    let next = Rc::new(RefCell::new(456));
    let mut retired = RetiredComponents::new();
    assert!(Rc::ptr_eq(
        &first,
        &pool
            .alloc(
                EntityEventType::AddedToWorld,
                &EntityOwner::from_ref(&next),
                &mut retired
            )
            .unwrap()
    ));
    assert!(Rc::ptr_eq(
        &second,
        &pool
            .alloc(
                EntityEventType::AddedToWorld,
                &EntityOwner::from_ref(&next),
                &mut retired
            )
            .unwrap()
    ));
}

#[test]
fn reset_happens_before_debug_callback_and_failed_assertion_does_not_offer() {
    let pool = EntityEventPool::default();
    let owner = Rc::new(RefCell::new(()));
    let mut retired = RetiredComponents::new();
    let event = pool
        .alloc(
            EntityEventType::AddedToWorld,
            &EntityOwner::from_ref(&owner),
            &mut retired,
        )
        .unwrap();
    pool.release(&event, &mut retired, false, || unreachable!())
        .unwrap();
    let observed = Cell::new(false);
    assert_eq!(
        pool.release(&event, &mut retired, true, || {
            assert!(event.try_borrow_mut().unwrap().entity_slot().is_none());
            assert_eq!(pool.len()?, 1);
            observed.set(true);
            Ok(true)
        }),
        Err(EventReleaseError::AlreadyInPool)
    );
    assert!(observed.get());
    assert_eq!(pool.len(), Ok(1));
}

#[test]
fn pool_borrow_failure_keeps_completed_reset_and_owner_retirement() {
    let pool = EntityEventPool::default();
    let owner = Rc::new(RefCell::new(()));
    let mut retired = RetiredComponents::new();
    let event = pool
        .alloc(
            EntityEventType::AddedToWorld,
            &EntityOwner::from_ref(&owner),
            &mut retired,
        )
        .unwrap();
    let guard = pool.values.borrow_mut();
    assert_eq!(
        pool.release(&event, &mut retired, false, || unreachable!()),
        Err(EventReleaseError::Host(RuntimeHostError::Borrowed(
            "EntityEvent.pool offer"
        )))
    );
    assert!(event.borrow().entity_slot().is_none());
    assert_eq!(
        event.borrow().event_type(),
        Some(EntityEventType::AddedToWorld)
    );
    assert!(!retired.is_empty());
    drop(guard);
    assert_eq!(pool.len(), Ok(0));
}

struct Host {
    owner: Rc<RefCell<()>>,
    counts: RefCell<VecDeque<usize>>,
    seen: Vec<EntityEventRef>,
    fail: bool,
}
impl EntityEventHost for Host {
    fn entity_owner(&self) -> HostResult<EntityOwner> {
        Ok(EntityOwner::from_ref(&self.owner))
    }
    fn has_component_container(&self) -> HostResult<bool> {
        Ok(true)
    }
    fn component_count(&self) -> HostResult<usize> {
        Ok(self.counts.borrow_mut().pop_front().unwrap_or(0))
    }
    fn component_event(
        &mut self,
        _: usize,
        event: &EntityEventRef,
        _: &mut RetiredComponents,
    ) -> HostResult<()> {
        assert!(event.try_borrow_mut().is_ok());
        assert!(event
            .borrow()
            .entity_slot()
            .unwrap()
            .ptr_eq(&EntityOwner::from_ref(&self.owner)));
        self.seen.push(event.clone());
        if self.fail {
            Err(RuntimeHostError::Unsupported {
                operation: "test component event",
                kind: None,
            })
        } else {
            Ok(())
        }
    }
    fn assertions_enabled(&self) -> bool {
        false
    }
    fn debug_enabled(&self) -> HostResult<bool> {
        panic!("assertions disabled")
    }
}

#[test]
fn component_failure_does_not_release_and_success_rereads_current_size() {
    let pool = entity_event_pool();
    let mut retired = RetiredComponents::new();
    let mut host = Host {
        owner: Rc::new(RefCell::new(())),
        counts: RefCell::new(VecDeque::from([2, 0])),
        seen: vec![],
        fail: false,
    };
    send_entity_event(&mut host, EntityEventType::AddedToWorld, &mut retired).unwrap();
    assert_eq!(host.seen.len(), 1);
    assert!(host.seen[0].borrow().entity_slot().is_none());
    let before = pool.len().unwrap();
    host.counts.borrow_mut().push_back(1);
    host.fail = true;
    assert!(send_entity_event(&mut host, EntityEventType::AddedToWorld, &mut retired).is_err());
    assert_eq!(pool.len().unwrap(), before.saturating_sub(1));
    assert!(host.seen[1].borrow().entity_slot().is_some());
}

#[test]
fn empty_component_send_still_allocates_and_releases_actual_event() {
    let pool = entity_event_pool();
    assert_eq!(pool.len(), Ok(0));
    let mut retired = RetiredComponents::new();
    let mut host = Host {
        owner: Rc::new(RefCell::new(())),
        counts: RefCell::new(VecDeque::new()),
        seen: vec![],
        fail: false,
    };
    send_entity_event(&mut host, EntityEventType::AddedToWorld, &mut retired).unwrap();
    assert!(host.seen.is_empty());
    assert_eq!(pool.len(), Ok(1));
    let event = pool
        .alloc(
            EntityEventType::AddedToWorld,
            &EntityOwner::from_ref(&host.owner),
            &mut retired,
        )
        .unwrap();
    assert!(event.borrow().entity_slot().is_some());
}
