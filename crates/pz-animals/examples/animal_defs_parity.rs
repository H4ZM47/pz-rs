//! Compares the animal definitions this crate loads from the game's Lua with
//! a dump of the original loaders (a local oracle run of
//! `AnimalDefinitions.loadAnimalDefinitions`, `loadTracksDefinitions`,
//! `loadMigrationsDefinitions`, `AnimalPartsDefinitions` and
//! `AnimalGene.initGenome`/`initGenesFromParents` with a seeded `Rand`).
//!
//! usage: animal_defs_parity <oracle dump> <game dir> [<rust dump out>]
//!
//! Every object prints as `<SimpleName field=value ...>` with the Java field
//! names sorted, floats as their IEEE bits, maps in iteration order (maps
//! keyed by `IsoDirections` by ordinal).

use std::path::PathBuf;

use pz_animals::genome::{init_genes_from_parents, init_genome};
use pz_animals::{
    parts, AnimalBreed, AnimalDefinitionTables, AnimalDefinitions, AnimalGene, AnimalPart,
    AnimalTracksDefinitions, AnimalTracksType, GeneDefinition, JHashMap, MigrationGroupDefinitions,
};
use pz_world::GameRand;

/// `java.util.Random`.
struct JavaRandom {
    seed: i64,
}

impl JavaRandom {
    fn new(seed: i64) -> Self {
        JavaRandom {
            seed: (seed ^ 0x5DEECE66D) & ((1 << 48) - 1),
        }
    }
    fn next_bits(&mut self, bits: u32) -> i32 {
        self.seed = (self.seed.wrapping_mul(0x5DEECE66D).wrapping_add(0xB)) & ((1 << 48) - 1);
        (self.seed >> (48 - bits)) as i32
    }
    fn next_int(&mut self, bound: i32) -> i32 {
        if (bound as u32).is_power_of_two() {
            return ((bound as i64 * self.next_bits(31) as i64) >> 31) as i32;
        }
        loop {
            let bits = self.next_bits(31);
            let val = bits % bound;
            if bits.wrapping_sub(val).wrapping_add(bound - 1) >= 0 {
                return val;
            }
        }
    }
}

impl GameRand for JavaRandom {
    fn next(&mut self, max: i32) -> i32 {
        if max <= 0 {
            0
        } else {
            self.next_int(max)
        }
    }
    fn next_float(&mut self) -> f32 {
        self.next_bits(24) as f32 / (1u32 << 24) as f32
    }
}

struct Obj {
    name: &'static str,
    fields: Vec<(&'static str, String)>,
}

impl Obj {
    fn new(name: &'static str) -> Self {
        Obj {
            name,
            fields: Vec::new(),
        }
    }
    fn f(mut self, k: &'static str, v: String) -> Self {
        self.fields.push((k, v));
        self
    }
    fn done(mut self) -> String {
        self.fields.sort_by(|a, b| a.0.cmp(b.0));
        let mut s = format!("<{}", self.name);
        for (k, v) in self.fields {
            s.push(' ');
            s.push_str(k);
            s.push('=');
            s.push_str(&v);
        }
        s.push('>');
        s
    }
}

fn s(v: &Option<String>) -> String {
    match v {
        Some(v) => format!("\"{v}\""),
        None => "null".into(),
    }
}
fn fl(v: f32) -> String {
    format!("f{:08x}", v.to_bits())
}
fn i(v: i32) -> String {
    v.to_string()
}
fn b(v: bool) -> String {
    v.to_string()
}
fn list<T>(v: &[T], f: impl Fn(&T) -> String) -> String {
    format!("[{}]", v.iter().map(f).collect::<Vec<_>>().join(","))
}
fn slist(v: &Option<Vec<String>>) -> String {
    match v {
        Some(v) => list(v, |x| format!("\"{x}\"")),
        None => "null".into(),
    }
}
fn map<V>(m: &JHashMap<V>, f: impl Fn(&V) -> String) -> String {
    let parts: Vec<String> = m.iter().map(|(k, v)| format!("\"{k}\"={}", f(v))).collect();
    format!("{{{}}}", parts.join(","))
}
fn omap<V>(m: &Option<JHashMap<V>>, f: impl Fn(&V) -> String) -> String {
    match m {
        Some(m) => map(m, f),
        None => "null".into(),
    }
}

fn gene_def(g: &GeneDefinition) -> String {
    Obj::new("AnimalGenomeDefinitions")
        .f("name", s(&g.name))
        .f("currentValue", fl(g.current_value))
        .f("ratios", omap(&g.ratios, |v| fl(*v)))
        .f("minValue", fl(g.min_value))
        .f("maxValue", fl(g.max_value))
        .f("forcedValues", b(g.forced_values))
        .done()
}

fn breed(br: &AnimalBreed) -> String {
    Obj::new("AnimalBreed")
        .f("name", s(&br.name))
        .f("texture", slist(&br.texture))
        .f("textureMale", s(&br.texture_male))
        .f("textureBaby", s(&br.texture_baby))
        .f("minWeightBonus", i(br.min_weight_bonus))
        .f("maxWeightBonus", i(br.max_weight_bonus))
        .f("milkType", s(&br.milk_type))
        .f("woolType", s(&br.wool_type))
        .f(
            "forcedGenes",
            omap(&br.forced_genes, |g| {
                Obj::new("ForcedGenes")
                    .f("name", s(&g.name))
                    .f("minValue", fl(g.min_value))
                    .f("maxValue", fl(g.max_value))
                    .done()
            }),
        )
        .f("invIconMale", s(&br.inv_icon_male))
        .f("invIconFemale", s(&br.inv_icon_female))
        .f("invIconBaby", s(&br.inv_icon_baby))
        .f("invIconMaleDead", s(&br.inv_icon_male_dead))
        .f("invIconFemaleDead", s(&br.inv_icon_female_dead))
        .f("invIconBabyDead", s(&br.inv_icon_baby_dead))
        .f("invIconMaleSkel", s(&br.inv_icon_male_skel))
        .f("invIconFemaleSkel", s(&br.inv_icon_female_skel))
        .f("invIconBabySkel", s(&br.inv_icon_baby_skel))
        .f("leather", s(&br.leather))
        .f("headItem", s(&br.head_item))
        .f("featherItem", s(&br.feather_item))
        .f("maxFeather", i(br.max_feather))
        .f(
            "sounds",
            map(&br.sounds, |x| {
                Obj::new("Sound")
                    .f("id", s(&x.id))
                    .f("soundName", s(&x.sound_name))
                    .f("intervalMin", i(x.interval_min))
                    .f("intervalMax", i(x.interval_max))
                    .f("slot", s(&x.slot))
                    .f("priority", i(x.priority))
                    .done()
            }),
        )
        .f("rottenTexture", s(&br.rotten_texture))
        .done()
}

fn animal_def(d: &AnimalDefinitions) -> String {
    Obj::new("AnimalDefinitions")
        .f("animalTypeStr", s(&d.animal_type_str))
        .f("bodyModelStr", s(&d.body_model_str))
        .f("bodyModelSkelNoHeadStr", s(&d.body_model_skel_no_head_str))
        .f("bodyModelFleeceStr", s(&d.body_model_fleece_str))
        .f("bodyModelSkelStr", s(&d.body_model_skel_str))
        .f("bodyModelHeadlessStr", s(&d.body_model_headless_str))
        .f("textureSkeleton", s(&d.texture_skeleton))
        .f("textureSkeletonBloody", s(&d.texture_skeleton_bloody))
        .f("textureRotten", s(&d.texture_rotten))
        .f("textureSkinned", s(&d.texture_skinned))
        .f("animset", s(&d.animset))
        .f("mate", s(&d.mate))
        .f("shadoww", fl(d.shadoww))
        .f("shadowfm", fl(d.shadowfm))
        .f("shadowbm", fl(d.shadowbm))
        .f("turnDelta", fl(d.turn_delta))
        .f("animalSize", fl(d.animal_size))
        .f("minSize", fl(d.min_size))
        .f("maxSize", fl(d.max_size))
        .f("minAge", i(d.min_age))
        .f("minEnclosureSize", i(d.min_enclosure_size))
        .f("babyType", s(&d.baby_type))
        .f("minAgeForBaby", i(d.min_age_for_baby))
        .f("maxAgeGeriatric", i(d.max_age_geriatric))
        .f("udder", b(d.udder))
        .f("female", b(d.female))
        .f("male", b(d.male))
        .f(
            "stages",
            list(&d.stages, |st| {
                Obj::new("AnimalGrowStage")
                    .f("ageToGrow", i(st.age_to_grow))
                    .f("nextStage", s(&st.next_stage))
                    .f("nextStageMale", s(&st.next_stage_male))
                    .f("stage", s(&st.stage))
                    .done()
            }),
        )
        .f("breeds", list(&d.breeds, breed))
        .f("genome", list(&d.genome, |_| String::new()))
        .f("alwaysFleeHumans", b(d.always_flee_humans))
        .f("fleeZombies", b(d.flee_zombies))
        .f("canBeAttached", b(d.can_be_attached))
        .f("canBeTransported", b(d.can_be_transported))
        .f("hungerMultiplier", fl(d.hunger_multiplier))
        .f("thirstMultiplier", fl(d.thirst_multiplier))
        .f("healthLossMultiplier", fl(d.health_loss_multiplier))
        .f("wanderMul", fl(d.wander_mul))
        .f("idleTypeNbr", i(d.idle_type_nbr))
        .f("eatingTypeNbr", i(d.eating_type_nbr))
        .f("sittingTypeNbr", i(d.sitting_type_nbr))
        .f("eatFromMother", b(d.eat_from_mother))
        .f("periodicRun", b(d.periodic_run))
        .f("pregnantPeriod", i(d.pregnant_period))
        .f("eatGrass", b(d.eat_grass))
        .f("sitRandomly", b(d.sit_randomly))
        .f("eatTypeTrough", slist(&d.eat_type_trough))
        .f("canBeMilked", b(d.can_be_milked))
        .f("minBaby", i(d.min_baby))
        .f("maxBaby", i(d.max_baby))
        .f("idleEmoteChance", i(d.idle_emote_chance))
        .f("eggsPerDay", i(d.eggs_per_day))
        .f("eggType", s(&d.egg_type))
        .f("fertilizedTimeMax", i(d.fertilized_time_max))
        .f("timeToHatch", i(d.time_to_hatch))
        .f("canBePicked", b(d.can_be_picked))
        .f("hutches", slist(&d.hutches))
        .f("enterHutchTime", i(d.enter_hutch_time))
        .f("exitHutchTime", i(d.exit_hutch_time))
        .f("genes", slist(&d.genes))
        .f("minMilk", fl(d.min_milk))
        .f("maxMilk", fl(d.max_milk))
        .f("maxWool", fl(d.max_wool))
        .f("minWeight", fl(d.min_weight))
        .f("maxWeight", fl(d.max_weight))
        .f("carcassItem", s(&d.carcass_item))
        .f("attackDist", i(d.attack_dist))
        .f("attackTimer", i(d.attack_timer))
        .f("dontAttackOtherMale", b(d.dont_attack_other_male))
        .f("canBeFeedByHand", b(d.can_be_feed_by_hand))
        .f("baseDmg", fl(d.base_dmg))
        .f("milkAnimPreset", s(&d.milk_anim_preset))
        .f("feedByHandType", slist(&d.feed_by_hand_type))
        .f("trailerBaseSize", fl(d.trailer_base_size))
        .f("canBePet", b(d.can_be_pet))
        .f("attackBack", b(d.attack_back))
        .f("collisionSize", fl(d.collision_size))
        .f("baseEncumbrance", fl(d.base_encumbrance))
        .f("matingPeriodStart", i(d.mating_period_start))
        .f("matingPeriodEnd", i(d.mating_period_end))
        .f("timeBeforeNextPregnancy", i(d.time_before_next_pregnancy))
        .f("thirstHungerTrigger", fl(d.thirst_hunger_trigger))
        .f("collidable", b(d.collidable))
        .f("canThump", b(d.can_thump))
        .f("wild", b(d.wild))
        .f("spottingDist", i(d.spotting_dist))
        .f("group", s(&d.group))
        .f("canBeAlerted", b(d.can_be_alerted))
        .f("dung", s(&d.dung))
        .f("attackIfStressed", b(d.attack_if_stressed))
        .f("happyAnim", i(d.happy_anim))
        .f("ropeBone", s(&d.rope_bone))
        .f("minClutchSize", i(d.min_clutch_size))
        .f("maxClutchSize", i(d.max_clutch_size))
        .f("layEggPeriodStart", i(d.lay_egg_period_start))
        .f("stressAboveGround", b(d.stress_above_ground))
        .f("canClimbStairs", b(d.can_climb_stairs))
        .f("stressUnderRain", b(d.stress_under_rain))
        .f("canClimbFences", b(d.can_climb_fences))
        .f("needMom", b(d.need_mom))
        .f("canBeDomesticated", b(d.can_be_domesticated))
        .f("dungChancePerDay", i(d.dung_chance_per_day))
        .f("hungerBoost", fl(d.hunger_boost))
        .f("thirstBoost", fl(d.thirst_boost))
        .f("distToEat", fl(d.dist_to_eat))
        .f("knockdownAttack", b(d.knockdown_attack))
        .f("minBodyPart", i(d.min_body_part))
        .f("canDoLaceration", b(d.can_do_laceration))
        .f("maxBlood", fl(d.max_blood))
        .f("minBlood", fl(d.min_blood))
        .f("litterEatTogether", b(d.litter_eat_together))
        .f("addTrackingXp", b(d.add_tracking_xp))
        .f("corpseSize", fl(d.corpse_size))
        .f("corpseLength", fl(d.corpse_length))
        .f("idleSoundRadius", fl(d.idle_sound_radius))
        .f("idleSoundVolume", fl(d.idle_sound_volume))
        .f(
            "wildFleeTimeUntilDeadTimer",
            fl(d.wild_flee_time_until_dead_timer),
        )
        .f(
            "canBeKilledWithoutWeapon",
            b(d.can_be_killed_without_weapon),
        )
        .f("feedByHandAnim", s(&d.feed_by_hand_anim))
        .done()
}

fn track_type(t: &AnimalTracksType) -> String {
    let sprites = match &t.sprites {
        None => "null".into(),
        Some(v) => {
            let mut v = v.clone();
            v.sort_by_key(|(d, _)| pz_world::IsoDirections::ALL.iter().position(|x| x == d));
            let parts: Vec<String> = v.iter().map(|(d, s)| format!("{d:?}=\"{s}\"")).collect();
            format!("{{{}}}", parts.join(","))
        }
    };
    Obj::new("AnimalTracksType")
        .f("type", s(&t.type_))
        .f("name", s(&t.name))
        .f("needDir", b(t.need_dir))
        .f("sprite", s(&t.sprite))
        .f("actionType", s(&t.action_type))
        .f("chanceToFindTrack", i(t.chance_to_find_track))
        .f("minSkill", i(t.min_skill))
        .f("chanceToSpawn", i(t.chance_to_spawn))
        .f("item", s(&t.item))
        .f("sprites", sprites)
        .done()
}

fn tracks_def(d: &AnimalTracksDefinitions) -> String {
    Obj::new("AnimalTracksDefinitions")
        .f("type", s(&d.type_))
        .f("tracks", omap(&d.tracks, track_type))
        .f("trackChance", omap(&d.track_chance, |v| i(*v)))
        .f("skillToIdentify", i(d.skill_to_identify))
        .f("trackType", s(&d.track_type))
        .f("chanceToFindTrack", i(d.chance_to_find_track))
        .done()
}

fn migration_def(d: &MigrationGroupDefinitions) -> String {
    let group = match &d.group {
        None => "null".into(),
        Some(g) => list(g, |x| {
            Obj::new("MigrationGroupDefinitionsGrouped")
                .f("animal", s(&x.animal))
                .f("chance", i(x.chance))
                .done()
        }),
    };
    Obj::new("MigrationGroupDefinitions")
        .f("type", s(&d.type_))
        .f("male", s(&d.male))
        .f("female", s(&d.female))
        .f("baby", s(&d.baby))
        .f("minAnimal", i(d.min_animal))
        .f("maxAnimal", i(d.max_animal))
        .f("maxMale", i(d.max_male))
        .f("babyChance", i(d.baby_chance))
        .f("minTimeBeforeEat", i(d.min_time_before_eat))
        .f("maxTimeBeforeEat", i(d.max_time_before_eat))
        .f("timeToEat", i(d.time_to_eat))
        .f("minTimeBeforeSleep", i(d.min_time_before_sleep))
        .f("maxTimeBeforeSleep", i(d.max_time_before_sleep))
        .f("sleepPeriodStart", slist(&d.sleep_period_start))
        .f("sleepPeriodEnd", slist(&d.sleep_period_end))
        .f("eatPeriodStart", slist(&d.eat_period_start))
        .f("eatPeriodEnd", slist(&d.eat_period_end))
        .f("timeToSleep", i(d.time_to_sleep))
        .f("speed", fl(d.speed))
        .f("trackChance", i(d.track_chance))
        .f("poopChance", i(d.poop_chance))
        .f("brokenTwigsChance", i(d.broken_twigs_chance))
        .f("herbGrazeChance", i(d.herb_graze_chance))
        .f("furChance", i(d.fur_chance))
        .f("flatHerbChance", i(d.flat_herb_chance))
        .f("possibleBreed", slist(&d.possible_breed))
        .f("group", group)
        .done()
}

fn part(p: &AnimalPart) -> String {
    Obj::new("AnimalPart")
        .f("item", s(&p.item))
        .f("minNb", i(p.min_nb))
        .f("maxNb", i(p.max_nb))
        .f("nb", i(p.nb))
        .done()
}

fn gene(g: &AnimalGene) -> String {
    let allele = |a: &pz_animals::AnimalAllele| {
        Obj::new("AnimalAllele")
            .f("name", s(&a.name))
            .f("currentValue", fl(a.current_value))
            .f("trueRatioValue", fl(a.true_ratio_value))
            .f("dominant", b(a.dominant))
            .f("used", b(a.used))
            .f("geneticDisorder", s(&a.genetic_disorder))
            .done()
    };
    Obj::new("AnimalGene")
        .f("name", s(&g.name))
        .f("id", i(g.id))
        .f("allele1", allele(&g.allele1))
        .f("allele2", allele(&g.allele2))
        .done()
}

fn dump(game: &PathBuf) -> Vec<String> {
    let fs = pz_fs::GameFs::new(game);
    let lua = pz_animals::lua::DefinitionsLua::load(&fs).expect("lua");
    for e in &lua.errors {
        eprintln!("lua error: {e}");
    }
    let env = lua.env();
    let t = AnimalDefinitionTables::load(&env).expect("definitions");
    let mut out = Vec::new();
    for (k, g) in t.genome.full_genome_def.iter() {
        out.push(format!("genomedef\t{k}\t{}", gene_def(g)));
    }
    out.push(format!("disorders\t{}", slist(&t.genome.genetic_disorder)));
    for (k, d) in t.animal_defs.iter() {
        out.push(format!("animaldef\t{k}\t{}", animal_def(d)));
    }
    for (k, d) in t.tracks.iter() {
        out.push(format!("tracksdef\t{k}\t{}", tracks_def(d)));
    }
    for (k, d) in t.migration.iter() {
        out.push(format!("migrationdef\t{k}\t{}", migration_def(d)));
    }
    let mut types: Vec<String> = Vec::new();
    if let Some(pd) = env.get_str("AnimalPartsDefinitions").as_table() {
        if let Some(animals) = pd.get_str("animals").as_table() {
            for (k, _) in animals.entries() {
                types.push(pz_animals::java::to_string(&k));
            }
        }
    }
    for k in t.animal_defs.keys() {
        if !types.contains(&k) {
            types.push(k);
        }
    }
    types.push("nosuchanimal".into());
    for ty in &types {
        out.push(format!(
            "parts\t{ty}\tleather={} parts={} bones={}",
            s(&parts::leather(&env, ty).unwrap()),
            list(&parts::all_parts_def(&env, ty).unwrap(), part),
            list(&parts::all_bones_def(&env, ty).unwrap(), part),
        ));
    }
    let mut seed = 1000i64;
    let mut prev: Option<JHashMap<AnimalGene>> = None;
    for (k, d) in t.animal_defs.iter() {
        for br in &d.breeds {
            let sd = seed;
            seed += 1;
            let mut rand = JavaRandom::new(sd);
            let res = init_genome(&t.genome, d, br, &mut rand).expect("initGenome");
            let next = rand.next(1_000_000);
            let g = match &res {
                Some(m) => map(m, gene),
                None => "null".into(),
            };
            let bn = br.name.clone().unwrap_or_else(|| "null".into());
            out.push(format!("initgenome\t{k}\t{bn}\t{sd}\t{g}\tnext={next}"));
            if let (Some(r), Some(p)) = (&res, &prev) {
                let s2 = seed;
                seed += 1;
                let mut rand = JavaRandom::new(s2);
                let child =
                    init_genes_from_parents(&t.genome, p, Some(r), &mut rand).expect("fromParents");
                let next = rand.next(1_000_000);
                out.push(format!(
                    "fromparents\t{s2}\t{}\tnext={next}",
                    map(&child, gene)
                ));
            }
            if res.is_some() {
                prev = res;
            }
        }
    }
    out
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let oracle = std::fs::read_to_string(&a[1]).expect("oracle dump");
    let game = PathBuf::from(&a[2]);
    let ours = dump(&game);
    if let Some(p) = a.get(3) {
        std::fs::write(p, ours.join("\n") + "\n").expect("write dump");
    }
    let theirs: Vec<&str> = oracle.lines().collect();
    let mut diffs = 0;
    for idx in 0..theirs.len().max(ours.len()) {
        let (t, o) = (theirs.get(idx).copied(), ours.get(idx).map(String::as_str));
        if t != o {
            diffs += 1;
            if diffs <= 10 {
                println!(
                    "line {}:\n  java: {}\n  rust: {}",
                    idx + 1,
                    t.unwrap_or("<none>"),
                    o.unwrap_or("<none>")
                );
            }
        }
    }
    let kinds = [
        "genomedef",
        "disorders",
        "animaldef",
        "tracksdef",
        "migrationdef",
        "parts",
        "initgenome",
        "fromparents",
    ];
    for k in kinds {
        let n = theirs
            .iter()
            .filter(|l| l.split('\t').next() == Some(k))
            .count();
        println!("{k}: {n} oracle lines");
    }
    println!(
        "{} oracle lines, {} rust lines, {diffs} differing",
        theirs.len(),
        ours.len()
    );
    if diffs > 0 {
        std::process::exit(1);
    }
}
