//! Vehicles, pushables and animal tracks: hand-made records saved,
//! loaded and saved again.

use super::chunk_file::{load_chunk, save_chunk};
use super::io::{Reader, Writer};
use super::items::{AllItems, ItemBlob};
use super::lua_table::{LuaKey, LuaTable, LuaValue};
use super::movers::{AnimalTracks, IsoPushableObject};
use super::vehicle::*;
use super::visual::MovingBase;
use super::*;
use crate::chunk::{IsoChunk, IsoGridSquare};
use crate::objects::*;
use crate::sprite::test_tables;

fn sprites() -> IsoSpriteManager {
    let mut m = IsoSpriteManager::new(test_tables());
    for (i, n) in ["floor_0", "crate_0"].iter().enumerate() {
        let s = m.add_sprite(n, 300 + i as i32);
        m.sprites[s].name = Some(n.to_string());
    }
    m
}

fn raw_item(reg: i16, id: i32) -> ItemBlob {
    let mut d = reg.to_be_bytes().to_vec();
    d.push(0xff);
    d.extend_from_slice(&id.to_be_bytes());
    d.extend_from_slice(&[1, 2, 3]);
    ItemBlob::Raw(d)
}

fn table(k: &str, v: &str) -> LuaTable {
    let mut t = LuaTable::default();
    t.rawset(LuaKey::Str(k.into()), Some(LuaValue::Str(v.into())));
    t
}

fn part(id: &str, condition: i32) -> VehiclePart {
    VehiclePart {
        part_id: id.into(),
        created: true,
        condition,
        suspension_compression: 0.6,
        ..VehiclePart::new()
    }
}

fn vehicle() -> BaseVehicle {
    let mut engine = part("Engine", 70);
    engine.mod_data = Some(table("a", "b"));
    let mut trunk = part("TruckBed", 40);
    let mut c = ItemContainer::new(Some("TruckBed"));
    c.capacity = 55;
    c.items.push(raw_item(7, 11));
    trunk.container = Some(c);
    trunk.item = Some(raw_item(8, 12));
    let mut light = part("HeadlightLeft", 90);
    light.light = Some(VehicleLight {
        active: true,
        offset_x: 0.5,
        ..VehicleLight::default()
    });
    let mut door = part("DoorFrontLeft", 50);
    door.door = Some(VehicleDoor {
        open: false,
        locked: true,
        lock_broken: false,
    });
    let mut win = part("Windshield", 30);
    win.window = Some(VehicleWindow {
        open: true,
        open_delta: 1.0,
    });
    let mut radio = part("Radio", 100);
    radio.device_data = Some(pz_inventory::device::DeviceData::default());
    radio.components.push(ComponentBlob {
        type_id: 5,
        data: vec![9, 9],
    });
    BaseVehicle {
        base: MovingBase {
            offset_x: 64.0,
            offset_y: 192.0,
            x: 100.25,
            y: 200.75,
            z: 0.0,
            dir: 3,
            mod_data: Some(table("owner", "x")),
        },
        saved_physics_z: 0.5,
        saved_rot: [0.0, 0.0, 0.0, 1.0],
        script_name: "Base.CarNormal".into(),
        skin_index: 2,
        engine: Some(VehicleEngine {
            running: true,
            loudness: 40,
            quality: 60,
            power: 2500,
        }),
        front_end_durability: 100,
        rear_end_durability: 90,
        current_front_end_durability: 80,
        current_rear_end_durability: 70,
        key_id: 1234,
        key_spawned: 1,
        headlights_on: true,
        created: true,
        sound_horn_on: false,
        sound_back_move_on: false,
        lightbar_lights_mode: 0,
        lightbar_siren_mode: 0,
        parts: vec![engine, trunk, light, door, win, radio],
        key_is_on_door: false,
        hotwired: true,
        hotwired_broken: false,
        keys_in_ignition: true,
        rust: 0.25,
        color_hue: 0.5,
        color_saturation: 0.75,
        color_value: 1.0,
        vehicle_id: -1,
        loaded_vehicle_id: -1,
        mechanical_id: 42,
        alarmed: true,
        alarm_start_time: 3.5,
        alarm_chosen_sound: Some("AlarmA".into()),
        siren_start_time: 1.5,
        current_key: Some(raw_item(9, 13)),
        blood_intensity: vec![("Front".into(), 3), ("Rear".into(), 1)],
        towing: Some(VehicleTowing {
            vehicle_id: 7,
            attachment_self: "trailer".into(),
            attachment_other: "trailerfront".into(),
            z_offset: 0.1,
        }),
        regulator_speed: 30.0,
        previously_entered: true,
        previously_moved: false,
        animals: vec![0],
    }
}

fn save(v: &BaseVehicle, square: Option<(i32, i32)>) -> Vec<u8> {
    let mut w = Writer::new();
    v.save(&mut w, 33, square, None, &AllItems).unwrap();
    w.buf
}

fn load(b: &[u8]) -> BaseVehicle {
    let mut r = Reader::new(b);
    assert!(r.bool().unwrap());
    assert_eq!(r.i8().unwrap(), 33);
    let v = BaseVehicle::load(&mut r, WORLD_VERSION, (64.0, 192.0), &AllItems).unwrap();
    assert_eq!(r.remaining(), 0);
    v
}

#[test]
fn vehicle_round_trip() {
    let v = vehicle();
    let b = save(&v, None);
    let back = load(&b);
    assert_eq!(back.parts, v.parts);
    assert_eq!(back.engine, v.engine);
    assert_eq!(back.towing, v.towing);
    assert_eq!(back.base, v.base);
    assert_eq!(back.current_key, v.current_key);
    assert_eq!(back.animals, v.animals);
    // Without a script (as right after loading) no alarm sound is saved.
    assert_eq!(back.alarm_chosen_sound, None);
    assert_eq!(save(&back, None), b);
    // With the script's sounds the chosen sound is kept.
    let list = vec!["AlarmA".to_string()];
    let sounds = AlarmSounds {
        alarm_enable: true,
        alarm: &list,
        alarm_loop: &[],
    };
    let mut w = Writer::new();
    v.save(&mut w, 33, None, Some(&sounds), &AllItems).unwrap();
    assert_eq!(load(&w.buf).alarm_chosen_sound.as_deref(), Some("AlarmA"));
    assert_ne!(w.buf, b);
    // A sound the script lacks needs a random pick.
    let mut w = Writer::new();
    let other = vec!["AlarmB".to_string()];
    let sounds = AlarmSounds {
        alarm_enable: true,
        alarm: &other,
        alarm_loop: &[],
    };
    assert!(back
        .save(&mut w, 33, None, Some(&sounds), &AllItems)
        .is_err());
}

#[test]
fn vehicle_load_changes() {
    let mut v = vehicle();
    v.base.z = 40.0;
    v.base.dir = 11;
    v.engine.as_mut().unwrap().quality = 150;
    v.lightbar_lights_mode = 7;
    v.lightbar_siren_mode = 2;
    v.parts[2].condition = 150;
    v.parts.push(part("lightbar", 0));
    v.saved_physics_z = 9.0;
    v.vehicle_id = 77;
    let back = load(&save(&v, None));
    assert_eq!(back.base.z, 31.0);
    assert_eq!(back.base.dir, 3);
    assert_eq!(back.engine.as_ref().unwrap().quality, 100);
    assert_eq!(back.lightbar_lights_mode, 3);
    // The lightbar part at condition 0 turns the siren off.
    assert_eq!(back.lightbar_siren_mode, 0);
    assert_eq!(back.parts[2].condition, 100);
    // Physics height clamped into level 31.
    assert_eq!(back.saved_physics_z, 31.0 * 2.44949);
    assert_eq!(back.loaded_vehicle_id, 77);
    assert_eq!(back.vehicle_id, -1);
    // Without an Engine part, the engine fields save as zeros.
    let mut v = vehicle();
    v.parts.remove(0);
    let back = load(&save(&v, None));
    assert_eq!(back.engine, None);
    let mut v = vehicle();
    v.saved_physics_z = f32::NAN;
    v.base.z = 1.0;
    // A NaN height makes the origin the vehicle's z, which is what is
    // saved; loading clamps it into the level.
    assert_eq!(v.physics_origin_y(), 1.0);
    let back = load(&save(&v, None));
    assert_eq!(back.saved_physics_z, 2.44949);
    // Blood intensity is written in HashMap order; a towing link with id
    // -1 is not written.
    let mut v = vehicle();
    v.blood_intensity = vec![("Rear".into(), 1), ("Front".into(), 3)];
    v.towing.as_mut().unwrap().vehicle_id = -1;
    let back = load(&save(&v, None));
    assert_eq!(back.towing, None);
    let mut keys: Vec<&str> = back.blood_intensity.iter().map(|e| e.0.as_str()).collect();
    let mut sorted = keys.clone();
    pz_inventory::java::sort_string_hash_map(&mut sorted, |k| k);
    assert_eq!(keys, sorted);
    keys.sort();
    assert_eq!(keys, ["Front", "Rear"]);
}

#[test]
fn vehicle_square_clamps_position() {
    let v = vehicle();
    let back = load(&save(&v, Some((90, 210))));
    assert_eq!(back.base.x, 90.0 + 1.0 - 5.0e-4);
    assert_eq!(back.base.y, 210.0 + 5.0e-4);
    let back = load(&save(&v, Some((100, 200))));
    assert_eq!((back.base.x, back.base.y), (100.25, 200.75));
}

#[test]
fn rotation_round_trips() {
    assert_eq!(
        rotation_round_trip([0.0, 0.0, 0.0, 1.0]),
        [0.0, 0.0, 0.0, 1.0]
    );
    // Unnormalised: the matrix rows are normalised again.
    assert_eq!(
        rotation_round_trip([0.0, 0.0, 0.0, 2.0]),
        [0.0, 0.0, 0.0, 1.0]
    );
    // Half turns take the non-trace branches.
    assert_eq!(
        rotation_round_trip([0.0, 1.0, 0.0, 0.0]),
        [0.0, 1.0, 0.0, 0.0]
    );
    assert_eq!(
        rotation_round_trip([1.0, 0.0, 0.0, 0.0]),
        [1.0, 0.0, 0.0, 0.0]
    );
    assert_eq!(
        rotation_round_trip([0.0, 0.0, 1.0, 0.0]),
        [0.0, 0.0, 1.0, 0.0]
    );
    let h = std::f32::consts::FRAC_1_SQRT_2;
    let q = rotation_round_trip([0.0, h, 0.0, h]);
    for (a, b) in q.iter().zip([0.0, h, 0.0, h]) {
        assert!((a - b).abs() < 1e-6);
    }
}

#[test]
fn vehicle_rows() {
    let v = vehicle();
    let mut row = VehicleRow {
        id: 1,
        wx: 12,
        wy: 25,
        x: 100.25,
        y: 200.75,
        world_version: WORLD_VERSION,
        in_meta: false,
        data: save(&v, None),
    };
    assert_eq!(row.square(), Some((100, 200)));
    let back = row.load(33, (64.0, 192.0), &AllItems).unwrap().unwrap();
    assert_eq!(back.script_name, "Base.CarNormal");
    assert!(row.load(32, (64.0, 192.0), &AllItems).unwrap().is_none());
    row.x = 120.0;
    assert_eq!(row.square(), None);
    row.data[0] = 0;
    assert!(row.load(33, (64.0, 192.0), &AllItems).unwrap().is_none());
}

#[test]
fn animal_tracks_and_pushables_in_chunks() {
    let m = sprites();
    let t = SaveTables::for_tests();
    let s = SaveSettings::default();
    let ctx = SaveCtx {
        sprites: &m,
        tables: &t,
        settings: &s,
        items: &AllItems,
        hooks: &NoSaveHooks,
        meta: None,
    };
    let mut ch = IsoChunk::new(10, 20);
    for x in 0..8 {
        for y in 0..8 {
            ch.set_square(x, y, 0, Some(IsoGridSquare::new(80 + x, 160 + y, 0)));
        }
    }
    let sq = ch.get_grid_square_mut(1, 1, 0).unwrap();
    let mut track = IsoObject::new(
        sq.new_object_id(),
        ObjectKind::AnimalTrack(Box::new(AnimalTracks {
            animal_type: "deer".into(),
            track_type: "footstep".into(),
            x: 81,
            y: 161,
            dir: Some(4),
            added_time: 123_456,
            added_to_world: true,
        })),
        Some(m.named_map["floor_0"]),
    );
    track.mod_data = Some(table("k", "v"));
    let id = track.id;
    sq.objects.push(track);
    sq.special_objects.push(id);
    let base = |x: f32, offsets: (f32, f32)| MovingBase {
        offset_x: offsets.0,
        offset_y: offsets.1,
        x,
        y: 161.5,
        z: 0.0,
        dir: 9,
        mod_data: None,
    };
    let mut push = IsoObject::new(
        sq.new_object_id(),
        ObjectKind::Pushable(Box::new(IsoPushableObject {
            wheelie_bin: false,
            base: base(81.5, (12.0, -60.0)),
        })),
        Some(m.named_map["crate_0"]),
    );
    let mut c = ItemContainer::new(Some("crate"));
    c.items.push(raw_item(3, 4));
    push.container = Some(c);
    sq.objects.push(push);
    let bin = IsoObject::new(
        sq.new_object_id(),
        ObjectKind::Pushable(Box::new(IsoPushableObject {
            wheelie_bin: true,
            base: base(81.25, (0.0, 0.0)),
        })),
        None,
    );
    sq.objects.push(bin);
    let b = save_chunk(&ch, &ctx).unwrap();
    let back = load_chunk(&b, 10, 20, &ctx).unwrap().chunk;
    let sq = back.get_grid_square(1, 1, 0).unwrap();
    assert_eq!(sq.objects.len(), 3);
    assert_eq!(sq.special_objects, vec![sq.objects[0].id]);
    let ObjectKind::AnimalTrack(tr) = &sq.objects[0].kind else {
        panic!("{:?}", sq.objects[0].kind);
    };
    assert_eq!(tr.animal_type, "deer");
    assert_eq!(tr.dir, Some(4));
    let ObjectKind::Pushable(p) = &sq.objects[1].kind else {
        panic!();
    };
    // The constructor's offsets replace the saved ones; dir is masked.
    assert_eq!((p.base.offset_x, p.base.offset_y), (64.0, 192.0));
    assert_eq!(p.base.dir, 1);
    assert_eq!(sq.objects[1].sprite, Some(m.named_map["crate_0"]));
    assert_eq!(sq.objects[1].container.as_ref().unwrap().items.len(), 1);
    // A wheelie bin saved without a container gets its own back.
    let ObjectKind::Pushable(w) = &sq.objects[2].kind else {
        panic!();
    };
    assert!(w.wheelie_bin);
    assert_eq!((w.base.offset_x, w.base.offset_y), (-26.0, -248.0));
    let wc = sq.objects[2].container.as_ref().unwrap();
    assert_eq!(wc.kind.as_deref(), Some("wheeliebin"));
    let b2 = save_chunk(&back, &ctx).unwrap();
    let back2 = load_chunk(&b2, 10, 20, &ctx).unwrap().chunk;
    assert_eq!(save_chunk(&back2, &ctx).unwrap(), b2);
    // A pushable whose sprite is unknown cannot be saved.
    let mut bad = back2;
    bad.get_grid_square_mut(1, 1, 0).unwrap().objects[1].sprite = None;
    assert!(save_chunk(&bad, &ctx).is_err());
}

/// A chunk file with `vehicles` spliced into its (empty) vehicle list.
fn with_vehicles(file: &[u8], rooms: usize, vehicles: &[(i8, i8, i8, Vec<u8>)]) -> Vec<u8> {
    let at = file.len() - 8 * rooms - 4 - 2 - 2;
    let mut out = file[..at].to_vec();
    out.extend_from_slice(&(vehicles.len() as i16).to_be_bytes());
    for (x, y, z, b) in vehicles {
        out.extend_from_slice(&[*x as u8, *y as u8, *z as u8]);
        out.extend_from_slice(b);
    }
    out.extend_from_slice(&file[at + 2..]);
    let len = out.len() as i32;
    out[5..9].copy_from_slice(&len.to_be_bytes());
    let crc = super::io::crc32(&out[17..]) as i64;
    out[9..17].copy_from_slice(&crc.to_be_bytes());
    out
}

#[test]
fn chunk_vehicle_lists() {
    let m = sprites();
    let t = SaveTables::for_tests();
    let s = SaveSettings::default();
    let ctx = SaveCtx {
        sprites: &m,
        tables: &t,
        settings: &s,
        items: &AllItems,
        hooks: &NoSaveHooks,
        meta: None,
    };
    let mut ch = IsoChunk::new(10, 20);
    for x in 0..8 {
        for y in 0..8 {
            ch.set_square(x, y, 0, Some(IsoGridSquare::new(80 + x, 160 + y, 0)));
        }
    }
    ch.saved.spawned_rooms = vec![5];
    let plain = save_chunk(&ch, &ctx).unwrap();
    let class = t.class_id("Vehicle");
    let mut w = Writer::new();
    vehicle()
        .save(&mut w, class, None, None, &AllItems)
        .unwrap();
    let file = with_vehicles(
        &plain,
        1,
        &[
            (2, 3, 0, w.buf.clone()),
            (0, 0, 0, vec![0]),
            (7, 7, 0, w.buf),
        ],
    );
    let back = load_chunk(&file, 10, 20, &ctx).unwrap();
    assert!(back.crc_ok);
    let vs = &back.chunk.saved.vehicles;
    // The entry with a clear serialise flag is skipped.
    assert_eq!(vs.len(), 2);
    assert_eq!((vs[0].x, vs[0].y), (2, 3));
    assert_eq!(vs[0].square(&back.chunk), Some((82, 163)));
    assert_eq!(vs[1].vehicle.script_name, "Base.CarNormal");
    // Saving writes the empty list, as the original.
    assert_eq!(save_chunk(&back.chunk, &ctx).unwrap(), plain);
    // Another class in the list cannot be read.
    let file = with_vehicles(
        &plain,
        1,
        &[(0, 0, 0, vec![1, t.class_id("Pushable") as u8])],
    );
    assert!(load_chunk(&file, 10, 20, &ctx).is_err());
}
