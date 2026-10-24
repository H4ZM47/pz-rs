//! Local stand-ins for types the characters thread (pz-characters) owns:
//! `ClothingDecal`/`ClothingDecals`, `HairStyles`/`BeardStyles` and the
//! `PopTemplateManager` skin lists, plus a [`HumanStyles`] over them.
//!
//! Only [`crate::external`] names this module; everything else goes
//! through that seam, so swapping in the owner's types is one line each.
//! Not ported further here: `HairOutfitDefinitions` (random hair/beard
//! styles and haircut colours) and `OutfitRNG` draws, which answer with
//! the [`HumanStyles`] defaults.

use std::path::Path;

use pz_anim::xml::{self, Element};
use pz_inventory::visual::human::{HumanStyles, SkinList};

fn text(e: &Element) -> String {
    e.text_content()
}

fn trimmed(e: &Element) -> String {
    pz_anim::vars::java_trim(&e.text_content()).to_string()
}

/// `ClothingDecal`.
#[derive(Clone, Debug, PartialEq)]
pub struct ClothingDecal {
    pub name: String,
    pub texture: String,
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

impl ClothingDecal {
    pub fn load(dir: &Path, name: &str) -> Option<ClothingDecal> {
        let root = xml::parse_xml_file(&dir.join(format!("{name}.xml"))).ok()?;
        let mut d = ClothingDecal {
            name: name.into(),
            texture: String::new(),
            x: 0,
            y: 0,
            width: 0,
            height: 0,
        };
        for f in root.elements() {
            let i = || trimmed(f).parse().unwrap_or(0);
            match f.tag.as_str() {
                "texture" => d.texture = text(f),
                "x" => d.x = i(),
                "y" => d.y = i(),
                "width" => d.width = i(),
                "height" => d.height = i(),
                _ => {}
            }
        }
        Some(d)
    }

    /// `isValid`.
    pub fn is_valid(&self) -> bool {
        !self.texture.trim().is_empty()
            && self.x >= 0
            && self.y >= 0
            && self.width > 0
            && self.height > 0
    }
}

/// `HairStyle` / `BeardStyle`.
#[derive(Clone, Debug, PartialEq)]
pub struct HairStyle {
    pub name: String,
    pub model: Option<String>,
    pub texture: Option<String>,
    /// (category, style)
    pub alternate: Vec<(String, String)>,
    pub level: i32,
}

impl HairStyle {
    /// `isValid`.
    pub fn is_valid(&self) -> bool {
        self.model.as_deref().is_some_and(|m| !m.trim().is_empty())
            && self
                .texture
                .as_deref()
                .is_some_and(|t| !t.trim().is_empty())
    }

    /// `getAlternate(category)`.
    pub fn alternate_for(&self, category: &str) -> &str {
        self.alternate
            .iter()
            .find(|(c, _)| c.eq_ignore_ascii_case(category))
            .map(|(_, s)| s.as_str())
            .unwrap_or(&self.name)
    }

    fn from_element(e: &Element) -> HairStyle {
        let mut s = HairStyle {
            name: String::new(),
            model: None,
            texture: Some("F_Hair_White".into()),
            alternate: Vec::new(),
            level: 0,
        };
        for f in e.elements() {
            match f.tag.as_str() {
                "name" => s.name = text(f),
                "model" => s.model = Some(text(f)),
                "texture" => s.texture = Some(text(f)),
                "level" => s.level = trimmed(f).parse().unwrap_or(0),
                "alternate" => s.alternate.push((
                    f.attr_or_empty("category").to_string(),
                    f.attr_or_empty("style").to_string(),
                )),
                _ => {}
            }
        }
        s
    }
}

/// `HairStyles` and `BeardStyles`.
#[derive(Clone, Debug, Default)]
pub struct HairStyles {
    pub male: Vec<HairStyle>,
    pub female: Vec<HairStyle>,
    pub beards: Vec<HairStyle>,
}

impl HairStyles {
    /// `hairStyles.xml` and `beardStyles.xml` from `media/hairStyles`.
    pub fn load(dir: &Path) -> HairStyles {
        let mut h = HairStyles::default();
        if let Ok(root) = xml::parse_xml_file(&dir.join("hairStyles.xml")) {
            for e in root.elements() {
                match e.tag.as_str() {
                    "male" => h.male.push(HairStyle::from_element(e)),
                    "female" => h.female.push(HairStyle::from_element(e)),
                    _ => {}
                }
            }
        }
        if let Ok(root) = xml::parse_xml_file(&dir.join("beardStyles.xml")) {
            for e in root.elements().filter(|e| e.tag == "style") {
                h.beards.push(HairStyle::from_element(e));
            }
        }
        h
    }

    /// `FindStyle(list, name)`: a blank name also finds `bald`.
    fn find(list: &[HairStyle], name: &str) -> Option<usize> {
        for (i, s) in list.iter().enumerate() {
            if s.name.eq_ignore_ascii_case(name) {
                return Some(i);
            }
            if name.is_empty() && s.name.eq_ignore_ascii_case("bald") {
                return Some(i);
            }
        }
        None
    }

    pub fn find_male(&self, name: &str) -> Option<&HairStyle> {
        Self::find(&self.male, name).map(|i| &self.male[i])
    }

    pub fn find_female(&self, name: &str) -> Option<&HairStyle> {
        Self::find(&self.female, name).map(|i| &self.female[i])
    }

    pub fn find_beard(&self, name: &str) -> Option<&HairStyle> {
        self.beards
            .iter()
            .find(|s| s.name.eq_ignore_ascii_case(name))
    }

    /// `getAlternateForHat`.
    pub fn alternate_for_hat<'a>(
        &'a self,
        style: &'a HairStyle,
        category: Option<&str>,
        female: bool,
    ) -> Option<&'a HairStyle> {
        let category = category.unwrap_or("");
        if category.eq_ignore_ascii_case("nohair") || category.eq_ignore_ascii_case("nohairnobeard")
        {
            return None;
        }
        let alt = style.alternate_for(category);
        if female && self.female.iter().any(|s| s == style) {
            return self.find_female(alt);
        }
        if !female && self.male.iter().any(|s| s == style) {
            return self.find_male(alt);
        }
        Some(style)
    }
}

/// `PopTemplateManager`'s human skin lists (`init()`).
#[derive(Clone, Debug, PartialEq)]
pub struct PopTemplateSkins {
    pub male: Vec<String>,
    pub female: Vec<String>,
    /// `maleSkinsZombie1..3`.
    pub male_zombie: [Vec<String>; 3],
    pub female_zombie: [Vec<String>; 3],
    pub skeleton_male_zombie: Vec<String>,
    pub skeleton_female_zombie: Vec<String>,
}

impl Default for PopTemplateSkins {
    fn default() -> Self {
        let body = |prefix: &str| (1..=5).map(|i| format!("{prefix}0{i}")).collect();
        let zed = |sex: &str, level: i32| {
            (1..=4)
                .map(|i| format!("{sex}_ZedBody0{i}_level{level}"))
                .collect::<Vec<_>>()
        };
        let skeleton: Vec<String> = ["SkeletonBurned", "Skeleton", "SkeletonMuscle"]
            .map(String::from)
            .into();
        PopTemplateSkins {
            male: body("MaleBody"),
            female: body("FemaleBody"),
            male_zombie: [zed("M", 1), zed("M", 2), zed("M", 3)],
            female_zombie: [zed("F", 1), zed("F", 2), zed("F", 3)],
            skeleton_male_zombie: skeleton.clone(),
            skeleton_female_zombie: skeleton,
        }
    }
}

impl PopTemplateSkins {
    /// The list `HumanVisual.getSkinTexture` picks from.
    pub fn list(&self, list: SkinList) -> &[String] {
        let zed = |s: i32| (s.clamp(1, 3) - 1) as usize;
        match list {
            SkinList::Female => &self.female,
            SkinList::Male => &self.male,
            SkinList::SkeletonFemaleZombie => &self.skeleton_female_zombie,
            SkinList::SkeletonMaleZombie => &self.skeleton_male_zombie,
            SkinList::FemaleZombie(s) => &self.female_zombie[zed(s)],
            SkinList::MaleZombie(s) => &self.male_zombie[zed(s)],
        }
    }
}

/// [`HumanStyles`] over the local hair/beard styles and skin lists.
/// Random draws (`OutfitRNG`, `HairOutfitDefinitions`) keep the trait's
/// defaults.
#[derive(Clone, Copy)]
pub struct CharacterStyles<'a> {
    pub hair: &'a HairStyles,
    pub skins: &'a PopTemplateSkins,
}

impl HumanStyles for CharacterStyles<'_> {
    fn skins(&self, list: SkinList) -> &[String] {
        self.skins.list(list)
    }

    /// `FindFemaleStyle(name) != null` / `FindMaleStyle` (null finds none).
    fn has_hair_style(&self, female: bool, name: Option<&str>) -> bool {
        let Some(name) = name else { return false };
        if female {
            self.hair.find_female(name).is_some()
        } else {
            self.hair.find_male(name).is_some()
        }
    }

    /// `BeardStyles.FindStyle(name) != null`.
    fn has_beard_style(&self, name: Option<&str>) -> bool {
        name.is_some_and(|n| self.hair.find_beard(n).is_some())
    }
}
