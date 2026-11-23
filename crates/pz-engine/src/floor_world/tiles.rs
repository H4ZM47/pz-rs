//! IsoGridSquare.addTileObject(String) uses the same live mutation resources
//! as the world's other tile edits. Lua callbacks run after the edit guards end.
use super::*;
use pz_lua_api::bind_map_objects::PlacedTile;
use pz_lua_api::{bind_map_objects, bind_world::square_queries::square_object_value};
use pz_world::square_mutation::{MutationEnv, NoMutationHooks, RenderSquareHooks};
use pz_world::{IsoObject, IsoObjectRef, ObjectKind, ObjectSlot, SquareRef};

impl LiveFloorWorld {
    fn with_tile_env<R>(
        &self,
        run: impl FnOnce(
            &mut MutationEnv<'_>,
            &pz_lua_api::bind_world::GeometryChanges,
        ) -> HostResult<R>,
    ) -> HostResult<R> {
        self.validate()?;
        let sprites = self
            .sprites
            .upgrade()
            .ok_or_else(|| unsupported("tile sprites"))?;
        let regions = self
            .regions
            .upgrade()
            .ok_or_else(|| unsupported("tile regions"))?;
        let doors = self
            .doors
            .upgrade()
            .ok_or_else(|| unsupported("tile door tables"))?;
        let render = self
            .render
            .upgrade()
            .ok_or_else(|| unsupported("tile renderer"))?;
        let mut sprites = sprites
            .try_borrow_mut()
            .map_err(|_| RuntimeHostError::Borrowed("tile sprites"))?;
        let mut regions = regions
            .try_borrow_mut()
            .map_err(|_| RuntimeHostError::Borrowed("tile regions"))?;
        let mut render = render
            .try_borrow_mut()
            .map_err(|_| RuntimeHostError::Borrowed("tile renderer"))?;
        let players = pz_lua_api::bind_players::num_players();
        let mut invalidation = render
            .as_mut()
            .map(|r| r.invalidation(sprites.tables.flag.water, players));
        let mut no_render = NoMutationHooks;
        let render: &mut dyn RenderSquareHooks = match invalidation.as_mut() {
            Some(r) => r,
            None => &mut no_render,
        };
        try_with_world(|world| {
            let cell = world
                .cell
                .as_mut()
                .ok_or_else(|| unsupported("tile cell"))?;
            let meta = world
                .meta_grid
                .as_ref()
                .ok_or_else(|| unsupported("tile meta grid"))?;
            let mut pathfind = NoMutationHooks;
            let mut hooks = super::tile_hooks::TileHooks {
                objects: cell.process_objects.clone(),
                items: cell.process_items.clone(),
            };
            let mut env = MutationEnv {
                cell,
                meta,
                sprites: &mut sprites,
                regions: &mut regions,
                doors: &doors,
                rand: &mut StandardRand,
                world: &mut hooks,
                pathfind: &mut pathfind,
                render,
                num_players: players,
            };
            run(&mut env, &world.geometry)
        })
    }

    fn add_tile(&self, pos: SquarePos, tile: PlacedTile) -> HostResult<(IsoObjectRef, SquareRef)> {
        self.with_tile_env(|env, geometry| {
            let square = env
                .cell
                .get_grid_square(pos.0, pos.1, pos.2)
                .ok_or_else(|| unsupported("tile square"))?;
            let (slot, index, special) = match tile {
                PlacedTile::Remove { .. } => {
                    return Err(unsupported("removing tile through add operation"))
                }
                PlacedTile::Named(name) => {
                    let mut object = IsoObject::new(
                        0,
                        ObjectKind::Object,
                        env.sprites.named_map.get(&name).copied(),
                    );
                    object.tile = Some(name);
                    object.replace_square(Some(&square), &mut pz_entity::RetiredComponents::new());
                    (ObjectSlot::new(object), -1, false)
                }
                PlacedTile::Existing {
                    object,
                    index,
                    special,
                } => (ObjectSlot::retain(&object), index, special),
            };
            let object = slot.ref_handle();
            if special {
                env.add_special_object(pos, slot, index)?;
            } else {
                env.add_tile_object(pos, slot, index)?;
            }
            geometry.notify(env.cell, env.meta, env.sprites, pos);
            Ok((object, square))
        })
    }
}

impl IsoWorld {
    pub(super) fn install_tile_placement(&self, service: &Rc<RefCell<LiveFloorWorld>>) {
        let service = Rc::downgrade(service);
        // WorldDictionary's persisted sprite-name overrides are loaded at world entry.
        let saved_names = pz_lua_api::bind_world::with_world(|world| world.sprite_names.clone());
        bind_map_objects::install_tile_placement(Some(Rc::new(move |thread, pos, tile| {
            let named = matches!(tile, PlacedTile::Named(_));
            let service = service
                .upgrade()
                .ok_or_else(|| pz_kahlua::LuaError::runtime("tile world has closed"))?;
            if let PlacedTile::Remove {
                object,
                transmit,
                safe,
            } = tile
            {
                return Ok(pz_kahlua::Value::from(remove_tiles(
                    thread, &service, pos, &object, transmit, safe,
                )?));
            }
            let ((object, square), sprites) = {
                let service = service
                    .try_borrow()
                    .map_err(|_| pz_kahlua::LuaError::runtime("tile world is in use"))?;
                let result = service
                    .add_tile(pos, tile)
                    .map_err(|e| pz_kahlua::LuaError::runtime(format!("{e:?}")))?;
                let sprites = service
                    .sprites
                    .upgrade()
                    .ok_or_else(|| pz_kahlua::LuaError::runtime("tile sprites have expired"))?;
                (result, sprites)
            };
            if matches!(object.borrow().kind, ObjectKind::FeedingTrough(_)) {
                pz_lua_api::bind_world_object::trough::added_to_world(thread, &object)?;
            }
            if !named {
                return Ok(pz_kahlua::Value::Nil);
            }
            for new_objects in [true, false] {
                bind_map_objects::dispatch_square(
                    thread,
                    &square,
                    &sprites,
                    Some(&saved_names),
                    new_objects,
                )?;
            }
            square_object_value(&object, pos)
        })));
    }
}

type RemoveTarget = (pz_world::square_mutation::ObjectRef, IsoObjectRef);

impl LiveFloorWorld {
    fn removal_targets(
        &self,
        pos: SquarePos,
        object: &IsoObjectRef,
        transmit: bool,
        safe: Option<bool>,
    ) -> HostResult<Vec<RemoveTarget>> {
        use pz_world::square_mutation::ObjectRef;
        self.with_tile_env(|env, _| {
            let Some(square) = env.cell.get_grid_square(pos.0, pos.1, pos.2) else {
                return Ok(Vec::new());
            };
            let square = square
                .try_borrow()
                .map_err(|_| RuntimeHostError::Borrowed("tile removal square"))?;
            let present = square.objects.iter().any(|slot| slot.ptr_eq_ref(object));
            if transmit && !present {
                return Ok(Vec::new());
            }
            let body = object
                .try_borrow()
                .map_err(|_| RuntimeHostError::Borrowed("tile removal object"))?;
            let safe = safe.unwrap_or_else(|| {
                transmit
                    || body.square_slot().is_some_and(|sq| {
                        sq.borrow()
                            .chunk_hot_save
                            .as_ref()
                            .is_some_and(|c| !c.borrow().prevent_hot_save)
                    })
            });
            let r = ObjectRef { pos, id: body.id };
            drop(body);
            drop(square);
            if !safe {
                return Ok(vec![(r, object.clone())]);
            }
            if !present {
                return Ok(Vec::new());
            }
            if !env.try_is_object_multi_square(r)? {
                return Ok(vec![(r, object.clone())]);
            }
            let mut targets = Vec::new();
            if let Some(parts) = env.try_get_all_multi_tile_objects(r)? {
                for part in parts {
                    if let Some(owner) = env.try_object_handle(part)? {
                        targets.push((part, owner));
                    }
                }
            }
            Ok(targets)
        })
    }
}

fn remove_tiles(
    thread: &mut pz_kahlua::Thread,
    service: &Rc<RefCell<LiveFloorWorld>>,
    pos: SquarePos,
    object: &IsoObjectRef,
    transmit: bool,
    safe: Option<bool>,
) -> pz_kahlua::LuaResult<i32> {
    use pz_kahlua::{LuaError, Value};
    let host_error = |e| LuaError::runtime(format!("removing map tile: {e:?}"));
    let get_service = || {
        service
            .try_borrow()
            .map_err(|_| LuaError::runtime("tile world is in use"))
    };
    let targets = get_service()?
        .removal_targets(pos, object, transmit, safe)
        .map_err(host_error)?;
    let trigger = thread.globals().get_str("triggerEvent");
    let mut target_index = -1;
    for (r, part) in targets {
        let pending = get_service()?
            .with_tile_env(|env, _| env.prepare_tile_removal(r.pos, r.id, Some(part.clone())))
            .map_err(host_error)?;
        let Some(pending) = pending else { continue };
        let value = if pending.actual_index.is_some() {
            square_object_value(&part, r.pos)?
        } else {
            pz_lua_api::bind_world_object::object_value(&part)?
        };
        if pending.actual_index.is_some() {
            thread.call1(
                &trigger,
                &[Value::str("OnObjectAboutToBeRemoved"), value.clone()],
            )?;
        }
        let (index, removed) = get_service()?
            .with_tile_env(|env, geometry| {
                if pending.actual_index.is_some() {
                    super::tile_hooks::check_tile_resources(&part, env.sprites)?;
                }
                let result = env.detach_removing_tile(pending)?;
                geometry.notify(env.cell, env.meta, env.sprites, r.pos);
                Ok(result)
            })
            .map_err(host_error)?;
        if removed.is_some() {
            try_with_world(|w| {
                if let Some(i) = w.hutches.iter().position(|old| Rc::ptr_eq(old, &part)) {
                    w.hutches.remove(i);
                }
                Ok(())
            })
            .map_err(host_error)?;
        }
        drop(removed);
        thread.call1(&trigger, &[Value::str("OnTileRemoved"), value])?;
        get_service()?
            .with_tile_env(|env, _| {
                env.finish_tile_removal(r.pos);
                Ok(())
            })
            .map_err(host_error)?;
        if Rc::ptr_eq(&part, object) {
            target_index = index;
        }
    }
    Ok(target_index)
}
