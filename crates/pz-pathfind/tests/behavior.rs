//! `PathFindBehavior2` driven frame by frame with a fake character on
//! hand-built squares (no game data).

use std::collections::HashMap;

use pz_pathfind::behavior::*;
use pz_pathfind::game_vehicle::NoVehicles;
use pz_pathfind::main_thread::MainThreadMap;
use pz_pathfind::mover::*;
use pz_pathfind::path::Path;
use pz_pathfind::service::*;
use pz_pathfind::square_bits::{DefaultBitsHooks, SquareBits};
use pz_world::chunk::{IsoChunk, IsoGridSquare};
use pz_world::objects::{IsoDoor, IsoObject};
use pz_world::sprite::PropertyValueMap;
use pz_world::{IsoCell, IsoDirections, IsoMetaGrid, IsoSpriteManager, ObjectKind, SpriteTables};

fn names(v: &[&str]) -> Vec<String> {
    v.iter().map(|s| s.to_string()).collect()
}

/// Hand-authored enum tables: every constant `pz-world` and this module
/// look up, in an arbitrary order.
fn tables() -> SpriteTables {
    SpriteTables::new(
        names(&[
            "collideW",
            "collideN",
            "exterior",
            "NoWallLighting",
            "doorW",
            "doorN",
            "transparentW",
            "transparentN",
            "cutW",
            "cutN",
            "solid",
            "invisible",
            "HoppableN",
            "HoppableW",
            "canPathW",
            "canPathN",
            "blocksight",
            "ForceAmbient",
            "WallSE",
            "FloorHeightOneThird",
            "FloorHeightTwoThirds",
            "CantClimb",
            "open",
            "windowW",
            "windowN",
            "WindowW",
            "WindowN",
            "diamondFloor",
            "transparentFloor",
            "solidfloor",
            "hidewalls",
            "container",
            "WallOverlay",
            "attachedSE",
            "attachedW",
            "attachedN",
            "attachedE",
            "attachedS",
            "FloorOverlay",
            "vegitation",
            "NeverCutaway",
            "HasLightOnSprite",
            "EntityScript",
            "water",
            "solidtrans",
            "blueprint",
            "BlockRain",
            "trans",
            "noStart",
            "forceRender",
            "burntOut",
            "TallHoppableN",
            "TallHoppableW",
            "WallN",
            "WallW",
            "WallNTrans",
            "WallWTrans",
            "DoorWallN",
            "DoorWallW",
            "isEave",
            "makeWindowInvincible",
            "climbSheetTopW",
            "climbSheetTopN",
            "climbSheetTopE",
            "climbSheetTopS",
            "climbSheetW",
            "climbSheetN",
            "climbSheetE",
            "climbSheetS",
        ]),
        names(&[
            "normal",
            "tree",
            "wall",
            "doorN",
            "doorW",
            "windowFN",
            "windowFW",
            "isMoveAbleObject",
            "lightswitch",
            "curtainN",
            "curtainS",
            "curtainE",
            "curtainW",
            "jukebox",
            "radio",
            "doorFrN",
            "doorFrW",
            "stairsTN",
            "stairsMN",
            "stairsBN",
            "stairsTW",
            "stairsMW",
            "stairsBW",
            "WestRoofB",
            "WestRoofM",
            "WestRoofT",
        ]),
        names(&["North", "West"]),
    )
    .unwrap()
}

/// Sprite name, tile type, tileset, properties (an empty value sets the
/// flag of that name).
type Spec<'a> = (&'a str, &'a str, &'a str, &'a [(&'a str, &'a str)]);

const SPRITES: &[Spec] = &[
    ("floor", "normal", "floors", &[("solidfloor", "")]),
    (
        "fence_n",
        "normal",
        "fences",
        &[("collideN", ""), ("HoppableN", ""), ("canPathN", "")],
    ),
    (
        "door_n",
        "doorN",
        "doors",
        &[("doorN", ""), ("collideN", "")],
    ),
    ("crate", "normal", "furniture", &[("solid", "")]),
];

fn manager() -> IsoSpriteManager {
    let mut map = PropertyValueMap::default();
    // Keys `SetCustomPropertyValues` expects.
    for k in [
        "WindowN",
        "WindowW",
        "DoorWallN",
        "DoorWallW",
        "WallSE",
        "tree",
        "lightR",
        "lightG",
        "lightB",
        "ItemHeight",
        "Surface",
    ] {
        map.entry(k);
    }
    for (_, _, _, props) in SPRITES {
        for (k, v) in *props {
            let e = map.entry(k);
            if !e.iter().any(|x| x == v) {
                e.push(v.to_string());
            }
        }
    }
    let mut m = IsoSpriteManager::new(tables());
    m.generate_alias_map(map, &[]).unwrap();
    for (i, (name, ty, tileset, props)) in SPRITES.iter().enumerate() {
        let s = m.add_sprite(name, 100 + i as i32);
        let t = m.tables.object_type_from_string(ty);
        let (tables, alias) = (m.tables.clone(), m.alias.clone());
        let sp = &mut m.sprites[s];
        sp.name = Some(name.to_string());
        sp.tileset_name = Some(tileset.to_string());
        sp.tile_type = t;
        for (k, v) in *props {
            sp.properties.set(&tables, &alias, k, v, v.is_empty());
        }
    }
    m
}

/// Squares `0..24` x `0..24` on level 0, all floor, in chunks `0..3`.
struct Map {
    cell: IsoCell,
    m: IsoSpriteManager,
    meta: IsoMetaGrid,
}

impl Map {
    fn new() -> Map {
        let m = manager();
        let mut cell = IsoCell::default();
        for wy in 0..3 {
            for wx in 0..3 {
                cell.chunks.insert((wx, wy), IsoChunk::new(wx, wy));
            }
        }
        let mut map = Map {
            cell,
            m,
            meta: IsoMetaGrid::empty(),
        };
        for y in 0..24 {
            for x in 0..24 {
                map.put(x, y, &["floor"]);
            }
        }
        map
    }

    fn put(&mut self, x: i32, y: i32, objs: &[&str]) {
        let mut sq = IsoGridSquare::new(x, y, 0);
        for o in objs {
            let id = sq.new_object_id();
            sq.objects.push(IsoObject::new(
                id,
                ObjectKind::Object,
                Some(self.m.named_map[*o]),
            ));
        }
        sq.recalc_properties(&self.m);
        self.cell
            .chunks
            .get_mut(&(x.div_euclid(8), y.div_euclid(8)))
            .unwrap()
            .set_square(x.rem_euclid(8), y.rem_euclid(8), 0, Some(sq));
    }

    /// Adds a special object (door) to a square.
    fn add_special(&mut self, x: i32, y: i32, name: &str, kind: ObjectKind) -> ObjectRef {
        let mut sq = self.cell.get_grid_square(x, y, 0).unwrap().clone();
        let id = sq.new_object_id();
        sq.objects
            .push(IsoObject::new(id, kind, Some(self.m.named_map[name])));
        sq.special_objects.push(id);
        sq.recalc_properties(&self.m);
        *self.cell.get_grid_square_mut(x, y, 0).unwrap() = sq;
        ObjectRef { x, y, z: 0, id }
    }

    /// `RecalcAllWithNeighbours` for every chunk (the collide matrices).
    fn recalc(&mut self) {
        for wy in 0..3 {
            for wx in 0..3 {
                self.cell.recalc_chunk_squares(wx, wy, &self.meta, &self.m);
            }
        }
    }

    /// A pathfinder loaded with these squares.
    fn service(&mut self) -> PathfindService {
        self.recalc();
        let bits = SquareBits::new(&self.m, &self.meta, &self.cell, &DefaultBitsHooks).unwrap();
        let mut s = PathfindService::new(0, 0, 1, 1, false);
        for wy in 0..3 {
            for wx in 0..3 {
                s.add_chunk_to_world(&self.cell.chunks[&(wx, wy)], 1, &bits);
            }
        }
        while s.has_thread_work() {
            s.update_thread();
        }
        s
    }
}

/// A character that walks `step` per frame while moving.
#[derive(Default)]
struct Chr {
    x: f32,
    y: f32,
    z: f32,
    step: f32,
    moving: bool,
    /// What the animation did last frame (deferred movement lags
    /// `setMoving`).
    anim_moving: bool,
    progress: PathFindProgress,
    vars: HashMap<String, bool>,
    log: Vec<String>,
    zombie: Option<Zed>,
    player: Option<Plr>,
    door_opens: bool,
    stuck: bool,
}

#[derive(Default)]
struct Zed {
    crawling: bool,
}
#[derive(Default)]
struct Plr;

impl CharacterView for Chr {
    fn x(&self) -> f32 {
        self.x
    }
    fn y(&self) -> f32 {
        self.y
    }
    fn z(&self) -> f32 {
        self.z
    }
    fn current_square(&self) -> Option<(i32, i32, i32)> {
        Some((self.x.floor() as i32, self.y.floor() as i32, self.z as i32))
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

impl PathMover for Chr {
    fn mover_id(&self) -> MoverId {
        MoverId(1)
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
        None
    }
    fn as_animal_mut(&mut self) -> Option<&mut dyn AnimalMover> {
        None
    }
    fn path_find_progress(&self) -> PathFindProgress {
        self.progress
    }
    fn set_path_find_progress(&mut self, p: PathFindProgress) {
        self.progress = p;
    }
    fn set_path2(&mut self, _: Option<&Path>) {}
    fn has_path2(&self) -> bool {
        true
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
        if self.anim_moving {
            (self.step, 0.0)
        } else {
            (0.0, 0.0)
        }
    }
    fn move_unmodded(&mut self, dx: f32, dy: f32) {
        if !self.stuck {
            self.x += dx;
            self.y += dy;
        }
    }
    fn set_moving(&mut self, moving: bool) {
        self.moving = moving;
    }
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
    fn face_direction(&mut self, d: IsoDirections) {
        self.log.push(format!("face {d:?}"));
    }
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
    fn set_collidable(&mut self, c: bool) {
        self.log.push(format!("collidable {c}"));
    }
    fn test_collide_adjacent(&mut self, _: (i32, i32, i32), _: i32, _: i32, _: i32) -> bool {
        false
    }
    fn climb_over_fence(&mut self, d: IsoDirections) {
        self.log.push(format!("climb fence {d:?}"));
    }
    fn climb_through_window(&mut self, _: ObjectRef) {
        self.log.push("climb window".into());
    }
    fn climb_through_window_frame(&mut self, _: ObjectRef) {
        self.log.push("climb frame".into());
    }
    fn can_climb_through_window(&self, _: ObjectRef) -> bool {
        true
    }
    fn could_open_door(&self, _: ObjectRef) -> bool {
        true
    }
    fn toggle_door(&mut self, d: ObjectRef) -> bool {
        self.log.push(format!("toggle door {},{}", d.x, d.y));
        self.door_opens
    }
    fn get_variable_boolean(&self, name: &str) -> bool {
        self.vars.get(name).copied().unwrap_or(false)
    }
    fn set_variable_bool(&mut self, name: &str, v: bool) {
        self.vars.insert(name.to_string(), v);
    }
    fn set_variable_float(&mut self, _: &str, _: f32) {}
}

impl PlayerMover for Plr {
    fn is_npc(&self) -> bool {
        false
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
        0.0
    }
    fn climb_over_wall(&mut self, _: IsoDirections) {}
}

impl ZombieMover for Zed {
    fn is_crawling(&self) -> bool {
        self.crawling
    }
    fn is_can_walk(&self) -> bool {
        true
    }
    fn is_can_crawl_under_vehicle(&self) -> bool {
        false
    }
    fn has_target(&self) -> bool {
        false
    }
    fn is_moving_to_player_sound(&self) -> bool {
        false
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

struct Game;
impl PathWorldHooks for Game {
    fn lore_speed(&self) -> i32 {
        2
    }
    fn easy_climbing(&self) -> bool {
        false
    }
    fn bent_fences_enabled(&self) -> bool {
        false
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

fn player(x: f32, y: f32) -> Chr {
    Chr {
        x,
        y,
        step: 0.1,
        player: Some(Plr),
        door_opens: true,
        ..Chr::default()
    }
}

/// Runs frames until the behaviour stops working (or `frames` run out),
/// delivering results to it like the game's `updateMain`.
fn run(
    map: &Map,
    service: &mut PathfindService,
    pfb: &mut PathFindBehavior2,
    chr: &mut Chr,
    frames: usize,
) -> BehaviorResult {
    let bits = SquareBits::new(&map.m, &map.meta, &map.cell, &DefaultBitsHooks).unwrap();
    let mtm = MainThreadMap::new(&bits, &NoVehicles);
    for _ in 0..frames {
        let r = {
            let mut ctx = PathContext {
                service,
                map: &mtm,
                world: &Game,
                vehicles: &NoVehicleScripts,
                seating: &NoSeating,
                characters: &NoCharacters,
            };
            pfb.update(chr, &mut ctx)
        };
        if r != BehaviorResult::Working {
            if std::env::var_os("PFB_DEBUG").is_some() {
                eprintln!(
                    "end {r:?} at {} {} progress {:?} path {:?}",
                    chr.x,
                    chr.y,
                    chr.progress,
                    pfb.path()
                );
            }
            return r;
        }
        chr.anim_moving = chr.moving;
        service.update_thread();
        let mut d = Delivered::default();
        service.update_main(&NoVehicles, &mut d);
        for (finder, _, path) in d.results {
            assert_eq!(finder, pfb.finder);
            match path {
                Some(p) => pfb.succeeded(chr, &NoSeating, &p),
                None => pfb.failed(chr),
            }
        }
    }
    BehaviorResult::Working
}

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
