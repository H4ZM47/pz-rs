//! The `IsoGameCharacter` / `IsoLivingCharacter` / `IsoPlayer` combat and
//! state-flag members Lua uses (`isAiming`, `isAttacking`, `isDoShove`,
//! `isCriticalHit`, `getHitReaction`, `setAttackedBy`, `Kill`,
//! `calculateCritChance`, `testDotSide`, ...), over
//! `pz_characters::character::combat`.
//!
//! Characters are referred to by id inside the simulation; the Lua objects
//! of attackers are remembered here when Lua passes them
//! (`setAttackedBy`, `Kill`) so `getAttackedBy` can return them.
//! `Hit(weapon, ...)` is not bound: it takes an `InventoryItem` Lua object,
//! which is not exposed yet. TODO(InventoryItem Lua objects).

use crate::bind_character_sim::{
    character_value, chr_of, CharacterObject, LuaGameCharacter, LuaGameCharacterRef,
};
use crate::bind_characters::shared_characters;
use crate::bind_registry::shared_script_manager;
use crate::bind_states::with_state_cx;
use crate::expose::{bool_arg, num_arg, str_arg, ClassBuilder};
use pz_kahlua::{NativeCall, Value};
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::{Rc, Weak};

thread_local! {
    static BY_ID: RefCell<HashMap<i32, Weak<RefCell<LuaGameCharacter>>>> = RefCell::new(HashMap::new());
}

fn remember(chr: &LuaGameCharacterRef) -> i32 {
    let id = chr.borrow().id;
    BY_ID.with(|m| m.borrow_mut().insert(id, Rc::downgrade(chr)));
    id
}

fn by_id(id: Option<i32>) -> Value {
    id.and_then(|id| BY_ID.with(|m| m.borrow().get(&id).and_then(|w| w.upgrade())))
        .map_or(Value::Nil, |c| character_value(&c))
}

/// A character argument (`nil` is `None`).
fn character_arg(c: &NativeCall<'_>, i: usize) -> Option<LuaGameCharacterRef> {
    match c.arg(i) {
        Value::Object(o) => o
            .as_any()
            .downcast_ref::<CharacterObject>()
            .map(|o| o.0.clone()),
        _ => None,
    }
}

fn float(v: f32) -> Value {
    Value::Num(f64::from(v))
}

macro_rules! flag {
    ($b:expr, $get:literal, $set:literal, $field:ident) => {
        $b.method($get, |c| {
            let v = chr_of(c)?.borrow().vars.$field;
            Ok(c.push(v))
        });
        $b.method($set, |c| {
            let v = bool_arg(c, 1);
            chr_of(c)?.borrow_mut().vars.$field = v;
            Ok(0)
        });
    };
}

macro_rules! float_field {
    ($b:expr, $get:literal, $set:literal, $field:ident) => {
        $b.method($get, |c| {
            let v = chr_of(c)?.borrow().vars.$field;
            Ok(c.push(float(v)))
        });
        $b.method($set, |c| {
            let v = num_arg(c, 1) as f32;
            chr_of(c)?.borrow_mut().vars.$field = v;
            Ok(0)
        });
    };
}

/// Adds the members to the `IsoGameCharacter` exposure.
pub fn register_character_methods(b: &ClassBuilder) {
    b.method("isAiming", |c| {
        let v = chr_of(c)?.borrow().is_aiming();
        Ok(c.push(v))
    });
    b.method("setIsAiming", |c| {
        let v = bool_arg(c, 1);
        chr_of(c)?.borrow_mut().vars.is_aiming = v;
        Ok(0)
    });
    // `IsoPlayer.isAttacking()`: an attack type other than NONE
    // (`IsoGameCharacter.isAttacking()` is false for the others).
    b.method("isAttacking", |c| {
        let chr = chr_of(c)?;
        let chr = chr.borrow();
        Ok(c.push(chr.is_player() && !chr.vars.attack_type.is_empty()))
    });
    flag!(b, "isDoShove", "setDoShove", do_shove);
    flag!(b, "isCriticalHit", "setCriticalHit", critical_hit);
    flag!(b, "isOnFloor", "setOnFloor", on_floor);
    flag!(b, "isKnockedDown", "setKnockedDown", knocked_down);
    flag!(b, "isAimAtFloor", "setAimAtFloor", aim_at_floor);
    flag!(b, "isHitFromBehind", "setHitFromBehind", hit_from_behind);
    flag!(
        b,
        "isRangedWeaponEmpty",
        "setRangedWeaponEmpty",
        ranged_weapon_empty
    );
    float_field!(b, "getHitForce", "setHitForce", hit_force);
    float_field!(
        b,
        "getStateEventDelayTimer",
        "setStateEventDelayTimer",
        state_event_delay_timer
    );
    float_field!(b, "getReanimateTimer", "setReanimateTimer", reanimate_timer);
    b.method("isFallOnFront", |c| {
        let v = chr_of(c)?.borrow().environment.fall_on_front;
        Ok(c.push(v))
    });
    b.method("setFallOnFront", |c| {
        let v = bool_arg(c, 1);
        chr_of(c)?.borrow_mut().environment.fall_on_front = v;
        Ok(0)
    });
    b.method("getHitReaction", |c| {
        let v = chr_of(c)?.borrow().vars.hit_reaction.clone();
        Ok(c.push(v))
    });
    b.method("setHitReaction", |c| {
        let v = str_arg(c, 1).map(|s| s.to_string()).unwrap_or_default();
        chr_of(c)?.borrow_mut().vars.hit_reaction = v;
        Ok(0)
    });
    b.method("getBumpType", |c| {
        let v = chr_of(c)?.borrow().vars.bump_type.clone();
        Ok(c.push(v))
    });
    b.method("setBumpType", |c| {
        let v = str_arg(c, 1).map(|s| s.to_string()).unwrap_or_default();
        chr_of(c)?.borrow_mut().set_bump_type(&v);
        Ok(0)
    });
    b.method("isBumped", |c| {
        let v = chr_of(c)?.borrow().is_bumped();
        Ok(c.push(v))
    });
    b.method("isProne", |c| {
        let v = chr_of(c)?.borrow().is_prone();
        Ok(c.push(v))
    });
    b.method("isStanding", |c| {
        let v = chr_of(c)?.borrow().is_standing();
        Ok(c.push(v))
    });
    b.method("getAttackedBy", |c| {
        let id = chr_of(c)?.borrow().vars.attacked_by;
        Ok(c.push(by_id(id)))
    });
    b.method("setAttackedBy", |c| {
        let a = character_arg(c, 1).map(|a| remember(&a));
        chr_of(c)?.borrow_mut().vars.attacked_by = a;
        Ok(0)
    });
    b.method("getLastHitCharacter", |c| {
        let id = chr_of(c)?.borrow().vars.last_hit_character;
        Ok(c.push(by_id(id)))
    });
    b.method("Kill", |c| {
        let killer = character_arg(c, 1).map(|a| remember(&a));
        with_state_cx(c, |chr, cx| chr.kill_by(killer, cx))?;
        Ok(0)
    });
    b.method("getWeaponLevel", |c| {
        let sm = shared_script_manager();
        let chars = shared_characters();
        let v = chr_of(c)?.borrow().weapon_level(&sm, &chars.borrow());
        Ok(c.push(Value::Num(f64::from(v))))
    });
    b.method("getHittingMod", |c| {
        let chars = shared_characters();
        let v = chr_of(c)?.borrow().hitting_mod(&chars.borrow());
        Ok(c.push(float(v)))
    });
    b.method("getShovingMod", |c| {
        let chars = shared_characters();
        let v = chr_of(c)?.borrow().shoving_mod(&chars.borrow());
        Ok(c.push(float(v)))
    });
    b.method("calculateCritChance", |c| {
        let target = character_arg(c, 1).ok_or_else(npe)?;
        let chr = chr_of(c)?;
        let sm = shared_script_manager();
        let chars = shared_characters();
        let ctx = crate::bind_character_sim::current_sim_context();
        let v = if Rc::ptr_eq(&chr, &target) {
            let p = chr.borrow();
            p.calculate_crit_chance(&sm, &chars.borrow(), &*ctx, &p)
        } else {
            chr.borrow()
                .calculate_crit_chance(&sm, &chars.borrow(), &*ctx, &target.borrow())
        };
        Ok(c.push(Value::Num(f64::from(v))))
    });
    b.method("testDotSide", |c| {
        let target = character_arg(c, 1).ok_or_else(npe)?;
        let (x, y) = {
            let t = target.borrow();
            (t.vars.x, t.vars.y)
        };
        let v = chr_of(c)?.borrow().test_dot_side(x, y).name();
        Ok(c.push(v))
    });
    b.method("isBehind", |c| {
        let target = character_arg(c, 1).ok_or_else(npe)?;
        let (x, y, f) = {
            let t = target.borrow();
            (t.vars.x, t.vars.y, (t.vars.forward_x, t.vars.forward_y))
        };
        let v = chr_of(c)?.borrow().is_behind(x, y, f);
        Ok(c.push(v))
    });
}

fn npe() -> pz_kahlua::LuaError {
    pz_kahlua::LuaError::Runtime {
        message: String::new(),
        class: "java.lang.NullPointerException",
    }
}
