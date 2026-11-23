//! `IsoZombie.onKilled`'s `DoDeath`: the death sound, the blood and the
//! giblets of a killed zombie (`IsoGameCharacter.doDeathSplatterAndSounds`).

use pz_scripts::ScriptManager;

use super::combat::WeaponStats;
use super::context::{SimContext, SimEvent};
use super::state::StateCx;
use super::states::ids;
use super::GameCharacter;

/// What `DoDeath` reads about the killer (`getAttackedBy()`).
#[derive(Clone, Debug)]
pub struct Killer {
    pub id: i32,
    /// `instanceof IsoPlayer`.
    pub player: bool,
    /// `getPrimaryHandItem()` when it is a `HandWeapon`.
    pub primary_weapon: Option<WeaponStats>,
    /// `getAttackingWeapon()` (`useHandWeapon`) is a `LONG_BLADE`.
    pub attacking_long_blade: bool,
    /// `getUseHandWeapon()`.
    pub use_hand_weapon: Option<WeaponStats>,
}

impl Killer {
    /// A killer that cannot be read.
    pub fn unknown(id: i32) -> Killer {
        Killer { id, player: false, primary_weapon: None, attacking_long_blade: false, use_hand_weapon: None }
    }

    pub fn of<I: 'static>(sm: &ScriptManager, c: &GameCharacter<I>) -> Killer {
        let stats = |item: Option<pz_inventory::shared::ItemRef>| item.and_then(|i| WeaponStats::of(sm, &i.borrow()));
        let use_hand_weapon = stats(c.vars.use_hand_weapon.and_then(|id| c.items.item(id)));
        Killer {
            id: c.id,
            player: c.is_player(),
            primary_weapon: stats(c.items.primary_hand_item()),
            attacking_long_blade: use_hand_weapon.as_ref().is_some_and(|w| w.is_of_category(sm, "LONG_BLADE")),
            use_hand_weapon,
        }
    }
}

impl<I: 'static> GameCharacter<I> {
    /// `IsoZombie.onKilled(killer, weapon, isGory)` after `DoZombieInventory`
    /// and `OnZombieDead` (the owner's part, from [`SimEvent::Killed`]):
    /// `DoDeath` with the weapon, else the killer's primary hand weapon,
    /// else this zombie's `useHandWeapon`.
    pub(crate) fn zombie_on_killed(
        &mut self,
        killer: Option<&Killer>,
        weapon: Option<&WeaponStats>,
        gory: bool,
        cx: &mut StateCx<'_, I>,
    ) {
        let own = || {
            self.vars
                .use_hand_weapon
                .and_then(|id| self.items.item(id))
                .and_then(|i| WeaponStats::of(cx.sm, &i.borrow()))
        };
        let weapon = match (killer, weapon) {
            (None, _) => None,
            (Some(_), Some(w)) => Some(w.clone()),
            (Some(k), None) => k.primary_weapon.clone().or_else(own),
        };
        self.do_death(weapon.as_ref(), killer, gory, cx.ctx);
    }

    /// `IsoPlayer.onKilled(killer, weapon, isGory)`: `DoDeath` with the
    /// killer's `useHandWeapon` (the weapon argument is not used). Its
    /// `OnDeath()` is [`GameCharacter::player_on_death`].
    pub(crate) fn player_on_killed(&mut self, killer: Option<&Killer>, gory: bool, cx: &mut StateCx<'_, I>) {
        let weapon = killer.and_then(|k| k.use_hand_weapon.clone());
        self.player_on_death(cx);
        if cx.has_host_error() {
            return;
        }
        self.do_death(weapon.as_ref(), killer, gory, cx.ctx);
    }

    /// `DoDeath(weapon, killer, isGory)` in single player: `OnDeath()`
    /// (`OnCharacterDeath`, raised by the owner with `OnZombieDead`), then
    /// `doDeathSplatterAndSounds`.
    pub(crate) fn do_death(
        &mut self,
        weapon: Option<&WeaponStats>,
        killer: Option<&Killer>,
        gory: bool,
        ctx: &dyn SimContext,
    ) {
        // `IsoZombie.onDeath_ShouldDoSplatterAndSounds`.
        if self.is_zombie() && self.zombie.is_reanimated_for_grapple_only() {
            return;
        }
        if self.vars.do_death_sound {
            self.play_dead_sound(killer, ctx);
        }
        self.vars.do_death_sound = false;
        if !self.is_dead() {
            return;
        }
        let mut dz = 0.5f32;
        if self.is_zombie() && (self.vars.crawling || self.is_current_state_id(ids::ZOMBIE_ON_GROUND)) {
            dz = 0.2;
        }
        let level = ctx.blood_level();
        if let Some(w) = weapon.filter(|_| level > 1 && gory) {
            let mut n = w.splat_number.max(1);
            if ctx.is_last_stand() {
                n *= 3;
            }
            match level {
                2 => n /= 2,
                4 => n *= 2,
                5 => n *= 5,
                _ => {}
            }
            for _ in 0..n {
                ctx.event(SimEvent::SplatBlood { amount: 3, speed: 0.3 });
            }
            ctx.event(SimEvent::CharacterSplatBloodFloorBig { chr: self.id });
        }
        // `killer.xp.AddXP(weapon, 3)` is empty in 42.21.
        let (x, y, z) = (self.vars.x, self.vars.y, self.vars.z + dz);
        let (hx, hy) = (self.vars.hit_dir_x, self.vars.hit_dir_y);
        // `weapon == ((IsoPlayer) killer).bareHands`: a player's
        // `BareHands` weapon.
        let bare_hands = match (killer, weapon) {
            (Some(k), Some(w)) => k.player && w.bare_hands,
            _ => false,
        };
        if level > 1 && self.vars.on_floor && bare_hands && gory {
            ctx.event(SimEvent::BloodSplatterSound { chr: self.id });
            for i in -1..=1 {
                for j in -1..=1 {
                    if i == 0 && j == 0 {
                        continue;
                    }
                    let vx = i as f32 * ctx.rand_next_float(0.25, 0.5);
                    let vy = j as f32 * ctx.rand_next_float(0.25, 0.5);
                    giblet(ctx, false, x, y, z, vx, vy);
                }
            }
            giblet(ctx, true, x, y, z, hx * 0.8, hy * 0.8);
        } else if level > 1 && gory {
            ctx.event(SimEvent::BloodSplatterSound { chr: self.id });
            giblet(ctx, false, x, y, z, hx * 1.5, hy * 1.5);
            let (mut rand, mut rand2, mut repeat) = (3, 0, 1);
            match level {
                1 => repeat = 0,
                2 => {
                    repeat = 1;
                    rand = 5;
                    rand2 = 2;
                }
                4 => {
                    repeat = 3;
                    rand = 2;
                }
                5 => {
                    repeat = 10;
                    rand = 0;
                }
                _ => {}
            }
            let close = self.vars.close_killed;
            let a = |ctx: &dyn SimContext| ctx.rand_next(if close { 8 } else { rand }) == 0;
            let b = |ctx: &dyn SimContext| ctx.rand_next(if close { 4 } else { rand2 }) == 0;
            for _ in 0..repeat {
                for m in [1.5, 1.5, 1.8, 1.9] {
                    if a(ctx) {
                        giblet(ctx, false, x, y, z, hx * m, hy * m);
                    }
                }
                for m in [3.5, 3.8, 3.9, 1.5, 3.8, 3.9] {
                    if b(ctx) {
                        giblet(ctx, false, x, y, z, hx * m, hy * m);
                    }
                }
                if ctx.rand_next(if close { 9 } else { 6 }) == 0 {
                    giblet(ctx, true, x, y, z, hx * 0.8, hy * 0.8);
                }
            }
        }
    }

    /// `playDeadSound()` in single player.
    fn play_dead_sound(&mut self, killer: Option<&Killer>, ctx: &dyn SimContext) {
        if self.is_animal() {
            return;
        }
        // `isKilledBySlicingWeapon()`.
        let slicing = !self.vars.damaged_by_vehicle && killer.is_some_and(|k| k.attacking_long_blade);
        if self.vars.close_killed {
            ctx.event(SimEvent::PlaySound("HeadStab".into()));
        } else if slicing {
            ctx.event(SimEvent::PlaySound("HeadSlice".into()));
        } else if self.is_zombie() {
            ctx.event(SimEvent::PlaySound("HeadSmash".into()));
        } else if self.is_player() && !self.death_drag_down && !self.killed_by_fall {
            ctx.event(SimEvent::PlayerVoiceSoundNamed("DeathAlone".into()));
        }
        if self.is_zombie() {
            // `parameterZombieState.setState(Death)`.
            self.zombie.sound_state = "Death";
        }
    }
}

/// `new IsoZombieGiblets(type, cell, x, y, z, xvel, yvel)`: the
/// constructor's draws (the velocity's spread and `invisible`).
fn giblet(ctx: &dyn SimContext, eye: bool, x: f32, y: f32, z: f32, vel_x: f32, vel_y: f32) {
    let rx = ctx.rand_next(4000) as f32 / 10000.0 - 0.2;
    let ry = ctx.rand_next(4000) as f32 / 10000.0 - 0.2;
    let invisible = ctx.rand_next(3) != 0;
    ctx.event(SimEvent::Giblet { x, y, z, vel_x: vel_x + rx, vel_y: vel_y + ry, invisible, eye });
}

#[cfg(test)]
#[path = "death_tests.rs"]
mod tests;
