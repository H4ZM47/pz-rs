//! Build 42.22 Lua bindings for:
//! - `zombie.iso.objects.ObjectRenderEffects`
//! - `zombie.iso.objects.RenderEffectType`

use std::any::Any;
use std::cell::RefCell;
use std::rc::Rc;

use pz_kahlua::value::LuaObject;
use pz_kahlua::{TableRef, Thread, Value};
use pz_world::object_render_effects::{
    ObjectRenderEffects, ObjectRenderEffectsPool, RenderEffectType,
};

use crate::expose::expose_class;

const OBJECT_RENDER_EFFECTS_CLASS: &str = "zombie.iso.objects.ObjectRenderEffects";
const RENDER_EFFECT_TYPE_CLASS: &str = "zombie.iso.objects.RenderEffectType";

thread_local! {
    static POOL: RefCell<ObjectRenderEffectsPool> = RefCell::new(ObjectRenderEffectsPool::default());
}

/// `ObjectRenderEffects.init()` called from `IsoWorld::init`.
pub fn init() {
    POOL.with(|p| p.borrow_mut().init());
}

/// `ObjectRenderEffects.updateStatic()` driven by wind intensity.
pub fn update_static(wind_intensity: f32, dt: f32) {
    POOL.with(|p| p.borrow_mut().update_static(wind_intensity, dt));
}

/// Reset session state on world teardown.
pub fn reset() {
    POOL.with(|p| *p.borrow_mut() = ObjectRenderEffectsPool::default());
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct RenderEffectTypeObj(RenderEffectType);

impl LuaObject for RenderEffectTypeObj {
    fn class_name(&self) -> &'static str {
        RENDER_EFFECT_TYPE_CLASS
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

#[derive(Debug)]
struct RenderEffectObj(RefCell<ObjectRenderEffects>);

impl LuaObject for RenderEffectObj {
    fn class_name(&self) -> &'static str {
        OBJECT_RENDER_EFFECTS_CLASS
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// Register `ObjectRenderEffects` and `RenderEffectType`.
pub fn register(thread: &mut Thread) {
    let et = expose_class(thread, RENDER_EFFECT_TYPE_CLASS, Some("java.lang.Object"));
    for variant in RenderEffectType::ALL {
        et.field(variant.name(), Value::Object(Rc::new(RenderEffectTypeObj(variant))));
    }
    et.method("name", |c| {
        let s = match c.arg(0) {
            Value::Object(o) => o
                .as_any()
                .downcast_ref::<RenderEffectTypeObj>()
                .map(|v| v.0.name())
                .unwrap_or(""),
            _ => "",
        };
        Ok(c.push(Value::str(s)))
    });
    et.method("toString", |c| {
        let s = match c.arg(0) {
            Value::Object(o) => o
                .as_any()
                .downcast_ref::<RenderEffectTypeObj>()
                .map(|v| v.0.name())
                .unwrap_or(""),
            _ => "",
        };
        Ok(c.push(Value::str(s)))
    });
    et.method("ordinal", |c| {
        let ord = match c.arg(0) {
            Value::Object(o) => o
                .as_any()
                .downcast_ref::<RenderEffectTypeObj>()
                .map(|v| v.0.ordinal())
                .unwrap_or(0),
            _ => 0,
        };
        Ok(c.push(ord as f64))
    });
    et.static_fn("values", |c| {
        let tbl = TableRef::new();
        for (i, v) in RenderEffectType::ALL.iter().enumerate() {
            tbl.rawset(
                Value::Num((i + 1) as f64),
                Value::Object(Rc::new(RenderEffectTypeObj(*v))),
            );
        }
        Ok(c.push(Value::Table(tbl)))
    });
    et.static_fn("valueOf", |c| {
        let arg0 = c.arg(0);
        let found = arg0
            .as_str()
            .and_then(RenderEffectType::from_name)
            .map(|v| Value::Object(Rc::new(RenderEffectTypeObj(v))));
        Ok(c.push(found.unwrap_or(Value::Nil)))
    });

    let ore = expose_class(thread, OBJECT_RENDER_EFFECTS_CLASS, Some("java.lang.Object"));
    ore.field("ENABLED", Value::Bool(true));
    ore.static_fn("init", |_| {
        init();
        Ok(0)
    });
    ore.static_fn("updateStatic", |_| {
        let wind = crate::bind_climate::shared_game_time()
            .borrow()
            .climate
            .get_wind_intensity();
        update_static(wind, 1.0);
        Ok(0)
    });
    ore.static_fn("alloc", |c| {
        Ok(c.push(Value::Object(Rc::new(RenderEffectObj(RefCell::new(
            ObjectRenderEffects::default(),
        ))))))
    });
    ore.static_fn("release", |_| Ok(0));
    ore.static_fn("getNew", |c| {
        let ty = match c.arg(1) {
            Value::Object(o) => o
                .as_any()
                .downcast_ref::<RenderEffectTypeObj>()
                .map(|v| v.0)
                .unwrap_or(RenderEffectType::VegetationRustle),
            _ => RenderEffectType::VegetationRustle,
        };
        let is_big = matches!(c.arg(2), Value::Bool(true));
        Ok(c.push(Value::Object(Rc::new(RenderEffectObj(RefCell::new(
            ObjectRenderEffects::new_effect(ty, is_big),
        ))))))
    });
    ore.static_fn("getNextWindEffect", |c| {
        let wind_type = c.arg(0).as_num().unwrap_or(1.0) as i32;
        let is_tree = matches!(c.arg(1), Value::Bool(true));
        let eff = POOL.with(|p| p.borrow_mut().next_wind_effect(wind_type, is_tree));
        Ok(c.push(eff.map_or(Value::Nil, |e| {
            Value::Object(Rc::new(RenderEffectObj(RefCell::new(e))))
        })))
    });
    ore.method("update", |c| {
        let alive = match c.arg(0) {
            Value::Object(o) => o
                .as_any()
                .downcast_ref::<RenderEffectObj>()
                .is_some_and(|e| e.0.borrow_mut().update(1.0)),
            _ => false,
        };
        Ok(c.push(alive))
    });
    ore.method("copyMainFromOther", |c| {
        if let (Value::Object(dst), Value::Object(src)) = (c.arg(0), c.arg(1)) {
            if let (Some(d), Some(s)) = (
                dst.as_any().downcast_ref::<RenderEffectObj>(),
                src.as_any().downcast_ref::<RenderEffectObj>(),
            ) {
                if !Rc::ptr_eq(&dst, &src) {
                    d.0.borrow_mut().copy_main_from_other(&s.0.borrow());
                }
            }
        }
        Ok(0)
    });
    ore.method("add", |c| {
        if let (Value::Object(dst), Value::Object(src)) = (c.arg(0), c.arg(1)) {
            if let (Some(d), Some(s)) = (
                dst.as_any().downcast_ref::<RenderEffectObj>(),
                src.as_any().downcast_ref::<RenderEffectObj>(),
            ) {
                if !Rc::ptr_eq(&dst, &src) {
                    d.0.borrow_mut().add(&s.0.borrow());
                }
            }
        }
        Ok(0)
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn object_render_effects_and_type_round_trip() {
        reset();
        let mut t = Thread::new(TableRef::new());
        pz_kahlua::stdlib::install(&mut t, None).unwrap();
        register(&mut t);

        let f = pz_kahlua::stdlib::load_string(
            r#"
            local function assert(v, msg) if not v then error(msg or "assertion failed") end end
            assert(ObjectRenderEffects.ENABLED == true)
            assert(RenderEffectType.Hit_Tree_Shudder:name() == "Hit_Tree_Shudder")
            assert(RenderEffectType.valueOf("Hit_Door"):ordinal() == 2)
            assert(#RenderEffectType.values() == 3)

            ObjectRenderEffects.init()
            ObjectRenderEffects.updateStatic()
            local wind = ObjectRenderEffects.getNextWindEffect(1, true)
            assert(wind ~= nil and wind:update() == true)

            local eff = ObjectRenderEffects.getNew(nil, RenderEffectType.Hit_Door, false)
            assert(eff ~= nil and eff:update() == true)
            local copy = ObjectRenderEffects.alloc()
            copy:copyMainFromOther(eff)
            copy:add(eff)
            ObjectRenderEffects.release(copy)
            "#,
            Some("object_render_effects_test"),
            t.globals().clone(),
        )
        .unwrap();
        t.pcall(&f, &[])
            .unwrap_or_else(|e| panic!("lua error: {e:?}"));
        reset();
    }
}
