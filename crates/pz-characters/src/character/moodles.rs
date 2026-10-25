//! `zombie.characters.Moodles.Moodle`, `MoodleStat` and `Moodles`.
//!
//! The moodle types are the `moodle_type` registry (`MoodleType`'s
//! `registerBase` keys, read from the jar); `MoodleStat`'s thresholds are
//! replayed from its static initializer ([`BuiltinTables::moodle_stats`]).
//! `Moodle.Update` branches on the type's Java field (`MoodleType.PANIC`),
//! resolved once per moodle from the jar's constant table; types registered
//! by mods match none and stay at the minimum level, as in the original.

use std::cell::RefCell;
use std::rc::Rc;

use pz_scripts::objects::items::MoodleType;
use pz_scripts::registry::builtin::BuiltinTables;
use pz_scripts::registry::RegistryKey;
use pz_scripts::{ScriptManager, ScriptResult};

use super::context::{SimContext, SimEvent};
use super::GameCharacter;
use crate::color::Color;
use crate::Characters;

/// `Moodle.MoodleLevel`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum MoodleLevel {
    MinMoodleLevel = 0,
    LowMoodleLevel = 1,
    ModerateMoodleLevel = 2,
    HighMoodleLevel = 3,
    MaxMoodleLevel = 4,
}

const MIN: i32 = MoodleLevel::MinMoodleLevel as i32;
const LOW: i32 = MoodleLevel::LowMoodleLevel as i32;
const MODERATE: i32 = MoodleLevel::ModerateMoodleLevel as i32;
const HIGH: i32 = MoodleLevel::HighMoodleLevel as i32;
const MAX: i32 = MoodleLevel::MaxMoodleLevel as i32;

impl MoodleLevel {
    /// `values()` names.
    pub const NAMES: [&'static str; 5] =
        ["MinMoodleLevel", "LowMoodleLevel", "ModerateMoodleLevel", "HighMoodleLevel", "MaxMoodleLevel"];
}

/// `MoodleStat`.
#[derive(Clone, Debug, PartialEq)]
pub struct MoodleStat {
    /// `moodleType`, as its `MoodleType` field.
    pub moodle_type: String,
    pub minimum_threshold: f32,
    pub lowest_threshold: f32,
    pub moderate_threshold: f32,
    pub highest_threshold: f32,
    pub maximum_threshold: f32,
}

pub type MoodleStatRef = Rc<RefCell<MoodleStat>>;

/// `MoodleStat`'s statics: the constants (thresholds are mutable through
/// the setters, and shared).
#[derive(Clone, Debug, Default)]
pub struct MoodleStats {
    fields: Vec<(String, MoodleStatRef)>,
}

impl MoodleStats {
    /// Replays the static initializer described by `tables`
    /// (`register` keeps the first registration of a type).
    pub fn from_tables(tables: &BuiltinTables) -> Self {
        let mut s = MoodleStats::default();
        for d in &tables.moodle_stats {
            let stat = match s.get(&d.moodle_type) {
                Some(existing) => existing,
                None => {
                    let [minimum, lowest, moderate, highest, maximum] = d.thresholds;
                    Rc::new(RefCell::new(MoodleStat {
                        moodle_type: d.moodle_type.clone(),
                        minimum_threshold: minimum,
                        lowest_threshold: lowest,
                        moderate_threshold: moderate,
                        highest_threshold: highest,
                        maximum_threshold: maximum,
                    }))
                }
            };
            s.fields.push((d.field.clone(), stat));
        }
        s
    }

    /// The static field `field` (`MoodleStat.PANIC`).
    pub fn constant(&self, field: &str) -> Option<MoodleStatRef> {
        self.fields.iter().find(|(f, _)| f == field).map(|(_, s)| s.clone())
    }

    /// `get(MoodleType)`, by the type's Java field.
    pub fn get(&self, moodle_type_field: &str) -> Option<MoodleStatRef> {
        self.fields.iter().find(|(_, s)| s.borrow().moodle_type == moodle_type_field).map(|(_, s)| s.clone())
    }

    /// The thresholds of constant `field` (minimum, lowest, moderate,
    /// highest, maximum); a missing constant is the original's NPE.
    fn thresholds(&self, field: &str) -> ScriptResult<[f32; 5]> {
        let s = self.constant(field).ok_or_else(|| {
            pz_scripts::ScriptError::new(format!("java.lang.NullPointerException: MoodleStat.{field}"))
        })?;
        let s = s.borrow();
        Ok([s.minimum_threshold, s.lowest_threshold, s.moderate_threshold, s.highest_threshold, s.maximum_threshold])
    }
}

/// A chevron colour. `Moodle` compares colours by identity: `Color.white`
/// and its two private constants are singletons, any other colour is the
/// object Lua passed.
#[derive(Clone, Debug)]
pub enum ChevronColor {
    /// `Color.white`.
    White,
    /// `colorNeg` (0.88235295, 0.15686275, 0.15686275).
    Negative,
    /// `colorPos` (0.15686275, 0.88235295, 0.15686275).
    Positive,
    Other(Rc<Color>),
}

impl PartialEq for ChevronColor {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (ChevronColor::White, ChevronColor::White)
            | (ChevronColor::Negative, ChevronColor::Negative)
            | (ChevronColor::Positive, ChevronColor::Positive) => true,
            (ChevronColor::Other(a), ChevronColor::Other(b)) => Rc::ptr_eq(a, b),
            _ => false,
        }
    }
}

impl ChevronColor {
    /// The colour's components.
    pub fn color(&self) -> Color {
        match self {
            ChevronColor::White => Color { r: 1.0, g: 1.0, b: 1.0, a: 1.0 },
            ChevronColor::Negative => Color { r: 0.882_352_95, g: 0.156_862_75, b: 0.156_862_75, a: 1.0 },
            ChevronColor::Positive => Color { r: 0.156_862_75, g: 0.882_352_95, b: 0.156_862_75, a: 1.0 },
            ChevronColor::Other(c) => **c,
        }
    }
}

/// `Moodle`.
#[derive(Clone, Debug)]
pub struct Moodle {
    moodle_type: Rc<RegistryKey>,
    /// The `MoodleType` field of the type, if it is a built-in one.
    type_field: Option<String>,
    moodle_level: i32,
    pain_timer: i32,
    chevron_color: ChevronColor,
    chevron_is_up: bool,
    chevron_count: i32,
    cant_sprint_timer: i32,
}

impl Moodle {
    const MAX_CHEVRONS: i32 = 3;
    const CANT_SPRINT_TIMER_DURATION: i32 = 300;
    const PAIN_TIMER_DURATION: i32 = 120;

    /// `new Moodle(type, chr)`.
    pub fn new(sm: &ScriptManager, moodle_type: Rc<RegistryKey>) -> Self {
        let type_field = sm
            .statics
            .items
            .borrow()
            .constants
            .constants(MoodleType::REGISTRY)
            .into_iter()
            .find(|(_, id)| pz_scripts::registry::ResourceLocation::of(id).is_ok_and(|l| l == moodle_type.location))
            .map(|(f, _)| f.to_string());
        Moodle {
            moodle_type,
            type_field,
            moodle_level: MIN,
            pain_timer: 0,
            chevron_color: ChevronColor::White,
            chevron_is_up: true,
            chevron_count: 0,
            cant_sprint_timer: Self::CANT_SPRINT_TIMER_DURATION,
        }
    }

    /// `getMoodleType`.
    pub fn moodle_type(&self) -> &Rc<RegistryKey> {
        &self.moodle_type
    }
    /// `getChevronCount`.
    pub fn chevron_count(&self) -> i32 {
        self.chevron_count
    }
    /// `isChevronIsUp`.
    pub fn is_chevron_is_up(&self) -> bool {
        self.chevron_is_up
    }
    /// `getChevronColor`.
    pub fn chevron_color(&self) -> &ChevronColor {
        &self.chevron_color
    }
    /// `chevronDifference`.
    pub fn chevron_difference(&self, count: i32, is_up: bool, col: &ChevronColor) -> bool {
        count != self.chevron_count || is_up != self.chevron_is_up || *col != self.chevron_color
    }
    /// `setChevron`: the count clamped to 0..=3; `None` is white.
    pub fn set_chevron(&mut self, count: i32, is_up: bool, col: Option<ChevronColor>) {
        self.chevron_count = count.clamp(0, Self::MAX_CHEVRONS);
        self.chevron_is_up = is_up;
        self.chevron_color = col.unwrap_or(ChevronColor::White);
    }
    /// `getLevel`.
    pub fn level(&self) -> i32 {
        self.moodle_level
    }

    fn update_moodle_level(&mut self, level: i32) -> bool {
        if level != self.moodle_level {
            self.moodle_level = MIN.max(level.min(MAX));
            return true;
        }
        false
    }

    fn is(&self, field: &str) -> bool {
        self.type_field.as_deref() == Some(field)
    }

    /// `Update`: whether the level changed.
    pub fn update<I>(
        &mut self,
        chr: &mut GameCharacter<I>,
        sm: &ScriptManager,
        chars: &Characters,
        ctx: &dyn SimContext,
    ) -> ScriptResult<bool> {
        let ms = &chars.sim.moodle_stats;
        let cs = &chars.sim.character_stats;
        let mut level = MIN;
        if chr.is_dead() && !self.is("DEAD") && !self.is("ZOMBIE") {
            return Ok(self.update_moodle_level(level));
        }
        let alive = chr.body_damage.health() != 0.0;
        // The ascending "> lowest, > moderate, > highest, > maximum" ladder.
        let ladder = |value: f32, t: [f32; 5], level: &mut i32| {
            if value > t[1] {
                *level = LOW;
            }
            if value > t[2] {
                *level = MODERATE;
            }
            if value > t[3] {
                *level = HIGH;
            }
            if value > t[4] {
                *level = MAX;
            }
        };
        if self.is("CANT_SPRINT") && chr.is_player() && chr.moodle_cant_sprint {
            level = LOW;
            self.cant_sprint_timer -= 1;
            ctx.event(SimEvent::MoodleWiggle(self.moodle_type.clone()));
            if self.cant_sprint_timer == 0 {
                level = MIN;
                self.cant_sprint_timer = Self::CANT_SPRINT_TIMER_DURATION;
                chr.moodle_cant_sprint = false;
            }
        }
        if self.is("ENDURANCE") && alive {
            let e = chr.stats.get(&cs.field("ENDURANCE")?);
            let t = ms.thresholds("ENDURANCE")?;
            level = if e > t[0] {
                MIN
            } else if e > t[1] {
                LOW
            } else if e > t[2] {
                MODERATE
            } else if e > t[3] {
                HIGH
            } else {
                MAX
            };
        }
        if self.is("ANGRY") {
            let a = chr.stats.get(&cs.field("ANGER")?);
            let t = ms.thresholds("ANGRY")?;
            if a > t[4] {
                level = MAX;
            } else if a > t[3] {
                level = HIGH;
            } else if a > t[2] {
                level = MODERATE;
            } else if a > t[1] {
                level = LOW;
            }
        }
        for (ty, stat, thresholds) in
            [("TIRED", "FATIGUE", "TIRED"), ("HUNGRY", "HUNGER", "HUNGRY"), ("PANIC", "PANIC", "PANIC")]
        {
            if self.is(ty) && alive {
                ladder(chr.stats.get(&cs.field(stat)?), ms.thresholds(thresholds)?, &mut level);
            }
        }
        if self.is("SICK") && alive {
            let s = chr.body_damage.apparent_infection_level(&chr.stats, cs)? / 100.0
                + chr.stats.get(&cs.field("SICKNESS")?);
            ladder(s, ms.thresholds("SICK")?, &mut level);
        }
        for (ty, stat, thresholds) in [("BORED", "BOREDOM", "BORED"), ("UNHAPPY", "UNHAPPINESS", "UNHAPPY")] {
            if self.is(ty) && alive {
                ladder(chr.stats.get(&cs.field(stat)?), ms.thresholds(thresholds)?, &mut level);
            }
        }
        if self.is("STRESS") {
            let s = chr.stats.nicotine_stress(cs)?;
            let t = ms.thresholds("STRESS")?;
            if s > t[4] {
                level = MAX;
            } else if s > t[3] {
                level = HIGH;
            } else if s > t[2] {
                level = MODERATE;
            } else if s > t[1] {
                level = LOW;
            }
        }
        if self.is("THIRST") {
            ladder(chr.stats.get(&cs.field("THIRST")?), ms.thresholds("THIRST")?, &mut level);
        }
        if self.is("BLEEDING") && alive {
            level = chr.body_damage.num_parts_bleeding();
            if chr.body_damage.is_neck_bleeding() {
                level = MAX;
            }
            if level > MAX {
                level = MAX;
            }
        }
        if self.is("WET") && alive {
            ladder(chr.stats.get(&cs.field("WETNESS")?), ms.thresholds("WET")?, &mut level);
        }
        if self.is("HAS_A_COLD") && alive {
            ladder(chr.body_damage.cold_strength(), ms.thresholds("HAS_A_COLD")?, &mut level);
        }
        if self.is("INJURED") && alive {
            ladder(100.0 - chr.body_damage.health(), ms.thresholds("INJURED")?, &mut level);
        }
        if self.is("PAIN") {
            self.pain_timer += 1;
            if self.pain_timer < Self::PAIN_TIMER_DURATION {
                return Ok(false);
            }
            self.pain_timer = 0;
            if chr.body_damage.health() != 0.0 {
                ladder(chr.stats.get(&cs.field("PAIN")?), ms.thresholds("PAIN")?, &mut level);
            }
            if level != self.level() && level >= HIGH && self.level() < HIGH && chr.is_local_player(ctx) {
                ctx.event(SimEvent::PlayerVoiceSound("PainMoodle"));
            }
        }
        if self.is("HEAVY_LOAD") {
            let ratio = chr.capacity_weight(sm, ctx) / chr.max_weight() as f32;
            if alive {
                let t = ms.thresholds("HEAVY_LOAD")?;
                if ratio >= t[4] {
                    level = MAX;
                } else if ratio >= t[3] {
                    level = HIGH;
                } else if ratio >= t[2] {
                    level = MODERATE;
                } else if ratio > t[1] {
                    level = LOW;
                }
            }
        }
        if self.is("DRUNK") && alive {
            ladder(chr.stats.get(&cs.field("INTOXICATION")?), ms.thresholds("DRUNK")?, &mut level);
        }
        let zombified = || -> ScriptResult<bool> {
            Ok(!chr.body_damage.is_fake_infected() && chr.stats.get(&cs.field("ZOMBIE_INFECTION")?) >= 0.001)
        };
        if self.is("DEAD") && chr.is_dead() {
            level = MAX;
            if zombified()? {
                level = MIN;
            }
        }
        if self.is("ZOMBIE") && chr.is_dead() && zombified()? {
            level = MAX;
        }
        if self.is("FOOD_EATEN") && alive {
            let timer = chr.body_damage.health_from_food_timer();
            let standard = chr.body_damage.standard_health_from_food_time() as f32;
            if timer > 0.0 {
                level = LOW;
            }
            if timer > standard {
                level = MODERATE;
            }
            if timer > standard * 2.0 {
                level = HIGH;
            }
            if timer > standard * 3.0 {
                level = MAX;
            }
        }
        let mut chev_count = self.chevron_count;
        let mut chev_is_up = self.chevron_is_up;
        let mut chev_col = self.chevron_color.clone();
        let temperature = chr.stats.get(&cs.field("TEMPERATURE")?);
        if (self.is("HYPERTHERMIA") || self.is("HYPOTHERMIA")) && chr.is_player() {
            if temperature < 36.5 || temperature > 37.5 {
                match chr.body_damage.thermoregulator() {
                    None => chev_count = 0,
                    Some(t) => {
                        chev_is_up = t.thermal_chevron_up();
                        chev_count = t.thermal_chevron_count();
                    }
                }
            } else {
                chev_count = 0;
            }
        }
        if self.is("HYPERTHERMIA") {
            if chev_count > 0 {
                chev_col = if chev_is_up { ChevronColor::Negative } else { ChevronColor::Positive };
            }
            if temperature != 0.0 {
                if temperature > 37.5 {
                    level = LOW;
                }
                if temperature > 39.0 {
                    level = MODERATE;
                }
                if temperature > 40.0 {
                    level = HIGH;
                }
                if temperature > 41.0 {
                    level = MAX;
                }
            }
            if level != self.level() || level > MIN && self.chevron_difference(chev_count, chev_is_up, &chev_col) {
                self.set_chevron(chev_count, chev_is_up, Some(chev_col.clone()));
            }
        }
        if self.is("HYPOTHERMIA") {
            if chev_count > 0 {
                chev_col = if chev_is_up { ChevronColor::Positive } else { ChevronColor::Negative };
            }
            if temperature != 0.0 {
                let intoxication = chr.stats.get(&cs.field("INTOXICATION")?);
                let t = ms.thresholds("HYPOTHERMIA")?;
                if temperature < 36.5 && intoxication <= t[1] {
                    level = LOW;
                }
                if temperature < 35.0 && intoxication <= t[2] {
                    level = MODERATE;
                }
                if temperature < 30.0 {
                    level = HIGH;
                }
                if temperature < 25.0 {
                    level = MAX;
                }
            }
            if level != self.level() || level > MIN && self.chevron_difference(chev_count, chev_is_up, &chev_col) {
                self.set_chevron(chev_count, chev_is_up, Some(chev_col));
            }
        }
        if self.is("WINDCHILL") && chr.is_player() {
            let w = chr.environment.wind_chill_amount_for_player(ctx);
            let t = ms.thresholds("WINDCHILL")?;
            // The original tests the highest threshold for the low level.
            if w > t[3] {
                level = LOW;
            }
            if w > t[2] {
                level = MODERATE;
            }
            if w > t[3] {
                level = HIGH;
            }
            if w > t[4] {
                level = MAX;
            }
        }
        if self.is("UNCOMFORTABLE") && alive {
            let d = chr.stats.get(&cs.field("DISCOMFORT")?);
            let t = ms.thresholds("UNCOMFORTABLE")?;
            if d >= t[4] {
                level = MAX;
            } else if d >= t[3] {
                level = HIGH;
            } else if d >= t[2] {
                level = MODERATE;
            } else if d >= t[1] {
                level = LOW;
            }
        }
        if self.is("NOXIOUS_SMELL") && alive {
            if chr.environment.building_toxic && !chr.items.is_protected_from_toxic(sm) {
                level = MAX;
            } else if ctx.decaying_corpse_health_impact() != 1 {
                let rate = chr.corpse_sickness_rate;
                if rate > 0.0 {
                    let t = ms.thresholds("NOXIOUS_SMELL")?;
                    level = if rate > t[3] {
                        HIGH
                    } else if rate > t[2] {
                        MODERATE
                    } else {
                        LOW
                    };
                }
            }
        }
        Ok(self.update_moodle_level(level))
    }
}

/// `Moodles`.
#[derive(Clone, Debug, Default)]
pub struct Moodles {
    moodles_state_changed: bool,
    /// One per registered `MoodleType`, in registration order (the
    /// original's identity-hashed map has no meaningful order, and no
    /// moodle's update depends on another's).
    moodles: Vec<Moodle>,
}

impl Moodles {
    /// `NeutralMoodleType`.
    pub const NEUTRAL_MOODLE_TYPE: i32 = 0;
    /// `GoodMoodleType`.
    pub const GOOD_MOODLE_TYPE: i32 = 1;
    /// `BadMoodleType`.
    pub const BAD_MOODLE_TYPE: i32 = 2;

    /// `new Moodles(chr)`: a moodle per `Registries.MOODLE_TYPE` value.
    pub fn new(sm: &ScriptManager) -> Self {
        let moodles = MoodleType::values(sm).into_iter().map(|t| Moodle::new(sm, t)).collect();
        Moodles { moodles_state_changed: false, moodles }
    }

    /// The moodle of `t`.
    pub fn moodle(&self, t: &RegistryKey) -> Option<&Moodle> {
        self.moodles.iter().find(|m| m.moodle_type.location == t.location)
    }

    /// Mutable [`Moodles::moodle`].
    pub fn moodle_mut(&mut self, t: &RegistryKey) -> Option<&mut Moodle> {
        self.moodles.iter_mut().find(|m| m.moodle_type.location == t.location)
    }

    /// All moodles.
    pub fn all(&self) -> &[Moodle] {
        &self.moodles
    }

    fn npe() -> pz_scripts::ScriptError {
        pz_scripts::ScriptError::new("java.lang.NullPointerException")
    }

    /// `getMoodleLevel`; an unknown type is the original's NPE.
    pub fn moodle_level(&self, t: Option<&RegistryKey>) -> ScriptResult<i32> {
        t.and_then(|t| self.moodle(t)).map(Moodle::level).ok_or_else(Self::npe)
    }

    /// `getMoodleLevel(MoodleType.<field>)`.
    pub fn moodle_level_of(&self, sm: &ScriptManager, field: &str) -> ScriptResult<i32> {
        self.moodle_level(MoodleType::constant(sm, field).as_deref())
    }

    /// `isMaxMoodleLevel`.
    pub fn is_max_moodle_level(&self, t: Option<&RegistryKey>) -> ScriptResult<bool> {
        Ok(self.moodle_level(t)? == MAX)
    }

    /// `getGoodBadNeutral`: good for `FOOD_EATEN`, else bad.
    pub fn good_bad_neutral(&self, sm: &ScriptManager, t: Option<&RegistryKey>) -> i32 {
        let food = MoodleType::constant(sm, "FOOD_EATEN");
        match (t, food) {
            (Some(t), Some(f)) if t.location == f.location => Self::GOOD_MOODLE_TYPE,
            _ => Self::BAD_MOODLE_TYPE,
        }
    }

    fn text(t: &RegistryKey, level: i32, desc: bool, get_text: &dyn Fn(&str) -> String) -> String {
        let level = if level > MAX {
            MAX
        } else if level == MIN {
            return "Invalid Moodle Level".to_string();
        } else {
            level
        };
        let mid = if desc { "_desc" } else { "" };
        get_text(&format!("Moodles_{}{mid}_lvl{level}", MoodleType::translation_name(t)))
    }

    /// `getMoodleDisplayString`.
    pub fn moodle_display_string(
        &self,
        t: Option<&RegistryKey>,
        get_text: &dyn Fn(&str) -> String,
    ) -> ScriptResult<String> {
        let m = t.and_then(|t| self.moodle(t)).ok_or_else(Self::npe)?;
        Ok(Self::text(t.unwrap(), m.level(), false, get_text))
    }

    /// `getMoodleDescriptionString`.
    pub fn moodle_description_string(
        &self,
        t: Option<&RegistryKey>,
        get_text: &dyn Fn(&str) -> String,
    ) -> ScriptResult<String> {
        let m = t.and_then(|t| self.moodle(t)).ok_or_else(Self::npe)?;
        Ok(Self::text(&m.moodle_type, m.level(), true, get_text))
    }

    /// `UI_RefreshNeeded`: the changed flag, cleared.
    pub fn ui_refresh_needed(&mut self) -> bool {
        std::mem::take(&mut self.moodles_state_changed)
    }

    /// `setMoodlesStateChanged`.
    pub fn set_moodles_state_changed(&mut self, b: bool) {
        self.moodles_state_changed = b;
    }

    /// `getNumMoodles`-style count (`moodles.size()`).
    pub fn len(&self) -> usize {
        self.moodles.len()
    }

    pub fn is_empty(&self) -> bool {
        self.moodles.is_empty()
    }
}

impl<I: 'static> GameCharacter<I> {
    /// `getMoodles().Update()`.
    pub fn update_moodles(&mut self, sm: &ScriptManager, chars: &Characters, ctx: &dyn SimContext) -> ScriptResult {
        let mut moodles = std::mem::take(&mut self.moodles.moodles);
        let mut result = Ok(());
        for m in &mut moodles {
            match m.update(self, sm, chars, ctx) {
                Ok(true) => self.moodles.moodles_state_changed = true,
                Ok(false) => {}
                Err(e) => {
                    result = Err(e);
                    break;
                }
            }
        }
        self.moodles.moodles = moodles;
        result
    }
}
