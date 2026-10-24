//! `IsoAnimal.save`/`load` as far as a hutch stores its animals
//! (`IsoHutch.animalInside` and the nest boxes), with the load-time changes
//! the original makes before saving again.
//!
//! Loading an animal reads its definition (`AnimalDefinitions`, built from
//! the Lua definition files) through [`SaveHooks::animal_definition`]; the
//! save format itself depends on it (wool and egg fields are written only
//! for some animals).

use super::io::{err, Reader, SaveResult, Writer};
use super::lua_table::LuaTable;
use super::{SaveCtx, SaveHooks};
use pz_inventory::item::AnimalGene;

/// One `CharacterStat` declaration: the static field, the stat id, its
/// range and default value. Mirrors `pz_scripts`' `CharacterStatDecl`
/// (`BuiltinTables::character_stats`, read from the installed game); this
/// crate only takes the list as input.
#[derive(Clone, Debug, PartialEq)]
pub struct CharacterStatDecl {
    pub field: String,
    pub id: String,
    pub minimum: f32,
    pub maximum: f32,
    pub default: f32,
}

/// The declarations in `CharacterStat.ORDERED_STATS` order (the order
/// `Stats.save` writes), from `BuiltinTables::character_stats` and
/// `BuiltinTables::ordered_stats` (field names). `None` if a name has no
/// declaration.
pub fn ordered_character_stats(
    decls: &[CharacterStatDecl],
    ordered: &[String],
) -> Option<Vec<CharacterStatDecl>> {
    ordered
        .iter()
        .map(|f| decls.iter().find(|d| d.field == *f).cloned())
        .collect()
}

/// `PZMath.clamp(float, float, float)`.
fn clamp(v: f32, min: f32, max: f32) -> f32 {
    if v < min {
        min
    } else if v > max {
        max
    } else {
        v
    }
}

/// `Stats` of a character: the `ORDERED_STATS` values.
#[derive(Clone, Debug, PartialEq)]
pub struct Stats(pub Vec<f32>);

impl Stats {
    /// Every stat at its default value.
    pub fn new(defs: &[CharacterStatDecl]) -> Self {
        Stats(defs.iter().map(|d| d.default).collect())
    }

    /// `Stats.set`: clamped to the stat's range.
    pub fn set(&mut self, defs: &[CharacterStatDecl], i: usize, v: f32) {
        self.0[i] = clamp(v, defs[i].minimum, defs[i].maximum);
    }

    /// `Stats.load(ByteBuffer, int)`.
    pub fn load(&mut self, defs: &[CharacterStatDecl], r: &mut Reader<'_>) -> SaveResult<()> {
        for i in 0..defs.len() {
            let v = r.f32()?;
            self.set(defs, i, v);
        }
        Ok(())
    }

    /// `Stats.save(ByteBuffer)`.
    pub fn save(&self, w: &mut Writer) {
        for &v in &self.0 {
            w.f32(v);
        }
    }
}

/// `AnimalBreed`: what loading an animal reads.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AnimalBreedDef {
    pub name: String,
    /// `woolType` (`None` when the breed has none).
    pub wool_type: Option<String>,
    /// `forcedGenes` keys, in their `HashMap` iteration order.
    pub forced_genes: Vec<String>,
}

/// `AnimalDefinitions`: what loading an animal reads.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AnimalDefinition {
    pub eggs_per_day: i32,
    pub max_wool: f32,
    pub min_size: f32,
    pub max_size: f32,
    pub can_be_domesticated: bool,
    pub wild: bool,
    /// Whether the animal's own grow stage has a next stage (`isBaby`).
    pub baby: bool,
    /// `genes` that `AnimalGene.initGenome` creates (those with an
    /// `AnimalGenomeDefinitions` entry), in order; `None` when the
    /// definition has no `genes`.
    pub genes: Option<Vec<String>>,
    pub breeds: Vec<AnimalBreedDef>,
}

impl AnimalDefinition {
    /// `getBreedByName`.
    pub fn breed(&self, name: &str) -> Option<&AnimalBreedDef> {
        self.breeds.iter().find(|b| b.name == name)
    }
}

/// An `IsoAnimal` as a hutch stores it.
#[derive(Clone, Debug, PartialEq)]
pub struct IsoAnimal {
    /// `animalZone.id` (most and least significant bits); `None` when the
    /// animal has no zone.
    pub animal_zone: Option<(i64, i64)>,
    pub x: f32,
    pub y: f32,
    pub z: f32,
    /// `getForwardIsoDirection().ordinal()`.
    pub dir: i32,
    pub stats: Stats,
    /// `type`.
    pub animal_type: String,
    /// `getData().getBreed().name`.
    pub breed: String,
    /// `customName` (`None` when unset; saved as `""`).
    pub custom_name: Option<String>,
    pub mod_data: LuaTable,
    pub item_id: i32,
    pub female: bool,
    pub animal_id: i32,
    /// `fullGenome` in its `HashMap<String, AnimalGene>` iteration order.
    pub genome: Vec<(String, AnimalGene)>,
    /// `getData().getAttachedTree()`'s square (unset after loading).
    pub attached_tree: Option<(i32, i32)>,
    pub attach_back_to_tree_x: i32,
    pub attach_back_to_tree_y: i32,
    pub age: i32,
    /// `IsoPlayer.hoursSurvived`.
    pub hours_survived: f64,
    /// `timeSinceLastUpdate`.
    pub time_since_last_update: i64,
    pub size: f32,
    pub attach_back_to_mother: i32,
    /// `mother.animalId` (unset after loading).
    pub mother: Option<i32>,
    pub pregnant: bool,
    pub pregnant_time: i32,
    pub can_have_milk: bool,
    pub milk_qty: f32,
    pub max_milk_actual: f32,
    pub milk_removed: i32,
    pub preferred_hutch_position: i32,
    pub hutch_position: i32,
    pub wool_qty: f32,
    pub fertilized_time: i32,
    pub fertilized: bool,
    pub eggs_today: i32,
    pub stress_level: f32,
    /// `playerAcceptanceList` in its `HashMap<Short, Float>` order.
    pub player_acceptance: Vec<(i16, f32)>,
    pub weight: f32,
    pub last_pregnancy_time: i64,
    pub last_milk_timer: i64,
    pub last_impregnate_time: i32,
    pub health: f32,
    pub virtual_id: f64,
    /// `migrationGroup` (`None` when unset; saved as `""`).
    pub migration_group: Option<String>,
    pub clutch_size: i32,
    pub on_hook: bool,
    /// The hook's square while the animal hangs on one (unset after
    /// loading).
    pub hook: Option<(i32, i32, i32)>,
    pub attach_back_to_hook_x: i32,
    pub attach_back_to_hook_y: i32,
    pub attach_back_to_hook_z: i32,
    pub pet_timer: f32,
    pub wild: bool,
    /// `getOnlineID()`: `IsoPlayer.onlineId` (1) unless a client sets it.
    pub online_id: i16,
    /// `geneticDisorder` from the loaded genome (`checkGeneticDisorder`).
    pub genetic_disorder: Vec<String>,
    /// The definition values saving reads.
    pub def: AnimalDefinition,
    /// `getBreed().woolType`.
    pub wool_type: Option<String>,
}

/// `HashMap<String, _>` iteration order of `keys` (insertion order).
fn string_map_order<T: Clone>(entries: &[(String, T)]) -> Vec<(String, T)> {
    let mut v = entries.to_vec();
    pz_inventory::java::sort_string_hash_map(&mut v, |e| e.0.as_str());
    v
}

impl IsoAnimal {
    /// `getUsedGene(name)`: a loaded gene's `used` flags are both clear
    /// (they are not saved), so the second allele is used.
    pub fn used_gene_value(&self, name: &str) -> Option<f32> {
        let name = name.to_lowercase();
        self.genome
            .iter()
            .find(|(k, _)| *k == name)
            .map(|(_, g)| g.allele2.current_value)
    }

    fn size_bound(&self, base: f32) -> f32 {
        let Some(v) = self.used_gene_value("maxSize") else {
            return base;
        };
        let mut base = base;
        if self.genetic_disorder.iter().any(|d| d == "dwarf") {
            base /= if self.def.baby { 1.5 } else { 2.2 };
        }
        base * v
    }

    /// `AnimalData.getMaxWool`.
    pub fn max_wool(&self) -> f32 {
        if self.def.max_wool <= 0.0 {
            return 0.0;
        }
        match self.used_gene_value("maxWool") {
            None => self.def.max_wool,
            Some(v) => self.def.max_wool * v,
        }
    }

    fn saves_wool(&self) -> bool {
        self.wool_type.is_some() && self.max_wool() > 0.0
    }

    /// `IsoAnimal.load` into `new IsoAnimal(cell)` (after the serialise
    /// flag and class id, which the hutch reads and drops).
    pub fn load(r: &mut Reader<'_>, wv: i32, ctx: &SaveCtx<'_>) -> SaveResult<Self> {
        let hooks = ctx.hooks;
        let stat_defs = &ctx.tables.character_stats;
        if stat_defs.is_empty() {
            return err("CharacterStat table not loaded");
        }
        let msb = r.i64()?;
        let lsb = r.i64()?;
        let animal_zone =
            (!(msb == 0 && lsb == 0) && hooks.animal_zone_exists(msb, lsb)).then_some((msb, lsb));
        // IsoWorld.saveoffsetx/y are 0 outside the editor.
        let x = r.f32()?;
        let y = r.f32()?;
        let z = r.f32()?;
        let dir = r.i32()? & 7;
        let mut stats = Stats::new(stat_defs);
        stats.load(stat_defs, r)?;
        let animal_type = r.string()?;
        let Some(def) = hooks.animal_definition(&animal_type) else {
            return err(format!("animal definition {animal_type} not found"));
        };
        let def = def.clone();
        let breed_name = r.string()?;
        let Some(breed) = def.breed(&breed_name).cloned() else {
            // init(null) draws a random breed.
            return err(format!("animal breed {animal_type}/{breed_name} not found"));
        };
        let custom_name = Some(r.string()?).filter(|s| !s.is_empty());
        let mod_data = LuaTable::load(r, wv)?;
        let item_id = r.i32()?;
        // init(breed): the genome AnimalGene.initGenome makes, which the
        // saved genes replace by name.
        let mut init_genes: Vec<String> = Vec::new();
        if let Some(genes) = &def.genes {
            for g in breed.forced_genes.iter().chain(genes.iter()) {
                let g = g.to_lowercase();
                if !init_genes.contains(&g) {
                    init_genes.push(g);
                }
            }
        }
        let female = r.bool()?;
        let animal_id = r.i32()?;
        let n = r.i32()?;
        let mut genome: Vec<(String, Option<AnimalGene>)> =
            init_genes.iter().map(|g| (g.clone(), None)).collect();
        for _ in 0..n.max(0) {
            let gene = AnimalGene::load(r)?;
            match genome.iter_mut().find(|(k, _)| *k == gene.name) {
                Some(e) => e.1 = Some(gene),
                None => genome.push((gene.name.clone(), Some(gene))),
            }
        }
        let mut full = Vec::new();
        for (k, g) in genome {
            let Some(g) = g else {
                // initGenome's random gene stays.
                return err(format!("animal gene {k} is not saved"));
            };
            full.push((k, g));
        }
        let genome = string_map_order(&full);
        let mut genetic_disorder: Vec<String> = Vec::new();
        for (_, g) in &genome {
            let d = &g.allele1.genetic_disorder;
            if !d.is_empty() && *d == g.allele2.genetic_disorder && !genetic_disorder.contains(d) {
                genetic_disorder.push(d.clone());
            }
        }
        let (mut tree_x, mut tree_y) = (i32::MAX, i32::MAX);
        if r.bool()? {
            tree_x = r.i32()?;
            tree_y = r.i32()?;
        }
        let age = r.i32()?;
        let hours_survived = r.f64()?;
        let time_since_last_update = r.i64()?;
        let saved_size = r.f32()?;
        let mut attach_back_to_mother = r.i32()?;
        if r.bool()? {
            attach_back_to_mother = r.i32()?;
        }
        let pregnant = r.bool()?;
        let pregnant_time = if pregnant { r.i32()? } else { 0 };
        let can_have_milk = r.bool()?;
        let milk_qty = r.f32()?;
        let max_milk_actual = r.f32()?;
        // setMilkQuantity from the constructor's 0: a smaller value draws
        // Rand and updates the milking time.
        if milk_qty < 0.0 {
            return err("animal with negative milk quantity");
        }
        let milk_qty = milk_qty.max(0.0).min(max_milk_actual);
        let milk_removed = r.i32()?;
        let preferred_hutch_position = r.i8()? as i32;
        let mut a = IsoAnimal {
            animal_zone,
            x,
            y,
            z,
            dir,
            stats,
            animal_type,
            breed: breed.name.clone(),
            custom_name,
            mod_data,
            item_id,
            female,
            animal_id,
            genome,
            attached_tree: None,
            attach_back_to_tree_x: tree_x,
            attach_back_to_tree_y: tree_y,
            age,
            hours_survived,
            time_since_last_update,
            size: 1.0,
            attach_back_to_mother,
            mother: None,
            pregnant,
            pregnant_time,
            can_have_milk,
            milk_qty,
            max_milk_actual,
            milk_removed,
            preferred_hutch_position,
            hutch_position: -1,
            wool_qty: 0.0,
            fertilized_time: 0,
            fertilized: false,
            eggs_today: 0,
            stress_level: 0.0,
            player_acceptance: Vec::new(),
            weight: 0.0,
            last_pregnancy_time: 0,
            last_milk_timer: 0,
            last_impregnate_time: 0,
            health: 1.0,
            virtual_id: 0.0,
            migration_group: None,
            clutch_size: 0,
            on_hook: false,
            hook: None,
            attach_back_to_hook_x: 0,
            attach_back_to_hook_y: 0,
            attach_back_to_hook_z: 0,
            pet_timer: 0.0,
            wild: def.wild,
            online_id: 1,
            genetic_disorder,
            wool_type: breed.wool_type.clone(),
            def,
        };
        // data.setSize: clamped to the genome's size range.
        a.size = clamp(
            saved_size,
            a.size_bound(a.def.min_size),
            a.size_bound(a.def.max_size),
        );
        if a.saves_wool() {
            a.wool_qty = r.f32()?;
        }
        a.fertilized_time = r.i32()?;
        a.fertilized = r.bool()?;
        if a.def.eggs_per_day > 0 {
            a.eggs_today = r.i32()?;
        }
        a.stress_level = r.f32()?;
        let n = r.i32()?;
        let mut acc: Vec<(i16, f32)> = Vec::new();
        for _ in 0..n.max(0) {
            let k = r.i16()?;
            let v = r.f32()?;
            match acc.iter_mut().find(|e| e.0 == k) {
                Some(e) => e.1 = v,
                None => acc.push((k, v)),
            }
        }
        pz_inventory::java::sort_int_hash_map(&mut acc, |e| e.0 as i32);
        a.player_acceptance = acc;
        a.weight = r.f32()?;
        a.last_pregnancy_time = r.i64()?;
        a.last_milk_timer = r.i64()?;
        a.last_impregnate_time = r.i32()?;
        // setHealth (animals are not invincible by default).
        a.health = r.f32()?;
        a.virtual_id = r.f64()?;
        a.migration_group = Some(r.string()?);
        a.clutch_size = r.i32()?;
        a.on_hook = r.bool()?;
        if a.on_hook {
            a.attach_back_to_hook_x = r.i32()?;
            a.attach_back_to_hook_y = r.i32()?;
            a.attach_back_to_hook_z = r.i32()?;
        }
        if wv >= 236 {
            a.pet_timer = r.f32()?;
        }
        if wv >= 245 {
            let wild = r.bool()?;
            a.set_wild(wild, hooks);
        }
        if wv >= 247 {
            // Only a client takes the saved online id.
            let _online_id = r.i16()?;
        }
        Ok(a)
    }

    /// `IsoAnimal.setWild`.
    pub fn set_wild(&mut self, b: bool, hooks: &dyn SaveHooks) {
        if !b && !self.def.can_be_domesticated {
            return;
        }
        if !b {
            self.migration_group = None;
            self.virtual_id = 0.0;
        } else {
            // setDebugStress(Rand.Next(50.0f, 80.0f)).
            self.stress_level = hooks.rand_next_float(50.0, 80.0);
        }
        self.wild = b;
    }

    /// `IsoAnimal.save(output, debug, serialize)`.
    pub fn save(&self, w: &mut Writer, ctx: &SaveCtx<'_>, serialize: bool) -> SaveResult<()> {
        if serialize {
            // Serialize() and factoryGetClassID("Animal").
            w.i8(1);
            w.i8(ctx.class_id("Animal")?);
        }
        let (msb, lsb) = self.animal_zone.unwrap_or((0, 0));
        w.i64(msb);
        w.i64(lsb);
        w.f32(self.x);
        w.f32(self.y);
        w.f32(self.z);
        w.i32(self.dir);
        self.stats.save(w);
        w.string(&self.animal_type);
        w.string(&self.breed);
        w.opt_string(self.custom_name.as_deref());
        self.mod_data.save(w);
        w.i32(self.item_id);
        w.bool(self.female);
        w.i32(self.animal_id);
        w.i32(self.genome.len() as i32);
        for (_, g) in &self.genome {
            g.save(w);
        }
        match self.attached_tree {
            Some((x, y)) => {
                w.i8(1);
                w.i32(x);
                w.i32(y);
            }
            None => w.i8(0),
        }
        w.i32(self.age);
        w.f64(self.hours_survived);
        // GameTime.getCalender().getTimeInMillis().
        w.i64(ctx.settings.calendar_millis);
        w.f32(self.size);
        w.i32(self.attach_back_to_mother);
        match self.mother {
            Some(id) => {
                w.i8(1);
                w.i32(id);
            }
            None => w.i8(0),
        }
        w.bool(self.pregnant);
        if self.pregnant {
            w.i32(self.pregnant_time);
        }
        w.bool(self.can_have_milk);
        w.f32(self.milk_qty);
        w.f32(self.max_milk_actual);
        w.i32(self.milk_removed);
        w.i8(self.preferred_hutch_position as i8);
        if self.saves_wool() {
            w.f32(self.wool_qty);
        }
        w.i32(self.fertilized_time);
        w.bool(self.fertilized);
        if self.def.eggs_per_day > 0 {
            w.i32(self.eggs_today);
        }
        w.f32(self.stress_level);
        w.i32(self.player_acceptance.len() as i32);
        for &(k, v) in &self.player_acceptance {
            w.i16(k);
            w.f32(v);
        }
        w.f32(self.weight);
        w.i64(self.last_pregnancy_time);
        w.i64(self.last_milk_timer);
        w.i32(self.last_impregnate_time);
        w.f32(self.health);
        w.f64(self.virtual_id);
        w.opt_string(self.migration_group.as_deref());
        w.i32(self.clutch_size);
        match (self.on_hook, self.hook) {
            (true, Some((hx, hy, hz))) => {
                w.i8(1);
                if self.attach_back_to_hook_x > 0 && self.attach_back_to_hook_y > 0 {
                    w.i32(self.attach_back_to_hook_x);
                    w.i32(self.attach_back_to_hook_y);
                    w.i32(self.attach_back_to_hook_z);
                } else {
                    w.i32(hx);
                    w.i32(hy);
                    w.i32(hz);
                }
            }
            _ => w.i8(0),
        }
        w.f32(self.pet_timer);
        w.bool(self.wild);
        w.i16(self.online_id);
        Ok(())
    }
}
