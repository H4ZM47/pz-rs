//! Replays a local oracle run of the original `BaseAnimalBehavior` and
//! animal states through `GameCharacter` and compares every traced tick.
//!
//! ```text
//! animal_behavior_parity <oracle beh dump> <game dir>
//! ```
//!
//! The oracle made `new IsoAnimal(cell, 812, 812, 0, type, breed)` for the
//! first breed of every definition with a seeded `Rand`, put it on a
//! 32x32 grid of squares at (800, 800) (some squares blocked, some
//! roofed), with an allocated player and zombie, then ran 260 ticks of
//! `getBehavior().update()` and `getStateMachine().update()` under changing
//! multipliers and calendar times, with scripted events (hunger, an
//! attacker, a leash, the player spotted at different distances and
//! speeds, a zombie closing in, alerting, holding, `goAttack`). There is
//! no path finder and no animation, so the harness stands in for both the
//! same way on each side: a path request walks straight, 0.2 a tick,
//! colliding with the blocked squares, and a walk that stops moving goes
//! back to the default state; an attack target puts the animal in
//! `AnimalAttackState`, whose `ActiveAnimFinishing` event comes on the
//! fifth tick. A second animal of the same breed is the one attacked
//! (`goAttack`, `fightAnimal`, `HitByAnimal` both ways).
//!
//! The dump also holds the pure helpers with recorded inputs:
//! `AnimalFollowWallState.decideRotation`,
//! `AnimalAlertedState.setTurnAlertedValues`, `animalShouldThump` and
//! `BaseAnimalBehavior.checkSit`.
//!
//! The oracle and its data are local only and never committed.

use std::collections::HashMap;

use pz_animals::{
    character_ref, character_ref_id, AnimalDefinitionTables, AnimalSandbox, AnimalWorld,
};
use pz_characters::character::animal::AnimalHost;
use pz_characters::character::state::{
    ActionGroups, SquareInfo, StateCx, StateRegistry, StateResult, StateWorld,
};
use pz_characters::character::states::ids;
use pz_characters::character::{CharacterKind, GameCharacter, SimContext, SimStatics};
use pz_characters::Characters;
use pz_pathfind::behavior::PathFindBehavior2;
use pz_pathfind::mover::{MoverId, PathMover};
use pz_pathfind::service::{FinderId, PathfindService};
use pz_scripts::registry::builtin::{BuiltinTables, CharacterStatDecl};
use pz_scripts::ScriptManager;

type Chr = GameCharacter<i32>;

const X0: i32 = 800;
const Y0: i32 = 800;
const W: i32 = 32;
const ANIMAL: i32 = 1;
const PLAYER: i32 = 2;
const ZOMBIE: i32 = 3;
const OTHER: i32 = 4;

fn blocked(x: i32, y: i32) -> bool {
    (x == 805 && (803..820).contains(&y)) || ((820..825).contains(&x) && y == 822)
}

fn roofed(x: i32, y: i32) -> bool {
    (808..812).contains(&x) && (808..812).contains(&y)
}

fn in_grid(x: i32, y: i32, z: i32) -> bool {
    z == 0 && (X0..X0 + W).contains(&x) && (Y0..Y0 + W).contains(&y)
}

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
    fn rain_manager_is_raining(&self) -> bool {
        false
    }
    fn rain_manager_intensity(&self) -> f32 {
        0.0
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
                let wad = f64::from_bits(u64::from_str_radix(v, 16).unwrap());
                c.world_age_hours = wad * 24.0;
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

/// The animal world of the grid: nothing but its defaults.
struct Animals;
impl AnimalWorld for Animals {}

/// `PathFindBehavior2.walkingOnTheSpot` as the oracle's animal has it.
#[derive(Default, Clone, Copy)]
struct Spot {
    x: f32,
    y: f32,
    time: f32,
}

struct Path {
    pfb: PathFindBehavior2,
    goal: &'static str,
    target: Option<i32>,
    spot: Spot,
}

struct World<'a> {
    defs: &'a AnimalDefinitionTables,
    animals: Animals,
    others: Vec<Chr>,
    taken: Vec<i32>,
    service: PathfindService,
    paths: HashMap<i32, Path>,
    /// Where the owner (borrowed during its update) stands.
    owner_pos: (f32, f32),
    mult: f32,
}

impl World<'_> {
    fn path(&mut self, id: i32) -> &mut Path {
        self.paths.entry(id).or_insert_with(|| Path {
            pfb: PathFindBehavior2::new(FinderId(id as u64)),
            goal: "None",
            target: None,
            spot: Spot::default(),
        })
    }
    fn positions(&self) -> Vec<(f32, f32)> {
        let mut v: Vec<(f32, f32)> = self.others.iter().map(|c| (c.vars.x, c.vars.y)).collect();
        v.push(self.owner_pos);
        v
    }
}

fn chr_id(chr: &dyn PathMover) -> i32 {
    chr.mover_id().0 as u32 as i32
}

impl StateWorld<i32> for World<'_> {
    fn character_mut(&mut self, id: i32) -> Option<&mut Chr> {
        self.others.iter_mut().find(|c| c.id == id)
    }
    fn take_character(&mut self, id: i32) -> Option<Chr> {
        let i = self.others.iter().position(|c| c.id == id)?;
        self.taken.push(id);
        Some(self.others.remove(i))
    }
    fn put_character(&mut self, chr: Chr) {
        self.taken.retain(|t| *t != chr.id);
        self.others.push(chr);
        self.others.sort_by_key(|c| c.id);
    }
    fn square(&self, x: i32, y: i32, z: i32) -> Option<SquareInfo> {
        in_grid(x, y, z).then(|| SquareInfo {
            x,
            y,
            z,
            ..Default::default()
        })
    }
    fn square_is_free(&self, sq: (i32, i32, i32)) -> bool {
        let occupied = self
            .positions()
            .iter()
            .any(|(x, y)| x.floor() as i32 == sq.0 && y.floor() as i32 == sq.1 && sq.2 == 0);
        !occupied && !blocked(sq.0, sq.1)
    }
    fn square_is_free_ignoring_characters(&self, sq: (i32, i32, i32)) -> bool {
        !blocked(sq.0, sq.1)
    }
    fn square_have_roof(&self, sq: (i32, i32, i32)) -> bool {
        roofed(sq.0, sq.1)
    }
    /// The grid's 4x4 chunks.
    fn chunk_loaded(&self, wx: i32, wy: i32) -> bool {
        (X0 / 8..(X0 + W) / 8).contains(&wx) && (Y0 / 8..(Y0 + W) / 8).contains(&wy)
    }
    /// The oracle's `PolygonalMap2` has no chunks: every line is blocked.
    fn line_clear_collide(
        &self,
        _x0: f32,
        _y0: f32,
        _x1: f32,
        _y1: f32,
        _z: i32,
        _ignore: Option<i32>,
        _flags: i32,
    ) -> bool {
        true
    }
    fn path_to_location(&mut self, chr: &mut dyn PathMover, x: i32, y: i32, z: i32) {
        let id = chr_id(chr);
        let mut p = self.paths.remove(&id).unwrap_or_else(|| Path {
            pfb: PathFindBehavior2::new(FinderId(id as u64)),
            goal: "None",
            target: None,
            spot: Spot::default(),
        });
        p.pfb.path_to_location(chr, &mut self.service, x, y, z);
        p.goal = "Location";
        p.target = None;
        self.paths.insert(id, p);
    }
    /// `pathToCharacter` of a target on foot and not sitting: its position.
    fn path_to_character(&mut self, chr: &mut dyn PathMover, target: MoverId) {
        let id = chr_id(chr);
        let tid = target.0 as u32 as i32;
        let (tx, ty, tz) = if tid == ANIMAL {
            (self.owner_pos.0, self.owner_pos.1, 0.0)
        } else {
            let t = self.others.iter().find(|c| c.id == tid).unwrap();
            (t.vars.x, t.vars.y, t.vars.z)
        };
        let mut p = self.paths.remove(&id).unwrap_or_else(|| Path {
            pfb: PathFindBehavior2::new(FinderId(id as u64)),
            goal: "None",
            target: None,
            spot: Spot::default(),
        });
        p.pfb.path_to_location_f(chr, &mut self.service, tx, ty, tz);
        p.goal = "Character";
        p.target = Some(tid);
        self.paths.insert(id, p);
    }
    fn path_reset(&mut self, chr: &mut dyn PathMover) {
        let id = chr_id(chr);
        let (x, y) = (chr.x(), chr.y());
        let p = self.path(id);
        let mut pfb = std::mem::replace(&mut p.pfb, PathFindBehavior2::new(FinderId(0)));
        pfb.reset(chr);
        let p = self.path(id);
        p.pfb = pfb;
        p.spot = Spot { x, y, time: 0.0 };
    }
    fn path_behavior(&self, id: i32) -> Option<&PathFindBehavior2> {
        self.paths.get(&id).map(|p| &p.pfb)
    }
    fn walking_on_the_spot_check(&mut self, id: i32, x: f32, y: f32) -> bool {
        let m = self.mult;
        let s = &mut self.path(id).spot;
        let (dx, dy) = ((x - s.x) as f64, (y - s.y) as f64);
        if ((dx * dx + dy * dy) as f32) < 0.010_000_001 {
            s.time += m;
        } else {
            *s = Spot { x, y, time: 0.0 };
        }
        s.time > 30.0
    }
    fn animal_host(&mut self, _chr: i32) -> Option<AnimalHost<'_>> {
        Some(AnimalHost {
            defs: self.defs,
            world: &mut self.animals,
        })
    }
}

fn f(x: f32) -> String {
    format!("{:08x}", x.to_bits())
}

thread_local! {
    static OTHER_ANIMAL_ID: std::cell::Cell<i32> = const { std::cell::Cell::new(-1) };
}

fn id_name(c: Option<i32>, animal_id: i32) -> String {
    match c {
        None => "null".into(),
        Some(PLAYER) => "P".into(),
        Some(ZOMBIE) => "Z".into(),
        Some(ANIMAL) => format!("A{animal_id}"),
        Some(OTHER) => format!("A{}", OTHER_ANIMAL_ID.with(|c| c.get())),
        Some(x) => format!("?{x}"),
    }
}

fn dir_name(d: pz_world::square::IsoDirections) -> String {
    format!("{d:?}")
}

/// The oracle's `trace(a)`.
fn trace(c: &Chr, w: &World<'_>) -> String {
    let f_ = c.animal_fields().unwrap();
    let a = &f_.animal;
    let b = &a.behavior;
    let aid = a.animal_id;
    let mut s = String::new();
    let mut p = |k: &str, v: String| {
        if !s.is_empty() {
            s.push(' ');
        }
        s.push_str(k);
        s.push('=');
        s.push_str(&v);
    };
    p("st", c.current_state_name().unwrap_or("null").to_string());
    p("x", f(c.vars.x));
    p("y", f(c.vars.y));
    p("dir", dir_name(c.vars.dir));
    p(
        "fwd",
        format!("{},{}", f(c.vars.forward_x), f(c.vars.forward_y)),
    );
    p("sedt", f(c.vars.state_event_delay_timer));
    p("moving", c.vars.moving.to_string());
    for v in [
        "idleAction",
        "sittingAnim",
        "eatingAnim",
        "animalRunning",
        "bMoving",
        "bPathfind",
    ] {
        p(v, c.variable_string(v));
    }
    for v in ["animalSpeed", "AlertX"] {
        p(v, f(c.variable_float(v, -1.0)));
    }
    let path = w.paths.get(&ANIMAL);
    let (tx, ty, tz) = path.map_or((0.0, 0.0, 0.0), |p| {
        (p.pfb.target_x(), p.pfb.target_y(), p.pfb.target_z())
    });
    p("tgt", format!("{},{},{}", f(tx), f(ty), f(tz)));
    p("goal", path.map_or("None", |p| p.goal).to_string());
    p("tchar", id_name(path.and_then(|p| p.target), aid));
    p("wanderMulMod", f(b.wander_mul_mod));
    p("blockMovement", b.block_movement.to_string());
    p("goToMomTimer", f(b.go_to_mom_timer));
    p("sitInTime", b.sit_in_time.to_string());
    p("sitOutTime", b.sit_out_time.to_string());
    p("blockedFor", f(b.blocked_for));
    p("attackAnimalTimer", f(b.attack_animal_timer));
    p("followChrTimer", f(b.follow_chr_timer));
    p("lastAlerted", f(b.last_alerted));
    p("idleAnimTimer", f(b.idle_anim_timer));
    p("behaviorCheckTimer", f(b.behavior_check_timer));
    p(
        "behaviorAction",
        b.behavior_action
            .map_or("null".into(), |a| a.name().to_string()),
    );
    p("isDoingBehavior", b.is_doing_behavior.to_string());
    p("behaviorFailsafe", f(b.behavior_failsafe));
    p("hutchPathTimer", f(b.hutch_path_timer));
    p(
        "enterHutchTimerAfterDestroy",
        f(b.enter_hutch_timer_after_destroy),
    );
    p("forcedOutsideHutch", b.forced_outside_hutch.to_string());
    p("timerFleeAgain", f(b.timer_flee_again));
    p("wildAndHurt", b.wild_and_hurt.to_string());
    p("wildDropDeadTimer", f(b.wild_drop_dead_timer));
    let bobj = b.behavior_object.and_then(|o| match o {
        pz_animals::BehaviorObject::Character(r) | pz_animals::BehaviorObject::Animal(r) => {
            character_ref_id(r)
        }
        _ => Some(-1),
    });
    p("bobj", id_name(bobj, aid));
    p("stress", f(a.stress_level));
    p(
        "acc",
        a.player_acceptance_ordered()
            .iter()
            .map(|(k, v)| format!("{k}:{},", f(*v)))
            .collect(),
    );
    p("alerted", a.alerted.to_string());
    p("spotted", id_name(f_.spotted_chr, aid));
    p("alertedChr", id_name(f_.alerted_chr, aid));
    p("fight", id_name(f_.fighting_opponent, aid));
    p("atk", id_name(f_.atk_target, aid));
    p("attackedBy", id_name(c.vars.attacked_by, aid));
    p("attackedTimer", a.attacked_timer.to_string());
    p("td", f(c.vars.turn_delta_normal));
    p("hunger", f(c.stats.get(&f_.hunger)));
    p("thirst", f(c.stats.get(&f_.thirst)));
    p("health", f(c.health));
    p("wild", a.wild.to_string());
    p("sfw", a.should_follow_wall.to_string());
    p("fw", a.following_wall.to_string());
    p(
        "attached",
        id_name(a.data.attached_player.and_then(character_ref_id), aid),
    );
    p("hitReaction", c.vars.hit_reaction.clone());
    if let Some(o) = w.others.iter().find(|o| o.id == OTHER) {
        let of = o.animal_fields().unwrap();
        p("B.health", f(o.health));
        p("B.atk", id_name(of.atk_target, aid));
        p("B.fight", id_name(of.fighting_opponent, aid));
        p("B.block", of.animal.behavior.block_movement.to_string());
        p("B.attackTimer", f(of.animal.behavior.attack_animal_timer));
        let path = w.paths.get(&OTHER);
        let (tx, ty) = path.map_or((0.0, 0.0), |p| (p.pfb.target_x(), p.pfb.target_y()));
        p("B.tgt", format!("{},{}", f(tx), f(ty)));
        p("B.hitReaction", o.vars.hit_reaction.clone());
        p("B.moving", o.vars.moving.to_string());
    }
    s
}

/// `place(c, x, y)`.
fn place(c: &mut Chr, x: f32, y: f32) {
    c.vars.x = x;
    c.vars.y = y;
    c.vars.z = 0.0;
    c.vars.next_x = x;
    c.vars.next_y = y;
    c.vars.last_x = x;
    c.vars.last_y = y;
    let (ix, iy) = (x.floor() as i32, y.floor() as i32);
    c.path.current_square = in_grid(ix, iy, 0).then_some((ix, iy, 0));
}

struct Setup {
    sm: ScriptManager,
    chars: Characters,
    defs: AnimalDefinitionTables,
    groups: ActionGroups,
}

fn setup(oracle: &str, game: &std::path::Path) -> Setup {
    let fs = pz_fs::GameFs::new(game);
    let lua = pz_animals::lua::DefinitionsLua::load(&fs).expect("lua");
    let defs = AnimalDefinitionTables::load(&lua.env()).expect("definitions");
    let mut t = BuiltinTables::default();
    for l in oracle.lines() {
        let x: Vec<&str> = l.split('\t').collect();
        if x[0] == "stat" {
            let d = CharacterStatDecl {
                field: x[1].into(),
                id: x[2].into(),
                minimum: x[3].parse().unwrap(),
                maximum: x[4].parse().unwrap(),
                default: x[5].parse().unwrap(),
            };
            t.ordered_stats.push(d.field.clone());
            t.character_stats.push(d);
        }
    }
    let chars = Characters {
        sim: SimStatics::from_tables(&t).expect("statics"),
        ..Default::default()
    };
    Setup {
        sm: ScriptManager::new(),
        chars,
        defs,
        groups: ActionGroups::from_media(game.join("media")),
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
            if self.diffs <= 12 {
                println!("DIFF {what}");
                let jf: Vec<&str> = java.split(' ').collect();
                let rf: Vec<&str> = rust.split(' ').collect();
                let mut shown = 0;
                for i in 0..jf.len().max(rf.len()) {
                    let (j, r) = (
                        jf.get(i).copied().unwrap_or("-"),
                        rf.get(i).copied().unwrap_or("-"),
                    );
                    if j != r && shown < 8 {
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

/// The scripted events of a tick.
fn events(
    ev: &str,
    c: &mut Chr,
    w: &mut World<'_>,
    s: &Setup,
    ctx: &Ctx,
    registry: &mut StateRegistry<i32>,
) -> StateResult<()> {
    let hunger = s.chars.sim.character_stats.field("HUNGER").unwrap();
    let thirst = s.chars.sim.character_stats.field("THIRST").unwrap();
    for e in ev.split(',').filter(|e| !e.is_empty()) {
        let cx_world =
            |c: &mut Chr, w: &mut World<'_>, f: &mut dyn FnMut(&mut Chr, &mut StateCx<'_, i32>)| -> StateResult<()> {
                w.owner_pos = (c.vars.x, c.vars.y);
                w.mult = ctx.mult;
                let mut cx = StateCx::new(&s.sm, &s.chars, ctx, w);
                f(c, &mut cx);
                cx.state_result()
            };
        match e {
            "sedt0" => c.vars.state_event_delay_timer = 0.0,
            "hunger35" => {
                c.stats.set(&hunger, 0.35);
            }
            "fed" => {
                c.stats.set(&hunger, 0.1);
                c.stats.set(&thirst, 0.1);
            }
            "pmove" => w.character_mut(PLAYER).unwrap().vars.is_player_moving = true,
            "pstop" => w.character_mut(PLAYER).unwrap().vars.is_player_moving = false,
            "attacked" => {
                c.vars.attacked_by = Some(PLAYER);
                c.animal_fields_mut().unwrap().animal.attacked_timer = ctx.millis;
            }
            "unattacked" => c.vars.attacked_by = None,
            "leash" => {
                c.animal_fields_mut().unwrap().animal.data.attached_player =
                    Some(character_ref(PLAYER))
            }
            "unleash" => c.animal_fields_mut().unwrap().animal.data.attached_player = None,
            "stress85" => c.animal_fields_mut().unwrap().animal.stress_level = 85.0,
            "alert" => {
                let f_ = c.animal_fields_mut().unwrap();
                f_.animal.alerted = true;
                f_.alerted_chr = Some(ZOMBIE);
                place(w.character_mut(ZOMBIE).unwrap(), 806.5, 812.5);
            }
            "toAlerted" => {
                let st = registry.get_or_opaque(ids::ANIMAL_ALERTED.0);
                cx_world(c, w, &mut |c, cx| c.change_state(Some(st), cx))?;
            }
            "zgone" => place(w.character_mut(ZOMBIE).unwrap(), 790.5, 790.5),
            "held" => c.animal_fields_mut().unwrap().animal.held_by = Some((1, 0)),
            "unheld" => c.animal_fields_mut().unwrap().animal.held_by = None,
            "thirsty" => {
                c.stats.set(&thirst, 0.95);
            }
            "goAttackP" => cx_world(c, w, &mut |c, cx| c.behavior_go_attack(PLAYER, cx))?,
            "calm" => {
                let f_ = c.animal_fields_mut().unwrap();
                f_.atk_target = None;
                f_.fighting_opponent = None;
                f_.animal.behavior.block_movement = false;
            }
            "bnear" => {
                place(w.character_mut(OTHER).unwrap(), 816.5, 818.5);
                let f_ = c.animal_fields_mut().unwrap();
                f_.spotted_chr = None;
                f_.animal.alerted = false;
                f_.alerted_chr = None;
                cx_world(c, w, &mut |c, cx| c.animal_stop_all_movement_now(cx))?;
                c.vars.state_event_delay_timer = 5000.0;
            }
            "goAttackB" => {
                cx_world(c, w, &mut |c, cx| c.behavior_go_attack(OTHER, cx))?;
                c.animal_fields_mut().unwrap().fighting_opponent = Some(OTHER);
            }
            "calm2" => {
                let f_ = c.animal_fields_mut().unwrap();
                f_.atk_target = None;
                f_.fighting_opponent = None;
                f_.animal.behavior.block_movement = false;
                let o = w.character_mut(OTHER).unwrap().animal_fields_mut().unwrap();
                o.atk_target = None;
                o.fighting_opponent = None;
                o.animal.behavior.block_movement = false;
            }
            e if e.starts_with("spotP") => {
                let d: f32 = e[5..].parse().unwrap();
                cx_world(c, w, &mut |c, cx| c.behavior_spotted(PLAYER, d, cx))?;
            }
            e if e.starts_with("spotZ") => {
                let d: f32 = e[5..].parse().unwrap();
                place(w.character_mut(ZOMBIE).unwrap(), 812.5 - d, 812.5);
                cx_world(c, w, &mut |c, cx| c.behavior_spotted(ZOMBIE, d, cx))?;
            }
            other => panic!("unknown event {other}"),
        }
    }
    Ok(())
}

/// The oracle's `afterTick`.
fn after_tick(
    c: &mut Chr,
    w: &mut World<'_>,
    s: &Setup,
    ctx: &Ctx,
    attack_ticks: &mut i32,
    registry: &mut StateRegistry<i32>,
) -> StateResult<()> {
    w.owner_pos = (c.vars.x, c.vars.y);
    if c.animal_fields().unwrap().atk_target.is_some()
        && c.current_state_id() != Some(ids::ANIMAL_ATTACK)
    {
        *attack_ticks = 0;
        let st = registry.get_or_opaque(ids::ANIMAL_ATTACK.0);
        let mut cx = StateCx::new(&s.sm, &s.chars, ctx, w);
        c.change_state(Some(st), &mut cx);
        cx.state_result()?;
    }
    if c.variable_boolean("bPathfind") {
        c.set_var_bool("bPathfind", false);
        c.vars.moving = true;
    }
    w.owner_pos = (c.vars.x, c.vars.y);
    let st = c.current_state_id();
    if st == Some(ids::ANIMAL_WALK) {
        let p = w.paths.get(&ANIMAL).unwrap();
        let (tx, ty) = (p.pfb.target_x(), p.pfb.target_y());
        let (x, y) = (c.vars.x, c.vars.y);
        let (mut dx, mut dy) = (tx - x, ty - y);
        let len = ((dx * dx + dy * dy) as f64).sqrt() as f32;
        if len > 0.2 {
            dx = dx / len * 0.2;
            dy = dy / len * 0.2;
        }
        let (nx, ny) = (x + dx, y + dy);
        let (ix, iy) = (nx.floor() as i32, ny.floor() as i32);
        if !in_grid(ix, iy, 0) || blocked(ix, iy) {
            c.vars.collided_this_frame = true;
        } else {
            c.vars.collided_this_frame = false;
            place(c, nx, ny);
        }
        if !c.variable_boolean("bMoving") {
            w.owner_pos = (c.vars.x, c.vars.y);
            let mut cx = StateCx::new(&s.sm, &s.chars, ctx, w);
            let d = c.default_state();
            c.change_state(d, &mut cx);
            cx.state_result()?;
        }
    } else if st == Some(ids::ANIMAL_ATTACK) {
        if c.animal_fields().unwrap().atk_target.is_some() {
            *attack_ticks += 1;
            if *attack_ticks == 5 {
                let ev = pz_anim::AnimEventRecord {
                    state_name: None,
                    layer_depth: -1,
                    track: None,
                    event: std::rc::Rc::new(pz_anim::AnimEvent::named(
                        "ActiveAnimFinishing",
                        Default::default(),
                        0.0,
                    )),
                };
                let state = c.current_state().unwrap();
                let mut cx = StateCx::new(&s.sm, &s.chars, ctx, w);
                state.anim_event(c, &mut cx, &ev);
                cx.state_result()?;
            }
        }
        if c.animal_fields().unwrap().atk_target.is_none() {
            let mut cx = StateCx::new(&s.sm, &s.chars, ctx, w);
            let d = c.default_state();
            c.change_state(d, &mut cx);
            cx.state_result()?;
        }
    } else if st == Some(ids::ANIMAL_ALERTED) {
        let f_ = c.animal_fields().unwrap();
        if !f_.animal.alerted && f_.atk_target.is_none() {
            let mut cx = StateCx::new(&s.sm, &s.chars, ctx, w);
            let d = c.default_state();
            c.change_state(d, &mut cx);
            cx.state_result()?;
        }
    } else {
        c.vars.collided_this_frame = false;
    }
    Ok(())
}

fn main() -> StateResult<()> {
    let a: Vec<String> = std::env::args().collect();
    let oracle = std::fs::read_to_string(&a[1]).expect("oracle dump");
    let mut s = setup(&oracle, std::path::Path::new(&a[2]));
    let mut t = Tally::default();
    let mut registry = StateRegistry::<i32>::with_ported();
    let mut cur: Option<(Chr, Ctx, World<'_>)> = None;
    let defs = s.defs.clone();
    let (mut animals, mut ticks) = (0, 0);
    let mut attack_ticks = 0;
    let mut helpers = 0;
    for line in oracle.lines() {
        let x: Vec<&str> = line.split('\t').collect();
        match x[0] {
            "made" => {
                cur = None;
                if x[3].starts_with("exc=") || x.len() < 9 {
                    continue;
                }
                animals += 1;
                let seed: i64 = x[3].parse().unwrap();
                let mut ctx = Ctx {
                    sandbox: Some(sandbox(x[5])),
                    ..Default::default()
                };
                parse_clock(x[6], &mut ctx);
                let mut world = World {
                    defs: &defs,
                    animals: Animals,
                    others: Vec::new(),
                    taken: Vec::new(),
                    service: PathfindService::new(0, 0, 1, 1, false),
                    paths: HashMap::new(),
                    owner_pos: (0.0, 0.0),
                    mult: ctx.mult,
                };
                let mut p = Chr::new(&s.sm, CharacterKind::Player);
                p.id = PLAYER;
                p.health = 1.0;
                place(&mut p, 818.5, 812.5);
                let mut z = Chr::new(&s.sm, CharacterKind::Zombie);
                z.id = ZOMBIE;
                z.health = 1.0;
                place(&mut z, 790.5, 790.5);
                world.others = vec![p, z];
                let breed = defs.def(x[1]).and_then(|d| d.breed_by_name(x[2])).cloned();
                pz_core::random::set_seed(seed + 5000);
                {
                    let mut b = Chr::new_animal(&s.sm, &mut s.chars)?;
                    b.id = OTHER;
                    place(&mut b, 790.5, 790.5);
                    b.path.current_square = None;
                    {
                        let mut host = world.animal_host(OTHER).unwrap();
                        b.init_animal(
                            x[1],
                            breed.as_ref(),
                            &mut registry,
                            &mut s.groups,
                            &ctx,
                            &mut host,
                        )?;
                    }
                    {
                        pz_animals::AnimalCharacter::set_dir(&mut b, pz_core::random::next(8));
                        OTHER_ANIMAL_ID
                            .with(|c| c.set(b.animal_fields().unwrap().animal.animal_id));
                        world.others.push(b);
                    }
                }
                pz_core::random::set_seed(seed);
                let mut c = Chr::new_animal(&s.sm, &mut s.chars).expect("new_animal");
                c.id = ANIMAL;
                place(&mut c, 812.5, 812.5);
                c.path.current_square = None;
                {
                    let mut host = world.animal_host(ANIMAL).unwrap();
                    if let Err(e) = c.init_animal(
                        x[1],
                        breed.as_ref(),
                        &mut registry,
                        &mut s.groups,
                        &ctx,
                        &mut host,
                    ) {
                        t.check(&format!("made {}", x[1]), x[7], &format!("exc={e}"));
                        continue;
                    }
                }
                pz_animals::AnimalCharacter::set_dir(&mut c, pz_core::random::next(8));
                place(&mut c, 812.5, 812.5);
                c.animal_fields_mut().unwrap().exists_in_world = true;
                world.owner_pos = (c.vars.x, c.vars.y);
                attack_ticks = 0;
                if let Some(mut b) = world.take_character(OTHER) {
                    place(&mut b, 790.5, 791.5);
                    b.animal_fields_mut().unwrap().exists_in_world = true;
                    let result = {
                        let mut cx = StateCx::new(&s.sm, &s.chars, &ctx, &mut world);
                        b.set_default_state(&mut cx);
                        cx.state_result()
                    };
                    world.put_character(b);
                    result?;
                }
                {
                    let mut cx = StateCx::new(&s.sm, &s.chars, &ctx, &mut world);
                    c.set_default_state(&mut cx);
                    cx.state_result()?;
                }
                t.check(&format!("made {}", x[1]), x[7], &trace(&c, &world));
                t.check(&format!("made next {}", x[1]), x[8], &next());
                cur = Some((c, ctx, world));
            }
            "tick" => {
                let Some((c, ctx, w)) = cur.as_mut() else {
                    continue;
                };
                ticks += 1;
                let i: i32 = x[2].parse().unwrap();
                parse_clock(x[3], ctx);
                // P runs from tick 140 of the spotting.
                if (130..150).contains(&i) {
                    w.character_mut(PLAYER).unwrap().running = i >= 140;
                }
                let ctx_v = *ctx;
                events(x[4], c, w, &s, &ctx_v, &mut registry)?;
                w.owner_pos = (c.vars.x, c.vars.y);
                w.mult = ctx_v.mult;
                {
                    let mut cx = StateCx::new(&s.sm, &s.chars, &ctx_v, w);
                    c.behavior_update(&mut cx);
                    cx.state_result()?;
                    c.state_machine_update(&mut cx);
                    cx.state_result()?;
                }
                after_tick(c, w, &s, &ctx_v, &mut attack_ticks, &mut registry)?;
                let rt = trace(c, w);
                if std::env::var("DUMP").is_ok_and(|d| d == format!("{}:{i}", x[1])) {
                    println!("JAVA {}\nRUST {}", x[5], rt);
                }
                t.check(&format!("tick {} {i}", x[1]), x[5], &rt);
                t.check(&format!("tick next {} {i}", x[1]), x[6], &next());
            }
            "hmade" => {
                let seed: i64 = x[3].parse().unwrap();
                let ctx = Ctx {
                    mult: 1.0,
                    sandbox: Some(sandbox("1,2,3,4,5,6,1,1,0")),
                    ..Default::default()
                };
                let mut world = World {
                    defs: &defs,
                    animals: Animals,
                    others: Vec::new(),
                    taken: Vec::new(),
                    service: PathfindService::new(0, 0, 1, 1, false),
                    paths: HashMap::new(),
                    owner_pos: (0.0, 0.0),
                    mult: 1.0,
                };
                let mut p = Chr::new(&s.sm, CharacterKind::Player);
                p.id = PLAYER;
                p.health = 1.0;
                let mut z = Chr::new(&s.sm, CharacterKind::Zombie);
                z.id = ZOMBIE;
                z.health = 1.0;
                world.others = vec![p, z];
                pz_core::random::set_seed(seed);
                let mut c = Chr::new_animal(&s.sm, &mut s.chars).expect("new_animal");
                c.id = ANIMAL;
                let breed = defs.def(x[1]).and_then(|d| d.breed_by_name(x[2])).cloned();
                {
                    let mut host = world.animal_host(ANIMAL).unwrap();
                    c.init_animal(
                        x[1],
                        breed.as_ref(),
                        &mut registry,
                        &mut s.groups,
                        &ctx,
                        &mut host,
                    )
                    .expect("init");
                }
                pz_animals::AnimalCharacter::set_dir(&mut c, pz_core::random::next(8));
                place(&mut c, 812.5, 812.5);
                c.animal_fields_mut().unwrap().exists_in_world = true;
                helpers += 1;
                t.check(&format!("hmade {}", x[1]), x[4], &next());
                cur = Some((c, ctx, world));
            }
            "rot" => {
                let Some((c, ctx, w)) = cur.as_mut() else {
                    continue;
                };
                let dir = |n: &str| pz_world::square::IsoDirections::from_name(n).unwrap();
                c.set_forward_iso_direction(dir(x[2]));
                let pre = match x[4] {
                    "0" => None,
                    "1" => Some(true),
                    _ => Some(false),
                };
                pz_characters::character::states::animal::CW.set(&mut c.state.params, pre);
                let cx = StateCx::new(&s.sm, &s.chars, &*ctx, w);
                let r = pz_characters::character::states::animal::AnimalFollowWallState::decide_rotation(c, dir(x[3]), &cx);
                cx.state_result()?;
                t.check(
                    &format!("rot {} {} {} {}", x[1], x[2], x[3], x[4]),
                    &format!("{}\t{}", x[5], x[6]),
                    &format!("{r}\t{}", next()),
                );
            }
            "alertx" => {
                let Some((c, ctx, w)) = cur.as_mut() else {
                    continue;
                };
                let dir = pz_world::square::IsoDirections::from_name(x[2]).unwrap();
                let zx = f32::from_bits(u32::from_str_radix(x[3], 16).unwrap());
                let zy = f32::from_bits(u32::from_str_radix(x[4], 16).unwrap());
                c.set_forward_iso_direction(dir);
                let cx = StateCx::new(&s.sm, &s.chars, &*ctx, w);
                pz_characters::character::states::animal::AnimalAlertedState::set_turn_alerted_values(c, (zx, zy), &cx);
                cx.state_result()?;
                t.check(
                    &format!("alertx {} {} {}", x[1], x[2], x[3]),
                    &format!("{}\t{}", x[5], x[6]),
                    &format!(
                        "{}\t{}",
                        f(c.variable_float("AlertX", -1.0)),
                        f(c.variable_float("AlertY", -1.0))
                    ),
                );
            }
            "thump" => {
                let Some((c, ctx, w)) = cur.as_mut() else {
                    continue;
                };
                let k: i32 = x[2].parse().unwrap();
                let hunger = s.chars.sim.character_stats.field("HUNGER").unwrap();
                let thirst = s.chars.sim.character_stats.field("THIRST").unwrap();
                c.stats.set(
                    &hunger,
                    f32::from_bits(u32::from_str_radix(x[3], 16).unwrap()),
                );
                c.stats.set(
                    &thirst,
                    f32::from_bits(u32::from_str_radix(x[4], 16).unwrap()),
                );
                c.animal_fields_mut().unwrap().animal.stress_level =
                    if k >= 12 { 85.0 } else { 10.0 };
                if k == 6 {
                    c.animal_fields_mut().unwrap().animal.thump_delay = 0.0;
                }
                c.vars.attacked_by = (k == 20).then_some(PLAYER);
                ctx.mult = f32::from_bits(u32::from_str_radix(x[7], 16).unwrap());
                let cx = StateCx::new(&s.sm, &s.chars, &*ctx, w);
                let r = c.animal_should_thump(&cx);
                cx.state_result()?;
                t.check(
                    &format!("thump {} {k}", x[1]),
                    &format!("{}\t{}", x[5], x[6]),
                    &format!("{r}\t{}", f(c.animal_fields().unwrap().animal.thump_delay)),
                );
            }
            "sit" => {
                let Some((c, ctx, w)) = cur.as_mut() else {
                    continue;
                };
                let k: i32 = x[2].parse().unwrap();
                c.vars.attacked_by = None;
                ctx.millis = x[3].parse().unwrap();
                ctx.mult = 0.8;
                let hunger = s.chars.sim.character_stats.field("HUNGER").unwrap();
                c.stats.set(
                    &hunger,
                    f32::from_bits(u32::from_str_radix(x[4], 16).unwrap()),
                );
                c.animal_fields_mut().unwrap().animal.data.attached_player =
                    (k == 17).then(|| character_ref(PLAYER));
                let p = if k % 5 == 2 { 809.5 } else { 812.5 };
                place(c, p, p);
                let mut cx = StateCx::new(&s.sm, &s.chars, &*ctx, w);
                c.behavior_check_sit(&mut cx);
                cx.state_result()?;
                let b = &c.animal_fields().unwrap().animal.behavior;
                let r = format!(
                    "{}\t{}\t{}\t{}",
                    c.variable_string("idleAction"),
                    b.sit_in_time,
                    b.sit_out_time,
                    next()
                );
                t.check(
                    &format!("sit {} {k}", x[1]),
                    &format!("{}\t{}\t{}\t{}", x[5], x[6], x[7], x[8]),
                    &r,
                );
            }
            _ => {}
        }
    }
    println!("{helpers} animals with helper checks");
    println!(
        "{animals} animals, {ticks} ticks; {} compared, {} differing",
        t.lines, t.diffs
    );
    if t.diffs > 0 {
        std::process::exit(1);
    }
    Ok(())
}
