//! Map objects: `IsoObject` and the `zombie.iso.objects` classes that
//! `CellLoader.DoTileObjectCreation` creates, with the fields map loading
//! sets.
//!
//! Each object is one [`IsoObject`] holding the fields every object has and
//! an [`ObjectKind`] with the class-specific state. Rendering, sound,
//! networking and update-loop state are not represented.

use crate::chunk::IsoGridSquare;
use crate::defs::RoomDef;
use crate::sprite::SpriteIdx;

/// Identity of an object within its square (stands in for Java object
/// identity, e.g. for `specialObjects`).
pub type ObjectId = u32;

/// `ItemContainer` as created from sprite properties (no items).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ItemContainer {
    /// `type`; `None` when the sprite has the `container` flag but no
    /// `container` property.
    pub kind: Option<String>,
    pub capacity: i32,
    pub container_position: Option<String>,
    pub freezer_position: Option<String>,
}

impl ItemContainer {
    /// `new ItemContainer(type, square, parent)`.
    pub fn new(kind: Option<&str>) -> Self {
        ItemContainer {
            kind: kind.map(str::to_string),
            capacity: 50,
            container_position: None,
            freezer_position: None,
        }
    }
}

/// `IsoLightSource` (map-loading fields).
#[derive(Clone, Debug, PartialEq)]
pub struct IsoLightSource {
    pub x: i32,
    pub y: i32,
    pub z: i32,
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub radius: i32,
    pub active: bool,
    pub hydro_powered: bool,
}

/// `IsoDoor`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IsoDoor {
    pub north: bool,
    pub open: bool,
    pub locked: bool,
    pub locked_by_key: bool,
    pub health: i32,
    pub max_health: i32,
    pub pushed_strength: i32,
    pub pushed_max_strength: i32,
    pub open_sprite: Option<SpriteIdx>,
    pub closed_sprite: Option<SpriteIdx>,
}

/// `IsoCurtain`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IsoCurtain {
    pub north: bool,
    pub open: bool,
    pub health: i32,
    pub max_health: i32,
    pub pushed_strength: i32,
    pub pushed_max_strength: i32,
    pub open_sprite: Option<SpriteIdx>,
    pub closed_sprite: Option<SpriteIdx>,
}

/// `IsoWindow`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IsoWindow {
    pub north: bool,
    pub locked: bool,
    pub perma_locked: bool,
    pub open: bool,
    pub destroyed: bool,
    pub glass_removed: bool,
    pub health: i32,
    pub max_health: i32,
    pub open_sprite: Option<SpriteIdx>,
    pub closed_sprite: Option<SpriteIdx>,
    pub smashed_sprite: Option<SpriteIdx>,
    pub glass_removed_sprite: Option<SpriteIdx>,
}

/// `IsoLightSwitch`.
#[derive(Clone, Debug, PartialEq)]
pub struct IsoLightSwitch {
    pub activated: bool,
    pub light_room: bool,
    pub room_id: i64,
    pub street_light: bool,
    pub can_be_modified: bool,
    pub primary_r: f32,
    pub primary_g: f32,
    pub primary_b: f32,
    pub lights: Vec<IsoLightSource>,
}

/// `IsoBarbecue`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IsoBarbecue {
    pub has_propane_tank: bool,
    pub fuel_amount_minutes: i32,
    pub normal_sprite: Option<SpriteIdx>,
    pub no_tank_sprite: Option<SpriteIdx>,
}

/// `IsoTree`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IsoTree {
    pub size: i32,
    pub log_yield: i32,
    pub damage: i32,
}

/// `PresetEntry`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PresetEntry {
    pub name: String,
    pub frequency: i32,
}

/// `DevicePresets`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DevicePresets {
    pub max_presets: i32,
    pub presets: Vec<PresetEntry>,
}

impl Default for DevicePresets {
    fn default() -> Self {
        DevicePresets {
            max_presets: 10,
            presets: Vec::new(),
        }
    }
}

impl DevicePresets {
    /// `addPreset`: skipped when the frequency or the name is present, or
    /// when full.
    pub fn add_preset(&mut self, name: &str, frequency: i32) {
        if self
            .presets
            .iter()
            .any(|p| p.frequency == frequency || p.name == name)
        {
            return;
        }
        if (self.presets.len() as i32) < self.max_presets {
            self.presets.push(PresetEntry {
                name: name.to_string(),
                frequency,
            });
        }
    }
}

/// `DeviceData` (state fields; sound and media playback are not ported).
#[derive(Clone, Debug, PartialEq)]
pub struct DeviceData {
    pub device_name: String,
    pub two_way: bool,
    pub transmit_range: i32,
    pub mic_range: i32,
    pub mic_is_muted: bool,
    pub base_volume_range: f32,
    pub device_volume: f32,
    pub is_portable: bool,
    pub is_television: bool,
    pub is_high_tier: bool,
    pub is_turned_on: bool,
    pub channel: i32,
    pub min_channel_range: i32,
    pub max_channel_range: i32,
    pub presets: DevicePresets,
    pub is_battery_powered: bool,
    pub has_battery: bool,
    pub power_delta: f32,
    pub use_delta: f32,
    pub headphone_type: i32,
    pub media_index: i16,
    pub media_type: i8,
    pub no_transmit: bool,
    pub last_minute_stamp: i64,
}

impl Default for DeviceData {
    /// `new DeviceData()`.
    fn default() -> Self {
        DeviceData {
            device_name: "WaveSignalDevice".to_string(),
            two_way: false,
            transmit_range: 1000,
            mic_range: 5,
            mic_is_muted: false,
            base_volume_range: 15.0,
            device_volume: 1.0,
            is_portable: false,
            is_television: false,
            is_high_tier: false,
            is_turned_on: false,
            channel: 88000,
            min_channel_range: 200,
            max_channel_range: 1000000,
            presets: DevicePresets::default(),
            is_battery_powered: true,
            has_battery: true,
            power_delta: 1.0,
            use_delta: 0.001,
            headphone_type: -1,
            media_index: -1,
            media_type: -1,
            no_transmit: false,
            last_minute_stamp: -1,
        }
    }
}

impl DeviceData {
    /// `setDeviceVolume` (without the network transmit).
    pub fn set_device_volume(&mut self, f: f32) {
        self.device_volume = f.clamp(0.0, 1.0);
    }

    /// `setTurnedOnRaw`.
    pub fn set_turned_on_raw(&mut self, b: bool) {
        self.is_turned_on = b;
        if !b {
            self.last_minute_stamp = -1;
        }
    }

    /// `setPower`.
    pub fn set_power(&mut self, p: f32) {
        self.power_delta = p.clamp(0.0, 1.0);
    }

    /// `setInitialPower` with `GameTime.getMinutesStamp()`.
    pub fn set_initial_power(&mut self, minutes_stamp: i64) {
        self.last_minute_stamp = minutes_stamp;
        self.set_power(self.power_delta - self.use_delta * minutes_stamp as f32);
    }
}

/// The class of a map object and its class-specific state.
#[derive(Clone, Debug, PartialEq)]
pub enum ObjectKind {
    /// Plain `IsoObject` (floors, walls, stairs, furniture, ...).
    Object,
    Door(IsoDoor),
    LightSwitch(IsoLightSwitch),
    Curtain(IsoCurtain),
    Window(IsoWindow),
    WindowFrame {
        north: bool,
    },
    Barbecue(IsoBarbecue),
    /// `IsoFireplace`: fireplaces, campfires, braziers and wood stoves.
    Fireplace,
    CombinationWasherDryer,
    ClothingDryer,
    ClothingWasher,
    Stove,
    Jukebox,
    /// `IsoRadio` (an `IsoWaveSignal`).
    Radio(DeviceData),
    /// `IsoTelevision` (an `IsoWaveSignal`).
    Television(DeviceData),
    Mannequin,
    Tree(IsoTree),
}

impl ObjectKind {
    /// The Java class name.
    pub fn class_name(&self) -> &'static str {
        match self {
            ObjectKind::Object => "IsoObject",
            ObjectKind::Door(_) => "IsoDoor",
            ObjectKind::LightSwitch(_) => "IsoLightSwitch",
            ObjectKind::Curtain(_) => "IsoCurtain",
            ObjectKind::Window(_) => "IsoWindow",
            ObjectKind::WindowFrame { .. } => "IsoWindowFrame",
            ObjectKind::Barbecue(_) => "IsoBarbecue",
            ObjectKind::Fireplace => "IsoFireplace",
            ObjectKind::CombinationWasherDryer => "IsoCombinationWasherDryer",
            ObjectKind::ClothingDryer => "IsoClothingDryer",
            ObjectKind::ClothingWasher => "IsoClothingWasher",
            ObjectKind::Stove => "IsoStove",
            ObjectKind::Jukebox => "IsoJukebox",
            ObjectKind::Radio(_) => "IsoRadio",
            ObjectKind::Television(_) => "IsoTelevision",
            ObjectKind::Mannequin => "IsoMannequin",
            ObjectKind::Tree(_) => "IsoTree",
        }
    }

    /// `instanceof IsoWaveSignal`: the device data.
    pub fn device_data(&self) -> Option<&DeviceData> {
        match self {
            ObjectKind::Radio(d) | ObjectKind::Television(d) => Some(d),
            _ => None,
        }
    }
}

/// `IsoObject` (map-loading fields) plus its class.
#[derive(Clone, Debug, PartialEq)]
pub struct IsoObject {
    pub id: ObjectId,
    pub kind: ObjectKind,
    pub sprite: Option<SpriteIdx>,
    /// `tile`: the map tile name the object was created from.
    pub tile: Option<String>,
    pub sx: f32,
    pub attached_anim_sprite: Option<Vec<SpriteIdx>>,
    pub container: Option<ItemContainer>,
    pub secondary_containers: Vec<ItemContainer>,
    pub outline_on_mouseover: bool,
    pub tint_r: f32,
    pub tint_g: f32,
    pub tint_b: f32,
    pub render_y_offset: f32,
    /// Whether the entity hook gave the object components
    /// (`GameEntity.hasComponents`).
    pub has_components: bool,
}

impl IsoObject {
    pub fn new(id: ObjectId, kind: ObjectKind, sprite: Option<SpriteIdx>) -> Self {
        IsoObject {
            id,
            kind,
            sprite,
            tile: None,
            sx: 0.0,
            attached_anim_sprite: None,
            container: None,
            secondary_containers: Vec::new(),
            outline_on_mouseover: false,
            tint_r: 1.0,
            tint_g: 1.0,
            tint_b: 1.0,
            render_y_offset: 0.0,
            has_components: false,
        }
    }

    /// `attachedAnimSprite.add(IsoSpriteInstance.get(spr))`.
    pub fn attach_anim_sprite(&mut self, s: SpriteIdx) {
        self.attached_anim_sprite
            .get_or_insert_with(|| Vec::with_capacity(4))
            .push(s);
    }
}

/// Lookup tables defined in game code that object creation needs. Not part
/// of this repository: read from the installed game at runtime.
#[derive(Clone, Debug)]
pub struct ObjectTables {
    /// `IsoTree.LOGS_PER_SIZE`, indexed by `size - 1` (sizes 1..=8).
    pub logs_per_size: Vec<i32>,
    /// `IsoWindow.LockedHouseFrequency` constants in ordinal order as
    /// `(value, lockChance)`. The first constant is `Never`; the last is
    /// `VeryOften`, which `fromValue` falls back to.
    pub locked_house_frequency: Vec<(i32, i32)>,
}

impl ObjectTables {
    /// Reads the tables from the installed game's `projectzomboid.jar`.
    ///
    /// LOADER STUB: to be wired to the `pz-classfile` class-file reader
    /// (the static initialiser of `zombie.iso.objects.IsoTree` and the enum
    /// constants of `zombie.iso.objects.IsoWindow$LockedHouseFrequency`).
    pub fn from_game_jar(_jar: &std::path::Path) -> Result<Self, String> {
        Err("ObjectTables::from_game_jar is not wired to pz-classfile yet".into())
    }

    /// Reads the tables from text: line 1 the comma-separated logs per
    /// size, line 2 comma-separated `value:lockChance` pairs.
    pub fn from_text(text: &str) -> Result<Self, String> {
        let mut lines = text.lines();
        let logs = lines
            .next()
            .unwrap_or("")
            .split(',')
            .filter(|s| !s.trim().is_empty())
            .map(|s| s.trim().parse::<i32>().map_err(|e| e.to_string()))
            .collect::<Result<Vec<_>, _>>()?;
        let freq = lines
            .next()
            .unwrap_or("")
            .split(',')
            .filter(|s| !s.trim().is_empty())
            .map(|p| {
                let (v, c) = p.trim().split_once(':').ok_or("expected value:chance")?;
                Ok((
                    v.parse::<i32>().map_err(|e| e.to_string())?,
                    c.parse::<i32>().map_err(|e| e.to_string())?,
                ))
            })
            .collect::<Result<Vec<_>, String>>()?;
        Self::new(logs, freq)
    }

    pub fn new(
        logs_per_size: Vec<i32>,
        locked_house_frequency: Vec<(i32, i32)>,
    ) -> Result<Self, String> {
        if logs_per_size.len() != 8 {
            return Err(format!("{} logs per size, expected 8", logs_per_size.len()));
        }
        if locked_house_frequency.is_empty() {
            return Err("empty LockedHouseFrequency table".into());
        }
        Ok(ObjectTables {
            logs_per_size,
            locked_house_frequency,
        })
    }

    /// `LockedHouseFrequency.fromValue(value).getLockChance()`.
    pub fn window_lock_chance(&self, value: i32) -> i32 {
        self.locked_house_frequency
            .iter()
            .find(|&&(v, _)| v == value)
            .unwrap_or(self.locked_house_frequency.last().unwrap())
            .1
    }

    /// `LockedHouseFrequency.Never.lockChance`.
    pub fn never_lock_chance(&self) -> i32 {
        self.locked_house_frequency[0].1
    }
}

/// Game state object creation reads.
#[derive(Clone, Copy, Debug)]
pub struct ObjectLoadSettings {
    /// `SandboxOptions.lockedHouses`.
    pub locked_houses: i32,
    /// `Core.gameMode.equals("Tutorial")`.
    pub tutorial: bool,
    /// `GameTime.getMinutesStamp()`.
    pub minutes_stamp: i64,
}

impl Default for ObjectLoadSettings {
    fn default() -> Self {
        ObjectLoadSettings {
            locked_houses: 6,
            tutorial: false,
            minutes_stamp: 0,
        }
    }
}

/// Systems object creation calls into that are outside this crate. The
/// defaults behave like a game with no item or entity scripts loaded and no
/// power.
pub trait ObjectLoadHooks {
    /// `IsoGridSquare.haveElectricity() || hasGridPower()` for a square in
    /// `room` (generators, sandbox power-grid timing, derelict and
    /// user-defined rooms, `NoPower` zones).
    fn square_powered(&mut self, _sq: &IsoGridSquare, _room: &RoomDef) -> bool {
        false
    }

    /// `GameEntityFactory.CreateIsoEntityFromCellLoading(obj)` for an object
    /// whose sprite has the `EntityScript` flag or both `IsMoveAble` and
    /// `CustomItem`. Returns whether components were added.
    fn create_iso_entity(&mut self, _obj: &IsoObject, _rand: &mut dyn crate::GameRand) -> bool {
        false
    }

    /// `IsoWaveSignal.cloneDeviceDataFromItem(item)` (including the item
    /// creation it does the first time an item type is seen).
    fn device_data_from_item(
        &mut self,
        _item: &str,
        _rand: &mut dyn crate::GameRand,
    ) -> Option<DeviceData> {
        None
    }

    /// `ZomboidRadio.GetChannelList(category)` in iteration order.
    fn channel_list(&self, _category: &str) -> Option<Vec<(i32, String)>> {
        None
    }
}

/// [`ObjectLoadHooks`] with every default.
pub struct NoHooks;
impl ObjectLoadHooks for NoHooks {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn presets_and_tables() {
        let mut p = DevicePresets {
            max_presets: 2,
            ..Default::default()
        };
        p.add_preset("a", 1);
        p.add_preset("a", 2);
        p.add_preset("b", 1);
        p.add_preset("c", 3);
        p.add_preset("d", 4);
        assert_eq!(p.presets.len(), 2);
        let t = ObjectTables::from_text("2,2,3,3,4,4,5,5\n0:0,1:9,2:7\n").unwrap();
        assert_eq!(t.window_lock_chance(1), 9);
        assert_eq!(t.window_lock_chance(9), 7);
        assert_eq!(t.never_lock_chance(), 0);
        assert!(ObjectTables::from_text("1,2\n0:0").is_err());
        let mut d = DeviceData::default();
        d.set_initial_power(100);
        assert!((d.power_delta - 0.9).abs() < 1e-6);
        d.set_turned_on_raw(false);
        assert_eq!(d.last_minute_stamp, -1);
    }
}
