//! Main-thread object activation with world/sprite guards released at host
//! boundaries. The streamer prepares chunks; this coordinator completes their
//! object and per-square phases before publishing a renderer/population handoff.

use pz_lua_api::bind_world::with_world;
use pz_lua_api::LuaManager;
use pz_world::save::chunk_file::{finish_square_erosion, ChunkActivationStep};
use pz_world::square_mutation::{
    MutationEnv, NoMutationHooks, ObjectRef, SquareMutationHooks, WorldCall,
};
use pz_world::zones::ZoneKind;
use pz_world::{GameRand, IsoObjectRef};

use crate::iso_world::{opt_bool, sandbox, IsoWorld};

#[cfg(test)]
mod tests;

#[derive(Default)]
struct Calls(Vec<(IsoObjectRef, WorldCall)>);
impl SquareMutationHooks for Calls {
    fn world_call(
        &mut self,
        _: ObjectRef,
        object: IsoObjectRef,
        _: &mut dyn GameRand,
        call: WorldCall,
    ) {
        self.0.push((object, call));
    }
}

impl IsoWorld {
    /// Drain prepared chunks before subsequent world observers. No arbitrary
    /// callback is invoked inside with_stream or the WORLD/sprite guards.
    pub(crate) fn activate_streamed_chunks(&mut self, lua: &mut LuaManager) -> Result<(), String> {
        if let Some((_, error)) = &self.failed_chunk_activation {
            return Err(error.clone());
        }
        while let Some(mut cursor) = self
            .streamer
            .as_mut()
            .and_then(|s| s.pop_pending_world_activation())
        {
            let result = (|| -> Result<(), String> {
                loop {
                    let next = with_world(|w| w.cell.as_ref().map(|cell| cursor.next_step(cell)));
                    let next = next
                        .ok_or("chunk activation requires the live cell")?
                        .map_err(|e| e.to_string())?;
                    match next {
                        ChunkActivationStep::Object { square, object, .. } => {
                            let pos = {
                                let sq = square.borrow();
                                (sq.x, sq.y, sq.z)
                            };
                            let mut calls = Calls::default();
                            let (Some(sprites), Some(regions), Some(doors)) = (
                                &self.sprites,
                                self.regions.as_mut(),
                                self.door_tables.as_ref(),
                            ) else {
                                return Err("object activation requires sprites, regions and native door tables".into());
                            };
                            {
                                let mut sprites = sprites.borrow_mut();
                                let activated = with_world(|w| {
                                    if let (Some(cell), Some(meta)) =
                                        (w.cell.as_mut(), w.meta_grid.as_ref())
                                    {
                                        let mut path = NoMutationHooks;
                                        let mut render = NoMutationHooks;
                                        let mut env = MutationEnv {
                                            cell,
                                            meta,
                                            sprites: &mut sprites,
                                            regions,
                                            rand: &mut self.rand,
                                            doors,
                                            world: &mut calls,
                                            pathfind: &mut path,
                                            render: &mut render,
                                            num_players: 1,
                                        };
                                        let id = object.borrow().id;
                                        env.object_add_to_world(ObjectRef { pos, id }, &object);
                                        true
                                    } else {
                                        false
                                    }
                                });
                                if !activated {
                                    return Err(
                                        "object activation requires live cell and meta grid".into(),
                                    );
                                }
                            }
                            for (receiver, call) in calls.0 {
                                match call {
                                    WorldCall::MannequinInit => {
                                        let sandbox = sandbox(lua).ok_or(
                                            "mannequin activation requires sandbox options",
                                        )?;
                                        if sandbox
                                            .borrow()
                                            .option_by_name("NoBlackClothes")
                                            .is_none()
                                        {
                                            return Err("NoBlackClothes is missing from installed sandbox definitions".into());
                                        }
                                        let no_black_clothes =
                                            opt_bool(&sandbox.borrow(), "NoBlackClothes");
                                        let sprite_name = self.sprites.as_ref().and_then(|s| {
                                            let index = receiver.borrow().sprite?;
                                            s.borrow().sprites.get(index)?.name.clone()
                                        });
                                        let zone = with_world(|w| {
                                            let attached = square
                                                .borrow()
                                                .objects
                                                .iter()
                                                .any(|o| o.ptr_eq_ref(&receiver));
                                            if !attached {
                                                return None;
                                            }
                                            let meta = w.meta_grid.as_ref()?;
                                            let cell =
                                                meta.get_cell_data(pos.0 / 256, pos.1 / 256)?;
                                            cell.mannequin_zones.iter().find_map(|id| {
                                                let zone = meta.zone(*id);
                                                if !zone.contains(pos.0, pos.1, pos.2) {
                                                    return None;
                                                }
                                                if let ZoneKind::Mannequin(data) = &zone.kind {
                                                    Some(data.clone())
                                                } else {
                                                    None
                                                }
                                            })
                                        });
                                        pz_lua_api::world_mannequins::activate(
                                            &mut lua.thread,
                                            &receiver,
                                            sprite_name.as_deref(),
                                            zone.as_ref(),
                                            &self.game_dir,
                                            no_black_clothes,
                                        )
                                        .map_err(
                                            |error| {
                                                format!(
                                                    "IsoMannequin.addToWorld {},{},{}: {error}",
                                                    pos.0, pos.1, pos.2
                                                )
                                            },
                                        )?;
                                    }
                                    // The lifecycle's data changes are applied above.
                                    // Other subsystem registries still need their
                                    // actual engine adapters; do not equate these
                                    // queued notifications with completed effects.
                                    _ => {}
                                }
                            }
                        }
                        ChunkActivationStep::SquareReady { square, x, y, z } => {
                            let (wx, wy) = (cursor.wx, cursor.wy);
                            let result = self.with_stream(|_, cell, ctx| {
                                // Production erosion is not installed yet. Keep the
                                // exact per-square seam for its real hook and the
                                // subsequent MapObjects/Lua observer adapter.
                                finish_square_erosion(
                                    cell,
                                    wx,
                                    wy,
                                    x,
                                    y,
                                    z,
                                    &square,
                                    ctx.sprites,
                                    None,
                                )
                            });
                            result
                                .ok_or("chunk erosion requires streaming state")?
                                .map_err(|e| e.to_string())?;
                        }
                        ChunkActivationStep::Complete => {
                            let result = self.with_stream(|streamer, cell, ctx| {
                                streamer.finish_deferred_world_activation(&mut cursor, cell, ctx)
                            });
                            result
                                .ok_or("chunk completion requires streaming state")?
                                .map_err(|e| e.to_string())?;
                            self.with_stream(|streamer, cell, ctx| {
                                streamer.process_next_pending_load_grid_square(cell, ctx)
                            });
                            break;
                        }
                    }
                }
                Ok(())
            })();
            if let Err(error) = result {
                self.failed_chunk_activation = Some((cursor, error.clone()));
                return Err(error);
            }
        }
        Ok(())
    }
}
