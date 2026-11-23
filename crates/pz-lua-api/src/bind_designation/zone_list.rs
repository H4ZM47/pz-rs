//! `DesignationZoneAnimal.getAllZones()` returns the class's own
//! `designationAnimalZoneList`. Lua gets one userdata for that native list.
//! The reads (`size`, `get`, `isEmpty`, `contains`, `indexOf`) go to the
//! list itself on each call, so nothing is copied or replaced. Writes
//! report the missing mutable backend.
//!
//! `DesignationZone.getAllZonesByType(type)` builds a new `ArrayList` in the
//! original, so it returns an ordinary Lua `ArrayList` of the matching zones.

use std::any::Any;
use std::cell::RefCell;
use std::rc::{Rc, Weak};

use pz_kahlua::{LuaError, LuaObject, LuaResult, NativeCall, Thread, Value};
use pz_world::designation::{DesignationList, DesignationListRef, DesignationZoneRef};

use super::{borrowed, capture_context, registry, unsupported, zone_value};

/// The Lua class of the native zone list view. It answers `instanceof`
/// `ArrayList`; its methods are its own, so `java.util.ArrayList`'s Lua
/// methods never see it.
pub const ZONE_LIST: &str = "zombie.iso.areas.DesignationZoneAnimal$ZoneList";

thread_local! {
    static VIEW: RefCell<Option<Weak<ZoneListView>>> = const { RefCell::new(None) };
}

pub struct ZoneListView {
    list: Weak<RefCell<DesignationList>>,
}
impl LuaObject for ZoneListView {
    fn class_name(&self) -> &str {
        ZONE_LIST
    }
    fn super_classes(&self) -> &[&'static str] {
        &["java.util.ArrayList"]
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// The animal zone list of the installed registry, one userdata per list.
fn animal_zone_list() -> LuaResult<Value> {
    let registry = registry(&capture_context()?)?;
    let list = registry
        .try_borrow()
        .map_err(|_| borrowed("Designation registry"))?
        .animal_zones_slot()
        .cloned()
        .ok_or_else(|| unsupported("DesignationZoneAnimal.designationAnimalZoneList is retired"))?;
    let view = VIEW.with(|slot| {
        let mut slot = slot.borrow_mut();
        if let Some(view) = slot.as_ref().and_then(Weak::upgrade) {
            if view.list.upgrade().is_some_and(|l| Rc::ptr_eq(&l, &list)) {
                return view;
            }
        }
        let view = Rc::new(ZoneListView { list: Rc::downgrade(&list) });
        *slot = Some(Rc::downgrade(&view));
        view
    });
    Ok(Value::Object(view))
}

/// The receiver's zones, read from the native list now.
fn zones(c: &NativeCall<'_>) -> LuaResult<Vec<DesignationZoneRef>> {
    let Value::Object(o) = c.arg(0) else {
        return Err(LuaError::runtime("expected the designation zone list"));
    };
    let view = (&*o as &dyn Any)
        .downcast_ref::<ZoneListView>()
        .ok_or_else(|| LuaError::runtime("expected the designation zone list"))?;
    let list: DesignationListRef = view
        .list
        .upgrade()
        .ok_or_else(|| unsupported("DesignationZoneAnimal.designationAnimalZoneList is retired"))?;
    let list = list.try_borrow().map_err(|_| borrowed("Designation zone list"))?;
    match &*list {
        DesignationList::Zones(zones) => Ok(zones.clone()),
        _ => Err(unsupported("Designation zone list holds no zones")),
    }
}

/// The index of `value` in `zones` (`ArrayList.indexOf`: identity, as
/// `DesignationZone` does not override `equals`).
fn index_of(zones: &[DesignationZoneRef], value: &Value) -> Option<usize> {
    let Value::Object(o) = value else { return None };
    let zone = (&**o as &dyn Any).downcast_ref::<super::LuaDesignationZone>()?;
    zones.iter().position(|z| Rc::ptr_eq(z, &zone.zone))
}

pub(super) fn register(thread: &mut Thread) {
    let b = crate::expose::expose_class(thread, ZONE_LIST, None);
    b.method("size", |c| {
        let n = zones(c)?.len();
        Ok(c.push(n as f64))
    });
    b.method("isEmpty", |c| {
        let empty = zones(c)?.is_empty();
        Ok(c.push(empty))
    });
    b.method("get", |c| {
        let zones = zones(c)?;
        let i = crate::expose::int_arg(c, 1);
        let zone = usize::try_from(i).ok().and_then(|i| zones.get(i)).ok_or_else(|| LuaError::Runtime {
            message: format!("Index {i} out of bounds for length {}", zones.len()),
            class: "java.lang.IndexOutOfBoundsException",
        })?;
        let v = zone_value(zone)?;
        Ok(c.push(v))
    });
    b.method("contains", |c| {
        let found = index_of(&zones(c)?, &c.arg(1)).is_some();
        Ok(c.push(found))
    });
    b.method("indexOf", |c| {
        let i = index_of(&zones(c)?, &c.arg(1)).map_or(-1.0, |i| i as f64);
        Ok(c.push(i))
    });
    for name in ["add", "remove", "clear", "set", "addAll", "removeAll", "sort"] {
        b.method(name, |_| Err(unsupported("actual mutable Designation ArrayList Lua backend")));
    }
}

/// `DesignationZoneAnimal.getAllZones()`.
pub(super) fn get_all_zones(c: &mut NativeCall<'_>) -> LuaResult<usize> {
    let v = animal_zone_list()?;
    Ok(c.push(v))
}

/// `DesignationZone.getAllZonesByType(type)`: a new list of the zones in
/// `allZones` whose type equals `type`.
pub(super) fn get_all_zones_by_type(c: &mut NativeCall<'_>) -> LuaResult<usize> {
    let wanted = crate::expose::str_arg(c, 0);
    let registry = registry(&capture_context()?)?;
    let list = registry
        .try_borrow()
        .map_err(|_| borrowed("Designation registry"))?
        .all_zones_slot()
        .cloned()
        .ok_or_else(|| unsupported("DesignationZone.allZones is retired"))?;
    let zones = match &*list.try_borrow().map_err(|_| borrowed("Designation zone list"))? {
        DesignationList::Zones(zones) => zones.clone(),
        _ => return Err(unsupported("Designation zone list holds no zones")),
    };
    let mut out = Vec::new();
    for zone in &zones {
        let matches = {
            let body = zone.try_borrow().map_err(|_| borrowed("DesignationZone type"))?;
            // `type.equals(type)` on the zone's field: a null field throws.
            let ty = body.zone_type.as_deref().ok_or_else(|| LuaError::Runtime {
                message: "Cannot invoke \"String.equals(Object)\" because \"type\" is null".into(),
                class: "java.lang.NullPointerException",
            })?;
            wanted.as_deref() == Some(ty)
        };
        if matches {
            out.push(zone_value(zone)?);
        }
    }
    Ok(c.push(crate::bind_java_util::list_value(out)))
}
