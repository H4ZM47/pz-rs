//! State floor queries use the borrowed actual cell; only placement is
//! delegated to the engine's real shared mutation service.
use crate::StatesWorldAdapter;
use pz_inventory::floor_drop::{FloorWorld, HostResult, SquarePos};
use pz_inventory::shared::ItemRef;

impl FloorWorld for StatesWorldAdapter<'_> {
    fn solid_floor_ref_at(
        &self,
        x: i32,
        y: i32,
        mut z: i32,
    ) -> HostResult<Option<pz_inventory::floor_drop::FloorSquare>> {
        self.engine.validate_floor_session()?;
        while z >= 0 {
            if let Some(square) = self.cell.get_grid_square(x, y, z) {
                let solid = square
                    .try_borrow()
                    .map_err(|_| {
                        pz_inventory::floor_drop::RuntimeHostError::Borrowed("solid floor square")
                    })?
                    .treat_as_solid_floor(self.sprites);
                if solid {
                    return Ok(Some(pz_inventory::floor_drop::FloorSquare::actual(
                        (x, y, z),
                        pz_entity::EntityOwner::from_ref(&square),
                    )));
                }
            }
            z -= 1;
        }
        Ok(None)
    }
    fn apparent_z_on(
        &self,
        target: &pz_inventory::floor_drop::FloorSquare,
        x: f32,
        y: f32,
    ) -> HostResult<f32> {
        self.engine.validate_floor_session()?;
        let square = target
            .owner_slot()
            .and_then(|owner| owner.try_handle::<pz_world::IsoGridSquare>())
            .ok_or(pz_inventory::floor_drop::RuntimeHostError::Unsupported {
                operation: "actual retained state floor square",
                kind: None,
            })?;
        let square = square.try_borrow().map_err(|_| {
            pz_inventory::floor_drop::RuntimeHostError::Borrowed("apparent floor square")
        })?;
        Ok(square.get_apparent_z(x, y, self.sprites, self.engine.floor_fbo_render_chunk()?))
    }
    fn add_world_inventory_item_on(
        &mut self,
        target: &pz_inventory::floor_drop::FloorSquare,
        item: &ItemRef,
        x: f32,
        y: f32,
        height: f32,
    ) -> HostResult<()> {
        self.engine.validate_floor_session()?;
        let square = target
            .owner_slot()
            .and_then(|owner| owner.try_handle::<pz_world::IsoGridSquare>())
            .ok_or(pz_inventory::floor_drop::RuntimeHostError::Unsupported {
                operation: "actual retained state floor square",
                kind: None,
            })?;
        self.engine.add_world_inventory_item_on(
            self.cell,
            self.meta,
            self.sprites,
            &square,
            item,
            x,
            y,
            height,
        )
    }

    fn solid_floor_at(&self, x: i32, y: i32, z: i32) -> HostResult<Option<SquarePos>> {
        self.engine.validate_floor_session()?;
        self.cell.try_get_solid_floor_at(x, y, z, self.sprites)
    }

    fn apparent_z(&self, square: SquarePos, dx: f32, dy: f32) -> HostResult<f32> {
        self.engine.validate_floor_session()?;
        self.cell.try_get_apparent_z(
            square,
            dx,
            dy,
            self.sprites,
            self.engine.floor_fbo_render_chunk()?,
        )
    }

    fn rand_float(&mut self, min: f32, max: f32) -> HostResult<f32> {
        self.engine.validate_floor_session()?;
        Ok(self.engine.rand().next_float_range(min, max))
    }

    fn add_world_inventory_item(
        &mut self,
        square: SquarePos,
        item: &ItemRef,
        x: f32,
        y: f32,
        height: f32,
    ) -> HostResult<()> {
        self.engine.validate_floor_session()?;
        self.engine.add_world_inventory_item(
            self.cell,
            self.meta,
            self.sprites,
            square,
            item,
            x,
            y,
            height,
        )
    }
}
