//! The Java-side port (`pz_lighting::jni`) on synthetic squares.

use std::collections::HashMap;

use pz_lighting::jni::*;
use pz_lighting::{Character, LightingError, LightingSession, TestResult, Transmission};

#[derive(Default)]
struct World(HashMap<(i32, i32, i32), GridSquare>);

impl LightingWorld for World {
    fn square(&self, x: i32, y: i32, z: i32) -> Option<&GridSquare> {
        self.0.get(&(x, y, z))
    }
}

impl World {
    fn put(&mut self, sq: GridSquare) {
        self.0.insert((sq.x, sq.y, sq.z), sq);
    }
    fn get_mut(&mut self, x: i32, y: i32, z: i32) -> &mut GridSquare {
        self.0.get_mut(&(x, y, z)).unwrap()
    }
}

fn outdoor(x: i32, y: i32) -> GridSquare {
    GridSquare { x, y, z: 0, exterior: true, ..GridSquare::default() }
}

/// visionMatrix bit for a neighbour delta.
fn wall(dx: i32, dy: i32) -> i32 {
    1 << ((dx + 1) + (dy + 1) * 3 + 9)
}

/// An 8x8 chunk at chunk (10, 10): outdoors, with a 3x3 room at
/// (83..86, 83..86) walled on its west side.
fn world() -> World {
    let mut w = World::default();
    for y in 80..88 {
        for x in 80..88 {
            let mut sq = outdoor(x, y);
            if (83..86).contains(&x) && (83..86).contains(&y) {
                sq.exterior = false;
                sq.room_id = Some(5);
                sq.building_id = Some(1);
            }
            if x == 83 && (83..86).contains(&y) {
                sq.vision_matrix |= wall(-1, 0);
            }
            if x == 82 && (83..86).contains(&y) {
                sq.vision_matrix |= wall(1, 0);
            }
            w.put(sq);
        }
    }
    w
}

#[test]
fn vision_flags_follow_the_vision_matrix() {
    let w = world();
    let a = square_args(&w, w.square(83, 84, 0).unwrap(), 0, 0x00c0c0c0);
    let west = 1 << IsoDirection::W as i32;
    let east = 1 << IsoDirection::E as i32;
    assert_eq!(a.vision_unblocked & west, 0);
    assert_ne!(a.vision_unblocked & east, 0);
    assert_eq!(a.room_id, 5);
    assert_eq!(a.building_id, 1);
    assert!(!a.open_air);
    let o = square_args(&w, w.square(80, 80, 0).unwrap(), 0, 0x00c0c0c0);
    assert!(o.open_air);
    assert_eq!(o.light_level, 0x00c0c0c0);
    assert_eq!(o.vision_unblocked, 0xff);
}

#[test]
fn open_air_needs_every_square_above_to_be_exterior() {
    let mut w = world();
    w.put(GridSquare { x: 80, y: 80, z: 1, exterior: false, ..GridSquare::default() });
    w.put(GridSquare { x: 81, y: 80, z: 1, exterior: true, ..GridSquare::default() });
    assert!(!open_air(&w, w.square(80, 80, 0).unwrap()));
    assert!(open_air(&w, w.square(81, 80, 0).unwrap()));
    let mut b = outdoor(0, 0);
    b.z = -1;
    let mut bw = World::default();
    bw.put(b.clone());
    assert!(!open_air(&bw, &b), "below ground is never open air");
}

#[test]
fn doors_windows_and_curtains_in_line_of_sight() {
    let mut w = world();
    let door = Door { north: false, open: false, ..Door::default() };
    w.get_mut(86, 84, 0).special_objects.push(SpecialObject::Door(door));
    // From inside to the door square's west edge: (85,84) -> (86,84).
    let from = w.square(85, 84, 0).unwrap().clone();
    assert_eq!(test_vision_adjacent(&w, &from, 1, 0, 0, true, false), TestResult::Blocked);
    assert_eq!(test_vision_adjacent(&w, &from, 1, 0, 0, true, true), TestResult::Clear, "doors ignored");
    w.get_mut(86, 84, 0).special_objects[0] = SpecialObject::Door(Door { open: true, ..door });
    // An open west door swings to the north edge.
    assert_eq!(test_vision_adjacent(&w, &from, 1, 0, 0, true, false), TestResult::Clear);
    w.get_mut(86, 84, 0).special_objects[0] = SpecialObject::Door(Door { door_trans: true, ..door });
    assert_eq!(test_vision_adjacent(&w, &from, 1, 0, 0, true, false), TestResult::ClearThroughClosedDoor);
    w.get_mut(86, 84, 0).special_objects[0] = SpecialObject::Window(Window::default());
    assert_eq!(test_vision_adjacent(&w, &from, 1, 0, 0, true, false), TestResult::ClearThroughWindow);
    let curtain = Curtain {
        kind: CurtainType::W,
        north: false,
        open: false,
        light_filter_r: None,
        light_filter_g: None,
        light_filter_b: None,
        light_filter_intensity: None,
        light_filter_mix: None,
    };
    w.get_mut(86, 84, 0).special_objects.push(SpecialObject::Curtain(curtain));
    assert_eq!(test_vision_adjacent(&w, &from, 1, 0, 0, true, false), TestResult::Blocked);
}

#[test]
fn transmissions_from_doors_curtains_and_barricades() {
    let mut w = world();
    let sq = w.get_mut(84, 84, 0);
    sq.special_objects.push(SpecialObject::Door(Door { north: true, ..Door::default() }));
    sq.special_objects.push(SpecialObject::Curtain(Curtain {
        kind: CurtainType::W,
        north: false,
        open: false,
        light_filter_r: Some("255".into()),
        light_filter_g: Some(" 51 ".into()),
        light_filter_b: Some("x".into()),
        light_filter_intensity: Some("40".into()),
        light_filter_mix: None,
    }));
    let planks = Barricade { dir: Some(IsoDirection::S), planks: 2, ..Barricade::default() };
    sq.special_objects.push(SpecialObject::Window(Window { north: true, open: false, barricade_same: None, barricade_opposite: Some(planks) }));
    let a = square_args(&w, w.square(84, 84, 0).unwrap(), 0, 0);
    let [tw, tn, te, ts] = a.transmissions;
    assert_eq!(tn, Transmission { r: 0.0, g: 0.0, b: 0.0, intensity: 0.15, mix: 0.0 }, "closed door");
    assert_eq!(tw, Transmission { r: 1.0, g: 51.0 / 255.0, b: 0.0, intensity: 0.4, mix: 0.0 }, "curtain filter");
    assert_eq!(te.intensity, 0.0);
    assert_eq!(ts.intensity, 1.0 - 2.0 * 0.15, "two planks");
    assert_eq!(
        a.objects,
        vec![
            ObjectCall::Door { north: true, open: false, trans: false },
            ObjectCall::Curtain { wnes: 4, open: false },
            ObjectCall::Window { north: true, open: false, opaque: false },
        ]
    );
}

#[test]
fn vision_cone_settles_on_the_daylight_cone() {
    let mut c = VisionCone::default();
    let input = VisionConeInput { day_light_strength: 1.0, luma_inverted: 0.0, worn_items_vision_multiplier: 1.0, ..Default::default() };
    let mut cone = 0.0;
    for _ in 0..2000 {
        cone = c.calculate(&input);
    }
    // 144 degrees -> (144 - 180) / 180.
    assert!((cone - (-0.2)).abs() < 0.001, "{cone}");
    assert_eq!(effective_fatigue(0.8), (0.8f32 - 0.6) * 2.5);
    assert_eq!(detection_range(false, false, false, 1.0, 0.0, 0.0), 3.5);
}

fn frame_for(chunks: &[ChunkInfo], player: PlayerInput) -> PlayerFrame<'_> {
    PlayerFrame {
        index: 0,
        chunk_x_min: 10,
        chunk_y_min: 10,
        chunk_grid_width: 1,
        player: Some(player),
        cone: -1.0,
        render: PlayerRenderSettings { rmod: 0.5, gmod: 0.4, bmod: 0.3, ambient: 1.0, night: 0.0, view_distance: 30.0 },
        cache_cleared: false,
        chunks,
    }
}

#[test]
fn a_frame_drives_the_library_and_the_decoder_flags_the_level() {
    let w = world();
    let mut s = LightingSession::new();
    let mut chunk = ChunkInfo { wx: 10, wy: 10, min_level: 0, max_level: 0, loaded: true, light_check: true, level_light_check: vec![true] };
    let player = PlayerInput { x: 80.5, y: 80.5, z: 0.0, look_x: 1.0, detection_range: 100.0, forward: Some(IsoDirection::E), ..Default::default() };
    let render = frame_for(&[], player).render;
    let params = FrameParams { view_dist_max: 40.0, light_source_update: 100.0, sky_light_level: render.sky_light_level() };
    let r = update_player(&mut s, &w, &frame_for(std::slice::from_ref(&chunk), player), &params).unwrap();
    assert_eq!(r.chunks_sent, vec![(10, 10)]);
    assert_eq!(r.levels_sent, vec![(10, 10, 0)]);
    assert_eq!(r.lighting_never_done, vec![((10, 10), true)]);
    assert_eq!(r.update_counter, 0);
    chunk.light_check = false;
    s.do_lighting_update(0, true);
    let params = FrameParams { light_source_update: 0.0, ..params };
    let r = update_player(&mut s, &w, &frame_for(std::slice::from_ref(&chunk), player), &params).unwrap();
    assert_eq!(r.update_counter, 1);
    assert!(r.chunks_sent.is_empty());

    let mut level = ChunkLevelLight::new(10, 10, 0);
    let dp = DecodePlayer { x: 80.5, y: 80.5, z: 0.0, forward: Some(IsoDirection::E) };
    let lr = level.refresh(&mut s, 0, r.update_counter, Some(&dp), true, |_| true).unwrap();
    assert!(lr.invalidate);
    assert!(lr.first_seen.contains(&0), "the player's square is seen");
    assert!(lr.first_seen.len() < 64, "the wall hides some squares");
    let out = &level.squares[0];
    // Sky level packs b | g << 8 | r << 16 and is read back byte 0 first:
    // the light colour's red is the sky's blue.
    let sky = params.sky_light_level as u32;
    assert_eq!(out.light_level as u32, sky);
    assert_eq!((out.light_info.r * 255.0).round() as u32, sky & 0xff);
    assert_eq!(out.dark_multi, 1.0);
    // Same tick again: nothing to read.
    let lr = level.refresh(&mut s, 0, r.update_counter, Some(&dp), true, |_| true).unwrap();
    assert!(!lr.invalidate);
    assert!(lr.first_seen.is_empty());
}

#[test]
fn legacy_decode_dims_levels_below_the_player() {
    let w = world();
    let mut s = LightingSession::new();
    let chunk = ChunkInfo { wx: 10, wy: 10, min_level: 0, max_level: 0, loaded: true, light_check: true, level_light_check: vec![true] };
    let player = PlayerInput { x: 80.5, y: 80.5, z: 0.0, detection_range: 100.0, ..Default::default() };
    let params = FrameParams { view_dist_max: 40.0, light_source_update: 100.0, sky_light_level: 0x00c8c8c8 };
    update_player(&mut s, &w, &frame_for(std::slice::from_ref(&chunk), player), &params).unwrap();
    s.do_lighting_update(0, true);
    let chunk = ChunkInfo { light_check: false, ..chunk };
    let r = update_player(&mut s, &w, &frame_for(std::slice::from_ref(&chunk), player), &params).unwrap();
    let pos = SquarePos { x: 81, y: 81, z: 0 };
    let mut same = SquareLight::new();
    same.update_legacy(&mut s, 0, pos, r.update_counter, Some(&DecodePlayer { z: 0.0, ..Default::default() })).unwrap();
    let raw = s.get_vert_light(0, 81, 81, 32, 0).unwrap() as u32;
    assert_eq!(same.vert_light[0], raw);
    // The same block decoded for a player one level up: floor corners
    // * 0.85, wall tops unchanged.
    let mut below = SquareLight::new();
    let mut buf = [0i32; 49];
    s.get_square_lighting_into(0, 81, 81, 32, &mut buf).unwrap();
    let mut n = Fixed(buf);
    below.update_legacy(&mut n, 0, pos, r.update_counter, Some(&DecodePlayer { z: 1.0, ..Default::default() })).unwrap();
    let c = 0xc8 as f32 * 0.85;
    assert_eq!(below.vert_light[0] & 0xff, c as u32);
    assert_eq!(below.vert_light[4], raw | 0xff00_0000, "wall tops unchanged one level down");
}

/// A `LightingNatives` that serves one fixed `getSquareLighting` block and
/// records the other calls.
struct Fixed([i32; 49]);

#[allow(unused_variables)]
impl LightingNatives for Fixed {
    fn state_begin_update(&mut self, p: i32, x: i32, y: i32, w: i32, h: i32) -> Result<(), LightingError> {
        Ok(())
    }
    fn player_set(&mut self, c: Character) -> Result<(), LightingError> {
        Ok(())
    }
    fn state_end_frame(&mut self, a: f32, b: f32, c: f32, d: f32, e: f32, f: f32, g: f32, h: bool, i: f32, j: i32) -> Result<(), LightingError> {
        Ok(())
    }
    fn state_end_update(&mut self) -> Result<(), LightingError> {
        Ok(())
    }
    fn state_update_counter(&mut self, p: i32) -> Result<i32, LightingError> {
        Ok(0)
    }
    fn chunk_lighting_done(&mut self, wx: i32, wy: i32) -> Result<bool, LightingError> {
        Ok(true)
    }
    fn chunk_begin_update(&mut self, wx: i32, wy: i32, min: i32, max: i32) -> Result<(), LightingError> {
        Ok(())
    }
    fn chunk_end_update(&mut self) -> Result<(), LightingError> {
        Ok(())
    }
    fn chunk_level_begin_update(&mut self, z: i32) -> Result<(), LightingError> {
        Ok(())
    }
    fn chunk_level_end_update(&mut self) -> Result<(), LightingError> {
        Ok(())
    }
    fn square_set_null(&mut self, x: i32, y: i32, z: i32) -> Result<(), LightingError> {
        Ok(())
    }
    fn square_begin_update(&mut self, x: i32, y: i32, z: i32) -> Result<(), LightingError> {
        Ok(())
    }
    fn square_set(&mut self, vu: i32, up: bool, down: bool, el: bool, vm: i32, b: i64, r: i64, lv: i32, oa: bool) -> Result<(), LightingError> {
        Ok(())
    }
    fn square_set_light_transmission(&mut self, t: [Transmission; 4]) -> Result<(), LightingError> {
        Ok(())
    }
    fn square_add_curtain(&mut self, wnes: i32, open: bool) -> Result<(), LightingError> {
        Ok(())
    }
    fn square_add_door(&mut self, north: bool, open: bool, trans: bool) -> Result<(), LightingError> {
        Ok(())
    }
    fn square_add_thumpable(&mut self, a: bool, b: bool, c: bool, d: bool) -> Result<(), LightingError> {
        Ok(())
    }
    fn square_add_window(&mut self, a: bool, b: bool, c: bool) -> Result<(), LightingError> {
        Ok(())
    }
    fn square_end_update(&mut self) -> Result<(), LightingError> {
        Ok(())
    }
    fn add_light(&mut self, id: i32, x: i32, y: i32, z: i32, r: i32, a: f32, b: f32, c: f32, bu: i32, ac: bool) -> Result<(), LightingError> {
        Ok(())
    }
    fn add_temp_light(&mut self, id: i32, x: i32, y: i32, z: i32, r: i32, a: f32, b: f32, c: f32, l: i32) -> Result<(), LightingError> {
        Ok(())
    }
    fn remove_light(&mut self, id: i32) -> Result<(), LightingError> {
        Ok(())
    }
    fn set_light_active(&mut self, id: i32, a: bool) -> Result<(), LightingError> {
        Ok(())
    }
    fn set_light_color(&mut self, id: i32, r: f32, g: f32, b: f32) -> Result<(), LightingError> {
        Ok(())
    }
    fn add_room_light(&mut self, id: i32, b: i64, r: i64, x: i32, y: i32, z: i32, w: i32, h: i32, a: bool) -> Result<(), LightingError> {
        Ok(())
    }
    fn remove_room_light(&mut self, id: i32) -> Result<(), LightingError> {
        Ok(())
    }
    fn set_room_light_active(&mut self, id: i32, a: bool) -> Result<(), LightingError> {
        Ok(())
    }
    fn update_torch(&mut self, id: i32, x: f32, y: f32, z: f32, r: f32, g: f32, b: f32, ax: f32, ay: f32, d: f32, s: f32, c: bool, dot: f32, f: i32) -> Result<(), LightingError> {
        Ok(())
    }
    fn remove_torch(&mut self, id: i32) -> Result<(), LightingError> {
        Ok(())
    }
    fn get_square_dirty(&mut self, p: i32, x: i32, y: i32, z: i32) -> Result<bool, LightingError> {
        Ok(true)
    }
    fn get_square_lighting(&mut self, p: i32, x: i32, y: i32, z: i32, out: &mut [i32; 49]) -> Result<bool, LightingError> {
        *out = self.0;
        Ok(true)
    }
    fn get_visible_room_count(&mut self, p: i32) -> Result<i32, LightingError> {
        Ok(0)
    }
    fn get_visible_rooms(&mut self, p: i32, out: &mut [i64]) -> Result<i32, LightingError> {
        Ok(0)
    }
}

#[test]
fn forced_vis_marks_the_squares_just_ahead() {
    let mut block = [0i32; 49];
    block[0] = 4; // in line of sight, outside the cone
    let mut n = Fixed(block);
    let p = DecodePlayer { x: 10.5, y: 10.5, z: 0.0, forward: Some(IsoDirection::E) };
    let mut ahead = SquareLight::new();
    ahead.update_fbo(&mut n, 0, SquarePos { x: 11, y: 10, z: 0 }, 1, Some(&p), false).unwrap();
    assert!(ahead.can_see());
    let mut behind = SquareLight::new();
    behind.update_fbo(&mut n, 0, SquarePos { x: 9, y: 10, z: 0 }, 1, Some(&p), false).unwrap();
    assert!(!behind.can_see());
}

#[test]
fn light_sync_assigns_ids_and_follows_changes() {
    let mut s = LightingSession::new();
    let mut sync = LightSync::new();
    let lamp = LightSource { key: 7, x: 1, y: 2, z: 0, radius: 30, r: 0.3, g: 0.6, b: 0.9, active: true, life: -1, local_to_building: None, alive: true };
    let temp = LightSource { key: 8, life: 5, ..lamp };
    let room = RoomLightSource { key: 9, building_id: 1, room_id: 2, x: 0, y: 0, z: 0, width: 3, height: 3, active: true, in_bounds: true };
    let r = sync.check_lights(&mut s, &[lamp, temp], &[room], &[], 30).unwrap();
    assert!(r.light_source_update);
    assert_eq!(r.consumed_temp_lights, vec![8]);
    assert_eq!(sync.light_id(7), Some(1));
    assert_eq!(sync.room_light_id(9), Some(100001));
    // Unchanged: no new calls, no update flag.
    let r = sync.check_lights(&mut s, &[lamp], &[room], &[], 30).unwrap();
    assert!(!r.light_source_update);
    // Re-adding the same id would fail, so success means nothing was re-sent.
    let r = sync.check_lights(&mut s, &[LightSource { alive: false, ..lamp }], &[RoomLightSource { in_bounds: false, ..room }], &[], 30).unwrap();
    assert!(r.light_source_update);
    assert_eq!(r.removed_lights, vec![7]);
    assert_eq!(sync.light_id(7), None);
    assert_eq!(s.remove_light(1), Err(LightingError::IllegalState("unknown light id")));
    assert_eq!(player_torch_id(1, 2), 7);
    assert_eq!(vehicle_torch_id(3, 1), 4127);
}

#[test]
fn sky_light_level_packs_blue_first() {
    let r = PlayerRenderSettings { rmod: 0.5, gmod: 0.25, bmod: 0.0, ambient: 1.0, night: 0.0, view_distance: 0.0 };
    assert_eq!(r.sky_light_level(), (255 << 16) | (127 << 8));
}
