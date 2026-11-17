//! `zombie.iso.objects.IsoDeadBody` exposed to Lua.
//!
//! A userdata owns one real corpse ([`DeadBodyRef`], the object in its
//! square's `staticMovingObjects`); while it lives, the same corpse gives
//! the same Lua object, so Lua `==` is Java identity.
//!
//! Bound: what vanilla Lua calls on a corpse in the inventory pane
//! (`refreshBackpacks`), the grab-corpse menu and the transfer action.
//! TODO(port): the rest of `IsoDeadBody` (`getModData`, highlights,
//! `isFakeDead`, reanimation, the hand items and the grappling calls).

use std::any::Any;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::{Rc, Weak};

use pz_kahlua::{LuaError, LuaObject, LuaResult, NativeCall, Thread, Value};
use pz_world::save::visual::DeadBodyRef;

use crate::expose::{expose_class, int_arg, num_arg, ClassBuilder};

pub const ISO_DEAD_BODY: &str = "zombie.iso.objects.IsoDeadBody";
const ISO_MOVING_OBJECT: &str = "zombie.iso.IsoMovingObject";

pub struct LuaDeadBody {
    pub body: DeadBodyRef,
}

impl LuaObject for LuaDeadBody {
    fn class_name(&self) -> &str {
        ISO_DEAD_BODY
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn super_classes(&self) -> &[&'static str] {
        &[ISO_MOVING_OBJECT, "zombie.iso.IsoObject", "zombie.entity.GameEntity"]
    }
}

thread_local! {
    static USERDATA: RefCell<HashMap<usize, Weak<LuaDeadBody>>> = RefCell::new(HashMap::new());
}

/// The Lua object of `body` (one per corpse while it lives).
pub fn dead_body_value(body: &DeadBodyRef) -> Value {
    let key = Rc::as_ptr(body) as usize;
    let value = USERDATA.with(|cache| {
        let mut cache = cache.borrow_mut();
        if let Some(v) = cache.get(&key).and_then(Weak::upgrade) {
            if Rc::ptr_eq(&v.body, body) {
                return v;
            }
        }
        if cache.len() == cache.capacity() {
            cache.retain(|_, v| v.strong_count() != 0);
        }
        let v = Rc::new(LuaDeadBody { body: body.clone() });
        cache.insert(key, Rc::downgrade(&v));
        v
    });
    Value::Object(value)
}

/// The corpse behind a Lua value, if it is an `IsoDeadBody`.
pub fn dead_body_of(v: &Value) -> Option<DeadBodyRef> {
    match v {
        Value::Object(o) => o.as_any().downcast_ref::<LuaDeadBody>().map(|b| b.body.clone()),
        _ => None,
    }
}

fn this(c: &NativeCall<'_>) -> LuaResult<DeadBodyRef> {
    dead_body_of(&c.arg(0)).ok_or_else(|| {
        LuaError::runtime(format!(
            "Expected an IsoDeadBody but got {}",
            pz_kahlua::stdlib::java_class_name(&c.arg(0))
        ))
    })
}

fn borrowed() -> LuaError {
    crate::bind_world_object::borrowed("IsoDeadBody")
}

fn get(b: &ClassBuilder, name: &str, f: impl Fn(&mut NativeCall<'_>) -> LuaResult<Value> + 'static) {
    b.method(name, move |c| {
        let v = f(c)?;
        Ok(c.push(v))
    });
}

/// `x`, `y` and `z`.
fn position(body: &DeadBodyRef) -> LuaResult<(f32, f32, f32)> {
    let b = body.try_borrow().map_err(|_| borrowed())?;
    Ok((b.base.x, b.base.y, b.base.z))
}

/// The square the corpse lies on: `current`/`square`, the grid square at
/// its position when it was made or loaded.
fn square_of(x: f32, y: f32, z: f32) -> (i32, i32, i32) {
    (x.floor() as i32, y.floor() as i32, z.floor() as i32)
}

/// `other.getX()`, `other.getY()` of an `IsoMovingObject` argument.
fn other_xy(t: &mut Thread, obj: &Value) -> LuaResult<Option<(f32, f32)>> {
    if obj.is_nil() {
        return Ok(None);
    }
    if let Some(body) = dead_body_of(obj) {
        let (x, y, _) = position(&body)?;
        return Ok(Some((x, y)));
    }
    let mut call = |name: &str| -> LuaResult<f32> {
        let f = t.table_get(obj, &Value::str(name))?;
        if f.is_nil() {
            return Err(LuaError::runtime(format!(
                "{}.{name} is not bound",
                pz_kahlua::stdlib::java_class_name(obj)
            )));
        }
        Ok(t.call1(&f, std::slice::from_ref(obj))?.as_num().unwrap_or(0.0) as f32)
    };
    let x = call("getX")?;
    let y = call("getY")?;
    Ok(Some((x, y)))
}

/// `getStaticMovingObjectIndex()`: the corpse's index in its square's
/// `staticMovingObjects`, `-1` when it is on no square.
fn static_moving_index(body: &DeadBodyRef) -> LuaResult<i32> {
    let (x, y, z) = position(body)?;
    let (x, y, z) = square_of(x, y, z);
    let index = crate::bind_world::with_cell_square(x, y, z, |sq| {
        sq.static_moving.iter().position(|b| Rc::ptr_eq(b, body))
    });
    Ok(index.flatten().map_or(-1, |i| i as i32))
}

/// `getContainer()`: the live container. As `IsoDeadBody.setContainer`
/// makes it, its parent is the corpse and its `sourceGrid` the square.
fn container(c: &NativeCall<'_>, body: &DeadBodyRef) -> LuaResult<Value> {
    let container = body.try_borrow_mut().map_err(|_| borrowed())?.runtime_container();
    let Some(container) = container else {
        return Ok(Value::Nil);
    };
    // `setContainer`: `container.parent = this`.
    let parent = crate::bind_item_container::parent_of(&container.borrow());
    if dead_body_of(&parent).is_none_or(|p| !Rc::ptr_eq(&p, body)) {
        crate::bind_item_container::set_parent(&container, c.arg(0));
        let (x, y, z) = position(body)?;
        let (x, y, z) = square_of(x, y, z);
        let grid = crate::bind_world::square_value(x, y, z);
        container.borrow_mut().source_grid =
            (!grid.is_nil()).then(|| Rc::new(crate::bind_item_container::LuaRef(grid)) as Rc<dyn Any>);
    }
    Ok(crate::bind_item_container::container_value(&container))
}

pub(crate) fn register(thread: &mut Thread) {
    // `IsoDeadBody extends IsoMovingObject extends IsoObject`: the corpse
    // finds the parents' methods through the class tables.
    crate::bind_world_object::register_classes(thread);
    expose_class(thread, ISO_MOVING_OBJECT, Some(crate::bind_world_object::ISO_OBJECT));
    let b = expose_class(thread, ISO_DEAD_BODY, Some(ISO_MOVING_OBJECT));
    // The `IsoObject` methods the loot window calls on a container's
    // parent. A corpse is not an `IsoFireplace` or `IsoBarbecue`, so
    // `isFireInteractionObject()`, `isPropaneBBQ()` and `hasPropaneTank()`
    // are false; `IsoObject.isLit()` is false.
    for name in ["isFireInteractionObject", "isPropaneBBQ", "hasPropaneTank", "isLit"] {
        get(&b, name, |c| {
            this(c)?;
            Ok(Value::Bool(false))
        });
    }
    get(&b, "isFemale", |c| Ok(Value::Bool(this(c)?.try_borrow().map_err(|_| borrowed())?.female)));
    get(&b, "isZombie", |c| Ok(Value::Bool(this(c)?.try_borrow().map_err(|_| borrowed())?.was_zombie)));
    // `!StringUtils.isNullOrEmpty(animalType)`.
    get(&b, "isAnimal", |c| {
        let body = this(c)?;
        let b = body.try_borrow().map_err(|_| borrowed())?;
        Ok(Value::Bool(b.animal.as_ref().is_some_and(|a| !a.animal_type.is_empty())))
    });
    get(&b, "isSkeleton", |c| Ok(Value::Bool(this(c)?.try_borrow().map_err(|_| borrowed())?.is_skeleton())));
    get(&b, "getX", |c| Ok(Value::Num(f64::from(position(&this(c)?)?.0))));
    get(&b, "getY", |c| Ok(Value::Num(f64::from(position(&this(c)?)?.1))));
    get(&b, "getZ", |c| Ok(Value::Num(f64::from(position(&this(c)?)?.2))));
    get(&b, "getSquare", |c| {
        let (x, y, z) = position(&this(c)?)?;
        let (x, y, z) = square_of(x, y, z);
        Ok(crate::bind_world::square_value(x, y, z))
    });
    // `DistTo(int, int)` / `DistTo(IsoMovingObject)`:
    // `IsoUtils.DistanceManhatten`, 0 for a null object.
    get(&b, "DistTo", |c| {
        let (x, y, _) = position(&this(c)?)?;
        let other = match c.arg(1) {
            Value::Num(_) => Some((int_arg(c, 1) as f32, int_arg(c, 2) as f32)),
            obj => other_xy(c.thread, &obj)?,
        };
        let d = other.map_or(0.0, |(ox, oy)| (ox - x).abs() + (oy - y).abs());
        Ok(Value::Num(f64::from(d)))
    });
    // `DistToSquared(float, float)` / `DistToSquared(IsoMovingObject)`:
    // `IsoUtils.DistanceToSquared`.
    // TODO(port): the vehicle polygon cases (`BaseVehicle.getClosestPointOnPoly`).
    get(&b, "DistToSquared", |c| {
        let (x, y, _) = position(&this(c)?)?;
        let (ox, oy) = match c.arg(1) {
            Value::Num(_) => (num_arg(c, 1) as f32, num_arg(c, 2) as f32),
            obj => other_xy(c.thread, &obj)?.ok_or_else(|| LuaError::Runtime {
                class: "java.lang.NullPointerException",
                message: String::new(),
            })?,
        };
        Ok(Value::Num(f64::from((ox - x) * (ox - x) + (oy - y) * (oy - y))))
    });
    get(&b, "getStaticMovingObjectIndex", |c| Ok(Value::Num(f64::from(static_moving_index(&this(c)?)?))));
    get(&b, "getContainer", |c| {
        let body = this(c)?;
        container(c, &body)
    });
    // `getInventoryWeight()`: 0 without a container; else per item the
    // hotbar weight of an attached item not equipped, the equipped weight
    // of an equipped one, or the unequipped weight.
    get(&b, "getInventoryWeight", |c| {
        let body = this(c)?;
        let container = body.try_borrow_mut().map_err(|_| borrowed())?.runtime_container();
        let Some(container) = container else {
            return Ok(Value::Num(0.0));
        };
        let items = container.borrow().items.clone();
        let e = crate::bind_items::item_env();
        let ctx = e.context();
        let mut total = 0.0f32;
        for item in &items {
            let equipped = body.try_borrow().map_err(|_| borrowed())?.is_equipped(item);
            let owner = e.owner(item);
            let i = item.borrow();
            if i.attached_slot > -1 && !equipped {
                total += i.get_hotbar_equipped_weight(ctx, owner.as_ref());
            } else if equipped {
                total += i.get_equipped_weight(ctx, owner.as_ref());
            } else {
                total += i.get_unequipped_weight(ctx, owner.as_ref());
            }
        }
        Ok(Value::Num(f64::from(total)))
    });
    // `checkClothing(item)`: worn and attached items no longer in the
    // container leave `wornItems`/`attachedItems` (the body texture follows
    // from the new item visuals).
    // TODO(port): the hand items (`setPrimaryHandItem(null)` when `item` is
    // one); the port's corpses have none.
    b.method("checkClothing", |c| {
        let body = this(c)?;
        body.try_borrow_mut().map_err(|_| borrowed())?.check_clothing();
        Ok(0)
    });
}
