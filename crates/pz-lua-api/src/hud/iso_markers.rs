//! `zombie.iso.IsoMarkers` and `zombie.iso.IsoMarkers$IsoMarker`:
//! billboarded world-tile sprite/item markers and optional debug vision
//! circles (used by Build 42.22 Foraging `ISBaseIcon` / `ISSearchManager`
//! and Lua scripts via `getIsoMarkers():addIsoMarker(...)`).

use super::host::with_host;
use crate::expose::{bool_arg, expose_class, global, int_arg, num_arg, str_arg, with_this};
use pz_inventory::shared::ItemRef;
use pz_kahlua::value::LuaObject;
use pz_kahlua::{LuaError, LuaResult, NativeCall, Thread, Value};
use pz_render::Texture;
use std::any::Any;
use std::cell::RefCell;
use std::rc::Rc;

pub const ISO_MARKERS: &str = "zombie.iso.IsoMarkers";
pub const ISO_MARKER: &str = "zombie.iso.IsoMarkers$IsoMarker";

/// Render-ready snapshot of an active `IsoMarker` for `pz-iso-render`.
#[derive(Clone, Debug)]
pub struct IsoMarkerSnapshot {
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub z_layer: i32,
    pub rgba: [f32; 4],
    pub active: bool,
    pub circle_size: f32,
    pub textures: Vec<Texture>,
}

/// Live state of a single `IsoMarkers.IsoMarker`.
#[derive(Clone, Debug)]
pub struct IsoMarkerState {
    pub id: i32,
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
    pub alpha: f32,
    pub alpha_min: f32,
    pub alpha_max: f32,
    pub fade_speed: f32,
    pub do_alpha: bool,
    pub alpha_inc: bool,
    pub active: bool,
    pub is_removed: bool,
    pub circle_size: f32,
    pub rotation: f32,
    pub textures: Vec<Texture>,
    pub overlay_textures: Vec<Texture>,
    pub item: Option<ItemRef>,
    /// Retained Lua `IsoGridSquare` value if passed at creation.
    pub square: Value,
}

impl IsoMarkerState {
    fn new(id: i32, x: f32, y: f32, z: f32, r: f32, g: f32, b: f32, a: f32) -> Self {
        Self {
            id,
            x,
            y,
            z,
            r,
            g,
            b,
            a,
            alpha: a,
            alpha_min: 0.3,
            alpha_max: 1.0,
            fade_speed: 0.006,
            do_alpha: false,
            alpha_inc: true,
            active: true,
            is_removed: false,
            circle_size: 0.0,
            rotation: 0.0,
            textures: Vec::new(),
            overlay_textures: Vec::new(),
            item: None,
            square: Value::Nil,
        }
    }

    fn effective_alpha(&self) -> f32 {
        if self.do_alpha {
            self.alpha
        } else {
            self.a
        }
    }

    fn snapshot(&self) -> IsoMarkerSnapshot {
        IsoMarkerSnapshot {
            x: self.x,
            y: self.y,
            z: self.z,
            z_layer: self.z.floor() as i32,
            rgba: [self.r, self.g, self.b, self.effective_alpha()],
            active: self.active && !self.is_removed,
            circle_size: self.circle_size,
            textures: self.textures.clone(),
        }
    }
}

pub type IsoMarkerRef = Rc<RefCell<IsoMarkerState>>;

/// `zombie.iso.IsoMarkers` singleton state.
#[derive(Default)]
pub struct IsoMarkers {
    next_id: i32,
    markers: Vec<IsoMarkerRef>,
}

impl IsoMarkers {
    pub fn init(&mut self) {
        self.reset();
    }

    pub fn reset(&mut self) {
        for m in &self.markers {
            m.borrow_mut().is_removed = true;
        }
        self.markers.clear();
    }

    pub fn update(&mut self) {
        let mult = with_host(|h| h.game_time.get_multiplier());
        let step_scale = if mult > 0.0 { mult } else { 1.0 };
        self.markers.retain(|m| {
            let mut st = m.borrow_mut();
            if st.is_removed {
                return false;
            }
            if st.do_alpha {
                if st.alpha_inc {
                    st.alpha += st.fade_speed * step_scale;
                    if st.alpha >= st.alpha_max {
                        st.alpha = st.alpha_max;
                        st.alpha_inc = false;
                    }
                } else {
                    st.alpha -= st.fade_speed * step_scale;
                    if st.alpha <= st.alpha_min {
                        st.alpha = st.alpha_min;
                        st.alpha_inc = true;
                    }
                }
            }
            true
        });
    }

    pub fn get_marker(&self, id: i32) -> Option<IsoMarkerRef> {
        self.markers
            .iter()
            .find(|m| {
                let st = m.borrow();
                st.id == id && !st.is_removed
            })
            .cloned()
    }

    pub fn remove_marker(&mut self, id: i32) -> bool {
        let mut removed = false;
        self.markers.retain(|m| {
            let mut st = m.borrow_mut();
            if st.id == id {
                st.is_removed = true;
                removed = true;
                false
            } else {
                true
            }
        });
        removed
    }

    pub fn add_marker(&mut self, mut state: IsoMarkerState) -> IsoMarkerRef {
        let id = self.next_id;
        self.next_id = self.next_id.wrapping_add(1);
        state.id = id;
        let rc = Rc::new(RefCell::new(state));
        self.markers.push(rc.clone());
        rc
    }

    pub fn snapshot(&self) -> Vec<IsoMarkerSnapshot> {
        self.markers
            .iter()
            .filter_map(|m| {
                let st = m.borrow();
                if st.is_removed || !st.active {
                    None
                } else {
                    Some(st.snapshot())
                }
            })
            .collect()
    }

    pub fn len(&self) -> usize {
        self.markers.len()
    }

    pub fn is_empty(&self) -> bool {
        self.markers.is_empty()
    }
}

thread_local! {
    static MARKERS: RefCell<IsoMarkers> = RefCell::new(IsoMarkers::default());
    static INSTANCE_VALUE: Value = Value::Object(Rc::new(IsoMarkersObject));
}

pub fn with_iso_markers<R>(f: impl FnOnce(&mut IsoMarkers) -> R) -> R {
    MARKERS.with(|m| f(&mut m.borrow_mut()))
}

pub fn update() {
    with_iso_markers(IsoMarkers::update);
}

pub fn reset() {
    with_iso_markers(IsoMarkers::reset);
}

pub fn snapshot() -> Vec<IsoMarkerSnapshot> {
    with_iso_markers(|m| m.snapshot())
}

struct IsoMarkersObject;

impl LuaObject for IsoMarkersObject {
    fn class_name(&self) -> &str {
        ISO_MARKERS
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

struct IsoMarkerObject(IsoMarkerRef);

impl LuaObject for IsoMarkerObject {
    fn class_name(&self) -> &str {
        ISO_MARKER
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

fn marker_value(m: Option<IsoMarkerRef>) -> Value {
    match m {
        Some(r) => Value::Object(Rc::new(IsoMarkerObject(r))),
        None => Value::Nil,
    }
}

fn marker_this(c: &NativeCall<'_>) -> LuaResult<IsoMarkerRef> {
    with_this::<IsoMarkerObject, _>(c, |o| o.0.clone())
}

fn f(c: &NativeCall<'_>, i: usize) -> f32 {
    num_arg(c, i) as f32
}

fn resolve_texture_name(name: &str) -> Option<Texture> {
    if name.is_empty() {
        return None;
    }
    pz_render::get_shared_texture(name)
        .or_else(|| pz_render::get_shared_texture(&format!("media/textures/{name}.png")))
        .or_else(|| pz_render::get_shared_texture(&format!("media/textures/Item_{name}.png")))
}

fn collect_textures(v: &Value) -> (Vec<Texture>, Option<ItemRef>) {
    match v {
        Value::Nil => (Vec::new(), None),
        Value::Str(s) => (resolve_texture_name(s).into_iter().collect(), None),
        Value::Table(tbl) => {
            let mut out = Vec::new();
            let mut idx = 1;
            loop {
                let elem = tbl.rawget(&Value::Num(idx as f64));
                if elem.is_nil() {
                    break;
                }
                if let Value::Str(s) = elem {
                    if let Some(tex) = resolve_texture_name(&s) {
                        out.push(tex);
                    }
                }
                idx += 1;
            }
            (out, None)
        }
        Value::Object(_) => {
            if let Some(item) = crate::bind_items::item_arg_value(v) {
                let mut out = Vec::new();
                if let Ok(Some(tex_ref)) = pz_inventory::item_texture::get_tex_ref(&item) {
                    out.push(tex_ref.get());
                } else {
                    let icon = item.borrow().template.icon.clone();
                    if let Some(icon) = icon.as_deref() {
                        if let Some(tex) = resolve_texture_name(icon) {
                            out.push(tex);
                        }
                    }
                }
                (out, Some(item))
            } else {
                (Vec::new(), None)
            }
        }
        _ => (Vec::new(), None),
    }
}

fn extract_square_coords(sq: &Value) -> Option<(i32, i32, i32)> {
    with_host(|h| h.picking.square_coords(sq)).or_else(|| crate::bind_world::square_coords(sq))
}

fn marker_id_arg(c: &NativeCall<'_>, idx: usize) -> LuaResult<i32> {
    match c.arg(idx) {
        Value::Object(o) => {
            if let Some(m) = o.as_any().downcast_ref::<IsoMarkerObject>() {
                return Ok(m.0.borrow().id);
            }
            Err(LuaError::runtime("Expected IsoMarker or integer id"))
        }
        _ => Ok(int_arg(c, idx)),
    }
}

/// Register `getIsoMarkers()`, `zombie.iso.IsoMarkers`, and
/// `zombie.iso.IsoMarkers$IsoMarker`.
pub fn register(thread: &mut Thread) {
    global(thread, "getIsoMarkers", |c| {
        Ok(c.push(INSTANCE_VALUE.with(Clone::clone)))
    });

    let b = expose_class(thread, ISO_MARKERS, None);
    b.field("instance", INSTANCE_VALUE.with(Clone::clone));
    b.method("init", |_| {
        with_iso_markers(IsoMarkers::init);
        Ok(0)
    });
    b.method("reset", |_| {
        with_iso_markers(IsoMarkers::reset);
        Ok(0)
    });
    b.method("update", |_| {
        with_iso_markers(IsoMarkers::update);
        Ok(0)
    });
    b.method("render", |_| Ok(0));
    b.method("renderIsoMarkers", |_| Ok(0));
    b.method("getIsoMarker", |c| {
        let id = int_arg(c, 1);
        let m = with_iso_markers(|im| im.get_marker(id));
        Ok(c.push(marker_value(m)))
    });
    b.method("removeIsoMarker", |c| {
        let id = marker_id_arg(c, 1)?;
        let removed = with_iso_markers(|im| im.remove_marker(id));
        Ok(c.push(removed))
    });
    b.method("addCircleIsoMarker", |c| {
        let sq_val = c.arg(1);
        let Some((x, y, z)) = extract_square_coords(&sq_val) else {
            return Ok(c.push(Value::Nil));
        };
        let (r, g, b_col, a) = (f(c, 2), f(c, 3), f(c, 4), f(c, 5));
        let mut st = IsoMarkerState::new(0, x as f32, y as f32, z as f32, r, g, b_col, a);
        st.circle_size = 1.0;
        st.square = sq_val;
        let m = with_iso_markers(|im| im.add_marker(st));
        Ok(c.push(marker_value(Some(m))))
    });
    b.method("addIsoMarker", |c| {
        // Supported Build 42.22 overloads:
        // 1) (spriteOrTableOrItem, square, r, g, b, a [, rotation])
        // 2) (spriteOrTable, overlayOrTable, square, r, g, b, doAlpha [, doIsoObject [, fadeSpeed, alphaMin, alphaMax]])
        let arg1 = c.arg(1);
        let arg2 = c.arg(2);
        let arg3 = c.arg(3);
        let (textures, overlay_textures, item, sq_val, sq_idx, has_overlay) =
            if extract_square_coords(&arg2).is_some() {
                let (tex, item) = collect_textures(&arg1);
                (tex, Vec::new(), item, arg2, 2usize, false)
            } else if extract_square_coords(&arg3).is_some() {
                let (tex, item) = collect_textures(&arg1);
                let (ov, _) = collect_textures(&arg2);
                (tex, ov, item, arg3, 3usize, true)
            } else {
                return Ok(c.push(Value::Nil));
            };
        let Some((x, y, z)) = extract_square_coords(&sq_val) else {
            return Ok(c.push(Value::Nil));
        };
        let r = f(c, sq_idx + 1);
        let g = f(c, sq_idx + 2);
        let b_col = f(c, sq_idx + 3);
        let mut st = if has_overlay {
            let do_alpha = bool_arg(c, sq_idx + 4);
            let mut st = IsoMarkerState::new(0, x as f32, y as f32, z as f32, r, g, b_col, 1.0);
            st.do_alpha = do_alpha;
            if c.nargs() > sq_idx + 8 {
                st.fade_speed = f(c, sq_idx + 6);
                st.alpha_min = f(c, sq_idx + 7);
                st.alpha_max = f(c, sq_idx + 8);
            }
            st
        } else {
            let a = if c.nargs() > sq_idx + 4 {
                f(c, sq_idx + 4)
            } else {
                1.0
            };
            let mut st = IsoMarkerState::new(0, x as f32, y as f32, z as f32, r, g, b_col, a);
            if c.nargs() > sq_idx + 5 {
                st.rotation = f(c, sq_idx + 5);
            }
            st
        };
        st.textures = textures;
        st.overlay_textures = overlay_textures;
        st.item = item;
        st.square = sq_val;
        let m = with_iso_markers(|im| im.add_marker(st));
        Ok(c.push(marker_value(Some(m))))
    });

    let m = expose_class(thread, ISO_MARKER, None);
    m.method("getID", |c| Ok(c.push(marker_this(c)?.borrow().id)));
    m.method("getX", |c| Ok(c.push(marker_this(c)?.borrow().x as f64)));
    m.method("getY", |c| Ok(c.push(marker_this(c)?.borrow().y as f64)));
    m.method("getZ", |c| Ok(c.push(marker_this(c)?.borrow().z as f64)));
    m.method("setPos", |c| {
        let r = marker_this(c)?;
        let mut st = r.borrow_mut();
        st.x = f(c, 1);
        st.y = f(c, 2);
        st.z = f(c, 3);
        Ok(0)
    });
    m.method("getSquare", |c| {
        Ok(c.push(marker_this(c)?.borrow().square.clone()))
    });
    m.method("setSquare", |c| {
        let r = marker_this(c)?;
        let sq = c.arg(1);
        let mut st = r.borrow_mut();
        if let Some((x, y, z)) = extract_square_coords(&sq) {
            st.x = x as f32;
            st.y = y as f32;
            st.z = z as f32;
        }
        st.square = sq;
        Ok(0)
    });
    m.method("getR", |c| Ok(c.push(marker_this(c)?.borrow().r as f64)));
    m.method("setR", |c| {
        marker_this(c)?.borrow_mut().r = f(c, 1);
        Ok(0)
    });
    m.method("getG", |c| Ok(c.push(marker_this(c)?.borrow().g as f64)));
    m.method("setG", |c| {
        marker_this(c)?.borrow_mut().g = f(c, 1);
        Ok(0)
    });
    m.method("getB", |c| Ok(c.push(marker_this(c)?.borrow().b as f64)));
    m.method("setB", |c| {
        marker_this(c)?.borrow_mut().b = f(c, 1);
        Ok(0)
    });
    m.method("getA", |c| Ok(c.push(marker_this(c)?.borrow().a as f64)));
    m.method("setA", |c| {
        let v = f(c, 1);
        let r = marker_this(c)?;
        let mut st = r.borrow_mut();
        st.a = v;
        st.alpha = v;
        Ok(0)
    });
    m.method("getAlpha", |c| {
        Ok(c.push(marker_this(c)?.borrow().effective_alpha() as f64))
    });
    m.method("setAlpha", |c| {
        let v = f(c, 1);
        let r = marker_this(c)?;
        let mut st = r.borrow_mut();
        st.a = v;
        st.alpha = v;
        Ok(0)
    });
    m.method("getAlphaMin", |c| {
        Ok(c.push(marker_this(c)?.borrow().alpha_min as f64))
    });
    m.method("setAlphaMin", |c| {
        marker_this(c)?.borrow_mut().alpha_min = f(c, 1);
        Ok(0)
    });
    m.method("getAlphaMax", |c| {
        Ok(c.push(marker_this(c)?.borrow().alpha_max as f64))
    });
    m.method("setAlphaMax", |c| {
        marker_this(c)?.borrow_mut().alpha_max = f(c, 1);
        Ok(0)
    });
    m.method("getFadeSpeed", |c| {
        Ok(c.push(marker_this(c)?.borrow().fade_speed as f64))
    });
    m.method("setFadeSpeed", |c| {
        marker_this(c)?.borrow_mut().fade_speed = f(c, 1);
        Ok(0)
    });
    m.method("isDoAlpha", |c| {
        Ok(c.push(marker_this(c)?.borrow().do_alpha))
    });
    m.method("setDoAlpha", |c| {
        marker_this(c)?.borrow_mut().do_alpha = bool_arg(c, 1);
        Ok(0)
    });
    m.method("isActive", |c| Ok(c.push(marker_this(c)?.borrow().active)));
    m.method("setActive", |c| {
        marker_this(c)?.borrow_mut().active = bool_arg(c, 1);
        Ok(0)
    });
    m.method("getCircleSize", |c| {
        Ok(c.push(marker_this(c)?.borrow().circle_size as f64))
    });
    m.method("setCircleSize", |c| {
        marker_this(c)?.borrow_mut().circle_size = f(c, 1);
        Ok(0)
    });
    m.method("getRotation", |c| {
        Ok(c.push(marker_this(c)?.borrow().rotation as f64))
    });
    m.method("setRotation", |c| {
        marker_this(c)?.borrow_mut().rotation = f(c, 1);
        Ok(0)
    });
    m.method("remove", |c| {
        let id = {
            let r = marker_this(c)?;
            let mut st = r.borrow_mut();
            st.is_removed = true;
            st.id
        };
        with_iso_markers(|im| im.remove_marker(id));
        Ok(0)
    });
    m.method("isRemoved", |c| {
        Ok(c.push(marker_this(c)?.borrow().is_removed))
    });
    m.method("hasTempSquareObject", |c| {
        let _ = marker_this(c)?;
        Ok(c.push(false))
    });
    m.method("init", |c| {
        let r = marker_this(c)?;
        let sprite = str_arg(c, 1);
        let x = int_arg(c, 2) as f32;
        let y = int_arg(c, 3) as f32;
        let z = int_arg(c, 4) as f32;
        let mut st = r.borrow_mut();
        st.x = x;
        st.y = y;
        st.z = z;
        if let Some(s) = sprite {
            if let Some(tex) = resolve_texture_name(&s) {
                st.textures = vec![tex];
            }
        }
        Ok(0)
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use pz_kahlua::TableRef;

    #[test]
    fn foraging_add_update_circle_and_remove_iso_markers() {
        reset();
        let mut t = Thread::new(TableRef::new());
        pz_kahlua::stdlib::install(&mut t, None).unwrap();
        crate::bind_world::register(&mut t);
        register(&mut t);

        let mut cell = pz_world::IsoCell::default();
        let mut chunk = pz_world::IsoChunk::new(1, 2);
        chunk.set_square(2, 4, 0, Some(pz_world::IsoGridSquare::new(10, 20, 0).into()));
        cell.chunks.insert((1, 2), chunk);
        crate::bind_world::set_cell(Some(cell));

        let f = pz_kahlua::stdlib::load_string(
            r#"
            local sq = getCell():getGridSquare(10, 20, 0)
            local m1 = getIsoMarkers():addIsoMarker("forage_stone", sq, 1, 0.5, 0.25, 0.8)
            if m1 == nil then error("expected m1") end
            m1:setCircleSize(4.5)
            m1:setAlpha(0.6)
            local m2 = getIsoMarkers():addIsoMarker({"tex_a", "tex_b"}, sq, 1, 1, 1, 0.9)
            if m2 == nil then error("expected m2") end
            if getIsoMarkers():getIsoMarker(m1:getID()) == nil then error("getIsoMarker failed") end
            return m1:getID(), m1:getX(), m1:getY(), m1:getZ(), m1:getCircleSize(), m1:getAlpha(), m2:getID()
            "#,
            Some("iso_markers_test"),
            t.globals().clone(),
        )
        .unwrap();
        let r = t.pcall(&f, &[]).unwrap();
        assert_eq!(r[1].as_num(), Some(10.0));
        assert_eq!(r[2].as_num(), Some(20.0));
        assert_eq!(r[3].as_num(), Some(0.0));
        assert_eq!(r[4].as_num(), Some(4.5));
        assert!((r[5].as_num().unwrap() - 0.6).abs() < 1e-5);

        let snap = snapshot();
        assert_eq!(snap.len(), 2);
        assert_eq!(snap[0].circle_size, 4.5);
        assert!((snap[0].rgba[3] - 0.6).abs() < 1e-5);

        let id1 = r[0].as_num().unwrap() as i32;
        assert!(with_iso_markers(|im| im.remove_marker(id1)));
        assert_eq!(snapshot().len(), 1);
        reset();
        assert!(snapshot().is_empty());
        crate::bind_world::set_cell(None);
    }
}
