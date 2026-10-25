//! Builds `ItemContainer`s from a plan a local harness writes for the
//! original classes (items made with `InventoryItemFactory.CreateItem`,
//! `Rand` seeded as `new java.util.Random(1000 + n)` per item, a bag with
//! items inside) and prints the container query results through the Lua
//! binding, in the harness's format. No game data or harness output is
//! part of this repository.
//!
//! ```text
//! container_parity <game dir> <plan file> <out tsv>
//! ```
//!
//! Plan lines: `C <n>` starts a container, `I <type>` adds an item, `X
//! <type>` is an item the original could not create, `B <type>` adds a bag,
//! `N <type>` puts an item in the last bag, `T <type>` is a second bag made
//! after the queries for the bag-to-bag transfers, `Q type|tag <arg>`
//! lists the query arguments.

use pz_inventory::shared::{self, SharedContainer};
use pz_kahlua::{TableRef, Thread, Value};
use pz_lua_api::bind_item_container::container_value;
use pz_lua_api::bind_items::{create_item, install_item_env, item_value, GameItemEnv, ItemRef};
use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

const QUERIES: &str = r#"
local function idx(c, it)
    if it == nil then return -1 end
    local l = c:getItems()
    for i = 0, l:size() - 1 do if l:get(i) == it then return i end end
    return "nested"
end
local function q(name, f)
    local ok, v = pcall(f)
    if not ok then v = "ERR" end
    emit(name, v)
end
local cats = {"Item", "Food", "Weapon", "Clothing", "Container", "Literature", "Key", "WeaponPart", "AlarmClock", "Animal", "Drainable"}
function runContainer(c, types, tags)
    q("contentsWeight", function() return c:getContentsWeight() end)
    q("capacityWeight", function() return c:getCapacityWeight() end)
    q("maxWeight", function() return c:getMaxWeight() end)
    q("availableWeightCapacity", function() return c:getAvailableWeightCapacity() end)
    q("capacity", function() return c:getCapacity() end)
    q("effectiveCapacity", function() return c:getEffectiveCapacity(nil) end)
    q("freeCapacity", function() return c:getFreeCapacity(nil) end)
    q("isFull", function() return c:isFull(nil) end)
    q("hasRoomFor1.5", function() return c:hasRoomFor(nil, 1.5) end)
    q("hasRoomFor40", function() return c:hasRoomFor(nil, 40) end)
    q("hasRoomForItem0", function() return c:hasRoomFor(nil, c:getItems():get(0)) end)
    q("isEmpty", function() return c:isEmpty() end)
    q("size", function() return c:getItems():size() end)
    q("bestFood", function() return idx(c, c:getBestFood(nil)) end)
    q("totalFoodScore", function() return c:getTotalFoodScore(nil) end)
    for w = 0, c:getItems():size() - 1 do
        q("itemWeight " .. w, function() return c:getItems():get(w):getUnequippedWeight() end)
    end
    q("waterContainerCount", function() return c:getWaterContainerCount() end)
    q("findWaterSource", function() return idx(c, c:FindWaterSource()) end)
    q("findAndReturnWaterItem0", function() return idx(c, c:FindAndReturnWaterItem(0)) end)
    q("findAndReturnWaterItem3", function() return idx(c, c:FindAndReturnWaterItem(3)) end)
    q("allWaterFillables", function() return c:getAllWaterFillables():size() end)
    q("allWaterFluidSources", function() return c:getAllWaterFluidSources(false):size() end)
    q("allWaterFluidSourcesT", function() return c:getAllWaterFluidSources(true):size() end)
    q("firstWaterFluidSources", function() return idx(c, c:getFirstWaterFluidSources(false)) end)
    q("firstWaterFluidSourcesT", function() return idx(c, c:getFirstWaterFluidSources(true)) end)
    q("firstWaterFluidSourcesTP", function() return idx(c, c:getFirstWaterFluidSources(true, true)) end)
    q("allCleaningFluidSources", function() return c:getAllCleaningFluidSources():size() end)
    q("firstCleaningFluidSources", function() return idx(c, c:getFirstCleaningFluidSources()) end)
    q("soapList", function() return c:getSoapList(nil, false):size() end)
    q("soapListLiquid", function() return c:getSoapList(nil, true):size() end)
    local fls = {"null", "", "Water", "TaintedWater", "Bleach", "CleaningLiquid", "Petrol", "Beer", "water"}
    for n = 1, #fls do
        local fl = fls[n]
        local a = fl
        if fl == "null" then a = nil end
        q("firstFluidContainer " .. fl, function() return idx(c, c:getFirstFluidContainer(a)) end)
        q("availableFluidContainer " .. fl, function() return c:getAvailableFluidContainer(a):size() end)
        q("availableFluidContainersCapacity " .. fl, function() return c:getAvailableFluidContainersCapacity(a) end)
        q("firstAvailableFluidContainer " .. fl, function() return idx(c, c:getFirstAvailableFluidContainer(a)) end)
    end
    q("isItemAllowed0", function() return c:isItemAllowed(c:getItems():get(0)) end)
    for n = 1, #types do
        local t = types[n]
        q("countType " .. t, function() return c:getCountType(t) end)
        q("countTypeRecurse " .. t, function() return c:getCountTypeRecurse(t) end)
        q("itemCount " .. t, function() return c:getItemCount(t) end)
        q("itemCountBags " .. t, function() return c:getItemCount(t, true) end)
        q("itemCountRecurse " .. t, function() return c:getItemCountRecurse(t) end)
        q("numberOfItem " .. t, function() return c:getNumberOfItem(t) end)
        q("numberOfItemInside " .. t, function() return c:getNumberOfItem(t, true, true) end)
        q("usesType " .. t, function() return c:getUsesType(t) end)
        q("usesTypeRecurse " .. t, function() return c:getUsesTypeRecurse(t) end)
        q("containsType " .. t, function() return c:containsType(t) end)
        q("containsTypeRecurse " .. t, function() return c:containsTypeRecurse(t) end)
        q("contains " .. t, function() return c:contains(t) end)
        q("containsDoInv " .. t, function() return c:contains(t, true) end)
        q("itemsFromType " .. t, function() return c:getItemsFromType(t):size() end)
        q("itemsFromFullType " .. t, function() return c:getItemsFromFullType(t):size() end)
        q("allType " .. t, function() return c:getAllType(t):size() end)
        q("someType2 " .. t, function() return c:getSomeType(t, 2):size() end)
        q("firstType " .. t, function() return idx(c, c:getFirstType(t)) end)
        q("firstTypeRecurse " .. t, function() return idx(c, c:getFirstTypeRecurse(t)) end)
        q("findAndReturn " .. t, function() return idx(c, c:FindAndReturn(t)) end)
        q("bestCondition " .. t, function() return idx(c, c:getBestCondition(t)) end)
        q("numItems " .. t, function() return c:getNumItems(t) end)
        q("countFromTypeRecurse " .. t, function() return c:getItemCountFromTypeRecurse(t) end)
        q("itemFromType3 " .. t, function() return idx(c, c:getItemFromType(t, false, false)) end)
        q("containsWithModule " .. t, function() return c:containsWithModule(t) end)
        q("findItem " .. t, function() return idx(c, c:findItem(t, false, true)) end)
    end
    for n = 1, #cats do
        local k = cats[n]
        q("allCategory " .. k, function() return c:getAllCategory(k):size() end)
        q("firstCategory " .. k, function() return idx(c, c:getFirstCategory(k)) end)
        q("findAndReturnCategory " .. k, function() return idx(c, c:FindAndReturnCategory(k)) end)
        q("someCategory2 " .. k, function() return c:getSomeCategory(k, 2):size() end)
        q("itemsFromCategory " .. k, function() return c:getItemsFromCategory(k):size() end)
    end
    for n = 1, #tags do
        local g = tags[n]
        q("containsTag " .. g, function() return c:containsTag(g) end)
        q("containsTagRecurse " .. g, function() return c:containsTagRecurse(g) end)
        q("countTag " .. g, function() return c:getCountTag(g) end)
        q("countTagRecurse " .. g, function() return c:getCountTagRecurse(g) end)
        q("firstTag " .. g, function() return idx(c, c:getFirstTag(g)) end)
        q("firstTagRecurse " .. g, function() return idx(c, c:getFirstTagRecurse(g)) end)
        q("allTag " .. g, function() return c:getAllTag(g):size() end)
        q("itemFromTag " .. g, function() return idx(c, c:getItemFromTag(g, false, false)) end)
    end
end
local function where(o, c, b1, b2)
    if o == nil then return "null" end
    if o == c then return "cont" end
    if o == b1 then return "b1" end
    if o == b2 then return "b2" end
    return "other"
end
function runXfer(c, bag1, bag2, nested)
    local b1, b2 = bag1:getInventory(), bag2:getInventory()
    for st = 0, 3 do
        if st == 1 and #nested > 0 then b2:AddItem(nested[1]) end
        if st == 2 then b1:AddItem(bag2) end
        if st == 3 then
            if #nested > 0 then q("xfer removeIdRecurse", function() return c:removeItemWithIDRecurse(nested[1]:getID()) end) end
            if #nested > 1 then q("xfer removeOneOfInside", function()
                local r = c:RemoveOneOf(nested[2]:getFullType(), true)
                if r == bag1 then return "bag1" elseif r == nil then return "null" end
                return "other"
            end) end
        end
        q("xfer " .. st .. " contSize", function() return c:getItems():size() end)
        q("xfer " .. st .. " b1Size", function() return b1:getItems():size() end)
        q("xfer " .. st .. " b2Size", function() return b2:getItems():size() end)
        q("xferf " .. st .. " b1Contents", function() return b1:getContentsWeight() end)
        q("xferf " .. st .. " b2Contents", function() return b2:getContentsWeight() end)
        q("xferf " .. st .. " b1Unequipped", function() return bag1:getUnequippedWeight() end)
        q("xferf " .. st .. " b2Unequipped", function() return bag2:getUnequippedWeight() end)
        q("xferf " .. st .. " b1Equipped", function() return bag1:getEquippedWeight() end)
        q("xferf " .. st .. " b2Equipped", function() return bag2:getEquippedWeight() end)
        q("xferf " .. st .. " b1InvWeight", function() return bag1:getInventoryWeight() end)
        q("xferf " .. st .. " b2InvWeight", function() return bag2:getInventoryWeight() end)
        q("xferf " .. st .. " contContents", function() return c:getContentsWeight() end)
        q("xferf " .. st .. " b1CapWeight", function() return b1:getCapacityWeight() end)
        q("xferf " .. st .. " b1Free", function() return b1:getFreeCapacity(nil) end)
        q("xferf " .. st .. " b2Free", function() return b2:getFreeCapacity(nil) end)
        q("xfer " .. st .. " b1Cap", function() return bag1:getCapacity() end)
        q("xfer " .. st .. " b2Cap", function() return bag2:getCapacity() end)
        q("xfer " .. st .. " b2EffCap", function() return bag2:getEffectiveCapacity(nil) end)
        q("xfer " .. st .. " b2InvCap", function() return b2:getCapacity() end)
        q("xfer " .. st .. " b2Room5", function() return b2:hasRoomFor(nil, 5) end)
        q("xfer " .. st .. " b2Room30", function() return b2:hasRoomFor(nil, 30) end)
        q("xfer " .. st .. " b2RoomItem0", function() return b2:hasRoomFor(nil, c:getItems():get(0)) end)
        q("xfer " .. st .. " b2Containing", function() return b2:getContainingItem() == bag2 end)
        q("xfer " .. st .. " b2Outer", function() return where(b2:getOutermostContainer(), c, b1, b2) end)
        q("xfer " .. st .. " bag2Container", function() return where(bag2:getContainer(), c, b1, b2) end)
        q("xfer " .. st .. " b1InCont", function() return c:contains(bag2, false) end)
        q("xfer " .. st .. " b1InContInv", function() return c:contains(bag2, true) end)
        if #nested > 0 then
            local n0 = nested[1]
            q("xfer " .. st .. " n0Container", function() return where(n0:getContainer(), c, b1, b2) end)
            q("xfer " .. st .. " n0Outer", function() return where(n0:getOutermostContainer(), c, b1, b2) end)
            q("xfer " .. st .. " n0Recurse", function() return c:getFirstTypeRecurse(n0:getFullType()) == n0 end)
            q("xfer " .. st .. " n0Count", function() return c:getCountTypeRecurse(n0:getFullType()) end)
            q("xfer " .. st .. " n0ById", function() return c:getItemWithIDRecursiv(n0:getID()) == n0 end)
            q("xfer " .. st .. " n0InCharInv", function() return b2:isInside(bag1) end)
        end
    end
end
"#;

/// Queries the harness prints as `Float.floatToIntBits` hex.
const FLOATS: [&str; 9] = [
    "itemWeight",
    "xferf",
    "availableFluidContainersCapacity",
    "contentsWeight",
    "capacityWeight",
    "maxWeight",
    "availableWeightCapacity",
    "freeCapacity",
    "totalFoodScore",
];

fn format(name: &str, v: &Value) -> String {
    match v {
        Value::Bool(b) => b.to_string(),
        Value::Num(n) if FLOATS.contains(&name.split(' ').next().unwrap_or("")) => {
            format!("{:x}", (*n as f32).to_bits())
        }
        Value::Num(n) => format!("{}", *n as i64),
        other => pz_kahlua::exec::java_value_of(other),
    }
}

/// A number field of the `ZomboidGlobals` table in the game's Lua.
fn zomboid_global(game: &std::path::Path, name: &str) -> Result<f64, String> {
    let src = std::fs::read_to_string(game.join("media/lua/shared/defines.lua"))
        .map_err(|e| e.to_string())?;
    let mut t = Thread::new(TableRef::new());
    pz_kahlua::stdlib::install(&mut t, None).map_err(|e| format!("{e:?}"))?;
    let f = pz_kahlua::stdlib::load_string(&src, Some("ZomboidGlobals"), t.globals().clone())
        .map_err(|e| format!("{e:?}"))?;
    t.pcall(&f, &[])
        .map_err(|(e, _)| pz_kahlua::exec::java_value_of(&e))?;
    match t.globals().get_str("ZomboidGlobals") {
        Value::Table(tb) => match tb.get_str(name) {
            Value::Num(n) => Ok(n),
            _ => Err(format!("ZomboidGlobals.{name} is not a number")),
        },
        _ => Err("no ZomboidGlobals table".into()),
    }
}

struct Plan {
    items: Vec<(char, String)>,
    types: Vec<String>,
    tags: Vec<String>,
}

fn main() -> Result<(), String> {
    let args: Vec<String> = std::env::args().collect();
    let game = PathBuf::from(&args[1]);
    let plan_text = std::fs::read_to_string(&args[2]).map_err(|e| e.to_string())?;
    let mut fs = pz_fs::GameFs::new(&game);
    let sm = pz_scripts::ScriptManager::new();
    sm.load_builtin_registries(&game.join("projectzomboid.jar"))
        .map_err(|e| e.to_string())?;
    sm.load(&fs, &[]).map_err(|e| e.to_string())?;
    let sm = Rc::new(sm);
    pz_lua_api::bind_registry::install_script_manager(sm.clone());
    {
        let core = pz_lua_api::bind_core::shared_core();
        pz_lua_api::bind_text::boot(&fs, &core.borrow())?;
    }
    pz_lua_api::bind_appearance::shared_population()
        .borrow_mut()
        .init(&mut fs);
    let assets = pz_inventory::ItemAssets::load(&game)?;
    let entity = Rc::new(pz_lua_api::bind_items::item_entity_context(
        &game.join("projectzomboid.jar"),
        &sm,
    )?);

    // ZomboidGlobals.Load: EquippedOrWornEncumbranceMultiplier from
    // media/lua/shared/defines.lua (the harness loads it too).
    let encumbrance = zomboid_global(&game, "EquippedOrWornEncumbranceMultiplier")?;
    let mut plans: Vec<Plan> = Vec::new();
    for l in plan_text.lines() {
        let p: Vec<&str> = l.splitn(3, '\t').collect();
        match p[0] {
            "C" => plans.push(Plan {
                items: Vec::new(),
                types: Vec::new(),
                tags: Vec::new(),
            }),
            "Q" if p[1] == "type" => plans.last_mut().unwrap().types.push(p[2].into()),
            "Q" => plans.last_mut().unwrap().tags.push(p[2].into()),
            k => plans
                .last_mut()
                .unwrap()
                .items
                .push((k.chars().next().unwrap(), p[1].into())),
        }
    }

    let mut t = Thread::new(TableRef::new());
    pz_kahlua::stdlib::install(&mut t, None).map_err(|e| format!("{e:?}"))?;
    pz_lua_api::bind_java_util::register(&mut t);
    pz_lua_api::bind_globals::register(&mut t);
    pz_lua_api::bind_registry::register(&mut t, sm);
    pz_lua_api::bind_items::register(&mut t);
    pz_lua_api::bind_item_container::register(&mut t);
    let lines: Rc<RefCell<Vec<(String, String)>>> = Rc::default();
    {
        let lines = lines.clone();
        pz_lua_api::expose::global(&mut t, "emit", move |c| {
            let name = pz_kahlua::exec::java_value_of(&c.arg(0));
            let v = format(&name, &c.arg(1));
            lines.borrow_mut().push((name, v));
            Ok(0)
        });
    }
    let f = pz_kahlua::stdlib::load_string(QUERIES, Some("queries"), t.globals().clone())
        .map_err(|e| format!("{e:?}"))?;
    t.pcall(&f, &[])
        .map_err(|(e, _)| pz_kahlua::exec::java_value_of(&e))?;

    let mut seed = 0i64;
    let mut out = String::new();
    for (k, plan) in plans.iter().enumerate() {
        let cont = SharedContainer::default().into_ref();
        let mut bag: Option<ItemRef> = None;
        let mut nested: Vec<ItemRef> = Vec::new();
        let mut second: Option<String> = None;
        for (kind, ty) in &plan.items {
            if *kind == 'T' {
                second = Some(ty.clone());
                continue;
            }
            let mut env = GameItemEnv::new(Some(assets.clone()));
            env.rand = Some(pz_core::random::Rand::new(1000 + seed));
            env.entity = Some(entity.clone());
            env.encumbrance_multiplier = Some(encumbrance);
            seed += 1;
            install_item_env(Rc::new(env));
            let item = create_item(ty, 1.0);
            match (kind, item) {
                ('X', _) => {}
                ('I', Some(i)) => shared::add_item(&cont, &i),
                ('B', Some(i)) => {
                    shared::add_item(&cont, &i);
                    bag = Some(i);
                }
                ('N', Some(i)) => {
                    let b = bag.as_ref().ok_or("no bag")?;
                    let inv = shared::inventory_of(&b.borrow()).ok_or("not a bag")?;
                    shared::add_item(&inv, &i);
                    nested.push(i);
                }
                (_, None) => return Err(format!("{ty}: not created")),
                _ => {}
            }
        }
        let list = |v: &[String]| {
            let tb = TableRef::new();
            for (i, s) in v.iter().enumerate() {
                tb.set_int(i as i64 + 1, s.as_str());
            }
            Value::Table(tb)
        };
        let run = t.globals().get_str("runContainer");
        lines.borrow_mut().clear();
        t.call(
            &run,
            &[container_value(&cont), list(&plan.types), list(&plan.tags)],
        )
        .map_err(|e| format!("{e:?}"))?;
        if let Some(ty) = second {
            let mut env = GameItemEnv::new(Some(assets.clone()));
            env.rand = Some(pz_core::random::Rand::new(1000 + seed));
            env.entity = Some(entity.clone());
            env.encumbrance_multiplier = Some(encumbrance);
            seed += 1;
            install_item_env(Rc::new(env));
            let b2 = create_item(&ty, 1.0).ok_or("second bag not created")?;
            shared::add_item(&cont, &b2);
            let n = TableRef::new();
            for (i, r) in nested.iter().enumerate() {
                n.set_int(i as i64 + 1, item_value(r));
            }
            let b1 = bag.as_ref().ok_or("no bag")?;
            let x = t.globals().get_str("runXfer");
            t.call(
                &x,
                &[
                    container_value(&cont),
                    item_value(b1),
                    item_value(&b2),
                    Value::Table(n),
                ],
            )
            .map_err(|e| format!("{e:?}"))?;
        }
        for (n, v) in lines.borrow().iter() {
            out.push_str(&format!("{k}\t{n}\t{v}\n"));
        }
    }
    std::fs::write(&args[3], out).map_err(|e| e.to_string())
}
