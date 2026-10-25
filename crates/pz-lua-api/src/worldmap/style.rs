//! `WorldMapStyleV1`/`WorldMapStyleV2` (the `getStyleAPI()` objects) and
//! their layer objects (`WorldMapStyleLayerV1` and subclasses) over the
//! map's [`pz_worldmap::style::WorldMapStyle`].

use super::ui::UIWorldMap;
use crate::expose::{bool_arg, expose_class, int_arg, num_arg, str_arg, ClassBuilder};
use pz_kahlua::value::LuaObject;
use pz_kahlua::{LuaError, LuaResult, NativeCall, Thread, Value};
use pz_ui::UIElement;
use pz_worldmap::style::{
    ColorStop, Filter, FloatStop, LayerKind, LayerRef, StyleLayer, TextureScaling, TextureStop,
};
use std::any::Any;
use std::cell::RefCell;
use std::rc::{Rc, Weak};

pub const STYLE_V1: &str = "zombie.worldMap.styles.WorldMapStyleV1";
pub const STYLE_V2: &str = "zombie.worldMap.styles.WorldMapStyleV2";
pub const LAYER_V1: &str = "zombie.worldMap.styles.WorldMapStyleV1$WorldMapStyleLayerV1";
pub const LINE_LAYER_V1: &str = "zombie.worldMap.styles.WorldMapStyleV1$WorldMapLineStyleLayerV1";
pub const POLYGON_LAYER_V1: &str =
    "zombie.worldMap.styles.WorldMapStyleV1$WorldMapPolygonStyleLayerV1";
pub const TEXTURE_LAYER_V1: &str =
    "zombie.worldMap.styles.WorldMapStyleV1$WorldMapTextureStyleLayerV1";
pub const PYRAMID_LAYER_V1: &str =
    "zombie.worldMap.styles.WorldMapStyleV2$WorldMapPyramidStyleLayerV1";
pub const TEXT_LAYER_V1: &str = "zombie.worldMap.styles.WorldMapStyleV2$WorldMapTextStyleLayerV1";
const LAYER_SUPERS: &[&str] = &[LAYER_V1];
const STYLE_V2_SUPERS: &[&str] = &[STYLE_V1];

/// `WorldMapStyleV1` (`v2` false) or `WorldMapStyleV2` of one map.
pub struct StyleApi {
    pub(super) el: Weak<UIElement>,
    pub(super) v2: bool,
}

impl LuaObject for StyleApi {
    fn class_name(&self) -> &str {
        if self.v2 {
            STYLE_V2
        } else {
            STYLE_V1
        }
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn super_classes(&self) -> &[&'static str] {
        if self.v2 {
            STYLE_V2_SUPERS
        } else {
            &[]
        }
    }
}

/// A layer object: the class the owning style API wraps the layer in.
pub struct LayerObj {
    el: Weak<UIElement>,
    pub layer: LayerRef,
    v2: bool,
}

impl LuaObject for LayerObj {
    fn class_name(&self) -> &str {
        match self.layer.borrow().kind {
            LayerKind::Line { .. } => LINE_LAYER_V1,
            LayerKind::Polygon { .. } => POLYGON_LAYER_V1,
            LayerKind::Texture { .. } => TEXTURE_LAYER_V1,
            LayerKind::Pyramid { .. } if self.v2 => PYRAMID_LAYER_V1,
            LayerKind::Text { .. } if self.v2 => TEXT_LAYER_V1,
            _ => LAYER_V1,
        }
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn super_classes(&self) -> &[&'static str] {
        if self.class_name() == LAYER_V1 {
            &[]
        } else {
            LAYER_SUPERS
        }
    }
}

fn style_api(c: &NativeCall<'_>) -> LuaResult<(Rc<UIElement>, bool)> {
    match c.arg(0) {
        Value::Object(o) => match o.as_any().downcast_ref::<StyleApi>() {
            Some(s) => {
                s.el.upgrade()
                    .map(|e| (e, s.v2))
                    .ok_or_else(|| LuaError::runtime("map is gone"))
            }
            None => Err(LuaError::runtime(format!(
                "Expected WorldMapStyleV1 but got {}",
                o.class_name()
            ))),
        },
        other => Err(LuaError::runtime(format!(
            "Expected WorldMapStyleV1 but got {}",
            pz_kahlua::stdlib::java_class_name(&other)
        ))),
    }
}

fn layer_value(el: &Rc<UIElement>, layer: LayerRef, v2: bool) -> Value {
    Value::Object(Rc::new(LayerObj {
        el: Rc::downgrade(el),
        layer,
        v2,
    }))
}

fn layer_obj(c: &NativeCall<'_>) -> LuaResult<(Weak<UIElement>, LayerRef)> {
    match c.arg(0) {
        Value::Object(o) => match o.as_any().downcast_ref::<LayerObj>() {
            Some(l) => Ok((l.el.clone(), l.layer.clone())),
            None => Err(LuaError::runtime(format!(
                "Expected WorldMapStyleLayerV1 but got {}",
                o.class_name()
            ))),
        },
        other => Err(LuaError::runtime(format!(
            "Expected WorldMapStyleLayerV1 but got {}",
            pz_kahlua::stdlib::java_class_name(&other)
        ))),
    }
}

fn with_layer<R>(c: &NativeCall<'_>, f: impl FnOnce(&mut StyleLayer) -> R) -> LuaResult<R> {
    let (_, l) = layer_obj(c)?;
    let mut b = l.borrow_mut();
    Ok(f(&mut b))
}

fn oob(i: i32, n: usize) -> LuaError {
    LuaError::Runtime {
        message: format!("Index {i} out of bounds for length {n}"),
        class: "java.lang.IndexOutOfBoundsException",
    }
}

fn idx(i: i32, n: usize) -> LuaResult<usize> {
    usize::try_from(i)
        .ok()
        .filter(|&i| i < n)
        .ok_or_else(|| oob(i, n))
}

/// `ArrayList.remove(from)` then `add(to, e)`.
fn move_in<T>(v: &mut Vec<T>, from: i32, to: i32) -> LuaResult<()> {
    let f = idx(from, v.len())?;
    let e = v.remove(f);
    let t = usize::try_from(to).ok().filter(|&t| t <= v.len());
    match t {
        Some(t) => {
            v.insert(t, e);
            Ok(())
        }
        None => {
            v.insert(f, e);
            Err(oob(to, v.len() - 1))
        }
    }
}

fn zoom_clamp(z: f32) -> f32 {
    pz_worldmap::renderer::clamp(z, 0.0, 24.0)
}

fn color_stop(c: &NativeCall<'_>) -> ColorStop {
    ColorStop {
        zoom: num_arg(c, 1) as f32,
        r: int_arg(c, 2),
        g: int_arg(c, 3),
        b: int_arg(c, 4),
        a: int_arg(c, 5),
    }
}

/// The map's style and the change notifications for its API objects.
fn add_layer(el: &Rc<UIElement>, layer: StyleLayer, v2: bool) -> Value {
    let m = el.ext::<UIWorldMap>().unwrap();
    let l = Rc::new(RefCell::new(layer));
    m.st_mut().style.add_layer(l.clone());
    layer_value(el, l, v2)
}

/// `getStyleAPI()` of `UIWorldMapV1` (`v2` false) or `V3`.
pub fn style_api_value(el: &Rc<UIElement>, v2: bool) -> Value {
    let m = el.ext::<UIWorldMap>().unwrap();
    let mut cache = m.style_apis.borrow_mut();
    let o = cache[v2 as usize]
        .get_or_insert_with(|| {
            Rc::new(StyleApi {
                el: Rc::downgrade(el),
                v2,
            }) as Rc<dyn LuaObject>
        })
        .clone();
    Value::Object(o)
}

pub fn register(thread: &mut Thread) {
    let b = expose_class(thread, STYLE_V1, None);
    b.method("newLineLayer", |c| {
        let (el, v2) = style_api(c)?;
        let id = str_arg(c, 1).unwrap_or_default();
        Ok(c.push(add_layer(&el, StyleLayer::new_line(&id), v2)))
    });
    b.method("newPolygonLayer", |c| {
        let (el, v2) = style_api(c)?;
        let id = str_arg(c, 1).unwrap_or_default();
        Ok(c.push(add_layer(&el, StyleLayer::new_polygon(&id), v2)))
    });
    b.method("newTextureLayer", |c| {
        let (el, v2) = style_api(c)?;
        let id = str_arg(c, 1).unwrap_or_default();
        Ok(c.push(add_layer(&el, StyleLayer::new_texture(&id), v2)))
    });
    b.method("getLayerCount", |c| {
        let (el, _) = style_api(c)?;
        let n = el.ext::<UIWorldMap>().unwrap().st().style.get_layer_count();
        Ok(c.push(n as f64))
    });
    b.method("getLayerByIndex", |c| {
        let (el, v2) = style_api(c)?;
        let i = int_arg(c, 1);
        let l = {
            let s = el.ext::<UIWorldMap>().unwrap().st();
            let n = s.style.get_layer_count();
            s.style.get_layer_by_index(idx(i, n)?).unwrap().clone()
        };
        Ok(c.push(layer_value(&el, l, v2)))
    });
    b.method("getLayerByName", |c| {
        let (el, v2) = style_api(c)?;
        let id = str_arg(c, 1);
        let l = {
            let s = el.ext::<UIWorldMap>().unwrap().st();
            s.style
                .layers()
                .iter()
                .find(|l| id.as_deref() == Some(&*l.borrow().id))
                .cloned()
        };
        Ok(c.push(l.map_or(Value::Nil, |l| layer_value(&el, l, v2))))
    });
    b.method("indexOfLayer", |c| {
        let (el, _) = style_api(c)?;
        let id = str_arg(c, 1);
        let i = {
            let s = el.ext::<UIWorldMap>().unwrap().st();
            s.style
                .layers()
                .iter()
                .position(|l| id.as_deref() == Some(&*l.borrow().id))
        };
        Ok(c.push(i.map_or(-1.0, |i| i as f64)))
    });
    b.method("moveLayer", |c| {
        let (el, _) = style_api(c)?;
        let (from, to) = (int_arg(c, 1), int_arg(c, 2));
        let m = el.ext::<UIWorldMap>().unwrap();
        let n = m.st().style.get_layer_count();
        let f = idx(from, n)?;
        let t = usize::try_from(to)
            .ok()
            .filter(|&t| t < n)
            .ok_or_else(|| oob(to, n - 1))?;
        m.st_mut().style.move_layer(f, t);
        Ok(0)
    });
    b.method("removeLayerById", |c| {
        let (el, _) = style_api(c)?;
        let id = str_arg(c, 1);
        let m = el.ext::<UIWorldMap>().unwrap();
        let i = m
            .st()
            .style
            .layers()
            .iter()
            .position(|l| id.as_deref() == Some(&*l.borrow().id));
        if let Some(i) = i {
            m.st_mut().style.remove_at(i);
        }
        Ok(0)
    });
    b.method("removeLayerByIndex", |c| {
        let (el, _) = style_api(c)?;
        let i = int_arg(c, 1);
        let m = el.ext::<UIWorldMap>().unwrap();
        let n = m.st().style.get_layer_count();
        m.st_mut().style.remove_at(idx(i, n)?);
        Ok(0)
    });
    b.method("clear", |c| {
        let (el, _) = style_api(c)?;
        el.ext::<UIWorldMap>().unwrap().st_mut().style.clear();
        Ok(0)
    });

    let b = expose_class(thread, STYLE_V2, Some(STYLE_V1));
    b.method("newPyramidLayer", |c| {
        let (el, v2) = style_api(c)?;
        let id = str_arg(c, 1).unwrap_or_default();
        Ok(c.push(add_layer(&el, StyleLayer::new_pyramid(&id), v2)))
    });
    b.method("newTextLayer", |c| {
        let (el, v2) = style_api(c)?;
        let id = str_arg(c, 1).unwrap_or_default();
        Ok(c.push(add_layer(&el, StyleLayer::new_text(&id), v2)))
    });

    register_layers(thread);
}

/// Methods every layer class has.
fn register_base(b: &ClassBuilder) {
    b.method("getTypeString", |c| {
        let v = with_layer(c, |l| l.type_string())?;
        Ok(c.push(v))
    });
    b.method("setId", |c| {
        let (el, l) = layer_obj(c)?;
        let id = str_arg(c, 1).unwrap_or_default();
        match el.upgrade() {
            Some(el) => el
                .ext::<UIWorldMap>()
                .unwrap()
                .st_mut()
                .style
                .set_layer_id(&l, &id),
            None => l.borrow_mut().id = id,
        }
        Ok(0)
    });
    for name in ["getId", "getID"] {
        b.method(name, |c| {
            let v = with_layer(c, |l| l.id.clone())?;
            Ok(c.push(Value::Str(v)))
        });
    }
    b.method("setMinZoom", |c| {
        let z = num_arg(c, 1) as f32;
        with_layer(c, |l| l.min_zoom = z)?;
        Ok(0)
    });
    b.method("getMinZoom", |c| {
        let v = with_layer(c, |l| l.min_zoom)?;
        Ok(c.push(v as f64))
    });
}

/// `addFill` alone (line layers).
fn register_add_fill(b: &ClassBuilder) {
    b.method("addFill", |c| {
        let s = color_stop(c);
        with_layer(c, |l| l.fill_mut().push(s))?;
        Ok(0)
    });
}

/// The fill-stop editing methods of polygon, texture, pyramid and text
/// layers.
fn register_fill(b: &ClassBuilder, remove_all: bool) {
    register_add_fill(b);
    b.method("removeFill", |c| {
        let i = int_arg(c, 1);
        with_layer(c, |l| {
            let f = l.fill_mut();
            idx(i, f.len()).map(|i| {
                f.remove(i);
            })
        })??;
        Ok(0)
    });
    if remove_all {
        b.method("removeAllFill", |c| {
            with_layer(c, |l| l.fill_mut().clear())?;
            Ok(0)
        });
    }
    b.method("moveFill", |c| {
        let (from, to) = (int_arg(c, 1), int_arg(c, 2));
        with_layer(c, |l| move_in(l.fill_mut(), from, to))??;
        Ok(0)
    });
    b.method("getFillStops", |c| {
        let v = with_layer(c, |l| l.fill().len())?;
        Ok(c.push(v as f64))
    });
    b.method("setFillRGBA", |c| {
        let i = int_arg(c, 1);
        let (r, g, bl, a) = (int_arg(c, 2), int_arg(c, 3), int_arg(c, 4), int_arg(c, 5));
        with_layer(c, |l| {
            let f = l.fill_mut();
            idx(i, f.len()).map(|i| {
                let s = &mut f[i];
                (s.r, s.g, s.b, s.a) = (r, g, bl, a);
            })
        })??;
        Ok(0)
    });
    b.method("setFillZoom", |c| {
        let i = int_arg(c, 1);
        let z = num_arg(c, 2) as f32;
        with_layer(c, |l| {
            let f = l.fill_mut();
            idx(i, f.len()).map(|i| f[i].zoom = zoom_clamp(z))
        })??;
        Ok(0)
    });
    b.method("getFillZoom", |c| {
        let i = int_arg(c, 1);
        let v = with_layer(c, |l| idx(i, l.fill().len()).map(|i| l.fill()[i].zoom))??;
        Ok(c.push(v as f64))
    });
    for (name, ch) in [
        ("getFillRed", 0),
        ("getFillGreen", 1),
        ("getFillBlue", 2),
        ("getFillAlpha", 3),
    ] {
        b.method(name, move |c| {
            let i = int_arg(c, 1);
            let v = with_layer(c, |l| {
                idx(i, l.fill().len()).map(|i| {
                    let s = l.fill()[i];
                    [s.r, s.g, s.b, s.a][ch]
                })
            })??;
            Ok(c.push(v as f64))
        });
    }
}

/// `setFilter(key, value)`.
fn set_filter(l: &mut StyleLayer, key: Option<Rc<str>>, value: Option<Rc<str>>) {
    l.filter_key = key.clone();
    l.filter_value = value.clone();
    let line = matches!(l.kind, LayerKind::Line { .. });
    // A null key or value fails inside the filter in the original; no
    // feature matches here.
    let (Some(k), Some(v)) = (key, value) else {
        l.filter = None;
        return;
    };
    l.filter = Some(if line {
        Filter::LineEquals(k, v)
    } else if &*v == "*" {
        Filter::PolygonHas(k)
    } else {
        Filter::PolygonEquals(k, v)
    });
}

/// The texture-stop methods of polygon and texture layers.
fn register_textures(b: &ClassBuilder, polygon: bool) {
    b.method("addTexture", move |c| {
        let z = num_arg(c, 1) as f32;
        let path = str_arg(c, 2);
        let scaling = if polygon && c.nargs() >= 4 {
            let s = str_arg(c, 3).unwrap_or_default();
            Some(TextureScaling::value_of(&s).ok_or_else(|| {
                LuaError::runtime(format!(
                    "No enum constant zombie.worldMap.styles.WorldMapStyleLayer.TextureScaling.{s}"
                ))
            })?)
        } else {
            None
        };
        let stop = TextureStop::new(
            z,
            path.as_deref(),
            scaling.unwrap_or(TextureScaling::IsoGridSquare),
        );
        with_layer(c, |l| l.textures_mut().unwrap().push(stop))?;
        Ok(0)
    });
    b.method("removeTexture", |c| {
        let i = int_arg(c, 1);
        with_layer(c, |l| {
            let t = l.textures_mut().unwrap();
            idx(i, t.len()).map(|i| {
                t.remove(i);
            })
        })??;
        Ok(0)
    });
    b.method("moveTexture", |c| {
        let (from, to) = (int_arg(c, 1), int_arg(c, 2));
        with_layer(c, |l| move_in(l.textures_mut().unwrap(), from, to))??;
        Ok(0)
    });
    b.method("getTextureStops", |c| {
        let v = with_layer(c, |l| l.textures_mut().unwrap().len())?;
        Ok(c.push(v as f64))
    });
    b.method("setTextureZoom", |c| {
        let i = int_arg(c, 1);
        let z = num_arg(c, 2) as f32;
        with_layer(c, |l| {
            let t = l.textures_mut().unwrap();
            idx(i, t.len()).map(|i| t[i].zoom = zoom_clamp(z))
        })??;
        Ok(0)
    });
    b.method("getTextureZoom", |c| {
        let i = int_arg(c, 1);
        let v = with_layer(c, |l| {
            let t = l.textures_mut().unwrap();
            idx(i, t.len()).map(|i| t[i].zoom)
        })??;
        Ok(c.push(v as f64))
    });
    b.method("setTexturePath", |c| {
        let i = int_arg(c, 1);
        let path = str_arg(c, 2);
        with_layer(c, |l| {
            let t = l.textures_mut().unwrap();
            idx(i, t.len()).map(|i| {
                t[i].texture = path.as_deref().and_then(pz_render::get_shared_texture);
                t[i].texture_path = path;
            })
        })??;
        Ok(0)
    });
    b.method("getTexturePath", |c| {
        let i = int_arg(c, 1);
        let v = with_layer(c, |l| {
            let t = l.textures_mut().unwrap();
            idx(i, t.len()).map(|i| t[i].texture_path.clone())
        })??;
        Ok(c.push(v.map_or(Value::Nil, Value::Str)))
    });
    b.method("getTexture", |c| {
        let i = int_arg(c, 1);
        let v = with_layer(c, |l| {
            let t = l.textures_mut().unwrap();
            idx(i, t.len()).map(|i| t[i].texture.clone())
        })??;
        Ok(c.push(crate::bind_render::texture_value(v)))
    });
}

fn register_layers(thread: &mut Thread) {
    let b = expose_class(thread, LAYER_V1, None);
    register_base(&b);

    let b = expose_class(thread, LINE_LAYER_V1, Some(LAYER_V1));
    b.method("setFilter", |c| {
        let (k, v) = (str_arg(c, 1), str_arg(c, 2));
        with_layer(c, |l| set_filter(l, k, v))?;
        Ok(0)
    });
    register_add_fill(&b);
    b.method("addLineWidth", |c| {
        let s = FloatStop {
            zoom: num_arg(c, 1) as f32,
            f: num_arg(c, 2) as f32,
        };
        with_layer(c, |l| {
            if let LayerKind::Line { line_width, .. } = &mut l.kind {
                line_width.push(s);
            }
        })?;
        Ok(0)
    });

    let b = expose_class(thread, POLYGON_LAYER_V1, Some(LAYER_V1));
    b.method("setFilter", |c| {
        let (k, v) = (str_arg(c, 1), str_arg(c, 2));
        with_layer(c, |l| set_filter(l, k, v))?;
        Ok(0)
    });
    b.method("getFilterKey", |c| {
        let v = with_layer(c, |l| l.filter_key.clone())?;
        Ok(c.push(v.map_or(Value::Nil, Value::Str)))
    });
    b.method("getFilterValue", |c| {
        let v = with_layer(c, |l| l.filter_value.clone())?;
        Ok(c.push(v.map_or(Value::Nil, Value::Str)))
    });
    register_fill(&b, false);
    register_textures(&b, true);
    fn scales(l: &mut StyleLayer) -> &mut Vec<FloatStop> {
        match &mut l.kind {
            LayerKind::Polygon { scale, .. } => scale,
            _ => unreachable!(),
        }
    }
    b.method("addScale", |c| {
        let s = FloatStop {
            zoom: num_arg(c, 1) as f32,
            f: num_arg(c, 2) as f32,
        };
        with_layer(c, |l| scales(l).push(s))?;
        Ok(0)
    });
    b.method("removeScale", |c| {
        let i = int_arg(c, 1);
        with_layer(c, |l| {
            let s = scales(l);
            idx(i, s.len()).map(|i| {
                s.remove(i);
            })
        })??;
        Ok(0)
    });
    b.method("moveScale", |c| {
        let (from, to) = (int_arg(c, 1), int_arg(c, 2));
        with_layer(c, |l| move_in(scales(l), from, to))??;
        Ok(0)
    });
    b.method("getScaleStops", |c| {
        let v = with_layer(c, |l| scales(l).len())?;
        Ok(c.push(v as f64))
    });
    b.method("setScaleZoom", |c| {
        let i = int_arg(c, 1);
        let z = num_arg(c, 2) as f32;
        with_layer(c, |l| {
            let s = scales(l);
            idx(i, s.len()).map(|i| s[i].zoom = zoom_clamp(z))
        })??;
        Ok(0)
    });
    b.method("setScaleValue", |c| {
        let i = int_arg(c, 1);
        let v = int_arg(c, 2) as f32;
        with_layer(c, |l| {
            let s = scales(l);
            idx(i, s.len()).map(|i| s[i].f = pz_worldmap::renderer::clamp(v, 0.0, f32::MAX))
        })??;
        Ok(0)
    });
    b.method("setTextureScaling", |c| {
        let i = int_arg(c, 1);
        let s = str_arg(c, 2).unwrap_or_default();
        let sc = TextureScaling::value_of(&s).ok_or_else(|| {
            LuaError::runtime(format!(
                "No enum constant zombie.worldMap.styles.WorldMapStyleLayer.TextureScaling.{s}"
            ))
        })?;
        with_layer(c, |l| {
            let t = l.textures_mut().unwrap();
            idx(i, t.len()).map(|i| t[i].scaling = sc)
        })??;
        Ok(0)
    });
    b.method("getTextureScaling", |c| {
        let i = int_arg(c, 1);
        let v = with_layer(c, |l| {
            let t = l.textures_mut().unwrap();
            idx(i, t.len()).map(|i| t[i].scaling.name())
        })??;
        Ok(c.push(v))
    });

    let b = expose_class(thread, TEXTURE_LAYER_V1, Some(LAYER_V1));
    register_fill(&b, true);
    register_textures(&b, false);
    b.method("removeAllTexture", |c| {
        with_layer(c, |l| l.textures_mut().unwrap().clear())?;
        Ok(0)
    });
    fn tex<R>(
        c: &NativeCall<'_>,
        f: impl FnOnce(&mut i32, &mut i32, &mut i32, &mut i32, &mut bool, &mut bool) -> R,
    ) -> LuaResult<R> {
        with_layer(c, |l| match &mut l.kind {
            LayerKind::Texture {
                world_x1,
                world_y1,
                world_x2,
                world_y2,
                use_world_bounds,
                tile,
                ..
            } => f(
                world_x1,
                world_y1,
                world_x2,
                world_y2,
                use_world_bounds,
                tile,
            ),
            _ => unreachable!(),
        })
    }
    b.method("setBoundsInSquares", |c| {
        let v = [int_arg(c, 1), int_arg(c, 2), int_arg(c, 3), int_arg(c, 4)];
        tex(c, |x1, y1, x2, y2, _, _| {
            (*x1, *y1, *x2, *y2) = (v[0], v[1], v[2], v[3]);
        })?;
        Ok(0)
    });
    b.method("getMinXInSquares", |c| {
        let v = tex(c, |x1, _, _, _, _, _| *x1)?;
        Ok(c.push(v as f64))
    });
    b.method("getMinYInSquares", |c| {
        let v = tex(c, |_, y1, _, _, _, _| *y1)?;
        Ok(c.push(v as f64))
    });
    b.method("getMaxXInSquares", |c| {
        let v = tex(c, |_, _, x2, _, _, _| *x2)?;
        Ok(c.push(v as f64))
    });
    b.method("getMaxYInSquares", |c| {
        let v = tex(c, |_, _, _, y2, _, _| *y2)?;
        Ok(c.push(v as f64))
    });
    b.method("getWidthInSquares", |c| {
        let v = tex(c, |x1, _, x2, _, _, _| x2.wrapping_sub(*x1))?;
        Ok(c.push(v as f64))
    });
    b.method("getHeightInSquares", |c| {
        let v = tex(c, |_, y1, _, y2, _, _| y2.wrapping_sub(*y1))?;
        Ok(c.push(v as f64))
    });
    b.method("setTile", |c| {
        let v = bool_arg(c, 1);
        tex(c, |_, _, _, _, _, t| *t = v)?;
        Ok(0)
    });
    b.method("isTile", |c| {
        let v = tex(c, |_, _, _, _, _, t| *t)?;
        Ok(c.push(v))
    });
    b.method("setUseWorldBounds", |c| {
        let v = bool_arg(c, 1);
        tex(c, |_, _, _, _, u, _| *u = v)?;
        Ok(0)
    });
    b.method("isUseWorldBounds", |c| {
        let v = tex(c, |_, _, _, _, u, _| *u)?;
        Ok(c.push(v))
    });

    let b = expose_class(thread, PYRAMID_LAYER_V1, Some(LAYER_V1));
    register_fill(&b, true);
    b.method("getPyramidFileName", |c| {
        let v = with_layer(c, |l| match &l.kind {
            LayerKind::Pyramid { file_name, .. } => file_name.clone(),
            _ => None,
        })?;
        Ok(c.push(v.map_or(Value::Nil, Value::Str)))
    });
    b.method("setPyramidFileName", |c| {
        let v = str_arg(c, 1);
        with_layer(c, |l| {
            if let LayerKind::Pyramid { file_name, .. } = &mut l.kind {
                *file_name = v;
            }
        })?;
        Ok(0)
    });

    let b = expose_class(thread, TEXT_LAYER_V1, Some(LAYER_V1));
    register_fill(&b, true);
    b.method("getFont", |c| {
        let v = with_layer(c, |l| l.font())?;
        Ok(c.push(crate::bind_text::font_value(v)))
    });
    b.method("setFont", |c| {
        let f = crate::bind_text::font_arg(c, 1);
        with_layer(c, |l| {
            if let (LayerKind::Text { font, .. }, Some(f)) = (&mut l.kind, f) {
                *font = f;
            }
        })?;
        // TODO(worldmap symbols): getSymbolsDirect().invalidateLayout().
        Ok(0)
    });
    b.method("getLineHeight", |c| {
        let v = with_layer(c, |l| match &l.kind {
            LayerKind::Text { line_height, .. } => *line_height,
            _ => 0,
        })?;
        Ok(c.push(v as f64))
    });
    b.method("setLineHeight", |c| {
        let v = int_arg(c, 1);
        with_layer(c, |l| {
            if let LayerKind::Text { line_height, .. } = &mut l.kind {
                *line_height = v;
            }
        })?;
        Ok(0)
    });
}
