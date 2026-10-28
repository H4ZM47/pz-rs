//! Typed index arenas standing in for the native `ObjectPool<T>` /
//! `std::deque` pools. Objects are addressed by small integer ids instead
//! of pointers; a released slot is handed out again by the next `alloc`
//! (last released first, like the native pools).

use std::marker::PhantomData;

/// A typed index into a [`Pool`].
pub struct Id<T> {
    raw: u32,
    _marker: PhantomData<fn() -> T>,
}

impl<T> Id<T> {
    #[inline]
    pub(crate) fn new(raw: u32) -> Self {
        Id {
            raw,
            _marker: PhantomData,
        }
    }

    /// The raw slot index. Stable for the lifetime of the object, so it
    /// doubles as the native `id` field where one is used as a map key.
    #[inline]
    pub fn raw(self) -> u32 {
        self.raw
    }
}

impl<T> Clone for Id<T> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<T> Copy for Id<T> {}
impl<T> PartialEq for Id<T> {
    fn eq(&self, other: &Self) -> bool {
        self.raw == other.raw
    }
}
impl<T> Eq for Id<T> {}
impl<T> PartialOrd for Id<T> {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}
impl<T> Ord for Id<T> {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.raw.cmp(&other.raw)
    }
}
impl<T> std::hash::Hash for Id<T> {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.raw.hash(state)
    }
}
impl<T> std::fmt::Debug for Id<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "#{}", self.raw)
    }
}

/// Slot arena with a LIFO free list.
pub struct Pool<T> {
    items: Vec<T>,
    free: Vec<u32>,
}

impl<T> Default for Pool<T> {
    fn default() -> Self {
        Pool {
            items: Vec::new(),
            free: Vec::new(),
        }
    }
}

impl<T> Pool<T> {
    pub fn alloc(&mut self, value: T) -> Id<T> {
        if let Some(raw) = self.free.pop() {
            self.items[raw as usize] = value;
            Id::new(raw)
        } else {
            self.items.push(value);
            Id::new((self.items.len() - 1) as u32)
        }
    }

    pub fn release(&mut self, id: Id<T>) {
        debug_assert!(!self.free.contains(&id.raw));
        self.free.push(id.raw);
    }

    #[inline]
    pub fn get(&self, id: Id<T>) -> &T {
        &self.items[id.raw as usize]
    }

    #[inline]
    pub fn get_mut(&mut self, id: Id<T>) -> &mut T {
        &mut self.items[id.raw as usize]
    }

    /// Number of live objects.
    pub fn live(&self) -> usize {
        self.items.len() - self.free.len()
    }
}

impl<T> std::ops::Index<Id<T>> for Pool<T> {
    type Output = T;
    #[inline]
    fn index(&self, id: Id<T>) -> &T {
        self.get(id)
    }
}

impl<T> std::ops::IndexMut<Id<T>> for Pool<T> {
    #[inline]
    fn index_mut(&mut self, id: Id<T>) -> &mut T {
        self.get_mut(id)
    }
}
