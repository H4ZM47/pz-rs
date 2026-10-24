use super::*;

struct Counter(i64);
impl UuidSource for Counter {
    fn random_uuid(&mut self) -> JavaUuid {
        self.0 += 1;
        JavaUuid {
            msb: self.0,
            lsb: -self.0,
        }
    }
}

#[test]
fn hash_order_matches_java_buckets() {
    // "Aa" and "BB" share hash 2112 (bucket 0 of 16, after "" / null at 0).
    let order = java_hash_order(&[
        Some("b"),
        Some("BB"),
        None,
        Some("a"),
        Some("Aa"),
        Some("b"),
    ]);
    assert_eq!(
        order,
        vec![Some("BB"), None, Some("Aa"), Some("a"), Some("b")]
    );
    // Thirteen keys resize the table to 32 buckets.
    let keys: Vec<String> = (0..13)
        .map(|i| ((b'a' + i * 15 % 26) as char).to_string())
        .collect();
    let refs: Vec<Option<&str>> = keys.iter().map(|k| Some(k.as_str())).collect();
    let order = java_hash_order(&refs);
    let mut want = refs.clone();
    want.sort_by_key(|k| (k.unwrap().as_bytes()[0] as usize) & 31);
    assert_eq!(order, want);
}

fn grid() -> IsoMetaGrid {
    let mut g = IsoMetaGrid::with_bounds(0, 0, 1, 1);
    g.register_zone("Forest1", "Forest", 10, 10, 0, 20, 20);
    let pts = [5, 5, 30, 5, 30, 40];
    g.register_zone_with_geometry(
        "Road",
        "Nav",
        5,
        5,
        0,
        26,
        36,
        ZoneGeometryType::Polyline,
        Some(&pts),
        3,
    );
    g.register_zone_with_geometry(
        "Pond",
        "Water",
        5,
        5,
        -1,
        26,
        36,
        ZoneGeometryType::Polygon,
        Some(&pts),
        0,
    );
    let id = g.register_zone("Forest1", "Forest", 100, 100, 0, 8, 8);
    g.zone_mut(id).set_original_name(None);
    g.zone_mut(id).hour_last_seen = 77;
    g.zone_mut(id).have_construction = true;
    g.register_zone("Gen", "WorldGen", 0, 0, 0, 8, 8);
    // Suspicious (too wide): created but not part of `zones`.
    g.register_zone("Huge", "Forest", 0, 0, 0, 5000, 8);
    g
}

#[test]
fn layout_and_round_trip() {
    let mut g = grid();
    let mut spawned = SpawnedZombieZones::default();
    spawned.put("Forest1".into(), vec![JavaUuid { msb: 9, lsb: 9 }]);
    spawned.put("Road".into(), vec![]);
    let bytes = g.save_zones(&spawned, &mut Counter(0)).unwrap();
    assert_eq!(&bytes[..8], b"ZONE\0\0\0\xf9");
    // Eight unique strings plus null (the cleared original name).
    assert_eq!(&bytes[8..12], &[0, 0, 0, 9]);

    let mut g2 = IsoMetaGrid::with_bounds(0, 0, 1, 1);
    let mut spawned2 = SpawnedZombieZones::default();
    let rep = g2
        .load_zones(&bytes, -1, &mut spawned2, &mut Counter(100))
        .unwrap();
    assert_eq!((rep.read, rep.skipped_world_gen, rep.suspicious), (5, 1, 0));
    assert_eq!(spawned2, spawned);
    let z = g2.zone(g2.zones[3]);
    assert_eq!(z.get_original_name(), Some(""));
    assert_eq!((z.hour_last_seen, z.have_construction), (77, true));
    let polyline = g2.zone(g2.zones[1]);
    assert_eq!((polyline.polyline_width, polyline.points.len()), (3, 6));
    // The world-gen zone is gone, otherwise identical (ids are kept; the
    // null original name now reads as "", which hashes alike).
    let mut g3 = grid();
    let gen = g3.zones[4];
    g3.zones.retain(|&z| z != gen);
    for (i, &id) in g3.zones.clone().iter().enumerate() {
        g3.zone_mut(id).id = Some(JavaUuid {
            msb: i as i64 + 1,
            lsb: -(i as i64 + 1),
        });
    }
    let without_gen = g3.save_zones(&spawned, &mut Counter(0)).unwrap();
    let again = g2.save_zones(&spawned2, &mut Counter(500)).unwrap();
    assert_eq!(again, without_gen);
}

#[test]
fn reload_replaces_zones_and_clears_chunks() {
    let mut g = grid();
    let bytes = g
        .save_zones(&SpawnedZombieZones::default(), &mut Counter(0))
        .unwrap();
    assert!(g.get_cell(0, 0).unwrap().has_chunk(1, 1));
    let empty = IsoMetaGrid::with_bounds(0, 0, 1, 1)
        .save_zones(&SpawnedZombieZones::default(), &mut Counter(0))
        .unwrap();
    g.load_zones(
        &empty,
        -1,
        &mut SpawnedZombieZones::default(),
        &mut Counter(0),
    )
    .unwrap();
    assert!(g.zones.is_empty());
    assert!(!g.get_cell(0, 0).unwrap().has_chunk(1, 1));
    g.load_zones(
        &bytes,
        -1,
        &mut SpawnedZombieZones::default(),
        &mut Counter(0),
    )
    .unwrap();
    assert_eq!(g.zones.len(), 4);
}

#[test]
fn bad_magic_and_old_versions() {
    let mut g = grid();
    let rep = g
        .load_zones(
            b"NOPE",
            -1,
            &mut SpawnedZombieZones::default(),
            &mut Counter(0),
        )
        .unwrap();
    assert!(rep.bad_magic);
    assert_eq!(g.zones.len(), 5);

    // Version 214: a double in place of the UUID, random ids for spawned
    // zone entries.
    let mut data = b"ZONE".to_vec();
    data.put_i32(214);
    data.put_i32(1);
    data.write_string_utf(Some("A"));
    data.put_i32(1);
    data.put_i16(0);
    data.put_i16(0);
    for v in [1, 2] {
        data.put_i32(v);
    }
    data.put_i8(3);
    data.put_i32(4);
    data.put_i32(5);
    data.put_i8(9); // unknown geometry: a rectangle
    data.put_i32(0);
    data.put_i8(0);
    data.put_i32(0);
    data.put_i16(0);
    data.put_f64(1.5);
    data.put_i32(1);
    data.write_string_utf(Some("A"));
    data.put_i32(2);
    let mut spawned = SpawnedZombieZones::default();
    let mut g = IsoMetaGrid::with_bounds(0, 0, 1, 1);
    g.load_zones(&data, -1, &mut spawned, &mut Counter(0))
        .unwrap();
    let z = g.zone(g.zones[0]);
    assert!(z.is_rectangle());
    assert_eq!(z.id, Some(JavaUuid { msb: 1, lsb: -1 }));
    assert_eq!(spawned.entries[0].1.len(), 2);

    let mut bad = data.clone();
    bad[20] = 5;
    assert!(g
        .load_zones(
            &bad,
            -1,
            &mut SpawnedZombieZones::default(),
            &mut Counter(0)
        )
        .is_err());
}
