//! `IsoAnimal.load`/`save` for the live animal state, and the conversion
//! between it and the save codec's [`pz_world::save::animal::IsoAnimal`].
//!
//! [`Animal::load`] reads the saved bytes the way the original does, with
//! every `Rand` draw in its place (`init(breed)`, one draw per loaded gene
//! for `new AnimalGene()`, `setWild`). [`Animal::from_saved`] starts from a
//! record the codec already decoded: it makes the same draws except
//! `setWild`'s, which the codec makes through `SaveHooks` while decoding
//! (so for an animal saved wild that draw comes before the others instead
//! of after them).

use crate::animal::{Animal, NewAnimal};
use crate::env::{AnimalCharacter, AnimalEnv, CharacterValues};
use crate::genome::{AnimalAllele, AnimalGene};
use crate::java::{JResult, JavaException};
use pz_inventory::item as inv;
use pz_world::save::animal as codec;
use pz_world::save::io::Reader;
use pz_world::save::lua_table::LuaTable;

fn io<T>(r: pz_world::save::io::SaveResult<T>) -> JResult<T> {
    r.map_err(|e| JavaException(format!("BufferUnderflowException: {e}")))
}

fn allele_from(a: &inv::AnimalAllele) -> AnimalAllele {
    AnimalAllele {
        name: Some(a.name.clone()),
        current_value: a.current_value,
        true_ratio_value: a.true_ratio_value,
        dominant: a.dominant,
        used: false,
        genetic_disorder: Some(a.genetic_disorder.clone()),
    }
}

fn allele_to(a: &AnimalAllele) -> inv::AnimalAllele {
    inv::AnimalAllele {
        name: a.name.clone().unwrap_or_default(),
        current_value: a.current_value,
        true_ratio_value: a.true_ratio_value,
        dominant: a.dominant,
        genetic_disorder: a.genetic_disorder.clone().unwrap_or_default(),
    }
}

/// A saved gene as the live genome holds it (`AnimalGene.load`: both
/// alleles unused).
pub fn gene_from_saved(g: &inv::AnimalGene) -> AnimalGene {
    AnimalGene {
        name: Some(g.name.clone()),
        id: g.id,
        allele1: allele_from(&g.allele1),
        allele2: allele_from(&g.allele2),
    }
}

/// A live gene in the saved form.
pub fn gene_to_saved(g: &AnimalGene) -> inv::AnimalGene {
    inv::AnimalGene {
        id: g.id,
        name: g.name.clone().unwrap_or_default(),
        allele1: allele_to(&g.allele1),
        allele2: allele_to(&g.allele2),
    }
}

impl Animal {
    /// `new IsoAnimal(cell)` followed by `load(input, worldVersion)` (after
    /// the serialise flag and class id the caller reads). The animal's zone
    /// is kept only when [`AnimalWorld::animal_zone_exists`](crate::AnimalWorld)
    /// finds it. `IsoWorld.saveoffsetx/y` are 0 outside the editor.
    pub fn load(r: &mut Reader<'_>, wv: i32, env: &mut AnimalEnv<'_>) -> JResult<NewAnimal> {
        let ctx = env.ctx;
        let mut chr = CharacterValues::new(env.stat_defs.clone());
        let mut a = Animal::new(None);
        let msb = io(r.i64())?;
        let lsb = io(r.i64())?;
        a.animal_zone = (!(msb == 0 && lsb == 0) && env.world.animal_zone_exists(msb, lsb))
            .then_some((msb, lsb));
        chr.x = io(r.f32())?;
        chr.y = io(r.f32())?;
        chr.z = io(r.f32())?;
        chr.dir = io(r.i32())? & 7;
        for i in 0..chr.stats.len() {
            let v = io(r.f32())?;
            let d = &chr.stat_defs[i];
            chr.stats[i] = crate::java::clamp(v, d.minimum, d.maximum);
        }
        let animal_type = io(r.string())?;
        a.animal_type = Some(animal_type.clone());
        let Some(adef) = ctx.def(&animal_type) else {
            return crate::java::npe("adef");
        };
        let breed_name = io(r.string())?;
        let breed = adef.breed_by_name(&breed_name).cloned();
        let name = io(r.string())?;
        if !name.is_empty() {
            a.custom_name = Some(name);
        }
        a.mod_data = io(LuaTable::load(r, wv))?;
        a.item_id = io(r.i32())?;
        a.init(breed.as_ref(), &mut chr, env)?;
        chr.female = io(r.bool())?;
        a.animal_id = io(r.i32())?;
        let n = io(r.i32())?;
        for _ in 0..n.max(0) {
            // new AnimalGene(): the id initialiser draws.
            env.rand.next(1_000_000);
            let g = io(inv::AnimalGene::load(r))?;
            a.full_genome.insert(g.name.clone(), gene_from_saved(&g));
        }
        a.check_genetic_disorder();
        let (mut tx, mut ty) = (i32::MAX, i32::MAX);
        if io(r.bool())? {
            tx = io(r.i32())?;
            ty = io(r.i32())?;
        }
        a.attach_back_to_tree_x = tx;
        a.attach_back_to_tree_y = ty;
        a.data.age = io(r.i32())?;
        chr.hours_survived = io(r.f64())?;
        a.time_since_last_update = io(r.i64())?;
        let size = io(r.f32())?;
        a.data_set_size(size, ctx);
        a.attach_back_to_mother = io(r.i32())?;
        if io(r.bool())? {
            a.attach_back_to_mother = io(r.i32())?;
        }
        a.data.pregnant = io(r.bool())?;
        if a.data.pregnant {
            a.data.pregnant_time = io(r.i32())?;
        }
        a.data.can_have_milk = io(r.bool())?;
        let milk = io(r.f32())?;
        a.data.max_milk_actual = io(r.f32())?;
        a.data_set_milk_quantity(milk, ctx, env.rand);
        a.milk_removed = io(r.i32())?;
        a.data.preferred_hutch_position = io(r.i8())? as i32;
        if a.breed().is_some_and(|b| b.wool_type.is_some()) && a.max_wool(ctx) > 0.0 {
            a.data.wool_qty = io(r.f32())?;
        }
        a.data.fertilized_time = io(r.i32())?;
        a.data.fertilized = io(r.bool())?;
        if adef.eggs_per_day > 0 {
            a.data.eggs_today = io(r.i32())?;
        }
        a.stress_level = io(r.f32())?;
        let n = io(r.i32())?;
        for _ in 0..n.max(0) {
            let k = io(r.i16())?;
            let v = io(r.f32())?;
            a.set_debug_acceptance(k, v);
        }
        a.data.weight = io(r.f32())?;
        a.data.last_pregnancy_time = io(r.i64())?;
        a.data.last_milk_timer = io(r.i64())?;
        a.data.last_impregnate_time = io(r.i32())?;
        let h = io(r.f32())?;
        a.set_health(&mut chr, h);
        a.virtual_id = io(r.f64())?;
        a.migration_group = Some(io(r.string())?);
        a.data.clutch_size = io(r.i32())?;
        a.on_hook = io(r.bool())?;
        if a.on_hook {
            a.attach_back_to_hook_x = io(r.i32())?;
            a.attach_back_to_hook_y = io(r.i32())?;
            a.attach_back_to_hook_z = io(r.i32())?;
        }
        if wv >= 236 {
            a.pet_timer = io(r.f32())?;
        }
        if wv >= 245 {
            let wild = io(r.bool())?;
            a.set_wild(wild, ctx, env.rand);
        }
        if wv >= 247 {
            // Only a client takes the saved online id.
            io(r.i16())?;
        }
        Ok(NewAnimal { animal: a, chr })
    }

    /// The live animal from a record the codec decoded (see the module
    /// documentation for the one `Rand` draw that moves).
    pub fn from_saved(saved: &codec::IsoAnimal, env: &mut AnimalEnv<'_>) -> JResult<NewAnimal> {
        let ctx = env.ctx;
        let mut chr = CharacterValues::new(env.stat_defs.clone());
        let mut a = Animal::new(Some(saved.animal_type.clone()));
        a.animal_zone = saved.animal_zone;
        chr.x = saved.x;
        chr.y = saved.y;
        chr.z = saved.z;
        chr.dir = saved.dir;
        if saved.stats.0.len() == chr.stats.len() {
            chr.stats = saved.stats.0.clone();
        }
        let Some(adef) = ctx.def(&saved.animal_type) else {
            return crate::java::npe("adef");
        };
        let breed = adef.breed_by_name(&saved.breed).cloned();
        a.custom_name = saved.custom_name.clone();
        a.mod_data = saved.mod_data.clone();
        a.item_id = saved.item_id;
        a.init(breed.as_ref(), &mut chr, env)?;
        chr.female = saved.female;
        a.animal_id = saved.animal_id;
        for (name, g) in &saved.genome {
            env.rand.next(1_000_000);
            a.full_genome.insert(name.clone(), gene_from_saved(g));
        }
        a.check_genetic_disorder();
        a.attach_back_to_tree_x = saved.attach_back_to_tree_x;
        a.attach_back_to_tree_y = saved.attach_back_to_tree_y;
        a.data.age = saved.age;
        chr.hours_survived = saved.hours_survived;
        a.time_since_last_update = saved.time_since_last_update;
        a.data_set_size(saved.size, ctx);
        a.attach_back_to_mother = saved.attach_back_to_mother;
        a.data.pregnant = saved.pregnant;
        a.data.pregnant_time = saved.pregnant_time;
        a.data.can_have_milk = saved.can_have_milk;
        a.data.max_milk_actual = saved.max_milk_actual;
        a.data_set_milk_quantity(saved.milk_qty, ctx, env.rand);
        a.milk_removed = saved.milk_removed;
        a.data.preferred_hutch_position = saved.preferred_hutch_position;
        a.data.hutch_position = saved.hutch_position;
        a.data.wool_qty = saved.wool_qty;
        a.data.fertilized_time = saved.fertilized_time;
        a.data.fertilized = saved.fertilized;
        a.data.eggs_today = saved.eggs_today;
        a.stress_level = saved.stress_level;
        a.player_acceptance = saved.player_acceptance.clone();
        a.data.weight = saved.weight;
        a.data.last_pregnancy_time = saved.last_pregnancy_time;
        a.data.last_milk_timer = saved.last_milk_timer;
        a.data.last_impregnate_time = saved.last_impregnate_time;
        a.set_health(&mut chr, saved.health);
        a.virtual_id = saved.virtual_id;
        a.migration_group = saved.migration_group.clone();
        a.data.clutch_size = saved.clutch_size;
        a.on_hook = saved.on_hook;
        a.hook = saved.hook;
        a.attach_back_to_hook_x = saved.attach_back_to_hook_x;
        a.attach_back_to_hook_y = saved.attach_back_to_hook_y;
        a.attach_back_to_hook_z = saved.attach_back_to_hook_z;
        a.pet_timer = saved.pet_timer;
        a.wild = saved.wild;
        a.item_id = saved.item_id;
        chr.online_id = saved.online_id;
        Ok(NewAnimal { animal: a, chr })
    }

    /// The codec record `IsoAnimal.save` writes for this animal.
    /// `mother_id` is the mother's `animalId` when the animal has one,
    /// `hook` the hook's square while on a live hook.
    pub fn to_saved(
        &self,
        chr: &CharacterValues,
        ctx: crate::env::AnimalCtx<'_>,
        mother_id: Option<i32>,
    ) -> JResult<codec::IsoAnimal> {
        let adef = self.adef(ctx);
        let Some(breed) = self.breed() else {
            return crate::java::npe("breed");
        };
        let genome: Vec<(String, inv::AnimalGene)> = self
            .full_genome
            .iter()
            .map(|(k, g)| (k.clone(), gene_to_saved(g)))
            .collect();
        Ok(codec::IsoAnimal {
            animal_zone: self.animal_zone,
            x: chr.x,
            y: chr.y,
            z: chr.z,
            dir: chr.dir,
            stats: codec::Stats(chr.stats.clone()),
            animal_type: self.animal_type.clone().unwrap_or_default(),
            breed: breed.name.clone().unwrap_or_default(),
            custom_name: self.custom_name.clone(),
            mod_data: self.mod_data.clone(),
            item_id: self.item_id,
            female: chr.is_female(),
            animal_id: self.animal_id,
            genome,
            attached_tree: self
                .data
                .attached_tree
                .map(|_| (self.data.attached_tree_x, self.data.attached_tree_y)),
            attach_back_to_tree_x: self.attach_back_to_tree_x,
            attach_back_to_tree_y: self.attach_back_to_tree_y,
            age: self.data.age,
            hours_survived: chr.hours_survived,
            time_since_last_update: self.time_since_last_update,
            size: self.data.size,
            attach_back_to_mother: self.attach_back_to_mother,
            mother: mother_id,
            pregnant: self.data.pregnant,
            pregnant_time: self.data.pregnant_time,
            can_have_milk: self.data.can_have_milk,
            milk_qty: self.data.milk_qty,
            max_milk_actual: self.data.max_milk_actual,
            milk_removed: self.milk_removed,
            preferred_hutch_position: self.data.preferred_hutch_position,
            hutch_position: self.data.hutch_position,
            wool_qty: self.data.wool_qty,
            fertilized_time: self.data.fertilized_time,
            fertilized: self.data.fertilized,
            eggs_today: self.data.eggs_today,
            stress_level: self.stress_level,
            player_acceptance: self.player_acceptance_ordered(),
            weight: self.data.weight,
            last_pregnancy_time: self.data.last_pregnancy_time,
            last_milk_timer: self.data.last_milk_timer,
            last_impregnate_time: self.data.last_impregnate_time,
            health: chr.health(),
            virtual_id: self.virtual_id,
            migration_group: self.migration_group.clone(),
            clutch_size: self.data.clutch_size,
            on_hook: self.on_hook,
            hook: self.hook,
            attach_back_to_hook_x: self.attach_back_to_hook_x,
            attach_back_to_hook_y: self.attach_back_to_hook_y,
            attach_back_to_hook_z: self.attach_back_to_hook_z,
            pet_timer: self.pet_timer,
            wild: self.wild,
            online_id: chr.online_id(),
            genetic_disorder: self.genetic_disorder.clone(),
            def: adef.to_save_definition(&ctx.defs.genome),
            wool_type: breed.wool_type.clone(),
        })
    }
}
