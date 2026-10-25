//! What square edits invalidate on the render side: the
//! [`RenderSquareHooks`] implementation pz-world's square mutations
//! (`pz_world::square_mutation::MutationEnv`) and `WorldRegionToMetaGrid`
//! are given.
//!
//! The original reads squares live when it records a chunk texture or
//! sends a chunk to the lighting library, so a square edit only sets flags.
//! Here the render squares ([`RenderWorld`]) and the lighting inputs
//! ([`WorldLighting::squares`]) are snapshots built from pz-world, so the
//! edit has two parts, each at the original's time:
//!
//! * immediately, inside the mutation ([`RenderInvalidation`]), what the
//!   original sets: the chunk textures' dirty flags
//!   (`FBORenderLevels.invalidateLevel` / `invalidateAll` for every
//!   player's [`ChunkCache`]), the lighting flags
//!   (`checkLightingLater_*`, `LosUtil.cachecleared`,
//!   `Core.dirtyGlobalLightsCount`, `GameTime.lightSourceUpdate`,
//!   `recalcLightTime`, `LightingJNI.buildingsChanged`) and the water
//!   geometry a square loses (`clearWater`);
//! * once per frame, before the lighting frames and the chunk recording
//!   (where the original would read the squares), [`PendingRender::apply`]
//!   rebuilds the snapshots of the squares and chunks that changed, and
//!   re-initialises the puddle geometry of the squares whose properties
//!   were recalculated (the original's `getPuddles().recalc = true`, run
//!   by `recalcIfNeeded` when the square's puddles are next drawn).
//!
//! After every local player's lighting frame, [`check_changed_buildings`]
//! runs the second half of `LightingJNI.update`'s
//! `checkChangedBuildings`.
//!
//! What the original caches and this port recomputes every frame needs no
//! invalidation beyond the flags kept here for a consumer:
//! `FBORenderCutaways` (`squareChanged` drops each player's `checkSquare`,
//! `ChunkLevelsData.invalidateAll` forces `recreateLevel`;
//! [`pz_iso_render::Cutaways::update`] recomputes all of it) and
//! `IsoGridOcclusionData` (its `objectEpochCount`, kept as
//! [`PendingRender::grid_occlusion_epoch`]). `VisibilityPolygon2` is not
//! built in this crate: its dirty set ([`PendingRender::vispoly`]) is left
//! for its owner to drain. Chunk physics (`checkPhysicsLater`) is kept by
//! pz-world's `IsoChunk` itself.

use std::collections::BTreeSet;

use pz_iso_render::cache::ChunkCache;
use pz_iso_render::chunk::dirty;
use pz_world::isoregion::IsoRegions;
use pz_world::sprite::IsoFlagType;
use pz_world::square_mutation::{RenderSquareHooks, SquarePos};
use pz_world::IsoGridSquare;

use crate::hooks::WorldHooks;
use crate::lighting::WorldLighting;
use crate::render_world::{RenderWorld, WorldView};
use crate::sprites::{SpriteCache, SpriteTextures};

/// `WorldRegionToMetaGrid`'s `invalidateRenderChunkLevels(2112)`
/// (`OBJECT_ADD | CUTAWAYS`).
pub const CHUNK_BUILDINGS_CHANGED: u64 = dirty::OBJECT_ADD | dirty::CUTAWAYS;

/// `checkChangedBuildings`' `invalidateAll(18496)` (`OBJECT_ADD | CUTAWAYS
/// | REDO_CUTAWAYS`).
pub const BUILDINGS_CHANGED: u64 = dirty::OBJECT_ADD | dirty::CUTAWAYS | dirty::REDO_CUTAWAYS;

/// What [`PendingRender::apply`] re-initialises puddle geometry with.
#[derive(Clone, Copy)]
pub struct PuddleInputs<'a> {
    /// The world regions (`isInARoom()` counts player-built rooms).
    pub regions: &'a IsoRegions,
    /// `Core.getPerfPuddles()`.
    pub perf_puddles: i32,
}

/// The square edits waiting for the next frame, and the dirty state of
/// the systems this crate does not own.
#[derive(Clone, Debug, Default)]
pub struct PendingRender {
    /// Squares whose render square and lighting input are rebuilt.
    squares: BTreeSet<SquarePos>,
    /// Chunks rebuilt whole (`WorldRegionToMetaGrid` changed their rooms).
    chunks: BTreeSet<(i32, i32)>,
    /// The meta grid's buildings changed: [`RenderWorld::set_buildings`].
    buildings: bool,
    /// `IsoChunk.invalidateVispolyChunkLevel`: `VisibilityPolygon2` chunk
    /// levels to rebuild.
    pub vispoly: BTreeSet<(i32, i32, i32)>,
    /// `getPuddles().recalc = true`: squares whose puddle geometry is
    /// re-initialised by the next [`PendingRender::apply`].
    pub puddles: BTreeSet<SquarePos>,
    /// `FBORenderCutaways.squareChanged` or `ChunkLevelsData.invalidateAll`
    /// since the last [`PendingRender::apply`] (the cutaways are recomputed
    /// every frame; kept for a consumer).
    pub cutaways_changed: bool,
    /// `IsoGridOcclusionData.objectEpochCount`.
    pub grid_occlusion_epoch: i32,
}

impl PendingRender {
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether squares or chunks wait for [`PendingRender::apply`].
    pub fn has_rebuilds(&self) -> bool {
        !self.squares.is_empty() || !self.chunks.is_empty() || self.buildings
    }

    /// Rebuilds the render squares and lighting inputs of what changed
    /// since the last call; run once per frame before the lighting frames
    /// and chunk recording. Chunks with changed squares rescan their light
    /// switches. Then the queued squares' puddle geometry, and that of
    /// squares newly built in `render` ([`RenderWorld::puddles_recalc`]), is
    /// re-initialised from the world (`IsoPuddlesGeometry.recalcIfNeeded`,
    /// which the original runs when the square's puddles are next drawn,
    /// within this frame's chunk recording or puddle pass, reading its
    /// neighbours as they are then). Only the square itself: neighbours
    /// keep their geometry, and no chunk level is invalidated (the edit
    /// that recalculated the properties invalidated what it changed).
    /// Clears [`PendingRender::cutaways_changed`].
    #[allow(clippy::too_many_arguments)]
    pub fn apply(
        &mut self,
        w: &WorldView<'_>,
        render: &mut RenderWorld,
        lighting: &mut WorldLighting,
        hooks: &dyn WorldHooks,
        cache: &mut SpriteCache,
        tex: &mut dyn SpriteTextures,
        puddles: PuddleInputs<'_>,
    ) {
        if std::mem::take(&mut self.buildings) {
            render.set_buildings(w.meta);
        }
        let chunks = std::mem::take(&mut self.chunks);
        for &(wx, wy) in &chunks {
            let mut positions: BTreeSet<SquarePos> = render
                .squares
                .keys()
                .copied()
                .filter(|&(x, y, _)| (x.div_euclid(8), y.div_euclid(8)) == (wx, wy))
                .collect();
            if let Some(ch) = w.cell.chunks.get(&(wx, wy)) {
                positions.extend(ch.squares().map(|s| (s.x, s.y, s.z)));
            }
            for p in positions {
                render.rebuild_square(w, p, cache, tex);
            }
            lighting.sync_chunk_squares(w, wx, wy, hooks);
        }
        let mut touched = chunks.clone();
        for p in std::mem::take(&mut self.squares) {
            let c = (p.0.div_euclid(8), p.1.div_euclid(8));
            if chunks.contains(&c) {
                continue;
            }
            render.rebuild_square(w, p, cache, tex);
            lighting.sync_square(w, p, hooks);
            touched.insert(c);
        }
        for (wx, wy) in touched {
            lighting.sync_chunk_lights(w, wx, wy);
        }
        let mut recalc = std::mem::take(&mut self.puddles);
        recalc.append(&mut render.puddles_recalc);
        for p in recalc {
            render.init_puddles(w, puddles.regions, p, puddles.perf_puddles);
        }
        self.cutaways_changed = false;
    }

    /// `IsoGridOcclusionData.SquareChanged()`.
    fn grid_occlusion_square_changed(&mut self) {
        self.grid_occlusion_epoch = self.grid_occlusion_epoch.wrapping_add(1);
        if self.grid_occlusion_epoch < 0 {
            self.grid_occlusion_epoch = 0;
        }
    }
}

/// The render side of a square mutation: [`RenderSquareHooks`] over the
/// render world, the lighting, every local player's chunk textures and
/// the pending rebuilds.
pub struct RenderInvalidation<'a> {
    pub render: &'a mut RenderWorld,
    pub lighting: &'a mut WorldLighting,
    /// Each local player's chunk textures (`getRenderLevels(player)`).
    pub caches: &'a mut [ChunkCache],
    pub pending: &'a mut PendingRender,
    /// The sprite tables' `water` flag (`IsoFlagType.water`).
    pub water_flag: IsoFlagType,
    /// `IsoPlayer.numPlayers`.
    pub num_players: i32,
    /// `PerformanceSettings.fboRenderChunk` (on in build 42).
    pub fbo_render_chunk: bool,
}

impl<'a> RenderInvalidation<'a> {
    /// The hooks with `fboRenderChunk` on.
    pub fn new(
        render: &'a mut RenderWorld,
        lighting: &'a mut WorldLighting,
        caches: &'a mut [ChunkCache],
        pending: &'a mut PendingRender,
        water_flag: IsoFlagType,
        num_players: i32,
    ) -> Self {
        RenderInvalidation {
            render,
            lighting,
            caches,
            pending,
            water_flag,
            num_players,
            fbo_render_chunk: true,
        }
    }
}

fn chunk_of(x: i32, y: i32) -> (i32, i32) {
    (x.div_euclid(8), y.div_euclid(8))
}

impl RenderSquareHooks for RenderInvalidation<'_> {
    fn fbo_render_chunk(&self) -> bool {
        self.fbo_render_chunk
    }

    /// `IsoChunk.invalidateRenderChunkLevel`: `invalidateLevel(level,
    /// flags)` of every player's render levels.
    fn invalidate_render_chunk_level(&mut self, wx: i32, wy: i32, level: i32, flags: u64) {
        if !self.fbo_render_chunk {
            return;
        }
        for c in self.caches.iter_mut() {
            c.invalidate(wx, wy, level, flags);
        }
    }

    fn invalidate_vispoly_chunk_level(&mut self, wx: i32, wy: i32, level: i32) {
        self.pending.vispoly.insert((wx, wy, level));
    }

    fn cutaways_square_changed(&mut self, sq: Option<SquarePos>) {
        self.pending.cutaways_changed = true;
        if let Some(p) = sq {
            self.pending.squares.insert(p);
        }
    }

    fn los_cache_cleared(&mut self, player: i32) {
        self.lighting.clear_los_cache(player);
    }

    fn set_recalc_light_time(&mut self, t: f32) {
        self.lighting.recalc_light_time = t;
    }

    fn dirty_global_lights(&mut self) {
        self.lighting.dirty_global_lights += 1;
    }

    fn light_source_update(&mut self) {
        self.lighting.light_source_update();
    }

    fn grid_occlusion_square_changed(&mut self) {
        self.pending.grid_occlusion_square_changed();
    }

    /// `RecalcProperties`' tail: puddles recalculated (next
    /// [`PendingRender::apply`]), the square's level
    /// flagged for lighting (and its snapshot rebuilt next frame), and
    /// water geometry dropped from a square that lost `water`.
    /// (`chunk.checkPhysicsLater` and `chunk.collision.clear()` are
    /// pz-world's own.)
    fn properties_recalculated(&mut self, sq: &IsoGridSquare) {
        let p = (sq.x, sq.y, sq.z);
        let (wx, wy) = chunk_of(sq.x, sq.y);
        self.pending.puddles.insert(p);
        self.lighting.mark_level(wx, wy, sq.z);
        if let Some(rs) = self.render.squares.get_mut(&p) {
            if rs.water.is_some_and(|g| g.is_valid()) && !sq.has(self.water_flag) {
                rs.water = None;
            }
        }
        self.pending.squares.insert(p);
    }

    fn clear_water(&mut self, sq: &IsoGridSquare) {
        if let Some(rs) = self.render.squares.get_mut(&(sq.x, sq.y, sq.z)) {
            rs.water = None;
        }
    }

    fn buildings_changed(&mut self) {
        self.lighting.buildings_changed(self.num_players);
        self.pending.buildings = true;
    }

    /// `invalidateRenderChunkLevels(2112)`, `getCutawayData().invalidateAll()`
    /// and `checkLightingLater_AllPlayers_AllLevels()`; the chunk's squares
    /// (rooms and buildings) are rebuilt next frame.
    fn chunk_buildings_changed(&mut self, wx: i32, wy: i32) {
        if self.fbo_render_chunk {
            for c in self.caches.iter_mut() {
                c.invalidate_chunk(wx, wy, CHUNK_BUILDINGS_CHANGED);
            }
        }
        self.pending.cutaways_changed = true;
        self.lighting.mark_chunk(wx, wy);
        self.pending.chunks.insert((wx, wy));
        self.pending.buildings = true;
    }
}

/// `LightingJNI.checkChangedBuildings()`, after every local player's
/// lighting frame: for each player whose buildings-changed counter the
/// lighting updates reached, every loaded chunk of its chunk map has its
/// render levels invalidated (`invalidateAll(18496)`, that player's
/// [`ChunkCache`] in `caches`), its cutaways invalidated and its lighting
/// flagged for that player; then, if any player did,
/// `IsoGridOcclusionData.SquareChanged()` and
/// `FBORenderCutaways.squareChanged(null)`.
pub fn check_changed_buildings(
    lighting: &mut WorldLighting,
    caches: &mut [ChunkCache],
    pending: &mut PendingRender,
    num_players: i32,
) {
    let mut updated = false;
    for (p, chunks) in lighting.check_changed_buildings(num_players) {
        updated = true;
        if let Some(c) = caches.get_mut(p as usize) {
            for &(wx, wy) in &chunks {
                c.invalidate_chunk(wx, wy, BUILDINGS_CHANGED);
            }
        }
    }
    if updated {
        pending.grid_occlusion_square_changed();
        pending.cutaways_changed = true;
    }
}
