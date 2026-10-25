//! Nutrition, Thermoregulator and Fitness with invented data: the
//! `BodyPartType`/`Metabolics` field names are the original's code symbols,
//! every number and id behind them is made up.

use std::cell::Cell;

use pz_inventory::ItemClass;
use pz_scripts::objects::items::{CharacterProfession, CharacterTrait, ItemBodyLocation, ItemTag, WeaponCategory};
use pz_scripts::registry::builtin::BodyPartDecl;
use pz_scripts::ScriptManager;

use super::super::tests::{fixture, give_item, item_constant, player, visual, Recorder};
use super::super::*;
use super::{true_insulation_value, BODY_PARTS};
use crate::Characters;

/// The fixture plus invented body parts and metabolics.
pub(crate) fn body_fixture() -> (ScriptManager, Characters) {
    let (sm, mut chars) = fixture();
    {
        let mut t = sm.builtin_tables.borrow_mut();
        let mut fields: Vec<&str> = BODY_PARTS.iter().map(|(f, _)| *f).collect();
        fields.push("MAX");
        t.body_parts = fields
            .iter()
            .enumerate()
            .map(|(i, f)| BodyPartDecl {
                field: f.to_string(),
                name: f.to_lowercase(),
                skin_surface: 0.01 * (i + 1) as f32,
                dist_to_core: (i % 5) as f32 * 0.2,
                ..Default::default()
            })
            .collect();
        t.metabolics = [
            ("Default", 1.25),
            ("DefaultExercise", 2.5),
            ("Walking2kmh", 1.75),
            ("Walking5kmh", 2.75),
            ("Running10kmh", 5.5),
            ("Running15kmh", 7.5),
            ("LightWork", 2.25),
            ("MediumWork", 3.25),
            ("HeavyWork", 4.5),
            ("UsingTools", 2.0),
            ("Fitness", 4.25),
            ("FitnessHeavy", 6.5),
            ("MAX", 9.0),
        ]
        .iter()
        .map(|(n, m)| (n.to_string(), *m))
        .collect();
    }
    for (field, id) in [
        ("UNEMPLOYED", "t:idle"),
        ("FIRE_OFFICER", "t:hose"),
        ("FITNESS_INSTRUCTOR", "t:coach"),
        ("SECURITY_GUARD", "t:door"),
    ] {
        CharacterProfession::register(&sm, id).unwrap();
        sm.statics.items.borrow_mut().constants.insert("character_profession", field, id);
    }
    chars.sim.body_part_types = BodyPartTypes::from_tables(&sm.builtin_tables.borrow());
    (sm, chars)
}

/// A context with nutrition on and controllable time.
#[derive(Default)]
struct Ctx {
    rec: Recorder,
    minutes: Cell<i32>,
    millis: Cell<i64>,
    nutrition_off: bool,
}

impl SimContext for Ctx {
    fn multiplier(&self) -> f32 {
        8.0
    }
    fn nutrition_enabled(&self) -> bool {
        !self.nutrition_off
    }
    fn minutes(&self) -> i32 {
        self.minutes.get()
    }
    fn calendar_time_in_millis(&self) -> i64 {
        self.millis.get()
    }
    fn rand_next_range(&self, min: i32, _max: i32) -> i32 {
        min + 1
    }
    fn event(&self, e: SimEvent) {
        self.rec.event(e)
    }
}

#[test]
fn nutrition_burns_and_gains_weight() {
    let (sm, mut chars) = body_fixture();
    let mut c = player(&sm, &mut chars);
    let n = c.nutrition.as_ref().unwrap();
    assert_eq!((n.calories(), n.weight()), (800.0, 80.0));
    // Disabled in the sandbox options: nothing changes.
    let off = Ctx { nutrition_off: true, ..Default::default() };
    c.nutrition_update(&sm, &chars, &off).unwrap();
    assert_eq!(c.nutrition.as_ref().unwrap().calories(), 800.0);
    // Idle: the resting burn.
    let ctx = Ctx::default();
    let secs = ctx.game_world_seconds_since_last_update();
    assert_eq!(secs, 8.0 / 0.8 / 1.0 / 60.0 * (1440.0 / 30.0));
    c.nutrition_update(&sm, &chars, &ctx).unwrap();
    let n = c.nutrition.as_ref().unwrap();
    assert_eq!(n.carbohydrates(), 0.0 - 0.0035 * secs);
    assert_eq!(n.calories(), 800.0 - 0.016 * 1.0 * 1.0 * 1.0 * secs);
    assert!(!n.is_inc_weight() && !n.is_dec_weight());
    // Running burns more.
    c.running = true;
    c.player_moving = true;
    let before = c.nutrition.as_ref().unwrap().calories();
    c.nutrition_update(&sm, &chars, &ctx).unwrap();
    assert_eq!(c.nutrition.as_ref().unwrap().calories(), before - 0.13 * 1.0 * 1.0 * secs);
    // A big meal with many carbohydrates gains weight fast.
    let n = c.nutrition.as_mut().unwrap();
    n.set_calories(5000.0);
    assert_eq!(n.calories(), 3700.0);
    n.set_carbohydrates(900.0);
    let w = n.weight();
    c.nutrition_update(&sm, &chars, &ctx).unwrap();
    let n = c.nutrition.as_ref().unwrap();
    assert!(n.is_inc_weight() && n.is_inc_weight_lot());
    assert!(n.weight() > w);
    // Starving loses weight.
    c.nutrition.as_mut().unwrap().set_calories(-2200.0);
    let w = c.nutrition.as_ref().unwrap().weight();
    c.nutrition_update(&sm, &chars, &ctx).unwrap();
    assert!(c.nutrition.as_ref().unwrap().is_dec_weight());
    assert!(c.nutrition.as_ref().unwrap().weight() < w);
    // Clamps.
    let n = c.nutrition.as_mut().unwrap();
    n.set_proteins(-900.0);
    n.set_lipids(2000.0);
    assert_eq!((n.proteins(), n.lipids()), (-500.0, 1000.0));
    assert!(n.set_weight(20.0));
    assert_eq!(n.weight(), 35.0);
}

#[test]
fn weight_and_traits() {
    let (sm, mut chars) = body_fixture();
    let mut c = player(&sm, &mut chars);
    for (w, t) in
        [(101.0, "OBESE"), (90.0, "OVERWEIGHT"), (70.0, "UNDERWEIGHT"), (60.0, "VERY_UNDERWEIGHT"), (50.0, "EMACIATED")]
    {
        c.nutrition.as_mut().unwrap().set_weight(w);
        c.apply_trait_from_weight(&sm).unwrap();
        for other in ["OBESE", "OVERWEIGHT", "UNDERWEIGHT", "VERY_UNDERWEIGHT", "EMACIATED"] {
            assert_eq!(c.has_trait(&sm, other).unwrap(), other == t, "{w} {other}");
        }
    }
    // The traits set the starting weight (applyTraits).
    let mut d = player(&sm, &mut chars);
    let ctx = Recorder::default();
    d.apply_traits(&sm, &chars, Some(&[CharacterTrait::constant(&sm, "OVERWEIGHT")]), &ctx).unwrap();
    assert_eq!(d.nutrition.as_ref().unwrap().weight(), 95.0);
}

#[test]
fn thermoregulator_nodes_and_cold() {
    let (sm, mut chars) = body_fixture();
    let mut c = player(&sm, &mut chars);
    let ctx = Ctx::default();
    {
        let t = c.body_damage.thermoregulator().unwrap();
        assert_eq!(t.node_size(), 17);
        let core = t.core_node().unwrap();
        assert_eq!(t.nodes()[core].body_part_type(), "Torso_Upper");
        assert!(t.nodes()[core].is_core() && !t.nodes()[core].has_upstream() && t.nodes()[core].has_downstream());
        let hand = t.nodes().iter().find(|n| n.body_part_type() == "Hand_L").unwrap();
        assert!(hand.has_upstream() && !hand.has_downstream());
        // Values come from the table, the index is the BodyPartType ordinal.
        let decl = &sm.builtin_tables.borrow().body_parts[hand.body_part_index()];
        assert_eq!((decl.field.as_str(), hand.skin_surface()), ("Hand_L", decl.skin_surface));
        assert_eq!((t.core_temperature().unwrap(), t.metabolic_rate()), (37.0, 1.25));
        assert_eq!(t.thermal_chevron_count(), 0);
    }
    // The target only rises, up to MAX.
    c.set_metabolic_target(1.0).unwrap();
    assert_eq!(c.body_damage.thermoregulator().unwrap().metabolic_target(), 1.25);
    c.set_metabolic_target(20.0).unwrap();
    assert_eq!(c.body_damage.thermoregulator().unwrap().metabolic_target(), 9.0);
    // A hat (no covered parts) goes on the head.
    let hat = item_constant(&sm, ItemBodyLocation::REGISTRY, "HAT");
    give_item(&sm, &mut c, ItemClass::Clothing, "Hat", 7, |t| t.insulation = 0.5, |s| s.body_location = Some(hat));
    c.items.worn_items = vec![7];
    // Without a visual (no clothing asset) it is not in the item visuals.
    c.thermoregulator_update(&sm, &chars, &ctx).unwrap();
    assert_eq!(c.body_damage.thermoregulator().unwrap().nodes().iter().map(|n| n.insulation()).sum::<f32>(), 0.0);
    c.items.item_mut(7).unwrap().visual = Some(visual(18));
    c.environment.air_temperature = -10.0;
    c.environment.air_and_wind_temperature = -14.0;
    for _ in 0..400 {
        c.thermoregulator_update(&sm, &chars, &ctx).unwrap();
    }
    let t = c.body_damage.thermoregulator().unwrap();
    let head = t.nodes().iter().find(|n| n.body_part_type() == "Head").unwrap();
    assert_eq!(head.insulation(), true_insulation_value(0.5) * 1.0 + 0.05);
    let hand = t.nodes().iter().find(|n| n.body_part_type() == "Hand_L").unwrap();
    assert_eq!(hand.insulation(), 0.0);
    assert!(hand.skin_celcius() < 33.0);
    let core = t.core_temperature().unwrap();
    assert!(core < 37.0);
    assert_eq!(c.stat(&chars, "TEMPERATURE").unwrap(), core);
    assert!(t.energy_multiplier() > 1.0 && t.fluids_multiplier() == 1.0);
    assert!(!t.thermal_chevron_up());
    assert!(t.timed_action_time_modifier() > 1.0);
    assert_eq!(c.body_damage.cold_damage_stage, 0.0);
    assert_eq!(t.metabolic_target(), -1.0);
}

fn exercise(ty: &str, stiffness: &str, met: &str, xp: f32) -> (String, FitnessExercise) {
    (ty.to_string(), FitnessExercise::new(Some(ty.to_string()), Some(met.to_string()), stiffness, xp))
}

#[test]
fn fitness_regularity_stiffness_and_xp() {
    let (sm, mut chars) = body_fixture();
    let mut c = player(&sm, &mut chars);
    let ctx = Ctx::default();
    let list = vec![exercise("hop", "legs", "Fitness", -1.0), exercise("lift", "arms,chest", "FitnessHeavy", 2.0)];
    c.fitness_init(&sm, Some(list.clone()), &ctx).unwrap();
    // No regularity without a fitness profession; init runs once.
    assert!(c.fitness.as_ref().unwrap().regularity_map().is_empty());
    c.fitness_init(&sm, Some(vec![exercise("x", "abs", "Fitness", 1.0)]), &ctx).unwrap();
    assert_eq!(c.fitness.as_ref().unwrap().exercises().len(), 2);
    // Without a current exercise a repetition is the original's NPE.
    assert!(c.fitness_exercise_repeat(&sm, &chars, &ctx).is_err());
    c.fitness.as_mut().unwrap().set_current_exercise(Some("lift"));
    c.stats.set(&chars.sim.character_stats.field("ENDURANCE").unwrap(), 1.0);
    ctx.millis.set(5000);
    c.fitness_exercise_repeat(&sm, &chars, &ctx).unwrap();
    let f = c.fitness.as_ref().unwrap();
    let inc = (f64::from(0.08f32) * (5.0f64.ln() / f64::from(0.0f32 / 5.0 + 4.0).ln())) as f32;
    assert_eq!(f.regularity("lift"), inc);
    assert_eq!(f.current_exe_stiffness_timer("arms,legs").unwrap(), 72);
    assert_eq!(f.current_exe_stiffness_timer("legs").unwrap(), 0);
    let stiff = 0.5 * ((120.0 - inc) / 170.0) * 1.3 * 1.0;
    assert_eq!(f.current_exe_stiffness_inc("chest").unwrap(), stiff);
    assert_eq!(f.exe_timer().get("lift"), Some(&5000));
    // XP: arms 4 + chest 2 Strength, times the exercise's modifier.
    assert_eq!(c.xp.xp(&chars.perks.constant("Strength").unwrap()), 6.0 * 2.0);
    assert_eq!(c.xp.xp(&chars.perks.constant("Fitness").unwrap()), 0.0);
    let red = (f64::from(0.015f32) * (f64::from(inc / 50.0 + 50.0).ln() / 51.0f64.ln())) as f32 * 1.3;
    assert_eq!(c.stat(&chars, "ENDURANCE").unwrap(), 1.0 - red);
    // Every ten minutes the timers run down; at zero the stiffness turns
    // into body-part stiffness, 2.5 per update, until it is used up.
    let arm = sm.builtin_tables.borrow().body_parts.iter().position(|p| p.field == "UpperArm_R").unwrap();
    let torso = sm.builtin_tables.borrow().body_parts.iter().position(|p| p.field == "Torso_Upper").unwrap();
    c.fitness_update(&sm, &ctx).unwrap();
    for m in 1..=73 {
        ctx.minutes.set(m * 10);
        c.fitness_update(&sm, &ctx).unwrap();
    }
    let f = c.fitness.as_ref().unwrap();
    assert!(f.stiffness_timer_map().is_empty());
    assert!(!f.on_going_stiffness());
    assert_eq!(c.body_damage.body_part_stiffness(arm), 2.5);
    assert_eq!(c.body_damage.body_part_stiffness(torso), 2.5);
    // A day after the last repetition the regularity decays.
    ctx.millis.set(5000 + 86_400_001);
    ctx.minutes.set(0);
    c.fitness_update(&sm, &ctx).unwrap();
    assert_eq!(c.fitness.as_ref().unwrap().regularity("lift"), inc - 0.002);
}

#[test]
fn fitness_profession_regularity() {
    let (sm, mut chars) = body_fixture();
    let mut c = player(&sm, &mut chars);
    c.descriptor.as_ref().unwrap().borrow_mut().character_profession =
        CharacterProfession::constant(&sm, "FIRE_OFFICER");
    let ctx = Ctx::default();
    let list = vec![exercise("hop", "legs", "Fitness", 1.0), exercise("lift", "arms", "FitnessHeavy", 1.0)];
    c.fitness_init(&sm, Some(list), &ctx).unwrap();
    // Fire officers: Rand.Next(10, 20) (this context's Rand returns min + 1).
    let f = c.fitness.as_ref().unwrap();
    assert_eq!((f.regularity("hop"), f.regularity("lift")), (11.0, 11.0));
}

/// A context whose `Rand.Next(n)` is `n / 2` and that records events.
#[derive(Default)]
struct HalfRand {
    rec: Recorder,
}

impl SimContext for HalfRand {
    fn zomboid_globals(&self) -> &ZomboidGlobals {
        &self.rec.globals
    }
    fn rand_next(&self, max: i32) -> i32 {
        max / 2
    }
    fn rand_next_range(&self, min: i32, max: i32) -> i32 {
        (min + max) / 2
    }
    fn rand_next_float(&self, min: f32, _max: f32) -> f32 {
        min
    }
    fn event(&self, e: SimEvent) {
        self.rec.event(e)
    }
}

fn food(t: &mut pz_inventory::ItemTemplate) {
    t.poison_power = 6;
    t.hunger_change = -20.0;
    t.boredom_change = -4.0;
    t.unhappy_change = 2.0;
    t.dangerous_uncooked = true;
}

#[test]
fn food_pills_and_books_are_items() {
    let (sm, mut chars) = body_fixture();
    let mut c = player(&sm, &mut chars);
    let ctx = HalfRand::default();
    let st = |c: &GameCharacter<()>, f| c.stat(&chars, f).unwrap();
    let egg = item_constant(&sm, ItemTag::REGISTRY, "EGG");
    give_item(&sm, &mut c, ItemClass::Food, "Stew", 1, food, |_| {});
    give_item(&sm, &mut c, ItemClass::Food, "Omelette", 2, |t| {
        food(t);
        t.tags = vec![egg.location.to_string()];
    }, |_| {});
    let stew = c.items.item(1).unwrap().clone();
    c.just_ate_food(&sm, &chars, &ctx, &stew, 0.5, false).unwrap();
    // Poison 6 * 0.5, plus 15 * 0.5 for dangerous uncooked food.
    assert_eq!(st(&c, "POISON"), 3.0 + 7.5);
    assert_eq!(st(&c, "PAIN"), 6.0 * 0.5 / 6.0);
    assert_eq!(st(&c, "UNHAPPINESS"), 2.0 * 0.5);
    // Eggs: the chance is 5, still above 0.
    let omelette = c.items.item(2).unwrap().clone();
    c.just_ate_food(&sm, &chars, &ctx, &omelette, 1.0, true).unwrap();
    assert_eq!(st(&c, "POISON"), 10.5 + 6.0 + 15.0);
    // Pills: the original's types, and one use per pill.
    give_item(&sm, &mut c, ItemClass::Drainable, "PillsVitamins", 3, |t| t.fatigue_change = -10.0, |_| {});
    c.items.item_mut(3).unwrap().uses = 5;
    c.just_took_held_pill(&sm, &chars, &ctx, 3).unwrap().unwrap();
    assert_eq!((st(&c, "FATIGUE"), c.items.item(3).unwrap().uses), (0.0, 4));
    c.stats.set(&chars.sim.character_stats.field("FATIGUE").unwrap(), 0.5);
    c.just_took_held_pill(&sm, &chars, &ctx, 3).unwrap().unwrap();
    assert_eq!((st(&c, "FATIGUE"), c.items.item(3).unwrap().uses), (0.4, 3));
    // A loose pill: the outcome is the caller's (no container to leave).
    let mut last = c.items.item(3).unwrap().clone();
    last.uses = 1;
    let out = c.just_took_pill(&sm, &chars, &ctx, &mut last, &pz_inventory::context::Loose).unwrap();
    assert_eq!(last.uses, 0);
    assert!(!out.remove && !out.remove_from_hands);
    // The last pill in the inventory is used up and leaves it.
    c.items.item_mut(3).unwrap().uses = 1;
    let out = c.just_took_held_pill(&sm, &chars, &ctx, 3).unwrap().unwrap();
    assert!(out.remove && out.remove_from_hands);
    assert!(c.items.item(3).is_none());
    give_item(&sm, &mut c, ItemClass::Literature, "Comic", 4, |t| t.boredom_change = -0.25, |_| {});
    let comic = c.items.item(4).unwrap().clone();
    let before = st(&c, "BOREDOM");
    c.just_read_something(&chars, &comic).unwrap();
    assert_eq!(st(&c, "BOREDOM"), (before - 0.25).max(0.0));
}

#[test]
fn worn_clothing_defends_and_weighs() {
    let (sm, mut chars) = body_fixture();
    let mut c = player(&sm, &mut chars);
    let ctx = HalfRand::default();
    let spiked = item_constant(&sm, ItemTag::REGISTRY, "SPIKED");
    give_item(&sm, &mut c, ItemClass::Clothing, "Gloves", 10, |t| {
        t.scratch_defense = 80.0;
        t.bite_defense = 30.0;
        t.tags = vec![spiked.location.to_string()];
    }, |s| s.blood_clothing_type = Some(vec!["Hands".into()]));
    give_item(&sm, &mut c, ItemClass::Clothing, "Mittens", 11, |t| t.scratch_defense = 40.0, |s| {
        s.blood_clothing_type = Some(vec!["Hand_L".into()])
    });
    c.items.worn_items = vec![10, 11];
    // No visuals yet: nothing is worn as far as the visuals go.
    assert_eq!(c.items.body_part_clothing_defense(&sm, 0, false, false), 0.0);
    for id in [10, 11] {
        c.items.item_mut(id).unwrap().visual = Some(visual(18));
    }
    let hand_l = 0;
    assert_eq!(c.items.body_part_clothing_defense(&sm, hand_l, false, false), 100.0);
    assert_eq!(c.items.body_part_clothing_defense(&sm, 1, true, false), 30.0);
    assert!(c.items.body_part_is_spiked(&sm, 1) && !c.items.body_part_is_spiked_behind(&sm, 1));
    // A hole removes a layer for that part.
    c.items.item_mut(10).unwrap().visual.as_mut().unwrap().arrays[2].as_mut().unwrap()[1] = -1;
    assert_eq!(c.items.body_part_clothing_defense(&sm, 1, false, false), 0.0);
    // Dirt and blood above the thresholds.
    c.items.item_mut(11).unwrap().visual.as_mut().unwrap().arrays[1].as_mut().unwrap()[0] = 40;
    c.items.item_mut(10).unwrap().visual.as_mut().unwrap().arrays[0].as_mut().unwrap()[0] = 70;
    assert!(c.items.has_dirty_clothing(&sm, 0) && c.items.has_bloody_clothing(&sm, 0));
    assert!(!c.items.has_dirty_clothing(&sm, 1));

    // A blunt weapon on the covered hand: 50 < 100, the armour holds.
    let blunt = item_constant(&sm, WeaponCategory::REGISTRY, "BLUNT");
    give_item(&sm, &mut c, ItemClass::Weapon, "Bat", 20, |t| t.weapon_categories = vec![blunt.location.to_string()], |_| {});
    let bat = c.items.item(20).unwrap().clone();
    c.damage_from_weapon(&sm, &chars, &ctx, &bat, None, hand_l).unwrap();
    assert_eq!(
        ctx.rec.events.borrow().clone(),
        [
            "AddHoleFromZombieAttacks { part: 0, scratch_or_bite: false }",
            "WeaponHitArmourSound { part: 0, bullet: false }"
        ]
    );
    assert_eq!(c.body_damage.parts[0].scratch_time, 0.0);

    // Weight: the two worn and the held item count with the multiplier.
    let g = ZomboidGlobals { equipped_or_worn_encumbrance_multiplier: 0.5, ..Default::default() };
    let wctx = HalfRand { rec: Recorder { globals: g, ..Default::default() } };
    give_item(&sm, &mut c, ItemClass::Normal, "Rock", 30, |t| t.actual_weight = 3.0, |_| {});
    c.items.primary_hand_item = Some(20);
    assert_eq!(c.capacity_weight(&sm, &wctx), 0.5 * 3.0 + 3.0);
}

#[test]
fn tissues_and_no_sprint_hand_items() {
    let (sm, mut chars) = body_fixture();
    let mut c = player(&sm, &mut chars);
    give_item(&sm, &mut c, ItemClass::Drainable, "Tissue", 1, |_| {}, |_| {});
    give_item(&sm, &mut c, ItemClass::Normal, "Plank", 2, |_| {}, |s| s.equipped_no_sprint = true);
    c.items.item_mut(1).unwrap().uses = 2;
    let ctx = crate::character::DefaultSimContext;
    // Not in a hand: nothing used.
    assert!(!c.use_tissue(&sm, &ctx));
    c.items.secondary_hand_item = Some(1);
    assert!(c.use_tissue(&sm, &ctx));
    assert_eq!(c.items.item(1).unwrap().uses, 1);
    // At 0 uses it is used up (`Use()`): out of the hand and the inventory.
    assert!(c.use_tissue(&sm, &ctx));
    assert_eq!((c.items.item(1), c.items.secondary_hand_item), (None, None));
    assert!(!c.use_tissue(&sm, &ctx));
    assert!(!c.items.hand_item_equipped_no_sprint(&sm));
    c.items.primary_hand_item = Some(2);
    assert!(c.items.hand_item_equipped_no_sprint(&sm));
}

#[test]
fn rain_wets_the_outer_layer() {
    let (sm, mut chars) = body_fixture();
    let mut c = player(&sm, &mut chars);
    let g = ZomboidGlobals { wetness_increase: 0.5, ..Default::default() };
    let ctx = HalfRand { rec: Recorder { globals: g, ..Default::default() } };
    give_item(&sm, &mut c, ItemClass::Clothing, "Coat", 1, |_| {}, |s| s.blood_clothing_type = Some(vec!["Jacket".into()]));
    give_item(&sm, &mut c, ItemClass::Clothing, "Tee", 2, |_| {}, |s| s.blood_clothing_type = Some(vec!["ShirtNoSleeves".into()]));
    c.items.worn_items = vec![2, 1];
    for id in [1, 2] {
        c.items.item_mut(id).unwrap().visual = Some(visual(18));
    }
    let mut cw = c.clothing_wetness.take().unwrap();
    let thermo = c.body_damage.thermoregulator.take().unwrap();
    cw.update_wetness(&sm, &ctx, &chars.sim.body_part_types, &mut c.items, &mut c.body_damage.parts, &thermo, 1.0, 0.0);
    // The coat (outermost) takes the rain on its parts, split over its 9
    // parts per covered part; the tee and the skin under it stay dry.
    let coat = c.items.item(1).unwrap().get_wetness();
    assert!(coat > 0.0);
    assert_eq!(c.items.item(2).unwrap().get_wetness(), 0.0);
    // The hands have no clothing: the skin gets wet.
    assert_eq!(c.body_damage.parts[0].wetness, 0.5);
}
