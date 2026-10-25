//! `AnimalGenomeDefinitions` (the Lua global of the same name, from
//! `media/lua/shared/Definitions/animal/AnimalGenomeDefinitions.lua`),
//! `AnimalGene` and `AnimalAllele`, and the genome generation and
//! inheritance rules (`AnimalGene.initGenome`, `initGenesFromParents`,
//! `doMutation`, `doRatio`).

use crate::breed::AnimalBreed;
use crate::definitions::AnimalDefinitions;
use crate::java::{self, as_table, entries, lower, JHashMap, JResult, JavaException};
use pz_kahlua::{TableRef, Value};
use pz_world::GameRand;

/// One gene's definition (an `AnimalGenomeDefinitions` instance).
#[derive(Clone, Debug, PartialEq)]
pub struct GeneDefinition {
    pub name: Option<String>,
    /// Never set by the loader.
    pub current_value: f32,
    pub ratios: Option<JHashMap<f32>>,
    pub min_value: f32,
    pub max_value: f32,
    pub forced_values: bool,
}

impl Default for GeneDefinition {
    fn default() -> Self {
        GeneDefinition {
            name: None,
            current_value: 0.0,
            ratios: None,
            min_value: 0.2,
            max_value: 0.6,
            forced_values: false,
        }
    }
}

/// The statics of `AnimalGenomeDefinitions`: `fullGenomeDef` and
/// `geneticDisorder`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AnimalGenomeDefinitions {
    pub full_genome_def: JHashMap<GeneDefinition>,
    /// `None` when the Lua global is missing (the original leaves the list
    /// `null`).
    pub genetic_disorder: Option<Vec<String>>,
}

impl AnimalGenomeDefinitions {
    /// `loadGenomeDefinition()` over a Lua environment.
    pub fn load(env: &TableRef) -> JResult<AnimalGenomeDefinitions> {
        let mut out = AnimalGenomeDefinitions::default();
        let Some(definitions) = env.get_str("AnimalGenomeDefinitions").as_table().cloned() else {
            return Ok(out);
        };
        let genes = as_table(&definitions.rawget(&Value::str("genes")))?;
        for (k, v) in entries(&genes) {
            let name = lower(&java::to_string(&k));
            let mut def = GeneDefinition {
                name: Some(name.clone()),
                ..Default::default()
            };
            for (k2, v2) in entries(&as_table(&v)?) {
                let key = match &k2 {
                    Value::Str(s) => s.to_string(),
                    other => {
                        return Err(JavaException(format!(
                            "ClassCastException: {} cannot be cast to String",
                            java::to_string(other)
                        )))
                    }
                };
                let s = java::to_string_trim(&v2);
                match lower(&key).as_str() {
                    "minvalue" => def.min_value = java::parse_float(&s)?,
                    "maxvalue" => def.max_value = java::parse_float(&s)?,
                    "forcedvalues" => def.forced_values = java::parse_bool(&s),
                    "ratio" => {
                        let mut ratios = JHashMap::new();
                        for (rk, rv) in entries(&as_table(&v2)?) {
                            let rname = lower(&java::to_string(&rk));
                            ratios.insert(rname, java::parse_float(&java::to_string_trim(&rv))?);
                        }
                        def.ratios = Some(ratios);
                    }
                    _ => {}
                }
            }
            out.full_genome_def.insert(name, def);
        }
        let disorder = as_table(&definitions.rawget(&Value::str("geneticDisorder")))?;
        let mut list: Vec<String> = Vec::new();
        for (k, _) in entries(&disorder) {
            let name = lower(&java::to_string(&k));
            if !list.contains(&name) {
                list.push(name);
            }
        }
        out.genetic_disorder = Some(list);
        Ok(out)
    }

    fn disorders(&self) -> JResult<&[String]> {
        self.genetic_disorder
            .as_deref()
            .ok_or_else(|| JavaException("NullPointerException: geneticDisorder".into()))
    }
}

/// `AnimalAllele`.
#[derive(Clone, Debug, PartialEq)]
pub struct AnimalAllele {
    pub name: Option<String>,
    pub current_value: f32,
    pub true_ratio_value: f32,
    pub dominant: bool,
    pub used: bool,
    pub genetic_disorder: Option<String>,
}

impl Default for AnimalAllele {
    fn default() -> Self {
        AnimalAllele {
            name: None,
            current_value: 0.0,
            true_ratio_value: 0.0,
            dominant: true,
            used: false,
            genetic_disorder: None,
        }
    }
}

impl AnimalAllele {
    /// The saved form (`save`/`load` write name, current value, true ratio
    /// value, dominant and genetic disorder; `used` is not saved).
    pub fn to_save(&self) -> pz_world::save::visual::AnimalAllele {
        pz_world::save::visual::AnimalAllele {
            name: self.name.clone().unwrap_or_default(),
            current_value: self.current_value,
            true_ratio_value: self.true_ratio_value,
            dominant: self.dominant,
            genetic_disorder: self.genetic_disorder.clone().unwrap_or_default(),
        }
    }

    pub fn from_save(a: &pz_world::save::visual::AnimalAllele) -> AnimalAllele {
        AnimalAllele {
            name: Some(a.name.clone()),
            current_value: a.current_value,
            true_ratio_value: a.true_ratio_value,
            dominant: a.dominant,
            used: false,
            genetic_disorder: Some(a.genetic_disorder.clone()),
        }
    }
}

/// `AnimalGene`.
#[derive(Clone, Debug, PartialEq)]
pub struct AnimalGene {
    pub name: Option<String>,
    pub id: i32,
    pub allele1: AnimalAllele,
    pub allele2: AnimalAllele,
}

/// `Rand.NextBool(n)`.
fn next_bool(rand: &mut dyn GameRand, n: i32) -> bool {
    rand.next(n) == 0
}

/// `Math.max(float, float)`.
fn java_max(a: f32, b: f32) -> f32 {
    if a.is_nan() {
        return a;
    }
    if a == 0.0 && b == 0.0 && a.is_sign_negative() {
        return b;
    }
    if a >= b {
        a
    } else {
        b
    }
}

fn is_null_or_empty(s: &Option<String>) -> bool {
    s.as_deref().is_none_or(str::is_empty)
}

impl AnimalGene {
    /// `new AnimalGene()`: the id is drawn from `Rand` by the field
    /// initialiser.
    pub fn new(rand: &mut dyn GameRand) -> AnimalGene {
        AnimalGene {
            name: None,
            id: rand.next(1_000_000),
            allele1: AnimalAllele::default(),
            allele2: AnimalAllele::default(),
        }
    }

    /// `getUsedGene()`.
    pub fn used_gene(&self) -> &AnimalAllele {
        if self.allele1.used {
            &self.allele1
        } else {
            &self.allele2
        }
    }

    /// `initUsedGene()`.
    pub fn init_used_gene(&mut self, rand: &mut dyn GameRand) {
        let nb = if self.allele1.dominant {
            if self.allele2.dominant {
                rand.next(2)
            } else {
                0
            }
        } else if self.allele2.dominant {
            1
        } else {
            rand.next(2)
        };
        if nb == 0 {
            self.allele1.used = true;
        } else {
            self.allele2.used = true;
        }
    }

    /// The saved form.
    pub fn to_save(&self) -> pz_world::save::visual::AnimalGene {
        pz_world::save::visual::AnimalGene {
            id: self.id,
            name: self.name.clone().unwrap_or_default(),
            allele1: self.allele1.to_save(),
            allele2: self.allele2.to_save(),
        }
    }

    pub fn from_save(g: &pz_world::save::visual::AnimalGene) -> AnimalGene {
        AnimalGene {
            name: Some(g.name.clone()),
            id: g.id,
            allele1: AnimalAllele::from_save(&g.allele1),
            allele2: AnimalAllele::from_save(&g.allele2),
        }
    }
}

/// `AnimalGene.initAllele(name, forcedDominant, value, forcedValues)`.
fn init_allele(
    genome: &AnimalGenomeDefinitions,
    name: &str,
    forced_dominant: bool,
    mut value: f32,
    forced_values: bool,
    rand: &mut dyn GameRand,
) -> JResult<AnimalAllele> {
    if value < 0.0 {
        value = 0.0;
    }
    let mut a = AnimalAllele::default();
    if forced_dominant {
        a.dominant = true;
    } else {
        if !forced_values && rand.next(100) < 15 {
            value = rand.next_float_range(0.0, 1.0);
        }
        let avg = ((value - 0.5f32).abs()) as f64;
        let rec_chance = if avg > 0.45 {
            90
        } else if avg > 0.4 {
            80
        } else if avg > 0.3 {
            75
        } else if avg > 0.2 {
            60
        } else if avg > 0.15 {
            50
        } else if avg > 0.1 {
            40
        } else {
            30
        };
        a.dominant = rand.next(100) > rec_chance;
    }
    a.name = Some(name.to_string());
    a.current_value = value;
    do_mutation(genome, &mut a, rand)?;
    Ok(a)
}

/// `AnimalGene.doMutation(allele)`.
pub fn do_mutation(
    genome: &AnimalGenomeDefinitions,
    allele: &mut AnimalAllele,
    rand: &mut dyn GameRand,
) -> JResult<()> {
    if rand.next(100) <= 10 {
        allele.current_value += if next_bool(rand, 2) { 0.05 } else { -0.05 };
    }
    if rand.next(100) <= 5 {
        allele.current_value += 0.2;
    }
    if rand.next(100) <= 5 {
        allele.dominant = !allele.dominant;
    }
    if is_null_or_empty(&allele.genetic_disorder) && rand.next(100) <= 2 {
        let list = genome.disorders()?;
        let i = rand.next_range(0, list.len() as i32);
        let d = list.get(i as usize).ok_or_else(|| {
            JavaException(format!(
                "IndexOutOfBoundsException: Index {i} out of bounds for length {}",
                list.len()
            ))
        })?;
        allele.genetic_disorder = Some(d.clone());
    }
    if rand.next(100) <= 2 {
        allele.genetic_disorder = None;
    }
    if allele.current_value <= 0.05 {
        allele.current_value = 0.05;
    }
    Ok(())
}

/// `AnimalGene.doRatio(def, fullGenome, allele)` with the used allele's
/// current value.
pub fn do_ratio(
    def: &GeneDefinition,
    full_genome: &mut JHashMap<AnimalGene>,
    allele_value: f32,
    rand: &mut dyn GameRand,
) -> JResult<()> {
    let Some(ratios) = &def.ratios else {
        return Ok(());
    };
    for s in ratios.keys() {
        let name = lower(&s);
        let value = *ratios
            .get(&name)
            .ok_or_else(|| JavaException("NullPointerException: ratio".into()))?;
        let Some(affected) = full_genome.get_mut(&name) else {
            continue;
        };
        let new_value = value - allele_value;
        let r = rand.next_float_range(0.01, 0.05);
        affected.allele1.true_ratio_value = java_max(r, affected.allele1.current_value + new_value);
        let r = rand.next_float_range(0.01, 0.05);
        affected.allele2.true_ratio_value = java_max(r, affected.allele2.current_value + new_value);
    }
    Ok(())
}

fn apply_ratios(
    genome: &AnimalGenomeDefinitions,
    full: &mut JHashMap<AnimalGene>,
    rand: &mut dyn GameRand,
) -> JResult<()> {
    for s in full.keys() {
        let gene = full.get(&s).expect("key");
        let gene_name = gene.name.clone().unwrap_or_default();
        let def = genome
            .full_genome_def
            .get(&gene_name)
            .ok_or_else(|| {
                JavaException(format!(
                    "NullPointerException: no genome definition {gene_name}"
                ))
            })?
            .clone();
        let base = if gene.allele2.used {
            gene.allele2.current_value
        } else {
            gene.allele1.current_value
        };
        do_ratio(&def, full, base, rand)?;
    }
    Ok(())
}

/// `AnimalGene.initGenome(animal)` for an animal of definition `def` and
/// breed `breed`: the new `fullGenome`, or `None` when the definition has
/// no `genes` (the original then leaves the animal's genome as it was).
pub fn init_genome(
    genome: &AnimalGenomeDefinitions,
    def: &AnimalDefinitions,
    breed: &AnimalBreed,
    rand: &mut dyn GameRand,
) -> JResult<Option<JHashMap<AnimalGene>>> {
    let Some(genes) = &def.genes else {
        return Ok(None);
    };
    let mut full: JHashMap<AnimalGene> = JHashMap::new();
    if let Some(forced) = &breed.forced_genes {
        for key in forced.keys() {
            let mut set = AnimalGene::new(rand);
            let name = lower(&key);
            set.name = Some(name.clone());
            let value = forced
                .get(&name)
                .ok_or_else(|| JavaException("NullPointerException: forced gene".into()))?;
            let Some(gdef) = genome.full_genome_def.get(&name) else {
                continue;
            };
            for j in 0..2 {
                let v = rand.next_float_range(value.min_value, value.max_value);
                let a = init_allele(genome, &name, true, v, gdef.forced_values, rand)?;
                if j == 0 {
                    set.allele1 = a;
                } else {
                    set.allele2 = a;
                }
            }
            set.init_used_gene(rand);
            full.insert(name, set);
        }
    }
    for g in genes {
        let mut set = AnimalGene::new(rand);
        let name = lower(g);
        set.name = Some(name.clone());
        if full.contains_key(&name) {
            continue;
        }
        let Some(gdef) = genome.full_genome_def.get(&name) else {
            continue;
        };
        for j in 0..2 {
            let v = rand.next_float_range(gdef.min_value, gdef.max_value);
            let a = init_allele(genome, &name, false, v, gdef.forced_values, rand)?;
            if j == 0 {
                set.allele1 = a;
            } else {
                set.allele2 = a;
            }
        }
        set.init_used_gene(rand);
        full.insert(name, set);
    }
    apply_ratios(genome, &mut full, rand)?;
    Ok(Some(full))
}

/// `AnimalGene.initGenesFromParents(femaleGenome, maleGenome)`.
pub fn init_genes_from_parents(
    genome: &AnimalGenomeDefinitions,
    female: &JHashMap<AnimalGene>,
    male: Option<&JHashMap<AnimalGene>>,
    rand: &mut dyn GameRand,
) -> JResult<JHashMap<AnimalGene>> {
    let male = match male {
        Some(m) if !m.is_empty() => m,
        _ => female,
    };
    let mut full: JHashMap<AnimalGene> = JHashMap::new();
    for name in female.keys() {
        let fg = female.get(&name).expect("key");
        let mg = male.get(&name).unwrap_or(fg);
        let af = if next_bool(rand, 2) {
            &fg.allele1
        } else {
            &fg.allele2
        };
        let am = if next_bool(rand, 2) {
            &mg.allele1
        } else {
            &mg.allele2
        };
        let mut gene = AnimalGene::new(rand);
        gene.name = Some(name.clone());
        let copy = |a: &AnimalAllele| AnimalAllele {
            name: a.name.clone(),
            current_value: a.current_value,
            dominant: a.dominant,
            genetic_disorder: a.genetic_disorder.clone(),
            ..Default::default()
        };
        gene.allele1 = copy(af);
        gene.allele2 = copy(am);
        gene.init_used_gene(rand);
        full.insert(name, gene);
    }
    for name in full.keys() {
        let gene = full.get_mut(&name).expect("key");
        do_mutation(genome, &mut gene.allele1, rand)?;
        do_mutation(genome, &mut gene.allele2, rand)?;
    }
    apply_ratios(genome, &mut full, rand)?;
    Ok(full)
}

/// `AnimalGene.checkGeneticDisorder(animal)`: adds to `disorders` each
/// disorder carried by both alleles of a gene, visiting `genome_list`
/// (`getFullGenomeList()`) in order.
pub fn check_genetic_disorder(genome_list: &[&AnimalGene], disorders: &mut Vec<String>) {
    for gene in genome_list {
        let Some(d) = gene
            .allele1
            .genetic_disorder
            .as_deref()
            .filter(|d| !d.is_empty())
        else {
            continue;
        };
        if gene.allele2.genetic_disorder.as_deref() != Some(d) || disorders.iter().any(|x| x == d) {
            continue;
        }
        disorders.push(d.to_string());
    }
}
