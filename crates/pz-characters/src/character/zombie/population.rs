//! The zombie side of `pz_popman::PopulationHost`: `VirtualZombieManager.
//! createRealZombieAlways` for a new zombie (`createZombieOutsideWorld`
//! without the reuse pool) and the `IsoZombie` setters the population
//! manager calls. The world's adapter owns the zombie list, the squares
//! and `ZombiePopulationManager.registerZombie`.

use pz_scripts::ScriptManager;
use pz_world::square::IsoDirections;

use pz_world_popman::{ChunkZombie, RealZombieInfo, ZombieStateSource};

use super::{GameCharacter, ZombieSpawn};
use crate::survivor::SurvivorDescRef;
use crate::character::state::{StateCx, StateWorld};
use crate::character::states::ids;
use crate::character::{SimContext, SimEvent};

/// What `createRealZombieAlways` made.
pub struct RealZombie<I: 'static> {
    pub zombie: GameCharacter<I>,
    /// The chosen square.
    pub square: (i32, i32, i32),
    /// `trySpawnBuildingKey` rolled under the key chance
    /// (`checkAndSpawnZombieForBuildingKey` is the world's).
    pub spawn_building_key: bool,
}

/// `VirtualZombieManager.createRealZombieAlways(dir, bDead, outfitID,
/// persistentId)` with a new `IsoZombie` (`zombie.persistentId` stays 0:
/// `ZombiePopulationManager.registerZombie` is the caller's). `water`
/// answers `isWaterSquare()`. `Err` is the exception the constructor threw
/// (`SurvivorFactory.CreateSurvivor()` with empty name lists).
#[allow(clippy::too_many_arguments)]
pub fn create_real_zombie_always<I: 'static>(
    sm: &ScriptManager,
    ctx: &dyn SimContext,
    choices: &[(i32, i32, i32)],
    water: &dyn Fn((i32, i32, i32)) -> bool,
    dir: IsoDirections,
    dead: bool,
    outfit_id: i32,
    key_spawn_chance_d100: f32,
    spawn: ZombieSpawn<'_, I>,
) -> pz_scripts::ScriptResult<Option<RealZombie<I>>> {
    if choices.is_empty() {
        return Ok(None);
    }
    let choice = choices[ctx.rand_next(choices.len() as i32) as usize];
    if water(choice) {
        return Ok(None);
    }
    let mut z = GameCharacter::new_zombie(sm, ctx, spawn)?;
    z.zombie.dress_in_random_outfit = outfit_id == 0;
    z.zombie.persistent_outfit_id = outfit_id;
    z.zombie.persistent_id = 0;
    z.set_forward_iso_direction(dir);
    if dead {
        z.zombie.dress_in_random_outfit = true;
    }
    z.vars.target = None;
    z.vars.time_since_seen_flesh = 100000.0;
    if !z.is_fake_dead(ctx, false) {
        match ctx.lore_toughness() {
            1 => z.health = 3.5 + ctx.rand_next_float(0.0, 0.3),
            2 => z.health = 1.5 + ctx.rand_next_float(0.0, 0.3),
            3 => z.health = 0.5 + ctx.rand_next_float(0.0, 0.3),
            4 => z.health = ctx.rand_next_float(0.5, 3.5) + ctx.rand_next_float(0.0, 0.3),
            _ => {}
        }
    } else {
        z.health = 0.5 + ctx.rand_next_float(0.0, 0.3);
    }
    let spec_x = ctx.rand_next_range(0, 1000) as f32 / 1000.0;
    let spec_y = ctx.rand_next_range(0, 1000) as f32 / 1000.0;
    z.path.current_square = Some(choice);
    z.vars.x = spec_x + choice.0 as f32;
    z.vars.y = spec_y + choice.1 as f32;
    z.vars.z = choice.2 as f32;
    z.vars.last_x = z.vars.x;
    z.vars.last_y = z.vars.y;
    z.vars.next_x = z.vars.x;
    z.vars.next_y = z.vars.y;
    if dead {
        let d = IsoDirections::ALL[ctx.rand_next(8) as usize];
        z.set_forward_iso_direction(d);
        z.set_fake_dead(ctx, false);
        z.health = 0.0;
        return Ok(Some(RealZombie { zombie: z, square: choice, spawn_building_key: false }));
    }
    if ctx.is_client() {
        z.zombie.remote = true;
    }
    let roll = ctx.rand_next(100).min(ctx.rand_next(100));
    let key = (roll as f32) < key_spawn_chance_d100;
    Ok(Some(RealZombie { zombie: z, square: choice, spawn_building_key: key }))
}

impl<I: 'static> GameCharacter<I> {
    /// `setX(x); setY(y)`.
    pub fn pop_set_position(&mut self, x: f32, y: f32) {
        self.vars.x = x;
        self.vars.y = y;
        self.vars.last_x = x;
        self.vars.last_y = y;
        self.vars.next_x = x;
        self.vars.next_y = y;
    }

    /// `sitAgainstWall`'s effect: position, `setSitAgainstWall(true)` and
    /// the direction away from the wall.
    pub fn pop_sit_against_wall(&mut self, x: f32, y: f32, dir: Option<IsoDirections>) {
        self.pop_set_position(x, y);
        self.vars.sit_against_wall = true;
        if let Some(d) = dir {
            self.set_forward_iso_direction(d);
        }
    }

    /// `setHealth(health); setFakeDead(true)`.
    pub fn pop_make_fake_dead(&mut self, ctx: &dyn SimContext, health: f32) {
        self.health = health;
        self.set_fake_dead(ctx, true);
    }

    /// `setCrawler(true); setCanWalk(canWalk); setOnFloor(true);
    /// setFallOnFront(true); walkVariant = "ZombieWalk"; DoZombieStats()`.
    pub fn pop_make_crawler(&mut self, ctx: &dyn SimContext, can_walk: bool) {
        self.vars.crawling = true;
        self.zombie.can_walk = can_walk;
        self.vars.on_floor = true;
        self.environment.fall_on_front = true;
        self.zombie.walk_variant = "ZombieWalk".into();
        self.do_zombie_stats(ctx);
    }

    /// `allowRepathDelay = -1; pathToLocation(x, y, 0)`.
    pub fn pop_path_to(&mut self, x: i32, y: i32, cx: &mut StateCx<'_, I>) {
        self.zombie.allow_repath_delay = -1.0;
        self.zombie_path_to_location(x, y, 0, cx);
    }

    /// `die()`.
    pub fn pop_die(&mut self, cx: &mut StateCx<'_, I>) {
        if !self.vars.on_kill_done {
            let k = self.vars.attacked_by;
            self.kill_by(k, cx);
        }
        cx.ctx.event(SimEvent::Die);
    }

    /// `removeFromWorld()`'s zombie part: aggro, path and eating cleared
    /// (`VirtualZombieManager.RemoveZombie` and the square are the
    /// world's).
    pub fn zombie_remove_from_world(&mut self, cx: &mut StateCx<'_, I>) {
        self.clear_aggro_list();
        self.clear_path2();
        cx.world.path_cancel_request(self);
        use pz_pathfind::mover::PathFindProgress;
        if !matches!(self.path.progress, PathFindProgress::NotRunning | PathFindProgress::Found) {
            self.path.progress = PathFindProgress::NotRunning;
        }
        self.set_eat_body_target(None, false, cx);
    }
}

/// `z` as `ZombiePopulationManager` reads a real zombie
/// (`registerZombie`, `packRealZombies`, `removeChunkFromWorld`):
/// position, `getForwardIsoDirection()`, `getPersistentOutfitID()`,
/// `ZombieStateFlags.intFromZombie`'s flags, `getPathTargetX/Y()` from the
/// world's path behaviour, `isDead()`, `isReanimatedPlayer()` and whether
/// the current state is `WalkTowardState` or `PathFindState`.
///
/// `isFakeDead()` is read without its stairs test (a fake-dead zombie is
/// never left on stairs: `setFakeDead` refuses there).
pub fn real_zombie_info<I: 'static>(z: &GameCharacter<I>, ctx: &dyn SimContext, world: &dyn StateWorld<I>) -> RealZombieInfo<i32> {
    let (px, py) = world
        .path_behavior(z.id)
        .map_or((0, 0), |b| (pz_world::java::fastfloor(b.target_x()), pz_world::java::fastfloor(b.target_y())));
    let state = z.current_state_id();
    RealZombieInfo {
        handle: z.id,
        persistent_id: z.zombie.persistent_id,
        x: z.vars.x,
        y: z.vars.y,
        z: z.vars.z,
        dir: z.forward_iso_direction(),
        outfit: z.zombie.persistent_outfit_id,
        state: ZombieStateSource {
            crawling: z.vars.crawling,
            can_walk: z.zombie.can_walk,
            fake_dead: z.zombie.fake_dead && ctx.lore_disable_fake_dead() != 3,
            can_crawl_under_vehicle: z.zombie.can_crawl_under_vehicle,
            reanimated_for_grapple_only: z.zombie.reanimated_for_grapple_only,
        },
        path_target_x: px,
        path_target_y: py,
        dead: z.is_dead(),
        reanimated_player: z.zombie.reanimated_player,
        walking: state == Some(ids::WALK_TOWARD) || state == Some(ids::PATH_FIND),
    }
}

/// `pz_world_popman::ZombieOps` over real zombies: the world's zombie list
/// and what creating, pathing and killing them needs. The zombie handle is
/// the character id.
pub struct RealZombies<'a, I: 'static> {
    pub sm: &'a ScriptManager,
    pub chars: &'a crate::Characters,
    pub ctx: &'a dyn SimContext,
    /// The rest of the world (path finding, corpses).
    pub world: &'a mut dyn crate::character::state::StateWorld<I>,
    /// The real zombies (`IsoCell.getZombieList()`).
    pub zombies: &'a mut Vec<GameCharacter<I>>,
    /// The next `IsoMovingObject` id.
    pub next_id: &'a mut i32,
    /// `SurvivorFactory.CreateSurvivor()` (`Err`: the exception it threw).
    pub create_survivor: &'a mut dyn FnMut() -> pz_scripts::ScriptResult<SurvivorDescRef<I>>,
    /// `isWaterSquare()`.
    pub water: &'a dyn Fn((i32, i32, i32)) -> bool,
    /// `getKeySpawnChanceD100()`.
    pub key_spawn_chance_d100: f32,
    /// Zombies whose `trySpawnBuildingKey` roll passed (the world spawns
    /// the key).
    pub building_keys: Vec<i32>,
    /// Zombies `removeFromWorld` took out.
    pub removed: Vec<GameCharacter<I>>,
}

impl<I: 'static> RealZombies<'_, I> {
    fn find(&mut self, id: i32) -> Option<usize> {
        self.zombies.iter().position(|z| z.id == id)
    }

    fn with<R>(&mut self, id: i32, f: impl FnOnce(&mut GameCharacter<I>, &mut StateCx<'_, I>) -> R) -> Option<R> {
        let i = self.find(id)?;
        let mut z = self.zombies.remove(i);
        let r = {
            let mut cx = StateCx::new(self.sm, self.chars, self.ctx, &mut *self.world);
            f(&mut z, &mut cx)
        };
        self.zombies.insert(i, z);
        Some(r)
    }
}

impl<I: 'static> RealZombies<'_, I> {
    /// The zombie list as `updateRealZombies` packs it.
    pub fn infos(&self) -> Vec<RealZombieInfo<i32>> {
        self.zombies.iter().map(|z| real_zombie_info(z, self.ctx, &*self.world)).collect()
    }

    /// The real zombies standing in chunk `(wx, wy)`, in
    /// `removeChunkFromWorld`'s square order (level, then `y`, then `x`;
    /// list order on one square). `in_room` answers `sq.getRoom() != null`
    /// for a square.
    pub fn chunk_zombies(&self, wx: i32, wy: i32, in_room: &dyn Fn((i32, i32, i32)) -> bool) -> Vec<ChunkZombie<i32>> {
        let mut v: Vec<((i32, i32, i32), usize)> = Vec::new();
        for (i, z) in self.zombies.iter().enumerate() {
            let sq = z.path.current_square.unwrap_or((
                pz_world::java::fastfloor(z.vars.x),
                pz_world::java::fastfloor(z.vars.y),
                pz_world::java::fastfloor(z.vars.z),
            ));
            if sq.0.div_euclid(8) == wx && sq.1.div_euclid(8) == wy {
                v.push((sq, i));
            }
        }
        v.sort_by_key(|&((x, y, l), i)| (l, y.rem_euclid(8), x.rem_euclid(8), i));
        v.into_iter()
            .map(|(sq, i)| ChunkZombie {
                info: real_zombie_info(&self.zombies[i], self.ctx, &*self.world),
                level: sq.2,
                in_room: in_room(sq),
            })
            .collect()
    }
}

impl<I: 'static> pz_world_popman::ZombieOps for RealZombies<'_, I> {
    type Zombie = i32;

    /// `createRealZombieAlways(outfit, dir, false, persistentId)` with
    /// `choices = [square]` (the outfit id is the outfit system's; 0 dresses
    /// in a random outfit). `persistentId` goes to `registerZombie`, which
    /// the population manager calls next ([`Self::set_persistent_id`]).
    /// `Err` is the Java exception the constructor threw.
    fn create_real_zombie(&mut self, outfit: i32, dir: IsoDirections, _persistent_id: i32, square: (i32, i32, i32)) -> Result<Option<i32>, String> {
        let spawn = ZombieSpawn { descriptor: None, create_survivor: &mut *self.create_survivor };
        let r = create_real_zombie_always(self.sm, self.ctx, &[square], self.water, dir, false, outfit, self.key_spawn_chance_d100, spawn)
            .map_err(|e| e.to_string())?;
        let Some(r) = r else {
            return Ok(None);
        };
        let mut z = r.zombie;
        *self.next_id += 1;
        z.id = *self.next_id;
        if r.spawn_building_key {
            self.building_keys.push(z.id);
        }
        let id = z.id;
        self.zombies.push(z);
        Ok(Some(id))
    }
    fn real_zombie_info(&self, z: i32) -> Option<RealZombieInfo<i32>> {
        let z = self.zombies.iter().find(|c| c.id == z)?;
        Some(real_zombie_info(z, self.ctx, &*self.world))
    }
    fn set_persistent_id(&mut self, z: i32, id: i32) {
        if let Some(i) = self.find(z) {
            self.zombies[i].zombie.persistent_id = id;
        }
    }
    fn set_position(&mut self, z: i32, x: f32, y: f32) {
        if let Some(i) = self.find(z) {
            self.zombies[i].pop_set_position(x, y);
        }
    }
    fn sit_against_wall(&mut self, z: i32, x: f32, y: f32, dir: Option<IsoDirections>) {
        if let Some(i) = self.find(z) {
            self.zombies[i].pop_sit_against_wall(x, y, dir);
        }
    }
    fn make_fake_dead(&mut self, z: i32, health: f32) {
        let ctx = self.ctx;
        if let Some(i) = self.find(z) {
            self.zombies[i].pop_make_fake_dead(ctx, health);
        }
    }
    fn make_crawler(&mut self, z: i32, can_walk: bool) {
        let ctx = self.ctx;
        if let Some(i) = self.find(z) {
            self.zombies[i].pop_make_crawler(ctx, can_walk);
        }
    }
    fn set_can_crawl_under_vehicle(&mut self, z: i32, v: bool) {
        if let Some(i) = self.find(z) {
            self.zombies[i].zombie.can_crawl_under_vehicle = v;
        }
    }
    fn set_reanimated_for_grapple_only(&mut self, z: i32, v: bool) {
        if let Some(i) = self.find(z) {
            self.zombies[i].zombie.reanimated_for_grapple_only = v;
        }
    }
    fn path_to(&mut self, z: i32, x: i32, y: i32) {
        self.with(z, |z, cx| z.pop_path_to(x, y, cx));
    }
    fn die(&mut self, z: i32) {
        self.with(z, |z, cx| z.pop_die(cx));
    }
    fn remove_zombie(&mut self, z: i32) {
        self.with(z, |z, cx| z.zombie_remove_from_world(cx));
        if let Some(i) = self.find(z) {
            let z = self.zombies.remove(i);
            self.removed.push(z);
        }
    }
}

#[cfg(test)]
#[path = "population_tests.rs"]
mod tests;
