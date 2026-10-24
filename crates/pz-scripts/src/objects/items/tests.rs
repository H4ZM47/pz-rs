use std::rc::Rc;

use super::*;
use crate::base::ScriptObject;
use crate::manager::ScriptManager;
use crate::script_type::ScriptLoadMode;

/// A manager with module `Base` registered from `body` (script stubs
/// created, nothing loaded yet).
fn manager(body: &str) -> (ScriptManager, Rc<ScriptModule>) {
    let sm = ScriptManager::new();
    sm.builtin_tables.borrow_mut().perk_ids = test_perks();
    sm.parse_script(ScriptLoadMode::Init, &format!("module Base {{ {body} }}"))
        .unwrap();
    let m = sm.get_module("Base").unwrap();
    (sm, m)
}

fn load_item(
    sm: &ScriptManager,
    m: &Rc<ScriptModule>,
    name: &str,
    body: &str,
) -> (Item, ScriptResult) {
    let mut it = Item::new();
    it.set_module(m);
    it.init_load_pp(sm, name);
    let r = it.load(sm, name, body);
    (it, r)
}

/// Loads `files` (relative path, text) as a game install with a full
/// `ScriptManager.Load`.
fn load_game(files: &[(&str, &str)]) -> ScriptManager {
    let dir = std::env::temp_dir().join(format!(
        "pz-items-test-{}-{}",
        std::process::id(),
        files.iter().map(|f| f.1.len()).sum::<usize>()
    ));
    for (path, text) in files {
        let p = dir.join(path);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, text).unwrap();
    }
    let sm = ScriptManager::new();
    sm.builtin_tables.borrow_mut().perk_ids = test_perks();
    sm.load(&pz_fs::GameFs::new(dir.clone()), &[]).unwrap();
    std::fs::remove_dir_all(dir).unwrap();
    sm
}

fn register(sm: &ScriptManager, registry: &str, id: &str) -> Rc<crate::registry::RegistryKey> {
    sm.registries
        .get(registry)
        .unwrap()
        .borrow_mut()
        .register_id(id, true)
        .unwrap()
}

#[test]
fn params_and_defaults() {
    let (sm, m) = manager("");
    let body = "item Gadget { Weight = -2, DisplayName = Shiny = Thing, HitSound = null, ImpactSound = null, \
        KeepOnDeplete = TRUE, Capacity = 70, ReplaceTypes = WaterSource Bottle; Broken ; Fuel Can, \
        replaceInPrimaryHand = Glove   RightMask, Palettes = A / B, AcceptMediaType = 3, \
        RainLevel = 1.5, Flavour = sweet, }";
    let (it, r) = load_item(&sm, &m, "Gadget", body);
    r.unwrap();
    assert_eq!(it.full_name(), "Base.Gadget");
    assert_eq!(it.actual_weight, 0.0);
    // Only the text before a second '=' is the value.
    assert_eq!(it.display_name.as_deref(), Some("Shiny"));
    assert_eq!(
        (it.hit_sound.as_deref(), it.impact_sound.as_deref()),
        (None, None)
    );
    assert!(it.is_keep_on_deplete());
    assert_eq!(it.capacity, 50);
    assert_eq!(it.replace_type("WaterSource"), Some("Bottle"));
    assert_eq!(it.replace_type("Fuel"), Some("Can"));
    assert!(!it.has_replace_type("Broken"));
    let rep = it.replace_primary_hand.as_ref().unwrap();
    assert_eq!(rep.clothing_item_name.as_deref(), Some("Glove"));
    assert_eq!(rep.mask_variable_name.as_deref(), Some("RightHandMask"));
    assert_eq!(it.palette_choices, ["A", "B"]);
    assert_eq!(it.accept_media_type, 3);
    assert_eq!(
        it.default_mod_data.as_deref().unwrap(),
        [
            ("RainLevel".to_string(), ModDataValue::Number(1.5)),
            (
                "Flavour".to_string(),
                ModDataValue::String("sweet".to_string())
            )
        ]
    );
    assert_eq!(
        (it.icon.as_deref(), it.days_fresh, it.use_delta),
        (Some("None"), Item::MAXIMUM_FOOD_AGE, 0.03125)
    );
}

#[test]
fn load_errors_are_invalid_parameters() {
    let (sm, m) = manager("");
    let (_, r) = load_item(&sm, &m, "Gadget", "item Gadget { Weight = heavy, }");
    assert_eq!(
        r.unwrap_err().0,
        "java.security.InvalidParameterException: Error: Weight is not a valid parameter in item: Gadget"
    );
    let (_, r) = load_item(&sm, &m, "Gadget", "item Gadget { Weight, }");
    assert_eq!(
        r.unwrap_err().0,
        "java.security.InvalidParameterException: Index 1 out of bounds for length 1"
    );
    let (_, r) = load_item(&sm, &m, "Gadget", "item Gadget { AcceptMediaType = 300, }");
    assert!(r
        .unwrap_err()
        .0
        .ends_with("AcceptMediaType is not a valid parameter in item: Gadget"));
}

#[test]
fn registry_keys_and_attributes() {
    let (sm, m) = manager("");
    let tag = register(&sm, "item_tag", "Shiny");
    register(&sm, "item_type", "Gizmo");
    let body =
        "item Gadget { Tags = shiny;Missing;SHINY, ItemType = base:gizmo, HeadCondition = 7, }";
    let (it, r) = load_item(&sm, &m, "Gadget", body);
    r.unwrap();
    assert_eq!(it.item_tags.len(), 2);
    assert!(it.has_tag(Some(&tag)));
    assert!(it.has_tag(None));
    assert_eq!(it.item_type.as_ref().unwrap().location.id(), "base:gizmo");
    let attrs = it
        .entity
        .component::<crate::objects::entity::components::attributes::AttributesScript>(
            crate::objects::entity::ComponentType::Attributes,
        )
        .unwrap();
    assert_eq!(
        attrs.kv_pairs().get("HeadCondition").map(String::as_str),
        Some("7")
    );
    assert_eq!(
        attrs
            .kv_pairs()
            .get("TimesHeadRepaired")
            .map(String::as_str),
        Some("0")
    );
}

#[test]
fn model_weapon_parts_share_updated_entries() {
    let (sm, m) = manager("");
    let body = "item Gun { modelWeaponPart = Mod.Scope ScopeA none parent, modelWeaponPart = Mod.Scope ScopeB, \
        modelWeaponPart = Grip, }";
    let (it, r) = load_item(&sm, &m, "Gun", body);
    r.unwrap();
    let parts = it.model_weapon_part.as_ref().unwrap();
    assert_eq!(parts.len(), 2);
    assert!(Rc::ptr_eq(&parts[0], &parts[1]));
    let p = parts[0].borrow();
    assert_eq!(
        (p.part_type.as_str(), p.model_name.as_str()),
        ("Mod.Scope", "Base.ScopeB")
    );
    assert_eq!(p.attachment_name_self, None);
}

#[test]
fn evolved_recipe_param() {
    let (sm, m) = manager("");
    let (it, r) = load_item(
        &sm,
        &m,
        "Leek",
        "item Leek { EvolvedRecipe = Soup:4;PastaPot:2|Cooked;Stew, }",
    );
    r.unwrap();
    assert_eq!(it.evolved_recipe, ["Soup:4", "PastaPot:2|Cooked", "Stew"]);
    let pasta = it.item_recipe_map.get("Pasta").unwrap();
    assert_eq!(
        (pasta.use_, pasta.cooked, pasta.full_type()),
        (2, true, "Base.Leek".to_string())
    );
    assert_eq!(it.item_recipe_map.get("Stew").unwrap().use_, 0);
    let (_, r) = load_item(&sm, &m, "Leek", "item Leek { EvolvedRecipe = Soup:, }");
    assert!(r.is_err());
}

#[test]
fn full_load_links_recipes_and_builds_maps() {
    let script = "module Base {
        evolvedrecipe Broth { BaseItem = Pot, MaxItems = 4, ResultItem = Broth, Name = Broth, }
        item Leek { EvolvedRecipe = Broth:3, ClothingItem = LeekHat, }
        item Pot { ReplaceTypes = Water Leek, magazineType = Leek, }
        item Sock { DisplayName = Sock, }
    }";
    let sm = load_game(&[("media/scripts/test_items.txt", script)]);
    let broth = crate::objects::recipes::get_evolved_recipe(&sm, "Broth").unwrap();
    assert_eq!(broth.borrow().items_list.get("Leek").unwrap().use_, 3);
    let pot = get_item(&sm, "Pot").unwrap();
    assert_eq!(pot.borrow().replace_type("Water"), Some("Base.Leek"));
    assert_eq!(pot.borrow().magazine_type.as_deref(), Some("Base.Leek"));
    assert_eq!(pot.borrow().display_name.as_deref(), Some("Base.Pot"));
    assert_eq!(
        get_item(&sm, "Sock")
            .unwrap()
            .borrow()
            .display_name
            .as_deref(),
        Some("Sock")
    );
    assert_eq!(
        get_item_type_for_clothing_item(&sm, "LeekHat").as_deref(),
        Some("Base.Leek")
    );
    assert_eq!(
        get_item_for_clothing_item(&sm, "LeekHat")
            .unwrap()
            .borrow()
            .name,
        "Leek"
    );
    let ids: Vec<i32> = ["Base.Leek", "Base.Pot", "Base.Sock"]
        .iter()
        .map(|t| net_item_to_id(&sm, t).unwrap())
        .collect();
    assert_eq!(ids, [0, 1, 2]);
    assert_eq!(net_id_to_item(&sm, 1).as_deref(), Some("Base.Pot"));
}

#[test]
fn missing_references_fail_scripts_loaded() {
    let (sm, m) = manager("");
    let (mut it, r) = load_item(&sm, &m, "Leek", "item Leek { EvolvedRecipe = Nothing:1, }");
    r.unwrap();
    let err = it.on_scripts_loaded(&sm, ScriptLoadMode::Init).unwrap_err();
    assert_eq!(
        err.0,
        "java.security.InvalidParameterException: Could not find evolved recipe or template: 'Nothing' in item: Base.Leek"
    );
    let (mut it, _) = load_item(&sm, &m, "Box", "item Box { ItemConfig = Nope, }");
    let err = it.on_scripts_loaded(&sm, ScriptLoadMode::Init).unwrap_err();
    assert_eq!(
        err.0,
        "java.lang.Exception: Cannot set item config 'Nope' to item: Base.Box"
    );
}

#[test]
fn trait_and_profession_definitions() {
    let sm = ScriptManager::new();
    sm.builtin_tables.borrow_mut().perk_ids = test_perks();
    register(&sm, "character_trait", "Brave");
    let timid = register(&sm, "character_trait", "Timid");
    let bold = register(&sm, "character_trait", "Bold");
    register(&sm, "character_profession", "Baker");
    let load = |s: &mut dyn ScriptObject, body: &str| s.load(&sm, "x", body);
    let body = "t x { CharacterTrait = base:timid, UIName = UI_Timid, Cost = -2, XPBoosts = Cooking=2;Unknown=1;Fitness=x, }";
    load(&mut CharacterTraitDefinitionScript::new(), body).unwrap();
    let body = "t x { CharacterTrait = brave, Cost = 4, MutuallyExclusiveTraits = timid, GrantedRecipes = A;B, }";
    load(&mut CharacterTraitDefinitionScript::new(), body).unwrap();
    load(
        &mut CharacterTraitDefinitionScript::new(),
        "t x { CharacterTrait = bold, GrantedTraits = brave, }",
    )
    .unwrap();
    let defs = character_trait_definitions(&sm);
    assert_eq!(defs.len(), 3);
    assert_eq!(
        defs[0].xp_boosts,
        [("Cooking".to_string(), 2), ("Fitness".to_string(), 1)]
    );
    assert_eq!(defs[0].right_label(), "+2");
    assert_eq!(defs[1].granted_recipes, ["A", "B"]);
    // Bold grants Brave, which excludes Timid: Bold and Timid become exclusive.
    let has = |d: &CharacterTraitDefinition, k: &Rc<crate::registry::RegistryKey>| {
        d.mutually_exclusive_traits
            .iter()
            .any(|t| t.as_ref() == Some(k))
    };
    assert!(has(&defs[2], &timid) && has(&defs[0], &bold));
    assert!(!has(&defs[1], &bold));

    let err = load(
        &mut CharacterTraitDefinitionScript::new(),
        "t x { UIName = Nobody, }",
    )
    .unwrap_err();
    assert_eq!(err.0, "java.lang.NullPointerException");

    let body = "p x { CharacterProfession = base:baker, Cost = 3, XPBoosts = Cooking=3=4, GrantedTraits = bold, Oops = 1, }";
    load(&mut CharacterProfessionDefinitionScript::new(), body).unwrap();
    let profs = character_profession_definitions(&sm);
    assert_eq!(profs.len(), 1);
    assert_eq!(profs[0].xp_boosts, [("Cooking".to_string(), 1)]);
    assert_eq!(profs[0].right_label(), "-3");
    assert_eq!(profs[0].description, "");
}

#[test]
fn item_filter_lists() {
    let sm = ScriptManager::new();
    sm.builtin_tables.borrow_mut().perk_ids = test_perks();
    register(&sm, "item_type", "Food");
    let mut f = ItemFilterScript::new();
    let body = "itemFilter F { items = Base.A / Base.B / Base.A, types = food, \
        tags { shiny, x = y, } blacklist { items = Base.C, blacklist { tags = dull, } } }";
    f.load(&sm, "F", body).unwrap();
    assert_eq!(f.whitelist.loaded_items, ["Base.A", "Base.B"]);
    assert_eq!(f.whitelist.tags, ["shiny"]);
    assert_eq!(f.blacklist.loaded_items, ["Base.C"]);
    assert_eq!(f.blacklist.tags, ["dull"]);
    f.on_loaded_after_lua(&sm).unwrap();
    assert_eq!(f.whitelist.items, ["Base.A", "Base.B"]);
    assert_eq!(f.whitelist.item_types.len(), 1);
    let mut items = vec!["[fn]".to_string(), "Base.X".to_string()];
    resolve_get_item_types(&mut items).unwrap();
    assert_eq!(items, ["Base.X"]);
    assert!(resolve_get_item_types(&mut vec!["[fn".to_string()]).is_err());
}

#[test]
fn physics_hit_reactions() {
    let (sm, m) = manager("");
    let mut s = PhysicsHitReactionScript::new();
    s.set_module(&m);
    s.load(
        &sm,
        "Rock",
        "physicsHitReaction Rock { impulse_head = 5, upwardImpulse_head = 2.5, other = 1, }",
    )
    .unwrap();
    {
        let st = sm.statics.items.borrow();
        let r = &st.physics_hit_reaction_list[0];
        assert_eq!(r.physics_object.as_deref(), Some("???.Rock"));
        assert_eq!(
            (r.impulse[0], r.upward_impulse[0], r.impulse[1]),
            (5.0, 2.5, 0.0)
        );
    }
    assert_eq!(impulse_for_object(&sm, 0, Some("???.Rock")), 5.0);
    // A reaction without ammo type matches a null ammo type.
    assert_eq!(upward_impulse_for_ammo(&sm, 0, None), 2.5);
    let ammo = register(&sm, "ammo_type", "pebble");
    assert_eq!(upward_impulse_for_ammo(&sm, 0, Some(&ammo)), 40.0);
    let many: String = (0..=BODYPART_COUNT)
        .map(|i| format!("impulse_{i} = 1, "))
        .collect();
    let err = s
        .load(
            &sm,
            "Rock",
            &format!("physicsHitReaction Rock {{ {many} }}"),
        )
        .unwrap_err();
    assert_eq!(
        err.0,
        "java.lang.ArrayIndexOutOfBoundsException: Index 11 out of bounds for length 11"
    );
}

#[test]
fn builtin_constants_name_keys() {
    let (sm, m) = manager("");
    let food = register(&sm, "item_type", "Edible");
    sm.statics
        .items
        .borrow_mut()
        .constants
        .insert("item_type", "FOOD", "Edible");
    let (it, r) = load_item(
        &sm,
        &m,
        "Apple",
        "item Apple { ItemType = edible, Icon = Apple, }",
    );
    r.unwrap();
    assert!(it.is_item_type(&sm, "FOOD"));
    assert!(it.is_item_type_key(Some(&food)));
    assert_eq!(
        ItemType::constant(&sm, "FOOD").unwrap().location,
        food.location
    );
    // Without textures, the rotten and burnt names take the last fallbacks
    // the original's replacement chain leaves.
    assert_eq!(
        it.special_world_texture_names,
        [
            "media/inventory/world/WItem_AppleSpoiled.png",
            "media/inventory/world/WItem_AppleCooked.png",
            "media/inventory/world/WItem_AppleBurnt.png"
        ]
    );
    assert_eq!(
        it.world_texture_name.as_deref(),
        Some("media/inventory/world/WItem_Apple.png")
    );
}

#[test]
fn create_instance_covers_group_types() {
    let m = ScriptModule::new();
    for t in [
        ScriptType::Item,
        ScriptType::ItemFilter,
        ScriptType::CharacterTraitDefinition,
        ScriptType::CharacterProfessionDefinition,
        ScriptType::PhysicsHitReaction,
    ] {
        assert!(create_instance(t, &m, "x", "").is_some());
    }
    assert!(create_instance(ScriptType::Recipe, &m, "x", "").is_none());
}

/// Invented perk ids standing in for the jar's `PerkFactory.Perks`.
fn test_perks() -> Vec<String> {
    ["Axe", "Cooking", "Fitness", "Woodwork"].map(String::from).to_vec()
}
