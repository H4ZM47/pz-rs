//! `zombie.scripting.objects.BaseScriptObject` and the shared handle type
//! scripts are stored behind.

use std::any::Any;
use std::cell::{Ref, RefCell, RefMut};
use std::fmt;
use std::rc::{Rc, Weak};

use crate::java;
use crate::manager::ScriptManager;
use crate::module::ScriptModule;
use crate::parser::{self, Block};
use crate::script_type::{ScriptLoadMode, ScriptType};

/// An exception thrown while loading a script. The original logs these
/// through `ExceptionLogger` and flags the bucket as having load errors.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScriptError(pub String);

impl fmt::Display for ScriptError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for ScriptError {}

impl ScriptError {
    pub fn new(msg: impl Into<String>) -> Self {
        ScriptError(msg.into())
    }
}

pub type ScriptResult<T = ()> = Result<T, ScriptError>;

/// State common to every script object (`BaseScriptObject`'s fields).
#[derive(Debug)]
pub struct BaseScriptObject {
    loaded_script_bodies: Vec<Option<String>>,
    script_object_type: ScriptType,
    module: Weak<ScriptModule>,
    script_object_name: Option<String>,
    script_version: i64,
    pub enabled: bool,
    pub debug_only: bool,
}

impl BaseScriptObject {
    pub fn new(type_: ScriptType) -> Self {
        BaseScriptObject {
            loaded_script_bodies: Vec::new(),
            script_object_type: type_,
            module: Weak::new(),
            script_object_name: None,
            script_version: -1,
            enabled: true,
            debug_only: false,
        }
    }

    pub fn script_object_type(&self) -> ScriptType {
        self.script_object_type
    }

    pub fn module(&self) -> Option<Rc<ScriptModule>> {
        self.module.upgrade()
    }

    pub fn set_module(&mut self, module: &Rc<ScriptModule>) {
        self.module = Rc::downgrade(module);
    }

    pub fn module_name(&self) -> Option<String> {
        self.module().map(|m| m.name())
    }

    pub fn script_object_name(&self) -> &str {
        self.script_object_name.as_deref().unwrap_or("")
    }

    pub fn init_load_pp(&mut self, name: &str) {
        self.script_object_name = Some(name.to_string());
    }

    /// `getScriptObjectFullType`: `module.name + "." + name`.
    pub fn full_type(&self) -> ScriptResult<String> {
        match (self.module_name(), &self.script_object_name) {
            (Some(m), Some(n)) => Ok(format!("{m}.{n}")),
            (m, n) => Err(ScriptError(format!(
                "[{}] Module or name missing, module: {}, script: {}",
                self.script_object_type.name(),
                m.as_deref().unwrap_or("null"),
                n.as_deref().unwrap_or("null")
            ))),
        }
    }

    pub fn debug_string(&self) -> String {
        format!(
            "[type={}, module={}, name={}, fulltype={}]",
            self.script_object_type.name(),
            self.module_name().as_deref().unwrap_or("null"),
            self.script_object_name.as_deref().unwrap_or("null"),
            self.full_type().unwrap_or_default()
        )
    }

    pub fn script_version(&self) -> i64 {
        self.script_version
    }

    /// `calculateScriptVersion`: murmur64 of the full type followed by every
    /// (mod id, body) pair that was loaded for this script.
    pub fn calculate_script_version(&mut self) -> ScriptResult {
        let mut sb = self.full_type()?;
        for b in &self.loaded_script_bodies {
            sb.push_str(b.as_deref().unwrap_or("null"));
        }
        self.script_version = murmur_64(sb.as_bytes());
        Ok(())
    }

    pub fn reset_loaded_script_bodies(&mut self) {
        self.loaded_script_bodies.clear();
    }

    pub fn add_loaded_script_body(&mut self, mod_id: Option<&str>, body: &str) {
        self.loaded_script_bodies.push(mod_id.map(str::to_string));
        self.loaded_script_bodies.push(Some(body.to_string()));
    }

    /// `getLoadedScriptBodies`: alternating mod id / body entries.
    pub fn loaded_script_bodies(&self) -> &[Option<String>] {
        &self.loaded_script_bodies
    }

    pub fn loaded_script_body_count(&self) -> usize {
        self.loaded_script_bodies.len()
    }

    /// `LoadCommonBlock(String)`: parses the body and reads the first block.
    pub fn load_common_block_str(&mut self, body: &str) -> ScriptResult {
        let block = parser::parse(body);
        let first = block.children.first().ok_or_else(index_error)?;
        self.load_common_block(first);
        Ok(())
    }

    /// `LoadCommonBlock(Block)`: the `enabled` and `debugOnly` keys.
    pub fn load_common_block(&mut self, block: &Block) {
        for value in &block.values {
            let key = java::trim(value.get_key());
            let val = java::trim(value.get_value());
            if key.is_empty() || val.is_empty() {
                continue;
            }
            if java::eq_ignore_case(key, "enabled") {
                self.enabled = java::eq_ignore_case(val, "true");
            } else if java::eq_ignore_case(key, "debugOnly") {
                self.debug_only = java::eq_ignore_case(val, "true");
            }
        }
    }

    /// `getAllScriptLines`: bodies newest first, each preceded by a
    /// `/* SCRIPT BODY: <mod> */` marker, collapsing runs of blank lines.
    pub fn all_script_lines(&self, list: &mut Vec<String>) {
        if self.loaded_script_bodies.len() < 2 {
            return;
        }
        let mut i = self.loaded_script_bodies.len() as isize - 2;
        while i >= 0 {
            let idx = i as usize;
            let modid = self.loaded_script_bodies[idx].as_deref().unwrap_or("null");
            list.push(format!("/* SCRIPT BODY: {modid} */"));
            push_body_lines(self.loaded_script_bodies[idx + 1].as_deref().unwrap_or(""), list);
            i -= 2;
        }
    }

    /// `getBodyScriptLines`.
    pub fn body_script_lines(&self, body_index: i32, list: &mut Vec<String>) {
        let i = body_index * 2 + 1;
        if i >= 0 && (i as usize) < self.loaded_script_bodies.len() {
            push_body_lines(self.loaded_script_bodies[i as usize].as_deref().unwrap_or(""), list);
        }
    }
}

fn push_body_lines(body: &str, list: &mut Vec<String>) {
    // split("\\r?\\n") drops trailing empty lines.
    let mut lines: Vec<&str> = body.split('\n').map(|l| l.strip_suffix('\r').unwrap_or(l)).collect();
    if !body.is_empty() {
        while lines.last().is_some_and(|l| l.is_empty()) {
            lines.pop();
        }
    }
    let mut empty = 0;
    for s in lines {
        if java::is_null_or_whitespace(Some(s)) {
            empty += 1;
        } else {
            empty = 0;
        }
        if empty >= 2 {
            continue;
        }
        list.push(s.to_string());
    }
}

/// `block.children.get(0)` on an empty list.
pub fn index_error() -> ScriptError {
    ScriptError::new("java.lang.IndexOutOfBoundsException: Index 0 out of bounds for length 0")
}

/// `PZHash.murmur_64(byte[], length, seed = -512093083)` (MurmurHash64A).
pub fn murmur_64(data: &[u8]) -> i64 {
    const M: u64 = 0xc6a4_a793_5bd1_e995;
    const R: u32 = 47;
    let seed: u32 = (-512_093_083i32) as u32;
    let len = data.len();
    let mut h: u64 = (seed as u64) ^ (len as u64).wrapping_mul(M);
    let n8 = len / 8;
    for i in 0..n8 {
        let mut k = u64::from_le_bytes(data[i * 8..i * 8 + 8].try_into().unwrap());
        k = k.wrapping_mul(M);
        k ^= k >> R;
        k = k.wrapping_mul(M);
        h ^= k;
        h = h.wrapping_mul(M);
    }
    let tail = &data[n8 * 8..];
    if !tail.is_empty() {
        for (j, &b) in tail.iter().enumerate().rev() {
            h ^= (b as u64) << (8 * j);
        }
        h = h.wrapping_mul(M);
    }
    h ^= h >> R;
    h = h.wrapping_mul(M);
    h ^= h >> R;
    h as i64
}

/// Behaviour every script type provides: the overridable methods of
/// `BaseScriptObject`. `sm` is `ScriptManager.instance`.
pub trait ScriptObject: Any {
    fn base(&self) -> &BaseScriptObject;
    fn base_mut(&mut self) -> &mut BaseScriptObject;

    /// Java class name, used in diagnostics.
    fn class_name(&self) -> &'static str;

    fn set_module(&mut self, module: &Rc<ScriptModule>) {
        self.base_mut().set_module(module);
    }

    fn init_load_pp(&mut self, _sm: &ScriptManager, name: &str) {
        self.base_mut().init_load_pp(name);
    }

    fn load(&mut self, sm: &ScriptManager, name: &str, body: &str) -> ScriptResult;

    fn get_obsolete(&self) -> bool {
        false
    }

    fn pre_reload(&mut self) {}

    fn reset(&mut self) {}

    fn on_scripts_loaded(&mut self, _sm: &ScriptManager, _mode: ScriptLoadMode) -> ScriptResult {
        Ok(())
    }

    fn on_loaded_after_lua(&mut self, _sm: &ScriptManager) -> ScriptResult {
        Ok(())
    }

    fn on_post_world_dictionary_init(&mut self, _sm: &ScriptManager) -> ScriptResult {
        Ok(())
    }
}

/// A shared, mutable script object, as Java's object references are.
/// Holds the same allocation twice: once as a trait object for the loader,
/// once as `Any` so typed accessors can recover `Rc<RefCell<T>>`.
#[derive(Clone)]
pub struct ScriptRef {
    obj: Rc<RefCell<dyn ScriptObject>>,
    any: Rc<dyn Any>,
}

impl fmt::Debug for ScriptRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.obj.try_borrow() {
            Ok(o) => write!(f, "ScriptRef({})", o.base().debug_string()),
            Err(_) => f.write_str("ScriptRef(<borrowed>)"),
        }
    }
}

impl ScriptRef {
    pub fn new<T: ScriptObject>(script: T) -> Self {
        let rc = Rc::new(RefCell::new(script));
        ScriptRef { obj: rc.clone(), any: rc }
    }

    pub fn borrow(&self) -> Ref<'_, dyn ScriptObject> {
        self.obj.borrow()
    }

    pub fn borrow_mut(&self) -> RefMut<'_, dyn ScriptObject> {
        self.obj.borrow_mut()
    }

    pub fn try_borrow(&self) -> Result<Ref<'_, dyn ScriptObject>, std::cell::BorrowError> {
        self.obj.try_borrow()
    }

    pub fn try_borrow_mut(&self) -> Result<RefMut<'_, dyn ScriptObject>, std::cell::BorrowMutError> {
        self.obj.try_borrow_mut()
    }

    /// Address of the shared script (identity, like Java's `==`).
    pub fn as_ptr(&self) -> *const () {
        Rc::as_ptr(&self.any) as *const ()
    }

    pub fn downcast<T: ScriptObject>(&self) -> Option<Rc<RefCell<T>>> {
        self.any.clone().downcast::<RefCell<T>>().ok()
    }

    pub fn ptr_eq(&self, other: &ScriptRef) -> bool {
        Rc::ptr_eq(&self.any, &other.any)
    }

    pub fn name(&self) -> String {
        self.obj.borrow().base().script_object_name().to_string()
    }

    pub fn full_type(&self) -> ScriptResult<String> {
        self.obj.borrow().base().full_type()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn murmur_covers_every_tail_length() {
        // Script versions from a full Build 42 load matched the original's
        // PZHash.murmur_64 for every script (checked locally); here, make
        // sure each tail length takes its own path.
        let data = b"0123456789abcdef";
        let hashes: Vec<i64> = (0..=16).map(|n| murmur_64(&data[..n])).collect();
        for (i, a) in hashes.iter().enumerate() {
            assert!(hashes[i + 1..].iter().all(|b| a != b));
        }
    }

    #[test]
    fn body_lines_collapse_blank_runs() {
        let mut out = Vec::new();
        push_body_lines("a\r\n\n\n\nb\n\n", &mut out);
        assert_eq!(out, vec!["a", "", "b"]);
    }
}
