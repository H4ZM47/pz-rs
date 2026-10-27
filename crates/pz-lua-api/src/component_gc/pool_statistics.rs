//! Physical statistics/capture/pool graph, sharing designation's one worklist.
//! No trace path invokes the capture value reader or Native pool operations.
use super::designation::{register_graph, Node};
use super::*;
use pz_core::pool_statistics as stats;
use pz_world::designation::{
    DesignationPositionPool, Position3d, Position3dRef, PositionAllocator, PositionAllocatorRef,
    PositionDeque, PositionDequeRef, PositionPoolAuthority, PositionSet, PositionSetRef,
};
use std::any::Any;

pub(super) enum PendingNode {
    Globals(Weak<RefCell<stats::ClassStatics>>),
    Statistic(Weak<RefCell<stats::PoolStatistic>>),
    CounterMap(Weak<RefCell<stats::CounterMap>>),
    NumberMap(Weak<RefCell<stats::NumberMap>>),
    Counter(Weak<RefCell<stats::Counter>>),
    Capture(Weak<RefCell<stats::PoolSizeCapture>>),
    Long(Weak<RefCell<stats::AtomicLongValue>>),
    Double(Weak<RefCell<stats::AtomicDoubleValue>>),
    Deque(Weak<RefCell<PositionDeque>>),
    Set(Weak<RefCell<PositionSet>>),
    Allocator(Weak<RefCell<PositionAllocator>>),
    Position(Weak<RefCell<Position3d>>),
}
impl PendingNode {
    pub(super) fn key(&self) -> (&'static str, usize) {
        match self {
            Self::Globals(v) => ("pool statistics statics", v.as_ptr() as usize),
            Self::Statistic(v) => ("pool statistic", v.as_ptr() as usize),
            Self::CounterMap(v) => ("pool counter map", v.as_ptr() as usize),
            Self::NumberMap(v) => ("pool number map", v.as_ptr() as usize),
            Self::Counter(v) => ("pool counter", v.as_ptr() as usize),
            Self::Capture(v) => ("pool counter capture", v.as_ptr() as usize),
            Self::Long(v) => ("pool atomic long", v.as_ptr() as usize),
            Self::Double(v) => ("pool atomic double", v.as_ptr() as usize),
            Self::Deque(v) => ("position deque", v.as_ptr() as usize),
            Self::Set(v) => ("position set", v.as_ptr() as usize),
            Self::Allocator(v) => ("position allocator", v.as_ptr() as usize),
            Self::Position(v) => ("position3d", v.as_ptr() as usize),
        }
    }
    pub(super) fn alive(&self) -> bool {
        match self {
            Self::Globals(v) => v.strong_count() != 0,
            Self::Statistic(v) => v.strong_count() != 0,
            Self::CounterMap(v) => v.strong_count() != 0,
            Self::NumberMap(v) => v.strong_count() != 0,
            Self::Counter(v) => v.strong_count() != 0,
            Self::Capture(v) => v.strong_count() != 0,
            Self::Long(v) => v.strong_count() != 0,
            Self::Double(v) => v.strong_count() != 0,
            Self::Deque(v) => v.strong_count() != 0,
            Self::Set(v) => v.strong_count() != 0,
            Self::Allocator(v) => v.strong_count() != 0,
            Self::Position(v) => v.strong_count() != 0,
        }
    }
    pub(super) fn retry(self) {
        let root = match self {
            Self::Globals(v) => v.upgrade().map(Node::StatsGlobals),
            Self::Statistic(v) => v.upgrade().map(Node::Statistic),
            Self::CounterMap(v) => v.upgrade().map(Node::CounterMap),
            Self::NumberMap(v) => v.upgrade().map(Node::NumberMap),
            Self::Counter(v) => v.upgrade().map(Node::Counter),
            Self::Capture(v) => v.upgrade().map(Node::Capture),
            Self::Long(v) => v.upgrade().map(Node::AtomicLong),
            Self::Double(v) => v.upgrade().map(Node::AtomicDouble),
            Self::Deque(v) => v.upgrade().map(Node::PoolDeque),
            Self::Set(v) => v.upgrade().map(Node::PoolSet),
            Self::Allocator(v) => v.upgrade().map(Node::PoolAllocator),
            Self::Position(v) => v.upgrade().map(Node::Position),
        };
        if let Some(root) = root {
            let _ = register_graph(root);
        }
    }
}
fn record(value: PendingNode) -> Pending {
    Pending::PoolStatistics(value)
}

pub struct GlobalsDescriptor;
impl TraceDescriptor<stats::ClassStatics> for GlobalsDescriptor {
    fn trace(v: &stats::ClassStatics, t: &mut TraceVisitor) {
        if let Some(x) = v.id_slot() {
            t.owner(x);
        }
        if let Some(x) = v.instance_slot() {
            t.owner(x);
        }
    }
    fn detach(v: &mut stats::ClassStatics) -> Option<Box<dyn Any>> {
        Some(Box::new(v.take_for_gc()))
    }
}
pub struct StatisticDescriptor;
impl TraceDescriptor<stats::PoolStatistic> for StatisticDescriptor {
    fn trace(v: &stats::PoolStatistic, t: &mut TraceVisitor) {
        if let Some(x) = v.counters_slot() {
            t.owner(x);
        }
        if let Some(x) = v.statistics_slot() {
            t.owner(x);
        }
        if let Some(x) = v.local_table_slot() {
            t.table(x);
        }
        if let Some(x) = v.remote_table_slot() {
            t.table(x);
        }
        if let Some(x) = v.counter_for_delete_slot() {
            t.owner(x);
        }
    }
    fn detach(v: &mut stats::PoolStatistic) -> Option<Box<dyn Any>> {
        Some(Box::new(v.take_for_gc()))
    }
}
pub struct CounterMapDescriptor;
impl TraceDescriptor<stats::CounterMap> for CounterMapDescriptor {
    fn trace(v: &stats::CounterMap, t: &mut TraceVisitor) {
        for counter in v.entries().values() {
            t.owner(counter);
        }
    }
    fn detach(v: &mut stats::CounterMap) -> Option<Box<dyn Any>> {
        Some(Box::new(v.take_for_gc()))
    }
}
pub struct NumberMapDescriptor;
impl TraceDescriptor<stats::NumberMap> for NumberMapDescriptor {
    fn trace(_: &stats::NumberMap, _: &mut TraceVisitor) {}
    fn detach(v: &mut stats::NumberMap) -> Option<Box<dyn Any>> {
        Some(Box::new(v.take_for_gc()))
    }
}
pub struct CounterDescriptor;
impl TraceDescriptor<stats::Counter> for CounterDescriptor {
    fn trace(v: &stats::Counter, t: &mut TraceVisitor) {
        if let Some(x) = v.value_slot() {
            t.owner(x);
        }
        if let Some(x) = v.lambda_slot() {
            t.owner(x);
        }
    }
    fn detach(v: &mut stats::Counter) -> Option<Box<dyn Any>> {
        Some(Box::new(v.take_for_gc()))
    }
}
pub struct CaptureDescriptor;
impl TraceDescriptor<stats::PoolSizeCapture> for CaptureDescriptor {
    fn trace(v: &stats::PoolSizeCapture, t: &mut TraceVisitor) {
        if let Some(x) = v.owner_slot() {
            if x.is::<RefCell<DesignationPositionPool>>() {
                t.owner_erased::<DesignationPositionPool>(x);
            }
            // Other genuine pool classes remain conservative until their
            // actual descriptor is ported. Never run the capture reader here.
        }
    }
    fn detach(v: &mut stats::PoolSizeCapture) -> Option<Box<dyn Any>> {
        Some(Box::new(v.take_for_gc()))
    }
}
pub struct AtomicLongDescriptor;
impl TraceDescriptor<stats::AtomicLongValue> for AtomicLongDescriptor {
    fn trace(_: &stats::AtomicLongValue, _: &mut TraceVisitor) {}
    fn detach(_: &mut stats::AtomicLongValue) -> Option<Box<dyn Any>> {
        None
    }
}
pub struct AtomicDoubleDescriptor;
impl TraceDescriptor<stats::AtomicDoubleValue> for AtomicDoubleDescriptor {
    fn trace(_: &stats::AtomicDoubleValue, _: &mut TraceVisitor) {}
    fn detach(_: &mut stats::AtomicDoubleValue) -> Option<Box<dyn Any>> {
        None
    }
}
pub struct DequeDescriptor;
impl TraceDescriptor<PositionDeque> for DequeDescriptor {
    fn trace(v: &PositionDeque, t: &mut TraceVisitor) {
        for x in &v.entries {
            designation::trace_erased(x, t);
        }
    }
    fn detach(v: &mut PositionDeque) -> Option<Box<dyn Any>> {
        Some(Box::new(std::mem::take(&mut v.entries)))
    }
}
pub struct SetDescriptor;
impl TraceDescriptor<PositionSet> for SetDescriptor {
    fn trace(v: &PositionSet, t: &mut TraceVisitor) {
        if let Some(pool) = &v.pool {
            t.owner(pool);
        }
        for x in &v.entries {
            designation::trace_erased(x, t);
        }
    }
    fn detach(v: &mut PositionSet) -> Option<Box<dyn Any>> {
        Some(Box::new((v.pool.take(), std::mem::take(&mut v.entries))))
    }
}
pub struct AllocatorDescriptor;
impl TraceDescriptor<PositionAllocator> for AllocatorDescriptor {
    fn trace(_: &PositionAllocator, _: &mut TraceVisitor) {}
    fn detach(_: &mut PositionAllocator) -> Option<Box<dyn Any>> {
        None
    }
}
pub struct PositionDescriptor;
impl TraceDescriptor<Position3d> for PositionDescriptor {
    fn trace(_: &Position3d, _: &mut TraceVisitor) {}
    fn detach(_: &mut Position3d) -> Option<Box<dyn Any>> {
        None
    }
}

pub(super) fn register_node(
    node: Node,
    work: &mut VecDeque<Node>,
    status: &mut MetadataStatus,
) -> Result<(), RegistrationFault> {
    macro_rules! registered_node {
        ($value:ident,$descriptor:ty,$pending:ident) => {
            *status = merged(
                *status,
                registered(
                    gc::register_existing::<_, $descriptor>(&$value),
                    record(PendingNode::$pending(Rc::downgrade(&$value))),
                )?,
            );
        };
    }
    macro_rules! body {
        ($value:ident,$pending:ident) => {
            match $value.try_borrow() {
                Ok(body) => body,
                Err(_) => {
                    pending(record(PendingNode::$pending(Rc::downgrade(&$value))));
                    *status = merged(*status, MetadataStatus::Deferred);
                    return Ok(());
                }
            }
        };
    }
    match node {
        Node::StatsGlobals(v) => {
            registered_node!(v, GlobalsDescriptor, Globals);
            let b = body!(v, Globals);
            if let Some(x) = b.id_slot() {
                work.push_back(Node::AtomicLong(x.clone()));
            }
            if let Some(x) = b.instance_slot() {
                work.push_back(Node::Statistic(x.clone()));
            }
        }
        Node::Statistic(v) => {
            registered_node!(v, StatisticDescriptor, Statistic);
            let b = body!(v, Statistic);
            if let Some(x) = b.counters_slot() {
                work.push_back(Node::CounterMap(x.clone()));
            }
            if let Some(x) = b.statistics_slot() {
                work.push_back(Node::NumberMap(x.clone()));
            }
            if let Some(x) = b.counter_for_delete_slot() {
                work.push_back(Node::Counter(x.clone()));
            }
        }
        Node::CounterMap(v) => {
            registered_node!(v, CounterMapDescriptor, CounterMap);
            let b = body!(v, CounterMap);
            for x in b.entries().values() {
                work.push_back(Node::Counter(x.clone()));
            }
        }
        Node::NumberMap(v) => {
            registered_node!(v, NumberMapDescriptor, NumberMap);
        }
        Node::Counter(v) => {
            registered_node!(v, CounterDescriptor, Counter);
            let b = body!(v, Counter);
            if let Some(x) = b.value_slot() {
                work.push_back(Node::AtomicDouble(x.clone()));
            }
            if let Some(x) = b.lambda_slot() {
                work.push_back(Node::Capture(x.clone()));
            }
        }
        Node::Capture(v) => {
            registered_node!(v, CaptureDescriptor, Capture);
            let b = body!(v, Capture);
            if let Some(x) = b.owner_slot() {
                if let Ok(pool) = x.clone().downcast::<RefCell<DesignationPositionPool>>() {
                    work.push_back(Node::Pool(pool));
                }
            }
        }
        Node::AtomicLong(v) => {
            registered_node!(v, AtomicLongDescriptor, Long);
        }
        Node::AtomicDouble(v) => {
            registered_node!(v, AtomicDoubleDescriptor, Double);
        }
        Node::PoolDeque(v) => {
            registered_node!(v, DequeDescriptor, Deque);
            let b = body!(v, Deque);
            for x in &b.entries {
                designation::enqueue_erased(x, work, status)?;
            }
        }
        Node::PoolSet(v) => {
            registered_node!(v, SetDescriptor, Set);
            let b = body!(v, Set);
            if let Some(x) = &b.pool {
                work.push_back(Node::Pool(x.clone()));
            }
            for x in &b.entries {
                designation::enqueue_erased(x, work, status)?;
            }
        }
        Node::PoolAllocator(v) => {
            registered_node!(v, AllocatorDescriptor, Allocator);
        }
        Node::Position(v) => {
            registered_node!(v, PositionDescriptor, Position);
        }
        _ => unreachable!("designation nodes are handled by their own branch"),
    }
    Ok(())
}
pub fn observe_statistics(value: stats::StatisticsObserved<'_>) {
    let node = match value {
        stats::StatisticsObserved::ClassStatics(v) => Node::StatsGlobals(v.clone()),
        stats::StatisticsObserved::Statistic(v) => Node::Statistic(v.clone()),
        stats::StatisticsObserved::CounterMap(v) => Node::CounterMap(v.clone()),
        stats::StatisticsObserved::NumberMap(v) => Node::NumberMap(v.clone()),
        stats::StatisticsObserved::Counter(v) => Node::Counter(v.clone()),
        stats::StatisticsObserved::Capture(v) => Node::Capture(v.clone()),
        stats::StatisticsObserved::AtomicLong(v) => Node::AtomicLong(v.clone()),
        stats::StatisticsObserved::AtomicDouble(v) => Node::AtomicDouble(v.clone()),
    };
    let _ = register_graph(node);
}
pub fn install_statistics_observer() {
    stats::install_observer(Some(observe_statistics));
    if let Ok(Some(value)) = stats::try_existing_class_statics() {
        let _ = register_graph(Node::StatsGlobals(value));
    }
}
pub fn observe_pool_deque(v: &PositionDequeRef) {
    let _ = register_graph(Node::PoolDeque(v.clone()));
}
pub fn observe_pool_set(v: &PositionSetRef) {
    let _ = register_graph(Node::PoolSet(v.clone()));
}
pub fn observe_pool_allocator(v: &PositionAllocatorRef) {
    let _ = register_graph(Node::PoolAllocator(v.clone()));
}
pub fn observe_position(v: &Position3dRef) {
    let _ = register_graph(Node::Position(v.clone()));
}

#[cfg(test)]
mod tests;
