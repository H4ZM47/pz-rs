//! Build 42.22 `zombie.debug.LineDrawer`:
//! stores queued debug lines, named lines, iso rects, and iso circles for
//! debug overlays and clears per-frame lines in `IngameState::render`.

use crate::expose::{bool_arg, expose_class, int_arg, num_arg, str_arg};
use pz_kahlua::{Thread, Value};
use std::cell::RefCell;

pub const LINE_DRAWER_CLASS: &str = "zombie.debug.LineDrawer";

#[derive(Clone, Debug, PartialEq)]
pub struct DebugLine {
    pub x1: f32,
    pub y1: f32,
    pub z1: f32,
    pub x2: f32,
    pub y2: f32,
    pub z2: f32,
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
    pub name: Option<String>,
    pub solid: bool,
}

#[derive(Default)]
struct LineDrawerState {
    lines: Vec<DebugLine>,
}

thread_local! {
    static STATE: RefCell<LineDrawerState> = RefCell::new(LineDrawerState::default());
}

/// `LineDrawer.clear()`: drops all unnamed per-frame lines (named lines remain
/// until `removeLine(name)` or session reset, matching Build 42.22).
pub fn clear() {
    STATE.with(|s| {
        s.borrow_mut().lines.retain(|l| l.name.is_some());
    });
}

/// Full reset on session teardown (clears named lines as well).
pub(crate) fn reset_session_handles() {
    STATE.with(|s| s.borrow_mut().lines.clear());
}

/// Snapshot of currently queued debug lines for renderers / tests.
pub fn snapshot_lines() -> Vec<DebugLine> {
    STATE.with(|s| s.borrow().lines.clone())
}

fn push_line(line: DebugLine) {
    STATE.with(|s| {
        let mut st = s.borrow_mut();
        if let Some(ref name) = line.name {
            st.lines.retain(|existing| existing.name.as_deref() != Some(name.as_str()));
        }
        st.lines.push(line);
    });
}

fn remove_named_line(name: &str) {
    STATE.with(|s| {
        s.borrow_mut()
            .lines
            .retain(|l| l.name.as_deref() != Some(name));
    });
}

pub fn register(thread: &mut Thread) {
    let b = expose_class(thread, LINE_DRAWER_CLASS, Some("java.lang.Object"));

    // `addLine(x1, y1, z1, x2, y2, z2, r, g, b, [a_or_name], [solid])`
    b.static_fn("addLine", |c| {
        let x1 = num_arg(c, 0) as f32;
        let y1 = num_arg(c, 1) as f32;
        let z1 = num_arg(c, 2) as f32;
        let x2 = num_arg(c, 3) as f32;
        let y2 = num_arg(c, 4) as f32;
        let z2 = num_arg(c, 5) as f32;
        let r = num_arg(c, 6) as f32;
        let g = num_arg(c, 7) as f32;
        let b = num_arg(c, 8) as f32;
        let (a, name, solid) = match c.nargs() {
            n if n >= 11 => (
                1.0,
                match c.arg(9) {
                    Value::Str(s) => Some(s.to_string()),
                    _ => None,
                },
                bool_arg(c, 10),
            ),
            10 => match c.arg(9) {
                Value::Str(s) => (1.0, Some(s.to_string()), true),
                _ => (num_arg(c, 9) as f32, None, true),
            },
            _ => (1.0, None, true),
        };
        push_line(DebugLine {
            x1,
            y1,
            z1,
            x2,
            y2,
            z2,
            r,
            g,
            b,
            a,
            name,
            solid,
        });
        Ok(0)
    });

    // `addRect(x, y, z, w, h, r, g, b)` -> 4 edges.
    b.static_fn("addRect", |c| {
        let x = num_arg(c, 0) as f32;
        let y = num_arg(c, 1) as f32;
        let z = num_arg(c, 2) as f32;
        let w = num_arg(c, 3) as f32;
        let h = num_arg(c, 4) as f32;
        let r = num_arg(c, 5) as f32;
        let g = num_arg(c, 6) as f32;
        let b = num_arg(c, 7) as f32;
        for (x1, y1, x2, y2) in [
            (x, y, x + w, y),
            (x + w, y, x + w, y + h),
            (x + w, y + h, x, y + h),
            (x, y + h, x, y),
        ] {
            push_line(DebugLine {
                x1,
                y1,
                z1: z,
                x2,
                y2,
                z2: z,
                r,
                g,
                b,
                a: 1.0,
                name: None,
                solid: true,
            });
        }
        Ok(0)
    });

    b.static_fn("addRectYOffset", |c| {
        let x = num_arg(c, 0) as f32;
        let y = num_arg(c, 1) as f32;
        let z = num_arg(c, 2) as f32;
        let w = num_arg(c, 3) as f32;
        let h = num_arg(c, 4) as f32;
        let _y_off = int_arg(c, 5);
        let r = num_arg(c, 6) as f32;
        let g = num_arg(c, 7) as f32;
        let b = num_arg(c, 8) as f32;
        for (x1, y1, x2, y2) in [
            (x, y, x + w, y),
            (x + w, y, x + w, y + h),
            (x + w, y + h, x, y + h),
            (x, y + h, x, y),
        ] {
            push_line(DebugLine {
                x1,
                y1,
                z1: z,
                x2,
                y2,
                z2: z,
                r,
                g,
                b,
                a: 1.0,
                name: None,
                solid: true,
            });
        }
        Ok(0)
    });

    b.static_fn("DrawIsoLine", |c| {
        let x1 = num_arg(c, 0) as f32;
        let y1 = num_arg(c, 1) as f32;
        let z1 = num_arg(c, 2) as f32;
        let x2 = num_arg(c, 3) as f32;
        let y2 = num_arg(c, 4) as f32;
        let z2 = num_arg(c, 5) as f32;
        let r = num_arg(c, 6) as f32;
        let g = num_arg(c, 7) as f32;
        let b = num_arg(c, 8) as f32;
        let a = num_arg(c, 9) as f32;
        push_line(DebugLine {
            x1,
            y1,
            z1,
            x2,
            y2,
            z2,
            r,
            g,
            b,
            a,
            name: None,
            solid: true,
        });
        Ok(0)
    });

    b.static_fn("addAlphaDecayingLine", |c| {
        let x1 = num_arg(c, 0) as f32;
        let y1 = num_arg(c, 1) as f32;
        let z1 = num_arg(c, 2) as f32;
        let x2 = num_arg(c, 3) as f32;
        let y2 = num_arg(c, 4) as f32;
        let z2 = num_arg(c, 5) as f32;
        let r = num_arg(c, 6) as f32;
        let g = num_arg(c, 7) as f32;
        let b = num_arg(c, 8) as f32;
        let a = num_arg(c, 9) as f32;
        push_line(DebugLine {
            x1,
            y1,
            z1,
            x2,
            y2,
            z2,
            r,
            g,
            b,
            a,
            name: None,
            solid: true,
        });
        Ok(0)
    });

    for draw_fn in [
        "DrawRect",
        "DrawIsoRect",
        "DrawIsoRectRotated",
        "DrawIsoTransform",
        "DrawIsoCircle",
        "drawLine",
        "drawRect",
        "drawArc",
        "drawCircle",
        "drawDirectionLine",
        "drawDotLines",
        "addAlphaDecayingIsoCircle",
        "render",
        "drawLines",
    ] {
        b.static_fn(draw_fn, |_c| Ok(0));
    }

    b.static_fn("removeLine", |c| {
        if let Some(name) = str_arg(c, 0) {
            remove_named_line(&name);
        }
        Ok(0)
    });

    b.static_fn("clear", |_c| {
        clear();
        Ok(0)
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use pz_kahlua::{stdlib, TableRef};

    #[test]
    fn line_drawer_queues_named_and_per_frame_lines() {
        reset_session_handles();
        let mut t = Thread::new(TableRef::new());
        stdlib::install(&mut t, None).unwrap();
        register(&mut t);

        let src = r#"
            LineDrawer.addLine(1, 2, 0, 3, 4, 0, 1, 0, 0, 1)
            LineDrawer.addLine(5, 6, 0, 7, 8, 0, 0, 1, 0, "persistent", true)
            LineDrawer.addRect(10, 10, 0, 2, 2, 0, 0, 1)
            LineDrawer.DrawIsoCircle(10, 10, 0, 1.5, 16, 1, 1, 1, 1)
        "#;
        let f = stdlib::load_string(src, Some("line_drawer"), t.globals().clone()).unwrap();
        t.pcall(&f, &[]).unwrap();

        // 1 unnamed + 1 named + 4 rect edges = 6 lines.
        assert_eq!(snapshot_lines().len(), 6);

        // `LineDrawer.clear()` drops unnamed per-frame lines and retains named ones.
        clear();
        let remaining = snapshot_lines();
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].name.as_deref(), Some("persistent"));

        let f2 = stdlib::load_string(
            r#"LineDrawer.removeLine("persistent")"#,
            Some("remove"),
            t.globals().clone(),
        )
        .unwrap();
        t.pcall(&f2, &[]).unwrap();
        assert!(snapshot_lines().is_empty());
    }
}
