//! `IsoZombie`'s members Lua uses (`setUseless`, `isFakeDead`,
//! `setFakeDead`, `toggleCrawling`, `knockDown`, `getTarget`, `isCanWalk`,
//! `setAlwaysKnockedDown`, `DoZombieStats`, ...), the zombie lore getters
//! (`IsoZombie.getSpeedType`, `strength`, `memory`, ...), and
//! `WorldSoundManager` (`getWorldSoundManager()`, the global `addSound`,
//! `WorldSoundManager:addSound/addSoundRepeating/getStressFromSounds`)
//! over `pz_audio::world_sound`.
//!
//! The sound manager is one shared instance with the cell installed by
//! the world's owner ([`install_world_sounds`]); without one the cell is
//! empty (`pz_audio::world_sound::NoWorld`).

use crate::bind_character_sim::{chr_of, with_ctx, CharacterObject, ISO_GAME_CHARACTER_CLASS, ISO_ZOMBIE_CLASS};
use crate::bind_states::with_state_cx;
use crate::expose::{bool_arg, expose_class, global, int_arg, num_arg, str_arg};
use pz_audio::world_sound::{NoWorld, SoundId, SoundSource, SoundWorld, WorldSoundManager};
use pz_kahlua::value::LuaObject;
use pz_kahlua::{NativeCall, Thread, Value};
use std::any::Any;
use std::cell::RefCell;
use std::rc::Rc;

pub const WORLD_SOUND_MANAGER_CLASS: &str = "zombie.WorldSoundManager";
pub const WORLD_SOUND_CLASS: &str = "zombie.WorldSoundManager$WorldSound";

type SharedSounds = Rc<RefCell<WorldSoundManager>>;
type SharedSoundWorld = Rc<RefCell<dyn SoundWorld>>;

thread_local! {
    static SOUNDS: RefCell<SharedSounds> = RefCell::new(Rc::new(RefCell::new(WorldSoundManager::new())));
    static SOUND_WORLD: RefCell<SharedSoundWorld> = RefCell::new(Rc::new(RefCell::new(NoWorld)));
}

/// Installs the shared `WorldSoundManager.instance` and the cell it reads.
pub fn install_world_sounds(sounds: SharedSounds, world: SharedSoundWorld) {
    SOUNDS.with(|s| *s.borrow_mut() = sounds);
    SOUND_WORLD.with(|w| *w.borrow_mut() = world);
}

/// `WorldSoundManager.instance`.
pub fn shared_world_sounds() -> SharedSounds {
    SOUNDS.with(|s| s.borrow().clone())
}

/// The `WorldSoundManager` singleton object.
struct ManagerObject;

impl LuaObject for ManagerObject {
    fn class_name(&self) -> &str {
        WORLD_SOUND_MANAGER_CLASS
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// A `WorldSound` handle.
pub struct WorldSoundObject(pub SoundId);

impl LuaObject for WorldSoundObject {
    fn class_name(&self) -> &str {
        WORLD_SOUND_CLASS
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

thread_local! {
    static MANAGER: Value = Value::Object(Rc::new(ManagerObject));
}

/// A Lua value as a sound source: a character by its stable id; any other
/// object by identity.
fn source_of(v: &Value) -> Option<SoundSource> {
    match v {
        Value::Object(o) => {
            if let Some(c) = o.as_any().downcast_ref::<CharacterObject>() {
                return Some(c.0.borrow().sound_source());
            }
            Some(SoundSource { id: (Rc::as_ptr(o) as *const () as usize as u64) | (1 << 63), ..Default::default() })
        }
        _ => None,
    }
}

fn sound_value(id: Option<SoundId>) -> Value {
    id.map_or(Value::Nil, |id| Value::Object(Rc::new(WorldSoundObject(id))))
}

/// `addSound(source, x, y, z, radius, volume[, stressHumans[,
/// zombieIgnoreDist, stressMod]])` starting at argument `a`.
fn add_sound(c: &mut NativeCall<'_>, a: usize, repeating: bool) -> Value {
    let source = source_of(&c.arg(a));
    let (x, y, z) = (int_arg(c, a + 1), int_arg(c, a + 2), int_arg(c, a + 3));
    let (radius, volume) = (int_arg(c, a + 4), int_arg(c, a + 5));
    let n = c.nargs();
    let stress = n > a + 6 && bool_arg(c, a + 6);
    let (ignore, stress_mod) = if n > a + 8 { (num_arg(c, a + 7) as f32, num_arg(c, a + 8) as f32) } else { (0.0, 1.0) };
    let sounds = shared_world_sounds();
    let world = SOUND_WORLD.with(|w| w.borrow().clone());
    let mut world = world.borrow_mut();
    let mut m = sounds.borrow_mut();
    let id = if repeating {
        m.add_sound_repeating(&mut *world, source, x, y, z, radius, volume, stress, ignore, stress_mod)
    } else {
        m.add_sound_stress(&mut *world, source, x, y, z, radius, volume, stress, ignore, stress_mod)
    };
    sound_value(id)
}

macro_rules! zfield {
    ($b:expr, $get:literal, $set:literal, $($f:ident).+, bool) => {
        $b.method($get, |c| {
            let v = chr_of(c)?.borrow().$($f).+;
            Ok(c.push(v))
        });
        $b.method($set, |c| {
            let v = bool_arg(c, 1);
            chr_of(c)?.borrow_mut().$($f).+ = v;
            Ok(0)
        });
    };
    ($b:expr, $get:literal, $($f:ident).+, int) => {
        $b.method($get, |c| {
            let v = chr_of(c)?.borrow().$($f).+;
            Ok(c.push(Value::Num(f64::from(v))))
        });
    };
}

pub fn register(thread: &mut Thread) {
    let b = expose_class(thread, ISO_ZOMBIE_CLASS, Some(ISO_GAME_CHARACTER_CLASS));
    zfield!(b, "isUseless", "setUseless", zombie.useless, bool);
    zfield!(b, "isCanWalk", "setCanWalk", zombie.can_walk, bool);
    zfield!(b, "isCanCrawlUnderVehicle", "setCanCrawlUnderVehicle", zombie.can_crawl_under_vehicle, bool);
    zfield!(b, "isAlwaysKnockedDown", "setAlwaysKnockedDown", vars.always_knocked_down, bool);
    zfield!(b, "isOnlyJawStab", "setOnlyJawStab", vars.only_jaw_stab, bool);
    zfield!(b, "isNoTeeth", "setNoTeeth", zombie.no_teeth, bool);
    zfield!(b, "isForceEatingAnimation", "setForceEatingAnimation", zombie.force_eating_animation, bool);
    zfield!(b, "isForceFakeDead", "setForceFakeDead", zombie.force_fake_dead, bool);
    zfield!(b, "isReanimate", "setReanimate", zombie.reanimate, bool);
    zfield!(b, "isReanimatedPlayer", "setReanimatedPlayer", zombie.reanimated_player, bool);
    zfield!(b, "wasFakeDead", "setWasFakeDead", zombie.was_fake_dead, bool);
    zfield!(b, "isSkeleton", "setSkeleton", zombie.is_skeleton, bool);
    zfield!(b, "isStaggerBack", "setStaggerBack", vars.stagger_back, bool);
    zfield!(b, "isKnifeDeath", "setKnifeDeath", zombie.knife_death, bool);
    zfield!(b, "isJawStabAttach", "setJawStabAttach", zombie.jaw_stab_attach, bool);
    zfield!(b, "isSitAgainstWall", "setSitAgainstWall", vars.sit_against_wall, bool);
    zfield!(b, "isHitLegsWhileOnFloor", "setHitLegsWhileOnFloor", vars.hit_legs_while_on_floor, bool);
    zfield!(b, "shouldDressInRandomOutfit", "setDressInRandomOutfit", zombie.dress_in_random_outfit, bool);
    b.method("isReanimatedForGrappleOnly", |c| {
        let value = chr_of(c)?.borrow().zombie.is_reanimated_for_grapple_only();
        Ok(c.push(value))
    });
    b.method("setReanimatedForGrappleOnly", |c| {
        let value = bool_arg(c, 1);
        chr_of(c)?.borrow().zombie.set_reanimated_for_grapple_only(value);
        Ok(0)
    });
    zfield!(b, "getSpeedType", zombie.speed_type, int);
    zfield!(b, "getCrawlerType", zombie.crawler_type, int);
    zfield!(b, "getThumpTimer", zombie.thump_timer, int);
    zfield!(b, "getHitTime", vars.hit_time, int);
    zfield!(b, "getHitHeadWhileOnFloor", vars.hit_head_while_on_floor, int);
    b.method("setImmortalTutorialZombie", |c| {
        let v = bool_arg(c, 1);
        chr_of(c)?.borrow_mut().zombie.immortal_tutorial_zombie = v;
        Ok(0)
    });
    b.method("setCrawlerType", |c| {
        let v = int_arg(c, 1);
        chr_of(c)?.borrow_mut().zombie.crawler_type = v;
        Ok(0)
    });
    b.method("setCrawler", |c| {
        let v = bool_arg(c, 1);
        chr_of(c)?.borrow_mut().vars.crawling = v;
        Ok(0)
    });
    b.method("isCrawling", |c| {
        let v = chr_of(c)?.borrow().vars.crawling;
        Ok(c.push(v))
    });
    b.method("isBecomeCrawler", |c| {
        let v = chr_of(c)?.borrow().vars.become_crawler;
        Ok(c.push(v))
    });
    b.method("setBecomeCrawler", |c| {
        let v = bool_arg(c, 1);
        let this = c.arg(0);
        let chr = chr_of(c)?;
        with_ctx(c, &this, |ctx| {
            chr.borrow_mut().set_become_crawler(ctx, v);
            Ok(())
        })?;
        Ok(0)
    });
    b.method("isFakeDead", |c| {
        let this = c.arg(0);
        let chr = chr_of(c)?;
        let v = with_ctx(c, &this, |ctx| Ok(chr.borrow_mut().is_fake_dead(ctx, false)))?;
        Ok(c.push(v))
    });
    b.method("setFakeDead", |c| {
        let v = bool_arg(c, 1);
        let this = c.arg(0);
        let chr = chr_of(c)?;
        with_ctx(c, &this, |ctx| {
            chr.borrow_mut().set_fake_dead(ctx, v);
            Ok(())
        })?;
        Ok(0)
    });
    b.method("toggleCrawling", |c| {
        let this = c.arg(0);
        let chr = chr_of(c)?;
        with_ctx(c, &this, |ctx| {
            chr.borrow_mut().toggle_crawling(ctx);
            Ok(())
        })?;
        Ok(0)
    });
    b.method("DoZombieStats", |c| {
        let this = c.arg(0);
        let chr = chr_of(c)?;
        with_ctx(c, &this, |ctx| {
            chr.borrow_mut().do_zombie_stats(ctx);
            Ok(())
        })?;
        Ok(0)
    });
    b.method("doZombieSpeed", |c| {
        let this = c.arg(0);
        let speed = (c.nargs() >= 2).then(|| int_arg(c, 1));
        let chr = chr_of(c)?;
        with_ctx(c, &this, |ctx| {
            let mut z = chr.borrow_mut();
            match speed {
                Some(s) => z.do_zombie_speed_with(ctx, s),
                None => z.do_zombie_speed(ctx),
            }
            Ok(())
        })?;
        Ok(0)
    });
    b.method("makeInactive", |c| {
        let v = bool_arg(c, 1);
        let this = c.arg(0);
        let chr = chr_of(c)?;
        with_ctx(c, &this, |ctx| {
            chr.borrow_mut().make_inactive(ctx, v);
            Ok(())
        })?;
        Ok(0)
    });
    b.method("getWalkType", |c| {
        let v = chr_of(c)?.borrow().zombie.walk_type.clone();
        Ok(c.push(v.map_or(Value::Nil, |s| Value::str(&s))))
    });
    b.method("setWalkType", |c| {
        let v = str_arg(c, 1).map(|s| s.to_string());
        chr_of(c)?.borrow_mut().zombie.walk_type = v;
        Ok(0)
    });
    b.method("setSpeedTypeFromWalkType", |c| {
        let chr = chr_of(c)?;
        let mut z = chr.borrow_mut();
        z.zombie.speed_type = pz_characters::GameCharacter::<crate::bind_characters::LuaItem>::speed_type_from_walk_type(z.zombie.walk_type.as_deref());
        Ok(0)
    });
    b.static_fn("getSpeedTypeFromWalkType", |c| {
        let w = str_arg(c, 0).map(|s| s.to_string());
        let v = pz_characters::GameCharacter::<crate::bind_characters::LuaItem>::speed_type_from_walk_type(w.as_deref());
        Ok(c.push(Value::Num(f64::from(v))))
    });
    b.method("knockDown", |c| {
        let v = bool_arg(c, 1);
        chr_of(c)?.borrow_mut().zombie_knock_down(v);
        Ok(0)
    });
    b.method("Wander", |c| {
        with_state_cx(c, |z, cx| z.wander(cx))?;
        Ok(0)
    });
    b.method("isTargetLocationKnown", |c| {
        let v = chr_of(c)?.borrow().is_target_location_known();
        Ok(c.push(v))
    });
    b.method("isZombieAttacking", |c| {
        let v = chr_of(c)?.borrow().is_zombie_attacking();
        Ok(c.push(v))
    });
    b.method("getTargetSeenTime", |c| {
        let v = chr_of(c)?.borrow().zombie.target_seen_time;
        Ok(c.push(Value::Num(f64::from(v))))
    });
    b.method("setTargetSeenTime", |c| {
        let v = num_arg(c, 1) as f32;
        chr_of(c)?.borrow_mut().zombie.target_seen_time = v;
        Ok(0)
    });
    b.method("getEatSpeed", |c| {
        let v = chr_of(c)?.borrow().zombie.eat_speed;
        Ok(c.push(Value::Num(f64::from(v))))
    });
    b.method("getThumpCondition", |c| {
        let v = chr_of(c)?.borrow().zombie.thump_condition;
        Ok(c.push(Value::Num(f64::from(v))))
    });
    b.method("getUnbalancedLevel", |c| {
        let v = chr_of(c)?.borrow().zombie.unbalanced_level;
        Ok(c.push(Value::Num(f64::from(v))))
    });
    b.method("setUnbalancedLevel", |c| {
        let v = num_arg(c, 1) as f32;
        chr_of(c)?.borrow_mut().zombie.unbalanced_level = v;
        Ok(0)
    });
    b.method("getPlayerAttackPosition", |c| {
        let v = chr_of(c)?.borrow().vars.player_attack_position.clone();
        Ok(c.push(v.map_or(Value::Nil, |s| Value::str(&s))))
    });
    b.method("setPlayerAttackPosition", |c| {
        let v = str_arg(c, 1).map(|s| s.to_string());
        chr_of(c)?.borrow_mut().vars.player_attack_position = v;
        Ok(0)
    });
    b.method("getOutfitName", |c| {
        let v = chr_of(c)?.borrow().zombie.outfit_name.clone();
        Ok(c.push(v.map_or(Value::Nil, |s| Value::str(&s))))
    });
    b.method("isRespondingToPlayerSound", |c| {
        let v = chr_of(c)?.borrow().is_responding_to_player_sound();
        Ok(c.push(v))
    });
    b.method("setTurnAlertedValues", |c| {
        let (x, y) = (int_arg(c, 1), int_arg(c, 2));
        chr_of(c)?.borrow_mut().set_turn_alerted_values(x, y);
        Ok(0)
    });
    b.method("getVoiceSoundName", |c| {
        let this = c.arg(0);
        let chr = chr_of(c)?;
        let v = with_ctx(c, &this, |ctx| Ok(chr.borrow_mut().voice_sound_name(ctx)))?;
        Ok(c.push(v))
    });
    b.method("getBiteSoundName", |c| {
        let this = c.arg(0);
        let chr = chr_of(c)?;
        let v = with_ctx(c, &this, |ctx| Ok(chr.borrow_mut().bite_sound_name(ctx)))?;
        Ok(c.push(v))
    });
    b.method("RespondToSound", |c| {
        with_state_cx(c, |z, cx| z.respond_to_sound(cx))?;
        Ok(0)
    });
    b.method("pathToSound", |c| {
        let (x, y, zz) = (int_arg(c, 1), int_arg(c, 2), int_arg(c, 3));
        with_state_cx(c, |z, cx| z.zombie_path_to_sound(x, y, zz, cx))?;
        Ok(0)
    });

    // WorldSoundManager.
    let m = expose_class(thread, WORLD_SOUND_MANAGER_CLASS, None);
    expose_class(thread, WORLD_SOUND_CLASS, None);
    m.method("addSound", |c| {
        let v = add_sound(c, 1, false);
        Ok(c.push(v))
    });
    m.method("addSoundRepeating", |c| {
        let v = add_sound(c, 1, true);
        Ok(c.push(v))
    });
    m.method("getStressFromSounds", |c| {
        let (x, y, z) = (int_arg(c, 1), int_arg(c, 2), int_arg(c, 3));
        let v = shared_world_sounds().borrow().get_stress_from_sounds(x, y, z);
        Ok(c.push(Value::Num(f64::from(v))))
    });
    m.method("KillCell", |c| {
        let _ = c;
        shared_world_sounds().borrow_mut().kill_cell();
        Ok(0)
    });
    m.method("getHearingMultiplier", |c| {
        let v = WorldSoundManager::hearing_multiplier(int_arg(c, 1));
        Ok(c.push(Value::Num(f64::from(v))))
    });
    global(thread, "getWorldSoundManager", |c| Ok(c.push(MANAGER.with(|m| m.clone()))));
    global(thread, "addSound", |c| {
        add_sound(c, 0, false);
        Ok(0)
    });
}

/// Registers the bindings.
pub fn register_shared(thread: &mut Thread) {
    register(thread);
}

#[cfg(test)]
mod tests {
    use super::*;
    use pz_kahlua::TableRef;

    #[test]
    fn world_sounds_from_lua() {
        let mut t = Thread::new(TableRef::new());
        pz_kahlua::stdlib::install(&mut t, None).unwrap();
        register(&mut t);
        let src = r#"
            local function assert(v) if not v then error("assertion failed!", 2) end end
            local m = getWorldSoundManager()
            assert(m == getWorldSoundManager())
            local s = m:addSound(nil, 10, 10, 0, 20, 30, true)
            assert(s ~= nil)
            assert(m:addSound(nil, 10, 10, 0, 0, 30) == nil)
            addSound(nil, 12, 10, 0, 10, 5)
            assert(m:getStressFromSounds(10, 12, 0) > 0)
            assert(m:getHearingMultiplier(1) == 3)
        "#;
        let p = pz_kahlua::compiler::compile(src, Some("t"), Some("t"), None).unwrap();
        let f = Value::Closure(pz_kahlua::vm::LuaClosure::new(Rc::new(p), t.globals().clone()));
        t.pcall(&f, &[]).map_err(|(e, tr)| format!("{}\n{tr}", pz_kahlua::exec::java_value_of(&e))).unwrap();
        assert_eq!(shared_world_sounds().borrow().sound_list.len(), 2);
    }
}
