//! Kahlua virtual machine: prototypes, closures, coroutines and the
//! `KahluaThread` interpreter loop (see `exec.rs`).
//!
//! The public contract other crates build on is [`NativeFunction`] /
//! [`NativeCall`] (the equivalent of Kahlua's `JavaFunction` and
//! `LuaCallFrame`) and [`Thread`]'s call entry points.

use crate::table::TableRef;
use crate::value::Value;
use std::cell::RefCell;
use std::fmt;
use std::rc::Rc;

/// Compiled function (`se.krka.kahlua.vm.Prototype`).
#[derive(Default, Debug)]
pub struct Prototype {
    pub code: Vec<u32>,
    pub constants: Vec<Value>,
    pub prototypes: Vec<Rc<Prototype>>,
    pub num_params: u8,
    pub is_vararg: bool,
    pub name: Option<Rc<str>>,
    pub lines: Vec<i32>,
    pub num_upvalues: u8,
    pub max_stack_size: u8,
    /// Short file name (`FuncState.currentFile`).
    pub file: Option<Rc<str>>,
    /// Full file name (`FuncState.currentfullFile`).
    pub filename: Option<Rc<str>>,
    pub locvars: Vec<Rc<str>>,
}

pub struct LuaClosure {
    pub prototype: Rc<Prototype>,
    pub env: RefCell<TableRef>,
    pub upvalues: Vec<UpValueRef>,
    pub debug_name: RefCell<Option<Rc<str>>>,
}

impl LuaClosure {
    pub fn new(prototype: Rc<Prototype>, env: TableRef) -> Rc<Self> {
        Rc::new(LuaClosure { prototype, env: RefCell::new(env), upvalues: Vec::new(), debug_name: RefCell::new(None) })
    }
    /// `LuaClosure.toString`.
    pub fn describe(&self) -> String {
        let p = &self.prototype;
        match p.lines.first() {
            Some(l) => format!("function {}:{}", p.name.as_deref().unwrap_or("null"), l),
            None => "function[?]".to_string(),
        }
    }
}

pub type UpValueRef = Rc<RefCell<UpValue>>;

pub enum UpValue {
    /// Still on a coroutine stack at this absolute index.
    Open { coroutine: CoroutineRef, index: usize },
    Closed(Value),
}

pub type CoroutineRef = Rc<RefCell<Coroutine>>;

/// Execution state of one coroutine. While a coroutine runs, its state is
/// moved into the [`Thread`]; while suspended, it lives here.
#[derive(Default)]
pub struct Coroutine {
    pub(crate) stack: Vec<Value>,
    pub(crate) top: usize,
    pub(crate) frames: Vec<CallFrame>,
    pub(crate) open_upvalues: Vec<UpValueRef>,
    pub(crate) parent: Option<CoroutineRef>,
    pub(crate) status: CoStatus,
    pub env: Option<TableRef>,
    pub stack_trace: String,
    pub(crate) started: bool,
    pub(crate) yielded: Option<Vec<Value>>,
    pub(crate) yield_return_base: Option<usize>,
    pub(crate) resume_level: usize,
}

#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
pub(crate) enum CoStatus {
    #[default]
    Suspended,
    Running,
    Normal,
    Dead,
}

#[derive(Clone)]
pub(crate) struct CallFrame {
    pub closure: Option<Rc<LuaClosure>>,
    pub native: Option<Rc<NativeFunction>>,
    pub pc: usize,
    pub local_base: usize,
    pub return_base: usize,
    pub n_arguments: usize,
    pub from_lua: bool,
    pub restore_top: bool,
}

/// A Lua error. Kahlua distinguishes `KahluaException` (carrying any Lua
/// value, raised by `error()` and compile errors) from Java runtime
/// exceptions, whose pcall message is `getMessage() + " " + className`.
#[derive(Clone, Debug)]
pub enum LuaError {
    Kahlua(Value),
    Runtime { message: String, class: &'static str },
}

impl LuaError {
    pub fn kahlua(msg: impl Into<String>) -> Self {
        LuaError::Kahlua(Value::from(msg.into()))
    }
    /// `KahluaUtil.fail` / `new RuntimeException(msg)`.
    pub fn runtime(msg: impl Into<String>) -> Self {
        LuaError::Runtime { message: msg.into(), class: "java.lang.RuntimeException" }
    }
    /// The `errorMessage` value pcall returns.
    pub fn pcall_message(&self) -> Value {
        match self {
            LuaError::Kahlua(v) => v.clone(),
            LuaError::Runtime { message, class } => Value::from(format!("{message} {class}")),
        }
    }
}

impl fmt::Display for LuaError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LuaError::Kahlua(Value::Str(s)) => f.write_str(s),
            LuaError::Kahlua(v) => write!(f, "{v:?}"),
            LuaError::Runtime { message, .. } => f.write_str(message),
        }
    }
}

pub type LuaResult<T> = Result<T, LuaError>;

/// Native callback: reads arguments from `call`, pushes results with
/// [`NativeCall::push`] and returns how many it pushed (Kahlua's
/// `JavaFunction.call` contract).
pub type NativeFn = dyn Fn(&mut NativeCall<'_>) -> LuaResult<usize>;

pub struct NativeFunction {
    pub name: Rc<str>,
    pub f: Box<NativeFn>,
}

impl NativeFunction {
    pub fn new(name: &str, f: impl Fn(&mut NativeCall<'_>) -> LuaResult<usize> + 'static) -> Rc<Self> {
        Rc::new(NativeFunction { name: Rc::from(name), f: Box::new(f) })
    }
}

/// The interpreter (`KahluaThread`).
pub struct Thread {
    /// Identity of the running coroutine (its fields are moved out).
    pub(crate) current: CoroutineRef,
    pub(crate) root: CoroutineRef,
    pub(crate) stack: Vec<Value>,
    pub(crate) top: usize,
    pub(crate) frames: Vec<CallFrame>,
    pub(crate) open_upvalues: Vec<UpValueRef>,
    pub(crate) env: TableRef,
    /// Metatables for non-table values, keyed by type: `"string"` for
    /// strings, the Java class name for engine objects (`classMetatables`).
    pub class_metatables: TableRef,
    /// Engine-wide host state reachable from native callbacks.
    pub host: Option<Box<dyn std::any::Any>>,
    /// Sink for `print` and Lua error dumps (`DebugType.Lua` log stream).
    pub out: Box<dyn FnMut(&str)>,
    /// Sink for Lua error dumps (`DebugType.General.error`).
    pub log: Box<dyn FnMut(&str)>,
    pub(crate) exec_level: usize,
}

/// View of the current native call frame (`LuaCallFrame` for a Java function).
pub struct NativeCall<'a> {
    pub thread: &'a mut Thread,
    pub(crate) base: usize,
    pub(crate) nargs: usize,
}

impl NativeCall<'_> {
    pub fn nargs(&self) -> usize {
        self.nargs
    }
    /// Argument `i` (0-based), nil when absent.
    pub fn arg(&self, i: usize) -> Value {
        if i < self.nargs {
            self.thread.stack.get(self.base + i).cloned().unwrap_or_default()
        } else {
            Value::Nil
        }
    }
    /// Push a return value; returns 1 so `return Ok(call.push(v))` mirrors
    /// Kahlua's `return callFrame.push(v)`.
    pub fn push(&mut self, v: impl Into<Value>) -> usize {
        self.thread.push_value(v.into());
        1
    }
    /// `callFrame.setTop(n)` relative to this frame's arguments.
    pub fn set_top(&mut self, n: usize) -> LuaResult<()> {
        self.thread.set_top(self.base + n)
    }
    /// `callFrame.set(i, v)`.
    pub fn set(&mut self, i: usize, v: Value) {
        self.thread.stack[self.base + i] = v;
    }
    /// `callFrame.getTop()`.
    pub fn top(&self) -> usize {
        self.thread.top - self.base
    }
}
