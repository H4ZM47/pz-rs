use pz_lua_api::bind_world::with_world;
use pz_world::objects::ObjectKind;
use std::path::Path;

#[derive(Debug, PartialEq, Eq)]
pub(super) struct MannequinState {
    script: Option<String>,
    pose: Option<String>,
    skin: Option<String>,
    dir: i8,
    female: bool,
    // Dictionary/type/identity and condition must survive a fresh engine;
    // retaining only the same count would miss an outfit reroll.
    items: Vec<(String, i16, i32, i32)>,
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
                                    Some((
                                        item.get_full_type(),
                                        item.registry_id,
                                        item.id,
                                        item.condition,
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
pub(super) fn check_live() -> Vec<(i32, i32)> {
    with_world(|world| {
        let cell = world.cell.as_ref().expect("live mannequin regression cell");
        let chunk = cell
            .chunks
            .get(&(1338, 1175))
            .expect("regression spawn must load the previously failing chunk");
        let mut found = 0;
        let mut clothing_items = 0;
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
                        clothing_items += 1;
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
        assert!(clothing_items > 0, "installed fixture must exercise real generated clothing; empty mannequin payloads do not establish this regression");
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
