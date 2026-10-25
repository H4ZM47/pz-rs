//! `zombie.network.Server` and `zombie.network.Account`.

use crate::datetime::LocalDateTime;
use std::cell::RefCell;
use std::rc::Rc;

/// An account shared between its server's list and whoever holds it.
pub type AccountRef = Rc<RefCell<Account>>;

/// `zombie.network.Account`.
#[derive(Clone, Debug)]
pub struct Account {
    pub id: i32,
    pub user_name: Option<String>,
    pub pwd: Option<String>,
    pub auth_type: i32,
    pub use_steam_relay: bool,
    pub save_pwd: bool,
    pub player_first_and_last_name: Option<String>,
    /// PNG bytes of the account icon (`getIcon()` makes a texture of them).
    pub icon: Option<Vec<u8>>,
    pub time_played: i32,
    pub last_logon: Option<LocalDateTime>,
}

impl Default for Account {
    /// `new Account()`.
    fn default() -> Self {
        Account {
            id: 0,
            user_name: Some(String::new()),
            pwd: Some(String::new()),
            auth_type: 1,
            use_steam_relay: false,
            save_pwd: true,
            player_first_and_last_name: None,
            icon: None,
            time_played: 0,
            last_logon: Some(LocalDateTime::now()),
        }
    }
}

impl Account {
    /// `getLastLogon()`: formatted as the database stores it.
    pub fn last_logon_string(&self) -> Option<String> {
        self.last_logon.map(|t| t.format_db())
    }
}

/// `zombie.network.Server`, the fields a client keeps offline.
#[derive(Clone, Debug)]
pub struct Server {
    pub id: i32,
    pub name: Option<String>,
    /// Resolved address (`getIp2`); `None` after `setIp` until resolved.
    pub ip: Option<String>,
    /// The address as entered (`getIp`).
    pub host: Option<String>,
    pub local_ip: Option<String>,
    pub port: i32,
    pub server_pwd: String,
    pub description: Option<String>,
    pub last_update: i32,
    pub last_wipe: i64,
    pub players: Option<String>,
    pub max_players: Option<String>,
    pub open: bool,
    pub public: bool,
    pub version: Option<String>,
    pub mods: Option<String>,
    pub password_protected: bool,
    pub steam_id: Option<String>,
    pub ping: Option<String>,
    pub hosted: bool,
    pub need_save: bool,
    pub map_name: Option<String>,
    pub last_online: Option<LocalDateTime>,
    pub last_data_update: Option<LocalDateTime>,
    pub accounts: Vec<AccountRef>,
    pub server_customization_last_update: i32,
    pub featured: bool,
    pub responded: bool,
}

impl Default for Server {
    /// `new Server()`.
    fn default() -> Self {
        let now = LocalDateTime::now();
        Server {
            id: 0,
            name: Some("My Server".into()),
            ip: Some("127.0.0.1".into()),
            host: Some("127.0.0.1".into()),
            local_ip: Some(String::new()),
            port: 16262,
            server_pwd: String::new(),
            description: Some(String::new()),
            last_update: 0,
            last_wipe: 0,
            players: None,
            max_players: None,
            open: false,
            public: true,
            version: None,
            mods: None,
            password_protected: false,
            steam_id: None,
            ping: Some("-1".into()),
            hosted: false,
            need_save: false,
            map_name: Some("Default".into()),
            last_online: Some(now),
            last_data_update: Some(now),
            accounts: Vec::new(),
            server_customization_last_update: 0,
            featured: false,
            responded: false,
        }
    }
}

impl Server {
    /// `setIp(ip)`: the resolved address is cleared.
    pub fn set_ip(&mut self, ip: Option<String>) {
        self.ip = None;
        self.host = ip;
    }

    /// `setServerPassword(pwd)`: null becomes "".
    pub fn set_server_password(&mut self, pwd: Option<String>) {
        self.server_pwd = pwd.unwrap_or_default();
    }

    /// `addAccount(username, password, savePwd, useSteamRelay, authType)`.
    pub fn add_new_account(
        &mut self,
        user_name: Option<String>,
        pwd: Option<String>,
        save_pwd: bool,
        use_steam_relay: bool,
        auth_type: i32,
    ) {
        let a = Account {
            user_name,
            pwd,
            save_pwd,
            use_steam_relay,
            auth_type,
            ..Account::default()
        };
        self.accounts.push(Rc::new(RefCell::new(a)));
    }

    /// `removeAccount(account)`: the first occurrence, by identity.
    pub fn remove_account(&mut self, a: &AccountRef) {
        if let Some(i) = self.accounts.iter().position(|x| Rc::ptr_eq(x, a)) {
            self.accounts.remove(i);
        }
    }

    /// The deprecated single-account accessors act on the first account,
    /// adding one when a setter finds none.
    pub fn first_account(&mut self) -> AccountRef {
        if self.accounts.is_empty() {
            self.accounts
                .push(Rc::new(RefCell::new(Account::default())));
        }
        self.accounts[0].clone()
    }

    /// `getLastWipeDate()`: `yyyy-MM-dd` in the system time zone, "" if unset.
    pub fn last_wipe_date(&self) -> String {
        if self.last_wipe <= 0 {
            return String::new();
        }
        let Ok(ts) = jiff::Timestamp::from_second(self.last_wipe) else {
            return String::new();
        };
        let d = ts.to_zoned(jiff::tz::TimeZone::system()).date();
        format!("{:04}-{:02}-{:02}", d.year(), d.month(), d.day())
    }

    /// `getTimeFromServerCustomizationLastUpdate()`.
    pub fn time_from_server_customization_last_update(&self) -> i32 {
        if self.server_customization_last_update == 0 {
            return 1_000_000;
        }
        (jiff::Timestamp::now().as_second() - self.server_customization_last_update as i64) as i32
    }

    /// `updateServerCustomizationLastUpdate()`.
    pub fn update_server_customization_last_update(&mut self) {
        self.server_customization_last_update = jiff::Timestamp::now().as_second() as i32;
    }
}
