//! Tests for the `ItemCodeOnCreate` naming and literature callbacks.
//!
//! Expected values replay the original's `Rand` draws in the original's
//! order; the translator is not installed, so a `getText` returns its key.

use std::rc::Rc;

use pz_inventory::{shared, InventoryItem, ItemClass, ItemKind, ItemTemplate};
use pz_kahlua::{stdlib, TableRef, Thread, Value};

use super::on_create_callbacks::TEST_YEAR;
use super::on_create_data::{install_for_test, Entry, Tables};
use super::ItemRef;

fn thread() -> Thread {
    let mut thread = Thread::new(TableRef::new());
    stdlib::install(&mut thread, None).unwrap();
    crate::bind_java_util::register(&mut thread);
    crate::bind_globals::register(&mut thread);
    crate::bind_items::register(&mut thread);
    thread
}

fn run(thread: &mut Thread, src: &str) -> Result<Vec<Value>, String> {
    let f = stdlib::load_string(src, Some("test"), thread.globals().clone()).unwrap();
    thread
        .pcall(&f, &[])
        .map_err(|(m, _)| pz_kahlua::exec::java_value_of(&m))
}

fn item(thread: &mut Thread, class: ItemClass, name: &str, tags: &[&str]) -> ItemRef {
    let mut template = ItemTemplate::new(class, "Base", name);
    template.tags = tags.iter().map(|t| (*t).to_string()).collect();
    let item = shared::try_new_item(InventoryItem::create(Rc::new(template), 1)).unwrap();
    thread
        .globals()
        .set_str("item", crate::bind_items::try_item_value(&item).unwrap());
    item
}

fn mod_str(item: &ItemRef, key: &str) -> Option<String> {
    item.borrow().mod_data_string(key).unwrap()
}

fn name_of(item: &ItemRef) -> String {
    item.borrow().name.clone()
}

fn people() {
    let characters = crate::bind_characters::shared_characters();
    let mut characters = characters.borrow_mut();
    characters.survivor_factory.female_forenames = vec!["Ann".into(), "Beth".into()];
    characters.survivor_factory.male_forenames = vec!["Carl".into(), "Dan".into()];
    characters.survivor_factory.surnames = vec!["Poe".into(), "Roe".into(), "Doe".into()];
}

/// The shared generator's draws, replayed.
fn person(female: bool) -> String {
    let characters = crate::bind_characters::shared_characters();
    let characters = characters.borrow();
    let forenames = if female {
        &characters.survivor_factory.female_forenames
    } else {
        &characters.survivor_factory.male_forenames
    };
    let forename = forenames[pz_core::random::next(forenames.len() as i32) as usize].clone();
    let surname = &characters.survivor_factory.surnames;
    let surname = surname[pz_core::random::next(surname.len() as i32) as usize].clone();
    format!("{forename} {surname}")
}

fn entry(name: &str, keys: &[(&str, String)]) -> Entry {
    Entry {
        name: name.into(),
        keys: keys
            .iter()
            .map(|(f, v)| ((*f).to_string(), v.clone()))
            .collect(),
        ..Entry::default()
    }
}

#[test]
fn photos_draw_a_photo_then_an_id() {
    let mut thread = thread();
    let tables = Tables::default()
        .with_entries(
            "Photo",
            vec![
                entry("Dock", &[("translationKey", "IGUI_Photo_Dock".into())]),
                entry("Barn", &[("translationKey", "IGUI_Photo_Barn".into())]),
            ],
        )
        .with_list("PHOTOS", "Photo", &["Dock", "Barn"]);
    install_for_test(tables);
    let photo = item(&mut thread, ItemClass::Normal, "Photo", &[]);
    pz_core::random::set_seed(3);
    let which = pz_core::random::next(2) as usize;
    let number = pz_core::random::next(1_000_000);
    let key = ["IGUI_Photo_Dock", "IGUI_Photo_Barn"][which];
    pz_core::random::set_seed(3);
    run(&mut thread, "ItemCodeOnCreate.onCreatePhoto(item)").unwrap();
    assert_eq!(name_of(&photo), "IGUI_PhotoOf");
    assert_eq!(
        mod_str(&photo, "literatureTitle"),
        Some(format!("OldPhoto_{key}_{number}"))
    );
    assert_eq!(mod_str(&photo, "collectibleKey").as_deref(), Some(key));
}

#[test]
fn comic_books_number_their_issue_and_retail_skips_out_of_print() {
    let mut thread = thread();
    let mut serial = entry("Serial", &[("translationKey", "IGUI_Comic_Serial".into())]);
    serial.number = 120;
    serial.in_print = true;
    let mut one_shot = entry(
        "OneShot",
        &[("translationKey", "IGUI_Comic_OneShot".into())],
    );
    one_shot.in_print = false;
    let mut old = entry("Old", &[("translationKey", "IGUI_Comic_Old".into())]);
    old.number = 7;
    install_for_test(Tables::default().with_entries("ComicBook", vec![serial, one_shot, old]));

    let comic = item(&mut thread, ItemClass::Normal, "ComicBook", &[]);
    pz_core::random::set_seed(2);
    let draw = pz_core::random::next(3);
    let expected = match draw {
        0 => {
            let issue = pz_core::random::next(120);
            Some(format!("IGUI_Comic_Serial#{issue}"))
        }
        1 => None,
        _ => {
            let issue = pz_core::random::next(7);
            Some(format!("IGUI_Comic_Old#{issue}"))
        }
    };
    pz_core::random::set_seed(2);
    run(&mut thread, "ItemCodeOnCreate.onCreateComicBook(item)").unwrap();
    match expected {
        Some(title) => assert_eq!(mod_str(&comic, "literatureTitle"), Some(title)),
        None => assert_eq!(
            mod_str(&comic, "literatureTitle").as_deref(),
            Some("IGUI_Comic_OneShot")
        ),
    }

    // Retail redraws until the issue is in print.
    pz_core::random::set_seed(9);
    let mut draws = 1;
    while pz_core::random::next(3) == 1 {
        draws += 1;
    }
    assert!(draws >= 1);
    pz_core::random::set_seed(9);
    let retail = item(&mut thread, ItemClass::Normal, "ComicBook", &[]);
    run(
        &mut thread,
        "ItemCodeOnCreate.onCreateComicBookRetail(item)",
    )
    .unwrap();
    assert!(mod_str(&retail, "literatureTitle")
        .unwrap()
        .starts_with("IGUI_Comic_"));
    assert_ne!(
        mod_str(&retail, "literatureTitle").as_deref(),
        Some("IGUI_Comic_OneShot")
    );
}

#[test]
fn schematics_learn_one_to_five_recipes() {
    let mut thread = thread();
    let ids: Vec<String> = (0..8).map(|i| format!("Recipe{i}")).collect();
    install_for_test(Tables::default().with_recipes(
        "MELEE_WEAPON_SCHEMATICS",
        &ids.iter().map(String::as_str).collect::<Vec<_>>(),
    ));
    let schematic = item(&mut thread, ItemClass::Literature, "Schematic", &[]);
    pz_core::random::set_seed(21);
    let mut count = 1;
    if pz_core::random::next(100) < 30 {
        count = 5.min(pz_core::random::next_range(2, 9));
    }
    let expected: Vec<String> = (0..count)
        .map(|_| ids[pz_core::random::next(8) as usize].clone())
        .collect();
    pz_core::random::set_seed(21);
    run(
        &mut thread,
        "ItemCodeOnCreate.onCreateMeleeWeaponSchematic(item)",
    )
    .unwrap();
    let ItemKind::Literature(data) = &schematic.borrow().kind else {
        unreachable!()
    };
    assert_eq!(data.learned_recipes.as_ref().unwrap(), &expected);
    assert!(
        schematic.borrow().table.is_some(),
        "getLearnedRecipes creates the mod data"
    );
    if expected.len() == 1 {
        assert_eq!(
            mod_str(&schematic, "collectibleKey").as_deref(),
            Some("IGUI_MagazineNameNoIssue")
        );
    }
}

#[test]
fn subject_books_match_cover_and_subject() {
    let mut thread = thread();
    let book = |name: &str, cover: &str, subjects: &[&str]| Entry {
        name: name.into(),
        keys: vec![("translationKey".into(), format!("IGUI_BookTitle_{name}"))],
        cover: cover.into(),
        subjects: subjects.iter().map(|s| (*s).to_string()).collect(),
        ..Entry::default()
    };
    install_for_test(
        Tables::default()
            .with_entries(
                "Book",
                vec![
                    book("HardArt", "HARDCOVER", &["art"]),
                    book("SoftArt", "SOFTCOVER", &["art"]),
                    book("EitherSport", "BOTH", &["sports", "art"]),
                    book("SoftSport", "SOFTCOVER", &["sports"]),
                ],
            )
            .with_key("ItemTag", "HARDCOVER", "base:hardcover")
            .with_key("ItemTag", "HOLLOW_BOOK", "base:hollowbook")
            .with_key("ItemTag", "FANCY_BOOK", "base:fancybook")
            .with_key("ItemTag", "SOFTCOVER", "base:softcover"),
    );
    // Item scripts carry their subjects.
    let mut template = ItemTemplate::new(ItemClass::Literature, "Base", "BookArt");
    template.tags = vec!["base:hardcover".into()];
    let script = Rc::new(std::cell::RefCell::new(
        pz_scripts::objects::items::Item::new(),
    ));
    let key = |path: &str| {
        Some(Rc::new(pz_scripts::registry::RegistryKey {
            location: pz_scripts::registry::ResourceLocation::of(&format!("base:{path}")).unwrap(),
            name: path.into(),
            item_key: None,
        }))
    };
    template.script = Some(pz_inventory::template::ScriptItemRef(script.clone()));
    let item = shared::try_new_item(InventoryItem::create(Rc::new(template), 1)).unwrap();
    thread
        .globals()
        .set_str("item", crate::bind_items::try_item_value(&item).unwrap());
    // No subjects on the script: nothing matches, the draw faults.
    let error = run(&mut thread, "ItemCodeOnCreate.onCreateSubjectBook(item)").unwrap_err();
    assert!(
        error.contains("IndexOutOfBounds") || error.contains("out of bounds"),
        "{error}"
    );

    // A hard-cover art book: HardArt (HARDCOVER) and EitherSport (BOTH, art).
    script.borrow_mut().book_subjects = vec![key("art")];
    pz_core::random::set_seed(6);
    let which = ["HardArt", "EitherSport"][pz_core::random::next(2) as usize];
    pz_core::random::set_seed(6);
    run(&mut thread, "ItemCodeOnCreate.onCreateSubjectBook(item)").unwrap();
    assert_eq!(name_of(&item), "IGUI_MagazineNameNoIssue");
    assert_eq!(
        mod_str(&item, "literatureTitle"),
        Some(format!("IGUI_BookTitle_{which}"))
    );

    // No cover tag at all means both covers: every art book qualifies.
    let plain = item_with_script(&mut thread, &script, &[]);
    pz_core::random::set_seed(6);
    let which = ["HardArt", "SoftArt", "EitherSport"][pz_core::random::next(3) as usize];
    pz_core::random::set_seed(6);
    run(&mut thread, "ItemCodeOnCreate.onCreateSubjectBook(item)").unwrap();
    assert_eq!(
        mod_str(&plain, "literatureTitle"),
        Some(format!("IGUI_BookTitle_{which}"))
    );
}

fn item_with_script(
    thread: &mut Thread,
    script: &Rc<std::cell::RefCell<pz_scripts::objects::items::Item>>,
    tags: &[&str],
) -> ItemRef {
    let mut template = ItemTemplate::new(ItemClass::Literature, "Base", "Book");
    template.tags = tags.iter().map(|t| (*t).to_string()).collect();
    template.script = Some(pz_inventory::template::ScriptItemRef(script.clone()));
    let item = shared::try_new_item(InventoryItem::create(Rc::new(template), 1)).unwrap();
    thread
        .globals()
        .set_str("item", crate::bind_items::try_item_value(&item).unwrap());
    item
}

#[test]
fn newspapers_pick_an_issue_and_the_latest_for_new_ones() {
    let mut thread = thread();
    let mut paper = entry(
        "KentuckyHerald",
        &[(
            "translationKey",
            "IGUI_NewspaperTitle_KentuckyHerald".into(),
        )],
    );
    paper.issues = vec!["KH_1".into(), "KH_2".into(), "KH_3".into()];
    install_for_test(
        Tables::default()
            .with_entries("Newspaper", vec![paper])
            .with_key("ItemTag", "NEWSPAPER_NEW", "base:newspapernew"),
    );
    let old = item(&mut thread, ItemClass::Normal, "Newspaper", &[]);
    pz_core::random::set_seed(8);
    let drawn = ["KH_1", "KH_2", "KH_3"][pz_core::random::next(3) as usize];
    pz_core::random::set_seed(8);
    run(
        &mut thread,
        "ItemCodeOnCreate.onCreateRecentNewspaper(item)",
    )
    .unwrap();
    assert_eq!(name_of(&old), format!("Print_Media_{drawn}_title"));
    assert_eq!(
        mod_str(&old, "literatureTitle"),
        Some(format!("Print_Media_{drawn}_title"))
    );
    let media = run(
        &mut thread,
        "local m = item:getModData().printMedia return m.id, m.info, m.text, m.title",
    )
    .unwrap();
    assert_eq!(media[0].as_str(), Some("kentuckyherald"));
    assert_eq!(
        media[1].as_str(),
        Some(format!("Print_Media_{drawn}_info").as_str())
    );
    assert_eq!(
        media[2].as_str(),
        Some(format!("Print_Text_{drawn}_info").as_str())
    );
    assert_eq!(
        media[3].as_str(),
        Some(format!("Print_Media_{drawn}_title").as_str())
    );

    let fresh = item(
        &mut thread,
        ItemClass::Normal,
        "Newspaper",
        &["base:newspapernew"],
    );
    pz_core::random::set_seed(8);
    run(
        &mut thread,
        "ItemCodeOnCreate.onCreateRecentNewspaper(item)",
    )
    .unwrap();
    assert_eq!(name_of(&fresh), "Print_Media_KH_3_title");
}

#[test]
fn magazines_date_from_the_sandbox_year() {
    let mut thread = thread();
    install_for_test(Tables::default().with_key("ItemTag", "NEW", "base:new"));
    TEST_YEAR.with(|y| y.set(Some(1993)));
    let current = item(&mut thread, ItemClass::Normal, "TVMagazine", &["base:new"]);
    pz_core::random::set_seed(4);
    let month = pz_core::random::next_range(1, 8);
    pz_core::random::set_seed(4);
    run(&mut thread, "ItemCodeOnCreate.onCreateTVMagazine(item)").unwrap();
    assert_eq!(name_of(&current), "IGUI_MagazineName");
    assert_eq!(
        mod_str(&current, "literatureTitle"),
        Some(format!("TVMagazine_{month}_1993"))
    );

    // An older magazine moves to a year before the start year and rerolls the month.
    let old = item(&mut thread, ItemClass::Normal, "TVMagazine", &[]);
    pz_core::random::set_seed(4);
    let _first_month = pz_core::random::next_range(1, 8);
    let year = 1953 + pz_core::random::next(1993 - 1953);
    let month = if year == 1993 {
        _first_month
    } else {
        pz_core::random::next_range(1, 13)
    };
    pz_core::random::set_seed(4);
    run(&mut thread, "ItemCodeOnCreate.onCreateTVMagazine(item)").unwrap();
    assert_eq!(
        mod_str(&old, "literatureTitle"),
        Some(format!("TVMagazine_{month}_{year}"))
    );
    TEST_YEAR.with(|y| y.set(None));
}

#[test]
fn gas_masks_get_a_filter_with_a_random_fill() {
    let mut thread = thread();
    install_for_test(
        Tables::default()
            .with_key("ItemTag", "GAS_MASK", "base:gas_mask")
            .with_key("ItemTag", "RESPIRATOR", "base:respirator")
            .with_key("ItemTag", "IMPROVISED_GAS_MASK", "base:improvised_gas_mask")
            .with_key("ItemTag", "SCBA", "base:scba")
            .with_key("ItemKey$Drainable", "GASMASK_FILTER", "Base.GasmaskFilter")
            .with_key(
                "ItemKey$Drainable",
                "RESPIRATOR_FILTERS",
                "Base.RespiratorFilters",
            )
            .with_key("ItemKey$Drainable", "RAG_FILTER", "Base.RagFilter")
            .with_key("ItemKey$Drainable", "OXYGEN_TANK", "Base.OxygenTank"),
    );
    for (tag, key, id) in [
        ("base:gas_mask", "filterType", "Base.GasmaskFilter"),
        ("base:respirator", "filterType", "Base.RespiratorFilters"),
        ("base:improvised_gas_mask", "filterType", "Base.RagFilter"),
        ("base:scba", "tankType", "Base.OxygenTank"),
    ] {
        let mask = item(&mut thread, ItemClass::Clothing, "Mask", &[tag]);
        pz_core::random::set_seed(9);
        let fill = pz_core::random::next(1000) as f32 / 1000.0;
        pz_core::random::set_seed(9);
        run(&mut thread, "ItemCodeOnCreate.onCreateGasMask(item)").unwrap();
        assert_eq!(mod_str(&mask, key), Some(id.to_string()));
        let stored = run(&mut thread, "return item:getModData().usedDelta").unwrap();
        assert_eq!(stored[0].as_num(), Some(f64::from(fill)));
    }
    // No mask tag: nothing happens and nothing is drawn.
    let other = item(&mut thread, ItemClass::Clothing, "Hat", &[]);
    run(&mut thread, "ItemCodeOnCreate.onCreateGasMask(item)").unwrap();
    assert_eq!(mod_str(&other, "filterType"), None);
}

#[test]
fn scratch_tickets_draw_a_weighted_winning() {
    let mut thread = thread();
    install_for_test(Tables::default().with_key(
        "ModelKey",
        "SCRATCH_TICKET_WINNER",
        "ScratchTicket_Winner",
    ));
    let ticket = item(&mut thread, ItemClass::Literature, "ScratchTicket", &[]);
    // One draw over the weights' total (1194) and nothing else.
    pz_core::random::set_seed(3);
    let _ = pz_core::random::next(1194);
    let after = pz_core::random::next(1000);
    pz_core::random::set_seed(3);
    let result = run(&mut thread, "ItemCodeOnCreate.scratchTicketWinner(item)");
    assert_eq!(pz_core::random::next(1000), after);
    assert_eq!(name_of(&ticket), "IGUI_ScratchingTicketNameWinner");
    // Unit tests carry no texture provider: the callback stops at setTexture,
    // after the name, as the original orders them.
    assert!(result.unwrap_err().contains("texture provider"));
}

#[test]
fn fluid_callbacks_fault_without_a_container() {
    let mut thread = thread();
    install_for_test(Tables::default().with_key("FluidKey", "SODA_POP", "SodaPop"));
    item(&mut thread, ItemClass::Normal, "PopBottle", &[]);
    for name in [
        "onCreatePopBottle",
        "onCreateSodaCan",
        "onCreateHairDyeBottle",
    ] {
        let error = run(&mut thread, &format!("ItemCodeOnCreate.{name}(item)")).unwrap_err();
        assert!(error.contains("NullPointerException"), "{name}: {error}");
    }
}

#[test]
fn argument_conversion_checks_the_declared_class() {
    let mut thread = thread();
    install_for_test(Tables::default());
    let _ = item(&mut thread, ItemClass::Normal, "Plain", &[]);
    let error = run(&mut thread, "ItemCodeOnCreate.onCreateMonogram(item)").unwrap_err();
    assert_eq!(
        error,
        "expected argument of type Clothing, got ComboItem java.lang.RuntimeException"
    );
    let error = run(&mut thread, "ItemCodeOnCreate.onCreatePaperwork(item)").unwrap_err();
    assert_eq!(
        error,
        "expected argument of type Literature, got ComboItem java.lang.RuntimeException"
    );
    let error = run(&mut thread, "ItemCodeOnCreate.onCreatePaperwork()").unwrap_err();
    assert_eq!(
        error,
        "expected 1 argument, got 0 java.lang.RuntimeException"
    );
    let error = run(&mut thread, "ItemCodeOnCreate.onCreateDogTagPet(item, 1)").unwrap_err();
    assert_eq!(
        error,
        "expected 1 argument, got 2 java.lang.RuntimeException"
    );
    let error = run(&mut thread, "ItemCodeOnCreate.onCreateDogTagPet(nil)").unwrap_err();
    assert!(error.contains("pz.host.Unsupported"), "{error}");
}

/// With an install, the tables load from the jar.
#[test]
fn installed_jar_tables_load() {
    let Ok(dir) = std::env::var("PZ_GAME_DIR") else {
        eprintln!("PZ_GAME_DIR not set; skipping");
        return;
    };
    let jar =
        pz_classfile::Jar::open(std::path::Path::new(&dir).join("projectzomboid.jar")).unwrap();
    // Every public static method of the original is bound.
    let cf = pz_classfile::load_class(&jar, "zombie/scripting/logic/ItemCodeOnCreate").unwrap();
    let mut thread = thread();
    let unbound: Vec<&str> = cf
        .methods
        .iter()
        .filter(|m| m.access & 0x0001 != 0 && m.access & 0x0008 != 0 && m.name != "<init>")
        .map(|m| m.name.as_str())
        .filter(|name| {
            let result = run(
                &mut thread,
                &format!("return ItemCodeOnCreate.{name} ~= nil"),
            )
            .unwrap();
            !matches!(result[0], Value::Bool(true))
        })
        .collect();
    assert!(unbound.is_empty(), "unbound callbacks: {unbound:?}");
    super::on_create_data::register_from_jar(Some(&jar));
    let tables = super::on_create_data::tables().unwrap();
    for class in [
        "Brochure",
        "Flier",
        "Newspaper",
        "Catalogue",
        "Book",
        "Magazine",
        "ComicBook",
        "Photo",
        "Job",
    ] {
        assert!(!tables.values(class).unwrap().is_empty(), "{class}");
    }
    let airport = &tables.values("Brochure").unwrap()[0];
    assert_eq!(airport.name, "Airport");
    assert_eq!(airport.key("translationKey"), "Print_Media_Airport_title");
    assert_eq!(airport.key("translationTextKey"), "Print_Text_Airport_info");
    assert!(tables
        .registry("Flier")
        .unwrap()
        .field("NOLANS_USED_CARS")
        .is_some());
    assert_eq!(tables.key("ItemTag", "NEW").unwrap(), "base:new");
    assert_eq!(tables.key("ModelKey", "PAPERWORK_1").unwrap(), "Paperwork1");
    for list in [
        "STOCK_CERTIFICATE_1",
        "PET_NAMES",
        "PHOTOS",
        "OLD_NEWSPAPER",
    ] {
        assert!(!tables.list(list).unwrap().is_empty(), "{list}");
    }
    for list in ["GENERIC_MAIL", "LETTER_HANDWRITTEN", "POSTCARD"] {
        tables.weighted(list).unwrap();
    }
    for list in ["FOOD_RECIPES", "SEWING_PATTERNS", "EXPLOSIVE_SCHEMATICS"] {
        assert!(!tables.recipes(list).unwrap().is_empty(), "{list}");
    }
    let sports = tables
        .values("Book")
        .unwrap()
        .iter()
        .filter(|b| b.subjects.iter().any(|s| s == "sports"))
        .count();
    assert!(sports > 0);
}
