//! `zombie.characters.animals.datas.AnimalData`: an animal's growth, milk,
//! wool, eggs, pregnancy, hunger and thirst, health and feeding. The
//! fields live in [`AnimalData`]; the methods are on [`Animal`] (the
//! original's `parent`), named `data_*` where `IsoAnimal` has a method of
//! the same name.

use crate::animal::{java_round, next_bool, Animal, Male, NewAnimal};
use crate::breed::{AnimalBreed, AnimalGrowStage};
use crate::definitions::AnimalDefinitions;
use crate::env::{AnimalCharacter, AnimalCtx, AnimalEnv, AnimalStat, FeedItemKind, ObjectRef};
use crate::genome::AnimalGene;
use crate::java::{clamp, fmax, fmin, is_null_or_empty, npe, JHashMap, JResult, JavaException};
use pz_world::GameRand;

/// `AnimalData.ONE_DAY_MILLISECONDS`.
pub const ONE_DAY_MILLISECONDS: i64 = 86_400_000;
/// `AnimalData.TIME_TO_LOSE_MILK`.
const TIME_TO_LOSE_MILK: i64 = 1_814_400_000;

/// The fields of `AnimalData`.
#[derive(Clone, Debug, PartialEq)]
pub struct AnimalData {
    /// `attachedPlayer`.
    pub attached_player: Option<ObjectRef>,
    pub attached_tree: Option<ObjectRef>,
    pub attached_tree_x: i32,
    pub attached_tree_y: i32,
    pub breed: Option<AnimalBreed>,
    pub milk_qty: f32,
    pub wool_qty: f32,
    pub can_have_milk: bool,
    pub weight: f32,
    pub size: f32,
    pub original_size: f32,
    pub age: i32,
    pub current_stage_nbr: i32,
    pub last_hour_check: i32,
    pub current_stage: Option<AnimalGrowStage>,
    pub pregnant: bool,
    pub pregnant_time: i32,
    pub animal_to_inseminate: Vec<ObjectRef>,
    pub max_milk_actual: f32,
    pub going_to_mom_test: bool,
    pub going_to_mom: bool,
    pub going_to_mom_timer: f32,
    pub eating_grass: bool,
    pub eggs_today: i32,
    pub egg_time: i64,
    pub fertilized: bool,
    pub fertilized_time: i32,
    /// `maleGenome` (`None` for `null`).
    pub male_genome: Option<JHashMap<AnimalGene>>,
    pub hutch_position: i32,
    pub preferred_hutch_position: i32,
    pub trough_path_timer: i32,
    pub trough_to_check: Option<ObjectRef>,
    pub last_milk_timer: i64,
    pub last_pregnancy_time: i64,
    pub last_impregnate_time: i32,
    pub clutch_size: i32,
    pub clutch_size_done: bool,
    pub enter_hutch_timer_after_destroy: i32,
}

impl AnimalData {
    /// The field initialisers of `AnimalData`, with `breed`.
    pub fn new(breed: Option<AnimalBreed>) -> AnimalData {
        AnimalData {
            attached_player: None,
            attached_tree: None,
            attached_tree_x: i32::MAX,
            attached_tree_y: i32::MAX,
            breed,
            milk_qty: 0.0,
            wool_qty: 0.0,
            can_have_milk: false,
            weight: 720.0,
            size: 1.0,
            original_size: 1.0,
            age: 0,
            current_stage_nbr: 0,
            last_hour_check: -1,
            current_stage: None,
            pregnant: false,
            pregnant_time: 0,
            animal_to_inseminate: Vec::new(),
            max_milk_actual: 0.0,
            going_to_mom_test: false,
            going_to_mom: false,
            going_to_mom_timer: 0.0,
            eating_grass: false,
            eggs_today: 0,
            egg_time: 0,
            fertilized: false,
            fertilized_time: 0,
            male_genome: Some(JHashMap::new()),
            hutch_position: -1,
            preferred_hutch_position: -1,
            trough_path_timer: -1,
            trough_to_check: None,
            last_milk_timer: 0,
            last_pregnancy_time: 0,
            last_impregnate_time: 0,
            clutch_size: 0,
            clutch_size_done: false,
            enter_hutch_timer_after_destroy: 0,
        }
    }

    /// `new AnimalData(parent, breed)`: a `null` breed is drawn from the
    /// definition (with the original's off-by-one bound, which throws when
    /// it draws the size); the sex is the definition's or drawn.
    pub fn construct(
        adef: &AnimalDefinitions,
        breed: Option<&AnimalBreed>,
        chr: &mut dyn AnimalCharacter,
        rand: &mut dyn GameRand,
    ) -> JResult<AnimalData> {
        let breed = match breed {
            Some(b) => b.clone(),
            None => {
                let n = adef.breeds.len() as i32;
                let i = rand.next_range(0, n + 1);
                adef.breeds.get(i as usize).cloned().ok_or_else(|| {
                    JavaException(format!(
                        "IndexOutOfBoundsException: Index {i} out of bounds for length {n}"
                    ))
                })?
            }
        };
        if adef.female {
            chr.set_female(true);
        } else if adef.male {
            chr.set_female(false);
        } else {
            chr.set_female(next_bool(rand, 2));
        }
        Ok(AnimalData::new(Some(breed)))
    }
}

/// The 1..6 "speed" sandbox enums of the animal options.
fn speed(v: i32, s: [f32; 5]) -> f32 {
    match v {
        1 => s[0],
        2 => s[1],
        3 => s[2],
        5 => s[3],
        6 => s[4],
        _ => 1.0,
    }
}

fn stage_age_to_grow(animal: &Animal) -> JResult<i32> {
    match &animal.data.current_stage {
        Some(s) => Ok(s.age_to_grow_for(animal.used_gene("ageToGrow").map(|a| a.current_value))),
        None => npe("currentStage"),
    }
}

/// What the character side does after [`Animal::data_eat`] (the
/// behaviour calls of `AnimalData.eat()`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EatFollowUp {
    None,
    /// `setDoingBehavior(false)`, `checkBehavior()`.
    CheckBehavior,
    /// `setDoingBehavior(false)`, `resetBehaviorAction()` (the state event
    /// timer is already 0).
    ResetBehavior,
    /// Grass: when `check`, `setDoingBehavior(false)` and
    /// `checkBehavior()`; then `resetEatingCheck()`.
    Grass {
        check: bool,
    },
}

impl Animal {
    fn adef_of<'d>(&self, ctx: AnimalCtx<'d>) -> &'d AnimalDefinitions {
        self.adef(ctx)
    }

    /// `checkStages()`.
    pub fn data_check_stages(
        &mut self,
        chr: &mut dyn AnimalCharacter,
        env: &mut AnimalEnv<'_>,
    ) -> JResult<()> {
        let adef = self.adef_of(env.ctx);
        let days = self.days_survived(chr);
        let mut stage: Option<AnimalGrowStage> = None;
        for s in &adef.stages {
            if s.stage.as_deref() != self.animal_type.as_deref() {
                continue;
            }
            if days < s.age_to_grow_for(self.used_gene("ageToGrow").map(|a| a.current_value)) {
                continue;
            }
            stage = Some(s.clone());
            break;
        }
        if let Some(stage) = stage {
            if stage.next_stage.is_some() {
                let target = if chr.is_female() {
                    stage.next_stage
                } else {
                    stage.next_stage_male
                };
                self.data_grow(target.as_deref(), chr, env)?;
            }
        }
        Ok(())
    }

    /// `AnimalData.update()`.
    pub fn data_update(
        &mut self,
        chr: &mut dyn AnimalCharacter,
        env: &mut AnimalEnv<'_>,
    ) -> JResult<()> {
        let ctx = env.ctx;
        let mut hour_grow = false;
        if ctx.clock.hour() != self.data.last_hour_check {
            self.data.last_hour_check = ctx.clock.hour();
            chr.set_hours_survived(chr.hours_survived() + 1.0);
            hour_grow = true;
        }
        let mut grow_up = false;
        if self.data.age < self.days_survived(chr) {
            let m = self.data_age_grow_modifier(ctx);
            self.data.age = (self.days_survived(chr) as f32 + (m - 1.0)) as i32;
            chr.set_hours_survived(self.data.age.wrapping_mul(24) as f64);
            grow_up = true;
        }
        if hour_grow {
            self.data_hour_grow(false, chr, env)?;
        }
        if grow_up {
            self.data_grow_up(false, chr, env)?;
        }
        self.data_check_stages(chr, env)?;
        self.data_check_pregnancy(chr, env)?;
        if self.data.eggs_today < self.adef_of(ctx).eggs_per_day && self.data.egg_time == 0 {
            let secs = (ctx.clock.calendar_millis() / 1000) as i32;
            self.data.egg_time = secs.wrapping_add(env.rand.next_range(0, 43200)) as i64;
        }
        Ok(())
    }

    /// `callToTrough(trough)`: `trough_square_exists` is
    /// `trough != null && trough.getSquare() != null`. The caller stops
    /// the animal and turns it to the trough.
    pub fn data_call_to_trough(
        &mut self,
        trough: ObjectRef,
        trough_square_exists: bool,
        chr: &mut dyn AnimalCharacter,
        rand: &mut dyn GameRand,
    ) {
        if chr.exists_in_world() && trough_square_exists {
            self.data_stop_all_movement_now(chr);
            if self.data.trough_path_timer <= -1 {
                self.data.trough_path_timer = rand.next_range(100, 400);
            }
            self.data.trough_to_check = Some(trough);
        }
    }

    /// `stopAllMovementNow()` (the animal's part, `resetEatingCheck`).
    pub fn data_stop_all_movement_now(&mut self, chr: &mut dyn AnimalCharacter) {
        chr.stop_all_movement_now();
        self.data_reset_eating_check(chr);
    }

    /// `checkPregnancy()`.
    fn data_check_pregnancy(
        &mut self,
        chr: &mut dyn AnimalCharacter,
        env: &mut AnimalEnv<'_>,
    ) -> JResult<()> {
        if !(self.data.pregnant && self.data.pregnant_time > 0) {
            return Ok(());
        }
        if self.stress_level > 80.0 && next_bool(env.rand, 50) {
            self.data.pregnant = false;
            self.data.pregnant_time = 0;
        }
        if self.data.pregnant_time >= self.data_pregnant_period(env.ctx) {
            self.data.pregnant = false;
            self.data.pregnant_time = 0;
            let adef = self.adef_of(env.ctx);
            let mut baby_nbr = env.rand.next_range(adef.min_baby, adef.max_baby + 1);
            let health = chr.health();
            if (health as f64) < 0.5 {
                baby_nbr = baby_nbr.wrapping_mul(health as i32);
            }
            for _ in 0..baby_nbr {
                self.add_baby(chr, env)?;
            }
        }
        Ok(())
    }

    /// `getAgeGrowModifier()`.
    pub fn data_age_grow_modifier(&self, ctx: AnimalCtx<'_>) -> f32 {
        speed(ctx.sandbox.animal_age_modifier, [90.0, 10.0, 5.0, 1.0, 1.0])
    }

    /// `growUp(meta)`: a day passed.
    pub fn data_grow_up(
        &mut self,
        meta: bool,
        chr: &mut dyn AnimalCharacter,
        env: &mut AnimalEnv<'_>,
    ) -> JResult<()> {
        let ctx = env.ctx;
        self.data.eggs_today = 0;
        if self.data.pregnant {
            self.data.pregnant_time += 1;
            if self.adef_of(ctx).udder {
                self.data_set_can_have_milk(true, ctx);
            }
        }
        let age_to_grow = stage_age_to_grow(self)? as f32;
        let mut size_grow = (self.max_size(ctx)? - self.min_size(ctx)?) / age_to_grow;
        let mut weight_grow = (self.max_weight(ctx) - self.min_weight(ctx)) / age_to_grow;
        if self.small_enclosure {
            size_grow /= 8.0;
            weight_grow /= 8.0;
        }
        let h = chr.health();
        let s = fmin(self.max_size(ctx)?, self.data.size + size_grow * h);
        self.data_set_size(s, ctx)?;
        let w = fmin(self.max_weight(ctx), self.data.weight + weight_grow * h);
        self.data_set_weight(w, ctx);
        self.data_check_poop(meta, false, chr, env)?;
        self.data.animal_to_inseminate.clear();
        Ok(())
    }

    fn drop_square(
        &self,
        meta: bool,
        chr: &dyn AnimalCharacter,
        env: &mut AnimalEnv<'_>,
    ) -> Option<ObjectRef> {
        let mut sq: Option<ObjectRef> = None;
        if meta {
            let mut check = 1000;
            while !(check <= 0 || sq.is_some_and(|s| env.world.square_is_free(s))) {
                sq = self.random_square_in_zone(env);
                check -= 1;
            }
        }
        if sq.is_none() {
            sq = chr.current_square();
        }
        sq
    }

    /// `checkPoop(meta, force)`: the dung item, if one was dropped.
    pub fn data_check_poop(
        &mut self,
        meta: bool,
        force: bool,
        chr: &mut dyn AnimalCharacter,
        env: &mut AnimalEnv<'_>,
    ) -> JResult<Option<ObjectRef>> {
        let ctx = env.ctx;
        let adef = self.adef_of(ctx);
        if !force {
            if env.rand.next(100) > 60 {
                return Ok(None);
            }
            if self.wild || self.d_zone.is_none() {
                return Ok(None);
            }
            if is_null_or_empty(adef.dung.as_deref()) || self.hutch.is_some() {
                return Ok(None);
            }
            if env.rand.next(100) > adef.dung_chance_per_day {
                return Ok(None);
            }
        }
        let total: i32 = self
            .connected_d_zone
            .iter()
            .map(|z| env.world.zone_dung_count(*z))
            .sum();
        if !force {
            let full = env.world.zone_full_size(self.d_zone.expect("checked"));
            if total > full / 2 {
                return Ok(None);
            }
        }
        let mut base_size = 1.0f32;
        if self.is_baby()? {
            base_size = 0.3;
        }
        let Some(dung) = env.world.create_item(adef.dung.as_deref().unwrap_or("")) else {
            return npe("dung item");
        };
        base_size *= self.data.size / self.max_size(ctx)?;
        env.world.scale_item_weight(dung, base_size);
        if let Some(sq) = self.drop_square(meta, chr, env) {
            env.world.add_world_item(sq, dung, 0.0, 0.0, 0.0);
        }
        Ok(Some(dung))
    }

    /// `dropFeather(meta)`.
    pub fn data_drop_feather(
        &mut self,
        meta: bool,
        chr: &mut dyn AnimalCharacter,
        env: &mut AnimalEnv<'_>,
    ) -> JResult<Option<ObjectRef>> {
        if self.wild || self.d_zone.is_none() {
            return Ok(None);
        }
        let feather = self.breed().and_then(|b| b.feather_item.clone());
        if is_null_or_empty(feather.as_deref()) || self.hutch.is_some() {
            return Ok(None);
        }
        let total: i32 = self
            .connected_d_zone
            .iter()
            .map(|z| env.world.zone_feather_count(*z))
            .sum();
        let full = env.world.zone_full_size(self.d_zone.expect("checked"));
        if total > full / 2 {
            return Ok(None);
        }
        if env.rand.next(100) >= 1 {
            return Ok(None);
        }
        let Some(item) = env.world.create_item(feather.as_deref().unwrap_or("")) else {
            return Ok(None);
        };
        if let Some(sq) = self.drop_square(meta, chr, env) {
            env.world.add_world_item(sq, item, 0.0, 0.0, 0.0);
        }
        Ok(Some(item))
    }

    /// `updateHungerAndThirst(fromMeta)`.
    pub fn data_update_hunger_and_thirst(
        &mut self,
        from_meta: bool,
        chr: &mut dyn AnimalCharacter,
        ctx: AnimalCtx<'_>,
    ) {
        if self.wild {
            return;
        }
        let mut m = 1.0f32;
        if self.hutch.is_some() || Animal::is_animal_sitting(chr) {
            m = 0.5;
        }
        if !self.invincible {
            let mut hunger_red = self.hunger_reduction(ctx);
            let mut thirst_red = self.thirst_reduction(ctx);
            let mult = ctx.clock.multiplier();
            if ctx.sandbox.ultra_speed_doesnot_affect_to_animals && mult > 40.0 {
                hunger_red /= mult / 100.0;
                thirst_red /= mult / 100.0;
            }
            if from_meta {
                hunger_red *= self.hunger_reduction_meta_mod(ctx);
                thirst_red *= self.hunger_reduction_meta_mod(ctx);
            }
            chr.add_stat(AnimalStat::Hunger, hunger_red * m);
            chr.add_stat(AnimalStat::Thirst, thirst_red * m);
        }
    }

    /// `reduceHealthDueToMilk()`.
    pub fn data_reduce_health_due_to_milk(&self, ctx: AnimalCtx<'_>) -> bool {
        if !self.data.can_have_milk {
            return false;
        }
        self.data.milk_qty / self.max_milk(ctx) > 1.15
    }

    /// `updateHealth()`.
    pub fn data_update_health(&mut self, chr: &mut dyn AnimalCharacter, env: &mut AnimalEnv<'_>) {
        let ctx = env.ctx;
        if self.wild {
            return;
        }
        let mut increase = true;
        if let Some(h) = self.hutch {
            if env.world.hutch_dirt(h) > 40.0 {
                increase = false;
            }
        }
        if chr.stat(AnimalStat::Hunger) > 0.8 || chr.stat(AnimalStat::Thirst) > 0.8 {
            increase = false;
            let h = chr.health() - self.data_health_loss(Some(3.0), ctx);
            self.set_health(chr, h);
        }
        if self.is_geriatric(chr, ctx) {
            increase = false;
            let h = chr.health() - self.data_health_loss(Some(30.0), ctx);
            self.set_health(chr, h);
        }
        if !self.data_reduce_health_due_to_milk(ctx) && chr.health() < 1.0 && increase {
            let h = fmin(1.0, chr.health() + self.data_health_loss(Some(3.0), ctx));
            self.set_health(chr, h);
        }
    }

    /// `hourGrow(meta)`: an hour passed.
    pub fn data_hour_grow(
        &mut self,
        meta: bool,
        chr: &mut dyn AnimalCharacter,
        env: &mut AnimalEnv<'_>,
    ) -> JResult<()> {
        let ctx = env.ctx;
        self.ignored_trough.clear();
        if self.data.last_impregnate_time > 0 {
            self.data.last_impregnate_time -= 1;
        }
        self.data_update_hunger_and_thirst(meta, chr, ctx);
        self.data_drop_feather(meta, chr, env)?;
        self.data_update_health(chr, env);
        if meta {
            self.data_eat_and_drink_after_meta(chr, env)?;
        }
        self.data_update_milk(chr, env);
        if self.breed().is_some_and(|b| b.wool_type.is_some()) && self.max_wool(ctx) > 0.0 {
            let w = fmin(
                self.max_wool(ctx),
                self.data.wool_qty + self.data_wool_inc(ctx),
            );
            self.data_set_wool_quantity(w, false, chr, ctx);
        }
        self.data_find_female_to_inseminate(None, chr, env);
        if meta && !self.data.animal_to_inseminate.is_empty() {
            // fertilize replaces this list, so only the first female is
            // reached.
            let mut i = 0;
            while i < self.data.animal_to_inseminate.len() {
                let female = self.data.animal_to_inseminate[i];
                self.fertilize_other(female, env)?;
                i += 1;
            }
            self.data.animal_to_inseminate.clear();
        }
        self.small_enclosure = false;
        if let Some(z) = self.d_zone {
            if env.world.zone_full_size(z) < self.adef_of(ctx).min_enclosure_size {
                self.small_enclosure = true;
            }
        }
        if !meta {
            let cal = ctx.clock.calendar_millis();
            self.data_check_eggs(cal, false, chr, env)?;
        }
        self.data_check_fertilized_time(ctx);
        self.data_check_old(chr, ctx)?;
        self.data_update_weight(chr, ctx);
        Ok(())
    }

    /// `female.fertilize(this, false)` for another animal.
    fn fertilize_other(&mut self, female: ObjectRef, env: &mut AnimalEnv<'_>) -> JResult<()> {
        let ctx = env.ctx;
        let AnimalEnv { rand, world, .. } = env;
        if let Some((f, _)) = world.animal_mut(female) {
            f.fertilize(Male::Other(self), false, ctx, &mut **rand)?;
        }
        Ok(())
    }

    fn data_update_weight(&mut self, chr: &dyn AnimalCharacter, ctx: AnimalCtx<'_>) {
        if chr.stat(AnimalStat::Hunger) as f64 > 0.8 {
            let w = self.data.weight - self.data.weight / 800.0;
            self.data_set_weight(w, ctx);
        }
    }

    fn data_update_milk(&mut self, chr: &mut dyn AnimalCharacter, env: &mut AnimalEnv<'_>) {
        let ctx = env.ctx;
        if self.adef_of(ctx).udder {
            if self.data.can_have_milk {
                let q = self.data.milk_qty + self.data_milk_inc(ctx);
                self.data_set_milk_quantity(q, ctx, env.rand);
                if !self.data.pregnant
                    && ctx.clock.calendar_millis() - self.data.last_milk_timer > TIME_TO_LOSE_MILK
                    && self.babies.as_ref().is_none_or(Vec::is_empty)
                {
                    self.data.can_have_milk = false;
                }
            } else {
                let q = self.data.milk_qty - self.data_milk_inc(ctx) / 2.0;
                self.data_set_milk_quantity(q, ctx, env.rand);
            }
        }
        if self.data_reduce_health_due_to_milk(ctx) {
            let h = chr.health()
                - self.data_health_loss(Some(10.0), ctx)
                    * (self.data.milk_qty / self.max_milk(ctx));
            self.set_health(chr, h);
        }
    }

    fn data_check_old(&mut self, chr: &mut dyn AnimalCharacter, ctx: AnimalCtx<'_>) -> JResult<()> {
        if !self.is_baby()?
            && self.max_age_geriatric(ctx) > 0.0
            && self.geriatric_percentage(ctx) >= 0.95
        {
            let h = chr.health() - 0.1;
            self.set_health(chr, h);
        }
        Ok(())
    }

    /// `getHealthLoss(divide)` (`None` is a `null` divisor, taken as 1).
    pub fn data_health_loss(&self, divide: Option<f32>, ctx: AnimalCtx<'_>) -> f32 {
        let divide = divide.unwrap_or(1.0);
        let base = self.adef_of(ctx).health_loss_multiplier;
        let Some(res) = self.used_gene("resistance") else {
            return base;
        };
        let allele_value = 1.5 - res.current_value;
        let mut dbg_divide = 1.0f32;
        let mult = ctx.clock.multiplier();
        if ctx.sandbox.ultra_speed_doesnot_affect_to_animals && mult > 50.0 {
            dbg_divide = (1000.0 - mult).abs() / 1000.0;
        }
        base * allele_value / divide * dbg_divide
    }

    /// `getMaxMilk()`.
    pub fn max_milk(&self, ctx: AnimalCtx<'_>) -> f32 {
        let a = self.adef_of(ctx);
        match self.used_gene("maxMilk") {
            None => a.max_milk,
            Some(g) => a.max_milk * g.current_value,
        }
    }

    /// `getMinMilk()`.
    pub fn min_milk(&self, ctx: AnimalCtx<'_>) -> f32 {
        let a = self.adef_of(ctx);
        match self.used_gene("maxMilk") {
            None => a.min_milk,
            Some(g) => a.min_milk * g.current_value,
        }
    }

    /// `getMaxWool()`.
    pub fn max_wool(&self, ctx: AnimalCtx<'_>) -> f32 {
        let a = self.adef_of(ctx);
        if a.max_wool <= 0.0 {
            return 0.0;
        }
        match self.used_gene("maxWool") {
            None => a.max_wool,
            Some(g) => a.max_wool * g.current_value,
        }
    }

    fn stress_and_disorder(&self, poor: &str) -> (f32, f32) {
        let mut stress_mod = 1.0f32;
        if self.stress_level > 40.0 {
            stress_mod = 40.0 / self.stress_level;
        }
        let mut disorder_mod = 1.0f32;
        if self.has_genetic_disorder(poor) {
            disorder_mod = 0.2;
        }
        (stress_mod, disorder_mod)
    }

    /// `getMilkInc()`.
    pub fn data_milk_inc(&self, ctx: AnimalCtx<'_>) -> f32 {
        let mut inc = self.data.max_milk_actual / 20.0;
        let Some(g) = self.used_gene("milkInc") else {
            return inc;
        };
        inc *= g.current_value;
        let (s, d) = self.stress_and_disorder("poormilk");
        inc * speed(
            ctx.sandbox.animal_milk_inc_modifier,
            [30.0, 5.0, 2.5, 0.7, 0.2],
        ) * s
            * d
    }

    /// `getWoolInc()`.
    pub fn data_wool_inc(&self, ctx: AnimalCtx<'_>) -> f32 {
        let mut inc = self.max_wool(ctx) / 2400.0;
        let Some(g) = self.used_gene("woolInc") else {
            return inc;
        };
        inc *= g.current_value;
        let (s, d) = self.stress_and_disorder("poorwool");
        inc * speed(
            ctx.sandbox.animal_wool_inc_modifier,
            [30.0, 5.0, 2.5, 0.7, 0.2],
        ) * s
            * d
    }

    fn calc_clutch_size(&self, ctx: AnimalCtx<'_>) -> i32 {
        let m = self
            .used_gene("eggClutch")
            .map(|a| a.current_value)
            .unwrap_or(1.0);
        (self.adef_of(ctx).max_clutch_size as f32 * m) as i32
    }

    /// `checkEggs(realCal, meta)`; `cal_millis` is the calendar's time.
    pub fn data_check_eggs(
        &mut self,
        cal_millis: i64,
        meta: bool,
        chr: &mut dyn AnimalCharacter,
        env: &mut AnimalEnv<'_>,
    ) -> JResult<()> {
        let ctx = env.ctx;
        if self.wild || chr.is_dead() {
            return Ok(());
        }
        let adef = self.adef_of(ctx);
        if self.data.eggs_today < adef.eggs_per_day && self.data.egg_time == 0 && meta {
            self.data.egg_time = ((cal_millis / 1000) as i32).wrapping_sub(10) as i64;
        }
        if adef.eggs_per_day <= 0 {
            return Ok(());
        }
        let in_period = self.data_is_in_laying_egg_period(cal_millis, ctx);
        if self.data_have_laying_egg_period(ctx) && !in_period {
            self.data.clutch_size_done = false;
            return Ok(());
        }
        let season = ctx.sandbox.animal_mating_season;
        if self.data_have_laying_egg_period(ctx)
            && !self.data.clutch_size_done
            && self.data.clutch_size == 0
            && adef.min_clutch_size > 0
            && in_period
            && season
        {
            self.data.clutch_size = self.calc_clutch_size(ctx);
            self.data.clutch_size_done = true;
        }
        if self.data_have_laying_egg_period(ctx)
            && self.data.clutch_size == 0
            && adef.min_clutch_size > 0
            && season
        {
            return Ok(());
        }
        if self.data.egg_time > 0 && cal_millis / 1000 > self.data.egg_time {
            let region = self.data_region_hutch(ctx, env)?;
            let use_hutch = adef.hutches.is_some()
                && region.is_some_and(|h| {
                    !env.world.hutch_door_closed(h) && env.world.hutch_has_room_for_eggs(h)
                })
                && self.hutch.is_none();
            if use_hutch {
                let h = region.expect("checked");
                if meta {
                    if !env.world.hutch_add_meta_egg(h) {
                        self.add_egg(true, chr, env)?;
                    }
                    self.data.eggs_today += 1;
                    self.data.egg_time = 0;
                } else {
                    self.behavior_call_to_hutch(None, true, chr, env)?;
                }
            } else if self.add_egg(false, chr, env)? {
                self.data.eggs_today += 1;
                self.data.egg_time = 0;
            }
        }
        Ok(())
    }

    /// `checkFertilizedTime()`.
    pub fn data_check_fertilized_time(&mut self, ctx: AnimalCtx<'_>) {
        if self.data.fertilized {
            self.data.fertilized_time += 1;
        }
        if self.data.fertilized_time > self.adef_of(ctx).fertilized_time_max {
            self.data.fertilized = false;
            self.data.fertilized_time = 0;
        }
    }

    /// `getPregnantPeriod()`.
    pub fn data_pregnant_period(&self, ctx: AnimalCtx<'_>) -> i32 {
        let base = self.adef_of(ctx).pregnant_period;
        let m = speed(
            ctx.sandbox.animal_pregnancy_time,
            [0.01, 0.2, 0.7, 2.0, 3.0],
        );
        let r = (base as f32 * m) as i32;
        r.max(8)
    }

    fn thirst_reduction(&self, ctx: AnimalCtx<'_>) -> f32 {
        let mut base = self.adef_of(ctx).thirst_multiplier;
        let Some(a) = self.used_gene("thirstResistance") else {
            return base;
        };
        let m = 1.0 - a.current_value + 1.0;
        if self.has_genetic_disorder("highthirst") {
            base *= 10.0;
        }
        base * (m * self.hunger_reduction_mod(ctx))
    }

    fn hunger_reduction(&self, ctx: AnimalCtx<'_>) -> f32 {
        let mut base = self.adef_of(ctx).hunger_multiplier;
        let Some(a) = self.used_gene("hungerResistance") else {
            return base;
        };
        let m = 1.0 - a.current_value + 1.0;
        if self.has_genetic_disorder("gluttonous") {
            base *= 10.0;
        }
        base * (m * self.hunger_reduction_mod(ctx))
    }

    fn hunger_reduction_meta_mod(&self, ctx: AnimalCtx<'_>) -> f32 {
        speed(
            ctx.sandbox.animal_meta_stats_modifier,
            [4.0, 2.0, 1.5, 0.7, 0.2],
        )
    }

    fn hunger_reduction_mod(&self, ctx: AnimalCtx<'_>) -> f32 {
        speed(ctx.sandbox.animal_stats_modifier, [4.0, 2.0, 1.5, 0.7, 0.2])
    }

    // ---------------------------------------------------------------
    // eating and drinking

    fn eat_and_drink_after_meta_vehicle(
        &mut self,
        chr: &mut dyn AnimalCharacter,
        env: &mut AnimalEnv<'_>,
    ) -> JResult<()> {
        if self.is_baby()? {
            return Ok(());
        }
        let mut iteration = 0;
        loop {
            let i = iteration;
            iteration += 1;
            if !(i < 50 && chr.stat(AnimalStat::Hunger) >= 0.1) {
                return Ok(());
            }
            let mut found = false;
            if chr.vehicle().is_some() && self.eat_from_vehicle(chr, env)? {
                found = true;
            }
            if found {
                continue;
            }
            return Ok(());
        }
    }

    fn edible_type(&self, item: ObjectRef, ctx: AnimalCtx<'_>, env: &AnimalEnv<'_>) -> bool {
        let Some(types) = &self.adef_of(ctx).eat_type_trough else {
            return false;
        };
        let Some(fi) = env.world.feed_item(item) else {
            return false;
        };
        for t in types {
            let feed = fi.animal_feed_type.as_deref() == Some(t.as_str());
            match &fi.kind {
                FeedItemKind::Food { food_type, .. } => {
                    if food_type.as_deref() == Some(t.as_str()) || feed {
                        return true;
                    }
                }
                FeedItemKind::Drainable { .. } => {
                    if feed {
                        return true;
                    }
                }
                FeedItemKind::Other => {}
            }
        }
        false
    }

    fn eat_from_vehicle(
        &mut self,
        chr: &mut dyn AnimalCharacter,
        env: &mut AnimalEnv<'_>,
    ) -> JResult<bool> {
        let ctx = env.ctx;
        let Some(vehicle) = chr.vehicle() else {
            return Ok(false);
        };
        let Some(items) = env.world.vehicle_food_items(vehicle) else {
            return Ok(false);
        };
        let edible = items.into_iter().find(|i| self.edible_type(*i, ctx, env));
        match edible {
            Some(item) => {
                self.data_eat_item(item, false, chr, env)?;
                Ok(true)
            }
            None => Ok(false),
        }
    }

    fn mother_milk(&self, env: &mut AnimalEnv<'_>) -> Option<(f32, bool)> {
        let m = self.mother?;
        env.world
            .animal_mut(m)
            .map(|(a, c)| (a.data.milk_qty, c.exists_in_world()))
    }

    fn set_mother_milk(
        &self,
        delta_draw: impl FnOnce(&mut dyn GameRand) -> f32,
        env: &mut AnimalEnv<'_>,
    ) {
        let Some(m) = self.mother else { return };
        let ctx = env.ctx;
        let AnimalEnv { rand, world, .. } = env;
        if let Some((a, _)) = world.animal_mut(m) {
            let q = a.data.milk_qty - delta_draw(&mut **rand);
            a.data_set_milk_quantity(q, ctx, &mut **rand);
        }
    }

    /// `eatAndDrinkAfterMeta()`.
    fn data_eat_and_drink_after_meta(
        &mut self,
        chr: &mut dyn AnimalCharacter,
        env: &mut AnimalEnv<'_>,
    ) -> JResult<()> {
        let ctx = env.ctx;
        if chr.vehicle().is_some() {
            return self.eat_and_drink_after_meta_vehicle(chr, env);
        }
        let adef = self.adef_of(ctx);
        let mut iteration = 0;
        loop {
            let it = iteration;
            iteration += 1;
            if !(it < 50 && chr.stat(AnimalStat::Hunger) >= 0.1) {
                break;
            }
            if self.is_baby()? && adef.eat_from_mother {
                if let Some((milk, exists)) = self.mother_milk(env) {
                    if exists && milk as f64 > 0.1 {
                        chr.remove_stat(AnimalStat::Hunger, 0.2);
                        self.set_mother_milk(|r| r.next_float_range(0.1, 0.3), env);
                        continue;
                    }
                }
            }
            let mut found = false;
            let zones = self.connected_d_zone.clone();
            'block1: for zone in zones {
                let troughs = env.world.zone_troughs(zone);
                let food = env.world.zone_food_on_ground(zone);
                if troughs.is_empty() && food.is_empty() {
                    break;
                }
                for f in &food {
                    if adef.eat_type_trough.is_none() {
                        continue;
                    }
                    if let Some((item, _)) = env.world.world_item(*f) {
                        if self.edible_type(item, ctx, env) {
                            self.eat_from_ground = Some(*f);
                        }
                    }
                    if self.eat_from_ground.is_none() {
                        break;
                    }
                    let f = self.data_eat(chr, env)?;
                    self.eat_follow_up_away(f, chr);
                    found = true;
                    break;
                }
                if troughs.is_empty() {
                    continue;
                }
                for t in troughs {
                    if self.data_can_eat_from_trough(t, ctx, env).is_none() {
                        continue;
                    }
                    self.eat_from_trough = Some(t);
                    let f = self.data_eat(chr, env)?;
                    self.eat_follow_up_away(f, chr);
                    found = true;
                    continue 'block1;
                }
            }
            if found {
                continue;
            }
            break;
        }
        let mut iteration = 0;
        loop {
            let it = iteration;
            iteration += 1;
            if !(it < 50 && chr.stat(AnimalStat::Thirst) >= 0.1) {
                break;
            }
            let mut found = false;
            for zone in self.connected_d_zone.clone() {
                if let Some(sq) = env.world.zone_first_near_water_square(zone) {
                    self.drink_from_river = Some(sq);
                    self.data_drink(chr, env)?;
                    found = true;
                    break;
                }
                if env.world.puddles_size() > 0.13 {
                    self.drink_from_puddle = chr.current_square();
                    self.data_drink(chr, env)?;
                    found = true;
                    break;
                }
                for t in env.world.zone_troughs(zone) {
                    // `!(water > 0)`: NaN counts as empty.
                    if env.world.trough_water(t).partial_cmp(&0.0)
                        != Some(std::cmp::Ordering::Greater)
                    {
                        continue;
                    }
                    self.drink_from_trough = Some(t);
                    self.data_drink(chr, env)?;
                    found = true;
                    break;
                }
                if !found {
                    break;
                }
            }
            if found {
                continue;
            }
            break;
        }
        Ok(())
    }

    /// `getRandomTroughList()` (deprecated in the original): the connected
    /// zones' troughs shuffled with a fresh `java.util.Random` (`random`)
    /// whose first `nextInt()` is discarded.
    pub fn data_random_trough_list(
        &self,
        env: &AnimalEnv<'_>,
        random: &mut dyn FnMut(Option<i32>) -> i32,
    ) -> Vec<ObjectRef> {
        let mut result: Vec<ObjectRef> = Vec::new();
        for z in &self.connected_d_zone {
            result.extend(env.world.zone_troughs(*z));
        }
        let n = result.len();
        random(None);
        for i in 0..n {
            let change = i + random(Some((n - i) as i32)) as usize;
            result.swap(i, change);
        }
        result
    }

    /// `resetEatingCheck()`.
    pub fn data_reset_eating_check(&mut self, chr: &mut dyn AnimalCharacter) {
        self.drink_from_puddle = None;
        self.drink_from_river = None;
        self.drink_from_trough = None;
        self.eat_from_trough = None;
        self.eat_from_ground = None;
        self.moving_to_food = None;
        self.data.eating_grass = false;
        chr.clear_variable("idleAction");
    }

    /// `canEatFromTrough(trough)`: the first item the animal eats.
    pub fn data_can_eat_from_trough(
        &self,
        trough: ObjectRef,
        ctx: AnimalCtx<'_>,
        env: &AnimalEnv<'_>,
    ) -> Option<ObjectRef> {
        let types = self.adef_of(ctx).eat_type_trough.as_ref()?;
        let has = |s: Option<&str>| s.is_some_and(|s| types.iter().any(|t| t == s));
        for item in env.world.trough_items(trough) {
            let Some(fi) = env.world.feed_item(item) else {
                continue;
            };
            if let FeedItemKind::Food { rotten: true, .. } = fi.kind {
                continue;
            }
            if has(Some("All")) || has(Some(&fi.full_type)) || has(fi.animal_feed_type.as_deref()) {
                return Some(item);
            }
            if let FeedItemKind::Food { food_type, .. } = &fi.kind {
                if has(food_type.as_deref()) {
                    return Some(item);
                }
            }
        }
        None
    }

    fn eat_anim(&self, chr: &mut dyn AnimalCharacter, ctx: AnimalCtx<'_>, rand: &mut dyn GameRand) {
        chr.set_variable("idleAction", "eat");
        let n = self.adef_of(ctx).eating_type_nbr;
        if n > 0 {
            let k = rand.next_range(1, n + 1);
            chr.set_variable("eatingAnim", &format!("eat{k}"));
        }
    }

    fn after_drink(&mut self, chr: &mut dyn AnimalCharacter, done: bool, env: &mut AnimalEnv<'_>) {
        if done {
            self.data_reset_eating_check(chr);
            chr.set_state_event_delay_timer(0.0);
            return;
        }
        if chr.stat(AnimalStat::Thirst) > 0.1 {
            self.eat_anim(chr, env.ctx, env.rand);
        }
    }

    /// `drinkFromGround()`: from the `IsoWorldInventoryObject` the animal
    /// eats from ([`AnimalWorld::world_item_use_fluid`](crate::env::AnimalWorld::world_item_use_fluid)).
    pub fn data_drink_from_ground(
        &mut self,
        chr: &mut dyn AnimalCharacter,
        env: &mut AnimalEnv<'_>,
    ) {
        let tb = self.adef_of(env.ctx).thirst_boost;
        let Some(obj) = self.eat_from_ground else {
            return;
        };
        let has_fluid = env.world.world_item_use_fluid(obj, 2.0 / tb);
        chr.remove_stat(AnimalStat::Thirst, 0.2 * tb);
        let done = chr.stat(AnimalStat::Thirst) < 0.1 || !has_fluid;
        self.after_drink(chr, done, env);
    }

    /// `drink()`.
    pub fn data_drink(
        &mut self,
        chr: &mut dyn AnimalCharacter,
        env: &mut AnimalEnv<'_>,
    ) -> JResult<()> {
        let tb = self.adef_of(env.ctx).thirst_boost;
        if self.drink_from_river.is_some() || self.drink_from_puddle.is_some() {
            chr.remove_stat(AnimalStat::Thirst, 0.2 * tb);
            let done = chr.stat(AnimalStat::Thirst) < 0.1;
            self.after_drink(chr, done, env);
            return Ok(());
        }
        let Some(t) = self.drink_from_trough else {
            return Ok(());
        };
        if env.world.trough_water(t) <= 0.0 {
            return Ok(());
        }
        let amount = env.rand.next_float_range(0.4, 0.6) / tb;
        env.world.trough_remove_water(t, amount);
        chr.remove_stat(AnimalStat::Thirst, 0.2 * tb);
        let done = chr.stat(AnimalStat::Thirst) < 0.1 || env.world.trough_water(t) <= 0.0;
        self.after_drink(chr, done, env);
        Ok(())
    }

    /// `eatItem(item, onground)`.
    pub fn data_eat_item(
        &mut self,
        item: ObjectRef,
        on_ground: bool,
        chr: &mut dyn AnimalCharacter,
        env: &mut AnimalEnv<'_>,
    ) -> JResult<()> {
        let ctx = env.ctx;
        let adef = self.adef_of(ctx);
        let Some(fi) = env.world.feed_item(item) else {
            return npe("item");
        };
        let hb = adef.hunger_boost;
        let tb = adef.thirst_boost;
        if fi.pure_milk {
            while !env.world.fluid_empty(item)
                && (chr.stat(AnimalStat::Hunger) > 0.1 || chr.stat(AnimalStat::Thirst) > 0.1)
            {
                chr.remove_stat(AnimalStat::Hunger, 0.2);
                chr.remove_stat(AnimalStat::Thirst, 0.2);
                let a = env.rand.next_float_range(0.2 / hb, 0.5 / hb);
                env.world.remove_fluid(item, a);
            }
        }
        match fi.kind {
            FeedItemKind::Food {
                hunger_change,
                hung_change,
                thirst_change,
                ..
            } => {
                let hung_reduce = chr.stat(AnimalStat::Hunger) / hb;
                let hung_on_food = hunger_change;
                if hung_on_food.abs() <= hung_reduce {
                    let h = chr.stat(AnimalStat::Hunger) + hung_on_food * hb;
                    chr.set_stat(AnimalStat::Hunger, h);
                    chr.add_stat(AnimalStat::Thirst, thirst_change);
                    env.world.consume_item(item, on_ground, false);
                } else {
                    let thirst_left = thirst_change.abs() - chr.stat(AnimalStat::Thirst) / tb;
                    chr.reset_stat(AnimalStat::Hunger);
                    chr.add_stat(AnimalStat::Thirst, thirst_change * tb);
                    let new_hung = hung_change * ((hung_on_food + hung_reduce) / hung_on_food);
                    let new_thirst = if thirst_change.abs() > 0.0 {
                        if thirst_left > 0.0 {
                            -thirst_left
                        } else {
                            0.0
                        }
                    } else {
                        thirst_change
                    };
                    env.world.food_set_changes(item, new_hung, new_thirst);
                    if new_hung as f64 > -0.01 {
                        env.world.consume_item(item, false, false);
                    }
                }
            }
            FeedItemKind::Drainable { current_uses } => {
                let hung_reduce = chr.stat(AnimalStat::Hunger);
                let hung_on_food = current_uses as f32 * 0.1;
                if hung_on_food.abs() <= hung_reduce {
                    chr.remove_stat(AnimalStat::Hunger, hung_on_food);
                    for _ in 0..current_uses {
                        env.world.drainable_use(item);
                        chr.remove_stat(AnimalStat::Hunger, 0.1);
                    }
                    env.world.consume_item(item, on_ground, true);
                } else {
                    let using_delta = java_round(hung_reduce / 0.1) as f32;
                    let mut i = 0;
                    while (i as f32) < using_delta {
                        env.world.drainable_use(item);
                        i += 1;
                    }
                    chr.reset_stat(AnimalStat::Hunger);
                }
            }
            FeedItemKind::Other => {}
        }
        if let Some(t) = fi.in_trough {
            env.world.trough_eaten_from(t);
        }
        Ok(())
    }

    /// The behaviour calls of `eat()` for an animal that is away (the meta
    /// update): it is not in the world, so `checkBehavior()` stops after
    /// `setDoingBehavior(false)` and clearing an `idle*` idle action, and
    /// `resetBehaviorAction()` clears the action (the turn delta and the
    /// fighting opponent are the character's, and the character is not
    /// updated while away).
    fn eat_follow_up_away(&mut self, f: EatFollowUp, chr: &mut dyn AnimalCharacter) {
        let in_vehicle = chr.vehicle().is_some();
        let check = |a: &mut Animal, chr: &mut dyn AnimalCharacter| {
            a.behavior.set_doing_behavior(false, in_vehicle);
            let idle = chr.variable("idleAction");
            if !idle.is_empty() && idle.starts_with("idle") {
                chr.clear_variable("idleAction");
            }
        };
        match f {
            EatFollowUp::None => {}
            EatFollowUp::CheckBehavior => check(self, chr),
            EatFollowUp::ResetBehavior => {
                self.behavior.set_doing_behavior(false, in_vehicle);
                self.behavior.behavior_action = None;
                self.behavior.behavior_object = None;
            }
            EatFollowUp::Grass { check: c } => {
                if c {
                    check(self, chr);
                }
                self.data_reset_eating_check(chr);
            }
        }
    }

    /// `eat()`. The behaviour calls it makes (`checkBehavior`,
    /// `resetBehaviorAction`) move the animal, so they are the character
    /// side's: the answer says which one to make, and for grass the
    /// character side then finishes with [`Animal::data_reset_eating_check`].
    pub fn data_eat(
        &mut self,
        chr: &mut dyn AnimalCharacter,
        env: &mut AnimalEnv<'_>,
    ) -> JResult<EatFollowUp> {
        let ctx = env.ctx;
        let adef = self.adef_of(ctx);
        if let Some(obj) = self.eat_from_ground {
            let Some((item, true)) = env.world.world_item(obj) else {
                self.eat_from_ground = None;
                return Ok(EatFollowUp::None);
            };
            self.data_eat_item(item, true, chr, env)?;
            self.data_reset_eating_check(chr);
            if chr.stat(AnimalStat::Hunger) >= adef.thirst_hunger_trigger {
                return Ok(EatFollowUp::CheckBehavior);
            }
            chr.set_state_event_delay_timer(0.0);
            return Ok(EatFollowUp::ResetBehavior);
        }
        if let Some(t) = self.eat_from_trough {
            let Some(item) = self.data_can_eat_from_trough(t, ctx, env) else {
                self.data_reset_eating_check(chr);
                chr.set_state_event_delay_timer(0.0);
                return Ok(EatFollowUp::None);
            };
            self.data_eat_item(item, false, chr, env)?;
            if chr.stat(AnimalStat::Hunger) < 0.1 {
                self.data_reset_eating_check(chr);
                chr.set_state_event_delay_timer(0.0);
                return Ok(EatFollowUp::None);
            }
            if chr.stat(AnimalStat::Hunger) > adef.thirst_hunger_trigger {
                self.eat_anim(chr, ctx, env.rand);
            }
            self.eat_anim(chr, ctx, env.rand);
            return Ok(EatFollowUp::None);
        }
        if let Some(m) = self.mother {
            let enough = env
                .world
                .animal_mut(m)
                .map(|(a, _)| a.have_enough_milk_to_feed_from());
            if let (Some(d), Some(true)) = (env.world.distance_to(m), enough) {
                if d <= 2.0 {
                    chr.remove_stat(AnimalStat::Hunger, 0.2);
                    chr.remove_stat(AnimalStat::Thirst, 0.2);
                    let hb = adef.hunger_boost;
                    self.set_mother_milk(|r| r.next_float_range(0.2 / hb, 0.5 / hb), env);
                    let enough = env
                        .world
                        .animal_mut(m)
                        .is_some_and(|(a, _)| a.have_enough_milk_to_feed_from());
                    if chr.stat(AnimalStat::Hunger) < 0.1
                        || chr.stat(AnimalStat::Thirst) < 0.1
                        || !enough
                    {
                        self.data_reset_eating_check(chr);
                        chr.set_state_event_delay_timer(0.0);
                        if let Some((ma, _)) = env.world.animal_mut(m) {
                            ma.behavior.block_movement = false;
                        }
                        return Ok(EatFollowUp::None);
                    }
                    if chr.stat(AnimalStat::Hunger) as f64 > 0.05 && enough {
                        chr.set_variable("idleAction", "eat");
                        chr.set_variable("eatingAnim", "feed");
                    }
                    return Ok(EatFollowUp::None);
                }
            }
        }
        if self.data.eating_grass {
            chr.remove_stat(AnimalStat::Hunger, 0.15);
            if let Some(sq) = chr.current_square() {
                env.world.remove_grass(sq);
            }
            return Ok(EatFollowUp::Grass {
                check: chr.stat(AnimalStat::Hunger) > 0.0,
            });
        }
        Ok(EatFollowUp::None)
    }

    // ---------------------------------------------------------------
    // breeding

    /// `canBePregnant()`.
    pub fn data_can_be_pregnant(&self, chr: &dyn AnimalCharacter, ctx: AnimalCtx<'_>) -> bool {
        if !chr.is_female() {
            return false;
        }
        let tb = self.data_time_before_next_pregnancy(ctx);
        if tb > 0
            && self.data.last_pregnancy_time > 0
            && ctx.clock.calendar_millis() - self.data.last_pregnancy_time
                < ONE_DAY_MILLISECONDS.wrapping_mul(tb as i64)
        {
            return false;
        }
        if self.is_geriatric(chr, ctx) {
            return false;
        }
        let adef = self.adef_of(ctx);
        let common = self.is_in_mating_season(ctx)
            && !is_null_or_empty(adef.baby_type.as_deref())
            && chr.current_square().is_some()
            && self.days_survived(chr) >= self.min_age_for_baby(ctx);
        if common && adef.eggs_per_day == 0 && !self.data.pregnant {
            return true;
        }
        common && adef.eggs_per_day > 0 && self.data.fertilized_time == 0 && !self.data.fertilized
    }

    /// `tryInseminateInMeta(realCal)`.
    pub fn data_try_inseminate_in_meta(
        &mut self,
        cal_millis: i64,
        chr: &mut dyn AnimalCharacter,
        env: &mut AnimalEnv<'_>,
    ) -> JResult<()> {
        if chr.is_female() {
            return Ok(());
        }
        self.data_find_female_to_inseminate(Some(cal_millis), chr, env);
        if self.data.animal_to_inseminate.is_empty() {
            return Ok(());
        }
        let i = env
            .rand
            .next_range(0, self.data.animal_to_inseminate.len() as i32);
        let female = self.data.animal_to_inseminate[i as usize];
        self.fertilize_other(female, env)
    }

    /// `findFemaleToInseminate(realCal)`.
    pub fn data_find_female_to_inseminate(
        &mut self,
        _cal_millis: Option<i64>,
        chr: &dyn AnimalCharacter,
        env: &mut AnimalEnv<'_>,
    ) {
        let ctx = env.ctx;
        if chr.current_square().is_none() {
            return;
        }
        let adef = self.adef_of(ctx);
        if !(self.data.animal_to_inseminate.is_empty()
            && !chr.is_female()
            && !is_null_or_empty(adef.mate.as_deref())
            && self.days_survived(chr) >= self.min_age_for_baby(ctx)
            && self.data_last_impregnate_period(chr, ctx) == 0)
        {
            return;
        }
        let mate = adef.mate.clone().unwrap_or_default();
        for zone in self.connected_d_zone.clone() {
            for a in env.world.zone_animals(zone) {
                if a == self.self_ref {
                    continue;
                }
                let ok = env.world.animal_mut(a).is_some_and(|(o, oc)| {
                    o.animal_type.as_deref() == Some(mate.as_str())
                        && o.data_can_be_pregnant(oc, ctx)
                });
                if ok {
                    self.data.animal_to_inseminate.push(a);
                }
            }
        }
    }

    // ---------------------------------------------------------------
    // size, weight, stage

    /// `initSize()`.
    pub fn data_init_size(
        &mut self,
        _chr: &dyn AnimalCharacter,
        ctx: AnimalCtx<'_>,
    ) -> JResult<()> {
        let min = self.min_size(ctx)?;
        self.data_set_size(min, ctx)?;
        let size_grow = (self.max_size(ctx)? - self.min_size(ctx)?) / stage_age_to_grow(self)? as f32;
        let v = fmax(
            self.min_size(ctx)?,
            fmin(
                self.max_size(ctx)?,
                self.data.size + size_grow * (self.data.age - self.adef_of(ctx).min_age) as f32,
            ),
        );
        self.data_set_size(v, ctx)?;
        self.data_set_size(0.1, ctx)?;
        Ok(())
    }

    /// `initWeight()`.
    pub fn data_init_weight(&mut self, ctx: AnimalCtx<'_>) -> JResult<()> {
        let min = self.min_weight(ctx);
        self.data_set_weight(min, ctx);
        let weight_grow =
            (self.max_weight(ctx) - self.min_weight(ctx)) / stage_age_to_grow(self)? as f32;
        let v = fmax(
            self.min_weight(ctx),
            fmin(
                self.max_weight(ctx),
                self.data.weight + weight_grow * (self.data.age - self.adef_of(ctx).min_age) as f32,
            ),
        );
        self.data_set_weight(v, ctx);
        Ok(())
    }

    /// `initStage()`.
    pub fn data_init_stage(&mut self, ctx: AnimalCtx<'_>) {
        if self.data.current_stage.is_some() {
            return;
        }
        let adef = self.adef_of(ctx);
        self.data.current_stage = adef
            .stages
            .iter()
            .find(|s| s.stage.as_deref() == self.animal_type.as_deref())
            .cloned();
    }

    /// `grow(newtype)`: builds the grown animal and hands it to the world.
    pub fn data_grow(
        &mut self,
        new_type: Option<&str>,
        chr: &mut dyn AnimalCharacter,
        env: &mut AnimalEnv<'_>,
    ) -> JResult<()> {
        let ctx = env.ctx;
        if let Some(m) = self.mother {
            let me = self.self_ref;
            if let Some((mother, _)) = env.world.animal_mut(m) {
                if let Some(b) = &mut mother.babies {
                    if let Some(i) = b.iter().position(|x| *x == me) {
                        b.remove(i);
                    }
                }
            }
        }
        let Some(def) = new_type.and_then(|t| ctx.def(t)) else {
            return npe("animalDef");
        };
        let Some(breed) = &self.data.breed else {
            return npe("breed");
        };
        let new_breed = def
            .breed_by_name(breed.name.as_deref().unwrap_or(""))
            .cloned();
        let (x, y, z) = chr.position();
        let new_type = new_type.expect("checked");
        let mut g = Animal::construct(
            new_type,
            new_breed.as_ref(),
            (x.floor(), y.floor(), z.floor()),
            env,
        )?;
        let NewAnimal { animal: n, chr: nc } = &mut g;
        n.data.age = self.data.age;
        nc.set_hours_survived(self.data.age.wrapping_mul(24) as f64);
        n.data.current_stage_nbr = self.data.current_stage_nbr + 1;
        n.data.attached_player = self.data.attached_player;
        n.data.attached_tree = self.data.attached_tree;
        n.data.attached_tree_x = self.data.attached_tree_x;
        n.data.attached_tree_y = self.data.attached_tree_y;
        nc.set_stat(AnimalStat::Hunger, chr.stat(AnimalStat::Hunger));
        nc.set_stat(AnimalStat::Thirst, chr.stat(AnimalStat::Thirst));
        n.player_acceptance = self.player_acceptance.clone();
        n.stress_level = self.stress_level;
        n.full_genome = self.full_genome.clone();
        n.check_genetic_disorder();
        n.data_init_size(nc, ctx)?;
        n.custom_name = self.custom_name.clone();
        nc.set_female(chr.is_female());
        nc.vehicle = chr.vehicle();
        n.animal_id = self.animal_id;
        if env.world.new_animal_blocked() {
            return Ok(());
        }
        let mut size_delta =
            (self.data.size - self.min_size(ctx)?) / (self.max_size(ctx)? - self.min_size(ctx)?);
        if size_delta as f64 > 0.7 {
            size_delta -= 0.7;
            let s = n.data.size * (1.0 + size_delta);
            n.data_set_size(s, ctx)?;
        }
        n.invincible = self.invincible;
        env.world.replace_grown_animal(g);
        Ok(())
    }

    /// `getDaysSurvived()`.
    pub fn days_survived(&self, chr: &dyn AnimalCharacter) -> i32 {
        let hours = fmax(0.0, chr.hours_survived() as f32);
        hours as i32 / 24
    }

    /// `canHaveBaby()`.
    pub fn data_can_have_baby(&self, chr: &dyn AnimalCharacter, ctx: AnimalCtx<'_>) -> bool {
        chr.is_female()
            && self.adef_of(ctx).baby_type.is_some()
            && self.days_survived(chr) >= self.min_age_for_baby(ctx)
    }

    /// `AnimalData.init()`.
    pub fn data_init(&mut self, chr: &dyn AnimalCharacter, ctx: AnimalCtx<'_>) -> JResult<()> {
        self.data_init_stage(ctx);
        self.data_init_size(chr, ctx)?;
        self.data_init_weight(ctx)?;
        self.data.last_hour_check = ctx.clock.hour();
        Ok(())
    }

    /// `setAttachedTree(tree)`: `square` is the tree's square, if any.
    pub fn data_set_attached_tree(&mut self, tree: Option<ObjectRef>, square: Option<(i32, i32)>) {
        match (tree, square) {
            (Some(_), Some((x, y))) => {
                self.data.attached_tree_x = x;
                self.data.attached_tree_y = y;
            }
            _ => {
                self.data.attached_tree_x = i32::MAX;
                self.data.attached_tree_y = i32::MAX;
            }
        }
        self.data.attached_tree = tree;
    }

    /// `setMilkQuantity(qty)`: taking milk raises the actual maximum.
    pub fn data_set_milk_quantity(
        &mut self,
        qty: f32,
        ctx: AnimalCtx<'_>,
        rand: &mut dyn GameRand,
    ) {
        if qty < self.data.milk_qty {
            let max = self.max_milk(ctx);
            if self.data.max_milk_actual < max {
                self.data.max_milk_actual += rand.next_float_range(0.01, 0.03);
                self.data.max_milk_actual = fmin(self.data.max_milk_actual, self.max_milk(ctx));
            }
            if self.data.can_have_milk {
                self.data_update_last_time_milked(ctx);
            }
        }
        self.data.milk_qty = fmin(fmax(qty, 0.0), self.data.max_milk_actual);
    }

    /// `setSize(size)`: clamped to the genome's range.
    pub fn data_set_size(&mut self, size: f32, ctx: AnimalCtx<'_>) -> JResult<()> {
        self.data.size = clamp(size, self.min_size(ctx)?, self.max_size(ctx)?);
        Ok(())
    }

    /// `setSizeForced(size)`.
    pub fn data_set_size_forced(&mut self, size: f32) {
        self.data.original_size = self.data.size;
        self.data.size = size;
    }

    /// `setCanHaveMilk(b)`.
    pub fn data_set_can_have_milk(&mut self, mut b: bool, ctx: AnimalCtx<'_>) {
        if self.geriatric_percentage(ctx) as f64 > 0.8 {
            b = false;
        }
        if self.has_genetic_disorder("nomilk") {
            b = false;
        }
        if !self.data.can_have_milk && b {
            self.data_update_last_time_milked(ctx);
        }
        if b && self.data.max_milk_actual == 0.0 {
            self.data.max_milk_actual = self.min_milk(ctx);
        }
        self.data.can_have_milk = b;
    }

    /// `setWoolQuantity(qty, force)`.
    pub fn data_set_wool_quantity(
        &mut self,
        mut qty: f32,
        force: bool,
        chr: &mut dyn AnimalCharacter,
        ctx: AnimalCtx<'_>,
    ) {
        if !force && self.data.age < 200 {
            return;
        }
        if !force && self.has_genetic_disorder("nowool") {
            qty = 0.0;
        }
        let max = self.max_wool(ctx);
        let was_fleece = self.data.wool_qty >= max / 2.0;
        self.data.wool_qty = fmin(qty, max);
        let fleece = self.data.wool_qty >= self.max_wool(ctx) / 2.0;
        if was_fleece != fleece {
            chr.on_fleece_changed();
        }
    }

    /// `getRegionHutch()`.
    pub fn data_region_hutch(&self, ctx: AnimalCtx<'_>, env: &AnimalEnv<'_>) -> JResult<Option<ObjectRef>> {
        let types = self.adef_of(ctx).hutches.as_ref();
        for z in &self.connected_d_zone {
            for (h, ty) in env.world.zone_hutches(*z) {
                let Some(types) = types else {
                    // adef.hutches.size() is dereferenced only after a real hutch.
                    return npe("adef.hutches");
                };
                if types.contains(&ty) {
                    return Ok(Some(h));
                }
            }
        }
        Ok(None)
    }

    /// `getGeriatricPercentage()`.
    pub fn geriatric_percentage(&self, ctx: AnimalCtx<'_>) -> f32 {
        let age = self.data.age as f32;
        let max = self.max_age_geriatric(ctx);
        if age < max / 10.0 * 7.0 {
            return 0.0;
        }
        let diff = age - max / 10.0 * 7.0;
        let total = max - max / 10.0 * 7.0;
        if diff > total {
            return 1.0;
        }
        diff / total
    }

    /// `getMaxAgeGeriatric()`.
    pub fn max_age_geriatric(&self, ctx: AnimalCtx<'_>) -> f32 {
        let mut base = self.adef_of(ctx).max_age_geriatric;
        let a = self
            .used_gene("lifeExpectancy")
            .map(|g| g.current_value)
            .unwrap_or(1.0);
        if self.has_genetic_disorder("poorlife") {
            base /= 3;
        }
        if self.has_genetic_disorder("dwarf") {
            base /= 2;
        }
        let modifier = 0.25f32 - a / 4.0 + 1.0;
        (base as f32 * modifier) as i32 as f32
    }

    fn size_bound(&self, base: f32, ctx: AnimalCtx<'_>) -> JResult<f32> {
        let Some(g) = self.used_gene("maxSize") else {
            return Ok(base);
        };
        let mut base = base;
        if self.has_genetic_disorder("dwarf") {
            base = if self.is_baby()? {
                base / 1.5
            } else {
                base / 2.2
            };
        }
        let _ = ctx;
        Ok(base * g.current_value)
    }

    /// `getMinSize()`.
    pub fn min_size(&self, ctx: AnimalCtx<'_>) -> JResult<f32> {
        self.size_bound(self.adef_of(ctx).min_size, ctx)
    }

    /// `getMaxSize()`.
    pub fn max_size(&self, ctx: AnimalCtx<'_>) -> JResult<f32> {
        self.size_bound(self.adef_of(ctx).max_size, ctx)
    }

    fn weight_bound(&self, base: f32) -> f32 {
        let Some(g) = self.used_gene("maxWeight") else {
            return base;
        };
        let mut base = base;
        if self.has_genetic_disorder("skinny") {
            base /= 3.0;
        }
        base * g.current_value
    }

    /// `getMinWeight()`.
    pub fn min_weight(&self, ctx: AnimalCtx<'_>) -> f32 {
        self.weight_bound(self.adef_of(ctx).min_weight)
    }

    /// `getMaxWeight()`.
    pub fn max_weight(&self, ctx: AnimalCtx<'_>) -> f32 {
        self.weight_bound(self.adef_of(ctx).max_weight)
    }

    /// `setWeight(weight)`: clamped to the genome's range.
    pub fn data_set_weight(&mut self, weight: f32, ctx: AnimalCtx<'_>) {
        self.data.weight = fmin(self.max_weight(ctx), fmax(self.min_weight(ctx), weight));
    }

    /// `getTimeBeforeNextPregnancy()`.
    pub fn data_time_before_next_pregnancy(&self, ctx: AnimalCtx<'_>) -> i32 {
        let mut t = self.adef_of(ctx).time_before_next_pregnancy;
        if t > 0 {
            let m = speed(
                ctx.sandbox.animal_pregnancy_time,
                [0.01, 0.2, 0.7, 2.0, 3.0],
            );
            t = (t as f32 * m) as i32;
        }
        t
    }

    /// `getLastPregnancyPeriod()`: the days left before the next pregnancy.
    pub fn data_last_pregnancy_period(&self, ctx: AnimalCtx<'_>) -> Option<String> {
        let t = self.data_time_before_next_pregnancy(ctx);
        if t > 0 && self.data.last_pregnancy_time > 0 {
            let days = ((self.data.last_pregnancy_time + t as i64 * ONE_DAY_MILLISECONDS
                - ctx.clock.calendar_millis())
                / ONE_DAY_MILLISECONDS) as i32;
            if days > 0 {
                return Some(days.to_string());
            }
        }
        None
    }

    /// `updateLastPregnancyTime()`.
    pub fn data_update_last_pregnancy_time(&mut self, ctx: AnimalCtx<'_>) {
        self.data.last_pregnancy_time = ctx.clock.calendar_millis();
    }

    /// `getLastImpregnatePeriod(realCal)`.
    pub fn data_last_impregnate_period(
        &self,
        chr: &dyn AnimalCharacter,
        ctx: AnimalCtx<'_>,
    ) -> i32 {
        if self.adef_of(ctx).min_age_for_baby == 0
            || self.days_survived(chr) < self.min_age_for_baby(ctx)
        {
            return -1;
        }
        self.data.last_impregnate_time
    }

    /// `getLastTimeMilkedInHour()` (in days, despite the name).
    pub fn data_last_time_milked_in_hour(&self, ctx: AnimalCtx<'_>) -> f32 {
        (ctx.clock.calendar_millis() - self.data.last_milk_timer) as f32 / 3_600_000.0 / 24.0
    }

    /// `updateLastTimeMilked()`.
    pub fn data_update_last_time_milked(&mut self, ctx: AnimalCtx<'_>) {
        self.data.last_milk_timer = ctx.clock.calendar_millis();
    }

    /// `isInLayingEggPeriod(cal)`.
    pub fn data_is_in_laying_egg_period(&self, cal_millis: i64, ctx: AnimalCtx<'_>) -> bool {
        if !ctx.sandbox.animal_mating_season {
            return true;
        }
        let start = self.adef_of(ctx).lay_egg_period_start;
        start > -1 && ctx.clock.calendar_month_hour(cal_millis).0 + 1 == start
    }

    /// `haveLayingEggPeriod()`.
    pub fn data_have_laying_egg_period(&self, ctx: AnimalCtx<'_>) -> bool {
        ctx.sandbox.animal_mating_season && self.adef_of(ctx).lay_egg_period_start > -1
    }

    /// `getInventoryIconTextureName()`.
    pub fn data_inventory_icon_texture_name(&self, chr: &dyn AnimalCharacter) -> JResult<Option<String>> {
        // isBaby precedes every getBreed/dereference in the original.
        let baby = self.is_baby()?;
        if baby {
            return Ok(self.breed().ok_or_else(|| JavaException("NullPointerException: breed".into()))?.inv_icon_baby.clone());
        }
        if chr.is_female() {
            return Ok(self.breed().ok_or_else(|| JavaException("NullPointerException: breed".into()))?.inv_icon_female.clone());
        }
        Ok(self.breed().ok_or_else(|| JavaException("NullPointerException: breed".into()))?.inv_icon_male.clone())
    }

    /// `getAgeString(chr)`.
    pub fn data_age_string(&self) -> &'static str {
        "Adult"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::env::{AnimalSandbox, CharacterValues, NoWorld};
    use crate::{AnimalClock, AnimalDefinitionTables};
    use pz_world::save::animal::CharacterStatDecl;
    use std::rc::Rc;
    use std::sync::atomic::{AtomicU32, Ordering};

    static COUNTER: AtomicU32 = AtomicU32::new(0);

    /// A linear congruential generator (test values only).
    struct Lcg(u64);
    impl GameRand for Lcg {
        fn next(&mut self, max: i32) -> i32 {
            if max <= 0 {
                return 0;
            }
            self.0 = self
                .0
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            ((self.0 >> 33) % max as u64) as i32
        }
        fn next_float(&mut self) -> f32 {
            self.next(1 << 24) as f32 / (1 << 24) as f32
        }
    }

    struct Clock(i32);
    impl AnimalClock for Clock {
        fn hour(&self) -> i32 {
            self.0 % 24
        }
        fn month(&self) -> i32 {
            5
        }
        fn calendar_millis(&self) -> i64 {
            self.0 as i64 * 3_600_000
        }
        fn multiplier(&self) -> f32 {
            1.0
        }
        fn world_age_days_since_begin(&self) -> f64 {
            1.0
        }
    }

    /// Made-up definitions in a scratch game directory (unique per call).
    fn tables() -> AnimalDefinitionTables {
        let n = COUNTER.fetch_add(1, Ordering::SeqCst);
        let dir =
            std::env::temp_dir().join(format!("pz-animals-data-test-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let d = dir.join("media/lua/shared/Definitions/animal");
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(
            d.join("T.lua"),
            r#"
AnimalGenomeDefinitions = { genes = { fertility = { minValue = 0.9, maxValue = 0.95, forcedValues = true } },
                            geneticDisorder = { none = true } }
AnimalDefinitions = { animals = {
  doe = { female = true, minAge = 10, minSize = 0.5, maxSize = 1.0, minWeight = 10, maxWeight = 20,
          hungerMultiplier = 0.001, thirstMultiplier = 0.002, babyType = "kid", babyNbr = "2,2",
          pregnantPeriod = 1, udder = true, minMilk = 1, maxMilk = 4, maxAgeGeriatric = 1000,
          stages = { doe = { ageToGrow = 20 } }, breeds = { brown = { texture = "a,b" } },
          genes = { "fertility" } },
  kid = { minSize = 0.1, maxSize = 0.4, minWeight = 1, maxWeight = 5,
          stages = { kid = { ageToGrow = 5, nextStage = "doe", nextStageMale = "doe" } },
          breeds = { brown = { texture = "c" } }, genes = { "fertility" } } } }
"#,
        )
        .unwrap();
        let fs = pz_fs::GameFs::new(&dir);
        let lua = crate::lua::DefinitionsLua::load(&fs).unwrap();
        let t = AnimalDefinitionTables::load(&lua.env()).unwrap();
        let _ = std::fs::remove_dir_all(&dir);
        t
    }

    fn stat_defs() -> Rc<[CharacterStatDecl]> {
        ["HUNGER", "THIRST"]
            .iter()
            .map(|f| CharacterStatDecl {
                field: f.to_string(),
                id: f.to_string(),
                minimum: 0.0,
                maximum: 1.0,
                default: 0.0,
            })
            .collect::<Vec<_>>()
            .into()
    }

    fn sandbox() -> AnimalSandbox {
        AnimalSandbox {
            animal_stats_modifier: 4,
            animal_meta_stats_modifier: 4,
            animal_pregnancy_time: 1,
            animal_age_modifier: 6,
            animal_milk_inc_modifier: 4,
            animal_wool_inc_modifier: 4,
            animal_meta_predator: false,
            animal_mating_season: false,
            ultra_speed_doesnot_affect_to_animals: false,
        }
    }

    #[derive(Default)]
    struct Babies(Vec<NewAnimal>);
    impl crate::AnimalWorld for Babies {
        fn add_new_animal(&mut self, animal: NewAnimal, _v: Option<ObjectRef>) -> ObjectRef {
            self.0.push(animal);
            self.0.len() as ObjectRef
        }
    }

    #[test]
    fn init_hunger_pregnancy_and_birth() {
        let t = tables();
        let sb = sandbox();
        let mut rand = Lcg(7);
        let mut world = Babies::default();
        let mut hour = 100;
        let mut chr = CharacterValues::new(stat_defs());
        let mut a = Animal::new(Some("doe".into()));
        let breed = t.def("doe").unwrap().breeds[0].clone();
        {
            let clock = Clock(hour);
            let mut env = AnimalEnv {
                ctx: AnimalCtx {
                    defs: &t,
                    clock: &clock,
                    sandbox: &sb,
                },
                rand: &mut rand,
                world: &mut world,
                stat_defs: stat_defs(),
                patience_range: (20, 150),
            };
            assert!(a.init(Some(&breed), &mut chr, &mut env).unwrap());
            assert!(chr.female);
            assert_eq!(a.data.age, 10);
            assert_eq!(chr.hours_survived, 240.0);
            assert!(a.skin_texture_name.is_some());
            assert!(a.stress_level > 0.0 && a.stress_level <= 70.0);
            a.fertilize(Male::None, true, env.ctx, env.rand).unwrap();
            assert!(a.data.pregnant);
        }
        // Hunger rises each hour; a day later the pregnancy (8 days at the
        // minimum) has not ended, after 9 days two kids are born.
        for _ in 0..(24 * 9) {
            hour += 1;
            let clock = Clock(hour);
            let mut env = AnimalEnv {
                ctx: AnimalCtx {
                    defs: &t,
                    clock: &clock,
                    sandbox: &sb,
                },
                rand: &mut rand,
                world: &mut world,
                stat_defs: stat_defs(),
                patience_range: (20, 150),
            };
            a.data_update(&mut chr, &mut env).unwrap();
        }
        assert!(chr.stat(AnimalStat::Hunger) > 0.0);
        assert_eq!(world.0.len(), 2);
        assert!(!a.data.pregnant);
        assert!(a.data.can_have_milk);
        assert_eq!(a.babies.as_ref().map(Vec::len), Some(2));
        let kid = &world.0[0];
        assert_eq!(kid.animal.animal_type.as_deref(), Some("kid"));
        assert_eq!(kid.animal.mother_id, a.animal_id);
        assert_eq!(kid.animal.full_genome.len(), 1);
    }

    #[test]
    fn stress_and_milk_limits() {
        let t = tables();
        let sb = sandbox();
        let clock = Clock(5);
        let mut rand = Lcg(1);
        let mut world = NoWorld;
        let mut chr = CharacterValues::new(stat_defs());
        let mut a = Animal::new(Some("doe".into()));
        let mut env = AnimalEnv {
            ctx: AnimalCtx {
                defs: &t,
                clock: &clock,
                sandbox: &sb,
            },
            rand: &mut rand,
            world: &mut world,
            stat_defs: stat_defs(),
            patience_range: (20, 150),
        };
        a.init(None, &mut chr, &mut env).ok();
        let breed = t.def("doe").unwrap().breeds[0].clone();
        a.init(Some(&breed), &mut chr, &mut env).unwrap();
        a.change_stress(500.0);
        assert_eq!(a.stress_level, 100.0);
        a.change_stress(-500.0);
        assert_eq!(a.stress_level, 0.0);
        a.data_set_can_have_milk(true, env.ctx);
        assert_eq!(a.data.max_milk_actual, a.min_milk(env.ctx));
        a.data_set_milk_quantity(10.0, env.ctx, env.rand);
        assert_eq!(a.data.milk_qty, a.data.max_milk_actual);
        let before = a.data.max_milk_actual;
        a.data_set_milk_quantity(0.0, env.ctx, env.rand);
        assert!(a.data.max_milk_actual > before);
        assert_eq!(a.data.last_milk_timer, clock.calendar_millis());
    }

    #[test]
    fn missing_stage_stops_hour_before_weight_loss_after_completed_prefix() {
        let t = tables(); let sb = sandbox(); let clock = Clock(5);
        let mut rand = Lcg(1); let mut world = NoWorld;
        let mut chr = CharacterValues::new(stat_defs());
        chr.set_stat(AnimalStat::Hunger, 0.9);
        let mut a = Animal::new(Some("doe".into()));
        a.wild = true; a.data.weight = 12.0; a.data.last_impregnate_time = 2;
        a.ignored_trough.push(44);
        let mut env = AnimalEnv { ctx: AnimalCtx { defs: &t, clock: &clock, sandbox: &sb },
            rand: &mut rand, world: &mut world, stat_defs: stat_defs(), patience_range: (20,150) };
        let error = a.data_hour_grow(false, &mut chr, &mut env).unwrap_err();
        assert_eq!(error, JavaException("NullPointerException: currentStage".into()));
        assert!(a.ignored_trough.is_empty());
        assert_eq!(a.data.last_impregnate_time, 1);
        assert_eq!(a.data.weight, 12.0, "weight suffix must not execute");
    }

    #[test]
    fn dwarf_size_failure_preserves_setter_and_actual_gene_absence_branch() {
        let t = tables(); let sb = sandbox(); let clock = Clock(5);
        let ctx = AnimalCtx { defs: &t, clock: &clock, sandbox: &sb };
        let mut a = Animal::new(Some("doe".into()));
        a.genetic_disorder.push("dwarf".into());
        // No maxSize gene means the original never calls isBaby.
        assert_eq!(a.min_size(ctx).unwrap(), t.def("doe").unwrap().min_size);
        let mut gene = AnimalGene::new(&mut Lcg(1));
        gene.allele1.used = true; gene.allele1.current_value = 1.0;
        a.full_genome.insert("maxsize".into(), gene);
        assert_eq!(a.used_gene("maxSize").unwrap().current_value, 1.0, "admit the actual lowercased gene lookup");
        assert!(a.data.current_stage.is_none(), "exercise the deliberately missing stage");
        a.data.size = 0.75;
        assert_eq!(a.data_set_size(0.5, ctx).unwrap_err(), JavaException("NullPointerException: currentStage".into()));
        assert_eq!(a.data.size, 0.75, "setter must not publish a failed bound");
        assert_eq!(a.max_size(ctx).unwrap_err(), JavaException("NullPointerException: currentStage".into()));
    }

    #[test]
    fn inventory_icon_checks_stage_then_selected_breed_and_preserves_null_icon() {
        let mut a = Animal::new(None);
        let chr = CharacterValues::new(stat_defs());
        assert_eq!(a.data_inventory_icon_texture_name(&chr).unwrap_err(), JavaException("NullPointerException: currentStage".into()));
        let t = tables();
        a.data.current_stage = Some(t.def("doe").unwrap().stages[0].clone());
        assert_eq!(a.data_inventory_icon_texture_name(&chr).unwrap_err(), JavaException("NullPointerException: breed".into()));
        a.data.breed = Some(t.def("doe").unwrap().breeds[0].clone());
        assert_eq!(a.data_inventory_icon_texture_name(&chr).unwrap(), None);
    }

    #[test]
    fn hutch_stage_failure_keeps_door_prefix_without_timer_or_action_suffix() {
        use std::cell::Cell;
        struct Hutch { closed: Cell<bool>, toggles: Cell<u32> }
        impl crate::AnimalWorld for Hutch {
            fn hutch_inside_count(&self, _: ObjectRef) -> i32 { 0 }
            fn hutch_max_animals(&self, _: ObjectRef) -> i32 { 1 }
            fn hutch_door_closed(&self, _: ObjectRef) -> bool { self.closed.get() }
            fn hutch_toggle_door(&mut self, _: ObjectRef) {
                self.closed.set(!self.closed.get()); self.toggles.set(self.toggles.get()+1);
            }
        }
        let t = tables(); let sb = sandbox(); let clock = Clock(5);
        let mut rand = Lcg(7); let before_rand = rand.0;
        let mut world = Hutch { closed: Cell::new(true), toggles: Cell::new(0) };
        let mut chr = CharacterValues::new(stat_defs()); let mut a = Animal::new(Some("doe".into()));
        let mut env = AnimalEnv { ctx: AnimalCtx { defs: &t, clock: &clock, sandbox: &sb },
            rand: &mut rand, world: &mut world, stat_defs: stat_defs(), patience_range: (20,150) };
        assert_eq!(a.behavior_can_go_to_hutch(1, true, &mut chr, &mut env).unwrap_err(), JavaException("NullPointerException: currentStage".into()));
        assert_eq!(a.behavior.hutch_path_timer, -1.0);
        assert!(a.behavior.behavior_action.is_none() && !a.behavior.is_doing_behavior);
        assert_eq!(rand.0, before_rand);
        assert!(!world.closed.get()); assert_eq!(world.toggles.get(), 1);
    }

    #[test]
    fn region_hutch_only_dereferences_nullable_types_after_a_real_hutch() {
        struct Hutch;
        impl crate::AnimalWorld for Hutch {
            fn zone_hutches(&self, _: ObjectRef) -> Vec<(ObjectRef,String)> { vec![(1,"type".into())] }
        }
        let t = tables(); let sb = sandbox(); let clock = Clock(5); let mut rand = Lcg(7); let mut world = Hutch;
        let mut a = Animal::new(Some("doe".into()));
        let env = AnimalEnv { ctx: AnimalCtx { defs: &t, clock: &clock, sandbox: &sb },
            rand: &mut rand, world: &mut world, stat_defs: stat_defs(), patience_range: (20,150) };
        assert_eq!(a.data_region_hutch(env.ctx, &env).unwrap(), None);
        a.connected_d_zone.push(4);
        assert_eq!(a.data_region_hutch(env.ctx, &env).unwrap_err(), JavaException("NullPointerException: adef.hutches".into()));
    }

}
