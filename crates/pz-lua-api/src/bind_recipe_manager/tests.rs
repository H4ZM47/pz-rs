use super::*;
use pz_inventory::template::ScriptItemRef;
use pz_inventory::{ItemClass, ItemKind, ItemTemplate};
use pz_kahlua::TableRef;
use pz_scripts::ScriptObject;

const RECIPES: &str = "module Base {
    evolvedrecipe Stew { BaseItem = Base.WaterPot, ResultItem = Base.PotOfStew, MaxItems = 6, Cookable = true, AddIngredientIfCooked = true, }
    evolvedrecipe Salad { BaseItem = Base.Bowl, ResultItem = Base.Salad, MaxItems = 4, }
}";

fn manager() -> Rc<ScriptManager> {
    static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("pz-recipe-manager-{}-{n}", std::process::id()));
    let file = dir.join("media/scripts/test.txt");
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    std::fs::write(file, RECIPES).unwrap();
    let sm = ScriptManager::new();
    sm.enable_texture_capture();
    {
        let reg = sm.registries.get("item_type").unwrap();
        let mut r = reg.borrow_mut();
        let _ = r.register_id("base:normal", true);
        let _ = r.register_id("base:food", true);
    }
    {
        let mut st = sm.statics.items.borrow_mut();
        st.constants.insert("item_type", "NORMAL", "base:normal");
        st.constants.insert("item_type", "FOOD", "base:food");
    }
    sm.load(&pz_fs::GameFs::new(dir.clone()), &[]).unwrap();
    std::fs::remove_dir_all(dir).unwrap();
    Rc::new(sm)
}

fn item(sm: &ScriptManager, class: ItemClass, name: &str) -> crate::bind_items::ItemRef {
    struct Env;
    impl pz_inventory::ItemEnv for Env {}
    let mut definition = pz_scripts::objects::items::Item::new();
    definition.set_module(&sm.get_module("Base").unwrap());
    definition.init_load_pp(sm, name);
    definition.load(sm, name, &format!("item {name} {{}}")).unwrap();
    let mut template = ItemTemplate::new(class, "Base", name);
    template.script = Some(ScriptItemRef(Rc::new(RefCell::new(definition))));
    pz_inventory::env::create_from_template_ref(&Env, Rc::new(template), 1).unwrap()
}

fn food(item: &crate::bind_items::ItemRef, f: impl FnOnce(&mut pz_inventory::item::FoodData)) {
    match &mut item.borrow_mut().kind {
        ItemKind::Food(x) => f(x),
        _ => panic!("not food"),
    }
}

fn names(sm: &ScriptManager, item: &crate::bind_items::ItemRef, level: i32, include: bool) -> LuaResult<Vec<String>> {
    let list = evolved_recipes_for(sm, &item.borrow(), || Ok(level), include)?;
    Ok(list.iter().map(|s| s.downcast::<EvolvedRecipe>().unwrap().borrow().untranslated_name().to_string()).collect())
}

#[test]
fn matches_base_and_result_items_only() {
    let sm = manager();
    assert_eq!(names(&sm, &item(&sm, ItemClass::Normal, "Bowl"), 0, false).unwrap(), ["Salad"]);
    assert_eq!(names(&sm, &item(&sm, ItemClass::Food, "Salad"), 0, false).unwrap(), ["Salad"]);
    assert!(names(&sm, &item(&sm, ItemClass::Food, "Apple"), 0, false).unwrap().is_empty());
    assert!(names(&sm, &item(&sm, ItemClass::Weapon, "BaseballBat"), 0, false).unwrap().is_empty());
}

#[test]
fn cooked_items_need_add_ingredient_if_cooked() {
    let sm = manager();
    let salad = item(&sm, ItemClass::Food, "Salad");
    let stew = item(&sm, ItemClass::Food, "PotOfStew");
    food(&salad, |f| f.cooked = true);
    food(&stew, |f| f.cooked = true);
    assert!(names(&sm, &salad, 0, false).unwrap().is_empty());
    assert_eq!(names(&sm, &stew, 0, false).unwrap(), ["Stew"]);
}

#[test]
fn water_pot_needs_three_quarters_of_its_uses() {
    let sm = manager();
    let pot = item(&sm, ItemClass::Normal, "WaterPot");
    pot.borrow_mut().uses = 0;
    assert!(names(&sm, &pot, 0, false).unwrap().is_empty());
    pot.borrow_mut().uses = 1;
    assert_eq!(names(&sm, &pot, 0, false).unwrap(), ["Stew"]);
}

#[test]
fn rotten_food_needs_cooking_seven_and_fresh_food_skips_the_read() {
    let sm = manager();
    let salad = item(&sm, ItemClass::Food, "Salad");
    let fresh = evolved_recipes_for(&sm, &salad.borrow(), || panic!("level read for fresh food"), false).unwrap();
    assert_eq!(fresh.len(), 1);
    food(&salad, |f| {
        f.off_age_max = 2;
        f.age = 3.0;
    });
    assert!(salad.borrow().is_rotten());
    assert!(names(&sm, &salad, 6, false).unwrap().is_empty());
    assert_eq!(names(&sm, &salad, 7, false).unwrap(), ["Salad"]);
}

#[test]
fn evolved_recipe_inventory_methods_and_add_item_roundtrip() {
    let _collection = pz_kahlua::gc::defer_collection();
    let dir = std::env::temp_dir().join(format!("pz-evolved-rt-{}", std::process::id()));
    let file = dir.join("media/scripts/test.txt");
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    std::fs::write(
        &file,
        "module Base {
            item Bowl { DisplayName = Bowl, ItemType = base:normal, Weight = 0.5, }
            item Salad { DisplayName = Salad, ItemType = base:food, Weight = 0.5, HungerChange = 0, }
            item Carrot { DisplayName = Carrot, ItemType = base:food, Weight = 0.2, HungerChange = -20, Calories = 40, EvolvedRecipe = Salad:10, }
            item RawMeat { DisplayName = RawMeat, ItemType = base:food, Weight = 0.3, HungerChange = -30, DangerousUncooked = true, EvolvedRecipe = Salad:15, }
            item Salt { DisplayName = Salt, ItemType = base:food, Weight = 0.1, HungerChange = -5, Spice = true, EvolvedRecipe = Salad:5, }
            evolvedrecipe Salad { BaseItem = Base.Bowl, ResultItem = Base.Salad, MaxItems = 4, }
        }",
    )
    .unwrap();
    let sm = ScriptManager::new();
    sm.enable_texture_capture();
    {
        let reg = sm.registries.get("item_type").unwrap();
        let mut r = reg.borrow_mut();
        let _ = r.register_id("base:normal", true);
        let _ = r.register_id("base:food", true);
    }
    {
        let mut st = sm.statics.items.borrow_mut();
        st.constants.insert("item_type", "NORMAL", "base:normal");
        st.constants.insert("item_type", "FOOD", "base:food");
    }
    sm.load(&pz_fs::GameFs::new(dir.clone()), &[]).unwrap();
    let _ = std::fs::remove_dir_all(&dir);
    let sm = Rc::new(sm);
    crate::bind_registry::install_script_manager(sm.clone());

    let mut chars = Characters::default();
    let cooking = chars.perks.new_perk("Cooking").unwrap();
    chars.perks.perks.push(("Cooking".into(), cooking));
    let chars = Rc::new(RefCell::new(chars));

    let mut t = Thread::new(TableRef::new());
    pz_kahlua::stdlib::install(&mut t, None).unwrap();
    crate::bind_java_util::register(&mut t);
    crate::bind_items::register(&mut t);
    crate::bind_item_container::register(&mut t);
    crate::bind_scripts::register(&mut t, sm.clone());
    crate::bind_skills::register(&mut t, sm.clone(), chars.clone());
    register(&mut t, sm.clone(), chars.clone());

    let player = Rc::new(RefCell::new(crate::bind_character_sim::LuaGameCharacter::new(
        &sm,
        pz_characters::CharacterKind::Player,
    )));
    let inv = player.borrow().items.inventory.clone();
    let bowl = crate::bind_items::create_item("Base.Bowl", 1.0).unwrap();
    let carrot = crate::bind_items::create_item("Base.Carrot", 1.0).unwrap();
    let meat = crate::bind_items::create_item("Base.RawMeat", 1.0).unwrap();
    let salt = crate::bind_items::create_item("Base.Salt", 1.0).unwrap();
    crate::bind_item_container::add_item(&mut t, &inv, Some(bowl.clone())).unwrap();
    crate::bind_item_container::add_item(&mut t, &inv, Some(carrot.clone())).unwrap();
    crate::bind_item_container::add_item(&mut t, &inv, Some(meat.clone())).unwrap();
    crate::bind_item_container::add_item(&mut t, &inv, Some(salt.clone())).unwrap();

    t.globals().set_str("player", Value::Object(Rc::new(CharacterObject::try_new(&player).unwrap())));
    t.globals().set_str("bowl", crate::bind_items::item_value(&bowl));
    t.globals().set_str("carrot", crate::bind_items::item_value(&carrot));
    t.globals().set_str("meat", crate::bind_items::item_value(&meat));
    t.globals().set_str("salt", crate::bind_items::item_value(&salt));

    let out = run(
        &mut t,
        "local recipes = RecipeManager.getEvolvedRecipe(bowl, player, nil, true)
         local r = recipes:get(0)
         local usable_before = r:getItemsCanBeUse(player, bowl, nil)
         local need_meat = r:needToBeCooked(meat)
         local need_carrot = r:needToBeCooked(carrot)
         local ir = r:getItemRecipe(carrot)
         local usable_id = r:isItemUsableInRecipe(player, bowl, carrot:getID())
         local dish = r:addItem(bowl, carrot, player)
         local usable_after = r:getItemsCanBeUse(player, dish, nil)
         local spice_before = r:isSpiceAdded(dish, salt)
         dish = r:addItem(dish, salt, player)
         local spice_after = r:isSpiceAdded(dish, salt)
         return recipes:size(), usable_before:size(), need_meat, need_carrot, ir:getUse(),
             usable_id, dish:getType(), dish:haveExtraItems(), usable_after:size(),
             spice_before, spice_after, dish:getSpices():size()",
    )
    .unwrap();
    assert_eq!(out[0].as_num(), Some(1.0));
    assert_eq!(out[1].as_num(), Some(1.0), "only carrot usable before base has ingredients");
    assert!(out[2].truthy(), "raw dangerous meat needs cooking for salad");
    assert!(!out[3].truthy(), "carrot does not need cooking");
    assert_eq!(out[4].as_num(), Some(10.0));
    assert!(out[5].truthy());
    assert_eq!(out[6].as_str(), Some("Salad"));
    assert!(out[7].truthy());
    assert_eq!(out[8].as_num(), Some(2.0), "carrot (remaining half) and salt both usable after first ingredient");
    assert!(!out[9].truthy());
    assert!(out[10].truthy());
    assert_eq!(out[11].as_num(), Some(1.0));
}

fn run(t: &mut Thread, src: &str) -> Result<Vec<Value>, String> {
    let f = pz_kahlua::stdlib::load_string(src, Some("test"), t.globals().clone()).unwrap();
    t.pcall_ex(&f, &[]).map_err(|(_, _, e)| format!("{e:?}"))
}

#[test]
fn lua_menu_reads() {
    let _collection = pz_kahlua::gc::defer_collection();
    let sm = manager();
    let mut chars = Characters::default();
    let cooking = chars.perks.new_perk("Cooking").unwrap();
    chars.perks.perks.push(("Cooking".into(), cooking));
    let mut t = Thread::new(TableRef::new());
    pz_kahlua::stdlib::install(&mut t, None).unwrap();
    crate::bind_java_util::register(&mut t);
    crate::bind_items::register(&mut t);
    crate::bind_scripts::register(&mut t, sm.clone());
    register(&mut t, sm.clone(), Rc::new(RefCell::new(chars)));

    let player = Rc::new(RefCell::new(crate::bind_character_sim::LuaGameCharacter::new(
        &ScriptManager::new(),
        pz_characters::CharacterKind::Player,
    )));
    let player = Value::Object(Rc::new(CharacterObject::try_new(&player).unwrap()));
    let salad = item(&sm, ItemClass::Food, "Salad");
    let rotten = item(&sm, ItemClass::Food, "Salad");
    food(&rotten, |f| {
        f.off_age_max = 2;
        f.age = 3.0;
    });
    for (name, value) in [
        ("player", player),
        ("salad", crate::bind_items::try_item_value(&salad).unwrap()),
        ("rotten", crate::bind_items::try_item_value(&rotten).unwrap()),
        ("apple", crate::bind_items::try_item_value(&item(&sm, ItemClass::Food, "Apple")).unwrap()),
    ] {
        t.globals().set_str(name, value);
    }
    let out = run(
        &mut t,
        "local all = getEvolvedRecipes()
         local stew, salad_recipe = all:get(0), all:get(1)
         local found = RecipeManager.getEvolvedRecipe(salad, player, nil, false)
         return all:size(), stew:getName(), salad_recipe:getName(),
             salad_recipe:isResultItem(salad), salad_recipe:isResultItem(apple), salad_recipe:isResultItem(nil),
             found:size(), found:get(0) == salad_recipe,
             RecipeManager.getEvolvedRecipe(apple, player, nil, true):size(),
             RecipeManager.getEvolvedRecipe(rotten, player, nil, false):size()",
    )
    .unwrap();
    let expect: [Value; 10] = [
        Value::Num(2.0),
        Value::from("Stew"),
        Value::from("Salad"),
        Value::Bool(true),
        Value::Bool(false),
        Value::Bool(false),
        Value::Num(1.0),
        Value::Bool(true),
        Value::Num(0.0),
        Value::Num(0.0),
    ];
    assert_eq!(out.len(), expect.len());
    for (i, (a, b)) in out.iter().zip(&expect).enumerate() {
        assert!(a.raw_equals(b), "value {i}: {a:?} != {b:?}");
    }
    let err = run(&mut t, "return RecipeManager.getEvolvedRecipe(rotten, nil, nil, false)").unwrap_err();
    assert!(err.contains("NullPointerException"), "{err}");
}
