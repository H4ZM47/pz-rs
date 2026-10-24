//! `zombie.gameStates.ChooseGameInfo.Mod` (mod.info), `zombie.core.GameVersion`
//! and `zombie.modding.ActiveModsFile` (`mods/default.txt`).

use crate::java;
use crate::script_parser;

/// `zombie.core.GameVersion`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GameVersion {
    pub major: i32,
    pub minor: i32,
    pub suffix: String,
}

impl GameVersion {
    /// The constructor's checks: major >= 0, minor in 0..=999.
    pub fn new(major: i32, minor: i32, suffix: &str) -> Result<Self, String> {
        if major < 0 {
            return Err("major version must be greater than zero".into());
        }
        if !(0..=999).contains(&minor) {
            return Err("minor version must be from 0 to 999".into());
        }
        Ok(GameVersion { major, minor, suffix: suffix.to_string() })
    }

    /// `Core.gameVersion` of the build this port targets.
    pub fn current() -> Self {
        GameVersion { major: 42, minor: 21, suffix: String::new() }
    }

    /// `ChooseGameInfo.minRequiredVersion`.
    pub fn min_required() -> Self {
        GameVersion { major: 42, minor: 0, suffix: String::new() }
    }

    pub fn as_int(&self) -> i32 {
        self.major * 1000 + self.minor
    }

    /// `GameVersion.parse`: regex `([0-9]+)\.([0-9]+)(.*)`, whole string.
    pub fn parse(s: &str) -> Result<Self, String> {
        let err = || format!("invalid game version \"{s}\"");
        let b = s.as_bytes();
        let major_end = b.iter().position(|c| !c.is_ascii_digit()).unwrap_or(b.len());
        if major_end == 0 || b.get(major_end) != Some(&b'.') {
            return Err(err());
        }
        let minor_start = major_end + 1;
        let minor_len = b[minor_start..].iter().position(|c| !c.is_ascii_digit()).unwrap_or(b.len() - minor_start);
        if minor_len == 0 {
            return Err(err());
        }
        let suffix = &s[minor_start + minor_len..];
        // `.` does not match Java's line terminators.
        if suffix.chars().any(|c| matches!(c, '\n' | '\r' | '\u{85}' | '\u{2028}' | '\u{2029}')) {
            return Err(err());
        }
        let major = java::try_parse_int(&s[..major_end], 0);
        let minor = java::try_parse_int(&s[minor_start..minor_start + minor_len], 0);
        GameVersion::new(major, minor, suffix)
    }
}

impl std::fmt::Display for GameVersion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{}{}", self.major, self.minor, self.suffix)
    }
}

/// `ZomboidFileSystem.getGameVersionIntFromName`.
pub fn game_version_int_from_name(name: &str) -> i32 {
    // `name.split("\\.")`: same trailing-empty rules as a literal split.
    let parts = java::split_char(name, '.');
    match parts.len() {
        1 => java::try_parse_int(&parts[0], 0) * 1000,
        n if n >= 2 => java::try_parse_int(&parts[0], 0) * 1000 + java::try_parse_int(&parts[1], 0).min(999),
        _ => 0,
    }
}

/// `ChooseGameInfo.PackFile`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PackFile {
    pub name: String,
    pub flags: i32,
}

/// `ChooseGameInfo.TileDef`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TileDef {
    pub name: String,
    pub file_number: i32,
}

/// `ChooseGameInfo.Mod`.
#[derive(Clone, Debug)]
pub struct ModInfo {
    pub dir: String,
    pub common_dir: String,
    pub version_dir: String,
    pub posters: Vec<String>,
    pub require: Option<Vec<String>>,
    pub incompatible: Option<Vec<String>>,
    pub load_after: Option<Vec<String>>,
    pub load_before: Option<Vec<String>>,
    pub name: String,
    pub description: String,
    pub id: String,
    pub url: Option<String>,
    pub author: Option<String>,
    pub mod_version: Option<String>,
    pub icon: Option<String>,
    pub category: Option<String>,
    pub workshop_id: String,
    pub source: String,
    pub version_min: Option<GameVersion>,
    pub version_max: Option<GameVersion>,
    pub packs: Vec<PackFile>,
    pub tile_defs: Vec<TileDef>,
    pub(crate) available: Option<bool>,
    pub(crate) read: bool,
    pub(crate) valid: bool,
}

impl ModInfo {
    /// `new Mod(dir)`; `version_dir_name` is `getModVersionDirName(dir)`.
    pub fn new(dir: &str, common_dir: String, version_dir: String) -> Self {
        let mut source = "Mods".to_string();
        let mut workshop_id = String::new();
        if let Some(root) = grandparent_name(dir) {
            if root.eq_ignore_ascii_case("Contents") {
                source = "Workshop".into();
            } else if is_valid_steam_id(&root) {
                source = "Steam".into();
                workshop_id = root;
            }
        }
        ModInfo {
            dir: dir.to_string(),
            common_dir,
            version_dir,
            posters: Vec::new(),
            require: None,
            incompatible: None,
            load_after: None,
            load_before: None,
            name: "Unnamed Mod".into(),
            description: String::new(),
            id: "undefined_id".into(),
            url: None,
            author: None,
            mod_version: None,
            icon: None,
            category: None,
            workshop_id,
            source,
            version_min: None,
            version_max: None,
            packs: Vec::new(),
            tile_defs: Vec::new(),
            available: None,
            read: false,
            valid: false,
        }
    }

    /// `Mod.isAvailableSelf`.
    pub fn is_available_self(&self, game: &GameVersion) -> bool {
        if self.version_min.as_ref().is_some_and(|v| v.as_int() > game.as_int()) {
            return false;
        }
        self.version_max.as_ref().is_none_or(|v| v.as_int() >= game.as_int())
    }

    /// `Mod.setUrl`: only Indie Stone / Steam URLs are kept.
    pub fn set_url(&mut self, url: &str) {
        if is_indie_stone_url(url) {
            self.url = Some(url.to_string());
        }
    }

    /// Applies the lines of a mod.info file (`readModInfoAux`'s loop).
    /// `exists(path)` checks whether a poster/icon path exists. Returns
    /// `false` where the original aborts and returns null.
    pub(crate) fn apply_mod_info(&mut self, text: &str, texture_compression: bool, exists: impl Fn(&str) -> bool) -> bool {
        let sep = std::path::MAIN_SEPARATOR;
        let (version_dir, common_dir) = (self.version_dir.clone(), self.common_dir.clone());
        let with_fallback = |value: &str| -> String {
            let p = format!("{version_dir}{sep}{value}");
            if exists(&p) { p } else { format!("{common_dir}{sep}{value}") }
        };
        let list = |line: &str, key: &str| -> Vec<String> {
            java::split_char(&line.replace(key, "").replace('\\', ""), ',')
        };
        for line in java::read_lines(text) {
            if line.contains("name=") {
                let v = line.replace("name=", "");
                if !java::is_null_or_whitespace(&v) {
                    self.name = v;
                }
            } else if line.contains("poster=") {
                let v = line.replace("poster=", "");
                if !java::is_null_or_whitespace(&v) {
                    let p = with_fallback(&v);
                    self.posters.push(p);
                }
            } else if line.contains("description=") {
                self.description.push_str(&line.replace("description=", ""));
            } else if line.contains("require=") {
                self.require = Some(list(line, "require="));
            } else if line.contains("incompatible=") {
                self.incompatible = Some(list(line, "incompatible="));
            } else if line.contains("loadModAfter=") {
                self.load_after = Some(list(line, "loadModAfter="));
            } else if line.contains("loadModBefore=") {
                self.load_before = Some(list(line, "loadModBefore="));
            } else if line.starts_with("id=") {
                let v = line.replace("id=", "");
                let v = java::trim(&v);
                if !java::is_null_or_whitespace(v) {
                    self.id = v.to_string();
                }
            } else if line.contains("author=") {
                self.author = Some(line.replace("author=", ""));
            } else if line.contains("modversion=") {
                self.mod_version = Some(line.replace("modversion=", ""));
            } else if line.contains("icon=") {
                let v = line.replace("icon=", "");
                if !java::is_null_or_whitespace(&v) {
                    let p = with_fallback(&v);
                    self.icon = Some(p);
                }
            } else if line.contains("category=") {
                self.category = Some(line.replace("category=", ""));
            } else if line.contains("url=") {
                let v = line.replace("url=", "");
                self.set_url(&v);
            } else if line.contains("pack=") {
                let mut name = java::trim(&line.replace("pack=", "")).to_string();
                if name.is_empty() {
                    eprintln!("pack= line requires a file name");
                    return false;
                }
                let mut flags = if texture_compression { 4 } else { 0 };
                flags |= 64;
                if let Some(idx) = name.find("type=") {
                    let kind = &name[idx + "type=".len()..];
                    if kind == "ui" {
                        flags = 2;
                    } else {
                        eprintln!("unknown pack type \"{kind}\"");
                    }
                    // `substring(0, indexOf(' '))` throws when there is no space,
                    // which aborts reading the whole file.
                    let Some(space) = name.find(' ') else { return false };
                    name = java::trim(&name[..space]).to_string();
                }
                if name.ends_with(".floor") {
                    flags &= !4;
                }
                self.packs.push(PackFile { name, flags });
            } else if line.contains("tiledef=") {
                let ss = java::split_ws(java::trim(&line.replace("tiledef=", "")));
                if ss.len() != 2 {
                    eprintln!("tiledef= line requires file name and file number");
                    return false;
                }
                let Some(n) = java::parse_int(&ss[1]) else {
                    eprintln!("tiledef= line requires file name and file number");
                    return false;
                };
                if !(100..=8189).contains(&n) {
                    eprintln!("tiledef={} {n} file number must be from 100 to 8189", ss[0]);
                    return false;
                }
                self.tile_defs.push(TileDef { name: ss[0].clone(), file_number: n });
            } else if line.starts_with("versionMax=") {
                let v = line.replace("versionMax=", "");
                let v = java::trim(&v);
                if v.is_empty() {
                    continue;
                }
                match GameVersion::parse(v) {
                    Ok(gv) => self.version_max = Some(gv),
                    Err(e) => {
                        eprintln!("{e}");
                        return false;
                    }
                }
            } else if line.starts_with("versionMin=") {
                let v = line.replace("versionMin=", "");
                let v = java::trim(&v);
                if v.is_empty() {
                    continue;
                }
                match GameVersion::parse(v) {
                    Ok(gv) => self.version_min = Some(gv),
                    Err(e) => {
                        eprintln!("{e}");
                        return false;
                    }
                }
            }
        }
        true
    }
}

/// `Paths.get(dir, "..", "..").normalize().getFileName()`.
fn grandparent_name(dir: &str) -> Option<String> {
    let mut parts: Vec<&str> = dir.split(['/', std::path::MAIN_SEPARATOR]).filter(|p| !p.is_empty() && *p != ".").collect();
    for _ in 0..2 {
        match parts.last() {
            Some(&"..") | None => parts.push(".."),
            Some(_) => {
                parts.pop();
            }
        }
    }
    match parts.last() {
        Some(&"..") | None => None,
        Some(p) => Some(p.to_string()),
    }
}

/// `SteamUtils.isValidSteamID`: a `BigInteger` in `0..=2^64-1`.
pub fn is_valid_steam_id(s: &str) -> bool {
    let digits = match s.as_bytes().first() {
        Some(b'+') => &s[1..],
        Some(b'-') => {
            // Negative values fail, except "-0" which BigInteger reads as 0.
            let d = &s[1..];
            return !d.is_empty() && d.bytes().all(|b| b == b'0');
        }
        _ => s,
    };
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return false;
    }
    let trimmed = digits.trim_start_matches('0');
    trimmed.len() < 20 || (trimmed.len() == 20 && trimmed <= "18446744073709551615")
}

/// `LuaManager.isIndieStoneUrl`.
pub fn is_indie_stone_url(url: &str) -> bool {
    ["https://steamcommunity.com", "https://projectzomboid.com", "https://theindiestone.com", "https://pzwiki.net"]
        .iter()
        .any(|base| url == *base || url.starts_with(&format!("{base}/")))
}

/// `zombie.modding.ActiveMods` contents.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ActiveMods {
    pub mods: Vec<String>,
    pub map_order: Vec<String>,
}

/// `ActiveModsFile.fromString`. Lines are joined without separators by the
/// caller, as `read` does.
pub fn active_mods_from_string(contents: &str) -> ActiveMods {
    let mut out = ActiveMods::default();
    let block = script_parser::parse(&script_parser::strip_comments(contents));
    let version = block.value("VERSION").map_or(-1, |v| java::try_parse_int(v.value(), -1));
    if version != 1 {
        return out;
    }
    if let Some(mods) = block.block("mods", None) {
        for v in &mods.values {
            let id = java::trim(v.value()).replace('\\', "");
            if java::trim(v.key()).eq_ignore_ascii_case("mod") && !java::is_null_or_whitespace(&id) {
                out.mods.push(id);
            }
        }
    }
    if let Some(maps) = block.block("maps", None) {
        for v in &maps.values {
            let map = java::trim(v.value());
            if java::trim(v.key()).eq_ignore_ascii_case("map") && !java::is_null_or_whitespace(map) {
                out.map_order.push(map.to_string());
            }
        }
    }
    out
}

/// `ActiveModsFile.read`: `None` when the file is missing or unreadable.
pub fn read_active_mods(path: &std::path::Path) -> Option<ActiveMods> {
    let bytes = std::fs::read(path).ok()?;
    // `FileReader` uses the platform charset (UTF-8 on current JDKs).
    let text = java::decode_utf8(&bytes);
    Some(active_mods_from_string(&java::read_lines(&text).concat()))
}

/// `ActiveModsFile.toString`.
pub fn active_mods_to_string(mods: &ActiveMods) -> String {
    let mut block = script_parser::Block::default();
    block.set_value("VERSION", "1");
    let m = block.add_block("mods", None);
    for id in &mods.mods {
        m.add_value("mod", id);
    }
    let maps = block.add_block("maps", None);
    for map in &mods.map_order {
        maps.add_value("map", map);
    }
    let eol = if cfg!(windows) { "\r\n" } else { "\n" };
    let mut s = String::new();
    block.pretty_print_elements(0, &mut s, eol, "    ");
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read(text: &str) -> Option<ModInfo> {
        let mut m = ModInfo::new("/m/Mods/x", "/m/Mods/x/common".into(), "/m/Mods/x/42.0".into());
        m.apply_mod_info(text, true, |_| false).then_some(m)
    }

    #[test]
    fn mod_info_fields() {
        let m = read("name=Test\nid= T1 \nrequire=\\A,B,\npack=P\npack=Q.floor\npack=U type=ui\nversionMin=42.0\nposter=poster.png\nurl=https://example.com\n").unwrap();
        assert_eq!(m.name, "Test");
        assert_eq!(m.id, "T1");
        assert_eq!(m.require.as_deref(), Some(&["A".to_string(), "B".to_string()][..]));
        assert_eq!(m.packs, [PackFile { name: "P".into(), flags: 68 }, PackFile { name: "Q.floor".into(), flags: 64 }, PackFile { name: "U".into(), flags: 2 }]);
        assert_eq!(m.version_min, Some(GameVersion { major: 42, minor: 0, suffix: String::new() }));
        assert_eq!(m.posters, [format!("/m/Mods/x/common{}poster.png", std::path::MAIN_SEPARATOR)]);
        assert_eq!(m.url, None);
    }

    #[test]
    fn mod_info_quirks() {
        // "name=" is checked before "description=".
        assert_eq!(read("description=my name=x").unwrap().name, "description=my x");
        // A pack type without a following space aborts the read.
        assert!(read("pack=Ptype=ui").is_none());
        assert!(read("tiledef=a 99").is_none());
        assert!(read("versionMax=42.1000").is_none());
        // A BOM hides a leading id= line from startsWith.
        assert_eq!(read("\u{feff}id=X").unwrap().id, "undefined_id");
    }

    #[test]
    fn versions() {
        assert_eq!(game_version_int_from_name("42.13"), 42013);
        assert_eq!(game_version_int_from_name("42"), 42000);
        assert_eq!(game_version_int_from_name("42.5000"), 42999);
        assert_eq!(game_version_int_from_name("common"), 0);
        assert_eq!(GameVersion::parse("42.0beta").unwrap().to_string(), "42.0beta");
        assert!(GameVersion::parse("42").is_err());
    }

    #[test]
    fn steam_ids_and_sources() {
        assert!(is_valid_steam_id("2392709985"));
        assert!(is_valid_steam_id("18446744073709551615"));
        assert!(!is_valid_steam_id("18446744073709551616"));
        assert!(!is_valid_steam_id("mods"));
        assert_eq!(ModInfo::new("/w/108600/2392709985/mods/X", String::new(), String::new()).source, "Steam");
        assert_eq!(ModInfo::new("/w/Item/Contents/mods/X", String::new(), String::new()).source, "Workshop");
        assert_eq!(ModInfo::new("/u/Zomboid/mods/X", String::new(), String::new()).source, "Mods");
    }

    #[test]
    fn active_mods_round_trip() {
        let mods = ActiveMods { mods: vec!["A".into(), "B".into()], map_order: vec!["Muldraugh, KY".into()] };
        let text = active_mods_to_string(&mods);
        // Map names containing ',' split into two values when read back, as in Java.
        let back = active_mods_from_string(&java::read_lines(&text).concat());
        assert_eq!(back.mods, mods.mods);
        assert_eq!(back.map_order, ["Muldraugh"]);
    }
}
