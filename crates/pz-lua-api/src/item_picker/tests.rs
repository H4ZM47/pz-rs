use super::*;

fn t() -> TableRef {
    TableRef::new()
}

fn list(values: &[Value]) -> TableRef {
    let l = t();
    for (i, v) in values.iter().enumerate() {
        l.set_int(i as i64 + 1, v.clone());
    }
    l
}

/// `SuburbsDistributions`, `ProceduralDistributions` and
/// `NoContainerFillRooms` as the Lua files lay them out.
fn globals() -> TableRef {
    let g = t();
    let counter = t();
    counter.set_str("rolls", 2.7);
    counter.set_str("items", list(&[
        Value::str("Base.Rock"),
        Value::Num(10.0),
        Value::str("Base.Missing"),
        Value::Num(5.0),
        Value::str("Base.Apple"),
        Value::Num(0.5),
    ]));
    let junk = t();
    junk.set_str("rolls", 1.0);
    junk.set_str("items", list(&[Value::str("Base.Apple"), Value::Num(4.0)]));
    counter.set_str("junk", junk);
    let shelves = t();
    shelves.set_str("procedural", true);
    let entry = t();
    entry.set_str("name", "KitchenBooks");
    entry.set_str("min", 0.0);
    entry.set_str("max", 2.0);
    entry.set_str("weightChance", 40.0);
    entry.set_str("forceForTiles", "fixtures_01;fixtures_02");
    shelves.set_str("procList", list(&[Value::Table(entry)]));
    let broken = t();
    broken.set_str("items", list(&[]));
    let kitchen = t();
    kitchen.set_str("counter", counter);
    kitchen.set_str("shelves", shelves);
    kitchen.set_str("broken", broken);
    kitchen.set_str("isShop", true);
    kitchen.set_str("professionChance", 30.0);
    kitchen.set_str("outfit", "Chef");
    let bag = t();
    bag.set_str("rolls", 1.0);
    bag.set_str("items", list(&[Value::str("Base.Rock"), Value::Num(1.0)]));
    let sd = t();
    sd.set_str("kitchen", kitchen);
    sd.set_str("Bag_Schoolbag", bag);
    g.set_str("SuburbsDistributions", sd);
    let books = t();
    books.set_str("rolls", 4.0);
    books.set_str("items", list(&[Value::str("Base.Rock"), Value::Num(2.0)]));
    let pl = t();
    pl.set_str("KitchenBooks", books);
    let pd = t();
    pd.set_str("list", pl);
    g.set_str("ProceduralDistributions", pd);
    let ncf = t();
    ncf.set_str("closet", true);
    g.set_str("NoContainerFillRooms", ncf);
    g
}

fn valid(name: &str) -> bool {
    name != "Base.Missing"
}

#[test]
fn parse_reads_rooms_bags_procedural_lists_and_no_fill_rooms() {
    let d = parse(&globals(), &valid);
    assert_eq!(d.no_container_fill_rooms, vec!["closet".to_string()]);
    let kitchen = &d.rooms["kitchen"];
    assert!(kitchen.is_shop);
    // A number is `fillRand`, whatever its key.
    assert_eq!(kitchen.fill_rand, 30);
    assert_eq!(kitchen.strings["outfit"], "Chef");
    // A container without `rolls`/`items` (and not procedural) is skipped.
    assert!(!kitchen.containers.contains_key("broken"));
    let counter = &kitchen.containers["counter"];
    // `(float)(int) rolls`.
    assert_eq!(counter.rolls, 2.0);
    assert_eq!(
        counter.items,
        vec![
            PickerItem { item_name: "Base.Rock".into(), chance: 10.0 },
            PickerItem { item_name: "Base.Apple".into(), chance: 0.5 },
        ]
    );
    assert!(counter.default_inventory_loot);
    assert_eq!(counter.junk.as_ref().unwrap().items.len(), 1);
    let shelves = &kitchen.containers["shelves"];
    assert!(shelves.procedural);
    assert_eq!(
        shelves.procedural_items,
        vec![ProceduralItem {
            name: "KitchenBooks".into(),
            min: 0,
            max: 2,
            weight_chance: 40,
            force_for_tiles: Some(vec!["fixtures_01".into(), "fixtures_02".into()]),
            ..Default::default()
        }]
    );
    assert_eq!(d.containers["Bag_Schoolbag"].rolls, 1.0);
    assert!(!d.rooms.contains_key("Bag_Schoolbag"));
    assert_eq!(d.procedural["KitchenBooks"].rolls, 4.0);
    assert_eq!(d.invalid_reported, vec!["Base.Missing".to_string()]);
}

#[test]
fn java_string_hash_matches_string_hash_code() {
    assert_eq!(java_string_hash(""), 0);
    assert_eq!(java_string_hash("hello"), 99162322);
    assert_eq!(java_string_hash("Aa"), java_string_hash("BB"));
}

#[test]
fn java_hash_map_iterates_by_bucket_then_insertion() {
    let mut m = JavaHashMap::default();
    m.put("a", 1);
    m.put("BB", 2);
    m.put("Aa", 3);
    // "BB" and "Aa" share bucket 0 (hash 2112); "a" is in bucket 1.
    let keys: Vec<String> = m.iter_order().into_iter().map(|e| e.0).collect();
    assert_eq!(keys, ["BB", "Aa", "a"]);
    m.put("BB", 5);
    assert_eq!(m.get("BB"), Some(5));
    assert_eq!(m.len(), 3);
}

#[test]
fn java_hash_map_grows_past_three_quarters() {
    // 13 keys need 32 buckets: "q" (113) is in bucket 17 then, after "a"
    // (97, bucket 1) and before nothing in bucket 18+.
    let mut m = JavaHashMap::default();
    for c in "qabcdefghijkl".chars() {
        m.put(&c.to_string(), 1);
    }
    let keys: Vec<String> = m.iter_order().into_iter().map(|e| e.0).collect();
    assert_eq!(keys.last().map(String::as_str), Some("q"));
    assert_eq!(keys.first().map(String::as_str), Some("a"));
}

#[test]
fn distrib_weighted_pick_is_cumulative_in_key_order() {
    let mut m = JavaHashMap::default();
    m.put("a", 10);
    m.put("b", 30);
    let mut asked = Vec::new();
    let mut r = |max| {
        asked.push(max);
        10
    };
    assert_eq!(distrib_in_hash_map(&m, &mut r).as_deref(), Some("a"));
    let mut r = |_| 11;
    assert_eq!(distrib_in_hash_map(&m, &mut r).as_deref(), Some("b"));
    assert_eq!(asked, vec![40]);
}

#[test]
fn distrib_forced_entry_returns_the_first_key() {
    let mut m = JavaHashMap::default();
    m.put("b", -1);
    let mut r = |_| 0;
    assert_eq!(distrib_in_hash_map(&m, &mut r).as_deref(), Some("b"));
}

#[test]
fn loot_zombie_intensity_follows_the_meta_chunk() {
    let mut no = |_| 299;
    assert!((loot_zombie_intensity(255, false, &mut no) - 12.0).abs() < 1e-4);
    let mut yes = |_| 0;
    assert_eq!(loot_zombie_intensity(255, false, &mut yes), 120.0);
    assert_eq!(loot_zombie_intensity(0, true, &mut no), 400.0);
}

#[test]
fn loot_settings_map_types_and_generator_factors() {
    let mut l = LootSettings::default();
    l.modifiers.insert("Other", 0.6);
    l.modifiers.insert("Food", 2.0);
    l.loot_factors = [0.1, 0.2, 0.3, 0.4, 0.5, 0.6];
    assert_eq!(l.modifier_from_type("Food"), 2.0);
    assert_eq!(l.modifier_from_type("Unknown"), 0.6);
    l.generator_spawning = 1;
    assert_eq!(l.modifier_from_type("Generator"), 0.0);
    l.generator_spawning = 3;
    assert_eq!(l.modifier_from_type("Generator"), 0.2);
    l.generator_spawning = 9;
    assert_eq!(l.modifier_from_type("Generator"), 0.6);
}

#[test]
fn no_generic_loot_takes_plain_and_base_ids() {
    assert!(no_generic_loot("fridge"));
    assert!(no_generic_loot("base:stove"));
    assert!(no_generic_loot("SurvivorCrate"));
    assert!(!no_generic_loot("survivorcrate"));
    assert!(!no_generic_loot("counter"));
}
