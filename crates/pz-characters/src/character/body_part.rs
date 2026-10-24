//! `zombie.characters.BodyDamage.BodyPart`, `BodyPartLast`,
//! `BodyPartContacts`, and `BodyPartType`'s static behaviour over the
//! constants read from the jar ([`BodyPartTypes`]).
//!
//! A `BodyPart` holds its owner in the original; here the methods that read
//! the owner (healer traits) or engine singletons (`Rand`, sandbox options)
//! take an [`InjuryEnv`]. `DamageUpdate`, which also changes the owner and
//! every other part, is [`super::GameCharacter::body_part_damage_update`].
//!
//! TODO(Thermoregulator): `thermalNode` (`getInnerTemperature`,
//! `getSkinTemperature` read 0 and `getDistToCore`/`getSkinSurface` the
//! type's values, as the original does without a node).

use pz_scripts::registry::builtin::{BodyPartDecl, BuiltinTables};
use pz_scripts::{ScriptError, ScriptResult};

use super::context::{SimContext, SimEvent};
use super::stats::clamp_f32;

/// Java's `Math.min(float, float)` (NaN wins).
pub(crate) fn java_min(a: f32, b: f32) -> f32 {
    if a.is_nan() || b.is_nan() {
        f32::NAN
    } else if a <= b {
        a
    } else {
        b
    }
}

/// Java's `Math.max(float, float)` (NaN wins).
pub(crate) fn java_max(a: f32, b: f32) -> f32 {
    if a.is_nan() || b.is_nan() {
        f32::NAN
    } else if a >= b {
        a
    } else {
        b
    }
}

/// `(float)(k * (double)GameTime.getMultiplier())`.
fn dmul(k: f64, mult: f32) -> f32 {
    (k * mult as f64) as f32
}

// ---- BodyPartType ---------------------------------------------------------

/// `BodyPartType`'s constants (declaration order, `MAX` last) and its
/// static methods.
#[derive(Clone, Debug, Default)]
pub struct BodyPartTypes {
    pub decls: Vec<BodyPartDecl>,
}

impl BodyPartTypes {
    pub fn from_tables(tables: &BuiltinTables) -> Self {
        BodyPartTypes { decls: tables.body_parts.clone() }
    }

    /// `BodyPartType.ToIndex(MAX)`: the number of real parts.
    pub fn max_index(&self) -> usize {
        self.decls.iter().position(|d| d.field == "MAX").unwrap_or(self.decls.len())
    }

    /// `BodyPartType.ToIndex(BodyPartType.<field>)`.
    pub fn index(&self, field: &str) -> ScriptResult<usize> {
        self.decls
            .iter()
            .position(|d| d.field == field)
            .ok_or_else(|| ScriptError::new(format!("BodyPartType.{field} is not a constant")))
    }

    /// `BodyPartType.FromIndex`: out of range is `MAX`.
    pub fn from_index(&self, index: i32) -> Option<&BodyPartDecl> {
        usize::try_from(index).ok().and_then(|i| self.decls.get(i)).or_else(|| self.decls.get(self.max_index()))
    }

    /// The constant name of `FromIndex(index)`.
    pub fn field_of(&self, index: i32) -> &str {
        self.from_index(index).map_or("MAX", |d| d.field.as_str())
    }

    /// `BodyPartType.getPainModifyer`.
    pub fn pain_modifier(&self, index: i32) -> f32 {
        self.from_index(index).map_or(0.0, |d| d.pain_modifier)
    }

    /// `BodyPartType.getDamageModifyer`.
    pub fn damage_modifier(&self, index: i32) -> f32 {
        self.from_index(index).map_or(0.0, |d| d.damage_modifier)
    }

    /// `BodyPartType.getBleedingTimeModifyer`.
    pub fn bleeding_time_modifier(&self, index: i32) -> f32 {
        self.from_index(index).map_or(0.0, |d| d.bleeding_time_modifier)
    }

    /// `BodyPartType.ToString(FromIndex(index))`.
    pub fn name_of(&self, index: i32) -> &str {
        self.from_index(index).map_or("", |d| d.name.as_str())
    }
}

// ---- InjuryEnv ------------------------------------------------------------

/// What a `BodyPart` reads from its owner and the engine when it injures or
/// heals: `Rand`, the sandbox options (through `ctx`) and the owner's healer
/// traits.
pub struct InjuryEnv<'a> {
    pub ctx: &'a dyn SimContext,
    /// `parentChar.hasTrait(CharacterTrait.FAST_HEALER)`.
    pub fast_healer: bool,
    /// `parentChar.hasTrait(CharacterTrait.SLOW_HEALER)`.
    pub slow_healer: bool,
}

impl InjuryEnv<'_> {
    fn next_float(&self, min: f32, max: f32) -> f32 {
        self.ctx.rand_next_float(min, max)
    }

    /// `switch (injurySeverity) { case 1: *= 0.5; case 3: *= 1.5 }`.
    fn severity(&self, v: f32) -> f32 {
        match self.ctx.injury_severity() {
            1 => v * 0.5,
            3 => v * 1.5,
            _ => v,
        }
    }

    /// The healer-dependent random duration: `normal`, or `fast`/`slow`
    /// redrawn for the traits.
    fn heal_time(&self, normal: (f32, f32), fast: (f32, f32), slow: (f32, f32)) -> f32 {
        let mut t = self.next_float(normal.0, normal.1);
        if self.fast_healer {
            t = self.next_float(fast.0, fast.1);
        } else if self.slow_healer {
            t = self.next_float(slow.0, slow.1);
        }
        t
    }
}

// ---- BodyPart -------------------------------------------------------------

/// `BodyPart`.
#[derive(Clone, Debug, PartialEq)]
pub struct BodyPart {
    /// `type` (the `BodyPartType` constant name).
    pub type_field: String,
    /// `BodyPartType.ToIndex(type)`.
    pub index: usize,
    bleeding_time_modifier: f32,
    skin_surface: f32,
    dist_to_core: f32,
    pub damage_scaler: f32,
    pub health: f32,
    pub bandaged: bool,
    pub manipulating_username: Option<String>,
    pub bitten: bool,
    pub bleeding: bool,
    pub is_bleeding_stemmed: bool,
    pub is_cauterized: bool,
    pub scratched: bool,
    pub stitched: bool,
    pub deep_wounded: bool,
    pub is_infected: bool,
    pub is_fake_infected: bool,
    pub bandage_life: f32,
    pub scratch_time: f32,
    pub bite_time: f32,
    pub alcoholic_bandage: bool,
    pub stiffness: f32,
    pub wound_infection_level: f32,
    pub infected_wound: bool,
    pub bleeding_time: f32,
    pub deep_wound_time: f32,
    pub have_glass: bool,
    pub stitch_time: f32,
    pub alcohol_level: f32,
    pub additional_pain: f32,
    pub bandage_type: Option<String>,
    pub get_bandage_xp: bool,
    pub get_stitch_xp: bool,
    pub get_splint_xp: bool,
    pub fracture_time: f32,
    pub splint: bool,
    pub splint_factor: f32,
    pub have_bullet: bool,
    pub burn_time: f32,
    pub need_burn_wash: bool,
    pub last_time_burn_wash: f32,
    pub splint_item: Option<String>,
    pub plantain_factor: f32,
    pub comfrey_factor: f32,
    pub garlic_factor: f32,
    pub cut_time: f32,
    pub cut: bool,
    pub scratch_speed_modifier: f32,
    pub cut_speed_modifier: f32,
    pub burn_speed_modifier: f32,
    pub deep_wound_speed_modifier: f32,
    pub wetness: f32,
}

const BITE_DAMAGE: f32 = 2.1875;
const BLEED_DAMAGE: f32 = 0.285_714_3;
const SCRATCH_DAMAGE: f32 = 0.9375;
const CUT_DAMAGE: f32 = 1.875;
const BURN_DAMAGE: f32 = 3.75;

impl BodyPart {
    /// `new BodyPart(type, parent)`.
    pub fn new(decl: &BodyPartDecl, index: usize) -> Self {
        let t = decl.field.as_str();
        let mut damage_scaler = 0.005_714_285_7f32;
        if t == "Neck" {
            damage_scaler *= 5.0;
        }
        let (mut s, mut c, mut b, mut d) = (0.0f32, 0.0f32, 0.0f32, 0.0f32);
        if matches!(t, "Hand_L" | "Hand_R" | "ForeArm_L" | "ForeArm_R") {
            (s, c, b, d) = (85.0, 95.0, 45.0, 60.0);
        }
        if matches!(t, "UpperArm_L" | "UpperArm_R") {
            (s, c, b, d) = (65.0, 75.0, 35.0, 40.0);
        }
        if matches!(t, "UpperLeg_L" | "UpperLeg_R" | "LowerLeg_L" | "LowerLeg_R") {
            (s, c, b, d) = (45.0, 55.0, 15.0, 20.0);
        }
        if matches!(t, "Foot_L" | "Foot_R") {
            (s, c, b, d) = (35.0, 45.0, 10.0, 15.0);
        }
        if t == "Groin" {
            (s, c, b, d) = (45.0, 55.0, 15.0, 20.0);
        }
        let mut p = BodyPart {
            type_field: decl.field.clone(),
            index,
            bleeding_time_modifier: decl.bleeding_time_modifier,
            skin_surface: decl.skin_surface,
            dist_to_core: decl.dist_to_core,
            damage_scaler,
            health: 0.0,
            bandaged: false,
            manipulating_username: None,
            bitten: false,
            bleeding: false,
            is_bleeding_stemmed: false,
            is_cauterized: false,
            scratched: false,
            stitched: false,
            deep_wounded: false,
            is_infected: false,
            is_fake_infected: false,
            bandage_life: 0.0,
            scratch_time: 0.0,
            bite_time: 0.0,
            alcoholic_bandage: false,
            stiffness: 0.0,
            wound_infection_level: 0.0,
            infected_wound: false,
            bleeding_time: 0.0,
            deep_wound_time: 0.0,
            have_glass: false,
            stitch_time: 0.0,
            alcohol_level: 0.0,
            additional_pain: 0.0,
            bandage_type: None,
            get_bandage_xp: true,
            get_stitch_xp: true,
            get_splint_xp: true,
            fracture_time: 0.0,
            splint: false,
            splint_factor: 0.0,
            have_bullet: false,
            burn_time: 0.0,
            need_burn_wash: false,
            last_time_burn_wash: 0.0,
            splint_item: None,
            plantain_factor: 0.0,
            comfrey_factor: 0.0,
            garlic_factor: 0.0,
            cut_time: 0.0,
            cut: false,
            scratch_speed_modifier: s,
            cut_speed_modifier: c,
            burn_speed_modifier: b,
            deep_wound_speed_modifier: d,
            wetness: 0.0,
        };
        p.restore_to_full_health();
        p
    }

    /// `getIndex`.
    pub fn get_index(&self) -> usize {
        self.index
    }
    /// `AddDamage`.
    pub fn add_damage(&mut self, val: f32) {
        self.reduce_health(val);
    }
    /// `isBandageDirty`.
    pub fn is_bandage_dirty(&self) -> bool {
        self.bandage_life <= 0.0
    }
    /// `getHealth`.
    pub fn health(&self) -> f32 {
        self.health
    }
    /// `SetHealth`.
    pub fn set_health(&mut self, v: f32) {
        self.health = v;
    }
    /// `AddHealth`.
    pub fn add_health(&mut self, val: f32) {
        self.health = clamp_f32(self.health + val, 0.0, 100.0);
    }
    /// `ReduceHealth`.
    pub fn reduce_health(&mut self, val: f32) {
        self.health = clamp_f32(self.health - val, 0.0, 100.0);
    }
    /// `HasInjury`.
    pub fn has_injury(&self) -> bool {
        self.bitten
            | self.scratched
            | self.deep_wounded
            | self.bleeding
            | (self.bite_time > 0.0)
            | (self.scratch_time > 0.0)
            | (self.cut_time > 0.0)
            | (self.fracture_time > 0.0)
            | self.have_bullet
            | (self.burn_time > 0.0)
    }
    /// `DisableFakeInfection`.
    pub fn disable_fake_infection(&mut self) {
        self.is_fake_infected = false;
    }

    /// `RestoreToFullHealth`.
    pub fn restore_to_full_health(&mut self) {
        self.health = 100.0;
        self.additional_pain = 0.0;
        self.alcoholic_bandage = false;
        self.alcohol_level = 0.0;
        self.bleeding = false;
        self.bandaged = false;
        self.bandage_life = 0.0;
        self.bite_time = 0.0;
        self.bitten = false;
        self.bleeding_time = 0.0;
        self.burn_time = 0.0;
        self.comfrey_factor = 0.0;
        self.deep_wounded = false;
        self.deep_wound_time = 0.0;
        self.fracture_time = 0.0;
        self.garlic_factor = 0.0;
        self.have_bullet = false;
        self.have_glass = false;
        self.infected_wound = false;
        self.is_bleeding_stemmed = false;
        self.is_cauterized = false;
        self.is_fake_infected = false;
        self.is_infected = false;
        self.last_time_burn_wash = 0.0;
        self.need_burn_wash = false;
        self.plantain_factor = 0.0;
        self.scratched = false;
        self.scratch_time = 0.0;
        self.splint = false;
        self.splint_factor = 0.0;
        self.splint_item = None;
        self.stitched = false;
        self.stitch_time = 0.0;
        self.wound_infection_level = 0.0;
        self.cut_time = 0.0;
        self.cut = false;
        self.stiffness = 0.0;
    }

    /// `setBandaged(bandaged, bandageLife)`.
    pub fn set_bandaged_simple(&mut self, bandaged: bool, bandage_life: f32) {
        self.set_bandaged(bandaged, bandage_life, false, None);
    }

    /// `setBandaged(bandaged, bandageLife, isAlcoholic, bandageType)`.
    pub fn set_bandaged(
        &mut self,
        bandaged: bool,
        bandage_life: f32,
        is_alcoholic: bool,
        bandage_type: Option<String>,
    ) {
        if bandaged {
            self.bleeding = false;
            self.bitten = false;
            self.scratched = false;
            self.cut = false;
            self.alcoholic_bandage = is_alcoholic;
            self.stitched = false;
            self.deep_wounded = false;
            self.bandage_type = bandage_type;
            self.get_bandage_xp = false;
        } else {
            if self.scratch_time > 0.0 {
                self.scratched = true;
            }
            if self.cut_time > 0.0 {
                self.cut = true;
            }
            if self.bleeding_time > 0.0 {
                self.bleeding = true;
            }
            if self.bite_time > 0.0 {
                self.bitten = true;
            }
            if self.stitch_time > 0.0 {
                self.stitched = true;
            }
            if self.deep_wound_time > 0.0 {
                self.deep_wounded = true;
            }
            self.bandage_type = None;
            self.reset_poultice_factors();
        }
        self.set_bandage_life(bandage_life);
        self.bandaged = bandaged;
    }

    /// `SetBitten(boolean)`.
    pub fn set_bitten(&mut self, env: &InjuryEnv<'_>, bitten: bool) {
        self.bitten = bitten;
        if bitten {
            self.bleeding = true;
            self.is_bleeding_stemmed = false;
            self.is_cauterized = false;
            self.bandaged = false;
            self.infected_wound = true;
            self.bite_time = env.next_float(50.0, 80.0);
            if env.fast_healer {
                self.bite_time = env.next_float(30.0, 50.0);
            } else if env.slow_healer {
                self.bite_time = env.next_float(80.0, 150.0);
            }
        }
        if env.ctx.lore_transmission() != 4 {
            self.is_infected = true;
            self.is_fake_infected = false;
        }
        if self.is_infected && env.ctx.lore_mortality() == 7 {
            self.is_infected = false;
            self.is_fake_infected = true;
        }
        self.generate_bleeding(env);
    }

    /// `SetBitten(boolean, boolean)`.
    pub fn set_bitten_infected(&mut self, ctx: &dyn SimContext, bitten: bool, mut infected: bool) {
        self.bitten = bitten;
        if ctx.lore_transmission() == 4 {
            self.is_infected = false;
            self.is_fake_infected = false;
            infected = false;
        }
        if bitten {
            self.bleeding = true;
            self.is_bleeding_stemmed = false;
            self.is_cauterized = false;
            self.bandaged = false;
            if infected {
                self.is_infected = true;
            }
            self.is_fake_infected = false;
            if self.is_infected && ctx.lore_mortality() == 7 {
                self.is_infected = false;
                self.is_fake_infected = true;
            }
        }
    }

    /// `setBleeding`.
    pub fn set_bleeding(&mut self, bleeding: bool) {
        self.bleeding = bleeding;
    }
    /// `SetBleedingStemmed`.
    pub fn set_bleeding_stemmed(&mut self, _bleeding_stemmed: bool) {
        if self.bleeding {
            self.bleeding = false;
            self.is_bleeding_stemmed = true;
        }
    }
    /// `SetCauterized`.
    pub fn set_cauterized(&mut self, cauterized: bool) {
        self.is_cauterized = cauterized;
        if cauterized {
            self.bleeding = false;
            self.is_bleeding_stemmed = false;
            self.deep_wounded = false;
            self.bandaged = false;
        }
    }

    /// `setCut(cut, forceNoInfection)` (`setCut(cut)` passes `true`).
    pub fn set_cut(&mut self, env: &InjuryEnv<'_>, cut: bool, force_no_infection: bool) {
        self.cut = cut;
        if !cut {
            self.set_bleeding(false);
            return;
        }
        self.set_stitched(env, false);
        self.set_bandaged_simple(false, 0.0);
        let t = env.heal_time((10.0, 20.0), (5.0, 10.0), (20.0, 30.0));
        self.set_cut_time(env.severity(t));
        self.generate_bleeding(env);
        if !force_no_infection {
            self.generate_zombie_infection(env, 25);
        }
    }

    /// `generateZombieInfection`.
    pub fn generate_zombie_infection(&mut self, env: &InjuryEnv<'_>, base_chance: i32) {
        if env.ctx.rand_next(100) < base_chance {
            self.is_infected = true;
        }
        let t = env.ctx.lore_transmission();
        if t == 2 || t == 4 {
            self.is_infected = false;
            self.is_fake_infected = false;
        }
        if self.is_infected && env.ctx.lore_mortality() == 7 {
            self.is_infected = false;
            self.is_fake_infected = true;
        }
    }

    /// `setScratched(scratched, forceNoInfection)`.
    pub fn set_scratched(&mut self, env: &InjuryEnv<'_>, scratched: bool, force_no_infection: bool) {
        self.scratched = scratched;
        if !scratched {
            self.set_bleeding(false);
            return;
        }
        self.set_stitched(env, false);
        self.set_bandaged_simple(false, 0.0);
        let t = env.heal_time((7.0, 15.0), (4.0, 10.0), (15.0, 25.0));
        self.set_scratch_time(env.severity(t));
        if !force_no_infection {
            self.generate_zombie_infection(env, 7);
        }
    }

    /// `SetScratchedWeapon`.
    pub fn set_scratched_weapon(&mut self, env: &InjuryEnv<'_>, scratched: bool) {
        self.scratched = scratched;
        if !scratched {
            self.set_bleeding(false);
            return;
        }
        self.set_stitched(env, false);
        self.set_bandaged_simple(false, 0.0);
        let t = env.heal_time((5.0, 10.0), (1.0, 5.0), (10.0, 20.0));
        self.set_scratch_time(env.severity(t));
        self.generate_bleeding(env);
    }

    /// `generateDeepWound`.
    pub fn generate_deep_wound(&mut self, env: &InjuryEnv<'_>) {
        let t = env.heal_time((15.0, 20.0), (11.0, 15.0), (20.0, 32.0));
        self.deep_wound_time = env.severity(t);
        self.set_deep_wounded(true);
        self.generate_bleeding(env);
    }

    /// `generateDeepShardWound`.
    pub fn generate_deep_shard_wound(&mut self, env: &InjuryEnv<'_>) {
        let t = env.heal_time((15.0, 20.0), (11.0, 15.0), (20.0, 32.0));
        self.deep_wound_time = env.severity(t);
        self.have_glass = true;
        self.set_deep_wounded(true);
        self.generate_bleeding(env);
    }

    /// `generateFracture`.
    pub fn generate_fracture(&mut self, ctx: &dyn SimContext, fracture_time: f32) {
        if !ctx.bone_fracture() {
            return;
        }
        if self.fracture_time <= 0.0 && fracture_time > 0.0 {
            ctx.event(SimEvent::PlaySound("FirstAidFracture".to_string()));
        }
        self.fracture_time = fracture_time;
    }

    /// `generateFractureNew`.
    pub fn generate_fracture_new(&mut self, env: &InjuryEnv<'_>, mut fracture_time: f32) {
        if env.fast_healer {
            fracture_time *= 0.6;
        } else if env.slow_healer {
            fracture_time *= 1.8;
        }
        let t = env.severity(fracture_time);
        self.generate_fracture(env.ctx, t);
    }

    /// `SetScratchedWindow`.
    pub fn set_scratched_window(&mut self, env: &InjuryEnv<'_>, scratched: bool) {
        if !scratched {
            return;
        }
        self.set_bandaged_simple(false, 0.0);
        self.set_stitched(env, false);
        if env.ctx.rand_next(7) == 0 {
            self.generate_deep_shard_wound(env);
        } else {
            self.scratched = scratched;
            let t = env.heal_time((12.0, 20.0), (5.0, 10.0), (20.0, 30.0));
            self.set_scratch_time(env.severity(t));
        }
        self.generate_bleeding(env);
    }

    /// `setStitched`.
    pub fn set_stitched(&mut self, env: &InjuryEnv<'_>, stitched: bool) {
        if stitched {
            self.set_bleeding_time(0.0);
            self.set_bleeding(false);
            self.deep_wound_time = 0.0;
            self.set_deep_wounded(false);
            self.get_stitch_xp = false;
        } else if self.stitched {
            self.stitched = false;
            if self.stitch_time < 40.0 {
                self.deep_wound_time = env.next_float(10.0, self.stitch_time);
                let t = env.next_float(10.0, self.stitch_time);
                self.set_bleeding_time(t);
                self.stitch_time = 0.0;
                self.set_deep_wounded(true);
            } else {
                let t = env.next_float(2.0, self.stitch_time - 40.0);
                self.set_scratch_time(t);
                self.scratched = true;
                self.stitch_time = 0.0;
            }
        }
        self.stitched = stitched;
    }

    /// `damageFromFirearm`.
    pub fn damage_from_firearm(&mut self, env: &InjuryEnv<'_>, _damage: f32) {
        self.set_have_bullet(env, true, 0);
    }

    /// `getPain`.
    pub fn pain(&self, ctx: &dyn SimContext) -> f32 {
        let mut result = 0.0f32;
        if self.scratch_time > 0.0 {
            result += self.scratch_time * 1.7;
        }
        if self.cut_time > 0.0 {
            result += self.cut_time * 2.5;
        }
        if self.bite_time > 0.0 {
            result += if self.bandaged { 30.0 } else { 50.0 };
        }
        if self.deep_wound_time > 0.0 {
            result += self.deep_wound_time * 3.7;
        }
        if self.stitch_time > 0.0 && self.stitch_time < 35.0 {
            result += if self.bandaged { (35.0 - self.stitch_time) / 2.0 } else { 35.0 - self.stitch_time };
        }
        if self.fracture_time > 0.0 {
            result += if self.splint_factor > 0.0 { self.fracture_time / 2.0 } else { self.fracture_time };
        }
        if self.have_bullet {
            result += 50.0;
        }
        if self.have_glass {
            result += 10.0;
        }
        if self.burn_time > 0.0 {
            result += self.burn_time;
        }
        if self.bandaged {
            result /= 1.5;
        }
        if self.wound_infection_level > 0.0 {
            result += self.wound_infection_level;
        }
        result += self.additional_pain_with(true);
        match ctx.injury_severity() {
            1 => result *= 0.7,
            3 => result *= 1.3,
            _ => {}
        }
        result
    }

    /// `getAdditionalPain(includeStiffness)`.
    pub fn additional_pain_with(&self, include_stiffness: bool) -> f32 {
        if include_stiffness {
            self.additional_pain + self.stiffness / 3.5
        } else {
            self.additional_pain
        }
    }

    /// `setSplint`.
    pub fn set_splint(&mut self, splint: bool, splint_factor: f32) {
        self.splint = splint;
        self.splint_factor = splint_factor;
        if splint {
            self.get_splint_xp = false;
        }
    }

    /// `setHaveBullet`.
    pub fn set_have_bullet(&mut self, env: &InjuryEnv<'_>, have_bullet: bool, doctor_level: i32) {
        if self.have_bullet && !have_bullet {
            let d = (doctor_level / 2) as f32;
            let mut t = env.next_float(17.0, 23.0) - d;
            if env.fast_healer {
                t = env.next_float(12.0, 18.0) - d;
            } else if env.slow_healer {
                t = env.next_float(22.0, 28.0) - d;
            }
            self.deep_wound_time = env.severity(t);
            self.set_deep_wounded(true);
            self.have_bullet = false;
            self.generate_bleeding(env);
        } else if have_bullet {
            self.have_bullet = true;
            self.generate_bleeding(env);
        }
        self.have_bullet = have_bullet;
    }

    /// `setNeedBurnWash`.
    pub fn set_need_burn_wash(&mut self, need: bool) {
        if self.need_burn_wash && !need {
            self.last_time_burn_wash = self.burn_time;
        }
        self.need_burn_wash = need;
    }

    /// `setBleedingTime`.
    pub fn set_bleeding_time(&mut self, t: f32) {
        self.bleeding_time = t;
        if !self.bandaged {
            self.set_bleeding(t > 0.0);
        }
    }

    /// `setDeepWounded`.
    pub fn set_deep_wounded(&mut self, wounded: bool) {
        self.deep_wounded = wounded;
        if wounded {
            self.bleeding = true;
            self.is_bleeding_stemmed = false;
            self.is_cauterized = false;
            self.bandaged = false;
            self.stitched = false;
        }
    }

    /// `setBandageLife`.
    pub fn set_bandage_life(&mut self, life: f32) {
        self.bandage_life = life;
        if self.bandage_life <= 0.0 {
            self.alcoholic_bandage = false;
        }
    }

    /// `setScratchTime`: at most 100.
    pub fn set_scratch_time(&mut self, t: f32) {
        self.scratch_time = java_min(100.0, t);
    }

    /// `setCutTime`: at most 100.
    pub fn set_cut_time(&mut self, t: f32) {
        self.cut_time = java_min(100.0, t);
    }

    /// `setWoundInfectionLevel`.
    pub fn set_wound_infection_level(&mut self, level: f32) {
        self.wound_infection_level = level;
        if self.wound_infection_level <= 0.0 {
            self.infected_wound = false;
            if self.wound_infection_level < -2.0 {
                self.wound_infection_level = -2.0;
            }
        } else {
            self.infected_wound = true;
        }
    }

    /// `setBurned`.
    pub fn set_burned(&mut self, env: &InjuryEnv<'_>) {
        let t = env.next_float(50.0, 100.0);
        self.burn_time = env.severity(t);
        self.set_need_burn_wash(true);
        self.last_time_burn_wash = 0.0;
    }

    /// `setPlantainFactor`.
    pub fn set_plantain_factor(&mut self, v: f32) {
        self.plantain_factor = clamp_f32(v, 0.0, 100.0);
    }
    /// `setGarlicFactor`.
    pub fn set_garlic_factor(&mut self, v: f32) {
        self.garlic_factor = clamp_f32(v, 0.0, 100.0);
    }
    /// `setComfreyFactor`.
    pub fn set_comfrey_factor(&mut self, v: f32) {
        self.comfrey_factor = clamp_f32(v, 0.0, 100.0);
    }
    /// `setWetness`.
    pub fn set_wetness(&mut self, v: f32) {
        self.wetness = clamp_f32(v, 0.0, 100.0);
    }
    /// `setStiffness`.
    pub fn set_stiffness(&mut self, v: f32) {
        self.stiffness = clamp_f32(v, 0.0, 100.0);
    }
    /// `addStiffness`.
    pub fn add_stiffness(&mut self, v: f32) {
        self.stiffness = clamp_f32(self.stiffness + v, 0.0, 100.0);
    }

    pub(crate) fn reset_poultice_factors(&mut self) {
        self.set_comfrey_factor(0.0);
        self.set_garlic_factor(0.0);
        self.set_plantain_factor(0.0);
    }

    /// `isBurnt`.
    pub fn is_burnt(&self) -> bool {
        self.burn_time > 0.0
    }

    /// `generateBleeding`.
    pub fn generate_bleeding(&mut self, env: &InjuryEnv<'_>) {
        let mut t = 0.0f32;
        if self.scratched {
            t = env.next_float(self.scratch_time * 0.3, self.scratch_time * 0.6);
        }
        if self.cut {
            t += env.next_float(self.cut_time * 0.7, self.cut_time * 1.0);
        }
        if self.is_burnt() {
            t += env.next_float(self.burn_time * 0.3, self.burn_time * 0.6);
        }
        if self.deep_wounded {
            t += env.next_float(self.deep_wound_time * 0.7, self.deep_wound_time);
        }
        if self.have_glass {
            t += env.next_float(5.0, 10.0);
        }
        if self.have_bullet {
            t += env.next_float(5.0, 10.0);
        }
        if self.bitten {
            t += env.next_float(7.5, 15.0);
        }
        t = env.severity(t);
        t *= self.bleeding_time_modifier;
        self.set_bleeding_time(t);
    }

    /// `getInnerTemperature` (no thermal node: 0).
    pub fn inner_temperature(&self) -> f32 {
        0.0
    }
    /// `getSkinTemperature` (no thermal node: 0).
    pub fn skin_temperature(&self) -> f32 {
        0.0
    }
    /// `getDistToCore` (no thermal node: the type's).
    pub fn dist_to_core(&self) -> f32 {
        self.dist_to_core
    }
    /// `getSkinSurface` (no thermal node: the type's).
    pub fn skin_surface(&self) -> f32 {
        self.skin_surface
    }

    /// `getBandageNeededDamageLevel`.
    pub fn bandage_needed_damage_level(&self) -> f32 {
        if self.bandaged {
            return 0.0;
        }
        let bleed = if self.bleeding_time > 0.0 { BLEED_DAMAGE * (self.bleeding_time / 10.0) } else { 0.0 };
        let bite = if self.bitten { BITE_DAMAGE } else { 0.0 };
        let burn = if self.is_burnt() { BURN_DAMAGE } else { 0.0 };
        let cut = if self.cut { CUT_DAMAGE } else { 0.0 };
        let deep = if (self.deep_wound_time > 0.0) & !self.stitched { 3.125 } else { 0.0 };
        let scratch = if self.scratched { SCRATCH_DAMAGE } else { 0.0 };
        let damage = java_max(java_max(java_max(java_max(bite, burn), cut), deep), scratch) + bleed;
        damage * self.damage_scaler
    }

    /// The healing and damage half of `DamageUpdate` that touches only this
    /// part (everything after the damage over time), given the `Rand`
    /// rolls. Returns nothing; see
    /// [`super::GameCharacter::body_part_damage_update`].
    pub(crate) fn heal_update(
        &mut self,
        ctx: &dyn SimContext,
        reduce_infection_power: &mut f32,
        dirty_clothing: bool,
        bloody_clothing: bool,
        stiffness_ongoing: Option<bool>,
    ) {
        let mult = ctx.multiplier();
        if self.bite_time > 0.0 {
            if self.bandaged {
                self.bite_time -= dmul(1.0e-4, mult);
                self.set_bandage_life(self.bandage_life - dmul(1.0e-4, mult));
            } else {
                self.bite_time -= dmul(5.0e-6, mult);
            }
        }
        if self.burn_time > 0.0 {
            if self.bandaged {
                self.burn_time -= dmul(1.0e-4, mult);
                self.set_bandage_life(self.bandage_life - dmul(1.0e-4, mult));
            } else {
                self.burn_time -= dmul(5.0e-6, mult);
            }
            if self.last_time_burn_wash - self.burn_time >= 20.0 {
                self.last_time_burn_wash = 0.0;
                self.set_need_burn_wash(true);
            }
        }
        if self.bleeding_time > 0.0 {
            if self.bandaged {
                self.set_bleeding_time(self.bleeding_time - dmul(2.0e-4, mult));
                if self.deep_wound_time > 0.0 {
                    self.set_bandage_life(self.bandage_life - dmul(0.005, mult));
                } else {
                    self.set_bandage_life(self.bandage_life - dmul(3.0e-4, mult));
                }
            } else {
                self.set_bleeding_time(self.bleeding_time - dmul(2.0e-5, mult));
            }
            if self.bleeding_time < 3.0 && self.have_glass {
                self.set_bleeding_time(3.0);
            }
            if self.bleeding_time < 0.0 {
                self.set_bleeding_time(0.0);
                self.set_bleeding(false);
            }
        }
        let wound_open = self.deep_wound_time > 0.0 || self.scratch_time > 0.0 || self.cut_time > 0.0;
        if !(self.infected_wound
            || self.is_infected
            || self.alcoholic_bandage && self.bandage_life > 0.0
            || !wound_open && self.stitch_time <= 0.0)
        {
            let mut chance = 40000;
            if !self.bandaged {
                chance -= 10000;
            } else if self.bandage_life == 0.0 {
                chance -= 35000;
            }
            if self.scratch_time > 0.0 {
                chance -= 20000;
            }
            if self.cut_time > 0.0 {
                chance -= 25000;
            }
            if self.deep_wound_time > 0.0 {
                chance -= 30000;
            }
            if self.have_glass {
                chance -= 24000;
            }
            if self.burn_time > 0.0 {
                chance -= 23000;
                if self.need_burn_wash {
                    chance -= 7000;
                }
            }
            if dirty_clothing {
                chance -= 20000;
            }
            if bloody_clothing {
                chance -= 24000;
            }
            if chance <= 5000 {
                chance = 5000;
            }
            if ctx.rand_next(adjust_for_framerate(ctx, chance)) == 0 {
                self.infected_wound = true;
            }
        } else if self.infected_wound {
            let mut reduce = false;
            if self.alcohol_level > 0.0 {
                self.alcohol_level -= 2.0e-4 * mult;
                self.set_wound_infection_level(self.wound_infection_level - 2.0e-4 * mult);
                if self.alcohol_level < 0.0 {
                    self.alcohol_level = 0.0;
                }
                reduce = true;
            }
            if *reduce_infection_power > 0.0 {
                self.set_wound_infection_level(self.wound_infection_level - 2.0e-4 * mult);
                *reduce_infection_power -= 2.0e-4 * mult;
                if *reduce_infection_power < 0.0 {
                    *reduce_infection_power = 0.0;
                }
                reduce = true;
            }
            if self.garlic_factor > 0.0 {
                self.set_wound_infection_level(self.wound_infection_level - 2.0e-4 * mult);
                self.set_garlic_factor(self.garlic_factor - 8.0e-4 * mult);
                reduce = true;
            }
            if !reduce {
                let k = if self.is_infected {
                    2.0e-4
                } else if self.have_glass {
                    1.0e-4
                } else {
                    1.0e-5
                };
                self.set_wound_infection_level(self.wound_infection_level + k * mult);
            }
        }
        if self.wound_infection_level > 10.0 {
            self.set_wound_infection_level(10.0);
        }
        if !self.infected_wound && self.alcohol_level > 0.0 {
            self.alcohol_level -= 2.0e-4 * mult;
            if self.alcohol_level < 0.0 {
                self.alcohol_level = 0.0;
            }
        }
        if self.infected_wound && self.bandage_life > 0.0 {
            if self.alcoholic_bandage {
                self.set_wound_infection_level(self.wound_infection_level - 6.0e-4 * mult);
            }
            self.set_bandage_life(self.bandage_life - dmul(2.0e-4, mult));
        }
        if self.scratch_time > 0.0 {
            if self.bandaged {
                self.set_scratch_time(self.scratch_time - dmul(1.5e-4, mult));
                self.set_bandage_life(self.bandage_life - dmul(8.0e-5, mult));
                if self.plantain_factor > 0.0 {
                    self.set_scratch_time(self.scratch_time - dmul(1.0e-4, mult));
                    self.set_plantain_factor(self.plantain_factor - dmul(8.0e-4, mult));
                }
            } else {
                self.set_scratch_time(self.scratch_time - dmul(1.0e-5, mult));
            }
            if self.scratch_time < 0.0 {
                self.set_scratch_time(0.0);
                self.get_bandage_xp = true;
                self.get_stitch_xp = true;
                // `setScratched(false, false)`.
                self.scratched = false;
                self.set_bleeding(false);
                self.set_bleeding(false);
                self.set_bleeding_time(0.0);
                self.reset_poultice_factors();
            }
        }
        if self.cut_time > 0.0 {
            if self.bandaged {
                self.set_cut_time(self.cut_time - dmul(5.0e-5, mult));
                self.set_bandage_life(self.bandage_life - dmul(1.0e-5, mult));
                if self.plantain_factor > 0.0 {
                    self.set_cut_time(self.cut_time - dmul(5.0e-5, mult));
                    self.set_plantain_factor(self.plantain_factor - dmul(8.0e-4, mult));
                }
            } else {
                self.set_cut_time(self.cut_time - dmul(1.0e-6, mult));
            }
            if self.cut_time < 0.0 {
                self.set_cut_time(0.0);
                self.get_bandage_xp = true;
                self.get_stitch_xp = true;
                self.set_bleeding(false);
                self.set_bleeding_time(0.0);
                self.reset_poultice_factors();
            }
        }
        if self.deep_wound_time > 0.0 {
            if self.bandaged {
                self.deep_wound_time -= dmul(2.0e-5, mult);
                self.set_bandage_life(self.bandage_life - dmul(1.0e-4, mult));
                if self.plantain_factor > 0.0 {
                    self.deep_wound_time -= dmul(7.0e-6, mult);
                    self.set_plantain_factor(self.plantain_factor - dmul(8.0e-4, mult));
                    if self.plantain_factor < 0.0 {
                        self.set_plantain_factor(0.0);
                    }
                }
            } else {
                self.deep_wound_time -= dmul(2.0e-6, mult);
            }
            if (self.have_glass || !self.bandaged) && self.deep_wound_time < 3.0 {
                self.deep_wound_time = 3.0;
            }
            if self.deep_wound_time < 0.0 {
                self.get_bandage_xp = true;
                self.get_stitch_xp = true;
                self.deep_wound_time = 0.0;
                self.set_deep_wounded(false);
            }
        }
        if self.stitch_time > 0.0 && self.stitch_time < 50.0 {
            if self.bandaged {
                self.stitch_time += dmul(4.0e-4, mult);
                self.set_bandage_life(self.bandage_life - dmul(1.0e-4, mult));
                if !self.alcoholic_bandage && ctx.rand_next(adjust_for_framerate(ctx, 80000)) == 0 {
                    self.infected_wound = true;
                }
                self.stitch_time += dmul(1.0e-4, mult);
            } else {
                self.stitch_time += dmul(2.0e-4, mult);
                if ctx.rand_next(adjust_for_framerate(ctx, 20000)) == 0 {
                    self.infected_wound = true;
                }
            }
            if self.stitch_time > 30.0 {
                self.get_stitch_xp = true;
            }
            if self.stitch_time > 50.0 {
                self.stitch_time = 50.0;
            }
        }
        if self.fracture_time > 0.0 {
            if self.splint_factor > 0.0 {
                self.fracture_time -= (5.0e-5 * mult as f64 * self.splint_factor as f64) as f32;
            } else {
                self.fracture_time -= dmul(5.0e-6, mult);
            }
            if self.comfrey_factor > 0.0 {
                self.fracture_time -= dmul(5.0e-6, mult);
                self.set_comfrey_factor(self.comfrey_factor - dmul(5.0e-4, mult));
            }
            if self.fracture_time < 0.0 {
                self.fracture_time = 0.0;
                self.get_splint_xp = true;
                self.reset_poultice_factors();
            }
        }
        if self.additional_pain > 0.0 {
            self.additional_pain -= dmul(0.005, mult);
            if self.additional_pain < 0.0 {
                self.additional_pain = 0.0;
            }
        }
        if self.stiffness > 0.0 && stiffness_ongoing == Some(false) {
            self.set_stiffness(self.stiffness - dmul(0.002, mult));
            if self.stiffness < 0.0 {
                self.set_stiffness(0.0);
            }
        }
        if self.bandage_life < 0.0 {
            self.set_bandage_life(0.0);
            self.get_bandage_xp = true;
        }
        if (self.wound_infection_level > 0.0 || self.infected_wound)
            && self.burn_time <= 0.0
            && self.fracture_time <= 0.0
            && self.deep_wound_time <= 0.0
            && self.scratch_time <= 0.0
            && self.bite_time <= 0.0
            && self.cut_time <= 0.0
            && self.stitch_time <= 0.0
        {
            self.set_wound_infection_level(0.0);
        }
        if self.health < 0.0 {
            self.health = 0.0;
        }
    }
}

/// `Rand.AdjustForFramerate`.
pub fn adjust_for_framerate(ctx: &dyn SimContext, chance: i32) -> i32 {
    if ctx.is_server() {
        (chance as f32 * 0.333_333_34) as i32
    } else {
        (chance as f32 * (ctx.lock_fps() as f32 / 30.0)) as i32
    }
}

/// `Rand.NextBool(int invProbability)`.
pub fn next_bool(ctx: &dyn SimContext, inv_probability: i32) -> bool {
    ctx.rand_next(inv_probability) == 0
}

/// `Rand.PerThirtiethOfASecond`.
pub fn per_thirtieth_of_a_second(ctx: &dyn SimContext, chance: i32) -> bool {
    let mult = ctx.thirty_fps_multiplier();
    ctx.rand_next_float(0.0, chance as f32 * mult) < mult
}

// ---- BodyPartLast ----------------------------------------------------------

/// `BodyPartLast`: a part's state at the last `setBodyPartsLastState`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct BodyPartLast {
    pub bandaged: bool,
    pub bitten: bool,
    pub scratched: bool,
    pub cut: bool,
    pub dirty_bandage: bool,
}

impl BodyPartLast {
    /// `copy`.
    pub fn copy(&mut self, p: &BodyPart) {
        self.bandaged = p.bandaged;
        self.bitten = p.bitten;
        self.scratched = p.scratched;
        self.cut = p.cut;
        self.dirty_bandage = p.bandaged && p.is_bandage_dirty();
    }
}

// ---- BodyPartContacts ------------------------------------------------------

/// One node of `BodyPartContacts`' tree (by part index).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct ContactNode {
    depth: i32,
    parent: Option<usize>,
    children: Vec<usize>,
    all_contacts: Vec<usize>,
    initialised: bool,
}

/// `BodyPartContacts`: the static contact tree rooted at `Torso_Upper`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BodyPartContacts {
    nodes: Vec<ContactNode>,
}

impl BodyPartContacts {
    /// The static initializer.
    pub fn new(types: &BodyPartTypes) -> ScriptResult<Self> {
        let max = types.max_index();
        let mut nodes = vec![ContactNode { depth: -1, ..ContactNode::default() }; max];
        let tree: [(&str, &[&str]); 12] = [
            ("Torso_Upper", &["Neck", "Torso_Lower", "UpperArm_L", "UpperArm_R"]),
            ("Neck", &["Head"]),
            ("UpperArm_L", &["ForeArm_L"]),
            ("ForeArm_L", &["Hand_L"]),
            ("UpperArm_R", &["ForeArm_R"]),
            ("ForeArm_R", &["Hand_R"]),
            ("Torso_Lower", &["Groin"]),
            ("Groin", &["UpperLeg_L", "UpperLeg_R"]),
            ("UpperLeg_L", &["LowerLeg_L"]),
            ("LowerLeg_L", &["Foot_L"]),
            ("UpperLeg_R", &["LowerLeg_R"]),
            ("LowerLeg_R", &["Foot_R"]),
        ];
        for (p, cs) in tree {
            let pi = types.index(p)?;
            let mut v = Vec::new();
            for c in cs {
                v.push(types.index(c)?);
            }
            nodes[pi].children = v;
        }
        let root = types.index("Torso_Upper")?;
        fn init(nodes: &mut [ContactNode], cur: usize, depth: i32, up: Option<usize>) {
            nodes[cur].parent = up;
            nodes[cur].depth = depth;
            let mut all = Vec::new();
            if let Some(p) = up {
                all.push(p);
            }
            for c in nodes[cur].children.clone() {
                all.push(c);
                init(nodes, c, depth + 1, Some(cur));
            }
            nodes[cur].all_contacts = all;
            nodes[cur].initialised = true;
        }
        init(&mut nodes, root, 0, None);
        Ok(BodyPartContacts { nodes })
    }

    /// `getAllContacts` (`None` for `MAX`).
    pub fn all_contacts(&self, part: usize) -> Option<&[usize]> {
        self.nodes.get(part).map(|n| n.all_contacts.as_slice())
    }
    /// `getChildren`.
    pub fn children(&self, part: usize) -> Option<&[usize]> {
        self.nodes.get(part).map(|n| n.children.as_slice())
    }
    /// `getParent`.
    pub fn parent(&self, part: usize) -> Option<usize> {
        self.nodes.get(part).and_then(|n| n.parent)
    }
    /// `getNodeDepth` (-1 when absent).
    pub fn node_depth(&self, part: usize) -> i32 {
        self.nodes.get(part).map_or(-1, |n| n.depth)
    }
}
