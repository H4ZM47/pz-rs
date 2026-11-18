//! `zombie.inventory.ItemPickerJava`'s distribution tables and the parts of
//! its rolls that need no world: `Parse()` of Lua's
//! `SuburbsDistributions` / `ProceduralDistributions` /
//! `NoContainerFillRooms`, `InitSandboxLootSettings()`, the loot type and
//! loot modifier of an item script, the spawn chance and the weighted
//! choice of a procedural distribution (`getDistribInHashMap`).
//!
//! [`crate::bind_item_picker`] runs the rolls against the world and Lua.

use pz_kahlua::{TableRef, Value};
use pz_scripts::objects::items::Item;
use pz_scripts::ScriptManager;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

/// `ItemPickerJava$ItemPickerItem`.
#[derive(Clone, Debug, PartialEq)]
pub struct PickerItem {
    pub item_name: String,
    pub chance: f32,
}

/// `ItemPickerJava$ProceduralItem`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ProceduralItem {
    pub name: String,
    pub min: i32,
    pub max: i32,
    pub weight_chance: i32,
    pub force_for_items: Option<Vec<String>>,
    pub force_for_zones: Option<Vec<String>>,
    pub force_for_tiles: Option<Vec<String>>,
    pub force_for_rooms: Option<Vec<String>>,
}

/// `ItemPickerJava$ItemPickerContainer`.
#[derive(Clone, Debug, PartialEq)]
pub struct PickerContainer {
    pub items: Vec<PickerItem>,
    pub rolls: f32,
    pub no_auto_age: bool,
    pub is_shop: bool,
    pub fill_rand: i32,
    pub max_map: i32,
    pub stash_chance: i32,
    pub junk: Option<Box<PickerContainer>>,
    pub bags: Option<Box<PickerContainer>>,
    pub procedural: bool,
    pub dont_spawn_ammo: bool,
    pub gun_storage: bool,
    pub ignore_zombie_density: bool,
    pub cook_food: bool,
    pub can_burn: bool,
    pub is_trash: bool,
    pub is_worn: bool,
    pub is_rotten: bool,
    pub only_one: bool,
    pub default_inventory_loot: bool,
    pub procedural_items: Vec<ProceduralItem>,
}

impl Default for PickerContainer {
    /// The constructor: no items, `defaultInventoryLoot = true`.
    fn default() -> Self {
        Self {
            items: Vec::new(),
            rolls: 0.0,
            no_auto_age: false,
            is_shop: false,
            fill_rand: 0,
            max_map: 0,
            stash_chance: 0,
            junk: None,
            bags: None,
            procedural: false,
            dont_spawn_ammo: false,
            gun_storage: false,
            ignore_zombie_density: false,
            cook_food: false,
            can_burn: false,
            is_trash: false,
            is_worn: false,
            is_rotten: false,
            only_one: false,
            default_inventory_loot: true,
            procedural_items: Vec::new(),
        }
    }
}

/// `ItemPickerJava$ItemPickerRoom` (the string fields only feed the
/// randomized-world stories, which do not read them from here yet).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PickerRoom {
    pub containers: HashMap<String, PickerContainer>,
    pub fill_rand: i32,
    pub is_shop: bool,
    pub strings: HashMap<String, String>,
}

/// The tables `ItemPickerJava.Parse()` fills.
#[derive(Clone, Debug, Default)]
pub struct Distributions {
    /// `rooms`.
    pub rooms: HashMap<String, PickerRoom>,
    /// `containers`: `SuburbsDistributions` entries with `rolls` (bags).
    pub containers: HashMap<String, PickerContainer>,
    /// `ProceduralDistributions`.
    pub procedural: HashMap<String, PickerContainer>,
    /// `NoContainerFillRooms`.
    pub no_container_fill_rooms: Vec<String>,
    /// `addedInvalidAlready`: invalid item names reported once.
    pub invalid_reported: Vec<String>,
}

thread_local! {
    static SHARED: RefCell<Rc<Distributions>> = RefCell::new(Rc::new(Distributions::default()));
    static LOOT: RefCell<LootSettings> = RefCell::new(LootSettings::default());
}

/// The parsed tables (empty until [`install`]).
pub fn shared() -> Rc<Distributions> {
    SHARED.with(|s| s.borrow().clone())
}

/// Replace the parsed tables.
pub fn install(d: Distributions) {
    SHARED.with(|s| *s.borrow_mut() = Rc::new(d));
}

/// The loot settings of the last `InitSandboxLootSettings()`.
pub fn loot_settings() -> LootSettings {
    LOOT.with(|l| l.borrow().clone())
}

pub fn install_loot_settings(l: LootSettings) {
    LOOT.with(|s| *s.borrow_mut() = l);
}

/// The `ContainerType`s in `NO_GENERIC_LOOT_CONTAINERS`
/// (`initNoGenericLootContainers`): an `other` distribution never fills
/// them.
pub const NO_GENERIC_LOOT_CONTAINERS: [&str; 41] = [
    "barbecue",
    "barbecuepropane",
    "bin",
    "brazier",
    "campfire",
    "cashregister",
    "clothingdryer",
    "clothingdryerbasic",
    "clothingrack",
    "clothingwasher",
    "coffeemaker",
    "coffin",
    "composter",
    "dishwasher",
    "doghouse",
    "dumpster",
    "fireplace",
    "freezer",
    "fridge",
    "icecream",
    "logs",
    "Mannequin",
    "medicine",
    "microwave",
    "newspaper_dispatch",
    "newspaper_herald",
    "newspaper_knews",
    "newspaper_times",
    "plankstash",
    "postbox",
    "shelter",
    "stonefurnace",
    "stove",
    "SurvivorCrate",
    "tent",
    "toaster",
    "trough",
    "vendingGt",
    "vendingpop",
    "vendingsnack",
    "woodstove",
];

/// True when the container type, without the `base:` namespace, is in
/// `NO_GENERIC_LOOT_CONTAINERS`.
pub fn no_generic_loot(container_type: &str) -> bool {
    let id = container_type.strip_prefix("base:").unwrap_or(container_type);
    NO_GENERIC_LOOT_CONTAINERS.contains(&id)
}

fn table(v: &Value) -> Option<&TableRef> {
    match v {
        Value::Table(t) => Some(t),
        _ => None,
    }
}

fn key_string(v: &Value) -> String {
    match v {
        Value::Str(s) => s.to_string(),
        Value::Num(n) => pz_kahlua::exec::java_value_of(&Value::Num(*n)),
        other => pz_kahlua::exec::java_value_of(other),
    }
}

/// `KahluaTableImpl.rawgetBool`: `false` unless a boolean `true`.
fn raw_bool(t: &TableRef, k: &str) -> bool {
    matches!(t.rawget(&Value::str(k)), Value::Bool(true))
}

/// `KahluaTableImpl.rawgetInt`: a number's `intValue()`, `-1` otherwise.
fn raw_int(t: &TableRef, k: &str) -> i32 {
    match t.rawget(&Value::str(k)) {
        Value::Num(n) => n as i32,
        _ => -1,
    }
}

/// `KahluaTableImpl.rawgetStr`: a string, `None` otherwise.
fn raw_str(t: &TableRef, k: &str) -> Option<String> {
    match t.rawget(&Value::str(k)) {
        Value::Str(s) => Some(s.to_string()),
        _ => None,
    }
}

fn has(t: &TableRef, k: &str) -> bool {
    t.contains_own(&Value::str(k))
}

/// `ItemPickerJava.ExtractProcList(table)`.
fn extract_proc_list(t: &TableRef) -> Vec<ProceduralItem> {
    let Some(list) = table(&t.rawget(&Value::str("procList"))).cloned() else {
        return Vec::new();
    };
    let split = |s: Option<String>| -> Option<Vec<String>> {
        s.filter(|s| !s.trim().is_empty())
            .map(|s| s.split(';').map(str::to_string).collect())
    };
    let mut out = Vec::new();
    for i in 1..=list.len() {
        let Some(e) = table(&list.get_int(i)).cloned() else {
            continue;
        };
        out.push(ProceduralItem {
            name: raw_str(&e, "name").unwrap_or_default(),
            min: raw_int(&e, "min"),
            max: raw_int(&e, "max"),
            weight_chance: raw_int(&e, "weightChance"),
            force_for_items: split(raw_str(&e, "forceForItems")),
            force_for_zones: split(raw_str(&e, "forceForZones")),
            force_for_tiles: split(raw_str(&e, "forceForTiles")),
            force_for_rooms: split(raw_str(&e, "forceForRooms")),
        });
    }
    out
}

/// `ItemPickerJava.ExtractContainersFromLua(table)`. `valid` answers
/// whether an item name has a script (`FindItem`, or
/// `InventoryItemFactory.getItem(name, true)`) that is not obsolete.
pub fn extract_container(
    t: &TableRef,
    valid: &dyn Fn(&str) -> bool,
    invalid: &mut Vec<String>,
) -> PickerContainer {
    let mut c = PickerContainer::default();
    let flag = |k: &str, f: &mut bool| {
        if has(t, k) {
            *f = raw_bool(t, k);
        }
    };
    flag("isShop", &mut c.is_shop);
    if has(t, "procedural") {
        c.procedural = raw_bool(t, "procedural");
        c.procedural_items = extract_proc_list(t);
        return c;
    }
    flag("noAutoAge", &mut c.no_auto_age);
    if has(t, "fillRand") {
        c.fill_rand = raw_int(t, "fillRand");
    }
    if has(t, "maxMap") {
        c.max_map = raw_int(t, "maxMap");
    }
    if has(t, "stashChance") {
        c.stash_chance = raw_int(t, "stashChance");
    }
    flag("dontSpawnAmmo", &mut c.dont_spawn_ammo);
    flag("gunStorage", &mut c.gun_storage);
    flag("ignoreZombieDensity", &mut c.ignore_zombie_density);
    flag("cookFood", &mut c.cook_food);
    flag("canBurn", &mut c.can_burn);
    flag("isTrash", &mut c.is_trash);
    flag("isWorn", &mut c.is_worn);
    flag("isRotten", &mut c.is_rotten);
    flag("onlyOne", &mut c.only_one);
    // `((Double) rawget("rolls")).doubleValue()`, then `(float)(int)`.
    let rolls = match t.rawget(&Value::str("rolls")) {
        Value::Num(n) => n,
        _ => 0.0,
    };
    if let Some(j) = table(&t.rawget(&Value::str("junk"))) {
        c.junk = Some(Box::new(extract_container(j, valid, invalid)));
    }
    if let Some(b) = table(&t.rawget(&Value::str("bags"))) {
        c.bags = Some(Box::new(extract_container(b, valid, invalid)));
    }
    flag("defaultInventoryLoot", &mut c.default_inventory_loot);
    c.rolls = rolls as i32 as f32;
    if let Some(items) = table(&t.rawget(&Value::str("items"))).cloned() {
        let n = items.len();
        let mut i = 0;
        while i < n {
            let name = match items.get_int(i + 1) {
                Value::Str(s) => Some(s.to_string()),
                _ => None,
            };
            let chance = match items.get_int(i + 2) {
                Value::Num(n) => Some(n as f32),
                _ => None,
            };
            i += 2;
            let (Some(name), Some(chance)) = (name, chance) else {
                continue;
            };
            if !valid(&name) {
                if !invalid.contains(&name) {
                    invalid.push(name);
                }
                continue;
            }
            c.items.push(PickerItem {
                item_name: name,
                chance,
            });
        }
    }
    c
}

/// The `ItemPickerRoom` string fields `ParseSuburbsDistributions` copies.
const ROOM_STRINGS: [&str; 15] = [
    "outfit",
    "outfitFemale",
    "outfitMale",
    "outfitChance",
    "vehicle",
    "vehicleChance",
    "vehicleDistribution",
    "femaleChance",
    "roomTypes",
    "zoneRequires",
    "zoneDisallows",
    "containerChance",
    "femaleOdds",
    "bagType",
    "bagTable",
];

/// `ItemPickerJava.Parse()` over Lua's globals (the weapon upgrades and
/// the vehicle distributions are not read).
pub fn parse(globals: &TableRef, valid: &dyn Fn(&str) -> bool) -> Distributions {
    let mut d = Distributions::default();
    if let Some(t) = table(&globals.rawget(&Value::str("NoContainerFillRooms"))) {
        for (k, _) in t.entries() {
            d.no_container_fill_rooms.push(key_string(&k));
        }
    }
    let mut invalid = Vec::new();
    if let Some(sd) = table(&globals.rawget(&Value::str("SuburbsDistributions"))).cloned() {
        for (k, v) in sd.entries() {
            let name = key_string(&k);
            let Some(v) = table(&v).cloned() else { continue };
            if has(&v, "rolls") {
                let c = extract_container(&v, valid, &mut invalid);
                d.containers.insert(name, c);
                continue;
            }
            let mut room = PickerRoom::default();
            for (k2, v2) in v.entries() {
                let key = key_string(&k2);
                // A number value is `fillRand` whatever its key (so
                // `professionChance` never reaches its own field).
                if let Value::Num(n) = v2 {
                    room.fill_rand = n as i32;
                    continue;
                }
                if key == "isShop" {
                    room.is_shop = matches!(v2, Value::Bool(true));
                    continue;
                }
                if key == "vehicles" || ROOM_STRINGS.contains(&key.as_str()) {
                    if let Value::Str(s) = &v2 {
                        room.strings.insert(key, s.to_string());
                    }
                    continue;
                }
                let Some(ct) = table(&v2).cloned() else {
                    continue;
                };
                if !has(&ct, "procedural")
                    && (key.is_empty() || !has(&ct, "rolls") || !has(&ct, "items"))
                {
                    pz_core::debug::DebugType::ItemPicker.error(
                        None,
                        &format!("ERROR: SuburbsDistributions[\"{name}\"] is broken"),
                    );
                    continue;
                }
                let c = extract_container(&ct, valid, &mut invalid);
                room.containers.insert(key, c);
            }
            d.rooms.insert(name, room);
        }
    }
    if let Some(pd) = table(&globals.rawget(&Value::str("ProceduralDistributions"))) {
        if let Some(list) = table(&pd.rawget(&Value::str("list"))) {
            for (k, v) in list.entries() {
                if let Some(v) = table(&v) {
                    let c = extract_container(v, valid, &mut invalid);
                    d.procedural.insert(key_string(&k), c);
                }
            }
        }
    }
    d.invalid_reported = invalid;
    d
}

/// `ItemPickerJava`'s static loot modifiers, set by
/// `InitSandboxLootSettings()` from the sandbox options, plus the other
/// options the rolls read.
#[derive(Clone, Debug, PartialEq)]
pub struct LootSettings {
    pub modifiers: HashMap<&'static str, f32>,
    /// `generatorSpawning` (an enum index).
    pub generator_spawning: i32,
    /// The loot factors `doSandboxSettings` maps `generatorSpawning` to:
    /// insane, extreme, rare, normal, common, abundant.
    pub loot_factors: [f32; 6],
    pub rolls_multiplier: f64,
    pub zombie_pop_loot_effect: i32,
    /// `zombies` (an enum index; 6 is "none").
    pub zombies: i32,
    pub population_multiplier: f64,
    /// `getCurrentLootMultiplier()`.
    pub loot_multiplier: f32,
    /// `LootItemRemovalList`.
    pub removal_list: Vec<String>,
}

impl Default for LootSettings {
    /// Before `InitSandboxLootSettings()`: every static modifier is 0
    /// (Java's default), so nothing spawns.
    fn default() -> Self {
        Self {
            modifiers: HashMap::new(),
            generator_spawning: 0,
            loot_factors: [0.0; 6],
            rolls_multiplier: 1.0,
            zombie_pop_loot_effect: 0,
            zombies: 0,
            population_multiplier: 1.0,
            loot_multiplier: 1.0,
            removal_list: Vec::new(),
        }
    }
}

/// The loot types `getLootModifierFromType` maps to a modifier, with the
/// sandbox option each modifier comes from.
pub const LOOT_TYPES: [(&str, &str); 21] = [
    ("Other", "OtherLootNew"),
    ("Food", "FoodLootNew"),
    ("CannedFood", "CannedFoodLootNew"),
    ("Weapon", "WeaponLootNew"),
    ("RangedWeapon", "RangedWeaponLootNew"),
    ("Ammo", "AmmoLootNew"),
    ("Literature", "LiteratureLootNew"),
    ("SurvivalGears", "SurvivalGearsLootNew"),
    ("Medical", "MedicalLootNew"),
    ("Mechanics", "MechanicsLootNew"),
    ("Clothing", "ClothingLootNew"),
    ("Container", "ContainerLootNew"),
    ("Key", "KeyLootNew"),
    ("Media", "MediaLootNew"),
    ("Memento", "MementoLootNew"),
    ("Cookware", "CookwareLootNew"),
    ("Material", "MaterialLootNew"),
    ("Farming", "FarmingLootNew"),
    ("Tool", "ToolLootNew"),
    ("SkillBook", "SkillBookLoot"),
    ("RecipeResource", "RecipeResourceLoot"),
];

impl LootSettings {
    /// `InitSandboxLootSettings()` and the other options the rolls read,
    /// from `SandboxOptions.instance`.
    pub fn from_sandbox(sb: &pz_sandbox::SandboxOptions, world_age_days: f64) -> Self {
        let d = |n: &str| sb.option_by_name(n).map_or(0.0, |o| o.config.get_double());
        let i = |n: &str| sb.option_by_name(n).map_or(0, |o| o.config.get_int());
        let mut modifiers = HashMap::new();
        for (ty, opt) in LOOT_TYPES {
            modifiers.insert(ty, d(opt) as f32);
        }
        Self {
            modifiers,
            generator_spawning: i("GeneratorSpawning"),
            loot_factors: [
                d("InsaneLootFactor") as f32,
                d("ExtremeLootFactor") as f32,
                d("RareLootFactor") as f32,
                d("NormalLootFactor") as f32,
                d("CommonLootFactor") as f32,
                d("AbundantLootFactor") as f32,
            ],
            rolls_multiplier: d("RollsMultiplier"),
            zombie_pop_loot_effect: i("ZombiePopLootEffect"),
            zombies: i("Zombies"),
            population_multiplier: d("ZombieConfig.PopulationMultiplier"),
            loot_multiplier: sb.current_loot_multiplier(world_age_days),
            removal_list: sb
                .option_by_name("LootItemRemovalList")
                .map(|o| o.split_csv_list())
                .unwrap_or_default(),
        }
    }

    fn modifier(&self, ty: &str) -> f32 {
        self.modifiers.get(ty).copied().unwrap_or(0.0)
    }

    /// `doSandboxSettings(value)`.
    fn do_sandbox_settings(&self, v: i32) -> f32 {
        match v {
            1 => 0.0,
            2..=7 => self.loot_factors[(v - 2) as usize],
            _ => 0.6,
        }
    }

    /// `getLootModifierFromType(lootType)`.
    pub fn modifier_from_type(&self, loot_type: &str) -> f32 {
        if loot_type == "Generator" {
            return self.do_sandbox_settings(self.generator_spawning);
        }
        match LOOT_TYPES.iter().find(|(t, _)| *t == loot_type) {
            Some((t, _)) => self.modifier(t),
            None => self.modifier("Other"),
        }
    }

    /// `lootItemRemovalListContains(name)`.
    pub fn removal_listed(&self, name: &str) -> bool {
        self.removal_list.iter().any(|s| s == name)
    }
}

/// `ScriptManager.FindItem(name)`.
pub fn find_item(sm: &ScriptManager, name: &str) -> Option<Rc<RefCell<Item>>> {
    pz_scripts::objects::items::find_item(sm, name)
}

/// `FindItem(name)`, and for an unknown `<type>Empty` the `<type>` script
/// (the fluid-container convention of the distributions).
pub fn find_item_or_empty(sm: &ScriptManager, name: &str) -> (String, Option<Rc<RefCell<Item>>>) {
    if let Some(i) = find_item(sm, name) {
        return (name.to_string(), Some(i));
    }
    if let Some(base) = name.strip_suffix("Empty") {
        return (base.to_string(), find_item(sm, base));
    }
    (name.to_string(), None)
}

/// `ItemPickerJava.getLootType(item)`.
pub fn loot_type(sm: &ScriptManager, item: &Item) -> &'static str {
    if item.name() == "Generator"
        || item.full_name() == "Base.Generator"
        || item.has_tag_constant(sm, "GENERATOR")
    {
        return "Generator";
    }
    if item.is_memento_loot(sm) {
        return "Memento";
    }
    if item.is_medical_loot() {
        return "Medical";
    }
    if item.is_mechanics_loot() {
        return "Mechanics";
    }
    if item.is_material_loot() {
        return "Material";
    }
    if item.is_farming_loot(sm) {
        return "Farming";
    }
    if item.is_tool_loot() {
        return "Tool";
    }
    if item.is_cookware_loot() {
        return "Cookware";
    }
    if item.is_survival_gear_loot() {
        return "SurvivalGears";
    }
    let cat = item.display_category.as_deref();
    let food = item.is_item_type(sm, "FOOD");
    if food || cat == Some("Food") {
        if item.canned_food || item.days_fresh == 1_000_000_000 || !food {
            return "CannedFood";
        }
        return "Food";
    }
    if cat == Some("Ammo")
        || item.has_tag_constant(sm, "AMMO_CASE")
        || (item.is_item_type(sm, "NORMAL") && item.ammo_type.is_some())
    {
        return "Ammo";
    }
    let weapon = item.is_item_type(sm, "WEAPON");
    if weapon && !item.ranged {
        return "Weapon";
    }
    if item.is_item_type(sm, "WEAPON_PART")
        || (weapon && item.ranged)
        || item.has_tag_constant(sm, "FIREARM_LOOT")
    {
        return "RangedWeapon";
    }
    if item.is_item_type(sm, "KEY")
        || item.is_item_type(sm, "KEY_RING")
        || item.has_tag_constant(sm, "KEY_RING")
    {
        return "Key";
    }
    if item.capacity > 0 || item.is_item_type(sm, "CONTAINER") || cat == Some("Bag") {
        return "Container";
    }
    if cat == Some("SkillBook") {
        return "SkillBook";
    }
    if cat == Some("RecipeResource") {
        return "RecipeResource";
    }
    if item.is_item_type(sm, "LITERATURE") {
        return "Literature";
    }
    if item.is_item_type(sm, "CLOTHING") {
        return "Clothing";
    }
    if item.recorded_media_cat.is_some() {
        return "Media";
    }
    "Other"
}

/// `ItemPickerJava.getLootModifier(name)`.
pub fn loot_modifier(sm: &ScriptManager, loot: &LootSettings, name: &str) -> f32 {
    let (_, item) = find_item_or_empty(sm, name);
    let Some(item) = item else {
        return loot.modifier("Other");
    };
    // `lootItemRemovalListContains(name)` or of `getItemName(name)`.
    let short = name.rsplit('.').next().unwrap_or(name);
    if loot.removal_listed(name) || loot.removal_listed(short) {
        return 0.0;
    }
    let ty = loot_type(sm, &item.borrow());
    loot.modifier_from_type(ty)
}

/// `getLootModifier(name, isJunk)`: junk of a positive modifier is 1.
pub fn loot_modifier_junk(sm: &ScriptManager, loot: &LootSettings, name: &str, is_junk: bool) -> f32 {
    let m = loot_modifier(sm, loot, name);
    if is_junk && m > 0.0 {
        1.0
    } else {
        m
    }
}

/// `getBaseChanceMultiplier(chr, isJunk, item)`.
pub fn base_chance_multiplier(
    sm: &ScriptManager,
    loot: &LootSettings,
    is_junk: bool,
    item: Option<&Item>,
) -> f32 {
    let mut m = 1.0f32;
    if is_junk {
        m *= 1.4;
    }
    if let Some(item) = item {
        if item.has_tag_constant(sm, "MORE_WHEN_NO_ZOMBIES")
            && (loot.zombies == 6 || loot.population_multiplier == 0.0)
        {
            m *= 2.0;
        }
    }
    m
}

/// `getAdjustedZombieDensity(density, item, isJunk)`.
pub fn adjusted_zombie_density(sm: &ScriptManager, density: f32, item: Option<&Item>, is_junk: bool) -> f32 {
    if is_junk || item.is_some_and(|i| i.ignore_zombie_density(sm)) {
        0.0
    } else {
        density
    }
}

/// `getActualSpawnChance(item, chr, container, density, isJunk)`.
pub fn actual_spawn_chance(
    sm: &ScriptManager,
    loot: &LootSettings,
    item: &PickerItem,
    density: f32,
    is_junk: bool,
) -> f32 {
    let script = find_item(sm, &item.item_name);
    let script = script.as_ref().map(|s| s.borrow());
    let modifier = loot_modifier_junk(sm, loot, &item.item_name, is_junk);
    if modifier == 0.0 {
        return 0.0;
    }
    let base = item.chance * base_chance_multiplier(sm, loot, is_junk, script.as_deref());
    let density = adjusted_zombie_density(sm, density, script.as_deref(), is_junk);
    (base * 100.0 * modifier + density) * loot.loot_multiplier
}

/// `IsoMetaChunk.getLootZombieIntensity()` for a chunk of zombie
/// intensity `intensity`; draws one `Rand.Next(300)`.
pub fn loot_zombie_intensity(intensity: u8, zombies_disabled: bool, rand: &mut dyn FnMut(i32) -> i32) -> f32 {
    let f = intensity as f32 / 255.0;
    let mut v = 0.06f32 + 11.94f32 * f;
    if rand(300) as f32 <= f * 10.0 {
        v = 120.0;
    }
    if zombies_disabled {
        return 400.0;
    }
    v
}

/// `String.hashCode()`.
pub fn java_string_hash(s: &str) -> i32 {
    s.encode_utf16()
        .fold(0i32, |h, c| h.wrapping_mul(31).wrapping_add(c as i32))
}

/// A `java.util.HashMap<String, Integer>` built by `put` calls: the
/// iteration order of its keys is the bucket order of the final table.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct JavaHashMap {
    entries: Vec<(String, i32)>,
}

impl JavaHashMap {
    pub fn put(&mut self, k: &str, v: i32) {
        match self.entries.iter_mut().find(|(e, _)| e == k) {
            Some(e) => e.1 = v,
            None => self.entries.push((k.to_string(), v)),
        }
    }
    pub fn get(&self, k: &str) -> Option<i32> {
        self.entries.iter().find(|(e, _)| e == k).map(|e| e.1)
    }
    pub fn clear(&mut self) {
        self.entries.clear();
    }
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
    pub fn len(&self) -> usize {
        self.entries.len()
    }
    /// The entries in `keySet()` order: by bucket (`(h ^ h >>> 16) &
    /// (capacity - 1)`, capacity 16 doubled while over 0.75 full), then
    /// in insertion order within a bucket (resizes keep it).
    pub fn iter_order(&self) -> Vec<(String, i32)> {
        let mut cap = 16usize;
        while self.entries.len() > cap * 3 / 4 {
            cap *= 2;
        }
        let mut v: Vec<(usize, usize)> = self
            .entries
            .iter()
            .enumerate()
            .map(|(i, (k, _))| {
                let h = java_string_hash(k) as u32;
                (((h ^ (h >> 16)) as usize) & (cap - 1), i)
            })
            .collect();
        v.sort();
        v.into_iter().map(|(_, i)| self.entries[i].clone()).collect()
    }
}

/// `ItemPickerJava.getDistribInHashMap(map)`: a weighted choice; a total
/// of -1 (one forced entry) picks uniformly.
pub fn distrib_in_hash_map(map: &JavaHashMap, rand: &mut dyn FnMut(i32) -> i32) -> Option<String> {
    let order = map.iter_order();
    let total: i32 = order.iter().map(|(_, v)| *v).fold(0i32, |a, b| a.wrapping_add(b));
    if total == -1 {
        // The original draws an index but never advances its key
        // iterator while counting to it, so it returns the first key.
        rand(map.len() as i32);
        return order.into_iter().next().map(|(k, _)| k);
    }
    let pick = rand(total);
    let mut sum = 0i32;
    for (k, v) in order {
        sum = sum.wrapping_add(v);
        if sum >= pick {
            return Some(k);
        }
    }
    None
}

#[cfg(test)]
#[path = "item_picker/tests.rs"]
mod tests;
