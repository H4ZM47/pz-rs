//! Reads of world objects that `ISWorldObjectContextMenuLogic` makes and
//! that no other binding answers yet.

use pz_kahlua::{Thread, Value};

use crate::bind_world_object::{object_from_value, object_square, ISO_OBJECT};
use crate::expose::expose_class;

pub(super) fn register(thread: &mut Thread) {
    let object = expose_class(thread, ISO_OBJECT, None);
    // getSquare(): `square`.
    object.method("getSquare", |c| {
        let this = c.arg(0);
        let object = object_from_value(&this).ok_or_else(|| receiver_error(&this))?;
        let square = object_square(object)?;
        Ok(c.push(square))
    });
}

fn receiver_error(v: &Value) -> pz_kahlua::LuaError {
    crate::bind_java_util::class_cast(v, ISO_OBJECT)
}

