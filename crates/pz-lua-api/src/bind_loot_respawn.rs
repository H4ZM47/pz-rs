//! Build 42.22 Lua bindings and engine driver for `zombie.LootRespawn`.

use std::cell::RefCell;

use pz_kahlua::Thread;
use pz_world::loot_respawn::{LootRespawn, LootRespawnConfig};

use crate::bind_world::with_world;
use crate::expose::expose_class;

const LOOT_RESPAWN_CLASS: &str = "zombie.LootRespawn";

thread_local! {
    static STATE: RefCell<LootRespawn> = RefCell::new(LootRespawn::new());
}

/// Configure `LootRespawn` from sandbox/server options.
pub fn configure(cfg: LootRespawnConfig) {
    STATE.with(|s| s.borrow_mut().config = cfg);
}

/// `LootRespawn.Reset()`.
pub fn reset() {
    STATE.with(|s| s.borrow_mut().reset());
}

/// `LootRespawn.update()`: advances the respawn clock against `GameTime` and
/// stamps eligible loaded chunks whose respawn bucket elapsed.
pub fn update() -> usize {
    let world_age_hours = crate::bind_climate::shared_game_time()
        .borrow()
        .world_age_hours() as i32;
    let should_scan = STATE.with(|s| s.borrow_mut().check_update(world_age_hours));
    if !should_scan {
        return 0;
    }
    let cfg = STATE.with(|s| s.borrow().config);
    let helper = LootRespawn {
        last_respawn: world_age_hours,
        config: cfg,
    };
    with_world(|w| {
        let (Some(meta), Some(cell)) = (w.meta_grid.as_ref(), w.cell.as_mut()) else {
            return 0;
        };
        let mut updated = 0usize;
        for (&(wx, wy), chunk) in cell.chunks.iter_mut() {
            if LootRespawn::chunk_in_respawn_zone(meta, wx, wy)
                && helper.process_chunk(chunk, world_age_hours)
            {
                updated += 1;
            }
        }
        updated
    })
}

/// `LootRespawn.chunkLoaded(wx, wy)` for newly loaded chunks.
pub fn chunk_loaded(wx: i32, wy: i32) -> bool {
    let world_age_hours = crate::bind_climate::shared_game_time()
        .borrow()
        .world_age_hours() as i32;
    let cfg = STATE.with(|s| s.borrow().config);
    let helper = LootRespawn {
        last_respawn: world_age_hours,
        config: cfg,
    };
    with_world(|w| {
        let (Some(meta), Some(cell)) = (w.meta_grid.as_ref(), w.cell.as_mut()) else {
            return false;
        };
        if !LootRespawn::chunk_in_respawn_zone(meta, wx, wy) {
            return false;
        }
        let Some(chunk) = cell.chunks.get_mut(&(wx, wy)) else {
            return false;
        };
        helper.process_chunk(chunk, world_age_hours)
    })
}

/// Register `zombie.LootRespawn`.
pub fn register(thread: &mut Thread) {
    let lr = expose_class(thread, LOOT_RESPAWN_CLASS, Some("java.lang.Object"));
    lr.static_fn("update", |_| {
        update();
        Ok(0)
    });
    lr.static_fn("Reset", |_| {
        reset();
        Ok(0)
    });
    lr.static_fn("chunkLoaded", |_| Ok(0));
}

#[cfg(test)]
mod tests {
    use super::*;
    use pz_kahlua::TableRef;

    #[test]
    fn loot_respawn_lua_binding_round_trip() {
        reset();
        let mut t = Thread::new(TableRef::new());
        pz_kahlua::stdlib::install(&mut t, None).unwrap();
        register(&mut t);

        configure(LootRespawnConfig {
            hours_for_respawn: 24,
            ..Default::default()
        });
        let f = pz_kahlua::stdlib::load_string(
            r#"
            LootRespawn.Reset()
            LootRespawn.update()
            LootRespawn.chunkLoaded(nil)
            "#,
            Some("loot_respawn_test"),
            t.globals().clone(),
        )
        .unwrap();
        t.pcall(&f, &[])
            .unwrap_or_else(|e| panic!("lua error: {e:?}"));
        reset();
    }
}
