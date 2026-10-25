use super::*;
use crate::character::state::{SquareInfo, StateWorld};
use crate::character::tests::{fixture, player, Recorder};
use crate::character::thermoregulator::body_tests::body_fixture;
use pz_world::fire::character::{set_on_fire, stop_burning};

fn frame() -> CharacterFireFrame {
    CharacterFireFrame { multiplier: 1.0, thirty_fps: 1.0, lock_fps: 30 }
}

#[test]
fn a_burning_zombie_loses_health_until_it_dies() {
    let (sm, chars) = fixture();
    let ctx = Recorder::default();
    let mut z = GameCharacter::<()>::new(&sm, CharacterKind::Zombie);
    z.id = 3;
    z.health = 0.001;
    let mut mgr = IsoFireManager::new();
    let mut rand = CtxRand(&ctx);
    set_on_fire(&mut mgr, &mut z.as_fire(&sm, &chars, &ctx), &mut rand);
    assert!(z.vars.on_fire && z.vars.fire_anim.is_some());
    assert_eq!(mgr.burning_characters, vec![3]);
    let before = z.health;
    reduce_health_when_burning(&mut mgr, &mut z.as_fire(&sm, &chars, &ctx), frame(), &mut rand);
    assert_eq!(z.health, before - 0.0038 / 20.0);
    for _ in 0..10 {
        reduce_health_when_burning(&mut mgr, &mut z.as_fire(&sm, &chars, &ctx), frame(), &mut rand);
    }
    assert!(z.is_dead());
    assert!(mgr.burning_characters.is_empty());
    let ev = ctx.events.borrow();
    assert!(ev.iter().any(|e| e.contains("BurningFlesh")));
    assert_eq!(ev.iter().filter(|e| *e == "ZombieDead").count(), 1);
}

#[test]
fn a_burning_player_burns_a_part_and_takes_general_damage() {
    let (sm, mut chars) = body_fixture();
    let ctx = Recorder::default();
    let mut p = player(&sm, &mut chars);
    p.id = 1;
    let mut mgr = IsoFireManager::new();
    let mut rand = CtxRand(&ctx);
    set_on_fire(&mut mgr, &mut p.as_fire(&sm, &chars, &ctx), &mut rand);
    assert!(p.body_damage.parts.iter().any(|b| b.burn_time > 0.0));
    let parts = |p: &GameCharacter<()>| p.body_damage.parts.iter().map(|b| b.health).sum::<f32>();
    let health = parts(&p);
    let mut c = p.as_fire(&sm, &chars, &ctx);
    c.burn_player(1.0);
    assert!(parts(&p) < health && p.body_damage.is_on_fire);
    assert!(ctx.events.borrow().iter().any(|e| e.contains("PlayerGetDamage") && e.contains("FIRE")));
    stop_burning(&mut mgr, &mut p.as_fire(&sm, &chars, &ctx));
    assert!(!p.vars.on_fire && p.vars.fire_anim.is_none());
    assert!(ctx.events.borrow().iter().any(|e| e.contains("StopSound(\"BurningFlesh\")")));
}

/// A world whose fire manager has no squares: only the burning damage.
#[derive(Default)]
struct Burning {
    mgr: IsoFireManager,
    calls: usize,
}

impl StateWorld<()> for Burning {
    fn square(&self, x: i32, y: i32, z: i32) -> Option<SquareInfo> {
        Some(SquareInfo { x, y, z, solid_floor: true, ..Default::default() })
    }
    fn character_fire(&mut self, chr: &mut dyn FireCharacter, frame: CharacterFireFrame, rand: &mut dyn GameRand) {
        self.calls += 1;
        reduce_health_when_burning(&mut self.mgr, chr, frame, rand);
    }
}

#[test]
fn the_update_hands_the_character_to_the_world() {
    let (sm, chars) = fixture();
    let ctx = Recorder::default();
    let mut z = GameCharacter::<()>::new(&sm, CharacterKind::Zombie);
    z.vars.on_fire = true;
    let mut w = Burning::default();
    let mut cx = StateCx::new(&sm, &chars, &ctx, &mut w);
    z.update_fire(&mut cx);
    z.update_fire(&mut cx);
    assert_eq!(w.calls, 2);
    let d = 0.0038f32 / 20.0 * fire_frame(&ctx).multiplier;
    assert_eq!(z.health, 1.0 - d - d);
}
