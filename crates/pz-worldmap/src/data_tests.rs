//! Parity of the data readers with the original `WorldMapBinary` and
//! `WorldMapXML` (oracles `data_bin.txt`, `data_xml.txt`,
//! `data_forest.txt` in `PZ_WORLDMAP_ORACLE_DIR`, read from the map
//! directory `PZ_WORLDMAP_MAP_DIR`): every cell, feature, ring (point
//! count, winding, an FNV hash of the points), geometry bounds and
//! properties, plus point-in-polygon hits on a grid.

use crate::data::*;

fn digest(cells: &[WorldMapCell]) -> Vec<String> {
    let mut out = vec!["ok true".to_string()];
    for c in cells {
        out.push(format!("cell {} {} {}", c.x, c.y, c.features.len()));
        for f in &c.features {
            let mut s = String::from("f");
            match &f.geometry {
                None => s.push_str(" -"),
                Some(g) => {
                    s.push_str(&format!(
                        " {:?} {},{},{},{}",
                        g.kind, g.min_x, g.min_y, g.max_x, g.max_y
                    ));
                    for p in &g.points {
                        let mut h: u64 = 0xcbf29ce484222325;
                        for i in 0..p.num_points() {
                            h = (h ^ (p.get_x(&c.points, i) as u64 & 0xffff))
                                .wrapping_mul(0x100000001b3);
                            h = (h ^ (p.get_y(&c.points, i) as u64 & 0xffff))
                                .wrapping_mul(0x100000001b3);
                        }
                        s.push_str(&format!(
                            " r{}:{}:{:x}",
                            p.num_points(),
                            p.is_clockwise(&c.points),
                            h
                        ));
                    }
                }
            }
            let mut props: Vec<_> = f.properties.map.iter().collect();
            props.sort();
            for (k, v) in props {
                s.push_str(&format!(" {k}={v}"));
            }
            out.push(s);
        }
        let mut hs = String::from("hit");
        for y in (8..256).step_by(32) {
            for x in (8..256).step_by(32) {
                let mut got = Vec::new();
                c.hit_test(
                    (c.x * 256 + x) as f32 + 0.5,
                    (c.y * 256 + y) as f32 + 0.25,
                    &mut got,
                );
                hs.push_str(&format!(" {}", got.len()));
                for i in got {
                    hs.push_str(&format!(".{i}"));
                }
            }
        }
        out.push(hs);
    }
    out
}

fn check(oracle: &str, file: &str) {
    let (Some(dir), Some(map)) = (
        std::env::var_os("PZ_WORLDMAP_ORACLE_DIR"),
        std::env::var_os("PZ_WORLDMAP_MAP_DIR"),
    ) else {
        eprintln!("PZ_WORLDMAP_ORACLE_DIR / PZ_WORLDMAP_MAP_DIR not set; skipping");
        return;
    };
    let want = std::fs::read_to_string(std::path::Path::new(&dir).join(oracle)).unwrap();
    let path = std::path::Path::new(&map).join(file);
    let cells = if file.ends_with(".bin") {
        crate::binary::read(&path)
    } else {
        crate::xml::read(&path)
    }
    .unwrap();
    let got = digest(&cells);
    let want: Vec<&str> = want.lines().collect();
    for (i, (g, w)) in got.iter().zip(&want).enumerate() {
        assert_eq!(g, w, "line {i}");
    }
    assert_eq!(got.len(), want.len());
}

#[test]
fn matches_binary_oracle() {
    check("data_bin.txt", "worldmap.xml.bin");
}

#[test]
fn matches_forest_binary_oracle() {
    check("data_forest.txt", "worldmap-forest.xml.bin");
}

#[test]
fn matches_xml_oracle() {
    check("data_xml.txt", "worldmap.xml");
}

/// A tiny file built by hand: header checks and one cell.
#[test]
fn binary_layout() {
    let mut b = b"IGMB".to_vec();
    for v in [2i32, 256, 2, 1] {
        b.extend(v.to_le_bytes());
    }
    let strings = ["Polygon", "water", "river"];
    b.extend((strings.len() as i32).to_le_bytes());
    for s in strings {
        b.extend((s.len() as i16).to_le_bytes());
        b.extend(s.as_bytes());
    }
    // cell (3, 4): one square polygon with water=river.
    b.extend(3i32.to_le_bytes());
    b.extend(4i32.to_le_bytes());
    b.extend(1i32.to_le_bytes());
    b.extend(0i16.to_le_bytes());
    b.push(1);
    b.extend(4i16.to_le_bytes());
    for (x, y) in [(0i16, 0i16), (10, 0), (10, 10), (0, 10)] {
        b.extend(x.to_le_bytes());
        b.extend(y.to_le_bytes());
    }
    b.push(1);
    b.extend(1i16.to_le_bytes());
    b.extend(2i16.to_le_bytes());
    // second cell slot empty.
    b.extend((-1i32).to_le_bytes());
    let cells = crate::binary::read_bytes(&b).unwrap();
    assert_eq!(cells.len(), 1);
    let c = &cells[0];
    assert_eq!((c.x, c.y), (3, 4));
    assert_eq!(c.features[0].properties.get("water"), Some("river"));
    let mut hit = Vec::new();
    c.hit_test(3.0 * 256.0 + 5.0, 4.0 * 256.0 + 5.0, &mut hit);
    assert_eq!(hit, vec![0]);
    assert!(crate::binary::read_bytes(b"IGMX").is_err());
}
