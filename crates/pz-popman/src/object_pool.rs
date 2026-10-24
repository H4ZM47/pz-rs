//! `zombie.popman.ObjectPool<T>`: a bounded free list. Objects come back
//! out in the order they were released (`ArrayDeque.removeFirst`).
//!
//! The `DebugOptions.checks.objectPoolContains` double-release check is
//! [`ObjectPool::check_contains`]; with it off (the default) a value can be
//! released twice.

use std::collections::VecDeque;

/// `ObjectPool.DEFAULT_MAX_SIZE`.
pub const DEFAULT_MAX_SIZE: usize = 1024;

pub struct ObjectPool<T> {
    pub name: String,
    allocator: Option<Box<dyn FnMut() -> T>>,
    pool: VecDeque<T>,
    max_size: usize,
    /// `DebugOptions.instance.checks.objectPoolContains`.
    pub check_contains: bool,
}

impl<T: PartialEq> ObjectPool<T> {
    /// `new ObjectPool(alloc, name, maxSize)`.
    pub fn new(allocator: Option<Box<dyn FnMut() -> T>>, name: &str, max_size: usize) -> ObjectPool<T> {
        ObjectPool { name: name.to_string(), allocator, pool: VecDeque::new(), max_size, check_contains: false }
    }

    /// `setMaxSize`: drops the oldest pooled objects beyond `size`.
    pub fn set_max_size(&mut self, size: usize) {
        self.max_size = size;
        while self.pool.len() > size {
            self.pool.pop_front();
        }
    }

    /// `alloc()`: a pooled object, or a new one from the allocator.
    /// Panics without an allocator when the pool is empty (the original
    /// throws `UnsupportedOperationException`).
    pub fn alloc(&mut self) -> T {
        match self.pool.pop_front() {
            Some(o) => o,
            None => (self.allocator.as_mut().expect("Allocator is null"))(),
        }
    }

    /// `release(obj)`.
    pub fn release(&mut self, obj: T) {
        if self.pool.len() >= self.max_size {
            return;
        }
        if self.check_contains && self.pool.contains(&obj) {
            return;
        }
        self.pool.push_back(obj);
    }

    /// `releaseAll(objs)`.
    pub fn release_all(&mut self, objs: impl IntoIterator<Item = T>) {
        for o in objs {
            self.release(o);
        }
    }

    pub fn clear(&mut self) {
        self.pool.clear();
    }

    /// `size()`.
    pub fn size(&self) -> usize {
        self.pool.len()
    }

    /// `forEach`.
    pub fn iter(&self) -> impl Iterator<Item = &T> {
        self.pool.iter()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fifo_and_cap() {
        let mut next = 0;
        let mut p = ObjectPool::new(
            Some(Box::new(move || {
                next += 1;
                next
            })),
            "t",
            2,
        );
        assert_eq!(p.alloc(), 1);
        p.release(7);
        p.release(8);
        p.release(9);
        assert_eq!(p.size(), 2);
        assert_eq!(p.alloc(), 7);
        p.release(8);
        assert_eq!(p.size(), 2);
        p.check_contains = true;
        p.set_max_size(5);
        p.release(8);
        assert_eq!(p.size(), 2);
        p.set_max_size(1);
        assert_eq!(p.alloc(), 8);
        assert_eq!(p.alloc(), 2);
    }
}
