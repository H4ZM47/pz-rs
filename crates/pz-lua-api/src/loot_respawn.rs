//! `zombie.LootRespawn`: periodic container loot refilling across loaded
//! chunks based on sandbox `LootRespawn`, `SeenHoursPreventLootRespawn`,
//! `MaxItemsForLootRespawn`, and safehouse exclusion.

use crate::expose::expose_class;
use pz_inventory::shared::ContainerRef;
use pz_kahlua::{Thread, Value};
use std::cell::RefCell;
use std::collections::HashMap;

pub const LOOT_RESPAWN_CLASS: &str = "zombie.LootRespawn";

/// Maps sandbox `LootRespawn` enum (1..=5) to respawn interval hours
/// (`LootRespawn.getRespawnInterval()`):
/// 1 = None (`-1`), 2 = Every Day (`24`), 3 = Every Week (`168`),
/// 4 = Every Month (`720`), 5 = Every 2 Months (`1440`).
pub fn loot_respawn_interval_hours(option: i32) -> i32 {
    match option {
        2 => 24,
        3 => 168,
        4 => 720,
        5 => 1440,
        _ => -1,
    }
}

/// Live clock and sandbox parameters for one [`update`] tick.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LootRespawnConfig {
    /// Respawn interval in game hours (`<= 0` disables loot respawn).
    pub respawn_hours: i32,
    /// Sandbox `SeenHoursPreventLootRespawn` (`<= 0` disables seen guard).
    pub seen_hours_prevent: i32,
    /// Sandbox `MaxItemsForLootRespawn` (`<= 0` disables item cap check).
    pub max_items: i32,
    /// `GameTime.getInstance().getWorldAgeHours()`.
    pub world_age_hours: f64,
}

impl Default for LootRespawnConfig {
    fn default() -> Self {
        Self {
            respawn_hours: -1,
            seen_hours_prevent: 0,
            max_items: 4,
            world_age_hours: 0.0,
        }
    }
}

#[derive(Default)]
struct LootRespawnState {
    last_respawn_hour: i32,
    initialized: bool,
    /// Last `world_age_hours` when any active player was in or adjacent to chunk `(wx, wy)`.
    chunk_seen_hour: HashMap<(i32, i32), f64>,
}

thread_local! {
    static STATE: RefCell<LootRespawnState> = RefCell::new(LootRespawnState::default());
}

/// `LootRespawn.Reset()`.
pub fn reset() {
    STATE.with(|s| {
        let mut st = s.borrow_mut();
        st.last_respawn_hour = -1;
        st.initialized = false;
        st.chunk_seen_hour.clear();
    });
}

/// Marks chunk `(wx, wy)` as seen by a player at `world_age_hours`.
pub fn record_player_seen(wx: i32, wy: i32, world_age_hours: f64) {
    STATE.with(|s| {
        s.borrow_mut()
            .chunk_seen_hour
            .insert((wx, wy), world_age_hours);
    });
}

fn recently_seen(wx: i32, wy: i32, cfg: &LootRespawnConfig) -> bool {
    if cfg.seen_hours_prevent <= 0 {
        return false;
    }
    STATE.with(|s| {
        s.borrow()
            .chunk_seen_hour
            .get(&(wx, wy))
            .is_some_and(|&seen| (cfg.world_age_hours - seen) < cfg.seen_hours_prevent as f64)
    })
}

/// Attempts to respawn eligible containers in chunk `(wx, wy)` for the
/// given `respawn_hour` boundary. Returns the number of containers refilled.
pub fn respawn_in_chunk(
    thread: &mut Thread,
    wx: i32,
    wy: i32,
    respawn_hour: i32,
    cfg: &LootRespawnConfig,
) -> usize {
    if cfg.respawn_hours <= 0 || recently_seen(wx, wy, cfg) {
        return 0;
    }
    let containers: Vec<ContainerRef> = crate::bind_world::with_world(|w| {
        let Some(cell) = w.cell.as_mut() else {
            return Vec::new();
        };
        let Some(chunk) = cell.chunks.get_mut(&(wx, wy)) else {
            return Vec::new();
        };
        if chunk.saved.loot_respawn_hour == -1 {
            chunk.saved.loot_respawn_hour = respawn_hour;
            return Vec::new();
        }
        if chunk.saved.loot_respawn_hour >= respawn_hour {
            return Vec::new();
        }
        chunk.saved.loot_respawn_hour = respawn_hour;

        let mut out = Vec::new();
        for sq_ref in chunk.squares() {
            let Ok(sq) = sq_ref.try_borrow() else {
                continue;
            };
            if crate::bind_safehouse::is_in_safehouse(sq.x, sq.y) {
                continue;
            }
            for obj_ref in &sq.objects {
                let Ok(obj) = obj_ref.try_borrow() else {
                    continue;
                };
                let Some(rc) = &obj.runtime_containers else {
                    continue;
                };
                for c in rc.iter() {
                    let Ok(cb) = c.try_borrow() else {
                        continue;
                    };
                    if !cb.explored {
                        continue;
                    }
                    if cfg.max_items > 0 && cb.items.len() >= cfg.max_items as usize {
                        continue;
                    }
                    out.push(c.clone());
                }
            }
        }
        out
    });

    if containers.is_empty() {
        return 0;
    }

    let dist = crate::item_picker::shared();
    let mut refilled = 0;
    for c in containers {
        if let Ok(mut cb) = c.try_borrow_mut() {
            cb.has_been_looted = false;
        }
        if let Err(e) = crate::bind_item_picker::fill_container(thread, &dist, &c, &Value::Nil) {
            eprintln!("ERROR: LootRespawn.fill_container: {e}");
        } else {
            refilled += 1;
        }
    }
    refilled
}

/// `LootRespawn.update()`: advances the respawn hour slot and refills
/// eligible containers across all loaded chunks when a new boundary passes.
pub fn update(thread: &mut Thread, cfg: &LootRespawnConfig) -> usize {
    if cfg.respawn_hours <= 0 {
        return 0;
    }
    let cur_hour = cfg.world_age_hours.floor() as i32;
    let respawn_hour = cur_hour - cur_hour.rem_euclid(cfg.respawn_hours);

    let should_run = STATE.with(|s| {
        let mut st = s.borrow_mut();
        if !st.initialized {
            st.initialized = true;
            st.last_respawn_hour = respawn_hour;
            return false;
        }
        if respawn_hour <= st.last_respawn_hour {
            return false;
        }
        st.last_respawn_hour = respawn_hour;
        true
    });
    if !should_run {
        return 0;
    }

    let chunk_coords: Vec<(i32, i32)> = crate::bind_world::with_world(|w| {
        w.cell
            .as_ref()
            .map(|cell| cell.chunks.keys().copied().collect())
            .unwrap_or_default()
    });
    let mut total = 0;
    for (wx, wy) in chunk_coords {
        total += respawn_in_chunk(thread, wx, wy, respawn_hour, cfg);
    }
    total
}

/// Engine convenience entry: reads live sandbox options and `GameTime`,
/// records active players' current chunks as seen, and runs [`update`].
pub fn update_from_sandbox(thread: &mut Thread) -> usize {
    let world_age_hours = crate::bind_climate::shared_game_time()
        .borrow()
        .world_age_hours();
    let cfg = match crate::bind_sandbox::instance(thread) {
        Ok(Value::Object(o)) => o
            .as_any()
            .downcast_ref::<crate::bind_sandbox::SandboxObj>()
            .map(|sb| {
                let sb = sb.0.borrow();
                let respawn_opt = sb
                    .option_by_name("LootRespawn")
                    .map_or(1, |o| o.config.get_int());
                let seen_hours = sb
                    .option_by_name("SeenHoursPreventLootRespawn")
                    .map_or(0, |o| o.config.get_int());
                let max_items = sb
                    .option_by_name("MaxItemsForLootRespawn")
                    .map_or(4, |o| o.config.get_int());
                LootRespawnConfig {
                    respawn_hours: loot_respawn_interval_hours(respawn_opt),
                    seen_hours_prevent: seen_hours,
                    max_items,
                    world_age_hours,
                }
            })
            .unwrap_or_default(),
        _ => LootRespawnConfig::default(),
    };
    if cfg.respawn_hours <= 0 {
        return 0;
    }
    if cfg.seen_hours_prevent > 0 {
        for i in 0..crate::bind_players::MAX_PLAYERS {
            if let Some(p) = crate::bind_players::player(i) {
                let (wx, wy) = {
                    let p = p.borrow();
                    (
                        (p.vars.x.floor() as i32).div_euclid(8),
                        (p.vars.y.floor() as i32).div_euclid(8),
                    )
                };
                record_player_seen(wx, wy, world_age_hours);
            }
        }
    }
    update(thread, &cfg)
}

pub fn register(thread: &mut Thread) {
    let b = expose_class(thread, LOOT_RESPAWN_CLASS, None);
    b.static_fn("Reset", |_| {
        reset();
        Ok(0)
    });
    b.static_fn("update", |c| {
        let _ = update_from_sandbox(c.thread);
        Ok(0)
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use pz_inventory::shared::SharedContainer;
    use pz_kahlua::TableRef;
    use pz_world::object_containers::RuntimeContainers;
    use pz_world::{IsoCell, IsoChunk, IsoGridSquare, IsoObject, ObjectKind};
    use std::cell::RefCell;
    use std::rc::Rc;

    fn make_container(c: SharedContainer) -> ContainerRef {
        Rc::new(RefCell::new(c))
    }

    #[test]
    fn interval_mapping_matches_sandbox_enum() {
        assert_eq!(loot_respawn_interval_hours(1), -1);
        assert_eq!(loot_respawn_interval_hours(2), 24);
        assert_eq!(loot_respawn_interval_hours(3), 168);
        assert_eq!(loot_respawn_interval_hours(4), 720);
        assert_eq!(loot_respawn_interval_hours(5), 1440);
    }

    #[test]
    fn respawn_respects_interval_seen_hours_max_items_and_safehouses() {
        reset();
        crate::bind_safehouse::install_safehouses(vec![[8, 0, 8, 8]]);
        let mut t = Thread::new(TableRef::new());
        pz_kahlua::stdlib::install(&mut t, None).unwrap();

        let mut cell = IsoCell::default();
        // Chunk (0, 0): eligible explored container (< max_items), not in safehouse.
        let mut c0 = IsoChunk::new(0, 0);
        c0.saved.loot_respawn_hour = 0;
        let mut sq0 = IsoGridSquare::new(2, 2, 0);
        let cont0 = make_container(SharedContainer {
            kind: "crate".into(),
            explored: true,
            ..SharedContainer::default()
        });
        let mut obj0 = IsoObject::new(1, ObjectKind::Object, None);
        obj0.runtime_containers = Some(RuntimeContainers {
            main: Some(cont0.clone()),
            secondary: Vec::new(),
        });
        sq0.objects.push(obj0.into());
        c0.set_square(2, 2, 0, Some(sq0.into()));
        cell.chunks.insert((0, 0), c0);

        // Chunk (1, 0): inside safehouse [8..16, 0..8) -> skipped.
        let mut c1 = IsoChunk::new(1, 0);
        c1.saved.loot_respawn_hour = 0;
        let mut sq1 = IsoGridSquare::new(10, 2, 0);
        let cont1 = make_container(SharedContainer {
            kind: "crate".into(),
            explored: true,
            ..SharedContainer::default()
        });
        let mut obj1 = IsoObject::new(2, ObjectKind::Object, None);
        obj1.runtime_containers = Some(RuntimeContainers {
            main: Some(cont1.clone()),
            secondary: Vec::new(),
        });
        sq1.objects.push(obj1.into());
        c1.set_square(2, 2, 0, Some(sq1.into()));
        cell.chunks.insert((1, 0), c1);

        crate::bind_world::set_cell(Some(cell));

        // First tick initializes baseline at hour 0 -> 0 refills.
        let base_cfg = LootRespawnConfig {
            respawn_hours: 24,
            seen_hours_prevent: 12,
            max_items: 4,
            world_age_hours: 10.0,
        };
        assert_eq!(update(&mut t, &base_cfg), 0);

        // Mark chunk (0, 0) seen at hour 20 -> at hour 24 only 4h elapsed (< 12h), skipped!
        record_player_seen(0, 0, 20.0);
        assert_eq!(
            update(
                &mut t,
                &LootRespawnConfig {
                    world_age_hours: 24.0,
                    ..base_cfg
                }
            ),
            0
        );

        // At hour 48 (28h since seen >= 12h): chunk (0, 0) respawns, chunk (1, 0) skipped by safehouse!
        assert_eq!(
            update(
                &mut t,
                &LootRespawnConfig {
                    world_age_hours: 48.0,
                    ..base_cfg
                }
            ),
            1
        );
        // Same boundary again -> already respawned, 0 refills.
        assert_eq!(
            update(
                &mut t,
                &LootRespawnConfig {
                    world_age_hours: 50.0,
                    ..base_cfg
                }
            ),
            0
        );

        crate::bind_safehouse::install_safehouses(Vec::new());
        crate::bind_world::set_cell(None);
        reset();
    }
}
