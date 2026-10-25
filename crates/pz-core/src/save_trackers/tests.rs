use super::*;

#[test]
fn trove_iterates_slots_from_the_end() {
    let mut m = TroveObjectIntMap::default();
    // "a" (97) is slot 97 % 23 = 5, "b" slot 6: the last slot comes first.
    m.put("a", 1);
    m.put("b", 2);
    assert_eq!(m.entries().collect::<Vec<_>>(), [("b", 2), ("a", 1)]);
    m.put("a", 3);
    assert_eq!(m.len(), 2);
    assert_eq!(m.get("a"), 3);
    assert_eq!(m.get("zz"), 0);
}

#[test]
fn trove_grows_past_half_full() {
    let mut m = TroveObjectIntMap::default();
    for i in 0..12 {
        m.put(&format!("k{i}"), i);
    }
    // 11 entries fit 23 slots; the 12th rehashes to 47.
    assert_eq!(m.slots.len(), 47);
    for i in 0..12 {
        assert_eq!(m.get(&format!("k{i}")), i);
    }
    let mut keys: Vec<i32> = m.entries().map(|(_, v)| v).collect();
    keys.sort();
    assert_eq!(keys, (0..12).collect::<Vec<_>>());
}

#[test]
fn colliding_keys_probe_downwards() {
    let mut m = TroveObjectIntMap::default();
    // "Aa" and "BB" share hash 2112: slot 2112 % 23 = 19, then 19 - (1 +
    // 2112 % 21) = 6.
    m.put("Aa", 1);
    m.put("BB", 2);
    assert_eq!(m.slot("BB"), (6, false));
    assert_eq!(m.entries().collect::<Vec<_>>(), [("Aa", 1), ("BB", 2)]);
}

#[test]
fn hash_map_order_by_bucket() {
    let mut o = JavaHashOrder::default();
    for k in ["b", "Aa", "a", "BB"] {
        o.insert(k);
    }
    assert_eq!(o.iter_order(), ["Aa", "BB", "a", "b"]);
}

#[test]
fn instance_tracker_round_trips() {
    let mut t = InstanceTracker::default();
    t.inc(ITEMS, "Base.Axe");
    t.adj(ITEMS, "Base.Apple", 5);
    t.set(CONTAINERS, "crate", 7);
    t.set(CONTAINERS, "  ", 7);
    t.dec("", "x");
    let bytes = t.save_bytes();
    let mut u = InstanceTracker::default();
    u.load_bytes(&bytes).unwrap();
    assert_eq!(u.save_bytes(), bytes);
    assert_eq!(u.get(ITEMS, "Base.Apple"), 5);
    assert_eq!(u.get("", "x"), -1);
    assert_eq!(
        u.item_tracker_log(),
        "[Item Spawns] ; 2 entries\nBase.Apple = 5\nBase.Axe = 1\n\n[Container Rolls] ; 1 entries\ncrate = 7\n\n"
    );
}

#[test]
fn meta_tracker_round_trips() {
    let m = MetaTracker {
        helicopter: HelicopterSave {
            active: true,
            state: 2,
            x: 10.5,
            y: -3.0,
        },
        alarms: vec![AlarmSave {
            x: 1,
            y: 2,
            end_game_time: 3.5,
        }],
    };
    let b = m.save_bytes();
    assert_eq!(b.len(), 19 + 12);
    let mut n = MetaTracker::default();
    n.load_bytes(&b, true).unwrap();
    assert_eq!(n, m);
    // No player 0 to aim at: the helicopter is inactive.
    let mut n = MetaTracker::default();
    n.load_bytes(&b, false).unwrap();
    assert!(!n.helicopter.active);
    let mut bad = b.clone();
    bad[5..9].copy_from_slice(&4i32.to_be_bytes());
    assert!(MetaTracker::default().load_bytes(&bad, true).is_err());
}

#[test]
fn statistics_round_trip_in_modified_utf8() {
    let mut s = Statistics::default();
    s.increment(0, 1, "kills\u{0}", 2.0);
    s.set(2, 8, "distance", 10.25);
    let b = s.save_bytes().unwrap();
    // The NUL is two bytes in modified UTF-8.
    assert!(b.windows(2).any(|w| w == [0xc0, 0x80]));
    let mut t = Statistics::default();
    t.load_bytes(&b).unwrap();
    assert_eq!(t.save_bytes().unwrap(), b);
    assert_eq!(t.get("distance"), Some(10.25));
    let mut bad = Vec::new();
    bad.put_i32(1);
    write_modified_utf(&mut bad, "x").unwrap();
    write_modified_utf(&mut bad, "Alien").unwrap();
    assert!(Statistics::default().load_bytes(&bad).is_err());
}
