use super::tests::{run, setup_with};
use super::*;

#[test]
fn safety_is_the_characters_own_state() {
    let (mut thread, player, _) = setup_with(ScriptManager::new());
    run(&mut thread, r#"
        local s = player:getSafety()
        assert(s == player:getSafety(), "one object")
        assert(s:isEnabled() == true and s:isLast() == true)
        assert(s:getCooldown() == 0 and s:getToggle() == 0)
        assert(s:getCharacter() == player)
        assert(s:getDescription() == "enabled=true last=true cooldown=0.0 toggle=0.0", s:getDescription())
        s:setEnabled(false)
        s:setCooldown(2.5)
        s:setToggle(1)
        assert(s:getDescription() == "enabled=false last=true cooldown=2.5 toggle=1.0", s:getDescription())
    "#).unwrap();
    let safety = player.borrow().safety;
    assert!(!safety.enabled && safety.last);
    assert_eq!((safety.cooldown, safety.toggle), (2.5, 1.0));
}
