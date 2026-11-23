//! `IsoObject`'s outline highlight Lua methods (`setOutlineHighlight`,
//! `isOutlineHighlight`, `setOutlineHighlightCol`, `getOutlineHighlightCol`,
//! `setOutlineHlAttached`, `isOutlineHlAttached`, `setOutlineHlBlink`,
//! `isOutlineHlBlink`, `unsetOutlineHighlight`).

use crate::bind_character_sim::outline::{ui_times_now, RENDER_CONTAINER_HIGHLIGHT};
use crate::bind_characters::color_info_arg;
use crate::expose::{bool_arg, int_arg, num_arg, ClassBuilder};
use pz_kahlua::{LuaError, LuaResult, NativeCall, Value};
use pz_world::objects::ObjectOutlineState;

use super::receiver_object;

fn out_of_bounds(index: i32) -> LuaError {
    LuaError::Runtime {
        message: format!("Index {index} out of bounds for length 4"),
        class: "java.lang.ArrayIndexOutOfBoundsException",
    }
}

fn player_index(c: &NativeCall<'_>, i: usize) -> LuaResult<usize> {
    let n = int_arg(c, i);
    usize::try_from(n)
        .ok()
        .filter(|&n| n < 4)
        .ok_or_else(|| out_of_bounds(n))
}

fn shift_index(c: &NativeCall<'_>, i: usize) -> usize {
    (int_arg(c, i) & 31) as usize
}

fn with_outline<R>(
    c: &NativeCall<'_>,
    f: impl FnOnce(&mut ObjectOutlineState) -> R,
) -> LuaResult<R> {
    let obj_ref = receiver_object(c)?;
    let mut obj = obj_ref
        .try_borrow_mut()
        .map_err(|_| super::borrowed("IsoObject outline"))?;
    Ok(f(&mut obj.outline))
}

fn ui_now() -> (i64, i64) {
    let t = ui_times_now();
    (t.render, t.update)
}

pub(super) fn register(b: &ClassBuilder) {
    b.method("setOutlineHighlight", |c| {
        if c.nargs() >= 3 {
            let player = shift_index(c, 1);
            let on = bool_arg(c, 2);
            let ui = ui_now();
            with_outline(c, |o| o.set(player, on, ui))?;
        } else {
            let on = bool_arg(c, 1);
            let ui = ui_now();
            with_outline(c, |o| o.set_all(on, ui))?;
        }
        Ok(0)
    });
    b.method("isOutlineHighlight", |c| {
        let on = if c.nargs() >= 2 {
            let player = shift_index(c, 1);
            with_outline(c, |o| o.is_on(player, RENDER_CONTAINER_HIGHLIGHT))?
        } else {
            with_outline(c, |o| o.is_any())?
        };
        Ok(c.push(on))
    });
    b.method("setOutlineHlAttached", |c| {
        if c.nargs() >= 3 {
            let player = shift_index(c, 1);
            let on = bool_arg(c, 2);
            with_outline(c, |o| o.set_attached(player, on))?;
        } else {
            let on = bool_arg(c, 1);
            with_outline(c, |o| o.set_attached_all(on))?;
        }
        Ok(0)
    });
    b.method("isOutlineHlAttached", |c| {
        let on = if c.nargs() >= 2 {
            let player = shift_index(c, 1);
            with_outline(c, |o| o.is_attached(player))?
        } else {
            with_outline(c, |o| o.is_attached_any())?
        };
        Ok(c.push(on))
    });
    b.method("setOutlineHlBlink", |c| {
        if c.nargs() >= 3 {
            let player = shift_index(c, 1);
            let on = bool_arg(c, 2);
            with_outline(c, |o| o.set_blink(player, on))?;
        } else {
            let on = bool_arg(c, 1);
            with_outline(c, |o| o.set_blink_all(on))?;
        }
        Ok(0)
    });
    b.method("isOutlineHlBlink", |c| {
        let on = if c.nargs() >= 2 {
            let player = shift_index(c, 1);
            with_outline(c, |o| o.is_blink(player))?
        } else {
            with_outline(c, |o| o.is_blink_any())?
        };
        Ok(c.push(on))
    });
    b.method("setOutlineHighlightCol", |c| {
        match c.nargs() {
            n if n >= 6 => {
                let player = player_index(c, 1)?;
                let [r, g, b, a] = [2, 3, 4, 5].map(|i| num_arg(c, i) as f32);
                with_outline(c, |o| o.set_col(player, r, g, b, a))?;
            }
            n if n >= 5 => {
                let [r, g, b, a] = [1, 2, 3, 4].map(|i| num_arg(c, i) as f32);
                with_outline(c, |o| o.set_col_all(r, g, b, a))?;
            }
            3 => {
                if matches!(c.arg(2), Value::Nil) {
                    return Ok(0);
                }
                let player = player_index(c, 1)?;
                let ci = color_info_arg(c, 2)?;
                with_outline(c, |o| o.set_col(player, ci.r, ci.g, ci.b, ci.a))?;
            }
            _ => {
                if matches!(c.arg(1), Value::Nil) {
                    return Ok(0);
                }
                let ci = color_info_arg(c, 1)?;
                with_outline(c, |o| o.set_col_all(ci.r, ci.g, ci.b, ci.a))?;
            }
        }
        Ok(0)
    });
    b.method("getOutlineHighlightCol", |c| {
        let player = player_index(c, 1)?;
        let col = with_outline(c, |o| o.col(player))?;
        Ok(c.push(f64::from(col)))
    });
    b.method("unsetOutlineHighlight", |c| {
        with_outline(c, ObjectOutlineState::unset)?;
        Ok(0)
    });
}
