//! `RadioData` and `RadioTranslationData`: finding the radio script files
//! (`media/radio/*.xml` of the game and the mods), reading them into
//! channels, scripts, broadcasts and lines, and the community translation
//! files (`media/radio/*.txt`).

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::channel::{RadioChannel, RadioDataRef};
use crate::java::{
    float_to_string, parse_float, parse_int, rand_next, read_lines, split_char, trim, JavaHashMap,
    Rng,
};
use crate::script::{Broadcasts, RadioBroadCast, RadioLine, RadioScript};
use crate::xml::{parse_document, Element};
use crate::{ChannelCategory, RadioError};

/// A mod's media roots (`ChooseGameInfo.Mod.getCommonDir` /
/// `getVersionDir`).
#[derive(Clone, Debug, Default)]
pub struct ModRadioDirs {
    pub common_dir: Option<PathBuf>,
    pub version_dir: Option<PathBuf>,
}

/// What `RadioData` reads from `ZomboidFileSystem`, `Core` and
/// `Translator`.
#[derive(Clone, Debug, Default)]
pub struct RadioSource {
    /// `ZomboidFileSystem.getMediaFile("")`: the game's `media` directory.
    pub media_dir: PathBuf,
    /// The active mods that have details, in load order.
    pub mods: Vec<ModRadioDirs>,
    /// `Core.getContentTranslationsEnabled()`.
    pub content_translations_enabled: bool,
    /// `Translator.getLanguage().toString()`.
    pub language: String,
    /// `Translator.getDefaultLanguage().toString()`.
    pub default_language: String,
    /// `Translator.getAvailableLanguage()`, by name.
    pub available_languages: Vec<String>,
}

/// `Translator.getText(key)`: the radio lines are `RD_<line id>` entries of
/// the translation tables.
pub trait RadioText {
    fn get_text(&self, key: &str) -> String;
}

impl<F: Fn(&str) -> String> RadioText for F {
    fn get_text(&self, key: &str) -> String {
        self(key)
    }
}

/// `RadioTranslationData`.
#[derive(Clone, Debug, Default)]
pub struct RadioTranslationData {
    pub file_path: String,
    pub guid: Option<String>,
    pub language: Option<String>,
    /// The language, when it is one of the available ones.
    pub language_enum: Option<String>,
    pub version: i32,
    pub translators: Vec<String>,
    translations: HashMap<String, String>,
}

impl RadioTranslationData {
    pub fn translation_count(&self) -> usize {
        self.translations.len()
    }

    /// `getTranslation(guid)`.
    pub fn translation(&self, guid: &str) -> Option<&str> {
        self.translations.get(guid).map(String::as_str)
    }

    /// `validate`.
    pub fn validate(&self) -> bool {
        self.guid.is_some() && self.language.is_some() && self.version >= 0
    }

    /// `ReadFile(file)`: the `[Info]` header (`guid`, `language`,
    /// `version`, `translator`). `None` for an unknown language or an
    /// incomplete header.
    pub fn read_file(file: &str, available_languages: &[String]) -> Option<Self> {
        let mut t = RadioTranslationData {
            file_path: file.to_string(),
            version: -1,
            ..Default::default()
        };
        let path = Path::new(file);
        if path.is_file() {
            if let Ok(bytes) = std::fs::read(path) {
                let text = String::from_utf8_lossy(&bytes);
                for line in read_lines(&text) {
                    let parts = split_char(line, '=');
                    if parts.len() > 1 {
                        let id = trim(parts[0]);
                        let joined: String = parts[1..].concat();
                        let txt = trim(&joined).to_string();
                        match id {
                            "guid" => t.guid = Some(txt),
                            "language" => t.language = Some(txt),
                            "version" => match parse_int(&txt) {
                                Some(v) => t.version = v,
                                // The original's reader throws out of the loop.
                                None => break,
                            },
                            "translator" => {
                                t.translators
                                    .extend(split_char(&txt, ',').into_iter().map(String::from));
                            }
                            _ => {}
                        }
                    }
                    if trim(line) == "[/Info]" {
                        break;
                    }
                }
            }
        }
        if let Some(lang) = &t.language {
            t.language_enum = Some(available_languages.iter().find(|l| *l == lang)?.clone());
        }
        t.validate().then_some(t)
    }

    /// `loadTranslations()`: the `[Translations]` block, `key = text` lines
    /// and `[Collection]` groups giving several members one text. Only for
    /// the current language.
    pub fn load_translations(&mut self, current_language: &str) -> bool {
        if self.language_enum.as_deref() != Some(current_language) {
            return false;
        }
        let path = Path::new(&self.file_path);
        if !path.is_file() {
            return false;
        }
        let Ok(bytes) = std::fs::read(path) else {
            return false;
        };
        let text = String::from_utf8_lossy(&bytes);
        let lines = read_lines(&text);
        let mut i = 0;
        let mut open = false;
        let mut members: Vec<String> = Vec::new();
        let split2 = |l: &str| {
            l.split_once('=')
                .map(|(k, v)| (trim(k).to_string(), trim(v).to_string()))
        };
        while i < lines.len() {
            let line = trim(lines[i]);
            i += 1;
            if line == "[Translations]" {
                open = true;
                continue;
            }
            if !open {
                continue;
            }
            if line == "[Collection]" {
                let mut text: Option<String> = None;
                while i < lines.len() {
                    let l = trim(lines[i]);
                    i += 1;
                    if l == "[/Collection]" {
                        break;
                    }
                    if let Some((k, v)) = split2(l) {
                        if k == "text" {
                            text = Some(v);
                        } else if k == "member" {
                            members.push(v);
                        }
                    }
                }
                if let Some(text) = &text {
                    for m in &members {
                        self.translations.insert(m.clone(), text.clone());
                    }
                }
                members.clear();
                continue;
            }
            if line == "[/Translations]" {
                return true;
            }
            if let Some((k, v)) = split2(line) {
                self.translations.insert(k, v);
            }
        }
        false
    }
}

/// `RadioData`: one radio script file.
#[derive(Clone, Debug, Default)]
pub struct RadioData {
    is_vanilla: bool,
    guid: Option<String>,
    version: i32,
    xml_file_path: String,
    radio_channels: Vec<RadioChannel>,
    translation_data_list: Vec<RadioTranslationData>,
    current_translation: Option<usize>,
    root: Option<Element>,
    advert_que: HashMap<String, RadioScript>,
}

/// `RadioData.searchForFiles`: every file below `path` whose lower-case
/// absolute path contains `extension` anywhere (not only as the suffix),
/// in directory listing order.
fn search_for_files(path: &Path, extension: &str, files: &mut Vec<String>) {
    if path.is_dir() {
        let Ok(rd) = std::fs::read_dir(path) else {
            return;
        };
        for e in rd.flatten() {
            search_for_files(&path.join(e.file_name()), extension, files);
        }
    } else {
        let abs = absolute(path);
        if abs.to_lowercase().contains(extension) {
            files.push(abs);
        }
    }
}

fn absolute(path: &Path) -> String {
    std::path::absolute(path)
        .unwrap_or_else(|_| path.to_path_buf())
        .to_string_lossy()
        .into_owned()
}

/// `ZomboidFileSystem.getRelativeFile(lowercased root, file)` lower-cased:
/// the file's path below the mod root, or the whole path when it is not
/// below it.
fn relative_lower(root: &Path, file: &str) -> String {
    let root = absolute(root).to_lowercase();
    let root = root.trim_end_matches('/');
    let lower = file.to_lowercase();
    match lower.strip_prefix(root).and_then(|r| r.strip_prefix('/')) {
        Some(rel) => rel.to_string(),
        None => lower,
    }
}

impl RadioData {
    pub fn guid(&self) -> Option<&str> {
        self.guid.as_deref()
    }

    pub fn version(&self) -> i32 {
        self.version
    }

    pub fn xml_file_path(&self) -> &str {
        &self.xml_file_path
    }

    /// `isVanilla`: the file is one of the game's own.
    pub fn is_vanilla(&self) -> bool {
        self.is_vanilla
    }

    /// `getRadioChannels`.
    pub fn radio_channels(&self) -> &[RadioChannel] {
        &self.radio_channels
    }

    /// Moves the channels out (to the script manager).
    pub fn take_channels(&mut self) -> Vec<RadioChannel> {
        std::mem::take(&mut self.radio_channels)
    }

    pub fn translations(&self) -> &[RadioTranslationData] {
        &self.translation_data_list
    }

    /// `getTranslatorNames(language)`: the translators credited by the
    /// game's own radio translation files for `language` (none for the
    /// default language).
    pub fn translator_names(source: &RadioSource, language: &str) -> Vec<String> {
        let mut names: Vec<String> = Vec::new();
        if language != source.default_language {
            for rd in Self::fetch_radio_data(source, false) {
                for t in &rd.translation_data_list {
                    if t.language_enum.as_deref() != Some(language) {
                        continue;
                    }
                    for n in &t.translators {
                        if !names.contains(n) {
                            names.push(n.clone());
                        }
                    }
                }
            }
        }
        names
    }

    /// `fetchRadioData(loadMods)`: every readable radio file with the
    /// translation files that share its guid.
    pub fn fetch_radio_data(source: &RadioSource, load_mods: bool) -> Vec<RadioData> {
        let mut list = Vec::new();
        let radio_dir = source.media_dir.join("radio");
        let mut files = Vec::new();
        search_for_files(&radio_dir, "xml", &mut files);
        let vanilla = files.clone();
        if load_mods {
            for m in &source.mods {
                let mut map: JavaHashMap<String, String> = JavaHashMap::new();
                for dir in [&m.common_dir, &m.version_dir].into_iter().flatten() {
                    let mut mod_files = Vec::new();
                    search_for_files(&dir.join("media").join("radio"), "xml", &mut mod_files);
                    for f in mod_files {
                        let rel = relative_lower(dir, &f);
                        if !map.contains_key(&rel) {
                            map.insert(rel, f);
                        }
                    }
                }
                files.extend(map.iter().into_iter().map(|(_, f)| f.clone()));
            }
        }
        for file in &files {
            if let Some(mut rd) = Self::read_file(file) {
                if vanilla.contains(file) {
                    rd.is_vanilla = true;
                }
                list.push(rd);
            }
        }
        let mut files = Vec::new();
        search_for_files(&radio_dir, "txt", &mut files);
        if load_mods {
            for m in &source.mods {
                for dir in [&m.common_dir, &m.version_dir].into_iter().flatten() {
                    search_for_files(&dir.join("media").join("radio"), "txt", &mut files);
                }
            }
        }
        for file in &files {
            if let Some(t) = RadioTranslationData::read_file(file, &source.available_languages) {
                for rd in list.iter_mut() {
                    if rd.guid == t.guid {
                        rd.translation_data_list.push(t.clone());
                    }
                }
            }
        }
        list
    }

    /// `fetchAllRadioData()`: the files whose scripts load, each read in
    /// reverse file order (the order the game RNG is drawn in). A malformed
    /// script entry is an error, as the original's exception aborts
    /// `ZomboidRadio.Init`.
    pub fn fetch_all_radio_data(
        source: &RadioSource,
        text: &dyn RadioText,
        arena: &mut Broadcasts,
        rng: &mut Rng<'_>,
    ) -> Result<Vec<RadioData>, RadioError> {
        let mut list = Self::fetch_radio_data(source, true);
        for i in (0..list.len()).rev() {
            if list[i].load_radio_scripts(source, text, arena, rng)? {
                list[i].current_translation = None;
                list[i].translation_data_list.clear();
            } else {
                list.remove(i);
            }
        }
        Ok(list)
    }

    /// `ReadFile(path)`: `None` unless the document has a `RadioData`
    /// element whose `RootInfo` gives a guid and a version.
    pub fn read_file(file_path: &str) -> Option<RadioData> {
        let bytes = std::fs::read(file_path).ok()?;
        let doc = parse_document(&bytes).ok()?;
        let root = doc.first_named("RadioData")?.clone();
        let mut rd = RadioData {
            xml_file_path: file_path.to_string(),
            root: Some(root),
            ..Default::default()
        };
        rd.load_root_info().then_some(rd)
    }

    /// `loadRootInfo`. A `Version` that does not parse rejects the file.
    fn load_root_info(&mut self) -> bool {
        let root = self.root.as_ref().unwrap();
        for n in root.elements().filter(|e| e.name == "RootInfo") {
            for n2 in n.elements() {
                let val = n2.text_content();
                match trim(&n2.name) {
                    "Version" => match parse_int(&val) {
                        Some(v) => self.version = v,
                        None => return false,
                    },
                    "FileGUID" => self.guid = Some(val),
                    _ => {}
                }
            }
        }
        self.guid.is_some() && self.version >= 0
    }

    fn data_ref(&self) -> RadioDataRef {
        RadioDataRef {
            guid: self.guid.clone().unwrap_or_default(),
            vanilla: self.is_vanilla,
        }
    }

    /// `loadRadioScripts`: the adverts first, then the channels; `false`
    /// when the file has no `Channels` element.
    pub fn load_radio_scripts(
        &mut self,
        source: &RadioSource,
        text: &dyn RadioText,
        arena: &mut Broadcasts,
        rng: &mut Rng<'_>,
    ) -> Result<bool, RadioError> {
        self.current_translation = None;
        self.advert_que.clear();
        if source.content_translations_enabled && source.language != source.default_language {
            // The translation is loaded but, as in the original, never
            // consulted: the lines come from the translator tables.
            for i in 0..self.translation_data_list.len() {
                let t = &mut self.translation_data_list[i];
                if t.language_enum.as_deref() == Some(source.language.as_str())
                    && t.load_translations(&source.language)
                {
                    self.current_translation = Some(i);
                }
            }
        }
        let root = self
            .root
            .take()
            .ok_or_else(|| RadioError("radio data already loaded".into()))?;
        for n in root.elements().filter(|e| e.name == "Adverts") {
            for script in self.load_scripts(n, true, text, arena, rng)? {
                if !self.advert_que.contains_key(script.name()) {
                    self.advert_que.insert(script.guid().to_string(), script);
                }
            }
        }
        let mut success = false;
        for n in root.elements().filter(|e| e.name == "Channels") {
            self.load_channels(n, text, arena, rng)?;
            success = true;
        }
        Ok(success)
    }

    fn load_channels(
        &mut self,
        parent: &Element,
        text: &dyn RadioText,
        arena: &mut Broadcasts,
        rng: &mut Rng<'_>,
    ) -> Result<(), RadioError> {
        for n in parent.elements().filter(|e| e.name == "ChannelEntry") {
            let id = attrib(n, "ID")?;
            let name = attrib(n, "name")?;
            let cat = attrib(n, "cat")?;
            let freq = attrib(n, "freq")?;
            let start_script = attrib(n, "startscript")?;
            let category = ChannelCategory::value_of(&cat).ok_or_else(|| {
                RadioError(format!(
                    "No enum constant zombie.radio.ChannelCategory.{cat}"
                ))
            })?;
            let mut chan = RadioChannel::new(&name, int(&freq)?, category, &id);
            for script in self.load_scripts(n, false, text, arena, rng)? {
                chan.add_radio_script(script);
            }
            chan.set_active_script(&start_script, 0, 1, -1, rng);
            chan.set_radio_data(Some(self.data_ref()));
            self.radio_channels.push(chan);
        }
        Ok(())
    }

    fn load_scripts(
        &mut self,
        parent: &Element,
        advert_script: bool,
        text: &dyn RadioText,
        arena: &mut Broadcasts,
        rng: &mut Rng<'_>,
    ) -> Result<Vec<RadioScript>, RadioError> {
        let mut scripts = Vec::new();
        for n in parent.elements().filter(|e| e.name == "ScriptEntry") {
            let id = attrib(n, "ID")?;
            let name = attrib(n, "name")?;
            let loop_min = attrib(n, "loopmin")?;
            let loop_max = attrib(n, "loopmax")?;
            let mut script = RadioScript::new(&name, int(&loop_min)?, int(&loop_max)?, &id);
            for nn in n.elements() {
                if nn.name == "BroadcastEntry" {
                    self.load_broadcast(nn, &mut script, text, arena, rng)?;
                } else if !advert_script && nn.name == "ExitOptions" {
                    load_exit_options(nn, &mut script)?;
                }
            }
            scripts.push(script);
        }
        Ok(scripts)
    }

    fn load_broadcast(
        &mut self,
        node: &Element,
        script: &mut RadioScript,
        text: &dyn RadioText,
        arena: &mut Broadcasts,
        rng: &mut Rng<'_>,
    ) -> Result<(), RadioError> {
        let id = attrib(node, "ID")?;
        let start = attrib(node, "timestamp")?;
        let end = attrib(node, "endstamp")?;
        let (start, end) = (int(&start)?, int(&end)?);
        let is_segment = attrib(node, "isSegment")?.to_lowercase() == "true";
        let advert_cat = attrib(node, "advertCat")?;
        let mut bc = RadioBroadCast::new(&id, start, end);
        if !is_segment
            && advert_cat.to_lowercase() != "none"
            && self.advert_que.contains_key(&advert_cat)
            && rand_next(rng, 101) < 75
        {
            let adverts = self.advert_que[&advert_cat].broadcasts();
            if !adverts.is_empty() {
                if rand_next(rng, 101) < 50 {
                    bc.set_pre_segment(Some(
                        adverts[rand_next(rng, adverts.len() as i32) as usize],
                    ));
                } else {
                    bc.set_post_segment(Some(
                        adverts[rand_next(rng, adverts.len() as i32) as usize],
                    ));
                }
            }
        }
        for l in node.elements().filter(|e| e.name == "LineEntry") {
            let line_id = attrib(l, "ID")?;
            let r = attrib(l, "r")?;
            let g = attrib(l, "g")?;
            let b = attrib(l, "b")?;
            let codes = match l.attr("codes") {
                Some(_) => Some(attrib(l, "codes")?),
                None => None,
            };
            let t = text.get_text(&format!("RD_{line_id}"));
            let mut line = RadioLine::new(
                Some(&t),
                float(&r)? / 255.0,
                float(&g)? / 255.0,
                float(&b)? / 255.0,
                codes.as_deref(),
            );
            let trimmed = trim(&t);
            if trimmed.to_lowercase().starts_with("${t:") {
                let s = check_for_custom_air_timer(trimmed, &mut line);
                line.set_text(s);
            }
            bc.add_line(line);
        }
        let bid = arena.add(bc);
        // A refused broadcast is logged by the original and dropped.
        let _ = script.add_broadcast(arena, bid, is_segment);
        Ok(())
    }
}

/// `getAttrib(node, name)`: trimmed; a missing attribute throws.
fn attrib(node: &Element, name: &str) -> Result<String, RadioError> {
    node.attr(name)
        .map(|v| trim(v).to_string())
        .ok_or_else(|| RadioError(format!("<{}> has no attribute \"{name}\"", node.name)))
}

fn int(s: &str) -> Result<i32, RadioError> {
    parse_int(s).ok_or_else(|| RadioError(format!("For input string: \"{s}\"")))
}

fn float(s: &str) -> Result<f32, RadioError> {
    parse_float(s).ok_or_else(|| RadioError(format!("For input string: \"{s}\"")))
}

fn load_exit_options(node: &Element, script: &mut RadioScript) -> Result<(), RadioError> {
    for e in node.elements().filter(|e| e.name == "ExitOption") {
        let name = attrib(e, "script")?;
        let chance = attrib(e, "chance")?;
        let delay = attrib(e, "delay")?;
        let (chance, delay) = (int(&chance)?, int(&delay)?);
        script.add_exit_option(&name, chance, delay);
    }
    Ok(())
}

/// The first `${t:<value>}` field of `s` (the regex `\$\{t:([^}]+)\}`):
/// its byte range and the value.
fn find_time_field(s: &str) -> Option<(usize, usize, &str)> {
    let mut from = 0;
    while let Some(p) = s[from..].find("${t:") {
        let start = from + p;
        let body = start + 4;
        let q = s[body..].find('}')?;
        if q > 0 {
            return Some((start, body + q + 1, &s[body..body + q]));
        }
        from = start + 1;
    }
    None
}

/// `checkForCustomAirTimer`: a `${t:<minutes>}` field sets the line's air
/// time and is replaced by a `[cdt=<minutes>]` prefix.
fn check_for_custom_air_timer(line: &str, radio_line: &mut RadioLine) -> String {
    let mut result = line.to_string();
    let mut val = -1.0f32;
    if let Some((a, b, v)) = find_time_field(line) {
        if let Some(f) = parse_float(trim(&v.to_lowercase())) {
            val = f;
            radio_line.set_air_time(f);
        }
        result.replace_range(a..b, "");
    }
    if val >= 0.0 {
        format!("[cdt={}]{}", float_to_string(val), trim(&result))
    } else {
        trim(&result).to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn custom_air_timer() {
        let mut l = RadioLine::new(Some("x"), 1.0, 1.0, 1.0, None);
        assert_eq!(
            check_for_custom_air_timer("${t:2} Hi there", &mut l),
            "[cdt=2.0]Hi there"
        );
        assert_eq!(l.air_time(), 2.0);
        let mut l = RadioLine::new(Some("x"), 1.0, 1.0, 1.0, None);
        assert_eq!(
            check_for_custom_air_timer("${t:} a ${t:x} b", &mut l),
            "${t:} a  b"
        );
        assert_eq!(l.air_time(), -1.0);
    }
}
