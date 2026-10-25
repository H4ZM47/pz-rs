//! How a zombie gets dressed: `ModelManager.dressInRandomOutfit(chr)` (run
//! when the zombie is added to the model manager, so for new and loaded
//! zombies alike), `ZombiesZoneDefinition.dressInRandomOutfit`,
//! `IsoGameCharacter.dressInPersistentOutfit`,
//! `IsoZombie.dressInPersistentOutfitID` / `dressInNamedOutfit`,
//! `PersistentOutfits.applyOutfit` / `ZombiesZoneDefinition.ApplyCustomOutfit`,
//! `UnderwearDefinition.addRandomUnderwear`,
//! `AttachedWeaponDefinitions.addRandomAttachedWeapon`,
//! `IsoZombie.addRandomBloodDirtHolesEtc` with the visual damage and
//! bandages, and `PersistentOutfits.removeFallenHat`, in the original's
//! `Rand` / `OutfitRNG` order.
//!
//! Not modelled: the model manager, animation and sound side effects
//! (`ModelManager.ResetNextFrame`, `onWornItemsChanged`'s ragdoll flag,
//! the voice emitter), `Outfit.isEmpty()` (outfits load their items
//! synchronously here), the debug `ZombieOutfitRandom` option and the
//! multiplayer outfitters.

use pz_core::random::{self as rand, outfit_rng};
use pz_inventory::visual::human::HumanVisualOwner;
use pz_inventory::visual::ItemVisual;
use pz_inventory::{InventoryItem, ItemKind};
use pz_scripts::objects::items::{find_item, get_item, get_zed_dmg_map};
use pz_scripts::{ScriptError, ScriptResult};

use super::super::items::blood_part;
use super::GameCharacter;
use crate::population::dress::{add_clothing_item_for_script, dress_in_named_outfit_keep, synch_human_with_outfit};
use crate::population::persistent_outfits::{Outfitter, PersistentOutfits, NO_HAT_BIT};
use crate::population::zombie_defs::{
    advanced_outfit_name, get_random_hair_or_beard, AttachedWeaponDefinition, AttachedWeaponDefinitions, UnderwearDefinition,
    ZombiesZoneDefinition, ZoneWorld,
};
use crate::population::{blood_visuals, ClothingItemReference, DressContext};
use crate::Characters;

/// `BodyPartType.getBandageModel()` for `Hand_L` .. `Foot_R`.
const BANDAGE_MODELS: [&str; 17] = [
    "Base.Bandage_LeftHand",
    "Base.Bandage_RightHand",
    "Base.Bandage_LeftLowerArm",
    "Base.Bandage_RightLowerArm",
    "Base.Bandage_LeftUpperArm",
    "Base.Bandage_RightUpperArm",
    "Base.Bandage_Chest",
    "Base.Bandage_Abdomen",
    "Base.Bandage_Head",
    "Base.Bandage_Neck",
    "Base.Bandage_Groin",
    "Base.Bandage_LeftUpperLeg",
    "Base.Bandage_RightUpperLeg",
    "Base.Bandage_LeftLowerLeg",
    "Base.Bandage_RightLowerLeg",
    "Base.Bandage_LeftFoot",
    "Base.Bandage_RightFoot",
];

/// The singletons a zombie's outfit comes from.
#[derive(Clone, Debug, Default)]
pub struct ZombieOutfits {
    /// `PersistentOutfits.instance`.
    pub persistent: PersistentOutfits,
    /// `ZombiesZoneDefinition`'s statics.
    pub zones: ZombiesZoneDefinition,
    /// `UnderwearDefinition.instance`.
    pub underwear: UnderwearDefinition,
    /// `AttachedWeaponDefinitions.instance`.
    pub weapons: AttachedWeaponDefinitions,
}

/// What dressing reads from the world and the Lua tables.
pub trait ZombieDressWorld: ZoneWorld {
    /// `ZombiesZoneDefinition.checkDirty()`: reload from Lua's
    /// `ZombiesZoneDefinition` table when dirty.
    fn check_dirty_zones(&mut self, zones: &mut ZombiesZoneDefinition);
    /// `UnderwearDefinition.checkDirty()`.
    fn check_dirty_underwear(&mut self, underwear: &mut UnderwearDefinition);
    /// `AttachedWeaponDefinitions.checkDirty()` (its
    /// `chanceOfAttachedWeapon` is the sandbox option at that time).
    fn check_dirty_weapons(&mut self, weapons: &mut AttachedWeaponDefinitions);
    /// `InventoryItemFactory.CreateItem(itemType)` (with its `Rand` draws).
    fn create_item(&mut self, item_type: &str) -> Option<InventoryItem>;
    /// `ItemPickerJava.getLootModifier(itemType)`.
    fn loot_modifier(&self, item_type: &str) -> f32;
}

/// The settings dressing reads.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DressSettings {
    /// `SandboxOptions.clothingDegradation`.
    pub clothing_degradation: i32,
    /// `SandboxOptions.removeZombieLoot`.
    pub remove_zombie_loot: bool,
    /// `Core.getGameMode()` is `Tutorial`.
    pub tutorial: bool,
    /// `IsoWorld.getWorldAgeDays()`.
    pub world_age_days: f32,
    /// `GameTime.getWorldAgeDaysSinceBegin()`.
    pub world_age_days_since_begin: f64,
}

/// Everything dressing a zombie needs.
pub struct ZombieDressCx<'a> {
    /// `SurvivorFactory`'s names, `AttachedLocations`.
    pub chars: &'a Characters,
    /// Outfits, clothing items, scripts, the `Human` body locations and the
    /// style tables.
    pub dress: &'a DressContext<'a>,
    pub outfits: &'a mut ZombieOutfits,
    pub world: &'a mut dyn ZombieDressWorld,
    pub settings: DressSettings,
}

fn npe() -> ScriptError {
    ScriptError::new("java.lang.NullPointerException")
}

impl<I: 'static> GameCharacter<I> {
    fn visual_mut(&mut self) -> &mut pz_inventory::visual::HumanVisual {
        self.human_visual.get_or_insert_with(|| pz_inventory::visual::HumanVisual::new(blood_part::MAX))
    }

    /// `isUsingWornItems()` (`isOnDeathDone()` is the `dead` flag here).
    pub fn is_using_worn_items(&self) -> bool {
        self.vars.on_kill_done || self.dead || self.zombie.reanimated_player || self.zombie.was_fake_dead
    }

    /// `IsoZombie.setFemaleEtc(female)` (the voice emitter is not modelled).
    pub fn set_female_etc(&mut self, female: bool) {
        self.zombie.female = female;
        if let Some(d) = &self.descriptor {
            let mut d = d.borrow_mut();
            d.set_female(female);
            d.voice_prefix = Some(format!("{}Zombie", if female { "Female" } else { "Male" }));
        }
        self.zombie.sprite_name = if female { "KateZ" } else { "BobZ" }.into();
        self.zombie.hurt_sound = format!("{}Hurt", self.voice_prefix());
    }

    /// `getDescriptor().setForename(SurvivorFactory.getRandomForename(isFemale()))`.
    fn random_forename(&mut self, chars: &Characters) -> ScriptResult {
        let name = chars.random_forename(self.is_female())?;
        self.descriptor.as_ref().ok_or_else(npe)?.borrow_mut().forename = Some(name);
        Ok(())
    }

    /// `ModelManager.dressInRandomOutfit(chr)`: a zombie flagged to dress
    /// at random picks a forename and an outfit
    /// (`ZombiesZoneDefinition.dressInRandomOutfit`), then a zombie whose
    /// persistent outfit is not set up dresses in its outfit id. Reanimated
    /// players and zombies that were fake dead keep what they wear. `Err`
    /// is the exception the original throws.
    pub fn model_manager_dress_in_random_outfit(&mut self, cx: &mut ZombieDressCx<'_>) -> ScriptResult {
        if !self.is_zombie() || self.zombie.reanimated_player || self.zombie.was_fake_dead {
            return Ok(());
        }
        if self.zombie.dress_in_random_outfit {
            self.random_forename(cx.chars)?;
            self.zombie.dress_in_random_outfit = false;
            self.zzd_dress_in_random_outfit(cx)?;
        }
        if !self.zombie.persistent_outfit_init {
            let id = self.zombie.persistent_outfit_id;
            self.dress_in_persistent_outfit_id(id, cx)?;
        }
        Ok(())
    }

    /// `ZombiesZoneDefinition.dressInRandomOutfit(chr)`: the zone's
    /// definition for the zombie's square, else a `Default` outfit, with
    /// random underwear first.
    pub fn zzd_dress_in_random_outfit(&mut self, cx: &mut ZombieDressCx<'_>) -> ScriptResult {
        if HumanVisualOwner::is_skeleton(self) {
            return Ok(());
        }
        let Some((x, y, z)) = self.path.current_square else {
            return Ok(());
        };
        let female = self.is_female();
        let pick = match cx.world.square_room(x, y, z) {
            None => None,
            Some(_) => {
                cx.world.check_dirty_zones(&mut cx.outfits.zones);
                cx.outfits.zones.pick_definition(&*cx.world, x, y, z, female, false)
            }
        };
        let Some(pick) = pick else {
            let room = cx.world.square_room(x, y, z).flatten();
            let name = cx.outfits.zones.random_default_outfit_name(female, room.as_deref()).map_err(ScriptError::new)?;
            let found = name.and_then(|n| cx.dress.outfits.get_specific_outfit(female, &n).map(|o| o.name.clone()));
            let outfit = match found {
                Some(n) => n,
                None => cx.dress.outfits.get_random_outfit(female).map(|o| o.name.clone()).ok_or_else(npe)?,
            };
            self.add_random_underwear(cx)?;
            return self.dress_in_persistent_outfit(&outfit, cx);
        };
        self.add_random_underwear(cx)?;
        // `applyDefinition`.
        let female = pick.female;
        self.set_female_etc(female);
        let table_name = cx.outfits.zones.outfit(pick.table).name.clone().unwrap_or_default();
        let source = cx.dress.outfits.get_specific_outfit(female, &table_name).map(|o| o.name.clone());
        let source = match source {
            None => cx.dress.outfits.get_random_outfit(female).map(|o| o.name.clone()),
            Some(n) => {
                cx.outfits.zones.count_spawned(pick.zone_key, &n);
                Some(n)
            }
        };
        if let Some(n) = source {
            self.dress_in_persistent_outfit(&n, cx)?;
        }
        Ok(())
    }

    /// `IsoGameCharacter.dressInPersistentOutfit(outfitName)`: a zombie
    /// picks a forename, then an outfit id (`pickOutfit`, with the later
    /// stage `ZombiesStageDefinitions` gives) to dress in.
    pub fn dress_in_persistent_outfit(&mut self, outfit_name: &str, cx: &mut ZombieDressCx<'_>) -> ScriptResult {
        if self.is_zombie() {
            self.random_forename(cx.chars)?;
        }
        let advanced = advanced_outfit_name(outfit_name, cx.settings.world_age_days_since_begin);
        let id = cx.outfits.persistent.pick_outfit(outfit_name, advanced.as_deref(), self.is_female());
        self.dress_in_persistent_outfit_id(id, cx)
    }

    /// `IsoZombie.dressInPersistentOutfitID(outfitID)`.
    pub fn dress_in_persistent_outfit_id(&mut self, outfit_id: i32, cx: &mut ZombieDressCx<'_>) -> ScriptResult {
        self.visual_mut().clear();
        self.zombie.item_visuals.clear();
        self.zombie.persistent_outfit_id = outfit_id;
        self.zombie.persistent_outfit_init = true;
        if outfit_id == 0 {
            return Ok(());
        }
        self.zombie.dress_in_random_outfit = false;
        let Some(apply) = cx.outfits.persistent.dress_in_outfit(outfit_id) else {
            return Ok(());
        };
        let female = apply.outfit_id & i32::MIN != 0;
        match apply.outfitter {
            Outfitter::Outfit => {
                // `applyOutfit`: the entry by the id's index.
                let name = cx.outfits.persistent.data_of_id(apply.outfit_id).map(|d| d.outfit_name.clone()).ok_or_else(npe)?;
                self.set_female_etc(female);
                self.zombie_dress_in_named_outfit(&name, cx)?;
                if self.zombie.do_dirt_blood_etc {
                    self.add_random_attached_weapon(cx)?;
                    self.add_random_blood_dirt_holes_etc(cx);
                }
                self.remove_fallen_hat(apply.outfit_id, cx);
            }
            Outfitter::ZombiesZone => {
                let Some(def) = cx.outfits.zones.custom_outfit(&apply.outfit_name).cloned() else {
                    return Err(npe());
                };
                self.set_female_etc(female);
                self.zombie_dress_in_named_outfit(def.name.as_deref().unwrap_or("null"), cx)?;
                self.add_random_attached_weapon(cx)?;
                self.add_random_blood_dirt_holes_etc(cx);
                let female = self.is_female();
                let hv = self.visual_mut();
                if female {
                    if let Some(s) = &def.female_hair_styles {
                        hv.hair_model = get_random_hair_or_beard(s);
                    }
                }
                if !female {
                    if let Some(s) = &def.male_hair_styles {
                        hv.hair_model = get_random_hair_or_beard(s);
                    }
                    if let Some(s) = &def.beard_styles {
                        hv.beard_model = get_random_hair_or_beard(s);
                    }
                }
                self.remove_fallen_hat(apply.outfit_id, cx);
            }
            // Multiplayer only.
            Outfitter::ReanimatedPlayer => {}
        }
        Ok(())
    }

    /// `IsoZombie.dressInNamedOutfit(outfitName)`.
    pub fn zombie_dress_in_named_outfit(&mut self, outfit_name: &str, cx: &mut ZombieDressCx<'_>) -> ScriptResult {
        self.items.worn_items.clear();
        self.items.worn_locations.clear();
        self.visual_mut().clear();
        self.zombie.item_visuals.clear();
        let female = self.is_female();
        if cx.dress.outfits.get_specific_outfit(female, outfit_name).is_none() {
            return Ok(());
        }
        self.add_random_underwear(cx)?;
        self.random_forename(cx.chars)?;
        let mut hv = self.human_visual.take().unwrap_or_else(|| pz_inventory::visual::HumanVisual::new(blood_part::MAX));
        let r = dress_in_named_outfit_keep(&mut hv, female, Some(outfit_name), &mut self.zombie.item_visuals, cx.dress, false);
        let r = r.map(|o| {
            if let Some(o) = o {
                self.zombie.human_outfit = Some(o);
            }
            let styles = cx.dress.outfit.population.styles(cx.dress.outfit.hair_common_colors, cx.dress.outfit.world_age_days);
            synch_human_with_outfit(&mut hv, self.zombie.human_outfit.as_ref(), &*self, &styles);
        });
        self.zombie.outfit_name = hv.outfit.clone();
        self.human_visual = Some(hv);
        r.map_err(ScriptError::new)
    }

    /// `UnderwearDefinition.addRandomUnderwear(zed)`.
    pub fn add_random_underwear(&mut self, cx: &mut ZombieDressCx<'_>) -> ScriptResult {
        if HumanVisualOwner::is_skeleton(self) {
            return Ok(());
        }
        cx.world.check_dirty_underwear(&mut cx.outfits.underwear);
        let Some(pick) = cx.outfits.underwear.pick(self.is_female()) else {
            return Ok(());
        };
        let sm = cx.dress.scripts;
        let mut bottom_tint = None;
        let bottom = pick.bottom.as_deref().and_then(|b| find_item(sm, b));
        if let Some(s) = bottom {
            let at = add_clothing_item_for_script(cx.dress, &mut self.zombie.item_visuals, &s.borrow()).map_err(ScriptError::new)?;
            bottom_tint = at.map(|i| self.zombie.item_visuals[i].tint);
        }
        if let Some(tops) = &pick.tops {
            let top = UnderwearDefinition::pick_top(tops, pick.top_total_chance);
            if let Some(s) = top.as_deref().and_then(|t| find_item(sm, t)) {
                let at = add_clothing_item_for_script(cx.dress, &mut self.zombie.item_visuals, &s.borrow()).map_err(ScriptError::new)?;
                if outfit_rng::next(100) < 60 {
                    if let (Some(i), Some(tint)) = (at, bottom_tint) {
                        self.zombie.item_visuals[i].tint = tint;
                    }
                }
            }
        }
        Ok(())
    }

    /// `AttachedWeaponDefinitions.addRandomAttachedWeapon(zed)`.
    pub fn add_random_attached_weapon(&mut self, cx: &mut ZombieDressCx<'_>) -> ScriptResult {
        if cx.settings.tutorial {
            return Ok(());
        }
        cx.world.check_dirty_weapons(&mut cx.outfits.weapons);
        let outfit = self.zombie.human_outfit.as_ref().map(|o| o.name.clone());
        let Some(mut rounds) = cx.outfits.weapons.rounds(outfit.as_deref()) else {
            return Ok(());
        };
        while rounds.repeat > 0 {
            let Some(i) = AttachedWeaponDefinitions::pick_random_in_list(&rounds.pool, outfit.as_deref(), cx.settings.world_age_days) else {
                return Ok(());
            };
            let def = rounds.pool.remove(i);
            rounds.repeat -= 1;
            self.add_attached_weapon(&def, cx)?;
            match rounds.custom_chance {
                Some(c) if outfit_rng::next(100) >= c => return Ok(()),
                _ => {}
            }
        }
        Ok(())
    }

    /// `AttachedWeaponDefinitions.addAttachedWeapon(toDo, zed)`.
    fn add_attached_weapon(&mut self, def: &AttachedWeaponDefinition, cx: &mut ZombieDressCx<'_>) -> ScriptResult {
        let weapon_type = outfit_rng::pick_random(&def.weapons).cloned();
        let weapon_type = weapon_type.unwrap_or_else(|| "null".into());
        if cx.settings.remove_zombie_loot && cx.world.loot_modifier(&weapon_type) == 0.0 {
            return Ok(());
        }
        let Some(mut weapon) = cx.world.create_item(&weapon_type) else {
            return Ok(());
        };
        if matches!(weapon.kind, ItemKind::Weapon(_)) {
            randomize_bullets(&mut weapon, cx);
        }
        let max = weapon.condition_max;
        let c = outfit_rng::next_range((max - 5).max(2), max);
        weapon.condition = c.max(0).min(max);
        let location = outfit_rng::pick_random(&def.weapon_location).cloned();
        self.set_attached_item(location.as_deref(), weapon.clone(), cx.chars)?;
        if let Some(ensure) = &def.ensure_item {
            if !self.outfit_has_item(ensure) {
                let sm = cx.dress.scripts;
                let script = find_item(sm, ensure);
                let clothing = script.as_ref().is_some_and(|s| {
                    s.borrow().clothing_item.as_deref().is_some_and(|n| cx.dress.clothing.get(n).is_some())
                });
                match script {
                    Some(s) if clothing => {
                        add_clothing_item_for_script(cx.dress, &mut self.zombie.item_visuals, &s.borrow()).map_err(ScriptError::new)?;
                    }
                    _ => {
                        if let Some(item) = cx.world.create_item(ensure) {
                            if !self.zombie.items_to_spawn_at_death.iter().any(|i| i.id == item.id) {
                                self.zombie.items_to_spawn_at_death.push(item);
                            }
                        }
                    }
                }
            }
        }
        if !def.blood_locations.is_empty() {
            let mut level = (rand::next(100) as f32 / 100.0).max(rand::next(100) as f32 / 100.0);
            level = level.max(rand::next(100) as f32 / 100.0);
            if let Some((_, w)) = self.zombie.attached_items.iter_mut().find(|(_, w)| w.id == weapon.id) {
                if let ItemKind::Weapon(d) = &mut w.kind {
                    d.blood_level = level.clamp(0.0, 1.0);
                }
            }
            for &part in &def.blood_locations {
                for _ in 0..3 {
                    self.add_blood(Some(part), true, true, true, cx);
                }
                if def.add_holes {
                    self.add_hole(Some(part), true, cx);
                }
            }
        }
        Ok(())
    }

    /// `outfitHasItem(zombie, ensureItem)`.
    fn outfit_has_item(&self, ensure: &str) -> bool {
        self.zombie.item_visuals.iter().any(|v| {
            v.full_type == ensure
                || ensure == "Base.HolsterSimple" && v.full_type == "Base.HolsterDouble"
                || ensure == "Base.HolsterDouble" && v.full_type == "Base.HolsterSimple"
        })
    }

    /// `setAttachedItem(location, item)` on the `Human` attached locations
    /// (`AttachedItems.setItem`).
    pub fn set_attached_item(&mut self, location: Option<&str>, item: InventoryItem, chars: &Characters) -> ScriptResult {
        let group = chars.attached_locations.groups().iter().find(|g| g.borrow().id() == "Human").cloned();
        let index_of = |loc: &str| group.as_ref().map_or(-1, |g| g.borrow().index_of(Some(loc)));
        match &group {
            Some(g) => g.borrow().check_valid(location)?,
            None => crate::attached_items::AttachedLocationGroup::new(Some("Human"))?.check_valid(location)?,
        }
        let location = location.ok_or_else(npe)?;
        let items = &mut self.zombie.attached_items;
        if let Some(i) = items.iter().position(|(l, _)| l == location) {
            items.remove(i);
        }
        items.retain(|(_, w)| w.id != item.id);
        let at = items.iter().position(|(l, _)| index_of(l) > index_of(location)).unwrap_or(items.len());
        items.insert(at, (location.to_string(), item));
        Ok(())
    }

    /// `IsoZombie.addRandomBloodDirtHolesEtc()`.
    pub fn add_random_blood_dirt_holes_etc(&mut self, cx: &mut ZombieDressCx<'_>) {
        if !HumanVisualOwner::is_skeleton(self) {
            self.add_random_visual_damages(cx);
            self.add_random_visual_bandages(cx);
        }
        let mut df = cx.settings.clothing_degradation as f32;
        if df == 1.0 {
            return;
        }
        df = (df - 1.0) / 2.0;
        df *= df;
        let all_layers = df >= 1.0;
        self.add_blood(None, false, true, false, cx);
        let months = (cx.settings.world_age_days / 30.0 * df) as i32;
        let n = ((rand::next_range(5, 20) + months.min(24)) as f32 * df) as i32;
        self.add_lots_of_dirt(None, Some(n), all_layers, cx);
        let mut chance = (8 - months).max(0);
        let rolls = months.clamp(5, 10) * df as i32;
        for _ in 0..rolls {
            if outfit_rng::next_bool(chance) {
                self.add_blood(None, false, true, false, cx);
            }
            if outfit_rng::next_bool(chance / 2) {
                self.add_lots_of_dirt(None, None, all_layers, cx);
            }
        }
        let rolls = months.clamp(8, 16) * df as i32;
        chance /= 2;
        for _ in 0..rolls {
            if !outfit_rng::next_bool(chance) {
                continue;
            }
            let part = outfit_rng::next_range(0, blood_part::MAX as i32) as usize;
            self.add_hole(Some(part), false, cx);
            self.add_blood(Some(part), true, false, false, cx);
        }
        for _ in 0..rolls {
            if !outfit_rng::next_bool(chance) {
                continue;
            }
            let part = outfit_rng::next_range(0, blood_part::MAX as i32) as usize;
            self.add_hole(Some(part), false, cx);
            self.add_lots_of_dirt(Some(part), Some(1), all_layers, cx);
        }
    }

    /// `IsoZombie.addRandomVisualDamages()`.
    pub fn add_random_visual_damages(&mut self, cx: &mut ZombieDressCx<'_>) {
        let map = get_zed_dmg_map(cx.dress.scripts);
        for _ in 0..5 {
            if outfit_rng::next(5) != 0 {
                continue;
            }
            let dmg = outfit_rng::pick_random(&map).cloned().unwrap_or_else(|| "null".into());
            self.add_body_visual_from_item_type(&format!("Base.{dmg}"), cx);
        }
    }

    /// `IsoZombie.addRandomVisualBandages()`.
    pub fn add_random_visual_bandages(&mut self, cx: &mut ZombieDressCx<'_>) {
        if cx.settings.tutorial {
            return;
        }
        for _ in 0..5 {
            if outfit_rng::next(10) != 0 {
                continue;
            }
            let part = outfit_rng::next_range(0, BANDAGE_MODELS.len() as i32) as usize;
            self.add_body_visual_from_item_type(&format!("{}_Blood", BANDAGE_MODELS[part]), cx);
        }
    }

    /// `IsoGameCharacter.addBodyVisualFromItemType(itemType)`: the item's
    /// clothing with a fresh randomized reference, unless an identical body
    /// visual is there.
    pub fn add_body_visual_from_item_type(&mut self, item_type: &str, cx: &mut ZombieDressCx<'_>) -> bool {
        let Some(script) = get_item(cx.dress.scripts, item_type) else {
            return false;
        };
        let Some(clothing) = script.borrow().clothing_item.as_deref().and_then(|n| cx.dress.clothing.get(n)) else {
            return false;
        };
        let mut r = ClothingItemReference { item_guid: Some(clothing.guid.clone()), ..Default::default() };
        if r.randomize(cx.dress.outfit).is_err() {
            return false;
        }
        let picked = cx
            .dress
            .outfit
            .clothing_items
            .clothing_item(r.picked_guid().unwrap_or_default())
            .flatten()
            .unwrap_or(clothing);
        let mut v = ItemVisual::new(item_type);
        crate::population::dress::synch_with_outfit(&mut v, &r, picked);
        let hv = self.visual_mut();
        let dup = hv
            .body_visuals
            .iter()
            .any(|b| b.clothing_item_name == v.clothing_item_name && b.texture_choice == v.texture_choice && b.base_texture == v.base_texture);
        if dup {
            return false;
        }
        hv.body_visuals.push(v);
        true
    }

    /// `IsoGameCharacter.addBlood(part, scratched, bitten, allLayers)` for a
    /// zombie (no weapon in hand).
    pub fn add_blood(&mut self, part: Option<usize>, scratched: bool, bitten: bool, all_layers: bool, cx: &mut ZombieDressCx<'_>) {
        let mut n = 1;
        if bitten {
            n = 20;
        }
        if scratched {
            n = 5;
        }
        if self.is_zombie() {
            n += 8;
        }
        let sm = cx.dress.scripts;
        let deg = cx.settings.clothing_degradation;
        for _ in 0..n {
            let p = part.unwrap_or_else(|| outfit_rng::next_range(0, blood_part::MAX as i32) as usize);
            let intensity = blood_visuals::degradation_intensity(deg);
            let mut hv = self.human_visual.take().unwrap_or_else(|| pz_inventory::visual::HumanVisual::new(blood_part::MAX));
            blood_visuals::add_blood(sm, p, intensity, &mut hv, &mut self.zombie.item_visuals, all_layers);
            self.human_visual = Some(hv);
        }
    }

    /// `IsoGameCharacter.addLotsOfDirt(part, nbr, allLayers)`.
    pub fn add_lots_of_dirt(&mut self, part: Option<usize>, n: Option<i32>, all_layers: bool, cx: &mut ZombieDressCx<'_>) {
        let n = n.unwrap_or_else(|| outfit_rng::next_range(5, 10));
        let sm = cx.dress.scripts;
        for _ in 0..n {
            let p = part.unwrap_or_else(|| outfit_rng::next_range(0, blood_part::MAX as i32) as usize);
            let intensity = rand::next_float(0.01, 1.0);
            let mut hv = self.human_visual.take().unwrap_or_else(|| pz_inventory::visual::HumanVisual::new(blood_part::MAX));
            blood_visuals::add_dirt(sm, p, intensity, &mut hv, &mut self.zombie.item_visuals, all_layers);
            self.human_visual = Some(hv);
        }
    }

    /// `IsoGameCharacter.addHole(part, allLayers)`.
    pub fn add_hole(&mut self, part: Option<usize>, all_layers: bool, cx: &mut ZombieDressCx<'_>) -> bool {
        let p = part.unwrap_or_else(|| outfit_rng::next_range(0, blood_part::MAX as i32) as usize);
        let mut hv = self.human_visual.take().unwrap_or_else(|| pz_inventory::visual::HumanVisual::new(blood_part::MAX));
        let r = blood_visuals::add_hole(cx.dress.scripts, p, &mut hv, &mut self.zombie.item_visuals, all_layers);
        self.human_visual = Some(hv);
        r
    }

    /// `PersistentOutfits.removeFallenHat(outfitID, chr)`: a zombie whose
    /// hat fell keeps none of the items that can fall.
    pub fn remove_fallen_hat(&mut self, outfit_id: i32, cx: &ZombieDressCx<'_>) -> bool {
        if outfit_id & NO_HAT_BIT == 0 || self.is_using_worn_items() {
            return false;
        }
        let sm = cx.dress.scripts;
        let before = self.zombie.item_visuals.len();
        self.zombie
            .item_visuals
            .retain(|v| v.full_type.trim().is_empty() || get_item(sm, &v.full_type).is_none_or(|s| s.borrow().chance_to_fall <= 0));
        self.zombie.item_visuals.len() != before
    }
}

/// `HandWeapon.randomizeBullets()`: a ranged weapon is loaded three times
/// in four (`Rand`).
fn randomize_bullets(w: &mut InventoryItem, cx: &ZombieDressCx<'_>) {
    if !w.is_ranged() || rand::next(4) == 0 {
        return;
    }
    let max_ammo = get_item(cx.dress.scripts, &w.get_full_type()).map_or(0, |s| s.borrow().max_ammo);
    w.set_current_ammo_count(rand::next_range(max_ammo - 2, max_ammo));
    let magazine = w.template.magazine_type.clone();
    if magazine.as_deref().is_some_and(|m| !m.is_empty()) {
        w.set_contains_clip(true);
    }
    let have_chamber = w.template.have_chamber;
    if have_chamber {
        w.set_round_chambered(true, have_chamber);
    }
}

#[cfg(test)]
#[path = "dress_tests.rs"]
mod tests;
