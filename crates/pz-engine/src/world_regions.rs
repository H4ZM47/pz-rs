//! `IsoRegions.update()` (`WorldRegionToMetaGrid`): turns player-built enclosed
//! roofed rooms into live `IsoMetaGrid` buildings/rooms each frame, updates
//! square `room_id`s, and routes the resulting render and pathfinding
//! invalidations.

use pz_lua_api::bind_world::with_world;
use pz_world::isoregion::metagrid::BuildingCtx;
use pz_world::square_mutation::{NoMutationHooks, RenderSquareHooks, SquareMutationHooks};
use pz_world::{IsoSpriteManager, SquareRef};

use crate::iso_world::IsoWorld;
use crate::world_update::QueuedPathfind;

/// `SquareMutationHooks` for `WorldRegionToMetaGrid::client_process_buildings`.
///
/// `WorldLighting` derives room lights from `IsoMetaGrid`'s `RoomDef`s whenever
/// `RenderInvalidation::chunk_buildings_changed` marks a chunk dirty.
struct RegionWorldHooks;

impl SquareMutationHooks for RegionWorldHooks {
    fn ambient_walls_square_changed(&mut self, _sq: SquareRef) {
        // TODO(port): `FMODAmbientWalls.getInstance().squareChanged(sq)`.
    }

    fn room_square_updated(&mut self, _sq: SquareRef, _room: pz_world::defs::RoomIdx) {
        // TODO(port): runtime `IsoRoom` generator toxicity and `lightSwitches`
        // registration once `IsoCell` owns runtime `IsoRoom` instances.
    }

    fn iso_room_removed(&mut self, _room_id: i64) {
        // TODO(port): `IsoRoom.clear(false)` once `IsoCell` owns runtime
        // `IsoRoom` instances; `WorldLighting` already drops stale room lights
        // on `chunk_buildings_changed`.
    }
}

impl IsoWorld {
    /// `IsoRegions.update()` with [`BuildingCtx`]: swaps queued region updates,
    /// turns player-built rooms into `IsoMetaGrid` buildings, invalidates the
    /// renderer (`RenderInvalidation`), and flushes queued square edits to
    /// `PathHost` after releasing the world borrow.
    pub(crate) fn update_regions(&mut self, player: Option<(f32, f32)>) -> bool {
        let num_players = pz_lua_api::bind_players::num_players();
        let IsoWorld {
            regions,
            sprites,
            render,
            rand,
            pathfind,
            ..
        } = self;
        let (Some(regions), Some(sprites_rc)) = (regions.as_ref(), sprites.as_ref()) else {
            return false;
        };
        let sprites: std::cell::Ref<'_, IsoSpriteManager> = sprites_rc.borrow();
        let water_flag = sprites.tables.flag.water;
        let mut pf = QueuedPathfind::default();

        let swapped = with_world(|w| {
            let Some(cell) = w.cell.as_mut() else {
                return false;
            };
            let Some(meta) = w.meta_grid.as_mut() else {
                // Tests or early states without a meta grid still swap region
                // data via `IsoRegions::update`.
                return regions.borrow_mut().update(cell, &sprites, player);
            };
            let mut render_guard = render.borrow_mut();
            let mut no_render = NoMutationHooks;
            let mut inv = render_guard
                .as_mut()
                .map(|r| r.invalidation(water_flag, num_players));
            let render_hooks: &mut dyn RenderSquareHooks = match inv.as_mut() {
                Some(i) => i,
                None => &mut no_render,
            };
            let mut world_hooks = RegionWorldHooks;
            let mut ctx = BuildingCtx {
                cell,
                meta,
                sprites: &sprites,
                rand,
                world: &mut world_hooks,
                pathfind: &mut pf,
                render: render_hooks,
            };
            regions.borrow_mut().update_with_buildings(&mut ctx, player)
        });

        if let Some(h) = pathfind.as_mut() {
            pf.apply(h, &sprites);
        }
        swapped
    }

    /// Ensures the shared `IsoWorld` `regions` handle is also cleared when the
    /// world stops (`IsoRegions.reset()`).
    pub(crate) fn clear_regions(&mut self) {
        self.regions = None;
        with_world(|w| {
            w.regions = None;
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::path::PathBuf;
    use std::rc::Rc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use pz_fs::GameFs;
    use pz_render::TextureManager;
    use pz_world::isoregion::data::{BIT_HAS_FLOOR, BIT_WALL_N, BIT_WALL_W};
    use pz_world::isoregion::{IsoRegions, JobChunkUpdate, RegionJob};
    use pz_world::{IsoCell, IsoChunk, IsoGridSquare, IsoMetaGrid, IsoSpriteManager};

    use crate::world_render::WorldRender;

    fn temp_dir(tag: &str) -> PathBuf {
        static N: AtomicUsize = AtomicUsize::new(0);
        let n = N.fetch_add(1, Ordering::SeqCst);
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos());
        let d = std::env::temp_dir().join(format!(
            "pz-engine-regions-{tag}-{}-{n}-{nanos}",
            std::process::id()
        ));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    /// Builds a single chunk's `isoregiondata` block payload for levels 0..=max_z.
    fn region_block(cx: i32, cy: i32, levels: &[Vec<i8>]) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(&0i32.to_be_bytes());
        out.extend_from_slice(&pz_world::isoregion::REGION_WORLD_VERSION.to_be_bytes());
        out.extend_from_slice(&cx.to_be_bytes());
        out.extend_from_slice(&cy.to_be_bytes());
        out.extend_from_slice(&(12 + 64 * levels.len() as i32).to_be_bytes());
        out.extend_from_slice(&(levels.len() as i32 - 1).to_be_bytes());
        out.extend_from_slice(&(64 * levels.len() as i32).to_be_bytes());
        for level in levels {
            assert_eq!(level.len(), 64);
            for &b in level {
                out.push(b as u8);
            }
        }
        let size = out.len() as i32;
        out[0..4].copy_from_slice(&size.to_be_bytes());
        out
    }

    fn enclosed_room_level(x0: usize, y0: usize, x1: usize, y1: usize) -> Vec<i8> {
        let mut g = vec![BIT_HAS_FLOOR; 64];
        for x in x0..x1 {
            g[x + y0 * 8] |= BIT_WALL_N;
            g[x + y1 * 8] |= BIT_WALL_N;
        }
        for y in y0..y1 {
            g[x0 + y * 8] |= BIT_WALL_W;
            g[x1 + y * 8] |= BIT_WALL_W;
        }
        g
    }

    fn roof_level(x0: usize, y0: usize, x1: usize, y1: usize) -> Vec<i8> {
        let mut g = vec![0i8; 64];
        for y in y0..y1 {
            for x in x0..x1 {
                g[x + y * 8] |= BIT_HAS_FLOOR;
            }
        }
        g
    }

    fn make_single_chunk_cell() -> IsoCell {
        let mut cell = IsoCell::default();
        let mut ch = IsoChunk::new(0, 0);
        for z in 0..=1 {
            for y in 0..8 {
                for x in 0..8 {
                    ch.set_square(x, y, z, Some(IsoGridSquare::new(x, y, z).into()));
                }
            }
        }
        cell.chunks.insert((0, 0), ch);
        cell
    }

    #[test]
    fn player_built_room_becomes_meta_building_and_invalidates_render_and_pathfind() {
        let root = temp_dir("building_ctx");
        let game_dir = root.join("game");
        let cache_dir = root.join("cache");
        let save_dir = root.join("save");
        std::fs::create_dir_all(&game_dir).unwrap();
        std::fs::create_dir_all(&cache_dir).unwrap();
        std::fs::create_dir_all(&save_dir).unwrap();

        let cell = make_single_chunk_cell();
        let mut meta = IsoMetaGrid::blank(0, 0, 0, 0);
        meta.create_cell(0, 0);
        pz_lua_api::bind_world::set_cell(Some(cell));
        pz_lua_api::bind_world::set_meta_grid(Some(meta));

        let sprites = Rc::new(RefCell::new(IsoSpriteManager::new(
            pz_world::sprite::test_tables(),
        )));
        let mut fs = GameFs::new(&game_dir);
        let mut textures = TextureManager::new();
        let renderer = WorldRender::new(&game_dir, &mut fs, &mut textures);
        let pathhost = pz_pathfind::host::PathHost::new(
            pz_pathfind::service::PathfindService::new(0, 0, 1, 1, false),
        );

        let mut regions = IsoRegions::new(13);
        let mut job = JobChunkUpdate::new();
        job.add_chunk_from_file(&region_block(
            0,
            0,
            &[enclosed_room_level(2, 2, 5, 6), roof_level(2, 2, 5, 6)],
        ));
        regions.enqueue_job(RegionJob::ChunkUpdate(job));
        regions.apply_chunk_changes(false);

        let mut world = IsoWorld::new(&game_dir, &cache_dir, save_dir);
        world.sprites = Some(sprites.clone());
        world.regions = Some(Rc::new(RefCell::new(regions)));
        *world.render.borrow_mut() = Some(renderer);
        world.pathfind = Some(pathhost);

        assert!(world.update_regions(None));

        // 1. Meta grid now contains a user-defined building + room and interior
        //    squares carry the new room_id.
        let room_id = with_world(|w| {
            let meta = w.meta_grid.as_ref().unwrap();
            let cell_data = meta.get_cell_data(0, 0).unwrap();
            assert_eq!(cell_data.buildings.len(), 1);
            let b = &meta.building_defs[cell_data.buildings[0].0 as usize];
            assert!(b.user_defined);
            assert_eq!((b.x, b.y, b.x2, b.y2), (2, 2, 5, 6));
            assert_eq!(b.rooms.len(), 1);
            let rdef = &meta.room_defs[b.rooms[0].0 as usize];
            assert!(rdef.user_defined && rdef.explored);

            let cell = w.cell.as_ref().unwrap();
            assert_eq!(
                cell.get_grid_square(3, 3, 0).unwrap().borrow().room_id,
                rdef.id
            );
            assert_eq!(cell.get_grid_square(0, 0, 0).unwrap().borrow().room_id, -1);
            rdef.id
        });
        assert_ne!(room_id, -1);

        // 2. RenderInvalidation recorded building, cutaway, occlusion and
        //    vispoly invalidations.
        {
            let guard = world.render.borrow();
            let r = guard.as_ref().unwrap();
            assert!(r.pending.has_rebuilds());
            assert!(r.pending.cutaways_changed);
            assert!(r.pending.grid_occlusion_epoch > 0);
            assert!(r.pending.vispoly.contains(&(0, 0, 0)));
        }

        // 3. Opening a wall removes the user-defined building and clears
        //    square room_id back to -1.
        {
            let r_rc = world.regions.as_ref().unwrap();
            let mut r = r_rc.borrow_mut();
            let mut open = enclosed_room_level(2, 2, 5, 6);
            open[3 + 2 * 8] = BIT_HAS_FLOOR;
            let mut job = JobChunkUpdate::new();
            job.add_chunk_from_file(&region_block(0, 0, &[open, roof_level(2, 2, 5, 6)]));
            r.enqueue_job(RegionJob::ChunkUpdate(job));
            r.apply_chunk_changes(false);
        }
        assert!(world.update_regions(None));

        with_world(|w| {
            let meta = w.meta_grid.as_ref().unwrap();
            assert!(meta.get_cell_data(0, 0).unwrap().buildings.is_empty());
            let cell = w.cell.as_ref().unwrap();
            assert_eq!(cell.get_grid_square(3, 3, 0).unwrap().borrow().room_id, -1);
        });

        world.clear_regions();
        assert!(world.regions.is_none());
        assert!(with_world(|w| w.regions.is_none()));
        pz_lua_api::bind_world::set_cell(None);
        pz_lua_api::bind_world::set_meta_grid(None);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn saved_isoregiondata_loads_and_builds_rooms_on_first_update() {
        let root = temp_dir("saved_regions");
        let game_dir = root.join("game");
        let cache_dir = root.join("cache");
        let save_dir = root.join("save");
        std::fs::create_dir_all(&game_dir).unwrap();
        std::fs::create_dir_all(&cache_dir).unwrap();
        std::fs::create_dir_all(&save_dir).unwrap();

        let sprites = Rc::new(RefCell::new(IsoSpriteManager::new(
            pz_world::sprite::test_tables(),
        )));

        // Session 1: write a player-built room to save_dir/isoregiondata.
        {
            let cell = make_single_chunk_cell();
            let mut meta = IsoMetaGrid::blank(0, 0, 0, 0);
            meta.create_cell(0, 0);
            pz_lua_api::bind_world::set_cell(Some(cell));
            pz_lua_api::bind_world::set_meta_grid(Some(meta));

            let mut regions = IsoRegions::init_world(&save_dir, 13, false);
            let mut job = JobChunkUpdate::new();
            job.add_chunk_from_file(&region_block(
                0,
                0,
                &[enclosed_room_level(1, 1, 4, 4), roof_level(1, 1, 4, 4)],
            ));
            regions.enqueue_job(RegionJob::ChunkUpdate(job));
            regions.apply_chunk_changes(true);

            let mut world = IsoWorld::new(&game_dir, &cache_dir, save_dir.clone());
            world.sprites = Some(sprites.clone());
            world.regions = Some(Rc::new(RefCell::new(regions)));
            assert!(world.update_regions(None));
            world.clear_regions();
        }
        assert!(save_dir.join("isoregiondata").join("datachunk_0_0.bin").is_file());

        // Session 2: reload from save_dir via init_world; first update_regions
        // swaps the saved region data in and reconstructs the building def.
        {
            let cell = make_single_chunk_cell();
            let mut meta = IsoMetaGrid::blank(0, 0, 0, 0);
            meta.create_cell(0, 0);
            pz_lua_api::bind_world::set_cell(Some(cell));
            pz_lua_api::bind_world::set_meta_grid(Some(meta));

            let regions = IsoRegions::init_world(&save_dir, 13, false);
            assert!(regions.get_iso_world_region(2, 2, 0).is_none());

            let mut world = IsoWorld::new(&game_dir, &cache_dir, save_dir.clone());
            world.sprites = Some(sprites);
            world.regions = Some(Rc::new(RefCell::new(regions)));
            assert!(world.update_regions(None));

            assert!(world
                .regions
                .as_ref()
                .unwrap()
                .borrow()
                .get_iso_world_region(2, 2, 0)
                .is_some());
            with_world(|w| {
                let meta = w.meta_grid.as_ref().unwrap();
                assert_eq!(meta.get_cell_data(0, 0).unwrap().buildings.len(), 1);
                let cell = w.cell.as_ref().unwrap();
                assert_ne!(cell.get_grid_square(2, 2, 0).unwrap().borrow().room_id, -1);
            });
            world.clear_regions();
        }

        pz_lua_api::bind_world::set_cell(None);
        pz_lua_api::bind_world::set_meta_grid(None);
        let _ = std::fs::remove_dir_all(&root);
    }
}
