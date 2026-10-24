//! Chunk saving and loading: the per-chunk `map/<wx>/<wy>.bin` files
//! (`IsoChunk.Save`/`LoadFromDiskOrBufferInternal`, `IsoGridSquare.save`/
//! `load`, `IsoObject.save`/`load` and the object classes).
//!
//! A chunk file is not compressed. Its header is a debug flag, the world
//! version, the total length and a `CRC32` of everything after the
//! 17-byte header; see [`chunk_file`].
//!
//! Inventory items and ECS components are kept as their length-delimited
//! bytes ([`items`], [`crate::objects::ComponentBlob`]): the format frames
//! both, so they are read and written exactly without the item or entity
//! systems.

pub mod animal;
pub mod chunk_file;
pub mod io;
pub mod items;
pub mod lua_table;
pub mod object_io;
pub mod visual;

use crate::java::string_hash;
use crate::meta_grid::IsoMetaGrid;
use crate::sprite::IsoSpriteManager;
use io::{err, SaveResult};
use items::ItemRegistry;

/// `IsoChunk.Save` writes this world version.
pub const WORLD_VERSION: i32 = 249;

/// Game-code tables the format depends on. Not part of this repository:
/// read from the installed game at runtime.
#[derive(Clone, Debug)]
pub struct SaveTables {
    /// `IsoObject.initFactory`: `(classId, objectName)`.
    pub factory: Vec<(i8, String)>,
    /// `BloodBodyPartType.MAX.index()`: length of the visual blood, dirt
    /// and hole arrays.
    pub blood_body_parts: usize,
    /// `CharacterStat` declarations in `ORDERED_STATS` order (animals'
    /// saved stats), filled from `BuiltinTables::character_stats` and
    /// `ordered_stats` (see [`animal::ordered_character_stats`]). Empty by
    /// default; loading an animal needs it.
    pub character_stats: Vec<animal::CharacterStatDecl>,
}

impl SaveTables {
    /// Reads the tables from the installed game's `projectzomboid.jar`.
    ///
    /// LOADER STUB: to be wired to the `pz-classfile` class-file reader
    /// (the `IsoObjectFactory` constructor arguments in
    /// `IsoObject.initFactory` and the constant count of
    /// `zombie.characterTextures.BloodBodyPartType`).
    pub fn from_game_jar(_jar: &std::path::Path) -> Result<Self, String> {
        Err("SaveTables::from_game_jar is not wired to pz-classfile yet".into())
    }

    /// Reads `id:name` lines and a `bloodBodyParts=<n>` line.
    pub fn from_text(text: &str) -> Result<Self, String> {
        let mut factory = Vec::new();
        let mut parts = None;
        for line in text.lines().map(str::trim).filter(|l| !l.is_empty()) {
            if let Some(n) = line.strip_prefix("bloodBodyParts=") {
                parts = Some(n.parse::<usize>().map_err(|e| e.to_string())?);
            } else {
                let (id, name) = line.split_once(':').ok_or("expected id:name")?;
                factory.push((id.parse::<i8>().map_err(|e| e.to_string())?, name.into()));
            }
        }
        let t = SaveTables {
            factory,
            blood_body_parts: parts.ok_or("missing bloodBodyParts")?,
            character_stats: Vec::new(),
        };
        if t.class_name(0) != Some("IsoObject") {
            return Err("factory table has no IsoObject at 0".into());
        }
        Ok(t)
    }

    /// `IsoObject.factoryGetClassID(name)`: matched by `String.hashCode`,
    /// falling back to the `IsoObject` id.
    pub fn class_id(&self, name: &str) -> i8 {
        let h = string_hash(name);
        self.factory
            .iter()
            .find(|(_, n)| string_hash(n) == h)
            .or_else(|| self.factory.iter().find(|(_, n)| n == "IsoObject"))
            .map_or(0, |&(id, _)| id)
    }

    pub fn class_name(&self, id: i8) -> Option<&str> {
        self.factory
            .iter()
            .find(|&&(i, _)| i == id)
            .map(|(_, n)| n.as_str())
    }

    #[cfg(test)]
    pub fn for_tests() -> Self {
        let names = [
            "IsoObject",
            "Player",
            "Survivor",
            "Zombie",
            "Pushable",
            "WheelieBin",
            "WorldInventoryItem",
            "Jukebox",
            "Curtain",
            "Radio",
            "Television",
            "DeadBody",
            "Barbecue",
            "ClothingDryer",
            "ClothingWasher",
            "Fireplace",
            "Stove",
            "Door",
            "Thumpable",
            "IsoTrap",
            "IsoBrokenGlass",
            "IsoCarBatteryCharger",
            "IsoGenerator",
            "IsoCompost",
            "Mannequin",
            "Window",
            "Barricade",
            "Tree",
            "LightSwitch",
            "ZombieGiblets",
            "MolotovCocktail",
            "Fire",
            "Vehicle",
            "CombinationWasherDryer",
            "StackedWasherDryer",
            "Animal",
            "FeedingTrough",
            "IsoHutch",
            "IsoAnimalTrack",
            "ButcherHook",
            "IsoWindowFrame",
        ];
        // Synthetic ids: tests must not depend on the game's numbering.
        SaveTables {
            factory: std::iter::once((0, "IsoObject".to_string()))
                .chain(
                    names[1..]
                        .iter()
                        .enumerate()
                        .map(|(i, n)| ((i as i8 + 1) * 3, n.to_string())),
                )
                .collect(),
            blood_body_parts: 7,
            character_stats: Vec::new(),
        }
    }
}

/// Game state loading and saving read.
#[derive(Clone, Debug)]
pub struct SaveSettings {
    /// `GameTime.getWorldAgeHours()`.
    pub world_age_hours: f64,
    /// `SandboxOptions.bloodSplatLifespanDays`.
    pub blood_splat_lifespan_days: i32,
    /// `SandboxOptions.hoursForWorldItemRemoval`.
    pub hours_for_world_item_removal: f64,
    /// `SandboxOptions.worldItemRemovalList`, split on commas and trimmed.
    pub world_item_removal_list: Vec<String>,
    /// `SandboxOptions.itemRemovalListBlacklistToggle`.
    pub item_removal_list_blacklist: bool,
    /// `Core.tileScale`: moving objects are created with
    /// `offsetX = 32 * tileScale`, `offsetY = 96 * tileScale` (saving writes those, not the
    /// loaded offsets).
    pub tile_scale: i32,
    /// `GameTime.getCalender().getTimeInMillis()` (saved by `IsoFire`).
    pub calendar_millis: i64,
}

impl Default for SaveSettings {
    fn default() -> Self {
        SaveSettings {
            world_age_hours: 0.0,
            blood_splat_lifespan_days: 0,
            hours_for_world_item_removal: 24.0,
            world_item_removal_list: Vec::new(),
            item_removal_list_blacklist: false,
            tile_scale: 2,
            calendar_millis: 0,
        }
    }
}

impl SaveSettings {
    /// `SandboxOptions.worldItemRemovalListContains`.
    pub fn removal_list_contains(&self, s: &str) -> bool {
        self.world_item_removal_list.iter().any(|x| x == s)
    }
}

/// Hooks into systems outside this crate. The defaults behave like a game
/// with no animal or Lua definitions loaded and no animal zones.
pub trait SaveHooks {
    /// Components the class's `(IsoCell)` constructor adds before `load`,
    /// as their saved bytes, besides the ones this crate builds itself
    /// (the `IsoFeedingTrough` fluid container). A loaded component of the
    /// same type loads in place. The default adds none.
    fn factory_components(&self, _object_name: &str) -> Vec<crate::objects::ComponentBlob> {
        Vec::new()
    }

    /// `IsoHutch.getMaxNestBox()` for the hutch sprite, if its definition
    /// is known: nest boxes past it are dropped on load.
    fn hutch_max_nest_box(&self, _sprite_name: &str) -> Option<i32> {
        None
    }

    /// `IsoHutch.getMaxAnimals()` for the hutch sprite. Needed only when a
    /// loaded animal has to draw a new place in the hutch.
    fn hutch_max_animals(&self, _sprite_name: &str) -> Option<i32> {
        None
    }

    /// `FeedingTroughDef` for a trough sprite (`IsoFeedingTrough.initWithDef`):
    /// `maxWater` and `maxFeed`.
    fn feeding_trough_def(&self, _sprite_name: &str) -> Option<(f32, i32)> {
        None
    }

    /// `AnimalDefinitions.getDef(type)`. Loading an animal without its
    /// definition fails, as in the original.
    fn animal_definition(&self, _animal_type: &str) -> Option<&animal::AnimalDefinition> {
        None
    }

    /// Whether `animalZoneHandler.getZone(uuid)` finds the animal's zone.
    fn animal_zone_exists(&self, _msb: i64, _lsb: i64) -> bool {
        false
    }

    /// `Rand.Next(min, max)` for ints (`max` exclusive). The game's
    /// generator is not seeded here; the default returns `min`.
    fn rand_next(&self, min: i32, _max: i32) -> i32 {
        min
    }

    /// `Rand.Next(min, max)` for floats. The default returns `min`.
    fn rand_next_float(&self, min: f32, _max: f32) -> f32 {
        min
    }
}

/// [`SaveHooks`] with every default.
pub struct NoSaveHooks;
impl SaveHooks for NoSaveHooks {}

/// What chunk loading and saving need.
pub struct SaveCtx<'a> {
    /// `IsoSpriteManager.instance` (sprite ids, properties for
    /// `FixStackableObjects`).
    pub sprites: &'a IsoSpriteManager,
    pub tables: &'a SaveTables,
    pub settings: &'a SaveSettings,
    pub items: &'a dyn ItemRegistry,
    pub hooks: &'a dyn SaveHooks,
    /// `IsoWorld.instance.getMetaGrid()`, for squares of chunks whose lot
    /// header is loaded (`IsoChunk.lotheader != null`): room ids and light
    /// switch state. `None` behaves like a chunk with no lot header.
    pub meta: Option<&'a IsoMetaGrid>,
}

impl SaveCtx<'_> {
    pub fn sprite_id(&self, s: crate::sprite::SpriteIdx) -> i32 {
        self.sprites.sprites[s].id
    }

    pub fn class_id(&self, name: &str) -> SaveResult<i8> {
        let id = self.tables.class_id(name);
        if self.tables.factory.is_empty() {
            return err("empty factory table");
        }
        Ok(id)
    }
}

#[cfg(test)]
mod animal_tests;
#[cfg(test)]
mod tests;
