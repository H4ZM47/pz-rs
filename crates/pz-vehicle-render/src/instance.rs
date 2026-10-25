//! `VehicleModelInstance`: the vehicle body's textures and the per-zone
//! enable matrices its shader reads, as `BaseVehicle.updateLights` and
//! `doDamageOverlay` (doors, windows, other bodywork, blood) set them.

use pz_render::model::VehicleUniforms;
use pz_render::Texture;
use pz_scripts::objects::vehicles::Skin;
use pz_vehicles::part::PartItem;
use pz_vehicles::{BaseVehicle, PartRef};

/// The body's textures (`updateSkin` / `ModelManager.addVehicle`): a
/// skin's textures with the script's own filling the gaps
/// (`Skin.copyMissingFrom`), loaded as `LoadVehicleTexture` does.
#[derive(Clone, Debug, Default)]
pub struct VehicleSkinTextures {
    pub tex: Option<Texture>,
    pub rust: Option<Texture>,
    pub mask: Option<Texture>,
    pub lights: Option<Texture>,
    pub damage1_overlay: Option<Texture>,
    pub damage1_shell: Option<Texture>,
    pub damage2_overlay: Option<Texture>,
    pub damage2_shell: Option<Texture>,
    pub shadow: Option<Texture>,
}

/// `Texture.getSharedTexture` flags `LoadVehicleTexture` passes (compression
/// is on; 0x100 is `TEXTURE_FLAGS` bit 8).
pub const VEHICLE_TEXTURE_FLAGS: i32 = 4 | 0x100;
/// The mask is loaded uncompressed (its zone colours must stay exact).
pub const VEHICLE_MASK_FLAGS: i32 = 0x100;

/// `Skin.copyMissingFrom(script.getTextures())`.
pub fn skin_with_defaults(skin: &Skin, defaults: &Skin) -> Skin {
    let pick = |a: &Option<String>, b: &Option<String>| a.clone().or_else(|| b.clone());
    Skin {
        texture: pick(&skin.texture, &defaults.texture),
        texture_rust: pick(&skin.texture_rust, &defaults.texture_rust),
        texture_mask: pick(&skin.texture_mask, &defaults.texture_mask),
        texture_lights: pick(&skin.texture_lights, &defaults.texture_lights),
        texture_damage1_overlay: pick(
            &skin.texture_damage1_overlay,
            &defaults.texture_damage1_overlay,
        ),
        texture_damage1_shell: pick(&skin.texture_damage1_shell, &defaults.texture_damage1_shell),
        texture_damage2_overlay: pick(
            &skin.texture_damage2_overlay,
            &defaults.texture_damage2_overlay,
        ),
        texture_damage2_shell: pick(&skin.texture_damage2_shell, &defaults.texture_damage2_shell),
        texture_shadow: pick(&skin.texture_shadow, &defaults.texture_shadow),
    }
}

impl VehicleSkinTextures {
    /// `LoadVehicleTextures(skin)`: `load(path, flags)` gets
    /// `media/textures/<name>.png` with `Texture.getSharedTexture` flags.
    pub fn load(skin: &Skin, load: &mut dyn FnMut(&str, i32) -> Option<Texture>) -> Self {
        let mut get = |name: &Option<String>, flags: i32| -> Option<Texture> {
            let n = name.as_deref()?;
            if n.trim().is_empty() {
                return None;
            }
            load(&format!("media/textures/{n}.png"), flags)
        };
        VehicleSkinTextures {
            tex: get(&skin.texture, VEHICLE_TEXTURE_FLAGS),
            mask: get(&skin.texture_mask, VEHICLE_MASK_FLAGS),
            damage1_overlay: get(&skin.texture_damage1_overlay, VEHICLE_TEXTURE_FLAGS),
            damage1_shell: get(&skin.texture_damage1_shell, VEHICLE_TEXTURE_FLAGS),
            damage2_overlay: get(&skin.texture_damage2_overlay, VEHICLE_TEXTURE_FLAGS),
            damage2_shell: get(&skin.texture_damage2_shell, VEHICLE_TEXTURE_FLAGS),
            lights: get(&skin.texture_lights, VEHICLE_TEXTURE_FLAGS),
            rust: get(&skin.texture_rust, VEHICLE_TEXTURE_FLAGS),
            shadow: get(&skin.texture_shadow, VEHICLE_TEXTURE_FLAGS),
        }
    }

    /// The vehicle's current skin (`getSkinIndex()` when in range, else
    /// `script.getTextures()`), defaults filled in.
    pub fn skin_of(v: &BaseVehicle) -> Option<Skin> {
        let script = v.script()?;
        let s = script.borrow();
        let i = v.st().skin_index;
        let skin = if i >= 0 && (i as usize) < s.skin_count() {
            s.skin(i as usize)
        } else {
            s.textures()
        };
        Some(skin_with_defaults(skin, s.textures()))
    }
}

/// `VehicleModelInstance`'s shader state.
#[derive(Clone, Debug, PartialEq)]
pub struct VehicleModelInstance {
    pub texture_uninstall1: [f32; 16],
    pub texture_uninstall2: [f32; 16],
    pub texture_lights_enables1: [f32; 16],
    pub texture_lights_enables2: [f32; 16],
    pub texture_damage1_enables1: [f32; 16],
    pub texture_damage1_enables2: [f32; 16],
    pub texture_damage2_enables1: [f32; 16],
    pub texture_damage2_enables2: [f32; 16],
    pub matrix_blood1_enables1: [f32; 16],
    pub matrix_blood1_enables2: [f32; 16],
    pub matrix_blood2_enables1: [f32; 16],
    pub matrix_blood2_enables2: [f32; 16],
    pub texture_rust_a: f32,
    pub ref_windows: f32,
    pub ref_body: f32,
    pub pain_color: [f32; 3],
    /// `ModelInstance.scale` (`applyModelScriptScale`).
    pub scale: f32,
}

impl Default for VehicleModelInstance {
    fn default() -> Self {
        VehicleModelInstance {
            texture_uninstall1: [0.0; 16],
            texture_uninstall2: [0.0; 16],
            texture_lights_enables1: [0.0; 16],
            texture_lights_enables2: [0.0; 16],
            texture_damage1_enables1: [0.0; 16],
            texture_damage1_enables2: [0.0; 16],
            texture_damage2_enables1: [0.0; 16],
            texture_damage2_enables2: [0.0; 16],
            matrix_blood1_enables1: [0.0; 16],
            matrix_blood1_enables2: [0.0; 16],
            matrix_blood2_enables1: [0.0; 16],
            matrix_blood2_enables2: [0.0; 16],
            texture_rust_a: 0.0,
            ref_windows: 0.5,
            ref_body: 0.4,
            pain_color: [0.0, 0.5, 0.5],
            scale: 1.0,
        }
    }
}

fn part_installed(v: &BaseVehicle, id: &str) -> bool {
    v.part_by_id(id).is_some_and(|p| p.has_item())
}

/// `((InventoryItem)part.getInventoryItem()).getCondition()`; an item kept
/// as raw bytes reads as undamaged.
fn item_condition(p: &PartRef) -> Option<i32> {
    match &p.st().item {
        Some(PartItem::Item(i)) => Some(i.borrow().condition),
        Some(PartItem::Raw(_)) => Some(100),
        None => None,
    }
}

fn window_open(p: &PartRef) -> bool {
    p.st().window.is_some_and(|w| w.open)
}

impl VehicleModelInstance {
    /// `BaseVehicle.updateLights`: rust, paint and the light-zone enables.
    pub fn update_lights(&mut self, v: &BaseVehicle) {
        let script = v.script();
        let st = v.st();
        self.texture_rust_a = st.rust;
        if script
            .as_ref()
            .is_some_and(|s| s.borrow().wheel_count() == 0)
        {
            self.texture_rust_a = 0.0;
        }
        self.pain_color = [st.color_hue, st.color_saturation, st.color_value];
        let window_lights_on = st.window_lights_on;
        let headlights_on = st.headlights_on;
        let stoplights = st.stoplights_on;
        let lightbar_mode = st.lightbar_lights_mode;
        let rust = st.rust;
        let script_name = st.script_name.clone();
        drop(st);
        let w = |id: &str| window_lights_on && part_installed(v, id);
        let (wf, wr) = (w("Windshield"), w("WindshieldRear"));
        let (wfl, wml, wrl) = (
            w("WindowFrontLeft"),
            w("WindowMiddleLeft"),
            w("WindowRearLeft"),
        );
        let (wfr, wmr, wrr) = (
            w("WindowFrontRight"),
            w("WindowMiddleRight"),
            w("WindowRearRight"),
        );
        let b = |on: bool| if on { 1.0 } else { 0.0 };
        self.texture_lights_enables1[10] = b(wf);
        self.texture_lights_enables1[14] = b(wr);
        self.texture_lights_enables1[2] = b(wfl);
        self.texture_lights_enables1[6] = b(wml | wrl);
        self.texture_lights_enables1[9] = b(wfr);
        self.texture_lights_enables1[13] = b(wmr | wrr);
        let battery = v.battery_charge();
        let hl = |id: &str| headlights_on && battery > 0.0 && part_installed(v, id);
        self.texture_lights_enables2[4] = b(hl("HeadlightRight"));
        self.texture_lights_enables2[8] = b(hl("HeadlightLeft"));
        self.texture_lights_enables2[12] = b(hl("HeadlightRearRight"));
        self.texture_lights_enables2[1] = b(hl("HeadlightRearLeft"));
        let mut stop = stoplights && battery > 0.0;
        if script_name.contains("Trailer") {
            if let Some(t) = v.vehicle_towed_by() {
                if t.stoplights_on() && t.battery_charge() > 0.0 {
                    stop = true;
                }
            }
        }
        self.texture_lights_enables2[5] = b(stop);
        self.texture_lights_enables2[9] = b(stop);
        if script
            .as_ref()
            .is_some_and(|s| s.borrow().lightbar().enable)
        {
            let (e13, e2) = if lightbar_mode.is_enable() && battery > 0.0 {
                match lightbar_mode.light {
                    1 => (0.0, 1.0),
                    2 => (1.0, 0.0),
                    _ => (0.0, 0.0),
                }
            } else {
                (0.0, 0.0)
            };
            self.texture_lights_enables2[13] = e13;
            self.texture_lights_enables2[2] = e2;
        }
        self.ref_body = 0.3;
        self.ref_windows = 0.4;
        if rust > 0.8 {
            self.ref_body = 0.1;
            self.ref_windows = 0.2;
        }
    }

    /// `checkDamage(part, matrixName, doBlack)`.
    fn check_damage(&mut self, part: Option<PartRef>, i: usize, mut black: bool) {
        if black {
            if let Some(p) = &part {
                let is_window = p.id().is_some_and(|id| id.starts_with("Window"));
                let has_default = p.script_part().is_some_and(|sp| {
                    sp.models
                        .as_ref()
                        .is_some_and(|m| m.iter().any(|m| m.id.as_deref() == Some("Default")))
                });
                if is_window && has_default {
                    black = false;
                }
            }
        }
        self.texture_damage1_enables1[i] = 0.0;
        self.texture_damage2_enables1[i] = 0.0;
        self.texture_uninstall1[i] = 0.0;
        let Some(p) = part else { return };
        match item_condition(&p) {
            Some(c) => {
                if (40..60).contains(&c) {
                    self.texture_damage1_enables1[i] = 1.0;
                }
                if c < 40 {
                    self.texture_damage2_enables1[i] = 1.0;
                }
                if window_open(&p) && black {
                    self.texture_uninstall1[i] = 1.0;
                }
            }
            None => {
                if black {
                    self.texture_uninstall1[i] = 1.0;
                }
            }
        }
    }

    /// `checkDamage2(part, matrixName, doBlack)`.
    fn check_damage2(&mut self, part: Option<PartRef>, i: usize, black: bool) {
        self.texture_damage1_enables2[i] = 0.0;
        self.texture_damage2_enables2[i] = 0.0;
        self.texture_uninstall2[i] = 0.0;
        let Some(p) = part else { return };
        match item_condition(&p) {
            Some(c) => {
                if (40..60).contains(&c) {
                    self.texture_damage1_enables2[i] = 1.0;
                }
                if c < 40 {
                    self.texture_damage2_enables2[i] = 1.0;
                }
                if window_open(&p) && black {
                    self.texture_uninstall2[i] = 1.0;
                }
            }
            None => {
                if black {
                    self.texture_uninstall2[i] = 1.0;
                }
            }
        }
    }

    /// `checkUninstall2(part, matrixName)`.
    fn check_uninstall2(&mut self, part: &PartRef, i: usize) {
        self.texture_uninstall2[i] = 0.0;
        if !part.has_item() {
            self.texture_uninstall2[i] = 1.0;
        }
    }

    /// `doDamageOverlay`: doors, windows, other bodywork and blood.
    /// `blood_decals` is `Core.getOptionBloodDecals() != 0`.
    pub fn do_damage_overlay(&mut self, v: &BaseVehicle, blood_decals: bool) {
        let p = |id: &str| v.part_by_id(id);
        let either = |a: &str, b: &str| p(a).or_else(|| p(b));
        // doDoorDamage.
        self.check_damage(p("DoorFrontLeft"), 1, true);
        self.check_damage(p("DoorFrontRight"), 8, true);
        if let Some(d) = either("DoorRearLeft", "DoorMiddleLeft") {
            self.check_damage(Some(d), 5, true);
        }
        if let Some(d) = either("DoorRearRight", "DoorMiddleRight") {
            self.check_damage(Some(d), 12, true);
        }
        // doWindowDamage.
        self.check_damage(p("WindowFrontLeft"), 2, true);
        self.check_damage(p("WindowFrontRight"), 9, true);
        if let Some(w) = either("WindowRearLeft", "WindowMiddleLeft") {
            self.check_damage(Some(w), 6, true);
        }
        if let Some(w) = either("WindowRearRight", "WindowMiddleRight") {
            self.check_damage(Some(w), 13, true);
        }
        self.check_damage(p("Windshield"), 10, true);
        self.check_damage(p("WindshieldRear"), 14, true);
        // doOtherBodyWorkDamage.
        self.check_damage(p("EngineDoor"), 0, false);
        self.check_damage(p("EngineDoor"), 3, false);
        self.check_damage(p("EngineDoor"), 11, false);
        self.check_damage2(p("EngineDoor"), 6, true);
        self.check_damage(p("TruckBed"), 4, false);
        self.check_damage(p("TruckBed"), 7, false);
        self.check_damage(p("TruckBed"), 15, false);
        if let Some(back) = either("TrunkDoor", "DoorRear") {
            self.check_damage2(Some(back.clone()), 10, true);
            if back.script_part().is_some_and(|sp| sp.has_lights_rear) {
                for i in [12, 1, 5, 9] {
                    self.check_uninstall2(&back, i);
                }
            }
        }
        self.do_blood_overlay(v, blood_decals);
    }

    /// `doBloodOverlay` (`s_PartToMaskMap` is empty in the original, so
    /// per-part intensities add nothing).
    pub fn do_blood_overlay(&mut self, v: &BaseVehicle, blood_decals: bool) {
        self.matrix_blood1_enables1 = [0.0; 16];
        self.matrix_blood1_enables2 = [0.0; 16];
        self.matrix_blood2_enables1 = [0.0; 16];
        self.matrix_blood2_enables2 = [0.0; 16];
        if !blood_decals {
            return;
        }
        let (m1, m2) = (
            &mut self.matrix_blood1_enables1,
            &mut self.matrix_blood1_enables2,
        );
        let front = v.blood_intensity("Front");
        for i in [0, 10] {
            m1[i] = front;
        }
        for i in [6, 4, 8] {
            m2[i] = front;
        }
        let rear = v.blood_intensity("Rear");
        for i in [4, 14] {
            m1[i] = rear;
        }
        for i in [10, 12, 1, 5, 9] {
            m2[i] = rear;
        }
        let left = v.blood_intensity("Left");
        for i in [11, 1, 5, 15, 2, 6] {
            m1[i] = left;
        }
        let right = v.blood_intensity("Right");
        for i in [3, 8, 12, 7, 9, 13] {
            m1[i] = right;
        }
        // doBloodOverlayAux(matrixBlood2, 1).
        for i in [0, 4, 7, 15, 3, 8, 12, 11, 1, 5, 10, 14, 9, 13, 2, 6] {
            self.matrix_blood2_enables1[i] = 1.0;
        }
        for i in [6, 4, 8, 10, 12, 1, 5, 9, 0] {
            self.matrix_blood2_enables2[i] = 1.0;
        }
    }

    /// The shader uniforms (`Model.DrawVehicle`): `TexturePainColor` takes
    /// the slot's alpha; `ReflectionParam` the sky box's texture shift.
    pub fn uniforms(&self, alpha: f32, sky_texture_shift: f32) -> VehicleUniforms {
        VehicleUniforms {
            texture_uninstall: [self.texture_uninstall1, self.texture_uninstall2],
            texture_lights_enables: [self.texture_lights_enables1, self.texture_lights_enables2],
            texture_damage1_enables: [self.texture_damage1_enables1, self.texture_damage1_enables2],
            texture_damage2_enables: [self.texture_damage2_enables1, self.texture_damage2_enables2],
            matrix_blood1_enables: [self.matrix_blood1_enables1, self.matrix_blood1_enables2],
            matrix_blood2_enables: [self.matrix_blood2_enables1, self.matrix_blood2_enables2],
            texture_rust_a: self.texture_rust_a,
            paint_color: [
                self.pain_color[0],
                self.pain_color[1],
                self.pain_color[2],
                alpha,
            ],
            reflection: [sky_texture_shift, self.ref_windows, self.ref_body],
        }
    }
}
