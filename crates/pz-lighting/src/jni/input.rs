//! Plain-data mirrors of what the Java side reads from the world
//! (`IsoGridSquare`, its special objects, `IsoChunk`, `IsoLightSource`,
//! `IsoRoomLight`, `IsoGameCharacter.TorchInfo`). The world crate fills
//! these; nothing here refers to world types.

/// `IsoDirections`, in ordinal order (the order `updateChunk` packs the
/// vision bits in and `ForcedVis` is indexed by).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum IsoDirection {
    N = 0,
    NW = 1,
    W = 2,
    SW = 3,
    S = 4,
    SE = 5,
    E = 6,
    NE = 7,
}

impl IsoDirection {
    /// `IsoDirections.values()`.
    pub const ALL: [IsoDirection; 8] = [
        IsoDirection::N,
        IsoDirection::NW,
        IsoDirection::W,
        IsoDirection::SW,
        IsoDirection::S,
        IsoDirection::SE,
        IsoDirection::E,
        IsoDirection::NE,
    ];

    /// `IsoDirections.dx()/dy()`.
    pub fn delta(self) -> (i32, i32) {
        match self {
            IsoDirection::N => (0, -1),
            IsoDirection::NW => (-1, -1),
            IsoDirection::W => (-1, 0),
            IsoDirection::SW => (-1, 1),
            IsoDirection::S => (0, 1),
            IsoDirection::SE => (1, 1),
            IsoDirection::E => (1, 0),
            IsoDirection::NE => (1, -1),
        }
    }
}

/// One square as the lighting code sees it (`IsoGridSquare`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct GridSquare {
    /// `IsoGridSquare.x`, `.y`, `.z` (world level, not `+ 32`).
    pub x: i32,
    pub y: i32,
    pub z: i32,
    /// `IsoGridSquare.visionMatrix`: bit `(dx+1) + 3 (dy+1) + 9 (dz+1)` set
    /// when vision to that neighbour is blocked by walls.
    pub vision_matrix: i32,
    /// `getProperties().has(IsoFlagType.exterior)`.
    pub exterior: bool,
    /// `HasElevatedFloor()`: has a `stairsTN`, `stairsMN`, `stairsTW` or
    /// `stairsMW` object.
    pub elevated_floor: bool,
    /// `getBuildingDef().getID()`, `None` outside buildings.
    pub building_id: Option<i64>,
    /// `getRoomID()` when `getRoom() != null`, else `None`.
    pub room_id: Option<i64>,
    /// `IsoGridSquare.lightLevel`: the last propagated light level the
    /// decoder returned for this square (see
    /// [`crate::jni::SquareLight::light_level`]). Sent as is for squares that
    /// are not open air.
    pub light_level: i32,
    /// `isSeen(playerIndex)`, one bit per player (the decoder's `seen` bit).
    pub seen_by: u8,
    /// `getSpecialObjects()`, in list order.
    pub special_objects: Vec<SpecialObject>,
}

/// The special objects lighting looks at.
#[derive(Clone, Debug, PartialEq)]
pub enum SpecialObject {
    Curtain(Curtain),
    Door(Door),
    Thumpable(Thumpable),
    Window(Window),
    Barricade(Barricade),
}

/// Which `IsoObjectType` a curtain is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CurtainType {
    /// `curtainW`
    W,
    /// `curtainN`
    N,
    /// `curtainE`
    E,
    /// `curtainS`
    S,
    /// Any other type (never sent as a side).
    Other,
}

/// `IsoCurtain`.
#[derive(Clone, Debug, PartialEq)]
pub struct Curtain {
    /// `getType()`.
    pub kind: CurtainType,
    /// `IsoCurtain.north`.
    pub north: bool,
    /// `IsoCurtain.open` (`IsOpen()`).
    pub open: bool,
    /// Sprite properties `LightFilterR/G/B`, `LightFilterIntensity`,
    /// `LightFilterMix`, raw strings (`PZMath.tryParseInt` semantics).
    pub light_filter_r: Option<String>,
    pub light_filter_g: Option<String>,
    pub light_filter_b: Option<String>,
    pub light_filter_intensity: Option<String>,
    pub light_filter_mix: Option<String>,
}

/// `IsoBarricade` (also referenced from doors and windows through
/// `getBarricadeOnSameSquare()` / `getBarricadeOnOppositeSquare()`).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Barricade {
    /// `getForwardIsoDirection()`.
    pub dir: Option<IsoDirection>,
    /// `metalHealth`.
    pub metal_health: i32,
    /// `metalBarHealth > 0` (`isMetalBar()`).
    pub metal_bar: bool,
    /// `getNumPlanks()`.
    pub planks: i32,
}

impl Barricade {
    /// `isBlockVision()`.
    pub fn block_vision(&self) -> bool {
        self.metal_health > 0 || self.planks > 2
    }

    /// `getLightTransmission()`.
    pub fn light_transmission(&self) -> f32 {
        if self.metal_health > 0 {
            0.0
        } else if self.metal_bar {
            1.0
        } else {
            1.0 - self.planks as f32 * 0.15
        }
    }
}

/// `IsoDoor`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Door {
    /// `IsoDoor.north`.
    pub north: bool,
    /// `isOpen()`.
    pub open: bool,
    /// Sprite property `doorTrans`.
    pub door_trans: bool,
    /// Sprite property `GarageDoor` (`getGarageDoorIndex() != -1`).
    pub garage_door: bool,
    /// `hasCurtain` (`HasCurtains() != null`).
    pub has_curtain: bool,
    /// `curtainOpen`.
    pub curtain_open: bool,
    /// `getBarricadeOnSameSquare()`.
    pub barricade_same: Option<Barricade>,
    /// `getBarricadeOnOppositeSquare()`.
    pub barricade_opposite: Option<Barricade>,
}

/// `IsoThumpable`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Thumpable {
    /// `IsoThumpable.north`.
    pub north: bool,
    /// `IsoThumpable.open`.
    pub open: bool,
    /// `isDoor()`.
    pub is_door: bool,
    /// `isWindow()`.
    pub is_window: bool,
    /// `canPassThrough`.
    pub can_pass_through: bool,
    /// Sprite property `doorTrans`.
    pub door_trans: bool,
    pub barricade_same: Option<Barricade>,
    pub barricade_opposite: Option<Barricade>,
}

/// `IsoWindow`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Window {
    /// `IsoWindow.north` (`isNorth()`).
    pub north: bool,
    /// `IsOpen()`.
    pub open: bool,
    pub barricade_same: Option<Barricade>,
    pub barricade_opposite: Option<Barricade>,
}

/// Read access to the cell's squares (`IsoCell.getGridSquare`).
pub trait LightingWorld {
    /// The square at world `(x, y, z)`, if loaded.
    fn square(&self, x: i32, y: i32, z: i32) -> Option<&GridSquare>;
}

/// One loaded chunk of a player's chunk map (`IsoChunk`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ChunkInfo {
    /// `IsoChunk.wx`, `.wy`.
    pub wx: i32,
    pub wy: i32,
    /// `getMinLevel()`, `getMaxLevel()` (world levels).
    pub min_level: i32,
    pub max_level: i32,
    /// `IsoChunk.loaded`.
    pub loaded: bool,
    /// `IsoChunk.lightCheck[playerIndex]`: the chunk has changed.
    pub light_check: bool,
    /// `getLevelData(z).lightCheck[playerIndex]` for `z` in
    /// `min_level..=max_level`.
    pub level_light_check: Vec<bool>,
}

/// `IsoLightSource` as `checkLights` sees it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LightSource {
    /// A stable handle chosen by the world (the Java object identity).
    pub key: u64,
    /// World position (level not `+ 32`).
    pub x: i32,
    pub y: i32,
    pub z: i32,
    pub radius: i32,
    pub r: f32,
    pub g: f32,
    pub b: f32,
    /// `active`, after the world's power/switch logic.
    pub active: bool,
    /// `life`: -1 for a permanent light, else frames left (temporary).
    pub life: i32,
    /// `localToBuilding.id`, `None` when null.
    pub local_to_building: Option<i32>,
    /// `life != 0 && isInBounds()` (and the light's chunk is still the one
    /// at its position). A light that is no longer alive is removed.
    pub alive: bool,
}

/// `IsoRoomLight` as `checkLights` sees it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RoomLightSource {
    /// A stable handle chosen by the world.
    pub key: u64,
    /// `room.building.def.id`.
    pub building_id: i64,
    /// `room.def.id`.
    pub room_id: i64,
    pub x: i32,
    pub y: i32,
    /// World level.
    pub z: i32,
    pub width: i32,
    pub height: i32,
    /// `active` after the world's power logic (`room.def.lightsActive`).
    pub active: bool,
    /// `isInBounds()`.
    pub in_bounds: bool,
}

/// `IsoGameCharacter.TorchInfo` for an active light item or vehicle light.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct TorchInfo {
    /// Player torches `playerIndex * 4 + i + 1`, vehicle lights
    /// `4096 + vehicleId * 10 + j` (see [`crate::jni::player_torch_id`]).
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

/// `RenderSettings.PlayerRenderSettings`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PlayerRenderSettings {
    pub rmod: f32,
    pub gmod: f32,
    pub bmod: f32,
    pub ambient: f32,
    pub night: f32,
    pub view_distance: f32,
}

impl PlayerRenderSettings {
    /// `GameTime.getSkyLightLevel()` for these settings: `b | g << 8 |
    /// r << 16` (byte 0 is blue, which the library reads as red).
    pub fn sky_light_level(&self) -> i32 {
        let clamp = |v: f32| v.clamp(0.0, 1.0);
        let r = clamp(self.rmod * 2.0 * self.ambient);
        let g = clamp(self.gmod * 2.0 * self.ambient);
        let b = clamp(self.bmod * 2.0 * self.ambient);
        let byte = |v: f32| (v.min(1.0) * 255.0) as i32;
        byte(b) | (byte(g) << 8) | (byte(r) << 16)
    }
}

/// What `updatePlayer` reads from `IsoPlayer`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PlayerInput {
    /// `getX()`, `getY()`, `getZ()` (world level).
    pub x: f32,
    pub y: f32,
    pub z: f32,
    /// `getLookVector()`.
    pub look_x: f32,
    pub look_y: f32,
    /// The look vector is rotated by pi (driving backwards: in a vehicle,
    /// not aiming, not looking, driver, speed < -1 km/h).
    pub reversing_driver: bool,
    /// `reanimatedCorpse != null`.
    pub reanimated: bool,
    /// `isGhostMode()`.
    pub ghost: bool,
    /// `hasTrait(SHORT_SIGHTED)`.
    pub short_sighted: bool,
    /// `getEffectiveFatigue()`, see [`crate::jni::effective_fatigue`].
    pub effective_fatigue: f32,
    /// `getDetectionRange()`, see [`crate::jni::detection_range`].
    pub detection_range: f32,
    /// `getForwardIsoDirection()` (for `ForcedVis`).
    pub forward: Option<IsoDirection>,
}
