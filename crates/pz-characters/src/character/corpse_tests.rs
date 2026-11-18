//! `new IsoDeadBody(zombie)` with an invented zombie (the ids, items and
//! positions are made up).

use pz_inventory::ItemClass;

use crate::character::state::{NullWorld, StateCx};
use crate::character::tests::{give_item, item_constant, Recorder};
use crate::character::thermoregulator::body_tests::body_fixture;
use crate::character::{CharacterKind, GameCharacter};

/// A killed zombie with an apple (id 3) and a worn shirt (id 4).
fn killed_zombie() -> (GameCharacter<()>, std::rc::Rc<pz_scripts::registry::RegistryKey>) {
    let (sm, mut chars) = body_fixture();
    let shirt = item_constant(&sm, "item_body_location", "SHIRT");
    let group = chars.body_locations.get_group("Human").unwrap();
    group.borrow_mut().get_or_create_location(Some(shirt.clone())).unwrap();
    let mut z: GameCharacter<()> = GameCharacter::new(&sm, CharacterKind::Zombie);
    (z.vars.x, z.vars.y, z.vars.z) = (12.25, 7.5, 0.0);
    z.vars.dir = pz_world::square::IsoDirections::S;
    z.set_forward_direction(0.0, 1.0);
    z.zombie.persistent_outfit_id = 77;
    z.vars.crawling = true;
    z.init_worn_items(&sm, group);
    give_item(&sm, &mut z, ItemClass::Normal, "Apple", 3, |_| {}, |_| {});
    give_item(&sm, &mut z, ItemClass::Clothing, "Shirt", 4, |_| {}, |_| {});
    z.items.worn_items.as_mut().unwrap().set_item(Some(shirt.clone()), Some(4)).unwrap();
    z.kill_by(None, &mut StateCx::new(&sm, &chars, &Recorder::default(), &mut NullWorld));
    (z, shirt)
}

/// The zombie still has the apple and wears the shirt.
fn assert_keeps_its_items(z: &GameCharacter<()>) {
    let ids: Vec<_> = z.items.inventory.borrow().items.iter().map(|i| i.borrow().id).collect();
    assert_eq!(ids, vec![3, 4]);
    assert_eq!(z.items.worn_items.as_ref().unwrap().items().len(), 1);
}

#[test]
fn zombie_corpse_takes_the_inventory_and_worn_items() {
    let (mut z, shirt) = killed_zombie();
    let mut placed = None;

    assert!(z.become_dead_body(5.5, |b| {
        placed = Some(b);
        true
    }).unwrap());

    let body = placed.unwrap();

    assert_eq!((body.base.x, body.base.y, body.base.z), (12.25, 7.5, 0.0));
    assert_eq!(body.base.dir, pz_world::square::IsoDirections::S as i32);
    assert_eq!(body.angle, z.anim_angle_radians());
    assert!(body.was_zombie && body.female);
    // Crawling, not reanimated: on its front.
    assert!(body.crawling && body.fall_on_front);
    assert_eq!(body.persistent_outfit_id, 77);
    assert_eq!(body.death_time, 5.5);
    assert_eq!(body.reanimate_time, -1.0);
    assert_eq!(body.object_id.1, 4);
    let container = body.container.as_ref().unwrap();
    assert!(container.explored);
    assert_eq!(container.items.iter().map(|i| i.id()).collect::<Vec<_>>(), vec![3, 4]);
    assert_eq!(body.worn_items, vec![(shirt.location.id(), 1)]);
    // The zombie is left a new empty inventory and no worn items.
    assert!(z.items.inventory.borrow().items.is_empty());
    assert!(z.items.inventory.borrow().explored);
    assert_eq!(z.items.worn_items.as_ref().unwrap().items().len(), 0);
}

#[test]
fn animal_corpse_is_unsupported() {
    let (sm, _chars) = body_fixture();
    let mut a: GameCharacter<()> = GameCharacter::new(&sm, CharacterKind::Animal);
    assert!(a.become_dead_body(0.0, |_| true).unwrap_err().to_string().contains("unsupported"));
}

#[test]
fn unplaced_corpse_leaves_the_zombie_its_items() {
    let (mut z, _) = killed_zombie();

    assert!(!z.become_dead_body(5.5, |_| false).unwrap());

    assert_keeps_its_items(&z);
}

#[test]
fn failed_corpse_leaves_the_zombie_its_items() {
    let (mut z, _) = killed_zombie();
    z.items.primary_hand_item = Some(3);

    let result = z.become_dead_body(5.5, |_| panic!("no corpse to place"));

    assert!(result.unwrap_err().to_string().contains("dropHeldItems"));
    assert_keeps_its_items(&z);
}
