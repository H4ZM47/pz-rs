//! `CombatManager`'s ranged paths: the ballistics branch of
//! `calcValidTargets`, `calculateHitInfoList` / `calculateHitListWeapon`
//! for ranged weapons with `processBallisticsTargets`, `filterTargetsByZ`
//! and the `RangeTargetComparator`, `fireWeapon` (tracers and shotgun
//! spread), the ranged `attackCollisionCheck` (hit roll, piercing, damage,
//! `processHit`, `applyRangeHitLocationDamage`, `splash`) and
//! `setAimingDelay`.
//!
//! Vehicles, windows and barricades as targets are not ported
//! (TODO(vehicles), TODO(world objects)): the hit list holds characters.

use pz_anim::AnimationVariableSource;
use pz_inventory::shared::ItemRef;
use pz_inventory::InventoryItem;
use pz_scripts::{ScriptManager, ScriptResult};

use super::ballistics::{v3_distance, v3_normalize, BallisticsController};
use super::hit_chance::TargetInfo;
use super::physics::{RagdollBodyPart, BULLET_Z_SCALE};
use super::reaction::HitWeapon;
use super::weapon::FirearmStats;
use super::CombatManagerState;
use crate::character::combat::{config, WeaponStats};
use crate::character::state::{StateCx, TestResults};
use crate::character::states::ids;
use crate::character::states::swipe::{HitInfo, ATTACKED, LOWER_CONDITION};
use crate::character::{GameCharacter, SimContext, SimEvent};
use crate::Characters;

/// `IsoUtils.DistanceToSquared(x0, y0, z0, x1, y1, z1)`: float
/// differences squared and summed in double.
pub fn iso_dist_sq3(a: (f32, f32, f32), b: (f32, f32, f32)) -> f32 {
    let d = |p: f32, q: f32| {
        let v = (q - p) as f64;
        v * v
    };
    (d(a.0, b.0) + d(a.1, b.1) + d(a.2, b.2)) as f32
}

/// `CombatManager.isPointWithinDistance(start, end, target, threshold)`.
pub fn is_point_within_distance(start: [f32; 3], end: [f32; 3], target: [f32; 3], threshold: f32) -> bool {
    let ab = [end[0] - start[0], end[1] - start[1], end[2] - start[2]];
    let ap = [target[0] - start[0], target[1] - start[1], target[2] - start[2]];
    let cx = ap[1] * ab[2] - ap[2] * ab[1];
    let cy = ap[2] * ab[0] - ap[0] * ab[2];
    let cz = ap[0] * ab[1] - ap[1] * ab[0];
    let cross_sq = cx * cx + cy * cy + cz * cz;
    let ab_sq = ab[0] * ab[0] + ab[1] * ab[1] + ab[2] * ab[2];
    let dot = ab[0] * ap[0] + ab[1] * ap[1] + ab[2] * ap[2];
    cross_sq <= threshold * threshold * ab_sq && dot > 0.0
}

/// `PZMath.closestVector3(l0, l1, p)`.
pub fn closest_vector3(a: [f32; 3], b: [f32; 3], p: [f32; 3]) -> [f32; 3] {
    let ab = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
    let ap = [p[0] - a[0], p[1] - a[1], p[2] - a[2]];
    let d_apab = ap[0] * ab[0] + ap[1] * ab[1] + ap[2] * ab[2];
    let d_abab = ab[0] * ab[0] + ab[1] * ab[1] + ab[2] * ab[2];
    // `Math.max(0, Math.min(1, t))`: NaN stays NaN.
    let q = d_apab / d_abab;
    let t = if q.is_nan() { q } else { q.clamp(0.0, 1.0) };
    [a[0] + t * ab[0], a[1] + t * ab[1], a[2] + t * ab[2]]
}

/// `PZMath.fastfloor`.
fn fastfloor(v: f32) -> i32 {
    let i = v as i32;
    if v < i as f32 {
        i - 1
    } else {
        i
    }
}

/// What the range comparator reads of a hit target.
#[derive(Clone, Copy, Debug, Default)]
struct RangeKey {
    zombie: bool,
    prone: bool,
    getting_up: bool,
    crawling: bool,
}

/// `RangeTargetComparator.compare` (`MeleeTargetComparator` differs only in
/// how "getting up" is read).
fn range_compare(a: (&HitInfo, RangeKey), b: (&HitInfo, RangeKey)) -> std::cmp::Ordering {
    use std::cmp::Ordering as O;
    let (o1, k1) = a;
    let (o2, k2) = b;
    if k1.zombie && k2.zombie {
        if k1.getting_up && !k2.getting_up && k2.prone {
            return O::Less;
        }
        if !k1.getting_up && k1.prone && k2.getting_up {
            return O::Greater;
        }
        if k1.prone && k2.prone {
            if k1.crawling && !k2.crawling {
                return O::Less;
            }
            if !k1.crawling && k2.crawling {
                return O::Greater;
            }
        }
    }
    if o1.dist_sq > o2.dist_sq {
        return O::Greater;
    }
    if o2.dist_sq > o1.dist_sq {
        return O::Less;
    }
    O::Equal
}

impl<I: 'static> GameCharacter<I> {
    /// `getUseHandWeapon()`'s item, else the primary hand weapon
    /// (`getAttackingWeapon()` without the hand-to-hand case).
    fn use_hand_weapon_item(&self) -> Option<ItemRef> {
        if let Some(i) = self.vars.use_hand_weapon.and_then(|id| self.items.item(id)) {
            return Some(i);
        }
        self.items.primary_hand_item().filter(|i| crate::character::items::is_hand_weapon(&i.borrow()))
    }

    /// `getAttackingWeapon()`: the bare hands for a hand-to-hand attack,
    /// else the weapon in use, else the primary hand weapon.
    pub fn attacking_weapon(&self, sm: &ScriptManager) -> (WeaponStats, Option<InventoryItem>) {
        let hand_to_hand = self.vars.do_shove || self.vars.do_grapple;
        if !hand_to_hand {
            if let Some(item) = self.use_hand_weapon_item() {
                let item = &*item.borrow();
                if let Some(w) = WeaponStats::of(sm, item) {
                    return (w, Some(item.clone()));
                }
            }
        }
        (super::super::states::swipe::bare_hands_stats(), self.bare_hands.clone())
    }

    /// `getUseHandWeapon().isRackAfterShoot()` (false without one).
    pub fn use_hand_weapon_rack_after_shoot(&self, sm: &ScriptManager) -> bool {
        self.vars.use_hand_weapon.and_then(|id| self.items.item(id)).and_then(|i| FirearmStats::of(sm, &i.borrow())).is_some_and(|f| f.rack_after_shoot)
    }

    /// `setUseHandWeapon(weapon)` by item id.
    pub fn set_use_hand_weapon(&mut self, id: Option<i32>) {
        self.vars.use_hand_weapon = id;
    }

    /// `CombatManager.calculateBallistics(owner, range)`: the muzzle ray in
    /// the combat statics.
    pub fn calculate_ballistics(&self, st: &mut CombatManagerState, muzzle: Option<&super::ballistics::MuzzleAttachment>, range: f32) {
        if self.ballistics_controller.is_none() {
            return;
        }
        let pos = [self.vars.x, self.vars.y, self.vars.z];
        if let Some((p, d, _)) = BallisticsController::calculate_muzzle_position(pos, muzzle) {
            st.ballistics_start_position = p;
            st.ballistics_direction_vector = d;
        }
        let d = v3_normalize(st.ballistics_direction_vector);
        st.ballistics_direction_vector = d;
        let s = st.ballistics_start_position;
        st.ballistics_end_position = [s[0] + d[0] * range, s[1] + d[1] * range, s[2] + d[2] * range];
    }

    /// `isHittableBallisticsTarget(owner, range, threshold, pos)`: near the
    /// reticle, or near the muzzle ray.
    fn is_hittable_ballistics_target(&self, st: &CombatManagerState, threshold: f32, pos: [f32; 3]) -> bool {
        let reticle = self.ballistics_controller.as_ref().is_some_and(|c| v3_distance(c.iso_aiming_position, pos) < threshold);
        reticle || is_point_within_distance(st.ballistics_start_position, st.ballistics_end_position, pos, threshold)
    }

    /// `removeTargetObjects(obj, owner, isMelee)` for a character: true to
    /// drop it.
    pub(crate) fn remove_target_object(&self, t: &GameCharacter<I>, ctx: &dyn SimContext, on_square: bool) -> bool {
        if t.is_dead() || (t.is_god_mod(ctx) && !t.is_zombie()) {
            return true;
        }
        if t.is_zombie() && (t.is_current_state_id(ids::FAKE_DEAD_ZOMBIE) || t.zombie.reanimated_for_grapple_only) {
            return true;
        }
        if t.id == self.id || !on_square {
            return true;
        }
        !check_pvp(self, t, ctx)
    }

    /// The range comparator's view of a target.
    fn range_key(t: &GameCharacter<I>) -> RangeKey {
        RangeKey {
            zombie: t.is_zombie(),
            prone: t.is_prone(),
            getting_up: t.is_current_state_id(ids::ZOMBIE_GET_UP),
            crawling: t.vars.crawling,
        }
    }

    /// `calculateHitListWeapon(owner, attackVars, hitList)` for a ranged
    /// weapon, then `processBallisticsTargets`.
    fn calculate_hit_list_ranged(&mut self, cx: &mut StateCx<'_, I>, w: &FirearmStats, list: &mut Vec<HitInfo>) {
        let (sm, chars, ctx) = (cx.sm, cx.chars, cx.ctx);
        let ws = self.items.item(w.id).and_then(|i| WeaponStats::of(sm, &i.borrow()));
        let range_mod = ws.as_ref().map_or(1.0, |ws| self.weapon_range_mod(sm, chars, ws));
        let weapon_range = self.firearm_max_range(chars, w) * range_mod;
        let threshold = config::BALLISTICS_CONTROLLER_DISTANCE_THRESHOLD;
        let st = chars.combat.borrow().clone();
        let candidates = cx.world.characters_near(self.vars.x, self.vars.y, self.vars.z, weapon_range + 4.0);
        let from = self.square_xyz();
        let mut temp = st.temp_vector4f;
        for id in candidates {
            let to = {
                let Some(t) = cx.world.character_mut(id) else { continue };
                t.square_xyz()
            };
            let on_square = cx.world.square(to.0, to.1, to.2).is_some();
            let line = cx.world.los_line_clear(from, to, false);
            let Some(t) = cx.world.character_mut(id) else { continue };
            if w.aimed_firearm && v3_distance(st.ballistics_start_position, [t.vars.x, t.vars.y, t.vars.z]) > weapon_range {
                continue;
            }
            if !w.aimed_firearm && !self.vars.aim_at_floor && t.is_prone() {
                continue;
            }
            if self.remove_target_object(t, ctx, on_square) {
                continue;
            }
            if !w.aimed_firearm {
                let hittable = Some(id) == self.attack_vars.target_on_ground
                    || t.vars.shootable && t.is_standing() && !self.attack_vars.aim_at_floor
                    || t.vars.shootable && t.is_prone() && self.attack_vars.aim_at_floor;
                if !hittable {
                    continue;
                }
            }
            let pos = [t.vars.x, t.vars.y, t.vars.z];
            temp[0] = pos[0];
            temp[1] = pos[1];
            temp[2] = pos[2];
            if !self.is_hittable_ballistics_target(&st, threshold, pos) {
                continue;
            }
            if line == TestResults::Blocked || line == TestResults::ClearThroughClosedDoor {
                continue;
            }
            let dist_sq = iso_dist_sq3(
                (self.vars.x, self.vars.y, fastfloor(self.vars.z) as f32 * 3.0),
                (pos[0], pos[1], fastfloor(t.vars.z) as f32 * 3.0),
            );
            list.push(HitInfo { target: id, dot: temp[3], dist_sq, x: pos[0], y: pos[1], z: pos[2], chance: 0 });
        }
        chars.combat.borrow_mut().temp_vector4f = temp;
        if !list.is_empty() && w.ranged {
            self.process_ballistics_targets(cx, w, list);
        }
    }

    /// `processBallisticsTargets(owner, weapon, hitList)`.
    fn process_ballistics_targets(&mut self, cx: &mut StateCx<'_, I>, w: &FirearmStats, list: &mut Vec<HitInfo>) {
        let (chars, ctx) = (cx.chars, cx.ctx);
        for h in list.iter() {
            let Some(t) = cx.world.character_mut(h.target) else { continue };
            if t.is_animal() {
                continue;
            }
            let mut bt = t.ballistics_target.take().unwrap_or_default();
            let (pos, dir) = ([t.vars.x, t.vars.y, t.vars.z], t.direction_angle());
            let pelvis = cx.world.ragdoll_pelvis(h.target);
            let skeleton = cx.world.ballistics_skeleton(h.target);
            if let Some(p) = cx.world.physics() {
                bt.add(h.target, pos, pelvis, dir, skeleton.as_ref(), p);
            }
            if let Some(t) = cx.world.character_mut(h.target) {
                t.ballistics_target = Some(bt);
            }
        }
        let Some(mut c) = self.ballistics_controller.take() else { return };
        if w.ranged {
            ctx.event(SimEvent::ReticleMode { player_index: self.seen.player_index });
            let range = self.firearm_max_range(chars, w);
            c.set_range(self.id, range, cx.world.physics());
            if w.range_falloff {
                let scale = chars.combat.borrow().ballistics_spread_angle_scale;
                c.get_camera_targets(self.id, range + 0.5, true, cx.world.physics());
                c.get_spread_data(self.id, range, w.projectile_spread * scale, w.projectile_weight_center, w.projectile_count, cx.world.physics());
            } else {
                c.get_camera_targets(self.id, range + 0.5, true, cx.world.physics());
                c.get_targets(self.id, range, cx.world.physics());
            }
        }
        let mut i = 0;
        while i < list.len() {
            let id = list[i].target;
            if let Some(p) = c.target_hit_location(id) {
                (list[i].x, list[i].y, list[i].z) = (p[0], p[1], p[2]);
            }
            if let Some(p) = c.camera_target_hit_location(id) {
                (list[i].x, list[i].y, list[i].z) = (p[0], p[1], p[2]);
            }
            let has_target = cx.world.character_mut(id).is_some_and(|t| t.ballistics_target.is_some());
            if !has_target {
                i += 1;
                continue;
            }
            if c.is_valid_target(id) || c.is_valid_cached_target(id) {
                i += 1;
                continue;
            }
            list.remove(i);
        }
        if w.range_falloff {
            let n = list.len();
            for i in 0..n {
                let id = list[i].target;
                let has_target = cx.world.character_mut(id).is_some_and(|t| t.ballistics_target.is_some());
                if !has_target {
                    continue;
                }
                let count = c.spread_count(id);
                if count <= 1 {
                    continue;
                }
                for _ in 0..count - 1 {
                    let mut copy = list[i].clone();
                    copy.chance = 0;
                    list.push(copy);
                }
            }
        }
        self.ballistics_controller = Some(c);
    }

    /// `filterTargetsByZ(ownerZ, hitList)`.
    fn filter_targets_by_z(cx: &mut StateCx<'_, I>, owner_z: f32, list: &mut Vec<HitInfo>) {
        let z_of = |cx: &mut StateCx<'_, I>, h: &HitInfo| cx.world.character_mut(h.target).map_or(h.z, |t| t.vars.z);
        let mut min = f32::MAX;
        let mut best: Option<usize> = None;
        for (i, h) in list.iter().enumerate() {
            let dz = (z_of(cx, h) - owner_z).abs();
            if dz < min {
                min = dz;
                best = Some(i);
            }
        }
        let Some(b) = best else { return };
        let best_z = list[b].z;
        let mut i = list.len();
        while i > 0 {
            i -= 1;
            if i == b {
                continue;
            }
            if (z_of(cx, &list[i]) - best_z).abs() > 0.5 {
                list.remove(i);
            }
        }
    }

    /// `calculateHitInfoList(owner)` for a ranged weapon: fills
    /// [`GameCharacter::hit_info_list`] with the targets, nearest first,
    /// each with its hit chance.
    pub fn calculate_ranged_hit_info_list(&mut self, cx: &mut StateCx<'_, I>, w: &FirearmStats) -> ScriptResult {
        let sm = cx.sm;
        self.hit_info_list.clear();
        let ws = self.items.item(w.id).and_then(|i| WeaponStats::of(sm, &i.borrow()));
        let mut max_hit = ws.as_ref().map_or(1, |ws| ws.max_hit_count);
        if self.attack_vars.do_shove {
            max_hit = if self.weapon_type_of(sm) != crate::character::combat::WeaponType::Unarmed { 3 } else { 1 };
        }
        if max_hit <= 0 {
            return Ok(());
        }
        let mut list = Vec::new();
        self.calculate_hit_list_ranged(cx, w, &mut list);
        if w.ranged && !w.range_falloff {
            Self::filter_targets_by_z(cx, self.vars.z, &mut list);
        }
        let keys: Vec<RangeKey> = list.iter().map(|h| cx.world.character_mut(h.target).map_or(RangeKey::default(), |t| Self::range_key(t))).collect();
        let mut keyed: Vec<(HitInfo, RangeKey)> = list.into_iter().zip(keys).collect();
        keyed.sort_by(|a, b| range_compare((&a.0, a.1), (&b.0, b.1)));
        let list: Vec<HitInfo> = keyed.into_iter().map(|(h, _)| h).collect();
        let mut out = Vec::new();
        let mut reference_angle = -1.0f64;
        for h in list {
            let Some(t) = cx.world.character_mut(h.target) else { continue };
            let (tx, ty, animal) = (t.vars.x, t.vars.y, t.is_animal());
            let valid = self.ballistics_controller.as_ref().is_some_and(|c| c.is_valid_target(h.target));
            if !animal && !valid {
                continue;
            }
            if w.piercing_bullets {
                let angle = ((self.vars.y - ty) as f64).atan2((tx - self.vars.x) as f64).to_degrees();
                if reference_angle < 0.0 {
                    reference_angle = angle;
                } else if (reference_angle - angle).abs() >= 1.0 {
                    continue;
                }
            }
            out.push(h);
            if out.len() as i32 >= max_hit {
                break;
            }
        }
        let vehicle = cx.world.vehicle_aim(self.id);
        for h in out.iter_mut() {
            let info = {
                let to = match cx.world.character_mut(h.target) {
                    Some(t) => t.square_xyz(),
                    None => continue,
                };
                let on_square = cx.world.square(to.0, to.1, to.2).is_some();
                let t = cx.world.character_mut(h.target).expect("present above");
                TargetInfo::of(cx.sm, cx.ctx, t, on_square)
            };
            let data = self.calculate_hit_chance_data(cx, w, Some((h.dist_sq, &info)), vehicle.as_ref());
            h.chance = data.hit_chance as i32;
            if cx.ctx.debug_always_hit_target() {
                h.chance = config::MAXIMUM_TO_HIT_CHANCE as i32;
            }
        }
        self.hit_info_list = out;
        Ok(())
    }

    /// `addTracerEffect(chr, range[, end, square])`.
    fn tracer(&self, ctx: &dyn SimContext, range: f32, end: Option<[f32; 3]>, square: Option<(i32, i32, i32)>) {
        ctx.event(SimEvent::BulletTracer { chr: self.id, range, end: end.map(|e| (e[0], e[1], e[2])), square });
    }

    /// `missedShot(chr, range, firingVector)`.
    fn missed_shot(&self, cx: &mut StateCx<'_, I>, st: &CombatManagerState, range: f32, firing: [f32; 3]) {
        let s = st.ballistics_start_position;
        let from = (fastfloor(s[0]), fastfloor(s[1]), fastfloor(s[2]));
        let to = (fastfloor(firing[0]), fastfloor(firing[1]), fastfloor(firing[2]));
        let hit = cx.world.first_blocking_square(from, to);
        if hit.result == TestResults::Clear || hit.result == TestResults::ClearThroughClosedDoor {
            self.tracer(cx.ctx, range, None, None);
        } else {
            let p = closest_vector3(s, firing, [hit.position.0, hit.position.1, firing[2]]);
            self.tracer(cx.ctx, range, Some(p), hit.square);
        }
    }

    /// `fireWeapon(weapon, chr)`: the muzzle ray, the shotgun spread rays and
    /// the tracers.
    pub fn fire_weapon(&mut self, cx: &mut StateCx<'_, I>, w: &FirearmStats) {
        let chars = cx.chars;
        let range = self.firearm_max_range(chars, w);
        let muzzle = cx.world.muzzle_attachment(self.id);
        let pos = [self.vars.x, self.vars.y, self.vars.z];
        let mut st = chars.combat.borrow().clone();
        let offset = match BallisticsController::calculate_muzzle_position(pos, muzzle.as_ref()) {
            Some((p, d, off)) => {
                st.ballistics_start_position = p;
                st.ballistics_direction_vector = d;
                off
            }
            None => 0.0,
        };
        st.ballistics_spread_angle_scale = 1.0 / (1.0 + offset);
        st.ballistics_direction_vector = v3_normalize(st.ballistics_direction_vector);
        let (s, d) = (st.ballistics_start_position, st.ballistics_direction_vector);
        let firing = [s[0] + d[0] * range, s[1] + d[1] * range, s[2] + d[2] * range];
        *chars.combat.borrow_mut() = st.clone();
        if w.range_falloff {
            let Some(mut c) = self.ballistics_controller.take() else { return };
            if !c.has_spread_data() {
                c.set_range(self.id, range, cx.world.physics());
                let reticle = cx.world.aim_reticle_iso(self.id);
                let wr = self.attacking_weapon_range(cx.sm, chars);
                c.update(self.id, pos, muzzle.as_ref(), reticle, wr, cx.ctx.is_server(), cx.ctx.is_debug(), cx.world.physics());
                let spread = w.projectile_spread * st.ballistics_spread_angle_scale;
                c.get_spread_data(self.id, range, spread, w.projectile_weight_center, w.projectile_count, cx.world.physics());
            }
            let n = c.number_of_spread_data;
            let data = c.spread_data;
            self.ballistics_controller = Some(c);
            for i in 0..n.max(0) as usize {
                let id = data[i * 4];
                let x = data[i * 4 + 1];
                let z = data[i * 4 + 2] / BULLET_Z_SCALE;
                let y = data[i * 4 + 3];
                if id == 0.0 {
                    let from = (fastfloor(s[0]), fastfloor(s[1]), fastfloor(s[2]));
                    let hit = cx.world.first_blocking_square(from, (fastfloor(x), fastfloor(y), fastfloor(z)));
                    if hit.result == TestResults::Clear || hit.result == TestResults::ClearThroughClosedDoor {
                        self.tracer(cx.ctx, range, Some([x, y, z]), None);
                        continue;
                    }
                    let p = closest_vector3(s, [x, y, z], [hit.position.0, hit.position.1, z]);
                    self.tracer(cx.ctx, range, Some(p), hit.square);
                    continue;
                }
                self.tracer(cx.ctx, range, Some([x, y, z]), None);
            }
        } else if let Some(h) = self.hit_info_list.first() {
            let end = [h.x, h.y, h.z];
            self.tracer(cx.ctx, range, Some(end), None);
        } else {
            self.missed_shot(cx, &st, range, firing);
        }
    }

    /// `addCombatMuscleStrain(weapon, hitCount)` for an aimed firearm.
    fn add_firearm_muscle_strain(&mut self, sm: &ScriptManager, chars: &Characters, ctx: &dyn SimContext, w: &FirearmStats) -> ScriptResult {
        let strength = self.perk_level(chars.perks.constant("Strength").as_ref());
        let skill = self.perk_level(chars.perks.constant("Aiming").as_ref());
        let strain = self.items.item(w.id).and_then(|i| crate::character::items::script_item(sm, &i.borrow())).map_or(1.0, |s| s.borrow().strain_modifier);
        let strain_mod = (1.0 - skill as f32 * 0.075) * strain;
        let mut value = self.firearm_recoil_delay(chars, w) as f32
            * config::FIREARM_RECOIL_MUSCLE_STRAIN_MODIFIER
            * strain_mod
            * ((15 - strength) as f32 / 10.0);
        if w.fire_mode.as_deref().is_some_and(|m| m.eq_ignore_ascii_case("Auto")) {
            value *= 0.5;
        }
        value *= ctx.muscle_strain_factor() as f32;
        if value == 0.0 {
            return Ok(());
        }
        let types = &chars.sim.body_part_types;
        let mut add = |field: &str, v: f32| -> ScriptResult {
            let p = &mut self.body_damage.parts[types.index(field)?];
            p.stiffness = super::hit_chance::clamp(p.stiffness + v, 0.0, 100.0);
            Ok(())
        };
        add("Hand_R", value)?;
        add("ForeArm_R", value)?;
        if self.items.secondary_hand_item == Some(w.id) {
            add("UpperArm_R", value)?;
            add("Hand_L", value * 0.1)?;
            add("ForeArm_L", value * 0.1)?;
        }
        Ok(())
    }

    /// `CombatManager.splash(target, weapon, owner)`: `self` is the owner,
    /// `target` the hit character.
    pub fn splash(&self, ctx: &dyn SimContext, target: &GameCharacter<I>, splat_number: i32) {
        let level = ctx.blood_level();
        if level > 1 {
            let mut spn = splat_number.max(1);
            if ctx.is_last_stand() {
                spn *= 3;
            }
            match level {
                2 => spn /= 2,
                4 => spn *= 2,
                5 => spn *= 5,
                _ => {}
            }
            for _ in 0..spn {
                ctx.event(SimEvent::CharacterSplatBlood { chr: target.id, dist: 3, alpha: 0.3 });
            }
        }
        let (mut rand, mut repeat) = (3, 7);
        match level {
            1 => repeat = 0,
            2 => {
                repeat = 4;
                rand = 5;
            }
            4 => {
                repeat = 10;
                rand = 2;
            }
            5 => {
                repeat = 15;
                rand = 0;
            }
            _ => {}
        }
        if level > 1 {
            ctx.event(SimEvent::CharacterSplatBloodFloorBig { chr: target.id });
        }
        let mut dz = 0.5f32;
        if target.is_zombie() && (target.vars.crawling || target.is_current_state_id(ids::ZOMBIE_ON_GROUND)) {
            dz = 0.2;
        }
        let mut ax = ctx.rand_next_float(1.5, 5.0);
        let mut ay = ctx.rand_next_float(1.5, 5.0);
        if self.is_player() && self.vars.do_shove {
            ax = ctx.rand_next_float(0.0, 0.5);
            ay = ctx.rand_next_float(0.0, 0.5);
        }
        if repeat > 0 {
            ctx.event(SimEvent::BloodSplatterSound { chr: target.id });
        }
        for _ in 0..repeat {
            if ctx.rand_next(rand) != 0 {
                continue;
            }
            // `new IsoZombieGiblets(...)`: its constructor's draws.
            let rx = ctx.rand_next(4000) as f32 / 10000.0 - 0.2;
            let ry = ctx.rand_next(4000) as f32 / 10000.0 - 0.2;
            let invisible = ctx.rand_next(3) != 0;
            ctx.event(SimEvent::Giblet {
                x: target.vars.x,
                y: target.vars.y,
                z: target.vars.z + dz,
                vel_x: target.vars.hit_dir_x * ax + rx,
                vel_y: target.vars.hit_dir_y * ay + ry,
                invisible,
            });
        }
    }

    /// `applyRangeHitLocationDamage(target, weapon, bodyPart, damageSplit)`
    /// on `self` (the target).
    pub fn range_hit_location_damage(&self, cx: &StateCx<'_, I>, w: &WeaponStats, part: i32, mut damage_split: f32) -> ScriptResult<f32> {
        let ctx = cx.ctx;
        if RagdollBodyPart::is_head(part) {
            if !ctx.is_client() {
                for (p, s, b, a) in [
                    (crate::character::items::blood_part::HEAD, true, true, true),
                    (crate::character::items::blood_part::TORSO_UPPER, true, false, false),
                    (crate::character::items::blood_part::UPPER_ARM_L, true, false, false),
                    (crate::character::items::blood_part::UPPER_ARM_R, true, false, false),
                ] {
                    ctx.event(SimEvent::CharacterAddBlood { chr: self.id, part: p as i32, scratched: s, bitten: b, all_layers: a });
                }
            }
            damage_split *= config::HEAD_HIT_DAMAGE_SPLIT_MODIFIER;
        }
        if RagdollBodyPart::is_leg(part) || RagdollBodyPart::is_arm(part) {
            damage_split *= config::LEG_HIT_DAMAGE_SPLIT_MODIFIER;
        }
        let secondary = crate::character::body_part::next_bool(ctx, 2);
        let field = RagdollBodyPart::body_part_type_field(part, secondary);
        let part_hit = if field == "MAX" { cx.chars.sim.body_part_types.max_index() as i32 } else { cx.chars.sim.body_part_types.index(field)? as i32 };
        let def = self.total_defense(cx.sm, ctx, part_hit, w);
        let damage = damage_split * (1.0 - def / 100.0).abs();
        ctx.event(SimEvent::CombatStatistic { name: "Bullet Damage", value: damage });
        Ok(damage)
    }

    /// `CombatManager.setAimingDelay(player, weapon)`.
    pub fn set_aiming_delay_after_shot(&mut self, chars: &Characters, w: &FirearmStats) {
        let delay = self.vars.aiming_delay
            + (self.firearm_recoil_delay(chars, w) as f32 * config::POST_SHOT_AIMING_DELAY_RECOIL_MODIFIER
                + w.aiming_time as f32 * config::POST_SHOT_AIMING_DELAY_AIMING_MODIFIER);
        let max = self
            .items
            .primary_hand_item()
            .filter(|i| crate::character::items::is_hand_weapon(&i.borrow()))
            .map_or(0.0, |_| w.aiming_time as f32);
        self.vars.aiming_delay = super::hit_chance::clamp(delay, 0.0, max);
    }

    /// `CombatManager.releaseBallisticsTargets(chr)`.
    pub fn release_ballistics_targets(&self, cx: &mut StateCx<'_, I>) {
        for h in &self.hit_info_list {
            let Some(t) = cx.world.character_mut(h.target) else { continue };
            if !t.is_zombie() {
                continue;
            }
            let Some(mut bt) = t.ballistics_target.take() else { continue };
            bt.release(h.target, cx.world.physics());
        }
    }

    /// `CombatManager.attackCollisionCheck(owner, weapon, swipeState, NONE)`
    /// for a ranged weapon.
    pub fn ranged_attack_collision_check(&mut self, cx: &mut StateCx<'_, I>, weapon_item: &InventoryItem) -> ScriptResult {
        let (sm, chars) = (cx.sm, cx.chars);
        let Some(w) = FirearmStats::of(sm, weapon_item) else { return Ok(()) };
        let Some(ws) = WeaponStats::of(sm, weapon_item) else { return Ok(()) };
        if self.vars.performing_shove_animation {
            self.vars.do_shove = true;
        }
        if self.vars.performing_grapple_grab_animation {
            self.vars.do_grapple = true;
        }
        cx.ctx.event(SimEvent::WeaponSwingHitPoint);
        let mut ignore_damage = self.vars.do_shove && !self.vars.shove_stomp_anim;
        let mut process_condition_loss = false;
        self.attack_vars.weapon = Some(w.id);
        self.attack_vars.target_on_ground = self.vars.target_on_ground;
        self.attack_vars.aim_at_floor = self.vars.aim_at_floor;
        self.attack_vars.do_shove = self.vars.do_shove || self.vars.performing_shove_animation;
        self.calculate_ranged_hit_info_list(cx, &w)?;
        if w.aimed_firearm {
            // `EffectsManager.startMuzzleFlash(owner, 1)`: `Rand.NextBool(1)`
            // (always true), then the flash light and model effect.
            if crate::character::body_part::next_bool(cx.ctx, 1) {
                cx.ctx.event(SimEvent::MuzzleFlash { chr: self.id });
            }
            if let Some(mut c) = self.ballistics_controller.take() {
                let pos = [self.vars.x, self.vars.y, self.vars.z];
                let muzzle = cx.world.muzzle_attachment(self.id);
                let reticle = cx.world.aim_reticle_iso(self.id);
                let wr = self.attacking_weapon_range(sm, chars);
                c.update(self.id, pos, muzzle.as_ref(), reticle, wr, cx.ctx.is_server(), cx.ctx.is_debug(), cx.world.physics());
                self.ballistics_controller = Some(c);
            }
            self.fire_weapon(cx, &w);
            cx.ctx.event(SimEvent::CombatStatistic { name: "Shots Fired", value: 1.0 });
        }
        let hits = self.hit_info_list.clone();
        let hit_count = hits.len() as i32;
        if !cx.ctx.is_client() {
            self.process_weapon_endurance(sm, chars, &ws)?;
            if w.aimed_firearm && !self.vars.do_shove && !self.vars.performing_stomp_animation {
                self.add_firearm_muscle_strain(sm, chars, cx.ctx, &w)?;
            }
        }
        self.vars.last_hit_count = hit_count;
        chars.combat.borrow_mut().dot_list.clear();
        if hit_count == 0 && !self.vars.do_shove {
            if let Some(s) = self.vars.click_sound.clone() {
                cx.ctx.event(SimEvent::EmitterPlaySound(s));
                self.vars.recoil_delay = config::RECOIL_DELAY;
            }
        }
        let mut shotgun_xp_awarded = false;
        let hit_weapon = HitWeapon::of(sm, weapon_item);
        for h in &hits {
            let mut ignore_hit_count_damage = false;
            let Some(t) = cx.world.character_mut(h.target) else { continue };
            let target_zombie = t.is_zombie();
            let target_player = t.is_player();
            let (tx, ty, tz) = (t.vars.x, t.vars.y, t.vars.z);
            let hit = cx.ctx.rand_next(100) <= h.chance;
            if !hit {
                cx.ctx.event(SimEvent::CombatStatistic { name: "Bullets Chance Missed", value: 1.0 });
                let mode = cx.ctx.firearm_use_damage_chance();
                if mode != 3 && (mode != 2 || !target_zombie) {
                    continue;
                }
                cx.ctx.event(SimEvent::CombatStatistic { name: "Bullets Damage Ignored", value: 1.0 });
                ignore_hit_count_damage = true;
            }
            cx.ctx.event(SimEvent::CombatStatistic { name: "Bullets Chance Hit", value: 1.0 });
            let (tpx, tpy) = crate::character::zombie::math::normalize(tx - self.vars.x, ty - self.vars.y);
            let (lx, ly) = self.look_vector();
            // `Vector2.tangent()`: a rotation by 90 degrees in double.
            let r = 90.0f64.to_radians();
            let (ax, ay) = (((lx as f64) * r.cos() - (ly as f64) * r.sin()) as f32, ((lx as f64) * r.sin() + (ly as f64) * r.cos()) as f32);
            let mut min = ws.min_damage;
            let mut max = ws.max_damage;
            if !w.range_falloff {
                let dot = ax * tpx + ay * tpy;
                let mut st = chars.combat.borrow_mut();
                let pierced = st.dot_list.iter().any(|d| (dot - d).abs() < 1.0e-4);
                if st.dot_list.is_empty() {
                    st.dot_list.push(dot);
                }
                if pierced {
                    min /= config::PIERCING_BULLET_DAMAGE_REDUCTION;
                    max /= config::PIERCING_BULLET_DAMAGE_REDUCTION;
                }
            }
            if self.get_variable_boolean("PistolWhipAnim") {
                if let Some(s) = &w.zombie_hit_sound {
                    cx.ctx.event(SimEvent::PlaySound(s.clone()));
                }
            }
            let crit_close_kill = self.attack_vars.close_kill && self.vars.critical_hit;
            if !(self.attack_vars.aim_at_floor || crit_close_kill || self.vars.do_shove || (self.is_player() && !self.is_local_player(cx.ctx))) {
                if let Some(s) = &w.zombie_hit_sound {
                    cx.ctx.event(SimEvent::BulletHitSound { chr: h.target, sound: s.clone(), hollow: false });
                }
            }
            let t = cx.world.character_mut(h.target).expect("present above");
            if target_zombie {
                let (fx, fy) = crate::character::zombie::math::normalize(t.vars.forward_x, t.vars.forward_y);
                t.vars.hit_from_behind = tpx * fx + tpy * fy > 0.5;
            }
            if target_zombie && t.is_current_state_id(ids::ZOMBIE_ON_GROUND) {
                t.vars.reanimate_timer += cx.ctx.rand_next(10) as f32;
            }
            if target_zombie && t.is_current_state_id(ids::ZOMBIE_GET_UP) {
                t.vars.reanimate_timer = (cx.ctx.rand_next(60) + 30) as f32;
            }
            let damage = cx.ctx.rand_next_float(min, max);
            let mut damage_split = damage;
            let mut range_del = if w.range_falloff { 1.0 } else { 0.5 };
            range_del *= 2.0;
            if range_del < 0.3 {
                range_del = 1.0;
            }
            if damage_split < 0.0 {
                damage_split = 0.1;
            }
            if self.vars.aim_at_floor && self.vars.do_shove {
                damage_split = cx.ctx.rand_next_float(0.7, 1.0) + self.perk_level(chars.perks.constant("Strength").as_ref()) as f32 * 0.2;
                damage_split *= 0.5;
            }
            self.vars.knockback_attack_mod = 1.0;
            if self.get_variable_string("ZombieHitReaction") == "KnifeDeath" {
                range_del *= 1000.0;
                self.vars.knockback_attack_mod = 0.0;
                cx.ctx.event(SimEvent::WorldSound { radius: 4, volume: 4 });
                self.attack_vars.close_kill = true;
                if let Some(t) = cx.world.character_mut(h.target) {
                    t.vars.close_killed = true;
                }
            } else {
                self.attack_vars.close_kill = false;
                if let Some(t) = cx.world.character_mut(h.target) {
                    t.vars.close_killed = false;
                }
                cx.ctx.event(SimEvent::WorldSound { radius: 8, volume: 8 });
                if cx.ctx.rand_next(3) == 0 || (self.vars.aim_at_floor && self.vars.do_shove) {
                    cx.ctx.event(SimEvent::WorldSound { radius: 10, volume: 10 });
                } else if cx.ctx.rand_next(7) == 0 {
                    cx.ctx.event(SimEvent::WorldSound { radius: 16, volume: 16 });
                }
            }
            let behind = {
                let t = cx.world.character_mut(h.target).expect("present above");
                t.vars.hit_from_angle = h.dot;
                (t.vars.x, t.vars.y, (t.vars.forward_x, t.vars.forward_y))
            };
            if target_zombie {
                let is_behind = self.is_behind(behind.0, behind.1, behind.2);
                let t = cx.world.character_mut(h.target).expect("present above");
                t.vars.hit_from_behind = is_behind;
                t.vars.player_attack_position = Some(t.test_dot_side(self.vars.x, self.vars.y).name().to_string());
                t.vars.hit_head_while_on_floor = 0;
                t.vars.hit_legs_while_on_floor = false;
                process_condition_loss = true;
            }
            // The ranged branch: `processHit` and the hit location.
            let Some(mut target) = cx.world.take_character(h.target) else { continue };
            let part = target.process_hit(&hit_weapon, self, cx);
            let r = target.range_hit_location_damage(cx, &ws, part, damage_split);
            let damage_split = match r {
                Ok(d) => d,
                Err(e) => {
                    cx.world.put_character(target);
                    return Err(e);
                }
            };
            if !cx.ctx.is_client() && !cx.ctx.is_server() {
                let fallen = target.items.helmet_fall_from_worn_items(sm, cx.ctx, false);
                let _ = fallen;
            }
            let on_squares = cx.world.square(fastfloor(tx), fastfloor(ty), fastfloor(tz)).is_some();
            let mut dead = false;
            if on_squares {
                target.vars.close_killed = self.attack_vars.close_kill;
                if self.is_local_player(cx.ctx) || !self.is_player() {
                    let d = target.hit(weapon_item, self, damage_split, ignore_damage, range_del, cx);
                    if let Err(e) = d {
                        cx.world.put_character(target);
                        return Err(e);
                    }
                }
                if !(ignore_hit_count_damage || cx.ctx.is_client() || cx.ctx.is_server() || w.range_falloff && shotgun_xp_awarded) {
                    cx.ctx.event(SimEvent::WeaponHitXp { target: h.target, damage: damage_split });
                    if w.range_falloff {
                        shotgun_xp_awarded = true;
                    }
                }
                let near = iso_dist_sq3((self.vars.x, self.vars.y, 0.0), (target.vars.x, target.vars.y, 0.0)) < 2.0;
                if (!self.vars.do_shove || self.vars.aim_at_floor) && near && (self.vars.z - target.vars.z).abs() < 0.5 {
                    cx.ctx.event(SimEvent::AddBlood { part: -1, scratched: false, bitten: false, all_layers: false });
                }
                process_condition_loss = true;
                dead = target.is_dead();
                if !dead && !(target_player || self.vars.do_shove && !self.vars.aim_at_floor) {
                    self.splash(cx.ctx, &target, w.splat_number);
                }
            }
            cx.world.put_character(target);
            if dead {
                self.stats.remove(&chars.sim.character_stats.field("STRESS")?, 0.02);
            }
            ignore_damage |= ignore_hit_count_damage;
        }
        let process = self.check_for_condition_loss(&ws, process_condition_loss);
        if process && !ws.is_melee() {
            // `weapon.damageCheck(0, 1f, false, true, owner)`.
            if let Some(item) = self.items.item(w.id) {
                let clc = ws.condition_lower_chance;
                if crate::character::body_part::next_bool(cx.ctx, clc) {
                    item.borrow_mut().condition -= 1;
                }
            }
        }
        LOWER_CONDITION.set(&mut self.state.params, process);
        ATTACKED.set(&mut self.state.params, true);
        Ok(())
    }
}

/// `CombatManager.checkPVP(owner, target, isMelee)` in single player: a
/// god-mode player is never a target; player against player needs coop PVP.
fn check_pvp<I: 'static>(owner: &GameCharacter<I>, target: &GameCharacter<I>, ctx: &dyn SimContext) -> bool {
    if target.is_player() && target.is_god_mod(ctx) {
        return false;
    }
    if !owner.is_player() || !target.is_player() {
        return true;
    }
    ctx.is_client() || ctx.coop_pvp()
}
