//! Lua identity for actual complete world-object allocations.
//!
//! The caches own only weak metadata. A userdata owns exactly one real
//! `IsoObjectRef`; it never resolves an object through a square, id or index.

use std::any::Any;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::{Rc, Weak};

use pz_entity::runtime_load::RuntimeHostError;
use pz_entity::{ComponentRef, EntityOwner, RuntimeError};
use pz_kahlua::{LuaError, LuaObject, LuaResult, Thread, Value};
use pz_world::{IsoObject, IsoObjectRef};

pub const ISO_OBJECT: &str = "zombie.iso.IsoObject";
pub const WORLD_INVENTORY_OBJECT: &str = "zombie.iso.objects.IsoWorldInventoryObject";
const GAME_ENTITY: &str = "zombie.entity.GameEntity";

pub struct LuaWorldObject {
    pub(crate) object: IsoObjectRef,
    class: &'static str,
}

impl LuaObject for LuaWorldObject {
    fn class_name(&self) -> &str {
        self.class
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn super_classes(&self) -> &[&'static str] {
        match self.class {
            ISO_OBJECT => &[GAME_ENTITY],
            "zombie.iso.IsoPushableObject" => {
                &["zombie.iso.IsoMovingObject", ISO_OBJECT, GAME_ENTITY]
            }
            "zombie.iso.objects.IsoWheelieBin" => &[
                "zombie.iso.IsoPushableObject",
                "zombie.iso.IsoMovingObject",
                ISO_OBJECT,
                GAME_ENTITY,
            ],
            "zombie.iso.objects.IsoRadio" | "zombie.iso.objects.IsoTelevision" => {
                &["zombie.iso.objects.IsoWaveSignal", ISO_OBJECT, GAME_ENTITY]
            }
            _ => &[ISO_OBJECT, GAME_ENTITY],
        }
    }
}

struct ClassStamp {
    object: Weak<RefCell<IsoObject>>,
    class: &'static str,
    fault: Option<NativeClassFault>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NativeClassFault {
    pub address: usize,
    pub expected: &'static str,
    pub observed: &'static str,
}
impl std::fmt::Display for NativeClassFault {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "native world class at {:x}: {} changed to {}",
            self.address, self.expected, self.observed
        )
    }
}
impl NativeClassFault {
    fn error(self) -> LuaError {
        LuaError::Runtime {
            class: "pz.host.InvalidState",
            message: self.to_string(),
        }
    }
}

thread_local! {
    static CLASSES: RefCell<HashMap<usize, ClassStamp>> = RefCell::new(HashMap::new());
    static USERDATA: RefCell<HashMap<usize, Weak<LuaWorldObject>>> = RefCell::new(HashMap::new());
    static CLASS_FAULTS: RefCell<Vec<NativeClassFault>> = const { RefCell::new(Vec::new()) };
    #[cfg(test)]
    static CLASS_PRUNES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    #[cfg(test)]
    static USERDATA_PRUNES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// Weak metadata is pruned only before capacity growth, not on every world
/// allocation or cache hit. Surviving live entries reserve geometrically.
fn prune_for_insert<V>(cache: &mut HashMap<usize, V>, alive: impl Fn(&V) -> bool) -> bool {
    if cache.len() != cache.capacity() {
        return false;
    }
    let capacity = cache.capacity();
    cache.retain(|_, value| alive(value));
    if cache.len() > capacity / 2 {
        cache.reserve(capacity.max(1));
    }
    true
}

pub fn take_native_class_faults() -> Vec<NativeClassFault> {
    CLASS_FAULTS
        .try_with(|faults| {
            faults
                .try_borrow_mut()
                .ok()
                .map(|mut faults| std::mem::take(&mut *faults))
        })
        .ok()
        .flatten()
        .unwrap_or_default()
}

fn record_class_fault(fault: NativeClassFault) {
    let _ = CLASS_FAULTS.try_with(|faults| {
        if let Ok(mut faults) = faults.try_borrow_mut() {
            faults.push(fault);
        }
    });
}

pub(crate) fn borrowed(operation: &str) -> LuaError {
    LuaError::Runtime {
        class: "pz.host.Borrowed",
        message: operation.to_owned(),
    }
}
pub(crate) fn unsupported(operation: impl Into<String>) -> LuaError {
    LuaError::Runtime {
        class: "pz.host.Unsupported",
        message: operation.into(),
    }
}
pub(crate) fn host_error(error: RuntimeHostError) -> LuaError {
    match error {
        RuntimeHostError::Borrowed(operation) => borrowed(operation),
        RuntimeHostError::Unsupported { operation, .. } => unsupported(operation),
        RuntimeHostError::InvalidHostState(operation) => LuaError::Runtime {
            class: "pz.host.InvalidState",
            message: operation.to_owned(),
        },
        RuntimeHostError::Storage(error) => runtime_error(error),
        RuntimeHostError::NativeException(error) => {
            unsupported(format!("world item native exception translation: {error}"))
        }
    }
}
pub(crate) fn runtime_error(error: RuntimeError) -> LuaError {
    match error {
        RuntimeError::BorrowedComponent { .. } | RuntimeError::BorrowedTransient(_) => {
            borrowed(&error.to_string())
        }
        _ => unsupported(error.to_string()),
    }
}

/// Allocation-time metadata, before cycles and independently of GC registry
/// availability. Native class identity cannot change when a body is borrowed.
pub(crate) fn remember_class(object: &IsoObjectRef, class: &'static str) -> LuaResult<()> {
    CLASSES
        .try_with(|classes| {
            let mut classes = classes
                .try_borrow_mut()
                .map_err(|_| borrowed("world native-class metadata"))?;
            let key = Rc::as_ptr(object) as usize;
            if let Some(stamp) = classes.get_mut(&key) {
                if let Some(fault) = stamp.fault {
                    return Err(fault.error());
                }
                if stamp.class != class {
                    let fault = NativeClassFault {
                        address: key,
                        expected: stamp.class,
                        observed: class,
                    };
                    stamp.fault = Some(fault);
                    record_class_fault(fault);
                    return Err(fault.error());
                }
            } else {
                let scanned =
                    prune_for_insert(&mut classes, |stamp| stamp.object.strong_count() != 0);
                #[cfg(test)]
                if scanned {
                    CLASS_PRUNES.with(|count| count.set(count.get() + 1));
                }
                #[cfg(not(test))]
                let _ = scanned;
                classes.insert(
                    key,
                    ClassStamp {
                        object: Rc::downgrade(object),
                        class,
                        fault: None,
                    },
                );
            }
            Ok(())
        })
        .unwrap_or_else(|_| Err(unsupported("world native-class metadata unavailable")))
}

fn class_of(object: &IsoObjectRef) -> LuaResult<&'static str> {
    CLASSES
        .try_with(|classes| {
            let classes = classes
                .try_borrow()
                .map_err(|_| borrowed("world native-class metadata"))?;
            let stamp = classes
                .get(&(Rc::as_ptr(object) as usize))
                .filter(|stamp| stamp.object.strong_count() != 0)
                .ok_or_else(|| unsupported("world allocation has no native-class stamp"))?;
            match stamp.fault {
                Some(fault) => Err(fault.error()),
                None => Ok(stamp.class),
            }
        })
        .unwrap_or_else(|_| Err(unsupported("world native-class metadata unavailable")))
}

pub(crate) fn ensure_class(object: &IsoObjectRef, class: &'static str) -> LuaResult<()> {
    remember_class(object, class)
}

pub(crate) fn object_from_value(value: &Value) -> Option<&LuaWorldObject> {
    match value {
        Value::Object(object) => (&**object as &dyn Any).downcast_ref::<LuaWorldObject>(),
        _ => None,
    }
}

pub(crate) fn item_argument(value: &Value) -> LuaResult<pz_inventory::shared::ItemRef> {
    if let Value::Object(value) = value {
        if let Some(item) =
            (&**value as &dyn Any).downcast_ref::<crate::bind_items::LuaInventoryItem>()
        {
            return Ok(item.item.clone());
        }
    }
    Err(LuaError::runtime("expected InventoryItem receiver"))
}

pub(crate) fn register_classes(thread: &mut Thread) {
    crate::expose::expose_class(thread, ISO_OBJECT, Some(GAME_ENTITY));
    crate::expose::expose_class(thread, WORLD_INVENTORY_OBJECT, Some(ISO_OBJECT));
}

pub(crate) fn object_value(object: &IsoObjectRef) -> LuaResult<Value> {
    let class = class_of(object)?;
    let userdata = USERDATA
        .try_with(|cache| -> LuaResult<Rc<LuaWorldObject>> {
            let mut cache = cache
                .try_borrow_mut()
                .map_err(|_| borrowed("world userdata cache"))?;
            let key = Rc::as_ptr(object) as usize;
            if let Some(value) = cache.get(&key).and_then(Weak::upgrade) {
                if Rc::ptr_eq(&value.object, object) {
                    return Ok(value);
                }
            }
            cache.remove(&key);
            let scanned = prune_for_insert(&mut cache, |weak| weak.strong_count() != 0);
            #[cfg(test)]
            if scanned {
                USERDATA_PRUNES.with(|count| count.set(count.get() + 1));
            }
            #[cfg(not(test))]
            let _ = scanned;
            let value = Rc::new(LuaWorldObject {
                object: Rc::clone(object),
                class,
            });
            cache.insert(key, Rc::downgrade(&value));
            Ok(value)
        })
        .map_err(|_| unsupported("world userdata cache unavailable"))??;
    // Register typed userdata before erasure; cache hits retry registration.
    crate::component_gc::register_world_userdata(&userdata)
        .map_err(|error| unsupported(error.to_string()))?;
    // Registration can discover an inconsistent adopted body. It must not
    // return an old cached class as a successfully validated receiver.
    class_of(object)?;
    Ok(Value::Object(userdata))
}

pub(crate) fn get_world_item(item: &pz_inventory::shared::ItemRef) -> LuaResult<Value> {
    let owner = pz_inventory::world_item::get_world_item_ref(item).map_err(host_error)?;
    match owner {
        None => Ok(Value::Nil),
        Some(owner) => {
            let object = owner.try_handle::<IsoObject>().ok_or_else(|| {
                unsupported("InventoryItem.worldItem has an unsupported actual owner type")
            })?;
            if class_of(&object)? != WORLD_INVENTORY_OBJECT {
                return Err(unsupported(
                    "InventoryItem.worldItem is not an actual IsoWorldInventoryObject",
                ));
            }
            object_value(&object)
        }
    }
}

pub(crate) fn set_world_item(item: &pz_inventory::shared::ItemRef, value: &Value) -> LuaResult<()> {
    let object = match value {
        Value::Nil => None,
        Value::Native(_) => {
            return Err(unsupported(
                "native function concrete class in world-item argument diagnostic",
            ))
        }
        _ => {
            let object = object_from_value(value)
                .filter(|object| object.class == WORLD_INVENTORY_OBJECT)
                .ok_or_else(|| {
                    LuaError::runtime(format!(
                        "expected argument of type IsoWorldInventoryObject, got {}",
                        crate::expose::simple_name(&pz_kahlua::stdlib::java_class_name(value))
                    ))
                })?;
            class_of(&object.object)?;
            Some(EntityOwner::from_ref(&object.object))
        }
    };
    pz_inventory::world_item::set_world_item_ref(item, object.as_ref()).map_err(host_error)?;
    if let Some(object) = object.and_then(|owner| owner.try_handle::<IsoObject>()) {
        class_of(&object)?;
    }
    Ok(())
}

/// GameEntity dispatch uses the real allocation belonging to each userdata.
/// An encoded or unsupported component authority is never component absence.
pub(crate) fn fluid_component(value: &Value) -> LuaResult<Option<ComponentRef>> {
    if let Some(world) = object_from_value(value) {
        class_of(&world.object)?;
        let body = world
            .object
            .try_borrow()
            .map_err(|_| borrowed("world fluid component getter"))?;
        return body
            .components
            .get_ref(pz_entity::ComponentType::FluidContainer)
            .map_err(runtime_error);
    }
    if let Value::Object(value) = value {
        if let Some(item) =
            (&**value as &dyn Any).downcast_ref::<crate::bind_items::LuaInventoryItem>()
        {
            let body = item
                .item
                .try_borrow()
                .map_err(|_| borrowed("inventory fluid component getter"))?;
            return body.get_fluid_ref().map_err(runtime_error);
        }
    }
    Err(unsupported(
        "GameEntity getter requires an actual item or world-object userdata",
    ))
}

#[cfg(test)]
mod tests;
