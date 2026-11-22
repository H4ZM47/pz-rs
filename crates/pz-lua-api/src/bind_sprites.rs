//! `IsoSpriteManager.instance`, `IsoSprite` and the sprite's
//! `PropertyContainer`, over the world's sprite manager the engine installs
//! ([`install_sprite_manager`]) once the tile definitions are loaded
//! (`media/lua/shared/Util/CustomTileProps.lua` edits tile properties on
//! `OnGameStart`).
//!
//! `IsoSpriteManager.instance` is one object from the start, as in Java;
//! its methods throw while no manager is installed.

use crate::expose::{expose_class, int_arg, str_arg, JavaEnum};
use pz_kahlua::value::LuaObject;
use pz_kahlua::{LuaError, LuaResult, NativeCall, Thread, Value};
use pz_world::sprite::{IsoSpriteManager, PropertyContainer, SpriteIdx};
use std::any::Any;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::{Rc, Weak};

const MANAGER: &str = "zombie.iso.sprite.IsoSpriteManager";
const SPRITE: &str = "zombie.iso.sprite.IsoSprite";
const PROPS: &str = "zombie.core.properties.PropertyContainer";

thread_local! {
    static MANAGER_INSTANCE: Rc<ManagerObj> = Rc::new(ManagerObj);
    static SPRITES: RefCell<Option<Rc<SpriteRegistry>>> = const { RefCell::new(None) };
}

/// Install (or clear) the sprite manager behind `IsoSpriteManager.instance`.
pub fn install_sprite_manager(m: Option<Rc<RefCell<IsoSpriteManager>>>) {
    SPRITES.with_borrow_mut(|slot| {
        if let (Some(active), Some(next)) = (&*slot, &m) {
            if Rc::ptr_eq(&active.manager, next) {
                return;
            }
        }
        *slot = m.map(|manager| Rc::new(SpriteRegistry {
            manager,
            handles: RefCell::default(),
        }));
    });
}

pub(crate) fn manager() -> LuaResult<Rc<RefCell<IsoSpriteManager>>> {
    SPRITES
        .with_borrow(|s| s.as_ref().map(|owner| owner.manager.clone()))
        .ok_or_else(|| LuaError::runtime("IsoSpriteManager: tile definitions are not loaded"))
}

fn with_manager<R>(f: impl FnOnce(&mut IsoSpriteManager) -> LuaResult<R>) -> LuaResult<R> {
    let _collection_deferral = pz_kahlua::gc::defer_collection();
    let m = manager()?;
    let mut m = m
        .try_borrow_mut()
        .map_err(|_| LuaError::runtime("IsoSpriteManager is in use"))?;
    f(&mut m)
}

/// `IsoSpriteManager.instance.getSprite(name).getProperties().get(key)`
/// (adding a sprite for an unknown name, as `getSprite` does); `None` while
/// no manager is installed.
pub fn sprite_property(name: &str, key: &str) -> Option<Option<String>> {
    let _collection_deferral = pz_kahlua::gc::defer_collection();
    let m = manager().ok()?;
    let mut m = m.try_borrow_mut().ok()?;
    let i = match m.named_map.get(name) {
        Some(&i) => i,
        None => m.add_sprite_named(name),
    };
    Some(m.sprites[i].properties.get(&m.alias, key).map(str::to_string))
}

struct ManagerObj;

/// Lua handle for the permanent IsoSpriteManager.instance.
pub fn instance_value() -> Value {
    MANAGER_INSTANCE.with(|instance| Value::Object(instance.clone()))
}

impl LuaObject for ManagerObj {
    fn class_name(&self) -> &str {
        MANAGER
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// Each Lua handle uses the registry that allocated its sprite index. The
/// weak cache gives name and ID lookups the same Lua identity.
struct SpriteRegistry {
    manager: Rc<RefCell<IsoSpriteManager>>,
    handles: RefCell<HashMap<(SpriteIdx, bool), Weak<SpriteObj>>>,
}

#[derive(Clone)]
struct SpriteObj {
    owner: Option<Rc<SpriteRegistry>>,
    idx: SpriteIdx,
    props: bool,
}

impl SpriteObj {
    fn with_manager<R>(&self, f: impl FnOnce(&mut IsoSpriteManager) -> LuaResult<R>) -> LuaResult<R> {
        let _collection_deferral = pz_kahlua::gc::defer_collection();
        let owner = self.owner.as_ref()
            .ok_or_else(|| LuaError::runtime("IsoSpriteManager: tile definitions are not loaded"))?;
        let mut manager = owner.manager.try_borrow_mut()
            .map_err(|_| LuaError::runtime("IsoSpriteManager is in use"))?;
        if self.idx >= manager.sprites.len() {
            return Err(LuaError::runtime("IsoSprite index is outside its tile definitions"));
        }
        f(&mut manager)
    }
}

impl LuaObject for SpriteObj {
    fn class_name(&self) -> &str {
        if self.props {
            PROPS
        } else {
            SPRITE
        }
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// An `IsoGridSquare`'s `PropertyContainer`, copied when Lua reads it.
struct SquareProps(PropertyContainer);

impl LuaObject for SquareProps {
    fn class_name(&self) -> &str {
        PROPS
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// The Lua `PropertyContainer` of a square's `properties`.
pub(crate) fn square_props_value(props: PropertyContainer) -> Value {
    Value::Object(Rc::new(SquareProps(props)))
}

/// Run `f` on the `PropertyContainer` that receives `has` or `get`.
fn with_props<R>(
    c: &NativeCall<'_>,
    f: impl FnOnce(&IsoSpriteManager, &PropertyContainer) -> LuaResult<R>,
) -> LuaResult<R> {
    if let Value::Object(o) = c.arg(0) {
        if let Some(square) = o.as_any().downcast_ref::<SquareProps>() {
            return with_manager(|m| f(m, &square.0));
        }
    }
    let sprite = self_sprite(c, true)?;
    sprite.with_manager(|m| f(m, &m.sprites[sprite.idx].properties))
}

pub(crate) fn sprite_value(idx: Option<SpriteIdx>, props: bool) -> Value {
    idx.map_or(Value::Nil, |idx| {
        sprite_handle(SPRITES.with_borrow(Clone::clone), idx, props)
    })
}

fn sprite_handle(owner: Option<Rc<SpriteRegistry>>, idx: SpriteIdx, props: bool) -> Value {
    let Some(registry) = &owner else {
        return Value::Object(Rc::new(SpriteObj { owner, idx, props }));
    };
    let mut handles = registry.handles.borrow_mut();
    let key = (idx, props);
    if let Some(handle) = handles.get(&key).and_then(Weak::upgrade) {
        return Value::Object(handle);
    }
    let handle = Rc::new(SpriteObj { owner: owner.clone(), idx, props });
    handles.insert(key, Rc::downgrade(&handle));
    Value::Object(handle)
}

fn self_sprite(c: &NativeCall<'_>, props: bool) -> LuaResult<SpriteObj> {
    match c.arg(0) {
        Value::Object(o) => match o.as_any().downcast_ref::<SpriteObj>() {
            Some(s) if s.props == props => Ok(s.clone()),
            _ => Err(LuaError::runtime(format!(
                "Expected {} but got {}",
                if props { PROPS } else { SPRITE },
                o.class_name()
            ))),
        },
        other => Err(LuaError::runtime(format!(
            "Expected an object but got {}",
            other.type_name()
        ))),
    }
}

/// An `IsoFlagType` Lua constant's name.
fn flag_name(v: &Value) -> Option<&'static str> {
    match v {
        Value::Object(o) => o
            .as_any()
            .downcast_ref::<JavaEnum>()
            .filter(|e| e.class == crate::bind_iso_enums::ISO_FLAG_TYPE)
            .map(|e| e.name),
        _ => None,
    }
}

/// A `String` key, or an `IsoPropertyType`'s name: the `IsoPropertyType`
/// overloads of `set`, `unset`, `has` and `get` call `getName()` and then
/// the `String` overload.
fn key_name(c: &NativeCall<'_>, i: usize) -> Option<String> {
    crate::bind_iso_enums::property_type_name(&c.arg(i)).or_else(|| str_arg(c, i).map(|s| s.to_string()))
}

pub fn register(thread: &mut Thread) {
    let b = expose_class(thread, MANAGER, Some("java.lang.Object"));
    b.field("instance", instance_value());
    // `getSprite(String)` (adding a sprite for an unknown name) and
    // `getSprite(int)`.
    b.method("getSprite", |c| {
        crate::expose::this::<ManagerObj>(c)?;
        let arg = c.arg(1);
        let idx = with_manager(|m| {
            Ok(match &arg {
                Value::Num(_) => m.idx_by_id(int_arg(c, 1)),
                Value::Str(name) => Some(match m.named_map.get(&**name) {
                    Some(&i) => i,
                    None => m.add_sprite_named(name),
                }),
                _ => None,
            })
        })?;
        Ok(c.push(sprite_value(idx, false)))
    });
    let s = expose_class(thread, SPRITE, Some("java.lang.Object"));
    s.method("getProperties", |c| {
        let sprite = self_sprite(c, false)?;
        Ok(c.push(sprite_handle(sprite.owner, sprite.idx, true)))
    });
    s.method("getName", |c| {
        let sprite = self_sprite(c, false)?;
        let n = sprite.with_manager(|m| Ok(m.sprites[sprite.idx].name.clone()))?;
        Ok(c.push(n.map_or(Value::Nil, |n| Value::str(&n))))
    });
    s.method("getID", |c| {
        let sprite = self_sprite(c, false)?;
        let id = sprite.with_manager(|m| Ok(m.sprites[sprite.idx].id))?;
        Ok(c.push(id))
    });
    let p = expose_class(thread, PROPS, Some("java.lang.Object"));
    // `set(String)`, `set(String, String)`, `set(IsoFlagType)`,
    // `set(IsoFlagType, String)`, `set(IsoPropertyType, String)`: a
    // property name that names an `IsoFlagType` sets the flag.
    p.method("set", |c| {
        let sprite = self_sprite(c, true)?;
        let key = c.arg(1);
        let value = if c.nargs() > 2 {
            str_arg(c, 2).map(|s| s.to_string())
        } else {
            Some(String::new())
        };
        sprite.with_manager(|m| {
            let IsoSpriteManager {
                tables,
                alias,
                sprites,
                ..
            } = m;
            let props = &mut sprites[sprite.idx].properties;
            if let Some(f) = flag_name(&key) {
                if let Some(flag) = tables.flag_from_string(f) {
                    props.set_flag(flag);
                }
                return Ok(());
            }
            let Some(name) = key_name(c, 1) else {
                return Ok(());
            };
            props.set(tables, alias, &name, value.as_deref().unwrap_or(""), true);
            Ok(())
        })?;
        Ok(0)
    });
    // `unset(String)` / `unset(IsoFlagType)` / `unset(IsoPropertyType)`.
    p.method("unset", |c| {
        let sprite = self_sprite(c, true)?;
        let key = c.arg(1);
        sprite.with_manager(|m| {
            let IsoSpriteManager {
                tables,
                alias,
                sprites,
                ..
            } = m;
            let props = &mut sprites[sprite.idx].properties;
            if let Some(f) = flag_name(&key) {
                if let Some(flag) = tables.flag_from_string(f) {
                    props.unset_flag(flag);
                }
                return Ok(());
            }
            if let Some(name) = key_name(c, 1) {
                props.unset(alias, &name);
            }
            Ok(())
        })?;
        Ok(0)
    });
    // `has(String)` / `has(IsoFlagType)` / `has(IsoPropertyType)`.
    p.method("has", |c| {
        let key = c.arg(1);
        let r = with_props(c, |m, props| {
            Ok(match flag_name(&key) {
                Some(f) => m
                    .tables
                    .flag_from_string(f)
                    .is_some_and(|flag| props.has(flag)),
                None => key_name(c, 1).is_some_and(|n| props.has_name(&m.alias, &n)),
            })
        })?;
        Ok(c.push(r))
    });
    // `get(String)` / `get(IsoPropertyType)`.
    p.method("get", |c| {
        let v = with_props(c, |m, props| {
            Ok(key_name(c, 1).and_then(|n| props.get(&m.alias, &n).map(str::to_string)))
        })?;
        Ok(c.push(v.map_or(Value::Nil, |v| Value::str(&v))))
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bind_iso_enums::{register_with, IsoClassData};
    use pz_kahlua::TableRef;

    #[test]
    fn retained_sprites_and_properties_use_their_original_tile_definitions() {
        let previous = manager().ok();
        let make_manager = || {
            let mut m = IsoSpriteManager::new(pz_world::sprite::test_tables());
            let mut values = pz_world::sprite::PropertyValueMap::default();
            values.put("CustomName", vec!["Original".into(), "Retained".into()]);
            m.alias = pz_world::sprite::TilePropertyAliasMap::generate(&values, &[]);
            Rc::new(RefCell::new(m))
        };
        let old = make_manager();
        let old_lifetime = Rc::downgrade(&old);
        {
            let mut tiles = old.borrow_mut();
            tiles.add_sprite_named("first");
            let idx = tiles.add_sprite("old", 17);
            tiles.sprites[idx].name = Some("old".into());
        }
        install_sprite_manager(Some(old));
        let mut t = Thread::new(TableRef::new());
        pz_kahlua::stdlib::install(&mut t, None).unwrap();
        register(&mut t);
        let run = |t: &mut Thread, code| {
            let f = pz_kahlua::stdlib::load_string(code, Some("sprite owners"), t.globals().clone()).unwrap();
            let values = t.call(&f, &[]).unwrap();
            assert!(!values.is_empty() && values.iter().all(|v| matches!(v, Value::Bool(true))), "{values:?}");
        };
        run(&mut t, r#"
            retained = IsoSpriteManager.instance:getSprite('old')
            retainedProps = retained:getProperties()
            retainedProps:set('CustomName', 'Original')
            return retained == IsoSpriteManager.instance:getSprite('old'), retainedProps == retained:getProperties(),
                retained == IsoSpriteManager.instance:getSprite(17)
        "#);
        install_sprite_manager(Some(make_manager()));
        run(&mut t, r#"
            local current = IsoSpriteManager.instance:getSprite('new')
            local old, props = retained, retainedProps
            retained, retainedProps = nil, nil
            local original = props:get('CustomName')
            props:set('CustomName', 'Retained')
            return current ~= old, old:getName() == 'old', old:getProperties() == props,
                original == 'Original', current:getProperties():get('CustomName') == nil,
                old:getProperties():get('CustomName') == 'Retained'
        "#);
        assert!(old_lifetime.upgrade().is_none());
        install_sprite_manager(previous);
    }

    #[test]
    fn property_container_takes_iso_property_type_keys() {
        let old = manager().ok();
        let mut m = IsoSpriteManager::new(pz_world::sprite::test_tables());
        let mut values = pz_world::sprite::PropertyValueMap::default();
        values.put("CustomName", vec!["Grave".into()]);
        m.alias = pz_world::sprite::TilePropertyAliasMap::generate(&values, &[]);
        install_sprite_manager(Some(Rc::new(RefCell::new(m))));
        let mut t = Thread::new(TableRef::new());
        pz_kahlua::stdlib::install(&mut t, None).unwrap();
        crate::bind_java_util::register(&mut t);
        let data = IsoClassData {
            property_types: vec![("CUSTOM_NAME".into(), "CustomName".into())],
            ..Default::default()
        };
        register_with(&mut t, &data);
        register(&mut t);
        let src = "local p = IsoSpriteManager.instance:getSprite('grave_1'):getProperties()
            local before = p:has(IsoPropertyType.CUSTOM_NAME)
            p:set(IsoPropertyType.CUSTOM_NAME, 'Grave')
            local r = { before, p:has(IsoPropertyType.CUSTOM_NAME), p:get(IsoPropertyType.CUSTOM_NAME), p:get('CustomName') }
            p:unset(IsoPropertyType.CUSTOM_NAME)
            r[5] = p:has('CustomName')
            return r[1], r[2], r[3], r[4], r[5]";
        let proto = pz_kahlua::compiler::compile(src, Some("t"), Some("t"), None).unwrap();
        let f = Value::Closure(pz_kahlua::vm::LuaClosure::new(Rc::new(proto), t.globals().clone()));
        let r = t.call(&f, &[]).unwrap_or_else(|e| panic!("{e}"));
        install_sprite_manager(old);
        let shown: Vec<String> = r.iter().map(pz_kahlua::exec::java_value_of).collect();
        assert!(!r[0].truthy() && r[1].truthy() && !r[4].truthy(), "{shown:?}");
        assert_eq!(pz_kahlua::exec::java_value_of(&r[2]), "Grave");
        assert_eq!(pz_kahlua::exec::java_value_of(&r[3]), "Grave");
    }
}
