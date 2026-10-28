use super::*;
use crate::io::{Reader, Writer};
use crate::item_texture::CapturedScriptTextures;
use crate::template::ScriptItemRef;
use crate::{InventoryItem, ItemEnv};
use pz_kahlua::table::RetiredTableValues;
use pz_scripts::ScriptObject;
use std::cell::RefCell;

struct ProviderRestore(Option<pz_render::TextureRefLoader>);
impl Drop for ProviderRestore {
    fn drop(&mut self) {
        drop(pz_render::install_texture_ref_loader(self.0.take()));
        crate::component_host::drain_retired_components();
    }
}

struct Env;
impl ItemEnv for Env {}

fn template() -> ItemTemplate {
    let mut t = ItemTemplate::new(ItemClass::Literature, "Base", "Reading");
    t.texture_capture = Some(CapturedScriptTextures::new(None, Vec::new()));
    t
}
fn script() -> ScriptItemRef {
    let sm = pz_scripts::ScriptManager::new();
    sm.enable_texture_capture();
    sm.parse_script(pz_scripts::ScriptLoadMode::Init, "module Base {} ")
        .unwrap();
    let mut definition = pz_scripts::objects::items::Item::new();
    definition.set_module(&sm.get_module("Base").unwrap());
    definition.init_load_pp(&sm, "Reading");
    definition
        .load(&sm, "Reading", "item Reading {}")
        .unwrap();
    assert!(definition.texture_capture_complete());
    ScriptItemRef(Rc::new(RefCell::new(definition)))
}
fn actual(t: ItemTemplate) -> ItemRef {
    crate::env::create_from_template_ref(&Env, Rc::new(t), 1).unwrap()
}
fn edit(item: &ItemRef, f: impl FnOnce(&mut LiteratureData)) {
    let mut body = item.borrow_mut();
    let ItemKind::Literature(v) = &mut body.kind else {
        panic!("Literature fixture")
    };
    f(v);
}
fn with_script() -> (ItemRef, ScriptItemRef) {
    let s = script();
    let mut t = template();
    t.script = Some(s.clone());
    (actual(t), s)
}

#[test]
fn failed_super_retains_java_defaults_before_literature_initializers() {
    let mut t = template();
    t.texture_capture = None;
    t.number_of_pages = 999;
    t.skill_trained = Some("Template".into());
    t.lvl_skill_trained = 99;
    t.num_levels_trained = 77;
    let failure = crate::env::create_from_template_ref(&Env, Rc::new(t), 1).unwrap_err();
    let item = failure.item.as_ref().unwrap();
    assert!(matches!(
        failure.error,
        RuntimeHostError::Unsupported { .. }
    ));
    assert_eq!(get_number_of_pages_ref(item).unwrap(), 0);
    assert_eq!(get_skill_trained_ref(item).unwrap(), None);
    assert_eq!(get_lvl_skill_trained_ref(item).unwrap(), 0);
    assert_eq!(get_num_levels_trained_ref(item).unwrap(), 0);
    failure.into_error_and_retire();
    crate::component_host::drain_retired_components();
}

#[test]
fn allocation_observer_sees_raw_fields_before_any_constructor_or_failure() {
    thread_local! { static OBSERVED: RefCell<Vec<(i32, i32, i32, Option<String>)>> = const { RefCell::new(Vec::new()) }; }
    fn observe(item: &ItemRef) {
        let body = item.borrow();
        if let ItemKind::Literature(v) = &body.kind {
            OBSERVED.with(|values| {
                values.borrow_mut().push((
                    v.number_of_pages,
                    v.lvl_skill_trained,
                    v.num_levels_trained,
                    v.skill_trained.clone(),
                ))
            });
        }
    }
    struct Restore(Option<crate::shared::ItemObserver>);
    impl Drop for Restore {
        fn drop(&mut self) {
            crate::shared::install_item_observer(self.0);
        }
    }
    OBSERVED.with(|values| values.borrow_mut().clear());
    let _restore = Restore(crate::shared::install_item_observer(Some(observe)));
    let mut t = template();
    t.texture_capture = None;
    t.number_of_pages = 919;
    t.lvl_skill_trained = 17;
    t.num_levels_trained = 18;
    t.skill_trained = Some("Template".into());
    let failure = crate::env::create_from_template_ref(&Env, Rc::new(t), 1).unwrap_err();
    OBSERVED.with(|values| assert_eq!(values.borrow().first(), Some(&(0, 0, 0, None))));
    failure.into_error_and_retire();
    crate::component_host::drain_retired_components();
}

#[test]
fn string_super_failure_and_completed_constructor_have_distinct_field_prefixes() {
    let _restore = ProviderRestore(pz_render::install_texture_ref_loader(Some(Rc::new(
        |_, _| {
            Err(pz_render::TextureHostError::Borrowed(
                "actual constructor provider",
            ))
        },
    ))));
    let item = crate::shared::try_new_item(InventoryItem::create(Rc::new(template()), 1)).unwrap();
    let result = crate::item_texture::constructor_string_ref(&item, "Item_Reading");
    assert!(matches!(result, Err(RuntimeHostError::Borrowed(_))));
    assert_eq!(get_number_of_pages_ref(&item).unwrap(), 0);
    assert_eq!(get_skill_trained_ref(&item).unwrap(), None);
    drop(pz_render::install_texture_ref_loader(Some(Rc::new(
        |_, _| Ok(None),
    ))));
    crate::item_texture::constructor_string_ref(&item, "Item_Reading").unwrap();
    assert_eq!(get_number_of_pages_ref(&item).unwrap(), -1);
    assert_eq!(get_lvl_skill_trained_ref(&item).unwrap(), -1);
    assert_eq!(get_num_levels_trained_ref(&item).unwrap(), 0);
    assert_eq!(
        get_skill_trained_ref(&item).unwrap().as_deref(),
        Some("None")
    );
    assert_eq!(get_max_level_trained_ref(&item).unwrap(), -2);
    crate::component_host::drain_retired_components();
}

#[test]
fn factory_copies_current_argument_then_getters_keep_independent_instance_fields() {
    let s = script();
    {
        let mut v = s.0.borrow_mut();
        v.number_of_pages = 240;
        v.skill_trained = Some("Carpentry".into());
        v.lvl_skill_trained = 3;
        v.num_levels_trained = 2;
        v.can_be_write = true;
        v.page_to_write = 17;
    }
    let mut t = template();
    t.script = Some(s.clone());
    t.number_of_pages = 999;
    t.skill_trained = Some("WrongTemplate".into());
    t.lvl_skill_trained = 90;
    t.num_levels_trained = 80;
    let item = actual(t);
    {
        let mut current = s.0.borrow_mut();
        current.number_of_pages = 700;
        current.skill_trained = None;
        current.lvl_skill_trained = -50;
        current.num_levels_trained = 30;
        // Scalar getters never reread a later script or its borrowed body.
        assert_eq!(get_number_of_pages_ref(&item).unwrap(), 240);
        assert_eq!(
            get_skill_trained_ref(&item).unwrap().as_deref(),
            Some("Carpentry")
        );
        assert_eq!(get_lvl_skill_trained_ref(&item).unwrap(), 3);
        assert_eq!(get_num_levels_trained_ref(&item).unwrap(), 2);
        assert_eq!(get_max_level_trained_ref(&item).unwrap(), 4);
    }
    edit(&item, |v| {
        assert!(v.can_be_write);
        assert_eq!(v.page_to_write, 17);
    });
    let _held = item.borrow_mut();
    assert!(matches!(
        get_skill_trained_ref(&item),
        Err(RuntimeHostError::Borrowed(_))
    ));
    assert!(matches!(
        get_number_of_pages_ref(&item),
        Err(RuntimeHostError::Borrowed(_))
    ));
}

#[test]
fn later_mask_failure_keeps_current_script_literature_prefix_on_same_receiver() {
    let s = script();
    {
        let mut current = s.0.borrow_mut();
        current.number_of_pages = 81;
        current.skill_trained = Some("Cooking".into());
        current.lvl_skill_trained = 5;
        current.num_levels_trained = 2;
        current.icon_color_mask = Some("FailReadingMask".into());
    }
    let _restore = ProviderRestore(pz_render::install_texture_ref_loader(Some(Rc::new(
        |_, _| {
            Err(pz_render::TextureHostError::Borrowed(
                "actual mask provider",
            ))
        },
    ))));
    let mut t = template();
    t.script = Some(s.clone());
    t.number_of_pages = 999;
    t.skill_trained = Some("WrongTemplate".into());
    let failure = crate::env::create_from_template_ref(&Env, Rc::new(t), 1).unwrap_err();
    assert!(matches!(failure.error, RuntimeHostError::Borrowed(_)));
    let item = failure.item.as_ref().unwrap();
    assert_eq!(get_number_of_pages_ref(item).unwrap(), 81);
    assert_eq!(
        get_skill_trained_ref(item).unwrap().as_deref(),
        Some("Cooking")
    );
    assert_eq!(get_max_level_trained_ref(item).unwrap(), 6);
    assert_eq!(
        crate::item_texture::get_script_item_ref(item).unwrap(),
        Some(s)
    );
    failure.into_error_and_retire();
    crate::component_host::drain_retired_components();
}

#[test]
fn native_runtime_max_wraps_and_does_not_use_script_sentinel_or_formula() {
    let item = actual(template());
    edit(&item, |v| {
        v.lvl_skill_trained = i32::MAX;
        v.num_levels_trained = 2;
    });
    assert_eq!(get_max_level_trained_ref(&item).unwrap(), i32::MIN);
    edit(&item, |v| {
        v.lvl_skill_trained = -1;
        v.num_levels_trained = 8;
    });
    assert_eq!(get_max_level_trained_ref(&item).unwrap(), 6);
    let mut definition = pz_scripts::objects::items::Item::new();
    definition.lvl_skill_trained = -1;
    definition.num_levels_trained = 8;
    assert_eq!(definition.max_level_trained(), -1);
    definition.lvl_skill_trained = i32::MAX;
    definition.num_levels_trained = 2;
    assert_eq!(definition.max_level_trained(), i32::MIN + 1);
}

#[test]
fn wire_load_resets_saved_fields_but_preserves_unsaved_trained_fields_and_partial_prefix() {
    let item = actual(template());
    edit(&item, |v| {
        v.skill_trained = Some("Electrical".into());
        v.lvl_skill_trained = 9;
        v.num_levels_trained = 2;
        v.page_to_write = 33;
        v.number_of_pages = 444;
        v.learned_recipes = Some(vec!["ArchiveOverride".into()]);
    });
    // A truncated header fails after native subclass reset, not before it.
    assert!(item
        .borrow_mut()
        .load_kind_native(&mut Reader::new(&[]), 249, &Env)
        .is_err());
    assert_eq!(get_number_of_pages_ref(&item).unwrap(), -1);
    assert_eq!(
        get_skill_trained_ref(&item).unwrap().as_deref(),
        Some("Electrical")
    );
    assert_eq!(get_max_level_trained_ref(&item).unwrap(), 10);
    edit(&item, |v| {
        assert_eq!(v.page_to_write, 33);
        assert!(v.learned_recipes.is_none());
    });
    let mut reader = Reader::new(&[0, 0]);
    item.borrow_mut()
        .load_kind_native(&mut reader, 249, &Env)
        .unwrap();
    assert_eq!(reader.pos, 2);
    assert_eq!(get_lvl_skill_trained_ref(&item).unwrap(), 9);
    assert_eq!(get_num_levels_trained_ref(&item).unwrap(), 2);
}

#[test]
fn unsaved_training_fields_do_not_change_existing_literature_save_wire() {
    let mut value = InventoryItem::create(Rc::new(template()), 1);
    let mut before = Writer::new();
    value.save(&mut before, &Env).unwrap();
    let ItemKind::Literature(v) = &mut value.kind else {
        panic!("Literature fixture")
    };
    v.skill_trained = Some("MetalWelding".into());
    v.lvl_skill_trained = 42;
    v.num_levels_trained = -15;
    v.page_to_write = i32::MAX;
    let mut after = Writer::new();
    value.save(&mut after, &Env).unwrap();
    assert_eq!(before.buf, after.buf);
}

#[test]
fn genuine_null_recipe_list_allocates_real_moddata_and_returns_null() {
    let (item, _script) = with_script();
    assert!(item.borrow().table.is_none());
    assert_eq!(
        get_learned_recipes_ref(&item).unwrap(),
        LearnedRecipesResult::Null
    );
    let table = crate::moddata::get_mod_data_ref(&item).unwrap();
    assert!(table
        .try_rawget(&Value::str("learnedRecipe"))
        .unwrap()
        .is_nil());
    assert!(item.borrow().table.is_some());
    crate::component_host::drain_retired_components();
}

#[test]
fn recipe_moddata_read_precedes_override_and_held_script_or_nonnull_list() {
    let (item, s) = with_script();
    let table = crate::moddata::get_mod_data_ref(&item).unwrap();
    edit(&item, |v| v.learned_recipes = Some(Vec::new()));
    {
        let _table = table.try_borrow_body_mut().unwrap();
        assert!(matches!(
            get_learned_recipes_ref(&item),
            Err(RuntimeHostError::Borrowed(_))
        ));
    }
    assert!(matches!(
        get_learned_recipes_ref(&item),
        Err(RuntimeHostError::Unsupported {
            operation: "Literature.getLearnedRecipes actual override List return",
            ..
        })
    ));
    edit(&item, |v| v.learned_recipes = None);
    {
        let _script = s.0.borrow_mut();
        assert!(matches!(
            get_learned_recipes_ref(&item),
            Err(RuntimeHostError::Borrowed(
                "Literature.getLearnedRecipes script"
            ))
        ));
    }
    s.0.borrow_mut().learned_recipes = Some(Vec::new());
    assert!(matches!(
        get_learned_recipes_ref(&item),
        Err(RuntimeHostError::Unsupported {
            operation: "Literature.getLearnedRecipes actual script List return",
            ..
        })
    ));
    crate::component_host::drain_retired_components();
}

#[test]
fn nonnull_recipe_key_stops_before_list_allocation_reread_cast_publish_and_remove() {
    let (item, s) = with_script();
    let table = crate::moddata::get_mod_data_ref(&item).unwrap();
    let mut values = RetiredTableValues::new();
    table
        .try_rawset(
            &Value::str("learnedRecipe"),
            &Value::Bool(false),
            &mut values,
        )
        .unwrap();
    {
        let _script = s.0.borrow_mut();
        assert!(matches!(
            get_learned_recipes_ref(&item),
            Err(RuntimeHostError::Unsupported {
                operation: "Literature.getLearnedRecipes ArrayList allocation",
                ..
            })
        ));
    }
    assert!(matches!(
        table.try_rawget(&Value::str("learnedRecipe")).unwrap(),
        Value::Bool(false)
    ));
    edit(&item, |v| assert!(v.learned_recipes.is_none()));
    crate::component_host::drain_retired_components();
}

#[test]
fn null_actual_script_does_not_fall_back_to_template_script_or_genuine_null_list() {
    let (item, _s) = with_script();
    crate::item_texture::set_script_item_ref(&item, None).unwrap();
    assert!(matches!(
        get_learned_recipes_ref(&item),
        Err(RuntimeHostError::Unsupported {
            operation: "Literature.getLearnedRecipes null scriptItem receiver",
            ..
        })
    ));
    assert!(item.borrow().table.is_some());
    assert!(item.borrow().template.script.is_some());
    crate::component_host::drain_retired_components();
}
