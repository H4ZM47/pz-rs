//! `KahluaThread`: the interpreter loop, calls, metatables and coroutines.

use crate::compiler::*;
use crate::number::{java_double_to_string, number_to_string};
use crate::table::TableRef;
use crate::value::Value;
use crate::vm::*;
use std::cell::RefCell;
use std::rc::Rc;

const MAX_STACK_SIZE: usize = 3000;
const MAX_CALL_FRAMES: usize = 1000;
const MAX_INDEX_RECURSION: usize = 100;
const FIELDS_PER_FLUSH: usize = 50;

fn meta_op_name(op: u32) -> &'static str {
    match op {
        OP_ADD => "__add",
        OP_SUB => "__sub",
        OP_MUL => "__mul",
        OP_DIV => "__div",
        OP_MOD => "__mod",
        OP_POW => "__pow",
        OP_EQ => "__eq",
        OP_LT => "__lt",
        OP_LE => "__le",
        _ => "",
    }
}

#[inline]
fn a8(op: u32) -> usize {
    ((op >> 6) & 0xFF) as usize
}
#[inline]
fn b9(op: u32) -> usize {
    ((op >> 23) & 0x1FF) as usize
}
#[inline]
fn c9(op: u32) -> usize {
    ((op >> 14) & 0x1FF) as usize
}
#[inline]
fn bx(op: u32) -> usize {
    (op >> 14) as usize
}
#[inline]
fn sbx(op: u32) -> isize {
    (op >> 14) as isize - 131071
}

/// `KahluaUtil.rawTonumber`: numbers, and strings parsed by `tonumber`.
pub fn raw_tonumber(v: &Value) -> Option<f64> {
    match v {
        Value::Num(n) => Some(*n),
        Value::Str(s) => crate::number::kahlua_tonumber(s, 10).ok().flatten(),
        _ => None,
    }
}

/// `KahluaUtil.rawTostring`: strings and numbers only.
pub fn raw_tostring(v: &Value) -> Option<Rc<str>> {
    match v {
        Value::Str(s) => Some(s.clone()),
        Value::Num(n) => Some(Rc::from(number_to_string(*n))),
        _ => None,
    }
}

/// Java `String.valueOf(o)` of a Lua value, as used in error messages.
pub fn java_value_of(v: &Value) -> String {
    match v {
        Value::Nil => "null".into(),
        Value::Bool(b) => b.to_string(),
        Value::Num(n) => java_double_to_string(*n),
        Value::Str(s) => s.to_string(),
        Value::Table(t) => {
            // KahluaTableImpl.toString
            match t.get_str("Type") {
                Value::Str(ty) => format!("{} 0x{}", ty, identity_hash(v)),
                _ => format!("table 0x{}", identity_hash(v)),
            }
        }
        Value::Closure(c) => c.describe(),
        Value::Native(f) => f.name.to_string(),
        Value::Coroutine(_) => format!("se.krka.kahlua.vm.Coroutine@{:x}", identity_hash(v)),
        Value::Object(o) => o.to_lua_string(),
    }
}

/// Stand-in for `System.identityHashCode` (decimal, as Java prints it).
pub fn identity_hash(v: &Value) -> u32 {
    let p = v.ptr_id() as u64;
    ((p >> 4) ^ (p >> 20)) as u32 & 0x7FFF_FFFF
}

/// UTF-16 lexicographic comparison (`String.compareTo`).
pub fn java_compare(a: &str, b: &str) -> std::cmp::Ordering {
    if a.is_ascii() && b.is_ascii() {
        return a.cmp(b);
    }
    a.encode_utf16().cmp(b.encode_utf16())
}

/// Java `String.length()`.
pub fn java_len(s: &str) -> usize {
    if s.is_ascii() {
        s.len()
    } else {
        s.encode_utf16().count()
    }
}

enum Flow {
    Continue,
    Return,
}

impl Thread {
    /// A thread over `env` (the global table). Install the standard library
    /// with [`crate::stdlib::install`].
    pub fn new(env: TableRef) -> Self {
        let root = Rc::new(RefCell::new(Coroutine { env: Some(env.clone()), status: CoStatus::Running, ..Default::default() }));
        Thread {
            current: root.clone(),
            root,
            stack: vec![Value::Nil; 64],
            top: 0,
            frames: Vec::with_capacity(64),
            open_upvalues: Vec::new(),
            env,
            class_metatables: TableRef::new(),
            host: None,
            out: Box::new(|s| println!("{s}")),
            log: Box::new(|s| eprintln!("{s}")),
            exec_level: 0,
            on_missing_global: None,
            on_missing_member: None,
        }
    }

    pub fn globals(&self) -> &TableRef {
        &self.env
    }

    /// Environment of the running coroutine (`callFrame.getEnvironment()`
    /// for a Java function).
    pub fn current_env(&self) -> TableRef {
        self.current.borrow().env.clone().unwrap_or_else(|| self.env.clone())
    }

    // ---- stack ----

    pub(crate) fn set_top(&mut self, new_top: usize) -> LuaResult<()> {
        if new_top > self.top {
            if new_top > MAX_STACK_SIZE {
                return Err(LuaError::runtime("Stack overflow"));
            }
            if self.stack.len() < new_top {
                self.stack.resize(new_top.max(self.stack.len() * 2), Value::Nil);
            }
        } else {
            for v in &mut self.stack[new_top..self.top] {
                *v = Value::Nil;
            }
        }
        self.top = new_top;
        Ok(())
    }

    pub(crate) fn push_value(&mut self, v: Value) {
        let t = self.top;
        if self.set_top(t + 1).is_ok() {
            self.stack[t] = v;
        }
    }

    // ---- calls ----

    /// Call `f` with `args`, returning all results (`KahluaThread.call`).
    pub fn call(&mut self, f: &Value, args: &[Value]) -> LuaResult<Vec<Value>> {
        let base = self.top;
        self.set_top(base + 1 + args.len())?;
        self.stack[base] = f.clone();
        for (i, a) in args.iter().enumerate() {
            self.stack[base + 1 + i] = a.clone();
        }
        let n = self.call_at(base, args.len())?;
        let out = self.stack[base..base + n].to_vec();
        self.set_top(base)?;
        Ok(out)
    }

    /// Call returning the first result (`call(fun, a, b, c)`).
    pub fn call1(&mut self, f: &Value, args: &[Value]) -> LuaResult<Value> {
        Ok(self.call(f, args)?.into_iter().next().unwrap_or_default())
    }

    /// `KahluaThread.pcall`: `Ok(results)` or `Err((message, stacktrace))`.
    pub fn pcall(&mut self, f: &Value, args: &[Value]) -> Result<Vec<Value>, (Value, String)> {
        self.pcall_ex(f, args).map_err(|(m, t, _)| (m, t))
    }

    /// `pcall` that also returns the error itself (the Java exception
    /// object the original passes as pcall's fourth result).
    pub fn pcall_ex(&mut self, f: &Value, args: &[Value]) -> Result<Vec<Value>, (Value, String, LuaError)> {
        let depth = self.frames.len();
        let base = self.top;
        let upv_floor = self.frames.last().map(|f| f.local_base).unwrap_or(0);
        match self.call(f, args) {
            Ok(v) => Ok(v),
            Err(e) => {
                self.close_upvalues(upv_floor);
                let mut trace = std::mem::take(&mut self.current.borrow_mut().stack_trace);
                while self.frames.len() > depth {
                    let fr = self.frames.pop().unwrap();
                    trace.push_str(&frame_trace(&fr));
                }
                let _ = self.set_top(base);
                Err((e.pcall_message(), trace, e))
            }
        }
    }

    /// Call the function at `stack[base]` with `nargs` arguments above it.
    /// Results are left at `stack[base..base+n]`; returns `n`.
    pub(crate) fn call_at(&mut self, base: usize, nargs: usize) -> LuaResult<usize> {
        let f = self.stack[base].clone();
        match f {
            // KahluaThread.call(int) wraps exceptions from a directly called
            // Java function as "<class> <message> in <function>".
            Value::Native(nf) => self.call_native(&nf, base + 1, base, nargs).map_err(|e| {
                let (class, msg) = match &e {
                    LuaError::Kahlua(v) => ("se.krka.kahlua.vm.KahluaException", java_value_of(v)),
                    LuaError::Runtime { message, class } => (*class, message.clone()),
                };
                LuaError::runtime(format!("{class} {msg} in {}", nf.name))
            }),
            Value::Closure(c) => {
                self.push_frame(c, base + 1, base, nargs, false)?;
                self.execute()?;
                Ok(self.top - base)
            }
            Value::Nil => Err(LuaError::runtime("tried to call nil")),
            _ => {
                // __call through the metatable, with the object as first arg.
                let mf = self.get_meta_op(&f, "__call");
                if mf.is_function() {
                    self.set_top(base + nargs + 1)?;
                    self.stack.copy_within_values(base, base + 1, nargs + 1);
                    self.stack[base] = mf;
                    self.call_at(base, nargs + 1)
                } else {
                    Err(LuaError::runtime("tried to call a non-function"))
                }
            }
        }
    }

    fn call_native(&mut self, f: &Rc<NativeFunction>, local_base: usize, return_base: usize, nargs: usize) -> LuaResult<usize> {
        if self.frames.len() >= MAX_CALL_FRAMES {
            return Err(LuaError::runtime("Stack overflow"));
        }
        self.frames.push(CallFrame {
            closure: None,
            native: Some(f.clone()),
            pc: 0,
            local_base,
            return_base,
            n_arguments: nargs,
            from_lua: false,
            restore_top: false,
        });
        let r = {
            let mut call = NativeCall { thread: self, base: local_base, nargs };
            (f.f)(&mut call)
        };
        let n = match r {
            Ok(n) => n,
            Err(e) => {
                // The native frame stays for the error handler to unwind.
                return Err(e);
            }
        };
        let top = self.top;
        let actual = top.saturating_sub(n);
        for i in 0..n {
            self.stack[return_base + i] = self.stack[actual + i].clone();
        }
        self.set_top(return_base + n)?;
        self.frames.pop();
        Ok(n)
    }

    fn push_frame(&mut self, c: Rc<LuaClosure>, local_base: usize, return_base: usize, nargs: usize, from_lua: bool) -> LuaResult<()> {
        if self.frames.len() >= MAX_CALL_FRAMES {
            return Err(LuaError::runtime("Stack overflow"));
        }
        let p = c.prototype.clone();
        let mut frame = CallFrame {
            closure: Some(c),
            native: None,
            pc: 0,
            local_base,
            return_base,
            n_arguments: nargs,
            from_lua,
            restore_top: false,
        };
        // LuaCallFrame.init
        let max = p.max_stack_size as usize;
        if p.is_vararg {
            frame.local_base += nargs;
            self.set_top(frame.local_base + max)?;
            let to_copy = nargs.min(p.num_params as usize);
            for i in 0..to_copy {
                self.stack[frame.local_base + i] = self.stack[frame.local_base - nargs + i].clone();
            }
        } else {
            self.set_top(local_base + max)?;
            // stackClear(numParams, nArguments)
            let np = p.num_params as usize;
            for i in np..=nargs {
                if local_base + i < self.stack.len() {
                    self.stack[local_base + i] = Value::Nil;
                }
            }
        }
        self.frames.push(frame);
        Ok(())
    }

    // ---- upvalues ----

    fn find_upvalue(&mut self, index: usize) -> UpValueRef {
        let mut i = self.open_upvalues.len();
        while i > 0 {
            let uv = &self.open_upvalues[i - 1];
            let idx = match &*uv.borrow() {
                UpValue::Open { index, .. } => *index,
                UpValue::Closed(_) => usize::MAX,
            };
            if idx == index {
                return uv.clone();
            }
            if idx < index {
                break;
            }
            i -= 1;
        }
        let uv = Rc::new(RefCell::new(UpValue::Open { coroutine: self.current.clone(), index }));
        self.open_upvalues.insert(i, uv.clone());
        uv
    }

    pub(crate) fn close_upvalues(&mut self, close_index: usize) {
        while let Some(uv) = self.open_upvalues.last() {
            let idx = match &*uv.borrow() {
                UpValue::Open { index, .. } => *index,
                UpValue::Closed(_) => 0,
            };
            if idx < close_index {
                return;
            }
            let uv = self.open_upvalues.pop().unwrap();
            let v = self.stack.get(idx).cloned().unwrap_or_default();
            *uv.borrow_mut() = UpValue::Closed(v);
        }
    }

    fn upvalue_get(&self, uv: &UpValueRef) -> Value {
        match &*uv.borrow() {
            UpValue::Closed(v) => v.clone(),
            UpValue::Open { coroutine, index } => {
                if Rc::ptr_eq(coroutine, &self.current) {
                    self.stack[*index].clone()
                } else {
                    coroutine.borrow().stack.get(*index).cloned().unwrap_or_default()
                }
            }
        }
    }

    fn upvalue_set(&mut self, uv: &UpValueRef, v: Value) {
        let mut b = uv.borrow_mut();
        match &mut *b {
            UpValue::Closed(c) => *c = v,
            UpValue::Open { coroutine, index } => {
                if Rc::ptr_eq(coroutine, &self.current) {
                    self.stack[*index] = v;
                } else if let Some(slot) = coroutine.borrow_mut().stack.get_mut(*index) {
                    *slot = v;
                }
            }
        }
    }

    // ---- metatables and indexing ----

    /// `getmetatable(o, raw=true)`.
    pub fn raw_metatable(&self, o: &Value) -> Option<TableRef> {
        match o {
            Value::Nil => None,
            Value::Table(t) => t.metatable(),
            Value::Str(_) => self.class_metatables.get_str("java.lang.String").as_table().cloned(),
            Value::Num(_) => self.class_metatables.get_str("java.lang.Double").as_table().cloned(),
            Value::Bool(_) => self.class_metatables.get_str("java.lang.Boolean").as_table().cloned(),
            Value::Object(obj) => {
                if let Value::Table(m) = self.class_metatables.get_str(obj.class_name()) {
                    return Some(m);
                }
                for c in obj.super_classes() {
                    if let Value::Table(m) = self.class_metatables.get_str(c) {
                        return Some(m);
                    }
                }
                None
            }
            Value::Coroutine(_) => self.class_metatables.get_str("se.krka.kahlua.vm.Coroutine").as_table().cloned(),
            Value::Closure(_) | Value::Native(_) => None,
        }
    }

    /// `getmetatable(o, raw=false)`: honours `__metatable`.
    pub fn metatable(&self, o: &Value) -> Value {
        match self.raw_metatable(o) {
            Some(m) => {
                let mm = m.rawget(&Value::str("__metatable"));
                if mm.is_nil() {
                    Value::Table(m)
                } else {
                    mm
                }
            }
            None => Value::Nil,
        }
    }

    pub fn get_meta_op(&self, o: &Value, op: &str) -> Value {
        match self.raw_metatable(o) {
            Some(m) => m.rawget(&Value::str(op)),
            None => Value::Nil,
        }
    }

    fn get_bin_meta_op(&self, a: &Value, b: &Value, op: &str) -> Value {
        let m = self.get_meta_op(a, op);
        if !m.is_nil() {
            return m;
        }
        self.get_meta_op(b, op)
    }

    fn get_comp_meta_op(&self, a: &Value, b: &Value, op: &str) -> Value {
        let (Some(m1), Some(m2)) = (self.raw_metatable(a), self.raw_metatable(b)) else { return Value::Nil };
        let o1 = m1.rawget(&Value::str(op));
        let o2 = m2.rawget(&Value::str(op));
        if o1.is_nil() || !o1.raw_equals(&o2) {
            return Value::Nil;
        }
        o1
    }

    /// `KahluaThread.tableget`.
    pub fn table_get(&mut self, table: &Value, key: &Value) -> LuaResult<Value> {
        let mut cur = table.clone();
        for _ in 0..MAX_INDEX_RECURSION {
            if let Value::Table(t) = &cur {
                let r = t.rawget(key);
                if !r.is_nil() {
                    return Ok(r);
                }
            }
            let meta = self.get_meta_op(&cur, "__index");
            if meta.is_nil() {
                if matches!(cur, Value::Table(_)) {
                    if let (Value::Object(o), Some(h)) = (table, self.on_missing_member.as_mut()) {
                        h(o.class_name(), &java_value_of(key));
                    }
                    return Ok(Value::Nil);
                }
                let msg = format!("attempted index: {} of non-table: {}", java_value_of(key), java_value_of(&cur));
                self.log_error(&format!(
                    "-------------------------------------------------------------\n{msg}"
                ));
                return Err(LuaError::runtime(msg));
            }
            if meta.is_function() {
                // The original passes the starting object, not `cur`.
                return self.call1(&meta, &[table.clone(), key.clone()]);
            }
            cur = meta;
        }
        Err(LuaError::runtime("loop in gettable"))
    }

    /// `KahluaThread.tableSet`.
    pub fn table_set(&mut self, table: &Value, key: &Value, value: Value) -> LuaResult<()> {
        let mut cur = table.clone();
        for _ in 0..MAX_INDEX_RECURSION {
            let meta;
            if let Value::Table(t) = &cur {
                if !t.rawget(key).is_nil() {
                    t.rawset(key.clone(), value);
                    return Ok(());
                }
                meta = self.get_meta_op(&cur, "__newindex");
                if meta.is_nil() {
                    t.rawset(key.clone(), value);
                    return Ok(());
                }
            } else {
                meta = self.get_meta_op(&cur, "__newindex");
                if meta.is_nil() {
                    return Err(LuaError::runtime("attempted index of non-table"));
                }
            }
            if meta.is_function() {
                self.call(&meta, &[table.clone(), key.clone(), value])?;
                return Ok(());
            }
            cur = meta;
        }
        Err(LuaError::runtime("loop in settable"))
    }

    // ---- tostring ----

    /// `KahluaUtil.tostring`.
    pub fn tostring(&mut self, o: &Value) -> LuaResult<String> {
        match o {
            Value::Nil => return Ok("nil".into()),
            Value::Str(s) => return Ok(s.to_string()),
            Value::Num(n) => return Ok(number_to_string(*n)),
            Value::Bool(b) => return Ok(if *b { "true" } else { "false" }.into()),
            Value::Closure(_) => return Ok(format!("closure 0x{}", identity_hash(o))),
            Value::Native(_) => return Ok(format!("function 0x{}", identity_hash(o))),
            _ => {}
        }
        if let Value::Table(_) = o {
            let m = self.table_get(o, &Value::str("tostring"))?;
            if m.is_function() {
                let r = self.call1(&m, std::slice::from_ref(o))?;
                return Ok(java_value_of(&r));
            }
        }
        let f = self.get_meta_op(o, "__tostring");
        if !f.is_nil() {
            let r = self.call1(&f, std::slice::from_ref(o))?;
            return Ok(java_value_of(&r));
        }
        if let Value::Table(t) = o {
            let mut s = self.table_to_string(o)?;
            s.push_str("{ ");
            let mut any = false;
            for (k, v) in t.entries() {
                if any {
                    s.push_str(", ");
                }
                any = true;
                let ks = if matches!(k, Value::Table(_)) { self.table_to_string(&k)? } else { self.tostring(&k)? };
                s.push_str(&ks);
                s.push('=');
                let vs = if matches!(v, Value::Table(_)) { self.table_to_string(&v)? } else { self.tostring(&v)? };
                s.push_str(&vs);
            }
            s.push_str(" }");
            return Ok(s);
        }
        Ok(java_value_of(o))
    }

    fn table_to_string(&mut self, o: &Value) -> LuaResult<String> {
        let ty = self.table_get(o, &Value::str("Type"))?;
        Ok(match ty {
            Value::Str(s) => format!("{} 0x{}", s, identity_hash(o)),
            _ => format!("{} 0x{}", o.type_name(), identity_hash(o)),
        })
    }

    pub(crate) fn log_error(&mut self, s: &str) {
        (self.log)(s);
    }

    // ---- arithmetic helpers ----

    fn arith(&mut self, op: u32, b: &Value, c: &Value, fname: &str) -> LuaResult<Value> {
        if let (Some(x), Some(y)) = (raw_tonumber(b), raw_tonumber(c)) {
            return Ok(Value::Num(primitive_math(x, y, op)));
        }
        let name = meta_op_name(op);
        let mf = self.get_bin_meta_op(b, c, name);
        if mf.is_nil() {
            return Err(LuaError::runtime(format!("{name} not defined for operands in {fname}")));
        }
        self.call1(&mf, &[b.clone(), c.clone()])
    }

    // ---- interpreter loop ----

    fn frame(&self) -> &CallFrame {
        self.frames.last().unwrap()
    }

    #[inline]
    fn rk(&self, base: usize, k: &[Value], x: usize) -> Value {
        if x >= 256 {
            k[x - 256].clone()
        } else {
            self.stack[base + x].clone()
        }
    }

    /// Run Lua frames until the frame that was on top when entered returns
    /// (`luaMainloop`).
    pub(crate) fn execute(&mut self) -> LuaResult<()> {
        self.exec_level += 1;
        let r = loop {
            match self.step() {
                Ok(Flow::Continue) => {}
                Ok(Flow::Return) => break Ok(()),
                Err(e) => {
                    self.unwind_on_error();
                    break Err(e);
                }
            }
        };
        self.exec_level -= 1;
        r
    }

    /// Error handler of `luaMainloop`: pop frames down to and including the
    /// entry frame of this loop invocation, recording the stack trace.
    fn unwind_on_error(&mut self) {
        let mut last_base = None;
        while let Some(fr) = self.frames.pop() {
            let t = frame_trace(&fr);
            self.current.borrow_mut().stack_trace.push_str(&t);
            last_base = Some(fr.local_base);
            if !fr.from_lua {
                break;
            }
        }
        if let Some(b) = last_base {
            self.close_upvalues(b);
        }
    }

    fn step(&mut self) -> LuaResult<Flow> {
        let (closure, base, pc) = {
            let f = self.frames.last_mut().unwrap();
            let pc = f.pc;
            f.pc += 1;
            (f.closure.clone().unwrap(), f.local_base, pc)
        };
        let proto = closure.prototype.clone();
        let op = proto.code[pc];
        let k = &proto.constants;
        match op & 63 {
            OP_MOVE => {
                let v = self.stack[base + b9(op)].clone();
                self.stack[base + a8(op)] = v;
            }
            OP_LOADK => self.stack[base + a8(op)] = k[bx(op)].clone(),
            OP_LOADBOOL => {
                self.stack[base + a8(op)] = Value::Bool(b9(op) != 0);
                if c9(op) != 0 {
                    self.frames.last_mut().unwrap().pc += 1;
                }
            }
            OP_LOADNIL => {
                for i in a8(op)..=b9(op) {
                    self.stack[base + i] = Value::Nil;
                }
            }
            OP_GETUPVAL => {
                let v = self.upvalue_get(&closure.upvalues[b9(op)]);
                self.stack[base + a8(op)] = v;
            }
            OP_GETGLOBAL => {
                let env = Value::Table(closure.env.borrow().clone());
                let v = self.table_get(&env, &k[bx(op)])?;
                if v.is_nil() {
                    if let (Some(h), Value::Str(name)) = (self.on_missing_global.as_mut(), &k[bx(op)]) {
                        h(name);
                    }
                }
                self.stack[base + a8(op)] = v;
            }
            OP_GETTABLE => {
                let t = self.stack[base + b9(op)].clone();
                let key = self.rk(base, k, c9(op));
                let v = self.table_get(&t, &key)?;
                self.stack[base + a8(op)] = v;
            }
            OP_SETGLOBAL => {
                let v = self.stack[base + a8(op)].clone();
                let key = &k[bx(op)];
                if let (Value::Closure(c), Value::Str(s)) = (&v, key) {
                    *c.debug_name.borrow_mut() = Some(s.clone());
                }
                let env = Value::Table(closure.env.borrow().clone());
                self.table_set(&env, key, v)?;
            }
            OP_SETUPVAL => {
                let v = self.stack[base + a8(op)].clone();
                self.upvalue_set(&closure.upvalues[b9(op)], v);
            }
            OP_SETTABLE => {
                let t = self.stack[base + a8(op)].clone();
                let key = self.rk(base, k, b9(op));
                let v = self.rk(base, k, c9(op));
                self.table_set(&t, &key, v)?;
            }
            OP_NEWTABLE => self.stack[base + a8(op)] = Value::Table(TableRef::new()),
            OP_SELF => {
                let key = self.rk(base, k, c9(op));
                let obj = self.stack[base + b9(op)].clone();
                let f = self.table_get(&obj, &key)?;
                let a = a8(op);
                self.stack[base + a] = f;
                self.stack[base + a + 1] = obj;
            }
            OP_ADD | OP_SUB | OP_MUL | OP_DIV | OP_MOD | OP_POW => {
                let b = self.rk(base, k, b9(op));
                let c = self.rk(base, k, c9(op));
                let fname = closure_name(&closure);
                let r = self.arith(op & 63, &b, &c, &fname)?;
                self.stack[base + a8(op)] = r;
            }
            OP_UNM => {
                let b = self.stack[base + b9(op)].clone();
                let r = match raw_tonumber(&b) {
                    Some(n) => Value::Num(-n),
                    None => {
                        let mf = self.get_meta_op(&b, "__unm");
                        self.call1(&mf, &[b.clone()])?
                    }
                };
                self.stack[base + a8(op)] = r;
            }
            OP_NOT => {
                let b = self.stack[base + b9(op)].truthy();
                self.stack[base + a8(op)] = Value::Bool(!b);
            }
            OP_LEN => {
                let o = self.stack[base + b9(op)].clone();
                let r = match &o {
                    Value::Table(t) => Value::Num(t.len() as f64),
                    Value::Str(s) => Value::Num(java_len(s) as f64),
                    _ => {
                        let f = self.get_meta_op(&o, "__len");
                        if f.is_nil() {
                            return Err(LuaError::runtime("__len not defined for operand"));
                        }
                        self.call1(&f, std::slice::from_ref(&o))?
                    }
                };
                self.stack[base + a8(op)] = r;
            }
            OP_CONCAT => {
                let first = b9(op);
                let mut last = c9(op);
                let mut res = self.stack[base + last].clone();
                while first < last {
                    last -= 1;
                    let mut lastc = last + 1; // index one past the run to merge
                    if let Some(rs) = raw_tostring(&res) {
                        let mut n = 0;
                        let mut pos = last as isize;
                        while first as isize <= pos {
                            if raw_tostring(&self.stack[base + pos as usize]).is_none() {
                                break;
                            }
                            n += 1;
                            pos -= 1;
                        }
                        if n > 0 {
                            let mut s = String::new();
                            for i in (last + 1 - n)..=last {
                                s.push_str(&raw_tostring(&self.stack[base + i]).unwrap());
                            }
                            s.push_str(&rs);
                            res = Value::from(s);
                            lastc = last + 1 - n;
                            if lastc == first {
                                break;
                            }
                        }
                    }
                    // `last` now indexes the left operand that is not a string.
                    let left_idx = lastc - 1;
                    if left_idx < first {
                        break;
                    }
                    let left = self.stack[base + left_idx].clone();
                    let mf = self.get_bin_meta_op(&left, &res, "__concat");
                    if mf.is_nil() {
                        return Err(LuaError::runtime(format!(
                            "__concat not defined for operands: {} and {}",
                            java_value_of(&left),
                            java_value_of(&res)
                        )));
                    }
                    res = self.call1(&mf, &[left, res.clone()])?;
                    last = left_idx;
                }
                self.stack[base + a8(op)] = res;
            }
            OP_JMP => self.jump(sbx(op)),
            OP_EQ | OP_LT | OP_LE => {
                let opc = op & 63;
                let a = a8(op);
                let mut bo = self.rk(base, k, b9(op));
                let mut co = self.rk(base, k, c9(op));
                let res_bool = match (&bo, &co) {
                    (Value::Num(x), Value::Num(y)) => match opc {
                        OP_EQ => x == y,
                        OP_LT => x < y,
                        _ => x <= y,
                    },
                    (Value::Str(x), Value::Str(y)) => match opc {
                        OP_EQ => x == y,
                        OP_LT => java_compare(x, y).is_lt(),
                        _ => java_compare(x, y).is_le(),
                    },
                    _ => {
                        if opc == OP_EQ && bo.raw_equals(&co) && !bo.is_nil() && bo.ptr_id() != 0 {
                            true
                        } else {
                            let mut invert = false;
                            let name = meta_op_name(opc);
                            let mut mf = self.get_comp_meta_op(&bo, &co, name);
                            if mf.is_nil() && opc == OP_LE {
                                mf = self.get_comp_meta_op(&bo, &co, "__lt");
                                std::mem::swap(&mut bo, &mut co);
                                invert = true;
                            }
                            let r = if mf.is_nil() && opc == OP_EQ {
                                bo.raw_equals(&co)
                            } else {
                                if mf.is_nil() {
                                    return Err(LuaError::runtime(format!("{name} not defined for operand")));
                                }
                                self.call1(&mf, &[bo.clone(), co.clone()])?.truthy()
                            };
                            if invert {
                                !r
                            } else {
                                r
                            }
                        }
                    }
                };
                if res_bool == (a == 0) {
                    self.frames.last_mut().unwrap().pc += 1;
                }
            }
            OP_TEST => {
                let v = self.stack[base + a8(op)].truthy();
                if v == (c9(op) == 0) {
                    self.frames.last_mut().unwrap().pc += 1;
                }
            }
            OP_TESTSET => {
                let v = self.stack[base + b9(op)].clone();
                if v.truthy() != (c9(op) == 0) {
                    self.stack[base + a8(op)] = v;
                } else {
                    self.frames.last_mut().unwrap().pc += 1;
                }
            }
            OP_CALL => return self.op_call(op, base, &closure),
            OP_TAILCALL => return self.op_tailcall(op, base),
            OP_RETURN => return self.op_return(op, base),
            OP_FORPREP => {
                let a = a8(op);
                let iter = num(&self.stack[base + a]);
                let step = num(&self.stack[base + a + 2]);
                self.stack[base + a] = Value::Num(iter - step);
                self.jump(sbx(op));
            }
            OP_FORLOOP => {
                let a = a8(op);
                let step = num(&self.stack[base + a + 2]);
                let end = num(&self.stack[base + a + 1]);
                let iter = num(&self.stack[base + a]) + step;
                self.stack[base + a] = Value::Num(iter);
                if if step > 0.0 { iter <= end } else { iter >= end } {
                    self.jump(sbx(op));
                    self.stack[base + a + 3] = Value::Num(iter);
                } else {
                    // clearFromIndex(a)
                    let idx = base + a;
                    if self.top < idx {
                        self.set_top(idx)?;
                    }
                    for v in &mut self.stack[idx..self.top] {
                        *v = Value::Nil;
                    }
                }
            }
            OP_TFORLOOP => {
                let a = a8(op);
                let c = c9(op);
                self.set_top(base + a + 6)?;
                for i in 0..3 {
                    self.stack[base + a + 3 + i] = self.stack[base + a + i].clone();
                }
                self.call_at(base + a + 3, 2)?;
                // clearFromIndex(a + 3 + c)
                let idx = base + a + 3 + c;
                if self.top < idx {
                    self.set_top(idx)?;
                }
                for v in &mut self.stack[idx..self.top] {
                    *v = Value::Nil;
                }
                self.set_top(base + proto.max_stack_size as usize)?;
                let v = self.stack[base + a + 3].clone();
                if !v.is_nil() {
                    self.stack[base + a + 2] = v;
                } else {
                    self.frames.last_mut().unwrap().pc += 1;
                }
            }
            OP_SETLIST => {
                let a = a8(op);
                let mut b = b9(op);
                let mut c = c9(op);
                if b == 0 {
                    b = self.top - (base + a) - 1;
                }
                if c == 0 {
                    let f = self.frames.last_mut().unwrap();
                    c = proto.code[f.pc] as usize;
                    f.pc += 1;
                }
                let offset = (c - 1) * FIELDS_PER_FLUSH;
                let Value::Table(t) = self.stack[base + a].clone() else {
                    return Err(LuaError::runtime("SETLIST on non-table"));
                };
                for i in 1..=b {
                    t.rawset(Value::Num((offset + i) as f64), self.stack[base + a + i].clone());
                }
            }
            OP_CLOSE => self.close_upvalues(base + a8(op)),
            OP_CLOSURE => {
                let a = a8(op);
                let np = proto.prototypes[bx(op)].clone();
                let nup = np.num_upvalues as usize;
                let mut ups = Vec::with_capacity(nup);
                for _ in 0..nup {
                    let f = self.frames.last_mut().unwrap();
                    let uop = proto.code[f.pc];
                    f.pc += 1;
                    let b = b9(uop);
                    match uop & 63 {
                        OP_MOVE => ups.push(self.find_upvalue(base + b)),
                        _ => ups.push(closure.upvalues[b].clone()),
                    }
                }
                let env = closure.env.borrow().clone();
                let c = LuaClosure { prototype: np, env: RefCell::new(env), upvalues: ups, debug_name: RefCell::new(None) };
                self.stack[base + a] = Value::Closure(Rc::new(c));
            }
            OP_VARARG => {
                let a = a8(op);
                let b = b9(op) as isize - 1;
                self.push_varargs(base, a, b, &proto)?;
            }
            _ => return Err(LuaError::runtime(format!("bad opcode {}", op & 63))),
        }
        Ok(Flow::Continue)
    }

    fn jump(&mut self, off: isize) {
        let f = self.frames.last_mut().unwrap();
        f.pc = (f.pc as isize + off) as usize;
    }

    fn push_varargs(&mut self, base: usize, index: usize, n: isize, proto: &Prototype) -> LuaResult<()> {
        let nargs = self.frame().n_arguments;
        let nparams = proto.num_params as usize;
        let mut nvarargs = nargs.saturating_sub(nparams);
        let n = if n == -1 {
            self.set_top(base + index + nvarargs)?;
            nvarargs
        } else {
            n as usize
        };
        if nvarargs > n {
            nvarargs = n;
        }
        let src = base - nargs + nparams;
        for i in 0..nvarargs {
            self.stack[base + index + i] = self.stack[src + i].clone();
        }
        for i in nvarargs..n {
            self.stack[base + index + i] = Value::Nil;
        }
        Ok(())
    }

    fn op_call(&mut self, op: u32, base: usize, closure: &Rc<LuaClosure>) -> LuaResult<Flow> {
        let a = a8(op);
        let b = b9(op);
        let c = c9(op);
        let mut nargs = b as isize - 1;
        if nargs != -1 {
            self.set_top(base + a + nargs as usize + 1)?;
        } else {
            nargs = (self.top - base - a - 1) as isize;
        }
        let mut nargs = nargs as usize;
        self.frames.last_mut().unwrap().restore_top = c != 0;
        let mut local_base = base + a + 1;
        let return_base = base + a;
        let fun_obj = self.stack[base + a].clone();
        if fun_obj.is_nil() {
            let name = closure
                .debug_name
                .borrow()
                .clone()
                .or_else(|| closure.prototype.name.clone())
                .map(|s| s.to_string())
                .unwrap_or_else(|| "unknown".into());
            return Err(LuaError::runtime(format!("Object tried to call nil in {name}")));
        }
        let fun = if fun_obj.is_function() {
            fun_obj.clone()
        } else {
            let m = self.get_meta_op(&fun_obj, "__call");
            if m.is_nil() {
                return Err(LuaError::runtime(format!(
                    "Object {} did not have __call metatable set",
                    java_value_of(&fun_obj)
                )));
            }
            local_base = return_base;
            nargs += 1;
            m
        };
        match fun {
            Value::Closure(lc) => {
                self.push_frame(lc, local_base, return_base, nargs, true)?;
            }
            Value::Native(nf) => {
                self.call_native(&nf, local_base, return_base, nargs)?;
                if self.current.borrow().yielded.is_some() {
                    self.current.borrow_mut().yield_return_base = Some(return_base);
                    return Ok(Flow::Return);
                }
                if self.frame().restore_top {
                    let max = self.frame().closure.as_ref().unwrap().prototype.max_stack_size as usize;
                    let lb = self.frame().local_base;
                    self.set_top(lb + max)?;
                }
            }
            other => return Err(LuaError::runtime(format!("Tried to call a non-function: {}", java_value_of(&other)))),
        }
        Ok(Flow::Continue)
    }

    fn op_tailcall(&mut self, op: u32, base: usize) -> LuaResult<Flow> {
        self.close_upvalues(base);
        let a = a8(op);
        let b = b9(op);
        let mut nargs = b as isize - 1;
        if nargs == -1 {
            nargs = (self.top - base - a - 1) as isize;
        }
        let mut nargs = nargs as usize;
        self.frames.last_mut().unwrap().restore_top = false;
        let fun_obj = self.stack[base + a].clone();
        if fun_obj.is_nil() {
            return Err(LuaError::runtime(""));
        }
        let return_base = self.frame().return_base;
        let mut local_base = return_base + 1;
        let fun = if fun_obj.is_function() {
            fun_obj.clone()
        } else {
            let m = self.get_meta_op(&fun_obj, "__call");
            if m.is_nil() {
                return Err(LuaError::runtime("Object did not have __call metatable set"));
            }
            local_base = return_base;
            nargs += 1;
            m
        };
        // stackCopy(base + a, returnBase, nArguments + 1)
        for i in 0..=nargs {
            self.stack[return_base + i] = self.stack[base + a + i].clone();
        }
        self.set_top(return_base + nargs + 1)?;
        if fun_obj.is_function() {
            // The callee sits at return_base already.
        } else {
            self.stack[return_base] = fun.clone();
        }
        match fun {
            Value::Closure(lc) => {
                let from_lua = self.frame().from_lua;
                self.frames.pop();
                self.push_frame(lc, local_base, return_base, nargs, from_lua)?;
                Ok(Flow::Continue)
            }
            Value::Native(nf) => {
                self.call_native(&nf, local_base, return_base, nargs)?;
                let fr = self.frames.pop().unwrap();
                if !fr.from_lua {
                    return Ok(Flow::Return);
                }
                if self.frame().restore_top {
                    let max = self.frame().closure.as_ref().unwrap().prototype.max_stack_size as usize;
                    let lb = self.frame().local_base;
                    self.set_top(lb + max)?;
                }
                Ok(Flow::Continue)
            }
            other => Err(LuaError::runtime(format!("Tried to call a non-function: {}", java_value_of(&other)))),
        }
    }

    fn op_return(&mut self, op: u32, base: usize) -> LuaResult<Flow> {
        let a = a8(op);
        let mut b = b9(op) as isize - 1;
        self.close_upvalues(base);
        if b == -1 {
            b = (self.top - base - a) as isize;
        }
        let b = b as usize;
        let return_base = self.frame().return_base;
        for i in 0..b {
            self.stack[return_base + i] = self.stack[base + a + i].clone();
        }
        self.set_top(return_base + b)?;
        let fr = self.frames.pop().unwrap();
        if !fr.from_lua {
            return Ok(Flow::Return);
        }
        let caller = self.frame();
        if caller.restore_top {
            let max = caller.closure.as_ref().unwrap().prototype.max_stack_size as usize;
            let lb = caller.local_base;
            self.set_top(lb + max)?;
        }
        Ok(Flow::Continue)
    }

    // ---- coroutines ----

    /// Swap the running coroutine's state out and `co`'s in.
    fn switch_to(&mut self, co: &CoroutineRef) {
        {
            let mut cur = self.current.borrow_mut();
            cur.stack = std::mem::take(&mut self.stack);
            cur.top = self.top;
            cur.frames = std::mem::take(&mut self.frames);
            cur.open_upvalues = std::mem::take(&mut self.open_upvalues);
        }
        {
            let mut n = co.borrow_mut();
            self.stack = std::mem::take(&mut n.stack);
            if self.stack.is_empty() {
                self.stack = vec![Value::Nil; 64];
            }
            self.top = n.top;
            self.frames = std::mem::take(&mut n.frames);
            self.open_upvalues = std::mem::take(&mut n.open_upvalues);
        }
        self.current = co.clone();
    }

    /// `coroutine.create`.
    pub fn create_coroutine(&self, f: Rc<LuaClosure>) -> CoroutineRef {
        let env = f.env.borrow().clone();
        let mut stack = vec![Value::Nil; 64];
        stack[0] = Value::Closure(f);
        Rc::new(RefCell::new(Coroutine { stack, top: 1, env: Some(env), ..Default::default() }))
    }

    pub fn coroutine_status(&self, co: &CoroutineRef) -> &'static str {
        if Rc::ptr_eq(co, &self.current) {
            return "running";
        }
        match co.borrow().status {
            CoStatus::Suspended => "suspended",
            CoStatus::Running => "running",
            CoStatus::Normal => "normal",
            CoStatus::Dead => "dead",
        }
    }

    pub fn current_coroutine(&self) -> Option<CoroutineRef> {
        if Rc::ptr_eq(&self.current, &self.root) {
            None
        } else {
            Some(self.current.clone())
        }
    }

    /// `coroutine.resume`: `Ok(values)` when it yields or returns,
    /// `Err((message, trace))` when it errors.
    pub fn resume(&mut self, co: &CoroutineRef, args: &[Value]) -> LuaResult<Result<Vec<Value>, (Value, String)>> {
        let status = self.coroutine_status(co);
        if status != "suspended" {
            return Err(LuaError::runtime(format!("Can not resume coroutine that is in status: {status}")));
        }
        let parent = self.current.clone();
        parent.borrow_mut().status = CoStatus::Normal;
        co.borrow_mut().parent = Some(parent.clone());
        co.borrow_mut().status = CoStatus::Running;
        self.switch_to(co);
        let started = std::mem::replace(&mut co.borrow_mut().started, true);
        co.borrow_mut().resume_level = self.exec_level + 1;
        let result = if !started {
            // First resume: call the body with the arguments.
            self.set_top(1 + args.len())?;
            for (i, a) in args.iter().enumerate() {
                self.stack[1 + i] = a.clone();
            }
            match self.call_at(0, args.len()) {
                Ok(n) => Ok(self.stack[0..n].to_vec()),
                Err(e) => Err(e),
            }
        } else {
            // Resumed after a yield: the resume arguments are the results
            // of the pending yield call.
            let pending = self.frames.last().cloned();
            let rb = co.borrow_mut().yield_return_base.take().unwrap_or(self.top);
            self.set_top(rb + args.len())?;
            for (i, a) in args.iter().enumerate() {
                self.stack[rb + i] = a.clone();
            }
            if let Some(f) = pending {
                if f.restore_top {
                    let max = f.closure.as_ref().unwrap().prototype.max_stack_size as usize;
                    self.set_top(f.local_base + max)?;
                }
            }
            match self.execute_coroutine_body() {
                Ok(()) => Ok(self.stack[0..self.top].to_vec()),
                Err(e) => Err(e),
            }
        };
        let yielded = co.borrow_mut().yielded.take();
        let out = match (result, yielded) {
            (_, Some(vals)) => {
                co.borrow_mut().status = CoStatus::Suspended;
                Ok(vals)
            }
            (Ok(vals), None) => {
                co.borrow_mut().status = CoStatus::Dead;
                Ok(vals)
            }
            (Err(e), None) => {
                co.borrow_mut().status = CoStatus::Dead;
                let trace = std::mem::take(&mut co.borrow_mut().stack_trace);
                let msg = match &e {
                    LuaError::Kahlua(v) => v.clone(),
                    LuaError::Runtime { message, .. } => Value::from(message.clone()),
                };
                Err((msg, trace))
            }
        };
        self.switch_to(&parent);
        parent.borrow_mut().status = CoStatus::Running;
        co.borrow_mut().parent = None;
        Ok(out)
    }

    fn execute_coroutine_body(&mut self) -> LuaResult<()> {
        if self.frames.is_empty() {
            return Ok(());
        }
        self.execute()
    }

    /// `coroutine.yield` from a native called directly by coroutine code.
    pub fn yield_values(&mut self, values: Vec<Value>) -> LuaResult<()> {
        if Rc::ptr_eq(&self.current, &self.root) || self.current.borrow().resume_level != self.exec_level {
            return Err(LuaError::runtime("Can not yield outside of a coroutine"));
        }
        self.current.borrow_mut().yielded = Some(values);
        Ok(())
    }
}

trait CopyWithin {
    fn copy_within_values(&mut self, src: usize, dst: usize, len: usize);
}
impl CopyWithin for Vec<Value> {
    fn copy_within_values(&mut self, src: usize, dst: usize, len: usize) {
        if dst > src {
            for i in (0..len).rev() {
                self[dst + i] = self[src + i].clone();
            }
        } else {
            for i in 0..len {
                self[dst + i] = self[src + i].clone();
            }
        }
    }
}

fn num(v: &Value) -> f64 {
    match v {
        Value::Num(n) => *n,
        _ => f64::NAN,
    }
}

/// `KahluaThread.primitiveMath`.
pub fn primitive_math(v1: f64, v2: f64, op: u32) -> f64 {
    match op {
        OP_ADD => v1 + v2,
        OP_SUB => v1 - v2,
        OP_MUL => v1 * v2,
        OP_DIV => v1 / v2,
        OP_MOD => {
            if v2 == 0.0 {
                f64::NAN
            } else {
                let ipart = (v1 / v2) as i32; // Java (int) cast: saturating
                v1 - ipart as f64 * v2
            }
        }
        OP_POW => libm::pow(v1, v2),
        _ => 0.0,
    }
}

fn closure_name(c: &LuaClosure) -> String {
    if let Some(n) = c.debug_name.borrow().as_ref() {
        return n.to_string();
    }
    c.prototype.name.as_deref().unwrap_or("unknown").to_string()
}

fn frame_trace(fr: &CallFrame) -> String {
    if let Some(c) = &fr.closure {
        let lines = &c.prototype.lines;
        if fr.pc >= 1 && fr.pc - 1 < lines.len() {
            return format!("at {}:{}\n", c.prototype.name.as_deref().unwrap_or("null"), lines[fr.pc - 1]);
        }
        return String::new();
    }
    match &fr.native {
        Some(n) => format!("at {}\n", n.name),
        None => String::new(),
    }
}
