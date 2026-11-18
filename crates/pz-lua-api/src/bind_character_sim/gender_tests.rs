use super::tests::{run, setup_with};
use super::*;

#[test]
fn gender_follows_the_descriptor_and_is_female_without_one() {
    let (mut thread, player, _) = setup_with(ScriptManager::new());
    run(&mut thread, r#"
        local desc = player:getDescriptor()
        desc:setFemale(false)
        assert(player:isFemale() == false)
        assert(player:getCharacterGender() == desc:getCharacterGender())
        desc:setFemale(true)
        assert(player:isFemale() == true)
        assert(player:getCharacterGender() == desc:getCharacterGender())
    "#).unwrap();
    player.borrow_mut().descriptor = None;
    run(&mut thread, r#"
        assert(player:isFemale() == true)
        assert(player:getCharacterGender() ~= nil)
    "#).unwrap();
}
