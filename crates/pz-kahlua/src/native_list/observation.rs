//! Intrinsic canonical host descriptors. No higher crate installs alternatives.
use super::*;
use std::cell::Cell;
use std::collections::{HashSet, VecDeque};
use std::rc::Weak;

pub enum ListObserved<'a> {
    List(&'a NativeListRef),
    Array(&'a NativeArrayRef),
    Iterator(&'a NativeIteratorRef),
}
/// Pure metadata notification only: no Lua/user callbacks, semantic mutations,
/// collection or held foreign guards. Node publication is not yet complete.
pub type ListObserver = for<'a> fn(ListObserved<'a>);
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ObservationStatus {
    Complete,
    Deferred,
}
thread_local! {
    static OBSERVER:Cell<Option<ListObserver>>=const {Cell::new(None)};
    static PENDING:RefCell<Vec<Pending>>=const {RefCell::new(Vec::new())};
}
pub fn install_observer(observer: Option<ListObserver>) -> Option<ListObserver> {
    OBSERVER.with(|v| v.replace(observer))
}
fn notify(v: ListObserved<'_>) {
    if let Ok(Some(f)) = OBSERVER.try_with(Cell::get) {
        f(v);
    }
}
struct ListDescriptor;
impl gc::TraceDescriptor<NativeArrayList> for ListDescriptor {
    fn trace(v: &NativeArrayList, t: &mut gc::TraceVisitor) {
        if let Some(a) = &v.element_data {
            t.owner(&a.owner);
        }
    }
    fn detach(v: &mut NativeArrayList) -> Option<Box<dyn std::any::Any>> {
        Some(Box::new(v.take_for_gc()))
    }
}
struct ArrayDescriptor;
impl gc::TraceDescriptor<NativeObjectArray> for ArrayDescriptor {
    fn trace(v: &NativeObjectArray, t: &mut gc::TraceVisitor) {
        if let Some(slots) = &v.slots {
            for value in slots.iter() {
                t.value(value);
            }
        }
    }
    fn detach(v: &mut NativeObjectArray) -> Option<Box<dyn std::any::Any>> {
        Some(Box::new(v.take_for_gc()))
    }
}
struct IteratorDescriptor;
impl gc::TraceDescriptor<NativeArrayListItr> for IteratorDescriptor {
    fn trace(v: &NativeArrayListItr, t: &mut gc::TraceVisitor) {
        if let Some(list) = &v.list {
            t.owner(list);
        }
    }
    fn detach(v: &mut NativeArrayListItr) -> Option<Box<dyn std::any::Any>> {
        Some(Box::new(v.take_for_gc()))
    }
}
enum Pending {
    List(Weak<RefCell<NativeArrayList>>),
    Array(Weak<RefCell<NativeObjectArray>>, i32),
    Iterator(Weak<RefCell<NativeArrayListItr>>),
}
enum Node {
    List(NativeListRef),
    Array(NativeArrayRef),
    Iterator(NativeIteratorRef),
}
impl Node {
    fn key(&self) -> (&'static str, usize) {
        match self {
            Self::List(v) => ("list", Rc::as_ptr(v) as usize),
            Self::Array(v) => ("array", v.ptr_id()),
            Self::Iterator(v) => ("iterator", Rc::as_ptr(v) as usize),
        }
    }
    fn weak(&self) -> Pending {
        match self {
            Self::List(v) => Pending::List(Rc::downgrade(v)),
            Self::Array(v) => Pending::Array(Rc::downgrade(&v.owner), v.length),
            Self::Iterator(v) => Pending::Iterator(Rc::downgrade(v)),
        }
    }
}
impl Pending {
    fn upgrade(self) -> Option<Node> {
        match self {
            Self::List(v) => v.upgrade().map(Node::List),
            Self::Array(v, length) => v
                .upgrade()
                .map(|owner| Node::Array(NativeArrayRef { owner, length })),
            Self::Iterator(v) => v.upgrade().map(Node::Iterator),
        }
    }
}
fn pending(v: Pending) {
    let _ = PENDING.try_with(|p| {
        if let Ok(mut p) = p.try_borrow_mut() {
            p.push(v);
        }
    });
}
fn register(root: Node) -> Result<ObservationStatus, gc::RegistrationError> {
    let mut work = VecDeque::from([root]);
    let mut seen = HashSet::new();
    let mut status = ObservationStatus::Complete;
    while let Some(node) = work.pop_front() {
        if !seen.insert(node.key()) {
            continue;
        }
        let result = match &node {
            Node::List(v) => gc::register_existing::<_, ListDescriptor>(v),
            Node::Array(v) => gc::register_existing::<_, ArrayDescriptor>(&v.owner),
            Node::Iterator(v) => gc::register_existing::<_, IteratorDescriptor>(v),
        };
        match result {
            Ok(_) => {}
            Err(gc::RegistrationError::RegistryUnavailable) => {
                pending(node.weak());
                status = ObservationStatus::Deferred;
                continue;
            }
            Err(e) => return Err(e),
        }
        match &node {
            Node::List(v) => match v.try_borrow() {
                Ok(b) => {
                    if let Some(a) = &b.element_data {
                        work.push_back(Node::Array(a.clone()));
                    }
                }
                Err(_) => {
                    pending(node.weak());
                    status = ObservationStatus::Deferred;
                }
            },
            Node::Iterator(v) => match v.try_borrow() {
                Ok(b) => {
                    if let Some(a) = &b.list {
                        work.push_back(Node::List(a.clone()));
                    }
                }
                Err(_) => {
                    pending(node.weak());
                    status = ObservationStatus::Deferred;
                }
            },
            Node::Array(_) => {}
        }
    }
    Ok(status)
}
pub fn observe_list(list: &NativeListRef) -> Result<ObservationStatus, gc::RegistrationError> {
    let result = register(Node::List(list.clone()));
    notify(ListObserved::List(list));
    result
}
pub fn observe_array(array: &NativeArrayRef) -> Result<ObservationStatus, gc::RegistrationError> {
    let result = register(Node::Array(array.clone()));
    notify(ListObserved::Array(array));
    result
}
pub fn observe_iterator(
    iterator: &NativeIteratorRef,
) -> Result<ObservationStatus, gc::RegistrationError> {
    let result = register(Node::Iterator(iterator.clone()));
    notify(ListObserved::Iterator(iterator));
    result
}
pub fn retry_observation() -> Result<ObservationStatus, gc::RegistrationError> {
    let pending = PENDING
        .try_with(|p| p.try_borrow_mut().map(|mut p| std::mem::take(&mut *p)))
        .ok()
        .and_then(Result::ok);
    let Some(pending) = pending else {
        return Ok(ObservationStatus::Deferred);
    };
    let mut status = ObservationStatus::Complete;
    for p in pending {
        if let Some(root) = p.upgrade() {
            if register(root)? == ObservationStatus::Deferred {
                status = ObservationStatus::Deferred;
            }
        }
    }
    Ok(status)
}
