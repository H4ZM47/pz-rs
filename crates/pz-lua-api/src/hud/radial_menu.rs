//! `zombie.ui.RadialMenu`: the ring of slices `ISRadialMenu` builds (the
//! controller and quick-action radial menus).

use crate::bind_misc::UITransition;
use pz_kahlua::Thread;
use pz_render::{Rgba, Texture};
use pz_text::UIFont;
use pz_ui::{with_ui, TextAlign, UIElement, UIElementExt};
use std::any::Any;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

pub const RADIAL_MENU: &str = "zombie.ui.RadialMenu";
const SUPERS: &[&str] = &["zombie.ui.UIElement"];

/// `RadialMenu.Slice`.
#[derive(Clone, Default)]
pub struct Slice {
    pub text: Option<String>,
    pub texture: Option<Texture>,
}

impl Slice {
    fn is_empty(&self) -> bool {
        self.text.is_none() && self.texture.is_none()
    }
}

/// The `JoypadManager.instance` calls the menu makes.
#[derive(Clone, Copy, Default)]
pub struct JoypadView {
    pub connected: bool,
    pub aiming_applied: bool,
    pub aiming: (f32, f32),
}

#[cfg(test)]
thread_local! {
    pub(crate) static TEST_JOYPAD: Cell<Option<JoypadView>> = const { Cell::new(None) };
}

fn joypad_view(j: i32) -> JoypadView {
    #[cfg(test)]
    if let Some(v) = TEST_JOYPAD.with(Cell::get) {
        return v;
    }
    JoypadView {
        connected: crate::bind_joypad::is_joypad_connected(j),
        aiming_applied: crate::bind_joypad::is_aiming_axis_being_applied(j),
        aiming: crate::bind_joypad::aiming_axis(j),
    }
}

/// `zombie.ui.RadialMenu`.
pub struct RadialMenu {
    pub outer_radius: i32,
    pub inner_radius: i32,
    slices: RefCell<Vec<Slice>>,
    pub highlight: Cell<i32>,
    joypad: Cell<i32>,
    transition: UITransition,
    select: UITransition,
    deselect: UITransition,
    select_index: Cell<i32>,
    deselect_index: Cell<i32>,
}

fn is_null_or_whitespace(s: &str) -> bool {
    s.chars().all(|c| c.is_whitespace())
}

impl RadialMenu {
    /// `new RadialMenu(x, y, innerRadius, outerRadius)`.
    pub fn create(x: i32, y: i32, inner: i32, outer: i32) -> Rc<UIElement> {
        let e = UIElement::new_ext(
            None,
            Box::new(RadialMenu {
                outer_radius: outer,
                inner_radius: inner,
                slices: RefCell::default(),
                highlight: Cell::new(-1),
                joypad: Cell::new(-1),
                transition: UITransition::new(),
                select: UITransition::new(),
                deselect: UITransition::new(),
                select_index: Cell::new(-1),
                deselect_index: Cell::new(-1),
            }),
        );
        e.set_x(x as f64);
        e.set_y(y as f64);
        e.set_width((outer * 2) as f64);
        e.set_height((outer * 2) as f64);
        e
    }

    /// `clear()`.
    pub fn clear(&self) {
        self.slices.borrow_mut().clear();
        self.transition.reset();
        self.transition.init(66.666664, false);
        self.select_index.set(-1);
        self.deselect_index.set(-1);
    }

    /// `addSlice(text, texture)`.
    pub fn add_slice(&self, text: Option<String>, texture: Option<Texture>) {
        self.slices.borrow_mut().push(Slice { text, texture });
    }

    fn slice(&self, i: i32) -> Option<Slice> {
        usize::try_from(i)
            .ok()
            .and_then(|i| self.slices.borrow().get(i).cloned())
    }

    pub fn set_slice_text(&self, i: i32, text: Option<String>) {
        if let Ok(i) = usize::try_from(i) {
            if let Some(s) = self.slices.borrow_mut().get_mut(i) {
                s.text = text;
            }
        }
    }

    pub fn set_slice_texture(&self, i: i32, texture: Option<Texture>) {
        if let Ok(i) = usize::try_from(i) {
            if let Some(s) = self.slices.borrow_mut().get_mut(i) {
                s.texture = texture;
            }
        }
    }

    pub fn set_joypad(&self, j: i32) {
        self.joypad.set(j);
    }

    fn count(&self) -> usize {
        self.slices.borrow().len()
    }

    fn degrees_per_slice(&self) -> f32 {
        360.0 / self.count().max(2) as f32
    }

    fn start_angle(&self) -> f32 {
        90.0 - self.degrees_per_slice() / 2.0
    }

    fn slice_at(&self, degrees: f64) -> i32 {
        let (start, per) = (self.start_angle() as f64, self.degrees_per_slice() as f64);
        if degrees < start {
            ((degrees + 360.0 - start) / per) as i32
        } else {
            ((degrees - start) / per) as i32
        }
    }

    /// `getSliceIndexFromMouse(mx, my)` (element coordinates).
    pub fn slice_index_from_mouse(&self, el: &UIElement, mx: i32, my: i32) -> i32 {
        let cx = 0.0 + el.get_width() as f32 / 2.0;
        let cy = 0.0 + el.get_height() as f32 / 2.0;
        let dist =
            (((mx as f32 - cx) as f64).powf(2.0) + ((my as f32 - cy) as f64).powf(2.0)).sqrt();
        if dist > self.outer_radius as f64 || dist < self.inner_radius as f64 {
            return -1;
        }
        let radians =
            ((my as f32 - cy) as f64).atan2((mx as f32 - cx) as f64) + std::f64::consts::PI;
        self.slice_at(radians.to_degrees())
    }

    /// `getSliceIndexFromJoypad(joypad)`: the aiming stick's direction.
    pub fn slice_index_from_joypad(&self, joypad: i32) -> i32 {
        let v = joypad_view(joypad);
        if !v.aiming_applied {
            return -1;
        }
        let radians = ((-v.aiming.1) as f64).atan2((-v.aiming.0) as f64);
        self.slice_at(radians.to_degrees())
    }

    fn drop_disconnected_joypad(&self) {
        if self.joypad.get() != -1 && !joypad_view(self.joypad.get()).connected {
            self.joypad.set(-1);
        }
    }

    /// `drawTextWithBackground`: a dark box behind the characters
    /// `start..=end`, centred on `x`.
    fn draw_text_with_background(&self, text: &str, x: f32, y: f32, start: i32, end: i32) {
        let font = Some(UIFont::Medium);
        let w = with_ui(|m| m.text.clone())
            .map_or(0, |t| t.width_range(font, text, start, end, false)) as f32;
        let fh = pz_ui::draw::font_height(font);
        let scale = self.transition.fraction();
        let a1 = 0.45 + 0.45 * scale;
        with_ui(|m| {
            m.sprite.renderi(
                None,
                (x - w / 2.0) as i32 - 2,
                y as i32,
                w as i32 + 4,
                fh,
                Rgba(0.1, 0.1, 0.1, a1),
            );
        });
        let a = 1.0 / 2.0 + 1.0 / 2.0 * scale;
        let sub: String = if end < start {
            String::new()
        } else {
            text.chars()
                .skip(start as usize)
                .take((end - start + 1) as usize)
                .collect()
        };
        // `AngelCodeFont.drawString(x, y, text, r, g, b, a, start, end)`.
        pz_ui::draw::draw_string(
            font,
            (x - w / 2.0) as f64,
            y as f64,
            1.0,
            &sub,
            Rgba(1.0, 1.0, 1.0, a),
            TextAlign::Left,
            true,
        );
    }

    /// `formatTextInsideCircle(text)`: each line centred in the ring.
    fn format_text_inside_circle(&self, el: &UIElement, text: &str) {
        let fh = pz_ui::draw::font_height(Some(UIFont::Medium));
        let chars: Vec<char> = text.chars().collect();
        let n_lines = 1 + chars.iter().filter(|&&c| c == '\n').count() as i32;
        let x = el.get_absolute_x() as i32 + el.get_width() as i32 / 2;
        if n_lines > 1 {
            let mut y = el.get_absolute_y() as i32 + el.get_height() as i32 / 2 - n_lines * fh / 2;
            let mut offset = 0i32;
            for (i, &c) in chars.iter().enumerate() {
                if c != '\n' {
                    continue;
                }
                self.draw_text_with_background(text, x as f32, y as f32, offset, i as i32 - 1);
                offset = i as i32 + 1;
                y += fh;
            }
            if (offset as usize) < chars.len() {
                self.draw_text_with_background(
                    text,
                    x as f32,
                    y as f32,
                    offset,
                    chars.len() as i32 - 1,
                );
            }
        } else {
            let y = el.get_absolute_y() as i32 + el.get_height() as i32 / 2 - fh / 2;
            self.draw_text_with_background(text, x as f32, y as f32, 0, chars.len() as i32 - 1);
        }
    }
}

impl UIElementExt for RadialMenu {
    fn class_name(&self) -> &'static str {
        RADIAL_MENU
    }
    fn super_classes(&self) -> &'static [&'static str] {
        SUPERS
    }
    fn as_any(&self) -> &dyn Any {
        self
    }

    /// `update()` (no `super.update()`): forget a disconnected joypad.
    fn update(&self, _el: &UIElement, _t: &mut Thread) {
        self.drop_disconnected_joypad();
    }

    /// `render()` (no `super.render()`): the dark disc, each slice's
    /// segments (highlighted, fading out, or faint), the icons, the
    /// highlighted slice's text. The polygons use the element's own
    /// `x`/`y`, not its absolute position, as in the original.
    fn render(&self, el: &UIElement, _t: &mut Thread) {
        if !el.is_visible() {
            return;
        }
        self.drop_disconnected_joypad();
        self.transition.set_ignore_update_time(true);
        self.transition.set_fade_in(true);
        self.transition.update();
        if self.count() == 0 {
            return;
        }
        let scale = self.transition.fraction();
        let inner = self.inner_radius as f32 * 0.85 + self.inner_radius as f32 * scale * 0.15;
        let outer = self.outer_radius as f32 * 0.85 + self.outer_radius as f32 * scale * 0.15;
        let (ex, ey) = (el.get_x(), el.get_y());
        let (w, h) = (el.get_width() as f32, el.get_height() as f32);
        let cx = ex + (w / 2.0) as f64;
        let cy = ey + (h / 2.0) as f64;
        let poly = |p: [[f64; 2]; 4], c: Rgba| {
            with_ui(|m| {
                m.sprite
                    .render4(None, p.map(|q| [q[0] as f32, q[1] as f32]), c)
            });
        };
        for i in 0..48 {
            let t2 = ((i as f32 * 7.5) as f64).to_radians();
            let t3 = (((i + 1) as f32 * 7.5) as f64).to_radians();
            let x2 = cx + (outer * t2.cos() as f32) as f64;
            let y2 = cy + (outer * t2.sin() as f32) as f64;
            let x3 = cx + (outer * t3.cos() as f32) as f64;
            let mut y3 = cy + (outer * t3.sin() as f32) as f64;
            if i == 47 {
                y3 = cy;
            }
            poly(
                [[cx, cy], [x2, y2], [x3, y3], [cx, cy]],
                Rgba(0.1, 0.1, 0.1, 0.45 + 0.45 * scale),
            );
        }
        let n = self.count();
        let per = self.degrees_per_slice();
        let pad = if n == 1 { 0.0 } else { 1.5f32 };
        let mut highlight = self.highlight.get();
        if highlight == -1 {
            highlight = if self.joypad.get() != -1 {
                self.slice_index_from_joypad(self.joypad.get())
            } else {
                let (mx, my) = with_ui(|m| (m.mouse.x, m.mouse.y));
                self.slice_index_from_mouse(
                    el,
                    mx - el.get_absolute_x() as i32,
                    my - el.get_absolute_y() as i32,
                )
            };
        }
        let slice = self.slice(highlight);
        if slice.as_ref().is_some_and(Slice::is_empty) {
            highlight = -1;
        }
        if highlight != self.select_index.get() {
            self.select.reset();
            self.select.set_ignore_update_time(true);
            if self.select_index.get() != -1 {
                self.deselect_index.set(self.select_index.get());
                self.deselect.reset();
                self.deselect.set_fade_in(false);
                self.deselect.init(66.666664, true);
            }
            self.select_index.set(highlight);
        }
        self.select.update();
        self.deselect.update();
        let start = self.start_angle() - 180.0;
        for i in 0..n {
            let sub = 6.max(48 / n.max(2)) as i32;
            for j in 0..sub {
                let base = start + i as f32 * per;
                let t0 = ((base + j as f32 * per / sub as f32 + if j == 0 { pad } else { 0.0 })
                    as f64)
                    .to_radians();
                let t1 = ((base + (j + 1) as f32 * per / sub as f32
                    - if j == sub - 1 { pad } else { 0.0 }) as f64)
                    .to_radians();
                let t2 = ((base
                    + j as f32 * per / sub as f32
                    + if j == 0 { pad / 2.0 } else { 0.0 }) as f64)
                    .to_radians();
                let t3 = ((base + (j + 1) as f32 * per / sub as f32) as f64
                    - if j == sub - 1 { pad as f64 / 1.5 } else { 0.0 })
                .to_radians();
                let at = |r: f32, t: f64| {
                    [
                        cx + (r * t.cos() as f32) as f64,
                        cy + (r * t.sin() as f32) as f64,
                    ]
                };
                let (p0, p1, p2, p3) = (at(inner, t0), at(inner, t1), at(outer, t2), at(outer, t3));
                let a = if i as i32 == highlight {
                    0.25 + 0.25 * self.select.fraction()
                } else if i as i32 == self.deselect_index.get() {
                    0.025 + 0.475 * self.deselect.fraction()
                } else {
                    0.025
                };
                poly([p0, p2, p3, p1], Rgba(1.0, 1.0, 1.0, a));
            }
            let Some(tex) = self.slices.borrow()[i].texture.clone() else {
                continue;
            };
            let theta = ((start + i as f32 * per + per / 2.0) as f64).to_radians();
            let r = inner + (outer - inner) / 2.0;
            let tcx = 0.0 + w / 2.0 + r * theta.cos() as f32;
            let tcy = 0.0 + h / 2.0 + r * theta.sin() as f32;
            if tex.width > 64 {
                el.draw_texture_scaled_aspect(
                    Some(&tex),
                    tcx as f64 - 32.0 - tex.offset_x as f64,
                    tcy as f64 - 32.0 - tex.offset_y as f64,
                    64.0,
                    64.0,
                    1.0,
                    1.0,
                    1.0,
                    scale as f64,
                );
                continue;
            }
            let x = tcx - (tex.width as i32 / 2) as f32 - tex.offset_x;
            let y = tcy - (tex.height as i32 / 2) as f32 - tex.offset_y;
            el.draw_texture(Some(&tex), x as f64, y as f64, scale as f64);
        }
        if let Some(text) = slice.and_then(|s| s.text) {
            if !is_null_or_whitespace(&text) {
                self.format_text_inside_circle(el, &text);
            }
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

    /// The oracle's monospace font: 8 px a character, Medium lines 20 px.
    struct Mono;
    impl pz_ui::TextRenderer for Mono {
        fn measure_x(&self, _: Option<UIFont>, s: &str) -> i32 {
            8 * s.chars().count() as i32
        }
        fn measure_y(&self, f: Option<UIFont>, _: &str) -> i32 {
            self.font_height(f)
        }
        fn font_height(&self, f: Option<UIFont>) -> i32 {
            match f {
                Some(UIFont::Large) => 24,
                Some(UIFont::Medium) => 20,
                _ => 16,
            }
        }
        #[allow(clippy::too_many_arguments)]
        fn draw(
            &self,
            out: &mut pz_ui::SpriteRenderer,
            font: Option<UIFont>,
            x: f64,
            y: f64,
            zoom: f32,
            text: &str,
            color: Rgba,
            align: TextAlign,
            trimmed: bool,
        ) {
            out.push(UiDrawCmd::Text {
                font,
                x: x as f32,
                y: y as f32,
                zoom,
                text: Rc::from(text),
                color,
                align,
                trimmed,
            });
        }
    }

    fn flush(got: &mut Vec<String>, label: &str) {
        got.push(format!("case {label}"));
        let v: Vec<String> = with_ui(|m| {
            m.sprite
                .cmds
                .iter()
                .filter_map(|c| match c {
                    UiDrawCmd::Quad {
                        texture,
                        x,
                        y,
                        w,
                        h,
                        color,
                        ..
                    } => Some(format!(
                        "draw {} {} {} {} {} {} {} {} {}",
                        texture
                            .as_ref()
                            .map_or("-".to_string(), |t| t.name.to_string()),
                        *x as i32,
                        *y as i32,
                        *w as i32,
                        *h as i32,
                        jf(color.0),
                        jf(color.1),
                        jf(color.2),
                        jf(color.3)
                    )),
                    UiDrawCmd::Quad4 { points, color, .. } => {
                        let mut s = "poly4 -".to_string();
                        for p in points {
                            s.push_str(&format!(" {} {}", jf(p[0]), jf(p[1])));
                        }
                        Some(format!(
                            "{s} {} {} {} {}",
                            jf(color.0),
                            jf(color.1),
                            jf(color.2),
                            jf(color.3)
                        ))
                    }
                    UiDrawCmd::Text {
                        x, y, text, color, ..
                    } => Some(format!(
                        "text Medium left {} {} [{}] {} {} {} {}",
                        jf(*x),
                        jf(*y),
                        text,
                        jf(color.0),
                        jf(color.1),
                        jf(color.2),
                        jf(color.3)
                    )),
                    _ => None,
                })
                .collect()
        });
        got.extend(v);
        with_ui(|m| m.sprite.clear());
    }

    /// Parity with the original `RadialMenu` (oracle `radialmenu.txt`):
    /// the fade-in, the disc and slice segments for 1, 2, 5 and 9 slices,
    /// the mouse highlight and the fade-out of the previous slice (with a
    /// long frame), small and scaled icons, one- and two-line text, an
    /// empty slice, edits, clear, the mouse index over a grid, hidden.
    #[test]
    fn matches_radial_menu_oracle() {
        let Some(text) = oracle("radialmenu.txt") else {
            eprintln!("PZ_HUD_ORACLE_DIR not set; skipping");
            return;
        };
        install_textures_ex(&oracle_textures_ex(&text));
        install(HudHost::default());
        let mut t = thread();
        pz_ui::UIManager::set_text_renderer(Some(Rc::new(Mono)));
        let now = Cell::new(1000i64);
        let tick = |ms: i64| {
            now.set(now.get() + ms);
            crate::bind_misc::set_ui_transition_clock(now.get(), ms);
        };
        let mouse = |x: i32, y: i32| {
            with_ui(|m| {
                m.mouse.x = x;
                m.mouse.y = y;
            })
        };
        let tex = |n: &str| pz_render::get_shared_texture(n);
        let (small, big) = (tex("media/ui/small.png"), tex("media/ui/big.png"));
        tick(0);
        let e = RadialMenu::create(200, 100, 40, 100);
        let m = e.ext::<RadialMenu>().unwrap();
        with_ui(|m| m.sprite.clear());
        let mut got = Vec::new();
        mouse(0, 0);
        e.render(&mut t);
        flush(&mut got, "empty");
        m.add_slice(Some("One".into()), small.clone());
        tick(16);
        e.render(&mut t);
        flush(&mut got, "one slice");
        m.clear();
        m.add_slice(Some("Left\nside".into()), small.clone());
        m.add_slice(None, big.clone());
        for f in 0..3 {
            tick(16);
            mouse(230, 200);
            e.render(&mut t);
            flush(&mut got, &format!("two {f}"));
        }
        m.clear();
        for i in 0..5 {
            let tx = if i == 3 {
                None
            } else if i % 2 == 0 {
                small.clone()
            } else {
                big.clone()
            };
            m.add_slice(
                if i == 3 {
                    None
                } else {
                    Some(format!("Slice {i}"))
                },
                tx,
            );
        }
        let mice = [
            (300, 120),
            (380, 200),
            (300, 290),
            (215, 200),
            (300, 200),
            (300, 40),
        ];
        for (f, (x, y)) in mice.iter().enumerate() {
            tick(if f == 2 { 200 } else { 16 });
            mouse(*x, *y);
            e.render(&mut t);
            flush(&mut got, &format!("five {f}"));
        }
        m.set_slice_text(1, Some("  ".into()));
        m.set_slice_texture(0, big.clone());
        m.set_slice_text(9, Some("x".into()));
        tick(16);
        mouse(380, 200);
        e.render(&mut t);
        flush(&mut got, "five edit");
        m.clear();
        for i in 0..9 {
            m.add_slice(Some(format!("S{i}")), None);
        }
        tick(16);
        mouse(330, 210);
        e.render(&mut t);
        flush(&mut got, "nine");
        let mut idx = "index".to_string();
        for y in (0..=200).step_by(25) {
            for x in (0..=200).step_by(25) {
                idx.push_str(&format!(" {}", m.slice_index_from_mouse(&e, x, y)));
            }
        }
        got.push(idx);
        e.set_visible(false);
        e.render(&mut t);
        flush(&mut got, "hidden");
        let want: Vec<String> = text
            .lines()
            .filter(|l| !l.starts_with("fake ") && !l.starts_with("tex "))
            .map(normalise_oracle)
            .collect();
        for (i, (g, w)) in got.iter().zip(&want).enumerate() {
            assert_eq!(g, w, "line {i}");
        }
        assert_eq!(got.len(), want.len());
    }

    /// The aiming stick picks the slice it points at; a disconnected
    /// joypad is dropped (back to the mouse).
    #[test]
    fn joypad_slices() {
        install_textures(&[]);
        install(HudHost::default());
        let mut t = thread();
        let e = RadialMenu::create(0, 0, 40, 100);
        let m = e.ext::<RadialMenu>().unwrap();
        for i in 0..4 {
            m.add_slice(Some(format!("S{i}")), None);
        }
        let set = |x: f32, y: f32| {
            TEST_JOYPAD.with(|j| {
                j.set(Some(JoypadView {
                    connected: true,
                    aiming_applied: true,
                    aiming: (x, y),
                }))
            })
        };
        // Slice 0 is at the top, then clockwise.
        set(0.0, -1.0);
        assert_eq!(m.slice_index_from_joypad(0), 0);
        set(1.0, 0.0);
        assert_eq!(m.slice_index_from_joypad(0), 1);
        set(0.0, 1.0);
        assert_eq!(m.slice_index_from_joypad(0), 2);
        set(-1.0, 0.0);
        assert_eq!(m.slice_index_from_joypad(0), 3);
        TEST_JOYPAD.with(|j| {
            j.set(Some(JoypadView {
                connected: true,
                aiming_applied: false,
                aiming: (1.0, 0.0),
            }))
        });
        assert_eq!(m.slice_index_from_joypad(0), -1);
        m.set_joypad(2);
        TEST_JOYPAD.with(|j| j.set(Some(JoypadView::default())));
        e.update(&mut t);
        assert_eq!(m.joypad.get(), -1);
        TEST_JOYPAD.with(|j| j.set(None));
    }

    /// `ISRadialMenu` drives it from Lua.
    #[test]
    fn lua_methods() {
        install_textures(&[]);
        install(HudHost::default());
        let mut t = thread();
        run(
            &mut t,
            "m = RadialMenu.new(10, 20, 30, 50); m:addSlice('a', nil); m:addSlice(nil, nil)
             m:setSliceText(1, 'b'); m:setJoypad(-1); w = m:getWidth()
             i = m:getSliceIndexFromMouse(50, 10); m:clear()",
        );
        assert_eq!(eval(&mut t, "w").as_num(), Some(100.0));
        assert_eq!(eval(&mut t, "i").as_num(), Some(0.0));
    }
}
