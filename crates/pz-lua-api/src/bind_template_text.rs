//! Build 42.22 Lua bindings for `zombie.text.templating`:
//! - `TemplateText`
//! - `TemplateTextBuilder`
//! - `ReplaceProvider`
//! - `ReplaceProviderCharacter`

use std::any::Any;
use std::cell::RefCell;
use std::rc::Rc;

use pz_characters::Characters;
use pz_kahlua::value::LuaObject;
use pz_kahlua::{NativeCall, TableRef, Thread, Value};
use pz_text::template::{
    BuiltinNameKind, ReplaceProvider, Replacer, TemplateRng, TemplateTextBuilder,
};

use crate::events::EventManager;
use crate::expose::expose_class;

const TEMPLATE_TEXT_CLASS: &str = "zombie.text.templating.TemplateText";
const TEMPLATE_BUILDER_CLASS: &str = "zombie.text.templating.TemplateTextBuilder";
const REPLACE_PROVIDER_CLASS: &str = "zombie.text.templating.ReplaceProvider";
const REPLACE_PROVIDER_CHAR_CLASS: &str = "zombie.text.templating.ReplaceProviderCharacter";

thread_local! {
    static BUILDER: RefCell<TemplateTextBuilder> = RefCell::new(TemplateTextBuilder::new());
    static RNG: RefCell<TemplateRng> = RefCell::new(TemplateRng::default());
    static CHARACTERS: RefCell<Option<Rc<RefCell<Characters>>>> = const { RefCell::new(None) };
    static EVENTS: RefCell<Option<EventManager>> = const { RefCell::new(None) };
}

/// Reset template text session state on world teardown.
pub fn reset() {
    BUILDER.with(|b| b.borrow_mut().reset());
    RNG.with(|r| *r.borrow_mut() = TemplateRng::default());
    CHARACTERS.with(|c| *c.borrow_mut() = None);
    EVENTS.with(|e| *e.borrow_mut() = None);
}

/// `TemplateText.Initialize()`: registers default name keys (`lastname`,
/// `firstname`, `maleName`, `femaleName`) and fires `Events.OnTemplateTextInit`.
pub fn initialize(thread: &mut Thread) {
    BUILDER.with(|b| b.borrow_mut().register_default_name_keys());
    let sink = EVENTS.with(|e| e.borrow().clone());
    if let Some(events) = sink {
        events.trigger(thread, "OnTemplateTextInit", &[]);
    }
}

#[derive(Debug)]
struct BuilderObj(RefCell<TemplateTextBuilder>);

impl LuaObject for BuilderObj {
    fn class_name(&self) -> &'static str {
        TEMPLATE_BUILDER_CLASS
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

#[derive(Debug)]
struct ProviderObj {
    class_name: &'static str,
    inner: RefCell<ReplaceProvider>,
}

impl LuaObject for ProviderObj {
    fn class_name(&self) -> &'static str {
        self.class_name
    }

    fn super_classes(&self) -> &'static [&'static str] {
        if self.class_name == REPLACE_PROVIDER_CHAR_CLASS {
            &[REPLACE_PROVIDER_CLASS, "zombie.text.templating.IReplaceProvider"]
        } else {
            &["zombie.text.templating.IReplaceProvider"]
        }
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

fn value_to_string(v: &Value) -> Option<String> {
    match v {
        Value::Str(s) => Some(s.as_ref().to_string()),
        Value::Num(n) => Some(if n.fract() == 0.0 {
            format!("{}", *n as i64)
        } else {
            format!("{n}")
        }),
        Value::Bool(b) => Some(b.to_string()),
        _ => None,
    }
}

fn table_to_string_list(tbl: &TableRef) -> Vec<String> {
    let mut out = Vec::new();
    let mut idx = 1usize;
    loop {
        let v = tbl.rawget(&Value::Num(idx as f64));
        if v.is_nil() {
            break;
        }
        if let Some(s) = value_to_string(&v) {
            out.push(s);
        }
        idx += 1;
    }
    if out.is_empty() {
        for (_, v) in tbl.try_entries().unwrap_or_default() {
            if let Some(s) = value_to_string(&v) {
                out.push(s);
            }
        }
    }
    out
}

fn provider_from_table(tbl: &TableRef) -> ReplaceProvider {
    let mut provider = ReplaceProvider::new();
    for (k, v) in tbl.try_entries().unwrap_or_default() {
        if let Value::Str(k_str) = k {
            match &v {
                Value::Table(sub) => {
                    provider.add_list(k_str.as_ref(), table_to_string_list(sub));
                }
                _ => {
                    if let Some(s) = value_to_string(&v) {
                        provider.add_single(k_str.as_ref(), s);
                    }
                }
            }
        }
    }
    provider
}

fn resolve_builtin_name(kind: BuiltinNameKind, rng: &mut TemplateRng) -> String {
    let from_factory = CHARACTERS.with(|slot| {
        let ch_opt = slot.borrow();
        let ch_rc = ch_opt.as_ref()?;
        let ch = ch_rc.borrow();
        let sf = &ch.survivor_factory;
        let pick = |list: &[String], rng: &mut TemplateRng| -> Option<String> {
            if list.is_empty() {
                None
            } else {
                let idx = rng.rand_next_int(list.len() as i32) as usize;
                Some(list[idx].clone())
            }
        };
        match kind {
            BuiltinNameKind::Lastname => pick(&sf.surnames, rng),
            BuiltinNameKind::MaleName => pick(&sf.male_forenames, rng),
            BuiltinNameKind::FemaleName => pick(&sf.female_forenames, rng),
            BuiltinNameKind::Firstname => {
                let female = rng.rand_next_int(2) == 0;
                if female {
                    pick(&sf.female_forenames, rng).or_else(|| pick(&sf.male_forenames, rng))
                } else {
                    pick(&sf.male_forenames, rng).or_else(|| pick(&sf.female_forenames, rng))
                }
            }
        }
    });
    from_factory.unwrap_or_else(|| match kind {
        BuiltinNameKind::Lastname => "Smith".to_string(),
        BuiltinNameKind::FemaleName => "Kate".to_string(),
        BuiltinNameKind::Firstname | BuiltinNameKind::MaleName => "Bob".to_string(),
    })
}

fn build_with_override_arg(builder: &TemplateTextBuilder, input: &str, ov: &Value) -> String {
    RNG.with(|rng| {
        let mut rng = rng.borrow_mut();
        match ov {
            Value::Object(obj) => {
                if let Some(p) = obj.as_any().downcast_ref::<ProviderObj>() {
                    let guard = p.inner.borrow();
                    return builder.build(input, Some(&guard), &mut rng, resolve_builtin_name);
                }
                builder.build(input, None, &mut rng, resolve_builtin_name)
            }
            Value::Table(tbl) => {
                let provider = provider_from_table(tbl);
                builder.build(input, Some(&provider), &mut rng, resolve_builtin_name)
            }
            _ => builder.build(input, None, &mut rng, resolve_builtin_name),
        }
    })
}

fn register_key_on_builder(builder: &mut TemplateTextBuilder, key: &str, val: &Value) {
    match val {
        Value::Table(tbl) => {
            builder.register_list(key, table_to_string_list(tbl));
        }
        v if !v.is_nil() => {
            if let Some(s) = value_to_string(v) {
                builder.register_single(key, s);
            }
        }
        _ => {}
    }
}

fn call_method_zero(thread: &mut Thread, obj: &Value, method: &str) -> Option<Value> {
    let func = thread.table_get(obj, &Value::str(method)).ok()?;
    if func.is_nil() {
        return None;
    }
    let res = thread.pcall(&func, std::slice::from_ref(obj)).ok()?;
    res.into_iter().next()
}

fn extract_character_names(thread: &mut Thread, chr: &Value) -> (String, String) {
    let mut forename = "Bob".to_string();
    let mut surname = "Smith".to_string();
    if !chr.is_nil() {
        if let Some(desc) = call_method_zero(thread, chr, "getDescriptor") {
            if !desc.is_nil() {
                if let Some(Value::Str(f)) = call_method_zero(thread, &desc, "getForename") {
                    if !f.is_empty() {
                        forename = f.as_ref().to_string();
                    }
                }
                if let Some(Value::Str(s)) = call_method_zero(thread, &desc, "getSurname") {
                    if !s.is_empty() {
                        surname = s.as_ref().to_string();
                    }
                }
            }
        }
    }
    (forename, surname)
}

fn rand_next(c: &mut NativeCall<'_>) -> pz_kahlua::LuaResult<usize> {
    let a = c.arg(0).as_num();
    let b = c.arg(1).as_num();
    let res = RNG.with(|rng| {
        let mut rng = rng.borrow_mut();
        match (a, b) {
            (Some(a), Some(b)) => {
                if a.fract() == 0.0 && b.fract() == 0.0 {
                    rng.rand_next_int_range(a as i32, b as i32) as f64
                } else {
                    rng.rand_next_float_range(a as f32, b as f32) as f64
                }
            }
            (Some(a), None) => {
                if a.fract() == 0.0 {
                    rng.rand_next_int(a as i32) as f64
                } else {
                    rng.rand_next_float(a as f32) as f64
                }
            }
            _ => 0.0,
        }
    });
    Ok(c.push(res))
}

/// Register `TemplateText`, `TemplateTextBuilder`, `ReplaceProvider`, and
/// `ReplaceProviderCharacter`.
pub fn register(
    thread: &mut Thread,
    characters: Option<Rc<RefCell<Characters>>>,
    events: Option<EventManager>,
) {
    CHARACTERS.with(|c| *c.borrow_mut() = characters);
    EVENTS.with(|e| *e.borrow_mut() = events);

    let builder = expose_class(thread, TEMPLATE_BUILDER_CLASS, Some("java.lang.Object"));
    builder.ctor(|c| {
        Ok(c.push(Value::Object(Rc::new(BuilderObj(RefCell::new(
            TemplateTextBuilder::new(),
        ))))))
    });
    builder.method("Build", |c| {
        let Value::Object(obj) = c.arg(0) else {
            return Ok(c.push(Value::str("")));
        };
        let Some(b) = obj.as_any().downcast_ref::<BuilderObj>() else {
            return Ok(c.push(Value::str("")));
        };
        let arg1 = c.arg(1);
        let Some(input) = arg1.as_str() else {
            return Ok(c.push(Value::Nil));
        };
        let arg2 = c.arg(2);
        let out = build_with_override_arg(&b.0.borrow(), input, &arg2);
        Ok(c.push(Value::str(&out)))
    });
    builder.method("RegisterKey", |c| {
        if let Value::Object(obj) = c.arg(0) {
            let arg1 = c.arg(1);
            if let (Some(b), Some(key)) = (obj.as_any().downcast_ref::<BuilderObj>(), arg1.as_str())
            {
                let arg2 = c.arg(2);
                register_key_on_builder(&mut b.0.borrow_mut(), key, &arg2);
            }
        }
        Ok(0)
    });
    builder.method("Reset", |c| {
        if let Value::Object(obj) = c.arg(0) {
            if let Some(b) = obj.as_any().downcast_ref::<BuilderObj>() {
                b.0.borrow_mut().reset();
            }
        }
        Ok(0)
    });
    builder.method("CopyFrom", |c| {
        if let (Value::Object(dst), Value::Object(src)) = (c.arg(0), c.arg(1)) {
            if let (Some(dst_b), Some(src_b)) = (
                dst.as_any().downcast_ref::<BuilderObj>(),
                src.as_any().downcast_ref::<BuilderObj>(),
            ) {
                if !Rc::ptr_eq(&dst, &src) {
                    dst_b.0.borrow_mut().copy_from(&src_b.0.borrow());
                }
            }
        }
        Ok(0)
    });

    let prov = expose_class(thread, REPLACE_PROVIDER_CLASS, Some("java.lang.Object"));
    prov.ctor(|c| {
        Ok(c.push(Value::Object(Rc::new(ProviderObj {
            class_name: REPLACE_PROVIDER_CLASS,
            inner: RefCell::new(ReplaceProvider::new()),
        }))))
    });
    prov.method("addKey", |c| {
        if let Value::Object(obj) = c.arg(0) {
            let arg1 = c.arg(1);
            if let (Some(p), Some(key)) =
                (obj.as_any().downcast_ref::<ProviderObj>(), arg1.as_str())
            {
                let mut guard = p.inner.borrow_mut();
                let arg2 = c.arg(2);
                match &arg2 {
                    Value::Table(tbl) => guard.add_list(key, table_to_string_list(tbl)),
                    v if !v.is_nil() => {
                        if let Some(s) = value_to_string(v) {
                            guard.add_single(key, s);
                        }
                    }
                    _ => {}
                }
            }
        }
        Ok(0)
    });
    prov.method("addReplacer", |c| {
        if let Value::Object(obj) = c.arg(0) {
            let arg1 = c.arg(1);
            if let (Some(p), Some(key)) =
                (obj.as_any().downcast_ref::<ProviderObj>(), arg1.as_str())
            {
                let arg2 = c.arg(2);
                if let Some(v) = value_to_string(&arg2) {
                    p.inner.borrow_mut().add_replacer(key, Replacer::Single(v));
                }
            }
        }
        Ok(0)
    });
    prov.method("hasReplacer", |c| {
        let has = if let Value::Object(obj) = c.arg(0) {
            let arg1 = c.arg(1);
            match (obj.as_any().downcast_ref::<ProviderObj>(), arg1.as_str()) {
                (Some(p), Some(key)) => p.inner.borrow().has_replacer(key),
                _ => false,
            }
        } else {
            false
        };
        Ok(c.push(has))
    });
    prov.method("getReplacer", |c| {
        let val = if let Value::Object(obj) = c.arg(0) {
            let arg1 = c.arg(1);
            match (obj.as_any().downcast_ref::<ProviderObj>(), arg1.as_str()) {
                (Some(p), Some(key)) => {
                    let guard = p.inner.borrow();
                    guard
                        .get_replacer(key)
                        .map(|rep| {
                            RNG.with(|rng| {
                                Value::str(&rep.resolve(
                                    &mut rng.borrow_mut(),
                                    resolve_builtin_name,
                                ))
                            })
                        })
                        .unwrap_or(Value::Nil)
                }
                _ => Value::Nil,
            }
        } else {
            Value::Nil
        };
        Ok(c.push(val))
    });

    let prov_chr = expose_class(
        thread,
        REPLACE_PROVIDER_CHAR_CLASS,
        Some(REPLACE_PROVIDER_CLASS),
    );
    prov_chr.ctor(|c| {
        let chr = c.arg(0);
        let (forename, surname) = extract_character_names(c.thread, &chr);
        let mut provider = ReplaceProvider::new();
        provider.add_single("firstname", forename);
        provider.add_single("lastname", surname);
        Ok(c.push(Value::Object(Rc::new(ProviderObj {
            class_name: REPLACE_PROVIDER_CHAR_CLASS,
            inner: RefCell::new(provider),
        }))))
    });

    let tt = expose_class(thread, TEMPLATE_TEXT_CLASS, Some("java.lang.Object"));
    tt.static_fn("Initialize", |c| {
        initialize(c.thread);
        Ok(0)
    });
    tt.static_fn("Reset", |_| {
        BUILDER.with(|b| b.borrow_mut().reset());
        Ok(0)
    });
    tt.static_fn("CreateBlanc", |c| {
        Ok(c.push(Value::Object(Rc::new(BuilderObj(RefCell::new(
            TemplateTextBuilder::new(),
        ))))))
    });
    tt.static_fn("CreateCopy", |c| {
        let cloned = BUILDER.with(|b| b.borrow().clone());
        Ok(c.push(Value::Object(Rc::new(BuilderObj(RefCell::new(cloned))))))
    });
    tt.static_fn("Build", |c| {
        let arg0 = c.arg(0);
        let Some(input) = arg0.as_str() else {
            return Ok(c.push(Value::Nil));
        };
        let arg1 = c.arg(1);
        let out = BUILDER.with(|b| build_with_override_arg(&b.borrow(), input, &arg1));
        Ok(c.push(Value::str(&out)))
    });
    tt.static_fn("RegisterKey", |c| {
        let arg0 = c.arg(0);
        if let Some(key) = arg0.as_str() {
            let arg1 = c.arg(1);
            BUILDER.with(|b| register_key_on_builder(&mut b.borrow_mut(), key, &arg1));
        }
        Ok(0)
    });
    tt.static_fn("RandNext", rand_next);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn template_text_and_replace_provider_character_round_trip() {
        reset();
        let mut t = Thread::new(TableRef::new());
        pz_kahlua::stdlib::install(&mut t, None).unwrap();
        let events = EventManager::register(t.globals(), "Events", crate::events::EVENT_NAMES);
        let chars = Rc::new(RefCell::new(Characters::default()));
        {
            let mut c = chars.borrow_mut();
            c.survivor_factory.male_forenames = vec!["Arthur".into()];
            c.survivor_factory.female_forenames = vec!["Alice".into()];
            c.survivor_factory.surnames = vec!["Pendleton".into()];
        }
        register(&mut t, Some(chars), Some(events));

        let f = pz_kahlua::stdlib::load_string(
            r#"
            local function assert(v, msg) if not v then error(msg or "assertion failed") end end
            local init_fired = false
            Events.OnTemplateTextInit.Add(function()
                init_fired = true
                TemplateText.RegisterKey("fruit", { "Apple", "Pear" })
            end)
            TemplateText.Initialize()
            assert(init_fired, "OnTemplateTextInit should fire")

            local built = TemplateText.Build("Hi ${maleName} ${lastname}!")
            assert(built == "Hi Arthur Pendleton!", "got: " .. tostring(built))

            local ov = TemplateText.Build("${firstname} ${lastname} ate ${fruit}", {
                firstname = "John",
                lastname = "Doe",
                fruit = { "Peach" },
            })
            assert(ov == "John Doe ate Peach", "got: " .. tostring(ov))

            local fake_chr = {
                getDescriptor = function()
                    return {
                        getForename = function() return "Kate" end,
                        getSurname = function() return "Baldspot" end,
                    }
                end
            }
            local prov = ReplaceProviderCharacter.new(fake_chr)
            assert(prov:hasReplacer("firstname") and prov:hasReplacer("lastname"))
            prov:addKey("fruit", { "Gold" })
            local from_chr = TemplateText.Build("${firstname} ${lastname}: ${fruit}", prov)
            assert(from_chr == "Kate Baldspot: Gold", "got: " .. tostring(from_chr))

            local blanc = TemplateText.CreateBlanc()
            assert(blanc:Build("${fruit}") == "missing_fruit")
            local copy = TemplateText.CreateCopy()
            local cf = copy:Build("${maleName}")
            assert(cf == "Arthur", "got: " .. tostring(cf))

            local r = TemplateText.RandNext(10, 20)
            assert(r >= 10 and r < 20, "bad rand: " .. tostring(r))
            "#,
            Some("template_text_test"),
            t.globals().clone(),
        )
        .unwrap();
        t.pcall(&f, &[])
            .unwrap_or_else(|e| panic!("lua error: {e:?}"));
        reset();
    }
}
