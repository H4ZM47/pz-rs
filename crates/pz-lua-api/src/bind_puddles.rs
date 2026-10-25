//! The live IsoPuddles singleton and its five PuddlesFloat views.
//! Scalar state survives menu resets; the actual IsoWeatherFx belongs to
//! the loaded cell. Climate effects advance after the climate's Lua events,
//! independently of whether the host has a GPU.

use crate::expose::{bool_arg, expose_class, global, int_arg, num_arg, with_this};
use pz_core::weather::ClimateContext;
use pz_iso_render::weather::{ClimateInputs, PuddlesFloat, PuddlesState};
use pz_iso_render::weather_fx::{IsoWeatherFx, WeatherFxClimate, WeatherFxSandbox, WeatherFxTextures};
use pz_kahlua::{LuaResult, NativeCall, Thread, Value};
use std::any::Any;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

const CLASS: &str = "zombie.iso.IsoPuddles";
const FLOAT_CLASS: &str = "zombie.iso.IsoPuddles$PuddlesFloat";
const NAME_KEYS: [&str; 5] = [
    "IGUI_PuddlesControl_Rain",
    "IGUI_PuddlesControl_WetGround",
    "IGUI_PuddlesControl_MudPuddle",
    "IGUI_PuddlesControl_PuddleSize",
    "IGUI_PuddlesControl_RainIntensity",
];

/// IsoPuddles' simulation, with the names translated once by setup().
pub struct PuddlesManager {
    pub state: PuddlesState,
    names: [String; 5],
}

impl Default for PuddlesManager {
    fn default() -> Self {
        Self {
            state: PuddlesState::default(),
            names: NAME_KEYS.map(|key| crate::bind_text::with_translator(|t| t.get_text(key, &[])).unwrap_or_else(|| key.into())),
        }
    }
}

impl PuddlesManager {
    fn float(&mut self, id: usize) -> &mut PuddlesFloat {
        match id {
            0 => &mut self.state.rain,
            1 => &mut self.state.wet_ground,
            2 => &mut self.state.muddy_puddles,
            3 => &mut self.state.puddles_size,
            4 => &mut self.state.rain_intensity,
            _ => unreachable!("validated puddles float id"),
        }
    }
}

type Shared = Rc<RefCell<PuddlesManager>>;

thread_local! {
    static INSTANCE: Shared = Rc::default();
    static HANDLES: RefCell<HashMap<Option<usize>, Value>> = RefCell::default();
}

/// The singleton used by Lua and the host's surface simulation.
pub fn shared_puddles() -> Shared {
    INSTANCE.with(Rc::clone)
}

/// Retire old Lua identities, preserving the original scalar singleton.
pub(crate) fn reset_session_handles() {
    HANDLES.with(|h| h.borrow_mut().clear());
}

struct ManagerHandle(Shared);
struct FloatHandle {
    owner: Shared,
    id: usize,
}

impl pz_kahlua::LuaObject for ManagerHandle {
    fn class_name(&self) -> &str {
        CLASS
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}
impl pz_kahlua::LuaObject for FloatHandle {
    fn class_name(&self) -> &str {
        FLOAT_CLASS
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

fn handle(id: Option<usize>) -> Value {
    HANDLES.with(|handles| {
        handles
            .borrow_mut()
            .entry(id)
            .or_insert_with(|| match id {
                None => Value::Object(Rc::new(ManagerHandle(shared_puddles()))),
                Some(id) => Value::Object(Rc::new(FloatHandle {
                    owner: shared_puddles(),
                    id,
                })),
            })
            .clone()
    })
}

fn with_float<R>(c: &NativeCall<'_>, f: impl FnOnce(&mut PuddlesFloat) -> R) -> LuaResult<R> {
    with_this::<FloatHandle, _>(c, |h| f(h.owner.borrow_mut().float(h.id)))
}

/// IsoCell.initWeatherFx: create the real particle simulation once per
/// cell, at the world-init phase, using the original shared Rand stream.
/// Missing textures remain optional; no GPU device is needed to simulate.
pub fn init_weather_fx(fs: &pz_fs::GameFs) -> bool {
    let context = crate::bind_climate::LuaClimateContext::new();
    if context.is_server() {
        return false;
    }
    let create = crate::bind_world::with_world(|w| w.cell.is_some() && w.weather_fx.is_none());
    if create {
        let tm = crate::bind_render::textures();
        let textures = WeatherFxTextures::load(|path, flags| tm.borrow_mut().shared_texture_flags(fs, path, flags));
        let fx = pz_core::random::with_standard(|rand| IsoWeatherFx::new(rand, &textures));
        crate::bind_world::with_world(|w| w.weather_fx = Some(Rc::new(RefCell::new(fx))));
    }
    crate::bind_world::with_world(|w| w.weather_fx.is_some())
}

fn fx_sandbox(thread: &mut Thread) -> WeatherFxSandbox {
    let mut settings = WeatherFxSandbox::default();
    if let Ok(Value::Object(obj)) = crate::bind_sandbox::instance(thread) {
        if let Some(obj) = obj.as_any().downcast_ref::<crate::bind_sandbox::SandboxObj>() {
            let opts = obj.0.borrow();
            let option = |name: &str| {
                opts.option_by_name(name)
                    .and_then(|v| v.config.get_value_as_string().parse::<i32>().ok())
            };
            settings.max_fog_intensity = option("MaxFogIntensity").unwrap_or(settings.max_fog_intensity);
            settings.max_rain_fx_intensity = option("MaxRainFxIntensity").unwrap_or(settings.max_rain_fx_intensity);
        }
    }
    settings
}

/// ClimateManager.updateFx's scalar/particle setters. Called exactly once
/// after its final Lua event dispatch, with every GameTime borrow released.
pub(crate) fn update_climate_fx(thread: &mut Thread) {
    let gt = crate::bind_climate::shared_game_time();
    if gt.borrow().climate.disable_fx_update {
        return;
    }
    let context = crate::bind_climate::LuaClimateContext::new();
    let fx = crate::bind_world::with_world(|w| w.weather_fx.clone());
    if !context.is_server() && fx.is_none() {
        return;
    }
    let sandbox = fx_sandbox(thread);
    let (inputs, particle_inputs, multiplier, thirty_fps_multiplier) = {
        let g = gt.borrow();
        let cm = &g.climate;
        if cm.disable_fx_update {
            return;
        }
        (
            ClimateInputs {
                rain_intensity: cm.get_rain_intensity(),
                wind_intensity: cm.get_wind_intensity(),
                corrected_wind_angle_intensity: cm.get_corrected_wind_angle_intensity(),
                temperature: cm.get_temperature(),
                ..Default::default()
            },
            WeatherFxClimate::from_render_values(&cm.render_values(0, false)),
            g.get_multiplier(&context),
            g.get_thirty_fps_multiplier(&context),
        )
    };
    if let Some(fx) = fx {
        fx.borrow_mut().update_fx(&particle_inputs, &sandbox);
    }
    shared_puddles()
        .borrow_mut()
        .state
        .update(&inputs, multiplier, thirty_fps_multiplier);
}

pub fn register(thread: &mut Thread) {
    let b = expose_class(thread, CLASS, Some("java.lang.Object"));
    for (name, id) in [
        ("FLOAT_RAIN", 0),
        ("FLOAT_WETGROUND", 1),
        ("FLOAT_MUDDYPUDDLES", 2),
        ("FLOAT_PUDDLESSIZE", 3),
        ("FLOAT_RAININTENSITY", 4),
        ("FLOAT_MAX", 5),
    ] {
        b.field(name, id);
    }
    b.static_fn("getInstance", |c| Ok(c.push(handle(None))));
    global(thread, "getPuddlesManager", |c| Ok(c.push(handle(None))));
    type Getter = fn(&PuddlesState) -> f32;
    for (name, get) in [
        ("getPuddlesSize", (|s: &PuddlesState| s.puddles_size.get()) as Getter),
        ("getPuddlesSizeFinalValue", |s| s.puddles_size.get()),
        ("getWetGroundFinalValue", |s| s.wet_ground.get()),
        ("getMuddyPuddlesFinalValue", |s| s.muddy_puddles.get()),
        ("getRainIntensity", |s| s.rain_intensity.get()),
        ("getShaderTime", |s| s.time),
    ] {
        b.method(name, move |c| {
            let value = with_this::<ManagerHandle, _>(c, |h| get(&h.0.borrow().state))?;
            Ok(c.push(value as f64))
        });
    }
    for (name, value) in [("getFloatMax", 5), ("getBoolMax", 0)] {
        b.method(name, move |c| {
            with_this::<ManagerHandle, _>(c, |_| ())?;
            Ok(c.push(value))
        });
    }
    b.method("getPuddlesFloat", |c| {
        with_this::<ManagerHandle, _>(c, |_| ())?;
        let id = int_arg(c, 1);
        let value = if (0..5).contains(&id) {
            handle(Some(id as usize))
        } else {
            eprintln!("ERROR: Climate: cannot get float override id.");
            Value::Nil
        };
        Ok(c.push(value))
    });
    let b = expose_class(thread, FLOAT_CLASS, Some("java.lang.Object"));
    b.method("getID", |c| {
        let id = with_this::<FloatHandle, _>(c, |h| h.id)?;
        Ok(c.push(id as i32))
    });
    b.method("getName", |c| {
        let name = with_this::<FloatHandle, _>(c, |h| h.owner.borrow().names[h.id].clone())?;
        Ok(c.push(name))
    });
    for (name, value) in [("getMin", 0), ("getMax", 1)] {
        b.method(name, move |c| {
            with_this::<FloatHandle, _>(c, |_| ())?;
            Ok(c.push(value))
        });
    }
    b.method("getFinalValue", |c| {
        let value = with_float(c, |f| f.get())?;
        Ok(c.push(value as f64))
    });
    b.method("getAdminValue", |c| {
        let value = with_float(c, |f| f.get_admin_value())?;
        Ok(c.push(value as f64))
    });
    b.method("isEnableAdmin", |c| {
        let value = with_float(c, |f| f.admin_override.is_some())?;
        Ok(c.push(value))
    });
    b.method("setEnableAdmin", |c| {
        let enabled = bool_arg(c, 1);
        with_float(c, |f| f.set_enable_admin(enabled))?;
        Ok(0)
    });
    for (name, set) in [
        ("setFinalValue", PuddlesFloat::set as fn(&mut PuddlesFloat, f32)),
        ("addFinalValue", PuddlesFloat::add),
        ("setAdminValue", PuddlesFloat::set_admin_value),
        ("interpolateFinalValue", PuddlesFloat::interpolate_final_value),
    ] {
        b.method(name, move |c| {
            let value = num_arg(c, 1) as f32;
            with_float(c, |f| set(f, value))?;
            Ok(0)
        });
    }
    b.method("addFinalValueForMax", |c| {
        let (value, max) = (num_arg(c, 1) as f32, num_arg(c, 2) as f32);
        with_float(c, |f| f.add_for_max(value, max))?;
        Ok(0)
    });
}

#[cfg(test)]
mod tests;
