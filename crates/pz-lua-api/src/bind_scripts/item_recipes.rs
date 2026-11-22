//! The character overloads of `Item.getUsedInRecipes` and
//! `Item.getResearchableRecipes`, with `CraftRecipe.canResearch`.

use pz_kahlua::LuaResult;
use pz_scripts::objects::entity::components::crafting::CraftRecipe;
use std::cell::RefCell;
use std::rc::Rc;

/// What these checks ask of the `IsoGameCharacter` and the game.
pub(crate) trait RecipeKnowledge {
    /// `SandboxOptions.instance.seeNotLearntRecipe.getValue()`.
    fn see_not_learnt_recipe(&self) -> LuaResult<bool>;
    /// `isRecipeActuallyKnown(String)`.
    fn actually_knows_name(&self, name: &str) -> LuaResult<bool>;
    /// `isKnowAllRecipes()`.
    fn knows_all(&self) -> LuaResult<bool>;
    /// `getKnownRecipes().contains(name)`.
    fn knows(&self, name: &str) -> bool;
    /// `getPerkLevel(Perks.FromString(perk))`.
    fn perk_level(&self, perk: &str) -> i32;
    /// `isInventive()`.
    fn is_inventive(&self) -> LuaResult<bool>;
}

/// `ScriptManager.getCraftRecipe(name)`.
pub(crate) type RecipeLookup<'a> = &'a dyn Fn(&str) -> Option<Rc<RefCell<CraftRecipe>>>;

/// `Item.getUsedInRecipes(chr, new ArrayList())` for a non-null `chr`: the
/// craft recipes the item is used in that need no learning, or are shown
/// unlearnt, or that the character actually knows.
pub(crate) fn used_in_recipes(
    names: &[String],
    recipe: RecipeLookup<'_>,
    k: &dyn RecipeKnowledge,
) -> LuaResult<Vec<String>> {
    let mut out = Vec::new();
    for name in names {
        let Some(r) = recipe(name) else { continue };
        let need = r.borrow().need_to_be_learn();
        if !need || k.see_not_learnt_recipe()? || k.actually_knows_name(name)? {
            out.push(name.clone());
        }
    }
    Ok(out)
}

/// `Item.getResearchableRecipes(chr, b)`: of `getResearchableRecipes()`,
/// the recipes that exist and that `chr` can research.
pub(crate) fn researchable_recipes(
    names: &[String],
    recipe: RecipeLookup<'_>,
    k: &dyn RecipeKnowledge,
    b: bool,
) -> LuaResult<Vec<String>> {
    let mut out = Vec::new();
    for name in names {
        let Some(r) = recipe(name) else { continue };
        if can_research(&mut r.borrow_mut(), k, b)? {
            out.push(name.clone());
        }
    }
    Ok(out)
}

/// `IsoGameCharacter.isRecipeKnown(CraftRecipe, true)`.
fn actually_knows_recipe(r: &CraftRecipe, k: &dyn RecipeKnowledge) -> LuaResult<bool> {
    if k.knows_all()? || !r.need_to_be_learn() {
        return Ok(true);
    }
    let knows = |n: Option<&str>| n.is_some_and(|n| k.knows(n));
    Ok(knows(r.name()) || knows(r.meta_recipe()) || knows(r.translation_name()))
}

/// `CraftRecipe.getResearchSkillLevel(chr)`: two levels lower for an
/// inventive character.
fn research_skill_level(r: &mut CraftRecipe, k: &dyn RecipeKnowledge) -> LuaResult<i32> {
    let mut level = r.get_research_skill_level();
    if k.is_inventive()? {
        level -= 2;
    }
    Ok(CraftRecipe::normalize_skill_level(level))
}

/// `CraftRecipe.canResearch(chr, b)` for a non-null `chr`.
pub(crate) fn can_research(r: &mut CraftRecipe, k: &dyn RecipeKnowledge, b: bool) -> LuaResult<bool> {
    if actually_knows_recipe(r, k)? && b {
        return Ok(false);
    }
    if !r.can_be_researched() {
        return Ok(false);
    }
    if r.can_always_be_researched() {
        return Ok(true);
    }
    let level = research_skill_level(r, k)?;
    if level < 1 {
        return Ok(true);
    }
    if !r.research_all().is_empty() {
        return Ok(r.research_all().iter().all(|p| k.perk_level(p) >= level));
    }
    if r.research_any().iter().any(|p| k.perk_level(p) >= level) {
        return Ok(true);
    }
    let required = r.skill_required.as_deref().unwrap_or_default();
    if required.iter().any(|s| k.perk_level(s.perk()) >= level) {
        return Ok(true);
    }
    if !r.research_any().is_empty() || !required.is_empty() {
        return Ok(false);
    }
    let any = r.auto_learn_any.as_deref().unwrap_or_default();
    if any.iter().any(|s| k.perk_level(s.perk()) >= level) {
        return Ok(true);
    }
    let all_count = r.auto_learn_all.as_ref().map_or(0, Vec::len);
    if all_count == 0 {
        return Ok(false);
    }
    // The original reads `getAutoLearnAnySkill(i)` for the AutoLearnAll
    // count, and that is null past the end of AutoLearnAny.
    let mut result = true;
    for i in 0..all_count {
        let skill = any.get(i).ok_or_else(super::npe)?;
        if k.perk_level(skill.perk()) < level {
            result = false;
        }
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[derive(Default)]
    struct Chr {
        see_not_learnt: bool,
        knows_all: bool,
        known: Vec<&'static str>,
        perks: HashMap<&'static str, i32>,
        inventive: bool,
    }
    impl RecipeKnowledge for Chr {
        fn see_not_learnt_recipe(&self) -> LuaResult<bool> {
            Ok(self.see_not_learnt)
        }
        fn actually_knows_name(&self, name: &str) -> LuaResult<bool> {
            Ok(self.knows_all || self.knows(name))
        }
        fn knows_all(&self) -> LuaResult<bool> {
            Ok(self.knows_all)
        }
        fn knows(&self, name: &str) -> bool {
            self.known.contains(&name)
        }
        fn perk_level(&self, perk: &str) -> i32 {
            self.perks.get(perk).copied().unwrap_or(0)
        }
        fn is_inventive(&self) -> LuaResult<bool> {
            Ok(self.inventive)
        }
    }

    fn recipe(name: &str, body: &str) -> Rc<RefCell<CraftRecipe>> {
        let sm = pz_scripts::ScriptManager::new();
        sm.builtin_tables.borrow_mut().perk_ids = vec!["Woodwork".into(), "Carving".into()];
        let mut r = CraftRecipe::new();
        use pz_scripts::ScriptObject;
        r.load(&sm, name, &format!("craftRecipe {name} {{ {body} }}")).unwrap();
        Rc::new(RefCell::new(r))
    }

    #[test]
    fn used_in_recipes_keeps_unlearnt_only_when_shown_or_known() {
        let free = recipe("Free", "time = 10,");
        let learnt = recipe("Learnt", "time = 10, NeedToBeLearn = true,");
        let lookup = |n: &str| match n {
            "Free" => Some(free.clone()),
            "Learnt" => Some(learnt.clone()),
            _ => None,
        };
        let names: Vec<String> = ["Free", "Learnt", "Missing"].map(String::from).to_vec();
        let chr = Chr::default();
        assert_eq!(used_in_recipes(&names, &lookup, &chr).unwrap(), ["Free"]);
        let chr = Chr { known: vec!["Learnt"], ..Chr::default() };
        assert_eq!(used_in_recipes(&names, &lookup, &chr).unwrap(), ["Free", "Learnt"]);
        let chr = Chr { see_not_learnt: true, ..Chr::default() };
        assert_eq!(used_in_recipes(&names, &lookup, &chr).unwrap(), ["Free", "Learnt"]);
    }

    #[test]
    fn can_research_follows_the_research_lists_and_skill_level() {
        // Not to be learned: never researchable.
        let chr = Chr::default();
        assert!(!can_research(&mut recipe("Free", "time = 10,").borrow_mut(), &chr, true).unwrap());
        // ResearchAll: every perk at the research level (max(1, round(4 * 2 / 3)) = 3).
        let all = recipe("All", "time = 10, NeedToBeLearn = true, SkillRequired = Woodwork:4, ResearchAll = Woodwork;Carving,");
        let lv = |w, c| Chr { perks: HashMap::from([("Woodwork", w), ("Carving", c)]), ..Chr::default() };
        assert!(can_research(&mut all.borrow_mut(), &lv(3, 3), true).unwrap());
        assert!(!can_research(&mut all.borrow_mut(), &lv(3, 2), true).unwrap());
        // Inventive lowers the level by two.
        let inventive = Chr { inventive: true, ..lv(1, 1) };
        assert!(can_research(&mut all.borrow_mut(), &inventive, true).unwrap());
        // Already known: only researchable when b is false.
        let known = Chr { known: vec!["All"], ..lv(3, 3) };
        assert!(!can_research(&mut all.borrow_mut(), &known, true).unwrap());
        assert!(can_research(&mut all.borrow_mut(), &known, false).unwrap());
        // No research lists: the required skills.
        let req = recipe("Req", "time = 10, NeedToBeLearn = true, SkillRequired = Woodwork:4,");
        assert!(can_research(&mut req.borrow_mut(), &lv(3, 0), true).unwrap());
        assert!(!can_research(&mut req.borrow_mut(), &lv(2, 0), true).unwrap());
    }
}
