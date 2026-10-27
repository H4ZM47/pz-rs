use super::*;
use crate::io::{Reader, Writer};
use crate::{ItemClass, ItemEnv, ItemTemplate};
use pz_render::{LuaTexture, Texture, TextureLookup, TextureRefLoader};
use std::cell::{Cell, RefCell};

struct Restore(Option<TextureRefLoader>, bool);
impl Restore {
    fn new() -> Self {
        Self(
            pz_render::install_texture_ref_loader(None),
            pz_render::warn_fail_find_texture(),
        )
    }
}
impl Drop for Restore {
    fn drop(&mut self) {
        drop(pz_render::install_texture_ref_loader(self.0.take()));
        pz_render::set_warn_fail_find_texture(self.1);
        crate::component_host::drain_retired_components();
    }
}
struct Env;
impl ItemEnv for Env {}
fn texture(name: &str) -> TextureRef {
    Rc::new(LuaTexture::new(Texture::empty(name)))
}
fn template(class: ItemClass, base: Option<TextureRef>) -> ItemTemplate {
    let mut t = ItemTemplate::new(class, "Base", "ActualTexture");
    t.texture_capture = Some(CapturedScriptTextures::new(base, Vec::new()));
    t
}
fn actual(t: ItemTemplate) -> ItemRef {
    crate::env::create_from_template_ref(&Env, Rc::new(t), 1).unwrap()
}
fn pointer(a: Option<TextureRef>, b: &TextureRef) {
    assert!(Rc::ptr_eq(&a.unwrap(), b));
}
fn food(item: &ItemRef, f: impl FnOnce(&mut crate::item::FoodData)) {
    let mut item = item.borrow_mut();
    let ItemKind::Food(food) = &mut item.kind else {
        panic!("Food fixture")
    };
    f(food);
}
fn script(icon: &str) -> ScriptItemRef {
    use pz_scripts::ScriptObject;
    let sm = pz_scripts::ScriptManager::new();
    sm.parse_script(pz_scripts::ScriptLoadMode::Init, "module Base {} ")
        .unwrap();
    sm.enable_texture_capture();
    let module = sm.get_module("Base").unwrap();
    let mut item = pz_scripts::objects::items::Item::new();
    item.set_module(&module);
    item.init_load_pp(&sm, "ActualTexture");
    item.load(
        &sm,
        "ActualTexture",
        &format!("item ActualTexture {{ Icon = {icon}, }}"),
    )
    .unwrap();
    assert!(item.texture_capture_complete());
    ScriptItemRef(Rc::new(RefCell::new(item)))
}

#[test]
fn script_capture_same_rc_getters_work_under_held_texture_and_survive_script_replacement() {
    let _restore = Restore::new();
    let base = texture("Item_First");
    let second = texture("Item_Second");
    let b = base.clone();
    let s = second.clone();
    pz_render::install_texture_ref_loader(Some(Rc::new(move |_, name| {
        Ok(match name {
            "Item_First" => Some(b.clone()),
            "Item_Second" => Some(s.clone()),
            _ => None,
        })
    })));
    let definition = script("First");
    let mut t = template(ItemClass::Normal, None);
    t.script = Some(definition.clone());
    let item = actual(t);
    pointer(get_tex_ref(&item).unwrap(), &base);
    pointer(get_icon_ref(&item).unwrap(), &base);
    {
        let _body = base.0.borrow_mut();
        pointer(get_texture_ref(&item).unwrap(), &base);
        assert!(matches!(
            pz_render::try_texture_descriptor(&base),
            Err(TextureHostError::Borrowed(_))
        ));
        let copy = item.borrow().try_archive_copy().unwrap();
        pointer(copy.textures.live().unwrap().slots[0].clone(), &base);
    }
    base.0.borrow_mut().offset_x = 19.0;
    assert_eq!(
        pz_render::try_texture_descriptor(&get_tex_ref(&item).unwrap().unwrap())
            .unwrap()
            .offset_x,
        19.0
    );
    // Real DoParam mutates the actual definition. Existing instance asset is a
    // captured owning field; its script metadata still points to that definition.
    let sm = pz_scripts::ScriptManager::new();
    sm.enable_texture_capture();
    definition
        .0
        .borrow_mut()
        .do_param(&sm, "Icon", "Second")
        .unwrap();
    pointer(
        definition.0.borrow().normal_texture_slot().cloned(),
        &second,
    );
    pointer(get_tex_ref(&item).unwrap(), &base);
    definition.0.borrow_mut().scale_world_icon = 3.25;
    assert_eq!(
        script_scale_world_icon(&get_script_item_ref(&item).unwrap().unwrap()).unwrap(),
        3.25
    );
}

#[test]
fn strict_factory_distinguishes_uncaptured_profile_from_genuine_null_before_rng() {
    struct Draws(Cell<usize>);
    impl ItemEnv for Draws {
        fn rand_next(&self, _: i32) -> i32 {
            self.0.set(self.0.get() + 1);
            0
        }
    }
    let _restore = Restore::new();
    let env = Draws(Cell::new(0));
    let t = Rc::new(ItemTemplate::new(ItemClass::Normal, "Base", "Unknown"));
    let failure = crate::env::create_from_template_ref(&env, t, 1).unwrap_err();
    assert!(matches!(
        failure.error,
        RuntimeHostError::Unsupported { .. }
    ));
    assert_eq!(env.0.get(), 0);
    assert!(failure
        .item
        .as_ref()
        .unwrap()
        .borrow()
        .textures
        .live()
        .unwrap()
        .base_slot()
        .is_none());
    failure.into_error_and_retire();
    let item = actual(template(ItemClass::Normal, None));
    assert!(get_tex_ref(&item).unwrap().is_none());
    let archive = shared::try_new_item(InventoryItem::create(
        Rc::new(ItemTemplate::new(ItemClass::Normal, "Base", "Archive")),
        1,
    ))
    .unwrap();
    assert!(matches!(
        get_tex_ref(&archive),
        Err(RuntimeHostError::Unsupported { .. })
    ));
}

#[test]
fn food_variants_use_captured_null_fallback_and_burnt_rotten_cooked_precedence() {
    let _restore = Restore::new();
    let base = texture("base");
    let rotten = texture("rotten");
    let cooked = texture("cooked");
    let burnt = texture("burnt");
    let custom = texture("custom");
    let c = custom.clone();
    pz_render::install_texture_ref_loader(Some(Rc::new(move |_, name| {
        Ok((name == "Custom").then(|| c.clone()))
    })));
    let mut t = template(ItemClass::Food, Some(base.clone()));
    let capture = t.texture_capture.as_mut().unwrap();
    capture.special = vec![
        Some(rotten.clone()),
        Some(cooked.clone()),
        Some(burnt.clone()),
    ];
    capture.special_world_names = vec![
        "rotten-world".into(),
        "cooked-world".into(),
        "burnt-world".into(),
    ];
    let item = actual(t);
    crate::moddata::set_mod_data_value_ref(
        &item,
        "customInventoryIcon",
        Some(Value::str("Custom")),
    )
    .unwrap();
    pointer(get_texture_ref(&item).unwrap(), &custom);
    food(&item, |f| {
        f.cooked = true;
        f.age = f.off_age_max as f32;
        f.burnt = true;
    });
    let table = crate::moddata::get_mod_data_ref(&item).unwrap();
    let _held = table.try_borrow_body_mut().unwrap();
    pointer(get_tex_ref(&item).unwrap(), &burnt);
    assert_eq!(
        get_world_texture_ref(&item).unwrap().as_deref(),
        Some("burnt-world")
    );
    food(&item, |f| f.burnt = false);
    pointer(get_tex_ref(&item).unwrap(), &rotten);
    food(&item, |f| f.age = f32::NAN);
    pointer(get_tex_ref(&item).unwrap(), &cooked);
    food(&item, |f| f.cooked = false);
    assert!(matches!(
        get_tex_ref(&item),
        Err(RuntimeHostError::Borrowed(_))
    ));
    let fallback = actual(template(ItemClass::Food, Some(base.clone())));
    food(&fallback, |f| f.burnt = true);
    pointer(get_tex_ref(&fallback).unwrap(), &base);
}

#[test]
fn mask_null_write_precedes_fallback_error_and_retirement_and_observers_are_guard_free() {
    let _restore = Restore::new();
    crate::component_host::drain_retired_components();
    let item = actual(template(ItemClass::Normal, None));
    let old = texture("old-mask");
    let weak = Rc::downgrade(&old);
    replace_slot_ref(&item, 1, Some(&old)).unwrap();
    drop(old);
    crate::component_host::drain_retired_components();
    let receiver = Rc::downgrade(&item);
    let calls = Rc::new(RefCell::new(Vec::new()));
    let log = calls.clone();
    pz_render::install_texture_ref_loader(Some(Rc::new(move |lookup, name| {
        assert!(receiver.upgrade().unwrap().try_borrow_mut().is_ok());
        log.borrow_mut().push((lookup, name.to_owned()));
        match lookup {
            TextureLookup::Try => Ok(None),
            TextureLookup::Shared => Err(TextureHostError::Borrowed("Question fallback")),
        }
    })));
    assert!(matches!(
        set_texture_color_mask_ref(&item, "Mask"),
        Err(RuntimeHostError::Borrowed("Question fallback"))
    ));
    assert!(get_texture_color_mask_ref(&item).unwrap().is_none());
    assert!(weak.upgrade().is_some());
    assert_eq!(
        calls.borrow().as_slice(),
        &[
            (TextureLookup::Try, "Mask".into()),
            (TextureLookup::Shared, QUESTION_TEXTURE.into())
        ]
    );
    crate::component_host::drain_retired_components();
    assert!(weak.upgrade().is_none());
}

#[test]
fn string_food_constructor_preserves_suffix_prefix_world_names_and_warning_static_on_error() {
    let _restore = Restore::new();
    let base = texture("Item_Food");
    let rotten = texture("Item_FoodSpoiled");
    let b = base.clone();
    let r = rotten.clone();
    let calls = Rc::new(RefCell::new(Vec::new()));
    let log = calls.clone();
    pz_render::install_texture_ref_loader(Some(Rc::new(move |_, name| {
        log.borrow_mut().push(name.to_owned());
        if name != "Item_Food" {
            assert!(!pz_render::warn_fail_find_texture());
        }
        Ok(match name {
            "Item_Food" => Some(b.clone()),
            "Item_FoodSpoiled" => Some(r.clone()),
            _ => None,
        })
    })));
    let item = shared::try_new_item(InventoryItem::create(
        Rc::new(ItemTemplate::new(ItemClass::Food, "Base", "Food")),
        1,
    ))
    .unwrap();
    constructor_string_ref(&item, "Item_Food").unwrap();
    assert!(pz_render::warn_fail_find_texture());
    food(&item, |f| f.age = f.off_age_max as f32);
    pointer(get_tex_ref(&item).unwrap(), &rotten);
    assert_eq!(
        get_world_texture_ref(&item).unwrap().as_deref(),
        Some("media/inventory/world/WItem_FoodSpoiled.png")
    );
    assert!(!calls.borrow().iter().any(|s| s == "Item_Food_Rotten"));
    pz_render::install_texture_ref_loader(Some(Rc::new(move |_, name| {
        if name == "Item_FoodCooked" {
            Err(TextureHostError::Unavailable("cooked lookup"))
        } else {
            Ok(None)
        }
    })));
    assert!(matches!(
        constructor_string_ref(&item, "Item_Food"),
        Err(RuntimeHostError::Unsupported {
            operation: "cooked lookup",
            ..
        })
    ));
    assert!(!pz_render::warn_fail_find_texture());
    assert!(get_texture_ref(&item).unwrap().is_none());
}

#[test]
fn loaded_texture_is_published_before_later_model_read_error_on_original_cursor() {
    let _restore = Restore::new();
    let old = texture("old");
    let loaded = texture("loaded");
    let l = loaded.clone();
    let item = actual(template(ItemClass::Normal, Some(old.clone())));
    let receiver = Rc::downgrade(&item);
    pz_render::install_texture_ref_loader(Some(Rc::new(move |_, name| {
        assert!(receiver.upgrade().unwrap().try_borrow_mut().is_ok());
        assert_eq!(name, "loaded-name");
        Ok(Some(l.clone()))
    })));
    let mut w = Writer::new();
    w.i32(42);
    w.i8(64);
    w.i32(0x3000_0000);
    w.string("loaded-name");
    let mut r = Reader::new(&w.buf);
    let failure = crate::component_host::load_item_ref(&item, &mut r, 249, &Env).unwrap_err();
    assert_eq!(r.pos, w.buf.len());
    assert_eq!(failure.position, r.pos);
    assert!(matches!(
        failure.error,
        RuntimeHostError::NativeException(_)
    ));
    pointer(get_tex_ref(&item).unwrap(), &loaded);
    assert_eq!(item.borrow().id, 42);
    failure.into_error_and_retire();
}

#[test]
fn sync_first_texture_and_model_index_survive_later_fallback_error_without_color_phase() {
    let _restore = Restore::new();
    let base = texture("base");
    let mut t = template(ItemClass::Normal, Some(base));
    t.icons_for_texture = Some(vec!["Choice".into()]);
    let item = actual(t);
    item.borrow_mut().model_index = 0;
    item.borrow_mut().col = [0.1, 0.2, 0.3, 0.4];
    let receiver = Rc::downgrade(&item);
    pz_render::install_texture_ref_loader(Some(Rc::new(move |lookup, _| {
        assert!(receiver.upgrade().unwrap().try_borrow_mut().is_ok());
        if lookup == TextureLookup::Try {
            Ok(None)
        } else {
            Err(TextureHostError::Borrowed("sync Question"))
        }
    })));
    assert!(matches!(
        synch_with_visual_ref(
            &item,
            None,
            &crate::context::NoOwner,
            &crate::visual::NoVisualRng,
            &mut |_| 0
        ),
        Err(RuntimeHostError::Borrowed("sync Question"))
    ));
    assert!(get_tex_ref(&item).unwrap().is_none());
    assert_eq!(item.borrow().col, [0.1, 0.2, 0.3, 0.4]);
    assert_eq!(item.borrow().model_index, 0);
}

#[test]
fn save_uses_question_pointer_identity_and_keeps_direct_writer_prefix_on_host_error() {
    let _restore = Restore::new();
    let question = texture(QUESTION_TEXTURE);
    let same_name = texture(QUESTION_TEXTURE);
    let q = question.clone();
    pz_render::install_texture_ref_loader(Some(Rc::new(move |_, _| Ok(Some(q.clone())))));
    let definition = script("ScriptIcon");
    let mut t = template(ItemClass::Normal, None);
    t.script = Some(definition);
    let item = actual(t);
    set_texture_ref(&item, Some(&question)).unwrap();
    let mut first = Writer::new();
    save_item_ref(&item, &mut first, &Env).unwrap();
    set_texture_ref(&item, Some(&same_name)).unwrap();
    let mut second = Writer::new();
    save_item_ref(&item, &mut second, &Env).unwrap();
    assert!(second
        .buf
        .windows(QUESTION_TEXTURE.len())
        .any(|x| x == QUESTION_TEXTURE.as_bytes()));
    assert!(!first
        .buf
        .windows(QUESTION_TEXTURE.len())
        .any(|x| x == QUESTION_TEXTURE.as_bytes()));
    pz_render::install_texture_ref_loader(Some(Rc::new(|_, _| {
        Err(TextureHostError::Unavailable("save Question provider"))
    })));
    let mut failed = Writer::new();
    failed.i8(99);
    assert!(matches!(
        save_item_ref(&item, &mut failed, &Env),
        Err(ItemPersistenceError::Host(RuntimeHostError::Unsupported {
            operation: "save Question provider",
            ..
        }))
    ));
    assert!(failed.buf.len() > 8);
    assert_eq!(failed.buf[0], 99);
}

#[test]
fn physical_gc_detach_keeps_all_alias_occurrences_and_strings_and_never_uses_provider() {
    let _restore = Restore::new();
    let base = texture("base");
    let mut t = template(ItemClass::Food, Some(base.clone()));
    t.texture_capture.as_mut().unwrap().world_name = Some("world".into());
    let item = actual(t);
    replace_slot_ref(&item, 1, Some(&base)).unwrap();
    replace_slot_ref(&item, 2, Some(&base)).unwrap();
    let before = Rc::strong_count(&base);
    {
        let body = item.borrow();
        assert_eq!(body.textures.live().unwrap().slots().count(), 6);
        assert_eq!(Rc::strong_count(&base), before);
    }
    let (slots, script) = item.borrow_mut().take_texture_slots_for_gc();
    assert_eq!(slots.len(), 6);
    assert!(slots.iter().all(|slot| Rc::ptr_eq(slot, &base)));
    assert!(script.is_none());
    let body = item.borrow();
    assert!(body.textures.live().unwrap().slots().next().is_none());
    assert_eq!(
        body.textures.live().unwrap().world_name.as_deref(),
        Some("world")
    );
}

#[test]
fn factory_texture_failure_retains_native_visual_seed_and_partial_receiver_before_id_draw() {
    struct Rng(Cell<i64>);
    impl crate::visual::VisualRng for Rng {
        fn get_seed(&self) -> i64 {
            self.0.get()
        }
        fn set_seed(&self, seed: i64) {
            self.0.set(seed);
        }
    }
    struct SeedEnv {
        rng: Rng,
        draws: Cell<usize>,
    }
    impl ItemEnv for SeedEnv {
        fn visual_rng(&self) -> &dyn crate::visual::VisualRng {
            &self.rng
        }
        fn rand_next(&self, bound: i32) -> i32 {
            self.draws.set(self.draws.get() + 1);
            if bound == i32::MAX {
                7
            } else {
                0
            }
        }
    }
    let _restore = Restore::new();
    let env = SeedEnv {
        rng: Rng(Cell::new(19)),
        draws: Cell::new(0),
    };
    pz_render::install_texture_ref_loader(Some(Rc::new(|lookup, _| {
        if lookup == TextureLookup::Try {
            Ok(None)
        } else {
            Err(TextureHostError::Borrowed("factory sync fallback"))
        }
    })));
    let mut t = template(ItemClass::Normal, Some(texture("old-base")));
    t.static_models_by_index = Some(Rc::new(vec!["Model".into()]));
    t.icons_for_texture = Some(vec!["Choice".into()]);
    let failure = crate::env::create_from_template_ref(&env, Rc::new(t), 99).unwrap_err();
    assert!(matches!(
        failure.error,
        RuntimeHostError::Borrowed("factory sync fallback")
    ));
    let partial = failure.item.as_ref().unwrap();
    assert_eq!(partial.borrow().id, 0);
    assert_eq!(partial.borrow().model_index, 0);
    assert!(get_tex_ref(partial).unwrap().is_none());
    assert_eq!(
        env.rng.0.get(),
        7,
        "no finally in native InstanceItem PC4005; failure precedes restorePC4010"
    );
    assert_eq!(
        env.draws.get(),
        2,
        "seed and model draw only; no later item-id draw"
    );
    failure.into_error_and_retire();
}

#[test]
fn factory_mask_recipes_and_script_publication_follow_actual_condition_prefix() {
    let _restore = Restore::new();
    let base = texture("base");
    let b = base.clone();
    let calls = Rc::new(RefCell::new(Vec::new()));
    let log = calls.clone();
    pz_render::install_texture_ref_loader(Some(Rc::new(move |lookup, name| {
        log.borrow_mut().push((lookup, name.to_owned()));
        Ok(Some(b.clone()))
    })));
    let definition = script("Original");
    definition.0.borrow_mut().icon_color_mask = Some("ColorMask".into());
    definition.0.borrow_mut().icon_fluid_mask = Some("FluidMask".into());
    let mut t = template(ItemClass::Normal, None);
    t.script = Some(definition.clone());
    calls.borrow_mut().clear();
    let item = actual(t.clone());
    assert!(Rc::ptr_eq(
        &get_script_item_ref(&item).unwrap().unwrap().0,
        &definition.0
    ));
    assert_eq!(
        calls.borrow().as_slice(),
        &[
            (TextureLookup::Try, "ColorMask.png".into()),
            (TextureLookup::Try, "FluidMask.png".into()),
        ]
    );
    pointer(get_texture_color_mask_ref(&item).unwrap(), &base);
    pointer(get_texture_fluid_mask_ref(&item).unwrap(), &base);
    assert!(!item.borrow().broken);

    calls.borrow_mut().clear();
    t.condition_max = 0;
    t.on_break = Some("ActualOnBreak".into());
    let failure = crate::env::create_from_template_ref(&Env, Rc::new(t), 1).unwrap_err();
    assert!(matches!(
        failure.error,
        RuntimeHostError::Unsupported {
            operation: "Item.InstanceItem setCondition OnBreak callback",
            ..
        }
    ));
    let prefix = failure.item.as_ref().unwrap();
    assert!(prefix.borrow().broken);
    assert_eq!(prefix.borrow().condition, 0);
    assert_eq!(prefix.borrow().id, 0);
    assert!(
        get_script_item_ref(prefix).unwrap().is_none(),
        "PC3398 is after condition callback"
    );
    assert!(
        calls.borrow().is_empty(),
        "mask lookup is after failed condition callback"
    );
    pointer(get_tex_ref(prefix).unwrap(), &base);
    failure.into_error_and_retire();
}

#[test]
fn completed_factory_condition_setter_differs_from_raw_constructor_default() {
    let _restore = Restore::new();
    let mut t = template(ItemClass::Normal, None);
    t.condition_max = 0;
    assert!(!InventoryItem::create(Rc::new(t.clone()), 1).broken);
    let item = actual(t);
    assert_eq!(item.borrow().condition, 0);
    assert!(item.borrow().broken);
}


#[test]
fn food_set_cooked_publishes_state_then_only_raises_time_for_strict_ordered_threshold() {
    let _restore = Restore::new();
    let item = actual(template(ItemClass::Food, None));
    food(&item, |food| {
        food.minutes_to_cook = 23.0;
        food.cooking_time = 2.0;
        food.set_cooked(true);
        assert!(food.cooked);
        assert_eq!(food.cooking_time, 23.0);
        food.cooking_time = 24.0;
        food.set_cooked(true);
        assert_eq!(food.cooking_time, 24.0);
        food.cooking_time = 23.0;
        food.set_cooked(true);
        assert_eq!(food.cooking_time, 23.0);
        food.cooking_time = 1.0;
        food.set_cooked(false);
        assert!(!food.cooked);
        assert_eq!(food.cooking_time, 1.0);
        let nan = f32::from_bits(0x7fc0_0042);
        food.cooking_time = nan;
        food.set_cooked(true);
        assert!(food.cooked);
        assert_eq!(food.cooking_time.to_bits(), nan.to_bits());
        food.minutes_to_cook = nan;
        food.cooking_time = 3.0;
        food.set_cooked(true);
        assert_eq!(food.cooking_time, 3.0);
        food.minutes_to_cook = 0.0;
        food.cooking_time = -0.0;
        food.set_cooked(true);
        assert_eq!(food.cooking_time.to_bits(), (-0.0f32).to_bits());
    });
}

#[test]
fn food_instance_copies_current_script_threshold_after_constructor_without_live_proxy() {
    let _restore = Restore::new();
    pz_render::install_texture_ref_loader(Some(Rc::new(|_, _| Ok(None))));
    let definition = script("Threshold");
    definition.0.borrow_mut().minutes_to_cook = 17;
    let mut t = template(ItemClass::Food, None);
    t.minutes_to_cook = 123;
    t.script = Some(definition.clone());
    let raw = InventoryItem::create(Rc::new(t.clone()), 1);
    let ItemKind::Food(raw_food) = &raw.kind else {
        panic!("Food fixture")
    };
    assert_eq!(
        raw_food.minutes_to_cook, 60.0,
        "constructor default precedes InstanceItem632"
    );
    let item = actual(t);
    food(&item, |food| assert_eq!(food.minutes_to_cook, 17.0));
    definition.0.borrow_mut().minutes_to_cook = 31;
    let held = definition.0.borrow_mut();
    food(&item, |food| {
        food.set_cooked(true);
        assert_eq!(food.minutes_to_cook, 17.0);
        assert_eq!(food.cooking_time, 17.0);
    });
    drop(held);
    let mut archived = template(ItemClass::Food, None);
    archived.minutes_to_cook = 44;
    let value = crate::env::create_from_template(&Env, Rc::new(archived), 1);
    let ItemKind::Food(food) = value.kind else {
        panic!("Food fixture")
    };
    assert_eq!(
        food.minutes_to_cook, 44.0,
        "explicit value factory uses its captured scalar"
    );
}

#[test]
fn food_threshold_is_not_serialized_and_load_preserves_current_receiver_threshold() {
    let _restore = Restore::new();
    let mut t = template(ItemClass::Food, None);
    t.minutes_to_cook = 17;
    let item = actual(t);
    food(&item, |food| food.cooking_time = 2.0);
    let mut first = Writer::new();
    save_item_ref(&item, &mut first, &Env).unwrap();
    food(&item, |food| food.minutes_to_cook = 91.0);
    let mut second = Writer::new();
    save_item_ref(&item, &mut second, &Env).unwrap();
    assert_eq!(first.buf, second.buf, "native save omits minutesToCook");
    let copy = item.borrow().try_archive_copy().unwrap();
    let ItemKind::Food(copy_food) = &copy.kind else {
        panic!("Food fixture")
    };
    assert_eq!(copy_food.minutes_to_cook, 91.0);
    let mut reader = Reader::new(&first.buf[3..]);
    crate::component_host::load_item_ref(&item, &mut reader, 249, &Env).unwrap();
    assert_eq!(reader.pos, first.buf.len() - 3);
    food(&item, |food| {
        assert_eq!(food.minutes_to_cook, 91.0);
        assert_eq!(food.cooking_time, 2.0);
        assert!(!food.cooked);
    });
    let mut reader = Reader::new(&first.buf[3..first.buf.len() - 1]);
    let failure = crate::component_host::load_item_ref(&item, &mut reader, 249, &Env).unwrap_err();
    food(&item, |food| assert_eq!(food.minutes_to_cook, 91.0));
    failure.into_error_and_retire();
}

#[test]
fn food_constructor_default_and_later_instance_copy_survive_texture_failures_at_their_phases() {
    let _restore = Restore::new();
    let mut uncaptured = ItemTemplate::new(ItemClass::Food, "Base", "MissingCapture");
    uncaptured.minutes_to_cook = 99;
    let failure = crate::env::create_from_template_ref(&Env, Rc::new(uncaptured), 1).unwrap_err();
    let item = failure.item.as_ref().unwrap();
    food(item, |food| assert_eq!(food.minutes_to_cook, 60.0));
    failure.into_error_and_retire();

    let mut raw_template = template(ItemClass::Food, None);
    raw_template.minutes_to_cook = 99;
    let raw = crate::shared::try_new_item(InventoryItem::create(Rc::new(raw_template), 1)).unwrap();
    pz_render::install_texture_ref_loader(Some(Rc::new(|_, _| {
        Err(TextureHostError::Unavailable("raw Food texture"))
    })));
    assert!(matches!(
        constructor_string_ref(&raw, "Item_Raw"),
        Err(RuntimeHostError::Unsupported {
            operation: "raw Food texture",
            ..
        })
    ));
    food(&raw, |food| assert_eq!(food.minutes_to_cook, 60.0));

    let mut captured = template(ItemClass::Food, None);
    captured.minutes_to_cook = 27;
    captured.texture_capture.as_mut().unwrap().color_mask_name = Some("Mask".into());
    pz_render::install_texture_ref_loader(Some(Rc::new(|_, name| {
        assert_eq!(name, "Mask.png");
        Err(TextureHostError::Unavailable("later Food mask"))
    })));
    let failure = crate::env::create_from_template_ref(&Env, Rc::new(captured), 1).unwrap_err();
    assert!(matches!(
        failure.error,
        RuntimeHostError::Unsupported {
            operation: "later Food mask",
            ..
        }
    ));
    food(failure.item.as_ref().unwrap(), |food| {
        assert_eq!(food.minutes_to_cook, 27.0)
    });
    failure.into_error_and_retire();
}
