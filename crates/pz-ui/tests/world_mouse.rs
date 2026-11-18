//! `UIManager.update`'s world part of the mouse: the context pick each
//! frame, and what the right button does with the picked object. The
//! world's end is a recording [`WorldMouse`].

use pz_kahlua::{TableRef, Thread, Value};
use pz_ui::{with_ui, PickedObject, UIElement, UIManager, WorldMouse};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

#[derive(Default)]
struct World {
    /// What `ContextPick` finds.
    pick: RefCell<Option<PickedObject>>,
    /// `getObjectIndex()` of every object.
    index: Cell<i32>,
    cell: Cell<bool>,
    player: Cell<Option<(bool, bool)>>,
    paused: Cell<bool>,
    calls: RefCell<Vec<String>>,
}

impl World {
    fn new() -> Rc<World> {
        let w = World::default();
        w.index.set(3);
        w.cell.set(true);
        w.player.set(Some((false, false)));
        Rc::new(w)
    }
    fn call(&self, s: impl Into<String>) {
        self.calls.borrow_mut().push(s.into());
    }
    fn take(&self) -> Vec<String> {
        std::mem::take(&mut *self.calls.borrow_mut())
    }
}

impl WorldMouse for World {
    fn has_player0(&self) -> bool {
        self.player.get().is_some()
    }
    fn context_pick(&self, _: &mut Thread, mx: i32, my: i32) -> pz_kahlua::LuaResult<Option<PickedObject>> {
        self.call(format!("pick {mx} {my}"));
        Ok(self.pick.borrow().clone())
    }
    fn object_index(&self, _: &Value) -> i32 {
        self.index.get()
    }
    fn has_cell(&self) -> bool {
        self.cell.get()
    }
    fn player_state(&self) -> Option<(bool, bool)> {
        self.player.get()
    }
    fn is_game_paused(&self) -> bool {
        self.paused.get()
    }
    fn set_current_game_speed(&self, speed: i32) {
        self.call(format!("speed {speed}"));
    }
    fn on_mouse_right_click(&self, _: &mut Thread, _: &Value, lx: i32, ly: i32) -> pz_kahlua::LuaResult<()> {
        self.call(format!("rightClick {lx} {ly}"));
        Ok(())
    }
    fn on_mouse_right_released(&self, _: &mut Thread, _: &Value) -> pz_kahlua::LuaResult<()> {
        self.call("rightReleased");
        Ok(())
    }
    fn clear_drag_object(&self) {
        self.call("dragCleared");
    }
}

/// An event as the sink saw it: name and arguments (numbers, or `tile`
/// for the picked object's table).
type Events = Rc<RefCell<Vec<String>>>;

fn describe(v: &Value, tile: &TableRef) -> String {
    match v {
        Value::Num(n) => format!("{n}"),
        Value::Table(t) if t.ptr_eq(tile) => "tile".into(),
        _ => "?".into(),
    }
}

struct Setup {
    t: Thread,
    world: Rc<World>,
    events: Events,
    tile: TableRef,
}

fn setup() -> Setup {
    UIManager::reset();
    let mut t = Thread::new(TableRef::new());
    pz_kahlua::stdlib::install(&mut t, None).unwrap();
    let world = World::new();
    let tile = TableRef::new();
    *world.pick.borrow_mut() = Some(PickedObject {
        tile: Value::Table(tile.clone()),
        lx: 17,
        ly: 23,
    });
    let events: Events = Rc::default();
    let (log, tile2) = (events.clone(), tile.clone());
    UIManager::set_event_sink(Some(Rc::new(move |_, name, args| {
        let args: Vec<String> = args.iter().map(|a| describe(a, &tile2)).collect();
        log.borrow_mut().push(format!("{name}({})", args.join(", ")));
    })));
    with_ui(|m| {
        m.world_mouse = Some(world.clone());
        m.speed_controls = Some(UIElement::new(None));
        m.mouse.x = 640;
        m.mouse.y = 360;
        m.mouse.world_x = 600;
        m.mouse.world_y = 340;
    });
    Setup { t, world, events, tile }
}

/// One frame with the right button in `down`.
fn frame(s: &mut Setup, right_down: bool) {
    with_ui(|m| m.mouse.down[1] = right_down);
    UIManager::update(&mut s.t).unwrap();
    with_ui(|m| m.mouse.end_frame());
}

fn names(s: &Setup) -> Vec<String> {
    std::mem::take(&mut *s.events.borrow_mut())
}

#[test]
fn picks_under_the_mouse_each_frame_and_reports_the_move() {
    let mut s = setup();
    frame(&mut s, false);
    assert_eq!(s.world.take(), ["pick 640 360"]);
    assert_eq!(names(&s), ["OnMouseMove(640, 360, 600, 340)"]);
    // Every frame, not only when the mouse moved.
    frame(&mut s, false);
    assert_eq!(s.world.take(), ["pick 640 360"]);
    let picked = with_ui(|m| m.picked.clone()).unwrap();
    assert_eq!((picked.lx, picked.ly), (17, 23));
    // Nothing under the mouse clears the pick.
    *s.world.pick.borrow_mut() = None;
    frame(&mut s, false);
    assert!(with_ui(|m| m.picked.is_none()));
}

#[test]
fn nothing_is_picked_without_a_player_or_a_world_host() {
    let mut s = setup();
    s.world.player.set(None);
    frame(&mut s, false);
    assert!(s.world.take().is_empty());
    assert!(names(&s).is_empty());
    with_ui(|m| m.world_mouse = None);
    frame(&mut s, false);
    assert!(with_ui(|m| m.picked.is_none()));
}

/// Frame `A` with nothing pressed, so the pick is in place, then the
/// returned events/calls are cleared.
fn warmed_up() -> Setup {
    let mut s = setup();
    frame(&mut s, false);
    s.world.take();
    names(&s);
    s
}

#[test]
fn right_press_fires_the_object_event_and_starts_the_click() {
    let mut s = warmed_up();
    frame(&mut s, true);
    assert_eq!(
        names(&s),
        [
            "OnRightMouseDown(640, 360)",
            "OnObjectRightMouseButtonDown(tile, 640, 360)",
            "OnMouseMove(640, 360, 600, 340)",
        ]
    );
    // Normal speed, then the click with the point inside the object.
    assert_eq!(s.world.take(), ["speed 1", "rightClick 17 23", "pick 640 360"]);
    assert!(with_ui(|m| m.right_down_object.is_some()));
}

#[test]
fn right_release_fires_the_object_event_with_the_same_tile() {
    let mut s = warmed_up();
    frame(&mut s, true);
    frame(&mut s, true);
    names(&s);
    s.world.take();
    let down = with_ui(|m| m.right_down_object.clone()).unwrap();
    frame(&mut s, false);
    assert_eq!(
        names(&s),
        [
            "OnRightMouseUp(640, 360)",
            "OnObjectRightMouseButtonUp(tile, 640, 360)",
            "OnMouseMove(640, 360, 600, 340)",
        ]
    );
    assert_eq!(s.world.take(), ["dragCleared", "rightReleased", "pick 640 360"]);
    assert!(with_ui(|m| m.right_down_object.is_none()));
    // The object that went down is the one the release reports.
    let (Value::Table(a), Value::Table(b)) = (down, Value::Table(s.tile.clone())) else { panic!() };
    assert!(a.ptr_eq(&b));
}

#[test]
fn held_right_button_keeps_the_key_pressed() {
    let mut s = warmed_up();
    frame(&mut s, true);
    names(&s);
    frame(&mut s, true);
    assert_eq!(names(&s)[0], "OnKeyKeepPressed(10001)");
}

#[test]
fn an_object_off_its_square_gets_no_object_event() {
    let mut s = warmed_up();
    s.world.index.set(-1);
    frame(&mut s, true);
    assert_eq!(names(&s), ["OnRightMouseDown(640, 360)", "OnMouseMove(640, 360, 600, 340)"]);
    // The click itself does not look at the index.
    assert_eq!(s.world.take(), ["speed 1", "rightClick 17 23", "pick 640 360"]);
}

#[test]
fn the_click_waits_for_a_player_who_is_up_and_a_running_game() {
    for (state, paused, cell) in [
        (Some((true, false)), false, true),
        (Some((false, true)), false, true),
        (Some((false, false)), true, true),
        (Some((false, false)), false, false),
        (None, false, true),
    ] {
        let mut s = warmed_up();
        s.world.player.set(state);
        s.world.paused.set(paused);
        s.world.cell.set(cell);
        frame(&mut s, true);
        let calls = s.world.take();
        assert!(!calls.iter().any(|c| c.starts_with("speed") || c.starts_with("rightClick")), "{state:?} {paused} {cell}: {calls:?}");
        assert!(with_ui(|m| m.right_down_object.is_none()));
        // The Lua events do not depend on it.
        assert!(names(&s).contains(&"OnObjectRightMouseButtonDown(tile, 640, 360)".to_string()));
    }
}

#[test]
fn a_release_without_a_press_to_finish_only_ends_the_drag() {
    let mut s = warmed_up();
    s.world.player.set(Some((true, false)));
    with_ui(|m| m.right_down_object = Some(Value::Num(1.0)));
    frame(&mut s, true);
    s.world.take();
    frame(&mut s, false);
    // Aiming: the dragged object ends, `onMouseRightReleased` waits.
    assert_eq!(s.world.take(), ["dragCleared", "pick 640 360"]);
    assert!(with_ui(|m| m.right_down_object.is_some()));
}
