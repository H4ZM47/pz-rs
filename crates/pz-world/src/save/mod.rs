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
pub mod movers;
pub mod object_io;
pub mod vehicle;
pub mod vehicles_db;
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
    /// Reads the tables from the installed game's `projectzomboid.jar`
    /// (see [`SaveTables::from_jar`]).
    pub fn from_game_jar(jar: &std::path::Path) -> Result<Self, String> {
        let jar = pz_classfile::Jar::open(jar).map_err(|e| format!("{}: {e}", jar.display()))?;
        Self::from_jar(&jar)
    }

    /// Reads the tables from an open game jar: the `(classId, objectName)`
    /// constructor arguments of the factories `IsoObject.initFactory`
    /// passes to `addIsoObjectFactory`, in call order, and the ordinal of
    /// `zombie.characterTextures.BloodBodyPartType.MAX`.
    pub fn from_jar(jar: &pz_classfile::Jar) -> Result<Self, String> {
        const BLOOD: &str = "zombie/characterTextures/BloodBodyPartType";
        let parts =
            pz_classfile::enum_constants(jar, BLOOD).map_err(|e| format!("{BLOOD}: {e}"))?;
        let blood_body_parts = parts
            .iter()
            .position(|n| n == "MAX")
            .ok_or_else(|| format!("{BLOOD} has no MAX"))?;
        let t = SaveTables {
            factory: init_factory_calls(jar)?,
            blood_body_parts,
            character_stats: Vec::new(),
        };
        if t.class_name(0) != Some("IsoObject") {
            return Err("factory table has no IsoObject at 0".into());
        }
        Ok(t)
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

    /// The tables in the [`SaveTables::from_text`] format.
    pub fn to_text(&self) -> String {
        let mut s = String::new();
        for (id, name) in &self.factory {
            s.push_str(&format!("{id}:{name}\n"));
        }
        s.push_str(&format!("bloodBodyParts={}\n", self.blood_body_parts));
        s
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

/// The `(classId, objectName)` pairs of `IsoObject.initFactory()`: a run of
/// `addIsoObjectFactory(new IsoObject$N(id, "name"))` calls (`new`, `dup`,
/// an int push, `ldc`, `invokespecial <init>(BLjava/lang/String;)V`,
/// `invokestatic addIsoObjectFactory`, then `pop` or `putstatic`). Any other
/// instruction is an error, so a changed method is reported rather than
/// misread.
fn init_factory_calls(jar: &pz_classfile::Jar) -> Result<Vec<(i8, String)>, String> {
    let cf = pz_classfile::load_class(jar, "zombie/iso/IsoObject")
        .map_err(|e| format!("zombie/iso/IsoObject: {e}"))?;
    factory_calls(&cf)
}

fn factory_calls(cf: &pz_classfile::ClassFile) -> Result<Vec<(i8, String)>, String> {
    use pz_classfile::class::Constant;
    const CLASS: &str = "zombie/iso/IsoObject";
    let e = |e: pz_classfile::Error| format!("{CLASS}.initFactory: {e}");
    let code = cf
        .method("initFactory")
        .and_then(|m| m.code.as_deref())
        .ok_or_else(|| format!("{CLASS} has no initFactory"))?;
    let u16_at = |pc: usize| -> Result<u16, String> {
        code.get(pc + 1..pc + 3)
            .map(|b| u16::from_be_bytes([b[0], b[1]]))
            .ok_or_else(|| format!("{CLASS}.initFactory: truncated"))
    };
    let mut out = Vec::new();
    let (mut int, mut string, mut pending) = (None, None, None);
    let mut pc = 0;
    while pc < code.len() {
        let op = code[pc];
        let len = match op {
            0xbb | 0xb3 => 3, // new, putstatic
            0x59 | 0x57 => 1, // dup, pop
            0x02..=0x08 => {
                int = Some(op as i64 - 3);
                1
            }
            0x10 => {
                int = Some(*code.get(pc + 1).ok_or("truncated")? as i8 as i64);
                2
            }
            0x11 => {
                int = Some(u16_at(pc)? as i16 as i64);
                3
            }
            0x12 | 0x13 => {
                let idx = if op == 0x12 {
                    *code.get(pc + 1).ok_or("truncated")? as u16
                } else {
                    u16_at(pc)?
                };
                string = match cf.constant(idx).map_err(e)? {
                    Constant::String(s) => Some(cf.utf8(*s).map_err(e)?.to_string()),
                    other => return Err(format!("{CLASS}.initFactory: ldc {other:?}")),
                };
                if op == 0x12 {
                    2
                } else {
                    3
                }
            }
            0xb7 => {
                let r = cf.member_ref(u16_at(pc)?).map_err(e)?;
                if r.name != "<init>" || r.descriptor != "(BLjava/lang/String;)V" {
                    return Err(format!(
                        "{CLASS}.initFactory: calls {}.{}{}",
                        r.class, r.name, r.descriptor
                    ));
                }
                let id = int.take().ok_or("factory id is not a constant")?;
                let id = i8::try_from(id).map_err(|_| format!("factory id {id}"))?;
                pending = Some((id, string.take().ok_or("factory name is not a constant")?));
                3
            }
            0xb8 => {
                let r = cf.member_ref(u16_at(pc)?).map_err(e)?;
                if r.name != "addIsoObjectFactory" {
                    return Err(format!("{CLASS}.initFactory: calls {}.{}", r.class, r.name));
                }
                out.push(
                    pending
                        .take()
                        .ok_or("addIsoObjectFactory without a factory")?,
                );
                3
            }
            0xb1 => break, // return
            _ => {
                return Err(format!(
                    "{CLASS}.initFactory: unexpected opcode {op:#04x} at {pc}"
                ))
            }
        };
        pc += len;
    }
    if out.is_empty() {
        return Err(format!("{CLASS}.initFactory adds no factories"));
    }
    Ok(out)
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
    /// `SandboxOptions.worldItemRemovalList` as `getSplitCSVList()` has it
    /// (see [`SaveSettings::set_world_item_removal_list`]).
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
    /// Sets the removal list from the `WorldItemRemovalList` option string,
    /// as `StringConfigOption.setValueInternal` caches it: every whitespace
    /// character removed, then split on commas.
    pub fn set_world_item_removal_list(&mut self, value: &str) {
        let squeezed: String = value.chars().filter(|c| !is_java_regex_space(*c)).collect();
        self.world_item_removal_list = java_split(&squeezed, ',');
    }

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

/// `\s` in a `java.util.regex` pattern: `[ \t\n\x0B\f\r]`.
fn is_java_regex_space(c: char) -> bool {
    matches!(c, ' ' | '\t' | '\n' | '\u{b}' | '\u{c}' | '\r')
}

/// `String.split(String.valueOf(sep))`: a string without `sep` is the
/// only element; otherwise trailing empty strings are removed.
fn java_split(s: &str, sep: char) -> Vec<String> {
    if !s.contains(sep) {
        return vec![s.to_string()];
    }
    let mut v: Vec<String> = s.split(sep).map(str::to_string).collect();
    while v.last().is_some_and(String::is_empty) {
        v.pop();
    }
    v
}

#[cfg(test)]
mod animal_tests;
#[cfg(test)]
pub(crate) mod movers_tests;
#[cfg(test)]
mod tests;
