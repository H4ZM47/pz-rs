use super::tests::{run, setup_with};
use super::*;

#[test]
fn see_designation_zone_is_a_player_flag() {
    let (mut thread, player, _) = setup_with(ScriptManager::new());
    run(&mut thread, r#"
        assert(player:isSeeDesignationZone() == false)
        player:setSeeDesignationZone(true)
        assert(player:isSeeDesignationZone() == true)
    "#).unwrap();
    assert!(player.borrow().player.see_designation_zone);
}
