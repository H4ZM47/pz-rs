//! `IsoPlayer.getUseableVehicle` / `getNearVehicle` over a real
//! `BaseVehicle` built from a synthetic vehicle script, standing in a
//! pz-world cell's chunk list.

use pz_vehicles::env::{self, SquareInfo, VehicleWorld};
use pz_vehicles::interact::{get_near_vehicle, get_useable_vehicle, Interactor};
use pz_vehicles::{BaseVehicle, VehicleRef};
use pz_world::chunk_vehicles::{LoadedVehicle, VehicleOrigin};
use pz_world::IsoCell;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

const SCRIPT: &str = r#"
module Base
{
    vehicle LookupCar
    {
        model
        {
            file = LookupCar,
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
        area Engine
        {
            xywh = 0.0 2.3 1.8 0.6,
        }
        part Engine
        {
            area = Engine,
            lua
            {
                use = Vehicles.Use.Engine,
            }
        }
    }
}
"#;

/// Where the vehicle stands (chunk 1250,1100) and what its world answers
/// for `PolygonalMap2.lineClearCollide`.
const CAR: (f32, f32) = (1250.0 * 8.0 + 4.5, 1100.0 * 8.0 + 3.25);

#[derive(Default)]
struct World {
    /// `None`: the world has no pathfinding map.
    blocked: Cell<Option<bool>>,
    /// `(from, to, z, ignore_doors, close_to_walls)` of the last query.
    last: RefCell<Option<((f32, f32), (f32, f32), i32, bool, bool)>>,
}
impl VehicleWorld for World {
    fn square(&self, _: i32, _: i32, _: i32) -> Option<SquareInfo> {
        None
    }
    fn is_valid_chunk(&self, _: i32, _: i32) -> bool {
        true
    }
    fn chunk_loaded(&self, _: i32, _: i32) -> bool {
        true
    }
    fn line_clear_collide_with(
        &self,
        x1: f32,
        y1: f32,
        x2: f32,
        y2: f32,
        z: i32,
        _vehicle: &BaseVehicle,
        ignore_doors: bool,
        close_to_walls: bool,
    ) -> Option<bool> {
        *self.last.borrow_mut() = Some(((x1, y1), (x2, y2), z, ignore_doors, close_to_walls));
        self.blocked.get()
    }
}

struct Fixture {
    cell: IsoCell,
    world: Rc<World>,
    car: VehicleRef,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        env::set_world(Rc::new(env::OpenWorld));
    }
}

fn script_manager() -> Rc<pz_scripts::ScriptManager> {
    let root = {
        static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        std::env::temp_dir().join(format!("pz-vehicles-lookup-{}-{n}", std::process::id()))
    };
    let dir = root.join("media/scripts/vehicles");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("lookup_car.txt"), SCRIPT).unwrap();
    let sm = pz_scripts::ScriptManager::new();
    sm.load(&pz_fs::GameFs::new(root.to_str().unwrap()), &[])
        .unwrap();
    let _ = std::fs::remove_dir_all(&root);
    Rc::new(sm)
}

/// A clear line, one car facing +y (its engine area on the +y end).
fn fixture() -> Fixture {
    pz_vehicles::vehicle::set_script_manager(script_manager());
    let world = Rc::new(World::default());
    world.blocked.set(Some(false));
    env::set_world(world.clone());
    let mut cell = IsoCell::default();
    for cx in 1248..=1252 {
        for cy in 1098..=1102 {
            cell.chunks.insert((cx, cy), pz_world::IsoChunk::new(cx, cy));
        }
    }
    let car = BaseVehicle::new();
    car.set_script_name("Base.LookupCar");
    car.set_script("Base.LookupCar");
    assert!(car.script().is_some(), "script loads");
    car.set_position(CAR.0, CAR.1, 0.0);
    {
        let q = pz_vehicles::jmath::Quaternionf::angle_axis(0.0, 0.0, 1.0, 0.0);
        let (ox, oy) = env::offset();
        let mut st = car.st_mut();
        st.saved_rot = q;
        st.jni_transform.set_rotation(&q);
        st.jni_transform.origin = pz_vehicles::jmath::Vector3f::new(CAR.0 - ox, 0.0, CAR.1 - oy);
        st.poly_dirty = true;
        st.chunk = Some((1250, 1100));
    }
    pz_vehicles::cell_link::add_to_chunk(&mut cell, &car, (1250, 1100));
    Fixture { cell, world, car }
}

fn player(x: f32, y: f32, forward: (f32, f32)) -> Interactor {
    Interactor {
        x,
        y,
        z: 0.0,
        forward,
        in_vehicle: false,
        is_local_player: true,
        player_index: 0,
    }
}

/// Standing in the engine area, looking at the car.
fn at_engine() -> Interactor {
    player(CAR.0, CAR.1 + 2.3, (0.0, -1.0))
}

/// Where a player standing beside the car looks to see it: a little off its
/// axes, because a segment parallel to a polygon edge finds no crossing
/// (`intersectLineWithVehicle` gives up on a zero denominator).
const TOWARD: (f32, f32) = (-0.995, 0.0998);

/// Two and a half squares beside the car, looking at it.
fn beside() -> Interactor {
    player(CAR.0 + 2.5, CAR.1, TOWARD)
}

#[test]
fn useable_vehicle_is_the_one_whose_part_the_player_faces() {
    let f = fixture();
    let found = get_useable_vehicle(&f.cell, &at_engine()).unwrap().expect("car");
    assert!(Rc::ptr_eq(&found, &f.car));
    let part = f.car.useable_part(&at_engine(), true).unwrap().expect("part");
    assert_eq!(part.id().as_deref(), Some("Engine"));
    // The reach check is `lineClearCollide(player, part, ..., false, true)`.
    let (from, to, z, ignore_doors, close_to_walls) = f.world.last.borrow().unwrap();
    assert_eq!(from, (CAR.0, CAR.1 + 2.3));
    assert_eq!(to.0, CAR.0);
    assert!((to.1 - CAR.1).abs() < 1e-3, "aims at the car's axis: {to:?}");
    assert_eq!((z, ignore_doors, close_to_walls), (0, false, true));
}

#[test]
fn useable_vehicle_needs_the_player_to_face_the_part() {
    let f = fixture();
    let away = player(CAR.0, CAR.1 + 2.3, (0.0, 1.0));
    assert!(get_useable_vehicle(&f.cell, &away).unwrap().is_none());
    // Without the direction check the part is found.
    assert!(f.car.useable_part(&away, false).unwrap().is_some());
}

#[test]
fn useable_vehicle_stops_at_an_obstacle() {
    let f = fixture();
    f.world.blocked.set(Some(true));
    assert!(get_useable_vehicle(&f.cell, &at_engine()).unwrap().is_none());
}

#[test]
fn useable_vehicle_rejects_far_other_level_and_seated_players() {
    let f = fixture();
    // Not in the part's area.
    assert!(get_useable_vehicle(&f.cell, &player(CAR.0 + 3.0, CAR.1 + 2.3, (0.0, -1.0)))
        .unwrap()
        .is_none());
    // More than six squares away (Manhattan distance).
    assert!(f
        .car
        .useable_part(&player(CAR.0 + 6.0, CAR.1 + 2.0, (0.0, -1.0)), true)
        .unwrap()
        .is_none());
    let upstairs = Interactor { z: 1.0, ..at_engine() };
    assert!(get_useable_vehicle(&f.cell, &upstairs).unwrap().is_none());
    let seated = Interactor { in_vehicle: true, ..at_engine() };
    assert!(get_useable_vehicle(&f.cell, &seated).unwrap().is_none());
    assert!(get_near_vehicle(&f.cell, &seated).unwrap().is_none());
}

#[test]
fn near_vehicle_is_the_one_in_front_within_four_squares() {
    let f = fixture();
    let found = get_near_vehicle(&f.cell, &beside()).unwrap().expect("car");
    assert!(Rc::ptr_eq(&found, &f.car));
    // The reach check runs to the point where the forward line meets the
    // car, from the player.
    let (from, to, _, ignore_doors, close_to_walls) = f.world.last.borrow().unwrap();
    assert_eq!(from, (CAR.0 + 2.5, CAR.1));
    let want = (CAR.0 + 0.9, CAR.1 + 1.6 * 0.0998 / 0.995);
    assert!((to.0 - want.0).abs() < 1e-2 && (to.1 - want.1).abs() < 1e-2, "{to:?} vs {want:?}");
    assert_eq!((ignore_doors, close_to_walls), (false, true));
}

#[test]
fn near_vehicle_needs_the_line_to_cross_the_car() {
    let f = fixture();
    let away = player(CAR.0 + 2.5, CAR.1, (0.995, 0.0998));
    assert!(get_near_vehicle(&f.cell, &away).unwrap().is_none());
    // Looking at it, but too far: 5 - 0.9 squares from the polygon.
    let far = player(CAR.0 + 5.0, CAR.1, TOWARD);
    assert!(get_near_vehicle(&f.cell, &far).unwrap().is_none());
    // Looking along one of its edges finds no crossing.
    assert!(get_near_vehicle(&f.cell, &player(CAR.0 + 2.5, CAR.1, (-1.0, 0.0)))
        .unwrap()
        .is_none());
    // Looking at it, but something is in the way.
    f.world.blocked.set(Some(true));
    assert!(get_near_vehicle(&f.cell, &beside()).unwrap().is_none());
}

#[test]
fn near_vehicle_skips_a_vehicle_hidden_from_a_local_player() {
    let f = fixture();
    f.car.st_mut().target_alpha[1] = 0.0;
    // Player 1 does not see it; player 0 does.
    let p1 = Interactor { player_index: 1, ..beside() };
    assert!(get_near_vehicle(&f.cell, &p1).unwrap().is_none());
    assert!(get_near_vehicle(&f.cell, &beside()).unwrap().is_some());
    // The alpha only matters for a local player.
    let remote = Interactor { is_local_player: false, ..p1 };
    assert!(get_near_vehicle(&f.cell, &remote).unwrap().is_some());
    // Another level.
    let upstairs = Interactor { z: 1.0, ..beside() };
    assert!(get_near_vehicle(&f.cell, &upstairs).unwrap().is_none());
}

#[test]
fn no_vehicle_around_answers_none() {
    let f = fixture();
    let nowhere = player(CAR.0 + 200.0, CAR.1 + 200.0, (-1.0, 0.0));
    assert!(get_useable_vehicle(&f.cell, &nowhere).unwrap().is_none());
    assert!(get_near_vehicle(&f.cell, &nowhere).unwrap().is_none());
    // Chunks that are not loaded are skipped.
    let empty = IsoCell::default();
    assert!(get_useable_vehicle(&empty, &at_engine()).unwrap().is_none());
    assert!(get_near_vehicle(&empty, &beside()).unwrap().is_none());
}

#[test]
fn an_unanswerable_reach_check_is_an_error_not_a_miss() {
    let f = fixture();
    f.world.blocked.set(None);
    let e = get_useable_vehicle(&f.cell, &at_engine()).unwrap_err();
    assert!(e.0.contains("lineClearCollide"), "{e}");
    assert!(get_near_vehicle(&f.cell, &beside()).is_err());
}

#[test]
fn a_vehicle_chunk_loading_has_not_adopted_is_an_error_not_a_miss() {
    let mut f = fixture();
    let loaded = LoadedVehicle {
        origin: VehicleOrigin::Database { sql_id: 7 },
        square: None,
        vehicle: saved_vehicle(),
    };
    f.cell
        .chunks
        .get_mut(&(1249, 1100))
        .unwrap()
        .vehicles
        .add(Rc::new(loaded));
    // The list is walked in order: the car in chunk 1250 comes after it.
    let e = get_near_vehicle(&f.cell, &beside()).unwrap_err();
    assert!(e.0.contains("1249, 1100"), "{e}");
    // Chunks the search does not reach are not looked at.
    let far = player(CAR.0 - 400.0, CAR.1, TOWARD);
    assert!(get_near_vehicle(&f.cell, &far).unwrap().is_none());
}

/// A vehicle as chunk loading leaves it in the list.
fn saved_vehicle() -> pz_world::save::vehicle::BaseVehicle {
    let base = pz_world::save::visual::MovingBase {
        offset_x: 0.0,
        offset_y: 0.0,
        x: 0.0,
        y: 0.0,
        z: 0.0,
        dir: 0,
        mod_data: None,
    };
    pz_vehicles::save::to_save(&BaseVehicle::new(), &base)
}
