//! `zombie.PersistentOutfits`: the outfit ids zombies keep across
//! loads. An id packs the outfit's index in [`PersistentOutfits::outfit_names`]
//! (bits 16..30), a variant `1..=500` naming one of 500 saved `OutfitRNG`
//! seeds (bits 0..15), [`FEMALE_BIT`] and [`NO_HAT_BIT`]. Dressing in an id
//! reseeds `OutfitRNG` from the variant's seed, so a zombie dresses the same
//! way every time.
//!
//! The outfit list is `OutfitManager`'s female then male outfits (each
//! sorted by name), then the custom outfitters (`ZombiesZoneDefinition`'s
//! `ZZD.<zone>.<outfit>` entries; `ReanimatedPlayer` in multiplayer). The
//! seeds are drawn from `Rand` and saved as `z_outfits.bin` in the save.

use pz_core::random::{self as rand, outfit_rng};
use pz_scripts::java::{compare, eq_ignore_case};
use std::path::Path;

use super::OutfitManager;

/// `INVALID_ID`.
pub const INVALID_ID: i32 = 0;
/// `FEMALE_BIT`.
pub const FEMALE_BIT: i32 = i32::MIN;
/// `NO_HAT_BIT`.
pub const NO_HAT_BIT: i32 = 32768;
/// `NUM_SEEDS`.
pub const NUM_SEEDS: usize = 500;
const FILE_MAGIC: [u8; 4] = *b"PSTZ";
const FILE_VERSION_LATEST: i32 = 1;

/// `IOutfitter`: what dressing in an outfit of this entry runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outfitter {
    /// `PersistentOutfits::ApplyOutfit` (an `OutfitManager` outfit).
    Outfit,
    /// `ZombiesZoneDefinition::ApplyCustomOutfit`.
    ZombiesZone,
    /// `SharedDescriptors::ApplyReanimatedPlayerOutfit` (multiplayer).
    ReanimatedPlayer,
}

/// `PersistentOutfits.Data`.
#[derive(Clone, Debug, PartialEq)]
pub struct OutfitData {
    pub index: i32,
    pub outfit_name: String,
    pub use_seed: bool,
    pub outfitter: Outfitter,
}

/// What [`PersistentOutfits::dress_in_outfit`] chose: `OutfitRNG` is
/// seeded (for a seeded entry) and the outfitter runs with these.
#[derive(Clone, Debug, PartialEq)]
pub struct OutfitToApply {
    /// The id after `getOutfit` (a fresh variant if it had none).
    pub outfit_id: i32,
    pub outfit_name: String,
    pub outfitter: Outfitter,
}

/// A name map with `String.CASE_INSENSITIVE_ORDER` keys (a `TreeMap`):
/// `put` keeps the first spelling and replaces the value.
#[derive(Clone, Debug, Default)]
struct NameMap(Vec<(String, usize)>);

impl NameMap {
    fn get(&self, name: &str) -> Option<usize> {
        self.0.iter().find(|(k, _)| eq_ignore_case(k, name)).map(|&(_, v)| v)
    }
    fn put(&mut self, name: &str, v: usize) {
        match self.0.iter_mut().find(|(k, _)| eq_ignore_case(k, name)) {
            Some(e) => e.1 = v,
            None => self.0.push((name.to_string(), v)),
        }
    }
}

/// `PersistentOutfits.instance`.
#[derive(Clone, Debug)]
pub struct PersistentOutfits {
    pub seeds: [i64; NUM_SEEDS],
    outfit_names: Vec<String>,
    all: Vec<OutfitData>,
    female: Vec<usize>,
    male: Vec<usize>,
    outfit_to_data: NameMap,
    outfit_to_female: NameMap,
    outfit_to_male: NameMap,
}

impl Default for PersistentOutfits {
    fn default() -> Self {
        PersistentOutfits {
            seeds: [0; NUM_SEEDS],
            outfit_names: Vec::new(),
            all: Vec::new(),
            female: Vec::new(),
            male: Vec::new(),
            outfit_to_data: NameMap::default(),
            outfit_to_female: NameMap::default(),
            outfit_to_male: NameMap::default(),
        }
    }
}

/// Where `init` saves and loads: `z_outfits.bin` in the current save, and
/// whether saving is off (`Core.isNoSave()`).
pub struct OutfitsFile<'a> {
    pub path: &'a Path,
    pub no_save: bool,
}

impl PersistentOutfits {
    /// `init()`: the seeds (`Rand.Next(Integer.MAX_VALUE)` each, not on a
    /// client), the outfit lists, then the custom outfitters
    /// (`custom`: `ZombiesZoneDefinition.registerCustomOutfits`'s names,
    /// in order; `RandomizedVehicleStoryBase.registerCustomOutfits` adds
    /// none; `ReanimatedPlayer` when `multiplayer`), then (not on a client)
    /// `load()` and `save()` of `file`. Read and write failures are
    /// logged, as `ExceptionLogger` does.
    pub fn init(&mut self, outfits: &OutfitManager, custom: &[String], client: bool, multiplayer: bool, file: Option<OutfitsFile<'_>>) {
        self.all.clear();
        self.female.clear();
        self.male.clear();
        self.outfit_to_data = NameMap::default();
        self.outfit_to_female = NameMap::default();
        self.outfit_to_male = NameMap::default();
        self.outfit_names.clear();
        if !client {
            for s in self.seeds.iter_mut() {
                *s = rand::next(i32::MAX) as i64;
            }
        }
        self.init_outfit_list(outfits, true);
        self.init_outfit_list(outfits, false);
        for name in custom {
            self.register_outfitter(name, true, Outfitter::ZombiesZone);
        }
        if multiplayer {
            self.register_outfitter("ReanimatedPlayer", false, Outfitter::ReanimatedPlayer);
        }
        if client {
            return;
        }
        if let Some(f) = file {
            self.load(f.path);
            if !f.no_save {
                self.save(f.path);
            }
        }
    }

    fn init_outfit_list(&mut self, outfits: &OutfitManager, female: bool) {
        let list = if female { &outfits.female_outfits } else { &outfits.male_outfits };
        let mut names: Vec<&str> = list.iter().map(|o| o.name.as_str()).collect();
        names.sort_by(|a, b| compare(a, b));
        for n in names {
            self.init_outfit(n, female, true, Outfitter::Outfit);
        }
    }

    fn init_outfit(&mut self, outfit_name: &str, female: bool, use_seed: bool, outfitter: Outfitter) {
        let i = match self.outfit_to_data.get(outfit_name) {
            Some(i) => i,
            None => {
                let i = self.all.len();
                self.all.push(OutfitData {
                    index: i as i16 as i32,
                    outfit_name: outfit_name.to_string(),
                    use_seed,
                    outfitter,
                });
                self.outfit_names.push(outfit_name.to_string());
                self.outfit_to_data.put(outfit_name, i);
                i
            }
        };
        if female {
            self.female.push(i);
            self.outfit_to_female.put(outfit_name, i);
        } else {
            self.male.push(i);
            self.outfit_to_male.put(outfit_name, i);
        }
    }

    /// `registerOutfitter(id, useSeed, outfitter)`: for both sexes.
    pub fn register_outfitter(&mut self, id: &str, use_seed: bool, outfitter: Outfitter) {
        self.init_outfit(id, true, use_seed, outfitter);
        self.init_outfit(id, false, use_seed, outfitter);
    }

    /// `getOutfitNames()` (what `ZombiePopulationManager.init` hands the
    /// native library, lower-cased there).
    pub fn outfit_names(&self) -> &[String] {
        &self.outfit_names
    }

    /// The entry of an outfit index.
    pub fn data(&self, index: usize) -> Option<&OutfitData> {
        self.all.get(index)
    }

    /// `pickRandomFemale()`: `PZArrayUtil.pickRandom` (`Rand`) then
    /// [`Self::pick_outfit_female`].
    pub fn pick_random_female(&self) -> i32 {
        if self.female.is_empty() {
            return INVALID_ID;
        }
        let i = self.female[rand::next(self.female.len() as i32) as usize];
        let name = self.all[i].outfit_name.clone();
        self.pick_outfit_female(&name)
    }

    /// `pickRandomMale()`.
    pub fn pick_random_male(&self) -> i32 {
        if self.male.is_empty() {
            return INVALID_ID;
        }
        let i = self.male[rand::next(self.male.len() as i32) as usize];
        let name = self.all[i].outfit_name.clone();
        self.pick_outfit_male(&name)
    }

    fn pick(&self, map: &NameMap, outfit_name: &str, bit: i32) -> i32 {
        let Some(i) = map.get(outfit_name) else {
            return INVALID_ID;
        };
        let d = &self.all[i];
        let index = d.index as i16 as i32;
        let variant = if d.use_seed { rand::next(NUM_SEEDS as i32) } else { 0 };
        bit | (index << 16) | (variant + 1)
    }

    /// `pickOutfitFemale(outfitName)`: a random variant (`Rand`).
    pub fn pick_outfit_female(&self, outfit_name: &str) -> i32 {
        self.pick(&self.outfit_to_female, outfit_name, FEMALE_BIT)
    }

    /// `pickOutfitMale(outfitName)`.
    pub fn pick_outfit_male(&self, outfit_name: &str) -> i32 {
        self.pick(&self.outfit_to_male, outfit_name, 0)
    }

    /// `pickOutfit(outfitName, female)`: `advanced` is
    /// `ZombiesStageDefinitions.getAdvancedOutfitName(outfitName)` (which
    /// the caller runs first, with its `Rand` draw).
    pub fn pick_outfit(&self, outfit_name: &str, advanced: Option<&str>, female: bool) -> i32 {
        let name = advanced.unwrap_or(outfit_name);
        if female {
            self.pick_outfit_female(name)
        } else {
            self.pick_outfit_male(name)
        }
    }

    /// `getOutfit(id)`: the id with a fresh variant (`Rand`) if a seeded
    /// entry's variant is out of range; 0 for an unknown index.
    pub fn get_outfit(&self, id: i32) -> i32 {
        if id == INVALID_ID {
            return INVALID_ID;
        }
        let female_bit = id & FEMALE_BIT;
        let id = id & i32::MAX;
        let no_hat_bit = id & NO_HAT_BIT;
        let id = id & !NO_HAT_BIT;
        let index = (id >> 16) as i16;
        let mut variant = id as i16;
        if index < 0 || index as usize >= self.all.len() {
            return INVALID_ID;
        }
        let d = &self.all[index as usize];
        if d.use_seed && !(1..=NUM_SEEDS as i16).contains(&variant) {
            variant = (rand::next(NUM_SEEDS as i32) + 1) as i16;
        }
        female_bit | no_hat_bit | ((index as i32) << 16) | variant as i32
    }

    /// `dressInOutfit(chr, outfitID)` up to the outfitter: `getOutfit`, then
    /// `OutfitRNG.setSeed(seeds[variant - 1])` for a seeded entry. `None`
    /// when the id names no outfit (nothing is dressed).
    pub fn dress_in_outfit(&self, outfit_id: i32) -> Option<OutfitToApply> {
        let outfit_id = self.get_outfit(outfit_id);
        if outfit_id == INVALID_ID {
            return None;
        }
        let id2 = outfit_id & 0x7FFF7FFF;
        let index = (id2 >> 16) as i16;
        let variant = id2 as i16;
        let d = &self.all[index as usize];
        if d.use_seed {
            outfit_rng::set_seed(self.seeds[variant as usize - 1]);
        }
        Some(OutfitToApply { outfit_id, outfit_name: d.outfit_name.clone(), outfitter: d.outfitter })
    }

    /// The outfit an id dresses in, as `applyOutfit` reads it
    /// (`(short)((id & Integer.MAX_VALUE) >> 16)`).
    pub fn data_of_id(&self, outfit_id: i32) -> Option<&OutfitData> {
        let index = ((outfit_id & i32::MAX) >> 16) as i16;
        if index < 0 {
            return None;
        }
        self.all.get(index as usize)
    }

    /// `isHatFallen(outfitID)`.
    pub fn is_hat_fallen(outfit_id: i32) -> bool {
        outfit_id & NO_HAT_BIT != 0
    }

    /// `setFallenHat`'s new id (0 stays 0).
    pub fn with_fallen_hat(outfit_id: i32, fallen: bool) -> i32 {
        if outfit_id == INVALID_ID {
            return outfit_id;
        }
        if fallen {
            outfit_id | NO_HAT_BIT
        } else {
            outfit_id & !NO_HAT_BIT
        }
    }

    /// `save(ByteBuffer)`.
    pub fn save_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(10 + NUM_SEEDS * 8);
        out.extend_from_slice(&FILE_MAGIC);
        out.extend_from_slice(&FILE_VERSION_LATEST.to_be_bytes());
        out.extend_from_slice(&(NUM_SEEDS as i16).to_be_bytes());
        for s in &self.seeds {
            out.extend_from_slice(&s.to_be_bytes());
        }
        out
    }

    /// `load(ByteBuffer)`: `Err` is the exception it throws (seeds read
    /// before a buffer underflow stay set).
    pub fn load_bytes(&mut self, input: &[u8]) -> Result<(), String> {
        const UNDERFLOW: &str = "java.nio.BufferUnderflowException";
        let mut pos = 0usize;
        let mut take = |n: usize| -> Result<&[u8], String> {
            let b = input.get(pos..pos + n).ok_or_else(|| UNDERFLOW.to_string())?;
            pos += n;
            Ok(b)
        };
        if take(4)? != FILE_MAGIC {
            return Err("java.io.IOException: not magic".into());
        }
        let version = i32::from_be_bytes(take(4)?.try_into().unwrap());
        if version != FILE_VERSION_LATEST {
            return Ok(());
        }
        let n = i16::from_be_bytes(take(2)?.try_into().unwrap()) as i32;
        for i in 0..n.max(0) as usize {
            if i >= NUM_SEEDS {
                continue;
            }
            self.seeds[i] = i64::from_be_bytes(take(8)?.try_into().unwrap());
        }
        Ok(())
    }

    /// `load()`: a missing file is nothing; other failures are logged.
    pub fn load(&mut self, path: &Path) {
        match std::fs::read(path) {
            Ok(bytes) if bytes.is_empty() => {
                // `input.limit(-1)`.
                eprintln!("ERROR: java.lang.IllegalArgumentException: newLimit > capacity: (-1 > 0)");
            }
            Ok(bytes) => {
                if let Err(e) = self.load_bytes(&bytes) {
                    eprintln!("ERROR: {e}");
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => eprintln!("ERROR: java.io.IOException: {e}"),
        }
    }

    /// `save()` (the caller checks `Core.isNoSave()`).
    pub fn save(&self, path: &Path) {
        if let Err(e) = std::fs::write(path, self.save_bytes()) {
            eprintln!("ERROR: java.io.FileNotFoundException: {}: {e}", path.display());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_and_file() {
        let doc = b"<outfitManager><m_MaleOutfits><m_Name>Generic02</m_Name></m_MaleOutfits>\
            <m_MaleOutfits><m_Name>Bandit</m_Name></m_MaleOutfits>\
            <m_FemaleOutfits><m_Name>generic02</m_Name></m_FemaleOutfits></outfitManager>";
        let om = OutfitManager::from_xml(doc, "game").unwrap();
        let mut p = PersistentOutfits::default();
        rand::set_seed(5);
        p.init(&om, &["ZZD.Army.ArmyCamoGreen".to_string()], false, false, None);
        assert_eq!(p.outfit_names(), ["generic02", "Bandit", "ZZD.Army.ArmyCamoGreen"]);
        let id = p.pick_outfit_male("GENERIC02");
        assert_eq!(id >> 16, 0);
        assert!((1..=500).contains(&(id & 0xFFFF)));
        assert_eq!(p.pick_outfit_female("Bandit"), INVALID_ID);
        let f = p.pick_outfit_female("zzd.army.armycamogreen");
        assert_eq!((f & FEMALE_BIT, (f & i32::MAX) >> 16), (FEMALE_BIT, 2));
        assert!(p.get_outfit(1 << 16) & 0xFFFF != 0);
        assert_eq!(p.get_outfit(7 << 16), INVALID_ID);
        let hat = PersistentOutfits::with_fallen_hat(id, true);
        assert!(PersistentOutfits::is_hat_fallen(hat));
        assert_eq!(p.get_outfit(hat), hat);
        let a = p.dress_in_outfit(hat).unwrap();
        assert_eq!((a.outfit_name.as_str(), a.outfitter), ("generic02", Outfitter::Outfit));
        assert_eq!(outfit_rng::get_seed(), p.seeds[(id & 0xFFFF) as usize - 1].wrapping_add(-7046029254386353131i64.wrapping_mul(2)));
        let bytes = p.save_bytes();
        assert_eq!(bytes.len(), 10 + 4000);
        let mut q = PersistentOutfits::default();
        q.load_bytes(&bytes).unwrap();
        assert_eq!(q.seeds, p.seeds);
        assert_eq!(q.load_bytes(b"PSTZ\0\0\0\x01\0\x02\0\0\0\0\0\0\0\x09").unwrap_err(), "java.nio.BufferUnderflowException");
        assert_eq!(q.seeds[0], 9);
        assert_eq!(q.load_bytes(b"XXXX").unwrap_err(), "java.io.IOException: not magic");
    }
}
