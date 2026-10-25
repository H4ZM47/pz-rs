//! `PathFindBehavior2` driven frame by frame with a fake character on
//! hand-built squares (no game data).

mod common;

use common::*;

#[test]
fn walks_to_a_location() {
    let mut map = Map::new();
    let mut service = map.service();
    let mut chr = player(2.5, 2.5);
    let mut pfb = PathFindBehavior2::new(FinderId(9));
    pfb.path_to_location(&mut chr, &mut service, 10, 2, 0);
    assert!(pfb.is_goal_location() && !pfb.is_cancelled());
    assert_eq!((pfb.target_x(), pfb.target_y()), (10.5, 2.5));
    let r = run(&map, &mut service, &mut pfb, &mut chr, 500);
    assert_eq!(r, BehaviorResult::Succeeded);
    assert!(
        (chr.x - 10.5).abs() < 0.06 && (chr.y - 2.5).abs() < 1e-4,
        "{} {}",
        chr.x,
        chr.y
    );
    assert!(pfb.has_started_moving());
    assert!(!pfb.path_next_is_set);
}

#[test]
fn request_carries_the_alternative_targets_and_set_data_cancels() {
    let mut map = Map::new();
    let mut service = map.service();
    let mut chr = player(2.5, 2.5);
    let mut pfb = PathFindBehavior2::new(FinderId(9));
    assert_eq!(
        pfb.path_to_nearest(&mut chr, &mut service, &[1.0, 2.0]),
        Err(PathArgumentError::NotTriples)
    );
    assert_eq!(
        pfb.path_to_nearest(&mut chr, &mut service, &[]),
        Err(PathArgumentError::Empty)
    );
    pfb.path_to_nearest(&mut chr, &mut service, &[12.5, 2.5, 0.0, 4.5, 2.5, 0.0])
        .unwrap();
    let bits = SquareBits::new(&map.m, &map.meta, &map.cell, &DefaultBitsHooks).unwrap();
    let mtm = MainThreadMap::new(&bits, &NoVehicles);
    let mut ctx = PathContext {
        service: &mut service,
        map: &mtm,
        world: &Game,
        vehicles: &NoVehicleScripts,
        seating: &NoSeating,
        characters: &NoCharacters,
    };
    assert_eq!(pfb.update(&mut chr, &mut ctx), BehaviorResult::Working);
    assert_eq!(chr.progress, PathFindProgress::NotYetFound);
    let id = ctx.service.request_for(MoverId(1)).unwrap();
    assert_eq!(
        ctx.service.request(id).unwrap().target_xyz,
        vec![4.5, 2.5, 0.0]
    );
    pfb.path_to_location(&mut chr, ctx.service, 5, 5, 0);
    assert!(ctx.service.request(id).unwrap().cancel);
    assert_eq!(chr.progress, PathFindProgress::NotRunning);
    // The shortest target wins.
    let mut pfb = PathFindBehavior2::new(FinderId(9));
    let mut chr = player(2.5, 2.5);
    pfb.path_to_nearest(&mut chr, &mut service, &[12.5, 2.5, 0.0, 4.5, 2.5, 0.0])
        .unwrap();
    assert_eq!(
        run(&map, &mut service, &mut pfb, &mut chr, 500),
        BehaviorResult::Succeeded
    );
    assert!((chr.x - 4.5).abs() < 0.06);
}

#[test]
fn opens_a_closed_door_on_the_way() {
    let mut map = Map::new();
    // Door on the north edge of (5, 6): between (5, 5) and (5, 6).
    map.add_special(
        5,
        6,
        "door_n",
        ObjectKind::Door(IsoDoor {
            north: true,
            ..IsoDoor::default()
        }),
    );
    let mut service = map.service();
    let mut chr = player(5.5, 3.5);
    let mut pfb = PathFindBehavior2::new(FinderId(1));
    pfb.path_to_location(&mut chr, &mut service, 5, 9, 0);
    let r = run(&map, &mut service, &mut pfb, &mut chr, 600);
    assert!(
        chr.log.iter().any(|l| l == "toggle door 5,6"),
        "{:?}",
        chr.log
    );
    assert!(chr.log.iter().any(|l| l == "collidable true"));
    assert_eq!(r, BehaviorResult::Succeeded);

    // A door that stays shut fails the path.
    let mut chr = player(5.5, 3.5);
    chr.door_opens = false;
    let mut pfb = PathFindBehavior2::new(FinderId(1));
    pfb.path_to_location(&mut chr, &mut service, 5, 9, 0);
    assert_eq!(
        run(&map, &mut service, &mut pfb, &mut chr, 600),
        BehaviorResult::Failed
    );
}

#[test]
fn climbs_a_fence() {
    let mut map = Map::new();
    for x in 0..24 {
        map.put(x, 8, &["floor", "fence_n"]);
    }
    let mut service = map.service();
    let mut chr = player(5.5, 5.5);
    let mut pfb = PathFindBehavior2::new(FinderId(1));
    pfb.path_to_location(&mut chr, &mut service, 5, 11, 0);
    // The fake does not climb, so it walks on the spot until it fails.
    let r = run(&map, &mut service, &mut pfb, &mut chr, 2000);
    assert!(chr.log.iter().any(|l| l == "face S"), "{:?}", chr.log);
    assert!(chr.log.iter().any(|l| l == "climb fence S"));
    let _ = r;
}

#[test]
fn standing_still_fails_after_400_multiplier_units() {
    let mut map = Map::new();
    let mut service = map.service();
    let mut chr = player(2.5, 2.5);
    chr.stuck = true;
    let mut pfb = PathFindBehavior2::new(FinderId(1));
    pfb.path_to_location(&mut chr, &mut service, 10, 2, 0);
    let mut frames = 0;
    let bits = SquareBits::new(&map.m, &map.meta, &map.cell, &DefaultBitsHooks).unwrap();
    let mtm = MainThreadMap::new(&bits, &NoVehicles);
    loop {
        frames += 1;
        let mut ctx = PathContext {
            service: &mut service,
            map: &mtm,
            world: &Game,
            vehicles: &NoVehicleScripts,
            seating: &NoSeating,
            characters: &NoCharacters,
        };
        let r = pfb.update(&mut chr, &mut ctx);
        if r != BehaviorResult::Working {
            assert_eq!(r, BehaviorResult::Failed);
            break;
        }
        chr.anim_moving = chr.moving;
        ctx.service.update_thread();
        let mut d = Delivered::default();
        ctx.service.update_main(&NoVehicles, &mut d);
        for (_, _, p) in d.results {
            pfb.succeeded(&mut chr, &NoSeating, p.as_ref().unwrap());
        }
        assert!(frames < 1000);
    }
    // Request, then the found path's first frame resets the timer; 401
    // frames standing still after that.
    assert!((400..=404).contains(&frames), "{frames}");
}

#[test]
fn zombie_gets_down_before_a_crawl_stretch() {
    let mut map = Map::new();
    let mut service = map.service();
    let mut chr = Chr {
        id: 1,
        x: 2.5,
        y: 2.5,
        step: 0.05,
        zombie: Some(Zed::default()),
        ..Chr::default()
    };
    let mut pfb = PathFindBehavior2::new(FinderId(1));
    pfb.path_to_location(&mut chr, &mut service, 8, 2, 0);
    let mut path = Path::default();
    path.add_node(2.5, 2.5, 0.0, 0);
    path.add_node(4.5, 2.5, 0.0, 1);
    path.add_node(8.5, 2.5, 0.0, 1);
    pfb.succeeded(&mut chr, &NoSeating, &path);
    assert_eq!(chr.progress, PathFindProgress::Found);
    let r = run(&map, &mut service, &mut pfb, &mut chr, 200);
    assert!(chr.get_variable_boolean("ShouldBeCrawling"));
    let _ = r;
}

#[test]
fn strafing_only_for_short_non_zombie_paths() {
    let mut map = Map::new();
    let mut service = map.service();
    let mut chr = player(2.5, 2.5);
    let mut pfb = PathFindBehavior2::new(FinderId(1));
    pfb.path_to_location_f(&mut chr, &mut service, 2.8, 2.5, 0.0);
    let mut path = Path::default();
    path.add_node(2.5, 2.5, 0.0, 0);
    path.add_node(2.8, 2.5, 0.0, 0);
    pfb.succeeded(&mut chr, &NoSeating, &path);
    assert!(pfb.is_strafing(&chr));
    chr.zombie = Some(Zed::default());
    assert!(!pfb.is_strafing(&chr));
    // getPathLength: straight to the target once past the last segment.
    assert!((pfb.get_path_length(&chr) - 0.3).abs() < 1e-6);
}

#[test]
fn closest_and_advance_on_a_straight_path() {
    let mut map = Map::new();
    map.recalc();
    let bits = SquareBits::new(&map.m, &map.meta, &map.cell, &DefaultBitsHooks).unwrap();
    let mtm = MainThreadMap::new(&bits, &NoVehicles);
    let mut path = Path::default();
    path.add_node(2.5, 2.5, 0.0, 0);
    path.add_node(4.5, 2.5, 0.0, 0);
    path.add_node(4.5, 6.5, 0.0, 0);
    let mut pop = PointOnPath::default();
    closest_point_on_path(&mtm, 3.0, 3.0, 0.0, None, &path, &mut pop);
    assert_eq!(
        (pop.path_index, pop.dist, pop.x, pop.y),
        (0, 0.25, 3.0, 2.5)
    );
    advance_along_path(&mtm, 3.0, 2.5, 0.0, 2.5, None, &path, &mut pop);
    assert_eq!(pop.path_index, 1);
    assert_eq!((pop.x, pop.y), (4.5, 3.5));
    advance_along_path(&mtm, 3.0, 2.5, 0.0, 50.0, None, &path, &mut pop);
    assert_eq!((pop.path_index, pop.dist, pop.x, pop.y), (2, 1.0, 4.5, 6.5));
}

struct Body {
    animal: bool,
}
impl DeadBodyView for Body {
    fn position(&self) -> (f32, f32, f32) {
        (5.0, 5.0, 0.0)
    }
    fn is_animal(&self) -> bool {
        self.animal
    }
    fn animal_collision_size(&self) -> Option<f32> {
        Some(0.1)
    }
    fn animal_size(&self) -> f32 {
        2.0
    }
    fn grab_head_position(&self) -> (f32, f32) {
        (5.5, 5.0)
    }
    fn grab_legs_position(&self) -> (f32, f32) {
        (4.5, 5.0)
    }
    fn can_be_grabbed_from(&self, x: f32, _: f32) -> bool {
        x > 5.0
    }
}

#[test]
fn grab_corpse_positions() {
    let pts = PathFindBehavior2::grab_corpse_locations(&Body { animal: false });
    assert_eq!(pts, vec![(5.5, 5.0)]);
    let pts = PathFindBehavior2::grab_corpse_locations(&Body { animal: true });
    // 12 points on a circle of radius 2 * max(0.1, 0.2) * 2.
    assert_eq!(pts.len(), 12);
    assert!((pts[0].0 - 5.8).abs() < 1e-6 && pts[0].1 == 5.0);
    let mut service = PathfindService::new(0, 0, 1, 1, false);
    let mut chr = player(1.0, 1.0);
    let mut pfb = PathFindBehavior2::new(FinderId(1));
    assert!(pfb.path_to_grab_corpse(&mut chr, &mut service, &Body { animal: true }));
    assert_eq!(pfb.goal(), Goal::GrabCorpse);
    assert_eq!(pfb.target_xyz().len(), 33);
}

struct Car;
impl VehicleScripts for Car {
    fn position(&self, _: i16) -> (f32, f32, f32) {
        (8.0, 8.0, 0.0)
    }
    fn current_speed_km_hour(&self, _: i16) -> f32 {
        0.0
    }
    fn closest_point_on_poly_dist_sq(&self, _: i16, _: f32, _: f32) -> f32 {
        50.0
    }
    fn choose_best_attack_position(
        &self,
        _: i16,
        _: &dyn CharacterView,
        _: &dyn PathMover,
    ) -> Option<(f32, f32, f32)> {
        None
    }
    fn extents(&self, _: i16) -> (f32, f32, f32) {
        (2.0, 1.0, 4.0)
    }
    fn center_of_mass_offset(&self, _: i16) -> (f32, f32, f32) {
        (0.0, 0.0, 0.0)
    }
    fn world_pos(&self, _: i16, x: f32, _: f32, z: f32) -> (f32, f32, f32) {
        (8.0 + x, 8.0 + z, 0.0)
    }
    fn area_center(&self, _: i16, area: &str) -> Option<(f32, f32)> {
        (area == "Engine").then_some((8.0, 5.5))
    }
    fn passenger_position(&self, _: i16, _: i32, _: &str) -> Option<PassengerPosition> {
        None
    }
    fn passenger_position_world_pos(&self, _: i16, _: i32, _: &str) -> (f32, f32, f32) {
        (0.0, 0.0, 0.0)
    }
    fn area_position_world_4_player_interact(&self, _: i16, _: &str) -> (f32, f32) {
        (0.0, 0.0)
    }
    fn transform_origin_y(&self, _: i16) -> f32 {
        0.0
    }
    fn passenger_door_area(&self, _: i16, seat: i32) -> Option<String> {
        (seat == 0).then(|| "Engine".to_string())
    }
}

#[test]
fn vehicle_goals() {
    let mut map = Map::new();
    let mut service = map.service();
    let bits = SquareBits::new(&map.m, &map.meta, &map.cell, &DefaultBitsHooks).unwrap();
    let mtm = MainThreadMap::new(&bits, &NoVehicles);
    let mut ctx = PathContext {
        service: &mut service,
        map: &mtm,
        world: &Game,
        vehicles: &Car,
        seating: &NoSeating,
        characters: &NoCharacters,
    };
    // In sight on the same level: a direct two-node path.
    let mut chr = player(8.0, 2.5);
    let mut pfb = PathFindBehavior2::new(FinderId(1));
    pfb.path_to_vehicle_area(&mut chr, &mut ctx, 3, "Engine");
    assert!(pfb.is_goal_vehicle_area());
    assert_eq!(chr.progress, PathFindProgress::Found);
    assert_eq!(pfb.path().len(), 2);
    // No such area: fails at once.
    let mut pfb = PathFindBehavior2::new(FinderId(1));
    pfb.path_to_vehicle_area(&mut chr, &mut ctx, 3, "Trunk");
    assert_eq!(chr.progress, PathFindProgress::Failed);
    // A seat without an outside position uses the door's area.
    let mut pfb = PathFindBehavior2::new(FinderId(1));
    pfb.path_to_vehicle_seat(&mut chr, &mut ctx, 3, 0);
    assert!(pfb.is_goal_vehicle_area());
    let mut pfb = PathFindBehavior2::new(FinderId(1));
    pfb.path_to_vehicle_seat(&mut chr, &mut ctx, 3, 1);
    assert_eq!(chr.progress, PathFindProgress::Failed);
    // Four sides; all on floor.
    let mut pfb = PathFindBehavior2::new(FinderId(1));
    assert!(pfb.path_to_vehicle_adjacent(&mut chr, &mut ctx, 3));
    assert_eq!((pfb.target_x(), pfb.target_y()), (6.7, 8.0));
    assert_eq!(pfb.target_xyz().len(), 9);
}

#[test]
fn should_ignore_collision_with_the_furniture_square() {
    let mut service = PathfindService::new(0, 0, 1, 1, false);
    let map = Map::new();
    let bits = SquareBits::new(&map.m, &map.meta, &map.cell, &DefaultBitsHooks).unwrap();
    let mtm = MainThreadMap::new(&bits, &NoVehicles);
    let mut ctx = PathContext {
        service: &mut service,
        map: &mtm,
        world: &Game,
        vehicles: &NoVehicleScripts,
        seating: &NoSeating,
        characters: &NoCharacters,
    };
    let mut chr = player(1.5, 1.5);
    let mut pfb = PathFindBehavior2::new(FinderId(1));
    let chair = ObjectRef {
        x: 4,
        y: 4,
        z: 0,
        id: 1,
    };
    pfb.path_to_sit_on_furniture(&mut chr, &mut ctx, chair, false);
    // No seat positions without a seating manager: fails, goal kept.
    assert_eq!(chr.progress, PathFindProgress::Failed);
    assert!(pfb.should_ignore_collision_with_square((4, 4, 0)));
    assert!(!pfb.should_ignore_collision_with_square((4, 5, 0)));
    assert!(!pfb.should_get_up_from_crawl(&chr));
}

#[test]
fn fake_character_reports_floor_death_and_grappling() {
    use pz_pathfind::mover::CharacterView;
    let chr = Chr {
        on_floor: true,
        dead: true,
        reanimated_for_grappling: true,
        ..Chr::default()
    };
    assert!(chr.is_on_floor() && chr.is_dead() && chr.is_reanimated_for_grappling());
    let plain = Chr::default();
    assert!(!plain.is_on_floor() && !plain.is_dead() && !plain.is_reanimated_for_grappling());
}
