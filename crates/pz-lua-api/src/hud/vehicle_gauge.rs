//! `zombie.ui.VehicleGauge`: a dashboard dial (`ISVehicleGauge`) with a
//! red needle from `minAngle` to `maxAngle`.

use pz_kahlua::Thread;
use pz_render::{Rgba, Texture};
use pz_ui::{with_ui, UIElement, UIElementExt};
use std::any::Any;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

pub const VEHICLE_GAUGE: &str = "zombie.ui.VehicleGauge";
const SUPERS: &[&str] = &["zombie.ui.UIElement"];

/// `zombie.ui.VehicleGauge`.
pub struct VehicleGauge {
    pub needle_x: i32,
    pub needle_y: i32,
    pub min_angle: f32,
    pub max_angle: f32,
    value: Cell<f32>,
    texture: RefCell<Option<Texture>>,
    needle_width: Cell<i32>,
}

impl VehicleGauge {
    /// `new VehicleGauge(texture, needleX, needleY, minAngle, maxAngle)`:
    /// the element takes the texture's size.
    pub fn create(
        texture: Texture,
        needle_x: i32,
        needle_y: i32,
        min_angle: f32,
        max_angle: f32,
    ) -> Rc<UIElement> {
        let (w, h) = (texture.width as f32, texture.height as f32);
        let e = UIElement::new_ext(
            None,
            Box::new(VehicleGauge {
                needle_x,
                needle_y,
                min_angle,
                max_angle,
                value: Cell::new(0.0),
                texture: RefCell::new(Some(texture)),
                needle_width: Cell::new(45),
            }),
        );
        {
            let mut s = e.st_mut();
            s.width = w;
            s.height = h;
        }
        e
    }

    pub fn set_needle_width(&self, w: i32) {
        self.needle_width.set(w);
    }

    /// `setValue(value)`: capped at 1 (not floored).
    pub fn set_value(&self, v: f32) {
        self.value.set(v.min(1.0));
    }

    pub fn value(&self) -> f32 {
        self.value.get()
    }

    pub fn set_texture(&self, t: Option<Texture>) {
        *self.texture.borrow_mut() = t;
    }
}

impl UIElementExt for VehicleGauge {
    fn class_name(&self) -> &'static str {
        VEHICLE_GAUGE
    }
    fn super_classes(&self) -> &'static [&'static str] {
        SUPERS
    }
    fn as_any(&self) -> &dyn Any {
        self
    }

    /// `render()`: the children and Lua `render` first, then the dial and
    /// the needle (a one-pixel red line, its tip's y rounded up).
    fn render(&self, el: &UIElement, t: &mut Thread) {
        if !el.is_visible() {
            return;
        }
        el.render_base(t);
        let tex = self.texture.borrow().clone();
        el.draw_texture(tex.as_ref(), 0.0, 0.0, 1.0);
        let (min, max, v) = (self.min_angle, self.max_angle, self.value.get());
        let radians = if min < max {
            ((min + (max - min) * v) as f64).to_radians()
        } else {
            ((max + (max - min) * (1.0 - v)) as f64).to_radians()
        };
        let nw = self.needle_width.get() as f64;
        let (base_x, base_y) = (self.needle_x as f64, self.needle_y as f64);
        let tip_x = self.needle_x as f64 + nw * radians.cos();
        let tip_y = (self.needle_y as f64 + nw * radians.sin()).ceil();
        let dx = el.get_absolute_x() as i32;
        let dy = el.get_absolute_y() as i32;
        with_ui(|m| {
            m.sprite.renderline(
                None,
                dx + base_x as i32,
                dy + base_y as i32,
                dx + tip_x as i32,
                dy + tip_y as i32,
                Rgba(1.0, 0.0, 0.0, 1.0),
                1.0,
            )
        });
    }
}

#[cfg(test)]
mod tests {
    use super::super::host::{install, HudHost};
    use super::super::test_util::*;
    use super::*;
    use pz_ui::UiDrawCmd;

    fn lines() -> Vec<String> {
        let mut v = draw_lines();
        let l: Vec<String> = with_ui(|m| {
            m.sprite
                .cmds
                .iter()
                .filter_map(|c| match c {
                    UiDrawCmd::Line {
                        texture,
                        x1,
                        y1,
                        x2,
                        y2,
                        thickness,
                        color,
                    } => Some(format!(
                        "line {} {} {} {} {} {} {} {} {} {}",
                        texture
                            .as_ref()
                            .map_or("-".to_string(), |t| t.name.to_string()),
                        *x1 as i32,
                        *y1 as i32,
                        *x2 as i32,
                        *y2 as i32,
                        jf(color.0),
                        jf(color.1),
                        jf(color.2),
                        jf(color.3),
                        jf(*thickness)
                    )),
                    _ => None,
                })
                .collect()
        });
        // The gauge draws its dial, then its needle.
        v.extend(l);
        with_ui(|m| m.sprite.clear());
        v
    }

    /// Parity with the original `VehicleGauge` (oracle `gauge.txt`): the
    /// size from the texture, the needle for rising and falling angle
    /// ranges and ranges past 0..180, values across and past the range,
    /// a narrower needle and another texture, hidden.
    #[test]
    fn matches_gauge_oracle() {
        let Some(text) = oracle("gauge.txt") else {
            eprintln!("PZ_HUD_ORACLE_DIR not set; skipping");
            return;
        };
        install_textures_ex(&oracle_textures_ex(&text));
        install(HudHost::default());
        let mut t = thread();
        let parent = UIElement::new(None);
        parent.set_x(30.0);
        parent.set_y(40.0);
        let tex = pz_render::get_shared_texture("media/ui/gauge.png").unwrap();
        let mut got = Vec::new();
        with_ui(|m| m.sprite.clear());
        for (min, max) in [
            (180.0f32, 0.0f32),
            (0.0, 180.0),
            (200.0, -20.0),
            (-30.0, 210.0),
        ] {
            let g = VehicleGauge::create(tex.clone(), 64, 60, min, max);
            g.set_x(5.0);
            g.set_y(7.0);
            parent.add_child(&g);
            got.push(format!(
                "gauge {} {} {} {}",
                jf(min),
                jf(max),
                jf(g.get_width() as f32),
                jf(g.get_height() as f32)
            ));
            for v in [0f32, 0.1, 0.25, 0.333, 0.5, 0.77, 1.0, 1.5, -0.25] {
                g.ext::<VehicleGauge>().unwrap().set_value(v);
                g.render(&mut t);
                got.push(format!("case {} {} v {}", jf(min), jf(max), jf(v)));
                got.extend(lines());
            }
            parent.remove_child(&g);
        }
        let g = VehicleGauge::create(tex, 40, 37, 180.0, 0.0);
        let gg = g.ext::<VehicleGauge>().unwrap();
        gg.set_needle_width(20);
        gg.set_value(0.6);
        gg.set_texture(pz_render::get_shared_texture("media/ui/gauge2.png"));
        g.render(&mut t);
        got.push("case narrow".into());
        got.extend(lines());
        g.set_visible(false);
        g.render(&mut t);
        got.push("case hidden".into());
        got.extend(lines());
        let want: Vec<String> = text
            .lines()
            .filter(|l| !l.starts_with("fake ") && !l.starts_with("tex "))
            .map(normalise_oracle)
            .collect();
        assert_eq!(got, want);
    }

    /// `ISVehicleGauge:instantiate` builds it from Lua; the table's
    /// `render` runs before the dial.
    #[test]
    fn lua_construction() {
        install_textures(&[("media/ui/gauge.png", 100, 50)]);
        install(HudHost::default());
        let mut t = thread();
        t.globals().set_str(
            "tex",
            crate::bind_render::texture_value(pz_render::get_shared_texture("media/ui/gauge.png")),
        );
        run(
            &mut t,
            "g = VehicleGauge.new(tex, 40, 37, 180, 0); g:setNeedleWidth(30); g:setValue(3)
             w = g:getWidth(); h = g:getHeight()
             ok, err = pcall(function() return VehicleGauge.new(nil, 1, 2, 3, 4) end)",
        );
        assert_eq!(eval(&mut t, "w").as_num(), Some(100.0));
        assert_eq!(eval(&mut t, "h").as_num(), Some(50.0));
        assert!(!eval(&mut t, "ok").truthy());
        let e = crate::bind_ui::element_of(&t.globals().get_str("g")).unwrap();
        assert_eq!(e.ext::<VehicleGauge>().unwrap().value(), 1.0);
    }
}
