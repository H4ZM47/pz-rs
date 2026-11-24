//! `zombie.GameWindow`'s Lua-visible statics: the `doRenderEvent` flag the
//! frame loop reads, `isIngameState`, `DoLoadingText`, `getUpdateTime`,
//! `getCoopServerHome`, and the public static fields as they stand when
//! the class is exposed; `zombie.gameStates.MainScreenState`'s statics;
//! and `forceChangeState` with the one state Lua makes,
//! `LoadingQueueState`.

use std::any::Any;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

use pz_kahlua::java_io::{JavaReader, JavaUuid, JavaWrite};
use pz_kahlua::value::LuaObject;
use pz_kahlua::vm::Thread;
use pz_kahlua::{LuaError, LuaResult, Value};

use crate::expose::{bool_arg, expose_class, int_arg, num_arg, str_arg};

pub const GAME_WINDOW_CLASS: &str = "zombie.GameWindow";
pub const MAIN_SCREEN_STATE_CLASS: &str = "zombie.gameStates.MainScreenState";
pub const BYTE_BUFFER_CLASS: &str = "java.nio.ByteBuffer";
pub const UUID_CLASS: &str = "java.util.UUID";

/// Minimal big-endian `java.nio.ByteBuffer` for Lua `GameWindow.*` IO calls.
pub struct LuaByteBuffer {
    pub data: RefCell<Vec<u8>>,
    pub pos: Cell<usize>,
    pub limit: Cell<usize>,
}

impl LuaByteBuffer {
    pub fn from_bytes(bytes: Vec<u8>) -> Rc<Self> {
        let len = bytes.len();
        Rc::new(Self {
            data: RefCell::new(bytes),
            pos: Cell::new(0),
            limit: Cell::new(len),
        })
    }

    pub fn allocate(cap: usize) -> Rc<Self> {
        Rc::new(Self {
            data: RefCell::new(Vec::with_capacity(cap)),
            pos: Cell::new(0),
            limit: Cell::new(cap),
        })
    }

    fn read_with<T>(&self, f: impl FnOnce(&mut JavaReader<'_>) -> std::io::Result<T>) -> LuaResult<T> {
        let data = self.data.borrow();
        let lim = self.limit.get().min(data.len());
        let pos = self.pos.get().min(lim);
        let mut r = JavaReader::with_limit(&data, lim)
            .map_err(|e| LuaError::runtime(format!("ByteBuffer: {e}")))?;
        r.set_position(pos)
            .map_err(|e| LuaError::runtime(format!("ByteBuffer: {e}")))?;
        let out = f(&mut r).map_err(|e| LuaError::runtime(format!("ByteBuffer: {e}")))?;
        self.pos.set(r.position());
        Ok(out)
    }

    fn write_with(&self, f: impl FnOnce(&mut Vec<u8>)) {
        let mut data = self.data.borrow_mut();
        let pos = self.pos.get();
        if pos == data.len() {
            f(&mut data);
        } else {
            let mut tmp = Vec::new();
            f(&mut tmp);
            let end = pos + tmp.len();
            if end > data.len() {
                data.resize(end, 0);
            }
            data[pos..end].copy_from_slice(&tmp);
        }
        let new_pos = data.len().max(self.pos.get());
        self.pos.set(new_pos);
        if self.limit.get() < data.len() {
            self.limit.set(data.len());
        }
    }
}

impl LuaObject for LuaByteBuffer {
    fn class_name(&self) -> &str {
        BYTE_BUFFER_CLASS
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// `java.util.UUID` for `GameWindow.ReadUUID`/`WriteUUID`.
pub struct LuaUuid(pub JavaUuid);

impl LuaUuid {
    pub fn format_hyphenated(&self) -> String {
        let mut b = [0u8; 16];
        b[..8].copy_from_slice(&self.0.msb.to_be_bytes());
        b[8..].copy_from_slice(&self.0.lsb.to_be_bytes());
        format!(
            "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
            b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7],
            b[8], b[9], b[10], b[11], b[12], b[13], b[14], b[15]
        )
    }

    pub fn parse_hyphenated(s: &str) -> Option<JavaUuid> {
        let hex: String = s.chars().filter(|&c| c != '-').collect();
        if hex.len() != 32 {
            return None;
        }
        let mut b = [0u8; 16];
        for i in 0..16 {
            b[i] = u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16).ok()?;
        }
        Some(JavaUuid {
            msb: i64::from_be_bytes(b[..8].try_into().ok()?),
            lsb: i64::from_be_bytes(b[8..].try_into().ok()?),
        })
    }
}

impl LuaObject for LuaUuid {
    fn class_name(&self) -> &str {
        UUID_CLASS
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

fn with_byte_buffer<R>(v: &Value, f: impl FnOnce(&LuaByteBuffer) -> LuaResult<R>) -> LuaResult<R> {
    match v {
        Value::Object(o) => {
            let b = o
                .as_any()
                .downcast_ref::<LuaByteBuffer>()
                .ok_or_else(|| LuaError::runtime("Expected java.nio.ByteBuffer"))?;
            f(b)
        }
        _ => Err(LuaError::runtime("Expected java.nio.ByteBuffer")),
    }
}

fn uuid_of(v: &Value) -> LuaResult<JavaUuid> {
    match v {
        Value::Object(o) => o
            .as_any()
            .downcast_ref::<LuaUuid>()
            .map(|u| u.0)
            .ok_or_else(|| LuaError::runtime("Expected java.util.UUID")),
        Value::Str(s) => LuaUuid::parse_hyphenated(s)
            .ok_or_else(|| LuaError::runtime("Invalid UUID string")),
        _ => Err(LuaError::runtime("Expected java.util.UUID")),
    }
}

/// The Lua handle of `MainScreenState.instance`.
pub struct MainScreenStateObject;

impl pz_kahlua::value::LuaObject for MainScreenStateObject {
    fn class_name(&self) -> &str {
        MAIN_SCREEN_STATE_CLASS
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

pub const LOADING_QUEUE_STATE_CLASS: &str = "zombie.gameStates.LoadingQueueState";

/// A `LoadingQueueState` Lua made (`LoadingQueueState.new()`).
pub struct LoadingQueueStateObject;

impl pz_kahlua::value::LuaObject for LoadingQueueStateObject {
    fn class_name(&self) -> &str {
        LOADING_QUEUE_STATE_CLASS
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

/// A state `forceChangeState` asked for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ForcedState {
    LoadingQueue,
}

thread_local! {
    /// `GameWindow.states.forceNext`, as Lua last set it.
    static FORCE_NEXT: Cell<Option<Option<ForcedState>>> = const { Cell::new(None) };
    static DO_RENDER_EVENT: Cell<bool> = const { Cell::new(false) };
    static UPDATE_TIME: Cell<i64> = const { Cell::new(0) };
    static MAIN_SCREEN: std::cell::RefCell<Option<std::rc::Rc<MainScreenStateObject>>> = const { std::cell::RefCell::new(None) };
}

/// `MainScreenState.instance = this` (the end of `enter`, after
/// `OnMainMenuEnter`); the original never clears it.
pub fn set_main_screen_instance() {
    MAIN_SCREEN.with(|m| {
        m.borrow_mut().get_or_insert_with(|| std::rc::Rc::new(MainScreenStateObject));
    });
}

/// What `forceChangeState(state)` set since the last call:
/// `Some(Some(s))` for a state, `Some(None)` for `null` (clears the
/// pending state), `None` when Lua did not call it.
pub fn take_forced_state() -> Option<Option<ForcedState>> {
    FORCE_NEXT.with(|f| f.take())
}

/// `GameWindow.doRenderEvent`: whether `GameWindow.render` fires
/// `OnRenderTick` after the frame.
pub fn render_event_enabled() -> bool {
    DO_RENDER_EVENT.with(Cell::get)
}

/// `GameWindow.updateTime` (the logic step's length, as `getUpdateTime`
/// returns it).
pub fn set_update_time(t: i64) {
    UPDATE_TIME.with(|u| u.set(t));
}

thread_local! {
    static SAVE_REQUESTED: Cell<bool> = const { Cell::new(false) };
}

/// `GameWindow.save(doCharacter)` from Lua (`save`) or
/// `SleepingEvent.wakeUp`. The save needs the live world, which the
/// running update holds, so the engine runs it when that update ends
/// ([`take_save_request`]).
pub(crate) fn request_save() {
    SAVE_REQUESTED.with(|s| s.set(true));
}

/// Whether a `GameWindow.save` was asked for since the last call.
pub fn take_save_request() -> bool {
    SAVE_REQUESTED.with(|s| s.replace(false))
}

/// `GameWindow.version`, the static initializer's string in the jar.
fn version_from_jar(jar: &pz_classfile::Jar) -> Option<String> {
    let cf = pz_classfile::load_class(jar, "zombie/GameWindow.class").ok()?;
    pz_classfile::static_init::static_inits(&cf)
        .ok()?
        .into_iter()
        .find(|i| i.class == cf.this_class && i.field == "version")
        .and_then(|i| i.value.as_str().map(str::to_string))
}

pub fn register(thread: &mut Thread, jar: Option<&pz_classfile::Jar>, cache_dir: &std::path::Path) {
    let b = expose_class(thread, GAME_WINDOW_CLASS, None);
    b.static_fn("doRenderEvent", |c| {
        let v = bool_arg(c, 0);
        DO_RENDER_EVENT.with(|d| d.set(v));
        Ok(0)
    });
    b.static_fn("isIngameState", |c| Ok(c.push(crate::bind_misc::is_ingame_state())));
    // `DoLoadingText(text)`: `DebugLog.log(text)`.
    b.static_fn("DoLoadingText", |c| {
        let s = str_arg(c, 0).map(|s| s.to_string()).unwrap_or_else(|| "null".into());
        pz_core::debug::log(&s);
        Ok(0)
    });
    b.static_fn("getUpdateTime", |c| Ok(c.push(UPDATE_TIME.with(Cell::get) as f64)));
    // `new File(getCacheDir()).getParent()`.
    let parent = cache_dir.parent().map(|p| p.to_string_lossy().into_owned());
    b.static_fn("getCoopServerHome", move |c| Ok(c.push(parent.as_deref().map(pz_kahlua::value::Value::str).unwrap_or_default())));
    b.static_fn("save", |_c| {
        request_save();
        Ok(0)
    });
    b.static_fn("getEncodedBytesUTF", |c| {
        let bytes = str_arg(c, 0)
            .map(|s| s.as_bytes().to_vec())
            .unwrap_or_default();
        Ok(c.push(Value::Object(LuaByteBuffer::from_bytes(bytes))))
    });
    b.static_fn("WriteString", |c| {
        let s = str_arg(c, 1);
        with_byte_buffer(&c.arg(0), |buf| {
            buf.write_with(|out| out.write_string_utf(s.as_deref()));
            Ok(())
        })?;
        Ok(0)
    });
    b.static_fn("ReadString", |c| {
        let s = with_byte_buffer(&c.arg(0), |buf| buf.read_with(|r| r.read_string_utf()))?;
        Ok(c.push(Value::str(&s)))
    });
    b.static_fn("WriteUUID", |c| {
        let id = uuid_of(&c.arg(1))?;
        with_byte_buffer(&c.arg(0), |buf| {
            buf.write_with(|out| out.write_uuid(id));
            Ok(())
        })?;
        Ok(0)
    });
    b.static_fn("ReadUUID", |c| {
        let id = with_byte_buffer(&c.arg(0), |buf| buf.read_with(|r| r.read_uuid()))?;
        Ok(c.push(Value::Object(Rc::new(LuaUuid(id)))))
    });
    b.static_fn("readInt", |c| {
        let v = with_byte_buffer(&c.arg(0), |buf| buf.read_with(|r| r.get_i32()))?;
        Ok(c.push(v as f64))
    });
    b.static_fn("readLong", |c| {
        let v = with_byte_buffer(&c.arg(0), |buf| buf.read_with(|r| r.get_i64()))?;
        Ok(c.push(v as f64))
    });
    // Expose `java.nio.ByteBuffer` and `java.util.UUID` helpers used alongside
    // `GameWindow.ReadString`/`WriteString`/`ReadUUID`/`WriteUUID`/`getEncodedBytesUTF`.
    let bb = expose_class(thread, BYTE_BUFFER_CLASS, Some("java.lang.Object"));
    bb.static_fn("allocate", |c| {
        let cap = int_arg(c, 0).max(0) as usize;
        Ok(c.push(Value::Object(LuaByteBuffer::allocate(cap))))
    });
    bb.method("position", |c| {
        let p = with_byte_buffer(&c.arg(0), |buf| {
            if c.nargs() > 1 {
                let np = int_arg(c, 1).max(0) as usize;
                buf.pos.set(np.min(buf.limit.get()));
                return Ok(None);
            }
            Ok(Some(buf.pos.get()))
        })?;
        match p {
            Some(pos) => Ok(c.push(pos as f64)),
            None => Ok(c.push(c.arg(0))),
        }
    });
    bb.method("limit", |c| {
        let l = with_byte_buffer(&c.arg(0), |buf| Ok(buf.limit.get()))?;
        Ok(c.push(l as f64))
    });
    bb.method("remaining", |c| {
        let rem = with_byte_buffer(&c.arg(0), |buf| {
            Ok(buf.limit.get().saturating_sub(buf.pos.get()))
        })?;
        Ok(c.push(rem as f64))
    });
    bb.method("flip", |c| {
        with_byte_buffer(&c.arg(0), |buf| {
            buf.limit.set(buf.pos.get());
            buf.pos.set(0);
            Ok(())
        })?;
        Ok(c.push(c.arg(0)))
    });
    bb.method("rewind", |c| {
        with_byte_buffer(&c.arg(0), |buf| {
            buf.pos.set(0);
            Ok(())
        })?;
        Ok(c.push(c.arg(0)))
    });
    bb.method("clear", |c| {
        with_byte_buffer(&c.arg(0), |buf| {
            buf.data.borrow_mut().clear();
            buf.pos.set(0);
            buf.limit.set(0);
            Ok(())
        })?;
        Ok(c.push(c.arg(0)))
    });
    bb.method("putInt", |c| {
        let v = int_arg(c, 1);
        with_byte_buffer(&c.arg(0), |buf| {
            buf.write_with(|out| out.put_i32(v));
            Ok(())
        })?;
        Ok(c.push(c.arg(0)))
    });
    bb.method("putLong", |c| {
        let v = num_arg(c, 1) as i64;
        with_byte_buffer(&c.arg(0), |buf| {
            buf.write_with(|out| out.put_i64(v));
            Ok(())
        })?;
        Ok(c.push(c.arg(0)))
    });
    bb.method("getInt", |c| {
        let v = with_byte_buffer(&c.arg(0), |buf| buf.read_with(|r| r.get_i32()))?;
        Ok(c.push(v as f64))
    });
    bb.method("getLong", |c| {
        let v = with_byte_buffer(&c.arg(0), |buf| buf.read_with(|r| r.get_i64()))?;
        Ok(c.push(v as f64))
    });

    let uuid = expose_class(thread, UUID_CLASS, Some("java.lang.Object"));
    uuid.static_fn("fromString", |c| {
        let s = str_arg(c, 0).unwrap_or_default();
        let id = LuaUuid::parse_hyphenated(&s)
            .ok_or_else(|| LuaError::runtime(format!("Invalid UUID string: {s}")))?;
        Ok(c.push(Value::Object(Rc::new(LuaUuid(id)))))
    });
    uuid.static_fn("randomUUID", |c| {
        let mut bytes = [0u8; 16];
        for b in &mut bytes {
            *b = pz_core::random::next(256) as u8;
        }
        Ok(c.push(Value::Object(Rc::new(LuaUuid(
            JavaUuid::from_random_bytes(bytes),
        )))))
    });
    uuid.method("toString", |c| {
        let id = uuid_of(&c.arg(0))?;
        Ok(c.push(Value::str(&LuaUuid(id).format_hyphenated())))
    });
    uuid.method("getMostSignificantBits", |c| {
        let id = uuid_of(&c.arg(0))?;
        Ok(c.push(id.msb as f64))
    });
    uuid.method("getLeastSignificantBits", |c| {
        let id = uuid_of(&c.arg(0))?;
        Ok(c.push(id.lsb as f64))
    });
    // TODO(port): render, LoadTexturePack, setTexturePackLookup, initFonts,
    // InitDisplay, InitGameThread, doEpilepsyWarningText, uncaughtException,
    // and the object fields (states, GameInput, fileSystem, assetManagers,
    // texturePacks, texturePackTextures, averageFPS).
    if let Some(v) = jar.and_then(version_from_jar) {
        b.field("version", pz_kahlua::value::Value::str(&v));
    }
    for name in [
        "DEBUG_SAVE",
        "okToSaveOnExit",
        "serverDisconnected",
        "loadedAsClient",
        "drawReloadingLua",
        "closeRequested",
        "luaDebuggerKeyDown",
        "gameThreadExited",
    ] {
        b.field(name, false);
    }
    let m = expose_class(thread, MAIN_SCREEN_STATE_CLASS, None);
    m.field("totalScale", 1.0f64);
    m.static_fn("getInstance", |c| {
        let v = MAIN_SCREEN.with(|m| m.borrow().clone());
        Ok(c.push(v.map(|o| pz_kahlua::value::Value::Object(o)).unwrap_or_default()))
    });
    m.static_fn("DrawTexture", |_c| Ok(0));
    m.static_fn("preloadBackgroundTextures", |_c| Ok(0));
    // TODO(port): loadIcons.
    if let Some(cf) = jar.and_then(|j| pz_classfile::load_class(j, "zombie/gameStates/MainScreenState.class").ok()) {
        for (name, v) in cf.static_final_strings() {
            if name == "VERSION" {
                m.field(name, pz_kahlua::value::Value::str(v));
            }
        }
    }
    let q = expose_class(thread, LOADING_QUEUE_STATE_CLASS, None);
    q.static_fn("new", |c| {
        Ok(c.push(pz_kahlua::value::Value::Object(std::rc::Rc::new(LoadingQueueStateObject))))
    });
    // `forceChangeState(state)`: `GameWindow.states.forceNextState(state)`.
    crate::expose::global(thread, "forceChangeState", |c| {
        let s = match c.arg(0) {
            pz_kahlua::value::Value::Nil => None,
            pz_kahlua::value::Value::Object(o) if o.as_any().is::<LoadingQueueStateObject>() => {
                Some(ForcedState::LoadingQueue)
            }
            v => {
                return Err(pz_kahlua::LuaError::runtime(format!(
                    "forceChangeState: {} is not a GameState",
                    pz_kahlua::exec::java_value_of(&v)
                )))
            }
        };
        FORCE_NEXT.with(|f| f.set(Some(s)));
        Ok(0)
    });
    // `isSteamRunningOnSteamDeck()`: false outside Steam mode, and there is
    // no Steam integration.
    crate::expose::global(thread, "isSteamRunningOnSteamDeck", |c| Ok(c.push(false)));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn do_render_event_sets_the_flag() {
        let mut t = Thread::new(pz_kahlua::TableRef::new());
        pz_kahlua::stdlib::install(&mut t, None).unwrap();
        register(&mut t, None, std::path::Path::new("/tmp/x/Zomboid"));
        let f = pz_kahlua::stdlib::load_string(
            "GameWindow.doRenderEvent(true); return GameWindow.getCoopServerHome(), GameWindow.okToSaveOnExit",
            Some("t"),
            t.globals().clone(),
        )
        .unwrap();
        let r = t.pcall(&f, &[]).map_err(|(e, tr)| format!("{e:?}\n{tr}")).unwrap();
        assert_eq!(format!("{r:?}"), format!("{:?}", [pz_kahlua::value::Value::str("/tmp/x"), pz_kahlua::value::Value::Bool(false)]));
        assert!(render_event_enabled());
    }

    #[test]
    fn force_change_state_records_the_state() {
        let mut t = Thread::new(pz_kahlua::TableRef::new());
        pz_kahlua::stdlib::install(&mut t, None).unwrap();
        register(&mut t, None, std::path::Path::new("/tmp/x/Zomboid"));
        let run = |t: &mut Thread, src: &str| {
            let f = pz_kahlua::stdlib::load_string(src, Some("t"), t.globals().clone()).unwrap();
            t.pcall(&f, &[]).map_err(|(e, _)| format!("{e:?}"))
        };
        assert_eq!(take_forced_state(), None);
        run(&mut t, "forceChangeState(LoadingQueueState.new())").unwrap();
        assert_eq!(take_forced_state(), Some(Some(ForcedState::LoadingQueue)));
        assert_eq!(take_forced_state(), None);
        run(&mut t, "forceChangeState(nil)").unwrap();
        assert_eq!(take_forced_state(), Some(None));
        assert!(run(&mut t, "forceChangeState({})").is_err());
        assert_eq!(take_forced_state(), None);
    }

    #[test]
    fn game_window_io_and_main_screen_state_lua() {
        let mut t = Thread::new(pz_kahlua::TableRef::new());
        pz_kahlua::stdlib::install(&mut t, None).unwrap();
        register(&mut t, None, std::path::Path::new("/tmp/x/Zomboid"));
        let src = r#"
            local enc = GameWindow.getEncodedBytesUTF("hello")
            local rem = enc:remaining()

            local buf = ByteBuffer.allocate(64)
            GameWindow.WriteString(buf, "Zomboid 42.22")
            local id = UUID.fromString("12345678-9abc-def0-1234-56789abcdef0")
            GameWindow.WriteUUID(buf, id)
            buf:putInt(4242)
            buf:putLong(987654321)
            buf:flip()

            local s = GameWindow.ReadString(buf)
            local id2 = GameWindow.ReadUUID(buf)
            local iv = GameWindow.readInt(buf)
            local lv = GameWindow.readLong(buf)
            GameWindow.save(true)
            MainScreenState.preloadBackgroundTextures()
            return rem, s, id2:toString(), iv, lv, MainScreenState.totalScale
        "#;
        let f = pz_kahlua::stdlib::load_string(src, Some("gw_io"), t.globals().clone()).unwrap();
        let r = t.pcall(&f, &[]).map_err(|(e, tr)| format!("{e:?}\n{tr}")).unwrap();
        assert_eq!(pz_kahlua::exec::raw_tonumber(&r[0]), Some(5.0));
        assert_eq!(pz_kahlua::exec::java_value_of(&r[1]), "Zomboid 42.22");
        assert_eq!(
            pz_kahlua::exec::java_value_of(&r[2]),
            "12345678-9abc-def0-1234-56789abcdef0"
        );
        assert_eq!(pz_kahlua::exec::raw_tonumber(&r[3]), Some(4242.0));
        assert_eq!(pz_kahlua::exec::raw_tonumber(&r[4]), Some(987654321.0));
        assert_eq!(pz_kahlua::exec::raw_tonumber(&r[5]), Some(1.0));
        assert!(take_save_request());
    }
}
