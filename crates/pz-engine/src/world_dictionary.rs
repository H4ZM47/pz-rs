//! `zombie.world.WorldDictionary.init()` for a local game, and the save's
//! item registry built over it.
//!
//! The original keeps the dictionary in statics (`WorldDictionary.data`)
//! that every item load asks for registry ids. The port reads it into a
//! [`WorldDictionary`] and wraps it in [`ScriptItems`] (the dictionary's
//! ids mapped to the loaded scripts), which the chunk loads and
//! `IsoPlayer.load` take as their item environment.

use std::path::Path;

use pz_inventory::component::{ComponentBlob, ComponentCodec, EntityCodec, FactoryRng};
use pz_inventory::dictionary::{DictionaryInfo, Register, WorldDictionary};
use pz_inventory::factory::ScriptItems;
use pz_inventory::io::{Reader, SaveResult, Writer};
use pz_inventory::ItemTemplate;

/// `WorldDictionaryException`'s message for a failed `init()`.
const CANNOT_LOAD: &str = "WorldDictionary: Cannot load world due to WorldDictionary error.";

/// `DictionaryData.load()`: `WorldDictionary.bin` of the current save
/// (`None`: nothing read, for a `noSave` game or a new game without the
/// file). A missing file in a loaded game, or one that does not read, is
/// a `WorldDictionaryException`.
fn load_data(
    save_dir: &Path,
    no_save: bool,
    is_new_game: bool,
) -> Result<Option<WorldDictionary>, String> {
    if no_save {
        return Ok(None);
    }
    let path = save_dir.join("WorldDictionary.bin");
    if !path.exists() {
        if !is_new_game {
            return Err("WorldDictionary data file is missing from world folder.".into());
        }
        return Ok(None);
    }
    println!("Loading WorldDictionary:{}", path.display());
    let bytes = std::fs::read(&path).map_err(|e| format!("Error loading WorldDictionary.: {e}"))?;
    WorldDictionary::load(&bytes)
        .map(Some)
        .map_err(|e| format!("Error loading WorldDictionary.: {}", e.0))
}

/// The script-loading callback's item/entity information. The script
/// loader retains every (mod id, body) pair, including overridden bodies.
fn script_info(
    entity: &pz_scripts::objects::entity::GameEntityScript,
    obsolete: bool,
) -> Result<DictionaryInfo, String> {
    let full_type = entity.full_name().map_err(|e| e.to_string())?;
    let module_name = entity
        .module_name()
        .ok_or_else(|| format!("Missing module for {full_type}"))?;
    let name = entity
        .name()
        .ok_or_else(|| format!("Missing name for {full_type}"))?
        .to_string();
    let mod_id = entity.mod_id().unwrap_or("pz-vanilla").to_string();
    let mut overrides = Vec::new();
    let mut previous: Option<&str> = None;
    let mut exists_as_vanilla = entity.exists_as_vanilla() || mod_id == "pz-vanilla";
    for pair in entity.base.loaded_script_bodies().chunks_exact(2) {
        let Some(id) = pair[0].as_deref() else {
            continue;
        };
        if id == "pz-vanilla" {
            exists_as_vanilla = true;
        }
        if let Some(old) = previous.filter(|old| *old != id) {
            if !overrides.iter().any(|s| s == old) {
                overrides.push(old.to_string());
            }
        }
        previous = Some(id);
    }
    Ok(DictionaryInfo {
        registry_id: -1,
        module_name,
        name,
        full_type,
        is_modded: mod_id != "pz-vanilla",
        mod_id,
        exists_as_vanilla,
        obsolete,
        removed: false,
        mod_overrides: if overrides.is_empty() {
            None
        } else {
            Some(overrides)
        },
    })
}

fn register_scripts(
    dictionary: &mut WorldDictionary,
    sm: &pz_scripts::ScriptManager,
) -> Result<(), String> {
    use pz_scripts::objects::entity::GameEntityScript;
    use pz_scripts::script_type::ScriptType;
    // WorldDictionary's load lists are java.util.HashMaps keyed by full
    // type. Reuse the script loader's Java ordering, not Rust's randomized
    // HashMap order, when assigning new ids.
    let mut items = pz_scripts::java::JHashMap::new();
    let mut entities = pz_scripts::java::JHashMap::new();
    let modules = sm.module_list();
    let item_scripts: Vec<_> = modules
        .iter()
        .flat_map(|m| m.bucket(ScriptType::Item).loaded_definitions())
        .filter_map(|s| s.downcast::<pz_scripts::objects::items::Item>())
        .collect();
    let entity_scripts: Vec<_> = modules
        .iter()
        .flat_map(|m| m.bucket(ScriptType::Entity).loaded_definitions())
        .filter_map(|s| s.downcast::<GameEntityScript>())
        .collect();
    for item in &item_scripts {
        let item = item.borrow();
        let info = script_info(&item.entity, item.obsolete)?;
        items.insert(info.full_type.clone(), info);
    }
    for entity in &entity_scripts {
        let info = script_info(&entity.borrow(), false)?;
        entities.insert(info.full_type.clone(), info);
    }
    dictionary
        .reconcile_scripts(
            &items.values().cloned().collect::<Vec<_>>(),
            &entities.values().cloned().collect::<Vec<_>>(),
        )
        .map_err(|e| e.to_string())?;
    let item_ids: std::collections::HashMap<_, _> = dictionary
        .items
        .values()
        .map(|i| (i.full_type.as_str(), i.registry_id))
        .collect();
    let entity_ids: std::collections::HashMap<_, _> = dictionary
        .entities
        .values()
        .map(|i| (i.full_type.as_str(), i.registry_id))
        .collect();
    for item in item_scripts {
        let mut item = item.borrow_mut();
        if let Some(&id) = item_ids.get(item.full_name().as_str()) {
            // The Rust host can open another world without reloading the
            // scripts. An identical assignment is already registered.
            if item.entity.registry_id() != id {
                item.entity.set_registry_id(id).map_err(|e| e.to_string())?;
            }
        }
    }
    for entity in entity_scripts {
        let mut entity = entity.borrow_mut();
        let full_type = entity.full_name().map_err(|e| e.to_string())?;
        if let Some(&id) = entity_ids.get(full_type.as_str()) {
            if entity.registry_id() != id {
                entity.set_registry_id(id).map_err(|e| e.to_string())?;
            }
        }
    }
    Ok(())
}

fn save_data(save_dir: &Path, dictionary: &WorldDictionary) -> Result<(), String> {
    let bytes = dictionary.save().map_err(|e| e.to_string())?;
    std::fs::create_dir_all(save_dir).map_err(|e| e.to_string())?;
    let temporary = save_dir.join("WorldDictionary.tmp");
    std::fs::write(&temporary, bytes).map_err(|e| format!("Error saving WorldDictionary.: {e}"))?;
    std::fs::rename(&temporary, save_dir.join("WorldDictionary.bin"))
        .map_err(|e| format!("Error saving WorldDictionary.: {e}"))
}

/// `WorldDictionary.init()` for a local game: load saved ids, reconcile
/// the current item/entity scripts, register erosion object names, then
/// persist the dictionary before chunks and players load. `noSave` still
/// creates usable in-memory ids but neither reads nor writes a file.
///
/// TODO(port): register the Generic strings from XUI/FluidFilter and
/// component script registers (including their original version hashes)
/// when those script-loading hooks are implemented. Existing register
/// records are preserved. Readable/log/error-backup files remain unported.
pub fn init(
    save_dir: &Path,
    no_save: bool,
    is_new_game: bool,
    sm: &pz_scripts::ScriptManager,
    object_names: &[String],
) -> Result<Option<WorldDictionary>, String> {
    println!("WorldDictionary: Checking dictionary...");
    let initialize = || -> Result<WorldDictionary, String> {
        let mut dictionary =
            load_data(save_dir, no_save, is_new_game)?.unwrap_or_else(WorldDictionary::new);
        register_scripts(&mut dictionary, sm)?;
        dictionary
            .register_object_names(object_names, sm.debug)
            .map_err(|e| e.to_string())?;
        if dictionary.string_register("Generic").is_none() {
            dictionary.string_registers.push(Register {
                name: "Generic".into(),
                ..Default::default()
            });
        }
        for name in [
            "SpriteConfigs",
            "SpriteOverlayConfigs",
            "ContextMenuConfigs",
        ] {
            if dictionary.script_register(name).is_none() {
                dictionary.script_registers.push(Register {
                    name: name.into(),
                    ..Default::default()
                });
            }
        }
        if !no_save {
            save_data(save_dir, &dictionary)?;
        }
        Ok(dictionary)
    };
    match initialize() {
        Ok(d) => Ok(Some(d)),
        Err(e) => {
            eprintln!("ERROR: WorldDictionary: {e}");
            println!("WorldDictionary: Warning: error occurred loading dictionary!");
            Err(CANNOT_LOAD.into())
        }
    }
}

/// The game's `Rand` (`RandStandard.INSTANCE`) for the item factory.
struct StandardFactoryRng;

impl FactoryRng for StandardFactoryRng {
    fn next_int(&mut self, max: i32) -> i32 {
        pz_core::random::next(max)
    }
    fn next_float(&mut self, min: f32, max: f32) -> f32 {
        pz_core::random::next_float(min, max)
    }
}

/// [`EntityCodec`] over registries the registry owns.
struct OwnedEntityCodec(pz_entity::BasicContext);

impl OwnedEntityCodec {
    fn codec(&self) -> EntityCodec<'_> {
        EntityCodec { ctx: &self.0 }
    }
}

impl ComponentCodec for OwnedEntityCodec {
    fn load_archive_entity(
        &self,
        reader: &mut Reader<'_>,
        world_version: i32,
        components: &mut pz_inventory::ItemComponents,
    ) -> SaveResult<pz_entity::RetiredComponents> {
        self.codec().load_archive_entity(reader, world_version, components)
    }
    fn factory_runtime(
        &self,
        item: &pz_inventory::shared::ItemRef,
        template: &ItemTemplate,
        rng: &mut dyn FactoryRng,
        retired: &mut pz_entity::RetiredComponents,
        native_notices: &mut Vec<pz_entity::runtime_fluid::RuntimeNativeNotice>,
    ) -> pz_entity::runtime_load::HostResult<()> {
        self.codec().factory_runtime(item, template, rng, retired, native_notices)
    }
    fn load_entity(
        &self,
        r: &mut Reader<'_>,
        world_version: i32,
        components: &mut Vec<ComponentBlob>,
    ) -> SaveResult<Vec<i16>> {
        self.codec().load_entity(r, world_version, components)
    }
    fn save_entity(&self, w: &mut Writer, components: &[ComponentBlob]) -> SaveResult<()> {
        self.codec().save_entity(w, components)
    }
    fn factory_components(
        &self,
        template: &ItemTemplate,
        rng: &mut dyn FactoryRng,
    ) -> Vec<ComponentBlob> {
        self.codec().factory_components(template, rng)
    }
    fn factory_entity(
        &self,
        template: &ItemTemplate,
        rng: &mut dyn FactoryRng,
    ) -> Option<pz_entity::Components> {
        self.codec().factory_entity(template, rng)
    }
    fn entity_context(&self) -> Option<&dyn pz_entity::EntityContext> {
        Some(&self.0)
    }
    fn knows_factory_components(&self, template: &ItemTemplate) -> bool {
        self.codec().knows_factory_components(template)
    }
}

/// `Translator.getItemNameFromFullType` for every script item, as the
/// registry's item names (none while no translator is installed).
fn item_names(sm: &pz_scripts::ScriptManager) -> std::collections::HashMap<String, String> {
    let mut out = std::collections::HashMap::new();
    for item in pz_scripts::objects::items::get_all_items(sm) {
        let full = item.borrow().full_name();
        if !full.contains('.') {
            continue;
        }
        let name = pz_lua_api::bind_text::with_translator_mut(|t| {
            t.get_item_name_from_full_type(&full, |_| None)
        });
        if let Some(n) = name.filter(|n| *n != full) {
            out.insert(full, n);
        }
    }
    out
}

/// What the save's item registry reads besides the dictionary.
pub struct RegistryInputs<'a> {
    pub game_dir: &'a Path,
    pub save_tables: &'a pz_world::save::SaveTables,
    pub sprites: Option<&'a pz_world::IsoSpriteManager>,
    /// `GameTime.getWorldAgeHours()` now.
    pub world_age_hours: f64,
}

/// The save's item registry: `dictionary`'s ids over the shared
/// `ScriptManager`, with the game's `Rand`, the `ClothingItem` assets and
/// entity registries the Lua item environment has, the moveable sprites
/// and item textures (a client has every texture).
///
/// TODO(port): `ZomboidRadio.getRecordedMedia()` (`recorded_media`), the
/// radio channel lists (`channels`) and `MapDefinitions` (`map_definitions`).
pub fn script_items(dictionary: WorldDictionary, inputs: &RegistryInputs<'_>) -> ScriptItems {
    let sm = pz_lua_api::bind_registry::shared_script_manager();
    let names = item_names(&sm);
    let mut items = ScriptItems::new(&sm, dictionary, &names);
    items.world_age_hours = inputs.world_age_hours;
    items.blood_body_parts = inputs.save_tables.blood_body_parts;
    if let Some(s) = inputs.sprites {
        items.world_sprites = pz_world::save::items::moveable_sprites(s);
    }
    items.textures_loaded = true;
    items.rand = std::cell::RefCell::new(Some(Box::new(StandardFactoryRng)));
    items.assets = pz_lua_api::bind_items::item_env()
        .context()
        .assets()
        .cloned();
    match pz_lua_api::bind_items::item_entity_context(
        &inputs.game_dir.join("projectzomboid.jar"),
        &sm,
    ) {
        Ok(ctx) => items.components = Box::new(OwnedEntityCodec(ctx)),
        Err(e) => eprintln!("ERROR: item entity registries: {e}"),
    }
    items
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// A new empty directory, unique per call.
    fn temp_dir() -> std::path::PathBuf {
        static N: AtomicUsize = AtomicUsize::new(0);
        let n = N.fetch_add(1, Ordering::SeqCst);
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos());
        let d =
            std::env::temp_dir().join(format!("pz-engine-wd-{}-{n}-{nanos}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    /// A `WorldDictionary.bin` with one item, `Base.Axe` as id 1.
    fn one_item() -> Vec<u8> {
        let mut w = Writer::new();
        w.i32(1);
        w.i16(2);
        w.i8(0);
        w.i32(0);
        w.i32(0);
        w.i32(1);
        w.string("Base");
        w.i32(1);
        w.i16(1);
        w.i8(0);
        w.string("Axe");
        w.i8(2);
        for _ in 0..4 {
            w.i32(0);
        }
        w.buf
    }

    #[test]
    fn loads_the_save_dictionary() {
        let d = temp_dir();
        std::fs::write(d.join("WorldDictionary.bin"), one_item()).unwrap();
        let dict = init(&d, false, false, &pz_scripts::ScriptManager::new(), &[])
            .unwrap()
            .unwrap();
        assert_eq!(dict.item_type(1), Some("Base.Axe"));
        assert!(dict.items[&1].removed);
        assert_eq!(
            WorldDictionary::load(&std::fs::read(d.join("WorldDictionary.bin")).unwrap())
                .unwrap()
                .item_id("Base.Axe"),
            Some(1)
        );
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn missing_file_fails_a_loaded_game_only() {
        let d = temp_dir();
        let sm = pz_scripts::ScriptManager::new();
        assert_eq!(init(&d, false, false, &sm, &[]).unwrap_err(), CANNOT_LOAD);
        assert!(init(&d, false, true, &sm, &[]).unwrap().is_some());
        assert!(d.join("WorldDictionary.bin").is_file());
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn no_save_reads_and_writes_nothing_but_registers_in_memory() {
        let d = temp_dir();
        std::fs::write(d.join("WorldDictionary.bin"), [0u8, 1]).unwrap();
        let sm = fixture_scripts("item Pebble { Weight = 1, }");
        let dict = init(&d, true, false, &sm, &["Fern".into()])
            .unwrap()
            .unwrap();
        assert_eq!(dict.item_id("Base.Pebble"), Some(0));
        assert_eq!(dict.object_id("Fern"), Some(0));
        assert_eq!(
            std::fs::read(d.join("WorldDictionary.bin")).unwrap(),
            [0u8, 1]
        );
        assert!(!d.join("WorldDictionary.tmp").exists());
        assert_eq!(init(&d, false, false, &sm, &[]).unwrap_err(), CANNOT_LOAD);
        std::fs::remove_file(d.join("WorldDictionary.bin")).unwrap();
        init(&d, true, true, &sm, &[]).unwrap();
        assert!(!d.join("WorldDictionary.bin").exists());
        std::fs::remove_dir_all(&d).unwrap();
    }

    fn fixture_scripts(body: &str) -> pz_scripts::ScriptManager {
        use pz_scripts::script_type::{ScriptLoadMode, ScriptType};
        let sm = pz_scripts::ScriptManager::new();
        sm.parse_script(ScriptLoadMode::Init, &format!("module Base {{ {body} }}"))
            .unwrap();
        let module = sm.get_module("Base").unwrap();
        for t in [ScriptType::Item, ScriptType::Entity] {
            module.bucket(t).load_scripts(&sm, ScriptLoadMode::Init);
        }
        sm
    }

    #[test]
    fn new_game_dictionary_reloads_with_same_script_ids_and_appends_new_definitions() {
        let d = temp_dir();
        let sm = fixture_scripts(
            "item Pebble { Weight = 1, } item Retired { Obsolete = true, } entity Bench { }",
        );
        let initial = init(&d, false, true, &sm, &["Fern".into()])
            .unwrap()
            .unwrap();
        let pebble = initial.item_id("Base.Pebble").unwrap();
        let bench = initial.entity_id("Base.Bench").unwrap();
        assert_eq!(pebble, 0);
        assert_eq!(bench, 1);
        assert_eq!(initial.item_id("Base.Retired"), None);
        let item = pz_scripts::objects::items::get_all_items(&sm)
            .pop()
            .unwrap();
        assert_eq!(item.borrow().entity.registry_id(), pebble);

        let reload_sm = fixture_scripts(
            "item Pebble { Weight = 2, } item Cord { Weight = 1, } entity Bench { }",
        );
        let loaded = init(
            &d,
            false,
            false,
            &reload_sm,
            &["Fern".into(), "Shrub".into()],
        )
        .unwrap()
        .unwrap();
        assert_eq!(loaded.item_id("Base.Pebble"), Some(pebble));
        assert_eq!(loaded.entity_id("Base.Bench"), Some(bench));
        assert_eq!(loaded.item_id("Base.Cord"), Some(2));
        assert_eq!(loaded.object_id("Fern"), Some(0));
        assert_eq!(loaded.object_id("Shrub"), Some(1));
        assert_eq!(loaded.next_info_id, 3);
        // Reopening with the same in-process script manager must keep the
        // existing registry assignments, including id 0.
        init(&d, false, false, &reload_sm, &[]).unwrap();
        let disk =
            WorldDictionary::load(&std::fs::read(d.join("WorldDictionary.bin")).unwrap()).unwrap();
        assert_eq!(disk.items, loaded.items);
        assert_eq!(disk.entities, loaded.entities);
        assert!(!d.join("WorldDictionary.tmp").exists());
        std::fs::remove_dir_all(&d).unwrap();
    }
}
