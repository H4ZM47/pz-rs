//! The character's facing, distance and player-control methods that game
//! Lua calls every frame: `IsoGameCharacter`/`IsoMovingObject`/`IsoObject`
//! (`isAlive`, `faceLocation`, `setDir`, `DistTo`, ...), `IsoPlayer`'s
//! movement/context flags and `CharacterInputComponentEntity`'s input
//! component setters.

use super::{chr_of, float_arg, float_value, CharacterKind, CharacterObject, LuaGameCharacterRef, CONTEXT};
use crate::expose::{bool_arg, int_arg, ClassBuilder, JavaEnum};
use pz_kahlua::{LuaError, LuaResult, NativeCall, Thread, Value};
use pz_world::IsoDirections;

/// The `IsoMovingObject` argument at `i` when it is a character.
fn chr_arg(c: &NativeCall<'_>, i: usize) -> Option<LuaGameCharacterRef> {
    match c.arg(i) {
        Value::Object(o) => o.as_any().downcast_ref::<CharacterObject>().map(|o| o.0.clone()),
        _ => None,
    }
}

/// `other.getX()`, `other.getY()` for any `IsoObject` through its binding
/// (a character answers directly).
fn object_xy(c: &mut NativeCall<'_>, i: usize) -> LuaResult<Option<(f32, f32)>> {
    if let Some(chr) = chr_arg(c, i) {
        let chr = chr.borrow();
        return Ok(Some((chr.vars.x, chr.vars.y)));
    }
    let obj = c.arg(i);
    if obj.is_nil() {
        return Ok(None);
    }
    let mut get = |t: &mut Thread, name: &str| -> LuaResult<f32> {
        let f = t.table_get(&obj, &Value::str(name))?;
        if f.is_nil() {
            return Err(LuaError::runtime(format!(
                "{}.{name} is not bound",
                pz_kahlua::stdlib::java_class_name(&obj)
            )));
        }
        Ok(t.call1(&f, std::slice::from_ref(&obj))?.as_num().unwrap_or(0.0) as f32)
    };
    let x = get(c.thread, "getX")?;
    let y = get(c.thread, "getY")?;
    Ok(Some((x, y)))
}

fn npe() -> LuaError {
    LuaError::Runtime { class: "java.lang.NullPointerException", message: String::new() }
}

/// An `IsoDirections` constant or its ordinal (`setDir(int)`:
/// `IsoDirections.fromIndex(dir)`, `VALUES[index & 7]`).
fn direction_arg(c: &NativeCall<'_>, i: usize) -> LuaResult<IsoDirections> {
    match c.arg(i) {
        Value::Object(o) => o
            .as_any()
            .downcast_ref::<JavaEnum>()
            .filter(|e| e.class == crate::bind_iso_enums::ISO_DIRECTIONS)
            .and_then(|e| IsoDirections::from_name(e.name))
            .ok_or_else(|| LuaError::runtime("Expected an IsoDirections")),
        Value::Num(_) => Ok(IsoDirections::ALL[(int_arg(c, i) & 7) as usize]),
        Value::Nil => Err(npe()),
        _ => Err(LuaError::runtime("Expected an IsoDirections")),
    }
}

/// `IsoUtils.DistanceManhatten(x0, y0, x1, y1)`.
fn manhattan(x0: f32, y0: f32, x1: f32, y1: f32) -> f32 {
    (x1 - x0).abs() + (y1 - y0).abs()
}

/// `IsoUtils.DistanceToSquared(x0, y0, x1, y1)`.
fn dist_sq(x0: f32, y0: f32, x1: f32, y1: f32) -> f32 {
    (x1 - x0) * (x1 - x0) + (y1 - y0) * (y1 - y0)
}

/// `IsoGameCharacter.faceLocationF(x, y)`: false (and no turn) when the
/// point is the character's own position.
fn face_location_f(chr: &LuaGameCharacterRef, x: f32, y: f32) -> bool {
    let mut chr = chr.borrow_mut();
    let (dx, dy) = (x - chr.vars.x, y - chr.vars.y);
    // `PZMath.equal(a, b)`: within 1e-7.
    if dx.abs() < 1e-7 && dy.abs() < 1e-7 {
        return false;
    }
    chr.set_forward_direction(dx, dy);
    true
}

/// `Core.debug && DebugOptions.instance.<name>.getValue()`.
fn debug_cheat(name: &str) -> bool {
    crate::bind_core::shared_core().borrow().debug
        && pz_core::debug::debug_options().borrow().get_boolean(name)
}

/// The `LuaManager.GlobalObject` functions about player control.
pub(super) fn register_globals(thread: &mut Thread) {
    // `getLoosingXpTick(timer)`. TODO(port): the `DebugType.Multiplayer`
    // noise line in debug.
    crate::expose::global(thread, "getLoosingXpTick", |c| {
        let v = if debug_cheat("Cheat.Player.FastLooseXp") { 30000 } else { 10 };
        Ok(c.push(v))
    });
}

/// The methods on `IsoGameCharacter` (and so every character class).
pub(super) fn register(b: &ClassBuilder) {
    b.method("isAlive", |c| {
        let v = !chr_of(c)?.borrow().is_dead();
        Ok(c.push(v))
    });
    b.method("faceLocation", |c| {
        let (x, y) = (float_arg(c, 1), float_arg(c, 2));
        let v = face_location_f(&chr_of(c)?, x + 0.5, y + 0.5);
        Ok(c.push(v))
    });
    b.method("faceLocationF", |c| {
        let (x, y) = (float_arg(c, 1), float_arg(c, 2));
        let v = face_location_f(&chr_of(c)?, x, y);
        Ok(c.push(v))
    });
    // `faceDirection(dir)`, `setDir(dir)` / `setDir(int)`:
    // `setForwardIsoDirection`, which also turns the forward vector.
    for name in ["faceDirection", "setDir", "setForwardIsoDirection"] {
        b.method(name, |c| {
            let dir = direction_arg(c, 1)?;
            chr_of(c)?.borrow_mut().set_forward_iso_direction(dir);
            Ok(0)
        });
    }
    // `DistTo(int x, int y)` / `DistTo(IsoMovingObject)` (0 for null).
    b.method("DistTo", |c| {
        let (x, y) = { let chr = chr_of(c)?; let chr = chr.borrow(); (chr.vars.x, chr.vars.y) };
        let v = match c.arg(1) {
            Value::Num(_) => manhattan(int_arg(c, 1) as f32, int_arg(c, 2) as f32, x, y),
            _ => object_xy(c, 1)?.map_or(0.0, |(ox, oy)| manhattan(x, y, ox, oy)),
        };
        Ok(c.push(float_value(v)))
    });
    // `DistToSquared(float x, float y)` / `DistToSquared(IsoMovingObject)`.
    // TODO(port): the vehicle polygon cases (`BaseVehicle.getClosestPointOnPoly`).
    b.method("DistToSquared", |c| {
        let (x, y) = { let chr = chr_of(c)?; let chr = chr.borrow(); (chr.vars.x, chr.vars.y) };
        let v = match c.arg(1) {
            Value::Num(_) => dist_sq(float_arg(c, 1), float_arg(c, 2), x, y),
            _ => {
                let (ox, oy) = object_xy(c, 1)?.ok_or_else(npe)?;
                dist_sq(x, y, ox, oy)
            }
        };
        Ok(c.push(float_value(v)))
    });
    // `DistToProper(IsoObject)`: `sqrt(DistToSquared)` for moving objects,
    // `IsoUtils.DistanceTo` otherwise (the same distance).
    b.method("DistToProper", |c| {
        let (x, y) = { let chr = chr_of(c)?; let chr = chr.borrow(); (chr.vars.x, chr.vars.y) };
        let (ox, oy) = object_xy(c, 1)?.ok_or_else(npe)?;
        Ok(c.push(float_value(dist_sq(x, y, ox, oy).sqrt())))
    });
    b.method("isSitOnGround", |c| {
        let v = chr_of(c)?.borrow().sit_on_ground;
        Ok(c.push(v))
    });
    b.method("getIgnoreMovement", |c| {
        let v = chr_of(c)?.borrow().vars.ignore_movement;
        Ok(c.push(v))
    });
    // TODO(port): `IsoPlayer.setIgnoreMovement`'s
    // `getNetworkCharacterAI().needToUpdate()` (multiplayer).
    b.method("setIgnoreMovement", |c| {
        let v = bool_arg(c, 1);
        chr_of(c)?.borrow_mut().vars.ignore_movement = v;
        Ok(0)
    });
    b.method("setAutoWalk", |c| {
        let v = bool_arg(c, 1);
        chr_of(c)?.borrow_mut().player.auto_walk = v;
        Ok(0)
    });
    b.method("isClimbingRope", |c| {
        let v = chr_of(c)?.borrow().is_climbing_rope();
        Ok(c.push(v))
    });
    // `IsoGameCharacter.isPlayerMoving()` is false; `IsoPlayer` returns
    // its `isPlayerMoving` field.
    b.method("isPlayerMoving", |c| {
        let chr = chr_of(c)?;
        let chr = chr.borrow();
        let v = matches!(chr.kind, CharacterKind::Player | CharacterKind::Animal) && chr.player_moving;
        Ok(c.push(v))
    });
    // `IsoGameCharacter.getTorchStrength()` is 0; `IsoPlayer`'s is the
    // active light item's strength (none for animals). TODO(port): the
    // attached items (`getAttachedItems`) and remote players'
    // `mpTorchStrength`.
    b.method("getTorchStrength", |c| {
        let chr = chr_of(c)?;
        let chr = chr.borrow();
        let v = if chr.is_player() {
            [chr.items.primary_hand_item(), chr.items.secondary_hand_item()]
                .into_iter()
                .flatten()
                .find(|i| i.borrow().is_emitting_light())
                .map_or(0.0, |i| i.borrow().get_light_strength())
        } else {
            0.0
        };
        Ok(c.push(float_value(v)))
    });
    // `Core.debug && DebugOptions.cheat.timedAction.instant` or the
    // character's `TIMED_ACTION_INSTANT` cheat. TODO(port): `IsoPlayer`'s
    // multiplayer access-level check.
    b.method("isTimedActionInstant", |c| {
        let option = debug_cheat("Cheat.TimedAction.Instant");
        let ctx = CONTEXT.with(|c| c.borrow().clone());
        let v = option || chr_of(c)?.borrow().cheats.is_set(&*ctx, "TIMED_ACTION_INSTANT");
        Ok(c.push(v))
    });
    register_input_component(b);
}

/// `CharacterInputComponentEntity`: only `IsoPlayer` (and so `IsoAnimal`)
/// adds a `CharacterInputComponent`; on other characters the setters do
/// nothing and the getters answer false.
fn register_input_component(b: &ClassBuilder) {
    type Field = fn(&mut pz_characters::character::player::CharacterInput) -> &mut bool;
    let fields: [(&str, &str, Field); 5] = [
        ("setIgnoreInputsForDirection", "isIgnoreInputsForDirection", |i| &mut i.ignore_inputs_for_direction),
        ("setJoypadIgnoreAim", "isJoypadIgnoreAim", |i| &mut i.joypad_ignore_aim),
        ("setJoypadIgnoreAimUntilCentered", "isJoypadIgnoreAimUntilCentered", |i| &mut i.joypad_ignore_aim_until_centered),
        ("setAllowSprint", "isAllowSprint", |i| &mut i.allow_sprint),
        ("setAllowRun", "isAllowRun", |i| &mut i.allow_run),
    ];
    for (set, get, field) in fields {
        b.method(set, move |c| {
            let v = bool_arg(c, 1);
            let chr = chr_of(c)?;
            let mut chr = chr.borrow_mut();
            if has_input_component(chr.kind) {
                *field(&mut chr.player.input) = v;
            }
            Ok(0)
        });
        b.method(get, move |c| {
            let chr = chr_of(c)?;
            let mut chr = chr.borrow_mut();
            let v = has_input_component(chr.kind) && *field(&mut chr.player.input);
            Ok(c.push(v))
        });
    }
}

fn has_input_component(kind: CharacterKind) -> bool {
    matches!(kind, CharacterKind::Player | CharacterKind::Animal)
}

/// The methods only `IsoPlayer` (and `IsoAnimal`) declares.
pub(super) fn register_player(b: &ClassBuilder) {
    b.method("setBlockMovement", |c| {
        let v = bool_arg(c, 1);
        chr_of(c)?.borrow_mut().vars.block_movement = v;
        Ok(0)
    });
    b.method("isIgnoreContextKey", |c| {
        let v = chr_of(c)?.borrow().player.ignore_context_key;
        Ok(c.push(v))
    });
    b.method("setIgnoreContextKey", |c| {
        let v = bool_arg(c, 1);
        chr_of(c)?.borrow_mut().player.ignore_context_key = v;
        Ok(0)
    });
    b.method("setIgnoreAutoVault", |c| {
        let v = bool_arg(c, 1);
        chr_of(c)?.borrow_mut().vars.ignore_auto_vault = v;
        Ok(0)
    });
    // `setAuthorizedHandToHandAction(enabled)`.
    b.method("setAuthorizeMeleeAction", |c| {
        let v = bool_arg(c, 1);
        chr_of(c)?.borrow_mut().player.authorize_hand_to_hand_action = v;
        Ok(0)
    });
    b.method("setBannedAttacking", |c| {
        let v = bool_arg(c, 1);
        chr_of(c)?.borrow_mut().vars.banned_attacking = v;
        Ok(0)
    });
}

#[cfg(test)]
mod tests;
