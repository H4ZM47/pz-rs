//! Tests with invented ids. The Java field names are the original's code
//! symbols; the ids behind them are made up.

use std::cell::RefCell;
use std::rc::Rc;

use pz_inventory::saved::SavedItem;
use pz_inventory::{InventoryItem, ItemClass, ItemTemplate};
use pz_scripts::objects::items::{add_character_trait_definition, get_item, CharacterTrait, Item, MoodleType};
use pz_scripts::registry::builtin::{BuiltinTables, CharacterStatDecl, MoodleStatDecl, PerkDecl, PerkInit};
use pz_scripts::base::{ScriptObject, ScriptRef};
use pz_scripts::script_type::ScriptLoadMode;
use pz_scripts::ScriptManager;

use super::*;
use crate::survivor::SurvivorDesc;
use crate::{PerkFactory, PerkRef};

pub(crate) const TRAITS: &[&str] = &[
    "FAST_LEARNER",
    "SLOW_LEARNER",
    "PACIFIST",
    "CRAFTY",
    "HIGH_THIRST",
    "LOW_THIRST",
    "DEAF",
    "HEMOPHOBIC",
    "NEEDS_LESS_SLEEP",
    "NEEDS_MORE_SLEEP",
    "HEARTY_APPETITE",
    "LIGHT_EATER",
    "INSOMNIAC",
    "NIGHT_OWL",
    "OBESE",
    "OVERWEIGHT",
    "VERY_UNDERWEIGHT",
    "EMACIATED",
    "UNDERWEIGHT",
    "WEIGHT_GAIN",
    "WEIGHT_LOSS",
    "AGORAPHOBIC",
    "CLAUSTROPHOBIC",
    "IRON_GUT",
    "WEAK_STOMACH",
    "ASTHMATIC",
    "STRONG",
    "WEAK",
    "THICK_SKINNED",
    "THIN_SKINNED",
    "INCONSPICUOUS",
    "CONSPICUOUS",
    "FAST_HEALER",
    "SLOW_HEALER",
];

const STATS: &[&str] = &[
    "ANGER",
    "BOREDOM",
    "DISCOMFORT",
    "ENDURANCE",
    "FATIGUE",
    "FITNESS",
    "FOOD_SICKNESS",
    "HUNGER",
    "IDLENESS",
    "INTOXICATION",
    "MORALE",
    "NICOTINE_WITHDRAWAL",
    "PAIN",
    "PANIC",
    "SANITY",
    "SICKNESS",
    "STRESS",
    "TEMPERATURE",
    "THIRST",
    "UNHAPPINESS",
    "WETNESS",
    "ZOMBIE_FEVER",
    "ZOMBIE_INFECTION",
    "POISON",
];

const MOODLES: &[&str] = &[
    "PANIC",
    "TIRED",
    "ENDURANCE",
    "STRESS",
    "PAIN",
    "DEAD",
    "ZOMBIE",
    "FOOD_EATEN",
    "HYPOTHERMIA",
    "HYPERTHERMIA",
    "HEAVY_LOAD",
];

const PERKS: &[&str] = &["None", "Fitness", "Strength", "Sprinting", "Crafting", "Carving", "Axe", "Juggle", "MAX"];

/// A fixture: invented ranges, thresholds and XP tables.
pub(crate) fn fixture() -> (ScriptManager, Characters) {
    let sm = ScriptManager::new();
    for (i, f) in TRAITS.iter().enumerate() {
        let id = format!("t:trait{i}");
        CharacterTrait::register(&sm, &id).unwrap();
        sm.statics.items.borrow_mut().constants.insert("character_trait", f, &id);
    }
    for (i, f) in MOODLES.iter().enumerate() {
        let id = format!("t:moodle{i}");
        MoodleType::register(&sm, &id).unwrap();
        sm.statics.items.borrow_mut().constants.insert("moodle_type", f, &id);
    }
    let mut t = BuiltinTables::default();
    for f in STATS {
        let (min, max, def) = match *f {
            "ENDURANCE" | "MORALE" | "SANITY" => (0.0, 1.0, 1.0),
            "FITNESS" => (-1.0, 1.0, 0.0),
            "TEMPERATURE" => (20.0, 40.0, 37.0),
            "PANIC" | "PAIN" | "POISON" => (0.0, 100.0, 0.0),
            _ => (0.0, 1.0, 0.0),
        };
        t.character_stats.push(CharacterStatDecl {
            field: f.to_string(),
            id: f.to_lowercase(),
            minimum: min,
            maximum: max,
            default: def,
        });
    }
    t.ordered_stats = STATS.iter().map(|s| s.to_string()).collect();
    let ms =
        |field: &str, th: [f32; 5]| MoodleStatDecl { field: field.into(), moodle_type: field.into(), thresholds: th };
    t.moodle_stats = vec![
        ms("ENDURANCE", [0.8, 0.6, 0.4, 0.2, 0.0]),
        ms("TIRED", [0.0, 0.5, 0.6, 0.7, 0.8]),
        ms("PANIC", [0.0, 10.0, 20.0, 40.0, 80.0]),
        ms("STRESS", [0.0, 0.2, 0.4, 0.6, 0.8]),
        ms("PAIN", [0.0, 10.0, 20.0, 50.0, 70.0]),
        ms("HYPOTHERMIA", [0.0, 30.0, 70.0, 9.0, 100.0]),
        ms("HYPERTHERMIA", [0.0, 30.0, 70.0, 9.0, 100.0]),
        ms("HEAVY_LOAD", [0.0, 0.25, 0.5, 0.75, 1.0]),
    ];
    t.level_up_levels = vec![10, 20, 40];
    t.cheat_types = vec!["GOD_MODE".into(), "UNLIMITED_ENDURANCE".into(), "INVISIBLE".into()];
    t.perks = PERKS.iter().map(|f| PerkDecl { field: f.to_string(), id: f.to_string(), parent: None }).collect();
    t.perk_ids = PERKS.iter().map(|s| s.to_string()).collect();
    let xp = [10, 20, 30, 40, 50, 60, 70, 80, 90, 100];
    let add = |perk: &str, parent: Option<&str>, passiv| PerkInit::AddPerk {
        perk: perk.into(),
        translation: perk.into(),
        parent: parent.map(str::to_string),
        xp,
        passiv,
    };
    t.perk_init = vec![
        PerkInit::SetParent { perk: "None".into(), parent: "None".into() },
        add("Fitness", None, true),
        add("Strength", None, true),
        add("Sprinting", None, false),
        add("Crafting", None, false),
        add("Carving", Some("Crafting"), false),
        add("Axe", None, false),
        add("Juggle", None, false),
    ];
    let mut chars = Characters { perks: PerkFactory::from_tables(&t).unwrap(), ..Default::default() };
    chars.perks.init(&t, &|k| k.to_string()).unwrap();
    chars.sim = SimStatics::from_tables(&t).unwrap();
    (sm, chars)
}

/// Records events.
#[derive(Default)]
pub(crate) struct Recorder {
    pub events: RefCell<Vec<String>>,
    pub globals: ZomboidGlobals,
    pub debug: bool,
}

impl SimContext for Recorder {
    fn zomboid_globals(&self) -> &ZomboidGlobals {
        &self.globals
    }
    fn is_debug(&self) -> bool {
        self.debug
    }
    fn event(&self, e: SimEvent) {
        let s = match e {
            SimEvent::LevelPerk { perk, level, increased } => {
                format!("LevelPerk {} {level} {increased}", perk.borrow().id())
            }
            SimEvent::AddXp { perk, amount } => format!("AddXP {} {amount}", perk.borrow().id()),
            other => format!("{other:?}"),
        };
        self.events.borrow_mut().push(s);
    }
}

/// An invented item: registers the script `Base.<ty>` (changed by
/// `script`) and puts an item of it with id `id` in `c`'s inventory.
pub(crate) fn give_item<I>(
    sm: &ScriptManager,
    c: &mut GameCharacter<I>,
    class: ItemClass,
    ty: &str,
    id: i32,
    tpl: impl FnOnce(&mut ItemTemplate),
    script: impl FnOnce(&mut Item),
) {
    if sm.get_module("Base").is_none() {
        sm.parse_script(ScriptLoadMode::Init, "module Base { }").unwrap();
    }
    let mut it = Item::new();
    it.set_module(&sm.get_module("Base").unwrap());
    it.init_load_pp(sm, ty);
    script(&mut it);
    sm.add_runtime_script(ScriptRef::new(it)).unwrap();
    assert!(get_item(sm, &format!("Base.{ty}")).is_some());
    let mut t = ItemTemplate::new(class, "Base", ty);
    tpl(&mut t);
    let mut item = InventoryItem::create(Rc::new(t), 0);
    item.id = id;
    c.items.inventory.items.push(SavedItem::Item(Box::new(item)));
}

/// Registers `ItemTag.<field>` (or another item registry's constant) as
/// the invented id `t:<field>`.
pub(crate) fn item_constant(sm: &ScriptManager, registry: &str, field: &str) -> Rc<pz_scripts::registry::RegistryKey> {
    let id = format!("t:{}", field.to_lowercase());
    let k = sm.registries.get(registry).unwrap().borrow_mut().register_id(&id, false).unwrap();
    sm.statics.items.borrow_mut().constants.insert(registry, field, &id);
    k
}

/// An empty visual with `n` blood/dirt/hole entries.
pub(crate) fn visual(n: usize) -> pz_inventory::visual::ItemVisual {
    pz_inventory::visual::ItemVisual {
        full_type: String::new(),
        alternate_model_name: String::new(),
        clothing_item_name: String::new(),
        tint: None,
        base_texture: -1,
        texture_choice: -1,
        hue: f32::INFINITY,
        decal: None,
        arrays: [Some(vec![0; n]), Some(vec![0; n]), Some(vec![0; n]), None, None, None],
    }
}

pub(crate) fn player(sm: &ScriptManager, chars: &mut Characters) -> GameCharacter<()> {
    let mut c = GameCharacter::new(sm, CharacterKind::Player);
    c.in_players = true;
    c.player_instance = true;
    c.descriptor = Some(Rc::new(RefCell::new(SurvivorDesc::new(chars, sm).unwrap())));
    c
}

fn perk(chars: &Characters, f: &str) -> PerkRef {
    chars.perks.constant(f).unwrap()
}

#[test]
fn add_xp_boosts_levels_and_events() {
    let (sm, mut chars) = fixture();
    let mut c = player(&sm, &mut chars);
    let ctx = Recorder::default();
    let axe = perk(&chars, "Axe");
    // No boost: a quarter of the XP.
    c.add_xp(&sm, &chars, Some(&axe), 100.0, &ctx).unwrap();
    assert_eq!(c.xp.xp(&axe), 25.0);
    assert_eq!(c.perk_level(Some(&axe)), 1);
    // A level 2 boost: 1.33, crossing several levels at once.
    c.xp_set_perk_boost(&chars, Some(&axe), 2).unwrap();
    c.add_xp(&sm, &chars, Some(&axe), 100.0, &ctx).unwrap();
    assert_eq!(c.xp.xp(&axe), 25.0 + 100.0 * 1.33);
    assert_eq!(c.perk_level(Some(&axe)), 4);
    let ev = ctx.events.borrow().clone();
    assert_eq!(ev[0], "LevelPerk Axe 1 true");
    assert_eq!(ev[2], "AddXP Axe 25");
    assert!(ev.contains(&"LevelPerk Axe 3 true".to_string()));
    // Negative XP loses levels.
    c.add_xp_full(&sm, &chars, Some(&axe), -1000.0, true, false, false, false, &ctx).unwrap();
    assert_eq!((c.xp.xp(&axe), c.perk_level(Some(&axe))), (0.0, 0));
    // Capped at the total for level 10.
    c.add_xp_full(&sm, &chars, Some(&axe), 1.0e6, true, false, false, false, &ctx).unwrap();
    assert_eq!((c.xp.xp(&axe), c.perk_level(Some(&axe))), (axe.borrow().total_xp_for_level(10), 10));
    // Only local players get XP from the short overload; asleep blocks all.
    let mut npc: GameCharacter<()> = GameCharacter::new(&sm, CharacterKind::Survivor);
    npc.add_xp(&sm, &chars, Some(&axe), 10.0, &ctx).unwrap();
    assert!(npc.xp.xp_map.is_empty());
    c.asleep = true;
    c.add_xp_full(&sm, &chars, Some(&perk(&chars, "Juggle")), 10.0, true, false, false, false, &ctx).unwrap();
    assert_eq!(c.xp.xp(&perk(&chars, "Juggle")), 0.0);
    // A perk outside PerkList is the original's NPE.
    c.asleep = false;
    assert!(c.add_xp(&sm, &chars, Some(&perk(&chars, "None")), 1.0, &ctx).is_err());
}

#[test]
fn add_xp_traits_and_multipliers() {
    let (sm, mut chars) = fixture();
    let mut c = player(&sm, &mut chars);
    let ctx = Recorder::default();
    let carving = perk(&chars, "Carving");
    c.xp_set_perk_boost(&chars, Some(&carving), 3).unwrap();
    for t in ["FAST_LEARNER", "CRAFTY"] {
        c.character_traits.add(CharacterTrait::constant(&sm, t).as_ref()).unwrap();
    }
    c.xp.add_xp_multiplier(&carving, 2.0, 1, 2);
    c.add_xp(&sm, &chars, Some(&carving), 1.0, &ctx).unwrap();
    let expected = 1.0f32 * (1.66f32 * 1.3 * 1.3) * 2.0;
    assert_eq!(c.xp.xp(&carving), expected);
    // Reaching the multiplier's max level drops it.
    c.add_xp(&sm, &chars, Some(&carving), 100.0, &ctx).unwrap();
    assert!(c.xp.multiplier_entry(&carving).is_none());
    // AddXPNoMultiplier keeps the entry.
    let juggle = perk(&chars, "Juggle");
    c.xp.add_xp_multiplier(&juggle, 4.0, 1, 10);
    c.add_xp_no_multiplier(&sm, &chars, Some(&juggle), 8.0, &ctx).unwrap();
    assert_eq!(c.xp.xp(&juggle), 8.0 * 0.25 * 1.3);
    assert_eq!(c.xp.multiplier(&juggle), 4.0);
    // Fitness XP sets the Fitness stat from the level.
    let fitness = perk(&chars, "Fitness");
    c.add_xp(&sm, &chars, Some(&fitness), 30.0, &ctx).unwrap();
    assert_eq!(c.perk_level(Some(&fitness)), 1);
    assert_eq!(c.stat(&chars, "FITNESS").unwrap(), 1.0 / 5.0 - 1.0);
    // Strength XP with proteins between 50 and 300 is 1.5x.
    c.nutrition.as_mut().unwrap().set_proteins(100.0);
    let strength = perk(&chars, "Strength");
    c.add_xp(&sm, &chars, Some(&strength), 4.0, &ctx).unwrap();
    assert_eq!(c.xp.xp(&strength), 6.0);
    assert_eq!(c.perk_to_unit(&strength), 0.1 * (6.0 / 15.0));
}

#[test]
fn apply_traits_levels_perks() {
    let (sm, mut chars) = fixture();
    let mut c = player(&sm, &mut chars);
    let ctx = Recorder::default();
    let t = CharacterTrait::constant(&sm, "CRAFTY");
    let def = add_character_trait_definition(&sm, t.clone(), Some("x"), 1, Some("d"), false, false).unwrap();
    def.borrow_mut().xp_boosts = vec![("Carving".into(), 2), ("Strength".into(), -2)];
    c.descriptor.as_ref().unwrap().borrow_mut().xp_boost_map = vec![("Carving".into(), 3)];
    c.apply_traits(&sm, &chars, Some(&[t.clone(), None]), &ctx).unwrap();
    assert!(c.has_trait(&sm, "CRAFTY").unwrap());
    let level = |c: &GameCharacter<()>, f| c.perk_level(Some(&perk(&chars, f)));
    assert_eq!((level(&c, "Fitness"), level(&c, "Strength"), level(&c, "Carving")), (5, 3, 5));
    let carving = perk(&chars, "Carving");
    assert_eq!(c.xp.xp(&carving), carving.borrow().total_xp_for_level(5));
    let boosts = c.descriptor.as_ref().unwrap().borrow().xp_boost_map.clone();
    assert!(boosts.contains(&("Carving".into(), 3)) && boosts.contains(&("Fitness".into(), 3)));
    c.modify_trait_xp_boost(Some(&def), true).unwrap();
    let boosts = c.descriptor.as_ref().unwrap().borrow().xp_boost_map.clone();
    assert!(boosts.contains(&("Carving".into(), 1)) && boosts.contains(&("Strength".into(), 5)));
    assert_eq!(c.xp_for_level(&chars, 1).unwrap(), 50);
    assert_eq!(c.xp_for_level(&chars, 4).unwrap(), ((40 + 2 * 400) as f32 * 2.5) as i32);
    assert_eq!(c.level_up_levels(&chars, 7).unwrap(), 40);
}

#[test]
fn moodles_follow_stats() {
    let (sm, mut chars) = fixture();
    let mut c = player(&sm, &mut chars);
    let ctx = Recorder::default();
    let cs = chars.sim.character_stats.clone();
    c.stats.set(&cs.field("PANIC").unwrap(), 30.0);
    c.stats.set(&cs.field("ENDURANCE").unwrap(), 0.5);
    c.stats.set(&cs.field("STRESS").unwrap(), 0.5);
    c.stats.set(&cs.field("TEMPERATURE").unwrap(), 34.0);
    c.update_moodles(&sm, &chars, &ctx).unwrap();
    let lvl = |c: &GameCharacter<()>, f| c.moodles.moodle_level_of(&sm, f).unwrap();
    assert_eq!((lvl(&c, "PANIC"), lvl(&c, "ENDURANCE"), lvl(&c, "STRESS")), (2, 2, 2));
    assert_eq!(lvl(&c, "HYPOTHERMIA"), 2);
    assert!(c.moodles.ui_refresh_needed() && !c.moodles.ui_refresh_needed());
    // Pain only updates every 120 calls.
    c.stats.set(&cs.field("PAIN").unwrap(), 60.0);
    // The first update above already counted once.
    for _ in 0..118 {
        c.update_moodles(&sm, &chars, &ctx).unwrap();
    }
    assert_eq!(lvl(&c, "PAIN"), 0);
    c.update_moodles(&sm, &chars, &ctx).unwrap();
    assert_eq!(lvl(&c, "PAIN"), 3);
    assert!(ctx.events.borrow().iter().any(|e| e.contains("PainMoodle")));
    // Dead: everything resets except Dead.
    c.health = 0.0;
    c.update_moodles(&sm, &chars, &ctx).unwrap();
    assert_eq!((lvl(&c, "PANIC"), lvl(&c, "DEAD"), lvl(&c, "ZOMBIE")), (0, 4, 0));
    let t = MoodleType::constant(&sm, "PANIC").unwrap();
    let text = c.moodles.moodle_display_string(Some(&t), &|k| k.to_string()).unwrap();
    assert_eq!(text, "Invalid Moodle Level");
    assert_eq!(c.moodles.good_bad_neutral(&sm, MoodleType::constant(&sm, "FOOD_EATEN").as_deref()), 1);
}

#[test]
fn calculate_stats_tick() {
    let (sm, mut chars) = fixture();
    let mut c = player(&sm, &mut chars);
    let mut ctx = Recorder::default();
    ctx.globals.thirst_increase = 0.001;
    ctx.globals.fatigue_increase = 0.002;
    ctx.globals.hunger_increase = 0.003;
    ctx.globals.stress_reduction = 0.0005;
    ctx.globals.anger_decrease = 0.01;
    ctx.globals.idle_decrease_rate = 0.01;
    let cs = chars.sim.character_stats.clone();
    c.stats.set(&cs.field("ANGER").unwrap(), 0.5);
    c.stats.set(&cs.field("ENDURANCE").unwrap(), 0.5);
    c.calculate_stats(&sm, &chars, &ctx).unwrap();
    assert_eq!(c.stat(&chars, "THIRST").unwrap(), 0.001);
    assert_eq!(c.stat(&chars, "ANGER").unwrap(), 0.5 - 0.01);
    assert_eq!(c.stat(&chars, "FATIGUE").unwrap(), (0.002f64 * 0.5) as f32);
    assert_eq!(c.stat(&chars, "HUNGER").unwrap(), 0.003);
    assert_eq!(c.stat(&chars, "FITNESS").unwrap(), -1.0);
    assert_eq!(c.stats.last_endurance(), 0.5);
    // Asleep: the player sleeping update.
    c.asleep = true;
    c.stats.set(&cs.field("FATIGUE").unwrap(), 0.5);
    c.calculate_stats(&sm, &chars, &ctx).unwrap();
    let elapsed = 1.0f32 / 30.0 / 60.0 * 1.0 / 2.0;
    assert_eq!(c.stat(&chars, "FATIGUE").unwrap(), 0.5 - elapsed / 5.0 * 0.7);
    // Unlimited endurance needs cheats allowed (debug).
    ctx.debug = true;
    c.cheats.set(&ctx, "UNLIMITED_ENDURANCE", true);
    c.calculate_stats(&sm, &chars, &ctx).unwrap();
    assert_eq!(c.stat(&chars, "ENDURANCE").unwrap(), 1.0);
}
