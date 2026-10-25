//! `PathfindMutationHooks` and the `pathfind_square_hooks!` impl, against
//! a stand-in for `pz_world::square_mutation`'s trait.

mod common;

use common::Map;
use pz_pathfind::mutation_hooks::PathfindMutationHooks;
use pz_pathfind::square_bits::DefaultBitsHooks;

/// The trait as pz-world's `square_mutation` declares it.
mod world {
    pub use pz_world::chunk;
    pub mod square_mutation {
        #[allow(dead_code)]
        pub trait PathfindSquareHooks {
            fn uses_native_code(&self) -> bool {
                true
            }
            fn square_changed(&mut self, _cell: &super::chunk::IsoCell, _sq: (i32, i32, i32)) {}
            fn map_collision_square_changed(&mut self, _sq: &pz_world::IsoGridSquare) {}
            fn add_chunk_to_world(&mut self, _cell: &super::chunk::IsoCell, _wx: i32, _wy: i32) {}
        }
    }
}

pz_pathfind::pathfind_square_hooks!(crate::world);

use world::square_mutation::PathfindSquareHooks;

#[test]
fn square_edits_queue_square_and_chunk_updates() {
    let mut map = Map::new();
    let mut s = map.service();
    let mut hooks = PathfindSquareHooksImpl(PathfindMutationHooks {
        service: &mut s,
        sprites: &map.m,
        meta: &map.meta,
        bits_hooks: &DefaultBitsHooks,
    });
    // An inner square: its 8 neighbours, then itself.
    PathfindSquareHooks::square_changed(&mut hooks, &map.cell, (10, 10, 0));
    assert_eq!(hooks.0.service.queued_square_tasks().len(), 9);
    // A square the cell does not have queues nothing.
    PathfindSquareHooks::square_changed(&mut hooks, &map.cell, (10, 10, 3));
    assert_eq!(hooks.0.service.queued_square_tasks().len(), 9);
    PathfindSquareHooks::add_chunk_to_world(&mut hooks, &map.cell, 1, 1);
    assert_eq!(hooks.0.service.queued_chunk_tasks().len(), 1);
    assert_eq!(hooks.0.service.chunk_load_id(1, 1), Some(1));
    while s.has_thread_work() {
        s.update_thread();
    }
}
