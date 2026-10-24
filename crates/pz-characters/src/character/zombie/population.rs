//! The zombie side of `pz_popman::PopulationHost`: `VirtualZombieManager.
//! createRealZombieAlways` for a new zombie (`createZombieOutsideWorld`
//! without the reuse pool) and the `IsoZombie` setters the population
//! manager calls. The world's adapter owns the zombie list, the squares
//! and `ZombiePopulationManager.registerZombie`.

use pz_scripts::ScriptManager;
use pz_world::square::IsoDirections;

use super::{GameCharacter, ZombieSpawn};
use crate::character::state::StateCx;
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
/// answers `isWaterSquare()`.
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
) -> Option<RealZombie<I>> {
    if choices.is_empty() {
        return None;
    }
    let choice = choices[ctx.rand_next(choices.len() as i32) as usize];
    if water(choice) {
        return None;
    }
    let mut z = GameCharacter::new_zombie(sm, ctx, spawn);
    z.zombie.dress_in_random_outfit = outfit_id == 0;
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
    z.zombie.next_x = z.vars.x;
    z.zombie.next_y = z.vars.y;
    if dead {
        let d = IsoDirections::ALL[ctx.rand_next(8) as usize];
        z.set_forward_iso_direction(d);
        z.set_fake_dead(ctx, false);
        z.health = 0.0;
        return Some(RealZombie { zombie: z, square: choice, spawn_building_key: false });
    }
    if ctx.is_client() {
        z.zombie.remote = true;
    }
    let roll = ctx.rand_next(100).min(ctx.rand_next(100));
    let key = (roll as f32) < key_spawn_chance_d100;
    Some(RealZombie { zombie: z, square: choice, spawn_building_key: key })
}

impl<I: 'static> GameCharacter<I> {
    /// `setX(x); setY(y)`.
    pub fn pop_set_position(&mut self, x: f32, y: f32) {
        self.vars.x = x;
        self.vars.y = y;
        self.vars.last_x = x;
        self.vars.last_y = y;
        self.zombie.next_x = x;
        self.zombie.next_y = y;
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
        cx.world.path_cancel(self);
        self.set_eat_body_target(None, false, cx);
    }
}
