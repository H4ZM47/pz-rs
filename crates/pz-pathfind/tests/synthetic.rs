//! Behavioural tests on hand-built chunks (no game data).

use pz_pathfind::square::bits::*;
use pz_pathfind::{MoverKind, PathFindRequest, PathNode, World};

/// Builds `ChunkUpdateTask`-style buffers for a block of chunks.
struct Map {
    /// (x, y, z) -> (bits, cost); z without the +32 offset.
    squares: std::collections::HashMap<(i32, i32, i32), (u64, i16)>,
}

impl Map {
    fn new() -> Self {
        Map {
            squares: Default::default(),
        }
    }

    fn floor(&mut self, x0: i32, y0: i32, x1: i32, y1: i32, z: i32) {
        for y in y0..=y1 {
            for x in x0..=x1 {
                self.squares.entry((x, y, z)).or_insert((0, 0)).0 |= SOLID_FLOOR;
            }
        }
    }

    fn add(&mut self, x: i32, y: i32, z: i32, bits: u64) {
        self.squares.entry((x, y, z)).or_insert((0, 0)).0 |= bits;
    }

    fn chunk_bytes(&self, wx: i32, wy: i32) -> Vec<u8> {
        let mut levels: Vec<i32> = self
            .squares
            .keys()
            .filter(|(x, y, _)| x.div_euclid(8) == wx && y.div_euclid(8) == wy)
            .map(|k| k.2)
            .collect();
        levels.push(0);
        let min = *levels.iter().min().unwrap();
        let max = *levels.iter().max().unwrap();
        let mut b = Vec::new();
        b.extend_from_slice(&(min + 32).to_be_bytes());
        b.extend_from_slice(&(max + 32).to_be_bytes());
        for z in min..=max {
            for y in 0..8 {
                for x in 0..8 {
                    let (bits, cost) = self
                        .squares
                        .get(&(wx * 8 + x, wy * 8 + y, z))
                        .copied()
                        .unwrap_or((0, 0));
                    b.extend_from_slice(&bits.to_be_bytes());
                    b.extend_from_slice(&cost.to_be_bytes());
                }
            }
        }
        b.extend_from_slice(&0i16.to_be_bytes());
        b
    }

    /// Load chunks `0..n` x `0..n` into a fresh world.
    fn world(&self, n: i32) -> World {
        let mut w = World::new(0, 0, 1, 1, false);
        for wy in 0..n {
            for wx in 0..n {
                w.update_chunk(1, wx, wy, &self.chunk_bytes(wx, wy))
                    .unwrap();
            }
        }
        w.update();
        w
    }
}

fn request(mover: MoverKind, from: (f32, f32, f32), to: (f32, f32, f32)) -> PathFindRequest {
    PathFindRequest::new(mover, from, to)
}

fn passes_through(path: &[PathNode], x: i32, y: i32) -> bool {
    // A straight segment between consecutive nodes crosses the square.
    path.windows(2).any(|w| {
        (0..=100).any(|i| {
            let t = i as f32 / 100.0;
            let px = w[0].x + (w[1].x - w[0].x) * t;
            let py = w[0].y + (w[1].y - w[0].y) * t;
            px.floor() as i32 == x && py.floor() as i32 == y
        })
    })
}

#[test]
fn start_equals_target() {
    let mut m = Map::new();
    m.floor(0, 0, 23, 23, 0);
    let mut w = m.world(3);
    let p = w
        .find_path(&request(
            MoverKind::Zombie,
            (10.5, 10.5, 0.0),
            (10.5, 10.5, 0.0),
        ))
        .expect("path");
    assert!(p.len() >= 2);
    assert_eq!((p[0].x, p[0].y, p[0].z), (10.5, 10.5, 32.0));
    assert_eq!((p[p.len() - 1].x, p[p.len() - 1].y), (10.5, 10.5));
}

#[test]
fn straight_line_in_open_space() {
    let mut m = Map::new();
    m.floor(0, 0, 23, 23, 0);
    let mut w = m.world(3);
    let p = w
        .find_path(&request(
            MoverKind::Player,
            (2.5, 3.5, 0.0),
            (20.25, 18.75, 0.0),
        ))
        .expect("path");
    assert_eq!(p.len(), 2, "{p:?}");
    assert_eq!((p[1].x, p[1].y, p[1].level()), (20.25, 18.75, 0.0));
}

/// A north-south wall on the west edge of column x=12, y=2..=21, with a
/// door at y=12.
fn wall_with_door(door: bool) -> Map {
    let mut m = Map::new();
    m.floor(0, 0, 23, 23, 0);
    for y in 2..=21 {
        m.add(12, y, 0, COLLIDE_W);
    }
    if door {
        m.add(12, 12, 0, CAN_PATH_W | DOOR_W_UNBLOCKED);
    }
    m
}

#[test]
fn path_through_door() {
    let mut w = wall_with_door(true).world(3);
    for mover in [MoverKind::Player, MoverKind::Zombie] {
        let p = w
            .find_path(&request(mover, (6.5, 12.5, 0.0), (18.5, 12.5, 0.0)))
            .expect("path through the door");
        assert!(
            passes_through(&p, 12, 12) || passes_through(&p, 11, 12),
            "{p:?}"
        );
        // Must not cut through the wall anywhere else.
        for win in p.windows(2) {
            let crosses = (win[0].x < 12.0) != (win[1].x < 12.0);
            if crosses {
                let t = (12.0 - win[0].x) / (win[1].x - win[0].x);
                let y = win[0].y + (win[1].y - win[0].y) * t;
                assert!(
                    (12.0..13.0).contains(&y),
                    "crossed the wall at y={y}: {p:?}"
                );
            }
        }
    }
}

#[test]
fn wall_without_door_goes_around() {
    let mut w = wall_with_door(false).world(3);
    let p = w
        .find_path(&request(
            MoverKind::Player,
            (6.5, 12.5, 0.0),
            (18.5, 12.5, 0.0),
        ))
        .expect("path around the wall");
    assert!(p.iter().any(|n| n.y < 2.5 || n.y > 21.5), "{p:?}");
}

/// A 6x6 room (x 8..=13, y 8..=13) enclosed by walls; optionally a window
/// in the west wall at y=10.
fn room(window: bool) -> Map {
    let mut m = Map::new();
    m.floor(0, 0, 23, 23, 0);
    for i in 8..=13 {
        m.add(8, i, 0, COLLIDE_W); // west wall
        m.add(14, i, 0, COLLIDE_W); // east wall (west edge of x=14)
        m.add(i, 8, 0, COLLIDE_N); // north wall
        m.add(i, 14, 0, COLLIDE_N); // south wall
    }
    if window {
        m.add(8, 10, 0, WINDOW_W | WINDOW_W_UNBLOCKED | CAN_PATH_W);
    }
    m
}

#[test]
fn no_path_into_sealed_room() {
    let mut w = room(false).world(3);
    for mover in [MoverKind::Player, MoverKind::Zombie] {
        assert!(w
            .find_path(&request(mover, (2.5, 2.5, 0.0), (10.5, 10.5, 0.0)))
            .is_none());
    }
}

#[test]
fn path_through_window() {
    let mut w = room(true).world(3);
    let p = w
        .find_path(&request(
            MoverKind::Zombie,
            (2.5, 10.5, 0.0),
            (11.5, 10.5, 0.0),
        ))
        .expect("path through the window");
    assert!(
        passes_through(&p, 8, 10) || passes_through(&p, 7, 10),
        "{p:?}"
    );
}

#[test]
fn stairs_change_level() {
    let mut m = Map::new();
    m.floor(0, 0, 23, 23, 0);
    // Upper floor west of the stairs.
    m.floor(2, 8, 9, 15, 1);
    // West-facing staircase on y=10: top at x=10, middle 11, bottom 12;
    // the top step leads to (9, 10) on level 1.
    m.add(10, 10, 0, STAIR_TW);
    m.add(11, 10, 0, STAIR_MW);
    m.add(12, 10, 0, STAIR_BW);
    let mut w = m.world(3);
    let p = w
        .find_path(&request(
            MoverKind::Player,
            (18.5, 10.5, 0.0),
            (5.5, 12.5, 1.0),
        ))
        .expect("path up the stairs");
    assert_eq!(p[0].level(), 0.0);
    assert_eq!(p.last().unwrap().level(), 1.0);
    assert!(
        p.iter()
            .any(|n| (n.x - 9.5).abs() < 0.6 && n.level() == 1.0),
        "{p:?}"
    );
    // And back down.
    let p = w
        .find_path(&request(
            MoverKind::Zombie,
            (5.5, 12.5, 1.0),
            (18.5, 10.5, 0.0),
        ))
        .expect("path down the stairs");
    assert_eq!(p.last().unwrap().level(), 0.0);
}

#[test]
fn byte_api_matches_typed_api() {
    let mut w = wall_with_door(true).world(3);
    let r = request(MoverKind::Zombie, (6.5, 12.5, 0.0), (18.5, 12.5, 0.0));
    let typed = w.find_path(&r).unwrap();
    let mut out = Vec::new();
    let code = w.find_path_into(&r.to_bytes(), &mut out, 3072).unwrap();
    assert_eq!(code, 1);
    assert_eq!(i16::from_be_bytes([out[0], out[1]]) as usize, typed.len());
    assert_eq!(out.len(), 2 + 16 * typed.len());
}

#[test]
fn unloaded_target_and_removed_chunk() {
    let mut m = Map::new();
    m.floor(0, 0, 23, 23, 0);
    let mut w = m.world(3);
    w.remove_chunk(2, 2).unwrap();
    // Target in the removed chunk: the search heads for the loaded edge.
    let p = w.find_path(&request(
        MoverKind::Zombie,
        (2.5, 2.5, 0.0),
        (20.5, 20.5, 0.0),
    ));
    assert!(p.is_some());
}

fn car(id: i32, x0: f32, y0: f32, x1: f32, y1: f32) -> pz_pathfind::Vehicle {
    let poly = |r: f32| pz_pathfind::VehiclePoly {
        x1: x1 + r,
        y1: y0 - r,
        x2: x0 - r,
        y2: y0 - r,
        x3: x0 - r,
        y3: y1 + r,
        x4: x1 + r,
        y4: y1 + r,
        z: 32.0,
    };
    pz_pathfind::Vehicle {
        id,
        poly: poly(0.0),
        poly_plus_radius: poly(0.3),
        up_vector_dot: 1.0,
        crawl_offsets: vec![0.25, 0.5, 0.75],
    }
}

/// Whether any path segment passes through the open rectangle.
fn crosses_rect(path: &[PathNode], x0: f32, y0: f32, x1: f32, y1: f32) -> bool {
    path.windows(2).any(|w| {
        (0..=200).any(|i| {
            let t = i as f32 / 200.0;
            let px = w[0].x + (w[1].x - w[0].x) * t;
            let py = w[0].y + (w[1].y - w[0].y) * t;
            px > x0 && px < x1 && py > y0 && py < y1
        })
    })
}

#[test]
fn vehicle_bytes_round_trip() {
    let v = car(7, 10.0, 8.0, 12.0, 14.0);
    let back = pz_pathfind::Vehicle::from_bytes(&v.to_bytes()).unwrap();
    assert_eq!(back.id, 7);
    assert_eq!(back.poly, v.poly);
    assert_eq!(back.poly_plus_radius, v.poly_plus_radius);
    assert_eq!(back.crawl_offsets, v.crawl_offsets);
}

#[test]
fn path_goes_around_vehicle() {
    let mut m = Map::new();
    m.floor(0, 0, 23, 23, 0);
    let mut w = m.world(3);
    let r = request(MoverKind::Player, (5.5, 11.5, 0.0), (17.5, 11.5, 0.0));
    assert_eq!(w.find_path(&r).unwrap().len(), 2);

    w.add_vehicle(&car(1, 10.0, 8.0, 12.0, 14.0).to_bytes())
        .unwrap();
    assert_eq!(w.vehicle_count(), 1);
    for mover in [MoverKind::Player, MoverKind::Zombie] {
        let p = w
            .find_path(&request(mover, (5.5, 11.5, 0.0), (17.5, 11.5, 0.0)))
            .expect("path around the car");
        assert!(p.len() > 2, "{p:?}");
        assert!(!crosses_rect(&p, 10.0, 8.0, 12.0, 14.0), "{p:?}");
        let last = p.last().unwrap();
        assert_eq!((last.x, last.y), (17.5, 11.5));
    }

    // Moving the car out of the way restores the straight line.
    w.teleport_vehicle(&car(1, 10.0, 18.0, 12.0, 22.0).to_bytes())
        .unwrap();
    assert_eq!(w.find_path(&r).unwrap().len(), 2);
    w.teleport_vehicle(&car(1, 10.0, 8.0, 12.0, 14.0).to_bytes())
        .unwrap();
    assert!(w.find_path(&r).unwrap().len() > 2);
    w.remove_vehicle(1);
    assert_eq!(w.vehicle_count(), 0);
    assert_eq!(w.find_path(&r).unwrap().len(), 2);
}

#[test]
fn start_under_vehicle_and_two_cars() {
    let mut m = Map::new();
    m.floor(0, 0, 23, 23, 0);
    let mut w = m.world(3);
    w.add_vehicle(&car(1, 10.0, 8.0, 12.0, 14.0).to_bytes())
        .unwrap();
    w.add_vehicle(&car(2, 12.5, 6.0, 14.5, 12.0).to_bytes())
        .unwrap();
    // Start inside the first car's footprint: moved onto its outline.
    let p = w
        .find_path(&request(
            MoverKind::Zombie,
            (11.0, 11.0, 0.0),
            (3.5, 3.5, 0.0),
        ))
        .expect("path from under the car");
    assert!(p.len() >= 2);
    let p = w
        .find_path(&request(
            MoverKind::Player,
            (16.5, 9.5, 0.0),
            (6.5, 9.5, 0.0),
        ))
        .expect("path around both cars");
    assert!(!crosses_rect(&p, 10.0, 8.0, 12.0, 14.0), "{p:?}");
    assert!(!crosses_rect(&p, 12.5, 6.0, 14.5, 12.0), "{p:?}");
    // Repeated requests leave the graphs as they were.
    let again = w
        .find_path(&request(
            MoverKind::Player,
            (16.5, 9.5, 0.0),
            (6.5, 9.5, 0.0),
        ))
        .unwrap();
    assert_eq!(p, again);
}

#[test]
fn vehicle_next_to_wall_and_door() {
    // The car's graph also takes in the wall as a clipped world obstacle.
    let mut w = wall_with_door(true).world(3);
    w.add_vehicle(&car(3, 13.5, 13.5, 15.5, 18.0).to_bytes())
        .unwrap();
    for mover in [MoverKind::Player, MoverKind::Zombie] {
        let p = w
            .find_path(&request(mover, (6.5, 12.5, 0.0), (14.5, 19.5, 0.0)))
            .expect("path through the door past the car");
        assert!(!crosses_rect(&p, 13.5, 13.5, 15.5, 18.0), "{p:?}");
        for win in p.windows(2) {
            if (win[0].x < 12.0) != (win[1].x < 12.0) {
                let t = (12.0 - win[0].x) / (win[1].x - win[0].x);
                let y = win[0].y + (win[1].y - win[0].y) * t;
                assert!(
                    (12.0..13.0).contains(&y),
                    "crossed the wall at y={y}: {p:?}"
                );
            }
        }
    }
}
