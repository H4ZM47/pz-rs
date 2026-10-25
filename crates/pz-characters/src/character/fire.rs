//! Characters and fire: [`FireCharacter`] over a [`GameCharacter`]
//! (players, zombies and animals), and the `IsoGameCharacter.update` step
//! that runs `FireCheck()`, `SpreadFire()` and `ReduceHealthWhenBurning()`
//! ([`GameCharacter::update_fire`]).
//!
//! The fire manager and the squares are the world's (pz-world's
//! `IsoFireManager` and `FireWorld`): the character update hands the
//! character to [`StateWorld::character_fire`], which runs
//! [`update_character_fire`] with them. `SetOnFire()` and `StopBurning()`
//! are `pz_world::fire::character::{set_on_fire, stop_burning}` over a
//! [`CharacterFire`].
//!
//! Not here: `onFireLightSourceCheck` (the light is the renderer's, from
//! `vars.on_fire`) and the multiplayer paths.
//!
//! [`StateWorld::character_fire`]: super::state::StateWorld::character_fire

use pz_scripts::ScriptManager;
use pz_world::fire::character::{
    fire_check, reduce_health_when_burning, spread_fire, CharacterFireFrame, CharacterKind as FireKind, FireCharacter,
};
use pz_world::fire::manager::{CharacterId, FireWorld, IsoFireManager};
use pz_world::java::GameRand;

use super::context::{SimContext, SimEvent};
use super::state::StateCx;
use super::{CharacterKind, GameCharacter};
use crate::Characters;

/// The global `Rand` (through the [`SimContext`]) as the world's
/// [`GameRand`], so fire draws stay in the game's stream.
pub struct CtxRand<'a>(pub &'a dyn SimContext);

impl GameRand for CtxRand<'_> {
    fn next(&mut self, max: i32) -> i32 {
        self.0.rand_next(max)
    }

    fn next_float(&mut self) -> f32 {
        self.0.rand_next_float(0.0, 1.0)
    }

    fn next_range(&mut self, min: i32, max: i32) -> i32 {
        self.0.rand_next_range(min, max)
    }

    fn next_float_range(&mut self, min: f32, max: f32) -> f32 {
        self.0.rand_next_float(min, max)
    }
}

/// The frame values fire reads, from the [`SimContext`].
pub fn fire_frame(ctx: &dyn SimContext) -> CharacterFireFrame {
    CharacterFireFrame { multiplier: ctx.multiplier(), thirty_fps: ctx.thirty_fps_multiplier(), lock_fps: ctx.lock_fps() }
}

/// `FireCheck()`, `SpreadFire()` and `ReduceHealthWhenBurning()`, in the
/// order `IsoGameCharacter.update` calls them (what a
/// [`super::state::StateWorld::character_fire`] runs).
pub fn update_character_fire(
    mgr: &mut IsoFireManager,
    w: &mut FireWorld<'_>,
    c: &mut dyn FireCharacter,
    frame: CharacterFireFrame,
    rand: &mut dyn GameRand,
) {
    fire_check(mgr, w, c, frame, rand);
    spread_fire(mgr, w, c, frame, rand);
    reduce_health_when_burning(mgr, c, frame, rand);
}

/// A character as fire sees it (`IsoGameCharacter`'s fire fields, body
/// damage and events).
pub struct CharacterFire<'a, I: 'static> {
    pub chr: &'a mut GameCharacter<I>,
    pub sm: &'a ScriptManager,
    pub chars: &'a Characters,
    pub ctx: &'a dyn SimContext,
}

/// The fire manager's id of a character (its `getID()`).
pub fn fire_character_id(id: i32) -> CharacterId {
    u64::from(id as u32)
}

impl<I: 'static> FireCharacter for CharacterFire<'_, I> {
    fn id(&self) -> CharacterId {
        fire_character_id(self.chr.id)
    }

    fn kind(&self) -> FireKind {
        match self.chr.kind {
            CharacterKind::Player => FireKind::Player,
            CharacterKind::Zombie => FireKind::Zombie,
            CharacterKind::Animal => FireKind::Animal,
            CharacterKind::Survivor => FireKind::Other,
        }
    }

    fn square(&self) -> Option<(i32, i32, i32)> {
        self.chr.path.current_square
    }

    fn on_fire(&self) -> bool {
        self.chr.vars.on_fire
    }

    fn set_on_fire_flag(&mut self, on: bool) {
        self.chr.vars.on_fire = on;
    }

    fn in_vehicle(&self) -> bool {
        self.chr.path.vehicle.is_some()
    }

    fn fire_kill_rate(&self) -> f32 {
        self.chr.vars.fire_kill_rate
    }

    fn fire_spread_probability(&self) -> i32 {
        self.chr.vars.fire_spread_probability
    }

    fn is_god_mode(&self) -> bool {
        self.chr.is_god_mod(self.ctx)
    }

    fn is_dead(&self) -> bool {
        self.chr.is_dead()
    }

    fn is_running(&self) -> bool {
        self.chr.is_running(self.sm)
    }

    fn attach_fire(&mut self, dx: i32, dy: i32, part_index: Option<i32>) {
        self.chr.vars.fire_anim = Some((dx, dy));
        if let Some(i) = part_index.and_then(|i| usize::try_from(i).ok()) {
            if let Ok(env) = self.chr.injury_env(self.sm, self.ctx) {
                if let Some(p) = self.chr.body_damage.parts.get_mut(i) {
                    p.set_burned(&env);
                }
            }
        }
        // `if (!getEmitter().isPlaying("BurningFlesh")) playSoundImpl(...)`:
        // the emitter knows whether it plays.
        self.ctx.event(SimEvent::PlaySound("BurningFlesh".into()));
    }

    fn burn_part_range(&self) -> (i32, i32) {
        let types = &self.chars.sim.body_part_types;
        (types.index("Hand_L").map_or(0, |i| i as i32), types.max_index() as i32)
    }

    fn detach_fire(&mut self) {
        self.chr.vars.fire_anim = None;
        self.ctx.event(SimEvent::StopSound("BurningFlesh"));
    }

    fn apply_damage(&mut self, damage: f32) {
        self.chr.apply_damage_checked(self.ctx, damage);
    }

    fn clear_attacked_by(&mut self) {
        self.chr.vars.attacked_by = None;
    }

    fn burn_player(&mut self, dpm: f32) {
        self.chr.reduce_general_health(self.chars, dpm);
        self.ctx.event(SimEvent::PlayerGetDamage { kind: "FIRE", amount: dpm });
        self.chr.body_damage.is_on_fire = true;
    }

    fn force_awake(&mut self) {
        self.chr.force_awake();
    }

    fn zombie_died(&mut self) {
        self.ctx.event(SimEvent::ZombieDead);
    }
}

impl<I: 'static> GameCharacter<I> {
    /// This character as a [`FireCharacter`].
    pub fn as_fire<'a>(&'a mut self, sm: &'a ScriptManager, chars: &'a Characters, ctx: &'a dyn SimContext) -> CharacterFire<'a, I> {
        CharacterFire { chr: self, sm, chars, ctx }
    }

    /// `IsoGameCharacter.update`'s fire step (after the state machine):
    /// `FireCheck()`, `SpreadFire()`, `ReduceHealthWhenBurning()` through
    /// the world's fire manager.
    pub fn update_fire(&mut self, cx: &mut StateCx<'_, I>) {
        let (sm, chars, ctx) = (cx.sm, cx.chars, cx.ctx);
        let frame = fire_frame(ctx);
        let mut rand = CtxRand(ctx);
        let mut c = self.as_fire(sm, chars, ctx);
        cx.world.character_fire(&mut c, frame, &mut rand);
    }
}

#[cfg(test)]
mod tests;
