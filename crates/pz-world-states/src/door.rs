//! `IsoDoor.ToggleDoor(chr)` for a player: what the interact key does to a
//! door once `IsoPlayer.performContextualAction` has chosen it.
//!
//! Ported is the plain door: the key id of the door and of its building,
//! the lock (`canBeOpenFromInside`, the keys of the player's inventory and
//! its key rings, the `CustomLock` mod data, the "Locked" outcome), the open
//! flag, the open and closed sprites, the sound, the square's recalculation
//! and the path-finding and Lua notifications. What the original does for
//! the other doors is not: [`StatesWorldAdapter::toggle_player_door`] fails
//! with `RuntimeHostError::Unsupported` for a double door, a garage door and
//! an `IsoThumpable` door instead of guessing.

use pz_characters::character::player::context::DoorActor;
use pz_inventory::floor_drop::{HostResult, RuntimeHostError};
use pz_inventory::lua_table::LuaValue;
use pz_pathfind::mover::ObjectRef;
use pz_world::objects::ObjectKind;
use pz_world::save::lua_table::LuaKey;

use crate::StatesWorldAdapter;

fn unsupported<T>(operation: &'static str) -> HostResult<T> {
    Err(RuntimeHostError::Unsupported { operation, kind: None })
}

impl StatesWorldAdapter<'_> {
    /// `IsoDoor.isDoorObstructed(door)` without its vehicle test: a solid
    /// square, a solid transparent square or a tree blocks the door.
    // TODO(vehicles): the original also checks the vehicles of the chunks
    // around the square (`BaseVehicle.isIntersectingSquareWithShadow`).
    fn door_obstructed(&self, r: ObjectRef) -> bool {
        let Some(sq) = self.at((r.x, r.y, r.z)) else { return false };
        let sq = sq.borrow();
        sq.is_solid(self.sprites) || sq.is_solid_trans(self.sprites) || sq.has_type(self.sprites.tables.ty.tree)
    }

    /// `IsoDoor.canBeOpenFromInside(chr)` for a player: standing inside, in
    /// the room of either side of the door, and the door is not
    /// `forceLocked`.
    fn can_be_open_from_inside(&self, r: ObjectRef, actor: &DoorActor<'_>) -> bool {
        let Some((sq_owner, o_owner)) = self.find(r) else { return false };
        let (outside, same_room) = {
            let sq = sq_owner.borrow();
            let o = o_owner.borrow();
            let north = matches!(&o.kind, ObjectKind::Door(d) if d.north);
            let room = |p: (i32, i32, i32)| self.at(p).map(|s| s.borrow().room_id);
            let mine = room(actor.square);
            let outside = self.at(actor.square).is_some_and(|s| s.borrow().is_exterior(self.sprites));
            let opposite = self.opposite(&sq, north).map(|s| s.borrow().room_id);
            (outside, mine.is_some() && (mine == Some(sq.room_id) || mine == opposite))
        };
        !outside && same_room && !self.env().obj_has_prop(&o_owner.borrow(), "forceLocked")
    }

    /// `IsoDoor.checkKeyId()`: the door's key id, taken from the building
    /// of one of its sides when the door has none, and the `lockedByKey`
    /// flag of a locked door that has one.
    fn store_door_key_id(&mut self, r: ObjectRef) -> i32 {
        let Some((sq_owner, o_owner)) = self.find(r) else { return -1 };
        let key = {
            let sq = sq_owner.borrow();
            let o = o_owner.borrow();
            let north = matches!(&o.kind, ObjectKind::Door(d) if d.north);
            self.check_key_id(&sq, &o, north, false)
        };
        let mut o = o_owner.borrow_mut();
        if key != -1 {
            o.key_id = key;
        }
        let key_id = o.key_id;
        if let ObjectKind::Door(d) = &mut o.kind {
            if d.locked && !d.locked_by_key && key_id != -1 {
                d.locked_by_key = true;
            }
        }
        key_id
    }

    /// `IsoDoor.ToggleDoor(chr)` / `IsoDoor.ToggleDoorActual(chr)` of a
    /// local player in a single-player game.
    pub fn toggle_player_door(&mut self, r: ObjectRef, chr: i32, actor: &DoorActor<'_>) -> HostResult<()> {
        let Some((sq_owner, o_owner)) = self.find(r) else { return Ok(()) };
        let (hoppable, barricaded, double, garage) = {
            let o = o_owner.borrow();
            let sq = sq_owner.borrow();
            if !matches!(&o.kind, ObjectKind::Door(_)) {
                return unsupported("IsoThumpable.ToggleDoor");
            }
            let env = self.env();
            (
                o.is_hoppable(self.sprites),
                env.hooks().object_barricaded(&sq, &o),
                env.obj_has_prop(&o, "DoubleDoor"),
                env.obj_has_prop(&o, "GarageDoor"),
            )
        };
        let prefix = self.sound_prefix(&o_owner.borrow());
        // `locked = false; setLockedByKey(false)`.
        let unlock = || {
            if let ObjectKind::Door(d) = &mut o_owner.borrow_mut().kind {
                d.locked = false;
                d.locked_by_key = false;
            }
        };
        // A hoppable door (a gate) is never locked.
        if hoppable {
            unlock();
        }
        // TODO(sounds): the barricaded door's `TriggerBarricadedDoor` sound
        // and shake; the door itself stays as it is.
        if barricaded {
            return Ok(());
        }
        let key_id = self.store_door_key_id(r);
        // TODO(player): `timeSinceOpenDoor` / `timeSinceCloseDoor` of the
        // player, the light and special-path invalidation.
        if self.can_be_open_from_inside(r, actor) {
            unlock();
        }
        let (locked_by_key, open) = match &o_owner.borrow().kind {
            ObjectKind::Door(d) => (d.locked_by_key, d.open),
            _ => return Ok(()),
        };
        let custom_lock = matches!(
            o_owner.borrow().mod_data.as_ref().and_then(|t| t.rawget(&LuaKey::Str("CustomLock".into()))),
            Some(LuaValue::Bool(true))
        );
        let has_key = (actor.have_key)(key_id);
        if (locked_by_key || custom_lock) && !open {
            if !has_key {
                // TODO(sounds): "Locked" through the door's sound prefix,
                // `Hit_Door` render effect and `wasTryingToggleLockedDoor`.
                self.engine.play_sound(chr, &format!("{prefix}Locked"), Some(r));
                return Ok(());
            }
            self.engine.play_sound(chr, &format!("{prefix}Unlock"), Some(r));
            self.engine.play_sound(chr, &format!("{prefix}Open"), Some(r));
            unlock();
        }
        if garage {
            // The key counts only on the interior side of a garage door.
            return unsupported("IsoDoor.ToggleDoorActual of a garage door");
        }
        let locked = matches!(&o_owner.borrow().kind, ObjectKind::Door(d) if d.locked);
        let open = matches!(&o_owner.borrow().kind, ObjectKind::Door(d) if d.open);
        if locked && !has_key && !open {
            self.engine.play_sound(chr, &format!("{prefix}Locked"), Some(r));
            return Ok(());
        }
        if double {
            return unsupported("IsoDoor.toggleDoubleDoor");
        }
        // TODO(sounds): the "Blocked" sound and the halo note.
        if self.door_obstructed(r) {
            return Ok(());
        }
        let now_open = {
            let mut o = o_owner.borrow_mut();
            let ObjectKind::Door(d) = &mut o.kind else { return Ok(()) };
            d.locked = false;
            d.locked_by_key = false;
            d.open = !d.open;
            d.open
        };
        let sprite = {
            let o = o_owner.borrow();
            let ObjectKind::Door(d) = &o.kind else { return Ok(()) };
            if now_open { d.open_sprite } else { d.closed_sprite }
        };
        self.replace_object_sprite_tile(&o_owner, sprite);
        let sound = if now_open { "Open" } else { "Close" };
        self.engine.play_sound(chr, &format!("{prefix}{sound}"), Some(r));
        let p = (r.x, r.y, r.z);
        self.cell.recalc_all_with_neighbours(p.0, p.1, p.2, self.meta, self.sprites);
        self.engine.geometry_changed(self.cell, self.meta, self.sprites, p);
        self.engine.on_container_update();
        Ok(())
    }
}
