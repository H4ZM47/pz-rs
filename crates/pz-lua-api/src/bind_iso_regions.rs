//! `zombie.iso.areas.isoregion.IsoRegions`, `DataChunk`, and `IsoRegionsLogger`
//! Lua bindings backed by `pz_world::isoregion::IsoRegions`.

use std::any::Any;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

use pz_kahlua::value::LuaObject;
use pz_kahlua::{Thread, Value};

use crate::bind_java_util::JavaList;
use crate::bind_world::try_with_world_read;
use crate::expose::{bool_arg, expose_class, int_arg, with_this};

const BIT_WALL_N: i8 = 1;
const BIT_WALL_W: i8 = 2;
const BIT_PATH_WALL_N: i8 = 4;
const BIT_PATH_WALL_W: i8 = 8;
const BIT_HAS_FLOOR: i8 = 16;
const BIT_STAIRCASE: i8 = 32;
const BIT_HAS_ROOF: i8 = 64;

pub const ISO_REGIONS_CLASS: &str = "zombie.iso.areas.isoregion.IsoRegions";
pub const DATA_CHUNK_CLASS: &str = "zombie.iso.areas.isoregion.data.DataChunk";
pub const LOGGER_CLASS: &str = "zombie.iso.areas.isoregion.IsoRegionsLogger";

/// Lua handle to one `IsoRegions` data chunk `(cx, cy)` with selected-square flags.
pub struct LuaDataChunk {
    pub cx: i32,
    pub cy: i32,
    pub selected_flags: Cell<i8>,
}

impl LuaObject for LuaDataChunk {
    fn class_name(&self) -> &str {
        DATA_CHUNK_CLASS
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// Lua handle to `IsoRegionsLogger`.
pub struct LuaIsoRegionsLogger {
    dirty_ui: Cell<bool>,
    logs: RefCell<Vec<Value>>,
}

impl LuaObject for LuaIsoRegionsLogger {
    fn class_name(&self) -> &str {
        LOGGER_CLASS
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

thread_local! {
    static DEBUG_LOAD_ALL: Cell<bool> = const { Cell::new(false) };
    static LOGGER: Rc<LuaIsoRegionsLogger> = Rc::new(LuaIsoRegionsLogger {
        dirty_ui: Cell::new(false),
        logs: RefCell::new(Vec::new()),
    });
}

fn square_flags_at(x: i32, y: i32, z: i32) -> i8 {
    try_with_world_read(|w| {
        Ok(w.regions
            .as_ref()
            .map(|r| r.borrow().get_square_flags(x, y, z))
            .unwrap_or(0))
    })
    .unwrap_or(0)
}

/// Expose `IsoRegions`, `DataChunk`, and `IsoRegionsLogger`.
pub fn register(thread: &mut Thread) {
    let dc = expose_class(thread, DATA_CHUNK_CLASS, Some("java.lang.Object"));
    dc.method("getChunkX", |c| {
        let v = with_this::<LuaDataChunk, _>(c, |ch| ch.cx)?;
        Ok(c.push(v))
    });
    dc.method("getChunkY", |c| {
        let v = with_this::<LuaDataChunk, _>(c, |ch| ch.cy)?;
        Ok(c.push(v))
    });
    dc.method("setSelectedFlags", |c| {
        let (lx, ly, z) = (int_arg(c, 1), int_arg(c, 2), int_arg(c, 3));
        with_this::<LuaDataChunk, _>(c, |ch| {
            let wx = ch.cx * 8 + lx;
            let wy = ch.cy * 8 + ly;
            ch.selected_flags.set(square_flags_at(wx, wy, z));
        })?;
        Ok(0)
    });
    dc.method("selectedHasFlags", |c| {
        let bit = int_arg(c, 1) as i8;
        let has = with_this::<LuaDataChunk, _>(c, |ch| (ch.selected_flags.get() & bit) != 0)?;
        Ok(c.push(has))
    });
    dc.method("getSquare", |c| {
        let (lx, ly, z) = (int_arg(c, 1), int_arg(c, 2), int_arg(c, 3));
        let flags = with_this::<LuaDataChunk, _>(c, |ch| {
            square_flags_at(ch.cx * 8 + lx, ch.cy * 8 + ly, z) as i32
        })?;
        Ok(c.push(flags))
    });

    let lg = expose_class(thread, LOGGER_CLASS, Some("java.lang.Object"));
    lg.method("getLogs", |c| {
        let items = with_this::<LuaIsoRegionsLogger, _>(c, |l| l.logs.borrow().clone())?;
        Ok(c.push(Value::Object(Rc::new(JavaList::new(items)))))
    });
    lg.method("isDirtyUI", |c| {
        let v = with_this::<LuaIsoRegionsLogger, _>(c, |l| l.dirty_ui.get())?;
        Ok(c.push(v))
    });
    lg.method("unsetDirtyUI", |c| {
        with_this::<LuaIsoRegionsLogger, _>(c, |l| l.dirty_ui.set(false))?;
        Ok(0)
    });

    let b = expose_class(thread, ISO_REGIONS_CLASS, Some("java.lang.Object"));
    b.field("BIT_EMPTY", Value::from(0));
    b.field("BIT_WALL_N", Value::from(BIT_WALL_N as i32));
    b.field("BIT_WALL_W", Value::from(BIT_WALL_W as i32));
    b.field("BIT_WALL_NW", Value::from((BIT_WALL_N | BIT_WALL_W) as i32));
    b.field("BIT_PATH_WALL_N", Value::from(BIT_PATH_WALL_N as i32));
    b.field("BIT_PATH_WALL_W", Value::from(BIT_PATH_WALL_W as i32));
    b.field("BIT_HAS_FLOOR", Value::from(BIT_HAS_FLOOR as i32));
    b.field("BIT_STAIRCASE", Value::from(BIT_STAIRCASE as i32));
    b.field("BIT_HAS_ROOF", Value::from(BIT_HAS_ROOF as i32));

    b.static_fn("getSquareFlags", |c| {
        let (x, y, z) = (int_arg(c, 0), int_arg(c, 1), int_arg(c, 2));
        Ok(c.push(square_flags_at(x, y, z) as i32))
    });
    b.static_fn("getDataChunk", |c| {
        let (cx, cy) = (int_arg(c, 0), int_arg(c, 1));
        let exists = try_with_world_read(|w| {
            Ok(w.regions
                .as_ref()
                .is_some_and(|r| r.borrow().get_data_chunk(cx, cy).is_some()))
        })
        .unwrap_or(false);
        let v = if exists {
            Value::Object(Rc::new(LuaDataChunk {
                cx,
                cy,
                selected_flags: Cell::new(0),
            }))
        } else {
            Value::Nil
        };
        Ok(c.push(v))
    });
    b.static_fn("getLogger", |c| {
        let v = LOGGER.with(|l| Value::Object(l.clone()));
        Ok(c.push(v))
    });
    b.static_fn("isDebugLoadAllChunks", |c| {
        Ok(c.push(DEBUG_LOAD_ALL.with(Cell::get)))
    });
    b.static_fn("setDebugLoadAllChunks", |c| {
        DEBUG_LOAD_ALL.with(|d| d.set(bool_arg(c, 0)));
        Ok(0)
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use pz_kahlua::stdlib::load_string;
    use pz_kahlua::TableRef;

    #[test]
    fn iso_regions_constants_logger_and_square_flags() {
        let mut t = Thread::new(TableRef::new());
        crate::bind_java_util::register(&mut t);
        register(&mut t);
        let f = load_string(
            r#"
            local logger = IsoRegions.getLogger()
            return IsoRegions.BIT_WALL_N, IsoRegions.BIT_HAS_FLOOR, IsoRegions.BIT_HAS_ROOF,
                   IsoRegions.getSquareFlags(0, 0, 0), logger:getLogs():size(), logger:isDirtyUI()
            "#,
            Some("iso_regions_test.lua"),
            t.globals().clone(),
        )
        .unwrap();
        let res = t.pcall(&f, &[]).unwrap();
        assert_eq!(res[0].as_num(), Some(1.0));
        assert_eq!(res[1].as_num(), Some(16.0));
        assert_eq!(res[2].as_num(), Some(64.0));
        assert_eq!(res[3].as_num(), Some(0.0));
        assert_eq!(res[4].as_num(), Some(0.0));
        assert!(!res[5].truthy());
    }
}
