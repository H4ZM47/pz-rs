use super::tests::{run, setup_with};
use super::*;

#[test]
fn outline_highlight_methods_follow_iso_object() {
    let (mut thread, player, _) = setup_with(ScriptManager::new());
    run(&mut thread, r#"
        assert(player:isOutlineHighlight() == false)
        assert(player:getOutlineHighlightCol(0) == -1)
        player:setOutlineHighlight(1, true)
        assert(player:isOutlineHighlight() == true)
        assert(player:isOutlineHighlight(1) == true)
        assert(player:isOutlineHighlight(0) == false)
        player:setOutlineHighlightCol(1, 1.0, 0.0, 0.0, 0.5)
        player:setOutlineHlBlink(true)
        player:setOutlineHlAttached(2, true)
        assert(player:isOutlineHlBlink(3) == true)
        assert(player:isOutlineHlAttached(2) and not player:isOutlineHlAttached(1))
        player:setOutlineHighlightCol(nil)
        player:unsetOutlineHighlight()
        assert(player:isOutlineHighlight() == false)
        assert(player:isOutlineHlBlink() == false and player:isOutlineHlAttached() == false)
        assert(not pcall(function() player:getOutlineHighlightCol(4) end))
    "#).unwrap();
    let o = player.borrow().outline;
    // ABGR 0x7F0000FF as a Java int keeps its colour after unset.
    assert_eq!(o.col[1], 0x7F00_00FF);
    assert_eq!(o.col[0], u32::MAX);
}
