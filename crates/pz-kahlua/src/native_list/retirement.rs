//! One-generation safe release of physical owners; no native semantic hooks.
use super::*;
use std::any::Any;
use std::cell::Cell;

#[derive(Default)]
pub(super) struct Payload {
    values: Vec<Value>,
    owners: Vec<Rc<dyn Any>>,
}
impl Payload {
    pub(super) fn values(values: Vec<Value>) -> Self {
        Self {
            values,
            owners: Vec::new(),
        }
    }
    pub(super) fn keep_list(&mut self, v: &NativeListRef) {
        self.owners.push(v.clone());
    }
    pub(super) fn keep_array(&mut self, v: &NativeArrayRef) {
        self.owners.push(v.owner.clone());
    }
    fn keep_list_n(&mut self, v: &NativeListNRef) {
        self.owners.push(v.clone());
    }
    fn keep_iterator(&mut self, v: &NativeIteratorRef) {
        self.owners.push(v.clone());
    }
    fn is_empty(&self) -> bool {
        self.values.is_empty() && self.owners.is_empty()
    }
}
thread_local! {
    static RETIRED:RefCell<Vec<Payload>>=const {RefCell::new(Vec::new())};
    static CONSERVATIVE_REJECTIONS:Cell<usize>=const {Cell::new(0)};
}
/// A rejected enqueue must not final-drop arbitrary owners under its caller's
/// foreign guard. At TLS teardown/reentrant queue borrow, conservatively leak
/// that payload and count the occurrence, rather than invoke callbacks there.
/// Normal producers never hold the queue while dropping owners.
pub(super) fn retire_payload(payload: Payload) {
    if payload.is_empty() {
        return;
    }
    let mut pending = Some(payload);
    let _ = RETIRED.try_with(|queue| {
        if let Ok(mut queue) = queue.try_borrow_mut() {
            queue.push(pending.take().unwrap());
        }
    });
    if let Some(payload) = pending {
        let _ = CONSERVATIVE_REJECTIONS.try_with(|v| v.set(v.get().saturating_add(1)));
        std::mem::forget(payload);
    }
}
/// Actual inputs, replaced slots and native locals survive success, failure
/// and unwinding. Drop schedules one payload; it never clears a native node.
#[derive(Default)]
#[must_use = "keep this sink through the complete operation and outer host guard"]
pub struct NativeListRetired {
    payload: Payload,
}
impl NativeListRetired {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn keep_value(&mut self, value: &Value) {
        self.payload.values.push(value.clone());
    }
    pub fn keep_list(&mut self, value: &NativeListRef) {
        self.payload.keep_list(value);
    }
    pub fn keep_array(&mut self, value: &NativeArrayRef) {
        self.payload.keep_array(value);
    }
    pub fn keep_list_n(&mut self, value: &NativeListNRef) {
        self.payload.keep_list_n(value);
    }
    pub fn keep_iterator(&mut self, value: &NativeIteratorRef) {
        self.payload.keep_iterator(value);
    }
    pub fn is_empty(&self) -> bool {
        self.payload.is_empty()
    }
    pub fn append(&mut self, other: &mut Self) {
        self.payload.values.append(&mut other.payload.values);
        self.payload.owners.append(&mut other.payload.owners);
    }
}
impl Drop for NativeListRetired {
    fn drop(&mut self) {
        retire_payload(std::mem::take(&mut self.payload));
    }
}
/// One generation only. Destructors can enqueue work for a later safe tail.
/// The shared VM gate prevents overlap with table drains, GC and deferrals.
pub fn drain_retired() -> usize {
    let Some(_guard) = gc::try_enter_retirement_drain() else {
        return 0;
    };
    let payloads = RETIRED
        .try_with(|queue| queue.try_borrow_mut().map(|mut q| std::mem::take(&mut *q)))
        .ok()
        .and_then(Result::ok);
    let Some(payloads) = payloads else {
        return 0;
    };
    let n = payloads
        .iter()
        .map(|p| p.values.len() + p.owners.len())
        .sum();
    drop(payloads);
    n
}
