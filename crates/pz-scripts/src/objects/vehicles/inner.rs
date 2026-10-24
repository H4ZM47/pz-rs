//! The nested classes of `VehicleScript` (`VehicleScript.Part`,
//! `VehicleScript.Passenger`, ...) and a stand-in for the `KahluaTable`s that
//! part `table` blocks load into.

use crate::java::JHashMap;
use crate::math::Vector3;

/// `VehicleScript.Skin`. The `textureData*` fields (`Texture`) are filled by
/// the renderer and not kept here.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Skin {
    pub texture: Option<String>,
    pub texture_rust: Option<String>,
    pub texture_mask: Option<String>,
    pub texture_lights: Option<String>,
    pub texture_damage1_overlay: Option<String>,
    pub texture_damage1_shell: Option<String>,
    pub texture_damage2_overlay: Option<String>,
    pub texture_damage2_shell: Option<String>,
    pub texture_shadow: Option<String>,
}

impl Skin {
    /// `copyMissingFrom`.
    pub fn copy_missing_from(&mut self, other: &Skin) {
        let pairs = [
            (&mut self.texture_rust, &other.texture_rust),
            (&mut self.texture_mask, &other.texture_mask),
            (&mut self.texture_lights, &other.texture_lights),
            (&mut self.texture_damage1_overlay, &other.texture_damage1_overlay),
            (&mut self.texture_damage1_shell, &other.texture_damage1_shell),
            (&mut self.texture_damage2_overlay, &other.texture_damage2_overlay),
            (&mut self.texture_damage2_shell, &other.texture_damage2_shell),
            (&mut self.texture_shadow, &other.texture_shadow),
        ];
        for (mine, theirs) in pairs {
            if mine.is_none() {
                mine.clone_from(theirs);
            }
        }
    }
}

/// `VehicleScript.LightBar`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LightBar {
    pub enable: bool,
    pub sound_siren0: String,
    pub sound_siren1: String,
    pub sound_siren2: String,
}

/// `VehicleScript.Sounds`.
#[derive(Clone, Debug)]
pub struct Sounds {
    pub alarm_enable: bool,
    pub alarm: Vec<String>,
    pub alarm_loop: Vec<String>,
    pub horn_enable: bool,
    pub horn: Option<String>,
    pub back_signal_enable: bool,
    pub back_signal: Option<String>,
    pub engine: Option<String>,
    pub engine_start: Option<String>,
    pub engine_turn_off: Option<String>,
    pub hand_brake: Option<String>,
    pub ignition_fail: Option<String>,
    pub ignition_fail_no_power: Option<String>,
    /// `HashSet<String> specified`, in `HashSet` order.
    pub specified: JHashMap<()>,
}

impl Default for Sounds {
    fn default() -> Self {
        Sounds {
            alarm_enable: false,
            alarm: Vec::new(),
            alarm_loop: Vec::new(),
            horn_enable: false,
            horn: Some(String::new()),
            back_signal_enable: false,
            back_signal: Some(String::new()),
            engine: None,
            engine_start: None,
            engine_turn_off: None,
            hand_brake: None,
            ignition_fail: None,
            ignition_fail_no_power: None,
            specified: JHashMap::new(),
        }
    }
}

/// `org.joml`-free stand-in for `zombie.core.ImmutableColor`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ImmutableColor {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

impl ImmutableColor {
    /// `new ImmutableColor(r, g, b)`: components clamped to [0, 1], alpha 1.
    pub fn rgb(r: f32, g: f32, b: f32) -> Self {
        ImmutableColor { r: clamp(r, 0.0, 1.0), g: clamp(g, 0.0, 1.0), b: clamp(b, 0.0, 1.0), a: 1.0 }
    }
}

/// `PZMath.clamp(float, float, float)`.
pub(super) fn clamp(v: f32, min: f32, max: f32) -> f32 {
    if v < min {
        min
    } else if v > max {
        max
    } else {
        v
    }
}

/// `VehicleScript.Area`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Area {
    pub id: Option<String>,
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

/// `VehicleScript.Model`.
#[derive(Clone, Debug, PartialEq)]
pub struct Model {
    pub id: Option<String>,
    pub file: Option<String>,
    pub scale: f32,
    pub offset: Vector3,
    pub rotate: Vector3,
    pub attachment_name_parent: Option<String>,
    pub attachment_name_self: Option<String>,
    pub ignore_vehicle_scale: bool,
}

impl Default for Model {
    fn default() -> Self {
        Model {
            id: None,
            file: None,
            scale: 1.0,
            offset: Vector3::default(),
            rotate: Vector3::default(),
            attachment_name_parent: None,
            attachment_name_self: None,
            ignore_vehicle_scale: false,
        }
    }
}

/// A key of a Lua table loaded from a script: `checkIntegerKey` turns
/// all-digit keys into numbers.
#[derive(Clone, Debug, PartialEq)]
pub enum LuaKey {
    Str(String),
    Num(f64),
}

#[derive(Clone, Debug, PartialEq)]
pub enum LuaValue {
    Str(String),
    Table(LuaTable),
}

/// The data of a `KahluaTable` built by `VehicleScript.LoadTable`, in
/// insertion order.
// TODO(lua): materialize as a KahluaTable when parts are exposed to Lua.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LuaTable {
    pub entries: Vec<(LuaKey, LuaValue)>,
}

impl LuaTable {
    /// `rawget`.
    pub fn rawget(&self, key: &LuaKey) -> Option<&LuaValue> {
        self.entries.iter().find(|(k, _)| k == key).map(|(_, v)| v)
    }

    /// `rawget` with a string key.
    pub fn rawget_str(&self, key: &str) -> Option<&LuaValue> {
        self.entries.iter().find(|(k, _)| matches!(k, LuaKey::Str(s) if s == key)).map(|(_, v)| v)
    }

    /// `rawset`: a null value removes the key.
    pub fn rawset(&mut self, key: LuaKey, value: Option<LuaValue>) {
        let pos = self.entries.iter().position(|(k, _)| *k == key);
        match (pos, value) {
            (Some(i), Some(v)) => self.entries[i].1 = v,
            (Some(i), None) => {
                self.entries.remove(i);
            }
            (None, Some(v)) => self.entries.push((key, v)),
            (None, None) => {}
        }
    }
}

/// `put` on a `THashMap` kept as an insertion-ordered list.
pub(super) fn map_put<K: PartialEq, V>(map: &mut Vec<(K, V)>, key: K, value: V) {
    match map.iter_mut().find(|(k, _)| *k == key) {
        Some(e) => e.1 = value,
        None => map.push((key, value)),
    }
}

/// `VehicleScript.Part`.
#[derive(Clone, Debug, PartialEq)]
pub struct Part {
    pub id: Option<String>,
    pub parent: Option<String>,
    pub item_type: Option<Vec<String>>,
    pub container: Option<Container>,
    pub area: Option<String>,
    pub mechanic_area: Option<String>,
    pub wheel: Option<String>,
    /// `THashMap<String, KahluaTable> tables`.
    pub tables: Option<Vec<(Option<String>, LuaTable)>>,
    /// `THashMap<String, String> luaFunctions`.
    pub lua_functions: Option<Vec<(String, String)>>,
    pub models: Option<Vec<Model>>,
    pub set_all_models_visible: bool,
    pub door: Option<Door>,
    pub window: Option<Window>,
    pub anims: Option<Vec<Anim>>,
    pub category: Option<String>,
    pub specific_item: bool,
    pub mechanic_require_key: bool,
    pub repair_mechanic: bool,
    pub has_lights_rear: bool,
    pub(super) durability: f32,
}

impl Default for Part {
    fn default() -> Self {
        Part {
            id: Some("Unknown".to_string()),
            parent: None,
            item_type: None,
            container: None,
            area: None,
            mechanic_area: None,
            wheel: None,
            tables: None,
            lua_functions: None,
            models: None,
            set_all_models_visible: true,
            door: None,
            window: None,
            anims: None,
            category: None,
            specific_item: true,
            mechanic_require_key: false,
            repair_mechanic: false,
            has_lights_rear: false,
            durability: 0.0,
        }
    }
}

impl Part {
    /// `getId`.
    pub fn id(&self) -> Option<&str> {
        self.id.as_deref()
    }

    /// `getModelCount`.
    pub fn model_count(&self) -> usize {
        self.models.as_ref().map_or(0, Vec::len)
    }

    /// `getDurability`.
    pub fn durability(&self) -> f32 {
        self.durability
    }

    /// `getMechanicArea`.
    pub fn mechanic_area(&self) -> Option<&str> {
        self.mechanic_area.as_deref()
    }

    /// `getAnimById` (the original throws on an anim without id).
    pub fn anim_by_id(&self, id: &str) -> Option<&Anim> {
        self.anims.as_ref()?.iter().find(|a| a.id.as_deref() == Some(id))
    }

    /// `getModelById` (the original throws on a model without id).
    pub fn model_by_id(&self, id: &str) -> Option<&Model> {
        self.models.as_ref()?.iter().find(|m| m.id.as_deref() == Some(id))
    }

    /// `makeCopy`: `LuaManager.copyTable` deep-copies the tables.
    pub fn make_copy(&self) -> Part {
        Part { container: self.container.as_ref().map(Container::make_copy), ..self.clone() }
    }
}

/// `VehicleScript.Passenger.SwitchSeat`.
#[derive(Clone, Debug, PartialEq)]
pub struct SwitchSeat {
    pub id: Option<String>,
    pub seat: i32,
    pub anim: Option<String>,
    pub rate: f32,
    pub sound: Option<String>,
}

impl Default for SwitchSeat {
    fn default() -> Self {
        SwitchSeat { id: None, seat: 0, anim: None, rate: 1.0, sound: None }
    }
}

/// `VehicleScript.Passenger`.
#[derive(Clone, Debug, PartialEq)]
pub struct Passenger {
    pub id: Option<String>,
    pub anims: Vec<Anim>,
    pub switch_seats: Vec<SwitchSeat>,
    pub has_roof: bool,
    pub show_passenger: bool,
    pub door: Option<String>,
    pub door2: Option<String>,
    pub area: Option<String>,
    pub positions: Vec<Position>,
}

impl Default for Passenger {
    fn default() -> Self {
        Passenger {
            id: None,
            anims: Vec::new(),
            switch_seats: Vec::new(),
            has_roof: true,
            show_passenger: false,
            door: None,
            door2: None,
            area: None,
            positions: Vec::new(),
        }
    }
}

impl Passenger {
    /// `makeCopy`: positions are copied by `Position.makeCopy`, which drops
    /// their `area`.
    pub fn make_copy(&self) -> Passenger {
        Passenger { positions: self.positions.iter().map(Position::make_copy).collect(), ..self.clone() }
    }

    /// `getPositionById`.
    pub fn position_by_id(&self, id: &str) -> Option<&Position> {
        self.positions.iter().find(|p| p.id.as_deref() == Some(id))
    }

    /// `getSwitchSeatById`.
    pub fn switch_seat_index(&self, id: Option<&str>) -> Option<usize> {
        self.switch_seats.iter().position(|s| s.id.is_some() && s.id.as_deref() == id)
    }
}

/// `VehicleScript.PHYSICS_SHAPE_BOX`.
pub const PHYSICS_SHAPE_BOX: i32 = 1;
/// `VehicleScript.PHYSICS_SHAPE_SPHERE`.
pub const PHYSICS_SHAPE_SPHERE: i32 = 2;
/// `VehicleScript.PHYSICS_SHAPE_MESH`.
pub const PHYSICS_SHAPE_MESH: i32 = 3;

/// `VehicleScript.PhysicsShape`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PhysicsShape {
    pub type_: i32,
    pub offset: Vector3,
    pub rotate: Vector3,
    pub extents: Vector3,
    pub radius: f32,
    pub physics_shape_script: Option<String>,
}

impl PhysicsShape {
    /// `getTypeString`.
    pub fn type_string(&self) -> Option<&'static str> {
        match self.type_ {
            PHYSICS_SHAPE_BOX => Some("box"),
            PHYSICS_SHAPE_SPHERE => Some("sphere"),
            PHYSICS_SHAPE_MESH => Some("mesh"),
            _ => None,
        }
    }

    /// `setRadius`: clamped to [0.05, 5].
    pub fn set_radius(&mut self, radius: f32) {
        self.radius = clamp(radius, 0.05, 5.0);
    }

    /// `makeCopy`: does not copy `physicsShapeScript`.
    pub fn make_copy(&self) -> PhysicsShape {
        PhysicsShape { physics_shape_script: None, ..self.clone() }
    }
}

/// `VehicleScript.Wheel`.
#[derive(Clone, Debug, PartialEq)]
pub struct Wheel {
    pub id: Option<String>,
    pub model: Option<String>,
    pub front: bool,
    pub offset: Vector3,
    pub radius: f32,
    pub width: f32,
}

impl Default for Wheel {
    fn default() -> Self {
        Wheel { id: None, model: None, front: false, offset: Vector3::default(), radius: 0.5, width: 0.4 }
    }
}

/// `VehicleScript.Position`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Position {
    pub id: Option<String>,
    pub offset: Vector3,
    pub rotate: Vector3,
    pub area: Option<String>,
}

impl Position {
    /// `makeCopy`: does not copy `area`.
    pub fn make_copy(&self) -> Position {
        Position { area: None, ..self.clone() }
    }
}

/// `VehicleScript.Container`.
#[derive(Clone, Debug)]
pub struct Container {
    pub capacity: i32,
    pub seat: i32,
    pub seat_id: Option<String>,
    pub lua_test: Option<String>,
    pub content_type: Option<String>,
    pub condition_affects_capacity: bool,
    pub sound_map: JHashMap<String>,
}

impl PartialEq for Container {
    fn eq(&self, o: &Self) -> bool {
        self.capacity == o.capacity
            && self.seat == o.seat
            && self.seat_id == o.seat_id
            && self.lua_test == o.lua_test
            && self.content_type == o.content_type
            && self.condition_affects_capacity == o.condition_affects_capacity
            && self.sound_map.len() == o.sound_map.len()
            && self.sound_map.iter().all(|(k, v)| o.sound_map.get(k) == Some(v))
    }
}

impl Default for Container {
    fn default() -> Self {
        Container {
            capacity: 0,
            seat: -1,
            seat_id: None,
            lua_test: None,
            content_type: None,
            condition_affects_capacity: false,
            sound_map: JHashMap::new(),
        }
    }
}

impl Container {
    /// `getSoundByID`.
    pub fn sound_by_id(&self, id: &str) -> Option<&str> {
        self.sound_map.get(id).map(String::as_str)
    }

    /// `makeCopy`: `soundMap.putAll` into a fresh map.
    pub fn make_copy(&self) -> Container {
        let mut sound_map = JHashMap::new();
        for (k, v) in self.sound_map.iter() {
            sound_map.insert(k.to_string(), v.clone());
        }
        Container { sound_map, ..self.clone() }
    }
}

/// `VehicleScript.Anim`.
#[derive(Clone, Debug, PartialEq)]
pub struct Anim {
    pub id: Option<String>,
    pub anim: Option<String>,
    pub rate: f32,
    pub animate: bool,
    pub loop_: bool,
    pub reverse: bool,
    pub offset: Vector3,
    pub angle: Vector3,
    pub sound: Option<String>,
}

impl Default for Anim {
    fn default() -> Self {
        Anim {
            id: None,
            anim: None,
            rate: 1.0,
            animate: true,
            loop_: false,
            reverse: false,
            offset: Vector3::default(),
            angle: Vector3::default(),
            sound: None,
        }
    }
}

/// `VehicleScript.Door` (no fields).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Door;

/// `VehicleScript.Window`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Window {
    pub openable: bool,
}
