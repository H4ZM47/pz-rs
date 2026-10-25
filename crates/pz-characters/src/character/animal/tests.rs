//! The animal branches of the character, on the invented fixture.

use std::cell::Cell;

use pz_animals::env::character_construction_draws;
use pz_animals::{AnimalCharacter, AnimalDefinitionTables, AnimalStat, AnimalWorld};
use pz_pathfind::mover::PathMover;

use super::*;
use crate::character::player::{NoInput, PlayerUpdate};
use crate::character::state::{ActionGroups, StateCx, StateRegistry, StateWorld};
use crate::character::states::ids;
use crate::character::tests::{fixture, Recorder};
use crate::character::CharacterKind;

type Chr = GameCharacter<i32>;

fn animal(seed: i64) -> (ScriptManager, Characters, Chr) {
    let (sm, mut chars) = fixture();
    pz_core::random::set_seed(seed);
    let c = Chr::new_animal(&sm, &mut chars).unwrap();
    (sm, chars, c)
}

fn defs() -> AnimalDefValues {
    AnimalDefValues {
        animset: Some("cow".into()),
        turn_delta: 0.6,
        animal_size: 0.8,
        collision_size: 0.3,
        collidable: true,
        can_thump: true,
        can_climb_stairs: false,
        can_climb_fences: true,
        corpse_size: 1.5,
        corpse_length: 2.0,
        base_dmg: 4.0,
        attack_back: false,
        wild_flee_time_until_dead_timer: 100.0,
        adef: Default::default(),
    }
}

#[derive(Default)]
struct Calls {
    instance: Cell<i32>,
    removed: Cell<i32>,
    recent: Cell<i32>,
}

struct Rec<'a>(&'a Calls);

impl AnimalWorld for Rec<'_> {
    fn instance_manager_update(&mut self) {
        self.0.instance.set(self.0.instance.get() + 1);
    }
    fn removed_from_world(&mut self) {
        self.0.removed.set(self.0.removed.get() + 1);
    }
    fn add_to_recently_removed(&mut self) {
        self.0.recent.set(self.0.recent.get() + 1);
    }
}

struct World<'a> {
    defs: AnimalDefinitionTables,
    rec: Rec<'a>,
    others: Vec<Chr>,
}

impl StateWorld<i32> for World<'_> {
    fn animal_host(&mut self, _chr: i32) -> Option<AnimalHost<'_>> {
        Some(AnimalHost {
            defs: &self.defs,
            world: &mut self.rec,
        })
    }
    fn characters_near(&self, _x: f32, _y: f32, _z: f32, _r: f32) -> Vec<i32> {
        self.others.iter().map(|c| c.id).collect()
    }
    fn character_mut(&mut self, id: i32) -> Option<&mut Chr> {
        self.others.iter_mut().find(|c| c.id == id)
    }
}

fn world(calls: &Calls) -> World<'_> {
    World {
        defs: AnimalDefinitionTables::default(),
        rec: Rec(calls),
        others: Vec::new(),
    }
}

#[test]
fn new_animal_makes_the_construction_draws_and_stays_out_of_player_slots() {
    let (_sm, _chars, c) = animal(77);
    let ctx = Recorder::default();
    pz_core::random::set_seed(77);
    let want =
        character_construction_draws(&mut CtxRand(&ctx), Chr::PATIENCE_MIN, Chr::PATIENCE_MAX);
    let f = c.animal_fields().unwrap();
    assert_eq!(f.construction, want);
    assert_eq!(c.patience, want.patience);
    // The next draw follows the seven construction draws.
    let after = pz_core::random::next(1000);
    pz_core::random::set_seed(77);
    for _ in 0..6 {
        pz_core::random::next(2);
    }
    pz_core::random::next_range(Chr::PATIENCE_MIN, Chr::PATIENCE_MAX);
    assert_eq!(pz_core::random::next(1000), after);
    assert_eq!(c.kind, CharacterKind::Animal);
    assert!(!c.in_players && !c.player_instance);
    assert!(c.nutrition.is_none() && c.fitness.is_none() && c.clothing_wetness.is_none());
    assert!(c.items.worn_items.is_none());
    assert!(c.path.collidable);
    assert_eq!(
        (c.vars.x, c.vars.y, c.vars.dir),
        (0.5, 0.5, IsoDirections::W)
    );
}

#[test]
fn animal_character_reads_and_writes_the_character() {
    let (_sm, chars, mut c) = animal(1);
    c.set_stat(AnimalStat::Hunger, 2.0);
    let hunger = chars.sim.character_stats.field("HUNGER").unwrap();
    assert_eq!(c.stats.get(&hunger), 1.0, "clamped to the stat's range");
    c.set_female(true);
    assert!(AnimalCharacter::is_female(&c));
    c.set_variable("idleAction", "sit");
    assert!(pz_animals::Animal::is_animal_sitting(&c));
    c.set_dir(6);
    assert_eq!(c.vars.dir, IsoDirections::E);
    let v = c.animal_character_values(&chars).unwrap();
    assert_eq!(v.stat(AnimalStat::Hunger), 1.0);
    assert!(v.female);
}

#[test]
fn initialize_animal_states_registers_the_twelve_states() {
    let (_sm, _chars, mut c) = animal(2);
    let mut registry = StateRegistry::<i32>::with_ported();
    c.initialize_animal_states(&mut registry);
    for name in [
        "idle",
        "eating",
        "attack",
        "walk",
        "pathfind",
        "followwall",
        "hitreaction",
        "falldown",
        "onground",
        "zone",
        "alerted",
        "climbfence",
    ] {
        assert!(c.state.ai_state_map.contains_key(name), "{name}");
    }
    assert_eq!(c.state.ai_state_map["zone"].id(), ids::ANIMAL_ZONE);
    assert_eq!(
        c.state.default_state.map(|s| s.id()),
        Some(ids::ANIMAL_IDLE)
    );
}

#[test]
fn update_never_runs_player_systems() {
    let (sm, chars, mut c) = animal(3);
    c.animal_fields_mut().unwrap().def = Some(defs());
    c.player.time_since_open_door = 7.0;
    c.player.last_targeted = 3.0;
    c.player.now_ms = 99;
    c.vars.melee_button_down = true;
    let calls = Calls::default();
    let mut w = world(&calls);
    let ctx = Recorder::default();
    let mut registry = StateRegistry::<i32>::with_ported();
    let mut groups = ActionGroups::default();
    let mut cx = StateCx::new(&sm, &chars, &ctx, &mut w);
    c.set_var_bool("bPathfind", true);
    let out = c
        .update_player(&NoInput, &mut registry, &mut groups, &mut cx)
        .unwrap();
    assert!(out.events.is_empty());
    assert_eq!(c.player.time_since_open_door, 7.0);
    assert_eq!(c.player.last_targeted, 3.0);
    assert_eq!(c.player.now_ms, 99);
    assert!(c.vars.melee_button_down, "no input is read for an animal");
    assert!(c.nutrition.is_none() && c.fitness.is_none());
    assert!(!c.in_players);
    // bPathfind puts the animal in AnimalPathFindState; the turn delta is
    // the definition's.
    assert_eq!(c.current_state_id(), Some(ids::ANIMAL_PATH_FIND));
    assert_eq!(c.vars.turn_delta_normal, 0.6);
    // Not initialised: no animal logic, no IsoPlayer.update part.
    assert_eq!(calls.instance.get(), 0);
}

#[test]
fn an_animal_on_a_hook_only_reports_to_the_instance_manager_on_a_server() {
    let (sm, chars, mut c) = animal(4);
    c.animal_fields_mut().unwrap().animal.on_hook = true;
    let calls = Calls::default();
    let mut w = world(&calls);
    struct Server;
    impl SimContext for Server {
        fn is_server(&self) -> bool {
            true
        }
    }
    let mut registry = StateRegistry::<i32>::with_ported();
    let mut out = PlayerUpdate::default();
    let mut cx = StateCx::new(&sm, &chars, &Server, &mut w);
    c.update_animal(&mut registry, &mut out, &mut cx).unwrap();
    assert_eq!(calls.instance.get(), 1);
    let ctx = Recorder::default();
    let mut cx = StateCx::new(&sm, &chars, &ctx, &mut w);
    c.update_animal(&mut registry, &mut out, &mut cx).unwrap();
    assert_eq!(calls.instance.get(), 1);
}

#[test]
fn los_spots_zombies_and_visible_players_but_not_animals() {
    let (sm, mut chars, mut c) = animal(5);
    let mut z = Chr::new(&sm, CharacterKind::Zombie);
    z.id = 10;
    z.path.current_square = Some((1, 1, 0));
    z.vars.x = 3.5;
    z.vars.y = 0.5;
    let mut grapple = Chr::new(&sm, CharacterKind::Zombie);
    grapple.id = 11;
    grapple.path.current_square = Some((1, 1, 0));
    grapple.zombie.reanimated_for_grapple_only = true;
    let mut p = Chr::new(&sm, CharacterKind::Player);
    p.id = 12;
    p.path.current_square = Some((1, 1, 0));
    p.vars.z = 0.5;
    let mut up = Chr::new(&sm, CharacterKind::Player);
    up.id = 13;
    up.path.current_square = Some((1, 1, 2));
    up.vars.z = 2.0;
    let mut other = Chr::new_animal(&sm, &mut chars).unwrap();
    other.id = 14;
    other.path.current_square = Some((1, 1, 0));
    let calls = Calls::default();
    let mut w = world(&calls);
    w.others = vec![z, grapple, p, up, other];
    let ctx = Recorder::default();
    let mut cx = StateCx::new(&sm, &chars, &ctx, &mut w);
    c.animal_update_los(&mut cx);
    let s = &c.animal_fields().unwrap().spotted;
    assert_eq!(s.iter().map(|s| s.chr).collect::<Vec<_>>(), vec![10, 12]);
    assert_eq!(s[0].dist, 3.0);
}

#[test]
fn path_mover_sees_the_animal() {
    let (sm, chars, mut c) = animal(6);
    assert!(c.as_animal().is_some());
    c.animal_fields_mut().unwrap().def = Some(defs());
    let a = c.as_animal().unwrap();
    assert!(a.can_climb_fences());
    assert_eq!(a.animal_size(), 0.8);
    assert!(!a.should_break_obstacles_during_pathfinding());
    let hunger = chars.sim.character_stats.field("HUNGER").unwrap();
    c.stats.set(&hunger, 0.9);
    assert!(c
        .as_animal()
        .unwrap()
        .should_break_obstacles_during_pathfinding());
    let p = Chr::new(&sm, CharacterKind::Player);
    assert!(p.as_animal().is_none());
}

#[test]
fn overrides() {
    let (sm, _chars, mut c) = animal(7);
    assert_eq!(c.animal_object_name(), "Animal");
    assert_eq!(c.animal_anim_set_name(), Some("cow"), "no definition");
    c.animal_fields_mut().unwrap().def = Some(defs());
    assert_eq!(c.animal_calc_damage(), Some(4.0));
    assert_eq!(c.animal_corpse_size(), Some((1.5, 2.0)));
    assert!(
        !c.animal_should_become_zombie_after_death()
            && !c.animal_can_ragdoll()
            && !c.animal_allows_twist()
    );
    assert!(c.animal_can_use_current_pose_for_corpse(true));
    c.animal_fields_mut().unwrap().animal.on_hook = true;
    assert!(!c.animal_can_use_current_pose_for_corpse(true));
    c.animal_fields_mut().unwrap().animal.invincible = true;
    c.animal_set_health(0.2);
    assert_eq!(c.health, 1.0);
    c.animal_set_health(1.5);
    assert_eq!(c.health, 1.5);
    let p = Chr::new(&sm, CharacterKind::Player);
    assert_eq!(c.animal_pushed_by_for_separate(&p), Some(true));
    c.animal_fields_mut().unwrap().animal.behavior.block_movement = true;
    assert_eq!(c.animal_pushed_by_for_separate(&p), Some(false));
    assert_eq!(p.animal_pushed_by_for_separate(&p), None);
}

#[test]
fn remove_from_world_calls_the_population_hooks() {
    let (_sm, _chars, mut c) = animal(8);
    {
        let a = &mut c.animal_fields_mut().unwrap().animal;
        a.on_hook = true;
        a.hook = Some((4, 5, 1));
        a.eat_from_trough = Some(3);
    }
    let calls = Calls::default();
    let mut rec = Rec(&calls);
    let defs = AnimalDefinitionTables::default();
    let mut host = AnimalHost {
        defs: &defs,
        world: &mut rec,
    };
    c.animal_remove_from_world(&mut host);
    let a = &c.animal_fields().unwrap().animal;
    assert_eq!(
        (
            a.attach_back_to_hook_x,
            a.attach_back_to_hook_y,
            a.attach_back_to_hook_z
        ),
        (4, 5, 1)
    );
    assert!(a.eat_from_trough.is_none());
    assert_eq!((calls.removed.get(), calls.recent.get()), (1, 1));
    c.animal_add_to_world(None);
    assert!(c.variable_boolean("onhook"));
}

#[test]
fn hit_consequences_take_the_attacker_and_draw_the_stress() {
    let (sm, chars, mut c) = animal(9);
    c.animal_fields_mut().unwrap().def = Some(defs());
    let mut wielder = Chr::new(&sm, CharacterKind::Player);
    wielder.id = 42;
    let ctx = Recorder::default();
    let calls = Calls::default();
    let mut w = world(&calls);
    let mut cx = StateCx::new(&sm, &chars, &ctx, &mut w);
    pz_core::random::set_seed(5);
    c.animal_hit_consequences(&wielder, 0, true, 1.0, &mut cx)
        .unwrap();
    pz_core::random::set_seed(5);
    let r = pz_core::random::next_range(20, 40);
    let f = c.animal_fields().unwrap();
    assert_eq!(f.animal.stress_level, r as f32);
    assert_eq!(c.vars.attacked_by, Some(42));
    assert_eq!(c.vars.hit_reaction, "hitreact");
    assert_eq!(c.health, 1.0, "ignoreDamage");
}
