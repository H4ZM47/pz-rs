use super::*;
use pz_inventory::{ItemAssets, ItemClass, ItemTemplate};
use pz_kahlua::TableRef;
use pz_scripts::{ScriptManager, ScriptObject};

struct Restore {
    env: Rc<dyn crate::bind_items::ItemLuaEnv>,
    provider: Option<pz_render::TextureRefLoader>,
}
impl Drop for Restore {
    fn drop(&mut self) {
        crate::bind_items::install_item_env(self.env.clone());
        let old = pz_render::install_texture_ref_loader(self.provider.take());
        drop(old);
        pz_inventory::component_host::drain_retired_components();
    }
}

#[test]
fn direct_lua_sync_texture_fault_precedes_gas_mask_moddata_allocation() {
    let texture = Rc::new(pz_render::LuaTexture::new(pz_render::Texture::empty(
        "Item_Early",
    )));
    let initial = texture.clone();
    let _restore = Restore {
        env: crate::bind_items::item_env(),
        provider: pz_render::install_texture_ref_loader(Some(Rc::new(move |_, _| {
            Ok(Some(initial.clone()))
        }))),
    };
    let manager = ScriptManager::new();
    manager.enable_texture_capture();
    let script = Rc::new(RefCell::new(pz_scripts::objects::items::Item::new()));
    script
        .borrow_mut()
        .load(
            &manager,
            "SyncMask",
            "item SyncMask { IconsForTexture=Early, }",
        )
        .unwrap();
    let mut template = ItemTemplate::new(ItemClass::Clothing, "Base", "SyncMask");
    template.script = Some(pz_inventory::template::ScriptItemRef(script));
    template.icons_for_texture = Some(vec!["Early".into()]);
    template.static_models_by_index = Some(Rc::new(vec!["SyncMaskModel".into()]));
    template
        .tags
        .push(pz_inventory::gameplay::tags::GAS_MASK.into());
    let item = pz_inventory::env::create_from_template_ref(
        &pz_inventory::env::NoItemEnv,
        Rc::new(template),
        1,
    )
    .map_err(|failure| failure.into_error_and_retire())
    .unwrap();
    assert!(item.borrow().is_gas_mask());
    assert!(item.borrow().table.is_none());
    crate::bind_items::install_item_env(Rc::new(crate::bind_items::GameItemEnv::new(Some(
        ItemAssets::default(),
    ))));
    pz_render::install_texture_ref_loader(Some(Rc::new(|_, _| {
        Err(pz_render::TextureHostError::Borrowed("first sync asset"))
    })));
    let mut thread = Thread::new(TableRef::new());
    pz_kahlua::stdlib::install(&mut thread, None).unwrap();
    crate::bind_items::register(&mut thread);
    register_item_methods(&mut thread);
    thread.globals().set_str("syncItem", item_value(&item));
    let method = pz_kahlua::stdlib::load_string(
        "syncItem:synchWithVisual()",
        Some("early-sync-fault"),
        thread.globals().clone(),
    )
    .unwrap();
    assert!(matches!(
        thread.call(&method, &[]),
        Err(LuaError::Runtime {
            class: "pz.host.Borrowed",
            ..
        })
    ));
    assert!(item.borrow().table.is_none());
    assert_eq!(item.borrow().model_index, 0);
    let actual = pz_inventory::item_texture::get_texture_ref(&item)
        .unwrap()
        .unwrap();
    assert!(Rc::ptr_eq(&actual, &texture));
}
