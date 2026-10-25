//! pz-world squares as pz-iso-render [`RenderSquare`]s: objects with their
//! sprites, seam neighbours, cutaway and occlusion inputs, plus a
//! [`CutawayWorld`] over the result.

use std::collections::{BTreeSet, HashMap, HashSet};

use pz_iso_render::cell::LevelSquares;
use pz_iso_render::cutaways::BuildingDef;
use pz_iso_render::frame::ChunkInput;
use pz_iso_render::puddles::{puddles_dir, PuddleSquare, PuddlesGeometry};
use pz_iso_render::world::{
    AttachedSprite, BuildingId, SeamNeighbours, SlopeDir, SquareCutawayInfo,
};
use pz_iso_render::{CutawayWorld, ObjectKind, RenderObject, RenderSquare};
use pz_world::isoregion::IsoRegions;
use pz_world::sprite::IsoFlagType;
use pz_world::square::IsoDirections;
use pz_world::{BuildingIdx, GetSquare, IsoCell, IsoGridSquare, IsoMetaGrid, IsoSpriteManager};

use crate::sprites::{SpriteCache, SpriteTextures};

/// The world data the builders read.
pub struct WorldView<'a> {
    pub cell: &'a IsoCell,
    pub meta: &'a IsoMetaGrid,
    pub sprites: &'a IsoSpriteManager,
}

/// `BuildingDef` index to the renderer's id.
pub fn building_id(b: BuildingIdx) -> BuildingId {
    BuildingId(b.0)
}

/// The render squares of the loaded chunks.
#[derive(Default)]
pub struct RenderWorld {
    pub squares: HashMap<(i32, i32, i32), RenderSquare>,
    /// Loaded chunks and their `(minLevel, maxLevel)`.
    pub chunks: HashMap<(i32, i32), (i32, i32)>,
    pub buildings: HashMap<BuildingId, BuildingDef>,
    /// `perPlayerData.onScreenChunks` for cutaways; every loaded chunk
    /// when empty.
    pub on_screen: Vec<(i32, i32)>,
    /// Squares built here whose puddle geometry was never initialised: a
    /// new square's `getPuddles()` starts with `recalc = true`, and
    /// `recalcIfNeeded` inits it on first use. Resolved by
    /// [`crate::PendingRender::apply`] (via [`RenderWorld::init_puddles`])
    /// before the frame draws.
    pub puddles_recalc: BTreeSet<(i32, i32, i32)>,
}

/// The render square of world square `sq`: its objects and the cutaway
/// inputs. Seams are filled by [`RenderWorld::refresh_seams`].
pub fn render_square(
    w: &WorldView<'_>,
    sq: &IsoGridSquare,
    cache: &mut SpriteCache,
    tex: &mut dyn SpriteTextures,
) -> RenderSquare {
    let mut rs = RenderSquare::new(sq.x, sq.y, sq.z);
    rs.cut = cutaway_info(w, sq);
    rs.flies = sq.has_flies;
    for o in &sq.objects {
        let kind = match &o.kind {
            pz_world::ObjectKind::Door(d) => ObjectKind::Door { open: d.open },
            pz_world::ObjectKind::Window(win) => ObjectKind::Window { north: win.north },
            pz_world::ObjectKind::Tree(_) => ObjectKind::Tree,
            _ => ObjectKind::Plain,
        };
        let mut ro = RenderObject::new(kind, o.sprite.map(|s| cache.get(w.sprites, s, tex)));
        ro.id = o.id;
        ro.render_y_offset = o.render_y_offset;
        ro.tint = [o.tint_r, o.tint_g, o.tint_b];
        ro.container_count = u32::from(o.container.is_some()) + o.secondary_containers.len() as u32;
        if let Some(att) = &o.attached_anim_sprite {
            ro.attached = att
                .iter()
                .map(|s| AttachedSprite {
                    sprite: cache.get(w.sprites, s.sprite, tex),
                    alpha: s.alpha,
                })
                .collect();
        }
        rs.objects.push(ro);
    }
    rs
}

/// What cutaways and occlusion read from `sq` (lighting fields keep their
/// defaults until lighting writes them).
pub fn cutaway_info(w: &WorldView<'_>, sq: &IsoGridSquare) -> SquareCutawayInfo {
    let mut c = SquareCutawayInfo::default();
    if let Some(room) = sq.room_def(w.meta) {
        c.room = Some(sq.room_id);
        c.building = room.building.map(building_id);
    }
    c.roof_hide_building = sq.roof_hide_building.map(building_id);
    c.associated_building = sq.associated_building.map(building_id);
    c.has_stairs = sq.has_stairs(w.sprites);
    c.sloped = match sq.sloped_surface_direction {
        Some(IsoDirections::N) => Some(SlopeDir::N),
        Some(IsoDirections::S) => Some(SlopeDir::S),
        Some(IsoDirections::E) => Some(SlopeDir::E),
        Some(IsoDirections::W) => Some(SlopeDir::W),
        _ => None,
    };
    c.vision_blocked_below = sq.get_vision_matrix(0, 0, -1);
    // `getPlayerBuiltFloor()` and `getStairPillar()` need player
    // construction and stair geometry pz-world does not have: false.
    c
}

/// What `IsoPuddlesGeometry` reads from world square `sq`: its
/// `exterior`, `water`, `collideW` and `collideN` flags,
/// `hasSlopedSurface()` and `getPuddlesDir()` (`isInARoom()` and the
/// attached sprites' names).
pub fn puddle_square(w: &WorldView<'_>, regions: &IsoRegions, sq: &IsoGridSquare) -> PuddleSquare {
    let f = &w.sprites.tables.flag;
    let names = sq
        .objects
        .iter()
        .filter_map(|o| o.attached_anim_sprite.as_ref())
        .flatten()
        .filter_map(|a| w.sprites.sprites[a.sprite].name.as_deref());
    PuddleSquare {
        exterior: sq.has(f.exterior),
        water: sq.has(f.water),
        collide_w: sq.has(f.collideW),
        collide_n: sq.has(f.collideN),
        sloped: sq.has_sloped_surface(),
        puddles_dir: puddles_dir(sq.is_in_a_room(w.meta, regions), names),
    }
}

/// `getWallType()`: 1 north wall, 4 west wall (5 for `WallNW`), 8 the east
/// neighbour has a west wall, 2 the south neighbour has a north wall.
pub fn wall_type(w: &WorldView<'_>, sq: &IsoGridSquare) -> i32 {
    let t = &w.sprites.tables;
    let flag =
        |s: &IsoGridSquare, n: &str| t.flag_from_string(n).is_some_and(|f: IsoFlagType| s.has(f));
    let mut ty = 0;
    if flag(sq, "WallN") {
        ty |= 1;
    }
    if flag(sq, "WallW") {
        ty |= 4;
    }
    if flag(sq, "WallNW") {
        ty |= 5;
    }
    if let Some(e) = sq.get_adjacent_square(IsoDirections::E, w.cell) {
        if flag(e, "WallW") || flag(e, "WallNW") {
            ty |= 8;
        }
    }
    if let Some(s) = sq.get_adjacent_square(IsoDirections::S, w.cell) {
        if flag(s, "WallN") || flag(s, "WallNW") {
            ty |= 2;
        }
    }
    ty
}

/// `getWindowFrame(GridSquareEdgeFacingDirection)`: an `IsoWindowFrame` in
/// `objects` on the north (`north`) or west edge.
pub fn has_window_frame(sq: &IsoGridSquare, north: bool) -> bool {
    sq.objects
        .iter()
        .any(|o| matches!(o.kind, pz_world::ObjectKind::WindowFrame { north: n } if n == north))
}

/// The neighbour tests of `renderSeamFix2_Floor` / `renderSeamFix2_Wall`
/// for `sq`. Water uses the `water` flag; the shore test
/// (`IsoWaterGeometry`) is not ported.
pub fn seam_neighbours(w: &WorldView<'_>, sq: &IsoGridSquare) -> SeamNeighbours {
    let t = &w.sprites.tables;
    let flag =
        |s: &IsoGridSquare, n: &str| t.flag_from_string(n).is_some_and(|f: IsoFlagType| s.has(f));
    let s = sq.get_adjacent_square(IsoDirections::S, w.cell);
    let e = sq.get_adjacent_square(IsoDirections::E, w.cell);
    let floor = |s: &IsoGridSquare| s.floor_index(w.sprites).is_some();
    SeamNeighbours {
        s_floor: s.is_some_and(floor),
        e_floor: e.is_some_and(floor),
        s_water: s.is_some_and(|s| s.has(t.flag.water)),
        e_water: e.is_some_and(|e| e.has(t.flag.water)),
        s_wall_w: s.is_some_and(|s| {
            wall_type(w, s) & 4 != 0 || has_window_frame(s, false) || flag(s, "DoorWallW")
        }),
        e_wall_n: e.is_some_and(|e| {
            wall_type(w, e) & 1 != 0 || has_window_frame(e, true) || flag(e, "DoorWallN")
        }),
    }
}

impl RenderWorld {
    pub fn new() -> Self {
        RenderWorld::default()
    }

    /// Copies the meta grid's building bounds (`isEntirelyEmptyOutside`
    /// is not tracked by pz-world: false).
    pub fn set_buildings(&mut self, meta: &IsoMetaGrid) {
        self.buildings = meta
            .building_defs
            .iter()
            .enumerate()
            .map(|(i, b)| {
                (
                    BuildingId(i as u32),
                    BuildingDef {
                        x: b.x,
                        y: b.y,
                        x2: b.x2,
                        y2: b.y2,
                        entirely_empty_outside: false,
                    },
                )
            })
            .collect();
    }

    /// Builds (or rebuilds) the render squares of loaded chunk `(wx, wy)`
    /// and refreshes the seams of its squares and of the neighbours whose
    /// south/east square is in it. Run `IsoCell::recalc_chunk_squares` on
    /// the chunk first. Returns the chunk's levels to invalidate.
    pub fn build_chunk(
        &mut self,
        w: &WorldView<'_>,
        wx: i32,
        wy: i32,
        cache: &mut SpriteCache,
        tex: &mut dyn SpriteTextures,
    ) -> Vec<(i32, i32, i32)> {
        self.remove_chunk(wx, wy);
        let Some(ch) = w.cell.chunks.get(&(wx, wy)) else {
            return Vec::new();
        };
        self.chunks.insert((wx, wy), (ch.min_level, ch.max_level));
        for sq in ch.squares() {
            self.squares
                .insert((sq.x, sq.y, sq.z), render_square(w, sq, cache, tex));
            self.puddles_recalc.insert((sq.x, sq.y, sq.z));
        }
        self.refresh_seams(w, wx, wy);
        (ch.min_level..=ch.max_level).map(|z| (wx, wy, z)).collect()
    }

    /// Rebuilds the render square at `(x, y, z)` from the world after its
    /// objects or properties changed, keeping what lighting wrote
    /// (`light_info`, `vert_lights`, `light_level`, the cutaway visibility
    /// fields) and its water and puddle geometry, and refreshes the seams
    /// of it and of its north and west neighbours. A square gone from the
    /// world is dropped. Does nothing for a chunk not built here.
    pub fn rebuild_square(
        &mut self,
        w: &WorldView<'_>,
        (x, y, z): (i32, i32, i32),
        cache: &mut SpriteCache,
        tex: &mut dyn SpriteTextures,
    ) {
        let (wx, wy) = (x.div_euclid(8), y.div_euclid(8));
        if !self.chunks.contains_key(&(wx, wy)) {
            return;
        }
        if let Some(ch) = w.cell.chunks.get(&(wx, wy)) {
            self.chunks.insert((wx, wy), (ch.min_level, ch.max_level));
        }
        let Some(sq) = w.cell.get_grid_square(x, y, z) else {
            self.squares.remove(&(x, y, z));
            return;
        };
        let mut rs = render_square(w, sq, cache, tex);
        if let Some(old) = self.squares.get(&(x, y, z)) {
            rs.light_info = old.light_info;
            rs.vert_lights = old.vert_lights;
            rs.light_level = old.light_level;
            rs.cut.seen = old.cut.seen;
            rs.cut.can_see = old.cut.can_see;
            rs.cut.could_see = old.cut.could_see;
            rs.cut.dark_multi = old.cut.dark_multi;
            rs.water = old.water;
            rs.puddles = old.puddles;
        } else {
            self.puddles_recalc.insert((x, y, z));
        }
        rs.seam = seam_neighbours(w, sq);
        self.squares.insert((x, y, z), rs);
        for p in [(x - 1, y, z), (x, y - 1, z)] {
            if let (Some(sq), Some(rs)) = (
                w.cell.get_grid_square(p.0, p.1, p.2),
                self.squares.get_mut(&p),
            ) {
                rs.seam = seam_neighbours(w, sq);
            }
        }
    }

    /// `IsoPuddlesGeometry.recalcIfNeeded()` for the render square at
    /// `(x, y, z)`: its puddle geometry re-initialised from the world
    /// square and its eight neighbours on the level (`init`, then
    /// `shouldRender`'s puddles leaking into rooms). `perf_puddles` is
    /// `Core.getPerfPuddles()`. Does nothing without both squares.
    pub fn init_puddles(
        &mut self,
        w: &WorldView<'_>,
        regions: &IsoRegions,
        (x, y, z): (i32, i32, i32),
        perf_puddles: i32,
    ) {
        let (Some(sq), Some(rs)) = (
            w.cell.get_grid_square(x, y, z),
            self.squares.get_mut(&(x, y, z)),
        ) else {
            return;
        };
        let at = |dx: i32, dy: i32| {
            w.cell
                .get_grid_square(x + dx, y + dy, z)
                .map(|n| puddle_square(w, regions, n))
        };
        rs.puddles = Some(PuddlesGeometry::init(
            &puddle_square(w, regions, sq),
            perf_puddles,
            at,
        ));
    }

    /// Drops chunk `(wx, wy)`'s squares.
    pub fn remove_chunk(&mut self, wx: i32, wy: i32) {
        if self.chunks.remove(&(wx, wy)).is_none() {
            return;
        }
        self.squares
            .retain(|&(x, y, _), _| (x.div_euclid(8), y.div_euclid(8)) != (wx, wy));
        self.puddles_recalc
            .retain(|&(x, y, _)| (x.div_euclid(8), y.div_euclid(8)) != (wx, wy));
    }

    /// Recomputes [`SeamNeighbours`] for chunk `(wx, wy)` and for the
    /// squares west and north of it whose east/south neighbour is in it.
    pub fn refresh_seams(&mut self, w: &WorldView<'_>, wx: i32, wy: i32) {
        let mut targets = Vec::new();
        if let Some(ch) = w.cell.chunks.get(&(wx, wy)) {
            targets.extend(ch.squares().map(|s| (s.x, s.y, s.z)));
        }
        let (x0, y0) = (wx * 8, wy * 8);
        for (_, &(min, max)) in self
            .chunks
            .iter()
            .filter(|(&(cx, cy), _)| (cx, cy) == (wx - 1, wy) || (cx, cy) == (wx, wy - 1))
        {
            for z in min..=max {
                for i in 0..8 {
                    targets.push((x0 - 1, y0 + i, z));
                    targets.push((x0 + i, y0 - 1, z));
                }
            }
        }
        for p in targets {
            let (Some(sq), Some(rs)) = (
                w.cell.get_grid_square(p.0, p.1, p.2),
                self.squares.get_mut(&p),
            ) else {
                continue;
            };
            rs.seam = seam_neighbours(w, sq);
        }
    }

    /// The squares of chunk `(wx, wy)` at level `z`, index `x + y * 8`.
    pub fn level_squares(&self, wx: i32, wy: i32, z: i32) -> LevelSquares<'_> {
        let mut out = [None; 64];
        for (i, o) in out.iter_mut().enumerate() {
            *o = self
                .squares
                .get(&(wx * 8 + (i % 8) as i32, wy * 8 + (i / 8) as i32, z));
        }
        out
    }

    /// The renderer's chunk inputs for `chunks` (in the order given, e.g.
    /// `checkNewlyOnScreenChunks`' x-outer, y-inner order); unloaded ones
    /// are skipped.
    pub fn chunk_inputs(&self, chunks: &[(i32, i32)]) -> Vec<ChunkInput<'_>> {
        chunks
            .iter()
            .filter_map(|&(wx, wy)| {
                let &(min, max) = self.chunks.get(&(wx, wy))?;
                let levels = (min..=max)
                    .map(|z| (z, self.level_squares(wx, wy, z)))
                    .collect();
                Some(ChunkInput {
                    wx,
                    wy,
                    min_level: min,
                    max_level: max,
                    levels,
                })
            })
            .collect()
    }
}

impl CutawayWorld for RenderWorld {
    fn square(&self, x: i32, y: i32, z: i32) -> Option<&RenderSquare> {
        self.squares.get(&(x, y, z))
    }

    fn building(&self, id: BuildingId) -> Option<BuildingDef> {
        self.buildings.get(&id).copied()
    }

    fn chunk_levels(&self, wx: i32, wy: i32) -> Option<(i32, i32)> {
        self.chunks.get(&(wx, wy)).copied()
    }

    fn on_screen_chunks(&self) -> Vec<(i32, i32)> {
        if !self.on_screen.is_empty() {
            return self.on_screen.clone();
        }
        let mut c: Vec<_> = self
            .chunks
            .keys()
            .copied()
            .collect::<HashSet<_>>()
            .into_iter()
            .collect();
        c.sort();
        c
    }
}

/// [`GetSquare`] through a [`WorldView`]'s cell.
impl GetSquare for WorldView<'_> {
    fn get_grid_square(&self, x: i32, y: i32, z: i32) -> Option<&IsoGridSquare> {
        self.cell.get_grid_square(x, y, z)
    }
}
