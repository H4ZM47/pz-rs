//! The animal logic of `zombie.characters.animals.IsoAnimal` that does not
//! need the character runtime: the animal's own fields, `init(breed)`,
//! stress and acceptance, fertilisation, babies, eggs, the meta update
//! while the animal was away, and the meta predator. The character side
//! (`IsoGameCharacter`/`IsoPlayer` state) is reached through
//! [`AnimalCharacter`], the world through [`AnimalWorld`].

use crate::breed::AnimalBreed;
use crate::data::AnimalData;
use crate::definitions::AnimalDefinitions;
use crate::env::{
    AnimalCharacter, AnimalCtx, AnimalEnv, AnimalStat, CharacterValues, EggHatch, ObjectRef,
};
use crate::genome::{self, AnimalAllele, AnimalGene};
use crate::java::{fmin, is_null_or_empty, npe, JHashMap, JResult, JavaException};
use pz_world::save::lua_table::{LuaKey, LuaTable, LuaValue};
use pz_world::GameRand;

/// `Rand.NextBool(n)`.
pub(crate) fn next_bool(rand: &mut dyn GameRand, n: i32) -> bool {
    rand.next(n) == 0
}

/// The animal's `IsoAnimal` fields that are animal logic (the rendering,
/// sound, behaviour-state and network fields stay with the character side).
#[derive(Clone, Debug, PartialEq)]
pub struct Animal {
    /// `type`.
    pub animal_type: Option<String>,
    /// `animalId`.
    pub animal_id: i32,
    pub item_id: i32,
    pub data: AnimalData,
    pub attacked_timer: i64,
    pub invincible: bool,
    pub attach_back_to_mother: i32,
    pub attach_back_to_tree_x: i32,
    pub attach_back_to_tree_y: i32,
    pub time_since_last_update: i64,
    pub custom_name: Option<String>,
    pub small_enclosure: bool,
    pub mother: Option<ObjectRef>,
    pub mother_id: i32,
    pub search_radius: i32,
    pub milk_removed: i32,
    pub eat_from_trough: Option<ObjectRef>,
    /// An `IsoWorldInventoryObject`.
    pub eat_from_ground: Option<ObjectRef>,
    pub drink_from_trough: Option<ObjectRef>,
    pub drink_from_river: Option<ObjectRef>,
    pub drink_from_puddle: Option<ObjectRef>,
    pub hutch: Option<ObjectRef>,
    pub full_genome: JHashMap<AnimalGene>,
    pub stress_level: f32,
    /// `getAnimalVisual().getSkinTextureName()`.
    pub skin_texture_name: Option<String>,
    /// `animalZone.id` (most and least significant bits).
    pub animal_zone: Option<(i64, i64)>,
    pub move_forward_on_zone: bool,
    pub egg_timer_in_hutch: i32,
    pub nest_box: i32,
    /// `playerAcceptanceList` (`HashMap<Short, Float>`), in insertion
    /// order; [`Animal::player_acceptance_ordered`] gives the iteration
    /// order.
    pub player_acceptance: Vec<(i16, f32)>,
    /// `heldBy`: the holding player's online id and husbandry level.
    pub held_by: Option<(i16, i32)>,
    pub genetic_disorder: Vec<String>,
    pub pet_timer: f32,
    pub d_zone: Option<ObjectRef>,
    pub connected_d_zone: Vec<ObjectRef>,
    pub zone_check_timer: f32,
    pub moving_to_food: Option<ObjectRef>,
    pub moving_to_food_timer: f32,
    pub ignored_trough: Vec<ObjectRef>,
    pub attach_back_to_mother_timer: f32,
    pub virtual_id: f64,
    pub migration_group: Option<String>,
    pub wild: bool,
    pub alerted: bool,
    pub from_meta: bool,
    pub thump_delay: f32,
    pub should_be_skeleton: bool,
    pub babies: Option<Vec<ObjectRef>>,
    pub zone_acceptance: f32,
    pub following_wall: bool,
    pub should_follow_wall: bool,
    pub on_hook: bool,
    /// The hook's square while on a hook.
    pub hook: Option<(i32, i32, i32)>,
    pub attach_back_to_hook_x: i32,
    pub attach_back_to_hook_y: i32,
    pub attach_back_to_hook_z: i32,
    pub road_kill: bool,
    pub last_cell_saved_to_x: i32,
    pub last_cell_saved_to_y: i32,
    /// `getModData()`.
    pub mod_data: LuaTable,
    /// `getBehavior()`: the behaviour's state.
    pub behavior: crate::behavior::AnimalBehavior,
    /// This animal's own handle in the [`AnimalWorld`](crate::env::AnimalWorld).
    pub self_ref: ObjectRef,
}

/// A newly constructed animal and its character values, before the world
/// takes it.
#[derive(Clone, Debug, PartialEq)]
pub struct NewAnimal {
    pub animal: Animal,
    pub chr: CharacterValues,
}

impl Animal {
    /// The fields of `new IsoAnimal(cell)` before `init`.
    pub fn new(animal_type: Option<String>) -> Animal {
        Animal {
            animal_type,
            animal_id: -1,
            item_id: 0,
            data: AnimalData::new(None),
            attacked_timer: -1,
            invincible: false,
            attach_back_to_mother: 0,
            attach_back_to_tree_x: i32::MAX,
            attach_back_to_tree_y: i32::MAX,
            time_since_last_update: -1,
            custom_name: None,
            small_enclosure: false,
            mother: None,
            mother_id: 0,
            search_radius: 3,
            milk_removed: 0,
            eat_from_trough: None,
            eat_from_ground: None,
            drink_from_trough: None,
            drink_from_river: None,
            drink_from_puddle: None,
            hutch: None,
            full_genome: JHashMap::new(),
            stress_level: 0.0,
            skin_texture_name: None,
            animal_zone: None,
            move_forward_on_zone: false,
            egg_timer_in_hutch: 0,
            nest_box: -1,
            player_acceptance: Vec::new(),
            held_by: None,
            genetic_disorder: Vec::new(),
            pet_timer: 0.0,
            d_zone: None,
            connected_d_zone: Vec::new(),
            zone_check_timer: 0.0,
            moving_to_food: None,
            moving_to_food_timer: 0.0,
            ignored_trough: Vec::new(),
            attach_back_to_mother_timer: 0.0,
            virtual_id: 0.0,
            migration_group: None,
            wild: false,
            alerted: false,
            from_meta: false,
            thump_delay: 20000.0,
            should_be_skeleton: false,
            babies: None,
            zone_acceptance: 0.0,
            following_wall: false,
            should_follow_wall: false,
            on_hook: false,
            hook: None,
            attach_back_to_hook_x: 0,
            attach_back_to_hook_y: 0,
            attach_back_to_hook_z: 0,
            road_kill: false,
            last_cell_saved_to_x: i32::MIN,
            last_cell_saved_to_y: i32::MIN,
            mod_data: LuaTable::default(),
            behavior: crate::behavior::AnimalBehavior::default(),
            self_ref: 0,
        }
    }

    /// `adef`. Animals only exist for known types (`init` returns early
    /// otherwise and leaves the animal without data).
    pub fn adef<'d>(&self, ctx: AnimalCtx<'d>) -> &'d AnimalDefinitions {
        ctx.def(self.animal_type.as_deref().unwrap_or(""))
            .unwrap_or_else(|| panic!("animal {:?} has no definition", self.animal_type))
    }

    /// `getBreed()`.
    pub fn breed(&self) -> Option<&AnimalBreed> {
        self.data.breed.as_ref()
    }

    /// `getUsedGene(name)`.
    pub fn used_gene(&self, name: &str) -> Option<&AnimalAllele> {
        self.full_genome
            .get(&name.to_lowercase())
            .map(AnimalGene::used_gene)
    }

    fn used_gene_value(&self, name: &str) -> Option<f32> {
        self.used_gene(name).map(|a| a.current_value)
    }

    /// `geneticDisorder.contains(d)`.
    pub fn has_genetic_disorder(&self, d: &str) -> bool {
        self.genetic_disorder.iter().any(|x| x == d)
    }

    /// `isBaby()`: the current stage has a next stage.
    pub fn is_baby(&self) -> JResult<bool> {
        match &self.data.current_stage {
            Some(s) => Ok(s.next_stage.is_some()),
            None => npe("currentStage"),
        }
    }

    /// `getAge()`.
    pub fn age(&self) -> i32 {
        self.data.age
    }

    /// `isWild()`.
    pub fn is_wild(&self) -> bool {
        self.wild
    }

    /// `isAnimalSitting()`.
    pub fn is_animal_sitting(chr: &dyn AnimalCharacter) -> bool {
        chr.variable("idleAction") == "sit"
    }

    /// `isAnimalEating()`.
    pub fn is_animal_eating(chr: &dyn AnimalCharacter) -> bool {
        chr.variable("idleAction") == "eat"
    }

    /// `getFullGenomeList()`: the genes in `HashMap` order.
    pub fn full_genome_list(&self) -> Vec<&AnimalGene> {
        self.full_genome.values().collect()
    }

    /// `AnimalGene.checkGeneticDisorder(this)`.
    pub fn check_genetic_disorder(&mut self) {
        let list: Vec<AnimalGene> = self.full_genome.values().cloned().collect();
        let refs: Vec<&AnimalGene> = list.iter().collect();
        genome::check_genetic_disorder(&refs, &mut self.genetic_disorder);
    }

    /// `setHealth(health)`: an invincible animal does not lose health.
    pub fn set_health(&self, chr: &mut dyn AnimalCharacter, health: f32) {
        if self.invincible && health < chr.health() {
            return;
        }
        chr.set_health_field(health);
    }

    /// `setDebugStress(stress)`.
    pub fn set_debug_stress(&mut self, stress: f32) {
        self.stress_level = stress;
    }

    /// `setWild(b)`.
    pub fn set_wild(&mut self, b: bool, ctx: AnimalCtx<'_>, rand: &mut dyn GameRand) {
        if !b && !self.adef(ctx).can_be_domesticated {
            return;
        }
        if !b {
            self.migration_group = None;
            self.virtual_id = 0.0;
        } else {
            let s = rand.next_float_range(50.0, 80.0);
            self.set_debug_stress(s);
        }
        self.wild = b;
    }

    // ---------------------------------------------------------------
    // init

    /// `init(breed)`. The character parts of it (animation set, AI states,
    /// sprite parts, turn delta) are the character side's; this does the
    /// rest, in the original's order and with its `Rand` draws. Returns
    /// `Ok(false)` when the type has no definition (the original returns
    /// early and leaves the animal without data).
    pub fn init(
        &mut self,
        breed: Option<&AnimalBreed>,
        chr: &mut dyn AnimalCharacter,
        env: &mut AnimalEnv<'_>,
    ) -> JResult<bool> {
        let ctx = env.ctx;
        let Some(adef) = ctx.def(self.animal_type.as_deref().unwrap_or("")) else {
            return Ok(false);
        };
        // initType: new BaseAnimalBehavior, new AnimalData(this, breed).
        self.behavior = crate::behavior::AnimalBehavior::default();
        self.data = AnimalData::construct(adef, breed, chr, env.rand)?;
        self.animal_id = env.rand.next(10000);
        // initAge.
        self.data.age = 1;
        if adef.min_age > 0 {
            self.data.age = adef.min_age;
        }
        chr.set_hours_survived((self.data.age * 24) as f64);
        let breed = self
            .data
            .breed
            .clone()
            .ok_or_else(|| JavaException("NullPointerException: breed".into()))?;
        if let Some(g) = genome::init_genome(&ctx.defs.genome, adef, &breed, env.rand)? {
            self.full_genome = g;
        }
        self.check_genetic_disorder();
        self.data_init(chr, ctx)?;
        self.init_texture(chr, ctx, env.rand)?;
        let w = self.pick_random_wander_interval(env)?;
        chr.set_state_event_delay_timer(w);
        self.init_stress(env)?;
        self.set_wild(adef.wild, ctx, env.rand);
        Ok(true)
    }

    /// `new IsoAnimal(cell, x, y, z, type, breed)`: the character's
    /// construction draws, `init(breed)` and
    /// `setDir(IsoDirections.getRandom())` on a fresh character at the
    /// position.
    pub fn construct(
        animal_type: &str,
        breed: Option<&AnimalBreed>,
        pos: (f32, f32, f32),
        env: &mut AnimalEnv<'_>,
    ) -> JResult<NewAnimal> {
        let mut chr = CharacterValues::new(env.stat_defs.clone());
        let (pmin, pmax) = env.patience_range;
        chr.construction = crate::env::character_construction_draws(env.rand, pmin, pmax);
        chr.x = pos.0;
        chr.y = pos.1;
        chr.z = pos.2;
        let mut animal = Animal::new(Some(animal_type.to_string()));
        animal.init(breed, &mut chr, env)?;
        chr.dir = env.rand.next(8);
        Ok(NewAnimal { animal, chr })
    }

    fn init_texture(
        &mut self,
        chr: &dyn AnimalCharacter,
        ctx: AnimalCtx<'_>,
        rand: &mut dyn GameRand,
    ) -> JResult<()> {
        let adef = self.adef(ctx);
        if self.should_be_skeleton {
            self.skin_texture_name = adef.texture_skeleton.clone();
            return Ok(());
        }
        let skinned = matches!(
            self.mod_data.rawget(&LuaKey::Str("skinned".into())),
            Some(LuaValue::Bool(true))
        );
        if !is_null_or_empty(adef.texture_skinned.as_deref()) && skinned {
            self.skin_texture_name = adef.texture_skinned.clone();
            return Ok(());
        }
        let stage = match &self.data.current_stage {
            Some(s) => s,
            None => return npe("currentStage"),
        };
        let breed = match &self.data.breed {
            Some(b) => b,
            None => return npe("breed"),
        };
        if !is_null_or_empty(stage.next_stage.as_deref())
            && !is_null_or_empty(breed.texture_baby.as_deref())
        {
            self.skin_texture_name = breed.texture_baby.clone();
        } else if chr.is_female() {
            let Some(t) = &breed.texture else {
                return npe("texture");
            };
            let i = rand.next_range(0, t.len() as i32);
            self.skin_texture_name = Some(t.get(i as usize).cloned().ok_or_else(|| {
                JavaException(format!(
                    "IndexOutOfBoundsException: Index {i} out of bounds for length {}",
                    t.len()
                ))
            })?);
        } else {
            self.skin_texture_name = breed.texture_male.clone();
        }
        Ok(())
    }

    /// `BaseAnimalBehavior.pickRandomWanderInterval()`.
    pub fn pick_random_wander_interval(&self, env: &mut AnimalEnv<'_>) -> JResult<f32> {
        let adef = self.adef(env.ctx);
        let raining = env.world.is_raining();
        Ok(self.wander_interval(
            adef,
            env.ctx.sandbox.ultra_speed_doesnot_affect_to_animals,
            env.ctx.clock.multiplier(),
            raining,
            env.rand,
        ))
    }

    /// `pickRandomWanderInterval()` with what it reads given: the
    /// definition, `ServerOptions.ultraSpeedDoesnotAffectToAnimals`, the
    /// multiplier and `RainManager.isRaining()`.
    pub fn wander_interval(
        &self,
        adef: &AnimalDefinitions,
        ultra_speed_does_not_affect: bool,
        multiplier: f32,
        raining: bool,
        rand: &mut dyn GameRand,
    ) -> f32 {
        let base = adef.wander_mul - self.behavior.wander_mul_mod;
        let mut interval = rand.next_float_range(base, base * 5.0);
        if ultra_speed_does_not_affect && multiplier > 60.0 {
            interval *= 10.0;
        }
        if self.has_genetic_disorder("fidget") {
            interval /= 10.0;
        }
        if raining {
            interval *= 3.0;
        }
        interval
    }

    /// `initStress()`.
    fn init_stress(&mut self, env: &mut AnimalEnv<'_>) -> JResult<()> {
        let allele = self.used_gene_value("stress");
        let mut base = env.rand.next_float_range(0.01, 0.15);
        if allele.is_none() {
            self.stress_level = base;
        }
        let mut m = 1.0f32;
        if let Some(v) = allele {
            m = 1.0 - v + 1.0;
        }
        if !self.is_baby()? {
            base += (env.ctx.clock.world_age_days_since_begin() * 0.005f32 as f64) as f32;
        }
        self.stress_level = m * base * 100.0;
        self.stress_level = fmin(70.0, self.stress_level);
        Ok(())
    }

    // ---------------------------------------------------------------
    // stress and acceptance

    /// `changeStress(inc)`.
    pub fn change_stress(&mut self, mut inc: f32) {
        if self.invincible && inc < 0.0 {
            return;
        }
        if let Some(v) = self.used_gene_value("stress") {
            inc = if inc > 0.0 { inc * (1.0 + v) } else { inc * v };
        }
        if self.has_genetic_disorder("highstress") {
            inc = if inc < 0.0 { inc * 10.0 } else { inc / 10.0 };
        }
        self.stress_level += inc;
        self.stress_level = fmin(100.0, crate::java::fmax(0.0, self.stress_level));
    }

    /// `shouldAnimalStressAboveGround()`.
    pub fn should_animal_stress_above_ground(
        &self,
        chr: &dyn AnimalCharacter,
        ctx: AnimalCtx<'_>,
    ) -> bool {
        self.adef(ctx).stress_above_ground && chr.position().2 > 0.0
    }

    /// `updateStress()`. `vehicle_speed_kmh` is
    /// `getVehicle().getCurrentSpeedKmHour()` when in a vehicle, and
    /// `under_rain` whether the animal stands on a square without a roof
    /// (`getSquare() != null && !getSquare().haveRoof`).
    pub fn update_stress(
        &mut self,
        chr: &dyn AnimalCharacter,
        ctx: AnimalCtx<'_>,
        rand: &mut dyn GameRand,
        raining: bool,
        vehicle_speed_kmh: Option<f32>,
        under_rain: bool,
    ) {
        if self.wild {
            return;
        }
        let mult = ctx.clock.multiplier();
        let mut inc_stress = true;
        if let Some((id, husbandry)) = self.held_by {
            self.change_stress(-(mult / 3000.0));
            self.add_acceptance(id, husbandry, mult / 10000.0, rand);
        }
        if vehicle_speed_kmh.is_some_and(|s| s > 5.0) {
            self.change_stress(mult / 50000.0);
            inc_stress = false;
        }
        if chr.stat(AnimalStat::Hunger) > 0.8 || chr.stat(AnimalStat::Thirst) > 0.8 {
            self.change_stress(mult / 50000.0);
            inc_stress = false;
        }
        if self.should_animal_stress_above_ground(chr, ctx) {
            self.change_stress(mult / 10000.0);
            inc_stress = false;
        }
        if self.adef(ctx).stress_under_rain && raining && under_rain {
            self.change_stress(mult / 40000.0);
            inc_stress = false;
        }
        if inc_stress {
            self.change_stress(-(mult / 5500.0));
        }
    }

    /// `playerAcceptanceList` in its `HashMap<Short, Float>` iteration
    /// order.
    pub fn player_acceptance_ordered(&self) -> Vec<(i16, f32)> {
        let mut v = self.player_acceptance.clone();
        pz_inventory::java::sort_int_hash_map(&mut v, |e| e.0 as i32);
        v
    }

    fn acceptance_put(&mut self, id: i16, v: f32) {
        match self.player_acceptance.iter_mut().find(|e| e.0 == id) {
            Some(e) => e.1 = v,
            None => self.player_acceptance.push((id, v)),
        }
    }

    fn acceptance_get(&self, id: i16) -> Option<f32> {
        self.player_acceptance
            .iter()
            .find(|e| e.0 == id)
            .map(|e| e.1)
    }

    /// `getAcceptanceLevel(chr)` for the player with online id `id`: a new
    /// player starts at `Rand.Next(0, 20)`.
    pub fn acceptance_level(&mut self, id: i16, rand: &mut dyn GameRand) -> f32 {
        if self.acceptance_get(id).is_none() {
            let v = rand.next_float_range(0.0, 20.0);
            self.acceptance_put(id, v);
        }
        self.acceptance_get(id).unwrap_or(0.0)
    }

    /// `getPlayerAcceptance(chr)`.
    pub fn player_acceptance_of(&mut self, id: i16, rand: &mut dyn GameRand) -> f32 {
        if self.player_acceptance.is_empty() {
            return 0.0;
        }
        let v = self.acceptance_level(id, rand);
        // Math.round(float) returns an int.
        java_round(v) as f32
    }

    /// `addAcceptance(chr, acceptance)` for a player with online id `id`
    /// and husbandry level `husbandry`.
    pub fn add_acceptance(
        &mut self,
        id: i16,
        husbandry: i32,
        acceptance: f32,
        rand: &mut dyn GameRand,
    ) {
        let mut a = self.acceptance_level(id, rand);
        a += acceptance;
        a *= 1.0 + husbandry as f32 / 10.0;
        if a > 100.0 {
            a = 100.0;
        }
        self.acceptance_put(id, a);
    }

    /// `setDebugAcceptance(chr, acceptance)`.
    pub fn set_debug_acceptance(&mut self, id: i16, acceptance: f32) {
        self.acceptance_put(id, acceptance);
    }

    /// `petAnimal(chr)` (the animal's part; the player's own `petAnimal`
    /// and the XP are the character side's). `client` is
    /// `GameClient.client`.
    pub fn pet_animal(&mut self, id: i16, husbandry: i32, client: bool, rand: &mut dyn GameRand) {
        if !client && self.pet_timer == 0.0 {
            let s = (rand.next_range(-11, -7) - husbandry) as f32;
            self.change_stress(s);
            let a = fmin(
                100.0,
                self.acceptance_level(id, rand) + rand.next_range(2, 7) as f32,
            );
            self.set_debug_acceptance(id, a);
            self.pet_timer = (rand.next_range(15000, 25000) - husbandry * 200) as f32;
            // XP: Rand.Next(5, 10).
            rand.next_range(5, 10);
        }
    }

    /// `petTimerDone()`.
    pub fn pet_timer_done(&self) -> bool {
        self.pet_timer <= 0.0
    }

    /// `updateZoneAcceptance()`.
    pub fn update_zone_acceptance(&mut self, ctx: AnimalCtx<'_>) {
        if !self.connected_d_zone.is_empty() && self.zone_acceptance < 100.0 {
            self.zone_acceptance += ctx.clock.multiplier() / 1000.0;
            if self.zone_acceptance > 100.0 {
                self.zone_acceptance = 100.0;
            }
        }
    }

    /// `checkZone()`.
    pub fn check_zone(&mut self, chr: &dyn AnimalCharacter, env: &mut AnimalEnv<'_>) {
        if self.zone_check_timer > 0.0 {
            self.zone_check_timer -= env.ctx.clock.multiplier();
            return;
        }
        self.zone_check_timer = 2000.0;
        let (x, y, z) = chr.position();
        let current = env.world.zone_at(x, y, z);
        // setDZone.
        env.world.move_animal_to_zone(self.d_zone, current);
        self.d_zone = current;
        self.connected_d_zone = env.world.connected_zones(self.d_zone);
        if !self.connected_d_zone.is_empty() {
            self.set_wild(false, env.ctx, env.rand);
        }
        self.update_zone_acceptance(env.ctx);
    }

    /// `getRandomSquareInZone()`.
    pub fn random_square_in_zone(&self, env: &mut AnimalEnv<'_>) -> Option<ObjectRef> {
        if self.connected_d_zone.is_empty() {
            return None;
        }
        let i = env.rand.next_range(0, self.connected_d_zone.len() as i32);
        let zone = self.connected_d_zone[i as usize];
        env.world.zone_random_square(zone, env.rand)
    }

    // ---------------------------------------------------------------
    // ages, seasons

    /// `isGeriatric()`.
    pub fn is_geriatric(&self, chr: &dyn AnimalCharacter, ctx: AnimalCtx<'_>) -> bool {
        let adef = self.adef(ctx);
        if adef.max_age_geriatric <= 0 {
            return false;
        }
        let delta = self.max_age_geriatric(ctx) - adef.min_age as f32;
        self.days_survived(chr) as f64 > delta as f64 * 0.8
    }

    /// `haveMatingSeason()`.
    pub fn have_mating_season(&self, ctx: AnimalCtx<'_>) -> bool {
        let a = self.adef(ctx);
        a.mating_period_start != 0 && a.mating_period_end != 0
    }

    /// `isInMatingSeason()`.
    pub fn is_in_mating_season(&self, ctx: AnimalCtx<'_>) -> bool {
        Self::in_mating_season(self.adef(ctx), ctx.sandbox.animal_mating_season, ctx.clock.month())
    }

    /// `isInMatingSeason()` with the definition, the sandbox's
    /// `AnimalMatingSeason` and `getMonth()` (0-based).
    pub fn in_mating_season(a: &AnimalDefinitions, mating_season: bool, month: i32) -> bool {
        if !mating_season || !(a.mating_period_start != 0 && a.mating_period_end != 0) {
            return true;
        }
        let m = month + 1;
        let (start, end) = (a.mating_period_start, a.mating_period_end);
        if start < end {
            return m >= start && m < end;
        }
        m < end || m >= start
    }

    /// `getMinAgeForBaby()`.
    pub fn min_age_for_baby(&self, ctx: AnimalCtx<'_>) -> i32 {
        self.min_age_for_baby_of(self.adef(ctx))
    }

    /// `getMinAgeForBaby()` with the animal's definition.
    pub fn min_age_for_baby_of(&self, adef: &AnimalDefinitions) -> i32 {
        let a = self.used_gene_value("ageToGrow").unwrap_or(1.0);
        let base = adef.min_age_for_baby;
        let modifier = 0.25f32 - a / 4.0 + 1.0;
        (base as f32 * modifier) as i32
    }

    /// `getMeatRatio()`.
    pub fn meat_ratio(&self) -> f32 {
        self.used_gene_value("meatRatio").unwrap_or(1.0)
    }

    /// `getEggGeneMod()`.
    pub fn egg_gene_mod(&self) -> f32 {
        match self.used_gene_value("eggSize") {
            Some(mut v) => {
                if self.has_genetic_disorder("smalleggs") {
                    v = 0.1;
                }
                v
            }
            None => 1.0,
        }
    }

    /// `haveEnoughMilkToFeedFrom()`.
    pub fn have_enough_milk_to_feed_from(&self) -> bool {
        self.data.milk_qty as f64 >= 0.02
    }

    /// `canBeSheared()`.
    pub fn can_be_sheared(&self, ctx: AnimalCtx<'_>) -> bool {
        self.breed().is_some_and(|b| b.wool_type.is_some()) && self.max_wool(ctx) > 0.0
    }

    // ---------------------------------------------------------------
    // fertilisation, babies, eggs

    /// `fertilize(male, force)`. `male` is `None` for a `null` male (only
    /// valid with `force`), [`Male::This`] when the animal fertilises
    /// itself.
    pub fn fertilize(
        &mut self,
        mut male: Male<'_>,
        force: bool,
        ctx: AnimalCtx<'_>,
        rand: &mut dyn GameRand,
    ) -> JResult<()> {
        if !force {
            fn fertility(m: &Animal, ctx: AnimalCtx<'_>) -> JResult<f32> {
                let Some(a) = m.used_gene("fertility") else {
                    return npe("fertility gene");
                };
                let mut f = a.current_value;
                if m.has_genetic_disorder("poorfertility") {
                    f = 1.0;
                }
                if m.has_genetic_disorder("fertile") {
                    f = 100.0;
                }
                if m.has_genetic_disorder("sterile") {
                    f = 0.0;
                }
                Ok(f * (1.0 - m.geriatric_percentage(ctx)))
            }
            let male_fert = match &male {
                Male::None => return npe("male"),
                Male::This => fertility(self, ctx)?,
                Male::Other(m) => fertility(m, ctx)?,
            };
            let female_fert = fertility(self, ctx)?;
            let rand_value = male_fert * female_fert * 100.0;
            match &mut male {
                Male::None => {}
                Male::This => {
                    self.data.last_impregnate_time = 24;
                    self.data.animal_to_inseminate = Vec::new();
                }
                Male::Other(m) => {
                    m.data.last_impregnate_time = 24;
                    m.data.animal_to_inseminate = Vec::new();
                }
            }
            if rand.next(100) as f32 > rand_value {
                return Ok(());
            }
        }
        self.fertilize_done(male, ctx)
    }

    fn fertilize_done(&mut self, male: Male<'_>, ctx: AnimalCtx<'_>) -> JResult<()> {
        self.data.male_genome = Some(match male {
            Male::None | Male::This => self.full_genome.clone(),
            Male::Other(m) => m.full_genome.clone(),
        });
        if self.adef(ctx).eggs_per_day > 0 {
            self.data.fertilized = true;
            self.data.fertilized_time = 1;
        } else {
            self.data.pregnant_time = 0;
            self.data.pregnant = true;
        }
        Ok(())
    }

    /// `addBaby()`: constructs the baby, gives it the parents' genes and
    /// hands it to the world. `None` when the type has no `babyType`.
    pub fn add_baby(
        &mut self,
        chr: &mut dyn AnimalCharacter,
        env: &mut AnimalEnv<'_>,
    ) -> JResult<Option<ObjectRef>> {
        let ctx = env.ctx;
        let adef = self.adef(ctx);
        let Some(baby_type) = adef.baby_type.clone() else {
            return Ok(None);
        };
        let Some(baby_def) = ctx.def(&baby_type) else {
            return npe("baby definition");
        };
        let breed_name = self
            .breed()
            .and_then(|b| b.name.clone())
            .unwrap_or_default();
        let breed = baby_def.breed_by_name(&breed_name).cloned();
        let (x, y, z) = chr.position();
        let pos = (x.floor(), y.floor(), z.floor());
        let mut baby = Animal::construct(&baby_type, breed.as_ref(), pos, env)?;
        baby.animal.full_genome = genome::init_genes_from_parents(
            &ctx.defs.genome,
            &self.full_genome,
            self.data.male_genome.as_ref(),
            env.rand,
        )?;
        baby.animal.check_genetic_disorder();
        baby.animal.data_init_size(&baby.chr, ctx)?;
        if baby.animal.has_genetic_disorder("dieatbirth") {
            let b = &baby.animal;
            b.set_health(&mut baby.chr, 0.0);
        }
        baby.animal.mother = Some(self.self_ref);
        baby.animal.mother_id = self.animal_id;
        baby.animal.invincible = self.invincible;
        let vehicle = chr.vehicle();
        let baby_id = baby.animal.animal_id;
        let handle = env.world.add_new_animal(baby, vehicle);
        self.data_update_last_pregnancy_time(env.ctx);
        self.data.male_genome = None;
        self.data.fertilized = false;
        if adef.udder {
            self.data_set_can_have_milk(true, ctx);
            self.data_update_last_time_milked(ctx);
        }
        // baby.setMother(this).
        let babies = self.babies.get_or_insert_with(Vec::new);
        let mut found = None;
        for (i, b) in babies.iter().enumerate() {
            let same = *b == handle
                || env
                    .world
                    .animal_mut(*b)
                    .is_some_and(|(o, _)| o.animal_id == baby_id);
            if same {
                found = Some(i);
                break;
            }
        }
        let babies = self.babies.get_or_insert_with(Vec::new);
        if let Some(i) = found {
            babies.remove(i);
        }
        babies.push(handle);
        Ok(Some(handle))
    }

    /// `createEgg()`: `None` when the egg item cannot be created.
    pub fn create_egg(&mut self, env: &mut AnimalEnv<'_>) -> JResult<Option<ObjectRef>> {
        let ctx = env.ctx;
        let adef = self.adef(ctx);
        let Some(egg) = env
            .world
            .create_item(adef.egg_type.as_deref().unwrap_or(""))
        else {
            return Ok(None);
        };
        let hung_gene = self.egg_gene_mod();
        let hung_mod = 1.0 + hung_gene / 2.0;
        let mut base_hung = 0.07f32;
        base_hung *= hung_mod;
        base_hung -= base_hung * 2.0;
        if self.data.fertilized {
            let genome = genome::init_genes_from_parents(
                &ctx.defs.genome,
                &self.full_genome,
                self.data.male_genome.as_ref(),
                env.rand,
            )?;
            env.world.set_egg_hatch(
                egg,
                EggHatch {
                    mother_id: self.animal_id,
                    time_to_hatch: adef.time_to_hatch,
                    animal_hatch: adef.baby_type.clone(),
                    base_hunger: base_hung,
                    hung_change: base_hung,
                    animal_hatch_breed: self.breed().and_then(|b| b.name.clone()),
                    genome,
                },
            );
        }
        if self.data.clutch_size > 0 {
            self.data.clutch_size -= 1;
        }
        Ok(Some(egg))
    }

    /// `addEgg(meta)`.
    pub fn add_egg(
        &mut self,
        meta: bool,
        chr: &dyn AnimalCharacter,
        env: &mut AnimalEnv<'_>,
    ) -> JResult<bool> {
        if self.has_genetic_disorder("noeggs") {
            return Ok(false);
        }
        if let Some(h) = self.hutch {
            return Ok(env.world.hutch_add_animal_in_nest_box(h));
        }
        if let Some(vehicle) = chr.vehicle() {
            let egg = self.create_egg(env)?;
            if env.world.vehicle_store_egg(vehicle, egg) {
                return Ok(true);
            }
            let x = env.rand.next_float_range(0.0, 0.5);
            let y = env.rand.next_float_range(0.0, 0.5);
            if let (Some(sq), Some(egg)) = (env.world.vehicle_square(vehicle), egg) {
                env.world.add_world_item(sq, egg, x, y, 0.0);
                env.world.add_to_process_items(egg);
            }
            return Ok(true);
        }
        let egg = self.create_egg(env)?;
        let mut sq = None;
        if let Some(c) = chr.container() {
            if let Some(egg) = egg {
                env.world.container_add_item(c, egg);
            }
        } else if meta && !self.connected_d_zone.is_empty() {
            sq = self.random_square_in_zone(env);
        } else {
            sq = chr.current_square();
        }
        if sq.is_some() {
            let x = env.rand.next_float_range(0.0, 0.5);
            let y = env.rand.next_float_range(0.0, 0.5);
            // The original drops the egg on getSquare(), not on sq.
            let Some(own) = chr.current_square() else {
                return npe("getSquare()");
            };
            if let Some(egg) = egg {
                env.world.add_world_item(own, egg, x, y, 0.0);
                env.world.add_to_process_items(egg);
            }
        }
        Ok(true)
    }

    // ---------------------------------------------------------------
    // away / meta

    /// `updateStatsAway(hours)`: the hours the animal spent unloaded, from
    /// `timeSinceLastUpdate`.
    pub fn update_stats_away(
        &mut self,
        hours: i32,
        chr: &mut dyn AnimalCharacter,
        env: &mut AnimalEnv<'_>,
    ) -> JResult<()> {
        self.from_meta = false;
        if self.wild {
            return Ok(());
        }
        self.zone_check_timer = 0.0;
        self.check_zone(chr, env);
        let age = self.data.age;
        let delta_age = (hours / 24).wrapping_mul(self.data_age_grow_modifier(env.ctx) as i32);
        let new_age = age.wrapping_add(delta_age);
        chr.set_hours_survived(new_age.wrapping_mul(24) as f64);
        self.data.last_hour_check = env.ctx.clock.hour();
        self.data.age = new_age;
        for _ in 0..hours {
            self.data_hour_grow(true, chr, env)?;
            self.time_since_last_update += 3_600_000;
            let cal = self.time_since_last_update;
            let (_, real_hour) = env.ctx.clock.calendar_month_hour(cal);
            self.data_try_inseminate_in_meta(cal, chr, env)?;
            if real_hour == 0 {
                self.data_grow_up(true, chr, env)?;
            }
            self.data_check_eggs(cal, true, chr, env)?;
            if self.check_killed_by_meta_predator(real_hour, chr, env)? {
                return Ok(());
            }
        }
        self.data_init(chr, env.ctx)
    }

    /// `checkKilledByMetaPredator(hour)`.
    pub fn check_killed_by_meta_predator(
        &mut self,
        hour: i32,
        chr: &mut dyn AnimalCharacter,
        env: &mut AnimalEnv<'_>,
    ) -> JResult<bool> {
        let ctx = env.ctx;
        if !ctx.sandbox.animal_meta_predator {
            return Ok(false);
        }
        let adef = self.adef(ctx);
        if adef.enter_hutch_time == 0 && adef.exit_hutch_time == 0 {
            return Ok(false);
        }
        let hour = if hour < 0 { ctx.clock.hour() } else { hour };
        if !adef.is_inside_hutch_time(hour) {
            return Ok(false);
        }
        let mut base_chance = 90;
        if self.is_baby()? {
            base_chance = 45;
        }
        if !chr.is_female() {
            base_chance = 150;
        }
        if !next_bool(env.rand, base_chance) {
            return Ok(false);
        }
        match self.hutch.filter(|h| !env.world.hutch_door_closed(*h)) {
            Some(h) => {
                env.world.hutch_kill_animal(h);
                let dirt = env.world.hutch_dirt(h) + env.rand.next_range(10, 20) as f32;
                env.world.set_hutch_dirt(h, dirt);
                if let Some((sq, x, y, z)) = env.world.hutch_enter_square(h) {
                    for _ in 0..20 {
                        let bx = x as f32 + env.rand.next_float_range(-0.8, 0.8);
                        let by = y as f32 + env.rand.next_float_range(-0.8, 0.8);
                        let k = env.rand.next(8);
                        env.world.add_blood_splat(sq, bx, by, z as f32, k);
                    }
                }
            }
            None => {
                if let Some(sq) = chr.current_square() {
                    let (x, y, z) = env.world.square_xyz(sq);
                    for _ in 0..20 {
                        let bx = x as f32 + env.rand.next_float_range(-0.8, 0.8);
                        let by = y as f32 + env.rand.next_float_range(-0.8, 0.8);
                        let k = env.rand.next(8);
                        env.world.add_blood_splat(sq, bx, by, z as f32, k);
                    }
                }
                self.hit_consequences_meta(666.0, chr, env)?;
            }
        }
        Ok(true)
    }

    /// `hitConsequences(null, null, false, damage, false)` (no weapon or
    /// attacker; single player).
    pub fn hit_consequences_meta(
        &mut self,
        damage: f32,
        chr: &mut dyn AnimalCharacter,
        env: &mut AnimalEnv<'_>,
    ) -> JResult<()> {
        let loss = self.data_health_loss(Some(0.025), env.ctx);
        let h = chr.health() - damage * loss;
        self.set_health(chr, h);
        if self.wild {
            // getWildFleeTimeUntilDeadTimer draws, then wielder.getPerkLevel
            // throws on the null wielder.
            let t = self.adef(env.ctx).wild_flee_time_until_dead_timer;
            env.rand.next_float_range(t / 2.0, t);
            return npe("wielder");
        }
        chr.splat_blood_floor_big(env.rand);
        chr.clear_attacked_by();
        self.attacked_timer = env.ctx.clock.calendar_millis();
        chr.set_hit_reaction("hitreact");
        let s = self.stress_level + env.rand.next_range(20, 40) as f32;
        self.set_debug_stress(s);
        Ok(())
    }

    // ---------------------------------------------------------------
    // copying

    /// `copyFrom(animal)` (the animal fields; the character copies its
    /// hours survived, hunger, thirst, health and sex itself).
    pub fn copy_from(&mut self, other: &Animal) {
        self.mod_data = other.mod_data.clone();
        self.custom_name = other.custom_name.clone();
        self.stress_level = other.stress_level;
        self.player_acceptance = other.player_acceptance.clone();
        self.animal_id = other.animal_id;
        self.pet_timer = other.pet_timer;
        self.full_genome = other.full_genome.clone();
        self.genetic_disorder = other.genetic_disorder.clone();
        self.attach_back_to_mother = other.attach_back_to_mother;
        self.data = other.data.clone();
        self.wild = other.wild;
    }

    /// `unloaded()` (the animal fields).
    pub fn unloaded(&mut self, ctx: AnimalCtx<'_>) {
        self.time_since_last_update = ctx.clock.calendar_millis();
        if self.data.attached_tree.is_some() {
            self.attach_back_to_tree_x = self.data.attached_tree_x;
            self.attach_back_to_tree_y = self.data.attached_tree_y;
        }
        self.data.attached_tree = None;
        self.data.attached_tree_x = i32::MAX;
        self.data.attached_tree_y = i32::MAX;
    }

    /// `setAgeDebug(newAge)`.
    pub fn set_age_debug(
        &mut self,
        new_age: i32,
        chr: &mut dyn AnimalCharacter,
        ctx: AnimalCtx<'_>,
    ) -> JResult<()> {
        self.data.age = new_age.max(self.adef(ctx).min_age);
        chr.set_hours_survived((self.data.age * 24) as f64);
        self.data_init(chr, ctx)
    }
}

/// The male of [`Animal::fertilize`].
pub enum Male<'a> {
    /// `null`.
    None,
    /// The animal itself.
    This,
    Other(&'a mut Animal),
}

/// `Math.round(float)`.
pub fn java_round(v: f32) -> i32 {
    if v.is_nan() {
        return 0;
    }
    (v as f64 + 0.5).floor() as i32
}
