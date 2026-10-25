//! Square edits invalidating the render side: each `RenderSquareHooks`
//! hook of `RenderInvalidation`, and a wall added through pz-world's
//! square mutations re-recording its chunk level and changing the
//! lighting. Hand-built scene (authored test data, not game data).

use pz_iso_render::cache::ChunkCache;
use pz_iso_render::chunk::dirty;
use pz_iso_render::water::WaterGeometry;
use pz_lighting::jni::{IsoDirection, VisionConeInput};
use pz_world::isoregion::IsoRegions;
use pz_world::sprite::PropertyValueMap;
use pz_world::square_mutation::{DoorTables, MutationEnv, NoMutationHooks, RenderSquareHooks};
use pz_world::{
    GameRand, IsoCell, IsoChunk, IsoGridSquare, IsoMetaGrid, IsoObject, IsoSpriteManager,
    ObjectKind, SpriteTables,
};
use pz_world_render::invalidate::{BUILDINGS_CHANGED, CHUNK_BUILDINGS_CHANGED};
use pz_world_render::{
    check_changed_buildings, default_player, NoTextures, NoWorldHooks, NoonSettings, PendingRender,
    PlayerFrameInput, PuddleInputs, RenderInvalidation, RenderWorld, SpriteCache, WorldLighting,
    WorldView,
};

fn tables() -> SpriteTables {
    let flags = "collideW,collideN,exterior,NoWallLighting,doorW,doorN,transparentW,\
transparentN,cutW,cutN,solid,invisible,HoppableN,HoppableW,canPathW,canPathN,blocksight,\
ForceAmbient,WallSE,FloorHeightOneThird,FloorHeightTwoThirds,CantClimb,open,windowW,windowN,\
WindowW,WindowN,diamondFloor,transparentFloor,solidfloor,hidewalls,container,WallOverlay,\
attachedSE,attachedW,attachedN,FloorOverlay,vegitation,NeverCutaway,HasLightOnSprite,\
EntityScript,water,solidtrans,blueprint,BlockRain,trans,noStart,forceRender,burntOut,WallN,WallW,WallNW";
    let types = "normal,wall,doorW,doorN,tree,windowFN,windowFW,isMoveAbleObject,lightswitch,\
curtainN,curtainS,curtainE,curtainW,jukebox,radio,doorFrN,doorFrW,stairsTN,stairsMN,stairsBN,\
stairsTW,stairsMW,stairsBW";
    SpriteTables::from_text(&format!("{flags}\n{types}\nN,W\n")).unwrap()
}

const SPRITES: &[(&str, &str, &[&str])] = &[
    ("floor", "normal", &["solidfloor"]),
    ("water", "normal", &["solidfloor", "water"]),
    ("wall_w", "wall", &["collideW", "cutW", "WallW"]),
];

fn sprites() -> IsoSpriteManager {
    let mut map = PropertyValueMap::default();
    for k in [
        "WindowN",
        "WindowW",
        "DoorWallN",
        "DoorWallW",
        "WallSE",
        "tree",
        "lightR",
        "lightG",
        "lightB",
        "ItemHeight",
        "Surface",
    ] {
        map.entry(k);
    }
    let mut m = IsoSpriteManager::new(tables());
    m.generate_alias_map(map, &[]).unwrap();
    for (i, (name, ty, flags)) in SPRITES.iter().enumerate() {
        let s = m.add_sprite(name, 10 + i as i32);
        let t = m.tables.object_type_from_string(ty);
        let (tables, alias) = (m.tables.clone(), m.alias.clone());
        let sp = &mut m.sprites[s];
        sp.name = Some(name.to_string());
        sp.tile_type = t;
        for f in *flags {
            sp.properties.set(&tables, &alias, f, "", true);
        }
        sp.cut_n = sp.properties.has(tables.flag.cutN);
        sp.cut_w = sp.properties.has(tables.flag.cutW);
    }
    m
}

struct Zero;
impl GameRand for Zero {
    fn next(&mut self, _max: i32) -> i32 {
        0
    }
}

/// Chunks (1..=3, 1..=3) with floors on level 0 and water at (9, 9),
/// their render squares and lighting, and two players' chunk textures.
struct Scene {
    cell: IsoCell,
    m: IsoSpriteManager,
    meta: IsoMetaGrid,
    render: RenderWorld,
    lighting: WorldLighting,
    caches: Vec<ChunkCache>,
    pending: PendingRender,
    sprite_cache: SpriteCache,
    frame: u64,
    regions: IsoRegions,
    perf_puddles: i32,
}

impl Scene {
    fn new() -> Scene {
        let m = sprites();
        let meta = IsoMetaGrid::empty();
        let mut cell = IsoCell::default();
        for wx in 1..=3 {
            for wy in 1..=3 {
                cell.chunks.insert((wx, wy), IsoChunk::new(wx, wy));
            }
        }
        for x in 8..32 {
            for y in 8..32 {
                let mut sq = IsoGridSquare::new(x, y, 0);
                let id = sq.new_object_id();
                let name = if (x, y) == (9, 9) { "water" } else { "floor" };
                sq.objects.push(pz_world::ObjectSlot::new(IsoObject::new(
                    id,
                    ObjectKind::Object,
                    Some(m.named_map[name]),
                )));
                cell.chunks.get_mut(&(x / 8, y / 8)).unwrap().set_square(
                    x % 8,
                    y % 8,
                    0,
                    Some((sq).into()),
                );
            }
        }
        for wx in 1..=3 {
            for wy in 1..=3 {
                cell.recalc_chunk_squares(wx, wy, &meta, &m);
            }
        }
        let mut s = Scene {
            cell,
            m,
            meta,
            render: RenderWorld::new(),
            lighting: WorldLighting::new(),
            caches: vec![ChunkCache::default(), ChunkCache::default()],
            pending: PendingRender::new(),
            sprite_cache: SpriteCache::new(),
            frame: 0,
            regions: IsoRegions::new(3),
            perf_puddles: 0,
        };
        let view = WorldView {
            cell: &s.cell,
            meta: &s.meta,
            sprites: &s.m,
        };
        for wx in 1..=3 {
            for wy in 1..=3 {
                s.render
                    .build_chunk(&view, wx, wy, &mut s.sprite_cache, &mut NoTextures);
                s.lighting.sync_chunk(&view, wx, wy, &NoWorldHooks);
                // Recorded once: clean until invalidated.
                for c in &mut s.caches {
                    c.begin_level(wx, wy, 0, 0, 0);
                }
            }
        }
        s
    }

    fn hooks(&mut self) -> RenderInvalidation<'_> {
        let water = self.m.tables.flag.water;
        RenderInvalidation::new(
            &mut self.render,
            &mut self.lighting,
            &mut self.caches,
            &mut self.pending,
            water,
            2,
        )
    }

    fn sq(&self, x: i32, y: i32) -> pz_world::SquareRef {
        self.cell.get_grid_square(x, y, 0).unwrap()
    }

    /// The frame's world side: pending rebuilds, then player 0's lighting
    /// at (14.5, 15.5) facing east at noon, the results into the render
    /// squares and `checkChangedBuildings`.
    fn frame(&mut self) -> Vec<(i32, i32, i32)> {
        let view = WorldView {
            cell: &self.cell,
            meta: &self.meta,
            sprites: &self.m,
        };
        self.pending.apply(
            &view,
            &mut self.render,
            &mut self.lighting,
            &NoWorldHooks,
            &mut self.sprite_cache,
            &mut NoTextures,
            PuddleInputs {
                regions: &self.regions,
                perf_puddles: self.perf_puddles,
            },
        );
        let input = PlayerFrameInput {
            index: 0,
            player: default_player(14.5, 15.5, 0.0, IsoDirection::E),
            cone: VisionConeInput {
                worn_items_vision_multiplier: 1.0,
                ..Default::default()
            },
            chunk_x_min: 1,
            chunk_y_min: 1,
            chunk_grid_width: 3,
        };
        let r = self
            .lighting
            .frame(
                &view,
                &input,
                &mut NoonSettings::default(),
                &NoWorldHooks,
                self.frame * 33_000_000,
                &|_, _, _| true,
            )
            .unwrap();
        self.frame += 1;
        self.lighting.apply(0, &mut self.render);
        check_changed_buildings(&mut self.lighting, &mut self.caches, &mut self.pending, 1);
        for &(wx, wy, z) in &r.dirty_levels {
            for c in &mut self.caches {
                c.invalidate(wx, wy, z, dirty::LIGHTING);
            }
        }
        r.dirty_levels
    }

    /// Runs frames until lighting settles, then records every level so
    /// the chunk textures are clean.
    fn settle(&mut self) {
        for _ in 0..12 {
            self.frame();
        }
        for wx in 1..=3 {
            for wy in 1..=3 {
                for c in &mut self.caches {
                    c.begin_level(wx, wy, 0, 0, 0);
                }
            }
        }
    }
}

#[test]
fn chunk_levels_are_invalidated_for_every_player() {
    let mut s = Scene::new();
    s.hooks()
        .invalidate_render_chunk_level(2, 1, 0, dirty::OBJECT_REMOVE);
    for c in &s.caches {
        assert_eq!(c.dirty_flags(2, 1, 0), dirty::OBJECT_REMOVE);
        assert_eq!(c.dirty_flags(1, 1, 0), 0);
    }
    assert!(s.caches[1].begin_level(2, 1, 0, 0, 0).1);
    // Without fboRenderChunk the original does nothing.
    let mut h = s.hooks();
    h.fbo_render_chunk = false;
    assert!(!h.fbo_render_chunk());
    h.invalidate_render_chunk_level(1, 1, 0, dirty::OBJECT_ADD);
    assert_eq!(s.caches[0].dirty_flags(1, 1, 0), 0);
}

#[test]
fn lighting_flags_are_set_at_once() {
    let mut s = Scene::new();
    s.settle();
    assert!(!s.lighting.los_cache_is_cleared(0));
    assert_eq!(s.lighting.light_source_update_value(), 0.0);
    let before = s.lighting.dirty_global_lights;
    let mut h = s.hooks();
    h.los_cache_cleared(0);
    h.set_recalc_light_time(-1.0);
    h.dirty_global_lights();
    h.light_source_update();
    assert!(s.lighting.los_cache_is_cleared(0));
    assert_eq!(s.lighting.recalc_light_time, -1.0);
    assert_eq!(s.lighting.dirty_global_lights, before + 1);
    assert_eq!(s.lighting.light_source_update_value(), 100.0);
    // The next frame sends and clears them.
    s.frame();
    assert_eq!(s.lighting.light_source_update_value(), 0.0);
    assert!(!s.lighting.los_cache_is_cleared(0));
}

#[test]
fn recalculated_properties_flag_lighting_puddles_and_water() {
    let mut s = Scene::new();
    s.settle();
    assert_eq!(s.lighting.light_check(2, 1, 0, 0), Some((false, false)));
    let geom = WaterGeometry {
        has_water: true,
        ..Default::default()
    };
    s.render.squares.get_mut(&(17, 9, 0)).unwrap().water = Some(geom);
    s.render.squares.get_mut(&(9, 9, 0)).unwrap().water = Some(geom);
    let (dry, wet) = (s.sq(17, 9), s.sq(9, 9));
    let mut h = s.hooks();
    h.properties_recalculated(dry);
    h.properties_recalculated(wet);
    // checkLightingLater_AllPlayers_OneLevel.
    assert_eq!(s.lighting.light_check(2, 1, 0, 0), Some((true, true)));
    assert_eq!(s.lighting.light_check(2, 1, 0, 1), Some((true, true)));
    assert_eq!(s.lighting.light_check(2, 2, 0, 0), Some((false, false)));
    // Valid water geometry on a square without `water` goes.
    assert_eq!(s.render.squares[&(17, 9, 0)].water, None);
    assert_eq!(s.render.squares[&(9, 9, 0)].water, Some(geom));
    assert!(s.pending.puddles.contains(&(17, 9, 0)));
    assert!(s.pending.has_rebuilds());
    // The next frame sends the level.
    s.frame();
    assert_eq!(s.lighting.light_check(2, 1, 0, 0), Some((false, false)));
    assert!(!s.pending.has_rebuilds());
}

#[test]
fn clear_water_drops_the_geometry() {
    let mut s = Scene::new();
    let geom = WaterGeometry {
        has_water: true,
        ..Default::default()
    };
    s.render.squares.get_mut(&(9, 9, 0)).unwrap().water = Some(geom);
    let wet = s.sq(9, 9);
    s.hooks().clear_water(wet);
    assert_eq!(s.render.squares[&(9, 9, 0)].water, None);
}

#[test]
fn vispoly_cutaways_and_occlusion_are_recorded() {
    let mut s = Scene::new();
    // The first frames run the initial `checkChangedBuildings`.
    s.settle();
    let epoch = s.pending.grid_occlusion_epoch;
    let mut h = s.hooks();
    h.invalidate_vispoly_chunk_level(1, 2, 0);
    h.cutaways_square_changed(Some((10, 17, 0)));
    h.grid_occlusion_square_changed();
    h.grid_occlusion_square_changed();
    assert!(s.pending.vispoly.contains(&(1, 2, 0)));
    assert!(s.pending.cutaways_changed);
    assert_eq!(s.pending.grid_occlusion_epoch, epoch + 2);
    assert!(s.pending.has_rebuilds());
    s.frame();
    assert!(!s.pending.cutaways_changed);
    // The other sets are their owners' to drain.
    assert!(s.pending.vispoly.contains(&(1, 2, 0)));
}

#[test]
fn a_chunk_whose_buildings_changed_is_redrawn_and_relit() {
    let mut s = Scene::new();
    s.settle();
    s.hooks().chunk_buildings_changed(3, 2);
    for c in &s.caches {
        assert_eq!(c.dirty_flags(3, 2, 0), CHUNK_BUILDINGS_CHANGED);
        assert_eq!(c.dirty_flags(2, 2, 0), 0);
    }
    assert_eq!(s.lighting.light_check(3, 2, 0, 0), Some((true, true)));
    assert_eq!(s.lighting.light_check(3, 2, 0, 1), Some((true, true)));
    assert!(s.pending.cutaways_changed && s.pending.has_rebuilds());
    s.frame();
    assert!(!s.pending.has_rebuilds());
    assert_eq!(s.render.squares.len(), 24 * 24);
}

#[test]
fn buildings_changed_redraws_the_players_chunks_two_updates_later() {
    let mut s = Scene::new();
    s.settle();
    let before = s.lighting.dirty_global_lights;
    let epoch = s.pending.grid_occlusion_epoch;
    s.hooks().buildings_changed();
    assert!(s.lighting.los_cache_is_cleared(0) && s.lighting.los_cache_is_cleared(3));
    assert_eq!(s.lighting.light_source_update_value(), 100.0);
    assert_eq!(s.lighting.dirty_global_lights, before + 1);
    // Not before the lighting has run two more updates for the player.
    check_changed_buildings(&mut s.lighting, &mut s.caches, &mut s.pending, 1);
    assert_eq!(s.caches[0].dirty_flags(1, 1, 0), 0);
    let mut frames = 0;
    while s.caches[0].dirty_flags(2, 2, 0) & BUILDINGS_CHANGED != BUILDINGS_CHANGED {
        s.frame();
        frames += 1;
        assert!(frames < 10, "buildings never checked");
    }
    assert!(frames >= 2, "{frames}");
    assert_eq!(s.pending.grid_occlusion_epoch, epoch + 1);
    // Every loaded chunk of player 0's map, for player 0 only.
    for wx in 1..=3 {
        for wy in 1..=3 {
            assert_eq!(
                s.caches[0].dirty_flags(wx, wy, 0) & BUILDINGS_CHANGED,
                BUILDINGS_CHANGED
            );
            assert_eq!(s.caches[1].dirty_flags(wx, wy, 0) & BUILDINGS_CHANGED, 0);
        }
    }
    // Once.
    s.settle();
    for _ in 0..4 {
        s.frame();
    }
    assert_eq!(s.caches[0].dirty_flags(1, 1, 0) & BUILDINGS_CHANGED, 0);
}

#[test]
fn adding_a_wall_re_records_its_chunk_level_and_relights() {
    let mut s = Scene::new();
    s.settle();
    // Ahead of the player, east of x = 17: lit and seen.
    let ahead = &s.render.squares[&(19, 15, 0)];
    assert!(ahead.cut.can_see && ahead.light_level > 0.9);
    assert!(!s.caches[0].begin_level(2, 1, 0, 0, 0).1);

    // A west wall along x = 17 for y in 10..=21, one AddTileObject each.
    let mut regions = IsoRegions::new(3);
    let doors = DoorTables {
        x_offset: [[[0; 4]; 2]; 2],
        y_offset: [[[0; 4]; 2]; 2],
    };
    let wall = s.m.named_map["wall_w"];
    let water = s.m.tables.flag.water;
    {
        let mut render = RenderInvalidation::new(
            &mut s.render,
            &mut s.lighting,
            &mut s.caches,
            &mut s.pending,
            water,
            1,
        );
        let mut env = MutationEnv {
            cell: &mut s.cell,
            meta: &s.meta,
            sprites: &mut s.m,
            regions: &mut regions,
            rand: &mut Zero,
            doors: &doors,
            world: &mut NoMutationHooks,
            pathfind: &mut NoMutationHooks,
            render: &mut render,
            num_players: 1,
        };
        for y in 10..=21 {
            let o = IsoObject::new(0, ObjectKind::Object, Some(wall));
            env.add_tile_object((17, y, 0), pz_world::ObjectSlot::new(o), -1)
                .unwrap();
        }
    }
    // The chunk levels holding the wall are dirty (OBJECT_ADD), for every
    // player; a chunk away from it is not.
    for c in &s.caches {
        assert_ne!(c.dirty_flags(2, 1, 0) & dirty::OBJECT_ADD, 0);
        assert_ne!(c.dirty_flags(2, 2, 0) & dirty::OBJECT_ADD, 0);
        assert_eq!(c.dirty_flags(1, 3, 0), 0);
    }
    assert_eq!(s.lighting.light_check(2, 1, 0, 0), Some((true, true)));
    assert_eq!(s.lighting.light_source_update_value(), 100.0);
    assert!(s.lighting.los_cache_is_cleared(0));
    // The render square still shows the old objects until the frame.
    assert_eq!(s.render.squares[&(17, 15, 0)].objects.len(), 1);

    let dirty_levels = s.frame();
    let sq = &s.render.squares[&(17, 15, 0)];
    assert_eq!(sq.objects.len(), 2);
    assert!(sq.objects[1].sprite.as_ref().unwrap().flags.wall_w);
    // The square north of the wall sees it in its seams.
    assert!(s.render.squares[&(17, 9, 0)].seam.s_wall_w);
    assert!(!s.render.squares[&(16, 9, 0)].seam.s_wall_w);
    // The level is recorded again with the wall.
    let chunk = s.render.chunk_inputs(&[(2, 1)]);
    assert_eq!(chunk[0].levels[0].1[1 + 7 * 8].unwrap().objects.len(), 2);
    assert!(s.caches[0].begin_level(2, 1, 0, 0, 0).1);
    assert!(!s.caches[0].begin_level(2, 1, 0, 0, 0).1);

    // Lighting: the next updates see the wall; the squares behind it go
    // out of sight and their levels are invalidated.
    let mut lit = dirty_levels;
    for _ in 0..12 {
        lit.extend(s.frame());
    }
    assert!(lit.contains(&(2, 1, 0)));
    let behind = &s.render.squares[&(19, 15, 0)];
    assert!(!behind.cut.can_see, "still visible behind the wall");
    let near = &s.render.squares[&(16, 15, 0)];
    assert!(near.cut.can_see);
}

#[test]
fn a_square_going_indoors_re_initialises_its_puddles_next_frame() {
    let mut s = Scene::new();
    // `Core.getPerfPuddles()` 1: no puddles leaking into rooms.
    s.perf_puddles = 1;
    s.settle();
    // Outdoors, open ground all round: puddles everywhere.
    let out = s.render.squares[&(13, 13, 0)].puddles.unwrap();
    assert!(out.render);
    assert_eq!(out.pda, [1.0; 4]);
    assert_eq!(out.pnon, [0.0; 4]);
    let neighbour = s.render.squares[&(14, 13, 0)].puddles;

    // A roof over (13, 13): RecalcProperties drops `exterior`.
    s.cell
        .get_grid_square_mut(13, 13, 0)
        .unwrap()
        .borrow_mut()
        .have_roof = true;
    let doors = DoorTables {
        x_offset: [[[0; 4]; 2]; 2],
        y_offset: [[[0; 4]; 2]; 2],
    };
    let water = s.m.tables.flag.water;
    let mut regions = IsoRegions::new(3);
    {
        let mut render = RenderInvalidation::new(
            &mut s.render,
            &mut s.lighting,
            &mut s.caches,
            &mut s.pending,
            water,
            1,
        );
        let mut env = MutationEnv {
            cell: &mut s.cell,
            meta: &s.meta,
            sprites: &mut s.m,
            regions: &mut regions,
            rand: &mut Zero,
            doors: &doors,
            world: &mut NoMutationHooks,
            pathfind: &mut NoMutationHooks,
            render: &mut render,
            num_players: 1,
        };
        env.recalc_properties((13, 13, 0));
    }
    assert!(!s.sq(13, 13).borrow().has(s.m.tables.flag.exterior));
    assert!(s.pending.puddles.contains(&(13, 13, 0)));
    // Unchanged until the frame re-initialises it.
    assert_eq!(s.render.squares[&(13, 13, 0)].puddles, Some(out));
    s.frame();
    let inside = s.render.squares[&(13, 13, 0)].puddles.unwrap();
    assert!(!inside.render);
    assert_eq!(inside.pda, [0.0; 4]);
    // The neighbours keep theirs (no `recalc` on them).
    assert_eq!(s.render.squares[&(14, 13, 0)].puddles, neighbour);

    // With puddles leaking into rooms (perf 0) the roofed square, open on
    // every side, takes puddles in from its exterior neighbours.
    s.perf_puddles = 0;
    s.pending.puddles.insert((13, 13, 0));
    s.frame();
    let leak = s.render.squares[&(13, 13, 0)].puddles.unwrap();
    assert!(leak.render);
    assert_eq!(leak.pda, [1.0; 4]);
}

#[test]
fn loaded_squares_get_puddles_on_the_first_frame() {
    // Built in `Scene::new` (chunk load): no geometry until a frame runs.
    let mut s = Scene::new();
    assert!(s.render.squares.values().all(|sq| sq.puddles.is_none()));
    assert_eq!(s.render.puddles_recalc.len(), 24 * 24);
    s.frame();
    assert!(s.render.puddles_recalc.is_empty());
    // Every square has its geometry; outdoor ground has puddles, water none.
    assert!(s.render.squares.values().all(|sq| sq.puddles.is_some()));
    let open = s.render.squares[&(20, 20, 0)].puddles.unwrap();
    assert!(open.render);
    assert_eq!(open.pda, [1.0; 4]);
    assert!(!s.render.squares[&(9, 9, 0)].puddles.unwrap().render);

    // A chunk loaded again later: its squares start over, the rest keep
    // theirs.
    let view = WorldView {
        cell: &s.cell,
        meta: &s.meta,
        sprites: &s.m,
    };
    s.render
        .build_chunk(&view, 2, 2, &mut s.sprite_cache, &mut NoTextures);
    assert_eq!(s.render.squares[&(20, 20, 0)].puddles, None);
    assert_eq!(s.render.puddles_recalc.len(), 64);
    s.frame();
    assert_eq!(s.render.squares[&(20, 20, 0)].puddles, Some(open));
}
