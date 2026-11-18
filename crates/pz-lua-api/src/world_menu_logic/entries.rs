//! `ISWorldObjectContextMenuLogic.createMenuEntries`: turns the `fetch`
//! table into the options of the world context menu. It returns true when
//! `createMenu` should stop (a test call found an option, or a helper took
//! the click).

use pz_kahlua::{LuaResult, TableRef, Value};

use super::helpers::{self, is_empty_menu, joypad_state, set_tool_tip, sub_menu};
use super::{contains_value, table_at, Cx};
use crate::bind_world_object::unsupported;

pub(super) struct Args {
    pub fetch: TableRef,
    pub context: TableRef,
    pub player: f64,
    pub worldobjects: TableRef,
    pub x: i32,
    pub y: i32,
    pub test: bool,
}

fn todo(name: &str) -> pz_kahlua::LuaError {
    unsupported(format!("ISWorldObjectContextMenuLogic.{name}"))
}

pub(super) fn create_menu_entries(cx: &mut Cx<'_>, a: &Args) -> LuaResult<bool> {
    let fetch = &a.fetch;
    let test = a.test;
    let player = a.player;
    let wo = Value::Table(a.worldobjects.clone());
    let pane_menu = match cx.t.globals().rawget(&Value::str("ISInventoryPaneContextMenu")) {
        Value::Table(t) => t,
        _ => return Err(super::null_pointer()),
    };
    let context = Value::Table(a.context.clone());
    let player_obj = cx.player(player)?;
    let inventory = cx.call(&player_obj, "getInventory", &[])?;
    let health = fetch.get_str("health");

    let report = {
        let options = cx.global("getDebugOptions", &[])?;
        cx.is(&options, "getBoolean", &[Value::str("UI.uiShowContextMenuReportOptions")])?
    };
    if report && !health.is_nil() && cx.core_debug()? && cx.num(&health, "getMaxHealth", &[])? > 0.0 {
        let label = cx.text("ContextMenu_ObjectHealth")?;
        let text = format!(
            "{}: {}/{}",
            pz_kahlua::exec::java_value_of(&label),
            cx.num(&health, "getHealth", &[])? as i32,
            cx.num(&health, "getMaxHealth", &[])? as i32
        );
        cx.option(&context, "addDebugOption", Value::str(&text), Value::Nil, Value::Nil, &[])?;
    }
    if !fetch.get_str("tilename").is_nil() && (cx.core_debug()? || unstable_script_name_spam(cx)?) {
        // The Lua `addTileDebugInfo` is the same report as the Java one.
        cx.lua_class("ISWorldObjectContextMenu", "addTileDebugInfo", Value::Nil, &[context.clone(), Value::Table(fetch.clone())]);
    }
    let clicked_square = fetch.get_str("clickedSquare");
    if report && (cx.core_debug()? || unstable_script_name_spam(cx)?) {
        return Err(todo("createMenuEntries (context menu report options)"));
    }
    let all_objects = helpers::get_all_objects(cx, &a.worldobjects)?;
    let copy = TableRef::new();
    for (k, v) in a.worldobjects.entries() {
        copy.rawset(k, v);
    }
    cx.lua_class(
        "DebugContextMenu",
        "doDebugMenu",
        Value::Nil,
        &[Value::Num(player), context.clone(), Value::Table(copy.clone()), Value::Bool(test)],
    );
    copy.wipe();

    let allow_interact = matches!(fetch.get_str("safehouseAllowInteract"), Value::Bool(true));
    if allow_interact && interact_entries(cx, a, &context, &player_obj, &inventory, &pane_menu, &all_objects, &clicked_square)? {
        return Ok(true);
    }

    // Safehouse view / no-permission / claim.
    let safehouse = fetch.get_str("safehouse");
    if !safehouse.is_nil() && cx.is(&safehouse, "playerAllowed", &[player_obj.clone()])? {
        if test {
            return Ok(true);
        }
        let text = cx.text("ContextMenu_ViewSafehouse")?;
        let f = cx.func("ISWorldObjectContextMenu.onViewSafeHouse");
        cx.option(&context, "addOption", text, wo.clone(), f, &[safehouse.clone(), player_obj.clone()])?;
    }
    if !matches!(fetch.get_str("safehouseAllowLoot"), Value::Bool(true)) {
        let text = cx.text("ContextMenu_NoSafehousePermissionObjects")?;
        let option = cx.option(&context, "addOption", text, Value::Nil, Value::Nil, &[])?;
        option.as_table().ok_or_else(super::null_pointer)?.set_str("notAvailable", true);
        let text = cx.text("ContextMenu_NoSafehousePermissionObjects")?;
        cx.lua_class("HaloTextHelper", "addBadText", Value::Nil, &[player_obj.clone(), text]);
    }
    if safehouse.is_nil() && !clicked_square.is_nil() {
        let building = cx.call(&clicked_square, "getBuilding", &[])?;
        if !building.is_nil() && !cx.call(&building, "getDef", &[])?.is_nil() {
            let reason = cx.static_call("SafeHouse", "canBeSafehouse", &[clicked_square.clone(), player_obj.clone()])?;
            if !reason.is_nil() {
                if test {
                    return Ok(true);
                }
                let text = cx.text("ContextMenu_SafehouseClaim")?;
                let f = cx.func("ISWorldObjectContextMenu.onTakeSafeHouse");
                let option =
                    cx.option(&context, "addOption", text, wo.clone(), f, &[clicked_square.clone(), Value::Num(player)])?;
                if reason.as_str().is_some_and(|r| !r.is_empty()) {
                    let tooltip = helpers::add_tool_tip(cx)?;
                    cx.call(&tooltip, "setVisible", &[Value::Bool(false)])?;
                    tooltip.as_table().ok_or_else(super::null_pointer)?.set_str("description", reason);
                    let o = option.as_table().ok_or_else(super::null_pointer)?;
                    o.set_str("notAvailable", true);
                    o.set_str("toolTip", tooltip);
                }
            }
        }
    }
    if allow_interact {
        let animals = table_at(fetch, "clickedAnimals");
        if animals.as_ref().is_some_and(|t| !t.is_empty()) {
            cx.global(
                "triggerEvent",
                &[
                    Value::str("OnClickedAnimalForContext"),
                    Value::Num(player),
                    context.clone(),
                    Value::Table(animals.unwrap()),
                    Value::Bool(test),
                ],
            )?;
        }
        let track = fetch.get_str("animaltrack");
        if !track.is_nil() {
            cx.lua_class("ISAnimalTracksMenu", "handleIsoTracks", Value::Nil, &[context.clone(), track, player_obj.clone()]);
        }
    }
    if cx.static_call("SafeHouse", "isSafehouseAllowClaimWar", &[safehouse.clone(), player_obj.clone()])?.truthy() {
        let text = cx.text("ContextMenu_WarClaim")?;
        let f = cx.func("ISWorldObjectContextMenu.onClaimWar");
        let id = cx.call(&safehouse, "getOnlineID", &[])?;
        let name = cx.call(&player_obj, "getUsername", &[])?;
        cx.option(&context, "addOption", text, wo.clone(), f, &[id, name])?;
    }
    let zone = fetch.get_str("animalZone");
    if !zone.is_nil() {
        cx.lua_class("AnimalContextMenu", "doDesignationZoneMenu", Value::Nil, &[context.clone(), zone, player_obj.clone()]);
    }
    if allow_interact && cx.call(&player_obj, "getVehicle", &[])?.is_nil() {
        if test {
            return Ok(true);
        }
        let mut dung = cx.first_recurse(&inventory, |cx, i| cx.not_broken_with_tag(i, "TAKE_DUNG"))?;
        if let Some(item) = hand_item_with_tag(cx, &player_obj, "TAKE_DUNG")? {
            dung = item;
        }
        let mut shovel = cx.first_recurse(&inventory, |cx, i| cx.not_broken_with_tag(i, "DIG_GRAVE"))?;
        if let Some(item) = hand_item_with_tag(cx, &player_obj, "DIG_GRAVE")? {
            shovel = item;
        }
        let graves = fetch.get_str("graves");
        if !shovel.is_nil() {
            return Err(todo("doShovelAndRackMenu"));
        }
        if !graves.is_nil() {
            return Err(todo("isGraveFullOfCorpses / doBuryCorpseMenu"));
        }
        if !dung.is_nil() && cx.call(&player_obj, "getVehicle", &[])?.is_nil() && !dung.raw_equals(&shovel) {
            let text = cx.text("ContextMenu_Rake")?;
            let option = cx.option(&context, "addOption", text, wo.clone(), Value::Nil, &[])?;
            let sub = sub_menu(cx, &context, &option)?;
            let text = cx.text("ContextMenu_RakeDung")?;
            let f = cx.func("ISWorldObjectContextMenu.onRakeDung");
            cx.option(&sub, "addGetUpOption", text, player_obj.clone(), f, &[dung])?;
        }
        helpers::do_gardening_submenu(cx, &context, &a.worldobjects, &player_obj, fetch, test)?;
        cx.lua_class(
            "ISDisassembleMenu",
            "createMenu",
            Value::Table(all_objects.clone()),
            &[context.clone(), player_obj.clone()],
        );
        helpers::do_destroy_menu(cx, &a.worldobjects, &inventory, &context, &player_obj)?;
    }
    if cx.call(&player_obj, "getVehicle", &[])?.is_nil()
        && !cx.is(&player_obj, "isSitOnGround", &[])?
        && !cx.is(&player_obj, "isSittingOnFurniture", &[])?
    {
        if test {
            return Ok(true);
        }
        let text = cx.text("ContextMenu_SitGround")?;
        let f = cx.func("ISWorldObjectContextMenu.onSitOnGround");
        cx.option(&context, "addOption", text, Value::Num(player), f, &[])?;
    }
    if joypad_state(cx, player).is_nil() && cx.call(&player_obj, "getVehicle", &[])?.is_nil() {
        if test {
            return Ok(true);
        }
        let text = cx.text("ContextMenu_Walk_to")?;
        let f = cx.func("ISWorldObjectContextMenu.onWalkTo");
        cx.option(&context, "addGetUpOption", text, wo, f, &[fetch.get_str("item"), Value::Num(player)])?;
    }
    Ok(false)
}

/// The block of `createMenuEntries` that runs when the safehouse allows
/// interaction (bytecode 514 to 2847). True means "return true".
#[allow(clippy::too_many_arguments)]
fn interact_entries(
    cx: &mut Cx<'_>,
    a: &Args,
    context: &Value,
    player_obj: &Value,
    inventory: &Value,
    pane_menu: &TableRef,
    all_objects: &TableRef,
    clicked_square: &Value,
) -> LuaResult<bool> {
    let fetch = &a.fetch;
    let test = a.test;
    let player = a.player;
    let wo = Value::Table(a.worldobjects.clone());
    let context_table = Value::Table(a.context.clone());

    if !fetch.get_str("firetile").is_nil() && !fetch.get_str("extinguisher").is_nil() {
        return Err(todo("doExtinguishFireMenu"));
    }
    let primary = cx.call(player_obj, "getPrimaryHandItem", &[])?;
    if !primary.is_nil() && cx.is(&primary, "isForceDropHeavyItem", &[])? {
        let items = TableRef::new();
        items.rawset(Value::Num(1.0), primary.clone());
        let name = cx.call(&primary, "getDisplayName", &[])?;
        let text = cx.text_args("ContextMenu_DropNamedItem", &[name])?;
        let on_unequip = pane_menu.get_str("onUnEquip");
        cx.option(context, "addOption", text, Value::Table(items), on_unequip, &[Value::Num(player)])?;
    }
    helpers::handle_interaction(cx, a.x, a.y, test, context, &a.worldobjects, player_obj)?;
    if helpers::handle_grab_world_item(cx, fetch, a.x, a.y, test, context, &a.worldobjects, player_obj, inventory)? {
        return Ok(true);
    }
    let body = fetch.get_str("body");
    if !body.is_nil() && !cx.is(&body, "isAnimal", &[])? && cx.call(player_obj, "getVehicle", &[])?.is_nil() {
        return Err(todo("doBurnBodyMenu"));
    }
    if !joypad_state(cx, player).is_nil()
        && !cx.call(player_obj, "getCurrentSquare", &[])?.is_nil()
        && !fetch.get_str("window").is_nil()
        && !fetch.get_str("thumpableWindow").is_nil()
    {
        return Err(todo("getBestWindowFrame"));
    }
    let has_hammer = !cx
        .first_recurse(inventory, |cx, i| {
            let tag = cx.item_tag("HAMMER");
            Ok(cx.is(i, "hasTag", &[tag])? && !cx.is(i, "isBroken", &[])?)
        })?
        .is_nil();
    let has_remove_barricade = !cx
        .first_recurse(inventory, |cx, i| {
            let tag = cx.item_tag("REMOVE_BARRICADE");
            Ok(cx.is(i, "hasTag", &[tag])? && !cx.is(i, "isBroken", &[])?)
        })?
        .is_nil();
    let _ = (has_hammer, has_remove_barricade);
    if helpers::do_thumpable_window_frame_menu(cx, fetch)? {
        return Ok(true);
    }
    let invincible = matches!(fetch.get_str("invincibleWindow"), Value::Bool(true));
    let thump = fetch.get_str("thump");
    let window = fetch.get_str("window");
    if !thump.is_nil() && !invincible && window.is_nil() {
        return Err(todo("doThumpableMenu"));
    }
    if !fetch.get_str("throwCorpseOver").is_nil() {
        return Err(todo("doHoppableMenu"));
    }
    let Some(window_menu) =
        helpers::do_window_menu(cx, fetch, &a.worldobjects, test, context, &window, invincible, player_obj)?
    else {
        return Ok(true);
    };
    if is_empty_menu(cx, &window_menu)? {
        cx.call(context, "removeLastOption", &[])?;
    }
    let glass = fetch.get_str("brokenGlass");
    if !glass.is_nil() {
        let text = cx.text("ContextMenu_PickupBrokenGlass")?;
        let f = cx.func("ISWorldObjectContextMenu.onPickupBrokenGlass");
        let index = cx.call(player_obj, "getIndex", &[])?;
        cx.option(context, "addGetUpOption", text, wo.clone(), f, &[glass, index])?;
    }
    let Some(door_menu) = helpers::do_door_menu(cx, fetch, context)? else {
        return Ok(true);
    };
    if is_empty_menu(cx, &door_menu)? {
        cx.call(context, "removeLastOption", &[])?;
    }
    if !fetch.get_str("generator").is_nil() && cx.call(player_obj, "getVehicle", &[])?.is_nil() {
        return Err(todo("doGeneratorMenu"));
    }
    let container = fetch.get_str("container");
    if cx.core_debug()? && !container.is_nil() {
        let loot = cx.static_call("ItemPickerJava", "getLootDebugString", &[container])?;
        if !loot.is_nil() {
            cx.option(context, "addDebugOption", loot, Value::Nil, Value::Nil, &[])?;
        }
    }
    let fuel_debug = fetch.get_str("haveFuelDebug");
    if !fuel_debug.is_nil() && cx.core_debug()? {
        let label = cx.text("ContextMenu_PumpFuelAmount")?;
        let amount = cx.num(&fuel_debug, "getPipedFuelAmount", &[])? as i32;
        let text = format!("{}: {amount}", pz_kahlua::exec::java_value_of(&label));
        cx.option(context, "addDebugOption", Value::str(&text), Value::Nil, Value::Nil, &[])?;
    }
    let hook = fetch.get_str("butcherHook");
    if !hook.is_nil() && cx.call(player_obj, "getVehicle", &[])?.is_nil() {
        if test {
            return Ok(true);
        }
        let text = cx.text("ContextMenu_ButcherHook")?;
        let f = cx.func("ISWorldObjectContextMenu.onButcherHook");
        cx.option(context, "addGetUpOption", text, hook, f, &[player_obj.clone()])?;
    }
    if !fetch.get_str("carBatteryCharger").is_nil() {
        let r = cx.lua_class(
            "ISWorldObjectContextMenu",
            "handleCarBatteryCharger",
            Value::Nil,
            &[Value::Bool(test), context_table.clone(), wo.clone(), player_obj.clone(), inventory.clone()],
        );
        if matches!(r, Value::Bool(true)) {
            return Ok(true);
        }
    }
    for (class, method) in
        [("ISTrapMenu", "doTrapMenu"), ("ISFarmingMenu", "doFarmingMenu"), ("ISBBQMenu", "OnFillWorldObjectContextMenu"), ("ISCampingMenu", "doCampingMenu")]
    {
        cx.lua_class(class, method, Value::Nil, &[Value::Num(player), context_table.clone(), wo.clone(), Value::Bool(test)]);
    }
    if !fetch.get_str("entityContext").is_nil() {
        if test {
            return Ok(true);
        }
        return Err(todo("doContextConfigOptionsFromFetch"));
    }
    if test {
        return Ok(true);
    }
    cx.lua_class("ISRadioAndTvMenu", "createMenu", Value::Table(all_objects.clone()), &[context_table.clone(), player_obj.clone()]);
    let ashes = fetch.get_str("ashes");
    if !ashes.is_nil() {
        let text = cx.text("ContextMenu_Clear_Ashes")?;
        let f = cx.func("ISWorldObjectContextMenu.onClearAshes");
        cx.option(context, "addGetUpOption", text, wo.clone(), f, &[Value::Num(player), ashes])?;
    }
    let ore = fetch.get_str("ore");
    if !ore.is_nil() && cx.call(player_obj, "getVehicle", &[])?.is_nil() {
        let text = cx.text("ContextMenu_Remove_Ore")?;
        let f = cx.func("ISWorldObjectContextMenu.onRemoveGroundCoverItemHammerOrPickAxe");
        cx.option(context, "addGetUpOption", text, wo.clone(), f, &[Value::Num(player), ore])?;
    }
    let trap = fetch.get_str("trap");
    if !trap.is_nil() && !cx.call(&trap, "getItem", &[])?.is_nil() {
        return Err(todo("doTrapMenu"));
    }
    let picker = cx.static_field("IsoObjectPicker", "Instance");
    let corpse = cx.call(&picker, "PickCorpse", &[Value::Num(a.x as f64), Value::Num(a.y as f64)])?;
    if !corpse.is_nil() && cx.is(&corpse, "isAnimal", &[])? {
        fetch.set_str("animalbody", corpse);
    }
    let animal_body = fetch.get_str("animalbody");
    if cx.call(player_obj, "getVehicle", &[])?.is_nil() && !animal_body.is_nil() {
        cx.lua_class("AnimalContextMenu", "doAnimalBodyMenu", Value::Nil, &[context_table.clone(), Value::Num(player), animal_body]);
    }
    for slot in ["padlockThump", "padlockedThump", "digitalPadlockedThump"] {
        if !fetch.get_str(slot).is_nil() {
            return Err(todo("doPadLockMenu"));
        }
    }
    if !fetch.get_str("canBeWaterPiped").is_nil() {
        return Err(todo("doWaterPipeMenu"));
    }
    let ground = fetch.get_str("groundType");
    if cx.call(player_obj, "getVehicle", &[])?.is_nil()
        && ground.as_table().is_some_and(|g| !g.get_str("sand").is_nil())
        && cx.is(player_obj, "isRecipeActuallyKnown", &[Value::str("MakeChum")])?
    {
        return Err(todo("doCreateChumOptions"));
    }
    let rope = fetch.get_str("sheetRopeSquare");
    if !rope.is_nil() && cx.is(player_obj, "canClimbSheetRope", &[rope.clone()])? {
        let strength = cx.static_field("Perks", "Strength");
        if cx.num(player_obj, "getPerkLevel", &[strength])? >= 0.0 {
            let text = cx.text("ContextMenu_Climb_Sheet_Rope")?;
            let f = cx.func("ISWorldObjectContextMenu.onClimbSheetRope");
            cx.option(context, "addGetUpOption", text, wo.clone(), f, &[rope, Value::Bool(false), Value::Num(player)])?;
        }
    }
    if table_at(fetch, "fluidcontainer").is_some_and(|t| !t.is_empty()) {
        return Err(todo("doFluidContainerMenu"));
    }
    let store_water = table_at(fetch, "storeWater");
    let fluid = table_at(fetch, "fluidcontainer");
    if let Some(store) = &store_water {
        if store.entries().iter().any(|(_, o)| !o.is_nil() && !contains_value(fluid.as_ref(), o)) {
            return Err(todo("doWashClothingOrYourselfMenu"));
        }
    }
    let combo = fetch.get_str("comboWasherDryer");
    if store_water.as_ref().is_some_and(|t| t.is_empty()) && !combo.is_nil() {
        return Err(todo("toggleComboWasherDryer"));
    }
    cx.lua_class(
        "ISFeedingTroughMenu",
        "OnFillWorldObjectContextMenu",
        Value::Nil,
        &[Value::Num(player), context_table.clone(), wo.clone(), Value::Bool(test)],
    );
    let barrel = fetch.get_str("rainCollectorBarrel");
    if !barrel.is_nil() && !contains_value(fluid.as_ref(), &barrel) {
        return Err(todo("addFluidFromItem (rain collector barrel)"));
    }
    let world_item = fetch.get_str("worldItem");
    if !world_item.is_nil()
        && !cx.call(&world_item, "getItem", &[])?.is_nil()
        && cx.num(&world_item, "getFluidCapacity", &[])? > 0.0
        && !contains_value(fluid.as_ref(), &world_item)
    {
        return Err(todo("addFluidFromItem (world item)"));
    }
    if !fetch.get_str("clothingDryer").is_nil() {
        return Err(todo("onWashingDryer"));
    }
    if !fetch.get_str("stove").is_nil() {
        return Err(todo("doStoveOption"));
    }
    let bed = fetch.get_str("bed");
    let tired = {
        let stats = cx.call(player_obj, "getStats", &[])?;
        let stat = cx.static_field("CharacterStat", "FATIGUE");
        cx.num(&stats, "get", &[stat])? as f32 > 0.9
    };
    if (!bed.is_nil() || tired) && helpers::do_bed_menu(cx, player, test, context, &bed, player_obj)? {
        return Ok(true);
    }
    let light = cx.lua_class(
        "ISWorldObjectContextMenu",
        "doLightSwitchOption",
        Value::Nil,
        &[Value::Bool(test), context_table.clone(), Value::Num(player)],
    );
    if matches!(light, Value::Bool(true)) {
        return Ok(true);
    }
    if helpers::do_tree_menu(cx, fetch, &context_table, &a.worldobjects, test, player_obj, inventory, context)? {
        return Ok(true);
    }
    if !clicked_square.is_nil()
        && helpers::do_fishing_menu(cx, fetch, &a.worldobjects, test, context, player, clicked_square)?
    {
        return Ok(true);
    }
    if helpers::do_fuel_menu(cx, fetch, &a.worldobjects, player, test, context)? {
        return Ok(true);
    }
    let clicked_player = fetch.get_str("clickedPlayer");
    if !clicked_player.is_nil() && !clicked_player.raw_equals(player_obj) {
        return Err(todo("doClickedPlayerMenu"));
    }
    if helpers::do_cleaning_menu(cx, fetch, player, &a.worldobjects, test, player_obj, context)? {
        return Ok(true);
    }
    let _ = set_tool_tip;
    Ok(false)
}

/// The primary hand item when it is not broken and has the tag.
fn hand_item_with_tag(cx: &mut Cx<'_>, player_obj: &Value, tag: &str) -> LuaResult<Option<Value>> {
    let item = cx.call(player_obj, "getPrimaryHandItem", &[])?;
    if item.is_nil() || cx.is(&item, "isBroken", &[])? {
        return Ok(None);
    }
    let tag = cx.item_tag(tag);
    Ok(cx.is(&item, "hasTag", &[tag])?.then_some(item))
}

/// `SandboxOptions.instance.isUnstableScriptNameSpam()`.
fn unstable_script_name_spam(cx: &mut Cx<'_>) -> LuaResult<bool> {
    let sandbox = cx.global("getSandboxOptions", &[])?;
    cx.is(&sandbox, "isUnstableScriptNameSpam", &[])
}
