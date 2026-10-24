use super::*;

/// A scratch directory removed on drop.
struct Scratch(PathBuf);

impl Scratch {
    fn new(tag: &str) -> Self {
        let p = std::env::temp_dir().join(format!("pz-fs-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).unwrap();
        Scratch(p)
    }

    fn file(&self, rel: &str, text: &str) {
        let p = self.0.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, text).unwrap();
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn install(s: &Scratch) -> GameFs {
    s.file("game/media/lua/client/B.lua", "");
    s.file("game/media/lua/client/a/Z.lua", "");
    s.file("game/media/lua/client/notes.txt", "");
    s.file("game/media/lua/shared/S.lua", "");
    s.file("game/media/ui/Icon.png", "");
    s.file("game/media/maps/Town, KY/map.info", "title=Town");
    let mut fs = GameFs::new(s.0.join("game"));
    fs.set_cache_dir(s.0.join("user"));
    fs
}

#[test]
fn file_map_lookups() {
    let s = Scratch::new("map");
    let fs = install(&s);
    let icon = fs.lookup("media/UI/icon.PNG").unwrap();
    assert!(icon.ends_with(&format!("ui{SEP}Icon.png")), "{icon}");
    assert_eq!(fs.lookup("media/ui/missing.png"), None);
    assert_eq!(fs.get_string("media/ui/missing.png"), "media/ui/missing.png");
    // Absolute paths inside the install map to the same key.
    let abs = fs.base.join("media").join("ui").join("Icon.png");
    assert_eq!(fs.lookup(&abs.to_string_lossy()), Some(icon));
    // Dot segments are folded by URI normalisation.
    assert_eq!(fs.lookup("media/lua/../ui/Icon.png"), Some(icon));
    // Map directories are recorded with a trailing separator.
    let town = fs.directory_lookup("media/maps/Town, KY").unwrap();
    assert!(town.ends_with(SEP), "{town}");
    assert!(fs.is_known_file(icon));
    assert_eq!(fs.relative_file("/elsewhere/X.png"), "/elsewhere/X.png");
}

#[test]
fn lua_order_and_mod_overrides() {
    let s = Scratch::new("mods");
    let mut fs = install(&s);
    s.file("user/mods/ModA/common/mod.info", "name=Mod A\nid=ModA\nrequire=ModB\npack=APack\n");
    s.file("user/mods/ModA/42/media/lua/client/B.lua", "-- override");
    s.file("user/mods/ModA/42/media/lua/client/C.lua", "");
    s.file("user/mods/ModA/42/media/texturepacks/APack.pack", "");
    s.file("user/mods/ModB/common/mod.info", "id=ModB\nversionMin=42.0\n");
    s.file("user/mods/ModB/common/media/lua/client/A_b.lua", "");
    s.file("user/mods/ModFuture/common/mod.info", "id=ModFuture\nversionMin=43.0\n");

    fs.load_mods(&["ModA".to_string(), "ModFuture".to_string(), "Missing".to_string()]);
    // Requirements load first; unavailable and missing mods are skipped.
    assert_eq!(fs.mod_ids(), ["ModB", "ModA"]);

    let files = fs.lua_files("client");
    assert_eq!(files, ["media/lua/client/a/z.lua", "media/lua/client/b.lua", "media/lua/client/a_b.lua", "media/lua/client/c.lua"]);
    let b = fs.absolute_path("media/lua/client/b.lua").unwrap();
    assert!(b.contains("ModA"), "{b}");

    let dir = fs.mod_details("ModA").unwrap();
    let info = fs.mod_info(&dir).unwrap();
    assert_eq!(info.name, "Mod A");
    assert!(info.version_dir.ends_with(&format!("ModA{SEP}42")), "{}", info.version_dir);
    assert_eq!(fs.mod_pack_files(), [("APack".to_string(), 68, "ModA".to_string())]);
    assert!(fs.mod_pack_files().is_empty());
}

#[test]
fn default_mods_file() {
    let s = Scratch::new("default");
    let mut fs = install(&s);
    s.file("user/mods/default.txt", "VERSION = 1,\n\nmods\n{\n    mod = A,\n}\n\nmaps\n{\n}\n");
    s.file("user/mods/loaded.txt", "B\n\n C \n");
    let m = fs.read_default_mods();
    assert_eq!(m.mods, ["A", "B", "C"]);
    assert!(!s.0.join("user/mods/loaded.txt").exists());
}

/// Parity checks against an installed game: `PZ_GAME_DIR=... cargo test -- --ignored`.
#[test]
#[ignore]
fn installed_game() {
    let Some(dir) = std::env::var_os("PZ_GAME_DIR") else { return };
    let mut fs = GameFs::new(PathBuf::from(dir));
    assert!(fs.resolve("stdlib.lua").is_some());
    let s = Scratch::new("installed");
    fs.set_cache_dir(&s.0);
    eprintln!("active files: {}", fs.active_file_map.len());
    for p in ["media/lua/shared/luautils.lua", "media/ui/white.png", "media/texturepacks/UI.pack", "media/maps/Muldraugh, KY/"] {
        eprintln!("{p} -> {:?}", fs.directory_lookup(p));
    }
    let shared = fs.lua_files("shared");
    let client = fs.lua_files("client");
    eprintln!("lua shared {} client {}; first {:?} last {:?}", shared.len(), client.len(), client.first(), client.last());
    assert!(!client.is_empty());
}
