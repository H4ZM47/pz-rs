//! Build 42.22 ambient audio managers:
//! - `zombie.audio.FMODAmbientWalls` (`ENABLE`, `getInstance`, `update`,
//!   `squareChanged`, `render`)
//! - `zombie.audio.ObjectAmbientEmitters` (`getInstance`, `update`, `render`,
//!   `Reset`)
//! Wired into `IngameState::update`, `IngameState::exit`, and `session_reset`.

use crate::expose::expose_class;
use pz_kahlua::value::LuaObject;
use pz_kahlua::{Thread, Value};
use std::any::Any;
use std::cell::RefCell;
use std::collections::HashSet;
use std::rc::Rc;

pub const FMOD_AMBIENT_WALLS_CLASS: &str = "zombie.audio.FMODAmbientWalls";
pub const OBJECT_AMBIENT_EMITTERS_CLASS: &str = "zombie.audio.ObjectAmbientEmitters";

pub struct FmodAmbientWallsObject;

impl LuaObject for FmodAmbientWallsObject {
    fn class_name(&self) -> &str {
        FMOD_AMBIENT_WALLS_CLASS
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

pub struct ObjectAmbientEmittersObject;

impl LuaObject for ObjectAmbientEmittersObject {
    fn class_name(&self) -> &str {
        OBJECT_AMBIENT_EMITTERS_CLASS
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

#[derive(Default)]
struct AmbientAudioState {
    walls_enabled: bool,
    dirty_wall_squares: HashSet<(i32, i32, i32)>,
    walls_updates: u64,
    emitters_updates: u64,
}

thread_local! {
    static STATE: RefCell<AmbientAudioState> = RefCell::new(AmbientAudioState {
        walls_enabled: true,
        ..Default::default()
    });
    static WALLS_INST: RefCell<Option<Rc<FmodAmbientWallsObject>>> = const { RefCell::new(None) };
    static EMITTERS_INST: RefCell<Option<Rc<ObjectAmbientEmittersObject>>> = const { RefCell::new(None) };
}

fn walls_instance() -> Rc<FmodAmbientWallsObject> {
    WALLS_INST.with(|s| {
        s.borrow_mut()
            .get_or_insert_with(|| Rc::new(FmodAmbientWallsObject))
            .clone()
    })
}

fn emitters_instance() -> Rc<ObjectAmbientEmittersObject> {
    EMITTERS_INST.with(|s| {
        s.borrow_mut()
            .get_or_insert_with(|| Rc::new(ObjectAmbientEmittersObject))
            .clone()
    })
}

/// `FMODAmbientWalls.getInstance().update()`.
pub fn update_ambient_walls() {
    STATE.with(|s| {
        let mut st = s.borrow_mut();
        if st.walls_enabled {
            st.dirty_wall_squares.clear();
            st.walls_updates = st.walls_updates.saturating_add(1);
        }
    });
}

/// `FMODAmbientWalls.getInstance().squareChanged(sq)`.
pub fn wall_square_changed(x: i32, y: i32, z: i32) {
    STATE.with(|s| {
        s.borrow_mut().dirty_wall_squares.insert((x, y, z));
    });
}

/// `ObjectAmbientEmitters.getInstance().update()`.
pub fn update_object_emitters() {
    STATE.with(|s| {
        let mut st = s.borrow_mut();
        st.emitters_updates = st.emitters_updates.saturating_add(1);
    });
}

/// `ObjectAmbientEmitters.getInstance().Reset()`.
pub fn reset_object_emitters() {
    STATE.with(|s| {
        let mut st = s.borrow_mut();
        st.dirty_wall_squares.clear();
        st.walls_updates = 0;
        st.emitters_updates = 0;
    });
}

pub(crate) fn reset_session_handles() {
    reset_object_emitters();
}

pub fn register(thread: &mut Thread) {
    let w = expose_class(thread, FMOD_AMBIENT_WALLS_CLASS, Some("java.lang.Object"));
    w.field("ENABLE", true);
    w.static_fn("getInstance", |c| Ok(c.push(Value::Object(walls_instance()))));
    w.method("update", |_c| {
        update_ambient_walls();
        Ok(0)
    });
    w.method("squareChanged", |c| {
        if let Some((x, y, z)) = crate::bind_world::square_coords(&c.arg(1)) {
            wall_square_changed(x, y, z);
        }
        Ok(0)
    });
    w.method("render", |_c| Ok(0));

    let e = expose_class(thread, OBJECT_AMBIENT_EMITTERS_CLASS, Some("java.lang.Object"));
    e.static_fn("getInstance", |c| Ok(c.push(Value::Object(emitters_instance()))));
    e.method("update", |_c| {
        update_object_emitters();
        Ok(0)
    });
    e.method("render", |_c| Ok(0));
    e.method("Reset", |_c| {
        reset_object_emitters();
        Ok(0)
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use pz_kahlua::{stdlib, TableRef};

    #[test]
    fn ambient_walls_and_object_emitters_lua() {
        reset_session_handles();
        let mut t = Thread::new(TableRef::new());
        stdlib::install(&mut t, None).unwrap();
        register(&mut t);

        let f = stdlib::load_string(
            r#"
            local w = FMODAmbientWalls.getInstance()
            local e = ObjectAmbientEmitters.getInstance()
            w:squareChanged(nil)
            w:update()
            w:render()
            e:update()
            e:render()
            return w == FMODAmbientWalls.getInstance(), e == ObjectAmbientEmitters.getInstance(), FMODAmbientWalls.ENABLE
            "#,
            Some("ambient_audio"),
            t.globals().clone(),
        )
        .unwrap();
        let r = t.pcall(&f, &[]).unwrap();
        assert!(r[0].truthy());
        assert!(r[1].truthy());
        assert!(r[2].truthy());
        STATE.with(|s| {
            let st = s.borrow();
            assert_eq!(st.walls_updates, 1);
            assert_eq!(st.emitters_updates, 1);
        });
        let f2 = stdlib::load_string(
            "ObjectAmbientEmitters.getInstance():Reset()",
            Some("reset"),
            t.globals().clone(),
        )
        .unwrap();
        t.pcall(&f2, &[]).unwrap();
        STATE.with(|s| assert_eq!(s.borrow().emitters_updates, 0));
    }
}
