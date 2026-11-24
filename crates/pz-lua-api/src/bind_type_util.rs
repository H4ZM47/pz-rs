//! Build 42.22 utility classes and Lua-backed definition readers:
//! - `zombie.util.Type` (`tryCastTo`, `asBoolean`)
//! - `zombie.ChunkMapFilenames` (`instance`, `clear`, `getHeader`, `getFilename`, `getDir`)
//! - `DefaultClothing` reader over the Lua global `DefaultClothing` table
//!   (`Pants`, `TShirt`, `TShirtDecal`, `Vest` -> `hue`, `texture`, `tint`).

use crate::expose::{bool_arg, expose_class, int_arg, str_arg};
use pz_kahlua::value::LuaObject;
use pz_kahlua::{Thread, Value};
use std::any::Any;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

pub const TYPE_CLASS: &str = "zombie.util.Type";
pub const CHUNK_MAP_FILENAMES_CLASS: &str = "zombie.ChunkMapFilenames";

#[derive(Default)]
struct ChunkMapFilenamesState {
    headers: HashMap<(i32, i32), String>,
    prefix: String,
}

#[derive(Default, Clone)]
pub struct DefaultClothingEntry {
    pub hue: Vec<String>,
    pub texture: Vec<String>,
    pub tint: Vec<String>,
}

#[derive(Default, Clone)]
pub struct DefaultClothingData {
    pub pants: DefaultClothingEntry,
    pub tshirt: DefaultClothingEntry,
    pub tshirt_decal: DefaultClothingEntry,
    pub vest: DefaultClothingEntry,
    pub dirty: bool,
}

thread_local! {
    static CMF_STATE: RefCell<ChunkMapFilenamesState> = RefCell::new(ChunkMapFilenamesState::default());
    static CMF_INSTANCE: RefCell<Option<Rc<ChunkMapFilenamesObject>>> = const { RefCell::new(None) };
    static DEFAULT_CLOTHING: RefCell<DefaultClothingData> = RefCell::new(DefaultClothingData {
        dirty: true,
        ..Default::default()
    });
}

pub struct ChunkMapFilenamesObject;

impl LuaObject for ChunkMapFilenamesObject {
    fn class_name(&self) -> &str {
        CHUNK_MAP_FILENAMES_CLASS
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

fn cmf_instance() -> Rc<ChunkMapFilenamesObject> {
    CMF_INSTANCE.with(|slot| {
        slot.borrow_mut()
            .get_or_insert_with(|| Rc::new(ChunkMapFilenamesObject))
            .clone()
    })
}

/// `ChunkMapFilenames.instance.clear()`.
pub fn clear_chunk_map_filenames() {
    CMF_STATE.with(|s| {
        let mut st = s.borrow_mut();
        st.headers.clear();
        st.prefix.clear();
    });
}

/// `ChunkMapFilenames.instance.getHeader(wx, wy)`: caches `"map_{wx}_{wy}.bin"`.
pub fn chunk_map_header(wx: i32, wy: i32) -> String {
    CMF_STATE.with(|s| {
        let mut st = s.borrow_mut();
        st.headers
            .entry((wx, wy))
            .or_insert_with(|| format!("map_{wx}_{wy}.bin"))
            .clone()
    })
}

fn read_string_list(tbl: &pz_kahlua::TableRef, key: &str) -> Vec<String> {
    let Value::Table(sub) = tbl.get_str(key) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    let mut i = 1i64;
    loop {
        match sub.get_int(i) {
            Value::Nil => break,
            Value::Str(s) => out.push(s.to_string()),
            other => out.push(pz_kahlua::exec::java_value_of(&other)),
        }
        i += 1;
    }
    out
}

fn read_clothing_entry(root: &pz_kahlua::TableRef, name: &str) -> DefaultClothingEntry {
    let Value::Table(sub) = root.get_str(name) else {
        return DefaultClothingEntry::default();
    };
    DefaultClothingEntry {
        hue: read_string_list(&sub, "hue"),
        texture: read_string_list(&sub, "texture"),
        tint: read_string_list(&sub, "tint"),
    }
}

/// Populate `DefaultClothing` from the Lua global `DefaultClothing` table.
pub fn init_default_clothing_from_lua(thread: &Thread) {
    let data = match thread.globals().get_str("DefaultClothing") {
        Value::Table(root) => DefaultClothingData {
            pants: read_clothing_entry(&root, "Pants"),
            tshirt: read_clothing_entry(&root, "TShirt"),
            tshirt_decal: read_clothing_entry(&root, "TShirtDecal"),
            vest: read_clothing_entry(&root, "Vest"),
            dirty: false,
        },
        _ => DefaultClothingData {
            dirty: false,
            ..Default::default()
        },
    };
    DEFAULT_CLOTHING.with(|slot| *slot.borrow_mut() = data);
}

fn pick_from(list: &[String]) -> String {
    if list.is_empty() {
        return String::new();
    }
    let idx = pz_core::random::next(list.len() as i32).max(0) as usize;
    list.get(idx).cloned().unwrap_or_default()
}

pub fn pick_pants_hue() -> String {
    DEFAULT_CLOTHING.with(|d| pick_from(&d.borrow().pants.hue))
}
pub fn pick_pants_texture() -> String {
    DEFAULT_CLOTHING.with(|d| pick_from(&d.borrow().pants.texture))
}
pub fn pick_pants_tint() -> String {
    DEFAULT_CLOTHING.with(|d| pick_from(&d.borrow().pants.tint))
}
pub fn pick_tshirt_texture() -> String {
    DEFAULT_CLOTHING.with(|d| pick_from(&d.borrow().tshirt.texture))
}
pub fn pick_tshirt_tint() -> String {
    DEFAULT_CLOTHING.with(|d| pick_from(&d.borrow().tshirt.tint))
}
pub fn pick_tshirt_decal_texture() -> String {
    DEFAULT_CLOTHING.with(|d| pick_from(&d.borrow().tshirt_decal.texture))
}
pub fn pick_tshirt_decal_tint() -> String {
    DEFAULT_CLOTHING.with(|d| pick_from(&d.borrow().tshirt_decal.tint))
}
pub fn pick_vest_texture() -> String {
    DEFAULT_CLOTHING.with(|d| pick_from(&d.borrow().vest.texture))
}
pub fn pick_vest_tint() -> String {
    DEFAULT_CLOTHING.with(|d| pick_from(&d.borrow().vest.tint))
}

pub(crate) fn reset_session_handles() {
    clear_chunk_map_filenames();
    DEFAULT_CLOTHING.with(|d| {
        *d.borrow_mut() = DefaultClothingData {
            dirty: true,
            ..Default::default()
        };
    });
}

fn target_class_name(v: &Value) -> Option<String> {
    match v {
        Value::Str(s) => Some(s.to_string()),
        Value::Table(t) => match t.get_str("class") {
            Value::Str(s) => Some(s.to_string()),
            Value::Object(o) => Some(pz_kahlua::exec::java_value_of(&Value::Object(o))),
            _ => None,
        },
        Value::Object(o) => {
            let s = pz_kahlua::exec::java_value_of(&Value::Object(o.clone()));
            if let Some(stripped) = s.strip_prefix("class ") {
                Some(stripped.to_string())
            } else {
                Some(o.class_name().to_string())
            }
        }
        _ => None,
    }
}

fn matches_class(obj: &dyn LuaObject, target: &str) -> bool {
    let cls = obj.class_name();
    if cls == target || cls.rsplit('.').next() == Some(target) {
        return true;
    }
    obj.super_classes()
        .iter()
        .any(|&s| s == target || s.rsplit('.').next() == Some(target))
}

pub fn register(thread: &mut Thread) {
    let t = expose_class(thread, TYPE_CLASS, Some("java.lang.Object"));
    t.static_fn("tryCastTo", |c| {
        let obj = c.arg(0);
        let Some(target) = target_class_name(&c.arg(1)) else {
            return Ok(c.push(Value::Nil));
        };
        let matched = match &obj {
            Value::Object(o) => matches_class(&**o, &target),
            _ => false,
        };
        Ok(c.push(if matched { obj } else { Value::Nil }))
    });
    t.static_fn("asBoolean", |c| {
        let default = if c.nargs() >= 2 { bool_arg(c, 1) } else { false };
        let res = match c.arg(0) {
            Value::Bool(b) => b,
            Value::Nil => default,
            other => other.truthy(),
        };
        Ok(c.push(res))
    });

    let cmf = expose_class(thread, CHUNK_MAP_FILENAMES_CLASS, Some("java.lang.Object"));
    cmf.field("instance", Value::Object(cmf_instance()));
    cmf.method("clear", |_c| {
        clear_chunk_map_filenames();
        Ok(0)
    });
    cmf.method("getHeader", |c| {
        let wx = int_arg(c, 1);
        let wy = int_arg(c, 2);
        let h = chunk_map_header(wx, wy);
        Ok(c.push(Value::str(&h)))
    });
    cmf.method("getFilename", |c| {
        let wx = int_arg(c, 1);
        let wy = int_arg(c, 2);
        let h = chunk_map_header(wx, wy);
        Ok(c.push(Value::str(&h)))
    });
    cmf.method("getDir", |c| {
        let mode = str_arg(c, 1).map(|s| s.to_string()).unwrap_or_default();
        Ok(c.push(Value::str(&mode)))
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use pz_kahlua::{stdlib, TableRef};

    #[test]
    fn type_chunk_map_filenames_and_default_clothing_lua() {
        reset_session_handles();
        let mut t = Thread::new(TableRef::new());
        stdlib::install(&mut t, None).unwrap();
        register(&mut t);

        let src = r#"
            local inst = ChunkMapFilenames.instance
            local h = inst:getHeader(10, 20)
            local cast_ok = Type.tryCastTo(inst, "zombie.ChunkMapFilenames") == inst
            local cast_short = Type.tryCastTo(inst, "ChunkMapFilenames") == inst
            local cast_miss = Type.tryCastTo(inst, "zombie.iso.IsoObject") == nil
            local b1 = Type.asBoolean(true)
            local b2 = Type.asBoolean(nil, true)
            local b3 = Type.asBoolean(false, true)
            inst:clear()

            DefaultClothing = {
                Pants = { hue = { "0.5" }, texture = { "PantsTex" }, tint = { "1,1,1" } },
                TShirt = { texture = { "ShirtTex" }, tint = { "0.8,0.8,0.8" } },
                TShirtDecal = { texture = { "DecalTex" }, tint = { "0.5,0.5,0.5" } },
                Vest = { texture = { "VestTex" }, tint = { "0.2,0.2,0.2" } },
            }
            return h, cast_ok, cast_short, cast_miss, b1, b2, b3
        "#;
        let f = stdlib::load_string(src, Some("type_util"), t.globals().clone()).unwrap();
        let r = t.pcall(&f, &[]).map_err(|(e, tr)| format!("{e:?}\n{tr}")).unwrap();
        assert_eq!(pz_kahlua::exec::java_value_of(&r[0]), "map_10_20.bin");
        assert_eq!(pz_kahlua::exec::java_value_of(&r[1]), "true");
        assert_eq!(pz_kahlua::exec::java_value_of(&r[2]), "true");
        assert_eq!(pz_kahlua::exec::java_value_of(&r[3]), "true");
        assert_eq!(pz_kahlua::exec::java_value_of(&r[4]), "true");
        assert_eq!(pz_kahlua::exec::java_value_of(&r[5]), "true");
        assert_eq!(pz_kahlua::exec::java_value_of(&r[6]), "false");

        init_default_clothing_from_lua(&t);
        assert_eq!(pick_pants_hue(), "0.5");
        assert_eq!(pick_pants_texture(), "PantsTex");
        assert_eq!(pick_pants_tint(), "1,1,1");
        assert_eq!(pick_tshirt_texture(), "ShirtTex");
        assert_eq!(pick_tshirt_decal_texture(), "DecalTex");
        assert_eq!(pick_vest_texture(), "VestTex");
    }
}
