//! `zombie.characters.SurvivorFactory`, `SurvivorDesc` and
//! `CharacterGender`.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use pz_core::random as rand;
use pz_scripts::objects::items::{CharacterProfession, CharacterProfessionDefinition};
use pz_inventory::io::{Reader, Writer};
use pz_scripts::registry::{RegistryKey, ResourceLocation};
use pz_scripts::{ScriptManager, ScriptResult};

use pz_inventory::visual::human::HumanVisualOwner;
use pz_inventory::visual::HumanVisual;

use crate::character::items::blood_part;
use crate::character::save::io_error;
use crate::color::{Color, ImmutableColor};
use crate::observation::ObservationRef;
use crate::worn_items::{ItemIdentity, WornItems};
use crate::{index_out_of_bounds, Characters};

/// `SurvivorFactory.SurvivorType`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SurvivorType {
    Friendly,
    Neutral,
    Aggressive,
}

impl SurvivorType {
    pub const NAMES: &'static [&'static str] = &["Friendly", "Neutral", "Aggressive"];
    pub const VALUES: [SurvivorType; 3] = [SurvivorType::Friendly, SurvivorType::Neutral, SurvivorType::Aggressive];

    pub fn name(self) -> &'static str {
        Self::NAMES[self as usize]
    }
}

/// `zombie.characters.CharacterGender`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CharacterGender {
    Male,
    Female,
}

impl CharacterGender {
    /// `getGenderStr` (also `toString`).
    pub fn gender_str(self) -> &'static str {
        match self {
            CharacterGender::Male => "Male",
            CharacterGender::Female => "Female",
        }
    }

    /// `getDisplayStringKey`.
    pub fn display_string_key(self) -> &'static str {
        match self {
            CharacterGender::Male => "IGUI_char_Male",
            CharacterGender::Female => "IGUI_char_Female",
        }
    }

    /// `fromString`: case-insensitive on the gender string.
    pub fn from_string(s: &str) -> Option<Self> {
        [CharacterGender::Male, CharacterGender::Female]
            .into_iter()
            .find(|g| pz_scripts::java::eq_ignore_case(g.gender_str(), s))
    }

    pub fn name(self) -> &'static str {
        match self {
            CharacterGender::Male => "MALE",
            CharacterGender::Female => "FEMALE",
        }
    }
}

/// `SurvivorFactory`'s name lists (translated when added).
#[derive(Clone, Debug, Default)]
pub struct SurvivorFactory {
    pub female_forenames: Vec<String>,
    pub male_forenames: Vec<String>,
    pub surnames: Vec<String>,
}

/// `SurvivorDesc`'s static fields.
#[derive(Clone, Debug, Default)]
pub struct SurvivorDescStatics {
    pub id_count: i32,
    pub trouser_common_colors: Vec<Color>,
    pub hair_common_colors: Vec<ImmutableColor>,
}

/// `Rand.Next(list.size())` then `list.get`, as the name pickers do.
fn random_entry(list: &[String]) -> ScriptResult<String> {
    let i = rand::next(list.len() as i32);
    list.get(i as usize).cloned().ok_or_else(|| index_out_of_bounds(i as i64, list.len()))
}

impl Characters {
    /// `Translator.getText(key)` through the installed
    /// [`Characters::translator`]; without one the key is returned, as for
    /// a missing translation.
    pub fn get_text(&self, key: &str) -> String {
        match &self.translator {
            Some(t) => t(key),
            None => Self::translate(key),
        }
    }

    /// [`Characters::get_text`] without a borrow of the statics.
    pub fn translate(key: &str) -> String {
        key.to_string()
    }

    /// `SurvivorFactory.Reset`.
    pub fn survivor_factory_reset(&mut self) {
        self.survivor_factory = SurvivorFactory::default();
        self.survivor_desc.hair_common_colors.clear();
        self.survivor_desc.trouser_common_colors.clear();
    }

    /// `SurvivorFactory.addSurname`.
    pub fn add_surname(&mut self, name: &str) {
        let t = self.get_text(&format!("SurvivorSurname_{name}"));
        self.survivor_factory.surnames.push(t);
    }

    /// `SurvivorFactory.addFemaleForename`.
    pub fn add_female_forename(&mut self, name: &str) {
        let t = self.get_text(&format!("SurvivorName_{name}"));
        self.survivor_factory.female_forenames.push(t);
    }

    /// `SurvivorFactory.addMaleForename`.
    pub fn add_male_forename(&mut self, name: &str) {
        let t = self.get_text(&format!("SurvivorName_{name}"));
        self.survivor_factory.male_forenames.push(t);
    }

    /// `SurvivorFactory.getRandomSurname`.
    pub fn random_surname(&self) -> ScriptResult<String> {
        random_entry(&self.survivor_factory.surnames)
    }

    /// `SurvivorFactory.getRandomForename`.
    pub fn random_forename(&self, female: bool) -> ScriptResult<String> {
        let f = &self.survivor_factory;
        random_entry(if female { &f.female_forenames } else { &f.male_forenames })
    }

    /// `SurvivorFactory.randomName`: forename then surname.
    pub fn random_name<I>(&self, desc: &mut SurvivorDesc<I>) -> ScriptResult {
        desc.forename = Some(self.random_forename(desc.is_female())?);
        desc.surname = Some(self.random_surname()?);
        Ok(())
    }

    /// `SurvivorFactory.CreateSurvivor()`: a random type.
    pub fn create_survivor<I: ItemIdentity>(&mut self, sm: &ScriptManager) -> ScriptResult<SurvivorDesc<I>> {
        let t = SurvivorType::VALUES[rand::next(3) as usize];
        self.create_survivor_of_type(sm, t)
    }

    /// `SurvivorFactory.CreateSurvivor(type)`: `Rand.Next(2) == 0` is female.
    pub fn create_survivor_of_type<I: ItemIdentity>(
        &mut self,
        sm: &ScriptManager,
        survivor_type: SurvivorType,
    ) -> ScriptResult<SurvivorDesc<I>> {
        let female = rand::next(2) == 0;
        self.create_survivor_with(sm, survivor_type, female)
    }

    /// `SurvivorFactory.CreateSurvivor(type, bFemale)`.
    /// TODO(IsoGameCharacter): `IsoGameCharacter.getSurvivorMap().put`.
    pub fn create_survivor_with<I: ItemIdentity>(
        &mut self,
        sm: &ScriptManager,
        survivor_type: SurvivorType,
        female: bool,
    ) -> ScriptResult<SurvivorDesc<I>> {
        let mut desc = SurvivorDesc::new(self, sm)?;
        desc.survivor_type = survivor_type;
        desc.set_female(female);
        self.random_name(&mut desc)?;
        set_torso(&mut desc);
        Ok(desc)
    }

    /// `SurvivorFactory.CreateFamily`: everyone takes the first one's surname.
    pub fn create_family<I: ItemIdentity>(&mut self, sm: &ScriptManager, count: i32) -> ScriptResult<Vec<SurvivorDesc<I>>> {
        let mut out: Vec<SurvivorDesc<I>> = Vec::new();
        for n in 0..count.max(0) {
            let mut d = self.create_survivor(sm)?;
            if n > 0 {
                d.surname = out[0].surname.clone();
            }
            out.push(d);
        }
        Ok(out)
    }

    /// `SurvivorFactory.CreateSurvivorGroup`.
    pub fn create_survivor_group<I: ItemIdentity>(&mut self, sm: &ScriptManager, count: i32) -> ScriptResult<Vec<SurvivorDesc<I>>> {
        (0..count.max(0)).map(|_| self.create_survivor(sm)).collect()
    }

    /// `SurvivorDesc.addTrouserColor(ColorInfo)`.
    pub fn add_trouser_color(&mut self, r: f32, g: f32, b: f32, a: f32) {
        self.survivor_desc.trouser_common_colors.push(Color::new(r, g, b, a));
    }

    /// `SurvivorDesc.addHairColor(ColorInfo)`.
    pub fn add_hair_color(&mut self, r: f32, g: f32, b: f32, a: f32) {
        self.survivor_desc.hair_common_colors.push(ImmutableColor::new(r, g, b, a));
    }
}

/// `SurvivorDesc implements IHumanVisual`: what its `HumanVisual` asks.
impl<I> HumanVisualOwner for SurvivorDesc<I> {
    fn is_female(&self) -> bool {
        SurvivorDesc::is_female(self)
    }
    fn is_zombie(&self) -> bool {
        false
    }
    fn is_skeleton(&self) -> bool {
        false
    }
}

/// `SurvivorFactory.setTorso`.
pub fn set_torso<I>(desc: &mut SurvivorDesc<I>) {
    desc.torso = Some(if desc.is_female() { "Kate" } else { "Male" }.to_string());
}

/// `zombie.characters.SurvivorDesc`.
///
/// TODO(IsoWorld): registration in `IsoWorld.instance.survivorDescriptors`;
/// TODO(IsoGameCharacter): `instance`; `SurvivorGroup` has no state the
/// original uses (`getLeader` is always `null`).
#[derive(Clone, Debug)]
pub struct SurvivorDesc<I> {
    pub human_visual: HumanVisual,
    pub worn_items: WornItems<I>,
    /// `xpBoostMap` (perk id -> level; perks are singletons, one per id).
    pub xp_boost_map: Vec<(String, i32)>,
    pub character_profession: Option<Rc<RegistryKey>>,
    pub forename: Option<String>,
    pub id: i32,
    pub character_gender: CharacterGender,
    pub surname: Option<String>,
    pub inventory_script: Option<String>,
    pub torso: Option<String>,
    pub met_count: HashMap<i32, i32>,
    pub bravery: f32,
    pub loner: f32,
    pub aggressiveness: f32,
    pub compassion: f32,
    pub temper: f32,
    pub friendliness: f32,
    pub favourindoors: f32,
    pub loyalty: f32,
    pub extra: Vec<String>,
    pub observations: Vec<ObservationRef>,
    pub survivor_type: SurvivorType,
    pub voice_prefix: Option<String>,
    pub voice_pitch: f32,
    pub voice_type: i32,
    pub dead: bool,
}

pub type SurvivorDescRef<I> = Rc<RefCell<SurvivorDesc<I>>>;

impl<I: ItemIdentity> SurvivorDesc<I> {
    /// The field initializers.
    fn fields(chars: &mut Characters, sm: &ScriptManager) -> ScriptResult<Self> {
        let group = chars.body_locations.get_group("Human")?;
        Ok(SurvivorDesc {
            human_visual: HumanVisual::new(blood_part::MAX),
            worn_items: WornItems::new(group),
            xp_boost_map: Vec::new(),
            character_profession: CharacterProfession::constant(sm, "UNEMPLOYED"),
            forename: Some("None".to_string()),
            id: 0,
            character_gender: CharacterGender::Female,
            surname: Some("None".to_string()),
            inventory_script: None,
            torso: Some("Base_Torso".to_string()),
            met_count: HashMap::new(),
            bravery: 1.0,
            loner: 0.0,
            aggressiveness: 1.0,
            compassion: 1.0,
            temper: 0.0,
            friendliness: 0.0,
            favourindoors: 0.0,
            loyalty: 0.0,
            extra: Vec::new(),
            observations: Vec::new(),
            survivor_type: SurvivorType::Neutral,
            voice_prefix: Some("VoiceFemale".to_string()),
            voice_pitch: 0.0,
            voice_type: 0,
            dead: false,
        })
    }

    /// `new SurvivorDesc()`: the next id, random stats.
    pub fn new(chars: &mut Characters, sm: &ScriptManager) -> ScriptResult<Self> {
        Self::new_unregistered(chars, sm)
    }

    /// `new SurvivorDesc(boolean)`: as [`SurvivorDesc::new`] without the
    /// world registration.
    pub fn new_unregistered(chars: &mut Characters, sm: &ScriptManager) -> ScriptResult<Self> {
        let mut d = Self::fields(chars, sm)?;
        d.id = chars.survivor_desc.id_count;
        chars.survivor_desc.id_count = chars.survivor_desc.id_count.wrapping_add(1);
        d.do_stats();
        Ok(d)
    }

    /// `new SurvivorDesc(SurvivorDesc other)`: copies the identity, stats and
    /// voice; visuals, worn items, observations and boosts start fresh and
    /// the id stays 0.
    pub fn copy_of(chars: &mut Characters, sm: &ScriptManager, other: &SurvivorDesc<I>) -> ScriptResult<Self> {
        let mut d = Self::fields(chars, sm)?;
        d.aggressiveness = other.aggressiveness;
        d.dead = other.dead;
        d.character_gender = other.character_gender;
        d.bravery = other.bravery;
        d.compassion = other.compassion;
        d.extra.extend(other.extra.iter().cloned());
        d.favourindoors = other.favourindoors;
        d.forename = other.forename.clone();
        d.friendliness = other.friendliness;
        d.inventory_script = other.inventory_script.clone();
        d.loner = other.loner;
        d.loyalty = other.loyalty;
        d.character_profession = other.character_profession.clone();
        d.surname = other.surname.clone();
        d.temper = other.temper;
        d.torso = other.torso.clone();
        d.survivor_type = other.survivor_type;
        d.voice_pitch = other.voice_pitch;
        d.voice_type = other.voice_type;
        d.voice_prefix = other.voice_prefix.clone();
        Ok(d)
    }
}

impl<I> SurvivorDesc<I> {
    /// `load(input, worldVersion, chr)`: the identity (the id raises
    /// `idCount`), `doStats`, the extras, the XP boosts (unknown perks are
    /// dropped) and the voice. TODO(IsoWorld): the
    /// `IsoWorld.survivorDescriptors` registration; `instance` is the
    /// caller's.
    pub fn load(&mut self, sm: &ScriptManager, chars: &mut Characters, r: &mut Reader<'_>, world_version: i32) -> ScriptResult {
        self.id = r.i32().map_err(io_error)?;
        self.forename = Some(r.string().map_err(io_error)?);
        self.surname = Some(r.string().map_err(io_error)?);
        self.torso = Some(r.string().map_err(io_error)?);
        self.character_gender = if r.i32().map_err(io_error)? == 1 { CharacterGender::Female } else { CharacterGender::Male };
        let profession = r.string().map_err(io_error)?;
        self.character_profession = CharacterProfession::get(sm, &ResourceLocation::of(&profession)?);
        self.do_stats();
        if chars.survivor_desc.id_count < self.id {
            chars.survivor_desc.id_count = self.id;
        }
        self.extra.clear();
        if r.i32().map_err(io_error)? == 1 {
            let n = r.i32().map_err(io_error)?;
            for _ in 0..n {
                self.extra.push(r.string().map_err(io_error)?);
            }
        }
        let n = r.i32().map_err(io_error)?;
        for _ in 0..n {
            let name = r.string().map_err(io_error)?;
            let level = r.i32().map_err(io_error)?;
            // `loadPerk`: `Perks.FromString`, `null` for `Perks.MAX`.
            let perk = chars.perks.from_string(&name);
            let max = chars.perks.max();
            let Some(perk) = perk.filter(|p| !max.as_ref().is_some_and(|m| Rc::ptr_eq(m, p))) else { continue };
            let id = perk.borrow().id().to_string();
            match self.xp_boost_map.iter_mut().find(|(k, _)| *k == id) {
                Some(e) => e.1 = level,
                None => self.xp_boost_map.push((id, level)),
            }
        }
        if world_version >= 208 {
            self.voice_prefix = Some(r.string().map_err(io_error)?);
            self.voice_pitch = r.f32().map_err(io_error)?;
            self.voice_type = r.i32().map_err(io_error)?;
        } else {
            self.voice_prefix = Some(format!("Voice{}", self.character_gender.gender_str()));
            self.voice_pitch = 0.0;
            self.voice_type = rand::next(3);
        }
        Ok(())
    }

    /// `save` (the XP boosts in insertion order; the original's map is
    /// identity-hashed).
    pub fn save(&self, w: &mut Writer) -> ScriptResult {
        w.i32(self.id);
        w.opt_string(self.forename.as_deref());
        w.opt_string(self.surname.as_deref());
        w.opt_string(self.torso.as_deref());
        w.i32((self.character_gender == CharacterGender::Female) as i32);
        let profession = self.character_profession.as_ref().ok_or_else(crate::npe)?;
        w.string(&profession.location.id());
        if self.extra.is_empty() {
            w.i32(0);
        } else {
            w.i32(1);
            w.i32(self.extra.len() as i32);
            for e in &self.extra {
                w.string(e);
            }
        }
        w.i32(self.xp_boost_map.len() as i32);
        for (perk, level) in &self.xp_boost_map {
            w.string(perk);
            w.i32(*level);
        }
        w.opt_string(self.voice_prefix.as_deref());
        w.f32(self.voice_pitch);
        w.i32(self.voice_type);
        Ok(())
    }

    /// `doStats`.
    fn do_stats(&mut self) {
        let coin = || if rand::next(2) == 0 { 10.0 } else { 0.0 };
        self.bravery = coin();
        self.aggressiveness = coin();
        self.compassion = 10.0 - self.aggressiveness;
        self.loner = coin();
        self.temper = coin();
        self.friendliness = 10.0 - self.loner;
        self.favourindoors = coin();
        self.loyalty = coin();
    }

    /// `isFemale`.
    pub fn is_female(&self) -> bool {
        self.character_gender == CharacterGender::Female
    }

    /// `setFemale`.
    pub fn set_female(&mut self, female: bool) {
        self.character_gender = if female { CharacterGender::Female } else { CharacterGender::Male };
    }

    /// `getFullname`.
    pub fn full_name(&self) -> String {
        format!("{} {}", self.forename.as_deref().unwrap_or("null"), self.surname.as_deref().unwrap_or("null"))
    }

    /// `getMetCount(descriptor)`.
    pub fn met_count_of(&self, id: i32) -> i32 {
        self.met_count.get(&id).copied().unwrap_or(0)
    }

    /// `hasObservation`.
    pub fn has_observation(&self, o: &str) -> bool {
        self.observations.iter().any(|x| x.borrow().trait_id.as_deref() == Some(o))
    }

    /// `addObservation`: unknown observations are ignored.
    pub fn add_observation(&mut self, chars: &Characters, obv: &str) {
        if let Some(o) = chars.observations.get_observation(obv) {
            self.observations.push(o);
        }
    }

    /// `isAggressive`.
    pub fn is_aggressive(&self) -> bool {
        self.has_observation("Aggressive")
    }

    /// `isFriendly`.
    pub fn is_friendly(&self) -> bool {
        self.has_observation("Friendly")
    }

    /// `isCharacterProfession` (identity of the key).
    pub fn is_character_profession(&self, p: Option<&Rc<RegistryKey>>) -> bool {
        match (&self.character_profession, p) {
            (Some(a), Some(b)) => a.location == b.location,
            (None, None) => true,
            _ => false,
        }
    }

    /// `setProfessionSkills`: the profession's XP boosts replace the map.
    pub fn set_profession_skills(&mut self, def: &CharacterProfessionDefinition) {
        self.xp_boost_map = def.xp_boosts.clone();
    }

    /// `getDescription(newStr)`.
    pub fn description(&self, new_str: &str) -> String {
        let mut s = format!("SurvivorDesc [{new_str}");
        s += &format!("ID={} | {new_str}", self.id);
        s += &format!("forename={} | {new_str}", self.forename.as_deref().unwrap_or("null"));
        s += &format!("surname={} | {new_str}", self.surname.as_deref().unwrap_or("null"));
        s += &format!("torso={} | {new_str}", self.torso.as_deref().unwrap_or("null"));
        s += &format!("bFemale={} | {new_str}", self.is_female());
        let prof = self.character_profession.as_ref().map_or("null".to_string(), |p| p.location.id());
        s += &format!("Profession={prof} | {new_str}");
        if !self.extra.is_empty() {
            s += "extra=";
            for e in &self.extra {
                s += &format!("{e},");
            }
            s += &format!(" | {new_str}");
        }
        if !self.xp_boost_map.is_empty() {
            s += &format!("XPBoost={new_str}");
            for (perk, level) in &self.xp_boost_map {
                // TODO(PerkFactory): `getName()` is the translated perk name.
                s += &format!("{perk}({perk}):{level}, {new_str}");
            }
            s += " ] ";
        }
        s += &format!("voicePrefix={} | {new_str}", self.voice_prefix.as_deref().unwrap_or("null"));
        s += &format!("voicePitch={} | {new_str}", java_float_string(self.voice_pitch));
        s += &format!("voiceType={} | {new_str}", self.voice_type);
        s
    }
}

/// `SurvivorDesc.meet`: both met counts go up (twice for meeting oneself).
pub fn meet<I>(a: &SurvivorDescRef<I>, b: &SurvivorDescRef<I>) {
    let (ia, ib) = (a.borrow().id, b.borrow().id);
    *a.borrow_mut().met_count.entry(ib).or_insert(0) += 1;
    *b.borrow_mut().met_count.entry(ia).or_insert(0) += 1;
}

/// `Float.toString`.
pub fn java_float_string(f: f32) -> String {
    if f.is_nan() {
        return "NaN".into();
    }
    if f.is_infinite() {
        return if f > 0.0 { "Infinity" } else { "-Infinity" }.into();
    }
    let a = f.abs();
    if a == 0.0 || (1e-3..1e7).contains(&a) {
        let s = format!("{f}");
        return if s.contains('.') { s } else { format!("{s}.0") };
    }
    let s = format!("{f:e}");
    let (m, e) = s.split_once('e').expect("exponent");
    let m = if m.contains('.') { m.to_string() } else { format!("{m}.0") };
    format!("{m}E{e}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_and_name() {
        let sm = ScriptManager::new();
        let mut chars = Characters::default();
        assert!(chars.create_survivor::<()>(&sm).is_err());
        let ids = chars.survivor_desc.id_count;
        chars.add_female_forename("Ann");
        chars.add_male_forename("Bob");
        chars.add_surname("Cole");
        let d = chars.create_survivor_with::<()>(&sm, SurvivorType::Aggressive, false).unwrap();
        assert_eq!(d.id, ids);
        assert_eq!(d.full_name(), "SurvivorName_Bob SurvivorSurname_Cole");
        assert_eq!(d.torso.as_deref(), Some("Male"));
        assert_eq!(d.survivor_type, SurvivorType::Aggressive);
        assert!(d.character_profession.is_none());
        assert!(d.bravery == 0.0 || d.bravery == 10.0);
        assert_eq!(d.compassion, 10.0 - d.aggressiveness);
        let fam = chars.create_family::<()>(&sm, 3).unwrap();
        assert!(fam.iter().all(|f| f.surname == fam[0].surname));
        let copy = SurvivorDesc::copy_of(&mut chars, &sm, &d).unwrap();
        assert_eq!((copy.id, copy.forename.clone()), (0, d.forename.clone()));
        chars.survivor_factory_reset();
        assert!(chars.random_surname().is_err());
    }

    #[test]
    fn observations_and_meet() {
        let sm = ScriptManager::new();
        let mut chars = Characters::default();
        chars.observations.add_observation("Friendly", Some("F"), None);
        let a = Rc::new(RefCell::new(SurvivorDesc::<()>::new(&mut chars, &sm).unwrap()));
        let b = Rc::new(RefCell::new(SurvivorDesc::<()>::new(&mut chars, &sm).unwrap()));
        a.borrow_mut().add_observation(&chars, "Friendly");
        a.borrow_mut().add_observation(&chars, "Unknown");
        assert!(a.borrow().is_friendly() && !a.borrow().is_aggressive());
        meet(&a, &b);
        meet(&a, &a);
        let (ia, ib) = (a.borrow().id, b.borrow().id);
        assert_eq!(a.borrow().met_count_of(ib), 1);
        assert_eq!(a.borrow().met_count_of(ia), 2);
        assert!(a.borrow().description("\n").contains("voicePitch=0.0 | "));
    }

    #[test]
    fn float_strings() {
        assert_eq!(java_float_string(0.0), "0.0");
        assert_eq!(java_float_string(1.5), "1.5");
        assert_eq!(java_float_string(1e7), "1.0E7");
        assert_eq!(java_float_string(0.0001), "1.0E-4");
        assert_eq!(CharacterGender::from_string("male"), Some(CharacterGender::Male));
    }
}
