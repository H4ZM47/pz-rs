//! `IsoPlayer.getUseableVehicle()` and `IsoPlayer.getNearVehicle()`: the
//! vehicle search of `pz_vehicles::interact` over the current cell.
//!
//! `ISVehicleMenu.getVehicleToInteractWith` and the key handlers in
//! `ISUIHandler` call them on every key press.

use super::*;
use crate::bind_character_sim::{
    chr_of, current_sim_context, ISO_GAME_CHARACTER_CLASS, ISO_PLAYER_CLASS,
};
use pz_vehicles::interact::{self, Interactor, Unsupported};
use pz_vehicles::vehicle::VehicleRef;

type Search = fn(&IsoCell, &Interactor) -> Result<Option<VehicleRef>, Unsupported>;

pub(super) fn register(thread: &mut Thread) {
    let p = expose_class(thread, ISO_PLAYER_CLASS, Some(ISO_GAME_CHARACTER_CLASS));
    p.method("getUseableVehicle", |c| search(c, interact::get_useable_vehicle));
    p.method("getNearVehicle", |c| search(c, interact::get_near_vehicle));
}

fn search(c: &mut NativeCall<'_>, find: Search) -> LuaResult<usize> {
    let chr = chr_of(c)?;
    let ctx = current_sim_context();
    let who = {
        let chr = chr.borrow();
        Interactor {
            x: chr.vars.x,
            y: chr.vars.y,
            z: chr.vars.z,
            forward: (chr.vars.forward_x, chr.vars.forward_y),
            in_vehicle: chr.path.vehicle.is_some(),
            is_local_player: chr.is_local_player(&*ctx),
            player_index: chr.player.player_index.max(0) as usize,
        }
    };
    // An unadopted vehicle or a missing pathfinding map is a gap in the
    // port, not an answer: it surfaces instead of becoming "no vehicle".
    let found = cell(|cell| find(cell, &who).map_err(|e| crate::bind_world_object::unsupported(e.0)))?;
    Ok(c.push(found.map_or(Value::Nil, |v| pz_vehicles::lua::vehicle_value(&v))))
}

#[cfg(test)]
mod tests;
