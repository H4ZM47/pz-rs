use super::tests::{run, setup_with};
use super::*;
use pz_world::IsoDirections;

#[test]
fn direction_queries_return_live_inherited_field_and_canonical_enum() {
    let (mut thread, player, _) = setup_with(ScriptManager::new());
    crate::expose::expose_enum(
        &mut thread,
        crate::bind_iso_enums::ISO_DIRECTIONS,
        &["N", "NW", "W", "SW", "S", "SE", "E", "NE"],
    );
    for direction in IsoDirections::ALL {
        // The getter reads dir rather than recomputing an angle from the
        // separate forward vector or retaining the registration-time value.
        player.borrow_mut().vars.dir = direction;
        thread.globals().set_str("expectedDirection", format!("{direction:?}"));
        run(&mut thread, r#"
            local expected = IsoDirections[expectedDirection]
            assert(player:getDir() == expected)
            assert(player:getForwardIsoDirection() == expected)
        "#).unwrap();
    }
    let zombie = Rc::new(RefCell::new(LuaGameCharacter::new(
        &shared_script_manager(), CharacterKind::Zombie,
    )));
    zombie.borrow_mut().vars.dir = IsoDirections::SW;
    thread.globals().set_str("zombie", character_value(&zombie));
    run(&mut thread, r#"
        assert(zombie:getDir() == IsoDirections.SW)
        assert(zombie:getForwardIsoDirection() == IsoDirections.SW)
        assert(player:getDir() == IsoDirections.NE)
    "#).unwrap();
}
