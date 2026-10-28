use super::*;
use crate::template::ScriptItemRef;
use crate::{InventoryItem, ItemClass, ItemEnv, ItemTemplate};
use pz_scripts::{ScriptLoadMode, ScriptManager, ScriptObject};
use std::cell::RefCell;
use std::rc::Rc;
struct Boundary;
impl Drop for Boundary {
    fn drop(&mut self) {
        crate::component_host::drain_retired_components();
    }
}
struct Env;
impl ItemEnv for Env {}
fn actual() -> (ItemRef, ScriptItemRef, Rc<ScriptManager>) {
    let manager = Rc::new(ScriptManager::new());
    manager.enable_texture_capture();
    manager
        .parse_script(ScriptLoadMode::Init, "module Base {}")
        .unwrap();
    let mut script = pz_scripts::objects::items::Item::new();
    script.set_module(&manager.get_module("Base").unwrap());
    script.init_load_pp(&manager, "ReadFlag");
    script
        .load(&manager, "ReadFlag", "item ReadFlag {}")
        .unwrap();
    assert!(script.texture_capture_complete());
    let script = ScriptItemRef(Rc::new(RefCell::new(script)));
    let mut template = ItemTemplate::new(ItemClass::Normal, "Base", "ReadFlag");
    template.script = Some(script.clone());
    let item = crate::env::create_from_template_ref(&Env, Rc::new(template), 1).unwrap();
    (item, script, manager)
}
#[test]
fn negative_actual_index_skips_busy_script_and_category_selects_only_genuine_radio_cap() {
    let _boundary = Boundary;
    let (item, script, _manager) = actual();
    let held = script.0.borrow_mut();
    for index in [-1, i16::MIN] {
        item.borrow_mut().recorded_media_index = index;
        assert!(!has_been_seen_ref(&item).unwrap());
        assert!(!has_been_heard_ref(&item).unwrap());
    }
    item.borrow_mut().recorded_media_index = 0;
    assert!(matches!(
        has_been_seen_ref(&item),
        Err(RuntimeHostError::Borrowed(
            "InventoryItem media read script category"
        ))
    ));
    drop(held);
    assert!(!has_been_seen_ref(&item).unwrap() && !has_been_heard_ref(&item).unwrap());
    for category in ["Retail-VHS", "Home-VHS"] {
        script.0.borrow_mut().recorded_media_cat = Some(category.into());
        assert!(matches!(
            has_been_seen_ref(&item),
            Err(RuntimeHostError::Unsupported {
                operation: "InventoryItem.hasBeenSeen actual ZomboidRadio context",
                ..
            })
        ));
        assert!(!has_been_heard_ref(&item).unwrap());
    }
    script.0.borrow_mut().recorded_media_cat = Some("retail-vhs".into());
    assert!(!has_been_seen_ref(&item).unwrap());
    assert!(matches!(
        has_been_heard_ref(&item),
        Err(RuntimeHostError::Unsupported {
            operation: "InventoryItem.hasBeenHeard actual ZomboidRadio context",
            ..
        })
    ));
}
#[test]
fn needed_item_body_and_null_actual_script_remain_faults_not_false() {
    let _boundary = Boundary;
    let (item, _script, _manager) = actual();
    let held = item.borrow_mut();
    assert!(matches!(
        has_been_seen_ref(&item),
        Err(RuntimeHostError::Borrowed(
            "InventoryItem media read recordedMediaIndex"
        ))
    ));
    drop(held);
    {
        let mut b = item.borrow_mut();
        b.recorded_media_index = 1;
        let crate::item_texture::ItemTextures::Live(live) = &mut b.textures else {
            panic!("actual");
        };
        live.script_item = None;
    }
    assert!(matches!(
        has_been_heard_ref(&item),
        Err(RuntimeHostError::Unsupported {
            operation: "InventoryItem media read null scriptItem receiver",
            ..
        })
    ));
}
#[test]
fn map_stash_is_nullable_actual_prefix_and_archive_map_id_never_substitutes() {
    let _boundary = Boundary;
    // Explicit owned-map fixture tests only shared physical stash fields and
    // the selected nullable native MapID boundary, not full Map construction.
    let item = crate::shared::try_new_item(InventoryItem::create(
        Rc::new(ItemTemplate::new(ItemClass::Map, "Base", "Map")),
        1,
    ))
    .unwrap();
    {
        let mut body = item.borrow_mut();
        let ItemKind::Map { map_id, .. } = &mut body.kind else {
            unreachable!()
        };
        *map_id = "ArchiveShadow".into();
    }
    assert!(matches!(
        map_media_id_ref(&item),
        Err(RuntimeHostError::Unsupported {
            operation: "MapItem.getMapID actual nullable authority",
            ..
        })
    ));
    for stash in ["", "RealCase"] {
        item.borrow_mut().stash_map = Some(stash.into());
        assert_eq!(map_media_id_ref(&item).unwrap().as_deref(), Some(stash));
    }
    let held = item.borrow_mut();
    assert!(matches!(
        map_media_id_ref(&item),
        Err(RuntimeHostError::Borrowed("MapItem media-id stash field"))
    ));
    drop(held);
}
