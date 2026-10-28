//! `zombie.ui.SpeedControls` (the game speed buttons) and its buttons,
//! `zombie.ui.HUDButton` / `SpeedControls.SCButton`.

use super::host::with_host;
use pz_render::Texture;
use pz_ui::{UIElement, UIElementExt};
use std::any::Any;
use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};

pub const SPEED_CONTROLS: &str = "zombie.ui.SpeedControls";
pub const HUD_BUTTON: &str = "zombie.ui.HUDButton";
const SC_SUPERS: &[&str] = &["zombie.ui.UIElement"];
const BUTTON_SUPERS: &[&str] = &["zombie.ui.UIElement"];
const SCBUTTON_SUPERS: &[&str] = &[HUD_BUTTON, "zombie.ui.UIElement"];

pub const PAUSE_SPEED: i32 = 0;
pub const PLAY_SPEED: i32 = 1;
pub const FAST_FORWARD_SPEED: i32 = 2;
pub const FASTER_FORWARD_SPEED: i32 = 3;
pub const WAIT_SPEED: i32 = 4;
const RESUME_SPEED: i32 = 5;

// ---- HUDButton --------------------------------------------------------------

/// `zombie.ui.HUDButton` (`sc`: `SpeedControls.SCButton`).
pub struct HUDButton {
    pub clicked: Cell<bool>,
    display: Weak<UIElement>,
    pub highlight: Option<Texture>,
    pub overicon: Option<Texture>,
    pub mouse_over: Cell<bool>,
    pub name: Rc<str>,
    pub texture: Option<Texture>,
    pub notclicked_alpha: f32,
    pub clickedalpha: f32,
    activate_sound_name: String,
    sc: bool,
}

fn tex_w(t: &Option<Texture>) -> f64 {
    t.as_ref().map_or(0.0, |t| t.width as f64)
}
fn tex_h(t: &Option<Texture>) -> f64 {
    t.as_ref().map_or(0.0, |t| t.height as f64)
}

impl HUDButton {
    /// `new HUDButton(name, x, y, texture, highlight, display)`.
    pub fn create(
        name: &str,
        x: f64,
        y: f64,
        texture: &str,
        highlight: &str,
        display: &UIElement,
    ) -> Rc<UIElement> {
        Self::make(name, x, y, texture, highlight, display, false)
    }

    /// `new SpeedControls.SCButton(name, x, y, texture, highlight, display)`:
    /// a `HUDButton` 6 pixels larger each way.
    pub fn create_sc(
        name: &str,
        x: f32,
        y: f32,
        texture: &str,
        highlight: &str,
        display: &UIElement,
    ) -> Rc<UIElement> {
        let e = Self::make(name, x as f64, y as f64, texture, highlight, display, true);
        {
            let mut s = e.st_mut();
            s.width += 6.0;
            s.height += 6.0;
        }
        e
    }

    fn make(
        name: &str,
        x: f64,
        y: f64,
        texture: &str,
        highlight: &str,
        display: &UIElement,
        sc: bool,
    ) -> Rc<UIElement> {
        let texture = pz_render::get_shared_texture(texture);
        let highlight = pz_render::get_shared_texture(highlight);
        let (w, h) = (tex_w(&texture), tex_h(&texture));
        let b = HUDButton {
            clicked: Cell::new(false),
            display: Rc::downgrade(&display.rc()),
            highlight,
            overicon: None,
            mouse_over: Cell::new(false),
            name: Rc::from(name),
            texture,
            notclicked_alpha: 0.85,
            clickedalpha: 1.0,
            activate_sound_name: "UIActivateButton".into(),
            sc,
        };
        let e = UIElement::new_ext(None, Box::new(b));
        {
            let mut s = e.st_mut();
            s.x = x;
            s.y = y;
            s.width = w as f32;
            s.height = h as f32;
        }
        e
    }

    fn display_shows_me(&self) -> bool {
        self.display
            .upgrade()
            .and_then(|d| d.get_clicked_value())
            .is_some_and(|v| *v == *self.name)
    }
}

impl UIElementExt for HUDButton {
    fn class_name(&self) -> &'static str {
        HUD_BUTTON
    }
    fn super_classes(&self) -> &'static [&'static str] {
        if self.sc {
            SCBUTTON_SUPERS
        } else {
            BUTTON_SUPERS
        }
    }
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn on_mouse_down(&self, _el: &UIElement, _t: &mut pz_kahlua::Thread, _x: f64, _y: f64) -> bool {
        self.clicked.set(true);
        true
    }
    fn on_mouse_move(
        &self,
        _el: &UIElement,
        _t: &mut pz_kahlua::Thread,
        _dx: f64,
        _dy: f64,
    ) -> bool {
        self.mouse_over.set(true);
        true
    }
    fn on_mouse_move_outside(
        &self,
        _el: &UIElement,
        _t: &mut pz_kahlua::Thread,
        _dx: f64,
        _dy: f64,
    ) {
        self.clicked.set(false);
        self.mouse_over.set(false);
    }
    fn on_mouse_up(&self, _el: &UIElement, _t: &mut pz_kahlua::Thread, _x: f64, _y: f64) -> bool {
        if self.clicked.get() {
            if !self.activate_sound_name.trim().is_empty() {
                with_host(|h| h.sound.play_ui_sound(&self.activate_sound_name));
            }
            // (No UIEventHandler buttons are made by the ported widgets.)
            if let Some(d) = self.display.upgrade() {
                d.button_clicked(Some(self.name.clone()));
            }
        }
        self.clicked.set(false);
        true
    }

    fn render(&self, el: &UIElement, t: &mut pz_kahlua::Thread) {
        let lit = self.mouse_over.get() || self.display_shows_me();
        if self.sc {
            // SCButton.render: no super.render().
            let mut dy = 3.0;
            if self.clicked.get() {
                dy += 1.0;
            }
            let (w, h) = (el.get_width(), el.get_height());
            el.draw_texture_scaled_col(
                None,
                0.0,
                if self.clicked.get() { 1.0 } else { 0.0 },
                w,
                h,
                0.0,
                0.0,
                0.0,
                0.75,
            );
            if lit {
                el.draw_texture_scaled(
                    self.highlight.as_ref(),
                    3.0,
                    dy,
                    tex_w(&self.highlight),
                    tex_h(&self.highlight),
                    self.clickedalpha as f64,
                );
            } else {
                el.draw_texture_scaled(
                    self.texture.as_ref(),
                    3.0,
                    dy,
                    tex_w(&self.texture),
                    tex_h(&self.texture),
                    self.notclicked_alpha as f64,
                );
            }
            if self.overicon.is_some() {
                el.draw_texture_scaled(
                    self.overicon.as_ref(),
                    3.0,
                    dy,
                    tex_w(&self.overicon),
                    tex_h(&self.overicon),
                    1.0,
                );
            }
            return;
        }
        let dy = if self.clicked.get() { 1.0 } else { 0.0 };
        if lit {
            el.draw_texture_scaled(
                self.highlight.as_ref(),
                0.0,
                dy,
                el.get_width(),
                el.get_height(),
                self.clickedalpha as f64,
            );
        } else {
            el.draw_texture_scaled(
                self.texture.as_ref(),
                0.0,
                dy,
                el.get_width(),
                el.get_height(),
                self.notclicked_alpha as f64,
            );
        }
        if self.overicon.is_some() {
            el.draw_texture_scaled(
                self.overicon.as_ref(),
                0.0,
                dy,
                tex_w(&self.overicon),
                tex_h(&self.overicon),
                1.0,
            );
        }
        el.render_base(t);
    }
}

// ---- SpeedControls ----------------------------------------------------------

struct Buttons {
    step_forward: Rc<UIElement>,
    pause: Rc<UIElement>,
    play: Rc<UIElement>,
    fast_forward: Rc<UIElement>,
    faster_forward: Rc<UIElement>,
    wait: Rc<UIElement>,
}

impl Buttons {
    fn all(&self) -> [&Rc<UIElement>; 6] {
        [
            &self.step_forward,
            &self.pause,
            &self.play,
            &self.fast_forward,
            &self.faster_forward,
            &self.wait,
        ]
    }
}

fn name_of(b: &UIElement) -> Rc<str> {
    b.ext::<HUDButton>()
        .map(|h| h.name.clone())
        .unwrap_or_else(|| Rc::from(""))
}

#[derive(Clone, Copy)]
struct SpeedState {
    current_speed: i32,
    speed_before_pause: i32,
    multi_before_pause: f32,
    do_frame_step: bool,
    frame_skipped: bool,
    mouse_over: bool,
}

/// `zombie.ui.SpeedControls`.
pub struct SpeedControls {
    st: Cell<SpeedState>,
    buttons: RefCell<Option<Buttons>>,
}

impl SpeedControls {
    /// `new SpeedControls()`. `debug` is `Core.debug` (the step button).
    pub fn create(debug: bool) -> Rc<UIElement> {
        let sc = SpeedControls {
            st: Cell::new(SpeedState {
                current_speed: 1,
                speed_before_pause: 1,
                multi_before_pause: 1.0,
                do_frame_step: false,
                frame_skipped: false,
                mouse_over: false,
            }),
            buttons: RefCell::new(None),
        };
        let e = UIElement::new_ext(None, Box::new(sc));
        {
            let mut s = e.st_mut();
            s.x = 0.0;
            s.y = 0.0;
        }
        let right = |b: &Rc<UIElement>| (b.get_x() + b.get_width() + 2.0) as f32;
        let d = "media/ui/speedControls/";
        let step_forward = HUDButton::create_sc(
            "StepForward",
            1.0,
            0.0,
            &format!("{d}StepForward_Off.png"),
            &format!("{d}StepForward_Off.png"),
            &e,
        );
        let pause = HUDButton::create_sc(
            "Pause",
            right(&step_forward),
            0.0,
            &format!("{d}Pause_Off.png"),
            &format!("{d}Pause_On.png"),
            &e,
        );
        let play = HUDButton::create_sc(
            "Play",
            right(&pause),
            0.0,
            &format!("{d}Play_Off.png"),
            &format!("{d}Play_On.png"),
            &e,
        );
        let fast_forward = HUDButton::create_sc(
            "Fast Forward x 1",
            right(&play),
            0.0,
            &format!("{d}FFwd1_Off.png"),
            &format!("{d}FFwd1_On.png"),
            &e,
        );
        let faster_forward = HUDButton::create_sc(
            "Fast Forward x 2",
            right(&fast_forward),
            0.0,
            &format!("{d}FFwd2_Off.png"),
            &format!("{d}FFwd2_On.png"),
            &e,
        );
        let wait = HUDButton::create_sc(
            "Wait",
            right(&faster_forward),
            0.0,
            &format!("{d}Wait_Off.png"),
            &format!("{d}Wait_On.png"),
            &e,
        );
        {
            let mut s = e.st_mut();
            s.width = (wait.get_x() as i32) as f32 + wait.get_width() as f32;
            s.height = wait.get_height() as f32;
        }
        if debug {
            e.add_child(&step_forward);
        }
        for b in [&pause, &play, &fast_forward, &faster_forward, &wait] {
            e.add_child(b);
        }
        let me = e.ext::<SpeedControls>().unwrap();
        *me.buttons.borrow_mut() = Some(Buttons {
            step_forward,
            pause,
            play,
            fast_forward,
            faster_forward,
            wait,
        });
        e
    }

    fn state(&self) -> SpeedState {
        self.st.get()
    }

    fn update_state(&self, f: impl FnOnce(&mut SpeedState)) {
        let mut s = self.st.get();
        f(&mut s);
        self.st.set(s);
    }

    fn names(&self) -> [Rc<str>; 6] {
        let b = self.buttons.borrow();
        let b = b.as_ref().expect("SpeedControls buttons");
        b.all().map(|e| name_of(e))
    }

    /// `ButtonClicked(name)`.
    pub fn button_clicked(&self, el: &UIElement, name: &str) {
        let [step, pause, play, ff, fff, wait] = self.names();
        let set_mult = |m: f32| with_host(|h| h.game_time.set_multiplier(m));
        set_mult(1.0);
        if name == &*step {
            if self.state().current_speed > 0 {
                self.set_current_game_speed(el, 0);
            } else {
                self.set_current_game_speed(el, 1);
                self.update_state(|s| s.do_frame_step = true);
            }
        }
        if name == &*pause {
            if self.state().current_speed > 0 {
                self.set_current_game_speed(el, 0);
            } else {
                self.set_current_game_speed(el, RESUME_SPEED);
            }
        }
        if name == &*play {
            self.set_current_game_speed(el, 1);
            set_mult(1.0);
        }
        if name == &*ff {
            self.set_current_game_speed(el, 2);
            set_mult(5.0);
        }
        if name == &*fff {
            self.set_current_game_speed(el, 3);
            set_mult(20.0);
        }
        if name == &*wait {
            self.set_current_game_speed(el, 4);
            set_mult(40.0);
        }
    }

    /// `getCurrentGameSpeed()`.
    pub fn get_current_game_speed(&self) -> i32 {
        if with_host(|h| h.network.is_client() || h.network.is_server()) {
            return 1;
        }
        self.state().current_speed
    }

    /// `SetCorrectIconStates()`: `super.ButtonClicked` with the button
    /// matching the speed (the clicked value drives the lit icon).
    pub fn set_correct_icon_states(&self, el: &UIElement) {
        let [_, pause, play, ff, fff, wait] = self.names();
        let speed = self.state().current_speed;
        if speed == 0 {
            el.button_clicked_base(Some(pause));
        }
        if speed == 1 {
            el.button_clicked_base(Some(play));
        }
        let tm = with_host(|h| h.game_time.get_true_multiplier());
        if tm == 5.0 {
            el.button_clicked_base(Some(ff));
        }
        if tm == 20.0 {
            el.button_clicked_base(Some(fff));
        }
        if tm == 40.0 {
            el.button_clicked_base(Some(wait));
        }
    }

    /// `Pause()`.
    pub fn pause(&self, el: &UIElement) {
        self.set_current_game_speed(el, 0);
    }

    /// `SetCurrentGameSpeed(newSpeed)`.
    pub fn set_current_game_speed(&self, el: &UIElement, mut new_speed: i32) {
        let cur = self.state().current_speed;
        with_host(|h| {
            if cur > 0 && new_speed == 0 {
                h.sound.pause_sound_and_music(true);
                h.sound.set_music_state("PauseMenu");
            } else if cur == 0 && new_speed > 0 {
                h.sound.set_music_state("InGame");
                h.sound.resume_sound_and_music();
            }
            h.game_time.set_multiplier(1.0);
        });
        if new_speed == 0 {
            let m = with_host(|h| h.game_time.get_multiplier());
            self.update_state(|s| {
                s.speed_before_pause = s.current_speed;
                s.multi_before_pause = m;
            });
        }
        if new_speed == RESUME_SPEED {
            let s = self.state();
            new_speed = s.speed_before_pause;
            with_host(|h| h.game_time.set_multiplier(s.multi_before_pause));
        }
        self.update_state(|s| s.current_speed = new_speed);
        self.set_correct_icon_states(el);
    }

    /// `stepForward()`.
    pub fn step_forward(&self, el: &UIElement) {
        if with_host(|h| h.network.is_client()) {
            // TODO(network): GameClient.SendCommandToServer("/setTimeSpeed 0").
            return;
        }
        if self.state().current_speed > 0 {
            self.set_current_game_speed(el, 0);
        } else {
            self.set_current_game_speed(el, 1);
            self.update_state(|s| s.do_frame_step = true);
        }
    }

    /// `isPaused()`.
    pub fn is_paused(&self) -> bool {
        self.state().current_speed == 0
    }

    /// `mouseOver`.
    pub fn is_mouse_over_controls(&self) -> bool {
        self.state().mouse_over
    }
}

impl UIElementExt for SpeedControls {
    fn class_name(&self) -> &'static str {
        SPEED_CONTROLS
    }
    fn super_classes(&self) -> &'static [&'static str] {
        SC_SUPERS
    }
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn button_clicked(&self, el: &UIElement, name: Option<Rc<str>>) {
        // `name.equals(...)` on null throws in the original.
        let Some(n) = name else { return };
        SpeedControls::button_clicked(self, el, &n);
    }

    fn on_mouse_move(&self, el: &UIElement, t: &mut pz_kahlua::Thread, dx: f64, dy: f64) -> bool {
        if !el.is_visible() {
            return false;
        }
        self.update_state(|s| s.mouse_over = true);
        el.on_mouse_move_base(t, dx, dy);
        self.set_correct_icon_states(el);
        true
    }

    fn on_mouse_move_outside(&self, el: &UIElement, t: &mut pz_kahlua::Thread, dx: f64, dy: f64) {
        el.on_mouse_move_outside_base(t, dx, dy);
        self.update_state(|s| s.mouse_over = false);
        self.set_correct_icon_states(el);
    }

    fn render(&self, el: &UIElement, t: &mut pz_kahlua::Thread) {
        el.render_base(t);
        let tutorial = crate::bind_core::shared_core().borrow().game_mode == "Tutorial";
        if tutorial {
            if let Some(b) = self.buttons.borrow().as_ref() {
                for e in b.all() {
                    e.set_visible(false);
                }
            }
        }
        self.set_correct_icon_states(el);
    }

    fn update(&self, el: &UIElement, t: &mut pz_kahlua::Thread) {
        el.update_base(t);
        self.set_correct_icon_states(el);
        if self.state().frame_skipped {
            self.set_current_game_speed(el, 0);
            self.update_state(|s| s.frame_skipped = false);
        }
        if self.state().do_frame_step {
            self.update_state(|s| {
                s.frame_skipped = true;
                s.do_frame_step = false;
            });
        }
    }
}
