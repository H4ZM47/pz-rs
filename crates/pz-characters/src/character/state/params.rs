//! `State.Param` and the character's state-machine parameter maps
//! (`StateMachineComponent.stateMachineParams`, read through
//! `IsoGameCharacter.getStateMachineParams(stateClass)`).

use std::any::Any;
use std::collections::HashMap;

use super::StateId;

/// `State.Param<T>`: a typed key owned by one state class, with the value
/// `get` stores when the key is missing (`computeIfAbsent`).
pub struct Param<T: 'static> {
    /// `fromClazz`.
    pub state: StateId,
    /// `name`.
    pub name: &'static str,
    /// The default supplier.
    pub default: fn() -> T,
}

impl<T: 'static> Clone for Param<T> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<T: 'static> Copy for Param<T> {}

impl<T: Clone + 'static> Param<T> {
    pub const fn new(state: StateId, name: &'static str, default: fn() -> T) -> Self {
        Param { state, name, default }
    }

    /// `get(owner)`: the stored value, storing the default first when
    /// missing.
    pub fn get(&self, params: &mut StateParams) -> T {
        self.get_or(params, self.default)
    }

    /// `get(owner, defaultSupplier)`.
    pub fn get_or(&self, params: &mut StateParams, default: impl FnOnce() -> T) -> T {
        let map = params.map_mut(self.state);
        let v = map.entry(self.name).or_insert_with(|| Box::new(default()));
        match v.downcast_ref::<T>() {
            Some(t) => t.clone(),
            // A value of another type under the same name: the original's
            // unchecked cast would throw; keep the default.
            None => {
                let d = (self.default)();
                *v = Box::new(d.clone());
                d
            }
        }
    }

    /// The stored value without storing the default.
    pub fn peek(&self, params: &StateParams) -> Option<T> {
        params.maps.get(&self.state)?.get(self.name)?.downcast_ref::<T>().cloned()
    }

    /// `set(owner, value)`.
    pub fn set(&self, params: &mut StateParams, v: T) {
        params.map_mut(self.state).insert(self.name, Box::new(v));
    }

    /// `remove(owner)`.
    pub fn remove(&self, params: &mut StateParams) -> Option<T> {
        params.maps.get_mut(&self.state)?.remove(self.name).and_then(|b| b.downcast::<T>().ok()).map(|b| *b)
    }
}

/// The per-character parameter maps, one per state class.
#[derive(Default)]
pub struct StateParams {
    maps: HashMap<StateId, HashMap<&'static str, Box<dyn Any>>>,
}

impl std::fmt::Debug for StateParams {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut m = f.debug_map();
        for (k, v) in &self.maps {
            m.entry(&k.0, &v.keys().collect::<Vec<_>>());
        }
        m.finish()
    }
}

impl StateParams {
    fn map_mut(&mut self, state: StateId) -> &mut HashMap<&'static str, Box<dyn Any>> {
        self.maps.entry(state).or_default()
    }

    /// `IsoGameCharacter.clear(stateClass)`.
    pub fn clear(&mut self, state: StateId) {
        if let Some(m) = self.maps.get_mut(&state) {
            m.clear();
        }
    }

    /// Whether `state` has a value under `name`.
    pub fn contains(&self, state: StateId, name: &str) -> bool {
        self.maps.get(&state).is_some_and(|m| m.contains_key(name))
    }

    /// The number of values stored for `state`.
    pub fn len(&self, state: StateId) -> usize {
        self.maps.get(&state).map_or(0, |m| m.len())
    }

    pub fn is_empty(&self, state: StateId) -> bool {
        self.len(state) == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const S: StateId = StateId("t.S");
    const A: Param<i32> = Param::new(S, "a", || 7);
    const B: Param<Option<String>> = Param::new(S, "b", || None);

    #[test]
    fn get_stores_the_default() {
        let mut p = StateParams::default();
        assert_eq!(A.peek(&p), None);
        assert_eq!(A.get(&mut p), 7);
        assert_eq!(A.peek(&p), Some(7));
        A.set(&mut p, 3);
        assert_eq!(A.get_or(&mut p, || 9), 3);
        assert_eq!(B.get(&mut p), None);
        B.set(&mut p, Some("x".into()));
        assert_eq!(p.len(S), 2);
        assert_eq!(B.remove(&mut p), Some(Some("x".into())));
        p.clear(S);
        assert!(p.is_empty(S));
        assert_eq!(A.get_or(&mut p, || 9), 9);
    }
}
