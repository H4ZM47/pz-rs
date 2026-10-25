//! `LuaManager.GlobalObject` globals on the main-menu path that no
//! subsystem owns: random numbers, Lua cache-dir file I/O, string helpers,
//! sandbox presets, and the network/Steam globals as the original behaves
//! with no server connection and Steam absent (`GameClient.client`,
//! `GameServer.server` and `SteamUtils.isSteamModeEnabled()` all false).

use crate::events::EventManager;
use crate::expose::{bool_arg, expose_class, global, int_arg, num_arg, str_arg, with_this};
use pz_fs::GameFs;
use pz_kahlua::value::LuaObject;
use pz_kahlua::{LuaError, LuaResult, NativeCall, TableRef, Thread, Value};
use std::any::Any;
use std::cell::{Cell, RefCell};
use std::io::Write;
use std::path::{PathBuf, MAIN_SEPARATOR};
use std::rc::Rc;

// ---- random numbers ---------------------------------------------------------

/// `RandLua.INSTANCE`: a `java.util.Random` stream. The original draws from
/// a cellular-automaton generator seeded by `SecureRandom`, so sequences
/// are never reproducible there either; the ranges and rounding below are
/// the original's.
struct LuaRand {
    seed: Cell<i64>,
}

const MULTIPLIER: i64 = 0x5DEECE66D;
const MASK: i64 = (1 << 48) - 1;

impl LuaRand {
    fn new() -> Self {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos() as i64)
            .unwrap_or(0);
        LuaRand {
            seed: Cell::new((nanos ^ MULTIPLIER) & MASK),
        }
    }

    /// `Random.next(bits)`.
    fn next(&self, bits: u32) -> i32 {
        let s = (self.seed.get().wrapping_mul(MULTIPLIER).wrapping_add(0xB)) & MASK;
        self.seed.set(s);
        (s >> (48 - bits)) as i32
    }

    /// `Random.nextInt(bound)`, bound > 0.
    fn next_int(&self, bound: i32) -> i32 {
        let m = bound - 1;
        if bound & m == 0 {
            return ((bound as i64 * self.next(31) as i64) >> 31) as i32;
        }
        loop {
            let u = self.next(31);
            let r = u % bound;
            if u.wrapping_sub(r).wrapping_add(m) >= 0 {
                return r;
            }
        }
    }

    /// `Random.nextFloat()`.
    fn next_float(&self) -> f32 {
        self.next(24) as f32 / (1 << 24) as f32
    }

    /// `RandAbstract.Next(int max)`.
    fn next_max(&self, max: i32) -> i32 {
        if max <= 0 {
            return 0;
        }
        self.next_int(max)
    }

    /// `RandAbstract.Next(float min, float max)`.
    fn next_range_f(&self, mut min: f32, mut max: f32) -> f32 {
        if max == min {
            return min;
        }
        if min > max {
            std::mem::swap(&mut min, &mut max);
        }
        min + self.next_float() * (max - min)
    }
}

thread_local! {
    static RAND: LuaRand = LuaRand::new();
}

/// `ZombRand(max)`: `[0, max)` (or `(max, 0]` for negative max), with
/// `max` truncated to a `long` and then to an `int`.
pub fn zomb_rand(max: f64) -> f64 {
    if max == 0.0 {
        return 0.0;
    }
    RAND.with(|r| {
        if max < 0.0 {
            -(r.next_max((-(max as i64)) as i32) as f64)
        } else {
            r.next_max((max as i64) as i32) as f64
        }
    })
}

/// `ZombRandBetween(min, max)`: `RandLua.Next(long, long)`.
fn zomb_rand_between(min: f64, max: f64) -> f64 {
    let (mut lo, mut hi) = (min as i64, max as i64);
    if hi == lo {
        return lo as f64;
    }
    if lo > hi {
        std::mem::swap(&mut lo, &mut hi);
    }
    RAND.with(|r| (r.next_int((hi - lo) as i32) as i64 + lo) as f64)
}

// ---- files ----------------------------------------------------------------------

/// `StringUtils.containsDoubleDot`.
fn has_relative_path(path: &str) -> bool {
    if path.contains("..") {
        eprintln!("WARN : Lua: relative paths not allowed");
        return true;
    }
    false
}

/// `LuaManager.getLuaCacheDir()`: `<cache>/Lua`, created if missing.
fn lua_cache_dir(fs: &GameFs) -> String {
    let dir = format!("{}{MAIN_SEPARATOR}Lua", fs.cache_dir().display());
    let _ = std::fs::create_dir(&dir);
    dir
}

/// `LuaManager.getSandboxCacheDir()`: `<cache>/Sandbox Presets`.
fn sandbox_cache_dir(fs: &GameFs) -> String {
    let dir = format!(
        "{}{MAIN_SEPARATOR}Sandbox Presets",
        fs.cache_dir().display()
    );
    let _ = std::fs::create_dir(&dir);
    dir
}

fn native_separators(s: &str) -> String {
    s.replace(['/', '\\'], std::path::MAIN_SEPARATOR_STR)
}

/// `ZomboidFileSystem.getFileExtension`: text after the last '.' ("" if none).
fn file_extension(name: &str) -> &str {
    name.rfind('.').map_or("", |i| &name[i + 1..])
}

/// `java.io.BufferedReader` over a UTF-8 file.
struct LuaReader {
    lines: RefCell<std::vec::IntoIter<String>>,
}

impl LuaObject for LuaReader {
    fn class_name(&self) -> &str {
        "java.io.BufferedReader"
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn super_classes(&self) -> &[&'static str] {
        &["java.io.Reader"]
    }
}

/// A `java.io.BufferedReader` over `text` (a file read as UTF-8).
pub(crate) fn reader_value(text: &str) -> Value {
    Value::Object(Rc::new(LuaReader {
        lines: RefCell::new(split_lines(text).into_iter()),
    }))
}

/// `BufferedReader.readLine` splitting: '\n', '\r' or "\r\n".
fn split_lines(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\n' => out.push(std::mem::take(&mut cur)),
            '\r' => {
                if chars.peek() == Some(&'\n') {
                    chars.next();
                }
                out.push(std::mem::take(&mut cur));
            }
            c => cur.push(c),
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

/// `LuaManager.GlobalObject.LuaFileWriter` over a `PrintWriter`: output is
/// buffered and reaches the file on `close()`.
struct LuaFileWriter {
    file: RefCell<Option<std::fs::File>>,
    buf: RefCell<String>,
}

impl LuaObject for LuaFileWriter {
    fn class_name(&self) -> &str {
        "zombie.Lua.LuaManager$GlobalObject$LuaFileWriter"
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

fn line_separator() -> &'static str {
    if cfg!(windows) {
        "\r\n"
    } else {
        "\n"
    }
}

fn str_or_null(c: &NativeCall<'_>, i: usize) -> String {
    // A Lua nil passed for a String parameter prints as "null".
    str_arg(c, i).map_or_else(|| "null".to_string(), |s| s.to_string())
}

// ---- strings ----------------------------------------------------------------------

fn is_punctuation(c: u16) -> bool {
    matches!(c, 0x2e | 0x2c | 0x21 | 0x3f | 0x3b | 0x3a)
}

/// `Character.isWhitespace` for a UTF-16 unit.
fn is_java_whitespace(c: u16) -> bool {
    matches!(c, 0x09..=0x0d | 0x1c..=0x1f)
        || char::from_u32(c as u32)
            .is_some_and(|ch| ch.is_whitespace() && !matches!(c, 0xa0 | 0x2007 | 0x202f))
}

fn java_trim16(s: &[u16]) -> &[u16] {
    let start = s.iter().position(|&c| c > 0x20).unwrap_or(s.len());
    let end = s.iter().rposition(|&c| c > 0x20).map_or(start, |e| e + 1);
    &s[start..end.max(start)]
}

/// `GlobalObject.splitString(input, maxSize)`: `{[0]=first, [1]=rest}`,
/// split at the last whitespace (or after the last punctuation) before
/// `maxSize`, the rest cut to `maxSize`.
pub fn split_string(input: Option<&str>, max_size: i32) -> (String, String) {
    let Some(input) = input else {
        return (String::new(), String::new());
    };
    let s: Vec<u16> = input.encode_utf16().collect();
    if s.len() as i64 <= max_size as i64 {
        return (input.to_string(), String::new());
    }
    let mut split = -1i32;
    for i in (1..=max_size).rev() {
        if (i as usize) < s.len() && is_java_whitespace(s[i as usize]) {
            split = i;
            break;
        }
    }
    if split == -1 {
        for i in (1..=max_size).rev() {
            if (i as usize) < s.len() && is_punctuation(s[i as usize]) {
                split = i + 1;
                break;
            }
        }
    }
    if split == -1 {
        split = max_size;
    }
    let split = split as usize;
    let first = java_trim16(&s[..split]);
    let mut second = java_trim16(&s[split..]).to_vec();
    if second.len() as i64 > max_size as i64 {
        second.truncate(max_size.max(0) as usize);
    }
    (
        String::from_utf16_lossy(first),
        String::from_utf16_lossy(&second),
    )
}

/// Regex `\\s`: ASCII whitespace.
const JAVA_SPACE: [char; 6] = [' ', '\t', '\n', '\u{b}', '\u{c}', '\r'];

/// `GlobalObject.getTwoLetters`: initials of the first two words, upper-cased.
pub fn get_two_letters(input: &str) -> String {
    let cleaned: String = input
        .chars()
        .filter(|c| c.is_ascii_alphabetic() || JAVA_SPACE.contains(c))
        .collect();
    // cleaned.split("\\s+"): a leading separator yields an empty first word.
    let mut words: Vec<&str> = cleaned.split(JAVA_SPACE).collect();
    // Collapse runs of separators like "\\s+" (keeping a leading empty word).
    let lead_empty = words.first().is_some_and(|w| w.is_empty());
    words.retain(|w| !w.is_empty());
    if lead_empty && !cleaned.is_empty() {
        words.insert(0, "");
    }
    let first = |w: Option<&&str>, d: char| w.and_then(|w| w.chars().next()).unwrap_or(d);
    let l1 = first(words.first(), '-');
    let l2 = first(words.get(1), ' ');
    format!("{l1}{l2}").to_uppercase()
}

/// `LuaManager.isIndieStoneUrl`.
pub fn is_indie_stone_url(url: &str) -> bool {
    [
        "https://steamcommunity.com",
        "https://projectzomboid.com",
        "https://theindiestone.com",
        "https://pzwiki.net",
    ]
    .iter()
    .any(|base| url == *base || url.starts_with(&format!("{base}/")))
}

/// `SteamUtils.isValidSteamID`: a decimal integer in `[0, 2^64)`.
pub fn is_valid_steam_id(s: &str) -> bool {
    let digits = s.strip_prefix('+').unwrap_or(s);
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        // BigInteger accepts "-0"; any other negative is out of range.
        return s
            .strip_prefix('-')
            .is_some_and(|d| !d.is_empty() && d.bytes().all(|b| b == b'0'));
    }
    digits.trim_start_matches('0').parse::<u64>().is_ok() || digits.bytes().all(|b| b == b'0')
}

/// `ServerWorldDatabase.isValidUserName`.
pub fn is_valid_user_name(user: Option<&str>) -> bool {
    let Some(user) = user else { return false };
    let trimmed = user.trim_matches(|c: char| c <= ' ');
    let len16 = |s: &str| s.encode_utf16().count();
    if trimmed.is_empty()
        || user.contains([';', '@', '$', ',', '\\', '/', '.', '\'', '?', '"'])
        || len16(trimmed) < 2
        || len16(user) > 32
    {
        return false;
    }
    if user.contains('\0') {
        return false;
    }
    if trimmed == "admin" {
        return true;
    }
    !trimmed.to_lowercase().starts_with("admin")
}

// ---- registration -------------------------------------------------------------------

fn nil(c: &mut NativeCall<'_>) -> LuaResult<usize> {
    Ok(c.push(Value::Nil))
}

fn no_op(_: &mut NativeCall<'_>) -> LuaResult<usize> {
    Ok(0)
}

pub fn register(thread: &mut Thread, fs: Rc<RefCell<GameFs>>, events: EventManager) {
    register_ui_transition(thread);
    register_pz_math(thread);
    register_lua_event_manager(thread, events.clone());
    register_clipboard(thread);

    // Random numbers.
    global(thread, "ZombRand", |c| {
        let r = if c.nargs() >= 2 {
            zomb_rand_between(num_arg(c, 0), num_arg(c, 1))
        } else {
            zomb_rand(num_arg(c, 0))
        };
        Ok(c.push(r))
    });
    global(thread, "ZombRandBetween", |c| {
        let r = zomb_rand_between(num_arg(c, 0), num_arg(c, 1));
        Ok(c.push(r))
    });
    global(thread, "ZombRandFloat", |c| {
        let r = RAND.with(|r| r.next_range_f(num_arg(c, 0) as f32, num_arg(c, 1) as f32));
        Ok(c.push(r as f64))
    });

    global(thread, "triggerEvent", move |c| {
        let name = str_or_null(c, 0);
        events.trigger(c.thread, &name, &[]);
        Ok(0)
    });

    // Files under <cache>/Lua.
    let fs2 = fs.clone();
    global(thread, "getFileReader", move |c| {
        let name = str_or_null(c, 0);
        let create = bool_arg(c, 1);
        if has_relative_path(&name) {
            return nil(c);
        }
        let path = native_separators(&format!(
            "{}{MAIN_SEPARATOR}{name}",
            lua_cache_dir(&fs2.borrow())
        ));
        let p = PathBuf::from(&path);
        if !p.exists() && create {
            std::fs::File::create(&p).map_err(|e| io_error(&path, e))?;
        }
        if !p.exists() {
            return nil(c);
        }
        let text = std::fs::read(&p)
            .map(|b| String::from_utf8_lossy(&b).into_owned())
            .unwrap_or_default();
        let reader = LuaReader {
            lines: RefCell::new(split_lines(&text).into_iter()),
        };
        Ok(c.push(Value::Object(Rc::new(reader))))
    });
    let fs2 = fs.clone();
    global(thread, "getFileWriter", move |c| {
        let name = str_or_null(c, 0);
        let (create, append) = (bool_arg(c, 1), bool_arg(c, 2));
        if has_relative_path(&name)
            || !["ini", "cfg", "txt", "log", "json"].contains(&file_extension(&name))
        {
            return nil(c);
        }
        let path = native_separators(&format!(
            "{}{MAIN_SEPARATOR}{name}",
            lua_cache_dir(&fs2.borrow())
        ));
        if let Some(i) = path.rfind(MAIN_SEPARATOR) {
            let _ = std::fs::create_dir_all(&path[..i]);
        }
        let p = PathBuf::from(&path);
        if !p.exists() && create {
            let _ = std::fs::File::create(&p);
        }
        let file = std::fs::OpenOptions::new()
            .create(true)
            .write(true)
            .append(append)
            .truncate(!append)
            .open(&p);
        match file {
            Ok(f) => {
                let w = LuaFileWriter {
                    file: RefCell::new(Some(f)),
                    buf: RefCell::default(),
                };
                Ok(c.push(Value::Object(Rc::new(w))))
            }
            Err(_) => nil(c),
        }
    });
    let r = expose_class(thread, "java.io.BufferedReader", None);
    r.method("readLine", |c| {
        let line = with_this::<LuaReader, _>(c, |r| r.lines.borrow_mut().next())?;
        Ok(c.push(line.map_or(Value::Nil, Value::from)))
    });
    r.method("close", no_op);
    let w = expose_class(
        thread,
        "zombie.Lua.LuaManager$GlobalObject$LuaFileWriter",
        None,
    );
    w.method("write", |c| {
        let s = str_or_null(c, 1);
        with_this::<LuaFileWriter, _>(c, |w| w.buf.borrow_mut().push_str(&s))?;
        Ok(0)
    });
    w.method("writeln", |c| {
        let s = str_or_null(c, 1);
        with_this::<LuaFileWriter, _>(c, |w| {
            let mut b = w.buf.borrow_mut();
            b.push_str(&s);
            b.push_str(line_separator());
        })?;
        Ok(0)
    });
    w.method("close", |c| {
        with_this::<LuaFileWriter, _>(c, |w| {
            if let Some(mut f) = w.file.borrow_mut().take() {
                let _ = f.write_all(w.buf.borrow().as_bytes());
                w.buf.borrow_mut().clear();
            }
        })?;
        Ok(0)
    });

    // Sandbox presets under <cache>/Sandbox Presets.
    let fs2 = fs.clone();
    global(thread, "getSandboxPresets", move |c| {
        let dir = sandbox_cache_dir(&fs2.borrow());
        let mut names: Vec<String> = std::fs::read_dir(&dir)
            .map(|rd| {
                rd.flatten()
                    .map(|e| e.file_name().to_string_lossy().into_owned())
                    .filter(|n| n.ends_with(".cfg"))
                    .map(|n| n.replace(".cfg", ""))
                    .collect()
            })
            .unwrap_or_default();
        names.sort_by(|a, b| a.encode_utf16().cmp(b.encode_utf16()));
        Ok(c.push(crate::bind_java_util::list_value(
            names.into_iter().map(Value::from).collect(),
        )))
    });
    let fs2 = fs.clone();
    global(thread, "deleteSandboxPreset", move |c| {
        let name = str_or_null(c, 0);
        if has_relative_path(&name) {
            return Ok(0);
        }
        let _ = std::fs::remove_file(format!(
            "{}{MAIN_SEPARATOR}{name}.cfg",
            sandbox_cache_dir(&fs2.borrow())
        ));
        Ok(0)
    });
    let fs2 = fs;
    global(thread, "serverFileExists", move |c| {
        let name = native_separators(&str_or_null(c, 0));
        let p = format!(
            "{}{MAIN_SEPARATOR}Server{MAIN_SEPARATOR}{name}",
            fs2.borrow().cache_dir().display()
        );
        Ok(c.push(std::path::Path::new(&p).exists()))
    });

    // Strings.
    global(thread, "splitString", |c| {
        let s = str_arg(c, 0);
        let (a, b) = split_string(s.as_deref(), int_arg(c, 1));
        let t = TableRef::new();
        t.rawset(Value::Num(0.0), Value::from(a));
        t.rawset(Value::Num(1.0), Value::from(b));
        Ok(c.push(t))
    });
    global(thread, "getTwoLetters", |c| {
        let s = str_arg(c, 0).ok_or_else(npe)?;
        Ok(c.push(get_two_letters(&s)))
    });
    global(thread, "transformIntoKahluaTable", |c| {
        let t = TableRef::new();
        if let Value::Object(o) = c.arg(0) {
            // Entries are copied as they are (no Integer -> number conversion).
            if let Some(entries) = map_entries(o.as_ref()) {
                for (k, v) in entries {
                    t.rawset(k, v);
                }
            }
        }
        Ok(c.push(t))
    });
    global(thread, "openUrl", |c| {
        let url = str_arg(c, 0);
        if let Some(url) = url.filter(|u| is_indie_stone_url(u)) {
            open_in_browser(&url);
        }
        Ok(0)
    });
    global(thread, "showFolderInDesktop", |c| {
        let folder = str_or_null(c, 0);
        if std::path::Path::new(&folder).is_dir() {
            let cmd = if cfg!(windows) {
                "explorer.exe"
            } else if cfg!(target_os = "macos") {
                "open"
            } else {
                "xdg-open"
            };
            let _ = std::process::Command::new(cmd).arg(&folder).spawn();
        }
        Ok(0)
    });

    // Player slots and states.
    global(thread, "getMaxActivePlayers", |c| Ok(c.push(4.0)));
    // GameWindow.states.current is MainScreenState on the menu path.
    global(thread, "isIngameState", |c| Ok(c.push(false)));
    // SafetySystemManager.getCooldown(null): no connection, no cooldown.
    global(thread, "isQuitCooldown", |c| Ok(c.push(false)));

    // Network and Steam with no connection and Steam absent.
    for name in [
        "activateSteamOverlayToWebPage",
        "activateSteamOverlayToWorkshop",
        "activateSteamOverlayToWorkshopItem",
        "activateSteamOverlayToWorkshopUser",
        "backToSinglePlayer",
        "connectToServerStateCallback",
        "forceDisconnect",
        "scoreboardUpdate",
        "sendPersonalColor",
        "stopSendSecretKey",
    ] {
        global(thread, name, no_op);
    }
    for name in [
        "getClientUsername",
        "getCurrentUserSteamID",
        "getCurrentUserProfileName",
        "getSteamAvatarFromSteamID",
        "getSteamProfileNameFromSteamID",
        "getSteamWorkshopItemIDs",
        "getSteamWorkshopStagedItems",
    ] {
        global(thread, name, nil);
    }
    for name in [
        "canInviteFriends",
        "canModifyPlayerScoreboard",
        "getSteamScoreboard",
        "haveAccess",
    ] {
        global(thread, name, |c| Ok(c.push(false)));
    }
    global(thread, "isSteamServerBrowserEnabled", |c| Ok(c.push(true)));
    global(thread, "isPublicServerListAllowed", |c| Ok(c.push(true)));
    global(thread, "getFriendsList", |c| Ok(c.push(TableRef::new())));
    global(thread, "getOnlinePlayers", |c| {
        Ok(c.push(crate::bind_java_util::list_value(Vec::new())))
    });
    for name in ["getServerIP", "getServerName", "getServerPort"] {
        global(thread, name, |c| Ok(c.push("")));
    }
    global(thread, "getMaxUsernameLength", |c| Ok(c.push(32.0)));
    global(thread, "getMinUsernameLength", |c| Ok(c.push(2.0)));
    global(thread, "isValidSteamID", |c| {
        let r = str_arg(c, 0).is_some_and(|s| !s.is_empty() && is_valid_steam_id(&s));
        Ok(c.push(r))
    });
    global(thread, "isValidUserName", |c| {
        let r = is_valid_user_name(str_arg(c, 0).as_deref());
        Ok(c.push(r))
    });
    global(thread, "getHostByName", |c| {
        let host = str_arg(c, 0).map(|s| s.to_string()).unwrap_or_default();
        Ok(c.push(host_by_name(&host).map_or(Value::Nil, Value::from)))
    });
    // ServerBrowser with Steam absent: no servers, no requests.
    global(thread, "steamRequestInternetServersCount", |c| {
        Ok(c.push(0.0))
    });
    global(thread, "steamGetInternetServerDetails", nil);
    global(thread, "steamRequestServerDetails", |c| Ok(c.push(false)));
    global(thread, "steamRequestServerRules", |c| Ok(c.push(false)));
    global(thread, "steamRequestInternetServersList", no_op);
    global(thread, "steamReleaseInternetServersRequest", no_op);
    // The arguments the launcher passes as system properties
    // (-Dargs.server.connect/password); none are passed here.
    global(thread, "getServerAddressFromArgs", nil);
    global(thread, "getServerPasswordFromArgs", nil);
}

// ---- zombie.ui.UITransition ---------------------------------------------------------

thread_local! {
    /// `UITransition.currentTimeMS` / `elapsedTimeMS`.
    static TRANSITION_CLOCK: Cell<(i64, i64)> = const { Cell::new((0, 0)) };
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// `UITransition.UpdateAll()`, called once per frame by `UIManager.update`.
pub fn ui_transition_update_all() {
    TRANSITION_CLOCK.with(|c| {
        let current = now_ms();
        let (prev, _) = c.get();
        c.set((current, current - prev));
    });
}

/// `zombie.ui.UITransition`: a fade fraction advanced by frame time.
#[derive(Default)]
pub struct UITransition {
    duration: Cell<f32>,
    elapsed: Cell<f32>,
    frac: Cell<f32>,
    fade_out: Cell<bool>,
    ignore_update_time: Cell<bool>,
    update_time_ms: Cell<i64>,
}

impl LuaObject for UITransition {
    fn class_name(&self) -> &str {
        "zombie.ui.UITransition"
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

impl UITransition {
    pub fn new() -> Self {
        let t = UITransition::default();
        t.duration.set(100.0);
        t
    }

    pub fn init(&self, duration: f32, fade_out: bool) {
        self.duration.set(duration.max(1.0));
        let (frac, d) = (self.frac.get(), self.duration.get());
        self.elapsed.set(if frac >= 1.0 {
            0.0
        } else if self.fade_out.get() != fade_out {
            (1.0 - frac) * d
        } else {
            frac * d
        });
        self.fade_out.set(fade_out);
    }

    pub fn update(&self) {
        let (current, elapsed_ms) = TRANSITION_CLOCK.with(Cell::get);
        let d = self.duration.get();
        if !self.ignore_update_time.get()
            && self.update_time_ms.get() != 0
            && self.update_time_ms.get() + (d as i64) < current
        {
            self.elapsed.set(d);
        }
        self.update_time_ms.set(current);
        self.frac.set(self.elapsed.get() / d);
        self.elapsed
            .set((self.elapsed.get() + elapsed_ms as f32).min(d));
    }

    pub fn fraction(&self) -> f32 {
        if self.fade_out.get() {
            1.0 - self.frac.get()
        } else {
            self.frac.get()
        }
    }

    pub fn set_fade_in(&self, fade_in: bool) {
        if fade_in {
            if self.fade_out.get() {
                self.init(100.0, false);
            }
        } else if !self.fade_out.get() {
            self.init(200.0, true);
        }
    }
}

fn register_ui_transition(thread: &mut Thread) {
    let t = expose_class(thread, "zombie.ui.UITransition", None);
    t.ctor(|c| Ok(c.push(Value::Object(Rc::new(UITransition::new())))));
    t.static_fn("UpdateAll", |_| {
        ui_transition_update_all();
        Ok(0)
    });
    t.method("init", |c| {
        let (d, out) = (num_arg(c, 1) as f32, bool_arg(c, 2));
        with_this::<UITransition, _>(c, |t| t.init(d, out))?;
        Ok(0)
    });
    t.method("update", |c| {
        with_this::<UITransition, _>(c, UITransition::update)?;
        Ok(0)
    });
    t.method("fraction", |c| {
        let v = with_this::<UITransition, _>(c, UITransition::fraction)?;
        Ok(c.push(v as f64))
    });
    t.method("setFadeIn", |c| {
        let b = bool_arg(c, 1);
        with_this::<UITransition, _>(c, |t| t.set_fade_in(b))?;
        Ok(0)
    });
    t.method("reset", |c| {
        with_this::<UITransition, _>(c, |t| t.elapsed.set(0.0))?;
        Ok(0)
    });
    t.method("setIgnoreUpdateTime", |c| {
        let b = bool_arg(c, 1);
        with_this::<UITransition, _>(c, |t| t.ignore_update_time.set(b))?;
        Ok(0)
    });
    t.method("getElapsed", |c| {
        let v = with_this::<UITransition, _>(c, |t| t.elapsed.get())?;
        Ok(c.push(v as f64))
    });
    t.method("setElapsed", |c| {
        let v = num_arg(c, 1) as f32;
        with_this::<UITransition, _>(c, |t| t.elapsed.set(v))?;
        Ok(0)
    });
}

// ---- zombie.core.math.PZMath (the statics menu code uses) ------------------------------

fn register_pz_math(thread: &mut Thread) {
    let m = expose_class(thread, "zombie.core.math.PZMath", None);
    // Kahlua passes Lua numbers as Double, which selects the double overload.
    m.static_fn("clamp", |c| {
        let (v, lo, hi) = (num_arg(c, 0), num_arg(c, 1), num_arg(c, 2));
        Ok(c.push(if v < lo {
            lo
        } else if v > hi {
            hi
        } else {
            v
        }))
    });
    m.static_fn("clampFloat", |c| {
        let (v, lo, hi) = (
            num_arg(c, 0) as f32,
            num_arg(c, 1) as f32,
            num_arg(c, 2) as f32,
        );
        Ok(c.push(
            (if v < lo {
                lo
            } else if v > hi {
                hi
            } else {
                v
            }) as f64,
        ))
    });
    m.static_fn("clamp_01", |c| {
        let v = num_arg(c, 0) as f32;
        Ok(c.push(v.clamp(0.0, 1.0) as f64))
    });
    m.static_fn("lerp", |c| {
        let (a, b, t) = (
            num_arg(c, 0) as f32,
            num_arg(c, 1) as f32,
            num_arg(c, 2) as f32,
        );
        Ok(c.push((a + (b - a) * t) as f64))
    });
    m.static_fn("radToDeg", |c| {
        let r = num_arg(c, 0) as f32;
        Ok(c.push((57.295776f32 * r) as f64))
    });
}

// ---- zombie.Lua.LuaEventManager -----------------------------------------------------------

fn register_lua_event_manager(thread: &mut Thread, events: EventManager) {
    let m = expose_class(thread, "zombie.Lua.LuaEventManager", None);
    m.static_fn("AddEvent", move |c| {
        let name = str_or_null(c, 0);
        events.add_event(&name);
        Ok(0)
    });
}

// ---- Clipboard ---------------------------------------------------------------------------------

thread_local! {
    /// Text set through `Clipboard.setClipboard` until the window backend
    /// provides the system clipboard.
    pub static CLIPBOARD: RefCell<String> = const { RefCell::new(String::new()) };
}

fn register_clipboard(thread: &mut Thread) {
    let c = expose_class(thread, "zombie.core.Clipboard", None);
    c.static_fn("setClipboard", |c| {
        let s = str_or_null(c, 0);
        CLIPBOARD.with(|b| *b.borrow_mut() = s);
        Ok(0)
    });
    c.static_fn("getClipboard", |c| {
        let s = CLIPBOARD.with(|b| b.borrow().clone());
        Ok(c.push(s))
    });
}

/// Reads a Java map object's entries for `transformIntoKahluaTable`;
/// `None` when the object is not a map this source knows.
pub type MapSource = fn(&dyn LuaObject) -> Option<Vec<(Value, Value)>>;

thread_local! {
    static MAP_SOURCES: RefCell<Vec<MapSource>> = RefCell::new(vec![java_map_entries as MapSource]);
}

fn java_map_entries(o: &dyn LuaObject) -> Option<Vec<(Value, Value)>> {
    o.as_any().downcast_ref::<crate::bind_java_util::JavaMap>().map(|m| m.ordered())
}

/// Lets another binding's map objects (for example a view onto an engine
/// `HashMap`) be passed to `transformIntoKahluaTable`.
pub fn register_map_source(f: MapSource) {
    MAP_SOURCES.with(|s| {
        let mut s = s.borrow_mut();
        if !s.iter().any(|g| *g as usize == f as usize) {
            s.push(f);
        }
    });
}

fn map_entries(o: &dyn LuaObject) -> Option<Vec<(Value, Value)>> {
    let sources = MAP_SOURCES.with(|s| s.borrow().clone());
    sources.iter().find_map(|f| f(o))
}

fn npe() -> LuaError {
    LuaError::Runtime {
        message: "null".into(),
        class: "java.lang.NullPointerException",
    }
}

fn io_error(path: &str, e: std::io::Error) -> LuaError {
    LuaError::Runtime {
        message: format!("{path}: {e}"),
        class: "java.io.IOException",
    }
}

/// `InetAddress.getByName(host).getHostAddress()`; an empty name is the
/// loopback address.
fn host_by_name(host: &str) -> Option<String> {
    use std::net::ToSocketAddrs;
    if host.is_empty() {
        return Some("127.0.0.1".into());
    }
    (host, 0)
        .to_socket_addrs()
        .ok()?
        .next()
        .map(|a| a.ip().to_string())
}

/// `DesktopBrowser.openURL`.
fn open_in_browser(url: &str) {
    let r = if cfg!(windows) {
        std::process::Command::new("rundll32")
            .args(["url.dll,FileProtocolHandler", url])
            .spawn()
    } else if cfg!(target_os = "macos") {
        std::process::Command::new("open").arg(url).spawn()
    } else {
        std::process::Command::new("xdg-open").arg(url).spawn()
    };
    let _ = r;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zomb_rand_ranges() {
        for _ in 0..1000 {
            let v = zomb_rand(5.9);
            assert!((0.0..5.0).contains(&v) && v.fract() == 0.0);
            let n = zomb_rand(-3.0);
            assert!((-2.0..=0.0).contains(&n));
        }
        assert_eq!(zomb_rand(0.0), 0.0);
        assert_eq!(zomb_rand(0.5), 0.0);
    }

    #[test]
    fn split_and_letters() {
        assert_eq!(
            split_string(Some("hello world foo"), 8),
            ("hello".into(), "world fo".into())
        );
        assert_eq!(split_string(Some("abc"), 8), ("abc".into(), "".into()));
        assert_eq!(split_string(None, 8), ("".into(), "".into()));
        assert_eq!(
            split_string(Some("abcdefghij"), 4),
            ("abcd".into(), "efgh".into())
        );
        assert_eq!(get_two_letters("project zomboid"), "PZ");
        assert_eq!(get_two_letters("x"), "X ");
        assert_eq!(get_two_letters("123"), "- ");
        assert_eq!(get_two_letters(" a b"), "-A");
    }

    #[test]
    fn ui_transition_fades() {
        ui_transition_update_all();
        let t = UITransition::new();
        t.set_fade_in(false);
        assert_eq!(t.fraction(), 1.0);
        t.set_fade_in(true);
        t.elapsed.set(50.0);
        t.update();
        assert_eq!(t.fraction(), 0.5);
    }

    #[test]
    fn validators() {
        assert!(is_indie_stone_url("https://pzwiki.net/wiki/X"));
        assert!(!is_indie_stone_url("https://pzwiki.net.evil"));
        assert!(is_valid_steam_id("76561198000000000"));
        assert!(!is_valid_steam_id("18446744073709551616"));
        assert!(!is_valid_steam_id("abc"));
        assert!(is_valid_user_name(Some("Bob")));
        assert!(!is_valid_user_name(Some("a.b")));
        assert!(!is_valid_user_name(Some("administrator")));
        assert!(is_valid_user_name(Some("admin")));
    }
}
