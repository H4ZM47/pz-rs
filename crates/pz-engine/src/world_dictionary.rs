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
use pz_inventory::dictionary::WorldDictionary;
use pz_inventory::factory::ScriptItems;
use pz_inventory::io::{Reader, SaveResult, Writer};
use pz_inventory::ItemTemplate;

/// `WorldDictionaryException`'s message for a failed `init()`.
const CANNOT_LOAD: &str = "WorldDictionary: Cannot load world due to WorldDictionary error.";

/// `DictionaryData.load()`: `WorldDictionary.bin` of the current save
/// (`None`: nothing read, for a `noSave` game or a new game without the
/// file). A missing file in a loaded game, or one that does not read, is
/// a `WorldDictionaryException`.
fn load_data(save_dir: &Path, no_save: bool, is_new_game: bool) -> Result<Option<WorldDictionary>, String> {
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
    WorldDictionary::load(&bytes).map(Some).map_err(|e| format!("Error loading WorldDictionary.: {}", e.0))
}

/// `WorldDictionary.init()` in a local game (`isNewGame` is
/// `GameLoadingState.newGame`, set by `WorldDictionary.setIsNewGame`):
/// `DictionaryData.load()`, then the original matches the loaded scripts
/// against it. An error is logged and becomes the
/// `WorldDictionaryException` that stops the world load.
///
/// TODO(port): the rest of `init()`: `collectObjectNames`,
/// `WorldDictionaryLogger`, `backupCurrentDataSet`, `collectStrings`,
/// `parseInfoLoadList` / `parseCurrentInfoSet` (registry ids for items and
/// entities new since the save, removed and mod-changed ones),
/// `parseObjectNameLoadList`, the `StringDictionary` / `ScriptsDictionary`
/// registers, `getDictionaryMods`, then `saveAsText`
/// (`WorldDictionaryReadable.lua`), `save()` and
/// `deleteBackupCurrentDataSet`; on error `createErrorBackups` and
/// `saveLog("WorldDictionaryLog.lua")`. Until then a new game has no
/// dictionary, and items added to the scripts since a save have no ids.
pub fn init(save_dir: &Path, no_save: bool, is_new_game: bool) -> Result<Option<WorldDictionary>, String> {
    println!("WorldDictionary: Checking dictionary...");
    match load_data(save_dir, no_save, is_new_game) {
        Ok(d) => Ok(d),
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
    fn load_entity(&self, r: &mut Reader<'_>, world_version: i32, components: &mut Vec<ComponentBlob>) -> SaveResult<Vec<i16>> {
        self.codec().load_entity(r, world_version, components)
    }
    fn save_entity(&self, w: &mut Writer, components: &[ComponentBlob]) -> SaveResult<()> {
        self.codec().save_entity(w, components)
    }
    fn factory_components(&self, template: &ItemTemplate, rng: &mut dyn FactoryRng) -> Vec<ComponentBlob> {
        self.codec().factory_components(template, rng)
    }
    fn factory_entity(&self, template: &ItemTemplate, rng: &mut dyn FactoryRng) -> Option<pz_entity::Components> {
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
        let name = pz_lua_api::bind_text::with_translator_mut(|t| t.get_item_name_from_full_type(&full, |_| None));
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
    items.assets = pz_lua_api::bind_items::item_env().context().assets().cloned();
    match pz_lua_api::bind_items::item_entity_context(&inputs.game_dir.join("projectzomboid.jar"), &sm) {
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
        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos());
        let d = std::env::temp_dir().join(format!("pz-engine-wd-{}-{n}-{nanos}", std::process::id()));
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
        let dict = init(&d, false, false).unwrap().unwrap();
        assert_eq!(dict.item_type(1), Some("Base.Axe"));
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn missing_file_fails_a_loaded_game_only() {
        let d = temp_dir();
        assert_eq!(init(&d, false, false).unwrap_err(), CANNOT_LOAD);
        assert!(init(&d, false, true).unwrap().is_none());
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn no_save_reads_nothing() {
        let d = temp_dir();
        std::fs::write(d.join("WorldDictionary.bin"), one_item()).unwrap();
        assert!(init(&d, true, false).unwrap().is_none());
        std::fs::write(d.join("WorldDictionary.bin"), [0u8, 1]).unwrap();
        assert_eq!(init(&d, false, false).unwrap_err(), CANNOT_LOAD);
        std::fs::remove_dir_all(&d).unwrap();
    }
}
