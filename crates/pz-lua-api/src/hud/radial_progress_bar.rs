//! `zombie.ui.RadialProgressBar`: a texture revealed clockwise from 12
//! o'clock in eight triangular segments (`ISRadialProgressBar`).

use crate::bind_core::LuaColorInfo;
use pz_kahlua::{Thread, Value};
use pz_render::{Rgba, Texture};
use pz_ui::{with_ui, UIElement, UIElementExt};
use std::any::Any;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

pub const RADIAL_PROGRESS_BAR: &str = "zombie.ui.RadialProgressBar";
const SUPERS: &[&str] = &["zombie.ui.UIElement"];
// The original's truncated constants, not TAU / FRAC_PI_2.
#[allow(clippy::approx_constant)]
const TWO_PI: f32 = 6.283185;
#[allow(clippy::approx_constant)]
const PI_OVER_TWO: f32 = 1.570796;

/// `segments`: each triangle's corners (also its texture coordinates),
/// clockwise from the top centre; the third is the centre.
const SEGMENTS: [[[f32; 2]; 3]; 8] = [
    [[0.5, 0.0], [1.0, 0.0], [0.5, 0.5]],
    [[1.0, 0.0], [1.0, 0.5], [0.5, 0.5]],
    [[1.0, 0.5], [1.0, 1.0], [0.5, 0.5]],
    [[1.0, 1.0], [0.5, 1.0], [0.5, 0.5]],
    [[0.5, 1.0], [0.0, 1.0], [0.5, 0.5]],
    [[0.0, 1.0], [0.0, 0.5], [0.5, 0.5]],
    [[0.0, 0.5], [0.0, 0.0], [0.5, 0.5]],
    [[0.0, 0.0], [0.5, 0.0], [0.5, 0.5]],
];

/// The colour: `ColorInfo(1, 1, 1, 1)` or the Lua `ColorInfo` passed to
/// `setColor` (kept by reference, as the original keeps the object).
enum RadialColor {
    Own(Rgba),
    Lua(Value),
}

/// `zombie.ui.RadialProgressBar`.
pub struct RadialProgressBar {
    radial_texture: RefCell<Option<Texture>>,
    radial_color: RefCell<RadialColor>,
    delta_value: Cell<f32>,
}

impl RadialProgressBar {
    /// `new RadialProgressBar(table, tex)`.
    pub fn create(table: Option<pz_kahlua::TableRef>, tex: Option<Texture>) -> Rc<UIElement> {
        UIElement::new_ext(
            table,
            Box::new(RadialProgressBar {
                radial_texture: RefCell::new(tex),
                radial_color: RefCell::new(RadialColor::Own(Rgba(1.0, 1.0, 1.0, 1.0))),
                delta_value: Cell::new(1.0),
            }),
        )
    }

    /// `setValue(delta)`: clamped to 0..1.
    pub fn set_value(&self, v: f32) {
        self.delta_value.set(v.clamp(0.0, 1.0));
    }
    pub fn get_value(&self) -> f32 {
        self.delta_value.get()
    }
    pub fn set_texture(&self, t: Option<Texture>) {
        *self.radial_texture.borrow_mut() = t;
    }
    pub fn get_texture(&self) -> Option<Texture> {
        self.radial_texture.borrow().clone()
    }
    /// `setColor(color)` with a Lua `ColorInfo` (or nil, which the
    /// original would fail on at render).
    pub fn set_color_value(&self, v: Value) {
        *self.radial_color.borrow_mut() = RadialColor::Lua(v);
    }
    pub fn set_color(&self, c: Rgba) {
        *self.radial_color.borrow_mut() = RadialColor::Own(c);
    }

    fn color(&self) -> Option<Rgba> {
        match &*self.radial_color.borrow() {
            RadialColor::Own(c) => Some(*c),
            RadialColor::Lua(Value::Object(o)) => {
                o.as_any().downcast_ref::<LuaColorInfo>().map(|c| {
                    let c = c.c.get();
                    Rgba(c.r, c.g, c.b, c.a)
                })
            }
            RadialColor::Lua(_) => None,
        }
    }
}

impl UIElementExt for RadialProgressBar {
    fn class_name(&self) -> &'static str {
        RADIAL_PROGRESS_BAR
    }
    fn super_classes(&self) -> &'static [&'static str] {
        SUPERS
    }
    fn as_any(&self) -> &dyn Any {
        self
    }

    /// `render()` (no `super.render()`: the Lua table's `prerender` and
    /// `render` and any children are not drawn): the full segments up to
    /// the value, then the partial one to the edge point at its angle.
    fn render(&self, el: &UIElement, _t: &mut Thread) {
        if !el.is_enabled() || !el.is_visible() {
            return;
        }
        if let Some(p) = el.get_parent() {
            let mdh = p.st().max_draw_height;
            if mdh != -1 && mdh as f64 <= el.get_y() {
                return;
            }
        }
        let Some(tex) = self.radial_texture.borrow().clone() else {
            return;
        };
        // A null ColorInfo throws in the original before anything is drawn.
        let Some(color) = self.color() else { return };
        let dx = (el.get_x_scroll() + el.get_absolute_x() + tex.offset_x as f64) as f32;
        let dy = (el.get_y_scroll() + el.get_absolute_y() + tex.offset_y as f64) as f32;
        let (uv_x, uv_y) = (tex.x_start, tex.y_start);
        let (uv_w, uv_h) = (tex.x_end - tex.x_start, tex.y_end - tex.y_start);
        let (w, h) = (el.get_width() as f32, el.get_height() as f32);
        let center_x = dx + 0.5 * w;
        let center_y = dy + 0.5 * h;
        let percent = self.delta_value.get();
        let angle = percent * TWO_PI - PI_OVER_TWO;
        let (ax, ay) = ((angle as f64).cos() as f32, (angle as f64).sin() as f32);
        let (scalar, tex_scalar) = if (w / 2.0 / ax).abs() < (h / 2.0 / ay).abs() {
            ((w / 2.0 / ax).abs(), (0.5 / ax).abs())
        } else {
            ((h / 2.0 / ay).abs(), (0.5 / ay).abs())
        };
        let tx = center_x + ax * scalar;
        let ty = center_y + ay * scalar;
        let uvx = 0.5 + ax * tex_scalar;
        let uvy = 0.5 + ay * tex_scalar;
        let index = if percent <= 0.0 {
            -1
        } else {
            (percent * 8.0) as i32
        };
        let (tw, th) = (tex.width as f32, tex.height as f32);
        let p = |v: [f32; 2]| [dx + v[0] * tw, dy + v[1] * th];
        let uv = |v: [f32; 2]| [uv_x + v[0] * uv_w, uv_y + v[1] * uv_h];
        for (i, s) in SEGMENTS.iter().enumerate() {
            if i as i32 > index {
                continue;
            }
            let (p1, uv1) = if i as i32 != index {
                (p(s[1]), uv(s[1]))
            } else {
                ([tx, ty], [uv_x + uvx * uv_w, uv_y + uvy * uv_h])
            };
            let points = [p(s[0]), p1, p(s[2]), p(s[2])];
            let uvs = [uv(s[0]), uv1, uv(s[2]), uv(s[2])];
            with_ui(|m| m.sprite.render_poly_uv(Some(&tex), points, color, uvs));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::host::{install, HudHost};
    use super::super::test_util::*;
    use super::*;
    use pz_ui::tooltip::java_float_to_string as jf;
    use pz_ui::UiDrawCmd;

    fn polys() -> Vec<String> {
        let v = with_ui(|m| {
            m.sprite
                .cmds
                .iter()
                .filter_map(|c| match c {
                    UiDrawCmd::Poly {
                        texture,
                        points,
                        color,
                        uv,
                    } => {
                        let mut s = format!(
                            "poly {}",
                            texture
                                .as_ref()
                                .map_or("-".to_string(), |t| t.name.to_string())
                        );
                        for p in points {
                            s.push_str(&format!(" {} {}", jf(p[0]), jf(p[1])));
                        }
                        s.push_str(&format!(
                            " {} {} {} {}",
                            jf(color.0),
                            jf(color.1),
                            jf(color.2),
                            jf(color.3)
                        ));
                        for p in uv {
                            s.push_str(&format!(" {} {}", jf(p[0]), jf(p[1])));
                        }
                        Some(s)
                    }
                    _ => None,
                })
                .collect()
        });
        with_ui(|m| m.sprite.clear());
        v
    }

    /// Parity with the original `RadialProgressBar` (oracle `radial.txt`):
    /// zero, partial and whole segments, each boundary, full, clamping, a
    /// wide element, texture offsets and atlas coordinates, a colour,
    /// the parent's draw-height clip, hidden, disabled, no texture.
    #[test]
    fn matches_radial_oracle() {
        let Some(text) = oracle("radial.txt") else {
            eprintln!("PZ_HUD_ORACLE_DIR not set; skipping");
            return;
        };
        install_textures_uv(&[(
            "media/ui/radial.png".into(),
            64,
            48,
            2.0,
            3.0,
            [0.25, 0.125, 0.75, 0.5],
        )]);
        install(HudHost::default());
        let mut t = thread();
        let parent = UIElement::new(None);
        parent.set_x(100.0);
        parent.set_y(50.0);
        parent.set_width(300.0);
        parent.set_height(300.0);
        let r =
            RadialProgressBar::create(None, pz_render::get_shared_texture("media/ui/radial.png"));
        r.set_x(10.0);
        r.set_y(20.0);
        r.set_width(64.0);
        r.set_height(48.0);
        parent.add_child(&r);
        let b = r.ext::<RadialProgressBar>().unwrap();
        with_ui(|m| m.sprite.clear());
        let mut got = Vec::new();
        let case = |got: &mut Vec<String>, r: &UIElement, t: &mut Thread, l: &str| {
            r.render(t);
            got.push(format!("case {l}"));
            got.extend(polys());
        };
        for v in [
            0f32, 0.05, 0.125, 0.3, 0.5, 0.62, 0.875, 0.99, 1.0, -0.5, 1.5,
        ] {
            b.set_value(v);
            got.push(format!("value {} -> {}", jf(v), jf(b.get_value())));
            case(&mut got, &r, &mut t, &format!("v {}", jf(v)));
        }
        r.set_width(80.0);
        r.set_height(40.0);
        b.set_value(0.4);
        case(&mut got, &r, &mut t, "wide");
        b.set_value(0.7);
        case(&mut got, &r, &mut t, "wide 0.7");
        let ci = crate::bind_core::LuaColorInfo::new(pz_core::core::ColorInfo {
            r: 0.2,
            g: 0.4,
            b: 0.6,
            a: 0.8,
        });
        b.set_color_value(Value::Object(ci));
        case(&mut got, &r, &mut t, "colour");
        parent.st_mut().max_draw_height = 20;
        case(&mut got, &r, &mut t, "clipped");
        parent.st_mut().max_draw_height = 21;
        case(&mut got, &r, &mut t, "unclipped");
        r.set_visible(false);
        case(&mut got, &r, &mut t, "hidden");
        r.set_visible(true);
        r.st_mut().enabled = false;
        case(&mut got, &r, &mut t, "disabled");
        r.st_mut().enabled = true;
        b.set_texture(None);
        case(&mut got, &r, &mut t, "no texture");
        let want: Vec<String> = text
            .lines()
            .filter(|l| !l.starts_with("fake ") && !l.starts_with("tex "))
            .map(str::to_string)
            .collect();
        assert_eq!(got, want);
    }

    /// The Lua class `ISRadialProgressBar` builds and drives.
    #[test]
    fn lua_methods() {
        install_textures(&[("media/ui/radial.png", 32, 32)]);
        install(HudHost::default());
        let mut t = thread();
        let tex =
            crate::bind_render::texture_value(pz_render::get_shared_texture("media/ui/radial.png"));
        t.globals().set_str("tex", tex);
        run(
            &mut t,
            "o = {}; r = RadialProgressBar.new(o, tex)
             r:setValue(2); v = r:getValue(); r:setTexture(nil); n = r:getTexture()
             r:setTexture(tex); same = r:getTexture() == tex",
        );
        assert_eq!(eval(&mut t, "v").as_num(), Some(1.0));
        assert!(eval(&mut t, "n").is_nil());
        assert!(eval(&mut t, "same").truthy());
        assert!(eval(&mut t, "r:getTable() == o").truthy());
    }
}
