//! Synthetic scenes with hand-checked results.

use pz_lighting::{Character, LightingError, LightingSession, SquareLighting, Transmission};

/// IsoDirections ordinal bits as `squareSet` expects them.
const N: i32 = 1 << 0;
const W: i32 = 1 << 2;
const S: i32 = 1 << 4;
const E: i32 = 1 << 6;
const ALL: i32 = 0xff;
const SEEN: i32 = 1 << 27;

#[derive(Clone)]
struct Sq {
    vu: i32,
    up: bool,
    down: bool,
    vm: i32,
    room: i64,
    open_air: bool,
    trans: [Transmission; 4],
}

impl Sq {
    fn outdoor() -> Sq {
        Sq { vu: ALL, up: true, down: false, vm: SEEN, room: -1, open_air: true, trans: [Transmission::default(); 4] }
    }
    fn indoor(room: i64) -> Sq {
        Sq { vu: ALL, up: false, down: false, vm: SEEN, room, open_air: false, trans: [Transmission::default(); 4] }
    }
}

/// One chunk (8x8, level 32) at chunk (10, 10); `f(x, y)` describes the
/// squares (`None` for a missing square).
struct Scene {
    sky: i32,
    squares: Vec<Option<Sq>>,
    player: Character,
}

const CX: i32 = 10;
const CY: i32 = 10;
const OX: i32 = CX * 8;
const OY: i32 = CY * 8;

impl Scene {
    fn new(sky: i32, f: impl Fn(i32, i32) -> Option<Sq>) -> Scene {
        let squares = (0..64).map(|i| f(i % 8, i / 8)).collect();
        let player = Character {
            x: OX as f32 + 0.5,
            y: OY as f32 + 0.5,
            z: 32.0,
            look_x: 1.0,
            look_y: 0.0,
            perception: 100.0,
            cone: -1.0,
            ..Character::default()
        };
        Scene { sky, squares, player }
    }

    fn frame(&self, s: &mut LightingSession, first: bool) {
        self.frame_lsu(s, first, if first { 100.0 } else { 0.0 });
    }

    /// `light_source_update` above 30 makes the next update rebuild the
    /// light lists (Java sets it to 100 whenever lights change).
    fn frame_lsu(&self, s: &mut LightingSession, first: bool, lsu: f32) {
        s.state_begin_update(0, CX, CY, 1, 1).unwrap();
        s.player_set(self.player).unwrap();
        s.state_end_frame(1.0, 1.0, 1.0, 0.5, 0.0, 30.0, 40.0, first, lsu, self.sky).unwrap();
        if first {
            s.chunk_begin_update(CX, CY, 32, 32).unwrap();
            s.chunk_level_begin_update(32).unwrap();
            for y in 0..8 {
                for x in 0..8 {
                    match &self.squares[(y * 8 + x) as usize] {
                        Some(q) => {
                            s.square_begin_update(x, y, 32).unwrap();
                            let level = if q.open_air { self.sky } else { 0 };
                            s.square_set(q.vu, q.up, q.down, false, q.vm, -1, q.room, level, q.open_air).unwrap();
                            s.square_set_light_transmission(q.trans).unwrap();
                            s.square_end_update().unwrap();
                        }
                        None => s.square_set_null(x, y, 32).unwrap(),
                    }
                }
            }
            s.chunk_level_end_update().unwrap();
            s.chunk_end_update().unwrap();
        }
        s.state_end_update().unwrap();
    }

    /// Two frames: the first hands the world over, the update computes, and
    /// the second picks the results up.
    fn run(&self) -> LightingSession {
        let mut s = LightingSession::new();
        s.configure(0.005);
        self.frame(&mut s, true);
        s.do_lighting_update(0, true);
        self.frame(&mut s, false);
        s
    }
}

fn get(s: &mut LightingSession, x: i32, y: i32) -> SquareLighting {
    SquareLighting(s.get_square_lighting(0, OX + x, OY + y, 32).unwrap().expect("square in grid"))
}

/// `(int)(min(1, dark * (c / 255)) * 255)`, the CalcLightInfo byte.
fn info_byte(c: u8, dark: f32) -> u8 {
    let v = dark * (f32::from(c) / 255.0);
    let v = if v > 1.0 { 1.0 } else { v };
    (v * 255.0) as i32 as u8
}

const SKY: i32 = 0x00_40_80_c8; // GameTime packs b | g << 8 | r << 16

#[test]
fn flat_outdoor_chunk_gets_the_sky_everywhere() {
    let mut s = Scene::new(SKY, |_, _| Some(Sq::outdoor())).run();
    for y in 0..8 {
        for x in 0..8 {
            let l = get(&mut s, x, y);
            assert_eq!(l.light_level(), SKY as u32);
            assert_eq!(l.dark_multi_raw(), 100000);
            assert_eq!(l.vis() & 1, 1);
            // Byte 0 of the sky is reported as "red" even though GameTime
            // stores blue there.
            assert_eq!(l.light_info(), [info_byte(0xc8, 1.0), info_byte(0x80, 1.0), info_byte(0x40, 1.0)]);
            let rgb = l.light_info();
            let expect = 0xff00_0000 | u32::from(rgb[0]) | (u32::from(rgb[1]) << 8) | (u32::from(rgb[2]) << 16);
            for k in 0..4 {
                assert_eq!(l.vertex(k), expect, "floor vertex {k} at {x},{y}");
            }
        }
    }
}

#[test]
fn results_lag_one_hand_off() {
    let scene = Scene::new(SKY, |_, _| Some(Sq::outdoor()));
    let mut s = LightingSession::new();
    assert_eq!(s.state_update_counter(0).unwrap(), -1);
    scene.frame(&mut s, true);
    s.do_lighting_update(0, true);
    assert!(s.waiting_for_main());
    // Nothing swapped yet: the front buffers are still empty.
    assert_eq!(get(&mut s, 3, 3).light_level(), 0);
    assert_eq!(s.state_update_counter(0).unwrap(), 0);
    scene.frame(&mut s, false);
    assert!(!s.waiting_for_main());
    assert_eq!(s.state_update_counter(0).unwrap(), 1);
    assert_eq!(get(&mut s, 3, 3).light_level(), SKY as u32);
}

#[test]
fn sunlight_falls_off_sixteen_per_square_indoors() {
    // Outdoor column x = 0; indoor squares further east, open edges.
    let mut s = Scene::new(0x00_c8_c8_c8, |x, _| Some(if x == 0 { Sq::outdoor() } else { Sq::indoor(7) })).run();
    for x in 0..8 {
        let v = (0xc8 - 16 * x).max(0) as u32;
        assert_eq!(get(&mut s, x, 4).light_level() & 0xff, v, "x = {x}");
    }
}

#[test]
fn closed_edge_scales_by_its_transmission() {
    // x = 0 outdoor; x >= 1 indoor; the edge between x = 0 and x = 1 is
    // closed and lets 50% through from the indoor side.
    let mut s = Scene::new(0x00_c8_c8_c8, |x, _| {
        if x == 0 {
            let mut q = Sq::outdoor();
            q.vu &= !E;
            Some(q)
        } else {
            let mut q = Sq::indoor(7);
            if x == 1 {
                q.vu &= !W;
                q.trans[0] = Transmission { r: 0.0, g: 0.0, b: 0.0, intensity: 0.5, mix: 0.0 };
            }
            Some(q)
        }
    })
    .run();
    let expect = ((200.0f32 / 255.0 * 0.5) * 255.0) as i32;
    assert_eq!(get(&mut s, 1, 4).light_level() & 0xff, expect as u32);
    assert_eq!(get(&mut s, 2, 4).light_level() & 0xff, (expect - 16) as u32);
}

#[test]
fn fully_closed_edge_without_transmission_stops_the_sun() {
    let mut s = Scene::new(0x00_c8_c8_c8, |x, _| {
        let mut q = if x < 4 { Sq::outdoor() } else { Sq::indoor(7) };
        if x == 3 {
            q.vu &= !E;
        }
        if x == 4 {
            q.vu &= !(W | N | S);
        }
        if x > 4 {
            q.vu = 0;
        }
        Some(q)
    })
    .run();
    // Every edge into the x >= 4 half is closed and none transmits, so the
    // indoor half stays dark.
    for x in 4..8 {
        assert_eq!(get(&mut s, x, 4).light_level(), 0, "x = {x}");
    }
}

#[test]
fn unseen_blocked_square_is_dark() {
    // The player at (0, 0) looks east; a wall between x = 3 and x = 4
    // blocks light (vision flags) and sight (vision matrix: bit
    // `dx + 3 dy + 9 dz + 13`), and the squares behind it were never seen.
    let mut s = Scene::new(0x00_c8_c8_c8, |x, _| {
        let mut q = Sq::outdoor();
        q.vm = 0;
        if x == 3 {
            q.vu &= !E;
            q.vm |= 1 << 14;
        }
        if x == 4 {
            q.vu &= !W;
            q.vm |= 1 << 12;
        }
        Some(q)
    })
    .run();
    let near = get(&mut s, 2, 0);
    assert_eq!(near.vis(), 7);
    assert_eq!(near.dark_multi_raw(), 100000);
    let far = get(&mut s, 6, 0);
    assert_eq!(far.vis() & 4, 0, "behind the wall is not in line of sight");
    assert_eq!(far.target_dark_multi_raw(), 0);
    assert_eq!(far.light_info(), [0, 0, 0]);
}

#[test]
fn room_light_reaches_its_room_and_falls_off_outside() {
    let scene = Scene::new(0, |x, _| Some(if x < 3 { Sq::indoor(9) } else { Sq::indoor(10) }));
    let mut s = LightingSession::new();
    s.add_room_light(100001, 1, 9, OX, OY, 32, 3, 8, true).unwrap();
    scene.frame(&mut s, true);
    s.do_lighting_update(0, true);
    // The room light is handed over at this swap and seeded by the next
    // update, which rebuilds the light lists because of the 100.
    scene.frame_lsu(&mut s, false, 100.0);
    s.do_lighting_update(1, false);
    scene.frame(&mut s, false);
    let inside = get(&mut s, 1, 1);
    assert_eq!(inside.light_count(), 1);
    let [id, x, y, z, radius, packed] = inside.light(0);
    assert_eq!((id, x, y, z, radius), (100001, OX + 1, OY + 1, 32, 7));
    assert_eq!(packed as u32, 0x01d1_d8e5);
    assert_eq!(inside.light_info(), [229, 216, 209]);
    let outside = get(&mut s, 5, 1);
    assert_eq!(outside.light_count(), 1);
    let r = (1.0f32 - 3.0 / 7.0) * 229.0;
    assert_eq!(outside.light_info()[0], r as i32 as u8);
    assert_eq!(s.get_visible_room_count(0).unwrap(), 2);
    let mut rooms = [0i64; 2];
    s.get_visible_rooms(0, &mut rooms).unwrap();
    assert_eq!(rooms, [9, 10]);
}

#[test]
fn missing_squares_break_nothing() {
    let mut s = Scene::new(SKY, |x, y| if (x + y) % 3 == 0 { None } else { Some(Sq::outdoor()) }).run();
    assert_eq!(get(&mut s, 1, 0).light_level(), SKY as u32);
}

#[test]
fn call_order_errors_match_the_native_messages() {
    let mut s = LightingSession::new();
    assert_eq!(s.state_end_update(), Err(LightingError::IllegalState("missing stateBeginUpdate?")));
    assert_eq!(s.state_begin_update(4, 0, 0, 1, 1), Err(LightingError::IllegalArgument("invalid playerIndex")));
    s.state_begin_update(0, 0, 0, 1, 1).unwrap();
    assert_eq!(s.state_begin_update(0, 0, 0, 1, 1), Err(LightingError::IllegalState("missing stateEndUpdate?")));
    assert_eq!(s.square_begin_update(0, 0, 32), Err(LightingError::IllegalState("missing chunkBeginUpdate?")));
    s.chunk_begin_update(0, 0, 32, 32).unwrap();
    assert_eq!(s.square_begin_update(0, 0, 40), Err(LightingError::IllegalArgument("invalid x,y,z")));
    assert_eq!(s.square_set(0, false, false, false, 0, -1, -1, 0, false), Err(LightingError::IllegalState("missing squareBeginUpdate?")));
    s.chunk_end_update().unwrap();
    s.state_end_update().unwrap();
    assert_eq!(s.add_light(1, 0, 0, 32, 5, 1.0, 1.0, 1.0, -1, true), Ok(()));
    assert_eq!(s.add_light(1, 0, 0, 32, 5, 1.0, 1.0, 1.0, -1, true), Err(LightingError::IllegalState("duplicate light id")));
    assert_eq!(s.remove_light(2), Err(LightingError::IllegalState("unknown light id")));
}

#[test]
fn point_light_falls_off_with_distance() {
    let scene = Scene::new(0, |_, _| Some(Sq::indoor(3)));
    let mut s = LightingSession::new();
    s.add_light(1, OX + 4, OY + 4, 32, 5, 1.0, 1.0, 1.0, -1, true).unwrap();
    scene.frame(&mut s, true);
    s.do_lighting_update(0, true);
    scene.frame_lsu(&mut s, false, 100.0);
    s.do_lighting_update(1, false);
    scene.frame(&mut s, false);
    let at = get(&mut s, 4, 4);
    assert_eq!(at.light_count(), 1);
    assert_eq!(at.light_info(), [255, 255, 255]);
    let two = get(&mut s, 6, 4);
    assert_eq!(two.light_info()[0], ((1.0f32 - 2.0 / 5.0) * 255.0) as u8);
    // (0, 0) is sqrt(32) away, beyond the radius.
    assert_eq!(get(&mut s, 0, 0).light_count(), 0);
}

#[test]
fn inactive_point_light_adds_nothing() {
    let scene = Scene::new(0, |_, _| Some(Sq::indoor(3)));
    let mut s = LightingSession::new();
    s.add_light(1, OX + 4, OY + 4, 32, 5, 1.0, 1.0, 1.0, -1, false).unwrap();
    scene.frame(&mut s, true);
    s.do_lighting_update(0, true);
    scene.frame_lsu(&mut s, false, 100.0);
    s.do_lighting_update(1, false);
    scene.frame(&mut s, false);
    assert_eq!(get(&mut s, 4, 4).light_count(), 0);
}

#[test]
fn vision_cone_limits_can_see() {
    // Looking east with a narrow cone and no close-range perception.
    let mut scene = Scene::new(0x00_c8_c8_c8, |_, _| {
        let mut q = Sq::outdoor();
        q.vm = 0;
        Some(q)
    });
    scene.player.x = OX as f32 + 4.5;
    scene.player.y = OY as f32 + 4.5;
    scene.player.perception = 0.0;
    scene.player.cone = -0.9;
    let mut s = scene.run();
    assert_eq!(get(&mut s, 7, 4).vis(), 7, "ahead");
    assert_eq!(get(&mut s, 1, 4).vis(), 4, "behind: in line of sight but outside the cone");
    assert_eq!(get(&mut s, 4, 4).vis(), 7, "own square");
}

#[test]
fn torch_lights_the_squares_ahead_on_the_player_level() {
    let mut scene = Scene::new(0, |_, _| Some(Sq::indoor(3)));
    scene.player.x = OX as f32 + 1.5;
    scene.player.y = OY as f32 + 4.5;
    let mut s = LightingSession::new();
    s.update_torch(1, OX as f32 + 1.5, OY as f32 + 4.5, 32.0, 1.0, 1.0, 1.0, 1.0, 0.0, 6.0, 1.0, false, 0.0, 0);
    scene.frame(&mut s, true);
    s.do_lighting_update(0, true);
    scene.frame(&mut s, false);
    s.do_lighting_update(1, false);
    scene.frame(&mut s, false);
    let ahead = get(&mut s, 4, 4);
    assert!(ahead.light_info()[0] > 0);
    assert_eq!(ahead.light_count(), 1);
    assert_eq!(ahead.light(0)[5] as u32 >> 24, 2, "torch entries are flagged 2");
    assert_eq!(get(&mut s, 0, 4).light_info(), [0, 0, 0], "behind the torch");
}

#[test]
fn see_all_renders_without_a_player_square() {
    let mut scene = Scene::new(SKY, |_, _| {
        let mut q = Sq::outdoor();
        q.vm = 0;
        Some(q)
    });
    scene.player.see_all = true;
    scene.player.x = -1000.0;
    let mut s = scene.run();
    let l = get(&mut s, 5, 5);
    assert_eq!(l.vis(), 7);
    assert_eq!(l.dark_multi_raw(), 100000);
    assert_eq!(l.light_info(), [info_byte(0xc8, 1.0), info_byte(0x80, 1.0), info_byte(0x40, 1.0)]);
}
