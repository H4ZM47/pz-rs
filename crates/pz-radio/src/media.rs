//! `RecordedMedia` and `MediaData`: the CDs and VHS tapes the Lua side
//! registers (`OnInitRecordedMedia`), their spawn tables, and
//! `recorded_media.bin` in the world save (the media index list and the
//! home VHS tapes already spawned).

use std::path::Path;

use pz_inventory::io::{Reader, Writer};

use crate::java::{compare_ignore_case, rand_range, JavaHashMap, Rng};
use crate::RadioText;

/// `RecordedMedia.SAVE_FILE`.
pub const MEDIA_SAVE_FILE: &str = "recorded_media.bin";
/// `RecordedMedia.VERSION`.
pub const MEDIA_VERSION: i32 = 2;

/// `MediaData.MediaLineData`.
#[derive(Clone, Debug, PartialEq)]
pub struct MediaLineData {
    text: String,
    r: f32,
    g: f32,
    b: f32,
    codes: Option<String>,
}

impl MediaLineData {
    /// Black is turned into white, as in the original.
    pub fn new(text: &str, r: f32, g: f32, b: f32, codes: Option<&str>) -> Self {
        let (r, g, b) = if r == 0.0 && g == 0.0 && b == 0.0 {
            (1.0, 1.0, 1.0)
        } else {
            (r, g, b)
        };
        MediaLineData {
            text: text.to_string(),
            r,
            g,
            b,
            codes: codes.map(String::from),
        }
    }

    /// `getTextGuid`: the translation key of the line.
    pub fn text_guid(&self) -> &str {
        &self.text
    }

    /// `getTranslatedText`.
    pub fn translated_text(&self, text: &dyn RadioText) -> String {
        text.get_text(&self.text)
    }

    pub fn r(&self) -> f32 {
        self.r
    }

    pub fn g(&self) -> f32 {
        self.g
    }

    pub fn b(&self) -> f32 {
        self.b
    }

    pub fn codes(&self) -> Option<&str> {
        self.codes.as_deref()
    }
}

/// `MediaData`.
#[derive(Clone, Debug, PartialEq)]
pub struct MediaData {
    id: String,
    item_display_name: String,
    pub title: Option<String>,
    pub subtitle: Option<String>,
    pub author: Option<String>,
    pub extra: Option<String>,
    index: i16,
    category: Option<String>,
    spawning: i32,
    lines: Vec<MediaLineData>,
}

impl MediaData {
    pub fn new(id: &str, item_display_name: &str, spawning: i32) -> Self {
        MediaData {
            id: id.to_string(),
            item_display_name: item_display_name.to_string(),
            title: None,
            subtitle: None,
            author: None,
            extra: None,
            index: 0,
            category: None,
            spawning,
            lines: Vec::new(),
        }
    }

    pub fn add_line(&mut self, text: &str, r: f32, g: f32, b: f32, codes: Option<&str>) {
        self.lines.push(MediaLineData::new(text, r, g, b, codes));
    }

    pub fn line_count(&self) -> usize {
        self.lines.len()
    }

    pub fn line(&self, index: i32) -> Option<&MediaLineData> {
        usize::try_from(index).ok().and_then(|i| self.lines.get(i))
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn item_display_name(&self) -> &str {
        &self.item_display_name
    }

    /// `getTranslatedItemDisplayName`.
    pub fn translated_item_display_name(&self, text: &dyn RadioText) -> String {
        text.get_text(&self.item_display_name)
    }

    pub fn index(&self) -> i16 {
        self.index
    }

    pub fn category(&self) -> Option<&str> {
        self.category.as_deref()
    }

    pub fn spawning(&self) -> i32 {
        self.spawning
    }

    /// `getMediaType`: 0 for CDs, 1 for everything else, -1 without a
    /// category.
    pub fn media_type(&self) -> i8 {
        media_type_for_category(self.category.as_deref())
    }
}

/// `RecordedMedia.getMediaTypeForCategory`.
pub fn media_type_for_category(category: Option<&str>) -> i8 {
    match category {
        None => -1,
        Some(c) if c.eq_ignore_ascii_case("cds") => 0,
        Some(_) => 1,
    }
}

/// `RecordedMedia`.
#[derive(Clone, Debug)]
pub struct RecordedMedia {
    indexes: Vec<String>,
    media: JavaHashMap<String, MediaData>,
    /// Category -> media ids, in registration order.
    categorized: std::collections::HashMap<String, Vec<String>>,
    categories: Vec<String>,
    legacy_listened_lines: Vec<String>,
    home_vhs_spawned: JavaHashMap<i16, ()>,
    retail_vhs_spawn_table: [Vec<String>; 3],
    retail_cd_spawn_table: [Vec<String>; 3],
    requires_saving: bool,
    /// `RecordedMedia.disableLineLearning`.
    pub disable_line_learning: bool,
}

impl Default for RecordedMedia {
    fn default() -> Self {
        RecordedMedia {
            indexes: Vec::new(),
            media: JavaHashMap::new(),
            categorized: Default::default(),
            categories: Vec::new(),
            legacy_listened_lines: Vec::new(),
            home_vhs_spawned: JavaHashMap::new(),
            retail_vhs_spawn_table: Default::default(),
            retail_cd_spawn_table: Default::default(),
            requires_saving: true,
            disable_line_learning: false,
        }
    }
}

/// Where `recorded_media.bin` goes: `None` is `Core.isNoSave()`.
pub type MediaSaveDir<'a> = Option<&'a Path>;

impl RecordedMedia {
    pub fn new() -> Self {
        Self::default()
    }

    /// `init()`: loads the save, lets the Lua side register the media
    /// (`OnInitRecordedMedia`, through `register_media`), builds the retail
    /// spawn tables by rarity and saves.
    pub fn init(
        &mut self,
        save_dir: MediaSaveDir<'_>,
        register_media: &mut dyn FnMut(&mut RecordedMedia),
    ) {
        self.load(save_dir);
        register_media(self);
        self.retail_cd_spawn_table = Default::default();
        self.retail_vhs_spawn_table = Default::default();
        for (cat, table) in [("CDs", 0), ("Retail-VHS", 1)] {
            let Some(ids) = self.categorized.get(cat) else {
                continue;
            };
            for id in ids {
                let rarity = match self.media.get(id).map(MediaData::spawning) {
                    Some(1) => 1,
                    Some(2) => 2,
                    _ => 0,
                };
                let t = if table == 0 {
                    &mut self.retail_cd_spawn_table
                } else {
                    &mut self.retail_vhs_spawn_table
                };
                t[rarity].push(id.clone());
            }
        }
        self.save(save_dir);
    }

    pub fn categories(&self) -> &[String] {
        &self.categories
    }

    pub fn indexes(&self) -> &[String] {
        &self.indexes
    }

    pub fn legacy_listened_lines(&self) -> &[String] {
        &self.legacy_listened_lines
    }

    /// The spawned home VHS indexes, in the original's set order.
    pub fn home_vhs_spawned(&self) -> Vec<i16> {
        self.home_vhs_spawned
            .iter()
            .into_iter()
            .map(|(k, _)| *k)
            .collect()
    }

    fn sorted_by_name<'a>(
        &'a self,
        mut v: Vec<&'a MediaData>,
        text: &dyn RadioText,
    ) -> Vec<&'a MediaData> {
        v.sort_by(|a, b| {
            compare_ignore_case(
                &a.translated_item_display_name(text),
                &b.translated_item_display_name(text),
            )
        });
        v
    }

    /// `getAllMediaForType(type)`, sorted by translated name.
    pub fn all_media_for_type(&self, media_type: i8, text: &dyn RadioText) -> Vec<&MediaData> {
        let v = self
            .media
            .iter()
            .into_iter()
            .map(|(_, m)| m)
            .filter(|m| m.media_type() == media_type)
            .collect();
        self.sorted_by_name(v, text)
    }

    /// `getAllMediaForCategory(category)`, sorted by translated name.
    pub fn all_media_for_category(&self, category: &str, text: &dyn RadioText) -> Vec<&MediaData> {
        let v = self
            .media
            .iter()
            .into_iter()
            .map(|(_, m)| m)
            .filter(|m| {
                m.category()
                    .is_some_and(|c| c.to_lowercase() == category.to_lowercase())
            })
            .collect();
        self.sorted_by_name(v, text)
    }

    /// `register(category, id, itemDisplayName, spawning)`: `None` when the
    /// id is taken. A media id keeps the index it had in the save.
    pub fn register(
        &mut self,
        category: &str,
        id: &str,
        item_display_name: &str,
        spawning: i32,
    ) -> Option<&mut MediaData> {
        if self.media.contains_key(id) {
            return None;
        }
        let mut m = MediaData::new(id, item_display_name, spawning.max(0));
        m.category = Some(category.to_string());
        if !self.categorized.contains_key(category) {
            self.categorized.insert(category.to_string(), Vec::new());
            self.categories.push(category.to_string());
        }
        self.categorized
            .get_mut(category)
            .unwrap()
            .push(id.to_string());
        m.index = match self.indexes.iter().position(|i| i == id) {
            Some(i) => i as i16,
            None => {
                self.indexes.push(id.to_string());
                (self.indexes.len() - 1) as i16
            }
        };
        self.media.insert(id.to_string(), m);
        self.requires_saving = true;
        self.media.get_mut(id)
    }

    /// `getMediaDataFromIndex`.
    pub fn media_data_from_index(&self, index: i16) -> Option<&MediaData> {
        let i = usize::try_from(index).ok()?;
        self.indexes.get(i).and_then(|id| self.media.get(id))
    }

    /// `getIndexForMediaData`.
    pub fn index_for_media_data(&self, data: &MediaData) -> i16 {
        self.indexes
            .iter()
            .position(|i| *i == data.id)
            .map_or(-1, |i| i as i16)
    }

    /// `getMediaData(id)`.
    pub fn media_data(&self, id: &str) -> Option<&MediaData> {
        self.media.get(id)
    }

    pub fn media_data_mut(&mut self, id: &str) -> Option<&mut MediaData> {
        self.media.get_mut(id)
    }

    fn pick_retail(table: &[Vec<String>; 3], rng: &mut Rng<'_>) -> Option<String> {
        let pick = |list: &Vec<String>, rng: &mut Rng<'_>| -> Option<String> {
            list.get(rand_range(rng, 0, list.len() as i32) as usize)
                .cloned()
        };
        let chance = rand_range(rng, 0, 1000);
        let data = if chance < 100 {
            if table[2].is_empty() {
                None
            } else {
                pick(&table[2], rng)
            }
        } else if chance < 400 {
            if table[1].is_empty() {
                None
            } else {
                pick(&table[1], rng)
            }
        } else {
            pick(&table[0], rng)
        };
        data.or_else(|| pick(&table[0], rng))
    }

    /// `getRandomFromCategory(category)`: a CD or retail tape by rarity
    /// (exceptional 10%, rare 30%, common otherwise, falling back to a
    /// common one), or, one time in five, a home tape that has not spawned
    /// yet. Where the original throws on an empty common table this
    /// answers `None`.
    pub fn random_from_category(
        &mut self,
        category: &str,
        rng: &mut Rng<'_>,
    ) -> Option<&MediaData> {
        if !self.categorized.contains_key(category) {
            return None;
        }
        let id = if category.eq_ignore_ascii_case("cds") {
            Self::pick_retail(&self.retail_cd_spawn_table, rng)?
        } else if category.eq_ignore_ascii_case("retail-vhs") {
            Self::pick_retail(&self.retail_vhs_spawn_table, rng)?
        } else if category.eq_ignore_ascii_case("home-vhs") {
            if rand_range(rng, 0, 1000) >= 200 {
                return None;
            }
            let list = self.categorized.get("Home-VHS")?;
            let id = list
                .get(rand_range(rng, 0, list.len() as i32) as usize)?
                .clone();
            let index = self.media.get(&id)?.index;
            if self.home_vhs_spawned.contains_key(&index) {
                return None;
            }
            self.home_vhs_spawned.insert(index, ());
            self.requires_saving = true;
            id
        } else {
            return None;
        };
        self.media.get(&id)
    }

    /// `load()`: the index list (and, in version 1 files, the lines heard)
    /// and the spawned home tapes. A truncated file keeps what was read.
    pub fn load(&mut self, save_dir: MediaSaveDir<'_>) {
        self.indexes.clear();
        let Some(dir) = save_dir else {
            return;
        };
        let Ok(bytes) = std::fs::read(dir.join(MEDIA_SAVE_FILE)) else {
            return;
        };
        let _ = self.read(&bytes);
    }

    fn read(&mut self, bytes: &[u8]) -> pz_inventory::io::SaveResult<()> {
        let mut r = Reader::new(bytes);
        let version = r.i32()?;
        let n = r.i32()?;
        for _ in 0..n {
            let id = r.string()?;
            self.indexes.push(id);
        }
        if version == 1 {
            let n = r.i32()?;
            for _ in 0..n {
                let id = r.string()?;
                self.legacy_listened_lines.push(id);
            }
        }
        let n = r.i32()?;
        for _ in 0..n {
            let s = r.i16()?;
            self.home_vhs_spawned.insert(s, ());
        }
        Ok(())
    }

    /// The bytes `save()` writes, or `None` where they overflow the
    /// original's fixed buffer (40 bytes per index, 2 per tape, plus 512),
    /// which makes it write nothing.
    pub fn save_bytes(&self) -> Option<Vec<u8>> {
        let capacity = self.indexes.len() * 40 + self.home_vhs_spawned.len() * 2 + 512;
        let mut w = Writer::new();
        w.i32(MEDIA_VERSION);
        w.i32(self.indexes.len() as i32);
        for id in &self.indexes {
            w.string(id);
        }
        w.i32(self.home_vhs_spawned.len() as i32);
        for s in self.home_vhs_spawned() {
            w.i16(s);
        }
        (w.buf.len() <= capacity).then_some(w.buf)
    }

    /// `save()`: writes `recorded_media.bin` when something changed.
    pub fn save(&mut self, save_dir: MediaSaveDir<'_>) {
        let Some(dir) = save_dir else {
            return;
        };
        if !self.requires_saving {
            return;
        }
        if let Some(bytes) = self.save_bytes() {
            let _ = std::fs::write(dir.join(MEDIA_SAVE_FILE), bytes);
        }
        self.requires_saving = false;
    }

    /// `handleLegacyListenedLines(player)`: hands the lines a version 1
    /// save recorded to `learn` (the player's known media lines) once.
    pub fn handle_legacy_listened_lines(&mut self, learn: Option<&mut dyn FnMut(&str)>) {
        if self.legacy_listened_lines.is_empty() {
            return;
        }
        if let Some(learn) = learn {
            for g in &self.legacy_listened_lines {
                learn(g);
            }
        }
        self.legacy_listened_lines.clear();
    }

    /// `hasListenedToAll(player, media)`: every line is known and there is
    /// at least one.
    pub fn has_listened_to_all(
        &self,
        knows_line: &dyn Fn(&str) -> bool,
        media: &MediaData,
    ) -> bool {
        media.lines.iter().all(|l| knows_line(&l.text)) && !media.lines.is_empty()
    }
}

/// `RecordedMedia.toAscii`: decomposes and drops everything outside ASCII
/// (so accented letters keep their base letter).
pub fn to_ascii(s: &str) -> String {
    s.chars()
        .flat_map(decompose)
        .filter(|c| (*c as u32) <= 0x7f)
        .collect()
}

/// The base letter of the Latin-1 and Latin Extended-A letters with
/// diacritics (`Normalizer.Form.NFD` keeps it first, then combining
/// marks, which are dropped anyway).
fn decompose(c: char) -> Vec<char> {
    const MAP: &[(char, &str)] = &[
        ('A', "ÀÁÂÃÄÅĀĂĄ"),
        ('a', "àáâãäåāăą"),
        ('C', "ÇĆĈĊČ"),
        ('c', "çćĉċč"),
        ('D', "Ď"),
        ('d', "ď"),
        ('E', "ÈÉÊËĒĔĖĘĚ"),
        ('e', "èéêëēĕėęě"),
        ('G', "ĜĞĠĢ"),
        ('g', "ĝğġģ"),
        ('H', "Ĥ"),
        ('h', "ĥ"),
        ('I', "ÌÍÎÏĨĪĬĮİ"),
        ('i', "ìíîïĩīĭį"),
        ('J', "Ĵ"),
        ('j', "ĵ"),
        ('K', "Ķ"),
        ('k', "ķ"),
        ('L', "ĹĻĽ"),
        ('l', "ĺļľ"),
        ('N', "ÑŃŅŇ"),
        ('n', "ñńņň"),
        ('O', "ÒÓÔÕÖŌŎŐ"),
        ('o', "òóôõöōŏő"),
        ('R', "ŔŖŘ"),
        ('r', "ŕŗř"),
        ('S', "ŚŜŞŠ"),
        ('s', "śŝşš"),
        ('T', "ŢŤ"),
        ('t', "ţť"),
        ('U', "ÙÚÛÜŨŪŬŮŰŲ"),
        ('u', "ùúûüũūŭůűų"),
        ('W', "Ŵ"),
        ('w', "ŵ"),
        ('Y', "ÝŶŸ"),
        ('y', "ýÿŷ"),
        ('Z', "ŹŻŽ"),
        ('z', "źżž"),
    ];
    for (base, accented) in MAP {
        if accented.contains(c) {
            return vec![*base];
        }
    }
    vec![c]
}
