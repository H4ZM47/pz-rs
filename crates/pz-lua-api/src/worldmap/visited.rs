//! `WorldMapVisited`'s statics: the one instance (`getInstance`), the
//! per-frame `update` from the local players, `SaveAll` and `Reset`.

use pz_worldmap::visited::{MetaBounds, WorldMapVisited};
use std::cell::RefCell;
use std::path::PathBuf;

thread_local! {
    /// `WorldMapVisited.instance`.
    static INSTANCE: RefCell<Option<WorldMapVisited>> = const { RefCell::new(None) };
}

/// `ZomboidFileSystem.getFileNameInCurrentSave("map_visited.bin")`; `None`
/// without an installed file system.
fn save_file() -> Option<PathBuf> {
    let fs = super::FS.with(|f| f.borrow().clone())?;
    let saves = crate::bind_save::save_dir(&fs);
    let core = crate::bind_core::shared_core();
    let core = core.borrow();
    Some(
        PathBuf::from(saves)
            .join(&core.game_mode)
            .join(&core.game_save_world)
            .join("map_visited.bin"),
    )
}

/// `save()`: skipped for `Core.isNoSave()` (and on a client).
fn save(v: &WorldMapVisited) -> std::io::Result<()> {
    if crate::bind_core::shared_core().borrow().is_no_save() || v.client {
        return Ok(());
    }
    match save_file() {
        Some(path) => v.save(&path),
        None => Ok(()),
    }
}

/// `getInstance()` then `f` on it. The original throws when the world has
/// no meta grid, and `setBounds` throws (after the instance exists) for a
/// grid whose bounds give a negative length.
pub fn with_instance<R>(f: impl FnOnce(&mut WorldMapVisited) -> R) -> Result<R, &'static str> {
    let Some((min_x, min_y, max_x, max_y)) = crate::bind_world::meta_grid_cell_bounds() else {
        return Err("java.lang.NullPointerException: IsoWorld.instance.MetaGrid is null");
    };
    INSTANCE.with(|i| {
        let mut i = i.borrow_mut();
        if i.is_none() {
            let meta = MetaBounds { min_x, min_y, max_x, max_y };
            let (mut v, err) = WorldMapVisited::new(meta);
            // `GameClient.client`: this port runs no client.
            v.client = false;
            if let Some(err) = err {
                *i = Some(v);
                return Err(err);
            }
            if !v.main_menu {
                let loaded = match save_file() {
                    Some(path) => v.load(&path),
                    None => Ok(()),
                };
                let saved = loaded.and_then(|()| {
                    if crate::bind_sandbox::current_bool("Map.MapAllKnown") == Some(true) {
                        v.set_known_in_cells(min_x, min_y, max_x, max_y);
                    }
                    save(&v)
                });
                // `ExceptionLogger.logException(t)`.
                if let Err(e) = saved {
                    eprintln!("WorldMapVisited.getInstance: {e}");
                }
            }
            *i = Some(v);
        }
        Ok(f(i.as_mut().expect("instance set above")))
    })
}

/// `WorldMapVisited.update()`: each live local player marks the units
/// within 25 squares of its own position.
pub fn update() -> Result<(), &'static str> {
    let n = crate::bind_players::num_players();
    let players: Vec<Option<(f32, f32)>> = (0..n.clamp(0, 4) as usize)
        .map(|i| {
            let p = crate::bind_players::player(i)?;
            let p = p.borrow();
            (!p.is_dead()).then_some((p.vars.x, p.vars.y))
        })
        .collect();
    with_instance(|v| v.update(&players))
}

/// `WorldMapVisited.SaveAll()`.
pub fn save_all() {
    INSTANCE.with(|i| {
        if let Some(v) = i.borrow().as_ref() {
            if let Err(e) = save(v) {
                eprintln!("WorldMapVisited.SaveAll: {e}");
            }
        }
    });
}

/// `WorldMapVisited.Reset()`.
pub fn reset() {
    INSTANCE.with(|i| *i.borrow_mut() = None);
}

/// `WorldMapVisited.instance` exists.
pub fn has_instance() -> bool {
    INSTANCE.with(|i| i.borrow().is_some())
}
