//! `zombie.characters.HairOutfitDefinitions`: per-outfit hair, beard and
//! hair colour chances and per-style outfit/world-age restrictions, read
//! from Lua's `HairOutfitDefinitions` table (the binding fills
//! [`HairOutfitDefinitions`] through [`HaircutDefinition::new`] and
//! [`HaircutOutfitDefinition::new`] on `checkDirty`).
//!
//! `IsoWorld.getWorldAgeDays()` and `SurvivorDesc.HairCommonColors` are
//! inputs.

use super::beard_styles::BeardStyle;
use super::hair_styles::HairStyle;
use pz_core::random::outfit_rng;
use pz_fs::java::{is_null_or_whitespace, split_char};
use pz_inventory::visual::Rgb;
use pz_scripts::java::{eq_ignore_case, parse_f32};

/// `HairOutfitDefinitions.StringChance`.
#[derive(Clone, Debug, PartialEq)]
pub struct StringChance {
    pub str: String,
    pub chance: f32,
}

/// `HaircutDefinition`: a style only some outfits get, after a world age.
#[derive(Clone, Debug, PartialEq)]
pub struct HaircutDefinition {
    pub hair_style: String,
    pub min_world_age: i32,
    pub only_for: Vec<String>,
}

impl HaircutDefinition {
    /// From the Lua entry: `onlyFor` is split on `,`.
    pub fn new(hair_style: String, min_world_age: i32, only_for: &str) -> Self {
        HaircutDefinition { hair_style, min_world_age, only_for: split_char(only_for, ',') }
    }
}

/// `HaircutOutfitDefinition`.
#[derive(Clone, Debug, PartialEq)]
pub struct HaircutOutfitDefinition {
    pub outfit: String,
    pub haircut_chance: Option<Vec<StringChance>>,
    pub female_haircut_chance: Option<Vec<StringChance>>,
    pub male_haircut_chance: Option<Vec<StringChance>>,
    pub beard_chance: Option<Vec<StringChance>>,
    pub haircut_color: Option<Vec<StringChance>>,
}

impl HaircutOutfitDefinition {
    /// From the Lua entry's strings (`haircut`, `femaleHaircut`,
    /// `maleHaircut`, `beard`, `haircutColor`), each through
    /// [`init_string_chance`]. `Err` is the exception the original throws.
    pub fn new(
        outfit: String,
        haircut: Option<&str>,
        female_haircut: Option<&str>,
        male_haircut: Option<&str>,
        beard: Option<&str>,
        haircut_color: Option<&str>,
    ) -> Result<Self, String> {
        Ok(HaircutOutfitDefinition {
            outfit,
            haircut_chance: init_string_chance(haircut)?,
            female_haircut_chance: init_string_chance(female_haircut)?,
            male_haircut_chance: init_string_chance(male_haircut)?,
            beard_chance: init_string_chance(beard)?,
            haircut_color: init_string_chance(haircut_color)?,
        })
    }
}

/// `initStringChance(styles)`: `"a:60;b:10"`; a total under 100 adds
/// `random` with the rest. `None` for a blank string.
pub fn init_string_chance(styles: Option<&str>) -> Result<Option<Vec<StringChance>>, String> {
    let Some(styles) = styles.filter(|s| !is_null_or_whitespace(s)) else { return Ok(None) };
    let mut result = Vec::new();
    let mut total = 0i32;
    for style in split_char(styles, ';') {
        let parts = split_char(&style, ':');
        let str = parts.first().cloned().ok_or("java.lang.ArrayIndexOutOfBoundsException: Index 0 out of bounds for length 0")?;
        let chance = parts.get(1).ok_or("java.lang.ArrayIndexOutOfBoundsException: Index 1 out of bounds for length 1")?;
        let chance =
            parse_f32(chance).ok_or_else(|| format!("java.lang.NumberFormatException: For input string: \"{chance}\""))?;
        total = (total as f32 + chance) as i32;
        result.push(StringChance { str, chance });
    }
    if total < 100 {
        result.push(StringChance { str: "random".into(), chance: (100 - total) as f32 });
    }
    Ok(Some(result))
}

/// The entry of `chances` a `[0, 100)` roll lands in.
fn roll(chances: &[StringChance]) -> Option<&StringChance> {
    let choice = outfit_rng::next_float(0.0, 100.0);
    let mut subtotal = 0.0f32;
    chances.iter().find(|c| {
        subtotal += c.chance;
        choice < subtotal
    })
}

/// `HairOutfitDefinitions`.
#[derive(Clone, Debug, PartialEq)]
pub struct HairOutfitDefinitions {
    /// Set when Lua's table may have changed; the binding reloads on use.
    pub dirty: bool,
    pub haircut_definition: Vec<HaircutDefinition>,
    pub outfit_definition: Vec<HaircutOutfitDefinition>,
}

impl Default for HairOutfitDefinitions {
    fn default() -> Self {
        HairOutfitDefinitions { dirty: true, haircut_definition: Vec::new(), outfit_definition: Vec::new() }
    }
}

impl HairOutfitDefinitions {
    /// `isHaircutValid(outfit, haircut)`.
    pub fn is_haircut_valid(&self, outfit: Option<&str>, haircut: &str, world_age_days: f32) -> bool {
        let Some(outfit) = outfit.filter(|o| !o.is_empty()) else { return true };
        for def in &self.haircut_definition {
            if def.hair_style != haircut {
                continue;
            }
            if !def.only_for.iter().any(|o| o == outfit) {
                return false;
            }
            if world_age_days < def.min_world_age as f32 {
                return false;
            }
        }
        true
    }

    /// `getValidHairStylesForOutfit`.
    pub fn get_valid_hair_styles_for_outfit<'a>(
        &self,
        outfit: Option<&str>,
        styles: &'a [HairStyle],
        world_age_days: f32,
    ) -> Vec<&'a HairStyle> {
        styles.iter().filter(|s| !s.no_choose && self.is_haircut_valid(outfit, &s.name, world_age_days)).collect()
    }

    fn outfits<'a>(&'a self, outfit: Option<&'a str>) -> impl Iterator<Item = &'a HaircutOutfitDefinition> + 'a {
        self.outfit_definition.iter().filter(move |d| Some(d.outfit.as_str()) == outfit)
    }

    /// `getRandomHaircut(outfit, hairList)`.
    pub fn get_random_haircut(&self, outfit: Option<&str>, styles: &[HairStyle], world_age_days: f32) -> String {
        let valid = self.get_valid_hair_styles_for_outfit(outfit, styles, world_age_days);
        let Some(first) = outfit_rng::pick_random(&valid) else { return String::new() };
        let haircut = first.name.clone();
        match self.outfits(outfit).find(|d| d.haircut_chance.is_some()) {
            Some(def) => Self::haircut_from_chances(def.haircut_chance.as_deref().unwrap(), haircut, &valid),
            None => haircut,
        }
    }

    /// `getRandomHaircutFromOutfitDef` over one chance list.
    fn haircut_from_chances(chances: &[StringChance], haircut: String, valid: &[&HairStyle]) -> String {
        match roll(chances) {
            Some(c) => Self::chosen(&c.str, valid),
            None => haircut,
        }
    }

    /// A rolled entry: `null` is no hair, `random` another pick.
    fn chosen(s: &str, valid: &[&HairStyle]) -> String {
        if eq_ignore_case("null", s) {
            String::new()
        } else if eq_ignore_case("random", s) {
            outfit_rng::pick_random(valid).map(|h| h.name.clone()).unwrap_or_default()
        } else {
            s.to_string()
        }
    }

    /// `getRandomFemaleHaircut` / `getRandomMaleHaircut`: the first
    /// definition for the outfit with a sex-specific list rolls it; one
    /// without (but with `haircut`) rolls that instead.
    fn random_sexed_haircut(
        &self,
        outfit: Option<&str>,
        styles: &[HairStyle],
        world_age_days: f32,
        pick: fn(&HaircutOutfitDefinition) -> Option<&Vec<StringChance>>,
    ) -> String {
        let valid = self.get_valid_hair_styles_for_outfit(outfit, styles, world_age_days);
        let Some(first) = outfit_rng::pick_random(&valid) else { return String::new() };
        let mut haircut = first.name.clone();
        for def in self.outfits(outfit) {
            if let Some(chances) = pick(def) {
                if let Some(c) = roll(chances) {
                    haircut = Self::chosen(&c.str, &valid);
                    break;
                }
                continue;
            }
            if let Some(chances) = &def.haircut_chance {
                return Self::haircut_from_chances(chances, haircut, &valid);
            }
        }
        haircut
    }

    /// `getRandomFemaleHaircut(outfit, hairList)`.
    pub fn get_random_female_haircut(&self, outfit: Option<&str>, styles: &[HairStyle], world_age_days: f32) -> String {
        self.random_sexed_haircut(outfit, styles, world_age_days, |d| d.female_haircut_chance.as_ref())
    }

    /// `getRandomMaleHaircut(outfit, hairList)`.
    pub fn get_random_male_haircut(&self, outfit: Option<&str>, styles: &[HairStyle], world_age_days: f32) -> String {
        self.random_sexed_haircut(outfit, styles, world_age_days, |d| d.male_haircut_chance.as_ref())
    }

    /// `getRandomHaircutColor(outfit)`: a common hair colour, unless the
    /// outfit's colour roll gives `r,g,b`. `None` when there are no common
    /// colours (the original throws).
    pub fn get_random_haircut_color(&self, outfit: Option<&str>, common: &[Rgb]) -> Option<Rgb> {
        let mut result = *common.get(outfit_rng::next(common.len() as i32) as usize)?;
        let mut str_color: Option<String> = None;
        for def in self.outfits(outfit) {
            let Some(chances) = &def.haircut_color else { continue };
            if let Some(c) = roll(chances) {
                str_color = Some(c.str.clone());
                if eq_ignore_case("random", &c.str) {
                    result = *common.get(outfit_rng::next(common.len() as i32) as usize)?;
                    str_color = None;
                }
                break;
            }
        }
        if let Some(s) = str_color.filter(|s| !s.is_empty()) {
            let parts = split_char(&s, ',');
            let f = |i: usize| parts.get(i).and_then(|p| parse_f32(p)).unwrap_or(0.0).clamp(0.0, 1.0);
            result = [f(0), f(1), f(2)];
        }
        Some(result)
    }

    /// `getRandomBeard(outfit, beardList)`. `None` only for an empty list
    /// (the original throws).
    pub fn get_random_beard(&self, outfit: Option<&str>, beards: &[BeardStyle]) -> Option<String> {
        let mut beard = outfit_rng::pick_random(beards)?.name.clone();
        for def in self.outfits(outfit) {
            let Some(chances) = &def.beard_chance else { continue };
            if let Some(c) = roll(chances) {
                beard = if eq_ignore_case("null", &c.str) {
                    String::new()
                } else if eq_ignore_case("random", &c.str) {
                    outfit_rng::pick_random(beards)?.name.clone()
                } else {
                    c.str.clone()
                };
                break;
            }
        }
        Some(beard)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn string_chances() {
        let c = init_string_chance(Some("Mohawk:60;Bald:10")).unwrap().unwrap();
        assert_eq!(c.len(), 3);
        assert_eq!(c[2], StringChance { str: "random".into(), chance: 30.0 });
        assert!(init_string_chance(Some("  ")).unwrap().is_none());
        assert_eq!(init_string_chance(Some("a:100")).unwrap().unwrap().len(), 1);
        assert!(init_string_chance(Some("a")).is_err());
        assert!(init_string_chance(Some("a:x")).is_err());
    }

    #[test]
    fn validity() {
        let mut d = HairOutfitDefinitions::default();
        d.haircut_definition.push(HaircutDefinition::new("Mohawk".into(), 10, "Punk,Bandit"));
        assert!(d.is_haircut_valid(None, "Mohawk", 0.0));
        assert!(d.is_haircut_valid(Some(""), "Mohawk", 0.0));
        assert!(!d.is_haircut_valid(Some("Punk"), "Mohawk", 9.5));
        assert!(d.is_haircut_valid(Some("Bandit"), "Mohawk", 10.0));
        assert!(!d.is_haircut_valid(Some("Office"), "Mohawk", 99.0));
        assert!(d.is_haircut_valid(Some("Office"), "mohawk", 0.0));
    }
}
