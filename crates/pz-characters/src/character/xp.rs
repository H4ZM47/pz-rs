//! `IsoGameCharacter.XP`, `IsoGameCharacter.PerkInfo`,
//! `IsoGameCharacter.XPMultiplier` and the character's perk-level methods
//! (`getPerkLevel`, `LevelPerk`, `LoseLevel`, `applyTraits`, ...).
//!
//! The original keys `xpMap`, `xpMapMultiplier`, the descriptor's
//! `xpBoostMap` and `applyTraits`' level map by `Perk`, which does not
//! override `hashCode`, so its iteration order is arbitrary. Here they keep
//! insertion order, which is one of the orders the original can produce;
//! only `applyTraits`' `perkList` order and the save order depend on it.

use std::rc::Rc;

use pz_scripts::objects::items::character_profession_definition;
use pz_scripts::objects::items::{character_trait_definition, CharacterTraitDefinitionRef};
use pz_scripts::registry::RegistryKey;
use pz_scripts::{ScriptError, ScriptManager, ScriptResult};

use super::context::{HaloText, SimContext, SimEvent};
use super::GameCharacter;
use crate::skills::PerkRef;
use crate::survivor::java_float_string;
use crate::Characters;

/// `IsoGameCharacter.XPMultiplier`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct XPMultiplier {
    pub multiplier: f32,
    pub min_level: i32,
    pub max_level: i32,
}

/// `IsoGameCharacter.PerkInfo`.
#[derive(Clone, Debug)]
pub struct PerkInfo {
    pub level: i32,
    pub perk: PerkRef,
}

/// `IsoGameCharacter.XP` (its methods that reach the character are on
/// [`GameCharacter`]).
#[derive(Clone, Debug, Default)]
pub struct XP {
    pub level: i32,
    pub lastlevel: i32,
    pub total_xp: f32,
    /// `xpMap`.
    pub xp_map: Vec<(PerkRef, f32)>,
    /// `xpMapMultiplier`.
    pub xp_map_multiplier: Vec<(PerkRef, XPMultiplier)>,
}

fn npe(what: &str) -> ScriptError {
    ScriptError::new(format!("java.lang.NullPointerException: {what}"))
}

fn is(perk: &PerkRef, constant: Option<&PerkRef>) -> bool {
    constant.is_some_and(|c| Rc::ptr_eq(perk, c))
}

impl XP {
    /// `addXpMultiplier`.
    pub fn add_xp_multiplier(&mut self, perk: &PerkRef, multiplier: f32, min_level: i32, max_level: i32) {
        let m = XPMultiplier { multiplier, min_level, max_level };
        match self.xp_map_multiplier.iter_mut().find(|(p, _)| Rc::ptr_eq(p, perk)) {
            Some(e) => e.1 = m,
            None => self.xp_map_multiplier.push((perk.clone(), m)),
        }
    }

    /// `getMultiplierMap().get(perk)`.
    pub fn multiplier_entry(&self, perk: &PerkRef) -> Option<XPMultiplier> {
        self.xp_map_multiplier.iter().find(|(p, _)| Rc::ptr_eq(p, perk)).map(|(_, m)| *m)
    }

    /// `getMultiplierMap().remove(perk)`.
    pub fn remove_multiplier(&mut self, perk: &PerkRef) -> Option<XPMultiplier> {
        let i = self.xp_map_multiplier.iter().position(|(p, _)| Rc::ptr_eq(p, perk))?;
        Some(self.xp_map_multiplier.remove(i).1)
    }

    /// `getMultiplier`: 0 without an entry.
    pub fn multiplier(&self, perk: &PerkRef) -> f32 {
        self.multiplier_entry(perk).map_or(0.0, |m| m.multiplier)
    }

    /// `getLevel`.
    pub fn level(&self) -> i32 {
        self.level
    }
    /// `setLevel`.
    pub fn set_level(&mut self, level: i32) {
        self.level = level;
    }
    /// `getTotalXp`.
    pub fn total_xp(&self) -> f32 {
        self.total_xp
    }
    /// `setTotalXP`.
    pub fn set_total_xp(&mut self, xp: f32) {
        self.total_xp = xp;
    }

    /// `getXP`: 0 for a perk without XP.
    pub fn xp(&self, perk: &PerkRef) -> f32 {
        self.xp_map.iter().find(|(p, _)| Rc::ptr_eq(p, perk)).map_or(0.0, |(_, v)| *v)
    }

    fn put_xp(&mut self, perk: &PerkRef, v: f32) {
        match self.xp_map.iter_mut().find(|(p, _)| Rc::ptr_eq(p, perk)) {
            Some(e) => e.1 = v,
            None => self.xp_map.push((perk.clone(), v)),
        }
    }
}

impl<I> GameCharacter<I> {
    // ---- perk levels ----------------------------------------------------

    /// `getPerkInfo`.
    pub fn perk_info(&self, perk: Option<&PerkRef>) -> Option<&PerkInfo> {
        let perk = perk?;
        self.perk_list.iter().find(|i| Rc::ptr_eq(&i.perk, perk))
    }

    fn perk_info_mut(&mut self, perk: &PerkRef) -> Option<&mut PerkInfo> {
        self.perk_list.iter_mut().find(|i| Rc::ptr_eq(&i.perk, perk))
    }

    /// `getPerkLevel`: 0 without a `PerkInfo`.
    pub fn perk_level(&self, perk: Option<&PerkRef>) -> i32 {
        self.perk_info(perk).map_or(0, |i| i.level)
    }

    /// `setPerkLevelDebug`.
    pub fn set_perk_level_debug(&mut self, perk: &PerkRef, level: i32, ctx: &dyn SimContext) {
        match self.perk_info_mut(perk) {
            Some(i) => i.level = level,
            None => self.perk_list.push(PerkInfo { level, perk: perk.clone() }),
        }
        if ctx.is_client() && self.is_player() {
            ctx.event(SimEvent::SendPerks);
        }
    }

    /// `level0`.
    pub fn level0(&mut self, perk: &PerkRef) {
        if let Some(i) = self.perk_info_mut(perk) {
            i.level = 0;
        }
    }

    /// `LoseLevel`.
    pub fn lose_level(&mut self, chars: &Characters, perk: &PerkRef, ctx: &dyn SimContext) {
        let player = self.is_player();
        match self.perk_info_mut(perk) {
            Some(info) => {
                info.level -= 1;
                if info.level < 0 {
                    info.level = 0;
                }
                let level = info.level;
                ctx.event(SimEvent::LevelPerk { perk: perk.clone(), level, increased: false });
                if is(perk, chars.perks.constant("Sneak").as_ref()) && ctx.is_client() && player {
                    ctx.event(SimEvent::SendPerks);
                }
            }
            None => ctx.event(SimEvent::LevelPerk { perk: perk.clone(), level: 0, increased: false }),
        }
    }

    fn level_up_halo(&self, perk: &PerkRef, ctx: &dyn SimContext) {
        if self.is_player() && ctx.game_mode() != "Tutorial" && self.hours_survived(ctx) > 0.016666666666666666 {
            ctx.event(SimEvent::Halo(HaloText::GoodWithArrowUp(format!("+1 {}", perk.borrow().name()))));
        }
    }

    /// `LevelPerk(perk, removePick)` (`removePick` is unused).
    pub fn level_perk(&mut self, chars: &Characters, perk: Option<&PerkRef>, ctx: &dyn SimContext) -> ScriptResult {
        let perk = perk.ok_or_else(|| npe("perk is null"))?;
        if is(perk, chars.perks.max().as_ref()) {
            return Err(ScriptError::new("java.lang.IllegalArgumentException: perk == Perks.MAX"));
        }
        let fitness = chars.perks.constant("Fitness");
        if self.perk_info(Some(perk)).is_some() {
            self.level_up_halo(perk, ctx);
            let info = self.perk_info_mut(perk).expect("checked");
            info.level += 1;
            if info.level > 10 {
                info.level = 10;
            }
            let level = info.level;
            if ctx.is_debug() && is(perk, fitness.as_ref()) {
                // `player.getStats()`: the original throws for a non-player.
                if !self.is_player() {
                    return Err(npe("player"));
                }
                let f = chars.sim.character_stats.field("FITNESS")?;
                self.stats.set(&f, level as f32 / 5.0 - 1.0);
            }
            ctx.event(SimEvent::LevelPerk { perk: perk.clone(), level, increased: true });
            if ctx.is_client() && self.is_player() {
                ctx.event(SimEvent::SendPerks);
            }
            return Ok(());
        }
        self.perk_list.push(PerkInfo { level: 1, perk: perk.clone() });
        self.level_up_halo(perk, ctx);
        ctx.event(SimEvent::LevelPerk { perk: perk.clone(), level: 1, increased: true });
        Ok(())
    }

    /// `getPerkToUnit`: progress through all ten levels, 0..1.
    pub fn perk_to_unit(&self, perk: &PerkRef) -> f32 {
        let level = self.perk_level(Some(perk));
        if level == 10 {
            return 1.0;
        }
        let p = perk.borrow();
        let next = p.xp_for_level(level + 1);
        let mut xp = self.xp.xp(perk);
        let mut subtract = 0.0f32;
        if level > 0 {
            subtract = p.total_xp_for_level(level);
        }
        xp -= subtract;
        let progress = level as f32 * 0.1 + xp / next * 0.1;
        super::stats::clamp_f32(progress, 0.0, 1.0)
    }

    /// `getLevelUpLevels(level)`.
    pub fn level_up_levels(&self, chars: &Characters, level: i32) -> ScriptResult<i32> {
        let l = &chars.sim.level_up_levels;
        if l.len() as i32 <= level {
            return l.last().copied().ok_or_else(|| index_error(-1, 0));
        }
        l.get(level as usize).copied().ok_or_else(|| index_error(level, l.len()))
    }

    /// `getLevelMaxForXp`.
    pub fn level_max_for_xp(&self, chars: &Characters) -> i32 {
        chars.sim.level_up_levels.len() as i32
    }

    /// `getXpForLevel`.
    pub fn xp_for_level(&self, chars: &Characters, level: i32) -> ScriptResult<i32> {
        let l = &chars.sim.level_up_levels;
        let n = l.len() as i32;
        if level < n {
            let v = *l.get(level as usize).ok_or_else(|| index_error(level, l.len()))?;
            return Ok((v as f32 * self.level_up_multiplier) as i32);
        }
        let last = *l.last().ok_or_else(|| index_error(-1, 0))?;
        Ok(((last.wrapping_add((level - n + 1).wrapping_mul(400))) as f32 * self.level_up_multiplier) as i32)
    }

    // ---- XP --------------------------------------------------------------

    fn descriptor_boost(&self, perk: &PerkRef) -> ScriptResult<Option<i32>>
    where
        I: 'static,
    {
        let d = self.descriptor.as_ref().ok_or_else(|| npe("descriptor"))?;
        let id = perk.borrow().id().to_string();
        let d = d.borrow();
        Ok(d.xp_boost_map.iter().find(|(p, _)| *p == id).map(|(_, l)| *l))
    }

    /// `XP.getPerkBoost`.
    pub fn xp_perk_boost(&self, perk: &PerkRef) -> ScriptResult<i32>
    where
        I: 'static,
    {
        Ok(self.descriptor_boost(perk)?.unwrap_or(0))
    }

    /// `XP.setPerkBoost`: ignores `null`, `None` and `MAX`; 0 removes.
    pub fn xp_set_perk_boost(&mut self, chars: &Characters, perk: Option<&PerkRef>, level: i32) -> ScriptResult {
        let Some(perk) = perk else { return Ok(()) };
        if is(perk, chars.perks.none().as_ref()) || is(perk, chars.perks.max().as_ref()) {
            return Ok(());
        }
        let level = level.clamp(0, 10);
        let d = self.descriptor.as_ref().ok_or_else(|| npe("descriptor"))?;
        let id = perk.borrow().id().to_string();
        let mut d = d.borrow_mut();
        let map = &mut d.xp_boost_map;
        let pos = map.iter().position(|(p, _)| *p == id);
        match (level, pos) {
            (0, Some(i)) => {
                map.remove(i);
            }
            (0, None) => {}
            (l, Some(i)) => map[i].1 = l,
            (l, None) => map.push((id, l)),
        }
        Ok(())
    }

    /// `XP.AddXP(perk, amount)`: only for a local player.
    pub fn add_xp(
        &mut self,
        sm: &ScriptManager,
        chars: &Characters,
        perk: Option<&PerkRef>,
        amount: f32,
        ctx: &dyn SimContext,
    ) -> ScriptResult
    where
        I: 'static,
    {
        if self.is_local_player(ctx) {
            self.add_xp_full(sm, chars, perk, amount, true, true, false, false, ctx)?;
        }
        Ok(())
    }

    /// `XP.AddXPHaloText(perk, amount)`.
    pub fn add_xp_halo_text(
        &mut self,
        sm: &ScriptManager,
        chars: &Characters,
        perk: Option<&PerkRef>,
        amount: f32,
        ctx: &dyn SimContext,
    ) -> ScriptResult
    where
        I: 'static,
    {
        if self.is_local_player(ctx) {
            self.add_xp_full(sm, chars, perk, amount, true, true, false, true, ctx)?;
        }
        Ok(())
    }

    /// `XP.AddXP(perk, amount, noMultiplier[, haloText])`.
    #[allow(clippy::too_many_arguments)]
    pub fn add_xp_no_multiplier_flag(
        &mut self,
        sm: &ScriptManager,
        chars: &Characters,
        perk: Option<&PerkRef>,
        amount: f32,
        no_multiplier: bool,
        halo_text: bool,
        ctx: &dyn SimContext,
    ) -> ScriptResult
    where
        I: 'static,
    {
        if self.is_local_player(ctx) {
            self.add_xp_full(sm, chars, perk, amount, true, !no_multiplier, false, halo_text, ctx)?;
        }
        Ok(())
    }

    /// `XP.AddXPNoMultiplier`: `AddXP(perk, amount)` with the perk's
    /// multiplier entry removed for the call (and put back after).
    pub fn add_xp_no_multiplier(
        &mut self,
        sm: &ScriptManager,
        chars: &Characters,
        perk: Option<&PerkRef>,
        amount: f32,
        ctx: &dyn SimContext,
    ) -> ScriptResult
    where
        I: 'static,
    {
        let removed = perk.and_then(|p| self.xp.remove_multiplier(p));
        let r = self.add_xp(sm, chars, perk, amount, ctx);
        if let (Some(m), Some(p)) = (removed, perk) {
            self.xp.add_xp_multiplier(p, m.multiplier, m.min_level, m.max_level);
        }
        r
    }

    fn excluded_from_speed_reduction(chars: &Characters, perk: &PerkRef) -> bool {
        is(perk, chars.perks.constant("Sprinting").as_ref())
            || is(perk, chars.perks.constant("Fitness").as_ref())
            || is(perk, chars.perks.constant("Strength").as_ref())
    }

    fn excluded_from_speed_increase(chars: &Characters, perk: &PerkRef) -> bool {
        is(perk, chars.perks.constant("Fitness").as_ref()) || is(perk, chars.perks.constant("Strength").as_ref())
    }

    /// `XP.AddXP(perk, amount, callLua, doXPBoost, remote, haloText)`
    /// (`callLua` and `remote` are unused by the original).
    #[allow(clippy::too_many_arguments)]
    pub fn add_xp_full(
        &mut self,
        sm: &ScriptManager,
        chars: &Characters,
        ty: Option<&PerkRef>,
        mut amount: f32,
        _call_lua: bool,
        do_xp_boost: bool,
        _remote: bool,
        mut halo_text: bool,
        ctx: &dyn SimContext,
    ) -> ScriptResult
    where
        I: 'static,
    {
        if self.asleep {
            return Ok(());
        }
        let perks = &chars.perks;
        let perk =
            ty.and_then(|t| perks.perk_list.iter().find(|p| Rc::ptr_eq(p, t))).cloned().ok_or_else(|| npe("perk"))?;
        let fitness = perks.constant("Fitness");
        let strength = perks.constant("Strength");
        if is(&perk, fitness.as_ref()) && self.is_player() {
            let level = self.perk_level(fitness.as_ref());
            let has = |f: &str| self.has_trait(sm, f);
            let n = self.nutrition.as_ref().ok_or_else(|| npe("nutrition"))?;
            if !n.can_add_fitness_xp(level, &has)? {
                return Ok(());
            }
        }
        if is(&perk, strength.as_ref()) && self.is_player() {
            let n = self.nutrition.as_ref().ok_or_else(|| npe("nutrition"))?;
            if n.proteins() > 50.0 && n.proteins() < 300.0 {
                amount *= 1.5;
            }
            if n.proteins() < -300.0 {
                amount *= 0.7;
            }
        }
        let old_xp = self.xp.xp(&perk);
        let max_xp = perk.borrow().total_xp_for_level(10);
        if amount >= 0.0 && old_xp >= max_xp {
            return Ok(());
        }
        let mut m = 1.0f32;
        if do_xp_boost {
            let mut done = false;
            if let Some(v) = self.descriptor_boost(&perk)? {
                done = true;
                if v == 0 && !Self::excluded_from_speed_reduction(chars, &perk) {
                    m *= 0.25;
                } else if v == 1 && is(&perk, perks.constant("Sprinting").as_ref()) {
                    m *= 1.25;
                } else if v == 1 {
                    m *= 1.0;
                } else if v == 2 && !Self::excluded_from_speed_increase(chars, &perk) {
                    m *= 1.33;
                } else if v >= 3 && !Self::excluded_from_speed_increase(chars, &perk) {
                    m *= 1.66;
                }
            }
            if !done && !Self::excluded_from_speed_reduction(chars, &perk) {
                m = 0.25;
            }
            if self.has_trait(sm, "FAST_LEARNER")? && !Self::excluded_from_speed_increase(chars, &perk) {
                m *= 1.3;
            }
            if self.has_trait(sm, "SLOW_LEARNER")? && !Self::excluded_from_speed_reduction(chars, &perk) {
                m *= 0.7;
            }
            if self.has_trait(sm, "PACIFIST")? {
                // Melee perks, then (in a separate branch) Aiming.
                let pacified = ["SmallBlade", "LongBlade", "SmallBlunt", "Spear", "Blunt", "Axe", "Aiming"];
                if pacified.iter().any(|f| is(&perk, perks.constant(f).as_ref())) {
                    m *= 0.75;
                }
            }
            if self.has_trait(sm, "CRAFTY")? {
                let parent = perk.borrow().parent.clone();
                if parent.is_some_and(|p| is(&p, perks.constant("Crafting").as_ref())) {
                    m *= 1.3;
                }
            }
            amount *= m;
            let multiplier = self.xp.multiplier(&perk);
            if multiplier > 1.0 {
                amount *= multiplier;
            }
            if ctx.xp_multiplier_global_toggle() {
                amount *= ctx.xp_multiplier_global() as f32;
            } else {
                amount *= ctx.xp_multiplier_for(perk.borrow().id());
            }
        }
        let mut new_xp = old_xp + amount;
        if new_xp < 0.0 {
            new_xp = 0.0;
            amount = -old_xp;
        }
        if new_xp > max_xp {
            new_xp = max_xp;
            amount = new_xp - old_xp;
        }
        self.xp.put_xp(&perk, new_xp);
        if let Some(xm) = self.xp.multiplier_entry(&perk) {
            let p = perk.borrow();
            let xp_min = p.total_xp_for_level(xm.min_level - 1);
            let xp_max = p.total_xp_for_level(xm.max_level);
            drop(p);
            if old_xp >= xp_min && new_xp < xp_min || old_xp < xp_max && new_xp >= xp_max {
                self.xp.remove_multiplier(&perk);
            }
        }
        let total = |c: &Self, extra: i32| perk.borrow().total_xp_for_level(c.perk_level(Some(&perk)) + extra);
        let mut next = total(self, 1);
        while old_xp < next && new_xp >= next {
            self.level_perk(chars, Some(&perk), ctx)?;
            if self.is_player() {
                let capped =
                    (is(&perk, strength.as_ref()) || is(&perk, fitness.as_ref())) && self.perk_level(Some(&perk)) == 10;
                if !capped {
                    ctx.event(SimEvent::GainExperienceLevelSound);
                }
            }
            if self.perk_level(Some(&perk)) >= 10 {
                break;
            }
            next = total(self, 1);
        }
        let mut this_level = total(self, 0);
        while old_xp >= this_level && new_xp < this_level {
            self.lose_level(chars, &perk, ctx);
            if self.perk_level(Some(&perk)) >= 10 {
                break;
            }
            this_level = total(self, 0);
        }
        if self.is_player() && is(&perk, fitness.as_ref()) {
            let f = chars.sim.character_stats.field("FITNESS")?;
            self.stats.set(&f, self.perk_level(Some(&perk)) as f32 / 5.0 - 1.0);
        }
        if !self.is_player() {
            halo_text = false;
        }
        let new_total = self.xp.xp(&perk);
        if halo_text && new_total > old_xp {
            let award = java_round(new_xp - old_xp) as f32 / 10.0;
            if award > 0.0 {
                let name = ctx.get_text(perk.borrow().name());
                ctx.event(SimEvent::Halo(HaloText::Good(format!("{name} XP: {}", java_float_string(award)))));
            }
        }
        if !ctx.is_client() {
            ctx.event(SimEvent::AddXp { perk: perk.clone(), amount });
        }
        Ok(())
    }

    /// `XP.setXPToLevel`.
    pub fn xp_set_xp_to_level(
        &mut self,
        chars: &Characters,
        key: &PerkRef,
        level: i32,
        ctx: &dyn SimContext,
    ) -> ScriptResult {
        let Some(perk) = chars.perks.perk_list.iter().find(|p| Rc::ptr_eq(p, key)).cloned() else {
            return Ok(());
        };
        let total = perk.borrow().total_xp_for_level(level);
        self.xp.put_xp(key, total);
        if ctx.is_debug() && is(&perk, chars.perks.constant("Fitness").as_ref()) {
            let f = chars.sim.character_stats.field("FITNESS")?;
            self.stats.set(&f, level as f32 / 5.0 - 1.0);
        }
        Ok(())
    }

    // ---- traits and professions -------------------------------------------

    /// `modifyTraitXPBoost`: adds (or removes) the trait definition's XP
    /// boosts to the descriptor's.
    pub fn modify_trait_xp_boost(&mut self, def: Option<&CharacterTraitDefinitionRef>, removing: bool) -> ScriptResult {
        let Some(def) = def else { return Ok(()) };
        let d = self.descriptor.as_ref().ok_or_else(|| npe("descriptor"))?;
        let mut d = d.borrow_mut();
        for (perk, v) in &def.borrow().xp_boosts {
            let map = &mut d.xp_boost_map;
            let delta = if removing { -*v } else { *v };
            match map.iter_mut().find(|(p, _)| p == perk) {
                Some(e) => e.1 += delta,
                None => map.push((perk.clone(), delta)),
            }
        }
        Ok(())
    }

    /// `applyTraits`: sets the traits, then levels every perk to the sum of
    /// 5 (Fitness, Strength), the traits' boosts and the descriptor's boosts
    /// (clamped to 0..=10), storing each total (at most 3) as the
    /// descriptor's boost and the XP at the reached level.
    pub fn apply_traits(
        &mut self,
        sm: &ScriptManager,
        chars: &Characters,
        traits: Option<&[Option<Rc<RegistryKey>>]>,
        ctx: &dyn SimContext,
    ) -> ScriptResult
    where
        I: 'static,
    {
        let Some(traits) = traits else { return Ok(()) };
        let perks = &chars.perks;
        let mut levels: Vec<(PerkRef, i32)> = Vec::new();
        let add = |levels: &mut Vec<(PerkRef, i32)>, perk: PerkRef, v: i32| match levels
            .iter_mut()
            .find(|(p, _)| Rc::ptr_eq(p, &perk))
        {
            Some(e) => e.1 += v,
            None => levels.push((perk, v)),
        };
        for f in ["Fitness", "Strength"] {
            let p = perks.constant(f).ok_or_else(|| npe(f))?;
            match levels.iter_mut().find(|(q, _)| Rc::ptr_eq(q, &p)) {
                Some(e) => e.1 = 5,
                None => levels.push((p, 5)),
            }
        }
        for t in traits.iter().flatten() {
            self.character_traits.set(Some(t), true)?;
            let Some(def) = character_trait_definition(sm, Some(t)) else {
                continue;
            };
            for (id, v) in &def.borrow().xp_boosts {
                let p = perks.from_string(id).ok_or_else(|| npe("perk"))?;
                add(&mut levels, p, *v);
            }
        }
        if self.is_player() {
            if let Some(mut n) = self.nutrition.take() {
                let r = n.apply_weight_from_traits(&|f| self.has_trait(sm, f));
                self.nutrition = Some(n);
                r?;
            }
        }
        let d = self.descriptor.clone().ok_or_else(|| npe("descriptor"))?;
        let boosts = d.borrow().xp_boost_map.clone();
        for (id, v) in boosts {
            let p = perks.from_string(&id).ok_or_else(|| npe("perk"))?;
            add(&mut levels, p, v);
        }
        for (perk, level) in levels {
            let level = level.clamp(0, 10);
            {
                let mut d = d.borrow_mut();
                let id = perk.borrow().id().to_string();
                let v = level.min(3);
                match d.xp_boost_map.iter_mut().find(|(p, _)| *p == id) {
                    Some(e) => e.1 = v,
                    None => d.xp_boost_map.push((id, v)),
                }
            }
            for _ in 0..level {
                self.level_perk(chars, Some(&perk), ctx)?;
            }
            let reached = self.perk_level(Some(&perk));
            self.xp_set_xp_to_level(chars, &perk, reached, ctx)?;
        }
        Ok(())
    }

    /// `applyProfessionRecipes`.
    pub fn apply_profession_recipes(&mut self, sm: &ScriptManager) -> ScriptResult
    where
        I: 'static,
    {
        let d = self.descriptor.as_ref().ok_or_else(|| npe("descriptor"))?;
        let prof = d.borrow().character_profession.clone();
        let def = character_profession_definition(sm, prof.as_ref()).ok_or_else(|| npe("profession definition"))?;
        self.known_recipes.extend(def.borrow().granted_recipes.iter().cloned());
        Ok(())
    }

    /// `applyCharacterTraitsRecipes`.
    pub fn apply_character_traits_recipes(&mut self, sm: &ScriptManager) {
        for (t, on) in self.character_traits.traits() {
            if !on {
                continue;
            }
            if let Some(def) = character_trait_definition(sm, Some(t)) {
                self.known_recipes.extend(def.borrow().granted_recipes.iter().cloned());
            }
        }
    }
}

fn index_error(index: i32, len: usize) -> ScriptError {
    ScriptError::new(format!("java.lang.ArrayIndexOutOfBoundsException: Index {index} out of bounds for length {len}"))
}

/// `Math.round(float)`: floor(x + 0.5) as an int, NaN to 0, saturating.
pub fn java_round(x: f32) -> i32 {
    if x.is_nan() {
        return 0;
    }
    (x as f64 + 0.5).floor() as i32
}
