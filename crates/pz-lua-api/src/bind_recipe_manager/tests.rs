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
fn items_can_be_use_check_is_an_explicit_gap() {
    let sm = manager();
    assert!(names(&sm, &item(&sm, ItemClass::Food, "Apple"), 0, true).unwrap().is_empty());
    let err = names(&sm, &item(&sm, ItemClass::Normal, "Bowl"), 0, true).unwrap_err();
    assert!(matches!(err, LuaError::Runtime { class: "pz.host.Unsupported", .. }), "{err:?}");
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
