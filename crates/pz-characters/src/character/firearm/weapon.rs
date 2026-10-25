//! The firearm side of `HandWeapon`: the fields the ranged combat code
//! reads ([`FirearmStats`]) and the perk-scaled getters, the jam checks and
//! the ammunition state, over pz-inventory's item API.
//!
//! The script values are the ones `Item.InstanceItem` copies onto the
//! weapon; weapon parts (`activeSight` and the part modifiers) are not
//! applied. TODO(weapon parts).

use pz_inventory::InventoryItem;
use pz_scripts::objects::items::Item;
use pz_scripts::ScriptManager;

use crate::character::items::{is_hand_weapon, script_item};
use crate::character::{GameCharacter, SimContext};
use crate::Characters;

/// What the ranged combat code reads of a `HandWeapon`.
#[derive(Clone, Debug, PartialEq)]
pub struct FirearmStats {
    /// `InventoryItem.id`.
    pub id: i32,
    pub ranged: bool,
    pub aimed_firearm: bool,
    pub range_falloff: bool,
    pub piercing_bullets: bool,
    pub two_hand: bool,
    /// `maxRange`.
    pub max_range: f32,
    pub hit_chance: i32,
    pub aiming_perk_hit_chance_modifier: f32,
    pub aiming_perk_range_modifier: f32,
    pub aiming_perk_crit_modifier: i32,
    pub recoil_delay: i32,
    pub aiming_time: i32,
    /// The instance's `minSightRange` / `maxSightRange`.
    pub min_sight_range: f32,
    pub max_sight_range: f32,
    /// `getLowLightBonus()` (an active sight's; none without weapon parts).
    pub low_light_bonus: f32,
    /// `getActiveSight().hasTag(THERMAL)`.
    pub thermal_sight: bool,
    pub projectile_count: i32,
    pub projectile_spread: f32,
    pub projectile_weight_center: f32,
    pub stop_power: f32,
    pub jam_gun_chance: f32,
    pub have_chamber: bool,
    pub rack_after_shoot: bool,
    pub manually_remove_spent_rounds: bool,
    /// `ammoPerShoot` (no script value sets it).
    pub ammo_per_shoot: i32,
    /// `getMaxAmmo()`.
    pub max_ammo: i32,
    /// `getAmmoType()`.
    pub ammo_type: Option<std::rc::Rc<pz_scripts::registry::RegistryKey>>,
    /// `getPhysicsObject()`.
    pub physics_object: Option<String>,
    pub splat_number: i32,
    /// `getZombieHitSound()`.
    pub zombie_hit_sound: Option<String>,
    pub shell_fall_sound: Option<String>,
    /// `getFireMode()`.
    pub fire_mode: Option<String>,
    pub condition: i32,
    pub condition_max: i32,
    /// `hasTag(NO_CRITICALS)`.
    pub no_criticals: bool,
}

impl FirearmStats {
    /// The firearm fields of a hand weapon item (its script looked up in
    /// `sm`).
    pub fn of(sm: &ScriptManager, item: &InventoryItem) -> Option<FirearmStats> {
        if !is_hand_weapon(item) {
            return None;
        }
        let script = script_item(sm, item);
        let guard = script.as_ref().map(|s| s.borrow());
        let s: Option<&Item> = guard.as_deref();
        let data = item.weapon_data();
        Some(FirearmStats {
            id: item.id,
            ranged: item.is_ranged(),
            aimed_firearm: item.is_aimed_firearm(),
            range_falloff: s.is_some_and(|s| s.range_falloff),
            piercing_bullets: s.is_some_and(|s| s.piercing_bullets),
            two_hand: s.is_some_and(|s| s.two_hand_weapon),
            max_range: s.map_or(1.0, |s| s.max_range),
            hit_chance: s.map_or(0, |s| s.hit_chance),
            aiming_perk_hit_chance_modifier: s.map_or(0.0, |s| s.aiming_perk_hit_chance_modifier),
            aiming_perk_range_modifier: s.map_or(0.0, |s| s.aiming_perk_range_modifier),
            aiming_perk_crit_modifier: s.map_or(0, |s| s.aiming_perk_crit_modifier),
            recoil_delay: s.map_or(0, |s| s.recoil_delay),
            aiming_time: s.map_or(0, |s| s.aiming_time),
            min_sight_range: data.map_or(2.0, |d| d.min_sight_range),
            max_sight_range: data.map_or(6.0, |d| d.max_sight_range),
            low_light_bonus: 0.0,
            thermal_sight: false,
            projectile_count: s.map_or(1, |s| s.projectile_count),
            projectile_spread: s.map_or(0.0, |s| s.projectile_spread),
            projectile_weight_center: s.map_or(1.0, |s| s.projectile_weight_center),
            stop_power: s.map_or(5.0, |s| s.stop_power),
            jam_gun_chance: s.map_or(5.0, |s| s.jam_gun_chance),
            have_chamber: s.is_none_or(|s| s.have_chamber),
            rack_after_shoot: s.is_some_and(|s| s.rack_after_shoot),
            manually_remove_spent_rounds: s.is_some_and(|s| s.manually_remove_spent_rounds),
            ammo_per_shoot: 1,
            max_ammo: s.map_or(0, |s| s.max_ammo),
            ammo_type: s.and_then(|s| s.ammo_type.clone()),
            physics_object: s.and_then(|s| s.physics_object.clone()),
            splat_number: s.map_or(2, |s| s.splat_number),
            zombie_hit_sound: s.and_then(|s| s.hit_sound.clone()),
            shell_fall_sound: s.and_then(|s| s.shell_fall_sound.clone()),
            fire_mode: data.and_then(|d| d.fire_mode.clone()),
            condition: item.condition,
            condition_max: item.condition_max,
            no_criticals: item_has_tag(sm, s, "NO_CRITICALS"),
        })
    }
}

fn item_has_tag(sm: &ScriptManager, s: Option<&Item>, field: &str) -> bool {
    let Some(s) = s else { return false };
    pz_scripts::objects::items::ItemTag::constant(sm, field).is_some_and(|k| s.item_tags.contains(Some(&k)))
}

impl<I: 'static> GameCharacter<I> {
    fn perk_of(&self, chars: &Characters, field: &str) -> i32 {
        self.perk_level(chars.perks.constant(field).as_ref())
    }

    /// `HandWeapon.getMaxRange(owner)`: ranged weapons add the aiming perk
    /// bonus.
    pub fn firearm_max_range(&self, chars: &Characters, w: &FirearmStats) -> f32 {
        if w.ranged {
            return w.max_range + w.aiming_perk_range_modifier * (self.perk_of(chars, "Aiming") as f32 / 2.0);
        }
        w.max_range
    }

    /// `HandWeapon.getRecoilDelay(owner)`.
    pub fn firearm_recoil_delay(&self, chars: &Characters, w: &FirearmStats) -> i32 {
        let aiming = self.perk_of(chars, "Aiming") as f32;
        let strength = self.perk_of(chars, "Strength") as f32;
        let primary_only = self.items.primary_hand_item == Some(w.id)
            && self.items.secondary_hand_item != Some(w.id)
            && self.items.secondary_hand_item.is_some();
        let v = (w.recoil_delay as f32
            * (1.0 - aiming / 40.0)
            * (1.0 - (-10.0 + strength * 2.0) / 40.0)
            * if primary_only { 1.3 } else { 1.0 }) as i32;
        v.max(0)
    }

    /// `HandWeapon.getMinSightRange(character)`.
    pub fn firearm_min_sight_range(&self, chars: &Characters, w: &FirearmStats) -> f32 {
        w.min_sight_range * (1.0 - self.perk_of(chars, "Aiming") as f32 / 30.0)
    }

    /// `HandWeapon.getMaxSightRange(character)`.
    pub fn firearm_max_sight_range(&self, sm: &ScriptManager, chars: &Characters, w: &FirearmStats) -> f32 {
        if self.has_trait(sm, "SHORT_SIGHTED").unwrap_or(false) && !self.is_wearing_glasses(sm) {
            return self.firearm_min_sight_range(chars, w);
        }
        let eagle = if self.has_trait(sm, "EAGLE_EYED").unwrap_or(false) { 1.2 } else { 1.0 };
        w.max_sight_range * (1.0 + self.perk_of(chars, "Aiming") as f32 / 30.0) * eagle
    }

    /// `isWearingGlasses()`: a worn visual aid.
    pub fn is_wearing_glasses(&self, sm: &ScriptManager) -> bool {
        self.items.worn().any(|i| script_item(sm, &i.borrow()).is_some_and(|s| s.borrow().visual_aid))
    }

    /// `HandWeapon.checkJam(player, racking)` on the weapon item `id`:
    /// draws `Rand.Next(0f, 1f)` unless the chance is 0; returns
    /// `isJammed()`.
    pub fn check_jam(&mut self, sm: &ScriptManager, chars: &Characters, ctx: &dyn SimContext, id: i32, racking: bool) -> bool {
        let Some(mut item) = self.items.item(id).map(|i| i.borrow().clone()) else { return false };
        let r = self.check_jam_item(sm, chars, ctx, &mut item, racking);
        *self.items.item(id).expect("weapon present above").borrow_mut() = item;
        r
    }

    /// [`Self::check_jam`] on a weapon item the character fires (`self` is
    /// the player argument).
    pub fn check_jam_item(&self, sm: &ScriptManager, chars: &Characters, ctx: &dyn SimContext, item: &mut InventoryItem, racking: bool) -> bool {
        let Some(w) = FirearmStats::of(sm, item) else { return false };
        let mut chance = (w.jam_gun_chance as f64 * ctx.firearm_jam_multiplier()) as f32;
        if chance == 0.0 {
            return false;
        }
        let aiming = self.perk_of(chars, "Aiming");
        let strength = self.perk_of(chars, "Strength");
        let limp = !racking && !w.manually_remove_spent_rounds && (aiming < 3 && strength < 5 || aiming < 5 && strength < 3);
        let modifier = 8.0 * (w.condition as f32 / w.condition_max as f32);
        chance = (chance + if limp { 0.5 } else { 0.0 } + (w.condition_max - w.condition) as f32 / modifier) * 0.01;
        let roll = ctx.rand_next_float(0.0, 1.0);
        if roll < chance {
            item.set_jammed(true);
        }
        item.is_jammed()
    }

    /// `HandWeapon.checkUnJam(player)` on the weapon item `id`; returns
    /// `isJammed()`.
    pub fn check_unjam(&mut self, sm: &ScriptManager, chars: &Characters, ctx: &dyn SimContext, id: i32) -> bool {
        let Some(mut item) = self.items.item(id).map(|i| i.borrow().clone()) else { return false };
        let r = self.check_unjam_item(sm, chars, ctx, &mut item);
        *self.items.item(id).expect("weapon present above").borrow_mut() = item;
        r
    }

    /// [`Self::check_unjam`] on a weapon item.
    pub fn check_unjam_item(&self, sm: &ScriptManager, chars: &Characters, ctx: &dyn SimContext, item: &mut InventoryItem) -> bool {
        let Some(w) = FirearmStats::of(sm, item) else { return false };
        let modifier = 8.0 * (w.condition as f32 / w.condition_max as f32);
        let moodles = self.moodles.moodle_level_of(sm, "PANIC").unwrap_or(0)
            + self.moodles.moodle_level_of(sm, "STRESS").unwrap_or(0)
            + self.moodles.moodle_level_of(sm, "DRUNK").unwrap_or(0);
        let mut chance = 8.0 - self.perk_of(chars, "Aiming") as f32 * 0.5 + (moodles * 3) as f32;
        chance -= if self.has_trait(sm, "DEXTROUS").unwrap_or(false) { 2.0 } else { 0.0 };
        chance += if self.has_trait(sm, "ALL_THUMBS").unwrap_or(false) { 2.0 } else { 0.0 };
        chance = chance.max(1.0);
        chance = 1.0 - (chance + (w.condition_max - w.condition) as f32 / modifier) * 0.01;
        let roll = ctx.rand_next_float(0.0, 1.0);
        if roll < chance {
            item.set_jammed(false);
            if let Some(s) = w.shell_fall_sound {
                ctx.event(crate::character::SimEvent::EmitterPlaySound(s));
            }
        }
        item.is_jammed()
    }

    /// `ISReloadWeaponAction.onShoot`'s ammunition part (the Lua the game
    /// runs on `OnWeaponSwingHitPoint`): spends a round, chambers the next
    /// and checks for a jam. Here so the Rust driver and tests can fire
    /// without Lua; the game runs the Lua.
    pub fn on_shoot_ammo(&mut self, sm: &ScriptManager, chars: &Characters, ctx: &dyn SimContext, id: i32) {
        let Some(w) = self.items.item(id).and_then(|i| FirearmStats::of(sm, &i.borrow())) else { return };
        if !w.ranged {
            return;
        }
        let item_ref = self.items.item(id).expect("weapon present above");
        let mut item = item_ref.borrow_mut();
        if w.have_chamber {
            item.set_round_chambered(false, w.have_chamber);
            item.set_spent_round_chambered(true);
        }
        if !w.rack_after_shoot {
            if !w.manually_remove_spent_rounds {
                item.set_spent_round_chambered(false);
                if let Some(s) = &w.shell_fall_sound {
                    ctx.event(crate::character::SimEvent::EmitterPlaySound(s.clone()));
                }
            }
            if item.get_current_ammo_count() >= w.ammo_per_shoot {
                if w.have_chamber {
                    item.set_round_chambered(true, w.have_chamber);
                }
                if !ctx.is_client() {
                    let n = item.get_current_ammo_count() - w.ammo_per_shoot;
                    item.set_current_ammo_count(n);
                }
                if w.jam_gun_chance > 0.0 {
                    drop(item);
                    self.check_jam(sm, chars, ctx, id, false);
                }
            }
        }
        if w.manually_remove_spent_rounds {
            let item = &mut *item_ref.borrow_mut();
            let n = item.get_spent_round_count() + w.ammo_per_shoot;
            item.set_spent_round_count(n, w.max_ammo);
        }
    }
}
