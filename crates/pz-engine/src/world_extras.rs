//! Per-world and per-frame `FrameExtras` state:
//! - Cell fog (`pz_iso_render::fog::ImprovedFog` + `FogFrame`)
//! - Corpse flies (`pz_iso_render::flies::CorpseFlies`)
//! - Vision cone (`pz_iso_render::vispoly::{VisPolyChunks, VisionFrame}`)
//! - World markers (`pz_iso_render::markers::MarkersFrame` from
//!   `pz_lua_api::hud::world_markers`)

use crate::world_render::RenderView;
use pz_fs::GameFs;
use pz_iso_render::flies::CorpseFlies;
use pz_iso_render::fog::{FogClimate, FogFrame, FogPlayer, ImprovedFog};
use pz_iso_render::frame::FrameExtras;
use pz_iso_render::markers::{GridSquareMarker, MarkersFrame};
use pz_iso_render::vispoly::{ChunkLevelData, VisPolyChunks, VisSquare, VisionFrame};
use pz_iso_render::Camera;
use pz_render::{Texture, TextureManager};
use pz_world::objects::ObjectKind;
use pz_world::los::TestResults;
use pz_world::square::{SquareHooks, VisionEnv};
use pz_world_render::{PendingRender, WorldLighting, WorldView};

struct DefaultSquareHooks;
impl SquareHooks for DefaultSquareHooks {}

/// Per-world state feeding [`FrameExtras`] in [`crate::world_render::WorldRender`].
#[derive(Default)]
pub struct WorldExtras {
    /// `ImprovedFog` static state updated once per tick in `IsoWorld.update`.
    pub fog: ImprovedFog,
    /// `media/textures/weather/fognew/fog_noise.png`.
    pub fog_noise: Option<Texture>,
    /// `CorpseFlies` animation state + `media/textures/CorpseFlies.png`.
    pub flies: CorpseFlies,
    /// `VisibilityPolygon2` per-chunk-level obstacle cache.
    pub vispoly: VisPolyChunks,
    /// `media/textures/worldMap/circle_center.png`.
    pub circle_center: Option<Texture>,
    /// `media/textures/worldMap/circle_only_highlight.png`.
    pub circle_only_highlight: Option<Texture>,
    /// `Texture.getWhite()`.
    pub white: Option<Texture>,
}

impl WorldExtras {
    /// Loads the extra pass textures and initializes `ImprovedFog` from the
    /// current `ClimateManager` fog intensity (`ImprovedFog.init`).
    pub fn new(fs: &GameFs, tm: &mut TextureManager) -> Self {
        let mut fog = ImprovedFog::default();
        let initial_fog = pz_lua_api::bind_climate::shared_game_time()
            .try_borrow()
            .map_or(0.0, |gt| gt.climate.get_fog_intensity());
        fog.init(initial_fog);
        let flies = CorpseFlies {
            texture: tm.shared_texture(fs, "media/textures/CorpseFlies.png"),
            ..Default::default()
        };
        WorldExtras {
            fog,
            fog_noise: tm.shared_texture(fs, "media/textures/weather/fognew/fog_noise.png"),
            flies,
            vispoly: VisPolyChunks::default(),
            circle_center: tm.shared_texture(fs, "media/textures/worldMap/circle_center.png"),
            circle_only_highlight: tm.shared_texture(fs, "media/textures/worldMap/circle_only_highlight.png"),
            white: Some(tm.white()),
        }
    }

    /// Per-tick updates run from `IsoWorld.update`: `ImprovedFog.update` and
    /// `CorpseFlies.update`.
    pub fn update_tick(&mut self, multiplier: f32, realworld_seconds: f32) {
        let fog_quality = pz_lua_api::bind_core::shared_core()
            .try_borrow()
            .map_or(0, |c| c.performance.fog_quality);
        if let Ok(gt) = pz_lua_api::bind_climate::shared_game_time().try_borrow() {
            let ext = gt.climate.get_color_new_fog().borrow().exterior;
            let climate = FogClimate {
                fog_intensity: gt.climate.get_fog_intensity(),
                color: [ext[0], ext[1], ext[2]],
                wind_angle_radians: gt.climate.get_wind_angle_radians(),
                wind_intensity: gt.climate.get_wind_intensity(),
                multiplier,
                fog_quality,
            };
            self.fog.update(&climate);
        }
        self.flies.update(realworld_seconds);
    }

    /// Drains `PendingRender::vispoly` invalidations and unloaded chunks into
    /// `VisPolyChunks`.
    pub fn drain_vispoly(&mut self, pending: &mut PendingRender) {
        for (wx, wy, level) in std::mem::take(&mut pending.vispoly) {
            self.vispoly.invalidate(wx, wy, level);
        }
    }

    /// Notify `VisPolyChunks` when chunks are added or removed from the cell.
    pub fn on_chunk_added(&mut self) {
        self.vispoly.add_chunk_to_world();
    }

    pub fn on_chunk_removed(&mut self, wx: i32, wy: i32) {
        self.vispoly.remove_chunk(wx, wy);
    }

    /// Builds this player's [`FrameExtras`] for `world_frame_extras`.
    pub fn build_frame(
        &mut self,
        view: &WorldView<'_>,
        cam: &Camera,
        v: &RenderView,
        lighting: &WorldLighting,
        order: &[(i32, i32)],
    ) -> FrameExtras<'static> {
        let (fog_quality, view_cone_opacity, num_players) = pz_lua_api::bind_core::shared_core()
            .try_borrow()
            .map_or((0, 3, 1), |c| {
                (
                    c.performance.fog_quality,
                    c.performance.view_cone_opacity,
                    c.num_players.max(1),
                )
            });
        let storm = pz_lua_api::bind_climate::shared_game_time()
            .try_borrow()
            .is_ok_and(|gt| {
                let wp = &gt.climate.weather_period;
                wp.is_running && (wp.is_thunder_storm || wp.is_tropical_storm)
            });
        let (sx, sy, sz) = (
            v.x.floor() as i32,
            v.y.floor() as i32,
            v.z.floor() as i32,
        );
        let player_sq = view.cell.get_grid_square(sx, sy, sz);
        let on_square = player_sq.is_some();
        let (in_room, inside_tree) = match player_sq {
            Some(sq_owner) => match sq_owner.try_borrow() {
                Ok(sq) => (
                    sq.room_id != -1,
                    square_has_blocking_tree(&sq, view.sprites),
                ),
                Err(_) => (false, false),
            },
            None => (false, false),
        };
        let (in_vehicle, look_vec) = pz_lua_api::bind_players::player(v.player)
            .and_then(|p| {
                p.try_borrow().ok().map(|c| {
                    let fx = c.vars.forward_x;
                    let fy = c.vars.forward_y;
                    let len = (fx * fx + fy * fy).sqrt();
                    let lv = if len > 1.0e-5 {
                        [fx / len, fy / len]
                    } else {
                        let (dx, dy) = v.dir.delta();
                        let dl = ((dx * dx + dy * dy) as f32).sqrt().max(1.0);
                        [dx as f32 / dl, dy as f32 / dl]
                    };
                    (c.path.vehicle.is_some(), lv)
                })
            })
            .unwrap_or_else(|| {
                let (dx, dy) = v.dir.delta();
                let dl = ((dx * dx + dy * dy) as f32).sqrt().max(1.0);
                (false, [dx as f32 / dl, dy as f32 / dl])
            });
        let look_angle = pz_iso_render::vispoly::direction(look_vec[0], look_vec[1]);

        let (x0, y0, n) = v.chunk_map;
        let chunk_map = [x0 * 8, y0 * 8, (x0 + n) * 8, (y0 + n) * 8];

        // Collect on-screen chunk levels at `floor(camZ)` in `order` for `VisibilityPolygon2`.
        let cam_z = pz_render::iso::fastfloor(cam.z);
        let mut vis_chunks = Vec::new();
        for &(wx, wy) in order {
            if !pz_iso_render::cache::level_on_screen(cam, v.width, v.height, wx, wy, cam_z) {
                continue;
            }
            let Some(chunk) = view.cell.chunks.get(&(wx, wy)) else {
                continue;
            };
            let adj_counter = neighbor_chunk_mask(view, wx, wy);
            let data = self.vispoly.level(wx, wy, cam_z, adj_counter, || {
                build_chunk_vispoly(view, chunk, wx, wy, cam_z)
            });
            vis_chunks.push(data.clone());
        }

        let vision = VisionFrame {
            vision_cone: lighting.vision_cone(),
            look_angle,
            look_vector: look_vec,
            reversing: false,
            on_square,
            inside_tree,
            chunk_map,
            chunks: vis_chunks,
            use_circle: false,
            view_cone_opacity,
            num_players,
            player_index: v.player as i32,
        };

        let fog = FogFrame {
            fog: self.fog.clone(),
            fog_quality,
            noise: self.fog_noise.clone(),
            player: FogPlayer {
                x: v.x,
                y: v.y,
                z: v.z,
                in_room,
                in_vehicle,
            },
            storm,
            right_click_off: [0.0, 0.0],
            offscreen_left: v.screen.offscreen_left as i32,
            offscreen_top: v.screen.offscreen_top as i32,
        };

        let grid_square_markers = collect_grid_square_markers();
        let markers = MarkersFrame {
            grid_square_markers,
            circle_center: self.circle_center.clone(),
            circle_only_highlight: self.circle_only_highlight.clone(),
            white: self.white.clone(),
            chunk_map: Some(chunk_map),
            iso_markers: Vec::new(),
            player_z: Some(v.z),
            lines: Vec::new(),
        };

        FrameExtras {
            flies: Some(self.flies.clone()),
            highlight: None,
            opaque_event: None,
            vision: Some(vision),
            markers: Some(markers),
            fog: Some(fog),
            area_highlights: None,
        }
    }
}

/// 4-neighbour chunk presence bitmask used as `adjacentChunkLoadedCounter` so
/// loading or unloading a neighbour forces a `ChunkLevelData::recreate`.
fn neighbor_chunk_mask(view: &WorldView<'_>, wx: i32, wy: i32) -> i64 {
    let mut mask = 0i64;
    for (i, (dx, dy)) in [(-1, 0), (1, 0), (0, -1), (0, 1)].iter().enumerate() {
        if view.cell.chunks.contains_key(&(wx + dx, wy + dy)) {
            mask |= 1 << i;
        }
    }
    mask
}

/// True when `sq` holds an `IsoTree` whose sprite has `blocksight`.
fn square_has_blocking_tree(
    sq: &pz_world::IsoGridSquare,
    sprites: &pz_world::IsoSpriteManager,
) -> bool {
    let blocksight = sprites.tables.flag.blocksight;
    sq.objects.iter().any(|slot| {
        let Ok(obj) = slot.try_borrow() else { return false };
        if !matches!(obj.kind, ObjectKind::Tree(_)) {
            return false;
        }
        obj.sprite
            .try_read(sprites)
            .ok()
            .flatten()
            .is_some_and(|s| s.properties.has(blocksight))
    })
}

/// Builds `ChunkLevelData` from `chunk`'s squares at `level` using `pz-world`'s
/// `IsoGridSquare::test_vision_adjacent` and sprite solid/tree/blocksight flags.
pub(crate) fn build_chunk_vispoly(
    view: &WorldView<'_>,
    chunk: &pz_world::IsoChunk,
    wx: i32,
    wy: i32,
    level: i32,
) -> ChunkLevelData {
    let hooks = DefaultSquareHooks;
    let env = VisionEnv::new(view.cell, view.sprites, &hooks);
    let mut squares: [Option<VisSquare>; 64] = [None; 64];
    if level >= chunk.min_level && level <= chunk.max_level {
        for y in 0..8 {
            for x in 0..8 {
                let Some(sq_owner) = chunk.get_grid_square(x, y, level) else {
                    continue;
                };
                let Ok(sq) = sq_owner.try_borrow() else {
                    continue;
                };
                let tree = sq.objects.iter().any(|slot| {
                    slot.try_borrow()
                        .is_ok_and(|o| matches!(o.kind, ObjectKind::Tree(_)))
                });
                let tree_blocks_sight = tree && square_has_blocking_tree(&sq, view.sprites);
                let solid = sq.is_solid(view.sprites);
                let vision_blocked_n =
                    sq.test_vision_adjacent(0, -1, 0, false, false, &env) == TestResults::Blocked;
                let vision_blocked_w =
                    sq.test_vision_adjacent(-1, 0, 0, false, false, &env) == TestResults::Blocked;
                squares[(x + y * 8) as usize] = Some(VisSquare {
                    solid,
                    tree,
                    tree_blocks_sight,
                    vision_blocked_n,
                    vision_blocked_w,
                });
            }
        }
    }
    ChunkLevelData::recreate(wx, wy, level, chunk.min_level, chunk.max_level, &squares)
}

/// Converts the active `WorldMarkers` grid-square markers into
/// `pz_iso_render::markers::GridSquareMarker` entries for `MarkersFrame`.
fn collect_grid_square_markers() -> Vec<GridSquareMarker> {
    pz_lua_api::hud::world_markers::with_markers(|m| {
        m.grid_square_markers()
            .iter()
            .filter_map(|mr| {
                let g = mr.try_borrow().ok()?;
                if g.is_removed || !g.active {
                    return None;
                }
                Some(GridSquareMarker {
                    orig: [g.orig_x, g.orig_y, g.orig_z],
                    z: g.z,
                    size: g.size,
                    rgb: [g.r, g.g, g.b],
                    alpha: g.alpha,
                    active: g.active,
                    texture_name: g.texture_name.clone(),
                    overlay_texture_name: g.overlay_texture_name.clone(),
                })
            })
            .collect()
    })
}

#[cfg(test)]
mod tests;
