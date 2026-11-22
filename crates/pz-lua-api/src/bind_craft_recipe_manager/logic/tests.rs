use super::*;
use pz_scripts::ScriptManager;

fn thread_with_player() -> Thread {
    let mut t = Thread::new(TableRef::new());
    pz_kahlua::stdlib::install(&mut t, None).unwrap();
    crate::bind_character_sim::register(&mut t, Rc::new(ScriptManager::new()), Default::default());
    register(&mut t, Rc::new(ScriptManager::new()), Default::default());
    let player = Rc::new(RefCell::new(crate::bind_character_sim::LuaGameCharacter::new(
        &ScriptManager::new(),
        pz_characters::CharacterKind::Player,
    )));
    // `filterRecipeList` reads `IsoPlayer.getSpecificPlayer(0)`.
    crate::bind_players::set_player(0, Some(player.clone()));
    let player = Value::Object(Rc::new(CharacterObject::try_new(&player).unwrap()));
    t.globals().set_str("player", player);
    t
}

fn run(t: &mut Thread, src: &str) -> Result<Vec<String>, String> {
    let f = pz_kahlua::stdlib::load_string(src, Some("test"), t.globals().clone()).unwrap();
    t.pcall(&f, &[])
        .map(|out| out.iter().map(pz_kahlua::exec::java_value_of).collect())
        .map_err(|(e, _)| pz_kahlua::exec::java_value_of(&e))
}

#[test]
fn the_constructor_reads_the_view_settings_from_the_player_mod_data() {
    let _collection = pz_kahlua::gc::defer_collection();
    let mut t = thread_with_player();
    let out = run(
        &mut t,
        r#"
        local logic = HandcraftLogic.new(player, nil, nil)
        local a = { logic:getRecipeSortMode(), logic:isManualSelectInputs(), tostring(logic:getSelectedRecipeStyle()) }
        player:getModData().handcraftManualInputs = "true"
        logic:setSelectedRecipeStyle("grid")
        logic:setRecipeSortMode("MostUsed")
        local b = HandcraftLogic.new(player, nil, nil)
        return a[1], a[2], a[3], b:isManualSelectInputs(), b:getSelectedRecipeStyle(), b:getRecipeSortMode()
        "#,
    )
    .unwrap();
    assert_eq!(out, ["RecipeName", "false", "nil", "true", "grid", "MostUsed"]);
}

#[test]
fn the_constructor_needs_a_player_and_no_craft_bench() {
    let _collection = pz_kahlua::gc::defer_collection();
    let mut t = thread_with_player();
    let err = run(&mut t, "return HandcraftLogic.new(nil, nil, nil)").unwrap_err();
    assert!(err.contains("NullPointerException"), "{err}");
    let err = run(&mut t, "return HandcraftLogic.new(player, {}, nil)").unwrap_err();
    assert!(err.contains("CraftBench"), "{err}");
    let err = run(&mut t, "return HandcraftLogic.new(player, nil, nil):getSourceResources()").unwrap_err();
    assert!(err.contains("HandcraftLogic.getSourceResources"), "{err}");
}

#[test]
fn listeners_get_their_target_first_and_only_on_a_change() {
    let _collection = pz_kahlua::gc::defer_collection();
    let mut t = thread_with_player();
    let out = run(
        &mut t,
        r#"
        local logic = HandcraftLogic.new(player, nil, nil)
        local seen = {}
        local panel = { name = "panel" }
        logic:addEventListener("onManualSelectChanged", function(self, on) seen[#seen + 1] = self.name .. "=" .. tostring(on) end, panel)
        logic:addEventListener("onShowManualSelectChanged", function(show) seen[#seen + 1] = "show=" .. tostring(show) end)
        logic:addEventListener("noSuchEvent", function() seen[#seen + 1] = "never" end)
        logic:setManualSelectInputs(true)
        logic:setManualSelectInputs(true)
        logic:setShowManualSelectInputs(true)
        logic:setManualSelectInputs(false)
        return table.concat(seen, ","), logic:shouldShowManualSelectInputs()
        "#,
    )
    .unwrap();
    assert_eq!(out, ["panel=true,show=true,panel=false", "false"]);
}

#[test]
fn an_empty_recipe_list_keeps_no_recipe_without_events() {
    let _collection = pz_kahlua::gc::defer_collection();
    let mut t = thread_with_player();
    t.globals().set_str("empty", Value::Object(Rc::new(JavaList::new(Vec::new()))));
    let out = run(
        &mut t,
        r#"
        local logic = HandcraftLogic.new(player, nil, nil)
        local seen = {}
        logic:addEventListener("onRecipeChanged", function() seen[#seen + 1] = "recipe" end)
        logic:addEventListener("onUpdateRecipeList", function() seen[#seen + 1] = "list" end)
        logic:setRecipes(empty)
        logic:setRecipe(nil)
        logic:checkValidRecipeSelected()
        return table.concat(seen, ","), tostring(logic:getRecipe()), logic:getRecipeList():isEmpty()
        "#,
    )
    .unwrap();
    assert_eq!(out, ["list", "nil", "true"]);
}
