//! `zombie.ui.MoodlesUI` (a player's moodle column) and
//! `zombie.ui.MoodleTextureSet`.

use super::host::with_host;
use pz_render::{Rgba, Texture};
use pz_ui::{with_ui, UIElement, UIElementExt};
use std::any::Any;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

pub const MOODLES_UI: &str = "zombie.ui.MoodlesUI";
const SUPERS: &[&str] = &["zombie.ui.UIElement"];

const MAX_MOUSE_OVER_SLOT: i32 = 1000;
const DEFAULT_MOODLE_DIST_Y: f32 = 10.0;
const OFFSCREEN_Y: f32 = 10000.0;
const OSCILLATOR_RATE: f32 = 0.8;
const OSCILLATOR_SCALAR: f32 = 15.6;
const TEXTURE_SIZES: [i32; 6] = [32, 48, 64, 80, 96, 128];
/// `Moodle.MoodleLevel.HighMoodleLevel.ordinal()`.
const HIGH_MOODLE_LEVEL: i32 = 3;
const GRAY: Rgba = Rgba(0.5, 0.5, 0.5, 1.0);

/// The base `MoodleType`s in registry order with their `MoodleTextureSet`
/// image.
pub const MOODLE_TYPES: [(&str, &str); 26] = [
    ("Endurance", "Status_DifficultyBreathing"),
    ("Tired", "Mood_Sleepy"),
    ("Hungry", "Status_Hunger"),
    ("Panic", "Mood_Panicked"),
    ("Sick", "Mood_Nauseous"),
    ("Bored", "Mood_Bored"),
    ("Unhappy", "Mood_Sad"),
    ("Bleeding", "Status_Bleeding"),
    ("Wet", "Status_Wet"),
    ("HasACold", "Mood_Ill"),
    ("Angry", "Mood_Angry"),
    ("Stress", "Mood_Stressed"),
    ("Thirst", "Status_Thirst"),
    ("Injured", "Status_InjuredMinor"),
    ("Pain", "Mood_Pained"),
    ("HeavyLoad", "Status_HeavyLoad"),
    ("Drunk", "Mood_Drunk"),
    ("Dead", "Mood_Dead"),
    ("Zombie", "Mood_Zombified"),
    ("Hyperthermia", "Status_TemperatureHot"),
    ("Hypothermia", "Status_TemperatureLow"),
    ("Windchill", "Status_Windchill"),
    ("CantSprint", "Status_MovementRestricted"),
    ("Uncomfortable", "Mood_Discomfort"),
    ("NoxiousSmell", "Mood_NoxiousSmell"),
    ("FoodEaten", "Status_Hunger"),
];

/// `zombie.ui.MoodleTextureSet`.
pub struct MoodleTextureSet {
    pub size: i32,
    pub border: Option<Texture>,
    pub background: Option<Texture>,
    textures: Vec<Option<Texture>>,
}

impl MoodleTextureSet {
    /// `new MoodleTextureSet(size)` (the border with `TEXTURE_FLAG 0x40`).
    pub fn new(size: i32) -> Self {
        let dir = format!("media/ui/Moodles/{size}");
        MoodleTextureSet {
            size,
            border: pz_render::get_shared_texture(&format!("{dir}/_Moodles_BGoutline.png")),
            background: pz_render::get_shared_texture(&format!("{dir}/_Moodles_BGsolid.png")),
            textures: MOODLE_TYPES
                .iter()
                .map(|(_, f)| pz_render::get_shared_texture(&format!("{dir}/{f}.png")))
                .collect(),
        }
    }

    /// `getTexture(type)`.
    pub fn texture(&self, moodle: &str) -> Option<&Texture> {
        let i = MOODLE_TYPES.iter().position(|(n, _)| *n == moodle)?;
        self.textures[i].as_ref()
    }
}

/// `MoodlesUI.MoodleUIData`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MoodleUIData {
    pub good_bad_neutral: i32,
    pub level: i32,
    pub oscillation_level: f32,
    pub slots_desired_pos: f32,
    pub slots_pos: f32,
    pub slots_pulse1: f32,
    pub slots_pulse2: f32,
}

impl Default for MoodleUIData {
    fn default() -> Self {
        MoodleUIData {
            good_bad_neutral: 0,
            level: 0,
            oscillation_level: 0.0,
            slots_desired_pos: OFFSCREEN_Y,
            slots_pos: OFFSCREEN_Y,
            slots_pulse1: 0.0,
            slots_pulse2: 0.0,
        }
    }
}

struct MoodlesState {
    /// `moodleUiState`, in iteration order. The original's is a `HashMap`
    /// keyed by `MoodleType` objects without a `hashCode`, so its order
    /// (the slot order on screen) depends on identity hashes; registry
    /// order stands in, and [`MoodlesUI::set_slot_order`] can set another.
    ui_state: Vec<(&'static str, MoodleUIData)>,
    alpha: f32,
    texture_sets: Vec<Rc<MoodleTextureSet>>,
    current: Rc<MoodleTextureSet>,
    moodle_dist_y: f32,
    mouse_over: bool,
    mouse_over_slot: i32,
    num_used_slots: i32,
    debug_key_delay: i32,
    oscillator_step: f32,
    /// `isoGameCharacter` (its player index).
    character: Option<usize>,
    alpha_increase: bool,
    background_colour: Rgba,
}

/// `zombie.ui.MoodlesUI`.
pub struct MoodlesUI {
    st: RefCell<MoodlesState>,
    pub client_w: Cell<f32>,
    pub client_h: Cell<f32>,
}

thread_local! {
    /// `MoodlesUI.instance` (the last one built).
    static INSTANCE: RefCell<Option<std::rc::Weak<UIElement>>> = const { RefCell::new(None) };
}

/// `MoodlesUI.getInstance()`.
pub fn instance() -> Option<Rc<UIElement>> {
    INSTANCE.with(|i| i.borrow().as_ref().and_then(|w| w.upgrade()))
}

fn texture_size_for_option() -> i32 {
    let core = crate::bind_core::shared_core();
    let c = core.borrow();
    let moodle = c.get_option_moodle_size() - 1;
    if (0..TEXTURE_SIZES.len() as i32).contains(&moodle) {
        return TEXTURE_SIZES[moodle as usize];
    }
    if moodle == 6 {
        let font = c.get_option_font_size_real() - 1;
        if (0..TEXTURE_SIZES.len() as i32).contains(&font) {
            return TEXTURE_SIZES[font as usize];
        }
    }
    32
}

fn texture_set_index_for_size(size: i32) -> usize {
    TEXTURE_SIZES.iter().position(|&s| s == size).unwrap_or(0)
}

// ---- Color's ABGR helpers -----------------------------------------------------

fn to_abgr(c: Rgba) -> u32 {
    let b = |v: f32| (v.clamp(0.0, 1.0) * 255.0) as i32 as u32 & 0xff;
    (b(c.3) << 24) | (b(c.2) << 16) | (b(c.1) << 8) | b(c.0)
}

fn channel(v: u32, shift: u32) -> f32 {
    0.003_921_569 * ((v >> shift) & 0xff) as f32
}

/// `Color.abgrToColor(Color.lerpABGR(a, b, t))`.
fn lerp_abgr(a: Rgba, b: Rgba, t: f32) -> Rgba {
    let (ca, cb) = (to_abgr(a), to_abgr(b));
    let lerp = |s: u32| channel(ca, s) * (1.0 - t) + channel(cb, s) * t;
    let v = to_abgr(Rgba(lerp(0), lerp(8), lerp(16), lerp(24)));
    Rgba(channel(v, 0), channel(v, 8), channel(v, 16), channel(v, 24))
}

impl MoodlesUI {
    /// `new MoodlesUI()`.
    pub fn create() -> Rc<UIElement> {
        let (sw, sh) = {
            let core = crate::bind_core::shared_core();
            let c = core.borrow();
            (c.get_screen_width(), c.get_screen_height())
        };
        let width = texture_size_for_option();
        let sets: Vec<Rc<MoodleTextureSet>> = TEXTURE_SIZES
            .iter()
            .map(|&s| Rc::new(MoodleTextureSet::new(s)))
            .collect();
        let current = sets[texture_set_index_for_size(width)].clone();
        let m = MoodlesUI {
            st: RefCell::new(MoodlesState {
                ui_state: MOODLE_TYPES
                    .iter()
                    .map(|(n, _)| (*n, MoodleUIData::default()))
                    .collect(),
                alpha: 1.0,
                texture_sets: sets,
                current,
                moodle_dist_y: 74.0,
                mouse_over: false,
                mouse_over_slot: 0,
                num_used_slots: 0,
                debug_key_delay: 0,
                oscillator_step: 0.0,
                character: None,
                alpha_increase: true,
                background_colour: GRAY,
            }),
            client_w: Cell::new(width as f32),
            client_h: Cell::new(sh as f32),
        };
        let e = UIElement::new_ext(None, Box::new(m));
        {
            let mut s = e.st_mut();
            s.x = (sw - 10) as f64;
            s.y = 120.0;
            s.width = width as f32;
            s.height = sh as f32;
        }
        INSTANCE.with(|i| *i.borrow_mut() = Some(Rc::downgrade(&e)));
        e
    }

    /// `wiggle(type)`.
    pub fn wiggle(&self, moodle: &str) {
        let mut s = self.st.borrow_mut();
        if let Some((_, d)) = s.ui_state.iter_mut().find(|(n, _)| *n == moodle) {
            d.oscillation_level = 1.0;
        }
    }

    /// `setCharacter(chr)` (the player index; `None` clears).
    pub fn set_character(&self, player: Option<usize>) {
        if self.st.borrow().character == player {
            return;
        }
        self.st.borrow_mut().character = player;
        if let Some(p) = player {
            with_host(|h| h.moodles.set_moodles_state_changed(p, true));
        }
    }

    /// The slot order (see `MoodlesState::ui_state`).
    pub fn set_slot_order(&self, order: &[&'static str]) {
        let mut s = self.st.borrow_mut();
        let mut v = Vec::with_capacity(order.len());
        for n in order {
            if let Some(i) = s.ui_state.iter().position(|(m, _)| m == n) {
                v.push(s.ui_state.remove(i));
            }
        }
        v.append(&mut s.ui_state);
        s.ui_state = v;
    }

    /// A moodle's UI state.
    pub fn data(&self, moodle: &str) -> Option<MoodleUIData> {
        self.st
            .borrow()
            .ui_state
            .iter()
            .find(|(n, _)| *n == moodle)
            .map(|(_, d)| *d)
    }

    fn is_currently_animating(s: &MoodlesState) -> bool {
        s.ui_state
            .iter()
            .any(|(_, d)| d.slots_pos != d.slots_desired_pos)
    }

    /// `update()`.
    fn update_moodles(&self, el: &UIElement, t: &mut pz_kahlua::Thread) {
        self.st.borrow_mut().moodle_dist_y = DEFAULT_MOODLE_DIST_Y + el.get_width() as f32;
        el.update_base(t);
        let Some(player) = self.st.borrow().character else {
            return;
        };
        let lock_fps = with_ui(|m| m.lock_fps);
        let refresh = with_host(|h| h.moodles.ui_refresh_needed(player));
        let mut s = self.st.borrow_mut();
        if !Self::is_currently_animating(&s) {
            if s.debug_key_delay > 0 {
                s.debug_key_delay -= 1;
            } else if crate::bind_input::is_key_down(57) {
                s.debug_key_delay = 10;
            }
        }
        let fps_fraction = lock_fps as f32 / 30.0;
        for (_, d) in s.ui_state.iter_mut() {
            d.oscillation_level -= d.oscillation_level * 0.040_000_02 / fps_fraction;
            if d.oscillation_level < 0.01 {
                d.oscillation_level = 0.0;
            }
        }
        if refresh {
            let mut slot = 0;
            let dist = s.moodle_dist_y;
            with_host(|h| {
                for (name, d) in s.ui_state.iter_mut() {
                    let level = h.moodles.moodle_level(player, name);
                    if *name == "FoodEaten" && level < HIGH_MOODLE_LEVEL {
                        d.slots_pos = OFFSCREEN_Y;
                        d.slots_desired_pos = OFFSCREEN_Y;
                        d.oscillation_level = 0.0;
                        continue;
                    }
                    if level > 0 {
                        let mut changed = false;
                        if d.level != level {
                            changed = true;
                            d.level = level;
                            d.oscillation_level = 1.0;
                        }
                        d.slots_desired_pos = dist * slot as f32;
                        if changed {
                            if d.slots_pos == OFFSCREEN_Y {
                                d.slots_pos = d.slots_desired_pos + 500.0;
                                d.oscillation_level = 0.0;
                            }
                            d.good_bad_neutral = h.moodles.good_bad_neutral(player, name);
                        } else {
                            d.oscillation_level = 0.0;
                        }
                        slot += 1;
                        continue;
                    }
                    d.slots_pos = OFFSCREEN_Y;
                    d.slots_desired_pos = OFFSCREEN_Y;
                    d.oscillation_level = 0.0;
                    d.level = 0;
                }
            });
            s.num_used_slots = slot;
        }
        for (_, d) in s.ui_state.iter_mut() {
            if (d.slots_pos - d.slots_desired_pos).abs() > 0.8 {
                d.slots_pos += (d.slots_desired_pos - d.slots_pos) * 0.15;
            } else {
                d.slots_pos = d.slots_desired_pos;
            }
        }
    }

    /// `render()`.
    fn render_moodles(&self, el: &UIElement, t: &mut pz_kahlua::Thread) {
        let width_required = texture_size_for_option();
        {
            let mut s = self.st.borrow_mut();
            if width_required != s.current.size {
                s.current = s.texture_sets[texture_set_index_for_size(width_required)].clone();
                el.st_mut().width = width_required as f32;
            }
        }
        let Some(player) = self.st.borrow().character else {
            return;
        };
        if self.st.borrow().moodle_dist_y != DEFAULT_MOODLE_DIST_Y + el.get_width() as f32 {
            with_host(|h| h.moodles.set_moodles_state_changed(player, true));
            self.update_moodles(el, t);
        }
        let millis = with_ui(|m| m.ui_render_interval_ms) as f64;
        let (ui_fps, blinking, good, bad) = {
            let core = crate::bind_core::shared_core();
            let mut c = core.borrow_mut();
            let ui_fps = if with_ui(|m| m.use_ui_fbo) {
                c.get_option_ui_render_fps()
            } else {
                with_ui(|m| m.lock_fps)
            };
            let g = c.get_good_highlited_color();
            let b = c.get_bad_highlited_color();
            (
                ui_fps,
                c.get_blinking_moodle(),
                Rgba(g.r, g.g, g.b, g.a),
                Rgba(b.r, b.g, b.b, b.a),
            )
        };
        let width = el.get_width() as f32;
        let mut rendered_slot = 0;
        let n = self.st.borrow().ui_state.len();
        let fps_fraction = (millis / 33.3f32 as f64) as f32;
        let oscillator = {
            let mut s = self.st.borrow_mut();
            s.oscillator_step += OSCILLATOR_RATE * fps_fraction * 0.5;
            (s.oscillator_step as f64).sin() as f32
        };
        for k in 0..n {
            let (name, d, set) = {
                let s = self.st.borrow();
                let (name, d) = s.ui_state[k];
                (name, d, s.current.clone())
            };
            if d.slots_pos == OFFSCREEN_Y {
                continue;
            }
            let wiggle = oscillator * OSCILLATOR_SCALAR * d.oscillation_level;
            let bg_colour = match d.good_bad_neutral {
                1 => lerp_abgr(GRAY, good, d.level as f32 / 4.0),
                2 => lerp_abgr(GRAY, bad, d.level as f32 / 4.0),
                _ => GRAY,
            };
            let alpha = {
                let mut s = self.st.borrow_mut();
                s.background_colour = bg_colour;
                if blinking.as_deref() == Some(&format!("base:{}", name.to_lowercase())) {
                    let step = 0.1 * (30.0 / ui_fps as f32);
                    if s.alpha_increase {
                        s.alpha += step;
                        if s.alpha > 1.0 {
                            s.alpha = 1.0;
                            s.alpha_increase = false;
                        }
                    } else {
                        s.alpha -= step;
                        if s.alpha < 0.0 {
                            s.alpha = 0.0;
                            s.alpha_increase = true;
                        }
                    }
                }
                if blinking.is_none() {
                    s.alpha = 1.0;
                }
                // isPercentageBackground is always false: no pulse.
                s.ui_state[k].1.slots_pulse2 = 0.0;
                s.alpha
            };
            // TODO(render): the background, border and icon textures'
            // GL_LINEAR_MIPMAP_NEAREST / GL_LINEAR filters.
            let (x, y) = ((wiggle as i32) as f64, (d.slots_pos as i32) as f64);
            el.draw_texture_col(set.background.as_ref(), x, y, bg_colour);
            el.draw_texture(set.border.as_ref(), x, y, alpha as f64);
            // `scale = width; offset = ceil((width - scale) / 2)`: always 0.
            let scale = width;
            let offset = ((width - scale) / 2.0).ceil() as f64;
            el.draw_texture(
                set.texture(name),
                ((wiggle as f64 + offset) as i32) as f64,
                ((d.slots_pos as f64 + offset) as i32) as f64,
                alpha as f64,
            );
            let (mouse_over, mouse_slot) = {
                let s = self.st.borrow();
                (s.mouse_over, s.mouse_over_slot)
            };
            if mouse_over && rendered_slot == mouse_slot {
                let (s1, s2) = with_host(|h| {
                    (
                        h.moodles.display_string(player, name),
                        h.moodles.description_string(player, name),
                    )
                });
                let w1 = pz_ui::draw::measure_string_x(None, &s1);
                let w2 = pz_ui::draw::measure_string_x(None, &s2);
                let w = w1.max(w2);
                let font_hgt = pz_ui::draw::font_height(None);
                let mut ty = d.slots_pos as i32 + 1;
                let h = (2 + font_hgt) * 2;
                if width > h as f32 {
                    ty += ((width - h as f32) / 2.0) as i32;
                }
                el.draw_texture_scaled_col(
                    None,
                    -10.0 - w as f64 - 6.0,
                    ty as f64 - 2.0,
                    w as f64 + 12.0,
                    h as f64,
                    0.0,
                    0.0,
                    0.0,
                    0.6,
                );
                el.draw_text_right(None, &s1, -10.0, ty as f64, 1.0, 1.0, 1.0, 1.0);
                el.draw_text_right(
                    None,
                    &s2,
                    -10.0,
                    (ty + font_hgt) as f64,
                    0.8f32 as f64,
                    0.8f32 as f64,
                    0.8f32 as f64,
                    1.0,
                );
            }
            rendered_slot += 1;
        }
        el.render_base(t);
    }
}

impl UIElementExt for MoodlesUI {
    fn class_name(&self) -> &'static str {
        MOODLES_UI
    }
    fn super_classes(&self) -> &'static [&'static str] {
        SUPERS
    }
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn on_mouse_move(&self, el: &UIElement, t: &mut pz_kahlua::Thread, dx: f64, dy: f64) -> bool {
        if !el.is_visible() {
            return false;
        }
        self.st.borrow_mut().mouse_over = true;
        el.on_mouse_move_base(t, dx, dy);
        let my = with_ui(|m| m.mouse.y) as f64;
        let mut s = self.st.borrow_mut();
        s.mouse_over_slot = ((my - el.get_y()) / s.moodle_dist_y as f64) as i32;
        if s.mouse_over_slot >= s.num_used_slots {
            s.mouse_over_slot = MAX_MOUSE_OVER_SLOT;
        }
        true
    }

    fn on_mouse_move_outside(&self, el: &UIElement, t: &mut pz_kahlua::Thread, dx: f64, dy: f64) {
        el.on_mouse_move_outside_base(t, dx, dy);
        let mut s = self.st.borrow_mut();
        s.mouse_over_slot = MAX_MOUSE_OVER_SLOT;
        s.mouse_over = false;
    }

    fn render(&self, el: &UIElement, t: &mut pz_kahlua::Thread) {
        self.render_moodles(el, t);
    }

    fn update(&self, el: &UIElement, t: &mut pz_kahlua::Thread) {
        self.update_moodles(el, t);
    }
}
