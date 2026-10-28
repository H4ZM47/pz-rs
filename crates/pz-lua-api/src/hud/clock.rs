//! `zombie.ui.Clock`: the digital clock (time, alarm, temperature, date)
//! shown while a player wears or holds an alarm clock.

// The digit loops index by position like the original.
#![allow(clippy::needless_range_loop)]

use super::host::{with_host, AlarmClockItem, AlarmClockRef};
use pz_render::{Rgba, Texture};
use pz_ui::{with_ui, UIElement, UIElementExt};
use std::any::Any;
use std::cell::RefCell;
use std::rc::Rc;

pub const CLOCK: &str = "zombie.ui.Clock";
const SUPERS: &[&str] = &["zombie.ui.UIElement"];
const DIR: &str = "media/ui/ClockAssets/";

/// `new Color(r, g, b, a)` with 0..255 components.
fn color(r: u8, g: u8, b: u8, a: u8) -> Rgba {
    Rgba(
        r as f32 / 255.0,
        g as f32 / 255.0,
        b as f32 / 255.0,
        a as f32 / 255.0,
    )
}

struct Textures {
    digits_large: Vec<Option<Texture>>,
    digits_small: Vec<Option<Texture>>,
    colon: Option<Texture>,
    slash: Option<Texture>,
    minus: Option<Texture>,
    dot: Option<Texture>,
    temp_c: Option<Texture>,
    temp_f: Option<Texture>,
    temp_e: Option<Texture>,
    tex_am: Option<Texture>,
    tex_pm: Option<Texture>,
    alarm_on: Option<Texture>,
    alarm_ringing: Option<Texture>,
}

#[derive(Default, Clone, Copy)]
struct Offsets {
    ux_original: i32,
    uy_original: i32,
    large_digit_spacing: i32,
    small_digit_spacing: i32,
    colon_spacing: i32,
    ampm_spacing: i32,
    alarm_bell_spacing: i32,
    decimal_spacing: i32,
    degree_spacing: i32,
    slash_spacing: i32,
    temp_date_spacing: i32,
    date_offset: i32,
    minus_offset: i32,
    am_vertical_spacing: i32,
    pm_vertical_spacing: i32,
    alarm_bell_vertical_spacing: i32,
    display_vertical_spacing: i32,
    decimal_vertical_spacing: i32,
}

const SMALL: Offsets = Offsets {
    ux_original: 3,
    uy_original: 3,
    large_digit_spacing: 1,
    small_digit_spacing: 1,
    colon_spacing: 1,
    ampm_spacing: 1,
    alarm_bell_spacing: 1,
    decimal_spacing: 1,
    degree_spacing: 1,
    slash_spacing: 1,
    temp_date_spacing: 5,
    date_offset: 33,
    minus_offset: 0,
    am_vertical_spacing: 7,
    pm_vertical_spacing: 12,
    alarm_bell_vertical_spacing: 1,
    display_vertical_spacing: 2,
    decimal_vertical_spacing: 6,
};

const LARGE: Offsets = Offsets {
    ux_original: 3,
    uy_original: 3,
    large_digit_spacing: 2,
    small_digit_spacing: 1,
    colon_spacing: 3,
    ampm_spacing: 3,
    alarm_bell_spacing: 5,
    decimal_spacing: 2,
    degree_spacing: 2,
    slash_spacing: 2,
    temp_date_spacing: 8,
    date_offset: 65,
    minus_offset: -2,
    am_vertical_spacing: 15,
    pm_vertical_spacing: 25,
    alarm_bell_vertical_spacing: 1,
    display_vertical_spacing: 5,
    decimal_vertical_spacing: 15,
};

#[derive(Default)]
struct ClockState {
    large_textures: bool,
    background: Option<Texture>,
    tex: Option<Textures>,
    off: Offsets,
    digital: bool,
    is_alarm_set: bool,
    is_alarm_ringing: bool,
    /// `clockPlayer` (its index).
    clock_player: Option<usize>,
}

/// `zombie.ui.Clock`.
pub struct Clock {
    st: RefCell<ClockState>,
}

fn tw(t: &Option<Texture>) -> i32 {
    t.as_ref().map_or(0, |t| t.width as i32)
}
fn th(t: &Option<Texture>) -> i32 {
    t.as_ref().map_or(0, |t| t.height as i32)
}
fn tex(name: String) -> Option<Texture> {
    pz_render::get_shared_texture(&name)
}

fn clock_size_large() -> bool {
    crate::bind_core::shared_core()
        .borrow()
        .get_option_clock_size()
        == 2
}

fn background_for(large: bool) -> Option<Texture> {
    tex(format!(
        "{DIR}{}",
        if large {
            "ClockLargeBackground.png"
        } else {
            "ClockSmallBackground.png"
        }
    ))
}

impl Clock {
    /// `new Clock(x, y)`.
    pub fn create(x: i32, y: i32) -> Rc<UIElement> {
        let e = UIElement::new_ext(
            None,
            Box::new(Clock {
                st: RefCell::default(),
            }),
        );
        {
            let mut s = e.st_mut();
            s.x = x as f64;
            s.y = y as f64;
        }
        e
    }

    /// `assignTextures(largeTextures)`.
    fn assign_textures(&self, large: bool) {
        let mut s = self.st.borrow_mut();
        if s.tex.is_some() && s.large_textures == large {
            return;
        }
        s.large_textures = large;
        let (lt, st) = if large {
            ("Large", "Medium")
        } else {
            ("Medium", "Small")
        };
        s.off = if large { LARGE } else { SMALL };
        s.background = background_for(large);
        let t = Textures {
            digits_large: (0..10)
                .map(|n| tex(format!("{DIR}ClockDigits{lt}{n}.png")))
                .collect(),
            digits_small: (0..10)
                .map(|n| tex(format!("{DIR}ClockDigits{st}{n}.png")))
                .collect(),
            colon: tex(format!("{DIR}ClockDivide{lt}.png")),
            slash: tex(format!("{DIR}DateDivide{st}.png")),
            minus: tex(format!("{DIR}ClockDigits{st}Minus.png")),
            dot: tex(format!("{DIR}ClockDigits{st}Dot.png")),
            temp_c: tex(format!("{DIR}ClockDigits{st}C.png")),
            temp_f: tex(format!("{DIR}ClockDigits{st}F.png")),
            temp_e: tex(format!("{DIR}ClockDigits{st}E.png")),
            tex_am: tex(format!("{DIR}ClockAm{lt}.png")),
            tex_pm: tex(format!("{DIR}ClockPm{lt}.png")),
            alarm_on: tex(format!("{DIR}ClockAlarm{lt}Set.png")),
            alarm_ringing: tex(format!("{DIR}ClockAlarm{lt}Sound.png")),
        };
        s.tex = Some(t);
    }

    /// `timeDigits()`.
    fn time_digits() -> [usize; 4] {
        let mut time = with_host(|h| h.game_time.digits_time_of_day());
        if !crate::bind_core::shared_core()
            .borrow()
            .get_option_clock24_hour()
        {
            if time >= 13.0 {
                time -= 12.0;
            }
            if time < 1.0 {
                time += 12.0;
            }
        }
        let hours = time as i32;
        let minutes = (time - (time as i32) as f32) * 60.0;
        [
            (hours / 10) as usize,
            (hours % 10) as usize,
            (minutes / 10.0) as i32 as usize,
            0,
        ]
    }

    /// `dateDigits()`.
    fn date_digits() -> [usize; 4] {
        let (day, month) = with_host(|h| (h.game_time.day() + 1, h.game_time.month() + 1));
        let d = [(day / 10) as usize, (day % 10) as usize];
        let m = [(month / 10) as usize, (month % 10) as usize];
        if crate::bind_core::shared_core()
            .borrow()
            .get_option_clock_format()
            == 1
        {
            [m[0], m[1], d[0], d[1]]
        } else {
            [d[0], d[1], m[0], m[1]]
        }
    }

    /// `tempDigits()`.
    fn temp_digits(player: usize) -> [usize; 6] {
        let mut t = with_host(|h| h.climate.air_temperature_for_character(player));
        let (mut negative, mut fahrenheit) = (0, 0);
        if !crate::bind_core::shared_core()
            .borrow()
            .get_option_temperature_display_celsius()
        {
            t = t * 1.8 + 32.0;
            fahrenheit = 1;
        }
        if t < 0.0 {
            negative = 1;
            t *= -1.0;
        }
        let hund = (t as i32) / 100;
        let tens = ((t % 100.0) as i32) / 10;
        let unit = (t as i32) % 10;
        let deci = ((t * 10.0) as i32) % 10;
        [
            negative,
            hund as usize,
            tens as usize,
            unit as usize,
            deci as usize,
            fahrenheit,
        ]
    }

    /// `renderDisplay(ghostDisplay, color)`.
    fn render_display(&self, el: &UIElement, ghost: bool, col: Rgba) {
        let s = self.st.borrow();
        let o = s.off;
        let t = s.tex.as_ref().expect("textures assigned");
        let draw = |tx: &Option<Texture>, x: i32, y: i32| {
            el.draw_texture_col(tx.as_ref(), x as f64, y as f64, col)
        };
        let mut ux = o.ux_original;
        let mut uy = o.uy_original;
        let lw = tw(&t.digits_large[0]);
        for i in 0..4 {
            let td = Self::time_digits();
            if ghost {
                draw(&t.digits_large[8], ux, uy);
            } else {
                draw(&t.digits_large[td[i]], ux, uy);
            }
            ux += lw;
            if i == 1 {
                ux += o.colon_spacing;
                draw(&t.colon, ux, uy);
                ux += tw(&t.colon) + o.colon_spacing;
                continue;
            }
            if i >= 3 {
                continue;
            }
            ux += o.large_digit_spacing;
        }
        ux += o.ampm_spacing;
        let h24 = crate::bind_core::shared_core()
            .borrow()
            .get_option_clock24_hour();
        if !h24 || ghost {
            if ghost {
                draw(&t.tex_am, ux, uy + o.am_vertical_spacing);
                draw(&t.tex_pm, ux, uy + o.pm_vertical_spacing);
            } else if with_host(|h| h.game_time.time_of_day()) < 12.0 {
                draw(&t.tex_am, ux, uy + o.am_vertical_spacing);
            } else {
                draw(&t.tex_pm, ux, uy + o.pm_vertical_spacing);
            }
        }
        let bell_x = ux + tw(&t.tex_am) + o.alarm_bell_spacing;
        if s.is_alarm_ringing || ghost {
            draw(&t.alarm_ringing, bell_x, uy + o.alarm_bell_vertical_spacing);
        } else if s.is_alarm_set {
            draw(&t.alarm_on, bell_x, uy + o.alarm_bell_vertical_spacing);
        }
        if !(s.digital || ghost) {
            return;
        }
        ux = o.ux_original;
        uy += th(&t.digits_large[0]) + o.display_vertical_spacing;
        let sw = tw(&t.digits_small[0]);
        if let Some(player) = s.clock_player {
            let td = Self::temp_digits(player);
            if td[0] == 1 || ghost {
                draw(&t.minus, ux, uy);
            }
            ux += o.minus_offset;
            if td[1] == 1 || ghost {
                draw(&t.digits_small[1], ux, uy);
            }
            ux += sw + o.small_digit_spacing;
            for i in 2..5 {
                if ghost {
                    draw(&t.digits_small[8], ux, uy);
                } else {
                    draw(&t.digits_small[td[i]], ux, uy);
                }
                ux += sw;
                if i == 3 {
                    ux += o.decimal_spacing;
                    draw(&t.dot, ux, uy + o.decimal_vertical_spacing);
                    ux += tw(&t.dot) + o.decimal_spacing;
                    continue;
                }
                if i >= 4 {
                    continue;
                }
                ux += o.small_digit_spacing;
            }
            ux += o.degree_spacing;
            draw(&t.dot, ux, uy);
            ux += tw(&t.dot) + o.degree_spacing;
            if ghost {
                draw(&t.temp_e, ux, uy);
            } else if td[5] == 0 {
                draw(&t.temp_c, ux, uy);
            } else {
                draw(&t.temp_f, ux, uy);
            }
            ux += sw + o.temp_date_spacing;
        } else {
            ux += o.date_offset;
        }
        let dd = Self::date_digits();
        for i in 0..4 {
            if ghost {
                draw(&t.digits_small[8], ux, uy);
            } else {
                draw(&t.digits_small[dd[i]], ux, uy);
            }
            ux += sw;
            if i == 1 {
                ux += o.slash_spacing;
                draw(&t.slash, ux, uy);
                ux += tw(&t.slash) + o.slash_spacing;
                continue;
            }
            if i >= 3 {
                continue;
            }
            ux += o.small_digit_spacing;
        }
    }

    /// `resize()`: visible, digital and the alarm state from the players'
    /// alarm clocks; the size from the background.
    pub fn resize(&self, el: &UIElement) {
        let visible_all = with_ui(|m| m.visible_all_ui);
        el.set_visible(false);
        let mut digital = false;
        let (mut set, mut ringing) = (false, false);
        let mut clock_player = None;
        let mut visible = false;
        with_host(|h| {
            if !h.players.has_player_instance() {
                return;
            }
            for i in 0..h.players.num_players().max(0) as usize {
                if !h.players.player_exists(i) || h.players.player_dead(i) {
                    continue;
                }
                let worn = h.alarm_clocks.worn_clocks(i);
                let inv = h.alarm_clocks.inventory_clocks(i);
                let mut take = |c: &AlarmClockItem, player: &mut Option<usize>| {
                    visible = visible_all;
                    digital |= c.digital_tag;
                    set |= c.alarm_set;
                    ringing |= c.ringing;
                    *player = Some(i);
                };
                for c in &worn {
                    take(c, &mut clock_player);
                }
                if clock_player.is_some() {
                    break;
                }
                for c in inv.iter().filter(|c| c.worn || c.equipped) {
                    take(c, &mut clock_player);
                }
            }
            if h.debug.cheat_clock_visible() {
                digital = true;
                visible = visible_all;
            }
        });
        el.set_visible(visible);
        let bg = {
            let mut s = self.st.borrow_mut();
            s.digital = digital;
            s.is_alarm_set = set;
            s.is_alarm_ringing = ringing;
            s.clock_player = clock_player;
            if s.background.is_none() {
                s.background = background_for(clock_size_large());
            }
            s.background.clone()
        };
        el.set_height(th(&bg) as f64);
        el.set_width(tw(&bg) as f64);
    }

    /// Sets what `resize` derives from the players' clocks (tests).
    #[cfg(test)]
    pub(crate) fn set_state(
        &self,
        digital: bool,
        player: Option<usize>,
        alarm_set: bool,
        ringing: bool,
    ) {
        let mut s = self.st.borrow_mut();
        s.digital = digital;
        s.clock_player = player;
        s.is_alarm_set = alarm_set;
        s.is_alarm_ringing = ringing;
    }

    /// `isDateVisible()`.
    pub fn is_date_visible(&self, el: &UIElement) -> bool {
        el.is_visible() && self.st.borrow().digital
    }
}

impl UIElementExt for Clock {
    fn class_name(&self) -> &'static str {
        CLOCK
    }
    fn super_classes(&self) -> &'static [&'static str] {
        SUPERS
    }
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn render(&self, el: &UIElement, t: &mut pz_kahlua::Thread) {
        if !el.is_visible() {
            return;
        }
        self.assign_textures(clock_size_large());
        let bg = self.st.borrow().background.clone();
        el.draw_texture(bg.as_ref(), 0.0, 0.0, 0.75);
        self.render_display(el, true, color(40, 40, 40, 128));
        self.render_display(el, false, color(100, 200, 210, 255));
        el.render_base(t);
    }

    /// `onMouseDown`: stop a ringing alarm, else clear a set alarm, else
    /// set the first digital alarm clock's alarm.
    fn on_mouse_down(&self, el: &UIElement, _t: &mut pz_kahlua::Thread, _x: f64, _y: f64) -> bool {
        if !el.is_visible() {
            return false;
        }
        let (set, ringing) = {
            let s = self.st.borrow();
            (s.is_alarm_set, s.is_alarm_ringing)
        };
        with_host(|h| {
            if !h.players.has_player_instance() {
                return;
            }
            let n = h.players.num_players().max(0) as usize;
            for i in 0..n {
                if !h.players.player_exists(i) || h.players.player_dead(i) {
                    continue;
                }
                let worn = h.alarm_clocks.worn_clocks(i);
                let inv = h.alarm_clocks.inventory_clocks(i);
                if ringing {
                    for j in 0..worn.len() {
                        h.alarm_clocks.stop_ringing(i, AlarmClockRef::Worn(j));
                    }
                    for j in 0..inv.len() {
                        h.alarm_clocks.stop_ringing(i, AlarmClockRef::Inventory(j));
                    }
                } else if set {
                    for (j, c) in worn.iter().enumerate() {
                        if c.alarm_set {
                            h.alarm_clocks
                                .set_alarm_set(i, AlarmClockRef::Worn(j), false);
                        }
                    }
                    for (j, c) in inv.iter().enumerate() {
                        if c.alarm_set {
                            h.alarm_clocks
                                .set_alarm_set(i, AlarmClockRef::Inventory(j), false);
                        }
                    }
                } else {
                    // `isAlarmSet` is false on this path, so every digital
                    // clock without an alarm gets one.
                    for (j, c) in worn.iter().enumerate() {
                        if c.is_digital && !c.alarm_set {
                            h.alarm_clocks
                                .set_alarm_set(i, AlarmClockRef::Worn(j), true);
                        }
                    }
                    for (j, c) in inv.iter().enumerate() {
                        if c.is_digital && !c.alarm_set {
                            h.alarm_clocks
                                .set_alarm_set(i, AlarmClockRef::Inventory(j), true);
                        }
                    }
                }
            }
        });
        true
    }
}
