//! `HairStyle` and `HairStyles` (`media/hairStyles/hairStyles.xml`).

use super::hair_outfit_definitions::HairOutfitDefinitions;
use super::xml::{parse_root, Element};
use super::{base_file, merge_by, mod_files, parse_file};
use pz_fs::java::is_null_or_whitespace;
use pz_fs::GameFs;
use pz_scripts::java::eq_ignore_case;

pub const HAIR_STYLES_FILE: &str = "media/hairStyles/hairStyles.xml";

/// `HairStyle.Alternate`: the style to use under a hat category.
#[derive(Clone, Debug, PartialEq)]
pub struct HairStyleAlternate {
    pub category: Option<String>,
    pub style: Option<String>,
}

/// `HairStyle`.
#[derive(Clone, Debug, PartialEq)]
pub struct HairStyle {
    pub name: String,
    pub model: Option<String>,
    pub texture: String,
    pub alternate: Vec<HairStyleAlternate>,
    pub level: i32,
    pub trim_choices: Vec<String>,
    pub grow_reference: bool,
    pub attached_hair: bool,
    pub no_choose: bool,
}

impl Default for HairStyle {
    fn default() -> Self {
        HairStyle {
            name: String::new(),
            model: None,
            texture: "F_Hair_White".into(),
            alternate: Vec::new(),
            level: 0,
            trim_choices: Vec::new(),
            grow_reference: false,
            attached_hair: false,
            no_choose: false,
        }
    }
}

impl HairStyle {
    /// Bind a `<male>`/`<female>` element.
    pub fn from_element(e: &Element) -> Self {
        let d = HairStyle::default();
        HairStyle {
            name: e.string_field("name").unwrap_or(d.name),
            model: e.string_field("model"),
            texture: e.string_field("texture").unwrap_or(d.texture),
            alternate: e
                .children_named("alternate")
                .map(|a| HairStyleAlternate {
                    category: a.attr("category").map(str::to_string),
                    style: a.attr("style").map(str::to_string),
                })
                .collect(),
            level: e.int_field("level").unwrap_or(0),
            trim_choices: e.string_list("trimChoices"),
            grow_reference: e.bool_field("growReference").unwrap_or(false),
            attached_hair: e.bool_field("attachedHair").unwrap_or(false),
            no_choose: e.bool_field("noChoose").unwrap_or(false),
        }
    }

    /// `isValid()`: a model and a texture.
    pub fn is_valid(&self) -> bool {
        !self.model.as_deref().is_none_or(is_null_or_whitespace) && !is_null_or_whitespace(&self.texture)
    }

    /// `getAlternate(category)`: the first alternate whose category equals
    /// `category` ignoring case (its style may be `null`), else the name.
    pub fn get_alternate(&self, category: &str) -> Option<&str> {
        for a in &self.alternate {
            if a.category.as_deref().is_some_and(|c| eq_ignore_case(category, c)) {
                return a.style.as_deref();
            }
        }
        Some(&self.name)
    }
}

/// `HairStyles`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct HairStyles {
    /// `maleStyles`.
    pub male: Vec<HairStyle>,
    /// `femaleStyles`.
    pub female: Vec<HairStyle>,
}

impl HairStyles {
    /// Unmarshal a `<hairStyles>` document.
    pub fn from_xml(bytes: &[u8]) -> Result<Self, String> {
        let root = parse_root(bytes, "hairStyles")?;
        Ok(HairStyles {
            male: root.children_named("male").map(HairStyle::from_element).collect(),
            female: root.children_named("female").map(HairStyle::from_element).collect(),
        })
    }

    /// `Parse(filename)`: `None` if missing or invalid (the latter logged).
    pub fn parse(path: &str) -> Option<Self> {
        parse_file(path, Self::from_xml)
    }

    /// `init()`: the game's file, then each mod's (female styles merged
    /// before male ones). `None` if the game's file does not load.
    pub fn init(fs: &mut GameFs) -> Option<Self> {
        let mut inst = Self::parse(&base_file(fs, HAIR_STYLES_FILE))?;
        for (_mod_id, m) in mod_files(fs, HAIR_STYLES_FILE, Self::parse) {
            merge_by(&mut inst.female, m.female, |l, s| find_index(l, Some(&s.name)));
            merge_by(&mut inst.male, m.male, |l, s| find_index(l, Some(&s.name)));
        }
        Some(inst)
    }

    /// The styles of one sex (`getAllFemaleStyles` / `getAllMaleStyles`).
    pub fn get(&self, female: bool) -> &[HairStyle] {
        if female {
            &self.female
        } else {
            &self.male
        }
    }

    /// `FindFemaleStyle(name)` / `FindMaleStyle(name)`.
    pub fn find_style(&self, female: bool, name: Option<&str>) -> Option<&HairStyle> {
        let list = self.get(female);
        find_index(list, name).map(|i| &list[i])
    }

    /// `FindMaleStyle(name)`.
    pub fn find_male_style(&self, name: Option<&str>) -> Option<&HairStyle> {
        self.find_style(false, name)
    }

    /// `FindFemaleStyle(name)`.
    pub fn find_female_style(&self, name: Option<&str>) -> Option<&HairStyle> {
        self.find_style(true, name)
    }

    /// `getRandomMaleStyle(outfitName)`.
    pub fn get_random_male_style(&self, defs: &HairOutfitDefinitions, outfit: Option<&str>, world_age_days: f32) -> String {
        defs.get_random_male_haircut(outfit, &self.male, world_age_days)
    }

    /// `getRandomFemaleStyle(outfitName)`.
    pub fn get_random_female_style(&self, defs: &HairOutfitDefinitions, outfit: Option<&str>, world_age_days: f32) -> String {
        defs.get_random_female_haircut(outfit, &self.female, world_age_days)
    }

    /// `getAlternateForHat(style, category)`: `None` for the `nohair`
    /// categories; the alternate of a style in either list (by identity);
    /// `style` itself if it is in neither.
    pub fn get_alternate_for_hat<'a>(&'a self, style: &'a HairStyle, category: &str) -> Option<&'a HairStyle> {
        if eq_ignore_case("nohair", category) || eq_ignore_case("nohairnobeard", category) {
            return None;
        }
        if self.female.iter().any(|s| std::ptr::eq(s, style)) {
            return self.find_female_style(style.get_alternate(category));
        }
        if self.male.iter().any(|s| std::ptr::eq(s, style)) {
            return self.find_male_style(style.get_alternate(category));
        }
        Some(style)
    }
}

/// `FindStyle(list, name)`: the first style named `name` ignoring case; an
/// empty name also matches `"bald"`.
fn find_index(list: &[HairStyle], name: Option<&str>) -> Option<usize> {
    list.iter().position(|s| match name {
        Some(n) => eq_ignore_case(&s.name, n) || (n.is_empty() && eq_ignore_case(&s.name, "bald")),
        None => false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const DOC: &str = r#"<?xml version="1.0"?>
<hairStyles>
  <male><name>Bald</name><model></model><alternate category="default" style="Bald"/><level>0</level></male>
  <male><name>Short</name><model>skinned/hair/short</model><alternate category="Hat" style="Bald"/>
    <alternate category="x"/><trimChoices>Bald</trimChoices><level>1</level><growReference>true</growReference></male>
  <female><name>Long</name><model>skinned/hair/long</model><texture>T</texture><noChoose>1</noChoose>
    <attachedHair>true</attachedHair><alternate category="hat" style="Gone"/></female>
</hairStyles>"#;

    #[test]
    fn binds_and_finds() {
        let h = HairStyles::from_xml(DOC.as_bytes()).unwrap();
        assert_eq!(h.male.len(), 2);
        let short = &h.male[1];
        assert_eq!(short.level, 1);
        assert!(short.grow_reference && short.is_valid());
        assert_eq!(short.texture, "F_Hair_White");
        assert!(!h.male[0].is_valid());
        let long = &h.female[0];
        assert!(long.no_choose && long.attached_hair);
        assert_eq!(h.find_male_style(Some("")).unwrap().name, "Bald");
        assert_eq!(h.find_male_style(Some("SHORT")).unwrap().name, "Short");
        assert!(h.find_male_style(None).is_none());
        assert_eq!(h.get_alternate_for_hat(short, "hat").unwrap().name, "Bald");
        assert!(h.get_alternate_for_hat(short, "x").is_none());
        assert_eq!(h.get_alternate_for_hat(short, "other").unwrap().name, "Short");
        assert!(h.get_alternate_for_hat(short, "NoHair").is_none());
        assert!(h.get_alternate_for_hat(long, "HAT").is_none());
        let outside = HairStyle::default();
        assert!(std::ptr::eq(h.get_alternate_for_hat(&outside, "hat").unwrap(), &outside));
        assert!(HairStyles::from_xml(b"<beardStyles/>").is_err());
    }

    #[test]
    fn mods_replace_in_place_or_append() {
        let mut list = HairStyles::from_xml(DOC.as_bytes()).unwrap().male;
        let mut bald = HairStyle { name: "BALD".into(), level: 7, ..Default::default() };
        let new = HairStyle { name: "New".into(), ..Default::default() };
        merge_by(&mut list, vec![bald.clone(), new], |l, s| find_index(l, Some(&s.name)));
        assert_eq!(list.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(), ["BALD", "Short", "New"]);
        bald.name = String::new();
        merge_by(&mut list, vec![bald], |l, s| find_index(l, Some(&s.name)));
        assert_eq!(list[0].name, "");
    }
}
