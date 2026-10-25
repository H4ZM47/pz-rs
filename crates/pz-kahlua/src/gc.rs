//! Conservative collection of cycles made entirely from Lua tables.
//!
//! Each table allocation is weakly registered. At a host safe point, the
//! collector counts every owning table key, value and metatable slot. Any
//! remaining strong reference is a root, including Rust handles, interpreter
//! stacks, closures, coroutines, native captures and engine objects. Rooted
//! tables and everything reachable from them retain their contents unchanged.
//!
//! This is deliberately a table-only collector. Cycles through closures or
//! engine userdata remain conservatively retained until those allocation
//! types have their own tracing support. In particular, this does not yet
//! reclaim a character whose mod-data points back to that character.
//!
//! Lua `collectgarbage` queues a request instead of sweeping inside a native
//! callback. The interpreter drains requests after outer calls unwind; hosts
//! may also call [`collect_pending`] after their final callback for a frame.
//! Direct coroutine resumes can leave a request pending until that host drain.

use crate::table::Table;
use crate::value::Value;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::{Rc, Weak};

#[derive(Default)]
struct Collector {
    tables: Vec<Weak<RefCell<Table>>>,
    active: bool,
    pending: bool,
}

thread_local! {
    static COLLECTOR: RefCell<Collector> = RefCell::new(Collector::default());
}

pub(crate) fn register_table(table: Weak<RefCell<Table>>) {
    // A host TLS destructor can allocate after this registry was destroyed.
    // Such allocations stay untracked; no further pass can run on that thread.
    let _ = COLLECTOR.try_with(|collector| {
        let mut collector = collector.borrow_mut();
        // Also reclaim dead allocation headers in hosts that never request a
        // collection. Pruning only when the vector would grow is amortized and
        // drops Weak references, never user values or table contents.
        if collector.tables.len() == collector.tables.capacity() {
            let capacity = collector.tables.capacity();
            collector.tables.retain(|table| table.strong_count() != 0);
            // If most entries are still live, pruning alone would leave too
            // little headroom and repeat the linear scan on each allocation.
            if collector.tables.len() > capacity / 2 {
                collector.tables.reserve(capacity);
            }
        }
        collector.tables.push(table);
    });
}

/// Request a table collection at the next host safe point.
pub fn request_collection() {
    let _ = COLLECTOR.try_with(|collector| collector.borrow_mut().pending = true);
}

/// Table counts for a completed pass, not Lua/JVM heap memory statistics.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CollectionStats {
    pub tables_scanned: usize,
    pub tables_cleared: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CollectionResult {
    NotRequested,
    /// A borrow, reentrant call or inconsistent reference count prevented a
    /// pass. The request remains pending and no table contents were changed.
    Deferred,
    Collected(CollectionStats),
}

/// Drain a queued request at a host safe point. Returned Lua values and any
/// other handles held by the caller remain ordinary strong roots.
pub fn collect_pending() -> CollectionResult {
    if COLLECTOR
        .try_with(|collector| collector.borrow().pending)
        .unwrap_or(false)
    {
        collect_tables()
    } else {
        CollectionResult::NotRequested
    }
}

struct ActivePass;

impl Drop for ActivePass {
    fn drop(&mut self) {
        let _ = COLLECTOR.try_with(|collector| collector.borrow_mut().active = false);
    }
}

/// Attempt an immediate table-only pass at a host safe point.
///
/// No user callbacks may be executing while the host invokes this function.
/// A borrowed table defers the entire pass, and destructors may safely allocate
/// tables or request another pass: sweeping is never reentrant. Collection
/// first detaches all dead edges, releases every table borrow, then drops the
/// detached values. Unknown ownership is preserved as an external root.
pub fn collect_tables() -> CollectionResult {
    let started = COLLECTOR.try_with(|collector| {
        let mut collector = collector.borrow_mut();
        if collector.active {
            collector.pending = true;
            false
        } else {
            collector.active = true;
            collector.pending = false;
            true
        }
    });
    if !matches!(started, Ok(true)) {
        return CollectionResult::Deferred;
    }
    let _active = ActivePass;
    let result = collect_registered_tables();
    // No collector-state borrow is held while values or node pins are dropped.
    // Those drops may register new tables or queue a further collection.
    COLLECTOR.with(|collector| {
        collector
            .borrow_mut()
            .tables
            .retain(|table| table.strong_count() != 0);
    });
    match result {
        Ok(stats) => CollectionResult::Collected(stats),
        Err(()) => {
            request_collection();
            CollectionResult::Deferred
        }
    }
}

fn collect_registered_tables() -> Result<CollectionStats, ()> {
    // Exactly one collector pin per allocation. Registration only occurs in
    // TableRef::default, shared by new(), so this list has no duplicate nodes.
    let nodes: Vec<_> = COLLECTOR.with(|collector| {
        let mut collector = collector.borrow_mut();
        collector.tables.retain(|table| table.strong_count() != 0);
        collector.tables.iter().filter_map(Weak::upgrade).collect()
    });
    let indices: HashMap<_, _> = nodes
        .iter()
        .enumerate()
        .map(|(index, table)| (Rc::as_ptr(table) as usize, index))
        .collect();
    let mut edges = vec![Vec::new(); nodes.len()];
    let mut incoming = vec![0usize; nodes.len()];
    for (index, node) in nodes.iter().enumerate() {
        // A mutable borrow also rejects outstanding shared Ref guards. Do not
        // use entries()/keys(): their snapshots would add uncounted Rc pins.
        let table = node.try_borrow_mut().map_err(|_| ())?;
        let mut edge = |target: &crate::TableRef| {
            if let Some(&target_index) = indices.get(&target.ptr_id()) {
                incoming[target_index] += 1;
                edges[index].push(target_index);
            }
        };
        for (key, value) in &table.map {
            if let Value::Table(target) = &key.0 {
                edge(target);
            }
            if let Value::Table(target) = value {
                edge(target);
            }
        }
        if let Some(target) = &table.meta {
            edge(target);
        }
    }

    let mut reachable = vec![false; nodes.len()];
    let mut work = Vec::new();
    for (index, node) in nodes.iter().enumerate() {
        let internal = incoming[index].checked_add(1).ok_or(())?;
        let external = Rc::strong_count(node).checked_sub(internal).ok_or(())?;
        if external != 0 {
            reachable[index] = true;
            work.push(index);
        }
    }
    while let Some(index) = work.pop() {
        for &target in &edges[index] {
            if !reachable[target] {
                reachable[target] = true;
                work.push(target);
            }
        }
    }

    // Acquire every required borrow before changing any table. Keep all node
    // pins until every dead map/metatable has been detached.
    let mut dead = Vec::new();
    for (index, node) in nodes.iter().enumerate() {
        if !reachable[index] {
            dead.push(node.try_borrow_mut().map_err(|_| ())?);
        }
    }
    let mut detached = Vec::with_capacity(dead.len());
    for table in &mut dead {
        detached.push(std::mem::take(&mut **table));
    }
    let stats = CollectionStats {
        tables_scanned: nodes.len(),
        tables_cleared: dead.len(),
    };
    drop(dead);
    drop(detached);
    drop(nodes);
    Ok(stats)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{LuaObject, NativeFunction, TableRef, Thread};
    use std::any::Any;
    use std::cell::Cell;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;

    fn self_cycle() -> Weak<RefCell<Table>> {
        let table = TableRef::new();
        table.set_str("self", table.clone());
        Rc::downgrade(&table.0)
    }

    fn collected() -> CollectionStats {
        match collect_tables() {
            CollectionResult::Collected(stats) => stats,
            other => panic!("collection did not complete: {other:?}"),
        }
    }

    #[test]
    fn default_and_new_cycles_are_collected_and_weak_records_pruned() {
        let table = TableRef::default();
        table.set_str("self", table.clone());
        let default = Rc::downgrade(&table.0);
        drop(table);
        let new = self_cycle();
        assert!(collected().tables_cleared >= 2);
        assert!(default.upgrade().is_none());
        assert!(new.upgrade().is_none());
        assert_eq!(collected().tables_cleared, 0);
        COLLECTOR.with(|collector| assert!(collector.borrow().tables.is_empty()));
    }

    #[test]
    fn acyclic_allocation_headers_do_not_accumulate_without_gc_requests() {
        let rooted = TableRef::new();
        rooted.set_str("kept", 19);
        for _ in 0..10_000 {
            drop(TableRef::new());
        }
        COLLECTOR.with(|collector| assert!(collector.borrow().tables.len() <= 4));
        assert_eq!(rooted.get_str("kept").as_num(), Some(19.0));
    }

    #[test]
    fn mostly_live_registry_reserves_headroom_for_temporary_allocations() {
        let mut roots: Vec<_> = (0..1024).map(|_| TableRef::new()).collect();
        let old_capacity = COLLECTOR.with(|collector| collector.borrow().tables.capacity());
        while roots.len() < old_capacity {
            roots.push(TableRef::new());
        }
        roots[0].set_str("kept", 41);
        // Fill the registry with N-1 live entries and one expired temporary.
        // Repeatedly pruning only that one entry used to scan N records for
        // every subsequent allocation.
        drop(roots.pop().unwrap());
        drop(TableRef::new());
        let new_capacity = COLLECTOR.with(|collector| {
            let collector = collector.borrow();
            assert!(collector.tables.capacity() >= old_capacity + roots.len());
            assert_eq!(collector.tables.len(), old_capacity);
            collector.tables.capacity()
        });
        let temporary_count = (new_capacity - old_capacity) / 2;
        for _ in 0..temporary_count {
            drop(TableRef::new());
        }
        COLLECTOR.with(|collector| {
            let collector = collector.borrow();
            assert_eq!(collector.tables.capacity(), new_capacity);
            assert_eq!(collector.tables.len(), old_capacity + temporary_count);
        });
        collected();
        assert_eq!(roots[0].get_str("kept").as_num(), Some(41.0));
        assert_eq!(roots.len(), old_capacity - 1);
    }

    struct TeardownAllocation(Arc<AtomicBool>);

    thread_local! {
        static EARLY_TLS: RefCell<Option<TeardownAllocation>> = const { RefCell::new(None) };
    }

    impl Drop for TeardownAllocation {
        fn drop(&mut self) {
            let result = std::panic::catch_unwind(|| {
                let table = TableRef::new();
                table.set_str("value", 37);
                request_collection();
                assert_eq!(collect_tables(), CollectionResult::Deferred);
                assert_eq!(collect_pending(), CollectionResult::NotRequested);
                assert_eq!(table.get_str("value").as_num(), Some(37.0));
            });
            self.0.store(result.is_ok(), Ordering::SeqCst);
        }
    }

    #[test]
    fn allocation_and_gc_entry_points_are_safe_after_collector_tls_teardown() {
        let succeeded = Arc::new(AtomicBool::new(false));
        let observed = succeeded.clone();
        std::thread::spawn(move || {
            // TLS destructors run in reverse initialization order. This owner
            // allocates after COLLECTOR has already been destroyed.
            EARLY_TLS.with(|slot| *slot.borrow_mut() = Some(TeardownAllocation(observed)));
            drop(TableRef::new());
        })
        .join()
        .unwrap();
        assert!(succeeded.load(Ordering::SeqCst));
    }

    #[test]
    fn repeated_owning_slots_preserve_external_table_and_raw_rc_roots() {
        let table = TableRef::new();
        table.rawset(table.clone().into(), table.clone().into());
        table.set_metatable(Some(table.clone()));
        let weak = Rc::downgrade(&table.0);
        assert_eq!(collected().tables_cleared, 0);
        assert_eq!(table.size(), 1);
        assert!(table.metatable().unwrap().ptr_eq(&table));
        let raw = table.0.clone();
        drop(table);
        assert_eq!(collected().tables_cleared, 0);
        assert_eq!(raw.borrow().map.len(), 1);
        drop(raw);
        assert_eq!(collected().tables_cleared, 1);
        assert!(weak.upgrade().is_none());
    }

    #[test]
    fn external_child_keeps_its_cycle_but_not_unreachable_parent_cycle() {
        let parent = TableRef::new();
        let child = TableRef::new();
        parent.set_str("child1", child.clone());
        parent.set_str("child2", child.clone());
        child.set_str("parent", parent.clone());
        let parent_weak = Rc::downgrade(&parent.0);
        drop(parent);
        assert_eq!(collected().tables_cleared, 0);
        assert_eq!(child.get_str("parent").as_table().unwrap().size(), 2);
        child.rawset(Value::str("parent"), Value::Nil);
        assert!(parent_weak.upgrade().is_none());

        let cycle = TableRef::new();
        cycle.set_str("self", cycle.clone());
        cycle.set_str("rooted child", child.clone());
        child.set_str("kept", 23);
        let cycle_weak = Rc::downgrade(&cycle.0);
        drop(cycle);
        assert_eq!(collected().tables_cleared, 1);
        assert!(cycle_weak.upgrade().is_none());
        assert_eq!(child.get_str("kept").as_num(), Some(23.0));
    }

    #[test]
    fn table_keys_and_metatables_are_strong_edges() {
        let key_cycle = TableRef::new();
        key_cycle.rawset(key_cycle.clone().into(), true.into());
        let key_weak = Rc::downgrade(&key_cycle.0);
        drop(key_cycle);
        let meta_cycle = TableRef::new();
        meta_cycle.set_metatable(Some(meta_cycle.clone()));
        let meta_weak = Rc::downgrade(&meta_cycle.0);
        drop(meta_cycle);
        assert_eq!(collected().tables_cleared, 2);
        assert!(key_weak.upgrade().is_none());
        assert!(meta_weak.upgrade().is_none());
    }

    #[test]
    fn native_capture_is_an_external_root() {
        let table = TableRef::new();
        let child = TableRef::new();
        child.set_str("kept", 71);
        table.set_str("self", table.clone());
        table.set_str("child", child);
        let weak = Rc::downgrade(&table.0);
        let native = NativeFunction::new("root", move |call| Ok(call.push(table.clone())));
        assert_eq!(collected().tables_cleared, 0);
        assert_eq!(weak.upgrade().unwrap().borrow().map.len(), 2);
        drop(native);
        assert_eq!(collected().tables_cleared, 2);
        assert!(weak.upgrade().is_none());
    }

    #[test]
    fn any_outstanding_borrow_defers_the_whole_pass_and_keeps_request_pending() {
        let live = TableRef::new();
        let dead = self_cycle();
        let borrowed = live.0.borrow();
        request_collection();
        assert_eq!(collect_pending(), CollectionResult::Deferred);
        assert!(dead.upgrade().is_some());
        drop(borrowed);
        let borrowed = live.0.borrow_mut();
        assert_eq!(collect_pending(), CollectionResult::Deferred);
        assert!(dead.upgrade().is_some());
        drop(borrowed);
        assert!(matches!(collect_pending(), CollectionResult::Collected(_)));
        assert!(dead.upgrade().is_none());
        assert_eq!(collect_pending(), CollectionResult::NotRequested);
    }

    struct ReentrantDrop {
        dropped: Rc<Cell<bool>>,
        table: Weak<RefCell<Table>>,
    }

    impl LuaObject for ReentrantDrop {
        fn class_name(&self) -> &str {
            "test.ReentrantDrop"
        }
        fn as_any(&self) -> &dyn Any {
            self
        }
    }

    impl Drop for ReentrantDrop {
        fn drop(&mut self) {
            // Every dead table borrow must have ended before a host object is
            // dropped, even if that object reborrows its former containing map.
            if let Some(table) = self.table.upgrade() {
                assert!(table.try_borrow_mut().is_ok());
            }
            let new = TableRef::new();
            new.set_str("self", new.clone());
            request_collection();
            assert_eq!(collect_tables(), CollectionResult::Deferred);
            self.dropped.set(true);
        }
    }

    #[test]
    fn destructors_may_reborrow_allocate_and_request_a_nonreentrant_pass() {
        let table = TableRef::new();
        let weak = Rc::downgrade(&table.0);
        let dropped = Rc::new(Cell::new(false));
        table.set_str("self", table.clone());
        table.set_str(
            "object",
            Value::Object(Rc::new(ReentrantDrop {
                dropped: dropped.clone(),
                table: weak.clone(),
            })),
        );
        drop(table);
        assert_eq!(collected().tables_cleared, 1);
        assert!(dropped.get());
        assert!(weak.upgrade().is_none());
        assert!(matches!(
            collect_pending(),
            CollectionResult::Collected(CollectionStats {
                tables_cleared: 1,
                ..
            })
        ));
        assert_eq!(collect_pending(), CollectionResult::NotRequested);
    }

    #[test]
    fn host_owned_mod_data_survives_unrelated_environment_retirement() {
        // Models a retained GameTime mod-data table across a menu reset. Its
        // owner is intentionally outside the table graph and remains a root.
        let mod_data = TableRef::new();
        let environment = TableRef::new();
        environment.set_str("_G", environment.clone());
        mod_data.set_str("environment", environment.clone());
        mod_data.set_str("value", 47);
        let weak = Rc::downgrade(&environment.0);
        drop(environment);
        collected();
        assert_eq!(mod_data.get_str("value").as_num(), Some(47.0));
        assert!(weak.upgrade().is_some());
        mod_data.rawset(Value::str("environment"), Value::Nil);
        collected();
        assert!(weak.upgrade().is_none());
        assert_eq!(mod_data.get_str("value").as_num(), Some(47.0));
    }

    #[test]
    fn native_nested_call_does_not_drain_until_outer_host_boundary() {
        let weak = self_cycle();
        let inside = weak.clone();
        let nested = Value::Native(NativeFunction::new("nested", |_| Ok(0)));
        let parent = Value::Native(NativeFunction::new("parent", move |call| {
            request_collection();
            call.thread.call(&nested, &[])?;
            assert!(inside.upgrade().is_some());
            Ok(0)
        }));
        let mut thread = Thread::new(TableRef::new());
        thread.call(&parent, &[]).unwrap();
        assert!(weak.upgrade().is_none());
        assert_eq!(collect_pending(), CollectionResult::NotRequested);
    }

    #[test]
    fn lua_collection_options_and_returned_values_preserve_live_tables() {
        let mut thread = Thread::new(TableRef::new());
        crate::stdlib::install(&mut thread, None).unwrap();
        for option in ["", "'collect'", "'step'"] {
            let weak = self_cycle();
            let source =
                format!("local t = {{value = 13}}; t.self = t; collectgarbage({option}); return t");
            let function =
                crate::stdlib::load_string(&source, None, thread.globals().clone()).unwrap();
            let out = thread.call(&function, &[]).unwrap();
            assert!(weak.upgrade().is_none());
            let table = out[0].as_table().unwrap();
            assert_eq!(table.get_str("value").as_num(), Some(13.0));
            assert!(table.get_str("self").as_table().unwrap().ptr_eq(table));
        }
        let count = thread.globals().get_str("collectgarbage");
        let out = thread.call(&count, &[Value::str("count")]).unwrap();
        assert_eq!(out.len(), 3);
        assert!(out.iter().all(|value| value.as_num() == Some(0.0)));
    }

    #[test]
    fn error_unwind_drains_requests_without_erasing_a_table_valued_error() {
        let mut thread = Thread::new(TableRef::new());
        crate::stdlib::install(&mut thread, None).unwrap();
        let weak = self_cycle();
        let function = crate::stdlib::load_string(
            "local t = {code = 29}; t.self = t; collectgarbage(); error(t)",
            None,
            thread.globals().clone(),
        )
        .unwrap();
        let (message, _, _) = thread.pcall_ex(&function, &[]).unwrap_err();
        assert!(weak.upgrade().is_none());
        let table = message.as_table().unwrap();
        assert_eq!(table.get_str("code").as_num(), Some(29.0));
        assert!(table.get_str("self").as_table().unwrap().ptr_eq(table));
        assert_eq!(collect_pending(), CollectionResult::NotRequested);
    }
}
