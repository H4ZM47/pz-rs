//! `ISWorldObjectContextMenuLogic.fetch` and the helpers only it uses: it
//! sorts each clicked object (and, for the clicked object, everything on its
//! square) into the named slots of the `fetch` table.

use pz_kahlua::{LuaResult, TableRef, Value};

use super::{append, contains_value, table_at, Cx};
use crate::bind_globals::instance_of;

/// `fetch(fetch, v, player, doSquare)`.
pub(super) fn fetch(cx: &mut Cx<'_>, fetch: &TableRef, v: &Value, player: f64, do_square: bool) -> LuaResult<()> {
    let menu = match cx.t.globals().rawget(&Value::str("ISWorldObjectContextMenu")) {
        Value::Table(t) => t,
        _ => return Err(super::null_pointer()),
    };
    let fetch_squares = table_at(&menu, "fetchSquares");
    let player_obj = cx.player(player)?;
    let inventory = cx.call(&player_obj, "getInventory", &[])?;
    let props = cx.call(v, "getProperties", &[])?;

    if do_square {
        let square = cx.call(v, "getSquare", &[])?;
        if !square.is_nil() {
            let world_objects = cx.call(&square, "getWorldObjects", &[])?;
            if !world_objects.is_nil() && !cx.is(&world_objects, "isEmpty", &[])? {
                let first = cx.call(&world_objects, "get", &[Value::Num(0.0)])?;
                fetch.set_str("worldItem", first);
            }
            let building = cx.call(&square, "getBuilding", &[])?;
            fetch.set_str("building", building);
        }
    }
    if cx.is(v, "hasFluid", &[])? {
        let store_water = table_at(fetch, "storeWater");
        if instance_of(v, "IsoFeedingTrough") {
            let master = cx.call(v, "getMasterTrough", &[])?;
            if !master.is_nil() && !contains_value(store_water.as_ref(), &master) {
                append(store_water.as_ref().ok_or_else(super::null_pointer)?, master);
            }
        } else if !contains_value(store_water.as_ref(), v) {
            append(store_water.as_ref().ok_or_else(super::null_pointer)?, v.clone());
        }
    }
    let c = fetch.get_str("c").as_num().ok_or_else(super::null_pointer)?;
    fetch.set_str("c", c + 1.0);
    if instance_of(v, "IsoWindow") {
        fetch.set_str("window", v.clone());
    } else if instance_of(v, "IsoCurtain") {
        fetch.set_str("curtain", v.clone());
    }
    if instance_of(v, "IsoDoor") || (instance_of(v, "IsoThumpable") && cx.is(v, "isDoor", &[])?) {
        fetch.set_str("door", v.clone());
        if instance_of(v, "IsoDoor") {
            let key_id = cx.num(v, "checkKeyId", &[])?;
            fetch.set_str("doorKeyId", key_id);
            if key_id == -1.0 {
                fetch.set_str("doorKeyId", Value::Nil);
            }
        }
        if instance_of(v, "IsoThumpable") {
            let key_id = cx.num(v, "getKeyId", &[])?;
            if key_id != -1.0 {
                fetch.set_str("doorKeyId", key_id);
            }
        }
    }
    if instance_of(v, "IsoAnimalTrack") {
        fetch.set_str("animaltrack", v.clone());
    }
    if do_square {
        let square = cx.call(v, "getSquare", &[])?;
        let track = cx.call(&square, "getAnimalTrack", &[])?;
        if !track.is_nil() {
            fetch.set_str("animaltrack", track);
        }
    }
    fetch.set_str("item", v.clone());
    if instance_of(v, "IsoButcherHook") {
        fetch.set_str("butcherHook", v.clone());
    }
    let context_menu_config = cx.static_field("ComponentType", "ContextMenuConfig");
    let fluid_container = cx.static_field("ComponentType", "FluidContainer");
    if cx.is(v, "hasComponent", &[context_menu_config])? && !cx.is(v, "hasComponent", &[fluid_container.clone()])? {
        fetch.set_str("entityContext", v.clone());
    }
    let sprite = cx.call(v, "getSprite", &[])?;
    if !sprite.is_nil() {
        let name = cx.call(&sprite, "getName", &[])?;
        if !name.is_nil() {
            fetch.set_str("tilename", name);
            let container = cx.call(v, "getContainer", &[])?;
            if !container.is_nil() {
                let kind = cx.call(&container, "getType", &[])?;
                if !kind.is_nil() {
                    let tilename = java_string(&fetch.get_str("tilename"));
                    fetch.set_str("tilename", Value::str(&format!("{tilename}{}", java_string(&kind))));
                }
            }
            fetch.set_str("tileObj", v.clone());
        }
    }
    if instance_of(v, "IsoCompost") {
        fetch.set_str("compost", v.clone());
    }
    for (flag, key, key2) in [("HoppableN", "hoppableN", "hoppableN_2"), ("HoppableW", "hoppableW", "hoppableW_2")] {
        if !cx.call(v, "getSprite", &[])?.is_nil() && cx.sprite_has(v, flag)? {
            set_first_or_second(fetch, key, key2, v);
        }
    }
    if cx.is(&player_obj, "isDraggingCorpse", &[])?
        && cx.num(&player_obj, "getJoypadBind", &[])? != -1.0
        && !cx.call(&player_obj, "getCurrentSquare", &[])?.is_nil()
    {
        let dir = cx.call(&player_obj, "getCardinalDirection", &[])?;
        let dir = cx.call(&dir, "Rot180", &[])?;
        let current = cx.call(&player_obj, "getCurrentSquare", &[])?;
        let adjacent = cx.call(&current, "getAdjacentSquare", &[dir])?;
        for (method, key) in
            [("getWindowTo", "window"), ("getWindowFrameTo", "windowFrame"), ("getWindowThumpableTo", "thumpableWindow")]
        {
            let current = cx.call(&player_obj, "getCurrentSquare", &[])?;
            let found = cx.call(&current, method, &[adjacent.clone()])?;
            if !found.is_nil() {
                fetch.set_str(key, found);
            }
        }
    }
    if cx.is(&player_obj, "isDraggingCorpse", &[])? && fetch.get_str("throwCorpseOver").is_nil() {
        fetch_throw_corpse_over(cx, &player_obj, fetch, "hoppableW")?;
        fetch_throw_corpse_over(cx, &player_obj, fetch, "hoppableN")?;
        if cx.num(&player_obj, "getJoypadBind", &[])? != -1.0 {
            let dir = cx.call(&player_obj, "getCardinalDirection", &[])?;
            let dir = cx.call(&dir, "Rot180", &[])?;
            if cx.is(&player_obj, "canThrowCorpseOver", &[dir.clone()])? {
                let current = cx.call(&player_obj, "getCurrentSquare", &[])?;
                let adjacent = cx.call(&current, "getAdjacentSquare", &[dir])?;
                let current = cx.call(&player_obj, "getCurrentSquare", &[])?;
                let hoppable = cx.call(&current, "getHoppableTo", &[adjacent])?;
                fetch.set_str("throwCorpseOver", hoppable);
            }
        }
    }
    if instance_of(v, "IsoThumpable") && !cx.is(v, "isDoor", &[])? {
        fetch.set_str("thump", v.clone());
        if cx.is(v, "canBeLockByPadlock", &[])?
            && !cx.is(v, "isLockedByPadlock", &[])?
            && cx.num(v, "getLockedByCode", &[])? == 0.0
        {
            fetch.set_str("padlockThump", v.clone());
        }
        if cx.is(v, "isLockedByPadlock", &[])? {
            fetch.set_str("padlockedThump", v.clone());
        }
        if cx.num(v, "getLockedByCode", &[])? > 0.0 {
            fetch.set_str("digitalPadlockedThump", v.clone());
        }
        if cx.is(v, "isWindow", &[])? {
            fetch.set_str("thumpableWindow", v.clone());
        }
        if cx.is(v, "isWindowN", &[])? {
            set_first_or_second(fetch, "thumpableWindowN", "thumpableWindowN_2", v);
        }
        if cx.is(v, "isWindowW", &[])? {
            set_first_or_second(fetch, "thumpableWindowW", "thumpableWindowW_2", v);
        }
        if cx.is(v, "propertyEquals", &[Value::str("CustomName"), Value::str("Rain Collector Barrel")])? {
            fetch.set_str("rainCollectorBarrel", v.clone());
        }
    }
    if instance_of(v, "IsoTree") {
        fetch.set_str("tree", v.clone());
    }
    let attach_animal = instance_of(v, "IsoTree") || {
        let props = cx.call(v, "getProperties", &[])?;
        if props.is_nil() {
            false
        } else {
            // `TilePropertyKey.CAN_ATTACH_ANIMAL.toString()`: the id of its
            // `ResourceLocation`, `namespace:path` in lower case.
            cx.is(&props, "has", &[Value::str("base:canattachanimal")])?
        }
    };
    if attach_animal {
        fetch.set_str("attachAnimalTo", v.clone());
    }
    for (class, key) in [
        ("IsoClothingDryer", "clothingDryer"),
        ("IsoClothingWasher", "clothingWasher"),
        ("IsoCombinationWasherDryer", "comboWasherDryer"),
    ] {
        if instance_of(v, class) {
            fetch.set_str(key, v.clone());
        }
    }
    if instance_of(v, "IsoStove") && !cx.call(v, "getContainer", &[])?.is_nil() {
        fetch.set_str("stove", v.clone());
    }
    if instance_of(v, "IsoDeadBody") && !cx.is(v, "isAnimal", &[])? {
        let body = fetch.get_str("body");
        let closer = body.is_nil() || {
            let old = cx.num(&body, "DistToSquared", &[player_obj.clone()])?;
            let new = cx.num(v, "DistToSquared", &[player_obj.clone()])?;
            old > new
        };
        if closer {
            fetch.set_str("body", v.clone());
        }
    }
    if instance_of(v, "IsoDeadBody") && cx.is(v, "isAnimal", &[])? {
        fetch.set_str("animalbody", v.clone());
    }
    if instance_of(v, "IsoCarBatteryCharger") {
        fetch.set_str("carBatteryCharger", v.clone());
    }
    if instance_of(v, "IsoGenerator") {
        fetch.set_str("generator", v.clone());
    }
    if do_square {
        let square = cx.call(v, "getSquare", &[])?;
        let body = if square.is_nil() { Value::Nil } else { cx.call(&square, "getDeadBody", &[])? };
        if !body.is_nil() && fetch.get_str("body").is_nil() && cx.is(&body, "isAnimal", &[])? {
            fetch.set_str("body", body.clone());
        }
        if !body.is_nil() && fetch.get_str("animalbody").is_nil() && cx.is(&body, "isAnimal", &[])? {
            fetch.set_str("animalbody", body);
        }
    }
    if !cx.call(v, "getSprite", &[])?.is_nil() {
        if cx.sprite_has(v, "bed")? {
            fetch.set_str("bed", v.clone());
        }
        if cx.sprite_has(v, "makeWindowInvincible")? {
            fetch.set_str("invincibleWindow", true);
        }
    }
    for (class, key) in [("IsoWindowFrame", "windowFrame"), ("IsoBrokenGlass", "brokenGlass"), ("IsoTrap", "trap")] {
        if instance_of(v, class) {
            fetch.set_str(key, v.clone());
        }
    }
    if cx.call(v, "getName", &[])?.as_str() == Some("EmptyGraves") && !is_grave_filled_in(cx, v)? {
        fetch.set_str("graves", v.clone());
    }
    if instance_of(v, "IsoLightSwitch") {
        let square = cx.call(v, "getSquare", &[])?;
        if !square.is_nil()
            && (!cx.call(&square, "getRoom", &[])?.is_nil() || cx.is(v, "getCanBeModified", &[])?)
        {
            fetch.set_str("lightSwitch", v.clone());
        }
    }
    if do_square {
        let square = cx.call(v, "getSquare", &[])?;
        if !square.is_nil() {
            let square_props = cx.call(&square, "getProperties", &[])?;
            if cx.has_flag(&square_props, "HoppableW")? || {
                let square_props = cx.call(&square, "getProperties", &[])?;
                cx.has_flag(&square_props, "HoppableN")?
            } {
                fetch.set_str("canClimbThrough", true);
            }
        }
    }
    if !cx.call(v, "getSprite", &[])?.is_nil()
        && cx.sprite_has(v, "water")?
        && square_dist(cx, v, &player_obj)? < 20.0
        && !cx.is(&player_obj, "isSitOnGround", &[])?
    {
        fetch.set_str("canFish", true);
        let square = cx.call(v, "getSquare", &[])?;
        let water = cx.call(&square, "getWater", &[])?;
        if !water.is_nil() && cx.is(&water, "isShore", &[])? {
            fetch.set_str("canFish", false);
        }
        if square_dist(cx, v, &player_obj)? < 8.0 {
            let inventory = cx.call(&player_obj, "getInventory", &[])?;
            let chum = cx.call(&inventory, "getAllTypeRecurse", &[Value::str("Base.Chum")])?;
            if !cx.is(&chum, "isEmpty", &[])? {
                fetch.set_str("canAddChum", true);
            }
        }
    }
    if do_square {
        let square = cx.call(v, "getSquare", &[])?;
        let ground = get_dirt_gravel_sand(cx, &square)?;
        fetch.set_str("groundType", ground.clone());
        if !ground.is_nil() {
            fetch.set_str("groundSquare", square);
        }
    }
    let cut_plant = cx.first_recurse(&inventory, |cx, i| cx.not_broken_with_tag(i, "CUT_PLANT"))?;
    if !cx.call(v, "getSprite", &[])?.is_nil() {
        if cx.sprite_has(v, "canBeCut")? && !cut_plant.is_nil() {
            let square = cx.call(v, "getSquare", &[])?;
            fetch.set_str("canBeCut", square);
        }
        if cx.sprite_has(v, "canBeRemoved")? {
            let square = cx.call(v, "getSquare", &[])?;
            fetch.set_str("canBeRemoved", square);
        }
    }
    let anim_sprites = cx.call(v, "getAttachedAnimSprite", &[])?;
    if !cut_plant.is_nil() && !anim_sprites.is_nil() {
        let n = cx.num(&anim_sprites, "size", &[])? as i64;
        for i in 0..n {
            let instance = cx.call(&anim_sprites, "get", &[Value::Num(i as f64)])?;
            if instance.is_nil() {
                continue;
            }
            let parent = cx.call(&instance, "getParentSprite", &[])?;
            if parent.is_nil() {
                continue;
            }
            let name = cx.call(&parent, "getName", &[])?;
            if name.as_str().is_some_and(|n| n.starts_with("f_wallvines_")) {
                let square = cx.call(v, "getSquare", &[])?;
                fetch.set_str("wallVine", square);
                break;
            }
        }
    }
    if !cx.call(v, "getSprite", &[])?.is_nil()
        && cx.sprite_has(v, "water")?
        && cx.is(&inventory, "containsTypeRecurse", &[Value::str("FishingNet")])?
    {
        fetch.set_str("canTrapFish", true);
    }
    if cx.call(v, "getName", &[])?.as_str() == Some("FishingNet") && !cx.call(v, "getSquare", &[])?.is_nil() {
        fetch.set_str("trapFish", v.clone());
    }
    if do_square {
        let square = cx.call(v, "getSquare", &[])?;
        let mut climb = false;
        for flag in ["climbSheetN", "climbSheetW", "climbSheetS", "climbSheetE"] {
            let square_props = cx.call(&square, "getProperties", &[])?;
            if cx.has_flag(&square_props, flag)? {
                climb = true;
                break;
            }
        }
        if climb {
            fetch.set_str("sheetRopeSquare", square.clone());
        }
        if !square_to_extinguish(cx, &square)?.is_nil() {
            // fetch.extinguisher = FireFighting.getExtinguisher(player) (the
            // hands, then the inventory's extinguisher, sand bag or water
            // source); fetch.firetile = square.
            return Err(crate::bind_world_object::unsupported("FireFighting.getExtinguisher"));
        }
        fetch.set_str("clickedSquare", square);
    }
    if do_square
        && !cx.first_recurse(&inventory, |cx, i| cx.not_broken_with_tag(i, "CLEAR_ASHES"))?.is_nil()
        && !cx.call(v, "getSprite", &[])?.is_nil()
    {
        let sprite = cx.call(v, "getSprite", &[])?;
        let mut name = cx.call(&sprite, "getName", &[])?;
        if name.is_nil() {
            name = cx.call(v, "getSpriteName", &[])?;
        }
        if matches!(name.as_str(), Some("floors_burnt_01_1" | "floors_burnt_01_2")) {
            let ashes = fetch.get_str("ashes");
            if ashes.is_nil() || cx.num(&ashes, "getTargetAlpha", &[])? <= cx.num(v, "getTargetAlpha", &[])? {
                fetch.set_str("ashes", v.clone());
            }
        }
    }
    if do_square {
        let mut hammer = cx.first_recurse(&inventory, |cx, i| sledgehammer(cx, i, "Sledgehammer"))?;
        if hammer.is_nil() {
            hammer = cx.first_recurse(&inventory, |cx, i| sledgehammer(cx, i, "Sledgehammer2"))?;
        }
        if !hammer.is_nil()
            && cx.num(&hammer, "getCondition", &[])? > 0.0
            && !cx.call(v, "getSprite", &[])?.is_nil()
            && destroyable(cx, v)?
        {
            let sprite = cx.call(v, "getSprite", &[])?;
            let name = cx.call(&sprite, "getName", &[])?;
            let burnt_blend = name
                .as_str()
                .is_some_and(|n| n.starts_with("blends_natural_02") && n.starts_with("floors_burnt_01_"));
            if !burnt_blend {
                let destroy = fetch.get_str("destroy");
                if destroy.is_nil()
                    || cx.num(&destroy, "getTargetAlpha", &[])? <= cx.num(v, "getTargetAlpha", &[])?
                {
                    fetch.set_str("destroy", v.clone());
                }
            }
        }
        let square = cx.call(v, "getSquare", &[])?;
        if can_clean_blood(cx, &player_obj, &square)? {
            fetch.set_str("haveBlood", square.clone());
        }
        if can_clean_graffiti(cx, &player_obj, &square)? {
            fetch.set_str("haveGraffiti", square);
        }
    }
    if instance_of(v, "IsoPlayer") && !v.raw_equals(&player_obj) {
        fetch.set_str("clickedPlayer", v.clone());
    }
    if instance_of(v, "IsoAnimal") {
        let animals = table_at(fetch, "clickedAnimals");
        if !contains_value(animals.as_ref(), v) {
            append(animals.as_ref().ok_or_else(super::null_pointer)?, v.clone());
        }
    }
    if cx.num(v, "getPipedFuelAmount", &[])? > 0.0 {
        if !cx.first_recurse(&inventory, |cx, i| predicate_store_fuel(cx, i))?.is_nil() {
            fetch.set_str("haveFuel", v.clone());
        }
        fetch.set_str("haveFuelDebug", v.clone());
    }
    if !cx.call(v, "getSprite", &[])?.is_nil() {
        let sprite = cx.call(v, "getSprite", &[])?;
        let sprite_props = cx.call(&sprite, "getProperties", &[])?;
        if cx.is(&sprite_props, "has", &[Value::str("fuelAmount")])? {
            fetch.set_str("fuelPump", v.clone());
        }
    }
    if instance_of(v, "IsoFeedingTrough") {
        let master = cx.call(v, "getMasterTrough", &[])?;
        if !master.is_nil() && cx.is(&master, "hasComponent", &[fluid_container.clone()])? {
            let containers = table_at(fetch, "fluidcontainer");
            if !contains_value(containers.as_ref(), &master) {
                append(containers.as_ref().ok_or_else(super::null_pointer)?, master);
            }
        }
    } else if cx.is(v, "hasComponent", &[fluid_container])? {
        let containers = table_at(fetch, "fluidcontainer");
        if !contains_value(containers.as_ref(), v) {
            append(containers.as_ref().ok_or_else(super::null_pointer)?, v.clone());
        }
    }
    if do_square {
        let square = cx.call(v, "getSquare", &[])?;
        let safehouse = cx.static_call("SafeHouse", "getSafeHouse", &[square.clone()])?;
        fetch.set_str("safehouse", safehouse);
        let allow = cx.static_call("SafeHouse", "isSafehouseAllowInteract", &[square.clone(), player_obj.clone()])?;
        fetch.set_str("safehouseAllowInteract", allow.truthy());
        let allow = cx.static_call("SafeHouse", "isSafehouseAllowLoot", &[square, player_obj.clone()])?;
        let allow = allow.truthy();
        fetch.set_str("safehouseAllowLoot", allow);
    }
    // GameTime.getWorldAgeHours() / 24 + (timeSinceApo - 1) * 30
    //     < waterShutModifier
    let game_time = cx.global("getGameTime", &[])?;
    let age_days = cx.num(&game_time, "getWorldAgeHours", &[])? / 24.0;
    let sandbox = cx.global("getSandboxOptions", &[])?;
    let since_apo = cx.num(&sandbox, "getTimeSinceApo", &[])?;
    let water_shut = cx.call(&sandbox, "getOptionByName", &[Value::str("WaterShutModifier")])?;
    let water_shut = cx.num(&water_shut, "getValue", &[])?;
    let water_on = age_days + f64::from((since_apo as i32 - 1) * 30) < water_shut;
    let can_be_piped = {
        let mod_data = cx.call(v, "getModData", &[])?;
        let table = mod_data.as_table().ok_or_else(super::null_pointer)?;
        matches!(table.get_str("canBeWaterPiped"), Value::Bool(true))
    };
    if cx.is(v, "hasModData", &[])? {
        let square = cx.call(v, "getSquare", &[])?;
        if !square.is_nil()
            && can_be_piped
            && cx.is(&square, "isInARoom", &[])?
            && !cx.call(v, "FindExternalWaterSource", &[])?.is_nil()
        {
            fetch.set_str("canBeWaterPiped", v.clone());
        }
    }
    if !props.is_nil() {
        let square = cx.call(v, "getSquare", &[])?;
        if !square.is_nil() && cx.has_flag(&props, "waterPiped")? && !cx.is(v, "getUsesExternalWaterSource", &[])? {
            let piped = (cx.is(&square, "isInARoom", &[])? && !cx.call(v, "FindExternalWaterSource", &[])?.is_nil())
                || (!cx.call(&square, "getRoom", &[])?.is_nil() && water_on && can_be_piped);
            if piped {
                fetch.set_str("canBeWaterPiped", v.clone());
            }
        }
        fetch_pickup_items(cx, fetch, v, &props, &inventory)?;
    }
    if instance_of(v, "IHasHealth") {
        fetch.set_str("health", v.clone());
    }
    fetch.set_str("item", v.clone());
    if do_square {
        let square = cx.call(v, "getSquare", &[])?;
        let seen = fetch_squares.as_ref().ok_or_else(super::null_pointer)?;
        if !square.is_nil() && seen.rawget(&square).is_nil() {
            let objects = cx.call(&square, "getObjects", &[])?;
            let mut i = 0;
            while (i as f64) < cx.num(&objects, "size", &[])? {
                let object = cx.call(&objects, "get", &[Value::Num(i as f64)])?;
                self::fetch(cx, fetch, &object, player, false)?;
                i += 1;
            }
            let statics = cx.call(&square, "getStaticMovingObjects", &[])?;
            let mut i = 0;
            while (i as f64) < cx.num(&statics, "size", &[])? {
                let object = cx.call(&statics, "get", &[Value::Num(i as f64)])?;
                self::fetch(cx, fetch, &object, player, false)?;
                i += 1;
            }
            let animals = table_at(fetch, "clickedAnimals");
            let sx = cx.num(&square, "getX", &[])? as i32;
            let sy = cx.num(&square, "getY", &[])? as i32;
            let sz = cx.num(&square, "getZ", &[])?;
            let cell = cx.call(&square, "getCell", &[])?;
            for x in sx - 1..=sx + 1 {
                for y in sy - 1..=sy + 1 {
                    let near = cx.call(&cell, "getGridSquare", &[Value::Num(x as f64), Value::Num(y as f64), Value::Num(sz)])?;
                    if near.is_nil() {
                        continue;
                    }
                    let moving = cx.call(&near, "getMovingObjects", &[])?;
                    let n = cx.num(&moving, "size", &[])? as i64;
                    for i in 0..n {
                        let object = cx.call(&moving, "get", &[Value::Num(i as f64)])?;
                        if instance_of(&object, "IsoPlayer") && !object.raw_equals(&player_obj) {
                            fetch.set_str("clickedPlayer", object.clone());
                        }
                        if instance_of(&object, "IsoAnimal") && !contains_value(animals.as_ref(), &object) {
                            append(animals.as_ref().ok_or_else(super::null_pointer)?, object);
                        }
                    }
                }
            }
        }
    }
    if do_square {
        let square = cx.call(v, "getSquare", &[])?;
        let args = [
            cx.call(&square, "getX", &[])?,
            cx.call(&square, "getY", &[])?,
            cx.call(&square, "getZ", &[])?,
        ];
        let zone = cx.static_call("DesignationZoneAnimal", "getZone", &args)?;
        fetch.set_str("animalZone", zone);
        fetch_squares.as_ref().ok_or_else(super::null_pointer)?.rawset(square, Value::Bool(true));
    }
    Ok(())
}

/// `String.valueOf(o)` for the tile name concatenation.
fn java_string(v: &Value) -> String {
    match v {
        Value::Nil => "null".into(),
        other => pz_kahlua::exec::java_value_of(other).to_string(),
    }
}

/// The original's "first slot, else a different second slot" pattern.
fn set_first_or_second(fetch: &TableRef, key: &str, key2: &str, v: &Value) {
    let first = fetch.get_str(key);
    if first.is_nil() {
        fetch.set_str(key, v.clone());
    } else if !first.raw_equals(v) && !fetch.get_str(key2).raw_equals(v) {
        fetch.set_str(key2, v.clone());
    }
}

/// `v.getSquare().DistToProper(player.getSquare())`.
fn square_dist(cx: &mut Cx<'_>, v: &Value, player: &Value) -> LuaResult<f64> {
    let square = cx.call(v, "getSquare", &[])?;
    let player_square = cx.call(player, "getSquare", &[])?;
    cx.num(&square, "DistToProper", &[player_square])
}

/// `lambda$fetch$2/3`: `!item.isBroken() && compareType(type, item)`.
fn sledgehammer(cx: &mut Cx<'_>, item: &Value, kind: &str) -> LuaResult<bool> {
    Ok(!cx.is(item, "isBroken", &[])? && compare_type_item(cx, kind, item)?)
}

/// The sprite tests that decide whether a sledgehammer can destroy `v`.
fn destroyable(cx: &mut Cx<'_>, v: &Value) -> LuaResult<bool> {
    for flag in ["solidtrans", "collideW", "collideN", "bed"] {
        if cx.sprite_has(v, flag)? {
            return Ok(true);
        }
    }
    if instance_of(v, "IsoThumpable") {
        return Ok(true);
    }
    for flag in ["windowN", "windowW"] {
        if cx.sprite_has(v, flag)? {
            return Ok(true);
        }
    }
    let kind = cx.call(v, "getType", &[])?;
    for stairs in ["stairsBN", "stairsMN", "stairsTN", "stairsBW", "stairsMW", "stairsTW"] {
        if kind.raw_equals(&cx.static_field("IsoObjectType", stairs)) {
            return Ok(true);
        }
    }
    let props = cx.call(v, "getProperties", &[])?;
    let door_wall = cx.is(&props, "has", &[Value::str("DoorWallN")])? || {
        let props = cx.call(v, "getProperties", &[])?;
        cx.is(&props, "has", &[Value::str("DoorWallW")])?
    };
    if door_wall {
        let square = cx.call(v, "getSquare", &[])?;
        if !cx.is(&square, "haveDoor", &[])? {
            return Ok(true);
        }
    }
    cx.sprite_has(v, "waterPiped")
}

/// `fetchThrowCorpseOver(player, fetch, key)`.
fn fetch_throw_corpse_over(cx: &mut Cx<'_>, player: &Value, fetch: &TableRef, key: &str) -> LuaResult<()> {
    if !fetch.get_str("throwCorpseOver").is_nil() {
        return Ok(());
    }
    let object = fetch.get_str(key);
    if !instance_of(&object, "IsoObject") {
        return Ok(());
    }
    let hoppable_n = cx.static_field("IsoFlagType", "HoppableN");
    let (square, dir) = if cx.is(&object, "hasProperty", &[hoppable_n])? {
        if cx.num(&object, "getY", &[])? > cx.num(player, "getY", &[])? {
            let square = cx.call(&object, "getSquare", &[])?;
            let n = cx.static_field("IsoDirections", "N");
            (cx.call(&square, "getAdjacentSquare", &[n])?, cx.static_field("IsoDirections", "S"))
        } else {
            (cx.call(&object, "getSquare", &[])?, cx.static_field("IsoDirections", "N"))
        }
    } else if cx.num(&object, "getX", &[])? > cx.num(player, "getX", &[])? {
        let square = cx.call(&object, "getSquare", &[])?;
        let w = cx.static_field("IsoDirections", "W");
        (cx.call(&square, "getAdjacentSquare", &[w])?, cx.static_field("IsoDirections", "E"))
    } else {
        (cx.call(&object, "getSquare", &[])?, cx.static_field("IsoDirections", "W"))
    };
    if cx.is(player, "canThrowCorpseOver", &[square, dir])? {
        fetch.set_str("throwCorpseOver", object);
    }
    Ok(())
}

/// `fetchPickupItems(fetch, v, props, inventory)`.
fn fetch_pickup_items(cx: &mut Cx<'_>, fetch: &TableRef, v: &Value, props: &Value, inventory: &Value) -> LuaResult<()> {
    if !cx.call(v, "getContainer", &[])?.is_nil() {
        fetch.set_str("container", v.clone());
    }
    let custom_name = cx.call(props, "get", &[Value::str("CustomName")])?;
    let Some(custom_name) = custom_name.as_str().map(str::to_owned) else {
        return Ok(());
    };
    if !cx.is(props, "has", &[Value::str("IsMoveAble")])? && fetch.get_str("pickupItem").is_nil() {
        let script_manager = cx.global("getScriptManager", &[])?;
        if !cx.call(&script_manager, "getItem", &[Value::str(&custom_name)])?.is_nil() {
            fetch.set_str("pickupItem", v.clone());
        } else {
            let ground_cover = match cx.t.globals().rawget(&Value::str("GroundCoverItems")) {
                Value::Table(t) => t,
                _ => return Err(super::null_pointer()),
            };
            let item = ground_cover.get_str(&custom_name);
            if !item.is_nil() && !cx.call(&script_manager, "getItem", &[item])?.is_nil() {
                fetch.set_str("pickupItem", v.clone());
            }
        }
    }
    if !cx.first_recurse(inventory, |cx, i| cx.not_broken_with_tag(i, "REMOVE_STUMP"))?.is_nil()
        && cx.is(v, "isStump", &[])?
    {
        fetch.set_str("stump", v.clone());
    }
    let square = cx.call(v, "getSquare", &[])?;
    if !cx.call(&square, "getOre", &[])?.is_nil()
        && !cx
            .first_recurse(inventory, |cx, i| {
                if cx.is(i, "isBroken", &[])? {
                    return Ok(false);
                }
                for tag in ["PICK_AXE", "STONE_MAUL", "SLEDGEHAMMER", "CLUB_HAMMER"] {
                    let tag = cx.item_tag(tag);
                    if cx.is(i, "hasTag", &[tag])? {
                        return Ok(true);
                    }
                }
                Ok(false)
            })?
            .is_nil()
        && (custom_name.contains("ironOre")
            || custom_name.contains("copperOre")
            || custom_name == "FlintBoulder"
            || custom_name == "LimestoneBoulder")
    {
        fetch.set_str("ore", v.clone());
    }
    Ok(())
}

/// `isGraveFilledIn(v)`.
fn is_grave_filled_in(cx: &mut Cx<'_>, v: &Value) -> LuaResult<bool> {
    if v.is_nil() || cx.call(v, "getName", &[])?.as_str() != Some("EmptyGraves") {
        return Ok(false);
    }
    let mod_data = cx.call(v, "getModData", &[])?;
    if matches!(mod_data.as_table().map(|t| t.get_str("filled")), Some(Value::Bool(true))) {
        return Ok(true);
    }
    let sprite = cx.call(v, "getSprite", &[])?;
    if sprite.is_nil() {
        return Ok(false);
    }
    let name = cx.call(&sprite, "getName", &[])?;
    Ok(matches!(
        name.as_str(),
        Some(
            "location_community_cemetary_01_40"
                | "location_community_cemetary_01_41"
                | "location_community_cemetary_01_42"
                | "location_community_cemetary_01_43"
        )
    ))
}

/// `canCleanBlood(player, square)`.
fn can_clean_blood(cx: &mut Cx<'_>, player: &Value, square: &Value) -> LuaResult<bool> {
    let inventory = cx.call(player, "getInventory", &[])?;
    let tool = cx.first_recurse(&inventory, |cx, i| cx.not_broken_with_tag(i, "CLEAN_STAINS"))?;
    let liquid = cx.first_recurse(&inventory, |cx, i| predicate_cleaning_liquid(cx, i))?;
    Ok(!square.is_nil() && cx.is(square, "haveStains", &[])? && !tool.is_nil() && !liquid.is_nil())
}

/// `canCleanGraffiti(player, square)`.
fn can_clean_graffiti(cx: &mut Cx<'_>, player: &Value, square: &Value) -> LuaResult<bool> {
    let inventory = cx.call(player, "getInventory", &[])?;
    let petrol = cx.first_recurse(&inventory, |cx, i| {
        let fluid = cx.call(i, "getFluidContainer", &[])?;
        if fluid.is_nil() || cx.num(&fluid, "getAmount", &[])? < 1.25 {
            return Ok(false);
        }
        let petrol = cx.static_field("Fluid", "Petrol");
        cx.is(&fluid, "contains", &[petrol])
    })?;
    let tool = cx.first_recurse(&inventory, |cx, i| cx.not_broken_with_tag(i, "CLEAN_STAINS"))?;
    Ok(!square.is_nil() && cx.is(square, "haveGraffiti", &[])? && !petrol.is_nil() && !tool.is_nil())
}

/// `predicateCleaningLiquid(item)`.
fn predicate_cleaning_liquid(cx: &mut Cx<'_>, item: &Value) -> LuaResult<bool> {
    if item.is_nil() {
        return Ok(false);
    }
    let fluid_container = cx.static_field("ComponentType", "FluidContainer");
    if !cx.is(item, "hasComponent", &[fluid_container])? {
        return Ok(false);
    }
    let fluid = cx.call(item, "getFluidContainer", &[])?;
    let bleach = cx.static_field("Fluid", "Bleach");
    let cleaning = cx.static_field("Fluid", "CleaningLiquid");
    if !cx.is(&fluid, "contains", &[bleach])? && !cx.is(&fluid, "contains", &[cleaning])? {
        return Ok(false);
    }
    let amount = cx.num(&fluid, "getAmount", &[])? as f32 as f64;
    let globals = cx.t.globals().rawget(&Value::str("ZomboidGlobals"));
    let needed = match &globals {
        Value::Table(t) => t.get_str("CleanBloodBleachAmount").as_num().unwrap_or(0.0),
        _ => 0.0,
    };
    Ok(amount >= needed)
}

/// `predicateStoreFuel(item)`.
pub(super) fn predicate_store_fuel(cx: &mut Cx<'_>, item: &Value) -> LuaResult<bool> {
    let fluid = cx.call(item, "getFluidContainer", &[])?;
    if fluid.is_nil() {
        return Ok(false);
    }
    if cx.is(&fluid, "isEmpty", &[])? {
        return Ok(true);
    }
    let petrol = cx.static_field("Fluid", "Petrol");
    Ok(cx.is(&fluid, "contains", &[petrol])?
        && cx.num(&fluid, "getAmount", &[])? < cx.num(&fluid, "getCapacity", &[])?
        && !cx.is(item, "isBroken", &[])?)
}

/// `getDirtGravelSand(square)`.
fn get_dirt_gravel_sand(cx: &mut Cx<'_>, square: &Value) -> LuaResult<Value> {
    let mut i = cx.num(square, "getNextNonItemObjectIndex", &[Value::Num(0.0)])? as i64;
    loop {
        let objects = cx.call(square, "getObjects", &[])?;
        if i < 0 || i as f64 >= cx.num(&objects, "size", &[])? {
            return Ok(Value::Nil);
        }
        let object = cx.call(&objects, "get", &[Value::Num(i as f64)])?;
        i = cx.num(square, "getNextNonItemObjectIndex", &[Value::Num((i + 1) as f64)])? as i64;
        if cx.is(&object, "hasModData", &[])? {
            let mod_data = cx.call(&object, "getModData", &[])?;
            if matches!(mod_data.as_table().map(|t| t.get_str("shovelled")), Some(Value::Bool(true))) {
                continue;
            }
        }
        if !farming_object_on_square(cx, square)?.is_nil() {
            continue;
        }
        let sprite = cx.call(&object, "getSprite", &[])?;
        if sprite.is_nil() {
            continue;
        }
        let name = cx.call(&sprite, "getName", &[])?;
        let Some(name) = name.as_str() else { continue };
        let kind = match name {
            "floors_exterior_natural_01_13"
            | "blends_street_01_55"
            | "blends_street_01_54"
            | "blends_street_01_53"
            | "blends_street_01_48" => "gravel",
            "blends_natural_01_0"
            | "blends_natural_01_5"
            | "blends_natural_01_6"
            | "blends_natural_01_7"
            | "floors_exterior_natural_01_24" => "sand",
            "blends_natural_01_96" | "blends_natural_01_103" | "blends_natural_01_101" | "blends_natural_01_102" => {
                "clay"
            }
            n if n.starts_with("blends_natural_01_") || n.starts_with("floors_exterior_natural") => "dirt",
            _ => continue,
        };
        let t = TableRef::new();
        t.set_str(kind, object);
        return Ok(Value::Table(t));
    }
}

/// `farmingSystem_getLuaObjectOnSquare(square)` (client side): the farming
/// system's object at the square, refreshed from its IsoObject.
fn farming_object_on_square(cx: &mut Cx<'_>, square: &Value) -> LuaResult<Value> {
    if square.is_nil() {
        return Ok(Value::Nil);
    }
    let system = cx.static_call("CGlobalObjects", "getSystemByName", &[Value::str("CFarmingSystem")])?;
    if system.is_nil() {
        return Ok(Value::Nil);
    }
    let args = [cx.call(square, "getX", &[])?, cx.call(square, "getY", &[])?, cx.call(square, "getZ", &[])?];
    let object = cx.call(&system, "getObjectAt", &args)?;
    if object.is_nil() {
        return Ok(Value::Nil);
    }
    let mod_data = cx.call(&object, "getModData", &[])?;
    let update = cx.func("CGlobalObject.updateFromIsoObject");
    cx.t.call(&update, &[mod_data.clone()])?;
    Ok(mod_data)
}

/// `compareType(type, item)`.
pub(super) fn compare_type_item(cx: &mut Cx<'_>, kind: &str, item: &Value) -> LuaResult<bool> {
    if !kind.contains('.') {
        let t = cx.call(item, "getType", &[])?;
        return Ok(compare_type(kind, t.as_str().unwrap_or("")));
    }
    let full = cx.call(item, "getFullType", &[])?;
    if compare_type(kind, full.as_str().unwrap_or("")) {
        return Ok(true);
    }
    let t = cx.call(item, "getType", &[])?;
    Ok(compare_type(kind, t.as_str().unwrap_or("")))
}

/// `compareType(type1, type2)`: equality, or `type2` as one `/`-separated
/// entry of `type1`.
fn compare_type(type1: &str, type2: &str) -> bool {
    if !type1.contains('/') {
        return type1 == type2;
    }
    let Some(at) = type1.find(type2) else { return false };
    let bytes = type1.as_bytes();
    let before = if at > 0 { bytes[at - 1] } else { 0 };
    let after = bytes.get(at + type2.len()).copied().unwrap_or(0);
    (before == 0 && after == b'/') || (before == b'/' && after == 0) || (before == b'/' && after == b'/')
}

#[cfg(test)]
pub(super) fn compare_type_for_test(a: &str, b: &str) -> bool {
    compare_type(a, b)
}

/// `FireFighting.getSquareToExtinguish(square)`: the square, else the
/// first of its eight neighbours that has a fire to put out.
fn square_to_extinguish(cx: &mut Cx<'_>, square: &Value) -> LuaResult<Value> {
    if is_square_to_extinguish(cx, square)? {
        return Ok(square.clone());
    }
    let x = cx.num(square, "getX", &[])?;
    let y = cx.num(square, "getY", &[])?;
    let z = cx.num(square, "getZ", &[])?;
    let cell = cx.global("getCell", &[])?;
    for dy in -1..=1 {
        for dx in -1..=1 {
            if dx == 0 && dy == 0 {
                continue;
            }
            let near = cx.call(&cell, "getGridSquare", &[Value::Num(x + f64::from(dx)), Value::Num(y + f64::from(dy)), Value::Num(z)])?;
            if is_square_to_extinguish(cx, &near)? {
                return Ok(near);
            }
        }
    }
    Ok(Value::Nil)
}

/// `FireFighting.isSquareToExtinguish(square)`: a burning square with a
/// fire that is not permanent, or a character on fire on the square.
fn is_square_to_extinguish(cx: &mut Cx<'_>, square: &Value) -> LuaResult<bool> {
    if square.is_nil() {
        return Ok(false);
    }
    // IsoGridSquare.has(flag) reads the square's properties.
    let props = cx.call(square, "getProperties", &[])?;
    if cx.has_flag(&props, "burning")? {
        let objects = cx.call(square, "getObjects", &[])?;
        let n = cx.num(&objects, "size", &[])? as i64;
        for i in 0..n {
            let object = cx.call(&objects, "get", &[Value::Num(i as f64)])?;
            if instance_of(&object, "IsoFire") && !cx.is(&object, "isPermanent", &[])? {
                return Ok(true);
            }
        }
    }
    let moving = cx.call(square, "getMovingObjects", &[])?;
    let n = cx.num(&moving, "size", &[])? as i64;
    for i in 0..n {
        let object = cx.call(&moving, "get", &[Value::Num(i as f64)])?;
        if instance_of(&object, "IsoGameCharacter") && cx.is(&object, "isOnFire", &[])? {
            return Ok(true);
        }
    }
    Ok(false)
}
