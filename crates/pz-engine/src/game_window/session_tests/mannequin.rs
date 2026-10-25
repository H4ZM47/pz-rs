use pz_lua_api::bind_world::with_world;
use pz_world::objects::ObjectKind;
use std::path::Path;

/// Authored map-style receivers, using only installed mannequin scripts,
/// outfit assets and the normal item factory. Scarecrow03's CostumeScarecrow
/// has required clothing; Scarecrow02 explicitly requests no outfit. Neither
/// changes the original map mannequin or reseeds the world's outfit RNG.
pub(super) fn install_outfit_fixtures(
    lua: &mut pz_lua_api::LuaManager,
    game_dir: &Path,
    parts: usize,
    sprites: &std::rc::Rc<std::cell::RefCell<pz_world::sprite::IsoSpriteManager>>,
) {
    let (square, sprite) = with_world(|world| {
        let cell = world.cell.as_ref().unwrap();
        let original = cell.get_grid_square(10710, 9402, 0).unwrap();
        let sprite = original.borrow().objects.iter().find_map(|o| {
            let object = o.borrow();
            matches!(object.kind, ObjectKind::Mannequin(_)).then_some(object.sprite)
        }).expect("installed map mannequin");
        // Adjacent to the one-tile authored map zone, in the same saved chunk.
        (cell.get_grid_square(10710, 9403, 0).unwrap(), sprite)
    });
    let sprite_name = {
        let sprites = sprites.borrow();
        sprite.and_then(|index| sprites.sprites.get(index))
            .and_then(|sprite| sprite.name.clone()).expect("installed mannequin sprite name")
    };
    let no_black_clothes = {
        let sandbox = crate::iso_world::sandbox(lua).expect("live world sandbox");
        let value = crate::iso_world::opt_bool(&sandbox.borrow(), "NoBlackClothes");
        value
    };
    for script in ["MannequinScarecrow02", "MannequinScarecrow03"] {
        let mannequin = pz_world::objects::IsoMannequin::new(parts);
        let object = pz_world::ObjectSlot::new(pz_world::IsoObject::new(
            square.borrow_mut().new_object_id(),
            ObjectKind::Mannequin(Box::new(mannequin)),
            sprite,
        ));
        let receiver = object.ref_handle();
        square.borrow_mut().objects.push(object);
        // Match authored map-zone precedence even when the reused installed
        // sprite supplies a different default mannequin script.
        let zone = pz_world::zones::MannequinZoneData {
            female: -1, dir: None, mannequin_script: Some(script.into()),
            pose: None, skin: None, outfit: None,
        };
        // The real map receiver already exercised streaming activation. These
        // extra receivers isolate both clothing outcomes without RNG retries.
        pz_lua_api::world_mannequins::activate(
            &mut lua.thread, &receiver, Some(&sprite_name), Some(&zone),
            game_dir, no_black_clothes,
        ).expect("activate installed outfit fixture");
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(super) struct MannequinState {
    script: Option<String>,
    pose: Option<String>,
    skin: Option<String>,
    dir: i8,
    female: bool,
    // Dictionary/type/identity and condition must survive a fresh engine;
    // retaining only the same count would miss an outfit reroll.
    items: Vec<(String, i16, i32, i32, Vec<u8>)>,
    worn: Vec<(String, i32)>,
}

pub(super) fn saved_state() -> Vec<MannequinState> {
    with_world(|world| {
        let cell = world.cell.as_ref().unwrap();
        let chunk = cell.chunks.get(&(1338, 1175)).unwrap();
        let mut states = Vec::new();
        for square in chunk.squares() {
            for object in &square.borrow().objects {
                let object = object.borrow();
                if let ObjectKind::Mannequin(m) = &object.kind {
                    let (container, worn) = m.save_inventory_snapshot().unwrap().unwrap();
                    states.push(MannequinState {
                        script: m.mannequin_script_name.clone(),
                        pose: m.pose.clone(),
                        skin: m.human_visual.skin_texture_name.clone(),
                        dir: m.dir,
                        female: m.female,
                        items: container
                            .items
                            .iter()
                            .filter_map(|saved| {
                                if let pz_inventory::SavedItem::Item(item) = saved {
                                    let mut visual = pz_inventory::io::Writer::new();
                                    item.visual.as_ref().unwrap().save(&mut visual);
                                    Some((
                                        item.get_full_type(),
                                        item.registry_id,
                                        item.id,
                                        item.condition,
                                        visual.buf,
                                    ))
                                } else {
                                    None
                                }
                            })
                            .collect(),
                        worn,
                    });
                }
            }
        }
        states
    })
}

/// The installed map chunk that previously failed both close/menu saves.
pub(super) fn check_live(new_world: bool) -> Vec<(i32, i32)> {
    with_world(|world| {
        let cell = world.cell.as_ref().expect("live mannequin regression cell");
        let chunk = cell
            .chunks
            .get(&(1338, 1175))
            .expect("regression spawn must load the previously failing chunk");
        let mut found = 0;
        let mut required_outfit = 0;
        let mut empty_outfit = 0;
        let mut original_map = 0;
        for square in chunk.squares() {
            for object in &square.borrow().objects {
                let object = object.borrow();
                if let ObjectKind::Mannequin(m) = &object.kind {
                    found += 1;
                    assert!(m.init, "map mannequin missed addToWorld initialization");
                    let inventory = m
                        .runtime_container
                        .as_ref()
                        .expect("map mannequin has no actual clothing container");
                    assert!(inventory.borrow().explored);
                    let count = inventory.borrow().items.len();
                    let script = m.mannequin_script_name.as_deref();
                    match script {
                        Some("MannequinScarecrow01") => {
                            original_map += 1;
                            assert_eq!(m.outfit.as_deref(), Some("Scarecrow"));
                            if new_world {
                                assert_eq!(m.human_visual.outfit.as_deref(), Some("Scarecrow"),
                                    "installed named outfit was silently skipped");
                            }
                            // All six probabilities are below one. Empty is
                            // legitimate for this authentic map mannequin.
                        }
                        Some("MannequinScarecrow02") => {
                            empty_outfit += 1;
                            assert_eq!(m.outfit.as_deref(), Some("none"));
                            assert_eq!(count, 0, "explicit no-outfit mannequin was dressed");
                        }
                        Some("MannequinScarecrow03") => {
                            required_outfit += 1;
                            assert_eq!(m.outfit.as_deref(), Some("CostumeScarecrow"));
                            assert!(count > 0, "required installed outfit produced no genuine clothing");
                            assert!(inventory.borrow().items.iter().any(|i| i.borrow().get_full_type() == "Base.Socks_Long"),
                                "required installed Socks_Long reference missed assets/scripts/Human-group/factory conversion");
                            if new_world {
                                assert_eq!(m.human_visual.outfit.as_deref(), Some("CostumeScarecrow"));
                            }
                        }
                        _ => panic!("unexpected regression mannequin script: {script:?}"),
                    }
                    eprintln!("mannequin-check new={new_world} script={script:?} outfit={:?} items={count} visuals={}", m.outfit, m.item_visuals.len());
                    assert!(
                        m.human_visual.skin_texture_name.is_some(),
                        "syncModel did not set actual mannequin skin"
                    );
                    assert!(
                        m.pose
                            .as_deref()
                            .is_some_and(|p| p != "Invalid" && !p.is_empty()),
                        "map mannequin pose was not validated from installed animation data"
                    );
                    for item in &inventory.borrow().items {
                        let item = item.borrow();
                        assert!(
                            item.is_initialised,
                            "factory result missed genuine initialiseItem"
                        );
                        assert!(
                            item.registry_id >= 0,
                            "generated clothing lost the world dictionary ID"
                        );
                        assert!(
                            item.visual.is_some(),
                            "generated mannequin clothing has no item visual"
                        );
                    }
                }
            }
        }
        assert!(
            found > 0,
            "installed fixture no longer exercises a mannequin"
        );
        assert_eq!(original_map, 1, "authentic map mannequin is missing");
        assert_eq!(empty_outfit, 1, "explicit no-outfit fixture is missing");
        assert_eq!(required_outfit, 1, "required-clothing fixture is missing");
        cell.chunks.keys().copied().collect()
    })
}

pub(super) fn check_saved(save_dir: &Path, expected: &[(i32, i32)]) {
    for &(wx, wy) in expected {
        let path = pz_world::save::chunk_file::chunk_path(save_dir, wx, wy);
        let bytes = std::fs::read(&path).unwrap_or_else(|error| {
            panic!("world chunk was not saved {}: {error}", path.display())
        });
        assert!(
            pz_world::save::chunk_file::validate_header(&bytes),
            "invalid world chunk {}",
            path.display()
        );
    }
    assert!(pz_world::save::chunk_file::chunk_path(save_dir, 1338, 1175).is_file());
}
