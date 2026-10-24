//! `zombie.characters.BodyDamage.Thermoregulator` and its
//! `ThermalNode`s (players only, owned by
//! [`super::body_damage::BodyDamage::thermoregulator`]).
//!
//! The original holds its `BodyDamage`, character and `ClimateManager`;
//! here [`Thermoregulator::update`] takes the owning character, the climate
//! readings come from [`super::context::CharacterEnvironment`]
//! (`air_temperature`, `air_and_wind_temperature`, `worn_clothing`) and
//! [`SimContext::climate_humidity`], and the `Metabolics` values from the
//! jar table.
//!
//! The nodes are made for the body parts `BodyDamage`'s constructor
//! creates, in its order; their `distToCore`/`skinSurface` are the
//! `BodyPartType` constructor arguments read from the jar.
//!
//! Clothing: the original rebuilds the per-node clothing lists only when
//! the worn item visuals change (checked against a cache shared by all
//! characters) and reads each item's live insulation, wetness, condition
//! and holes every update; this rebuilds the lists from
//! [`super::context::CharacterEnvironment::worn_clothing`] every update,
//! which gives the same lists. TODO(saves): `save`/`load`.

use std::cell::Cell;

use pz_scripts::{ScriptError, ScriptManager, ScriptResult};

use super::body_part::CONTACT_CHILDREN;
use super::context::SimContext;
use super::metabolics::Metabolics;
use super::stats::{clamp_f32, CharacterStats, Stats};
use super::GameCharacter;
use crate::Characters;

/// The body parts `BodyDamage`'s constructor creates, in its order, with
/// the `ThermalNode` insulation multiplier `initNodes` gives each.
const BODY_PARTS: [(&str, f32); 17] = [
    ("Hand_L", 1.0),
    ("Hand_R", 1.0),
    ("ForeArm_L", 0.25),
    ("ForeArm_R", 0.25),
    ("UpperArm_L", 0.25),
    ("UpperArm_R", 0.25),
    ("Torso_Upper", 0.25),
    ("Torso_Lower", 0.25),
    ("Head", 1.0),
    ("Neck", 0.5),
    ("Groin", 0.5),
    ("UpperLeg_L", 0.5),
    ("UpperLeg_R", 0.5),
    ("LowerLeg_L", 0.5),
    ("LowerLeg_R", 0.5),
    ("Foot_L", 0.5),
    ("Foot_R", 0.5),
];

thread_local! {
    /// `Thermoregulator.simulationMultiplier`.
    static SIMULATION_MULTIPLIER: Cell<f32> = const { Cell::new(1.0) };
}

/// `Thermoregulator.Multiplier` (the ordinal picks the factor).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Multiplier {
    Default,
    MetabolicRateInc,
    MetabolicRateDec,
    BodyHeat,
    CoreHeatExpand,
    CoreHeatContract,
    SkinCelcius,
    SkinCelciusContract,
    SkinCelciusExpand,
    PrimaryDelta,
    SecondaryDelta,
}

/// `PZMath.abs(float)`: `val * sign(val)`.
fn pz_abs(v: f32) -> f32 {
    let sign = if v > 0.0 {
        1
    } else if v < 0.0 {
        -1
    } else {
        0
    };
    v * sign as f32
}

fn clamp_01(v: f32) -> f32 {
    clamp_f32(v, 0.0, 1.0)
}

/// `PZMath.lerp`.
fn lerp(src: f32, dest: f32, alpha: f32) -> f32 {
    src + (dest - src) * alpha
}

/// `PZMath.c_lerp`.
fn c_lerp(src: f32, dest: f32, alpha: f32) -> f32 {
    let t2 = (1.0 - f64::from(alpha * std::f64::consts::PI as f32).cos()) as f32 / 2.0;
    src * (1.0 - t2) + dest * t2
}

/// `PZMath.equal(a, b)`.
fn pz_equal(a: f32, b: f32) -> bool {
    pz_abs(b - a) < 1.0e-7
}

/// `Temperature.getTrueInsulationValue`.
pub fn true_insulation_value(insulation: f32) -> f32 {
    insulation * 2.0 + 0.5 * insulation * insulation * insulation
}

/// `Temperature.getTrueWindresistanceValue`.
pub fn true_windresistance_value(windresist: f32) -> f32 {
    windresist * 1.0 + 0.5 * windresist * windresist
}

fn npe(what: &str) -> ScriptError {
    ScriptError::new(format!("java.lang.NullPointerException: {what}"))
}

/// `Thermoregulator.ThermalNode`.
#[derive(Clone, Debug)]
pub struct ThermalNode {
    dist_to_core: f32,
    skin_surface: f32,
    /// `bodyPartType` (its `BodyPartType` field) and its index, which is
    /// also the `BloodBodyPartType` index (`bloodBpt`).
    body_part_type: String,
    body_part_index: usize,
    is_core: bool,
    insulation_layer_multiplier_ui: f32,
    upstream: Option<usize>,
    downstream: Option<Vec<usize>>,
    insulation: f32,
    windresist: f32,
    celcius: f32,
    skin_celcius: f32,
    heat_delta: f32,
    primary_delta: f32,
    secondary_delta: f32,
    clothing_wetness: f32,
    body_wetness: f32,
    /// `clothing`: indices into the environment's worn clothing.
    clothing: Vec<usize>,
}

impl ThermalNode {
    /// `bodyPartType` (the `BodyPartType` field name).
    pub fn body_part_type(&self) -> &str {
        &self.body_part_type
    }
    /// `BodyPartType.ToIndex(bodyPartType)`.
    pub fn body_part_index(&self) -> usize {
        self.body_part_index
    }
    /// `hasUpstream`.
    pub fn has_upstream(&self) -> bool {
        self.upstream.is_some()
    }
    /// `hasDownstream`.
    pub fn has_downstream(&self) -> bool {
        self.downstream.as_ref().is_some_and(|d| !d.is_empty())
    }
    /// `getDistToCore`.
    pub fn dist_to_core(&self) -> f32 {
        self.dist_to_core
    }
    /// `getSkinSurface`.
    pub fn skin_surface(&self) -> f32 {
        self.skin_surface
    }
    /// `isCore`.
    pub fn is_core(&self) -> bool {
        self.is_core
    }
    /// `getInsulation`.
    pub fn insulation(&self) -> f32 {
        self.insulation
    }
    /// `getWindresist`.
    pub fn windresist(&self) -> f32 {
        self.windresist
    }
    /// `getCelcius`.
    pub fn celcius(&self) -> f32 {
        self.celcius
    }
    /// `getSkinCelcius`.
    pub fn skin_celcius(&self) -> f32 {
        self.skin_celcius
    }
    /// `getHeatDelta`.
    pub fn heat_delta(&self) -> f32 {
        self.heat_delta
    }
    /// `getPrimaryDelta`.
    pub fn primary_delta(&self) -> f32 {
        self.primary_delta
    }
    /// `getSecondaryDelta`.
    pub fn secondary_delta(&self) -> f32 {
        self.secondary_delta
    }
    /// `getClothingWetness`.
    pub fn clothing_wetness(&self) -> f32 {
        self.clothing_wetness
    }
    /// `getBodyWetness`.
    pub fn body_wetness(&self) -> f32 {
        self.body_wetness
    }
    /// `getBodyResponse`.
    pub fn body_response(&self) -> f32 {
        lerp(self.primary_delta, self.secondary_delta, 0.5)
    }
    /// `getSkinCelciusUI`.
    pub fn skin_celcius_ui(&self) -> f32 {
        let v = clamp_f32(self.skin_celcius, 20.0, 42.0);
        if v < 33.0 {
            (v - 20.0) / 13.0 * 0.5
        } else {
            0.5 + (v - 33.0) / 9.0
        }
    }
    /// `getHeatDeltaUI`.
    pub fn heat_delta_ui(&self) -> f32 {
        clamp_f32((self.heat_delta * 0.2 + 1.0) / 2.0, 0.0, 1.0)
    }
    /// `getPrimaryDeltaUI`.
    pub fn primary_delta_ui(&self) -> f32 {
        clamp_f32((self.primary_delta + 1.0) / 2.0, 0.0, 1.0)
    }
    /// `getSecondaryDeltaUI`.
    pub fn secondary_delta_ui(&self) -> f32 {
        clamp_f32((self.secondary_delta + 1.0) / 2.0, 0.0, 1.0)
    }
    /// `getInsulationUI`.
    pub fn insulation_ui(&self) -> f32 {
        clamp_f32(self.insulation * self.insulation_layer_multiplier_ui, 0.0, 1.0)
    }
    /// `getWindresistUI`.
    pub fn windresist_ui(&self) -> f32 {
        clamp_f32(self.windresist * self.insulation_layer_multiplier_ui, 0.0, 1.0)
    }
    /// `getClothingWetnessUI`.
    pub fn clothing_wetness_ui(&self) -> f32 {
        clamp_f32(self.clothing_wetness, 0.0, 1.0)
    }
    /// `getBodyWetnessUI`.
    pub fn body_wetness_ui(&self) -> f32 {
        clamp_f32(self.body_wetness, 0.0, 1.0)
    }
    /// `getBodyResponseUI`.
    pub fn body_response_ui(&self) -> f32 {
        clamp_f32((self.body_response() + 1.0) / 2.0, 0.0, 1.0)
    }

    /// `calculateInsulation`, with the body part's wetness and the worn
    /// clothing.
    fn calculate_insulation(&mut self, part_wetness: f32, worn: &[super::context::WornClothing]) {
        let layers = self.clothing.len();
        self.insulation = 0.0;
        self.windresist = 0.0;
        self.clothing_wetness = 0.0;
        self.body_wetness = part_wetness * 0.01;
        self.body_wetness = clamp_01(self.body_wetness);
        if layers > 0 {
            for &c in &self.clothing {
                let item = &worn[c];
                let item_wetness = clamp_f32(item.wetness * 0.01, 0.0, 1.0);
                self.clothing_wetness += item_wetness;
                let hole = item.holes.get(self.body_part_index).copied().unwrap_or(0.0);
                if hole > 0.0 {
                    continue;
                }
                let mut ins = true_insulation_value(item.insulation);
                let mut wind = true_windresistance_value(item.windresistance);
                let mut condition = clamp_f32(item.current_condition * 0.01, 0.0, 1.0);
                condition = 0.5 + 0.5 * condition;
                ins *= (1.0 - item_wetness * 0.75) * condition;
                self.insulation += ins;
                wind *= (1.0 - item_wetness * 0.45) * condition;
                self.windresist += wind;
            }
            self.clothing_wetness /= layers as f32;
            self.insulation += layers as f32 * 0.05;
            self.windresist += layers as f32 * 0.05;
        }
    }
}

/// `Thermoregulator`.
#[derive(Clone, Debug)]
pub struct Thermoregulator {
    metabolics: Metabolics,
    set_point: f32,
    metabolic_rate: f32,
    metabolic_rate_real: f32,
    metabolic_target: f32,
    fluids_multiplier: f64,
    energy_multiplier: f64,
    fatigue_multiplier: f64,
    body_heat_delta: f32,
    core_heat_delta: f32,
    thermal_chevron_up: bool,
    core: Option<usize>,
    nodes: Vec<ThermalNode>,
    total_heat_raw: f32,
    total_heat: f32,
    prim_total: f32,
    sec_total: f32,
    external_air_temperature: f32,
    air_temperature: f32,
    air_and_wind_temp: f32,
    rate_of_change_counter: f32,
    core_celcius_cache: f32,
    core_rate_of_change: f32,
    thermal_damage: f32,
    damage_counter: f32,
    thermal_duration: f32,
}

/// What the update reads from the owner's moodles.
struct MoodleLevels {
    hypothermia: i32,
    hyperthermia: i32,
}

impl Thermoregulator {
    /// The constructor (`initNodes` over `BodyDamage`'s body parts).
    pub fn new(sm: &ScriptManager) -> Self {
        let tables = sm.builtin_tables.borrow();
        let metabolics = Metabolics::from_tables(&tables);
        let default_met = metabolics.met_or_zero("Default");
        let mut nodes: Vec<ThermalNode> = Vec::new();
        let mut core = None;
        for (field, insulation_multiplier) in BODY_PARTS {
            let Some(index) = tables.body_parts.iter().position(|p| p.field == field) else {
                continue;
            };
            let decl = &tables.body_parts[index];
            let is_core = field == "Torso_Upper";
            if is_core {
                core = Some(nodes.len());
            }
            nodes.push(ThermalNode {
                dist_to_core: decl.dist_to_core,
                skin_surface: decl.skin_surface,
                body_part_type: field.to_string(),
                body_part_index: index,
                is_core,
                insulation_layer_multiplier_ui: insulation_multiplier,
                upstream: None,
                downstream: None,
                insulation: 0.0,
                windresist: 0.0,
                celcius: 37.0,
                skin_celcius: 33.0,
                heat_delta: 0.0,
                primary_delta: 0.0,
                secondary_delta: 0.0,
                clothing_wetness: 0.0,
                body_wetness: 0.0,
                clothing: Vec::new(),
            });
        }
        let find = |nodes: &[ThermalNode], f: &str| nodes.iter().position(|n| n.body_part_type == f);
        for i in 0..nodes.len() {
            let ty = nodes[i].body_part_type.clone();
            let parent = CONTACT_CHILDREN.iter().find(|(_, c)| c.contains(&ty.as_str())).map(|(p, _)| *p);
            if let Some(p) = parent {
                nodes[i].upstream = find(&nodes, p);
            }
            if let Some((_, children)) = CONTACT_CHILDREN.iter().find(|(p, _)| *p == ty) {
                let ds: Vec<usize> = children.iter().filter_map(|c| find(&nodes, c)).collect();
                nodes[i].downstream = Some(ds);
            }
        }
        let mut t = Thermoregulator {
            metabolics,
            set_point: 37.0,
            metabolic_rate: default_met,
            metabolic_rate_real: default_met,
            metabolic_target: default_met,
            fluids_multiplier: 1.0,
            energy_multiplier: 1.0,
            fatigue_multiplier: 1.0,
            body_heat_delta: 0.0,
            core_heat_delta: 0.0,
            thermal_chevron_up: true,
            core,
            nodes,
            total_heat_raw: 0.0,
            total_heat: 0.0,
            prim_total: 0.0,
            sec_total: 0.0,
            external_air_temperature: 27.0,
            air_temperature: 0.0,
            air_and_wind_temp: 0.0,
            rate_of_change_counter: 0.0,
            core_celcius_cache: 37.0,
            core_rate_of_change: 0.0,
            thermal_damage: 0.0,
            damage_counter: 0.0,
            thermal_duration: 0.0,
        };
        if let Some(c) = t.core {
            t.nodes[c].celcius = t.set_point;
        }
        t
    }

    fn met(&self, field: &str) -> ScriptResult<f32> {
        self.metabolics.met(field)
    }

    fn core_index(&self) -> ScriptResult<usize> {
        self.core.ok_or_else(|| npe("Thermoregulator.core"))
    }

    /// `setSimulationMultiplier` (static).
    pub fn set_simulation_multiplier(multiplier: f32) {
        SIMULATION_MULTIPLIER.with(|m| m.set(multiplier));
    }

    /// `getSimulationMultiplier()`: the static multiplier.
    pub fn simulation_multiplier() -> f32 {
        SIMULATION_MULTIPLIER.with(Cell::get)
    }

    /// `reset`.
    pub fn reset(&mut self) -> ScriptResult {
        let core = self.core_index()?;
        self.set_point = 37.0;
        self.metabolic_rate = self.met("Default")?;
        self.metabolic_target = self.metabolic_rate;
        self.nodes[core].celcius = 37.0;
        self.body_heat_delta = 0.0;
        self.core_heat_delta = 0.0;
        self.thermal_damage = 0.0;
        for (i, node) in self.nodes.iter_mut().enumerate() {
            if i != core {
                node.celcius = 35.0;
            }
            node.primary_delta = 0.0;
            node.secondary_delta = 0.0;
            node.skin_celcius = 33.0;
            node.heat_delta = 0.0;
        }
        Ok(())
    }

    /// `getNodeSize`.
    pub fn node_size(&self) -> usize {
        self.nodes.len()
    }
    /// `getNode` (`None` out of range).
    pub fn node(&self, index: usize) -> Option<&ThermalNode> {
        self.nodes.get(index)
    }
    /// The nodes.
    pub fn nodes(&self) -> &[ThermalNode] {
        &self.nodes
    }
    /// `getNodeForType` by `BodyPartType` index: the node's position.
    pub fn node_index_for_type(&self, body_part_index: usize) -> Option<usize> {
        self.nodes.iter().position(|n| n.body_part_index == body_part_index)
    }
    /// `getNodeForBloodType` by `BloodBodyPartType` index (the same index as
    /// the node's body part).
    pub fn node_index_for_blood_type(&self, blood_index: usize) -> Option<usize> {
        self.node_index_for_type(blood_index)
    }
    /// The core node's position (`Torso_Upper`).
    pub fn core_node(&self) -> Option<usize> {
        self.core
    }
    /// `getBodyHeatDelta`.
    pub fn body_heat_delta(&self) -> f32 {
        self.body_heat_delta
    }
    /// `getFluidsMultiplier`.
    pub fn fluids_multiplier(&self) -> f64 {
        self.fluids_multiplier
    }
    /// `getEnergyMultiplier`.
    pub fn energy_multiplier(&self) -> f64 {
        self.energy_multiplier
    }
    /// `getFatigueMultiplier`.
    pub fn fatigue_multiplier(&self) -> f64 {
        self.fatigue_multiplier
    }

    /// `getMovementModifier`, given the owner's hypo- and hyperthermia
    /// moodle levels.
    pub fn movement_modifier(hypothermia: i32, hyperthermia: i32) -> f32 {
        let mut delta = 1.0f32;
        for lvl in [hypothermia, hyperthermia] {
            match lvl {
                2 => delta = 0.66,
                3 => delta = 0.33,
                4 => delta = 0.0,
                _ => {}
            }
        }
        delta
    }

    /// `getCombatModifier`, given the owner's hypo- and hyperthermia moodle
    /// levels.
    pub fn combat_modifier(hypothermia: i32, hyperthermia: i32) -> f32 {
        let mut delta = 1.0f32;
        for lvl in [hypothermia, hyperthermia] {
            match lvl {
                2 => delta = 0.66,
                3 => delta = 0.33,
                4 => delta = 0.1,
                _ => {}
            }
        }
        delta
    }

    /// `getCoreTemperature`.
    pub fn core_temperature(&self) -> ScriptResult<f32> {
        Ok(self.nodes[self.core_index()?].celcius)
    }
    /// `getHeatGeneration`.
    pub fn heat_generation(&self) -> f32 {
        self.metabolic_rate_real
    }
    /// `getMetabolicRate`.
    pub fn metabolic_rate(&self) -> f32 {
        self.metabolic_rate
    }
    /// `getMetabolicTarget`.
    pub fn metabolic_target(&self) -> f32 {
        self.metabolic_target
    }
    /// `getMetabolicRateReal`.
    pub fn metabolic_rate_real(&self) -> f32 {
        self.metabolic_rate_real
    }
    /// `getSetPoint`.
    pub fn set_point(&self) -> f32 {
        self.set_point
    }
    /// `getCoreHeatDelta`.
    pub fn core_heat_delta(&self) -> f32 {
        self.core_heat_delta
    }
    /// `getCoreRateOfChange`.
    pub fn core_rate_of_change(&self) -> f32 {
        self.core_rate_of_change
    }
    /// `getExternalAirTemperature`.
    pub fn external_air_temperature(&self) -> f32 {
        self.external_air_temperature
    }
    /// `getCoreTemperatureUI`.
    pub fn core_temperature_ui(&self) -> ScriptResult<f32> {
        let v = clamp_f32(self.core_temperature()?, 20.0, 42.0);
        Ok(if v < 37.0 { (v - 20.0) / 17.0 * 0.5 } else { 0.5 + (v - 37.0) / 5.0 * 0.5 })
    }
    /// `getHeatGenerationUI`.
    pub fn heat_generation_ui(&self) -> ScriptResult<f32> {
        let max = self.met("MAX")?;
        let def = self.met("Default")?;
        let v = clamp_f32(self.metabolic_rate_real, 0.0, max);
        Ok(if v < def { v / def * 0.5 } else { 0.5 + (v - def) / (max - def) * 0.5 })
    }
    /// `thermalChevronUp`.
    pub fn thermal_chevron_up(&self) -> bool {
        self.thermal_chevron_up
    }
    /// `thermalChevronCount`.
    pub fn thermal_chevron_count(&self) -> i32 {
        if self.core_rate_of_change > 0.01 {
            3
        } else if self.core_rate_of_change > 0.001 {
            2
        } else if self.core_rate_of_change > 1.0e-4 {
            1
        } else {
            0
        }
    }

    /// The skin penalty both `getCatchAColdDelta` and
    /// `getTimedActionTimeModifier` start from.
    fn skin_penalty(node: &ThermalNode) -> f32 {
        let mut skin = 0.0f32;
        if node.skin_celcius < 33.0 {
            skin = (node.skin_celcius - 20.0) / 13.0;
            skin = 1.0 - skin;
            skin *= skin;
        }
        0.25 * skin * node.skin_surface
    }

    /// `getCatchAColdDelta`, given the owner's hypothermia moodle level.
    pub fn catch_a_cold_delta(&self, hypothermia: i32) -> f32 {
        let mut delta = 0.0f32;
        for node in &self.nodes {
            let mut add = Self::skin_penalty(node);
            if node.body_wetness > 0.0 {
                add *= 1.0 + node.body_wetness * 1.0;
            }
            if node.clothing_wetness > 0.5 {
                add *= 1.0 + (node.clothing_wetness - 0.5) * 2.0;
            }
            match node.body_part_type.as_str() {
                "Neck" => add *= 8.0,
                "Torso_Upper" => add *= 16.0,
                "Head" => add *= 4.0,
                _ => {}
            }
            delta += add;
        }
        if hypothermia > 1 {
            delta *= hypothermia as f32;
        }
        delta
    }

    /// `getTimedActionTimeModifier`.
    pub fn timed_action_time_modifier(&self) -> f32 {
        let mut m = 1.0f32;
        for node in &self.nodes {
            let penalty = Self::skin_penalty(node);
            match node.body_part_type.as_str() {
                "Hand_R" | "Hand_L" => m += 0.3 * penalty,
                "ForeArm_R" | "ForeArm_L" => m += 0.15 * penalty,
                "UpperArm_R" | "UpperArm_L" => m += 0.1 * penalty,
                _ => {}
            }
        }
        m
    }

    /// `getSkinCelciusMin`.
    pub const SKIN_CELCIUS_MIN: f32 = 20.0;
    /// `getSkinCelciusFavorable`.
    pub const SKIN_CELCIUS_FAVORABLE: f32 = 33.0;
    /// `getSkinCelciusMax`.
    pub const SKIN_CELCIUS_MAX: f32 = 42.0;
    /// `THERMAL_COLD_DAMAGE_MOD`.
    pub const THERMAL_COLD_DAMAGE_MOD: f32 = 1.2;

    /// `setMetabolicTarget(Metabolics)`.
    pub fn set_metabolic_target_of(&mut self, field: &str) -> ScriptResult {
        let met = self.met(field)?;
        self.set_metabolic_target(met)
    }

    /// `setMetabolicTarget(float)`: only raises the target, up to
    /// `Metabolics.MAX`.
    pub fn set_metabolic_target(&mut self, target: f32) -> ScriptResult {
        if target < 0.0 || target < self.metabolic_target {
            return Ok(());
        }
        self.metabolic_target = target;
        let max = self.met("MAX")?;
        if self.metabolic_target > max {
            self.metabolic_target = max;
        }
        Ok(())
    }

    fn update_core_rate_of_change(&mut self, ctx: &dyn SimContext) -> ScriptResult {
        let core = self.core_index()?;
        self.rate_of_change_counter += ctx.multiplier();
        if self.rate_of_change_counter > 100.0 {
            self.rate_of_change_counter = 0.0;
            self.core_rate_of_change = self.nodes[core].celcius - self.core_celcius_cache;
            self.thermal_chevron_up = self.core_rate_of_change >= 0.0;
            self.core_rate_of_change = pz_abs(self.core_rate_of_change);
            self.core_celcius_cache = self.nodes[core].celcius;
        }
        Ok(())
    }

    /// `getSimulationMultiplier(Multiplier)`.
    pub fn simulation_multiplier_for(&self, ty: Multiplier, ctx: &dyn SimContext) -> f32 {
        let mut multiplier = ctx.multiplier();
        match ty as i32 {
            1 => multiplier *= 0.001,
            2 => multiplier *= 4.0e-4,
            3 => multiplier *= 2.5e-4,
            4 => multiplier *= 5.0e-5,
            5 => multiplier *= 5.0e-4,
            6 | 8 => multiplier *= 0.0025,
            7 => multiplier *= 0.005,
            9 => multiplier *= 5.0e-4,
            10 => multiplier *= 2.5e-4,
            _ => {}
        }
        multiplier * Self::simulation_multiplier()
    }

    /// `getThermalDamage`.
    pub fn thermal_damage(&self) -> f32 {
        self.thermal_damage
    }
    /// `getDbg_totalHeatRaw`.
    pub fn dbg_total_heat_raw(&self) -> f32 {
        self.total_heat_raw
    }
    /// `getDbg_totalHeat`.
    pub fn dbg_total_heat(&self) -> f32 {
        self.total_heat
    }
    /// `getCoreCelcius`: 0 without a core node.
    pub fn core_celcius(&self) -> f32 {
        self.core.map_or(0.0, |c| self.nodes[c].celcius)
    }
    /// `getDbg_primTotal`.
    pub fn dbg_prim_total(&self) -> f32 {
        self.prim_total
    }
    /// `getDbg_secTotal`.
    pub fn dbg_sec_total(&self) -> f32 {
        self.sec_total
    }

    /// `updateThermalDamage`; returns the cold damage stage to set on the
    /// owner's `BodyDamage`.
    fn update_thermal_damage(
        &mut self,
        air_temperature: f32,
        m: &MoodleLevels,
        ctx: &dyn SimContext,
    ) -> ScriptResult<f32> {
        let core = self.nodes[self.core_index()?].celcius;
        self.damage_counter += ctx.game_world_seconds_since_last_update();
        if self.damage_counter > 1.0 {
            if m.hypothermia == 4 && air_temperature < 0.0 && core - self.core_celcius_cache <= 0.0 {
                let mut frost_speed = (core - 20.0) / 5.0;
                frost_speed = 1.0 - frost_speed;
                let mut seconds = 120.0f32;
                seconds += 480.0 * frost_speed;
                self.thermal_damage += 1.0 / seconds * clamp_01(pz_abs(air_temperature) / 10.0);
                self.thermal_damage *= 1.2;
                self.thermal_duration += self.damage_counter;
                self.thermal_damage = self.thermal_damage.min(self.thermal_duration / 7200.0);
            } else if m.hyperthermia == 4 && air_temperature > 37.0 && core - self.core_celcius_cache >= 0.0 {
                let burnspeed = (core - 41.0) / 1.0;
                let mut seconds = 120.0f32;
                seconds += 480.0 * burnspeed;
                self.thermal_damage += 1.0 / seconds * clamp_01((air_temperature - 37.0) / 8.0);
                self.thermal_damage = self.thermal_damage.min(0.3);
                self.thermal_duration = 0.0;
            } else {
                self.thermal_damage -= 0.011_111_111;
                self.thermal_duration = 0.0;
            }
            self.damage_counter = 0.0;
            self.thermal_damage = clamp_01(self.thermal_damage);
        }
        Ok(self.thermal_damage)
    }

    /// `getEnergy`, from the owner's stats.
    pub fn energy(stats: &Stats, defs: &CharacterStats) -> ScriptResult<f32> {
        let hunger = stats.get(&defs.field("HUNGER")?);
        let fatigue = stats.get(&defs.field("FATIGUE")?);
        let h = 1.0 - (0.4 * hunger + 0.6 * hunger * hunger);
        let f = 1.0 - (0.4 * fatigue + 0.6 * fatigue * fatigue);
        Ok(0.6 * h + 0.4 * f)
    }

    /// `getBodyFluids`, from the owner's stats.
    pub fn body_fluids(stats: &Stats, defs: &CharacterStats) -> ScriptResult<f32> {
        Ok(1.0 - stats.get(&defs.field("THIRST")?))
    }

    /// `update`, for the owner `chr`; sets the owner's cold damage stage.
    pub fn update<I>(
        &mut self,
        chr: &mut GameCharacter<I>,
        sm: &ScriptManager,
        chars: &Characters,
        ctx: &dyn SimContext,
    ) -> ScriptResult {
        let defs = &chars.sim.character_stats;
        let moodles = MoodleLevels {
            hypothermia: chr.moodles.moodle_level_of(sm, "HYPOTHERMIA")?,
            hyperthermia: chr.moodles.moodle_level_of(sm, "HYPERTHERMIA")?,
        };
        self.air_temperature = chr.environment.air_temperature;
        self.air_and_wind_temp = chr.environment.air_and_wind_temperature;
        self.external_air_temperature = self.air_temperature;
        // updateSetPoint
        self.set_point = 37.0;
        let sickness = defs.field("SICKNESS")?;
        if chr.stats.is_above_minimum(&sickness) {
            self.set_point += chr.stats.get(&sickness) * 2.0;
        }
        self.update_core_rate_of_change(ctx)?;
        self.update_metabolic_rate(chr, sm, defs, &moodles, ctx)?;
        self.update_clothing(chr);
        self.update_nodes_heat_delta(chr, defs, ctx)?;
        self.update_heat_deltas(&mut chr.stats, defs, ctx)?;
        self.update_nodes(ctx)?;
        self.update_body_multipliers();
        let stage = self.update_thermal_damage(self.air_and_wind_temp, &moodles, ctx)?;
        chr.body_damage.set_cold_damage_stage(stage);
        Ok(())
    }

    fn update_metabolic_rate<I>(
        &mut self,
        chr: &GameCharacter<I>,
        sm: &ScriptManager,
        defs: &CharacterStats,
        m: &MoodleLevels,
        ctx: &dyn SimContext,
    ) -> ScriptResult {
        self.set_metabolic_target_of("Default")?;
        if chr.is_player() {
            if let Some(weapon) = &chr.environment.attacking_weapon_type {
                let target = match weapon.as_str() {
                    "UNARMED" | "ONE_HANDED" | "SPEAR" | "THROWING" => Some("MediumWork"),
                    "TWO_HANDED" => Some("HeavyWork"),
                    "HEAVY" | "CHAINSAW" => Some("Running15kmh"),
                    "KNIFE" | "FIREARM" => Some("LightWork"),
                    "HANDGUN" => Some("UsingTools"),
                    _ => None,
                };
                if let Some(t) = target {
                    self.set_metabolic_target_of(t)?;
                }
            }
            if chr.player_moving {
                if chr.is_sprinting(sm, ctx) {
                    self.set_metabolic_target_of("Running15kmh")?;
                } else if chr.is_running(sm) {
                    self.set_metabolic_target_of("Running10kmh")?;
                } else if chr.is_sneaking() {
                    self.set_metabolic_target_of("Walking2kmh")?;
                } else if chr.environment.current_speed > 0.0 {
                    self.set_metabolic_target_of("Walking5kmh")?;
                }
            }
            let fitness = chr.fitness.as_ref().ok_or_else(|| npe("getFitness()"))?;
            if fitness.current_exe().is_some() {
                self.set_metabolic_target_of("Fitness")?;
            }
        }
        let energy = Self::energy(&chr.stats, defs)?;
        let excercise = clamp_01(1.0 - chr.stats.get(&defs.field("ENDURANCE")?)) * self.met("DefaultExercise")?;
        self.set_metabolic_target(excercise * energy)?;
        let weight = clamp_01(chr.environment.inventory_capacity_weight / chr.max_weight() as f32);
        let ratio = 1.0 + weight * weight * 0.35;
        self.set_metabolic_target(self.metabolic_target * ratio)?;
        if !pz_equal(self.metabolic_rate, self.metabolic_target) {
            let rate = self.metabolic_target - self.metabolic_rate;
            if self.metabolic_target > self.metabolic_rate {
                self.metabolic_rate += rate * self.simulation_multiplier_for(Multiplier::MetabolicRateInc, ctx);
            } else {
                self.metabolic_rate += rate * self.simulation_multiplier_for(Multiplier::MetabolicRateDec, ctx);
            }
        }
        let mut meta_mod = 1.0f32;
        if m.hypothermia >= 1 {
            meta_mod = Self::movement_modifier(m.hypothermia, m.hyperthermia);
        }
        self.metabolic_rate_real = self.metabolic_rate * (0.2 + 0.8 * energy * meta_mod);
        self.metabolic_target = -1.0;
        Ok(())
    }

    /// `updateClothing`: each node's clothing from the worn items.
    // `!(x > 0)` keeps the original's handling of NaN.
    #[allow(clippy::neg_cmp_op_on_partial_ord)]
    fn update_clothing<I>(&mut self, chr: &GameCharacter<I>) {
        for node in &mut self.nodes {
            node.clothing.clear();
        }
        // `nodes[BodyPartType.ToIndex(BodyPartType.Head)]`.
        let head = self.nodes.iter().find(|n| n.body_part_type == "Head").map(|n| n.body_part_index);
        let n = self.nodes.len();
        for (i, item) in chr.environment.worn_clothing.iter().enumerate() {
            if !(item.insulation > 0.0) && !(item.windresistance > 0.0) {
                continue;
            }
            let mut added = false;
            if let Some(parts) = &item.covered_parts {
                for &p in parts {
                    if p < 0 || p as usize >= n {
                        continue;
                    }
                    added = true;
                    self.nodes[p as usize].clothing.push(i);
                }
            }
            if added || !item.hat_or_mask {
                continue;
            }
            if let Some(node) = head.and_then(|h| self.nodes.get_mut(h)) {
                node.clothing.push(i);
            }
        }
    }

    fn update_nodes_heat_delta<I>(
        &mut self,
        chr: &GameCharacter<I>,
        defs: &CharacterStats,
        ctx: &dyn SimContext,
    ) -> ScriptResult {
        let nutrition = chr.nutrition.as_ref().ok_or_else(|| npe("getNutrition()"))?;
        let mut weight_delta = clamp_01(((nutrition.weight() / 75.0 - 0.5) * f64::from(0.666f32)) as f32);
        weight_delta = (weight_delta - 0.5) * 2.0;
        let fitness_delta = chr.stats.get(&defs.field("FITNESS")?);
        let mut prim_heat_mod = 1.0f32;
        if self.air_and_wind_temp > self.set_point - 2.0 {
            if self.air_temperature < self.set_point + 2.0 {
                prim_heat_mod = (self.air_temperature - (self.set_point - 2.0)) / 4.0;
                prim_heat_mod = 1.0 - prim_heat_mod;
            } else {
                prim_heat_mod = 0.0;
            }
        }
        let mut humidity_mod = 1.0f32;
        let humidity = ctx.climate_humidity();
        if humidity > 0.5 {
            let hum = (humidity - 0.5) * 2.0;
            humidity_mod -= hum;
        }
        let energy = Self::energy(&chr.stats, defs)?;
        let fluids_level = Self::body_fluids(&chr.stats, defs)?;
        let default_met = self.met("Default")?;
        let max_met = self.met("MAX")?;
        let mut nodes_total_heat = 0.0f32;
        for node in &mut self.nodes {
            node.calculate_insulation(
                chr.body_damage.body_part_wetness(node.body_part_index),
                &chr.environment.worn_clothing,
            );
            let mut external_temp = self.air_temperature;
            if self.air_and_wind_temp < self.air_temperature {
                external_temp -= (self.air_temperature - self.air_and_wind_temp) / (1.0 + node.windresist);
            }
            let mut node_heat_delta = external_temp - node.skin_celcius;
            if node_heat_delta <= 0.0 {
                node_heat_delta *= 1.0 + 0.75 * node.body_wetness;
            } else {
                node_heat_delta /= 1.0 + 3.0 * node.body_wetness;
            }
            node_heat_delta *= 0.3;
            node_heat_delta /= 1.0 + node.insulation;
            node.heat_delta = node_heat_delta * node.skin_surface;
            if node.primary_delta > 0.0 {
                let fluids = 0.2 + 0.8 * fluids_level;
                let mut contribution = default_met * node.primary_delta * node.skin_surface / (1.0 + node.insulation);
                contribution *= fluids * (0.1 + 0.9 * prim_heat_mod);
                contribution *= humidity_mod;
                contribution *= 1.0 - 0.2 * weight_delta;
                contribution *= 1.0 + 0.2 * fitness_delta;
                node.heat_delta -= contribution;
            } else {
                let e = 0.2 + 0.8 * energy;
                let mut contribution = default_met * pz_abs(node.primary_delta) * node.skin_surface;
                contribution *= e;
                contribution *= 1.0 + 0.2 * weight_delta;
                contribution *= 1.0 + 0.2 * fitness_delta;
                node.heat_delta += contribution;
            }
            if node.secondary_delta > 0.0 {
                let fluids = 0.1 + 0.9 * fluids_level;
                let mut contribution =
                    max_met * 0.75 * node.secondary_delta * node.skin_surface / (1.0 + node.insulation);
                contribution *= fluids;
                contribution *= 0.85 + 0.15 * humidity_mod;
                contribution *= 1.0 - 0.2 * weight_delta;
                contribution *= 1.0 + 0.2 * fitness_delta;
                node.heat_delta -= contribution;
            } else {
                let e = 0.1 + 0.9 * energy;
                let mut contribution = default_met * pz_abs(node.secondary_delta) * node.skin_surface;
                contribution *= e;
                contribution *= 1.0 + 0.2 * weight_delta;
                contribution *= 1.0 + 0.2 * fitness_delta;
                node.heat_delta += contribution;
            }
            nodes_total_heat += node.heat_delta;
        }
        self.total_heat_raw = nodes_total_heat;
        nodes_total_heat += self.metabolic_rate_real;
        self.total_heat = nodes_total_heat;
        Ok(())
    }

    fn update_heat_deltas(&mut self, stats: &mut Stats, defs: &CharacterStats, ctx: &dyn SimContext) -> ScriptResult {
        let core = self.core_index()?;
        self.core_heat_delta = self.total_heat * self.simulation_multiplier_for(Multiplier::BodyHeat, ctx);
        let celcius = self.nodes[core].celcius;
        if self.core_heat_delta < 0.0 {
            if celcius > self.set_point {
                self.core_heat_delta *= 1.0 + (celcius - self.set_point) / 2.0;
            }
        } else if celcius < self.set_point {
            self.core_heat_delta *= 1.0 + (self.set_point - celcius) / 4.0;
        }
        let mut c = celcius + self.core_heat_delta;
        c = clamp_f32(c, 20.0, 42.0);
        let temperature = defs.field("TEMPERATURE")?;
        let current = stats.get(&temperature);
        if pz_abs(current - c) > 0.001 {
            c = lerp(c, current, 0.5);
        }
        self.nodes[core].celcius = c;
        stats.set(&temperature, c);
        self.body_heat_delta = 0.0;
        // Not `!=`: a NaN temperature leaves the delta at 0.
        #[allow(clippy::double_comparisons)]
        if c > self.set_point || c < self.set_point {
            self.body_heat_delta = c - self.set_point;
        }
        if self.body_heat_delta < 0.0 {
            let mut bhd = pz_abs(self.body_heat_delta);
            if bhd <= 1.0 {
                self.body_heat_delta *= 0.8;
            } else {
                bhd = clamp_f32(bhd, 1.0, 11.0) - 1.0;
                bhd /= 10.0;
                self.body_heat_delta = -0.8 + -0.2 * bhd;
            }
        }
        self.body_heat_delta = clamp_f32(self.body_heat_delta, -1.0, 1.0);
        Ok(())
    }

    fn update_nodes(&mut self, ctx: &dyn SimContext) -> ScriptResult {
        let core = self.core_index()?;
        let mut prim = 0.0f32;
        let mut sec = 0.0f32;
        let skin_mult = self.simulation_multiplier_for(Multiplier::SkinCelcius, ctx);
        for i in 0..self.nodes.len() {
            let core_celcius = self.nodes[core].celcius;
            let body_heat_delta = self.body_heat_delta;
            let air = self.air_temperature;
            let node = &mut self.nodes[i];
            let insulation = 1.0 + node.insulation;
            if body_heat_delta < 0.0 {
                let dist = node.dist_to_core;
                node.primary_delta = body_heat_delta * (1.0 + dist);
            } else {
                node.primary_delta = body_heat_delta * (1.0 + (1.0 - node.dist_to_core));
            }
            node.primary_delta = clamp_f32(node.primary_delta, -1.0, 1.0);
            node.secondary_delta = node.primary_delta * pz_abs(node.primary_delta) * pz_abs(node.primary_delta);
            prim += node.primary_delta * node.skin_surface;
            sec += node.secondary_delta * node.skin_surface;
            node.primary_delta = clamp_f32(node.primary_delta, -1.0, 1.0);
            let mut skin_min = core_celcius - 20.0;
            let skin_max = core_celcius;
            if skin_min < air {
                if air < 33.0 {
                    skin_min = air;
                } else {
                    let dist_core = 0.4 + 0.6 * (1.0 - node.dist_to_core);
                    let skinmod = (air - 33.0) / 6.0;
                    skin_min = 33.0;
                    skin_min += 4.0 * skinmod * dist_core;
                    skin_min = clamp_f32(skin_min, 33.0, air);
                    if skin_min > skin_max {
                        skin_min = skin_max - 0.25;
                    }
                }
            }
            let mut skin_target = core_celcius - 4.0;
            if node.primary_delta < 0.0 {
                let dist_core = 0.4 + 0.6 * node.dist_to_core;
                let target = skin_target - 12.0 * dist_core / insulation;
                skin_target = c_lerp(skin_target, target, pz_abs(node.primary_delta));
            } else {
                let dist_core = 0.4 + 0.6 * (1.0 - node.dist_to_core);
                let mut target = skin_target;
                let mut target_add = 4.0 * dist_core;
                target_add *= (insulation * 0.5 * dist_core).max(1.0);
                target = (target + target_add).min(skin_max);
                skin_target = c_lerp(skin_target, target, node.primary_delta);
            }
            skin_target = clamp_f32(skin_target, skin_min, skin_max);
            let skin_delta = skin_target - node.skin_celcius;
            let mut multipl = skin_mult;
            if skin_delta < 0.0 && node.skin_celcius > 33.0 || skin_delta > 0.0 && node.skin_celcius < 33.0 {
                multipl *= 3.0;
            }
            if multipl > 1.0 {
                multipl = 1.0;
            }
            node.skin_celcius += skin_delta * multipl;
            if i == core {
                continue;
            }
            node.celcius = if node.skin_celcius >= core_celcius {
                core_celcius
            } else {
                lerp(node.skin_celcius, core_celcius, 0.5)
            };
        }
        self.prim_total = prim;
        self.sec_total = sec;
        Ok(())
    }

    fn update_body_multipliers(&mut self) {
        self.energy_multiplier = 1.0;
        self.fluids_multiplier = 1.0;
        self.fatigue_multiplier = 1.0;
        let mut m = pz_abs(self.prim_total);
        m *= m;
        if self.prim_total < 0.0 {
            self.energy_multiplier += f64::from(0.05 * m);
            self.fatigue_multiplier += f64::from(0.25 * m);
        } else if self.prim_total > 0.0 {
            self.fluids_multiplier += f64::from(0.25 * m);
            self.fatigue_multiplier += f64::from(0.25 * m);
        }
        m = pz_abs(self.sec_total);
        m *= m;
        if self.sec_total < 0.0 {
            self.energy_multiplier += f64::from(0.1 * m);
            self.fatigue_multiplier += f64::from(0.75 * m);
        } else if self.sec_total > 0.0 {
            self.fluids_multiplier += f64::from(3.75 * m);
            self.fatigue_multiplier += f64::from(1.75 * m);
        }
    }
}

impl<I> GameCharacter<I> {
    /// `BodyDamage.Update`'s `if (thermoregulator != null)
    /// thermoregulator.update()`.
    pub fn thermoregulator_update(
        &mut self,
        sm: &ScriptManager,
        chars: &Characters,
        ctx: &dyn SimContext,
    ) -> ScriptResult {
        let Some(mut t) = self.body_damage.thermoregulator.take() else {
            return Ok(());
        };
        let r = t.update(self, sm, chars, ctx);
        self.body_damage.thermoregulator = Some(t);
        r
    }

    /// `IsoGameCharacter.setMetabolicTarget(float)`.
    pub fn set_metabolic_target(&mut self, target: f32) -> ScriptResult {
        match self.body_damage.thermoregulator.as_mut() {
            Some(t) => t.set_metabolic_target(target),
            None => Ok(()),
        }
    }

    /// `IsoPlayer.getTimedActionTimeModifier` (1 for other characters).
    pub fn timed_action_time_modifier(&self) -> f32 {
        match (self.is_player(), self.body_damage.thermoregulator()) {
            (true, Some(t)) => t.timed_action_time_modifier(),
            _ => 1.0,
        }
    }
}

#[cfg(test)]
#[path = "body_tests.rs"]
mod body_tests;
