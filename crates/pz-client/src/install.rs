//! Locating the installed game and the user's home on each host OS.
//!
//! The game ships per platform under the Steam library's
//! `steamapps/common/ProjectZomboid`: the macOS depot keeps `media/` in
//! `Project Zomboid.app/Contents/Java`, the Linux depot in
//! `projectzomboid/`, and the Windows depot (also what Proton installs on
//! Linux) directly in the app folder. Steam libraries on other drives are
//! listed in `steamapps/libraryfolders.vdf`.

use std::path::{Path, PathBuf};

/// `System.getProperty("user.home")`: the profile directory on Windows,
/// `$HOME` elsewhere.
pub fn home_dir() -> PathBuf {
    let (first, second) = if cfg!(windows) { ("USERPROFILE", "HOME") } else { ("HOME", "USERPROFILE") };
    std::env::var_os(first).or_else(|| std::env::var_os(second)).map(PathBuf::from).unwrap_or_default()
}

/// Steam's own install directories that may exist on this host.
fn steam_roots(home: &Path) -> Vec<PathBuf> {
    let mut roots = Vec::new();
    if cfg!(target_os = "macos") {
        roots.push(home.join("Library/Application Support/Steam"));
    } else if cfg!(windows) {
        for var in ["ProgramFiles(x86)", "ProgramFiles"] {
            if let Some(p) = std::env::var_os(var) {
                roots.push(PathBuf::from(p).join("Steam"));
            }
        }
        roots.push(PathBuf::from(r"C:\Program Files (x86)\Steam"));
    } else {
        roots.push(home.join(".steam/steam"));
        roots.push(home.join(".local/share/Steam"));
        // Flatpak and Snap packages of Steam.
        roots.push(home.join(".var/app/com.valvesoftware.Steam/.local/share/Steam"));
        roots.push(home.join("snap/steam/common/.local/share/Steam"));
    }
    roots
}

/// The `"path"` values of a `libraryfolders.vdf`.
fn library_paths(vdf: &str) -> Vec<PathBuf> {
    vdf.lines()
        .filter_map(|line| {
            let mut quoted = line.split('"').skip(1).step_by(2);
            if quoted.next()?.eq_ignore_ascii_case("path") {
                Some(PathBuf::from(quoted.next()?.replace(r"\\", r"\")))
            } else {
                None
            }
        })
        .collect()
}

/// Every Steam library: the roots, then the libraries each root lists.
fn steam_libraries(roots: &[PathBuf]) -> Vec<PathBuf> {
    let mut libs: Vec<PathBuf> = roots.to_vec();
    for root in roots {
        if let Ok(vdf) = std::fs::read_to_string(root.join("steamapps/libraryfolders.vdf")) {
            libs.extend(library_paths(&vdf));
        }
    }
    let mut seen = std::collections::HashSet::new();
    libs.retain(|p| seen.insert(p.clone()));
    libs
}

/// The directories inside an app folder that hold `media/`, this host's
/// depot layout first.
fn depot_layouts() -> [&'static str; 3] {
    const MAC: &str = "Project Zomboid.app/Contents/Java";
    const LINUX: &str = "projectzomboid";
    if cfg!(target_os = "macos") {
        [MAC, "", LINUX]
    } else if cfg!(windows) {
        ["", LINUX, MAC]
    } else {
        [LINUX, "", MAC]
    }
}

/// The app folder layouts in every library of the Steam `roots`.
fn game_dirs_in(roots: &[PathBuf]) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    for lib in steam_libraries(roots) {
        let app = lib.join("steamapps").join("common").join("ProjectZomboid");
        for layout in depot_layouts() {
            dirs.push(if layout.is_empty() { app.clone() } else { app.join(layout) });
        }
    }
    dirs
}

/// `--game-dir`, then `PZ_GAME_DIR`, then the default locations: the
/// first directory that contains `media/`.
pub fn find_game_dir(arg: Option<PathBuf>) -> Option<PathBuf> {
    let candidates = arg.into_iter().chain(std::env::var_os("PZ_GAME_DIR").map(PathBuf::from));
    candidates.chain(game_dirs_in(&steam_roots(&home_dir()))).find(|p| p.join("media").is_dir())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_library_folders() {
        let vdf = r#""libraryfolders"
{
	"0"
	{
		"path"		"C:\\Program Files (x86)\\Steam"
		"label"		""
		"apps"
		{
			"108600"		"7516412590"
		}
	}
	"1"
	{
		"path"		"D:\\SteamLibrary"
	}
	"2"
	{
		"path"		"/mnt/games/SteamLibrary"
	}
}
"#;
        assert_eq!(
            library_paths(vdf),
            [
                PathBuf::from(r"C:\Program Files (x86)\Steam"),
                PathBuf::from(r"D:\SteamLibrary"),
                PathBuf::from("/mnt/games/SteamLibrary"),
            ]
        );
    }

    #[test]
    fn finds_install_in_secondary_library() {
        static N: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let base = std::env::temp_dir().join(format!("pz-client-install-{}-{n}", std::process::id()));
        let root = base.join("Steam");
        let lib = base.join("Library 2");
        std::fs::create_dir_all(root.join("steamapps")).unwrap();
        let vdf = format!("\"libraryfolders\"\n{{\n\t\"1\"\n\t{{\n\t\t\"path\"\t\t\"{}\"\n\t}}\n}}\n", lib.display().to_string().replace('\\', r"\\"));
        std::fs::write(root.join("steamapps/libraryfolders.vdf"), vdf).unwrap();
        let app = lib.join("steamapps").join("common").join("ProjectZomboid");
        let want = if cfg!(target_os = "macos") { app.join("Project Zomboid.app/Contents/Java") } else if cfg!(windows) { app.clone() } else { app.join("projectzomboid") };
        std::fs::create_dir_all(want.join("media")).unwrap();
        let found = game_dirs_in(&[root]).into_iter().find(|p| p.join("media").is_dir());
        let _ = std::fs::remove_dir_all(&base);
        assert_eq!(found, Some(want));
    }
}
