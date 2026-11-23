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
        assert(player:isBlockMovement())
        player:setIgnoreAutoVault(true)
        player:setAuthorizeMeleeAction(false)
        player:setBannedAttacking(true)
        player:setIgnoreInputsForDirection(true)
        assert(player:isIgnoreInputsForDirection())
        player:setAllowRun(false)
        assert(not player:isAllowRun())
        zombie:setAllowRun(true)
        assert(not zombie:isAllowRun())
        assert(zombie.setBlockMovement == nil and zombie.isBlockMovement == nil)
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

/// `Fishing.Handler.onEquipPrimary` asks `player:isLocal()` on every equip.
#[test]
fn every_character_is_local_in_single_player() {
    let (mut thread, _, _) = setup();
    run(&mut thread, "assert(player:isLocal() == true) assert(zombie:isLocal() == true)").unwrap();
}

/// `ISGrabCorpseAction.OnContextKey` asks `playerObj:isGrappling()` on
/// every press of the interact key.
#[test]
fn is_grappling_reads_the_grapple_state() {
    let (mut thread, player, zombie) = setup();
    run(&mut thread, "assert(player:isGrappling() == false)").unwrap();
    player.borrow().set_grappling_target(Some(&zombie.borrow()), Some("test"));
    run(&mut thread, "assert(player:isGrappling() == true)").unwrap();
}

/// `xpUpdate.onPlayerMove` rolls `player:isRandXp(1000, 0.05, perk)` on
/// every step; a perk rolls at most once per interval.
#[test]
fn is_rand_xp_rolls_once_per_interval_per_perk() {
    let (mut thread, _, _) = setup();
    run(&mut thread, r#"
        assert(player:isRandXp(100000, 1.0, Perks.Fitness) == true)
        assert(player:isRandXp(100000, 1.0, Perks.Fitness) == false)
        assert(player:isRandXp(100000, 1.0, Perks.Sprinting) == true)
    "#).unwrap();
}

/// A Lua world object that tells when it is dropped.
struct Probe;

impl pz_kahlua::LuaObject for Probe {
    fn class_name(&self) -> &str {
        "zombie.iso.IsoObject"
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

fn probe() -> (Value, std::rc::Weak<dyn pz_kahlua::LuaObject>) {
    let object: Rc<dyn pz_kahlua::LuaObject> = Rc::new(Probe);
    let weak = Rc::downgrade(&object);
    (Value::Object(object), weak)
}

fn unpublished_zombie() -> LuaGameCharacterRef {
    Rc::new(RefCell::new(LuaGameCharacter::new(
        &shared_script_manager(),
        CharacterKind::Zombie,
    )))
}

#[test]
fn object_fields_are_per_character() {
    let (mut thread, _player, _zombie) = setup();
    let (bed, _) = probe();
    thread.globals().set_str("bed", bed);
    run(&mut thread, r#"
        assert(zombie:getBed() == nil and zombie:getSitOnFurnitureObject() == nil)
        zombie:setBed(bed)
        zombie:setSitOnFurnitureObject(bed)
        assert(zombie:getBed() == bed and zombie:isSitOnFurnitureObject(bed))
        assert(player:getBed() == nil and not player:isSitOnFurnitureObject(bed))
        zombie:setBed(nil)
        assert(zombie:getBed() == nil and zombie:getSitOnFurnitureObject() == bed)
    "#).unwrap();
}

#[test]
fn the_session_reset_keeps_no_object_fields() {
    let _ = setup();
    let (bed, bed_weak) = probe();
    let chr = unpublished_zombie();
    super::set_object_field(&chr, super::ObjectField::Bed, bed.clone());
    super::set_object_field(&chr, super::ObjectField::SitOnFurniture, bed);
    drop(chr);
    crate::bind_character_sim::reset_session_handles();
    assert!(bed_weak.upgrade().is_none(), "the bed outlived its character and the reset");
}

#[test]
fn a_new_character_at_a_reused_address_has_no_object_fields() {
    let _ = setup();
    let (bed, _) = probe();
    let old = unpublished_zombie();
    super::set_object_field(&old, super::ObjectField::Bed, bed.clone());
    super::set_object_field(&old, super::ObjectField::SitOnFurniture, bed);
    // A new character in the same place, as when the allocator gives the
    // freed block to the next character.
    *old.borrow_mut() = LuaGameCharacter::new(&shared_script_manager(), CharacterKind::Zombie);
    let fresh = old;
    assert!(super::object_field(&fresh, super::ObjectField::Bed).is_nil());
    assert!(super::object_field(&fresh, super::ObjectField::SitOnFurniture).is_nil());
}
