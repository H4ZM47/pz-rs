//! Live Build 42.22 animal population manager (`AnimalPopulationManager` /
//! `AnimalManagerMain` / `AnimalManagerWorker`) wired into [`IsoWorld`].
//!
//! Owns [`pz_animals::population::AnimalPopulation`] over [`PopAnimal`] (a
//! lightweight [`pz_animals::NewAnimal`] while virtual/serialized), bridges
//! `apop/apop_<x>_<y>.bin` save/load, materializes arriving herds into live
//! [`LuaGameCharacterRef`] instances (`IsoAnimal`) when chunks load, and
//! re-virtualizes live animals when their chunks unload or the world saves.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;

use pz_characters::character::animal::{ordered_stat_decls, AnimalDefValues};
use pz_characters::character::SimContext;
use pz_characters::pz_animals::population::{
    AnimalPopulation, HerdAnimal, PopCx, PopulationHost, RealAnimal, VirtualAnimal,
};
use pz_characters::pz_animals::{
    Animal, AnimalClock, AnimalCtx, AnimalDefinitionTables, AnimalEnv, AnimalSandbox, AnimalWorld,
    JResult, NewAnimal, ObjectRef,
};
use pz_lua_api::bind_character_sim::{LuaGameCharacter, LuaGameCharacterRef};
use pz_lua_api::bind_characters::shared_characters;
use pz_lua_api::bind_registry::shared_script_manager;
use pz_lua_api::bind_world::with_world;
use pz_lua_api::LuaManager;
use pz_world::save::animal::CharacterStatDecl;
use pz_world::save::io::{Reader, SaveError, SaveResult, Writer};
use pz_world::save::items::AllItems;
use pz_world::save::{NoSaveHooks, SaveCtx, SaveSettings, SaveTables};
use pz_world::zones::ZoneId;
use pz_world::{GameRand, IsoMetaGrid, IsoSpriteManager};

use crate::iso_world::IsoWorld;
use crate::sim::FrameSim;

/// Virtual/serialized animal carried inside [`AnimalPopulation`].
#[derive(Clone, Debug)]
pub struct PopAnimal(pub NewAnimal);

impl HerdAnimal for PopAnimal {
    fn animal_id(&self) -> i32 {
        self.0.animal.animal_id
    }

    fn attach_back_to_mother(&self) -> i32 {
        self.0.animal.attach_back_to_mother
    }

    fn set_animal_zone(&mut self, zone: Option<ZoneId>) {
        self.0.animal.animal_zone = zone.map(|z| (0, z.0 as i64));
    }

    fn set_move_forward_on_zone(&mut self, forward: bool) {
        self.0.animal.move_forward_on_zone = forward;
    }
}

struct SimAnimalClock<'a>(&'a FrameSim);

impl AnimalClock for SimAnimalClock<'_> {
    fn hour(&self) -> i32 {
        (self.0.game_time.borrow().time_of_day.floor() as i32).clamp(0, 23)
    }

    fn month(&self) -> i32 {
        self.0.game_time.borrow().month
    }

    fn calendar_millis(&self) -> i64 {
        self.0.calendar_millis
    }

    fn multiplier(&self) -> f32 {
        self.0.multiplier
    }

    fn world_age_days_since_begin(&self) -> f64 {
        self.0.world_age_hours / 24.0
    }
}

/// Minimal `AnimalWorld` for population-time `add_baby` / `load` calls,
/// capturing any newly constructed baby into [`Self::captured_baby`].
struct PopAnimalWorld<'a> {
    meta: Option<&'a IsoMetaGrid>,
    captured_baby: Option<NewAnimal>,
}

impl AnimalWorld for PopAnimalWorld<'_> {
    fn animal_zone_exists(&self, msb: i64, lsb: i64) -> bool {
        let Some(meta) = self.meta else {
            return true;
        };
        pz_characters::pz_animals::population::zone::zone_by_uuid(meta, msb, lsb).is_some()
    }

    fn add_new_animal(&mut self, animal: NewAnimal, _into_vehicle: Option<ObjectRef>) -> ObjectRef {
        let id = animal.animal.animal_id;
        self.captured_baby = Some(animal);
        pz_characters::pz_animals::character_ref(id)
    }
}

/// Engine `PopulationHost<PopAnimal>` driving `AnimalPopulation`.
struct EnginePopHost<'a> {
    defs: &'a AnimalDefinitionTables,
    sim: &'a FrameSim,
    sandbox: AnimalSandbox,
    animal_track_chance: i32,
    stat_defs: Rc<[CharacterStatDecl]>,
    save_dir: Option<&'a PathBuf>,
    no_save: bool,
    save_tables: Option<&'a SaveTables>,
    sprites: Option<&'a IsoSpriteManager>,
    rand: EnginePopRand,
    spawned: Vec<LuaGameCharacterRef>,
}

struct EnginePopRand(u64);

impl EnginePopRand {
    fn new(seed: u64) -> Self {
        Self(seed.max(1))
    }
}

impl GameRand for EnginePopRand {
    fn next(&mut self, bound: i32) -> i32 {
        if bound <= 0 {
            return 0;
        }
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        (x % (bound as u64)) as i32
    }

    fn next_float(&mut self) -> f32 {
        (self.next(1_000_000) as f32) / 1_000_000.0
    }
}

impl<'a> EnginePopHost<'a> {
    fn apop_path(&self, x: i32, y: i32) -> Option<PathBuf> {
        self.save_dir
            .map(|dir| dir.join("apop").join(format!("apop_{x}_{y}.bin")))
    }

    fn materialize(&mut self, meta: &IsoMetaGrid, herds: Vec<VirtualAnimal<PopAnimal>>) {
        let sm = shared_script_manager();
        let chars_rc = shared_characters();
        let mut state_reg = pz_characters::character::state::StateRegistry::with_ported();
        for mut va in herds {
            let zone_uuid = va.zone.and_then(|zid| {
                meta.zone(zid).id.map(|u| (u.msb, u.lsb))
            });
            for mut pa in va.animals.drain(..) {
                let mut chars = chars_rc.borrow_mut();
                let Ok(mut chr) = LuaGameCharacter::new_animal(&sm, &mut chars) else {
                    continue;
                };
                pa.0.chr.x = va.x;
                pa.0.chr.y = va.y;
                pa.0.chr.z = va.z;
                if let Some(zu) = zone_uuid {
                    pa.0.animal.animal_zone = Some(zu);
                }
                pa.0.animal.move_forward_on_zone = va.move_forward_on_zone;
                pa.0.animal.virtual_id = va.id;
                if va.migration_group.is_some() {
                    pa.0.animal.migration_group = va.migration_group.clone();
                }
                chr.apply_animal_character_values(&chars, &pa.0.chr);
                drop(chars);
                if let Some(fields) = chr.animal_fields_mut() {
                    if let Some(ty) = pa.0.animal.animal_type.as_deref() {
                        fields.def = self.defs.def(ty).map(AnimalDefValues::of);
                    }
                    fields.animal = pa.0.animal;
                    fields.initialized = true;
                    fields.exists_in_world = true;
                }
                chr.initialize_animal_states(&mut state_reg);
                chr.path.current_square =
                    Some((va.x.floor() as i32, va.y.floor() as i32, va.z.floor() as i32));
                self.spawned.push(Rc::new(RefCell::new(chr)));
            }
        }
    }
}

impl PopulationHost<PopAnimal> for EnginePopHost<'_> {
    fn rand(&mut self) -> &mut dyn GameRand {
        &mut self.rand
    }

    fn hour(&self) -> i32 {
        (self.sim.game_time.borrow().time_of_day.floor() as i32).clamp(0, 23)
    }

    fn world_age_hours(&self) -> f64 {
        self.sim.world_age_hours
    }

    fn calendar_millis(&self) -> i64 {
        self.sim.calendar_millis
    }

    fn multiplier(&self) -> f32 {
        self.sim.multiplier
    }

    fn thirty_fps_multiplier(&self) -> f32 {
        self.sim.thirty_fps_multiplier()
    }

    fn animal_track_chance(&self) -> i32 {
        self.animal_track_chance
    }

    fn new_animal(&mut self, animal_type: &str, breed_name: &str) -> JResult<PopAnimal> {
        let clock = SimAnimalClock(self.sim);
        let breed = self
            .defs
            .def(animal_type)
            .and_then(|d| d.breed_by_name(breed_name))
            .cloned();
        let mut world = PopAnimalWorld {
            meta: None,
            captured_baby: None,
        };
        let mut env = AnimalEnv {
            ctx: AnimalCtx {
                defs: self.defs,
                clock: &clock,
                sandbox: &self.sandbox,
            },
            rand: &mut self.rand,
            world: &mut world,
            stat_defs: self.stat_defs.clone(),
            patience_range: (LuaGameCharacter::PATIENCE_MIN, LuaGameCharacter::PATIENCE_MAX),
        };
        let na = Animal::construct(animal_type, breed.as_ref(), (0.0, 0.0, 0.0), &mut env)?;
        Ok(PopAnimal(na))
    }

    fn randomize_age(&mut self, a: &mut PopAnimal) -> JResult<()> {
        let clock = SimAnimalClock(self.sim);
        let ctx = AnimalCtx {
            defs: self.defs,
            clock: &clock,
            sandbox: &self.sandbox,
        };
        let adef = a.0.animal.adef(ctx);
        let min_age = adef.min_age.max(0);
        let max_age = adef.max_age_geriatric.max(min_age);
        let age = if max_age > min_age {
            min_age + self.rand.next(max_age - min_age + 1)
        } else {
            min_age
        };
        a.0.animal.data.age = age;
        a.0.animal.data_init_size(&a.0.chr, ctx)?;
        a.0.animal.data_init_weight(ctx)?;
        Ok(())
    }

    fn can_have_baby(&self, a: &PopAnimal) -> bool {
        let clock = SimAnimalClock(self.sim);
        let ctx = AnimalCtx {
            defs: self.defs,
            clock: &clock,
            sandbox: &self.sandbox,
        };
        a.0.animal.data_can_have_baby(&a.0.chr, ctx)
    }

    fn add_baby(&mut self, mother: &mut PopAnimal) -> JResult<Option<PopAnimal>> {
        let clock = SimAnimalClock(self.sim);
        let mut world = PopAnimalWorld {
            meta: None,
            captured_baby: None,
        };
        let mut env = AnimalEnv {
            ctx: AnimalCtx {
                defs: self.defs,
                clock: &clock,
                sandbox: &self.sandbox,
            },
            rand: &mut self.rand,
            world: &mut world,
            stat_defs: self.stat_defs.clone(),
            patience_range: (LuaGameCharacter::PATIENCE_MIN, LuaGameCharacter::PATIENCE_MAX),
        };
        let _ = mother.0.animal.add_baby(&mut mother.0.chr, &mut env)?;
        Ok(world.captured_baby.map(PopAnimal))
    }

    fn set_virtual_id(&mut self, a: &mut PopAnimal, virtual_id: f64, migration_group: &str) {
        a.0.animal.virtual_id = virtual_id;
        a.0.animal.migration_group = Some(migration_group.to_string());
    }

    fn set_mother(&mut self, baby: &mut PopAnimal, mother: &mut PopAnimal) {
        baby.0.animal.mother_id = mother.0.animal.animal_id;
        baby.0.animal.attach_back_to_mother = mother.0.animal.animal_id;
    }

    fn save_animal(&mut self, a: &PopAnimal, w: &mut Writer) -> SaveResult<()> {
        let clock = SimAnimalClock(self.sim);
        let ctx = AnimalCtx {
            defs: self.defs,
            clock: &clock,
            sandbox: &self.sandbox,
        };
        let mother_id = (a.0.animal.mother_id > 0).then_some(a.0.animal.mother_id);
        let saved = a
            .0
            .animal
            .to_saved(&a.0.chr, ctx, mother_id)
            .map_err(|e| SaveError(e.0))?;
        let fallback_tables =
            SaveTables::from_text("bloodBodyParts=17\n0:IsoObject\n1:Animal\n").unwrap();
        let tables = self.save_tables.unwrap_or(&fallback_tables);
        let sprites = self
            .sprites
            .ok_or_else(|| SaveError("missing IsoSpriteManager for animal save".into()))?;
        let settings = SaveSettings {
            calendar_millis: self.sim.calendar_millis,
            ..SaveSettings::default()
        };
        let items = AllItems;
        let save_ctx = SaveCtx {
            sprites,
            tables,
            settings: &settings,
            items: &items,
            hooks: &NoSaveHooks,
            meta: None,
        };
        let class_id = if tables.factory.is_empty() {
            0
        } else {
            tables.class_id("Animal")
        };
        w.i8(1);
        w.i8(class_id);
        saved.save(w, &save_ctx, false)
    }

    fn load_animal(&mut self, r: &mut Reader<'_>, world_version: i32) -> SaveResult<PopAnimal> {
        let _serialize = r.i8()?;
        let _class_id = r.i8()?;
        let clock = SimAnimalClock(self.sim);
        let mut world = PopAnimalWorld {
            meta: None,
            captured_baby: None,
        };
        let mut env = AnimalEnv {
            ctx: AnimalCtx {
                defs: self.defs,
                clock: &clock,
                sandbox: &self.sandbox,
            },
            rand: &mut self.rand,
            world: &mut world,
            stat_defs: self.stat_defs.clone(),
            patience_range: (LuaGameCharacter::PATIENCE_MIN, LuaGameCharacter::PATIENCE_MAX),
        };
        let na = Animal::load(r, world_version, &mut env).map_err(|e| SaveError(e.0))?;
        Ok(PopAnimal(na))
    }

    fn read_apop(&mut self, x: i32, y: i32) -> std::io::Result<Option<Vec<u8>>> {
        let Some(path) = self.apop_path(x, y) else {
            return Ok(None);
        };
        match std::fs::read(&path) {
            Ok(bytes) => Ok(Some(bytes)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e),
        }
    }

    fn write_apop(&mut self, x: i32, y: i32, bytes: &[u8]) -> std::io::Result<()> {
        if self.no_save {
            return Ok(());
        }
        let Some(path) = self.apop_path(x, y) else {
            return Ok(());
        };
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&path, bytes)
    }

    fn no_save(&self) -> bool {
        self.no_save
    }

    fn from_worker(&mut self, meta: &IsoMetaGrid, animals: Vec<VirtualAnimal<PopAnimal>>) {
        self.materialize(meta, animals);
    }
}

/// Live world animal population + active `IsoAnimal` instances.
pub struct WorldAnimals {
    pub population: AnimalPopulation<PopAnimal>,
    pub defs: Rc<AnimalDefinitionTables>,
    live: Vec<LuaGameCharacterRef>,
    save_dir: Option<PathBuf>,
    no_save: bool,
    animal_track_chance: i32,
    sandbox: AnimalSandbox,
    stat_defs: Rc<[CharacterStatDecl]>,
    last_saved_cells: HashMap<i32, (i32, i32)>,
    rng_seed: u64,
}

impl WorldAnimals {
    /// Initializes `AnimalPopulation` against `meta` and loads `apop/` metadata.
    pub fn init(
        defs: Rc<AnimalDefinitionTables>,
        meta: &mut IsoMetaGrid,
        save_dir: Option<PathBuf>,
        no_save: bool,
        animal_track_chance: i32,
        sandbox: AnimalSandbox,
        sim: &FrameSim,
    ) -> Self {
        let stat_defs = {
            let chars = shared_characters();
            let c = chars.borrow();
            ordered_stat_decls(&c.sim.character_stats).unwrap_or_else(|_| Rc::from([]))
        };
        let mut wa = Self {
            population: AnimalPopulation::default(),
            defs,
            live: Vec::new(),
            save_dir,
            no_save,
            animal_track_chance,
            sandbox,
            stat_defs,
            last_saved_cells: HashMap::new(),
            rng_seed: 0x414e_494d_414c_5301,
        };
        wa.with_host(meta, sim, None, None, |pop, cx| {
            pop.init(false, false, cx);
        });
        wa
    }

    fn with_host<R>(
        &mut self,
        meta: &mut IsoMetaGrid,
        sim: &FrameSim,
        save_tables: Option<&SaveTables>,
        sprites: Option<&IsoSpriteManager>,
        f: impl FnOnce(&mut AnimalPopulation<PopAnimal>, &mut PopCx<'_, PopAnimal>) -> R,
    ) -> R {
        self.rng_seed = self.rng_seed.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut host = EnginePopHost {
            defs: &self.defs,
            sim,
            sandbox: self.sandbox,
            animal_track_chance: self.animal_track_chance,
            stat_defs: self.stat_defs.clone(),
            save_dir: self.save_dir.as_ref(),
            no_save: self.no_save,
            save_tables,
            sprites,
            rand: EnginePopRand::new(self.rng_seed),
            spawned: Vec::new(),
        };
        let out = {
            let mut cx = PopCx {
                meta,
                defs: &self.defs,
                host: &mut host,
            };
            f(&mut self.population, &mut cx)
        };
        self.rng_seed = host.rand.0;
        self.live.extend(host.spawned);
        out
    }

    /// Converts a live `LuaGameCharacter` animal into a [`RealAnimal<PopAnimal>`].
    fn to_real_animal(&self, chr_ref: &LuaGameCharacterRef) -> Option<RealAnimal<PopAnimal>> {
        let chars_rc = shared_characters();
        let chars = chars_rc.borrow();
        let chr = chr_ref.try_borrow().ok()?;
        let fields = chr.animal_fields()?;
        let vals = chr.animal_character_values(&chars)?;
        let last_cell = self
            .last_saved_cells
            .get(&fields.animal.animal_id)
            .copied()
            .unwrap_or((i32::MIN, i32::MIN));
        Some(RealAnimal {
            x: chr.vars.x,
            y: chr.vars.y,
            z: chr.vars.z,
            move_forward_on_zone: fields.animal.move_forward_on_zone,
            forward_direction: (chr.vars.forward_x, chr.vars.forward_y),
            virtual_id: fields.animal.virtual_id,
            migration_group: fields.animal.migration_group.clone(),
            last_cell_saved_to: last_cell,
            animal: PopAnimal(NewAnimal {
                animal: fields.animal.clone(),
                chr: vals,
            }),
        })
    }

    /// `AnimalPopulationManager.addChunkToWorld(chunk)`.
    pub fn chunk_added(
        &mut self,
        meta: &mut IsoMetaGrid,
        wx: i32,
        wy: i32,
        new_chunk: bool,
        sim: &FrameSim,
        save_tables: Option<&SaveTables>,
        sprites: Option<&IsoSpriteManager>,
    ) {
        let _ = self.with_host(meta, sim, save_tables, sprites, |pop, cx| {
            pop.add_chunk_to_world(wx, wy, new_chunk, cx)
        });
    }

    /// `AnimalPopulationManager.removeChunkFromWorld(chunk)`: virtualizes any
    /// live animals currently standing in chunk `(wx, wy)`.
    pub fn chunk_removed(
        &mut self,
        meta: &mut IsoMetaGrid,
        wx: i32,
        wy: i32,
        sim: &FrameSim,
        save_tables: Option<&SaveTables>,
        sprites: Option<&IsoSpriteManager>,
    ) {
        if !self.population.remove_chunk_from_world_begin(wx, wy) {
            return;
        }
        let chunk_w = pz_world::chunk::CHUNK_WIDTH;
        let mut leaving = Vec::new();
        let live = std::mem::take(&mut self.live);
        let mut kept = Vec::with_capacity(live.len());
        for chr_ref in live {
            let in_chunk = chr_ref.try_borrow().is_ok_and(|c| {
                let cx = (c.vars.x.floor() as i32).div_euclid(chunk_w);
                let cy = (c.vars.y.floor() as i32).div_euclid(chunk_w);
                cx == wx && cy == wy
            });
            if in_chunk {
                if let Ok(mut c) = chr_ref.try_borrow_mut() {
                    if let Some(f) = c.animal_fields_mut() {
                        f.exists_in_world = false;
                    }
                }
                if let Some(real) = self.to_real_animal(&chr_ref) {
                    leaving.push(real);
                }
            } else {
                kept.push(chr_ref);
            }
        }
        self.live = kept;
        self.with_host(meta, sim, save_tables, sprites, |pop, cx| {
            for real in leaving {
                let _ = pop.add_real_animal(real, cx);
            }
            pop.remove_chunk_from_world_end(wx, wy);
        });
    }

    /// Steps virtual herds (`AnimalPopulationManager.update`).
    pub fn update(
        &mut self,
        meta: &mut IsoMetaGrid,
        sim: &FrameSim,
        save_tables: Option<&SaveTables>,
        sprites: Option<&IsoSpriteManager>,
    ) {
        let _ = self.with_host(meta, sim, save_tables, sprites, |pop, cx| {
            pop.update_virtual_animals(cx)
        });
    }

    /// Saves all live animals and dirty virtual cells to `apop/apop_<x>_<y>.bin`.
    pub fn save(
        &mut self,
        meta: &mut IsoMetaGrid,
        sim: &FrameSim,
        save_tables: Option<&SaveTables>,
        sprites: Option<&IsoSpriteManager>,
    ) {
        let mut ids = Vec::with_capacity(self.live.len());
        let mut reals = Vec::with_capacity(self.live.len());
        for chr_ref in &self.live {
            if let Some(real) = self.to_real_animal(chr_ref) {
                ids.push(real.animal.0.animal.animal_id);
                reals.push(real);
            }
        }
        let saved_to = self.with_host(meta, sim, save_tables, sprites, |pop, cx| {
            pop.save(reals, cx).unwrap_or_default()
        });
        for (id, cell) in ids.into_iter().zip(saved_to) {
            self.last_saved_cells.insert(id, cell);
        }
    }

    /// `AnimalPopulationManager.stop()`.
    pub fn stop(&mut self) {
        self.population.stop();
        self.live.clear();
        self.last_saved_cells.clear();
    }

    /// Currently materialized live `IsoAnimal` instances.
    pub fn live_animals(&self) -> &[LuaGameCharacterRef] {
        &self.live
    }

    /// Registers an externally constructed live `IsoAnimal`.
    pub fn push_live_animal(&mut self, chr: LuaGameCharacterRef) {
        self.live.push(chr);
    }
}

fn load_animal_defs_lenient(env: &pz_kahlua::TableRef) -> AnimalDefinitionTables {
    let has_sub = |global: &str, key: &str| {
        env.get_str(global)
            .as_table()
            .is_some_and(|t| t.rawget(&pz_kahlua::Value::str(key)).as_table().is_some())
    };
    let genome = if has_sub("AnimalGenomeDefinitions", "genes") {
        pz_characters::pz_animals::AnimalGenomeDefinitions::load(env).unwrap_or_default()
    } else {
        Default::default()
    };
    let animal_defs = if has_sub("AnimalDefinitions", "animals") {
        pz_characters::pz_animals::AnimalDefinitions::load_all(env).unwrap_or_default()
    } else {
        Default::default()
    };
    let tracks = if has_sub("AnimalTracksDefinitions", "animallist") {
        pz_characters::pz_animals::AnimalTracksDefinitions::load_all(env).unwrap_or_default()
    } else {
        Default::default()
    };
    let migration = if env.get_str("MigrationGroupDefinitions").as_table().is_some() {
        pz_characters::pz_animals::MigrationGroupDefinitions::load_all(env).unwrap_or_default()
    } else {
        Default::default()
    };
    AnimalDefinitionTables {
        genome,
        animal_defs,
        tracks,
        migration,
    }
}

impl IsoWorld {
    /// Initializes `WorldAnimals` (`AnimalPopulationManager.init`) during `IsoWorld::init`.
    pub(crate) fn init_animals(&mut self, lua: &mut LuaManager, no_save: bool) {
        let defs = Rc::new(load_animal_defs_lenient(lua.thread.globals()));
        let sim = FrameSim::capture();
        let (track_chance, sandbox) = crate::iso_world::sandbox(lua)
            .map(|sb| {
                let s = sb.borrow();
                let tc = crate::iso_world::opt_int(&s, "AnimalTrackChance").max(1);
                let mut asb = AnimalSandbox::default();
                let iopt = |n: &str, d: i32| {
                    let v = crate::iso_world::opt_int(&s, n);
                    if v > 0 { v } else { d }
                };
                asb.animal_stats_modifier = iopt("AnimalStatsModifier", asb.animal_stats_modifier);
                asb.animal_meta_stats_modifier =
                    iopt("AnimalMetaStatsModifier", asb.animal_meta_stats_modifier);
                asb.animal_pregnancy_time = iopt("AnimalPregnancyTime", asb.animal_pregnancy_time);
                asb.animal_age_modifier = iopt("AnimalAgeModifier", asb.animal_age_modifier);
                asb.animal_milk_inc_modifier =
                    iopt("AnimalMilkIncModifier", asb.animal_milk_inc_modifier);
                asb.animal_wool_inc_modifier =
                    iopt("AnimalWoolIncModifier", asb.animal_wool_inc_modifier);
                asb.animal_meta_predator =
                    crate::iso_world::opt_bool(&s, "AnimalMetaPredator");
                asb.animal_mating_season =
                    crate::iso_world::opt_bool(&s, "AnimalMatingSeason");
                (tc, asb)
            })
            .unwrap_or_else(|| (4, AnimalSandbox::default()));
        let save_dir = (!no_save).then(|| self.save_dir.clone());
        self.animals = with_world(|w| {
            w.meta_grid.as_mut().map(|meta| {
                WorldAnimals::init(defs, meta, save_dir, no_save, track_chance, sandbox, &sim)
            })
        });
    }

    pub(crate) fn animals_gen_path(&mut self, wx: i32, wy: i32) {
        let Some(wa) = self.animals.as_mut() else { return };
        let sim = FrameSim::capture();
        let save_tables = self.save_tables.as_ref();
        let sprites_rc = self.sprites.as_ref();
        let sprites_guard = sprites_rc.map(|s| s.borrow());
        let cx = wx.div_euclid(pz_characters::pz_animals::population::CHUNKS_PER_CELL);
        let cy = wy.div_euclid(pz_characters::pz_animals::population::CHUNKS_PER_CELL);
        with_world(|w| {
            if let Some(meta) = w.meta_grid.as_mut() {
                wa.with_host(meta, &sim, save_tables, sprites_guard.as_deref(), |pop, pcx| {
                    pop.alloc_cell_at(cx, cy, pcx);
                });
            }
        });
    }

    pub(crate) fn animals_chunk_added(&mut self, wx: i32, wy: i32) {
        let Some(wa) = self.animals.as_mut() else { return };
        let sim = FrameSim::capture();
        let save_tables = self.save_tables.as_ref();
        let sprites_rc = self.sprites.as_ref();
        let sprites_guard = sprites_rc.map(|s| s.borrow());
        with_world(|w| {
            if let Some(meta) = w.meta_grid.as_mut() {
                wa.chunk_added(
                    meta,
                    wx,
                    wy,
                    false,
                    &sim,
                    save_tables,
                    sprites_guard.as_deref(),
                );
            }
        });
    }

    pub(crate) fn animals_chunk_removed(&mut self, wx: i32, wy: i32) {
        let Some(wa) = self.animals.as_mut() else { return };
        let sim = FrameSim::capture();
        let save_tables = self.save_tables.as_ref();
        let sprites_rc = self.sprites.as_ref();
        let sprites_guard = sprites_rc.map(|s| s.borrow());
        with_world(|w| {
            if let Some(meta) = w.meta_grid.as_mut() {
                wa.chunk_removed(meta, wx, wy, &sim, save_tables, sprites_guard.as_deref());
            }
        });
    }

    pub(crate) fn update_animals(&mut self, sim: &FrameSim) {
        let Some(wa) = self.animals.as_mut() else { return };
        let save_tables = self.save_tables.as_ref();
        let sprites_rc = self.sprites.as_ref();
        let sprites_guard = sprites_rc.map(|s| s.borrow());
        with_world(|w| {
            if let Some(meta) = w.meta_grid.as_mut() {
                wa.update(meta, sim, save_tables, sprites_guard.as_deref());
            }
        });
    }

    pub(crate) fn save_animals(&mut self, sim: &FrameSim) {
        let Some(wa) = self.animals.as_mut() else { return };
        let save_tables = self.save_tables.as_ref();
        let sprites_rc = self.sprites.as_ref();
        let sprites_guard = sprites_rc.map(|s| s.borrow());
        with_world(|w| {
            if let Some(meta) = w.meta_grid.as_mut() {
                wa.save(meta, sim, save_tables, sprites_guard.as_deref());
            }
        });
    }

    pub fn reset_animals(&mut self) {
        if let Some(mut wa) = self.animals.take() {
            wa.stop();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pz_characters::pz_animals::{AnimalBreed, AnimalDefinitions, AnimalGrowStage};

    fn synthetic_defs() -> Rc<AnimalDefinitionTables> {
        let mut tables = AnimalDefinitionTables::default();
        let mut def = AnimalDefinitions::default();
        def.animal_type_str = Some("cow".to_string());
        def.group = Some("cow".to_string());
        def.min_age = 30;
        def.max_age_geriatric = 100;
        def.min_size = 0.8;
        def.max_size = 1.0;
        def.min_weight = 100.0;
        def.max_weight = 200.0;
        def.female = true;
        def.stages.push(AnimalGrowStage {
            stage: Some("cow".to_string()),
            age_to_grow: 0,
            next_stage: None,
            next_stage_male: None,
        });
        def.breeds.push(AnimalBreed {
            name: Some("angus".to_string()),
            texture: Some(vec!["Cow_Angus".to_string()]),
            ..Default::default()
        });
        tables.animal_defs.insert("cow".to_string(), def);
        Rc::new(tables)
    }

    #[test]
    fn animal_population_persists_apop_and_restores_on_chunk_reload() {
        {
            let mut t = pz_scripts::registry::builtin::BuiltinTables::default();
            for f in ["HUNGER", "THIRST"] {
                t.character_stats.push(pz_scripts::registry::builtin::CharacterStatDecl {
                    field: f.to_string(),
                    id: f.to_lowercase(),
                    minimum: 0.0,
                    maximum: 1.0,
                    default: 0.0,
                });
                t.ordered_stats.push(f.to_string());
            }
            shared_characters().borrow_mut().sim =
                pz_characters::SimStatics::from_tables(&t).unwrap();
        }
        let root = std::env::temp_dir().join(format!(
            "pz-engine-animals-apop-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();

        let defs = synthetic_defs();
        let mut meta = IsoMetaGrid::with_bounds(0, 0, 0, 0);

        let sim = FrameSim::capture();
        let mut wa = WorldAnimals::init(
            defs.clone(),
            &mut meta,
            Some(root.clone()),
            false,
            1,
            AnimalSandbox::default(),
            &sim,
        );

        // Create a live cow at (20.5, 20.5, 0.0) in chunk (2, 2) of cell (0, 0).
        let sm = shared_script_manager();
        let chars_rc = shared_characters();
        let mut chr = {
            let mut chars = chars_rc.borrow_mut();
            LuaGameCharacter::new_animal(&sm, &mut chars).unwrap()
        };
        let clock = SimAnimalClock(&sim);
        let stat_defs = {
            let chars = chars_rc.borrow();
            ordered_stat_decls(&chars.sim.character_stats).unwrap()
        };
        let mut rand = EnginePopRand::new(77);
        let mut world = PopAnimalWorld {
            meta: Some(&meta),
            captured_baby: None,
        };
        let sandbox = AnimalSandbox::default();
        let mut env = AnimalEnv {
            ctx: AnimalCtx {
                defs: &defs,
                clock: &clock,
                sandbox: &sandbox,
            },
            rand: &mut rand,
            world: &mut world,
            stat_defs,
            patience_range: (LuaGameCharacter::PATIENCE_MIN, LuaGameCharacter::PATIENCE_MAX),
        };
        let breed = defs.def("cow").unwrap().breed_by_name("angus").cloned();
        let na = Animal::construct("cow", breed.as_ref(), (20.5, 20.5, 0.0), &mut env).unwrap();
        let animal_id = na.animal.animal_id;
        {
            let chars = chars_rc.borrow();
            chr.apply_animal_character_values(&chars, &na.chr);
        }
        if let Some(f) = chr.animal_fields_mut() {
            f.animal = na.animal;
            f.def = defs.def("cow").map(AnimalDefValues::of);
            f.initialized = true;
            f.exists_in_world = true;
        }
        wa.push_live_animal(Rc::new(RefCell::new(chr)));

        let sprites = IsoSpriteManager::new(pz_world::sprite::test_tables());
        // Save to disk -> writes `apop/apop_0_0.bin`.
        wa.save(&mut meta, &sim, None, Some(&sprites));
        assert!(root.join("apop/apop_0_0.bin").exists());
        wa.stop();

        // Reload fresh `WorldAnimals` against the same save dir and load chunk (2, 2).
        let mut wa2 = WorldAnimals::init(
            defs,
            &mut meta,
            Some(root.clone()),
            false,
            1,
            AnimalSandbox::default(),
            &sim,
        );
        assert!(wa2.live_animals().is_empty());
        wa2.chunk_added(&mut meta, 2, 2, false, &sim, None, Some(&sprites));
        assert_eq!(wa2.live_animals().len(), 1);
        {
            let restored = wa2.live_animals()[0].borrow();
            let f = restored.animal_fields().expect("animal fields");
            assert_eq!(f.animal.animal_type.as_deref(), Some("cow"));
            assert_eq!(f.animal.animal_id, animal_id);
            assert!((restored.vars.x - 20.5).abs() < 0.01);
            assert!((restored.vars.y - 20.5).abs() < 0.01);
        }

        // Unloading chunk (2, 2) virtualizes the live animal back into AnimalPopulation.
        wa2.chunk_removed(&mut meta, 2, 2, &sim, None, Some(&sprites));
        assert!(wa2.live_animals().is_empty());
        wa2.stop();
        let _ = std::fs::remove_dir_all(&root);
    }
}
