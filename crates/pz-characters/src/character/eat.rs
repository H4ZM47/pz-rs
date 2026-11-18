//! The character half of `IsoGameCharacter.Eat(item, percentage, useUtensil)`:
//! the stat, nutrition and sickness changes. The caller then runs the food's
//! `OnEat` function and consumes the item.

use pz_inventory::InventoryItem;
use pz_scripts::{ScriptManager, ScriptResult};

use super::context::SimContext;
use super::GameCharacter;
use crate::Characters;

impl<I: 'static> GameCharacter<I> {
    /// `Eat(food, percentage, useUtensil)` up to and including
    /// `BodyDamage.JustAteFood`. Returns the percentage the food values
    /// were scaled by (the original rewrites its `percentage`), or `None`
    /// for an item that is not food.
    pub fn eat_food(
        &mut self,
        sm: &ScriptManager,
        chars: &Characters,
        ctx: &dyn SimContext,
        food: &InventoryItem,
        percentage: f32,
        use_utensil: bool,
    ) -> ScriptResult<Option<f32>> {
        if !matches!(food.kind, pz_inventory::item::ItemKind::Food(_)) {
            return Ok(None);
        }
        let mut pct = percentage.clamp(0.0, 1.0);
        let hung = food.get_hung_change();
        let thirst = food.get_thirst_change();
        if food.get_base_hunger() != 0.0 && hung != 0.0 {
            let eaten = food.get_base_hunger() * pct;
            pct = (eaten / hung).clamp(0.0, 1.0);
        }
        if hung < 0.0 && hung * (1.0 - pct) > -0.01 {
            pct = 1.0;
        }
        if hung == 0.0 && thirst < 0.0 && thirst * (1.0 - pct) > -0.01 {
            pct = 1.0;
        }
        let stat = |name: &str| Self::stat_ref_pub(chars, name);
        self.stats.add(&stat("THIRST")?, food.get_thirst_change() * pct);
        self.stats.add(&stat("HUNGER")?, food.get_hunger_change() * pct);
        self.stats.add(&stat("ENDURANCE")?, food.get_endurance_change() * pct);
        self.stats.add(&stat("STRESS")?, food.get_stress_change() * pct);
        self.stats.add(&stat("FATIGUE")?, food.get_fatigue_change() * pct);
        if let Some(n) = self.nutrition.as_mut() {
            // Burnt food gives a fifth of its nutrition.
            let k = if food.is_burnt() { pct / 5.0 } else { pct };
            n.set_calories(n.calories() + food.get_calories() * k);
            n.set_carbohydrates(n.carbohydrates() + food.get_carbohydrates() * k);
            n.set_proteins(n.proteins() + food.get_proteins() * k);
            n.set_lipids(n.lipids() + food.get_lipids() * k);
        }
        self.body_damage.pain_reduction += food.get_pain_reduction() * pct;
        self.body_damage.cold_reduction += food.get_flu_reduction() as f32 * pct;
        let sickness = stat("FOOD_SICKNESS")?;
        if self.stats.is_above_minimum(&sickness)
            && food.get_food_sickness_change() < 0
            && self.effective_edible_buff_timer <= 0.0
        {
            let cure = food.get_food_sickness_change().abs() as f32 * pct;
            self.stats.remove(&sickness, cure);
            self.stats.remove(&stat("POISON")?, cure);
            let (min, max) = if self.has_trait(sm, "IRON_GUT")? {
                (80.0, 150.0)
            } else if self.has_trait(sm, "WEAK_STOMACH")? {
                (200.0, 280.0)
            } else {
                (120.0, 230.0)
            };
            self.effective_edible_buff_timer = ctx.rand_next_float(min, max);
        }
        self.just_ate_food(sm, chars, ctx, food, pct, use_utensil)?;
        Ok(Some(pct))
    }
}
