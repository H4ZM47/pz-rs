//! Build 42.22 `zombie.characters.animals.AnimalPartsDefinitions` and
//! `zombie.characters.animals.AnimalPart` Lua bindings over `pz_animals::parts`.

use std::any::Any;
use std::rc::Rc;

use pz_characters::pz_animals::parts::{self, AnimalPart};
use pz_kahlua::value::LuaObject;
use pz_kahlua::{LuaError, NativeFunction, Thread, Value};

use crate::bind_java_util::list_value;
use crate::expose::{expose_class, str_arg, with_this};

pub const ANIMAL_PART_CLASS: &str = "zombie.characters.animals.AnimalPart";
pub const ANIMAL_PARTS_DEFS_CLASS: &str = "zombie.characters.animals.AnimalPartsDefinitions";

#[derive(Clone, Debug)]
struct LuaAnimalPart(AnimalPart);

impl LuaObject for LuaAnimalPart {
    fn class_name(&self) -> &str {
        ANIMAL_PART_CLASS
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

fn part_value(p: AnimalPart) -> Value {
    Value::Object(Rc::new(LuaAnimalPart(p)))
}

fn parts_list(v: Vec<AnimalPart>) -> Value {
    list_value(v.into_iter().map(part_value).collect())
}

pub fn register(thread: &mut Thread) {
    // Repeat-register_all safe guard when replacing __index with a native lookup fn.
    if let Some(meta) = thread
        .class_metatables
        .get_str(ANIMAL_PART_CLASS)
        .as_table()
        .cloned()
    {
        if let Value::Table(methods) = meta.get_str("__pz_methods") {
            meta.set_str("__index", methods);
        }
    }

    let part = expose_class(thread, ANIMAL_PART_CLASS, Some("java.lang.Object"));
    part.static_fn("new", |c| {
        let p = match c.arg(0) {
            Value::Table(t) => AnimalPart::from_table(&t).map_err(|e| LuaError::runtime(e.0))?,
            _ => AnimalPart::default(),
        };
        Ok(c.push(part_value(p)))
    });
    part.method("getItem", |c| {
        let item = with_this::<LuaAnimalPart, _>(c, |p| p.0.item.clone())?;
        Ok(c.push(item.map_or(Value::Nil, |s| Value::str(&s))))
    });
    part.method("getMinNb", |c| {
        let v = with_this::<LuaAnimalPart, _>(c, |p| p.0.min_nb)?;
        Ok(c.push(v as f64))
    });
    part.method("getMaxNb", |c| {
        let v = with_this::<LuaAnimalPart, _>(c, |p| p.0.max_nb)?;
        Ok(c.push(v as f64))
    });
    part.method("getNb", |c| {
        let v = with_this::<LuaAnimalPart, _>(c, |p| p.0.nb)?;
        Ok(c.push(v as f64))
    });

    // Public Java fields (`item`, `minNb`, `maxNb`, `nb`) via __index fallback.
    if let Some(meta) = thread
        .class_metatables
        .get_str(ANIMAL_PART_CLASS)
        .as_table()
        .cloned()
    {
        if let Some(methods) = meta.get_str("__index").as_table().cloned() {
            meta.set_str("__pz_methods", methods.clone());
            meta.set_str(
                "__index",
                Value::Native(NativeFunction::new("__index", move |c| {
                    let key = c.arg(1);
                    let m = methods.rawget(&key);
                    if !m.is_nil() {
                        return Ok(c.push(m));
                    }
                    let Some(name) = key.as_str() else {
                        return Ok(c.push(Value::Nil));
                    };
                    let val = with_this::<LuaAnimalPart, _>(c, |p| match name {
                        "item" => p.0.item.as_deref().map_or(Value::Nil, Value::str),
                        "minNb" => Value::Num(p.0.min_nb as f64),
                        "maxNb" => Value::Num(p.0.max_nb as f64),
                        "nb" => Value::Num(p.0.nb as f64),
                        _ => Value::Nil,
                    })?;
                    Ok(c.push(val))
                })),
            );
        }
    }

    let defs = expose_class(thread, ANIMAL_PARTS_DEFS_CLASS, Some("java.lang.Object"));
    defs.static_fn("getLeather", |c| {
        let Some(ty) = str_arg(c, 0) else {
            return Ok(c.push(Value::Nil));
        };
        let env = c.thread.globals().clone();
        let res = parts::leather(&env, &ty).map_err(|e| LuaError::runtime(e.0))?;
        Ok(c.push(res.map_or(Value::Nil, |s| Value::str(&s))))
    });
    defs.static_fn("getAnimalDef", |c| {
        let Some(ty) = str_arg(c, 0) else {
            return Ok(c.push(Value::Nil));
        };
        let env = c.thread.globals().clone();
        let res = parts::animal_def(&env, &ty).map_err(|e| LuaError::runtime(e.0))?;
        Ok(c.push(res.map_or(Value::Nil, Value::Table)))
    });
    defs.static_fn("getDef", |c| {
        let Value::Table(tbl) = c.arg(0) else {
            return Ok(c.push(parts_list(Vec::new())));
        };
        let Some(kind) = str_arg(c, 1) else {
            return Ok(c.push(parts_list(Vec::new())));
        };
        let res = parts::parts_of(&tbl, &kind).map_err(|e| LuaError::runtime(e.0))?;
        Ok(c.push(parts_list(res)))
    });
    defs.static_fn("getAllPartsDef", |c| {
        let Some(ty) = str_arg(c, 0) else {
            return Ok(c.push(parts_list(Vec::new())));
        };
        let env = c.thread.globals().clone();
        let res = parts::all_parts_def(&env, &ty).map_err(|e| LuaError::runtime(e.0))?;
        Ok(c.push(parts_list(res)))
    });
    defs.static_fn("getAllBonesDef", |c| {
        let Some(ty) = str_arg(c, 0) else {
            return Ok(c.push(parts_list(Vec::new())));
        };
        let env = c.thread.globals().clone();
        let res = parts::all_bones_def(&env, &ty).map_err(|e| LuaError::runtime(e.0))?;
        Ok(c.push(parts_list(res)))
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use pz_kahlua::TableRef;

    #[test]
    fn animal_parts_definitions_round_trip_from_lua() {
        let mut t = Thread::new(TableRef::new());
        pz_kahlua::stdlib::install(&mut t, None).unwrap();
        crate::bind_java_util::register(&mut t);
        register(&mut t);
        // Verify repeat registration preserves __index methods table safely.
        register(&mut t);

        let src = r#"
            local function assert(v, msg) if not v then error(msg or "assertion failed") end end

            AnimalPartsDefinitions = AnimalPartsDefinitions or {}
            AnimalPartsDefinitions.animals = {
                cow = {
                    leather = "Base.Leather_Cow_Full",
                    parts = {
                        { item = "Base.Beef", minNb = 2, maxNb = 4 },
                    },
                    bones = {
                        { item = "Base.AnimalBone", nb = 3 },
                    },
                },
            }

            assert(AnimalPartsDefinitions.getLeather("cow") == "Base.Leather_Cow_Full")
            assert(AnimalPartsDefinitions.getLeather("missing") == nil)

            local def = AnimalPartsDefinitions.getAnimalDef("cow")
            assert(def ~= nil and def.leather == "Base.Leather_Cow_Full")

            local parts = AnimalPartsDefinitions.getAllPartsDef("cow")
            assert(parts:size() == 1)
            local p = parts:get(0)
            assert(p:getItem() == "Base.Beef" and p.item == "Base.Beef")
            assert(p:getMinNb() == 2 and p.minNb == 2)
            assert(p:getMaxNb() == 4 and p.maxNb == 4)
            assert(p:getNb() == -1 and p.nb == -1)

            local bones = AnimalPartsDefinitions.getAllBonesDef("cow")
            assert(bones:size() == 1)
            local b = bones:get(0)
            assert(b:getItem() == "Base.AnimalBone" and b:getNb() == 3)

            local direct = AnimalPartsDefinitions.getDef(def, "parts")
            assert(direct:size() == 1 and direct:get(0):getItem() == "Base.Beef")

            local custom = AnimalPart.new({ item = "Base.Venison", minNb = 1, maxNb = 2 })
            assert(custom:getItem() == "Base.Venison" and custom:getMaxNb() == 2)
            return true
        "#;
        let cl = pz_kahlua::stdlib::load_string(src, Some("bind_animal_parts_test"), t.globals().clone())
            .unwrap();
        let res = t
            .pcall(&cl, &[])
            .unwrap_or_else(|e| panic!("lua error: {e:?}"));
        assert!(matches!(res.first(), Some(Value::Bool(true))));
    }
}
