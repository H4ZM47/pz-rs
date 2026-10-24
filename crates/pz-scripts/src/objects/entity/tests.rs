use std::rc::Rc;

use super::components::attributes::AttributesScript;
use super::components::combat::{DurabilityScript, MaterialType};
use super::components::contextmenuconfig::ContextMenuConfigScript;
use super::components::crafting::*;
use super::components::fluids::FluidContainerScript;
use super::components::resources::{
    ResourceBlueprint, ResourceChannel, ResourceFlag, ResourceIO, ResourceType, ResourcesScript,
};
use super::components::sound::CraftBenchSoundsScript;
use super::components::spriteconfig::{SpriteConfigScript, SpriteOverlayConfigScript};
use super::components::ui::UiConfigScript;
use super::item_config::{load_selector_bucket, SelectorType, SituatedType};
use super::*;
use crate::base::ScriptObject;
use crate::manager::ScriptManager;
use crate::module::ScriptModule;
use crate::script_type::ScriptLoadMode;

/// A manager with module `Base` registered from `body` (templates and
/// script stubs created, nothing loaded yet).
fn manager(body: &str) -> (ScriptManager, Rc<ScriptModule>) {
    let sm = ScriptManager::new();
    sm.parse_script(ScriptLoadMode::Init, &format!("module Base {{ {body} }}")).unwrap();
    let m = sm.get_module("Base").unwrap();
    (sm, m)
}

fn load_entity(sm: &ScriptManager, m: &Rc<ScriptModule>, name: &str, body: &str) -> (GameEntityScript, ScriptResult) {
    let mut e = GameEntityScript::new();
    e.base.set_module(m);
    e.init_load_pp(sm, name);
    let r = e.load(sm, name, body);
    (e, r)
}

fn load_recipe(sm: &ScriptManager, m: &Rc<ScriptModule>, name: &str, body: &str) -> (CraftRecipe, ScriptResult) {
    let mut r = CraftRecipe::new();
    r.base.set_module(m);
    r.init_load_pp(sm, name);
    let res = r.load(sm, name, body);
    (r, res)
}

#[test]
fn tag_query_formatting() {
    assert_eq!(format_recipe_tag_query("Zeta;alpha; Mid ").unwrap(), "alpha;mid;zeta");
    assert_eq!(format_recipe_tag_query("B;a-Y;x").unwrap(), "a;b-x;y");
    assert_eq!(format_recipe_tag_query("-Q;p").unwrap(), "-p;q");
    assert!(format_recipe_tag_query("  ").is_err());
}

#[test]
fn java_string_helpers() {
    assert!(contains_whitespace("a b"));
    assert!(contains_whitespace("a\nb"));
    assert!(!contains_whitespace("a\n\nb"));
    assert!(!contains_whitespace("ab"));
    assert_eq!(remove_whitespace(" a\tb "), "ab");
    assert_eq!(bracket_contents("x[a;b]").unwrap(), "a;b");
    assert_eq!(
        bracket_contents("plain").unwrap_err().0,
        "java.lang.StringIndexOutOfBoundsException: begin 0, end -1, length 5"
    );
    let mut w = [1.0, 3.0];
    normalize(&mut w);
    assert_eq!(w, [0.25, 0.75]);
}

#[test]
fn component_type_values() {
    assert_eq!(ComponentType::CraftRecipe.id(), 20);
    assert_eq!(ComponentType::from_id(3), ComponentType::SpriteConfig);
    assert_eq!(ComponentType::from_id(99), ComponentType::Undefined);
    assert!(ComponentType::Resources.is_add_to_engine() && ComponentType::Resources.is_run_in_meta());
    assert!(ComponentType::FluidContainer.is_run_in_meta() && !ComponentType::FluidContainer.is_add_to_engine());
    assert_eq!(ComponentType::list().len(), 23);
    assert_eq!(
        ComponentType::MetaTag.create_component_script().unwrap_err().0,
        "java.lang.RuntimeException: Unable to create script for component (No script class defined?): MetaTag"
    );
    assert_eq!(
        ComponentType::value_of("bogus").unwrap_err().0,
        "java.lang.IllegalArgumentException: No enum constant zombie.entity.ComponentType.bogus"
    );
}

#[test]
fn entity_components_and_unknown_blocks() {
    let (sm, m) = manager("");
    let body = "entity Bench { component UiConfig { xuiSkin = Skin, entityStyle = Style, uiEnabled = false, } \
        component Nope { } component CraftLogic { recipes = B;a, startMode = Automatic, inputGroup = in, } \
        stray { } }";
    let (mut e, r) = load_entity(&sm, &m, "Bench", body);
    r.unwrap();
    assert_eq!(e.name(), Some("Bench"));
    assert_eq!(e.component_scripts().len(), 2);
    let ui = e.component::<UiConfigScript>(ComponentType::UiConfig).unwrap();
    assert_eq!((ui.xui_skin_name(), ui.entity_style(), ui.is_ui_enabled()), (Some("Skin"), Some("Style"), false));
    assert_eq!(ui.component.base.script_object_name(), "Bench");
    let log = sm.take_log();
    assert!(log.iter().any(|l| l.message == "Could not parse component block id = Nope"));
    assert!(log.iter().any(|l| l.message == "Unknown block 'stray' in entity  script: Bench"));
    e.on_scripts_loaded(&sm, ScriptLoadMode::Init).unwrap();
    let logic = e.component::<CraftLogicScript>(ComponentType::CraftLogic).unwrap();
    assert_eq!(logic.recipe_tag_query(), Some("a;b"));
    assert_eq!(logic.start_mode(), StartMode::Automatic);
    assert_eq!(logic.inputs_group_name(), Some("in"));
    e.pre_reload();
    assert!(!e.has_components());
}

#[test]
fn entity_template_copies_components() {
    let (sm, m) = manager(
        "template entity Tpl { component Durability { MaxHitPoints = 12.5, Material = Wood_Solid, } \
         component UiConfig { xuiSkin = S, } }",
    );
    let (e, r) = load_entity(&sm, &m, "Thing", "entity Thing { entitytemplate = Tpl, }");
    r.unwrap();
    let d = e.component::<DurabilityScript>(ComponentType::Durability).unwrap();
    assert_eq!((d.max_hit_points(), d.current_hit_points()), (12.5, 12.5));
    assert_eq!(d.material(), Some(MaterialType::Wood_Solid));
    // UiConfigScript.copyFrom copies nothing.
    let ui = e.component::<UiConfigScript>(ComponentType::UiConfig).unwrap();
    assert_eq!(ui.xui_skin_name(), None);
    assert_eq!(ui.component.base.script_object_name(), "Thing");

    let (_, r) = load_entity(&sm, &m, "Missing", "entity Missing { entitytemplate = Nope, }");
    assert_eq!(r.unwrap_err().0, "java.lang.NullPointerException");
}

#[test]
fn attributes_and_load_attribute() {
    let (sm, m) = manager("");
    let (mut e, r) = load_entity(&sm, &m, "A", "entity A { component Attributes { Sharpness = 0.5, Bogus = 1, } }");
    r.unwrap();
    assert!(sm.take_log().iter().any(|l| l.message == "Unknown attribute, key = Bogus, value = 1"));
    assert!(e.load_attribute(&sm, "quality", "10").unwrap());
    assert!(!e.load_attribute(&sm, "other", "1").unwrap());
    let a = e.component::<AttributesScript>(ComponentType::Attributes).unwrap();
    assert_eq!(a.kv_pairs().len(), 2);
    assert_eq!(a.kv_pairs().get("Sharpness").map(String::as_str), Some("0.5"));
}

#[test]
fn fluid_container_loads_and_normalizes() {
    let (sm, m) = manager("");
    let body = "entity Tank { component FluidContainer { Capacity = 0.01, ContainerName = My Tank, \
        InitialPercentMin = 0.8, InitialPercentMax = 0.2, \
        fluids { fluid = Water:3, fluid = Juice:1:0.1:0.2:0.3, Fluid { type = Oil, percentage = 4, g = 0.5, } } \
        whitelist { fluid = Water/Oil, } } }";
    let (mut e, r) = load_entity(&sm, &m, "Tank", body);
    r.unwrap();
    e.on_scripts_loaded(&sm, ScriptLoadMode::Init).unwrap();
    let f = e.component::<FluidContainerScript>(ComponentType::FluidContainer).unwrap();
    assert_eq!(f.container_name(), "MyTank");
    assert_eq!(f.capacity(), 0.05);
    assert_eq!(f.initial_amount_range(), (0.2 * 0.05, 0.8 * 0.05));
    let fluids = f.initial_fluids().unwrap();
    assert_eq!(fluids.iter().map(|x| x.fluid_type()).collect::<Vec<_>>(), ["Water", "Juice", "Oil"]);
    assert_eq!(fluids.iter().map(|x| x.percentage()).collect::<Vec<_>>(), [0.375, 0.125, 0.5]);
    assert_eq!(fluids[2].custom_color().unwrap().g, 0.5);
    assert_eq!(f.whitelist().unwrap().fluids(), ["Water", "Oil"]);
    assert!(sm.take_log().iter().any(|l| l.message.starts_with("Sanitizing container name 'My Tank'")));

    let (_, r) = load_entity(&sm, &m, "Bad", "entity Bad { component FluidContainer { Capacity = , } }");
    assert_eq!(r.unwrap_err().0, "java.lang.NumberFormatException: empty String");
    let (_, r) = load_entity(&sm, &m, "Bad", "entity Bad { component FluidContainer { Capacity=, } }");
    assert_eq!(r.unwrap_err().0, "java.lang.ArrayIndexOutOfBoundsException: Index 1 out of bounds for length 1");
}

#[test]
fn resources_blueprints() {
    let (sm, m) = manager("");
    let body = "entity R { component Resources { in = Item@Input@4, \
        Fluid@Output@2.5@null@Channel_Blue@AutoDecay:Temporary, \
        group extra { e = Energy@Any@1@StackAny, } } }";
    let (mut e, r) = load_entity(&sm, &m, "R", body);
    r.unwrap();
    e.on_scripts_loaded(&sm, ScriptLoadMode::Init).unwrap();
    let res = e.component::<ResourcesScript>(ComponentType::Resources).unwrap();
    let main = res.blueprint_group("resources").unwrap();
    assert_eq!(main[0].id(), Some("in"));
    assert_eq!(
        (main[0].resource_type(), main[0].io(), main[0].capacity()),
        (ResourceType::Item, ResourceIO::Input, 4.0)
    );
    assert_eq!(main[1].id(), Some("resources_1"));
    assert_eq!(main[1].channel(), ResourceChannel::Channel_Blue);
    assert!(main[1].has_flag(ResourceFlag::AutoDecay) && main[1].has_flag(ResourceFlag::Temporary));
    assert!(res.blueprint_group("extra").unwrap()[0].is_stack_any_item());
    assert_eq!(res.blueprints().len(), 3);

    assert_eq!(
        ResourceBlueprint::deserialize_from_script("a@Item@Input").unwrap_err().0,
        "java.lang.IllegalArgumentException: Serial string has invalid number of elements."
    );
    assert_eq!(
        ResourceBlueprint::deserialize_from_script("a@Item@Input@1.5").unwrap_err().0,
        "java.lang.NumberFormatException: For input string: \"1.5\""
    );
}

#[test]
fn sprite_config_faces_and_quirks() {
    let (sm, m) = manager("");
    let body = "entity S { component SpriteConfig { previousStage = a;b, health = x, \
        face N { layer { row = t_0 t_1, row = false t_2, } layer { row = t_3, } } \
        face N_OPEN { row = t_4 t_5, } face bogus { } } }";
    let (e, r) = load_entity(&sm, &m, "S", body);
    r.unwrap();
    let s = e.component::<SpriteConfigScript>(ComponentType::SpriteConfig).unwrap();
    let n = s.face(0).unwrap();
    assert_eq!((n.face_name(), n.z_layers(), n.total_width(), n.total_height()), ("n".to_string(), 2, 2, 2));
    assert!(n.layer(0).unwrap().row(1).unwrap().tile(0).unwrap().is_empty_space());
    assert_eq!(s.all_tile_names(), ["t_0", "t_1", "t_2", "t_3", "t_4", "t_5"]);
    assert!(s.is_multi_tile() && s.is_valid());
    assert_eq!(s.health(), -1);
    // Values are read once per valid child block.
    assert_eq!(s.previous_stages(), ["a", "b", "a", "b"]);
    let log = sm.take_log();
    assert!(log.iter().any(|l| l.message == "Cannot find face for: bogus"));
    assert!(log.iter().any(|l| l.message.starts_with("Complementary faces are different sizes.")));
}

#[test]
fn sprite_overlay_styles() {
    let (sm, m) = manager("");
    let body = "entity O { component SpriteOverlayConfig { style Dirty { progress 1 { face single { row = d_0, } } \
        progress 1 { } Progress 2 { } } } }";
    let (e, r) = load_entity(&sm, &m, "O", body);
    r.unwrap();
    let o = e.component::<SpriteOverlayConfigScript>(ComponentType::SpriteOverlayConfig).unwrap();
    assert!(o.is_valid());
    assert_eq!(o.all_style_names(), ["Dirty_1"]);
    let st = o.style("Dirty_1").unwrap();
    assert!(st.is_single_face() && st.is_valid());
    assert_eq!(st.all_tile_names(), ["d_0"]);
    assert!(sm.take_log().iter().any(|l| l.message == "Duplicate style name and progress (not allowed): Dirty_1"));
}

#[test]
fn misc_components() {
    let (sm, m) = manager("");
    let body =
        "entity M { component ContextMenuConfig { contextEntry { menu = Use, time = 7, allowDistance = TRUE, } } \
        component CraftBenchSounds { Start = SndA p1 p2 p3, } \
        component CraftBench { recipes = X, fluidInputChannels = Channel_Red;NO_CHANNEL, } \
        component WallCoveringConfig { type = PLASTER, sign = 33, name = Wall, } }";
    let (mut e, r) = load_entity(&sm, &m, "M", body);
    r.unwrap();
    let c = &e.component::<ContextMenuConfigScript>(ComponentType::ContextMenuConfig).unwrap().entries()[0];
    assert_eq!((c.menu.as_deref(), c.time, c.allow_distance), (Some("Use"), 7, true));
    let snd = &e.component::<CraftBenchSoundsScript>(ComponentType::CraftBenchSounds).unwrap().sounds()[0];
    assert_eq!((snd.param2.as_deref(), snd.param3.as_deref()), (Some("p2"), Some("p2")));
    let bench = e.component::<CraftBenchScript>(ComponentType::CraftBench).unwrap();
    assert_eq!(
        bench.fluid_input_channels().iter().copied().collect::<Vec<_>>(),
        [ResourceChannel::NO_CHANNEL, ResourceChannel::Channel_Red]
    );
    let w = e.component::<WallCoveringConfigScript>(ComponentType::WallCoveringConfig).unwrap();
    assert_eq!(w.covering_type(), Some(WallCoveringType::PLASTER));
    assert_eq!(w.sign_sprite_name().unwrap(), "constructedobjects_signs_01_33");
    assert_eq!(w.name(), Some("Wall"));
    e.on_scripts_loaded(&sm, ScriptLoadMode::Init).unwrap();
    assert_eq!(e.component::<CraftBenchScript>(ComponentType::CraftBench).unwrap().recipe_tag_query(), Some("x"));

    let (mut e, r) = load_entity(&sm, &m, "N", "entity N { component CraftBench { } }");
    r.unwrap();
    assert_eq!(
        e.on_scripts_loaded(&sm, ScriptLoadMode::Init).unwrap_err().0,
        "java.lang.Exception: Recipe tag query null or whitespace."
    );
}

#[test]
fn craft_recipe_io_lines_and_mappers() {
    let (sm, m) = manager("");
    let body = "craftRecipe Make { time = 20, Tags = InHandCraft;Other, SkillRequired = Woodwork:3;Nope:2;Cooking:0, \
        xpAward = Woodwork:5, icon = Saw, category = Build, \
        inputs { item 1 [Saw] flags[ToolLeft;Prop1] mode:keep, -fluid 0.5 [Water], \
                 item 2 [Base.A;3:Base.B] mappers[pick] overlayMapper, item variable[1:4] [*], } \
        outputs { item 1 mapper:pick, +item 1 Uses, } \
        itemMapper pick { Base.Out1 = Base.A;fluid$x, default = Base.Out2, } }";
    // `fluid$` patterns are rejected.
    let (_, res) = load_recipe(&sm, &m, "Make", body);
    assert_eq!(res.unwrap_err().0, "java.lang.Exception: Not yet implemented");
    let body = body.replace("fluid$x", "item$Base.B");
    let (mut r, res) = load_recipe(&sm, &m, "Make", &body);
    res.unwrap();
    assert_eq!(r.time(), 20);
    assert!(r.is_in_hand_craft());
    assert_eq!(r.required_skills(), ["Woodwork 3"]);
    assert!(sm.take_log().iter().any(|l| l.message == "Unknown skill \"Nope\" in recipe \"Make\""));
    assert_eq!(r.io_lines().len(), 6);
    assert_eq!(r.input_indices(), [0, 2, 3]);
    assert_eq!(r.output_indices(), [4]);
    let saw = r.io_lines()[0].as_input().unwrap();
    assert_eq!(saw.consume_from_item_script(), Some(1));
    assert_eq!(r.io_lines()[1].as_input().unwrap().parent_script(), Some(0));
    assert!(r.io_lines()[1].as_input().unwrap().is_accepts_any_item());
    let ab = r.io_lines()[2].as_input().unwrap();
    assert_eq!(ab.loaded_items(), ["Base.A", "Base.B"]);
    // Differing per-item amounts reset the line amount to 1.
    assert_eq!((ab.amount(), ab.amounts(), ab.max_amounts()), (1.0, &[2.0, 3.0][..], &[2.0, 3.0][..]));
    let any = r.io_lines()[3].as_input().unwrap();
    assert!(any.is_accepts_any_item() && any.is_variable_amount());
    let mapper = r.output_mapper("pick").unwrap();
    assert_eq!(mapper.input_scripts(), [2]);
    assert_eq!(mapper.entrees()[0].pattern_full_types(), ["Base.A", "Base.B"]);
    assert_eq!(r.overlay_mapper().input_script(), Some(2));
    let out = r.io_lines()[4].as_output().unwrap();
    assert_eq!(out.create_to_item_script(), Some(5));
    assert!(r.output_mapper_for(out).is_some());
    assert_eq!(r.outputs().count(), 1);

    r.on_scripts_loaded(&sm, ScriptLoadMode::Init).unwrap();
    assert_eq!(r.icon_name(), Some("Item_Saw"));
    assert!(r.is_uses_tools());
    assert_eq!(r.tool_left().unwrap().original_line(), "item 1 [Saw] flags[ToolLeft;Prop1] mode:keep");
    assert_eq!(r.prop1().map(|p| p.loaded_items().to_vec()), Some(vec!["Saw".to_string()]));
    assert_eq!(r.get_research_skill_level(), 2);
}

#[test]
fn craft_recipe_errors() {
    let (sm, m) = manager("");
    let cases = [
        ("inputs { +item 1 Base.X, }", "java.io.IOException: Previous input is null [R] line: +item 1 Base.X"),
        ("outputs { -item 1 Base.X, }", "java.io.IOException: Cannot add '-' line to output, [R] line: -item 1 Base.X"),
        ("inputs { thing 1 [A], }", "java.lang.Exception: unknown type in craftrecipe: thing"),
        ("inputs { item 1 [A] what, }", "java.lang.Exception: unknown recipe param: what"),
        ("inputs { item x [A], }", "java.lang.NumberFormatException: For input string: \"x\""),
        ("inputs { fluid 1 [A] tags[x], }", "java.lang.Exception: cannot set tags on non-item: tags[x]"),
        ("outputs { item 1 mapper:none, }", "java.lang.Exception: Could not find output mapper: none"),
        ("outputs { item 1 X apply:onTick, }", "java.lang.Exception: Cannot apply 'onTick' on item."),
        (
            "inputs { item 1 [A] flags[Nope], }",
            "java.lang.IllegalArgumentException: No enum constant zombie.entity.components.crafting.InputFlag.Nope",
        ),
        ("fluidMapper x { }", "java.lang.Exception: Not yet implemented."),
        ("itemMapper m { }", "java.lang.Exception: Failed to load contents for output mapper: m"),
    ];
    for (inner, expected) in cases {
        let (_, res) = load_recipe(&sm, &m, "R", &format!("craftRecipe R {{ {inner} }}"));
        assert_eq!(res.unwrap_err().0, expected, "{inner}");
    }
    let (mut r, res) = load_recipe(&sm, &m, "R", "craftRecipe R { timedAction = Missing, }");
    res.unwrap();
    assert_eq!(
        r.on_scripts_loaded(&sm, ScriptLoadMode::Init).unwrap_err().0,
        "java.lang.Exception: TimedActionScript 'Missing' could not be found in recipe R"
    );
    let body = "craftRecipe R { inputs { item 1 [A] flags[Prop1], item 1 [B] flags[Prop1], } }";
    let (mut r, res) = load_recipe(&sm, &m, "R", body);
    res.unwrap();
    assert_eq!(
        r.on_scripts_loaded(&sm, ScriptLoadMode::Init).unwrap_err().0,
        "java.io.IOException: Duplicate Prop1 in recipe R"
    );
}

#[test]
fn craft_recipe_component_is_tagged() {
    let (sm, m) = manager("");
    let (e, r) = load_entity(&sm, &m, "Wall", "entity Wall { component CraftRecipe { time = 5, Tags = Carpentry, } }");
    r.unwrap();
    let c = e.component::<CraftRecipeComponentScript>(ComponentType::CraftRecipe).unwrap();
    let recipe = c.craft_recipe().unwrap();
    assert_eq!(recipe.name(), Some("Wall"));
    assert_eq!(recipe.tags(), ["Carpentry", "EntityRecipe"]);
    assert!(recipe.is_buildable_recipe());
    assert_eq!(c.build_category(), Some("Carpentry"));
    assert_eq!(recipe.base.full_type().unwrap(), "Base.Wall");
}

#[test]
fn item_config_roots_and_errors() {
    let (sm, m) = manager("");
    let mut ic = ItemConfig::new();
    ic.base.set_module(&m);
    let body = "itemConfig C { include = Other, includes { More, } variables { $a = 1, } }";
    ic.load(&sm, "C", body).unwrap();
    assert_eq!(ic.includes(), ["Other", "More"]);
    assert_eq!(ic.variables().len(), 1);

    // The original keeps a root id only when it is blank (inverted test):
    // a missing id dereferences null, a real one is dropped.
    let mut bad = ItemConfig::new();
    let err = bad.load(&sm, "Bad", "itemConfig Bad { LuaFunc { } }").unwrap_err();
    assert_eq!(
        err.0,
        "zombie.scripting.itemConfig.ItemConfig$ItemConfigException: RecipeAttributes -> null \n[itemConfig=Bad, line=\"null\"]]"
    );
    let err = bad.load(&sm, "Bad", "itemConfig Bad { Attribute Sharpness { } }").unwrap_err();
    assert_eq!(
        err.0,
        "zombie.scripting.itemConfig.ItemConfig$ItemConfigException: RecipeAttributes -> Root node with type \
         'Attribute' requires id. \n[itemConfig=Bad, attribute=Sharpness, line=\"null\"]]"
    );

    // `Other` is not a loaded item config: getItemConfig returns null.
    assert_eq!(
        ic.on_scripts_loaded(&sm, ScriptLoadMode::Init).unwrap_err().0,
        "java.lang.Exception: java.lang.NullPointerException"
    );
    let mut lp = ItemConfig::new();
    lp.base.set_module(&m);
    lp.init_load_pp(&sm, "Loop");
    lp.load(&sm, "Loop", "itemConfig Loop { include = Loop, }").unwrap();
    sm.add_runtime_script(crate::base::ScriptRef::new(lp)).unwrap();
    let lp = sm.get::<ItemConfig>(crate::script_type::ScriptType::ItemConfig, "Loop").unwrap();
    let err = lp.borrow_mut().on_scripts_loaded(&sm, ScriptLoadMode::Init).unwrap_err();
    assert!(err.0.contains("Circular includes detected."), "{}", err.0);
}

#[test]
fn item_config_selector_buckets() {
    let sm = ScriptManager::new();
    let root =
        sm.parse("LuaFunc { zone Forest/Farm { chance=1 func=g, } worldagedays 3 { situated shop { func=h, } } }");
    let block = &root.children[0];
    let zone = load_selector_bucket(&sm, &block.children[0], true).unwrap();
    assert_eq!((zone.selector_type(), zone.selector_string()), (SelectorType::Zone, Some("Forest/Farm")));
    assert_eq!(zone.randomizers(), ["chance=1 func=g"]);
    let age = load_selector_bucket(&sm, &block.children[1], true).unwrap();
    assert_eq!((age.selector_type(), age.selector_world_age()), (SelectorType::WorldAge, 3));
    assert_eq!(age.children()[0].selector_situated(), Some(SituatedType::Shop));
    let root = sm.parse("x { default { a { } } custom { v=1, } }");
    let err = load_selector_bucket(&sm, &root.children[0].children[0], false).unwrap_err();
    assert!(err.0.contains("Default block may not have any nested children."), "{}", err.0);
    let err = load_selector_bucket(&sm, &root.children[0].children[1], true).unwrap_err();
    assert!(err.0.contains("A custom container bucket may not have any randomizer parameters."), "{}", err.0);
}
