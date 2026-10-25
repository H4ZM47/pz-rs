//! `zombie.ui.ObjectTooltip` for Lua and `UIManager`: the shared tooltip
//! (`UIManager.toolTip`, `updateTooltip`), the `ObjectTooltip` class and
//! `ObjectTooltip$Layout` / `$LayoutItem` handles over the pz-ui tooltip API
//! ([`pz_ui::tooltip`]).

use super::host::with_host;
use crate::bind_ui::{element_of, ui_font_from_name, UIELEMENT, UIFONT};
use crate::expose::{bool_arg, enum_value, expose_class, int_arg, num_arg, str_arg, JavaEnum};
use pz_kahlua::value::LuaObject;
use pz_kahlua::{LuaError, LuaResult, NativeCall, Thread, Value};
use pz_render::{LuaTexture, Rgba, Texture};
use pz_ui::tooltip::{Layout, ObjectTooltip, OBJECT_TOOLTIP};
use pz_ui::{with_ui, UIElement};
use std::any::Any;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

pub const LAYOUT: &str = "zombie.ui.ObjectTooltip$Layout";
pub const LAYOUT_ITEM: &str = "zombie.ui.ObjectTooltip$LayoutItem";

thread_local! {
    /// `UIManager.lastPicked` (the picked object's id).
    static LAST_PICKED: Cell<Option<u64>> = const { Cell::new(None) };
}

/// The tooltip font option and highlight colours from `Core`.
fn core_tooltip_font() -> String {
    let core = crate::bind_core::shared_core();
    let mut c = core.borrow_mut();
    let g = c.get_good_highlited_color();
    let b = c.get_bad_highlited_color();
    ObjectTooltip::set_highlight_colors(Rgba(g.r, g.g, g.b, g.a), Rgba(b.r, b.g, b.b, b.a));
    c.get_option_tooltip_font()
}

/// `new ObjectTooltip()`.
pub fn create_tooltip() -> Rc<UIElement> {
    ObjectTooltip::create(&core_tooltip_font())
}

/// `UIManager.updateTooltip(mx, my)`: the shared tooltip follows the
/// picked object with a special tooltip while the mouse is over no UI.
pub fn update_tooltip(_t: &mut Thread, mx: i32, my: i32) {
    let Some(tip) = with_ui(|m| m.tool_tip.clone()) else {
        LAST_PICKED.with(|l| l.set(None));
        return;
    };
    let tt = ObjectTooltip::of(&tip).unwrap();
    let (fx, fy) = (mx as f64, my as f64);
    let uis = with_ui(|m| m.ui.clone());
    let over_ui = uis.iter().rev().any(|ui| {
        !Rc::ptr_eq(ui, &tip)
            && ui.is_visible()
            && ui.is_over_element(fx, fy)
            && (ui.get_max_draw_height() == -1.0 || fy < ui.get_y() + ui.get_max_draw_height())
    });
    let mut over_object = None;
    if !over_ui {
        if let Some(tile) = with_host(|h| h.picking.picked_tile()) {
            let last = LAST_PICKED.with(|l| l.get());
            over_object = Some(tile.id);
            if Some(tile.id) != last {
                tt.life.borrow_mut().target_alpha = 0.0;
                if tile.have_special_tooltip {
                    let same = tt
                        .life
                        .borrow()
                        .object
                        .as_ref()
                        .is_some_and(|o| o.id == tile.id);
                    if !same {
                        tt.show_object(&tip, tile, (mx + 24) as f64, (my + 24) as f64);
                        if tip.is_visible() {
                            tt.life.borrow_mut().show_delay = 0;
                        }
                    } else {
                        tt.life.borrow_mut().target_alpha = 1.0;
                    }
                }
            }
        }
    }
    LAST_PICKED.with(|l| l.set(over_object));
    let (alpha, target) = {
        let l = tt.life.borrow();
        (l.alpha, l.target_alpha)
    };
    if over_object.is_none() || (alpha <= 0.0 && target <= 0.0) {
        tt.hide(&tip);
    }
    if tip.is_visible() {
        let (x, y) = with_ui(|m| (m.mouse.x, m.mouse.y));
        tip.set_x((x + 24) as f64);
        tip.set_y((y + 24) as f64);
    }
}

// ---- Lua ----------------------------------------------------------------------

fn tooltip(c: &NativeCall<'_>) -> LuaResult<Rc<UIElement>> {
    let e = element_of(&c.arg(0)).ok_or_else(|| LuaError::runtime("Expected ObjectTooltip"))?;
    if ObjectTooltip::of(&e).is_none() {
        return Err(LuaError::runtime(format!(
            "Expected ObjectTooltip but got {}",
            e.class_name()
        )));
    }
    Ok(e)
}

fn tt(e: &UIElement) -> &ObjectTooltip {
    ObjectTooltip::of(e).unwrap()
}

fn font_arg(c: &NativeCall<'_>, i: usize) -> Option<pz_text::UIFont> {
    match c.arg(i) {
        Value::Object(o) => o
            .as_any()
            .downcast_ref::<JavaEnum>()
            .filter(|e| e.class == UIFONT)
            .and_then(|e| ui_font_from_name(e.name)),
        _ => None,
    }
}

fn tex_arg(c: &NativeCall<'_>, i: usize) -> Option<Texture> {
    match c.arg(i) {
        Value::Object(o) => o.as_any().downcast_ref::<LuaTexture>().map(LuaTexture::get),
        _ => None,
    }
}

fn s(c: &NativeCall<'_>, i: usize) -> String {
    str_arg(c, i).map(|s| s.to_string()).unwrap_or_default()
}

fn f(c: &NativeCall<'_>, i: usize) -> f64 {
    num_arg(c, i)
}

/// A Lua `ObjectTooltip$Layout`: the layout and the chain behind `next`.
pub struct LayoutObject(pub Rc<RefCell<Layout>>);

impl LuaObject for LayoutObject {
    fn class_name(&self) -> &str {
        LAYOUT
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// A Lua `ObjectTooltip$LayoutItem`: a row of a layout.
pub struct LayoutItemObject {
    layout: Rc<RefCell<Layout>>,
    index: usize,
}

impl LuaObject for LayoutItemObject {
    fn class_name(&self) -> &str {
        LAYOUT_ITEM
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

fn layout_arg(c: &NativeCall<'_>, i: usize) -> LuaResult<Rc<RefCell<Layout>>> {
    match c.arg(i) {
        Value::Object(o) => match o.as_any().downcast_ref::<LayoutObject>() {
            Some(l) => Ok(l.0.clone()),
            None => Err(LuaError::runtime(format!(
                "Expected ObjectTooltip$Layout but got {}",
                o.class_name()
            ))),
        },
        _ => Err(LuaError::Runtime {
            message: "layout is null".into(),
            class: "java.lang.NullPointerException",
        }),
    }
}

fn with_item<R>(
    c: &NativeCall<'_>,
    f: impl FnOnce(&mut pz_ui::tooltip::LayoutItem) -> R,
) -> LuaResult<R> {
    match c.arg(0) {
        Value::Object(o) => match o.as_any().downcast_ref::<LayoutItemObject>() {
            Some(it) => {
                let mut l = it.layout.borrow_mut();
                let item = l
                    .items
                    .get_mut(it.index)
                    .ok_or_else(|| LuaError::runtime("layout item was freed"))?;
                Ok(f(item))
            }
            None => Err(LuaError::runtime("Expected ObjectTooltip$LayoutItem")),
        },
        _ => Err(LuaError::runtime("Expected ObjectTooltip$LayoutItem")),
    }
}

/// Register `ObjectTooltip`, `$Layout` and `$LayoutItem`.
pub fn register(thread: &mut Thread) {
    let b = expose_class(thread, OBJECT_TOOLTIP, Some(UIELEMENT));
    b.field("alphaStep", pz_ui::tooltip::ALPHA_STEP as f64);
    b.ctor(|c| Ok(c.push(Value::Object(create_tooltip() as Rc<dyn LuaObject>))));
    b.static_fn("checkFont", |_c| {
        ObjectTooltip::check_font(&core_tooltip_font());
        Ok(0)
    });
    b.method("getFont", |c| {
        tooltip(c)?;
        Ok(c.push(enum_value(UIFONT, &format!("{:?}", ObjectTooltip::font()))))
    });
    b.method("getLineSpacing", |c| {
        tooltip(c)?;
        Ok(c.push(ObjectTooltip::line_spacing() as f64))
    });
    // The measure-only overrides of UIElement's text and texture draws.
    b.method("DrawText", |c| {
        let e = tooltip(c)?;
        if c.nargs() >= 9 {
            tt(&e).draw_text(
                &e,
                font_arg(c, 1),
                &s(c, 2),
                f(c, 3),
                f(c, 4),
                f(c, 5),
                f(c, 6),
                f(c, 7),
                f(c, 8),
            );
        } else if !tt(&e).is_measure_only() {
            e.draw_text_default(
                &s(c, 1),
                f(c, 2),
                f(c, 3),
                f(c, 4),
                f(c, 5),
                f(c, 6),
                f(c, 7),
            );
        }
        Ok(0)
    });
    b.method("DrawTextCentre", |c| {
        let e = tooltip(c)?;
        if c.nargs() >= 9 {
            tt(&e).draw_text_centre(
                &e,
                font_arg(c, 1),
                &s(c, 2),
                f(c, 3),
                f(c, 4),
                f(c, 5),
                f(c, 6),
                f(c, 7),
                f(c, 8),
            );
        } else {
            // DrawTextCentre(String, ...) is not overridden.
            e.draw_text_centre(
                None,
                &s(c, 1),
                f(c, 2),
                f(c, 3),
                f(c, 4),
                f(c, 5),
                f(c, 6),
                f(c, 7),
            );
        }
        Ok(0)
    });
    b.method("DrawTextRight", |c| {
        let e = tooltip(c)?;
        if c.nargs() >= 9 {
            tt(&e).draw_text_right(
                &e,
                font_arg(c, 1),
                &s(c, 2),
                f(c, 3),
                f(c, 4),
                f(c, 5),
                f(c, 6),
                f(c, 7),
                f(c, 8),
            );
        } else {
            e.draw_text_right(
                None,
                &s(c, 1),
                f(c, 2),
                f(c, 3),
                f(c, 4),
                f(c, 5),
                f(c, 6),
                f(c, 7),
            );
        }
        Ok(0)
    });
    b.method("DrawValueRight", |c| {
        let e = tooltip(c)?;
        tt(&e).draw_value_right(
            &e,
            int_arg(c, 1),
            int_arg(c, 2),
            int_arg(c, 3),
            bool_arg(c, 4),
        );
        Ok(0)
    });
    // DrawValueRightNoPlus(int|float, x, y): a whole number takes the int
    // overload.
    b.method("DrawValueRightNoPlus", |c| {
        let e = tooltip(c)?;
        let v = f(c, 1);
        if v.fract() == 0.0 {
            tt(&e).draw_value_right_no_plus_int(&e, v as i32, int_arg(c, 2), int_arg(c, 3));
        } else {
            tt(&e).draw_value_right_no_plus_float(&e, v as f32, int_arg(c, 2), int_arg(c, 3));
        }
        Ok(0)
    });
    b.method("DrawTextureScaled", |c| {
        let e = tooltip(c)?;
        tt(&e).draw_texture_scaled(
            &e,
            tex_arg(c, 1).as_ref(),
            f(c, 2),
            f(c, 3),
            f(c, 4),
            f(c, 5),
            f(c, 6),
        );
        Ok(0)
    });
    b.method("DrawTextureScaledAspect", |c| {
        let e = tooltip(c)?;
        tt(&e).draw_texture_scaled_aspect(
            &e,
            tex_arg(c, 1).as_ref(),
            f(c, 2),
            f(c, 3),
            f(c, 4),
            f(c, 5),
            f(c, 6),
            f(c, 7),
            f(c, 8),
            f(c, 9),
        );
        Ok(0)
    });
    b.method("DrawProgressBar", |c| {
        let e = tooltip(c)?;
        tt(&e).draw_progress_bar(
            &e,
            int_arg(c, 1),
            int_arg(c, 2),
            int_arg(c, 3),
            int_arg(c, 4),
            f(c, 5) as f32,
            f(c, 6),
            f(c, 7),
            f(c, 8),
            f(c, 9),
        );
        Ok(0)
    });
    b.method("hide", |c| {
        let e = tooltip(c)?;
        tt(&e).hide(&e);
        Ok(0)
    });
    b.method("adjustWidth", |c| {
        let e = tooltip(c)?;
        tt(&e).adjust_width(&e, int_arg(c, 1), &s(c, 2));
        Ok(0)
    });
    b.method("beginLayout", |c| {
        let e = tooltip(c)?;
        core_tooltip_font();
        let l = tt(&e).begin_layout();
        Ok(
            c.push(Value::Object(Rc::new(LayoutObject(Rc::new(RefCell::new(
                l,
            )))))),
        )
    });
    b.method("endLayout", |c| {
        let e = tooltip(c)?;
        if !c.arg(1).is_nil() {
            let l = layout_arg(c, 1)?;
            // free(): the items and chain are cleared (and pooled in Java).
            *l.borrow_mut() = Layout::default();
            let _ = &e;
        }
        Ok(0)
    });
    b.method("getTexture", |c| {
        let e = tooltip(c)?;
        let t = tt(&e).texture.clone();
        Ok(c.push(
            t.map(|t| Value::Object(Rc::new(LuaTexture::new(t)) as Rc<dyn LuaObject>))
                .unwrap_or_default(),
        ))
    });
    b.method("setCharacter", |c| {
        let e = tooltip(c)?;
        let v = c.arg(1);
        tt(&e).set_character((!v.is_nil()).then_some(v));
        Ok(0)
    });
    b.method("getCharacter", |c| {
        let e = tooltip(c)?;
        Ok(c.push(tt(&e).character().unwrap_or_default()))
    });
    b.method("setMeasureOnly", |c| {
        let e = tooltip(c)?;
        tt(&e).set_measure_only(bool_arg(c, 1));
        Ok(0)
    });
    b.method("isMeasureOnly", |c| {
        let e = tooltip(c)?;
        Ok(c.push(tt(&e).is_measure_only()))
    });
    b.method("getWeightOfStack", |c| {
        let e = tooltip(c)?;
        Ok(c.push(tt(&e).weight_of_stack() as f64))
    });
    b.method("setWeightOfStack", |c| {
        let e = tooltip(c)?;
        tt(&e).set_weight_of_stack(f(c, 1) as f32);
        Ok(0)
    });

    let l = expose_class(thread, LAYOUT, None);
    l.method("addItem", |c| {
        let layout = layout_arg(c, 0)?;
        let index = {
            let mut l = layout.borrow_mut();
            l.add_item();
            l.items.len() - 1
        };
        Ok(c.push(Value::Object(Rc::new(LayoutItemObject { layout, index }))))
    });
    l.method("setMinLabelWidth", |c| {
        layout_arg(c, 0)?
            .borrow_mut()
            .set_min_label_width(int_arg(c, 1));
        Ok(0)
    });
    l.method("setMinValueWidth", |c| {
        layout_arg(c, 0)?
            .borrow_mut()
            .set_min_value_width(int_arg(c, 1));
        Ok(0)
    });
    l.method("render", |c| {
        let layout = layout_arg(c, 0)?;
        let e = element_of(&c.arg(3))
            .filter(|e| ObjectTooltip::of(e).is_some())
            .ok_or_else(|| LuaError::runtime("Expected ObjectTooltip"))?;
        let y = layout.borrow_mut().render(int_arg(c, 1), int_arg(c, 2), &e);
        Ok(c.push(y as f64))
    });
    l.method("free", |c| {
        *layout_arg(c, 0)?.borrow_mut() = Layout::default();
        Ok(0)
    });

    let i = expose_class(thread, LAYOUT_ITEM, None);
    i.method("setLabel", |c| {
        let (t, r, g, b, a) = (
            s(c, 1),
            f(c, 2) as f32,
            f(c, 3) as f32,
            f(c, 4) as f32,
            f(c, 5) as f32,
        );
        with_item(c, |it| it.set_label(&t, r, g, b, a))?;
        Ok(0)
    });
    i.method("setValue", |c| {
        let (t, r, g, b, a) = (
            s(c, 1),
            f(c, 2) as f32,
            f(c, 3) as f32,
            f(c, 4) as f32,
            f(c, 5) as f32,
        );
        with_item(c, |it| it.set_value(&t, r, g, b, a))?;
        Ok(0)
    });
    i.method("setValueRight", |c| {
        let (v, hg) = (int_arg(c, 1), bool_arg(c, 2));
        core_tooltip_font();
        with_item(c, |it| it.set_value_right(v, hg))?;
        Ok(0)
    });
    i.method("setValueRightNoPlus", |c| {
        let v = f(c, 1);
        with_item(c, |it| {
            if v.fract() == 0.0 {
                it.set_value_right_no_plus_int(v as i32)
            } else {
                it.set_value_right_no_plus_float(v as f32)
            }
        })?;
        Ok(0)
    });
    i.method("setProgress", |c| {
        let (p, r, g, b, a) = (
            f(c, 1) as f32,
            f(c, 2) as f32,
            f(c, 3) as f32,
            f(c, 4) as f32,
            f(c, 5) as f32,
        );
        with_item(c, |it| it.set_progress(p, r, g, b, a))?;
        Ok(0)
    });
    i.method("calcSizes", |c| {
        with_item(c, |it| it.calc_sizes())?;
        Ok(0)
    });
    i.method("reset", |c| {
        with_item(c, |it| {
            it.label = None;
            it.value = None;
            it.has_value = false;
            it.could_have_value = false;
            it.right_justify = false;
            it.progress_fraction = -1.0;
        })?;
        Ok(0)
    });
}
