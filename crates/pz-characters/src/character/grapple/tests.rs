use super::*;
use pz_scripts::ScriptManager;

fn character(kind: CharacterKind) -> GameCharacter<()> {
    GameCharacter::new(&ScriptManager::new(), kind)
}

#[test]
fn genuine_target_and_live_zombie_flag_derive_dragging() {
    let player = character(CharacterKind::Player);
    let zombie = character(CharacterKind::Zombie);
    assert!(!player.is_dragging_corpse());
    player.set_grappling_target(Some(&zombie), Some("authored"));
    assert!(player.is_grappling() && player.is_do_continue_grapple());
    assert!(
        !zombie.is_being_grappled(),
        "pure target assignment fabricated reciprocal acceptance"
    );
    assert!(player.is_grappling_target(Some(&zombie)));
    assert!(!player.is_dragging_corpse());
    zombie.zombie.set_reanimated_for_grapple_only(true);
    assert!(player.is_dragging_corpse());
    zombie.zombie.set_reanimated_for_grapple_only(false);
    assert!(!player.is_dragging_corpse());
    let survivor = character(CharacterKind::Survivor);
    survivor.zombie.set_reanimated_for_grapple_only(true);
    player.set_grappling_target(Some(&survivor), None);
    assert!(
        !player.is_dragging_corpse(),
        "non-zombie target passed real subclass check"
    );
    player.set_grappling_target(None, None);
    assert!(player.is_grappling() && player.is_grappling_target(None));
    assert!(!player.is_dragging_corpse());
}

#[test]
fn target_identity_survives_vec_moves_and_equal_character_ids() {
    let player = character(CharacterKind::Player);
    let mut zombies = vec![character(CharacterKind::Zombie)];
    zombies[0].zombie.set_reanimated_for_grapple_only(true);
    player.set_grappling_target(Some(&zombies[0]), Some("authored"));
    let endpoint = Rc::downgrade(&zombies[0].grapple);
    for _ in 0..128 {
        zombies.push(character(CharacterKind::Zombie));
    }
    assert_eq!(zombies[0].id, zombies[1].id);
    assert!(player.is_grappling_target(Some(&zombies[0])));
    assert!(!player.is_grappling_target(Some(&zombies[1])));
    let moved = zombies.remove(0);
    zombies.push(moved);
    assert!(Rc::ptr_eq(
        &endpoint.upgrade().unwrap(),
        &zombies.last().unwrap().grapple
    ));
    assert!(player.is_dragging_corpse());
    zombies
        .last()
        .unwrap()
        .zombie
        .set_reanimated_for_grapple_only(false);
    assert!(!player.is_dragging_corpse());
}

#[test]
fn zombie_field_clone_is_value_copy_while_endpoint_alias_is_live() {
    let zombie = character(CharacterKind::Zombie);
    zombie.zombie.set_reanimated_for_grapple_only(true);
    let copied = zombie.zombie.clone();
    copied.set_reanimated_for_grapple_only(false);
    assert!(zombie.zombie.is_reanimated_for_grapple_only());
    assert!(!copied.is_reanimated_for_grapple_only());
}

#[test]
fn body_damage_drag_transition_reads_actual_grapple_target() {
    let mut player = character(CharacterKind::Player);
    let zombie = character(CharacterKind::Zombie);
    player.set_grappling_target(Some(&zombie), Some("authored"));
    player.update_dragging_corpse();
    assert!(!player.body_damage.was_dragging_corpse);
    zombie.zombie.set_reanimated_for_grapple_only(true);
    player.update_dragging_corpse();
    assert!(player.body_damage.was_dragging_corpse && player.body_damage.started_dragging_corpse);
    player.update_dragging_corpse();
    assert!(player.body_damage.was_dragging_corpse && !player.body_damage.started_dragging_corpse);
    player.reset_grapple_state_to_default(Some("authored"));
    player.update_dragging_corpse();
    assert!(!player.body_damage.was_dragging_corpse && !player.body_damage.started_dragging_corpse);
}

#[test]
fn pure_reset_continue_and_type_keep_original_role_and_stale_link_semantics() {
    let a = character(CharacterKind::Player);
    let b = character(CharacterKind::Zombie);
    a.set_grappling_target(Some(&b), Some("old"));
    b.set_grappling_target(Some(&a), Some("old"));
    a.set_shared_grapple_type(Some("new"));
    assert_eq!(b.shared_grapple_type().as_deref(), Some("new"));
    a.set_do_continue_grapple(false);
    assert!(a.is_putting_down_body());
    a.set_do_grapple(true);
    // The accepted-side raw slot is authored fixture state; full Grappled
    // callbacks are not exposed by this bounded owner implementation.
    a.grapple.borrow_mut().grappled_by = Some(b.grapple.clone());
    a.grapple.borrow_mut().being_grappled = true;
    a.reset_grapple_state_to_default(Some("authored-reset"));
    assert!(!a.is_grappling() && !a.is_being_grappled() && !a.is_do_grapple());
    assert!(!a.is_putting_down_body() && !a.is_do_continue_grapple());
    assert_eq!(a.grapple_result().as_deref(), Some("authored-reset"));
    assert!(a.grapple.borrow().target.is_none());
    assert!(Rc::ptr_eq(
        a.grapple.borrow().grappled_by.as_ref().unwrap(),
        &b.grapple
    ));
}

#[test]
fn collector_preserves_live_endpoint_roots_and_reclaims_unreachable_cycles() {
    let a = character(CharacterKind::Zombie);
    let b = character(CharacterKind::Zombie);
    b.zombie.set_reanimated_for_grapple_only(true);
    a.set_grappling_target(Some(&b), Some("authored"));
    b.set_grappling_target(Some(&a), Some("authored"));
    let weak_a = Rc::downgrade(&a.grapple);
    let weak_b = Rc::downgrade(&b.grapple);
    assert!(matches!(
        pz_kahlua::gc::collect_tables(),
        pz_kahlua::gc::CollectionResult::Collected(_)
    ));
    assert!(a.is_dragging_corpse());
    drop(b);
    assert!(
        a.is_dragging_corpse(),
        "strong live target was weakened after value removal"
    );
    pz_kahlua::gc::collect_tables();
    assert!(weak_b.upgrade().is_some());
    drop(a);
    pz_kahlua::gc::collect_tables();
    assert!(weak_a.upgrade().is_none() && weak_b.upgrade().is_none());
    let self_target = character(CharacterKind::Zombie);
    self_target.set_grappling_target(Some(&self_target), None);
    let weak_self = Rc::downgrade(&self_target.grapple);
    drop(self_target);
    pz_kahlua::gc::collect_tables();
    assert!(weak_self.upgrade().is_none());
}
