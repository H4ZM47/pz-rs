//! `zombie.iso.worldgen.WorldGenUtils.INSTANCE` in the shared Lua
//! environment, as `media/lua/server/WorldGen/Features.lua` /
//! `Biomes.lua` (`getFiles`, `getFilesNum`, `getFile`) and the new-game
//! screens (`generateSeed`) use it. World generation itself reads its
//! definitions from its own environment (`pz_worldgen::lua`).

use crate::expose::{expose_class, int_arg, str_arg};
use pz_fs::GameFs;
use pz_kahlua::value::LuaObject;
use pz_kahlua::{LuaError, Thread, Value};
use std::any::Any;
use std::cell::RefCell;
use std::rc::Rc;

const CLASS: &str = "zombie.iso.worldgen.WorldGenUtils";

thread_local! {
    /// `WorldGenUtils.files` (the last `getFiles` result).
    static FILES: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
}

struct LuaWorldGenUtils;

impl LuaObject for LuaWorldGenUtils {
    fn class_name(&self) -> &str {
        CLASS
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// `generateSeed()`: 16 letters from `new Random()`.
pub fn generate_seed(rnd: &pz_core::random::JavaRandom) -> String {
    let mut res = String::new();
    for _ in 0..16 {
        let i = rnd.next_int(52);
        res.push(if i < 26 { (i + 65) as u8 as char } else { (i + 97 - 26) as u8 as char });
    }
    res
}

pub fn register(thread: &mut Thread, fs: Rc<RefCell<GameFs>>) {
    let b = expose_class(thread, CLASS, Some("java.lang.Object"));
    b.field("INSTANCE", Value::Object(Rc::new(LuaWorldGenUtils)));
    b.method("getFiles", move |c| {
        crate::expose::this::<LuaWorldGenUtils>(c)?;
        let base = str_arg(c, 1).ok_or_else(|| LuaError::Runtime {
            message: String::new(),
            class: "java.lang.NullPointerException",
        })?;
        let files = pz_worldgen::lua::get_files(&fs.borrow(), &base);
        FILES.with(|f| *f.borrow_mut() = files);
        Ok(0)
    });
    b.method("getFilesNum", |c| {
        crate::expose::this::<LuaWorldGenUtils>(c)?;
        let n = FILES.with(|f| f.borrow().len());
        Ok(c.push(Value::Num(n as f64)))
    });
    b.method("getFile", |c| {
        crate::expose::this::<LuaWorldGenUtils>(c)?;
        let i = int_arg(c, 1);
        let f = FILES.with(|f| {
            let f = f.borrow();
            usize::try_from(i).ok().and_then(|i| f.get(i).cloned()).ok_or(f.len())
        });
        match f {
            Ok(f) => Ok(c.push(Value::str(&f))),
            Err(len) => Err(LuaError::Runtime {
                message: format!("Index {i} out of bounds for length {len}"),
                class: "java.lang.IndexOutOfBoundsException",
            }),
        }
    });
    b.method("getCornerOfGeneration", |c| {
        crate::expose::this::<LuaWorldGenUtils>(c)?;
        let r = pz_worldgen::params::corner_of_generation(int_arg(c, 1));
        Ok(c.push(Value::Num(r as f64)))
    });
    b.method("generateSeed", |c| {
        crate::expose::this::<LuaWorldGenUtils>(c)?;
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos() as i64);
        let s = generate_seed(&pz_core::random::JavaRandom::new(nanos));
        Ok(c.push(Value::str(&s)))
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seed_letters() {
        let s = generate_seed(&pz_core::random::JavaRandom::new(42));
        assert_eq!(s.len(), 16);
        assert!(s.chars().all(|c| c.is_ascii_alphabetic()));
        // The first letter is the first nextInt(52).
        let r = pz_core::random::JavaRandom::new(42);
        let first = r.next_int(52);
        assert_eq!(s.chars().next().unwrap(), if first < 26 { (first + 65) as u8 as char } else { (first + 71) as u8 as char });
    }
}
