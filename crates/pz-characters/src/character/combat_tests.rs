//! Combat with invented weapons and characters (the field names are the
//! original's code symbols; every number and id is made up).

use std::rc::Rc;

use pz_anim::AnimationVariableSource;
use pz_inventory::ItemClass;
use pz_scripts::objects::items::WeaponCategory;

use super::combat::{WeaponStats, WeaponType};
use super::state::{NullWorld, StateCx, StateWorld};
use super::states::swipe;
use super::tests::{give_item, item_constant, player, Recorder};
use super::thermoregulator::body_tests::body_fixture;
use super::*;

/// Fixed draws: `Next(n)` is `n - 1` (never a hit-chance miss or a
/// one-in-n), ranges their lower bound, floats the middle.
#[derive(Default)]
struct Fixed {
    rec: Recorder,
}

impl SimContext for Fixed {
    fn rand_next(&self, max: i32) -> i32 {
        (max - 1).max(0)
    }
    fn rand_next_range(&self, min: i32, _max: i32) -> i32 {
        min
    }
    fn rand_next_float(&self, min: f32, max: f32) -> f32 {
        (min + max) / 2.0
    }
    fn event(&self, e: SimEvent) {
        self.rec.event(e)
    }
}

fn bat(sm: &pz_scripts::ScriptManager, c: &mut GameCharacter<()>, id: i32) {
    let blunt = item_constant(sm, WeaponCategory::REGISTRY, "BLUNT");
    give_item(
        sm,
        c,
        ItemClass::Weapon,
        "Bat",
        id,
        |t| {
            t.weapon_categories = vec![blunt.location.to_string()];
            t.min_damage = 0.8;
            t.max_damage = 1.2;
        },
        |s| {
            s.two_hand_weapon = true;
            s.critical_chance = 30.0;
            s.crit_dmg_multiplier = 4.0;
            s.actual_weight = 2.0;
            s.max_range = 1.4;
            s.push_back_mod = 0.5;
        },
    );
    if let Some(w) = c.items.item_mut(id).and_then(|i| i.weapon_data_mut()) {
        w.min_damage = 0.8;
        w.max_damage = 1.2;
        w.min_angle = 0.7;
        w.max_angle = 1.0;
    }
    c.items.primary_hand_item = Some(id);
    c.items.secondary_hand_item = Some(id);
}

#[test]
fn weapon_stats_type_and_mods() {
    let (sm, mut chars) = body_fixture();
    let mut c = player(&sm, &mut chars);
    bat(&sm, &mut c, 5);
    let w = WeaponStats::of(&sm, c.items.item(5).unwrap()).unwrap();
    assert!(w.is_melee() && w.two_hand && w.is_of_category(&sm, "BLUNT"));
    assert_eq!(c.weapon_type(&sm), WeaponType::TwoHanded);
    assert!(!c.get_variable_boolean("rangedWeapon"));
    c.items.secondary_hand_item = None;
    assert_eq!(c.weapon_type_of(&sm), WeaponType::OneHanded);
    assert_eq!(c.hitting_mod(&chars), 0.75);
    assert_eq!(c.weapon_damage_mod(&sm, &chars, &w), 1.0);
    // Strength 5: 1.0 shoving, stagger mod = pushBackMod.
    c.set_perk_level_debug(&chars.perks.constant("Strength").unwrap(), 5, &Fixed::default());
    assert_eq!(c.shoving_mod(&chars), 1.0);
    assert_eq!(c.weapon_stagger_back_time_mod(&sm, &chars, &w, true), 0.5);
}

#[test]
fn process_hit_damage_by_hand() {
    let (sm, mut chars) = body_fixture();
    let mut wielder = player(&sm, &mut chars);
    bat(&sm, &mut wielder, 5);
    let w = WeaponStats::of(&sm, wielder.items.item(5).unwrap()).unwrap();
    let mut z: GameCharacter<()> = GameCharacter::new(&sm, CharacterKind::Zombie);
    z.vars.x = 1.0;
    z.set_forward_direction(1.0, 0.0);
    z.vars.dir = pz_world::square::IsoDirections::E;
    // Facing away from the wielder at the origin: dot 1 > -0.3, x1.5.
    let d = z.process_hit_damage(&sm, &chars, &w, &wielder, 1.0, false, 1.0);
    // 1.0 * 1.5 * 1.5 (non-player) * 0.3 (weapon level 0).
    assert_eq!(d, 1.0 * 1.5 * 1.5 * 0.3);
    // hitForce: min(1, 1 * 0.75) * 2 for a player wielder.
    assert_eq!(z.vars.hit_force, 0.75 * 2.0);
    // A critical hit: x max(2, 4).
    wielder.vars.critical_hit = true;
    let d2 = z.process_hit_damage(&sm, &chars, &w, &wielder, 1.0, false, 1.0);
    assert_eq!(d2, d * 4.0);
    // Ignored damage: no multipliers but the received/level ones.
    wielder.vars.critical_hit = false;
    let d3 = z.process_hit_damage(&sm, &chars, &w, &wielder, 1.0, true, 1.0);
    assert_eq!(d3, 1.0 * 1.5 * 0.3);
    assert_eq!(z.vars.hit_force, 0.75 / 2.7);
    // One-handed use of the two-handed bat halves it.
    wielder.items.secondary_hand_item = None;
    let d4 = z.process_hit_damage(&sm, &chars, &w, &wielder, 1.0, true, 1.0);
    assert_eq!(d4, d3 * 0.5);
}

#[test]
fn crit_chance_melee_and_shove() {
    let (sm, mut chars) = body_fixture();
    let mut p = player(&sm, &mut chars);
    bat(&sm, &mut p, 5);
    let z: GameCharacter<()> = GameCharacter::new(&sm, CharacterKind::Zombie);
    let ctx = Fixed::default();
    // 30 - 30/5 (no charge) = 24, blunt 0.
    assert_eq!(p.calculate_crit_chance(&sm, &chars, &ctx, &z), 24);
    p.vars.charge_time = 2.0;
    assert_eq!(p.calculate_crit_chance(&sm, &chars, &ctx, &z), 30);
    p.vars.do_shove = true;
    assert_eq!(p.calculate_crit_chance(&sm, &chars, &ctx, &z), 35);
    let target = player(&sm, &mut chars);
    assert_eq!(p.calculate_crit_chance(&sm, &chars, &ctx, &target), 20);
}

/// A world of characters by id.
struct World(Vec<GameCharacter<()>>);

impl StateWorld<()> for World {
    fn character_mut(&mut self, id: i32) -> Option<&mut GameCharacter<()>> {
        self.0.iter_mut().find(|c| c.id == id)
    }
    fn take_character(&mut self, id: i32) -> Option<GameCharacter<()>> {
        let i = self.0.iter().position(|c| c.id == id)?;
        Some(self.0.remove(i))
    }
    fn put_character(&mut self, chr: GameCharacter<()>) {
        self.0.push(chr);
    }
    fn characters_near(&self, _x: f32, _y: f32, _z: f32, _r: f32) -> Vec<i32> {
        self.0.iter().map(|c| c.id).collect()
    }
}

#[test]
fn swipe_hits_the_zombie_in_front() {
    let (sm, mut chars) = body_fixture();
    let mut p = player(&sm, &mut chars);
    p.id = 1;
    bat(&sm, &mut p, 5);
    p.set_forward_direction(1.0, 0.0);
    let mut z: GameCharacter<()> = GameCharacter::new(&sm, CharacterKind::Zombie);
    z.id = 2;
    z.vars.x = 1.0;
    z.health = 10.0;
    let mut behind: GameCharacter<()> = GameCharacter::new(&sm, CharacterKind::Zombie);
    behind.id = 3;
    behind.vars.x = -1.0;
    let mut world = World(vec![z, behind]);
    let ctx = Fixed::default();
    let mut cx = StateCx::new(&sm, &chars, &ctx, &mut world);
    p.change_state(Some(swipe::instance()), &mut cx);
    assert_eq!(p.attack_vars.target_standing, Some(2));
    assert!(!p.attack_vars.do_shove);
    let e = pz_anim::AnimEventRecord {
        state_name: None,
        layer_depth: 0,
        track: None,
        event: Rc::new(pz_anim::AnimEvent::named("AttackCollisionCheck", pz_anim::AnimEventTime::Percentage, 0.5)),
    };
    p.in_players = true;
    p.on_anim_event(&e, &mut cx);
    assert!(swipe::ATTACKED.get(&mut p.state.params), "{:?}", ctx.rec.events.borrow());
    assert_eq!(p.vars.last_hit_count, 1);
    let z = world.0.iter().find(|c| c.id == 2).unwrap();
    // Damage 1.0 (mid draw) * 1 * 0.75 hitting mod / (1 * 0.5) = 1.5,
    // processHitDamage: x1.5 (from behind? no: facing E, dot 1) x1.5 x0.3,
    // reduced by 0.15 on the zombie's health.
    let split = 1.0f32 * 1.0 * 0.75 / 0.5;
    // modDelta is rangeDel: distance / maxRange * 2.
    let range_del = 1.0f32 / 1.4 * 2.0;
    let dmg = split * range_del * 1.5 * 1.5 * 0.3;
    assert_eq!(z.health, 10.0 - dmg * 0.15);
    assert_eq!(z.vars.attacked_by, Some(1));
    assert!(z.state.action_context.has_event_occurred("wasHit", None));
    assert!(world.0.iter().find(|c| c.id == 3).unwrap().vars.attacked_by.is_none());
    let mut nw = NullWorld;
    let mut cx = StateCx::new(&sm, &chars, &ctx, &mut nw);
    p.change_state(None, &mut cx);
    assert!(!p.vars.do_shove);
}
