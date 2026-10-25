//! `zombie.core.Translator`, `Language`, `Languages` and `LanguageFile`.
//!
//! The original keeps everything in statics; here it is one [`Translator`]
//! value. Lookups that the original performs on shared static maps while
//! also recording state (the missing-key set, the `<br>` cache, the lazily
//! resolved language) use interior mutability so they can take `&self`.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use crate::jfmt::{self, FormatError, JavaArg};
use crate::json;

/// `zombie.core.Language`. Compared by identity (`Rc::ptr_eq`) where the
/// original compares references.
#[derive(Debug, PartialEq, Eq)]
pub struct Language {
    pub name: String,
    pub text: String,
    pub base: Option<String>,
    pub azerty: bool,
}

impl std::fmt::Display for Language {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.name)
    }
}

/// A mod's `common/` and versioned directories (`ChooseGameInfo.Mod`
/// `getCommonDir` / `getVersionDir`), in active mod order.
#[derive(Clone, Debug, Default)]
pub struct ModDirs {
    pub common: Option<PathBuf>,
    pub version: Option<PathBuf>,
}

/// `String.equalsIgnoreCase`.
pub(crate) fn equals_ignore_case(a: &str, b: &str) -> bool {
    let (mut x, mut y) = (a.chars(), b.chars());
    loop {
        match (x.next(), y.next()) {
            (None, None) => return true,
            (Some(c1), Some(c2)) => {
                if c1 == c2 {
                    continue;
                }
                let (u1, u2) = (upper(c1), upper(c2));
                if u1 == u2 || lower(u1) == lower(u2) {
                    continue;
                }
                return false;
            }
            _ => return false,
        }
    }
}

fn upper(c: char) -> char {
    let mut it = c.to_uppercase();
    match (it.next(), it.next()) {
        (Some(u), None) => u,
        _ => c,
    }
}

fn lower(c: char) -> char {
    let mut it = c.to_lowercase();
    match (it.next(), it.next()) {
        (Some(l), None) => l,
        _ => c,
    }
}

/// `StringUtils.isNullOrWhitespace`.
fn is_null_or_whitespace(s: Option<&str>) -> bool {
    s.is_none_or(|s| s.chars().all(char::is_whitespace))
}

/// `zombie.core.Languages`.
#[derive(Debug)]
pub struct Languages {
    languages: Vec<Rc<Language>>,
    default_language: Rc<Language>,
}

impl Default for Languages {
    fn default() -> Self {
        let en = Rc::new(Language {
            name: "EN".into(),
            text: "English".into(),
            base: None,
            azerty: false,
        });
        Languages {
            languages: vec![en.clone()],
            default_language: en,
        }
    }
}

impl Languages {
    /// `Languages.init`: the built-in English, then every language
    /// directory of the game and of each mod (common dir, then version dir).
    pub fn init(&mut self, base: &Path, mods: &[ModDirs]) {
        *self = Languages::default();
        self.load_translate_directory(&base.join("media/lua/shared/Translate"));
        for m in mods {
            for dir in [&m.common, &m.version].into_iter().flatten() {
                let d = dir.join("media/lua/shared/Translate");
                if d.is_dir() {
                    self.load_translate_directory(&d);
                }
            }
        }
    }

    pub fn default_language(&self) -> Rc<Language> {
        self.default_language.clone()
    }

    pub fn languages(&self) -> &[Rc<Language>] {
        &self.languages
    }

    pub fn by_index(&self, index: i32) -> Option<Rc<Language>> {
        usize::try_from(index)
            .ok()
            .and_then(|i| self.languages.get(i))
            .cloned()
    }

    pub fn by_name(&self, name: &str) -> Option<Rc<Language>> {
        self.languages
            .iter()
            .find(|l| equals_ignore_case(&l.name, name))
            .cloned()
    }

    pub fn index_by_name(&self, name: &str) -> i32 {
        self.languages
            .iter()
            .position(|l| equals_ignore_case(&l.name, name))
            .map_or(-1, |i| i as i32)
    }

    fn load_translate_directory(&mut self, dir: &Path) {
        // `Files.newDirectoryStream` order is the file system's readdir
        // order, which is also what `read_dir` yields.
        let Ok(rd) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in rd {
            let Ok(entry) = entry else { return };
            let path = entry.path();
            if !path.is_dir() || !path.join("language.json").exists() {
                continue;
            }
            let Some(data) = LanguageFile::load(&path) else {
                continue;
            };
            let lang = Rc::new(Language {
                name: data.name.clone(),
                text: data.language_name,
                base: data.base,
                azerty: data.azerty,
            });
            let index = self.index_by_name(&data.name);
            if index == -1 {
                self.languages.push(lang);
                continue;
            }
            self.languages[index as usize] = lang.clone();
            if data.name == self.default_language.name {
                self.default_language = lang;
            }
        }
    }
}

/// `zombie.core.LanguageFile`: a language directory's `language.json`.
#[derive(Clone, Debug)]
pub struct LanguageFile {
    pub name: String,
    pub language_name: String,
    pub base: Option<String>,
    pub azerty: bool,
}

impl LanguageFile {
    fn load(dir: &Path) -> Option<LanguageFile> {
        let name = dir.file_name()?.to_string_lossy().into_owned();
        match Self::read(&name, &dir.join("language.json")) {
            Ok(f) => Some(f),
            Err(e) => {
                eprintln!("ERROR: {e}");
                None
            }
        }
    }

    pub fn read(name: &str, path: &Path) -> Result<LanguageFile, String> {
        let text = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
        let obj = json::parse_object(&text, false).map_err(|e| e.0)?;
        let version = obj.opt_int("version", 1);
        let language_name = obj.opt_string("language_name");
        let base = obj.opt_string("base");
        let azerty = obj.opt_boolean("azerty", false);
        if version != 1 {
            return Err("invalid or missing version".into());
        }
        if is_null_or_whitespace(language_name.as_deref()) {
            return Err(format!(
                "missing or empty value \"language_name\" for language {name}"
            ));
        }
        Ok(LanguageFile {
            name: name.into(),
            language_name: language_name.unwrap(),
            base,
            azerty,
        })
    }
}

/// The translation tables, in the original's `BY_NAME` order, which is
/// also the order files are loaded in.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Table {
    Tooltip,
    IgUi,
    Recipes,
    RecipeGroups,
    Farming,
    ContextMenu,
    SurvivalGuide,
    Ui,
    Items,
    ItemName,
    Moodles,
    Sandbox,
    Challenge,
    Stash,
    Moveables,
    MakeUp,
    GameSound,
    DynamicRadio,
    EvolvedRecipeName,
    RecordedMedia,
    SurvivorNames,
    Attributes,
    Fluids,
    PrintMedia,
    PrintText,
    Entity,
    RadioData,
    BodyParts,
    MapLabel,
    Credits,
}

/// `Translator.BY_NAME`: file name (without `.json`) for each table.
pub const BY_NAME: [(&str, Table); 30] = [
    ("Tooltip", Table::Tooltip),
    ("IG_UI", Table::IgUi),
    ("Recipes", Table::Recipes),
    ("RecipeGroups", Table::RecipeGroups),
    ("Farming", Table::Farming),
    ("ContextMenu", Table::ContextMenu),
    ("SurvivalGuide", Table::SurvivalGuide),
    ("UI", Table::Ui),
    ("Items", Table::Items),
    ("ItemName", Table::ItemName),
    ("Moodles", Table::Moodles),
    ("Sandbox", Table::Sandbox),
    ("Challenge", Table::Challenge),
    ("Stash", Table::Stash),
    ("Moveables", Table::Moveables),
    ("MakeUp", Table::MakeUp),
    ("GameSound", Table::GameSound),
    ("DynamicRadio", Table::DynamicRadio),
    ("EvolvedRecipeName", Table::EvolvedRecipeName),
    ("Recorded_Media", Table::RecordedMedia),
    ("SurvivorNames", Table::SurvivorNames),
    ("Attributes", Table::Attributes),
    ("Fluids", Table::Fluids),
    ("Print_Media", Table::PrintMedia),
    ("Print_Text", Table::PrintText),
    ("Entity", Table::Entity),
    ("RadioData", Table::RadioData),
    ("BodyParts", Table::BodyParts),
    ("MapLabel", Table::MapLabel),
    ("Credits", Table::Credits),
];

/// `getText` key prefixes and the table each one reads, in test order.
const PREFIXES: [(&str, Table); 25] = [
    ("UI_", Table::Ui),
    ("Moodles_", Table::Moodles),
    ("SurvivalGuide_", Table::SurvivalGuide),
    ("Farming_", Table::Farming),
    ("IGUI_", Table::IgUi),
    ("ContextMenu_", Table::ContextMenu),
    ("GameSound_", Table::GameSound),
    ("Sandbox_", Table::Sandbox),
    ("Tooltip_", Table::Tooltip),
    ("Challenge_", Table::Challenge),
    ("MakeUp", Table::MakeUp),
    ("Stash_", Table::Stash),
    ("RM_", Table::RecordedMedia),
    ("SurvivorName_", Table::SurvivorNames),
    ("SurvivorSurname_", Table::SurvivorNames),
    ("Attributes_", Table::Attributes),
    ("Fluid_", Table::Fluids),
    ("Print_Media_", Table::PrintMedia),
    ("Print_Text_", Table::PrintText),
    ("EC_", Table::Entity),
    ("RD_", Table::RadioData),
    ("BODYPART_", Table::BodyParts),
    ("MapLabel_", Table::MapLabel),
    ("credits_", Table::Credits),
    ("AEBS_", Table::DynamicRadio),
];

/// What the original's `Core` / `ZomboidFileSystem` / system properties
/// supply to the translator.
#[derive(Clone, Debug, Default)]
pub struct TranslatorEnv {
    /// `ZomboidFileSystem.base.canonicalFile` (contains `media/`).
    pub base_dir: PathBuf,
    /// Active mods, in load order.
    pub mods: Vec<ModDirs>,
    /// `Core.getOptionLanguageName()`.
    pub option_language_name: Option<String>,
    /// `System.getProperty("user.language")`.
    pub user_language: String,
    /// `Core.debug`.
    pub core_debug: bool,
    /// `DebugOptions.instance.translationPrefix`.
    pub translation_prefix: bool,
    /// `Core.IS_DEV`.
    pub is_dev: bool,
    /// `Translator.LOOKALIKE_CHARS`, the characters refused in English
    /// sources when `is_dev`. It is game data, so it is read from the
    /// installed `projectzomboid.jar` at runtime rather than kept here.
    pub lookalike_chars: Vec<char>,
    /// `ZomboidFileSystem.getCacheDir()`, for `translationProblems.txt`.
    pub cache_dir: Option<PathBuf>,
}

#[derive(Debug)]
pub enum TranslatorError {
    /// `RuntimeException("JSON Error in: <file>")`.
    Json { file: PathBuf, cause: String },
    /// `Core.IS_DEV` source checks failed (the original calls `System.exit(1)`).
    DevCheck(Vec<String>),
}

impl std::fmt::Display for TranslatorError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TranslatorError::Json { file, cause } => {
                write!(f, "JSON Error in: {}: {cause}", file.display())
            }
            TranslatorError::DevCheck(m) => write!(f, "{}", m.join("\n")),
        }
    }
}

impl std::error::Error for TranslatorError {}

type Map = HashMap<String, String>;

/// `zombie.core.Translator`.
pub struct Translator {
    pub env: TranslatorEnv,
    pub languages: Languages,
    language: RefCell<Option<Rc<Language>>>,
    available_language: Option<Vec<Rc<Language>>>,
    tables: HashMap<Table, Map>,
    recorded_media_en: Map,
    missing: RefCell<HashSet<String>>,
    br_replacements: RefCell<Map>,
    temp_map: Map,
    azerty_languages: Option<Vec<String>>,
    /// `Translator.debug`: log missing names to `translationProblems.txt`.
    pub debug: bool,
    debug_file: RefCell<Option<File>>,
    debug_item_evolved_recipe_name: HashSet<String>,
    debug_item: HashSet<String>,
    debug_multi_stage_build: HashSet<String>,
    debug_recipe: HashSet<String>,
    debug_recipe_groups: HashSet<String>,
}

impl Translator {
    pub fn new(env: TranslatorEnv) -> Self {
        Translator {
            env,
            languages: Languages::default(),
            language: RefCell::new(None),
            available_language: None,
            tables: BY_NAME.iter().map(|&(_, t)| (t, Map::new())).collect(),
            recorded_media_en: Map::new(),
            missing: RefCell::default(),
            br_replacements: RefCell::default(),
            temp_map: Map::new(),
            azerty_languages: None,
            debug: false,
            debug_file: RefCell::new(None),
            debug_item_evolved_recipe_name: HashSet::new(),
            debug_item: HashSet::new(),
            debug_multi_stage_build: HashSet::new(),
            debug_recipe: HashSet::new(),
            debug_recipe_groups: HashSet::new(),
        }
    }

    /// `Languages.instance.init()` with this translator's file system view.
    pub fn init_languages(&mut self) {
        let (base, mods) = (self.env.base_dir.clone(), self.env.mods.clone());
        self.languages.init(&base, &mods);
    }

    pub fn table(&self, t: Table) -> &Map {
        &self.tables[&t]
    }

    pub fn table_mut(&mut self, t: Table) -> &mut Map {
        self.tables.get_mut(&t).unwrap()
    }

    /// `Translator.loadFiles`. On a JSON error the original throws out of
    /// `loadFiles`, leaving the tables filled up to that file; so does this.
    /// `PerkFactory.initTranslations()`, which the original calls at the
    /// end, belongs to the perk system and is the caller's to run.
    pub fn load_files(&mut self) -> Result<(), TranslatorError> {
        *self.language.borrow_mut() = None;
        self.available_language = None;
        if self.debug {
            if let Some(dir) = &self.env.cache_dir {
                *self.debug_file.borrow_mut() =
                    File::create(dir.join("translationProblems.txt")).ok();
            }
        }
        for (t, m) in self.tables.iter_mut() {
            // `entity` is not cleared by the original.
            if *t != Table::Entity {
                m.clear();
            }
        }
        self.missing.borrow_mut().clear();
        if self.env.core_debug {
            eprintln!("translator: language is {}", self.get_language());
        }
        let base = self.env.base_dir.clone();
        let strict = self.env.is_dev;
        for (name, t) in BY_NAME {
            for lang in self.language_stack() {
                let mut map = std::mem::take(self.table_mut(t));
                let r = self
                    .fill_from_file(&base, name, &mut map, &lang, true, strict)
                    .and_then(|_| self.fill_from_mods(name, &mut map, &lang, strict));
                *self.table_mut(t) = map;
                r?;
            }
        }
        let en = self.default_language();
        let mut map = std::mem::take(&mut self.recorded_media_en);
        let r = self
            .fill_from_file(&base, "Recorded_Media", &mut map, &en, true, strict)
            .and_then(|_| self.fill_from_mods("Recorded_Media", &mut map, &en, strict));
        self.recorded_media_en = map;
        r?;
        if self.debug {
            self.debug_item_evolved_recipe_name.clear();
            self.debug_item.clear();
            self.debug_multi_stage_build.clear();
            self.debug_recipe.clear();
            self.debug_recipe_groups.clear();
        }
        if self.env.is_dev {
            let problems = self.check_placeholders()?;
            if !problems.is_empty() {
                return Err(TranslatorError::DevCheck(problems));
            }
        }
        Ok(())
    }

    /// The `Core.IS_DEV` block of `loadFiles`: malformed placeholders in the
    /// English sources, then placeholders in translations that the English
    /// text does not have. Returns the messages the original prints before
    /// exiting.
    pub fn check_placeholders(&self) -> Result<Vec<String>, TranslatorError> {
        let mut out = Vec::new();
        let base = self.env.base_dir.clone();
        let source = self.default_language();
        let cur = self.get_language();
        let mut source_maps: Vec<(&str, Map)> = Vec::new();
        for (name, _) in BY_NAME {
            let mut map = Map::new();
            self.fill_from_file(&base, name, &mut map, &source, false, true)?;
            for (key, value) in &map {
                for p in extract_placeholders(value) {
                    if !is_valid_placeholder(&p) {
                        out.push(format!(
                            "workdir/media/lua/Translate/{}/{}.json key \"{}\" has malformed \"{}\" in \"{}\"",
                            cur.name, name, key, p, value
                        ));
                    }
                }
            }
            source_maps.push((name, map));
        }
        if !out.is_empty() {
            out.push(format!(
                "Found {} problems with our english translation source files",
                out.len()
            ));
            return Ok(out);
        }
        for lang in self.languages.languages() {
            if Rc::ptr_eq(lang, &source) {
                continue;
            }
            for (name, source_map) in &source_maps {
                let mut map = Map::new();
                self.fill_from_file(&base, name, &mut map, lang, false, true)?;
                for (key, english) in source_map {
                    let Some(translated) = map.get(key) else {
                        continue;
                    };
                    if is_null_or_whitespace(Some(translated)) {
                        continue;
                    }
                    let mut found = extract_placeholders(translated);
                    for p in &found {
                        if !is_valid_placeholder(p) {
                            out.push(format!(
                                "workdir/media/lua/Translate/{}/{}.json key \"{}\" has malformed \"{}\" in \"{}\"",
                                lang.name, name, key, p, translated
                            ));
                        }
                    }
                    let expected = extract_placeholders(english);
                    found.retain(|p| !expected.contains(p) && is_valid_placeholder(p) && p != "%%");
                    if !found.is_empty() {
                        let mut list: Vec<_> = found.into_iter().collect();
                        list.sort();
                        out.push(format!(
                            "workdir/media/lua/Translate/{}/{}.json key \"{}\" has unknown [{}] in \"{}\" not found in source \"{}\" ",
                            lang.name, name, key, list.join(", "), translated, english
                        ));
                    }
                }
            }
        }
        if !out.is_empty() {
            out.push(format!("Found {} problems with translations", out.len()));
        }
        Ok(out)
    }

    /// `Translator.forLanguageStack`: the language, its `base` chain, then
    /// the default language, applied from the most basic to the selected one.
    pub fn language_stack(&self) -> Vec<Rc<Language>> {
        let mut bases: Vec<Rc<Language>> = Vec::new();
        let mut lang = Some(self.get_language());
        while let Some(l) = lang {
            if bases.iter().any(|b| Rc::ptr_eq(b, &l)) {
                break;
            }
            bases.push(l.clone());
            lang = l.base.as_deref().and_then(|b| self.languages.by_name(b));
        }
        let def = self.default_language();
        if !bases.iter().any(|b| Rc::ptr_eq(b, &def)) {
            bases.push(def);
        }
        bases.reverse();
        bases
    }

    /// `Translator.tryFillMapFromFile`: `<root>/media/lua/shared/Translate/<LANG>/<file>.json`.
    fn fill_from_file(
        &self,
        root: &Path,
        file_name: &str,
        map: &mut Map,
        language: &Language,
        fix: bool,
        strict: bool,
    ) -> Result<(), TranslatorError> {
        let file = PathBuf::from(format!(
            "{}/media/lua/shared/Translate/{}/{}.json",
            root.display(),
            language.name,
            file_name
        ));
        if !file.exists() {
            return Ok(());
        }
        let err = |cause: String| TranslatorError::Json {
            file: file.clone(),
            cause,
        };
        let content = std::fs::read_to_string(&file).map_err(|e| err(e.to_string()))?;
        if self.env.is_dev && language.name == "EN" && file_name != "Mod" {
            let found: Vec<String> = self
                .env
                .lookalike_chars
                .iter()
                .filter(|c| content.contains(**c))
                .map(|c| format!("'{c}'"))
                .collect();
            if !found.is_empty() {
                return Err(err(format!(
                    "Found look-a-like unicode char: {} in EN translation: {}",
                    found.join(","),
                    file.display()
                )));
            }
        }
        let obj = json::parse_object(&content, strict).map_err(|e| err(e.0))?;
        // `toMap().forEach(...)` casts every value to String: anything else
        // (numbers, booleans, null, nested values) is a ClassCastException
        // or NullPointerException, reported as a JSON error.
        let mut entries = Vec::with_capacity(obj.map.len());
        for (k, v) in obj.map {
            match v {
                json::JsonValue::String(s) => entries.push((k, s)),
                other => return Err(err(format!("value of \"{k}\" is not a string: {other:?}"))),
            }
        }
        for (k, v) in entries {
            if !map.contains_key(&k) || !v.is_empty() {
                let v = if fix { format_fixer(&v) } else { v };
                map.insert(k, v);
            }
        }
        Ok(())
    }

    fn fill_from_mods(
        &self,
        file_name: &str,
        map: &mut Map,
        language: &Language,
        strict: bool,
    ) -> Result<(), TranslatorError> {
        for m in &self.env.mods {
            for dir in [&m.common, &m.version].into_iter().flatten() {
                self.fill_from_file(dir, file_name, map, language, true, strict)?;
            }
        }
        Ok(())
    }

    /// `Translator.readMapTranslation`: a map's translated title and
    /// description (`None` keeps the map's own value).
    pub fn read_map_translation(
        &mut self,
        dir: &str,
    ) -> Result<(Option<String>, Option<String>), TranslatorError> {
        let mut map = Map::new();
        let base = self.env.base_dir.clone();
        for lang in self.language_stack() {
            self.fill_from_file(&base, dir, &mut map, &lang, true, self.env.is_dev)?;
            self.fill_from_mods(dir, &mut map, &lang, self.env.is_dev)?;
        }
        self.temp_map = map;
        Ok((
            self.temp_map.get("title").cloned(),
            self.temp_map.get("description").cloned(),
        ))
    }

    /// `Translator.readModTranslation`: a mod's translated name and description.
    pub fn read_mod_translation(
        &mut self,
        m: &ModDirs,
    ) -> Result<(Option<String>, Option<String>), TranslatorError> {
        let mut map = Map::new();
        for lang in self.language_stack() {
            for dir in [&m.common, &m.version].into_iter().flatten() {
                self.fill_from_file(dir, "Mod", &mut map, &lang, true, self.env.is_dev)?;
            }
        }
        self.temp_map = map;
        Ok((
            self.temp_map.get("name").cloned(),
            self.temp_map.get("description").cloned(),
        ))
    }

    fn dbg_prefix(&self) -> bool {
        self.env.core_debug && self.env.translation_prefix
    }

    fn get_text_internal(&self, desc: &str, null_ok: bool) -> Option<String> {
        let table = PREFIXES
            .iter()
            .find(|(p, _)| desc.starts_with(p))
            .map(|&(_, t)| t);
        let mut result = table.and_then(|t| self.tables[&t].get(desc).cloned());
        let mut dbg = if self.dbg_prefix() { Some("*") } else { None };
        if result.is_none() {
            if null_ok {
                return None;
            }
            self.report_missing(desc, self.env.core_debug);
            result = Some(desc.to_string());
            dbg = if self.dbg_prefix() { Some("!") } else { None };
        }
        let result = result.unwrap();
        if result.contains("<br>") || result.contains("<BR>") {
            let mut cache = self.br_replacements.borrow_mut();
            let v = cache
                .entry(result.clone())
                .or_insert_with(|| result.replace("<br>", "\n").replace("<BR>", "\n"));
            return Some(v.clone());
        }
        Some(match dbg {
            Some(p) => format!("{p}{result}"),
            None => result,
        })
    }

    fn report_missing(&self, desc: &str, log: bool) {
        if self.missing.borrow().contains(desc) {
            return;
        }
        if log {
            eprintln!("ERROR: Missing translation \"{desc}\"");
        }
        if self.debug {
            self.debug_write(&format!("ERROR: Missing translation \"{desc}\"\r\n"));
        }
        self.missing.borrow_mut().insert(desc.to_string());
    }

    /// `Translator.getText(desc, args...)`. Errors only in `Core.IS_DEV`,
    /// where the original rethrows non-missing-argument format errors.
    pub fn get_text_checked(&self, desc: &str, args: &[JavaArg]) -> Result<String, FormatError> {
        let text = self.get_text_internal(desc, false).unwrap();
        self.report_missing_arguments_from_past_abuse(desc, text, args)
    }

    /// `Translator.getText(desc, args...)`.
    pub fn get_text(&self, desc: &str, args: &[JavaArg]) -> String {
        let text = self.get_text_internal(desc, false).unwrap();
        self.report_missing_arguments_from_past_abuse(desc, text.clone(), args)
            .unwrap_or(text)
    }

    /// `Translator.getTextOrNull(desc, args...)`.
    pub fn get_text_or_null(&self, desc: &str, args: &[JavaArg]) -> Option<String> {
        let text = self.get_text_internal(desc, true)?;
        Some(
            self.report_missing_arguments_from_past_abuse(desc, text.clone(), args)
                .unwrap_or(text),
        )
    }

    /// `Translator.reportMissingArgumentsFromPastAbuse`: formats an
    /// already looked-up translation `text` for key `desc`.
    pub fn report_missing_arguments_from_past_abuse(
        &self,
        desc: &str,
        text: String,
        args: &[JavaArg],
    ) -> Result<String, FormatError> {
        let args = fixup_args(args);
        match jfmt::format(&text, &args) {
            Ok(s) => Ok(s),
            Err(e) if e.is_missing_argument() => {
                eprintln!("ERROR: Missing arguments for \"{desc}\"");
                Ok(text)
            }
            Err(e) => {
                eprintln!("ERROR: Formatting \"{desc}\"");
                if self.env.is_dev {
                    return Err(e);
                }
                let fixed = replace_digit_escapes(&escape_stray_percents(&text));
                Ok(jfmt::format(&fixed, &args).unwrap_or(text))
            }
        }
    }

    /// `Translator.setLanguage(Language)`; `None` selects the default.
    pub fn set_language(&mut self, lang: Option<Rc<Language>>) {
        *self.language.borrow_mut() = Some(lang.unwrap_or_else(|| self.default_language()));
    }

    /// `Translator.setLanguage(int)`.
    pub fn set_language_index(&mut self, id: usize) {
        let l = self.languages.languages()[id].clone();
        self.set_language(Some(l));
    }

    /// `Translator.getLanguage()`: the selected language, else the options'
    /// language, else the system language, else English.
    pub fn get_language(&self) -> Rc<Language> {
        let mut cur = self.language.borrow_mut();
        if cur.is_none() && !is_null_or_whitespace(self.env.option_language_name.as_deref()) {
            *cur = self
                .languages
                .by_name(self.env.option_language_name.as_deref().unwrap());
        }
        if cur.is_none() {
            *cur = self
                .languages
                .by_name(&self.env.user_language.to_uppercase());
        }
        if cur.is_none() {
            *cur = Some(self.default_language());
        }
        cur.clone().unwrap()
    }

    pub fn get_available_language(&mut self) -> &[Rc<Language>] {
        if self.available_language.is_none() {
            self.available_language = Some(self.languages.languages().to_vec());
        }
        self.available_language.as_deref().unwrap()
    }

    pub fn default_language(&self) -> Rc<Language> {
        self.languages.default_language()
    }

    fn is_default_language(&self) -> bool {
        Rc::ptr_eq(&self.get_language(), &self.default_language())
    }

    /// `Translator.getDisplayItemName`.
    pub fn get_display_item_name(&self, trim: &str) -> String {
        self.tables[&Table::Items]
            .get(&trim.replace([' ', '-'], "_"))
            .cloned()
            .unwrap_or_else(|| trim.to_string())
    }

    /// `Translator.getItemNameFromFullType`. `script_display_name` is
    /// `ScriptManager.instance.getItem(fullType)?.getDisplayName()`.
    pub fn get_item_name_from_full_type(
        &mut self,
        full_type: &str,
        script_display_name: impl FnOnce(&str) -> Option<String>,
    ) -> String {
        assert!(
            full_type.contains('.'),
            "fullType must contain \".\" i.e. module.type"
        );
        if let Some(n) = self.tables[&Table::ItemName].get(full_type) {
            return n.clone();
        }
        if self.debug && !self.is_default_language() {
            self.debug_item.insert(full_type.to_string());
        }
        let name = script_display_name(full_type).unwrap_or_else(|| full_type.to_string());
        self.table_mut(Table::ItemName)
            .insert(full_type.to_string(), name.clone());
        name
    }

    /// `Translator.setDefaultItemEvolvedRecipeName`.
    pub fn set_default_item_evolved_recipe_name(&mut self, full_type: &str, english: &str) {
        if !self.is_default_language() {
            return;
        }
        assert!(
            full_type.contains('.'),
            "fullType must contain \".\" i.e. module.type"
        );
        self.table_mut(Table::EvolvedRecipeName)
            .entry(full_type.to_string())
            .or_insert_with(|| english.to_string());
    }

    /// `Translator.getItemEvolvedRecipeName`.
    pub fn get_item_evolved_recipe_name(
        &mut self,
        full_type: &str,
        script_display_name: impl FnOnce(&str) -> Option<String>,
    ) -> String {
        assert!(
            full_type.contains('.'),
            "fullType must contain \".\" i.e. module.type"
        );
        if let Some(n) = self.tables[&Table::EvolvedRecipeName].get(full_type) {
            return n.clone();
        }
        if self.debug && !self.is_default_language() {
            self.debug_item_evolved_recipe_name
                .insert(full_type.to_string());
        }
        let name = script_display_name(full_type).unwrap_or_else(|| full_type.to_string());
        self.table_mut(Table::EvolvedRecipeName)
            .insert(full_type.to_string(), name.clone());
        name
    }

    /// `Translator.getMoveableDisplayName(IsoObject)`, given the object's
    /// `GroupName` and `CustomName` properties (`None` when it has none).
    pub fn get_moveable_display_name_for(
        &self,
        group_name: Option<&str>,
        custom_name: Option<&str>,
    ) -> Option<String> {
        let custom = custom_name.filter(|s| !s.is_empty())?;
        let name = match group_name.filter(|s| !s.is_empty()) {
            Some(g) => format!("{g} {custom}"),
            None => custom.to_string(),
        };
        Some(self.get_moveable_display_name(&name))
    }

    fn moveable_key(name: &str) -> String {
        // `replace("\\.", "")` is a literal replace of backslash-dot.
        name.replace([' ', '-'], "_")
            .replace('\'', "")
            .replace("\\.", "")
    }

    /// `Translator.getMoveableDisplayName(String)`.
    pub fn get_moveable_display_name(&self, name: &str) -> String {
        match self.tables[&Table::Moveables].get(&Self::moveable_key(name)) {
            None if self.dbg_prefix() => format!("!{name}"),
            None => name.to_string(),
            Some(r) if self.dbg_prefix() => format!("*{r}"),
            Some(r) => r.clone(),
        }
    }

    /// `Translator.getMoveableDisplayNameOrNull`.
    pub fn get_moveable_display_name_or_null(&self, name: &str) -> Option<String> {
        let r = self.tables[&Table::Moveables].get(&Self::moveable_key(name))?;
        Some(if self.dbg_prefix() {
            format!("*{r}")
        } else {
            r.clone()
        })
    }

    /// `Translator.getRecipeName`.
    pub fn get_recipe_name(&mut self, name: &str) -> String {
        match self.tables[&Table::Recipes].get(name) {
            Some(r) if !r.is_empty() => r.clone(),
            _ => {
                if self.debug && !self.is_default_language() {
                    self.debug_recipe.insert(name.to_string());
                }
                name.to_string()
            }
        }
    }

    /// `Translator.getRecipeGroupName`.
    pub fn get_recipe_group_name(&mut self, name: &str) -> String {
        match self.tables[&Table::RecipeGroups].get(name) {
            Some(r) if !r.is_empty() => r.clone(),
            _ => {
                if self.debug && !self.is_default_language() {
                    self.debug_recipe_groups.insert(name.to_string());
                }
                name.to_string()
            }
        }
    }

    /// `Translator.getAzertyMap`.
    pub fn get_azerty_map(&mut self) -> &[String] {
        self.azerty_languages
            .get_or_insert_with(|| vec!["FR".to_string()])
    }

    /// `Translator.getTextMediaEN`: recorded-media text in English.
    pub fn get_text_media_en(&self, desc: &str) -> String {
        let mut result = if desc.starts_with("RM_") {
            self.recorded_media_en.get(desc).cloned()
        } else {
            None
        };
        let mut dbg = if self.dbg_prefix() { Some("*") } else { None };
        if result.is_none() {
            if self.env.core_debug {
                self.report_missing(desc, true);
            }
            result = Some(desc.to_string());
            dbg = if self.dbg_prefix() { Some("!") } else { None };
        }
        let result = result.unwrap();
        if result.contains("<br>") {
            return result.replace("<br>", "\n");
        }
        match dbg {
            Some(p) => format!("{p}{result}"),
            None => result,
        }
    }

    /// `Translator.getAttributeText` / `getAttributeTextOrNull`.
    pub fn get_attribute_text(&self, s: &str, null_on_fail: bool) -> Option<String> {
        match self.tables[&Table::Attributes].get(s) {
            Some(r) => Some(r.clone()),
            None => {
                self.report_missing(s, true);
                if null_on_fail {
                    None
                } else {
                    Some(s.to_string())
                }
            }
        }
    }

    /// `Translator.getFluidText`.
    pub fn get_fluid_text(&self, s: &str) -> String {
        self.tables[&Table::Fluids]
            .get(s)
            .cloned()
            .unwrap_or_else(|| s.to_string())
    }

    /// `Translator.getEntityText`.
    pub fn get_entity_text(&self, s: &str) -> String {
        self.tables[&Table::Entity]
            .get(s)
            .cloned()
            .unwrap_or_else(|| s.to_string())
    }

    /// `Translator.getMapLabelText`.
    pub fn get_map_label_text(&self, s: &str) -> String {
        self.tables[&Table::MapLabel]
            .get(s)
            .cloned()
            .unwrap_or_else(|| s.to_string())
    }

    /// `Translator.getUI`.
    pub fn get_ui(&self) -> &Map {
        &self.tables[&Table::Ui]
    }

    fn debug_write(&self, s: &str) {
        if let Some(f) = self.debug_file.borrow_mut().as_mut() {
            let _ = f.write_all(s.as_bytes());
            let _ = f.flush();
        }
    }

    fn debug_names(&self, set: &HashSet<String>, header: &str, line: impl Fn(&str) -> String) {
        if !self.debug || set.is_empty() {
            return;
        }
        self.debug_write(&format!("{header}_{}.txt\r\n", self.get_language()));
        let mut sorted: Vec<&String> = set.iter().collect();
        sorted.sort_by(|a, b| java_compare(a, b));
        for name in sorted {
            self.debug_write(&line(name));
        }
    }

    /// `Translator.debugItemEvolvedRecipeNames`.
    pub fn debug_item_evolved_recipe_names(&mut self) {
        let set = std::mem::take(&mut self.debug_item_evolved_recipe_name);
        let t = &self.tables[&Table::EvolvedRecipeName];
        self.debug_names(&set, "EvolvedRecipeName", |n| {
            format!(
                "\tEvolvedRecipeName_{n} = \"{}\",\r\n",
                t.get(n).map_or("null", String::as_str)
            )
        });
    }

    /// `Translator.debugItemNames`.
    pub fn debug_item_names(&mut self) {
        let set = std::mem::take(&mut self.debug_item);
        let t = &self.tables[&Table::ItemName];
        self.debug_names(&set, "ItemName", |n| {
            format!(
                "\tItemName_{n} = \"{}\",\r\n",
                t.get(n).map_or("null", String::as_str)
            )
        });
    }

    /// `Translator.debugMultiStageBuildNames`.
    pub fn debug_multi_stage_build_names(&mut self) {
        let set = std::mem::take(&mut self.debug_multi_stage_build);
        self.debug_names(&set, "MultiStageBuild", |n| {
            format!("\tMultiStageBuild_{n} = \"\",\r\n")
        });
    }

    /// `Translator.debugRecipeNames`.
    pub fn debug_recipe_names(&mut self) {
        let set = std::mem::take(&mut self.debug_recipe);
        self.debug_names(&set, "Recipes", |n| {
            format!("\tRecipe_{} = \"\",\r\n", n.replace(' ', "_"))
        });
    }

    /// `Translator.debugRecipeGroupNames`.
    pub fn debug_recipe_group_names(&mut self) {
        let set = std::mem::take(&mut self.debug_recipe_groups);
        self.debug_names(&set, "RecipeGroups", |n| {
            format!("\tRecipeGroup_{} = \"\",\r\n", n.replace(' ', "_"))
        });
    }
}

/// `String.compareTo` (UTF-16 code unit order).
pub(crate) fn java_compare(a: &str, b: &str) -> std::cmp::Ordering {
    a.encode_utf16().cmp(b.encode_utf16())
}

/// `Translator.fixupArgs`: `null` becomes "", whole `Double`s their `long`
/// digits.
pub fn fixup_args(args: &[JavaArg]) -> Vec<JavaArg> {
    args.iter()
        .map(|a| match a {
            JavaArg::Null => JavaArg::Str(String::new()),
            JavaArg::Double(d) if *d == (*d as i64) as f64 => JavaArg::Str((*d as i64).to_string()),
            other => other.clone(),
        })
        .collect()
}

/// `Translator.formatFixer`: `%N` becomes `%N$s`, `%%` stays.
pub fn format_fixer(input: &str) -> String {
    let mut out = String::with_capacity(input.len() + 8);
    let mut it = input.chars().peekable();
    while let Some(c) = it.next() {
        if c != '%' {
            out.push(c);
            continue;
        }
        match it.peek() {
            Some('%') => {
                it.next();
                out.push_str("%%");
            }
            Some(&d @ '1'..='9') => {
                it.next();
                out.push('%');
                out.push(d);
                out.push_str("$s");
            }
            _ => out.push('%'),
        }
    }
    out
}

/// `text.replaceAll("%(?=[^ds.]|$)", "%%")`.
fn escape_stray_percents(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len() + 8);
    for (i, &c) in chars.iter().enumerate() {
        out.push(c);
        if c == '%' && !matches!(chars.get(i + 1), Some('d' | 's' | '.')) {
            out.push('%');
        }
    }
    out
}

/// `text.replaceAll("%%(\\d+)", "%$1\\$s")`. `\d` is ASCII digits.
fn replace_digit_escapes(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '%'
            && chars.get(i + 1) == Some(&'%')
            && chars.get(i + 2).is_some_and(char::is_ascii_digit)
        {
            let mut j = i + 2;
            while j < chars.len() && chars[j].is_ascii_digit() {
                j += 1;
            }
            out.push('%');
            out.extend(&chars[i + 2..j]);
            out.push_str("$s");
            i = j;
        } else {
            out.push(chars[i]);
            i += 1;
        }
    }
    out
}

/// `Translator.extractPlaceholders`: every `%` and the char after it.
fn extract_placeholders(text: &str) -> HashSet<String> {
    let mut out = HashSet::new();
    let mut it = text.chars();
    while let Some(c) = it.next() {
        if c == '%' {
            let mut p = String::from('%');
            if let Some(n) = it.next() {
                p.push(n);
            }
            out.insert(p);
        }
    }
    out
}

/// `Translator.isValidPlaceholder`: `%%` or `%1`..`%9`.
fn is_valid_placeholder(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() == 2 && b[0] == b'%' && (b[1] == b'%' || (b'1'..=b'9').contains(&b[1]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_fixer_matches_regex() {
        assert_eq!(format_fixer("%1 of %2, 5%% %x %"), "%1$s of %2$s, 5%% %x %");
        assert_eq!(format_fixer("%%1"), "%%1");
    }

    #[test]
    fn fallback_regexes() {
        assert_eq!(
            escape_stray_percents("50% off %d %s %.1f 5%"),
            "50%% off %d %s %.1f 5%%"
        );
        assert_eq!(replace_digit_escapes("%%12 %%%%1"), "%12$s %%%1$s");
    }

    #[test]
    fn fixup() {
        let a = fixup_args(&[JavaArg::Null, JavaArg::Double(3.0), JavaArg::Double(2.5)]);
        assert_eq!(
            a,
            vec![
                JavaArg::Str("".into()),
                JavaArg::Str("3".into()),
                JavaArg::Double(2.5)
            ]
        );
    }

    #[test]
    fn get_text_formats_like_the_original() {
        let mut t = Translator::new(TranslatorEnv::default());
        let ui = t.table_mut(Table::Ui);
        ui.insert("UI_a".into(), format_fixer("Hello %1, %2%%"));
        ui.insert("UI_b".into(), "5% luck".into());
        ui.insert("UI_c".into(), "x<br>y".into());
        assert_eq!(
            t.get_text("UI_a", &[JavaArg::Str("Bob".into()), JavaArg::Double(5.0)]),
            "Hello Bob, 5%"
        );
        // Missing argument: the unformatted text comes back.
        assert_eq!(t.get_text("UI_a", &[]), "Hello %1$s, %2$s%%");
        // Illegal format: stray percents are escaped and formatting retried.
        assert_eq!(t.get_text("UI_b", &[]), "5% luck");
        assert_eq!(t.get_text("UI_c", &[]), "x\ny");
        assert_eq!(t.get_text("UI_missing", &[]), "UI_missing");
        assert_eq!(t.get_text_or_null("UI_missing", &[]), None);
        assert_eq!(t.get_text("Nope", &[]), "Nope");
    }
}
