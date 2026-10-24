//! Game state `PathFindBehavior2` reads that has no port yet: vehicle
//! scripts and transforms (`BaseVehicle`, `VehicleScript`), the seating
//! manager, dead bodies and other characters. Each is a small trait with a
//! "nothing there" implementation.

use crate::mover::{CharacterView, MoverId, ObjectRef, PathMover};

/// A `VehicleScript.Position` returned by
/// `BaseVehicle.getPassengerPosition(seat, id)`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PassengerPosition {
    /// `posn.area`: the id of the script area the position belongs to.
    pub area: Option<String>,
}

/// `BaseVehicle` / `VehicleScript` data, by `vehicleId`. Positions are world
/// coordinates as the Java methods return them.
pub trait VehicleScripts {
    /// `BaseVehicle.getX()`, `getY()`, `getZ()`.
    fn position(&self, vehicle: i16) -> (f32, f32, f32);
    /// `BaseVehicle.getCurrentSpeedKmHour()`.
    fn current_speed_km_hour(&self, vehicle: i16) -> f32;
    /// `BaseVehicle.getClosestPointOnPoly(x, y, out)`: the squared distance
    /// from `(x, y)` to the vehicle's polygon (what
    /// `IsoMovingObject.DistToSquared(vehicle)` returns).
    fn closest_point_on_poly_dist_sq(&self, vehicle: i16, x: f32, y: f32) -> f32;
    /// `BaseVehicle.chooseBestAttackPosition(target, attacker, out)`.
    fn choose_best_attack_position(
        &self,
        vehicle: i16,
        target: &dyn CharacterView,
        attacker: &dyn PathMover,
    ) -> Option<(f32, f32, f32)>;
    /// `getScript().getExtents()`.
    fn extents(&self, vehicle: i16) -> (f32, f32, f32);
    /// `getScript().getCenterOfMassOffset()`.
    fn center_of_mass_offset(&self, vehicle: i16) -> (f32, f32, f32);
    /// `BaseVehicle.getWorldPos(x, y, z, out)`: a vehicle-local point in
    /// world coordinates.
    fn world_pos(&self, vehicle: i16, x: f32, y: f32, z: f32) -> (f32, f32, f32);
    /// `BaseVehicle.getAreaCenter(areaId)`.
    fn area_center(&self, vehicle: i16, area: &str) -> Option<(f32, f32)>;
    /// `BaseVehicle.getPassengerPosition(seat, id)` (`"outside"`,
    /// `"outside2"`).
    fn passenger_position(&self, vehicle: i16, seat: i32, id: &str) -> Option<PassengerPosition>;
    /// `BaseVehicle.getPassengerPositionWorldPos(posn, out)` for that
    /// position.
    fn passenger_position_world_pos(&self, vehicle: i16, seat: i32, id: &str) -> (f32, f32, f32);
    /// `BaseVehicle.areaPositionWorld4PlayerInteract(getScript()
    /// .getAreaById(area), out)`.
    fn area_position_world_4_player_interact(&self, vehicle: i16, area: &str) -> (f32, f32);
    /// `BaseVehicle.jniTransform.origin.y`.
    fn transform_origin_y(&self, vehicle: i16) -> f32;
    /// `BaseVehicle.getPassengerDoor(seat).getArea()`, `None` without a
    /// door.
    fn passenger_door_area(&self, vehicle: i16, seat: i32) -> Option<String>;
}

/// No vehicle data: every vehicle is at the origin, has no areas,
/// positions or doors, and no attack position.
pub struct NoVehicleScripts;

impl VehicleScripts for NoVehicleScripts {
    fn position(&self, _: i16) -> (f32, f32, f32) {
        (0.0, 0.0, 0.0)
    }
    fn current_speed_km_hour(&self, _: i16) -> f32 {
        0.0
    }
    fn closest_point_on_poly_dist_sq(&self, _: i16, _: f32, _: f32) -> f32 {
        f32::MAX
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
        (0.0, 0.0, 0.0)
    }
    fn center_of_mass_offset(&self, _: i16) -> (f32, f32, f32) {
        (0.0, 0.0, 0.0)
    }
    fn world_pos(&self, _: i16, x: f32, y: f32, z: f32) -> (f32, f32, f32) {
        (x, y, z)
    }
    fn area_center(&self, _: i16, _: &str) -> Option<(f32, f32)> {
        None
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
    fn passenger_door_area(&self, _: i16, _: i32) -> Option<String> {
        None
    }
}

/// `SeatingManager` and the furniture object queries of sitting down.
pub trait SeatingHooks {
    /// `SeatingManager.getInstance().getAdjacentPosition(chr, furniture,
    /// direction, side, "sitonfurniture", "SitOnFurniture" + side, out)`;
    /// `direction` is one of `N S W E`, `side` one of `Front Left Right`.
    fn adjacent_position(
        &self,
        chr: &dyn PathMover,
        furniture: ObjectRef,
        direction: &str,
        side: &str,
    ) -> Option<(f32, f32, f32)>;
    /// `furniture.getSpriteGridObjectsExcludingSelf(list)`.
    fn sprite_grid_objects_excluding_self(&self, furniture: ObjectRef) -> Vec<ObjectRef>;
    /// `object.getObjectIndex() != -1`: the object is still on its square.
    fn object_exists(&self, object: ObjectRef) -> bool;
}

/// No seating data: no adjacent positions, no sprite grids, every object
/// present.
pub struct NoSeating;

impl SeatingHooks for NoSeating {
    fn adjacent_position(
        &self,
        _: &dyn PathMover,
        _: ObjectRef,
        _: &str,
        _: &str,
    ) -> Option<(f32, f32, f32)> {
        None
    }
    fn sprite_grid_objects_excluding_self(&self, _: ObjectRef) -> Vec<ObjectRef> {
        Vec::new()
    }
    fn object_exists(&self, _: ObjectRef) -> bool {
        true
    }
}

/// `IsoDeadBody` as `pathToGrabCorpse` sees it.
pub trait DeadBodyView {
    /// `getX()`, `getY()`, `getZ()`.
    fn position(&self) -> (f32, f32, f32);
    /// `isAnimal()`.
    fn is_animal(&self) -> bool;
    /// `AnimalDefinitions.getDef(getAnimalType()).collisionSize`, `None`
    /// without a definition.
    fn animal_collision_size(&self) -> Option<f32>;
    /// `getAnimalSize()`.
    fn animal_size(&self) -> f32;
    /// `getGrabHeadPosition(out)`.
    fn grab_head_position(&self) -> (f32, f32);
    /// `getGrabLegsPosition(out)`.
    fn grab_legs_position(&self) -> (f32, f32);
    /// `canBeGrabbedFrom(x, y)`.
    fn can_be_grabbed_from(&self, x: f32, y: f32) -> bool;
}

/// Looks up the goal character of `pathToCharacter` each frame.
pub trait CharacterLookup {
    /// The live character, or `None` once it is gone.
    fn character(&self, id: MoverId) -> Option<&dyn CharacterView>;
}

/// No other characters.
pub struct NoCharacters;

impl CharacterLookup for NoCharacters {
    fn character(&self, _: MoverId) -> Option<&dyn CharacterView> {
        None
    }
}
