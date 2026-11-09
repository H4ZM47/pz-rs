//! The InventoryItem methods `ISInventoryPane:renderdetails` calls for
//! every row: the job bar, the heat tint and the favourite-recipe mark.
use super::*;
use pz_inventory::template::ScriptItemRef;
use pz_inventory::ItemTemplate;

fn thread() -> Thread {
    let mut thread = Thread::new(pz_kahlua::TableRef::new());
    pz_kahlua::stdlib::install(&mut thread, None).unwrap();
    crate::bind_java_util::register(&mut thread);
    crate::bind_globals::register(&mut thread);
    register(&mut thread);
    thread
}

fn item(thread: &mut Thread, class: ItemClass, edit: impl FnOnce(&mut ItemTemplate)) -> ItemRef {
    let mut template = ItemTemplate::new(class, "Base", "PaneRow");
    edit(&mut template);
    let item = Rc::new(RefCell::new(InventoryItem::create(Rc::new(template), 1)));
    thread.globals().set_str("item", item_value(&item));
    item
}

fn run(thread: &mut Thread, source: &str) -> Result<Vec<Value>, String> {
    let function =
        pz_kahlua::stdlib::load_string(source, Some("pane-row"), thread.globals().clone())
            .unwrap();
    thread
        .pcall(&function, &[])
        .map_err(|(error, _)| pz_kahlua::exec::java_value_of(&error))
}

fn num(value: &Value) -> f64 {
    match value {
        Value::Num(n) => *n,
        other => panic!("not a number: {other:?}"),
    }
}

/// `getJobDelta`/`getJobType` read the plain fields: 0 and null on a new
/// item, and what `setJobDelta`/`setJobType` stored.
#[test]
fn job_delta_and_type_are_plain_fields() {
    let mut t = thread();
    let actual = item(&mut t, ItemClass::Normal, |_| {});
    let r = run(&mut t, "return item:getJobDelta(), item:getJobType() == nil").unwrap();
    assert_eq!(num(&r[0]), 0.0);
    assert!(matches!(r[1], Value::Bool(true)));
    let r = run(
        &mut t,
        "item:setJobDelta(0.25); item:setJobType('Cooking')\n\
         return item:getJobDelta(), item:getJobType()",
    )
    .unwrap();
    assert_eq!(num(&r[0]), 0.25);
    assert_eq!(pz_kahlua::exec::java_value_of(&r[1]), "Cooking");
    assert_eq!(actual.borrow().job_type.as_deref(), Some("Cooking"));
    run(&mut t, "item:setJobType(nil)").unwrap();
    assert_eq!(actual.borrow().job_type, None);
}

/// `getItemHeat` is `itemHeat` (1 on a new item); `getInvHeat` is
/// `1.0F - itemHeat` in float arithmetic.
#[test]
fn item_heat_and_inverse_heat() {
    let mut t = thread();
    let actual = item(&mut t, ItemClass::Normal, |_| {});
    let r = run(&mut t, "return item:getItemHeat(), item:getInvHeat()").unwrap();
    assert_eq!(num(&r[0]), 1.0);
    assert_eq!(num(&r[1]), 0.0);
    actual.borrow_mut().item_heat = 2.3;
    let r = run(&mut t, "return item:getItemHeat(), item:getInvHeat()").unwrap();
    assert_eq!(num(&r[0]), 2.3f32 as f64);
    assert_eq!(num(&r[1]), (1.0f32 - 2.3f32) as f64);
}

/// `DrainableComboItem.getHeat` is its own `heat` field, 1 on a new item.
#[test]
fn drainable_heat() {
    let mut t = thread();
    let actual = item(&mut t, ItemClass::Drainable, |_| {});
    let r = run(&mut t, "return item:getHeat()").unwrap();
    assert_eq!(num(&r[0]), 1.0);
    actual.borrow_mut().drainable_heat = Some(1.5);
    let r = run(&mut t, "return item:getHeat()").unwrap();
    assert_eq!(num(&r[0]), 1.5);
}

/// `isFavouriteRecipeInput(player)`: false without a script item, and
/// false when the script is used in no recipe (the player is not read).
#[test]
fn favourite_recipe_input_without_recipes_is_false() {
    let mut t = thread();
    item(&mut t, ItemClass::Normal, |_| {});
    let r = run(&mut t, "return item:isFavouriteRecipeInput(nil)").unwrap();
    assert!(matches!(r[0], Value::Bool(false)));
    item(&mut t, ItemClass::Normal, |template| {
        template.script = Some(ScriptItemRef(Rc::new(RefCell::new(
            pz_scripts::objects::items::Item::new(),
        ))));
    });
    let r = run(&mut t, "return item:isFavouriteRecipeInput(nil)").unwrap();
    assert!(matches!(r[0], Value::Bool(false)));
}

/// A script used in recipes needs `CraftRecipe`/known-recipe checks that
/// are not ported: the call fails instead of answering false.
#[test]
fn favourite_recipe_input_with_recipes_is_unsupported() {
    let mut t = thread();
    item(&mut t, ItemClass::Normal, |template| {
        let mut script = pz_scripts::objects::items::Item::new();
        script.used_in_recipes = vec!["Base.MakeThing".into()];
        template.script = Some(ScriptItemRef(Rc::new(RefCell::new(script))));
    });
    let error = run(&mut t, "return item:isFavouriteRecipeInput(nil)").unwrap_err();
    assert!(error.contains("isFavouriteRecipeInput"), "{error}");
}
