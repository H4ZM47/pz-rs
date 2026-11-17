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

pub(super) fn register(b: &ClassBuilder) {
    // `IsoPlayer.isBlockMovement()` / `setBlockMovement(boolean)`.
    b.method("isBlockMovement", |c| {
        let blocked = chr_of(c)?.borrow().vars.block_movement;
        Ok(c.push(blocked))
    });
    b.method("setBlockMovement", |c| {
        let blocked = c.arg(1).truthy();
        chr_of(c)?.borrow_mut().vars.block_movement = blocked;
        Ok(0)
    });
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
