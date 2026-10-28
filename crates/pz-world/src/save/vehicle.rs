//! `BaseVehicle.save`/`load` with `VehiclePart`, `VehicleEngine`,
//! `VehicleLight`, `VehicleDoor` and `VehicleWindow`, shared by the two
//! places vehicles are stored: the vehicle list of a chunk file (only read;
//! `IsoChunk.Save` always writes an empty list) and the `data` blob of a
//! `vehicles.db` row (`VehiclesDB2.VehicleBuffer`).
//!
//! What is read and written is the state the save format carries. The
//! vehicle script is not attached on load (the original does that in
//! `addToWorld`), so part ids are the saved ones and the alarm sound is
//! chosen only when a script is supplied when saving. Animals in a trailer
//! (`IsoAnimal.load`/`save`) are kept as their length-framed bytes.

use super::io::{err, Reader, SaveResult, Writer};
use super::items::{ItemBlob, ItemRegistry};
use super::lua_table::LuaTable;
use super::visual::MovingBase;
use crate::java::fastfloor;
use crate::objects::{ComponentBlob, ItemContainer};
use pz_inventory::device::DeviceData;
use pz_inventory::java::sort_string_hash_map;

/// `VehicleLight` (the saved fields).
#[derive(Clone, Debug, PartialEq)]
pub struct VehicleLight {
    pub active: bool,
    pub offset_x: f32,
    pub offset_y: f32,
    pub intensity: f32,
    pub dist: f32,
    pub focusing: i32,
}

impl Default for VehicleLight {
    /// `new VehicleLight()`.
    fn default() -> Self {
        VehicleLight {
            active: false,
            offset_x: 0.0,
            offset_y: 0.0,
            intensity: 1.0,
            dist: 16.0,
            focusing: 0,
        }
    }
}

impl VehicleLight {
    fn load(&mut self, r: &mut Reader<'_>) -> SaveResult<()> {
        self.active = r.bool()?;
        self.offset_x = r.f32()?;
        self.offset_y = r.f32()?;
        self.intensity = r.f32()?;
        self.dist = r.f32()?;
        self.focusing = r.i32()?;
        Ok(())
    }

    fn save(&self, w: &mut Writer) {
        w.bool(self.active);
        w.f32(self.offset_x);
        w.f32(self.offset_y);
        w.f32(self.intensity);
        w.f32(self.dist);
        w.i32(self.focusing);
    }
}

/// `VehicleDoor` (the saved fields).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct VehicleDoor {
    pub open: bool,
    pub locked: bool,
    pub lock_broken: bool,
}

/// `VehicleWindow` (the saved fields; its condition is the part's).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct VehicleWindow {
    pub open: bool,
    /// `openDelta`: 1 when loaded open.
    pub open_delta: f32,
}

/// `VehicleEngine` state that saving reads (`BaseVehicle.load` passes the
/// saved values to `VehicleEngine.load`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct VehicleEngine {
    /// `state == Running`.
    pub running: bool,
    pub loudness: i32,
    pub quality: i32,
    pub power: i32,
}

/// `VehiclePart` (the saved fields).
#[derive(Clone, Debug, PartialEq)]
pub struct VehiclePart {
    /// `partId`; `getId()` while no script part is attached.
    pub part_id: String,
    pub created: bool,
    pub last_updated: f32,
    /// The installed item.
    pub item: Option<ItemBlob>,
    pub container: Option<ItemContainer>,
    pub mod_data: Option<LuaTable>,
    pub device_data: Option<DeviceData>,
    pub light: Option<VehicleLight>,
    pub door: Option<VehicleDoor>,
    pub window: Option<VehicleWindow>,
    pub condition: i32,
    pub wheel_friction: f32,
    pub mechanic_skill_installer: i32,
    pub suspension_compression: f32,
    pub suspension_damping: f32,
    /// ECS components as their saved bytes.
    pub components: Vec<ComponentBlob>,
}

impl VehiclePart {
    /// `new VehiclePart(vehicle)`.
    pub fn new() -> Self {
        VehiclePart {
            part_id: String::new(),
            created: false,
            last_updated: -1.0,
            item: None,
            container: None,
            mod_data: None,
            device_data: None,
            light: None,
            door: None,
            window: None,
            condition: -1,
            wheel_friction: 0.0,
            mechanic_skill_installer: 0,
            suspension_compression: 0.0,
            suspension_damping: 0.0,
            components: Vec::new(),
        }
    }

    /// `getId()`.
    pub fn id(&self) -> &str {
        &self.part_id
    }

    /// `setCondition(condition)` as loading calls it: clamped to 0..100
    /// and passed to the installed item (`setCondition(c, false)`). Returns
    /// whether the vehicle's siren mode is reset (a `lightbar` part at 0).
    fn set_condition(&mut self, condition: i32) -> bool {
        let c = condition.clamp(0, 100);
        self.condition = c;
        if let Some(item) = self.item.as_mut().and_then(ItemBlob::as_item_mut) {
            // InventoryItem.setCondition: max(0, c), then min(conditionMax).
            item.condition = c.max(0).min(item.condition_max);
        }
        c <= 0 && self.id() == "lightbar"
    }

    /// `VehiclePart.load`. Returns the part and whether loading reset the
    /// vehicle's siren mode.
    fn load(r: &mut Reader<'_>, wv: i32, items: &dyn ItemRegistry) -> SaveResult<(Self, bool)> {
        let mut p = VehiclePart::new();
        let mut siren_off = false;
        p.part_id = r.string()?;
        p.created = r.bool()?;
        p.last_updated = r.f32()?;
        if r.bool()? {
            p.item = ItemBlob::load(r, wv, items)?;
        }
        if r.bool()? {
            let mut c = ItemContainer::new(None);
            c.id = 0;
            c.load(r, wv, items)?;
            p.container = Some(c);
        }
        if r.bool()? {
            p.mod_data = Some(LuaTable::load(r, wv)?);
        }
        if r.bool()? {
            let mut d = DeviceData::default();
            d.load(r)?;
            p.device_data = Some(d);
        }
        if r.bool()? {
            let mut l = VehicleLight::default();
            l.load(r)?;
            p.light = Some(l);
        }
        if r.bool()? {
            p.door = Some(VehicleDoor {
                open: r.bool()?,
                locked: r.bool()?,
                lock_broken: r.bool()?,
            });
        }
        if r.bool()? {
            // VehicleWindow.load: part.setCondition(byte), then open.
            siren_off |= p.set_condition(r.i8()? as i32);
            let open = r.bool()?;
            p.window = Some(VehicleWindow {
                open,
                open_delta: if open { 1.0 } else { 0.0 },
            });
        }
        siren_off |= p.set_condition(r.i32()?);
        p.wheel_friction = r.f32()?;
        p.mechanic_skill_installer = r.i32()?;
        p.suspension_compression = r.f32()?;
        p.suspension_damping = r.f32()?;
        if wv >= 200 && r.bool()? {
            p.components = load_components(r)?;
        }
        Ok((p, siren_off))
    }

    /// `VehiclePart.save`.
    pub fn save(&self, w: &mut Writer, items: &dyn ItemRegistry) -> SaveResult<()> {
        w.string(self.id());
        w.bool(self.created);
        w.f32(self.last_updated);
        match &self.item {
            Some(i) => {
                w.i8(1);
                i.save(w, items)?;
            }
            None => w.i8(0),
        }
        match &self.container {
            Some(c) => {
                w.i8(1);
                c.save(w, items)?;
            }
            None => w.i8(0),
        }
        match self.mod_data.as_ref().filter(|t| !t.is_empty()) {
            Some(t) => {
                w.i8(1);
                t.save(w);
            }
            None => w.i8(0),
        }
        match &self.device_data {
            Some(d) => {
                w.i8(1);
                d.save(w);
            }
            None => w.i8(0),
        }
        match &self.light {
            Some(l) => {
                w.i8(1);
                l.save(w);
            }
            None => w.i8(0),
        }
        match &self.door {
            Some(d) => {
                w.i8(1);
                w.bool(d.open);
                w.bool(d.locked);
                w.bool(d.lock_broken);
            }
            None => w.i8(0),
        }
        match &self.window {
            Some(win) => {
                w.i8(1);
                w.i8(self.condition as i8);
                w.bool(win.open);
            }
            None => w.i8(0),
        }
        w.i32(self.condition);
        w.f32(self.wheel_friction);
        w.i32(self.mechanic_skill_installer);
        w.f32(self.suspension_compression);
        w.f32(self.suspension_damping);
        if self.components.is_empty() {
            w.i8(0);
        } else {
            w.i8(1);
            save_components(w, &self.components);
        }
        Ok(())
    }
}

impl Default for VehiclePart {
    fn default() -> Self {
        Self::new()
    }
}

/// `GameEntity.loadEntity` framing: a count and length-prefixed blocks.
fn load_components(r: &mut Reader<'_>) -> SaveResult<Vec<ComponentBlob>> {
    let n = r.i8()?;
    let mut out: Vec<ComponentBlob> = Vec::new();
    for _ in 0..n.max(0) {
        let len = r.i32()?;
        if len < 2 {
            return err("DataBlock possible corruption.");
        }
        let block = r.bytes(len as usize)?;
        let c = ComponentBlob {
            type_id: i16::from_be_bytes([block[0], block[1]]),
            data: block[2..].to_vec(),
        };
        match out.iter_mut().find(|e| e.type_id == c.type_id) {
            Some(e) => *e = c,
            None => out.push(c),
        }
    }
    Ok(out)
}

fn save_components(w: &mut Writer, cs: &[ComponentBlob]) {
    w.i8(cs.len() as i8);
    for c in cs {
        w.i32(c.data.len() as i32 + 2);
        w.i16(c.type_id);
        w.bytes(&c.data);
    }
}

/// A towing link (`vehicleTowingId` and its attachment points).
#[derive(Clone, Debug, PartialEq)]
pub struct VehicleTowing {
    pub vehicle_id: i32,
    pub attachment_self: String,
    pub attachment_other: String,
    pub z_offset: f32,
}

/// The sounds of a vehicle script that `VehicleAlarm.chooseSound` reads.
pub struct AlarmSounds<'a> {
    pub alarm_enable: bool,
    pub alarm: &'a [String],
    pub alarm_loop: &'a [String],
}

/// `BaseVehicle` (the saved state).
#[derive(Clone, Debug, PartialEq)]
pub struct BaseVehicle {
    pub base: MovingBase,
    /// `savedPhysicsZ`: the saved physics height clamped to the level.
    pub saved_physics_z: f32,
    /// `savedRot` as loaded (`x, y, z, w`).
    pub saved_rot: [f32; 4],
    pub script_name: String,
    pub skin_index: i32,
    /// The engine of the part with id `Engine`; `None` without one (saving
    /// then writes zeros).
    pub engine: Option<VehicleEngine>,
    pub front_end_durability: i32,
    pub rear_end_durability: i32,
    pub current_front_end_durability: i32,
    pub current_rear_end_durability: i32,
    pub key_id: i32,
    pub key_spawned: i8,
    pub headlights_on: bool,
    pub created: bool,
    pub sound_horn_on: bool,
    pub sound_back_move_on: bool,
    pub lightbar_lights_mode: i32,
    pub lightbar_siren_mode: i32,
    pub parts: Vec<VehiclePart>,
    pub key_is_on_door: bool,
    pub hotwired: bool,
    pub hotwired_broken: bool,
    pub keys_in_ignition: bool,
    pub rust: f32,
    pub color_hue: f32,
    pub color_saturation: f32,
    pub color_value: f32,
    /// `vehicleId`: the runtime id, -1 until the vehicle is added to the
    /// world (loading reads the saved id and drops it).
    pub vehicle_id: i16,
    /// The `vehicleId` the record held.
    pub loaded_vehicle_id: i16,
    pub mechanical_id: i32,
    pub alarmed: bool,
    pub alarm_start_time: f64,
    /// `VehicleAlarm.chosenSound` (blank names load as `None`).
    pub alarm_chosen_sound: Option<String>,
    pub siren_start_time: f64,
    /// The key in the ignition (`ignitionSwitch`).
    pub current_key: Option<ItemBlob>,
    /// `bloodIntensity` in insertion order.
    pub blood_intensity: Vec<(String, i8)>,
    pub towing: Option<VehicleTowing>,
    pub regulator_speed: f32,
    pub previously_entered: bool,
    pub previously_moved: bool,
    /// The trailer's animals block after its length: `0`, or `1`, a count
    /// and the `IsoAnimal` records, kept as bytes.
    pub animals: Vec<u8>,
}

/// `PZMath.clamp(float, float, float)`.
fn clamp_f(v: f32, lo: f32, hi: f32) -> f32 {
    if v < lo {
        lo
    } else if v > hi {
        hi
    } else {
        v
    }
}

/// `org.joml.Math.sqrt(float)`.
fn jsqrt(f: f32) -> f32 {
    (f as f64).sqrt() as f32
}

/// `Transform.setRotation(q)` followed by `Transform.getRotation(out)`
/// (JOML `Matrix3f.rotation` and `Quaternionf.setFromUnnormalized`): the
/// rotation `BaseVehicle.save` writes for a loaded `savedRot`.
pub fn rotation_round_trip(q: [f32; 4]) -> [f32; 4] {
    let [x, y, z, w] = q;
    let w2 = w * w;
    let x2 = x * x;
    let y2 = y * y;
    let z2 = z * z;
    let zw = z * w;
    let dzw = zw + zw;
    let xy = x * y;
    let dxy = xy + xy;
    let xz = x * z;
    let dxz = xz + xz;
    let yw = y * w;
    let dyw = yw + yw;
    let yz = y * z;
    let dyz = yz + yz;
    let xw = x * w;
    let dxw = xw + xw;
    let m00 = w2 + x2 - z2 - y2;
    let m01 = dxy + dzw;
    let m02 = dxz - dyw;
    let m10 = -dzw + dxy;
    let m11 = y2 - z2 + w2 - x2;
    let m12 = dyz + dxw;
    let m20 = dyw + dxz;
    let m21 = dyz - dxw;
    let m22 = z2 - y2 - x2 + w2;
    let invsqrt = |f: f32| 1.0f32 / jsqrt(f);
    let lx = invsqrt(m00 * m00 + m01 * m01 + m02 * m02);
    let ly = invsqrt(m10 * m10 + m11 * m11 + m12 * m12);
    let lz = invsqrt(m20 * m20 + m21 * m21 + m22 * m22);
    let (m00, m01, m02) = (m00 * lx, m01 * lx, m02 * lx);
    let (m10, m11, m12) = (m10 * ly, m11 * ly, m12 * ly);
    let (m20, m21, m22) = (m20 * lz, m21 * lz, m22 * lz);
    let tr = m00 + m11 + m22;
    let (ox, oy, oz, ow);
    if tr >= 0.0 {
        let t = jsqrt(tr + 1.0);
        ow = t * 0.5;
        let t = 0.5 / t;
        ox = (m12 - m21) * t;
        oy = (m20 - m02) * t;
        oz = (m01 - m10) * t;
    } else if m00 >= m11 && m00 >= m22 {
        let t = jsqrt(m00 - (m11 + m22) + 1.0);
        ox = t * 0.5;
        let t = 0.5 / t;
        oy = (m10 + m01) * t;
        oz = (m02 + m20) * t;
        ow = (m12 - m21) * t;
    } else if m11 > m22 {
        let t = jsqrt(m11 - (m22 + m00) + 1.0);
        oy = t * 0.5;
        let t = 0.5 / t;
        oz = (m21 + m12) * t;
        ox = (m10 + m01) * t;
        ow = (m20 - m02) * t;
    } else {
        let t = jsqrt(m22 - (m00 + m11) + 1.0);
        oz = t * 0.5;
        let t = 0.5 / t;
        ox = (m02 + m20) * t;
        oy = (m21 + m12) * t;
        ow = (m01 - m10) * t;
    }
    [ox, oy, oz, ow]
}

impl BaseVehicle {
    /// `IsoMovingObject.load` + `BaseVehicle.load` (after the factory
    /// bytes) into a `new BaseVehicle(cell)`. `offsets` are the object's
    /// `offsetX`/`offsetY` (`32 * tileScale`, `96 * tileScale`).
    ///
    /// The constructor's `Rand` draws (key id, rust) are overwritten by the
    /// loaded values and not made here.
    pub fn load(
        r: &mut Reader<'_>,
        wv: i32,
        offsets: (f32, f32),
        items: &dyn ItemRegistry,
    ) -> SaveResult<Self> {
        let base = MovingBase::load(r, wv, offsets)?;
        let physics_z = r.f32()?;
        let level = fastfloor(base.z) as f32;
        let saved_physics_z = clamp_f(physics_z, level * 2.44949, (level + 0.995) * 2.44949);
        let saved_rot = [r.f32()?, r.f32()?, r.f32()?, r.f32()?];
        let script_name = r.string()?;
        let skin_index = r.i32()?;
        let engine_running = r.bool()?;
        let front_end_durability = r.i32()?;
        let rear_end_durability = r.i32()?;
        let current_front_end_durability = r.i32()?;
        let current_rear_end_durability = r.i32()?;
        let engine_loudness = r.i32()?;
        let engine_quality = r.i32()?.clamp(0, 100);
        let key_id = r.i32()?;
        let key_spawned = r.i8()?;
        let headlights_on = r.bool()?;
        let created = r.bool()?;
        let sound_horn_on = r.bool()?;
        let sound_back_move_on = r.bool()?;
        let lightbar_lights_mode = (r.i8()? as i32).clamp(0, 3);
        let mut lightbar_siren_mode = (r.i8()? as i32).clamp(0, 3);
        let n = r.i16()?;
        let mut parts = Vec::new();
        for _ in 0..n.max(0) {
            let (p, siren_off) = VehiclePart::load(r, wv, items)?;
            if siren_off {
                lightbar_siren_mode = 0;
            }
            parts.push(p);
        }
        let has_engine = parts.iter().any(|p| p.id() == "Engine");
        let key_is_on_door = r.bool()?;
        let hotwired = r.bool()?;
        let hotwired_broken = r.bool()?;
        let keys_in_ignition = r.bool()?;
        let rust = r.f32()?;
        let color_hue = r.f32()?;
        let color_saturation = r.f32()?;
        let color_value = r.f32()?;
        let engine_power = r.i32()?;
        let engine = has_engine.then_some(VehicleEngine {
            running: engine_running,
            loudness: engine_loudness,
            quality: engine_quality,
            power: engine_power,
        });
        let loaded_vehicle_id = r.i16()?;
        if wv < 229 {
            let _ = r.string()?;
        }
        let mechanical_id = r.i32()?;
        let alarmed = r.bool()?;
        let (alarm_start_time, alarm_chosen_sound) = if wv >= 229 {
            let t = r.f64()?;
            let s = r.string()?;
            (t, Some(s).filter(|s| !s.trim().is_empty()))
        } else {
            (0.0, None)
        };
        let siren_start_time = r.f64()?;
        let current_key = if r.bool()? {
            // InventoryItem.loadItem: a load error is logged and the key
            // dropped (the record is still consumed).
            ItemBlob::load(r, wv, items)?
        } else {
            None
        };
        let n = r.i8()?;
        let mut blood_intensity: Vec<(String, i8)> = Vec::new();
        for _ in 0..n.max(0) {
            let id = r.string()?;
            let v = r.i8()?;
            match blood_intensity.iter_mut().find(|(k, _)| *k == id) {
                Some(e) => e.1 = v,
                None => blood_intensity.push((id, v)),
            }
        }
        let towing = if r.bool()? {
            Some(VehicleTowing {
                vehicle_id: r.i32()?,
                attachment_self: r.string()?,
                attachment_other: r.string()?,
                z_offset: r.f32()?,
            })
        } else {
            None
        };
        let regulator_speed = r.f32()?;
        let previously_entered = r.bool()?;
        let previously_moved = if wv >= 196 { r.bool()? } else { false };
        let animals = if wv >= 212 {
            let size = r.i32()?;
            if size < 1 {
                return err(format!("vehicle animal block of {size} bytes"));
            }
            let b = r.bytes(size as usize)?.to_vec();
            if b[0] != 0 && b.len() < 5 {
                return err("vehicle animal block too short");
            }
            b
        } else {
            return err("BaseVehicle from world version < 212");
        };
        Ok(BaseVehicle {
            base,
            saved_physics_z,
            saved_rot,
            script_name,
            skin_index,
            engine,
            front_end_durability,
            rear_end_durability,
            current_front_end_durability,
            current_rear_end_durability,
            key_id,
            key_spawned,
            headlights_on,
            created,
            sound_horn_on,
            sound_back_move_on,
            lightbar_lights_mode,
            lightbar_siren_mode,
            parts,
            key_is_on_door,
            hotwired,
            hotwired_broken,
            keys_in_ignition,
            rust,
            color_hue,
            color_saturation,
            color_value,
            vehicle_id: -1,
            loaded_vehicle_id,
            mechanical_id,
            alarmed,
            alarm_start_time,
            alarm_chosen_sound,
            siren_start_time,
            current_key,
            blood_intensity,
            towing,
            regulator_speed,
            previously_entered,
            previously_moved,
            animals,
        })
    }

    /// Whether the trailer holds animals (their records are not decoded).
    pub fn has_animals(&self) -> bool {
        self.animals.first().is_some_and(|&b| b != 0)
    }

    /// `jniTransform.origin.y` after load: the saved physics height, or the
    /// level when it is NaN.
    pub fn physics_origin_y(&self) -> f32 {
        if self.saved_physics_z.is_nan() {
            self.base.z
        } else {
            self.saved_physics_z
        }
    }

    /// `VehicleAlarm.getChosenSound()`: `None` without a script or with the
    /// alarm disabled; a sound the script does not list is replaced by a
    /// random pick, which is not reproduced here (error).
    fn chosen_sound(&self, sounds: Option<&AlarmSounds<'_>>) -> SaveResult<Option<&str>> {
        let Some(s) = sounds.filter(|s| s.alarm_enable) else {
            return Ok(None);
        };
        match self.alarm_chosen_sound.as_deref() {
            Some(c) if s.alarm.iter().chain(s.alarm_loop).any(|x| x == c) => Ok(Some(c)),
            _ if s.alarm.is_empty() && s.alarm_loop.is_empty() => Ok(None),
            _ => err("VehicleAlarm.chooseSound needs Rand"),
        }
    }

    /// `BaseVehicle.save` (including the serialise and class bytes).
    ///
    /// `square` is the `(x, y)` of the vehicle's square, which clamps the
    /// saved position into it (`None` when the vehicle has none). `sounds`
    /// are its script's alarm sounds once a script is attached; `None`
    /// behaves like a vehicle saved straight after loading.
    pub fn save(
        &self,
        w: &mut Writer,
        class_id: i8,
        square: Option<(i32, i32)>,
        sounds: Option<&AlarmSounds<'_>>,
        items: &dyn ItemRegistry,
    ) -> SaveResult<()> {
        let (mut x, mut y) = (self.base.x, self.base.y);
        if let Some((sx, sy)) = square {
            let d = 5.0e-4f32;
            x = clamp_f(x, sx as f32 + d, sx as f32 + 1.0 - d);
            y = clamp_f(y, sy as f32 + d, sy as f32 + 1.0 - d);
        }
        self.base.save_at(w, class_id, x, y);
        w.f32(self.physics_origin_y());
        for v in rotation_round_trip(self.saved_rot) {
            w.f32(v);
        }
        w.string(&self.script_name);
        w.i32(self.skin_index);
        let e = self.engine.clone().unwrap_or_default();
        w.bool(e.running);
        w.i32(self.front_end_durability);
        w.i32(self.rear_end_durability);
        w.i32(self.current_front_end_durability);
        w.i32(self.current_rear_end_durability);
        w.i32(e.loudness);
        w.i32(e.quality);
        w.i32(self.key_id);
        w.i8(self.key_spawned);
        w.bool(self.headlights_on);
        w.bool(self.created);
        w.bool(self.sound_horn_on);
        w.bool(self.sound_back_move_on);
        w.i8(self.lightbar_lights_mode as i8);
        w.i8(self.lightbar_siren_mode as i8);
        w.i16(self.parts.len() as i16);
        for p in &self.parts {
            p.save(w, items)?;
        }
        w.bool(self.key_is_on_door);
        w.bool(self.hotwired);
        w.bool(self.hotwired_broken);
        w.bool(self.keys_in_ignition);
        w.f32(self.rust);
        w.f32(self.color_hue);
        w.f32(self.color_saturation);
        w.f32(self.color_value);
        w.i32(e.power);
        w.i16(self.vehicle_id);
        w.i32(self.mechanical_id);
        w.bool(self.alarmed);
        w.f64(self.alarm_start_time);
        w.opt_string(self.chosen_sound(sounds)?);
        w.f64(self.siren_start_time);
        match &self.current_key {
            Some(k) => {
                w.i8(1);
                k.save(w, items)?;
            }
            None => w.i8(0),
        }
        let mut blood = self.blood_intensity.clone();
        sort_string_hash_map(&mut blood, |e| e.0.as_str());
        w.i8(blood.len() as i8);
        for (k, v) in &blood {
            w.string(k);
            w.i8(*v);
        }
        match self.towing.as_ref().filter(|t| t.vehicle_id != -1) {
            Some(t) => {
                w.i8(1);
                w.i32(t.vehicle_id);
                w.string(&t.attachment_self);
                w.string(&t.attachment_other);
                w.f32(t.z_offset);
            }
            None => w.i8(0),
        }
        w.f32(self.regulator_speed);
        w.bool(self.previously_entered);
        w.bool(self.previously_moved);
        w.i32(self.animals.len() as i32);
        w.bytes(&self.animals);
        Ok(())
    }
}

/// A row of the `vehicles` table of `vehicles.db`.
#[derive(Clone, Debug, PartialEq)]
pub struct VehicleRow {
    pub id: i32,
    pub wx: i32,
    pub wy: i32,
    pub x: f32,
    pub y: f32,
    pub world_version: i32,
    pub in_meta: bool,
    /// `BaseVehicle.save` output.
    pub data: Vec<u8>,
}

impl VehicleRow {
    /// The square `VehiclesDB2.WorldStreamerThread.vehicleLoaded` gives the
    /// vehicle: `chunk.getGridSquare((int)(x - wx*8), (int)(y - wy*8), 0)`
    /// (`None` outside the chunk).
    pub fn square(&self) -> Option<(i32, i32)> {
        let lx = (self.x - (self.wx * 8) as f32) as i32;
        let ly = (self.y - (self.wy * 8) as f32) as i32;
        ((0..8).contains(&lx) && (0..8).contains(&ly)).then(|| (self.wx * 8 + lx, self.wy * 8 + ly))
    }

    /// `SQLStore.loadChunk` + `vehicleLoaded`: `None` when the blob's
    /// serialise flag is clear or its class is not `vehicle_class`.
    pub fn load(
        &self,
        vehicle_class: i8,
        offsets: (f32, f32),
        items: &dyn ItemRegistry,
    ) -> SaveResult<Option<BaseVehicle>> {
        let mut r = Reader::new(&self.data);
        let serialise = r.bool()?;
        let class = r.i8()?;
        if class != vehicle_class || !serialise {
            return Ok(None);
        }
        BaseVehicle::load(&mut r, self.world_version, offsets, items).map(Some)
    }
}
