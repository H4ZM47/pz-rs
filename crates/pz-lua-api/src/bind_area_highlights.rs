//! `zombie.iso.fboRenderChunk.FBORenderAreaHighlights` and the Build 42.22
//! globals `addAreaHighlight` / `addAreaHighlightForPlayer` that queue
//! floor area highlights for the current UI frame.

use std::cell::RefCell;
use std::rc::Rc;

use pz_iso_render::area_highlights::AreaHighlight;
use pz_kahlua::{LuaObject, Thread, Value};

use crate::expose::{expose_class, global, int_arg, num_arg};

const CLASS: &str = "zombie.iso.fboRenderChunk.FBORenderAreaHighlights";

thread_local! {
    static HIGHLIGHTS: RefCell<Vec<(Option<i32>, AreaHighlight)>> = const { RefCell::new(Vec::new()) };
}

struct LuaAreaHighlights;

impl LuaObject for LuaAreaHighlights {
    fn class_name(&self) -> &str {
        CLASS
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

/// Pushes an area highlight queued for `player` (`None` = all players).
pub fn push_highlight(player: Option<i32>, area: AreaHighlight) {
    HIGHLIGHTS.with(|h| h.borrow_mut().push((player, area)));
}

/// Snapshots the highlights queued for the current UI frame.
pub fn snapshot() -> Vec<(Option<i32>, AreaHighlight)> {
    HIGHLIGHTS.with(|h| h.borrow().clone())
}

/// Clears all queued area highlights (called after the UI frame and on world exit).
pub fn clear() {
    HIGHLIGHTS.with(|h| h.borrow_mut().clear());
}

fn read_area(c: &pz_kahlua::NativeCall<'_>, base: usize) -> AreaHighlight {
    AreaHighlight {
        x1: int_arg(c, base),
        y1: int_arg(c, base + 1),
        x2: int_arg(c, base + 2),
        y2: int_arg(c, base + 3),
        z: int_arg(c, base + 4),
        rgba: [
            num_arg(c, base + 5) as f32,
            num_arg(c, base + 6) as f32,
            num_arg(c, base + 7) as f32,
            num_arg(c, base + 8) as f32,
        ],
    }
}

pub fn register(thread: &mut Thread) {
    let b = expose_class(thread, CLASS, Some("java.lang.Object"));
    b.static_fn("getInstance", |c| {
        Ok(c.push(Value::Object(Rc::new(LuaAreaHighlights))))
    });
    b.method("addHighlight", |c| {
        crate::expose::this::<LuaAreaHighlights>(c)?;
        push_highlight(None, read_area(c, 1));
        Ok(0)
    });
    b.method("addHighlightForPlayer", |c| {
        crate::expose::this::<LuaAreaHighlights>(c)?;
        let player = int_arg(c, 1);
        push_highlight(Some(player), read_area(c, 2));
        Ok(0)
    });
    b.method("clear", |c| {
        crate::expose::this::<LuaAreaHighlights>(c)?;
        clear();
        Ok(0)
    });

    global(thread, "addAreaHighlight", |c| {
        push_highlight(None, read_area(c, 0));
        Ok(0)
    });
    global(thread, "addAreaHighlightForPlayer", |c| {
        let player = int_arg(c, 0);
        push_highlight(Some(player), read_area(c, 1));
        Ok(0)
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use pz_kahlua::TableRef;

    #[test]
    fn lua_add_area_highlight_and_for_player_queue_highlights() {
        clear();
        let mut t = Thread::new(TableRef::new());
        pz_kahlua::stdlib::install(&mut t, None).unwrap();
        register(&mut t);
        let code = r#"
            addAreaHighlight(10, 20, 15, 25, 0, 1.0, 0.5, 0.25, 0.4)
            addAreaHighlightForPlayer(1, 30, 40, 32, 42, 1, 0.1, 0.2, 0.3, 0.5)
            local inst = zombie.iso.fboRenderChunk.FBORenderAreaHighlights.getInstance()
            inst:addHighlightForPlayer(0, 1, 2, 3, 4, 0, 0.0, 1.0, 0.0, 0.25)
        "#;
        let f = pz_kahlua::stdlib::load_string(code, Some("area_highlights"), t.globals().clone())
            .unwrap();
        t.pcall(&f, &[]).unwrap();
        let queued = snapshot();
        assert_eq!(queued.len(), 3);
        assert_eq!(queued[0].0, None);
        assert_eq!(
            queued[0].1,
            AreaHighlight {
                x1: 10,
                y1: 20,
                x2: 15,
                y2: 25,
                z: 0,
                rgba: [1.0, 0.5, 0.25, 0.4],
            }
        );
        assert_eq!(queued[1].0, Some(1));
        assert_eq!(queued[1].1.x1, 30);
        assert_eq!(queued[1].1.z, 1);
        assert_eq!(queued[2].0, Some(0));
        assert_eq!(queued[2].1.x2, 3);
        clear();
        assert!(snapshot().is_empty());
    }
}
