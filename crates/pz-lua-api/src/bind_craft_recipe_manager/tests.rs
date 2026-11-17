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
    /// `itemGetQueuedAmount` for every item.
    queued: f32,
    dark: bool,
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
    fn item_queued_amount(&self, _item: &dyn CraftItem, _ratio: f32) -> LuaResult<f32> {
        Ok(self.queued)
    }
    fn too_dark_to_read(&self) -> LuaResult<bool> {
        Ok(self.dark)
    }
}

fn player() -> Chr {
    Chr { player: true, ..Chr::default() }
}

/// An item with its status, by script name.
#[derive(Clone)]
struct It {
    key: usize,
    script: Option<Rc<RefCell<Item>>>,
    full_type: String,
    uses: i32,
    max_uses: i32,
    class: item_status::CraftItemClass,
    no_recipes: bool,
    favorite: bool,
    food: Option<item_status::FoodState>,
    broken: bool,
    dead_body: bool,
    worn: bool,
    equipped: bool,
    condition: i32,
    condition_max: i32,
    fluid: Option<item_status::FluidState>,
}

fn it(sm: &ScriptManager, key: usize, name: &str) -> It {
    It {
        key,
        script: Some(script_item(sm, name)),
        full_type: format!("Base.{name}"),
        uses: 1,
        max_uses: 1,
        class: item_status::CraftItemClass::Other,
        no_recipes: false,
        favorite: false,
        food: None,
        broken: false,
        dead_body: false,
        worn: false,
        equipped: false,
        condition: 10,
        condition_max: 10,
        fluid: None,
    }
}

impl CraftItem for It {
    fn key(&self) -> usize {
        self.key
    }
    fn has_replace_on_use(&self) -> bool {
        false
    }
    fn script_item(&self) -> Option<Rc<RefCell<Item>>> {
        self.script.clone()
    }
    fn full_type(&self) -> String {
        self.full_type.clone()
    }
    fn current_uses(&self) -> i32 {
        self.uses
    }
    fn max_uses(&self) -> i32 {
        self.max_uses
    }
    fn class(&self) -> item_status::CraftItemClass {
        self.class
    }
    fn is_no_recipes(&self) -> LuaResult<bool> {
        Ok(self.no_recipes)
    }
    fn is_favorite(&self) -> bool {
        self.favorite
    }
    fn food(&self) -> Option<item_status::FoodState> {
        self.food
    }
    fn is_broken(&self) -> bool {
        self.broken
    }
    fn in_dead_body(&self) -> bool {
        self.dead_body
    }
    fn is_worn(&self) -> bool {
        self.worn
    }
    fn is_equipped(&self) -> bool {
        self.equipped
    }
    fn condition(&self) -> i32 {
        self.condition
    }
    fn condition_max(&self) -> i32 {
        self.condition_max
    }
    fn is_full_uses(&self) -> bool {
        self.uses >= self.max_uses
    }
    fn is_empty_uses(&self) -> bool {
        self.uses <= 0
    }
    fn container_is_empty(&self) -> bool {
        true
    }
    fn is_fluid_container(&self) -> LuaResult<bool> {
        Ok(self.fluid.is_some())
    }
    fn fluid_container(&self) -> LuaResult<Option<item_status::FluidState>> {
        Ok(self.fluid.clone())
    }
}

fn unique_with(sm: &ScriptManager, item: &It, chr: &Chr, containers: &[It]) -> LuaResult<Vec<String>> {
    let refs: Vec<&dyn CraftItem> = containers.iter().map(|i| i as &dyn CraftItem).collect();
    Ok(names(&unique_recipe_items(sm, item, chr, &refs, &mut |_, _| true)?))
}

fn unique(sm: &ScriptManager, item: &str, chr: &Chr) -> LuaResult<Vec<String>> {
    let item = it(sm, 1, item);
    unique_with(sm, &item, chr, std::slice::from_ref(&item))
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

const FLAG_RECIPES: &str = "
    craftRecipe Damaged { time = 10, inputs { item 1 [Base.Apple] flags[IsDamaged], } outputs { item 1 Base.Apple, } }
    craftRecipe Undamaged { time = 10, inputs { item 1 [Base.Apple] flags[IsUndamaged], } outputs { item 1 Base.Apple, } }
    craftRecipe Worn { time = 10, inputs { item 1 [Base.Apple] flags[IsWorn], } outputs { item 1 Base.Apple, } }
    craftRecipe NotWorn { time = 10, inputs { item 1 [Base.Apple] flags[IsNotWorn], } outputs { item 1 Base.Apple, } }
    craftRecipe Rotten { time = 10, inputs { item 1 [Base.Apple] flags[AllowRottenItem;AllowFavorite], } outputs { item 1 Base.Apple, } }
    craftRecipe Destroyed { time = 10, inputs { item 1 [Base.Apple] flags[AllowDestroyedItem], } outputs { item 1 Base.Apple, } }
    craftRecipe Sharp { time = 10, inputs { item 1 [Base.Apple] flags[IsNotDull], } outputs { item 1 Base.Apple, } }
    craftRecipe Uses { time = 10, inputs { item 3 [Base.Apple], } outputs { item 1 Base.Apple, } }
    craftRecipe TwoBats { time = 10, Tags = InHandCraft, inputs { item 1 [Base.BaseballBat], item 1 [Base.BaseballBat], } outputs { item 1 Base.Apple, } }
    craftRecipe FixBat { time = 10, Tags = AnySurfaceCraft;CanBeDoneInDark, inputs { item 1 [Base.BaseballBat] mode:keep flags[IsDamaged], } outputs { } }
";

fn recipe(sm: &ScriptManager, name: &str) -> Rc<RefCell<CraftRecipe>> {
    sm.get_script(ScriptType::CraftRecipe, &format!("Base.{name}")).unwrap().downcast::<CraftRecipe>().unwrap()
}

/// `consumeInputItem(firstInput, item, true, null, chr)`.
fn accepts(sm: &ScriptManager, name: &str, item: &It, chr: &Chr) -> LuaResult<bool> {
    let r = recipe(sm, name);
    let r = r.borrow();
    let out = item_status::consume_input_item_test(r.inputs().next().unwrap(), &r, item, chr, &mut |_, _| true, None);
    out
}

/// `setRecipeFromContextClick` and `canPerformCurrentRecipe` for one recipe.
fn perform(sm: &ScriptManager, name: &str, clicked: &It, containers: &[It], chr: &Chr) -> LuaResult<bool> {
    let r = recipe(sm, name);
    let r = r.borrow();
    let mut items: Vec<&dyn CraftItem> = containers.iter().map(|i| i as &dyn CraftItem).collect();
    let at = match containers.iter().position(|i| i.key == clicked.key) {
        Some(at) => at,
        None => {
            items.push(clicked);
            items.len() - 1
        }
    };
    handcraft::can_perform_from_context_click(&r, &items, containers.len(), at, chr, &mut |_, _| true)
}

#[test]
fn items_no_hand_recipe_lists_give_an_empty_list() {
    let sm = manager("");
    // Apple is only in an EntityRecipe input.
    assert!(unique(&sm, "Apple", &player()).unwrap().is_empty());
}

#[test]
fn status_tests_follow_the_input_flags() {
    let sm = manager(FLAG_RECIPES);
    let chr = player();
    let apple = it(&sm, 1, "Apple");
    let damaged = It { condition: 3, ..apple.clone() };
    assert!(!accepts(&sm, "Damaged", &apple, &chr).unwrap());
    assert!(accepts(&sm, "Damaged", &damaged, &chr).unwrap());
    assert!(accepts(&sm, "Undamaged", &apple, &chr).unwrap());
    assert!(!accepts(&sm, "Undamaged", &damaged, &chr).unwrap());

    let worn = It { worn: true, ..apple.clone() };
    let equipped = It { equipped: true, ..apple.clone() };
    assert!(!accepts(&sm, "Worn", &apple, &chr).unwrap());
    assert!(accepts(&sm, "Worn", &worn, &chr).unwrap());
    assert!(!accepts(&sm, "NotWorn", &worn, &chr).unwrap());
    assert!(!accepts(&sm, "NotWorn", &equipped, &chr).unwrap());
    // A dead body's items pass the clothing tests.
    assert!(accepts(&sm, "Worn", &It { dead_body: true, ..apple.clone() }, &chr).unwrap());

    let rotten = It { food: Some(item_status::FoodState { rotten: true, ..Default::default() }), ..apple.clone() };
    let favorite = It { favorite: true, ..apple.clone() };
    let broken = It { broken: true, ..apple.clone() };
    assert!(!accepts(&sm, "Undamaged", &rotten, &chr).unwrap());
    assert!(accepts(&sm, "Rotten", &rotten, &chr).unwrap());
    assert!(!accepts(&sm, "Undamaged", &favorite, &chr).unwrap());
    assert!(accepts(&sm, "Rotten", &favorite, &chr).unwrap());
    assert!(!accepts(&sm, "Undamaged", &broken, &chr).unwrap());
    assert!(accepts(&sm, "Destroyed", &broken, &chr).unwrap());

    // `isNoRecipes` is read for a player only.
    let no_recipes = It { no_recipes: true, ..apple.clone() };
    assert!(!accepts(&sm, "Undamaged", &no_recipes, &chr).unwrap());
    assert!(accepts(&sm, "Undamaged", &no_recipes, &Chr::default()).unwrap());

    let err = accepts(&sm, "Sharp", &apple, &chr).unwrap_err();
    assert!(is_unsupported(&err, "SharpnessStatusTests"), "{err:?}");
    // Not listed: no status test runs.
    assert!(!accepts(&sm, "Sharp", &it(&sm, 1, "Nails"), &chr).unwrap());
}

#[test]
fn queued_uses_reduce_what_an_item_offers() {
    let sm = manager(FLAG_RECIPES);
    let apple = it(&sm, 1, "Apple");
    assert!(accepts(&sm, "Undamaged", &apple, &player()).unwrap());
    // `consumesEntireItem` gives `Float.MAX_VALUE`.
    assert!(!accepts(&sm, "Undamaged", &apple, &Chr { queued: f32::MAX, ..player() }).unwrap());
    assert!(!accepts(&sm, "Undamaged", &apple, &Chr { queued: 2.0, ..player() }).unwrap());
    assert!(accepts(&sm, "Undamaged", &It { uses: 2, ..apple }, &Chr { queued: 2.0, ..player() }).unwrap());
}

#[test]
fn context_click_needs_every_input_from_the_containers() {
    let sm = manager(FLAG_RECIPES);
    let chr = player();
    let bat = it(&sm, 1, "BaseballBat");
    let nail = |k| it(&sm, k, "Nails");
    assert!(!perform(&sm, "SpikeBat", &bat, &[bat.clone()], &chr).unwrap());
    assert!(!perform(&sm, "SpikeBat", &bat, &[bat.clone(), nail(2)], &chr).unwrap());
    assert!(perform(&sm, "SpikeBat", &bat, &[nail(2), bat.clone(), nail(3)], &chr).unwrap());
    // The clicked item counts even when no container holds it.
    assert!(perform(&sm, "SpikeBat", &bat, &[nail(2), nail(3)], &chr).unwrap());
    // Too dark to read.
    assert!(!perform(&sm, "SpikeBat", &bat, &[nail(2), nail(3)], &Chr { dark: true, ..player() }).unwrap());
    // A surface recipe then needs the craft surface in range.
    let err = perform(&sm, "Hammer", &nail(2), &[nail(2)], &Chr { dark: true, ..player() }).unwrap_err();
    assert!(is_unsupported(&err, "isCharacterInRangeOfWorkbench"), "{err:?}");
}

#[test]
fn an_item_covers_one_input_only() {
    let sm = manager(FLAG_RECIPES);
    let chr = player();
    let bat = it(&sm, 1, "BaseballBat");
    assert!(!perform(&sm, "TwoBats", &bat, &[bat.clone()], &chr).unwrap());
    assert!(perform(&sm, "TwoBats", &bat, &[bat.clone(), it(&sm, 2, "BaseballBat")], &chr).unwrap());
    // Uses add up: three one-use apples or one three-use apple.
    let apple = |k, uses| It { uses, ..it(&sm, k, "Apple") };
    assert!(!perform(&sm, "Uses", &apple(1, 1), &[apple(1, 1), apple(2, 1)], &chr).unwrap());
    assert!(perform(&sm, "Uses", &apple(1, 1), &[apple(1, 1), apple(2, 1), apple(3, 1)], &chr).unwrap());
    assert!(perform(&sm, "Uses", &apple(1, 3), &[apple(1, 3)], &chr).unwrap());
}

#[test]
fn input_tags_list_the_items_that_have_them() {
    let sm = manager("");
    let chr = player();
    assert!(perform(&sm, "Glue", &it(&sm, 1, "BaseballBat"), &[], &chr).unwrap());
    assert!(!accepts(&sm, "Glue", &it(&sm, 1, "Apple"), &chr).unwrap());
    assert!(!accepts(&sm, "Glue", &It { script: None, ..it(&sm, 1, "BaseballBat") }, &chr).unwrap());
}

#[test]
fn an_undamaged_bat_lists_no_hand_recipe_without_nails() {
    let sm = manager(FLAG_RECIPES);
    // Glue is covered by the bat itself and cannot be done in the dark.
    let dark = Chr { dark: true, ..player() };
    assert!(unique(&sm, "BaseballBat", &dark).unwrap().is_empty());
    assert_eq!(unique(&sm, "BaseballBat", &player()).unwrap(), ["Glue"]);
    // FixBat needs a damaged bat, and then the craft surface.
    let bat = It { condition: 3, ..it(&sm, 1, "BaseballBat") };
    let err = unique_with(&sm, &bat, &dark, &[]).unwrap_err();
    assert!(is_unsupported(&err, "isCharacterInRangeOfWorkbench"), "{err:?}");
}

#[test]
fn on_test_rejections_skip_the_recipe() {
    let sm = manager(
        "craftRecipe Tested { time = 10, Tags = InHandCraft, OnTest = Recipe.OnTest.No, inputs { item 1 [Base.Apple], } outputs { item 1 Base.Apple, } }",
    );
    let chr = player();
    let apple = it(&sm, 7, "Apple");
    let mut calls = Vec::new();
    let list = unique_recipe_items(&sm, &apple, &chr, &[], &mut |name, key| {
        calls.push((name.to_string(), key));
        false
    })
    .unwrap();
    assert!(list.is_empty());
    assert_eq!(calls, [("Recipe.OnTest.No".to_string(), 7)]);
    assert_eq!(unique_with(&sm, &apple, &chr, &[]).unwrap(), ["Tested"]);
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

    assert!(unique(&sm, "Apple", &novice).unwrap().is_empty());
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
    let out =
        run(&mut t, "return CraftRecipeManager.getUniqueRecipeItems(apple, player, ArrayList.new()):size()").unwrap();
    assert!(out[0].raw_equals(&Value::Num(0.0)), "{out:?}");
    let err = run(&mut t, "return CraftRecipeManager.getUniqueRecipeItems(bat, nil, ArrayList.new())").unwrap_err();
    assert!(err.contains("NullPointerException"), "{err}");
    let err = run(&mut t, "return CraftRecipeManager.getUniqueRecipeItems(bat, player, nil)").unwrap_err();
    assert!(err.contains("NullPointerException"), "{err}");
    // SpikeBat lacks the nails; Glue is covered by the bat. The character
    // is not a local player, so it is never too dark to read.
    let out =
        run(&mut t, "return CraftRecipeManager.getUniqueRecipeItems(bat, player, ArrayList.new()):get(0):getName()")
            .unwrap();
    assert!(out[0].raw_equals(&Value::str("Glue")), "{out:?}");
}

#[test]
fn a_queued_handcraft_action_holds_back_its_manual_inputs() {
    let _collection = pz_kahlua::gc::defer_collection();
    let sm = manager(
        "craftRecipe SliceApple { time = 10, Tags = InHandCraft, inputs { item 1 [Base.Apple] mode:destroy, } outputs { item 1 Base.Apple, } }",
    );
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
    t.globals().set_str("player", player);
    for name in ["apple", "other"] {
        t.globals().set_str(name, crate::bind_items::try_item_value(&item(&sm, ItemClass::Food, "Apple")).unwrap());
    }
    let count = |t: &mut Thread, item: &str| {
        let out = run(t, &format!("return CraftRecipeManager.getUniqueRecipeItems({item}, player, ArrayList.new()):size()"));
        out.unwrap()[0].clone()
    };
    assert!(count(&mut t, "apple").raw_equals(&Value::Num(1.0)));
    // The action holds the apple in its only input (index 0): a destroy
    // input takes all of it.
    let out = run(
        &mut t,
        r#"
        local list, recipe = CraftRecipeManager.queryRecipes("InHandCraft")
        for i = 0, list:size() - 1 do
            if list:get(i):getName() == "SliceApple" then recipe = list:get(i) end
        end
        local inputs = ArrayList.new()
        inputs:add(apple)
        local action = setmetatable({ manualInputs = { [0] = inputs }, craftRecipe = recipe }, { Type = "ISHandcraftAction" })
        ISTimedActionQueue = { queues = { [player] = { queue = { action } } } }
        return recipe:getName()
        "#,
    )
    .unwrap();
    assert!(out[0].raw_equals(&Value::str("SliceApple")), "{out:?}");
    assert!(count(&mut t, "apple").raw_equals(&Value::Num(0.0)));
    assert!(count(&mut t, "other").raw_equals(&Value::Num(1.0)), "another apple is not held");
    // A list of two items holds back neither.
    run(&mut t, "ISTimedActionQueue.queues[player].queue[1].manualInputs[0]:add(other)").unwrap();
    assert!(count(&mut t, "apple").raw_equals(&Value::Num(1.0)));
}

#[test]
fn lua_query_recipes() {
    let _collection = pz_kahlua::gc::defer_collection();
    let sm = manager("");
    let mut t = Thread::new(TableRef::new());
    pz_kahlua::stdlib::install(&mut t, None).unwrap();
    crate::bind_java_util::register(&mut t);
    crate::bind_scripts::register(&mut t, sm.clone());
    register(&mut t, sm.clone(), Rc::new(RefCell::new(Characters::default())));
    let out = run(
        &mut t,
        r#"
        local list = CraftRecipeManager.queryRecipes("InHandCraft;AnySurfaceCraft")
        return list:size(), list:get(0):getName(), CraftRecipeManager.queryRecipes(nil):size(),
            CraftRecipeManager.queryRecipes("*"):size()
        "#,
    )
    .unwrap();
    let want = [Value::Num(3.0), Value::str("Hammer"), Value::Num(0.0), Value::Num(5.0)];
    assert!(out.iter().zip(&want).all(|(a, b)| a.raw_equals(b)) && out.len() == 4, "{out:?}");
}

const FLUID_RECIPES: &str = "
    craftRecipe Fill { time = 10, Tags = InHandCraft, inputs { item 1 [Base.Apple] mode:keep, -fluid 0.5 [Water], } outputs { item 1 Base.Apple, } }
    craftRecipe FillAny { time = 10, Tags = InHandCraft, inputs { item 1 [Base.Apple] mode:keep, -fluid 0.5 [*], } outputs { item 1 Base.Apple, } }
";

/// Installs an item environment whose fluid registry has Water and Bleach.
fn install_fluids() {
    use pz_scripts::objects::definitions::FluidCategory;
    let mut fluids = pz_entity::FluidRegistry::default();
    for (name, categories) in [("Water", vec![FluidCategory::Water]), ("Bleach", vec![FluidCategory::Hazardous])] {
        fluids.insert(None, pz_entity::FluidInfo { name: name.into(), categories, ..Default::default() });
    }
    let ctx = pz_entity::BasicContext { fluids: Some(fluids), ..Default::default() };
    crate::bind_items::install_item_runtime(None, Some(ctx));
}

fn with_fluid(item: It, fluids: &[(&str, f32)], capacity: f32) -> It {
    let fluids: Vec<_> = fluids
        .iter()
        .map(|(n, a)| (Some(item_status::FluidId { name: n.to_string(), categories: Vec::new() }), *a))
        .collect();
    let amount = fluids.iter().map(|f| f.1).sum();
    It { fluid: Some(item_status::FluidState { amount, capacity, fluids }), ..item }
}

#[test]
fn consume_from_item_fluids_count_the_fluid_of_each_item() {
    install_fluids();
    let sm = manager(FLUID_RECIPES);
    let chr = player();
    let apple = |k| it(&sm, k, "Apple");
    // The fluid line's filter, then any amount above the epsilon.
    assert!(accepts(&sm, "Fill", &with_fluid(apple(1), &[("Water", 1.0)], 1.0), &chr).unwrap());
    assert!(accepts(&sm, "Fill", &with_fluid(apple(1), &[("Water", 0.2)], 1.0), &chr).unwrap());
    assert!(!accepts(&sm, "Fill", &with_fluid(apple(1), &[("Bleach", 1.0)], 1.0), &chr).unwrap());
    assert!(!accepts(&sm, "Fill", &with_fluid(apple(1), &[], 1.0), &chr).unwrap());
    assert!(!accepts(&sm, "Fill", &apple(1), &chr).unwrap());
    assert!(accepts(&sm, "FillAny", &with_fluid(apple(1), &[("Bleach", 1.0)], 1.0), &chr).unwrap());
    // The amount (0.5) comes from the fluid line: 0.2 is not enough, two
    // items of 0.3 are.
    let low = with_fluid(apple(1), &[("Water", 0.2)], 1.0);
    assert!(!perform(&sm, "Fill", &low, &[low.clone()], &chr).unwrap());
    let a = with_fluid(apple(1), &[("Water", 0.3)], 1.0);
    let b = with_fluid(apple(2), &[("Water", 0.3)], 1.0);
    assert!(perform(&sm, "Fill", &a, &[a.clone(), b], &chr).unwrap());
}

/// The crafting window's path through `CraftRecipeData`: `setRecipe`'s
/// `canConsumeInputs`, `autoPopulateInputs`, the craft count, `canPerform`,
/// then `perform`'s real consume and outputs.
#[test]
fn the_window_logic_counts_crafts_and_consumes_the_populated_inputs() {
    let sm = manager("");
    let chr = player();
    let r = recipe(&sm, "SpikeBat");
    let r = r.borrow();
    let items =
        [it(&sm, 1, "BaseballBat"), it(&sm, 2, "Nails"), it(&sm, 3, "Nails"), it(&sm, 4, "Nails"), it(&sm, 5, "Nails"), it(&sm, 6, "Apple")];
    let refs: Vec<&dyn CraftItem> = items.iter().map(|i| i as &dyn CraftItem).collect();
    let all: Vec<usize> = items.iter().map(|i| i.key).collect();
    let t = &mut |_: &str, _: usize| true;
    let mut d = handcraft::RecipeData::new(&r, &refs, &chr);
    assert!(d.consume_inputs_internal(true, Some(&all), true, true, t).unwrap());
    assert_eq!(d.st.all_viable_items, [1, 2, 3]);
    let viable = d.st.all_viable_items.clone();
    d.populate_inputs(&viable, false, t).unwrap();
    assert_eq!(d.st.inputs.iter().map(|i| i.items.clone()).collect::<Vec<_>>(), [vec![1], vec![2, 3]]);
    assert!(d.consume_inputs_internal(true, None, true, false, t).unwrap());
    // One bat, four nails (two per craft).
    assert_eq!(d.possible_craft_count(&all, false, &mut Vec::new(), t).unwrap(), 1);
    assert_eq!(d.possible_craft_count(&all[1..], false, &mut Vec::new(), t).unwrap(), 0);
    // With manual input selection only the applied item types count.
    let mut consumed = Vec::new();
    assert_eq!(d.possible_craft_count(&all, true, &mut consumed, t).unwrap(), 1);
    assert_eq!(consumed, [1, 2, 3, 4, 5]);
    // `getMulticraftConsumedItemsFor`: the applied items, then the other
    // counted items of the same types.
    let mut nails = Vec::new();
    d.multicraft_consumed_items_for(1, &consumed, &mut nails, t).unwrap();
    assert_eq!(nails, [2, 3, 4, 5]);
    let mut bats = Vec::new();
    d.multicraft_consumed_items_for(0, &consumed, &mut bats, t).unwrap();
    assert_eq!(bats, [1]);
    assert_eq!(d.applied_input_item_types(), ["Base.BaseballBat", "Base.Nails", "Base.Nails"]);
    assert_eq!(d.items_of_types(&all, &["Base.Nails".to_string()]), [2, 3, 4, 5]);
    assert!(d.can_perform(Some(&all), true, true, t).unwrap());
    assert!(matches!(d.st.to_output[..], [handcraft::OutputItem::Script(_)]));

    assert!(d.consume_inputs_internal(false, Some(&all), false, false, t).unwrap());
    assert!(d.create_outputs_internal(false).unwrap());
    assert!(matches!(&d.st.to_output[..], [handcraft::OutputItem::Create(s)] if Rc::ptr_eq(s, &script_item(&sm, "BaseballBat"))));
    assert_eq!(d.st.consumed_used_items, [1, 2, 3]);
    assert_eq!(d.consumed_items(), [1, 2, 3]);
    assert_eq!(d.all_input_items(), [1, 2, 3]);

    // The nails are gone from the applied items: no input can be covered.
    d.st.inputs[1].cache.applied.clear();
    assert_eq!(d.possible_craft_count(&all, true, &mut Vec::new(), t).unwrap(), 0);

    // `refresh`: the inputs that are not kept drop their items.
    d.verify_input_items(1, &all, t).unwrap();
    assert!(d.st.inputs[1].items.is_empty());
    d.populate_inputs(&[4, 5], true, t).unwrap();
    assert_eq!(d.st.inputs.iter().map(|i| i.items.clone()).collect::<Vec<_>>(), [vec![], vec![4, 5]]);

    // The action's manual inputs: cleared, then set per input.
    d.clear_manual_inputs();
    assert!(d.st.inputs.iter().all(|i| i.items.is_empty()));
    assert!(!d.are_all_input_items_satisfied(t).unwrap());
    assert!(!d.set_manual_inputs_for(1, &[4], t).unwrap(), "one nail is not enough");
    assert!(d.set_manual_inputs_for(1, &[4, 5], t).unwrap());
    assert!(!d.are_all_input_items_satisfied(t).unwrap(), "no bat yet");
    assert!(d.set_manual_inputs_for(0, &[1], t).unwrap());
    assert_eq!(d.st.inputs.iter().map(|i| i.items.clone()).collect::<Vec<_>>(), [vec![1], vec![4, 5]]);
    assert!(d.are_all_input_items_satisfied(t).unwrap());
}

#[test]
fn a_known_recipe_one_level_short_could_benefit_from_a_recipe_at_hand() {
    let sm = manager(
        "craftRecipe Skilled { time = 10, Tags = InHandCraft, SkillRequired = Woodwork:3, inputs { item 1 [Base.Apple], } outputs { item 1 Base.Apple, } }
         craftRecipe Learned { time = 10, Tags = InHandCraft, needToBeLearn = true, SkillRequired = Woodwork:3, inputs { item 1 [Base.Apple], } outputs { item 1 Base.Apple, } }
         craftRecipe Plain { time = 10, Tags = InHandCraft, needToBeLearn = true, inputs { item 1 [Base.Apple], } outputs { item 1 Base.Apple, } }",
    );
    let recipe = |n: &str| sm.get_script(ScriptType::CraftRecipe, &format!("Base.{n}")).unwrap().downcast::<CraftRecipe>().unwrap();
    let (skilled, learned, plain) = (recipe("Skilled"), recipe("Learned"), recipe("Plain"));
    let (skilled, learned, plain) = (skilled.borrow(), learned.borrow(), plain.borrow());
    let chr = |level: i32| Chr {
        player: true,
        levels: vec![("Woodwork", level)],
        known: vec!["Learned", "Plain"],
        ..Chr::default()
    };

    assert!(could_benefit_from_recipe_at_hand(&learned, &chr(2)));
    assert!(could_benefit_from_recipe_at_hand(&learned, &chr(3)), "no skill is short");
    assert!(!could_benefit_from_recipe_at_hand(&learned, &chr(1)), "two levels short");
    assert!(!could_benefit_from_recipe_at_hand(&skilled, &chr(2)), "need not be learned");
    assert!(!could_benefit_from_recipe_at_hand(&plain, &chr(2)), "no required skill");
    assert!(!could_benefit_from_recipe_at_hand(&learned, &Chr { known: vec![], ..chr(2) }), "unknown");
    assert!(!could_benefit_from_recipe_at_hand(&learned, &Chr { player: false, ..chr(2) }));

    assert!(character_has_required_skills(&learned, &chr(3)));
    assert!(!character_has_required_skills(&learned, &chr(2)));
    assert!(character_has_required_skills(&plain, &chr(0)));
}
