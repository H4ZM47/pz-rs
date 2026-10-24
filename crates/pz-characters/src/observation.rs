//! `zombie.characters.traits.ObservationFactory` and its `Observation`.

use std::cell::RefCell;
use std::rc::Rc;

use pz_scripts::java::JHashMap;
use pz_scripts::ScriptResult;

use crate::npe;

/// `ObservationFactory.Observation`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Observation {
    pub trait_id: Option<String>,
    pub name: Option<String>,
    pub description: Option<String>,
    /// `mutuallyExclusive`.
    pub mutually_exclusive: Vec<Option<String>>,
}

pub type ObservationRef = Rc<RefCell<Observation>>;

impl Observation {
    /// `new Observation(tr, name, desc)`.
    pub fn new(trait_id: Option<&str>, name: Option<&str>, description: Option<&str>) -> Self {
        Observation {
            trait_id: trait_id.map(str::to_string),
            name: name.map(str::to_string),
            description: description.map(str::to_string),
            mutually_exclusive: Vec::new(),
        }
    }

    /// `getLabel` / `getLeftLabel`.
    pub fn label(&self) -> Option<&str> {
        self.name.as_deref()
    }
}

/// `ObservationFactory`: `observationMap` (a `HashMap<String, Observation>`).
#[derive(Default)]
pub struct ObservationFactory {
    pub observation_map: JHashMap<ObservationRef>,
}

impl ObservationFactory {
    /// `addObservation`.
    pub fn add_observation(&mut self, type_: &str, name: Option<&str>, desc: Option<&str>) {
        let o = Observation::new(Some(type_), name, desc);
        self.observation_map.insert(type_.to_string(), Rc::new(RefCell::new(o)));
    }

    /// `getObservation`.
    pub fn get_observation(&self, name: &str) -> Option<ObservationRef> {
        self.observation_map.get(name).cloned()
    }

    /// `setMutualExclusive`; an unknown observation is a
    /// `NullPointerException` (after `a`'s list was updated if `a` exists).
    pub fn set_mutual_exclusive(&self, a: &str, b: &str) -> ScriptResult {
        let oa = self.observation_map.get(a).ok_or_else(npe)?;
        oa.borrow_mut().mutually_exclusive.push(Some(b.to_string()));
        let ob = self.observation_map.get(b).ok_or_else(npe)?;
        ob.borrow_mut().mutually_exclusive.push(Some(a.to_string()));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn observations() {
        let mut f = ObservationFactory::default();
        f.add_observation("Calm", Some("Calm"), Some("d1"));
        f.add_observation("Jumpy", Some("Jumpy"), None);
        f.set_mutual_exclusive("Calm", "Jumpy").unwrap();
        let calm = f.get_observation("Calm").unwrap();
        assert_eq!(calm.borrow().mutually_exclusive, [Some("Jumpy".to_string())]);
        assert_eq!(calm.borrow().label(), Some("Calm"));
        assert!(f.get_observation("Nope").is_none());
        assert!(f.set_mutual_exclusive("Calm", "Nope").is_err());
        assert_eq!(calm.borrow().mutually_exclusive.len(), 2);
        f.add_observation("Calm", Some("Other"), None);
        assert!(!Rc::ptr_eq(&calm, &f.get_observation("Calm").unwrap()));
    }
}
