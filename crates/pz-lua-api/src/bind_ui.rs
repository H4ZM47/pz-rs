//! Lua bindings for `zombie.ui.UIElement` and `zombie.ui.UIManager`
//! (engine side in `pz-ui`), plus the `zombie.ui.UIFont` enum when nobody
//! else has exposed it yet.
//!
//! Overloads are resolved the way Kahlua picks them for the vanilla call
//! sites: by argument count and, where counts collide, by whether the
//! first argument is a string (`DrawText(String, ...)` vs
//! `DrawText(UIFont, String, ...)`).
//!
//! `UIManager.getUI()` and `UIElement:getControls()` return live list views
//! (class `zombie.ui.UIElementList`, an `ArrayList` stand-in with `size`,
//! `get`, `isEmpty`, `contains`, `indexOf`).

use crate::expose::{
    bool_arg, expose_class, expose_enum, int_arg, num_arg, str_arg, this, JavaEnum,
};
use pz_kahlua::value::LuaObject;
use pz_kahlua::{LuaError, LuaResult, NativeCall, NativeFunction, TableRef, Thread, Value};
use pz_render::Rgba;
use pz_text::UIFont;
use pz_render::{LuaTexture, Texture};
use pz_ui::{with_ui, ColorObject, UIElement, UIManager};
use std::any::Any;
use std::rc::Rc;

pub const UIELEMENT: &str = "zombie.ui.UIElement";
pub const UIMANAGER: &str = "zombie.ui.UIManager";
pub const UIFONT: &str = "zombie.ui.UIFont";
const LIST: &str = "zombie.ui.UIElementList";

/// `zombie.ui.UIFont` constants in ordinal order (Build 42).
pub const UIFONT_NAMES: &[&str] = &[
    "Small",
    "Medium",
    "Large",
    "Massive",
    "MainMenu1",
    "MainMenu2",
    "Cred1",
    "Cred2",
    "NewSmall",
    "NewMedium",
    "NewLarge",
    "Code",
    "CodeSmall",
    "CodeMedium",
    "CodeLarge",
    "MediumNew",
    "AutoNormSmall",
    "AutoNormMedium",
    "AutoNormLarge",
    "Dialogue",
    "Intro",
    "Handwritten",
    "DebugConsole",
    "Title",
    "SdfRegular",
    "SdfBold",
    "SdfItalic",
    "SdfBoldItalic",
    "SdfOldRegular",
    "SdfOldBold",
    "SdfOldItalic",
    "SdfOldBoldItalic",
    "SdfRobertoSans",
    "SdfCaveat",
];

/// Map a Java `UIFont` name onto `pz_text::UIFont` (`UIFont.valueOf`).
pub fn ui_font_from_name(name: &str) -> Option<UIFont> {
    UIFont::from_string(name)
}

// ---- argument helpers ----

fn el(c: &NativeCall<'_>) -> LuaResult<Rc<UIElement>> {
    let o = this::<UIElement>(c)?;
    Ok(o.as_any().downcast_ref::<UIElement>().unwrap().rc())
}

pub fn element_of(v: &Value) -> Option<Rc<UIElement>> {
    match v {
        Value::Object(o) => o.as_any().downcast_ref::<UIElement>().map(|e| e.rc()),
        _ => None,
    }
}

fn el_arg(c: &NativeCall<'_>, i: usize) -> LuaResult<Rc<UIElement>> {
    element_of(&c.arg(i)).ok_or_else(|| LuaError::Runtime {
        message: format!(
            "Expected UIElement but got {}",
            pz_kahlua::stdlib::java_class_name(&c.arg(i))
        ),
        class: "java.lang.IllegalArgumentException",
    })
}

fn tex_arg(c: &NativeCall<'_>, i: usize) -> Option<Texture> {
    match c.arg(i) {
        Value::Object(o) => o.as_any().downcast_ref::<LuaTexture>().map(LuaTexture::get),
        _ => None,
    }
}

fn font_arg(c: &NativeCall<'_>, i: usize) -> Option<UIFont> {
    match c.arg(i) {
        Value::Object(o) => o
            .as_any()
            .downcast_ref::<JavaEnum>()
            .filter(|e| e.class == UIFONT)
            .and_then(|e| ui_font_from_name(e.name)),
        _ => None,
    }
}

fn color_arg(c: &NativeCall<'_>, i: usize) -> Rgba {
    match c.arg(i) {
        Value::Object(o) => o
            .as_any()
            .downcast_ref::<ColorObject>()
            .map(|c| c.rgba.get())
            .unwrap_or(Rgba(1.0, 1.0, 1.0, 1.0)),
        _ => Rgba(1.0, 1.0, 1.0, 1.0),
    }
}

fn nums<const N: usize>(c: &NativeCall<'_>, from: usize) -> [f64; N] {
    std::array::from_fn(|k| num_arg(c, from + k))
}

fn opt_str(s: Option<Rc<str>>) -> Value {
    s.map(Value::Str).unwrap_or_default()
}

fn opt_el(e: Option<Rc<UIElement>>) -> Value {
    e.map(|e| Value::Object(e as Rc<dyn LuaObject>))
        .unwrap_or_default()
}

fn args(c: &NativeCall<'_>) -> usize {
    c.nargs().saturating_sub(1)
}

// ---- list view ----

enum ListSrc {
    Ui,
    Controls(Rc<UIElement>),
}

/// Live `ArrayList<UIElement>` view.
pub struct UIElementList {
    src: ListSrc,
}

impl UIElementList {
    fn items(&self) -> Vec<Rc<UIElement>> {
        match &self.src {
            ListSrc::Ui => UIManager::get_ui(),
            ListSrc::Controls(e) => e.get_controls(),
        }
    }
}

impl LuaObject for UIElementList {
    fn class_name(&self) -> &str {
        LIST
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn super_classes(&self) -> &[&'static str] {
        &["java.util.ArrayList"]
    }
}

fn list_this(c: &NativeCall<'_>) -> LuaResult<Vec<Rc<UIElement>>> {
    let o = this::<UIElementList>(c)?;
    Ok(o.as_any().downcast_ref::<UIElementList>().unwrap().items())
}

fn index_of(items: &[Rc<UIElement>], v: &Value) -> i32 {
    match element_of(v) {
        Some(e) => items
            .iter()
            .position(|x| Rc::ptr_eq(x, &e))
            .map_or(-1, |i| i as i32),
        None => -1,
    }
}

fn register_list(thread: &mut Thread) {
    if !thread.class_metatables.get_str(LIST).is_nil() {
        return;
    }
    let meta = TableRef::new();
    let methods = TableRef::new();
    meta.set_str("__index", methods.clone());
    let reg = |name: &str, f: fn(&mut NativeCall<'_>) -> LuaResult<usize>| {
        methods.set_str(name, Value::Native(NativeFunction::new(name, f)));
    };
    reg("size", |c| {
        let n = list_this(c)?.len() as f64;
        Ok(c.push(n))
    });
    reg("isEmpty", |c| {
        let e = list_this(c)?.is_empty();
        Ok(c.push(e))
    });
    reg("get", |c| {
        let items = list_this(c)?;
        let i = int_arg(c, 1);
        match items.get(i as usize).filter(|_| i >= 0) {
            Some(e) => Ok(c.push(Value::Object(e.clone() as Rc<dyn LuaObject>))),
            None => Err(LuaError::Runtime {
                message: format!("Index {i} out of bounds for length {}", items.len()),
                class: "java.lang.IndexOutOfBoundsException",
            }),
        }
    });
    reg("contains", |c| {
        let items = list_this(c)?;
        let r = index_of(&items, &c.arg(1)) >= 0;
        Ok(c.push(r))
    });
    reg("indexOf", |c| {
        let items = list_this(c)?;
        let r = index_of(&items, &c.arg(1));
        Ok(c.push(r))
    });
    thread.class_metatables.set_str(LIST, meta);
}

/// Register `UIElement`, `UIManager` (and `UIFont` if missing).
pub fn register(thread: &mut Thread) {
    if thread.class_metatables.get_str(UIFONT).is_nil() {
        expose_enum(thread, UIFONT, UIFONT_NAMES);
    }
    register_list(thread);
    register_element(thread);
    register_manager(thread);
}

/// Route `UIManager`'s `LuaEventManager.triggerEvent` calls to `events`.
pub fn install_event_sink(events: crate::events::EventManager) {
    UIManager::set_event_sink(Some(Rc::new(
        move |t: &mut Thread, name: &str, a: &[Value]| {
            events.trigger(t, name, a);
        },
    )));
}

macro_rules! get {
    ($b:ident, $name:literal, $f:ident) => {
        $b.method($name, |c| {
            let v = el(c)?.$f();
            Ok(c.push(v))
        });
    };
}
macro_rules! set_num {
    ($b:ident, $name:literal, $f:ident) => {
        $b.method($name, |c| {
            el(c)?.$f(num_arg(c, 1));
            Ok(0)
        });
    };
}
macro_rules! set_bool {
    ($b:ident, $name:literal, $f:ident) => {
        $b.method($name, |c| {
            el(c)?.$f(bool_arg(c, 1));
            Ok(0)
        });
    };
}
macro_rules! act {
    ($b:ident, $name:literal, $f:ident) => {
        $b.method($name, |c| {
            el(c)?.$f();
            Ok(0)
        });
    };
}
/// `(Texture, x, y, w, h, r, g, b, a)` draw methods.
macro_rules! draw9 {
    ($b:ident, $name:literal, $f:ident) => {
        $b.method($name, |c| {
            let e = el(c)?;
            let t = tex_arg(c, 1);
            let [x, y, w, h, r, g, bb, a] = nums::<8>(c, 2);
            e.$f(t.as_ref(), x, y, w, h, r, g, bb, a);
            Ok(0)
        });
    };
}
/// `(Texture, ratio, x, y, w, h, r, g, b, a)` draw methods.
macro_rules! draw10 {
    ($b:ident, $name:literal, $f:ident) => {
        $b.method($name, |c| {
            let e = el(c)?;
            let t = tex_arg(c, 1);
            let [k, x, y, w, h, r, g, bb, a] = nums::<9>(c, 2);
            e.$f(t.as_ref(), k, x, y, w, h, r, g, bb, a);
            Ok(0)
        });
    };
}

fn register_element(thread: &mut Thread) {
    let b = expose_class(thread, UIELEMENT, None);
    b.ctor(|c| {
        let table = match c.arg(0) {
            Value::Table(t) => Some(t),
            _ => None,
        };
        let e = UIElement::new(table);
        Ok(c.push(Value::Object(e as Rc<dyn LuaObject>)))
    });

    get!(b, "getMaxDrawHeight", get_max_draw_height);
    set_num!(b, "setMaxDrawHeight", set_max_draw_height);
    act!(b, "clearMaxDrawHeight", clear_max_draw_height);
    get!(b, "getXScroll", get_x_scroll);
    get!(b, "getYScroll", get_y_scroll);
    b.method("setXScroll", |c| {
        let e = el(c)?;
        let v = num_arg(c, 1);
        e.set_x_scroll(c.thread, v);
        Ok(0)
    });
    b.method("setYScroll", |c| {
        let e = el(c)?;
        let v = num_arg(c, 1);
        e.set_y_scroll(c.thread, v);
        Ok(0)
    });
    set_bool!(b, "setAlwaysOnTop", set_always_on_top);
    get!(b, "isAlwaysOnTop", is_always_on_top);
    act!(b, "backMost", back_most);
    get!(b, "isBackMost", is_back_most);
    b.method("AddChild", |c| {
        let e = el(c)?;
        e.add_child(&*el_arg(c, 1)?);
        Ok(0)
    });
    b.method("RemoveChild", |c| {
        let e = el(c)?;
        e.remove_child(&*el_arg(c, 1)?);
        Ok(0)
    });
    b.method("RemoveControl", |c| {
        let e = el(c)?;
        e.remove_control(&*el_arg(c, 1)?);
        Ok(0)
    });
    b.method("BringToTop", |c| {
        let e = el(c)?;
        e.bring_child_to_top(&*el_arg(c, 1)?);
        Ok(0)
    });
    get!(b, "getScrollHeight", get_scroll_height);
    set_num!(b, "setScrollHeight", set_scroll_height);
    get!(b, "isConsumeMouseEvents", is_consume_mouse_events);
    set_bool!(b, "setConsumeMouseEvents", set_consume_mouse_events);
    act!(b, "ClearChildren", clear_children);
    b.method("ButtonClicked", |c| {
        el(c)?.button_clicked(str_arg(c, 1));
        Ok(0)
    });
    b.method("getClickedValue", |c| {
        let v = opt_str(el(c)?.get_clicked_value());
        Ok(c.push(v))
    });
    b.method("setClickedValue", |c| {
        el(c)?.set_clicked_value(str_arg(c, 1));
        Ok(0)
    });

    // ---- text ----
    b.method("DrawText", |c| {
        let e = el(c)?;
        if let Value::Str(text) = c.arg(1) {
            // (String, x, y, [w, h,] r, g, b, a): default font.
            let [x, y] = nums::<2>(c, 2);
            let col = if args(c) >= 9 {
                nums::<4>(c, 6)
            } else {
                nums::<4>(c, 4)
            };
            e.draw_text_default(&text, x, y, col[0], col[1], col[2], col[3]);
        } else {
            let font = font_arg(c, 1);
            let text = str_arg(c, 2);
            if args(c) >= 9 {
                let [x, y, z, r, g, bb, a] = nums::<7>(c, 3);
                e.draw_text_zoom(
                    font,
                    text.as_deref().unwrap_or("null"),
                    x,
                    y,
                    z,
                    r,
                    g,
                    bb,
                    a,
                );
            } else {
                let [x, y, r, g, bb, a] = nums::<6>(c, 3);
                e.draw_text(font, text.as_deref(), x, y, r, g, bb, a);
            }
        }
        Ok(0)
    });
    b.method("DrawTextUntrimmed", |c| {
        let e = el(c)?;
        let text = str_arg(c, 2);
        let [x, y, r, g, bb, a] = nums::<6>(c, 3);
        e.draw_text_untrimmed(font_arg(c, 1), text.as_deref(), x, y, r, g, bb, a);
        Ok(0)
    });
    fn font_text(c: &NativeCall<'_>) -> (Option<UIFont>, Rc<str>, usize) {
        match c.arg(1) {
            Value::Str(s) => (None, s, 2),
            _ => (
                font_arg(c, 1),
                str_arg(c, 2).unwrap_or_else(|| Rc::from("null")),
                3,
            ),
        }
    }
    b.method("DrawTextCentre", |c| {
        let e = el(c)?;
        let (font, text, at) = font_text(c);
        let [x, y, r, g, bb, a] = nums::<6>(c, at);
        e.draw_text_centre(font, &text, x, y, r, g, bb, a);
        Ok(0)
    });
    b.method("DrawTextRight", |c| {
        let e = el(c)?;
        let (font, text, at) = font_text(c);
        let [x, y, r, g, bb, a] = nums::<6>(c, at);
        e.draw_text_right(font, &text, x, y, r, g, bb, a);
        Ok(0)
    });
    b.method("drawTextWithBackground", |c| {
        let e = el(c)?;
        let text = str_arg(c, 2).unwrap_or_else(|| Rc::from("null"));
        let [x, y, r, g, bb, a, px, py, br, bg, bbb, ba] = nums::<12>(c, 3);
        e.draw_text_with_background(
            font_arg(c, 1),
            &text,
            x,
            y,
            [r, g, bb, a],
            px,
            py,
            [br, bg, bbb, ba],
        );
        Ok(0)
    });

    // ---- textures ----
    b.method("DrawTextureAngle", |c| {
        let e = el(c)?;
        let t = tex_arg(c, 1);
        let [x, y, ang] = nums::<3>(c, 2);
        let col = if args(c) >= 8 {
            nums::<4>(c, 5)
        } else {
            [1.0; 4]
        };
        e.draw_texture_angle(t.as_ref(), x, y, ang, col[0], col[1], col[2], col[3]);
        Ok(0)
    });
    b.method("DrawTexture", |c| {
        let e = el(c)?;
        let t = tex_arg(c, 1);
        if args(c) >= 13 {
            let p = nums::<8>(c, 2);
            let [r, g, bb, a] = nums::<4>(c, 10);
            e.draw_texture_4(t.as_ref(), p, r, g, bb, a);
        } else {
            let [x, y, a] = nums::<3>(c, 2);
            e.draw_texture(t.as_ref(), x, y, a);
        }
        Ok(0)
    });
    b.method("DrawTextureCol", |c| {
        let e = el(c)?;
        let t = tex_arg(c, 1);
        let [x, y] = nums::<2>(c, 2);
        e.draw_texture_col(t.as_ref(), x, y, color_arg(c, 4));
        Ok(0)
    });
    b.method("DrawTextureColor", |c| {
        let e = el(c)?;
        let t = tex_arg(c, 1);
        let [x, y, r, g, bb, a] = nums::<6>(c, 2);
        e.draw_texture_color(t.as_ref(), x, y, r, g, bb, a);
        Ok(0)
    });
    b.method("DrawTextureScaled", |c| {
        let e = el(c)?;
        let t = tex_arg(c, 1);
        let [x, y, w, h, a] = nums::<5>(c, 2);
        e.draw_texture_scaled(t.as_ref(), x, y, w, h, a);
        Ok(0)
    });
    b.method("DrawTextureScaledUniform", |c| {
        let e = el(c)?;
        let t = tex_arg(c, 1);
        let [x, y, s, r, g, bb, a] = nums::<7>(c, 2);
        e.draw_texture_scaled_uniform(t.as_ref(), x, y, s, r, g, bb, a);
        Ok(0)
    });
    draw9!(b, "DrawTextureScaledAspect", draw_texture_scaled_aspect);
    draw9!(b, "DrawTextureScaledAspect2", draw_texture_scaled_aspect2);
    draw9!(b, "DrawTextureScaledAspect3", draw_texture_scaled_aspect3);
    b.method("DrawTextureScaledCol", |c| {
        let e = el(c)?;
        let t = tex_arg(c, 1);
        if args(c) == 6 || matches!(c.arg(6), Value::Object(_)) {
            let [x, y, w, h] = nums::<4>(c, 2);
            e.draw_texture_scaled_col_c(t.as_ref(), x, y, w, h, color_arg(c, 6));
        } else {
            let [x, y, w, h, r, g, bb, a] = nums::<8>(c, 2);
            e.draw_texture_scaled_col(t.as_ref(), x, y, w, h, r, g, bb, a);
        }
        Ok(0)
    });
    draw9!(b, "DrawTextureScaledColor", draw_texture_scaled_col);
    b.method("DrawLine", |c| {
        let e = el(c)?;
        let t = tex_arg(c, 1);
        let [x1, y1, x2, y2, th, r, g, bb, a] = nums::<9>(c, 2);
        e.draw_line(t.as_ref(), x1, y1, x2, y2, th as f32, r, g, bb, a);
        Ok(0)
    });
    b.method("DrawPolygon", |c| {
        let e = el(c)?;
        let t = tex_arg(c, 1);
        let p = nums::<8>(c, 2);
        let [r, g, bb, a] = nums::<4>(c, 10);
        e.draw_polygon(t.as_ref(), p, r, g, bb, a);
        Ok(0)
    });
    act!(b, "DrawItemIcon", draw_item_icon);
    act!(b, "DrawScriptItemIcon", draw_script_item_icon);
    draw9!(b, "DrawTextureIcon", draw_texture_icon);
    draw10!(b, "DrawTextureIconMask", draw_texture_icon_mask);
    draw10!(b, "DrawTexturePercentage", draw_texture_percentage);
    draw10!(
        b,
        "DrawTexturePercentageBottomUp",
        draw_texture_percentage_bottom_up
    );
    b.method("DrawSubTextureRGBA", |c| {
        let e = el(c)?;
        let t = tex_arg(c, 1);
        let sub = nums::<4>(c, 2);
        let [x, y, w, h, r, g, bb, a] = nums::<8>(c, 6);
        e.draw_sub_texture_rgba(t.as_ref(), sub, x, y, w, h, r, g, bb, a);
        Ok(0)
    });
    draw9!(b, "DrawTextureTiled", draw_texture_tiled);
    draw9!(b, "DrawTextureTiledX", draw_texture_tiled_x);
    draw9!(b, "DrawTextureTiledY", draw_texture_tiled_y);
    draw9!(b, "DrawTextureTiledYOffset", draw_texture_tiled_y_offset);
    b.method("DrawTextureIgnoreOffset", |c| {
        let e = el(c)?;
        let t = tex_arg(c, 1);
        let [x, y] = nums::<2>(c, 2);
        e.draw_texture_ignore_offset(
            t.as_ref(),
            x,
            y,
            int_arg(c, 4),
            int_arg(c, 5),
            color_arg(c, 6),
        );
        Ok(0)
    });
    for name in ["DrawTexture_FlippedX", "DrawTexture_FlippedXIgnoreOffset"] {
        b.method(name, |c| {
            let e = el(c)?;
            let t = tex_arg(c, 1);
            let [x, y] = nums::<2>(c, 2);
            e.draw_texture_flipped_x(
                t.as_ref(),
                x,
                y,
                int_arg(c, 4),
                int_arg(c, 5),
                color_arg(c, 6),
            );
            Ok(0)
        });
    }
    b.method("DrawUVSliceTexture", |c| {
        let e = el(c)?;
        let t = tex_arg(c, 1);
        let [x, y, w, h] = nums::<4>(c, 2);
        let [xs, ys, xe, ye] = nums::<4>(c, 7);
        e.draw_uv_slice_texture(t.as_ref(), x, y, w, h, color_arg(c, 6), xs, ys, xe, ye);
        Ok(0)
    });

    // ---- geometry and flags ----
    get!(b, "getScrollChildren", get_scroll_children);
    set_bool!(b, "setScrollChildren", set_scroll_children);
    get!(b, "getScrollWithParent", get_scroll_with_parent);
    set_bool!(b, "setScrollWithParent", set_scroll_with_parent);
    set_bool!(b, "setRenderClippedChildren", set_render_clipped_children);
    get!(b, "getAbsoluteX", get_absolute_x);
    get!(b, "getAbsoluteY", get_absolute_y);
    act!(b, "bringToTop", bring_to_top);
    get!(b, "isCapture", is_capture);
    set_bool!(b, "setCapture", set_capture);
    get!(b, "isModalVisible", is_modal_visible);
    get!(b, "isIgnoreLossControl", is_ignore_loss_control);
    set_bool!(b, "setIgnoreLossControl", set_ignore_loss_control);
    b.method("getControls", |c| {
        let e = el(c)?;
        Ok(c.push(Value::Object(Rc::new(UIElementList {
            src: ListSrc::Controls(e),
        }))))
    });
    b.method("setControls", |_c| {
        // The original calls itself forever.
        Err(LuaError::Runtime {
            message: "null".into(),
            class: "java.lang.StackOverflowError",
        })
    });
    get!(b, "isDefaultDraw", is_default_draw);
    set_bool!(b, "setDefaultDraw", set_default_draw);
    get!(b, "isFollowGameWorld", is_follow_game_world);
    set_bool!(b, "setFollowGameWorld", set_follow_game_world);
    get!(b, "getRenderThisPlayerOnly", get_render_this_player_only);
    b.method("setRenderThisPlayerOnly", |c| {
        el(c)?.set_render_this_player_only(int_arg(c, 1));
        Ok(0)
    });
    get!(b, "getHeight", get_height);
    set_num!(b, "setHeight", set_height);
    get!(b, "getWidth", get_width);
    set_num!(b, "setWidth", set_width);
    b.method("getParent", |c| {
        let p = opt_el(el(c)?.get_parent());
        Ok(c.push(p))
    });
    b.method("setParent", |c| {
        let e = el(c)?;
        let p = element_of(&c.arg(1));
        e.set_parent(p.as_deref());
        Ok(0)
    });
    get!(b, "isVisible", is_visible);
    set_bool!(b, "setVisible", set_visible);
    get!(b, "isReallyVisible", is_really_visible);
    get!(b, "getX", get_x);
    set_num!(b, "setX", set_x);
    get!(b, "getY", get_y);
    set_num!(b, "setY", set_y);
    b.method("getXScrolled", |c| {
        let e = el(c)?;
        let p = element_of(&c.arg(1));
        let v = e.get_x_scrolled(p.as_deref());
        Ok(c.push(v))
    });
    b.method("getYScrolled", |c| {
        let e = el(c)?;
        let p = element_of(&c.arg(1));
        let v = e.get_y_scrolled(p.as_deref());
        Ok(c.push(v))
    });
    get!(b, "isEnabled", is_enabled);
    set_bool!(b, "setEnabled", set_enabled);
    b.method("isOverElement", |c| {
        let v = el(c)?.is_over_element(num_arg(c, 1), num_arg(c, 2));
        Ok(c.push(v))
    });
    act!(b, "suspendStencil", suspend_stencil);
    act!(b, "resumeStencil", resume_stencil);
    b.method("setStencilRect", |c| {
        let [x, y, w, h] = nums::<4>(c, 1);
        el(c)?.set_stencil_rect(x, y, w, h);
        Ok(0)
    });
    b.method("setStencilCircle", |c| {
        let [x, y, w, h] = nums::<4>(c, 1);
        el(c)?.set_stencil_circle(x, y, w, h);
        Ok(0)
    });
    act!(b, "clearStencilRect", clear_stencil_rect);
    b.method("repaintStencilRect", |c| {
        let [x, y, w, h] = nums::<4>(c, 1);
        el(c)?.repaint_stencil_rect(x, y, w, h);
        Ok(0)
    });
    b.method("getTable", |c| {
        let t = el(c)?.get_table().map(Value::Table).unwrap_or_default();
        Ok(c.push(t))
    });
    b.method("setTable", |c| {
        let t = match c.arg(1) {
            Value::Table(t) => Some(t),
            _ => None,
        };
        el(c)?.set_table(t);
        Ok(0)
    });
    set_num!(b, "setHeightSilent", set_height_silent);
    set_num!(b, "setWidthSilent", set_width_silent);
    set_num!(b, "setHeightOnly", set_height_only);
    set_num!(b, "setWidthOnly", set_width_only);
    get!(b, "isAnchorTop", is_anchor_top);
    set_bool!(b, "setAnchorTop", set_anchor_top);
    act!(b, "ignoreWidthChange", ignore_width_change);
    act!(b, "ignoreHeightChange", ignore_height_change);
    get!(b, "isAnchorLeft", is_anchor_left);
    set_bool!(b, "setAnchorLeft", set_anchor_left);
    get!(b, "isAnchorRight", is_anchor_right);
    set_bool!(b, "setAnchorRight", set_anchor_right);
    get!(b, "isAnchorBottom", is_anchor_bottom);
    set_bool!(b, "setAnchorBottom", set_anchor_bottom);
    get!(b, "getPlayerContext", get_player_context);
    b.method("setPlayerContext", |c| {
        el(c)?.set_player_context(int_arg(c, 1));
        Ok(0)
    });
    get!(b, "getUIName", get_ui_name);
    b.method("setUIName", |c| {
        el(c)?.set_ui_name(str_arg(c, 1).as_deref());
        Ok(0)
    });
    b.method("clampToParentX", |c| {
        let v = el(c)?.clamp_to_parent_x(num_arg(c, 1));
        Ok(c.push(v))
    });
    b.method("clampToParentY", |c| {
        let v = el(c)?.clamp_to_parent_y(num_arg(c, 1));
        Ok(c.push(v))
    });
    b.method("isPointOver", |c| {
        let v = el(c)?.is_point_over(num_arg(c, 1), num_arg(c, 2));
        Ok(c.push(v))
    });
    get!(b, "isMouseOver", is_mouse_over);
    set_bool!(b, "setWantKeyEvents", set_want_key_events);
    get!(b, "isWantKeyEvents", is_want_key_events);
    set_bool!(b, "setWantExtraMouseEvents", set_want_extra_mouse_events);
    get!(b, "isWantExtraMouseEvents", is_want_extra_mouse_events);
    get!(b, "isForceCursorVisible", is_force_cursor_visible);
    set_bool!(b, "setForceCursorVisible", set_force_cursor_visible);
    act!(b, "StartOutline", start_outline);
    act!(b, "EndOutline", end_outline);
    act!(b, "onresize", onresize);
    b.method("toString", |c| {
        let e = el(c)?;
        let s = match e.get_table() {
            Some(t) => c.thread.tostring(&Value::Table(t))?,
            None => e.to_lua_string(),
        };
        Ok(c.push(s))
    });

    // ---- event entry points (callable from Lua like any public method) ----
    macro_rules! ev_xy_bool {
        ($name:literal, $f:ident) => {
            b.method($name, |c| {
                let e = el(c)?;
                let (x, y) = (num_arg(c, 1), num_arg(c, 2));
                let r = e.$f(c.thread, x, y);
                Ok(c.push(r))
            });
        };
    }
    ev_xy_bool!("onMouseDown", on_mouse_down);
    ev_xy_bool!("onMouseUp", on_mouse_up);
    ev_xy_bool!("onRightMouseDown", on_right_mouse_down);
    ev_xy_bool!("onRightMouseUp", on_right_mouse_up);
    ev_xy_bool!("onMouseMove", on_mouse_move);
    b.method("onMouseUpOutside", |c| {
        let e = el(c)?;
        let (x, y) = (num_arg(c, 1), num_arg(c, 2));
        e.on_mouse_up_outside(c.thread, x, y);
        Ok(0)
    });
    b.method("onMouseMoveOutside", |c| {
        let e = el(c)?;
        let (x, y) = (num_arg(c, 1), num_arg(c, 2));
        e.on_mouse_move_outside(c.thread, x, y);
        Ok(0)
    });
    b.method("onExtendMouseMoveOutside", |c| {
        let e = el(c)?;
        let [dx, dy, x, y] = nums::<4>(c, 1);
        e.on_extend_mouse_move_outside(c.thread, dx, dy, x, y);
        Ok(0)
    });
    b.method("onConsumeMouseMove", |c| {
        let e = el(c)?;
        let [dx, dy, x, y] = nums::<4>(c, 1);
        let r = e.on_consume_mouse_move(c.thread, dx, dy, x, y);
        Ok(c.push(r))
    });
    b.method("onMouseWheel", |c| {
        let e = el(c)?;
        let d = num_arg(c, 1);
        let r = e.on_mouse_wheel(c.thread, d);
        Ok(c.push(r))
    });
    b.method("onConsumeMouseWheel", |c| {
        let e = el(c)?;
        let [d, x, y] = nums::<3>(c, 1);
        let r = e.on_consume_mouse_wheel(c.thread, d, x, y);
        Ok(c.push(r))
    });
    b.method("onMouseButtonDown", |c| {
        let e = el(c)?;
        let (btn, x, y) = (int_arg(c, 1), num_arg(c, 2), num_arg(c, 3));
        e.on_mouse_button_down(c.thread, btn, x, y);
        Ok(0)
    });
    b.method("onConsumeMouseButtonDown", |c| {
        let e = el(c)?;
        let (btn, x, y) = (int_arg(c, 1), num_arg(c, 2), num_arg(c, 3));
        let r = e.on_consume_mouse_button_down(c.thread, btn, x, y);
        Ok(c.push(r))
    });
    b.method("onMouseButtonDownOutside", |c| {
        let e = el(c)?;
        let (btn, x, y) = (int_arg(c, 1), num_arg(c, 2), num_arg(c, 3));
        e.on_mouse_button_down_outside(c.thread, btn, x, y);
        Ok(0)
    });
    b.method("onConsumeMouseButtonUp", |c| {
        let e = el(c)?;
        let (btn, x, y) = (int_arg(c, 1), num_arg(c, 2), num_arg(c, 3));
        let r = e.on_consume_mouse_button_up(c.thread, btn, x, y);
        Ok(c.push(r))
    });
    b.method("onMouseButtonUpOutside", |c| {
        let e = el(c)?;
        let (btn, x, y) = (int_arg(c, 1), num_arg(c, 2), num_arg(c, 3));
        e.on_mouse_button_up_outside(c.thread, btn, x, y);
        Ok(0)
    });
    b.method("onResize", |c| {
        el(c)?.on_resize(c.thread);
        Ok(0)
    });
    b.method("render", |c| {
        el(c)?.render(c.thread);
        Ok(0)
    });
    b.method("update", |c| {
        el(c)?.update(c.thread);
        Ok(0)
    });
    macro_rules! key_fn {
        ($name:literal, $f:ident, void) => {
            b.method($name, |c| {
                let e = el(c)?;
                let k = int_arg(c, 1);
                e.$f(c.thread, k);
                Ok(0)
            });
        };
        ($name:literal, $f:ident) => {
            b.method($name, |c| {
                let e = el(c)?;
                let k = int_arg(c, 1);
                let r = e.$f(c.thread, k);
                Ok(c.push(r))
            });
        };
    }
    key_fn!("isKeyConsumed", is_key_consumed);
    key_fn!("onConsumeKeyPress", on_consume_key_press);
    key_fn!("onKeyPress", on_key_press, void);
    key_fn!("onConsumeKeyRepeat", on_consume_key_repeat);
    key_fn!("onKeyRepeat", on_key_repeat, void);
    key_fn!("onConsumeKeyRelease", on_consume_key_release);
    key_fn!("onKeyRelease", on_key_release, void);
}

fn register_manager(thread: &mut Thread) {
    let b = expose_class(thread, UIMANAGER, None);
    b.field("DEBUGGER_FPS", 60);
    macro_rules! s_nil {
        ($($name:literal),*) => { $( b.static_fn($name, |c| Ok(c.push(Value::Nil))); )* };
    }
    macro_rules! s_noop {
        ($($name:literal),*) => { $( b.static_fn($name, |_c| Ok(0)); )* };
    }
    macro_rules! s_get {
        ($name:literal, |$m:ident| $e:expr) => {
            b.static_fn($name, |c| {
                let v: Value = with_ui(|$m| $e.into());
                Ok(c.push(v))
            });
        };
    }
    macro_rules! s_set {
        ($name:literal, |$m:ident, $c:ident| $e:expr) => {
            b.static_fn($name, |$c| {
                with_ui(|$m| $e);
                Ok(0)
            });
        };
    }

    b.static_fn("AddUI", |c| {
        UIManager::add_ui(el_arg(c, 0)?);
        Ok(0)
    });
    b.static_fn("RemoveElement", |c| {
        UIManager::remove_element(el_arg(c, 0)?);
        Ok(0)
    });
    b.static_fn("clearArrays", |_c| {
        UIManager::clear_arrays();
        Ok(0)
    });
    b.static_fn("updateBeforeFadeOut", |_c| {
        UIManager::update_before_fade_out();
        Ok(0)
    });
    b.static_fn("getUI", |c| {
        Ok(c.push(Value::Object(Rc::new(UIElementList { src: ListSrc::Ui }))))
    });
    b.static_fn("init", |_c| {
        UIManager::init();
        Ok(0)
    });
    b.static_fn("update", |c| {
        UIManager::update(c.thread);
        Ok(0)
    });
    b.static_fn("render", |c| {
        UIManager::render(c.thread);
        Ok(0)
    });
    b.static_fn("resize", |_c| {
        UIManager::resize();
        Ok(0)
    });
    b.static_fn("DrawTexture", |c| {
        let Some(t) = tex_arg(c, 0) else {
            return Err(LuaError::Runtime {
                message: "null".into(),
                class: "java.lang.NullPointerException",
            });
        };
        let (x, y) = (
            num_arg(c, 1) + t.offset_x as f64,
            num_arg(c, 2) + t.offset_y as f64,
        );
        let (w, h, a) = if c.nargs() >= 6 {
            (num_arg(c, 3) as i32, num_arg(c, 4) as i32, num_arg(c, 5))
        } else {
            (t.width as i32, t.height as i32, 1.0)
        };
        with_ui(|m| m.sprite.renderi(Some(&t), x as i32, y as i32, w, h, Rgba(1.0, 1.0, 1.0, a as f32)));
        Ok(0)
    });
    // FadeIn(seconds) / FadeIn(playerIndex, seconds); FadeOut likewise.
    b.static_fn("FadeIn", |c| {
        if c.nargs() >= 2 {
            let (p, s) = (num_arg(c, 0) as usize, num_arg(c, 1) as i32);
            with_ui(|m| {
                let fps = m.lock_fps;
                if let Some(f) = m.player_fade_info.get_mut(p) {
                    f.fade_in(s, fps)
                }
            });
        } else {
            let s = num_arg(c, 0);
            with_ui(|m| m.fade(s, false));
        }
        Ok(0)
    });
    b.static_fn("FadeOut", |c| {
        if c.nargs() >= 2 {
            let (p, s) = (num_arg(c, 0) as usize, num_arg(c, 1) as i32);
            with_ui(|m| {
                let fps = m.lock_fps;
                if let Some(f) = m.player_fade_info.get_mut(p) {
                    f.fade_out(s, fps)
                }
            });
        } else {
            let s = num_arg(c, 0);
            with_ui(|m| m.fade(s, true));
        }
        Ok(0)
    });
    b.static_fn("getFadeAlpha", |c| {
        let v = if c.nargs() >= 1 {
            let p = num_arg(c, 0) as usize;
            with_ui(|m| m.player_fade_info.get(p).map_or(0.0, |f| f.fade_alpha)) as f64
        } else {
            with_ui(|m| m.fade_alpha) as f64
        };
        Ok(c.push(v))
    });
    s_set!("setFadeAlpha", |m, c| m.set_fade_alpha(num_arg(c, 0)));
    s_get!("getFadeInTimeMax", |m| m.fade_in_time_max);
    s_set!("setFadeInTimeMax", |m, c| m.fade_in_time_max =
        num_arg(c, 0) as i32);
    s_get!("getFadeInTime", |m| m.fade_in_time);
    s_set!("setFadeInTime", |m, c| m.set_fade_in_time(num_arg(c, 0)));
    s_get!("isFadingOut", |m| m.fading_out);
    s_set!("setFadingOut", |m, c| m.fading_out = bool_arg(c, 0));
    s_get!("isbFadeBeforeUI", |m| m.fade_before_ui);
    s_set!("setbFadeBeforeUI", |m, c| m.fade_before_ui = bool_arg(c, 0));
    s_set!("setFadeBeforeUI", |m, c| {
        let p = int_arg(c, 0) as usize;
        let v = bool_arg(c, 1);
        if let Some(f) = m.player_fade_info.get_mut(p) {
            f.fade_before_ui = v;
        }
    });
    s_set!("setFadeTime", |m, c| {
        let p = num_arg(c, 0) as usize;
        let v = num_arg(c, 1) as i32;
        if let Some(f) = m.player_fade_info.get_mut(p) {
            f.fade_time = v;
        }
    });
    s_get!("getLastMouseX", |m| m.last_mouse_x);
    s_get!("getLastMouseY", |m| m.last_mouse_y);
    s_set!("setLastMouseX", |m, c| m.last_mouse_x =
        num_arg(c, 0) as i32);
    s_set!("setLastMouseY", |m, c| m.last_mouse_y =
        num_arg(c, 0) as i32);
    s_get!("getLastOffX", |m| m.last_off_x as f64);
    s_get!("getLastOffY", |m| m.last_off_y as f64);
    s_set!("setLastOffX", |m, c| m.last_off_x = num_arg(c, 0) as f32);
    s_set!("setLastOffY", |m, c| m.last_off_y = num_arg(c, 0) as f32);
    s_get!("getLastAlpha", |m| m.last_alpha as f64);
    s_set!("setLastAlpha", |m, c| m.last_alpha = num_arg(c, 0) as f32);
    s_get!("getBlack", |m| m.black.clone());
    s_set!("setBlack", |m, c| m.black = c.arg(0));
    s_get!("getMouseArrow", |m| m.mouse_arrow.clone());
    s_set!("setMouseArrow", |m, c| m.mouse_arrow = c.arg(0));
    s_get!("getMouseExamine", |m| m.mouse_examine.clone());
    s_set!("setMouseExamine", |m, c| m.mouse_examine = c.arg(0));
    s_get!("getMouseAttack", |m| m.mouse_attack.clone());
    s_set!("setMouseAttack", |m, c| m.mouse_attack = c.arg(0));
    s_get!("getMouseGrab", |m| m.mouse_grab.clone());
    s_set!("setMouseGrab", |m, c| m.mouse_grab = c.arg(0));
    s_get!("getLastMouseTexture", |m| m.last_mouse_texture.clone());
    s_set!("setLastMouseTexture", |m, c| m.last_mouse_texture =
        c.arg(0));
    b.static_fn("getModal", |c| {
        let v = opt_el(with_ui(|m| m.modal.clone()));
        Ok(c.push(v))
    });
    b.static_fn("setModal", |c| {
        let e = element_of(&c.arg(0));
        with_ui(|m| m.modal = e);
        Ok(0)
    });
    s_get!("isShowPausedMessage", |m| m.show_paused_message);
    s_set!("setShowPausedMessage", |m, c| m.show_paused_message =
        bool_arg(c, 0));
    s_get!("isShowLuaDebuggerOnError", |m| m.show_lua_debugger_on_error);
    s_set!("setShowLuaDebuggerOnError", |m, c| m
        .show_lua_debugger_on_error =
        bool_arg(c, 0));
    s_set!("setVisibleAllUI", |m, c| m.visible_all_ui = bool_arg(c, 0));
    s_get!("isFBOActive", |m| m.use_ui_fbo);
    s_get!("getMillisSinceLastUpdate", |m| m.ui_update_interval_ms
        as f64);
    s_get!("getSecondsSinceLastUpdate", |m| m.ui_update_interval_ms
        as f64
        / 1000.0);
    s_get!("getMillisSinceLastRender", |m| m.ui_render_interval_ms
        as f64);
    s_get!("getSecondsSinceLastRender", |m| m.ui_render_interval_ms
        as f64
        / 1000.0);
    s_get!("isRendering", |m| m.rendering);
    s_get!("isUpdating", |m| m.updating);
    b.static_fn("getDoubleClickInterval", |c| {
        Ok(c.push(UIManager::get_double_click_interval()))
    });
    b.static_fn("getDoubleClickDist", |c| {
        Ok(c.push(UIManager::get_double_click_dist()))
    });
    b.static_fn("isDoubleClick", |c| {
        let [x1, y1, x2, y2, t] = nums::<5>(c, 0);
        Ok(c.push(UIManager::is_double_click(x1, y1, x2, y2, t)))
    });
    b.static_fn("setPlayerInventory", |c| {
        if int_arg(c, 0) == 0 {
            let (inv, loot) = (element_of(&c.arg(1)), element_of(&c.arg(2)));
            with_ui(|m| {
                m.player_inventory_ui = inv;
                m.player_loot_ui = loot;
            });
        }
        Ok(0)
    });
    b.static_fn("setPlayerInventoryTooltip", |c| {
        if int_arg(c, 0) == 0 {
            let (inv, loot) = (element_of(&c.arg(1)), element_of(&c.arg(2)));
            with_ui(|m| {
                m.player_inventory_tooltip = inv;
                m.player_loot_tooltip = loot;
            });
        }
        Ok(0)
    });
    b.static_fn("isMouseOverInventory", |c| {
        Ok(c.push(UIManager::is_mouse_over_inventory()))
    });
    b.static_fn("isForceCursorVisible", |c| {
        Ok(c.push(UIManager::is_force_cursor_visible()))
    });
    b.static_fn("isModalVisible", |c| {
        Ok(c.push(UIManager::is_modal_visible()))
    });
    b.static_fn("onKeyPress", |c| {
        let k = int_arg(c, 0);
        let r = UIManager::on_key_press(c.thread, k);
        Ok(c.push(r))
    });
    b.static_fn("onKeyRepeat", |c| {
        let k = int_arg(c, 0);
        let r = UIManager::on_key_repeat(c.thread, k);
        Ok(c.push(r))
    });
    b.static_fn("onKeyRelease", |c| {
        let k = int_arg(c, 0);
        let r = UIManager::on_key_release(c.thread, k);
        Ok(c.push(r))
    });
    b.static_fn("tableget", |c| {
        let v = match c.arg(0) {
            Value::Table(_) => {
                let (t, k) = (c.arg(0), c.arg(1));
                c.thread.table_get(&t, &k)?
            }
            _ => Value::Nil,
        };
        Ok(c.push(v))
    });
    b.static_fn("getBlinkAlpha", |c| {
        Ok(c.push(UIManager::get_blink_alpha(int_arg(c, 0)) as f64))
    });
    b.static_fn("getSyncedIconIndex", |c| {
        Ok(c.push(UIManager::get_synced_icon_index(int_arg(c, 1))))
    });
    b.static_fn("resetSyncedIconIndex", |c| {
        with_ui(|m| m.player_blink_info[0].synced_icon_index = 0);
        Ok(c.push(0))
    });
    b.static_fn("trySetProgressBarValue", |c| Ok(c.push(false))); // TODO(world): ActionProgressBar
                                                                  // TODO(world): world objects and Java-side widgets not ported yet.
    s_nil!(
        "getSpeedControls",
        "getClock",
        "getToolTip",
        "getDebugConsole",
        "getMoodleUI",
        "getProgressBar",
        "getPicked",
        "getLastPicked",
        "getPickedTile",
        "getPickedTileLocal",
        "getRightDownObject",
        "getTileFromMouse",
        "getDoneTutorials",
        "getDefaultThread"
    );
    s_noop!(
        "closeContainers",
        "CloseContainers",
        "CreateFBO",
        "setPicked",
        "setLastPicked",
        "setClock",
        "setToolTip",
        "setSpeedControls",
        "setDebugConsole",
        "setMoodleUI",
        "setProgressBar",
        "setPickedTile",
        "setPickedTileLocal",
        "setRightDownObject",
        "setDoneTutorials",
        "setUI",
        "debugBreakpoint"
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use pz_ui::{TextAlign, UiDrawCmd};

    const CLASS: &str = r#"
        log = {}
        local function L(s) log[#log + 1] = s end
        Widget = {}
        Widget.__index = Widget
        function Widget:new(name, x, y, w, h)
            local o = setmetatable({}, self)
            o.name, o.x, o.y, o.width, o.height = name, x, y, w, h
            o.Type = "Widget"
            return o
        end
        function Widget:instantiate()
            self.javaObject = UIElement.new(self)
            self.javaObject:setX(self.x)
            self.javaObject:setY(self.y)
            self.javaObject:setWidth(self.width)
            self.javaObject:setHeight(self.height)
            -- ISUIElement:instantiate sets anchors after the size, which
            -- resets lastwidth/lastheight.
            self.javaObject:setAnchorLeft(true)
            self.javaObject:setAnchorTop(true)
            self.javaObject:setConsumeMouseEvents(true)
        end
        function Widget:addChild(o)
            if not self.javaObject then self:instantiate() end
            if not o.javaObject then o:instantiate() end
            o.parent = self
            self.javaObject:AddChild(o.javaObject)
        end
        function Widget:addToUIManager()
            if not self.javaObject then self:instantiate() end
            UIManager.AddUI(self.javaObject)
        end
        function Widget:prerender() L("prerender " .. self.name) end
        function Widget:render()
            L("render " .. self.name)
            self.javaObject:DrawTextureScaledColor(nil, 1, 2, 3, 4, 1, 1, 1, 1)
            self.javaObject:DrawText(UIFont.Medium, self.name, 0, 0, 1, 1, 1, 1)
        end
        function Widget:update() L("update " .. self.name) end
        function Widget:onMouseDown(x, y) L("down " .. self.name .. " " .. x .. " " .. y) return true end
        function Widget:onMouseUp(x, y) L("up " .. self.name .. " " .. x .. " " .. y) return true end
        function Widget:onMouseDownOutside(x, y) L("downOutside " .. self.name) end
        function Widget:onMouseMove(dx, dy) L("move " .. self.name .. " " .. dx .. " " .. dy) end
        function Widget:onMouseWheel(d) L("wheel " .. self.name .. " " .. d) return true end
        function Widget:onKeyPress(k) L("key " .. self.name .. " " .. k) end
        function Widget:isKeyConsumed(k) return true end
    "#;

    fn setup() -> Thread {
        UIManager::reset();
        with_ui(|m| m.time_override_ms = Some(1_000_000));
        let mut t = Thread::new(TableRef::new());
        pz_kahlua::stdlib::install(&mut t, None).unwrap();
        register(&mut t);
        run(&mut t, CLASS);
        t
    }

    fn run(t: &mut Thread, src: &str) {
        let p = pz_kahlua::compiler::compile(src, Some("test"), Some("test"), None).unwrap();
        let f = Value::Closure(pz_kahlua::vm::LuaClosure::new(
            Rc::new(p),
            t.globals().clone(),
        ));
        if let Err((e, tr)) = t.pcall(&f, &[]) {
            panic!("lua error: {e:?}\n{tr}");
        }
    }

    fn log(t: &Thread) -> Vec<String> {
        let l = t.globals().get_str("log");
        let l = l.as_table().unwrap();
        (1..=l.len())
            .map(|i| l.get_int(i).as_str().unwrap().to_string())
            .collect()
    }

    fn clear_log(t: &mut Thread) {
        run(t, "log = {}");
    }

    fn eval(t: &mut Thread, expr: &str) -> Value {
        run(t, &format!("__r = {expr}"));
        t.globals().get_str("__r")
    }

    fn tick(t: &mut Thread, at: i64) {
        with_ui(|m| m.time_override_ms = Some(at));
        UIManager::update(t);
        with_ui(|m| m.mouse.end_frame());
    }

    #[test]
    fn render_and_update_order_with_children() {
        let mut t = setup();
        run(
            &mut t,
            r#"
            root = Widget:new("root", 10, 20, 200, 200)
            a = Widget:new("a", 5, 5, 50, 50)
            b = Widget:new("b", 60, 5, 50, 50)
            root:addChild(a); root:addChild(b)
            root:addToUIManager()
        "#,
        );
        tick(&mut t, 1_000_000);
        assert_eq!(log(&t), ["update root", "update a", "update b"]);
        clear_log(&mut t);
        UIManager::render(&mut t);
        assert_eq!(
            log(&t),
            [
                "prerender root",
                "prerender a",
                "render a",
                "prerender b",
                "render b",
                "render root"
            ]
        );
        // Absolute positions in the draw list: root at (10,20), a at (15,25).
        let quads: Vec<(f32, f32)> = with_ui(|m| {
            m.sprite
                .cmds
                .iter()
                .filter_map(|c| match c {
                    UiDrawCmd::Quad { x, y, .. } => Some((*x, *y)),
                    _ => None,
                })
                .collect()
        });
        assert_eq!(quads, [(16.0, 27.0), (71.0, 27.0), (11.0, 22.0)]);
        let texts: Vec<(String, f32, f32, Option<UIFont>, TextAlign)> = with_ui(|m| {
            m.sprite
                .cmds
                .iter()
                .filter_map(|c| match c {
                    UiDrawCmd::Text {
                        text,
                        x,
                        y,
                        font,
                        align,
                        ..
                    } => Some((text.to_string(), *x, *y, *font, *align)),
                    _ => None,
                })
                .collect()
        });
        assert_eq!(
            texts[0],
            (
                "a".to_string(),
                15.0,
                25.0,
                Some(UIFont::Medium),
                TextAlign::Left
            )
        );
        assert_eq!(
            eval(&mut t, "a.javaObject:getAbsoluteX()").as_num(),
            Some(15.0)
        );
        assert!(eval(&mut t, "a.javaObject:getParent() == root.javaObject").truthy());
        assert_eq!(
            eval(&mut t, "root.javaObject:getControls():size()").as_num(),
            Some(2.0)
        );
        assert!(eval(&mut t, "UIManager.getUI():contains(root.javaObject)").truthy());
        assert!(eval(&mut t, "a.javaObject:isReallyVisible()").truthy());
    }

    #[test]
    fn update_lua_only_on_ticks() {
        let mut t = setup();
        run(
            &mut t,
            r#"w = Widget:new("w", 0, 0, 10, 10); w:addToUIManager()"#,
        );
        tick(&mut t, 1_000_000);
        tick(&mut t, 1_000_050);
        tick(&mut t, 1_000_100);
        assert_eq!(log(&t), ["update w", "update w"]);
    }

    #[test]
    fn mouse_dispatch_topmost_first_with_local_coords() {
        let mut t = setup();
        run(
            &mut t,
            r#"
            back = Widget:new("back", 0, 0, 300, 300)
            front = Widget:new("front", 100, 100, 100, 100)
            child = Widget:new("child", 10, 20, 30, 30)
            front:addChild(child)
            back:addToUIManager(); front:addToUIManager()
        "#,
        );
        tick(&mut t, 1_000_000);
        clear_log(&mut t);
        with_ui(|m| {
            m.mouse.x = 115;
            m.mouse.y = 125;
            m.mouse.down[0] = true;
        });
        tick(&mut t, 1_000_010);
        // front is on top; its child gets local coordinates; child consumes,
        // so front gets onMouseDownOutside and back is not reached.
        let l = log(&t);
        assert!(l.contains(&"move front 115 125".to_string()), "{l:?}");
        let downs: Vec<_> = l
            .iter()
            .filter(|s| s.contains("down") || s.contains("Outside"))
            .cloned()
            .collect();
        assert_eq!(downs, ["down child 5 5", "downOutside front"]);
        clear_log(&mut t);
        with_ui(|m| m.mouse.down[0] = false);
        tick(&mut t, 1_000_020);
        let ups: Vec<_> = log(&t)
            .into_iter()
            .filter(|s| s.starts_with("up"))
            .collect();
        assert_eq!(ups, ["up child 5 5"]);
    }

    #[test]
    fn capture_receives_clicks_outside_bounds() {
        let mut t = setup();
        run(
            &mut t,
            r#"
            a = Widget:new("a", 0, 0, 10, 10); a:addToUIManager()
            b = Widget:new("b", 500, 500, 10, 10); b:addToUIManager()
        "#,
        );
        tick(&mut t, 1_000_000);
        run(&mut t, "a.javaObject:setCapture(true); log = {}");
        with_ui(|m| {
            m.mouse.x = 300;
            m.mouse.y = 300;
            m.mouse.down[0] = true;
        });
        tick(&mut t, 1_000_010);
        let downs: Vec<_> = log(&t)
            .into_iter()
            .filter(|s| s.starts_with("down"))
            .collect();
        assert_eq!(downs, ["downOutside b", "down a 300 300"]);
    }

    #[test]
    fn z_order_bring_to_top_always_on_top_back_most() {
        let mut t = setup();
        run(
            &mut t,
            r#"
            a = Widget:new("a", 0, 0, 10, 10); a:addToUIManager()
            b = Widget:new("b", 0, 0, 10, 10); b:addToUIManager()
            c = Widget:new("c", 0, 0, 10, 10); c:addToUIManager()
            d = Widget:new("d", 0, 0, 10, 10); d:addToUIManager()
        "#,
        );
        tick(&mut t, 1_000_000);
        let order = |t: &mut Thread| -> String {
            match eval(t, "(function() local s = '' local ui = UIManager.getUI() for i = 0, ui:size() - 1 do s = s .. ui:get(i):getTable().name end return s end)()") {
                Value::Str(s) => s.to_string(),
                v => panic!("{v:?}"),
            }
        };
        assert_eq!(order(&mut t), "abcd");
        run(&mut t, "a.javaObject:bringToTop()");
        assert_eq!(order(&mut t), "abcd", "bringToTop is deferred to update");
        tick(&mut t, 1_000_200);
        assert_eq!(order(&mut t), "bcda");
        run(&mut t, "b.javaObject:setAlwaysOnTop(true)");
        tick(&mut t, 1_000_400);
        assert_eq!(order(&mut t), "cdab");
        run(&mut t, "d.javaObject:backMost()");
        tick(&mut t, 1_000_600);
        assert_eq!(order(&mut t), "dcab");
        // Re-adding moves c to the end; always-on-top b then goes after it.
        run(&mut t, "c:addToUIManager()");
        tick(&mut t, 1_000_800);
        assert_eq!(order(&mut t), "dacb");
        run(&mut t, "UIManager.RemoveElement(a.javaObject)");
        tick(&mut t, 1_001_000);
        assert_eq!(order(&mut t), "dcb");
    }

    #[test]
    fn child_bring_to_top_reorders_controls() {
        let mut t = setup();
        run(
            &mut t,
            r#"
            p = Widget:new("p", 0, 0, 100, 100)
            x = Widget:new("x", 0, 0, 10, 10); y = Widget:new("y", 0, 0, 10, 10)
            p:addChild(x); p:addChild(y); p:addToUIManager()
            x.javaObject:bringToTop()
        "#,
        );
        tick(&mut t, 1_000_000);
        assert!(eval(&mut t, "p.javaObject:getControls():get(1) == x.javaObject").truthy());
        assert_eq!(eval(&mut t, "UIManager.getUI():size()").as_num(), Some(1.0));
    }

    #[test]
    fn wheel_keys_and_anchors() {
        let mut t = setup();
        run(
            &mut t,
            r#"
            p = Widget:new("p", 0, 0, 100, 100)
            k = Widget:new("k", 50, 10, 40, 40)
            p:addChild(k); p:addToUIManager()
            k.javaObject:setAnchorLeft(false); k.javaObject:setAnchorRight(true)
            p.javaObject:setWantKeyEvents(true)
        "#,
        );
        tick(&mut t, 1_000_000);
        clear_log(&mut t);
        with_ui(|m| {
            m.mouse.x = 60;
            m.mouse.y = 20;
            m.mouse.wheel = -1;
        });
        tick(&mut t, 1_000_010);
        assert!(log(&t).contains(&"wheel k 1".to_string()), "{:?}", log(&t));
        assert!(!log(&t).contains(&"wheel p 1".to_string()));
        clear_log(&mut t);
        assert!(UIManager::on_key_press(&mut t, 28));
        assert_eq!(log(&t), ["key p 28"]);
        run(&mut t, "p.javaObject:setWidth(150)");
        tick(&mut t, 1_000_500);
        assert_eq!(eval(&mut t, "k.javaObject:getX()").as_num(), Some(100.0));
    }

    #[test]
    fn stencil_and_overloads() {
        let mut t = setup();
        run(
            &mut t,
            r#"
            s = Widget:new("s", 10, 10, 100, 100); s:instantiate()
            s.javaObject:setStencilRect(0, 0, 50, 50)
            s.javaObject:DrawTextCentre("hi", 5, 6, 1, 1, 1, 1)
            s.javaObject:DrawTextRight(UIFont.Small, "yo", 5, 6, 1, 1, 1, 1)
            s.javaObject:clearStencilRect()
        "#,
        );
        let cmds = with_ui(|m| m.sprite.cmds.clone());
        // setStencilRect: the stencil quad covers the element's rectangle.
        assert!(cmds.iter().any(|c| matches!(c, UiDrawCmd::Quad { texture: None, x, y, w, h, .. } if (*x, *y, *w, *h) == (10.0, 10.0, 50.0, 50.0))));
        let texts: Vec<_> = cmds.iter().filter(|c| matches!(c, UiDrawCmd::Text { .. })).collect();
        assert!(matches!(texts[0], UiDrawCmd::Text { align: TextAlign::Centre, font: None, x, .. } if *x == 15.0));
        assert!(matches!(texts[1], UiDrawCmd::Text { align: TextAlign::Right, font: Some(UIFont::Small), .. }));
        // clearStencilRect at level 0 clears the stencil buffer.
        assert!(matches!(cmds.last(), Some(UiDrawCmd::Render(pz_render::DrawCmd::Clear(1280)))));
        assert_eq!(eval(&mut t, "tostring(s.javaObject:getUIName())").as_str(), Some("Widget: s"));
    }

    // ---- Java subclasses through the pz-ui extension slot ----

    use pz_ui::UIElementExt;
    use std::cell::RefCell;

    const TEST_SUB: &str = "zombie.ui.TestSubclass";
    const TEST_SUPERS: &[&str] = &[UIELEMENT, "zombie.ui.ITestListener"];

    /// A subclass overriding `render` (calling `super.render()` in the
    /// middle, touching the element and re-entering Lua around it) and
    /// `onMouseDown` (no super call) when `override_render`; everything
    /// else falls back.
    struct TestSub {
        log: RefCell<Vec<String>>,
        override_render: bool,
    }

    impl UIElementExt for TestSub {
        fn class_name(&self) -> &'static str {
            TEST_SUB
        }
        fn super_classes(&self) -> &'static [&'static str] {
            TEST_SUPERS
        }
        fn as_any(&self) -> &dyn Any {
            self
        }
        fn render(&self, el: &UIElement, t: &mut Thread) {
            if !self.override_render {
                return el.render_base(t);
            }
            self.log.borrow_mut().push("hook before".into());
            // The element's state is free while the hook runs.
            el.set_width(el.get_width() + 1.0);
            let f = t.globals().get_str("onSubRender");
            if !f.is_nil() {
                t.call(&f, &[el.to_value()]).unwrap();
            }
            el.render_base(t);
            self.log.borrow_mut().push("hook after".into());
        }
        fn on_mouse_down(&self, el: &UIElement, t: &mut Thread, x: f64, y: f64) -> bool {
            if !self.override_render {
                return el.on_mouse_down_base(t, x, y);
            }
            self.log.borrow_mut().push(format!("hook down {x} {y}"));
            true
        }
    }

    fn setup_sub() -> Thread {
        let mut t = setup();
        let b = expose_class(&mut t, TEST_SUB, Some(UIELEMENT));
        b.ctor(|c| {
            let table = c.arg(0).as_table().cloned();
            let override_render = c.arg(1).truthy();
            let e = UIElement::new_ext(table, Box::new(TestSub { log: RefCell::default(), override_render }));
            Ok(c.push(Value::Object(e as Rc<dyn LuaObject>)))
        });
        b.method("subOnly", |c| {
            let e = el(c)?;
            let n = e.ext::<TestSub>().map_or(-1.0, |s| s.log.borrow().len() as f64);
            Ok(c.push(n))
        });
        // `instanceof` (bind_globals also defines `log`, so the test
        // classes are loaded again after it).
        crate::bind_globals::register(&mut t);
        run(&mut t, "UIManager = zombie.ui.UIManager");
        run(&mut t, CLASS);
        run(
            &mut t,
            r#"
            Sub = setmetatable({}, {__index = Widget})
            Sub.__index = Sub
            function Sub:instantiate()
                self.javaObject = TestSubclass.new(self, self.override)
                self.javaObject:setX(self.x)
                self.javaObject:setY(self.y)
                self.javaObject:setWidth(self.width)
                self.javaObject:setHeight(self.height)
            end
        "#,
        );
        t
    }

    fn sub_log(t: &mut Thread, var: &str) -> Vec<String> {
        let v = eval(t, &format!("{var}.javaObject"));
        let e = element_of(&v).unwrap();
        let l = e.ext::<TestSub>().unwrap().log.borrow().clone();
        l
    }

    /// The identity Lua sees is the subclass; it inherits UIElement's
    /// methods, keeps its own, and AddChild/AddUI accept it.
    #[test]
    fn subclass_identity_inheritance_and_add_child() {
        let mut t = setup_sub();
        run(
            &mut t,
            r#"
            root = Widget:new("root", 0, 0, 300, 300)
            s = Sub:new("s", 10, 10, 50, 50); s.override = true
            root:addChild(s)
            root:addToUIManager()
            s2 = Sub:new("s2", 0, 0, 10, 10); s2:addToUIManager()
        "#,
        );
        // AddUI takes effect at the next update.
        tick(&mut t, 1_000_000);
        let v = eval(&mut t, "s.javaObject");
        let Value::Object(o) = &v else { panic!() };
        assert_eq!(o.class_name(), TEST_SUB);
        assert_eq!(o.super_classes(), TEST_SUPERS);
        assert_eq!(eval(&mut t, "instanceof(s.javaObject, 'UIElement')").truthy(), true);
        assert_eq!(eval(&mut t, "instanceof(s.javaObject, 'TestSubclass')").truthy(), true);
        assert_eq!(eval(&mut t, "instanceof(s.javaObject, 'ITestListener')").truthy(), true);
        assert_eq!(eval(&mut t, "instanceof(root.javaObject, 'TestSubclass')").truthy(), false);
        // Inherited (UIElement) and own methods on one object.
        assert_eq!(eval(&mut t, "s.javaObject:getX()").as_num(), Some(10.0));
        assert_eq!(eval(&mut t, "s.javaObject:subOnly()").as_num(), Some(0.0));
        // In the child list and the UI list as the same object.
        assert_eq!(eval(&mut t, "root.javaObject:getControls():get(0) == s.javaObject").truthy(), true);
        assert_eq!(eval(&mut t, "UIManager.getUI():contains(s2.javaObject)").truthy(), true);
        assert!(eval(&mut t, "tostring(s.javaObject)").as_str().is_some());
    }

    /// Overridden hooks run in place of the base and may call it
    /// (`super.render()`); hooks left alone fall back to the base.
    #[test]
    fn subclass_hooks_override_and_fall_back() {
        let mut t = setup_sub();
        run(
            &mut t,
            r#"
            root = Widget:new("root", 0, 0, 300, 300)
            s = Sub:new("s", 10, 10, 50, 50); s.override = true
            plain = Sub:new("plain", 100, 10, 50, 50); plain.override = false
            root:addChild(s); root:addChild(plain)
            root:addToUIManager()
        "#,
        );
        tick(&mut t, 1_000_000);
        clear_log(&mut t);
        UIManager::render(&mut t);
        assert_eq!(
            log(&t),
            ["prerender root", "prerender s", "render s", "prerender plain", "render plain", "render root"]
        );
        assert_eq!(sub_log(&mut t, "s"), ["hook before", "hook after"]);
        assert!(sub_log(&mut t, "plain").is_empty());
        // update has no override: the base runs the Lua update.
        clear_log(&mut t);
        tick(&mut t, 1_000_200);
        assert_eq!(log(&t), ["update root", "update s", "update plain"]);
        // onMouseDown: the override replaces the base (no Lua callback for
        // `s`), the fallback reaches Lua for `plain`.
        clear_log(&mut t);
        let r = eval(&mut t, "root.javaObject");
        let root = element_of(&r).unwrap();
        root.on_mouse_down(&mut t, 20.0, 20.0);
        assert_eq!(sub_log(&mut t, "s").last().map(String::as_str), Some("hook down 10 10"));
        root.on_mouse_down(&mut t, 120.0, 20.0);
        assert!(log(&t).contains(&"down plain 20 10".to_string()), "{:?}", log(&t));
    }

    /// No element borrow is held while a hook runs: the hook mutates the
    /// element and calls Lua, which re-enters the element (setters,
    /// AddChild, RemoveChild, bringToTop) before the base render.
    #[test]
    fn subclass_hook_reentry_holds_no_borrow() {
        let mut t = setup_sub();
        run(
            &mut t,
            r#"
            root = Widget:new("root", 0, 0, 300, 300)
            s = Sub:new("s", 10, 10, 50, 50); s.override = true
            root:addChild(s)
            root:addToUIManager()
            extra = Widget:new("extra", 1, 1, 5, 5); extra:instantiate()
            function onSubRender(jo)
                jo:setX(jo:getX() + 1)
                jo:setHeight(jo:getHeight() + 1)
                jo:AddChild(extra.javaObject)
                jo:RemoveChild(extra.javaObject)
                jo:AddChild(extra.javaObject)
                jo:bringToTop()
                L2 = jo:getControls():size()
            end
        "#,
        );
        tick(&mut t, 1_000_000);
        UIManager::render(&mut t);
        assert_eq!(eval(&mut t, "s.javaObject:getX()").as_num(), Some(11.0));
        assert_eq!(eval(&mut t, "s.javaObject:getWidth()").as_num(), Some(51.0));
        assert_eq!(eval(&mut t, "L2").as_num(), Some(1.0));
        // The base render then drew the child added from inside the hook.
        assert!(log(&t).contains(&"render extra".to_string()), "{:?}", log(&t));
    }

}
