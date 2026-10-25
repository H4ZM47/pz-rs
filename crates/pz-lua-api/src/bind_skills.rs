//! `zombie.characters.skills.PerkFactory`, `PerkFactory.Perk` and
//! `PerkFactory.Perks` exposed to Lua, over the shared
//! [`Characters`]' [`PerkFactory`] (filled from the jar before
//! `LuaManager::init`, as `GameWindow.initShared` runs `PerkFactory.init`),
//! plus the members of other classes that take or return perks:
//! `CharacterTraitDefinition`/`CharacterProfessionDefinition`
//! `getXpBoosts`/`addXPBoost` and `SurvivorDesc.getXPBoostMap`.
//!
//! Perk objects are canonical (one Lua object per `Perk`), so Lua `==` and
//! table keys behave like the original's identity.
//!
//! # XP boost maps
//!
//! The original returns its live `HashMap<Perk, Integer>`. Here that is a
//! live [`JavaMap`] over the owner's boosts (stored by perk id) whose values
//! are boxed [`JavaInteger`]s (Lua reads them with `intValue()` after
//! `transformIntoKahluaTable`); mutations write back to the owner, with a
//! Lua number value stored as its `int` (the original would hold a
//! `Double` there and fail later, when the boost is applied). `Perk` does not override `hashCode`, so the
//! original's iteration order follows identity hash codes and is arbitrary;
//! the view iterates in insertion order (the order of the `XPBoosts` script
//! value, then later `addXPBoost` calls), which is one of the orders the
//! original can produce for the same contents.

use crate::bind_characters::{ProfessionDefObject, SurvivorDescObject, TraitDefObject};
use crate::bind_java_util::{class_cast, integer_value, JavaInteger, JavaList, JavaMap, INTEGER};
use crate::bind_registry::{java_error, shared_script_manager};
use crate::expose::{bool_arg, expose_class, int_arg, str_arg, with_this};
use pz_characters::{Characters, PerkFactory, PerkRef};
use pz_kahlua::value::LuaObject;
use pz_kahlua::{LuaError, LuaResult, NativeCall, Thread, Value};
use pz_scripts::ScriptManager;
use std::any::Any;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

pub const PERK_FACTORY_CLASS: &str = "zombie.characters.skills.PerkFactory";
pub const PERK_CLASS: &str = "zombie.characters.skills.PerkFactory$Perk";
pub const PERKS_CLASS: &str = "zombie.characters.skills.PerkFactory$Perks";

thread_local! {
    static PERKS: RefCell<HashMap<usize, Value>> = RefCell::new(HashMap::new());
}

/// A `PerkFactory.Perk`.
pub struct PerkObject(pub PerkRef);

impl LuaObject for PerkObject {
    fn class_name(&self) -> &str {
        PERK_CLASS
    }
    /// `toString`: the id.
    fn to_lua_string(&self) -> String {
        self.0.borrow().id().to_string()
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// The canonical Lua object for `perk` (nil for `null`).
pub fn perk_value(perk: Option<&PerkRef>) -> Value {
    let Some(perk) = perk else { return Value::Nil };
    let id = Rc::as_ptr(perk) as usize;
    PERKS.with(|m| {
        m.borrow_mut()
            .entry(id)
            .or_insert_with(|| Value::Object(Rc::new(PerkObject(perk.clone()))))
            .clone()
    })
}

/// A `Perk` argument: nil is `null`.
pub fn perk_arg(c: &NativeCall<'_>, i: usize) -> LuaResult<Option<PerkRef>> {
    match c.arg(i) {
        Value::Nil => Ok(None),
        Value::Object(o) => match o.as_any().downcast_ref::<PerkObject>() {
            Some(p) => Ok(Some(p.0.clone())),
            None => Err(LuaError::runtime(format!(
                "Expected Perk but got {}",
                o.class_name()
            ))),
        },
        other => Err(LuaError::runtime(format!(
            "Expected Perk but got {}",
            pz_kahlua::stdlib::java_class_name(&other)
        ))),
    }
}

fn npe() -> LuaError {
    LuaError::Runtime {
        message: String::new(),
        class: "java.lang.NullPointerException",
    }
}

fn this_perk(c: &NativeCall<'_>) -> LuaResult<PerkRef> {
    with_this::<PerkObject, _>(c, |p| p.0.clone())
}

/// `Translator.getText` as the character classes use it.
fn get_text(key: &str) -> String {
    Characters::translate(key)
}

/// The entries of a perk-keyed boost map stored as perk ids: each key is
/// `Perks.FromString(id)`, each value a boxed `Integer`.
fn boost_entries(chars: &Rc<RefCell<Characters>>, boosts: &[(String, i32)]) -> Vec<(Value, Value)> {
    let chars = chars.borrow();
    boosts
        .iter()
        .map(|(id, level)| {
            (
                perk_value(chars.perks.from_string(id).as_ref()),
                integer_value(*level),
            )
        })
        .collect()
}

/// The live boost map of `owner` (see the module docs).
fn boost_map<O: 'static>(
    chars: &Rc<RefCell<Characters>>,
    owner: Rc<RefCell<O>>,
    pick: fn(&O) -> &Vec<(String, i32)>,
    pick_mut: fn(&mut O) -> &mut Vec<(String, i32)>,
) -> Value {
    let (ch, r) = (chars.clone(), owner.clone());
    let read = move || boost_entries(&ch, pick(&r.borrow()));
    let write = move |entries: Vec<(Value, Value)>| -> LuaResult<()> {
        let mut out = Vec::with_capacity(entries.len());
        for (k, v) in &entries {
            let perk = match k {
                Value::Object(o) => o
                    .as_any()
                    .downcast_ref::<PerkObject>()
                    .map(|p| p.0.borrow().id().to_string()),
                _ => None,
            }
            .ok_or_else(|| class_cast(k, PERK_CLASS))?;
            let level = match v {
                Value::Object(o) => o.as_any().downcast_ref::<JavaInteger>().map(|i| i.0),
                Value::Num(n) => Some(*n as i32),
                _ => None,
            }
            .ok_or_else(|| class_cast(v, INTEGER))?;
            out.push((perk, level));
        }
        *pick_mut(&mut owner.borrow_mut()) = out;
        Ok(())
    };
    Value::Object(Rc::new(JavaMap::live(read, Some(Box::new(write)))))
}

/// Register the perk classes and the perk members of the character
/// classes (which [`crate::bind_characters::register`] must have exposed),
/// bound to `sm` and `chars`.
pub fn register(thread: &mut Thread, sm: Rc<ScriptManager>, chars: Rc<RefCell<Characters>>) {
    register_perk_factory(thread, &sm, &chars);
    register_perk(thread, &chars);
    register_perks(thread, &chars);
    register_xp_boosts(thread, &chars);
}

/// `register` on the shared ScriptManager and Characters.
pub fn register_shared(thread: &mut Thread) {
    register(
        thread,
        shared_script_manager(),
        crate::bind_characters::shared_characters(),
    );
}

fn register_perk_factory(
    thread: &mut Thread,
    sm: &Rc<ScriptManager>,
    chars: &Rc<RefCell<Characters>>,
) {
    let b = expose_class(thread, PERK_FACTORY_CLASS, None);
    let ch = chars.clone();
    b.field(
        "PerkList",
        Value::Object(Rc::new(JavaList::live(
            move || {
                ch.borrow()
                    .perks
                    .perk_list
                    .iter()
                    .map(|p| perk_value(Some(p)))
                    .collect()
            },
            None,
        ))),
    );
    b.static_fn("getPerkName", |c| {
        let p = perk_arg(c, 0)?.ok_or_else(npe)?;
        Ok(c.push(PerkFactory::perk_name(&p)))
    });
    let ch = chars.clone();
    b.static_fn("getPerkFromName", move |c| {
        let name = str_arg(c, 0).map(|s| s.to_string()).unwrap_or_default();
        let p = ch.borrow().perks.perk_from_name(&name);
        Ok(c.push(perk_value(p.as_ref())))
    });
    b.static_fn("getPerk", |c| {
        let p = perk_arg(c, 0)?;
        Ok(c.push(perk_value(p.as_ref())))
    });
    // The four overloads: (perk, translation, [parent,] xp1..xp10[, passiv]).
    let ch = chars.clone();
    b.static_fn("AddPerk", move |c| {
        let perk = perk_arg(c, 0)?.ok_or_else(npe)?;
        let translation = str_arg(c, 1)
            .map(|s| s.to_string())
            .unwrap_or_else(|| "null".into());
        let has_parent = matches!(c.arg(2), Value::Object(_)) || (c.nargs() == 14);
        let first_xp = if has_parent { 3 } else { 2 };
        let mut xp = [0; 10];
        for (i, x) in xp.iter_mut().enumerate() {
            *x = int_arg(c, first_xp + i);
        }
        let passiv = c.nargs() > first_xp + 10 && bool_arg(c, first_xp + 10);
        let mut chars = ch.borrow_mut();
        let parent = if has_parent {
            perk_arg(c, 2)?
        } else {
            chars.perks.none()
        };
        let p = chars
            .perks
            .add_perk(&perk, &translation, parent, xp, passiv, &get_text);
        Ok(c.push(perk_value(Some(&p))))
    });
    let (s, ch) = (sm.clone(), chars.clone());
    b.static_fn("init", move |_| {
        let tables = s.builtin_tables.borrow();
        ch.borrow_mut()
            .perks
            .init(&tables, &get_text)
            .map_err(java_error)?;
        Ok(0)
    });
    let ch = chars.clone();
    b.static_fn("initTranslations", move |_| {
        ch.borrow_mut().perks.init_translations(&get_text);
        Ok(0)
    });
    let ch = chars.clone();
    b.static_fn("Reset", move |_| {
        ch.borrow_mut().perks.reset();
        Ok(0)
    });
}

fn register_perk(thread: &mut Thread, chars: &Rc<RefCell<Characters>>) {
    let b = expose_class(thread, PERK_CLASS, None);
    // `new Perk(id)` / `new Perk(id, parent)`.
    let ch = chars.clone();
    b.ctor(move |c| {
        let id = str_arg(c, 0)
            .map(|s| s.to_string())
            .unwrap_or_else(|| "null".into());
        let parent = if c.nargs() >= 2 {
            Some(perk_arg(c, 1)?)
        } else {
            None
        };
        let mut chars = ch.borrow_mut();
        let p = match parent {
            Some(parent) => chars.perks.new_perk_with_parent(&id, parent),
            None => chars.perks.new_perk(&id),
        }
        .map_err(java_error)?;
        Ok(c.push(perk_value(Some(&p))))
    });
    b.method("getId", |c| {
        let v = this_perk(c)?.borrow().id().to_string();
        Ok(c.push(v))
    });
    b.method("toString", |c| {
        let v = this_perk(c)?.borrow().id().to_string();
        Ok(c.push(v))
    });
    b.method("index", |c| {
        let v = this_perk(c)?.borrow().index();
        Ok(c.push(v))
    });
    b.method("setCustom", |c| {
        this_perk(c)?.borrow_mut().set_custom();
        Ok(0)
    });
    b.method("isCustom", |c| {
        let v = this_perk(c)?.borrow().is_custom();
        Ok(c.push(v))
    });
    b.method("isPassiv", |c| {
        let v = this_perk(c)?.borrow().is_passiv();
        Ok(c.push(v))
    });
    b.method("getParent", |c| {
        let v = this_perk(c)?.borrow().parent.clone();
        Ok(c.push(perk_value(v.as_ref())))
    });
    b.method("getName", |c| {
        let v = this_perk(c)?.borrow().name().to_string();
        Ok(c.push(v))
    });
    b.method("getType", |c| {
        let p = this_perk(c)?;
        Ok(c.push(perk_value(Some(&p))))
    });
    for n in 1..=10usize {
        b.method(&format!("getXp{n}"), move |c| {
            let v = this_perk(c)?.borrow().xp_n(n);
            Ok(c.push(v))
        });
    }
    b.method("getXpForLevel", |c| {
        let level = int_arg(c, 1);
        let v = this_perk(c)?.borrow().xp_for_level(level);
        Ok(c.push(v as f64))
    });
    b.method("getTotalXpForLevel", |c| {
        let level = int_arg(c, 1);
        let v = this_perk(c)?.borrow().total_xp_for_level(level);
        Ok(c.push(v as f64))
    });
}

fn register_perks(thread: &mut Thread, chars: &Rc<RefCell<Characters>>) {
    let b = expose_class(thread, PERKS_CLASS, None);
    for (field, perk) in &chars.borrow().perks.perks {
        b.field(field, perk_value(Some(perk)));
    }
    let ch = chars.clone();
    b.static_fn("getMaxIndex", move |c| {
        let v = ch.borrow().perks.max_index();
        Ok(c.push(v))
    });
    let ch = chars.clone();
    b.static_fn("fromIndex", move |c| {
        let p = ch
            .borrow()
            .perks
            .from_index(int_arg(c, 0))
            .map_err(java_error)?;
        Ok(c.push(perk_value(p.as_ref())))
    });
    let ch = chars.clone();
    b.static_fn("FromString", move |c| {
        let id = str_arg(c, 0).map(|s| s.to_string());
        // `HashMap.getOrDefault(null, MAX)`: no perk has a null id.
        let p = match id {
            Some(id) => ch.borrow().perks.from_string(&id),
            None => ch.borrow().perks.max(),
        };
        Ok(c.push(perk_value(p.as_ref())))
    });
}

/// The perk id an `addXPBoost` key is stored under.
fn boost_key(c: &NativeCall<'_>) -> LuaResult<Option<String>> {
    Ok(perk_arg(c, 1)?.map(|p| p.borrow().id().to_string()))
}

fn register_xp_boosts(thread: &mut Thread, chars: &Rc<RefCell<Characters>>) {
    let b = expose_class(thread, crate::bind_characters::TRAIT_DEFINITION_CLASS, None);
    let ch = chars.clone();
    b.method("getXpBoosts", move |c| {
        let d = with_this::<TraitDefObject, _>(c, |o| o.0.clone())?;
        Ok(c.push(boost_map(&ch, d, |d| &d.xp_boosts, |d| &mut d.xp_boosts)))
    });
    b.method("addXPBoost", |c| {
        let d = with_this::<TraitDefObject, _>(c, |o| o.0.clone())?;
        // A `null` perk key cannot be stored by id; nothing reads one back.
        if let Some(perk) = boost_key(c)? {
            d.borrow_mut().add_xp_boost(&perk, int_arg(c, 2));
        }
        Ok(0)
    });

    let b = expose_class(
        thread,
        crate::bind_characters::PROFESSION_DEFINITION_CLASS,
        None,
    );
    let ch = chars.clone();
    b.method("getXpBoosts", move |c| {
        let d = with_this::<ProfessionDefObject, _>(c, |o| o.0.clone())?;
        Ok(c.push(boost_map(&ch, d, |d| &d.xp_boosts, |d| &mut d.xp_boosts)))
    });
    b.method("addXPBoost", |c| {
        let d = with_this::<ProfessionDefObject, _>(c, |o| o.0.clone())?;
        if let Some(perk) = boost_key(c)? {
            d.borrow_mut().add_xp_boost(&perk, int_arg(c, 2));
        }
        Ok(0)
    });

    let b = expose_class(thread, crate::bind_characters::SURVIVOR_DESC_CLASS, None);
    let ch = chars.clone();
    b.method("getXPBoostMap", move |c| {
        let d = with_this::<SurvivorDescObject, _>(c, |o| o.desc.clone())?;
        Ok(c.push(boost_map(
            &ch,
            d,
            |d| &d.xp_boost_map,
            |d| &mut d.xp_boost_map,
        )))
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use pz_kahlua::TableRef;
    use pz_scripts::registry::builtin::{PerkDecl, PerkInit};

    fn setup() -> Thread {
        let mut t = Thread::new(TableRef::new());
        pz_kahlua::stdlib::install(&mut t, None).unwrap();
        run(&mut t, "function assert(v, msg) if not v then error(msg or 'assertion failed!', 2) end return v end").unwrap();
        crate::bind_java_util::register(&mut t);
        crate::bind_core::register(
            &mut t,
            None,
            Rc::new(RefCell::new(pz_core::core::Core::new())),
        );
        let sm = Rc::new(ScriptManager::new());
        for (reg, id) in [
            ("character_trait", "t:brave"),
            ("character_profession", "t:baker"),
        ] {
            sm.registries
                .get(reg)
                .unwrap()
                .borrow_mut()
                .register_id(id, false)
                .unwrap();
        }
        {
            let mut st = sm.statics.items.borrow_mut();
            st.constants.insert("character_trait", "BRAVE", "t:brave");
            st.constants
                .insert("character_profession", "BAKER", "t:baker");
        }
        {
            let decl = |f: &str| PerkDecl {
                field: f.into(),
                id: f.into(),
                parent: None,
            };
            let mut tables = sm.builtin_tables.borrow_mut();
            tables.perks = ["None", "Digging", "Outdoors", "Shovel", "MAX"]
                .map(decl)
                .to_vec();
            tables.perk_ids = tables.perks.iter().map(|p| p.id.clone()).collect();
            let xp = [10, 20, 30, 40, 50, 60, 70, 80, 90, 100];
            tables.perk_init = vec![
                PerkInit::SetParent {
                    perk: "None".into(),
                    parent: "None".into(),
                },
                PerkInit::AddPerk {
                    perk: "Outdoors".into(),
                    translation: "Out".into(),
                    parent: None,
                    xp,
                    passiv: false,
                },
                PerkInit::AddPerk {
                    perk: "Shovel".into(),
                    translation: "Shov".into(),
                    parent: Some("Outdoors".into()),
                    xp,
                    passiv: true,
                },
            ];
        }
        let mut chars = Characters {
            perks: PerkFactory::from_tables(&sm.builtin_tables.borrow()).unwrap(),
            ..Default::default()
        };
        chars
            .perks
            .init(&sm.builtin_tables.borrow(), &get_text)
            .unwrap();
        let chars = Rc::new(RefCell::new(chars));
        crate::bind_registry::register(&mut t, sm.clone());
        crate::bind_characters::register(&mut t, sm.clone(), chars.clone());
        register(&mut t, sm, chars);
        t
    }

    fn run(t: &mut Thread, src: &str) -> Result<(), String> {
        let p = pz_kahlua::compiler::compile(src, Some("test"), Some("test"), None).unwrap();
        let f = Value::Closure(pz_kahlua::vm::LuaClosure::new(
            Rc::new(p),
            t.globals().clone(),
        ));
        t.pcall(&f, &[])
            .map(|_| ())
            .map_err(|(e, tr)| format!("{}\n{tr}", pz_kahlua::exec::java_value_of(&e)))
    }

    #[test]
    fn perk_factory_from_lua() {
        let mut t = setup();
        run(
            &mut t,
            r#"
            local s = Perks.Shovel
            assert(s == Perks.FromString("Shovel") and s:getType() == s and tostring(s) == "Shovel")
            assert(s:getParent() == Perks.Outdoors and Perks.Outdoors:getParent() == Perks.None)
            assert(Perks.None:getParent() == Perks.None and Perks.Digging:getParent() == Perks.None)
            assert(s:isPassiv() and not Perks.Outdoors:isPassiv())
            assert(s:getId() == "Shovel" and s:index() == 3 and s:getName() == "IGUI_perks_Shov")
            assert(PerkFactory.getPerkName(s) == "IGUI_perks_Shov")
            assert(PerkFactory.getPerkFromName("IGUI_perks_Out") == Perks.Outdoors)
            assert(PerkFactory.getPerk(s) == s)
            assert(s:getXp1() == 15 and s:getXp10() == 150 and s:getXpForLevel(11) == -1)
            assert(s:getTotalXpForLevel(2) == 45)
            assert(PerkFactory.PerkList:size() == 2 and PerkFactory.PerkList:get(1) == s)
            assert(Perks.getMaxIndex() == 4 and Perks.fromIndex(1) == Perks.Digging)
            assert(Perks.FromString("nope") == Perks.MAX)
            local p = Perk.new("Juggling", Perks.Outdoors)
            p:setCustom()
            PerkFactory.AddPerk(p, "Jug", Perks.Outdoors, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, true)
            assert(p:isCustom() and p:isPassiv() and p:getXp2() == 3 and Perks.getMaxIndex() == 6)
            assert(PerkFactory.AddPerk(Perks.Digging, "Dig", 2, 2, 2, 2, 2, 2, 2, 2, 2, 2):getParent() == Perks.None)
            PerkFactory.Reset()
            assert(Perks.FromString("Juggling") == Perks.MAX and PerkFactory.PerkList:size() == 3)
            "#,
        )
        .unwrap();
        let e = run(&mut t, "PerkFactory.getPerkName(nil)").unwrap_err();
        assert!(e.contains("NullPointerException"), "{e}");
    }

    #[test]
    fn xp_boost_maps() {
        let mut t = setup();
        run(
            &mut t,
            r#"
            local d = CharacterTraitDefinition.addCharacterTraitDefinition(CharacterTrait.BRAVE, "UI_Brave", 4, "", false)
            assert(d:getXpBoosts():isEmpty())
            d:addXPBoost(Perks.Shovel, 2)
            d:addXPBoost(Perks.Digging, -1)
            d:addXPBoost(Perks.Shovel, 3)
            local m = d:getXpBoosts()
            assert(m:size() == 2 and m:get(Perks.Shovel) == 3 and m:containsKey(Perks.Digging))
            assert(tostring(m) == "{Shovel=3, Digging=-1}")
            local tbl = transformIntoKahluaTable(m)
            assert(tbl[Perks.Shovel]:intValue() == 3 and tbl[Perks.Digging]:intValue() == -1)
            local n = 0
            for perk, level in table.pairs(tbl) do n = n + 1; assert(PerkFactory.getPerkName(perk) and level:intValue()) end
            assert(n == 2)

            local p = CharacterProfessionDefinition.addCharacterProfessionDefinition(CharacterProfession.BAKER, "UI_Baker", 0, "", "")
            p:addXPBoost(Perks.Outdoors, 1)
            local live = p:getXpBoosts()
            p:addXPBoost(Perks.Digging, 2)
            assert(live:size() == 2)
            local desc = SurvivorDesc.new()
            local map = desc:getXPBoostMap()
            assert(map:isEmpty())
            desc:setProfessionSkills(p)
            assert(map:size() == 2 and map:get(Perks.Digging) == 2)
            -- The map is live both ways.
            live:put(Perks.Shovel, 4)
            assert(p:getXpBoosts():get(Perks.Shovel) == 4 and live:size() == 3)
            live:remove(Perks.Outdoors)
            assert(transformIntoKahluaTable(p:getXpBoosts())[Perks.Outdoors] == nil)
            assert(not pcall(function() live:put("Shovel", 1) end))
            "#,
        )
        .unwrap();
    }
}
