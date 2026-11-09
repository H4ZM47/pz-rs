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
use pz_inventory::shared::ContainerRef;
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
    let object = crate::expose::expose_class(thread, ISO_OBJECT, Some(GAME_ENTITY));
    // getContainerCount(): 1 for the main container if there is one, plus
    // the number of secondary containers.
    object.method("getContainerCount", |c| {
        let object = crate::expose::this::<LuaWorldObject>(c)?;
        let object = object.as_any().downcast_ref::<LuaWorldObject>().unwrap();
        let count = object
            .object
            .try_borrow()
            .map_err(|_| borrowed("IsoObject body"))?
            .container_count();
        Ok(c.push(Value::from(count as f64)))
    });
    // getSquare(): the `square` field, null off the map.
    object.method("getSquare", |c| {
        let object = receiver_object(c)?;
        let xyz = object
            .try_borrow()
            .map_err(|_| borrowed("IsoObject body"))?
            .square_slot()
            .map(|sq| {
                let sq = sq.borrow();
                (sq.x, sq.y, sq.z)
            });
        Ok(c.push(xyz.map_or(Value::Nil, |(x, y, z)| crate::bind_world::square_value(x, y, z))))
    });
    register_door(thread);
    // getModData(): table, made on first use.
    object.method("getModData", |c| {
        let object = receiver_object(c)?;
        let table = object
            .try_borrow_mut()
            .map_err(|_| borrowed("IsoObject body"))?
            .install_runtime_mod_data()
            .map_err(|e| LuaError::runtime(e.0))?;
        Ok(c.push(Value::Table(table)))
    });
    // hasModData(): true when the object has a non-empty mod data table.
    object.method("hasModData", |c| {
        let object = receiver_object(c)?;
        let has = object
            .try_borrow()
            .map_err(|_| borrowed("IsoObject body"))?
            .has_mod_data()
            .map_err(|e| LuaError::runtime(e.0))?;
        Ok(c.push(Value::Bool(has)))
    });
    object.method("getContainer", |c| {
        let (main, _) = containers(c)?;
        Ok(c.push(opt_container(main.as_ref())))
    });
    object.method("getItemContainer", |c| {
        let (main, _) = containers(c)?;
        Ok(c.push(opt_container(main.as_ref())))
    });
    // getContainerByIndex(i): container at 0, then secondaryContainers;
    // null out of range.
    object.method("getContainerByIndex", |c| {
        let index = crate::expose::int_arg(c, 1);
        let (main, secondary) = containers(c)?;
        let all: Vec<_> = main.iter().chain(secondary.iter()).collect();
        let found = usize::try_from(index).ok().and_then(|i| all.get(i).copied());
        Ok(c.push(opt_container(found)))
    });
    // getContainerByType(type): the first getContainerByIndex(i) whose
    // getType().equals(type).
    object.method("getContainerByType", |c| {
        let kind = crate::expose::str_arg(c, 1);
        let found = by_type(c, &[kind])?;
        Ok(c.push(opt_container(found.as_ref())))
    });
    object.method("getContainerByEitherType", |c| {
        let kinds = [crate::expose::str_arg(c, 1), crate::expose::str_arg(c, 2)];
        let found = by_type(c, &kinds)?;
        Ok(c.push(opt_container(found.as_ref())))
    });
    // getContainerIndex(c): 0 for the main container (also when both are
    // absent). For a secondary container, its index, plus 1 when a main
    // container exists. -1 when the object does not have it.
    object.method("getContainerIndex", |c| {
        let arg = c.arg(1);
        let wanted = if arg.is_nil() {
            None
        } else {
            Some(crate::bind_item_container::container_arg(c, 1)?)
        };
        let (main, secondary) = containers(c)?;
        let same = |a: Option<&ContainerRef>, b: Option<&ContainerRef>| match (a, b) {
            (None, None) => true,
            (Some(a), Some(b)) => Rc::ptr_eq(a, b),
            _ => false,
        };
        let index = if same(wanted.as_ref(), main.as_ref()) {
            0
        } else {
            secondary
                .iter()
                .position(|s| same(wanted.as_ref(), Some(s)))
                .map_or(-1, |i| i as i32 + main.is_some() as i32)
        };
        Ok(c.push(Value::from(f64::from(index))))
    });
    crate::expose::expose_class(thread, WORLD_INVENTORY_OBJECT, Some(ISO_OBJECT));
}

/// The receiver's live containers (`container`, `secondaryContainers`).
/// On first use they are made from the loaded values. The object is their
/// parent and its square is their source grid, as the `ItemContainer`
/// constructor for a map object sets them.
fn containers(
    c: &mut pz_kahlua::NativeCall<'_>,
) -> LuaResult<(Option<ContainerRef>, Vec<ContainerRef>)> {
    let this = c.arg(0);
    let object = receiver_object(c)?;
    let square = {
        let body = object.try_borrow().map_err(|_| borrowed("IsoObject body"))?;
        match body.square_slot() {
            Some(square) => {
                let square = square.try_borrow().map_err(|_| borrowed("IsoGridSquare"))?;
                crate::bind_world::square_value(square.x, square.y, square.z)
            }
            None => Value::Nil,
        }
    };
    let mut body = object.try_borrow_mut().map_err(|_| borrowed("IsoObject body"))?;
    let installed = body.install_runtime_containers(|container| {
        crate::bind_item_container::set_parent(container, this.clone());
        container.borrow_mut().source_grid = (!square.is_nil())
            .then(|| Rc::new(crate::bind_item_container::LuaRef(square.clone())) as Rc<dyn Any>);
    });
    Ok((installed.main.clone(), installed.secondary.clone()))
}

const ISO_DOOR: &str = "zombie.iso.objects.IsoDoor";

/// `IsoDoor`'s state getters: `IsOpen()` (`isOpen()`, the `open` field),
/// `getNorth()`, `isLocked()`, `isLockedByKey()`.
fn register_door(thread: &mut Thread) {
    let door = crate::expose::expose_class(thread, ISO_DOOR, Some(ISO_OBJECT));
    fn field(c: &pz_kahlua::NativeCall<'_>, f: fn(&pz_world::objects::IsoDoor) -> bool) -> LuaResult<bool> {
        let object = receiver_object(c)?;
        let body = object.try_borrow().map_err(|_| borrowed("IsoDoor body"))?;
        match &body.kind {
            pz_world::objects::ObjectKind::Door(d) => Ok(f(d)),
            _ => Err(unsupported("IsoDoor method on another object kind")),
        }
    }
    door.method("IsOpen", |c| Ok(c.push(field(c, |d| d.open)?)));
    door.method("isOpen", |c| Ok(c.push(field(c, |d| d.open)?)));
    door.method("getNorth", |c| Ok(c.push(field(c, |d| d.north)?)));
    door.method("isLocked", |c| Ok(c.push(field(c, |d| d.locked)?)));
    door.method("isLockedByKey", |c| Ok(c.push(field(c, |d| d.locked_by_key)?)));
}

fn receiver_object(c: &pz_kahlua::NativeCall<'_>) -> LuaResult<IsoObjectRef> {
    let receiver = crate::expose::this::<LuaWorldObject>(c)?;
    Ok(receiver
        .as_any()
        .downcast_ref::<LuaWorldObject>()
        .unwrap()
        .object
        .clone())
}

fn opt_container(container: Option<&ContainerRef>) -> Value {
    container.map_or(Value::Nil, crate::bind_item_container::container_value)
}

/// The first container whose type equals one of `kinds` (`String.equals`:
/// a null argument matches nothing).
fn by_type(
    c: &mut pz_kahlua::NativeCall<'_>,
    kinds: &[Option<Rc<str>>],
) -> LuaResult<Option<ContainerRef>> {
    let (main, secondary) = containers(c)?;
    Ok(main.into_iter().chain(secondary).find(|container| {
        let container = container.borrow();
        kinds.iter().flatten().any(|kind| **kind == *container.kind)
    }))
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
