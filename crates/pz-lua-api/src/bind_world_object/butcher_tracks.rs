//! Lua bindings for Build 42.22 animal processing & tracking world objects:
//! - `zombie.iso.IsoButcherHook` (subclass of `IsoObject`)
//! - `zombie.iso.objects.IsoAnimalTrack` (subclass of `IsoObject`)
//! - `zombie.characters.animals.AnimalTracks`

use std::any::Any;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use pz_kahlua::value::LuaObject;
use pz_kahlua::{LuaResult, NativeCall, TableRef, Thread, Value};
use pz_world::objects::{IsoObject, ObjectKind};
use pz_world::save::movers::AnimalTracks;
use pz_world::{IsoDirections, IsoObjectRef};

use super::*;
use crate::bind_character_sim::{character_value, CharacterObject, LuaGameCharacterRef};
use crate::bind_java_util::list_value;
use crate::expose::{bool_arg, enum_value, expose_class, int_arg, str_arg, this};

pub const ISO_BUTCHER_HOOK: &str = "zombie.iso.IsoButcherHook";
pub const ISO_ANIMAL_TRACK: &str = "zombie.iso.objects.IsoAnimalTrack";
pub const ANIMAL_TRACKS_CLASS: &str = "zombie.characters.animals.AnimalTracks";

#[derive(Default)]
struct ButcherHookRuntime {
    animal: Option<LuaGameCharacterRef>,
    removing_blood: bool,
    removing_blood_progress: f32,
    play_removing_blood_sound: bool,
    lua_hook: Option<TableRef>,
}

thread_local! {
    static HOOKS: RefCell<HashMap<usize, ButcherHookRuntime>> = RefCell::new(HashMap::new());
}

/// Clears runtime butcher-hook attachments on session reset.
pub fn reset() {
    HOOKS.with(|h| h.borrow_mut().clear());
}

fn hook_key(obj: &IsoObjectRef) -> usize {
    Rc::as_ptr(obj) as usize
}

// ---- AnimalTracks userdata --------------------------------------------------

#[derive(Clone)]
pub struct LuaAnimalTracks(pub Rc<RefCell<AnimalTracks>>);

impl LuaObject for LuaAnimalTracks {
    fn class_name(&self) -> &str {
        ANIMAL_TRACKS_CLASS
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

pub fn animal_tracks_value(tracks: AnimalTracks) -> Value {
    Value::Object(Rc::new(LuaAnimalTracks(Rc::new(RefCell::new(tracks)))))
}

fn dir_value(dir: Option<i32>) -> Value {
    let Some(ord) = dir else { return Value::Nil };
    let d = IsoDirections::ALL[(ord & 7) as usize];
    enum_value("zombie.iso.IsoDirections", &format!("{d:?}"))
}

fn track_age_hours(added_time: i64) -> i32 {
    if added_time <= 0 {
        return 0;
    }
    let gt = crate::bind_climate::shared_game_time();
    let now_ms = gt.borrow().calendar_time_in_millis();
    let diff = (now_ms - added_time).max(0);
    (diff / 3_600_000) as i32
}

fn freshness_key(hours: i32, tracking_skill: i32) -> &'static str {
    let adjusted = if tracking_skill >= 6 {
        hours
    } else {
        hours + (6 - tracking_skill).max(0) * 2
    };
    if adjusted < 2 {
        "IGUI_AnimalTracks_VeryFresh"
    } else if adjusted < 12 {
        "IGUI_AnimalTracks_Fresh"
    } else if adjusted < 48 {
        "IGUI_AnimalTracks_Old"
    } else {
        "IGUI_AnimalTracks_VeryOld"
    }
}

fn register_animal_tracks(t: &mut Thread) {
    let b = expose_class(t, ANIMAL_TRACKS_CLASS, None);
    b.ctor(|c| Ok(c.push(animal_tracks_value(AnimalTracks::default()))));
    b.static_fn("getTrackStr", |c| {
        let s = str_arg(c, 0).unwrap_or_else(|| Rc::from(""));
        Ok(c.push(Value::str(&format!("IGUI_AnimalTracks_{s}"))))
    });
    b.static_fn("getAndFindNearestTracks", |c| Ok(c.push(list_value(Vec::new()))));
    b.static_fn("getNearestTracks", |c| Ok(c.push(list_value(Vec::new()))));

    b.method("getAnimalType", |c| {
        let o = this::<LuaAnimalTracks>(c)?;
        let tr = o.as_any().downcast_ref::<LuaAnimalTracks>().unwrap().0.borrow();
        Ok(c.push(Value::str(tr.animal_type.as_str())))
    });
    b.method("getTrackType", |c| {
        let o = this::<LuaAnimalTracks>(c)?;
        let tr = o.as_any().downcast_ref::<LuaAnimalTracks>().unwrap().0.borrow();
        Ok(c.push(Value::str(tr.track_type.as_str())))
    });
    b.method("getDir", |c| {
        let o = this::<LuaAnimalTracks>(c)?;
        let tr = o.as_any().downcast_ref::<LuaAnimalTracks>().unwrap().0.borrow();
        Ok(c.push(dir_value(tr.dir)))
    });
    b.method("isAddedToWorld", |c| {
        let o = this::<LuaAnimalTracks>(c)?;
        let tr = o.as_any().downcast_ref::<LuaAnimalTracks>().unwrap().0.borrow();
        Ok(c.push(Value::Bool(tr.added_to_world)))
    });
    b.method("setAddedToWorld", |c| {
        let o = this::<LuaAnimalTracks>(c)?;
        o.as_any()
            .downcast_ref::<LuaAnimalTracks>()
            .unwrap()
            .0
            .borrow_mut()
            .added_to_world = bool_arg(c, 1);
        Ok(0)
    });
    b.method("getTrackHours", |c| {
        let o = this::<LuaAnimalTracks>(c)?;
        let tr = o.as_any().downcast_ref::<LuaAnimalTracks>().unwrap().0.borrow();
        Ok(c.push(Value::Num(track_age_hours(tr.added_time) as f64)))
    });
    b.method("getTrackAgeDays", |c| {
        let o = this::<LuaAnimalTracks>(c)?;
        let tr = o.as_any().downcast_ref::<LuaAnimalTracks>().unwrap().0.borrow();
        Ok(c.push(Value::Num((track_age_hours(tr.added_time) / 24) as f64)))
    });
    b.method("getFreshnessString", |c| {
        let o = this::<LuaAnimalTracks>(c)?;
        let tr = o.as_any().downcast_ref::<LuaAnimalTracks>().unwrap().0.borrow();
        let skill = int_arg(c, 1);
        let key = freshness_key(track_age_hours(tr.added_time), skill);
        Ok(c.push(Value::str(key)))
    });
    b.method("getTrackAge", |c| {
        let o = this::<LuaAnimalTracks>(c)?;
        let tr = o.as_any().downcast_ref::<LuaAnimalTracks>().unwrap().0.borrow();
        let key = freshness_key(track_age_hours(tr.added_time), 0);
        Ok(c.push(Value::str(key)))
    });
    b.method("getTimestamp", |c| {
        let o = this::<LuaAnimalTracks>(c)?;
        let tr = o.as_any().downcast_ref::<LuaAnimalTracks>().unwrap().0.borrow();
        Ok(c.push(Value::str(&tr.added_time.to_string())))
    });
    b.method("getMinSkill", |c| Ok(c.push(Value::Num(0.0))));
    b.method("getTrackSprite", |c| Ok(c.push(Value::Nil)));
    b.method("getTrackItem", |c| Ok(c.push(Value::Nil)));
    b.method("isItem", |c| Ok(c.push(Value::Bool(false))));
    b.method("getItem", |c| Ok(c.push(Value::Nil)));
    b.method("setItem", |_c| Ok(0));
    b.method("getSquare", |c| {
        let o = this::<LuaAnimalTracks>(c)?;
        let tr = o.as_any().downcast_ref::<LuaAnimalTracks>().unwrap().0.borrow();
        Ok(c.push(crate::bind_world::square_value(tr.x, tr.y, 0)))
    });
    b.method("canFindTrack", |c| Ok(c.push(Value::Bool(true))));
    b.method("addTrackingExp", |_c| Ok(0));
    b.method("getAllIsoTracks", |c| Ok(c.push(list_value(Vec::new()))));
    b.method("addToWorld", |c| Ok(c.push(list_value(Vec::new()))));
}

// ---- IsoButcherHook ---------------------------------------------------------

fn construct_butcher_hook(c: &mut NativeCall<'_>) -> LuaResult<usize> {
    let mut body = IsoObject::new(0, ObjectKind::ButcherHook, None);
    let arg0 = c.arg(0);
    if !arg0.is_nil() && !arg0.raw_equals(&crate::bind_world::current_cell()) {
        if let Ok((_, sq)) = construction::get_square(&arg0) {
            body.replace_square(Some(&sq), &mut pz_entity::RetiredComponents::new());
        }
    }
    Ok(c.push(object_value(pz_world::ObjectSlot::new(body).owner_slot())?))
}

fn register_butcher_hook(t: &mut Thread) {
    let b = expose_class(t, ISO_BUTCHER_HOOK, Some(ISO_OBJECT));
    b.static_fn("new", construct_butcher_hook);
    b.method("getObjectName", |c| Ok(c.push(Value::str("ButcherHook"))));
    b.method("setAnimal", |c| {
        let obj = receiver_object(c)?;
        let key = hook_key(&obj);
        let animal = match c.arg(1) {
            Value::Object(o) => o
                .as_any()
                .downcast_ref::<CharacterObject>()
                .map(|co| co.0.clone()),
            _ => None,
        };
        if let Some(a) = animal.as_ref() {
            if let Ok(mut chr) = a.try_borrow_mut() {
                if let Some(f) = chr.animal_fields_mut() {
                    f.animal.on_hook = true;
                }
            }
        }
        HOOKS.with(|m| {
            let mut m = m.borrow_mut();
            let entry = m.entry(key).or_default();
            if animal.is_none() {
                if let Some(prev) = entry.animal.take() {
                    if let Ok(mut chr) = prev.try_borrow_mut() {
                        if let Some(f) = chr.animal_fields_mut() {
                            f.animal.on_hook = false;
                        }
                    }
                }
            } else {
                entry.animal = animal;
            }
        });
        Ok(0)
    });
    b.method("getAnimal", |c| {
        let obj = receiver_object(c)?;
        let key = hook_key(&obj);
        let out = HOOKS.with(|m| {
            m.borrow()
                .get(&key)
                .and_then(|h| h.animal.as_ref())
                .map(character_value)
                .unwrap_or(Value::Nil)
        });
        Ok(c.push(out))
    });
    b.method("reattachAnimal", |c| {
        let obj = receiver_object(c)?;
        let key = hook_key(&obj);
        if let Value::Object(o) = c.arg(1) {
            if let Some(co) = o.as_any().downcast_ref::<CharacterObject>() {
                if let Ok(mut chr) = co.0.try_borrow_mut() {
                    if let Some(f) = chr.animal_fields_mut() {
                        f.animal.on_hook = true;
                    }
                }
                HOOKS.with(|m| m.borrow_mut().entry(key).or_default().animal = Some(co.0.clone()));
            }
        }
        Ok(0)
    });
    b.method("startRemovingBlood", |c| {
        let obj = receiver_object(c)?;
        let key = hook_key(&obj);
        let lua_tbl = c.arg(1).as_table().cloned();
        HOOKS.with(|m| {
            let mut m = m.borrow_mut();
            let h = m.entry(key).or_default();
            h.removing_blood = true;
            h.removing_blood_progress = 0.0;
            if lua_tbl.is_some() {
                h.lua_hook = lua_tbl;
            }
        });
        Ok(0)
    });
    b.method("stopRemovingBlood", |c| {
        let obj = receiver_object(c)?;
        let key = hook_key(&obj);
        HOOKS.with(|m| {
            if let Some(h) = m.borrow_mut().get_mut(&key) {
                h.removing_blood = false;
                h.removing_blood_progress = 0.0;
            }
        });
        Ok(0)
    });
    b.method("isRemovingBlood", |c| {
        let obj = receiver_object(c)?;
        let key = hook_key(&obj);
        let v = HOOKS.with(|m| m.borrow().get(&key).is_some_and(|h| h.removing_blood));
        Ok(c.push(Value::Bool(v)))
    });
    b.method("getRemovingBloodProgress", |c| {
        let obj = receiver_object(c)?;
        let key = hook_key(&obj);
        let v = HOOKS.with(|m| {
            m.borrow()
                .get(&key)
                .map(|h| h.removing_blood_progress)
                .unwrap_or(0.0)
        });
        Ok(c.push(Value::Num(v as f64)))
    });
    b.method("setPlayRemovingBloodSound", |c| {
        let obj = receiver_object(c)?;
        let key = hook_key(&obj);
        let play = bool_arg(c, 1);
        HOOKS.with(|m| m.borrow_mut().entry(key).or_default().play_removing_blood_sound = play);
        Ok(0)
    });
    b.method("setLuaHook", |c| {
        let obj = receiver_object(c)?;
        let key = hook_key(&obj);
        let tbl = c.arg(1).as_table().cloned();
        HOOKS.with(|m| m.borrow_mut().entry(key).or_default().lua_hook = tbl);
        Ok(0)
    });
    b.method("update", |c| {
        let obj = receiver_object(c)?;
        let key = hook_key(&obj);
        HOOKS.with(|m| {
            if let Some(h) = m.borrow_mut().get_mut(&key) {
                if h.removing_blood {
                    h.removing_blood_progress = (h.removing_blood_progress + 0.01).min(1.0);
                    if h.removing_blood_progress >= 1.0 {
                        h.removing_blood = false;
                    }
                }
            }
        });
        Ok(0)
    });
    b.method("updateAnimalModel", |_c| Ok(0));
    b.method("playPutDownCorpseSound", |_c| Ok(0));
    b.method("removeHook", |c| {
        let obj = receiver_object(c)?;
        let key = hook_key(&obj);
        HOOKS.with(|m| {
            if let Some(mut h) = m.borrow_mut().remove(&key) {
                if let Some(a) = h.animal.take() {
                    if let Ok(mut chr) = a.try_borrow_mut() {
                        if let Some(f) = chr.animal_fields_mut() {
                            f.animal.on_hook = false;
                        }
                    }
                }
            }
        });
        Ok(0)
    });
    b.method("removeFromWorld", |c| {
        let obj = receiver_object(c)?;
        let key = hook_key(&obj);
        HOOKS.with(|m| {
            m.borrow_mut().remove(&key);
        });
        Ok(0)
    });
}

// ---- IsoAnimalTrack ---------------------------------------------------------

fn construct_animal_track(c: &mut NativeCall<'_>) -> LuaResult<usize> {
    let mut tracks = AnimalTracks::default();
    let mut sq_opt = None;
    let mut sprite_name = None;
    if c.nargs() >= 3 {
        if let Ok((_, sq)) = construction::get_square(&c.arg(0)) {
            if let Ok(s) = sq.try_borrow() {
                tracks.x = s.x;
                tracks.y = s.y;
            }
            sq_opt = Some(sq);
        }
        sprite_name = str_arg(c, 1).map(|s| s.to_string());
        if let Value::Object(o) = c.arg(2) {
            if let Some(lt) = o.as_any().downcast_ref::<LuaAnimalTracks>() {
                tracks = lt.0.borrow().clone();
            }
        }
    }
    let mut body = IsoObject::new(0, ObjectKind::AnimalTrack(Box::new(tracks)), None);
    if let Some(sp) = sprite_name {
        body.sprite_name = Some(pz_world::objects::SpriteName::Str(sp));
    }
    if let Some(sq) = sq_opt {
        body.replace_square(Some(&sq), &mut pz_entity::RetiredComponents::new());
    }
    Ok(c.push(object_value(pz_world::ObjectSlot::new(body).owner_slot())?))
}

fn register_iso_animal_track(t: &mut Thread) {
    let b = expose_class(t, ISO_ANIMAL_TRACK, Some(ISO_OBJECT));
    b.static_fn("new", construct_animal_track);
    b.method("getObjectName", |c| Ok(c.push(Value::str("IsoAnimalTrack"))));
    b.method("getAnimalTracks", |c| {
        let obj = receiver_object(c)?;
        let body = obj
            .try_borrow()
            .map_err(|_| borrowed("IsoAnimalTrack.getAnimalTracks"))?;
        let out = match &body.kind {
            ObjectKind::AnimalTrack(tr) => animal_tracks_value((**tr).clone()),
            _ => Value::Nil,
        };
        Ok(c.push(out))
    });
    b.method("glow", |_c| Ok(0));
    b.method("stopGlow", |_c| Ok(0));
}

pub(super) fn register(t: &mut Thread) {
    register_animal_tracks(t);
    register_butcher_hook(t);
    register_iso_animal_track(t);
}

#[cfg(test)]
mod tests {
    use super::*;
    use pz_kahlua::TableRef;

    #[test]
    fn butcher_hook_and_animal_tracks_round_trip_from_lua() {
        reset();
        pz_world::object_slot::install_object_observer(Some(crate::component_gc::observe_object));
        let mut t = Thread::new(TableRef::new());
        pz_kahlua::stdlib::install(&mut t, None).unwrap();
        crate::bind_java_util::register(&mut t);
        crate::bind_character_sim::register_shared(&mut t);
        crate::bind_world_object::register_classes(&mut t);

        let tracks = AnimalTracks {
            animal_type: "deer".to_string(),
            track_type: "footstep".to_string(),
            x: 15,
            y: 25,
            dir: Some(IsoDirections::N as i32),
            added_time: 0,
            added_to_world: false,
        };
        t.globals()
            .set_str("TEST_TRACKS", animal_tracks_value(tracks));

        let src = r#"
            local function assert(v, msg) if not v then error(msg or "assertion failed") end end

            local hook = IsoButcherHook.new(nil)
            assert(hook ~= nil and hook:getObjectName() == "ButcherHook")
            assert(hook:getAnimal() == nil)
            assert(hook:isRemovingBlood() == false)
            hook:startRemovingBlood(nil)
            assert(hook:isRemovingBlood() == true)
            hook:update()
            assert(hook:getRemovingBloodProgress() > 0)
            hook:stopRemovingBlood()
            assert(hook:isRemovingBlood() == false and hook:getRemovingBloodProgress() == 0)
            hook:removeHook()

            assert(TEST_TRACKS:getAnimalType() == "deer")
            assert(TEST_TRACKS:getTrackType() == "footstep")
            assert(TEST_TRACKS:isAddedToWorld() == false)
            TEST_TRACKS:setAddedToWorld(true)
            assert(TEST_TRACKS:isAddedToWorld() == true)
            assert(TEST_TRACKS:getTrackAgeDays() == 0)
            assert(TEST_TRACKS:getFreshnessString(8) == "IGUI_AnimalTracks_VeryFresh")
            assert(AnimalTracks.getTrackStr("footstep") == "IGUI_AnimalTracks_footstep")

            local isoTrack = IsoAnimalTrack.new(nil, "tr_01", TEST_TRACKS)
            assert(isoTrack ~= nil and isoTrack:getObjectName() == "IsoAnimalTrack")
            local got = isoTrack:getAnimalTracks()
            assert(got ~= nil and got:getAnimalType() == "deer" and got:isAddedToWorld() == true)
            isoTrack:glow(nil)
            isoTrack:stopGlow(nil)
            return true
        "#;
        let cl = pz_kahlua::stdlib::load_string(src, Some("butcher_tracks_test"), t.globals().clone())
            .unwrap();
        let res = t
            .pcall(&cl, &[])
            .unwrap_or_else(|e| panic!("lua error: {e:?}"));
        assert!(matches!(res.first(), Some(Value::Bool(true))));
        reset();
    }
}
