use super::*;
use pz_sqlite::writer::Writer;
use pz_sqlite::TextEncoding;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

const LOCAL_PLAYERS: &str =
    "CREATE TABLE localPlayers (id INTEGER PRIMARY KEY NOT NULL,name STRING,\
wx INTEGER,wy INTEGER,x FLOAT,y FLOAT,z FLOAT,worldversion INTEGER,data BLOB,isDead BOOLEAN)";

/// A `localPlayers` database with `(id, name, isDead)` rows.
fn players_db(path: &Path, rows: &[(i64, Option<&str>, i64)]) {
    use pz_sqlite::Value as V;
    let mut w = Writer::new(4096, 0, TextEncoding::Utf8);
    let rows: Vec<(i64, Vec<V>)> = rows
        .iter()
        .map(|&(id, name, dead)| {
            let name = name.map_or(V::Null, |n| V::Text(n.into()));
            let v = vec![
                V::Null,
                name,
                V::Integer(1),
                V::Integer(2),
                V::Real(1.5),
                V::Real(2.5),
                V::Integer(0),
                V::Integer(244),
                // Large enough to spill onto overflow pages.
                V::Blob(vec![9; 5000]),
                V::Integer(dead),
            ];
            (id, v)
        })
        .collect();
    w.table("localPlayers", LOCAL_PLAYERS, &rows);
    std::fs::write(path, w.finish()).unwrap();
}

fn java_string(s: &str) -> Vec<u8> {
    let mut v = (s.encode_utf16().count() as i32).to_be_bytes().to_vec();
    v.extend(s.encode_utf16().flat_map(u16::to_be_bytes));
    v
}

fn set_mtime(p: &Path, secs: u64) {
    let f = std::fs::File::open(p).unwrap();
    f.set_modified(SystemTime::UNIX_EPOCH + Duration::from_secs(secs))
        .unwrap();
}

struct Env {
    dir: PathBuf,
    saves: PathBuf,
    mgr: LuaManager,
}

impl Drop for Env {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn setup(tag: &str) -> Env {
    let dir = std::env::temp_dir().join(format!("pz-bind-save-{}-{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let maps = dir.join("game/media/maps");
    std::fs::create_dir_all(maps.join("TestMap")).unwrap();
    std::fs::write(
        maps.join("TestMap/map.info"),
        "title=Test Map\nlots=A\n  lots= B \ndescription=One \ndescription=two\nfixed2x=true\n\
         zoomX=1.5\nzoomY=2\nzoomS=0.25\ndemoVideo=x.bik\n",
    )
    .unwrap();
    std::fs::create_dir_all(maps.join("BadZoom")).unwrap();
    std::fs::write(maps.join("BadZoom/map.info"), "title=Bad\nzoomX=wide\n").unwrap();

    let saves = dir.join("user/Saves");
    let w1 = saves.join("Sandbox/w1");
    let w2 = saves.join("Apocalypse/w2");
    let w3 = saves.join("Sandbox/w3");
    for d in [&w1, &w2, &w3, &saves.join("Multiplayer/mp")] {
        std::fs::create_dir_all(d).unwrap();
    }
    std::fs::write(saves.join("Sandbox/not-a-save.txt"), "").unwrap();
    std::fs::write(saves.join("stray.txt"), "").unwrap();

    let mut b = 249i32.to_be_bytes().to_vec();
    b.extend(java_string("Muldraugh, KY;Riverside, KY"));
    std::fs::write(w1.join("map_ver.bin"), b).unwrap();
    std::fs::write(
        w1.join("mods.txt"),
        "VERSION = 1,\n\nmods\n{\n    mod = A,\n    mod = \\B,\n}\n\nmaps\n{\n    map = X,\n}\n",
    )
    .unwrap();
    players_db(
        &w1.join("players.db"),
        &[(1, Some("Kate"), 0), (2, None, 1), (5, Some("Bob"), 1)],
    );

    let mut b = 200i32.to_be_bytes().to_vec();
    b.extend(java_string("DEFAULT"));
    b.extend(java_string("Hard"));
    std::fs::write(w2.join("map_ver.bin"), b).unwrap();
    // Dead in players.db, but alive in vehicles.db, which is checked first.
    players_db(&w2.join("players.db"), &[(1, Some("Dead"), 1)]);
    players_db(&w2.join("vehicles.db"), &[(1, Some("Dead"), 0)]);

    // An empty file is an SQLite database without tables.
    std::fs::write(w3.join("players.db"), []).unwrap();

    set_mtime(&w1, 1_000_000);
    set_mtime(&w2, 3_000_000);
    set_mtime(&w3, 2_000_000);

    let mut gfs = GameFs::new(dir.join("game"));
    gfs.set_cache_dir(dir.join("user"));
    let fs = Rc::new(RefCell::new(gfs));
    let mut mgr = LuaManager::init(fs).unwrap();
    {
        let core = shared_core();
        let mut core = core.borrow_mut();
        core.set_game_mode("Sandbox");
        core.game_save_world = "w1".into();
    }
    run(
        &mut mgr,
        "function assert(v, msg) if not v then error(msg or 'assertion failed!', 2) end return v end",
    );
    Env { dir, saves, mgr }
}

fn run(mgr: &mut LuaManager, src: &str) {
    let f =
        pz_kahlua::stdlib::load_string(src, Some("test"), mgr.thread.globals().clone()).unwrap();
    if let Err((e, tr)) = mgr.thread.pcall(&f, &[]) {
        panic!("lua error: {e:?}\n{tr}");
    }
}

#[cfg(unix)]
#[test]
fn save_listing_and_info() {
    let mut env = setup("info");
    let saves = env.saves.to_string_lossy().into_owned();
    let abs_w1 = format!("{saves}/Sandbox/w1");
    run(
        &mut env.mgr,
        &format!(
            r#"
        local d = getFullSaveDirectoryTable()
        assert(#d == 3, "count " .. #d)
        assert(d[1] == "Apocalypse/w2" and d[2] == "Sandbox/w3" and d[3] == "Sandbox/w1")
        assert(#getSaveDirectoryTable() == 0)

        local i = getSaveInfo("Sandbox/w1")
        assert(i.saveName == "w1" and i.gameMode == "Sandbox" and i.saveDir == "Sandbox/w1")
        assert(i.worldVersion == 249 and i.mapName == "Muldraugh, KY;Riverside, KY" and i.difficulty == nil)
        assert(string.match(i.lastPlayed, "^%d%d/%d%d/%d%d%d%d %d%d:%d%d$"), i.lastPlayed)
        local mods = i.activeMods:getMods()
        assert(mods:size() == 2 and mods:get(0) == "A" and mods:get(1) == "B")
        assert(i.activeMods:getMapOrder():get(0) == "X")
        assert(i.playerAlive == true)
        assert(#i.players == 3)
        local p = i.players
        assert(p[1].sqlID == 1 and p[1].name == "Kate" and p[1].isDead == false)
        assert(p[2].sqlID == 2 and p[2].name == nil and p[2].isDead == true)
        assert(p[3].sqlID == 5 and p[3].name == "Bob" and p[3].isDead == true)

        -- Without a separator the save is looked up under Core's game mode.
        assert(getSaveInfo("w1").saveDir == "Sandbox/w1")

        local i2 = getSaveInfo("Apocalypse/w2")
        assert(i2.worldVersion == 200 and i2.mapName == "Muldraugh, KY" and i2.difficulty == "Hard")
        assert(i2.activeMods == nil and i2.playerAlive == true and i2.players[1].isDead == true)

        local i3 = getSaveInfo("Sandbox/w3")
        assert(i3.saveName == "w3" and i3.worldVersion == nil and i3.mapName == nil)
        assert(i3.playerAlive == false and #i3.players == 0)

        local m = getSaveInfo("Sandbox/nope")
        assert(m.saveName == nil and m.lastPlayed == nil and m.playerAlive == false and #m.players == 0)
        assert(not pcall(getSaveInfo, "/"))

        assert(getLastPlayedDate("Sandbox/nope") == "UI_LastPlayed???")
        assert(getLastPlayedDate("Sandbox/w1") == "UI_LastPlayed" .. i.lastPlayed)
        assert(checkSaveFolderExists("Sandbox/w1") and not checkSaveFolderExists("Sandbox/zz"))
        assert(getAbsoluteSaveFolderName("Sandbox/w1") == "{abs_w1}")
        assert(checkSaveFileExists("map_ver.bin") and not checkSaveFileExists("map_t.bin"))
        assert(checkSavePlayerExists() == true)

        local l = getSaveDirectory("{saves}/Sandbox")
        assert(l:size() == 2)
        local names = {{}}
        for k = 0, l:size() - 1 do
            local f = l:get(k)
            assert(f:isDirectory() and f:getParent() == "{saves}/Sandbox")
            names[f:getName()] = f:getPath()
        end
        assert(names.w1 == "{abs_w1}" and names.w3)
        assert(not pcall(getSaveDirectory, "/definitely/elsewhere"))

        assert(getTextureFromSaveDir("../thumb.png", "Sandbox/w1") == nil)
        assert(getTextureFromSaveDir("thumb.png", "Sandbox/w1") == nil)
    "#
        ),
    );
    // Player 1 dead and no vehicles.db.
    {
        let core = shared_core();
        let mut core = core.borrow_mut();
        core.set_game_mode("Apocalypse");
        core.game_save_world = "w2".into();
    }
    std::fs::remove_file(env.saves.join("Apocalypse/w2/vehicles.db")).unwrap();
    run(
        &mut env.mgr,
        "assert(checkSavePlayerExists() == false); assert(getSaveInfo('w2').playerAlive == false)",
    );
    shared_core().borrow_mut().set_no_save(true);
    run(
        &mut env.mgr,
        "local i = getSaveInfo('Sandbox/w1'); assert(i.playerAlive == false and #i.players == 0)",
    );
    shared_core().borrow_mut().set_no_save(false);
}

#[test]
fn map_info() {
    let mut env = setup("map");
    run(
        &mut env.mgr,
        r#"
        local m = getMapInfo("TestMap;Other")
        assert(m.title == "Test Map" and m.description == "Onetwo") -- lines are trimmed first
        assert(m.lots[1] == "B" and m.lots[2] == nil)
        assert(m.zoomX == 1.5 and m.zoomY == 2 and m.zoomS == 0.25 and m.demoVideo == "x.bik")
        assert(string.sub(m.dir, -7) == "TestMap")
        assert(m.thumb == nil and m.worldmap == nil and m.spawnSelectImagePyramid == nil)
        assert(m.only_for_game_mode == nil)
        assert(getMapInfo("TestMap").title == "Test Map")
        assert(getMapInfo("Nope") == nil)
        assert(getMapInfo("BadZoom") == nil)
        assert(not pcall(getMapInfo, ";"))
    "#,
    );
}

#[cfg(unix)]
#[test]
fn save_folder_changes() {
    let mut env = setup("changes");
    let s = env.saves.clone();
    let w = |rel: &str| {
        let p = s.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, rel).unwrap();
    };
    for rel in [
        "Sandbox/del/map/1/2.bin",
        "Sandbox/del/x.bin",
        "Sandbox/m/map/3/4.bin",
        "Sandbox/m/map/3/x.bin",
        "Sandbox/m/zpop/zpop_1_2.bin",
        "Sandbox/m/zpop/zpop_-1_2.bin",
        "Sandbox/m/entity_data.bin",
        "Sandbox/m/entityXdata.bin",
        "LastStand/a/b/c.bin",
    ] {
        w(rel);
    }
    let cache = env.dir.join("user");
    std::fs::create_dir_all(cache.join("db")).unwrap();
    std::fs::write(cache.join("db/coop.db"), "").unwrap();
    crate::savefile::set_active_mods(
        "currentGame",
        pz_fs::ActiveMods {
            mods: vec!["M1".into()],
            map_order: vec!["Muldraugh, KY".into()],
        },
    );
    run(
        &mut env.mgr,
        r#"
        deleteSave("Sandbox/../Sandbox/del")
        deleteSave("Sandbox/del")
        assert(renameSavefile("Sandbox", "w3", "w 4") == false)
        assert(renameSavefile("Sandbox", "w3", "w4") == true)
        assert(renameSavefile("Sandbox", "w4", "w1") == false)
        assert(renameSavefile("Sandbox", "w4", ".w") == false)
        assert(renameSavefile("Sandbox", "../x", "y") == false)
        manipulateSavefile("Sandbox/m", "DeleteMapXYBin")
        manipulateSavefile("Sandbox/m", "DeleteZPopXYBin")
        manipulateSavefile("Sandbox/m", "DeleteEntityDataBin")
        manipulateSavefile("Sandbox/m", "WriteModsDotTxt")
        assert(not pcall(manipulateSavefile, "Sandbox/m", "Nope"))
        manipulateSavefile("Sandbox/missing", "Nope")
        deleteAllGameModeSaves("LastStand")
        deleteDatabase("Sandbox/coop")
        setSavefilePlayer1("Sandbox", "w1", 5)
        "#,
    );
    assert!(!s.join("Sandbox/del").exists());
    assert!(s.join("Sandbox/w4").is_dir() && !s.join("Sandbox/w3").exists());
    assert!(!s.join("Sandbox/m/map/3/4.bin").exists() && s.join("Sandbox/m/map/3/x.bin").exists());
    assert!(!s.join("Sandbox/m/zpop/zpop_1_2.bin").exists());
    assert!(s.join("Sandbox/m/zpop/zpop_-1_2.bin").exists());
    assert!(
        !s.join("Sandbox/m/entity_data.bin").exists()
            && s.join("Sandbox/m/entityXdata.bin").exists()
    );
    let mods = std::fs::read_to_string(s.join("Sandbox/m/mods.txt")).unwrap();
    assert!(
        mods.contains("mod = M1,") && mods.contains("map = Muldraugh, KY,"),
        "{mods}"
    );
    assert!(!s.join("LastStand").exists());
    assert_eq!(shared_core().borrow().game_mode, "Sandbox");
    assert!(!cache.join("db/coop.db").exists());
    let ids: Vec<_> = crate::savefile::get_players(false, &s.join("Sandbox/w1").to_string_lossy())
        .unwrap()
        .into_iter()
        .map(|p| (p.sql_id, p.name))
        .collect();
    assert_eq!(
        ids,
        [(1, Some("Bob".into())), (2, None), (5, Some("Kate".into()))]
    );
}

/// A temp dir unique to this call (per-process counter), so parallel tests
/// never share one.
fn unique_temp_dir(prefix: &str) -> PathBuf {
    static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("{prefix}-{}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

struct LastStandEnv {
    dir: PathBuf,
    mgr: LuaManager,
}

impl Drop for LastStandEnv {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn last_stand_env() -> LastStandEnv {
    let dir = unique_temp_dir("pz-bind-save-laststand");
    std::fs::create_dir_all(dir.join("game/media")).unwrap();
    std::fs::create_dir_all(dir.join("user")).unwrap();
    let mut gfs = GameFs::new(dir.join("game"));
    gfs.set_cache_dir(dir.join("user"));
    let mut mgr = LuaManager::init(Rc::new(RefCell::new(gfs))).unwrap();
    run(
        &mut mgr,
        "function assert(v, msg) if not v then error(msg or 'assertion failed!', 2) end return v end",
    );
    LastStandEnv { dir, mgr }
}

#[test]
fn last_stand_file_names_create_the_folders() {
    let mut e = last_stand_env();
    let ls = e.dir.join("user/Lua/LastStand");
    assert!(!e.dir.join("user/Lua").exists());
    run(
        &mut e.mgr,
        r#"
        assert(getLastStandPlayersDirectory() == "LastStand")
        local l = getLastStandPlayerFileNames()
        assert(l ~= nil and l:size() == 0)
        assert(tostring(l) == "[]")
        "#,
    );
    assert!(ls.is_dir());
}

#[test]
fn last_stand_file_names_filter_and_order() {
    let mut e = last_stand_env();
    let ls = e.dir.join("user/Lua/LastStand");
    std::fs::create_dir_all(&ls).unwrap();
    for n in [
        "playerKateSmith.txt",
        "notes.cfg",
        "playerBob.txt",
        "x.TXT",
        "player.txt.bak",
    ] {
        std::fs::write(ls.join(n), "VERSION=1\n").unwrap();
    }
    // A directory with a .txt name is skipped.
    std::fs::create_dir_all(ls.join("dir.txt")).unwrap();
    // The order is the directory listing's (File.listFiles is unsorted).
    let expected: Vec<String> = std::fs::read_dir(&ls)
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.ends_with(".txt") && !ls.join(n).is_dir())
        .map(|n| format!("LastStand{SEP}{n}"))
        .collect();
    assert_eq!(expected.len(), 2);
    let src = format!(
        "local l = getLastStandPlayerFileNames()\nassert(l:size() == {})\n{}",
        expected.len(),
        expected
            .iter()
            .enumerate()
            .map(|(i, n)| format!("assert(l:get({i}) == {n:?}, tostring(l:get({i})))\n"))
            .collect::<String>()
    );
    run(&mut e.mgr, &src);
    // The names feed getFileReader (relative to the Lua cache dir).
    run(
        &mut e.mgr,
        r#"
        local l = getLastStandPlayerFileNames()
        local r = getFileReader(l:get(0), false)
        assert(r:readLine() == "VERSION=1")
        assert(r:readLine() == nil)
        r:close()
        "#,
    );
}

#[test]
fn delete_player_save_removes_the_player_file() {
    let mut e = last_stand_env();
    let ls = e.dir.join("user/Lua/LastStand");
    std::fs::create_dir_all(&ls).unwrap();
    std::fs::write(ls.join("playerKateSmith.txt"), "x").unwrap();
    std::fs::write(ls.join("playernull.txt"), "x").unwrap();
    std::fs::write(e.dir.join("user/Lua/playerEvil.txt"), "x").unwrap();
    run(
        &mut e.mgr,
        r#"
        deletePlayerSave("KateSmith")
        deletePlayerSave("Missing")
        deletePlayerSave("/../../Lua/playerEvil")
        deletePlayerSave(nil)
        "#,
    );
    assert!(!ls.join("playerKateSmith.txt").exists());
    assert!(!ls.join("playernull.txt").exists());
    assert!(e.dir.join("user/Lua/playerEvil.txt").exists());
}

#[test]
fn all_saved_players_reads_every_entry() {
    let mut e = last_stand_env();
    let ls = e.dir.join("user/Lua/LastStand");
    std::fs::create_dir_all(&ls).unwrap();
    std::fs::write(ls.join("playerA.txt"), "VERSION=1\r\nPlayer\n").unwrap();
    run(
        &mut e.mgr,
        r#"
        local l = getAllSavedPlayers()
        assert(l:size() == 1)
        local r = l:get(0)
        assert(r:readLine() == "VERSION=1")
        assert(r:readLine() == "Player")
        assert(r:readLine() == nil)
        "#,
    );
    std::fs::create_dir_all(ls.join("sub")).unwrap();
    run(
        &mut e.mgr,
        r#"
        local ok = pcall(getAllSavedPlayers)
        assert(not ok)
        "#,
    );
}
