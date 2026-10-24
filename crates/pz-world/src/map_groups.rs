//! `zombie.MapGroups`: groups the installed and modded map directories into
//! worlds by following their `lots=` dependencies, and orders each group.
//!
//! `MapDirectory` objects are compared by identity in the original; here they
//! live in `real_directories` and are referred to by index.

use std::path::{Path, PathBuf};

use crate::java::{hashmap_order, java_trim};

/// The parts of `zombie.modding.ActiveMods` map grouping reads.
#[derive(Clone, Debug, Default)]
pub struct ActiveModsView {
    /// `getMods()`, in load order.
    pub mods: Vec<String>,
    /// `getMapOrder()`.
    pub map_order: Vec<String>,
}

/// Mod folders for `ChooseGameInfo.getAvailableModDetails(id)`:
/// `(getCommonDir(), getVersionDir())`, or `None` if the mod is unavailable.
pub type ModDirs<'a> = &'a dyn Fn(&str) -> Option<(String, String)>;

#[derive(Clone, Debug)]
struct MapDirectory {
    name: String,
    path: String,
    lot_dirs: Vec<String>,
    conflicts: Vec<String>,
}

#[derive(Clone, Debug, Default)]
struct MapGroup {
    /// Indices into `real_directories`.
    directories: Vec<usize>,
}

#[derive(Clone, Debug, Default)]
pub struct MapGroups {
    groups: Vec<MapGroup>,
    real_directories: Vec<MapDirectory>,
}

/// `File.list()`: entry names in directory order, `None` if not a directory.
fn list(dir: &Path) -> Option<Vec<String>> {
    let rd = std::fs::read_dir(dir).ok()?;
    Some(
        rd.flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect(),
    )
}

/// `getVanillaMapDirectories(includeChallenges)`.
pub fn vanilla_map_directories(maps_dir: &Path, include_challenges: bool) -> Vec<String> {
    let mut result = Vec::new();
    let Some(names) = list(maps_dir) else {
        return result;
    };
    for name in names {
        if name.eq_ignore_ascii_case("challengemaps") {
            if !include_challenges {
                continue;
            }
            // Directories containing a map.info, in directory-stream order.
            if let Ok(rd) = std::fs::read_dir(maps_dir.join(&name)) {
                for e in rd.flatten() {
                    let p = e.path();
                    if p.is_dir() && p.join("map.info").exists() {
                        result.push(format!("{name}/{}", e.file_name().to_string_lossy()));
                    }
                }
            }
            continue;
        }
        result.push(name);
    }
    result
}

/// `MapGroups.addMissingVanillaDirectories(mapName)`.
pub fn add_missing_vanilla_directories(maps_dir: &Path, map_name: &str) -> String {
    let vanilla = vanilla_map_directories(maps_dir, false);
    let parts: Vec<&str> = map_name.split(';').collect();
    let uses_vanilla = parts
        .iter()
        .map(|n| java_trim(n))
        .any(|n| !n.is_empty() && vanilla.iter().any(|v| v == n));
    if !uses_vanilla {
        return map_name.to_string();
    }
    let mut names: Vec<String> = parts
        .iter()
        .map(|n| java_trim(n))
        .filter(|n| !n.is_empty())
        .map(str::to_string)
        .collect();
    for v in vanilla {
        if !names.contains(&v) {
            names.push(v);
        }
    }
    names.join(";")
}

/// `getLotDirectories(path)`: the `lots=` lines of `path/map.info`.
fn read_lot_directories(path: &str) -> Option<Vec<String>> {
    let file = PathBuf::from(format!("{path}/map.info"));
    if !file.exists() {
        return None;
    }
    let bytes = std::fs::read(&file).ok()?;
    let text = String::from_utf8_lossy(&bytes);
    let mut lot_dirs = Vec::new();
    // BufferedReader.readLine: \n, \r or \r\n end a line.
    for line in text.split('\n').flat_map(|l| l.split('\r')) {
        let line = java_trim(line);
        if line.starts_with("lots=") {
            lot_dirs.push(java_trim(&line.replace("lots=", "")).to_string());
        }
    }
    Some(lot_dirs)
}

impl MapGroups {
    pub fn new() -> Self {
        Self::default()
    }

    fn handle_map_directory(&mut self, name: &str, path: String) {
        let Some(lot_dirs) = read_lot_directories(&path) else {
            return;
        };
        self.real_directories.push(MapDirectory {
            name: name.to_string(),
            path,
            lot_dirs,
            conflicts: Vec::new(),
        });
    }

    /// `createGroups(activeMods, includeVanilla, includeChallenges)`.
    pub fn create_groups(
        &mut self,
        active_mods: &ActiveModsView,
        mod_dirs: ModDirs<'_>,
        maps_dir: &Path,
        include_vanilla: bool,
        include_challenges: bool,
    ) {
        self.groups.clear();
        self.real_directories.clear();
        for mod_id in &active_mods.mods {
            let Some((common_dir, version_dir)) = mod_dirs(mod_id) else {
                continue;
            };
            for base in [common_dir, version_dir] {
                let maps = format!("{base}/media/maps/");
                if !Path::new(&maps).exists() {
                    // The original stops looking at this mod here (it
                    // `continue`s the mod loop when either folder is missing).
                    break;
                }
                let Some(names) = list(Path::new(&maps)) else {
                    break;
                };
                for name in names {
                    // Challenge maps in mods are skipped either way.
                    if name.eq_ignore_ascii_case("challengemaps") {
                        continue;
                    }
                    self.handle_map_directory(&name, format!("{base}/media/maps/{name}"));
                }
            }
        }
        if include_vanilla {
            let maps_path = maps_dir.to_string_lossy().into_owned();
            for name in vanilla_map_directories(maps_dir, include_challenges) {
                self.handle_map_directory(
                    &name,
                    format!("{maps_path}{}{name}", std::path::MAIN_SEPARATOR),
                );
            }
        }
        for i in 0..self.real_directories.len() {
            let mut directories = Vec::new();
            self.dirs_recursively(i, &mut directories);
            let g = match self
                .groups
                .iter()
                .position(|g| directories.iter().any(|d| g.directories.contains(d)))
            {
                Some(g) => g,
                None => {
                    self.groups.push(MapGroup::default());
                    self.groups.len() - 1
                }
            };
            for d in directories {
                if self.group_has_directory(g, &self.real_directories[d].name) {
                    continue;
                }
                self.groups[g].directories.push(d);
            }
        }
        for g in 0..self.groups.len() {
            self.group_set_priority_all(g);
        }
        for g in 0..self.groups.len() {
            if !active_mods.map_order.is_empty() {
                self.group_set_priority(g, &active_mods.map_order);
            }
        }
    }

    /// `MapGroups.getDirsRecursively(mapDir, result)` over all directories.
    fn dirs_recursively(&self, d: usize, result: &mut Vec<usize>) {
        if result.contains(&d) {
            return;
        }
        result.push(d);
        for lot_dir in &self.real_directories[d].lot_dirs {
            if let Some(d2) = self
                .real_directories
                .iter()
                .position(|m| &m.name == lot_dir)
            {
                self.dirs_recursively(d2, result);
            }
        }
    }

    fn group_directory_by_name(&self, g: usize, name: &str) -> Option<usize> {
        self.groups[g]
            .directories
            .iter()
            .copied()
            .find(|&d| self.real_directories[d].name == name)
    }

    fn group_has_directory(&self, g: usize, name: &str) -> bool {
        self.group_directory_by_name(g, name).is_some()
    }

    fn is_referenced_by_other_maps(&self, g: usize, d: usize) -> bool {
        let name = &self.real_directories[d].name;
        self.groups[g]
            .directories
            .iter()
            .any(|&d2| d2 != d && self.real_directories[d2].lot_dirs.contains(name))
    }

    /// `MapGroup.getDirsRecursively(mapDir, names)` within one group.
    fn group_dirs_recursively(&self, g: usize, d: usize, result: &mut Vec<String>) {
        let name = &self.real_directories[d].name;
        if result.contains(name) {
            return;
        }
        result.push(name.clone());
        for lot_dir in &self.real_directories[d].lot_dirs {
            if let Some(d2) = self.group_directory_by_name(g, lot_dir) {
                self.group_dirs_recursively(g, d2, result);
            }
        }
    }

    /// `MapGroup.setPriority()`.
    fn group_set_priority_all(&mut self, g: usize) {
        let snapshot = self.groups[g].directories.clone();
        for d in snapshot {
            if self.is_referenced_by_other_maps(g, d) {
                continue;
            }
            let mut priority = Vec::new();
            self.group_dirs_recursively(g, d, &mut priority);
            self.group_set_priority(g, &priority);
        }
    }

    /// `MapGroup.setPriority(priorityList)`: reorder the listed directories
    /// into list order, keeping the slots of the others.
    fn group_set_priority(&mut self, g: usize, priority: &[String]) {
        let mut sorted: std::collections::VecDeque<usize> = priority
            .iter()
            .filter_map(|n| self.group_directory_by_name(g, n))
            .collect();
        for i in 0..self.groups[g].directories.len() {
            let d = self.groups[g].directories[i];
            if !priority.contains(&self.real_directories[d].name) {
                continue;
            }
            // `sorted2.remove(0)` throws if the list is shorter; it cannot be,
            // since every listed name present in the group was collected.
            if let Some(next) = sorted.pop_front() {
                self.groups[g].directories[i] = next;
            }
        }
    }

    pub fn get_number_of_groups(&self) -> usize {
        self.groups.len()
    }

    /// `getMapDirectoriesInGroup(i)`; `Err` is the original's
    /// `RuntimeException("invalid MapGroups index ...")`.
    pub fn get_map_directories_in_group(&self, group_index: i32) -> Result<Vec<String>, String> {
        if group_index < 0 || group_index as usize >= self.groups.len() {
            return Err(format!("invalid MapGroups index {group_index}"));
        }
        Ok(self.groups[group_index as usize]
            .directories
            .iter()
            .map(|&d| self.real_directories[d].name.clone())
            .collect())
    }

    /// The map string `setWorld(i)` passes to `IsoWorld.setMap`.
    pub fn world_map_name(&self, group_index: i32) -> Result<String, String> {
        Ok(self.get_map_directories_in_group(group_index)?.join(";"))
    }

    pub fn get_all_maps_in_order(&self) -> Vec<String> {
        self.groups
            .iter()
            .flat_map(|g| {
                g.directories
                    .iter()
                    .map(|&d| self.real_directories[d].name.clone())
            })
            .collect()
    }

    /// `checkMapConflicts()`. `get_text(key, args)` is
    /// `Translator.getText("UI_MapConflict", a, b, c)`.
    pub fn check_map_conflicts(&mut self, get_text: &dyn Fn(&str, &[&str]) -> String) -> bool {
        let mut has = false;
        for g in 0..self.groups.len() {
            has |= self.group_check_map_conflicts(g, get_text);
        }
        has
    }

    fn group_check_map_conflicts(
        &mut self,
        g: usize,
        get_text: &dyn Fn(&str, &[&str]) -> String,
    ) -> bool {
        // HashMap<String lotHeader, ArrayList<String mapName>>, keys kept in
        // insertion order to reproduce Java's iteration order.
        let mut keys: Vec<String> = Vec::new();
        let mut map: std::collections::HashMap<String, Vec<String>> =
            std::collections::HashMap::new();
        for &d in &self.groups[g].directories.clone() {
            self.real_directories[d].conflicts.clear();
            let dir = &self.real_directories[d];
            let Some(names) = list(Path::new(&dir.path)) else {
                continue;
            };
            for h in names.into_iter().filter(|n| n.ends_with(".lotheader")) {
                if !map.contains_key(&h) {
                    keys.push(h.clone());
                }
                map.entry(h).or_default().push(dir.name.clone());
            }
        }
        let key_refs: Vec<&str> = keys.iter().map(String::as_str).collect();
        let mut has = false;
        for h in hashmap_order(&key_refs) {
            let names = &map[h];
            if names.len() <= 1 {
                continue;
            }
            for i in 0..names.len() {
                let Some(d) = self.group_directory_by_name(g, &names[i]) else {
                    continue;
                };
                for (j, other) in names.iter().enumerate() {
                    if i == j {
                        continue;
                    }
                    let name = self.real_directories[d].name.clone();
                    let conflict = get_text("UI_MapConflict", &[&name, other, h]);
                    self.real_directories[d].conflicts.push(conflict);
                    has = true;
                }
            }
        }
        has
    }

    /// `getMapConflicts(mapName)`: `None` if no group has that directory.
    pub fn get_map_conflicts(&self, map_name: &str) -> Option<Vec<String>> {
        (0..self.groups.len())
            .find_map(|g| self.group_directory_by_name(g, map_name))
            .map(|d| self.real_directories[d].conflicts.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn map(root: &Path, name: &str, lots: &[&str], headers: &[&str]) {
        let dir = root.join(name);
        std::fs::create_dir_all(&dir).unwrap();
        let mut info = format!("title={name}\n");
        for l in lots {
            info.push_str(&format!("lots={l}\n"));
        }
        std::fs::write(dir.join("map.info"), info).unwrap();
        for h in headers {
            std::fs::write(dir.join(h), b"").unwrap();
        }
    }

    #[test]
    fn groups_and_priority() {
        let root = std::env::temp_dir().join(format!("pz-mapgroups-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let maps = root.join("maps");
        map(&maps, "Base", &[], &["0_0.lotheader", "1_0.lotheader"]);
        map(&maps, "Addon", &["Base"], &["1_0.lotheader"]);
        map(&maps, "Island", &[], &[]);
        map(&maps.join("challengemaps"), "Trial", &[], &[]);
        std::fs::write(maps.join("notes.txt"), b"").unwrap();

        let mut g = MapGroups::new();
        let none = |_: &str| None;
        g.create_groups(&ActiveModsView::default(), &none, &maps, true, false);
        assert_eq!(g.get_number_of_groups(), 2);
        let all = g.get_all_maps_in_order();
        // Addon depends on Base, so Addon comes first in their group.
        let ai = all.iter().position(|n| n == "Addon").unwrap();
        let bi = all.iter().position(|n| n == "Base").unwrap();
        assert!(ai < bi);
        assert!(!all.iter().any(|n| n.starts_with("challengemaps")));

        g.create_groups(&ActiveModsView::default(), &none, &maps, true, true);
        assert!(g
            .get_all_maps_in_order()
            .contains(&"challengemaps/Trial".to_string()));
        assert!(g.get_map_directories_in_group(9).is_err());

        let fmt = |k: &str, a: &[&str]| format!("{k}:{}", a.join(","));
        assert!(g.check_map_conflicts(&fmt));
        assert_eq!(
            g.get_map_conflicts("Addon").unwrap(),
            vec!["UI_MapConflict:Addon,Base,1_0.lotheader"]
        );
        assert_eq!(g.get_map_conflicts("Island").unwrap(), Vec::<String>::new());
        assert!(g.get_map_conflicts("Nope").is_none());

        // Explicit map order from ActiveMods wins.
        let order = ActiveModsView {
            mods: vec![],
            map_order: vec!["Base".into(), "Addon".into()],
        };
        g.create_groups(&order, &none, &maps, true, false);
        let all = g.get_all_maps_in_order();
        assert!(all.iter().position(|n| n == "Base") < all.iter().position(|n| n == "Addon"));

        let joined = add_missing_vanilla_directories(&maps, "Island");
        assert!(
            joined.starts_with("Island;")
                && joined.contains("Base")
                && joined.contains("notes.txt")
        );
        assert_eq!(
            add_missing_vanilla_directories(&maps, "Elsewhere"),
            "Elsewhere"
        );
        let _ = std::fs::remove_dir_all(&root);
    }
}
