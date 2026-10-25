// The JNI entry points (`Java_zombie_popman_*`, `Java_zombie_MapCollisionData_*`).
// Included into `popman.rs`.

/// An `IllegalArgumentException` the JNI layer would throw.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IllegalArgument(pub String);

impl Native {
    // ---- ZombiePopulationManager ------------------------------------------------

    /// `n_init(client, server, minX, minY, width, height)`.
    pub fn n_init(&mut self, client: bool, server: bool, min_x: i32, min_y: i32, width: i32, height: i32) {
        self.gs.server = server;
        self.worker.client = client;
        if client {
            return;
        }
        let w = &mut self.worker;
        w.min_x = min_x;
        w.min_y = min_y;
        w.width = width;
        w.height = height;
        w.cells.clear();
        for y in 0..height.max(0) {
            for x in 0..width.max(0) {
                w.cells.push(Cell::new(min_x + x, min_y + y));
            }
        }
        self.worker.next_save_ms = self.gs.now() + 300_000;
        self.id_file_load();
        self.virtual_file_load();
    }

    /// `n_config`.
    #[allow(clippy::too_many_arguments)]
    pub fn n_config(
        &mut self,
        population_multiplier: f32,
        start_multiplier: f32,
        peak_multiplier: f32,
        peak_day: i32,
        respawn_hours: f32,
        respawn_unseen_hours: f32,
        respawn_multiplier: f32,
        redistribute_hours: f32,
        follow_sound_distance: i32,
    ) {
        let p = self.cur();
        let f = &mut p.config_float;
        f.insert(b"PopulationMultiplier".to_vec(), population_multiplier);
        f.insert(b"PopulationStartMultiplier".to_vec(), start_multiplier);
        f.insert(b"PopulationPeakMultiplier".to_vec(), peak_multiplier);
        p.config_int.insert(b"PopulationPeakDay".to_vec(), peak_day);
        let f = &mut p.config_float;
        f.insert(b"RespawnHours".to_vec(), respawn_hours);
        f.insert(b"RespawnUnseenHours".to_vec(), respawn_unseen_hours);
        f.insert(b"RespawnMultiplier".to_vec(), respawn_multiplier);
        f.insert(b"RedistributeHours".to_vec(), redistribute_hours);
        p.config_int.insert(b"FollowSoundDistance".to_vec(), follow_sound_distance);
        p.config_changed = true;
    }

    /// `n_configFloat(name, value)`.
    pub fn n_config_float(&mut self, name: &str, v: f32) -> Result<(), IllegalArgument> {
        if !FLOAT_KEYS.contains(&name) {
            return Err(IllegalArgument(name.to_string()));
        }
        let p = self.cur();
        p.config_float.insert(name.as_bytes().to_vec(), v);
        p.config_changed = true;
        Ok(())
    }

    /// `n_configInt(name, value)`.
    pub fn n_config_int(&mut self, name: &str, v: i32) -> Result<(), IllegalArgument> {
        if !INT_KEYS.contains(&name) {
            return Err(IllegalArgument(name.to_string()));
        }
        let p = self.cur();
        p.config_int.insert(name.as_bytes().to_vec(), v);
        p.config_changed = true;
        Ok(())
    }

    /// `n_setSpawnOrigins(xywh)` (applied directly to the worker).
    pub fn n_set_spawn_origins(&mut self, xywh: &[i32]) -> Result<(), IllegalArgument> {
        if xywh.len() & 3 != 0 {
            return Err(IllegalArgument("xywh.length should multiple of 4".into()));
        }
        for o in xywh.chunks(4) {
            self.worker.spawn_origins.push([o[0], o[1], o[2], o[3]]);
        }
        Ok(())
    }

    /// `n_setOutfitNames(names)` (names already lower-cased by Java).
    pub fn n_set_outfit_names(&mut self, names: &[&str]) {
        self.outfit_names.clear();
        self.outfit_index.clear();
        for (i, n) in names.iter().enumerate() {
            self.outfit_names.push(n.as_bytes().to_vec());
            self.outfit_index.entry(n.as_bytes().to_vec()).or_insert(i as i32);
        }
    }

    /// `n_updateMain(multiplier, worldAgeHours)`.
    pub fn n_update_main(&mut self, multiplier: f32, hours: f64) {
        {
            let p = self.cur();
            let changed = !(p.multiplier == multiplier && p.hours == hours);
            p.time_changed |= changed;
            p.multiplier = multiplier;
            p.hours = hours;
        }
        self.main.combined.clear();
        while let Some(m) = self.q_to_main.pop_front() {
            let mut src = self.ptms[m].take().unwrap();
            let c = &mut self.main.combined;
            c.add.append(&mut src.add);
            if src.has_radar {
                c.radar.clone_from(&src.radar);
                c.has_radar = true;
            }
            if src.debug.has_data {
                c.debug = src.debug.clone();
            }
            c.repop_events.extend_from_slice(&src.repop_events);
            let rec = std::mem::take(&mut src.recycled);
            self.main.ptw_free.extend(rec);
            while self.main.ptw_free.len() > 50 {
                let p = self.main.ptw_free.pop().unwrap();
                self.ptws[p] = None;
            }
            src.clear();
            self.ptms[m] = Some(src);
            self.cur().recycled.push(m);
        }
        if self.main.combined.has_radar {
            self.main.radar = self.main.combined.radar.clone();
        }
        if self.main.combined.debug.has_data {
            self.mp_debug = self.main.combined.debug.clone();
        }
        if !self.main.combined.repop_events.is_empty() {
            self.mp_debug.repop_events.extend_from_slice(&self.main.combined.repop_events);
        }
    }

    /// `n_hasDataForThread`: queues the current batch for the worker.
    pub fn n_has_data_for_thread(&mut self) -> bool {
        if self.cur().is_empty() {
            return false;
        }
        let p = self.main.cur_ptw;
        self.q_to_worker.push_back(p);
        self.main.cur_ptw = match self.main.ptw_free.pop() {
            Some(f) => f,
            None => self.new_ptw(),
        };
        true
    }

    /// `n_readyToPause`.
    pub fn n_ready_to_pause(&mut self) -> bool {
        let p = self.cur();
        p.chunks.is_empty() && p.add.is_empty() && p.register.is_empty()
    }

    /// `n_updateThread`.
    pub fn n_update_thread(&mut self) {
        self.worker_update_thread();
        let m = self.worker.cur_ptm;
        if !self.ptm(m).is_empty() {
            self.q_to_main.push_back(m);
            self.worker.cur_ptm = match self.worker.ptm_free.pop() {
                Some(f) => f,
                None => self.new_ptm(),
            };
        }
    }

    /// `n_shouldWait`.
    pub fn n_should_wait(&mut self) -> bool {
        self.worker_should_wait()
    }

    /// `n_save`.
    pub fn n_save(&mut self) {
        self.worker_save();
    }

    /// `n_saveCell(cx, cy)`.
    pub fn n_save_cell(&mut self, cx: i32, cy: i32) {
        self.worker_save_cell(cx, cy);
    }

    /// `n_stop`.
    pub fn n_stop(&mut self) {
        let ids: Vec<u32> = self.worker.real_zombies.keys().copied().collect();
        for id in ids {
            self.worker_release_real_zombie(id);
        }
        self.worker.last_reported_unregistered = 0;
        let vzs = std::mem::take(&mut self.worker.vzs);
        for &v in &vzs {
            self.vz_clear_current_cell(v);
        }
        for &v in &vzs {
            let vz = &self.vzs[v as usize];
            let members: Vec<ZId> = if vz.group.len() < 2 { vec![vz.zombie.unwrap()] } else { vz.group.clone() };
            for m in members {
                self.release_zombie(m);
            }
            self.release_vz(v);
        }
        self.worker.vz_dirty = true;
        let loaded = std::mem::take(&mut self.worker.loaded_cells);
        for c in loaded {
            self.worker.cells[c].dirty = false;
            self.cell_unload(c);
        }
        self.worker.cells.clear();
        while let Some(p) = self.q_to_worker.pop_front() {
            let w = self.ptws[p].take().unwrap();
            for z in w.add.iter().chain(w.register.iter()) {
                self.release_zombie(*z);
            }
        }
        self.worker.aggro_targets.clear();
        // The native code drops the list without freeing the sounds.
        self.worker.sounds.clear();
        let m = self.worker.cur_ptm;
        let adds = std::mem::take(&mut self.ptm(m).add);
        for z in adds {
            self.release_zombie(z);
        }
        let rec = std::mem::take(&mut self.ptm(m).recycled);
        for p in rec {
            self.ptws[p] = None;
        }
        self.ptm(m).clear();
        // ManagerMain::stop
        while let Some(p) = self.q_to_main.pop_front() {
            self.ptms[p] = None;
        }
        self.main.combined = PassToMain::default();
        let cur = self.main.cur_ptw;
        let rec = std::mem::take(&mut self.ptw(cur).recycled);
        for p in rec {
            self.ptms[p] = None;
        }
        self.ptw(cur).clear();
        for p in std::mem::take(&mut self.main.ptw_free) {
            self.ptws[p] = None;
        }
        self.meta.clear();
        // The pools are destroyed: forget recycled objects.
        self.zombies.clear();
        self.zombie_free.clear();
        self.vzs.clear();
        self.vz_free.clear();
        for p in std::mem::take(&mut self.worker.ptm_free) {
            self.ptms[p] = None;
        }
        self.worker.loaded_areas.clear();
        self.worker.spawn_origins.clear();
        self.repop_tasks.clear();
    }

    /// `n_addZombie(x, y, z, dir, outfit, state, pathX, pathY, persistentId)`:
    /// returns the id used.
    #[allow(clippy::too_many_arguments)]
    pub fn n_add_zombie(
        &mut self,
        x: f32,
        y: f32,
        z: f32,
        dir: u8,
        outfit: i32,
        state: i32,
        path_x: i32,
        path_y: i32,
        id: u32,
    ) -> u32 {
        let r = self.alloc_zombie();
        let id = if id == 0 { self.next_zombie_id() } else { id };
        self.zombies[r as usize] = Zombie { x, y, z, dir, outfit, state, path_x, path_y, id };
        self.cur().add.push(r);
        id
    }

    /// `n_registerZombie(...)`: returns the persistent id.
    #[allow(clippy::too_many_arguments)]
    pub fn n_register_zombie(&mut self, x: f32, y: f32, z: f32, dir: u8, outfit: i32, state: i32, id: u32) -> u32 {
        let r = self.alloc_zombie();
        let id = if id == 0 { self.next_zombie_id() } else { id };
        self.zombies[r as usize] = Zombie { x, y, z, dir, outfit, state, path_x: NO_PATH, path_y: NO_PATH, id };
        self.cur().register.push(r);
        id
    }

    /// `n_releaseZombie(persistentId)`.
    pub fn n_release_zombie(&mut self, id: u32) {
        self.cur().release.push(id);
    }

    /// `n_aggroTarget(id, x, y)`.
    pub fn n_aggro_target(&mut self, id: i32, x: i32, y: i32) {
        self.cur().aggro.push((id, x, y));
    }

    /// `n_loadChunk(wx, wy, added)`.
    pub fn n_load_chunk(&mut self, wx: i32, wy: i32, added: bool) {
        self.cur().chunks.push((wx, wy, added));
    }

    /// `n_loadedAreas(count, areas, serverCells)`.
    pub fn n_loaded_areas(&mut self, count: i32, areas: &[i32], server_cells: bool) -> Result<(), IllegalArgument> {
        if count < 0 {
            return Err(IllegalArgument("count < 0".into()));
        }
        if (areas.len() as i64) < count as i64 * 4 {
            return Err(IllegalArgument("areas.length < count * 4".into()));
        }
        let p = self.cur();
        let la = if server_cells { &mut p.server_cells } else { &mut p.loaded_areas };
        la.clear();
        la.changed = true;
        for a in areas[..count as usize * 4].chunks(4) {
            la.areas.push([a[0], a[1], a[2], a[3]]);
        }
        Ok(())
    }

    /// `n_updateRealZombies(count, idsAndStates, positions)`.
    pub fn n_update_real_zombies(&mut self, count: i32, ids_states: &[i32], positions: &[f32]) -> Result<(), IllegalArgument> {
        if count < 0 {
            return Err(IllegalArgument("count < 0".into()));
        }
        if (ids_states.len() as i64) < count as i64 * 2 {
            return Err(IllegalArgument("idsAndStates.length < count * 2".into()));
        }
        if (positions.len() as i64) < count as i64 * 3 {
            return Err(IllegalArgument("positions.length < count * 3".into()));
        }
        let p = self.cur();
        p.real.clear();
        for i in 0..count as usize {
            p.real.push(RealUpdate {
                id: ids_states[i * 2] as u32,
                state: ids_states[i * 2 + 1],
                x: positions[i * 3],
                y: positions[i * 3 + 1],
                z: positions[i * 3 + 2],
            });
        }
        Ok(())
    }

    /// `n_spawnHorde(x, y, w, h, targetX, targetY, count)`.
    #[allow(clippy::too_many_arguments)]
    pub fn n_spawn_horde(&mut self, x: i32, y: i32, w: i32, h: i32, tx: f32, ty: f32, count: i32) {
        self.cur().hordes.push(Horde { x, y, w, h, tx, ty, count });
    }

    /// `n_worldSound(x, y, radius, volume)`.
    pub fn n_world_sound(&mut self, x: i32, y: i32, radius: i32, volume: i32) {
        let reuse = std::mem::replace(&mut self.reuse_freed_sound, true);
        let id = match if reuse { self.sound_free.pop() } else { None } {
            Some(id) => id,
            None => {
                let id = self.next_sound_id;
                self.next_sound_id = self.next_sound_id.wrapping_add(1);
                self.sounds.insert(id, WorldSound::default());
                id
            }
        };
        let s = self.sounds.get_mut(&id).unwrap();
        s.x = x;
        s.y = y;
        s.radius = radius;
        s.volume = volume;
        self.cur().sounds.push(id);
    }

    /// `n_getAddZombieCount`.
    pub fn n_get_add_zombie_count(&self) -> i32 {
        self.main.combined.add.len() as i32
    }

    /// `n_getAddZombieData(offset, buffer)`: copies records from `offset`
    /// while a whole record still fits; returns how many. The records go
    /// back to the main side's pool.
    pub fn n_get_add_zombie_data(&mut self, offset: i32, capacity: usize, out: &mut Vec<Zombie>) -> Result<i32, IllegalArgument> {
        let total = self.main.combined.add.len() as i32;
        if offset < 0 || offset >= total {
            return Err(IllegalArgument("invalid offset".into()));
        }
        let mut pos = 0usize;
        let mut n = 0;
        for k in offset..total {
            let z = self.main.combined.add[k as usize];
            if pos + 33 > capacity {
                break;
            }
            pos += 33;
            out.push(self.zombies[z as usize]);
            self.release_zombie(z);
            n += 1;
            if capacity < pos * 2 - (pos - 33) {
                return Ok(n);
            }
        }
        Ok(n)
    }

    /// `n_hasRadarData`.
    pub fn n_has_radar_data(&self) -> bool {
        self.main.combined.has_radar
    }

    /// `n_requestRadarData`.
    pub fn n_request_radar_data(&mut self) {
        self.cur().radar_request = true;
    }

    /// `n_getRadarZombieData(xy)`: fills x/y pairs, returns the count.
    pub fn n_get_radar_zombie_data(&self, xy: &mut [f32]) -> Result<i32, IllegalArgument> {
        if xy.len() < 2 {
            return Err(IllegalArgument("xy.length < 2".into()));
        }
        let pts = &self.main.combined.radar;
        if pts.is_empty() {
            return Ok(0);
        }
        let n = (pts.len() * 2).min(xy.len());
        for i in 0..n {
            xy[i] = if i % 2 == 0 { pts[i / 2].0 } else { pts[i / 2].1 };
        }
        Ok(n as i32 / 2)
    }

    /// `MPDebugInfo.n_requestData`.
    pub fn n_mp_request_data(&mut self) {
        self.cur().debug_request = true;
    }

    /// `DebugCommands.n_debugCommand(cmd, cellX, cellY)`.
    pub fn n_debug_command(&mut self, cmd: i32, x: i32, y: i32) {
        let c = match cmd {
            3 => DebugCmd::SpawnTimeToZero(x, y),
            4 => DebugCmd::ClearZombies(x, y),
            5 => DebugCmd::SpawnNow(x, y),
            _ => return,
        };
        self.cur().debug.push(c);
    }

    /// `setGamePaused` (`n_setGameState("PAUSED", b)`).
    pub fn set_game_paused(&mut self, paused: bool) {
        self.worker.paused = paused;
    }

    // ---- MapCollisionData -------------------------------------------------------

    /// `n_setGameState(String, boolean)`.
    pub fn mcd_set_game_state_bool(&mut self, key: &str, v: bool) -> Result<(), IllegalArgument> {
        if key == "PAUSED" {
            self.set_game_paused(v);
            return Ok(());
        }
        self.gs.set_bool(key, v).map_err(IllegalArgument)
    }

    /// `n_setGameState(String, int)`.
    pub fn mcd_set_game_state_int(&mut self, key: &str, v: i32) -> Result<(), IllegalArgument> {
        self.gs.set_int(key, v).map_err(IllegalArgument)
    }

    /// `n_setGameState(String, String)`.
    pub fn mcd_set_game_state_string(&mut self, key: &str, v: &str) -> Result<(), IllegalArgument> {
        self.gs.set_string(key, v).map_err(IllegalArgument)
    }

    /// `n_setGameState(String, double|float)`: no key is accepted except
    /// `"???"`.
    pub fn mcd_set_game_state_number(&mut self, key: &str) -> Result<(), IllegalArgument> {
        if key == "???" {
            Ok(())
        } else {
            Err(IllegalArgument(format!("unknown key {key}")))
        }
    }

    /// `n_initMetaGrid`.
    pub fn mcd_init_meta_grid(&mut self, min_x: i32, min_y: i32, width: i32, height: i32) {
        self.meta.init(min_x, min_y, width, height);
    }

    /// `n_initMetaCell(cx, cy, chunkdataFile)`.
    pub fn mcd_init_meta_cell(&mut self, cx: i32, cy: i32, file: Option<PathBuf>) {
        self.meta.init_meta_cell(cx, cy, file);
    }

    /// `n_initMetaChunk(cx, cy, wx, wy, intensity)`.
    pub fn mcd_init_meta_chunk(&mut self, cx: i32, cy: i32, wx: i32, wy: i32, intensity: u8) {
        self.meta.init_meta_chunk(cx, cy, wx, wy, intensity);
    }

    /// `n_init`.
    pub fn mcd_init(&mut self, min_x: i32, min_y: i32, width: i32, height: i32) {
        self.mcd.init(min_x, min_y, width, height);
    }

    /// `n_chunkUpdateTask(wx, wy, bits)`.
    pub fn mcd_chunk_update_task(&mut self, wx: i32, wy: i32, bits: &[u8]) -> Result<(), IllegalArgument> {
        if bits.len() < 64 {
            return Err(IllegalArgument("data.length < 10 * 10".into()));
        }
        let mut sq = [0u8; 64];
        sq.copy_from_slice(&bits[..64]);
        self.mcd.chunk_update_task(wx, wy, &sq);
        Ok(())
    }

    /// `n_squareUpdateTask` for one square.
    pub fn mcd_square_update_task(&mut self, x: i32, y: i32, bits: u8) {
        self.mcd.square_update_task(x, y, bits);
    }

    /// `n_pathTask(sx, sy, ex, ey, curXY)`: walks the whole path at once.
    pub fn mcd_path_task(&mut self, sx: i32, sy: i32, ex: i32, ey: i32) -> (i32, i32, i32) {
        let mut wf = std::mem::take(&mut self.path_wall_follower);
        wf.init_path(sx, sy, ex, ey);
        let mut r;
        loop {
            r = wf.update(&mut self.mcd, &self.gs, &self.meta);
            if r != wall_follower::CONTINUE {
                break;
            }
        }
        let (x, y) = wf.cur;
        self.path_wall_follower = wf;
        (r, x, y)
    }

    /// `n_hasDataForThread` (collision updates).
    pub fn mcd_has_data_for_thread(&mut self) -> bool {
        self.mcd.has_data_for_thread()
    }

    /// `n_shouldWait`.
    pub fn mcd_should_wait(&self) -> bool {
        self.mcd.should_wait()
    }

    /// `n_update`.
    pub fn mcd_n_update(&mut self) {
        self.mcd_update();
    }

    /// `n_save`.
    pub fn mcd_save(&mut self) {
        self.mcd.save(&self.gs);
    }

    /// `n_stop`.
    pub fn mcd_stop(&mut self) {
        self.mcd.stop(&self.gs);
    }

    // ---- inspection ------------------------------------------------------------------

    /// All stored zombies of a loaded cell, chunk by chunk.
    pub fn cell_zombies(&self, cx: i32, cy: i32) -> Option<Vec<Vec<Zombie>>> {
        let c = self.cell_from_cell_pos(cx, cy)?;
        let cell = &self.worker.cells[c];
        if !cell.loaded {
            return None;
        }
        Some(cell.chunks.iter().map(|ch| ch.zombies.iter().map(|&z| self.zombies[z as usize]).collect()).collect())
    }

    /// Every virtual zombie: leader record and group size.
    pub fn virtual_zombies(&self) -> Vec<(Zombie, usize)> {
        self.worker
            .vzs
            .iter()
            .map(|&v| (self.zombies[self.leader(v) as usize], self.vzs[v as usize].group_size()))
            .collect()
    }

    /// Population counters of a loaded cell: stored, desired, effective.
    pub fn cell_counts(&self, cx: i32, cy: i32) -> Option<(i32, i32, i32)> {
        let c = self.cell_from_cell_pos(cx, cy)?;
        let cell = &self.worker.cells[c];
        cell.loaded.then(|| (cell.stored as i32, cell.desired as i32, self.cell_effective_population(c)))
    }

    /// Collision bits as the worker sees them.
    pub fn collision_bits(&mut self, x: i32, y: i32) -> u8 {
        self.bits(x, y)
    }

    /// `true` if a chunkdata byte is a uniform chunk type.
    pub fn is_uniform_bits(b: u8) -> bool {
        type_of(b) != 2
    }

    /// The `n_getAddZombieData` records the main side currently holds.
    pub fn pending_add_zombies(&self) -> Vec<Zombie> {
        self.main.combined.add.iter().map(|&z| self.zombies[z as usize]).collect()
    }

    /// The meta grid a `Mcd` and the population share.
    pub fn meta_grid(&self) -> &NativeMetaGrid {
        &self.meta
    }

    /// The collision grid.
    pub fn collision(&self) -> &Mcd {
        &self.mcd
    }
}
