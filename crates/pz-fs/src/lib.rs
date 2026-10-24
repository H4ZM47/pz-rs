//! `zombie.ZomboidFileSystem`: locating game media and mod files.
//!
//! The original resolves `media/...` paths case-insensitively against the
//! installed game directory, then active mods (later mods override earlier
//! ones). All lookups use '/'-separated, lower-cased relative paths as keys.

use std::path::{Path, PathBuf};

pub struct GameFs {
    /// Directory containing `media/` (the original's `base`).
    pub base: PathBuf,
}

impl GameFs {
    pub fn new(base: impl Into<PathBuf>) -> Self {
        GameFs { base: base.into() }
    }

    /// `getMediaFile` / `getAbsolutePath`: resolve a relative game path
    /// ("media/lua/client/Foo.lua"), ignoring case. TODO: mod overrides.
    pub fn resolve(&self, rel: &str) -> Option<PathBuf> {
        let direct = self.base.join(rel);
        if direct.exists() {
            return Some(direct);
        }
        resolve_case_insensitive(&self.base, rel)
    }

    /// `LuaManager.searchFolders`: all `.lua` files under `media/lua/<sub>`,
    /// as lower-cased paths relative to `base`, sorted case-insensitively
    /// (`String.CASE_INSENSITIVE_ORDER`).
    pub fn lua_files(&self, sub: &str) -> Vec<String> {
        let mut out = Vec::new();
        let root = self.base.join("media").join("lua").join(sub);
        collect_lua(&self.base, &root, &mut out);
        out.sort_by_key(|a| a.to_lowercase());
        out
    }
}

fn collect_lua(base: &Path, dir: &Path, out: &mut Vec<String>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    // Java File.list() order is unspecified; the result is sorted afterwards.
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            collect_lua(base, &p, out);
        } else if p.to_string_lossy().to_lowercase().ends_with(".lua") {
            if let Ok(rel) = p.strip_prefix(base) {
                out.push(rel.to_string_lossy().replace('\\', "/").to_lowercase());
            }
        }
    }
}

fn resolve_case_insensitive(base: &Path, rel: &str) -> Option<PathBuf> {
    let mut cur = base.to_path_buf();
    for part in rel.split(['/', '\\']).filter(|p| !p.is_empty()) {
        let direct = cur.join(part);
        if direct.exists() {
            cur = direct;
            continue;
        }
        let lower = part.to_lowercase();
        let found = std::fs::read_dir(&cur)
            .ok()?
            .flatten()
            .find(|e| e.file_name().to_string_lossy().to_lowercase() == lower)?;
        cur = found.path();
    }
    Some(cur)
}
