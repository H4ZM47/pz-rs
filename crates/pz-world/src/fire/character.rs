//! Fire and characters: `IsoGameCharacter.FireCheck`, `SetOnFire`,
//! `StopBurning`, `ReduceHealthWhenBurning` and `SpreadFire`, over a
//! [`FireCharacter`] the characters code implements (it owns
//! `IsoGameCharacter`, body damage, animations and sounds).
//!
//! The character update calls [`fire_check`], [`spread_fire`] and
//! [`reduce_health_when_burning`] in that order (`IsoGameCharacter.update`).

use crate::fire::manager::{CharacterId, FireWorld, IsoFireManager};
use crate::java::GameRand;

/// What kind of character it is, as the fire code tells them apart.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CharacterKind {
    /// `IsoPlayer` (not an animal).
    Player,
    /// `isZombie()`.
    Zombie,
    /// `IsoAnimal`.
    Animal,
    /// Any other `IsoGameCharacter`.
    Other,
}

/// The parts of `IsoGameCharacter` fire reads and acts on.
#[allow(unused_variables)]
pub trait FireCharacter {
    fn id(&self) -> CharacterId;
    fn kind(&self) -> CharacterKind;
    /// The square it stands on (`square`), if any.
    fn square(&self) -> Option<(i32, i32, i32)>;
    /// `onFire`.
    fn on_fire(&self) -> bool;
    /// `setOnFire(b)`'s field.
    fn set_on_fire_flag(&mut self, on: bool);
    /// `getVehicle() != null`.
    fn in_vehicle(&self) -> bool {
        false
    }
    /// `VirtualZombieManager.instance.isReused(zombie)`.
    fn is_reused_zombie(&self) -> bool {
        false
    }
    /// `fireKillRate` (0.0038).
    fn fire_kill_rate(&self) -> f32 {
        0.0038
    }
    /// `fireSpreadProbability` (6).
    fn fire_spread_probability(&self) -> i32 {
        6
    }
    fn is_god_mode(&self) -> bool {
        false
    }
    fn is_dead(&self) -> bool;
    /// `IsoPlayer.IsRunning()`.
    fn is_running(&self) -> bool {
        false
    }
    /// `AttachAnim("Fire", "01", ...)` with the random parts of its offsets
    /// (`8 - Rand.Next(16)` and `10 + Rand.Next(20)`, already drawn), the
    /// `BurningFlesh` sound and, for players, the burnt body part
    /// (`part_index`, from `Rand.Next(Hand_L, MAX)`).
    fn attach_fire(&mut self, dx: i32, dy: i32, part_index: Option<i32>);
    /// `BodyPartType.ToIndex(Hand_L)` and `ToIndex(MAX)`.
    fn burn_part_range(&self) -> (i32, i32);
    /// `StopBurning`'s rest: the attached anims cleared, the sound stopped.
    fn detach_fire(&mut self) {}
    /// `CombatManager.applyDamage(this, damage)`.
    fn apply_damage(&mut self, damage: f32);
    /// `setAttackedBy(null)`.
    fn clear_attacked_by(&mut self) {}
    /// A player's `getBodyDamage().ReduceGeneralHealth(dpm)`, the
    /// `OnPlayerGetDamage(FIRE)` event and `getBodyDamage().OnFire(true)`.
    fn burn_player(&mut self, dpm: f32);
    /// `forceAwake()`.
    fn force_awake(&mut self) {}
    /// `OnZombieDead`.
    fn zombie_died(&mut self) {}
}

/// Frame values the character fire code reads.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CharacterFireFrame {
    /// `GameTime.getMultiplier()`.
    pub multiplier: f32,
    /// `getThirtyFPSMultiplier()`.
    pub thirty_fps: f32,
    /// `PerformanceSettings.getLockFPS()`.
    pub lock_fps: i32,
}

impl CharacterFireFrame {
    /// `Rand.AdjustForFramerate(chance)` (single player).
    pub fn adjust_for_framerate(&self, chance: i32) -> i32 {
        (chance as f32 * (self.lock_fps as f32 / 30.0)) as i32
    }
}

/// `SetOnFire()`.
pub fn set_on_fire(mgr: &mut IsoFireManager, c: &mut dyn FireCharacter, rand: &mut dyn GameRand) {
    if c.on_fire() {
        return;
    }
    // setOnFire(true): charactersOnFire is a server's list.
    c.set_on_fire_flag(true);
    let dx = 8 - rand.next(16);
    let dy = 10 + rand.next(20);
    mgr.add_burning_character(c.id());
    let (lo, hi) = c.burn_part_range();
    let part = rand.next_range(lo, hi);
    // `this instanceof IsoPlayer` (animals are players too).
    let part = matches!(c.kind(), CharacterKind::Player | CharacterKind::Animal).then_some(part);
    c.attach_fire(dx, dy, part);
}

/// `StopBurning()`.
pub fn stop_burning(mgr: &mut IsoFireManager, c: &mut dyn FireCharacter) {
    if !c.on_fire() {
        return;
    }
    mgr.remove_burning_character(c.id());
    c.set_on_fire_flag(false);
    c.detach_fire();
}

fn died(mgr: &mut IsoFireManager, c: &mut dyn FireCharacter) {
    if c.is_dead() {
        mgr.remove_burning_character(c.id());
        if c.kind() == CharacterKind::Zombie {
            c.zombie_died();
        }
    }
}

/// `FireCheck()`: a character on a burning square catches fire (zombies
/// and animals always, players one frame in `AdjustForFramerate(70)`) or
/// takes the fire's damage.
pub fn fire_check(
    mgr: &mut IsoFireManager,
    w: &FireWorld<'_>,
    c: &mut dyn FireCharacter,
    frame: CharacterFireFrame,
    rand: &mut dyn GameRand,
) {
    if c.on_fire() || c.is_reused_zombie() || c.in_vehicle() {
        return;
    }
    let burning = c
        .square()
        .and_then(|(x, y, z)| w.sq(x, y, z))
        .is_some_and(|sq| sq.has(w.flags.burning));
    if !burning {
        return;
    }
    let kind = c.kind();
    // `this instanceof IsoPlayer && Rand.Next(...) == 0 || isZombie() ||
    // this instanceof IsoAnimal`: an animal is an IsoPlayer, so it draws
    // too.
    let catches = match kind {
        CharacterKind::Player => rand.next(frame.adjust_for_framerate(70)) == 0,
        CharacterKind::Animal => {
            rand.next(frame.adjust_for_framerate(70));
            true
        }
        CharacterKind::Zombie => true,
        CharacterKind::Other => false,
    };
    if catches {
        set_on_fire(mgr, c, rand);
        return;
    }
    if kind != CharacterKind::Player {
        c.apply_damage(c.fire_kill_rate() * frame.multiplier / 2.0);
        c.clear_attacked_by();
    } else {
        c.burn_player(c.fire_kill_rate() * frame.thirty_fps * 60.0 / 2.0);
        c.force_awake();
    }
    died(mgr, c);
}

/// `ReduceHealthWhenBurning()`.
pub fn reduce_health_when_burning(
    mgr: &mut IsoFireManager,
    c: &mut dyn FireCharacter,
    frame: CharacterFireFrame,
    rand: &mut dyn GameRand,
) {
    if !c.on_fire() {
        return;
    }
    if c.is_god_mode() {
        stop_burning(mgr, c);
        return;
    }
    let kind = c.kind();
    if !c.is_dead() {
        let k = c.fire_kill_rate();
        match kind {
            CharacterKind::Zombie => {
                c.apply_damage(k / 20.0 * frame.multiplier);
                c.clear_attacked_by();
            }
            CharacterKind::Player => c.burn_player(k * frame.thirty_fps * 60.0),
            CharacterKind::Animal => {
                c.apply_damage(k * frame.multiplier - k / 10.0 * frame.multiplier)
            }
            CharacterKind::Other => c.apply_damage(k * frame.multiplier),
        }
        died(mgr, c);
    }
    if kind == CharacterKind::Player {
        let chance = frame.adjust_for_framerate(if c.is_running() { 150 } else { 400 });
        if rand.next(chance) == 0 {
            stop_burning(mgr, c);
        }
    }
}

/// `SpreadFire()`: a burning character sets its square alight, one frame
/// in `AdjustForFramerate(3000)` per point of `fireSpreadProbability`.
pub fn spread_fire(
    mgr: &mut IsoFireManager,
    w: &mut FireWorld<'_>,
    c: &dyn FireCharacter,
    frame: CharacterFireFrame,
    rand: &mut dyn GameRand,
) {
    if !c.on_fire() || !w.settings.fire_spread {
        return;
    }
    let Some(at) = c.square() else { return };
    let Some(sq) = w.sq(at.0, at.1, at.2) else {
        return;
    };
    if sq.has(w.flags.burning) {
        return;
    }
    if rand.next(frame.adjust_for_framerate(3000)) < c.fire_spread_probability() {
        mgr.start_fire(w, rand, at.0, at.1, at.2, false, 80, 0);
    }
}
