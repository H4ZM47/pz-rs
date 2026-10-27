//! Fluid userdata owns the actual stable component, independently of whichever
//! component its former item may later contain. Cached Color userdata owns the
//! actual final Color allocation and deliberately does not retain the component.

use std::any::Any;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::{Rc, Weak};

use pz_entity::color::Rgba;
use pz_entity::{ComponentRef, RuntimeError};
use pz_kahlua::{LuaError, LuaObject, LuaResult, NativeCall, Thread, Value};

use crate::bind_misc_classes::Color;
use crate::expose::expose_class;

pub const COMPONENT_CLASS: &str = "zombie.entity.Component";
pub const FLUID_CONTAINER_CLASS: &str = "zombie.entity.components.fluids.FluidContainer";

pub struct LuaFluidContainer {
    pub(crate) component: ComponentRef,
}

impl LuaObject for LuaFluidContainer {
    fn class_name(&self) -> &str {
        FLUID_CONTAINER_CLASS
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn super_classes(&self) -> &[&'static str] {
        &[COMPONENT_CLASS]
    }
}

thread_local! {
    static FLUIDS: RefCell<HashMap<usize, Weak<LuaFluidContainer>>> = RefCell::new(HashMap::new());
    static COLORS: RefCell<HashMap<usize, Weak<Color>>> = RefCell::new(HashMap::new());
}

fn prune<K, T>(cache: &mut HashMap<K, Weak<T>>)
where
    K: std::hash::Hash + Eq,
{
    if cache.len() == cache.capacity() {
        let capacity = cache.capacity();
        cache.retain(|_, value| value.strong_count() != 0);
        if cache.len() > capacity / 2 {
            cache.reserve(capacity);
        }
    }
}

/// Caller obtains this reference from the immutable FluidContainer slot. No
/// body guard is needed to preserve identity, including a cache hit/retry.
pub(crate) fn fluid_value(component: &ComponentRef) -> LuaResult<Value> {
    let key = Rc::as_ptr(component) as usize;
    let cached = FLUIDS.with(|cache| cache.borrow().get(&key).and_then(Weak::upgrade));
    let userdata = match cached {
        Some(userdata) if Rc::ptr_eq(&userdata.component, component) => userdata,
        _ => {
            let userdata = Rc::new(LuaFluidContainer {
                component: Rc::clone(component),
            });
            // No cache borrow spans metadata work or dyn erasure.
            FLUIDS.with(|cache| {
                let mut cache = cache.borrow_mut();
                prune(&mut cache);
                cache.insert(key, Rc::downgrade(&userdata));
            });
            userdata
        }
    };
    crate::component_gc::register_fluid_userdata(&userdata)
        .map_err(|error| LuaError::runtime(error.to_string()))?;
    Ok(Value::Object(userdata))
}

fn color_userdata(color: &Rc<RefCell<Rgba>>) -> Rc<Color> {
    let key = Rc::as_ptr(color) as usize;
    let cached = COLORS.with(|cache| cache.borrow().get(&key).and_then(Weak::upgrade));
    let userdata = match cached {
        Some(userdata) if Rc::ptr_eq(userdata.actual_slot(), color) => userdata,
        _ => {
            let userdata = Rc::new(Color::from_actual(color));
            COLORS.with(|cache| {
                let mut cache = cache.borrow_mut();
                prune(&mut cache);
                cache.insert(key, Rc::downgrade(&userdata));
            });
            userdata
        }
    };
    userdata
}

pub(crate) fn color_value(color: &Rc<RefCell<Rgba>>) -> LuaResult<Value> {
    let userdata = color_userdata(color);
    crate::component_gc::register_color_userdata(&userdata)
        .map_err(|error| LuaError::runtime(error.to_string()))?;
    Ok(Value::Object(userdata))
}

/// Existing native Color owners (named colors, constructors) enter the SAME
/// canonical cache before erasure. The cache owns only weak metadata.
pub(crate) fn existing_color_value(color: &Rc<Color>) -> Value {
    let key = Rc::as_ptr(color.actual_slot()) as usize;
    let cached = COLORS.with(|cache| cache.borrow().get(&key).and_then(Weak::upgrade));
    let userdata = match cached {
        Some(userdata) if Rc::ptr_eq(userdata.actual_slot(), color.actual_slot()) => userdata,
        _ => {
            COLORS.with(|cache| {
                let mut cache = cache.borrow_mut();
                prune(&mut cache);
                cache.insert(key, Rc::downgrade(color));
            });
            Rc::clone(color)
        }
    };
    let _ = crate::component_gc::register_color_userdata(&userdata);
    Value::Object(userdata)
}

pub(crate) fn default_color_value() -> Value {
    let userdata = color_userdata(&pz_entity::runtime_fluid::fluid_default_color_slot());
    // Global class registration is a legacy infallible adapter. It still
    // publishes the real Color; deferred/conflicting metadata is recorded
    // conservatively, never replaced by a copied white value.
    let _ = crate::component_gc::register_color_userdata(&userdata);
    Value::Object(userdata)
}

fn component(c: &NativeCall<'_>) -> LuaResult<ComponentRef> {
    match c.arg(0) {
        Value::Object(object) => object
            .as_any()
            .downcast_ref::<LuaFluidContainer>()
            .map(|fluid| Rc::clone(&fluid.component))
            .ok_or_else(|| LuaError::runtime("Expected a FluidContainer")),
        _ => Err(LuaError::runtime("Expected a FluidContainer")),
    }
}

fn runtime(error: RuntimeError) -> LuaError {
    LuaError::runtime(error.to_string())
}

fn capacity(component: &ComponentRef) -> LuaResult<f32> {
    let body = component
        .try_borrow()
        .map_err(|_| LuaError::runtime("FluidContainer is in use"))?;
    body.fluid()
        .map(|fluid| fluid.capacity)
        .ok_or_else(|| LuaError::runtime("FluidContainer allocation has an invalid class"))
}

/// Recalculation's data phase ends before the real owner dirty/notification
/// phase. Native getAmount reads amountCache AFTER that phase, so a callback's
/// later invalidation must not cause an extra synthetic recalculation here.
pub(crate) fn amount(component: &ComponentRef) -> LuaResult<f32> {
    let env = crate::bind_items::item_env();
    let fallback = pz_entity::BasicContext::default();
    let ctx = env.entity().unwrap_or(&fallback);
    let outcome = component
        .try_borrow_mut()
        .map_err(|_| LuaError::runtime("FluidContainer is in use"))?
        .fluid_recalculate_data(ctx, false)
        .map_err(runtime)?;
    pz_inventory::component_host::notify_fluid_update(&outcome)
        .map_err(|error| LuaError::runtime(format!("FluidContainer owner update: {error:?}")))?;
    let body = component
        .try_borrow()
        .map_err(|_| LuaError::runtime("FluidContainer is in use after owner update"))?;
    body.fluid()
        .map(|fluid| fluid.amount_cache)
        .ok_or_else(|| LuaError::runtime("FluidContainer allocation has an invalid class"))
}

pub(crate) fn cached_color(component: &ComponentRef) -> LuaResult<Value> {
    let env = crate::bind_items::item_env();
    let fallback = pz_entity::BasicContext::default();
    let ctx = env.entity().unwrap_or(&fallback);
    let outcome = component
        .try_borrow_mut()
        .map_err(|_| LuaError::runtime("FluidContainer is in use"))?
        .fluid_recalculate_data(ctx, false)
        .map_err(runtime)?;
    pz_inventory::component_host::notify_fluid_update(&outcome)
        .map_err(|error| LuaError::runtime(format!("FluidContainer owner update: {error:?}")))?;
    let color = {
        let body = component
            .try_borrow()
            .map_err(|_| LuaError::runtime("FluidContainer is in use after owner update"))?;
        body.fluid_color_slot()
            .cloned()
            .ok_or_else(|| LuaError::runtime("FluidContainer allocation has no native Color"))?
    };
    color_value(&color)
}

/// The actual component owner, with no coordinate/id fallback. A world owner
/// needs an actual IsoObjectRef-backed Lua binding before it can be exposed.
fn owner_value(component: &ComponentRef) -> LuaResult<Value> {
    let owner = component
        .try_borrow()
        .map_err(|_| crate::bind_world_object::borrowed("Component.getOwner"))?
        .owner_slot()
        .cloned();
    let Some(owner) = owner else {
        return Ok(Value::Nil);
    };
    if let Some(item) = owner.try_handle::<pz_inventory::InventoryItem>() {
        return crate::bind_items::try_item_value(&item);
    }
    if let Some(object) = owner.try_handle::<pz_world::IsoObject>() {
        return crate::bind_world_object::object_value(&object);
    }
    Err(crate::bind_world_object::unsupported(
        "Component.getOwner: actual owner class has no Lua binding",
    ))
}

pub fn register(thread: &mut Thread) {
    pz_inventory::component_host::install_item_fluid_observer(Some(
        crate::bind_item_container::on_fluid_update,
    ));
    let base = expose_class(thread, COMPONENT_CLASS, None);
    base.method("getOwner", |c| Ok(c.push(owner_value(&component(c)?)?)));
    let fluid = expose_class(thread, FLUID_CONTAINER_CLASS, Some(COMPONENT_CLASS));
    fluid.method("getCapacity", |c| {
        Ok(c.push(capacity(&component(c)?)? as f64))
    });
    fluid.method("getAmount", |c| Ok(c.push(amount(&component(c)?)? as f64)));
    fluid.method("getColor", |c| Ok(c.push(cached_color(&component(c)?)?)));
    fluid.method("isEmpty", |c| Ok(c.push(amount(&component(c)?)? == 0.0)));
    fluid.method("isFull", |c| {
        let component = component(c)?;
        let amount = amount(&component)?;
        Ok(c.push((capacity(&component)? - amount).abs() < 1.0e-4))
    });
    fluid.method("getFreeCapacity", |c| {
        let component = component(c)?;
        let capacity = capacity(&component)?;
        let free = capacity - amount(&component)?;
        Ok(c.push(if free.abs() < 1.0e-4 { 0.0 } else { free } as f64))
    });
    fluid.method("getFilledRatio", |c| {
        let component = component(c)?;
        let first = amount(&component)?;
        if (capacity(&component)? - first).abs() < 1.0e-4 {
            return Ok(c.push(1.0));
        }
        if amount(&component)? == 0.0 {
            return Ok(c.push(0.0));
        }
        if capacity(&component)? == 0.0 {
            return Ok(c.push(0.0));
        }
        let amount = amount(&component)?;
        Ok(c.push((amount / capacity(&component)?) as f64))
    });
}

#[cfg(test)]
mod tests;
