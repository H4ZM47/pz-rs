//! Loader tests on hand-authored `.tiles` bytes and tables.

use super::types::test_tables;
use super::*;

type Tile<'a> = Vec<(&'a str, &'a str)>;

fn int(v: &mut Vec<u8>, i: i32) {
    v.extend(i.to_le_bytes());
}

/// A `.tiles` file: `(tileset name, tileset number, tiles)`.
fn tiles_file(tilesets: &[(&str, i32, Vec<Tile>)]) -> Vec<u8> {
    let mut v = b"tdef".to_vec();
    int(&mut v, 1);
    int(&mut v, tilesets.len() as i32);
    for (name, number, tiles) in tilesets {
        v.extend(format!("{name}\n{name}.png\n").bytes());
        int(&mut v, 8);
        int(&mut v, 8);
        int(&mut v, *number);
        int(&mut v, tiles.len() as i32);
        for t in tiles {
            int(&mut v, t.len() as i32);
            for (p, val) in t {
                v.extend(format!("{p}\n{val}\n").bytes());
            }
        }
    }
    v
}

/// Property strings, `SetCustomPropertyValues` keys a hand-authored map
/// lacks, alias generation, then `LoadTileDefinitions` for each file.
fn load(files: &[(&str, i32, Vec<u8>)]) -> IsoSpriteManager {
    let mut map = PropertyValueMap::default();
    for (name, number, bytes) in files {
        load_property_strings(&mut map, &mut &bytes[..], name, *number).unwrap();
    }
    for k in [
        "WindowN",
        "WindowW",
        "DoorWallN",
        "DoorWallW",
        "WallSE",
        "tree",
        "lightR",
        "lightG",
        "lightB",
        "ItemHeight",
        "Surface",
    ] {
        map.entry(k);
    }
    let mut m = IsoSpriteManager::new(test_tables());
    m.generate_alias_map(map, &[]).unwrap();
    for (name, number, bytes) in files {
        let _ = m.load_tile_definitions(&mut &bytes[..], name, *number);
    }
    m
}

#[test]
fn walls_doors_trees() {
    let f = tiles_file(&[
        (
            "walls_test",
            1,
            vec![
                vec![("WallN", ""), ("interior", "false")],
                vec![("doorW", ""), ("wall", "")],
                vec![("tree", "2"), ("lightR", "40")],
                vec![("windowN", ""), ("HoppableN", "")],
                vec![
                    ("WindowW", ""),
                    ("container", "crate"),
                    ("FloorHeight", "OneThird"),
                ],
            ],
        ),
        ("vegetation_trees_x", 2, vec![vec![("tree", "1")]]),
    ]);
    let m = load(&[("a.tiles", 1, f)]);
    let t = &m.tables;
    let a = &m.alias;
    let s0 = m.by_name("walls_test_0").unwrap();
    assert_eq!(s0.id, 0);
    assert_eq!(s0.tile_type, t.ty.wall);
    assert!(s0.cut_n && s0.properties.has(t.flag.collideN) && s0.properties.has(t.flag.exterior));
    assert_eq!(s0.properties.get(a, "WallN"), Some(""));

    // A wall type after a door keeps the door; the open sprite is `_3`.
    let s1 = m.by_name("walls_test_1").unwrap();
    assert_eq!(s1.tile_type, t.ty.doorW);
    assert!(s1.properties.has(t.flag.doorW));
    let s3 = m.by_name("walls_test_3").unwrap();
    assert_eq!(s3.tile_type, t.ty.doorW);
    assert!(s3.properties.has(t.flag.open) && s3.properties.has(t.flag.doorW));
    // `windowN` and `HoppableN` on the same tile.
    assert!(s3.properties.has(t.flag.transparentN) && s3.properties.has(t.flag.canPathN));
    assert_eq!(s3.properties.get(a, "WindowN"), Some("WindowN"));

    let s2 = m.by_name("walls_test_2").unwrap();
    assert_eq!(s2.tile_type, t.ty.tree);
    // Size 2 clears blocksight; the missing light channels default to "0".
    assert!(!s2.properties.has(t.flag.blocksight));
    assert_eq!(s2.properties.get(a, "tree"), Some("2"));
    assert_eq!(s2.properties.get(a, "lightR"), Some("40"));
    assert_eq!(s2.properties.get(a, "lightG"), Some("0"));
    // "MaterialType" was never registered, so it is not stored.
    assert!(!s2.properties.has_name(a, "MaterialType"));

    let s4 = m.by_name("walls_test_4").unwrap();
    assert_eq!(s4.tile_type, t.ty.windowFW);
    assert!(s4.properties.has(t.flag_from_string("container").unwrap()));
    assert_eq!(s4.properties.get(a, "container"), Some("crate"));
    assert!(s4.properties.has(t.flag.FloorHeightOneThird));

    // `vegetation_trees*` are always size 4.
    let v = m.by_name("vegetation_trees_x_0").unwrap();
    assert_eq!(v.id, 1024);
    assert!(v.properties.has(t.flag.blocksight));
    assert_eq!(m.by_name("missing").map(|s| s.id), None);
}

#[test]
fn bad_int_abandons_rest_of_file() {
    let f = tiles_file(&[
        (
            "t_a",
            1,
            vec![vec![("solid", "")], vec![("WindType", "x")], vec![]],
        ),
        ("t_b", 2, vec![vec![]]),
    ]);
    let m = load(&[("a.tiles", 1, f)]);
    assert!(m.by_name("t_a_0").unwrap().solid);
    assert!(m.by_name("t_a_1").is_some());
    assert!(m.by_name("t_a_2").is_none());
    assert!(m.by_name("t_b_0").is_none());
}

#[test]
fn patch_changes_existing_only() {
    let base = tiles_file(&[("p_a", 1, vec![vec![], vec![("CustomName", "Old")]])]);
    let patch = tiles_file(&[
        (
            "p_a",
            1,
            vec![vec![("solid", "")], vec![("CustomName", "New")]],
        ),
        ("p_missing", 2, vec![vec![("solid", "")]]),
    ]);
    let m = load(&[("a.tiles", 1, base), ("b.patch.tiles", -1, patch)]);
    assert!(m.by_name("p_a_0").unwrap().solid);
    assert_eq!(
        m.prop(m.by_name("p_a_1").unwrap(), "CustomName"),
        Some("New")
    );
    assert!(m.by_name("p_missing_0").is_none());
}

#[test]
fn duplicate_names_keep_first_id() {
    let f1 = tiles_file(&[("d_a", 1, vec![vec![("solid", "")]])]);
    let f2 = tiles_file(&[("d_a", 3, vec![vec![]])]);
    let m = load(&[("a.tiles", 1, f1), ("b.tiles", 2, f2)]);
    let s = m.by_name("d_a_0").unwrap();
    assert_eq!(s.id, 0);
    // The replacement sprite starts with fresh properties.
    assert!(!s.solid);
    assert!(!m.int_map.contains_key(&(0x100000 + 2 * 512)));
}

#[test]
fn single_sprite_movable_offsets() {
    let mv = |face: &'static str| -> Tile<'static> {
        vec![
            ("IsMoveAble", ""),
            ("CustomName", "Chair"),
            ("GroupName", "Wood"),
            ("Facing", face),
        ]
    };
    let f = tiles_file(&[("m_a", 1, vec![mv("N"), vec![], mv("E"), mv("S")])]);
    let m = load(&[("a.tiles", 1, f)]);
    let n = m.by_name("m_a_0").unwrap();
    let s = m.by_name("m_a_3").unwrap();
    assert_eq!(m.prop(n, "Soffset"), Some("3"));
    assert_eq!(m.prop(n, "Eoffset"), Some("2"));
    assert_eq!(m.prop(n, "Noffset"), None);
    assert_eq!(m.prop(s, "Noffset"), Some("-3"));
}

#[test]
fn sprite_grid_movable() {
    let g = |face: &'static str, pos: &'static str| -> Tile<'static> {
        vec![
            ("CustomName", "Table"),
            ("GroupName", "Big"),
            ("Facing", face),
            ("SpriteGridPos", pos),
        ]
    };
    let f = tiles_file(&[(
        "g_a",
        1,
        vec![g("N", "0,0"), g("N", "1,0"), g("E", "0,0"), g("E", "0,1")],
    )]);
    let m = load(&[("a.tiles", 1, f)]);
    let s0 = m.by_name("g_a_0").unwrap();
    let s3 = m.by_name("g_a_3").unwrap();
    let g0 = &m.grids[s0.sprite_grid.unwrap()];
    assert_eq!((g0.width, g0.height, g0.levels), (2, 1, 1));
    let g3 = &m.grids[s3.sprite_grid.unwrap()];
    assert_eq!((g3.width, g3.height), (1, 2));
    assert_eq!(g3.sprite_pos(m.named_map["g_a_3"]), Some((0, 1, 0)));
    assert_eq!(m.prop(s0, "Eoffset"), Some("2"));
    assert_eq!(m.prop(s3, "Noffset"), Some("-3"));
}

#[test]
fn custom_values_need_base_keys() {
    let mut map = PropertyValueMap::default();
    assert!(set_custom_property_values(&mut map).is_err());
}

#[test]
fn helpers() {
    assert_eq!(java_split("1,2,", ','), vec!["1", "2"]);
    assert_eq!(java_split("", ','), vec![""]);
    assert!(java_split(",", ',').is_empty());
    assert_eq!(try_parse_int(" 5 ", -1), 5);
    assert_eq!(try_parse_int(" ", -1), -1);
    assert!(parse_int(" 5").is_err());
    assert!(parse_int("+5").is_ok());
    assert!(is_null_or_whitespace(Some("\u{1F} ")));
    assert!(!is_null_or_whitespace(Some("\u{A0}")));
    assert!(equals_ignore_case("Container", "cONTAINER"));
}

#[test]
fn post_tile_definitions_entity_sprites() {
    let mut map = PropertyValueMap::default();
    for k in [
        "WindowN",
        "WindowW",
        "DoorWallN",
        "DoorWallW",
        "WallSE",
        "tree",
        "lightR",
        "lightG",
        "lightB",
        "ItemHeight",
        "Surface",
    ] {
        map.entry(k);
    }
    let names: Vec<String> = ["Bench", "Bad", "Copy", "Late"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    let mut m = IsoSpriteManager::new(test_tables());
    m.generate_alias_map(map, &names).unwrap();
    for (i, n) in ["bench_0", "bench_1", "bench_2", "other_0"]
        .iter()
        .enumerate()
    {
        m.add_sprite(n, 10 + i as i32);
    }
    let cfg = |name: &str, config: &str, valid: bool, tiles: &[&str]| EntitySpriteConfig {
        entity_name: name.into(),
        config_name: config.into(),
        valid,
        tile_names: tiles.iter().map(|s| s.to_string()).collect(),
        faces: vec![tiles.iter().map(|s| s.to_string()).collect()],
        faces_out_of_bounds: false,
    };
    let errors = m.post_tile_definitions(&[
        cfg("Bench", "Bench", true, &["bench_0", "bench_1", "new_0"]),
        // Invalid: skipped, and its name stays free.
        cfg("Bad", "Copy", false, &["other_0"]),
        // `bench_2` is registered before the duplicate `bench_1` throws.
        cfg("Copy", "Copy", true, &["bench_2", "bench_1"]),
        // A second config named "Bench" is a double entry.
        cfg("Late", "Bench", true, &["other_0"]),
        // `bench_2` was registered by the failed "Copy" entity.
        cfg("Late", "Late", true, &["other_0", "bench_2"]),
    ]);
    assert!(errors);
    let f = m.tables.flag.EntityScript;
    let script = |m: &IsoSpriteManager, n: &str| {
        let s = &m.sprites[m.named_map[n]];
        (
            s.properties.has(f),
            s.properties
                .get(&m.alias, "EntityScriptName")
                .map(str::to_string),
        )
    };
    assert_eq!(script(&m, "bench_0"), (true, Some("Bench".into())));
    assert_eq!(script(&m, "bench_1"), (true, Some("Bench".into())));
    // `getSprite` added the unknown tile, and it gets the properties.
    assert_eq!(script(&m, "new_0"), (true, Some("Bench".into())));
    assert_eq!(script(&m, "bench_2"), (false, None));
    assert_eq!(script(&m, "other_0"), (false, None));
    // The first tile of a face is its master.
    assert!(m.is_multi_square_master("Bench", Some("bench_0")));
    assert!(!m.is_multi_square_master("Bench", Some("bench_1")));
    // "Copy" parsed (its entry is put) before its duplicate tile threw.
    assert!(m.is_multi_square_master("Copy", Some("bench_2")));
    // A sprite twice in one face fails the parse: no entry, no flags.
    let twice = EntitySpriteConfig {
        faces: vec![vec!["dup_0".into(), "dup_0".into()]],
        ..cfg("Dup", "Dup", true, &["dup_0"])
    };
    assert!(m.post_tile_definitions(&[twice]));
    assert!(!m.sprite_configs.contains_key("Dup"));
    assert_eq!(
        m.sprite_config_scripts.get("Dup").map(String::as_str),
        Some("Dup")
    );
    assert!(!m.post_tile_definitions(&[cfg("Late", "Late", true, &["other_0"])]));
    assert_eq!(script(&m, "other_0"), (true, Some("Late".into())));
}
