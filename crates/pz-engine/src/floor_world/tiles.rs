//! IsoGridSquare.addTileObject(String) uses the same live mutation resources
//! as the world's other tile edits. Lua callbacks run after the edit guards end.
use super::*;
use pz_lua_api::{bind_map_objects, bind_world::square_queries::square_object_value};
use pz_world::square_mutation::{MutationEnv, NoMutationHooks, RenderSquareHooks};
use pz_world::{IsoObject, IsoObjectRef, ObjectKind, ObjectSlot, SquareRef};

impl LiveFloorWorld {
    fn add_tile(&self, pos: SquarePos, name: &str) -> HostResult<(IsoObjectRef, SquareRef)> {
        self.validate()?;
        let sprites = self.sprites.upgrade().ok_or_else(|| unsupported("tile sprites"))?;
        let regions = self.regions.upgrade().ok_or_else(|| unsupported("tile regions"))?;
        let doors = self.doors.upgrade().ok_or_else(|| unsupported("tile door tables"))?;
        let render = self.render.upgrade().ok_or_else(|| unsupported("tile renderer"))?;
        let mut sprites = sprites.try_borrow_mut().map_err(|_| RuntimeHostError::Borrowed("tile sprites"))?;
        let mut regions = regions.try_borrow_mut().map_err(|_| RuntimeHostError::Borrowed("tile regions"))?;
        let mut render = render.try_borrow_mut().map_err(|_| RuntimeHostError::Borrowed("tile renderer"))?;
        let players = pz_lua_api::bind_players::num_players();
        let mut invalidation = render.as_mut().map(|r| r.invalidation(sprites.tables.flag.water, players));
        let mut no_render = NoMutationHooks;
        let render: &mut dyn RenderSquareHooks = match invalidation.as_mut() {
            Some(r) => r,
            None => &mut no_render,
        };
        try_with_world(|world| {
            let cell = world.cell.as_mut().ok_or_else(|| unsupported("tile cell"))?;
            let meta = world.meta_grid.as_ref().ok_or_else(|| unsupported("tile meta grid"))?;
            let square = cell.get_grid_square(pos.0, pos.1, pos.2).ok_or_else(|| unsupported("tile square"))?;
            let mut object = IsoObject::new(0, ObjectKind::Object, sprites.named_map.get(name).copied());
            object.tile = Some(name.to_owned());
            object.replace_square(Some(&square), &mut pz_entity::RetiredComponents::new());
            let slot = ObjectSlot::new(object);
            let object = slot.ref_handle();
            let mut pathfind = NoMutationHooks;
            let mut hooks = NoMutationHooks;
            MutationEnv {
                cell, meta, sprites: &mut sprites, regions: &mut regions, doors: &doors,
                rand: &mut StandardRand, world: &mut hooks, pathfind: &mut pathfind, render,
                num_players: players,
            }.add_tile_object(pos, slot, -1)?;
            world.geometry.notify(cell, meta, &sprites, pos);
            Ok((object, square))
        })
    }
}

impl IsoWorld {
    pub(super) fn install_tile_placement(&self, service: &Rc<RefCell<LiveFloorWorld>>) {
        let service = Rc::downgrade(service);
        // WorldDictionary's persisted sprite-name overrides are loaded at world entry.
        let saved_names = self.items.as_ref().map(|i| i.dictionary.sprite_names.clone());
        bind_map_objects::install_tile_placement(Some(Rc::new(move |thread, pos, name| {
            let service = service.upgrade().ok_or_else(|| pz_kahlua::LuaError::runtime("tile world has closed"))?;
            let ((object, square), sprites) = {
                let service = service.try_borrow().map_err(|_| pz_kahlua::LuaError::runtime("tile world is in use"))?;
                let result = service.add_tile(pos, name).map_err(|e| pz_kahlua::LuaError::runtime(format!("{e:?}")))?;
                let sprites = service.sprites.upgrade().ok_or_else(|| pz_kahlua::LuaError::runtime("tile sprites have expired"))?;
                (result, sprites)
            };
            for new_objects in [true, false] {
                bind_map_objects::dispatch_square(thread, &square, &sprites, saved_names.as_ref(), new_objects)?;
            }
            square_object_value(&object, pos)
        })));
    }
}
