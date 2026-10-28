//! `IsoGameCharacter.createKeyRing`: a normal bag in the character's
//! inventory, named for its descriptor, optionally holding its house key.

use super::{chr_of, class_cast, shared_script_manager};
use crate::bind_items::{item_value, set_key_id};
use crate::bind_registry::ItemKeyObject;
use pz_characters::character::zombie::math::fastfloor;
use pz_kahlua::{LuaError, LuaResult, NativeCall, Value};
use pz_text::jfmt::JavaArg;

fn npe() -> LuaError {
    LuaError::Runtime {
        message: String::new(),
        class: "java.lang.NullPointerException",
    }
}

pub(super) fn create_key_ring(c: &mut NativeCall<'_>) -> LuaResult<usize> {
    let chr = chr_of(c)?;
    let item_type = if c.nargs() == 1 {
        let scripts = shared_script_manager();
        let tables = scripts.builtin_tables.borrow();
        let key = tables
            .item_keys
            .iter()
            .find(|key| key.holder == "Container" && key.field == "KEY_RING")
            .ok_or_else(|| LuaError::runtime("ItemKey.Container.KEY_RING is unavailable"))?;
        format!("Base.{}", key.id)
    } else {
        match c.arg(1) {
            Value::Nil => return Err(npe()),
            Value::Object(o) => {
                let key = o.as_any().downcast_ref::<ItemKeyObject>().ok_or_else(|| {
                    class_cast(
                        &Value::Object(o.clone()),
                        crate::bind_registry::ITEM_KEY_CLASS,
                    )
                })?;
                format!("Base.{}", key.id)
            }
            other => return Err(class_cast(&other, crate::bind_registry::ITEM_KEY_CLASS)),
        }
    };
    let inventory = chr.borrow().items.inventory.clone();
    let ring = crate::bind_item_container::add_item_type(c.thread, &inventory, &item_type)?;
    let ring = ring.ok_or_else(npe)?;
    let bag = pz_inventory::shared::inventory_of(&ring.borrow()).ok_or_else(|| {
        class_cast(
            &item_value(&ring),
            "zombie.inventory.types.InventoryContainer",
        )
    })?;
    let descriptor = chr.borrow().descriptor.clone().ok_or_else(npe)?;
    let names = {
        let descriptor = descriptor.borrow();
        [&descriptor.forename, &descriptor.surname].map(|name| {
            name.as_ref()
                .map_or(JavaArg::Null, |name| JavaArg::Str(name.clone()))
        })
    };
    let name = crate::bind_text::with_translator(|t| t.get_text("IGUI_KeyRingName", &names))
        .unwrap_or_else(|| "IGUI_KeyRingName".into());
    ring.borrow_mut().set_name(&name);

    // Consume this draw after creation/naming, even outside a building.
    if pz_core::random::next(100) < 40 {
        let (x, y, z) = {
            let chr = chr.borrow();
            (
                fastfloor(chr.vars.x),
                fastfloor(chr.vars.y),
                fastfloor(chr.vars.z),
            )
        };
        let key_id = crate::bind_world::with_world(|world| {
            let meta = world.meta_grid.as_ref().ok_or_else(npe)?;
            Ok::<_, LuaError>(
                meta.get_room_at(x, y, z)
                    .and_then(|room| room.building)
                    .map(|building| meta.building_defs[building.0 as usize].key_id),
            )
        })?;
        if let Some(key_id) = key_id {
            let key = crate::bind_item_container::add_item_type(c.thread, &bag, "Base.Key1")?
                .ok_or_else(npe)?;
            set_key_id(&key, key_id);
        }
    }
    Ok(c.push(item_value(&ring)))
}

#[cfg(test)]
mod tests {
    use super::super::tests::{run, setup_with};
    use super::*;
    use crate::bind_character_sim::{character_value, LuaGameCharacter, LuaGameCharacterRef};
    use crate::bind_items::{GameItemEnv, ItemRef, LuaInventoryItem};
    use pz_inventory::{ItemEnv, ItemTemplate};
    use pz_kahlua::Thread;
    use pz_scripts::{ScriptLoadMode, ScriptManager, ScriptType};
    use std::cell::RefCell;
    use std::rc::Rc;

    fn setup() -> (Thread, LuaGameCharacterRef) {
        let sm = ScriptManager::new();
        sm.enable_texture_capture();
        for (field, id) in [
            ("CONTAINER", "base:container"),
            ("KEY", "base:key"),
            ("NORMAL", "base:normal"),
        ] {
            sm.registries
                .get("item_type")
                .unwrap()
                .borrow_mut()
                .register_id(id, true)
                .unwrap();
            sm.statics
                .items
                .borrow_mut()
                .constants
                .insert("item_type", field, id);
        }
        sm.parse_script(ScriptLoadMode::Init, r#"
            module Base {
                item TestRing { DisplayName = Invented Ring, ItemType = base:container, Weight = 0.07, Capacity = 2, WeightReduction = 73, }
                item Key1 { DisplayName = Invented House Key, ItemType = base:key, Weight = 0.02, }
                item WrongRing { DisplayName = Not A Bag, ItemType = base:normal, Weight = 1, }
            }
        "#).unwrap();
        sm.get_module("Base")
            .unwrap()
            .bucket(ScriptType::Item)
            .load_scripts(&sm, ScriptLoadMode::Init);
        for (id, name) in [
            (1, "Base.TestRing"),
            (2, "Base.Key1"),
            (3, "Base.WrongRing"),
        ] {
            sm.find_item(name, false)
                .unwrap()
                .downcast::<pz_scripts::objects::items::Item>()
                .unwrap()
                .borrow_mut()
                .entity
                .set_registry_id(id)
                .unwrap();
        }
        let (mut thread, chr, _) = setup_with(sm);
        let scripts = shared_script_manager();
        scripts.builtin_tables.borrow_mut().item_keys =
            vec![pz_scripts::registry::builtin::ItemKeyDecl {
                holder: "Container".into(),
                field: "KEY_RING".into(),
                id: "TestRing".into(),
                item_type: "CONTAINER".into(),
            }];
        crate::bind_registry::register(&mut thread, scripts);
        crate::bind_items::register(&mut thread);
        crate::bind_item_container::register(&mut thread);
        // Keep factory RNG independent to make the later global chance
        // draw observable without bypassing the actual item factory.
        let mut env = GameItemEnv::new(None);
        env.rand = Some(pz_core::random::Rand::new(7));
        crate::bind_items::install_item_env(Rc::new(env));
        let desc = chr.borrow().descriptor.clone().unwrap();
        desc.borrow_mut().forename = Some("Ann".into());
        desc.borrow_mut().surname = Some("Example".into());
        let mut translator = pz_text::Translator::new(pz_text::TranslatorEnv::default());
        translator
            .table_mut(pz_text::translator::Table::IgUi)
            .insert(
                "IGUI_KeyRingName".into(),
                pz_text::translator::format_fixer("Keys for %1 %2"),
            );
        crate::bind_text::install(crate::bind_text::TextState {
            translator: Rc::new(RefCell::new(translator)),
            text_manager: Rc::default(),
            draw: None,
            known_file: Rc::new(|_| false),
        });
        crate::bind_world::set_meta_grid(None);
        (thread, chr)
    }

    fn item(thread: &Thread, name: &str) -> ItemRef {
        let Value::Object(o) = thread.globals().get_str(name) else {
            panic!("missing {name}")
        };
        o.as_any()
            .downcast_ref::<LuaInventoryItem>()
            .unwrap()
            .item
            .clone()
    }

    fn seed_chance(hit: bool) -> pz_core::random::Rand {
        let seed = (0..100)
            .find(|seed| (pz_core::random::Rand::new(*seed).next(100) < 40) == hit)
            .unwrap();
        let expected = pz_core::random::Rand::new(seed);
        assert_eq!(expected.next(100) < 40, hit);
        pz_core::random::set_seed(seed);
        expected
    }

    fn install_house(chr: &LuaGameCharacterRef, key_id: i32) {
        struct Zero;
        impl pz_world::GameRand for Zero {
            fn next(&mut self, _: i32) -> i32 {
                0
            }
        }
        let mut meta = pz_world::IsoMetaGrid::blank(0, 0, 0, 0);
        meta.create_cell(0, 0).unwrap();
        let mut building = pz_world::defs::BuildingDef::new(&mut Zero);
        building.key_id = key_id;
        let mut room = pz_world::defs::RoomDef::new(0, "authored room".into());
        room.rects.push(pz_world::defs::RoomRect::new(2, 3, 2, 2));
        room.CalculateBounds();
        meta.add_building(0, 0, building, vec![room], vec![])
            .unwrap();
        crate::bind_world::set_meta_grid(Some(meta));
        let mut chr = chr.borrow_mut();
        chr.vars.x = 2.75;
        chr.vars.y = 3.25;
        chr.vars.z = 0.75;
    }

    #[test]
    fn create_key_ring_default_and_selected_returns_real_bag_without_house_key_on_chance_miss() {
        let (mut thread, chr) = setup();
        let expected = seed_chance(false);
        run(
            &mut thread,
            r#"
            ring = player:createKeyRing(ItemKey.Container.KEY_RING)
            local inventory = player:getInventory()
            assert(ring == inventory:getItems():get(0))
            assert(ring:getContainer() == inventory and inventory:getParent() == player)
            assert(ring:getInventory():getContainingItem() == ring)
            assert(ring:getInventory():getItems():size() == 0)
            assert(ring:getName() == "Keys for Ann Example")
            assert(ring:getCapacity() == 2 and ring:getWeightReduction() == 73)
        "#,
        )
        .unwrap();
        assert_eq!(
            pz_core::random::next(1000),
            expected.next(1000),
            "chance draw order changed"
        );
        let ring = item(&thread, "ring");
        assert!(Rc::ptr_eq(&ring, &chr.borrow().items.inventory_items()[0]));
        assert!(
            ring.borrow().id >= 1233423,
            "item bypassed ordinary factory ID generation"
        );
        seed_chance(false);
        run(&mut thread, "otherRing = player:createKeyRing() assert(otherRing ~= ring and player:getInventory():getItems():size() == 2)").unwrap();
    }

    #[test]
    fn create_key_ring_house_key_uses_real_meta_building_and_survives_inventory_save_load() {
        let (mut thread, chr) = setup();
        install_house(&chr, 424242);
        let expected = seed_chance(true);
        run(
            &mut thread,
            r#"
            ring = player:createKeyRing()
            key = ring:getInventory():getItems():get(0)
            assert(key:getFullType() == "Base.Key1" and key:getKeyId() == 424242)
            assert(key:getContainer() == ring:getInventory())
            assert(key:getContainer():getContainingItem() == ring)
            assert(ring == player:getInventory():getItems():get(0))
        "#,
        )
        .unwrap();
        assert_eq!(pz_core::random::next(1000), expected.next(1000));
        let ring = item(&thread, "ring");
        let key = item(&thread, "key");
        assert_eq!(
            (ring.borrow().registry_id, key.borrow().registry_id),
            (1, 2)
        );
        let ids = (ring.borrow().id, key.borrow().id);
        assert_ne!(ids.0, ids.1);
        struct Env(Vec<(i16, Rc<ItemTemplate>)>);
        impl ItemEnv for Env {
            fn template(&self, id: i16) -> Option<Rc<ItemTemplate>> {
                self.0.iter().find(|e| e.0 == id).map(|e| e.1.clone())
            }
            fn exists(&self, id: i16) -> bool {
                self.0.iter().any(|e| e.0 == id)
            }
        }
        let env = Env(vec![
            (1, ring.borrow().template.clone()),
            (2, key.borrow().template.clone()),
        ]);
        let inventory = chr.borrow().items.inventory.clone();
        let mut output = pz_inventory::io::Writer::new();
        {
            struct RestoreCapture(Option<pz_render::TextureRefLoader>);
            impl Drop for RestoreCapture {
                fn drop(&mut self) {
                    let displaced = pz_render::install_texture_ref_loader(self.0.take());
                    drop(displaced);
                }
            }
            // The genuine no-Icon script capture owns null base textures.
            // Native save skips Question/provider lookup for that null branch;
            // an accidental lookup must fail, rather than receive a fake null.
            let _capture = RestoreCapture(pz_render::install_texture_ref_loader(None));
            assert!(pz_inventory::item_texture::get_texture_ref(&ring)
                .unwrap()
                .is_none());
            assert!(pz_inventory::item_texture::get_texture_ref(&key)
                .unwrap()
                .is_none());
            // Save the actual container with no character/container/item body
            // guard spanning its nested item or native comparison phases.
            pz_inventory::shared_save::save_container_ref(&inventory, &mut output, &env).unwrap();
        }
        assert_eq!((ring.borrow().id, key.borrow().id), ids);
        let mut decoded = pz_inventory::container::ItemContainer::new(Some("none"));
        decoded
            .load(&mut pz_inventory::io::Reader::new(&output.buf), 249, &env)
            .unwrap();
        let loaded = Rc::new(RefCell::new(LuaGameCharacter::new(
            &shared_script_manager(),
            pz_characters::CharacterKind::Player,
        )));
        pz_inventory::shared::load_into(&loaded.borrow().items.inventory, decoded);
        thread.globals().set_str("loaded", character_value(&loaded));
        run(
            &mut thread,
            r#"
            loadedRing = loaded:getInventory():getItems():get(0)
            loadedKey = loadedRing:getInventory():getItems():get(0)
            assert(loadedRing ~= ring and loadedKey ~= key)
            assert(loadedKey:getKeyId() == 424242)
            assert(loadedKey:getContainer() == loadedRing:getInventory())
            assert(loadedRing:getContainer() == loaded:getInventory())
            assert(loadedRing:getInventory():getContainingItem() == loadedRing)
            assert(loaded:getInventory():getParent() == loaded)
        "#,
        )
        .unwrap();
        assert_eq!(
            (
                item(&thread, "loadedRing").borrow().id,
                item(&thread, "loadedKey").borrow().id
            ),
            ids
        );
        crate::bind_world::set_meta_grid(None);
    }

    #[test]
    fn create_key_ring_chance_hit_outside_room_does_not_fabricate_a_key() {
        let (mut thread, _) = setup();
        crate::bind_world::set_meta_grid(Some(pz_world::IsoMetaGrid::blank(0, 0, 0, 0)));
        seed_chance(true);
        run(
            &mut thread,
            "ring = player:createKeyRing() assert(ring:getInventory():getItems():size() == 0)",
        )
        .unwrap();
        crate::bind_world::set_meta_grid(None);
    }

    #[test]
    fn create_key_ring_failures_keep_original_partial_inventory_mutations() {
        let (mut thread, chr) = setup();
        thread.globals().set_str(
            "wrong",
            Value::Object(Rc::new(ItemKeyObject {
                id: "WrongRing".into(),
                item_type: None,
            })),
        );
        thread.globals().set_str(
            "missing",
            Value::Object(Rc::new(ItemKeyObject {
                id: "MissingRing".into(),
                item_type: None,
            })),
        );
        assert!(run(&mut thread, "player:createKeyRing(wrong)")
            .unwrap_err()
            .contains("ClassCastException"));
        assert_eq!(
            chr.borrow().items.inventory_items().len(),
            1,
            "wrong-kind item was rolled back"
        );
        assert!(run(&mut thread, "player:createKeyRing(missing)").is_err());
        assert_eq!(chr.borrow().items.inventory_items().len(), 1);
        assert!(run(&mut thread, "player:createKeyRing(nil)").is_err());
        chr.borrow_mut().descriptor = None;
        assert!(run(&mut thread, "player:createKeyRing()").is_err());
        assert_eq!(
            chr.borrow().items.inventory_items().len(),
            2,
            "descriptor failure rolled back ring"
        );
    }

    #[test]
    fn create_key_ring_key_minus_one_reuses_key_override_rng() {
        let (mut thread, chr) = setup();
        install_house(&chr, -1);
        seed_chance(true);
        run(
            &mut thread,
            "ring = player:createKeyRing() key = ring:getInventory():getItems():get(0)",
        )
        .unwrap();
        let key = item(&thread, "key");
        assert!((0..10_000_000).contains(&key.borrow().get_key_id()));
        assert_eq!(
            key.borrow().key_id,
            -1,
            "Key override wrote the base key field"
        );
        run(
            &mut thread,
            "key:setKeyId(123) assert(key:getKeyId() == 123)",
        )
        .unwrap();
        crate::bind_world::set_meta_grid(None);
    }
}
