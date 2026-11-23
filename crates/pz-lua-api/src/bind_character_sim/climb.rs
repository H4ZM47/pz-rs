//! The Lua calls that start a climb: `climbOverFence`, `climbOverWall`,
//! `climbThroughWindow`, `climbThroughWindowFrame` and `shouldBeTurning`.
//!
//! The original calls run the world's checks and set the climb state's
//! parameters at once. Here the call queues the same request that
//! `PathFindBehavior2` queues for a path-following character, and the
//! engine performs it with the states' world at the end of the character's
//! update (`GameCharacter::run_path_requests`).

use super::control::direction_arg;
use super::chr_of;
use crate::expose::ClassBuilder;
use pz_kahlua::{LuaError, LuaResult, Value};
use pz_pathfind::mover::{PathMover, PlayerMover};

/// The world object in argument `i` as the pathfinding identity of the
/// window or frame the request names.
fn object_arg(c: &pz_kahlua::NativeCall<'_>, i: usize) -> LuaResult<pz_pathfind::mover::ObjectRef> {
    match c.arg(i) {
        Value::Nil => Err(LuaError::Runtime { class: "java.lang.NullPointerException", message: String::new() }),
        v => crate::bind_world_object::object_ref(&v)?
            .ok_or_else(|| crate::bind_world_object::unsupported("climbing through a world object that is off the map")),
    }
}

fn aim_at_object(c: &pz_kahlua::NativeCall<'_>) -> LuaResult<()> {
    use pz_world::IsoDirections::{E, N, S, W};
    use crate::bind_world_object::borrowed;
    let value = c.arg(1);
    if matches!(value, Value::Nil) { return Ok(()) }
    let actor = chr_of(c)?;
    let (x, y, z, current) = {
        let actor = actor.try_borrow().map_err(|_| borrowed("facing character"))?;
        (actor.vars.x, actor.vars.y, actor.vars.z, actor.current_square_xyz())
    };
    if let Value::Object(value) = &value {
        if let Some(other) = value.as_any().downcast_ref::<super::CharacterObject>() {
            let other = other.0.try_borrow().map_err(|_| borrowed("character facing target"))?;
            let (dx, dy) = (other.vars.x - x, other.vars.y - y);
            drop(other);
            if dx != 0.0 || dy != 0.0 {
                actor.try_borrow_mut().map_err(|_| borrowed("facing character"))?.set_forward_direction(dx, dy);
            }
            return Ok(());
        }
    }
    if let Some(vehicle) = pz_vehicles::lua::vehicle_from_value(&value) {
        if vehicle.square_coords().is_some() {
            let point = vehicle.facing_position(x, y, z);
            actor.try_borrow_mut().map_err(|_| borrowed("facing character"))?
                .set_forward_direction(point.x - x, point.y - y);
        }
        return Ok(());
    }
    crate::bind_climb::with_live_object(&value, |adapter, square, object, _| {
        let mut actor = actor.try_borrow_mut().map_err(|_| borrowed("facing character"))?;
        let north = match object.kind {
            pz_world::objects::ObjectKind::WindowFrame { north } => Some(north),
            _ => pz_world::world_state::barricade_able_north(object),
        };
        if let Some(north) = north {
            let pos = (square.x, square.y, square.z);
            let opposite = (square.x - i32::from(!north), square.y - i32::from(north), square.z);
            if current == Some(pos) {
                actor.set_forward_iso_direction(if north { N } else { W });
                return Ok(());
            }
            if current == Some(opposite) {
                actor.set_forward_iso_direction(if north { S } else { E });
                return Ok(());
            }
        }
        let (fx, fy) = adapter.closest_facing_position(square, object, x, y);
        let (dx, dy) = (fx - x, fy - y);
        if dx == 0.0 && dy == 0.0 {
            PathMover::direction_from_vector(&mut *actor, dx, dy);
        } else {
            actor.set_forward_direction(dx, dy);
        }
        Ok(())
    })?;
    Ok(())
}

/// The timed action asks whether to wait in the same Lua call that changes
/// its facing. Use the last published body angle until the model next ticks.
fn face_this_object(c: &pz_kahlua::NativeCall<'_>) -> LuaResult<()> {
    let actor = chr_of(c)?;
    let before = {
        let actor = actor.try_borrow().map_err(|_| crate::bind_world_object::borrowed("facing character"))?;
        (actor.vars.forward_x, actor.vars.forward_y)
    };
    aim_at_object(c)?;
    let mut actor = actor.try_borrow_mut().map_err(|_| crate::bind_world_object::borrowed("facing character"))?;
    let v = &mut actor.vars;
    let after = (v.forward_x, v.forward_y);
    if v.has_animation_tracks && before != after {
        let target = pz_anim::AnimationPlayer::calculate_anim_player_angle(after.0, after.1);
        let (twist, excess) = pz_anim::AnimationPlayer::facing_twist(
            v.anim_angle.to_radians(), target, v.max_twist.to_radians()
        );
        v.target_twist = twist.to_degrees();
        v.excess_twist = excess.to_degrees();
    }
    Ok(())
}

pub(super) fn register(b: &ClassBuilder) {
    use pz_characters::character::states::window::WindowAction;
    b.method("faceThisObject", |c| {
        face_this_object(c)?;
        Ok(0)
    });
    for (name, action) in [("openWindow", WindowAction::Open), ("closeWindow", WindowAction::Close)] {
        b.method(name, move |c| {
            let actor = chr_of(c)?;
            let value = c.arg(1);
            if matches!(value, Value::Nil) {
                return Err(LuaError::Runtime { class: "java.lang.NullPointerException", message: String::new() });
            }
            crate::bind_climb::with_live_object(&value, |world, _, object, target| {
                if !matches!(object.kind, pz_world::objects::ObjectKind::Window(_)) {
                    return Err(crate::bind_java_util::class_cast(&value, "zombie.iso.objects.IsoWindow"));
                }
                if let Some(info) = world.window_info_of(target) {
                    actor.try_borrow_mut().map_err(|_| crate::bind_world_object::borrowed("window action character"))?
                        .request_window_action(target, info, action);
                }
                Ok(())
            })?;
            Ok(0)
        });
    }
    // `climbOverFence(dir)`.
    b.method("climbOverFence", |c| {
        let dir = direction_arg(c, 1)?;
        PathMover::climb_over_fence(&mut *chr_of(c)?.borrow_mut(), dir);
        Ok(0)
    });
    // `IsoPlayer.climbOverWall(dir)`.
    b.method("climbOverWall", |c| {
        let dir = direction_arg(c, 1)?;
        PlayerMover::climb_over_wall(&mut *chr_of(c)?.borrow_mut(), dir);
        Ok(0)
    });
    // `climbThroughWindow(IsoWindow)` / `(IsoThumpable)` and the
    // `(IsoWindow, Integer)` form.
    b.method("climbThroughWindow", |c| {
        let window = object_arg(c, 1)?;
        PathMover::climb_through_window(&mut *chr_of(c)?.borrow_mut(), window);
        Ok(0)
    });
    // `climbThroughWindowFrame(IsoWindowFrame)`.
    b.method("climbThroughWindowFrame", |c| {
        let frame = object_arg(c, 1)?;
        PathMover::climb_through_window_frame(&mut *chr_of(c)?.borrow_mut(), frame);
        Ok(0)
    });
    // `shouldBeTurning()`.
    b.method("shouldBeTurning", |c| {
        let turning = PathMover::should_be_turning(&*chr_of(c)?.borrow());
        Ok(c.push(turning))
    });
}
