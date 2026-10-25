//! `zombie.iso.areas.isoregion.metagrid.WorldRegionToMetaGrid`: player
//! rooms become meta-grid buildings.
//!
//! After every buffer swap the main thread hands the cells whose world
//! regions changed (`DataRoot.changedCells`) to
//! [`WorldRegionToMetaGrid::client_process_buildings`]. It removes the
//! user-defined buildings of those cells and their eight neighbours from
//! the meta grid, makes a `BuildingDef` (one `RoomDef` per world region,
//! its rects a greedy cover of the region's squares) from every enclosed,
//! at least half-roofed world region and the connected ones around it,
//! drops the buildings that touch or overlap a map building, merges each
//! building with the ones above it that touch it, adds the rest to the
//! meta grid and updates the squares of the chunks involved (room ids,
//! associated buildings, properties, pathfinding).
//!
//! The original keeps one `WorldRegionToMetaGrid` for both `DataRoot`
//! buffers, so the regions it remembers may belong to either; here they are
//! `(root, region)` pairs. `BuildingDef`, `RoomDef` and `RoomRect` come from
//! object pools; a pooled `BuildingDef` keeps the key id its constructor
//! drew from `Rand`, so the pools are kept as free lists of arena slots.
//!
//! Single player: `updateSquares` walks the cell's chunks (sorted by row,
//! then column, the order of player 0's chunk map) instead of every local
//! player's chunk map. The `Core.debug` consistency checks are not ported.

use std::collections::{HashMap, HashSet, VecDeque};

use super::data::DataRoot;
use super::regions::WorldRegionRef;
use crate::chunk::IsoCell;
use crate::defs::{room_id, BuildingDef, BuildingIdx, RoomDef, RoomIdx, RoomRect};
use crate::java::{fastfloor, GameRand};
use crate::meta_grid::IsoMetaGrid;
use crate::sprite::IsoSpriteManager;
use crate::square::IsoDirections;
use crate::square_mutation::{PathfindSquareHooks, RenderSquareHooks, SquareMutationHooks};

/// `ObjectPool.DEFAULT_MAX_SIZE`.
const POOL_MAX: usize = 1024;

/// A world region of one of the two roots (`roots[i]`).
pub type RootRegion = (usize, WorldRegionRef);

/// What `clientProcessBuildings` works on besides the region data.
pub struct BuildingCtx<'a> {
    pub cell: &'a mut IsoCell,
    pub meta: &'a mut IsoMetaGrid,
    pub sprites: &'a IsoSpriteManager,
    /// `new BuildingDef()` draws its key id; a new runtime `IsoBuilding`
    /// draws its `scoreUpdate` (`IsoMetaGrid::get_room_by_id`).
    pub rand: &'a mut dyn GameRand,
    pub world: &'a mut dyn SquareMutationHooks,
    pub pathfind: &'a mut dyn PathfindSquareHooks,
    pub render: &'a mut dyn RenderSquareHooks,
}

/// `WorldRegionToMetaGrid` with its pools.
#[derive(Debug, Default)]
pub struct WorldRegionToMetaGrid {
    building_pool: VecDeque<BuildingIdx>,
    room_pool: VecDeque<RoomIdx>,
    buildings_by_level: HashMap<i32, Vec<BuildingIdx>>,
    building_to_regions: HashMap<BuildingIdx, Vec<RootRegion>>,
    all_world_regions: Vec<RootRegion>,
    done: HashSet<RootRegion>,
    added: HashSet<BuildingIdx>,
    dirty_chunks: Vec<(i32, i32)>,
}

struct Run<'r, 'c, 'a> {
    roots: &'r mut [DataRoot; 2],
    main: usize,
    ctx: &'c mut BuildingCtx<'a>,
}

impl Run<'_, '_, '_> {
    fn enclosed(&self, r: RootRegion) -> bool {
        self.roots[r.0].region_manager.world_region_is_enclosed(r.1)
    }
    fn roofed(&self, r: RootRegion) -> f32 {
        self.roots[r.0]
            .region_manager
            .world_region(r.1)
            .roofed_percentage()
    }
    fn building_def(&self, r: RootRegion) -> Option<BuildingIdx> {
        self.roots[r.0]
            .region_manager
            .world_region(r.1)
            .building_def()
    }
    fn set_building_def(&mut self, r: RootRegion, b: Option<BuildingIdx>) {
        self.roots[r.0].region_manager.set_building_def(r.1, b);
    }
    fn regions_in_cell(&self, cx: i32, cy: i32) -> Vec<RootRegion> {
        self.roots[self.main]
            .get_iso_world_regions_in_cell(cx, cy)
            .into_iter()
            .map(|w| (self.main, w))
            .collect()
    }
    fn bdef(&self, b: BuildingIdx) -> &BuildingDef {
        &self.ctx.meta.building_defs[b.0 as usize]
    }
    fn bdef_mut(&mut self, b: BuildingIdx) -> &mut BuildingDef {
        &mut self.ctx.meta.building_defs[b.0 as usize]
    }
}

/// `BuildingDef.getCellX()` / `getCellY()`.
fn cell_of(d: &BuildingDef) -> (i32, i32) {
    (fastfloor(d.x as f32 / 256.0), fastfloor(d.y as f32 / 256.0))
}

/// `getCellX2()` / `getCellY2()`.
fn cell2_of(d: &BuildingDef) -> (i32, i32) {
    (
        fastfloor((d.x2 - 1) as f32 / 256.0),
        fastfloor((d.y2 - 1) as f32 / 256.0),
    )
}

impl WorldRegionToMetaGrid {
    /// `clientProcessBuildings(changedCells)` for the root `roots[main]`.
    pub fn client_process_buildings(
        &mut self,
        roots: &mut [DataRoot; 2],
        main: usize,
        changed_cells: &[(i32, i32)],
        ctx: &mut BuildingCtx<'_>,
    ) {
        if changed_cells.is_empty() {
            return;
        }
        let mut run = Run { roots, main, ctx };
        let run = &mut run;
        let mut removed = 0;
        let mut added = 0;
        self.dirty_chunks.clear();
        for &(x, y) in changed_cells {
            removed += self.remove_user_defined_buildings_from_cell(run, x, y);
            for d in IsoDirections::ALL {
                let (dx, dy) = d.delta();
                removed += self.remove_user_defined_buildings_from_cell(run, x + dx, y + dy);
            }
        }
        self.done.clear();
        self.added.clear();
        self.all_world_regions.clear();
        for l in self.buildings_by_level.values_mut() {
            l.clear();
        }
        for l in self.building_to_regions.values_mut() {
            l.clear();
        }
        for &(x, y) in changed_cells {
            self.create_buildings_from_regions(run, x, y);
            for d in IsoDirections::ALL {
                let (dx, dy) = d.delta();
                self.create_buildings_from_regions(run, x + dx, y + dy);
            }
        }
        for z in 0..31 {
            let Some(mut list) = self.buildings_by_level.remove(&z) else {
                continue;
            };
            for i in (0..list.len()).rev() {
                let b = list[i];
                if !self.is_adjacent_to_or_overlapping_a_predefined_building(run, b) {
                    continue;
                }
                self.discard_building(run, b, true);
                list.remove(i);
            }
            self.buildings_by_level.insert(z, list);
        }
        for z in 0..31 {
            let list = self.buildings_by_level.get(&z).cloned().unwrap_or_default();
            for b in list {
                self.combine_building_levels(run, b);
            }
        }
        for z in 0..31 {
            let list = self.buildings_by_level.get(&z).cloned().unwrap_or_default();
            for b in list {
                self.add_to_meta_grid(run, b);
                added += 1;
            }
        }
        for l in self.buildings_by_level.values_mut() {
            l.clear();
        }
        for l in self.building_to_regions.values_mut() {
            l.clear();
        }
        if removed == 0 && added == 0 {
            return;
        }
        for r in self.all_world_regions.clone() {
            self.mark_region_chunks_dirty(run, r);
        }
        run.ctx.render.buildings_changed();
        self.update_squares(run);
        run.ctx.render.grid_occlusion_square_changed();
        run.ctx.render.cutaways_square_changed(None);
    }

    fn create_buildings_from_regions(&mut self, run: &mut Run<'_, '_, '_>, cx: i32, cy: i32) {
        for wr in run.regions_in_cell(cx, cy) {
            if !run.enclosed(wr) || run.roofed(wr) < 0.5 || self.done.contains(&wr) {
                continue;
            }
            if let Some(b) = run.building_def(wr) {
                if self.added.contains(&b) {
                    continue;
                }
            }
            let b = self.region_to_building_def(run, wr);
            let m = &mut *run.ctx.meta;
            let level = m.building_defs[b.0 as usize].get_min_level(&m.room_defs);
            self.buildings_by_level.entry(level).or_default().push(b);
            self.added.insert(b);
        }
    }

    fn remove_user_defined_buildings_from_cell(
        &mut self,
        run: &mut Run<'_, '_, '_>,
        cx: i32,
        cy: i32,
    ) -> i32 {
        for wr in run.regions_in_cell(cx, cy) {
            if run.building_def(wr).is_none() {
                continue;
            }
            run.set_building_def(wr, None);
            self.mark_region_chunks_dirty(run, wr);
        }
        let Some(cell) = run.ctx.meta.get_cell_data(cx, cy) else {
            return 0;
        };
        let buildings = cell.buildings.clone();
        let mut removed = 0;
        for &b in buildings.iter().rev() {
            if !run.bdef(b).user_defined {
                continue;
            }
            self.mark_building_chunks_dirty(run, b);
            self.remove_from_meta_grid(run, b);
            self.discard_building(run, b, true);
            removed += 1;
        }
        removed
    }

    fn alloc_building(&mut self, run: &mut Run<'_, '_, '_>) -> BuildingIdx {
        if let Some(b) = self.building_pool.pop_front() {
            return b;
        }
        let def = BuildingDef::new(run.ctx.rand);
        let m = &mut *run.ctx.meta;
        m.building_defs.push(def);
        BuildingIdx(m.building_defs.len() as u32 - 1)
    }

    fn alloc_room(&mut self, run: &mut Run<'_, '_, '_>) -> RoomIdx {
        if let Some(r) = self.room_pool.pop_front() {
            return r;
        }
        let m = &mut *run.ctx.meta;
        m.room_defs.push(RoomDef::new(0, String::new()));
        RoomIdx(m.room_defs.len() as u32 - 1)
    }

    fn region_to_building_def(&mut self, run: &mut Run<'_, '_, '_>, wr: RootRegion) -> BuildingIdx {
        let b = self.alloc_building(run);
        run.bdef_mut(b).user_defined = true;
        self.building_to_regions.insert(b, Vec::new());
        let mut list = Vec::new();
        self.get_connected_regions(run, wr, &mut list);
        for n in list {
            if !self.done.insert(n) {
                continue;
            }
            if !run.enclosed(n) || run.roofed(n) < 0.5 {
                continue;
            }
            if let Some(nb) = run.building_def(n) {
                if self.added.contains(&nb) {
                    continue;
                }
                // "neighbour already has a BuildingDef" (logged upstream).
            }
            let room = self.region_to_room_def(run, n);
            let m = &mut *run.ctx.meta;
            m.room_defs[room.0 as usize].building = Some(b);
            m.room_defs[room.0 as usize].CalculateBounds();
            m.building_defs[b.0 as usize].rooms.push(room);
            run.set_building_def(n, Some(b));
            self.building_to_regions.get_mut(&b).unwrap().push(n);
            self.all_world_regions.push(n);
        }
        let m = &mut *run.ctx.meta;
        m.building_defs[b.0 as usize].CalculateBounds(&m.room_defs);
        b
    }

    fn region_to_room_def(&mut self, run: &mut Run<'_, '_, '_>, wr: RootRegion) -> RoomIdx {
        let room = self.alloc_room(run);
        {
            let r = &mut run.ctx.meta.room_defs[room.0 as usize];
            r.id = 0;
            r.name = String::new();
            r.explored = true;
            r.user_defined = true;
        }
        let root = &run.roots[wr.0];
        let mut z = i32::MAX;
        let crs = root
            .region_manager
            .world_region(wr.1)
            .chunk_regions()
            .to_vec();
        let mut rects = Vec::new();
        for cr in crs {
            let c = root.region_manager.chunk_region(cr);
            if z == i32::MAX {
                z = c.z_layer();
            }
            let Some(dci) = c.data_chunk() else {
                continue;
            };
            let dc = root.chunk(dci);
            let mut free: Vec<i16> = Vec::new();
            for i in (0..64).rev() {
                let (lx, ly) = (i % 8, i / 8);
                if dc.get_square(lx, ly, z) <= 0 || dc.get_iso_chunk_region(lx, ly, z) != Some(cr) {
                    continue;
                }
                free.push(i as i16);
            }
            let (chunk_x, chunk_y) = (dc.chunk_x() * 8, dc.chunk_y() * 8);
            while let Some(&index) = free.last() {
                let (lx, ly) = (index as i32 % 8, index as i32 / 8);
                let rect = add_room_rect(dc, &free, lx, ly, z);
                for y in rect.y..rect.get_y2() {
                    for x in rect.x..rect.get_x2() {
                        let index = (x - chunk_x + (y - chunk_y) * 8) as i16;
                        if let Some(p) = free.iter().position(|&f| f == index) {
                            free.remove(p);
                        }
                    }
                }
                rects.push(rect);
            }
        }
        let r = &mut run.ctx.meta.room_defs[room.0 as usize];
        r.rects.extend(rects);
        r.level = z;
        room
    }

    fn get_connected_regions(
        &self,
        run: &Run<'_, '_, '_>,
        wr: RootRegion,
        out: &mut Vec<RootRegion>,
    ) {
        if out.contains(&wr) {
            return;
        }
        out.push(wr);
        let ns = run.roots[wr.0]
            .region_manager
            .world_region(wr.1)
            .neighbors()
            .to_vec();
        for n in ns {
            let n = (wr.0, n);
            if !run.enclosed(n) || run.roofed(n) < 0.5 {
                continue;
            }
            self.get_connected_regions(run, n, out);
        }
    }

    fn add_to_meta_grid(&mut self, run: &mut Run<'_, '_, '_>, b: BuildingIdx) {
        let (cx, cy) = cell_of(run.bdef(b));
        if run.ctx.meta.get_cell_data(cx, cy).is_none() {
            return;
        }
        let rooms = run.bdef(b).rooms.clone();
        let empty = run.bdef(b).emptyoutside.clone();
        merge_rooms_onto_meta_cell(run.ctx.meta, b, &rooms, cx, cy);
        merge_rooms_onto_meta_cell(run.ctx.meta, b, &empty, cx, cy);
        let m = &mut *run.ctx.meta;
        let meta_id = m.building_defs[b.0 as usize].calculate_meta_id(&m.room_defs, cx, cy);
        m.building_defs[b.0 as usize].meta_id = meta_id;
        let cell = m.get_cell_data_mut(cx, cy).unwrap();
        // "duplicate building metaID" is logged upstream.
        cell.buildings.push(b);
        cell.building_by_meta_id.insert(meta_id, b);
        if !m.buildings.contains(&b) {
            m.buildings.push(b);
        }
        m.recalculate_building_and_room_ids(cx, cy);
    }

    fn remove_from_meta_grid(&mut self, run: &mut Run<'_, '_, '_>, b: BuildingIdx) {
        let (cx, cy) = cell_of(run.bdef(b));
        let Some(cell) = run.ctx.meta.get_cell_data(cx, cy) else {
            return;
        };
        let meta_id = run.bdef(b).meta_id;
        let b = match cell.building_by_meta_id.get(&meta_id) {
            Some(&b1) if b1 != b => b1,
            _ => b,
        };
        let (rooms, empty) = (run.bdef(b).rooms.clone(), run.bdef(b).emptyoutside.clone());
        let m = &mut *run.ctx.meta;
        let room_defs = std::mem::take(&mut m.room_defs);
        if let Some(cell) = m.get_cell_data_mut(cx, cy) {
            cell.remove_rooms(&room_defs, &rooms, -1);
            cell.remove_rooms(&room_defs, &empty, -1);
        }
        m.room_defs = room_defs;
        let c1 = cell_of(&m.building_defs[b.0 as usize]);
        let c2 = cell2_of(&m.building_defs[b.0 as usize]);
        m.remove_rooms_from_adjacent_cells(&rooms, c1, c2, -1);
        m.remove_rooms_from_adjacent_cells(&empty, c1, c2, -1);
        self.remove_building_from_meta_cell(run, b, cx, cy);
        run.ctx.meta.recalculate_building_and_room_ids(cx, cy);
    }

    /// `removeBuildingFromMetaCell` (with `removeRoomDef` for each room).
    fn remove_building_from_meta_cell(
        &mut self,
        run: &mut Run<'_, '_, '_>,
        b: BuildingIdx,
        cx: i32,
        cy: i32,
    ) {
        let m = &mut *run.ctx.meta;
        let def = &m.building_defs[b.0 as usize];
        let rooms: Vec<(RoomIdx, i64, i64)> = def
            .rooms
            .iter()
            .chain(&def.emptyoutside)
            .map(|&r| {
                (
                    r,
                    m.room_defs[r.0 as usize].id,
                    m.room_defs[r.0 as usize].meta_id,
                )
            })
            .collect();
        let (meta_id, bid) = (def.meta_id, def.id);
        let cell = m.get_cell_data_mut(cx, cy).expect("meta cell");
        for &(r, id, rmeta) in &rooms {
            cell.rooms.remove(&id);
            if let Some(i) = cell.room_list.iter().position(|&x| x == r) {
                cell.room_list.remove(i);
            }
            cell.room_by_meta_id.remove(&rmeta);
        }
        if let Some(i) = cell.buildings.iter().position(|&x| x == b) {
            cell.buildings.remove(i);
        }
        cell.building_by_meta_id.remove(&meta_id);
        cell.iso_buildings.borrow_mut().remove(&bid);
        for (_, id, _) in rooms {
            run.ctx.world.iso_room_removed(id);
        }
    }

    fn is_adjacent_to_or_overlapping_a_predefined_building(
        &self,
        run: &Run<'_, '_, '_>,
        b: BuildingIdx,
    ) -> bool {
        let m = &*run.ctx.meta;
        let d = &m.building_defs[b.0 as usize];
        let mut list = Vec::new();
        m.get_buildings_intersecting(d.x - 1, d.y - 1, d.get_w() + 2, d.get_h() + 2, &mut list);
        let basement = |d: &BuildingDef| {
            d.rooms
                .first()
                .is_some_and(|r| m.room_defs[r.0 as usize].level < 0)
        };
        for o in list {
            let od = &m.building_defs[o.0 as usize];
            if b == o || basement(d) != basement(od) || od.user_defined {
                continue;
            }
            if d.is_adjacent_ignore_z(&m.room_defs, od, true) {
                return true;
            }
            if d.overlaps(&m.room_defs, od, true) {
                return true;
            }
        }
        false
    }

    fn combine_building_levels(&mut self, run: &mut Run<'_, '_, '_>, b: BuildingIdx) {
        let m = &mut *run.ctx.meta;
        let max = m.building_defs[b.0 as usize].get_max_level(&m.room_defs);
        for z in max + 1..32 {
            let Some(len) = self.buildings_by_level.get(&z).map(Vec::len) else {
                continue;
            };
            for i in (0..len).rev() {
                let b1 = self.buildings_by_level[&z][i];
                assert!(b1 != b, "building is in two levels");
                let m = &*run.ctx.meta;
                let (d, d1) = (
                    &m.building_defs[b.0 as usize],
                    &m.building_defs[b1.0 as usize],
                );
                if !d.is_adjacent_ignore_z(&m.room_defs, d1, true)
                    && !d.overlaps(&m.room_defs, d1, true)
                {
                    continue;
                }
                run.ctx.meta.add_rooms_of(b, b1);
                let regions = std::mem::take(self.building_to_regions.get_mut(&b1).unwrap());
                self.assign_building_to_regions(run, &regions, Some(b));
                self.discard_building(run, b1, false);
                run.bdef_mut(b).reset_min_max_level();
                self.buildings_by_level.get_mut(&z).unwrap().remove(i);
            }
        }
    }

    fn discard_building(&mut self, run: &mut Run<'_, '_, '_>, b: BuildingIdx, release_rooms: bool) {
        let regions = std::mem::take(
            self.building_to_regions
                .get_mut(&b)
                .expect("discarded building has no region list"),
        );
        self.assign_building_to_regions(run, &regions, None);
        self.building_to_regions.remove(&b);
        if release_rooms {
            let rooms = std::mem::take(&mut run.bdef_mut(b).rooms);
            self.release_rooms(run, &rooms);
            let empty = std::mem::take(&mut run.bdef_mut(b).emptyoutside);
            self.release_rooms(run, &empty);
        } else {
            run.bdef_mut(b).rooms.clear();
            run.bdef_mut(b).emptyoutside.clear();
        }
        run.bdef_mut(b).reset_min_max_level();
        if self.building_pool.len() < POOL_MAX {
            self.building_pool.push_back(b);
        }
    }

    fn release_rooms(&mut self, run: &mut Run<'_, '_, '_>, rooms: &[RoomIdx]) {
        for &r in rooms {
            let room = &mut run.ctx.meta.room_defs[r.0 as usize];
            room.rects.clear();
            room.building = None;
            if self.room_pool.len() < POOL_MAX {
                self.room_pool.push_back(r);
            }
        }
    }

    fn assign_building_to_regions(
        &mut self,
        run: &mut Run<'_, '_, '_>,
        regions: &[RootRegion],
        b: Option<BuildingIdx>,
    ) {
        for &r in regions {
            run.set_building_def(r, b);
            match b {
                None => {
                    if let Some(i) = self.all_world_regions.iter().position(|&x| x == r) {
                        self.all_world_regions.remove(i);
                    }
                }
                Some(b) => {
                    if !self.all_world_regions.contains(&r) {
                        self.all_world_regions.push(r);
                    }
                    self.building_to_regions.get_mut(&b).unwrap().push(r);
                }
            }
        }
    }

    fn mark_chunk_dirty(&mut self, cx: i32, cy: i32) {
        if !self.dirty_chunks.contains(&(cx, cy)) {
            self.dirty_chunks.push((cx, cy));
        }
    }

    fn mark_region_chunks_dirty(&mut self, run: &Run<'_, '_, '_>, r: RootRegion) {
        let root = &run.roots[r.0];
        for &cr in root.region_manager.world_region(r.1).chunk_regions() {
            if let Some(dc) = root.region_manager.chunk_region(cr).data_chunk() {
                let dc = root.chunk(dc);
                self.mark_chunk_dirty(dc.chunk_x(), dc.chunk_y());
            }
        }
    }

    fn mark_building_chunks_dirty(&mut self, run: &Run<'_, '_, '_>, b: BuildingIdx) {
        let Some(v) = run.bdef(b).overlapped_chunks.clone() else {
            return;
        };
        for p in v.as_chunks::<2>().0 {
            self.mark_chunk_dirty(p[0] as i32, p[1] as i32);
        }
    }

    /// `updateSquares()`.
    fn update_squares(&mut self, run: &mut Run<'_, '_, '_>) {
        let mut chunks: Vec<(i32, i32)> = run
            .ctx
            .cell
            .chunks
            .keys()
            .copied()
            .filter(|&(wx, wy)| self.dirty_chunks.contains(&(wx, wy)))
            .collect();
        chunks.sort_by_key(|&(wx, wy)| (wy, wx));
        let squares = |run: &Run<'_, '_, '_>, c: (i32, i32)| -> Vec<(i32, i32, i32)> {
            let ch = &run.ctx.cell.chunks[&c];
            let mut out = Vec::new();
            for z in ch.min_level..=ch.max_level {
                for i in 0..64 {
                    if ch.get_grid_square(i % 8, i / 8, z).is_some() {
                        out.push((c.0 * 8 + i % 8, c.1 * 8 + i / 8, z));
                    }
                }
            }
            out
        };
        let exterior = run.ctx.sprites.tables.flag.exterior;
        for &c in &chunks {
            for (x, y, z) in squares(run, c) {
                let ctx = &mut *run.ctx;
                let room = ctx.meta.get_room_at(x, y, z).map(|r| r.id).unwrap_or(-1);
                let sq = ctx.cell.get_grid_square_mut(x, y, z).unwrap();
                // `setRoomID(roomID)`: `room = chunk.getRoom(roomID)`.
                sq.room_id = room;
                let mut room_idx = None;
                if room != -1 {
                    sq.properties.unset_flag(exterior);
                    if ctx.meta.get_room_by_id(room, ctx.rand).is_some() {
                        room_idx = ctx
                            .meta
                            .get_cell_data(room_id::get_cell_x(room), room_id::get_cell_y(room))
                            .and_then(|cell| cell.rooms.get(&room).copied());
                    }
                }
                if let Some(eo) = ctx.meta.get_empty_outside_at(x, y, z).map(|r| r.id) {
                    let hide = ctx
                        .meta
                        .get_room_by_id(eo, ctx.rand)
                        .and_then(|r| r.building);
                    ctx.cell
                        .get_grid_square_mut(x, y, z)
                        .unwrap()
                        .roof_hide_building = hide;
                }
                if let Some(r) = room_idx {
                    let sq = ctx.cell.get_grid_square(x, y, z).unwrap();
                    ctx.world.room_square_updated(sq, r);
                }
            }
        }
        for &c in &chunks {
            for (x, y, z) in squares(run, c) {
                let ctx = &mut *run.ctx;
                let assoc = ctx.meta.get_associated_building_at(x, y).ok().flatten();
                let sq = ctx.cell.get_grid_square_mut(x, y, z).unwrap();
                sq.associated_building = assoc;
                sq.recalc_properties(ctx.sprites);
                ctx.render.properties_recalculated(sq);
                let sq = ctx.cell.get_grid_square(x, y, z).unwrap();
                // `PolygonalMap2.instance.squareChanged(square)`.
                let native = ctx.pathfind.uses_native_code();
                if !native {
                    ctx.pathfind.square_changed(ctx.cell, (x, y, z));
                }
                ctx.render.invalidate_vispoly_chunk_level(c.0, c.1, z);
                ctx.render.cutaways_square_changed(Some((x, y, z)));
                ctx.world.ambient_walls_square_changed(sq);
                if native {
                    ctx.pathfind.square_changed(ctx.cell, (x, y, z));
                }
            }
        }
        for &c in &chunks {
            run.ctx.render.chunk_buildings_changed(c.0, c.1);
        }
    }
}

/// `getSpan(dataChunk, chunkRegion, lx, ly, z)`.
fn get_span(dc: &super::data::DataChunk, free: &[i16], mut lx: i32, ly: i32, z: i32) -> i32 {
    let mut span = 0;
    while lx < 8 {
        let index = (lx + ly * 8) as i16;
        if dc.get_square(lx, ly, z) <= 0 || !free.contains(&index) {
            break;
        }
        lx += 1;
        span += 1;
    }
    span
}

/// `addRoomRect(roomDef, chunkRegion, lx, ly, z)`: the widest run from
/// `(lx, ly)`, grown down while the rows below are at least as wide.
fn add_room_rect(
    dc: &super::data::DataChunk,
    free: &[i16],
    lx: i32,
    mut ly: i32,
    z: i32,
) -> RoomRect {
    let (x, y) = (lx, ly);
    let mut h = 1;
    let w = get_span(dc, free, lx, ly, z);
    ly += 1;
    while ly < 8 && get_span(dc, free, lx, ly, z) >= w {
        ly += 1;
        h += 1;
    }
    RoomRect::new(dc.chunk_x() * 8 + x, dc.chunk_y() * 8 + y, w, h)
}

/// `WorldRegionToMetaGrid.mergeRoomsOntoMetaCell(buildingDef, rooms,
/// metaCell)`: ids and meta ids from the cell, the cell's maps, then the
/// rooms go into the cell's chunks and the adjacent cells'.
fn merge_rooms_onto_meta_cell(
    m: &mut IsoMetaGrid,
    b: BuildingIdx,
    rooms: &[RoomIdx],
    cx: i32,
    cy: i32,
) {
    if rooms.is_empty() {
        return;
    }
    for &r in rooms {
        let n = m.get_cell_data(cx, cy).unwrap().room_list.len() as i32;
        let id = room_id::make_id(cx, cy, n);
        let room = &mut m.room_defs[r.0 as usize];
        room.id = id;
        room.meta_id = room.calculate_meta_id(cx, cy);
        let meta_id = room.meta_id;
        let cell = m.get_cell_data_mut(cx, cy).unwrap();
        cell.rooms.insert(id, r);
        cell.room_list.push(r);
        cell.room_by_meta_id.insert(meta_id, r);
    }
    let (bx, by) = cell_of(&m.building_defs[b.0 as usize]);
    m.add_rooms_to_cell(bx, by, rooms);
    m.add_rooms_to_adjacent_cells_of(b, rooms);
}
