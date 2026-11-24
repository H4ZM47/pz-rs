//! Build 42.22 Lua bindings for:
//! - `zombie.iso.weather.fog.ImprovedFog` (over `pz_iso_render::fog::ImprovedFog`)
//! - `zombie.iso.weather.WorldFlares` & `WorldFlares$Flare`
//! - `zombie.iso.FishSchoolManager`

use std::any::Any;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use pz_iso_render::fog::ImprovedFog;
use pz_kahlua::value::LuaObject;
use pz_kahlua::{NativeCall, Thread, Value};

use crate::expose::expose_class;

const IMPROVED_FOG_CLASS: &str = "zombie.iso.weather.fog.ImprovedFog";
const WORLD_FLARES_CLASS: &str = "zombie.iso.weather.WorldFlares";
const FLARE_CLASS: &str = "zombie.iso.weather.WorldFlares$Flare";
const FISH_SCHOOL_CLASS: &str = "zombie.iso.FishSchoolManager";

#[derive(Clone, Debug)]
struct FogState {
    fog: ImprovedFog,
    min_x_offset: i32,
    max_x_offset: i32,
    max_y_offset: i32,
    render_end_only: bool,
    alpha_circle_alpha: f32,
    alpha_circle_rad: f32,
    enable_editing: bool,
    draw_debug_colors: bool,
    render_only_one_row: bool,
    render_current_layer_only: bool,
    render_x_rows_from_center: i32,
}

impl Default for FogState {
    fn default() -> Self {
        Self {
            fog: ImprovedFog::default(),
            min_x_offset: -2,
            max_x_offset: 12,
            max_y_offset: -5,
            render_end_only: false,
            alpha_circle_alpha: 0.325,
            alpha_circle_rad: 2.25,
            enable_editing: false,
            draw_debug_colors: false,
            render_only_one_row: false,
            render_current_layer_only: false,
            render_x_rows_from_center: 0,
        }
    }
}

#[derive(Clone, Debug)]
struct FlareData {
    id: i32,
    x: f32,
    y: f32,
    range: i32,
    wind_speed: f32,
    has_launched: bool,
    intensity: f32,
    max_life_time: f32,
    life_time: f32,
    percent: f32,
    _r: f32,
    _g: f32,
    _b: f32,
    _ri: f32,
    _gi: f32,
    _bi: f32,
}

#[derive(Default)]
struct FlaresState {
    flares: Vec<Rc<RefCell<FlareData>>>,
    next_id: i32,
    debug_draw: bool,
}

#[derive(Clone, Debug)]
struct FishZoneData {
    abundance: f64,
    trash: f64,
    noise: i32,
    chum: i32,
}

impl Default for FishZoneData {
    fn default() -> Self {
        Self {
            abundance: 0.5,
            trash: 0.1,
            noise: 0,
            chum: 0,
        }
    }
}

#[derive(Default)]
struct FishSchoolState {
    seed: i64,
    zones: HashMap<(i32, i32), FishZoneData>,
}

impl FishSchoolState {
    fn zone_key(x: i32, y: i32) -> (i32, i32) {
        (x.div_euclid(20), y.div_euclid(20))
    }

    fn base_abundance(&self, x: i32, y: i32) -> f64 {
        let (zx, zy) = Self::zone_key(x, y);
        let h = (zx as i64)
            .wrapping_mul(374_761_393)
            .wrapping_add((zy as i64).wrapping_mul(668_265_263))
            .wrapping_add(self.seed.wrapping_mul(1_274_126_177));
        let mixed = ((h ^ (h >> 13)).wrapping_mul(1_274_126_177)) as u64;
        0.25 + ((mixed % 500) as f64 / 1000.0)
    }

    fn init(&mut self) {
        self.zones.clear();
        if self.seed == 0 {
            self.seed = 1;
        }
    }

    fn generate_seed(&mut self) {
        self.seed = self.seed.wrapping_add(1).max(1);
    }

    fn add_sound_noise(&mut self, x: i32, y: i32, radius: i32) {
        if radius <= 0 {
            return;
        }
        let key = Self::zone_key(x, y);
        let base = self.base_abundance(x, y);
        let entry = self.zones.entry(key).or_insert_with(|| FishZoneData {
            abundance: base,
            ..Default::default()
        });
        entry.noise = entry.noise.saturating_add(radius);
        entry.abundance = (entry.abundance - (radius as f64) * 0.01).clamp(0.0, 1.0);
    }

    fn add_chum(&mut self, x: i32, y: i32, amount: i32) {
        if amount <= 0 {
            return;
        }
        let key = Self::zone_key(x, y);
        let base = self.base_abundance(x, y);
        let entry = self.zones.entry(key).or_insert_with(|| FishZoneData {
            abundance: base,
            ..Default::default()
        });
        entry.chum = entry.chum.saturating_add(amount);
        entry.abundance = (entry.abundance + (amount as f64) * 0.05).clamp(0.0, 1.0);
    }

    fn catch_fish(&mut self, x: i32, y: i32) {
        let key = Self::zone_key(x, y);
        let base = self.base_abundance(x, y);
        let entry = self.zones.entry(key).or_insert_with(|| FishZoneData {
            abundance: base,
            ..Default::default()
        });
        entry.abundance = (entry.abundance - 0.1).clamp(0.0, 1.0);
    }

    fn fish_abundance(&self, x: i32, y: i32) -> f64 {
        let key = Self::zone_key(x, y);
        self.zones
            .get(&key)
            .map(|z| z.abundance)
            .unwrap_or_else(|| self.base_abundance(x, y))
    }

    fn trash_abundance(&self, x: i32, y: i32) -> f64 {
        let key = Self::zone_key(x, y);
        self.zones.get(&key).map(|z| z.trash).unwrap_or(0.1)
    }
}

thread_local! {
    static FOG: RefCell<FogState> = RefCell::new(FogState::default());
    static FLARES: RefCell<FlaresState> = RefCell::new(FlaresState::default());
    static FISH_SCHOOL: RefCell<FishSchoolState> = RefCell::new(FishSchoolState::default());
}

/// `ImprovedFog.init()` called from `IsoWorld::init`.
pub fn init_improved_fog(fog_intensity: f32) {
    FOG.with(|f| f.borrow_mut().fog.init(fog_intensity));
}

/// Snapshot of the current `ImprovedFog` state.
pub fn current_improved_fog() -> ImprovedFog {
    FOG.with(|f| f.borrow().fog.clone())
}

/// `FishSchoolManager.getInstance().init()` called from `IsoWorld::init`.
pub fn init_fish_school_manager() {
    FISH_SCHOOL.with(|f| f.borrow_mut().init());
}

/// `FishSchoolManager.getInstance().addSoundNoise(x, y, radius)`.
pub fn fish_add_sound_noise(x: i32, y: i32, radius: i32) {
    FISH_SCHOOL.with(|f| f.borrow_mut().add_sound_noise(x, y, radius));
}

/// Reset session state on world teardown.
pub fn reset() {
    FOG.with(|f| *f.borrow_mut() = FogState::default());
    FLARES.with(|f| *f.borrow_mut() = FlaresState::default());
    FISH_SCHOOL.with(|f| *f.borrow_mut() = FishSchoolState::default());
}

#[derive(Debug)]
struct FlareObj(Rc<RefCell<FlareData>>);

impl LuaObject for FlareObj {
    fn class_name(&self) -> &'static str {
        FLARE_CLASS
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

#[derive(Debug)]
struct FishSchoolHandle;

impl LuaObject for FishSchoolHandle {
    fn class_name(&self) -> &'static str {
        FISH_SCHOOL_CLASS
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

fn f32_arg(c: &NativeCall<'_>, idx: usize) -> f32 {
    c.arg(idx).as_num().unwrap_or(0.0) as f32
}

fn i32_arg(c: &NativeCall<'_>, idx: usize) -> i32 {
    c.arg(idx).as_num().unwrap_or(0.0) as i32
}

fn bool_arg(c: &NativeCall<'_>, idx: usize) -> bool {
    matches!(c.arg(idx), Value::Bool(true))
}

fn with_flare<R>(c: &NativeCall<'_>, f: impl FnOnce(&FlareData) -> R) -> Option<R> {
    let Value::Object(o) = c.arg(0) else {
        return None;
    };
    let fl = o.as_any().downcast_ref::<FlareObj>()?;
    let res = Some(f(&fl.0.borrow()));
    res
}

/// Register `ImprovedFog`, `WorldFlares`, `WorldFlares$Flare`, and
/// `FishSchoolManager`.
pub fn register(thread: &mut Thread) {
    let fog = expose_class(thread, IMPROVED_FOG_CLASS, Some("java.lang.Object"));
    fog.field("MAX_FOG_Z", Value::Num(1.0));
    fog.static_fn("init", |_| {
        let intensity = crate::bind_climate::shared_game_time()
            .borrow()
            .climate
            .get_fog_intensity();
        init_improved_fog(intensity);
        Ok(0)
    });
    fog.static_fn("update", |_| Ok(0));
    fog.static_fn("updateKeys", |_| Ok(0));
    fog.static_fn("startRender", |c| Ok(c.push(false)));
    fog.static_fn("renderRowsBehind", |_| Ok(0));
    fog.static_fn("endRender", |_| Ok(0));
    fog.static_fn("getMinXOffset", |c| {
        Ok(c.push(FOG.with(|f| f.borrow().min_x_offset) as f64))
    });
    fog.static_fn("setMinXOffset", |c| {
        FOG.with(|f| f.borrow_mut().min_x_offset = i32_arg(c, 0));
        Ok(0)
    });
    fog.static_fn("getMaxXOffset", |c| {
        Ok(c.push(FOG.with(|f| f.borrow().max_x_offset) as f64))
    });
    fog.static_fn("setMaxXOffset", |c| {
        FOG.with(|f| f.borrow_mut().max_x_offset = i32_arg(c, 0));
        Ok(0)
    });
    fog.static_fn("getMaxYOffset", |c| {
        Ok(c.push(FOG.with(|f| f.borrow().max_y_offset) as f64))
    });
    fog.static_fn("setMaxYOffset", |c| {
        FOG.with(|f| f.borrow_mut().max_y_offset = i32_arg(c, 0));
        Ok(0)
    });
    fog.static_fn("isRenderEndOnly", |c| {
        Ok(c.push(FOG.with(|f| f.borrow().render_end_only)))
    });
    fog.static_fn("setRenderEndOnly", |c| {
        FOG.with(|f| f.borrow_mut().render_end_only = bool_arg(c, 0));
        Ok(0)
    });
    fog.static_fn("getAlphaCircleAlpha", |c| {
        Ok(c.push(f64::from(FOG.with(|f| f.borrow().alpha_circle_alpha))))
    });
    fog.static_fn("setAlphaCircleAlpha", |c| {
        FOG.with(|f| f.borrow_mut().alpha_circle_alpha = f32_arg(c, 0));
        Ok(0)
    });
    fog.static_fn("getAlphaCircleRad", |c| {
        Ok(c.push(f64::from(FOG.with(|f| f.borrow().alpha_circle_rad))))
    });
    fog.static_fn("setAlphaCircleRad", |c| {
        FOG.with(|f| f.borrow_mut().alpha_circle_rad = f32_arg(c, 0));
        Ok(0)
    });
    fog.static_fn("isHighQuality", |c| {
        Ok(c.push(FOG.with(|f| f.borrow().fog.high_quality)))
    });
    fog.static_fn("setHighQuality", |c| {
        FOG.with(|f| f.borrow_mut().fog.high_quality = bool_arg(c, 0));
        Ok(0)
    });
    fog.static_fn("isEnableEditing", |c| {
        Ok(c.push(FOG.with(|f| f.borrow().enable_editing)))
    });
    fog.static_fn("setEnableEditing", |c| {
        FOG.with(|f| f.borrow_mut().enable_editing = bool_arg(c, 0));
        Ok(0)
    });
    fog.static_fn("getTopAlphaHeight", |c| {
        Ok(c.push(f64::from(FOG.with(|f| f.borrow().fog.top_alpha_height))))
    });
    fog.static_fn("setTopAlphaHeight", |c| {
        FOG.with(|f| f.borrow_mut().fog.top_alpha_height = f32_arg(c, 0));
        Ok(0)
    });
    fog.static_fn("getBottomAlphaHeight", |c| {
        Ok(c.push(f64::from(FOG.with(|f| f.borrow().fog.bottom_alpha_height))))
    });
    fog.static_fn("setBottomAlphaHeight", |c| {
        FOG.with(|f| f.borrow_mut().fog.bottom_alpha_height = f32_arg(c, 0));
        Ok(0)
    });
    fog.static_fn("isDrawDebugColors", |c| {
        Ok(c.push(FOG.with(|f| f.borrow().draw_debug_colors)))
    });
    fog.static_fn("setDrawDebugColors", |c| {
        FOG.with(|f| f.borrow_mut().draw_debug_colors = bool_arg(c, 0));
        Ok(0)
    });
    fog.static_fn("getOctaves", |c| {
        Ok(c.push(f64::from(FOG.with(|f| f.borrow().fog.octaves))))
    });
    fog.static_fn("setOctaves", |c| {
        FOG.with(|f| f.borrow_mut().fog.octaves = f32_arg(c, 0));
        Ok(0)
    });
    fog.static_fn("getColorR", |c| {
        Ok(c.push(f64::from(FOG.with(|f| f.borrow().fog.color[0]))))
    });
    fog.static_fn("setColorR", |c| {
        FOG.with(|f| f.borrow_mut().fog.color[0] = f32_arg(c, 0));
        Ok(0)
    });
    fog.static_fn("getColorG", |c| {
        Ok(c.push(f64::from(FOG.with(|f| f.borrow().fog.color[1]))))
    });
    fog.static_fn("setColorG", |c| {
        FOG.with(|f| f.borrow_mut().fog.color[1] = f32_arg(c, 0));
        Ok(0)
    });
    fog.static_fn("getColorB", |c| {
        Ok(c.push(f64::from(FOG.with(|f| f.borrow().fog.color[2]))))
    });
    fog.static_fn("setColorB", |c| {
        FOG.with(|f| f.borrow_mut().fog.color[2] = f32_arg(c, 0));
        Ok(0)
    });
    fog.static_fn("getSecondLayerAlpha", |c| {
        Ok(c.push(f64::from(FOG.with(|f| f.borrow().fog.second_layer_alpha))))
    });
    fog.static_fn("setSecondLayerAlpha", |c| {
        FOG.with(|f| f.borrow_mut().fog.second_layer_alpha = f32_arg(c, 0));
        Ok(0)
    });
    fog.static_fn("getScalingX", |c| {
        Ok(c.push(f64::from(FOG.with(|f| f.borrow().fog.scaling_x))))
    });
    fog.static_fn("setScalingX", |c| {
        FOG.with(|f| f.borrow_mut().fog.scaling_x = f32_arg(c, 0));
        Ok(0)
    });
    fog.static_fn("getScalingY", |c| {
        Ok(c.push(f64::from(FOG.with(|f| f.borrow().fog.scaling_y))))
    });
    fog.static_fn("setScalingY", |c| {
        FOG.with(|f| f.borrow_mut().fog.scaling_y = f32_arg(c, 0));
        Ok(0)
    });
    fog.static_fn("isRenderOnlyOneRow", |c| {
        Ok(c.push(FOG.with(|f| f.borrow().render_only_one_row)))
    });
    fog.static_fn("setRenderOnlyOneRow", |c| {
        FOG.with(|f| f.borrow_mut().render_only_one_row = bool_arg(c, 0));
        Ok(0)
    });
    fog.static_fn("getBaseAlpha", |c| {
        Ok(c.push(f64::from(FOG.with(|f| f.borrow().fog.base_alpha))))
    });
    fog.static_fn("setBaseAlpha", |c| {
        FOG.with(|f| f.borrow_mut().fog.base_alpha = f32_arg(c, 0));
        Ok(0)
    });
    fog.static_fn("getRenderEveryXRow", |c| {
        Ok(c.push(FOG.with(|f| f.borrow().fog.render_every_x_row) as f64))
    });
    fog.static_fn("setRenderEveryXRow", |c| {
        FOG.with(|f| f.borrow_mut().fog.render_every_x_row = i32_arg(c, 0));
        Ok(0)
    });
    fog.static_fn("isRenderCurrentLayerOnly", |c| {
        Ok(c.push(FOG.with(|f| f.borrow().render_current_layer_only)))
    });
    fog.static_fn("setRenderCurrentLayerOnly", |c| {
        FOG.with(|f| f.borrow_mut().render_current_layer_only = bool_arg(c, 0));
        Ok(0)
    });
    fog.static_fn("getRenderXRowsFromCenter", |c| {
        Ok(c.push(FOG.with(|f| f.borrow().render_x_rows_from_center) as f64))
    });
    fog.static_fn("setRenderXRowsFromCenter", |c| {
        FOG.with(|f| f.borrow_mut().render_x_rows_from_center = i32_arg(c, 0));
        Ok(0)
    });
    fog.static_fn("getNoiseTexture", |c| Ok(c.push(Value::Nil)));

    let fl = expose_class(thread, FLARE_CLASS, Some("java.lang.Object"));
    fl.method("getId", |c| Ok(c.push(with_flare(c, |f| f.id).unwrap_or(0) as f64)));
    fl.method("getX", |c| Ok(c.push(f64::from(with_flare(c, |f| f.x).unwrap_or(0.0)))));
    fl.method("getY", |c| Ok(c.push(f64::from(with_flare(c, |f| f.y).unwrap_or(0.0)))));
    fl.method("getRange", |c| Ok(c.push(with_flare(c, |f| f.range).unwrap_or(0) as f64)));
    fl.method("getWindSpeed", |c| {
        Ok(c.push(f64::from(with_flare(c, |f| f.wind_speed).unwrap_or(0.0))))
    });
    fl.method("isHasLaunched", |c| {
        Ok(c.push(with_flare(c, |f| f.has_launched).unwrap_or(false)))
    });
    fl.method("getIntensity", |c| {
        Ok(c.push(f64::from(with_flare(c, |f| f.intensity).unwrap_or(0.0))))
    });
    fl.method("getMaxLifeTime", |c| {
        Ok(c.push(f64::from(with_flare(c, |f| f.max_life_time).unwrap_or(0.0))))
    });
    fl.method("getLifeTime", |c| {
        Ok(c.push(f64::from(with_flare(c, |f| f.life_time).unwrap_or(0.0))))
    });
    fl.method("getPercent", |c| {
        Ok(c.push(f64::from(with_flare(c, |f| f.percent).unwrap_or(0.0))))
    });
    fl.method("getIntensityPlayer", |c| {
        Ok(c.push(f64::from(with_flare(c, |f| f.intensity).unwrap_or(0.0))))
    });
    fl.method("getLerpPlayer", |c| Ok(c.push(0.0)));
    fl.method("getDistModPlayer", |c| Ok(c.push(0.0)));

    let flares = expose_class(thread, WORLD_FLARES_CLASS, Some("java.lang.Object"));
    flares.field("ENABLED", Value::Bool(true));
    flares.static_fn("Clear", |_| {
        FLARES.with(|f| f.borrow_mut().flares.clear());
        Ok(0)
    });
    flares.static_fn("getFlareCount", |c| {
        let n = FLARES.with(|f| f.borrow().flares.len());
        Ok(c.push(n as f64))
    });
    flares.static_fn("getFlare", |c| {
        let idx = i32_arg(c, 0);
        let out = FLARES.with(|f| {
            if idx < 0 {
                return None;
            }
            f.borrow()
                .flares
                .get(idx as usize)
                .cloned()
                .map(|rc| Value::Object(Rc::new(FlareObj(rc))))
        });
        Ok(c.push(out.unwrap_or(Value::Nil)))
    });
    flares.static_fn("getFlareID", |c| {
        let id = i32_arg(c, 0);
        let out = FLARES.with(|f| {
            f.borrow()
                .flares
                .iter()
                .find(|fl| fl.borrow().id == id)
                .cloned()
                .map(|rc| Value::Object(Rc::new(FlareObj(rc))))
        });
        Ok(c.push(out.unwrap_or(Value::Nil)))
    });
    flares.static_fn("launchFlare", |c| {
        let lifetime = f32_arg(c, 0).max(0.0);
        let x = i32_arg(c, 1) as f32;
        let y = i32_arg(c, 2) as f32;
        let range = i32_arg(c, 3);
        let wind_speed = f32_arg(c, 4);
        let _r = f32_arg(c, 5);
        let _g = f32_arg(c, 6);
        let _b = f32_arg(c, 7);
        let _ri = f32_arg(c, 8);
        let _gi = f32_arg(c, 9);
        let _bi = f32_arg(c, 10);
        FLARES.with(|f| {
            let mut state = f.borrow_mut();
            state.next_id += 1;
            let id = state.next_id;
            state.flares.push(Rc::new(RefCell::new(FlareData {
                id,
                x,
                y,
                range,
                wind_speed,
                has_launched: true,
                intensity: 1.0,
                max_life_time: lifetime,
                life_time: 0.0,
                percent: 0.0,
                _r,
                _g,
                _b,
                _ri,
                _gi,
                _bi,
            })));
        });
        Ok(0)
    });
    flares.static_fn("update", |_| {
        FLARES.with(|f| {
            let mut state = f.borrow_mut();
            state.flares.retain(|fl| {
                let mut fl = fl.borrow_mut();
                fl.life_time += 1.0 / 60.0;
                if fl.max_life_time > 0.0 {
                    fl.percent = (fl.life_time / fl.max_life_time).clamp(0.0, 1.0);
                    fl.intensity = 1.0 - fl.percent;
                    fl.life_time < fl.max_life_time
                } else {
                    false
                }
            });
        });
        Ok(0)
    });
    flares.static_fn("setDebugDraw", |c| {
        FLARES.with(|f| f.borrow_mut().debug_draw = bool_arg(c, 0));
        Ok(0)
    });
    flares.static_fn("getDebugDraw", |c| {
        Ok(c.push(FLARES.with(|f| f.borrow().debug_draw)))
    });
    flares.static_fn("debugRender", |_| Ok(0));

    let fish = expose_class(thread, FISH_SCHOOL_CLASS, Some("java.lang.Object"));
    fish.static_fn("getInstance", |c| {
        Ok(c.push(Value::Object(Rc::new(FishSchoolHandle))))
    });
    fish.method("init", |_| {
        init_fish_school_manager();
        Ok(0)
    });
    fish.method("generateSeed", |_| {
        FISH_SCHOOL.with(|f| f.borrow_mut().generate_seed());
        Ok(0)
    });
    fish.method("updateSeed", |_| {
        FISH_SCHOOL.with(|f| f.borrow_mut().generate_seed());
        Ok(0)
    });
    fish.method("update", |_| Ok(0));
    fish.method("updateFishingData", |_| Ok(0));
    fish.method("addSoundNoise", |c| {
        fish_add_sound_noise(i32_arg(c, 1), i32_arg(c, 2), i32_arg(c, 3));
        Ok(0)
    });
    fish.method("addChum", |c| {
        FISH_SCHOOL.with(|f| {
            f.borrow_mut()
                .add_chum(i32_arg(c, 1), i32_arg(c, 2), i32_arg(c, 3));
        });
        Ok(0)
    });
    fish.method("catchFish", |c| {
        FISH_SCHOOL.with(|f| f.borrow_mut().catch_fish(i32_arg(c, 1), i32_arg(c, 2)));
        Ok(0)
    });
    fish.method("getFishAbundance", |c| {
        let v = FISH_SCHOOL.with(|f| f.borrow().fish_abundance(i32_arg(c, 1), i32_arg(c, 2)));
        Ok(c.push(v))
    });
    fish.method("getTrashAbundance", |c| {
        let v = FISH_SCHOOL.with(|f| f.borrow().trash_abundance(i32_arg(c, 1), i32_arg(c, 2)));
        Ok(c.push(v))
    });
    fish.method("load", |_| Ok(0));
    fish.method("save", |_| Ok(0));
}

#[cfg(test)]
mod tests {
    use super::*;
    use pz_kahlua::TableRef;

    #[test]
    fn improved_fog_world_flares_and_fish_school_round_trip() {
        reset();
        let mut t = Thread::new(TableRef::new());
        pz_kahlua::stdlib::install(&mut t, None).unwrap();
        register(&mut t);

        let f = pz_kahlua::stdlib::load_string(
            r#"
            local function assert(v, msg) if not v then error(msg or "assertion failed") end end

            assert(ImprovedFog.MAX_FOG_Z == 1)
            ImprovedFog.setBaseAlpha(0.75)
            assert(math.abs(ImprovedFog.getBaseAlpha() - 0.75) < 0.001)
            ImprovedFog.setHighQuality(false)
            assert(ImprovedFog.isHighQuality() == false)
            ImprovedFog.setOctaves(4.0)
            assert(math.abs(ImprovedFog.getOctaves() - 4.0) < 0.001)

            assert(WorldFlares.getFlareCount() == 0)
            WorldFlares.launchFlare(5.0, 100, 200, 30, 0.5, 1.0, 0.2, 0.2, 1.0, 0.2, 0.2)
            assert(WorldFlares.getFlareCount() == 1)
            local fl = WorldFlares.getFlare(0)
            assert(fl ~= nil and fl:getX() == 100 and fl:getY() == 200 and fl:getRange() == 30)
            assert(WorldFlares.getFlareID(fl:getId()) ~= nil)
            WorldFlares.setDebugDraw(true)
            assert(WorldFlares.getDebugDraw() == true)
            WorldFlares.Clear()
            assert(WorldFlares.getFlareCount() == 0)

            local fish = FishSchoolManager.getInstance()
            fish:init()
            local before = fish:getFishAbundance(10500, 9500)
            fish:addChum(10500, 9500, 2)
            local after_chum = fish:getFishAbundance(10500, 9500)
            assert(after_chum >= before)
            fish:catchFish(10500, 9500)
            assert(fish:getFishAbundance(10500, 9500) <= after_chum)
            assert(fish:getTrashAbundance(10500, 9500) >= 0)
            "#,
            Some("fog_flares_fish_test"),
            t.globals().clone(),
        )
        .unwrap();
        t.pcall(&f, &[])
            .unwrap_or_else(|e| panic!("lua error: {e:?}"));
        reset();
    }
}
