//! Vehicle lights in the lighting pass: headlights as torches through
//! `WorldHooks::torches` (`LightingJNI.checkLights`' vehicle loop) and the
//! lightbar's lampposts (`BaseVehicle.updateWorldLights`) on a hand-built
//! floor at night. Needs the game's vehicle scripts (`PZ_GAME_DIR`).

mod common;

use common::night::*;
use pz_lighting::jni::IsoDirection;
use pz_vehicle_render::lights::LightbarWorldLights;
use pz_world_render::{RenderWorld, WorldHooks, WorldView};

#[test]
fn headlights_light_the_squares_in_front() {
    let Some(game) = common::game_dir() else {
        return;
    };
    common::scripts(&game);
    let (cell, m, meta) = scene();
    let view = WorldView {
        cell: &cell,
        meta: &meta,
        sprites: &m,
    };
    let v = common::vehicle("Base.SportsCar_ez", 16.5, 16.5, 0.0);
    charge_battery(&v);
    for (id, x, y) in [
        ("HeadlightLeft", 0.5f32, 1.0f32),
        ("HeadlightRight", -0.5, 1.0),
        ("HeadlightRearLeft", 0.5, -1.0),
        ("HeadlightRearRight", -0.5, -1.0),
    ] {
        v.part_by_id(id)
            .unwrap()
            .create_spot_light(x, y, 8.0, 1.0, 0.96, 0);
    }
    v.create_physics(false);
    v.st_mut().vehicle_id = 1;
    v.set_headlights_on(true);
    // Front lights only first (`Vehicles.Update.Headlight` turns all four
    // on with the headlights; the rear pair is checked below).
    for id in ["HeadlightLeft", "HeadlightRight"] {
        v.part_by_id(id).unwrap().set_light_active(true);
    }
    let vehicles = [&*v];
    let hooks = VehicleTorches(&vehicles);
    let torches = hooks.torches();
    assert_eq!(torches.len(), 2, "{torches:?}");
    assert!(torches.iter().all(|t| t.id >= 4096 + 10 && t.cone));

    // The player stands at the car (as its driver would) and looks along
    // the beam (`S`, the car's forward here) or back (`N`); at night the
    // vision cone only shows squares close to that line.
    let t = torches[0];
    let (fx, fy) = (t.angle_x, t.angle_y);
    assert!(fx.abs() < 1e-3 && fy > 0.99, "car faces south: {t:?}");
    let pos = (16.5, 16.5);
    let dark_hooks = VehicleTorches(&[]);
    let mut dark = new_lighting(&view, &dark_hooks);
    let mut lit = new_lighting(&view, &hooks);
    let off_s = light_from(&view, &mut dark, &dark_hooks, 12, pos, IsoDirection::S);
    let on_s = light_from(&view, &mut lit, &hooks, 12, pos, IsoDirection::S);
    let off_n = light_from(&view, &mut dark, &dark_hooks, 12, pos, IsoDirection::N);
    let on_n = light_from(&view, &mut lit, &hooks, 12, pos, IsoDirection::N);

    // Along the car's axis, `d` squares from its centre (the car reaches
    // 1.9 squares each way).
    let at = |w: &RenderWorld, d: f32| {
        let x = (16.5 + fx * d).floor() as i32;
        let y = (16.5 + fy * d).floor() as i32;
        w.squares[&(x, y, 0)].light_level
    };
    for d in [2.5f32, 3.5, 4.5] {
        let (a, b) = (at(&on_s, d), at(&off_s, d));
        assert!(a > b + 0.1, "ahead {d}: on {a} off {b}");
        let (a, b) = (at(&on_n, -d), at(&off_n, -d));
        assert!(b > 0.0, "behind {d} is seen");
        assert!((a - b).abs() < 0.01, "behind {d}: on {a} off {b}");
    }

    // The rear pair (`Vehicles.Update.Headlight` turns it on with the
    // headlights) lights behind the car, as in the original.
    for id in ["HeadlightRearLeft", "HeadlightRearRight"] {
        v.part_by_id(id).unwrap().set_light_active(true);
    }
    assert_eq!(hooks.torches().len(), 4);
    let on_n = light_from(&view, &mut lit, &hooks, 12, pos, IsoDirection::N);
    assert!(at(&on_n, -3.5) > at(&off_n, -3.5) + 0.1);

    // Headlights off: the torches leave the lighting again.
    for p in v.parts() {
        if p.id().is_some_and(|i| i.contains("Headlight")) {
            p.set_light_active(false);
        }
    }
    assert!(hooks.torches().is_empty());
    let back = light_from(&view, &mut lit, &hooks, 12, pos, IsoDirection::S);
    assert!((at(&back, 3.5) - at(&off_s, 3.5)).abs() < 0.01);
}

#[test]
fn lightbar_adds_and_removes_a_lamppost() {
    let Some(game) = common::game_dir() else {
        return;
    };
    common::scripts(&game);
    let (cell, m, meta) = scene();
    let view = WorldView {
        cell: &cell,
        meta: &meta,
        sprites: &m,
    };
    let v = common::vehicle("Base.CarLightsPolice", 16.5, 16.5, 0.0);
    charge_battery(&v);
    v.st_mut().vehicle_id = 2;
    assert!(v.has_lightbar());
    let hooks = VehicleTorches(&[]);
    let mut lighting = new_lighting(&view, &hooks);
    let mut bar = LightbarWorldLights::new(&v);
    let off = light(&view, &mut lighting, &hooks, 12);

    // Off: nothing in the cell's list.
    bar.update(&v, &mut lighting);
    assert_eq!(lighting.lampposts().count(), 0);

    // Lights on, left lamp lit (`getLightTexIndex() == 1`).
    {
        let mut st = v.st_mut();
        st.lightbar_lights_mode.mode = 1;
        st.lightbar_lights_mode.light = 1;
    }
    bar.update(&v, &mut lighting);
    let (left, right) = bar.keys();
    let key = left.expect("left lamppost");
    assert!(right.is_none());
    let l: Vec<_> = lighting
        .lampposts()
        .map(|(k, l, dead)| (k, *l, dead))
        .collect();
    assert_eq!(l.len(), 1);
    let (_, lamp, dead) = l[0];
    assert!(!dead && lamp.radius == 8 && lamp.active);
    // `createParts`' siren colours, else the field initialiser's blue.
    let sc = v.script().unwrap();
    let col = match (sc.borrow().left_siren_col, sc.borrow().right_siren_col) {
        (Some(l), Some(_)) => (l.r, l.g, l.b),
        _ => (0.0, 0.0, 1.0),
    };
    assert_eq!((lamp.r, lamp.g, lamp.b), col);
    let p = v.world_pos(0.4, 0.0, 0.0);
    assert_eq!((lamp.x, lamp.y), (p.x.floor() as i32, p.y.floor() as i32));
    // Same square next update: the same lamppost.
    bar.update(&v, &mut lighting);
    assert_eq!(bar.keys().0, Some(key));
    assert_eq!(lighting.lampposts().count(), 1);

    let on = light(&view, &mut lighting, &hooks, 12);
    assert!(
        lighting.light_id(key).is_some(),
        "sent to the lighting library"
    );
    let (x, y) = (lamp.x, lamp.y);
    let near = |w: &RenderWorld| w.squares[&(x + 1, y, 0)].light_level;
    assert!(
        near(&on) > near(&off) + 0.05,
        "on {} off {}",
        near(&on),
        near(&off)
    );

    // The right lamp's turn: the left lamppost goes, a right one comes.
    v.st_mut().lightbar_lights_mode.light = 2;
    bar.update(&v, &mut lighting);
    let (left, right) = bar.keys();
    assert!(left.is_none() && right.is_some());
    let l: Vec<_> = lighting
        .lampposts()
        .map(|(k, _, dead)| (k, (), dead))
        .collect();
    assert_eq!(l.len(), 2);
    assert!(l.iter().any(|&(k, _, dead)| k == key && dead));

    // Battery flat / lights off: removed, and gone after the next frame.
    v.st_mut().lightbar_lights_mode.mode = 0;
    bar.update(&v, &mut lighting);
    assert_eq!(bar.keys(), (None, None));
    assert!(lighting.lampposts().all(|(_, _, dead)| dead));
    let after = light(&view, &mut lighting, &hooks, 12);
    assert_eq!(lighting.lampposts().count(), 0);
    assert!((near(&after) - near(&off)).abs() < 0.01);
}
