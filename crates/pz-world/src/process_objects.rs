//! IsoCell's ordered update lists use Java object identity. Removals from
//! processIsoObject are deferred until the start of its next update.
use crate::IsoObjectRef;
use std::collections::HashSet;
use std::rc::Rc;

#[derive(Default)]
struct ObjectList {
    objects: Vec<IsoObjectRef>,
    members: HashSet<usize>,
}

impl ObjectList {
    fn add(&mut self, object: &IsoObjectRef) {
        if self.members.insert(Rc::as_ptr(object) as usize) {
            self.objects.push(object.clone());
        }
    }

    fn extract(&mut self, mut remove: impl FnMut(&IsoObjectRef) -> bool) -> Vec<IsoObjectRef> {
        let members = &mut self.members;
        self.objects
            .extract_if(.., |o| {
                if remove(o) {
                    members.remove(&(Rc::as_ptr(o) as usize));
                    true
                } else {
                    false
                }
            })
            .collect()
    }
}

#[derive(Default)]
pub struct ProcessObjects {
    process: ObjectList,
    pending: HashSet<usize>,
    static_updaters: ObjectList,
}

impl ProcessObjects {
    pub fn add(&mut self, object: &IsoObjectRef) {
        self.pending.remove(&(Rc::as_ptr(object) as usize));
        self.process.add(object);
    }

    pub fn queue_remove(&mut self, object: &IsoObjectRef) {
        let key = Rc::as_ptr(object) as usize;
        if self.process.members.contains(&key) {
            self.pending.insert(key);
        }
    }

    pub fn drain_removed(&mut self) -> Vec<IsoObjectRef> {
        if self.pending.is_empty() {
            return Vec::new();
        }
        let pending = std::mem::take(&mut self.pending);
        self.process
            .extract(|o| pending.contains(&(Rc::as_ptr(o) as usize)))
    }

    pub fn process(&self) -> &[IsoObjectRef] {
        &self.process.objects
    }

    pub fn static_updaters(&self) -> &[IsoObjectRef] {
        &self.static_updaters.objects
    }

    pub fn static_updater(&mut self, object: &IsoObjectRef, add: bool) {
        if add {
            self.static_updaters.add(object);
        } else if self
            .static_updaters
            .members
            .contains(&(Rc::as_ptr(object) as usize))
        {
            self.static_updaters.extract(|o| Rc::ptr_eq(o, object));
        }
    }

    pub fn unload_chunk(&mut self, wx: i32, wy: i32) -> Vec<IsoObjectRef> {
        let in_chunk = |o: &IsoObjectRef| {
            o.borrow().square_slot().is_some_and(|sq| {
                let sq = sq.borrow();
                let size = crate::chunk::CHUNK_WIDTH;
                sq.x.div_euclid(size) == wx && sq.y.div_euclid(size) == wy
            })
        };
        let mut retired = self.process.extract(in_chunk);
        for obj in &retired {
            self.pending.remove(&(Rc::as_ptr(obj) as usize));
        }
        retired.extend(self.static_updaters.extract(in_chunk));
        retired
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{IsoObject, ObjectKind, ObjectSlot};

    #[test]
    fn deferred_removal_preserves_order_and_readd_cancels_by_identity() {
        let tile = || ObjectSlot::new(IsoObject::new(7, ObjectKind::Object, None)).ref_handle();
        let (a, b, c) = (tile(), tile(), tile());
        let mut lists = ProcessObjects::default();
        lists.queue_remove(&a);
        lists.add(&a);
        lists.add(&b);
        lists.add(&a);
        lists.queue_remove(&b);
        lists.queue_remove(&a);
        lists.add(&a);
        lists.add(&c);
        assert_eq!(lists.process().len(), 3);
        let retired = lists.drain_removed();
        assert_eq!(retired.len(), 1);
        assert!(Rc::ptr_eq(&retired[0], &b));
        assert!(Rc::ptr_eq(&lists.process()[0], &a));
        assert!(Rc::ptr_eq(&lists.process()[1], &c));
        lists.static_updater(&a, true);
        lists.static_updater(&b, true);
        lists.static_updater(&a, true);
        lists.static_updater(&b, false);
        assert_eq!(lists.static_updaters.objects.len(), 1);
        assert!(Rc::ptr_eq(&lists.static_updaters.objects[0], &a));
    }
}
