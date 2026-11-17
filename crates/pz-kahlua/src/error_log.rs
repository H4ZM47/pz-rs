//! What `KahluaThread.luaMainloop` logs when a Lua frame fails, before it
//! unwinds to the caller: `ExceptionLogger.logException(e)`, then
//! `doStacktraceProper(callFrame)`, which dumps the current coroutine's Lua
//! call frames (`LuaManager.getLuaStackStrace`) through `flushErrorMessage`
//! (`DebugType.General.error`).
//!
//! The Lua frames are named as `LuaClosure.DebugInfo` names them. Files
//! under a `mods` folder need `ZomboidFileSystem.getModInfoForDir`, which
//! this crate cannot reach; they are reported as `Vanilla`.

use crate::exec::java_value_of;
use crate::vm::{LuaClosure, LuaError, Thread};

/// A Java `StackTraceElement` for a Lua frame
/// (`LuaClosure.toStackTraceElement`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LuaStackElement {
    /// `DebugInfo.getSourceName()`: `Lua(<mod>)`.
    pub class_name: String,
    /// `DebugInfo.getFunctionName()`.
    pub method_name: String,
    /// `Prototype.file`.
    pub file_name: Option<String>,
    pub line_number: i32,
}

impl LuaStackElement {
    /// `StackTraceElement.toString()` for an element built from strings.
    pub fn java_to_string(&self) -> String {
        let loc = match (&self.file_name, self.line_number) {
            (None, _) => "Unknown Source".to_string(),
            (Some(f), l) if l >= 0 => format!("{f}:{l}"),
            (Some(f), _) => f.clone(),
        };
        format!("{}.{}({loc})", self.class_name, self.method_name)
    }
}

/// `LuaClosure.isBuiltInFunction`: lines but no full file name.
fn is_built_in(c: &LuaClosure) -> bool {
    !c.prototype.lines.is_empty() && c.prototype.filename.is_none()
}

/// `DebugInfo.generateFileName`: `getRawFileName()` (`filename`, else
/// `file`, else `(Unknown File)`) with `/` as the separator.
fn file_name(c: &LuaClosure) -> String {
    let p = &c.prototype;
    let raw = p.filename.as_deref().or(p.file.as_deref()).unwrap_or("(Unknown File)");
    raw.replace('/', std::path::MAIN_SEPARATOR_STR)
}

/// `DebugInfo.generateFunctionName`.
fn function_name(c: &LuaClosure, file: &str) -> String {
    let p = &c.prototype;
    if is_built_in(c) {
        return p.name.as_deref().unwrap_or("null").to_string();
    }
    if p.lines.is_empty() {
        // `hashCode()`: the identity hash, as `identity_hash` derives it.
        let a = c as *const LuaClosure as u64;
        let h = ((a >> 4) ^ (a >> 20)) as u32 & 0x7FFF_FFFF;
        return format!("function[{}]", to_base36(h as i32));
    }
    let name = p.name.as_deref().unwrap_or(file);
    if name != file {
        return name.to_string();
    }
    let mut n = name;
    if let Some(i) = n.rfind(std::path::MAIN_SEPARATOR) {
        n = &n[i + 1..];
    }
    if let Some(i) = n.rfind('.') {
        n = &n[..i];
    }
    n.to_string()
}

/// `Integer.toString(i, 36)`.
fn to_base36(i: i32) -> String {
    let mut n = (i as i64).unsigned_abs();
    let mut digits = Vec::new();
    loop {
        digits.push(std::char::from_digit((n % 36) as u32, 36).unwrap());
        n /= 36;
        if n == 0 {
            break;
        }
    }
    if i < 0 {
        digits.push('-');
    }
    digits.iter().rev().collect()
}

/// `DebugInfo.generateModName` without the mod lookup (see the module docs).
fn mod_name(c: &LuaClosure) -> &'static str {
    if is_built_in(c) {
        "BuiltIn"
    } else {
        "Vanilla"
    }
}

/// `LuaClosure.toStackTraceElement(pc)`.
pub fn closure_stack_element(c: &LuaClosure, pc: usize) -> LuaStackElement {
    let file = file_name(c);
    let line = c.prototype.lines.get(pc.saturating_sub(1)).copied().unwrap_or(0);
    LuaStackElement {
        class_name: format!("Lua({})", mod_name(c)),
        method_name: function_name(c, &file),
        file_name: c.prototype.file.as_deref().map(str::to_string),
        line_number: line,
    }
}

/// `Throwable.toString()` of the exception the main loop caught.
pub fn throwable_string(e: &LuaError) -> String {
    match e.diagnostic() {
        // `KahluaException.getMessage`: "nil" for a nil error value.
        LuaError::Kahlua(v) => {
            let m = if v.is_nil() { "nil".to_string() } else { java_value_of(v) };
            format!("se.krka.kahlua.vm.KahluaException: {m}")
        }
        LuaError::Runtime { message, class } if message.is_empty() => class.to_string(),
        LuaError::Runtime { message, class } => format!("{class}: {message}"),
        LuaError::WithCause { .. } => unreachable!("leaf diagnostic"),
    }
}

/// `doStacktraceProper`'s message, before `flushErrorMessage` strips its
/// trailing white space.
pub fn stack_trace_message(elements: &[LuaStackElement]) -> String {
    let mut s = String::from(
        "dumping Lua stack trace\n-----------------------------------------\nSTACK TRACE\n-----------------------------------------\n",
    );
    // StackTraceContainer.getStackTraceString(elements, "\t", 0, -1).
    let lines: Vec<String> = elements.iter().map(|e| format!("\t{}", e.java_to_string())).collect();
    s.push_str(&lines.join("\n"));
    s.trim_end().to_string()
}

impl Thread {
    /// `LuaManager.getLuaStackStrace(currentCoroutine)`: the current
    /// coroutine's call frames, innermost first; Java frames are left out.
    pub fn lua_stack_trace(&self) -> Vec<LuaStackElement> {
        self.frames
            .iter()
            .rev()
            .filter_map(|f| f.closure.as_ref().map(|c| closure_stack_element(c, f.pc)))
            .collect()
    }

    /// The logging at the top of `luaMainloop`'s `catch (Throwable)`.
    /// `Core.debug`'s extra stack dump and breakpoint are not ported.
    pub(crate) fn log_mainloop_error(&mut self, e: &LuaError) {
        (self.log_exception)(e);
        let msg = stack_trace_message(&self.lua_stack_trace());
        self.log_error(&msg);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{TableRef, Value};
    use std::cell::RefCell;
    use std::rc::Rc;

    fn thread(log: Rc<RefCell<Vec<String>>>) -> Thread {
        let mut t = Thread::new(TableRef::new());
        crate::stdlib::install(&mut t, None).unwrap();
        let l = log.clone();
        t.log = Box::new(move |s| l.borrow_mut().push(format!("error: {s}")));
        t.log_exception = Box::new(move |e| log.borrow_mut().push(format!("exception: {}", throwable_string(e))));
        t
    }

    fn load(t: &Thread, src: &str, file: &str) -> Value {
        let path = format!("media/lua/client/{file}");
        let p = crate::compiler::compile(src, Some(file), Some(file), Some(&path)).unwrap();
        Value::Closure(LuaClosure::new(Rc::new(p), t.globals().clone()))
    }

    #[test]
    fn a_failing_frame_logs_the_exception_and_the_lua_stack() {
        let log = Rc::default();
        let mut t = thread(Rc::clone(&log));
        let chunk = load(
            &t,
            "function Panel_render(self)\n  local x = nil\n  return x.width\nend\n\
             function Outer()\n  local w = Panel_render({})\n  return w\nend\n",
            "ISPanel.lua",
        );
        t.call(&chunk, &[]).unwrap();
        let outer = t.globals().get_str("Outer");
        assert!(t.pcall(&outer, &[]).is_err());
        let log = log.borrow();
        assert_eq!(
            log.last().unwrap(),
            "error: dumping Lua stack trace\n-----------------------------------------\nSTACK TRACE\n\
             -----------------------------------------\n\
             \tLua(Vanilla).Panel_render(ISPanel.lua:3)\n\tLua(Vanilla).Outer(ISPanel.lua:6)"
        );
        let exc: Vec<&String> = log.iter().filter(|l| l.starts_with("exception: ")).collect();
        assert_eq!(exc, ["exception: java.lang.RuntimeException: attempted index: width of non-table: null"]);
    }

    #[test]
    fn lua_error_values_are_kahlua_exceptions() {
        let log = Rc::default();
        let mut t = thread(Rc::clone(&log));
        let chunk = load(&t, "error('boom')\n", "Boom.lua");
        assert!(t.pcall(&chunk, &[]).is_err());
        let log = log.borrow();
        assert_eq!(log[0], "exception: se.krka.kahlua.vm.KahluaException: boom");
        assert!(log[1].ends_with("\tLua(Vanilla).Boom.lua(Boom.lua:1)"), "{}", log[1]);
    }

    #[test]
    fn element_strings_follow_the_jdk() {
        let e = |f: Option<&str>, l| LuaStackElement {
            class_name: "Lua(Vanilla)".into(),
            method_name: "render".into(),
            file_name: f.map(str::to_string),
            line_number: l,
        };
        assert_eq!(e(Some("A.lua"), 7).java_to_string(), "Lua(Vanilla).render(A.lua:7)");
        assert_eq!(e(Some("A.lua"), -1).java_to_string(), "Lua(Vanilla).render(A.lua)");
        assert_eq!(e(None, 7).java_to_string(), "Lua(Vanilla).render(Unknown Source)");
        assert_eq!((to_base36(71), to_base36(-36)), ("1z".to_string(), "-10".to_string()));
    }
}
