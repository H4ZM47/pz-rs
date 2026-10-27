//! One session-bound floor mutation service for Lua and borrowed state worlds.
//! Resources are the engine's actual shared owners. No cell, object or item
//! body is copied, and the state entry point never re-enters global WORLD.
use crate::iso_world::{IsoWorld, StandardRand};
use crate::world_render::WorldRender;
use pz_inventory::floor_drop::{FloorWorld, HostResult, RuntimeHostError, SquarePos};
use pz_inventory::shared::ItemRef;
use pz_lua_api::bind_character_sim::{floor_world_generation, install_floor_world};
use pz_lua_api::bind_world::{cell_generation, try_with_world};
use pz_world::isoregion::IsoRegions;
use pz_world::square_mutation::DoorTables;
use pz_world::{IsoCell, IsoMetaGrid, IsoSpriteManager};
use std::cell::RefCell;
use std::rc::{Rc, Weak};

fn unsupported(operation: &'static str) -> RuntimeHostError {
    RuntimeHostError::Unsupported {
        operation,
        kind: None,
    }
}

pub(crate) struct LiveFloorWorld {
    cell_generation: u64,
    provider_generation: u64,
    sprites: Weak<RefCell<IsoSpriteManager>>,
    regions: Weak<RefCell<IsoRegions>>,
    doors: Weak<DoorTables>,
    render: Weak<RefCell<Option<WorldRender>>>,
    session_owner: Weak<()>,
}

impl LiveFloorWorld {
    pub(crate) fn validate(&self) -> HostResult<()> {
        if self.cell_generation != cell_generation()
            || self.provider_generation != floor_world_generation()
        {
            return Err(unsupported("stale actual floor world session"));
        }
        if self.session_owner.strong_count() == 0
            || self.sprites.strong_count() == 0
            || self.regions.strong_count() == 0
            || self.doors.strong_count() == 0
            || self.render.strong_count() == 0
        {
            return Err(unsupported("expired actual floor world owner"));
        }
        Ok(())
    }

    pub(crate) fn fbo_render_chunk(&self) -> HostResult<bool> {
        self.validate()?;
        // The existing B42 WorldRender::invalidation uses this exact setting.
        // A not-yet-created renderer has no cached chunk levels to invalidate.
        Ok(true)
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn place_borrowed(
        &self,
        cell: &mut IsoCell,
        _meta: &IsoMetaGrid,
        _sprites: &mut IsoSpriteManager,
        square: SquarePos,
        item: &ItemRef,
        x: f32,
        y: f32,
        height: f32,
    ) -> HostResult<()> {
        let mut retired = pz_entity::RetiredComponents::new();
        retired.push_owner(pz_entity::EntityOwner::from_ref(item));
        pz_inventory::component_host::retire_components(retired);
        self.validate()?;
        self.place_prefix(cell, square, item, x, y, height)
    }

    fn place_prefix(
        &self,
        cell: &mut IsoCell,
        square: SquarePos,
        item: &ItemRef,
        x: f32,
        y: f32,
        height: f32,
    ) -> HostResult<()> {
        self.validate()?;
        let mut rand = StandardRand;
        let mut render = LazyRender(self);
        pz_world::square_mutation::NativeFloorPlacement {
            cell,
            rand: &mut rand,
            render: &mut render,
        }
        .place(square, item, x, y, height)
        .map(|_| ())
        .map_err(|error| match error {
            pz_world::square_mutation::WorldItemPlacementError::Component(error) => error,
            pz_world::square_mutation::WorldItemPlacementError::Binding(_) => {
                RuntimeHostError::InvalidHostState("floor constructor whole-owner binding failed")
            }
        })
    }
}

impl FloorWorld for LiveFloorWorld {
    fn solid_floor_at(&self, x: i32, y: i32, z: i32) -> HostResult<Option<SquarePos>> {
        self.validate()?;
        let sprites = self
            .sprites
            .upgrade()
            .ok_or_else(|| unsupported("actual floor sprite owner"))?;
        let sprites = sprites
            .try_borrow()
            .map_err(|_| RuntimeHostError::Borrowed("actual floor sprites"))?;
        try_with_world(|world| {
            let cell = world
                .cell
                .as_ref()
                .ok_or_else(|| unsupported("actual floor cell"))?;
            cell.try_get_solid_floor_at(x, y, z, &sprites)
        })
    }

    fn apparent_z(&self, square: SquarePos, dx: f32, dy: f32) -> HostResult<f32> {
        self.validate()?;
        let sprites = self
            .sprites
            .upgrade()
            .ok_or_else(|| unsupported("actual floor sprite owner"))?;
        let sprites = sprites
            .try_borrow()
            .map_err(|_| RuntimeHostError::Borrowed("actual floor sprites"))?;
        try_with_world(|world| {
            let cell = world
                .cell
                .as_ref()
                .ok_or_else(|| unsupported("actual floor cell"))?;
            cell.try_get_apparent_z(square, dx, dy, &sprites, self.fbo_render_chunk()?)
        })
    }

    fn rand_float(&mut self, min: f32, max: f32) -> HostResult<f32> {
        self.validate()?;
        Ok(pz_core::random::next_float(min, max))
    }

    fn add_world_inventory_item(
        &mut self,
        square: SquarePos,
        item: &ItemRef,
        x: f32,
        y: f32,
        height: f32,
    ) -> HostResult<()> {
        self.validate()?;
        try_with_world(|world| {
            let cell = world
                .cell
                .as_mut()
                .ok_or_else(|| unsupported("actual floor cell"))?;
            self.place_prefix(cell, square, item, x, y, height)
        })
    }
}

// This is the actual shared renderer operation at supplied-drop PC20.
// No regions, door tables, sprite body or post-constructor lifecycle is borrowed.
struct LazyRender<'a>(&'a LiveFloorWorld);
impl pz_world::square_mutation::NativeFloorRender for LazyRender<'_> {
    fn invalidate_render_chunk_level(
        &mut self,
        wx: i32,
        wy: i32,
        level: i32,
        flags: u64,
    ) -> HostResult<()> {
        self.0.validate()?;
        let render = self
            .0
            .render
            .upgrade()
            .ok_or_else(|| unsupported("actual floor renderer owner"))?;
        let mut render = render
            .try_borrow_mut()
            .map_err(|_| RuntimeHostError::Borrowed("actual floor renderer"))?;
        if let Some(render) = render.as_mut() {
            render.invalidate_floor_chunk(wx, wy, level, flags);
        }
        // None is the actual pre-first-frame owner with no cache allocation.
        Ok(())
    }
}

/// Session teardown can run before an IsoWorld was retained by App. The
/// provider owns only weak runtime resources, but retire even opaque test
/// providers after all outer host guards rather than dropping under TLS.
pub(crate) fn retire_installed_floor_world() {
    let old = pz_lua_api::bind_character_sim::reset_floor_world();
    let mut retired = pz_entity::RetiredComponents::new();
    retired.push_payload(Box::new(old));
    pz_inventory::component_host::retire_components(retired);
}

impl IsoWorld {
    pub(crate) fn install_floor_world(&mut self) -> Result<(), String> {
        let sprites = self
            .sprites
            .as_ref()
            .ok_or("floor service requires actual sprites")?;
        let regions = self
            .regions
            .as_ref()
            .ok_or("floor service requires actual regions")?;
        let doors = self
            .door_tables
            .as_ref()
            .ok_or("floor service requires native door tables")?;
        let session_owner = Rc::new(());
        let service = Rc::new(RefCell::new(LiveFloorWorld {
            cell_generation: cell_generation(),
            provider_generation: floor_world_generation().wrapping_add(1),
            sprites: Rc::downgrade(sprites),
            regions: Rc::downgrade(regions),
            doors: Rc::downgrade(doors),
            render: Rc::downgrade(&self.render),
            session_owner: Rc::downgrade(&session_owner),
        }));
        let floor: pz_lua_api::bind_character_sim::FloorWorldRef = service.clone();
        let old = install_floor_world(Some(floor));
        self.floor_session = Some(session_owner);
        self.floor_world = Some(service);
        let mut retired = pz_entity::RetiredComponents::new();
        retired.push_payload(Box::new(old));
        pz_inventory::component_host::retire_components(retired);
        Ok(())
    }
}

#[cfg(test)]
mod tests;
