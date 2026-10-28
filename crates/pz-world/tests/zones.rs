//! Meta-grid zone registration and queries with hand-authored zones.

use std::collections::HashMap;

use pz_world::zones::ANY_Z;
use pz_world::{ClipperOps, IsoMetaGrid, PropValue, ZoneError, ZoneKind, ZoneProps, ZoneTables};

fn grid() -> IsoMetaGrid {
    let mut g = IsoMetaGrid::with_bounds(0, 0, 2, 2);
    g.zone_tables = ZoneTables {
        preferred_zone_types: vec!["Meadow".into()],
        iso_direction_names: vec!["North".into(), "South".into()],
    };
    g
}

fn props(kv: &[(&str, PropValue)]) -> HashMap<String, PropValue> {
    kv.iter().map(|(k, v)| (k.to_string(), v.clone())).collect()
}

#[test]
fn rectangles_across_cells_and_ordering() {
    let mut g = grid();
    let a = g.register_zone("a", "Lot", 250, 250, 0, 20, 20);
    let b = g.register_zone("b", "Park", 255, 255, 0, 3, 3);
    let c = g.register_zone("c", "Meadow", 240, 240, 0, 40, 40);
    // Newest zone wins unless an older one has a preferred type.
    assert_eq!(g.get_zone_at(256, 256, 0), Some(c));
    let d = g.register_zone("d", "Lot", 256, 256, 0, 1, 1);
    assert_eq!(g.get_zone_at(256, 256, 0), Some(c));
    assert_eq!(g.get_zones_at(256, 256, 0), vec![a, b, c, d]);
    assert_eq!(g.get_zone_at(251, 251, 0), Some(c));
    assert_eq!(g.get_zones_at(251, 251, 1), vec![]);
    // The zone is in cells (0,0), (1,0), (0,1), (1,1).
    for (cx, cy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
        assert!(g.get_cell_data(cx, cy).is_some());
    }
    assert_eq!(
        g.get_zones_intersecting(0, 0, ANY_Z, 300, 300),
        vec![c, a, b, d]
    );
    assert_eq!(
        g.get_zone_with_bounds_and_type(255, 255, 0, 3, 3, "PARK"),
        Some(b)
    );
    assert!(g.is_zone_above(Some(d), Some(a), 256, 256, 0));
    assert!(!g.is_zone_above(Some(a), Some(d), 256, 256, 0));
    assert!(g.is_zone_above(Some(a), None, 256, 256, 0));
    assert_eq!(g.zones, vec![a, b, c, d]);
    assert_eq!(g.process_zones(), 4);

    g.remove_zone(c);
    assert_eq!(g.get_zone_at(256, 256, 0), Some(d));
    g.remove_zone(a);
    g.remove_zone(b);
    g.remove_zone(d);
    // Chunks left without rooms and zones are dropped.
    assert!(g.get_chunk_data_from_tile(256, 256).is_none());
    assert!(g.zones.is_empty());
}

#[test]
fn suspicious_zones_are_not_added() {
    let mut g = grid();
    let far = g.register_zone("x", "T", -200, 0, 0, 10, 10);
    assert!(g.zones.is_empty());
    assert_eq!(g.zone(far).x, -200);
    g.register_zone("x", "T", 0, 0, 0, 1203, 10);
    assert!(g.zones.is_empty());
    assert_eq!(g.register_zone_no_overlap("x", "T", 0, 0, 8, 10, 10), None);
    assert!(g
        .register_zone_no_overlap("x", "T", 0, 0, 7, 10, 10)
        .is_some());
}

#[test]
fn polygon_geometry_keeps_max_bounds() {
    let mut g = grid();
    let id = g
        .register_geometry_zone(
            "p",
            "Lot",
            0,
            "polygon",
            &[10.0, 10.0, 30.0, 10.0, 10.0, 30.0],
            None,
        )
        .unwrap()
        .unwrap();
    let z = g.zone(id);
    assert_eq!((z.x, z.y, z.w, z.h), (i32::MAX, i32::MAX, 2, 2));
    assert_eq!(g.zones, vec![id]);
    assert_eq!(g.get_zone_at(12, 12, 0), None);
    assert_eq!(
        g.register_geometry_zone("p", "Lot", 0, "circle", &[], None),
        Err(ZoneError::UnknownGeometry("circle".into()))
    );
    assert_eq!(
        g.register_geometry_zone("p", "Lot", 0, "point", &[1.0], None),
        Err(ZoneError::OddPointCount)
    );
}

/// Hand-written stand-in for the native offset: a box around the points.
struct BoxClipper;
impl ClipperOps for BoxClipper {
    fn offset_polyline(&mut self, p: &[f32], d: f64) -> Option<Vec<f32>> {
        let d = d as f32;
        let (mut x0, mut y0, mut x1, mut y1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
        for q in p.chunks(2) {
            x0 = x0.min(q[0]);
            x1 = x1.max(q[0]);
            y0 = y0.min(q[1]);
            y1 = y1.max(q[1]);
        }
        Some(vec![
            x0 - d,
            y0 - d,
            x1 + d,
            y0 - d,
            x1 + d,
            y1 + d,
            x0 - d,
            y1 + d,
        ])
    }
    fn clip_polygon_aabb(&mut self, _: &[f32], _: f32, _: f32, _: f32, _: f32) -> Vec<Vec<f32>> {
        Vec::new()
    }
}

#[test]
fn polyline_with_width_uses_outline_bounds() {
    let mut g = grid();
    g.clipper = Some(Box::new(BoxClipper));
    let p = props(&[("LineWidth", PropValue::Number(2.0))]);
    let id = g
        .register_geometry_zone(
            "r",
            "Nav",
            0,
            "polyline",
            &[10.0, 20.0, 40.0, 20.0],
            Some(&p),
        )
        .unwrap()
        .unwrap();
    let z = g.zone(id);
    assert_eq!((z.x, z.y, z.w, z.h, z.polyline_width), (9, 19, 34, 4, 2));
    assert_eq!(g.get_zone_at(15, 19, 0), Some(id));
    assert_eq!(g.get_zone_at(15, 20, 0), Some(id));
    assert_eq!(g.get_zone_at(15, 21, 0), None);
}

#[test]
fn vehicle_mannequin_world_gen_and_room_tone() {
    let mut g = grid();
    let p = props(&[
        ("Direction", PropValue::Str("South".into())),
        ("FaceDirection", PropValue::Bool(true)),
    ]);
    assert_eq!(
        g.register_vehicles_zone("v", "Lot", 0, 0, 0, 5, 5, None),
        Ok(None)
    );
    let v1 = g
        .register_vehicles_zone("v", "ParkingStall", 250, 10, 0, 10, 4, Some(&p))
        .unwrap()
        .unwrap();
    let v2 = g
        .register_vehicles_zone("v", "Vehicle", 250, 10, 0, 10, 4, None)
        .unwrap()
        .unwrap();
    assert!(g.zone(v1).is_face_direction());
    assert_eq!(g.zone(v1).vehicle().unwrap().dir, Some(1));
    assert_eq!(g.get_cell_data(1, 0).unwrap().vehicle_zones, vec![v1, v2]);
    assert_eq!(g.get_vehicle_zone_at(256, 12, 0), Some(v1));
    assert_eq!(g.get_vehicle_zone_at(256, 14, 0), None);
    // Vehicle zones are not in the chunk zone arrays.
    assert_eq!(g.get_zone_at(256, 12, 0), None);
    g.check_vehicles_zones();
    assert_eq!(g.vehicles_zones, vec![v1]);
    let bad = props(&[("Direction", PropValue::Str("Up".into()))]);
    assert!(g
        .register_vehicles_zone("v", "Vehicle", 0, 0, 0, 1, 1, Some(&bad))
        .is_err());

    let mq = props(&[
        ("Female", PropValue::Bool(true)),
        ("Outfit", PropValue::Str("Suit".into())),
    ]);
    let m = g
        .register_mannequin_zone("m", "Mannequin", 5, 5, 0, 1, 1, Some(&mq))
        .unwrap()
        .unwrap();
    match &g.zone(m).kind {
        ZoneKind::Mannequin(d) => {
            assert_eq!((d.female, d.outfit.as_deref()), (1, Some("Suit")))
        }
        k => panic!("{k:?}"),
    }
    assert_eq!(g.get_cell_data(0, 0).unwrap().mannequin_zones, vec![m]);

    let w = g
        .register_world_gen_zone("w", "WorldGen", 600, 600, 0, 10, 10, None)
        .unwrap();
    assert_eq!(g.zone(w).kind, ZoneKind::WorldGen { rocks: true });
    assert_eq!(
        g.get_cell_data(2, 2).unwrap().world_gen_zones,
        Some(vec![w])
    );

    let rt = props(&[
        ("RoomTone", PropValue::Str("Kitchen".into())),
        ("EntireBuilding", PropValue::Bool(true)),
    ]);
    g.register_room_tone("t", "RoomTone", 7, 8, 0, 1, 1, Some(&rt))
        .unwrap();
    let tones = &g.get_cell_data(0, 0).unwrap().room_tones;
    assert_eq!(tones[0].enum_value.as_deref(), Some("Kitchen"));
    assert!(tones[0].entire_building);
    // Cell (0,2) was never created.
    assert_eq!(
        g.register_room_tone("t", "RoomTone", 7, 520, 0, 1, 1, Some(&rt)),
        Err(ZoneError::NoCell)
    );
}

#[test]
fn remove_zones_for_cell_cuts_rectangles() {
    let mut g = grid();
    let big = g.register_zone("big", "Lot", 280, 10, 0, 40, 10);
    let inside = g.register_zone("in", "Lot", 400, 400, 0, 5, 5);
    let v = g
        .register_vehicles_zone("v", "Vehicle", 290, 12, 0, 20, 2, None)
        .unwrap()
        .unwrap();
    g.remove_zones_for_cell(1, 0);
    // `big` loses x >= 300; `inside` (in 300-cell (1,1)) stays.
    assert!(!g.zones.contains(&big));
    assert!(g.zones.contains(&inside));
    let piece = *g.zones.last().unwrap();
    let z = g.zone(piece);
    assert_eq!((z.x, z.y, z.w, z.h), (280, 10, 20, 10));
    assert!(!z.is_preferred_zone_for_square);
    assert_eq!(g.get_zone_at(299, 12, 0), Some(piece));
    assert_eq!(g.get_zone_at(300, 12, 0), None);
    // The vehicle zone list of meta cell (1,0) is filtered.
    assert!(g.get_cell_data(1, 0).unwrap().vehicle_zones.is_empty());
    assert_eq!(g.vehicles_zones, vec![v]);
}

#[test]
fn props_trait_is_object_safe() {
    let p = props(&[("A", PropValue::Number(1.0))]);
    let d: &dyn ZoneProps = &p;
    assert_eq!(d.rawget("A"), PropValue::Number(1.0));
    assert_eq!(d.rawget("B"), PropValue::Nil);
}

#[test]
fn vehicle_zone_views_per_cell() {
    let mut g = IsoMetaGrid::with_bounds(10, 20, 11, 21);
    g.zone_tables.iso_direction_names = ["N", "NW", "W", "SW", "S", "SE", "E", "NE"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    let p = props(&[
        ("Direction", PropValue::Str("E".into())),
        ("FaceDirection", PropValue::Bool(true)),
    ]);
    // Spans cells (10,20) and (11,20); registered in that order.
    let a = g
        .register_vehicles_zone("lot", "ParkingStall", 2810, 5130, 0, 20, 6, Some(&p))
        .unwrap()
        .unwrap();
    g.register_vehicles_zone("", "Vehicle", 2700, 5200, 0, 3, 3, None)
        .unwrap()
        .unwrap();
    // Not a vehicle zone type: no view, nothing registered.
    assert!(g
        .register_vehicles_zone("x", "TownZone", 2900, 5200, 0, 3, 3, None)
        .unwrap()
        .is_none());
    let c0 = g.cell_vehicle_zones(10, 20);
    assert_eq!(c0.len(), 2);
    let v = &c0[0];
    assert_eq!(
        (v.name.as_str(), v.zone_type.as_str(), v.x, v.y, v.w, v.h),
        ("lot", "ParkingStall", 2810, 5130, 20, 6)
    );
    assert_eq!(v.dir, Some(pz_world::IsoDirections::E));
    assert!(v.face_direction);
    assert!(!v.is_polyline() && v.is_rectangle());
    assert_eq!(v.polyline_length(), 0.0);
    assert_eq!(c0[1].dir, None);
    assert!(!c0[1].face_direction);
    let c1 = g.cell_vehicle_zones(11, 20);
    assert_eq!(c1.len(), 1);
    assert_eq!(c1[0], *v);
    assert!(g.cell_vehicle_zones(10, 21).is_empty());
    assert!(g.cell_vehicle_zones(50, 50).is_empty());
    // getVehicleZoneAt: the cell's zones in order.
    assert_eq!(g.vehicle_zone_at(2815, 5131, 0).as_ref(), Some(v));
    assert_eq!(g.get_vehicle_zone_at(2815, 5131, 0), Some(a));
    assert_eq!(g.vehicle_zone_at(2815, 5131, 1), None);
    assert_eq!(
        g.vehicle_zone_at(2701, 5201, 0).unwrap().zone_type,
        "Vehicle"
    );
}

#[test]
fn vehicle_zone_polyline_length() {
    let z = pz_world::VehicleZone {
        geometry_type: pz_world::ZoneGeometryType::Polyline,
        points: vec![0, 0, 3, 4, 3, 10],
        ..Default::default()
    };
    assert!(z.is_polyline());
    assert_eq!(z.polyline_length(), 11.0);
}
