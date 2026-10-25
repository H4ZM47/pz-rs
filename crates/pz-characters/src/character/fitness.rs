//! `zombie.characters.BodyDamage.Fitness` (players only, owned by
//! [`super::GameCharacter::fitness`]): exercise regularity, the stiffness
//! an exercise builds up and later turns into body-part stiffness, and the
//! Strength/Fitness XP it awards through
//! [`super::GameCharacter::add_xp`].
//!
//! The exercises are Lua data (`FitnessExercises.exercisesType`): the
//! binding reads them and hands them to [`GameCharacter::fitness_init`] as
//! [`FitnessExercise`]s. The maps are Java `HashMap<String, ...>`s, so
//! their iteration order (which decides the order stiffness is applied and
//! random regularities are drawn) is the JDK's.
//!
//! TODO(network): the server's `sendSyncPlayerFields`/`SyncPlayerStats`
//! packets and `GameServer.addXp` (on a server the XP is not awarded here).
//! TODO(saves): `save`/`load`.

use pz_scripts::java::{split_char, JHashMap};
use pz_scripts::objects::items::CharacterProfession;
use pz_scripts::{ScriptError, ScriptManager, ScriptResult};

use super::context::SimContext;
use super::stats::clamp_f32;
use super::GameCharacter;
use crate::Characters;

fn npe(what: &str) -> ScriptError {
    ScriptError::new(format!("java.lang.NullPointerException: {what}"))
}

/// `Fitness.FitnessExercise`.
#[derive(Clone, Debug, PartialEq)]
pub struct FitnessExercise {
    /// `type` (the Lua entry's `type`; `None` when nil).
    pub exercise_type: Option<String>,
    /// `metabolics`: the `Metabolics` constant's name (`None` when nil).
    pub metabolics: Option<String>,
    /// `stiffnessInc`: `stiffness.split(",")`.
    pub stiffness_inc: Vec<String>,
    /// `xpModifier`.
    pub xp_modifier: f32,
}

impl FitnessExercise {
    /// The constructor from the Lua entry's `type`, `metabolics`,
    /// `stiffness` and `rawgetFloat("xpMod")` (-1 when not a number).
    pub fn new(exercise_type: Option<String>, metabolics: Option<String>, stiffness: &str, xp_mod: f32) -> Self {
        FitnessExercise {
            exercise_type,
            metabolics,
            stiffness_inc: split_char(stiffness, ',').into_iter().map(str::to_string).collect(),
            xp_modifier: if xp_mod > 0.0 { xp_mod } else { 1.0 },
        }
    }

    /// The key the maps use for this exercise (`type`; a nil type is
    /// stored as the empty string).
    fn key(&self) -> String {
        self.exercise_type.clone().unwrap_or_default()
    }

    fn is_heavy(&self) -> bool {
        self.metabolics.as_deref() == Some("FitnessHeavy")
    }
}

/// `Fitness`.
#[derive(Clone, Debug)]
pub struct Fitness {
    regularity_map: JHashMap<f32>,
    fitness_lvl: i32,
    str_lvl: i32,
    stiffness_timer_map: JHashMap<i32>,
    stiffness_inc_map: JHashMap<f32>,
    bodypart_to_inc_stiffness: Vec<String>,
    exercises: JHashMap<FitnessExercise>,
    exe_timer: JHashMap<i64>,
    last_update: i32,
    current_exe: Option<FitnessExercise>,
}

impl Default for Fitness {
    fn default() -> Self {
        Fitness {
            regularity_map: JHashMap::new(),
            fitness_lvl: 0,
            str_lvl: 0,
            stiffness_timer_map: JHashMap::new(),
            stiffness_inc_map: JHashMap::new(),
            bodypart_to_inc_stiffness: Vec::new(),
            exercises: JHashMap::new(),
            exe_timer: JHashMap::new(),
            last_update: -1,
            current_exe: None,
        }
    }
}

/// `type.split(",")[0]`.
fn first_of(ty: &str) -> ScriptResult<&str> {
    split_char(ty, ',')
        .first()
        .copied()
        .ok_or_else(|| ScriptError::new("java.lang.ArrayIndexOutOfBoundsException: Index 0 out of bounds for length 0"))
}

impl Fitness {
    /// `setCurrentExercise` (`None` for nil).
    pub fn set_current_exercise(&mut self, ty: Option<&str>) {
        self.current_exe = ty.and_then(|t| self.exercises.get(t)).cloned();
    }

    /// `getCurrentExe`.
    pub fn current_exe(&self) -> Option<&FitnessExercise> {
        self.current_exe.as_ref()
    }

    /// The exercises read by `init`.
    pub fn exercises(&self) -> &JHashMap<FitnessExercise> {
        &self.exercises
    }

    /// `resetValues`.
    pub fn reset_values(&mut self) {
        self.stiffness_inc_map.clear();
        self.stiffness_timer_map.clear();
        self.regularity_map.clear();
    }

    /// `removeStiffnessValue`.
    pub fn remove_stiffness_value(&mut self, ty: &str) {
        self.stiffness_inc_map.remove(ty);
        self.stiffness_timer_map.remove(ty);
    }

    /// `onGoingStiffness`.
    pub fn on_going_stiffness(&self) -> bool {
        !self.bodypart_to_inc_stiffness.is_empty()
    }

    /// `getCurrentExeStiffnessTimer`.
    pub fn current_exe_stiffness_timer(&self, ty: &str) -> ScriptResult<i32> {
        Ok(self.stiffness_timer_map.get(first_of(ty)?).copied().unwrap_or(0))
    }

    /// `getCurrentExeStiffnessInc`.
    pub fn current_exe_stiffness_inc(&self, ty: &str) -> ScriptResult<f32> {
        Ok(self.stiffness_inc_map.get(first_of(ty)?).copied().unwrap_or(0.0))
    }

    /// `getRegularity`.
    pub fn regularity(&self, ty: &str) -> f32 {
        self.regularity_map.get(ty).copied().unwrap_or(0.0)
    }

    /// `getRegularityMap`.
    pub fn regularity_map(&self) -> &JHashMap<f32> {
        &self.regularity_map
    }

    /// `setRegularityMap`.
    pub fn set_regularity_map(&mut self, map: JHashMap<f32>) {
        self.regularity_map = map;
    }

    /// `stiffnessTimerMap`.
    pub fn stiffness_timer_map(&self) -> &JHashMap<i32> {
        &self.stiffness_timer_map
    }

    /// `stiffnessIncMap`.
    pub fn stiffness_inc_map(&self) -> &JHashMap<f32> {
        &self.stiffness_inc_map
    }

    /// `bodypartToIncStiffness`.
    pub fn bodypart_to_inc_stiffness(&self) -> &[String] {
        &self.bodypart_to_inc_stiffness
    }

    /// `exeTimer`.
    pub fn exe_timer(&self) -> &JHashMap<i64> {
        &self.exe_timer
    }

    fn current(&self) -> ScriptResult<&FitnessExercise> {
        self.current_exe.as_ref().ok_or_else(|| npe("Fitness.currentExe"))
    }

    /// `decreaseRegularity`.
    fn decrease_regularity(&mut self, ctx: &dyn SimContext) {
        let keys: Vec<String> = self.regularity_map.keys().map(str::to_string).collect();
        for k in keys {
            let Some(&t) = self.exe_timer.get(&k) else {
                continue;
            };
            if ctx.calendar_time_in_millis().wrapping_sub(t) <= 86_400_000 {
                continue;
            }
            let v = self.regularity_map.get(&k).copied().unwrap_or(0.0) - 0.002;
            self.regularity_map.insert(k, v);
        }
    }

    /// `incRegularity`.
    pub fn inc_regularity(&mut self) -> ScriptResult {
        let key = self.current()?.key();
        let inc_log = f64::from(self.fitness_lvl as f32 / 5.0 + 4.0).ln();
        let base_inc = (f64::from(0.08f32) * (5.0f64.ln() / inc_log)) as f32;
        let mut result = self.regularity_map.get(&key).copied().unwrap_or(0.0);
        result += base_inc;
        result = clamp_f32(result, 0.0, 100.0);
        self.regularity_map.insert(key, result);
        Ok(())
    }

    /// Adds one exercise as `init` does (`exercises.put`).
    pub fn put_exercise(&mut self, key: String, e: FitnessExercise) {
        self.exercises.insert(key, e);
    }

    /// `incFutureStiffness`, given the owner's tired moodle level.
    pub fn inc_future_stiffness(&mut self, tired: i32) -> ScriptResult {
        let exe = self.current()?.clone();
        let reg = self.regularity_map.get(&exe.key()).copied().unwrap_or(0.0);
        for body_part in &exe.stiffness_inc {
            let mut base_red = 0.5f32;
            if !self.stiffness_timer_map.contains_key(body_part) && !self.bodypart_to_inc_stiffness.contains(body_part)
            {
                self.stiffness_timer_map.insert(body_part.clone(), 72);
            }
            let mut inc = self.stiffness_inc_map.get(body_part).copied().unwrap_or(0.0);
            base_red *= (120.0 - reg) / 170.0;
            if exe.is_heavy() {
                base_red *= 1.3;
            }
            base_red *= (1 + tired / 3) as f32;
            inc += base_red;
            inc = inc.min(150.0);
            self.stiffness_inc_map.insert(body_part.clone(), inc);
        }
        Ok(())
    }

    /// `incStats`: the Strength and Fitness XP of one repetition.
    fn exercise_xp(&self) -> ScriptResult<(f32, f32)> {
        let exe = self.current()?;
        let mut str_xp = 0.0f32;
        let mut fit = 0.0f32;
        for part in &exe.stiffness_inc {
            match part.as_str() {
                "arms" => str_xp += 4.0,
                "chest" => str_xp += 2.0,
                "legs" => fit += 4.0,
                "abs" => fit += 2.0,
                _ => {}
            }
        }
        if self.str_lvl > 5 {
            str_xp *= (1 + (self.str_lvl - 5) / 10) as f32;
        }
        if self.fitness_lvl > 5 {
            fit *= (1 + (self.fitness_lvl - 5) / 10) as f32;
        }
        str_xp *= exe.xp_modifier;
        fit *= exe.xp_modifier;
        Ok((str_xp, fit))
    }
}

impl<I: 'static> GameCharacter<I> {
    fn fitness_ref(&self) -> ScriptResult<&Fitness> {
        self.fitness.as_ref().ok_or_else(|| npe("getFitness()"))
    }

    fn fitness_mut(&mut self) -> ScriptResult<&mut Fitness> {
        self.fitness.as_mut().ok_or_else(|| npe("getFitness()"))
    }

    /// `Fitness.init`: the exercises from `FitnessExercises.exercisesType`
    /// (`None` when the Lua tables are missing), once; then
    /// `initRegularityMapProfession`.
    pub fn fitness_init(
        &mut self,
        sm: &ScriptManager,
        exercises: Option<Vec<(String, FitnessExercise)>>,
        ctx: &dyn SimContext,
    ) -> ScriptResult {
        let f = self.fitness_mut()?;
        if !f.exercises.is_empty() {
            return Ok(());
        }
        let Some(exercises) = exercises else {
            return Ok(());
        };
        for (k, e) in exercises {
            f.exercises.insert(k, e);
        }
        self.init_regularity_map_profession(sm, ctx)
    }

    /// `Fitness.initRegularityMapProfession`: fire officers, fitness
    /// instructors and security guards start with random regularities.
    pub fn init_regularity_map_profession(&mut self, sm: &ScriptManager, ctx: &dyn SimContext) -> ScriptResult {
        if !self.fitness_ref()?.regularity_map.is_empty() {
            return Ok(());
        }
        let d = self.descriptor.clone().ok_or_else(|| npe("getDescriptor()"))?;
        let is = |f: &str| d.borrow().is_character_profession(CharacterProfession::constant(sm, f).as_ref());
        let fitnessinstructor = is("FITNESS_INSTRUCTOR");
        let fireman = is("FIRE_OFFICER");
        let securityguard = is("SECURITY_GUARD");
        if !(fireman || fitnessinstructor || securityguard) {
            return Ok(());
        }
        let f = self.fitness_mut()?;
        let keys: Vec<String> = f.exercises.keys().map(str::to_string).collect();
        for k in keys {
            let mut reg = ctx.rand_next_range(7, 12) as f32;
            if fireman {
                reg = ctx.rand_next_range(10, 20) as f32;
            } else if fitnessinstructor {
                reg = ctx.rand_next_range(40, 60) as f32;
            }
            f.regularity_map.insert(k, reg);
        }
        Ok(())
    }

    /// `Fitness.update`: every ten game minutes, the regularity decay and
    /// the stiffness timers.
    pub fn fitness_update(&mut self, sm: &ScriptManager, ctx: &dyn SimContext) -> ScriptResult {
        let current_min = ctx.minutes() / 10;
        let f = self.fitness_mut()?;
        if f.last_update == -1 {
            f.last_update = current_min;
        }
        if current_min == f.last_update {
            return Ok(());
        }
        f.last_update = current_min;
        let mut toremove = Vec::new();
        f.decrease_regularity(ctx);
        let keys: Vec<String> = f.stiffness_timer_map.keys().map(str::to_string).collect();
        for k in keys {
            let timer = f.stiffness_timer_map.get(&k).copied().unwrap_or(0) - 1;
            if timer <= 0 {
                toremove.push(k.clone());
                f.bodypart_to_inc_stiffness.push(k);
                continue;
            }
            f.stiffness_timer_map.insert(k, timer);
        }
        for k in &toremove {
            f.stiffness_timer_map.remove(k);
        }
        let mut i = 0usize;
        loop {
            let f = self.fitness_mut()?;
            if i >= f.bodypart_to_inc_stiffness.len() {
                break;
            }
            let ty = f.bodypart_to_inc_stiffness[i].clone();
            let Some(left) = f.stiffness_inc_map.get(&ty).copied() else {
                return Ok(());
            };
            let left = left - 1.0;
            self.increase_pain(sm, &ty);
            let f = self.fitness_mut()?;
            if left <= 0.0 {
                f.bodypart_to_inc_stiffness.remove(i);
                f.stiffness_inc_map.remove(&ty);
                continue;
            }
            f.stiffness_inc_map.insert(ty, left);
            i += 1;
        }
        Ok(())
    }

    /// `Fitness.increasePain`: 2.5 stiffness on the body parts of `ty`.
    fn increase_pain(&mut self, sm: &ScriptManager, ty: &str) {
        let index = |f: &str| sm.builtin_tables.borrow().body_parts.iter().position(|p| p.field == f);
        let range = |from: &str, to: &str| -> Vec<usize> {
            match (index(from), index(to)) {
                (Some(a), Some(b)) => (a..b + 1).collect(),
                _ => Vec::new(),
            }
        };
        let parts = match ty {
            "arms" => range("ForeArm_L", "UpperArm_R"),
            "legs" => range("UpperLeg_L", "LowerLeg_R"),
            "chest" => index("Torso_Upper").into_iter().collect(),
            "abs" => index("Torso_Lower").into_iter().collect(),
            _ => Vec::new(),
        };
        for i in parts {
            let v = self.body_damage.body_part_stiffness(i) + 2.5;
            self.body_damage.set_body_part_stiffness(i, v);
        }
    }

    /// `Fitness.exerciseRepeat`: one repetition of the current exercise.
    pub fn fitness_exercise_repeat(
        &mut self,
        sm: &ScriptManager,
        chars: &Characters,
        ctx: &dyn SimContext,
    ) -> ScriptResult {
        let fitness_lvl = self.perk_level(chars.perks.constant("Fitness").as_ref());
        let str_lvl = self.perk_level(chars.perks.constant("Strength").as_ref());
        {
            let f = self.fitness_mut()?;
            f.fitness_lvl = fitness_lvl;
            f.str_lvl = str_lvl;
            f.inc_regularity()?;
        }
        self.fitness_reduce_endurance(sm, chars, ctx)?;
        let tired = self.moodles.moodle_level_of(sm, "TIRED")?;
        self.fitness_mut()?.inc_future_stiffness(tired)?;
        self.fitness_inc_stats(sm, chars, ctx)?;
        // updateExeTimer
        let f = self.fitness_mut()?;
        let key = f.current()?.key();
        f.exe_timer.insert(key, ctx.calendar_time_in_millis());
        Ok(())
    }

    /// `Fitness.reduceEndurance`.
    pub fn fitness_reduce_endurance(
        &mut self,
        sm: &ScriptManager,
        chars: &Characters,
        ctx: &dyn SimContext,
    ) -> ScriptResult {
        let f = self.fitness_ref()?;
        let exe = f.current()?;
        let reg = f.regularity_map.get(&exe.key()).copied().unwrap_or(0.0);
        let inc_log = f64::from(reg / 50.0 + 50.0).ln();
        let mut base_red = (f64::from(0.015f32) * (inc_log / 51.0f64.ln())) as f32;
        if exe.is_heavy() {
            base_red *= 1.3;
        }
        base_red *= (1 + self.moodles.moodle_level_of(sm, "HEAVY_LOAD")? / 3) as f32;
        if !ctx.is_client() {
            let e = chars.sim.character_stats.field("ENDURANCE")?;
            self.stats.remove(&e, base_red);
        }
        Ok(())
    }

    /// `Fitness.incStats`.
    pub fn fitness_inc_stats(&mut self, sm: &ScriptManager, chars: &Characters, ctx: &dyn SimContext) -> ScriptResult {
        let (str_xp, fit) = self.fitness_ref()?.exercise_xp()?;
        if ctx.is_server() {
            // TODO(network): GameServer.addXp(player, Strength/Fitness, (int) xp).
            return Ok(());
        }
        if !ctx.is_client() {
            self.add_xp(sm, chars, chars.perks.constant("Strength").as_ref(), str_xp, ctx)?;
            self.add_xp(sm, chars, chars.perks.constant("Fitness").as_ref(), fit, ctx)?;
        }
        Ok(())
    }

    /// The `IsoPlayer.updateInternal2` step that runs the body systems:
    /// `Nutrition.update` (while `SystemDisabler.doCharacterStats`) and
    /// `Fitness.update`, for players that are not animals.
    pub fn update_nutrition_and_fitness(
        &mut self,
        sm: &ScriptManager,
        chars: &Characters,
        ctx: &dyn SimContext,
    ) -> ScriptResult {
        if !self.is_player() || self.is_animal() {
            return Ok(());
        }
        if ctx.do_character_stats() {
            self.nutrition_update(sm, chars, ctx)?;
        }
        self.fitness_update(sm, ctx)
    }
}
