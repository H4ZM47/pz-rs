use super::*;
use pz_kahlua::{NativeFunction, Value};

#[test]
fn real_cell_slots_keep_identity_and_are_independent_of_replacement_cells() {
    let cell = crate::IsoCell::default();
    let pinned = cell.drag.clone();
    for slot in -1..=4 {
        assert!(pinned.get(slot).is_none());
    }
    let table = TableRef::new();
    assert!(pinned.replace(2, Some(table.clone())).is_none());
    assert!(pinned.get(2).unwrap().ptr_eq(&table));
    assert!(pinned.get(1).is_none());
    drop(cell);
    let replacement = crate::IsoCell::default();
    assert!(replacement.drag.get(2).is_none());
    assert!(pinned.get(2).unwrap().ptr_eq(&table));
    let retired = pinned.replace(2, None).unwrap();
    assert!(retired.ptr_eq(&table) && pinned.get(2).is_none());
}

struct Capture(CellDrag);
impl TracePayload for Capture {
    fn trace(&self, visitor: &mut TraceVisitor) {
        visitor.owner(&self.0.slots);
    }
}

#[test]
fn traced_cursor_cycle_survives_live_cell_then_retires_after_its_owner() {
    let drag = CellDrag::default();
    let weak = Rc::downgrade(&drag.slots);
    let table = TableRef::new();
    table.set_str("marker", 7);
    table.set_str(
        "owner",
        Value::Native(NativeFunction::new_traced_capture(
            "authored-cell-drag-owner",
            Capture(drag.clone()),
            |_, _| Ok(0),
        )),
    );
    drag.replace(0, Some(table));
    pz_kahlua::gc::collect_tables();
    assert_eq!(drag.get(0).unwrap().get_str("marker").as_num(), Some(7.0));
    drop(drag);
    pz_kahlua::gc::collect_tables();
    assert!(weak.upgrade().is_none());
}
