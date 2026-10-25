//! The vehicle's light sources for the lighting system: headlights as
//! torches (`LightingJNI.checkTorch(part, id)` with
//! `IsoGameCharacter.TorchInfo.set(part)`), and the lightbar's flashing
//! lampposts (`BaseVehicle.updateWorldLights` →
//! `IsoCell.addLamppost`/`removeLamppost`).
//!
//! Wiring: `LightingJNI.checkLights` sends the players' light items, then
//! [`cell_torches`]; with pz-world-render that is the tail of
//! `WorldHooks::torches`. Each vehicle's [`LightbarWorldLights`] is
//! updated from `BaseVehicle.update` and removed with the vehicle, against
//! the cell's light list on [`WorldLighting`].

use pz_vehicles::jmath::{fastfloor, Vector3f};
use pz_vehicles::{BaseVehicle, PartRef};
use pz_world_render::{CellLight, WorldLighting};

/// `IsoGameCharacter.TorchInfo`: the lighting library's torch input
/// (`z` is the world level; pz-lighting passes `z + 32` to the native
/// `updateTorch` as `LightingJNI.checkTorch` does).
pub use pz_lighting::jni::TorchInfo as Torch;

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

/// `LightingJNI.checkLights`' loop over `IsoCell.getVehicles()`: every
/// vehicle's [`vehicle_torches`], in cell order. These follow the players'
/// torches in the list given to `LightSync::check_lights`.
pub fn cell_torches<'a>(vehicles: impl IntoIterator<Item = &'a BaseVehicle>) -> Vec<Torch> {
    vehicles.into_iter().flat_map(vehicle_torches).collect()
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

/// One of `BaseVehicle`'s four lightbar `IsoLightSource`s: its fields and,
/// while it is in the cell's light list, its key there.
#[derive(Clone, Copy, Debug, PartialEq)]
struct BarLight {
    light: CellLight,
    key: Option<u64>,
}

/// `BaseVehicle.leftLight1/2`, `rightLight1/2` and `leftLightIndex`/
/// `rightLightIndex`: the lightbar's world lights, which
/// `updateWorldLights` alternates between so that a moved light is a new
/// lamppost while the old one is removed.
#[derive(Clone, Debug, PartialEq)]
pub struct LightbarWorldLights {
    left: [BarLight; 2],
    right: [BarLight; 2],
    /// `leftLightIndex`/`rightLightIndex`: 1 or 2, -1 for none.
    pub left_index: i32,
    pub right_index: i32,
}

impl LightbarWorldLights {
    /// The field initialisers (`new IsoLightSource(0, 0, 0, 0, 0, 1, 8)`
    /// left, `(…, 1, 0, 0, 8)` right) with `createParts`' siren colours
    /// when the vehicle has a lightbar and both colours.
    pub fn new(v: &BaseVehicle) -> Self {
        let bar = |r, g, b| BarLight {
            light: CellLight::new(0, 0, 0, r, g, b, 8),
            key: None,
        };
        let mut s = LightbarWorldLights {
            left: [bar(0.0, 0.0, 1.0); 2],
            right: [bar(1.0, 0.0, 0.0); 2],
            left_index: -1,
            right_index: -1,
        };
        if let Some(script) = v.script().filter(|_| v.has_lightbar()) {
            let sc = script.borrow();
            if let (Some(l), Some(r)) = (sc.left_siren_col, sc.right_siren_col) {
                for b in &mut s.left {
                    (b.light.r, b.light.g, b.light.b) = (l.r, l.g, l.b);
                }
                for b in &mut s.right {
                    (b.light.r, b.light.g, b.light.b) = (r.r, r.g, r.b);
                }
            }
        }
        s
    }

    /// The lamppost keys currently in the cell's list (left, right).
    pub fn keys(&self) -> (Option<u64>, Option<u64>) {
        let k = |side: &[BarLight; 2], i: i32| {
            if i == 1 || i == 2 {
                side[i as usize - 1].key
            } else {
                None
            }
        };
        (
            k(&self.left, self.left_index),
            k(&self.right, self.right_index),
        )
    }

    /// `BaseVehicle.updateWorldLights()`.
    pub fn update(&mut self, v: &BaseVehicle, cell: &mut WorldLighting) {
        let enabled = v.script().is_some_and(|s| s.borrow().lightbar().enable);
        let mode = v.st().lightbar_lights_mode;
        if !enabled || !mode.is_enable() || v.battery_charge() <= 0.0 || mode.light == 0 {
            self.remove(cell);
            return;
        }
        for b in self.left.iter_mut().chain(self.right.iter_mut()) {
            b.light.radius = 8;
        }
        let left = mode.light == 1;
        let p = v.world_pos(if left { 0.4 } else { -0.4 }, 0.0, 0.0);
        let (lx, ly, lz) = (fastfloor(p.x), fastfloor(p.y), fastfloor(v.z()));
        let old = if left {
            self.left_index
        } else {
            self.right_index
        };
        let side = if left { &self.left } else { &self.right };
        if old == 1 || old == 2 {
            let l = &side[old as usize - 1].light;
            if (l.x, l.y, l.z) == (lx, ly, lz) {
                return;
            }
        }
        self.remove(cell);
        let (side, index) = if left {
            (&mut self.left, &mut self.left_index)
        } else {
            (&mut self.right, &mut self.right_index)
        };
        *index = if old == 1 { 2 } else { 1 };
        let b = &mut side[*index as usize - 1];
        b.light.x = lx;
        b.light.y = ly;
        b.light.z = lz;
        // `life = -1`, then `IsoCell.addLamppost`. (The original skips the
        // add when the same object is still listed, dead, from a removal
        // this frame; that light then lives on. Here the dead entry leaves
        // and a new one is added, which lights the same squares.)
        b.key = Some(cell.add_lamppost(b.light));
    }

    /// `BaseVehicle.removeWorldLights()` (also on `removeFromWorld`).
    pub fn remove(&mut self, cell: &mut WorldLighting) {
        for (side, index) in [
            (&mut self.left, &mut self.left_index),
            (&mut self.right, &mut self.right_index),
        ] {
            if *index == 1 || *index == 2 {
                if let Some(k) = side[*index as usize - 1].key.take() {
                    cell.remove_lamppost(k);
                }
                *index = -1;
            }
        }
    }
}
