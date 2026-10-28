//! Falls and landings: `FallDamage` and `FallingConstants`, `DoLand` and
//! `handleLandingImpact` (with `IsoPlayer`'s override), `fallenOnKnees`
//! and `helmetFall`, and `IsoPlayer.OnDeath`. Each of them drops the
//! hand items to the floor where the original does
//! ([`GameCharacter::drop_hand_items_in`]).
//!
//! The landing's floor and the falling items are the world's
//! ([`StateWorld::floor_sprite_name`], [`SimEvent::FallingItem`]).
//!
//! [`StateWorld::floor_sprite_name`]: crate::character::state::StateWorld::floor_sprite_name

use super::items::blood_part;
use super::state::StateCx;
use super::{GameCharacter, SimEvent};
use crate::Characters;

/// `FallSeverity`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub enum FallSeverity {
    #[default]
    None,
    Light,
    Hard,
    Severe,
    Lethal,
}

impl FallSeverity {
    /// The constant's name.
    pub fn name(self) -> &'static str {
        match self {
            FallSeverity::None => "None",
            FallSeverity::Light => "Light",
            FallSeverity::Hard => "Hard",
            FallSeverity::Severe => "Severe",
            FallSeverity::Lethal => "Lethal",
        }
    }
}

/// `FallingConstants.IsoFallAcceleration`.
const ISO_FALL_ACCELERATION: f32 = 5.0010414;

/// `FallingConstants.getIsoImpactSpeedFromHeight(fallHeight)`.
pub fn iso_impact_speed_from_height(fall_height: f32) -> f32 {
    let impact_time = ((fall_height * 2.0 / ISO_FALL_ACCELERATION) as f64).sqrt() as f32;
    ISO_FALL_ACCELERATION * impact_time
}

/// `FallingConstants`' thresholds: the impact speeds of falls from 0.35,
/// 0.5, 1.5, 2.5 and 3.5 levels.
pub fn is_falling_threshold() -> f32 {
    iso_impact_speed_from_height(0.35)
}
pub fn no_damage_threshold() -> f32 {
    iso_impact_speed_from_height(0.5)
}
pub fn hard_fall_threshold() -> f32 {
    iso_impact_speed_from_height(1.5)
}
pub fn severe_fall_threshold() -> f32 {
    iso_impact_speed_from_height(2.5)
}
pub fn lethal_fall_threshold() -> f32 {
    iso_impact_speed_from_height(3.5)
}

/// `FallingConstants.getFallSeverity(isoFallSpeed)`.
pub fn fall_severity(speed: f32) -> FallSeverity {
    if speed < is_falling_threshold() {
        FallSeverity::None
    } else if speed < hard_fall_threshold() {
        FallSeverity::Light
    } else if speed < severe_fall_threshold() {
        FallSeverity::Hard
    } else if speed < lethal_fall_threshold() {
        FallSeverity::Severe
    } else {
        FallSeverity::Lethal
    }
}

/// `FallDamage`: the last landing.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct FallDamage {
    pub impact_iso_speed: f32,
    pub damaging_fall: bool,
    pub severity: FallSeverity,
}

impl FallDamage {
    /// `reset()`.
    pub fn reset(&mut self) {
        *self = FallDamage::default();
    }
    /// `setLandingImpact(impactIsoSpeed)`.
    pub fn set_landing_impact(&mut self, speed: f32) {
        self.impact_iso_speed = speed;
        self.damaging_fall = speed >= no_damage_threshold();
        self.severity = fall_severity(speed);
    }
    pub fn is_damaging_fall(&self) -> bool {
        self.damaging_fall
    }
    pub fn is_light_fall(&self) -> bool {
        self.severity == FallSeverity::Light
    }
    pub fn is_more_than_light_fall(&self) -> bool {
        self.severity > FallSeverity::Light
    }
    pub fn is_hard_fall(&self) -> bool {
        self.severity == FallSeverity::Hard
    }
    pub fn is_more_than_hard_fall(&self) -> bool {
        self.severity > FallSeverity::Hard
    }
    pub fn is_lethal_fall(&self) -> bool {
        self.severity == FallSeverity::Lethal
    }
}

/// `FallingWhileInjured.fromBodyPartType(type)`'s constant for a
/// `BodyPartType` field.
fn falling_while_injured_of(field: &str) -> Option<&'static str> {
    Some(match field {
        "Head" => "Head",
        "UpperLeg_L" | "UpperLeg_R" | "LowerLeg_L" | "LowerLeg_R" => "Leg",
        "Foot_L" | "Foot_R" => "Foot",
        "UpperArm_L" | "UpperArm_R" | "ForeArm_L" | "ForeArm_R" => "Arm",
        "Hand_L" | "Hand_R" => "Hand",
        "Neck" | "Torso_Upper" | "Torso_Lower" | "Groin" => "Spine",
        _ => return None,
    })
}

impl<I: 'static> GameCharacter<I> {
    /// `FallingWhileInjured.getDamageMultiplier(bodyPart)` of the part in
    /// `slot`.
    fn falling_while_injured_multiplier(&self, chars: &Characters, slot: usize) -> f32 {
        let p = &self.body_damage.parts[slot];
        let field = chars.sim.body_part_types.field_of(p.index as i32);
        let Some(name) = falling_while_injured_of(field) else { return 1.0 };
        let Some((_, [injured, deep, fractured])) = chars.sim.falling_while_injured.iter().find(|(n, _)| n == name) else {
            return 1.0;
        };
        if p.fracture_time > 0.0 {
            *fractured
        } else if p.deep_wounded {
            *deep
        } else if p.has_injury() {
            *injured
        } else {
            1.0
        }
    }

    /// The `FallDamage` animation variables (`bLandLight`, `bHardFall`,
    /// ...).
    fn sync_fall_vars(&mut self) {
        let f = self.fall_damage;
        self.vars.fall_damaging = f.is_damaging_fall();
        self.vars.fall_light = f.is_light_fall();
        self.vars.fall_hard = f.is_hard_fall();
        self.vars.fall_more_than_hard = f.is_more_than_hard_fall();
        self.vars.fall_impact_severity = f.severity.name();
    }

    /// `clearFallDamage()`.
    pub fn clear_fall_damage(&mut self) {
        self.fall_damage.reset();
        self.sync_fall_vars();
    }

    /// `DoLand(impactIsoSpeed)`: nothing while climbing or in a ragdoll
    /// fall, else the impact of a fall.
    pub fn do_land(&mut self, impact_iso_speed: f32, cx: &mut StateCx<'_, I>) {
    if cx.has_host_error() { return; }
        if self.vars.climbing || self.vars.ragdoll_fall {
            self.clear_fall_damage();
            return;
        }
        if impact_iso_speed < is_falling_threshold() {
            return;
        }
        self.fall_damage.set_landing_impact(impact_iso_speed);
        self.sync_fall_vars();
        self.handle_landing_impact(cx);
    }

    /// `handleLandingImpact(fallDamage)`, with `IsoPlayer`'s override (the
    /// `DeathFall` voice stops, a hard landing resets the boredom).
    pub fn handle_landing_impact(&mut self, cx: &mut StateCx<'_, I>) {
    if cx.has_host_error() { return; }
        if self.is_player() {
            let f = self.fall_damage;
            if f.is_damaging_fall() {
                cx.ctx.event(SimEvent::StopPlayerVoiceSound("DeathFall"));
            }
            if cx.ctx.is_client() {
                return;
            }
            if f.is_more_than_light_fall() {
                if let Ok(b) = cx.chars.sim.character_stats.field("BOREDOM") {
                    self.stats.reset(&b);
                }
            }
        }
        self.handle_landing_impact_character(cx);
    }

    /// `IsoGameCharacter.handleLandingImpact(fallDamage)`.
    fn handle_landing_impact_character(&mut self, cx: &mut StateCx<'_, I>) {
    if cx.has_host_error() { return; }
        let f = self.fall_damage;
        let damaging = f.is_damaging_fall();
        let more_than_light = f.is_more_than_light_fall();
        let more_than_hard = f.is_more_than_hard_fall();
        let lethal = f.is_lethal_fall();
        let was_alive = !self.is_dead();
        if !damaging || self.vars.climbing {
            return;
        }
        let (sm, chars, ctx) = (cx.sm, cx.chars, cx.ctx);
        let perk = |c: &Self, id: &str| c.perk_level(chars.perks.constant(id).as_ref());
        let nimble = perk(self, "Nimble");
        let mut unscratch = ctx.rand_next(80 - nimble) == 0;
        let alpha = f.impact_iso_speed / hard_fall_threshold();
        // PZMath.lerpFunc_EaseOutQuad
        let raw = alpha * alpha;
        let mut hr = raw * 115.0;
        hr *= ctx.rand_next_float(0.5, 1.0);
        let weight = (self.capacity_weight(sm, ctx) / self.max_weight() as f32).min(1.8);
        hr *= weight;
        // The original dereferences the current square.
        let natural = self
            .path
            .current_square
            .and_then(|sq| cx.world.floor_sprite_name(sq))
            .is_some_and(|n| n.starts_with("blends_natural"));
        if natural {
            hr *= 0.8;
            if !unscratch {
                unscratch = ctx.rand_next(65 - nimble) == 0;
            }
        }
        let has = |c: &Self, t: &str| c.has_trait(sm, t).unwrap_or(false);
        if has(self, "OBESE") || has(self, "EMACIATED") {
            hr *= 1.4;
        } else if has(self, "OVERWEIGHT") || has(self, "VERY_UNDERWEIGHT") {
            hr *= 1.2;
        }
        hr *= (1.0 - perk(self, "Fitness") as f32 * 0.05).max(0.1);
        hr *= (1.0 - perk(self, "Nimble") as f32 * 0.05).max(0.1);
        let fatal = lethal && !unscratch;
        if fatal {
            hr = 1000.0;
        } else if lethal {
            hr *= 0.05;
        } else if unscratch {
            hr = 0.0;
        }
        if more_than_light {
            self.fallen_on_knees(true, cx);
            if cx.has_host_error() { return; }
            self.drop_hand_items_in(cx);
            if cx.has_host_error() { return; }
        } else {
            self.helmet_fall(false, cx);
        }
        let (sm, chars, ctx) = (cx.sm, cx.chars, cx.ctx);
        for slot in 0..self.body_damage.parts.len() {
            hr *= self.falling_while_injured_multiplier(chars, slot);
        }
        if hr <= 0.0 {
            return;
        }
        self.reduce_general_health(chars, hr);
        if hr > 0.0 {
            let _ = self.body_damage_update(sm, chars, ctx);
            self.killed_by_fall = was_alive && self.is_dead();
        }
        ctx.event(SimEvent::PlayerGetDamage { kind: "FALLDOWN", amount: hr });
        if hr > 5.0 && !self.is_dead() && self.is_player() {
            // IsoPlayer.playPainVoicesFromFallDamage (the low one has a
            // leading space in the original).
            ctx.event(SimEvent::PlayerVoiceSound(if more_than_light { "PainFromFallHigh" } else { " PainFromFallLow" }));
        }
        let injury = damaging && (ctx.rand_next(100) as f32) < hr;
        if injury {
            let mut rand = (raw * 55.0) as i32;
            let (cap, max) = (self.capacity_weight(sm, ctx), self.max_weight() as f32);
            if max - cap < 2.0 {
                rand = (rand as f32 + cap / max * 20.0) as i32;
            }
            if has(self, "OBESE") || has(self, "EMACIATED") {
                rand += 20;
            } else if has(self, "OVERWEIGHT") || has(self, "VERY_UNDERWEIGHT") {
                rand += 10;
            }
            let fitness = perk(self, "Fitness");
            if fitness > 4 {
                rand = (rand as f64 - (fitness - 4) as f64 * 1.5) as i32;
            }
            rand = (rand as f64 - perk(self, "Nimble") as f64 * 1.5) as i32;
            let types = &chars.sim.body_part_types;
            let idx = |f: &str| types.index(f).map_or(0, |i| i as i32);
            let (hand_l, upper_leg_l, foot_r) = (idx("Hand_L"), idx("UpperLeg_L"), idx("Foot_R"));
            let part = if more_than_hard || more_than_light && ctx.rand_next(2) == 0 {
                ctx.rand_next_range(hand_l, foot_r + 1)
            } else {
                ctx.rand_next_range(upper_leg_l, foot_r + 1)
            };
            self.landing_injury(part, rand, cx);
            if more_than_hard {
                let part = cx.ctx.rand_next_range(hand_l, foot_r + 1);
                self.landing_injury(part, rand, cx);
            }
        }
        if fatal {
            for _ in 0..4 {
                self.splat_blood_floor_big(cx.ctx);
            }
        }
    }

    /// One injury of a landing: a fracture, a deep wound or a stiff part.
    fn landing_injury(&mut self, part: i32, rand: i32, cx: &mut StateCx<'_, I>) {
        let (sm, chars, ctx) = (cx.sm, cx.chars, cx.ctx);
        let Ok(slot) = self.part_slot(chars, part) else { return };
        let Ok(env) = self.injury_env(sm, ctx) else { return };
        if ctx.rand_next(100) < rand && ctx.bone_fracture() {
            let t = ctx.rand_next_range(50, 80) as f32;
            self.body_damage.parts[slot].generate_fracture_new(&env, t);
        } else if ctx.rand_next(100) < rand + 10 {
            self.body_damage.parts[slot].generate_deep_wound(&env);
        } else {
            self.body_damage.parts[slot].stiffness = 100.0;
        }
    }

    /// `helmetFall(hitHead)`: worn clothing that may fall off does, with a
    /// better chance on a hit to the head. Returns whether any fell.
    pub fn helmet_fall(&mut self, hit_head: bool, cx: &mut StateCx<'_, I>) -> bool {
        let (sm, ctx) = (cx.sm, cx.ctx);
        if ctx.is_client() {
            return false;
        }
        // A zombie without worn items: helmetFallFromVisuals, TODO(zombie
        // visuals).
        if self.is_zombie() && !self.is_using_worn_items() {
            return false;
        }
        let fallen = self.items.helmet_fall_from_worn_items(sm, ctx, hit_head);
        for item in &fallen {
            self.on_worn_items_changed(sm);
            ctx.event(SimEvent::FallingItem(item.clone()));
        }
        let removed = !fallen.is_empty();
        if removed && self.is_local_player(ctx) {
            ctx.event(SimEvent::ClothingUpdated);
        }
        if removed && !self.is_zombie() {
            ctx.event(SimEvent::FallenHat);
        }
        removed
    }

    /// `fallenOnKnees()`.
    pub fn fallen_on_knees_soft(&mut self, cx: &mut StateCx<'_, I>) {
    if cx.has_host_error() { return; }
        self.fallen_on_knees(false, cx);
    }

    /// `fallenOnKnees(hardFall)`: the helmet may fall, the knees and hands
    /// get dirty, and with bad luck the hand items drop and a part is
    /// torn and scratched (or, scratched already, deeply wounded).
    pub fn fallen_on_knees(&mut self, hard_fall: bool, cx: &mut StateCx<'_, I>) {
    if cx.has_host_error() { return; }
        let (sm, chars, ctx) = (cx.sm, cx.chars, cx.ctx);
        if ctx.is_client() || self.is_god_mod(ctx) {
            return;
        }
        self.helmet_fall(hard_fall, cx);
        let ctx = cx.ctx;
        let pick = || {
            let mut p = ctx.rand_next_range(blood_part::HAND_L as i32, blood_part::TORSO_UPPER as i32);
            if ctx.rand_next(2) == 0 {
                p = ctx.rand_next_range(blood_part::UPPER_LEG_L as i32, blood_part::BACK as i32);
            }
            p
        };
        let part = pick();
        for _ in 0..4 {
            let part2 = pick();
            let n = ctx.rand_next_range(2, 6);
            if !self.is_animal() {
                self.add_dirt_part(sm, ctx, Some(part2 as usize), n, false);
                if self.is_local_player(ctx) && !ctx.is_server() {
                    ctx.event(SimEvent::ClothingUpdated);
                }
            }
        }
        if ctx.debug_always_trip_over_fence() {
            self.drop_hand_items_in(cx);
            if cx.has_host_error() { return; }
        }
        let ctx = cx.ctx;
        let nimble = self.perk_level(chars.perks.constant("Nimble").as_ref());
        if ctx.rand_next(4 + nimble) != 0 {
            return;
        }
        if ctx.rand_next(4) == 0 {
            self.drop_hand_items_in(cx);
            if cx.has_host_error() { return; }
        }
        let ctx = cx.ctx;
        self.items.add_hole(sm, ctx, part as usize, false);
        ctx.event(SimEvent::AddHole { part, all_layers: false });
        if self.is_local_player(ctx) && !ctx.is_server() {
            ctx.event(SimEvent::ClothingUpdated);
        }
        // BodyPartType.FromIndex(part.index()): the indices agree below
        // Back.
        let Ok(slot) = self.part_slot(chars, part) else { return };
        let defense = self.items.body_part_clothing_defense(sm, part, false, false);
        if (ctx.rand_next(100) as f32) >= defense {
            ctx.event(SimEvent::AddBlood { part, scratched: true, bitten: false, all_layers: false });
            let Ok(env) = self.injury_env(sm, ctx) else { return };
            if self.body_damage.parts[slot].scratched {
                self.body_damage.parts[slot].generate_deep_wound(&env);
                if self.is_player() {
                    if ctx.is_server() {
                        ctx.event(SimEvent::SyncDamage);
                    }
                    ctx.event(SimEvent::PlayerVoiceSound("PainFromLacerate"));
                }
            } else {
                self.body_damage.parts[slot].set_scratched(&env, true, true);
                if self.is_player() {
                    if ctx.is_server() {
                        ctx.event(SimEvent::SyncDamage);
                    }
                    ctx.event(SimEvent::PlayerVoiceSound("PainFromScratch"));
                }
            }
        }
        // IsoPlayer.syncVisuals(): multiplayer.
    }

    /// `IsoPlayer.OnDeath()`: `OnCharacterDeath`, then (not on a server)
    /// the action queue stops, a sleeper wakes, the hand items drop, and
    /// the rest is the engine's ([`SimEvent::PlayerDeath`]).
    pub fn player_on_death(&mut self, cx: &mut StateCx<'_, I>) {
    if cx.has_host_error() { return; }
        cx.ctx.event(SimEvent::CharacterDeath);
        if cx.ctx.is_server() {
            return;
        }
        cx.ctx.event(SimEvent::StopAllActionQueue);
        if self.is_asleep() {
            cx.ctx.event(SimEvent::FadeIn(0.5));
            self.asleep = false;
        }
        self.drop_hand_items_in(cx);
        if cx.has_host_error() { return; }
        let local = self.is_local_player(cx.ctx);
        cx.ctx.event(SimEvent::PlayerDeath { local });
    }
}

#[cfg(test)]
mod tests {
    use std::cell::{Cell, RefCell};
    use std::collections::VecDeque;

    use pz_inventory::ItemClass;

    use super::*;
    use crate::character::floor_drop::tests::TestFloor;
    use crate::character::state::StateWorld;
    use crate::character::tests::{give_item, player, Recorder};
    use crate::character::thermoregulator::body_tests::body_fixture;
    use crate::character::{SimContext, ZomboidGlobals};

    /// Scripted randoms: `Rand.Next(n)` answers from `ints` (then `n - 1`,
    /// never a hit), a range its minimum, a float `float`.
    #[derive(Default)]
    struct Ctx {
        rec: Recorder,
        ints: RefCell<VecDeque<i32>>,
        float: Cell<f32>,
    }

    impl SimContext for Ctx {
        fn zomboid_globals(&self) -> &ZomboidGlobals {
            &self.rec.globals
        }
        fn event(&self, e: SimEvent) {
            self.rec.event(e)
        }
        fn rand_next(&self, max: i32) -> i32 {
            self.ints.borrow_mut().pop_front().unwrap_or((max - 1).max(0))
        }
        fn rand_next_range(&self, min: i32, _max: i32) -> i32 {
            min
        }
        fn rand_next_float(&self, _min: f32, _max: f32) -> f32 {
            self.float.get()
        }
    }

    #[derive(Default)]
    struct Floor {
        floor: TestFloor,
    }

    impl StateWorld<()> for Floor {
        fn floor(&mut self) -> Option<&mut dyn pz_inventory::floor_drop::FloorWorld> {
            Some(&mut self.floor)
        }
    }

    fn events(ctx: &Ctx) -> Vec<String> {
        ctx.rec.events.borrow().clone()
    }

    #[test]
    fn severities_follow_the_impact_speed() {
        let t = [is_falling_threshold(), no_damage_threshold(), hard_fall_threshold(), severe_fall_threshold(), lethal_fall_threshold()];
        assert!(t.windows(2).all(|w| w[0] < w[1]));
        // v = a * sqrt(2h / a) = sqrt(2ah).
        assert!((iso_impact_speed_from_height(2.0) - (4.0 * ISO_FALL_ACCELERATION).sqrt()).abs() < 1e-5);
        assert_eq!(fall_severity(t[0] - 0.01), FallSeverity::None);
        assert_eq!(fall_severity(t[0]), FallSeverity::Light);
        assert_eq!(fall_severity(t[2]), FallSeverity::Hard);
        assert_eq!(fall_severity(t[3]), FallSeverity::Severe);
        assert_eq!(fall_severity(t[4] + 5.0), FallSeverity::Lethal);
        let mut f = FallDamage::default();
        // A light fall below the damage threshold.
        f.set_landing_impact((t[0] + t[1]) / 2.0);
        assert!(f.is_light_fall() && !f.is_damaging_fall() && !f.is_more_than_light_fall());
        f.set_landing_impact(t[3]);
        assert!(f.is_damaging_fall() && f.is_more_than_hard_fall() && !f.is_hard_fall() && !f.is_lethal_fall());
        f.reset();
        assert_eq!(f, FallDamage::default());
    }

    #[test]
    fn falling_while_injured_reads_the_table() {
        let (sm, mut chars) = body_fixture();
        chars.sim.falling_while_injured = vec![("Hand".into(), [2.0, 3.0, 4.0]), ("Spine".into(), [5.0, 6.0, 7.0])];
        let mut c = player(&sm, &mut chars);
        let hand = c.part_slot(&chars, chars.sim.body_part_types.index("Hand_R").unwrap() as i32).unwrap();
        let neck = c.part_slot(&chars, chars.sim.body_part_types.index("Neck").unwrap() as i32).unwrap();
        let head = c.part_slot(&chars, chars.sim.body_part_types.index("Head").unwrap() as i32).unwrap();
        assert_eq!(c.falling_while_injured_multiplier(&chars, hand), 1.0);
        c.body_damage.parts[hand].scratched = true;
        assert_eq!(c.falling_while_injured_multiplier(&chars, hand), 2.0);
        c.body_damage.parts[hand].deep_wounded = true;
        assert_eq!(c.falling_while_injured_multiplier(&chars, hand), 3.0);
        c.body_damage.parts[hand].fracture_time = 10.0;
        assert_eq!(c.falling_while_injured_multiplier(&chars, hand), 4.0);
        c.body_damage.parts[neck].deep_wounded = true;
        assert_eq!(c.falling_while_injured_multiplier(&chars, neck), 6.0);
        // No row for the head in this table.
        c.body_damage.parts[head].scratched = true;
        assert_eq!(c.falling_while_injured_multiplier(&chars, head), 1.0);
    }

    #[test]
    fn landing_while_climbing_or_barely_falling_does_nothing() {
        let (sm, mut chars) = body_fixture();
        let ctx = Ctx::default();
        let mut w = Floor::default();
        let mut c = player(&sm, &mut chars);
        let mut cx = StateCx::new(&sm, &chars, &ctx, &mut w);
        c.fall_damage.set_landing_impact(hard_fall_threshold());
        c.vars.climbing = true;
        c.do_land(lethal_fall_threshold(), &mut cx);
        assert_eq!(c.fall_damage, FallDamage::default());
        c.vars.climbing = false;
        c.do_land(is_falling_threshold() * 0.5, &mut cx);
        assert_eq!(c.fall_damage, FallDamage::default());
        assert!(events(&ctx).is_empty());
    }

    #[test]
    fn a_hard_landing_drops_the_hand_items() {
        let (sm, mut chars) = body_fixture();
        let ctx = Ctx::default();
        ctx.float.set(0.5);
        let mut w = Floor::default();
        let mut c = player(&sm, &mut chars);
        c.path.current_square = Some((4, 5, 0));
        (c.vars.x, c.vars.y) = (4.5, 5.5);
        give_item(&sm, &mut c, ItemClass::Normal, "Spoon", 2, |_| {}, |_| {});
        c.set_primary_hand_item(&sm, Some(2));
        let boredom = chars.sim.character_stats.field("BOREDOM").unwrap();
        c.stats.set(&boredom, 0.5);
        let mut cx = StateCx::new(&sm, &chars, &ctx, &mut w);
        c.do_land((hard_fall_threshold() + severe_fall_threshold()) / 2.0, &mut cx);
        let _ = cx;
        assert!(c.vars.fall_hard && c.vars.fall_damaging && !c.vars.fall_more_than_hard);
        assert_eq!(c.vars.fall_impact_severity, "Hard");
        // The spoon falls where the player stands; boredom is reset.
        assert_eq!(w.floor.put.iter().map(|d| (d.0, d.1)).collect::<Vec<_>>(), [((4, 5, 0), 2)]);
        assert_eq!(c.items.primary_hand_item, None);
        assert_eq!(c.stats.get(&boredom), 0.0);
        assert_eq!(events(&ctx)[0], format!("{:?}", SimEvent::StopPlayerVoiceSound("DeathFall")));
    }

    #[test]
    fn a_light_landing_keeps_the_hand_items() {
        let (sm, mut chars) = body_fixture();
        let ctx = Ctx::default();
        ctx.float.set(0.5);
        let mut w = Floor::default();
        let mut c = player(&sm, &mut chars);
        c.path.current_square = Some((4, 5, 0));
        give_item(&sm, &mut c, ItemClass::Normal, "Spoon", 2, |_| {}, |_| {});
        c.set_primary_hand_item(&sm, Some(2));
        let mut cx = StateCx::new(&sm, &chars, &ctx, &mut w);
        c.do_land((no_damage_threshold() + hard_fall_threshold()) / 2.0, &mut cx);
        let _ = cx;
        assert!(c.vars.fall_light && c.vars.fall_damaging);
        assert!(w.floor.put.is_empty());
        assert_eq!(c.items.primary_hand_item, Some(2));
    }

    #[test]
    fn a_lethal_fall_costs_1000_health_unless_lucky() {
        let (sm, mut chars) = body_fixture();
        // Every part counts fully towards the overall health.
        for p in sm.builtin_tables.borrow_mut().body_parts.iter_mut() {
            p.damage_modifier = 1.0;
        }
        chars.sim.body_part_types = crate::character::BodyPartTypes::from_tables(&sm.builtin_tables.borrow());
        let mut w = Floor::default();
        // The first roll (1 in 80 - Nimble) comes up: 5 % of a damage that
        // an empty inventory makes 0.
        let lucky = Ctx::default();
        lucky.float.set(1.0);
        lucky.ints.borrow_mut().push_back(0);
        let mut c = player(&sm, &mut chars);
        c.path.current_square = Some((4, 5, 0));
        c.environment.square_has_chunk = true;
        let mut cx = StateCx::new(&sm, &chars, &lucky, &mut w);
        c.do_land(lethal_fall_threshold() + 1.0, &mut cx);
        assert!(c.body_damage.parts.iter().all(|p| p.health == 100.0));
        assert!(!events(&lucky).iter().any(|e| e.starts_with("AddBloodSplat") || e.contains("FALLDOWN")));
        // Not lucky: 1000 spread over the 17 parts, and four big blood
        // splats.
        let ctx = Ctx::default();
        ctx.float.set(1.0);
        let mut c = player(&sm, &mut chars);
        c.path.current_square = Some((4, 5, 0));
        c.environment.square_has_chunk = true;
        let mut cx = StateCx::new(&sm, &chars, &ctx, &mut w);
        c.do_land(lethal_fall_threshold() + 1.0, &mut cx);
        assert!(c.body_damage.parts.iter().all(|p| p.health == 100.0 - 1000.0 / 17.0));
        assert_eq!(events(&ctx).iter().filter(|e| e.starts_with("AddBloodSplat")).count(), 4);
        assert!(events(&ctx).iter().any(|e| e.contains("FALLDOWN") && e.contains("1000.0")));
    }

    #[test]
    fn falling_on_the_knees_can_drop_the_hand_items() {
        let (sm, mut chars) = body_fixture();
        let mut w = Floor::default();
        // Rolls: the part, then four (part, dirt) pairs, then 1 in 4 + Nimble
        // and 1 in 4 both come up.
        let ctx = Ctx::default();
        ctx.ints.borrow_mut().extend([1, 1, 1, 1, 1, 0, 0]);
        let mut c = player(&sm, &mut chars);
        c.path.current_square = Some((4, 5, 0));
        give_item(&sm, &mut c, ItemClass::Normal, "Spoon", 2, |_| {}, |_| {});
        c.set_primary_hand_item(&sm, Some(2));
        let mut cx = StateCx::new(&sm, &chars, &ctx, &mut w);
        c.fallen_on_knees(true, &mut cx);
        let _ = cx;
        assert_eq!(w.floor.put.iter().map(|d| d.1).collect::<Vec<_>>(), [2]);
        // Without the bad luck the items stay.
        let ctx = Ctx::default();
        let mut w = Floor::default();
        let mut c = player(&sm, &mut chars);
        give_item(&sm, &mut c, ItemClass::Normal, "Spoon", 2, |_| {}, |_| {});
        c.set_primary_hand_item(&sm, Some(2));
        let mut cx = StateCx::new(&sm, &chars, &ctx, &mut w);
        c.fallen_on_knees(true, &mut cx);
        let _ = cx;
        assert!(w.floor.put.is_empty());
    }

    #[test]
    fn a_dying_player_wakes_and_drops_the_hand_items() {
        let (sm, mut chars) = body_fixture();
        let ctx = Ctx::default();
        let mut w = Floor::default();
        let mut c = player(&sm, &mut chars);
        c.path.current_square = Some((4, 5, 0));
        c.asleep = true;
        give_item(&sm, &mut c, ItemClass::Normal, "Spoon", 2, |_| {}, |_| {});
        c.set_primary_hand_item(&sm, Some(2));
        let mut cx = StateCx::new(&sm, &chars, &ctx, &mut w);
        c.player_on_death(&mut cx);
        let _ = cx;
        assert!(!c.asleep);
        assert_eq!(w.floor.put.iter().map(|d| d.1).collect::<Vec<_>>(), [2]);
        // The spoon's own ContainerUpdate and ItemFall come before the death.
        let named: Vec<String> = events(&ctx).into_iter().filter(|e| !e.starts_with("ContainerUpdate") && !e.starts_with("ItemFall")).collect();
        assert_eq!(
            named,
            [
                format!("{:?}", SimEvent::CharacterDeath),
                format!("{:?}", SimEvent::StopAllActionQueue),
                format!("{:?}", SimEvent::FadeIn(0.5)),
                format!("{:?}", SimEvent::PlayerDeath { local: true }),
            ]
        );
    }

    #[test]
    fn failed_death_drop_preserves_earlier_wake_prefix_and_stops_later_death_event() {
        let (sm, mut chars) = body_fixture();
        let ctx = Ctx::default();
        let mut w = Floor::default();
        w.floor.fail_at = Some(1);
        let mut c = player(&sm, &mut chars);
        c.path.current_square = Some((4, 5, 0));
        c.asleep = true;
        give_item(&sm, &mut c, ItemClass::Normal, "Spoon", 2, |_| {}, |_| {});
        c.set_primary_hand_item(&sm, Some(2));
        let mut cx = StateCx::new(&sm, &chars, &ctx, &mut w);
        c.player_on_death(&mut cx);
        assert_eq!(cx.state_result().map_err(|error| error.host().cloned().expect("original Host domain")), Err(pz_inventory::floor_drop::RuntimeHostError::Borrowed("actual floor allocation")));
        assert!(!c.asleep);
        assert!(c.items.primary_hand_item.is_none());
        assert_eq!(events(&ctx), [format!("{:?}", SimEvent::CharacterDeath), format!("{:?}", SimEvent::StopAllActionQueue), format!("{:?}", SimEvent::FadeIn(0.5))]);
        drop(cx);
        assert!(w.floor.put.is_empty());
    }
}
