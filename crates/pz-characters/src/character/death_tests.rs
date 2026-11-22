//! `DoDeath` with invented zombies and weapons.

use std::cell::RefCell;

use crate::character::combat::WeaponStats;
use crate::character::death::Killer;
use crate::character::state::{NullWorld, StateCx};
use crate::character::tests::Recorder;
use crate::character::thermoregulator::body_tests::body_fixture;
use crate::character::{CharacterKind, GameCharacter, SimContext, SimEvent};

/// Every one-in-n draw hits; records the events.
struct Zero {
    blood: i32,
    events: RefCell<Vec<String>>,
}

impl SimContext for Zero {
    fn rand_next(&self, _max: i32) -> i32 {
        0
    }
    fn rand_next_float(&self, min: f32, _max: f32) -> f32 {
        min
    }
    fn blood_level(&self) -> i32 {
        self.blood
    }
    fn event(&self, e: SimEvent) {
        self.events.borrow_mut().push(format!("{e:?}"));
    }
}

fn zero(blood: i32) -> Zero {
    Zero { blood, events: RefCell::new(Vec::new()) }
}

fn dead_zombie() -> GameCharacter<()> {
    let (sm, _) = body_fixture();
    let mut z = GameCharacter::new(&sm, CharacterKind::Zombie);
    z.health = 0.0;
    (z.vars.hit_dir_x, z.vars.hit_dir_y) = (1.0, 0.0);
    z
}

fn weapon(splat_number: i32, bare_hands: bool) -> WeaponStats {
    WeaponStats { id: 9, splat_number, bare_hands, ..WeaponStats::default() }
}

fn count(ctx: &Zero, prefix: &str) -> usize {
    ctx.events.borrow().iter().filter(|e| e.starts_with(prefix)).count()
}

#[test]
fn weapon_kill_plays_the_sound_then_splats_blood_and_giblets() {
    let mut z = dead_zombie();
    let ctx = zero(3);
    let killer = Killer::unknown(5);

    z.do_death(Some(&weapon(2, false)), Some(&killer), true, &ctx);

    let events = ctx.events.borrow().clone();
    assert_eq!(events[0], format!("{:?}", SimEvent::PlaySound("HeadSmash".into())));
    assert_eq!(count(&ctx, "SplatBlood "), 2);
    assert_eq!(count(&ctx, "CharacterSplatBloodFloorBig"), 1);
    assert_eq!(count(&ctx, "BloodSplatterSound"), 1);
    // The first giblet, then one pass at normal gore: 4 + 6 and an eye.
    assert_eq!(count(&ctx, "Giblet"), 12);
    assert_eq!(events.iter().filter(|e| e.contains("eye: true")).count(), 1);
    assert_eq!(z.zombie.sound_state, "Death");
    assert!(!z.vars.do_death_sound);
}

#[test]
fn bare_hands_kill_on_the_floor_scatters_giblets_around() {
    let mut z = dead_zombie();
    z.vars.on_floor = true;
    let ctx = zero(3);
    let killer = Killer { player: true, ..Killer::unknown(5) };

    z.do_death(Some(&weapon(1, true)), Some(&killer), true, &ctx);

    assert_eq!(count(&ctx, "SplatBlood "), 1);
    assert_eq!(count(&ctx, "BloodSplatterSound"), 1);
    // The eight squares around, then an eye.
    assert_eq!(count(&ctx, "Giblet"), 9);
    assert!(ctx.events.borrow().last().unwrap().contains("eye: true"));
}

#[test]
fn no_blood_level_only_plays_the_sound() {
    let mut z = dead_zombie();
    z.vars.close_killed = true;
    let ctx = zero(1);

    z.do_death(Some(&weapon(2, false)), None, true, &ctx);

    assert_eq!(*ctx.events.borrow(), [format!("{:?}", SimEvent::PlaySound("HeadStab".into()))]);
}

#[test]
fn long_blade_killer_plays_head_slice() {
    let mut z = dead_zombie();
    let ctx = zero(1);
    let killer = Killer { attacking_long_blade: true, ..Killer::unknown(5) };

    z.do_death(None, Some(&killer), true, &ctx);

    assert_eq!(*ctx.events.borrow(), [format!("{:?}", SimEvent::PlaySound("HeadSlice".into()))]);
}

#[test]
fn kill_runs_do_death_once() {
    let (sm, chars) = body_fixture();
    let mut z: GameCharacter<()> = GameCharacter::new(&sm, CharacterKind::Zombie);
    let rec = Recorder::default();

    z.kill_by(None, &mut StateCx::new(&sm, &chars, &rec, &mut NullWorld));
    z.kill_by(None, &mut StateCx::new(&sm, &chars, &rec, &mut NullWorld));

    let events = rec.events.borrow();
    let killed = events.iter().position(|e| e.starts_with("Killed")).unwrap();
    assert_eq!(events[killed + 1], format!("{:?}", SimEvent::PlaySound("HeadSmash".into())));
    assert_eq!(events.iter().filter(|e| e.starts_with("Killed")).count(), 1);
    assert_eq!(events.iter().filter(|e| e.starts_with("PlaySound")).count(), 1);
}
