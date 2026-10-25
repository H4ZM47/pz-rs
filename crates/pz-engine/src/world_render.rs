//! `IsoWorld.render` and `IsoCell.render` (`FBORenderCell`) through
//! pz-world-render and pz-iso-render: the render squares of the chunks
//! the streamer handed off, the lighting frame of each player, the chunk
//! textures culled to the screen (`plan_chunks`) and the frame itself
//! (`world_frame_full`: the chunk composite, the translucent passes and
//! the screen shader).

use crate::iso_world::{IsoWorld, RenderChunkEvents};
use crate::world_models::{CharacterView, WorldModels};
use pz_character_model::lights::LightContext;
use pz_fs::GameFs;
use pz_iso_render::cache::ChunkCache;
use pz_iso_render::chunk::dirty;
use pz_iso_render::cutaways::{CutawayPlayer, Cutaways};
use pz_iso_render::frame::{plan_chunks, world_frame_full, FrameSize};
use pz_iso_render::objects::FrameObjects;
use pz_iso_render::{Camera, ChunkRenderer};
use pz_lighting::jni::{IsoDirection, VisionConeInput};
use pz_lua_api::bind_character_sim::LuaGameCharacter;
use pz_render::tile_depth::{
    TileDepthTextureAssignmentManager, TileDepthTextureManager, TileGeometryManager,
};
use pz_render::{DrawCmd, ScreenParams, Texture, TextureManager};
use pz_world::isoregion::IsoRegions;
use pz_world::square::IsoDirections;
use pz_world_render::{
    check_changed_buildings, default_player, ClimateSettings, GameTextures, NoWorldHooks,
    PendingRender, PlayerFrameInput, PuddleInputs, RenderInvalidation, RenderWorld, SpriteCache,
    WorldLighting, WorldView,
};
use std::time::Instant;

/// `IsoPlayer.MAX`.
const MAX_PLAYERS: usize = 4;

/// What one player's frame of the world is drawn from.
pub struct RenderView {
    /// `IsoCamera.frameState.playerIndex`.
    pub player: usize,
    /// The camera character's position and `getDir()`.
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub dir: IsoDirections,
    /// The player's chunk map: `getWorldXMin()`, `getWorldYMin()` and
    /// `chunkGridWidth`.
    pub chunk_map: (i32, i32, i32),
    /// The offscreen view (`IsoCamera.getOffscreenWidth/Height(player)`).
    pub width: u32,
    pub height: u32,
    /// `Core.getZoom(player)`.
    pub zoom: f32,
    /// `RenderSettings` over `ClimateManager`.
    pub climate: ClimateSettings,
    /// The player's `WeatherShader` inputs.
    pub screen: ScreenParams,
    pub tick: i64,
    pub multiplier: f32,
}

/// The render side of the world: the tile depth tables, the render
/// squares and their lighting, each player's chunk textures and cutaways.
pub struct WorldRender {
    depth: TileDepthTextureManager,
    assignments: TileDepthTextureAssignmentManager,
    geometry: TileGeometryManager,
    /// `SEAMS_01.png`'s masks and the cutaway stencils.
    seams: Vec<Texture>,
    cutaway_stencils: Vec<Texture>,
    drop_shadow: Option<Texture>,
    sprites: SpriteCache,
    pub world: RenderWorld,
    pub lighting: WorldLighting,
    /// `FBORenderChunkManager`'s chunk textures, per player.
    caches: [ChunkCache; MAX_PLAYERS],
    cutaways: [Cutaways; MAX_PLAYERS],
    start: Instant,
    /// The lighting session failed; frames draw unlit.
    lighting_error: bool,
    /// What the square mutations changed since the last frame.
    pub pending: PendingRender,
}

impl WorldRender {
    /// The textures and tables the world renderer reads. The tile packs
    /// are `GameWindow.enter`'s (loaded here, on the first world frame, so
    /// that the menu does not pay for them); then
    /// `TileDepthTextureManager.init`, the tile depth assignments and the
    /// tile geometry, the seam and cutaway masks.
    pub fn new(game_dir: &std::path::Path, fs: &mut GameFs, tm: &mut TextureManager) -> Self {
        // `GameWindow.enter` at `Core.tileScale` 2.
        let flags = if tm.use_compression { 4 } else { 0 } | 0x40;
        for (pack, f) in [
            ("Tiles2x", flags),
            ("Overlays2x", flags),
            ("JumboTrees2x", flags),
            ("JumboTreesBigs2x", flags),
            ("Tiles2x.floor", flags & !4),
            ("B42ChunkCaching2x", flags),
            ("B42ChunkCaching2x.floor", flags & !4),
            ("Clock2x", flags),
        ] {
            tm.load_texture_pack(fs, pack, f, None);
        }
        let mods = fs.mod_ids().to_vec();
        tm.set_texture_pack_lookup(&mods);
        let depth = TileDepthTextureManager::init(fs, tm);
        let assignments = TileDepthTextureAssignmentManager::init(fs);
        let geometry = TileGeometryManager::from_media_dirs([game_dir.join("media").as_path()]);
        let seams = tm
            .shared_texture_flags(fs, "media/depthmaps/SEAMS_01.png", 0)
            .map(|t| pz_iso_render::seam::seam_masks(&t))
            .unwrap_or_default();
        let cutaway_stencils = [
            "media/wallcutaways.png",
            "media/windowframe_cutaways.png",
            "media/windowframe_cutaways_2.png",
            "media/windowframe_cutaways_3.png",
        ]
        .iter()
        .filter_map(|p| tm.shared_texture_flags(fs, p, 3))
        .collect();
        WorldRender {
            depth,
            assignments,
            geometry,
            seams,
            cutaway_stencils,
            drop_shadow: tm.shared_texture_flags(fs, "media/textures/NewShadow.png", 0),
            sprites: SpriteCache::new(),
            world: RenderWorld::new(),
            lighting: WorldLighting::new(),
            caches: Default::default(),
            cutaways: std::array::from_fn(|_| Cutaways::new()),
            start: Instant::now(),
            lighting_error: false,
            pending: PendingRender::new(),
        }
    }

    /// The `RenderSquareHooks` the square mutations (fire, Lua, the
    /// states) are given: chunk texture, lighting, water and building
    /// invalidation, rebuilt by the next frame.
    pub fn invalidation(
        &mut self,
        water_flag: pz_world::sprite::IsoFlagType,
        num_players: i32,
    ) -> RenderInvalidation<'_> {
        RenderInvalidation::new(
            &mut self.world,
            &mut self.lighting,
            &mut self.caches,
            &mut self.pending,
            water_flag,
            num_players,
        )
    }

    /// The pending rebuilds, before the lighting frames and chunk recording
    /// (`PendingRender::apply`).
    fn apply_pending(
        &mut self,
        view: &WorldView<'_>,
        regions: &IsoRegions,
        fs: &GameFs,
        tm: &mut TextureManager,
    ) {
        let mut tex = GameTextures {
            fs,
            textures: tm,
            depth: &self.depth,
            assignments: &self.assignments,
            geometry: &self.geometry,
        };
        let perf_puddles = pz_lua_api::bind_core::shared_core()
            .borrow()
            .get_perf_puddles();
        self.pending.apply(
            view,
            &mut self.world,
            &mut self.lighting,
            &NoWorldHooks,
            &mut self.sprites,
            &mut tex,
            PuddleInputs {
                regions,
                perf_puddles,
            },
        );
    }

    /// The chunks that entered the cell get their render squares and
    /// lighting chunk (`FBORenderChunk` creation, `LightingJNI.chunkLoaded`),
    /// the ones that left lose them.
    fn sync_chunks(
        &mut self,
        view: &WorldView<'_>,
        events: &mut RenderChunkEvents,
        fs: &GameFs,
        tm: &mut TextureManager,
    ) {
        for (wx, wy) in events.removed.drain(..) {
            self.world.remove_chunk(wx, wy);
            self.lighting.remove_chunk(wx, wy);
        }
        let mut tex = GameTextures {
            fs,
            textures: tm,
            depth: &self.depth,
            assignments: &self.assignments,
            geometry: &self.geometry,
        };
        // TODO(port): `NoWorldHooks`: thumpables, barricades, power and
        // the players' torches (`WorldHooks`).
        let hooks = NoWorldHooks;
        for (wx, wy) in events.added.drain(..) {
            for level in self
                .world
                .build_chunk(view, wx, wy, &mut self.sprites, &mut tex)
            {
                for c in self.caches.iter_mut() {
                    c.invalidate(level.0, level.1, level.2, dirty::CREATE);
                }
            }
            // The neighbours' seams changed with it.
            for (nx, ny) in [(wx - 1, wy), (wx, wy - 1)] {
                for c in self.caches.iter_mut() {
                    c.invalidate_chunk(nx, ny, dirty::REDRAW);
                }
            }
            self.lighting.sync_chunk(view, wx, wy, &hooks);
        }
    }

    /// One player's world frame (`FBORenderCell.renderInternal` and what
    /// `IsoWorld.render` draws around it), appended to `out`.
    fn render_player(
        &mut self,
        view: &WorldView<'_>,
        v: &mut RenderView,
        models: Option<&mut WorldModels>,
        zombies: &[LuaGameCharacter],
        out: &mut Vec<DrawCmd>,
    ) {
        let p = v.player;
        let cam = Camera::player(v.x, v.y, v.z, v.width as i32, v.height as i32, v.zoom);
        let size = FrameSize {
            width: v.width,
            height: v.height,
        };
        // `LightingJNI.update` for the player, then the decoded results
        // into the render squares.
        if !self.lighting_error {
            let input = PlayerFrameInput {
                index: p as i32,
                // TODO(port): the player's fatigue, detection range and
                // the worn items' vision multiplier.
                player: default_player(v.x, v.y, v.z, IsoDirection::ALL[v.dir as usize]),
                cone: VisionConeInput {
                    worn_items_vision_multiplier: 1.0,
                    ..Default::default()
                },
                chunk_x_min: v.chunk_map.0,
                chunk_y_min: v.chunk_map.1,
                chunk_grid_width: v.chunk_map.2,
            };
            let nanos = self.start.elapsed().as_nanos() as u64;
            let (w, h) = (v.width, v.height);
            let on_screen = |wx: i32, wy: i32, z: i32| {
                pz_iso_render::cache::level_on_screen(&cam, w, h, wx, wy, z)
            };
            match self.lighting.frame(
                view,
                &input,
                &mut v.climate,
                &NoWorldHooks,
                nanos,
                &on_screen,
            ) {
                Ok(r) => {
                    for (wx, wy, z) in r.dirty_levels {
                        self.caches[p].invalidate(wx, wy, z, dirty::LIGHTING);
                    }
                    self.lighting.apply(p as i32, &mut self.world);
                }
                Err(e) => {
                    eprintln!("ERROR: LightingJNI: {e:?}");
                    self.lighting_error = true;
                }
            }
        }
        // `checkNewlyOnScreenChunks`' order: x outer, y inner over the
        // player's chunk map.
        let (x0, y0, n) = v.chunk_map;
        let order: Vec<(i32, i32)> = (x0..x0 + n)
            .flat_map(|x| (y0..y0 + n).map(move |y| (x, y)))
            .collect();
        let inputs = self.world.chunk_inputs(&order);
        let mut cmds = Vec::new();
        let chunks = plan_chunks(&mut self.caches[p], &cam, size, &inputs, &mut cmds);
        let maps = pz_iso_render::seam::WorldDepthMaps {
            tiles: &self.depth,
            seams: self.seams.clone(),
            cutaways: self.cutaway_stencils.clone(),
        };
        let (dx, dy) = v.dir.delta();
        let len = ((dx * dx + dy * dy) as f32).sqrt().max(1.0);
        // TODO(port): aiming, the mouse point of interest and peeking.
        let player = CutawayPlayer {
            x: v.x,
            y: v.y,
            z: v.z,
            forward: [dx as f32 / len, dy as f32 / len],
            aiming: false,
            mouse_poi: None,
            peeking: None,
            occlusion_bounds: Some(pz_iso_render::occlusion::grid_bounds(
                &cam,
                v.width as i32,
                v.height as i32,
            )),
        };
        let state = self.cutaways[p].update(&self.world, &player);
        let cv = pz_iso_render::alpha::CutawayView {
            state: &state,
            world: &self.world,
        };
        let r = ChunkRenderer::new(cam, &maps, (v.width, v.height)).with_cutaways(Some(cv));
        // TODO(port): corpses, world items, the water and sky surfaces
        // (`SkyBox`, `SurfaceChecks`) and the
        // `FrameExtras` (flies, highlights, markers, vision cone, fog).
        let mut objects = FrameObjects::default();
        if let Some(models) = models {
            let camera = pz_iso_render::models::ModelCamera::new(
                &cam,
                v.width as f32 * v.zoom,
                v.height as f32 * v.zoom,
            );
            let display_player = pz_lua_api::bind_core::shared_core()
                .borrow()
                .is_display_player_model();
            let mut draw = |chr: &LuaGameCharacter| {
                let (x, y, z) = (
                    chr.vars.x.floor() as i32,
                    chr.vars.y.floor() as i32,
                    chr.vars.z.floor() as i32,
                );
                let sq_owner = view.cell.get_grid_square(x, y, z)?;
                let sq = sq_owner.borrow();
                let light = self.lighting.square_light(p as i32, x, y, z);
                let ambient = light.map(|l| {
                    pz_iso_render::models::interpolate_light(
                        &l.vert_light,
                        chr.vars.x - x as f32,
                        chr.vars.y - y as f32,
                    )
                });
                let color = ambient
                    .map(|c| [c.r, c.g, c.b])
                    .unwrap_or([v.climate.climate.ambient; 3]);
                let lighting = LightContext {
                    x: chr.vars.x,
                    y: chr.vars.y,
                    z: chr.vars.z,
                    result_lights: self.lighting.result_lights(p as i32, x, y, z).to_vec(),
                    target_ambient: color,
                    outside: sq.room_id == -1,
                    in_room: sq.room_id != -1,
                    // TODO(port): room-light ids and the player's equipped torch.
                    room_light_ids: Vec::new(),
                    player_index: chr.is_player().then_some(chr.player.player_index),
                    torch_strength: 0.0,
                    forward: [chr.vars.forward_x, chr.vars.forward_y],
                    multiplier: v.multiplier,
                };
                let hidden = (chr.is_player() && !display_player)
                    || (!chr.is_player() && light.is_some_and(|l| l.vis & 2 == 0));
                let cv = CharacterView {
                    camera: &camera,
                    camera_index: p,
                    lighting: &lighting,
                    alpha: 1.0,
                    hidden,
                    height_above_floor: chr.vars.z - z as f32,
                    tick: v.tick,
                    thirty_fps_multiplier: v.multiplier / 1.6,
                };
                models.render_character(chr, &cv)
            };
            for n in 0..MAX_PLAYERS {
                if let Some(player) = pz_lua_api::bind_players::player(n) {
                    if let Some(mut object) = draw(&player.borrow()) {
                        object.local_player = n == v.player;
                        objects.players.push(object);
                    }
                }
            }
            for zombie in zombies {
                if let Some(object) = draw(zombie) {
                    objects.moving.push(object);
                }
            }
        }
        // `renderWeatherFX` draws after the cell, in the world projection.
        // TODO(port): `render_weather_fx` from `ClimateRenderValues` with
        // the weather mask.
        let mut late = |_: &mut Vec<DrawCmd>| {};
        let cmds = world_frame_full(
            &r,
            size,
            &chunks,
            &objects,
            self.drop_shadow.clone(),
            v.screen,
            cmds,
            &mut late,
        );
        out.extend(cmds);
    }
}

impl IsoWorld {
    /// `IsoWorld.render()` for `view.player` (the camera character is
    /// `IsoPlayer.players[view.player]`), appended to `out`.
    pub fn render(
        &mut self,
        view: &mut RenderView,
        fs: &mut GameFs,
        tm: &std::rc::Rc<std::cell::RefCell<TextureManager>>,
        out: &mut Vec<DrawCmd>,
    ) {
        // TODO(port): `drawWorld`; `sceneCullZombies`/`sceneCullAnimals`,
        // `WeatherFxMask.initMask`, `DeadBodyAtlas`/`WorldItemAtlas.render`.
        // Each local player's chunk map for the lighting's cell lights
        // (`chunkMap[n].ignore` or no player: none).
        let maps: [Option<(i32, i32, i32)>; MAX_PLAYERS] = std::array::from_fn(|n| {
            let s = self.streamer.as_ref().filter(|s| n < s.num_players())?;
            let m = s.chunk_map(n);
            (!m.ignore && pz_lua_api::bind_players::player(n).is_some())
                .then(|| (m.world_x - s.width / 2, m.world_y - s.width / 2, s.width))
        });
        let num_players = pz_lua_api::bind_players::num_players();
        let IsoWorld {
            render,
            render_chunks,
            sprites,
            game_dir,
            regions,
            models,
            zombies,
            ..
        } = self;
        let no_regions;
        let regions = match regions.as_ref() {
            Some(r) => r,
            None => {
                no_regions = IsoRegions::new(0);
                &no_regions
            }
        };
        let Some(sprites) = sprites.as_ref() else {
            return;
        };
        let r = render.get_or_insert_with(|| {
            let mut r = WorldRender::new(game_dir, fs, &mut tm.borrow_mut());
            pz_lua_api::bind_world::with_world(|w| {
                if let Some(meta) = w.meta_grid.as_ref() {
                    r.world.set_buildings(meta);
                }
            });
            r
        });
        pz_lua_api::bind_world::with_world(|w| {
            let (Some(meta), Some(cell)) = (w.meta_grid.as_ref(), w.cell.as_ref()) else {
                return;
            };
            let sprites = sprites.borrow();
            let wv = WorldView {
                cell,
                meta,
                sprites: &sprites,
            };
            {
                let mut textures = tm.borrow_mut();
                r.sync_chunks(&wv, render_chunks, fs, &mut textures);
                r.apply_pending(&wv, regions, fs, &mut textures);
            }
            // Character resources may lazily load hair or equipped-item
            // file textures. Release the shared texture owner first.
            for (n, m) in maps.iter().enumerate() {
                r.lighting.set_chunk_map(n as i32, *m);
            }
            // `currentCell.render()`.
            r.render_player(&wv, view, models.as_mut(), zombies, out);
            check_changed_buildings(&mut r.lighting, &mut r.caches, &mut r.pending, num_players);
        });
        // TODO(port): `Gizmos.render`, `DrawIsoCursorHelper`, the
        // pathfinding/sound/vocals debug renders, `FBORenderAreaHighlights`,
        // `LineDrawer.render`, `SkyBox.render`.
    }
}
