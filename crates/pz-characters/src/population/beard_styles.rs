//! `BeardStyle` and `BeardStyles` (`media/hairStyles/beardStyles.xml`).

use super::hair_outfit_definitions::HairOutfitDefinitions;
use super::xml::{parse_root, Element};
use super::{base_file, merge_by, mod_files, parse_file};
use pz_fs::java::is_null_or_whitespace;
use pz_fs::GameFs;
use pz_scripts::java::eq_ignore_case;

pub const BEARD_STYLES_FILE: &str = "media/hairStyles/beardStyles.xml";

/// `BeardStyle`.
#[derive(Clone, Debug, PartialEq)]
pub struct BeardStyle {
    pub name: String,
    pub model: Option<String>,
    pub texture: String,
    pub level: i32,
    pub trim_choices: Vec<String>,
    pub grow_reference: bool,
}

impl Default for BeardStyle {
    fn default() -> Self {
        BeardStyle {
            name: String::new(),
            model: None,
            texture: "F_Hair_White".into(),
            level: 0,
            trim_choices: Vec::new(),
            grow_reference: false,
        }
    }
}

impl BeardStyle {
    /// Bind a `<style>` element.
    pub fn from_element(e: &Element) -> Self {
        let d = BeardStyle::default();
        BeardStyle {
            name: e.string_field("name").unwrap_or(d.name),
            model: e.string_field("model"),
            texture: e.string_field("texture").unwrap_or(d.texture),
            level: e.int_field("level").unwrap_or(0),
            trim_choices: e.string_list("trimChoices"),
            grow_reference: e.bool_field("growReference").unwrap_or(false),
        }
    }

    /// `isValid()`.
    pub fn is_valid(&self) -> bool {
        !self.model.as_deref().is_none_or(is_null_or_whitespace) && !is_null_or_whitespace(&self.texture)
    }
}

/// `BeardStyles`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BeardStyles {
    pub styles: Vec<BeardStyle>,
}

impl BeardStyles {
    /// Unmarshal a `<beardStyles>` document.
    pub fn from_xml(bytes: &[u8]) -> Result<Self, String> {
        let root = parse_root(bytes, "beardStyles")?;
        Ok(BeardStyles { styles: root.children_named("style").map(BeardStyle::from_element).collect() })
    }

    /// `Parse(filename)`.
    pub fn parse(path: &str) -> Option<Self> {
        parse_file(path, Self::from_xml)
    }

    /// `init()`: the game's file with a default (no beard) style inserted
    /// first, then each mod's.
    pub fn init(fs: &mut GameFs) -> Option<Self> {
        let mut inst = Self::parse(&base_file(fs, BEARD_STYLES_FILE))?;
        inst.styles.insert(0, BeardStyle::default());
        for (_mod_id, m) in mod_files(fs, BEARD_STYLES_FILE, Self::parse) {
            merge_by(&mut inst.styles, m.styles, |l, s| find_index(l, Some(&s.name)));
        }
        Some(inst)
    }

    /// `getAllStyles()`.
    pub fn get(&self) -> &[BeardStyle] {
        &self.styles
    }

    /// `FindStyle(name)`: by name ignoring case.
    pub fn find_style(&self, name: Option<&str>) -> Option<&BeardStyle> {
        find_index(&self.styles, name).map(|i| &self.styles[i])
    }

    /// `getRandomStyle(outfitName)`. `None` only for an empty list (where
    /// the original throws).
    pub fn get_random_style(&self, defs: &HairOutfitDefinitions, outfit: Option<&str>) -> Option<String> {
        defs.get_random_beard(outfit, &self.styles)
    }
}

fn find_index(list: &[BeardStyle], name: Option<&str>) -> Option<usize> {
    let name = name?;
    list.iter().position(|s| eq_ignore_case(&s.name, name))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn binds_and_finds() {
        let doc = b"<beardStyles><style><name>Full</name><model>m</model><level>2</level><trimChoices>Goatee</trimChoices>\
                    <growReference>true</growReference></style><style><name>Goatee</name></style></beardStyles>";
        let mut b = BeardStyles::from_xml(doc).unwrap();
        b.styles.insert(0, BeardStyle::default());
        assert_eq!(b.styles.len(), 3);
        assert!(b.styles[1].is_valid() && b.styles[1].grow_reference);
        assert!(!b.styles[2].is_valid());
        assert_eq!(b.find_style(Some("")).unwrap().name, "");
        assert_eq!(b.find_style(Some("full")).unwrap().level, 2);
        assert!(b.find_style(None).is_none());
    }
}
