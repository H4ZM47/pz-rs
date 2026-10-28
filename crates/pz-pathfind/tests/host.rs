//! `PathHost`: one behaviour per character, results routed by finder,
//! walking on the spot, cancel, and the collision queries, with the fake
//! characters and squares of `common`.

mod common;

use std::collections::HashMap;

use common::*;
use pz_pathfind::host::{CharacterId, MoverAccess, PathEnv, PathHost};
use pz_pathfind::main_thread::TestResults;

/// The world's characters by id.
#[derive(Default)]
struct Movers(HashMap<CharacterId, Chr>);

impl MoverAccess for Movers {
    fn with_mover(&mut self, id: CharacterId, f: &mut dyn FnMut(&mut dyn PathMover)) {
        if let Some(c) = self.0.get_mut(&id) {
            f(c);
        }
    }
}

fn chr(id: CharacterId, x: f32, y: f32) -> Chr {
    Chr {
        id: id as u64 + 100,
        ..player(x, y)
    }
}

/// Runs `f` with the map as the behaviours see it.
fn with_env<R>(map: &Map, f: impl FnOnce(&PathEnv<'_, '_, '_, IsoCell>) -> R) -> R {
    let bits = SquareBits::new(&map.m, &map.meta, &map.cell, &DefaultBitsHooks).unwrap();
    let mtm = MainThreadMap::new(&bits, &NoVehicles);
    let env = PathEnv {
        map: &mtm,
        world: &Game,
        vehicles: &NoVehicleScripts,
        seating: &NoSeating,
        characters: &NoCharacters,
    };
    f(&env)
}

/// The worker until it is idle, then `updateMain`.
fn finish_requests(host: &mut PathHost, movers: &mut Movers) {
    while host.service().has_thread_work() {
        host.update_thread();
    }
    host.update_main(&NoVehicles, &NoSeating, movers);
}

/// Frames of `path_update` for every character in `ids` until none is
/// working; the results.
fn run(
    map: &Map,
    host: &mut PathHost,
    movers: &mut Movers,
    ids: &[CharacterId],
    frames: usize,
) -> HashMap<CharacterId, BehaviorResult> {
    let mut done = HashMap::new();
    for _ in 0..frames {
        for &id in ids {
            if done.contains_key(&id) {
                continue;
            }
            let c = movers.0.get_mut(&id).unwrap();
            let r = with_env(map, |env| host.path_update(id, c, env));
            c.anim_moving = c.moving;
            if r != BehaviorResult::Working {
                done.insert(id, r);
            }
        }
        if done.len() == ids.len() {
            break;
        }
        host.update_thread();
        host.update_main(&NoVehicles, &NoSeating, movers);
    }
    done
}

#[test]
fn one_behaviour_per_character() {
    let mut map = Map::new();
    let mut host = PathHost::new(map.service());
    let (mut a, mut b) = (chr(1, 2.5, 2.5), chr(2, 2.5, 8.5));
    assert!(host.behavior(1).is_none());
    host.path_to_location(1, &mut a, 10, 2, 0);
    host.path_to_sound(2, &mut b, 4, 12, 0);
    let (pa, pb) = (host.behavior(1).unwrap(), host.behavior(2).unwrap());
    assert_ne!(pa.finder, pb.finder);
    assert!(pa.is_goal_location() && pb.is_goal_sound());
    assert_eq!((pa.target_x(), pa.target_y()), (10.5, 2.5));
    assert_eq!((pb.target_x(), pb.target_y()), (4.5, 12.5));
    assert_eq!(host.character_of(pa.finder), Some(1));
    assert_eq!(host.character_of(pb.finder), Some(2));
    assert!(host.behavior(3).is_none());
    // The same behaviour on the next call.
    let finder = host.behavior(1).unwrap().finder;
    host.path_to_location_f(1, &mut a, 5.25, 6.75, 0.0);
    let pa = host.behavior(1).unwrap();
    assert_eq!(pa.finder, finder);
    assert_eq!((pa.target_x(), pa.target_y()), (5.25, 6.75));
}

#[test]
fn results_go_to_the_character_that_asked() {
    let mut map = Map::new();
    let mut host = PathHost::new(map.service());
    let mut movers = Movers::default();
    movers.0.insert(1, chr(1, 2.5, 2.5));
    movers.0.insert(2, chr(2, 2.5, 8.5));
    host.path_to_location(1, movers.0.get_mut(&1).unwrap(), 10, 2, 0);
    host.path_to_location(2, movers.0.get_mut(&2).unwrap(), 6, 14, 0);
    // First frame: both ask.
    for id in [1, 2] {
        let c = movers.0.get_mut(&id).unwrap();
        let r = with_env(&map, |env| host.path_update(id, c, env));
        assert_eq!(r, BehaviorResult::Working);
        assert_eq!(c.progress, PathFindProgress::NotYetFound);
    }
    finish_requests(&mut host, &mut movers);
    for (id, end) in [(1, (10.5, 2.5)), (2, (6.5, 14.5))] {
        assert_eq!(movers.0[&id].progress, PathFindProgress::Found);
        let last = *host.behavior(id).unwrap().path().nodes.last().unwrap();
        assert_eq!((last.x, last.y), end, "character {id}");
    }
    let r = run(&map, &mut host, &mut movers, &[1, 2], 500);
    assert_eq!(r[&1], BehaviorResult::Succeeded);
    assert_eq!(r[&2], BehaviorResult::Succeeded);
    let (a, b) = (&movers.0[&1], &movers.0[&2]);
    assert!(
        (a.x - 10.5).abs() < 0.06 && (a.y - 2.5).abs() < 0.06,
        "{} {}",
        a.x,
        a.y
    );
    assert!(
        (b.x - 6.5).abs() < 0.06 && (b.y - 14.5).abs() < 0.06,
        "{} {}",
        b.x,
        b.y
    );
}

#[test]
fn a_failed_request_fails_only_its_character() {
    let mut map = Map::new();
    let mut host = PathHost::new(map.service());
    let mut movers = Movers::default();
    movers.0.insert(1, chr(1, 2.5, 2.5));
    movers.0.insert(2, chr(2, 4.5, 4.5));
    // No squares on level 5.
    host.path_to_location(1, movers.0.get_mut(&1).unwrap(), 10, 10, 5);
    host.path_to_location(2, movers.0.get_mut(&2).unwrap(), 8, 4, 0);
    let r = run(&map, &mut host, &mut movers, &[1, 2], 500);
    assert_eq!(r[&1], BehaviorResult::Failed);
    assert_eq!(movers.0[&1].progress, PathFindProgress::Failed);
    assert_eq!(r[&2], BehaviorResult::Succeeded);
}

#[test]
fn a_result_waits_for_a_character_the_world_cannot_lend() {
    let mut map = Map::new();
    let mut host = PathHost::new(map.service());
    let mut movers = Movers::default();
    movers.0.insert(1, chr(1, 2.5, 2.5));
    let mut away = chr(2, 2.5, 8.5);
    host.path_to_location(1, movers.0.get_mut(&1).unwrap(), 10, 2, 0);
    host.path_to_location(2, &mut away, 6, 8, 0);
    for id in [1, 2] {
        let c = movers.0.get_mut(&id).unwrap_or(&mut away);
        with_env(&map, |env| host.path_update(id, c, env));
    }
    // Character 2 is out of the world (borrowed) during updateMain.
    finish_requests(&mut host, &mut movers);
    assert_eq!(movers.0[&1].progress, PathFindProgress::Found);
    assert!(!host.has_pending(1));
    assert!(host.has_pending(2));
    assert_eq!(away.progress, PathFindProgress::NotYetFound);
    // Its next call hands the result over first.
    let r = with_env(&map, |env| host.path_update(2, &mut away, env));
    assert_eq!(r, BehaviorResult::Working);
    assert!(!host.has_pending(2));
    assert_eq!(away.progress, PathFindProgress::Found);
    assert!(away.moving);
    let last = *host.behavior(2).unwrap().path().nodes.last().unwrap();
    assert_eq!((last.x, last.y), (6.5, 8.5));
}

#[test]
fn walking_on_the_spot_per_character() {
    let mut map = Map::new();
    let mut host = PathHost::new(map.service());
    host.walking_on_the_spot_reset(1, 3.0, 3.0);
    host.walking_on_the_spot_reset(2, 3.0, 3.0);
    // 400 multiplier units at one spot are allowed, the 401st is not.
    for _ in 0..400 {
        assert!(!host.walking_on_the_spot_check(1, 3.0, 3.0, &Game));
    }
    assert!(host.walking_on_the_spot_check(1, 3.05, 3.0, &Game));
    // Character 2 has not been checked; moving resets it.
    assert!(!host.walking_on_the_spot_check(2, 3.0, 3.0, &Game));
    assert!(!host.walking_on_the_spot_check(1, 4.0, 3.0, &Game));
    assert_eq!(host.behavior(1).unwrap().walking_on_the_spot.seconds(), 0.0);
    // A new path resets it to the character's position.
    for _ in 0..10 {
        host.walking_on_the_spot_check(2, 3.0, 3.0, &Game);
    }
    let mut c = chr(2, 5.5, 5.5);
    host.path_to_location(2, &mut c, 8, 8, 0);
    let r = with_env(&map, |env| host.path_update(2, &mut c, env));
    assert_eq!(r, BehaviorResult::Working);
    assert_eq!(host.behavior(2).unwrap().walking_on_the_spot.seconds(), 0.0);
}

#[test]
fn cancel_sets_the_flag_and_a_new_goal_clears_it() {
    let mut map = Map::new();
    let mut host = PathHost::new(map.service());
    let mut movers = Movers::default();
    movers.0.insert(1, chr(1, 2.5, 2.5));
    host.path_to_location(1, movers.0.get_mut(&1).unwrap(), 10, 2, 0);
    assert!(!host.behavior(1).unwrap().is_cancelled());
    with_env(&map, |env| {
        host.path_update(1, movers.0.get_mut(&1).unwrap(), env)
    });
    host.path_cancel(1, movers.0.get_mut(&1).unwrap());
    let b = host.behavior(1).unwrap();
    assert!(b.is_cancelled() && !b.is_moving_using_path_find());
    // As in Java the running request is not withdrawn: its result still
    // arrives.
    finish_requests(&mut host, &mut movers);
    assert_eq!(movers.0[&1].progress, PathFindProgress::Found);
    host.path_to_location(1, movers.0.get_mut(&1).unwrap(), 4, 4, 0);
    let b = host.behavior(1).unwrap();
    assert!(!b.is_cancelled() && b.is_moving_using_path_find());
    assert_eq!(movers.0[&1].progress, PathFindProgress::NotRunning);
}

#[test]
fn a_removed_character_gets_nothing() {
    let mut map = Map::new();
    let mut host = PathHost::new(map.service());
    let mut movers = Movers::default();
    movers.0.insert(1, chr(1, 2.5, 2.5));
    host.path_to_location(1, movers.0.get_mut(&1).unwrap(), 10, 2, 0);
    with_env(&map, |env| {
        host.path_update(1, movers.0.get_mut(&1).unwrap(), env)
    });
    let finder = host.behavior(1).unwrap().finder;
    let mover = movers.0[&1].mover_id();
    assert!(host.service().request_for(mover).is_some());
    host.remove(1);
    assert!(host.behavior(1).is_none());
    assert_eq!(host.character_of(finder), None);
    assert!(host.service().request_for(mover).is_none());
    finish_requests(&mut host, &mut movers);
    assert_eq!(movers.0[&1].progress, PathFindProgress::NotYetFound);
    // Back in the world it starts over with a new finder.
    host.path_to_location(1, movers.0.get_mut(&1).unwrap(), 10, 2, 0);
    assert_ne!(host.behavior(1).unwrap().finder, finder);
}

#[test]
fn collision_queries() {
    let mut map = Map::new();
    map.put(6, 6, &["floor", "crate"]);
    map.put(10, 10, &["floor", "fence_n"]);
    map.recalc();
    let bits = SquareBits::new(&map.m, &map.meta, &map.cell, &DefaultBitsHooks).unwrap();
    let mtm = MainThreadMap::new(&bits, &NoVehicles);
    assert!(PathHost::can_stand_at(&mtm, 3.5, 3.5, 0));
    assert!(!PathHost::can_stand_at(&mtm, 6.5, 6.5, 0));
    // lineClearCollide: true when blocked.
    assert!(!PathHost::line_clear_collide(
        &mtm, 2.5, 2.5, 4.5, 2.5, 0, None, 3
    ));
    assert!(PathHost::line_clear_collide(
        &mtm, 4.5, 6.5, 8.5, 6.5, 0, None, 3
    ));
    assert!(PathHost::line_clear_collide_default(
        &mtm, 4.5, 6.5, 8.5, 6.5, 0, None
    ));
    // The fence on the north edge of (10, 10) stops movement.
    assert!(PathHost::los_line_clear_collide(
        &mtm,
        (10, 12, 0),
        (10, 8, 0),
        false
    ));
    assert!(!PathHost::los_line_clear_collide(
        &mtm,
        (12, 12, 0),
        (12, 8, 0),
        false
    ));
    assert!(!PathHost::los_line_clear_collide(
        &mtm,
        (13, 9, 0),
        (8, 9, 0),
        false
    ));
    // lineClear on the floor squares, and the melee test built on it.
    assert_eq!(
        PathHost::los_line_clear(&mtm, (12, 12, 0), (12, 8, 0), false),
        TestResults::Clear
    );
    let across = PathHost::los_line_clear(&mtm, (10, 12, 0), (10, 8, 0), false);
    assert_eq!(
        PathHost::line_clear(&mtm, 10.5, 12.5, 10.5, 8.5, 0),
        !matches!(
            across,
            TestResults::Blocked | TestResults::ClearThroughClosedDoor
        )
    );
    assert!(PathHost::line_clear(&mtm, 2.5, 2.5, 20.5, 3.5, 0));
}

/// A vehicle standing still (`getPolyPlusRadius()` as given).
struct Parked(pz_pathfind::visibility::VehiclePoly);

impl pz_pathfind::game_vehicle::GameVehicle for Parked {
    fn vehicle_id(&self) -> i16 {
        1
    }
    fn poly(&self) -> pz_pathfind::visibility::VehiclePoly {
        self.0
    }
    fn poly_plus_radius(&self) -> pz_pathfind::visibility::VehiclePoly {
        self.0
    }
    fn up_vector_dot(&self) -> f32 {
        1.0
    }
    fn crawl_offsets(&self) -> Vec<f32> {
        Vec::new()
    }
    fn z(&self) -> f32 {
        0.0
    }
    fn added_to_world(&self) -> bool {
        true
    }
    fn has_script(&self) -> bool {
        true
    }
}

/// One vehicle in chunk (0, 0).
struct OneCar(Parked);

impl pz_pathfind::game_vehicle::VehicleSource for OneCar {
    fn for_each_vehicle(&self, f: &mut dyn FnMut(&dyn pz_pathfind::game_vehicle::GameVehicle)) {
        f(&self.0);
    }
    fn for_each_vehicle_in_chunk(
        &self,
        wx: i32,
        wy: i32,
        f: &mut dyn FnMut(&dyn pz_pathfind::game_vehicle::GameVehicle) -> bool,
    ) {
        if (wx, wy) == (0, 0) {
            f(&self.0);
        }
    }
}

#[test]
fn resolve_collision_against_a_vehicle() {
    let mut map = Map::new();
    let mut host = PathHost::new(map.service());
    // Without vehicles nothing is in the way.
    {
        let bits = SquareBits::new(&map.m, &map.meta, &map.cell, &DefaultBitsHooks).unwrap();
        let mtm = MainThreadMap::new(&bits, &NoVehicles);
        let c = chr(1, 4.5, 6.0);
        assert_eq!(host.resolve_collision(&mtm, &c, 5.2, 6.0), (5.2, 6.0));
        assert_eq!(
            host.resolve_collision_radius(&mtm, &c, 0.3),
            (false, (4.5, 6.0))
        );
    }
    // A car over x 5..7, y 4..9, corners in `VehiclePoly.init`'s winding
    // (outward edge normals).
    let car = OneCar(Parked(pz_pathfind::visibility::VehiclePoly {
        x1: 5.0,
        y1: 4.0,
        x2: 5.0,
        y2: 9.0,
        x3: 7.0,
        y3: 9.0,
        x4: 7.0,
        y4: 4.0,
        z: 0.0,
    }));
    let bits = SquareBits::new(&map.m, &map.meta, &map.cell, &DefaultBitsHooks).unwrap();
    let mtm = MainThreadMap::new(&bits, &car);
    // Walking into its west side stops on the side.
    let c = chr(1, 4.5, 6.0);
    assert_eq!(host.resolve_collision(&mtm, &c, 5.2, 6.0), (5.0, 6.0));
    // Walking away, or alongside, is not changed.
    assert_eq!(host.resolve_collision(&mtm, &c, 4.2, 6.1), (4.2, 6.1));
    assert_eq!(host.resolve_collision(&mtm, &c, 4.5, 6.4), (4.5, 6.4));
    // Standing just inside the side, the radius query pushes the character
    // out by the radius.
    let inside = chr(1, 5.1, 6.0);
    let (hit, (x, y)) = host.resolve_collision_radius(&mtm, &inside, 0.3);
    assert!(hit);
    assert_eq!((x, y), (5.0f32 - 0.3, 6.0));
    // Far from the side (squared distance over the radius): no push.
    let deep = chr(1, 6.0, 6.5);
    assert_eq!(
        host.resolve_collision_radius(&mtm, &deep, 0.3),
        (false, (6.0, 6.5))
    );
    // A non-positive radius never collides.
    assert_eq!(
        host.resolve_collision_radius(&mtm, &inside, 0.0),
        (false, (5.1, 6.0))
    );
}
