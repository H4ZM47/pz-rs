use super::*;
use crate::zones::{ZoneGeometryType, ZoneKind};
use crate::Zone;

struct Counter(i64);
impl UuidSource for Counter {
    fn random_uuid(&mut self) -> JavaUuid {
        self.0 += 1;
        JavaUuid {
            msb: self.0 << 40,
            lsb: self.0 * 7,
        }
    }
}

/// A fresh directory per call.
struct Dir(std::path::PathBuf);

impl Dir {
    fn new() -> Self {
        static N: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let n = N.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let p = std::env::temp_dir().join(format!(
            "pz-world-animal-zones-{}-{n}-{:?}",
            std::process::id(),
            std::time::SystemTime::now()
        ));
        std::fs::create_dir_all(&p).unwrap();
        Dir(p)
    }
}

impl Drop for Dir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn path_zone(name: &str, x: i32, pts: &[i32]) -> Zone {
    let mut z = Zone::new(name, "Animal", x, 10, 0, 40, 40);
    z.geometry_type = ZoneGeometryType::Polyline;
    z.points = pts.to_vec();
    z.polyline_width = 2;
    z.kind = ZoneKind::Animal(AnimalZoneData {
        action: Some("Follow".into()),
        animal_type: Some("deer".into()),
        ..Default::default()
    });
    z
}

#[test]
fn uuid_hash_is_javas() {
    // new UUID(1L << 40, 7L).hashCode(): hilo = 0x0000010000000007.
    assert_eq!(
        uuid_hash(JavaUuid {
            msb: 1 << 40,
            lsb: 7
        }),
        0x100 ^ 7
    );
    assert_eq!(uuid_hash(JavaUuid { msb: -1, lsb: 0 }), 0);
}

#[test]
fn hash_map_order_keeps_the_grown_table() {
    // Bucket of 17 is 1 in a 16 table, 17 in a 32 one.
    assert_eq!(java_hash_order_of(&[17, 2], 2), [0, 1]);
    assert_eq!(java_hash_order_of(&[17, 2], 13), [1, 0]);
}

#[test]
fn map_animals_round_trips_with_junctions() {
    let mut g = IsoMetaGrid::with_bounds(0, 0, 1, 1);
    let a = g.register_animal_zone_def(path_zone("a", 10, &[10, 10, 30, 10, 30, 40]));
    let b = g.register_animal_zone_def(path_zone("b", 300, &[30, 40, 300, 40]));
    if let ZoneKind::Animal(d) = &mut g.zone_defs[a.0 as usize].kind {
        d.spawned_animals = true;
        d.junctions = Some(vec![AnimalZoneJunction {
            point_index_self: 2,
            zone_other: b,
            point_index_other: 0,
        }]);
    }
    let mut ids = Counter(0);
    let bytes = g.save_animal_zones(&mut ids).unwrap();
    assert_eq!(&bytes[..8], b"ZONE\0\0\0\xf9");

    let mut h = IsoMetaGrid::with_bounds(0, 0, 1, 1);
    let rep = h.load_animal_zones(&bytes, -1).unwrap();
    assert_eq!((rep.zones, rep.junctions, rep.world_version), (2, 1, 249));
    // Loading does not queue the cells.
    assert!(h.cells_to_save.borrow().is_empty());
    assert_eq!(h.get_cell_data(1, 0).unwrap().animal_zones.len(), 1);
    assert_eq!(h.save_animal_zones(&mut ids).unwrap(), bytes);
    let za = h
        .animal_zones
        .iter()
        .copied()
        .find(|&id| h.zone(id).name == "a")
        .unwrap();
    match &h.zone(za).kind {
        ZoneKind::Animal(d) => {
            assert!(d.spawn_animal && d.spawned_animals);
            assert_eq!(d.junctions.as_ref().unwrap()[0].point_index_self, 2);
        }
        k => panic!("{k:?}"),
    }
}

#[test]
fn a_junction_to_an_unknown_zone_ends_the_load() {
    let mut g = IsoMetaGrid::with_bounds(0, 0, 0, 0);
    g.register_animal_zone_def(path_zone("a", 10, &[10, 10, 30, 10]));
    let mut bytes = g.save_animal_zones(&mut Counter(0)).unwrap();
    let n = bytes.len();
    bytes[n - 4..].copy_from_slice(&1i32.to_be_bytes());
    bytes.extend_from_slice(&[0; 40]);
    let mut h = IsoMetaGrid::with_bounds(0, 0, 0, 0);
    assert!(h.load_animal_zones(&bytes, -1).is_err());
    assert_eq!(h.animal_zones.len(), 1);
}

#[test]
fn registered_zones_queue_their_cells_and_save_one_byte_each() {
    let d = Dir::new();
    let mut g = IsoMetaGrid::with_bounds(3, 4, 4, 4);
    let mut z = path_zone("a", 3 * 256 + 250, &[0, 0, 1, 1]);
    z.y = 4 * 256 + 10;
    g.register_animal_zone_def(z);
    assert_eq!(*g.cells_to_save.borrow(), [(3, 4), (4, 4)]);
    assert!(g.check_animal_zones_generated(4, 4));
    assert!(!g.check_animal_zones_generated(4, 4));
    // Without the folder the first write fails and nothing leaves the queue.
    assert!(g.save_cells_files(&d.0).is_err());
    assert_eq!(g.cells_to_save.borrow().len(), 2);
    std::fs::create_dir(d.0.join(METAGRID_DIR)).unwrap();
    g.save_cells_files(&d.0).unwrap();
    assert!(g.cells_to_save.borrow().is_empty());
    let read = |x, y| std::fs::read(d.0.join(METAGRID_DIR).join(metacell_file_name(x, y))).unwrap();
    assert_eq!(read(3, 4), [0]);
    assert_eq!(read(4, 4), [1]);
    std::fs::write(d.0.join(METAGRID_DIR).join("metacell_x_4.bin"), [1]).unwrap();

    let mut h = IsoMetaGrid::with_bounds(3, 4, 4, 4);
    let mut cells = h.load_cells_files(&d.0).unwrap();
    cells.sort();
    assert_eq!(cells, [(3, 4), (4, 4)]);
    assert!(h.get_cell_data(4, 4).unwrap().animal_zones_generated.get());
    assert!(!h.get_cell_data(3, 4).unwrap().animal_zones_generated.get());
    // The hot save takes the same bytes.
    h.add_cell_to_save(4, 4);
    assert_eq!(
        h.save_cells(),
        [("metagrid/metacell_4_4.bin".to_string(), vec![1])]
    );
}

#[test]
fn metacell_names() {
    assert_eq!(
        parse_metacell_name("metacell_-3_12.bin").unwrap(),
        Some((-3, 12))
    );
    assert_eq!(parse_metacell_name("metacell_3_12.bin.tmp").unwrap(), None);
    assert_eq!(parse_metacell_name("metacell_3__12.bin").unwrap(), None);
    assert!(parse_metacell_name("metacell_99999999999_1.bin").is_err());
    let mut g = IsoMetaGrid::with_bounds(0, 0, 0, 0);
    assert!(g.load_meta_cell(5, 5, &[1]).is_err());
    assert!(g.load_meta_cell(0, 0, &[]).is_err());
}
