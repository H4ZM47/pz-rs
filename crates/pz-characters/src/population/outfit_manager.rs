//! `OutfitManager` (`media/clothing/clothing.xml` with mods) and the
//! outfit draws `IsoGameCharacter`/`HumanVisual`/`IsoMannequin` make:
//! `GetRandomOutfit`, `GetRandomNonSillyOutfit`,
//! `GetRandomNonProfessionalOutfit`, `dressInNamedOutfit`,
//! `dressInRandomOutfit`, `dressInClothingItem`.
//!
//! Outfit lists are shuffled once after loading (`Collections.shuffle`
//! with a fresh `java.util.Random`), and the random outfit picks use
//! `Rand` (unseeded in the original), so only the order of draws within
//! one outfit (`OutfitRNG`) is reproducible.
//!
//! Not ported: clothing item loading and file watching (`pz-inventory`
//! owns the assets), `ZombiesZoneDefinition` and `PersistentOutfits`
//! (zombie outfits by zone). TODO(ZombiesZoneDefinition, PersistentOutfits).

use super::outfit::{dress_in_outfit, ClothingItemReference, DressItem, Outfit, OutfitContext};
use super::xml::parse_root;
use super::{base_file, parse_file};
use pz_core::random::{self, JavaRandom};
use pz_fs::java::{case_insensitive_cmp, is_null_or_whitespace};
use pz_fs::jfile::SEP;
use pz_fs::GameFs;
use std::cmp::Ordering;
use std::path::Path;
use std::rc::Rc;
use std::time::{SystemTime, UNIX_EPOCH};

pub const CLOTHING_FILE: &str = "media/clothing/clothing.xml";

/// A `TreeMap<String, Outfit>(String.CASE_INSENSITIVE_ORDER)`.
#[derive(Clone, Debug, Default)]
struct OutfitMap(Vec<(String, Rc<Outfit>)>);

impl OutfitMap {
    fn get(&self, name: &str) -> Option<&Rc<Outfit>> {
        self.0.iter().find(|(k, _)| case_insensitive_cmp(k, name) == Ordering::Equal).map(|(_, v)| v)
    }

    /// `put`: an equal key keeps its spelling and takes the new value.
    fn put(&mut self, name: &str, outfit: Rc<Outfit>) {
        match self.0.iter_mut().find(|(k, _)| case_insensitive_cmp(k, name) == Ordering::Equal) {
            Some(e) => e.1 = outfit,
            None => self.0.push((name.to_string(), outfit)),
        }
    }
}

/// `OutfitManager`.
#[derive(Clone, Debug, Default)]
pub struct OutfitManager {
    pub male_outfits: Vec<Rc<Outfit>>,
    pub female_outfits: Vec<Rc<Outfit>>,
    male_map: OutfitMap,
    female_map: OutfitMap,
}

impl OutfitManager {
    /// Unmarshal an `<outfitManager>` document, with `setModID(mod_id)`
    /// applied and the name maps filled (`parse`).
    pub fn from_xml(bytes: &[u8], mod_id: &str) -> Result<Self, String> {
        Self::from_xml_inner(bytes, mod_id, false)
    }

    /// `immutable`: mark the outfits and their items immutable, as
    /// `loaded()` does for every outfit it keeps.
    fn from_xml_inner(bytes: &[u8], mod_id: &str, immutable: bool) -> Result<Self, String> {
        let root = parse_root(bytes, "outfitManager")?;
        let mut m = OutfitManager::default();
        for (tag, female) in [("m_MaleOutfits", false), ("m_FemaleOutfits", true)] {
            for e in root.children_named(tag) {
                let mut o = Outfit::from_element(e);
                o.set_mod_id(mod_id);
                o.immutable = immutable;
                o.items.iter_mut().for_each(|r| r.immutable = immutable);
                let o = Rc::new(o);
                let (list, map) = if female { (&mut m.female_outfits, &mut m.female_map) } else { (&mut m.male_outfits, &mut m.male_map) };
                map.put(&o.name.clone(), o.clone());
                list.push(o);
            }
        }
        Ok(m)
    }

    /// `tryParse(modID, dir, "media/clothing/clothing.xml")` for a file
    /// path: `None` if it does not exist or fails to parse (logged).
    pub fn parse(path: &str, mod_id: &str) -> Option<Self> {
        Self::parse_inner(path, mod_id, false)
    }

    fn parse_inner(path: &str, mod_id: &str, immutable: bool) -> Option<Self> {
        if !Path::new(path).exists() {
            return None;
        }
        parse_file(path, |b| Self::from_xml_inner(b, mod_id, immutable))
    }

    /// `init()` with a clock-seeded shuffle, as the original.
    pub fn init(fs: &mut GameFs) -> Option<Self> {
        let seed = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_nanos() as i64);
        Self::init_with_shuffle(fs, &JavaRandom::new(seed))
    }

    /// `init()`: the game's file, each mod's (version folder, else
    /// `common/`), outfits immutable, then both lists shuffled by
    /// `shuffle` (`Collections.shuffle`).
    pub fn init_with_shuffle(fs: &mut GameFs, shuffle: &JavaRandom) -> Option<Self> {
        let mut inst = Self::parse_inner(&base_file(fs, CLOTHING_FILE), "game", true)?;
        let rel = CLOTHING_FILE.replace('/', &SEP.to_string());
        for id in fs.mod_ids().to_vec() {
            let Some(dir) = fs.available_mod_dir(&id) else { continue };
            let Some(info) = fs.mod_info(&dir) else { continue };
            let (version, common) = (info.version_dir.clone(), info.common_dir.clone());
            let Some(m) =
                Self::parse_inner(&format!("{version}{SEP}{rel}"), &id, true)
                    .or_else(|| Self::parse_inner(&format!("{common}{SEP}{rel}"), &id, true))
            else {
                continue;
            };
            inst.merge(m);
        }
        java_shuffle(&mut inst.male_outfits, shuffle);
        java_shuffle(&mut inst.female_outfits, shuffle);
        Some(inst)
    }

    /// The mod loop of `loaded()`.
    fn merge(&mut self, m: OutfitManager) {
        for (outfits, female) in [(m.male_outfits, false), (m.female_outfits, true)] {
            for o in outfits {
                let (list, map) =
                    if female { (&mut self.female_outfits, &mut self.female_map) } else { (&mut self.male_outfits, &mut self.male_map) };
                match map.get(&o.name).and_then(|old| list.iter().position(|x| Rc::ptr_eq(x, old))) {
                    Some(i) => list[i] = o.clone(),
                    None => list.push(o.clone()),
                }
                map.put(&o.name.clone(), o);
            }
        }
    }

    /// The outfits of one sex, in their shuffled order.
    pub fn get(&self, female: bool) -> &[Rc<Outfit>] {
        if female {
            &self.female_outfits
        } else {
            &self.male_outfits
        }
    }

    /// `FindMaleOutfit(name)`: by name ignoring case (the last loaded
    /// outfit of that name).
    pub fn find_male_outfit(&self, name: &str) -> Option<&Rc<Outfit>> {
        self.male_map.get(name)
    }

    /// `FindFemaleOutfit(name)`.
    pub fn find_female_outfit(&self, name: &str) -> Option<&Rc<Outfit>> {
        self.female_map.get(name)
    }

    /// `GetSpecificOutfit(female, name)`.
    pub fn get_specific_outfit(&self, female: bool, name: &str) -> Option<&Rc<Outfit>> {
        if female {
            self.find_female_outfit(name)
        } else {
            self.find_male_outfit(name)
        }
    }

    /// `GetRandomOutfit(female)` with `Rand` (`PZArrayUtil.pickRandom`).
    pub fn get_random_outfit(&self, female: bool) -> Option<&Rc<Outfit>> {
        self.get_random_outfit_with(female, &mut random::next)
    }

    /// [`Self::get_random_outfit`] drawing from `next` (`Rand.Next(max)`).
    pub fn get_random_outfit_with(&self, female: bool, next: &mut dyn FnMut(i32) -> i32) -> Option<&Rc<Outfit>> {
        let list = self.get(female);
        if list.is_empty() {
            return None;
        }
        list.get(next(list.len() as i32) as usize)
    }

    /// `GetRandomNonSillyOutfit(female)`: redraws while the name contains
    /// `Santa`, `Spiffo` or `Naked`. `None` when no outfit qualifies (the
    /// original loops forever, or throws on an empty list).
    pub fn get_random_non_silly_outfit(&self, female: bool) -> Option<&Rc<Outfit>> {
        self.get_random_non_silly_outfit_with(female, &mut random::next)
    }

    pub fn get_random_non_silly_outfit_with(&self, female: bool, next: &mut dyn FnMut(i32) -> i32) -> Option<&Rc<Outfit>> {
        let silly = |o: &Outfit| ["Santa", "Spiffo", "Naked"].iter().any(|s| o.name.contains(s));
        if self.get(female).iter().all(|o| silly(o)) {
            return None;
        }
        loop {
            let o = self.get_random_outfit_with(female, next)?;
            if !silly(o) {
                return Some(o);
            }
        }
    }

    /// `GetRandomNonProfessionalOutfit(female)` with `Rand`.
    pub fn get_random_non_professional_outfit(&self, female: bool) -> Option<&Rc<Outfit>> {
        self.get_random_non_professional_outfit_with(female, &mut random::next)
    }

    pub fn get_random_non_professional_outfit_with(
        &self,
        female: bool,
        next: &mut dyn FnMut(i32) -> i32,
    ) -> Option<&Rc<Outfit>> {
        let mut name = format!("Generic0{}", next(5) + 1);
        if next(4) == 0 {
            let r = next(3);
            let pick = if female { ["Mannequin1", "Mannequin2", "Classy"] } else { ["Classy", "Tourist", "MallSecurity"] };
            if let Some(p) = pick.get(r as usize) {
                name = (*p).to_string();
            }
        }
        self.get_specific_outfit(female, &name)
    }

    /// `HumanVisual.dressInNamedOutfit(name, itemVisuals)`: the named
    /// outfit cloned, randomized and dressed. `None` for a blank or
    /// unknown name (nothing dressed).
    pub fn dress_in_named_outfit(
        &self,
        female: bool,
        name: &str,
        ctx: &OutfitContext<'_>,
    ) -> Option<Result<(Outfit, Vec<DressItem>), String>> {
        if is_null_or_whitespace(name) {
            return None;
        }
        let source = self.get_specific_outfit(female, name)?;
        let mut outfit = source.clone_outfit();
        Some(outfit.randomize(ctx).map(|()| {
            let items = dress_in_outfit(&mut outfit, ctx);
            (outfit, items)
        }))
    }

    /// `IsoGameCharacter.dressInRandomOutfit()`: a random outfit's name,
    /// then [`Self::dress_in_named_outfit`].
    pub fn dress_in_random_outfit(
        &self,
        female: bool,
        ctx: &OutfitContext<'_>,
    ) -> Option<Result<(Outfit, Vec<DressItem>), String>> {
        let name = self.get_random_outfit(female)?.name.clone();
        self.dress_in_named_outfit(female, &name, ctx)
    }

    /// `IsoGameCharacter.dressInRandomNonSillyOutfit()`.
    pub fn dress_in_random_non_silly_outfit(
        &self,
        female: bool,
        ctx: &OutfitContext<'_>,
    ) -> Option<Result<(Outfit, Vec<DressItem>), String>> {
        let name = self.get_random_non_silly_outfit(female)?.name.clone();
        self.dress_in_named_outfit(female, &name, ctx)
    }
}

/// `HumanVisual.dressInClothingItem(guid, itemVisuals)`: a one-item outfit
/// without default pants/top. `None` if the guid names no clothing item.
pub fn dress_in_clothing_item(guid: &str, ctx: &OutfitContext<'_>) -> Option<Result<(Outfit, Vec<DressItem>), String>> {
    ctx.clothing_items.clothing_item(guid)?;
    let mut outfit = Outfit {
        items: vec![ClothingItemReference { item_guid: Some(guid.to_string()), ..Default::default() }],
        pants: false,
        top: false,
        ..Default::default()
    };
    Some(outfit.randomize(ctx).map(|()| {
        let items = dress_in_outfit(&mut outfit, ctx);
        (outfit, items)
    }))
}

/// `Collections.shuffle(list, rnd)`.
pub fn java_shuffle<T>(list: &mut [T], rnd: &JavaRandom) {
    for i in (2..=list.len()).rev() {
        list.swap(i - 1, rnd.next_int(i as i32) as usize);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DOC: &[u8] = b"<outfitManager><m_MaleOutfits><m_Name>Generic01</m_Name></m_MaleOutfits>\
        <m_MaleOutfits><m_Name>Santa</m_Name></m_MaleOutfits><m_MaleOutfits><m_Name>generic01</m_Name><m_Top>false</m_Top></m_MaleOutfits>\
        <m_FemaleOutfits><m_Name>Classy</m_Name></m_FemaleOutfits></outfitManager>";

    #[test]
    fn maps_and_draws() {
        let mut m = OutfitManager::from_xml(DOC, "game").unwrap();
        assert_eq!(m.male_outfits.len(), 3);
        let g = m.find_male_outfit("GENERIC01").unwrap();
        assert!(!g.top);
        assert!(m.find_female_outfit("Generic01").is_none());
        let mut seq = [0, 1, 1].into_iter();
        let o = m.get_random_non_silly_outfit_with(false, &mut |_| seq.next().unwrap()).unwrap();
        assert_eq!(o.name, "Generic01");
        let mut seq = [0, 0, 2].into_iter();
        assert_eq!(m.get_random_non_professional_outfit_with(true, &mut |_| seq.next().unwrap()).unwrap().name, "Classy");
        let modded = OutfitManager::from_xml(b"<outfitManager><m_MaleOutfits><m_Name>GENERIC01</m_Name></m_MaleOutfits></outfitManager>", "m").unwrap();
        m.merge(modded);
        assert_eq!(m.male_outfits.len(), 3);
        assert_eq!(m.male_outfits[2].name, "GENERIC01");
        assert_eq!(m.male_outfits[0].name, "Generic01");
        let mut v = vec![1, 2, 3, 4, 5];
        java_shuffle(&mut v, &JavaRandom::new(42));
        assert_eq!(v.len(), 5);
    }
}
