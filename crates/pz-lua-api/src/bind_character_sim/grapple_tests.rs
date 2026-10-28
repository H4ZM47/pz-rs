use super::tests::{run, setup_with};
use super::*;

#[test]
fn dragging_corpse_query_reads_the_actual_target_and_live_zombie_setter() {
    let (mut thread, player, _) = setup_with(ScriptManager::new());
    crate::bind_zombie::register(&mut thread);
    let zombie = Rc::new(RefCell::new(LuaGameCharacter::new(
        &shared_script_manager(),
        CharacterKind::Zombie,
    )));
    let other = Rc::new(RefCell::new(LuaGameCharacter::new(
        &shared_script_manager(),
        CharacterKind::Zombie,
    )));
    thread.globals().set_str("target", character_value(&zombie));
    thread.globals().set_str("other", character_value(&other));
    run(&mut thread, "assert(not player:isDraggingCorpse())").unwrap();
    player
        .borrow()
        .set_grappling_target(Some(&zombie.borrow()), Some("authored"));
    run(
        &mut thread,
        r#"
        assert(not player:isDraggingCorpse())
        other:setReanimatedForGrappleOnly(true)
        assert(not player:isDraggingCorpse())
        target:setReanimatedForGrappleOnly(true)
        assert(target:isReanimatedForGrappleOnly() and player:isDraggingCorpse())
        target:setReanimatedForGrappleOnly(false)
        assert(not target:isReanimatedForGrappleOnly() and not player:isDraggingCorpse())
    "#,
    )
    .unwrap();
    zombie.borrow().zombie.set_reanimated_for_grapple_only(true);
    run(&mut thread, "assert(player:isDraggingCorpse())").unwrap();
    player
        .borrow()
        .reset_grapple_state_to_default(Some("authored"));
    run(&mut thread, "assert(not player:isDraggingCorpse())").unwrap();
}
