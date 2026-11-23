//! IsoGridSquare.getPuddlesInGround, used by footsteps and ground fluid interactions.
use crate::bind_world_object::{borrowed, host_error};
use pz_entity::runtime_load::RuntimeHostError;
use pz_iso_render::puddles::ground::{directions, perlin_level, texture_level};
use pz_kahlua::LuaResult;

pub fn ground_level((x, y, z): (i32, i32, i32)) -> LuaResult<f32> {
    let mut owner = None;
    let room = crate::bind_world::try_with_world_read(|w| {
        owner = w.cell.as_ref().and_then(|c| c.get_grid_square(x, y, z));
        let Some(sq) = &owner else { return Ok(false) };
        let sq = sq
            .try_borrow()
            .map_err(|_| RuntimeHostError::Borrowed("puddle square"))?;
        if w.meta_grid
            .as_ref()
            .is_some_and(|m| sq.room_def(m).is_some())
        {
            return Ok(true);
        }
        let Some(regions) = &w.regions else {
            return Ok(false);
        };
        let regions = regions
            .try_borrow()
            .map_err(|_| RuntimeHostError::Borrowed("IsoRegions"))?;
        Ok(sq
            .get_iso_world_region(&regions)
            .is_some_and(|id| regions.is_player_room(id)))
    })
    .map_err(host_error)?;
    if room {
        return Ok(-1.0);
    }
    let Some(owner) = owner else { return Ok(-0.1) };
    let server = crate::bind_character_sim::current_sim_context().is_server();
    let player = crate::hud::host::with_host(|h| h.camera.frame_player_index());
    let core = crate::bind_core::shared_core();
    let (perf, enabled, low_quality, width) = {
        let core = core.try_borrow().map_err(|_| borrowed("Core"))?;
        let perf = core.get_perf_puddles();
        (
            perf,
            core.get_use_shaders()
                && (server || (core.get_perf_puddles_on_load() != 3 && perf != 3))
                && !(perf > 0 && z > 0),
            !server && core.performance.puddles_quality == 2,
            core.get_offscreen_width(player as i32) as f32,
        )
    };
    let size = super::shared_puddles()
        .try_borrow()
        .map_err(|_| borrowed("IsoPuddles"))?
        .state
        .puddles_size
        .get();
    let key = size + perf as f32 + width;
    let (previous, value) = owner
        .try_borrow()
        .map_err(|_| borrowed("puddle square"))?
        .puddles_cache
        .get();
    if !server && (key - previous).abs() < 0.01 {
        return Ok(value);
    }
    let level = if enabled && size > 0.0 {
        compute_level(&owner, (x, y, z), size, perf, low_quality, server)?
    } else {
        -0.1
    };
    owner
        .try_borrow()
        .map_err(|_| borrowed("puddle square"))?
        .puddles_cache
        .set((key, level));
    Ok(level)
}

fn compute_level(
    owner: &pz_world::SquareRef,
    (x, y, z): (i32, i32, i32),
    size: f32,
    perf: i32,
    low_quality: bool,
    server: bool,
) -> LuaResult<f32> {
    let mut dir = [0.0, 0.0, 1.0];
    if !server && perf <= 1 {
        let sprites = crate::bind_sprites::manager()?;
        let sprites = sprites
            .try_borrow()
            .map_err(|_| borrowed("IsoSpriteManager"))?;
        dir = crate::bind_world::try_with_world_read(|w| {
            let sq = owner
                .try_borrow()
                .map_err(|_| RuntimeHostError::Borrowed("puddle square"))?;
            let cell = w
                .cell
                .as_ref()
                .ok_or(RuntimeHostError::InvalidHostState("puddle cell"))?;
            let meta = w
                .meta_grid
                .as_ref()
                .ok_or(RuntimeHostError::InvalidHostState("puddle meta grid"))?;
            let regions = w
                .regions
                .as_ref()
                .ok_or(RuntimeHostError::InvalidHostState("puddle region graph"))?
                .try_borrow()
                .map_err(|_| RuntimeHostError::Borrowed("IsoRegions"))?;
            let view = pz_world_render::WorldView {
                cell,
                meta,
                sprites: &sprites,
            };
            Ok(directions(&pz_world_render::render_world::puddle_geometry(
                &view, &regions, &sq, perf,
            )?))
        })
        .map_err(host_error)?;
    }

    let (wx, wy) = (
        x as f32 + 0.5 - z as f32 * 3.0,
        y as f32 + 0.5 - z as f32 * 3.0,
    );
    let raw = [
        pz_render::iso::x_to_screen(wx, wy),
        pz_render::iso::y_to_screen(wx, wy, 0.0, 0),
    ];
    let elevation = z as f32 * 7.0;
    if server {
        let uv = [
            (raw[0] - 90000.0) * 0.0008 + elevation,
            (raw[1] - 640000.0) * 0.0008 + elevation,
        ];
        return Ok(perlin_level(uv, dir, size));
    }
    let (player, (ox, oy), (jx, jy)) = crate::hud::host::with_host(|h| {
        let p = h.camera.frame_player_index();
        (p, h.camera.camera_offset(p), h.camera.fix_jiggly_square(p))
    });
    let (width, height, scale) = {
        let core = crate::bind_core::shared_core();
        let core = core.try_borrow().map_err(|_| borrowed("Core"))?;
        (
            core.get_offscreen_width(player as i32).max(1) as f32,
            core.get_offscreen_height(player as i32).max(1) as f32,
            core.tile_scale as f32,
        )
    };
    let mut offset = [ox - (jx - jy) * 32.0 * scale, oy - (jx + jy) * 16.0 * scale];
    let frag = [
        (raw[0] - ox.trunc()) / width,
        (raw[1] - oy.trunc()) / height,
    ];
    if low_quality {
        offset[1] = -offset[1];
    }
    offset[0] -= 90000.0;
    offset[1] -= 640000.0;
    let fy = if low_quality {
        let span = (pz_iso_render::chunk::FLOOR_HEIGHT * 8) as f32;
        offset[0] += span;
        offset[1] += span;
        -frag[1]
    } else {
        frag[1]
    };
    let uv = [
        (frag[0] * width + offset[0]) * 0.0008 + elevation,
        (fy * height + offset[1]) * 0.0008 + elevation,
    ];
    if !low_quality {
        return Ok(perlin_level(uv, dir, size));
    }
    let Some(texture) = pz_render::get_shared_texture_ref("media/textures/puddles_hm.png")
        .map_err(crate::bind_render::texture_error)?
    else {
        return Ok(-1.0);
    };
    let handle =
        pz_render::try_texture_descriptor(&texture).map_err(crate::bind_render::texture_error)?;
    let textures = crate::bind_render::textures();
    let textures = textures
        .try_borrow()
        .map_err(|_| borrowed("texture pixels"))?;
    let image = handle
        .id
        .and_then(|id| textures.data(id))
        .and_then(|d| d.image.as_ref());
    Ok(image
        .and_then(|image| texture_level(uv, dir, size, image))
        .unwrap_or(-1.0))
}
