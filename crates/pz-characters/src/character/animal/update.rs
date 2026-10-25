//! `IsoAnimal.update()`: `updateInternal` (the animal logic), then
//! `IsoPlayer.update()`, whose `updateInternal1` for an animal is only
//! `IsoLivingCharacter.update()` (`IsoGameCharacter.update`) and
//! `AnimalInstanceManager.update`. None of the player's `updateInternal2`
//! (input, mechanics, breathing, temperature, weapons, sleeping pills,
//! torches, vehicle endurance) runs for an animal.

use pz_scripts::ScriptResult;

use super::{jerr, Spotted};
use crate::character::player::PlayerUpdate;
use crate::character::state::{StateCx, StateRegistry};
use crate::character::states::ids;
use crate::character::GameCharacter;

impl<I: 'static> GameCharacter<I> {
    /// `IsoAnimal.update()`.
    pub fn update_animal(
        &mut self,
        registry: &mut StateRegistry<I>,
        out: &mut PlayerUpdate,
        cx: &mut StateCx<'_, I>,
    ) -> ScriptResult {
        let Some(f) = self.animal.as_ref() else {
            return Ok(());
        };
        if f.animal.on_hook {
            self.animal_reattach_back_to_hook(cx);
            // ensureCorrectSkin (rendering).
            if cx.ctx.is_server() {
                if let Some(h) = cx.world.animal_host(self.id) {
                    h.world.instance_manager_update();
                }
            }
            return Ok(());
        }
        if self.variable_boolean("bPathfind") && !self.is_current_state_id(ids::ANIMAL_PATH_FIND) {
            let s = registry.get_or_opaque(ids::ANIMAL_PATH_FIND.0);
            self.change_state(Some(s), cx);
        }
        // updateEmitter: sound.
        if self.current_square_xyz().is_some() {
            self.do_deferred_movement(cx);
        }
        if self.is_dead() {
            return Ok(());
        }
        self.animal_update_internal(out, cx)?;
        let turn = self
            .animal
            .as_ref()
            .and_then(|f| f.def.as_ref())
            .map(|d| d.turn_delta);
        if cx.ctx.multiplier() > 10.0 {
            self.vars.turn_delta_normal = 1.0;
        } else if let Some(t) = turn {
            self.vars.turn_delta_normal = t;
        }
        Ok(())
    }

    /// `IsoAnimal.updateInternal()`.
    fn animal_update_internal(
        &mut self,
        out: &mut PlayerUpdate,
        cx: &mut StateCx<'_, I>,
    ) -> ScriptResult {
        let ctx = cx.ctx;
        {
            let Some(f) = self.animal.as_ref() else {
                return Ok(());
            };
            if f.animal.from_meta || !f.initialized {
                return Ok(());
            }
        }
        if ctx.is_client() {
            self.animal_update_width();
            cx.world.separate(self);
            self.animal_reattach_to_tree(cx);
            // BaseVehicle.updateNetworkHitByVehicle: network.
            return self.animal_iso_player_update(out, cx);
        }
        self.animal_behavior_update(cx);
        if let Some(mut host) = cx.world.animal_host(self.id) {
            let r = self.with_animal_env(ctx, &mut host, |a, c, env| a.data_update(c, env));
            if let Some(r) = r {
                r.map_err(jerr)?;
            }
        }
        if self.is_dead() {
            return Ok(());
        }
        self.animal_update_width();
        cx.world.separate(self);
        self.animal_check_tree_exists(cx);
        self.animal_reattach_to_tree(cx);
        if let Some(mut host) = cx.world.animal_host(self.id) {
            self.with_animal_env(ctx, &mut host, |a, c, env| a.check_zone(c, env));
        }
        self.animal_reattach_back_to_mom(cx);
        self.animal_respond_to_sound(cx);
        let m = ctx.multiplier();
        if let Some(f) = self.animal.as_mut() {
            let a = &mut f.animal;
            if a.pet_timer > 0.0 {
                a.pet_timer -= m;
                if a.pet_timer < 0.0 {
                    a.pet_timer = 0.0;
                }
            }
        }
        if let Some(mut host) = cx.world.animal_host(self.id) {
            let raining = host.world.is_raining();
            let speed = host.world.vehicle_speed_kmh();
            let open = host.world.under_open_sky();
            self.with_animal_env(ctx, &mut host, |a, c, env| {
                a.update_stress(c, env.ctx, &mut *env.rand, raining, speed, open)
            });
        }
        self.animal_update_lured(cx);
        // updateEmitter: sound.
        self.animal_try_thump(None, cx);
        self.animal_update_los(cx);
        if let Some(v) = self.animal.as_mut().and_then(|f| f.vehicle_for_collision_test.take()) {
            self.vars.vehicle_collision = self.animal_test_collide_with_vehicles(v as i16, cx);
        }
        self.animal_iso_player_update(out, cx)
    }

    /// `width = adef.collisionSize * getAnimalSize()`.
    fn animal_update_width(&mut self) {
        let size = self.animal_size();
        if let Some(f) = self.animal.as_mut() {
            if let Some(d) = &f.def {
                f.width = d.collision_size * size;
            }
        }
    }

    /// `behavior.update()` (`BaseAnimalBehavior`).
    pub fn animal_behavior_update(&mut self, cx: &mut StateCx<'_, I>) {
        self.behavior_update(cx);
    }

    /// `IsoAnimal.updateLOS()`: the cell's zombies (not those reanimated
    /// only to be grappled) and visible non-animal players on a square
    /// within one floor are `behavior.spotted(chr, false, distance)`, in
    /// order; [`super::AnimalFields::spotted`] keeps the calls.
    pub fn animal_update_los(&mut self, cx: &mut StateCx<'_, I>) {
        let (x, y, z) = (self.vars.x, self.vars.y, self.vars.z);
        let mut spotted = Vec::new();
        for id in cx.world.characters_near(x, y, z, f32::MAX) {
            if id == self.id {
                continue;
            }
            let Some(o) = cx.world.character_mut(id) else {
                continue;
            };
            if o.is_zombie() && o.zombie.reanimated_for_grapple_only {
                continue;
            }
            if (o.vars.z - z).abs() > 1.0 {
                continue;
            }
            let (dx, dy) = (o.vars.x - x, o.vars.y - y);
            let dist = ((dx * dx + dy * dy) as f64).sqrt() as f32;
            if o.current_square_xyz().is_none() {
                continue;
            }
            if o.is_zombie() {
                spotted.push(Spotted { chr: id, dist });
            }
            // instanceof IsoPlayer and not IsoAnimal.
            if o.is_player() && !o.is_ghost_mode(cx.ctx) {
                spotted.push(Spotted { chr: id, dist });
            }
        }
        if let Some(f) = self.animal.as_mut() {
            f.spotted = spotted.clone();
        }
        for s in spotted {
            self.behavior_spotted(s.chr, s.dist, cx);
        }
    }

    /// `IsoPlayer.update()` for an animal (`updateInternal1`):
    /// `IsoLivingCharacter.update()` then `AnimalInstanceManager.update`.
    /// A client's remote update is network.
    fn animal_iso_player_update(
        &mut self,
        out: &mut PlayerUpdate,
        cx: &mut StateCx<'_, I>,
    ) -> ScriptResult {
        // An animal has no timed actions between the two halves.
        if self.update_character_before_actions(cx)? {
            self.update_player_after_actions(out, cx)?;
        }
        if let Some(host) = cx.world.animal_host(self.id) {
            host.world.instance_manager_update();
        }
        Ok(())
    }
}
