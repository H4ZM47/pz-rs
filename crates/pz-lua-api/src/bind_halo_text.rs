//! `zombie.characters.HaloTextHelper` and `HaloTextHelper$ColorRGB` — queued
//! floating overhead halo notes for local players (`XpUpdate.lua`,
//! `forageSystem.lua`, `ISReadABook.lua`, `ISRadioInteractions.lua`, etc.).

use std::any::Any;
use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::rc::Rc;

use pz_kahlua::value::LuaObject;
use pz_kahlua::{NativeCall, Thread, Value};

use crate::bind_character_sim::CharacterObject;
use crate::expose::{bool_arg, expose_class, int_arg, str_arg, with_this};

pub const HALO_TEXT_HELPER_CLASS: &str = "zombie.characters.HaloTextHelper";
pub const COLOR_RGB_CLASS: &str = "zombie.characters.HaloTextHelper$ColorRGB";

const MAX_PLAYERS: usize = 4;

/// `HaloTextHelper$ColorRGB`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HaloColorRgb {
    pub r: i32,
    pub g: i32,
    pub b: i32,
}

impl LuaObject for HaloColorRgb {
    fn class_name(&self) -> &str {
        COLOR_RGB_CLASS
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// One queued halo text entry for a local player.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HaloEntry {
    pub text: String,
    pub separator: String,
    pub color: HaloColorRgb,
    pub arrow: Option<bool>,
}

thread_local! {
    static QUEUES: RefCell<[VecDeque<HaloEntry>; MAX_PLAYERS]> =
        RefCell::new(std::array::from_fn(|_| VecDeque::new()));
    static LAST_DISPLAYED: RefCell<[Option<HaloEntry>; MAX_PLAYERS]> =
        RefCell::new(std::array::from_fn(|_| None));
    static FORCE_NEXT: Cell<bool> = const { Cell::new(false) };
}

pub const COLOR_WHITE: HaloColorRgb = HaloColorRgb { r: 255, g: 255, b: 255 };
pub const COLOR_RED: HaloColorRgb = HaloColorRgb { r: 255, g: 32, b: 32 };
pub const COLOR_GREEN: HaloColorRgb = HaloColorRgb { r: 137, g: 232, b: 148 };

/// Good/bad colors read from `Core`'s accessibility highlight colors when
/// available, falling back to PZ's standard green/red RGB values.
pub fn good_color() -> HaloColorRgb {
    let c = crate::bind_core::shared_core()
        .borrow_mut()
        .get_good_highlited_color();
    HaloColorRgb {
        r: (c.r * 255.0).round().clamp(0.0, 255.0) as i32,
        g: (c.g * 255.0).round().clamp(0.0, 255.0) as i32,
        b: (c.b * 255.0).round().clamp(0.0, 255.0) as i32,
    }
}

pub fn bad_color() -> HaloColorRgb {
    let c = crate::bind_core::shared_core()
        .borrow_mut()
        .get_bad_highlited_color();
    HaloColorRgb {
        r: (c.r * 255.0).round().clamp(0.0, 255.0) as i32,
        g: (c.g * 255.0).round().clamp(0.0, 255.0) as i32,
        b: (c.b * 255.0).round().clamp(0.0, 255.0) as i32,
    }
}

/// Queue a halo text entry for `player_index` (`0..4`).
pub fn push_entry(player_index: usize, entry: HaloEntry) {
    if player_index >= MAX_PLAYERS {
        return;
    }
    let force = FORCE_NEXT.with(|f| {
        let v = f.get();
        f.set(false);
        v
    });
    QUEUES.with(|q| {
        let mut q = q.borrow_mut();
        let queue = &mut q[player_index];
        if !force && queue.back().is_some_and(|e| e.text == entry.text) {
            return;
        }
        queue.push_back(entry);
    });
}

/// `HaloTextHelper.update()`: advance queued halo text notes for each local
/// player and feed active notes into `IsoPlayer::add_line_chat_element` when
/// available.
pub fn update() {
    let popped: [Option<HaloEntry>; MAX_PLAYERS] = QUEUES.with(|q| {
        let mut q = q.borrow_mut();
        std::array::from_fn(|i| q[i].pop_front())
    });
    LAST_DISPLAYED.with(|last| {
        let mut last = last.borrow_mut();
        for (i, entry) in popped.into_iter().enumerate() {
            if let Some(e) = entry {
                last[i] = Some(e);
            }
        }
    });
}

/// Snapshot the latest displayed halo entry for player `index` (for UI/tests).
pub fn last_displayed(player_index: usize) -> Option<HaloEntry> {
    if player_index >= MAX_PLAYERS {
        return None;
    }
    LAST_DISPLAYED.with(|last| last.borrow()[player_index].clone())
}

/// Number of queued entries waiting for player `index`.
pub fn queued_len(player_index: usize) -> usize {
    if player_index >= MAX_PLAYERS {
        return 0;
    }
    QUEUES.with(|q| q.borrow()[player_index].len())
}

/// Reset all queued and active halo text state on world exit.
pub fn reset() {
    QUEUES.with(|q| {
        for queue in q.borrow_mut().iter_mut() {
            queue.clear();
        }
    });
    LAST_DISPLAYED.with(|last| {
        for slot in last.borrow_mut().iter_mut() {
            *slot = None;
        }
    });
    FORCE_NEXT.with(|f| f.set(false));
}

fn player_index_arg(c: &NativeCall<'_>, idx: usize) -> usize {
    match c.arg(idx) {
        Value::Object(o) => {
            if let Some(ch) = o.as_any().downcast_ref::<CharacterObject>() {
                let target = &ch.0;
                for i in 0..MAX_PLAYERS {
                    if let Some(p) = crate::bind_players::player(i) {
                        if Rc::ptr_eq(&p, target) {
                            return i;
                        }
                    }
                }
            }
            0
        }
        Value::Num(n) => (n as usize).min(MAX_PLAYERS - 1),
        _ => 0,
    }
}

fn color_arg(val: &Value) -> Option<HaloColorRgb> {
    match val {
        Value::Object(o) => o.as_any().downcast_ref::<HaloColorRgb>().copied(),
        _ => None,
    }
}

fn color_value(c: HaloColorRgb) -> Value {
    Value::Object(Rc::new(c))
}

/// Expose `zombie.characters.HaloTextHelper` and `HaloTextHelper$ColorRGB`.
pub fn register(thread: &mut Thread) {
    let rgb = expose_class(thread, COLOR_RGB_CLASS, Some("java.lang.Object"));
    rgb.static_fn("new", |c| {
        let col = HaloColorRgb {
            r: int_arg(c, 0),
            g: int_arg(c, 1),
            b: int_arg(c, 2),
        };
        Ok(c.push(color_value(col)))
    });
    rgb.method("getR", |c| {
        let v = with_this::<HaloColorRgb, _>(c, |col| col.r)?;
        Ok(c.push(v))
    });
    rgb.method("getG", |c| {
        let v = with_this::<HaloColorRgb, _>(c, |col| col.g)?;
        Ok(c.push(v))
    });
    rgb.method("getB", |c| {
        let v = with_this::<HaloColorRgb, _>(c, |col| col.b)?;
        Ok(c.push(v))
    });

    let b = expose_class(thread, HALO_TEXT_HELPER_CLASS, Some("java.lang.Object"));
    b.field("COLOR_WHITE", color_value(COLOR_WHITE));
    b.field("COLOR_RED", color_value(COLOR_RED));
    b.field("COLOR_GREEN", color_value(COLOR_GREEN));
    b.static_fn("getColorWhite", |c| Ok(c.push(color_value(COLOR_WHITE))));
    b.static_fn("getColorRed", |c| Ok(c.push(color_value(COLOR_RED))));
    b.static_fn("getColorGreen", |c| Ok(c.push(color_value(COLOR_GREEN))));
    b.static_fn("getGoodColor", |c| Ok(c.push(color_value(good_color()))));
    b.static_fn("getBadColor", |c| Ok(c.push(color_value(bad_color()))));
    b.static_fn("forceNextAddText", |_| {
        FORCE_NEXT.with(|f| f.set(true));
        Ok(0)
    });
    b.static_fn("update", |_| {
        update();
        Ok(0)
    });

    b.static_fn("addGoodText", |c| {
        let p = player_index_arg(c, 0);
        let text = str_arg(c, 1).map(|s| s.as_ref().to_owned()).unwrap_or_default();
        let sep = str_arg(c, 2)
            .map(|s| s.as_ref().to_owned())
            .unwrap_or_else(|| " ".into());
        push_entry(
            p,
            HaloEntry {
                text,
                separator: sep,
                color: good_color(),
                arrow: None,
            },
        );
        Ok(0)
    });
    b.static_fn("addBadText", |c| {
        let p = player_index_arg(c, 0);
        let text = str_arg(c, 1).map(|s| s.as_ref().to_owned()).unwrap_or_default();
        let sep = str_arg(c, 2)
            .map(|s| s.as_ref().to_owned())
            .unwrap_or_else(|| " ".into());
        push_entry(
            p,
            HaloEntry {
                text,
                separator: sep,
                color: bad_color(),
                arrow: None,
            },
        );
        Ok(0)
    });
    b.static_fn("addText", |c| {
        let p = player_index_arg(c, 0);
        let text = str_arg(c, 1).map(|s| s.as_ref().to_owned()).unwrap_or_default();
        let (sep, color) = match c.nargs() {
            0..=2 => (" ".into(), COLOR_WHITE),
            3 => {
                if let Some(col) = color_arg(&c.arg(2)) {
                    (" ".into(), col)
                } else {
                    (
                        str_arg(c, 2)
                            .map(|s| s.as_ref().to_owned())
                            .unwrap_or_else(|| " ".into()),
                        COLOR_WHITE,
                    )
                }
            }
            4 => (
                str_arg(c, 2)
                    .map(|s| s.as_ref().to_owned())
                    .unwrap_or_else(|| " ".into()),
                color_arg(&c.arg(3)).unwrap_or(COLOR_WHITE),
            ),
            5 => (
                " ".into(),
                HaloColorRgb {
                    r: int_arg(c, 2),
                    g: int_arg(c, 3),
                    b: int_arg(c, 4),
                },
            ),
            _ => (
                str_arg(c, 2)
                    .map(|s| s.as_ref().to_owned())
                    .unwrap_or_else(|| " ".into()),
                HaloColorRgb {
                    r: int_arg(c, 3),
                    g: int_arg(c, 4),
                    b: int_arg(c, 5),
                },
            ),
        };
        push_entry(
            p,
            HaloEntry {
                text,
                separator: sep,
                color,
                arrow: None,
            },
        );
        Ok(0)
    });
    b.static_fn("addTextWithArrow", |c| {
        let p = player_index_arg(c, 0);
        let text = str_arg(c, 1).map(|s| s.as_ref().to_owned()).unwrap_or_default();
        let (sep, arrow_up, color) = if matches!(c.arg(2), Value::Bool(_)) {
            let up = bool_arg(c, 2);
            let col = color_arg(&c.arg(3)).unwrap_or_else(|| HaloColorRgb {
                r: int_arg(c, 3),
                g: int_arg(c, 4),
                b: int_arg(c, 5),
            });
            (" ".into(), up, col)
        } else {
            let sep = str_arg(c, 2)
                .map(|s| s.as_ref().to_owned())
                .unwrap_or_else(|| " ".into());
            let up = bool_arg(c, 3);
            let col = color_arg(&c.arg(4)).unwrap_or_else(|| HaloColorRgb {
                r: int_arg(c, 4),
                g: int_arg(c, 5),
                b: int_arg(c, 6),
            });
            (sep, up, col)
        };
        push_entry(
            p,
            HaloEntry {
                text,
                separator: sep,
                color,
                arrow: Some(arrow_up),
            },
        );
        Ok(0)
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use pz_kahlua::stdlib::load_string;
    use pz_kahlua::TableRef;

    #[test]
    fn halo_text_helper_overloads_and_update_queue() {
        reset();
        let mut t = Thread::new(TableRef::new());
        register(&mut t);

        let f = load_string(
            r#"
            HaloTextHelper.addGoodText(nil, "Learned Recipe", "[br/]")
            HaloTextHelper.addTextWithArrow(nil, "Woodwork +5", "[br/]", true, HaloTextHelper.getGoodColor())
            HaloTextHelper.addBadText(nil, "Too dark")
            return HaloTextHelper.getColorGreen():getG(), HaloTextHelper.getColorRed():getR()
            "#,
            Some("halo_test.lua"),
            t.globals().clone(),
        )
        .unwrap();
        let res = t.pcall(&f, &[]).unwrap();
        assert_eq!(res[0].as_num(), Some(232.0));
        assert_eq!(res[1].as_num(), Some(255.0));
        assert_eq!(queued_len(0), 3);

        update();
        assert_eq!(last_displayed(0).unwrap().text, "Learned Recipe");
        update();
        let second = last_displayed(0).unwrap();
        assert_eq!(second.text, "Woodwork +5");
        assert_eq!(second.arrow, Some(true));
        update();
        assert_eq!(last_displayed(0).unwrap().text, "Too dark");
        assert_eq!(queued_len(0), 0);

        reset();
    }
}
