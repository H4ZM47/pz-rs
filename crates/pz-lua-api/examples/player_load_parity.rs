//! Runs a local game's player load on a save copy in the original's order
//! (`VehiclesDB2.init`, `PlayerDBHelper.isPlayerAlive`, `LoadPlayerForInfo`,
//! `LoadPlayer`) and writes what a local harness writes for the original:
//! `players.txt` (the info result and every stored row's fields and bytes
//! CRC) and `player<id>.bin` (the bytes). No game data is part of this
//! repository.
//!
//! ```text
//! player_load_parity <save dir> <out dir>
//! ```

use std::fmt::Write as _;

use pz_lua_api::player_db::{
    load_local_player_1, load_player_for_info, LocalPlayerLoad, PlayerDb, VehicleStore,
};

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let save = &a[1];
    let out = std::path::Path::new(&a[2]);
    std::fs::create_dir_all(out).unwrap();
    let mut vehicles = VehicleStore::default();
    vehicles.create(save).unwrap();
    let alive = pz_lua_api::savefile::is_player_alive(false, save, 1);
    let mut db = PlayerDb::new(false, save);
    let mut s = String::new();
    writeln!(s, "alive {alive}").unwrap();
    let map_p = std::path::Path::new(save).join("map_p.bin").exists();
    writeln!(s, "map_p {map_p}").unwrap();
    if !map_p {
        match load_player_for_info(&mut db, &mut vehicles, save, (0, 0)) {
            LocalPlayerLoad::Db(Some(i)) => writeln!(
                s,
                "info true {} {} {} {} {}",
                i.world_xa, i.world_ya, i.world_za, i.s_world_x, i.s_world_y
            )
            .unwrap(),
            LocalPlayerLoad::Db(None) => writeln!(s, "info false 0 0 0 0 0").unwrap(),
            LocalPlayerLoad::MapPBin(_) => unreachable!(),
        }
        // `LoadPlayer` imports again before `loadLocalPlayer(1)`.
        let _ = load_local_player_1(&mut db, &mut vehicles, save);
    }
    let ids: Vec<i32> = db.used_ids.iter().copied().collect();
    writeln!(s, "usedIds {ids:?}").unwrap();
    for id in ids {
        let Some(p) = db.load_local_player(id) else {
            writeln!(s, "player {id} none").unwrap();
            continue;
        };
        writeln!(
            s,
            "player {id} wv={} x={} y={} z={} dead={} name={} len={} crc={:x}",
            p.world_version,
            p.x.to_bits() as i32,
            p.y.to_bits() as i32,
            p.z.to_bits() as i32,
            p.is_dead,
            p.name.as_deref().unwrap_or("null"),
            p.bytes.len(),
            pz_inventory::io::crc32(&p.bytes)
        )
        .unwrap();
        std::fs::write(out.join(format!("player{id}.bin")), &p.bytes).unwrap();
    }
    std::fs::write(out.join("players.txt"), s).unwrap();
}
