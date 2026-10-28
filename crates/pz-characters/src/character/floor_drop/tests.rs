use std::rc::Rc;

use pz_inventory::floor_drop::{FloorWorld, SquarePos};
use pz_inventory::shared::ItemRef;
use pz_inventory::ItemClass;

use super::*;
use crate::character::tests::{fixture, give_item, item_constant, player, Recorder};
use crate::character::ZomboidGlobals;

/// A world with a solid floor everywhere at the character's level:
/// `Rand.Next(0.1f, 0.9f)` gives 0.5, the apparent z is the level, and
/// the items put down are recorded (square, id, x, y, height).
#[derive(Default)]
pub(crate) struct TestFloor {
    pub put: Vec<(SquarePos, i32, f32, f32, f32)>,
    pub fail_at: Option<usize>,
    pub attempts: usize,
}

impl FloorWorld for TestFloor {
    fn solid_floor_ref_at(
        &self,
        x: i32,
        y: i32,
        z: i32,
    ) -> pz_inventory::floor_drop::HostResult<Option<pz_inventory::floor_drop::FloorSquare>> {
        self.solid_floor_at(x, y, z)
            .map(|sq| sq.map(pz_inventory::floor_drop::FloorSquare::data_projection))
    }
    fn apparent_z_on(
        &self,
        sq: &pz_inventory::floor_drop::FloorSquare,
        x: f32,
        y: f32,
    ) -> pz_inventory::floor_drop::HostResult<f32> {
        assert!(
            sq.owner_slot().is_none(),
            "explicit coordinate-only test profile"
        );
        self.apparent_z(sq.position(), x, y)
    }
    fn add_world_inventory_item_on(
        &mut self,
        sq: &pz_inventory::floor_drop::FloorSquare,
        item: &pz_inventory::shared::ItemRef,
        x: f32,
        y: f32,
        h: f32,
    ) -> pz_inventory::floor_drop::HostResult<()> {
        assert!(
            sq.owner_slot().is_none(),
            "explicit coordinate-only test profile"
        );
        self.add_world_inventory_item(sq.position(), item, x, y, h)
    }

    fn solid_floor_at(&self, x: i32, y: i32, z: i32) -> HostResult<Option<SquarePos>> {
        Ok(Some((x, y, z)))
    }
    fn apparent_z(&self, sq: SquarePos, _dx: f32, _dy: f32) -> HostResult<f32> {
        Ok(sq.2 as f32)
    }
    fn rand_float(&mut self, _min: f32, _max: f32) -> HostResult<f32> {
        Ok(0.5)
    }
    fn add_world_inventory_item(
        &mut self,
        sq: SquarePos,
        item: &ItemRef,
        x: f32,
        y: f32,
        height: f32,
    ) -> HostResult<()> {
        self.attempts += 1;
        if self.fail_at == Some(self.attempts) {
            return Err(RuntimeHostError::Borrowed("actual floor allocation"));
        }
        self.put.push((sq, item.borrow().id, x, y, height));
        Ok(())
    }
}

fn ctx() -> Recorder {
    let globals = ZomboidGlobals { equipped_or_worn_encumbrance_multiplier: 0.5, ..Default::default() };
    Recorder { globals, ..Default::default() }
}

/// A player on (10, 20, 0) with an inventory of capacity 5 holding two
/// invented jackets (`Coat`, 4; `Vest`, 1) and a rock (2), the coat worn
/// at the only location.
fn dressed(
    sm: &ScriptManager,
    chars: &mut crate::Characters,
) -> (GameCharacter<()>, Rc<pz_scripts::registry::RegistryKey>) {
    let jacket = item_constant(sm, "item_body_location", "JACKET");
    let mut c = player(sm, chars);
    c.path.current_square = Some((10, 20, 0));
    let g = chars.body_locations.get_group("Human").unwrap();
    g.borrow_mut().get_or_create_location(Some(jacket.clone())).unwrap();
    c.init_worn_items(sm, g);
    give_item(
        sm,
        &mut c,
        ItemClass::Clothing,
        "Coat",
        1,
        |t| t.actual_weight = 4.0,
        |_| {},
    );
    give_item(
        sm,
        &mut c,
        ItemClass::Clothing,
        "Vest",
        2,
        |t| t.actual_weight = 1.0,
        |_| {},
    );
    give_item(
        sm,
        &mut c,
        ItemClass::Normal,
        "Rock",
        3,
        |t| t.actual_weight = 2.0,
        |_| {},
    );
    c.items.inventory.borrow_mut().capacity = 5;
    (c, jacket)
}

#[test]
fn a_clothing_swap_over_capacity_drops_the_old_item() {
    let (sm, mut chars) = fixture();
    let ctx = ctx();
    let mut floor = TestFloor::default();
    let (mut c, jacket) = dressed(&sm, &mut chars);
    // Nothing was worn there: nothing to drop.
    assert!(c.set_worn_item(&sm, None, Some(jacket.clone()), Some(1), true, Some(FloorCx::new(&ctx, &mut floor))).unwrap());
    assert!(floor.put.is_empty());
    // Coat 4 * 0.5 + vest 1 + rock 2 = 5: room. The swap leaves the coat
    // unequipped (4 + 0.5 + 2 = 6.5), and 6.5 + 4 > 5: it drops.
    assert_eq!(c.capacity_weight(&sm, &ctx), 5.0);
    assert!(c.set_worn_item(&sm, None, Some(jacket.clone()), Some(2), true, Some(FloorCx::new(&ctx, &mut floor))).unwrap());
    assert_eq!(floor.put, [((10, 20, 0), 1, 0.5, 0.5, 0.0)]);
    assert!(c.items.item(1).is_none());
    assert_eq!(c.items.worn_ids(), [2]);

    // removeWornItem goes the same way; without forceDropTooHeavy, or
    // with room, the item stays.
    c.items.inventory.borrow_mut().capacity = 2;
    assert!(c.remove_worn_item(&sm, None, Some(2), false, Some(FloorCx::new(&ctx, &mut floor))).unwrap());
    assert_eq!(floor.put.len(), 1);
    c.set_worn_item(&sm, None, Some(jacket.clone()), Some(2), false, None).unwrap();
    c.items.inventory.borrow_mut().capacity = 50;
    assert!(c.remove_worn_item(&sm, None, Some(2), true, Some(FloorCx::new(&ctx, &mut floor))).unwrap());
    assert_eq!(floor.put.len(), 1);
    assert!(c.items.item(2).is_some());
    c.set_worn_item(&sm, None, Some(jacket.clone()), Some(2), false, None).unwrap();
    c.items.inventory.borrow_mut().capacity = 2;
    assert!(c.remove_worn_item(&sm, None, Some(2), true, Some(FloorCx::new(&ctx, &mut floor))).unwrap());
    assert_eq!(floor.put.last().map(|p| p.1), Some(2));
    assert!(c.items.item(2).is_none());
}

#[test]
fn only_players_drop_and_a_client_does_not() {
    let (sm, mut chars) = fixture();
    let mut floor = TestFloor::default();
    let (mut c, jacket) = dressed(&sm, &mut chars);
    let ctx = ctx();
    c.set_worn_item(&sm, None, Some(jacket.clone()), Some(1), true, None).unwrap();
    // Not an IsoPlayer: no drop.
    c.kind = crate::CharacterKind::Survivor;
    c.set_worn_item(
        &sm,
        None,
        Some(jacket.clone()),
        Some(2),
        true,
        Some(FloorCx::new(&ctx, &mut floor)),
    )
    .unwrap();
    assert!(floor.put.is_empty() && c.items.item(1).is_some());

    struct Client(Recorder);
    impl SimContext for Client {
        fn zomboid_globals(&self) -> &ZomboidGlobals {
            self.0.zomboid_globals()
        }
        fn is_client(&self) -> bool {
            true
        }
    }
    let client = Client(ctx);
    c.kind = crate::CharacterKind::Player;
    c.set_worn_item(
        &sm,
        None,
        Some(jacket.clone()),
        Some(1),
        true,
        Some(FloorCx::new(&client, &mut floor)),
    )
    .unwrap();
    assert!(floor.put.is_empty() && c.items.item(2).is_some());
}

#[test]
fn heavy_and_hand_items_drop() {
    let (sm, mut chars) = fixture();
    let ctx = ctx();
    let mut floor = TestFloor::default();
    let mut c = player(&sm, &mut chars);
    c.path.current_square = Some((3, 4, 1));
    give_item(
        &sm,
        &mut c,
        ItemClass::Container,
        "Duffel",
        1,
        |t| t.actual_weight = 1.0,
        |s| s.drop_sound = Some("DropDuffel".into()),
    );
    give_item(&sm, &mut c, ItemClass::Normal, "Spoon", 2, |_| {}, |_| {});
    c.set_primary_hand_item(&sm, Some(1));
    c.set_secondary_hand_item(&sm, Some(2));
    // dropHeavyItems: only the bag, with its events and drop sound.
    let d = c.drop_heavy_items(&sm, FloorCx::new(&ctx, &mut floor)).unwrap();
    assert_eq!(d.len(), 1);
    assert_eq!(floor.put, [((3, 4, 1), 1, 0.5, 0.5, 0.0)]);
    assert!(c.items.primary_hand_item.is_none() && c.items.item(1).is_none());
    assert_eq!(c.items.secondary_hand_item, Some(2));
    let names: Vec<String> = ctx.events.borrow().iter().map(|e| e.split([' ', '(']).next().unwrap().to_string()).collect();
    assert_eq!(names, ["ContainerUpdate", "ItemFall", "PlaySound"]);
    assert!(ctx.events.borrow()[2].contains("DropDuffel"));
    // dropHandItems: the rest of the hands too, except in the tutorial.
    struct Tutorial;
    impl SimContext for Tutorial {
        fn game_mode(&self) -> &str {
            "Tutorial"
        }
    }
    assert!(c.drop_hand_items(&sm, FloorCx::new(&Tutorial, &mut floor)).unwrap().is_empty());
    assert_eq!(c.drop_hand_items(&sm, FloorCx::new(&ctx, &mut floor)).unwrap().len(), 1);
    assert_eq!(floor.put.last().map(|p| p.1), Some(2));
    assert!(c.items.secondary_hand_item.is_none() && c.items.inventory_items().is_empty());
}

#[test]
fn worn_swap_reports_typed_host_fault_after_worn_and_removal_prefix_before_later_changes() {
    let (sm, mut chars) = fixture();
    let ctx = ctx();
    let (mut c, jacket) = dressed(&sm, &mut chars);
    c.set_worn_item(&sm, None, Some(jacket.clone()), Some(1), false, None).unwrap();
    ctx.events.borrow_mut().clear();
    let mut floor = TestFloor {
        fail_at: Some(1),
        ..Default::default()
    };
    let result = c.set_worn_item(
        &sm,
        None,
        Some(jacket),
        Some(2),
        true,
        Some(FloorCx::new(&ctx, &mut floor)),
    );
    assert_eq!(
        result,
        Err(CharacterActionError::Host(RuntimeHostError::Borrowed(
            "actual floor allocation"
        )))
    );
    assert_eq!(
        c.items.worn_ids(),
        [2],
        "completed native worn prefix is retained"
    );
    assert!(
        c.items.item(1).is_none(),
        "completed inventory removal prefix is retained"
    );
    assert_eq!(floor.attempts, 1);
    assert!(floor.put.is_empty());
    assert!(ctx.events.borrow().is_empty(), "no later clothing callback or drop sound");
}
