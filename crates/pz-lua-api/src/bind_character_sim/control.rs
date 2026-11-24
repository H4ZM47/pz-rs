//! The character's facing, distance and player-control methods that game
//! Lua calls every frame: `IsoGameCharacter`/`IsoMovingObject`/`IsoObject`
//! (`isAlive`, `faceLocation`, `setDir`, `DistTo`, ...), `IsoPlayer`'s
//! movement/context flags and `CharacterInputComponentEntity`'s input
//! component setters.

use super::{chr_of, float_arg, float_value, CharacterKind, CharacterObject, LuaGameCharacterRef, CONTEXT};
use crate::expose::{bool_arg, int_arg, num_arg, ClassBuilder, JavaEnum};
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
pub(super) fn direction_arg(c: &NativeCall<'_>, i: usize) -> LuaResult<IsoDirections> {
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

/// The `IsoObject` fields of a character: `bed` and `sitOnFurnitureObject`.
/// They live on the character, so they go with it.
#[derive(Clone, Copy)]
enum ObjectField {
    Bed,
    SitOnFurniture,
}

/// The object field `field` of `chr`; nil when none is set.
fn object_field(chr: &LuaGameCharacterRef, field: ObjectField) -> Value {
    let chr = chr.borrow();
    let slot = match field {
        ObjectField::Bed => &chr.bed,
        ObjectField::SitOnFurniture => &chr.sit_on_furniture_object,
    };
    slot.clone().unwrap_or(Value::Nil)
}

/// Set the object field `field` of `chr` (nil clears it).
fn set_object_field(chr: &LuaGameCharacterRef, field: ObjectField, object: Value) {
    let mut chr = chr.borrow_mut();
    let slot = match field {
        ObjectField::Bed => &mut chr.bed,
        ObjectField::SitOnFurniture => &mut chr.sit_on_furniture_object,
    };
    *slot = (!object.is_nil()).then_some(object);
}

/// The `LuaManager.GlobalObject` functions about player control.
pub(super) fn register_globals(thread: &mut Thread) {
    // `getLoosingXpTick(timer)`. TODO(port): the `DebugType.Multiplayer`
    // noise line in debug.
    crate::expose::global(thread, "getLoosingXpTick", |c| {
        let v = if debug_cheat("Cheat.Player.FastLooseXp") { 30000 } else { 10 };
        Ok(c.push(v))
    });
    // `IsoPlayer.allPlayersAsleep()`: at least one living local player,
    // and all of them asleep.
    let player = crate::expose::expose_class(thread, super::ISO_PLAYER_CLASS, Some(super::ISO_GAME_CHARACTER_CLASS));
    player.static_fn("allPlayersAsleep", |c| {
        let (mut living, mut asleep) = (0, 0);
        for i in 0..crate::bind_players::num_players().max(0) as usize {
            let Some(p) = crate::bind_players::player(i) else { continue };
            let p = p.try_borrow().map_err(|_| LuaError::runtime("IsoPlayer is in use"))?;
            if !p.is_dead() {
                living += 1;
                if p.is_asleep() {
                    asleep += 1;
                }
            }
        }
        Ok(c.push(living > 0 && living == asleep))
    });
    super::sleeping::register_globals(thread);
}

/// The methods on `IsoGameCharacter` (and so every character class).
pub(super) fn register(b: &ClassBuilder) {
    // `isLocal()`: the `NetworkComponent`'s `isLocal`, default true. It is
    // false only for a remote character on a client or a server.
    // TODO(port): multiplayer.
    b.method("isLocal", |c| Ok(c.push(true)));
    // `isGrappling()`: `IGrappleableWrapper` asks the wrapped
    // `BaseGrappleable` for its `isGrappling` field.
    b.method("isGrappling", |c| {
        let v = chr_of(c)?.borrow().is_grappling();
        Ok(c.push(v))
    });
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
    b.method("DistToSquared", |c| {
        let (x, y) = { let chr = chr_of(c)?; let chr = chr.borrow(); (chr.vars.x, chr.vars.y) };
        let v = match c.arg(1) {
            Value::Num(_) => dist_sq(float_arg(c, 1), float_arg(c, 2), x, y),
            other => {
                if let Some(vehicle) = pz_vehicles::lua::vehicle_from_value(&other) {
                    vehicle.closest_point_on_poly(x, y).0
                } else {
                    let (ox, oy) = object_xy(c, 1)?.ok_or_else(npe)?;
                    dist_sq(x, y, ox, oy)
                }
            }
        };
        Ok(c.push(float_value(v)))
    });
    // `DistToProper(IsoObject)`: `sqrt(DistToSquared)` for moving objects,
    // `IsoUtils.DistanceTo` otherwise (the same distance).
    b.method("DistToProper", |c| {
        let (x, y) = { let chr = chr_of(c)?; let chr = chr.borrow(); (chr.vars.x, chr.vars.y) };
        let other = c.arg(1);
        let d_sq = if let Some(vehicle) = pz_vehicles::lua::vehicle_from_value(&other) {
            vehicle.closest_point_on_poly(x, y).0
        } else {
            let (ox, oy) = object_xy(c, 1)?.ok_or_else(npe)?;
            dist_sq(x, y, ox, oy)
        };
        Ok(c.push(float_value(d_sq.sqrt())))
    });
    b.method("isSitOnGround", |c| {
        let v = chr_of(c)?.borrow().sit_on_ground;
        Ok(c.push(v))
    });
    // `getSitOnFurnitureObject()`: `sitOnFurnitureObject`.
    b.method("getSitOnFurnitureObject", |c| {
        let v = object_field(&chr_of(c)?, ObjectField::SitOnFurniture);
        Ok(c.push(v))
    });
    // `setSitOnFurnitureObject(object)`.
    b.method("setSitOnFurnitureObject", |c| {
        set_object_field(&chr_of(c)?, ObjectField::SitOnFurniture, c.arg(1));
        Ok(0)
    });
    // `getBed()` / `setBed(object)`: `bed`.
    b.method("getBed", |c| {
        let v = object_field(&chr_of(c)?, ObjectField::Bed);
        Ok(c.push(v))
    });
    b.method("setBed", |c| {
        set_object_field(&chr_of(c)?, ObjectField::Bed, c.arg(1));
        Ok(0)
    });
    // `getForceWakeUpTime()` / `setForceWakeUpTime(hour)`: `forceWakeUpTime`.
    b.method("getForceWakeUpTime", |c| {
        let v = float_value(chr_of(c)?.borrow().force_wake_up_time);
        Ok(c.push(v))
    });
    b.method("setForceWakeUpTime", |c| {
        let v = float_arg(c, 1);
        chr_of(c)?.borrow_mut().force_wake_up_time = v;
        Ok(0)
    });
    // `IsoPlayer.getAsleepTime()` / `setAsleepTime(hours)`: `asleepTime`.
    b.method("getAsleepTime", |c| {
        let v = float_value(chr_of(c)?.borrow().asleep_time);
        Ok(c.push(v))
    });
    b.method("setAsleepTime", |c| {
        let v = float_arg(c, 1);
        chr_of(c)?.borrow_mut().asleep_time = v;
        Ok(0)
    });
    // `isSitOnFurnitureObject(object)`: false without a furniture object,
    // else the same object or `furniture.isConnectedSpriteGridObject(object)`.
    b.method("isSitOnFurnitureObject", |c| {
        let furniture = object_field(&chr_of(c)?, ObjectField::SitOnFurniture);
        let object = c.arg(1);
        let r = if furniture.is_nil() {
            false
        } else if object.raw_equals(&furniture) {
            true
        } else {
            let f = c.thread.table_get(&furniture, &Value::str("isConnectedSpriteGridObject"))?;
            c.thread.call1(&f, &[furniture, object])?.truthy()
        };
        Ok(c.push(r))
    });
    // `getLastHourSleeped()`: `lastHourSleeped`.
    b.method("getLastHourSleeped", |c| {
        let v = chr_of(c)?.borrow().last_hour_sleeped;
        Ok(c.push(v))
    });
    // `isOnFire()`: `onFire`.
    b.method("isOnFire", |c| {
        let v = chr_of(c)?.borrow().vars.on_fire;
        Ok(c.push(v))
    });
    // `isSittingOnFurniture()`: `isSitOnFurniture`.
    b.method("isSittingOnFurniture", |c| {
        let v = chr_of(c)?.borrow().sitting_on_furniture;
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
    b.method("getMusicThreatStatuses", |c| {
        let chr = chr_of(c)?;
        let player = chr.try_borrow().map_err(|_| crate::bind_world_object::borrowed("player music threat"))?;
        let levels = player.player.music_threat_levels.clone();
        drop(player);
        let key = (super::Handle::MusicThreatStatuses, std::rc::Rc::as_ptr(&chr) as usize, 0);
        let value = super::canonical(key, || Value::Object(std::rc::Rc::new(
            crate::bind_audio::MusicThreatStatuses::for_player(chr, levels)
        )));
        Ok(c.push(value))
    });
    b.method("getMusicIntensityEvents", |c| {
        let chr = chr_of(c)?;
        let player = chr.try_borrow().map_err(|_| crate::bind_world_object::borrowed("player music events"))?;
        Ok(c.push(Value::Object(player.player.music_intensity_events.clone())))
    });
    b.method("triggerMusicIntensityEvent", |c| {
        let id = crate::expose::str_arg(c, 1);
        let chr = chr_of(c)?;
        let player = chr.try_borrow().map_err(|_| crate::bind_world_object::borrowed("player music events"))?;
        crate::bind_audio::intensity_config().borrow().trigger(id.as_deref(), &player.player.music_intensity_events);
        Ok(0)
    });
    b.method("setBlockMovement", |c| {
        let v = bool_arg(c, 1);
        chr_of(c)?.borrow_mut().vars.block_movement = v;
        Ok(0)
    });
    b.method("isBlockMovement", |c| {
        let v = chr_of(c)?.borrow().vars.block_movement;
        Ok(c.push(v))
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
    // `isRandXp(interval, chance, perk)`: false until the perk's next check
    // time has passed. Then the next check moves `interval` ms ahead, and
    // the result is true when `ZombRand` over `1 / chance` values gives 0.
    b.method("isRandXp", |c| {
        let interval = int_arg(c, 1) as i64;
        let chance = float_arg(c, 2);
        let perk = crate::bind_skills::perk_arg(c, 3)?
            .ok_or_else(|| LuaError::runtime("isRandXp: perk is null"))?;
        let id = perk.borrow().id().to_string();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_millis() as i64);
        let chr = chr_of(c)?;
        let times = &mut chr.borrow_mut().player.next_rand_xp_check_times;
        if now <= times.get(&id).copied().unwrap_or(0) {
            return Ok(c.push(false));
        }
        times.insert(id, now + interval);
        let v = crate::bind_misc::zomb_rand((1.0 / chance) as f64) == 0.0;
        Ok(c.push(v))
    });
    // `isGhostMode()` / `setGhostMode(bool)`: `INVISIBLE` cheat flag.
    b.method("isGhostMode", |c| {
        let ctx = CONTEXT.with(|cx| cx.borrow().clone());
        let v = chr_of(c)?.borrow().is_ghost_mode(&*ctx);
        Ok(c.push(v))
    });
    b.method("setGhostMode", |c| {
        let on = bool_arg(c, 1);
        let ctx = CONTEXT.with(|cx| cx.borrow().clone());
        chr_of(c)?.borrow_mut().cheats.set(&*ctx, "INVISIBLE", on);
        Ok(0)
    });
    // `addMechanicsItem(itemID, vehicle, time)` / `getMechanicsItem(itemID)`:
    // per-player cooldown map persisted in `player_fields.mechanics_item`.
    // `addMechanicsItem(itemID, part, time)`: the first time an item id is
    // recorded (single player is never a client), the player earns Mechanics
    // XP sized by the part's uninstall Mechanics requirement; every call then
    // records the time.
    b.method("addMechanicsItem", |c| {
        let id = match c.arg(1) {
            Value::Str(s) => s.parse::<i64>().map_err(|_| {
                LuaError::Runtime { message: format!("For input string: \"{s}\""), class: "java.lang.NumberFormatException" }
            })?,
            Value::Num(n) => n as i64,
            _ => return Err(crate::bind_scripts::npe()),
        };
        let time = num_arg(c, 3) as i64;
        let chr = chr_of(c)?;
        let first = !chr.borrow().player_fields.mechanics_item.iter().any(|(k, _)| *k == id);
        if first {
            let part = c.arg(2);
            let skills = mechanics_uninstall_skills(c.thread, &part)?;
            let (lo, hi) = mechanics_xp_range(skills.as_deref());
            let amount = pz_core::random::next_range(lo, hi) as f64;
            let player = super::character_value(&chr);
            let xp = crate::item_tooltip::chr::call_or(c.thread, &player, "getXp", &[], Value::Nil)?;
            let perks = c.thread.globals().get_str("Perks");
            let mechanics = c.thread.table_get(&perks, &Value::str("Mechanics"))?;
            crate::item_tooltip::chr::call_or(c.thread, &xp, "AddXP", &[mechanics, Value::Num(amount)], Value::Nil)?;
        }
        let mut chr = chr.borrow_mut();
        let list = &mut chr.player_fields.mechanics_item;
        if let Some(entry) = list.iter_mut().find(|(k, _)| *k == id) {
            entry.1 = time;
        } else {
            list.push((id, time));
        }
        Ok(0)
    });
    b.method("getMechanicsItem", |c| {
        let id = match c.arg(1) {
            Value::Str(s) => s.parse::<i64>().unwrap_or(0),
            Value::Num(n) => n as i64,
            _ => 0,
        };
        let chr = chr_of(c)?;
        let chr = chr.borrow();
        let found = chr
            .player_fields
            .mechanics_item
            .iter()
            .find(|(k, _)| *k == id)
            .map(|(_, t)| Value::Num(*t as f64))
            .unwrap_or(Value::Nil);
        Ok(c.push(found))
    });
    b.method("getTagColor", |c| {
        let [r, g, b_col] = chr_of(c)?.borrow().player_fields.tag_color;
        Ok(c.push(Value::Object(std::rc::Rc::new(
            pz_ui::ColorObject::new(r, g, b_col, 1.0),
        ))))
    });
    // A long in the original; no Steam account in this port.
    b.method("getSteamID", |c| Ok(c.push(Value::Num(0.0))));
    b.method("getDisguisedDisplayName", |c| {
        let chr = chr_of(c)?;
        let chr = chr.borrow();
        let name = chr
            .chat
            .username
            .clone()
            .or_else(|| {
                chr.descriptor.as_ref().map(|d| {
                    let d = d.borrow();
                    format!(
                        "{} {}",
                        d.forename.as_deref().unwrap_or(""),
                        d.surname.as_deref().unwrap_or("")
                    )
                    .trim()
                    .to_string()
                })
            })
            .unwrap_or_default();
        Ok(c.push(Value::str(&name)))
    });
    b.method("getAttachedAnimals", |c| {
        Ok(c.push(Value::Object(std::rc::Rc::new(
            crate::bind_java_util::JavaList::new(Vec::new()),
        ))))
    });
    b.method("playerVoiceSound", |_c| Ok(0));
    // `setFishingStage(stage)`: the `FishingStage` animation variable, set
    // only when it changes (the client-only fishing state is not entered).
    b.method("setFishingStage", |c| {
        let stage = c.arg(1);
        let player = super::character_value(&chr_of(c)?);
        let name = Value::str("FishingStage");
        let same = crate::item_tooltip::chr::call_or(c.thread, &player, "isVariable", &[name.clone(), stage.clone()], Value::Bool(false))?;
        if !same.truthy() {
            crate::item_tooltip::chr::call_or(c.thread, &player, "setVariable", &[name, stage], Value::Nil)?;
        }
        Ok(0)
    });
}

/// The `skills` string of `part:getTable("uninstall")`, if any.
fn mechanics_uninstall_skills(t: &mut Thread, part: &Value) -> LuaResult<Option<String>> {
    if part.is_nil() {
        return Err(crate::bind_scripts::npe());
    }
    let table = crate::item_tooltip::chr::call_or(t, part, "getTable", &[Value::str("uninstall")], Value::Nil)?;
    let Value::Table(table) = table else { return Ok(None) };
    Ok(match table.get_str("skills") {
        Value::Nil => None,
        Value::Str(s) => Some(s.to_string()),
        v => return Err(crate::bind_java_util::class_cast(&v, "java.lang.String")),
    })
}

/// The Mechanics XP range for a part's uninstall skills (`Perk:level;...`):
/// higher Mechanics requirements earn more, a part needing little or none
/// earns 2, with a one-in-three chance of 4 for each low Mechanics entry.
fn mechanics_xp_range(skills: Option<&str>) -> (i32, i32) {
    let (mut lo, mut hi) = (1, 1);
    for entry in skills.into_iter().flat_map(|s| s.split(';')) {
        if !entry.contains("Mechanics") {
            continue;
        }
        let level: i32 = entry.split(':').nth(1).and_then(|n| n.trim().parse().ok()).unwrap_or(0);
        (lo, hi) = match level {
            6.. => (3, 7),
            4..=5 => (3, 5),
            2..=3 => (2, 4),
            _ if pz_core::random::next(3) == 0 => (2, 2),
            _ => (lo, hi),
        };
        lo *= 2;
        hi *= 2;
    }
    (lo, hi)
}

#[cfg(test)]
mod tests;
