use super::*;
use pz_inventory::item_texture::{CapturedScriptTextures, ItemTextures};
use pz_inventory::{ItemClass, ItemTemplate};
use pz_scripts::registry::{Registry, ResourceLocation};
use pz_scripts::{ScriptManager, ScriptObject};
use std::cell::Cell;

struct Host {
    no: Rc<RegistryKey>,
    show: Rc<RegistryKey>,
    food: Rc<RegistryKey>,
    cooking: PerkRef,
    recipe: RefCell<Option<Rc<RefCell<Recipe>>>>,
    calls: RefCell<Vec<&'static str>>,
    sandbox: Cell<bool>,
    debug: Cell<bool>,
    fail: Cell<Option<&'static str>>,
}
impl Host {
    fn call(&self, phase: &'static str) -> HostResult<()> {
        self.calls.borrow_mut().push(phase);
        if self.fail.get() == Some(phase) {
            return Err(borrowed(phase));
        }
        Ok(())
    }
}
impl KnownPoisonHost for Host {
    fn item_tag(&self, field: &'static str) -> HostResult<Rc<RegistryKey>> {
        self.call(field)?;
        Ok(Rc::clone(if field == "NO_DETECT" {
            &self.no
        } else {
            &self.show
        }))
    }
    fn food_item_type(&self) -> HostResult<Rc<RegistryKey>> {
        self.call("FOOD")?;
        Ok(Rc::clone(&self.food))
    }
    fn recipe(&self, name: &str) -> HostResult<Option<Rc<RefCell<Recipe>>>> {
        assert_eq!(name, "Herbalist");
        self.call("recipe")?;
        Ok(self.recipe.borrow().clone())
    }
    fn is_debug(&self) -> HostResult<bool> {
        self.call("debug")?;
        Ok(self.debug.get())
    }
    fn is_client(&self) -> HostResult<bool> {
        self.call("client")?;
        Ok(false)
    }
    fn is_server(&self) -> HostResult<bool> {
        self.call("server")?;
        Ok(false)
    }
    fn see_not_learnt_recipe(&self) -> HostResult<bool> {
        self.call("sandbox")?;
        Ok(self.sandbox.get())
    }
    fn cooking_perk(&self) -> HostResult<PerkRef> {
        self.call("Cooking")?;
        Ok(Rc::clone(&self.cooking))
    }
}
fn setup() -> (Rc<RefCell<GameCharacter<()>>>, Host) {
    let (sm, mut chars) = super::super::tests::fixture();
    let mut player = super::super::tests::player(&sm, &mut chars);
    player.descriptor = None;
    let mut registry = Registry::new("test");
    let mut key = |name| {
        registry
            .register(ResourceLocation::of(name).unwrap(), name, None)
            .unwrap()
    };
    let no = key("NoDetect");
    let show = key("ShowPoison");
    let food = key("Food");
    let cooking = chars.perks.new_perk("Cooking").unwrap();
    (
        Rc::new(RefCell::new(player)),
        Host {
            no,
            show,
            food,
            cooking,
            recipe: RefCell::new(None),
            calls: RefCell::default(),
            sandbox: Cell::new(false),
            debug: Cell::new(false),
            fail: Cell::new(None),
        },
    )
}
fn script_definition() -> ScriptItemRef {
    let sm = ScriptManager::new();
    sm.enable_texture_capture();
    sm.parse_script(pz_scripts::ScriptLoadMode::Init, "module Base {}")
        .unwrap();
    let module = sm.get_module("Base").unwrap();
    let mut item = pz_scripts::objects::items::Item::new();
    item.set_module(&module);
    item.init_load_pp(&sm, "Poison");
    item.load(&sm, "Poison", "item Poison {} ").unwrap();
    ScriptItemRef(Rc::new(RefCell::new(item)))
}
fn item(class: ItemClass, definition: &ScriptItemRef) -> ItemRef {
    struct Env;
    impl pz_inventory::ItemEnv for Env {}
    let mut template = ItemTemplate::new(class, "Base", "Poison");
    template.texture_capture = Some(CapturedScriptTextures::new(None, vec![]));
    template.script = Some(definition.clone());
    pz_inventory::env::create_from_template_ref(&Env, Rc::new(template), 1).unwrap()
}
fn food(item: &ItemRef, f: impl FnOnce(&mut pz_inventory::item::FoodData)) {
    let mut item = item.borrow_mut();
    let ItemKind::Food(food) = &mut item.kind else {
        panic!("Food")
    };
    f(food);
}
struct Drain;
impl Drop for Drain {
    fn drop(&mut self) {
        pz_inventory::component_host::drain_retired_components();
    }
}

#[test]
fn tag_precedence_nonfood_and_actual_script_beat_unused_bodies_and_template() {
    let _drain = Drain;
    let (character, host) = setup();
    let definition = script_definition();
    definition
        .0
        .borrow_mut()
        .item_tags
        .add(Some(host.no.clone()));
    definition
        .0
        .borrow_mut()
        .item_tags
        .add(Some(host.show.clone()));
    let item = item(ItemClass::Normal, &definition);
    let held = character.borrow_mut();
    assert!(!is_known_poison_item(&character, &item, &host).unwrap());
    assert_eq!(*host.calls.borrow(), ["NO_DETECT"]);
    drop(held);
    let other = script_definition();
    other.0.borrow_mut().item_tags.add(Some(host.show.clone()));
    if let ItemTextures::Live(live) = &mut item.borrow_mut().textures {
        live.script_item = Some(other);
    }
    assert!(
        is_known_poison_item(&character, &item, &host).unwrap(),
        "actual script differs from blueprint"
    );
    if let ItemTextures::Live(live) = &mut item.borrow_mut().textures {
        live.script_item = Some(script_definition());
    }
    assert!(!is_known_poison_item(&character, &item, &host).unwrap());
    assert!(item.borrow().mod_data_slot().is_none());
}
#[test]
fn herbalist_false_is_final_and_found_recipe_uses_current_original_name_and_sandbox() {
    let _drain = Drain;
    let (character, host) = setup();
    let definition = script_definition();
    let item = item(ItemClass::Food, &definition);
    food(&item, |food| {
        food.poison_power = 3;
        food.herbalist_type = Some("Berry".into());
        food.poison_detection_level = 99;
    });
    let table = pz_inventory::moddata::get_mod_data_ref(&item).unwrap();
    table.set_str("addedPoisonBy", "Bob Smith");
    host.fail.set(Some("Cooking"));
    assert!(!is_known_poison_item(&character, &item, &host).unwrap());
    assert_eq!(
        *host.calls.borrow(),
        [
            "NO_DETECT",
            "SHOW_POISON",
            "recipe",
            "debug",
            "client",
            "server"
        ]
    );
    let recipe = Rc::new(RefCell::new(Recipe::new()));
    recipe
        .borrow_mut()
        .set_originalname(Some("CurrentHerbs".into()));
    *host.recipe.borrow_mut() = Some(recipe.clone());
    character
        .borrow_mut()
        .known_recipes
        .push("Herbalist".into());
    assert!(
        !is_known_poison_item(&character, &item, &host).unwrap(),
        "found Recipe uses its current originalname"
    );
    character
        .borrow_mut()
        .known_recipes
        .push("CurrentHerbs".into());
    assert!(is_known_poison_item(&character, &item, &host).unwrap());
    let held = recipe.borrow_mut();
    host.sandbox.set(true);
    assert!(
        is_known_poison_item(&character, &item, &host).unwrap(),
        "sandbox true skips recipe body"
    );
    drop(held);
}
#[test]
fn canonical_cooking_identity_raw_level_and_script_fallback_are_distinct() {
    let _drain = Drain;
    let (character, host) = setup();
    let definition = script_definition();
    definition.0.borrow_mut().item_type = Some(host.food.clone());
    definition.0.borrow_mut().poison_power = 3;
    definition.0.borrow_mut().poison_detection_level = 2;
    let item = item(ItemClass::Food, &definition);
    food(&item, |food| food.poison_detection_level = 2);
    character
        .borrow_mut()
        .perk_list
        .push(super::super::xp::PerkInfo {
            level: 8,
            perk: host.cooking.clone(),
        });
    let held = host.cooking.borrow_mut();
    assert!(is_known_poison_item(&character, &item, &host).unwrap());
    drop(held);
    character.borrow_mut().perk_list[0].level = 7;
    assert!(!is_known_poison_item(&character, &item, &host).unwrap());
    assert!(
        is_known_poison_script(&character, &definition, &host).unwrap(),
        "script final nonnegative detection fallback"
    );
    character.borrow_mut().perk_list[0].level = 99;
    let shadow = crate::PerkFactory::default().new_perk("Cooking").unwrap();
    character.borrow_mut().perk_list[0].perk = shadow;
    assert!(
        !is_known_poison_item(&character, &item, &host).unwrap(),
        "same id shadow is not static Cooking"
    );
}
#[test]
fn fullname_precedes_lazy_item_table_and_string_equals_uses_actual_metatable_lookup() {
    let _drain = Drain;
    let (character, host) = setup();
    let definition = script_definition();
    let item = item(ItemClass::Food, &definition);
    food(&item, |food| {
        food.poison_power = 2;
        food.poison_detection_level = -1;
    });
    assert!(item.borrow().mod_data_slot().is_none());
    let held = character.borrow_mut();
    assert!(matches!(
        is_known_poison_item(&character, &item, &host),
        Err(RuntimeHostError::Borrowed("character full name"))
    ));
    assert!(item.borrow().mod_data_slot().is_none());
    drop(held);
    let table = pz_inventory::moddata::get_mod_data_ref(&item).unwrap();
    table.set_str("addedPoisonBy", 3.0);
    assert!(!is_known_poison_item(&character, &item, &host).unwrap());
    table.set_str("addedPoisonBy", Value::Nil);
    let parent = pz_kahlua::TableRef::new();
    parent.set_str("addedPoisonBy", "Bob Smith");
    let mut retired = pz_kahlua::table::RetiredTableValues::default();
    table
        .try_set_metatable(&Some(parent), &mut retired)
        .unwrap();
    assert!(is_known_poison_item(&character, &item, &host).unwrap());
    let held = table.try_borrow_body_mut().unwrap();
    assert!(matches!(
        is_known_poison_item(&character, &item, &host),
        Err(RuntimeHostError::Borrowed(
            "known poison addedPoisonBy table"
        ))
    ));
    drop(held);
}
#[test]
fn script_overload_ignores_no_detect_and_never_uses_item_moddata_or_fullname() {
    let _drain = Drain;
    let (character, host) = setup();
    let definition = script_definition();
    definition
        .0
        .borrow_mut()
        .item_tags
        .add(Some(host.no.clone()));
    definition
        .0
        .borrow_mut()
        .item_tags
        .add(Some(host.show.clone()));
    let held = character.borrow_mut();
    assert!(is_known_poison_script(&character, &definition, &host).unwrap());
    assert_eq!(*host.calls.borrow(), ["SHOW_POISON"]);
    drop(held);
}

#[test]
fn actual_descriptor_names_and_borrow_precede_item_moddata_allocation() {
    let _drain = Drain;
    let (character, host) = setup();
    let (sm, mut chars) = super::super::tests::fixture();
    let described = super::super::tests::player(&sm, &mut chars);
    let descriptor = described.descriptor.unwrap();
    character.borrow_mut().descriptor = Some(descriptor.clone());
    descriptor.borrow_mut().forename = None;
    descriptor.borrow_mut().surname = None;
    let definition = script_definition();
    let item = item(ItemClass::Food, &definition);
    food(&item, |food| {
        food.poison_power = 1;
        food.poison_detection_level = -1;
    });
    let held = descriptor.borrow_mut();
    assert!(matches!(
        is_known_poison_item(&character, &item, &host),
        Err(RuntimeHostError::Borrowed("character descriptor full name"))
    ));
    assert!(item.borrow().mod_data_slot().is_none());
    drop(held);
    let table = pz_inventory::moddata::get_mod_data_ref(&item).unwrap();
    table.set_str("addedPoisonBy", "null null");
    assert!(is_known_poison_item(&character, &item, &host).unwrap());
    descriptor.borrow_mut().forename = Some(String::new());
    descriptor.borrow_mut().surname = Some("Last".into());
    assert!(!is_known_poison_item(&character, &item, &host).unwrap());
    table.set_str("addedPoisonBy", " Last");
    assert!(is_known_poison_item(&character, &item, &host).unwrap());
}

#[test]
fn found_recipe_cheat_shortcircuits_sandbox_and_recipe_body_but_not_lookup() {
    let _drain = Drain;
    let (character, host) = setup();
    struct Debug;
    impl super::super::context::SimContext for Debug {
        fn is_debug(&self) -> bool {
            true
        }
    }
    character
        .borrow_mut()
        .cheats
        .set(&Debug, "KNOW_ALL_RECIPES", true);
    host.debug.set(true);
    host.fail.set(Some("sandbox"));
    let recipe = Rc::new(RefCell::new(Recipe::new()));
    *host.recipe.borrow_mut() = Some(recipe.clone());
    let definition = script_definition();
    let item = item(ItemClass::Food, &definition);
    food(&item, |food| {
        food.poison_power = 3;
        food.herbalist_type = Some("Herb".into());
    });
    let held = recipe.borrow_mut();
    assert!(is_known_poison_item(&character, &item, &host).unwrap());
    assert_eq!(
        *host.calls.borrow(),
        ["NO_DETECT", "SHOW_POISON", "recipe", "debug"]
    );
    drop(held);
    host.fail.set(Some("recipe"));
    assert!(matches!(
        is_known_poison_item(&character, &item, &host),
        Err(RuntimeHostError::Borrowed("recipe"))
    ));
}

#[test]
fn cyclic_metatable_is_an_explicit_host_limit_not_missing_poison_or_borrow_fault() {
    let _drain = Drain;
    let (character, host) = setup();
    let definition = script_definition();
    let item = item(ItemClass::Food, &definition);
    food(&item, |food| {
        food.poison_power = 1;
        food.poison_detection_level = -1;
    });
    let table = pz_inventory::moddata::get_mod_data_ref(&item).unwrap();
    let mut retired = pz_kahlua::table::RetiredTableValues::default();
    table
        .try_set_metatable(&Some(table.clone()), &mut retired)
        .unwrap();
    assert!(matches!(
        is_known_poison_item(&character, &item, &host),
        Err(RuntimeHostError::Unsupported {
            operation: "known poison metatable traversal boundary",
            ..
        })
    ));
    table.try_set_metatable(&None, &mut retired).unwrap();
}

#[test]
fn animal_fullname_override_is_required_only_at_late_fallback_before_moddata() {
    let _drain = Drain;
    let (character, host) = setup();
    character.borrow_mut().kind = crate::CharacterKind::Animal;
    let definition = script_definition();
    let item = item(ItemClass::Food, &definition);
    food(&item, |food| {
        food.poison_power = 1;
        food.poison_detection_level = -1;
    });
    assert!(matches!(
        is_known_poison_item(&character, &item, &host),
        Err(RuntimeHostError::Unsupported {
            operation: "IsoAnimal.getFullName actual animal data/translation",
            ..
        })
    ));
    assert!(item.borrow().mod_data_slot().is_none());
    definition
        .0
        .borrow_mut()
        .item_tags
        .add(Some(host.show.clone()));
    assert!(is_known_poison_item(&character, &item, &host).unwrap());
}

#[test]
fn actual_cheats_field_is_acquired_before_failing_debug_or_true_sandbox() {
    let _drain = Drain;
    let (character, host) = setup();
    let definition = script_definition();
    let item = item(ItemClass::Food, &definition);
    food(&item, |food| {
        food.poison_power = 3;
        food.herbalist_type = Some("Herb".into());
    });
    *host.recipe.borrow_mut() = Some(Rc::new(RefCell::new(Recipe::new())));
    host.fail.set(Some("debug"));
    host.sandbox.set(true);
    let held = character.borrow_mut();
    assert!(matches!(
        is_known_poison_item(&character, &item, &host),
        Err(RuntimeHostError::Borrowed("character cheats field"))
    ));
    assert_eq!(*host.calls.borrow(), ["NO_DETECT", "SHOW_POISON", "recipe"]);
    drop(held);
    assert!(matches!(
        is_known_poison_item(&character, &item, &host),
        Err(RuntimeHostError::Borrowed("debug"))
    ));
}

#[test]
fn sandbox_return_then_both_held_faults_known_list_before_recipe_originalname() {
    let _drain = Drain;
    let (character, base) = setup();
    let definition = script_definition();
    let item = item(ItemClass::Food, &definition);
    food(&item, |food| {
        food.poison_power = 3;
        food.herbalist_type = Some("Herb".into());
    });
    let recipe = Rc::new(RefCell::new(Recipe::new()));
    *base.recipe.borrow_mut() = Some(recipe.clone());
    // Sandbox is the one phase allowed to execute user code. Simulate an
    // externally retained actual character body guard acquired at that phase,
    // after getCheats completed and before native getKnownRecipes PC22.
    struct SandboxLease<'a> {
        base: &'a Host,
        character: &'a Rc<RefCell<GameCharacter<()>>>,
        held: RefCell<Option<std::cell::RefMut<'a, GameCharacter<()>>>>,
    }
    impl KnownPoisonHost for SandboxLease<'_> {
        fn item_tag(&self, field: &'static str) -> HostResult<Rc<RegistryKey>> {
            self.base.item_tag(field)
        }
        fn food_item_type(&self) -> HostResult<Rc<RegistryKey>> {
            self.base.food_item_type()
        }
        fn recipe(&self, name: &str) -> HostResult<Option<Rc<RefCell<Recipe>>>> {
            self.base.recipe(name)
        }
        fn is_debug(&self) -> HostResult<bool> {
            self.base.is_debug()
        }
        fn is_client(&self) -> HostResult<bool> {
            self.base.is_client()
        }
        fn is_server(&self) -> HostResult<bool> {
            self.base.is_server()
        }
        fn cooking_perk(&self) -> HostResult<PerkRef> {
            self.base.cooking_perk()
        }
        fn see_not_learnt_recipe(&self) -> HostResult<bool> {
            let character = self.character;
            *self.held.borrow_mut() = Some(character.borrow_mut());
            Ok(false)
        }
    }
    let host = SandboxLease {
        base: &base,
        character: &character,
        held: RefCell::new(None),
    };
    let held_recipe = recipe.borrow_mut();
    assert!(matches!(
        is_known_poison_item(&character, &item, &host),
        Err(RuntimeHostError::Borrowed("character known recipes"))
    ));
    drop(host.held.borrow_mut().take());
    drop(held_recipe);
    assert_eq!(
        *base.calls.borrow(),
        [
            "NO_DETECT",
            "SHOW_POISON",
            "recipe",
            "debug",
            "client",
            "server"
        ]
    );
}
