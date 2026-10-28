use std::collections::HashMap;

use super::*;
use crate::migration::MigrationGroupDefinitions;
use pz_world::zones::{AnimalZoneData, Zone, ZoneGeometryType, ZoneKind};

#[derive(Clone, Debug, PartialEq)]
struct Beast {
    id: i32,
    mother: i32,
    zone: Option<ZoneId>,
    forward: Option<bool>,
    mother_set: Option<i32>,
}

impl HerdAnimal for Beast {
    fn animal_id(&self) -> i32 {
        self.id
    }
    fn attach_back_to_mother(&self) -> i32 {
        self.mother
    }
    fn set_animal_zone(&mut self, zone: Option<ZoneId>) {
        self.zone = zone;
    }
    fn set_move_forward_on_zone(&mut self, forward: bool) {
        self.forward = Some(forward);
    }
}

/// `java.util.Random`'s generator.
struct JRand(i64);

impl JRand {
    fn bits(&mut self, n: u32) -> i32 {
        self.0 = (self.0.wrapping_mul(0x5DEECE66D).wrapping_add(0xB)) & ((1 << 48) - 1);
        (self.0 >> (48 - n)) as i32
    }
}

impl GameRand for JRand {
    fn next(&mut self, max: i32) -> i32 {
        if max <= 0 {
            return 0;
        }
        (self.bits(31) as i64 % max as i64) as i32
    }
    fn next_float(&mut self) -> f32 {
        self.bits(24) as f32 / (1 << 24) as f32
    }
}

struct Host {
    rand: JRand,
    files: HashMap<(i32, i32), Vec<u8>>,
    to_world: Vec<VirtualAnimal<Beast>>,
    next_id: i32,
    age: f64,
    hour: i32,
}

impl Host {
    fn new() -> Self {
        Host {
            rand: JRand(42 ^ 0x5DEECE66D),
            files: HashMap::new(),
            to_world: Vec::new(),
            next_id: 1,
            age: 10.0,
            hour: 12,
        }
    }
}

impl PopulationHost<Beast> for Host {
    fn rand(&mut self) -> &mut dyn GameRand {
        &mut self.rand
    }
    fn hour(&self) -> i32 {
        self.hour
    }
    fn world_age_hours(&self) -> f64 {
        self.age
    }
    fn calendar_millis(&self) -> i64 {
        1_000_000_000
    }
    fn multiplier(&self) -> f32 {
        20.0
    }
    fn thirty_fps_multiplier(&self) -> f32 {
        1.0
    }
    fn animal_track_chance(&self) -> i32 {
        1
    }
    fn new_animal(&mut self, _t: &str, _b: &str) -> JResult<Beast> {
        self.next_id += 1;
        Ok(Beast { id: self.next_id, mother: 0, zone: None, forward: None, mother_set: None })
    }
    fn randomize_age(&mut self, _a: &mut Beast) -> JResult<()> {
        Ok(())
    }
    fn can_have_baby(&self, _a: &Beast) -> bool {
        false
    }
    fn add_baby(&mut self, _m: &mut Beast) -> JResult<Option<Beast>> {
        Ok(None)
    }
    fn set_virtual_id(&mut self, _a: &mut Beast, _id: f64, _g: &str) {}
    fn set_mother(&mut self, baby: &mut Beast, mother: &mut Beast) {
        baby.mother_set = Some(mother.id);
    }
    fn save_animal(&mut self, a: &Beast, w: &mut Writer) -> SaveResult<()> {
        w.i32(a.id);
        w.i32(a.mother);
        Ok(())
    }
    fn load_animal(&mut self, r: &mut Reader<'_>, _v: i32) -> SaveResult<Beast> {
        Ok(Beast { id: r.i32()?, mother: r.i32()?, zone: None, forward: None, mother_set: None })
    }
    fn read_apop(&mut self, x: i32, y: i32) -> std::io::Result<Option<Vec<u8>>> {
        Ok(self.files.get(&(x, y)).cloned())
    }
    fn write_apop(&mut self, x: i32, y: i32, bytes: &[u8]) -> std::io::Result<()> {
        self.files.insert((x, y), bytes.to_vec());
        Ok(())
    }
    fn no_save(&self) -> bool {
        false
    }
    fn from_worker(&mut self, _meta: &IsoMetaGrid, animals: Vec<VirtualAnimal<Beast>>) {
        self.to_world.extend(animals);
    }
}

fn path(points: &[i32], action: &str, animal_type: Option<&str>) -> Zone {
    let mut z = Zone::default();
    z.geometry_type = ZoneGeometryType::Polyline;
    z.points = points.to_vec();
    let (xs, ys): (Vec<i32>, Vec<i32>) = points.chunks(2).map(|p| (p[0], p[1])).unzip();
    z.x = *xs.iter().min().unwrap();
    z.y = *ys.iter().min().unwrap();
    z.w = xs.iter().max().unwrap() - z.x + 1;
    z.h = ys.iter().max().unwrap() - z.y + 1;
    z.kind = ZoneKind::Animal(AnimalZoneData {
        action: Some(action.into()),
        animal_type: animal_type.map(Into::into),
        ..Default::default()
    });
    z
}

fn defs() -> AnimalDefinitionTables {
    let mut t = AnimalDefinitionTables::default();
    let deer = MigrationGroupDefinitions {
        type_: Some("deer".into()),
        male: Some("buck".into()),
        female: Some("doe".into()),
        min_animal: 2,
        max_animal: 3,
        max_male: 1,
        speed: 1.0,
        min_time_before_eat: 60,
        max_time_before_eat: 120,
        min_time_before_sleep: 60,
        max_time_before_sleep: 120,
        time_to_eat: 30,
        time_to_sleep: 30,
        possible_breed: Some(vec!["whitetailed".into()]),
        ..Default::default()
    };
    t.migration.insert("deer".into(), deer);
    t
}

#[test]
fn spawned_herd_round_trips_through_apop() {
    let mut meta = IsoMetaGrid::with_bounds(0, 0, 0, 0);
    let a = meta.register_animal_zone_def(path(&[20, 20, 200, 20, 200, 200], "Follow", Some("deer")));
    let b = meta.register_animal_zone_def(path(&[200, 200, 20, 20], "Follow", Some("deer")));
    let defs = defs();
    let mut host = Host::new();
    let mut pop = AnimalPopulation::<Beast>::default();
    {
        let mut cx = PopCx { meta: &mut meta, defs: &defs, host: &mut host };
        pop.init(false, false, &mut cx);
        // Shared end points become junctions both ways.
        assert_eq!(zone::junctions(cx.meta, a).len(), 2);
        assert_eq!(zone::junctions(cx.meta, b).len(), 2);
        // The first chunk loaded loads the cell, which spawns a herd per
        // Follow zone.
        pop.add_chunk_to_world(30, 30, true, &mut cx).unwrap();
    }
    assert_eq!(pop.animals.len(), 2);
    let herd = pop.animals.values().next().unwrap();
    assert_eq!(herd.migration_group.as_deref(), Some("deer"));
    assert!((3..=4).contains(&herd.animals.len()));
    assert!(matches!(&meta.zone(a).kind, ZoneKind::Animal(d) if d.spawned_animals));

    let saved: Vec<(f64, usize)> = {
        let mut v: Vec<_> = pop.animals.values().map(|h| (h.id, h.animals.len())).collect();
        v.sort_by(|x, y| x.0.total_cmp(&y.0));
        v
    };
    let mut cx = PopCx { meta: &mut meta, defs: &defs, host: &mut host };
    pop.save(Vec::new(), &mut cx).unwrap();
    drop(cx);
    let bytes = host.files[&(0, 0)].clone();
    assert_eq!(&bytes[..4], &APOP_WORLD_VERSION.to_be_bytes());
    pop.stop();
    let mut cx = PopCx { meta: &mut meta, defs: &defs, host: &mut host };

    let mut again = AnimalPopulation::<Beast>::default();
    again.init(false, false, &mut cx);
    again.add_chunk_to_world(31, 31, false, &mut cx).unwrap();
    let mut loaded: Vec<_> = again.animals.values().map(|h| (h.id, h.animals.len())).collect();
    loaded.sort_by(|x, y| x.0.total_cmp(&y.0));
    assert_eq!(loaded, saved);
    // Nothing changed: re-saving gives the same bytes.
    let c = again.cell_index_from_cell_pos(0, 0).unwrap();
    assert_eq!(again.cell_bytes(c, &mut cx).unwrap(), bytes);
}

#[test]
fn herd_walks_its_zone_and_enters_a_loaded_chunk() {
    let mut meta = IsoMetaGrid::with_bounds(0, 0, 0, 0);
    let mut zone = path(&[10, 100, 250, 100], "Follow", None);
    if let ZoneKind::Animal(d) = &mut zone.kind {
        // Only the hand-made herd below.
        d.spawn_animal = false;
    }
    let z = meta.register_animal_zone_def(zone);
    let defs = defs();
    let mut host = Host::new();
    let mut pop = AnimalPopulation::<Beast>::default();
    let mut cx = PopCx { meta: &mut meta, defs: &defs, host: &mut host };
    pop.init(false, false, &mut cx);
    let mut va = pop.new_virtual_animal(cx.host.rand());
    va.x = 20.5;
    va.y = 100.5;
    va.move_forward_on_zone = true;
    va.migration_group = Some("deer".into());
    va.init_value_from_def(cx.defs).unwrap();
    va.animals.push(Beast { id: 7, mother: 0, zone: None, forward: None, mother_set: None });
    pop.add_animal(va, &mut cx);
    assert_eq!(pop.chunks_with_tracks.len(), 1);

    // Multiplier 20: 2 squares per update along +x.
    pop.update_virtual_animals(&mut cx).unwrap();
    let herd = pop.animals.values().next().unwrap();
    assert_eq!(herd.state, Some(VirtualAnimalState::Follow));
    assert!((herd.x - 22.5).abs() < 1e-4, "{}", herd.x);
    assert_eq!(herd.animals[0].zone, Some(z));
    assert_eq!(herd.animals[0].forward, Some(true));
    assert!(herd.next_eat_time > 10.0 && herd.next_rest_time > 10.0);

    // Loading the chunk ahead hands the herd to the world when it walks in.
    pop.add_chunk_to_world(3, 12, false, &mut cx).unwrap();
    for _ in 0..3 {
        pop.update_virtual_animals(&mut cx).unwrap();
    }
    drop(cx);
    assert!(pop.animals.is_empty());
    assert_eq!(host.to_world.len(), 1);
    assert!(host.to_world[0].removed);
    assert!(pop.chunks_with_tracks.is_empty());
}

#[test]
fn reattaches_babies_to_mothers_on_load() {
    let mut w = Writer::new();
    w.i32(APOP_WORLD_VERSION);
    for i in 0..1024 {
        if i != 5 {
            w.i16(0);
            w.i16(0);
            continue;
        }
        w.i16(1);
        w.f64(123.0);
        w.i64(0);
        w.i64(0);
        for f in [44.0f32, 3.0, 0.0, 1.0, 0.0] {
            w.f32(f);
        }
        w.string("deer");
        w.f64(-1.0);
        w.f64(-1.0);
        w.i16(2);
        for (id, mother) in [(9, 4), (4, 0)] {
            w.i32(id);
            w.i32(mother);
        }
        w.i16(0);
    }
    let mut meta = IsoMetaGrid::with_bounds(0, 0, 0, 0);
    let defs = defs();
    let mut host = Host::new();
    host.files.insert((0, 0), w.buf);
    let mut pop = AnimalPopulation::<Beast>::default();
    let mut cx = PopCx { meta: &mut meta, defs: &defs, host: &mut host };
    pop.init(false, false, &mut cx);
    let key = pop.get_animal_chunk(44.0, 3.0, &mut cx).unwrap();
    let uid = pop.chunk(key).animals[0];
    let herd = &pop.animals[&uid];
    assert_eq!(herd.id, 123.0);
    assert_eq!(herd.animals[0].mother_set, Some(4));
    assert_eq!(herd.animals[1].mother_set, None);
    assert_eq!(herd.time_to_eat, 30);
}
