//! The saved server list: `zombie.network.Server`, `zombie.network.Account`
//! and the globals over `AccountDBHelper` (`getServerList`,
//! `addServerToAccountList`, ...), backed by `pz_network`.
//!
//! Offline only: `getIp2` resolves IP literals but does no DNS lookup
//! (a host name it cannot read as an address gives null, as an unknown host
//! does), and `getLastOnline`/`getLastDataUpdate` return the
//! `LocalDateTime` as its `toString()` text. Steam mode is off, so the list
//! lives in `<cache>/db/ServerList.db`.

use crate::bind_java_util::JavaList;
use crate::expose::{bool_arg, expose_class, global, int_arg, num_arg, str_arg};
use pz_fs::GameFs;
use pz_kahlua::value::LuaObject;
use pz_kahlua::{LuaError, LuaResult, NativeCall, TableRef, Thread, Value};
use pz_network::{Account, AccountDb, AccountRef, LocalDateTime, Server};
use std::any::Any;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::{Rc, Weak};

pub const SERVER_CLASS: &str = "zombie.network.Server";
pub const ACCOUNT_CLASS: &str = "zombie.network.Account";

/// A Lua `Server`.
pub struct ServerObj {
    pub server: RefCell<Server>,
    /// `serverIcon`, `serverLoginScreen`, `serverLoadingScreen` textures.
    textures: RefCell<[Value; 3]>,
}

/// A Lua `Account`, one per shared account.
pub struct AccountObj {
    pub account: AccountRef,
    /// `getIcon()`'s texture, made from the stored bytes on first use.
    icon: RefCell<Option<Value>>,
}

impl LuaObject for ServerObj {
    fn class_name(&self) -> &str {
        SERVER_CLASS
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

impl LuaObject for AccountObj {
    fn class_name(&self) -> &str {
        ACCOUNT_CLASS
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

thread_local! {
    static ACCOUNTS: RefCell<HashMap<usize, Weak<AccountObj>>> = RefCell::new(HashMap::new());
}

fn server_value(s: Server) -> Value {
    Value::Object(Rc::new(ServerObj { server: RefCell::new(s), textures: RefCell::new(Default::default()) }))
}

/// The Lua object of `a`, the same one each time.
fn account_value(a: &AccountRef) -> Value {
    let key = Rc::as_ptr(a) as usize;
    ACCOUNTS.with(|m| {
        let mut m = m.borrow_mut();
        if let Some(o) = m.get(&key).and_then(Weak::upgrade) {
            return Value::Object(o);
        }
        m.retain(|_, w| w.strong_count() > 0);
        let o = Rc::new(AccountObj { account: a.clone(), icon: RefCell::new(None) });
        m.insert(key, Rc::downgrade(&o));
        Value::Object(o)
    })
}

fn npe() -> LuaError {
    LuaError::Runtime { message: String::new(), class: "java.lang.NullPointerException" }
}

fn object<T: 'static>(v: Value) -> LuaResult<Rc<dyn LuaObject>> {
    match v {
        Value::Object(o) if o.as_any().is::<T>() => Ok(o),
        Value::Nil => Err(npe()),
        other => Err(LuaError::Runtime {
            message: format!(
                "class {} cannot be cast to class {}",
                pz_kahlua::stdlib::java_class_name(&other),
                std::any::type_name::<T>()
            ),
            class: "java.lang.ClassCastException",
        }),
    }
}

fn server_arg(c: &NativeCall<'_>, i: usize) -> LuaResult<Rc<dyn LuaObject>> {
    object::<ServerObj>(c.arg(i))
}

fn account_arg(c: &NativeCall<'_>, i: usize) -> LuaResult<Rc<dyn LuaObject>> {
    object::<AccountObj>(c.arg(i))
}

fn srv(o: &Rc<dyn LuaObject>) -> &ServerObj {
    o.as_any().downcast_ref().unwrap()
}

fn acc(o: &Rc<dyn LuaObject>) -> &AccountObj {
    o.as_any().downcast_ref().unwrap()
}

fn string(s: Option<&str>) -> Value {
    s.map_or(Value::Nil, Value::from)
}

fn opt_str(c: &NativeCall<'_>, i: usize) -> Option<String> {
    str_arg(c, i).map(|s| s.to_string())
}

/// A `LocalDateTime` argument: only nil (null) is understood; any other
/// value keeps the current time, as no Lua code builds one.
fn date_arg(c: &NativeCall<'_>, i: usize) -> Option<LocalDateTime> {
    match c.arg(i) {
        Value::Nil => None,
        Value::Str(s) => LocalDateTime::parse_db(&s),
        _ => Some(LocalDateTime::now()),
    }
}

/// `getIp2()`: offline, only addresses written as IP literals resolve.
fn resolve(s: &mut Server) -> Option<String> {
    if s.ip.is_none() {
        let host = s.host.clone()?;
        let addr: std::net::IpAddr = host.parse().ok()?;
        s.ip = Some(addr.to_string());
    }
    s.ip.clone()
}

/// The account list database under the game's cache directory.
fn db(fs: &RefCell<GameFs>) -> AccountDb {
    AccountDb::new(fs.borrow().cache_dir().to_path_buf(), false)
}

fn db_error(e: pz_network::account_db::Error) -> LuaError {
    match e {
        pz_network::account_db::Error::LegacyPort(_) => {
            LuaError::Runtime { message: e.to_string(), class: "java.lang.NumberFormatException" }
        }
        e => LuaError::runtime(e.to_string()),
    }
}

pub fn register(thread: &mut Thread, fs: Rc<RefCell<GameFs>>) {
    let cache = fs;
    register_server(thread);
    register_account(thread);

    let c0 = cache.clone();
    global(thread, "getServerList", move |c| {
        let list = db(&c0).get_server_list().map_err(db_error)?;
        let t = TableRef::new();
        for (i, s) in list.into_iter().enumerate() {
            t.set_int((i + 1) as i64, server_value(s));
        }
        Ok(c.push(Value::Table(t)))
    });
    global(thread, "getServerListFile", |c| Ok(c.push("ServerList.txt")));
    let c0 = cache.clone();
    global(thread, "addServerToAccountList", move |c| {
        let o = server_arg(c, 0)?;
        db(&c0).save_new_server(&srv(&o).server.borrow()).map_err(db_error)?;
        Ok(0)
    });
    let c0 = cache.clone();
    global(thread, "updateServerToAccountList", move |c| {
        // A null server is rejected before any database work.
        let Value::Object(_) = c.arg(0) else { return Ok(0) };
        let o = server_arg(c, 0)?;
        db(&c0).update_server(&srv(&o).server.borrow()).map_err(db_error)?;
        Ok(0)
    });
    let c0 = cache.clone();
    global(thread, "deleteServerToAccountList", move |c| {
        let o = server_arg(c, 0)?;
        db(&c0).delete_server(&srv(&o).server.borrow()).map_err(db_error)?;
        Ok(0)
    });
    let c0 = cache.clone();
    global(thread, "addAccountToAccountList", move |c| {
        let s = server_arg(c, 0)?;
        let a = account_arg(c, 1)?;
        db(&c0).save_new_account(&srv(&s).server.borrow(), &acc(&a).account.borrow()).map_err(db_error)?;
        Ok(0)
    });
    let c0 = cache.clone();
    global(thread, "updateAccountToAccountList", move |c| {
        let a = account_arg(c, 0)?;
        db(&c0).update_account(&acc(&a).account.borrow()).map_err(db_error)?;
        Ok(0)
    });
    let c0 = cache;
    global(thread, "deleteAccountToAccountList", move |c| {
        let a = account_arg(c, 0)?;
        db(&c0).delete_account(&acc(&a).account.borrow()).map_err(db_error)?;
        Ok(0)
    });
}

fn register_server(thread: &mut Thread) {
    let b = expose_class(thread, SERVER_CLASS, None);
    b.ctor(|c| Ok(c.push(server_value(Server::default()))));
    let this = |c: &NativeCall<'_>| server_arg(c, 0);
    macro_rules! get {
        ($name:literal, |$s:ident| $e:expr) => {
            b.method($name, move |c| {
                let o = this(c)?;
                let $s = srv(&o).server.borrow();
                let v: Value = $e;
                drop($s);
                Ok(c.push(v))
            });
        };
    }
    macro_rules! set {
        ($name:literal, |$s:ident, $c:ident| $e:expr) => {
            b.method($name, move |$c| {
                let o = this($c)?;
                let mut $s = srv(&o).server.borrow_mut();
                $e;
                Ok(0)
            });
        };
    }
    get!("getID", |s| s.id.into());
    set!("setID", |s, c| s.id = int_arg(c, 1));
    get!("getNeedSave", |s| s.need_save.into());
    set!("setNeedSave", |s, c| s.need_save = bool_arg(c, 1));
    get!("getPort", |s| s.port.into());
    set!("setPort", |s, c| s.port = int_arg(c, 1));
    b.method("getIp2", move |c| {
        let o = this(c)?;
        let v = resolve(&mut srv(&o).server.borrow_mut());
        Ok(c.push(string(v.as_deref())))
    });
    get!("getIp", |s| string(s.host.as_deref()));
    set!("setIp", |s, c| s.set_ip(opt_str(c, 1)));
    let streamer = || crate::bind_core::shared_core().borrow().get_option_streamer_mode();
    get!("getDisplayAddress", |s| {
        if streamer() {
            "xxx.xxx.xxx.xxx:xxxxxx".into()
        } else {
            format!("{}:{}", s.host.as_deref().unwrap_or("null"), s.port).into()
        }
    });
    get!("getDisplayIp", |s| if streamer() { "xxx.xxx.xxx.xxx".into() } else { string(s.host.as_deref()) });
    get!("getDisplayPort", |s| if streamer() { "xxxxxx".into() } else { s.port.to_string().into() });
    get!("getLocalIP", |s| string(s.local_ip.as_deref()));
    set!("setLocalIP", |s, c| s.local_ip = opt_str(c, 1));
    get!("getServerPassword", |s| s.server_pwd.as_str().into());
    set!("setServerPassword", |s, c| s.set_server_password(opt_str(c, 1)));
    get!("getDescription", |s| string(s.description.as_deref()));
    set!("setDescription", |s, c| s.description = opt_str(c, 1));
    get!("getLastOnline", |s| string(s.last_online.map(|t| t.to_java_string()).as_deref()));
    set!("setLastOnline", |s, c| s.last_online = date_arg(c, 1));
    set!("setLastOnlineNow", |s, c| s.last_online = Some(LocalDateTime::now()));
    get!("getLastDataUpdate", |s| string(s.last_data_update.map(|t| t.to_java_string()).as_deref()));
    set!("setLastDataUpdate", |s, c| s.last_data_update = date_arg(c, 1));
    set!("setLastDataUpdateNow", |s, c| s.last_data_update = Some(LocalDateTime::now()));
    b.method("getAccounts", move |c| {
        let o = this(c)?;
        let read_o = o.clone();
        let write_o = o.clone();
        let list = JavaList::live(
            move || srv(&read_o).server.borrow().accounts.iter().map(account_value).collect(),
            Some(Box::new(move |items: Vec<Value>| {
                let mut v = Vec::with_capacity(items.len());
                for it in items {
                    v.push(acc(&account_arg_value(it)?).account.clone());
                }
                srv(&write_o).server.borrow_mut().accounts = v;
                Ok(())
            })),
        );
        Ok(c.push(Value::Object(Rc::new(list))))
    });
    b.method("addAccount", move |c| {
        let o = this(c)?;
        if c.nargs() >= 6 {
            let (u, p) = (opt_str(c, 1), opt_str(c, 2));
            let (save, relay, auth) = (bool_arg(c, 3), bool_arg(c, 4), int_arg(c, 5));
            srv(&o).server.borrow_mut().add_new_account(u, p, save, relay, auth);
        } else {
            // A null account is added as null in the original; it has no
            // representation here, so it is skipped.
            if let Value::Object(_) = c.arg(1) {
                let a = account_arg(c, 1)?;
                srv(&o).server.borrow_mut().accounts.push(acc(&a).account.clone());
            }
        }
        Ok(0)
    });
    b.method("removeAccount", move |c| {
        let o = this(c)?;
        if let Value::Object(_) = c.arg(1) {
            let a = account_arg(c, 1)?;
            srv(&o).server.borrow_mut().remove_account(&acc(&a).account);
        }
        Ok(0)
    });
    // The deprecated single-account accessors (first account).
    macro_rules! first_get {
        ($name:literal, $default:expr, |$a:ident| $e:expr) => {
            b.method($name, move |c| {
                let o = this(c)?;
                let s = srv(&o).server.borrow();
                let v: Value = match s.accounts.first() {
                    Some(a) => {
                        let $a = a.borrow();
                        $e
                    }
                    None => $default,
                };
                drop(s);
                Ok(c.push(v))
            });
        };
    }
    macro_rules! first_set {
        ($name:literal, |$a:ident, $c:ident| $e:expr) => {
            b.method($name, move |$c| {
                let o = this($c)?;
                let first = srv(&o).server.borrow_mut().first_account();
                let mut $a = first.borrow_mut();
                $e;
                Ok(0)
            });
        };
    }
    first_get!("getUserName", "".into(), |a| string(a.user_name.as_deref()));
    first_set!("setUserName", |a, c| a.user_name = opt_str(c, 1));
    first_get!("getPwd", "".into(), |a| string(a.pwd.as_deref()));
    b.method("setPwd", move |c| {
        let o = this(c)?;
        let pwd = opt_str(c, 1);
        let pwd = if bool_arg(c, 2) { pwd.map(|p| pz_network::crypt::hash_password(&p)) } else { pwd };
        let first = srv(&o).server.borrow_mut().first_account();
        first.borrow_mut().pwd = pwd;
        Ok(0)
    });
    first_get!("getUseSteamRelay", true.into(), |a| a.use_steam_relay.into());
    first_set!("setUseSteamRelay", |a, c| a.use_steam_relay = bool_arg(c, 1));
    first_get!("isSavePwd", true.into(), |a| a.save_pwd.into());
    first_set!("setSavePwd", |a, c| a.save_pwd = bool_arg(c, 1));
    first_get!("getAuthType", 1.into(), |a| a.auth_type.into());
    first_set!("setAuthType", |a, c| a.auth_type = int_arg(c, 1));
    get!("getLastUpdate", |s| s.last_update.into());
    set!("setLastUpdate", |s, c| s.last_update = int_arg(c, 1));
    get!("getLastWipe", |s| (s.last_wipe as f64).into());
    set!("setLastWipe", |s, c| s.last_wipe = num_arg(c, 1) as i64);
    get!("getLastWipeDate", |s| s.last_wipe_date().into());
    get!("getPlayers", |s| string(s.players.as_deref()));
    set!("setPlayers", |s, c| s.players = opt_str(c, 1));
    get!("isOpen", |s| s.open.into());
    set!("setOpen", |s, c| s.open = bool_arg(c, 1));
    get!("isPublic", |s| s.public.into());
    set!("setPublic", |s, c| s.public = bool_arg(c, 1));
    get!("getVersion", |s| string(s.version.as_deref()));
    set!("setVersion", |s, c| s.version = opt_str(c, 1));
    get!("getMaxPlayers", |s| string(s.max_players.as_deref()));
    set!("setMaxPlayers", |s, c| s.max_players = opt_str(c, 1));
    get!("getMods", |s| string(s.mods.as_deref()));
    set!("setMods", |s, c| s.mods = opt_str(c, 1));
    get!("getName", |s| string(s.name.as_deref()));
    set!("setName", |s, c| s.name = opt_str(c, 1));
    get!("getPing", |s| string(s.ping.as_deref()));
    set!("setPing", |s, c| s.ping = opt_str(c, 1));
    get!("isPasswordProtected", |s| s.password_protected.into());
    set!("setPasswordProtected", |s, c| s.password_protected = bool_arg(c, 1));
    get!("getSteamId", |s| string(s.steam_id.as_deref()));
    set!("setSteamId", |s, c| s.steam_id = opt_str(c, 1));
    get!("isHosted", |s| s.hosted.into());
    set!("setHosted", |s, c| s.hosted = bool_arg(c, 1));
    get!("getMapName", |s| string(s.map_name.as_deref()));
    set!("setMapName", |s, c| s.map_name = opt_str(c, 1));
    for (i, (getter, setter)) in [
        ("getServerIcon", "setServerIcon"),
        ("getServerLoginScreen", "setServerLoginScreen"),
        ("getServerLoadingScreen", "setServerLoadingScreen"),
    ]
    .into_iter()
    .enumerate()
    {
        b.method(getter, move |c| {
            let o = this(c)?;
            let v = srv(&o).textures.borrow()[i].clone();
            Ok(c.push(v))
        });
        b.method(setter, move |c| {
            let o = this(c)?;
            srv(&o).textures.borrow_mut()[i] = c.arg(1);
            Ok(0)
        });
    }
    get!("getServerCustomizationLastUpdate", |s| s.server_customization_last_update.into());
    get!("getTimeFromServerCustomizationLastUpdate", |s| s.time_from_server_customization_last_update().into());
    set!("setServerCustomizationLastUpdate", |s, c| s.server_customization_last_update = int_arg(c, 1));
    set!("updateServerCustomizationLastUpdate", |s, c| s.update_server_customization_last_update());
    get!("isFeatured", |s| s.featured.into());
    set!("setFeatured", |s, c| s.featured = bool_arg(c, 1));
    get!("isResponded", |s| s.responded.into());
    set!("setResponded", |s, c| s.responded = bool_arg(c, 1));
}

fn account_arg_value(v: Value) -> LuaResult<Rc<dyn LuaObject>> {
    object::<AccountObj>(v)
}

fn register_account(thread: &mut Thread) {
    let b = expose_class(thread, ACCOUNT_CLASS, None);
    b.ctor(|c| Ok(c.push(account_value(&Rc::new(RefCell::new(Account::default()))))));
    let this = |c: &NativeCall<'_>| account_arg(c, 0);
    macro_rules! get {
        ($name:literal, |$a:ident| $e:expr) => {
            b.method($name, move |c| {
                let o = this(c)?;
                let $a = acc(&o).account.borrow();
                let v: Value = $e;
                drop($a);
                Ok(c.push(v))
            });
        };
    }
    macro_rules! set {
        ($name:literal, |$a:ident, $c:ident| $e:expr) => {
            b.method($name, move |$c| {
                let o = this($c)?;
                let mut $a = acc(&o).account.borrow_mut();
                $e;
                Ok(0)
            });
        };
    }
    get!("getID", |a| a.id.into());
    set!("setID", |a, c| a.id = int_arg(c, 1));
    get!("getUserName", |a| string(a.user_name.as_deref()));
    set!("setUserName", |a, c| a.user_name = opt_str(c, 1));
    get!("getPlayerFirstAndLastName", |a| string(a.player_first_and_last_name.as_deref()));
    set!("setPlayerFirstAndLastName", |a, c| a.player_first_and_last_name = opt_str(c, 1));
    get!("getPwd", |a| string(a.pwd.as_deref()));
    set!("setPwd", |a, c| a.pwd = opt_str(c, 1));
    b.method("encryptPwd", move |c| {
        let o = this(c)?;
        let p = str_arg(c, 1).ok_or_else(npe)?;
        acc(&o).account.borrow_mut().pwd = Some(pz_network::crypt::hash_password(&p));
        Ok(0)
    });
    get!("getUseSteamRelay", |a| a.use_steam_relay.into());
    set!("setUseSteamRelay", |a, c| a.use_steam_relay = bool_arg(c, 1));
    get!("isSavePwd", |a| a.save_pwd.into());
    set!("setSavePwd", |a, c| a.save_pwd = bool_arg(c, 1));
    get!("getAuthType", |a| a.auth_type.into());
    set!("setAuthType", |a, c| a.auth_type = int_arg(c, 1));
    get!("getTimePlayed", |a| a.time_played.into());
    set!("setTimePlayed", |a, c| a.time_played = int_arg(c, 1));
    get!("getLastLogon", |a| string(a.last_logon_string().as_deref()));
    set!("setLastLogon", |a, c| a.last_logon = date_arg(c, 1));
    set!("setLastLogonNow", |a, c| a.last_logon = Some(LocalDateTime::now()));
    b.method("getIcon", move |c| {
        let o = this(c)?;
        let a = acc(&o);
        if let Some(v) = a.icon.borrow().clone() {
            return Ok(c.push(v));
        }
        let (id, bytes) = {
            let x = a.account.borrow();
            (x.id, x.icon.clone())
        };
        let v = match bytes.and_then(|b| icon_texture(id, &b)) {
            Some(t) => crate::bind_render::texture_value(Some(t)),
            None => Value::Nil,
        };
        *a.icon.borrow_mut() = Some(v.clone());
        Ok(c.push(v))
    });
    b.method("setIcon", move |c| {
        let o = this(c)?;
        *acc(&o).icon.borrow_mut() = Some(c.arg(1));
        Ok(0)
    });
}

/// `new Texture("accountIcon" + id, stream, false)`.
fn icon_texture(id: i32, png: &[u8]) -> Option<pz_render::Texture> {
    let mut d = pz_render::png::PngDecoder::new(png, false).ok()?;
    let (w, h) = (d.width, d.height);
    let mut rgba = vec![0u8; (w * h * 4) as usize];
    d.decode_rgba(&mut rgba, (w * 4) as usize).ok()?;
    let tm = crate::bind_render::textures();
    let t = tm.borrow_mut().texture_from_pixels(&format!("accountIcon{id}"), w, h, rgba, 0);
    Some(t)
}

#[cfg(test)]
mod tests {
    use super::*;
    use pz_kahlua::exec::java_value_of;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn setup() -> (Thread, std::path::PathBuf) {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("pz-bind-server-list-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("game/media")).unwrap();
        let mut fs = GameFs::new(dir.join("game"));
        fs.set_cache_dir(dir.join("cache"));
        let mut t = Thread::new(TableRef::new());
        pz_kahlua::stdlib::install(&mut t, None).unwrap();
        crate::bind_java_util::register(&mut t);
        register(&mut t, Rc::new(RefCell::new(fs)));
        (t, dir)
    }

    fn run(t: &mut Thread, src: &str) -> Vec<String> {
        let f = pz_kahlua::stdlib::load_string(src, Some("test"), t.globals().clone()).unwrap();
        t.pcall(&f, &[]).map_err(|(m, _)| java_value_of(&m)).unwrap().iter().map(java_value_of).collect()
    }

    #[test]
    fn edit_server_and_account_flow() {
        let (mut t, dir) = setup();
        // ISMPEditServer / ISMPEditAccount / MultiplayerUI:refreshList.
        let r = run(
            &mut t,
            r#"
            local fresh = #getServerList()
            local s = Server.new()
            s:setName("Home"); s:setIp("192.168.0.10"); s:setPort(16261)
            s:setServerPassword(nil); s:setDescription("lan")
            addServerToAccountList(s)
            local list = getServerList()
            local srv = list[1]
            local a = Account.new()
            a:setUserName("kate"); a:encryptPwd(""); a:setSavePwd(true); a:setAuthType(1)
            addAccountToAccountList(srv, a)
            srv = getServerList()[1]
            local accs = srv:getAccounts()
            local first = accs:get(0)
            local same = first == srv:getAccounts():get(0)
            first:setPlayerFirstAndLastName("Kate Smith")
            updateAccountToAccountList(first)
            local again = getServerList()[1]:getAccounts():get(0)
            return fresh, #list, srv:getID(), srv:getName(), srv:getIp(), srv:getIp2(), srv:getPort(),
                srv:getServerPassword(), srv:getDescription(), srv:getDisplayAddress(), accs:size(),
                first:getUserName(), first:getPwd(), first:isSavePwd(), same,
                again:getPlayerFirstAndLastName(), srv:getUserName(), getServerListFile()
            "#,
        );
        assert_eq!(
            r,
            [
                "0.0",
                "1.0",
                "1.0",
                "Home",
                "192.168.0.10",
                "192.168.0.10",
                "16261.0",
                "",
                "lan",
                "192.168.0.10:16261",
                "1.0",
                "kate",
                "",
                "true",
                "true",
                "Kate Smith",
                "kate",
                "ServerList.txt"
            ]
        );
        assert!(dir.join("cache/db/ServerList.db").exists());
        let r = run(
            &mut t,
            r#"
            local srv = getServerList()[1]
            deleteAccountToAccountList(srv:getAccounts():get(0))
            local n = getServerList()[1]:getAccounts():size()
            deleteServerToAccountList(srv)
            local h = Server.new()
            h:setIp("not-an-address")
            return n, #getServerList(), h:getIp2(), Server.new():getName(), Server.new():getPort()
            "#,
        );
        assert_eq!(r, ["0.0", "0.0", "null", "My Server", "16262.0"]);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
