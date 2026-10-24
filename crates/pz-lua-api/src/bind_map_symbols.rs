//! `zombie.worldMap.symbols.MapSymbolDefinitions` and its nested
//! `MapSymbolDefinition`, exposed as globals `MapSymbolDefinitions` and
//! `MapSymbolDefinition` (the nested class also sits at
//! `MapSymbolDefinitions.MapSymbolDefinition`).
//!
//! The data lives in [`pz_core::map_symbols`]; this module keeps the
//! per-thread singleton (`getInstance`) and gives every definition one Lua
//! object so `==` compares identity like Java references.

use std::any::Any;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use pz_core::map_symbols::{MapSymbolDefinition, MapSymbolDefinitions};
use pz_fs::GameFs;
use pz_kahlua::value::LuaObject;
use pz_kahlua::{LuaError, LuaResult, NativeCall, Value};

use crate::expose::{expose_class, int_arg, str_arg, with_this};
use crate::LuaManager;

pub const MAP_SYMBOL_DEFINITIONS: &str = "zombie.worldMap.symbols.MapSymbolDefinitions";
pub const MAP_SYMBOL_DEFINITION: &str =
    "zombie.worldMap.symbols.MapSymbolDefinitions$MapSymbolDefinition";

/// A `MapSymbolDefinitions` instance (the singleton or one from `new()`).
pub struct DefinitionsObj(pub Rc<RefCell<MapSymbolDefinitions>>);

impl LuaObject for DefinitionsObj {
    fn class_name(&self) -> &str {
        MAP_SYMBOL_DEFINITIONS
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// A `MapSymbolDefinition`; `None` is one made by `MapSymbolDefinition.new()`
/// (all fields `null`/0 in the original).
pub struct DefinitionObj(pub Option<Rc<MapSymbolDefinition>>);

impl LuaObject for DefinitionObj {
    fn class_name(&self) -> &str {
        MAP_SYMBOL_DEFINITION
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

thread_local! {
    /// `MapSymbolDefinitions.instance`, created by the first `getInstance()`.
    static INSTANCE: RefCell<Option<Value>> = const { RefCell::new(None) };
    /// Lua objects by definition pointer. Each entry holds its `Rc`, so a
    /// pointer cannot be reused while it is cached.
    static OBJECTS: RefCell<HashMap<usize, Value>> = RefCell::new(HashMap::new());
}

/// The singleton's data (creating it), for engine code that reads symbols.
pub fn instance() -> Rc<RefCell<MapSymbolDefinitions>> {
    let v = instance_value();
    let Value::Object(o) = v else { unreachable!() };
    o.as_any()
        .downcast_ref::<DefinitionsObj>()
        .unwrap()
        .0
        .clone()
}

fn instance_value() -> Value {
    INSTANCE.with(|i| {
        i.borrow_mut()
            .get_or_insert_with(|| {
                Value::Object(Rc::new(DefinitionsObj(Rc::new(RefCell::new(
                    MapSymbolDefinitions::new(),
                )))))
            })
            .clone()
    })
}

fn definition_value(d: Option<&Rc<MapSymbolDefinition>>) -> Value {
    let Some(d) = d else { return Value::Nil };
    let key = Rc::as_ptr(d) as usize;
    OBJECTS.with(|m| {
        m.borrow_mut()
            .entry(key)
            .or_insert_with(|| Value::Object(Rc::new(DefinitionObj(Some(d.clone())))))
            .clone()
    })
}

fn defs(c: &NativeCall<'_>) -> LuaResult<Rc<RefCell<MapSymbolDefinitions>>> {
    with_this::<DefinitionsObj, _>(c, |d| d.0.clone())
}

/// Push `f(def)`, or `blank` for a `MapSymbolDefinition.new()` object.
fn field(
    c: &mut NativeCall<'_>,
    blank: Value,
    f: impl FnOnce(&MapSymbolDefinition) -> Value,
) -> LuaResult<usize> {
    let v = with_this::<DefinitionObj, _>(c, |d| d.0.as_deref().map(f))?;
    Ok(c.push(v.unwrap_or(blank)))
}

/// `Texture.getSharedTexture(path) != null`.
fn texture_found(fs: &RefCell<GameFs>, path: &str) -> bool {
    crate::bind_render::textures()
        .borrow_mut()
        .shared_texture(&fs.borrow(), path)
        .is_some()
}

pub fn register(mgr: &mut LuaManager) {
    let fs = mgr.fs.clone();
    let thread = &mut mgr.thread;

    let b = expose_class(thread, MAP_SYMBOL_DEFINITIONS, None);
    b.static_fn("getInstance", |c| Ok(c.push(instance_value())));
    b.static_fn("Reset", |_c| {
        if let Some(Value::Object(o)) = INSTANCE.with(|i| i.borrow().clone()) {
            let d = o.as_any().downcast_ref::<DefinitionsObj>().unwrap();
            let mut d = d.0.borrow_mut();
            OBJECTS.with(|m| {
                let mut m = m.borrow_mut();
                for i in 0..d.symbol_count() {
                    m.remove(&(Rc::as_ptr(d.symbol_by_index(i).unwrap()) as usize));
                }
            });
            d.clear();
        }
        Ok(0)
    });
    b.ctor(|c| {
        let v = Value::Object(Rc::new(DefinitionsObj(Rc::new(RefCell::new(
            MapSymbolDefinitions::new(),
        )))));
        Ok(c.push(v))
    });
    // Overloads by argument count: (id, path), (id, path, tab),
    // (id, path, width, height, tab). Nil strings become "" (tab stays null).
    b.method("addTexture", move |c| {
        let d = defs(c)?;
        let s = |c: &NativeCall<'_>, i| str_arg(c, i).map(|s| s.to_string()).unwrap_or_default();
        let (id, path) = (s(c, 1), s(c, 2));
        match c.nargs() {
            3 => {
                let found = texture_found(&fs, &path);
                d.borrow_mut().add_texture(&id, &path, None, found);
            }
            4 => {
                let tab = str_arg(c, 3);
                let found = texture_found(&fs, &path);
                d.borrow_mut()
                    .add_texture(&id, &path, tab.as_deref(), found);
            }
            6 => {
                let tab = str_arg(c, 5);
                d.borrow_mut().add_texture_sized(
                    &id,
                    &path,
                    int_arg(c, 3),
                    int_arg(c, 4),
                    tab.as_deref(),
                );
            }
            n => {
                return Err(LuaError::runtime(format!(
                    "No implementation found for addTexture with {} arguments",
                    n.saturating_sub(1)
                )))
            }
        }
        Ok(0)
    });
    b.method("getSymbolCount", |c| {
        let n = defs(c)?.borrow().symbol_count();
        Ok(c.push(n as f64))
    });
    b.method("getSymbolByIndex", |c| {
        let d = defs(c)?;
        let i = int_arg(c, 1);
        let d = d.borrow();
        let len = d.symbol_count();
        let def = usize::try_from(i)
            .ok()
            .and_then(|i| d.symbol_by_index(i))
            .ok_or_else(|| LuaError::Runtime {
                message: format!("Index {i} out of bounds for length {len}"),
                class: "java.lang.IndexOutOfBoundsException",
            })?;
        Ok(c.push(definition_value(Some(def))))
    });
    b.method("getSymbolById", |c| {
        let d = defs(c)?;
        let id = str_arg(c, 1).unwrap_or_else(|| Rc::from(""));
        let v = definition_value(d.borrow().symbol_by_id(&id));
        Ok(c.push(v))
    });

    let b = expose_class(thread, MAP_SYMBOL_DEFINITION, None);
    b.ctor(|c| Ok(c.push(Value::Object(Rc::new(DefinitionObj(None))))));
    b.method("getId", |c| field(c, Value::Nil, |d| Value::str(&d.id)));
    b.method("getTexturePath", |c| {
        field(c, Value::Nil, |d| Value::str(&d.texture_path))
    });
    b.method("getWidth", |c| {
        field(c, Value::Num(0.0), |d| Value::Num(f64::from(d.width)))
    });
    b.method("getHeight", |c| {
        field(c, Value::Num(0.0), |d| Value::Num(f64::from(d.height)))
    });
    b.method("getTab", |c| {
        field(c, Value::Nil, |d| {
            d.tab.as_deref().map_or(Value::Nil, Value::str)
        })
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A valid 1x1 RGBA PNG.
    const PNG_1X1: &[u8] = &[
        0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48, 0x44,
        0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1f,
        0x15, 0xc4, 0x89, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9c, 0x63, 0xf8,
        0xcf, 0xc0, 0xf0, 0x1f, 0x00, 0x05, 0x00, 0x01, 0xff, 0x89, 0x99, 0x3d, 0x1d, 0x00, 0x00,
        0x00, 0x00, 0x49, 0x45, 0x4e, 0x44, 0xae, 0x42, 0x60, 0x82,
    ];

    #[test]
    fn map_symbol_definitions_from_lua() {
        let dir = std::env::temp_dir().join(format!("pz-bind-map-symbols-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("media/ui/LootableMaps")).unwrap();
        std::fs::write(dir.join("media/ui/LootableMaps/sym.png"), PNG_1X1).unwrap();
        let fs = Rc::new(RefCell::new(GameFs::new(&dir)));
        let mut mgr = LuaManager::init(fs).unwrap();
        let src = r#"
            local d = MapSymbolDefinitions.getInstance()
            if d ~= MapSymbolDefinitions.getInstance() then error("singleton") end
            d:addTexture("Found", "media/ui/LootableMaps/sym.png", "Locations")
            d:addTexture("Missing", "media/ui/LootableMaps/nope.png")
            d:addTexture("Sized", "media/x.png", 7, 9, nil)
            local a = d:getSymbolById("Found")
            if a ~= d:getSymbolByIndex(0) then error("identity") end
            local ok = pcall(function() return d:getSymbolByIndex(3) end)
            local m = d:getSymbolByIndex(1)
            local s = d:getSymbolById("Sized")
            local other = MapSymbolDefinitions.new()
            local blank = MapSymbolDefinition.new()
            local n = d:getSymbolCount()
            local r = { n, a:getId(), a:getTexturePath(), a:getWidth(), a:getHeight(), a:getTab(),
                m:getWidth(), m:getTab(), s:getWidth(), s:getHeight(), ok, d:getSymbolById("None"),
                other:getSymbolCount(), blank:getId(), blank:getWidth(),
                MapSymbolDefinitions.MapSymbolDefinition == MapSymbolDefinition }
            MapSymbolDefinitions.Reset()
            return r, d:getSymbolCount()
        "#;
        let proto = pz_kahlua::compiler::compile(src, Some("test"), Some("test"), None).unwrap();
        let env = mgr.thread.globals().clone();
        let f = Value::Closure(pz_kahlua::vm::LuaClosure::new(Rc::new(proto), env));
        let out = mgr.thread.call(&f, &[]).unwrap();
        let t = out[0].as_table().unwrap();
        let mut r: Vec<Value> = (1..=16).map(|i| t.get_int(i)).collect();
        r.push(out[1].clone());
        let num = |i: usize| r[i].as_num().unwrap();
        assert_eq!(num(0), 3.0);
        assert_eq!(r[1].as_str(), Some("Found"));
        assert_eq!(r[2].as_str(), Some("media/ui/LootableMaps/sym.png"));
        assert_eq!((num(3), num(4)), (20.0, 20.0), "texture found");
        assert_eq!(r[5].as_str(), Some("Locations"));
        assert_eq!(num(6), 18.0, "texture missing");
        assert!(r[7].is_nil(), "no tab");
        assert_eq!((num(8), num(9)), (7.0, 9.0));
        assert!(!r[10].truthy(), "index out of range throws");
        assert!(r[11].is_nil());
        assert_eq!(num(12), 0.0, "new() is a separate instance");
        assert!(r[13].is_nil());
        assert_eq!(num(14), 0.0);
        assert!(r[15].truthy());
        assert_eq!(num(16), 0.0, "Reset clears the singleton");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
