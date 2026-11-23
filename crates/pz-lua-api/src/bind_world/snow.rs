//! The loaded cell's ground snow, shared by IsoCell and character audio.
use crate::bind_world_object::{borrowed, host_error};
use pz_entity::runtime_load::{HostResult, RuntimeHostError};
use pz_iso_render::{snow::Noise2D, water::WaterGeometry, water::WaterInit, water_flow::IsoWaterFlow};
use pz_kahlua::LuaResult;
use pz_world::{sprite::IsoSpriteManager, IsoCell, IsoGridSquare};
use std::sync::OnceLock;

pub fn set_snow_target(target: i32) {
    let enabled = crate::bind_sandbox::current_bool("EnableSnowOnGround").unwrap_or(true);
    super::with_world(|w| {
        if let Some(cell) = &mut w.cell { cell.snow_target = if enabled { target } else { 0 }; }
    });
}

pub fn winter() -> bool {
    crate::bind_core::shared_core().borrow().is_force_snow() ||
        crate::bind_climate::shared_game_time().borrow().climate.get_season().is_some_and(|s| s.is_season(5))
}

// W, NW, N, SW, S, SE, E, NE in IsoWaterGeometry.init order.
const NEIGHBOURS: [(i32,i32);8] = [(-1,0),(-1,-1),(0,-1),(-1,1),(0,1),(1,1),(1,0),(1,-1)];

fn water_valid(cell: &IsoCell, sq: &IsoGridSquare, sprites: &IsoSpriteManager, flow: &IsoWaterFlow) -> HostResult<bool> {
    let flag = sprites.tables.flag.water;
    if sq.has(flag) { return Ok(true); }
    if flow.get_shore(sq.x, sq.y) != 1 { return Ok(false); }
    let mut adjacent = [None;8];
    for (slot,(dx,dy)) in adjacent.iter_mut().zip(NEIGHBOURS) {
        let Some(other) = cell.get_grid_square(sq.x + dx, sq.y + dy, sq.z) else { return Ok(false) };
        *slot = Some(other.try_borrow().map_err(|_| RuntimeHostError::Borrowed("water neighbour"))?.has(flag));
    }
    if !adjacent.contains(&Some(true)) { return Ok(false); }
    let Some(floor) = sq.try_floor_index(sprites)? else { return Ok(false) };
    let body = sq.objects[floor].try_borrow().map_err(|_| RuntimeHostError::Borrowed("water floor"))?;
    let sprite = body.sprite.try_read(sprites)?;
    let inputs = WaterInit { shore_zone: 1, floor_sprite: sprite.as_ref().and_then(|s| s.name.as_deref()),
        neighbours: adjacent, ..WaterInit::default() };
    Ok(WaterGeometry::init(&inputs).is_some_and(|w| w.is_valid()))
}

pub fn square_is_snow(x: i32, y: i32, z: i32) -> LuaResult<bool> {
    let target = super::try_with_world_read(|w| Ok(w.cell.as_ref().map_or(0, |c| c.snow_target))).map_err(host_error)?;
    if target <= 0 || !winter() { return Ok(false); }
    static NOISE: OnceLock<Noise2D> = OnceLock::new();
    if !NOISE.get_or_init(Noise2D::snow).full_snow_tile(x, y, target) { return Ok(false); }
    let sprites = crate::bind_sprites::manager()?;
    let sprites = sprites.try_borrow().map_err(|_| borrowed("IsoSpriteManager"))?;
    let flow = super::lua_world::shared_water_flow();
    let flow = flow.try_borrow().map_err(|_| borrowed("IsoWaterFlow"))?;
    super::try_with_world_read(|w| {
        let Some(cell) = &w.cell else { return Ok(false) };
        let Some(owner) = cell.get_grid_square(x, y, z) else { return Ok(false) };
        let sq = owner.try_borrow().map_err(|_| RuntimeHostError::Borrowed("snow square"))?;
        let flags = &sprites.tables.flag;
        if sq.room_id != -1 || !sq.has(flags.solidfloor) || !sq.has(flags.exterior) || water_valid(cell, &sq, &sprites, &flow)? {
            return Ok(false);
        }
        if z == 0 {
            for (dx, dy) in NEIGHBOURS {
                if let Some(n) = cell.get_grid_square(x + dx, y + dy, z) {
                    let n = n.try_borrow().map_err(|_| RuntimeHostError::Borrowed("snow neighbour"))?;
                    if water_valid(cell, &n, &sprites, &flow)? { return Ok(false); }
                }
            }
        }
        Ok(true)
    }).map_err(host_error)
}
