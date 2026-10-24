//! `PathfindService` (the Java `PathfindNative` / `PathfindNativeThread`
//! layer) with fake movers, vehicles and a fake `IPathfinder`.

use std::cell::RefCell;

use pz_pathfind::game_vehicle::{GameVehicle, VehicleSource};
use pz_pathfind::mover::*;
use pz_pathfind::path::Path;
use pz_pathfind::service::*;
use pz_pathfind::square::bits::SOLID_FLOOR;
use pz_pathfind::visibility::VehiclePoly;
use pz_pathfind::MoverKind;
use pz_world::IsoDirections;

/// A `ChunkUpdateTask` buffer for an all-floor chunk on level 0.
fn floor_chunk() -> Vec<u8> {
    let mut b = Vec::new();
    b.extend_from_slice(&32i32.to_be_bytes());
    b.extend_from_slice(&32i32.to_be_bytes());
    for _ in 0..64 {
        b.extend_from_slice(&SOLID_FLOOR.to_be_bytes());
        b.extend_from_slice(&0i16.to_be_bytes());
    }
    b.extend_from_slice(&0i16.to_be_bytes());
    b
}

/// A service with chunks `0..n` x `0..n` of open floor, loaded.
fn service(n: i32) -> PathfindService {
    let mut s = PathfindService::new(0, 0, 1, 1, false);
    for wy in 0..n {
        for wx in 0..n {
            s.queue_chunk_update(1, wx, wy, floor_chunk());
        }
    }
    while !s.queued_chunk_tasks().is_empty() {
        s.update_thread();
    }
    s
}

fn snapshot(id: u64, class: MoverClass, queue: u8) -> MoverSnapshot {
    MoverSnapshot {
        id: MoverId(id),
        class,
        is_npc: false,
        can_crawl: false,
        crawling: false,
        ignore_crawl_cost: false,
        can_thump: class == MoverClass::Zombie,
        can_climb_fences: false,
        can_climb_tall_fences: false,
        avoid_farming_plants: class == MoverClass::Player,
        has_target: queue == 2,
        can_bend: false,
        queue,
    }
}

#[derive(Default)]
struct Finder {
    log: Vec<(FinderId, MoverId, Option<Path>)>,
}

impl PathfindCallback for Finder {
    fn succeeded(&mut self, finder: FinderId, mover: MoverId, path: &Path) {
        self.log.push((finder, mover, Some(path.clone())));
    }
    fn failed(&mut self, finder: FinderId, mover: MoverId) {
        self.log.push((finder, mover, None));
    }
}

struct NoCars;
impl VehicleSource for NoCars {
    fn for_each_vehicle(&self, _f: &mut dyn FnMut(&dyn GameVehicle)) {}
    fn for_each_vehicle_in_chunk(
        &self,
        _: i32,
        _: i32,
        _: &mut dyn FnMut(&dyn GameVehicle) -> bool,
    ) {
    }
}

fn add(s: &mut PathfindService, id: u64, class: MoverClass, queue: u8) -> RequestId {
    s.add_request(
        FinderId(id),
        snapshot(id, class, queue),
        (2.5, 2.5, 0.0),
        (10.5, 12.5, 0.0),
        &[],
    )
}

#[test]
fn chunk_updates_are_limited_per_pass_but_removals_are_not() {
    let mut s = PathfindService::new(0, 0, 1, 1, false);
    for i in 0..12 {
        s.queue_chunk_update(1, i % 4, i / 4, floor_chunk());
        if i == 3 {
            s.remove_chunk_from_world(3, 3);
        }
    }
    let pass = s.update_thread();
    // 10 updates plus the removal queued among them.
    assert_eq!(pass.chunk_tasks, 11);
    assert_eq!(s.queued_chunk_tasks().len(), 2);
    let pass = s.update_thread();
    assert_eq!(pass.chunk_tasks, 2);
    assert!(!s.has_thread_work());
    assert_eq!(s.world().loaded_chunk_count(), 12);
}

#[test]
fn requests_by_priority_two_per_pass() {
    let mut s = service(3);
    let other = add(&mut s, 1, MoverClass::Zombie, 3);
    let animal = add(&mut s, 2, MoverClass::Animal, 3);
    let aggro = add(&mut s, 3, MoverClass::Zombie, 2);
    let player = add(&mut s, 4, MoverClass::Player, 1);
    let pass = s.update_thread();
    assert_eq!(pass.request_tasks, 4);
    assert_eq!(pass.requests, vec![player, aggro]);
    let pass = s.update_thread();
    assert_eq!(pass.requests, vec![other, animal]);
    assert!(!s.has_thread_work());

    let mut f = Finder::default();
    s.update_main(&NoCars, &mut f);
    let order: Vec<u64> = f.log.iter().map(|e| e.1 .0).collect();
    assert_eq!(order, vec![4, 3, 1, 2]);
    assert!(f.log.iter().all(|e| e.2.is_some()));
    assert!(s.movers_with_requests().is_empty());
}

#[test]
fn cancelled_requests_are_dropped_and_do_not_count() {
    let mut s = service(3);
    let a = add(&mut s, 1, MoverClass::Player, 1);
    let b = add(&mut s, 2, MoverClass::Player, 1);
    let c = add(&mut s, 3, MoverClass::Player, 1);
    s.cancel_request(MoverId(1));
    assert!(s.request(a).unwrap().cancel);
    let pass = s.update_thread();
    // The cancelled one is passed straight back; two more are searched.
    assert_eq!(pass.requests, vec![a, b, c]);
    let mut f = Finder::default();
    s.update_main(&NoCars, &mut f);
    let movers: Vec<u64> = f.log.iter().map(|e| e.1 .0).collect();
    assert_eq!(movers, vec![2, 3]);
}

#[test]
fn a_new_request_cancels_the_movers_previous_one() {
    let mut s = service(3);
    let first = add(&mut s, 7, MoverClass::Zombie, 3);
    let second = s.add_request(
        FinderId(70),
        snapshot(7, MoverClass::Zombie, 3),
        (2.5, 2.5, 0.0),
        (5.5, 5.5, 0.0),
        &[],
    );
    assert!(s.request(first).unwrap().cancel);
    assert_eq!(s.request_for(MoverId(7)), Some(second));
    s.update_thread();
    let mut f = Finder::default();
    s.update_main(&NoCars, &mut f);
    assert_eq!(f.log.len(), 1);
    assert_eq!(f.log[0].0, FinderId(70));
    // Cancelled after the search: finished but not delivered.
    let late = add(&mut s, 8, MoverClass::Player, 1);
    s.update_thread();
    assert!(!s.request(late).unwrap().path.is_empty());
    s.cancel_request(MoverId(8));
    let mut f = Finder::default();
    s.update_main(&NoCars, &mut f);
    assert!(f.log.is_empty());
    assert!(s.request(late).is_none());
}

#[test]
fn delivered_paths_are_in_game_levels_and_failures_call_failed() {
    let mut s = service(3);
    add(&mut s, 1, MoverClass::Player, 1);
    // A level with no squares: no path.
    s.add_request(
        FinderId(2),
        snapshot(2, MoverClass::Player, 1),
        (2.5, 2.5, 0.0),
        (10.5, 12.5, 3.0),
        &[],
    );
    s.update_thread();
    let mut f = Finder::default();
    s.update_main(&NoCars, &mut f);
    let path = f.log[0].2.as_ref().unwrap();
    assert!(path.len() >= 2);
    assert!(path.nodes.iter().all(|n| n.z == 0.0));
    let last = path.nodes.last().unwrap();
    assert_eq!((last.x, last.y), (10.5, 12.5));
    assert_eq!(f.log[1], (FinderId(2), MoverId(2), None));
}

#[test]
fn shortest_of_multiple_targets() {
    let mut s = service(4);
    let id = s.add_request(
        FinderId(1),
        snapshot(1, MoverClass::Player, 1),
        (2.5, 2.5, 0.0),
        (28.5, 28.5, 0.0),
        &[20.5, 2.5, 0.0, 6.5, 2.5, 0.0, 9.5, 9.5, 3.0, 2.5, 6.5, 0.0],
    );
    s.update_thread();
    let r = s.request(id).unwrap();
    // 6.5,2.5 and 2.5,6.5 tie at length 4; the earlier one wins.
    assert_eq!((r.target_x, r.target_y), (6.5, 2.5));
    assert_eq!(r.path.java_length(), 4.0);
    let mut f = Finder::default();
    s.update_main(&NoCars, &mut f);
    let p = f.log[0].2.as_ref().unwrap();
    assert_eq!(p.nodes.last().map(|n| (n.x, n.y)), Some((6.5, 2.5)));
}

#[test]
fn shortest_of_multiple_when_the_first_target_fails() {
    let mut s = service(4);
    let id = s.add_request(
        FinderId(1),
        snapshot(1, MoverClass::Zombie, 3),
        (2.5, 2.5, 0.0),
        (10.5, 12.5, 3.0),
        &[12.5, 2.5, 0.0],
    );
    s.update_thread();
    let r = s.request(id).unwrap();
    assert_eq!((r.target_x, r.target_y), (12.5, 2.5));
    assert!(!r.path.is_empty());
}

#[test]
fn wire_request_matches_find_path() {
    let mut snap = snapshot(1, MoverClass::Player, 1);
    snap.is_npc = true;
    let r = PendingRequest {
        finder: FinderId(0),
        mover: snap,
        start_x: 1.0,
        start_y: 2.0,
        start_z: 0.0,
        target_x: 3.0,
        target_y: 4.0,
        target_z: 1.0,
        target_xyz: Vec::new(),
        min_level: 0,
        max_level: 63,
        path: Path::default(),
        cancel: false,
    };
    let w = r.wire();
    assert_eq!(w.mover, MoverKind::Player);
    assert!(w.is_npc && w.avoid_farming_plants);
    let b = w.to_bytes();
    assert_eq!(f32::from_be_bytes(b[8..12].try_into().unwrap()), 32.0);
    // An animal is mover type 0 and never an NPC.
    let mut animal = r.clone();
    animal.mover.class = MoverClass::Animal;
    animal.mover.is_npc = true;
    assert_eq!(animal.wire().mover, MoverKind::Animal);
    assert!(!animal.wire().is_npc);
}

// ----- vehicles -----

struct Car {
    id: i16,
    poly: RefCell<VehiclePoly>,
}

impl GameVehicle for Car {
    fn vehicle_id(&self) -> i16 {
        self.id
    }
    fn poly(&self) -> VehiclePoly {
        *self.poly.borrow()
    }
    fn poly_plus_radius(&self) -> VehiclePoly {
        *self.poly.borrow()
    }
    fn up_vector_dot(&self) -> f32 {
        1.0
    }
    fn crawl_offsets(&self) -> Vec<f32> {
        vec![0.5]
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

struct Cars<'a>(&'a [Car]);
impl VehicleSource for Cars<'_> {
    fn for_each_vehicle(&self, f: &mut dyn FnMut(&dyn GameVehicle)) {
        for c in self.0 {
            f(c);
        }
    }
    fn for_each_vehicle_in_chunk(
        &self,
        _: i32,
        _: i32,
        f: &mut dyn FnMut(&dyn GameVehicle) -> bool,
    ) {
        for c in self.0 {
            if f(c) {
                return;
            }
        }
    }
}

#[test]
fn vehicle_state_sends_moves_beyond_the_epsilon() {
    let mut s = service(2);
    let poly = VehiclePoly {
        x1: 4.0,
        y1: 4.0,
        x2: 6.0,
        y2: 4.0,
        x3: 6.0,
        y3: 8.0,
        x4: 4.0,
        y4: 8.0,
        z: 0.0,
    };
    let cars = [Car {
        id: 5,
        poly: RefCell::new(poly),
    }];
    s.add_vehicle(&cars[0]);
    match &s.queued_vehicle_tasks()[0] {
        VehicleTask::Add(v) => {
            assert_eq!(v.id, 5);
            assert_eq!(v.poly.z, 32.0);
            assert_eq!(v.crawl_offsets, vec![0.5]);
        }
        t => panic!("{t:?}"),
    }
    s.update_thread();
    assert_eq!(s.world().vehicle_count(), 1);
    let mut f = Finder::default();
    s.update_main(&Cars(&cars), &mut f);
    assert!(s.queued_vehicle_tasks().is_empty());
    cars[0].poly.borrow_mut().x1 += 0.0005;
    s.update_main(&Cars(&cars), &mut f);
    assert!(s.queued_vehicle_tasks().is_empty());
    cars[0].poly.borrow_mut().x1 += 0.01;
    s.update_main(&Cars(&cars), &mut f);
    assert!(matches!(
        s.queued_vehicle_tasks()[0],
        VehicleTask::Update(_)
    ));
    // The new polygon is now the reference.
    s.update_main(&Cars(&cars), &mut f);
    assert_eq!(s.queued_vehicle_tasks().len(), 1);
    s.remove_vehicle(5);
    s.update_thread();
    assert_eq!(s.world().vehicle_count(), 0);
    // Removed: no longer tracked.
    cars[0].poly.borrow_mut().x1 += 1.0;
    s.update_main(&Cars(&cars), &mut f);
    assert!(s.queued_vehicle_tasks().is_empty());
}

// ----- MoverSnapshot::capture -----

#[derive(Default)]
struct Fake {
    id: u64,
    player: Option<FakePlayer>,
    zombie: Option<FakeZombie>,
    animal: Option<FakeAnimal>,
}

#[derive(Default)]
struct FakePlayer {
    npc: bool,
    fail: f32,
}
#[derive(Default)]
struct FakeZombie {
    crawling: bool,
    can_walk: bool,
    under_vehicle: bool,
    target: bool,
    sound: bool,
}
#[derive(Default)]
struct FakeAnimal {
    breaks: bool,
    climbs: bool,
}

impl CharacterView for Fake {
    fn x(&self) -> f32 {
        0.0
    }
    fn y(&self) -> f32 {
        0.0
    }
    fn z(&self) -> f32 {
        0.0
    }
    fn current_square(&self) -> Option<(i32, i32, i32)> {
        None
    }
    fn vehicle(&self) -> Option<i16> {
        None
    }
    fn is_sitting_on_furniture(&self) -> bool {
        false
    }
    fn sit_on_furniture_direction(&self) -> Option<IsoDirections> {
        None
    }
}

impl PathMover for Fake {
    fn mover_id(&self) -> MoverId {
        MoverId(self.id)
    }
    fn as_player(&self) -> Option<&dyn PlayerMover> {
        self.player.as_ref().map(|p| p as &dyn PlayerMover)
    }
    fn as_player_mut(&mut self) -> Option<&mut dyn PlayerMover> {
        self.player.as_mut().map(|p| p as &mut dyn PlayerMover)
    }
    fn as_zombie(&self) -> Option<&dyn ZombieMover> {
        self.zombie.as_ref().map(|p| p as &dyn ZombieMover)
    }
    fn as_zombie_mut(&mut self) -> Option<&mut dyn ZombieMover> {
        self.zombie.as_mut().map(|p| p as &mut dyn ZombieMover)
    }
    fn as_animal(&self) -> Option<&dyn AnimalMover> {
        self.animal.as_ref().map(|p| p as &dyn AnimalMover)
    }
    fn as_animal_mut(&mut self) -> Option<&mut dyn AnimalMover> {
        self.animal.as_mut().map(|p| p as &mut dyn AnimalMover)
    }
    fn path_find_progress(&self) -> PathFindProgress {
        PathFindProgress::NotRunning
    }
    fn set_path_find_progress(&mut self, _: PathFindProgress) {}
    fn set_path2(&mut self, _: Option<&Path>) {}
    fn has_path2(&self) -> bool {
        false
    }
    fn current_state(&self) -> CharacterState {
        CharacterState::Other(0)
    }
    fn is_current_state(&self, s: CharacterState) -> bool {
        s == CharacterState::Other(0)
    }
    fn previous_state(&self) -> CharacterState {
        CharacterState::Other(0)
    }
    fn deferred_movement(&self) -> (f32, f32) {
        (0.0, 0.0)
    }
    fn move_unmodded(&mut self, _: f32, _: f32) {}
    fn set_moving(&mut self, _: bool) {}
    fn is_animation_updating_this_frame(&self) -> bool {
        true
    }
    fn is_performing_no_aim_short_strafe(&self) -> bool {
        false
    }
    fn set_strafe_state_speed(&mut self, _: f32) {}
    fn forward_direction(&self) -> (f32, f32) {
        (1.0, 0.0)
    }
    fn set_forward_direction(&mut self, _: f32, _: f32) {}
    fn normalize_forward_direction(&mut self) {}
    fn direction_from_vector(&mut self, _: f32, _: f32) {}
    fn face_location(&mut self, _: f32, _: f32) {}
    fn face_location_f(&mut self, _: f32, _: f32) {}
    fn face_direction(&mut self, _: IsoDirections) {}
    fn face_this_object(&mut self, _: FaceTarget) {}
    fn should_be_turning(&self) -> bool {
        false
    }
    fn is_aiming(&self) -> bool {
        false
    }
    fn is_animating_backwards(&self) -> bool {
        false
    }
    fn update_animation_forward_direction(&mut self) {}
    fn set_collidable(&mut self, _: bool) {}
    fn test_collide_adjacent(&mut self, _: (i32, i32, i32), _: i32, _: i32, _: i32) -> bool {
        false
    }
    fn climb_over_fence(&mut self, _: IsoDirections) {}
    fn climb_through_window(&mut self, _: ObjectRef) {}
    fn climb_through_window_frame(&mut self, _: ObjectRef) {}
    fn can_climb_through_window(&self, _: ObjectRef) -> bool {
        true
    }
    fn could_open_door(&self, _: ObjectRef) -> bool {
        true
    }
    fn toggle_door(&mut self, _: ObjectRef) {}
    fn get_variable_boolean(&self, _: &str) -> bool {
        false
    }
    fn set_variable_bool(&mut self, _: &str, _: bool) {}
    fn set_variable_float(&mut self, _: &str, _: f32) {}
}

impl PlayerMover for FakePlayer {
    fn is_npc(&self) -> bool {
        self.npc
    }
    fn is_local_player(&self) -> bool {
        true
    }
    fn is_local(&self) -> bool {
        true
    }
    fn time_since_close_door(&self) -> f32 {
        100.0
    }
    fn climbing_fail_chance_float(&self) -> f32 {
        self.fail
    }
    fn climb_over_wall(&mut self, _: IsoDirections) {}
}

impl ZombieMover for FakeZombie {
    fn is_crawling(&self) -> bool {
        self.crawling
    }
    fn is_can_walk(&self) -> bool {
        self.can_walk
    }
    fn is_can_crawl_under_vehicle(&self) -> bool {
        self.under_vehicle
    }
    fn has_target(&self) -> bool {
        self.target
    }
    fn is_moving_to_player_sound(&self) -> bool {
        self.sound
    }
    fn is_remote_zombie(&self) -> bool {
        false
    }
    fn allow_repath_delay(&self) -> f32 {
        0.0
    }
    fn set_allow_repath_delay(&mut self, _: f32) {}
    fn set_running(&mut self, _: bool) {}
    fn is_ghost(&self) -> bool {
        false
    }
    fn set_ghost(&mut self, _: bool) {}
    fn get_down_is_near_start_xy(&self) -> bool {
        false
    }
    fn get_down_set_params(&mut self) {}
}

impl AnimalMover for FakeAnimal {
    fn should_break_obstacles_during_pathfinding(&self) -> bool {
        self.breaks
    }
    fn can_climb_fences(&self) -> bool {
        self.climbs
    }
    fn animal_size(&self) -> f32 {
        1.0
    }
}

struct Sandbox {
    easy_climbing: bool,
    bent_fences: bool,
}

impl PathWorldHooks for Sandbox {
    fn lore_speed(&self) -> i32 {
        2
    }
    fn easy_climbing(&self) -> bool {
        self.easy_climbing
    }
    fn bent_fences_enabled(&self) -> bool {
        self.bent_fences
    }
    fn is_client(&self) -> bool {
        false
    }
    fn is_server(&self) -> bool {
        false
    }
    fn game_time_multiplier(&self) -> f32 {
        1.0
    }
    fn thirty_fps_multiplier(&self) -> f32 {
        1.0
    }
    fn current_game_speed(&self) -> i32 {
        1
    }
}

#[test]
fn capture_follows_path_find_request_init() {
    let world = Sandbox {
        easy_climbing: false,
        bent_fences: true,
    };
    let player = Fake {
        id: 1,
        player: Some(FakePlayer {
            npc: true,
            fail: 1.0,
        }),
        ..Fake::default()
    };
    let s = MoverSnapshot::capture(&player, &world);
    assert_eq!((s.class, s.queue), (MoverClass::Player, 1));
    assert!(s.is_npc && s.avoid_farming_plants && s.can_climb_tall_fences && !s.can_thump);

    let crawler = Fake {
        id: 2,
        zombie: Some(FakeZombie {
            crawling: true,
            ..FakeZombie::default()
        }),
        ..Fake::default()
    };
    let s = MoverSnapshot::capture(&crawler, &world);
    assert_eq!((s.class, s.queue), (MoverClass::Zombie, 3));
    assert!(s.can_crawl && s.crawling && s.ignore_crawl_cost && s.can_thump && s.can_bend);
    assert!(!s.has_target);

    let hunter = Fake {
        id: 3,
        zombie: Some(FakeZombie {
            can_walk: true,
            under_vehicle: true,
            target: true,
            ..FakeZombie::default()
        }),
        ..Fake::default()
    };
    let s = MoverSnapshot::capture(&hunter, &world);
    assert_eq!(s.queue, 2);
    assert!(s.can_crawl && !s.crawling && !s.ignore_crawl_cost && s.has_target);

    let listener = Fake {
        id: 4,
        zombie: Some(FakeZombie {
            sound: true,
            ..FakeZombie::default()
        }),
        ..Fake::default()
    };
    let s = MoverSnapshot::capture(&listener, &world);
    assert!(s.has_target);
    assert_eq!(s.queue, 3);

    // An IsoAnimal is an IsoPlayer too, but is not treated as one.
    let cow = Fake {
        id: 5,
        player: Some(FakePlayer {
            npc: true,
            fail: 1.0,
        }),
        animal: Some(FakeAnimal {
            breaks: true,
            climbs: true,
        }),
        ..Fake::default()
    };
    let s = MoverSnapshot::capture(&cow, &world);
    assert_eq!((s.class, s.queue), (MoverClass::Animal, 3));
    assert!(s.can_thump && s.can_climb_fences);
    assert!(!s.is_npc && !s.avoid_farming_plants && !s.can_climb_tall_fences);
}
