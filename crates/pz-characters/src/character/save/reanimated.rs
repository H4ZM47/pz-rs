//! `IsoZombie.load`/`save` and `ReanimatedPlayers`' `reanimated.bin`: the
//! zombies a dead player turned into, which the game keeps outside the
//! zombie population (the only zombies saved with their character record).
//!
//! The record is `IsoGameCharacter.save` (with `IsoMovingObject.save`; a
//! zombie writes no stats, body damage, XP or hand items), then the
//! zombie's own part: its version (1), `timeSinceSeenFlesh`, the
//! `ZombieStateFlags` and the worn items.
//!
//! Not modelled (rendering and the cell's lists): `InitSpritePartsZombie`'s
//! sprite (its `zombieId` draw is kept), the sprite tints (their draws are
//! kept), `setAlpha(0)` and the zombie list the load adds the zombie to
//! (the caller's list).

use pz_inventory::io::{Reader, Writer};
use pz_popman::state_flags::{ZombieStateFlags, ZombieStateSource};
use pz_scripts::{ScriptError, ScriptManager, ScriptResult};

use super::super::context::SimContext;
use super::super::states::zombie::idle;
use super::super::GameCharacter;
use super::{e, SaveEnv};
use crate::worn_items::ItemIdentity;
use crate::Characters;

/// The version `IsoZombie.save` writes after the character record.
pub const REANIMATED_FILE_VERSION: i32 = 1;

impl<I: ItemIdentity + 'static> GameCharacter<I> {
    /// `ZombieStateFlags.intFromZombie(this)`. `isFakeDead()` clears the
    /// flag with `disableFakeDead` 3 or on a square with stairs
    /// (`on_stairs`), as the getter does.
    pub fn zombie_state_flags(&mut self, ctx: &dyn SimContext, on_stairs: bool) -> i32 {
        let crawling = self.vars.crawling;
        let can_walk = self.zombie.can_walk;
        let fake_dead = self.is_fake_dead(ctx, on_stairs);
        ZombieStateFlags::int_from_zombie(&ZombieStateSource {
            crawling,
            can_walk,
            fake_dead,
            can_crawl_under_vehicle: self.zombie.can_crawl_under_vehicle,
            reanimated_for_grapple_only: self.zombie.reanimated_for_grapple_only,
        })
    }

    /// `IsoZombie.save(output, isDebugSave)`: the serialise and class bytes,
    /// `IsoGameCharacter.save`, then the zombie's part. `on_stairs` is
    /// `getSquare() != null && getSquare().HasStairs()` (see
    /// [`Self::zombie_state_flags`]).
    pub fn save_zombie(
        &mut self,
        sm: &ScriptManager,
        chars: &Characters,
        ctx: &dyn SimContext,
        env: &SaveEnv<'_>,
        on_stairs: bool,
        w: &mut Writer,
    ) -> ScriptResult {
        self.save_game_character(sm, chars, ctx, env, w)?;
        w.i32(REANIMATED_FILE_VERSION);
        w.i32(self.vars.time_since_seen_flesh as i32);
        let state = self.zombie_state_flags(ctx, on_stairs);
        w.i32(state);
        self.save_worn_items(w)
    }

    /// `IsoZombie.load(input, worldVersion, isDebugSave)` into a zombie
    /// made by `new IsoZombie(cell)` ([`GameCharacter::new_zombie`] with no
    /// descriptor), after `IsoObject.factoryFromFileInput` read the
    /// serialise and class bytes. In the original's `Rand` order:
    /// `IsoGameCharacter.load` (the loaded descriptor replaces the
    /// constructor's), `InitSpritePartsZombie`'s `zombieId`, the three tint
    /// draws, `DoZombieStats`, then the saved state (`setFakeDead(true)`'s
    /// draw, or a crawler's `DoZombieStats`).
    ///
    /// The default state is `ZombieIdleState` and the state machine is
    /// unlocked; the caller then calls `setDefaultState()` (it needs the
    /// world) and adds the zombie to its list.
    #[allow(clippy::too_many_arguments)]
    pub fn load_zombie(
        &mut self,
        sm: &ScriptManager,
        chars: &mut Characters,
        ctx: &dyn SimContext,
        env: &SaveEnv<'_>,
        r: &mut Reader<'_>,
        world_version: i32,
        set_on_fire: &mut dyn FnMut(&mut Self),
    ) -> ScriptResult {
        self.load_game_character(sm, chars, ctx, env, r, world_version, set_on_fire)?;
        let z = &mut self.zombie;
        z.walk_variant = "ZombieWalk".into();
        z.sprite_name = "BobZ".into();
        if z.palette != 1 {
            z.sprite_name = format!("{}{}", z.sprite_name, z.palette);
        }
        // `desc.isFemale()`: a missing descriptor is a NullPointerException.
        let female = self
            .descriptor
            .as_ref()
            .ok_or_else(crate::npe)?
            .borrow()
            .is_female();
        self.zombie.female = female;
        if female {
            self.zombie.sprite_name = if self.zombie.palette == 1 {
                "KateZ".into()
            } else {
                format!("KateZ{}", self.zombie.palette)
            };
        }
        self.zombie.hurt_sound = format!("{}Hurt", self.voice_prefix());
        // InitSpritePartsZombie(desc).
        self.zombie.zombie_id = ctx.rand_next(10000);
        // The sprite's tints.
        for _ in 0..3 {
            ctx.rand_next(5);
        }
        self.set_default_state_to(Some(idle()));
        self.zombie.speed_type = -1;
        self.zombie.inactive = ctx.is_zombie_inactivity_phase();
        self.do_zombie_stats(ctx);
        let loaded_file_version = e(r.i32())?;
        self.zombie.width = 0.3;
        self.vars.time_since_seen_flesh = e(r.i32())? as f32;
        let state_flags = e(r.i32())?;
        if loaded_file_version == 0 {
            self.set_fake_dead(ctx, state_flags == 1);
        } else if loaded_file_version >= 1 {
            let state = ZombieStateFlags::from_int(state_flags);
            if state.is_fake_dead() {
                self.set_fake_dead(ctx, true);
            } else if state.is_crawling() {
                self.pop_make_crawler(ctx, state.is_can_walk());
            }
            if state.is_initialized() {
                self.zombie.can_crawl_under_vehicle = state.is_can_crawl_under_vehicle();
            }
            self.zombie.reanimated_for_grapple_only = state.is_reanimated_for_grapple_only();
        }
        self.load_worn_items(sm, chars, r)?;
        self.set_state_machine_locked(false);
        Ok(())
    }
}

/// The zombies `ReanimatedPlayers.saveReanimatedPlayers` writes, as
/// `IsoZombie.save` bytes for `reanimated.bin`: every zombie the manager
/// tracks (`tracked`), then those of the cell's zombie list
/// (`cell_zombies`) that are reanimated players, not dead and not tracked.
/// `on_stairs` answers `getSquare().HasStairs()` for a zombie.
///
/// An error is the exception the original logs: `reanimated.bin` is then
/// not written.
#[allow(clippy::too_many_arguments)]
pub fn reanimated_players_to_save<I: ItemIdentity + 'static>(
    sm: &ScriptManager,
    chars: &Characters,
    ctx: &dyn SimContext,
    env: &SaveEnv<'_>,
    tracked: &mut [GameCharacter<I>],
    cell_zombies: &mut [GameCharacter<I>],
    on_stairs: &dyn Fn(&GameCharacter<I>) -> bool,
) -> ScriptResult<Vec<Vec<u8>>> {
    let mut out = Vec::new();
    for z in tracked.iter_mut() {
        let stairs = on_stairs(z);
        let mut w = Writer::new();
        z.save_zombie(sm, chars, ctx, env, stairs, &mut w)?;
        out.push(w.buf);
    }
    let tracked_ids: Vec<i32> = tracked.iter().map(|z| z.id).collect();
    for z in cell_zombies.iter_mut() {
        if !z.zombie.reanimated_player || z.is_dead() || tracked_ids.contains(&z.id) {
            continue;
        }
        let stairs = on_stairs(z);
        let mut w = Writer::new();
        z.save_zombie(sm, chars, ctx, env, stairs, &mut w)?;
        out.push(w.buf);
    }
    Ok(out)
}

/// `ReanimatedPlayers.loadReanimatedPlayers(in)` over the bytes of
/// `reanimated.bin` (its world version, the count, then the records): for each zombie,
/// `IsoObject.factoryFromFileInput` (`new_zombie` is `new IsoZombie(cell)`;
/// any other class is "expected IsoZombie here"), [`GameCharacter::load_zombie`],
/// the descriptor id set to 0 and `setReanimatedPlayer(true)`.
///
/// `out` is cleared first (`zombies.clear()`); on an error (logged by the
/// original) it keeps the zombies loaded before it.
#[allow(clippy::too_many_arguments)]
pub fn load_reanimated_players<I: ItemIdentity + 'static>(
    sm: &ScriptManager,
    chars: &mut Characters,
    ctx: &dyn SimContext,
    env: &SaveEnv<'_>,
    data: &[u8],
    new_zombie: &mut dyn FnMut(&mut Characters) -> ScriptResult<GameCharacter<I>>,
    set_on_fire: &mut dyn FnMut(&mut GameCharacter<I>),
    out: &mut Vec<GameCharacter<I>>,
) -> ScriptResult {
    out.clear();
    let mut r = Reader::new(data);
    let world_version = e(r.i32())?;
    let count = e(r.i32())?;
    for _ in 0..count {
        let serialise = e(r.i8())? != 0;
        let class = if serialise { Some(e(r.i8())?) } else { None };
        if class.and_then(|c| env.tables.class_name(c)) != Some("Zombie") {
            return Err(ScriptError::new(
                "java.lang.RuntimeException: expected IsoZombie here",
            ));
        }
        let mut z = new_zombie(chars)?;
        z.load_zombie(sm, chars, ctx, env, &mut r, world_version, set_on_fire)?;
        z.descriptor
            .as_ref()
            .ok_or_else(crate::npe)?
            .borrow_mut()
            .id = 0;
        z.zombie.reanimated_player = true;
        out.push(z);
    }
    Ok(())
}
