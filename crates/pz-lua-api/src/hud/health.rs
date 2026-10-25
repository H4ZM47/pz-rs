//! `zombie.ui.NewHealthPanel` (the body diagram of the health tab) and
//! `zombie.ui.UI_BodyPart` (one part's wound overlays).

use super::host::{with_host, BodyPartView};
use super::new_window::{th, tw, WindowCore};
use pz_kahlua::{Thread, Value};
use pz_render::Texture;
use pz_ui::{UIElement, UIElementExt};
use std::any::Any;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

pub const NEW_HEALTH_PANEL: &str = "zombie.ui.NewHealthPanel";
pub const UI_BODY_PART: &str = "zombie.ui.UI_BodyPart";
const PANEL_SUPERS: &[&str] = &[super::new_window::NEW_WINDOW, "zombie.ui.UIElement"];
const PART_SUPERS: &[&str] = &["zombie.ui.UIElement"];

/// The parts in the panel's child order: `BodyPartType.ToIndex` and the
/// overlay file suffix.
const PARTS: [(usize, &str); 17] = [
    (0, "hand_left.png"),
    (1, "hand_right.png"),
    (2, "lowerarm_left.png"),
    (3, "lowerarm_right.png"),
    (4, "upperarm_left.png"),
    (5, "upperarm_right.png"),
    (6, "chest.png"),
    (7, "abdomen.png"),
    (8, "head.png"),
    (9, "neck.png"),
    (10, "groin.png"),
    (11, "upperleg_left.png"),
    (12, "upperleg_right.png"),
    (13, "lowerleg_left.png"),
    (14, "lowerleg_right.png"),
    (15, "foot_left.png"),
    (16, "foot_right.png"),
];

fn tex(n: &str) -> Option<Texture> {
    pz_render::get_shared_texture(n)
}

/// `zombie.ui.UI_BodyPart`.
pub struct UIBodyPart {
    pub alpha: f32,
    /// `BodyPartType.ToIndex(bodyPartType)`.
    pub part: usize,
    pub is_flipped: bool,
    chr: RefCell<Value>,
    mouse_over: Cell<bool>,
    scratch: Option<Texture>,
    bandage: Option<Texture>,
    dirty_bandage: Option<Texture>,
    infection: Option<Texture>,
    deep_wound: Option<Texture>,
    stitch: Option<Texture>,
    bite: Option<Texture>,
    glass: Option<Texture>,
    bone: Option<Texture>,
    splint: Option<Texture>,
    burn: Option<Texture>,
    bullet: Option<Texture>,
}

impl UIBodyPart {
    /// `new UI_BodyPart(type, x, y, part, character, renderFlipped)`.
    pub fn create(
        part: usize,
        x: i32,
        y: i32,
        file: &str,
        chr: Value,
        flipped: bool,
    ) -> Rc<UIElement> {
        let sex = if with_host(|h| h.body_damage.is_female(&chr)) {
            "female"
        } else {
            "male"
        };
        let t = |kind: &str| tex(&format!("media/ui/BodyDamage/{sex}_{kind}_{file}"));
        let p = UIBodyPart {
            alpha: 1.0,
            part,
            is_flipped: flipped,
            chr: RefCell::new(chr),
            mouse_over: Cell::new(false),
            scratch: t("scratch"),
            bandage: t("bandage"),
            dirty_bandage: t("bandagedirty"),
            infection: t("infection"),
            bite: t("bite"),
            deep_wound: t("deepwound"),
            stitch: t("stitches"),
            glass: t("glass"),
            bone: t("bones"),
            splint: t("splint"),
            burn: t("burn"),
            bullet: t("bullet"),
        };
        let (w, h) = (tw(&p.scratch), th(&p.scratch));
        let e = UIElement::new_ext(None, Box::new(p));
        {
            let mut s = e.st_mut();
            s.x = x as f64;
            s.y = y as f64;
            s.width = w as f32;
            s.height = h as f32;
        }
        e
    }
}

impl UIElementExt for UIBodyPart {
    fn class_name(&self) -> &'static str {
        UI_BODY_PART
    }
    fn super_classes(&self) -> &'static [&'static str] {
        PART_SUPERS
    }
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn on_mouse_move_outside(&self, _el: &UIElement, _t: &mut Thread, _dx: f64, _dy: f64) {
        self.mouse_over.set(false);
    }

    /// `render()`: one wound overlay (infection, bandage, scratch, cut,
    /// bite, deep wound, stitches, in that priority), a fracture or splint,
    /// glass, bullet and burn, then the children.
    fn render(&self, el: &UIElement, t: &mut Thread) {
        let p: BodyPartView = {
            let chr = self.chr.borrow();
            with_host(|h| h.body_damage.body_part(&chr, self.part))
        };
        let d = |tx: &Option<Texture>, a: f64| el.draw_texture(tx.as_ref(), 0.0, 0.0, a);
        if self.infection.is_some() && !p.bandaged && p.wound_infection_level > 0.0 {
            d(&self.infection, (p.wound_infection_level / 10.0) as f64);
        }
        if self.bandage.is_some() && p.bandaged && p.bandage_life > 0.0 {
            d(&self.bandage, 1.0);
        } else if self.dirty_bandage.is_some() && p.bandaged && p.bandage_life <= 0.0 {
            d(&self.dirty_bandage, 1.0);
        } else if self.scratch.is_some() && p.scratched {
            d(&self.scratch, (p.scratch_time / 20.0) as f64);
        } else if self.scratch.is_some() && p.cut_time > 0.0 {
            d(&self.scratch, (p.cut_time / 20.0) as f64);
        } else if self.bite.is_some() && !p.bandaged && p.bitten && p.bite_time >= 0.0 {
            d(&self.bite, 1.0);
        } else if self.deep_wound.is_some() && p.deep_wounded {
            d(&self.deep_wound, (p.deep_wound_time / 15.0) as f64);
        } else if self.stitch.is_some() && p.stitched {
            d(&self.stitch, 1.0);
        }
        if self.bone.is_some() && p.fracture_time > 0.0 && p.splint_factor == 0.0 {
            d(&self.bone, 1.0);
        } else if self.splint.is_some() && p.splint_factor > 0.0 {
            d(&self.splint, 1.0);
        }
        if self.glass.is_some() && p.have_glass && !p.bandaged {
            d(&self.glass, 1.0);
        }
        if self.bullet.is_some() && p.have_bullet && !p.bandaged {
            d(&self.bullet, 1.0);
        }
        if self.burn.is_some() && p.burn_time > 0.0 && !p.bandaged {
            d(&self.burn, (p.burn_time / 100.0) as f64);
        }
        el.render_base(t);
    }
}

/// `zombie.ui.NewHealthPanel`.
pub struct NewHealthPanel {
    pub core: WindowCore,
    parent_char: RefCell<Value>,
    pub body_outline: Option<Texture>,
    pub health_bar: Option<Texture>,
    pub health_bar_back: Option<Texture>,
    pub health_icon: Option<Texture>,
    parts: RefCell<Vec<Rc<UIElement>>>,
}

thread_local! {
    /// `NewHealthPanel.instance`.
    static INSTANCE: RefCell<Option<std::rc::Weak<UIElement>>> = const { RefCell::new(None) };
}

/// `NewHealthPanel.instance`.
pub fn instance() -> Option<Rc<UIElement>> {
    INSTANCE.with(|i| i.borrow().as_ref().and_then(|w| w.upgrade()))
}

impl NewHealthPanel {
    /// `new NewHealthPanel(x, y, character)`.
    pub fn create(x: i32, y: i32, chr: Value) -> Rc<UIElement> {
        let sex = if with_host(|h| h.body_damage.is_female(&chr)) {
            "female"
        } else {
            "male"
        };
        let p = NewHealthPanel {
            core: WindowCore::new(),
            parent_char: RefCell::new(chr.clone()),
            body_outline: tex(&format!("media/ui/BodyDamage/{sex}_base.png")),
            health_icon: tex("media/ui/Heart_On.png"),
            health_bar_back: tex("media/ui/BodyDamage/DamageBar_Vert.png"),
            health_bar: tex("media/ui/BodyDamage/DamageBar_Vert_Fill.png"),
            parts: RefCell::default(),
        };
        let e = UIElement::new_ext(None, Box::new(p));
        let me = e.ext::<NewHealthPanel>().unwrap();
        me.core.init(&e, x, y, 10, 10, true);
        me.core.resize_to_fit_y.set(false);
        e.set_visible(false);
        INSTANCE.with(|i| *i.borrow_mut() = Some(Rc::downgrade(&e)));
        let (ow, oh) = (tw(&me.body_outline) as i32, th(&me.body_outline) as i32);
        let (bx, by) = (ow - 123 + 2, oh - 302 + 2);
        {
            let mut s = e.st_mut();
            s.width = (ow + tw(&me.health_icon) as i32 + 10) as f32;
            s.height = (270 + th(&me.core.title_right) as i32 + 5) as f32;
        }
        let parts: Vec<Rc<UIElement>> = PARTS
            .iter()
            .map(|(i, f)| UIBodyPart::create(*i, bx, by, f, chr.clone(), false))
            .collect();
        for p in &parts {
            e.add_child(p);
        }
        *me.parts.borrow_mut() = parts;
        e
    }

    /// `SetCharacter(chr)`: the panel's character (the parts keep theirs,
    /// as in the original).
    pub fn set_character(&self, chr: Value) {
        *self.parent_char.borrow_mut() = chr;
    }

    /// `getDamageStatusString()`.
    pub fn damage_status_string(&self) -> String {
        let health = {
            let chr = self.parent_char.borrow();
            with_host(|h| h.body_damage.health(&chr))
        };
        let key = if health == 100.0 {
            "IGUI_health_ok"
        } else if health > 90.0 {
            "IGUI_health_Slight_damage"
        } else if health > 80.0 {
            "IGUI_health_Very_Minor_damage"
        } else if health > 70.0 {
            "IGUI_health_Minor_damage"
        } else if health > 60.0 {
            "IGUI_health_Moderate_damage"
        } else if health > 50.0 {
            "IGUI_health_Severe_damage"
        } else if health > 40.0 {
            "IGUI_health_Very_Severe_damage"
        } else if health > 20.0 {
            "IGUI_health_Crital_damage"
        } else if health > 10.0 {
            "IGUI_health_Highly_Crital_damage"
        } else if health > 0.0 {
            "IGUI_health_Terminal_damage"
        } else {
            "IGUI_health_Deceased"
        };
        crate::bind_text::translate(key)
    }
}

impl UIElementExt for NewHealthPanel {
    fn class_name(&self) -> &'static str {
        NEW_HEALTH_PANEL
    }
    fn super_classes(&self) -> &'static [&'static str] {
        PANEL_SUPERS
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn button_clicked(&self, el: &UIElement, name: Option<Rc<str>>) {
        self.core.button_clicked(el, name)
    }
    fn on_mouse_down(&self, el: &UIElement, t: &mut Thread, x: f64, y: f64) -> bool {
        self.core.on_mouse_down(el, t, x, y)
    }
    fn on_mouse_move(&self, el: &UIElement, t: &mut Thread, dx: f64, dy: f64) -> bool {
        self.core.on_mouse_move(el, t, dx, dy)
    }
    fn on_mouse_move_outside(&self, el: &UIElement, t: &mut Thread, dx: f64, dy: f64) {
        self.core.on_mouse_move_outside(el, t, dx, dy)
    }
    fn on_mouse_up(&self, el: &UIElement, t: &mut Thread, x: f64, y: f64) -> bool {
        self.core.on_mouse_up(el, t, x, y)
    }

    /// `render()` (no `super.render()`): the outline, each part, the
    /// health bar.
    fn render(&self, el: &UIElement, t: &mut Thread) {
        let (ow, oh) = (tw(&self.body_outline) as i32, th(&self.body_outline) as i32);
        if !el.is_visible() {
            return;
        }
        let alpha = self.core.alpha.get() as f64;
        el.draw_texture(
            self.body_outline.as_ref(),
            (ow - 123 + 2) as f64,
            (oh - 302 + 2) as f64,
            alpha,
        );
        let parts = self.parts.borrow().clone();
        for p in &parts {
            p.render(t);
        }
        let health = {
            let chr = self.parent_char.borrow();
            with_host(|h| h.body_damage.health(&chr))
        };
        el.draw_texture_scaled_col(
            None,
            (ow + 10) as f64,
            0.0,
            25.0,
            258.0,
            0.15,
            0.15,
            0.15,
            1.0,
        );
        let len = (100.0 - health) * 2.56;
        el.draw_texture(
            self.health_icon.as_ref(),
            (ow + 10 - 1) as f64,
            (oh - th(&self.health_icon) as i32 - 2) as f64,
            alpha,
        );
        el.draw_texture_scaled(
            self.health_bar_back.as_ref(),
            (ow + 10 + 1) as f64,
            1.0,
            23.0,
            256.0,
            alpha,
        );
        el.draw_texture_scaled(
            self.health_bar.as_ref(),
            (ow + 10 + 1) as f64,
            (1 + len as i32) as f64,
            23.0,
            (256 - len as i32) as f64,
            alpha,
        );
        if with_host(|h| h.debug.ui_render_outline()) {
            let (x, y) = (-el.get_x_scroll(), -el.get_y_scroll());
            let (w, h) = (el.get_width(), el.get_height());
            el.draw_texture_scaled_col(None, x, y, 1.0, h, 1.0, 1.0, 1.0, 0.5);
            el.draw_texture_scaled_col(None, x + 1.0, y, w - 2.0, 1.0, 1.0, 1.0, 1.0, 0.5);
            el.draw_texture_scaled_col(None, x + w - 1.0, y, 1.0, h, 1.0, 1.0, 1.0, 0.5);
            el.draw_texture_scaled_col(
                None,
                x + 1.0,
                y + h - 1.0,
                w - 2.0,
                1.0,
                1.0,
                1.0,
                1.0,
                0.5,
            );
        }
    }

    fn update(&self, el: &UIElement, t: &mut Thread) {
        if !el.is_visible() {
            return;
        }
        self.core.update(el, t);
    }
}

#[cfg(test)]
mod tests {
    use super::super::host::{install, HudBodyDamage, HudHost};
    use super::super::new_window::NewWindow;
    use super::super::test_util::*;
    use super::*;
    use std::collections::HashMap;

    /// Female, health, parts.
    type Body = (bool, f32, Vec<BodyPartView>);

    thread_local! {
        /// Per character (`Value::Num` id): female, health, parts.
        static BODIES: RefCell<HashMap<i64, Body>> = RefCell::default();
    }

    struct FakeBodies;
    fn id(chr: &Value) -> i64 {
        match chr {
            Value::Num(n) => *n as i64,
            _ => -1,
        }
    }
    impl HudBodyDamage for FakeBodies {
        fn is_female(&self, chr: &Value) -> bool {
            BODIES.with(|b| b.borrow().get(&id(chr)).is_some_and(|b| b.0))
        }
        fn health(&self, chr: &Value) -> f32 {
            BODIES.with(|b| b.borrow().get(&id(chr)).map_or(100.0, |b| b.1))
        }
        fn body_part(&self, chr: &Value, index: usize) -> BodyPartView {
            BODIES.with(|b| {
                b.borrow()
                    .get(&id(chr))
                    .and_then(|b| b.2.get(index).cloned())
                    .unwrap_or_default()
            })
        }
    }

    /// The oracle's `bp <label> <i> <16 values>` lines as parts.
    fn parts_of(text: &str, label: &str) -> Vec<BodyPartView> {
        text.lines()
            .filter_map(|l| l.strip_prefix(&format!("bp {label} ")))
            .map(|l| {
                let v: Vec<f32> = l.split(' ').skip(1).map(|s| s.parse().unwrap()).collect();
                let b = |i: usize| v[i] != 0.0;
                BodyPartView {
                    bandaged: b(0),
                    bandage_life: v[1],
                    wound_infection_level: v[2],
                    scratched: b(3),
                    scratch_time: v[4],
                    cut_time: v[5],
                    bitten: b(6),
                    bite_time: v[7],
                    deep_wounded: b(8),
                    deep_wound_time: v[9],
                    stitched: b(10),
                    fracture_time: v[11],
                    splint_factor: v[12],
                    have_glass: b(13),
                    have_bullet: b(14),
                    burn_time: v[15],
                }
            })
            .collect()
    }

    fn st(e: &UIElement, l: &str) -> String {
        format!(
            "{l} {} {} {} {} {}",
            jf(e.get_x() as f32),
            jf(e.get_y() as f32),
            jf(e.get_width() as f32),
            jf(e.get_height() as f32),
            e.is_visible()
        )
    }

    /// The oracle's monospace `TextManager` heights (Small 16).
    struct Mono;
    impl pz_ui::TextRenderer for Mono {
        fn measure_x(&self, _: Option<pz_text::UIFont>, s: &str) -> i32 {
            8 * s.chars().count() as i32
        }
        fn measure_y(&self, f: Option<pz_text::UIFont>, _: &str) -> i32 {
            self.font_height(f)
        }
        fn font_height(&self, f: Option<pz_text::UIFont>) -> i32 {
            match f {
                Some(pz_text::UIFont::Large) => 24,
                Some(pz_text::UIFont::Medium) => 20,
                _ => 16,
            }
        }
        #[allow(clippy::too_many_arguments)]
        fn draw(
            &self,
            _: &mut pz_ui::SpriteRenderer,
            _: Option<pz_text::UIFont>,
            _: f64,
            _: f64,
            _: f32,
            _: &str,
            _: pz_render::Rgba,
            _: pz_ui::TextAlign,
            _: bool,
        ) {
        }
    }

    fn flush(got: &mut Vec<String>, label: &str) {
        got.push(format!("case {label}"));
        got.extend(draw_lines());
        pz_ui::with_ui(|m| m.sprite.clear());
    }

    /// Parity with the original `NewWindow`, `NewHealthPanel` and
    /// `UI_BodyPart` (oracle `health.txt`): the window frame, title-bar
    /// drag, `setMovable(false)`, a press below the title bar, `Nest` with
    /// `resizeToFitY` (including the original's stuck index on the close
    /// button) and without, close; the panel's size, children, every
    /// overlay branch, both sexes, the health bar at 73.5, 0.5 and 100.
    #[test]
    fn matches_health_oracle() {
        let Some(text) = oracle("health.txt") else {
            eprintln!("PZ_HUD_ORACLE_DIR not set; skipping");
            return;
        };
        install_textures_ex(&oracle_textures_ex(&text));
        install(HudHost {
            body_damage: Box::new(FakeBodies),
            ..HudHost::default()
        });
        let mut t = thread();
        pz_ui::UIManager::set_text_renderer(Some(Rc::new(Mono)));
        pz_ui::with_ui(|m| m.sprite.clear());
        let mut got = Vec::new();

        let w = NewWindow::create(10, 20, 100, 50, true);
        got.push(st(&w, "win"));
        w.render(&mut t);
        flush(&mut got, "win render");
        w.on_mouse_down(&mut t, 5.0, 10.0);
        w.on_mouse_move(&mut t, 7.0, 3.0);
        w.on_mouse_move_outside(&mut t, -2.0, 1.0);
        w.on_mouse_up(&mut t, 5.0, 10.0);
        got.push(st(&w, "win drag"));
        let core = &w.ext::<NewWindow>().unwrap().core;
        core.movable.set(false);
        w.on_mouse_down(&mut t, 5.0, 10.0);
        w.on_mouse_move(&mut t, 7.0, 3.0);
        w.on_mouse_up(&mut t, 5.0, 10.0);
        got.push(st(&w, "win fixed"));
        w.on_mouse_down(&mut t, 5.0, 40.0);
        core.movable.set(true);
        w.on_mouse_move(&mut t, 7.0, 3.0);
        w.on_mouse_up(&mut t, 5.0, 40.0);
        got.push(st(&w, "win body"));
        let c1 = UIElement::new(None);
        c1.set_height(33.0);
        let c2 = UIElement::new(None);
        c2.set_height(61.0);
        core.nest(&w, &c1, &mut t, 5, 6, 7, 8);
        core.nest(&w, &c2, &mut t, 1, 2, 3, 4);
        w.update(&mut t);
        got.push(st(&w, "win fit"));
        got.push(st(&c1, "c1"));
        got.push(st(&c2, "c2"));
        let w2 = NewWindow::create(0, 0, 300, 200, false);
        let core2 = &w2.ext::<NewWindow>().unwrap().core;
        core2.resize_to_fit_y.set(false);
        let c3 = UIElement::new(None);
        core2.nest(&w2, &c3, &mut t, 5, 6, 7, 8);
        w2.update(&mut t);
        got.push(st(&w2, "win2"));
        got.push(st(&c3, "c3"));
        w.button_clicked(Some("close".into()));
        got.push(st(&w, "win closed"));

        let male = parts_of(&text, "male");
        BODIES.with(|b| b.borrow_mut().insert(0, (false, 73.5, male)));
        got.push("health male 73.5".into());
        let h = NewHealthPanel::create(30, 40, Value::Num(0.0));
        assert!(Rc::ptr_eq(&instance().unwrap(), &h));
        got.push(st(&h, "panel"));
        for i in 0.. {
            let Some(c) = h.control(i) else { break };
            got.push(st(&c, "child"));
        }
        h.render(&mut t);
        flush(&mut got, "panel hidden");
        h.set_visible(true);
        h.render(&mut t);
        flush(&mut got, "panel male");
        h.update(&mut t);
        got.push(st(&h, "panel update"));
        let female = parts_of(&text, "female");
        BODIES.with(|b| b.borrow_mut().insert(1, (true, 0.5, female)));
        got.push("health female 0.5".into());
        let hf = NewHealthPanel::create(0, 0, Value::Num(1.0));
        got.push(st(&hf, "fpanel"));
        hf.set_visible(true);
        hf.render(&mut t);
        flush(&mut got, "panel female");
        BODIES.with(|b| b.borrow_mut().get_mut(&1).unwrap().1 = 100.0);
        got.push("health female 100.0".into());
        hf.render(&mut t);
        flush(&mut got, "panel full");

        let want: Vec<String> = text
            .lines()
            .filter(|l| !l.starts_with("tex ") && !l.starts_with("fake ") && !l.starts_with("bp "))
            .map(normalise_oracle)
            .collect();
        assert_eq!(got, want);
    }

    /// `getDamageStatusString` thresholds, the Lua constructor and
    /// methods, the class Lua sees.
    #[test]
    fn lua_and_status_strings() {
        install_textures(&[]);
        install(HudHost {
            body_damage: Box::new(FakeBodies),
            ..HudHost::default()
        });
        let mut t = thread();
        BODIES.with(|b| b.borrow_mut().insert(7, (false, 100.0, vec![])));
        t.globals().set_str("chr", Value::Num(7.0));
        run(
            &mut t,
            "p = NewHealthPanel.new(5, 6, chr); p:setMovable(false)",
        );
        let e = crate::bind_ui::element_of(&t.globals().get_str("p")).unwrap();
        assert_eq!(pz_kahlua::LuaObject::class_name(&*e), NEW_HEALTH_PANEL);
        assert!(!e.ext::<NewHealthPanel>().unwrap().core.movable.get());
        for (hp, key) in [
            (100.0, "IGUI_health_ok"),
            (95.0, "IGUI_health_Slight_damage"),
            (90.0, "IGUI_health_Very_Minor_damage"),
            (75.0, "IGUI_health_Minor_damage"),
            (61.0, "IGUI_health_Moderate_damage"),
            (55.0, "IGUI_health_Severe_damage"),
            (41.0, "IGUI_health_Very_Severe_damage"),
            (40.0, "IGUI_health_Crital_damage"),
            (15.0, "IGUI_health_Highly_Crital_damage"),
            (0.1, "IGUI_health_Terminal_damage"),
            (0.0, "IGUI_health_Deceased"),
        ] {
            BODIES.with(|b| b.borrow_mut().get_mut(&7).unwrap().1 = hp);
            let s = eval(&mut t, "p:getDamageStatusString()");
            assert_eq!(
                s.as_str(),
                Some(crate::bind_text::translate(key).as_str()),
                "{hp}"
            );
        }
        // SetCharacter switches the panel's character only.
        BODIES.with(|b| b.borrow_mut().insert(8, (true, 50.0, vec![])));
        t.globals().set_str("chr2", Value::Num(8.0));
        run(&mut t, "p:SetCharacter(chr2)");
        assert_eq!(
            eval(&mut t, "p:getDamageStatusString()").as_str(),
            Some(crate::bind_text::translate("IGUI_health_Very_Severe_damage").as_str())
        );
        // The close button hides it.
        e.set_visible(true);
        e.button_clicked(Some("close".into()));
        assert!(!e.is_visible());
    }

    /// `uiRenderOutline` draws the four edges.
    #[test]
    fn debug_outline() {
        struct Outline;
        impl super::super::host::HudDebug for Outline {
            fn ui_render_outline(&self) -> bool {
                true
            }
        }
        install_textures(&[]);
        install(HudHost {
            debug: Box::new(Outline),
            ..HudHost::default()
        });
        let mut t = thread();
        let h = NewHealthPanel::create(0, 0, Value::Num(99.0));
        h.set_visible(true);
        pz_ui::with_ui(|m| m.sprite.clear());
        h.render(&mut t);
        let l = draw_lines();
        let (w, hh) = (h.get_width() as i32, h.get_height() as i32);
        assert_eq!(
            l[l.len() - 4..],
            [
                format!("draw - 0 0 1 {hh} 1.0 1.0 1.0 0.5"),
                format!("draw - 1 0 {} 1 1.0 1.0 1.0 0.5", w - 2),
                format!("draw - {} 0 1 {hh} 1.0 1.0 1.0 0.5", w - 1),
                format!("draw - 1 {} {} 1 1.0 1.0 1.0 0.5", hh - 1, w - 2),
            ]
        );
    }
}
