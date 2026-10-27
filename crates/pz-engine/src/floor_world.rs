//! One session-bound floor mutation service for Lua and borrowed state worlds.
//! Resources are the engine's actual shared owners. No cell, object or item
//! body is copied, and the state entry point never re-enters global WORLD.
use crate::iso_world::{IsoWorld, StandardRand};
use crate::world_render::WorldRender;
use pz_inventory::floor_drop::{FloorSquare, FloorWorld, HostResult, RuntimeHostError, SquarePos};
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
    // Parsed immutable class constants, independent of manager body guards.
    sprite_tables: pz_world::sprite::SpriteTables,
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

    pub(crate) fn place_retained(
        &self,
        cell: &mut IsoCell,
        square: &pz_world::SquareRef,
        item: &ItemRef,
        x: f32,
        y: f32,
        height: f32,
    ) -> HostResult<()> {
        self.validate()?;
        let mut retired = pz_entity::RetiredComponents::new();
        retired.push_owner(pz_entity::EntityOwner::from_ref(item));
        retired.push_owner(pz_entity::EntityOwner::from_ref(square));
        pz_inventory::component_host::retire_components(retired);
        let mut rand = StandardRand;
        let mut render = LazyRender(self);
        let mut host = LiveNativeFloorHost(self);
        pz_world::square_mutation::NativeFloorPlacement {
            cell,
            rand: &mut rand,
            render: &mut render,
        }
        .place_retained(square, item, x, y, height, &mut host)
        .map(|_| ())
        .map_err(placement_error)
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

fn placement_error(error: pz_world::square_mutation::WorldItemPlacementError) -> RuntimeHostError {
    match error {
        pz_world::square_mutation::WorldItemPlacementError::Component(error) => error,
        pz_world::square_mutation::WorldItemPlacementError::Binding(_) => {
            RuntimeHostError::InvalidHostState("floor constructor whole-owner binding failed")
        }
    }
}

struct LiveNativeFloorHost<'a>(&'a LiveFloorWorld);
impl pz_world::square_mutation::NativeFloorHost for LiveNativeFloorHost<'_> {
    fn sprite_manager(&self) -> HostResult<Rc<RefCell<IsoSpriteManager>>> {
        self.0.validate()?;
        self.0
            .sprites
            .upgrade()
            .ok_or_else(|| unsupported("actual floor sprite manager owner"))
    }
    fn sprite_tables(&self) -> HostResult<pz_world::sprite::SpriteTables> {
        self.0.validate()?;
        Ok(self.0.sprite_tables.clone())
    }
    fn world_texture_enabled(&self) -> HostResult<bool> {
        self.0.validate()?;
        // This engine session implements the actual single-player role.
        Ok(true)
    }
    fn world_age_hours(&self) -> HostResult<f64> {
        let time = pz_lua_api::bind_climate::shared_game_time();
        let time = time
            .try_borrow()
            .map_err(|_| RuntimeHostError::Borrowed("floor constructor GameTime"))?;
        Ok(time.world_age_hours())
    }
    fn tile_scale(&self) -> HostResult<i32> {
        let core = pz_lua_api::bind_core::shared_core();
        let core = core
            .try_borrow()
            .map_err(|_| RuntimeHostError::Borrowed("floor updateSprite Core.tileScale"))?;
        Ok(core.tile_scale)
    }
    fn item_display_name(&self, item: &ItemRef) -> HostResult<String> {
        plain_item_display_name(item)
    }
    fn add_to_world(
        &mut self,
        object: &pz_world::IsoObjectRef,
        square: &pz_world::SquareRef,
        retired: &mut pz_entity::RetiredComponents,
    ) -> HostResult<()> {
        self.0.validate()?;
        pz_world::square_mutation::native_floor_add_to_world(
            object,
            square,
            &self.0.sprite_tables,
            retired,
        )
    }
    fn designation_item_on_ground(
        &mut self,
        _object: &pz_world::IsoObjectRef,
        _square: &pz_world::SquareRef,
    ) -> HostResult<()> {
        // Native performs an actual designation-zone registry query first.
        // Neither a missing runtime registry nor a nonfood item proves null.
        Err(unsupported("actual DesignationZoneAnimal.getZone registry"))
    }
}

/// Authenticated no-prefix path of ComboItem.getName(null). The full virtual
/// name/translation family remains a capability at its first selected branch.
fn plain_item_display_name(item: &ItemRef) -> HostResult<String> {
    use pz_entity::ComponentType;
    {
        let body = item
            .try_borrow()
            .map_err(|_| RuntimeHostError::Borrowed("floor getName class/item fluid"))?;
        if body.template.class != pz_inventory::ItemClass::Normal
            || !matches!(body.kind, pz_inventory::ItemKind::Plain)
        {
            return Err(unsupported("actual floor subclass getName dispatch"));
        }
        if body
            .components
            .get_ref(ComponentType::FluidContainer)
            .map_err(RuntimeHostError::Storage)?
            .is_some()
        {
            return Err(unsupported(
                "actual InventoryItem.getName item Fluid.getUiName",
            ));
        }
    }
    let world = pz_inventory::world_item::get_world_item_ref(item)?;
    if let Some(world) = world {
        let mut retired = pz_entity::RetiredComponents::new();
        retired.push_owner(world.clone());
        pz_inventory::component_host::retire_components(retired);
        let object =
            world
                .try_handle::<pz_world::IsoObject>()
                .ok_or(RuntimeHostError::InvalidHostState(
                    "getName worldItem allocation",
                ))?;
        let body = object
            .try_borrow()
            .map_err(|_| RuntimeHostError::Borrowed("getName world Fluid"))?;
        if !matches!(body.kind, pz_world::ObjectKind::WorldInventory(_)) {
            return Err(RuntimeHostError::InvalidHostState(
                "getName worldItem class",
            ));
        }
        if body
            .components
            .get_ref(ComponentType::FluidContainer)
            .map_err(RuntimeHostError::Storage)?
            .is_some()
        {
            return Err(unsupported(
                "actual InventoryItem.getName world Fluid UI name",
            ));
        }
    }
    let name = {
        let body = item
            .try_borrow()
            .map_err(|_| RuntimeHostError::Borrowed("getName remote controller/name"))?;
        if body.remote_control_id != -1 {
            return Err(unsupported("actual getName controller-linked translation"));
        }
        body.name.clone()
    };
    let script = pz_inventory::item_texture::get_script_item_ref(item)?
        .ok_or_else(|| unsupported("native getMechanicType null scriptItem diagnostic"))?;
    if script
        .0
        .try_borrow()
        .map_err(|_| RuntimeHostError::Borrowed("getName retained script vehicleType"))?
        .vehicle_type
        > 0
    {
        return Err(unsupported("actual getName mechanic-type translation"));
    }
    // ComboItem inherits base getBloodLevel()==0. Its independent broken
    // flag, condition and later component reads remain in native phase order.
    {
        let body = item
            .try_borrow()
            .map_err(|_| RuntimeHostError::Borrowed("getName broken flag"))?;
        if body.broken {
            return Err(unsupported("actual getName broken translation"));
        }
        if (body.condition as f32) < (body.condition_max as f32 / 3.0) {
            return Err(unsupported("actual getName worn translation"));
        }
    }
    {
        let body = item
            .try_borrow()
            .map_err(|_| RuntimeHostError::Borrowed("getName sharpness component"))?;
        if body.broken {
            return Err(unsupported("actual getName changed broken translation"));
        }
        if body
            .components
            .get_ref(ComponentType::Attributes)
            .map_err(RuntimeHostError::Storage)?
            .is_some()
        {
            return Err(unsupported("actual getName Attribute.Sharpness lookup"));
        }
    }
    {
        let body = item
            .try_borrow()
            .map_err(|_| RuntimeHostError::Borrowed("getName activated fields"))?;
        if body.can_be_activated && body.activated {
            return Err(unsupported("actual getName activated translation"));
        }
    }
    Ok(name)
}

fn actual_square(target: &FloorSquare) -> HostResult<pz_world::SquareRef> {
    target
        .owner_slot()
        .and_then(|owner| owner.try_handle::<pz_world::IsoGridSquare>())
        .ok_or_else(|| unsupported("actual retained square authority"))
}

impl FloorWorld for LiveFloorWorld {
    fn solid_floor_ref_at(&self, x: i32, y: i32, mut z: i32) -> HostResult<Option<FloorSquare>> {
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
            while z >= 0 {
                if let Some(square) = cell.get_grid_square(x, y, z) {
                    let solid = square
                        .try_borrow()
                        .map_err(|_| RuntimeHostError::Borrowed("solid floor square"))?
                        .treat_as_solid_floor(&sprites);
                    if solid {
                        return Ok(Some(FloorSquare::actual(
                            (x, y, z),
                            pz_entity::EntityOwner::from_ref(&square),
                        )));
                    }
                }
                z -= 1;
            }
            Ok(None)
        })
    }
    fn apparent_z_on(&self, target: &FloorSquare, dx: f32, dy: f32) -> HostResult<f32> {
        self.validate()?;
        let square = actual_square(target)?;
        let sprites = self
            .sprites
            .upgrade()
            .ok_or_else(|| unsupported("actual floor sprite owner"))?;
        let sprites = sprites
            .try_borrow()
            .map_err(|_| RuntimeHostError::Borrowed("actual floor sprites"))?;
        let body = square
            .try_borrow()
            .map_err(|_| RuntimeHostError::Borrowed("apparent floor square"))?;
        Ok(body.get_apparent_z(dx, dy, &sprites, self.fbo_render_chunk()?))
    }
    fn add_world_inventory_item_on(
        &mut self,
        target: &FloorSquare,
        item: &ItemRef,
        x: f32,
        y: f32,
        height: f32,
    ) -> HostResult<()> {
        self.validate()?;
        let square = actual_square(target)?;
        try_with_world(|world| {
            let cell = world
                .cell
                .as_mut()
                .ok_or_else(|| unsupported("actual floor cell"))?;
            self.place_retained(cell, &square, item, x, y, height)
        })
    }

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
        let sprite_tables = sprites
            .try_borrow()
            .map_err(|_| "floor service class metadata is borrowed")?
            .tables
            .clone();
        let service = Rc::new(RefCell::new(LiveFloorWorld {
            cell_generation: cell_generation(),
            provider_generation: floor_world_generation().wrapping_add(1),
            sprites: Rc::downgrade(sprites),
            regions: Rc::downgrade(regions),
            doors: Rc::downgrade(doors),
            render: Rc::downgrade(&self.render),
            session_owner: Rc::downgrade(&session_owner),
            sprite_tables,
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
