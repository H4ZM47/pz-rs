//! World inputs lighting reads beyond the squares' own objects. Each hook
//! has a default that behaves as if the feature did not exist
//! ([`NoWorldHooks`]); [`pz_world::WorldState`] implements them from the
//! loaded cell's barricades, thumpables, doors, generators, light switches
//! and the players' light items.

use pz_lighting::jni::{Barricade, IsoDirection, SpecialObject, TorchInfo};
use pz_world::objects::IsoBarricade;
use pz_world::{IsoGridSquare, IsoObject, SquareHooks, WorldState};

/// World state outside pz-world's squares. Extends pz-world's
/// [`SquareHooks`] (barricades and thumpables in the square queries).
pub trait WorldHooks: SquareHooks {
    /// Special objects of `sq` lighting sees that pz-world does not model,
    /// appended after the square's own (doors, windows, curtains,
    /// thumpables and barricades come from `sq.special_objects`).
    fn extra_special_objects(&self, _sq: &IsoGridSquare) -> Vec<SpecialObject> {
        Vec::new()
    }

    /// `getBarricadeOnSameSquare()` / `getBarricadeOnOppositeSquare()` of a
    /// door, window or thumpable.
    fn barricades(
        &self,
        _sq: &IsoGridSquare,
        _obj: &IsoObject,
    ) -> (Option<Barricade>, Option<Barricade>) {
        (None, None)
    }

    /// `IsoDoor.HasCurtains()`: `Some(curtainOpen)` when the door has one.
    fn door_curtain(&self, _sq: &IsoGridSquare, _obj: &IsoObject) -> Option<bool> {
        None
    }

    /// `!IsoWorld.isHydroPowerOn()`: the grid has been shut off.
    fn hydro_power_off(&self) -> bool {
        false
    }

    /// A light switch at `(x, y, z)` cannot light: `!canSwitchLight()`
    /// (no grid power and no generator, no bulb, flat battery), or for a
    /// street light `!hasGridPower()`.
    fn light_switch_unpowered(&self, _x: i32, _y: i32, _z: i32) -> bool {
        false
    }

    /// With the grid off: a room light switch at `(x, y, z)` still has
    /// power (`lightSwitch.square.haveElectricity()`, a generator).
    fn room_switch_has_power(&self, _x: i32, _y: i32, _z: i32) -> bool {
        false
    }

    /// Active torches this frame (players' light items, then vehicle
    /// lights), see `pz_lighting::jni::TorchInfo`.
    fn torches(&self) -> Vec<TorchInfo> {
        Vec::new()
    }
}

/// Every default: no barricades, door curtains or torches, power
/// everywhere.
pub struct NoWorldHooks;

impl SquareHooks for NoWorldHooks {}
impl WorldHooks for NoWorldHooks {}

/// An `IsoBarricade` as lighting reads it.
pub fn lighting_barricade(b: &IsoBarricade) -> Barricade {
    Barricade {
        dir: Some(IsoDirection::ALL[b.forward_dir() as usize]),
        metal_health: b.metal_health,
        metal_bar: b.is_metal_bar(),
        planks: b.num_planks(),
    }
}

impl WorldHooks for WorldState<'_> {
    fn barricades(
        &self,
        sq: &IsoGridSquare,
        obj: &IsoObject,
    ) -> (Option<Barricade>, Option<Barricade>) {
        let Some(north) = pz_world::world_state::barricade_able_north(obj) else {
            return (None, None);
        };
        let (a, b) = self.barricades_for(sq, north);
        (a.map(lighting_barricade), b.map(lighting_barricade))
    }

    fn door_curtain(&self, _sq: &IsoGridSquare, obj: &IsoObject) -> Option<bool> {
        WorldState::door_curtain(self, obj)
    }

    fn hydro_power_off(&self) -> bool {
        !self.power.hydro_power_on
    }

    fn light_switch_unpowered(&self, x: i32, y: i32, z: i32) -> bool {
        !self.lamp_switch_has_power(x, y, z)
    }

    fn room_switch_has_power(&self, x: i32, y: i32, z: i32) -> bool {
        self.room_switch_square_powered(x, y, z)
    }

    /// The players' light items. STUB: vehicle lights
    /// (`TorchInfo.set(VehiclePart)`) are not included.
    fn torches(&self) -> Vec<TorchInfo> {
        self.player_torches()
            .into_iter()
            .map(|t| TorchInfo {
                id: t.id,
                x: t.x,
                y: t.y,
                z: t.z,
                r: t.r,
                g: t.g,
                b: t.b,
                angle_x: t.angle_x,
                angle_y: t.angle_y,
                dist: t.dist,
                strength: t.strength,
                cone: t.cone,
                dot: t.dot,
                focusing: t.focusing,
            })
            .collect()
    }
}
