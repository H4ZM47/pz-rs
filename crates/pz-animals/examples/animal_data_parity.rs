//! Replays a local oracle run of the original `AnimalData` / `IsoAnimal`
//! code and compares every traced state.
//!
//! ```text
//! animal_data_parity sim   <oracle sim dump> <game dir>
//! animal_data_parity blobs <oracle sim dump> <game dir> <blobs out> <save dir>...
//! animal_data_parity save  <oracle sim dump> <game dir> <blobs> <oracle save dump>
//! ```
//!
//! * `sim`: the oracle built an `IsoAnimal` for every definition and breed
//!   with a seeded `Rand` and `init(breed)`, then ran `AnimalData.update()`
//!   (and the pet timer and `updateStress()` of `updateInternal`) for
//!   hundreds of simulated hours with scripted starving, feeding,
//!   fertilising, milking, shearing, stress and `updateStatsAway` events,
//!   tracing every field (floats as bits) and the next `Rand` draw, and
//!   every animal created on the way (babies, grown animals). The clock
//!   values of each step come from the trace; the time zone is UTC.
//! * `blobs`: extracts the saved `IsoAnimal`s of the `apop/apop_*.bin`
//!   files of save copies (`AnimalCell` → `AnimalChunk` → `VirtualAnimal`),
//!   for the oracle to load and save again.
//! * `save`: loads every blob with [`Animal::load`] (seeded like the
//!   oracle), converts it to the codec record and saves it with the codec;
//!   compares the bytes, the state and the next `Rand` draw with the
//!   original's `load` + `save`, and the bytes with the blob.
//!
//! The oracle and its data are local only and never committed.

use std::path::{Path, PathBuf};
use std::rc::Rc;

use pz_animals::env::{AnimalWorld, NoWorld};
use pz_animals::{
    Animal, AnimalCharacter, AnimalClock, AnimalCtx, AnimalDefinitionTables, AnimalEnv,
    AnimalSandbox, AnimalStat, CharacterValues, JavaException, Male, NewAnimal, ObjectRef,
};
use pz_core::calendar::{calendar_fields, JavaZone};
use pz_world::save::animal::CharacterStatDecl;
use pz_world::save::io::{Reader, Writer};
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

/// The traced `GameTime` values of one step.
#[derive(Clone, Copy, Default)]
struct Clock {
    hour: i32,
    month: i32,
    millis: i64,
    mult: f32,
    wad: f64,
}

impl AnimalClock for Clock {
    fn hour(&self) -> i32 {
        self.hour
    }
    fn month(&self) -> i32 {
        self.month
    }
    fn calendar_millis(&self) -> i64 {
        self.millis
    }
    fn multiplier(&self) -> f32 {
        self.mult
    }
    fn world_age_days_since_begin(&self) -> f64 {
        self.wad
    }
    fn calendar_month_hour(&self, millis: i64) -> (i32, i32) {
        let f = calendar_fields(&JavaZone::utc(), millis);
        (f.month, f.hour_of_day)
    }
}

fn parse_clock(s: &str) -> Clock {
    let mut c = Clock::default();
    for kv in s.split(' ') {
        let (k, v) = kv.split_once('=').unwrap();
        match k {
            "h" => c.hour = v.parse().unwrap(),
            "mo" => c.month = v.parse().unwrap(),
            "ms" => c.millis = v.parse().unwrap(),
            "mul" => c.mult = f32::from_bits(u32::from_str_radix(v, 16).unwrap()),
            "wad" => c.wad = f64::from_bits(u64::from_str_radix(v, 16).unwrap()),
            _ => {}
        }
    }
    c
}

fn sandbox(variant: &str) -> AnimalSandbox {
    let v: Vec<i32> = variant.split(',').map(|x| x.parse().unwrap()).collect();
    AnimalSandbox {
        animal_age_modifier: v[0],
        animal_milk_inc_modifier: v[1],
        animal_wool_inc_modifier: v[2],
        animal_pregnancy_time: v[3],
        animal_meta_stats_modifier: v[4],
        animal_stats_modifier: v[5],
        animal_mating_season: v[6] == 1,
        animal_meta_predator: v[7] == 1,
        ultra_speed_doesnot_affect_to_animals: v[8] == 1,
    }
}

/// The animals the simulated world receives (babies, grown animals).
#[derive(Default)]
struct SimWorld {
    created: Vec<NewAnimal>,
    /// Indices into `created` of animals the oracle printed as new.
    fresh: Vec<usize>,
}

impl AnimalWorld for SimWorld {
    fn animal_mut(&mut self, animal: ObjectRef) -> Option<(&mut Animal, &mut dyn AnimalCharacter)> {
        let n = self.created.get_mut((animal as usize).checked_sub(1)?)?;
        Some((&mut n.animal, &mut n.chr))
    }
    fn add_new_animal(&mut self, animal: NewAnimal, _into_vehicle: Option<ObjectRef>) -> ObjectRef {
        self.created.push(animal);
        self.fresh.push(self.created.len() - 1);
        self.created.len() as ObjectRef
    }
    fn replace_grown_animal(&mut self, grown: NewAnimal) -> Option<ObjectRef> {
        Some(self.add_new_animal(grown, None))
    }
}

fn f(x: f32) -> String {
    format!("{:08x}", x.to_bits())
}

fn d(x: f64) -> String {
    format!("{:016x}", x.to_bits())
}

fn b(x: bool) -> u8 {
    x as u8
}

fn opt(s: &Option<String>) -> String {
    s.clone().unwrap_or_else(|| "null".into())
}

/// The oracle's `state(animal)` line.
fn state(a: &Animal, c: &CharacterValues) -> String {
    let dt = &a.data;
    let mut s = String::new();
    let mut p = |k: &str, v: String| {
        if !s.is_empty() {
            s.push(' ');
        }
        s.push_str(k);
        s.push('=');
        s.push_str(&v);
    };
    p("id", a.animal_id.to_string());
    p("type", opt(&a.animal_type));
    p(
        "breed",
        dt.breed.as_ref().map_or("null".into(), |b| opt(&b.name)),
    );
    p("female", b(c.female).to_string());
    p("health", f(c.health));
    p("hours", d(c.hours_survived));
    p("hunger", f(c.stat(AnimalStat::Hunger)));
    p("thirst", f(c.stat(AnimalStat::Thirst)));
    p("stress", f(a.stress_level));
    p("wild", b(a.wild).to_string());
    p("pet", f(a.pet_timer));
    p("milkRemoved", a.milk_removed.to_string());
    p("small", b(a.small_enclosure).to_string());
    p("dis", format!("[{}]", a.genetic_disorder.join(", ")));
    p(
        "acc",
        a.player_acceptance_ordered()
            .iter()
            .map(|(k, v)| format!("{k}:{},", f(*v)))
            .collect(),
    );
    p("sedt", f(c.state_event_delay_timer));
    p("skin", opt(&a.skin_texture_name));
    p("tsl", a.time_since_last_update.to_string());
    p("fromMeta", b(a.from_meta).to_string());
    p("attackedTimer", a.attacked_timer.to_string());
    p(
        "babies",
        a.babies
            .as_ref()
            .map_or("null".into(), |v| v.len().to_string()),
    );
    p("milk", f(dt.milk_qty));
    p("wool", f(dt.wool_qty));
    p("canMilk", b(dt.can_have_milk).to_string());
    p("weight", f(dt.weight));
    p("size", f(dt.size));
    p("osize", f(dt.original_size));
    p("age", dt.age.to_string());
    p("stageNbr", dt.current_stage_nbr.to_string());
    p("lhc", dt.last_hour_check.to_string());
    p(
        "stage",
        dt.current_stage
            .as_ref()
            .map_or("null".into(), |s| opt(&s.stage)),
    );
    p("preg", b(dt.pregnant).to_string());
    p("pregT", dt.pregnant_time.to_string());
    p("maxMilkA", f(dt.max_milk_actual));
    p("eggsToday", dt.eggs_today.to_string());
    p("eggTime", dt.egg_time.to_string());
    p("fert", b(dt.fertilized).to_string());
    p("fertT", dt.fertilized_time.to_string());
    p(
        "maleG",
        dt.male_genome
            .as_ref()
            .map_or("null".into(), |g| g.len().to_string()),
    );
    p("hutchPos", dt.hutch_position.to_string());
    p("prefHutch", dt.preferred_hutch_position.to_string());
    p("troughT", dt.trough_path_timer.to_string());
    p("lastMilk", dt.last_milk_timer.to_string());
    p("lastPreg", dt.last_pregnancy_time.to_string());
    p("lastImpreg", dt.last_impregnate_time.to_string());
    p("clutch", dt.clutch_size.to_string());
    p("clutchDone", b(dt.clutch_size_done).to_string());
    p("toInsem", dt.animal_to_inseminate.len().to_string());
    p("treeX", dt.attached_tree_x.to_string());
    p("treeY", dt.attached_tree_y.to_string());
    let allele = |x: &pz_animals::AnimalAllele| {
        format!(
            "{}/{}/{}/{}/{}",
            f(x.current_value),
            f(x.true_ratio_value),
            b(x.dominant),
            b(x.used),
            opt(&x.genetic_disorder)
        )
    };
    let genome: Vec<String> = a
        .full_genome
        .iter()
        .map(|(k, g)| format!("{k}:{}:{}:{}", g.id, allele(&g.allele1), allele(&g.allele2)))
        .collect();
    p("genome", format!("{{{}}}", genome.join(";")));
    s
}

/// `exc=<ExceptionClass>`: the oracle's message is not compared.
fn exc_class(e: &str) -> String {
    let e = e.strip_prefix("exc=").unwrap_or(e);
    let e = e.strip_prefix("evexc=").unwrap_or(e);
    e.split(':').next().unwrap_or("").trim().to_string()
}

struct Setup {
    sprites: pz_world::IsoSpriteManager,
    defs: AnimalDefinitionTables,
    stats: Rc<[CharacterStatDecl]>,
    class_id: i8,
}

fn setup(oracle: &str, game: &Path) -> Setup {
    let fs = pz_fs::GameFs::new(game);
    let lua = pz_animals::lua::DefinitionsLua::load(&fs).expect("lua");
    let defs = AnimalDefinitionTables::load(&lua.env()).expect("definitions");
    let mut stats = Vec::new();
    let mut class_id = 0;
    for l in oracle.lines() {
        let x: Vec<&str> = l.split('\t').collect();
        match x[0] {
            "stat" => stats.push(CharacterStatDecl {
                field: x[1].into(),
                id: x[2].into(),
                minimum: x[3].parse().unwrap(),
                maximum: x[4].parse().unwrap(),
                default: x[5].parse().unwrap(),
            }),
            "classid" => class_id = x[2].parse().unwrap(),
            _ => {}
        }
    }
    let tables = pz_world::SpriteTables::from_game_jar(&game.join("projectzomboid.jar"))
        .expect("sprite tables");
    Setup {
        sprites: pz_world::IsoSpriteManager::new(tables),
        defs,
        stats: stats.into(),
        class_id,
    }
}

/// Counts and the first differences.
#[derive(Default)]
struct Tally {
    lines: usize,
    diffs: usize,
}

impl Tally {
    fn check(&mut self, what: &str, java: &str, rust: &str) {
        self.lines += 1;
        if java != rust {
            self.diffs += 1;
            if self.diffs <= 8 {
                let jf: Vec<&str> = java.split(' ').collect();
                let rf: Vec<&str> = rust.split(' ').collect();
                let mut shown = 0;
                println!("DIFF {what}");
                for i in 0..jf.len().max(rf.len()) {
                    let (j, r) = (
                        jf.get(i).copied().unwrap_or("-"),
                        rf.get(i).copied().unwrap_or("-"),
                    );
                    if j != r && shown < 6 {
                        shown += 1;
                        println!("   java {j}\n   rust {r}");
                    }
                }
            }
        }
    }
}

struct Current {
    animal: Animal,
    chr: CharacterValues,
    rand: JavaRandom,
    sandbox: AnimalSandbox,
    world: SimWorld,
    dead: bool,
}

fn run_sim(oracle: &str, s: &Setup) {
    let mut t = Tally::default();
    let mut sb = sandbox("4,4,4,2,4,4,1,0,0");
    let mut cur: Option<Current> = None;
    let mut counts = std::collections::BTreeMap::<&str, usize>::new();
    for line in oracle.lines() {
        let x: Vec<&str> = line.split('\t').collect();
        *counts.entry(x[0]).or_default() += 1;
        match x[0] {
            "ctor" => {
                let mut r = JavaRandom::new(7);
                pz_animals::env::character_construction_draws(&mut r, 20, 150);
                t.check("ctor", x[1], &format!("next={}", r.next(1_000_000)));
            }
            "initnull" | "init" => {
                let (ty, breed_name, seed, clock, rest, next) = if x[0] == "init" {
                    sb = sandbox(x[5]);
                    (x[1], Some(x[2]), x[3], x[6], x[7], x[8])
                } else {
                    (x[1], None, x[2], x[3], x[4], x[5])
                };
                let clock = parse_clock(clock);
                let mut rand = JavaRandom::new(seed.parse().unwrap());
                let mut world = SimWorld::default();
                let mut chr = CharacterValues::new(s.stats.clone());
                let mut animal = Animal::new(Some(ty.to_string()));
                let breed = breed_name
                    .and_then(|n| s.defs.def(ty).and_then(|d| d.breed_by_name(n)).cloned());
                let res = {
                    let mut env = AnimalEnv {
                        ctx: AnimalCtx {
                            defs: &s.defs,
                            clock: &clock,
                            sandbox: &sb,
                        },
                        rand: &mut rand,
                        world: &mut world,
                        stat_defs: s.stats.clone(),
                        patience_range: (20, 150),
                    };
                    animal.init(breed.as_ref(), &mut chr, &mut env)
                };
                let ours = match &res {
                    Ok(_) => state(&animal, &chr),
                    Err(JavaException(e)) => format!("exc={}", exc_class(e)),
                };
                let theirs = if rest.starts_with("exc=") {
                    format!("exc={}", exc_class(rest))
                } else {
                    rest.to_string()
                };
                t.check(&format!("{} {ty} {breed_name:?}", x[0]), &theirs, &ours);
                t.check(
                    &format!("{} {ty} next", x[0]),
                    next,
                    &format!("next={}", rand.next(1_000_000)),
                );
                cur = (x[0] == "init" && res.is_ok()).then_some(Current {
                    animal,
                    chr,
                    rand,
                    sandbox: sb,
                    world,
                    dead: false,
                });
            }
            "step" => {
                let Some(c) = cur.as_mut() else { continue };
                if c.dead {
                    continue;
                }
                let i: usize = x[3].parse().unwrap();
                let clock = parse_clock(x[4]);
                c.world.fresh.clear();
                let sbx = c.sandbox;
                let mut env = AnimalEnv {
                    ctx: AnimalCtx {
                        defs: &s.defs,
                        clock: &clock,
                        sandbox: &sbx,
                    },
                    rand: &mut c.rand,
                    world: &mut c.world,
                    stat_defs: s.stats.clone(),
                    patience_range: (20, 150),
                };
                let a = &mut c.animal;
                let chr = &mut c.chr;
                let mut ev = String::new();
                let mut evr = || -> Result<(), JavaException> {
                    if i == 5 {
                        chr.set_stat(AnimalStat::Hunger, 0.9);
                        chr.set_stat(AnimalStat::Thirst, 0.85);
                        ev.push_str("starve,");
                    }
                    if i == 40 {
                        chr.set_stat(AnimalStat::Hunger, 0.0);
                        chr.set_stat(AnimalStat::Thirst, 0.0);
                        ev.push_str("feed,");
                    }
                    if i == 10 && chr.is_female() {
                        a.fertilize(Male::None, true, env.ctx, env.rand)?;
                        ev.push_str("fertforce,");
                    }
                    if i == 20 {
                        a.data_set_can_have_milk(true, env.ctx);
                        ev.push_str("canmilk,");
                    }
                    if i == 45 {
                        let q = a.data.milk_qty * 0.5;
                        a.data_set_milk_quantity(q, env.ctx, env.rand);
                        ev.push_str("milk,");
                    }
                    if i == 60 {
                        ev.push_str("fertself,");
                        a.fertilize(Male::This, false, env.ctx, env.rand)?;
                    }
                    if i == 70 {
                        a.change_stress(30.0);
                        ev.push_str("stress+,");
                    }
                    if i == 71 {
                        a.change_stress(-12.0);
                        ev.push_str("stress-,");
                    }
                    if i == 90 {
                        let w = a.data.wool_qty - 1.0;
                        a.data_set_wool_quantity(w, false, chr, env.ctx);
                        ev.push_str("shear,");
                    }
                    if i == 330 {
                        a.stress_level = 85.0;
                        ev.push_str("stress85,");
                    }
                    if i == 150 {
                        ev.push_str("away,");
                        a.time_since_last_update = env.ctx.clock.calendar_millis() - 30 * 3_600_000;
                        a.update_stats_away(30, chr, &mut env)?;
                    }
                    if i == 200 {
                        a.set_health(chr, 0.4);
                        ev.push_str("health,");
                    }
                    Ok(())
                };
                if let Err(JavaException(e)) = evr() {
                    ev.push_str(&format!("evexc={},", exc_class(&e)));
                }
                let java_ev: String = x[5]
                    .split(',')
                    .filter(|p| !p.is_empty())
                    .map(|p| {
                        if p.starts_with("evexc=") {
                            format!("evexc={},", exc_class(p))
                        } else {
                            format!("{p},")
                        }
                    })
                    .collect();
                t.check(&format!("step {} {} {i} events", x[1], x[2]), &java_ev, &ev);
                let mut upd = || -> Result<(), JavaException> {
                    a.data_update(chr, &mut env)?;
                    if i < 24 {
                        a.data_update(chr, &mut env)?;
                    }
                    if !chr.is_dead() {
                        if a.pet_timer > 0.0 {
                            a.pet_timer -= env.ctx.clock.multiplier();
                            if a.pet_timer < 0.0 {
                                a.pet_timer = 0.0;
                            }
                        }
                        a.update_stress(chr, env.ctx, env.rand, false, None, false);
                    }
                    Ok(())
                };
                let r = upd();
                let ours = match &r {
                    Ok(()) => state(a, chr),
                    Err(JavaException(e)) => format!("exc={} {}", exc_class(e), state(a, chr)),
                };
                let theirs = match x[6].split_once(' ') {
                    Some((e, rest)) if e.starts_with("exc=") => {
                        format!("exc={} {rest}", exc_class(e))
                    }
                    _ => x[6].to_string(),
                };
                t.check(&format!("step {} {} {i}", x[1], x[2]), &theirs, &ours);
                t.check(
                    &format!("step {} {} {i} next", x[1], x[2]),
                    x[7],
                    &format!("next={}", c.rand.next(1_000_000)),
                );
                if r.is_err() {
                    c.dead = true;
                }
            }
            "new" => {
                let Some(c) = cur.as_mut() else { continue };
                let k: usize = x[2].parse().unwrap();
                let ours = c
                    .world
                    .fresh
                    .get(k)
                    .map(|&idx| state(&c.world.created[idx].animal, &c.world.created[idx].chr))
                    .unwrap_or_else(|| "<no new animal>".into());
                t.check(&format!("new {} {}", x[1], x[2]), x[3], &ours);
            }
            "switch" => {
                let Some(c) = cur.as_mut() else { continue };
                // The grown animal (same id, new type) replaces the old one.
                let pos = c.world.fresh.iter().copied().rfind(|&idx| {
                    let n = &c.world.created[idx];
                    n.animal.animal_id == c.animal.animal_id
                        && n.animal.animal_type != c.animal.animal_type
                });
                match pos {
                    Some(idx) => {
                        let n = c.world.created[idx].clone();
                        c.animal = n.animal;
                        c.chr = n.chr;
                    }
                    None => t.check("switch", "grown", "<none>"),
                }
                c.world.fresh.clear();
            }
            _ => {}
        }
    }
    println!("lines: {counts:?}");
    println!("{} compared, {} differing", t.lines, t.diffs);
    if t.diffs > 0 {
        std::process::exit(1);
    }
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn unhex(s: &str) -> Vec<u8> {
    (0..s.len() / 2)
        .map(|i| u8::from_str_radix(&s[2 * i..2 * i + 2], 16).unwrap())
        .collect()
}

/// A [`Animal::load`] environment for the oracle's fixed clock.
fn save_clock() -> Clock {
    Clock {
        hour: 12,
        month: 6,
        millis: 0,
        mult: 0.8,
        wad: 0.0,
    }
}

fn run_blobs(s: &Setup, out: &Path, saves: &[PathBuf]) {
    let mut lines = Vec::new();
    let clock = save_clock();
    let sb = sandbox("4,4,4,2,4,4,1,0,0");
    for save in saves {
        let dir = save.join("apop");
        let Ok(rd) = std::fs::read_dir(&dir) else {
            continue;
        };
        let mut files: Vec<PathBuf> = rd.filter_map(|e| e.ok().map(|e| e.path())).collect();
        files.sort();
        for file in files {
            let buf = std::fs::read(&file).unwrap();
            let mut r = Reader::new(&buf);
            let wv = r.i32().unwrap();
            let name = format!(
                "{}",
                file.strip_prefix(save.parent().unwrap().parent().unwrap())
                    .unwrap()
                    .display()
            );
            for _chunk in 0..1024 {
                let n = r.i16().unwrap();
                for _ in 0..n {
                    r.f64().unwrap();
                    r.i64().unwrap();
                    r.i64().unwrap();
                    for _ in 0..5 {
                        r.f32().unwrap();
                    }
                    r.string().unwrap();
                    r.f64().unwrap();
                    r.f64().unwrap();
                    let count = r.i16().unwrap();
                    for k in 0..count {
                        let _serialize = r.i8().unwrap();
                        let class = r.i8().unwrap();
                        assert_eq!(class, s.class_id, "expected IsoAnimal in {name}");
                        let start = buf.len() - r.remaining();
                        let mut rand = JavaRandom::new(1);
                        let mut world = NoWorld;
                        let mut env = AnimalEnv {
                            ctx: AnimalCtx {
                                defs: &s.defs,
                                clock: &clock,
                                sandbox: &sb,
                            },
                            rand: &mut rand,
                            world: &mut world,
                            stat_defs: s.stats.clone(),
                            patience_range: (20, 150),
                        };
                        Animal::load(&mut r, wv, &mut env)
                            .unwrap_or_else(|e| panic!("{name}: {e}"));
                        let end = buf.len() - r.remaining();
                        lines.push(format!(
                            "blob\t{name}#{k}@{start}\t{wv}\t{}",
                            hex(&buf[start..end])
                        ));
                    }
                }
                let tracks = r.i16().unwrap();
                for _ in 0..tracks {
                    r.string().unwrap();
                    r.string().unwrap();
                    r.i32().unwrap();
                    r.i32().unwrap();
                    if r.bool().unwrap() {
                        r.i32().unwrap();
                    }
                    r.i64().unwrap();
                    r.bool().unwrap();
                }
            }
        }
    }
    println!("{} animals", lines.len());
    std::fs::write(out, lines.join("\n") + "\n").unwrap();
}

fn run_save(s: &Setup, blobs: &str, oracle_save: &str) {
    let mut t = Tally::default();
    let blobs: Vec<Vec<&str>> = blobs.lines().map(|l| l.split('\t').collect()).collect();
    // The oracle's save run uses variant 3.
    let sb = sandbox("4,5,6,1,2,3,0,1,0");
    let mut identical_to_blob = 0;
    let mut changed = std::collections::BTreeMap::<&'static str, usize>::new();
    let mut n = 0;
    let mut wild = 0;
    for (line, blob) in oracle_save
        .lines()
        .filter(|l| l.starts_with("loaded\t"))
        .zip(&blobs)
    {
        n += 1;
        let x: Vec<&str> = line.split('\t').collect();
        assert_eq!(x[1], blob[1]);
        let seed: i64 = x[2].parse().unwrap();
        let wv: i32 = blob[2].parse().unwrap();
        let bytes = unhex(blob[3]);
        let clock = parse_clock(x[4]);
        let mut rand = JavaRandom::new(seed);
        let mut world = NoWorld;
        let mut env = AnimalEnv {
            ctx: AnimalCtx {
                defs: &s.defs,
                clock: &clock,
                sandbox: &sb,
            },
            rand: &mut rand,
            world: &mut world,
            stat_defs: s.stats.clone(),
            patience_range: (20, 150),
        };
        let mut r = Reader::new(&bytes);
        let loaded = Animal::load(&mut r, wv, &mut env);
        let used = bytes.len() - r.remaining();
        let NewAnimal { animal, chr } = match loaded {
            Ok(a) => a,
            Err(e) => {
                t.check(&format!("load {}", x[1]), x[3], &format!("exc={e}"));
                continue;
            }
        };
        wild += animal.wild as usize;
        t.check(&format!("used {}", x[1]), x[3], &format!("used={used}"));
        t.check(&format!("state {}", x[1]), x[5], &state(&animal, &chr));
        let next = format!("next={}", rand.next(1_000_000));
        t.check(&format!("next {}", x[1]), x[6], &next);
        // Live state -> codec record -> codec save.
        let ctx = AnimalCtx {
            defs: &s.defs,
            clock: &clock,
            sandbox: &sb,
        };
        let rec = animal.to_saved(&chr, ctx, None).expect("to_saved");
        let tables = pz_world::save::SaveTables::from_text(&format!(
            "0:IsoObject\n{}:Animal\nbloodBodyParts=0\n",
            s.class_id
        ))
        .unwrap();
        let mut tables = tables;
        tables.character_stats = s.stats.to_vec();
        let settings = pz_world::save::SaveSettings {
            calendar_millis: clock.millis,
            ..Default::default()
        };
        let items = pz_world::save::items::AllItems;
        let hooks = pz_world::save::NoSaveHooks;
        let sctx = pz_world::save::SaveCtx {
            sprites: &s.sprites,
            tables: &tables,
            settings: &settings,
            items: &items,
            hooks: &hooks,
            meta: None,
        };
        let mut w = Writer::new();
        rec.save(&mut w, &sctx, false).expect("codec save");
        let ours = format!("saved={}", hex(&w.buf));
        t.check(&format!("saved {}", x[1]), x[7], &ours);
        // Against the original bytes: the save writes the calendar time in
        // place of timeSinceLastUpdate.
        let orig = &bytes[..used];
        if orig == &w.buf[..] {
            identical_to_blob += 1;
        } else {
            // Which saved fields the original's load + save changed.
            for (name, ()) in saved_field_ranges(orig, &w.buf) {
                *changed.entry(name).or_insert(0) += 1;
            }
        }
    }
    println!("fields the original's load + save changes (animals): {changed:?}");
    println!("{n} saved animals ({wild} wild); {identical_to_blob} byte-identical to the original");
    println!("{} compared, {} differing", t.lines, t.diffs);
    if t.diffs > 0 {
        std::process::exit(1);
    }
}

/// Splits two saved `IsoAnimal`s (same layout as `IsoAnimal.save`) into
/// named fields and returns the ones that differ.
fn saved_field_ranges(a: &[u8], b: &[u8]) -> Vec<(&'static str, ())> {
    fn fields(buf: &[u8]) -> Option<Vec<(&'static str, Vec<u8>)>> {
        let mut r = Reader::new(buf);
        let mut out: Vec<(&'static str, Vec<u8>)> = Vec::new();
        let total = buf.len();
        let pos = |r: &Reader<'_>| total - r.remaining();
        macro_rules! take {
            ($name:expr, $e:expr) => {{
                let s0 = pos(&r);
                let v = $e;
                let s1 = pos(&r);
                out.push(($name, buf[s0..s1].to_vec()));
                v
            }};
        }
        take!("animalZone", {
            r.i64().ok()?;
            r.i64().ok()?
        });
        take!("x,y,z,dir", {
            r.f32().ok()?;
            r.f32().ok()?;
            r.f32().ok()?;
            r.i32().ok()?
        });
        take!(
            "stats",
            for _ in 0..24 {
                r.f32().ok()?;
            }
        );
        take!("type,breed,customName", {
            r.string().ok()?;
            r.string().ok()?;
            r.string().ok()?
        });
        take!(
            "modData",
            pz_world::save::lua_table::LuaTable::load(&mut r, 249).ok()?
        );
        take!("itemId,female,animalId", {
            r.i32().ok()?;
            r.bool().ok()?;
            r.i32().ok()?
        });
        take!("genome", {
            let n = r.i32().ok()?;
            for _ in 0..n {
                pz_inventory::item::AnimalGene::load(&mut r).ok()?;
            }
        });
        take!(
            "attachedTree",
            if r.bool().ok()? {
                r.i32().ok()?;
                r.i32().ok()?;
            }
        );
        take!("age,hoursSurvived", {
            r.i32().ok()?;
            r.f64().ok()?
        });
        take!("timeSinceLastUpdate", r.i64().ok()?);
        take!("size", r.f32().ok()?);
        // The rest is compared as one block after these fields.
        let s0 = pos(&r);
        out.push(("rest", buf[s0..].to_vec()));
        Some(out)
    }
    let (Some(fa), Some(fb)) = (fields(a), fields(b)) else {
        return vec![("unparsed", ())];
    };
    let mut v = Vec::new();
    for ((n, x), (_, y)) in fa.iter().zip(&fb) {
        if x != y {
            if *n == "rest" {
                // stress, acceptance, weight, timers, health, virtualId,
                // migrationGroup, clutch, hook, petTimer, wild, onlineID.
                let k = x.len().min(y.len());
                let tail_a = &x[..k];
                let tail_b = &y[..k];
                let d: Vec<usize> = (0..k).filter(|&i| tail_a[i] != tail_b[i]).collect();
                if x.len() != y.len() {
                    v.push(("rest (length)", ()));
                } else if d.iter().all(|&i| i >= k - 2) {
                    v.push(("onlineID", ()));
                } else {
                    v.push((
                        if d.iter().any(|&i| i >= k - 2) {
                            "rest (+onlineID)"
                        } else {
                            "rest"
                        },
                        (),
                    ));
                }
            } else {
                v.push((n, ()));
            }
        }
    }
    v
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let oracle = std::fs::read_to_string(&a[2]).expect("oracle dump");
    let s = setup(&oracle, Path::new(&a[3]));
    match a[1].as_str() {
        "sim" => run_sim(&oracle, &s),
        "blobs" => {
            let saves: Vec<PathBuf> = a[5..].iter().map(PathBuf::from).collect();
            run_blobs(&s, Path::new(&a[4]), &saves)
        }
        "save" => {
            let blobs = std::fs::read_to_string(&a[4]).unwrap();
            let os = std::fs::read_to_string(&a[5]).unwrap();
            run_save(&s, &blobs, &os)
        }
        m => panic!("unknown mode {m}"),
    }
}
