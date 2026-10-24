//! `zombie.scripting.objects.VehicleScript`.

use super::inner::{clamp, map_put};
use super::{
    at, discard_null_or_whitespace, non_empty, npe, parse_float, parse_int, Anim, Area, Container, Door,
    ImmutableColor, LightBar, LuaKey, LuaTable, LuaValue, Model, Part, Passenger, PhysicsShape,
    Position, Skin, Sounds, SwitchSeat, VehicleTemplate, Wheel, Window, PHYSICS_SHAPE_BOX, PHYSICS_SHAPE_MESH,
    PHYSICS_SHAPE_SPHERE,
};
use crate::base::{index_error, BaseScriptObject, ScriptError, ScriptObject, ScriptResult};
use crate::java;
use crate::manager::ScriptManager;
use crate::math::{Vector2, Vector3};
use crate::objects::models::ModelAttachment;
use crate::parser::{Block, BlockElement};
use crate::script_type::ScriptType;

/// `steeringClampMax`.
const STEERING_CLAMP_MAX: f32 = 0.9;

/// Size of the parameter array `toBullet` fills.
const BULLET_PARAMS_LEN: usize = 200;

#[derive(Debug)]
pub struct VehicleScript {
    pub base: BaseScriptObject,
    file_name: Option<String>,
    name: Option<String>,
    models: Vec<Model>,
    pub attachments: Vec<ModelAttachment>,
    mass: f32,
    center_of_mass_offset: Vector3,
    engine_force: f32,
    engine_idle_speed: f32,
    steering_increment: f32,
    steering_clamp: f32,
    wheel_friction: f32,
    stopping_movement_force: f32,
    animal_trailer_size: f32,
    suspension_stiffness: f32,
    suspension_damping: f32,
    suspension_compression: f32,
    suspension_rest_length: f32,
    max_suspension_travel_cm: f32,
    roll_influence: f32,
    extents: Vector3,
    shadow_extents: Vector2,
    shadow_offset: Vector2,
    had_shadow_o_extents: bool,
    had_shadow_offset: bool,
    extents_offset: Vector2,
    physics_chassis_shape: Vector3,
    physics_shapes: Vec<PhysicsShape>,
    wheels: Vec<Wheel>,
    crawl_through_wheels: Vec<Wheel>,
    passengers: Vec<Passenger>,
    pub max_speed: f32,
    use_chassis_physics_collision: bool,
    pub max_speed_reverse: f32,
    pub is_small_vehicle: bool,
    front_end_health: i32,
    rear_end_health: i32,
    storage_capacity: i32,
    engine_loudness: i32,
    engine_quality: i32,
    seats: i32,
    mechanic_type: i32,
    engine_repair_level: i32,
    player_damage_protection: f32,
    forced_hue: f32,
    forced_sat: f32,
    forced_val: f32,
    pub left_siren_col: Option<ImmutableColor>,
    pub right_siren_col: Option<ImmutableColor>,
    engine_rpm_type: String,
    offroad_efficiency: f32,
    crawl_offsets: Vec<f32>,
    zombie_type: Option<Vec<String>>,
    special_key_ring: Option<Vec<String>>,
    not_kill_crops: bool,
    has_lighter: bool,
    car_mechanics_overlay: Option<String>,
    car_model_name: Option<String>,
    special_loot_chance: i32,
    special_key_ring_chance: i32,
    never_spawn_key: bool,
    pub gear_ratio_count: i32,
    pub gear_ratio: [f32; 9],
    textures: Skin,
    skins: Vec<Skin>,
    areas: Vec<Area>,
    parts: Vec<Part>,
    has_siren: bool,
    lightbar: LightBar,
    sound: Sounds,
    pub texture_mask_enable: bool,
    bullet_params: Option<Vec<f32>>,
}

impl Default for VehicleScript {
    fn default() -> Self {
        Self::new()
    }
}

impl VehicleScript {
    pub fn new() -> Self {
        let mut gear_ratio = [0.0; 9];
        gear_ratio[..6].copy_from_slice(&[7.09, 6.44, 4.1, 2.29, 1.47, 1.0]);
        VehicleScript {
            base: BaseScriptObject::new(ScriptType::Vehicle),
            file_name: None,
            name: None,
            models: Vec::new(),
            attachments: Vec::new(),
            mass: 800.0,
            center_of_mass_offset: Vector3::default(),
            engine_force: 3000.0,
            engine_idle_speed: 750.0,
            steering_increment: 0.04,
            steering_clamp: 0.4,
            wheel_friction: 800.0,
            stopping_movement_force: 1.0,
            animal_trailer_size: 0.0,
            suspension_stiffness: 20.0,
            suspension_damping: 2.3,
            suspension_compression: 4.4,
            suspension_rest_length: 0.6,
            max_suspension_travel_cm: 500.0,
            roll_influence: 0.1,
            extents: Vector3::new(0.75, 0.5, 2.0),
            shadow_extents: Vector2::default(),
            shadow_offset: Vector2::default(),
            had_shadow_o_extents: false,
            had_shadow_offset: false,
            extents_offset: Vector2::new(0.5, 0.5),
            physics_chassis_shape: Vector3::default(),
            physics_shapes: Vec::new(),
            wheels: Vec::new(),
            crawl_through_wheels: Vec::new(),
            passengers: Vec::new(),
            max_speed: 20.0,
            use_chassis_physics_collision: true,
            max_speed_reverse: 40.0,
            is_small_vehicle: true,
            front_end_health: 100,
            rear_end_health: 100,
            storage_capacity: 100,
            engine_loudness: 100,
            engine_quality: 100,
            seats: 2,
            mechanic_type: 0,
            engine_repair_level: 0,
            player_damage_protection: 0.0,
            forced_hue: -1.0,
            forced_sat: -1.0,
            forced_val: -1.0,
            left_siren_col: None,
            right_siren_col: None,
            engine_rpm_type: "jeep".to_string(),
            offroad_efficiency: 1.0,
            crawl_offsets: Vec::new(),
            zombie_type: None,
            special_key_ring: None,
            not_kill_crops: false,
            has_lighter: true,
            car_mechanics_overlay: None,
            car_model_name: None,
            special_loot_chance: 8,
            special_key_ring_chance: 0,
            never_spawn_key: false,
            gear_ratio_count: 4,
            gear_ratio,
            textures: Skin::default(),
            skins: Vec::new(),
            areas: Vec::new(),
            parts: Vec::new(),
            has_siren: false,
            lightbar: LightBar::default(),
            sound: Sounds::default(),
            texture_mask_enable: false,
            bullet_params: None,
        }
    }

    /// `Load(name, totalFile)`.
    pub fn load_body(&mut self, sm: &ScriptManager, name: &str, total_file: &str) -> ScriptResult {
        let root = sm.parse(total_file);
        let block = root.children.first().ok_or_else(index_error)?;
        self.base.load_common_block(block);
        for element in &block.elements {
            match *element {
                BlockElement::Value(i) => self.load_value(sm, name, &block.values[i].string)?,
                BlockElement::Block(i) => self.load_child(&block.children[i])?,
            }
        }
        Ok(())
    }

    fn load_value(&mut self, sm: &ScriptManager, name: &str, string: &str) -> ScriptResult {
        let ss = java::split_char(string, '=');
        let k = java::trim(at(&ss, 0)?);
        let v = java::trim(at(&ss, 1)?);
        match k {
            "extents" => self.extents = load_vector3f(v)?,
            "shadowExtents" => {
                self.shadow_extents = load_vector2f(v)?;
                self.had_shadow_o_extents = true;
            }
            "shadowOffset" => {
                self.shadow_offset = load_vector2f(v)?;
                self.had_shadow_offset = true;
            }
            "physicsChassisShape" => self.physics_chassis_shape = load_vector3f(v)?,
            "extentsOffset" => self.extents_offset = load_vector2f(v)?,
            "mass" => self.mass = parse_float(v)?,
            _ if java::eq_ignore_case("offRoadEfficiency", k) => self.offroad_efficiency = parse_float(v)?,
            "centerOfMassOffset" => self.center_of_mass_offset = load_vector3f(v)?,
            "engineForce" => self.engine_force = parse_float(v)?,
            "engineIdleSpeed" => self.engine_idle_speed = parse_float(v)?,
            "gearRatioCount" => self.gear_ratio_count = parse_int(v)?,
            "gearRatioR" => self.gear_ratio[0] = parse_float(v)?,
            "gearRatio1" => self.gear_ratio[1] = parse_float(v)?,
            "gearRatio2" => self.gear_ratio[2] = parse_float(v)?,
            "gearRatio3" => self.gear_ratio[3] = parse_float(v)?,
            "gearRatio4" => self.gear_ratio[4] = parse_float(v)?,
            "gearRatio5" => self.gear_ratio[5] = parse_float(v)?,
            "gearRatio6" => self.gear_ratio[6] = parse_float(v)?,
            "gearRatio7" => self.gear_ratio[7] = parse_float(v)?,
            "gearRatio8" => self.gear_ratio[8] = parse_float(v)?,
            "textureMaskEnable" => self.texture_mask_enable = java::parse_bool(v),
            "textureRust" => self.textures.texture_rust = discard_null_or_whitespace(v),
            "textureMask" => self.textures.texture_mask = discard_null_or_whitespace(v),
            "textureLights" => self.textures.texture_lights = discard_null_or_whitespace(v),
            "textureDamage1Overlay" => self.textures.texture_damage1_overlay = discard_null_or_whitespace(v),
            "textureDamage1Shell" => self.textures.texture_damage1_shell = discard_null_or_whitespace(v),
            "textureDamage2Overlay" => self.textures.texture_damage2_overlay = discard_null_or_whitespace(v),
            "textureDamage2Shell" => self.textures.texture_damage2_shell = discard_null_or_whitespace(v),
            "textureShadow" => self.textures.texture_shadow = discard_null_or_whitespace(v),
            "rollInfluence" => self.roll_influence = parse_float(v)?,
            "steeringIncrement" => self.steering_increment = parse_float(v)?,
            "steeringClamp" => self.steering_clamp = parse_float(v)?,
            "suspensionStiffness" => self.suspension_stiffness = parse_float(v)?,
            "suspensionDamping" => self.suspension_damping = parse_float(v)?,
            "suspensionCompression" => self.suspension_compression = parse_float(v)?,
            "suspensionRestLength" => self.suspension_rest_length = parse_float(v)?,
            "maxSuspensionTravelCm" => self.max_suspension_travel_cm = parse_float(v)?,
            "wheelFriction" => self.wheel_friction = parse_float(v)?,
            "stoppingMovementForce" => self.stopping_movement_force = parse_float(v)?,
            _ if java::eq_ignore_case("animalTrailerSize", k) => self.animal_trailer_size = parse_float(v)?,
            "maxSpeed" => self.max_speed = parse_float(v)?,
            "maxSpeedReverse" => self.max_speed_reverse = parse_float(v)?,
            "isSmallVehicle" => self.is_small_vehicle = java::parse_bool(v),
            "frontEndDurability" => self.front_end_health = parse_int(v)?,
            "rearEndDurability" => self.rear_end_health = parse_int(v)?,
            "storageCapacity" => self.storage_capacity = parse_int(v)?,
            "engineLoudness" => self.engine_loudness = parse_int(v)?,
            "engineQuality" => self.engine_quality = parse_int(v)?,
            "seats" => self.seats = parse_int(v)?,
            "hasSiren" => self.has_siren = java::parse_bool(v),
            "mechanicType" => self.mechanic_type = parse_int(v)?,
            "forcedColor" => {
                let hsv = java::split_char(v, ' ');
                self.forced_hue = parse_float(at(&hsv, 0)?)?;
                self.forced_sat = parse_float(at(&hsv, 1)?)?;
                self.forced_val = parse_float(at(&hsv, 2)?)?;
            }
            "engineRPMType" => self.engine_rpm_type = java::trim(v).to_string(),
            "zombieType" => {
                let split = java::split_char(v, ';');
                self.zombie_type = Some(split.iter().map(|s| java::trim(s).to_string()).collect());
            }
            "specialKeyRing" => {
                let split = java::split_char(v, ';');
                self.special_key_ring = Some(
                    split
                        .iter()
                        .map(|s| java::trim(s))
                        .filter(|s| !java::is_null_or_whitespace(Some(s)))
                        .map(str::to_string)
                        .collect(),
                );
            }
            "notKillCrops" => self.not_kill_crops = java::parse_bool(v),
            "hasLighter" => self.has_lighter = java::parse_bool(v),
            "neverSpawnKey" => self.never_spawn_key = java::parse_bool(v),
            "carMechanicsOverlay" => self.car_mechanics_overlay = Some(java::trim(v).to_string()),
            "carModelName" => self.car_model_name = Some(java::trim(v).to_string()),
            "specialLootChance" => self.special_loot_chance = parse_int(v)?,
            "specialKeyRingChance" => self.special_key_ring_chance = parse_int(v)?,
            "template" => self.load_template(sm, v)?,
            "template!" => match sm.get::<VehicleTemplate>(ScriptType::VehicleTemplate, v) {
                None => sm.log(&format!(
                    "ERROR: template \"{v}\" not found in: {}",
                    self.file_name.as_deref().unwrap_or("null")
                )),
                Some(t) => {
                    let body = t.borrow().body.clone();
                    self.load_body(sm, name, &body)?;
                }
            },
            "engineRepairLevel" => self.engine_repair_level = parse_int(v)?,
            "playerDamageProtection" => self.player_damage_protection = parse_float(v)?,
            "useChassisPhysicsCollision" => self.use_chassis_physics_collision = java::parse_bool(v),
            _ => {}
        }
        Ok(())
    }

    fn load_child(&mut self, child: &Block) -> ScriptResult {
        match child.type_.as_deref() {
            Some("area") => self.load_area(child)?,
            Some("attachment") => self.load_attachment(child)?,
            Some("model") => load_model(child, &mut self.models)?,
            Some("part") => match child.id.as_deref() {
                Some(pattern) if pattern.contains('*') => {
                    let mut child = child.clone();
                    let mut i = 0;
                    while i < self.parts.len() {
                        let id = self.parts[i].id.clone();
                        if glob_match(pattern, id.as_deref())? {
                            child.id = id;
                            self.load_part(&child)?;
                        }
                        i += 1;
                    }
                }
                _ => self.load_part(child)?,
            },
            Some("passenger") => match child.id.as_deref() {
                Some(pattern) if pattern.contains('*') => {
                    let mut child = child.clone();
                    let mut i = 0;
                    while i < self.passengers.len() {
                        let id = self.passengers[i].id.clone();
                        if glob_match(pattern, id.as_deref())? {
                            child.id = id;
                            self.load_passenger(&child)?;
                        }
                        i += 1;
                    }
                }
                _ => self.load_passenger(child)?,
            },
            Some("physics") => {
                if let Some(shape) = load_physics_shape(child)? {
                    if self.physics_shapes.len() < 10 {
                        self.physics_shapes.push(shape);
                    }
                }
            }
            Some("skin") => {
                let skin = load_skin(child);
                if !java::is_null_or_whitespace(skin.texture.as_deref()) {
                    self.skins.push(skin);
                }
            }
            Some("wheel") => self.load_wheel(child, false)?,
            Some("crawlThroughWheel") => self.load_wheel(child, true)?,
            Some("lightbar") => self.load_lightbar(child)?,
            Some("sound") => self.load_sound(child),
            _ => {}
        }
        Ok(())
    }

    fn load_lightbar(&mut self, child: &Block) -> ScriptResult {
        for value in &child.values {
            let k = java::trim(value.get_key());
            let v = java::trim(value.get_value());
            if k == "soundSiren" {
                self.lightbar.sound_siren0 = format!("{v}Yelp");
                self.lightbar.sound_siren1 = format!("{v}Wall");
                self.lightbar.sound_siren2 = format!("{v}Alarm");
            }
            if k == "soundSiren0" {
                self.lightbar.sound_siren0 = v.to_string();
            }
            if k == "soundSiren1" {
                self.lightbar.sound_siren1 = v.to_string();
            }
            if k == "soundSiren2" {
                self.lightbar.sound_siren2 = v.to_string();
            }
            if k == "leftCol" {
                self.left_siren_col = Some(load_color(v)?);
            }
            if k == "rightCol" {
                self.right_siren_col = Some(load_color(v)?);
            }
            self.lightbar.enable = true;
            if self.part_by_id(Some("lightbar")).is_none() {
                self.parts.push(Part { id: Some("lightbar".to_string()), ..Part::default() });
            }
        }
        Ok(())
    }

    fn load_sound(&mut self, child: &Block) {
        let sound = &mut self.sound;
        for value in &child.values {
            let k = java::trim(value.get_key());
            let v = java::trim(value.get_value());
            match k {
                "alarm" => {
                    let ss = java::split_ws(v);
                    if !ss.is_empty() {
                        sound.alarm.extend(ss.iter().map(|s| s.to_string()));
                        sound.alarm_enable = true;
                    }
                }
                "alarmLoop" => {
                    let ss = java::split_ws(v);
                    if !ss.is_empty() {
                        sound.alarm_loop.extend(ss.iter().map(|s| s.to_string()));
                        sound.alarm_enable = true;
                    }
                }
                "backSignal" => {
                    sound.back_signal = discard_null_or_whitespace(v);
                    sound.back_signal_enable = sound.back_signal.is_some();
                }
                "engine" => sound.engine = discard_null_or_whitespace(v),
                "engineStart" => sound.engine_start = discard_null_or_whitespace(v),
                "engineTurnOff" => sound.engine_turn_off = discard_null_or_whitespace(v),
                "handBrake" => sound.hand_brake = discard_null_or_whitespace(v),
                "horn" => {
                    sound.horn = discard_null_or_whitespace(v);
                    sound.horn_enable = sound.horn.is_some();
                }
                "ignitionFail" => sound.ignition_fail = discard_null_or_whitespace(v),
                "ignitionFailNoPower" => sound.ignition_fail_no_power = discard_null_or_whitespace(v),
                _ => {}
            }
            sound.specified.insert(k.to_string(), ());
        }
    }

    /// `getFileName`.
    pub fn file_name(&self) -> Option<&str> {
        self.file_name.as_deref()
    }

    /// `Loaded`: scales the script by its model's scale, resolves seat
    /// indices and item types, and builds the Bullet vehicle definition.
    pub fn loaded(&mut self, sm: &ScriptManager) -> ScriptResult {
        let scale = self.model_scale();
        self.extents.mul(scale);
        self.max_suspension_travel_cm *= scale;
        self.suspension_rest_length *= scale;
        self.center_of_mass_offset.mul(scale);
        self.physics_chassis_shape.mul(scale);
        if self.had_shadow_o_extents {
            self.shadow_extents.mul(scale);
        } else {
            self.shadow_extents.set(self.extents.x, self.extents.z);
        }
        if self.had_shadow_offset {
            self.shadow_offset.mul(scale);
        } else {
            self.shadow_offset.set(self.center_of_mass_offset.x, self.center_of_mass_offset.z);
        }
        for model in &mut self.models {
            model.offset.mul(scale);
        }
        for attachment in &mut self.attachments {
            attachment.offset_mut().mul(scale);
        }
        for shape in &mut self.physics_shapes {
            shape.offset.mul(scale);
            match shape.type_ {
                PHYSICS_SHAPE_BOX | PHYSICS_SHAPE_MESH => shape.extents.mul(scale),
                PHYSICS_SHAPE_SPHERE => shape.radius *= scale,
                _ => {}
            }
        }
        for wheel in &mut self.wheels {
            wheel.radius *= scale;
            wheel.offset.mul(scale);
        }
        for area in &mut self.areas {
            area.x *= scale;
            area.y *= scale;
            area.w *= scale;
            area.h *= scale;
        }
        if self.has_physics_chassis_shape() && !vec3_equals(&self.extents, &self.physics_chassis_shape) {
            sm.script_warn(&format!(
                "vehicle \"{}\" extents != physicsChassisShape",
                self.name.as_deref().unwrap_or("null")
            ));
        }
        for i in 0..self.passengers.len() {
            for posn in &mut self.passengers[i].positions {
                posn.offset.mul(scale);
            }
            for j in 0..self.passengers[i].switch_seats.len() {
                let seat = self.passenger_index(self.passengers[i].switch_seats[j].id.as_deref());
                self.passengers[i].switch_seats[j].seat = seat;
            }
        }
        for i in 0..self.parts.len() {
            let seat_id = self.parts[i].container.as_ref().and_then(|c| c.seat_id.clone());
            if let Some(seat_id) = seat_id.filter(|s| !s.is_empty()) {
                let seat = self.passenger_index(Some(&seat_id));
                self.parts[i].container.as_mut().unwrap().seat = seat;
            }
            let mechanic_type = self.mechanic_type;
            let part = &mut self.parts[i];
            if part.specific_item {
                if let Some(item_type) = &mut part.item_type {
                    for t in item_type.iter_mut() {
                        t.push_str(&mechanic_type.to_string());
                    }
                }
            }
        }
        if !self.sound.alarm_enable && self.sound.horn_enable {
            self.sound.alarm_enable = true;
            // hornEnable implies horn != null.
            let horn = self.sound.horn.clone().unwrap_or_default();
            self.sound.alarm.push(horn);
        }
        self.init_crawl_offsets()?;
        if let Some(ring) = &mut self.special_key_ring {
            let module = self.base.module().ok_or_else(|| npe("module"))?;
            for item_type in ring.iter_mut() {
                // Entries are never blank, so this always resolves.
                if let Some(t) = sm.resolve_item_type(&module, item_type) {
                    *item_type = t;
                }
            }
        }
        // compact() only trims list capacities.
        // TODO(network): skipped when GameServer.server.
        self.to_bullet()
    }

    /// `toBullet`: the parameter block passed to
    /// `Bullet.defineVehicleScript(getFullName(), params)`.
    pub fn to_bullet(&mut self) -> ScriptResult {
        let mut params = Vec::with_capacity(64);
        let mut put = |v: f32| -> ScriptResult {
            if params.len() == BULLET_PARAMS_LEN {
                return Err(ScriptError::new(format!(
                    "java.lang.ArrayIndexOutOfBoundsException: Index {BULLET_PARAMS_LEN} out of bounds for length {BULLET_PARAMS_LEN}"
                )));
            }
            params.push(v);
            Ok(())
        };
        put(self.model_scale())?;
        put(self.mass)?;
        put(self.roll_influence)?;
        put(self.suspension_stiffness)?;
        put(self.suspension_compression)?;
        put(self.suspension_damping)?;
        put(self.max_suspension_travel_cm)?;
        put(self.suspension_rest_length)?;
        // SystemDisabler.getdoHighFriction() is false.
        put(self.wheel_friction)?;
        put(self.stopping_movement_force)?;
        put(self.wheels.len() as f32)?;
        let com = self.center_of_mass_offset;
        for wheel in &self.wheels {
            let model = self.model().ok_or_else(|| npe("getModel()"))?.offset;
            put(if wheel.front { 1.0 } else { 0.0 })?;
            put(wheel.offset.x + model.x - 0.0 * com.x)?;
            put(wheel.offset.y + model.y - 0.0 * com.y + 1.0 * self.suspension_rest_length)?;
            put(wheel.offset.z + model.z - 0.0 * com.z)?;
            put(wheel.radius)?;
        }
        let chassis = self.has_physics_chassis_shape() && self.use_chassis_physics_collision;
        let mut num_shapes = chassis as usize + self.physics_shapes.len();
        if num_shapes == 0 {
            num_shapes = 1;
        }
        put(num_shapes as f32)?;
        let box_shape = if chassis {
            Some(self.physics_chassis_shape)
        } else if self.physics_shapes.is_empty() {
            Some(self.extents)
        } else {
            None
        };
        if let Some(e) = box_shape {
            for v in [1.0, com.x, com.y, com.z, e.x, e.y, e.z, 0.0, 0.0, 0.0] {
                put(v)?;
            }
        }
        for shape in &self.physics_shapes {
            put(shape.type_ as f32)?;
            put(shape.offset.x)?;
            put(shape.offset.y)?;
            put(shape.offset.z)?;
            if shape.type_ == PHYSICS_SHAPE_BOX {
                for v in [shape.extents.x, shape.extents.y, shape.extents.z, shape.rotate.x, shape.rotate.y, shape.rotate.z]
                {
                    put(v)?;
                }
            } else if shape.type_ == PHYSICS_SHAPE_SPHERE {
                put(shape.radius)?;
            }
        }
        // TODO(physics): Bullet.defineVehicleScript(getFullName(), params).
        self.bullet_params = Some(params);
        Ok(())
    }

    /// The parameters of the last `toBullet` call (the used prefix of the
    /// original's 200-float array).
    pub fn bullet_params(&self) -> Option<&[f32]> {
        self.bullet_params.as_deref()
    }

    fn load_attachment(&mut self, block: &Block) -> ScriptResult {
        let idx = match self.attachment_index(block.id.as_deref()) {
            Some(i) => i,
            None => {
                // isNullOrWhitespace(null) rejects a missing id like a blank one.
                self.attachments.push(ModelAttachment::new(block.id.as_deref().unwrap_or(""))?);
                self.attachments.len() - 1
            }
        };
        let attachment = &mut self.attachments[idx];
        for value in &block.values {
            let k = java::trim(value.get_key());
            let v = java::trim(value.get_value());
            match k {
                "bone" => attachment.set_bone(v),
                "offset" => *attachment.offset_mut() = load_vector3f(v)?,
                "rotate" => *attachment.rotate_mut() = load_vector3f(v)?,
                "canAttach" => {
                    attachment.set_can_attach(Some(java::split_char(v, ';').iter().map(|s| s.to_string()).collect()))
                }
                "zoffset" => attachment.set_zoffset(parse_float(v)?),
                "updateconstraint" => attachment.set_update_constraint(java::parse_bool(v)),
                _ => {}
            }
        }
        Ok(())
    }

    fn load_wheel(&mut self, block: &Block, crawl_through_wheel: bool) -> ScriptResult {
        let wheel = match self.wheel_index(block.id.as_deref()) {
            Some(i) => &mut self.wheels[i],
            None => {
                let list = if crawl_through_wheel { &mut self.crawl_through_wheels } else { &mut self.wheels };
                list.push(Wheel { id: block.id.clone(), ..Wheel::default() });
                list.last_mut().unwrap()
            }
        };
        for value in &block.values {
            let k = java::trim(value.get_key());
            let v = java::trim(value.get_value());
            match k {
                "model" => wheel.model = Some(v.to_string()),
                "front" => wheel.front = java::parse_bool(v),
                "offset" => wheel.offset = load_vector3f(v)?,
                "radius" => wheel.radius = parse_float(v)?,
                "width" => wheel.width = parse_float(v)?,
                _ => {}
            }
        }
        Ok(())
    }

    fn load_passenger(&mut self, block: &Block) -> ScriptResult {
        let idx = match self.passenger_index(block.id.as_deref()) {
            -1 => {
                self.passengers.push(Passenger { id: block.id.clone(), ..Passenger::default() });
                self.passengers.len() - 1
            }
            i => i as usize,
        };
        let pngr = &mut self.passengers[idx];
        for value in &block.values {
            let k = java::trim(value.get_key());
            let v = java::trim(value.get_value());
            match k {
                "area" => pngr.area = Some(v.to_string()),
                "door" => pngr.door = Some(v.to_string()),
                "door2" => pngr.door2 = Some(v.to_string()),
                "hasRoof" => pngr.has_roof = java::parse_bool(v),
                "showPassenger" => pngr.show_passenger = java::parse_bool(v),
                _ => {}
            }
        }
        for child in &block.children {
            match child.type_.as_deref() {
                Some("anim") => load_anim(child, &mut pngr.anims)?,
                Some("position") => load_position(child, &mut pngr.positions)?,
                Some("switchSeat") => load_switch_seat(child, pngr)?,
                _ => {}
            }
        }
        Ok(())
    }

    fn load_area(&mut self, block: &Block) -> ScriptResult {
        let idx = match self.index_of_area_by_id(block.id.as_deref()) {
            -1 => {
                let id = block.id.clone().ok_or_else(|| npe("block.id"))?;
                self.areas.push(Area { id: Some(id), ..Area::default() });
                self.areas.len() - 1
            }
            i => i as usize,
        };
        let area = &mut self.areas[idx];
        for value in &block.values {
            let k = java::trim(value.get_key());
            let v = java::trim(value.get_value());
            if k != "xywh" {
                continue;
            }
            let ss = java::split_char(v, ' ');
            area.x = parse_float(at(&ss, 0)?)?;
            area.y = parse_float(at(&ss, 1)?)?;
            area.w = parse_float(at(&ss, 2)?)?;
            area.h = parse_float(at(&ss, 3)?)?;
        }
        Ok(())
    }

    fn load_part(&mut self, block: &Block) -> ScriptResult {
        let full_name = self.full_name();
        let idx = match self.index_of_part_by_id(block.id.as_deref()) {
            -1 => {
                self.parts.push(Part { id: block.id.clone(), ..Part::default() });
                self.parts.len() - 1
            }
            i => i as usize,
        };
        let part = &mut self.parts[idx];
        for value in &block.values {
            let k = java::trim(value.get_key());
            let v = java::trim(value.get_value());
            if k == "area" {
                part.area = non_empty(v);
            }
            match k {
                "mechanicArea" => part.mechanic_area = Some(v.to_string()),
                "itemType" => {
                    part.item_type = Some(java::split_char(v, ';').iter().map(|s| s.to_string()).collect());
                }
                "parent" => part.parent = non_empty(v),
                "mechanicRequireKey" => part.mechanic_require_key = java::parse_bool(v),
                "repairMechanic" => part.repair_mechanic = java::parse_bool(v),
                "setAllModelsVisible" => part.set_all_models_visible = java::parse_bool(v),
                "wheel" => part.wheel = Some(v.to_string()),
                "category" => part.category = Some(v.to_string()),
                "durability" => part.durability = parse_float(v)?,
                "specificItem" => part.specific_item = java::parse_bool(v),
                "hasLightsRear" => part.has_lights_rear = java::parse_bool(v),
                _ => {}
            }
        }
        for child in &block.children {
            match child.type_.as_deref() {
                Some("anim") => load_anim(child, part.anims.get_or_insert_with(Vec::new))?,
                Some("container") => part.container = Some(load_container(child, part.container.take())?),
                Some("door") => part.door = Some(load_door(child)),
                Some("lua") => part.lua_functions = Some(load_lua_functions(child, &full_name)?),
                Some("model") => load_model(child, part.models.get_or_insert_with(Vec::new))?,
                Some("table") => {
                    let existing = part
                        .tables
                        .as_mut()
                        .and_then(|t| t.iter_mut().find(|(k, _)| *k == child.id))
                        .map(|(_, t)| std::mem::take(t));
                    let table = load_table(child, existing)?;
                    map_put(part.tables.get_or_insert_with(Vec::new), child.id.clone(), table);
                }
                Some("window") => part.window = Some(load_window(child)),
                _ => {}
            }
        }
        Ok(())
    }

    fn load_template(&mut self, sm: &ScriptManager, s: &str) -> ScriptResult {
        let error = || sm.log(&format!("ERROR: template \"{s}\""));
        if s.contains('/') {
            let ss: Vec<&str> = java::split_char(s, '/').into_iter().map(java::trim).collect();
            if ss.is_empty() || ss.len() > 3 {
                error();
                return Ok(());
            }
            if ss.iter().any(|p| p.is_empty()) {
                error();
                return Ok(());
            }
            let Some(template) = sm.get::<VehicleTemplate>(ScriptType::VehicleTemplate, ss[0]) else {
                sm.log(&format!("ERROR: template \"{s}\" not found A"));
                return Ok(());
            };
            let script = VehicleTemplate::get_script(&template, sm);
            let kind = at(&ss, 1)?;
            if !matches!(kind, "area" | "part" | "passenger" | "wheel" | "physics") || ss.len() == 2 {
                error();
                return Ok(());
            }
            let Some(other) = borrow_template(sm, &script, s) else { return Ok(()) };
            match kind {
                "area" => self.copy_areas_from(sm, &other, ss[2]),
                "part" => self.copy_parts_from(sm, &other, ss[2]),
                "passenger" => self.copy_passengers_from(sm, &other, ss[2]),
                "wheel" => self.copy_wheels_from(sm, &other, ss[2]),
                _ => self.copy_physics_from(&other, ss[2]),
            }
        } else {
            let Some(template) = sm.get::<VehicleTemplate>(ScriptType::VehicleTemplate, java::trim(s)) else {
                sm.log(&format!("ERROR: template \"{s}\" not found B"));
                return Ok(());
            };
            let script = VehicleTemplate::get_script(&template, sm);
            let Some(other) = borrow_template(sm, &script, s) else { return Ok(()) };
            self.copy_areas_from(sm, &other, "*");
            self.copy_parts_from(sm, &other, "*");
            self.copy_passengers_from(sm, &other, "*");
            self.copy_sound_from(&other, "*");
            self.copy_wheels_from(sm, &other, "*");
            self.copy_physics_from(&other, "*");
        }
        Ok(())
    }

    /// `copyAreasFrom`.
    pub fn copy_areas_from(&mut self, sm: &ScriptManager, other: &VehicleScript, spec: &str) {
        let copy = |this: &mut Self, a: &Area| match this.index_of_area_by_id(a.id.as_deref()) {
            -1 => this.areas.push(a.clone()),
            i => this.areas[i as usize] = a.clone(),
        };
        if spec == "*" {
            for a in &other.areas {
                copy(self, a);
            }
        } else {
            match other.area_by_id(Some(spec)) {
                Some(a) => copy(self, a),
                None => sm.log(&format!("ERROR: area \"{spec}\" not found")),
            }
        }
    }

    /// `copyPartsFrom`.
    pub fn copy_parts_from(&mut self, sm: &ScriptManager, other: &VehicleScript, spec: &str) {
        let copy = |this: &mut Self, p: &Part| match this.index_of_part_by_id(p.id.as_deref()) {
            -1 => this.parts.push(p.make_copy()),
            i => this.parts[i as usize] = p.make_copy(),
        };
        if spec == "*" {
            for p in &other.parts {
                copy(self, p);
            }
        } else {
            match other.part_by_id(Some(spec)) {
                Some(p) => copy(self, p),
                None => sm.log(&format!("ERROR: part \"{spec}\" not found")),
            }
        }
    }

    /// `copyPhysicsFrom`: replaces every shape; `spec` is ignored.
    pub fn copy_physics_from(&mut self, other: &VehicleScript, _spec: &str) {
        self.physics_shapes = other.physics_shapes.iter().map(PhysicsShape::make_copy).collect();
        self.use_chassis_physics_collision = other.use_chassis_physics_collision;
    }

    /// `copyPassengersFrom`.
    pub fn copy_passengers_from(&mut self, sm: &ScriptManager, other: &VehicleScript, spec: &str) {
        let copy = |this: &mut Self, p: &Passenger| match this.passenger_index(p.id.as_deref()) {
            -1 => this.passengers.push(p.make_copy()),
            i => this.passengers[i as usize] = p.make_copy(),
        };
        if spec == "*" {
            for p in &other.passengers {
                copy(self, p);
            }
        } else {
            match other.passenger_by_id(Some(spec)) {
                Some(p) => copy(self, p),
                None => sm.log(&format!("ERROR: passenger \"{spec}\" not found")),
            }
        }
    }

    /// `copySoundFrom`: only `"*"` copies anything.
    pub fn copy_sound_from(&mut self, other: &VehicleScript, spec: &str) {
        if spec != "*" {
            return;
        }
        for s in other.sound.specified.keys() {
            let (mine, theirs) = (&mut self.sound, &other.sound);
            match s {
                "backSignal" => {
                    mine.back_signal.clone_from(&theirs.back_signal);
                    mine.back_signal_enable = theirs.back_signal_enable;
                }
                "engine" => mine.engine.clone_from(&theirs.engine),
                "engineStart" => mine.engine_start.clone_from(&theirs.engine_start),
                "engineTurnOff" => mine.engine_turn_off.clone_from(&theirs.engine_turn_off),
                "handBrake" => mine.hand_brake.clone_from(&theirs.hand_brake),
                "ignitionFail" => mine.ignition_fail.clone_from(&theirs.ignition_fail),
                "ignitionFailNoPower" => mine.ignition_fail_no_power.clone_from(&theirs.ignition_fail_no_power),
                _ => {}
            }
            mine.specified.insert(s.to_string(), ());
        }
    }

    /// `copyWheelsFrom`.
    pub fn copy_wheels_from(&mut self, sm: &ScriptManager, other: &VehicleScript, spec: &str) {
        let copy = |this: &mut Self, w: &Wheel| match this.wheel_index(w.id.as_deref()) {
            None => this.wheels.push(w.clone()),
            Some(i) => this.wheels[i] = w.clone(),
        };
        if spec == "*" {
            for w in &other.wheels {
                copy(self, w);
            }
        } else {
            match other.wheel_by_id(Some(spec)) {
                Some(w) => copy(self, w),
                None => sm.log(&format!("ERROR: wheel \"{spec}\" not found")),
            }
        }
    }

    fn init_crawl_offsets(&mut self) -> ScriptResult {
        let mut wheels: Vec<Wheel> = Vec::new();
        for wheel in self.wheels.iter().chain(&self.crawl_through_wheels) {
            if wheel.id.as_deref().ok_or_else(|| npe("wheel.id"))?.contains("Left") {
                wheels.push(wheel.clone());
            }
        }
        for wheel in &wheels {
            self.init_crawl_offsets_for(wheel)?;
        }
        let poly_length = self.extents.z + 0.3;
        for o in &mut self.crawl_offsets {
            *o = (self.extents.z / 2.0 + 0.15 + *o - self.center_of_mass_offset.z) / poly_length;
        }
        self.crawl_offsets.sort_by(f32::total_cmp);
        let mut i = 0;
        while i < self.crawl_offsets.len() {
            let o1 = self.crawl_offsets[i];
            let mut j = i + 1;
            while j < self.crawl_offsets.len() {
                let o2 = self.crawl_offsets[j];
                if (o2 - o1) * poly_length < 0.15 {
                    self.crawl_offsets.remove(j);
                } else {
                    j += 1;
                }
            }
            i += 1;
        }
        Ok(())
    }

    fn init_crawl_offsets_for(&mut self, wheel: &Wheel) -> ScriptResult {
        let model_offset_z = self.model().map_or(0.0, |m| m.offset.z);
        let front = self.center_of_mass_offset.z + self.extents.z / 2.0;
        let rear = self.center_of_mass_offset.z - self.extents.z / 2.0;
        for i in 0..10 {
            let fi = i as f32;
            let z = model_offset_z + wheel.offset.z + wheel.radius + 0.3 + 0.3 * fi;
            if z + 0.3 <= front && !self.is_overlapping_wheel(z)? {
                self.crawl_offsets.push(z);
            }
            let z = model_offset_z + wheel.offset.z - wheel.radius - 0.3 - 0.3 * fi;
            if z - 0.3 >= rear && !self.is_overlapping_wheel(z)? {
                self.crawl_offsets.push(z);
            }
        }
        Ok(())
    }

    fn is_overlapping_wheel(&self, z_offset: f32) -> ScriptResult<bool> {
        let model_offset_z = self.model().map_or(0.0, |m| m.offset.z);
        for wheel in &self.wheels {
            if wheel.id.as_deref().ok_or_else(|| npe("wheel.id"))?.contains("Left")
                && (model_offset_z + wheel.offset.z - z_offset).abs() < (wheel.radius + 0.3) * 0.99
            {
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// `getName`.
    pub fn name(&self) -> Option<&str> {
        self.name.as_deref()
    }

    /// `getFullName` / `getFullType`: `module + "." + name`.
    pub fn full_name(&self) -> String {
        format!(
            "{}.{}",
            self.base.module_name().as_deref().unwrap_or("null"),
            self.name.as_deref().unwrap_or("null")
        )
    }

    pub fn models(&self) -> &[Model] {
        &self.models
    }

    pub fn models_mut(&mut self) -> &mut [Model] {
        &mut self.models
    }

    pub fn areas(&self) -> &[Area] {
        &self.areas
    }

    pub fn areas_mut(&mut self) -> &mut [Area] {
        &mut self.areas
    }

    pub fn parts(&self) -> &[Part] {
        &self.parts
    }

    pub fn parts_mut(&mut self) -> &mut [Part] {
        &mut self.parts
    }

    pub fn wheels(&self) -> &[Wheel] {
        &self.wheels
    }

    pub fn passengers(&self) -> &[Passenger] {
        &self.passengers
    }

    pub fn physics_shapes(&self) -> &[PhysicsShape] {
        &self.physics_shapes
    }

    pub fn physics_shapes_mut(&mut self) -> &mut [PhysicsShape] {
        &mut self.physics_shapes
    }

    /// `getModel`: the first model.
    pub fn model(&self) -> Option<&Model> {
        self.models.first()
    }

    /// `getModelOffset`.
    pub fn model_offset(&self) -> Option<Vector3> {
        self.model().map(|m| m.offset)
    }

    /// `getModelScale`.
    pub fn model_scale(&self) -> f32 {
        self.model().map_or(1.0, |m| m.scale)
    }

    /// `setModelScale`: undoes the current scale and applies the new one
    /// (clamped to [0.01, 100]) by running `Loaded` twice.
    pub fn set_model_scale(&mut self, sm: &ScriptManager, scale: f32) -> ScriptResult {
        let Some(model) = self.models.first_mut() else { return Ok(()) };
        model.scale = 1.0 / model.scale;
        self.loaded(sm)?;
        self.models[0].scale = clamp(scale, 0.01, 100.0);
        self.loaded(sm)
    }

    /// `getModelCount`.
    pub fn model_count(&self) -> usize {
        self.models.len()
    }

    /// `getModelByIndex`.
    pub fn model_by_index(&self, index: usize) -> &Model {
        &self.models[index]
    }

    /// `getModelById(id)`.
    pub fn model_by_id(&self, id: Option<&str>) -> Option<&Model> {
        model_index(id, &self.models).map(|i| &self.models[i])
    }

    /// `getAttachmentCount`.
    pub fn attachment_count(&self) -> usize {
        self.attachments.len()
    }

    /// `getAttachment`.
    pub fn attachment(&self, index: usize) -> &ModelAttachment {
        &self.attachments[index]
    }

    fn attachment_index(&self, id: Option<&str>) -> Option<usize> {
        self.attachments.iter().position(|a| Some(a.id()) == id)
    }

    /// `getAttachmentById`.
    pub fn attachment_by_id(&self, id: Option<&str>) -> Option<&ModelAttachment> {
        self.attachment_index(id).map(|i| &self.attachments[i])
    }

    /// `addAttachment`.
    pub fn add_attachment(&mut self, attach: ModelAttachment) -> &mut ModelAttachment {
        self.attachments.push(attach);
        self.attachments.last_mut().unwrap()
    }

    /// `addAttachmentAt`.
    pub fn add_attachment_at(&mut self, index: usize, attach: ModelAttachment) -> &mut ModelAttachment {
        self.attachments.insert(index, attach);
        &mut self.attachments[index]
    }

    /// `removeAttachment(int)`.
    pub fn remove_attachment(&mut self, index: usize) -> ModelAttachment {
        self.attachments.remove(index)
    }

    /// `getLightbar`.
    pub fn lightbar(&self) -> &LightBar {
        &self.lightbar
    }

    /// `getSounds`.
    pub fn sounds(&self) -> &Sounds {
        &self.sound
    }

    /// `getHasSiren`.
    pub fn has_siren(&self) -> bool {
        self.has_siren
    }

    /// `getExtents`.
    pub fn extents(&self) -> Vector3 {
        self.extents
    }

    /// `getPhysicsChassisShape`.
    pub fn physics_chassis_shape(&self) -> Vector3 {
        self.physics_chassis_shape
    }

    /// `hasPhysicsChassisShape`.
    pub fn has_physics_chassis_shape(&self) -> bool {
        let v = self.physics_chassis_shape;
        v.x.mul_add(v.x, v.y.mul_add(v.y, v.z * v.z)) > 0.0
    }

    /// `useChassisPhysicsCollision`.
    pub fn use_chassis_physics_collision(&self) -> bool {
        self.use_chassis_physics_collision
    }

    /// `getShadowExtents`.
    pub fn shadow_extents(&self) -> Vector2 {
        self.shadow_extents
    }

    /// `getShadowOffset`.
    pub fn shadow_offset(&self) -> Vector2 {
        self.shadow_offset
    }

    /// `getExtentsOffset`.
    pub fn extents_offset(&self) -> Vector2 {
        self.extents_offset
    }

    /// `getMass`.
    pub fn mass(&self) -> f32 {
        self.mass
    }

    /// `getCenterOfMassOffset`.
    pub fn center_of_mass_offset(&self) -> Vector3 {
        self.center_of_mass_offset
    }

    /// `getEngineForce`.
    pub fn engine_force(&self) -> f32 {
        self.engine_force
    }

    /// `getEngineIdleSpeed`.
    pub fn engine_idle_speed(&self) -> f32 {
        self.engine_idle_speed
    }

    /// `getEngineQuality`.
    pub fn engine_quality(&self) -> i32 {
        self.engine_quality
    }

    /// `getEngineLoudness`.
    pub fn engine_loudness(&self) -> i32 {
        self.engine_loudness
    }

    /// `getRollInfluence`.
    pub fn roll_influence(&self) -> f32 {
        self.roll_influence
    }

    /// `getSteeringIncrement`.
    pub fn steering_increment(&self) -> f32 {
        self.steering_increment
    }

    /// `getSteeringClamp(speed)`: interpolates from `steeringClampMax` at
    /// rest to `steeringClamp` at `maxSpeed`.
    pub fn steering_clamp(&self, speed: f32) -> f32 {
        let mut delta = speed.abs() / self.max_speed;
        if delta > 1.0 {
            delta = 1.0;
        }
        delta = 1.0 - delta;
        (STEERING_CLAMP_MAX - self.steering_clamp) * delta + self.steering_clamp
    }

    /// `getSuspensionStiffness`.
    pub fn suspension_stiffness(&self) -> f32 {
        self.suspension_stiffness
    }

    /// `getSuspensionDamping`.
    pub fn suspension_damping(&self) -> f32 {
        self.suspension_damping
    }

    /// `getSuspensionCompression`.
    pub fn suspension_compression(&self) -> f32 {
        self.suspension_compression
    }

    /// `getSuspensionRestLength`.
    pub fn suspension_rest_length(&self) -> f32 {
        self.suspension_rest_length
    }

    /// `getSuspensionTravel`.
    pub fn suspension_travel(&self) -> f32 {
        self.max_suspension_travel_cm
    }

    /// `getWheelFriction`.
    pub fn wheel_friction(&self) -> f32 {
        self.wheel_friction
    }

    /// `stoppingMovementForce`.
    pub fn stopping_movement_force(&self) -> f32 {
        self.stopping_movement_force
    }

    /// `getWheelCount`.
    pub fn wheel_count(&self) -> usize {
        self.wheels.len()
    }

    /// `getCrawlThroughWheel()`: the number of crawl-through wheels.
    pub fn crawl_through_wheel_count(&self) -> usize {
        self.crawl_through_wheels.len()
    }

    /// `getWheel`.
    pub fn wheel(&self, index: usize) -> &Wheel {
        &self.wheels[index]
    }

    /// `getCrawlThroughWheel(index)`.
    pub fn crawl_through_wheel(&self, index: usize) -> &Wheel {
        &self.crawl_through_wheels[index]
    }

    fn wheel_index(&self, id: Option<&str>) -> Option<usize> {
        self.wheels.iter().position(|w| w.id.is_some() && w.id.as_deref() == id)
    }

    /// `getWheelById`.
    pub fn wheel_by_id(&self, id: Option<&str>) -> Option<&Wheel> {
        self.wheel_index(id).map(|i| &self.wheels[i])
    }

    /// `getIndexOfWheelById`.
    pub fn index_of_wheel_by_id(&self, id: Option<&str>) -> i32 {
        self.wheel_index(id).map_or(-1, |i| i as i32)
    }

    /// `getPassengerCount`.
    pub fn passenger_count(&self) -> usize {
        self.passengers.len()
    }

    /// `getPassenger`.
    pub fn passenger(&self, index: usize) -> &Passenger {
        &self.passengers[index]
    }

    /// `getPassengerById`.
    pub fn passenger_by_id(&self, id: Option<&str>) -> Option<&Passenger> {
        match self.passenger_index(id) {
            -1 => None,
            i => Some(&self.passengers[i as usize]),
        }
    }

    /// `getPassengerIndex`.
    pub fn passenger_index(&self, id: Option<&str>) -> i32 {
        self.passengers.iter().position(|p| p.id.is_some() && p.id.as_deref() == id).map_or(-1, |i| i as i32)
    }

    /// `getPhysicsShapeCount`.
    pub fn physics_shape_count(&self) -> usize {
        self.physics_shapes.len()
    }

    /// `getPhysicsShape`: `None` out of range.
    pub fn physics_shape(&self, index: usize) -> Option<&PhysicsShape> {
        self.physics_shapes.get(index)
    }

    /// `addPhysicsShape(type)`.
    pub fn add_physics_shape(&mut self, type_: &str) -> ScriptResult<&mut PhysicsShape> {
        let mut shape = PhysicsShape::default();
        match type_ {
            "box" => {
                shape.type_ = PHYSICS_SHAPE_BOX;
                shape.extents = Vector3::new(1.0, 1.0, 1.0);
            }
            "sphere" => {
                shape.type_ = PHYSICS_SHAPE_SPHERE;
                shape.radius = 0.5;
            }
            "mesh" => {
                shape.type_ = PHYSICS_SHAPE_MESH;
                shape.physics_shape_script = Some("Base.XXX".to_string());
                shape.extents = Vector3::new(1.0, 1.0, 1.0);
            }
            _ => {
                return Err(ScriptError::new(format!(
                    "java.lang.IllegalArgumentException: invalid vehicle physics shape \"{type_}\""
                )))
            }
        }
        self.physics_shapes.push(shape);
        Ok(self.physics_shapes.last_mut().unwrap())
    }

    /// `removePhysicsShape`.
    pub fn remove_physics_shape(&mut self, index: usize) -> PhysicsShape {
        self.physics_shapes.remove(index)
    }

    /// `getFrontEndHealth`.
    pub fn front_end_health(&self) -> i32 {
        self.front_end_health
    }

    /// `getRearEndHealth`.
    pub fn rear_end_health(&self) -> i32 {
        self.rear_end_health
    }

    /// `getStorageCapacity`.
    pub fn storage_capacity(&self) -> i32 {
        self.storage_capacity
    }

    /// `getTextures`.
    pub fn textures(&self) -> &Skin {
        &self.textures
    }

    /// `getSkinCount`.
    pub fn skin_count(&self) -> usize {
        self.skins.len()
    }

    /// `getSkin`.
    pub fn skin(&self, index: usize) -> &Skin {
        &self.skins[index]
    }

    /// `getAreaCount`.
    pub fn area_count(&self) -> usize {
        self.areas.len()
    }

    /// `getArea`.
    pub fn area(&self, index: usize) -> &Area {
        &self.areas[index]
    }

    /// `getAreaById`.
    pub fn area_by_id(&self, id: Option<&str>) -> Option<&Area> {
        match self.index_of_area_by_id(id) {
            -1 => None,
            i => Some(&self.areas[i as usize]),
        }
    }

    /// `getIndexOfAreaById`.
    pub fn index_of_area_by_id(&self, id: Option<&str>) -> i32 {
        self.areas.iter().position(|a| a.id.is_some() && a.id.as_deref() == id).map_or(-1, |i| i as i32)
    }

    /// `getPartCount`.
    pub fn part_count(&self) -> usize {
        self.parts.len()
    }

    /// `getPart`.
    pub fn part(&self, index: usize) -> &Part {
        &self.parts[index]
    }

    /// `getPartById`.
    pub fn part_by_id(&self, id: Option<&str>) -> Option<&Part> {
        match self.index_of_part_by_id(id) {
            -1 => None,
            i => Some(&self.parts[i as usize]),
        }
    }

    /// `getIndexOfPartById`.
    pub fn index_of_part_by_id(&self, id: Option<&str>) -> i32 {
        self.parts.iter().position(|p| p.id.is_some() && p.id.as_deref() == id).map_or(-1, |i| i as i32)
    }

    /// `getGearRatioCount`.
    pub fn gear_ratio_count(&self) -> i32 {
        self.gear_ratio_count
    }

    /// `getSeats`.
    pub fn seats(&self) -> i32 {
        self.seats
    }

    /// `setSeats`.
    pub fn set_seats(&mut self, seats: i32) {
        self.seats = seats;
    }

    /// `getMechanicType`.
    pub fn mechanic_type(&self) -> i32 {
        self.mechanic_type
    }

    /// `setMechanicType`.
    pub fn set_mechanic_type(&mut self, t: i32) {
        self.mechanic_type = t;
    }

    /// `getEngineRepairLevel`.
    pub fn engine_repair_level(&self) -> i32 {
        self.engine_repair_level
    }

    /// `setEngineRepairLevel`.
    pub fn set_engine_repair_level(&mut self, level: i32) {
        self.engine_repair_level = level;
    }

    /// `getHeadlightConfigLevel`.
    pub fn headlight_config_level(&self) -> i32 {
        2
    }

    /// `getPlayerDamageProtection`.
    pub fn player_damage_protection(&self) -> f32 {
        self.player_damage_protection
    }

    /// `setPlayerDamageProtection`.
    pub fn set_player_damage_protection(&mut self, v: f32) {
        self.player_damage_protection = v;
    }

    /// `getForcedHue`.
    pub fn forced_hue(&self) -> f32 {
        self.forced_hue
    }

    /// `setForcedHue`.
    pub fn set_forced_hue(&mut self, v: f32) {
        self.forced_hue = v;
    }

    /// `getForcedSat`.
    pub fn forced_sat(&self) -> f32 {
        self.forced_sat
    }

    /// `setForcedSat`.
    pub fn set_forced_sat(&mut self, v: f32) {
        self.forced_sat = v;
    }

    /// `getForcedVal`.
    pub fn forced_val(&self) -> f32 {
        self.forced_val
    }

    /// `setForcedVal`.
    pub fn set_forced_val(&mut self, v: f32) {
        self.forced_val = v;
    }

    /// `getEngineRPMType`.
    pub fn engine_rpm_type(&self) -> &str {
        &self.engine_rpm_type
    }

    /// `setEngineRPMType`.
    pub fn set_engine_rpm_type(&mut self, t: &str) {
        self.engine_rpm_type = t.to_string();
    }

    /// `getOffroadEfficiency`.
    pub fn offroad_efficiency(&self) -> f32 {
        self.offroad_efficiency
    }

    /// `setOffroadEfficiency`.
    pub fn set_offroad_efficiency(&mut self, v: f32) {
        self.offroad_efficiency = v;
    }

    /// `getCrawlOffsets`.
    pub fn crawl_offsets(&self) -> &[f32] {
        &self.crawl_offsets
    }

    /// `getAnimalTrailerSize`.
    pub fn animal_trailer_size(&self) -> f32 {
        self.animal_trailer_size
    }

    /// `getZombieType`.
    pub fn zombie_type(&self) -> Option<&[String]> {
        self.zombie_type.as_deref()
    }

    /// `getSpecialKeyRing`.
    pub fn special_key_ring(&self) -> Option<&[String]> {
        self.special_key_ring.as_deref()
    }

    /// `hasSpecialKeyRing`.
    pub fn has_special_key_ring(&self) -> bool {
        self.special_key_ring.as_ref().is_some_and(|r| !r.is_empty())
    }

    /// `getFirstZombieType` (the original throws when there is no list).
    pub fn first_zombie_type(&self) -> Option<&str> {
        self.zombie_type.as_ref()?.first().map(String::as_str)
    }

    /// `hasZombieType`. The original compares references, which for the
    /// interned outfit names it is given means string equality.
    pub fn has_zombie_type(&self, outfit: &str) -> bool {
        self.zombie_type.as_ref().is_some_and(|z| z.iter().any(|t| t == outfit))
    }

    /// `notKillCrops`.
    pub fn not_kill_crops(&self) -> bool {
        self.not_kill_crops
    }

    /// `hasLighter`.
    pub fn has_lighter(&self) -> bool {
        self.has_lighter
    }

    /// `getCarMechanicsOverlay`.
    pub fn car_mechanics_overlay(&self) -> Option<&str> {
        self.car_mechanics_overlay.as_deref()
    }

    /// `setCarMechanicsOverlay`.
    pub fn set_car_mechanics_overlay(&mut self, overlay: Option<String>) {
        self.car_mechanics_overlay = overlay;
    }

    /// `getCarModelName`.
    pub fn car_model_name(&self) -> Option<&str> {
        self.car_model_name.as_deref()
    }

    /// `setCarModelName`.
    pub fn set_car_model_name(&mut self, name: Option<String>) {
        self.car_model_name = name;
    }

    /// `getSpecialLootChance`.
    pub fn special_loot_chance(&self) -> i32 {
        self.special_loot_chance
    }

    /// `getSpecialKeyRingChance`: 0 without a special key ring.
    pub fn special_key_ring_chance(&self) -> i32 {
        if self.has_special_key_ring() {
            self.special_key_ring_chance
        } else {
            0
        }
    }

    /// `neverSpawnKey`.
    pub fn never_spawn_key(&self) -> bool {
        self.never_spawn_key
    }
}

impl ScriptObject for VehicleScript {
    fn base(&self) -> &BaseScriptObject {
        &self.base
    }

    fn base_mut(&mut self) -> &mut BaseScriptObject {
        &mut self.base
    }

    fn class_name(&self) -> &'static str {
        "zombie.scripting.objects.VehicleScript"
    }

    fn init_load_pp(&mut self, sm: &ScriptManager, name: &str) {
        self.base.init_load_pp(name);
        self.file_name = sm.current_file_name();
        self.name = Some(name.to_string());
    }

    fn load(&mut self, sm: &ScriptManager, name: &str, body: &str) -> ScriptResult {
        self.load_body(sm, name, body)
    }
}

/// Borrows a template's script for copying. A template whose body refers
/// back to a template still being loaded finds that script mid-load; the
/// original copies from its partial state, which cannot be borrowed here.
fn borrow_template<'a>(
    sm: &ScriptManager,
    script: &'a std::rc::Rc<std::cell::RefCell<VehicleScript>>,
    spec: &str,
) -> Option<std::cell::Ref<'a, VehicleScript>> {
    let r = script.try_borrow().ok();
    if r.is_none() {
        sm.warn(&format!("template \"{spec}\" refers to a template that is still loading"));
    }
    r
}

/// `globMatch`: `Pattern.compile(pattern.replaceAll("\\*", ".*")).matcher(s).matches()`.
/// Besides `*`, only `.` (any char) is treated as a regex construct.
fn glob_match(pattern: &str, s: Option<&str>) -> ScriptResult<bool> {
    let s = s.ok_or_else(|| npe("input"))?;
    let p: Vec<char> = pattern.chars().collect();
    let t: Vec<char> = s.chars().collect();
    fn m(p: &[char], t: &[char]) -> bool {
        match p.first() {
            None => t.is_empty(),
            Some('*') => (0..=t.len()).any(|k| m(&p[1..], &t[k..])),
            Some('.') => !t.is_empty() && m(&p[1..], &t[1..]),
            Some(c) => t.first() == Some(c) && m(&p[1..], &t[1..]),
        }
    }
    Ok(m(&p, &t))
}

/// `Vector3f.equals`: compares `floatToIntBits`.
fn vec3_equals(a: &Vector3, b: &Vector3) -> bool {
    let bits = |f: f32| if f.is_nan() { f32::NAN.to_bits() } else { f.to_bits() };
    bits(a.x) == bits(b.x) && bits(a.y) == bits(b.y) && bits(a.z) == bits(b.z)
}

/// `LoadVector2f`: `split(" ")`, then `Float.parseFloat` of each part.
fn load_vector2f(s: &str) -> ScriptResult<Vector2> {
    let ss = java::split_char(s, ' ');
    Ok(Vector2::new(parse_float(at(&ss, 0)?)?, parse_float(at(&ss, 1)?)?))
}

/// `LoadVector3f`.
fn load_vector3f(s: &str) -> ScriptResult<Vector3> {
    let ss = java::split_char(s, ' ');
    Ok(Vector3::new(parse_float(at(&ss, 0)?)?, parse_float(at(&ss, 1)?)?, parse_float(at(&ss, 2)?)?))
}

/// `leftCol`/`rightCol`: `r;g;b`.
fn load_color(v: &str) -> ScriptResult<ImmutableColor> {
    let split = java::split_char(v, ';');
    let r = parse_float(at(&split, 0)?)?;
    let g = parse_float(at(&split, 1)?)?;
    let b = parse_float(at(&split, 2)?)?;
    Ok(ImmutableColor::rgb(r, g, b))
}

/// `getModelById(id, models)`: blank ids match each other.
fn model_index(id: Option<&str>, models: &[Model]) -> Option<usize> {
    models.iter().position(|m| {
        (java::is_null_or_whitespace(m.id.as_deref()) && java::is_null_or_whitespace(id))
            || (m.id.is_some() && m.id.as_deref() == id)
    })
}

/// `LoadModel`.
fn load_model(block: &Block, models: &mut Vec<Model>) -> ScriptResult {
    let idx = match model_index(block.id.as_deref(), models) {
        Some(i) => i,
        None => {
            models.push(Model { id: block.id.clone(), ..Model::default() });
            models.len() - 1
        }
    };
    let model = &mut models[idx];
    for value in &block.values {
        let k = java::trim(value.get_key());
        let v = java::trim(value.get_value());
        match k {
            "file" => model.file = Some(v.to_string()),
            "offset" => model.offset = load_vector3f(v)?,
            "rotate" => model.rotate = load_vector3f(v)?,
            "scale" => model.scale = parse_float(v)?,
            "attachmentParent" => model.attachment_name_parent = non_empty(v),
            "attachmentSelf" => model.attachment_name_self = non_empty(v),
            _ if java::eq_ignore_case("ignoreVehicleScale", k) => model.ignore_vehicle_scale = java::parse_bool(v),
            _ => {}
        }
    }
    Ok(())
}

/// `LoadSkin`.
fn load_skin(block: &Block) -> Skin {
    let mut skin = Skin::default();
    for value in &block.values {
        let k = java::trim(value.get_key());
        let v = discard_null_or_whitespace(java::trim(value.get_value()));
        match k {
            "texture" => skin.texture = v,
            "textureRust" => skin.texture_rust = v,
            "textureMask" => skin.texture_mask = v,
            "textureLights" => skin.texture_lights = v,
            "textureDamage1Overlay" => skin.texture_damage1_overlay = v,
            "textureDamage1Shell" => skin.texture_damage1_shell = v,
            "textureDamage2Overlay" => skin.texture_damage2_overlay = v,
            "textureDamage2Shell" => skin.texture_damage2_shell = v,
            "textureShadow" => skin.texture_shadow = v,
            _ => {}
        }
    }
    skin
}

/// `LoadAnim`.
fn load_anim(block: &Block, anims: &mut Vec<Anim>) -> ScriptResult {
    let idx = match anims.iter().position(|a| a.id.is_some() && a.id == block.id) {
        Some(i) => i,
        None => {
            let id = block.id.clone().ok_or_else(|| npe("block.id"))?;
            anims.push(Anim { id: Some(id), ..Anim::default() });
            anims.len() - 1
        }
    };
    let anim = &mut anims[idx];
    for value in &block.values {
        let k = java::trim(value.get_key());
        let v = java::trim(value.get_value());
        match k {
            "angle" => anim.angle = load_vector3f(v)?,
            "anim" => anim.anim = Some(v.to_string()),
            "animate" => anim.animate = java::parse_bool(v),
            "loop" => anim.loop_ = java::parse_bool(v),
            "reverse" => anim.reverse = java::parse_bool(v),
            "rate" => anim.rate = parse_float(v)?,
            "offset" => anim.offset = load_vector3f(v)?,
            "sound" => anim.sound = Some(v.to_string()),
            _ => {}
        }
    }
    Ok(())
}

/// `LoadPassengerSwitchSeat`: an empty block removes the seat.
fn load_switch_seat(block: &Block, passenger: &mut Passenger) -> ScriptResult {
    let found = passenger.switch_seat_index(block.id.as_deref());
    if block.is_empty() {
        if let Some(i) = found {
            passenger.switch_seats.remove(i);
        }
        return Ok(());
    }
    let idx = found.unwrap_or_else(|| {
        passenger.switch_seats.push(SwitchSeat { id: block.id.clone(), ..SwitchSeat::default() });
        passenger.switch_seats.len() - 1
    });
    let seat = &mut passenger.switch_seats[idx];
    for value in &block.values {
        let k = java::trim(value.get_key());
        let v = java::trim(value.get_value());
        match k {
            "anim" => seat.anim = Some(v.to_string()),
            "rate" => seat.rate = parse_float(v)?,
            "sound" => seat.sound = non_empty(v),
            _ => {}
        }
    }
    Ok(())
}

/// `LoadPosition`: an empty block removes the position.
fn load_position(block: &Block, positions: &mut Vec<Position>) -> ScriptResult {
    let found = positions.iter().position(|p| p.id.is_some() && p.id == block.id);
    if block.is_empty() {
        if let Some(i) = found {
            positions.remove(i);
        }
        return Ok(());
    }
    let idx = match found {
        Some(i) => i,
        None => {
            let id = block.id.clone().ok_or_else(|| npe("block.id"))?;
            positions.push(Position { id: Some(id), ..Position::default() });
            positions.len() - 1
        }
    };
    let position = &mut positions[idx];
    for value in &block.values {
        let k = java::trim(value.get_key());
        let v = java::trim(value.get_value());
        match k {
            "rotate" => position.rotate = load_vector3f(v)?,
            "offset" => position.offset = load_vector3f(v)?,
            "area" => position.area = non_empty(v),
            _ => {}
        }
    }
    Ok(())
}

/// `LoadPhysicsShape`: `None` for an unknown type or a degenerate shape.
fn load_physics_shape(block: &Block) -> ScriptResult<Option<PhysicsShape>> {
    let type_ = match block.id.as_deref().ok_or_else(|| npe("block.id"))? {
        "box" => PHYSICS_SHAPE_BOX,
        "sphere" => PHYSICS_SHAPE_SPHERE,
        "mesh" => PHYSICS_SHAPE_MESH,
        _ => return Ok(None),
    };
    let mut shape = PhysicsShape { type_, ..PhysicsShape::default() };
    for value in &block.values {
        let k = java::trim(value.get_key());
        let v = java::trim(value.get_value());
        if java::eq_ignore_case("extents", k) {
            shape.extents = load_vector3f(v)?;
        } else if java::eq_ignore_case("offset", k) {
            shape.offset = load_vector3f(v)?;
        } else if java::eq_ignore_case("radius", k) {
            shape.radius = parse_float(v)?;
        } else if java::eq_ignore_case("rotate", k) {
            shape.rotate = load_vector3f(v)?;
        } else if java::eq_ignore_case("physicsShapeScript", k) {
            shape.physics_shape_script = discard_null_or_whitespace(v);
        } else if java::eq_ignore_case("scale", k) {
            let scale = parse_float(v)?;
            shape.extents = Vector3::new(scale, scale, scale);
        }
    }
    let e = shape.extents;
    match shape.type_ {
        PHYSICS_SHAPE_BOX if e.x <= 0.0 || e.y <= 0.0 || e.z <= 0.0 => return Ok(None),
        PHYSICS_SHAPE_SPHERE if shape.radius <= 0.0 => return Ok(None),
        PHYSICS_SHAPE_MESH => {
            if shape.physics_shape_script.is_none() {
                return Ok(None);
            }
            if e.x <= 0.0 {
                shape.extents = Vector3::new(1.0, 1.0, 1.0);
            }
        }
        _ => {}
    }
    Ok(Some(shape))
}

/// `LoadDoor`: reads no keys.
fn load_door(_block: &Block) -> Door {
    Door
}

/// `LoadWindow`.
fn load_window(block: &Block) -> Window {
    let mut window = Window::default();
    for value in &block.values {
        if java::trim(value.get_key()) == "openable" {
            window.openable = java::parse_bool(java::trim(value.get_value()));
        }
    }
    window
}

/// `LoadContainer`: loads into `existing` when the part already has one.
fn load_container(block: &Block, existing: Option<Container>) -> ScriptResult<Container> {
    let mut container = existing.unwrap_or_default();
    for value in &block.values {
        let k = java::trim(value.get_key());
        let v = java::trim(value.get_value());
        match k {
            "capacity" => container.capacity = parse_int(v)?,
            "conditionAffectsCapacity" => container.condition_affects_capacity = java::parse_bool(v),
            "contentType" => container.content_type = Some(v.to_string()),
            "seat" => container.seat_id = Some(v.to_string()),
            "soundMap" => {
                let ss = java::split_ws(v);
                if ss.len() != 2 || java::trim(ss[0]).is_empty() {
                    continue;
                }
                container.sound_map.insert(java::trim(ss[0]).to_string(), java::trim(ss[1]).to_string());
            }
            "test" => container.lua_test = Some(v.to_string()),
            _ => {}
        }
    }
    Ok(container)
}

/// `LoadLuaFunctions`: every value must be `key = value`.
fn load_lua_functions(block: &Block, full_name: &str) -> ScriptResult<Vec<(String, String)>> {
    let mut map = Vec::new();
    for value in &block.values {
        if !value.string.contains('=') {
            return Err(ScriptError::new(format!(
                "java.lang.RuntimeException: expected \"key = value\", got \"{}\" in {full_name}",
                java::trim(&value.string)
            )));
        }
        map_put(&mut map, java::trim(value.get_key()).to_string(), java::trim(value.get_value()).to_string());
    }
    Ok(map)
}

/// `checkIntegerKey`: all-digit keys become numbers (`Double.valueOf`).
fn check_integer_key(key: &str) -> ScriptResult<LuaKey> {
    if !key.chars().all(|c| c.is_ascii_digit()) {
        return Ok(LuaKey::Str(key.to_string()));
    }
    java::parse_f64(key)
        .map(LuaKey::Num)
        .ok_or_else(|| ScriptError::new("java.lang.NumberFormatException: empty String"))
}

/// `LoadTable`: values become string entries (blank removes the key),
/// child blocks nested tables.
// TODO(lua): built with LuaManager.platform.newTable() in the original.
fn load_table(block: &Block, existing: Option<LuaTable>) -> ScriptResult<LuaTable> {
    let mut table = existing.unwrap_or_default();
    for value in &block.values {
        let k = java::trim(value.get_key());
        let v = java::trim(value.get_value());
        table.rawset(check_integer_key(k)?, non_empty(v).map(LuaValue::Str));
    }
    for child in &block.children {
        let type_ = child.type_str();
        let existing = match table.rawget_str(type_) {
            Some(LuaValue::Table(t)) => Some(t.clone()),
            _ => None,
        };
        let child_table = load_table(child, existing)?;
        table.rawset(check_integer_key(type_)?, Some(LuaValue::Table(child_table)));
    }
    Ok(table)
}
