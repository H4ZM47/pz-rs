//! The vehicle's light sources for the lighting system: headlights as
//! torches (`LightingJNI.checkTorch(part, id)` with
//! `IsoGameCharacter.TorchInfo.set(part)`), and the lightbar's flashing
//! lampposts (`BaseVehicle.updateWorldLights` →
//! `IsoCell.addLamppost`/`removeLamppost`).

use pz_vehicles::jmath::{fastfloor, Vector3f};
use pz_vehicles::{BaseVehicle, PartRef};

/// `IsoGameCharacter.TorchInfo` (what `LightingJNI.updateTorch` gets;
/// the native call is passed `z + 32`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Torch {
    pub id: i32,
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub angle_x: f32,
    pub angle_y: f32,
    pub dist: f32,
    pub strength: f32,
    pub cone: bool,
    pub dot: f32,
    pub focusing: i32,
}

/// `TorchInfo.set(VehiclePart)`, or `None` for a part without an active
/// light.
pub fn torch_for_part(part: &PartRef, id: i32) -> Option<Torch> {
    let light = part.st().light.clone()?;
    if !light.active {
        return None;
    }
    let v = part.vehicle();
    let script = v.script()?;
    let ext = script.borrow().extents();
    let w = v.world_pos(
        light.offset.x * ext.x / 2.0,
        0.0,
        light.offset.y * ext.z / 2.0,
    );
    let mut f: Vector3f = v.forward_vector();
    if part.id().is_some_and(|id| id.contains("Rear")) {
        f = Vector3f::new(-f.x, -f.y, -f.z);
    }
    Some(Torch {
        id,
        x: w.x,
        y: w.y,
        z: w.z,
        r: light.r,
        g: light.g,
        b: light.b,
        angle_x: f.x,
        angle_y: f.z,
        dist: part.light_distance(),
        strength: part.light_intensity(),
        cone: true,
        dot: light.dot,
        focusing: part.light_focusing() as i32,
    })
}

/// `LightingJNI.checkLights`' vehicle loop: a torch per active light part,
/// id `4096 + vehicleId * 10 + index`. Vehicles with id -1 add none.
pub fn vehicle_torches(v: &BaseVehicle) -> Vec<Torch> {
    let vid = v.id() as i32;
    if vid == -1 {
        return Vec::new();
    }
    (0..v.light_count())
        .filter_map(|j| {
            v.light_by_index(j as i32)
                .and_then(|p| torch_for_part(&p, 4096 + vid * 10 + j as i32))
        })
        .collect()
}

/// A lightbar lamppost (`leftLight1/2`, `rightLight1/2`): radius 8 at a
/// square.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LightbarLamp {
    pub x: i32,
    pub y: i32,
    pub z: i32,
    pub radius: i32,
    /// Left (`getWorldPos(0.4, 0, 0)`) or right (`-0.4`).
    pub left: bool,
}

/// The lamppost `updateWorldLights` keeps in the cell this frame, if any
/// (which of the two alternating light objects carries it does not change
/// what is lit).
pub fn lightbar_lamp(v: &BaseVehicle) -> Option<LightbarLamp> {
    let script = v.script()?;
    if !script.borrow().lightbar().enable {
        return None;
    }
    let mode = v.st().lightbar_lights_mode;
    if !mode.is_enable() || v.battery_charge() <= 0.0 || mode.light == 0 {
        return None;
    }
    let left = mode.light == 1;
    let p = v.world_pos(if left { 0.4 } else { -0.4 }, 0.0, 0.0);
    Some(LightbarLamp {
        x: fastfloor(p.x),
        y: fastfloor(p.y),
        z: fastfloor(v.z()),
        radius: 8,
        left,
    })
}
