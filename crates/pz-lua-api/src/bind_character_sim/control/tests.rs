use super::super::tests::{run, setup_with};
use super::super::*;
use pz_world::IsoDirections;

fn setup() -> (Thread, LuaGameCharacterRef, LuaGameCharacterRef) {
    let (mut thread, player, _) = setup_with(ScriptManager::new());
    crate::expose::expose_enum(
        &mut thread,
        crate::bind_iso_enums::ISO_DIRECTIONS,
        &["N", "NW", "W", "SW", "S", "SE", "E", "NE"],
    );
    let zombie = Rc::new(RefCell::new(LuaGameCharacter::new(
        &shared_script_manager(),
        CharacterKind::Zombie,
    )));
    thread.globals().set_str("zombie", character_value(&zombie));
    (thread, player, zombie)
}

#[test]
fn facing_turns_the_forward_vector_and_dir() {
    let (mut thread, player, _) = setup();
    {
        let mut p = player.borrow_mut();
        p.vars.x = 10.5;
        p.vars.y = 20.5;
    }
    run(&mut thread, r#"
        assert(player:faceLocation(10, 18) == true)
        assert(player:getDir() == IsoDirections.N)
        assert(player:faceLocationF(10.5, 20.5) == false)
        assert(player:getDir() == IsoDirections.N)
        player:setDir(IsoDirections.E)
        assert(player:getDir() == IsoDirections.E)
        player:setDir(12)
        assert(player:getDir() == IsoDirections.S)
        player:faceDirection(IsoDirections.W)
    "#).unwrap();
    let p = player.borrow();
    assert_eq!(p.vars.dir, IsoDirections::W);
    assert!((p.vars.forward_x + 1.0).abs() < 1e-6 && p.vars.forward_y.abs() < 1e-6);
}

#[test]
fn distances_follow_iso_moving_object() {
    let (mut thread, player, zombie) = setup();
    {
        let mut p = player.borrow_mut();
        p.vars.x = 1.0;
        p.vars.y = 2.0;
        let mut z = zombie.borrow_mut();
        z.vars.x = 4.0;
        z.vars.y = 6.0;
    }
    run(&mut thread, r#"
        assert(player:DistTo(zombie) == 7)
        assert(player:DistTo(nil) == 0)
        assert(player:DistTo(0, 0) == 3)
        assert(player:DistToSquared(zombie) == 25)
        assert(player:DistToSquared(1, 0) == 4)
        assert(player:DistToProper(zombie) == 5)
        assert(player:isAlive())
    "#).unwrap();
}

#[test]
fn player_flags_and_input_component() {
    let (mut thread, player, zombie) = setup();
    run(&mut thread, r#"
        player:setIgnoreMovement(true)
        assert(player:getIgnoreMovement())
        player:setIgnoreContextKey(true)
        assert(player:isIgnoreContextKey())
        player:setBlockMovement(true)
        player:setIgnoreAutoVault(true)
        player:setAuthorizeMeleeAction(false)
        player:setBannedAttacking(true)
        player:setIgnoreInputsForDirection(true)
        assert(player:isIgnoreInputsForDirection())
        player:setAllowRun(false)
        assert(not player:isAllowRun())
        zombie:setAllowRun(true)
        assert(not zombie:isAllowRun())
        assert(zombie.setBlockMovement == nil)
        assert(not zombie:isPlayerMoving())
        assert(player:getTorchStrength() == 0)
        assert(not player:isTimedActionInstant())
        assert(not player:isClimbingRope())
        assert(getLoosingXpTick(25000) == 10)
    "#).unwrap();
    let p = player.borrow();
    assert!(p.vars.ignore_movement && p.vars.block_movement && p.vars.ignore_auto_vault && p.vars.banned_attacking);
    assert!(!p.player.authorize_hand_to_hand_action);
    assert!(p.player.input.ignore_inputs_for_direction && !p.player.input.allow_run);
    assert!(zombie.borrow().player.input.allow_run);
}
