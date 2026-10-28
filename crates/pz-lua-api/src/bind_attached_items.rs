//! `zombie.characters.AttachedItems` exposed to Lua: `AttachedLocations`,
//! `AttachedLocationGroup`, `AttachedLocation`, `AttachedItems` and
//! `AttachedItem`, over the shared [`Characters`]' `attached_locations`.
//!
//! Groups and locations are canonical (one Lua object each), so Lua `==`
//! behaves like the original's identity. Items are any Lua value
//! ([`LuaItem`], compared by identity) since `InventoryItem` is not ported.
//! Not exposed: `AttachedItems.forEach` (takes a Java `Consumer`).

use crate::bind_characters::LuaItem;
use crate::bind_registry::java_error;
use crate::expose::{expose_class, int_arg, str_arg, with_this};
use pz_characters::{
    AttachedItem, AttachedItems, AttachedLocation, AttachedLocationGroup, AttachedLocationGroupRef,
    AttachedLocationRef, Characters,
};
use pz_kahlua::value::LuaObject;
use pz_kahlua::{LuaError, LuaResult, NativeCall, Thread, Value};
use std::any::Any;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

pub const ATTACHED_LOCATIONS_CLASS: &str = "zombie.characters.AttachedItems.AttachedLocations";
pub const ATTACHED_LOCATION_GROUP_CLASS: &str = "zombie.characters.AttachedItems.AttachedLocationGroup";
pub const ATTACHED_LOCATION_CLASS: &str = "zombie.characters.AttachedItems.AttachedLocation";
pub const ATTACHED_ITEMS_CLASS: &str = "zombie.characters.AttachedItems.AttachedItems";
pub const ATTACHED_ITEM_CLASS: &str = "zombie.characters.AttachedItems.AttachedItem";

thread_local! {
    static HANDLES: RefCell<HashMap<usize, Value>> = RefCell::new(HashMap::new());
}

/// One Lua object per shared engine object (`Rc` identity; the cached
/// object keeps the `Rc` alive, so addresses are not reused).
fn canonical<T>(rc: &Rc<T>, make: impl FnOnce() -> Value) -> Value {
    let id = Rc::as_ptr(rc) as usize;
    HANDLES.with(|h| h.borrow_mut().entry(id).or_insert_with(make).clone())
}

/// An `AttachedLocationGroup`.
pub struct AttachedLocationGroupObject(pub AttachedLocationGroupRef);

impl LuaObject for AttachedLocationGroupObject {
    fn class_name(&self) -> &str {
        ATTACHED_LOCATION_GROUP_CLASS
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

pub fn group_value(g: &AttachedLocationGroupRef) -> Value {
    canonical(g, || Value::Object(Rc::new(AttachedLocationGroupObject(g.clone()))))
}

/// An `AttachedLocation`.
pub struct AttachedLocationObject(pub AttachedLocationRef);

impl LuaObject for AttachedLocationObject {
    fn class_name(&self) -> &str {
        ATTACHED_LOCATION_CLASS
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

fn location_value(l: Option<&AttachedLocationRef>) -> Value {
    match l {
        Some(l) => canonical(l, || Value::Object(Rc::new(AttachedLocationObject(l.clone())))),
        None => Value::Nil,
    }
}

/// An `AttachedItems`.
pub struct AttachedItemsObject(pub RefCell<AttachedItems<LuaItem>>);

impl LuaObject for AttachedItemsObject {
    fn class_name(&self) -> &str {
        ATTACHED_ITEMS_CLASS
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// An `AttachedItem` (immutable in the original).
pub struct AttachedItemObject(pub AttachedItem<LuaItem>);

impl LuaObject for AttachedItemObject {
    fn class_name(&self) -> &str {
        ATTACHED_ITEM_CLASS
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

fn npe() -> LuaError {
    LuaError::Runtime { message: String::new(), class: "java.lang.NullPointerException" }
}

fn opt_string(c: &NativeCall<'_>, i: usize) -> Option<String> {
    str_arg(c, i).map(|s| s.to_string())
}

/// A non-null item argument (`null` is `None`).
fn item_arg(c: &NativeCall<'_>, i: usize) -> Option<LuaItem> {
    let v = c.arg(i);
    (!v.is_nil()).then_some(LuaItem(v))
}

fn group_arg(c: &NativeCall<'_>, i: usize) -> LuaResult<AttachedLocationGroupRef> {
    match c.arg(i) {
        Value::Object(o) => match o.as_any().downcast_ref::<AttachedLocationGroupObject>() {
            Some(g) => Ok(g.0.clone()),
            None => Err(LuaError::runtime(format!("Expected AttachedLocationGroup but got {}", o.class_name()))),
        },
        Value::Nil => Err(npe()),
        _ => Err(LuaError::runtime("Expected AttachedLocationGroup")),
    }
}

fn group(c: &NativeCall<'_>) -> LuaResult<AttachedLocationGroupRef> {
    with_this::<AttachedLocationGroupObject, _>(c, |o| o.0.clone())
}

fn location(c: &NativeCall<'_>) -> LuaResult<AttachedLocationRef> {
    with_this::<AttachedLocationObject, _>(c, |o| o.0.clone())
}

fn attached<R>(c: &NativeCall<'_>, f: impl FnOnce(&mut AttachedItems<LuaItem>) -> R) -> LuaResult<R> {
    with_this::<AttachedItemsObject, _>(c, |o| f(&mut o.0.borrow_mut()))
}

/// Register the `AttachedItems` package classes, bound to `chars`.
pub fn register(thread: &mut Thread, chars: Rc<RefCell<Characters>>) {
    let b = expose_class(thread, ATTACHED_LOCATIONS_CLASS, None);
    let ch = chars.clone();
    b.static_fn("getGroup", move |c| {
        let id = opt_string(c, 0);
        let g = ch.borrow_mut().attached_locations.get_group(id.as_deref()).map_err(java_error)?;
        Ok(c.push(group_value(&g)))
    });
    let ch = chars;
    b.static_fn("Reset", move |_| {
        ch.borrow_mut().attached_locations.reset();
        Ok(0)
    });

    let b = expose_class(thread, ATTACHED_LOCATION_GROUP_CLASS, None);
    // `new AttachedLocationGroup(id)`: not added to `AttachedLocations`.
    b.ctor(|c| {
        let g = AttachedLocationGroup::new(opt_string(c, 0).as_deref()).map_err(java_error)?;
        Ok(c.push(group_value(&Rc::new(RefCell::new(g)))))
    });
    b.method("getLocation", |c| {
        let id = opt_string(c, 1);
        let l = group(c)?.borrow().location(id.as_deref());
        Ok(c.push(location_value(l.as_ref())))
    });
    b.method("getOrCreateLocation", |c| {
        let id = opt_string(c, 1);
        let l = group(c)?.borrow_mut().get_or_create_location(id.as_deref()).map_err(java_error)?;
        Ok(c.push(location_value(Some(&l))))
    });
    b.method("getLocationByIndex", |c| {
        let l = group(c)?.borrow().location_by_index(int_arg(c, 1));
        Ok(c.push(location_value(l.as_ref())))
    });
    b.method("size", |c| {
        let v = group(c)?.borrow().size() as i32;
        Ok(c.push(v))
    });
    b.method("indexOf", |c| {
        let id = opt_string(c, 1);
        let v = group(c)?.borrow().index_of(id.as_deref());
        Ok(c.push(v))
    });
    b.method("checkValid", |c| {
        let id = opt_string(c, 1);
        group(c)?.borrow().check_valid(id.as_deref()).map_err(java_error)?;
        Ok(0)
    });

    let b = expose_class(thread, ATTACHED_LOCATION_CLASS, None);
    // `new AttachedLocation(group, id)`: not added to the group.
    b.ctor(|c| {
        let l = AttachedLocation::new(opt_string(c, 1).as_deref()).map_err(java_error)?;
        Ok(c.push(location_value(Some(&Rc::new(RefCell::new(l))))))
    });
    b.method("getId", |c| {
        let v = location(c)?.borrow().id().to_string();
        Ok(c.push(v))
    });
    b.method("getAttachmentName", |c| {
        let v = location(c)?.borrow().attachment_name().map(Value::from).unwrap_or_default();
        Ok(c.push(v))
    });
    b.method("setAttachmentName", |c| {
        let name = opt_string(c, 1);
        location(c)?.borrow_mut().set_attachment_name(name.as_deref());
        Ok(0)
    });

    let b = expose_class(thread, ATTACHED_ITEMS_CLASS, None);
    // `new AttachedItems(group)` / `new AttachedItems(other)` (a copy).
    b.ctor(|c| {
        let items = match c.arg(0) {
            Value::Object(o) if o.as_any().is::<AttachedItemsObject>() => {
                o.as_any().downcast_ref::<AttachedItemsObject>().unwrap().0.borrow().clone()
            }
            _ => AttachedItems::new(group_arg(c, 0)?),
        };
        Ok(c.push(Value::Object(Rc::new(AttachedItemsObject(RefCell::new(items))))))
    });
    b.method("copyFrom", |c| {
        let other = match c.arg(1) {
            Value::Object(o) => match o.as_any().downcast_ref::<AttachedItemsObject>() {
                Some(a) => a.0.borrow().clone(),
                None => return Err(LuaError::runtime(format!("Expected AttachedItems but got {}", o.class_name()))),
            },
            _ => return Err(npe()),
        };
        attached(c, |a| a.copy_from(&other))?.map_err(java_error)?;
        Ok(0)
    });
    b.method("getGroup", |c| {
        let g = attached(c, |a| a.group().clone())?;
        Ok(c.push(group_value(&g)))
    });
    b.method("get", |c| {
        let i = int_arg(c, 1);
        let item = attached(c, |a| a.get(i).cloned())?.map_err(java_error)?;
        Ok(c.push(Value::Object(Rc::new(AttachedItemObject(item)))))
    });
    b.method("setItem", |c| {
        let (loc, item) = (opt_string(c, 1), item_arg(c, 2));
        attached(c, |a| a.set_item(loc.as_deref(), item))?.map_err(java_error)?;
        Ok(0)
    });
    b.method("getItem", |c| {
        let loc = opt_string(c, 1);
        let item = attached(c, |a| a.item(loc.as_deref()))?.map_err(java_error)?;
        Ok(c.push(item.map(|i| i.0).unwrap_or_default()))
    });
    b.method("getItemByIndex", |c| {
        let i = int_arg(c, 1);
        let item = attached(c, |a| a.item_by_index(i))?;
        Ok(c.push(item.map(|i| i.0).unwrap_or_default()))
    });
    b.method("remove", |c| {
        let item = LuaItem(c.arg(1));
        attached(c, |a| a.remove(&item))?;
        Ok(0)
    });
    b.method("clear", |c| {
        attached(c, |a| a.clear())?;
        Ok(0)
    });
    b.method("getLocation", |c| {
        let item = LuaItem(c.arg(1));
        let loc = attached(c, |a| a.location(&item).map(Value::from))?;
        Ok(c.push(loc.unwrap_or_default()))
    });
    b.method("contains", |c| {
        let item = LuaItem(c.arg(1));
        let v = attached(c, |a| a.contains(&item))?;
        Ok(c.push(v))
    });
    b.method("size", |c| {
        let v = attached(c, |a| a.size() as i32)?;
        Ok(c.push(v))
    });
    b.method("isEmpty", |c| {
        let v = attached(c, |a| a.is_empty())?;
        Ok(c.push(v))
    });

    let b = expose_class(thread, ATTACHED_ITEM_CLASS, None);
    b.ctor(|c| {
        let item = AttachedItem::new(opt_string(c, 0).as_deref(), item_arg(c, 1)).map_err(java_error)?;
        Ok(c.push(Value::Object(Rc::new(AttachedItemObject(item)))))
    });
    b.method("getLocation", |c| {
        let v = with_this::<AttachedItemObject, _>(c, |o| o.0.location().to_string())?;
        Ok(c.push(v))
    });
    b.method("getItem", |c| {
        let v = with_this::<AttachedItemObject, _>(c, |o| o.0.item().0.clone())?;
        Ok(c.push(v))
    });
}

/// `register` on the shared Characters.
pub fn register_shared(thread: &mut Thread) {
    register(thread, crate::bind_characters::shared_characters());
}

#[cfg(test)]
mod tests {
    use super::*;
    use pz_kahlua::TableRef;

    fn run(t: &mut Thread, src: &str) -> Result<(), String> {
        let p = pz_kahlua::compiler::compile(src, Some("test"), Some("test"), None).unwrap();
        let f = Value::Closure(pz_kahlua::vm::LuaClosure::new(Rc::new(p), t.globals().clone()));
        t.pcall(&f, &[]).map(|_| ()).map_err(|(e, tr)| format!("{}\n{tr}", pz_kahlua::exec::java_value_of(&e)))
    }

    #[test]
    fn attached_items_from_lua() {
        let mut t = Thread::new(TableRef::new());
        pz_kahlua::stdlib::install(&mut t, None).unwrap();
        run(&mut t, "function assert(v, msg) if not v then error(msg or 'assertion failed!', 2) end return v end").unwrap();
        let chars = Rc::new(RefCell::new(Characters::default()));
        register(&mut t, chars.clone());
        run(
            &mut t,
            r#"
            local g = AttachedLocations.getGroup("Critter")
            assert(g == AttachedLocations.getGroup("Critter"))
            local tail = g:getOrCreateLocation("tail")
            tail:setAttachmentName("tail_bone")
            assert(tail == g:getLocation("tail") and tail:getAttachmentName() == "tail_bone" and tail:getId() == "tail")
            g:getOrCreateLocation("ear"):setAttachmentName("ear_tip")
            assert(g:size() == 2 and g:indexOf("ear") == 1 and g:getLocationByIndex(0) == tail)
            assert(g:getLocation("nose") == nil and g:getLocationByIndex(-1) == nil)
            local a = AttachedItems.new(g)
            local bell, ribbon = {}, {}
            a:setItem("ear", bell)
            a:setItem("tail", ribbon)
            assert(a:size() == 2 and a:getItemByIndex(0) == ribbon and a:get(1):getItem() == bell)
            assert(a:get(1):getLocation() == "ear" and a:getLocation(bell) == "ear")
            assert(a:getItem("tail") == ribbon and a:getGroup() == g)
            local b = AttachedItems.new(a)
            a:remove(ribbon)
            assert(a:size() == 1 and b:size() == 2 and not a:contains(ribbon) and b:contains(ribbon))
            a:copyFrom(b)
            assert(a:size() == 2)
            a:setItem("ear", nil)
            a:clear()
            assert(a:isEmpty())
            assert(AttachedItem.new("tail", bell):getItem() == bell)
            AttachedLocations.Reset()
            assert(AttachedLocations.getGroup("Critter") ~= g)
            "#,
        )
        .unwrap();
        for (src, err) in [
            ("AttachedLocations.getGroup('Critter'):checkValid('nose')", "no such location \"nose\""),
            ("AttachedLocations.getGroup(nil)", "id is null"),
            ("AttachedLocations.getGroup('Critter'):getOrCreateLocation('')", "id is empty"),
            ("AttachedItem.new('tail', nil)", "item is null"),
            ("AttachedItems.new(AttachedLocations.getGroup('Critter')):setItem('x', {})", "no such location"),
        ] {
            let e = run(&mut t, src).unwrap_err();
            assert!(e.contains(err), "{src}: {e}");
        }
        assert_eq!(chars.borrow().attached_locations.groups().len(), 1);
    }
}
