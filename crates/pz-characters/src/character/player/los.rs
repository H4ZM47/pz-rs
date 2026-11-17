//! `IsoPlayer.updateLOS()` with `TestZombieSpotPlayer`, and
//! `IsoGameCharacter.updateSeenVisibility()` with `TestIfSeen` and
//! `getDetectionRange()`.
//!
//! Both reach the other characters through the world's character list: the
//! host lends the local players to it while they run.
//!
//! The alpha (`setAlphaAndTarget`, `setTargetAlpha`) is rendering.
//! `getHeadSquare()` is only set while a zombie climbs through a window and
//! is not ported, so `couldSeeHeadSquare` and `canSeeHeadSquare` are false.

use pz_scripts::{ScriptManager, ScriptResult};

use crate::character::state::StateCx;
use crate::character::states::ids;
use crate::character::zombie::math::{distance_manhatten, distance_to, distance_to_squared, fastfloor};
use crate::character::{Characters, GameCharacter, SimEvent};

/// `IsoMovingObject.DistToProper(other)` for two characters.
fn dist_to_proper(x: f32, y: f32, ox: f32, oy: f32) -> f32 {
    (distance_to_squared(x, y, ox, oy) as f64).sqrt() as f32
}

impl<I: 'static> GameCharacter<I> {
    /// `getDetectionRange()`.
    pub fn detection_range(&self, sm: &ScriptManager, chars: &Characters) -> ScriptResult<f32> {
        let mut f = if self.has_trait(sm, "DEAF")? { 2.0f32 } else { 3.5 };
        f += if self.has_trait(sm, "KEEN_HEARING")? { 3.0 } else { 0.0 };
        f -= if self.has_trait(sm, "HARD_OF_HEARING")? { 1.0 } else { 0.0 };
        let f2 = f.max(2.0) - 2.0;
        let mut f = 2.0 + f2 * self.worn_items_hearing_multiplier();
        f -= self.stat(chars, "FATIGUE")?;
        f -= self.stat(chars, "INTOXICATION")? * 0.01;
        Ok(f)
    }

    /// `updateSeenVisibility()`: `isVisibleToPlayer[i] = TestIfSeen(i,
    /// players[i])` for each local player in the world's character list.
    pub fn update_seen_visibility(&mut self, cx: &mut StateCx<'_, I>) -> ScriptResult {
        let mut players = Vec::new();
        for id in cx.world.characters_near(self.vars.x, self.vars.y, self.vars.z, f32::MAX) {
            if let Some(c) = cx.world.character_mut(id) {
                if c.is_player() && !c.is_animal() && (0..4).contains(&c.player.player_index) {
                    players.push((id, c.player.player_index));
                }
            }
        }
        for (id, index) in players {
            self.is_visible_to_player[index as usize] = self.test_if_seen(id, index, cx)?;
        }
        Ok(())
    }

    /// `TestIfSeen(playerIndex, player)` for the player `viewer` in the
    /// world's character list (never this character).
    fn test_if_seen(&self, viewer: i32, index: i32, cx: &mut StateCx<'_, I>) -> ScriptResult<bool> {
        let Some(my_sq) = self.current_square_xyz() else { return Ok(false) };
        let view_dist = cx.ctx.view_dist();
        let (sm, chars) = (cx.sm, cx.chars);
        let Some(p) = cx.world.character_mut(viewer) else { return Ok(false) };
        let p: &GameCharacter<I> = p;
        let dist = dist_to_proper(self.vars.x, self.vars.y, p.vars.x, p.vars.y);
        let range = p.detection_range(sm, chars)?;
        let p_sq = p.current_square_xyz();
        let mut dot = p.dot_with_forward_direction(self.vars.x, self.vars.y);
        if dist > view_dist {
            return Ok(false);
        }
        let could_see = cx.world.square_could_see(my_sq, index);
        let mut can_see = cx.world.square_can_see(my_sq, index);
        if !can_see && could_see {
            can_see = dist < range;
        }
        if !can_see {
            return Ok(false);
        }
        let mut light = cx.world.square_light(my_sq, index);
        if dist < range {
            if let Some(p_sq) = p_sq {
                light = light.max(cx.world.square_light(p_sq, index));
            }
        }
        if light > 0.6 {
            light = 1.0;
        }
        let mut dist_mod = 1.0 - dist / view_dist;
        if light == 1.0 && dist_mod > 0.3 {
            dist_mod = 1.0;
        }
        if dot < 0.5 {
            dot = 0.5;
        }
        light *= dot;
        if light < 0.0 {
            light = 0.0;
        }
        if dist <= 1.0 {
            dist_mod = 1.0;
            light *= 2.0;
        }
        light *= dist_mod;
        light *= 100.0;
        Ok(light > 0.025)
    }
}

/// What `updateLOS` counts while it walks the characters, written to the
/// player at the end (the zombies' `spotted` in between reach the player
/// only for other fields).
#[derive(Default)]
struct Counts {
    visible: i32,
    very_close: i32,
    seen_zombie: bool,
    seen_zombie_close: bool,
    music_visible: i32,
    distant_moving: i32,
    nearby_moving: i32,
    distant_not_moving: i32,
    nearby_not_moving: i32,
}

/// `IsoPlayer.updateLOS()` for the local player `id`, which the world's
/// character list holds while this runs, so that `TestZombieSpotPlayer`'s
/// `zombie.spotted(player)` reaches it. `IsoPlayer.update` runs it after
/// `updateInternal2`, before `IsoGameCharacter.update`.
pub fn update_player_los<I: 'static>(id: i32, cx: &mut StateCx<'_, I>) -> ScriptResult {
    let (sm, chars, ctx) = (cx.sm, cx.chars, cx.ctx);
    let Some(p) = cx.world.character_mut(id) else { return Ok(()) };
    p.player.spotted_list.clear();
    p.stats.num_visible_zombies = 0;
    let chasing = p.stats.num_chasing_zombies;
    p.stats.set_last_number_chasing_zombies(chasing);
    p.stats.num_chasing_zombies = 0;
    p.stats.music_zombies_targeting_distant_not_moving = 0;
    p.stats.music_zombies_targeting_nearby_not_moving = 0;
    p.stats.music_zombies_targeting_distant_moving = 0;
    p.stats.music_zombies_targeting_nearby_moving = 0;
    p.stats.music_zombies_visible = 0;
    p.num_survivors_in_vicinity = 0;
    let Some(p_sq) = p.current_square_xyz() else { return Ok(()) };
    let (server, client) = (ctx.is_server(), ctx.is_client());
    let index = p.player.player_index;
    let (px, py, pz) = (p.vars.x, p.vars.y, p.vars.z);
    let asleep = p.is_asleep();
    let ghost = p.is_ghost_mode(ctx);
    let range = p.detection_range(sm, chars)?;
    let last_spotted = p.player.last_spotted.clone();
    let mut closest = p.player.closest_zombie;
    let player_room = cx.world.square_room(p_sq);
    let mut n = Counts::default();
    // (id, is a zombie) in spottedList order.
    let mut spotted: Vec<(i32, bool)> = Vec::new();
    // getCell().getObjectList(): the characters (physics objects and
    // vehicles are skipped).
    for oid in cx.world.characters_near(px, py, pz, f32::MAX) {
        if oid == id {
            spotted.push((id, false));
            continue;
        }
        let Some((ox, oy, oz, o_sq)) =
            cx.world.character_mut(oid).map(|o| (o.vars.x, o.vars.y, o.vars.z, o.current_square_xyz()))
        else {
            continue;
        };
        let dist = distance_to(ox, oy, px, py);
        let Some(o_sq) = o_sq else { continue };
        // isSeeEveryone(): the debug option is off.
        let Some(o) = cx.world.character_mut(oid) else { continue };
        let zombie = o.is_zombie();
        if zombie && o.zombie.is_reanimated_for_grapple_only() {
            continue;
        }
        // A client's invisible characters: network.
        let could_see = cx.world.square_could_see(o_sq, index);
        // ServerOptions.hidePlayersBehindYou is true by default, so a
        // player is tested like the others.
        let can_see = cx.world.square_can_see(o_sq, index);
        if !asleep && (can_see || dist < range && could_see) {
            test_zombie_spot_player(id, (px, py), oid, &mut closest, cx)?;
            let on_stairs = cx.world.square(o_sq.0, o_sq.1, o_sq.2).is_some_and(|s| s.has_stairs);
            let o_room = cx.world.square_room(o_sq);
            let Some(o) = cx.world.character_mut(oid) else { continue };
            if !o.is_visible_to_player.get(index as usize).copied().unwrap_or(false) {
                continue;
            }
            // IsoSurvivor: none in the port.
            if zombie {
                n.seen_zombie = true;
                if oz >= pz - 1.0 && dist < 7.0 && !o.zombie.ghost && !o.is_fake_dead(ctx, on_stairs) && o_room == player_room {
                    n.seen_zombie_close = true;
                    n.visible += 1;
                }
                if dist < 3.0 {
                    n.very_close += 1;
                }
                // isSceneCulled(): the renderer's culling is not ported.
                n.music_visible += 1;
                if o.vars.target == Some(id) {
                    let moving = [ids::WALK_TOWARD, ids::LUNGE, ids::PATH_FIND].iter().any(|s| o.is_current_state_id(*s));
                    let far = dist_to_proper(px, py, ox, oy) >= 10.0;
                    match (moving, far) {
                        (true, true) => n.distant_moving += 1,
                        (true, false) => n.nearby_moving += 1,
                        (false, true) => n.distant_not_moving += 1,
                        (false, false) => n.nearby_not_moving += 1,
                    }
                }
            }
            spotted.push((oid, zombie));
            let near = if n.visible > 4 { 7.0 } else { 4.0 };
            let same_floor = fastfloor(oz) == fastfloor(pz);
            if dist < near && zombie && same_floor && !ghost && !client && !server {
                // GameTime.setMultiplier(1), then
                // SpeedControls.SetCurrentGameSpeed(1).
                ctx.event(SimEvent::SetGameSpeed(1));
            }
            if dist < near && zombie && same_floor && !last_spotted.contains(&oid) {
                n.visible += 2;
            }
        } else if could_see {
            test_zombie_spot_player(id, (px, py), oid, &mut closest, cx)?;
        }
    }
    let Some(p) = cx.world.character_mut(id) else { return Ok(()) };
    p.player.closest_zombie = closest;
    if n.seen_zombie {
        p.player.last_seen_zombie_time = 0.0;
    }
    if n.seen_zombie_close {
        p.player.ticks_since_seen_zombie = 0;
    }
    let s = &mut p.stats;
    s.num_visible_zombies = n.visible;
    s.music_zombies_visible = n.music_visible;
    s.music_zombies_targeting_distant_moving = n.distant_moving;
    s.music_zombies_targeting_nearby_moving = n.nearby_moving;
    s.music_zombies_targeting_distant_not_moving = n.distant_not_moving;
    s.music_zombies_targeting_nearby_not_moving = n.nearby_not_moving;
    let surprised = !server
        && !p.is_dead()
        && n.very_close > 0
        && p.stats.last_very_close_zombies == 0
        && n.visible > 0
        && p.stats.last_num_visible_zombies == 0
        && p.player.time_since_last_stab >= 600.0;
    if surprised {
        p.player.time_since_last_stab = 0.0;
        // The emitter's volume, Core.getOptionJumpScareVolume() / 10:
        // TODO(sound).
        ctx.event(SimEvent::PlaySound("ZombieSurprisedPlayer".into()));
    }
    if n.visible > 0 {
        p.player.time_since_last_stab = 0.0;
    }
    if p.player.time_since_last_stab < 600.0 {
        p.player.time_since_last_stab += ctx.thirty_fps_multiplier();
    }
    let mut zombies = 0;
    for &(s, zombie) in &spotted {
        if !p.player.last_spotted.contains(&s) {
            p.player.last_spotted.push(s);
        }
        if zombie {
            zombies += 1;
        }
    }
    p.player.spotted_list = spotted.into_iter().map(|(s, _)| s).collect();
    if p.player.clear_spotted_timer <= 0 && zombies == 0 {
        p.player.last_spotted.clear();
        p.player.clear_spotted_timer = 1000;
    } else {
        p.player.clear_spotted_timer -= 1;
    }
    p.stats.last_num_visible_zombies = n.visible;
    p.stats.last_very_close_zombies = n.very_close;
    Ok(())
}

/// `TestZombieSpotPlayer(o)` (not on a server): `o.spotted(player,
/// false)`, which only a zombie does something for, then the closest
/// standing zombie by `DistTo`.
fn test_zombie_spot_player<I: 'static>(
    player: i32,
    (px, py): (f32, f32),
    o: i32,
    closest: &mut f32,
    cx: &mut StateCx<'_, I>,
) -> ScriptResult {
    let Some(mut z) = cx.world.take_character(o) else { return Ok(()) };
    let spotted = if z.is_zombie() { z.spotted(player, false, cx).map(|_| ()) } else { Ok(()) };
    let (zombie, x, y, on_floor) = (z.is_zombie(), z.vars.x, z.vars.y, z.vars.on_floor);
    cx.world.put_character(z);
    spotted?;
    if zombie {
        let d = distance_manhatten(x, y, px, py);
        if d < *closest && !on_floor {
            *closest = d;
        }
    }
    Ok(())
}
