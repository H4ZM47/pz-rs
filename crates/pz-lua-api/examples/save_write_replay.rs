//! Replays a script of `players.db` / `vehicles.db` writes and save-folder
//! Lua calls through the port, for differential tests against the
//! original (run the same script through a Java harness on another scratch
//! copy and compare the results).
//!
//! Usage: `save_write_replay <game-dir> <user-folder> <script>`. Run it
//! only on scratch copies. Script lines (`_` in names stands for a space,
//! `-` for null):
//!
//! ```text
//! dir <save dir>                      database ops below work in this folder
//! pcreate | pclose                    PlayerDB store (PlayerDBHelper.create)
//! psave id x y z dead name wv seed len
//! setp1 id                            PlayerDBHelper.setPlayer1
//! premove name world                  PlayerDBHelper.removePlayer
//! vcreate | vclose                    VehiclesDB2.SQLStore.create
//! vupsert id wx wy x y wv inMeta seed len   (updateVehicle)
//! vdel id                             removeVehicle
//! vlegacy                             old player tables in vehicles.db
//! vdropold                            importPlayersFromOldDB's DROP TABLEs
//! lua <expression>                    Lua with the save globals bound
//! ```
//!
//! Row data is `len` bytes of xorshift64 from `seed`.

use pz_lua_api::player_db::{PlayerData, PlayerStore, VehicleBuffer, VehicleStore};
use std::collections::BTreeSet;
use std::rc::Rc;

fn bytes(mut seed: u64, n: usize) -> Vec<u8> {
    (0..n)
        .map(|_| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed as u8
        })
        .collect()
}

fn name(s: &str) -> Option<String> {
    (s != "-").then(|| s.replace('_', " "))
}

fn main() {
    let mut args = std::env::args().skip(1);
    let game = args.next().expect("game directory");
    let user = args.next().expect("user folder");
    let script = std::fs::read_to_string(args.next().expect("script")).expect("read script");
    let mut gfs = pz_fs::GameFs::new(game);
    gfs.set_cache_dir(user);
    let mut mgr =
        pz_lua_api::LuaManager::init(Rc::new(std::cell::RefCell::new(gfs))).expect("init");
    let mut dir = String::new();
    let mut players = PlayerStore::default();
    let mut vehicles = VehicleStore::default();
    for line in script.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (op, rest) = line.split_once(' ').unwrap_or((line, ""));
        let a: Vec<&str> = rest.split_whitespace().collect();
        let int = |i: usize| a[i].parse::<i32>().unwrap();
        let flt = |i: usize| a[i].parse::<f32>().unwrap();
        match op {
            "dir" => dir = rest.to_string(),
            "pcreate" => {
                let mut used = BTreeSet::new();
                players.init(false, &dir, &mut used).expect("players.db");
            }
            "pclose" => players.reset(),
            "psave" => {
                let p = PlayerData {
                    sql_id: int(0),
                    x: flt(1),
                    y: flt(2),
                    z: flt(3),
                    is_dead: a[4] == "1",
                    name: name(a[5]),
                    world_version: int(6),
                    bytes: bytes(a[7].parse().unwrap(), a[8].parse().unwrap()),
                };
                if let Err(e) = players.save(&p) {
                    println!("psave {}: error {e}", p.sql_id);
                }
            }
            "setp1" => {
                if let Err(e) = pz_lua_api::player_db::set_player1(false, &dir, int(0)) {
                    println!("setp1: error {e}");
                }
            }
            "premove" => {
                let (n, w) = (name(a[0]), name(a[1]));
                if let Err(e) =
                    pz_lua_api::player_db::remove_player(&dir, n.as_deref(), w.as_deref())
                {
                    println!("premove: error {e}");
                }
            }
            "vcreate" => vehicles.create(&dir).expect("vehicles.db"),
            "vclose" => vehicles.reset(),
            "vupsert" => {
                let v = VehicleBuffer {
                    id: int(0),
                    wx: int(1),
                    wy: int(2),
                    x: flt(3),
                    y: flt(4),
                    world_version: int(5),
                    in_meta: a[6] == "1",
                    bytes: bytes(a[7].parse().unwrap(), a[8].parse().unwrap()),
                };
                vehicles.update_vehicle(&v);
            }
            "vdel" => vehicles.remove_vehicle(int(0)),
            "vlegacy" => {
                let c = vehicles.conn.as_mut().expect("vehicles open");
                c.execute(pz_lua_api::player_db::CREATE_LOCAL_PLAYERS)
                    .unwrap();
                c.execute(pz_lua_api::player_db::CREATE_NETWORK_PLAYERS)
                    .unwrap();
                c.commit().unwrap();
            }
            "vdropold" => vehicles.drop_old_player_tables(),
            "lua" => {
                let f = pz_kahlua::stdlib::load_string(
                    &format!("return {rest}"),
                    Some("replay"),
                    mgr.thread.globals().clone(),
                )
                .expect("compile");
                match mgr.thread.pcall(&f, &[]) {
                    Ok(r) => println!(
                        "lua {rest} -> {}",
                        r.iter()
                            .map(pz_kahlua::exec::java_value_of)
                            .collect::<Vec<_>>()
                            .join(",")
                    ),
                    Err((e, _)) => println!("lua {rest} -> error {e:?}"),
                }
            }
            _ => panic!("unknown op {op}"),
        }
    }
}
