use super::tests::{run, setup_with};
use super::*;

#[test]
fn running_queries_share_actual_flags_moodle_gates_and_live_core_mode() {
    let sm = ScriptManager::new();
    let endurance = sm.registries.get("moodle_type").unwrap().borrow_mut()
        .register_id("test:endurance", false).unwrap();
    sm.statics.items.borrow_mut().constants.insert("moodle_type", "ENDURANCE", "test:endurance");
    let (mut thread, player, _) = setup_with(sm);
    struct RestoreCore(Rc<RefCell<pz_core::core::Core>>);
    impl Drop for RestoreCore {
        fn drop(&mut self) { crate::bind_core::install_core(self.0.clone()); }
    }
    let _restore = RestoreCore(crate::bind_core::shared_core());
    let core = Rc::new(RefCell::new(pz_core::core::Core::default()));
    crate::bind_core::install_core(core.clone());
    core.borrow_mut().game_mode = "Sandbox".into();
    player.borrow_mut().running = true;
    player.borrow_mut().environment.sprinting = true;
    player.borrow_mut().environment.allow_sprint = true;
    for (level, running, sprinting) in [(0, true, true), (2, true, false), (3, false, false)] {
        player.borrow_mut().moodles.moodle_mut(&endurance).unwrap().update_moodle_level(level);
        thread.globals().set_str("expectedRunning", running);
        thread.globals().set_str("expectedSprinting", sprinting);
        run(&mut thread, r#"
            assert(player:isRunning() == expectedRunning)
            assert(player:IsRunning() == expectedRunning)
            assert(player:isSprinting() == expectedSprinting)
            assert(player:canSprint() == expectedSprinting)
        "#).unwrap();
    }
    core.borrow_mut().game_mode = "Tutorial".into();
    run(&mut thread, "assert(player:canSprint()); assert(player:isSprinting())").unwrap();
    player.borrow_mut().environment.allow_sprint = false;
    run(&mut thread, "assert(not player:canSprint()); assert(not player:isSprinting())").unwrap();
    player.borrow_mut().environment.allow_sprint = true;
    core.borrow_mut().game_mode = "Sandbox".into();
    run(&mut thread, "assert(not player:canSprint()); assert(not player:isSprinting())").unwrap();
    player.borrow_mut().moodles.moodle_mut(&endurance).unwrap().update_moodle_level(0);
    player.borrow_mut().running = false;
    player.borrow_mut().environment.sprinting = false;
    run(&mut thread, r#"
        assert(not player:isRunning() and not player:IsRunning())
        assert(not player:isSprinting() and player:canSprint())
    "#).unwrap();
    let zombie = Rc::new(RefCell::new(LuaGameCharacter::new(&shared_script_manager(), CharacterKind::Zombie)));
    zombie.borrow_mut().environment.sprinting = true;
    zombie.borrow_mut().environment.allow_sprint = false;
    thread.globals().set_str("zombie", character_value(&zombie));
    run(&mut thread, r#"
        assert(zombie.IsRunning == nil)
        assert(zombie:canSprint() and zombie:isSprinting())
    "#).unwrap();
}
