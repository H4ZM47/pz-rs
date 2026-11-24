//! `zombie.iso.FishSchoolManager` Lua binding: `FishSchoolManager.getInstance()`,
//! `getFishAbundance`, `getTrashAbundance`, `catchFish`, `addChum`,
//! `addSoundNoise`, `updateSeed`, `generateSeed`, `init`, `update`.

use crate::expose::{expose_class, int_arg, num_arg};
use pz_kahlua::value::LuaObject;
use pz_kahlua::{Thread, Value};
use pz_world::fish_school::FishSchoolManager;
use std::any::Any;
use std::cell::RefCell;
use std::rc::Rc;

pub const FISH_SCHOOL_MANAGER_CLASS: &str = "zombie.iso.FishSchoolManager";

struct FishSchoolManagerObject;

impl LuaObject for FishSchoolManagerObject {
    fn class_name(&self) -> &str {
        FISH_SCHOOL_MANAGER_CLASS
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

thread_local! {
    static MANAGER: RefCell<FishSchoolManager> = RefCell::new(FishSchoolManager::default());
    static INSTANCE: Value = Value::Object(Rc::new(FishSchoolManagerObject));
}

pub fn with_manager<R>(f: impl FnOnce(&mut FishSchoolManager) -> R) -> R {
    MANAGER.with(|m| f(&mut m.borrow_mut()))
}

pub fn init(seed: i32, world_age_hours: f64) {
    with_manager(|m| m.init(seed, world_age_hours));
}

pub fn reset() {
    with_manager(FishSchoolManager::reset);
}

pub fn update(world_age_hours: f64) {
    with_manager(|m| m.update(world_age_hours));
}

pub fn add_sound_noise(x: i32, y: i32, radius: i32) {
    with_manager(|m| m.add_sound_noise(x, y, radius));
}

pub fn register(thread: &mut Thread) {
    let b = expose_class(thread, FISH_SCHOOL_MANAGER_CLASS, None);
    b.static_fn("getInstance", |c| Ok(c.push(INSTANCE.with(Clone::clone))));
    b.method("init", |_| {
        let age = crate::bind_climate::shared_game_time()
            .borrow()
            .world_age_hours();
        let seed = pz_core::random::next(i32::MAX);
        init(seed, age);
        Ok(0)
    });
    b.method("update", |_| {
        let age = crate::bind_climate::shared_game_time()
            .borrow()
            .world_age_hours();
        update(age);
        Ok(0)
    });
    b.method("updateSeed", |_| {
        with_manager(FishSchoolManager::update_seed);
        Ok(0)
    });
    b.method("generateSeed", |_| {
        let seed = pz_core::random::next(i32::MAX);
        with_manager(|m| m.generate_seed(seed));
        Ok(0)
    });
    b.method("getFishAbundance", |c| {
        let (x, y) = (int_arg(c, 1), int_arg(c, 2));
        let v = with_manager(|m| m.get_fish_abundance(x, y)) as f64;
        Ok(c.push(v))
    });
    b.method("getTrashAbundance", |c| {
        let (x, y) = (int_arg(c, 1), int_arg(c, 2));
        let v = with_manager(|m| m.get_trash_abundance(x, y)) as f64;
        Ok(c.push(v))
    });
    b.method("catchFish", |c| {
        let (x, y) = (int_arg(c, 1), int_arg(c, 2));
        with_manager(|m| m.catch_fish(x, y));
        Ok(0)
    });
    b.method("addChum", |c| {
        let (x, y, force) = (int_arg(c, 1), int_arg(c, 2), num_arg(c, 3) as f32);
        with_manager(|m| m.add_chum(x, y, force));
        Ok(0)
    });
    b.method("addSoundNoise", |c| {
        let (x, y, radius) = (int_arg(c, 1), int_arg(c, 2), int_arg(c, 3));
        add_sound_noise(x, y, radius);
        Ok(0)
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use pz_kahlua::TableRef;

    #[test]
    fn fishing_lua_calls_abundance_catch_chum_noise_and_seed() {
        init(123, 2.0);
        let mut t = Thread::new(TableRef::new());
        pz_kahlua::stdlib::install(&mut t, None).unwrap();
        register(&mut t);
        let f = pz_kahlua::stdlib::load_string(
            r#"
            local fsm = FishSchoolManager.getInstance()
            local base = fsm:getFishAbundance(100, 100)
            local trash = fsm:getTrashAbundance(100, 100)
            fsm:catchFish(100, 100)
            local afterCatch = fsm:getFishAbundance(100, 100)
            fsm:addChum(100, 100, 80)
            local afterChum = fsm:getFishAbundance(100, 100)
            fsm:addSoundNoise(100, 100, 12)
            local afterNoise = fsm:getFishAbundance(100, 100)
            fsm:updateSeed()
            return base, trash, afterCatch, afterChum, afterNoise
            "#,
            Some("fish_school_lua"),
            t.globals().clone(),
        )
        .unwrap();
        let r = t.pcall(&f, &[]).unwrap();
        let base = r[0].as_num().unwrap();
        let trash = r[1].as_num().unwrap();
        let after_catch = r[2].as_num().unwrap();
        let after_chum = r[3].as_num().unwrap();
        let after_noise = r[4].as_num().unwrap();
        assert!(base > 0.0);
        assert!((0.05..=0.45).contains(&trash));
        assert!((after_catch - (base - 1.0)).abs() < 1e-4);
        assert!(after_chum > after_catch);
        assert!(after_noise < after_chum);
        reset();
    }
}
