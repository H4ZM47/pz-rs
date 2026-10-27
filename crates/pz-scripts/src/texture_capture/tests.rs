use super::*;
use crate::objects::items::Item;
use crate::{ScriptLoadMode, ScriptObject, ScriptType};
use pz_render::{LuaTexture, Texture, TextureLookup, TextureRefLoader};
use std::collections::HashMap;
use std::rc::Rc;

struct Restore(Option<TextureRefLoader>);
impl Restore {
    fn new() -> Self {
        Self(pz_render::install_texture_ref_loader(None))
    }
}
impl Drop for Restore {
    fn drop(&mut self) {
        let retired = pz_render::install_texture_ref_loader(self.0.take());
        drop(retired);
        drop(take_retired_textures());
    }
}
fn manager() -> (ScriptManager, Rc<crate::ScriptModule>) {
    let sm = ScriptManager::new();
    sm.parse_script(ScriptLoadMode::Init, "module Base {} ")
        .unwrap();
    let module = sm.get_module("Base").unwrap();
    (sm, module)
}
fn texture(name: &str) -> TextureRef {
    Rc::new(LuaTexture::new(Texture::empty(name)))
}
fn item(sm: &ScriptManager, module: &Rc<crate::ScriptModule>, body: &str) -> Item {
    let mut item = Item::new();
    item.set_module(module);
    item.init_load_pp(sm, "Actual");
    item.load(sm, "Actual", body).unwrap();
    item
}
#[test]
fn native_icon_captures_same_base_world_refs_and_question_fallback_in_phase_order() {
    let _restore = Restore::new();
    let (sm, module) = manager();
    sm.enable_texture_capture();
    let base = texture("Question");
    let world = texture("World");
    let b = base.clone();
    let w = world.clone();
    let calls = Rc::new(RefCell::new(Vec::new()));
    let log = calls.clone();
    pz_render::install_texture_ref_loader(Some(Rc::new(move |lookup, name| {
        log.borrow_mut().push((lookup, name.to_string()));
        Ok(match name {
            "media/inventory/Question_On.png" => Some(b.clone()),
            "media/inventory/world/WItem_Missing.png" => Some(w.clone()),
            _ => None,
        })
    })));
    let item = item(&sm, &module, "item Actual { Icon=Missing, Weight=4, }");
    assert!(item.texture_capture_complete());
    assert!(Rc::ptr_eq(item.normal_texture_slot().unwrap(), &base));
    assert!(Rc::ptr_eq(item.world_texture_slot().unwrap(), &world));
    assert_eq!(item.actual_weight, 4.0);
    assert_eq!(
        *calls.borrow(),
        vec![
            (TextureLookup::Try, "Item_Missing".into()),
            (
                TextureLookup::Shared,
                "media/inventory/Question_On.png".into()
            ),
            (
                TextureLookup::Shared,
                "media/inventory/world/WItem_Missing.png".into()
            )
        ]
    );
    base.0.borrow_mut().offset_x = 19.0;
    assert_eq!(
        item.normal_texture_slot().unwrap().0.borrow().offset_x,
        19.0
    );
}
#[test]
fn food_nullable_variants_preserve_native_append_and_suffix_lookup_order() {
    let _restore = Restore::new();
    let (sm, module) = manager();
    sm.enable_texture_capture();
    let food = sm
        .registries
        .get("item_type")
        .unwrap()
        .borrow_mut()
        .register_id("Edible", true)
        .unwrap();
    sm.statics
        .items
        .borrow_mut()
        .constants
        .insert("item_type", "FOOD", "Edible");
    let base = texture("base");
    let cooked = texture("cooked");
    let burnt = texture("burnt");
    let map = HashMap::from([
        ("Item_Apple".to_string(), base.clone()),
        ("Item_AppleCooked".into(), cooked.clone()),
        ("Item_Apple_Burnt".into(), burnt.clone()),
    ]);
    let calls = Rc::new(RefCell::new(Vec::new()));
    let log = calls.clone();
    pz_render::install_texture_ref_loader(Some(Rc::new(move |_, name| {
        log.borrow_mut().push(name.to_string());
        Ok(map.get(name).cloned())
    })));
    let item = item(&sm, &module, "item Actual { ItemType=Edible, Icon=Apple, }");
    assert!(item.is_item_type_key(Some(&food)));
    assert!(item.texture_capture_complete());
    let variants = item.special_texture_slots();
    assert_eq!(variants.len(), 3);
    assert!(variants[0].is_none());
    assert!(Rc::ptr_eq(variants[1].as_ref().unwrap(), &cooked));
    assert!(Rc::ptr_eq(variants[2].as_ref().unwrap(), &burnt));
    assert_eq!(
        *calls.borrow(),
        [
            "Item_Apple",
            "media/inventory/world/WItem_Apple.png",
            "Item_AppleRotten",
            "Item_AppleSpoiled",
            "Item_Apple_Rotten",
            "Item_AppleCooked",
            "Item_AppleOverdone",
            "Item_AppleBurnt",
            "Item_Apple_Burnt"
        ]
    );
    assert_eq!(
        item.special_world_texture_names,
        [
            "media/inventory/world/WItem_AppleSpoiled.png",
            "media/inventory/world/WItem_AppleCooked.png",
            "media/inventory/world/WItem_AppleBurnt.png"
        ]
    );
}
#[test]
fn load_tail_icons_capture_has_no_question_fallback_and_offline_profile_is_distinct() {
    let _restore = Restore::new();
    let (sm, module) = manager();
    sm.enable_texture_capture();
    let calls = Rc::new(RefCell::new(Vec::new()));
    let log = calls.clone();
    pz_render::install_texture_ref_loader(Some(Rc::new(move |kind, name| {
        log.borrow_mut().push((kind, name.to_string()));
        Ok(None)
    })));
    let actual = item(
        &sm,
        &module,
        "item Actual { IconsForTexture=First;Second, }",
    );
    assert!(actual.texture_capture_complete());
    assert!(actual.normal_texture_slot().is_none());
    assert_eq!(
        *calls.borrow(),
        vec![(TextureLookup::Try, "Item_First".into())]
    );
    let (offline, module) = manager();
    let archived = item(&offline, &module, "item Actual { Icon=NoAsset, }");
    assert!(!archived.texture_capture_complete());
    assert!(archived.normal_texture_slot().is_none());
}
#[test]
fn fault_stops_later_param_and_preserves_prior_native_texture_publication() {
    let _restore = Restore::new();
    let (sm, module) = manager();
    sm.enable_texture_capture();
    let base = texture("base");
    let owned = base.clone();
    pz_render::install_texture_ref_loader(Some(Rc::new(move |kind, _| {
        if kind == TextureLookup::Try {
            Ok(Some(owned.clone()))
        } else {
            Err(TextureHostError::Borrowed("actual world asset"))
        }
    })));
    let item = item(&sm, &module, "item Actual { Icon=First, Weight=9, }");
    assert_eq!(
        sm.texture_host_fault(),
        Some(TextureHostError::Borrowed("actual world asset"))
    );
    assert!(Rc::ptr_eq(item.normal_texture_slot().unwrap(), &base));
    assert!(!item.texture_capture_complete());
    assert_eq!(item.actual_weight, 1.0);
    assert_eq!(
        item.world_texture_name.as_deref(),
        Some("media/inventory/world/WItem_First.png")
    );
    assert!(!sm
        .take_log()
        .iter()
        .any(|line| line.message.contains("actual world asset")));
}
#[test]
fn full_native_load_returns_typed_host_fault_without_publishing_or_marking_parse_error() {
    let _restore = Restore::new();
    let dir = std::env::temp_dir().join(format!(
        "pz-texture-load-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(dir.join("media/scripts")).unwrap();
    std::fs::write(
        dir.join("media/scripts/actual.txt"),
        "module Base { item Actual { Icon=Unknown, Weight=9, } item Next { Icon=Next, Weight=8, } }",
    )
    .unwrap();
    let calls = Rc::new(std::cell::Cell::new(0));
    let observed = calls.clone();
    pz_render::install_texture_ref_loader(Some(Rc::new(move |_, _| {
        observed.set(observed.get() + 1);
        Err(TextureHostError::Borrowed("full load image manager"))
    })));
    let sm = ScriptManager::new();
    let result = sm.load_with_textures(&pz_fs::GameFs::new(&dir), &[]);
    assert_eq!(
        result,
        Err(CapturedScriptLoadError::Host(TextureHostError::Borrowed(
            "full load image manager"
        )))
    );
    assert_eq!(
        calls.get(),
        1,
        "first host fault must stop the next item loop"
    );
    let module = sm.get_module("Base").unwrap();
    let bucket = module.bucket(ScriptType::Item);
    assert!(bucket.script_map_get("Actual").is_none());
    assert!(!bucket.has_load_errors());
    let definitions = bucket.loaded_definitions();
    let actual = definitions[0].downcast::<Item>().unwrap();
    assert_eq!(actual.borrow().actual_weight, 1.0);
    assert!(!sm
        .take_log()
        .iter()
        .any(|line| line.message.contains("full load image manager")));
    std::fs::remove_dir_all(dir).unwrap();
}
#[test]
fn replacing_script_texture_defers_old_physical_pin_until_safe_drain() {
    let _restore = Restore::new();
    let (sm, module) = manager();
    sm.enable_texture_capture();
    let old = texture("old");
    let weak = Rc::downgrade(&old);
    let incoming = Rc::new(RefCell::new(Some(old)));
    let slot = incoming.clone();
    pz_render::install_texture_ref_loader(Some(Rc::new(move |kind, name| {
        if kind == TextureLookup::Try && name == "Item_Old" {
            Ok(slot.borrow_mut().take())
        } else {
            Ok(None)
        }
    })));
    let mut actual = item(&sm, &module, "item Actual { Icon=Old, }");
    assert!(weak.upgrade().is_some());
    actual.do_param(&sm, "Icon", "New").unwrap();
    assert!(actual.normal_texture_slot().is_none());
    assert!(weak.upgrade().is_some());
    let retired = take_retired_textures();
    assert_eq!(retired.len(), 1);
    drop(retired);
    assert!(weak.upgrade().is_none());
}

#[test]
fn nested_load_scope_and_native_reset_preserve_first_fault_until_outer_completion() {
    let sm = ScriptManager::new();
    let outer = sm.begin_texture_load_operation();
    sm.record_texture_host_fault(TextureHostError::Borrowed("outer first"));
    {
        let nested = sm.begin_texture_load_operation();
        assert_eq!(
            nested.host_fault(),
            Some(TextureHostError::Borrowed("outer first"))
        );
        sm.record_texture_host_fault(TextureHostError::Unavailable("nested later"));
        sm.reset();
        assert_eq!(
            nested.host_fault(),
            Some(TextureHostError::Borrowed("outer first"))
        );
    }
    assert_eq!(
        outer.host_fault(),
        Some(TextureHostError::Borrowed("outer first"))
    );
    drop(outer);
    let fresh = sm.begin_texture_load_operation();
    assert!(fresh.host_fault().is_none());
    drop(fresh);
    assert_eq!(sm.texture_load_operation_depth.get(), 0);
}
