//! What the square queries, lighting and rendering read from the objects
//! and items of a loaded cell: barricades (`IsoBarricade`), player-built
//! doors and windows (`IsoThumpable`), door curtains (`IsoDoor`), power
//! (`IsoGenerator` range, the power grid, `IsoLightSwitch.canSwitchLight`)
//! and light-emitting items (`LightingJNI.checkPlayerTorches`).
//!
//! [`WorldState`] answers them over an [`IsoCell`]; sandbox options and the
//! world's power state are runtime input ([`PowerSettings`]).

use pz_inventory::item::InventoryItem;

use crate::chunk::{IsoCell, IsoGridSquare};
use crate::meta_grid::IsoMetaGrid;
use crate::objects::{IsoBarricade, IsoLightSwitch, IsoObject, IsoThumpable, ObjectKind};
use crate::sprite::IsoSpriteManager;
use crate::square::{EdgeFacing, IsoDirections, SquareHooks};

// `IsoGenerator.GeneratorMinZ`/`GeneratorMaxZ` are the world's level range.
use crate::chunk::{MAX_LEVEL, MIN_LEVEL};

impl IsoBarricade {
    /// `getForwardIsoDirection()` (`getDir()`).
    pub fn forward_dir(&self) -> IsoDirections {
        IsoDirections::ALL[(self.dir as usize) & 7]
    }

    /// `getNumPlanks()`: planks with health left.
    pub fn num_planks(&self) -> i32 {
        self.plank_health.iter().filter(|&&h| h > 0).count() as i32
    }

    /// `isMetal()`.
    pub fn is_metal(&self) -> bool {
        self.metal_health > 0
    }

    /// `isMetalBar()`.
    pub fn is_metal_bar(&self) -> bool {
        self.metal_bar_health > 0
    }

    /// `isBlockVision()`: metal sheet or more than two planks.
    pub fn is_block_vision(&self) -> bool {
        self.is_metal() || self.num_planks() > 2
    }

    /// `isDestroyed()`.
    pub fn is_destroyed(&self) -> bool {
        self.metal_health <= 0 && self.num_planks() <= 0 && self.metal_bar_health <= 0
    }

    /// `canAddPlank()`.
    pub fn can_add_plank(&self) -> bool {
        !self.is_metal() && self.num_planks() < 4 && !self.is_metal_bar()
    }

    /// `getLightTransmission()`.
    pub fn light_transmission(&self) -> f32 {
        if self.is_metal() {
            0.0
        } else if self.is_metal_bar() {
            1.0
        } else {
            1.0 - self.num_planks() as f32 * 0.15
        }
    }
}

impl IsoThumpable {
    /// `isBlockedDoor()` given `isBarricaded()`.
    pub fn is_blocked_door(&self, barricaded: bool) -> bool {
        self.is_door && (!self.open || barricaded)
    }
}

/// `IsoBarricade.GetBarricadeOnSquare(square, dir)`: the first barricade in
/// `specialObjects` facing `dir`.
pub fn get_barricade_on_square(
    sq: Option<&IsoGridSquare>,
    dir: IsoDirections,
) -> Option<&IsoBarricade> {
    let sq = sq?;
    sq.special_objects.iter().find_map(|&id| {
        let o = &sq.objects[sq.object_index(id)?];
        match &o.kind {
            ObjectKind::Barricade(b) if b.forward_dir() == dir => Some(b),
            _ => None,
        }
    })
}

/// `getNorth()` of a `BarricadeAble` (`IsoDoor`, `IsoWindow`,
/// `IsoThumpable`); `None` for other objects.
pub fn barricade_able_north(obj: &IsoObject) -> Option<bool> {
    match &obj.kind {
        ObjectKind::Door(d) => Some(d.north),
        ObjectKind::Window(w) => Some(w.north),
        ObjectKind::Thumpable(t) => Some(t.north),
        _ => None,
    }
}

/// `IsoGenerator.isPoweringSquare(generatorX, generatorY, generatorZ, x, y,
/// z)` for the sandbox generator range in `p`.
pub fn is_powering_square(p: &PowerSettings, g: (i32, i32, i32), x: i32, y: i32, z: i32) -> bool {
    let min_z = MIN_LEVEL.max(g.2 - p.generator_vertical_power_range);
    let max_z = MAX_LEVEL.min(g.2 + p.generator_vertical_power_range);
    if z < min_z || z > max_z {
        return false;
    }
    let dx = (g.0 as f32 + 0.5) - (x as f32 + 0.5);
    let dy = (g.1 as f32 + 0.5) - (y as f32 + 0.5);
    let r = p.generator_tile_range;
    dx * dx + dy * dy <= (r * r) as f32
}

/// World and sandbox values the power queries read (runtime input).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PowerSettings {
    /// `IsoWorld.isHydroPowerOn()` (set by the ambient sound manager from
    /// `doesPowerGridExist()`).
    pub hydro_power_on: bool,
    /// `IsoWorld.getWorldAgeDays()`.
    pub world_age_days: f32,
    /// `SandboxOptions.getElecShutModifier()`.
    pub elec_shut_modifier: i32,
    /// Sandbox `AllowExteriorGenerator`.
    pub allow_exterior_generator: bool,
    /// `IsoGenerator.generatorRadius`: sandbox `GeneratorTileRange` once a
    /// generator object exists.
    pub generator_tile_range: i32,
    /// `IsoGenerator.generatorVerticalPowerRange`: sandbox
    /// `GeneratorVerticalPowerRange`.
    pub generator_vertical_power_range: i32,
}

impl PowerSettings {
    /// `SandboxOptions.doesPowerGridExist(offset)`.
    pub fn does_power_grid_exist(&self, offset: i32) -> bool {
        self.world_age_days <= (self.elec_shut_modifier + offset) as f32
    }
}

/// A player's light sources for `LightingJNI.checkPlayerTorches`.
#[derive(Clone, Debug)]
pub struct PlayerLights<'a> {
    /// `IsoPlayer.getPlayerNum()` (torch ids are `index * 4 + i + 1`).
    pub index: i32,
    /// `getX()`, `getY()`, `getZ()` (world level).
    pub x: f32,
    pub y: f32,
    pub z: f32,
    /// `getLookVector()`.
    pub look_x: f32,
    pub look_y: f32,
    /// `getSecondaryHandItem()`, `getPrimaryHandItem()`, then
    /// `getAttachedItems()` in order (`getActiveLightItems` order).
    pub items: Vec<&'a InventoryItem>,
}

/// `IsoGameCharacter.TorchInfo.set(player, item)`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ItemTorch {
    pub id: i32,
    pub x: f32,
    pub y: f32,
    /// World level (not `+ 32`).
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

impl PlayerLights<'_> {
    /// `getActiveLightItems`: the items with `isEmittingLight()`, each once.
    pub fn active_light_items(&self) -> Vec<&InventoryItem> {
        let mut out: Vec<&InventoryItem> = Vec::new();
        for &it in &self.items {
            if it.is_emitting_light() && !out.iter().any(|o| std::ptr::eq(*o, it)) {
                out.push(it);
            }
        }
        out
    }

    /// `checkPlayerTorches`: up to four torches with ids
    /// `index * 4 + i + 1` from the active light items.
    pub fn torches(&self) -> Vec<ItemTorch> {
        self.active_light_items()
            .into_iter()
            .take(4)
            .enumerate()
            .map(|(i, item)| ItemTorch {
                id: self.index * 4 + i as i32 + 1,
                x: self.x,
                y: self.y,
                z: self.z,
                r: 1.0,
                g: 0.823_529_4,
                b: 0.705_882_4,
                angle_x: self.look_x,
                angle_y: self.look_y,
                dist: item.get_light_distance() as f32,
                strength: item.get_light_strength(),
                cone: item.is_torch_cone(),
                dot: item.get_torch_dot(),
                focusing: 0,
            })
            .collect()
    }
}

/// The loaded world as the square, lighting and render queries see it.
pub struct WorldState<'a> {
    pub cell: &'a IsoCell,
    pub meta: &'a IsoMetaGrid,
    pub sprites: &'a IsoSpriteManager,
    pub power: PowerSettings,
    /// STUB: `IsoMetaGrid.getZonesAt(x, y, 0)` has a `NoPower` or
    /// `NoPowerOrWater` zone (zones are not ported; runtime input).
    pub no_power_zone: &'a dyn Fn(i32, i32) -> bool,
    /// The local players' light items, in player order.
    pub players: Vec<PlayerLights<'a>>,
}

fn no_zone(_: i32, _: i32) -> bool {
    false
}

impl<'a> WorldState<'a> {
    /// A state without `NoPower` zones or players.
    pub fn new(
        cell: &'a IsoCell,
        meta: &'a IsoMetaGrid,
        sprites: &'a IsoSpriteManager,
        power: PowerSettings,
    ) -> Self {
        WorldState {
            cell,
            meta,
            sprites,
            power,
            no_power_zone: &no_zone,
            players: Vec::new(),
        }
    }

    fn square(&self, x: i32, y: i32, z: i32) -> Option<&'a IsoGridSquare> {
        self.cell.get_grid_square(x, y, z)
    }

    /// `getOppositeSquare()` of an edge element on `sq` (`getNorth()` or
    /// `getWest()` of the square).
    pub fn opposite_square(&self, sq: &IsoGridSquare, north: bool) -> Option<&'a IsoGridSquare> {
        if north {
            self.square(sq.x, sq.y - 1, sq.z)
        } else {
            self.square(sq.x - 1, sq.y, sq.z)
        }
    }

    /// `getBarricadeOnSameSquare()` and `getBarricadeOnOppositeSquare()` of
    /// an edge element on `sq` facing `north`.
    pub fn barricades_for<'s>(
        &self,
        sq: &'s IsoGridSquare,
        north: bool,
    ) -> (Option<&'s IsoBarricade>, Option<&'a IsoBarricade>) {
        let (same, opposite) = if north {
            (IsoDirections::N, IsoDirections::S)
        } else {
            (IsoDirections::W, IsoDirections::E)
        };
        (
            get_barricade_on_square(Some(sq), same),
            get_barricade_on_square(self.opposite_square(sq, north), opposite),
        )
    }

    /// `isBarricaded()` of a door, window or thumpable on `sq`.
    pub fn object_barricaded(&self, sq: &IsoGridSquare, obj: &IsoObject) -> bool {
        let Some(north) = barricade_able_north(obj) else {
            return false;
        };
        let (a, b) = self.barricades_for(sq, north);
        a.is_some() || b.is_some()
    }

    /// `IsoGridSquare.hasBlockedDoor(facing)` restricted to `IsoThumpable`:
    /// a thumpable in `objects` with `isBlockedDoor(facing)`.
    pub fn thumpable_door_blocked(&self, sq: &IsoGridSquare, facing: EdgeFacing) -> bool {
        sq.objects.iter().any(|o| match &o.kind {
            ObjectKind::Thumpable(t) => {
                EdgeFacing::of_north(t.north) == facing
                    && t.is_blocked_door(self.object_barricaded(sq, o))
            }
            _ => false,
        })
    }

    /// `IsoThumpable.isWindow()` (`IsoObject.isWindow`): the sprite has
    /// `WindowN` or `WindowW`.
    pub fn object_is_window(&self, obj: &IsoObject) -> bool {
        let f = &self.sprites.tables.flag;
        obj.sprite.is_some_and(|s| {
            let p = &self.sprites.sprites[s].properties;
            p.has(f.WindowN) || p.has(f.WindowW)
        })
    }

    /// `IsoDoor.HasCurtains() != null`, as `Some(isCurtainOpen())`.
    pub fn door_curtain(&self, obj: &IsoObject) -> Option<bool> {
        match &obj.kind {
            ObjectKind::Door(d) if d.has_curtain => Some(d.curtain_open),
            _ => None,
        }
    }

    // Power.

    /// `IsoGridSquare.isNoPower()`: derelict or user-defined room, or a
    /// `NoPower` zone.
    pub fn is_no_power(&self, sq: &IsoGridSquare) -> bool {
        if let Some(room) = sq.room_def(self.meta) {
            if room.name.to_lowercase().contains("derelict") || room.user_defined {
                return true;
            }
        }
        (self.no_power_zone)(sq.x, sq.y)
    }

    /// `IsoGridSquare.hasGridPower()`.
    pub fn has_grid_power(&self, sq: &IsoGridSquare) -> bool {
        !self.is_no_power(sq) && self.power.does_power_grid_exist(0)
    }

    /// `IsoChunk.isGeneratorPoweringSquare(x, y, z)` of the chunk holding
    /// `(x, y)`: one of its `generatorsTouchingThisChunk` reaches it.
    pub fn is_generator_powering_square(&self, x: i32, y: i32, z: i32) -> bool {
        let Some(ch) = self.cell.chunks.get(&(x.div_euclid(8), y.div_euclid(8))) else {
            return false;
        };
        ch.saved.generators.as_ref().is_some_and(|gens| {
            gens.iter()
                .any(|&(gx, gy, gz)| is_powering_square(&self.power, (gx, gy, gz as i32), x, y, z))
        })
    }

    /// `IsoGridSquare.haveElectricity()`: a generator powers it (exterior
    /// squares only when the sandbox allows exterior generators).
    pub fn have_electricity(&self, sq: &IsoGridSquare) -> bool {
        if !self.power.allow_exterior_generator && sq.is_exterior(self.sprites) {
            return false;
        }
        self.is_generator_powering_square(sq.x, sq.y, sq.z)
    }

    /// `IsoLightSwitch.isBuildingSquare(square)`.
    fn is_building_square(&self, sq: Option<&IsoGridSquare>) -> bool {
        sq.is_some_and(|sq| sq.room_def(self.meta).is_some() || sq.roof_hide_building.is_some())
    }

    /// STUB: `IsoLightSwitch.hasFasciaAdjacentToBuildingSquare(square)`
    /// (`IsoObject.isFascia` and `getFasciaAttachedSquare` are not ported):
    /// always false.
    fn has_fascia_adjacent_to_building_square(&self, _sq: &IsoGridSquare) -> bool {
        false
    }

    /// `IsoLightSwitch.hasElectricityAround()` for switch `sw` on `sq`.
    pub fn has_electricity_around(&self, sq: &IsoGridSquare, sw: &IsoLightSwitch) -> bool {
        let pre_elec_shut = self.has_grid_power(sq);
        let around = if pre_elec_shut {
            self.is_building_square(Some(sq))
                || self.has_fascia_adjacent_to_building_square(sq)
                || sw.street_light
        } else {
            self.have_electricity(sq)
        };
        if around {
            return true;
        }
        let mut dz = 0;
        while dz >= -1 && sq.z + dz >= MIN_LEVEL {
            for dx in -1..2 {
                for dy in -1..2 {
                    if dx == 0 && dy == 0 && dz == 0 {
                        continue;
                    }
                    let Some(gs) = self.square(sq.x + dx, sq.y + dy, sq.z + dz) else {
                        continue;
                    };
                    if pre_elec_shut
                        && (self.is_building_square(Some(gs))
                            || self.has_fascia_adjacent_to_building_square(gs))
                    {
                        return true;
                    }
                    if self.have_electricity(gs) {
                        return true;
                    }
                }
            }
            dz -= 1;
        }
        false
    }

    /// `IsoLightSwitch.canSwitchLight()` for switch `sw` on `sq`.
    pub fn can_switch_light(&self, sq: &IsoGridSquare, sw: &IsoLightSwitch) -> bool {
        if sw.bulb_item.is_none() {
            return false;
        }
        let around = self.has_electricity_around(sq, sw);
        !sw.use_battery && around
            || sw.can_be_modified && sw.use_battery && sw.has_battery && sw.power > 0.0
    }

    /// The light switch on `(x, y, z)` controlling a lamp
    /// (`IsoLightSource.switches.get(0)`): the first non-room switch there,
    /// else the first switch.
    pub fn lamp_switch(
        &self,
        x: i32,
        y: i32,
        z: i32,
    ) -> Option<(&'a IsoGridSquare, &'a IsoLightSwitch)> {
        let sq = self.square(x, y, z)?;
        let switches = || {
            sq.objects.iter().filter_map(|o| match &o.kind {
                ObjectKind::LightSwitch(s) => Some(s),
                _ => None,
            })
        };
        let sw = switches()
            .find(|s| !s.light_room)
            .or_else(|| switches().next())?;
        Some((sq, sw))
    }

    /// `checkLights`' power for a lamp switched at `(x, y, z)`:
    /// `canSwitchLight()`, and for a street light also `hasGridPower()`
    /// (the night test is the caller's). A missing switch has no power.
    pub fn lamp_switch_has_power(&self, x: i32, y: i32, z: i32) -> bool {
        let Some((sq, sw)) = self.lamp_switch(x, y, z) else {
            return false;
        };
        let mut has_power = self.can_switch_light(sq, sw);
        if sw.street_light && !self.has_grid_power(sq) {
            has_power = false;
        }
        has_power
    }

    /// `checkLights`' room-light test with the grid off:
    /// `lightSwitch.square.haveElectricity()` for a switch at `(x, y, z)`.
    pub fn room_switch_square_powered(&self, x: i32, y: i32, z: i32) -> bool {
        self.square(x, y, z)
            .is_some_and(|sq| self.have_electricity(sq))
    }

    /// The players' torches (`checkPlayerTorches` for each player).
    pub fn player_torches(&self) -> Vec<ItemTorch> {
        self.players.iter().flat_map(|p| p.torches()).collect()
    }
}

impl SquareHooks for WorldState<'_> {
    fn is_barricaded(&self, sq: &IsoGridSquare, obj: &IsoObject) -> bool {
        self.object_barricaded(sq, obj)
    }

    fn has_blocked_thumpable_door(&self, sq: &IsoGridSquare, facing: EdgeFacing) -> bool {
        self.thumpable_door_blocked(sq, facing)
    }
}

#[cfg(test)]
mod tests;
