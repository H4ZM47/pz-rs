use super::*;
use crate::item_texture::{CapturedScriptTextures, QUESTION_TEXTURE};
use crate::{ItemClass, ItemTemplate};
use pz_render::{LuaTexture, Texture, TextureRef, TextureRefLoader};
use std::cell::Cell;

struct Restore(Option<TextureRefLoader>, Option<NativeSaveBufferRef>);
impl Restore {
    fn new() -> Self {
        Self(
            pz_render::install_texture_ref_loader(None),
            COMPARE.with(|slot| slot.borrow_mut().take()),
        )
    }
}
impl Drop for Restore {
    fn drop(&mut self) {
        drop(pz_render::install_texture_ref_loader(self.0.take()));
        COMPARE.with(|slot| {
            *slot.borrow_mut() = self.1.take();
        });
        crate::component_host::drain_retired_components();
    }
}
struct Env(Vec<Rc<ItemTemplate>>);
impl ItemEnv for Env {
    fn template(&self, id: i16) -> Option<Rc<ItemTemplate>> {
        self.0.get((id - 1) as usize).cloned()
    }
    fn exists(&self, id: i16) -> bool {
        id > 0 && (id as usize) <= self.0.len()
    }
}
fn template(class: ItemClass, name: &str) -> ItemTemplate {
    let mut t = ItemTemplate::new(class, "Base", name);
    t.texture_capture = Some(CapturedScriptTextures::new(None, Vec::new()));
    t
}
fn actual(t: Rc<ItemTemplate>, id: i16, item_id: i32) -> ItemRef {
    let item = crate::env::create_from_template_ref(&Env(vec![]), t, id).unwrap();
    item.borrow_mut().id = item_id;
    item
}
fn asset_template(class: ItemClass, name: &str) -> Rc<ItemTemplate> {
    use crate::template::ScriptItemRef;
    use pz_scripts::ScriptObject;
    let texture: TextureRef = Rc::new(LuaTexture::new(Texture::empty(&format!("Item_{name}"))));
    let captured = texture.clone();
    let target = format!("Item_{name}");
    let old = pz_render::install_texture_ref_loader(Some(Rc::new(move |_, name| {
        Ok((name == target).then(|| captured.clone()))
    })));
    let sm = pz_scripts::ScriptManager::new();
    sm.parse_script(pz_scripts::ScriptLoadMode::Init, "module Base {} ")
        .unwrap();
    sm.enable_texture_capture();
    let module = sm.get_module("Base").unwrap();
    let mut script = pz_scripts::objects::items::Item::new();
    script.set_module(&module);
    script.init_load_pp(&sm, name);
    script
        .load(&sm, name, &format!("item {name} {{ Icon = {name}, }}"))
        .unwrap();
    assert!(script.texture_capture_complete());
    let mut t = template(class, name);
    t.script = Some(ScriptItemRef(Rc::new(RefCell::new(script))));
    drop(pz_render::install_texture_ref_loader(old));
    Rc::new(t)
}
fn inventory(item: &ItemRef) -> ContainerRef {
    let body = item.borrow();
    let ItemKind::Container(bag) = &body.kind else {
        panic!("bag fixture")
    };
    bag.container.clone()
}
fn decoded(w: &Writer, env: &dyn ItemEnv) -> crate::ItemContainer {
    let mut c = crate::ItemContainer::new(None);
    c.load(&mut crate::io::Reader::new(&w.buf), 249, env)
        .unwrap();
    c
}

#[test]
fn actual_nested_null_texture_bag_saves_without_provider_and_returns_same_owners() {
    let _restore = Restore::new();
    let bag_t = Rc::new(template(ItemClass::Container, "Bag"));
    let key_t = Rc::new(template(ItemClass::Key, "Key"));
    let env = Env(vec![bag_t.clone(), key_t.clone()]);
    let bag = actual(bag_t, 1, 101);
    let key = actual(key_t, 2, 202);
    inventory(&bag).borrow_mut().items.push(key.clone());
    let root = crate::shared::SharedContainer::default().into_ref();
    root.borrow_mut().items.push(bag.clone());
    let mut w = Writer::new();
    let saved = save_container_ref(&root, &mut w, &env).unwrap();
    assert!(Rc::ptr_eq(&saved[0], &bag));
    assert_eq!(bag.borrow().id, 101);
    assert_eq!(key.borrow().id, 202);
    let back = decoded(&w, &env);
    assert_eq!(back.items[0].id(), 101);
    let ItemKind::Container(bag_data) = &back.items[0].as_item().unwrap().kind else {
        panic!("decoded bag")
    };
    assert_eq!(bag_data.container.borrow().items[0].borrow().id, 202);
}

#[test]
fn first_comparison_asset_failure_keeps_output_prefix_and_restores_id_guard_free() {
    let _restore = Restore::new();
    let t = asset_template(ItemClass::Normal, "Asset");
    let item = actual(t.clone(), 1, 42);
    let root = crate::shared::SharedContainer::default().into_ref();
    root.borrow_mut().kind = "root".into();
    root.borrow_mut().explored = true;
    root.borrow_mut().items.push(item.clone());
    let receiver = item.clone();
    let parent = root.clone();
    pz_render::install_texture_ref_loader(Some(Rc::new(move |_, name| {
        assert_eq!(name, QUESTION_TEXTURE);
        let body = receiver
            .try_borrow_mut()
            .expect("item guard survived provider");
        assert_eq!(body.id, 0);
        let _parent = parent
            .try_borrow_mut()
            .expect("container guard survived provider");
        Err(pz_render::TextureHostError::Unavailable(
            "first compare asset",
        ))
    })));
    let mut w = Writer::new();
    w.i8(9);
    assert!(matches!(
        save_container_ref(&root, &mut w, &Env(vec![t])),
        Err(ItemPersistenceError::Host(RuntimeHostError::Unsupported {
            operation: "first compare asset",
            ..
        }))
    ));
    let mut prefix = Writer::new();
    prefix.i8(9);
    prefix.string("root");
    prefix.bool(true);
    prefix.i16(0);
    assert_eq!(
        w.buf, prefix.buf,
        "no run count/record before first comparison completes"
    );
    assert_eq!(item.borrow().id, 42);

    let calls = Rc::new(Cell::new(0));
    let count = calls.clone();
    let receiver = item.clone();
    pz_render::install_texture_ref_loader(Some(Rc::new(move |_, _| {
        let n = count.get() + 1;
        count.set(n);
        assert_eq!(receiver.borrow().id, if n == 1 { 0 } else { 42 });
        if n == 1 {
            Ok(None)
        } else {
            Err(pz_render::TextureHostError::Unavailable(
                "direct record asset",
            ))
        }
    })));
    let mut direct = Writer::new();
    direct.i8(9);
    let direct_template = item.borrow().template.clone();
    assert!(matches!(
        save_container_ref(&root, &mut direct, &Env(vec![direct_template])),
        Err(ItemPersistenceError::Host(RuntimeHostError::Unsupported {
            operation: "direct record asset",
            ..
        }))
    ));
    assert_eq!(calls.get(), 2);
    assert!(direct.buf.starts_with(&prefix.buf));
    let at = prefix.buf.len();
    assert_eq!(
        &direct.buf[at..at + 8],
        &[0, 0, 0, 1, 0, 0, 0, 0],
        "run count then success-only record size placeholder"
    );
    assert!(
        direct.buf.len() > at + 15,
        "actual failed record prefix is not staged away"
    );
    assert_eq!(
        i32::from_be_bytes(direct.buf[at + 11..at + 15].try_into().unwrap()),
        42
    );
    assert_eq!(item.borrow().id, 42);
}

#[test]
fn captured_groups_survive_list_replacement_and_duplicate_ids_and_tail_are_read_late() {
    let _restore = Restore::new();
    let a = asset_template(ItemClass::Normal, "A");
    let b = asset_template(ItemClass::Normal, "B");
    let env = Env(vec![a.clone(), b.clone()]);
    let first = actual(a.clone(), 1, 100);
    let duplicate = actual(a, 1, 200);
    let other = actual(b, 2, 300);
    let root = crate::shared::SharedContainer::default().into_ref();
    root.borrow_mut().items = vec![first.clone(), other.clone(), duplicate.clone()];
    let parent = root.clone();
    let f = first.clone();
    let d = duplicate.clone();
    let calls = Rc::new(Cell::new(0));
    let seen = calls.clone();
    pz_render::install_texture_ref_loader(Some(Rc::new(move |_, name| {
        assert_eq!(name, QUESTION_TEXTURE);
        let n = seen.get() + 1;
        seen.set(n);
        if n == 1 {
            assert_eq!(f.borrow().id, 0);
            parent.borrow_mut().items.clear();
            parent.borrow_mut().has_been_looted = true;
            parent.borrow_mut().capacity = 91;
            d.borrow_mut().id = 250;
        }
        if n == 2 {
            assert_eq!(d.borrow().id, 0);
        }
        if n == 3 {
            assert_eq!(f.borrow().id, 100);
            d.borrow_mut().id = 999;
        }
        Ok(None)
    })));
    let mut w = Writer::new();
    let saved = save_container_ref(&root, &mut w, &env).unwrap();
    assert_eq!(
        calls.get(),
        5,
        "lazy first/candidate/direct saves, then B first/direct"
    );
    assert!(root.borrow().items.is_empty());
    assert!(
        Rc::ptr_eq(&saved[0], &first)
            && Rc::ptr_eq(&saved[1], &duplicate)
            && Rc::ptr_eq(&saved[2], &other)
    );
    let back = decoded(&w, &env);
    assert_eq!(
        back.items.iter().map(|item| item.id()).collect::<Vec<_>>(),
        [100, 999, 300]
    );
    assert!(back.has_been_looted);
    assert_eq!(back.capacity, 91);
}

#[test]
fn comparison_overflow_retries_before_later_provider_and_reentrant_save_shares_buffer() {
    let _restore = Restore::new();
    let t = asset_template(ItemClass::Normal, "Large");
    let item = actual(t.clone(), 1, 64);
    item.borrow_mut().name = "x".repeat(4000);
    let root = crate::shared::SharedContainer::default().into_ref();
    root.borrow_mut().items.push(item.clone());
    let small_t = Rc::new(template(ItemClass::Normal, "Small"));
    let small = actual(small_t.clone(), 1, 7);
    let nested = crate::shared::SharedContainer::default().into_ref();
    nested.borrow_mut().items.push(small);
    let env = Env(vec![t]);
    let nested_env = Env(vec![small_t]);
    let calls = Rc::new(Cell::new(0));
    let seen = calls.clone();
    let receiver = item.clone();
    pz_render::install_texture_ref_loader(Some(Rc::new(move |_, _| {
        let n = seen.get() + 1;
        seen.set(n);
        if n == 1 {
            assert_eq!(receiver.borrow().id, 0);
            let before = current_buffer();
            assert!(io::native_buffer_capacity(&before).unwrap() > 1024);
            let old_position = io::native_buffer_position(&before).unwrap();
            let mut inner = Writer::new();
            save_container_ref(&nested, &mut inner, &nested_env).unwrap();
            assert_eq!(decoded(&inner, &nested_env).items[0].id(), 7);
            assert!(Rc::ptr_eq(&before, &current_buffer()));
            assert_ne!(io::native_buffer_position(&before).unwrap(), old_position);
        }
        Ok(None)
    })));
    let mut w = Writer::new();
    save_container_ref(&root, &mut w, &env).unwrap();
    assert_eq!(
        calls.get(),
        2,
        "overflow cannot call provider before growth/replay"
    );
    assert_eq!(item.borrow().id, 64);
    assert_eq!(decoded(&w, &env).items[0].id(), 64);
}

#[test]
fn typed_table_borrow_and_selected_raw_records_keep_native_stream_prefixes() {
    let _restore = Restore::new();
    let t = Rc::new(template(ItemClass::Normal, "Table"));
    let item = actual(t.clone(), 1, 55);
    let table = crate::moddata::get_mod_data_ref(&item).unwrap();
    let root = crate::shared::SharedContainer::default().into_ref();
    root.borrow_mut().items.push(item.clone());
    let held = table.try_borrow_body_mut().unwrap();
    let mut w = Writer::new();
    assert!(matches!(
        save_container_ref(&root, &mut w, &Env(vec![t])),
        Err(ItemPersistenceError::Host(RuntimeHostError::Borrowed(_)))
    ));
    assert_eq!(item.borrow().id, 55);
    let mut prefix = Writer::new();
    prefix.string("none");
    prefix.bool(false);
    prefix.i16(0);
    assert_eq!(w.buf, prefix.buf);
    drop(held);
    root.borrow_mut()
        .raw_items
        .push(crate::SavedItem::Raw(vec![0; 7]));
    let mut w = Writer::new();
    assert!(matches!(
        save_container_ref(&root, &mut w, &Env(vec![])),
        Err(ItemPersistenceError::Host(RuntimeHostError::Unsupported {
            operation: "actual container save selected raw archive records",
            ..
        }))
    ));
    let mut prefix = Writer::new();
    prefix.string("none");
    prefix.bool(false);
    assert_eq!(
        w.buf, prefix.buf,
        "raw capability fails before compression count placeholder"
    );
}

#[test]
fn unread_input_owners_are_retired_before_first_fallible_grouping_read() {
    let _restore = Restore::new();
    let t = Rc::new(template(ItemClass::Normal, "Owner"));
    let first = actual(t.clone(), 1, 1);
    let second = actual(t, 1, 2);
    crate::component_host::drain_retired_components();
    let weak = Rc::downgrade(&second);
    let input = vec![first.clone(), second];
    let held = first.borrow_mut();
    let mut w = Writer::new();
    w.i8(7);
    assert!(matches!(
        save_items_ref(&input, &mut w, &Env(vec![]), None),
        Err(ItemPersistenceError::Host(RuntimeHostError::Borrowed(_)))
    ));
    assert_eq!(w.buf, [7]);
    drop(held);
    drop(input); // The caller may still hold its outer WORLD guard here.
    assert!(
        weak.upgrade().is_some(),
        "unread second owner must remain queued"
    );
    crate::component_host::drain_retired_components();
    assert!(weak.upgrade().is_none());
}

#[test]
fn bag_tail_rereads_current_inventory_after_parent_asset_callback() {
    let _restore = Restore::new();
    let bag_t = asset_template(ItemClass::Container, "BagAsset");
    let key_t = Rc::new(template(ItemClass::Key, "Key"));
    let env = Env(vec![bag_t.clone(), key_t.clone()]);
    let bag = actual(bag_t, 1, 11);
    let key = actual(key_t, 2, 22);
    let old = inventory(&bag);
    let replacement = crate::shared::SharedContainer::default().into_ref();
    replacement.borrow_mut().id = 42;
    replacement.borrow_mut().items.push(key);
    let new = replacement.clone();
    let receiver = bag.clone();
    pz_render::install_texture_ref_loader(Some(Rc::new(move |_, _| {
        let mut body = receiver
            .try_borrow_mut()
            .expect("parent guard spans Question");
        let ItemKind::Container(data) = &mut body.kind else {
            panic!("bag")
        };
        data.container = new.clone();
        data.weight_reduction = 57;
        Ok(None)
    })));
    let root = crate::shared::SharedContainer::default().into_ref();
    root.borrow_mut().items.push(bag);
    let mut w = Writer::new();
    save_container_ref(&root, &mut w, &env).unwrap();
    let back = decoded(&w, &env);
    let ItemKind::Container(data) = &back.items[0].as_item().unwrap().kind else {
        panic!("decoded bag")
    };
    assert_eq!(data.weight_reduction, 57);
    assert_eq!(data.container.borrow().id, 42);
    assert_eq!(data.container.borrow().items[0].borrow().id, 22);
    assert!(old.borrow().items.is_empty());
}

#[test]
fn native_numeric_attribute_precheck_has_no_self_fast_path_or_id_substitute() {
    use pz_entity::context::Lookup;
    use pz_entity::tables::{AttributeTypeInfo, InitialValue};
    use pz_entity::{AttributeTypes, NumericAttributeValidation};
    let _restore = Restore::new();
    let mut types = AttributeTypes::default();
    let canonical = types.insert_with_validation(
        AttributeTypeInfo {
            id: 0,
            name: "Sharpness".into(),
            kind: AttributeKind::Float,
            initial: Some(InitialValue::Float(1.0)),
            hidden_ui: true,
        },
        false,
        Lookup::Found(NumericAttributeValidation {
            requires_validation: false,
            bounds: None,
        }),
    );
    let mut other_types = AttributeTypes::default();
    let same_id_different_ref = other_types.insert_with_validation(
        AttributeTypeInfo {
            id: 0,
            name: "Sharpness".into(),
            kind: AttributeKind::Float,
            initial: Some(InitialValue::Float(1.0)),
            hidden_ui: true,
        },
        false,
        Lookup::Found(NumericAttributeValidation {
            requires_validation: false,
            bounds: None,
        }),
    );
    let a = pz_entity::runtime_pool::obtain_component(ComponentType::Attributes).unwrap();
    let b = pz_entity::runtime_pool::obtain_component(ComponentType::Attributes).unwrap();
    let mut retired = RetiredComponents::new();
    a.borrow_mut()
        .attributes_mut()
        .unwrap()
        .set_value(&canonical, AttributeValue::Float(f32::NAN), &mut retired)
        .unwrap();
    assert!(!attributes_identical(&a, &a).unwrap());
    a.borrow_mut()
        .attributes_mut()
        .unwrap()
        .set_value(&canonical, AttributeValue::Float(-0.0), &mut retired)
        .unwrap();
    b.borrow_mut()
        .attributes_mut()
        .unwrap()
        .set_value(&canonical, AttributeValue::Float(0.0), &mut retired)
        .unwrap();
    assert!(attributes_identical(&a, &b).unwrap());
    let c = pz_entity::runtime_pool::obtain_component(ComponentType::Attributes).unwrap();
    c.borrow_mut()
        .attributes_mut()
        .unwrap()
        .set_value(
            &same_id_different_ref,
            AttributeValue::Float(0.0),
            &mut retired,
        )
        .unwrap();
    assert!(!attributes_identical(&a, &c).unwrap());
    crate::component_host::retire_components(retired);
}

#[test]
fn id_finally_restores_even_when_unrelated_provider_panic_is_rethrown() {
    let _restore = Restore::new();
    let t = asset_template(ItemClass::Normal, "Panic");
    let item = actual(t.clone(), 1, 77);
    let root = crate::shared::SharedContainer::default().into_ref();
    root.borrow_mut().items.push(item.clone());
    pz_render::install_texture_ref_loader(Some(Rc::new(|_, _| {
        panic!("unrelated provider sentinel")
    })));
    let mut w = Writer::new();
    let failure = catch_unwind(AssertUnwindSafe(|| {
        save_container_ref(&root, &mut w, &Env(vec![t]))
    }));
    let payload = failure.unwrap_err();
    assert_eq!(
        payload.downcast_ref::<&str>(),
        Some(&"unrelated provider sentinel")
    );
    assert_eq!(item.borrow().id, 77);
}

#[test]
fn selected_unported_subclass_payloads_stop_after_their_native_prefix() {
    let _restore = Restore::new();
    let map = actual(Rc::new(template(ItemClass::Map, "Map")), 1, 18);
    if let ItemKind::Map { map_id, .. } = &mut map.borrow_mut().kind {
        *map_id = "KnownMap".into();
    }
    let mut output = Writer::new();
    output.i8(99);
    assert!(matches!(
        crate::item_texture::save_item_ref(&map, &mut output, &Env(vec![])),
        Err(ItemPersistenceError::Host(RuntimeHostError::Unsupported {
            operation: "actual MapItem symbols save authority",
            ..
        }))
    ));
    assert_eq!(output.buf[0], 99);
    assert!(output.buf.ends_with(b"\0\x08KnownMap"));
    let weapon = actual(Rc::new(template(ItemClass::Weapon, "Weapon")), 1, 19);
    if let ItemKind::Weapon(data) = &mut weapon.borrow_mut().kind {
        data.clip_size = 8;
        let mut part = crate::InventoryItem::create(
            Rc::new(ItemTemplate::new(
                ItemClass::WeaponPart,
                "Base",
                "ArchivePart",
            )),
            2,
        );
        part.id = 0x1122_3344;
        data.attachments.push(part);
        data.fire_mode = Some("LATER".into());
    }
    let mut output = Writer::new();
    output.i8(98);
    assert!(matches!(
        crate::item_texture::save_item_ref(&weapon, &mut output, &Env(vec![])),
        Err(ItemPersistenceError::Host(RuntimeHostError::Unsupported {
            operation: "actual weapon attachment owner/serializer",
            ..
        }))
    ));
    assert_eq!(output.buf[0], 98);
    assert!(
        output.buf.ends_with(&[0, 0, 0, 8, 1]),
        "clip/count precede unsupported attachment serializer"
    );
    assert!(!output.buf.windows(5).any(|bytes| bytes == b"LATER"));
    assert!(!output
        .buf
        .windows(4)
        .any(|bytes| bytes == 0x1122_3344i32.to_be_bytes()));
}
