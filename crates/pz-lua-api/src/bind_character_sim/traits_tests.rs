use super::tests::{run, setup_with};
use super::*;

#[test]
fn character_traits_is_the_characters_own_live_object() {
    let (mut thread, _player, _) = setup_with(ScriptManager::new());
    run(&mut thread, r#"
        local traits = player:getCharacterTraits()
        assert(traits == player:getCharacterTraits(), "one object")
        assert(traits:getKnownTraits():size() == 0)
        assert(not player:hasTrait(CharacterTrait.STRONG))
        traits:add(CharacterTrait.STRONG)
        assert(player:hasTrait(CharacterTrait.STRONG), "add reaches the character")
        assert(traits:getKnownTraits():size() == 1)
        assert(traits:getKnownTraits():get(0) == CharacterTrait.STRONG)
        traits:remove(CharacterTrait.STRONG)
        assert(not player:hasTrait(CharacterTrait.STRONG), "remove reaches the character")
    "#).unwrap();
}
