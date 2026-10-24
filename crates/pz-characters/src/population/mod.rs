//! `zombie.core.skinnedmodel.population`'s appearance tables:
//! `ClothingDecals` (with `ClothingDecal`, `ClothingDecalGroup`),
//! `HairStyles`/`HairStyle`, `BeardStyles`/`BeardStyle`,
//! `VoiceStyles`/`VoiceStyle`, `PopTemplateManager`'s skin lists, and
//! `OutfitManager`/`Outfit`/`ClothingItemReference` with the outfit
//! draws, `DefaultClothing`, and `zombie.characters.HairOutfitDefinitions`.
//! `OutfitRNG` is
//! [`pz_core::random::outfit_rng`].
//!
//! The XML files are read from the installed game (`media/...`) and then
//! from each loaded mod (its version folder, else its `common/` folder);
//! a mod entry whose name matches an existing one (ignoring case) replaces
//! it in place, others are appended. [`xml`] holds the JAXB binding rules.
//!
//! [`Population`] keeps the statics (`HairStyles.instance`, ...);
//! [`PopulationStyles`] is the [`HumanStyles`] `HumanVisual` draws its
//! random hair, beard, skin and colours through.
//!
//! The original logs mod overrides on `DebugType.Clothing`; nothing is
//! logged here.

pub mod beard_styles;
pub mod clothing_decals;
pub mod default_clothing;
pub mod hair_outfit_definitions;
pub mod hair_styles;
pub mod outfit;
pub mod outfit_manager;
pub mod pop_template;
pub mod voice_styles;
pub mod xml;

pub use beard_styles::{BeardStyle, BeardStyles};
pub use clothing_decals::{ClothingDecal, ClothingDecalGroup, ClothingDecals};
pub use default_clothing::{DefaultClothing, DefaultClothingLists};
pub use hair_outfit_definitions::{
    HairOutfitDefinitions, HaircutDefinition, HaircutOutfitDefinition, StringChance,
};
pub use hair_styles::{HairStyle, HairStyleAlternate, HairStyles};
pub use outfit::{
    dress_in_outfit, ClothingItemRandomData, ClothingItemReference, ClothingItemSource, DressItem,
    Outfit, OutfitContext, OutfitRandomData,
};
pub use outfit_manager::{dress_in_clothing_item, OutfitManager};
pub use pop_template::PopTemplateManager;
pub use voice_styles::{VoiceStyle, VoiceStyles};

use pz_core::random::outfit_rng;
use pz_fs::jfile::SEP;
use pz_fs::GameFs;
use pz_inventory::visual::human::{HumanStyles, SkinList};
use pz_inventory::visual::Rgb;
use std::path::Path;

/// `ZomboidFileSystem.processFilePath(rel, File.separatorChar)`.
fn process_path(rel: &str) -> String {
    rel.replace(['/', '\\'], &SEP.to_string())
}

/// `<base>/<rel>`: the game's own copy, bypassing mod overrides.
pub(crate) fn base_file(fs: &GameFs, rel: &str) -> String {
    format!("{}{SEP}{}", fs.base.to_string_lossy(), process_path(rel))
}

/// The `Parse(filename)` pattern: a missing file is silently `null`; a
/// read or parse failure is logged and `null`.
pub(crate) fn parse_file<T>(
    path: &str,
    parse: impl FnOnce(&[u8]) -> Result<T, String>,
) -> Option<T> {
    let p = Path::new(path);
    if !p.is_file() {
        // `FileInputStream` throws `FileNotFoundException` for missing
        // files and directories alike.
        return None;
    }
    let bytes = match std::fs::read(p) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("ERROR: reading {path}: {e}");
            return None;
        }
    };
    match parse(&bytes) {
        Ok(v) => Some(v),
        Err(e) => {
            eprintln!("ERROR: parsing {path}: {e}");
            None
        }
    }
}

/// For each loaded mod (`getModIDs`, available ones only), the file `rel`
/// parsed from the mod's version folder, or failing that its `common/`
/// folder, with the mod id.
pub(crate) fn mod_files<T>(
    fs: &mut GameFs,
    rel: &str,
    parse: impl Fn(&str) -> Option<T>,
) -> Vec<(String, T)> {
    let mut out = Vec::new();
    for id in fs.mod_ids().to_vec() {
        let Some(dir) = fs.available_mod_dir(&id) else {
            continue;
        };
        let Some(info) = fs.mod_info(&dir) else {
            continue;
        };
        let (version, common) = (info.version_dir.clone(), info.common_dir.clone());
        let rel = process_path(rel);
        let found = parse(&format!("{version}{SEP}{rel}"))
            .or_else(|| parse(&format!("{common}{SEP}{rel}")));
        if let Some(v) = found {
            out.push((id, v));
        }
    }
    out
}

/// Merge `incoming` into `list`: replace the entry `find` locates, else
/// append (the mod loop of every `init`).
pub(crate) fn merge_by<T>(
    list: &mut Vec<T>,
    incoming: Vec<T>,
    find: impl Fn(&[T], &T) -> Option<usize>,
) {
    for item in incoming {
        match find(list, &item) {
            Some(i) => list[i] = item,
            None => list.push(item),
        }
    }
}

/// `SurvivorDesc.getRandomSkinColor()`.
pub fn random_skin_color() -> Rgb {
    if outfit_rng::next(3) == 0 {
        let r = outfit_rng::next_float(0.5, 0.6);
        let g = outfit_rng::next_float(0.3, 0.4);
        let b = outfit_rng::next_float(0.15, 0.23);
        return [r, g, b];
    }
    let r = outfit_rng::next_float(0.9, 1.0);
    let g = outfit_rng::next_float(0.75, 0.88);
    let b = outfit_rng::next_float(0.45, 0.58);
    [r, g, b]
}

/// The statics of the population classes.
#[derive(Default)]
pub struct Population {
    /// `ClothingDecals.instance`.
    pub clothing_decals: Option<ClothingDecals>,
    /// `HairStyles.instance`.
    pub hair_styles: Option<HairStyles>,
    /// `BeardStyles.instance`.
    pub beard_styles: Option<BeardStyles>,
    /// `VoiceStyles.instance`.
    pub voice_styles: Option<VoiceStyles>,
    /// `PopTemplateManager.instance`.
    pub pop_template: PopTemplateManager,
    /// `HairOutfitDefinitions.instance`.
    pub hair_outfit_definitions: HairOutfitDefinitions,
    /// `OutfitManager.instance`.
    pub outfit_manager: Option<OutfitManager>,
    /// `DefaultClothing.instance`.
    pub default_clothing: DefaultClothing,
}

impl Population {
    /// `GameWindow.initShared`'s loads, in its order (`ClothingDecals`,
    /// `BeardStyles`, `HairStyles`, `OutfitManager`, `VoiceStyles`), then
    /// `PopTemplateManager.init` (run by `ModelManager`).
    pub fn init(&mut self, fs: &mut GameFs) {
        self.clothing_decals = ClothingDecals::init(fs);
        self.beard_styles = BeardStyles::init(fs);
        self.hair_styles = HairStyles::init(fs);
        self.outfit_manager = OutfitManager::init(fs);
        self.voice_styles = VoiceStyles::init(fs);
        self.pop_template = PopTemplateManager::default();
        self.pop_template.init();
    }

    /// The [`HumanStyles`] over these tables. `hair_common_colors` is
    /// `SurvivorDesc.HairCommonColors`, `world_age_days`
    /// `IsoWorld.getWorldAgeDays()`.
    pub fn styles<'a>(
        &'a self,
        hair_common_colors: &'a [Rgb],
        world_age_days: f32,
    ) -> PopulationStyles<'a> {
        PopulationStyles {
            population: self,
            hair_common_colors,
            world_age_days,
        }
    }
}

/// [`HumanStyles`] backed by a [`Population`] and [`outfit_rng`].
/// Where the original would throw (`HairStyles.instance` null, no hair
/// colours) the lookups find nothing.
pub struct PopulationStyles<'a> {
    pub population: &'a Population,
    pub hair_common_colors: &'a [Rgb],
    pub world_age_days: f32,
}

impl HumanStyles for PopulationStyles<'_> {
    fn next(&self, bound: i32) -> i32 {
        outfit_rng::next(bound)
    }
    fn next_unit(&self) -> f32 {
        outfit_rng::next_float(0.0, 1.0)
    }
    fn skins(&self, list: SkinList) -> &[String] {
        self.population.pop_template.skins(list)
    }
    fn has_hair_style(&self, female: bool, name: Option<&str>) -> bool {
        self.population
            .hair_styles
            .as_ref()
            .is_some_and(|h| h.find_style(female, name).is_some())
    }
    fn random_hair_style(&self, female: bool, outfit: Option<&str>) -> Option<String> {
        let h = self.population.hair_styles.as_ref()?;
        let defs = &self.population.hair_outfit_definitions;
        Some(if female {
            h.get_random_female_style(defs, outfit, self.world_age_days)
        } else {
            h.get_random_male_style(defs, outfit, self.world_age_days)
        })
    }
    fn has_beard_style(&self, name: Option<&str>) -> bool {
        self.population
            .beard_styles
            .as_ref()
            .is_some_and(|b| b.find_style(name).is_some())
    }
    fn random_beard_style(&self, outfit: Option<&str>) -> Option<String> {
        let b = self.population.beard_styles.as_ref()?;
        b.get_random_style(&self.population.hair_outfit_definitions, outfit)
    }
    fn random_haircut_color(&self, outfit: Option<&str>) -> Option<Rgb> {
        self.population
            .hair_outfit_definitions
            .get_random_haircut_color(outfit, self.hair_common_colors)
    }
    fn random_skin_color(&self) -> Rgb {
        random_skin_color()
    }
    fn world_age_days(&self) -> f32 {
        self.world_age_days
    }
}
