use super::*;
use crate::{BuildingDef, BuildingIdx, GameRand, IsoMetaCell, RoomDef, RoomIdx};

struct Fixed;
impl GameRand for Fixed {
    fn next(&mut self, _max: i32) -> i32 {
        7
    }
}

struct Env;
impl MetaLoadEnv for Env {
    fn now_millis(&mut self) -> i64 {
        1234
    }
    fn nearest_spawn_region(&mut self, _x: i32, _y: i32) -> Option<String> {
        Some("Muldraugh, KY".into())
    }
}

/// A 2x2 grid; cell (10, 20) has map data with two rooms (one
/// user-defined) and one building, cell (11, 20) exists without map data.
fn grid() -> IsoMetaGrid {
    let mut g = IsoMetaGrid::with_bounds(10, 20, 11, 21);
    let mut cell = IsoMetaCell::new(10, 20);
    cell.info = Some((0, "10_20.lotheader".into()));
    for (i, (meta, user)) in [(100i64, false), (101, true), (102, false)]
        .into_iter()
        .enumerate()
    {
        let mut r = RoomDef::new(i as i64, "room".into());
        r.meta_id = meta;
        r.user_defined = user;
        let idx = RoomIdx(g.room_defs.len() as u32);
        g.room_defs.push(r);
        cell.room_list.push(idx);
        cell.room_by_meta_id.insert(meta, idx);
    }
    let mut b = BuildingDef::new(&mut Fixed);
    b.meta_id = 500;
    let idx = BuildingIdx(0);
    g.building_defs.push(b);
    cell.buildings.push(idx);
    cell.building_by_meta_id.insert(500, idx);
    *g.cell_slot(0, 0) = Some(cell);
    *g.cell_slot(1, 0) = Some(IsoMetaCell::new(11, 20));
    g
}

#[test]
fn cells_layout_and_grid_round_trip() {
    let mut g = grid();
    g.room_defs[2].explored = true;
    g.room_defs[2].roof_fixed = true;
    g.building_defs[0].alarmed = true;
    let world = MetaWorld {
        stash: Some(StashState::default()),
        ..MetaWorld::default()
    };
    let bytes = save_map_meta(&g, &world, NetRole::SinglePlayer);
    let mut want = b"META\0\0\0\xf9".to_vec();
    for v in [10, 20, 11, 21] {
        want.put_i32(v);
    }
    // cell (10,20): two non-user rooms, one building.
    want.put_i32(2);
    want.put_i64(100);
    want.put_i16(0);
    want.put_i64(102);
    want.put_i16(9);
    want.put_i32(1);
    want.put_i64(500);
    want.extend([1, 0, 0, 0, 7, 0, 0, 0xff, 0xff, 0xff, 0xff]);
    want.put_i32(10000000);
    // the other three cells are empty.
    for _ in 0..3 {
        want.put_i32(0);
        want.put_i32(0);
    }
    // part 1: four empty lists, three empty stash lists, no unique stories.
    for _ in 0..8 {
        want.put_i32(0);
    }
    assert_eq!(bytes, want);

    let mut g2 = grid();
    let mut w = MetaWorld::default();
    let rep = load_map_meta(&mut g2, &mut w, &bytes, NetRole::SinglePlayer, &mut Env).unwrap();
    assert!(!rep.size_mismatch);
    assert_eq!(rep.total_alarmed, 1);
    assert!(g2.room_defs[2].roof_fixed && g2.room_defs[2].explored);
    assert_eq!(save_map_meta(&g2, &w, NetRole::SinglePlayer), bytes);

    // Without an initialised stash system nothing is written for it, and
    // such a file does not load back in single player (as in the original).
    let bytes = save_map_meta(&g, &MetaWorld::default(), NetRole::SinglePlayer);
    assert_eq!(bytes.len(), want.len() - 12);
    assert!(load_map_meta(
        &mut grid(),
        &mut MetaWorld::default(),
        &bytes,
        NetRole::SinglePlayer,
        &mut Env
    )
    .is_err());
}

#[test]
fn unknown_and_cellless_records_are_dropped() {
    let mut m = grid().meta_cells();
    m.cells[0].0.push(RoomRecord {
        meta_id: 999,
        explored: true,
        ..RoomRecord::default()
    });
    m.cells[1].1.push(BuildingRecord {
        meta_id: 501,
        ..BuildingRecord::default()
    });
    let mut g = grid();
    let rep = g.apply_meta_cells(&m);
    assert_eq!(rep.unknown_rooms, vec![(10, 20, 999)]);
    assert!(rep.unknown_buildings.is_empty());
    assert_eq!(g.meta_cells(), grid().meta_cells());
}

#[test]
fn old_versions_and_flag_bits() {
    let mut data = b"META\0\0\0\xc8".to_vec();
    for v in [0, 0, 0, 0] {
        data.put_i32(v);
    }
    data.put_i32(1);
    data.put_i64(1);
    data.put_i16(0x7ff2);
    data.put_i32(1);
    data.put_i64(2);
    data.extend([5, 0, 0, 0, 1, 1, 0]);
    data.put_i32(3);
    let mut r = JavaReader::new(&data);
    let m = MetaCells::read(&mut r).unwrap();
    assert_eq!(r.remaining(), 0);
    let room = m.cells[0].0[0];
    assert!(room.lights_active && !room.explored && !room.done_spawn && !room.roof_fixed);
    let b = m.cells[0].1[0];
    assert!(b.alarmed && b.seen && !b.has_been_visited);
    assert_eq!((b.loot_respawn_hour, b.alarm_decay), (3, 0));
    let mut out = Vec::new();
    m.write(&mut out);
    // Rewritten at the current version, with only the known bits and an
    // alarm decay.
    assert_eq!(&out[4..8], &[0, 0, 0, 0xf9]);
    assert_eq!(out.len(), data.len() + 4);
}

fn sample_world() -> MetaWorld {
    let mut sh = SafeHouse::new(1, 2, 3, 4, "bob");
    sh.add_player("ann");
    sh.add_player("bob");
    sh.datetime_created = 77;
    sh.location = Some("Rosewood, KY".into());
    sh.players_respawn.push("ann".into());
    MetaWorld {
        safehouses: vec![sh],
        non_pvp_zones: vec![NonPvpZone {
            x: 1,
            y: 2,
            x2: 3,
            y2: 4,
            size: 4,
            title: "safe".into(),
        }],
        factions: vec![
            Faction {
                name: "f".into(),
                owner: "bob".into(),
                tag: Some(("TAG".into(), [0.5, 0.25, 1.0])),
                players: vec!["ann".into()],
            },
            Faction {
                name: "g".into(),
                owner: "cy".into(),
                tag: None,
                players: vec![],
            },
        ],
        designation_zones: vec![DesignationZone {
            id: 123456.0,
            x: 10,
            y: 11,
            z: 0,
            w: 5,
            h: 6,
            zone_type: "AnimalZone".into(),
            name: "pen".into(),
            hour_last_seen: 40,
        }],
        stash: Some(StashState {
            possible_stashes: vec![StashBuilding {
                stash_name: "Stash_X".into(),
                building_x: 5,
                building_y: 6,
            }],
            buildings_to_do: vec![],
            already_read_map: vec!["Map".into()],
        }),
        unique_rds_spawned: vec!["RDSHouse".into()],
    }
}

#[test]
fn world_round_trips_in_every_role() {
    // A client never writes the stash part (nor its offset) and expects a
    // server's layout when reading, so only these two round-trip.
    for role in [NetRole::SinglePlayer, NetRole::Server] {
        let file = MapMetaFile {
            cells: grid().meta_cells(),
            world: sample_world(),
        };
        let bytes = file.to_bytes(role);
        let back = MapMetaFile::parse(&bytes, role, &mut Env).unwrap();
        assert_eq!(back.to_bytes(role), bytes, "{role:?}");
        assert_eq!(back.world, file.world);
    }
    // A client reads past a server file's stash data using the offset.
    let file = MapMetaFile {
        cells: grid().meta_cells(),
        world: sample_world(),
    };
    let back =
        MapMetaFile::parse(&file.to_bytes(NetRole::Server), NetRole::Client, &mut Env).unwrap();
    assert_eq!(back.world.unique_rds_spawned, vec!["RDSHouse".to_string()]);
}

#[test]
fn load_side_effects() {
    let mut w = sample_world();
    // The owner is always a player; zero creation time and empty location
    // are filled in; tag colours are raised to 0.19; non-animal designation
    // zones are dropped; known zone ids are merged, new ones get ordered
    // corners.
    w.safehouses[0].players = vec!["ann".into(), "ann".into()];
    w.safehouses[0].datetime_created = 0;
    w.safehouses[0].location = None;
    w.factions[0].tag = Some(("T".into(), [0.0, 0.5, -1.0]));
    w.designation_zones[0].w = -5;
    let mut other = w.designation_zones[0].clone();
    other.zone_type = "Other".into();
    w.designation_zones.push(other);
    let mut again = w.designation_zones[0].clone();
    again.name = "renamed".into();
    w.designation_zones.push(again);
    let file = MapMetaFile {
        cells: grid().meta_cells(),
        world: w,
    };
    let back = MapMetaFile::parse(
        &file.to_bytes(NetRole::SinglePlayer),
        NetRole::SinglePlayer,
        &mut Env,
    )
    .unwrap()
    .world;
    assert_eq!(
        back.safehouses[0].players,
        vec!["bob".to_string(), "ann".to_string()]
    );
    assert_eq!(back.safehouses[0].datetime_created, 1234);
    assert_eq!(
        back.safehouses[0].location.as_deref(),
        Some("Muldraugh, KY")
    );
    assert_eq!(back.factions[0].tag.as_ref().unwrap().1, [0.19, 0.5, 0.19]);
    assert_eq!(back.designation_zones.len(), 1);
    let z = &back.designation_zones[0];
    assert_eq!((z.x, z.w, z.name.as_str()), (5, -5, "renamed"));
}

#[test]
fn old_safehouse_versions() {
    let mut out = Vec::new();
    for v in [1, 2, 3, 4] {
        out.put_i32(v);
    }
    out.write_string_utf(Some("bob"));
    out.put_i32(0);
    out.put_i64(9);
    out.write_string_utf(Some("t"));
    out.put_i32(0);
    let s = SafeHouse::load(&mut JavaReader::new(&out), 215, &mut Env).unwrap();
    assert_eq!((s.hit_points, s.datetime_created, s.location), (0, 0, None));
}
