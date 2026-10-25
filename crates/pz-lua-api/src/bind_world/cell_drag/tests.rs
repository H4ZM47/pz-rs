use super::*;
use crate::events::{EventManager, EVENT_NAMES};

struct Fixture {
    thread: Thread,
    events: EventManager,
    old_cell: Option<IsoCell>,
    old_weather: Option<Rc<RefCell<pz_iso_render::weather_fx::IsoWeatherFx>>>,
    errors: Rc<RefCell<Vec<String>>>,
}

impl Fixture {
    fn new() -> Self {
        let (old_cell, old_weather) = with_world(|w| (w.cell.take(), w.weather_fx.take()));
        set_cell(Some(IsoCell::default()));
        let mut thread = Thread::new(TableRef::new());
        pz_kahlua::stdlib::install(&mut thread, None).unwrap();
        let events = EventManager::register(thread.globals(), "Events", EVENT_NAMES);
        super::super::register_with_events(&mut thread, events.clone());
        thread.globals().set_str("cell", current_cell());
        let errors: Rc<RefCell<Vec<String>>> = Rc::default();
        let out = errors.clone();
        thread.log = Box::new(move |s| out.borrow_mut().push(s.into()));
        let mut f = Self {
            thread,
            events,
            old_cell,
            old_weather,
            errors,
        };
        f.run("function check(v) if not v then error('authored cell drag check failed') end end");
        f
    }

    fn run(&mut self, source: &str) {
        let f = pz_kahlua::stdlib::load_string(
            source,
            Some("cell-drag-test"),
            self.thread.globals().clone(),
        )
        .unwrap();
        self.thread.pcall(&f, &[]).unwrap();
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        set_cell(self.old_cell.take());
        with_world(|w| w.weather_fx = self.old_weather.take());
    }
}

#[test]
fn cell_drag_preserves_actual_slots_event_order_and_same_identity() {
    let mut f = Fixture::new();
    f.run(
        r#"
        for i=-1,4 do check(cell:getDrag(i) == nil) end
        order, events, calls = {}, 0, 0
        Events.SetDragItem.Add(function(value, player)
            events = events + 1
            order[#order+1] = 'event'
            check(cell:getDrag(player) == expectedOld)
            check(value == expectedNew and player == expectedPlayer)
        end)
        expectedOld, expectedNew, expectedPlayer = nil, nil, -1
        cell:setDrag(nil, -1)
        expectedPlayer = 4
        cell:setDrag(nil, 4)
        check(events == 2)
        cursorA, cursorB = {}, {}
        local class = {deactivate=function(self)
            calls = calls + 1
            check(self == cursorA and cell:getDrag(2) == cursorA)
            order[#order+1] = 'deactivate'
        end}
        setmetatable(cursorA, class)
        expectedNew, expectedPlayer = cursorA, 2
        cell:setDrag(cursorA, 2)
        check(cell:getDrag(2) == cursorA and cursorA.player == 2 and cell:getDrag(1) == nil)
        expectedOld = cursorA
        cell:setDrag(cursorA, 2)
        check(events == 4 and calls == 0)
        order = {}
        expectedNew = cursorB
        cell:setDrag(cursorB, 2)
        check(table.concat(order, ',') == 'event,deactivate')
        check(calls == 1 and cell:getDrag(2) == cursorB and cursorB.player == 2)
        expectedOld, expectedNew = cursorB, nil
        cell:setDrag(nil, 2)
        check(cell:getDrag(2) == nil and events == 6)
    "#,
    );
    assert!(f.errors.borrow().is_empty(), "{:?}", f.errors.borrow());
}

#[test]
fn cell_drag_callable_types_errors_and_reentrant_deactivation_match_native_phases() {
    let mut f = Fixture::new();
    let native_calls = Rc::new(Cell::new(0));
    let calls = native_calls.clone();
    f.thread.globals().set_str(
        "nativeDeactivate",
        Value::Native(NativeFunction::new(
            "authored-native-deactivate",
            move |c| {
                assert!(c.arg(0).as_table().is_some());
                calls.set(calls.get() + 1);
                Ok(0)
            },
        )),
    );
    f.run(r#"
        callableTableCalls = 0
        local callable = setmetatable({}, {__call=function() callableTableCalls=callableTableCalls+1 end})
        cell:setDrag({deactivate=callable}, 0)
        cell:setDrag({deactivate=nativeDeactivate}, 0)
        check(callableTableCalls == 0)
        cell:setDrag({deactivate=function() error('authored deactivate failure') end}, 0)
        cell:setDrag({}, 0)
        check(cell:getDrag(0).player == 0)
        deactivations = 0
        local old = {deactivate=function(self)
            deactivations = deactivations + 1
            check(cell:getDrag(0) == self)
            if not nested then nested=true; cell:setDrag({nested=true}, 0) end
        end}
        cell:setDrag(old, 0)
        final = {}
        cell:setDrag(final, 0)
        check(deactivations == 2 and cell:getDrag(0) == final and final.player == 0)
    "#);
    assert_eq!(native_calls.get(), 1);
    assert_eq!(f.errors.borrow().len(), 1);
    assert!(f.errors.borrow()[0].contains("authored deactivate failure"));
}

#[test]
fn cell_drag_reentrant_event_or_deactivate_keeps_original_cell_owner() {
    for replace_in_event in [true, false] {
        let mut f = Fixture::new();
        let original = cell(|c| Ok(c.drag.clone())).unwrap();
        f.thread
            .globals()
            .set_str("replaceInEvent", replace_in_event);
        f.thread.globals().set_str(
            "replaceCurrentCell",
            Value::Native(NativeFunction::new("authored-replace-cell", |_| {
                set_cell(Some(IsoCell::default()));
                Ok(0)
            })),
        );
        f.run(
            r#"
            cell:setDrag({deactivate=function(self)
                if not replaceInEvent then replaceCurrentCell() end
            end}, 1)
            Events.SetDragItem.Add(function(value, player)
                if replaceInEvent then replaceCurrentCell() end
            end)
            replacement = {}
            cell:setDrag(replacement, 1)
            check(replacement.player == 1)
        "#,
        );
        let replacement = f
            .thread
            .globals()
            .get_str("replacement")
            .as_table()
            .unwrap()
            .clone();
        assert!(original.get(1).unwrap().ptr_eq(&replacement));
        assert!(cell(|c| Ok(c.drag.get(1))).unwrap().is_none());
        assert!(f.errors.borrow().is_empty(), "{:?}", f.errors.borrow());
    }
}

struct ReentrantDrop(Rc<Cell<bool>>);
impl LuaObject for ReentrantDrop {
    fn class_name(&self) -> &str {
        "authored.ReentrantCursorDrop"
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}
impl Drop for ReentrantDrop {
    fn drop(&mut self) {
        self.0.set(with_world(|w| {
            w.cell.as_ref().unwrap().drag.get(0).is_none()
        }));
    }
}

#[test]
fn cell_drag_retirement_drops_final_cursor_owners_outside_world_borrow() {
    let _f = Fixture::new();
    let seen = Rc::new(Cell::new(false));
    let table = TableRef::new();
    table.set_str("drop", Value::Object(Rc::new(ReentrantDrop(seen.clone()))));
    cell(|c| {
        c.drag.replace(0, Some(table));
        Ok(())
    })
    .unwrap();
    set_cell(Some(IsoCell::default()));
    assert!(seen.get());
}

#[test]
fn cell_drag_binding_traces_registered_event_without_retaining_closed_environment() {
    let weak = {
        let mut f = Fixture::new();
        f.run("Events.SetDragItem.Add(function() check(cell ~= nil) end)");
        Rc::downgrade(&f.events.add_event("SetDragItem"))
    };
    pz_kahlua::gc::collect_tables();
    assert!(
        weak.upgrade().is_none(),
        "closed cursor binding retained its event/environment cycle"
    );
}
