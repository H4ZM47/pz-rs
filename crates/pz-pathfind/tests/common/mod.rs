//! Fakes shared by the `PathFindBehavior2` and `PathHost` tests: hand-built
//! squares, a fake character and world hooks (no game data).

#![allow(dead_code, unused_imports)]

pub use std::collections::HashMap;

pub use pz_pathfind::behavior::*;
pub use pz_pathfind::game_vehicle::NoVehicles;
pub use pz_pathfind::main_thread::MainThreadMap;
pub use pz_pathfind::mover::*;
pub use pz_pathfind::path::Path;
pub use pz_pathfind::service::*;
pub use pz_pathfind::square_bits::{DefaultBitsHooks, SquareBits};
pub use pz_world::chunk::{IsoChunk, IsoGridSquare};
pub use pz_world::objects::{IsoDoor, IsoObject};
pub use pz_world::sprite::PropertyValueMap;
pub use pz_world::{
    IsoCell, IsoDirections, IsoMetaGrid, IsoSpriteManager, ObjectKind, SpriteTables,
};

pub fn names(v: &[&str]) -> Vec<String> {
    v.iter().map(|s| s.to_string()).collect()
}

/// Hand-authored enum tables: every constant `pz-world` and this module
/// look up, in an arbitrary order.
pub fn tables() -> SpriteTables {
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
pub type Spec<'a> = (&'a str, &'a str, &'a str, &'a [(&'a str, &'a str)]);

pub const SPRITES: &[Spec] = &[
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

pub fn manager() -> IsoSpriteManager {
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
pub struct Map {
    pub cell: IsoCell,
    pub m: IsoSpriteManager,
    pub meta: IsoMetaGrid,
}

impl Map {
    pub fn new() -> Map {
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

    pub fn put(&mut self, x: i32, y: i32, objs: &[&str]) {
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
    pub fn add_special(&mut self, x: i32, y: i32, name: &str, kind: ObjectKind) -> ObjectRef {
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
    pub fn recalc(&mut self) {
        for wy in 0..3 {
            for wx in 0..3 {
                self.cell.recalc_chunk_squares(wx, wy, &self.meta, &self.m);
            }
        }
    }

    /// A pathfinder loaded with these squares.
    pub fn service(&mut self) -> PathfindService {
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
pub struct Chr {
    /// `mover_id()`.
    pub id: u64,
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub step: f32,
    pub moving: bool,
    /// What the animation did last frame (deferred movement lags
    /// `setMoving`).
    pub anim_moving: bool,
    pub progress: PathFindProgress,
    pub vars: HashMap<String, bool>,
    pub log: Vec<String>,
    pub zombie: Option<Zed>,
    pub player: Option<Plr>,
    pub door_opens: bool,
    pub stuck: bool,
}

#[derive(Default)]
pub struct Zed {
    pub crawling: bool,
}
#[derive(Default)]
pub struct Plr;

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

pub struct Game;
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

pub fn player(x: f32, y: f32) -> Chr {
    Chr {
        id: 1,
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
pub fn run(
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
