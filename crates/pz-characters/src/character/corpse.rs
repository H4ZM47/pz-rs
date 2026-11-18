//! `new IsoDeadBody(chr)`: the character's side of `becomeCorpse()`.

use pz_inventory::shared::SharedContainer;
use pz_inventory::visual::human::HumanVisualOwner;
use pz_world::save::visual::{CorpseVisual, HumanVisual, IsoDeadBody, MovingBase, SurvivorDesc};

use super::items::blood_part;
use super::GameCharacter;
use crate::survivor::CharacterGender;
use pz_scripts::{ScriptError, ScriptResult};

/// `ObjectIDManager.ObjectIDType.DeadBody`'s type index.
const DEAD_BODY_ID_TYPE: i8 = 4;

impl<I: 'static> GameCharacter<I> {
    /// The `IsoDeadBody(chr, false)` constructor for a zombie or a player:
    /// the corpse takes the character's position, look, inventory and worn
    /// items, and the character is left a new empty inventory.
    /// `death_time` is `GameTime.getWorldAgeHours()`.
    ///
    /// `place` puts the corpse on its square (`getStaticMovingObjects().add`,
    /// which the original does before it takes the inventory) and returns
    /// whether it is there. The character keeps its inventory and worn items
    /// when the corpse cannot be made or placed; the result is whether the
    /// corpse was placed.
    ///
    /// The square, the world registries and the Lua events are the
    /// caller's part. Not ported: the ragdoll pose (`diedBoneTransforms`),
    /// the eating zombies, the shadow, the speech fields, `StartFire` for a
    /// burning character, a player's `removeSaveFile` and `deathFinished`,
    /// and the `ObjectIDManager` id (the id is left unset).
    pub fn become_dead_body(
        &mut self,
        death_time: f32,
        place: impl FnOnce(IsoDeadBody) -> bool,
    ) -> ScriptResult<bool> {
        if self.is_animal() {
            return Err(ScriptError::new("unsupported: IsoDeadBody(IsoAnimal)"));
        }
        if self.items.primary_hand_item.is_some() || self.items.secondary_hand_item.is_some() {
            // `die()` drops the hand items before `becomeCorpse()`.
            return Err(ScriptError::new("unsupported: IsoGameCharacter.dropHeldItems"));
        }
        let zombie = self.is_zombie();
        let mut fall_on_front = self.environment.fall_on_front;
        let mut crawling = false;
        if zombie && self.vars.crawling {
            if !self.zombie.reanimate {
                fall_on_front = true;
            }
            crawling = true;
        }
        let female = HumanVisualOwner::is_female(self);
        let (fake_dead, was_skeleton) = if zombie {
            (self.zombie.fake_dead, HumanVisualOwner::is_skeleton(self))
        } else {
            (false, false)
        };
        let desc = if zombie {
            self.descriptor.as_ref().map(|d| {
                let d = d.borrow();
                SurvivorDesc {
                    id: d.id,
                    forename: d.forename.clone().unwrap_or_default(),
                    surname: d.surname.clone().unwrap_or_default(),
                    torso: d.torso.clone().unwrap_or_default(),
                    female: d.character_gender == CharacterGender::Female,
                    profession: d.character_profession.as_ref().map(|p| p.location.id()).unwrap_or_default(),
                    extra: d.extra.clone(),
                    xp_boosts: d.xp_boost_map.clone(),
                    voice_prefix: d.voice_prefix.clone().unwrap_or_default(),
                    voice_pitch: d.voice_pitch,
                    voice_type: d.voice_type,
                }
            })
        } else {
            // TODO(port): a player's descriptor and the
            // `FemaleZombie`/`MaleZombie` voice prefix.
            None
        };
        let visual = self.human_visual.clone().unwrap_or_else(|| HumanVisual::new(blood_part::MAX));
        let zombie_rot_stage_at_death = i32::from(visual.zombie_rot_stage);
        // `setContainer(chr.getInventory())`, `setWornItems(chr.getWornItems())`:
        // worn items are saved as their index in the container.
        let mut container = self.items.inventory.borrow().to_container();
        let worn_items = self
            .items
            .worn_items
            .as_ref()
            .map(|w| {
                w.items()
                    .iter()
                    .filter_map(|worn| {
                        let location = worn.location.as_ref()?.location.id();
                        let index = container.items.iter().position(|i| i.id() == worn.item)?;
                        Some((location, index as i16))
                    })
                    .collect()
            })
            .unwrap_or_default();
        // `setExplored(chr instanceof IsoPlayer || reanimated player)`, then
        // `setExplored(true)` once the character has left the world.
        container.explored = true;
        let body = IsoDeadBody {
            base: MovingBase {
                // `chr.offsetX`/`offsetY`: 0 for a character.
                offset_x: 0.0,
                offset_y: 0.0,
                x: self.vars.x,
                y: self.vars.y,
                z: if self.vars.z < -32.0 { 0.0 } else { self.vars.z },
                dir: self.vars.dir as i32,
                mod_data: self.saved_mod_data()?,
            },
            female,
            was_zombie: zombie,
            animal: None,
            object_id: (-1, DEAD_BODY_ID_TYPE),
            persistent_outfit_id: if zombie { self.zombie.persistent_outfit_id } else { 0 },
            desc,
            visual: CorpseVisual::Human(Box::new(visual)),
            container: Some(container),
            worn_items,
            // TODO(port): `IsoZombie.DoZombieInventory` (from `onKilled`)
            // puts the attached weapon in the inventory; without it the
            // zombie's attached items are not in the container.
            attached_items: Vec::new(),
            death_time,
            reanimate_time: -1.0,
            fall_on_front,
            killed_by_fall: self.killed_by_fall,
            was_skeleton,
            angle: self.anim_angle_radians(),
            zombie_rot_stage_at_death,
            animal_rot_stage_at_death: 0,
            rotten_texture: String::new(),
            skel_inv_icon: String::new(),
            crawling,
            fake_dead,
            ragdoll: None,
            burn_timer: 0.0,
        };
        if !place(body) {
            return Ok(false);
        }
        // `chr.setInventory(new ItemContainer())`, `clearWornItems()`,
        // `clearAttachedItems()`.
        self.items.inventory = SharedContainer { explored: true, ..Default::default() }.into_ref();
        if let Some(w) = self.items.worn_items.as_mut() {
            w.clear();
        }
        Ok(true)
    }
}

#[cfg(test)]
#[path = "corpse_tests.rs"]
mod tests;
