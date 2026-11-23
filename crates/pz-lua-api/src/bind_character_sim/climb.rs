//! The Lua calls for climbing and landings: `climbOverFence`, `climbOverWall`,
//! `canClimbOverWall`, `climbThroughWindow`, `climbThroughWindowFrame`,
//! `climb`, `climbDown`, `canClimbSheetRope`, `canClimbDownSheetRope`,
//! `canClimbDownSheetRopeInCurrentSquare`, `fallFromRope`, `getClimbRopeSpeed`,
//! `getClimbingFailChanceInt`, `getClimbingFailChanceFloat`,
//! `nearbyZombieClimbPenalty`, `fallenOnKnees`, `DoLand`, and
//! `shouldBeTurning`.
//!
//! Climb starts queue the same request that `PathFindBehavior2` queues for a
//! path-following character, and the engine performs it with the states' world
//! at the end of the character's update (`GameCharacter::run_path_requests`).

use super::control::direction_arg;
use super::{chr_of, float_value, with_ctx_state, FloorAccess, LuaGameCharacter, CONTEXT};
use crate::bind_characters::{shared_characters, LuaItem};
use crate::bind_registry::shared_script_manager;
use crate::expose::{bool_arg, num_arg, ClassBuilder};
use pz_characters::character::path_mover::PathMoverRequest;
use pz_characters::character::state::{StateCx, StateWorld};
use pz_kahlua::{LuaError, LuaResult, NativeCall, Value};
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
        if vehicle.st().added_to_world && vehicle.st().current.is_some() {
            let point = vehicle.facing_position(x, y, z);
            actor.try_borrow_mut().map_err(|_| borrowed("facing character"))?
                .set_forward_direction(point.x - x, point.y - y);
        }
        return Ok(());
    }
    crate::bind_climb::with_live_object(&value, |adapter, square, object, _| {
        let mut actor = actor.try_borrow_mut().map_err(|_| borrowed("facing character"))?;
        if let Some(north) = pz_world::world_state::barricade_able_north(object) {
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
        let (fx, fy) = adapter.try_closest_facing_position(square, object, x, y)
            .map_err(crate::bind_world_object::host_error)?
            .ok_or_else(|| LuaError::Runtime { class: "java.lang.NullPointerException", message: String::new() })?;
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

/// The original state keeps the part and its vehicle alive until the next
/// request replaces its parameters. A recycled VehicleManager ID may name
/// a different vehicle by the time the animation delivers its hit.
struct SmashVehiclePart {
    vehicle: pz_vehicles::vehicle::VehicleRef,
    part: pz_vehicles::part::PartRef,
}

impl pz_characters::character::states::window::VehicleWindowTarget for SmashVehiclePart {
    fn facing_position(&self, x: f32, y: f32, z: f32) -> Option<(f32, f32)> {
        let v = self.vehicle.st();
        if !v.added_to_world || v.current.is_none() { return None }
        drop(v);
        let point = self.vehicle.facing_position(x, y, z);
        Some((point.x, point.y))
    }
    fn destroyed(&self) -> Option<bool> {
        let part = self.part.st();
        part.window.map(|_| part.condition == 0)
    }
    fn hit(&self) { self.part.hit_window(); }
}

/// Optional `IsoGridSquare` argument (`nil` -> `None`, wrong object -> `ClassCastException`).
fn square_arg(c: &NativeCall<'_>, i: usize) -> LuaResult<Option<(i32, i32, i32)>> {
    let v = c.arg(i);
    if v.is_nil() {
        return Ok(None);
    }
    crate::bind_world::square_coords(&v)
        .map(Some)
        .ok_or_else(|| crate::bind_java_util::class_cast(&v, "zombie.iso.IsoGridSquare"))
}

/// Runs a read-only climb query against the live `bind_world` cell when one is
/// loaded, falling back to `bind_states::with_state_cx` when no live world is
/// active (e.g. tests with an installed mock `StateWorld`).
fn with_climb_query_cx<R>(
    c: &mut NativeCall<'_>,
    f: impl FnOnce(&LuaGameCharacter, &StateCx<'_, LuaItem>) -> R,
) -> LuaResult<R> {
    let chr = chr_of(c)?;
    let sm = shared_script_manager();
    let chars = shared_characters();
    let ctx = CONTEXT.with(|ctx| ctx.borrow().clone());
    let mut f = Some(f);
    let live = crate::bind_climb::with_live_state_world(|world| {
        let chars = chars.try_borrow().map_err(|_| crate::bind_world_object::borrowed("climb characters"))?;
        let chr = chr.try_borrow().map_err(|_| crate::bind_world_object::borrowed("climbing character"))?;
        let cx = StateCx::new(&sm, &chars, &*ctx, world);
        Ok((f.take().unwrap())(&chr, &cx))
    })?;
    if let Some(result) = live {
        return Ok(result);
    }
    crate::bind_states::with_state_cx(c, |chr, cx| (f.take().unwrap())(chr, cx))
}

/// `StateWorld` wrapper for Lua landing/fall calls (`fallenOnKnees`, `DoLand`):
/// prefers the installed Lua `FloorAccess` service when present (so dropped
/// hand items land on the real floor just like `dropHandItems`), falls back to
/// the installed `StateWorld`'s floor, and reads `floor_sprite_name` from the
/// live `bind_world` cell when the inner world has none.
struct LuaFallWorld<'a> {
    inner: &'a mut dyn StateWorld<LuaItem>,
    floor: &'a mut FloorAccess,
}

impl StateWorld<LuaItem> for LuaFallWorld<'_> {
    fn floor(&mut self) -> Option<&mut dyn pz_inventory::floor_drop::FloorWorld> {
        if self.floor.has_service() {
            Some(self.floor)
        } else {
            self.inner.floor()
        }
    }

    fn floor_sprite_name(&self, sq: (i32, i32, i32)) -> Option<String> {
        self.inner
            .floor_sprite_name(sq)
            .or_else(|| crate::bind_climb::with_live_state_world(|w| Ok(w.floor_sprite_name(sq))).ok().flatten().flatten())
    }
}

fn with_fall_cx<R>(
    c: &mut NativeCall<'_>,
    f: impl FnOnce(&mut LuaGameCharacter, &mut StateCx<'_, LuaItem>) -> R,
) -> LuaResult<R> {
    let chr = chr_of(c)?;
    let this = c.arg(0);
    let sm = shared_script_manager();
    let chars = shared_characters();
    let world = crate::bind_states::current_state_world();
    let mut floor = FloorAccess::capture().map_err(crate::bind_world_object::host_error)?;
    with_ctx_state(c, &this, |ctx| {
        use pz_entity::runtime_load::RuntimeHostError;
        let chars = chars.try_borrow().map_err(|_| RuntimeHostError::Borrowed("fall characters"))?;
        let mut world = world.try_borrow_mut().map_err(|_| RuntimeHostError::Borrowed("fall state world"))?;
        let mut fall_world = LuaFallWorld { inner: &mut *world, floor: &mut floor };
        let mut cx = StateCx::new(&sm, &chars, ctx, &mut fall_world);
        let mut chr = chr.try_borrow_mut().map_err(|_| RuntimeHostError::Borrowed("falling character"))?;
        let result = f(&mut chr, &mut cx);
        cx.state_result()?;
        Ok(result)
    })
}

pub(super) fn register_player(b: &ClassBuilder) {
    // `IsoPlayer.canClimbOverWall(dir)`.
    b.method("canClimbOverWall", |c| {
        let dir = direction_arg(c, 1)?;
        let can = with_climb_query_cx(c, |chr, cx| chr.can_climb_over_wall(dir, cx))?;
        Ok(c.push(can))
    });
    // `IsoPlayer.climbOverWall(dir) -> bool`.
    b.method("climbOverWall", |c| {
        let dir = direction_arg(c, 1)?;
        let can = with_climb_query_cx(c, |chr, cx| chr.can_climb_over_wall(dir, cx))?;
        if can {
            let chr = chr_of(c)?;
            let mut chr = chr.try_borrow_mut().map_err(|_| crate::bind_world_object::borrowed("climbing character"))?;
            PlayerMover::climb_over_wall(&mut *chr, dir);
        }
        Ok(c.push(can))
    });
    b.method("isClimbOverWallSuccess", |c| {
        let v = chr_of(c)?.try_borrow().map_err(|_| crate::bind_world_object::borrowed("climbing character"))?.player.climb_over_wall_success;
        Ok(c.push(v))
    });
    b.method("setClimbOverWallSuccess", |c| {
        let v = bool_arg(c, 1);
        chr_of(c)?.try_borrow_mut().map_err(|_| crate::bind_world_object::borrowed("climbing character"))?.player.climb_over_wall_success = v;
        Ok(0)
    });
    b.method("isClimbOverWallStruggle", |c| {
        let v = chr_of(c)?.try_borrow().map_err(|_| crate::bind_world_object::borrowed("climbing character"))?.player.climb_over_wall_struggle;
        Ok(c.push(v))
    });
    b.method("setClimbOverWallStruggle", |c| {
        let v = bool_arg(c, 1);
        chr_of(c)?.try_borrow_mut().map_err(|_| crate::bind_world_object::borrowed("climbing character"))?.player.climb_over_wall_struggle = v;
        Ok(0)
    });
}

pub(super) fn register(b: &ClassBuilder) {
    use pz_characters::character::states::window::WindowAction;
    b.method("faceThisObject", |c| {
        face_this_object(c)?;
        Ok(0)
    });
    for (name, action) in [("openWindow", WindowAction::Open), ("closeWindow", WindowAction::Close), ("smashWindow", WindowAction::Smash)] {
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
    b.method("smashCarWindow", |c| {
        let value = c.arg(1);
        if value.is_nil() {
            return Err(LuaError::Runtime { class: "java.lang.NullPointerException", message: String::new() });
        }
        let part = pz_vehicles::lua::part_from_value(&value)
            .ok_or_else(|| crate::bind_java_util::class_cast(&value, pz_vehicles::lua::VEHICLE_PART_CLASS))?;
        let vehicle = part.vehicle_weak().upgrade()
            .ok_or_else(|| crate::bind_world_object::unsupported("vehicle part has no owner"))?;
        let target = pz_characters::character::states::window::VehicleWindowRef::Retained(
            std::rc::Rc::new(SmashVehiclePart { vehicle, part })
        );
        chr_of(c)?.try_borrow_mut().map_err(|_| crate::bind_world_object::borrowed("smashing character"))?
            .request_vehicle_window_smash(target);
        Ok(0)
    });
    // `climbOverFence(dir)`.
    b.method("climbOverFence", |c| {
        let dir = direction_arg(c, 1)?;
        PathMover::climb_over_fence(&mut *chr_of(c)?.borrow_mut(), dir);
        Ok(0)
    });
    // `climb()` / `climbDown()`: Build 42.22 sheet-rope climb triggers called
    // by `ISClimbSheetRopeAction:perform()` (TODO(port): ladder branch).
    b.method("climb", |c| {
        let chr = chr_of(c)?;
        let mut chr = chr.try_borrow_mut().map_err(|_| crate::bind_world_object::borrowed("climbing character"))?;
        chr.path.requests.push(PathMoverRequest::ClimbSheetRope);
        Ok(0)
    });
    b.method("climbDown", |c| {
        let chr = chr_of(c)?;
        let mut chr = chr.try_borrow_mut().map_err(|_| crate::bind_world_object::borrowed("climbing character"))?;
        chr.path.requests.push(PathMoverRequest::ClimbDownSheetRope);
        Ok(0)
    });
    // `canClimbSheetRope(sq)` / `canClimbDownSheetRope(sq)` /
    // `canClimbDownSheetRopeInCurrentSquare()`.
    b.method("canClimbSheetRope", |c| {
        let sq = square_arg(c, 1)?;
        let can = with_climb_query_cx(c, |chr, cx| chr.can_climb_sheet_rope(sq, cx))?;
        Ok(c.push(can))
    });
    b.method("canClimbDownSheetRope", |c| {
        let sq = square_arg(c, 1)?;
        let can = with_climb_query_cx(c, |chr, cx| chr.can_climb_down_sheet_rope(sq, cx))?;
        Ok(c.push(can))
    });
    b.method("canClimbDownSheetRopeInCurrentSquare", |c| {
        let can = with_climb_query_cx(c, |chr, cx| chr.can_climb_down_sheet_rope_in_current_square(cx))?;
        Ok(c.push(can))
    });
    // `fallFromRope()`.
    b.method("fallFromRope", |c| {
        chr_of(c)?.try_borrow_mut().map_err(|_| crate::bind_world_object::borrowed("climbing character"))?.fall_from_rope();
        Ok(0)
    });
    // `getClimbRopeSpeed(down)`.
    b.method("getClimbRopeSpeed", |c| {
        let down = bool_arg(c, 1);
        let sm = shared_script_manager();
        let chars = shared_characters();
        let chars = chars.try_borrow().map_err(|_| crate::bind_world_object::borrowed("climb characters"))?;
        let chr = chr_of(c)?;
        let chr = chr.try_borrow().map_err(|_| crate::bind_world_object::borrowed("climbing character"))?;
        Ok(c.push(float_value(chr.climb_rope_speed(&sm, &chars, down))))
    });
    // `getClimbingFailChanceInt()` / `getClimbingFailChanceFloat()` /
    // `nearbyZombieClimbPenalty()`.
    b.method("getClimbingFailChanceInt", |c| {
        let v = crate::bind_states::with_state_cx(c, |chr, cx| chr.climbing_fail_chance_int(cx))?;
        Ok(c.push(v))
    });
    b.method("getClimbingFailChanceFloat", |c| {
        let v = crate::bind_states::with_state_cx(c, |chr, cx| chr.climbing_fail_chance_float(cx))?;
        Ok(c.push(float_value(v)))
    });
    b.method("nearbyZombieClimbPenalty", |c| {
        let v = crate::bind_states::with_state_cx(c, |chr, cx| chr.nearby_zombie_climb_penalty(cx))?;
        Ok(c.push(float_value(v)))
    });
    // `isClimbing()` / `setbClimbing(bool)` / `getClimbRopeTime()` / `setClimbRopeTime(float)`.
    b.method("isClimbing", |c| {
        let v = chr_of(c)?.try_borrow().map_err(|_| crate::bind_world_object::borrowed("climbing character"))?.vars.climbing;
        Ok(c.push(v))
    });
    b.method("setbClimbing", |c| {
        let v = bool_arg(c, 1);
        chr_of(c)?.try_borrow_mut().map_err(|_| crate::bind_world_object::borrowed("climbing character"))?.vars.climbing = v;
        Ok(0)
    });
    b.method("getClimbRopeTime", |c| {
        let v = chr_of(c)?.try_borrow().map_err(|_| crate::bind_world_object::borrowed("climbing character"))?.vars.climb_rope_time;
        Ok(c.push(float_value(v)))
    });
    b.method("setClimbRopeTime", |c| {
        let v = num_arg(c, 1) as f32;
        chr_of(c)?.try_borrow_mut().map_err(|_| crate::bind_world_object::borrowed("climbing character"))?.vars.climb_rope_time = v;
        Ok(0)
    });
    // `fallenOnKnees()` / `fallenOnKnees(hardFall)`.
    b.method("fallenOnKnees", |c| {
        let hard = if c.nargs() >= 2 { bool_arg(c, 1) } else { false };
        with_fall_cx(c, |chr, cx| chr.fallen_on_knees(hard, cx))?;
        Ok(0)
    });
    // `DoLand(impactIsoSpeed)`.
    b.method("DoLand", |c| {
        let speed = num_arg(c, 1) as f32;
        with_fall_cx(c, |chr, cx| chr.do_land(speed, cx))?;
        Ok(0)
    });
    // `climbThroughWindow(IsoWindow)` / `(IsoThumpable)` and the
    // `(IsoWindow, Integer)` form.
    b.method("climbThroughWindow", |c| {
        let window = object_arg(c, 1)?;
        let actor = chr_of(c)?;
        let mut actor = actor.try_borrow_mut().map_err(|_| crate::bind_world_object::borrowed("climbing character"))?;
        if c.nargs() > 2 {
            let frame = c.arg(2);
            if !matches!(frame, Value::Nil | Value::Num(_)) {
                return Err(crate::bind_java_util::class_cast(&frame, "java.lang.Integer"));
            }
            actor.path.requests.push(pz_characters::character::path_mover::PathMoverRequest::ClimbThroughWindowAtFrame(window));
        } else {
            PathMover::climb_through_window(&mut *actor, window);
        }
        Ok(0)
    });
    // `climbThroughWindowFrame(IsoWindowFrame)`.
    b.method("climbThroughWindowFrame", |c| {
        let frame = object_arg(c, 1)?;
        PathMover::climb_through_window_frame(&mut *chr_of(c)?.borrow_mut(), frame);
        Ok(0)
    });
    b.method("isTurning", |c| {
        let actor = chr_of(c)?;
        let actor = actor.try_borrow().map_err(|_| crate::bind_world_object::borrowed("turning character"))?;
        Ok(c.push(actor.vars.turning))
    });
    // `shouldBeTurning()`.
    b.method("shouldBeTurning", |c| {
        let turning = PathMover::should_be_turning(&*chr_of(c)?.borrow());
        Ok(c.push(turning))
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bind_character_sim::tests::{run, setup_with};
    use pz_characters::character::state::{NullWorld, SheetRopeSquare};
    use pz_inventory::floor_drop::{FloorWorld, HostResult};
    use pz_inventory::{InventoryItem, ItemClass, ItemTemplate};
    use pz_inventory::shared;
    use pz_scripts::ScriptManager;
    use pz_world::square::IsoDirections;
    use std::cell::RefCell;
    use std::collections::HashMap;
    use std::rc::Rc;

    #[derive(Default)]
    struct MockClimbWorld {
        rope: HashMap<(i32, i32, i32), SheetRopeSquare>,
        wall_dir: Option<IsoDirections>,
        zombies: Vec<LuaGameCharacter>,
    }

    impl StateWorld<LuaItem> for MockClimbWorld {
        fn sheet_rope_square(&self, sq: (i32, i32, i32)) -> Option<SheetRopeSquare> {
            self.rope.get(&sq).copied()
        }
        fn can_climb_over_wall(&self, _chr: &LuaGameCharacter, dir: IsoDirections) -> bool {
            self.wall_dir == Some(dir)
        }
        fn zombies_on_square(&self, sq: (i32, i32, i32)) -> Vec<i32> {
            if sq == (4, 5, 0) {
                self.zombies.iter().map(|z| z.id).collect()
            } else {
                Vec::new()
            }
        }
        fn character_mut(&mut self, id: i32) -> Option<&mut LuaGameCharacter> {
            self.zombies.iter_mut().find(|z| z.id == id)
        }
    }

    #[derive(Default)]
    struct MockFloor(Vec<((i32, i32, i32), i32)>);

    impl FloorWorld for MockFloor {
        fn solid_floor_ref_at(
            &self,
            x: i32,
            y: i32,
            z: i32,
        ) -> HostResult<Option<pz_inventory::floor_drop::FloorSquare>> {
            Ok(Some(pz_inventory::floor_drop::FloorSquare::data_projection((x, y, z))))
        }
        fn apparent_z_on(
            &self,
            sq: &pz_inventory::floor_drop::FloorSquare,
            x: f32,
            y: f32,
        ) -> HostResult<f32> {
            self.apparent_z(sq.position(), x, y)
        }
        fn add_world_inventory_item_on(
            &mut self,
            sq: &pz_inventory::floor_drop::FloorSquare,
            item: &pz_inventory::shared::ItemRef,
            x: f32,
            y: f32,
            h: f32,
        ) -> HostResult<()> {
            self.add_world_inventory_item(sq.position(), item, x, y, h)
        }
        fn solid_floor_at(&self, x: i32, y: i32, z: i32) -> HostResult<Option<(i32, i32, i32)>> {
            Ok(Some((x, y, z)))
        }
        fn apparent_z(&self, _sq: (i32, i32, i32), _x: f32, _y: f32) -> HostResult<f32> {
            Ok(0.0)
        }
        fn rand_float(&mut self, min: f32, _max: f32) -> HostResult<f32> {
            Ok(min)
        }
        fn add_world_inventory_item(
            &mut self,
            sq: (i32, i32, i32),
            item: &pz_inventory::shared::ItemRef,
            _x: f32,
            _y: f32,
            _h: f32,
        ) -> HostResult<()> {
            self.0.push((sq, item.borrow().id));
            Ok(())
        }
    }

    struct RestoreState {
        prev_world: Rc<RefCell<dyn StateWorld<LuaItem>>>,
        prev_floor: Option<crate::bind_character_sim::FloorWorldRef>,
        prev_cell: Option<pz_world::IsoCell>,
        prev_meta: Option<pz_world::IsoMetaGrid>,
    }

    impl Drop for RestoreState {
        fn drop(&mut self) {
            crate::bind_states::install_state_world(self.prev_world.clone());
            drop(crate::bind_character_sim::install_floor_world(self.prev_floor.take()));
            crate::bind_world::set_cell(self.prev_cell.take());
            crate::bind_world::set_meta_grid(self.prev_meta.take());
        }
    }

    #[test]
    fn climb_and_fall_lua_bindings_wire_sheet_rope_wall_and_landing() {
        let prev_world = crate::bind_states::current_state_world();
        let prev_floor = crate::bind_character_sim::reset_floor_world();
        let prev_cell = crate::bind_world::with_world(|w| w.cell.take());
        let prev_meta = crate::bind_world::with_world(|w| w.meta_grid.take());
        let _restore = RestoreState { prev_world, prev_floor, prev_cell, prev_meta };

        // Populate a cell with squares (4, 5, 0) and (4, 5, 2) so `bind_world::square_value`
        // produces real `IsoGridSquare` objects; `meta_grid` stays None so `with_climb_query_cx`
        // exercises `MockClimbWorld` via `bind_states`.
        let mut chunk = pz_world::IsoChunk::new(0, 0);
        chunk.set_square(4, 5, 0, Some(pz_world::IsoGridSquare::new(4, 5, 0).into()));
        chunk.set_square(4, 5, 2, Some(pz_world::IsoGridSquare::new(4, 5, 2).into()));
        let mut cell = pz_world::IsoCell::default();
        cell.chunks.insert((0, 0), chunk);
        crate::bind_world::set_cell(Some(cell));

        let (mut t, player, _) = setup_with(ScriptManager::new());
        crate::bind_world::register(&mut t);
        crate::expose::expose_enum(
            &mut t,
            crate::bind_iso_enums::ISO_DIRECTIONS,
            &["N", "NW", "W", "SW", "S", "SE", "E", "NE"],
        );

        let r = SheetRopeSquare { sheet_rope: true, can_climb_here: true, ..Default::default() };
        let mut world = MockClimbWorld {
            rope: HashMap::from([
                ((4, 5, 0), SheetRopeSquare { solid_floor: true, ..r }),
                ((4, 5, 1), r),
                ((4, 5, 2), SheetRopeSquare { top: true, ..r }),
            ]),
            wall_dir: Some(IsoDirections::N),
            zombies: Vec::new(),
        };
        let sm = shared_script_manager();
        let mut z = LuaGameCharacter::new(&sm, pz_characters::character::CharacterKind::Zombie);
        z.id = 99;
        world.zombies.push(z);
        crate::bind_states::install_state_world(Rc::new(RefCell::new(world)));

        {
            let mut p = player.borrow_mut();
            p.path.current_square = Some((4, 5, 0));
            (p.vars.x, p.vars.y, p.vars.z) = (4.5, 5.5, 0.0);
        }

        // Sheet rope & wall queries + climb request queueing + state flags.
        run(
            &mut t,
            r#"
            assert(player:canClimbSheetRope(nil) == false)
            assert(player:canClimbOverWall(IsoDirections.N) == true)
            assert(player:canClimbOverWall(IsoDirections.E) == false)
            assert(player:climbOverWall(IsoDirections.E) == false)
            assert(player:climbOverWall(IsoDirections.N) == true)
            player:climb()
            player:climbDown()
            assert(player:canClimbDownSheetRopeInCurrentSquare() == false)
            assert(player:nearbyZombieClimbPenalty() == 7.0)
            assert(player:getClimbingFailChanceInt() == 2)
            assert(player:getClimbingFailChanceFloat() == 2.0)
            assert(player:getClimbRopeSpeed(false) > 0)
            player:setbClimbing(true)
            assert(player:isClimbing() == true)
            player:setbClimbing(false)
            assert(player:isClimbing() == false)
            player:setClimbRopeTime(3.5)
            assert(player:getClimbRopeTime() == 3.5)
            player:setClimbOverWallSuccess(true)
            assert(player:isClimbOverWallSuccess() == true)
            player:setClimbOverWallStruggle(true)
            assert(player:isClimbOverWallStruggle() == true)
            "#,
        )
        .unwrap();

        assert_eq!(
            player.borrow().path.requests,
            vec![
                PathMoverRequest::ClimbOverWall(IsoDirections::N),
                PathMoverRequest::ClimbSheetRope,
                PathMoverRequest::ClimbDownSheetRope,
            ]
        );

        // `canClimbSheetRope(sq)` / `canClimbDownSheetRope(sq)` with IsoGridSquare.
        let sq0 = crate::bind_world::square_value(4, 5, 0);
        let sq2 = crate::bind_world::square_value(4, 5, 2);
        t.globals().set_str("sq0", sq0);
        t.globals().set_str("sq2", sq2);
        run(
            &mut t,
            r#"
            assert(player:canClimbSheetRope(sq0) == true)
            assert(player:canClimbDownSheetRope(sq0) == false)
            assert(player:canClimbDownSheetRope(sq2) == true)
            "#,
        )
        .unwrap();

        // `fallFromRope()` clears climbing and starts falling when in ClimbSheetRopeState.
        {
            let sm = shared_script_manager();
            let chars = shared_characters();
            let chars_ref = chars.borrow();
            let ctx = CONTEXT.with(|c| c.borrow().clone());
            let mut nw = NullWorld;
            let mut cx = StateCx::new(&sm, &chars_ref, &*ctx, &mut nw);
            let mut p = player.borrow_mut();
            p.vars.climbing = true;
            p.vars.z = 1.5;
            p.change_state(Some(pz_characters::character::states::climb_rope::climb_sheet_rope()), &mut cx);
        }
        run(&mut t, "player:fallFromRope()").unwrap();
        {
            let p = player.borrow();
            assert!(!p.vars.climbing && p.vars.falling && p.path.collidable);
            assert_eq!(p.vars.llz, 1.5);
        }

        // `DoLand(hardSpeed)` drops primary hand item onto installed floor world.
        let floor = Rc::new(RefCell::new(MockFloor::default()));
        drop(crate::bind_character_sim::install_floor_world(Some(floor.clone())));
        let mut raw_spoon = InventoryItem::create(
            Rc::new(ItemTemplate::new(ItemClass::Normal, "Base", "Spoon")),
            1,
        );
        raw_spoon.id = 77;
        let spoon = shared::try_new_item(raw_spoon).unwrap();
        {
            let mut p = player.borrow_mut();
            p.in_players = true;
            let inv = p.items.inventory.clone();
            shared::add_item(&inv, &spoon);
            p.set_primary_hand_item(&sm, Some(77));
        }
        let hard = (pz_characters::character::fall::hard_fall_threshold()
            + pz_characters::character::fall::severe_fall_threshold())
            / 2.0;
        run(&mut t, &format!("player:DoLand({hard})")).unwrap();
        assert_eq!(player.borrow().items.primary_hand_item, None);
        assert_eq!(floor.borrow().0, vec![((4, 5, 0), 77)]);
    }
}
