use super::*;
use crate::{IsoChunk, IsoGridSquare};

struct NoRand;
impl GameRand for NoRand {
    fn next(&mut self, _: i32) -> i32 {
        0
    }
    fn next_float_range(&mut self, min: f32, _: f32) -> f32 {
        min
    }
}
#[derive(Default)]
struct Removed(Vec<FireRef>);
impl FireSoundHooks for Removed {}
impl FireHooks for Removed {
    fn fire_removed(&mut self, fire: FireRef) {
        self.0.push(fire);
    }
}

#[test]
fn evict_missing_or_wrong_kind_receiver_keeps_original_registry_sound_and_hook_cleanup() {
    for wrong_kind in [false, true] {
        let mut sprites = IsoSpriteManager::new(crate::fire::tests::tables());
        let flags = FireFlags::resolve(&sprites.tables).unwrap();
        let mut square = IsoGridSquare::new(2, 2, 0);
        square.properties.set_flag(flags.burning);
        square.properties.set_flag(flags.smoke);
        if wrong_kind {
            square
                .objects
                .push(ObjectSlot::new(IsoObject::new(1, ObjectKind::Object, None)));
        }
        let mut chunk = IsoChunk::new(0, 0);
        chunk.set_square(2, 2, 0, Some((square).into()));
        let mut cell = IsoCell::default();
        cell.chunks.insert((0, 0), chunk);
        let meta = IsoMetaGrid::blank(0, 0, 0, 0);
        let mut regions = IsoRegions::new(13);
        let doors = DoorTables {
            x_offset: [[[0; 4]; 2]; 2],
            y_offset: [[[0; 4]; 2]; 2],
        };
        let (mut world, mut pathfind, mut render) = (
            crate::square_mutation::NoMutationHooks,
            crate::square_mutation::NoMutationHooks,
            crate::square_mutation::NoMutationHooks,
        );
        let mut removed = Removed::default();
        let fire = FireRef {
            x: 2,
            y: 2,
            z: 0,
            id: 1,
        };
        let mut manager = IsoFireManager::default();
        manager.fire_stack.push(fire);
        manager.sounds.add_fire(fire);
        {
            let mut w = FireWorld {
                cell: &mut cell,
                meta: &meta,
                sprites: &mut sprites,
                mutation: FireMutation {
                    regions: &mut regions,
                    doors: &doors,
                    world: &mut world,
                    pathfind: &mut pathfind,
                    render: &mut render,
                    num_players: 1,
                },
                flags,
                settings: FireSettings::default(),
                hooks: &mut removed,
            };
            manager.evict(&mut w, &mut NoRand, fire);
        }
        assert!(manager.fire_stack.is_empty());
        assert!(manager.sounds.fires.is_empty());
        assert_eq!(removed.0, [fire]);
        let square_owner = cell.get_grid_square(2, 2, 0).unwrap();
        let square = square_owner.borrow();
        assert!(!square.properties.has(flags.burning));
        assert!(!square.properties.has(flags.smoke));
        assert_eq!(square.objects.len(), usize::from(wrong_kind));
    }
}
