//! The server list database, checked against runs of the original
//! `AccountDBHelper` on the same sequence of calls.

use pz_network::{Account, AccountDb, LocalDateTime, Server};
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

fn scratch(tag: &str) -> PathBuf {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    let d = std::env::temp_dir().join(format!("pz-network-{tag}-{}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn t(s: &str) -> Option<LocalDateTime> {
    Some(LocalDateTime::parse_db(s).unwrap())
}

fn opt<T: ToString>(v: &Option<T>) -> String {
    v.as_ref().map_or("null".into(), |x| x.to_string())
}

fn server(name: &str, ip: &str, port: i32, pwd: Option<&str>, desc: Option<&str>) -> Server {
    let mut s = Server {
        name: Some(name.into()),
        port,
        description: desc.map(Into::into),
        ..Server::default()
    };
    s.set_ip(Some(ip.into()));
    s.set_server_password(pwd.map(Into::into));
    s.last_online = t("2026-01-02 03:04:05");
    s.last_data_update = None;
    s
}

fn account(
    user: &str,
    pwd: Option<&str>,
    save: bool,
    relay: bool,
    auth: i32,
    played: i32,
    logon: Option<&str>,
) -> Account {
    Account {
        user_name: Some(user.into()),
        pwd: pwd.map(Into::into),
        save_pwd: save,
        use_steam_relay: relay,
        auth_type: auth,
        time_played: played,
        last_logon: logon.and_then(t),
        ..Account::default()
    }
}

fn dump(db: &AccountDb, tag: &str, out: &mut Vec<String>) {
    let l = db.get_server_list().unwrap();
    out.push(format!("== {tag} ({})", l.len()));
    for s in &l {
        out.push(format!(
            "server {}|{}|{}|{}|{}|{}|{}|{}",
            s.id,
            opt(&s.name),
            opt(&s.host),
            s.port,
            s.server_pwd,
            opt(&s.description),
            opt(&s.last_online.map(|x| x.to_java_string())),
            opt(&s.last_data_update.map(|x| x.to_java_string())),
        ));
        for a in &s.accounts {
            let a = a.borrow();
            out.push(format!(
                "  account {}|{}|{}|{}|{}|{}|{}|{}|{}|{}",
                a.id,
                opt(&a.user_name),
                opt(&a.pwd),
                a.save_pwd,
                a.use_steam_relay,
                a.auth_type,
                opt(&a.player_first_and_last_name),
                a.time_played,
                opt(&a.last_logon_string()),
                a.icon.is_some()
            ));
        }
    }
}

#[test]
fn crud_matches_the_original() {
    let dir = scratch("crud");
    let db = AccountDb::new(&dir, false);
    let mut out = Vec::new();
    dump(&db, "fresh", &mut out);
    let mut a = server("Alpha", "10.0.0.1", 16261, None, Some("first"));
    let mut b = server("Beta", "beta.example", 16262, Some("pw"), None);
    let mut c = server("Gamma", "10.0.0.3", 1, Some(""), Some(""));
    let mut d = server("Delta", "10.0.0.4", 2, Some("x"), Some("y"));
    for s in [&mut a, &mut b, &mut c, &mut d] {
        s.id = db.save_new_server(s).unwrap();
        out.push(format!("saved {}", s.id));
    }
    let mut accts = [
        account(
            "u1",
            Some("p1"),
            true,
            false,
            1,
            5,
            Some("2026-03-01 10:00:00"),
        ),
        account("u2", None, false, true, 2, 0, Some("2026-05-01 10:00:00")),
        account("u3", Some("p3"), true, true, 3, 7, None),
        account(
            "u4",
            Some("p4"),
            false,
            false,
            1,
            9,
            Some("2026-05-01 10:00:00"),
        ),
        account(
            "u5",
            Some("p5"),
            false,
            false,
            1,
            9,
            Some("2025-12-31 23:59:59"),
        ),
    ];
    let owners = [&a, &b, &a, &d, &d];
    for (acc, s) in accts.iter_mut().zip(owners) {
        acc.id = db.save_new_account(s, acc).unwrap();
        out.push(format!("account {}", acc.id));
    }
    dump(&db, "after inserts", &mut out);
    accts[2].player_first_and_last_name = Some("Kate Smith".into());
    accts[2].last_logon = t("2027-01-01 00:00:00");
    out.push(format!(
        "updateAccount {}",
        db.update_account(&accts[2]).unwrap()
    ));
    out.push(format!(
        "deleteAccount {}",
        db.delete_account(&accts[3]).unwrap()
    ));
    out.push(format!(
        "deleteAccount again {}",
        db.delete_account(&accts[3]).unwrap()
    ));
    b.name = Some("Beta2".into());
    b.set_ip(Some("beta2.example".into()));
    b.last_online = None;
    out.push(format!("updateServer {}", db.update_server(&b).unwrap()));
    out.push(format!("deleteServer {}", db.delete_server(&c).unwrap()));
    out.push(format!(
        "deleteServer again {}",
        db.delete_server(&c).unwrap()
    ));
    let e = server("Eps", "10.0.0.5", 3, Some(""), Some(""));
    out.push(format!("saved {}", db.save_new_server(&e).unwrap()));
    dump(&db, "final", &mut out);

    let expected = "\
== fresh (0)
saved 1
saved 2
saved 3
saved 4
account 1
account 2
account 3
account 4
account 5
== after inserts (4)
server 2|Beta|beta.example|16262|pw|null|2026-01-02T03:04:05|null
  account 2|u2|null|false|true|2|null|0|2026-05-01 10:00:00|false
server 4|Delta|10.0.0.4|2|x|y|2026-01-02T03:04:05|null
  account 4|u4|p4|false|false|1|null|9|2026-05-01 10:00:00|false
  account 5|u5|p5|false|false|1|null|9|2025-12-31 23:59:59|false
server 1|Alpha|10.0.0.1|16261||first|2026-01-02T03:04:05|null
  account 1|u1|p1|true|false|1|null|5|2026-03-01 10:00:00|false
  account 3|u3|p3|true|true|3|null|7|null|false
server 3|Gamma|10.0.0.3|1|||2026-01-02T03:04:05|null
updateAccount true
deleteAccount true
deleteAccount again false
updateServer true
deleteServer true
deleteServer again false
saved 5
== final (4)
server 1|Alpha|10.0.0.1|16261||first|2026-01-02T03:04:05|null
  account 1|u1|p1|true|false|1|null|5|2026-03-01 10:00:00|false
  account 3|u3|p3|true|true|3|Kate Smith|7|2027-01-01 00:00:00|false
server 2|Beta2|beta2.example|16262|pw|null|null|null
  account 2|u2|null|false|true|2|null|0|2026-05-01 10:00:00|false
server 4|Delta|10.0.0.4|2|x|y|2026-01-02T03:04:05|null
  account 5|u5|p5|false|false|1|null|9|2025-12-31 23:59:59|false
server 5|Eps|10.0.0.5|3|||2026-01-02T03:04:05|null";
    assert_eq!(out.join("\n"), expected);
    if let Ok(o) = std::process::Command::new("sqlite3")
        .arg("-readonly")
        .arg(db.db_path())
        .arg("PRAGMA integrity_check")
        .output()
    {
        if o.status.success() {
            assert_eq!(String::from_utf8_lossy(&o.stdout).trim(), "ok");
        }
    }
    if std::env::var_os("PZ_NETWORK_KEEP").is_some() {
        eprintln!("kept {}", dir.display());
    } else {
        let _ = std::fs::remove_dir_all(&dir);
    }
}

fn write_legacy(dir: &std::path::Path, text: &str) {
    std::fs::create_dir_all(dir.join("Lua")).unwrap();
    std::fs::write(dir.join("Lua/ServerList.txt"), text).unwrap();
}

#[test]
fn legacy_import_matches_the_original() {
    let dir = scratch("legacy");
    let db = AccountDb::new(&dir, false);
    let mut out = Vec::new();
    write_legacy(
        &dir,
        "name=Old One\nip=1.2.3.4\nport=16261\nserverpassword=sp\ndescription=hello\nuser=bob\npassword=pw\nremember=true\nname=Old Two\nip=5.6.7.8\nport=99\n",
    );
    // Visible to one read, never committed; the file is gone either way.
    dump(&db, "legacy 1", &mut out);
    dump(&db, "legacy 2", &mut out);
    assert!(!dir.join("Lua/ServerList.txt").exists());
    // A write commits whatever the import inserted.
    write_legacy(&dir, "name=Old One\nip=1.2.3.4\nport=16261\n");
    out.push(format!(
        "saved {}",
        db.save_new_server(&server("Eps", "10.0.0.5", 3, Some(""), Some("")))
            .unwrap()
    ));
    dump(&db, "legacy committed", &mut out);
    assert_eq!(
        out.join("\n"),
        "\
== legacy 1 (1)
server 1|Old One|1.2.3.4|16261|sp|hello|null|null
== legacy 2 (0)
saved 2
== legacy committed (2)
server 1|Old One|1.2.3.4|16261|||null|null
server 2|Eps|10.0.0.5|3|||2026-01-02T03:04:05|null"
    );
    // An unparsable port escapes and leaves the file in place.
    write_legacy(&dir, "name=Bad\nport=abc\n");
    let e = db.get_server_list().unwrap_err();
    assert_eq!(e.to_string(), "For input string: \"abc\"");
    assert!(dir.join("Lua/ServerList.txt").exists());
    let _ = std::fs::remove_dir_all(&dir);
}
