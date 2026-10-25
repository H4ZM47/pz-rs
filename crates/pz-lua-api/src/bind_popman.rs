//! Lua exposure of the zombie population: the `LuaManager.GlobalObject`
//! functions that drive `ZombiePopulationManager.instance`
//! (`setMinMaxZombiesPerChunk`, `createHordeFromTo`, `createHordeInAreaTo`,
//! `zpopSpawnTimeToZero`, `zpopClearZombies`, `zpopSpawnNow`,
//! `addVirtualZombie`, `setAggroTarget`, `zpopNewRenderer`) and the exposed
//! classes `zombie.VirtualZombieManager` and
//! `zombie.popman.ZombiePopulationRenderer`.
//!
//! The manager is the engine's: install it with [`install_population`].
//! Until then the functions do nothing.
//!
//! Not bound here: `spawnHorde` and `createZombie` (they create real
//! `IsoZombie`s through `VirtualZombieManager`), the
//! `VirtualZombieManager` instance methods, and the renderer's drawing and
//! option methods.

use crate::expose::{expose_class, global, int_arg, num_arg};
use pz_kahlua::value::LuaObject;
use pz_kahlua::{Thread, Value};
use pz_popman::ZombiePopulationManager;
use pz_world::java::fastfloor;
use std::any::Any;
use std::cell::RefCell;
use std::rc::Rc;

const VIRTUAL_ZOMBIE_MANAGER: &str = "zombie.VirtualZombieManager";
const RENDERER: &str = "zombie.popman.ZombiePopulationRenderer";
const BOOLEAN_DEBUG_OPTION: &str = "zombie.popman.ZombiePopulationRenderer$BooleanDebugOption";

/// What the Lua functions call on `ZombiePopulationManager.instance`.
pub trait PopulationLua {
    fn set_zombies_min_per_chunk(&mut self, f: f32);
    fn set_zombies_max_per_chunk(&mut self, f: f32);
    fn create_horde_from_to(&mut self, sx: i32, sy: i32, tx: i32, ty: i32, count: i32);
    #[allow(clippy::too_many_arguments)]
    fn create_horde_in_area_to(
        &mut self,
        sx: i32,
        sy: i32,
        sw: i32,
        sh: i32,
        tx: i32,
        ty: i32,
        count: i32,
    );
    fn dbg_spawn_time_to_zero(&mut self, cx: i32, cy: i32);
    fn dbg_clear_zombies(&mut self, cx: i32, cy: i32);
    fn dbg_spawn_now(&mut self, cx: i32, cy: i32);
    fn set_aggro_target(&mut self, id: i32, x: i32, y: i32);
}

impl<H: Copy + Eq> PopulationLua for ZombiePopulationManager<H> {
    fn set_zombies_min_per_chunk(&mut self, f: f32) {
        ZombiePopulationManager::set_zombies_min_per_chunk(self, f);
    }
    fn set_zombies_max_per_chunk(&mut self, f: f32) {
        ZombiePopulationManager::set_zombies_max_per_chunk(self, f);
    }
    fn create_horde_from_to(&mut self, sx: i32, sy: i32, tx: i32, ty: i32, count: i32) {
        ZombiePopulationManager::create_horde_from_to(self, sx, sy, tx, ty, count);
    }
    fn create_horde_in_area_to(
        &mut self,
        sx: i32,
        sy: i32,
        sw: i32,
        sh: i32,
        tx: i32,
        ty: i32,
        count: i32,
    ) {
        ZombiePopulationManager::create_horde_in_area_to(self, sx, sy, sw, sh, tx, ty, count);
    }
    fn dbg_spawn_time_to_zero(&mut self, cx: i32, cy: i32) {
        ZombiePopulationManager::dbg_spawn_time_to_zero(self, cx, cy);
    }
    fn dbg_clear_zombies(&mut self, cx: i32, cy: i32) {
        ZombiePopulationManager::dbg_clear_zombies(self, cx, cy);
    }
    fn dbg_spawn_now(&mut self, cx: i32, cy: i32) {
        ZombiePopulationManager::dbg_spawn_now(self, cx, cy);
    }
    fn set_aggro_target(&mut self, id: i32, x: i32, y: i32) {
        ZombiePopulationManager::set_aggro_target(self, id, x, y);
    }
}

thread_local! {
    static POPULATION: RefCell<Option<Rc<RefCell<dyn PopulationLua>>>> = const { RefCell::new(None) };
}

/// Makes `m` the `ZombiePopulationManager.instance` Lua talks to.
pub fn install_population(m: Rc<RefCell<dyn PopulationLua>>) {
    POPULATION.with(|p| *p.borrow_mut() = Some(m));
}

fn with_population(f: impl FnOnce(&mut dyn PopulationLua)) {
    if let Some(m) = POPULATION.with(|p| p.borrow().clone()) {
        f(&mut *m.borrow_mut());
    }
}

/// `VirtualZombieManager.instance` (no exposed fields; the instance
/// methods need the live world).
pub struct VirtualZombieManagerObject;

impl LuaObject for VirtualZombieManagerObject {
    fn class_name(&self) -> &str {
        VIRTUAL_ZOMBIE_MANAGER
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// A `ZombiePopulationRenderer` (`zpopNewRenderer()`).
pub struct PopulationRendererObject;

impl LuaObject for PopulationRendererObject {
    fn class_name(&self) -> &str {
        RENDERER
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

pub fn register(thread: &mut Thread) {
    let vzm = expose_class(thread, VIRTUAL_ZOMBIE_MANAGER, Some("java.lang.Object"));
    vzm.field(
        "instance",
        Value::Object(Rc::new(VirtualZombieManagerObject)),
    );
    vzm.ctor(|c| Ok(c.push(Value::Object(Rc::new(VirtualZombieManagerObject)))));
    let r = expose_class(thread, RENDERER, Some("java.lang.Object"));
    r.ctor(|c| Ok(c.push(Value::Object(Rc::new(PopulationRendererObject)))));
    expose_class(
        thread,
        BOOLEAN_DEBUG_OPTION,
        Some("zombie.config.BooleanConfigOption"),
    );

    global(thread, "setMinMaxZombiesPerChunk", |c| {
        let (min, max) = (num_arg(c, 0) as f32, num_arg(c, 1) as f32);
        with_population(|m| {
            m.set_zombies_min_per_chunk(min);
            m.set_zombies_max_per_chunk(max);
        });
        Ok(0)
    });
    global(thread, "createHordeFromTo", |c| {
        let f = |i| fastfloor(num_arg(c, i) as f32);
        let (sx, sy, tx, ty, n) = (f(0), f(1), f(2), f(3), int_arg(c, 4));
        with_population(|m| m.create_horde_from_to(sx, sy, tx, ty, n));
        Ok(0)
    });
    global(thread, "createHordeInAreaTo", |c| {
        let a: Vec<i32> = (0..7).map(|i| int_arg(c, i)).collect();
        with_population(|m| m.create_horde_in_area_to(a[0], a[1], a[2], a[3], a[4], a[5], a[6]));
        Ok(0)
    });
    global(thread, "zpopNewRenderer", |c| {
        Ok(c.push(Value::Object(Rc::new(PopulationRendererObject))))
    });
    global(thread, "zpopSpawnTimeToZero", |c| {
        let (x, y) = (int_arg(c, 0), int_arg(c, 1));
        with_population(|m| m.dbg_spawn_time_to_zero(x, y));
        Ok(0)
    });
    global(thread, "zpopClearZombies", |c| {
        let (x, y) = (int_arg(c, 0), int_arg(c, 1));
        with_population(|m| m.dbg_clear_zombies(x, y));
        Ok(0)
    });
    global(thread, "zpopSpawnNow", |c| {
        let (x, y) = (int_arg(c, 0), int_arg(c, 1));
        with_population(|m| m.dbg_spawn_now(x, y));
        Ok(0)
    });
    // An empty method in the original.
    global(thread, "addVirtualZombie", |_| Ok(0));
    global(thread, "setAggroTarget", |c| {
        let (id, x, y) = (int_arg(c, 0), int_arg(c, 1), int_arg(c, 2));
        with_population(|m| m.set_aggro_target(id, x, y));
        Ok(0)
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use pz_kahlua::TableRef;

    #[derive(Default)]
    struct Rec(Vec<String>);

    impl PopulationLua for Rec {
        fn set_zombies_min_per_chunk(&mut self, f: f32) {
            self.0.push(format!("min {f}"));
        }
        fn set_zombies_max_per_chunk(&mut self, f: f32) {
            self.0.push(format!("max {f}"));
        }
        fn create_horde_from_to(&mut self, sx: i32, sy: i32, tx: i32, ty: i32, count: i32) {
            self.0.push(format!("horde {sx} {sy} {tx} {ty} {count}"));
        }
        fn create_horde_in_area_to(
            &mut self,
            sx: i32,
            sy: i32,
            sw: i32,
            sh: i32,
            tx: i32,
            ty: i32,
            count: i32,
        ) {
            self.0
                .push(format!("area {sx} {sy} {sw} {sh} {tx} {ty} {count}"));
        }
        fn dbg_spawn_time_to_zero(&mut self, cx: i32, cy: i32) {
            self.0.push(format!("t0 {cx} {cy}"));
        }
        fn dbg_clear_zombies(&mut self, cx: i32, cy: i32) {
            self.0.push(format!("clear {cx} {cy}"));
        }
        fn dbg_spawn_now(&mut self, cx: i32, cy: i32) {
            self.0.push(format!("now {cx} {cy}"));
        }
        fn set_aggro_target(&mut self, id: i32, x: i32, y: i32) {
            self.0.push(format!("aggro {id} {x} {y}"));
        }
    }

    #[test]
    fn globals_reach_the_manager() {
        let mut t = Thread::new(TableRef::new());
        pz_kahlua::stdlib::install(&mut t, None).unwrap();
        register(&mut t);
        let rec = Rc::new(RefCell::new(Rec::default()));
        install_population(rec.clone());
        let src = r#"
            local function assert(v) if not v then error("assertion failed!", 2) end end
            setMinMaxZombiesPerChunk(0.5, 40)
            createHordeFromTo(10.7, -3.2, 20, 30, 5)
            createHordeInAreaTo(1, 2, 3, 4, 5, 6, 7)
            zpopSpawnTimeToZero(1, 2) zpopClearZombies(3, 4) zpopSpawnNow(5, 6)
            addVirtualZombie(1, 1)
            setAggroTarget(9, 100, 200)
            assert(VirtualZombieManager.instance ~= nil)
            assert(zpopNewRenderer() ~= nil)
        "#;
        let p = pz_kahlua::compiler::compile(src, Some("t"), Some("t"), None).unwrap();
        let f = Value::Closure(pz_kahlua::vm::LuaClosure::new(
            Rc::new(p),
            t.globals().clone(),
        ));
        t.pcall(&f, &[])
            .map_err(|(e, tr)| format!("{}\n{tr}", pz_kahlua::exec::java_value_of(&e)))
            .unwrap();
        assert_eq!(
            rec.borrow().0,
            vec![
                "min 0.5",
                "max 40",
                "horde 10 -4 20 30 5",
                "area 1 2 3 4 5 6 7",
                "t0 1 2",
                "clear 3 4",
                "now 5 6",
                "aggro 9 100 200"
            ]
        );
    }
}
