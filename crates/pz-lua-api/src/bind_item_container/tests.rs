use super::*;
use crate::bind_items::{install_item_env, GameItemEnv};
use pz_inventory::ItemTemplate;
use pz_kahlua::TableRef;
use pz_scripts::ScriptManager;

fn run(t: &mut Thread, src: &str) -> Result<Vec<Value>, String> {
    let f =
        pz_kahlua::stdlib::load_string(src, Some("test"), t.globals().clone()).expect("compile");
    t.pcall(&f, &[])
        .map_err(|(e, tr)| format!("{}\n{tr}", pz_kahlua::exec::java_value_of(&e)))
}

fn ok(t: &mut Thread, src: &str) {
    if let Err(e) = run(t, src) {
        panic!("lua error: {e}");
    }
}

const SCRIPTS: &str = "module Base {
    item Rock { ItemType = base:normal, Weight = 2, }
    item OldRock { ItemType = base:normal, Weight = 1, OBSOLETE = true, }
    item Apple { ItemType = base:food, Weight = 0.2, HungerChange = -10, }
}";

/// A script manager with [`SCRIPTS`] (unique temp dir per call).
fn scripts() -> Rc<ScriptManager> {
    static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir =
        std::env::temp_dir().join(format!("pz-bind-item-container-{}-{n}", std::process::id()));
    let p = dir.join("media/scripts/test.txt");
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(&p, SCRIPTS).unwrap();
    let sm = ScriptManager::new();
    for id in ["base:normal", "base:food"] {
        sm.registries
            .get("item_type")
            .unwrap()
            .borrow_mut()
            .register_id(id, true)
            .unwrap();
    }
    let mut st = sm.statics.items.borrow_mut();
    st.constants.insert("item_type", "NORMAL", "base:normal");
    st.constants.insert("item_type", "FOOD", "base:food");
    drop(st);
    sm.load(&pz_fs::GameFs::new(dir.clone()), &[]).unwrap();
    std::fs::remove_dir_all(dir).unwrap();
    Rc::new(sm)
}

fn setup() -> Thread {
    let sm = scripts();
    crate::bind_registry::install_script_manager(sm.clone());
    let mut env = GameItemEnv::new(None);
    env.rand = Some(pz_core::random::Rand::new(7));
    install_item_env(Rc::new(env));
    let mut t = Thread::new(TableRef::new());
    pz_kahlua::stdlib::install(&mut t, None).unwrap();
    ok(&mut t, "function assert(v, msg) if not v then error(msg or 'assertion failed!', 2) end return v end");
    crate::bind_java_util::register(&mut t);
    crate::bind_globals::register(&mut t);
    crate::bind_registry::register(&mut t, sm);
    crate::bind_items::register(&mut t);
    register(&mut t);
    t
}

fn template(class: ItemClass, ty: &str, weight: f32, tags: &[&str]) -> Rc<ItemTemplate> {
    let mut t = ItemTemplate::new(class, "Base", ty);
    t.actual_weight = weight;
    t.display_name = ty.into();
    t.tags = tags.iter().map(|s| s.to_string()).collect();
    Rc::new(t)
}

fn item(class: ItemClass, ty: &str, weight: f32, tags: &[&str], id: i32) -> InventoryItem {
    let mut i = InventoryItem::create(template(class, ty, weight, tags), 1);
    i.id = id;
    i
}

fn shared(i: InventoryItem) -> ItemRef {
    Rc::new(RefCell::new(i))
}

#[test]
fn identity_add_remove_and_lists() {
    let mut t = setup();
    let inv = SharedContainer::new(-1, "none").into_ref();
    let crate_ = SharedContainer::new(5, "crate").into_ref();
    let rock = shared(item(ItemClass::Normal, "Rock", 2.0, &[], 1));
    let hammer = shared(item(ItemClass::Normal, "Hammer", 1.5, &["base:hammer"], 2));
    let rock2 = shared(item(ItemClass::Normal, "Rock", 2.0, &[], 3));
    let g = t.globals().clone();
    g.set_str("inv", container_value(&inv));
    g.set_str("inv2", container_value(&inv));
    g.set_str("crate", container_value(&crate_));
    g.set_str("rock", item_value(&rock));
    g.set_str("hammer", item_value(&hammer));
    g.set_str("rock2", item_value(&rock2));
    ok(
        &mut t,
        r#"
        assert(inv == inv2 and instanceof(inv, "ItemContainer"))
        assert(tostring(inv) == "ItemContainer:[type:none, parent:null]", tostring(inv))
        assert(inv:getType() == "none" and crate:getType() == "crate")
        assert(inv:isEmpty() and inv:getItems():size() == 0)
        assert(inv:getItems() == inv:getItems())
        assert(inv:AddItem(rock) == rock)
        assert(inv:addItem(hammer) == hammer)
        assert(inv:DoAddItem(rock2) == rock2)
        local items = inv:getItems()
        assert(items:size() == 3 and items:get(0) == rock and items:get(2) == rock2)
        assert(rock:getContainer() == inv)
        assert(inv:contains(rock) and inv:contains("Rock") and inv:containsType("Base.Rock"))
        assert(inv:containsID(2) and not inv:containsID(9))
        assert(inv:getItemWithID(2) == hammer and inv:getItemById(3) == rock2)
        assert(inv:getFirstType("Rock") == rock and inv:FindAndReturn("Base.Hammer") == hammer)
        assert(inv:getCountType("Rock") == 2 and inv:getItemCount("Base.Rock") == 2)
        assert(inv:getNumberOfItem("Rock") == 2 and inv:getNumberOfItem("Base.Hammer") == 1)
        assert(inv:getItemsFromType("Rock"):size() == 2)
        assert(inv:getItemsFromFullType("Rock"):size() == 0)
        assert(inv:getItemsFromFullType("Base.Rock"):size() == 2)
        assert(inv:getFirstTag("base:hammer") == hammer and inv:containsTag("base:hammer"))
        assert(inv:getCountTag("base:hammer") == 1)
        assert(inv:getFirstCategory("Item") == rock and inv:getItemsFromCategory("Item"):size() == 3)
        assert(inv:getSomeType("Rock", 1):size() == 1)
        -- compareType with variants.
        assert(inv:getCountType("Hammer/Rock") == 3 and inv:getCountType("Ham/Rock") == 2)
        -- An item moves: it leaves its old container.
        assert(crate:AddItem(rock) == rock)
        assert(rock:getContainer() == crate and not inv:contains(rock))
        assert(inv:getItems():size() == 2 and crate:getItems():get(0) == rock)
        -- Same id twice: the item already there.
        assert(inv:AddItem(rock2) == rock2 and inv:getItems():size() == 2)
        inv:Remove(rock2)
        assert(rock2:getContainer() == nil and inv:getItems():size() == 1)
        inv:AddItem(rock2)
        local removed = inv:RemoveAll("Rock")
        assert(removed:size() == 1 and removed:get(0) == rock2 and inv:getItems():size() == 1)
        inv:DoRemoveItem(hammer)
        assert(inv:isEmpty() and hammer:getContainer() == nil)
        -- Result lists are filled in place.
        crate:AddItem(rock2)
        local l = ArrayList.new()
        l:add("x")
        assert(crate:getAllType("Rock", l) == l and l:size() == 3 and l:get(1) == rock)
        -- getSome counts the list's own entries.
        local s = ArrayList.new()
        s:add("y")
        assert(crate:getSomeType("Rock", 2, s):size() == 2)
        assert(not pcall(function() crate:getAllType("Rock", nil) end))
        "#,
    );
    assert_eq!(crate_.borrow().items.len(), 2);
}

#[test]
fn eval_predicates_and_comparators() {
    let mut t = setup();
    let inv = SharedContainer::new(-1, "none").into_ref();
    for (i, cond) in [(1, 4), (2, 9), (3, 0)] {
        let mut it = item(ItemClass::Normal, "Knife", 1.0, &["base:sharpknife"], i);
        it.condition = cond;
        it.condition_max = 10;
        shared::add_item(&inv, &shared(it));
    }
    let mut rock = item(ItemClass::Normal, "Rock", 2.0, &[], 4);
    rock.condition = 1;
    shared::add_item(&inv, &shared(rock));
    t.globals().set_str("inv", container_value(&inv));
    ok(
        &mut t,
        r#"
        local function good(it) return it:getCondition() > 3 end
        local function above(it, n) return it:getCondition() > n end
        assert(inv:getCountEval(good) == 2 and inv:getCountEvalArg(above, 5) == 1)
        assert(inv:getFirstEval(good):getID() == 1)
        assert(inv:getFirstEvalArg(above, 5):getID() == 2)
        assert(inv:containsEval(good) and not inv:containsEvalArg(above, 50))
        assert(inv:getAllEval(good):size() == 2)
        assert(inv:getAllTypeEval("Knife", function(it) return it:getID() ~= 2 end):size() == 2)
        assert(inv:getAllTagEval("base:sharpknife", good, nil):size() == 2)
        assert(inv:getCountTagEval("base:sharpknife", good) == 2)
        -- Only `true` counts; errors are false.
        assert(inv:getCountEval(function() return 1 end) == 0)
        assert(inv:getCountEval(function() error("x") end) == 0)
        -- Comparators: the item with the highest score wins, ties keep the first.
        local function byCond(a, b) return a:getCondition() - b:getCondition() end
        assert(inv:getBestEval(function() return true end, byCond):getID() == 2)
        assert(inv:getBestTypeEval("Knife", function(a, b) return b:getCondition() - a:getCondition() end):getID() == 3)
        assert(inv:getBestTypeEval("Knife", function() return 0 end):getID() == 1)
        assert(inv:getBestEvalArg(above, function(a, b, n) return a:getID() - b:getID() end, 3):getID() == 2)
        -- getBestCondition: none when the best is broken.
        assert(inv:getBestCondition("Knife"):getID() == 2)
        assert(inv:getBestConditionEval(function(it) return it:getID() == 3 end) == nil)
        assert(not pcall(function() inv:getFirstEval(nil) end))
        assert(inv:getUsesType("Knife") == 3)
        "#,
    );
}

#[test]
fn bags_are_searched_by_value() {
    let mut t = setup();
    let inv = SharedContainer::new(-1, "none").into_ref();
    let mut bag = item(ItemClass::Container, "Bag", 0.5, &[], 10);
    if let ItemKind::Container(c) = &mut bag.kind {
        c.container.items.push(SavedItem::Item(Box::new(item(
            ItemClass::Normal,
            "Rock",
            2.0,
            &[],
            11,
        ))));
        let mut inner = item(ItemClass::Container, "Pouch", 0.1, &[], 12);
        if let ItemKind::Container(p) = &mut inner.kind {
            p.container.items.push(SavedItem::Item(Box::new(item(
                ItemClass::Normal,
                "Coin",
                0.0,
                &["base:coin"],
                13,
            ))));
        }
        c.container.items.push(SavedItem::Item(Box::new(inner)));
    }
    let bag = shared(bag);
    shared::add_item(&inv, &bag);
    shared::add_item(&inv, &shared(item(ItemClass::Normal, "Rock", 2.0, &[], 1)));
    t.globals().set_str("inv", container_value(&inv));
    t.globals().set_str("bag", item_value(&bag));
    ok(
        &mut t,
        r#"
        assert(inv:getCountType("Rock") == 1 and inv:getCountTypeRecurse("Rock") == 2)
        assert(inv:getItemCount("Rock", true) == 2 and inv:getItemCountRecurse("Coin") == 1)
        assert(inv:containsTypeRecurse("Coin") and not inv:containsType("Coin"))
        assert(inv:containsTagRecurse("base:coin") and inv:getCountTagRecurse("base:coin") == 1)
        assert(inv:getItemCountFromTypeRecurse("Base.Coin") == 1)
        assert(inv:getNumberOfItem("Rock", false, true) == 2)
        assert(inv:getUsesTypeRecurse("Rock") == 2)
        -- The first match is a shared item: fine.
        assert(inv:getFirstTypeRecurse("Rock"):getID() == 1)
        assert(inv:getFirstCategoryRecurse("Container") == bag)
        -- Returning an item inside a bag is not supported yet.
        assert(not pcall(function() inv:getAllTypeRecurse("Rock") end))
        assert(not pcall(function() inv:getFirstTypeRecurse("Coin") end))
        assert(not pcall(function() inv:getCountEvalRecurse(function() return true end) end))
        -- Removal inside bags works on their contents.
        assert(inv:removeItemWithIDRecurse(13))
        assert(inv:getItemCountRecurse("Coin") == 0)
        inv:RemoveOneOf("Rock")
        assert(inv:getCountType("Rock") == 0 and inv:getCountTypeRecurse("Rock") == 1)
        assert(inv:RemoveOneOf("Rock", true) == bag and inv:getCountTypeRecurse("Rock") == 0)
        "#,
    );
}

#[test]
fn fields_weights_and_capacity() {
    let mut t = setup();
    let inv = SharedContainer::new(-1, "fridge").into_ref();
    assert_eq!(inv.borrow().age_factor, 0.02);
    shared::add_item(&inv, &shared(item(ItemClass::Normal, "Rock", 2.25, &[], 1)));
    shared::add_item(&inv, &shared(item(ItemClass::Normal, "Rock", 1.5, &[], 2)));
    t.globals().set_str("inv", container_value(&inv));
    ok(
        &mut t,
        r#"
        assert(inv:isFridge() and not inv:isFreezer() and inv:getAgeFactor() < 0.03)
        assert(inv:getContentsWeight() == 3.75 and inv:getCapacityWeight() == 3.75)
        assert(inv:getCapacity() == 50 and inv:getMaxWeight() == 50)
        inv:setCapacity(150)
        assert(inv:getCapacity() == 100)
        inv:setCapacity(10)
        assert(inv:getEffectiveCapacity(nil) == 10)
        assert(inv:hasRoomFor(nil, 6.25) and not inv:hasRoomFor(nil, 6.5))
        assert(inv:getFreeCapacity(nil) == 6.25 and not inv:isFull(nil))
        assert(inv:getAvailableWeightCapacity() == 6.25)
        assert(math.abs(ItemContainer.floatingPointCorrection(1.234) - 1.23) < 1e-6)
        assert(math.abs(ItemContainer.floatingPointCorrection(2.346) - 2.35) < 1e-6)
        assert(not inv:isExplored()); inv:setExplored(true); assert(inv:isExplored())
        assert(inv:isDirty()); inv:setDirty(false); assert(not inv:isDirty())
        inv:setWeightReduction(140); assert(inv:getWeightReduction() == 100)
        inv:setOnlyAcceptCategory("  "); assert(inv:getOnlyAcceptCategory() == nil)
        inv:setOnlyAcceptCategory("Food"); assert(inv:getOnlyAcceptCategory() == "Food")
        assert(inv:getOpenSound() == nil); inv:setOpenSound("Open"); assert(inv:getOpenSound() == "Open")
        assert(inv:getParent() == nil and inv:getCharacter() == nil and inv:getContainingItem() == nil)
        assert(inv:getOutermostContainer() == inv and inv:getSquare() == nil)
        assert(inv:getVehiclePart() == nil and not inv:isVehicleSeat())
        inv:setCustomTemperature(0.5); assert(inv:getTemperature() == 0.5)
        inv:requestSync(); inv:requestServerItemsForContainer()
        inv:setType("crate"); assert(inv:getType() == "crate" and not inv:isFridge())
        inv:setCustomTemperature(0); assert(inv:getTemperature() == 1)
        -- Category filter.
        assert(not inv:isItemAllowed(inv:getItems():get(0)) and not inv:isItemAllowed(nil))
        inv:setOnlyAcceptCategory(nil)
        assert(inv:isItemAllowed(inv:getItems():get(0)))
        local c = ItemContainer.new()
        assert(c:getType() == "none" and c:isEmpty())
        local f = ItemContainer.new(3, "fridge", nil, nil)
        assert(f:getAgeFactor() < 0.03 and f:getCookingFactor() == 0)
        "#,
    );
}

/// A Lua object of class `IsoPlayer` (for the parent rules).
struct FakePlayer;
impl LuaObject for FakePlayer {
    fn class_name(&self) -> &str {
        ISO_PLAYER
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn super_classes(&self) -> &[&'static str] {
        &[ISO_GAME_CHARACTER]
    }
}

#[test]
fn add_by_type_and_parent() {
    let mut t = setup();
    let inv = SharedContainer::new(-1, "none").into_ref();
    t.globals().set_str("inv", container_value(&inv));
    ok(
        &mut t,
        r#"
        local r = inv:AddItem("Base.Rock")
        assert(r ~= nil and r:getFullType() == "Base.Rock" and r:getContainer() == inv)
        assert(inv:AddItem("Base.NoSuchItem") == nil)
        assert(inv:AddItem("Base.OldRock") == nil)
        local a = inv:AddItem("Apple")
        assert(a:getFullType() == "Base.Apple" and instanceof(a, "Food"))
        assert(inv:AddItems("Base.Rock", 2):size() == 2 and inv:getCountType("Rock") == 3)
        assert(inv:AddItem("Base.Rock", 1.0) == true)
        assert(inv:getItems():size() == 5)
        assert(inv:getBestFood(nil) == a and inv:getTotalFoodScore(nil) > 0)
        "#,
    );
    let b = expose_class(&mut t, ISO_PLAYER, None);
    b.method("getInventoryWeight", |c| Ok(c.push(42.0)));
    b.method("getMaxWeight", |c| Ok(c.push(8.0)));
    b.method("isUnlimitedCarry", |c| Ok(c.push(false)));
    b.method("isGhostMode", |c| Ok(c.push(false)));
    let chr = Value::Object(Rc::new(FakePlayer));
    set_parent(&inv, chr.clone());
    t.globals().set_str("chr", chr);
    ok(
        &mut t,
        r#"
        assert(inv:getParent() == chr and inv:getCharacter() == chr)
        assert(inv:getCapacityWeight() == 42 and inv:getMaxWeight() == 8)
        assert(inv:getContentsWeight() < 42)
        -- The character's own inventory ignores the trait rules.
        assert(inv:getEffectiveCapacity(chr) == 50)
        local r = inv:getFirstType("Rock")
        inv:Remove(r)
        assert(r:getContainer() == nil)
        "#,
    );
}

#[test]
fn shared_container_round_trip() {
    let mut c = ItemContainer::new(Some("crate"));
    c.capacity = 30;
    c.explored = true;
    c.items.push(SavedItem::Item(Box::new(item(
        ItemClass::Normal,
        "Rock",
        2.0,
        &[],
        1,
    ))));
    c.items.push(SavedItem::Raw(vec![0, 1, 2, 0, 0, 0, 9]));
    let s = SharedContainer::from_container(c.clone());
    assert_eq!(s.borrow().items.len(), 1);
    assert_eq!(s.borrow().raw_items.len(), 1);
    let i = s.borrow().items[0].clone();
    assert!(container_of(&i).is_some_and(|o| Rc::ptr_eq(&o, &s)));
    assert_eq!(s.borrow().to_container(), c);
}

fn with_fluid(ty: &str, id: i32, fluid: Option<(i8, f32)>, capacity: f32) -> ItemRef {
    let mut i = item(ItemClass::Normal, ty, 0.5, &[], id);
    let mut fc = pz_entity::FluidContainer {
        capacity,
        ..Default::default()
    };
    if let Some((f, amount)) = fluid {
        fc.fluids.push(pz_entity::FluidInstance::new(
            pz_entity::FluidRef::Type(f),
            amount,
        ));
        fc.recalculate_caches(true);
    }
    i.set_fluid_container(fc);
    shared(i)
}

#[test]
fn fluid_queries() {
    let mut t = setup();
    let inv = SharedContainer::new(-1, "none").into_ref();
    shared::add_item(&inv, &shared(item(ItemClass::Normal, "Rock", 2.0, &[], 1)));
    // Water 1, TaintedWater 4, Bleach 11.
    shared::add_item(&inv, &with_fluid("Bottle", 2, None, 1.0));
    shared::add_item(&inv, &with_fluid("Bottle", 3, Some((4, 0.5)), 1.0));
    shared::add_item(&inv, &with_fluid("Bottle", 4, Some((1, 1.0)), 1.0));
    shared::add_item(&inv, &with_fluid("Bleach", 5, Some((11, 0.25)), 0.5));
    t.globals().set_str("inv", container_value(&inv));
    ok(
        &mut t,
        r#"
        local function id(it) return it and it:getID() or -1 end
        assert(inv:getWaterContainerCount() == 4)
        assert(id(inv:FindWaterSource()) == 3)
        assert(inv:getAllWaterFillables():size() == 1 and inv:getAllWaterFillables() == inv:getAllWaterFillables())
        assert(inv:getAllWaterFluidSources(false):size() == 1 and inv:getAllWaterFluidSources(true):size() == 2)
        assert(id(inv:getFirstWaterFluidSources(true)) == 3 and id(inv:getFirstWaterFluidSources(false)) == 4)
        assert(id(inv:getFirstWaterFluidSources(true, true)) == 3)
        assert(id(inv:getFirstCleaningFluidSources()) == 5 and inv:getAllCleaningFluidSources():size() == 1)
        assert(id(inv:getFirstFluidContainer("Water")) == 2 and id(inv:getFirstFluidContainer(nil)) == 2)
        -- Empty ones, then not-full ones of the type (case-insensitive).
        assert(inv:getAvailableFluidContainer("taintedwater"):size() == 2)
        assert(inv:getAvailableFluidContainer("Water"):size() == 1)
        assert(math.abs(inv:getAvailableFluidContainersCapacity("TaintedWater") - 1.5) < 1e-6)
        assert(id(inv:getFirstAvailableFluidContainer("Bleach")) == 2)
        assert(inv:FindAndReturnWaterItem(0) == nil)
        assert(inv:getSoapList(nil, false):size() == 0)
        "#,
    );
}
