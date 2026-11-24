//! Build 42.22 `zombie.GameSounds`, `zombie.audio.GameSound`, and
//! `zombie.worldMap.WorldMapVisited` Lua bindings over live `pz_sound`
//! (`SoundSystem.sounds`) and `pz_worldmap::visited::WorldMapVisited`.

use std::any::Any;
use std::cell::RefCell;
use std::rc::Rc;

use pz_kahlua::value::LuaObject;
use pz_kahlua::{LuaError, Thread, Value};
use pz_scripts::objects::models::{GameSoundScript, MasterVolume};
use pz_sound::game_sounds::{set_user_volume, user_volume};
use pz_sound::system::SoundSystem;
use pz_worldmap::visited::{BIT_KNOWN, BIT_VISITED};

use crate::bind_audio::sound_system;
use crate::bind_java_util::list_value;
use crate::expose::{bool_arg, expose_class, int_arg, num_arg, str_arg, with_this};
use crate::worldmap::visited;

pub const GAME_SOUNDS_CLASS: &str = "zombie.GameSounds";
pub const GAME_SOUND_CLASS: &str = "zombie.audio.GameSound";
pub const WORLD_MAP_VISITED_CLASS: &str = "zombie.worldMap.WorldMapVisited";

struct LuaGameSound(Rc<RefCell<GameSoundScript>>);

impl LuaObject for LuaGameSound {
    fn class_name(&self) -> &str {
        GAME_SOUND_CLASS
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

struct LuaWorldMapVisited;

impl LuaObject for LuaWorldMapVisited {
    fn class_name(&self) -> &str {
        WORLD_MAP_VISITED_CLASS
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

fn game_sound_value(s: Rc<RefCell<GameSoundScript>>) -> Value {
    Value::Object(Rc::new(LuaGameSound(s)))
}

fn master_name(m: MasterVolume) -> &'static str {
    match m {
        MasterVolume::Primary => "Primary",
        MasterVolume::Ambient => "Ambient",
        MasterVolume::Music => "Music",
        MasterVolume::VehicleEngine => "VehicleEngine",
    }
}

pub fn register(thread: &mut Thread) {
    // zombie.audio.GameSound
    let gs = expose_class(thread, GAME_SOUND_CLASS, Some("java.lang.Object"));
    gs.method("getName", |c| {
        let name = with_this::<LuaGameSound, _>(c, |s| s.0.borrow().game_sound.name.clone())?;
        Ok(c.push(name.map_or(Value::Nil, |s| Value::str(&s))))
    });
    gs.method("getCategory", |c| {
        let cat = with_this::<LuaGameSound, _>(c, |s| s.0.borrow().game_sound.category.clone())?;
        Ok(c.push(cat))
    });
    gs.method("isLooped", |c| {
        let looped = with_this::<LuaGameSound, _>(c, |s| s.0.borrow().game_sound.loop_)?;
        Ok(c.push(looped))
    });
    gs.method("is3D", |c| {
        let is3d = with_this::<LuaGameSound, _>(c, |s| s.0.borrow().game_sound.is3d)?;
        Ok(c.push(is3d))
    });
    gs.method("getUserVolume", |c| {
        let sys_rc = sound_system();
        let sys = sys_rc.borrow();
        let vol = with_this::<LuaGameSound, _>(c, |s| {
            user_volume(&sys.sounds.env, &s.0.borrow().game_sound)
        })?;
        Ok(c.push(f64::from(vol)))
    });
    gs.method("setUserVolume", |c| {
        let gain = num_arg(c, 1) as f32;
        with_this::<LuaGameSound, _>(c, |s| {
            set_user_volume(&mut s.0.borrow_mut().game_sound, gain);
        })?;
        Ok(0)
    });
    gs.method("getMasterName", |c| {
        let m = with_this::<LuaGameSound, _>(c, |s| master_name(s.0.borrow().game_sound.master))?;
        Ok(c.push(m))
    });
    gs.method("getNumClips", |c| {
        let n = with_this::<LuaGameSound, _>(c, |s| s.0.borrow().game_sound.clips.len())?;
        Ok(c.push(n as f64))
    });
    gs.method("getMaxDistanceOfClips", |c| {
        let d = with_this::<LuaGameSound, _>(c, |s| {
            s.0.borrow().game_sound.max_distance_of_clips()
        })?;
        Ok(c.push(f64::from(d)))
    });
    gs.method("reset", |c| {
        with_this::<LuaGameSound, _>(c, |s| s.0.borrow_mut().game_sound.reset())?;
        Ok(0)
    });

    // zombie.GameSounds
    let mgr = expose_class(thread, GAME_SOUNDS_CLASS, Some("java.lang.Object"));
    mgr.static_fn("isKnownSound", |c| {
        let ok = str_arg(c, 0)
            .is_some_and(|name| sound_system().borrow().sounds.is_known_sound(&name));
        Ok(c.push(ok))
    });
    for fn_name in ["getSound", "getOrCreateSound"] {
        mgr.static_fn(fn_name, |c| {
            let Some(name) = str_arg(c, 0) else {
                return Ok(c.push(Value::Nil));
            };
            let sys_rc = sound_system();
            let mut sys = sys_rc.borrow_mut();
            let SoundSystem {
                sounds,
                backend,
                media_file_exists,
                ..
            } = &mut *sys;
            let s = sounds.get_or_create_sound(&name, backend.as_mut(), media_file_exists.as_ref());
            Ok(c.push(s.map_or(Value::Nil, game_sound_value)))
        });
    }
    mgr.static_fn("addSound", |c| {
        let Value::Object(obj) = c.arg(0) else {
            return Ok(0);
        };
        let Some(gs) = obj.as_any().downcast_ref::<LuaGameSound>() else {
            return Ok(0);
        };
        let sys_rc = sound_system();
        let mut sys = sys_rc.borrow_mut();
        let SoundSystem {
            sounds, backend, ..
        } = &mut *sys;
        sounds.add_sound(gs.0.clone(), backend.as_mut());
        Ok(0)
    });
    mgr.static_fn("getCategories", |c| {
        let cats = sound_system().borrow().sounds.categories();
        Ok(c.push(list_value(
            cats.into_iter().map(|s| Value::str(&s)).collect(),
        )))
    });
    mgr.static_fn("getSoundsInCategory", |c| {
        let cat = str_arg(c, 0).map(|s| s.to_string()).unwrap_or_default();
        let items = sound_system()
            .borrow()
            .sounds
            .sounds_in_category(&cat)
            .into_iter()
            .map(game_sound_value)
            .collect();
        Ok(c.push(list_value(items)))
    });
    mgr.static_fn("loadINI", |_| {
        sound_system().borrow_mut().sounds.load_ini();
        Ok(0)
    });
    mgr.static_fn("saveINI", |_| {
        sound_system().borrow_mut().sounds.save_ini();
        Ok(0)
    });
    mgr.static_fn("previewSound", |c| {
        if let Some(name) = str_arg(c, 0) {
            let sys_rc = sound_system();
            let mut sys = sys_rc.borrow_mut();
            let SoundSystem {
                sounds,
                backend,
                rand_next,
                ..
            } = &mut *sys;
            sounds.preview_sound(&name, backend.as_mut(), rand_next.as_mut());
        }
        Ok(0)
    });
    mgr.static_fn("stopPreview", |_| {
        let sys_rc = sound_system();
        let mut sys = sys_rc.borrow_mut();
        let SoundSystem {
            sounds, backend, ..
        } = &mut *sys;
        sounds.stop_preview(backend.as_mut());
        Ok(0)
    });
    mgr.static_fn("isPreviewPlaying", |c| {
        let sys_rc = sound_system();
        let mut sys = sys_rc.borrow_mut();
        let SoundSystem {
            sounds, backend, ..
        } = &mut *sys;
        let playing = sounds.is_preview_playing(backend.as_mut());
        Ok(c.push(playing))
    });
    mgr.static_fn("fix3DListenerPosition", |c| {
        let in_menu = bool_arg(c, 0);
        let sys_rc = sound_system();
        let mut sys = sys_rc.borrow_mut();
        let SoundSystem {
            sounds, backend, ..
        } = &mut *sys;
        sounds.fix_3d_listener_position(in_menu, &[], backend.as_mut());
        Ok(0)
    });
    mgr.static_fn("Reset", |_| {
        let sys_rc = sound_system();
        let mut sys = sys_rc.borrow_mut();
        let SoundSystem {
            sounds, backend, ..
        } = &mut *sys;
        sounds.reset(backend.as_mut());
        Ok(0)
    });

    // zombie.worldMap.WorldMapVisited
    let wmv = expose_class(thread, WORLD_MAP_VISITED_CLASS, Some("java.lang.Object"));
    wmv.static_fn("getInstance", |c| {
        let _ = visited::with_instance(|_| {});
        Ok(c.push(Value::Object(Rc::new(LuaWorldMapVisited))))
    });
    wmv.static_fn("update", |_| {
        let _ = visited::update();
        Ok(0)
    });
    wmv.static_fn("SaveAll", |_| {
        visited::save_all();
        Ok(0)
    });
    wmv.static_fn("Reset", |_| {
        visited::reset();
        Ok(0)
    });
    wmv.method("setBounds", |c| {
        let (x0, y0, x1, y1) = (int_arg(c, 1), int_arg(c, 2), int_arg(c, 3), int_arg(c, 4));
        visited::with_instance(|v| {
            let _ = v.set_bounds(x0, y0, x1, y1);
        })
        .map_err(LuaError::runtime)?;
        Ok(0)
    });
    wmv.method("getMinX", |c| {
        let v = visited::with_instance(|v| v.get_min_x()).unwrap_or(0);
        Ok(c.push(v as f64))
    });
    wmv.method("getMinY", |c| {
        let v = visited::with_instance(|v| v.get_min_y()).unwrap_or(0);
        Ok(c.push(v as f64))
    });
    wmv.method("getVisitedLength", |c| {
        let v = visited::with_instance(|v| v.visited.len()).unwrap_or(0);
        Ok(c.push(v as f64))
    });
    wmv.method("setKnownInCells", |c| {
        let (x0, y0, x1, y1) = (int_arg(c, 1), int_arg(c, 2), int_arg(c, 3), int_arg(c, 4));
        let _ = visited::with_instance(|v| v.set_known_in_cells(x0, y0, x1, y1));
        Ok(0)
    });
    wmv.method("clearKnownInCells", |c| {
        let (x0, y0, x1, y1) = (int_arg(c, 1), int_arg(c, 2), int_arg(c, 3), int_arg(c, 4));
        let _ = visited::with_instance(|v| v.clear_known_in_cells(x0, y0, x1, y1));
        Ok(0)
    });
    wmv.method("setVisitedInCells", |c| {
        let (x0, y0, x1, y1) = (int_arg(c, 1), int_arg(c, 2), int_arg(c, 3), int_arg(c, 4));
        let _ = visited::with_instance(|v| v.set_visited_in_cells(x0, y0, x1, y1));
        Ok(0)
    });
    wmv.method("clearVisitedInCells", |c| {
        let (x0, y0, x1, y1) = (int_arg(c, 1), int_arg(c, 2), int_arg(c, 3), int_arg(c, 4));
        let _ = visited::with_instance(|v| v.clear_visited_in_cells(x0, y0, x1, y1));
        Ok(0)
    });
    wmv.method("setKnownInSquares", |c| {
        let (x0, y0, x1, y1) = (int_arg(c, 1), int_arg(c, 2), int_arg(c, 3), int_arg(c, 4));
        let _ = visited::with_instance(|v| v.set_flags(x0, y0, x1, y1, BIT_KNOWN));
        Ok(0)
    });
    wmv.method("clearKnownInSquares", |c| {
        let (x0, y0, x1, y1) = (int_arg(c, 1), int_arg(c, 2), int_arg(c, 3), int_arg(c, 4));
        let _ = visited::with_instance(|v| v.clear_flags(x0, y0, x1, y1, BIT_KNOWN));
        Ok(0)
    });
    wmv.method("setVisitedInSquares", |c| {
        let (x0, y0, x1, y1) = (int_arg(c, 1), int_arg(c, 2), int_arg(c, 3), int_arg(c, 4));
        let _ = visited::with_instance(|v| v.set_flags(x0, y0, x1, y1, BIT_VISITED));
        Ok(0)
    });
    wmv.method("clearVisitedInSquares", |c| {
        let (x0, y0, x1, y1) = (int_arg(c, 1), int_arg(c, 2), int_arg(c, 3), int_arg(c, 4));
        let _ = visited::with_instance(|v| v.clear_flags(x0, y0, x1, y1, BIT_VISITED));
        Ok(0)
    });
    wmv.method("isKnown", |c| {
        let (x0, y0) = (int_arg(c, 1), int_arg(c, 2));
        let (x1, y1) = if c.nargs() >= 5 {
            (int_arg(c, 3), int_arg(c, 4))
        } else {
            (x0, y0)
        };
        let known = visited::with_instance(|v| v.is_known(x0, y0, x1, y1)).unwrap_or(false);
        Ok(c.push(known))
    });
    wmv.method("isVisited", |c| {
        let (x0, y0) = (int_arg(c, 1), int_arg(c, 2));
        let (x1, y1) = if c.nargs() >= 5 {
            (int_arg(c, 3), int_arg(c, 4))
        } else {
            (x0, y0)
        };
        let vis = visited::with_instance(|v| v.is_visited(x0, y0, x1, y1)).unwrap_or(false);
        Ok(c.push(vis))
    });
    wmv.method("forget", |_| {
        let _ = visited::with_instance(|v| v.forget());
        Ok(0)
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use pz_kahlua::TableRef;

    #[test]
    fn game_sounds_and_world_map_visited_round_trip_from_lua() {
        let mut t = Thread::new(TableRef::new());
        pz_kahlua::stdlib::install(&mut t, None).unwrap();
        crate::bind_java_util::register(&mut t);
        register(&mut t);
        sound_system().borrow_mut().sounds.env.advanced_sound_options = true;
        crate::bind_world::set_meta_grid(Some(pz_world::IsoMetaGrid::with_bounds(0, 0, 2, 2)));

        let src = r#"
            local function assert(v, msg) if not v then error(msg or "assertion failed") end end

            GameSounds.Reset()
            assert(GameSounds.isKnownSound("TestClipSound") == false)
            local s = GameSounds.getOrCreateSound("TestClipSound")
            assert(s ~= nil and s:getName() == "TestClipSound")
            assert(GameSounds.isKnownSound("TestClipSound") == true)
            assert(s:getCategory() == "AUTO")
            s:setUserVolume(0.5)
            assert(s:getUserVolume() == 0.5)
            assert(GameSounds.getCategories():size() >= 1)
            assert(GameSounds.getSoundsInCategory("AUTO"):size() >= 1)
            assert(GameSounds.isPreviewPlaying() == false)
            GameSounds.stopPreview()
            GameSounds.fix3DListenerPosition(true)

            WorldMapVisited.Reset()
            local wv = WorldMapVisited.getInstance()
            wv:setBounds(0, 0, 2, 2)
            assert(wv:getMinX() == 0 and wv:getMinY() == 0)
            assert(wv:isKnown(10, 10) == false and wv:isVisited(10, 10) == false)
            wv:setKnownInSquares(0, 0, 64, 64)
            assert(wv:isKnown(10, 10) == true and wv:isVisited(10, 10) == false)
            wv:setVisitedInSquares(0, 0, 64, 64)
            assert(wv:isVisited(10, 10) == true)
            wv:forget()
            assert(wv:isKnown(10, 10) == false and wv:isVisited(10, 10) == false)
            WorldMapVisited.Reset()
            GameSounds.Reset()
            return true
        "#;
        let cl = pz_kahlua::stdlib::load_string(
            src,
            Some("bind_game_sounds_visited_test"),
            t.globals().clone(),
        )
        .unwrap();
        let res = t
            .pcall(&cl, &[])
            .unwrap_or_else(|e| panic!("lua error: {e:?}"));
        crate::bind_world::set_meta_grid(None);
        assert!(matches!(res.first(), Some(Value::Bool(true))));
    }
}
