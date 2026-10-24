use super::*;
use crate::base::ScriptObject;
use crate::manager::ScriptManager;
use crate::script_type::ScriptLoadMode;

fn load<T: ScriptObject>(mut s: T, name: &str, body: &str) -> (T, ScriptResult) {
    let sm = ScriptManager::new();
    sm.builtin_tables.borrow_mut().perk_ids = test_perks();
    let r = s.load(&sm, name, body);
    (s, r)
}

#[test]
fn energy_type_from_name_and_color() {
    let (e, r) = load(EnergyDefinitionScript::new(), "electric", "energy electric { color = 0.5:0.25:1, r = 0.1, }");
    r.unwrap();
    assert_eq!(e.energy_type(), EnergyType::Electric);
    assert_eq!(e.color(), Color::new(0.1, 0.25, 1.0, 1.0));

    let (mut e, r) = load(EnergyDefinitionScript::new(), "Plasma", "energy Plasma { color = 1:2, }");
    r.unwrap();
    assert_eq!(e.energy_type(), EnergyType::Modded);
    assert_eq!(e.energy_type_string(), Some("Plasma"));
    assert_eq!(e.color(), Color::new(1.0, 1.0, 1.0, 1.0));
    e.on_scripts_loaded(&ScriptManager::new(), ScriptLoadMode::Init).unwrap();

    let (mut e, _) = load(EnergyDefinitionScript::new(), "none", "energy none { }");
    let err = e.on_scripts_loaded(&ScriptManager::new(), ScriptLoadMode::Init).unwrap_err();
    assert_eq!(err.0, "java.lang.Exception: No energy type set.");
}

#[test]
fn fluid_definition_blocks() {
    let body = "fluid Syrup { displayName = Syrup, color = 1:0.5:0, \
        categories { Beverage, Water, Beverage, } \
        properties { thirstChange = -50, foodSicknessChange = 2.9, } \
        poison { maxEffect = Mild, minAmount = 0.5, } \
        blendWhiteList { fluids { Water/Beer, } } }";
    let (f, r) = load(FluidDefinitionScript::new(), "Syrup", body);
    r.unwrap();
    assert_eq!(f.fluid_type(), FluidType::Modded);
    assert_eq!(f.display_name(), "Syrup");
    assert_eq!(f.categories().iter().copied().collect::<Vec<_>>(), vec![FluidCategory::Beverage, FluidCategory::Water]);
    assert_eq!(f.thirst_change(), -0.5);
    assert_eq!(f.food_sickness_change(), 2);
    assert!(f.has_properties_set());
    assert_eq!(f.poison_max_effect(), PoisonEffect::Mild);
    let wl = f.blend_whitelist().unwrap();
    assert!(wl.is_whitelist() && wl.is_anonymous());
    assert_eq!(wl.fluids(), ["Water", "Beer"]);

    let (_, r) = load(FluidDefinitionScript::new(), "x", "fluid x { categories { beverage, } }");
    assert_eq!(
        r.unwrap_err().0,
        "java.lang.IllegalArgumentException: No enum constant zombie.entity.components.fluids.FluidCategory.beverage"
    );
    let (_, r) = load(FluidDefinitionScript::new(), "x", "fluid x { r = abc, }");
    assert_eq!(r.unwrap_err().0, "java.lang.NumberFormatException: For input string: \"abc\"");
}

#[test]
fn fluid_filter_lists() {
    let body = "fluidFilter F { fluid = A/B/A, filterType = blacklist, categories { Fuel, x = y, } }";
    let (mut f, r) = load(FluidFilterScript::new(), "F", body);
    r.unwrap();
    assert_eq!(f.fluids(), ["A", "B"]);
    assert_eq!(f.categories(), ["Fuel"]);
    assert!(!f.is_whitelist());
    let filter = f.create_filter().unwrap();
    assert_eq!(filter.filter_type, FilterType::Blacklist);
    assert_eq!(filter.categories, vec![FluidCategory::Fuel]);

    let (_, r) = load(FluidFilterScript::get_anonymous(), "F", body);
    assert_eq!(r.unwrap_err().0, "java.lang.Exception: Cannot load, is anonymous");
}

#[test]
fn timed_action_fields() {
    let body = "timedAction T { time = 50, metabolics = LightWork, soundTime = animation_event, \
        muscleStrainParts = hand_l; Torso_Upper ;nope, muscleStrainSkill = Axe, faceObject = TRUE, }";
    let (t, r) = load(TimedActionScript::new(), "T", body);
    r.unwrap();
    assert_eq!(t.time(), 50);
    assert_eq!(t.metabolics(), Metabolics::LightWork);
    assert_eq!(t.sound_time(), ActionSoundTime::AnimationEvent);
    assert_eq!(t.muscle_strain_parts(), [BodyPartType::Hand_L, BodyPartType::Torso_Upper, BodyPartType::MAX]);
    assert_eq!(t.muscle_strain_skill(), Some("Axe"));
    assert!(t.is_face_object());

    let (_, r) = load(TimedActionScript::new(), "T", "timedAction T { time = 1.5, }");
    assert_eq!(r.unwrap_err().0, "java.lang.NumberFormatException: For input string: \"1.5\"");
    let (_, r) = load(TimedActionScript::new(), "T", "timedAction T { soundTime = ACTION_START, }");
    assert_eq!(r.unwrap_err().0, "java.lang.IllegalArgumentException: No enum constant for value: ACTION_START");
}

#[test]
fn xui_colors_both_notations() {
    let (c, r) = load(XuiColorsScript::new(), "C", "xuiGlobalColors C { a = 0.5:0.5, b = rgb:255:0:51:255:9, }");
    r.unwrap();
    assert_eq!(c.color_map().get("a"), Some(&Color::new(0.5, 0.5, 0.0, 1.0)));
    assert_eq!(c.color_map().get("b"), Some(&Color::new(1.0, 0.0, 0.2, 1.0)));
}

#[test]
fn xui_config_duplicates() {
    let (c, r) = load(XuiConfigScript::new(), "C", "xuiConfig C { String { a, b, a, } Boolean { c, } }");
    r.unwrap();
    assert_eq!(c.var_config(XuiVarType::String).unwrap(), ["a", "b"]);
    assert_eq!(c.var_config(XuiVarType::Boolean).unwrap(), ["c"]);

    let (_, r) = load(XuiConfigScript::new(), "C", "xuiConfig C { String { a, } Color { a, } }");
    assert!(r.unwrap_err().0.contains("Key 'a' duplicate from another value block"));
    let (_, r) = load(XuiConfigScript::new(), "C", "xuiConfig C { Unit { a, } }");
    assert_eq!(r.unwrap_err().0, "java.lang.Exception: VarType not allowed: Unit");
}

#[test]
fn xui_layout_defers_parsing() {
    let (mut l, r) = load(
        XuiLayoutScript::new(ScriptType::XuiStyle, XuiScriptType::Style),
        "S",
        "xuiStyle S { enabled = false, xuiLuaClass = ISButton, x = 1, }",
    );
    r.unwrap();
    assert!(!l.base.enabled);
    l.pre_parse().unwrap();
    assert_eq!(l.lua_class(), Some("ISButton"));
    assert_eq!(l.parse_script().unwrap().values.len(), 3);
    assert!(l.parse_script().is_none());
}

#[test]
fn xui_skin_sections() {
    let body = "xuiSkin S { \
        imports { Other, } \
        Button { a = \"x\", b = nil, c = , } \
        Button Big { a = 2, } \
        entity { clearComponents = true, components { UiConfig = PanelA:StyleA, Parts { listOrderZ = 4, } } } \
        entity Crate { luaWindowClass = W, } \
        colors { red = 1:0:0, } }";
    let (s, r) = load(XuiSkinScript::new(), "S", body);
    r.unwrap();
    assert_eq!(s.imports(), ["Other"]);
    let button = s.style_info_map().get("Button").unwrap();
    assert_eq!(button.default_style_block.get("a"), Some(&Some("x".to_string())));
    assert_eq!(button.default_style_block.get("b"), Some(&None));
    assert_eq!(button.default_style_block.get("c"), Some(&None));
    assert_eq!(button.style_blocks.get("Big").unwrap().get("a"), Some(&Some("2".to_string())));
    let def = s.default_entity_ui_script();
    assert!(def.clear_components);
    let ui = def.component_ui_script(ComponentType::UiConfig).unwrap();
    assert_eq!((ui.lua_panel_class.as_deref(), ui.xui_style.as_deref()), (Some("PanelA"), Some("StyleA")));
    let parts = def.component_ui_script(ComponentType::Parts).unwrap();
    assert!(parts.list_order_z_set && parts.list_order_z == 4);
    assert_eq!(s.entity_ui_script_map().get("Crate").unwrap().lua_window_class.as_deref(), Some("W"));
    assert_eq!(s.colors_script().color_map().get("red"), Some(&Color::new(1.0, 0.0, 0.0, 1.0)));

    let (_, r) = load(XuiSkinScript::new(), "S", "xuiSkin S { Button default { } }");
    assert_eq!(r.unwrap_err().0, "java.lang.Exception: Default is protected and cannot be used as style name.");
    let (_, r) = load(XuiSkinScript::new(), "S", "xuiSkin S { entity { components { UiConfig = :, } } }");
    assert_eq!(r.unwrap_err().0, "java.lang.ArrayIndexOutOfBoundsException: Index 0 out of bounds for length 0");
    let (_, r) = load(XuiSkinScript::new(), "S", "xuiSkin S { Button { a = \", } }");
    assert!(r.unwrap_err().0.starts_with("java.lang.StringIndexOutOfBoundsException"));
}

/// Invented perk ids standing in for the jar's `PerkFactory.Perks`.
fn test_perks() -> Vec<String> {
    ["Axe", "Cooking", "Fitness", "Woodwork"].map(String::from).to_vec()
}
