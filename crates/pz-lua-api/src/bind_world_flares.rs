//! `zombie.iso.weather.WorldFlares` and `WorldFlares$Flare`.

use std::any::Any;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

use pz_core::core::ColorInfo;
use pz_kahlua::value::LuaObject;
use pz_kahlua::{Thread, Value};

use crate::bind_climate::ColorInfoObject;
use crate::bind_core::LuaColorInfo;
use crate::expose::{bool_arg, expose_class, int_arg, num_arg, with_this};

pub const WORLD_FLARES_CLASS: &str = "zombie.iso.weather.WorldFlares";
pub const FLARE_CLASS: &str = "zombie.iso.weather.WorldFlares$Flare";

/// Live state for one launched flare (`WorldFlares$Flare`).
#[derive(Clone, Debug)]
pub struct Flare {
    pub id: i32,
    pub x: f32,
    pub y: f32,
    pub range: i32,
    pub wind_speed: f32,
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub ri: f32,
    pub gi: f32,
    pub bi: f32,
    pub max_life_time: f32,
    pub life_time: f32,
    pub intensity: f32,
    pub percent: f32,
    pub has_launched: bool,
}

impl Flare {
    fn color_climate(&self) -> pz_core::weather::ClimateColorInfo {
        pz_core::weather::ClimateColorInfo {
            exterior: [self.r, self.g, self.b, 1.0],
            interior: [self.ri, self.gi, self.bi, 1.0],
        }
    }
}

pub type FlareRef = Rc<RefCell<Flare>>;

pub struct LuaFlare(pub FlareRef);

impl LuaObject for LuaFlare {
    fn class_name(&self) -> &str {
        FLARE_CLASS
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

thread_local! {
    static FLARES: RefCell<Vec<FlareRef>> = const { RefCell::new(Vec::new()) };
    static NEXT_ID: Cell<i32> = const { Cell::new(0) };
    static DEBUG_DRAW: Cell<bool> = const { Cell::new(false) };
}

/// `WorldFlares.Clear()`.
pub fn clear() {
    FLARES.with(|f| f.borrow_mut().clear());
    NEXT_ID.with(|id| id.set(0));
}

/// `WorldFlares.update()` using elapsed game-time delta (in game minutes / frames).
pub fn update(delta_minutes: f32) {
    if delta_minutes <= 0.0 || !delta_minutes.is_finite() {
        return;
    }
    FLARES.with(|flares| {
        let mut list = flares.borrow_mut();
        for f in list.iter() {
            let mut f = f.borrow_mut();
            f.life_time += delta_minutes;
            let p = if f.max_life_time > 0.0 {
                (f.life_time / f.max_life_time).clamp(0.0, 1.0)
            } else {
                1.0
            };
            f.percent = p;
            f.intensity = (1.0 - p).clamp(0.0, 1.0);
        }
        list.retain(|f| {
            let f = f.borrow();
            f.life_time < f.max_life_time
        });
    });
}

/// Expose `zombie.iso.weather.WorldFlares` and `WorldFlares$Flare`.
pub fn register(thread: &mut Thread) {
    let fl = expose_class(thread, FLARE_CLASS, Some("java.lang.Object"));
    fl.method("getId", |c| {
        let v = with_this::<LuaFlare, _>(c, |f| f.0.borrow().id)?;
        Ok(c.push(v))
    });
    fl.method("getX", |c| {
        let v = with_this::<LuaFlare, _>(c, |f| f.0.borrow().x as f64)?;
        Ok(c.push(v))
    });
    fl.method("getY", |c| {
        let v = with_this::<LuaFlare, _>(c, |f| f.0.borrow().y as f64)?;
        Ok(c.push(v))
    });
    fl.method("getRange", |c| {
        let v = with_this::<LuaFlare, _>(c, |f| f.0.borrow().range)?;
        Ok(c.push(v))
    });
    fl.method("getWindSpeed", |c| {
        let v = with_this::<LuaFlare, _>(c, |f| f.0.borrow().wind_speed as f64)?;
        Ok(c.push(v))
    });
    fl.method("getIntensity", |c| {
        let v = with_this::<LuaFlare, _>(c, |f| f.0.borrow().intensity as f64)?;
        Ok(c.push(v))
    });
    fl.method("getMaxLifeTime", |c| {
        let v = with_this::<LuaFlare, _>(c, |f| f.0.borrow().max_life_time as f64)?;
        Ok(c.push(v))
    });
    fl.method("getLifeTime", |c| {
        let v = with_this::<LuaFlare, _>(c, |f| f.0.borrow().life_time as f64)?;
        Ok(c.push(v))
    });
    fl.method("getPercent", |c| {
        let v = with_this::<LuaFlare, _>(c, |f| f.0.borrow().percent as f64)?;
        Ok(c.push(v))
    });
    fl.method("isHasLaunched", |c| {
        let v = with_this::<LuaFlare, _>(c, |f| f.0.borrow().has_launched)?;
        Ok(c.push(v))
    });
    fl.method("getIntensityPlayer", |c| {
        let v = with_this::<LuaFlare, _>(c, |f| f.0.borrow().intensity as f64)?;
        Ok(c.push(v))
    });
    fl.method("getLerpPlayer", |c| {
        let v = with_this::<LuaFlare, _>(c, |f| f.0.borrow().intensity as f64)?;
        Ok(c.push(v))
    });
    fl.method("getDistModPlayer", |c| {
        let _ = with_this::<LuaFlare, _>(c, |_| ())?;
        Ok(c.push(1.0))
    });
    fl.method("getColor", |c| {
        let cc = with_this::<LuaFlare, _>(c, |f| f.0.borrow().color_climate())?;
        Ok(c.push(Value::Object(Rc::new(ColorInfoObject(Rc::new(
            RefCell::new(cc),
        ))))))
    });
    fl.method("getColorPlayer", |c| {
        let ci = with_this::<LuaFlare, _>(c, |f| {
            let f = f.0.borrow();
            ColorInfo::new(f.r, f.g, f.b, 1.0)
        })?;
        Ok(c.push(Value::Object(LuaColorInfo::new(ci))))
    });
    fl.method("getOutColorPlayer", |c| {
        let ci = with_this::<LuaFlare, _>(c, |f| {
            let f = f.0.borrow();
            ColorInfo::new(f.ri, f.gi, f.bi, 1.0)
        })?;
        Ok(c.push(Value::Object(LuaColorInfo::new(ci))))
    });

    let b = expose_class(thread, WORLD_FLARES_CLASS, Some("java.lang.Object"));
    b.static_fn("getDebugDraw", |c| Ok(c.push(DEBUG_DRAW.with(Cell::get))));
    b.static_fn("setDebugDraw", |c| {
        DEBUG_DRAW.with(|d| d.set(bool_arg(c, 0)));
        Ok(0)
    });
    b.static_fn("Clear", |_| {
        clear();
        Ok(0)
    });
    b.static_fn("launchFlare", |c| {
        let life = (num_arg(c, 0) as f32).max(1.0);
        let x = num_arg(c, 1) as f32;
        let y = num_arg(c, 2) as f32;
        let range = int_arg(c, 3).max(1);
        let wind_speed = num_arg(c, 4) as f32;
        let (r, g, b) = (num_arg(c, 5) as f32, num_arg(c, 6) as f32, num_arg(c, 7) as f32);
        let (ri, gi, bi) = if c.nargs() >= 11 {
            (num_arg(c, 8) as f32, num_arg(c, 9) as f32, num_arg(c, 10) as f32)
        } else {
            (r, g, b)
        };
        let id = NEXT_ID.with(|n| {
            let id = n.get();
            n.set(id + 1);
            id
        });
        let flare = Rc::new(RefCell::new(Flare {
            id,
            x,
            y,
            range,
            wind_speed,
            r,
            g,
            b,
            ri,
            gi,
            bi,
            max_life_time: life,
            life_time: 0.0,
            intensity: 1.0,
            percent: 0.0,
            has_launched: true,
        }));
        FLARES.with(|list| list.borrow_mut().push(flare));
        Ok(0)
    });
    b.static_fn("getFlareCount", |c| {
        let count = FLARES.with(|list| list.borrow().len() as i32);
        Ok(c.push(count))
    });
    b.static_fn("getFlare", |c| {
        let idx = int_arg(c, 0);
        let v = FLARES.with(|list| {
            let list = list.borrow();
            if idx >= 0 && (idx as usize) < list.len() {
                Value::Object(Rc::new(LuaFlare(list[idx as usize].clone())))
            } else {
                Value::Nil
            }
        });
        Ok(c.push(v))
    });
    b.static_fn("getFlareID", |c| {
        let id = int_arg(c, 0);
        let v = FLARES.with(|list| {
            list.borrow()
                .iter()
                .find(|f| f.borrow().id == id)
                .map(|f| Value::Object(Rc::new(LuaFlare(f.clone()))))
                .unwrap_or(Value::Nil)
        });
        Ok(c.push(v))
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use pz_kahlua::stdlib::load_string;
    use pz_kahlua::TableRef;

    #[test]
    fn world_flares_launch_update_and_clear() {
        clear();
        let mut t = Thread::new(TableRef::new());
        crate::bind_core::register(&mut t, None, crate::bind_core::shared_core());
        register(&mut t);
        let f = load_string(
            r#"
            WorldFlares.setDebugDraw(true)
            WorldFlares.launchFlare(60, 100, 200, 30, 0.5, 1.0, 0.2, 0.1, 0.8, 0.1, 0.05)
            local fl = WorldFlares.getFlare(0)
            return WorldFlares.getDebugDraw(), WorldFlares.getFlareCount(), fl:getId(), fl:getX(), fl:getRange()
            "#,
            Some("flares_test.lua"),
            t.globals().clone(),
        )
        .unwrap();
        let res = t.pcall(&f, &[]).unwrap();
        assert!(res[0].truthy());
        assert_eq!(res[1].as_num(), Some(1.0));
        assert_eq!(res[2].as_num(), Some(0.0));
        assert_eq!(res[3].as_num(), Some(100.0));
        assert_eq!(res[4].as_num(), Some(30.0));

        update(30.0);
        FLARES.with(|list| {
            let list = list.borrow();
            assert_eq!(list.len(), 1);
            assert!((list[0].borrow().percent - 0.5).abs() < 1e-4);
        });
        update(35.0);
        FLARES.with(|list| assert!(list.borrow().is_empty()));
    }
}
