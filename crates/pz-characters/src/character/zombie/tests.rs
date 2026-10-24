//! Zombie simulation with invented worlds, lore settings and seeds.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use pz_audio::world_sound::{RoomAt, SoundWorld, WorldSoundManager};
use pz_core::random::Rand;
use pz_pathfind::mover::PathMover;

use super::super::state::action::ActionGroup;
use super::super::state::{ActionGroups, StateCx, StateRegistry, StateWorld};
use super::super::states::{ids, zombie as zs};
use super::super::tests::{fixture, player};
use super::super::*;
use crate::survivor::SurvivorDesc;

/// Seeded draws and a lore matrix.
struct Lore {
    rand: Rand,
    speed: i32,
    memory: i32,
    sight: i32,
    hearing: i32,
    cognition: i32,
    strength: i32,
    events: RefCell<Vec<String>>,
}

impl Lore {
    fn new(seed: i64) -> Self {
        Lore { rand: Rand::new(seed), speed: 2, memory: 2, sight: 2, hearing: 2, cognition: 3, strength: 2, events: RefCell::default() }
    }
}

impl SimContext for Lore {
    fn rand_next(&self, max: i32) -> i32 {
        self.rand.next(max)
    }
    fn rand_next_range(&self, min: i32, max: i32) -> i32 {
        self.rand.next_range(min, max)
    }
    fn rand_next_float(&self, min: f32, max: f32) -> f32 {
        self.rand.next_float(min, max)
    }
    fn lore_speed(&self) -> i32 {
        self.speed
    }
    fn lore_memory(&self) -> i32 {
        self.memory
    }
    fn lore_sight(&self) -> i32 {
        self.sight
    }
    fn lore_hearing(&self) -> i32 {
        self.hearing
    }
    fn lore_cognition(&self) -> i32 {
        self.cognition
    }
    fn lore_strength(&self) -> i32 {
        self.strength
    }
    fn math_random(&self) -> f64 {
        0.5
    }
    fn current_time_millis(&self) -> i64 {
        1000
    }
    fn event(&self, e: SimEvent) {
        self.events.borrow_mut().push(format!("{e:?}"));
    }
}

/// Characters by id on an open, lit plane; sounds in one manager.
#[derive(Default)]
struct Plane {
    chars: Vec<GameCharacter<()>>,
    sounds: WorldSoundManager,
    light: f32,
    pathed: Vec<(i32, i32, i32)>,
}

struct Cells;

impl SoundWorld for Cells {
    fn chunk_loaded(&self, cx: i32, cy: i32) -> bool {
        (0..8).contains(&cx) && (0..8).contains(&cy)
    }
    fn room_at(&self, _x: i32, _y: i32, _z: i32) -> RoomAt {
        RoomAt::Outside
    }
}

static CELLS: Cells = Cells;

impl StateWorld<()> for Plane {
    fn character_mut(&mut self, id: i32) -> Option<&mut GameCharacter<()>> {
        self.chars.iter_mut().find(|c| c.id == id)
    }
    fn characters_near(&self, _x: f32, _y: f32, _z: f32, _r: f32) -> Vec<i32> {
        self.chars.iter().map(|c| c.id).collect()
    }
    fn square(&self, x: i32, y: i32, z: i32) -> Option<SquareInfo> {
        Some(SquareInfo { x, y, z, solid_floor: true, ..Default::default() })
    }
    fn square_light(&self, _sq: (i32, i32, i32), _p: i32) -> f32 {
        self.light
    }
    fn world_sounds(&self) -> Option<(&WorldSoundManager, &dyn SoundWorld)> {
        Some((&self.sounds, &CELLS))
    }
    fn path_to_sound(&mut self, _chr: &mut dyn PathMover, x: i32, y: i32, z: i32) {
        self.pathed.push((x, y, z));
    }
    fn path_to_character(&mut self, chr: &mut dyn PathMover, target: i32) {
        let _ = chr;
        self.pathed.push((target, -1, -1));
    }
}

use super::super::state::SquareInfo;

fn desc(sm: &pz_scripts::ScriptManager, chars: &mut crate::Characters, female: bool) -> crate::SurvivorDescRef<()> {
    let mut d = SurvivorDesc::new(chars, sm).unwrap();
    d.set_female(female);
    d.voice_prefix = Some(if female { "VoiceFemale" } else { "VoiceMale" }.into());
    Rc::new(RefCell::new(d))
}

fn zombie(sm: &pz_scripts::ScriptManager, chars: &mut crate::Characters, ctx: &Lore) -> GameCharacter<()> {
    let d = desc(sm, chars, true);
    let blood = Cell::new(0);
    let mut z = GameCharacter::new_zombie(
        sm,
        ctx,
        super::ZombieSpawn {
            descriptor: None,
            create_survivor: &mut || d.clone(),
            random_blood: &mut || blood.set(blood.get() + 1),
        },
    );
    assert_eq!(blood.get(), 1);
    z.id = 7;
    z
}

#[test]
fn construction_follows_the_lore() {
    let (sm, mut chars) = fixture();
    let ctx = Lore::new(42);
    let z = zombie(&sm, &mut chars, &ctx);
    assert!(z.is_zombie());
    assert!(z.health >= 1.8 && z.health < 2.1);
    assert_eq!(z.voice_prefix(), "FemaleZombie");
    assert!(z.zombie.sprite_name.starts_with("KateZ"));
    assert_eq!(z.zombie.strength, 3);
    assert_eq!(z.zombie.memory, 800);
    assert_eq!(z.zombie.sight, 2);
    assert_eq!(z.zombie.hearing, 2);
    assert!(matches!(z.zombie.speed_type, 2));
    assert!(z.zombie.walk_type.is_some());
    assert_eq!(z.zombie.hurt_sound, "FemaleZombieHurt");

    // Sprinters, eagle sight, pinpoint hearing, long memory, strong.
    let mut ctx = Lore::new(3);
    (ctx.speed, ctx.sight, ctx.hearing, ctx.memory, ctx.strength) = (1, 1, 1, 1, 1);
    let mut seen_sprinter = false;
    for _ in 0..20 {
        let z = zombie(&sm, &mut chars, &ctx);
        assert_eq!((z.zombie.sight, z.zombie.hearing, z.zombie.memory, z.zombie.strength), (1, 1, 1250, 5));
        if z.zombie.speed_type == 1 {
            seen_sprinter = true;
            assert!(z.zombie.walk_type.as_deref().unwrap().starts_with("sprint"));
            assert_eq!(z.vars.turn_delta_normal, 1.0);
        }
    }
    assert!(seen_sprinter);
    // Shamblers walk slow.
    ctx.speed = 3;
    let z = zombie(&sm, &mut chars, &ctx);
    assert!(z.zombie.walk_type.as_deref().unwrap().starts_with("slow"));
    assert_eq!(GameCharacter::<()>::speed_type_from_walk_type(z.zombie.walk_type.as_deref()), 3);
    assert_eq!(z.sandbox_memory_duration(&ctx), 1250);
}

#[test]
fn spotting_a_player_in_front() {
    let (sm, mut chars) = fixture();
    let ctx = Lore::new(9);
    let mut z = zombie(&sm, &mut chars, &ctx);
    z.vars.x = 10.5;
    z.vars.y = 10.5;
    z.path.current_square = Some((10, 10, 0));
    z.set_forward_direction(1.0, 0.0);
    let mut p = player(&sm, &mut chars);
    p.id = 1;
    p.vars.x = 13.5;
    p.vars.y = 10.5;
    p.path.current_square = Some((13, 10, 0));
    let mut world = Plane { chars: vec![p], light: 1.0, ..Default::default() };
    let mut cx = StateCx::new(&sm, &chars, &ctx, &mut world);
    // Full light, in front, 3 squares: certain.
    let r = z.spotted(1, false, &mut cx).unwrap().unwrap();
    assert!(r.success, "{r:?}");
    assert_eq!(z.vars.target, Some(1));
    assert_eq!((z.zombie.last_target_seen_x, z.zombie.last_target_seen_y), (13, 10));
    assert_eq!(z.zombie.bonus_spot_time, 720.0);
    assert_eq!(z.zombie.allow_repath_delay, 480.0);
    assert_eq!(z.vars.time_since_seen_flesh, 0.0);
    assert!(z.is_target_location_known());
    assert!(world.chars[0].seen.seen_this_frame);
    assert_eq!(world.pathed, vec![(1, -1, -1)]);

    // Darkness: no chance.
    let mut z2 = zombie(&sm, &mut chars, &ctx);
    z2.vars.x = 10.5;
    z2.vars.y = 10.5;
    z2.path.current_square = Some((10, 10, 0));
    world.light = 0.0;
    let mut cx = StateCx::new(&sm, &chars, &ctx, &mut world);
    let r = z2.spotted(1, false, &mut cx).unwrap().unwrap();
    assert!(!r.success && r.chance == 0.0);
    assert!(z2.vars.target.is_none());
}

#[test]
fn hearing_a_sound_paths_to_it() {
    let (sm, mut chars) = fixture();
    let ctx = Lore::new(5);
    let mut z = zombie(&sm, &mut chars, &ctx);
    z.vars.x = 20.5;
    z.vars.y = 20.5;
    z.path.current_square = Some((20, 20, 0));
    let mut world = Plane::default();
    let src = pz_audio::world_sound::SoundSource { id: 99, is_player: true, ..Default::default() };
    struct W;
    impl SoundWorld for W {
        fn chunk_loaded(&self, cx: i32, cy: i32) -> bool {
            (0..8).contains(&cx) && (0..8).contains(&cy)
        }
    }
    world.sounds.add_sound(&mut W, Some(src), 25, 20, 0, 30, 40);
    let mut cx = StateCx::new(&sm, &chars, &ctx, &mut world);
    z.vars.time_since_seen_flesh = 1000.0;
    z.respond_to_sound(&mut cx);
    assert_eq!(z.zombie.sound_source_target, Some(99));
    assert!(z.zombie.sound_source_is_player);
    assert!(z.zombie.sound_attract > 0.0);
    // The reaction delay (0..16 draws) runs down before it heads there.
    for _ in 0..2000 {
        if !world.pathed.is_empty() {
            break;
        }
        let mut cx = StateCx::new(&sm, &chars, &ctx, &mut world);
        z.respond_to_sound(&mut cx);
    }
    assert_eq!(world.pathed.len(), 1);
    let (x, y, _) = world.pathed[0];
    assert!((x - 25).abs() <= 2 && (y - 20).abs() <= 2);
    assert_eq!(z.zombie.allow_repath_delay, 120.0);
    assert!(z.is_responding_to_player_sound());
}

#[test]
fn turn_alerted_values() {
    let (sm, mut chars) = fixture();
    let ctx = Lore::new(1);
    let mut z = zombie(&sm, &mut chars, &ctx);
    z.vars.x = 10.5;
    z.vars.y = 10.5;
    z.set_forward_iso_direction(pz_world::square::IsoDirections::E);
    z.set_turn_alerted_values(10, 20);
    assert!(z.zombie.alerted);
    // The exact values are checked against the original by the oracle.
    assert!(!z.variable_string("turnalertedvalue").is_empty());
    assert!(super::super::states::zombie::TURN_ALERTED_TARGET_ANGLE.peek(&z.state.params).is_some());
}

#[test]
fn zombie_variables_and_states() {
    let (sm, mut chars) = fixture();
    let ctx = Lore::new(11);
    let mut z = zombie(&sm, &mut chars, &ctx);
    let mut reg = StateRegistry::with_ported();
    z.initialize_zombie_states(&mut reg);
    assert_eq!(z.try_get_ai_state("lunge").map(|s| s.name()), Some("LungeState"));
    assert_eq!(z.try_get_ai_state("thump").map(|s| s.name()), Some("ThumpState"));
    z.vars.crawling = true;
    z.initialize_zombie_states(&mut reg);
    assert!(z.try_get_ai_state("lunge").is_none());
    assert_eq!(z.try_get_ai_state("turn").map(|s| s.name()), Some("CrawlingZombieTurnState"));
    z.vars.crawling = false;
    z.initialize_zombie_states(&mut reg);

    z.vars.x = 5.5;
    z.vars.y = 5.5;
    z.path.current_square = Some((5, 5, 0));
    let mut p = player(&sm, &mut chars);
    p.id = 1;
    p.vars.x = 6.0;
    p.vars.y = 5.5;
    p.path.current_square = Some((6, 5, 0));
    let mut world = Plane { chars: vec![p], light: 1.0, ..Default::default() };
    let mut cx = StateCx::new(&sm, &chars, &ctx, &mut world);
    z.vars.target = Some(1);
    z.zombie.vector_to_target = (0.5, 0.0);
    z.refresh_zombie_view(&mut cx);
    assert!(z.variable_boolean("bHasTarget"));
    assert!(z.variable_boolean("battack"));
    assert!(z.variable_boolean("blunge"));
    assert_eq!(z.variable_float("distancetotarget", 0.0), 0.5 - 0.3 + 0.25);
    assert!(!z.variable_boolean("bthump"));
    z.set_var_bool("bCrawling", true);
    assert!(z.vars.crawling);
    z.vars.crawling = false;
    // A target on the floor is dropped by `battack`.
    world.chars[0].vars.on_floor = true;
    let mut cx = StateCx::new(&sm, &chars, &ctx, &mut world);
    z.refresh_zombie_view(&mut cx);
    assert!(!z.variable_boolean("battack"));
    z.apply_zombie_var_effects();
    assert!(z.vars.target.is_none());

    // The action group drives the AI states.
    let idle: &[(&str, &str)] =
        &[("to_lunge.xml", "<transition><transitionTo>lunge</transitionTo><conditions><isTrue>blunge</isTrue></conditions></transition>")];
    let mut groups = ActionGroups::default();
    groups.insert(ActionGroup::from_xml("zombie", None, &[("idle", idle), ("lunge", &[])]));
    z.ensure_zombie_action_group(&mut reg, &mut groups);
    world.chars[0].vars.on_floor = false;
    let mut cx = StateCx::new(&sm, &chars, &ctx, &mut world);
    z.vars.target = Some(1);
    z.postupdate_zombie(&mut cx);
    z.vars.target = Some(1);
    z.postupdate_zombie(&mut cx);
    assert_eq!(z.current_state_id(), Some(ids::LUNGE));
    assert_eq!(z.zombie.lunge_timer, 180.0);
    assert!(!pz_pathfind::mover::ZombieMover::is_moving_to_player_sound(&z));
    assert!(z.as_zombie().is_some());
    assert_eq!(super::super::state::vars::CharVar::ZLunge.name(), "blunge");
    let _ = zs::all::<()>();
}

#[test]
fn zombie_bites_through_attack_state() {
    let (sm, mut chars) = super::super::thermoregulator::body_tests::body_fixture();
    let ctx = Lore::new(21);
    let mut z = zombie(&sm, &mut chars, &ctx);
    z.vars.x = 5.5;
    z.vars.y = 5.5;
    z.path.current_square = Some((5, 5, 0));
    z.set_forward_direction(1.0, 0.0);
    let mut p = player(&sm, &mut chars);
    p.id = 1;
    p.vars.x = 6.2;
    p.vars.y = 5.5;
    p.path.current_square = Some((6, 5, 0));
    p.set_forward_direction(-1.0, 0.0);
    let health = p.body_damage.health();
    let mut world = Plane { chars: vec![p], light: 1.0, ..Default::default() };
    let mut cx = StateCx::new(&sm, &chars, &ctx, &mut world);
    z.vars.target = Some(1);
    z.change_state(Some(zs::attack()), &mut cx);
    assert_eq!(z.zombie.attack_outcome, "start");
    z.state_machine_update(&mut cx);
    assert_eq!(z.variable_string("AttackType"), "bite");
    z.set_var_str("PlayerHitReaction", "Bite");
    zs::trigger_player_reaction(&mut z, "Bite", &mut cx).unwrap();
    let p = &world.chars[0];
    assert_eq!(p.vars.attacked_by, Some(7));
    assert!(p.body_damage.health() <= health);
}

#[test]
fn population_creates_a_real_zombie() {
    let (sm, mut chars) = fixture();
    let ctx = Lore::new(4);
    let d = desc(&sm, &mut chars, false);
    let r = super::population::create_real_zombie_always(
        &sm,
        &ctx,
        &[(10, 12, 0), (30, 31, 0)],
        &|_| false,
        pz_world::square::IsoDirections::S,
        false,
        0,
        100.0,
        super::ZombieSpawn { descriptor: Some((d, 1)), create_survivor: &mut || unreachable!(), random_blood: &mut || {} },
    )
    .unwrap();
    let z = &r.zombie;
    assert!(r.spawn_building_key);
    assert_eq!(z.vars.dir, pz_world::square::IsoDirections::S);
    assert!(z.zombie.dress_in_random_outfit);
    assert!(z.health >= 1.5 && z.health < 1.8);
    assert_eq!((z.vars.x as i32, z.vars.y as i32), (r.square.0, r.square.1));
    let mut z = r.zombie;
    z.pop_make_crawler(&ctx, false);
    assert!(z.vars.crawling && !z.zombie.can_walk && z.vars.on_floor);
    z.pop_sit_against_wall(1.5, 2.5, Some(pz_world::square::IsoDirections::W));
    assert!(z.vars.sit_against_wall);
}
