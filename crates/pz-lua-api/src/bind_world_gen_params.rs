//! The real `WorldGenParams.INSTANCE` shared by the menu, world loader,
//! procedural generation setup and `map_worldgen.bin` saving.

use crate::expose::{expose_class, int_arg, str_arg, with_this};
use pz_kahlua::value::LuaObject;
use pz_kahlua::{LuaError, LuaResult, NativeCall, Thread, Value};
use pz_worldgen::WorldGenParams;
use std::any::Any;
use std::cell::RefCell;
use std::rc::Rc;

const CLASS: &str = "zombie.iso.worldgen.WorldGenParams";

pub type WorldGenParamsRef = Rc<RefCell<WorldGenParams>>;

thread_local! {
    // The installed class's constructor starts with an empty seed and
    // +/-250 cell bounds. INSTANCE is static final and survives Lua resets.
    static PARAMS: WorldGenParamsRef = Rc::new(RefCell::new(WorldGenParams::new("")));
    static INSTANCE: Value = Value::Object(Rc::new(ParamsObject(shared_world_gen_params())));
}

/// The same owner exposed by `WorldGenParams.INSTANCE`. Load into this
/// object rather than replacing it, so retained menu handles stay live.
pub fn shared_world_gen_params() -> WorldGenParamsRef {
    PARAMS.with(Rc::clone)
}

struct ParamsObject(WorldGenParamsRef);

impl LuaObject for ParamsObject {
    fn class_name(&self) -> &str {
        CLASS
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

fn params(c: &NativeCall<'_>) -> LuaResult<WorldGenParamsRef> {
    with_this::<ParamsObject, _>(c, |o| o.0.clone())
}

/// Menu-facing seed/bounds accessors. Random-generator objects and the
/// direct Lua save/load entry points are not exposed here yet.
pub fn register(thread: &mut Thread) {
    let b = expose_class(thread, CLASS, Some("java.lang.Object"));
    b.field("INSTANCE", INSTANCE.with(Clone::clone));
    b.field("GENERATION_SIZE", pz_worldgen::params::GENERATION_SIZE);
    b.field(
        "GENERATION_SQUARES",
        pz_worldgen::params::GENERATION_SQUARES,
    );
    b.method("getSeedString", |c| {
        let value = params(c)?.borrow().seed_string.clone();
        Ok(c.push(Value::str(&value)))
    });
    b.method("setSeedString", |c| {
        let owner = params(c)?;
        let seed = str_arg(c, 1).ok_or_else(|| LuaError::Runtime {
            message: String::new(),
            class: "java.lang.NullPointerException",
        })?;
        let mut owner = owner.borrow_mut();
        owner.seed = pz_worldgen::java::string_hash(&seed);
        owner.seed_string = seed.to_string();
        Ok(0)
    });
    b.method("getSeed", |c| {
        let seed = params(c)?.borrow().seed();
        Ok(c.push(Value::Num(seed as f64)))
    });
    for (get, set, index) in [
        ("getMinXCell", "setMinXCell", 0),
        ("getMinYCell", "setMinYCell", 1),
        ("getMaxXCell", "setMaxXCell", 2),
        ("getMaxYCell", "setMaxYCell", 3),
    ] {
        b.method(get, move |c| {
            let owner = params(c)?;
            let owner = owner.borrow();
            let value = match index {
                0 => owner.min_x_cell,
                1 => owner.min_y_cell,
                2 => owner.max_x_cell,
                _ => owner.max_y_cell,
            };
            Ok(c.push(value))
        });
        b.method(set, move |c| {
            let owner = params(c)?;
            let mut owner = owner.borrow_mut();
            let value = int_arg(c, 1);
            match index {
                0 => owner.min_x_cell = value,
                1 => owner.min_y_cell = value,
                2 => owner.max_x_cell = value,
                _ => owner.max_y_cell = value,
            }
            Ok(0)
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pz_kahlua::{stdlib, TableRef};

    fn run(thread: &mut Thread, source: &str) {
        let f = stdlib::load_string(
            source,
            Some("world-gen-params-test"),
            thread.globals().clone(),
        )
        .unwrap();
        thread.call(&f, &[]).unwrap();
    }

    #[test]
    fn menu_mutations_and_save_load_share_the_live_owner_across_lua_environments() {
        let owner = shared_world_gen_params();
        struct Restore(WorldGenParamsRef, WorldGenParams);
        impl Drop for Restore {
            fn drop(&mut self) {
                *self.0.borrow_mut() = self.1.clone();
            }
        }
        let _restore = Restore(owner.clone(), owner.borrow().clone());
        *owner.borrow_mut() = WorldGenParams::new("");
        let mut thread = Thread::new(TableRef::new());
        stdlib::install(&mut thread, None).unwrap();
        // UI text enters Lua as a host String. The installed LexState
        // intentionally truncates UTF-16 source characters to bytes, so
        // a Unicode source literal would test that original compiler quirk.
        thread.globals().set_str("TEST_SEED", Value::str("menu-é𝄞"));
        register(&mut thread);
        run(
            &mut thread,
            r#"
            local check = function(v) if not v then error("world-generation owner mismatch") end end
            retained = WorldGenParams.INSTANCE
            check(retained == zombie.iso.worldgen.WorldGenParams.INSTANCE)
            check(retained:getSeedString() == "" and retained:getSeed() == 0)
            retained:setSeedString(TEST_SEED)
            retained:setMinXCell(-7)
            retained:setMinYCell(-8)
            retained:setMaxXCell(-9)
            retained:setMaxYCell(17)
            check(retained:getMinXCell() == -7 and retained:getMaxXCell() == -9)
            check(retained:getMinYCell() == -8 and retained:getMaxYCell() == 17)
        "#,
        );
        assert_eq!(owner.borrow().seed_string, "menu-é𝄞");
        assert_eq!(
            owner.borrow().seed,
            pz_worldgen::java::string_hash("menu-é𝄞")
        );
        assert_eq!(
            (owner.borrow().min_x_cell, owner.borrow().max_x_cell),
            (-7, -9)
        );
        let before = owner.borrow().clone();
        let bytes = before.save_bytes();
        *owner.borrow_mut() = WorldGenParams::new("other world");
        owner
            .borrow_mut()
            .load_bytes(&bytes, "authored map_worldgen.bin")
            .unwrap();
        assert_eq!(*owner.borrow(), before);
        run(
            &mut thread,
            r#"
            if retained:getSeedString() ~= TEST_SEED or retained:getMaxYCell() ~= 17 then
                error("retained menu handle lost the loaded parameters")
            end
        "#,
        );
        let old_instance = thread
            .globals()
            .get_str("WorldGenParams")
            .as_table()
            .unwrap()
            .get_str("INSTANCE");
        let mut fresh = Thread::new(TableRef::new());
        stdlib::install(&mut fresh, None).unwrap();
        fresh.globals().set_str("TEST_SEED", Value::str("menu-é𝄞"));
        register(&mut fresh);
        let new_instance = fresh
            .globals()
            .get_str("WorldGenParams")
            .as_table()
            .unwrap()
            .get_str("INSTANCE");
        assert!(
            old_instance.raw_equals(&new_instance),
            "Lua reset replaced the static singleton"
        );
        run(
            &mut fresh,
            r#"
            if WorldGenParams.INSTANCE:getSeedString() ~= TEST_SEED then error("Lua reset changed the world seed") end
        "#,
        );
    }
}
