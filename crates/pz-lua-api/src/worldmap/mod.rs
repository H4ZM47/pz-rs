//! `zombie.worldMap` for Lua: the `UIWorldMap` element, its
//! `UIWorldMapV1/V2/V3` API objects and the renderer's option objects.
//! The engine-side map lives in the pz-worldmap crate.
//!
//! Inputs from other workstreams come through the traits in [`host`].

pub mod host;
pub mod ui;

use crate::bind_ui::{element_of, UIELEMENT};
use crate::expose::{bool_arg, expose_class, int_arg, num_arg, str_arg};
use pz_core::config::{ConfigObject, OptionKind};
use pz_kahlua::value::LuaObject;
use pz_kahlua::{LuaError, LuaResult, NativeCall, Thread, Value};
use pz_ui::UIElement;
use std::any::Any;
use std::rc::{Rc, Weak};
use ui::{MapState, UIWorldMap, UIWORLDMAP};

pub const UIWORLDMAP_V1: &str = "zombie.worldMap.UIWorldMapV1";
pub const UIWORLDMAP_V2: &str = "zombie.worldMap.UIWorldMapV2";
pub const UIWORLDMAP_V3: &str = "zombie.worldMap.UIWorldMapV3";
const API_CLASSES: [&str; 3] = [UIWORLDMAP_V1, UIWORLDMAP_V2, UIWORLDMAP_V3];
const V2_SUPERS: &[&str] = &[UIWORLDMAP_V1];
const V3_SUPERS: &[&str] = &[UIWORLDMAP_V2, UIWORLDMAP_V1];
const OPTION_SUPERS: &[&str] = &[
    "zombie.config.BooleanConfigOption",
    "zombie.config.ConfigOption",
];

/// `UIWorldMapV1`, `V2` or `V3` (`version` 0, 1, 2) of one map element.
pub struct WorldMapApi {
    el: Weak<UIElement>,
    version: usize,
}

impl LuaObject for WorldMapApi {
    fn class_name(&self) -> &str {
        API_CLASSES[self.version]
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn super_classes(&self) -> &[&'static str] {
        match self.version {
            0 => &[],
            1 => V2_SUPERS,
            _ => V3_SUPERS,
        }
    }
}

/// A `WorldMapRenderer.WorldMapBooleanOption` of one map element.
pub struct WorldMapOption {
    el: Weak<UIElement>,
    index: usize,
}

impl LuaObject for WorldMapOption {
    fn class_name(&self) -> &str {
        pz_worldmap::renderer::BOOLEAN_OPTION_CLASS
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn super_classes(&self) -> &[&'static str] {
        OPTION_SUPERS
    }
}

/// `getAPIvN()`: the element's API object, made once.
pub fn api_value(el: &Rc<UIElement>, version: usize) -> Value {
    let m = el.ext::<UIWorldMap>().unwrap();
    let mut apis = m.apis.borrow_mut();
    let o = apis[version]
        .get_or_insert_with(|| {
            Rc::new(WorldMapApi {
                el: Rc::downgrade(el),
                version,
            }) as Rc<dyn LuaObject>
        })
        .clone();
    Value::Object(o)
}

fn map_el(c: &NativeCall<'_>) -> LuaResult<Rc<UIElement>> {
    let el = match c.arg(0) {
        Value::Object(o) => {
            if let Some(a) = o.as_any().downcast_ref::<WorldMapApi>() {
                a.el.upgrade()
            } else {
                element_of(&Value::Object(o))
            }
        }
        _ => None,
    };
    match el {
        Some(e) if e.ext::<UIWorldMap>().is_some() => Ok(e),
        _ => Err(LuaError::runtime(format!(
            "Expected UIWorldMap but got {}",
            pz_kahlua::stdlib::java_class_name(&c.arg(0))
        ))),
    }
}

/// Run `f` on the element and its map state.
fn with_map<R>(c: &NativeCall<'_>, f: impl FnOnce(&UIElement, &UIWorldMap) -> R) -> LuaResult<R> {
    let el = map_el(c)?;
    let m = el.ext::<UIWorldMap>().unwrap();
    Ok(f(&el, m))
}

fn f32_arg(c: &NativeCall<'_>, i: usize) -> f32 {
    num_arg(c, i) as f32
}

/// Register the world-map classes.
pub fn register(thread: &mut Thread) {
    register_element(thread);
    register_api(thread);
    register_option(thread);
}

fn register_element(thread: &mut Thread) {
    let b = expose_class(thread, UIWORLDMAP, Some(UIELEMENT));
    b.ctor(|c| {
        let table = match c.arg(0) {
            Value::Table(t) => Some(t),
            _ => None,
        };
        let e = UIWorldMap::create(table);
        Ok(c.push(Value::Object(e as Rc<dyn LuaObject>)))
    });
    for (name, version) in [
        ("getAPI", 2),
        ("getAPIv1", 0),
        ("getAPIv2", 1),
        ("getAPIv3", 2),
    ] {
        b.method(name, move |c| {
            let el = map_el(c)?;
            Ok(c.push(api_value(&el, version)))
        });
    }
    b.method("isMapEditor", |c| {
        let v = with_map(c, |_, m| m.st().map_editor)?;
        Ok(c.push(v))
    });
    b.method("setMapEditor", |c| {
        let v = bool_arg(c, 1);
        with_map(c, |_, m| m.st_mut().map_editor = v)?;
        Ok(0)
    });
    b.method("scaleWidthToHeight", |c| {
        with_map(c, |el, m| m.scale_width_to_height(el))?;
        Ok(0)
    });
    b.method("setDoStencil", |c| {
        let v = bool_arg(c, 1);
        with_map(c, |_, m| m.st_mut().do_stencil = v)?;
        Ok(0)
    });
}

/// `resetView()` when the map can show something (`UIWorldMapV1`).
fn reset_view(m: &UIWorldMap) {
    let mut s = m.st_mut();
    let MapState {
        world_map,
        renderer,
        ..
    } = &mut *s;
    if !(world_map.has_data() && world_map.is_data_loaded() || world_map.has_images()) {
        return;
    }
    if !renderer.has_map() {
        return;
    }
    renderer.reset_view(world_map);
}

fn register_api(thread: &mut Thread) {
    let b = expose_class(thread, UIWORLDMAP_V1, None);
    macro_rules! int_get {
        ($name:literal, $f:ident) => {
            b.method($name, |c| {
                let v = with_map(c, |_, m| m.st().world_map.$f())?;
                Ok(c.push(v as f64))
            });
        };
    }
    int_get!("getMinXInCells", get_min_x_in_cells);
    int_get!("getMinYInCells", get_min_y_in_cells);
    int_get!("getMaxXInCells", get_max_x_in_cells);
    int_get!("getMaxYInCells", get_max_y_in_cells);
    int_get!("getWidthInCells", get_width_in_cells);
    int_get!("getHeightInCells", get_height_in_cells);
    int_get!("getMinXInSquares", get_min_x_in_squares);
    int_get!("getMinYInSquares", get_min_y_in_squares);
    int_get!("getMaxXInSquares", get_max_x_in_squares);
    int_get!("getMaxYInSquares", get_max_y_in_squares);
    int_get!("getWidthInSquares", get_width_in_squares);
    int_get!("getHeightInSquares", get_height_in_squares);
    int_get!("getDataCount", get_data_count);
    int_get!("getImagesCount", get_images_count);
    b.method("getDataFileByIndex", |c| {
        let i = int_arg(c, 1);
        let v = with_map(c, |_, m| {
            let s = m.st();
            usize::try_from(i)
                .ok()
                .and_then(|i| s.world_map.get_data_by_index(i))
                .map(|d| d.relative_file_name.borrow().clone())
                .ok_or(s.world_map.get_data_count())
        })?;
        match v {
            Ok(name) => Ok(c.push(Value::Str(name))),
            Err(n) => Err(LuaError::runtime(format!(
                "Index {i} out of bounds for length {n}"
            ))),
        }
    });
    b.method("clearData", |c| {
        with_map(c, |_, m| m.st_mut().world_map.clear_data())?;
        Ok(0)
    });
    b.method("endDirectoryData", |c| {
        with_map(c, |_, m| m.st_mut().world_map.end_directory_data())?;
        Ok(0)
    });
    b.method("setBoundsInCells", |c| {
        let (x0, y0, x1, y1) = (int_arg(c, 1), int_arg(c, 2), int_arg(c, 3), int_arg(c, 4));
        with_map(c, |_, m| {
            let changed = {
                let s = m.st();
                let w = &s.world_map;
                // As the original compares (maxY + 256 rather than * 256).
                x0.wrapping_mul(256) != w.min_x
                    || y0.wrapping_mul(256) != w.min_y
                    || x1.wrapping_mul(256).wrapping_add(255) != w.max_x
                    || y1.wrapping_add(256).wrapping_add(255) != w.max_y
            };
            m.st_mut().world_map.set_bounds_in_cells(x0, y0, x1, y1);
            if changed && m.st().world_map.has_data() {
                reset_view(m);
            }
        })?;
        Ok(0)
    });
    b.method("setBoundsInSquares", |c| {
        let (x0, y0, x1, y1) = (int_arg(c, 1), int_arg(c, 2), int_arg(c, 3), int_arg(c, 4));
        with_map(c, |el, m| {
            let changed = {
                let w = &m.st().world_map;
                x0 != w.min_x || y0 != w.min_y || x1 != w.max_x || y1 != w.max_y
            };
            m.st_mut().world_map.set_bounds_in_squares(x0, y0, x1, y1);
            if !m.st().renderer.has_map() && el.get_width() > 0.0 {
                m.set_map_from(el);
            }
            let has = {
                let w = &m.st().world_map;
                w.has_data() || w.has_images()
            };
            if changed && has {
                reset_view(m);
            }
        })?;
        Ok(0)
    });
    b.method("setBoundsFromWorld", |c| {
        let (x0, y0, x1, y1) = host::with_host(|h| h.meta_grid.cell_bounds())
            .ok_or(LuaError::Runtime {
                message: String::new(),
                class: "java.lang.NullPointerException",
            })?;
        with_map(c, |_, m| {
            m.st_mut().world_map.set_bounds_from_world(x0, y0, x1, y1)
        })?;
        Ok(0)
    });
    b.method("setBoundsFromData", |c| {
        with_map(c, |_, m| m.st_mut().world_map.set_bounds_from_data())?;
        Ok(0)
    });
    b.method("uiToWorldX", |c| {
        let (x, y) = (f32_arg(c, 1), f32_arg(c, 2));
        let v = if c.nargs() >= 6 {
            let (z, cx, cy) = (f32_arg(c, 3), f32_arg(c, 4), f32_arg(c, 5));
            with_map(c, |_, m| {
                let s = m.st();
                let r = &s.renderer;
                r.ui_to_world_mvp(x, y, z, cx, cy, r.get_model_view_projection_matrix())
                    .0
            })?
        } else {
            with_map(c, |_, m| m.ui_to_world_x(x, y))?
        };
        Ok(c.push(v as f64))
    });
    b.method("uiToWorldY", |c| {
        let (x, y) = (f32_arg(c, 1), f32_arg(c, 2));
        let v = if c.nargs() >= 6 {
            let (z, cx, cy) = (f32_arg(c, 3), f32_arg(c, 4), f32_arg(c, 5));
            with_map(c, |_, m| {
                let s = m.st();
                let r = &s.renderer;
                r.ui_to_world_mvp(x, y, z, cx, cy, r.get_model_view_projection_matrix())
                    .1
            })?
        } else {
            with_map(c, |_, m| m.ui_to_world_y(x, y))?
        };
        Ok(c.push(v as f64))
    });
    b.method("worldToUIX", |c| {
        let (x, y) = (f32_arg(c, 1), f32_arg(c, 2));
        let v = with_map(c, |_, m| m.world_to_ui(x, y).0)?;
        Ok(c.push(v as f64))
    });
    b.method("worldToUIY", |c| {
        let (x, y) = (f32_arg(c, 1), f32_arg(c, 2));
        let v = with_map(c, |_, m| m.world_to_ui(x, y).1)?;
        Ok(c.push(v as f64))
    });
    b.method("worldOriginX", |c| {
        let v = with_map(c, |_, m| {
            let r = &m.st().renderer;
            r.world_origin_ui_x(r.get_display_zoom_f(), r.get_center_world_x())
        })?;
        Ok(c.push(v as f64))
    });
    b.method("worldOriginY", |c| {
        let v = with_map(c, |_, m| {
            let r = &m.st().renderer;
            r.world_origin_ui_y(r.get_display_zoom_f(), r.get_center_world_y())
        })?;
        Ok(c.push(v as f64))
    });
    b.method("getBaseZoom", |c| {
        let v = with_map(c, |_, m| {
            let s = m.st();
            s.renderer.get_base_zoom(&s.world_map)
        })?;
        Ok(c.push(v as f64))
    });
    b.method("getZoomF", |c| {
        let v = with_map(c, |_, m| m.st().renderer.get_display_zoom_f())?;
        Ok(c.push(v as f64))
    });
    b.method("getWorldScale", |c| {
        let v = with_map(c, |_, m| {
            let r = &m.st().renderer;
            r.get_world_scale(r.get_display_zoom_f())
        })?;
        Ok(c.push(v as f64))
    });
    b.method("getCenterWorldX", |c| {
        let v = with_map(c, |_, m| m.st().renderer.get_center_world_x())?;
        Ok(c.push(v as f64))
    });
    b.method("getCenterWorldY", |c| {
        let v = with_map(c, |_, m| m.st().renderer.get_center_world_y())?;
        Ok(c.push(v as f64))
    });
    b.method("centerOn", |c| {
        let (x, y) = (f32_arg(c, 1), f32_arg(c, 2));
        with_map(c, |_, m| {
            let mut s = m.st_mut();
            if s.world_map.has_data() || s.world_map.has_images() {
                s.renderer.center_on(x, y);
            }
        })?;
        Ok(0)
    });
    b.method("moveView", |c| {
        let (x, y) = (f32_arg(c, 1), f32_arg(c, 2));
        with_map(c, |_, m| {
            let mut s = m.st_mut();
            if s.world_map.has_data() || s.world_map.has_images() {
                s.renderer.move_view(x as i32, y as i32);
            }
        })?;
        Ok(0)
    });
    b.method("zoomAt", |c| {
        let (x, y, d) = (f32_arg(c, 1), f32_arg(c, 2), f32_arg(c, 3));
        with_map(c, |_, m| {
            let mut s = m.st_mut();
            let MapState {
                world_map,
                renderer,
                ..
            } = &mut *s;
            if world_map.has_data() || world_map.has_images() {
                renderer.zoom_at(world_map, x as i32, y as i32, -(d as i32));
            }
        })?;
        Ok(0)
    });
    b.method("setZoom", |c| {
        let z = f32_arg(c, 1);
        with_map(c, |_, m| {
            let mut s = m.st_mut();
            let MapState {
                world_map,
                renderer,
                ..
            } = &mut *s;
            renderer.set_zoom(world_map, z);
        })?;
        Ok(0)
    });
    b.method("resetView", |c| {
        with_map(c, |_, m| reset_view(m))?;
        Ok(0)
    });
    b.method("mouseToWorldX", |c| {
        let v = with_map(c, |el, m| m.mouse_to_world_x(el))?;
        Ok(c.push(v as f64))
    });
    b.method("mouseToWorldY", |c| {
        let v = with_map(c, |el, m| m.mouse_to_world_y(el))?;
        Ok(c.push(v as f64))
    });
    b.method("setBackgroundRGBA", |c| {
        let col = [f32_arg(c, 1), f32_arg(c, 2), f32_arg(c, 3), f32_arg(c, 4)];
        with_map(c, |_, m| m.st_mut().color = col)?;
        Ok(0)
    });
    b.method("setDropShadowWidth", |c| {
        let w = int_arg(c, 1);
        with_map(c, |_, m| m.st_mut().renderer.set_drop_shadow_width(w))?;
        Ok(0)
    });
    b.method("getOptionCount", |c| {
        let v = with_map(c, |_, m| m.st().renderer.get_option_count())?;
        Ok(c.push(v as f64))
    });
    b.method("getOptionByIndex", |c| {
        let i = int_arg(c, 1);
        let el = map_el(c)?;
        let n = el
            .ext::<UIWorldMap>()
            .unwrap()
            .st()
            .renderer
            .get_option_count();
        match usize::try_from(i).ok().filter(|&i| i < n) {
            Some(index) => Ok(c.push(Value::Object(Rc::new(WorldMapOption {
                el: Rc::downgrade(&el),
                index,
            })))),
            None => Err(LuaError::runtime(format!(
                "Index {i} out of bounds for length {n}"
            ))),
        }
    });
    b.method("setBoolean", |c| {
        let (name, v) = (str_arg(c, 1), bool_arg(c, 2));
        with_map(c, |_, m| {
            if let Some(n) = name {
                m.st_mut().renderer.set_boolean(&n, v);
            }
        })?;
        Ok(0)
    });
    b.method("getBoolean", |c| {
        let name = str_arg(c, 1);
        let v = with_map(c, |_, m| {
            name.is_some_and(|n| m.st().renderer.get_boolean(&n))
        })?;
        Ok(c.push(v))
    });
    b.method("setDouble", |c| {
        let (name, v) = (str_arg(c, 1), num_arg(c, 2));
        with_map(c, |_, m| {
            if let Some(n) = name {
                m.st_mut().renderer.set_double(&n, v);
            }
        })?;
        Ok(0)
    });
    b.method("getDouble", |c| {
        let (name, d) = (str_arg(c, 1), num_arg(c, 2));
        let v = with_map(c, |_, m| {
            name.map_or(d, |n| m.st().renderer.get_double(&n, d))
        })?;
        Ok(c.push(v))
    });

    let b = expose_class(thread, UIWORLDMAP_V2, Some(UIWORLDMAP_V1));
    b.method("isDimUnsharedSymbols", |c| {
        let v = with_map(c, |_, m| m.st().renderer.is_dim_unshared_symbols())?;
        Ok(c.push(v))
    });

    let b = expose_class(thread, UIWORLDMAP_V3, Some(UIWORLDMAP_V2));
    b.method("isDataLoaded", |c| {
        let v = with_map(c, |_, m| m.st().world_map.is_data_loaded())?;
        Ok(c.push(v))
    });
    b.method("getDataWidthInCells", |c| {
        let v = with_map(c, |_, m| m.st().world_map.get_data_width_in_cells())?;
        Ok(c.push(v as f64))
    });
    b.method("getDataHeightInCells", |c| {
        let v = with_map(c, |_, m| m.st().world_map.get_data_height_in_cells())?;
        Ok(c.push(v as f64))
    });
    b.method("clearImages", |c| {
        with_map(c, |_, m| m.st_mut().world_map.clear_images())?;
        Ok(0)
    });
    b.method("setMaxZoom", |c| {
        let z = f32_arg(c, 1);
        with_map(c, |_, m| m.st_mut().renderer.set_max_zoom(z))?;
        Ok(0)
    });
    b.method("getMaxZoom", |c| {
        let v = with_map(c, |_, m| m.st().renderer.get_max_zoom())?;
        Ok(c.push(v as f64))
    });
    b.method("transitionTo", |c| {
        let (x, y, z) = (f32_arg(c, 1), f32_arg(c, 2), f32_arg(c, 3));
        with_map(c, |_, m| {
            let mut s = m.st_mut();
            let MapState {
                world_map,
                renderer,
                ..
            } = &mut *s;
            if !(world_map.has_data() && world_map.is_data_loaded() || world_map.has_images()) {
                return;
            }
            if !renderer.has_map() {
                return;
            }
            renderer.transition_to(world_map, x, y, z, pz_ui::manager::now_ms());
        })?;
        Ok(0)
    });
    b.method("setDisplayedArea", |c| {
        let (x1, y1, x2, y2) = (f32_arg(c, 1), f32_arg(c, 2), f32_arg(c, 3), f32_arg(c, 4));
        with_map(c, |_, m| {
            let mut s = m.st_mut();
            let MapState {
                world_map,
                renderer,
                ..
            } = &mut *s;
            renderer.center_on(x1 + (x2 - x1) / 2.0, y1 + (y2 - y1) / 2.0);
            let (w, h) = (renderer.get_width(), renderer.get_height());
            let zoom = if h <= w {
                let mpp = ((y2 - y1) / h as f32) as f64;
                pz_worldmap::projection::zoom_at_meters_per_pixel(mpp, h as f64)
            } else {
                let mpp = (w as f32 / (x2 - x1)) as f64;
                pz_worldmap::projection::zoom_at_meters_per_pixel(mpp, w as f64)
            };
            renderer.set_zoom(world_map, zoom as f32);
        })?;
        Ok(0)
    });
}

/// Run `f` on the option an option object stands for.
fn with_option<R>(
    c: &NativeCall<'_>,
    f: impl FnOnce(&mut pz_core::config::ConfigOption) -> R,
) -> LuaResult<R> {
    let (el, index) = match c.arg(0) {
        Value::Object(o) => match o.as_any().downcast_ref::<WorldMapOption>() {
            Some(w) => (w.el.upgrade(), w.index),
            None => (None, 0),
        },
        _ => (None, 0),
    };
    let el = el.ok_or_else(|| LuaError::runtime("Expected WorldMapBooleanOption"))?;
    let m = el.ext::<UIWorldMap>().unwrap();
    let mut s = m.st_mut();
    Ok(f(s.renderer.get_option_by_index_mut(index).unwrap()))
}

fn register_option(thread: &mut Thread) {
    let b = expose_class(thread, pz_worldmap::renderer::BOOLEAN_OPTION_CLASS, None);
    b.method("getName", |c| {
        let v = with_option(c, |o| o.name().to_string())?;
        Ok(c.push(v))
    });
    b.method("getType", |c| {
        let v = with_option(c, |o| o.get_type())?;
        Ok(c.push(v))
    });
    b.method("getValue", |c| {
        let v = with_option(c, |o| o.get_bool())?;
        Ok(c.push(v))
    });
    b.method("setValue", |c| {
        let v = bool_arg(c, 1);
        with_option(c, |o| o.set_bool(v))?;
        Ok(0)
    });
    b.method("getDefaultValue", |c| {
        let v = with_option(c, |o| match &o.kind {
            OptionKind::Boolean(b) => b.default_value,
            _ => false,
        })?;
        Ok(c.push(v))
    });
    b.method("getValueAsString", |c| {
        let v = with_option(c, |o| o.get_value_as_string())?;
        Ok(c.push(v))
    });
    b.method("getValueAsLuaString", |c| {
        let v = with_option(c, |o| o.get_value_as_lua_string())?;
        Ok(c.push(v))
    });
    b.method("getTooltip", |c| {
        let v = with_option(c, |o| o.get_tooltip())?;
        Ok(c.push(v))
    });
    b.method("resetToDefault", |c| {
        with_option(c, |o| o.reset_to_default())?;
        Ok(0)
    });
    b.method("setDefaultToCurrentValue", |c| {
        with_option(c, |o| o.set_default_to_current_value())?;
        Ok(0)
    });
    b.method("parse", |c| {
        let s = str_arg(c, 1).unwrap_or_default();
        with_option(c, |o| o.parse(&s))?;
        Ok(0)
    });
    b.method("isValidString", |c| {
        let s = str_arg(c, 1);
        let v = with_option(c, |o| s.is_some_and(|s| o.is_valid_string(&s)))?;
        Ok(c.push(v))
    });
    b.method("getValueAsObject", |c| {
        let v = with_option(c, |o| o.get_bool())?;
        Ok(c.push(v))
    });
    b.method("setValueFromObject", |c| {
        let obj = match c.arg(1) {
            Value::Bool(b) => ConfigObject::Bool(b),
            Value::Num(n) => ConfigObject::Double(n),
            Value::Str(s) => ConfigObject::Str(s.to_string()),
            Value::Nil => ConfigObject::Null,
            other => ConfigObject::Other(pz_kahlua::exec::java_value_of(&other)),
        };
        with_option(c, |o| o.set_value_from_object(&obj))?;
        Ok(0)
    });
}

#[cfg(test)]
mod tests;
