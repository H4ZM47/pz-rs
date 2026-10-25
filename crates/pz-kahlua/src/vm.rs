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
        Self::with_upvalues(prototype, env, Vec::new())
    }

    pub(crate) fn with_upvalues(prototype: Rc<Prototype>, env: TableRef, upvalues: Vec<UpValueRef>) -> Rc<Self> {
        let closure = Rc::new(LuaClosure { prototype, env: RefCell::new(env), upvalues, debug_name: RefCell::new(None) });
        crate::gc::register_closure(Rc::downgrade(&closure));
        closure
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

impl UpValue {
    /// A shared closed slot participating in conservative cycle collection.
    /// Raw `Rc<RefCell<UpValue>>` construction remains supported, but its
    /// outgoing references are opaque roots rather than traced slots.
    pub fn closed(value: Value) -> UpValueRef {
        Self::allocate(Self::Closed(value))
    }

    pub(crate) fn open(coroutine: CoroutineRef, index: usize) -> UpValueRef {
        Self::allocate(Self::Open { coroutine, index })
    }

    fn allocate(value: Self) -> UpValueRef {
        let upvalue = Rc::new(RefCell::new(value));
        crate::gc::register_upvalue(Rc::downgrade(&upvalue));
        upvalue
    }
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
        let native = Rc::new(NativeFunction { name: Rc::from(name), f: Box::new(f) });
        crate::gc::register_native(Rc::downgrade(&native));
        native
    }

    /// A reviewed explicit capture, held strongly by the runtime callable.
    /// The capture-free function pointer cannot conceal additional Fn state;
    /// collection traces its single backing Rc and that payload's own slots.
    /// Ordinary `new` callbacks remain opaque and conservatively retained.
    pub fn new_traced_capture<C: crate::gc::TracePayload>(
        name: &str,
        capture: C,
        invoke: fn(&RefCell<C>, &mut NativeCall<'_>) -> LuaResult<usize>,
    ) -> Rc<Self> {
        let capture = crate::gc::traceable(capture);
        let weak_capture = Rc::downgrade(&capture);
        let native = Rc::new(NativeFunction {
            name: Rc::from(name),
            f: Box::new(move |call| invoke(&capture, call)),
        });
        crate::gc::register_traced_native(Rc::downgrade(&native), weak_capture);
        native
    }
}

pub type StrHook = Box<dyn FnMut(&str)>;

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
    /// Development aid: called with the name of every global read that
    /// resolves to nil.
    pub on_missing_global: Option<StrHook>,
    /// Development aid: called with `(class, key)` when indexing an engine
    /// object yields nil (typically an unported Java method).
    pub on_missing_member: Option<Box<dyn FnMut(&str, &str)>>,
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

impl Prototype {
    /// `Prototype.dump`: Lua 5.1 bytecode in the original's exact layout
    /// (big-endian, modified-UTF-8 strings, no debug locals).
    pub fn dump(&self) -> Vec<u8> {
        let mut out = vec![27, 76, 117, 97, 81, 0, 0, 4, 4, 4, 8, 0];
        self.dump_into(&mut out);
        out
    }

    fn dump_into(&self, o: &mut Vec<u8>) {
        dump_string(self.name.as_deref(), o);
        o.extend_from_slice(&0i32.to_be_bytes());
        o.extend_from_slice(&0i32.to_be_bytes());
        o.push(self.num_upvalues);
        o.push(self.num_params);
        o.push(if self.is_vararg { 2 } else { 0 });
        o.push(self.max_stack_size);
        o.extend_from_slice(&(self.code.len() as i32).to_be_bytes());
        for c in &self.code {
            o.extend_from_slice(&c.to_be_bytes());
        }
        o.extend_from_slice(&(self.constants.len() as i32).to_be_bytes());
        for k in &self.constants {
            match k {
                Value::Nil => o.push(0),
                Value::Bool(b) => {
                    o.push(1);
                    o.push(*b as u8);
                }
                Value::Num(n) => {
                    o.push(3);
                    // Double.doubleToLongBits canonicalises NaN.
                    let bits = if n.is_nan() { 0x7ff8_0000_0000_0000u64 } else { n.to_bits() };
                    o.extend_from_slice(&bits.to_be_bytes());
                }
                Value::Str(s) => {
                    o.push(4);
                    dump_string(Some(s), o);
                }
                _ => panic!("Bad type in constant pool"),
            }
        }
        o.extend_from_slice(&(self.prototypes.len() as i32).to_be_bytes());
        for p in &self.prototypes {
            p.dump_into(o);
        }
        o.extend_from_slice(&(self.lines.len() as i32).to_be_bytes());
        for l in &self.lines {
            o.extend_from_slice(&l.to_be_bytes());
        }
        o.extend_from_slice(&0i32.to_be_bytes());
        o.extend_from_slice(&0i32.to_be_bytes());
    }
}

fn dump_string(s: Option<&str>, o: &mut Vec<u8>) {
    let Some(s) = s else {
        o.extend_from_slice(&0i16.to_be_bytes());
        return;
    };
    let bytes = modified_utf8(s);
    o.extend_from_slice(&(bytes.len() as i32 + 1).to_be_bytes());
    o.extend_from_slice(&bytes);
    o.push(0);
}

/// Java modified UTF-8 (`DataOutputStream.writeUTF` body).
pub fn modified_utf8(s: &str) -> Vec<u8> {
    let mut out = Vec::with_capacity(s.len());
    for c in s.encode_utf16() {
        match c {
            0x0001..=0x007F => out.push(c as u8),
            0x0000 | 0x0080..=0x07FF => {
                out.push(0xC0 | ((c >> 6) & 0x1F) as u8);
                out.push(0x80 | (c & 0x3F) as u8);
            }
            _ => {
                out.push(0xE0 | ((c >> 12) & 0x0F) as u8);
                out.push(0x80 | ((c >> 6) & 0x3F) as u8);
                out.push(0x80 | (c & 0x3F) as u8);
            }
        }
    }
    out
}
