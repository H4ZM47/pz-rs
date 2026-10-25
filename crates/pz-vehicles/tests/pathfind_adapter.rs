//! `pathfind_adapter!` on pz-pathfind's real traits, over a real
//! `BaseVehicle` built from a synthetic vehicle script.

use pz_pathfind::game_vehicle::{GameVehicle, VehicleSource};
use pz_pathfind::mover::*;
use pz_pathfind::path::Path;
use pz_vehicles::env::{self, SquareInfo, VehicleWorld};
use pz_vehicles::{BaseVehicle, VehicleRef};
use pz_world::IsoDirections;
use std::cell::RefCell;
use std::rc::Rc;

pz_vehicles::pathfind_adapter!(pz_pathfind);

const SCRIPT: &str = r#"
module Base
{
    vehicle AdapterCar
    {
        model
        {
            file = AdapterCar,
            scale = 1.0,
            offset = 0.0 0.2 0.0,
        }
        extents = 1.8 1.0 4.0,
        mass = 800,
        centerOfMassOffset = 0.0 0.4 0.0,
        wheel FrontLeft
        {
            front = true,
            offset = 0.8 -0.3 1.3,
            radius = 0.3,
            width = 0.2,
        }
        wheel RearLeft
        {
            front = false,
            offset = 0.8 -0.3 -1.3,
            radius = 0.3,
            width = 0.2,
        }
        passenger FrontLeft
        {
            position inside
            {
                offset = 0.4 0.0 0.3,
            }
            position outside
            {
                offset = 1.3 0.0 0.3,
                area = SeatFrontLeft,
            }
            area = SeatFrontLeft,
        }
        area SeatFrontLeft
        {
            xywh = 1.2 0.3 0.6 1.0,
        }
        area Engine
        {
            xywh = 0.0 2.3 1.8 0.6,
        }
    }
}
"#;

/// A world whose chunk lists come from a pz-world cell, and which knows the
/// zombies holding attack spots.
struct World(
    RefCell<pz_world::IsoCell>,
    RefCell<std::collections::HashMap<u64, pz_vehicles::surround::SurroundZombie>>,
);
impl VehicleWorld for World {
    fn surround_zombie(&self, key: u64) -> Option<pz_vehicles::surround::SurroundZombie> {
        self.1.borrow().get(&key).cloned()
    }
    fn square(&self, _: i32, _: i32, _: i32) -> Option<SquareInfo> {
        None
    }
    fn is_valid_chunk(&self, _: i32, _: i32) -> bool {
        true
    }
    fn chunk_loaded(&self, _: i32, _: i32) -> bool {
        true
    }
    fn chunk_vehicles(&self, wx: i32, wy: i32) -> Option<Vec<VehicleRef>> {
        pz_vehicles::cell_link::chunk_vehicles(&self.0.borrow(), wx, wy)
    }
}

/// The attacked character, seated in the vehicle.
struct Target {
    x: f32,
    y: f32,
    vehicle: i16,
}
impl CharacterView for Target {
    fn x(&self) -> f32 {
        self.x
    }
    fn y(&self) -> f32 {
        self.y
    }
    fn z(&self) -> f32 {
        0.0
    }
    fn current_square(&self) -> Option<(i32, i32, i32)> {
        None
    }
    fn vehicle(&self) -> Option<i16> {
        Some(self.vehicle)
    }
    fn is_sitting_on_furniture(&self) -> bool {
        false
    }
    fn sit_on_furniture_direction(&self) -> Option<IsoDirections> {
        None
    }
}

struct Zed;
impl ZombieMover for Zed {
    fn is_crawling(&self) -> bool {
        false
    }
    fn is_can_walk(&self) -> bool {
        true
    }
    fn is_can_crawl_under_vehicle(&self) -> bool {
        false
    }
    fn has_target(&self) -> bool {
        true
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
    fn set_allow_repath_delay(&mut self, _v: f32) {}
    fn set_running(&mut self, _running: bool) {}
    fn is_ghost(&self) -> bool {
        false
    }
    fn set_ghost(&mut self, _ghost: bool) {}
    fn get_down_is_near_start_xy(&self) -> bool {
        false
    }
    fn get_down_set_params(&mut self) {}
}

/// An attacking zombie: only identity, position and state are read.
struct Attacker {
    id: u64,
    x: f32,
    y: f32,
    on_floor: bool,
    zed: Zed,
}
impl CharacterView for Attacker {
    fn x(&self) -> f32 {
        self.x
    }
    fn y(&self) -> f32 {
        self.y
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
    fn is_on_floor(&self) -> bool {
        self.on_floor
    }
}
impl PathMover for Attacker {
    fn mover_id(&self) -> MoverId {
        MoverId(self.id)
    }
    fn as_player(&self) -> Option<&dyn PlayerMover> {
        None
    }
    fn as_player_mut(&mut self) -> Option<&mut dyn PlayerMover> {
        None
    }
    fn as_zombie(&self) -> Option<&dyn ZombieMover> {
        Some(&self.zed)
    }
    fn as_zombie_mut(&mut self) -> Option<&mut dyn ZombieMover> {
        Some(&mut self.zed)
    }
    fn as_animal(&self) -> Option<&dyn AnimalMover> {
        None
    }
    fn as_animal_mut(&mut self) -> Option<&mut dyn AnimalMover> {
        None
    }
    fn path_find_progress(&self) -> PathFindProgress {
        unimplemented!()
    }
    fn set_path_find_progress(&mut self, _p: PathFindProgress) {
        unimplemented!()
    }
    fn set_path2(&mut self, _path: Option<&Path>) {
        unimplemented!()
    }
    fn has_path2(&self) -> bool {
        unimplemented!()
    }
    fn current_state(&self) -> CharacterState {
        unimplemented!()
    }
    fn is_current_state(&self, _state: CharacterState) -> bool {
        unimplemented!()
    }
    fn previous_state(&self) -> CharacterState {
        unimplemented!()
    }
    fn deferred_movement(&self) -> (f32, f32) {
        unimplemented!()
    }
    fn move_unmodded(&mut self, _dx: f32, _dy: f32) {
        unimplemented!()
    }
    fn set_moving(&mut self, _moving: bool) {
        unimplemented!()
    }
    fn is_animation_updating_this_frame(&self) -> bool {
        unimplemented!()
    }
    fn is_performing_no_aim_short_strafe(&self) -> bool {
        unimplemented!()
    }
    fn set_strafe_state_speed(&mut self, _speed: f32) {
        unimplemented!()
    }
    fn forward_direction(&self) -> (f32, f32) {
        unimplemented!()
    }
    fn set_forward_direction(&mut self, _x: f32, _y: f32) {
        unimplemented!()
    }
    fn normalize_forward_direction(&mut self) {
        unimplemented!()
    }
    fn direction_from_vector(&mut self, _x: f32, _y: f32) {
        unimplemented!()
    }
    fn face_location(&mut self, _x: f32, _y: f32) {
        unimplemented!()
    }
    fn face_location_f(&mut self, _x: f32, _y: f32) {
        unimplemented!()
    }
    fn face_direction(&mut self, _dir: IsoDirections) {
        unimplemented!()
    }
    fn face_this_object(&mut self, _target: FaceTarget) {
        unimplemented!()
    }
    fn should_be_turning(&self) -> bool {
        unimplemented!()
    }
    fn is_aiming(&self) -> bool {
        unimplemented!()
    }
    fn is_animating_backwards(&self) -> bool {
        unimplemented!()
    }
    fn update_animation_forward_direction(&mut self) {
        unimplemented!()
    }
    fn set_collidable(&mut self, _collidable: bool) {
        unimplemented!()
    }
    fn test_collide_adjacent(
        &mut self,
        _from: (i32, i32, i32),
        _dx: i32,
        _dy: i32,
        _dz: i32,
    ) -> bool {
        unimplemented!()
    }
    fn climb_over_fence(&mut self, _dir: IsoDirections) {
        unimplemented!()
    }
    fn climb_through_window(&mut self, _window: ObjectRef) {
        unimplemented!()
    }
    fn climb_through_window_frame(&mut self, _frame: ObjectRef) {
        unimplemented!()
    }
    fn can_climb_through_window(&self, _window: ObjectRef) -> bool {
        unimplemented!()
    }
    fn could_open_door(&self, _door: ObjectRef) -> bool {
        unimplemented!()
    }
    fn toggle_door(&mut self, _door: ObjectRef) -> bool {
        unimplemented!()
    }
    fn get_variable_boolean(&self, _name: &str) -> bool {
        unimplemented!()
    }
    fn set_variable_bool(&mut self, _name: &str, _value: bool) {
        unimplemented!()
    }
    fn set_variable_float(&mut self, _name: &str, _value: f32) {
        unimplemented!()
    }
}

struct Seated;
impl pz_vehicles::vehicle::VehicleCharacter for Seated {}

fn script_manager() -> Rc<pz_scripts::ScriptManager> {
    let root = std::env::temp_dir().join(format!("pz-vehicles-adapter-{}", std::process::id()));
    let dir = root.join("media/scripts/vehicles");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("adapter_car.txt"), SCRIPT).unwrap();
    let sm = pz_scripts::ScriptManager::new();
    sm.load(&pz_fs::GameFs::new(root.to_str().unwrap()), &[])
        .unwrap();
    let _ = std::fs::remove_dir_all(&root);
    Rc::new(sm)
}

#[test]
fn adapter_reads_a_real_vehicle() {
    pz_vehicles::vehicle::set_script_manager(script_manager());
    let mut cell = pz_world::IsoCell::default();
    cell.chunks
        .insert((1250, 1100), pz_world::IsoChunk::new(1250, 1100));
    let world = Rc::new(World(RefCell::new(cell), Default::default()));
    env::set_world(world.clone());

    let v = BaseVehicle::new();
    v.set_script_name("Base.AdapterCar");
    v.set_script("Base.AdapterCar");
    assert!(v.script().is_some(), "script loads");
    let (x, y) = (1250.0 * 8.0 + 4.5, 1100.0 * 8.0 + 3.25);
    v.set_position(x, y, 0.0);
    {
        let q = pz_vehicles::jmath::Quaternionf::angle_axis(0.3, 0.0, 1.0, 0.0);
        let (ox, oy) = env::offset();
        let mut st = v.st_mut();
        st.saved_rot = q;
        st.jni_transform.set_rotation(&q);
        st.jni_transform.origin = pz_vehicles::jmath::Vector3f::new(x - ox, 0.0, y - oy);
        st.poly_dirty = true;
        st.chunk = Some((1250, 1100));
    }
    pz_vehicles::manager::add_to_world(&v);
    pz_vehicles::manager::process_add_vehicles();
    pz_vehicles::cell_link::add_to_chunk(&mut world.0.borrow_mut(), &v, (1250, 1100));
    let id = v.id();
    assert!(id >= 0);

    // GameVehicle.
    let gv = PathfindVehicle(v.clone());
    assert_eq!(gv.vehicle_id(), id);
    assert!(gv.has_script());
    let poly = gv.poly();
    assert!(poly.x1 != poly.x3 && poly.y1 != poly.y3);
    assert_eq!(gv.native_vehicle().poly.z, poly.z + 32.0);

    // VehicleSource.
    let mut n = 0;
    PathfindVehicles.for_each_vehicle(&mut |g| {
        assert_eq!(g.vehicle_id(), id);
        n += 1;
    });
    assert_eq!(n, 1);
    let mut in_chunk = 0;
    PathfindVehicles.for_each_vehicle_in_chunk(1250, 1100, &mut |_| {
        in_chunk += 1;
        false
    });
    assert_eq!(in_chunk, 1);
    let mut elsewhere = 0;
    PathfindVehicles.for_each_vehicle_in_chunk(1251, 1100, &mut |_| {
        elsewhere += 1;
        false
    });
    assert_eq!(elsewhere, 0);

    // VehicleScripts.
    let s: &dyn pz_pathfind::behavior::hooks::VehicleScripts = &PathfindVehicles;
    assert_eq!(s.position(id), (x, y, 0.0));
    assert_eq!(s.extents(id), (1.8, 1.0, 4.0));
    assert_eq!(s.center_of_mass_offset(id), (0.0, 0.4, 0.0));
    let w = v.world_pos(0.5, 0.0, 1.0);
    assert_eq!(s.world_pos(id, 0.5, 0.0, 1.0), (w.x, w.y, w.z));
    assert!(s.area_center(id, "Engine").is_some());
    assert!(s.area_center(id, "Nowhere").is_none());
    assert_eq!(
        s.passenger_position(id, 0, "outside").and_then(|p| p.area),
        Some("SeatFrontLeft".to_string())
    );
    assert_eq!(s.transform_origin_y(id), v.st().jni_transform.origin.y);
    assert!(s.closest_point_on_poly_dist_sq(id, x + 10.0, y) > 0.0);

    // chooseBestAttackPosition: a zombie by the seated target gets a spot
    // next to the vehicle and keeps it; a second zombie farther from that
    // spot than its holder gets another one.
    v.set_passenger(0, Some(Rc::new(Seated)), Default::default());
    let target = Target { x, y, vehicle: id };
    let attacker = |id: u64, dx: f32| Attacker {
        id,
        x: x + dx,
        y,
        on_floor: false,
        zed: Zed,
    };
    let known = |a: &Attacker| {
        world.1.borrow_mut().insert(
            a.id,
            pz_vehicles::surround::SurroundZombie {
                key: a.id,
                x: a.x,
                y: a.y,
                z: 0.0,
                on_floor: false,
                can_walk: true,
                dead: false,
                reused: false,
                target: Some(pz_vehicles::surround::SurroundTarget { x, y, seat: 0 }),
            },
        );
    };
    let a = attacker(7, 2.0);
    known(&a);
    let p1 = s
        .choose_best_attack_position(id, &target, &a)
        .expect("a spot");
    assert!(((p1.0 - x).powi(2) + (p1.1 - y).powi(2)).sqrt() < 3.0);
    assert_eq!(s.choose_best_attack_position(id, &target, &a), Some(p1));
    let b = attacker(8, 3.0);
    known(&b);
    let p2 = s
        .choose_best_attack_position(id, &target, &b)
        .expect("another spot");
    assert_ne!((p1.0, p1.1), (p2.0, p2.1));
    env::set_world(Rc::new(env::OpenWorld));
}
