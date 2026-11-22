//! `RecipeCodeOnTest`: static recipe test callbacks that Lua calls
//! directly. Implemented: `cutFish`.

use pz_inventory::ItemKind;
use pz_kahlua::{LuaResult, NativeCall, Thread, Value};

const CLASS: &str = "zombie.scripting.logic.RecipeCodeOnTest";
const SUPERCLASS: &str = "zombie.scripting.logic.RecipeCodeHelper";

/// `cutFish(item, character)`: a `Food` can be cut when its actual weight
/// is above 0.6; any other item (or null) can. The character is not used.
fn cut_fish(c: &mut NativeCall<'_>) -> LuaResult<usize> {
    let weight = match c.arg(0) {
        Value::Nil => None,
        _ => {
            let item = super::item_arg(c, 0)?;
            let env = super::env();
            let body = item.borrow();
            matches!(body.kind, ItemKind::Food(_))
                .then(|| body.get_actual_weight_with(env.context()))
        }
    };
    // `fcmpl` then `ifle`: NaN is not at most 0.6.
    let can_cut = weight.is_none_or(|w| !(w <= 0.6));
    Ok(c.push(Value::from(can_cut)))
}

pub(super) fn register(thread: &mut Thread) {
    let class = crate::expose::expose_class(thread, CLASS, Some(SUPERCLASS));
    class.static_fn("cutFish", cut_fish);
}
