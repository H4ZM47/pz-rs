//! Replays a local oracle run of the original `IsoAnimal` through
//! `GameCharacter` and compares every traced state.
//!
//! ```text
//! animal_character_parity tick <oracle tick dump> <game dir>
//! animal_character_parity save <oracle charsave dump> <game dir> <blobs>
//! ```
//!
//! * `tick`: the oracle made `new IsoAnimal(cell, 100, 200, 0, type, breed)`
//!   for every definition and breed with a seeded `Rand` (the character's
//!   construction draws, `init(breed)`, the random direction), then ran
//!   150 `IsoAnimal.update()`s without a square and with
//!   `BaseAnimalBehavior.update()` left out, under changing clocks, sandbox
//!   options and scripted starving, feeding, petting and stress. This
//!   builds the same animals with [`GameCharacter::new_animal`] and
//!   [`GameCharacter::init_animal`] and runs [`GameCharacter::update_animal`].
//! * `save`: the oracle made `new IsoAnimal(cell)` with a seeded `Rand`,
//!   loaded each saved animal of the save copies' `apop` files (the blobs
//!   of `pz-animals`' `animal_data_parity blobs`) and saved it again
//!   (serialised). This does the same with [`GameCharacter::new_animal`],
//!   [`GameCharacter::load_animal`] and [`GameCharacter::save_animal`].
//!
//! Both compare the state, the construction draws and the next `Rand`
//! draw. The oracle and its data are local only and never committed.

use std::path::Path;

use pz_animals::env::CharacterValues;
use pz_animals::{
    Animal, AnimalCharacter, AnimalDefinitionTables, AnimalSandbox, AnimalStat, AnimalWorld,
    NewAnimal, ObjectRef,
};
use pz_characters::character::animal::AnimalHost;
use pz_characters::character::player::PlayerUpdate;
use pz_characters::character::state::{ActionGroups, StateCx, StateRegistry, StateWorld};
use pz_characters::character::{GameCharacter, SimContext, SimStatics};
use pz_characters::Characters;
use pz_scripts::registry::builtin::{BuiltinTables, CharacterStatDecl};
use pz_scripts::ScriptManager;
use pz_world::save::io::{Reader, Writer};

type Chr = GameCharacter<i32>;

/// The clock and options the oracle printed for a step.
#[derive(Clone, Copy, Default)]
struct Ctx {
    hour: i32,
    month: i32,
    millis: i64,
    mult: f32,
    world_age_hours: f64,
    sandbox: Option<AnimalSandbox>,
}

impl SimContext for Ctx {
    fn multiplier(&self) -> f32 {
        self.mult
    }
    fn time_of_day(&self) -> f32 {
        self.hour as f32
    }
    fn month(&self) -> i32 {
        self.month
    }
    fn calendar_time_in_millis(&self) -> i64 {
        self.millis
    }
    fn world_age_hours(&self) -> f64 {
        self.world_age_hours
    }
    fn animal_sandbox(&self) -> AnimalSandbox {
        self.sandbox.unwrap_or_default()
    }
}

fn parse_clock(s: &str, c: &mut Ctx) {
    for kv in s.split(' ') {
        let (k, v) = kv.split_once('=').unwrap();
        match k {
            "h" => c.hour = v.parse().unwrap(),
            "mo" => c.month = v.parse().unwrap(),
            "ms" => c.millis = v.parse().unwrap(),
            "mul" => c.mult = f32::from_bits(u32::from_str_radix(v, 16).unwrap()),
            "wad" => {
                // getWorldAgeDaysSinceBegin = hours / 24 (TimeSinceApo 1).
                let wad = f64::from_bits(u64::from_str_radix(v, 16).unwrap());
                let mut h = wad * 24.0;
                for _ in 0..4 {
                    if h / 24.0 == wad {
                        break;
                    }
                    h = f64::from_bits(if h / 24.0 < wad {
                        h.to_bits() + 1
                    } else {
                        h.to_bits() - 1
                    });
                }
                c.world_age_hours = h;
            }
            _ => {}
        }
    }
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

/// The animals the animal code hands the world (babies, grown animals).
#[derive(Default)]
struct Created {
    n: usize,
}

impl AnimalWorld for Created {
    fn add_new_animal(&mut self, _animal: NewAnimal, _into: Option<ObjectRef>) -> ObjectRef {
        self.n += 1;
        self.n as ObjectRef
    }
    fn replace_grown_animal(&mut self, _grown: NewAnimal) -> Option<ObjectRef> {
        self.n += 1;
        Some(self.n as ObjectRef)
    }
}

struct World<'a> {
    defs: &'a AnimalDefinitionTables,
    created: Created,
}

impl StateWorld<i32> for World<'_> {
    fn animal_host(&mut self, _chr: i32) -> Option<AnimalHost<'_>> {
        Some(AnimalHost {
            defs: self.defs,
            world: &mut self.created,
        })
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

/// The oracle's `state(animal)`.
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

/// `state(a) + extra(a)` of the oracle for a character.
fn chr_state(c: &Chr, chars: &Characters) -> String {
    let v = c.animal_character_values(chars).unwrap();
    let a = &c.animal_fields().unwrap().animal;
    format!(
        "{} width={} td={} collidable={} objName={} animset={} dmg={}",
        state(a, &v),
        f(c.animal_fields().unwrap().width),
        f(c.vars.turn_delta_normal),
        b(c.path.collidable),
        c.animal_object_name(),
        c.animal_anim_set_name().unwrap_or("null"),
        f(c.animal_calc_damage().unwrap_or(0.0)),
    )
}

/// The oracle's `ctorInfo`.
fn ctor_info(c: &Chr) -> String {
    let k = c.animal_fields().unwrap().construction;
    let mut s = String::from("ctor=");
    for x in k.descriptor_stats {
        s.push_str(&f(x));
        s.push(',');
    }
    format!(
        "{s}{},dir={},x={},y={}",
        k.patience,
        c.vars.dir as i32,
        f(c.vars.x),
        f(c.vars.y)
    )
}

struct Setup {
    sm: ScriptManager,
    chars: Characters,
    defs: AnimalDefinitionTables,
    decls: Vec<pz_world::save::animal::CharacterStatDecl>,
    class_id: i8,
    groups: ActionGroups,
    sprites: pz_world::IsoSpriteManager,
}

fn setup(oracle: &str, game: &Path) -> Setup {
    let fs = pz_fs::GameFs::new(game);
    let lua = pz_animals::lua::DefinitionsLua::load(&fs).expect("lua");
    let defs = AnimalDefinitionTables::load(&lua.env()).expect("definitions");
    let mut t = BuiltinTables::default();
    let mut decls = Vec::new();
    let mut class_id = 0;
    for l in oracle.lines() {
        let x: Vec<&str> = l.split('\t').collect();
        match x[0] {
            "stat" => {
                let d = CharacterStatDecl {
                    field: x[1].into(),
                    id: x[2].into(),
                    minimum: x[3].parse().unwrap(),
                    maximum: x[4].parse().unwrap(),
                    default: x[5].parse().unwrap(),
                };
                decls.push(pz_world::save::animal::CharacterStatDecl {
                    field: d.field.clone(),
                    id: d.id.clone(),
                    minimum: d.minimum,
                    maximum: d.maximum,
                    default: d.default,
                });
                t.ordered_stats.push(d.field.clone());
                t.character_stats.push(d);
            }
            "classid" => class_id = x[2].parse().unwrap(),
            _ => {}
        }
    }
    let chars = Characters {
        sim: SimStatics::from_tables(&t).expect("statics"),
        ..Default::default()
    };
    let tables = pz_world::SpriteTables::from_game_jar(&game.join("projectzomboid.jar"))
        .expect("sprite tables");
    Setup {
        sm: ScriptManager::new(),
        chars,
        defs,
        decls,
        class_id,
        groups: ActionGroups::from_media(game.join("media")),
        sprites: pz_world::IsoSpriteManager::new(tables),
    }
}

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
            if self.diffs <= 10 {
                println!("DIFF {what}");
                let jf: Vec<&str> = java.split(' ').collect();
                let rf: Vec<&str> = rust.split(' ').collect();
                let mut shown = 0;
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

fn next() -> String {
    format!("next={}", pz_core::random::next(1_000_000))
}

fn run_tick(oracle: &str, s: &mut Setup) {
    let mut t = Tally::default();
    let mut registry = StateRegistry::<i32>::with_ported();
    let mut current: Option<(Chr, Ctx)> = None;
    let mut ticks = 0;
    let mut animals = 0;
    for line in oracle.lines() {
        let x: Vec<&str> = line.split('\t').collect();
        match x[0] {
            "made" => {
                current = None;
                if x[5].starts_with("exc=") {
                    continue;
                }
                animals += 1;
                let seed: i64 = x[3].parse().unwrap();
                let mut ctx = Ctx {
                    sandbox: Some(sandbox(x[5])),
                    ..Default::default()
                };
                parse_clock(x[6], &mut ctx);
                pz_core::random::set_seed(seed);
                let mut c = Chr::new_animal(&s.sm, &mut s.chars).expect("new_animal");
                c.vars.x = 100.5;
                c.vars.y = 200.5;
                c.vars.z = 0.0;
                c.vars.last_x = 100.5;
                c.vars.last_y = 200.5;
                let breed = s
                    .defs
                    .def(x[1])
                    .and_then(|d| d.breed_by_name(x[2]))
                    .cloned();
                let mut world = World {
                    defs: &s.defs,
                    created: Created::default(),
                };
                let mut host = world.animal_host(0).unwrap();
                let r = c.init_animal(
                    x[1],
                    breed.as_ref(),
                    &mut registry,
                    &mut s.groups,
                    &ctx,
                    &mut host,
                );
                if let Err(e) = r {
                    t.check(
                        &format!("made {} {}", x[1], x[2]),
                        x[8],
                        &format!("exc={e}"),
                    );
                    continue;
                }
                AnimalCharacter::set_dir(&mut c, pz_core::random::next(8));
                t.check(&format!("ctor {} {}", x[1], x[2]), x[7], &ctor_info(&c));
                t.check(
                    &format!("made {} {}", x[1], x[2]),
                    x[8],
                    &chr_state(&c, &s.chars),
                );
                t.check(&format!("made next {} {}", x[1], x[2]), x[9], &next());
                current = Some((c, ctx));
            }
            "tick" => {
                let Some((c, ctx)) = current.as_mut() else {
                    continue;
                };
                ticks += 1;
                parse_clock(x[2], ctx);
                let hunger = s.chars.sim.character_stats.field("HUNGER").unwrap();
                let thirst = s.chars.sim.character_stats.field("THIRST").unwrap();
                match x[3] {
                    "starve" => {
                        c.stats.set(&hunger, 0.9);
                        c.stats.set(&thirst, 0.85);
                    }
                    "feed" => {
                        c.stats.set(&hunger, 0.0);
                        c.stats.set(&thirst, 0.0);
                    }
                    "pet" => c.animal_fields_mut().unwrap().animal.pet_timer = 40.0,
                    "stress" => c.animal_fields_mut().unwrap().animal.change_stress(30.0),
                    _ => {}
                }
                let mut world = World {
                    defs: &s.defs,
                    created: Created::default(),
                };
                let mut out = PlayerUpdate::default();
                let ctx_ref: &dyn SimContext = &*ctx;
                let mut cx = StateCx::new(&s.sm, &s.chars, ctx_ref, &mut world);
                let result = c.update_animal(&mut registry, &mut out, &mut cx);
                let r = cx.state_result().and(result.map_err(pz_characters::character::state::StateFailure::from));
                let st = match r {
                    Ok(()) => chr_state(c, &s.chars),
                    Err(e) => format!("exc={e} {}", chr_state(c, &s.chars)),
                };
                t.check(&format!("tick {}", x[1]), x[4], &st);
                t.check(&format!("tick next {}", x[1]), x[5], &next());
            }
            "stop" => current = None,
            _ => {}
        }
    }
    println!(
        "{animals} animals, {ticks} ticks; {} compared, {} differing",
        t.lines, t.diffs
    );
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

fn run_save(oracle: &str, blobs: &str, s: &mut Setup) {
    let mut t = Tally::default();
    let mut registry = StateRegistry::<i32>::with_ported();
    let blobs: Vec<Vec<&str>> = blobs
        .lines()
        .filter(|l| l.starts_with("blob\t"))
        .map(|l| l.split('\t').collect())
        .collect();
    let sb = sandbox("4,5,6,1,2,3,0,1,0");
    let mut n = 0;
    let mut same_bytes = 0;
    for (line, blob) in oracle
        .lines()
        .filter(|l| l.starts_with("charloaded\t"))
        .zip(&blobs)
    {
        n += 1;
        let x: Vec<&str> = line.split('\t').collect();
        assert_eq!(x[1], blob[1]);
        let seed: i64 = x[2].parse().unwrap();
        let wv: i32 = blob[2].parse().unwrap();
        let bytes = unhex(blob[3]);
        let mut ctx = Ctx {
            sandbox: Some(sb),
            ..Default::default()
        };
        parse_clock(x[4], &mut ctx);
        pz_core::random::set_seed(seed);
        let mut c = Chr::new_animal(&s.sm, &mut s.chars).expect("new_animal");
        let ctor = ctor_info(&c);
        let mut world = World {
            defs: &s.defs,
            created: Created::default(),
        };
        let mut host = world.animal_host(0).unwrap();
        let mut r = Reader::new(&bytes);
        let loaded = c.load_animal(
            &s.chars,
            &mut r,
            wv,
            &mut registry,
            &mut s.groups,
            &ctx,
            &mut host,
        );
        let used = bytes.len() - r.remaining();
        if let Err(e) = loaded {
            t.check(&format!("load {}", x[1]), x[3], &format!("exc={e}"));
            continue;
        }
        t.check(&format!("used {}", x[1]), x[3], &format!("used={used}"));
        t.check(&format!("ctor {}", x[1]), x[5], &ctor);
        t.check(&format!("state {}", x[1]), x[6], &chr_state(&c, &s.chars));
        t.check(&format!("next {}", x[1]), x[7], &next());
        let mut tables = pz_world::save::SaveTables::from_text(&format!(
            "0:IsoObject\n{}:Animal\nbloodBodyParts=0\n",
            s.class_id
        ))
        .unwrap();
        tables.character_stats = s.decls.clone();
        let settings = pz_world::save::SaveSettings {
            calendar_millis: ctx.millis,
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
        c.save_animal(&s.chars, &ctx, &s.defs, &sctx, &mut w, true, None)
            .expect("save");
        t.check(
            &format!("saved {}", x[1]),
            x[8],
            &format!("saved={}", hex(&w.buf)),
        );
        if w.buf.get(2..) == Some(&bytes[..used]) {
            same_bytes += 1;
        }
        assert!(
            c.items.worn_items.is_none(),
            "IsoAnimal.load reads no worn items"
        );
        assert!(
            !c.in_players && !c.player_instance,
            "an animal is never one of IsoPlayer.players"
        );
    }
    println!("{n} saved animals; {same_bytes} saved byte-identical to their apop record");
    println!("{} compared, {} differing", t.lines, t.diffs);
    if t.diffs > 0 {
        std::process::exit(1);
    }
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let oracle = std::fs::read_to_string(&a[2]).expect("oracle dump");
    let mut s = setup(&oracle, Path::new(&a[3]));
    match a[1].as_str() {
        "tick" => run_tick(&oracle, &mut s),
        "save" => {
            let blobs = std::fs::read_to_string(&a[4]).unwrap();
            run_save(&oracle, &blobs, &mut s)
        }
        m => panic!("unknown mode {m}"),
    }
}
