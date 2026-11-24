//! `zombie.iso.objects.IsoFire` and `zombie.iso.objects.IsoFireManager` Lua
//! bindings (`SCampfireGlobalObject.lua`, `SCampfireSystem.lua`,
//! `FireFighting.lua`, `ISExtinguishCursor.lua`, `AReallyCDDAy.lua`,
//! `ClientCommands.lua`).

use std::cell::RefCell;
use std::rc::Rc;

use pz_core::core::ColorInfo;
use pz_kahlua::{LuaError, LuaResult, NativeCall, NativeFunction, TableRef, Thread, Value};
use pz_world::fire::manager::{FireRef, IsoFireManager};
use pz_world::fire::{
    AnimKind, FireAnim, FireSquare, IsoFire, FIRE_ANIM_DELAY, FIRE_TINT_MOD, MAX_FIRE_OBJECTS,
    MAX_LIFE, MIN_LIFE, NUM_FRAMES_FIRE, NUM_FRAMES_SMOKE, SMOKE_ANIM_DELAY, SMOKE_TINT_MOD,
};
use pz_world::objects::{IsoObject, ObjectKind};
use pz_world::{GameRand, IsoObjectRef};

use crate::bind_core::LuaColorInfo;
use crate::bind_world::{square_coords, square_value, with_world};
use crate::bind_world_object::{
    borrowed, object_from_value, object_square, object_value, remember_class, remember_square,
    ISO_OBJECT,
};
use crate::events::EventManager;
use crate::expose::{bool_arg, expose_class, int_arg, num_arg, str_arg};

pub const ISO_FIRE: &str = "zombie.iso.objects.IsoFire";

thread_local! {
    static SHARED_FIRE_MANAGER: RefCell<Rc<RefCell<IsoFireManager>>> =
        RefCell::new(Rc::new(RefCell::new(IsoFireManager::new())));
}

/// Shared [`IsoFireManager`] used by Lua `IsoFireManager.*` statics.
pub fn shared_fire_manager() -> Rc<RefCell<IsoFireManager>> {
    SHARED_FIRE_MANAGER.with(|m| m.borrow().clone())
}

/// Installs an engine-owned [`IsoFireManager`] or resets to a fresh manager.
pub fn install_fire_manager(mgr: Option<Rc<RefCell<IsoFireManager>>>) {
    SHARED_FIRE_MANAGER.with(|m| {
        *m.borrow_mut() = mgr.unwrap_or_else(|| Rc::new(RefCell::new(IsoFireManager::new())));
    });
}

struct LuaFireRand;
impl GameRand for LuaFireRand {
    fn next(&mut self, max: i32) -> i32 {
        pz_core::random::next(max)
    }
    fn next_float(&mut self) -> f32 {
        pz_core::random::next_float(0.0, 1.0)
    }
}

fn fire_receiver(c: &NativeCall<'_>) -> LuaResult<IsoObjectRef> {
    let this = c.arg(0);
    let obj = object_from_value(&this)
        .ok_or_else(|| LuaError::runtime("Expected IsoFire receiver"))?;
    Ok(obj.object.clone())
}

fn with_fire<R>(c: &NativeCall<'_>, f: impl FnOnce(&IsoFire) -> R) -> LuaResult<R> {
    let obj = fire_receiver(c)?;
    let body = obj.try_borrow().map_err(|_| borrowed("IsoFire body"))?;
    match &body.kind {
        ObjectKind::Fire(fire) => Ok(f(fire)),
        _ => Err(LuaError::runtime("Expected IsoFire object")),
    }
}

fn with_fire_mut<R>(c: &NativeCall<'_>, f: impl FnOnce(&mut IsoFire) -> R) -> LuaResult<R> {
    let obj = fire_receiver(c)?;
    let mut body = obj.try_borrow_mut().map_err(|_| borrowed("IsoFire body"))?;
    match &mut body.kind {
        ObjectKind::Fire(fire) => Ok(f(fire)),
        _ => Err(LuaError::runtime("Expected IsoFire object")),
    }
}

fn update_square_fire_flags(x: i32, y: i32, z: i32) {
    let flags = crate::bind_sprites::manager().ok().and_then(|s| {
        let s = s.try_borrow().ok()?;
        pz_world::fire::manager::FireFlags::resolve(&s.tables).ok()
    });
    let Some(flags) = flags else { return };
    with_world(|w| {
        let Some(cell) = w.cell.as_mut() else { return };
        let Some(sq_ref) = cell.get_grid_square(x, y, z) else {
            return;
        };
        let mut sq = sq_ref.borrow_mut();
        let mut has_fire = false;
        let mut has_smoke = false;
        for slot in &sq.objects {
            if let ObjectKind::Fire(f) = &slot.borrow().kind {
                if f.smoke {
                    has_smoke = true;
                } else {
                    has_fire = true;
                }
            }
        }
        if has_fire {
            sq.properties.set_flag(flags.burning);
        } else {
            sq.properties.unset_flag(flags.burning);
        }
        if has_smoke {
            sq.properties.set_flag(flags.smoke);
        } else {
            sq.properties.unset_flag(flags.smoke);
        }
    });
}

fn start_fire_on_square(
    thread: &mut Thread,
    events: &Option<EventManager>,
    x: i32,
    y: i32,
    z: i32,
    energy: i32,
    life: i32,
    is_smoke: bool,
) -> LuaResult<Option<IsoObjectRef>> {
    if !is_smoke && energy <= 0 {
        return Ok(None);
    }
    let mut rand = LuaFireRand;
    let tutorial = crate::bind_core::shared_core()
        .try_borrow()
        .is_ok_and(|c| c.tutorial);
    let veg_flag = crate::bind_sprites::manager()
        .ok()
        .and_then(|s| s.try_borrow().ok().map(|m| m.tables.flag.vegitation));

    let created = with_world(|w| -> Option<(IsoObjectRef, FireRef)> {
        let cell = w.cell.as_mut()?;
        let sq_ref = cell.get_grid_square(x, y, z)?;
        let (veg, id) = {
            let mut sq = sq_ref.borrow_mut();
            let veg = veg_flag.is_some_and(|f| sq.has(f));
            (veg, sq.new_object_id())
        };
        let fire = if is_smoke {
            IsoFire::new_smoke(energy, life, true, &mut rand)
        } else {
            let fs = FireSquare {
                vegitation: veg,
                floor_fire_requirement: None,
                tutorial,
            };
            IsoFire::new_fire(fs, energy.max(5), life, &mut rand)
        };
        let obj = IsoObject::new(id, ObjectKind::Fire(fire), None);
        let slot: pz_world::ObjectSlot = obj.into();
        let handle = slot.ref_handle();
        sq_ref.borrow_mut().objects.push(slot);
        Some((handle, FireRef { x, y, z, id }))
    });

    let Some((handle, fref)) = created else {
        return Ok(None);
    };
    remember_class(&handle, ISO_FIRE)?;
    update_square_fire_flags(x, y, z);

    {
        let mgr = shared_fire_manager();
        let mut mgr = mgr.borrow_mut();
        if !mgr.fire_stack.contains(&fref) {
            if mgr.fire_stack.len() >= MAX_FIRE_OBJECTS {
                mgr.fire_stack.remove(0);
            }
            mgr.fire_stack.push(fref);
        }
    }

    if !is_smoke {
        if let Some(ev) = events {
            let sq_val = square_value(x, y, z);
            ev.trigger(thread, "OnNewFire", &[sq_val]);
        }
    }

    Ok(Some(handle))
}

fn remove_all_fires_on(x: i32, y: i32, z: i32) {
    let removed_ids: Vec<u32> = with_world(|w| {
        let Some(cell) = w.cell.as_mut() else {
            return Vec::new();
        };
        let Some(sq_ref) = cell.get_grid_square(x, y, z) else {
            return Vec::new();
        };
        let mut sq = sq_ref.borrow_mut();
        let mut ids = Vec::new();
        sq.objects.retain(|slot| {
            let b = slot.borrow();
            if let ObjectKind::Fire(f) = &b.kind {
                if !f.perm {
                    ids.push(b.id);
                    return false;
                }
            }
            true
        });
        ids
    });
    if !removed_ids.is_empty() {
        let mgr = shared_fire_manager();
        let mut mgr = mgr.borrow_mut();
        for id in removed_ids {
            mgr.remove(FireRef { x, y, z, id });
        }
    }
    update_square_fire_flags(x, y, z);
}

fn register_iso_fire(thread: &mut Thread) {
    let b = expose_class(thread, ISO_FIRE, Some(ISO_OBJECT));

    b.method("isPermanent", |c| Ok(c.push(with_fire(c, |f| f.perm)?)));
    b.method("isSmoke", |c| Ok(c.push(with_fire(c, |f| f.smoke)?)));
    b.method("getLife", |c| Ok(c.push(with_fire(c, |f| f.life)?)));
    b.method("setLife", |c| {
        let v = int_arg(c, 1);
        with_fire_mut(c, |f| f.life = v)?;
        Ok(0)
    });
    b.method("getLifeStage", |c| {
        Ok(c.push(with_fire(c, |f| f.life_stage)?))
    });
    b.method("setLifeStage", |c| {
        let v = int_arg(c, 1);
        with_fire_mut(c, |f| f.life_stage = v)?;
        Ok(0)
    });
    b.method("getLifeStageDuration", |c| {
        Ok(c.push(with_fire(c, |f| f.life_stage_duration)?))
    });
    b.method("setLifeStageDuration", |c| {
        let v = int_arg(c, 1);
        with_fire_mut(c, |f| f.life_stage_duration = v)?;
        Ok(0)
    });
    b.method("getLifeStageTimer", |c| {
        Ok(c.push(with_fire(c, |f| f.life_stage_timer)?))
    });
    b.method("setLifeStageTimer", |c| {
        let v = int_arg(c, 1);
        with_fire_mut(c, |f| f.life_stage_timer = v)?;
        Ok(0)
    });
    b.method("getEnergy", |c| Ok(c.push(with_fire(c, |f| f.energy)?)));
    b.method("getSpreadDelay", |c| {
        Ok(c.push(with_fire(c, |f| f.spread_delay)?))
    });
    b.method("setSpreadDelay", |c| {
        let v = int_arg(c, 1);
        with_fire_mut(c, |f| f.spread_delay = v)?;
        Ok(0)
    });
    b.method("getSpreadTimer", |c| {
        Ok(c.push(with_fire(c, |f| f.spread_timer)?))
    });
    b.method("setSpreadTimer", |c| {
        let v = int_arg(c, 1);
        with_fire_mut(c, |f| f.spread_timer = v)?;
        Ok(0)
    });
    b.method("getLightRadius", |c| {
        Ok(c.push(with_fire(c, |f| f.light_radius)?))
    });
    b.method("setLightRadius", |c| {
        let v = int_arg(c, 1);
        with_fire_mut(c, |f| f.light_radius = v)?;
        Ok(0)
    });
    b.method("extinctFire", |c| {
        let this = c.arg(0);
        let Some(wo) = object_from_value(&this) else {
            return Ok(0);
        };
        let sq_val = object_square(wo)?;
        let obj_rc = wo.object.clone();
        let obj_id = obj_rc.try_borrow().map_err(|_| borrowed("IsoFire body"))?.id;
        if let Some((x, y, z)) = square_coords(&sq_val) {
            with_world(|w| {
                if let Some(sq) = w.cell.as_mut().and_then(|c| c.get_grid_square(x, y, z)) {
                    sq.borrow_mut()
                        .objects
                        .retain(|slot| !Rc::ptr_eq(&slot.ref_handle(), &obj_rc));
                }
            });
            shared_fire_manager()
                .borrow_mut()
                .remove(FireRef { x, y, z, id: obj_id });
            update_square_fire_flags(x, y, z);
        }
        Ok(0)
    });
    b.method("AttachAnim", |c| {
        let object_name = str_arg(c, 1);
        let anim_name = str_arg(c, 2);
        let frame_increase = num_arg(c, 4) as f32;
        let offset_x = int_arg(c, 5);
        let offset_y = int_arg(c, 6);
        let tint = match c.arg(11) {
            Value::Object(o) => match o.as_any().downcast_ref::<LuaColorInfo>() {
                Some(ci) => {
                    let col = ci.c.get();
                    [col.r, col.g, col.b]
                }
                None => FIRE_TINT_MOD,
            },
            _ => FIRE_TINT_MOD,
        };
        let kind = if object_name
            .as_deref()
            .is_some_and(|s| s.eq_ignore_ascii_case("Smoke"))
        {
            AnimKind::Smoke
        } else {
            AnimKind::Fire
        };
        let anim_idx = anim_name
            .as_deref()
            .and_then(|s| s.trim().parse::<u8>().ok())
            .unwrap_or(1);
        let mut rand = LuaFireRand;
        with_fire_mut(c, |f| {
            f.anims.push(FireAnim::attach(
                kind,
                anim_idx,
                frame_increase,
                offset_x,
                offset_y,
                tint,
                false,
                &mut rand,
            ));
        })?;
        Ok(0)
    });

    // Global `IsoFire` class table with constants and `new(...)` constructor.
    let t = TableRef::new();
    t.set_str("NUM_FRAMES_FIRE", Value::Num(NUM_FRAMES_FIRE as f64));
    t.set_str("NUM_FRAMES_SMOKE", Value::Num(NUM_FRAMES_SMOKE as f64));
    t.set_str("MaxLife", Value::Num(MAX_LIFE as f64));
    t.set_str("MinLife", Value::Num(MIN_LIFE as f64));
    t.set_str(
        "new",
        Value::Native(NativeFunction::new("IsoFire.new", |c| {
            // IsoFire.new(cell, square[, canBurnAnywhere, startingEnergy[, setLife[, isSmoke]]])
            let sq_val = c.arg(1);
            let coords = square_coords(&sq_val);
            let fire = if c.nargs() <= 2 {
                IsoFire::permanent()
            } else {
                let energy = int_arg(c, 3);
                let life = if c.nargs() >= 5 { int_arg(c, 4) } else { 0 };
                let is_smoke = c.nargs() >= 6 && bool_arg(c, 5);
                let mut rand = LuaFireRand;
                if is_smoke {
                    IsoFire::new_smoke(energy, life, true, &mut rand)
                } else {
                    let fs = FireSquare::default();
                    IsoFire::new_fire(fs, energy, life, &mut rand)
                }
            };
            let id = coords
                .and_then(|(x, y, z)| {
                    with_world(|w| {
                        w.cell
                            .as_mut()?
                            .get_grid_square(x, y, z)
                            .map(|sq| sq.borrow_mut().new_object_id())
                    })
                })
                .unwrap_or(0);
            let obj: IsoObjectRef = Rc::new(RefCell::new(IsoObject::new(
                id,
                ObjectKind::Fire(fire),
                None,
            )));
            remember_class(&obj, ISO_FIRE)?;
            let val = object_value(&obj)?;
            if let Some((x, y, z)) = coords {
                remember_square(&val, [x, y, z]);
            }
            Ok(c.push(val))
        })),
    );
    thread.globals().set_str("IsoFire", Value::Table(t));
}

fn register_iso_fire_manager(thread: &mut Thread, events: Option<EventManager>) {
    let t = TableRef::new();
    {
        t.set_str("FIRE_ANIM_DELAY", Value::Num(FIRE_ANIM_DELAY as f64));
        t.set_str("smokeAnimDelay", Value::Num(SMOKE_ANIM_DELAY as f64));
        t.set_str(
            "FIRE_TINT_MOD",
            Value::Object(LuaColorInfo::new(ColorInfo::new(
                FIRE_TINT_MOD[0],
                FIRE_TINT_MOD[1],
                FIRE_TINT_MOD[2],
                1.0,
            ))),
        );
        t.set_str(
            "smokeTintMod",
            Value::Object(LuaColorInfo::new(ColorInfo::new(
                SMOKE_TINT_MOD[0],
                SMOKE_TINT_MOD[1],
                SMOKE_TINT_MOD[2],
                1.0,
            ))),
        );
        t.set_str("maxFireObjects", Value::Num(MAX_FIRE_OBJECTS as f64));
        t.set_str(
            "getFireAnimDelay",
            Value::Native(NativeFunction::new("IsoFireManager.getFireAnimDelay", |c| {
                Ok(c.push(FIRE_ANIM_DELAY as f64))
            })),
        );
        t.set_str(
            "getSmokeAnimDelay",
            Value::Native(NativeFunction::new(
                "IsoFireManager.getSmokeAnimDelay",
                |c| Ok(c.push(SMOKE_ANIM_DELAY as f64)),
            )),
        );

        let ev_start = events.clone();
        t.set_str(
            "StartFire",
            Value::Native(NativeFunction::new("IsoFireManager.StartFire", move |c| {
                let sq_val = c.arg(1);
                let Some((x, y, z)) = square_coords(&sq_val) else {
                    return Ok(0);
                };
                let _ignite_on_any = bool_arg(c, 2);
                let energy = int_arg(c, 3);
                let life = if c.nargs() >= 5 { int_arg(c, 4) } else { 0 };
                let _ = start_fire_on_square(c.thread, &ev_start, x, y, z, energy, life, false)?;
                Ok(0)
            })),
        );

        let ev_smoke = events.clone();
        t.set_str(
            "StartSmoke",
            Value::Native(NativeFunction::new("IsoFireManager.StartSmoke", move |c| {
                let sq_val = c.arg(1);
                let Some((x, y, z)) = square_coords(&sq_val) else {
                    return Ok(0);
                };
                let _ignite_on_any = bool_arg(c, 2);
                let energy = int_arg(c, 3);
                let life = if c.nargs() >= 5 { int_arg(c, 4) } else { 0 };
                let _ = start_fire_on_square(c.thread, &ev_smoke, x, y, z, energy, life, true)?;
                Ok(0)
            })),
        );

        let ev_explode = events;
        t.set_str(
            "explode",
            Value::Native(NativeFunction::new("IsoFireManager.explode", move |c| {
                let sq_val = c.arg(1);
                let Some((gx, gy, gz)) = square_coords(&sq_val) else {
                    return Ok(0);
                };
                let power = int_arg(c, 2);
                let mut rand = LuaFireRand;
                for dx in -2..=2 {
                    for dy in -2..=2 {
                        for dz in 0..=1 {
                            let (x, y, z) = (gx + dx, gy + dy, gz + dz);
                            if rand.next(100) >= power {
                                continue;
                            }
                            let energy = rand.next_range(100, (250 + power).max(101));
                            let _ = start_fire_on_square(
                                c.thread,
                                &ev_explode,
                                x,
                                y,
                                z,
                                energy,
                                0,
                                false,
                            )?;
                        }
                    }
                }
                Ok(0)
            })),
        );

        t.set_str(
            "RemoveAllOn",
            Value::Native(NativeFunction::new("IsoFireManager.RemoveAllOn", |c| {
                let sq_val = c.arg(0);
                if let Some((x, y, z)) = square_coords(&sq_val) {
                    remove_all_fires_on(x, y, z);
                }
                Ok(0)
            })),
        );

        t.set_str(
            "Remove",
            Value::Native(NativeFunction::new("IsoFireManager.Remove", |c| {
                let fire_val = c.arg(0);
                if let Some(wo) = object_from_value(&fire_val) {
                    let sq_val = object_square(wo)?;
                    let id = wo
                        .object
                        .try_borrow()
                        .map_err(|_| borrowed("IsoFire body"))?
                        .id;
                    if let Some((x, y, z)) = square_coords(&sq_val) {
                        shared_fire_manager()
                            .borrow_mut()
                            .remove(FireRef { x, y, z, id });
                    }
                }
                Ok(0)
            })),
        );

        t.set_str(
            "stopSound",
            Value::Native(NativeFunction::new("IsoFireManager.stopSound", |c| {
                let fire_val = c.arg(0);
                if let Some(wo) = object_from_value(&fire_val) {
                    let sq_val = object_square(wo)?;
                    let id = wo
                        .object
                        .try_borrow()
                        .map_err(|_| borrowed("IsoFire body"))?
                        .id;
                    if let Some((x, y, z)) = square_coords(&sq_val) {
                        shared_fire_manager()
                            .borrow_mut()
                            .remove(FireRef { x, y, z, id });
                    }
                }
                Ok(0)
            })),
        );

        t.set_str(
            "Reset",
            Value::Native(NativeFunction::new("IsoFireManager.Reset", |_c| {
                let mgr = shared_fire_manager();
                let mut mgr = mgr.borrow_mut();
                mgr.fire_stack.clear();
                mgr.burning_characters.clear();
                Ok(0)
            })),
        );
    }
    thread
        .globals()
        .set_str("IsoFireManager", Value::Table(t));
}

pub fn register(thread: &mut Thread, events: Option<EventManager>) {
    register_iso_fire(thread);
    register_iso_fire_manager(thread, events);
}

#[cfg(test)]
mod tests {
    use super::*;
    use pz_kahlua::stdlib::{self, load_string};
    use pz_world::{IsoCell, IsoChunk, IsoGridSquare, IsoSpriteManager};

    fn run(t: &mut Thread, code: &str) -> Vec<Value> {
        let f = load_string(code, Some("bind_fire_test"), t.globals().clone()).unwrap();
        t.pcall(&f, &[])
            .unwrap_or_else(|(e, trace)| panic!("{e:?}\n{trace}"))
    }

    #[test]
    fn iso_fire_and_fire_manager_lua_lifecycle() {
        install_fire_manager(None);
        let mut t = Thread::new(TableRef::new());
        stdlib::install(&mut t, None).unwrap();
        crate::bind_java_util::register(&mut t);
        crate::bind_globals::register(&mut t);
        crate::bind_misc_classes::register(&mut t, None);
        let events = EventManager::register(t.globals(), "Events", crate::events::EVENT_NAMES);
        crate::bind_world::register_with_events(&mut t, events.clone());
        register(&mut t, Some(events));

        let sprites = Rc::new(RefCell::new(IsoSpriteManager::new(
            pz_world::sprite::test_tables(),
        )));
        crate::bind_sprites::install_sprite_manager(Some(sprites));

        let mut cell = IsoCell::default();
        let mut chunk = IsoChunk::new(1, 2);
        chunk.set_square(2, 4, 0, Some(IsoGridSquare::new(10, 20, 0).into()));
        cell.chunks.insert((1, 2), chunk);
        crate::bind_world::set_cell(Some(cell));

        let out = run(
            &mut t,
            r#"
            local function assert(v, msg) if not v then error(msg or "assertion failed") end end
            _G.new_fire_events = 0
            Events.OnNewFire.Add(function(sq)
                _G.new_fire_events = _G.new_fire_events + 1
            end)

            local cell = getCell()
            local sq = cell:getGridSquare(10, 20, 0)
            assert(sq ~= nil)
            assert(not sq:isFire())
            assert(sq:getFire() == nil)

            -- Start a real fire via IsoFireManager.StartFire
            IsoFireManager.StartFire(cell, sq, true, 120, 500)
            assert(sq:isFire())
            local f = sq:getFire()
            assert(f ~= nil)
            assert(instanceof(f, "IsoFire"))
            assert(not f:isPermanent())
            assert(not f:isSmoke())
            assert(f:getLife() == 500)
            assert(f:getEnergy() == 120)
            f:setLightRadius(5)
            assert(f:getLightRadius() == 5)
            assert(_G.new_fire_events == 1)

            -- Create a permanent campfire IsoFire and attach its anim (SCampfireGlobalObject.lua)
            local camp = IsoFire.new(cell, sq)
            assert(instanceof(camp, "IsoFire"))
            assert(camp:isPermanent())
            camp:AttachAnim("Fire", "01", IsoFire.NUM_FRAMES_FIRE, IsoFireManager.FIRE_ANIM_DELAY, 2, -2, true, 0, false, 0.7, IsoFireManager.FIRE_TINT_MOD)

            -- RemoveAllOn extinguishes non-permanent fires
            IsoFireManager.RemoveAllOn(sq)
            assert(not sq:isFire())
            assert(sq:getFire() == nil)

            -- StartSmoke
            IsoFireManager.StartSmoke(cell, sq, true, 50, 300)
            local sm = sq:getFire()
            assert(sm ~= nil and sm:isSmoke())
            sm:extinctFire()
            assert(sq:getFire() == nil)

            return IsoFire.NUM_FRAMES_FIRE, IsoFireManager.FIRE_ANIM_DELAY, IsoFireManager.maxFireObjects
            "#,
        );
        assert_eq!(out[0].as_num(), Some(30.0));
        assert_eq!(out[1].as_num(), Some(0.5));
        assert_eq!(out[2].as_num(), Some(75.0));

        crate::bind_world::set_cell(None);
        crate::bind_sprites::install_sprite_manager(None);
        install_fire_manager(None);
    }
}
