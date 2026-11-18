use super::tests::{run, setup_with};
use super::*;

#[test]
fn get_players_copies_all_four_slots_including_empty_ones() {
    let (mut thread, player, _) = setup_with(ScriptManager::new());
    crate::bind_players::reset();
    run(&mut thread, r#"
        local players = IsoPlayer.getPlayers()
        assert(players:size() == 4)
        for i = 0, 3 do assert(players:get(i) == nil) end
    "#).unwrap();
    crate::bind_players::set_player(2, Some(player.clone()));
    run(&mut thread, r#"
        local players = IsoPlayer.getPlayers()
        assert(players:size() == 4)
        assert(players:get(0) == nil and players:get(1) == nil and players:get(3) == nil)
        assert(players:get(2):isLocalPlayer() ~= nil)
        -- A copy: changing it leaves the slots alone.
        players:clear()
        assert(IsoPlayer.getPlayers():size() == 4)
    "#).unwrap();
    crate::bind_players::reset();
}
