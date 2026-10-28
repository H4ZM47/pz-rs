//! IsoCell cursor methods. Every Lua phase runs without a world or slot borrow.

use super::*;
use crate::events::{trigger_registered_event, Event};
use pz_kahlua::gc::{TracePayload, TraceVisitor};

struct SetDragCapture {
    event: Rc<RefCell<Event>>,
}

impl TracePayload for SetDragCapture {
    fn trace(&self, visitor: &mut TraceVisitor) {
        visitor.owner(&self.event);
    }
}

pub(super) fn register(b: &ClassBuilder, event: Option<Rc<RefCell<Event>>>) {
    get(b, "getDrag", |c| {
        self_cell(c)?;
        let player = int_arg(c, 1);
        Ok(cell(|cell| Ok(cell.drag.get(player)))?.map_or(Value::Nil, Value::Table))
    });
    // Standalone world exposure can omit event registration. The mutator
    // requires the actual session's SetDragItem owner; it is never a no-op.
    if let Some(event) = event {
        b.methods.set_str(
            "setDrag",
            Value::Native(NativeFunction::new_traced_capture(
                "setDrag",
                SetDragCapture { event },
                set_drag,
            )),
        );
    }
}

fn set_drag(capture: &RefCell<SetDragCapture>, c: &mut NativeCall<'_>) -> LuaResult<usize> {
    self_cell(c)?;
    let table = match c.arg(1) {
        Value::Nil => None,
        Value::Table(table) => Some(table),
        _ => return Err(wrong(c, 1, "se.krka.kahlua.vm.KahluaTable")),
    };
    let player = int_arg(c, 2);
    // Pin the receiver's actual owner before Lua can replace the current
    // cell. Original Java `this.drag` keeps referring to that same array.
    let drag = cell(|cell| Ok(cell.drag.clone()))?;
    let event = capture.borrow().event.clone();
    let value = table.clone().map_or(Value::Nil, Value::Table);
    trigger_registered_event(c.thread, &event, &[value, player.into()]);
    if !(0..4).contains(&player) {
        return Ok(0);
    }
    // Read only after the event: a reentrant handler may have changed it.
    let old = drag.get(player);
    if match (&old, &table) {
        (Some(a), Some(b)) => a.ptr_eq(b),
        (None, None) => true,
        _ => false,
    } {
        return Ok(0);
    }
    if let Some(old) = old {
        let deactivate = old.get_str("deactivate");
        // The original accepts JavaFunction/LuaClosure, not a table's __call.
        if matches!(deactivate, Value::Native(_) | Value::Closure(_)) {
            if let Err((error, trace)) = c.thread.pcall(&deactivate, &[Value::Table(old)]) {
                (c.thread.log)(&format!(
                    "{}\n{trace}",
                    pz_kahlua::exec::java_value_of(&error)
                ));
            }
        }
    }
    // The outer assignment follows even if deactivate recursively set a
    // different cursor or failed. Drop the displaced table outside borrows.
    let retired = drag.replace(player, table.clone());
    drop(retired);
    if let Some(table) = table {
        table.set_str("player", player);
    }
    Ok(0)
}

#[cfg(test)]
mod tests;
