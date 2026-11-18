//! The `do*` helpers of `ISWorldObjectContextMenuLogic` that
//! `createMenuEntries` reaches for a click on the floor or on a bed. Each
//! one checks its `fetch` slot first, as the original does; the parts for
//! objects that this port does not handle yet stop with `unsupported`.

use pz_kahlua::{LuaResult, TableRef, Value};

use super::{append, contains_key, Cx};
use crate::bind_globals::instance_of;
use crate::bind_world_object::unsupported;

fn todo(name: &str) -> pz_kahlua::LuaError {
    unsupported(format!("ISWorldObjectContextMenuLogic.{name}"))
}

/// A new submenu for `option` of `menu` (`getNew` + `addSubMenu`).
pub(super) fn sub_menu(cx: &mut Cx<'_>, menu: &Value, option: &Value) -> LuaResult<Value> {
    let sub = cx.call(menu, "getNew", &[menu.clone()])?;
    cx.call(menu, "addSubMenu", &[option.clone(), sub.clone()])?;
    Ok(sub)
}

/// `menu.getNumOptions() <= 1`: the submenu stayed empty.
pub(super) fn is_empty_menu(cx: &mut Cx<'_>, menu: &Value) -> LuaResult<bool> {
    let n = match menu {
        Value::Table(t) => t.get_str("numOptions").as_num().unwrap_or(0.0),
        _ => return Err(super::null_pointer()),
    };
    let _ = cx;
    Ok(n <= 1.0)
}

/// `addToolTip()`: `ISWorldObjectContextMenu.addToolTip` takes a tooltip
/// from the pool, as the Java helper does.
pub(super) fn add_tool_tip(cx: &mut Cx<'_>) -> LuaResult<Value> {
    let f = cx.func("ISWorldObjectContextMenu.addToolTip");
    cx.t.call1(&f, &[])
}

/// `option.toolTip = tooltip` with `description` set.
pub(super) fn set_tool_tip(cx: &mut Cx<'_>, option: &Value, name: Option<Value>, description: Value) -> LuaResult<()> {
    let tooltip = add_tool_tip(cx)?;
    if let Some(name) = name {
        cx.call(&tooltip, "setName", &[name])?;
    }
    let t = tooltip.as_table().ok_or_else(super::null_pointer)?;
    t.set_str("description", description);
    option.as_table().ok_or_else(super::null_pointer)?.set_str("toolTip", tooltip);
    Ok(())
}

fn set_not_available(option: &Value) -> LuaResult<()> {
    option.as_table().ok_or_else(super::null_pointer)?.set_str("notAvailable", true);
    Ok(())
}

/// `LuaHelpers.getJoypadState(player)`.
pub(super) fn joypad_state(cx: &mut Cx<'_>, player: f64) -> Value {
    match cx.t.globals().rawget(&Value::str("JoypadState")) {
        Value::Table(t) => match t.get_str("players") {
            Value::Table(p) => p.rawget(&Value::Num(player + 1.0)),
            _ => Value::Nil,
        },
        _ => Value::Nil,
    }
}

/// `getAllObjects(worldobjects)`: the clicked objects plus every object on
/// their squares, once each.
pub(super) fn get_all_objects(cx: &mut Cx<'_>, worldobjects: &TableRef) -> LuaResult<TableRef> {
    let result = TableRef::new();
    let seen_squares = TableRef::new();
    let seen = TableRef::new();
    for (_, object) in worldobjects.entries() {
        if seen.rawget(&object).truthy() {
            continue;
        }
        let square = cx.call(&object, "getSquare", &[])?;
        get_all_object_on_square(cx, &square, &seen_squares, &seen, &result)?;
        if !seen.rawget(&object).truthy() {
            seen.rawset(object.clone(), Value::Bool(true));
            append(&result, object);
        }
    }
    Ok(result)
}

/// `getAllObjectOnSquare(square, squares, objects, result)`.
fn get_all_object_on_square(
    cx: &mut Cx<'_>,
    square: &Value,
    seen_squares: &TableRef,
    seen: &TableRef,
    result: &TableRef,
) -> LuaResult<()> {
    if square.is_nil() || seen_squares.rawget(square).truthy() {
        return Ok(());
    }
    seen_squares.rawset(square.clone(), Value::Bool(true));
    let objects = cx.call(square, "getObjects", &[])?;
    let n = cx.num(&objects, "size", &[])? as i64;
    for i in 0..n {
        let object = cx.call(&objects, "get", &[Value::Num(i as f64)])?;
        if !seen.rawget(&object).truthy() {
            seen.rawset(object.clone(), Value::Bool(true));
            append(result, object);
        }
    }
    Ok(())
}

/// `getSquaresInRadius(x, y, z, radius, seen, squares)`.
fn get_squares_in_radius(cx: &mut Cx<'_>, x: f32, y: f32, z: f32, radius: f32, seen: &TableRef, squares: &TableRef) -> LuaResult<()> {
    let (x0, x1) = ((x - radius).floor() as i32, (x + radius).ceil() as i32);
    let (y0, y1) = ((y - radius).floor() as i32, (y + radius).ceil() as i32);
    let cell = cx.global("getCell", &[])?;
    for sy in y0..=y1 {
        for sx in x0..=x1 {
            let square =
                cx.call(&cell, "getGridSquare", &[Value::Num(sx as f64), Value::Num(sy as f64), Value::Num(z as f64)])?;
            if !square.is_nil() && !contains_key(seen, &square) {
                seen.rawset(square.clone(), Value::Bool(true));
                append(squares, square);
            }
        }
    }
    Ok(())
}

/// `getWorldObjectsInRadius(player, x, y, squares, radius, result)`: the
/// world items whose screen position is within 48 / zoom pixels of (x, y).
fn get_world_objects_in_radius(cx: &mut Cx<'_>, player: i32, x: i32, y: i32, squares: &TableRef, result: &TableRef) -> LuaResult<()> {
    let core = cx.global("getCore", &[])?;
    let zoom = cx.num(&core, "getZoom", &[Value::Num(player as f64)])? as f32;
    let radius = 48.0 / zoom;
    for (_, square) in squares.entries() {
        if square.is_nil() {
            continue;
        }
        let items = cx.call(&square, "getWorldObjects", &[])?;
        let n = cx.num(&items, "size", &[])? as i64;
        for i in 0..n {
            let item = cx.call(&items, "get", &[Value::Num(i as f64)])?;
            let sx = cx.num(&item, "getScreenPosX", &[Value::Num(player as f64)])? as f32;
            let sy = cx.num(&item, "getScreenPosY", &[Value::Num(player as f64)])? as f32;
            let (dx, dy) = (x as f32 - sx, y as f32 - sy);
            if dx * dx + dy * dy <= radius * radius {
                append(result, item);
            }
        }
    }
    Ok(())
}

/// The squares of `worldobjects` (once each) and then, without a joypad,
/// the squares within one tile of the click on each of their levels.
fn clicked_squares(cx: &mut Cx<'_>, worldobjects: &TableRef) -> LuaResult<(TableRef, TableRef)> {
    let squares = TableRef::new();
    let seen = TableRef::new();
    for (_, object) in worldobjects.entries() {
        if object.is_nil() {
            continue;
        }
        let square = cx.call(&object, "getSquare", &[])?;
        if square.is_nil() || contains_key(&seen, &square) {
            continue;
        }
        seen.rawset(square.clone(), Value::Bool(true));
        append(&squares, square);
    }
    Ok((squares, seen))
}

/// The world items near the click: on a joypad every item on the clicked
/// squares, else those within the screen radius of the squares around.
fn world_items_near(cx: &mut Cx<'_>, squares: &TableRef, seen: &TableRef, player: i32, x: i32, y: i32) -> LuaResult<TableRef> {
    let items = TableRef::new();
    if !joypad_state(cx, player as f64).is_nil() {
        for (_, square) in squares.entries() {
            let list = cx.call(&square, "getWorldObjects", &[])?;
            let n = cx.num(&list, "size", &[])? as i64;
            for i in 0..n {
                append(&items, cx.call(&list, "get", &[Value::Num(i as f64)])?);
            }
        }
        return Ok(items);
    }
    let copy = TableRef::new();
    for (k, v) in squares.entries() {
        copy.rawset(k, v);
    }
    for (_, square) in copy.entries() {
        if square.is_nil() {
            continue;
        }
        let z = cx.num(&square, "getZ", &[])? as f32;
        let iso_x = cx.global("screenToIsoX", &[Value::Num(player as f64), Value::Num(x as f64), Value::Num(y as f64), Value::Num(z as f64)])?;
        let iso_y = cx.global("screenToIsoY", &[Value::Num(player as f64), Value::Num(x as f64), Value::Num(y as f64), Value::Num(z as f64)])?;
        let (ix, iy) = (iso_x.as_num().unwrap_or(0.0) as f32, iso_y.as_num().unwrap_or(0.0) as f32);
        get_squares_in_radius(cx, ix, iy, z, 1.0, seen, squares)?;
    }
    get_world_objects_in_radius(cx, player, x, y, squares, &items)?;
    Ok(items)
}

/// `handleInteraction(x, y, test, context, worldobjects, player, inventory)`:
/// the device options of the radios lying near the click.
pub(super) fn handle_interaction(cx: &mut Cx<'_>, x: i32, y: i32, test: bool, context: &Value, worldobjects: &TableRef, player_obj: &Value) -> LuaResult<bool> {
    let core = cx.global("getCore", &[])?;
    if cx.call(&core, "getGameMode", &[])?.as_str() == Some("LastStand") {
        return Ok(false);
    }
    if test {
        return Ok(true);
    }
    let player = cx.num(player_obj, "getIndex", &[])? as i32;
    let (squares, seen) = clicked_squares(cx, worldobjects)?;
    if squares.is_empty() {
        return Ok(false);
    }
    let items = world_items_near(cx, &squares, &seen, player, x, y)?;
    if items.is_empty() {
        return Ok(false);
    }
    // Group by name; only a group whose first item is a radio counts.
    let groups = TableRef::new();
    for (_, object) in items.entries() {
        if object.is_nil() {
            continue;
        }
        let mut name = cx.call(&object, "getName", &[])?;
        if name.is_nil() && instance_of(&object, "IItemProvider") {
            let item = cx.call(&object, "getItem", &[])?;
            name = cx.call(&item, "getName", &[])?;
        }
        if name.is_nil() {
            name = Value::str("???");
        }
        let group = match groups.rawget(&name) {
            Value::Table(t) => t,
            _ => {
                let t = TableRef::new();
                groups.rawset(name, Value::Table(t.clone()));
                t
            }
        };
        append(&group, object);
    }
    for (_, group) in groups.entries() {
        let Value::Table(group) = group else { continue };
        let first = group.rawget(&Value::Num(1.0));
        let item = if instance_of(&first, "IItemProvider") { cx.call(&first, "getItem", &[])? } else { Value::Nil };
        if instance_of(&item, "Radio") {
            let _ = context;
            return Err(todo("handleInteraction (a radio item on the ground)"));
        }
    }
    Ok(false)
}

/// `handleGrabWorldItem(fetch, x, y, test, context, worldobjects, player,
/// inventory)`: the Grab options for the items and the corpse near the
/// click.
pub(super) fn handle_grab_world_item(cx: &mut Cx<'_>, fetch: &TableRef, x: i32, y: i32, test: bool, context: &Value, worldobjects: &TableRef, player_obj: &Value, inventory: &Value) -> LuaResult<bool> {
    if !cx.call(player_obj, "getVehicle", &[])?.is_nil() {
        return Ok(false);
    }
    let core = cx.global("getCore", &[])?;
    if cx.call(&core, "getGameMode", &[])?.as_str() == Some("LastStand") {
        return Ok(false);
    }
    if test {
        return Ok(true);
    }
    let player = cx.num(player_obj, "getIndex", &[])? as i32;
    if cx.is(player_obj, "isGrappling", &[])? {
        let text = cx.text("ContextMenu_Drop_Corpse")?;
        let f = cx.func("ISWorldObjectContextMenu.handleGrabWorldItem_onDropCorpse");
        let option = cx.option(context, "addOption", text, player_obj.clone(), f, &[])?;
        let description = cx.text("Tooltip_GrappleCorpse")?;
        set_tool_tip(cx, &option, None, description)?;
    }
    let (squares, seen) = clicked_squares(cx, worldobjects)?;
    if squares.is_empty() {
        return Ok(false);
    }
    let items = world_items_near(cx, &squares, &seen, player, x, y)?;
    let body = fetch.get_str("body");
    let grab_body = !body.is_nil()
        && !cx.is(&body, "isAnimal", &[])?
        && cx.call(player_obj, "getVehicle", &[])?.is_nil()
        && cx.num(inventory, "getItemCount", &[Value::str("Base.CorpseMale")])? == 0.0;
    if items.is_empty() && !grab_body {
        return Ok(false);
    }
    Err(todo("handleGrabWorldItem (items or a corpse near the click)"))
}

/// `doWindowMenu(fetch, worldobjects, test, context, window,
/// invincibleWindow, player, hasRemoveBarricadeTool, hasHammer)`: the
/// Window (or escape rope) submenu.
#[allow(clippy::too_many_arguments)]
pub(super) fn do_window_menu(cx: &mut Cx<'_>, fetch: &TableRef, worldobjects: &TableRef, test: bool, context: &Value, window: &Value, invincible: bool, player_obj: &Value) -> LuaResult<Option<Value>> {
    let window_frame = fetch.get_str("windowFrame");
    let thumpable_window = fetch.get_str("thumpableWindow");
    let key = if !window.is_nil() || !window_frame.is_nil() || !thumpable_window.is_nil() {
        "ContextMenu_Window"
    } else {
        "ContextMenu_escape_rope"
    };
    let text = cx.text(key)?;
    let option = cx.option(context, "addOption", text, Value::Nil, Value::Nil, &[])?;
    let sub = sub_menu(cx, context, &option)?;
    let curtain = fetch.get_str("curtain");
    if !window.is_nil() && !invincible {
        return Err(todo("doWindowMenu (a window)"));
    }
    for key in [
        "window",
        "hoppableN",
        "hoppableN_2",
        "hoppableW",
        "hoppableW_2",
        "thumpableWindowN",
        "thumpableWindowN_2",
        "thumpableWindowW",
        "thumpableWindowW_2",
    ] {
        let object = fetch.get_str(key);
        if object.is_nil() {
            continue;
        }
        if instance_of(&object, "IsoObject") && cx.sprite_has(&object, "makeWindowInvincible")? {
            continue;
        }
        let _ = worldobjects;
        return Err(todo("doWindowMenu (sheet rope options for a window or hoppable)"));
    }
    let mut curtain = curtain;
    if curtain.is_nil() && !window_frame.is_nil() {
        curtain = cx.call(&window_frame, "HasCurtains", &[])?;
    }
    if curtain.is_nil() && !thumpable_window.is_nil() {
        curtain = cx.call(&thumpable_window, "HasCurtains", &[])?;
    }
    if !curtain.is_nil() && !invincible {
        return Err(todo("doWindowMenu (curtains)"));
    }
    if !window_frame.is_nil() && window.is_nil() && thumpable_window.is_nil() {
        return Err(todo("doWindowMenu (a window frame)"));
    }
    let _ = (test, player_obj);
    Ok(Some(sub))
}

/// `doDoorMenu(fetch, context table, worldobjects, test, context, player,
/// hasHammer)`: the Door submenu.
pub(super) fn do_door_menu(cx: &mut Cx<'_>, fetch: &TableRef, context: &Value) -> LuaResult<Option<Value>> {
    let text = cx.text("ContextMenu_Door_option")?;
    let option = cx.option(context, "addOption", text, Value::Nil, Value::Nil, &[])?;
    let sub = sub_menu(cx, context, &option)?;
    if !fetch.get_str("door").is_nil() {
        return Err(todo("doDoorMenu (a door)"));
    }
    Ok(Some(sub))
}

/// `doThumpableWindowFrameMenu`: only for a thumpable window without a
/// window on its edge.
pub(super) fn do_thumpable_window_frame_menu(cx: &mut Cx<'_>, fetch: &TableRef) -> LuaResult<bool> {
    let thumpable = fetch.get_str("thumpableWindow");
    if thumpable.is_nil() {
        return Ok(false);
    }
    let square = cx.call(&thumpable, "getSquare", &[])?;
    let dir = cx.call(&thumpable, "getGridSquareEdgeFacingDirection", &[])?;
    if !cx.call(&square, "getWindow", &[dir])?.is_nil() {
        return Ok(false);
    }
    Err(todo("doThumpableWindowFrameMenu"))
}

/// `doBedMenu(player, test, context, bed, playerObj)`.
pub(super) fn do_bed_menu(cx: &mut Cx<'_>, player: f64, test: bool, context: &Value, bed: &Value, player_obj: &Value) -> LuaResult<bool> {
    if bed.is_nil() {
        do_sleep_option(cx, context, &Value::Nil, player, player_obj)?;
        return Ok(false);
    }
    let tile_name = cx.call(bed, "getTileName", &[])?;
    let existing = context_from_option(context, &tile_name);
    let sub = if existing.is_nil() {
        let option = cx.option(context, "addOption", tile_name, Value::Nil, Value::Nil, &[])?;
        let sprite_name = cx.call(bed, "getSpriteName", &[])?;
        let texture = cx.static_call("Texture", "trygetTexture", &[sprite_name])?;
        if instance_of(&texture, "Texture") {
            let icon = cx.call(&texture, "splitIcon", &[])?;
            option.as_table().ok_or_else(super::null_pointer)?.set_str("iconTexture", icon);
        }
        sub_menu(cx, context, &option)?
    } else {
        existing
    };
    let mut rest = !is_something_to(cx, bed, player_obj)? && !cx.is(bed, "isFurnitureOccupied", &[])?;
    if rest && cx.is(player_obj, "isSittingOnFurniture", &[])? {
        rest = !cx.is(player_obj, "isSitOnFurnitureObject", &[bed.clone()])?;
    }
    if rest {
        if test {
            return Ok(true);
        }
        let bed_square = cx.call(bed, "getSquare", &[])?;
        let bed_room = cx.call(&bed_square, "getRoom", &[])?;
        let player_square = cx.call(player_obj, "getSquare", &[])?;
        let player_room = cx.call(&player_square, "getRoom", &[])?;
        if bed_room.raw_equals(&player_room) || cx.is(&bed_square, "isCanSee", &[Value::Num(player.trunc())])? {
            let text = cx.text("ContextMenu_Rest")?;
            let f = cx.func("ISWorldObjectContextMenu.onRest");
            cx.option(&sub, "addGetUpOption", text, bed.clone(), f, &[Value::Num(player)])?;
        }
    }
    if cx.lua_class("ISWorldObjectContextMenu", "chairCheck", Value::Nil, &[bed.clone()]).truthy() {
        if is_empty_menu(cx, &sub)? {
            cx.call(context, "removeLastOption", &[])?;
        }
        return Ok(false);
    }
    if !is_something_to(cx, bed, player_obj)? && !is_client(cx)? {
        if test {
            return Ok(true);
        }
        do_sleep_option(cx, &sub, bed, player, player_obj)?;
    }
    if is_empty_menu(cx, &sub)? {
        cx.call(context, "removeLastOption", &[])?;
    }
    Ok(false)
}

/// `ISContextMenuWrapper.getContextFromOption(name)`: the submenu of the
/// option named `name`, null without one.
fn context_from_option(menu: &Value, name: &Value) -> Value {
    let Value::Table(menu) = menu else { return Value::Nil };
    let Value::Table(options) = menu.get_str("options") else { return Value::Nil };
    for (_, option) in options.entries() {
        let Value::Table(option) = option else { continue };
        if option.get_str("name").raw_equals(name) {
            return match menu.get_str("instanceMap") {
                Value::Table(map) => map.rawget(&option.get_str("subOption")),
                _ => Value::Nil,
            };
        }
    }
    Value::Nil
}

/// `GameClient.client`.
fn is_client(cx: &mut Cx<'_>) -> LuaResult<bool> {
    Ok(cx.global("isClient", &[])?.truthy())
}

/// `isSomethingTo(item, player)`.
fn is_something_to(cx: &mut Cx<'_>, item: &Value, player_obj: &Value) -> LuaResult<bool> {
    if item.is_nil() {
        return Ok(false);
    }
    let square = cx.call(item, "getSquare", &[])?;
    if square.is_nil() {
        return Ok(false);
    }
    let mut player_square = cx.call(player_obj, "getCurrentSquare", &[])?;
    if !cx.static_call("AdjacentFreeTileFinder", "isTileOrAdjacent", &[player_square.clone(), square.clone()])?.truthy() {
        player_square = cx.static_call("AdjacentFreeTileFinder", "Find", &[square.clone(), player_obj.clone()])?;
    }
    Ok(!player_square.is_nil() && cx.is(&square, "isSomethingTo", &[player_square])?)
}

/// `doSleepOption(context, bed, player, playerObj)`.
fn do_sleep_option(cx: &mut Cx<'_>, context: &Value, bed: &Value, player: f64, player_obj: &Value) -> LuaResult<()> {
    if !cx.call(player_obj, "getVehicle", &[])?.is_nil() {
        return Ok(());
    }
    let mut text = cx.text(if bed.is_nil() { "ContextMenu_SleepOnGround" } else { "ContextMenu_Sleep" })?;
    let quality = get_bed_quality(cx, player_obj, bed)?;
    if quality == "floorPillow" {
        text = cx.text("ContextMenu_SleepOnGroundPillow")?;
    }
    if !bed.is_nil() {
        let bed_square = cx.call(bed, "getSquare", &[])?;
        let bed_room = cx.call(&bed_square, "getRoom", &[])?;
        let player_square = cx.call(player_obj, "getSquare", &[])?;
        let player_room = cx.call(&player_square, "getRoom", &[])?;
        if !bed_room.raw_equals(&player_room) {
            return Ok(());
        }
    }
    let mut sitting_on = cx.call(player_obj, "getSitOnFurnitureObject", &[])?.raw_equals(bed);
    if !bed.is_nil() && !sitting_on {
        let grid = cx.static_call("ArrayList", "new", &[])?;
        cx.call(bed, "getSpriteGridObjectsIncludingSelf", &[grid.clone()])?;
        let sitting = cx.call(player_obj, "getSitOnFurnitureObject", &[])?;
        sitting_on = cx.is(&grid, "contains", &[sitting])?;
    }
    let f = cx.func("ISWorldObjectContextMenu.onSleep");
    let method = if sitting_on { "addOption" } else { "addGetUpOption" };
    let option = cx.option(context, method, text, bed.clone(), f, &[Value::Num(player)])?;
    let mut tooltip: Option<Value> = None;
    let sleep_needed = !is_client(cx)? || {
        let options = cx.global("getServerOptions", &[])?;
        cx.is(&options, "getBoolean", &[Value::str("SleepNeeded")])?
    };
    let fatigue = {
        let stats = cx.call(player_obj, "getStats", &[])?;
        let stat = cx.static_field("CharacterStat", "FATIGUE");
        cx.num(&stats, "get", &[stat])? as f32
    };
    if sleep_needed && fatigue <= 0.3 {
        set_not_available(&option)?;
        tooltip = Some(cx.text("IGUI_Sleep_NotTiredEnough")?);
    }
    let stats = cx.call(player_obj, "getStats", &[])?;
    let unsafe_ = cx.num(&stats, "getNumVisibleZombies", &[])? > 0.0
        || cx.num(&stats, "getNumChasingZombies", &[])? > 0.0
        || cx.num(&stats, "getNumVeryCloseZombies", &[])? > 0.0;
    if sleep_needed && unsafe_ {
        set_not_available(&option)?;
        tooltip = Some(cx.text("IGUI_Sleep_NotSafe")?);
    }
    let hours = cx.num(player_obj, "getHoursSurvived", &[])?;
    let last_slept = cx.num(player_obj, "getLastHourSleeped", &[])?;
    if sleep_needed && hours - last_slept <= 1.0 {
        set_not_available(&option)?;
        tooltip = Some(cx.text("ContextMenu_NoSleepTooEarly")?);
    } else if (cx.num(player_obj, "getSleepingTabletEffect", &[])? as f32) < 2000.0 {
        let moodles = cx.call(player_obj, "getMoodles", &[])?;
        let pain = cx.static_field("MoodleType", "PAIN");
        if cx.num(&moodles, "getMoodleLevel", &[pain])? >= 2.0 && fatigue <= 0.85 {
            set_not_available(&option)?;
            tooltip = Some(cx.text("ContextMenu_PainNoSleep")?);
        } else {
            let panic = cx.static_field("MoodleType", "PANIC");
            if cx.num(&moodles, "getMoodleLevel", &[panic])? >= 1.0 {
                set_not_available(&option)?;
                tooltip = Some(cx.text("ContextMenu_PanicNoSleep")?);
            }
        }
    }
    if !bed.is_nil() {
        let bed_type = cx.global("getTextOrNull", &[Value::str(&format!("Tooltip_BedType_{quality}"))])?;
        if !bed_type.is_nil() {
            let line = cx.text_args("Tooltip_BedType", &[bed_type])?;
            tooltip = Some(match tooltip {
                Some(t) => Value::str(&format!(
                    "{} <BR> {}",
                    pz_kahlua::exec::java_value_of(&t),
                    pz_kahlua::exec::java_value_of(&line)
                )),
                None => line,
            });
        }
    }
    if let Some(description) = tooltip {
        let name = cx.text("ContextMenu_Sleeping")?;
        set_tool_tip(cx, &option, Some(name), description)?;
    }
    Ok(())
}

/// `getBedQuality(player, bed)`.
fn get_bed_quality(cx: &mut Cx<'_>, player_obj: &Value, bed: &Value) -> LuaResult<String> {
    let pillow = cx.item_tag("PILLOW");
    let mut has_pillow = false;
    for hand in ["getPrimaryHandItem", "getSecondaryHandItem"] {
        let item = cx.call(player_obj, hand, &[])?;
        if !item.is_nil() && cx.is(&item, "hasTag", &[pillow.clone()])? {
            has_pillow = true;
            break;
        }
    }
    if !cx.call(player_obj, "getVehicle", &[])?.is_nil() {
        return Err(todo("getBedQuality (in a vehicle)"));
    }
    if bed.is_nil() {
        if has_pillow {
            return Ok("floorPillow".into());
        }
        let square = cx.call(player_obj, "getSquare", &[])?;
        if pillow_on(cx, &square, &pillow)? {
            return Ok("floorPillow".into());
        }
        return Ok("floor".into());
    }
    let bed_type = cx.call(bed, "getProperty", &[Value::str("BedType")])?;
    let mut quality = bed_type.as_str().map(str::to_owned).unwrap_or_else(|| "averageBed".into());
    let tent = cx.is(bed, "propertyEquals", &[Value::str("CustomName"), Value::str("Tent")])?
        || cx.is(bed, "propertyEquals", &[Value::str("CustomName"), Value::str("Shelter")])?;
    if tent {
        let container = cx.call(bed, "getContainer", &[])?;
        if !container.is_nil() {
            let tent_bed = cx.item_tag("TENT_BED");
            if cx.is(&container, "containsTag", &[tent_bed])? {
                quality = "averageBed".into();
            }
            if cx.is(&container, "containsTag", &[pillow.clone()])? {
                return Ok(format!("{quality}Pillow"));
            }
        }
        return Ok(if has_pillow { format!("{quality}Pillow") } else { quality });
    }
    if has_pillow {
        return Ok(format!("{quality}Pillow"));
    }
    if !cx.call(bed, "getSquare", &[])?.is_nil() {
        let grid = cx.static_call("ArrayList", "new", &[])?;
        cx.call(bed, "getSpriteGridObjectsIncludingSelf", &[grid.clone()])?;
        let n = cx.num(&grid, "size", &[])? as i64;
        // The original starts at 1 and reads index i - 1, so it skips
        // the last object of the grid.
        for i in 1..n {
            let object = cx.call(&grid, "get", &[Value::Num((i - 1) as f64)])?;
            let square = cx.call(&object, "getSquare", &[])?;
            if pillow_on(cx, &square, &pillow)? {
                return Ok(format!("{quality}Pillow"));
            }
        }
    }
    Ok(quality)
}

/// A world item with the PILLOW tag on `square`.
fn pillow_on(cx: &mut Cx<'_>, square: &Value, pillow: &Value) -> LuaResult<bool> {
    let items = cx.call(square, "getWorldObjects", &[])?;
    let n = cx.num(&items, "size", &[])? as i64;
    for i in 0..n {
        let object = cx.call(&items, "get", &[Value::Num(i as f64)])?;
        let item = cx.call(&object, "getItem", &[])?;
        if !item.is_nil() && cx.is(&item, "hasTag", &[pillow.clone()])? {
            return Ok(true);
        }
    }
    Ok(false)
}

/// `doTreeMenu(fetch, context table, worldobjects, test, player, inventory,
/// context)`.
pub(super) fn do_tree_menu(cx: &mut Cx<'_>, fetch: &TableRef, context_table: &Value, worldobjects: &TableRef, test: bool, player_obj: &Value, inventory: &Value, context: &Value) -> LuaResult<bool> {
    let attach = fetch.get_str("attachAnimalTo");
    cx.lua_class(
        "AnimalContextMenu",
        "attachAnimalToObject",
        Value::Nil,
        &[attach, player_obj.clone(), Value::Table(worldobjects.clone()), context_table.clone()],
    );
    let tree = fetch.get_str("tree");
    if tree.is_nil() {
        return Ok(false);
    }
    let axe = cx.first_recurse(inventory, |cx, i| cx.not_broken_with_tag(i, "CHOP_TREE"))?;
    if axe.is_nil() {
        return Ok(false);
    }
    if test {
        return Ok(true);
    }
    let text = cx.text("ContextMenu_Chop_Tree")?;
    let f = cx.func("ISWorldObjectContextMenu.onChopTree");
    let option = cx.option(context, "addGetUpOption", text, Value::Table(worldobjects.clone()), f, &[player_obj.clone(), tree])?;
    let tex = cx.call(&axe, "getTex", &[])?;
    option.as_table().ok_or_else(super::null_pointer)?.set_str("iconTexture", tex);
    Ok(false)
}

/// `doFishingMenu(fetch, worldobjects, test, context, player, square,
/// playerNum, inventory)`.
pub(super) fn do_fishing_menu(cx: &mut Cx<'_>, fetch: &TableRef, worldobjects: &TableRef, test: bool, context: &Value, player: f64, square: &Value) -> LuaResult<bool> {
    if !joypad_state(cx, player).is_nil() {
        return Err(todo("doFishingMenu (joypad)"));
    }
    if matches!(fetch.get_str("canTrapFish"), Value::Bool(true)) {
        if test {
            return Ok(true);
        }
        return Err(todo("doFishNetOptions"));
    }
    let args = [cx.call(square, "getX", &[])?, cx.call(square, "getY", &[])?];
    let no_fish = matches!(cx.lua_class("Fishing", "isNoFishZone", Value::Nil, &args), Value::Bool(true));
    if no_fish {
        return Ok(false);
    }
    if !fetch.get_str("trapFish").is_nil() {
        if test {
            return Ok(true);
        }
        return Err(todo("doPlacedFishNetOptions"));
    }
    if matches!(fetch.get_str("canFish"), Value::Bool(true)) {
        let text = cx.text("ContextMenu_Fishing")?;
        let f = cx.func("ISWorldObjectContextMenu.openFishWindow");
        let option = cx.option(context, "addOption", text, Value::Table(worldobjects.clone()), f, &[])?;
        let texture = cx.static_call("Texture", "trygetTexture", &[Value::str("Item_FishingRod")])?;
        let icon = cx.call(&texture, "splitIcon", &[])?;
        option.as_table().ok_or_else(super::null_pointer)?.set_str("iconTexture", icon);
    }
    if matches!(fetch.get_str("canAddChum"), Value::Bool(true)) {
        return Err(todo("doChumOptions"));
    }
    Ok(false)
}

/// `doFuelMenu(fetch, worldobjects, player, test, context)`.
pub(super) fn do_fuel_menu(cx: &mut Cx<'_>, fetch: &TableRef, worldobjects: &TableRef, player: f64, test: bool, context: &Value) -> LuaResult<bool> {
    let pump = fetch.get_str("fuelPump");
    let sandbox = cx.global("getSandboxOptions", &[])?;
    let exterior = cx.call(&sandbox, "getOptionByName", &[Value::str("AllowExteriorGenerator")])?;
    let powered = (cx.is(&exterior, "getValue", &[])? && !pump.is_nil() && {
        let square = cx.call(&pump, "getSquare", &[])?;
        cx.is(&square, "haveElectricity", &[])?
    }) || (!pump.is_nil() && {
        let square = cx.call(&pump, "getSquare", &[])?;
        cx.is(&square, "hasGridPower", &[])?
    });
    let have_fuel = fetch.get_str("haveFuel");
    if !have_fuel.is_nil() && powered {
        if test {
            return Ok(true);
        }
        cx.lua_class(
            "ISWorldObjectContextMenu",
            "doFillFuelMenu",
            Value::Nil,
            &[have_fuel, Value::Num(player), Value::Table(worldobjects.clone())],
        );
    } else if !pump.is_nil() {
        let key = if !powered {
            "ContextMenu_FuelPumpNoPower"
        } else if cx.num(&pump, "getPipedFuelAmount", &[])? <= 0.0 {
            "ContextMenu_FuelPumpEmpty"
        } else {
            "ContextMenu_FuelPumpNoContainer"
        };
        let text = cx.text(key)?;
        let option = cx.option(context, "addOption", text, Value::Nil, Value::Nil, &[])?;
        set_not_available(&option)?;
    }
    Ok(false)
}

/// `doCleaningMenu(fetch, player, worldobjects, test, playerObj, context)`.
pub(super) fn do_cleaning_menu(cx: &mut Cx<'_>, fetch: &TableRef, player: f64, worldobjects: &TableRef, test: bool, player_obj: &Value, context: &Value) -> LuaResult<bool> {
    for (slot, key, on, tip) in [
        ("haveBlood", "ContextMenu_CleanStains", "ISWorldObjectContextMenu.onCleanBlood", "Tooltip_CleanStains"),
        ("haveGraffiti", "ContextMenu_CleanGraffiti", "ISWorldObjectContextMenu.onCleanGraffiti", "Tooltip_CleanGraffiti"),
    ] {
        let square = fetch.get_str(slot);
        if square.is_nil() || !cx.call(player_obj, "getVehicle", &[])?.is_nil() {
            continue;
        }
        if test {
            return Ok(true);
        }
        let text = cx.text(key)?;
        let f = cx.func(on);
        let option = cx.option(context, "addGetUpOption", text, Value::Table(worldobjects.clone()), f, &[square, Value::Num(player)])?;
        let description = cx.text(tip)?;
        set_tool_tip(cx, &option, None, description)?;
    }
    Ok(false)
}

/// `doGardeningSubmenu(context, worldobjects, player, fetch, test)`.
pub(super) fn do_gardening_submenu(cx: &mut Cx<'_>, context: &Value, worldobjects: &TableRef, player_obj: &Value, fetch: &TableRef, test: bool) -> LuaResult<()> {
    let text = cx.text("ContextMenu_Gardening")?;
    let option = cx.option(context, "addOption", text, Value::Table(worldobjects.clone()), Value::Nil, &[])?;
    let sub = sub_menu(cx, context, &option)?;
    let inventory = cx.call(player_obj, "getInventory", &[])?;
    let player = cx.num(player_obj, "getIndex", &[])?;
    let scythe = cx.first_recurse(&inventory, |cx, i| cx.not_broken_with_tag(i, "SCYTHE"))?;
    let in_vehicle = |cx: &mut Cx<'_>| -> LuaResult<bool> { Ok(!cx.call(player_obj, "getVehicle", &[])?.is_nil()) };
    if !scythe.is_nil() && !in_vehicle(cx)? {
        let text = cx.text("ContextMenu_ScytheGrass")?;
        let f = cx.func("ISWorldObjectContextMenu.onScytheGrass");
        let option = cx.option(&sub, "addGetUpOption", text, player_obj.clone(), f, &[scythe.clone()])?;
        let tex = cx.call(&scythe, "getTex", &[])?;
        option.as_table().ok_or_else(super::null_pointer)?.set_str("iconTexture", tex);
    }
    let removable = fetch.get_str("canBeRemoved");
    if !removable.is_nil() && !in_vehicle(cx)? {
        let text = cx.text("ContextMenu_RemoveGrass")?;
        let f = cx.func("ISWorldObjectContextMenu.onRemoveGrass");
        cx.option(&sub, "addGetUpOption", text, Value::Table(worldobjects.clone()), f, &[removable, Value::Num(player)])?;
    }
    let compost = fetch.get_str("compost");
    if !compost.is_nil() {
        cx.lua_class(
            "ISWorldObjectContextMenu",
            "handleCompost",
            Value::Nil,
            &[Value::Bool(test), sub.clone(), Value::Table(worldobjects.clone()), player_obj.clone(), inventory.clone()],
        );
    }
    for (slot, key, wall) in [("canBeCut", "ContextMenu_RemoveBush", false), ("wallVine", "ContextMenu_RemoveWallVine", true)] {
        let square = fetch.get_str(slot);
        if !square.is_nil() && !in_vehicle(cx)? {
            let text = cx.text(key)?;
            let f = cx.func("ISWorldObjectContextMenu.onRemovePlant");
            cx.option(
                &sub,
                "addGetUpOption",
                text,
                Value::Table(worldobjects.clone()),
                f,
                &[square, Value::Bool(wall), Value::Num(player)],
            )?;
        }
    }
    let stump = fetch.get_str("stump");
    if !stump.is_nil() && !in_vehicle(cx)? {
        let text = cx.text("ContextMenu_Remove_Stump")?;
        let f = cx.func("ISWorldObjectContextMenu.onRemoveGroundCoverItemPickAxe");
        let option = cx.option(&sub, "addGetUpOption", text, Value::Table(worldobjects.clone()), f, &[Value::Num(player), stump.clone()])?;
        let sprite_name = cx.call(&stump, "getSpriteName", &[])?;
        let texture = cx.static_call("Texture", "trygetTexture", &[sprite_name])?;
        let icon = cx.call(&texture, "splitIcon", &[])?;
        option.as_table().ok_or_else(super::null_pointer)?.set_str("iconTexture", icon);
    }
    let pickup = fetch.get_str("pickupItem");
    if !pickup.is_nil() && !in_vehicle(cx)? {
        pre_pickup_ground_cover_item(cx, &sub, worldobjects, player, &pickup)?;
    }
    cx.lua_class(
        "ISFarmingMenu",
        "doDigMenu",
        Value::Nil,
        &[player_obj.clone(), sub.clone(), Value::Table(worldobjects.clone()), Value::Bool(test)],
    );
    if is_empty_menu(cx, &sub)? {
        cx.call(context, "removeLastOption", &[])?;
    }
    Ok(())
}

/// `prePickupGroundCoverItem(menu, worldobjects, player, object)`: the
/// "Remove <item>" option for an object whose `CustomName` is a
/// `GroundCoverItems` key that names an item script.
pub(super) fn pre_pickup_ground_cover_item(cx: &mut Cx<'_>, menu: &Value, worldobjects: &TableRef, player: f64, object: &Value) -> LuaResult<()> {
    let custom_name = cx.call(object, "getProperty", &[Value::str("CustomName")])?;
    let mut name = Value::Nil;
    if !custom_name.is_nil() {
        // `(KahluaTable) LuaManager.env.rawget("GroundCoverItems")`.
        let ground_cover = match cx.t.globals().rawget(&Value::str("GroundCoverItems")) {
            Value::Table(t) => t,
            Value::Nil => return Err(super::null_pointer()),
            v => return Err(crate::bind_java_util::class_cast(&v, "se.krka.kahlua.vm.KahluaTable")),
        };
        let item = ground_cover.rawget(&custom_name);
        if !item.is_nil() {
            if item.as_str().is_none() {
                return Err(crate::bind_java_util::class_cast(&item, "java.lang.String"));
            }
            let script_manager = cx.global("getScriptManager", &[])?;
            let script = cx.call(&script_manager, "getItem", &[item])?;
            if !script.is_nil() {
                let display = cx.call(&script, "getDisplayName", &[])?;
                name = cx.global("getText", &[display])?;
            }
        }
    }
    if name.is_nil() {
        return Ok(());
    }
    let text = cx.text_args("ContextMenu_Remove_GroundItem", &[name])?;
    let f = cx.func("ISWorldObjectContextMenu.onPickupGroundCoverItem");
    let option = cx.option(menu, "addGetUpOption", text, Value::Table(worldobjects.clone()), f, &[Value::Num(player), object.clone()])?;
    let sprite_name = cx.call(object, "getSpriteName", &[])?;
    let texture = cx.static_call("Texture", "trygetTexture", &[sprite_name])?;
    let icon = cx.call(&texture, "splitIcon", &[])?;
    option.as_table().ok_or_else(super::null_pointer)?.set_str("iconTexture", icon);
    Ok(())
}

/// `doDestroyMenu(worldobjects, inventory, context, player)`.
pub(super) fn do_destroy_menu(cx: &mut Cx<'_>, worldobjects: &TableRef, inventory: &Value, context: &Value, player_obj: &Value) -> LuaResult<()> {
    let tag = cx.item_tag("SLEDGEHAMMER");
    let hammer = cx.call(inventory, "getFirstTagRecurse", &[tag])?;
    if hammer.is_nil() && cx.is(player_obj, "isBuildCheat", &[])? {
        return Err(todo("doDestroyMenu (build cheat sledgehammer)"));
    }
    if hammer.is_nil() || cx.is(&hammer, "isBroken", &[])? {
        return Ok(());
    }
    if is_client(cx)? {
        let options = cx.global("getServerOptions", &[])?;
        if !cx.is(&options, "getBoolean", &[Value::str("AllowDestructionBySledgehammer")])? {
            return Ok(());
        }
    }
    let text = cx.text("ContextMenu_Destroy")?;
    let f = cx.func("ISWorldObjectContextMenu.onDestroy");
    let option = cx.option(context, "addOption", text, Value::Table(worldobjects.clone()), f, &[player_obj.clone(), hammer.clone()])?;
    let icon = cx.call(&hammer, "getIcon", &[])?;
    let icon = cx.call(&icon, "splitIcon", &[])?;
    option.as_table().ok_or_else(super::null_pointer)?.set_str("iconTexture", icon);
    Ok(())
}
