//! `zombie.core.PerformanceSettings` exposed to Lua, plus the
//! `getPerformance()` global (`LuaManager.GlobalObject.getPerformance`,
//! which returns `PerformanceSettings.instance`).
//!
//! The statics live on the shared Core's [`PerformanceSettings`]
//! (`Core::performance`); `UIManager.useUiFbo` is the UI manager's flag.
//! The world renderer samples the water and puddle quality for each frame.
//!
//! [`PerformanceSettings`]: pz_core::core::PerformanceSettings

use crate::expose::{expose_class, global, int_arg};
use pz_core::core::Core;
use pz_kahlua::value::LuaObject;
use pz_kahlua::{Thread, Value};
use std::any::Any;
use std::cell::RefCell;
use std::rc::Rc;

pub const PERFORMANCE_SETTINGS_CLASS: &str = "zombie.core.PerformanceSettings";

/// `PerformanceSettings.instance`.
struct PerformanceSettingsObject;

impl LuaObject for PerformanceSettingsObject {
    fn class_name(&self) -> &str {
        PERFORMANCE_SETTINGS_CLASS
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// `getUIRenderFPS`: `UIManager.useUiFbo ?
/// Core.getInstance().getOptionUIRenderFPS() : lockFps`.
pub fn ui_render_fps(core: &Core) -> i32 {
    if pz_ui::manager::with_ui(|m| m.use_ui_fbo) {
        core.get_option_ui_render_fps()
    } else {
        core.performance.lock_fps
    }
}

/// Register the class and `getPerformance()` on `thread`, over `core`.
pub fn register(thread: &mut Thread, core: Rc<RefCell<Core>>) {
    let b = expose_class(thread, PERFORMANCE_SETTINGS_CLASS, None);
    macro_rules! get {
        ($name:literal, |$c:ident| $e:expr) => {{
            let core = core.clone();
            b.method($name, move |c| {
                let $c = core.borrow();
                let v = $e;
                Ok(c.push(v))
            });
        }};
    }
    // getLockFPS is static; getFramerate calls it.
    {
        let core = core.clone();
        b.static_fn("getLockFPS", move |c| {
            let v = core.borrow().performance.lock_fps;
            Ok(c.push(v))
        });
    }
    get!("getFramerate", |p| p.performance.lock_fps);
    get!("isFramerateUncapped", |p| p.performance.uncapped_fps);
    get!("getUIRenderFPS", |p| ui_render_fps(&p));
    get!("getLightingFPS", |p| p.performance.lighting_fps);
    get!("getFogQuality", |p| p.performance.fog_quality);
    get!("getViewConeOpacity", |p| p.performance.view_cone_opacity);
    get!("getWaterQuality", |p| p.performance.water_quality);
    get!("getPuddlesQuality", |p| p.performance.puddles_quality);
    {
        let core = core.clone();
        b.method("setLightingFPS", move |c| {
            core.borrow_mut().performance.set_lighting_fps(int_arg(c, 1));
            Ok(0)
        });
    }
    {
        let core = core.clone();
        b.method("setFogQuality", move |c| {
            core.borrow_mut().performance.fog_quality = int_arg(c, 1).clamp(0, 2);
            Ok(0)
        });
    }
    {
        let core = core.clone();
        b.method("setViewConeOpacity", move |c| {
            core.borrow_mut().performance.view_cone_opacity = int_arg(c, 1).clamp(0, 5);
            Ok(0)
        });
    }
    {
        let core = core.clone();
        b.method("setWaterQuality", move |c| {
            core.borrow_mut().performance.water_quality = int_arg(c, 1);
            Ok(0)
        });
    }
    {
        let core = core.clone();
        b.method("setPuddlesQuality", move |c| {
            let quality = int_arg(c, 1);
            core.borrow_mut().performance.puddles_quality = if (0..=2).contains(&quality) { quality } else { 0 };
            Ok(0)
        });
    }
    let instance = Value::Object(Rc::new(PerformanceSettingsObject));
    global(thread, "getPerformance", move |c| Ok(c.push(instance.clone())));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(core: Rc<RefCell<Core>>, src: &str) {
        let mut t = Thread::new(pz_kahlua::TableRef::new());
        pz_kahlua::stdlib::install(&mut t, None).unwrap();
        register(&mut t, core);
        let src = format!("function assert(v) if not v then error('assertion failed!', 2) end end\n{src}");
        let p = pz_kahlua::compiler::compile(&src,Some("test"), Some("test"), None).unwrap();
        let f = Value::Closure(pz_kahlua::vm::LuaClosure::new(Rc::new(p), t.globals().clone()));
        if let Err((e, tr)) = t.pcall(&f, &[]) {
            panic!("{}\n{tr}", pz_kahlua::exec::java_value_of(&e));
        }
    }

    #[test]
    fn get_performance_reads_the_core_settings() {
        let core = Rc::new(RefCell::new(Core::new()));
        core.borrow_mut().performance.lock_fps = 144;
        core.borrow_mut().performance.lighting_fps = 30;
        run(
            core,
            "local p = getPerformance() \
             assert(p == getPerformance()) \
             assert(p:getFramerate() == 144 and PerformanceSettings.getLockFPS() == 144) \
             assert(p:getUIRenderFPS() == 144 and p:getLightingFPS() == 30) \
             assert(p:getViewConeOpacity() == 3 and p:isFramerateUncapped() == false)",
        );
    }

    #[test]
    fn ui_render_fps_follows_the_ui_fbo() {
        let core = Rc::new(RefCell::new(Core::new()));
        core.borrow_mut().performance.lock_fps = 60;
        core.borrow_mut().set_option_ui_render_fps(20);
        pz_ui::manager::with_ui(|m| m.use_ui_fbo = true);
        let on = ui_render_fps(&core.borrow());
        pz_ui::manager::with_ui(|m| m.use_ui_fbo = false);
        assert_eq!((on, ui_render_fps(&core.borrow())), (20, 60));
    }

    #[test]
    fn installed_menu_quality_selectors_update_performance_settings() {
        let core = Rc::new(RefCell::new(Core::new()));
        run(core, r#"
            local p = getPerformance()
            for i = 0, 2 do
                p:setWaterQuality(i)
                p:setPuddlesQuality(i)
                assert(p:getWaterQuality() == i and p:getPuddlesQuality() == i)
            end
            p:setWaterQuality(7)
            p:setPuddlesQuality(3)
            assert(p:getWaterQuality() == 7 and p:getPuddlesQuality() == 0)
            p:setPuddlesQuality(-1)
            assert(p:getPuddlesQuality() == 0)
        "#);
    }

    #[test]
    fn setters_clamp_like_the_original() {
        let core = Rc::new(RefCell::new(Core::new()));
        run(
            core.clone(),
            "local p = getPerformance() p:setFogQuality(7) p:setViewConeOpacity(-1) p:setLightingFPS(500)",
        );
        let p = core.borrow().performance.clone();
        assert_eq!((p.fog_quality, p.view_cone_opacity, p.lighting_fps), (2, 0, 120));
    }
}
