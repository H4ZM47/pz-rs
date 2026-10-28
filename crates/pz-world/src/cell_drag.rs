//! The four Lua cursor slots owned by an `IsoCell`. The backing allocation
//! stays stable when the cell moves and can be pinned across Lua callbacks.

use pz_kahlua::gc::{traceable, TracePayload, TraceVisitor};
use pz_kahlua::TableRef;
use std::any::Any;
use std::cell::RefCell;
use std::rc::Rc;

#[derive(Default)]
struct Slots {
    tables: [Option<TableRef>; 4],
}

impl TracePayload for Slots {
    fn trace(&self, visitor: &mut TraceVisitor) {
        for table in self.tables.iter().flatten() {
            visitor.table(table);
        }
    }

    fn detach(&mut self) -> Option<Box<dyn Any>> {
        Some(Box::new(std::mem::take(&mut self.tables)))
    }
}

/// Cloning pins the same cell's real slots; constructing a new cell creates
/// four independent null slots, as `IsoCell(int,int)` does.
#[derive(Clone)]
pub struct CellDrag {
    slots: Rc<RefCell<Slots>>,
}

impl Default for CellDrag {
    fn default() -> Self {
        Self {
            slots: traceable(Slots::default()),
        }
    }
}

impl CellDrag {
    pub fn get(&self, player: i32) -> Option<TableRef> {
        if !(0..4).contains(&player) {
            return None;
        }
        self.slots.borrow().tables[player as usize].clone()
    }

    /// The binding supplies original event/deactivate/player-field phases.
    /// Return the replaced table so its final drop happens outside borrows.
    pub fn replace(&self, player: i32, table: Option<TableRef>) -> Option<TableRef> {
        if !(0..4).contains(&player) {
            return None;
        }
        let retired =
            std::mem::replace(&mut self.slots.borrow_mut().tables[player as usize], table);
        retired
    }
}

#[cfg(test)]
mod tests;
