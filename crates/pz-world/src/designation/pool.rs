//! Concrete actual ObjectPool state for the stateless Position3D allocator.
//! The native constructor and the explicit test projection are exclusive.
use super::*;
use pz_core::pool_statistics as statistics;
use std::any::Any;
use std::collections::VecDeque;

pub type PositionDequeRef = Rc<RefCell<PositionDeque>>;
pub type PositionSetRef = Rc<RefCell<PositionSet>>;
pub type PositionAllocatorRef = Rc<RefCell<PositionAllocator>>;
pub type Position3dRef = Rc<RefCell<Position3d>>;
#[derive(Default)]
pub struct Position3d {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}
pub struct PositionAllocator;
#[derive(Default)]
pub struct PositionDeque {
    pub entries: VecDeque<EntityOwner>,
}
pub struct PositionSet {
    /// ObjectPool$1.this$0 is a real strong reference, not a weak owner hint.
    pub pool: Option<DesignationPoolRef>,
    pub entries: Vec<EntityOwner>,
    pub auto_compaction_factor: f32,
}
pub enum PositionPoolAuthority {
    Native {
        queue: Option<PositionDequeRef>,
        contains_set: Option<PositionSetRef>,
        allocator: Option<PositionAllocatorRef>,
        max_size: i32,
    },
    DataProjection {
        released_positions: Vec<EntityOwner>,
    },
}
pub struct DesignationPositionPool {
    pub authority: PositionPoolAuthority,
}
impl Default for DesignationPositionPool {
    fn default() -> Self {
        Self {
            authority: PositionPoolAuthority::Native {
                queue: None,
                contains_set: None,
                allocator: None,
                max_size: 0,
            },
        }
    }
}
impl DesignationPositionPool {
    pub fn data_projection() -> Self {
        Self {
            authority: PositionPoolAuthority::DataProjection {
                released_positions: Vec::new(),
            },
        }
    }
    pub fn data_projection_entries(&self) -> Option<&Vec<EntityOwner>> {
        match &self.authority {
            PositionPoolAuthority::DataProjection { released_positions } => {
                Some(released_positions)
            }
            _ => None,
        }
    }
    /// Move every actual owning slot, preserving native/data discriminant and
    /// native max scalar. No Native clear/reset/release or counter mutation.
    pub fn take_owner_slots_for_gc(&mut self) -> Box<dyn Any> {
        match &mut self.authority {
            PositionPoolAuthority::Native {
                queue,
                contains_set,
                allocator,
                ..
            } => Box::new((queue.take(), contains_set.take(), allocator.take())),
            PositionPoolAuthority::DataProjection { released_positions } => {
                Box::new(std::mem::take(released_positions))
            }
        }
    }
}
struct StatisticsScope(statistics::StatisticsRetired);
impl Drop for StatisticsScope {
    fn drop(&mut self) {
        let mut retired = RetiredComponents::new();
        retired.push_payload(Box::new(std::mem::take(&mut self.0)));
        pz_inventory::component_host::retire_components(retired);
    }
}
fn error(error: statistics::StatisticsError) -> RuntimeHostError {
    match error {
        statistics::StatisticsError::Borrowed(p) => RuntimeHostError::Borrowed(p),
        statistics::StatisticsError::Unsupported(p) => unsupported(p),
    }
}
fn read_pool_size(owner: &Rc<dyn Any>) -> statistics::StatisticsResult<f64> {
    let pool = owner
        .downcast_ref::<RefCell<DesignationPositionPool>>()
        .ok_or(statistics::StatisticsError::Unsupported(
            "ObjectPoolCounter capture class",
        ))?;
    let queue = {
        let body = pool
            .try_borrow()
            .map_err(|_| statistics::StatisticsError::Borrowed("ObjectPool.size receiver"))?;
        match &body.authority {
            PositionPoolAuthority::Native { queue, .. } => {
                queue
                    .clone()
                    .ok_or(statistics::StatisticsError::Unsupported(
                        "ObjectPool.size null deque diagnostic",
                    ))?
            }
            PositionPoolAuthority::DataProjection { .. } => {
                return Err(statistics::StatisticsError::Unsupported(
                    "ObjectPool.size data-projection receiver",
                ))
            }
        }
    };
    let n = queue
        .try_borrow()
        .map_err(|_| statistics::StatisticsError::Borrowed("ObjectPool.size actual deque"))?
        .entries
        .len();
    Ok(n as f64)
}
/// Native superclass prefix, over the actual already allocated receiver.
fn register_super(
    pool: &DesignationPoolRef,
    globals: &statistics::ClassStaticsRef,
    retired: &mut statistics::StatisticsRetired,
) -> HostResult<()> {
    statistics::try_register_object_pool_counter(
        globals,
        pool,
        Some("DesignationZoneAnimal.position3dPool"),
        read_pool_size,
        retired,
    )
    .map_err(error)?;
    Ok(())
}
fn finish_fields(
    pool: &DesignationPoolRef,
    allocator: &PositionAllocatorRef,
    retired: &mut RetiredComponents,
) -> HostResult<()> {
    let queue = Rc::new(RefCell::new(PositionDeque::default()));
    retired.push_owner(EntityOwner::from_ref(&queue));
    observe(DesignationObserved::PoolDeque(&queue));
    {
        let mut body = pool
            .try_borrow_mut()
            .map_err(|_| RuntimeHostError::Borrowed("ObjectPool deque publication"))?;
        let PositionPoolAuthority::Native { queue: slot, .. } = &mut body.authority else {
            return Err(unsupported("native pool field publication profile"));
        };
        *slot = Some(queue);
    }
    observe(DesignationObserved::Pool(pool));
    // Fresh THash -> TObjectHash -> THashSet constructors authenticate 0.5.
    // The subclass capture exists before its superclass initialization.
    let set = Rc::new(RefCell::new(PositionSet {
        pool: Some(pool.clone()),
        entries: Vec::new(),
        auto_compaction_factor: 0.5,
    }));
    retired.push_owner(EntityOwner::from_ref(&set));
    observe(DesignationObserved::PoolSet(&set));
    {
        let mut body = pool
            .try_borrow_mut()
            .map_err(|_| RuntimeHostError::Borrowed("ObjectPool containsSet publication"))?;
        let PositionPoolAuthority::Native { contains_set, .. } = &mut body.authority else {
            return Err(unsupported("native pool field publication profile"));
        };
        *contains_set = Some(set.clone());
    }
    observe(DesignationObserved::Pool(pool));
    {
        let mut body = pool
            .try_borrow_mut()
            .map_err(|_| RuntimeHostError::Borrowed("ObjectPool allocator/max publication"))?;
        let PositionPoolAuthority::Native {
            allocator: slot,
            max_size,
            ..
        } = &mut body.authority
        else {
            return Err(unsupported("native pool field publication profile"));
        };
        *slot = Some(allocator.clone());
        *max_size = 1024;
    }
    set.try_borrow_mut()
        .map_err(|_| RuntimeHostError::Borrowed("ObjectPool containsSet auto compaction"))?
        .auto_compaction_factor = 0.0;
    observe(DesignationObserved::PoolSet(&set));
    observe(DesignationObserved::Pool(pool));
    Ok(())
}
pub(super) fn try_construct_native(
    retired: &mut RetiredComponents,
) -> HostResult<DesignationPoolRef> {
    let mut stats = StatisticsScope(statistics::StatisticsRetired::default());
    // Superclass class initialization precedes the first actual pool receiver.
    let globals = statistics::try_class_statics(&mut stats.0).map_err(error)?;
    let pool = Rc::new(RefCell::new(DesignationPositionPool::default()));
    retired.push_owner(EntityOwner::from_ref(&pool));
    observe(DesignationObserved::Pool(&pool));
    // Actual stateless Position3D constructor-reference object passed as arg.
    // No invented ZoneRef capture and no claim about generated JVM class name.
    let allocator = Rc::new(RefCell::new(PositionAllocator));
    retired.push_owner(EntityOwner::from_ref(&allocator));
    observe(DesignationObserved::PoolAllocator(&allocator));
    register_super(&pool, &globals, &mut stats.0)?;
    finish_fields(&pool, &allocator, retired)?;
    Ok(pool)
}

/// Exact List-empty releaseAll branch (no pool body/allocator/debug access).
/// Nonempty release needs the actual debug option/release protocol, so stops
/// before replacing that dependency with a hardcoded false/queue mutation.
pub(super) fn try_release_roofs(
    pool: &DesignationPoolRef,
    roofs: &DesignationListRef,
    retired: &mut RetiredComponents,
) -> HostResult<()> {
    retired.push_owner(EntityOwner::from_ref(pool));
    retired.push_owner(EntityOwner::from_ref(roofs));
    let empty = {
        let body = roofs
            .try_borrow()
            .map_err(|_| RuntimeHostError::Borrowed("ObjectPool.releaseAll roof list size"))?;
        let DesignationList::RoofAreas(v) = &*body else {
            return Err(unsupported("ObjectPool.releaseAll actual roof list class"));
        };
        v.is_empty()
    };
    if empty {
        return Ok(());
    }
    // Selected constructor-only pool profile: release's first queue/max fields
    // are real and can fault before the still-unported debug field dependency.
    let (queue, max) = {
        let body = pool
            .try_borrow()
            .map_err(|_| RuntimeHostError::Borrowed("ObjectPool.release receiver"))?;
        match &body.authority {
            PositionPoolAuthority::Native {
                queue, max_size, ..
            } => (
                queue
                    .clone()
                    .ok_or_else(|| unsupported("ObjectPool.release null deque diagnostic"))?,
                *max_size,
            ),
            PositionPoolAuthority::DataProjection { .. } => {
                return Err(unsupported("ObjectPool.release data-projection receiver"))
            }
        }
    };
    let size = queue
        .try_borrow()
        .map_err(|_| RuntimeHostError::Borrowed("ObjectPool.release deque size"))?
        .entries
        .len();
    if size as i64 >= i64::from(max) {
        return Ok(());
    }
    Err(unsupported(
        "ObjectPool.release actual objectPoolContains debug option",
    ))
}

#[cfg(test)]
mod tests;
