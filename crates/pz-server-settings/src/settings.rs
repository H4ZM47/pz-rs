//! `zombie.network.ServerSettings` and `ServerSettingsManager`.

use std::path::{Path, PathBuf, MAIN_SEPARATOR};

use pz_classfile::{Jar, Value as Cv};
use pz_sandbox::SandboxOptions;

use crate::options::ServerOptions;
use crate::spawn_regions::{Profession, Region};

/// `FileName.RESERVED_NAMES`, read from the jar (the Windows device names).
pub fn reserved_names(jar: &Jar) -> pz_classfile::Result<Vec<String>> {
    let v = pz_classfile::static_value(jar, "zombie/scripting/FileName", "RESERVED_NAMES")?;
    // new HashSet(Arrays.asList(new String[]{...}))
    fn find(v: &Cv) -> Option<Vec<String>> {
        match v {
            Cv::Array(_) => v
                .as_str_array()
                .map(|a| a.into_iter().map(str::to_string).collect()),
            Cv::New { args, .. } | Cv::Call { args, .. } => args.iter().find_map(find),
            _ => None,
        }
    }
    v.as_ref()
        .and_then(find)
        .ok_or_else(|| pz_classfile::Error::Missing("FileName.RESERVED_NAMES".into()))
}

/// `FileName.sanitize(name)`: invalid file-name characters become '_',
/// trailing dots and whitespace go, and reserved device names get a '_'
/// prefix.
pub fn file_name_sanitize(name: &str, reserved: &[String]) -> String {
    let mut s: String = name
        .chars()
        .map(|c| {
            if matches!(
                c,
                '\\' | '/' | ':' | '*' | '?' | '"' | '<' | '>' | '|' | '\0'..='\u{1f}'
            ) {
                '_'
            } else {
                c
            }
        })
        .collect();
    // [.\s]+$ (\s is ASCII whitespace).
    let trimmed = s
        .trim_end_matches(['.', ' ', '\t', '\n', '\u{b}', '\u{c}', '\r'])
        .len();
    s.truncate(trimmed);
    if s.is_empty() {
        s = "_".into();
    }
    let base = s.split('.').next().unwrap_or("").to_uppercase();
    if reserved.contains(&base) {
        s = format!("_{s}");
    }
    s
}

/// `zombie.network.ServerSettingsManager`.
#[derive(Clone, Debug)]
pub struct ServerSettingsManager {
    /// `getCacheDir()`.
    pub cache_dir: PathBuf,
    pub settings: Vec<String>,
}

/// `LONGEST_SUFFIX_LENGTH`: `"_spawnregions.lua".length()`.
const LONGEST_SUFFIX: usize = "_spawnregions.lua".len();

/// `getSuffixes()`.
pub const SUFFIXES: [&str; 5] = [
    ".ini",
    "_SandboxVars.lua",
    "_spawnpoints.lua",
    "_spawnregions.lua",
    "_zombies.ini",
];

impl ServerSettingsManager {
    pub fn new(cache_dir: impl Into<PathBuf>) -> Self {
        ServerSettingsManager {
            cache_dir: cache_dir.into(),
            settings: Vec::new(),
        }
    }

    /// `getSettingsFolder()`: `<cache>/Server`.
    pub fn settings_folder(&self) -> String {
        format!("{}{MAIN_SEPARATOR}Server", self.cache_dir.display())
    }

    /// `getNameInSettingsFolder(name)`.
    pub fn name_in_settings_folder(&self, name: &str) -> String {
        format!("{}{MAIN_SEPARATOR}{name}", self.settings_folder())
    }

    fn len16(s: &str) -> usize {
        s.encode_utf16().count()
    }

    pub fn settings_folder_path_valid_length(&self, filename: &str) -> bool {
        Self::len16(&self.settings_folder()) + 1 + Self::len16(filename) + LONGEST_SUFFIX < 260
    }

    pub fn settings_folder_name_valid_length(&self, filename: &str) -> bool {
        Self::len16(filename) + LONGEST_SUFFIX < 255
    }

    pub fn settings_folder_valid_length_checks(&self, filename: &str) -> bool {
        self.settings_folder_path_valid_length(filename)
            && self.settings_folder_name_valid_length(filename)
    }

    /// `isValidName(name)`.
    pub fn is_valid_name(name: Option<&str>) -> bool {
        let Some(name) = name.filter(|n| !n.is_empty()) else {
            return false;
        };
        !name.contains(['/', '\\', ':', ';', '"', '.']) && !name.contains("_zombies")
    }

    fn any_files_exist(&self, name: &str) -> bool {
        SUFFIXES.iter().any(|s| {
            Path::new(&format!(
                "{}{MAIN_SEPARATOR}{name}{s}",
                self.settings_folder()
            ))
            .exists()
        })
    }

    /// `isValidNewName(newName)`.
    pub fn is_valid_new_name(&self, new_name: &str, reserved: &[String]) -> bool {
        let s = file_name_sanitize(new_name, reserved);
        Self::is_valid_name(Some(&s)) && !self.any_files_exist(&s)
    }

    /// `readAllSettings()`: the names of every `<name>.ini` (not
    /// `_zombies.ini`) with a valid name, in directory order.
    pub fn read_all_settings(&mut self) {
        self.settings.clear();
        let dir = self.settings_folder();
        if !Path::new(&dir).exists() {
            let _ = std::fs::create_dir_all(&dir);
            return;
        }
        let Ok(rd) = std::fs::read_dir(&dir) else {
            return;
        };
        for e in rd.flatten() {
            let file = e.file_name().to_string_lossy().into_owned();
            if e.path().is_dir() || !file.ends_with(".ini") || file.ends_with("_zombies.ini") {
                continue;
            }
            let name = file.replace(".ini", "");
            if Self::is_valid_name(Some(&name)) {
                self.settings.push(name);
            }
        }
    }
}

/// `zombie.network.ServerSettings`: one named server configuration.
#[derive(Clone, Debug)]
pub struct ServerSettings {
    pub name: String,
    pub server_options: Option<ServerOptions>,
    pub sandbox_options: Option<SandboxOptions>,
    pub spawn_regions: Option<Vec<Region>>,
    pub spawn_points: Option<Vec<Profession>>,
    pub valid: bool,
    pub error_msg: Option<String>,
}

impl ServerSettings {
    /// `new ServerSettings(name)`; `profanity` is
    /// `ProfanityFilter.validateString(name, true, true, true)` and
    /// `error_text` turns its result into the `UI_BadWordCheck` message.
    pub fn new(
        name: &str,
        reserved: &[String],
        profanity: Option<String>,
        error_text: impl FnOnce(&str) -> String,
    ) -> Self {
        let name = file_name_sanitize(name, reserved);
        let valid = profanity.as_deref().is_none_or(str::is_empty);
        let error_msg = (!valid).then(|| error_text(profanity.as_deref().unwrap_or("")));
        ServerSettings {
            name,
            server_options: None,
            sandbox_options: None,
            spawn_regions: None,
            spawn_points: None,
            valid,
            error_msg,
        }
    }

    /// `getSpawnRegionFile(index)`: `file`, else `serverfile`.
    pub fn spawn_region_file(&self, index: usize) -> Option<String> {
        let r = self.spawn_regions.as_ref()?.get(index)?;
        r.file.clone().or_else(|| r.serverfile.clone())
    }

    /// `addSpawnRegion(name, file)`: files under `media` are game files,
    /// anything else a server file.
    pub fn add_spawn_region(&mut self, name: &str, file: &str) {
        let r = if file.starts_with("media") {
            Region {
                name: name.into(),
                file: Some(file.into()),
                ..Default::default()
            }
        } else {
            Region {
                name: name.into(),
                serverfile: Some(file.into()),
                ..Default::default()
            }
        };
        self.spawn_regions.get_or_insert_with(Vec::new).push(r);
    }

    /// Repoints regions whose `serverfile` is this server's points file at
    /// `new_name`'s (`duplicateFiles` / `rename`).
    pub fn retarget_spawn_points(&mut self, new_name: &str) {
        let old = format!("{}_spawnpoints.lua", self.name);
        for r in self.spawn_regions.iter_mut().flatten() {
            if r.serverfile.as_deref() == Some(old.as_str()) {
                r.serverfile = Some(format!("{new_name}_spawnpoints.lua"));
            }
        }
    }

    /// `tryDeleteFile` for each of the server's files (`deleteFiles`).
    pub fn delete_files(&self, mgr: &ServerSettingsManager) -> bool {
        for s in SUFFIXES {
            let p = mgr.name_in_settings_folder(&format!("{}{s}", self.name));
            if Path::new(&p).exists() {
                let _ = std::fs::remove_file(&p);
            }
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_like_java() {
        let reserved = vec!["CON".to_string(), "NUL".to_string()];
        assert_eq!(file_name_sanitize("a/b:c. ", &reserved), "a_b_c");
        assert_eq!(file_name_sanitize("...", &reserved), "_");
        assert_eq!(file_name_sanitize("con.txt", &reserved), "_con.txt");
        assert!(ServerSettingsManager::is_valid_name(Some("servertest")));
        assert!(!ServerSettingsManager::is_valid_name(Some("a_zombies")));
    }
}
