//! Lua binding for `zombie.MapGroups` (logic in `pz_world::map_groups`).
//!
//! `ActiveMods`, mod folders and `Translator` belong to other subsystems;
//! they are reached through a [`MapGroupsHost`] that the engine installs.
//! Until then a default host lists the installed `media/maps` only (no mods,
//! untranslated conflict keys).

use crate::bind_java_util::JavaList;
use crate::expose::{bool_arg, expose_class, int_arg, str_arg, this};
use pz_kahlua::value::LuaObject;
use pz_kahlua::{LuaError, NativeCall, Thread, Value};
use pz_world::map_groups::{add_missing_vanilla_directories, ActiveModsView, MapGroups};
use std::any::Any;
use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

pub const MAP_GROUPS: &str = "zombie.MapGroups";

/// What `MapGroups` needs from systems outside the map code.
pub trait MapGroupsHost {
    /// `ZomboidFileSystem.getMediaFile("maps")`.
    fn maps_dir(&self) -> PathBuf;
    /// `getMods()`/`getMapOrder()` of an `ActiveMods` Lua value.
    fn active_mods(&self, v: &Value) -> ActiveModsView;
    /// `ActiveMods.getById("currentGame")`.
    fn current_game_mods(&self) -> ActiveModsView;
    /// `ChooseGameInfo.getAvailableModDetails(id)`: `(commonDir, versionDir)`.
    fn mod_dirs(&self, id: &str) -> Option<(String, String)>;
    /// `Translator.getText(key, args...)`.
    fn get_text(&self, key: &str, args: &[&str]) -> String;
}

struct DefaultHost {
    maps_dir: Box<dyn Fn() -> PathBuf>,
}

impl MapGroupsHost for DefaultHost {
    fn maps_dir(&self) -> PathBuf {
        (self.maps_dir)()
    }
    fn active_mods(&self, _v: &Value) -> ActiveModsView {
        ActiveModsView::default()
    }
    fn current_game_mods(&self) -> ActiveModsView {
        ActiveModsView::default()
    }
    fn mod_dirs(&self, _id: &str) -> Option<(String, String)> {
        None
    }
    fn get_text(&self, key: &str, _args: &[&str]) -> String {
        key.to_string()
    }
}

thread_local! {
    static HOST: RefCell<Option<Rc<dyn MapGroupsHost>>> = const { RefCell::new(None) };
}

/// Install the engine's host (replaces the default).
pub fn install_host(host: Rc<dyn MapGroupsHost>) {
    HOST.with(|h| *h.borrow_mut() = Some(host));
}

fn host() -> Rc<dyn MapGroupsHost> {
    HOST.with(|h| h.borrow().clone())
        .expect("MapGroups host installed by register")
}

/// `IsoMetaGrid.getLotDirectories()` with `Core.gameMap` "DEFAULT": `new
/// MapGroups()`, `createGroups()`, and with exactly one group
/// `setWorld(0)` (`Core.gameMap` becomes its directories joined by `;`).
pub fn set_default_world() -> Result<(), LuaError> {
    let h = host();
    let mut g = MapGroups::new();
    let dirs = |id: &str| h.mod_dirs(id);
    g.create_groups(&h.current_game_mods(), &dirs, &h.maps_dir(), true, false);
    if g.get_number_of_groups() != 1 {
        return Err(LuaError::Runtime {
            message: "GameMap is DEFAULT but there are multiple worlds to choose from".into(),
            class: "java.lang.RuntimeException",
        });
    }
    let name = g.world_map_name(0).map_err(LuaError::runtime)?;
    crate::bind_core::shared_core().borrow_mut().game_map = name;
    Ok(())
}

struct LuaMapGroups(RefCell<MapGroups>);

impl LuaObject for LuaMapGroups {
    fn class_name(&self) -> &str {
        MAP_GROUPS
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

fn with_groups<R>(c: &NativeCall<'_>, f: impl FnOnce(&mut MapGroups) -> R) -> Result<R, LuaError> {
    let o = this::<LuaMapGroups>(c)?;
    let g = o.as_any().downcast_ref::<LuaMapGroups>().unwrap();
    let r = f(&mut g.0.borrow_mut());
    Ok(r)
}

fn string_list(v: Vec<String>) -> Value {
    Value::Object(Rc::new(JavaList::new(v.into_iter().map(Value::from).collect())))
}

/// `maps_dir` is `ZomboidFileSystem.getMediaFile("maps")` for the default
/// host.
pub fn register(thread: &mut Thread, maps_dir: impl Fn() -> PathBuf + 'static) {
    HOST.with(|h| {
        if h.borrow().is_none() {
            *h.borrow_mut() = Some(Rc::new(DefaultHost {
                maps_dir: Box::new(maps_dir),
            }));
        }
    });
    let b = expose_class(thread, MAP_GROUPS, None);
    b.ctor(|c| {
        Ok(c.push(Value::Object(Rc::new(LuaMapGroups(RefCell::new(
            MapGroups::new(),
        ))))))
    });
    b.static_fn("addMissingVanillaDirectories", |c| {
        let name = str_arg(c, 0)
            .ok_or_else(|| LuaError::runtime("addMissingVanillaDirectories: expected String"))?;
        Ok(c.push(add_missing_vanilla_directories(&host().maps_dir(), &name)))
    });
    // createGroups(), createGroups(activeMods, includeVanilla),
    // createGroups(activeMods, includeVanilla, includeChallenges).
    b.method("createGroups", |c| {
        let h = host();
        let (mods, vanilla, challenges) = if c.nargs() <= 1 {
            (h.current_game_mods(), true, false)
        } else {
            (
                h.active_mods(&c.arg(1)),
                bool_arg(c, 2),
                c.nargs() > 3 && bool_arg(c, 3),
            )
        };
        let dirs = |id: &str| h.mod_dirs(id);
        let maps = h.maps_dir();
        with_groups(c, |g| {
            g.create_groups(&mods, &dirs, &maps, vanilla, challenges)
        })?;
        Ok(0)
    });
    b.method("getNumberOfGroups", |c| {
        let n = with_groups(c, |g| g.get_number_of_groups())?;
        Ok(c.push(n as f64))
    });
    b.method("getMapDirectoriesInGroup", |c| {
        let i = int_arg(c, 1);
        let dirs =
            with_groups(c, |g| g.get_map_directories_in_group(i))?.map_err(LuaError::runtime)?;
        Ok(c.push(string_list(dirs)))
    });
    b.method("setWorld", |c| {
        let i = int_arg(c, 1);
        let name = with_groups(c, |g| g.world_map_name(i))?.map_err(LuaError::runtime)?;
        // IsoWorld.setMap(world): Core.gameMap = world.
        crate::bind_core::shared_core().borrow_mut().game_map = name;
        Ok(0)
    });
    b.method("getAllMapsInOrder", |c| {
        let v = with_groups(c, |g| g.get_all_maps_in_order())?;
        Ok(c.push(string_list(v)))
    });
    b.method("checkMapConflicts", |c| {
        let h = host();
        let r = with_groups(c, |g| g.check_map_conflicts(&|k, a| h.get_text(k, a)))?;
        Ok(c.push(r))
    });
    b.method("getMapConflicts", |c| {
        let name = str_arg(c, 1).map(|s| s.to_string()).unwrap_or_default();
        let r = with_groups(c, |g| g.get_map_conflicts(&name))?;
        Ok(c.push(r.map(string_list).unwrap_or(Value::Nil)))
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use pz_kahlua::TableRef;

    fn run(t: &mut Thread, src: &str) -> Vec<Value> {
        let p = pz_kahlua::compiler::compile(src, Some("test"), Some("test"), None).unwrap();
        let f = Value::Closure(pz_kahlua::vm::LuaClosure::new(
            Rc::new(p),
            t.globals().clone(),
        ));
        match t.pcall(&f, &[]) {
            Ok(v) => v,
            Err((e, tr)) => panic!("lua error: {e:?}\n{tr}"),
        }
    }

    #[test]
    fn lua_calls() {
        let root = std::env::temp_dir().join(format!("pz-lua-mapgroups-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        for (name, lots) in [("Base", ""), ("Addon", "lots=Base\n")] {
            let d = root.join("media/maps").join(name);
            std::fs::create_dir_all(&d).unwrap();
            std::fs::write(d.join("map.info"), format!("title={name}\n{lots}")).unwrap();
        }
        let mut t = Thread::new(TableRef::new());
        pz_kahlua::stdlib::install(&mut t, None).unwrap();
        crate::bind_java_util::register(&mut t);
        crate::bind_core::register(&mut t, None, crate::bind_core::shared_core());
        let maps = root.join("media/maps");
        register(&mut t, move || maps.clone());
        run(&mut t, "function assert(v, msg) if not v then error(msg or 'assertion failed!', 2) end return v end");
        let out = run(
            &mut t,
            r#"
            local g = MapGroups.new()
            g:createGroups()
            assert(g:getNumberOfGroups() == 1)
            local dirs = g:getMapDirectoriesInGroup(0)
            assert(dirs:size() == 2 and dirs:get(0) == "Addon" and dirs:get(1) == "Base")
            g:setWorld(0)
            assert(not g:checkMapConflicts())
            assert(g:getMapConflicts("Nope") == nil)
            assert(not pcall(function() g:getMapDirectoriesInGroup(5) end))
            return MapGroups.addMissingVanillaDirectories("Base")
            "#,
        );
        let joined = pz_kahlua::exec::java_value_of(&out[0]);
        assert!(joined == "Base;Addon", "{joined}");
        assert_eq!(
            crate::bind_core::shared_core().borrow().game_map,
            "Addon;Base"
        );
        let _ = std::fs::remove_dir_all(&root);
    }
}
