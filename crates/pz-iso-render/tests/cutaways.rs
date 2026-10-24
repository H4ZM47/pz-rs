//! Cutaways, building collapse, occlusion and the target-alpha rules on
//! hand-built squares.

use std::rc::Rc;

use pz_iso_render::alpha::{WallDraw, WallSide};
use pz_iso_render::cell::Layer;
use pz_iso_render::cutaways::{BuildingDef, GridWorld, CUTAWAY_N, CUTAWAY_W};
use pz_iso_render::world::{BuildingId, SpriteFlags, TileType};
use pz_iso_render::{CutawayPlayer, CutawayView, Cutaways, ObjectKind, RenderObject, RenderSquare, SpriteInfo};

fn sprite(f: impl FnOnce(&mut SpriteInfo)) -> Rc<SpriteInfo> {
    let mut s = SpriteInfo { tint_mod: [1.0; 3], ..Default::default() };
    f(&mut s);
    Rc::new(s)
}

fn floor() -> RenderObject {
    RenderObject::new(ObjectKind::Plain, Some(sprite(|s| s.flags = SpriteFlags { solidfloor: true, ..Default::default() })))
}

fn north_wall() -> RenderObject {
    RenderObject::new(
        ObjectKind::Plain,
        Some(sprite(|s| {
            s.cut_n = true;
            s.tile_type = TileType::Wall;
            s.flags.wall_n = true;
        })),
    )
}

fn west_wall() -> RenderObject {
    RenderObject::new(
        ObjectKind::Plain,
        Some(sprite(|s| {
            s.cut_w = true;
            s.tile_type = TileType::Wall;
            s.flags.wall_w = true;
        })),
    )
}

/// A floor square, optionally in a room of a building.
fn square(x: i32, y: i32, z: i32, room: Option<(i64, u32)>) -> RenderSquare {
    let mut sq = RenderSquare::new(x, y, z);
    sq.objects.push(floor());
    if let Some((r, b)) = room {
        sq.cut.room = Some(r);
        sq.cut.building = Some(BuildingId(b));
        sq.cut.associated_building = Some(BuildingId(b));
    }
    sq.cut.vision_blocked_below = true;
    sq
}

/// Open ground `[x0, x1) x [y0, y1)` at level 0.
fn ground(w: &mut GridWorld, x0: i32, y0: i32, x1: i32, y1: i32) {
    for y in y0..y1 {
        for x in x0..x1 {
            w.insert(square(x, y, 0, None));
        }
    }
}

/// A two-storey building: rooms 1 (level 0) and 2 (level 1) over
/// `[x0, x1) x [y0, y1)`.
fn building(w: &mut GridWorld, id: u32, x0: i32, y0: i32, x1: i32, y1: i32) {
    w.buildings.insert(BuildingId(id), BuildingDef { x: x0, y: y0, x2: x1, y2: y1, entirely_empty_outside: false });
    for z in 0..2 {
        for y in y0..y1 {
            for x in x0..x1 {
                w.insert(square(x, y, z, Some((id as i64 * 10 + z as i64, id))));
            }
        }
    }
}

#[test]
fn a_building_in_front_of_the_player_collapses_above_the_player() {
    let mut w = GridWorld::default();
    ground(&mut w, 0, 0, 24, 24);
    building(&mut w, 7, 10, 10, 16, 16);
    let mut c = Cutaways::new();
    // The level-1 floor at (11, 11) projects onto (8, 8) at level 0.
    let st = c.update(&w, &CutawayPlayer::at(8.5, 8.5, 0.0));
    assert_eq!(st.buildings_to_collapse, vec![BuildingId(7)]);
    assert!(!st.should_render_square(12, 12, 1));
    assert!(st.should_render_square(12, 12, 0));
    assert!(st.can_building_square_occlude_player(9, 9));
    assert!(!st.can_building_square_occlude_player(8, 8));
    // From the far side nothing is in the way.
    let st2 = c.update(&w, &CutawayPlayer::at(20.5, 20.5, 0.0));
    assert!(st2.buildings_to_collapse.is_empty());
    assert!(st2.should_render_square(12, 12, 1));
    // The upper level's chunks change.
    let dirty = st2.dirty_chunk_levels(&st);
    assert!(dirty.contains(&(1, 1, 1)));
    assert!(!dirty.iter().any(|k| k.2 == 0));
}

#[test]
fn multi_level_objects_of_a_collapsed_building_are_force_rendered() {
    let mut w = GridWorld::default();
    ground(&mut w, 0, 0, 24, 24);
    building(&mut w, 7, 10, 10, 16, 16);
    // The top half of a two-level object at (12, 12, 1).
    let top = RenderObject::new(
        ObjectKind::Plain,
        Some(sprite(|s| {
            s.grid_levels = 2;
            s.grid_pos_z = 1;
        })),
    );
    w.square_mut(12, 12, 1).objects.push(top);
    let st = Cutaways::new().update(&w, &CutawayPlayer::at(8.5, 8.5, 0.0));
    assert!(st.should_render_square(12, 12, 1));
    assert!(st.is_force_render_square(12, 12, 1));
    assert!(!st.should_render_square(13, 12, 1));
}

#[test]
fn fences_in_front_of_the_player_are_cut_away() {
    let mut w = GridWorld::default();
    ground(&mut w, 0, 0, 16, 16);
    for x in 1..5 {
        w.square_mut(x, 6, 0).objects.push(north_wall());
    }
    for y in 9..12 {
        w.square_mut(12, y, 0).objects.push(west_wall());
    }
    let mut c = Cutaways::new();
    let st = c.update(&w, &CutawayPlayer::at(2.5, 3.5, 0.0));
    for x in 1..5 {
        assert_eq!(st.cutaway_flags(x, 6, 0), CUTAWAY_N, "x {x}");
    }
    assert_eq!(st.cutaway_flags(5, 6, 0), 0);
    // The west fence's column is out of range of x = 2 (12 - 6 = 6 > 2).
    assert_eq!(st.cutaway_flags(12, 10, 0), 0);
    let st = c.update(&w, &CutawayPlayer::at(8.5, 10.5, 0.0));
    assert_eq!(st.cutaway_flags(12, 10, 0), CUTAWAY_W);
    // Behind the fence nothing is cut.
    assert_eq!(st.cutaway_flags(2, 6, 0), 0);
    // Points of interest ahead of the player count too: from (2, 8) the
    // squares around include (3, 5), in front of the fence.
    let st = c.update(&w, &CutawayPlayer::at(2.5, 8.5, 0.0));
    assert_eq!(st.cutaway_flags(2, 6, 0), CUTAWAY_N);
    let st = c.update(&w, &CutawayPlayer::at(2.5, 12.5, 0.0));
    assert_eq!(st.cutaway_flags(2, 6, 0), 0);
}

/// Rooms 1 (y < 5) and 2 (y >= 5) of building 3, split by a north wall
/// at y = 5.
fn two_rooms() -> GridWorld {
    let mut w = GridWorld::default();
    ground(&mut w, -4, -4, 20, 20);
    w.buildings.insert(BuildingId(3), BuildingDef { x: 0, y: 0, x2: 8, y2: 10, entirely_empty_outside: false });
    for y in 0..10 {
        for x in 0..8 {
            let room = if y < 5 { 1 } else { 2 };
            w.insert(square(x, y, 0, Some((room, 3))));
        }
    }
    for x in 0..8 {
        w.square_mut(x, 5, 0).objects.push(north_wall());
    }
    w
}

#[test]
fn walls_hiding_the_players_room_are_cut_away() {
    let w = two_rooms();
    let mut player = CutawayPlayer::at(3.5, 2.5, 0.0);
    player.forward = [0.0, 1.0];
    let st = Cutaways::new().update(&w, &player);
    assert_eq!(st.cutaway_room_ids, vec![1]);
    for x in 0..8 {
        assert_eq!(st.cutaway_flags(x, 5, 0) & CUTAWAY_N, CUTAWAY_N, "x {x}");
    }
    // From the south room the wall hides nothing of the player's room.
    let st = Cutaways::new().update(&w, &CutawayPlayer::at(3.5, 7.5, 0.0));
    assert_eq!(st.cutaway_room_ids, vec![2]);
    assert_eq!(st.cutaway_flags(3, 5, 0), 0);
}

#[test]
fn only_the_seen_part_of_a_room_is_cut() {
    let mut w = two_rooms();
    // Squares of room 1 the player could not see.
    for x in 3..8 {
        for y in 0..5 {
            w.square_mut(x, y, 0).cut.could_see = false;
        }
    }
    let mut player = CutawayPlayer::at(1.5, 2.5, 0.0);
    player.forward = [0.0, 1.0];
    let st = Cutaways::new().update(&w, &player);
    assert_ne!(st.cutaway_flags(2, 5, 0), 0);
    // Wall square 6 only hides (3..6, 2..4) of room 1: all unseen.
    assert_eq!(st.cutaway_flags(6, 5, 0), 0);
}

#[test]
fn walls_of_a_collapsed_building_are_cut_when_outside() {
    let mut w = GridWorld::default();
    ground(&mut w, 0, 0, 24, 24);
    building(&mut w, 7, 10, 10, 16, 16);
    for x in 10..16 {
        w.square_mut(x, 13, 0).objects.push(north_wall());
    }
    let st = Cutaways::new().update(&w, &CutawayPlayer::at(8.5, 8.5, 0.0));
    assert!(st.cutaway_room_ids.is_empty());
    assert_eq!(st.cutaway_flags(12, 13, 0), CUTAWAY_N);
}

#[test]
fn floors_above_occlude_the_squares_they_cover() {
    let mut w = GridWorld::default();
    ground(&mut w, 0, 0, 24, 24);
    // An unowned platform at level 1 over (8..16, 8..16).
    for y in 8..16 {
        for x in 8..16 {
            w.insert(square(x, y, 1, None));
        }
    }
    let mut player = CutawayPlayer::at(1.5, 1.5, 0.0);
    player.occlusion_bounds = Some([-20, -20, 40, 40]);
    let st = Cutaways::new().update(&w, &player);
    assert!(st.buildings_to_collapse.is_empty());
    // Level-1 square (x, y) covers level-0 square (x - 3, y - 3).
    assert!(st.is_occluded(9, 9, 0));
    assert!(st.should_render_square(9, 9, 0));
    assert!(!st.is_visible(9, 9, 0));
    assert!(st.is_visible(9, 9, 1));
    assert!(st.is_visible(3, 3, 0));
    let mut no_occlusion = Cutaways { occlusion_enabled: false };
    assert!(no_occlusion.update(&w, &player).is_visible(9, 9, 0));
}

#[test]
fn cut_doors_and_windows_fade() {
    let mut w = two_rooms();
    let door = RenderObject::new(
        ObjectKind::Door { open: false },
        Some(sprite(|s| {
            s.tile_type = TileType::DoorN;
            s.cut_n = true;
        })),
    );
    w.square_mut(2, 5, 0).objects.push(door);
    let window = RenderObject::new(ObjectKind::Window { north: true }, Some(sprite(|s| s.flags.window_n = true)));
    w.square_mut(4, 5, 0).objects.push(window);
    let table = RenderObject::new(ObjectKind::Plain, Some(sprite(|_| {})));
    w.square_mut(5, 5, 0).objects.push(table);
    let mut player = CutawayPlayer::at(3.5, 2.5, 0.0);
    player.forward = [0.0, 1.0];
    let st = Cutaways::new().update(&w, &player);
    let cv = CutawayView { state: &st, world: &w };
    let sq = w.squares.get(&(2, 5, 0)).unwrap();
    let door = sq.objects.last().unwrap();
    assert_eq!(cv.target_alpha(sq, door, Layer::MinusFloor), 0.4);
    assert_eq!(cv.render_layer(sq, door), Layer::Translucent);
    let c = cv.wall_cutaway(sq, door);
    assert!(c.has_seen_door_n);
    assert_eq!(cv.wall_draw(sq, door, WallSide::N, &c), WallDraw::Normal { no_wall_lighting: true });
    // The plain wall of the same square is cut with the cutaway shader.
    let wall = &sq.objects[1];
    assert_eq!(cv.target_alpha(sq, wall, Layer::MinusFloor), 1.0);
    assert_eq!(cv.wall_draw(sq, wall, WallSide::N, &c), WallDraw::Cutaway);
    // The window fades with the player's squared distance to its centre:
    // (3.5, 2.5) to (4.5, 5.5) is 1 + 9.
    let sq = w.squares.get(&(4, 5, 0)).unwrap();
    let a = cv.target_alpha(sq, sq.objects.last().unwrap(), Layer::Translucent);
    assert!((a - (0.1 + 0.65 * (1.0 - 10.0 / 25.0))).abs() < 1e-6, "{a}");
    // Furniture on a cut square, outside the player's room: 0.25.
    let sq = w.squares.get(&(5, 5, 0)).unwrap();
    assert_eq!(cv.target_alpha(sq, sq.objects.last().unwrap(), Layer::MinusFloor), 0.25);
    // An open door is 0.6 regardless.
    let open = RenderObject::new(ObjectKind::Door { open: true }, Some(sprite(|s| s.tile_type = TileType::DoorW)));
    assert_eq!(cv.target_alpha(sq, &open, Layer::MinusFloor), 0.6);
}

#[test]
fn edge_wall_changes_dirty_the_neighbour_chunk() {
    let mut w = GridWorld::default();
    ground(&mut w, 0, 0, 24, 16);
    // A west fence on the north edge row of chunk (1, 1): y = 8.
    w.square_mut(12, 8, 0).objects.push(west_wall());
    let mut c = Cutaways::new();
    let near = c.update(&w, &CutawayPlayer::at(9.5, 8.5, 0.0));
    let far = c.update(&w, &CutawayPlayer::at(20.5, 8.5, 0.0));
    assert_eq!(near.cutaway_flags(12, 8, 0), CUTAWAY_W);
    assert_eq!(far.cutaway_flags(12, 8, 0), 0);
    let dirty = far.dirty_chunk_levels(&near);
    assert_eq!(dirty, vec![(1, 0, 0), (1, 1, 0)]);
}

/// Depth maps with no textures: sprites fall back to the default shader.
struct NoMaps;

impl pz_iso_render::DepthMaps for NoMaps {
    fn preset(&self, _: pz_iso_render::DepthPreset) -> Option<pz_render::Texture> {
        None
    }
    fn default_texture(&self) -> Option<pz_render::Texture> {
        None
    }
}

/// The draw commands of one recorded level, with or without cutaways.
fn record_level(w: &GridWorld, cv: Option<CutawayView<'_>>, wx: i32, wy: i32, z: i32) -> Vec<pz_render::DrawCmd> {
    let cam = pz_iso_render::Camera::centred(12.0, 12.0, 0.0, 256, 128, 1.0);
    let r = pz_iso_render::ChunkRenderer::new(cam, &NoMaps, (256, 128)).with_cutaways(cv);
    let mut level = [None; 64];
    for (i, slot) in level.iter_mut().enumerate() {
        *slot = w.squares.get(&(wx * 8 + (i % 8) as i32, wy * 8 + (i / 8) as i32, z));
    }
    let mut out = Vec::new();
    r.record(pz_iso_render::ChunkTexture::new(wx, wy, z, 0, 1), &[(z, level)], &mut out);
    out
}

#[test]
fn recording_skips_collapsed_squares() {
    let mut tm = pz_render::TextureManager::new();
    let tex = tm.texture_from_pixels("floor", 128, 64, [255u8; 4].repeat(128 * 64), 0);
    let mut w = GridWorld::default();
    ground(&mut w, 0, 0, 24, 24);
    building(&mut w, 7, 8, 8, 16, 16);
    for sq in w.squares.values_mut() {
        for o in &mut sq.objects {
            let mut s = (**o.sprite.as_ref().unwrap()).clone();
            s.texture = Some(tex.clone());
            o.sprite = Some(Rc::new(s));
        }
    }
    let st = Cutaways::new().update(&w, &CutawayPlayer::at(6.5, 6.5, 0.0));
    assert_eq!(st.buildings_to_collapse, vec![BuildingId(7)]);
    let cv = CutawayView { state: &st, world: &w };
    let plain = record_level(&w, None, 1, 1, 1);
    let cut = record_level(&w, Some(cv), 1, 1, 1);
    assert!(cut.len() < plain.len(), "{} {}", cut.len(), plain.len());
    // The ground floor is unaffected.
    assert_eq!(record_level(&w, Some(cv), 1, 1, 0).len(), record_level(&w, None, 1, 1, 0).len());
}

#[test]
fn trees_on_hidden_squares_are_not_drawn() {
    let mut tm = pz_render::TextureManager::new();
    let tex = tm.texture_from_pixels("tree", 128, 256, [255u8; 4].repeat(128 * 256), 0);
    let mut w = GridWorld::default();
    ground(&mut w, 0, 0, 24, 24);
    // An unowned platform at level 1 hides (9, 9, 0) from the camera.
    for y in 8..16 {
        for x in 8..16 {
            w.insert(square(x, y, 1, None));
        }
    }
    let tree = RenderObject::new(ObjectKind::Tree, Some(sprite(|s| s.texture = Some(tex.clone()))));
    w.square_mut(9, 9, 0).objects.push(tree);
    let mut player = CutawayPlayer::at(1.5, 1.5, 0.0);
    player.occlusion_bounds = Some([-20, -20, 40, 40]);
    let st = Cutaways::new().update(&w, &player);
    assert!(!st.is_visible(9, 9, 0));
    let squares: Vec<&RenderSquare> = w.squares.values().collect();
    let cam = pz_iso_render::Camera::centred(12.0, 12.0, 0.0, 256, 128, 1.0);
    let pass = |cv: Option<CutawayView<'_>>| {
        let r = pz_iso_render::ChunkRenderer::new(cam, &NoMaps, (256, 128)).with_cutaways(cv);
        let mut out = Vec::new();
        r.render_translucent(&squares, 0, &mut out);
        out.len()
    };
    let plain = pass(None);
    let cut = pass(Some(CutawayView { state: &st, world: &w }));
    assert!(cut < plain, "{cut} {plain}");
}
