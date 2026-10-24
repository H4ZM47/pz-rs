//! World inputs this crate cannot read from pz-world yet. Each hook has a
//! default that behaves as if the feature did not exist; they wait on the
//! world thread's item and object work (`IsoThumpable`, `IsoBarricade`,
//! door curtains, electricity and generators, light items).

use pz_lighting::jni::{Barricade, SpecialObject, TorchInfo};
use pz_world::{IsoGridSquare, IsoObject, SquareHooks};

/// World state outside pz-world. Extends pz-world's [`SquareHooks`]
/// (barricades and thumpables in the square queries).
pub trait WorldHooks: SquareHooks {
    /// `IsoThumpable`s and `IsoBarricade`s in `sq.getSpecialObjects()`, in
    /// list order, as lighting sees them. They are appended after the
    /// square's doors, windows and curtains (pz-world has neither yet).
    fn extra_special_objects(&self, _sq: &IsoGridSquare) -> Vec<SpecialObject> {
        Vec::new()
    }

    /// `getBarricadeOnSameSquare()` / `getBarricadeOnOppositeSquare()` of a
    /// door or window.
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
    /// power (`canSwitchLight()`, e.g. a generator).
    fn room_switch_has_power(&self, _x: i32, _y: i32, _z: i32) -> bool {
        false
    }

    /// Active torches this frame (players' light items, then vehicle
    /// lights), see `pz_lighting::jni::TorchInfo`.
    fn torches(&self) -> Vec<TorchInfo> {
        Vec::new()
    }
}

/// Every default: no thumpables, barricades, door curtains or torches,
/// power everywhere.
pub struct NoWorldHooks;

impl SquareHooks for NoWorldHooks {}
impl WorldHooks for NoWorldHooks {}
