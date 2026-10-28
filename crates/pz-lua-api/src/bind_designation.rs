//! Lua identity over the actual designation registry and zone allocations.
//!
//! Mutable native ArrayLists are not replaced by snapshots or closure views.
//! Their exposure remains an explicit capability until a typed list backend is
//! available. The registry/host context owns only weak session references.
use std::any::Any;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::{Rc, Weak};

use pz_entity::runtime_load::{HostResult, RuntimeHostError};
use pz_entity::RetiredComponents;
use pz_kahlua::{LuaError, LuaObject, LuaResult, NativeCall, Thread, Value};
use pz_world::designation::{
    self, DesignationHost, DesignationRegistryRef, DesignationZone, DesignationZoneKind,
    DesignationZoneRef,
};

pub const BASE: &str = "zombie.iso.areas.DesignationZone";
pub const ANIMAL: &str = "zombie.iso.areas.DesignationZoneAnimal";
pub type DesignationHostRef = Rc<RefCell<Box<dyn DesignationHost>>>;

#[derive(Clone)]
pub struct DesignationContext {
    registry: Weak<RefCell<designation::DesignationRegistry>>,
    host: Weak<RefCell<Box<dyn DesignationHost>>>,
}
impl DesignationContext {
    pub fn new(registry: &DesignationRegistryRef, host: &DesignationHostRef) -> Self {
        Self {
            registry: Rc::downgrade(registry),
            host: Rc::downgrade(host),
        }
    }
}
thread_local! {
    static CONTEXT: RefCell<Option<DesignationContext>> = const { RefCell::new(None) };
    static CLASSES: RefCell<HashMap<usize, ClassStamp>> = RefCell::new(HashMap::new());
    static USERDATA: RefCell<HashMap<usize, Weak<LuaDesignationZone>>> = RefCell::new(HashMap::new());
    static CLASS_FAULTS: RefCell<Vec<DesignationClassFault>> = const { RefCell::new(Vec::new()) };
}
/// Install only weak access to the actual engine-owned class registry/host.
/// Resetting the host context does not perform either Native class Reset.
pub fn install_context(context: Option<DesignationContext>) -> Option<DesignationContext> {
    CONTEXT.with(|slot| slot.replace(context))
}
fn capture_context() -> LuaResult<Option<DesignationContext>> {
    CONTEXT
        .try_with(|context| {
            context
                .try_borrow()
                .map(|value| value.clone())
                .map_err(|_| borrowed("Designation context"))
        })
        .unwrap_or_else(|_| Err(unsupported("Designation context unavailable")))
}
fn registry(context: &Option<DesignationContext>) -> LuaResult<DesignationRegistryRef> {
    context
        .as_ref()
        .and_then(|context| context.registry.upgrade())
        .ok_or_else(|| unsupported("actual Designation registry unavailable"))
}
fn pin<T: 'static>(owner: &Rc<T>) {
    let mut retired = RetiredComponents::new();
    retired.push_erased(owner.clone());
    pz_inventory::component_host::retire_components(retired);
}
fn borrowed(operation: &str) -> LuaError {
    crate::bind_world_object::borrowed(operation)
}
fn unsupported(operation: impl Into<String>) -> LuaError {
    crate::bind_world_object::unsupported(operation)
}
fn host_error(error: RuntimeHostError) -> LuaError {
    crate::bind_world_object::host_error(error)
}

/// The captured weak provider is never reread/retargeted halfway through an
/// action. Each needed native operation borrows it only at that operation.
struct LazyHost(Option<DesignationContext>);
impl LazyHost {
    fn with<R>(
        &mut self,
        f: impl FnOnce(&mut dyn DesignationHost) -> HostResult<R>,
    ) -> HostResult<R> {
        let host = self
            .0
            .as_ref()
            .and_then(|context| context.host.upgrade())
            .ok_or(RuntimeHostError::Unsupported {
                operation: "actual Designation host unavailable",
                kind: None,
            })?;
        pin(&host);
        let mut body = host
            .try_borrow_mut()
            .map_err(|_| RuntimeHostError::Borrowed("Designation host"))?;
        f(body.as_mut())
    }
}
impl DesignationHost for LazyHost {
    fn rand_next(&mut self, bound: i32) -> HostResult<i32> {
        self.with(|host| host.rand_next(bound))
    }
    fn current_cell_present(&mut self) -> HostResult<bool> {
        self.with(|host| host.current_cell_present())
    }
    fn corner_square(
        &mut self,
        corner: designation::DesignationCorner<'_>,
    ) -> HostResult<Option<pz_world::SquareRef>> {
        self.with(|host| host.corner_square(corner))
    }
    fn sync_zone(&mut self, zone: &DesignationZoneRef) -> HostResult<()> {
        self.with(|host| host.sync_zone(zone))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DesignationClassFault {
    pub address: usize,
    pub expected: &'static str,
    pub observed: &'static str,
}
impl std::fmt::Display for DesignationClassFault {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "native designation class at {:x}: {} changed to {}",
            self.address, self.expected, self.observed
        )
    }
}
impl DesignationClassFault {
    fn error(self) -> LuaError {
        LuaError::Runtime {
            class: "pz.host.InvalidState",
            message: self.to_string(),
        }
    }
}
struct ClassStamp {
    zone: Weak<RefCell<DesignationZone>>,
    class: &'static str,
    fault: Option<DesignationClassFault>,
}
fn prune_on_pressure<T>(map: &mut HashMap<usize, T>, alive: impl Fn(&T) -> bool) {
    if map.len() == map.capacity() {
        let capacity = map.capacity();
        map.retain(|_, value| alive(value));
        if map.len() > capacity / 2 {
            map.reserve(capacity.max(1));
        }
    }
}
pub fn take_native_class_faults() -> Vec<DesignationClassFault> {
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
pub(crate) fn remember_kind(zone: &DesignationZoneRef, kind: DesignationZoneKind) -> LuaResult<()> {
    let key = Rc::as_ptr(zone) as usize;
    let class = kind.native_class_name();
    CLASSES
        .try_with(|classes| {
            let mut classes = classes
                .try_borrow_mut()
                .map_err(|_| borrowed("Designation class metadata"))?;
            if let Some(stamp) = classes
                .get_mut(&key)
                .filter(|stamp| stamp.zone.strong_count() != 0)
            {
                if let Some(fault) = stamp.fault {
                    return Err(fault.error());
                }
                if stamp.class != class {
                    let fault = DesignationClassFault {
                        address: key,
                        expected: stamp.class,
                        observed: class,
                    };
                    stamp.fault = Some(fault);
                    let _ = CLASS_FAULTS.try_with(|faults| {
                        if let Ok(mut faults) = faults.try_borrow_mut() {
                            faults.push(fault);
                        }
                    });
                    return Err(fault.error());
                }
                return Ok(());
            }
            classes.remove(&key);
            prune_on_pressure(&mut classes, |stamp| stamp.zone.strong_count() != 0);
            classes.insert(
                key,
                ClassStamp {
                    zone: Rc::downgrade(zone),
                    class,
                    fault: None,
                },
            );
            Ok(())
        })
        .unwrap_or_else(|_| Err(unsupported("Designation class metadata unavailable")))
}
fn class_of(zone: &DesignationZoneRef) -> LuaResult<&'static str> {
    CLASSES
        .try_with(|classes| {
            let classes = classes
                .try_borrow()
                .map_err(|_| borrowed("Designation class metadata"))?;
            let stamp = classes
                .get(&(Rc::as_ptr(zone) as usize))
                .filter(|stamp| stamp.zone.strong_count() != 0)
                .ok_or_else(|| unsupported("Designation allocation has no native class stamp"))?;
            stamp
                .fault
                .map_or(Ok(stamp.class), |fault| Err(fault.error()))
        })
        .unwrap_or_else(|_| Err(unsupported("Designation class metadata unavailable")))
}

pub struct LuaDesignationZone {
    pub(crate) zone: DesignationZoneRef,
    class: &'static str,
}
impl LuaObject for LuaDesignationZone {
    fn class_name(&self) -> &str {
        self.class
    }
    fn super_classes(&self) -> &[&'static str] {
        if self.class == ANIMAL {
            &[BASE]
        } else {
            &[]
        }
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}
/// Canonical identity needs no zone-body borrow after allocation-time stamping.
pub fn zone_value(zone: &DesignationZoneRef) -> LuaResult<Value> {
    let class = class_of(zone)?;
    let object = USERDATA
        .try_with(|cache| {
            let mut cache = cache
                .try_borrow_mut()
                .map_err(|_| borrowed("Designation userdata cache"))?;
            let key = Rc::as_ptr(zone) as usize;
            if let Some(object) = cache.get(&key).and_then(Weak::upgrade) {
                if Rc::ptr_eq(&object.zone, zone) {
                    return Ok(object);
                }
            }
            cache.remove(&key);
            prune_on_pressure(&mut cache, |weak| weak.strong_count() != 0);
            let object = Rc::new(LuaDesignationZone {
                zone: zone.clone(),
                class,
            });
            cache.insert(key, Rc::downgrade(&object));
            Ok(object)
        })
        .unwrap_or_else(|_| Err(unsupported("Designation userdata cache unavailable")))?;
    crate::component_gc::register_designation_userdata(&object)
        .map_err(|error| unsupported(error.to_string()))?;
    class_of(zone)?;
    Ok(Value::Object(object))
}
fn zone_argument(value: &Value) -> LuaResult<DesignationZoneRef> {
    let Value::Object(object) = value else {
        return Err(unsupported("DesignationZone receiver conversion"));
    };
    let object = (&**object as &dyn Any)
        .downcast_ref::<LuaDesignationZone>()
        .ok_or_else(|| unsupported("DesignationZone receiver conversion"))?;
    class_of(&object.zone)?;
    pin(&object.zone);
    Ok(object.zone.clone())
}
fn nargs(call: &NativeCall<'_>, expected: usize) -> LuaResult<()> {
    if call.nargs() == expected {
        Ok(())
    } else {
        Err(unsupported("Designation method argument conversion/arity"))
    }
}
fn integer(call: &NativeCall<'_>, at: usize) -> LuaResult<i32> {
    match call.arg(at) {
        Value::Num(number) => Ok(number as i32),
        _ => Err(unsupported("Designation integer argument conversion")),
    }
}
fn number(call: &NativeCall<'_>, at: usize) -> LuaResult<f64> {
    match call.arg(at) {
        Value::Num(number) => Ok(number),
        _ => Err(unsupported("Designation number argument conversion")),
    }
}
fn boolean(call: &NativeCall<'_>, at: usize) -> LuaResult<bool> {
    match call.arg(at) {
        Value::Bool(value) => Ok(value),
        _ => Err(unsupported("Designation boolean argument conversion")),
    }
}
fn string(call: &NativeCall<'_>, at: usize) -> LuaResult<Option<String>> {
    match call.arg(at) {
        Value::Str(value) => Ok(Some(value.to_string())),
        Value::Nil => Ok(None),
        _ => Err(unsupported("Designation String argument conversion")),
    }
}
fn optional_zone(value: Option<DesignationZoneRef>) -> LuaResult<Value> {
    value.as_ref().map_or(Ok(Value::Nil), zone_value)
}
fn finish<T>(retired: RetiredComponents, result: HostResult<T>) -> LuaResult<T> {
    pz_inventory::component_host::retire_components(retired);
    result.map_err(host_error)
}

fn construct(
    call: &mut NativeCall<'_>,
    kind: DesignationZoneKind,
    add_zone: bool,
) -> LuaResult<usize> {
    if kind == DesignationZoneKind::Base && !add_zone && call.nargs() == 0 {
        return Ok(call.push(zone_value(&designation::new_base_default())?));
    }
    let animal = kind == DesignationZoneKind::Animal;
    nargs(call, if animal || add_zone { 7 } else { 8 })?;
    let zone_type = if animal {
        Some("AnimalZone".to_owned())
    } else {
        string(call, 0)?
    };
    let name = string(call, if animal { 0 } else { 1 })?;
    let at = if animal { 1 } else { 2 };
    let (x, y, z, x2, y2) = (
        integer(call, at)?,
        integer(call, at + 1)?,
        integer(call, at + 2)?,
        integer(call, at + 3)?,
        integer(call, at + 4)?,
    );
    let sync = if add_zone {
        true
    } else {
        boolean(call, at + 5)?
    };
    let context = capture_context()?;
    let registry = registry(&context)?;
    let mut host = LazyHost(context);
    let mut retired = RetiredComponents::new();
    let result = designation::try_new_zone(
        &registry,
        kind,
        zone_type.as_deref(),
        name.as_deref(),
        x,
        y,
        z,
        x2,
        y2,
        sync,
        &mut host,
        &mut retired,
    );
    let zone = finish(retired, result)?;
    Ok(call.push(zone_value(&zone)?))
}

pub fn register(thread: &mut Thread) {
    let base = crate::expose::expose_class(thread, BASE, None);
    let animal = crate::expose::expose_class(thread, ANIMAL, Some(BASE));
    animal.field("ZONE_TYPE", "AnimalZone");
    base.ctor(|call| construct(call, DesignationZoneKind::Base, false));
    animal.ctor(|call| construct(call, DesignationZoneKind::Animal, false));
    base.static_fn("addZone", |call| {
        construct(call, DesignationZoneKind::Base, true)
    });
    for (class, is_animal) in [(&base, false), (&animal, true)] {
        class.static_fn("getZone", move |call| {
            if is_animal && call.nargs() == 2 {
                let (x, y) = (integer(call, 0)?, integer(call, 1)?);
                let registry = registry(&capture_context()?)?;
                let zone = designation::try_get_zone_xy(&registry, x, y).map_err(host_error)?;
                return Ok(call.push(optional_zone(zone)?));
            }
            nargs(call, 3)?;
            let (x, y, z) = (integer(call, 0)?, integer(call, 1)?, integer(call, 2)?);
            let registry = registry(&capture_context()?)?;
            let zone = if is_animal {
                designation::try_get_zone(&registry, x, y, z)
            } else {
                designation::try_get_base_zone(&registry, x, y, z)
            }
            .map_err(host_error)?;
            Ok(call.push(optional_zone(zone)?))
        });
        class.static_fn("getZoneById", move |call| {
            nargs(call, 1)?;
            let id = number(call, 0)?;
            let registry = registry(&capture_context()?)?;
            let zone = if is_animal {
                designation::try_get_zone_by_id(&registry, id)
            } else {
                designation::try_get_base_zone_by_id(&registry, id)
            }
            .map_err(host_error)?;
            Ok(call.push(optional_zone(zone)?))
        });
        class.static_fn("Reset", move |call| {
            nargs(call, 0)?;
            let registry = registry(&capture_context()?)?;
            let mut retired = RetiredComponents::new();
            let result = if is_animal {
                designation::try_reset_animal(&registry, &mut retired)
            } else {
                designation::try_reset_base(&registry, &mut retired)
            };
            finish(retired, result)?;
            Ok(0)
        });
    }
    base.static_fn("getZoneByName", |call| {
        nargs(call, 1)?;
        let name = string(call, 0)?;
        let registry = registry(&capture_context()?)?;
        Ok(call.push(optional_zone(
            designation::try_get_zone_by_name(&registry, name.as_deref()).map_err(host_error)?,
        )?))
    });
    base.static_fn("getZoneByNameAndType", |call| {
        nargs(call, 2)?;
        let kind = string(call, 0)?;
        let name = string(call, 1)?;
        let registry = registry(&capture_context()?)?;
        Ok(call.push(optional_zone(
            designation::try_get_zone_by_name_and_type(&registry, kind.as_deref(), name.as_deref())
                .map_err(host_error)?,
        )?))
    });
    base.static_fn("getZoneByType", |call| {
        nargs(call, 4)?;
        let kind = string(call, 0)?;
        let (x, y, z) = (integer(call, 1)?, integer(call, 2)?, integer(call, 3)?);
        let registry = registry(&capture_context()?)?;
        Ok(call.push(optional_zone(
            designation::try_get_zone_by_type(&registry, kind.as_deref(), x, y, z)
                .map_err(host_error)?,
        )?))
    });
    base.static_fn("removeZone", |call| {
        nargs(call, 2)?;
        let first = call.arg(0);
        // Native returns before reading either class list or calling sync.
        if first.is_nil() && matches!(call.arg(1), Value::Bool(_)) {
            return Ok(0);
        }
        let context = capture_context()?;
        let registry = registry(&context)?;
        let mut retired = RetiredComponents::new();
        let result = if matches!(first, Value::Object(_) | Value::Nil)
            && matches!(call.arg(1), Value::Bool(_))
        {
            let zone = if first.is_nil() {
                None
            } else {
                Some(zone_argument(&first)?)
            };
            let sync = boolean(call, 1)?;
            designation::try_remove_base_zone(
                &registry,
                zone.as_ref(),
                sync,
                &mut LazyHost(context),
                &mut retired,
            )
        } else {
            let kind = string(call, 0)?;
            let name = string(call, 1)?;
            designation::try_remove_base_zone_by_name_and_type(
                &registry,
                kind.as_deref(),
                name.as_deref(),
                &mut retired,
            )
        };
        finish(retired, result)?;
        Ok(0)
    });
    for name in ["getX", "getY", "getZ", "getW", "getH", "getId", "getName"] {
        base.method(name, move |call| {
            nargs(call, 1)?;
            let zone = zone_argument(&call.arg(0))?;
            let value = {
                let body = zone
                    .try_borrow()
                    .map_err(|_| borrowed("Designation direct field"))?;
                match name {
                    "getX" => Value::Num(body.x as f64),
                    "getY" => Value::Num(body.y as f64),
                    "getZ" => Value::Num(body.z as f64),
                    "getW" => Value::Num(body.w as f64),
                    "getH" => Value::Num(body.h as f64),
                    "getId" => Value::Num(body.id),
                    _ => body.name.clone().map_or(Value::Nil, Value::from),
                }
            };
            Ok(call.push(value))
        });
    }
    base.method("setName", |call| {
        nargs(call, 2)?;
        let zone = zone_argument(&call.arg(0))?;
        let name = string(call, 1)?;
        {
            zone.try_borrow_mut()
                .map_err(|_| borrowed("Designation setName field"))?
                .name = name;
        }
        // Native putfield precedes sync, including an unavailable host.
        let mut host = LazyHost(capture_context()?);
        host.sync_zone(&zone).map_err(host_error)?;
        Ok(0)
    });
    animal.static_fn("getType", |call| {
        nargs(call, 0)?;
        // Native static returns the class constant without any receiver.
        Ok(call.push("AnimalZone"))
    });
    for name in ["getNbOfDung", "getNbOfFeather"] {
        animal.method(name, move |call| {
            nargs(call, 1)?;
            let zone = zone_argument(&call.arg(0))?;
            let value = {
                let body = zone
                    .try_borrow()
                    .map_err(|_| borrowed("Designation animal counter"))?;
                if name == "getNbOfDung" {
                    body.nb_of_dung
                } else {
                    body.nb_of_feather
                }
            };
            Ok(call.push(value as f64))
        });
    }
    base.method("isFullyStreamed", |call| {
        nargs(call, 1)?;
        let zone = zone_argument(&call.arg(0))?;
        let mut host = LazyHost(capture_context()?);
        Ok(call.push(designation::try_is_fully_streamed(&zone, &mut host).map_err(host_error)?))
    });
    for class in [&base, &animal] {
        class.method("check", |call| {
            nargs(call, 1)?;
            let zone = zone_argument(&call.arg(0))?;
            if class_of(&zone)? == BASE {
                return Ok(0);
            } // Exact empty base body.
            let context = capture_context()?;
            let registry = registry(&context)?;
            let mut host = LazyHost(context);
            let mut retired = RetiredComponents::new();
            let result = designation::try_check(&registry, &zone, &mut host, &mut retired);
            finish(retired, result)?;
            Ok(0)
        });
    }
    // Returning a snapshot here would replace the Native mutable allocation.
    for (class, names) in [
        (&base, &["getAllZonesByType"][..]),
        (&animal, &["getAllZones", "getAllDZones"][..]),
    ] {
        for name in names {
            class.static_fn(name, |_| {
                Err(unsupported(
                    "actual mutable Designation ArrayList Lua backend",
                ))
            });
        }
    }
    for name in [
        "getAnimals",
        "getCorpses",
        "getTroughs",
        "getHutchs",
        "getFoodOnGround",
        "getRoofAreas",
    ] {
        animal.method(name, move |call| {
            nargs(call, 1)?;
            let zone = zone_argument(&call.arg(0))?;
            let present = {
                let body = zone
                    .try_borrow()
                    .map_err(|_| borrowed("Designation list field"))?;
                match name {
                    "getAnimals" => body.animals.as_ref(),
                    "getCorpses" => body.corpses.as_ref(),
                    "getTroughs" => body.troughs.as_ref(),
                    "getHutchs" => body.hutchs.as_ref(),
                    "getFoodOnGround" => body.food_on_ground.as_ref(),
                    _ => body.roof_areas.as_ref(),
                }
                .is_some()
            };
            if !present {
                return Ok(call.push(Value::Nil));
            }
            Err(unsupported(
                "actual mutable Designation ArrayList Lua backend",
            ))
        });
    }
    animal.static_fn("removeZone", |_| {
        Err(unsupported(
            "DesignationZoneAnimal connected-zone traversal",
        ))
    });
    for name in [
        "getZoneF",
        "isItemFood",
        "isItemDung",
        "isItemFeather",
        "addItemOnGround",
        "removeItemFromGround",
        "addNewRoof",
    ] {
        animal.static_fn(name, |_| {
            Err(unsupported("Designation item/coordinate helper capability"))
        });
    }
    for name in [
        "getCorpsesConnected",
        "getAnimalsConnected",
        "getTroughsConnected",
        "getHutchsConnected",
        "getFoodOnGroundConnected",
        "getNearWaterSquaresConnected",
        "getRoofAreasConnected",
        "getFullZoneSize",
    ] {
        animal.method(name, |_| {
            Err(unsupported("Designation connected-zone traversal"))
        });
    }
    for name in ["addAnimal", "removeAnimal", "addCorpse", "removeCorpse"] {
        animal.method(name, |_| {
            Err(unsupported("Designation actual actor/position producer"))
        });
    }
    base.static_fn("update", |_| {
        Err(unsupported("Designation current clock/update host"))
    });
    // Java exposes inherited public static methods on the subclass too.
    for name in [
        "addZone",
        "getZoneByName",
        "getZoneByNameAndType",
        "getZoneByType",
        "getAllZonesByType",
    ] {
        animal.statics.set_str(name, base.statics.get_str(name));
    }
    for (name, bits) in [
        ("ZONE_COLOR_R", 0x3e4ccccdu32),
        ("ZONE_COLOR_G", 0x3e4ccccd),
        ("ZONE_COLOR_B", 0x3f666666),
        ("ZONE_SELECTED_COLOR_R", 0x3e4ccccd),
        ("ZONE_SELECTED_COLOR_G", 0x3f4ccccd),
        ("ZONE_SELECTED_COLOR_B", 0x3f666666),
    ] {
        // Field.get boxes the actual primitive float; exposeStatics rawsets
        // that Float without the Java-method return-to-Double conversion.
        animal.field(
            name,
            pz_kahlua::table::BoxedFloat::value(f32::from_bits(bits)),
        );
    }
    for (name, value) in [
        ("FENCE_WEST", "fencing_01_20"),
        ("FENCE_NORTH", "fencing_01_21"),
        ("FENCE_NORTHCORNER", "fencing_01_18"),
    ] {
        animal.field(name, value);
    }
    for class in [&base, &animal] {
        let meta = class
            .statics
            .metatable()
            .unwrap_or_else(pz_kahlua::TableRef::new);
        meta.set_str(
            "__index",
            Value::Native(pz_kahlua::NativeFunction::new(
                "designation static field",
                |call| {
                    if matches!(
                        call.arg(1).as_str(),
                        Some("allZones" | "designationAnimalZoneList" | "lastUpdate")
                    ) {
                        let operation = if call.arg(1).as_str() == Some("lastUpdate") {
                            "Designation raw static Long field backend"
                        } else {
                            "actual mutable Designation ArrayList Lua backend"
                        };
                        return Err(unsupported(operation));
                    }
                    Ok(call.push(Value::Nil))
                },
            )),
        );
        class.statics.set_metatable(Some(meta));
    }
}

#[cfg(test)]
mod tests;
