//! `zombie.characters.CharacterStat` and `zombie.characters.Stats`.
//!
//! The built-in stats (`CharacterStat.ANGER`, ...) and `ORDERED_STATS` are
//! replayed from the jar ([`BuiltinTables::character_stats`]); code names
//! them by their Java field through [`CharacterStats::field`].

use std::rc::Rc;

use pz_scripts::registry::builtin::BuiltinTables;
use pz_scripts::{ScriptError, ScriptResult};

use crate::survivor::java_float_string;

/// `CharacterStat`.
#[derive(Debug, PartialEq)]
pub struct CharacterStat {
    id: String,
    minimum_value: f32,
    maximum_value: f32,
    default_value: f32,
}

pub type CharacterStatRef = Rc<CharacterStat>;

/// `PZMath.clamp(float, float, float)`.
pub fn clamp_f32(val: f32, min: f32, max: f32) -> f32 {
    if val < min {
        return min;
    }
    if val > max {
        return max;
    }
    val
}

impl CharacterStat {
    /// `getId`.
    pub fn id(&self) -> &str {
        &self.id
    }
    /// `getMinimumValue`.
    pub fn minimum_value(&self) -> f32 {
        self.minimum_value
    }
    /// `getMaximumValue`.
    pub fn maximum_value(&self) -> f32 {
        self.maximum_value
    }
    /// `getDefaultValue`.
    pub fn default_value(&self) -> f32 {
        self.default_value
    }
    /// `clamp`.
    pub fn clamp(&self, value: f32) -> f32 {
        clamp_f32(value, self.minimum_value, self.maximum_value)
    }
    /// `isAtMinimum`.
    pub fn is_at_minimum(&self, value: f32) -> bool {
        value <= self.minimum_value
    }
    /// `isAtMaximum`.
    pub fn is_at_maximum(&self, value: f32) -> bool {
        value >= self.maximum_value
    }
    /// `toString`.
    pub fn to_java_string(&self) -> String {
        format!(
            "CharacterStat{{id='{}', min={}, max={}, default={}}}",
            self.id,
            java_float_string(self.minimum_value),
            java_float_string(self.maximum_value),
            java_float_string(self.default_value)
        )
    }
}

/// `CharacterStat`'s statics: `REGISTRY`, the public constants and
/// `ORDERED_STATS`.
#[derive(Clone, Debug, Default)]
pub struct CharacterStats {
    /// `REGISTRY` values, in registration order.
    registry: Vec<CharacterStatRef>,
    /// The static fields (`ANGER`, ...) in declaration order.
    fields: Vec<(String, CharacterStatRef)>,
    /// `ORDERED_STATS`.
    pub ordered: Vec<CharacterStatRef>,
}

impl CharacterStats {
    /// Replays the static initializer described by `tables`.
    pub fn from_tables(tables: &BuiltinTables) -> ScriptResult<Self> {
        let mut s = CharacterStats::default();
        for d in &tables.character_stats {
            let stat = s.register(&d.id, d.minimum, d.maximum, d.default);
            s.fields.push((d.field.clone(), stat));
        }
        for f in &tables.ordered_stats {
            let stat = s.constant(f).ok_or_else(|| ScriptError::new(format!("CharacterStat.ORDERED_STATS: {f}")))?;
            s.ordered.push(stat);
        }
        Ok(s)
    }

    /// `register`: `computeIfAbsent` on the id.
    pub fn register(&mut self, id: &str, minimum: f32, maximum: f32, default: f32) -> CharacterStatRef {
        if let Some(s) = self.get_by_id(id) {
            return s;
        }
        let s = Rc::new(CharacterStat {
            id: id.to_string(),
            minimum_value: minimum,
            maximum_value: maximum,
            default_value: default,
        });
        self.registry.push(s.clone());
        s
    }

    /// `getById`.
    pub fn get_by_id(&self, id: &str) -> Option<CharacterStatRef> {
        self.registry.iter().find(|s| s.id == id).cloned()
    }

    /// `REGISTRY.values()`.
    pub fn values(&self) -> &[CharacterStatRef] {
        &self.registry
    }

    /// The static field `field` (`CharacterStat.STRESS`, ...).
    pub fn constant(&self, field: &str) -> Option<CharacterStatRef> {
        self.fields.iter().find(|(f, _)| f == field).map(|(_, s)| s.clone())
    }

    /// The public static fields, in declaration order.
    pub fn constants(&self) -> &[(String, CharacterStatRef)] {
        &self.fields
    }

    /// [`CharacterStats::constant`] for code paths that dereference it (a
    /// missing one is the `NullPointerException` the original would throw).
    pub fn field(&self, field: &str) -> ScriptResult<CharacterStatRef> {
        self.constant(field)
            .ok_or_else(|| ScriptError::new(format!("java.lang.NullPointerException: CharacterStat.{field}")))
    }
}

/// `Stats`.
#[derive(Clone, Debug)]
pub struct Stats {
    /// `stats`: values that were set, in first-set order.
    values: Vec<(CharacterStatRef, f32)>,
    last_endurance: f32,
    pub num_visible_zombies: i32,
    pub last_num_visible_zombies: i32,
    tripping: bool,
    tripping_rot_angle: f32,
    pub num_chasing_zombies: i32,
    pub last_very_close_zombies: i32,
    last_num_chasing_zombies: i32,
    pub music_zombies_visible: i32,
    pub music_zombies_targeting_distant_not_moving: i32,
    pub music_zombies_targeting_nearby_not_moving: i32,
    pub music_zombies_targeting_distant_moving: i32,
    pub music_zombies_targeting_nearby_moving: i32,
}

impl Default for Stats {
    fn default() -> Self {
        Stats {
            values: Vec::new(),
            last_endurance: 1.0,
            num_visible_zombies: 0,
            last_num_visible_zombies: 0,
            tripping: false,
            tripping_rot_angle: 0.0,
            num_chasing_zombies: 0,
            last_very_close_zombies: 0,
            last_num_chasing_zombies: 0,
            music_zombies_visible: 0,
            music_zombies_targeting_distant_not_moving: 0,
            music_zombies_targeting_nearby_not_moving: 0,
            music_zombies_targeting_distant_moving: 0,
            music_zombies_targeting_nearby_moving: 0,
        }
    }
}

impl Stats {
    /// `ENDURANCE_DANGER_WARNING`.
    pub const ENDURANCE_DANGER_WARNING: f32 = 0.25;
    /// `ENDURANCE_WARNING`.
    pub const ENDURANCE_WARNING: f32 = 0.5;

    /// `get`: the stored value, else the stat's default.
    pub fn get(&self, stat: &CharacterStatRef) -> f32 {
        self.values.iter().find(|(s, _)| Rc::ptr_eq(s, stat)).map_or(stat.default_value, |(_, v)| *v)
    }

    /// `set`: stores the clamped value; whether it changed.
    pub fn set(&mut self, stat: &CharacterStatRef, value: f32) -> bool {
        let old = self.get(stat);
        let new = stat.clamp(value);
        match self.values.iter_mut().find(|(s, _)| Rc::ptr_eq(s, stat)) {
            Some(e) => e.1 = new,
            None => self.values.push((stat.clone(), new)),
        }
        new != old
    }

    /// `add`.
    pub fn add(&mut self, stat: &CharacterStatRef, amount: f32) -> bool {
        let v = self.get(stat) + amount;
        self.set(stat, v)
    }

    /// `remove`.
    pub fn remove(&mut self, stat: &CharacterStatRef, amount: f32) -> bool {
        self.add(stat, -amount)
    }

    /// `reset`.
    pub fn reset(&mut self, stat: &CharacterStatRef) -> bool {
        self.set(stat, stat.default_value)
    }

    /// `isAtMinimum`.
    pub fn is_at_minimum(&self, stat: &CharacterStatRef) -> bool {
        stat.is_at_minimum(self.get(stat))
    }

    /// `isAtMaximum`.
    pub fn is_at_maximum(&self, stat: &CharacterStatRef) -> bool {
        stat.is_at_maximum(self.get(stat))
    }

    /// `isAboveMinimum`.
    pub fn is_above_minimum(&self, stat: &CharacterStatRef) -> bool {
        !self.is_at_minimum(stat)
    }

    /// `resetStats`: every registered stat.
    pub fn reset_stats(&mut self, stats: &CharacterStats) {
        for s in stats.values() {
            self.reset(s);
        }
    }

    /// `getNicotineStress`: stress plus nicotine withdrawal, clamped to the
    /// stress range.
    pub fn nicotine_stress(&self, stats: &CharacterStats) -> ScriptResult<f32> {
        let stress = stats.field("STRESS")?;
        let v = self.get(&stress) + self.get(&stats.field("NICOTINE_WITHDRAWAL")?);
        Ok(stress.clamp(v))
    }

    /// `toString`. The original iterates an identity-hashed map (arbitrary
    /// order); this lists the stored values in first-set order.
    pub fn to_java_string(&self) -> String {
        let mut s = String::from("Stats{\n");
        for (k, v) in &self.values {
            s.push_str(&format!("  {} = {}\n", k.id, java_float_string(*v)));
        }
        s.push('}');
        s
    }

    /// `save`: every `ORDERED_STATS` value as a big-endian float.
    pub fn save(&self, stats: &CharacterStats, out: &mut Vec<u8>) {
        for s in &stats.ordered {
            out.extend_from_slice(&self.get(s).to_be_bytes());
        }
    }

    /// `load`: the inverse of [`Stats::save`]; returns the bytes read.
    pub fn load(&mut self, stats: &CharacterStats, input: &[u8]) -> ScriptResult<usize> {
        let mut pos = 0;
        for s in &stats.ordered {
            let b = input.get(pos..pos + 4).ok_or_else(|| ScriptError::new("java.io.EOFException"))?;
            self.set(s, f32::from_be_bytes([b[0], b[1], b[2], b[3]]));
            pos += 4;
        }
        Ok(pos)
    }

    /// `parse`: sets `ORDERED_STATS[field]`; false (a logged warning in the
    /// original) for an index out of range.
    pub fn parse(&mut self, stats: &CharacterStats, field: i8, value: f32) -> bool {
        match usize::try_from(field).ok().and_then(|i| stats.ordered.get(i)) {
            Some(s) => {
                self.set(s, value);
                true
            }
            None => false,
        }
    }

    /// `write`: `ORDERED_STATS[field]`'s value, `None` out of range.
    pub fn write(&self, stats: &CharacterStats, field: i8) -> Option<f32> {
        usize::try_from(field).ok().and_then(|i| stats.ordered.get(i)).map(|s| self.get(s))
    }

    /// `getNumVisibleZombies` / `getVisibleZombies`.
    pub fn num_visible_zombies(&self) -> i32 {
        self.num_visible_zombies
    }
    /// `setNumVisibleZombies`.
    pub fn set_num_visible_zombies(&mut self, n: i32) {
        self.num_visible_zombies = n;
    }
    /// `getNumChasingZombies`: the last count.
    #[allow(clippy::misnamed_getters)]
    pub fn num_chasing_zombies(&self) -> i32 {
        self.last_num_chasing_zombies
    }
    /// `setLastNumberChasingZombies`.
    pub fn set_last_number_chasing_zombies(&mut self, n: i32) {
        self.last_num_chasing_zombies = n;
    }
    /// `getNumVeryCloseZombies`.
    pub fn num_very_close_zombies(&self) -> i32 {
        self.last_very_close_zombies
    }
    /// `getLastEndurance`.
    pub fn last_endurance(&self) -> f32 {
        self.last_endurance
    }
    /// `setLastEndurance`.
    pub fn set_last_endurance(&mut self, e: f32) {
        self.last_endurance = e;
    }
    /// `isEnduranceRecharging`: always false.
    pub fn is_endurance_recharging(&self) -> bool {
        false
    }
    /// `isTripping`.
    pub fn is_tripping(&self) -> bool {
        self.tripping
    }
    /// `setTripping`.
    pub fn set_tripping(&mut self, t: bool) {
        self.tripping = t;
    }
    /// `getTrippingRotAngle`.
    pub fn tripping_rot_angle(&self) -> f32 {
        self.tripping_rot_angle
    }
    /// `setTrippingRotAngle`.
    pub fn set_tripping_rot_angle(&mut self, a: f32) {
        self.tripping_rot_angle = a;
    }
    /// `addTrippingRotAngle`.
    pub fn add_tripping_rot_angle(&mut self, a: f32) {
        self.tripping_rot_angle += a;
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use pz_scripts::registry::builtin::CharacterStatDecl;

    /// Invented stats: a 0..1 one defaulting to 1 and two 0..10 ones.
    pub(crate) fn tables() -> BuiltinTables {
        let d = |field: &str, id: &str, minimum, maximum, default| CharacterStatDecl {
            field: field.into(),
            id: id.into(),
            minimum,
            maximum,
            default,
        };
        BuiltinTables {
            character_stats: vec![
                d("ALPHA", "Alpha", 0.0, 1.0, 1.0),
                d("STRESS", "Stress", 0.0, 1.0, 0.0),
                d("NICOTINE_WITHDRAWAL", "Nic", 0.0, 0.5, 0.0),
                d("BETA", "Beta", -1.0, 10.0, 0.0),
            ],
            ordered_stats: vec!["BETA".into(), "ALPHA".into()],
            ..Default::default()
        }
    }

    #[test]
    fn stats_clamp_default_and_save() {
        let mut defs = CharacterStats::from_tables(&tables()).unwrap();
        let alpha = defs.field("ALPHA").unwrap();
        let beta = defs.field("BETA").unwrap();
        assert!(Rc::ptr_eq(&defs.register("Alpha", 5.0, 6.0, 7.0), &alpha));
        assert_eq!(alpha.to_java_string(), "CharacterStat{id='Alpha', min=0.0, max=1.0, default=1.0}");
        let mut s = Stats::default();
        assert_eq!(s.get(&alpha), 1.0);
        assert!(!s.set(&alpha, 3.0));
        assert!(s.is_at_maximum(&alpha));
        assert!(s.remove(&alpha, 0.25));
        assert_eq!(s.get(&alpha), 0.75);
        assert!(s.add(&beta, -5.0));
        assert!(s.is_at_minimum(&beta) && !s.is_above_minimum(&beta));
        s.set(&defs.field("STRESS").unwrap(), 0.9);
        s.set(&defs.field("NICOTINE_WITHDRAWAL").unwrap(), 0.4);
        assert_eq!(s.nicotine_stress(&defs).unwrap(), 1.0);
        let mut out = Vec::new();
        s.save(&defs, &mut out);
        assert_eq!(out, [(-1.0f32).to_be_bytes(), 0.75f32.to_be_bytes()].concat());
        let mut t = Stats::default();
        assert_eq!(t.load(&defs, &out).unwrap(), 8);
        assert_eq!((t.get(&alpha), t.get(&beta)), (0.75, -1.0));
        assert!(t.parse(&defs, 1, 0.5) && !t.parse(&defs, 2, 0.5));
        assert_eq!(t.write(&defs, 1), Some(0.5));
        s.reset_stats(&defs);
        assert_eq!((s.get(&alpha), s.get(&beta)), (1.0, 0.0));
        assert!(s.to_java_string().starts_with("Stats{\n  Alpha = 1.0\n"));
        assert!(defs.field("NOPE").is_err());
    }
}
