//! `zombie.Lua.MapObjects`: the per-sprite Lua callbacks the
//! `media/lua/server/Map/MapObjects/MO*.lua` files register
//! (`OnNewWithSprite` for objects of a brand-new square, `OnLoadWithSprite`
//! for loaded ones), ordered by descending priority with one function per
//! priority.
//! The engine dispatches callbacks after erosion during chunk activation.
use crate::expose::{expose_class, int_arg};
use pz_kahlua::{LuaError, LuaResult, NativeCall, Thread, Value};
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use pz_world::{IsoObjectRef, IsoSpriteManager, SquareRef};
use pz_world::square_mutation::SquarePos;

type Registry = RefCell<HashMap<String, Rc<RefCell<Callback>>>>;

/// `MapObjects.Callback`.
#[derive(Default)]
struct Callback {
    functions: Vec<Value>,
    priority: Vec<i16>,
}

thread_local! {
    static ON_NEW: Registry = RefCell::default();
    static ON_LOAD: Registry = RefCell::default();
}

fn java_err(class: &'static str, message: &str) -> LuaError {
    LuaError::Runtime {
        message: message.into(),
        class,
    }
}

/// `OnNewWithSprite(String, LuaClosure, int)` / `OnLoadWithSprite` on one
/// map.
fn add(map: &Registry, sprite: &str, function: Value, priority: i32) {
    let mut map = map.borrow_mut();
    let mut cb = map.entry(sprite.to_string()).or_default().borrow_mut();
    for i in 0..cb.functions.len() {
        if i32::from(cb.priority[i]) < priority {
            cb.functions.insert(i, function);
            cb.priority.insert(i, priority as i16);
            return;
        }
        if i32::from(cb.priority[i]) != priority {
            continue;
        }
        cb.functions[i] = function;
        cb.priority[i] = priority as i16;
        return;
    }
    cb.functions.push(function);
    cb.priority.push(priority as i16);
}

/// Both overloads: a sprite name or a table of names.
fn with_sprite(
    c: &mut NativeCall<'_>,
    map: &'static std::thread::LocalKey<Registry>,
) -> LuaResult<usize> {
    let names = c.arg(0);
    let function = c.arg(1);
    let priority = int_arg(c, 2);
    let check_fn = |f: &Value| match f {
        Value::Nil => Err(java_err(
            "java.lang.NullPointerException",
            "function is null",
        )),
        Value::Closure(_) => Ok(()),
        other => Err(LuaError::runtime(format!(
            "Expected se.krka.kahlua.vm.LuaClosure but got {}",
            pz_kahlua::stdlib::java_class_name(other)
        ))),
    };
    match &names {
        Value::Table(t) => {
            if t.is_empty() {
                return Err(java_err(
                    "java.lang.IllegalArgumentException",
                    "invalid sprite-name table",
                ));
            }
            check_fn(&function)?;
            for (_, v) in t.entries() {
                let Value::Str(s) = &v else {
                    return Err(java_err(
                        "java.lang.IllegalArgumentException",
                        &format!(
                            "expected string but got \"{}\"",
                            pz_kahlua::exec::java_value_of(&v)
                        ),
                    ));
                };
                if s.is_empty() {
                    return Err(java_err(
                        "java.lang.IllegalArgumentException",
                        "invalid sprite name",
                    ));
                }
                map.with(|m| add(m, s, function.clone(), priority));
            }
        }
        Value::Str(s) if !s.is_empty() => {
            check_fn(&function)?;
            map.with(|m| add(m, s, function.clone(), priority));
        }
        Value::Str(_) | Value::Nil => {
            return Err(java_err(
                "java.lang.IllegalArgumentException",
                "invalid sprite name",
            ));
        }
        other => {
            return Err(LuaError::runtime(format!(
                "Expected java.lang.String but got {}",
                pz_kahlua::stdlib::java_class_name(other)
            )))
        }
    }
    Ok(0)
}

fn callbacks(
    map: &'static std::thread::LocalKey<Registry>,
    sprite: &str,
) -> Vec<Value> {
    map.with(|m| {
        m.borrow()
            .get(sprite)
            .map(|c| c.borrow().functions.clone())
            .unwrap_or_default()
    })
}

/// The `OnNewWithSprite` functions of a sprite, highest priority first.
pub fn on_new_callbacks(sprite: &str) -> Vec<Value> {
    callbacks(&ON_NEW, sprite)
}

/// The `OnLoadWithSprite` functions of a sprite, highest priority first.
pub fn on_load_callbacks(sprite: &str) -> Vec<Value> {
    callbacks(&ON_LOAD, sprite)
}

fn sprite_name(
    object: &IsoObjectRef,
    sprites: &IsoSpriteManager,
    saved_names: Option<&HashMap<i32, String>>,
) -> LuaResult<Option<String>> {
    use pz_world::objects::ObjectSprite;
    let object = object.try_borrow().map_err(|_| LuaError::runtime("IsoObject is in use"))?;
    if matches!(object.kind, pz_world::ObjectKind::WorldInventory(_)) || object.sprite.is_none() {
        return Ok(None);
    }
    let name = match &object.sprite {
        ObjectSprite::ArchiveUnmanaged(id) => sprites.by_id(*id).and_then(|s| s.name.clone()),
        sprite => sprite.try_read(sprites).map_err(crate::bind_world_object::host_error)?
            .and_then(|s| s.name.clone()),
    };
    Ok(name.or_else(|| object.sprite_name.as_ref()?.resolve(saved_names, sprites).map(str::to_owned)))
}

/// Iterate the initial object list. A callback can remove or rename its
/// receiver, change the registered functions, or reenter the dispatcher.
pub fn dispatch_square(
    thread: &mut Thread,
    square: &SquareRef,
    sprites: &RefCell<IsoSpriteManager>,
    saved_names: Option<&HashMap<i32, String>>,
    new_objects: bool,
) -> LuaResult<()> {
    let registry = if new_objects { &ON_NEW } else { &ON_LOAD };
    if registry.with(|map| map.borrow().is_empty()) { return Ok(()); }
    let (xyz, objects) = {
        let sq = square.try_borrow().map_err(|_| LuaError::runtime("IsoGridSquare is in use"))?;
        ((sq.x, sq.y, sq.z), sq.objects.iter().map(|o| o.ref_handle()).collect::<Vec<_>>())
    };
    let current_name = |object: &IsoObjectRef| -> LuaResult<Option<String>> {
        let sq = square.try_borrow().map_err(|_| LuaError::runtime("IsoGridSquare is in use"))?;
        if !sq.objects.iter().any(|slot| Rc::ptr_eq(&slot.ref_handle(), object)) {
            return Ok(None);
        }
        let manager = sprites.try_borrow().map_err(|_| LuaError::runtime("IsoSpriteManager is in use"))?;
        sprite_name(object, &manager, saved_names)
    };
    for object in objects {
        let Some(name) = current_name(&object)? else { continue };
        let Some(callbacks) = registry.with(|map| map.borrow().get(&name).cloned()) else { continue };
        let value = crate::bind_world::square_queries::square_object_value(&object, xyz)?;
        let mut index = 0;
        loop {
            let Some(function) = callbacks.borrow().functions.get(index).cloned() else { break };
            if let Err((message, trace)) = thread.pcall(&function, std::slice::from_ref(&value)) {
                (thread.log)(&format!("{}\n{trace}", pz_kahlua::exec::java_value_of(&message)));
            }
            if current_name(&object)?.as_deref() != Some(name.as_str()) { break; }
            index += 1;
        }
    }
    Ok(())
}

pub type TilePlacement = Rc<dyn Fn(&mut Thread, SquarePos, &str) -> LuaResult<Value>>;

thread_local! {
    static TILE_PLACEMENT: RefCell<Option<TilePlacement>> = RefCell::new(None);
}

pub fn install_tile_placement(place: Option<TilePlacement>) {
    TILE_PLACEMENT.with(|host| host.replace(place));
}

pub(crate) fn add_tile_object(c: &mut NativeCall<'_>, pos: SquarePos) -> LuaResult<Value> {
    let name = crate::expose::str_arg(c, 1).ok_or_else(|| LuaError::runtime("expected tile sprite name"))?;
    let place = TILE_PLACEMENT.with(|host| host.borrow().clone())
        .ok_or_else(|| LuaError::runtime("tile placement requires a loaded world"))?;
    place(c.thread, pos, &name)
}

/// `MapObjects.Reset()`.
pub fn reset() {
    ON_NEW.with(|m| m.borrow_mut().clear());
    ON_LOAD.with(|m| m.borrow_mut().clear());
}

pub fn register(thread: &mut Thread) {
    let b = expose_class(thread, "zombie.Lua.MapObjects", Some("java.lang.Object"));
    b.static_fn("OnNewWithSprite", |c| with_sprite(c, &ON_NEW));
    b.static_fn("OnLoadWithSprite", |c| with_sprite(c, &ON_LOAD));
    b.static_fn("Reset", |_| {
        reset();
        Ok(0)
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use pz_kahlua::TableRef;

    fn run(src: &str) -> Result<(), String> {
        let mut t = Thread::new(TableRef::new());
        pz_kahlua::stdlib::install(&mut t, None).unwrap();
        register(&mut t);
        let f = pz_kahlua::stdlib::load_string(src, Some("test"), t.globals().clone()).unwrap();
        t.call(&f, &[]).map(|_| ()).map_err(|e| e.to_string())
    }

    #[test]
    fn priorities_order_and_replace() {
        reset();
        run(r#"
            a = function() end; b = function() end; c = function() end; d = function() end
            MapObjects.OnNewWithSprite("s_0", a, 5)
            MapObjects.OnNewWithSprite("s_0", b, 10)
            MapObjects.OnNewWithSprite({"s_0", "s_1"}, c, 1)
            MapObjects.OnNewWithSprite("s_0", d, 5)
            MapObjects.OnLoadWithSprite("s_0", a, 5)
        "#)
        .unwrap();
        let n = on_new_callbacks("s_0");
        assert_eq!(n.len(), 3);
        assert_eq!(on_new_callbacks("s_1").len(), 1);
        assert_eq!(on_load_callbacks("s_0").len(), 1);
        assert!(on_new_callbacks("other").is_empty());
        let e = run(r#"MapObjects.OnNewWithSprite("", function() end, 1)"#).unwrap_err();
        assert!(e.contains("invalid sprite name"), "{e}");
        let e = run(r#"MapObjects.OnNewWithSprite("x", nil, 1)"#).unwrap_err();
        assert!(e.contains("function is null"), "{e}");
        let e = run(r#"MapObjects.OnNewWithSprite({}, function() end, 1)"#).unwrap_err();
        assert!(e.contains("invalid sprite-name table"), "{e}");
    }
}
