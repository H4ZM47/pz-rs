use super::*;
use pz_inventory::template::ScriptItemRef;
use pz_inventory::{ItemClass, ItemTemplate};
use pz_kahlua::TableRef;
use pz_scripts::ScriptObject;

const SCRIPTS: &str = "module Base {
    item Apple { }
    item BaseballBat { Tags = t:glue, }
    item Nails { }
    craftRecipe SpikeBat { time = 10, Tags = InHandCraft, inputs { item 1 [Base.BaseballBat], item 2 [Base.Nails], } outputs { item 1 Base.BaseballBat, } }
    craftRecipe Bench { time = 10, Tags = EntityRecipe, inputs { item 1 [Base.Apple], } outputs { item 1 Base.Apple, } }
    craftRecipe Hammer { time = 10, Tags = AnySurfaceCraft;CanBeDoneInDark, inputs { item 1 [Base.Nails], } outputs { item 1 Base.Nails, } }
    craftRecipe Loose { time = 10, inputs { item 1 [Base.Nails], } outputs { item 1 Base.Nails, } }
    craftRecipe Glue { time = 10, Tags =  inHANDcraft ;Outdoors, inputs { item 1 tags[t:glue], } outputs { item 1 Base.Nails, } }
}";

fn manager(extra: &str) -> Rc<ScriptManager> {
    static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("pz-craft-recipe-manager-{}-{n}", std::process::id()));
    let file = dir.join("media/scripts/test.txt");
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    std::fs::write(&file, SCRIPTS).unwrap();
    std::fs::write(dir.join("media/scripts/extra.txt"), format!("module Base {{ {extra} }}")).unwrap();
    let sm = ScriptManager::new();
    sm.builtin_tables.borrow_mut().perk_ids = vec!["Woodwork".into()];
    let glue = pz_scripts::registry::ResourceLocation::of("t:glue").unwrap();
    sm.registries.get("item_tag").unwrap().borrow_mut().register(glue, "GLUE", None).unwrap();
    sm.enable_texture_capture();
    sm.load(&pz_fs::GameFs::new(dir.clone()), &[]).unwrap();
    std::fs::remove_dir_all(dir).unwrap();
    // Resolves the input items (`InputScript.OnPostWorldDictionaryInit`).
    sm.post_world_dictionary_init();
    Rc::new(sm)
}

fn names(list: &[ScriptRef]) -> Vec<String> {
    list.iter().map(|s| s.downcast::<CraftRecipe>().unwrap().borrow().name().unwrap().to_string()).collect()
}

fn script_item(sm: &ScriptManager, name: &str) -> Rc<RefCell<Item>> {
    sm.get_script(ScriptType::Item, &format!("Base.{name}")).unwrap().downcast::<Item>().unwrap()
}

#[derive(Default)]
struct Chr {
    levels: Vec<(&'static str, i32)>,
    player: bool,
    know_all: bool,
    known: Vec<&'static str>,
}

impl RecipeCharacter for Chr {
    fn perk_level(&self, perk: &str) -> i32 {
        self.levels.iter().find(|(p, _)| *p == perk).map_or(0, |(_, l)| *l)
    }
    fn is_player(&self) -> bool {
        self.player
    }
    fn is_know_all_recipes(&self) -> bool {
        self.know_all
    }
    fn knows(&self, name: &str) -> bool {
        self.known.contains(&name)
    }
}

fn unique(sm: &ScriptManager, item: &str, chr: Option<&Chr>, containers: bool) -> LuaResult<Vec<String>> {
    let item = script_item(sm, item);
    let list = unique_recipe_items(sm, Some(&item), chr.map(|c| c as &dyn RecipeCharacter), containers, &mut |_| true)?;
    Ok(names(&list))
}

fn is_unsupported(err: &LuaError, what: &str) -> bool {
    matches!(err, LuaError::Runtime { class: "pz.host.Unsupported", .. }) && format!("{err:?}").contains(what)
}

#[test]
fn queries_match_any_whitelisted_tag_in_script_order() {
    let sm = manager("");
    let q = |s: &str| names(&query_recipes(&sm, s).unwrap());
    // `getAllCraftRecipes()` order, kept by every query.
    let all = names(&sm.collection(ScriptType::CraftRecipe).get_all_scripts());
    let ordered =
        |want: &[&str]| -> Vec<String> { all.iter().filter(|n| want.contains(&n.as_str())).cloned().collect() };
    assert_eq!(q("*"), all);
    assert_eq!(all.len(), 5);
    assert_eq!(
        names(&query_recipes_for_tags(&sm, &["InHandCraft", "AnySurfaceCraft"]).unwrap()),
        ordered(&["SpikeBat", "Hammer", "Glue"])
    );
    assert_eq!(q(" inhandcraft "), ordered(&["SpikeBat", "Glue"]));
    assert_eq!(q("InHandCraft-Outdoors"), ["SpikeBat"]);
    assert_eq!(q("-InHandCraft;AnySurfaceCraft"), ordered(&["Bench", "Loose"]));
    assert_eq!(q("untagged"), ["Loose"]);
    assert!(q(" ").is_empty());
    assert!(q("Unknown").is_empty());
    assert!(q(";").is_empty());
}

#[test]
fn items_no_hand_recipe_lists_give_an_empty_list() {
    let sm = manager("");
    let chr = Chr { player: true, ..Chr::default() };
    // Apple is only in an EntityRecipe input.
    assert!(unique(&sm, "Apple", Some(&chr), true).unwrap().is_empty());
}

#[test]
fn listed_items_stop_at_the_item_status_tests() {
    let sm = manager("");
    let chr = Chr { player: true, ..Chr::default() };
    let err = unique(&sm, "BaseballBat", Some(&chr), true).unwrap_err();
    assert!(is_unsupported(&err, "status tests (SpikeBat)"), "{err:?}");
    // Hammer or SpikeBat, whichever `getAllCraftRecipes()` gives first.
    let err = unique(&sm, "Nails", Some(&chr), true).unwrap_err();
    assert!(is_unsupported(&err, "status tests ("), "{err:?}");
}

#[test]
fn input_tags_list_the_items_that_have_them() {
    let sm = manager("");
    let glue = sm.get_script(ScriptType::CraftRecipe, "Base.Glue").unwrap().downcast::<CraftRecipe>().unwrap();
    let glue = glue.borrow();
    let input = glue.inputs().next().unwrap();
    let mut pass = |_: &str| true;
    let bat = script_item(&sm, "BaseballBat");
    let apple = script_item(&sm, "Apple");
    assert!(is_item_valid_for_input_script(input, &glue, Some(&bat), true, &mut pass).is_err());
    assert!(!is_item_valid_for_input_script(input, &glue, Some(&apple), true, &mut pass).unwrap());
    assert!(!is_item_valid_for_input_script(input, &glue, None, true, &mut pass).unwrap());
}

#[test]
fn without_a_character_on_test_item_rejects_every_input() {
    let sm = manager("");
    assert!(unique(&sm, "BaseballBat", None, false).unwrap().is_empty());
}

#[test]
fn on_test_rejections_skip_the_recipe() {
    let sm = manager(
        "craftRecipe Tested { time = 10, Tags = InHandCraft, OnTest = Recipe.OnTest.No, inputs { item 1 [Base.Apple], } outputs { item 1 Base.Apple, } }",
    );
    let chr = Chr { player: true, ..Chr::default() };
    let apple = script_item(&sm, "Apple");
    let mut calls = Vec::new();
    let list = unique_recipe_items(&sm, Some(&apple), Some(&chr), true, &mut |name| {
        calls.push(name.to_string());
        false
    })
    .unwrap();
    assert!(list.is_empty());
    assert_eq!(calls, ["Recipe.OnTest.No"]);
    let err = unique_recipe_items(&sm, Some(&apple), Some(&chr), true, &mut |_| true).unwrap_err();
    assert!(is_unsupported(&err, "status tests (Tested)"), "{err:?}");
}

#[test]
fn skill_and_learning_checks_skip_recipes() {
    let sm = manager(
        "craftRecipe Skilled { time = 10, Tags = InHandCraft, SkillRequired = Woodwork:3, inputs { item 1 [Base.Apple], } outputs { item 1 Base.Apple, } }
         craftRecipe Learned { time = 10, Tags = InHandCraft, needToBeLearn = true, SkillRequired = Woodwork:3, inputs { item 1 [Base.Apple], } outputs { item 1 Base.Apple, } }",
    );
    let skilled = sm.get_script(ScriptType::CraftRecipe, "Base.Skilled").unwrap().downcast::<CraftRecipe>().unwrap();
    let learned = sm.get_script(ScriptType::CraftRecipe, "Base.Learned").unwrap().downcast::<CraftRecipe>().unwrap();
    let (skilled, learned) = (skilled.borrow(), learned.borrow());
    let valid = |r: &CraftRecipe, c: &Chr, containers: bool| is_valid_recipe_for_character(r, Some(c), containers);

    let novice = Chr { player: true, ..Chr::default() };
    let expert = Chr { player: true, levels: vec![("Woodwork", 3)], ..Chr::default() };
    assert!(valid(&skilled, &expert, true).unwrap());
    assert!(!valid(&skilled, &novice, false).unwrap());
    // A recipe that need not be learned gets no benefit from a recipe at hand.
    assert!(!valid(&skilled, &novice, true).unwrap());
    assert!(is_valid_recipe_for_character(&skilled, None, true).unwrap());

    // Unknown: no benefit, and not known.
    assert!(!valid(&learned, &novice, true).unwrap());
    assert!(!valid(&learned, &expert, true).unwrap());
    let known = Chr { player: true, levels: vec![("Woodwork", 3)], known: vec!["Learned"], ..Chr::default() };
    assert!(valid(&learned, &known, true).unwrap());
    let all = Chr { player: true, levels: vec![("Woodwork", 3)], know_all: true, ..Chr::default() };
    assert!(valid(&learned, &all, true).unwrap());
    // Known but short of the skill: the recipe-at-hand benefit is not ported.
    let short = Chr { player: true, known: vec!["Learned"], ..Chr::default() };
    let err = valid(&learned, &short, true).unwrap_err();
    assert!(is_unsupported(&err, "validateBenefitFromRecipeAtHand"), "{err:?}");
    assert!(!valid(&learned, &Chr { player: false, ..short }, true).unwrap());

    assert!(unique(&sm, "Apple", Some(&novice), true).unwrap().is_empty());
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

fn run(t: &mut Thread, src: &str) -> Result<Vec<Value>, String> {
    let f = pz_kahlua::stdlib::load_string(src, Some("test"), t.globals().clone()).unwrap();
    t.pcall_ex(&f, &[]).map_err(|(_, _, e)| format!("{e:?}"))
}

#[test]
fn lua_get_unique_recipe_items() {
    let _collection = pz_kahlua::gc::defer_collection();
    let sm = manager("");
    let mut t = Thread::new(TableRef::new());
    pz_kahlua::stdlib::install(&mut t, None).unwrap();
    crate::bind_java_util::register(&mut t);
    crate::bind_items::register(&mut t);
    crate::bind_scripts::register(&mut t, sm.clone());
    register(&mut t, sm.clone(), Rc::new(RefCell::new(Characters::default())));

    let player = Rc::new(RefCell::new(crate::bind_character_sim::LuaGameCharacter::new(
        &ScriptManager::new(),
        pz_characters::CharacterKind::Player,
    )));
    let player = Value::Object(Rc::new(CharacterObject::try_new(&player).unwrap()));
    for (name, value) in [
        ("player", player),
        ("apple", crate::bind_items::try_item_value(&item(&sm, ItemClass::Food, "Apple")).unwrap()),
        ("bat", crate::bind_items::try_item_value(&item(&sm, ItemClass::Weapon, "BaseballBat")).unwrap()),
    ] {
        t.globals().set_str(name, value);
    }
    let out = run(
        &mut t,
        "local list = CraftRecipeManager.getUniqueRecipeItems(apple, player, ArrayList.new())
         return list:size(), CraftRecipeManager.getUniqueRecipeItems(bat, nil, nil):size()",
    )
    .unwrap();
    assert!(out[0].raw_equals(&Value::Num(0.0)), "{out:?}");
    assert!(out[1].raw_equals(&Value::Num(0.0)), "{out:?}");
    let err = run(&mut t, "return CraftRecipeManager.getUniqueRecipeItems(bat, player, ArrayList.new())").unwrap_err();
    assert!(err.contains("status tests (SpikeBat)"), "{err}");
}
