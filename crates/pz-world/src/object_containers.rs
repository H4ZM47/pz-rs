//! `IsoObject.container` and `secondaryContainers` as live shared
//! containers.
//!
//! A loaded object keeps its containers as codec values. When Lua first
//! reaches a container (`getContainerByIndex` and its siblings), the values
//! move into [`RuntimeContainers`], as `IsoMannequin.runtime_container`
//! does. From then on the shared containers are the only state: the codec
//! fields are empty, and the readers below (count, types, save) use the
//! shared ones.
//!
//! Limit: `SharedContainer.kind` is a `String`, so a container whose saved
//! type is null reads as `"none"` once installed.

use std::borrow::Cow;

use pz_inventory::io::{SaveError, SaveResult};
use pz_inventory::shared::{ContainerRef, SharedContainer};

use crate::objects::{IsoObject, ItemContainer};

/// The live `container` and `secondaryContainers` of an object.
#[derive(Debug, Default, PartialEq)]
pub struct RuntimeContainers {
    pub main: Option<ContainerRef>,
    pub secondary: Vec<ContainerRef>,
}

impl RuntimeContainers {
    /// The `getContainerByIndex` order: `container`, then the secondary ones.
    pub fn iter(&self) -> impl Iterator<Item = &ContainerRef> {
        self.main.iter().chain(self.secondary.iter())
    }
}

impl Clone for RuntimeContainers {
    /// Archive/value copy: new containers with copied items and no owner
    /// links (see [`SharedContainer::deep_clone`]).
    fn clone(&self) -> Self {
        Self {
            main: self.main.as_ref().map(|c| c.borrow().deep_clone()),
            secondary: self.secondary.iter().map(|c| c.borrow().deep_clone()).collect(),
        }
    }
}

fn borrowed() -> SaveError {
    SaveError("IsoObject container: BorrowedContainer".into())
}

fn snapshot(c: &ContainerRef) -> SaveResult<ItemContainer> {
    c.try_borrow()
        .map_err(|_| borrowed())?
        .try_to_container()
        .map_err(|e| SaveError(format!("IsoObject container: {e}")))
}

impl IsoObject {
    /// `getContainerCount()`.
    pub fn container_count(&self) -> usize {
        match &self.runtime_containers {
            Some(r) => r.main.is_some() as usize + r.secondary.len(),
            None => self.container.is_some() as usize + self.secondary_containers.len(),
        }
    }

    /// `container != null`.
    pub fn has_main_container(&self) -> bool {
        match &self.runtime_containers {
            Some(r) => r.main.is_some(),
            None => self.container.is_some(),
        }
    }

    /// `getContainerByIndex(i).getType()` for each container, in order.
    pub fn container_types(&self) -> Vec<Option<String>> {
        match &self.runtime_containers {
            Some(r) => r.iter().map(|c| Some(c.borrow().kind.clone())).collect(),
            None => self.containers().map(|c| c.kind.clone()).collect(),
        }
    }

    /// `container.getType()` (`None` without a main container).
    pub fn main_container_type(&self) -> Option<String> {
        match &self.runtime_containers {
            Some(r) => r.main.as_ref().map(|c| c.borrow().kind.clone()),
            None => self.container.as_ref().and_then(|c| c.kind.clone()),
        }
    }

    /// The containers in `getContainerByIndex` order as saved values: the
    /// codec values, or a snapshot of the shared containers.
    pub fn saved_containers(&self) -> SaveResult<Vec<Cow<'_, ItemContainer>>> {
        match &self.runtime_containers {
            Some(r) => r.iter().map(|c| snapshot(c).map(Cow::Owned)).collect(),
            None => Ok(self.containers().map(Cow::Borrowed).collect()),
        }
    }

    /// `container` as a saved value (see [`Self::saved_containers`]).
    pub fn saved_main_container(&self) -> SaveResult<Option<Cow<'_, ItemContainer>>> {
        match &self.runtime_containers {
            Some(r) => r.main.as_ref().map(|c| snapshot(c).map(Cow::Owned)).transpose(),
            None => Ok(self.container.as_ref().map(Cow::Borrowed)),
        }
    }

    /// The shared containers, made from the codec values on first use.
    /// `setup` runs once for each new container (the caller sets `parent`
    /// and `sourceGrid`, which this crate keeps opaque).
    pub fn install_runtime_containers(
        &mut self,
        setup: impl Fn(&ContainerRef),
    ) -> &RuntimeContainers {
        if self.runtime_containers.is_none() {
            let main = self.container.take().map(SharedContainer::from_container);
            let secondary: Vec<_> = std::mem::take(&mut self.secondary_containers)
                .into_iter()
                .map(SharedContainer::from_container)
                .collect();
            let r = RuntimeContainers { main, secondary };
            r.iter().for_each(&setup);
            self.runtime_containers = Some(r);
        }
        self.runtime_containers.as_ref().unwrap()
    }

    /// Removes every container (codec and shared). Drop the result after
    /// releasing object guards: shared containers can own opaque parents.
    #[must_use]
    pub fn take_all_containers(
        &mut self,
    ) -> (Option<ItemContainer>, Vec<ItemContainer>, Option<RuntimeContainers>) {
        (
            self.container.take(),
            std::mem::take(&mut self.secondary_containers),
            self.runtime_containers.take(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::objects::ObjectKind;
    use std::rc::Rc;

    fn object() -> IsoObject {
        let mut o = IsoObject::new(1, ObjectKind::Object, None);
        o.container = Some(ItemContainer::new(Some("fridge")));
        o.secondary_containers.push(ItemContainer::new(Some("freezer")));
        o
    }

    #[test]
    fn install_moves_the_codec_containers_once_and_keeps_the_readers() {
        let mut o = object();
        let before = o.saved_containers().unwrap().iter().map(|c| c.clone().into_owned()).collect::<Vec<_>>();
        let calls = std::cell::Cell::new(0);
        let first = o.install_runtime_containers(|_| calls.set(calls.get() + 1)).main.clone().unwrap();
        let again = o.install_runtime_containers(|_| calls.set(calls.get() + 1)).main.clone().unwrap();
        assert!(Rc::ptr_eq(&first, &again));
        assert_eq!(calls.get(), 2);
        assert!(o.container.is_none() && o.secondary_containers.is_empty());
        assert_eq!(o.container_count(), 2);
        assert!(o.has_main_container());
        assert_eq!(o.main_container_type().as_deref(), Some("fridge"));
        let after = o.saved_containers().unwrap().iter().map(|c| c.clone().into_owned()).collect::<Vec<_>>();
        assert_eq!(after, before);
    }

    #[test]
    fn clone_copies_the_shared_containers_and_take_removes_them() {
        let mut o = object();
        o.install_runtime_containers(|_| {});
        let copy = o.clone();
        let (a, b) = (o.runtime_containers.as_ref().unwrap(), copy.runtime_containers.as_ref().unwrap());
        assert!(!Rc::ptr_eq(a.main.as_ref().unwrap(), b.main.as_ref().unwrap()));
        assert_eq!(copy.container_types(), o.container_types());
        let taken = o.take_all_containers();
        assert!(taken.2.is_some());
        assert_eq!(o.container_count(), 0);
        assert!(!o.has_main_container());
    }
}
