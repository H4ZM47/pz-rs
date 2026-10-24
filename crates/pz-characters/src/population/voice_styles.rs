//! `VoiceStyle` and `VoiceStyles` (`media/voiceStyles/voiceStyles.xml`).

use super::xml::{parse_root, Element};
use super::{base_file, merge_by, mod_files, parse_file};
use pz_fs::java::is_null_or_whitespace;
use pz_fs::GameFs;
use pz_scripts::java::eq_ignore_case;

pub const VOICE_STYLES_FILE: &str = "media/voiceStyles/voiceStyles.xml";

/// `VoiceStyle`.
#[derive(Clone, Debug, PartialEq)]
pub struct VoiceStyle {
    pub name: String,
    pub prefix: String,
    pub voice_type: i32,
    pub body_type_default: i32,
}

impl Default for VoiceStyle {
    fn default() -> Self {
        VoiceStyle { name: String::new(), prefix: String::new(), voice_type: 0, body_type_default: 1 }
    }
}

impl VoiceStyle {
    /// Bind a `<style>` element.
    pub fn from_element(e: &Element) -> Self {
        let d = VoiceStyle::default();
        VoiceStyle {
            name: e.string_field("name").unwrap_or(d.name),
            prefix: e.string_field("prefix").unwrap_or(d.prefix),
            voice_type: e.int_field("voiceType").unwrap_or(d.voice_type),
            body_type_default: e.int_field("bodyTypeDefault").unwrap_or(d.body_type_default),
        }
    }

    /// `isValid()`: a prefix.
    pub fn is_valid(&self) -> bool {
        !is_null_or_whitespace(&self.prefix)
    }
}

/// `VoiceStyles`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct VoiceStyles {
    pub styles: Vec<VoiceStyle>,
}

impl VoiceStyles {
    /// Unmarshal a `<voiceStyles>` document.
    pub fn from_xml(bytes: &[u8]) -> Result<Self, String> {
        let root = parse_root(bytes, "voiceStyles")?;
        Ok(VoiceStyles { styles: root.children_named("style").map(VoiceStyle::from_element).collect() })
    }

    /// `Parse(filename)`.
    pub fn parse(path: &str) -> Option<Self> {
        parse_file(path, Self::from_xml)
    }

    /// `init()`: the game's file with a default style inserted first, then
    /// each mod's, matched by prefix.
    pub fn init(fs: &mut GameFs) -> Option<Self> {
        let mut inst = Self::parse(&base_file(fs, VOICE_STYLES_FILE))?;
        inst.styles.insert(0, VoiceStyle::default());
        for (_mod_id, m) in mod_files(fs, VOICE_STYLES_FILE, Self::parse) {
            merge_by(&mut inst.styles, m.styles, |l, s| find_index(l, Some(&s.prefix)));
        }
        Some(inst)
    }

    /// `getAllStyles()`.
    pub fn get(&self) -> &[VoiceStyle] {
        &self.styles
    }

    /// `FindStyle(name)`: by prefix ignoring case.
    pub fn find_style(&self, prefix: Option<&str>) -> Option<&VoiceStyle> {
        find_index(&self.styles, prefix).map(|i| &self.styles[i])
    }
}

fn find_index(list: &[VoiceStyle], prefix: Option<&str>) -> Option<usize> {
    let prefix = prefix?;
    list.iter().position(|s| eq_ignore_case(&s.prefix, prefix))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn binds_defaults() {
        let v = VoiceStyles::from_xml(
            b"<voiceStyles><style><name>V</name><prefix>P</prefix></style><style><voiceType>4</voiceType>\
              <bodyTypeDefault>x</bodyTypeDefault></style></voiceStyles>",
        )
        .unwrap();
        assert_eq!(v.styles[0], VoiceStyle { name: "V".into(), prefix: "P".into(), voice_type: 0, body_type_default: 1 });
        assert_eq!((v.styles[1].voice_type, v.styles[1].body_type_default), (4, 0));
        assert!(!v.styles[1].is_valid());
        assert_eq!(v.find_style(Some("p")).unwrap().name, "V");
    }
}
