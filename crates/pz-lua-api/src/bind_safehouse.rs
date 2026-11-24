//! `zombie.iso.areas.SafeHouse`: the static checks the world context menu
//! makes, over `SafeHouse.safehouseList` as the engine loaded it from
//! `map_meta.bin` ([`install_safehouses`]).
//!
//! A SafeHouse has no Lua object yet, so a check that finds one on the
//! square stops with `unsupported`; with no safehouse there (always in a
//! new single-player game) the checks answer as the original does.

use std::cell::RefCell;

use pz_kahlua::{LuaError, LuaResult, Thread, Value};

use crate::bind_world_object::unsupported;
use crate::expose::expose_class;

const CLASS: &str = "zombie.iso.areas.SafeHouse";

/// A safehouse rectangle: `x`, `y`, `w`, `h`.
pub type SafeHouseRect = [i32; 4];

thread_local! {
    static SAFEHOUSES: RefCell<Vec<SafeHouseRect>> = const { RefCell::new(Vec::new()) };
}

/// Replace the safehouse list (`SafeHouse.safehouseList`).
pub fn install_safehouses(list: Vec<SafeHouseRect>) {
    SAFEHOUSES.with(|s| *s.borrow_mut() = list);
}

/// Whether world tile `(x, y)` lies inside any registered safehouse.
pub fn is_in_safehouse(x: i32, y: i32) -> bool {
    SAFEHOUSES.with(|list| {
        list.borrow()
            .iter()
            .any(|&[sx, sy, w, h]| x >= sx && x < sx + w && y >= sy && y < sy + h)
    })
}

pub(crate) fn register(thread: &mut Thread) {
    let class = expose_class(thread, CLASS, Some("java.lang.Object"));
    // getSafeHouse(IsoGridSquare): isSafeHouse(square, null, false), which
    // in single player is findSafeHouse(square).
    class.static_fn("getSafeHouse", |c| {
        let square = c.arg(0);
        if !square.is_nil() && !is_square(&square) {
            return Err(unsupported("SafeHouse.getSafeHouse(String) and getSafeHouse(int, int, int, int)"));
        }
        let found = find(c.thread, &square)?;
        Ok(c.push(safehouse_value(found)?))
    });
    class.static_fn("isSafehouseAllowInteract", |c| {
        let (square, player) = (c.arg(0), c.arg(1));
        let r = allow_interact(c.thread, &square, &player)?;
        Ok(c.push(r))
    });
    // isSafehouseAllowLoot(square, player): the SafehouseAllowLoot server
    // option, else isSafehouseAllowInteract.
    class.static_fn("isSafehouseAllowLoot", |c| {
        let (square, player) = (c.arg(0), c.arg(1));
        let r = server_option(c.thread, "SafehouseAllowLoot")? || allow_interact(c.thread, &square, &player)?;
        Ok(c.push(r))
    });
    // isSafehouseAllowClaimWar(safehouse, player): false without the War
    // server option or without a safehouse or a player.
    class.static_fn("isSafehouseAllowClaimWar", |c| {
        if !server_option(c.thread, "War")? || c.arg(0).is_nil() || c.arg(1).is_nil() {
            return Ok(c.push(false));
        }
        Err(unsupported("SafeHouse.isSafehouseAllowClaimWar: WarManager"))
    });
}

fn is_square(v: &Value) -> bool {
    pz_kahlua::stdlib::java_class_name(v) == "zombie.iso.IsoGridSquare"
}

/// isSafehouseAllowInteract(square, player): true without a square, a
/// player or a safehouse on the square.
fn allow_interact(t: &mut Thread, square: &Value, player: &Value) -> LuaResult<bool> {
    if square.is_nil() || player.is_nil() {
        return Ok(true);
    }
    match find(t, square)? {
        None => Ok(true),
        Some(_) => Err(unsupported("SafeHouse.isSafehouseAllowInteract: a SafeHouse has no Lua object")),
    }
}

/// `findSafeHouse(square)`: the first safehouse whose rectangle holds the
/// square.
fn find(t: &mut Thread, square: &Value) -> LuaResult<Option<SafeHouseRect>> {
    if square.is_nil() {
        return Ok(None);
    }
    if SAFEHOUSES.with(|s| s.borrow().is_empty()) {
        return Ok(None);
    }
    let x = method_num(t, square, "getX")? as i32;
    let y = method_num(t, square, "getY")? as i32;
    Ok(SAFEHOUSES.with(|s| {
        s.borrow()
            .iter()
            .copied()
            .find(|&[sx, sy, w, h]| x >= sx && x < sx + w && y >= sy && y < sy + h)
    }))
}

fn safehouse_value(found: Option<SafeHouseRect>) -> LuaResult<Value> {
    match found {
        None => Ok(Value::Nil),
        Some(_) => Err(unsupported("SafeHouse.getSafeHouse: a SafeHouse has no Lua object")),
    }
}

fn method_num(t: &mut Thread, obj: &Value, name: &str) -> LuaResult<f64> {
    let f = t.table_get(obj, &Value::str(name))?;
    Ok(t.call1(&f, &[obj.clone()])?.as_num().unwrap_or(0.0))
}

/// `ServerOptions.getInstance().<name>.getValue()`.
fn server_option(t: &mut Thread, name: &str) -> LuaResult<bool> {
    let get = t.globals().rawget(&Value::str("getServerOptions"));
    if get.is_nil() {
        return Err(LuaError::runtime("SafeHouse: getServerOptions is not bound"));
    }
    let options = t.call1(&get, &[])?;
    let f = t.table_get(&options, &Value::str("getBoolean"))?;
    Ok(t.call1(&f, &[options, Value::str(name)])?.truthy())
}

#[cfg(test)]
mod tests {
    use super::*;
    use pz_kahlua::TableRef;

    #[test]
    fn checks_answer_for_a_square_without_a_safehouse() {
        let mut t = Thread::new(TableRef::new());
        pz_kahlua::stdlib::install(&mut t, None).unwrap();
        register(&mut t);
        install_safehouses(Vec::new());
        let f = pz_kahlua::stdlib::load_string(
            "return SafeHouse.getSafeHouse(nil), SafeHouse.isSafehouseAllowInteract(nil, nil)",
            Some("test"),
            t.globals().clone(),
        )
        .unwrap();
        let r = t.call(&f, &[]).unwrap();
        assert!(r[0].is_nil());
        assert!(matches!(r[1], Value::Bool(true)));
    }
}
