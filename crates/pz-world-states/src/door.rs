//! `IsoDoor.ToggleDoor(chr)` for a player: what the interact key does to a
//! door once `IsoPlayer.performContextualAction` has chosen it.
//!
//! Ported is the plain door: the open flag, the open and closed sprites,
//! the sound, the square's recalculation and the path-finding and Lua
//! notifications. What the original does for the other doors is not:
//! [`StatesWorldAdapter::toggle_player_door`] fails with
//! `RuntimeHostError::Unsupported` for a locked door (keys, custom locks,
//! the building's key id), a double door, a garage door and an
//! `IsoThumpable` door instead of guessing.

use pz_inventory::floor_drop::{HostResult, RuntimeHostError};
use pz_pathfind::mover::ObjectRef;
use pz_world::objects::ObjectKind;

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

    /// `IsoDoor.ToggleDoor(chr)` / `IsoDoor.ToggleDoorActual(chr)` of a
    /// local player in a single-player game.
    pub fn toggle_player_door(&mut self, r: ObjectRef, chr: i32) -> HostResult<()> {
        let Some((sq_owner, o_owner)) = self.find(r) else { return Ok(()) };
        let (hoppable, barricaded, double, garage, locked, open) = {
            let o = o_owner.borrow();
            let sq = sq_owner.borrow();
            let ObjectKind::Door(d) = &o.kind else {
                return unsupported("IsoThumpable.ToggleDoor");
            };
            let env = self.env();
            (
                o.is_hoppable(self.sprites),
                env.hooks().object_barricaded(&sq, &o),
                env.obj_has_prop(&o, "DoubleDoor"),
                env.obj_has_prop(&o, "GarageDoor"),
                d.locked,
                d.open,
            )
        };
        // A hoppable door (a gate) is never locked.
        let locked = locked && !hoppable;
        if hoppable {
            if let ObjectKind::Door(d) = &mut o_owner.borrow_mut().kind {
                d.locked = false;
                d.locked_by_key = false;
            }
        }
        // TODO(sounds): the barricaded door's `TriggerBarricadedDoor` sound
        // and shake; the door itself stays as it is.
        if barricaded {
            return Ok(());
        }
        if locked && !open {
            return unsupported("IsoDoor.ToggleDoorActual of a locked door");
        }
        if double {
            return unsupported("IsoDoor.toggleDoubleDoor");
        }
        if garage {
            return unsupported("IsoDoor.toggleGarageDoor");
        }
        // TODO(sounds): the "Blocked" sound and the halo note.
        if self.door_obstructed(r) {
            return Ok(());
        }
        let prefix = self.sound_prefix(&o_owner.borrow());
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
        self.engine.square_changed(p);
        self.engine.on_container_update();
        Ok(())
    }
}
