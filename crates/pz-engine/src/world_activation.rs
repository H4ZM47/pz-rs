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

/// The byte reader records an unresolved base sprite until the active manager
/// and its texture source are available. IsoObject.load creates a fresh sprite
/// in this case, even when the stored name does not resolve to a texture.
fn restore_map_sprite(
    object: &IsoObjectRef,
    manager: &pz_world::sprite::runtime::SpriteManagerRef,
    saved_names: Option<&std::collections::HashMap<i32, String>>,
) -> pz_entity::runtime_load::HostResult<()> {
    use pz_world::sprite::runtime::{create_runtime_sprite, load_single_texture, ObjectSprite};
    use pz_world::save::object_io::UNREGISTERED_SPRITE_ID;
    let (tables, name) = {
        let body = object.borrow();
        if !matches!(body.kind, pz_world::ObjectKind::Object)
            || !matches!(body.sprite, ObjectSprite::ArchiveUnmanaged(UNREGISTERED_SPRITE_ID))
        {
            return Ok(());
        }
        let sprites = manager.borrow();
        let name = body.sprite_name.as_ref().and_then(|name| name.resolve(saved_names, &sprites));
        (sprites.tables.clone(), name.map(str::to_owned))
    };
    let mut retired = pz_entity::RetiredComponents::new();
    let result = (|| {
        let sprite = create_runtime_sprite(manager, &tables, &mut retired)?;
        object.borrow_mut().sprite = ObjectSprite::Native(sprite.clone());
        if let Some(name) = name {
            load_single_texture(&sprite, &name, &mut retired)?;
        }
        Ok(())
    })();
    pz_inventory::component_host::retire_components(retired);
    result
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
                            if matches!(object.borrow().kind, pz_world::ObjectKind::FeedingTrough(_)) {
                                pz_lua_api::bind_world_object::trough::restore_fluid(&object)
                                    .map_err(|e| format!("restoring trough fluid at {pos:?}: {e}"))?;
                            }
                            restore_map_sprite(&object, sprites, self.items.as_ref().map(|items| &items.dictionary.sprite_names))
                                .map_err(|e| format!("restoring object sprite at {pos:?}: {e:?}"))?;
                            {
                                let mut regions = regions.borrow_mut();
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
                                            regions: &mut regions,
                                            rand: &mut self.rand,
                                            doors,
                                            world: &mut calls,
                                            pathfind: &mut path,
                                            render: &mut render,
                                            num_players: 1,
                                        };
                                        let id = object.borrow().id;
                                        env.object_add_to_world(ObjectRef { pos, id }, &object)
                                            .map_err(|error| {
                                                format!("object activation: {error:?}")
                                            })?;
                                        Ok::<bool, String>(true)
                                    } else {
                                        Ok(false)
                                    }
                                })?;
                                if !activated {
                                    return Err(
                                        "object activation requires live cell and meta grid".into(),
                                    );
                                }
                            }
                            for (receiver, call) in calls.0 {
                                match call {
                                    WorldCall::FeedingTroughCheck => {
                                        pz_lua_api::bind_world_object::trough::added_to_world(&mut lua.thread, &receiver)
                                            .map_err(|e| format!("placing trough at {pos:?}: {e}"))?;
                                    }
                                    WorldCall::HutchManager { add: true } => {
                                        pz_lua_api::bind_world_object::hutch::register_master(&receiver, pos)
                                            .map_err(|e| format!("adding hutch at {pos:?}: {e}"))?;
                                    }
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
                                        let sprite_name = {
                                            let sprites=self.sprites.as_ref().ok_or("mannequin activation requires actual sprite manager")?;
                                            let sprites = sprites.try_borrow().map_err(|_| {
                                                "mannequin activation sprite manager is borrowed"
                                            })?;
                                            let object = receiver.try_borrow().map_err(|_| {
                                                "mannequin activation receiver is borrowed"
                                            })?;
                                            let sprite = object.sprite.try_read(&sprites).map_err(
                                                |error| {
                                                    format!(
                                                        "mannequin activation sprite: {error:?}"
                                                    )
                                                },
                                            )?;
                                            sprite.and_then(|sprite| sprite.name.clone())
                                        };
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
                                            self.items.as_ref().map(|env| env as &dyn pz_inventory::ItemEnv),
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
                            // doLoadGridsquare initializes a pump immediately after addToWorld.
                            let pump = {
                                let sprites = sprites.borrow();
                                let body = object.borrow();
                                match &body.sprite {
                                    pz_world::objects::ObjectSprite::ArchiveUnmanaged(id) =>
                                        sprites.by_id(*id).is_some_and(|s| s.properties.has_name(&sprites.alias, "fuelAmount")),
                                    sprite => sprite.try_read(&sprites).map_err(|e| format!("{e:?}"))?
                                        .is_some_and(|s| s.properties.has_name(&sprites.alias, "fuelAmount")),
                                }
                            };
                            if pump {
                                let value = pz_lua_api::bind_world::square_queries::square_object_value(&object, pos)
                                    .map_err(|e| e.to_string())?;
                                let getter = lua.thread.table_get(&value, &pz_kahlua::Value::str("getPipedFuelAmount"))
                                    .map_err(|e| e.to_string())?;
                                lua.thread.call1(&getter, &[value]).map_err(|e| e.to_string())?;
                            }
                        }
                        ChunkActivationStep::SquareReady { square, x, y, z } => {
                            let (wx, wy) = (cursor.wx, cursor.wy);
                            let result = self.with_stream(|_, cell, ctx| {
                                // Erosion precedes the two MapObjects phases.
                                let hook: Option<
                                    &mut dyn pz_world::save::chunk_file::ChunkLoadHook,
                                > = match &mut ctx.chunk_hook {
                                    Some(h) => Some(&mut **h),
                                    None => None,
                                };
                                finish_square_erosion(
                                    cell,
                                    wx,
                                    wy,
                                    x,
                                    y,
                                    z,
                                    &square,
                                    ctx.sprites,
                                    hook,
                                )
                            });
                            result
                                .ok_or("chunk erosion requires streaming state")?
                                .map_err(|e| e.to_string())?;
                            let new_objects = with_world(|world| world.cell.as_ref()
                                .and_then(|cell| cell.chunks.get(&(wx, wy)))
                                .is_some_and(|chunk| chunk.add_zombies));
                            let sprites = self.sprites.as_ref().ok_or("MapObjects requires loaded sprites")?;
                            let dictionary = self.items.as_ref().map(|items| &items.dictionary.sprite_names);
                            for first_load in [true, false] {
                                if first_load && !new_objects { continue; }
                                pz_lua_api::bind_map_objects::dispatch_square(
                                    &mut lua.thread, &square, sprites, dictionary, first_load,
                                ).map_err(|e| e.to_string())?;
                            }
                            if lua.events.callbacks("LoadGridsquare") != 0 {
                                let value = {
                                    let sq = square.borrow();
                                    pz_lua_api::bind_world::square_value(sq.x, sq.y, sq.z)
                                };
                                lua.trigger_event("LoadGridsquare", &[value]);
                            }
                        }
                        ChunkActivationStep::Complete => {
                            let result = self.with_stream(|streamer, cell, ctx| {
                                streamer.finish_deferred_world_activation(&mut cursor, cell, ctx)
                            });
                            result
                                .ok_or("chunk completion requires streaming state")?
                                .map_err(|e| e.to_string())?;
                            // `doLoadGridsquare`: `loadGridSquareIfNeeded` for
                            // each square, levels then x then y; it logs an
                            // exception and goes on.
                            let (wx, wy) = (cursor.wx, cursor.wy);
                            let levels = with_world(|w| {
                                let chunk = w.cell.as_ref()?.chunks.get(&(wx, wy))?;
                                Some((chunk.min_level, chunk.max_level))
                            });
                            if let Some((min, max)) = levels {
                                let w = pz_world::chunk::CHUNK_WIDTH;
                                for z in min..=max {
                                    for x in 0..w {
                                        for y in 0..w {
                                            let (x, y) = (wx * w + x, wy * w + y);
                                            if let Err(e) =
                                                pz_lua_api::bind_item_picker::load_gridsquare(
                                                    &mut lua.thread,
                                                    x,
                                                    y,
                                                    z,
                                                )
                                            {
                                                eprintln!("LoadGridsquare {x},{y},{z}: {e}");
                                            }
                                        }
                                    }
                                }
                            }
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
