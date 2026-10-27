//! Native ObjectPoolCounter/PoolStatistic ownership, single class-runtime thread.
//!
//! The two maps have one authoritative contents store. Only put/get and the
//! empty update are supported here: a nonempty native HashMap iteration is an
//! explicit capability boundary, never Rust HashMap iteration order.
use pz_kahlua::table::{RetiredTableValues, TableAccessError};
use pz_kahlua::TableRef;
use std::any::Any;
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};

pub type ClassStaticsRef = Rc<RefCell<ClassStatics>>;
pub type StatisticRef = Rc<RefCell<PoolStatistic>>;
pub type CounterMapRef = Rc<RefCell<CounterMap>>;
pub type NumberMapRef = Rc<RefCell<NumberMap>>;
pub type CounterRef = Rc<RefCell<Counter>>;
pub type CaptureRef = Rc<RefCell<PoolSizeCapture>>;
pub type AtomicLongRef = Rc<RefCell<AtomicLongValue>>;
pub type AtomicDoubleRef = Rc<RefCell<AtomicDoubleValue>>;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StatisticsError {
    Borrowed(&'static str),
    Unsupported(&'static str),
}
pub type StatisticsResult<T> = Result<T, StatisticsError>;
fn borrowed(phase: &'static str) -> StatisticsError {
    StatisticsError::Borrowed(phase)
}
fn unsupported(phase: &'static str) -> StatisticsError {
    StatisticsError::Unsupported(phase)
}

/// Keep this payload until outside all world/statistic/map/table/GC guards.
/// It owns real temporary pins/removed slots; it is not a runtime mirror.
#[derive(Default)]
#[must_use]
pub struct StatisticsRetired {
    owners: Vec<Rc<dyn Any>>,
    payloads: Vec<Box<dyn Any>>,
}
impl StatisticsRetired {
    pub fn keep<T: 'static>(&mut self, value: &Rc<RefCell<T>>) {
        self.owners.push(value.clone());
    }
    pub fn keep_erased(&mut self, value: &Rc<dyn Any>) {
        self.owners.push(value.clone());
    }
    pub fn payload(&mut self, value: impl Any) {
        self.payloads.push(Box::new(value));
    }
}

pub struct AtomicLongValue(AtomicI64);
impl AtomicLongValue {
    pub fn get(&self) -> i64 {
        self.0.load(Ordering::SeqCst)
    }
    pub fn increment_and_get(&self) -> i64 {
        self.0.fetch_add(1, Ordering::SeqCst).wrapping_add(1)
    }
}
pub struct AtomicDoubleValue(AtomicU64);
impl AtomicDoubleValue {
    pub fn get(&self) -> f64 {
        f64::from_bits(self.0.load(Ordering::SeqCst))
    }
    pub fn set(&self, value: f64) {
        self.0.store(value.to_bits(), Ordering::SeqCst);
    }
}
pub struct ClassStatics {
    id: Option<AtomicLongRef>,
    instance: Option<StatisticRef>,
    initializing: bool,
}
impl ClassStatics {
    pub fn id_slot(&self) -> Option<&AtomicLongRef> {
        self.id.as_ref()
    }
    pub fn instance_slot(&self) -> Option<&StatisticRef> {
        self.instance.as_ref()
    }
    pub fn take_for_gc(&mut self) -> (Option<AtomicLongRef>, Option<StatisticRef>) {
        (self.id.take(), self.instance.take())
    }
}
#[derive(Default)]
pub struct CounterMap {
    entries: HashMap<String, CounterRef>,
}
impl CounterMap {
    pub fn entries(&self) -> &HashMap<String, CounterRef> {
        &self.entries
    }
    pub fn take_for_gc(&mut self) -> HashMap<String, CounterRef> {
        std::mem::take(&mut self.entries)
    }
}
#[derive(Default)]
pub struct NumberMap {
    values: HashMap<String, f64>,
}
impl NumberMap {
    pub fn values(&self) -> &HashMap<String, f64> {
        &self.values
    }
    pub fn take_for_gc(&mut self) -> HashMap<String, f64> {
        std::mem::take(&mut self.values)
    }
}
pub struct PoolStatistic {
    pub name: Option<String>,
    counters: Option<CounterMapRef>,
    statistics: Option<NumberMapRef>,
    local_table: Option<TableRef>,
    remote_table: Option<TableRef>,
    counter_for_delete: Option<CounterRef>,
}
impl PoolStatistic {
    pub fn counters_slot(&self) -> Option<&CounterMapRef> {
        self.counters.as_ref()
    }
    pub fn statistics_slot(&self) -> Option<&NumberMapRef> {
        self.statistics.as_ref()
    }
    pub fn local_table_slot(&self) -> Option<&TableRef> {
        self.local_table.as_ref()
    }
    pub fn remote_table_slot(&self) -> Option<&TableRef> {
        self.remote_table.as_ref()
    }
    pub fn counter_for_delete_slot(&self) -> Option<&CounterRef> {
        self.counter_for_delete.as_ref()
    }
    pub fn take_for_gc(
        &mut self,
    ) -> (
        Option<CounterMapRef>,
        Option<NumberMapRef>,
        Option<TableRef>,
        Option<TableRef>,
        Option<CounterRef>,
    ) {
        (
            self.counters.take(),
            self.statistics.take(),
            self.local_table.take(),
            self.remote_table.take(),
            self.counter_for_delete.take(),
        )
    }
}
pub type PoolSizeReader = fn(&Rc<dyn Any>) -> StatisticsResult<f64>;
/// The actual capturing lambda owns exactly this actual allocation. The static
/// read function has no captured state; tracing never invokes the function.
pub struct PoolSizeCapture {
    owner: Option<Rc<dyn Any>>,
    read: PoolSizeReader,
}
impl PoolSizeCapture {
    pub fn owner_slot(&self) -> Option<&Rc<dyn Any>> {
        self.owner.as_ref()
    }
    pub fn take_for_gc(&mut self) -> Option<Rc<dyn Any>> {
        self.owner.take()
    }
}
pub struct Counter {
    pub name: Option<String>,
    pub tooltip: Option<String>,
    pub units: Option<String>,
    pub default_value: f64,
    pub perishable: bool,
    value: Option<AtomicDoubleRef>,
    lambda: Option<CaptureRef>,
}
impl Counter {
    pub fn value_slot(&self) -> Option<&AtomicDoubleRef> {
        self.value.as_ref()
    }
    pub fn lambda_slot(&self) -> Option<&CaptureRef> {
        self.lambda.as_ref()
    }
    pub fn take_for_gc(&mut self) -> (Option<AtomicDoubleRef>, Option<CaptureRef>) {
        (self.value.take(), self.lambda.take())
    }
}
pub enum StatisticsObserved<'a> {
    ClassStatics(&'a ClassStaticsRef),
    Statistic(&'a StatisticRef),
    CounterMap(&'a CounterMapRef),
    NumberMap(&'a NumberMapRef),
    Counter(&'a CounterRef),
    Capture(&'a CaptureRef),
    AtomicLong(&'a AtomicLongRef),
    AtomicDouble(&'a AtomicDoubleRef),
}
pub type StatisticsObserver = for<'a> fn(StatisticsObserved<'a>);
thread_local! {
    static GLOBALS:RefCell<Option<ClassStaticsRef>>=const {RefCell::new(None)};
    static OBSERVER:Cell<Option<StatisticsObserver>>=const {Cell::new(None)};
}
/// Metadata only: no class initialization, Native callbacks or semantic writes.
pub fn install_observer(value: Option<StatisticsObserver>) -> Option<StatisticsObserver> {
    OBSERVER.with(|slot| slot.replace(value))
}
fn observe(value: StatisticsObserved<'_>) {
    if let Some(f) = OBSERVER.with(Cell::get) {
        f(value);
    }
}

/// Pure static-slot inspection for metadata installation. Does not initialize
/// either class, reset a counter, or run any captured value function.
pub fn try_existing_class_statics() -> StatisticsResult<Option<ClassStaticsRef>> {
    GLOBALS.with(|slot| {
        slot.try_borrow()
            .map(|value| value.clone())
            .map_err(|_| borrowed("pool class globals metadata"))
    })
}
/// ObjectPoolCounter.<clinit>: real strong static ID owner, shared independently
/// of worlds/ScriptManagers. This service admits one native runtime thread;
/// arbitrary multi-thread/class-loader JVM sharing is not implemented.
pub fn try_class_statics(retired: &mut StatisticsRetired) -> StatisticsResult<ClassStaticsRef> {
    if let Some(value) = GLOBALS.with(|slot| {
        slot.try_borrow()
            .map(|v| v.clone())
            .map_err(|_| borrowed("pool class globals"))
    })? {
        retired.keep(&value);
        return Ok(value);
    }
    let globals = Rc::new(RefCell::new(ClassStatics {
        id: None,
        instance: None,
        initializing: false,
    }));
    retired.keep(&globals);
    observe(StatisticsObserved::ClassStatics(&globals));
    let id = Rc::new(RefCell::new(AtomicLongValue(AtomicI64::new(0))));
    retired.keep(&id);
    observe(StatisticsObserved::AtomicLong(&id));
    globals
        .try_borrow_mut()
        .map_err(|_| borrowed("ObjectPoolCounter.id publication"))?
        .id = Some(id);
    GLOBALS.with(|slot| {
        let mut slot = slot
            .try_borrow_mut()
            .map_err(|_| borrowed("pool class globals publication"))?;
        if slot.is_some() {
            return Err(unsupported("reentrant pool class initialization"));
        }
        *slot = Some(globals.clone());
        Ok(())
    })?;
    observe(StatisticsObserved::ClassStatics(&globals));
    Ok(globals)
}
/// PoolStatistic.<clinit>, including two real permanent TableRef allocations.
pub fn try_instance(
    globals: &ClassStaticsRef,
    retired: &mut StatisticsRetired,
) -> StatisticsResult<StatisticRef> {
    retired.keep(globals);
    {
        let mut g = globals
            .try_borrow_mut()
            .map_err(|_| borrowed("PoolStatistic.instance"))?;
        if let Some(value) = &g.instance {
            retired.keep(value);
            return Ok(value.clone());
        }
        if g.initializing {
            return Err(unsupported("partial PoolStatistic class initialization"));
        }
        g.initializing = true;
    }
    let value = Rc::new(RefCell::new(PoolStatistic {
        name: None,
        counters: None,
        statistics: None,
        local_table: None,
        remote_table: None,
        counter_for_delete: None,
    }));
    retired.keep(&value);
    observe(StatisticsObserved::Statistic(&value));
    let counters = Rc::new(RefCell::new(CounterMap::default()));
    retired.keep(&counters);
    observe(StatisticsObserved::CounterMap(&counters));
    value
        .try_borrow_mut()
        .map_err(|_| borrowed("Statistic.counters publication"))?
        .counters = Some(counters);
    let numbers = Rc::new(RefCell::new(NumberMap::default()));
    retired.keep(&numbers);
    observe(StatisticsObserved::NumberMap(&numbers));
    value
        .try_borrow_mut()
        .map_err(|_| borrowed("Statistic.statistics publication"))?
        .statistics = Some(numbers);
    let local = TableRef::new();
    retired.payload(local.clone());
    value
        .try_borrow_mut()
        .map_err(|_| borrowed("Statistic.localTable publication"))?
        .local_table = Some(local);
    let remote = TableRef::new();
    retired.payload(remote.clone());
    value
        .try_borrow_mut()
        .map_err(|_| borrowed("Statistic.remoteTable publication"))?
        .remote_table = Some(remote);
    value
        .try_borrow_mut()
        .map_err(|_| borrowed("Statistic.name publication"))?
        .name = Some("pool".into());
    // IStatistic.init()V is an authenticated genuine empty default method.
    observe(StatisticsObserved::Statistic(&value));
    {
        let mut g = globals
            .try_borrow_mut()
            .map_err(|_| borrowed("PoolStatistic.instance publication"))?;
        g.instance = Some(value.clone());
        g.initializing = false;
    }
    observe(StatisticsObserved::ClassStatics(globals));
    Ok(value)
}
pub fn try_counter_by_name(
    statistic: &StatisticRef,
    name: &str,
    retired: &mut StatisticsRetired,
) -> StatisticsResult<Option<CounterRef>> {
    retired.keep(statistic);
    let map = statistic
        .try_borrow()
        .map_err(|_| borrowed("Statistic counter map field"))?
        .counters
        .clone()
        .ok_or_else(|| unsupported("null Statistic.counters"))?;
    retired.keep(&map);
    let value = map
        .try_borrow()
        .map_err(|_| borrowed("Statistic counter map get"))?
        .entries
        .get(name)
        .cloned();
    if let Some(value) = &value {
        retired.keep(value);
    }
    Ok(value)
}
fn store(
    statistic: &StatisticRef,
    counter: &CounterRef,
    retired: &mut StatisticsRetired,
) -> StatisticsResult<()> {
    retired.keep(statistic);
    retired.keep(counter);
    let map = statistic
        .try_borrow()
        .map_err(|_| borrowed("Statistic.store map field"))?
        .counters
        .clone()
        .ok_or_else(|| unsupported("null Statistic.store map"))?;
    retired.keep(&map);
    let name = counter
        .try_borrow()
        .map_err(|_| borrowed("Statistic.store counter name"))?
        .name
        .clone()
        .ok_or_else(|| unsupported("nullable native Counter map key"))?;
    let old = map
        .try_borrow_mut()
        .map_err(|_| borrowed("Statistic.store map put"))?
        .entries
        .insert(name, counter.clone());
    if let Some(old) = old {
        retired.keep(&old);
    }
    observe(StatisticsObserved::CounterMap(&map));
    Ok(())
}
/// Native ObjectPoolCounter superclass body. The actual raw Counter receiver
/// exists before getInstance; ID increments AFTER getInstance succeeds; capture
/// and all Counter fields publish BEFORE the global map store.
pub fn try_register_object_pool_counter<T: 'static>(
    globals: &ClassStaticsRef,
    pool: &Rc<RefCell<T>>,
    name: Option<&str>,
    read: PoolSizeReader,
    retired: &mut StatisticsRetired,
) -> StatisticsResult<CounterRef> {
    retired.keep(globals);
    retired.keep(pool);
    let counter = Rc::new(RefCell::new(Counter {
        name: None,
        tooltip: None,
        units: None,
        default_value: 0.0,
        perishable: false,
        value: None,
        lambda: None,
    }));
    retired.keep(&counter);
    observe(StatisticsObserved::Counter(&counter));
    let statistic = try_instance(globals, retired)?;
    let id = globals
        .try_borrow()
        .map_err(|_| borrowed("ObjectPoolCounter.id field"))?
        .id
        .clone()
        .ok_or_else(|| unsupported("null ObjectPoolCounter.id"))?;
    retired.keep(&id);
    let number = id
        .try_borrow()
        .map_err(|_| borrowed("ObjectPoolCounter.id increment"))?
        .increment_and_get();
    let key = format!("{}-{number}", name.unwrap_or("null"));
    let owner: Rc<dyn Any> = pool.clone();
    let capture = Rc::new(RefCell::new(PoolSizeCapture {
        owner: Some(owner),
        read,
    }));
    retired.keep(&capture);
    observe(StatisticsObserved::Capture(&capture));
    {
        let mut c = counter
            .try_borrow_mut()
            .map_err(|_| borrowed("Counter string fields"))?;
        c.name = Some(key);
        c.tooltip = Some("Objects in the pool".into());
        c.units = Some("number".into());
    }
    let value = Rc::new(RefCell::new(AtomicDoubleValue(AtomicU64::new(
        0.0f64.to_bits(),
    ))));
    retired.keep(&value);
    observe(StatisticsObserved::AtomicDouble(&value));
    {
        let mut c = counter
            .try_borrow_mut()
            .map_err(|_| borrowed("Counter value/default/lambda fields"))?;
        c.value = Some(value);
        c.default_value = 0.0;
        c.lambda = Some(capture);
        c.perishable = true;
    }
    observe(StatisticsObserved::Counter(&counter));
    store(&statistic, &counter, retired)?;
    Ok(counter)
}
pub fn try_counter_get(
    counter: &CounterRef,
    retired: &mut StatisticsRetired,
) -> StatisticsResult<f64> {
    retired.keep(counter);
    let lambda = counter
        .try_borrow()
        .map_err(|_| borrowed("Counter.get lambda test"))?
        .lambda
        .clone();
    if let Some(lambda) = lambda {
        retired.keep(&lambda);
        let (owner, read) = {
            let l = lambda
                .try_borrow()
                .map_err(|_| borrowed("Counter.get capture"))?;
            (
                l.owner
                    .clone()
                    .ok_or_else(|| unsupported("Counter.get detached capture"))?,
                l.read,
            )
        };
        retired.keep_erased(&owner);
        return read(&owner);
    }
    let value = counter
        .try_borrow()
        .map_err(|_| borrowed("Counter.get atomic field"))?
        .value
        .clone()
        .ok_or_else(|| unsupported("Counter.get null AtomicDouble"))?;
    retired.keep(&value);
    let number = value
        .try_borrow()
        .map_err(|_| borrowed("Counter.get AtomicDouble"))?
        .get();
    Ok(number)
}
pub fn try_counter_clear(
    counter: &CounterRef,
    retired: &mut StatisticsRetired,
) -> StatisticsResult<()> {
    retired.keep(counter);
    let (value, default) = {
        let c = counter
            .try_borrow()
            .map_err(|_| borrowed("Counter.clear fields"))?;
        (
            c.value
                .clone()
                .ok_or_else(|| unsupported("Counter.clear null AtomicDouble"))?,
            c.default_value,
        )
    };
    retired.keep(&value);
    value
        .try_borrow()
        .map_err(|_| borrowed("Counter.clear AtomicDouble"))?
        .set(default);
    Ok(())
}
/// A raw actual map operation for the host/native public-map surface. Removed
/// counters are queued, not dropped under the map or caller's world guard.
pub fn try_remove_counter(
    statistic: &StatisticRef,
    name: &str,
    retired: &mut StatisticsRetired,
) -> StatisticsResult<()> {
    retired.keep(statistic);
    let map = statistic
        .try_borrow()
        .map_err(|_| borrowed("counter remove map field"))?
        .counters
        .clone()
        .ok_or_else(|| unsupported("null counter remove map"))?;
    retired.keep(&map);
    let old = map
        .try_borrow_mut()
        .map_err(|_| borrowed("counter remove map"))?
        .entries
        .remove(name);
    if let Some(old) = old {
        retired.keep(&old);
    }
    observe(StatisticsObserved::CounterMap(&map));
    Ok(())
}
pub fn try_update(
    statistic: &StatisticRef,
    retired: &mut StatisticsRetired,
) -> StatisticsResult<()> {
    retired.keep(statistic);
    let numbers = statistic
        .try_borrow()
        .map_err(|_| borrowed("Statistic.update statistics field"))?
        .statistics
        .clone()
        .ok_or_else(|| unsupported("null Statistic.statistics"))?;
    retired.keep(&numbers);
    numbers
        .try_borrow_mut()
        .map_err(|_| borrowed("Statistic.update statistics clear"))?
        .values
        .clear();
    observe(StatisticsObserved::NumberMap(&numbers));
    let local = statistic
        .try_borrow()
        .map_err(|_| borrowed("Statistic.update localTable field"))?
        .local_table
        .clone()
        .ok_or_else(|| unsupported("null Statistic.localTable"))?;
    retired.payload(local.clone());
    let mut values = RetiredTableValues::new();
    let wiped = local.try_wipe(&mut values);
    retired.payload(values);
    wiped.map_err(|e| match e {
        TableAccessError::Borrowed => borrowed("Statistic.update localTable wipe"),
        TableAccessError::TraversalLimit => unsupported("Statistic.update table traversal"),
    })?;
    let counters = statistic
        .try_borrow()
        .map_err(|_| borrowed("Statistic.update counters field"))?
        .counters
        .clone()
        .ok_or_else(|| unsupported("null Statistic.counters"))?;
    retired.keep(&counters);
    if !counters
        .try_borrow()
        .map_err(|_| borrowed("Statistic.update counters values"))?
        .entries
        .is_empty()
    {
        return Err(unsupported(
            "Statistic.update native HashMap values iterator",
        ));
    }
    // Genuine empty native iteration. A previously retained deletion slot still
    // follows the original remove-then-null order. There are no callbacks here.
    let deletion = statistic
        .try_borrow()
        .map_err(|_| borrowed("Statistic.update deletion field"))?
        .counter_for_delete
        .clone();
    if let Some(counter) = deletion {
        retired.keep(&counter);
        let name = counter
            .try_borrow()
            .map_err(|_| borrowed("Statistic.update deletion name"))?
            .name
            .clone()
            .ok_or_else(|| unsupported("nullable native deletion key"))?;
        try_remove_counter(statistic, &name, retired)?;
        let old = statistic
            .try_borrow_mut()
            .map_err(|_| borrowed("Statistic.update clear deletion field"))?
            .counter_for_delete
            .take();
        if let Some(old) = old {
            retired.keep(&old);
        }
    }
    observe(StatisticsObserved::Statistic(statistic));
    Ok(())
}

#[cfg(test)]
mod tests;
