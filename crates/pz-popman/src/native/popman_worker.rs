// The worker half of the library (`popman::ManagerWorker` and friends).
// Included into `popman.rs`.

impl Native {
    // ---- virtual zombie movement ------------------------------------------

    /// `VirtualZombie::WalkTowardState`.
    fn vz_walk_toward(&mut self, v: VzId) {
        let lead = self.leader(v);
        let z = self.zombies[lead as usize];
        let fx = ffloor(z.x);
        if fx == z.path_x && ffloor(z.y) == z.path_y {
            self.collided.this_frame = true;
            return;
        }
        let mut target = None;
        if self.gs.last_stand {
            let fy = ffloor(z.y);
            if self.bits(fx, fy) & BLOCK_MASK == 0 {
                let mut wf = std::mem::take(&mut self.vz_wall_follower);
                wf.init_path(fx, fy, z.path_x, z.path_y);
                let mut r;
                loop {
                    r = wf.update(&mut self.mcd, &self.gs, &self.meta);
                    if r != wall_follower::CONTINUE {
                        break;
                    }
                }
                if r == wall_follower::REACHED {
                    // The path's entries after the first one that equals
                    // the start.
                    let n = wf.path.len();
                    let mut k = 0usize;
                    for (i, p) in wf.path.iter().enumerate().skip(1) {
                        if *p == wf.start {
                            k = i;
                            break;
                        }
                    }
                    if n as i32 - k as i32 >= 2 {
                        target = Some(wf.path[k + 1]);
                    }
                }
                self.vz_wall_follower = wf;
            }
        }
        let target = match target {
            Some(t) => t,
            None => {
                let z = self.zombies[lead as usize];
                if !self.vz_is_clear_ahead(ffloor(z.x), ffloor(z.y), z.path_x, z.path_y, 10) {
                    self.collided.this_frame = true;
                    return;
                }
                (z.path_x, z.path_y)
            }
        };
        let z = self.zombies[lead as usize];
        let dx = (target.0 as f32 + 0.5) - z.x;
        let dy = (target.1 as f32 + 0.5) - z.y;
        let d = dx.mul_add(dx, dy * dy).sqrt();
        let (vx, vy) = if d < 0.001 { (0.0, 0.0) } else { ((dx / d) * 0.06, (dy / d) * 0.06) };
        let m = self.gs.multiplier;
        let vz = &mut self.vzs[v as usize];
        vz.next.0 = vx.mul_add(m, vz.next.0);
        vz.next.1 = vy.mul_add(m, vz.next.1);
    }

    /// `VirtualZombie::isClearAhead`.
    fn vz_is_clear_ahead(&mut self, mut x1: i32, mut y1: i32, x2: i32, y2: i32, mut steps: i32) -> bool {
        let dy = y2.wrapping_sub(y1);
        let dx = x2.wrapping_sub(x1);
        if dx.unsigned_abs() > dy.unsigned_abs() {
            let slope = dy as f32 / dx as f32;
            let mut fy = y1 as f32 + 0.5;
            let sx = (dx >> 31) | 1;
            let step = slope * sx as f32;
            loop {
                if x2 == x1 || steps < 1 {
                    return true;
                }
                steps -= 1;
                let iy = fy as i32;
                let nx = x1.wrapping_add(sx);
                fy += step;
                if self.blocked(x1, iy, nx, fy as i32) {
                    return false;
                }
                x1 = nx;
            }
        } else {
            let slope = dx as f32 / dy as f32;
            let mut fx = x1 as f32 + 0.5;
            let sy = (dy >> 31) | 1;
            let step = slope * sy as f32;
            loop {
                if y2 == y1 || steps < 1 {
                    return true;
                }
                steps -= 1;
                let ix = fx as i32;
                fx += step;
                let ny = y1.wrapping_add(sy);
                if self.blocked(ix, y1, fx as i32, ny) {
                    return false;
                }
                y1 = ny;
            }
        }
    }

    /// `VirtualZombie::DoCollide(mode)`.
    fn vz_do_collide(&mut self, v: VzId, mode: i32) -> bool {
        let vz = self.vzs[v as usize].clone();
        let (s0, s1) = vz.last;
        let (s3, s2) = vz.next;
        let lx = ffloor(s0);
        let ly = ffloor(s1);
        let nx = ffloor(s3);
        let ny = ffloor(s2);
        let mx = self.worker.min_x.wrapping_mul(256);
        let my = self.worker.min_y.wrapping_mul(256);
        let w14 = nx.wrapping_sub(mx);
        let mut oob = true;
        if w14 >= 0 {
            let w15 = ny.wrapping_sub(my);
            if w15 >= 0 {
                oob = w14 >= self.worker.width.wrapping_mul(256) || w15 >= self.worker.height.wrapping_mul(256);
            }
        }
        let same_x = nx == lx;
        if same_x && !((ny == ly) || oob) || (!same_x && !oob) {
            // Crossed a square boundary: check the move.
            if !self.blocked(lx, ly, nx, ny) {
                return false;
            }
            let c = &mut self.collided;
            if lx < nx {
                c.e = true;
            }
            if lx > nx {
                c.w = true;
            }
            if ly < ny {
                c.s = true;
            }
            if ly > ny {
                c.n = true;
            }
            return self.collide_mode(mode);
        }
        if s3 == s0 && s2 == s1 {
            return false;
        }
        if oob {
            let c = &mut self.collided;
            if s3 < s0 {
                c.w = true;
            }
            if s3 > s0 {
                c.e = true;
            }
            if s2 < s1 {
                c.n = true;
            }
            if s2 > s1 {
                c.s = true;
            }
            self.vzs[v as usize].next = (s0, s1);
            self.collided.this_frame = true;
            return true;
        }
        // Same square: probe half a square ahead of the leader.
        let dx = s3 - s0;
        let dy = s2 - s1;
        let d = dx.mul_add(dx, dy * dy).sqrt();
        let (ox, oy) = if d < 0.001 { (0.0, 0.0) } else { ((dx / d) * 0.5, (dy / d) * 0.5) };
        let (zx, zy) = self.leader_pos(v);
        let px = ffloor(ox + zx);
        let py = ffloor(oy + zy);
        let rx = px.wrapping_sub(mx);
        if rx < 0 {
            return false;
        }
        let ry = py.wrapping_sub(my);
        if ry < 0 || rx >= self.worker.width.wrapping_mul(256) || ry >= self.worker.height.wrapping_mul(256) {
            return false;
        }
        if px == nx && py == ny {
            return false;
        }
        if !self.blocked(nx, ny, px, py) {
            return false;
        }
        let c = &mut self.collided;
        if nx < px {
            c.e = true;
        }
        if nx > px {
            c.w = true;
        }
        if ny < py {
            c.s = true;
        }
        if ny > py {
            c.n = true;
        }
        self.collide_mode(mode)
    }

    fn collide_mode(&mut self, mode: i32) -> bool {
        let c = &mut self.collided;
        if mode == 1 {
            if c.n || c.s {
                c.w = false;
                c.e = false;
            }
        } else if mode == 2 && (c.w || c.e) {
            c.s = false;
            c.n = false;
        }
        c.this_frame = true;
        true
    }

    /// `VirtualZombie::postupdate`.
    fn vz_postupdate(&mut self, v: VzId) {
        let mx = self.worker.min_x.wrapping_mul(256) as f32;
        let my = self.worker.min_y.wrapping_mul(256) as f32;
        let (x0, y0) = self.leader_pos(v);
        let (mut s8, mut s9);
        {
            let vz = &mut self.vzs[v as usize];
            vz.next = (vz.vel.0 + vz.next.0, vz.vel.1 + vz.next.1);
            s8 = vz.next.0;
            if s8 < mx {
                vz.next.0 = mx;
                s8 = mx;
            }
            s9 = vz.next.1;
            if s9 < my {
                vz.next.1 = my;
                s9 = my;
            }
            let dx = s8 - x0;
            let dy = s9 - y0;
            let d = dx.mul_add(dx, dy * dy).sqrt();
            if d > 1.0 {
                let (ux, uy) = if d < 0.001 { (0.0, 0.0) } else { (dx / d, dy / d) };
                s8 = x0 + ux;
                s9 = y0 + uy;
                vz.next = (s8, s9);
            }
            vz.vel = (0.0, 0.0);
        }
        self.collided = Collided::default();
        let t = self.vzs[v as usize].toggle;
        self.vz_do_collide(v, if t { 2 } else { 1 });
        self.fix_ns(v);
        self.fix_we(v);
        self.vz_do_collide(v, if t { 1 } else { 2 });
        self.vzs[v as usize].toggle = !t;
        let ns = self.collided.n || self.collided.s;
        if ns {
            let vz = &mut self.vzs[v as usize];
            vz.next.1 = vz.last.1;
        }
        let (s10, s11, s12, s13);
        if !(self.collided.w || self.collided.e) {
            let vz = &mut self.vzs[v as usize];
            s10 = vz.next.0;
            s11 = vz.next.1;
            s12 = vz.last.0;
            s13 = vz.last.1;
            vz.next = (s8, s9);
            if !ns {
                self.vz_finish(v, s8, s9);
                return;
            }
        } else {
            let vz = &mut self.vzs[v as usize];
            s12 = vz.last.0;
            s13 = vz.last.1;
            s10 = s12;
            s11 = vz.next.1;
            vz.next = (s8, s9);
        }
        self.vz_do_collide(v, if t { 1 } else { 2 });
        self.fix_ns(v);
        self.fix_we(v);
        let a8 = (s10 - s12).abs();
        let a9 = (s11 - s13).abs();
        self.vz_do_collide(v, if !t { 1 } else { 2 });
        self.fix_ns(v);
        let s0 = a8 + a9;
        let vz = &mut self.vzs[v as usize];
        let (mut f8, f1);
        if !(self.collided.w || self.collided.e) {
            f8 = vz.next.0;
            f1 = vz.last.0;
        } else {
            f1 = vz.last.0;
            vz.next.0 = vz.last.0;
            f8 = f1;
        }
        let mut f9 = vz.next.1;
        let s1 = (f8 - f1).abs() + (f9 - vz.last.1).abs();
        if s1 < s0 {
            vz.next = (s10, s11);
            f8 = s10;
            f9 = s11;
        }
        self.vz_finish(v, f8, f9);
    }

    fn fix_ns(&mut self, v: VzId) {
        if self.collided.n || self.collided.s {
            let vz = &mut self.vzs[v as usize];
            vz.next.1 = vz.last.1;
        }
    }

    fn fix_we(&mut self, v: VzId) {
        if self.collided.w || self.collided.e {
            let vz = &mut self.vzs[v as usize];
            vz.next.0 = vz.last.0;
        }
    }

    fn vz_finish(&mut self, v: VzId, x: f32, y: f32) {
        let lead = self.leader(v);
        let z = &mut self.zombies[lead as usize];
        z.x += x - z.x;
        z.y += y - z.y;
    }

    /// `VirtualZombie::expandGroup`: spreads the group over the squares
    /// reachable from the leader inside its chunk.
    fn vz_expand_group(&mut self, v: VzId) {
        let (x, y) = self.leader_pos(v);
        let fx = ffloor(x);
        let fy = ffloor(y);
        let ox = (if fx < 0 { fx + 7 } else { fx }) & !7;
        let oy = (if fy < 0 { fy + 7 } else { fy }) & !7;
        let mut ff = std::mem::replace(&mut self.ff_expand, FloodFill::new(0));
        self.flood_one_chunk(&mut ff, fx, fy, ox, oy);
        let members = self.vzs[v as usize].group.clone();
        for m in members {
            let k = self.rand.next(ff.visited.len() as i32) as usize;
            let (vx, vy) = ff.visited.get(k).copied().unwrap_or((0, 0));
            let px = (ff.origin.0 + vx as i32) as f32 + 0.5;
            let py = (ff.origin.1 + vy as i32) as f32 + 0.5;
            let z = &mut self.zombies[m as usize];
            z.x = px;
            z.y = py;
        }
        self.ff_expand = ff;
    }

    /// A `FloodFillOneChunk` fill (8x8, blocked squares stop it).
    fn flood_one_chunk(&mut self, ff: &mut FloodFill, sx: i32, sy: i32, ox: i32, oy: i32) {
        ff.calculate(
            self,
            sx,
            sy,
            ox,
            oy,
            |_, _| {},
            |f, n: &mut Native, x1, y1, x2, y2| f.in_window_unvisited(x2, y2) && !n.blocked(x1, y1, x2, y2),
        );
    }

    /// Hands a virtual zombie's zombies to the main thread (they become real).
    /// `copy_path`: the members take the leader's path target (only when
    /// the group enters a loaded chunk, not when it hits the loaded edge).
    fn vz_pass_to_main(&mut self, v: VzId, copy_path: bool) {
        if self.vzs[v as usize].group_size() < 2 {
            let lead = self.leader(v);
            self.cur_ptm().add.push(lead);
        } else {
            self.vz_expand_group(v);
            let lead = self.leader(v);
            let (px, py) = (self.zombies[lead as usize].path_x, self.zombies[lead as usize].path_y);
            let members = self.vzs[v as usize].group.clone();
            for m in members {
                if copy_path {
                    self.zombies[m as usize].path_x = px;
                    self.zombies[m as usize].path_y = py;
                }
                self.cur_ptm().add.push(m);
            }
        }
    }

    /// The common tail of `VirtualZombie::updateThread` when the virtual
    /// zombie goes away: unhook it, recycle it and drop it from the list.
    fn vz_retire(&mut self, v: VzId, idx: i32) -> i32 {
        {
            let vz = &mut self.vzs[v as usize];
            vz.zombie = None;
            vz.group.clear();
        }
        self.vz_clear_current_cell(v);
        let vz = &mut self.vzs[v as usize];
        vz.zombie = None;
        vz.group.clear();
        vz.sound = None;
        vz.is_real = false;
        self.vz_free.push(v);
        if idx >= 0 && (idx as usize) < self.worker.vzs.len() {
            self.worker.vzs.remove(idx as usize);
        }
        self.worker.vz_dirty = true;
        idx - 1
    }

    /// `VirtualZombie::updateThread(index)`: returns the index to continue
    /// from.
    fn vz_update_thread(&mut self, v: VzId, idx: i32) -> i32 {
        if self.vzs[v as usize].cell.is_none() {
            self.vz_update_current_cell(v);
        }
        if let Some(c) = self.vzs[v as usize].cell {
            let (x, y) = self.leader_pos(v);
            let (fx, fy) = (ffloor(x), ffloor(y));
            if self.worker.cells[c].is_chunk_loaded_world(fx, fy) && !self.on_edge_of_loaded_area(fx, fy) {
                self.vz_pass_to_main(v, true);
                return self.vz_retire(v, idx);
            }
        }
        let pos = self.leader_pos(v);
        {
            let vz = &mut self.vzs[v as usize];
            vz.next = pos;
            vz.last = pos;
        }
        self.collided.this_frame = false;
        self.vz_walk_toward(v);
        let collided = if !self.collided.this_frame {
            self.vz_postupdate(v);
            let c = self.collided.this_frame;
            self.vz_update_current_cell(v);
            c
        } else {
            self.vz_update_current_cell(v);
            true
        };
        let Some(c) = self.vzs[v as usize].cell else { return idx };
        self.load_if_needed(c);
        let (x, y) = self.leader_pos(v);
        let (fx, fy) = (ffloor(x), ffloor(y));
        if !collided {
            let vz = &mut self.vzs[v as usize];
            if vz.sound.is_none() {
                return idx;
            }
            if ffloor(vz.last.0) == fx && ffloor(vz.last.1) == fy {
                return idx;
            }
            vz.sound_steps = vz.sound_steps.wrapping_add(1);
            if (vz.sound_steps as i32) < self.worker.config.follow_sound_distance {
                return idx;
            }
        } else {
            let c = self.vzs[v as usize].cell.unwrap();
            if self.worker.cells[c].is_chunk_loaded_world(fx, fy) && self.on_edge_of_loaded_area(fx, fy) {
                self.vz_pass_to_main(v, false);
                return self.vz_retire(v, idx);
            }
        }
        if !self.worker.aggro_targets.is_empty() && self.closest_aggro_target(fx, fy).is_some() {
            return idx;
        }
        if let Some(c2) = self.cell_from_world(fx, fy) {
            self.load_if_needed(c2);
            let ci = self.worker.cells[c2].chunk_index_world(fx, fy);
            if self.vzs[v as usize].group_size() < 2 {
                let lead = self.leader(v);
                if let Some(i) = ci {
                    self.worker.cells[c2].chunks[i].zombies.push(lead);
                }
                self.worker.cells[c2].dirty = true;
                self.worker.cells[c2].stored = self.worker.cells[c2].stored.wrapping_add(1);
            } else {
                self.vz_expand_group(v);
                let members = self.vzs[v as usize].group.clone();
                if let Some(i) = ci {
                    self.worker.cells[c2].chunks[i].zombies.extend_from_slice(&members);
                }
                self.worker.cells[c2].dirty = true;
                self.worker.cells[c2].stored = self.worker.cells[c2].stored.wrapping_add(members.len() as i16);
            }
        }
        self.vz_retire(v, idx)
    }

    // ---- worker queries ------------------------------------------------------

    /// `onEdgeOfLoadedArea`.
    fn on_edge_of_loaded_area(&self, x: i32, y: i32) -> bool {
        if self.gs.server {
            self.worker.server_cells.is_on_edge(x, y)
        } else {
            self.worker.loaded_areas.is_on_edge(x, y)
        }
    }

    /// `closestAggroTarget`: index of the nearest target under 300 squares
    /// (Manhattan).
    fn closest_aggro_target(&self, x: i32, y: i32) -> Option<usize> {
        let mut best = i32::MAX;
        let mut found = None;
        for (i, &(_, tx, ty)) in self.worker.aggro_targets.iter().enumerate() {
            let d = (tx.wrapping_sub(x).wrapping_abs()).wrapping_add(ty.wrapping_sub(y).wrapping_abs());
            if !(best <= d || (d as u32) > 299) {
                best = d;
                found = Some(i);
            }
        }
        found
    }

    /// `setAggroTarget`: (-1, -1) removes the target.
    fn set_aggro_target(&mut self, t: (i32, i32, i32)) {
        let list = &mut self.worker.aggro_targets;
        if let Some(i) = list.iter().position(|a| a.0 == t.0) {
            if t.1 != -1 || t.2 != -1 {
                list[i].1 = t.1;
                list[i].2 = t.2;
                return;
            }
            let cur = list[i];
            if let Some(j) = list.iter().position(|a| *a == cur) {
                list.remove(j);
            }
            return;
        }
        list.push(t);
    }

    /// `ManagerWorker::addZombie`: a zombie the game virtualized.
    fn worker_add_zombie(&mut self, z: ZId) {
        let r = self.zombies[z as usize];
        if r.path_x != NO_PATH {
            let v = self.alloc_vz();
            self.vz_init(v, z, Some((r.path_x, r.path_y)));
            self.worker.vzs.push(v);
            self.worker.vz_dirty = true;
            self.vz_update_current_cell(v);
            return;
        }
        if r.z as i32 == 0 && !self.worker.aggro_targets.is_empty() {
            let fx = ffloor(r.x);
            let fy = ffloor(r.y);
            if self.closest_aggro_target(fx, fy).is_some() && self.bits(fx, fy) & BLOCK_MASK == 0 {
                let v = self.alloc_vz();
                self.vz_init(v, z, Some((NO_PATH, NO_PATH)));
                self.vz_update_current_cell(v);
                self.worker.vzs.push(v);
                self.worker.vz_dirty = true;
                return;
            }
        }
        if let Some(c) = self.cell_from_world(ffloor(r.x), ffloor(r.y)) {
            self.load_if_needed(c);
            if let Some(i) = self.worker.cells[c].chunk_index_world(ffloor(r.x), ffloor(r.y)) {
                self.worker.cells[c].chunks[i].zombies.push(z);
                self.worker.cells[c].dirty = true;
                self.worker.cells[c].stored = self.worker.cells[c].stored.wrapping_add(1);
            }
        }
    }

    /// `ManagerWorker::addRealZombie`: tracks a real zombie for saving.
    fn worker_add_real_zombie(&mut self, z: ZId) {
        let id = self.zombies[z as usize].id;
        if self.worker.real_zombies.contains_key(&id) {
            self.write_log(2, format!("ZPOP: zombie id {id} was registered while already registered"));
            self.worker_release_real_zombie(id);
        }
        let v = self.alloc_vz();
        self.vz_init(v, z, Some((NO_PATH, NO_PATH)));
        self.vzs[v as usize].is_real = true;
        self.worker.real_zombies.insert(id, v);
        self.vz_update_current_cell(v);
    }

    /// `ManagerWorker::releaseRealZombie`.
    fn worker_release_real_zombie(&mut self, id: u32) {
        let Some(v) = self.worker.real_zombies.remove(&id) else { return };
        self.vz_clear_current_cell(v);
        let vz = &self.vzs[v as usize];
        let members: Vec<ZId> = if vz.group.len() < 2 { vec![vz.zombie.unwrap()] } else { vz.group.clone() };
        for m in members {
            self.release_zombie(m);
        }
        self.vzs[v as usize].group.clear();
        self.vzs[v as usize].zombie = None;
        self.release_vz(v);
    }

    /// `applyRealZombieUpdates`.
    fn apply_real_zombie_updates(&mut self, ups: &[RealUpdate]) {
        if ups.is_empty() {
            return;
        }
        for u in ups {
            if let Some(&v) = self.worker.real_zombies.get(&u.id) {
                let lead = self.leader(v);
                let z = &mut self.zombies[lead as usize];
                z.x = u.x;
                z.y = u.y;
                z.z = u.z;
                z.state = u.state;
                self.vz_update_current_cell(v);
            }
        }
        let n = self.worker.real_zombies.len() as i32 - ups.len() as i32;
        if n > 64 && self.worker.last_reported_unregistered < n {
            self.worker.last_reported_unregistered = n;
            self.write_log(1, format!("ZPOP: {n} zombie records are registered but no longer reported by the game"));
        }
    }

    /// `ChunkPos::apply`: a chunk was loaded into or removed from the world.
    fn chunk_pos_apply(&mut self, wx: i32, wy: i32, added: bool) {
        let Some(c) = self.cell_from_chunk(wx, wy) else { return };
        self.worker.cells[c].set_chunk_loaded(wx, wy, added);
        if !added {
            return;
        }
        self.load_if_needed(c);
        if let Some(i) = self.worker.cells[c].chunk_index_chunk(wx, wy) {
            let zs = std::mem::take(&mut self.worker.cells[c].chunks[i].zombies);
            let cell = &mut self.worker.cells[c];
            cell.stored = cell.stored.wrapping_sub(zs.len() as i16);
            cell.dirty = true;
            self.cur_ptm().add.extend_from_slice(&zs);
        }
    }

    /// `ManagerWorker::onConfigReloaded`.
    fn on_config_reloaded(&mut self, floats: &BTreeMap<Vec<u8>, f32>, ints: &BTreeMap<Vec<u8>, i32>) {
        let c = &mut self.worker.config;
        for (k, &v) in floats {
            match k.as_slice() {
                b"PopulationMultiplier" => c.population_multiplier = v,
                b"PopulationStartMultiplier" => c.population_start_multiplier = v,
                b"PopulationPeakMultiplier" => c.population_peak_multiplier = v,
                b"RespawnHours" => c.respawn_hours = v,
                b"RespawnUnseenHours" => c.respawn_unseen_hours = v,
                b"RespawnMultiplier" => c.respawn_multiplier = v,
                b"RedistributeHours" => c.redistribute_hours = v,
                b"MinZombiesPerChunk" => c.min_zombies_per_chunk = v,
                b"MaxZombiesPerChunk" => c.max_zombies_per_chunk = v,
                b"UniformZombiesPerChunk" => c.uniform_zombies_per_chunk = v,
                _ => {}
            }
        }
        for (k, &v) in ints {
            match k.as_slice() {
                b"PopulationPeakDay" => c.population_peak_day = v,
                b"FollowSoundDistance" => c.follow_sound_distance = v,
                _ => {}
            }
        }
    }

    // ---- repopulation ---------------------------------------------------------

    /// `adjacentOverpopulation`.
    fn adjacent_overpopulation(&mut self, c: CellIdx) -> i32 {
        let (cx, cy) = (self.worker.cells[c].x as i32, self.worker.cells[c].y as i32);
        let mut sum = 0i32;
        for dy in -1..=1 {
            for dx in -1..=1 {
                if dx == 0 && dy == 0 {
                    continue;
                }
                let Some(n) = self.cell_from_cell_pos(cx + dx, cy + dy) else { continue };
                if self.worker.cells[n].last_seen_ms == 0 {
                    self.worker.cells[n].last_seen_ms = self.gs.now();
                    self.cell_load(n);
                    self.cell_init_population(n);
                    self.cell_calc_population(n);
                    self.cell_unload(n);
                }
                let e = self.cell_effective_population(n);
                let t = self.cell_target_population(self.worker.cells[n].desired as i32);
                sum = sum.wrapping_add((e.wrapping_sub(t)).max(0));
            }
        }
        sum
    }

    /// `chooseSpawnOrigin`.
    fn choose_spawn_origin(&mut self) -> Option<(i32, i32)> {
        if self.worker.spawn_origins.is_empty() {
            let w = self.worker.width.wrapping_mul(256);
            let h = self.worker.height;
            for _ in 0..100 {
                let (x, y) = match self.rand.next(4) {
                    2 => (w - 1, self.rand.next(h.wrapping_mul(256))),
                    1 => (self.rand.next(w), 0),
                    0 => (0, self.rand.next(h.wrapping_mul(256))),
                    _ => {
                        let x = self.rand.next(w);
                        (x, h.wrapping_mul(256) - 1)
                    }
                };
                let x = x.wrapping_add(self.worker.min_x.wrapping_mul(256));
                let y = y.wrapping_add(self.worker.min_y.wrapping_mul(256));
                if self.can_spawn_at(x, y) {
                    return Some((x, y));
                }
            }
        } else {
            for _ in 0..100 {
                let k = self.rand.next(self.worker.spawn_origins.len() as i32) as usize;
                let o = self.worker.spawn_origins[k];
                let x = self.rand.next_range(o[0], o[2].wrapping_add(o[0]));
                let y = self.rand.next_range(o[1], o[3].wrapping_add(o[1]));
                if self.can_spawn_at(x, y) {
                    return Some((x, y));
                }
            }
        }
        None
    }

    /// `tryRepopChunk`.
    #[allow(clippy::neg_cmp_op_on_partial_ord)]
    fn try_repop_chunk(&mut self, c: CellIdx) {
        let cfg = self.worker.config;
        if self.gs.zombies_disabled || !(cfg.respawn_hours > 0.0) || self.worker.cells[c].pending_tasks > 0 {
            return;
        }
        let hours = self.gs.world_age_hours as f32;
        let t = self.cell_target_population(self.worker.cells[c].desired as i32);
        let e = self.cell_effective_population(c);
        let need0 = t.wrapping_sub(e).max(0);
        let need = need0.wrapping_sub(self.adjacent_overpopulation(c));
        if need <= 9 {
            let cell = &mut self.worker.cells[c];
            cell.repop_target = -1;
            cell.last_repop = hours;
            return;
        }
        {
            let cell = &mut self.worker.cells[c];
            if cell.last_repop > hours {
                cell.last_repop = hours;
            }
            if !(cell.last_repop + cfg.respawn_hours < hours) {
                cell.repop_target = -1;
                return;
            }
        }
        let mut list: Vec<usize> = Vec::new();
        for i in 0..1024 {
            let (wx, wy) = {
                let ch = &mut self.worker.cells[c].chunks[i];
                if ch.last_repop > hours {
                    ch.last_repop = hours;
                }
                (ch.wx as i32, ch.wy as i32)
            };
            let in_area = self.worker.loaded_areas.areas.iter().any(|a| {
                a[0] <= wx && wx * 8 < (a[2] + a[0]) * 8 && a[1] <= wy && wy * 8 < (a[3] + a[1]) * 8
            });
            if in_area {
                continue;
            }
            let ch = &self.worker.cells[c].chunks[i];
            if ch.last_seen + cfg.respawn_unseen_hours < hours
                && (ch.zombies.len() as i32) < self.chunk_target_population(ch.desired)
                && ch.last_repop + cfg.respawn_hours < hours
            {
                list.push(i);
            }
        }
        let per = if self.worker.cells[c].repop_target == -1 {
            let tp = self.cell_target_population(self.worker.cells[c].desired as i32);
            let per = ((cfg.respawn_multiplier * tp as f32) / 100.0) as i32;
            let cell = &mut self.worker.cells[c];
            cell.repop_target = (((hours - cell.last_repop) / cfg.respawn_hours) as i32).wrapping_mul(per);
            cell.repop_per = per;
            cell.repop_done = 0;
            per
        } else {
            self.worker.cells[c].repop_per
        };
        let per = (per / 36).max(10);
        let count = if need < per { need } else { per };
        while !list.is_empty() {
            let k = self.rand.next(list.len() as i32) as usize;
            let i = list[k];
            self.worker.cells[c].chunks[i].last_repop = hours;
            if let Some((ox, oy)) = self.choose_spawn_origin() {
                let (wx, wy) = (self.worker.cells[c].chunks[i].wx as i32, self.worker.cells[c].chunks[i].wy as i32);
                for _ in 0..20 {
                    let sx = self.rand.next(8) + wx * 8;
                    let sy = self.rand.next(8) + wy * 8;
                    if self.can_spawn_at(sx, sy) {
                        self.task_id = self.task_id.wrapping_add(1);
                        let id = self.task_id;
                        self.repop_tasks.insert(id, RepopTask { cell: c, count });
                        self.worker.cells[c].pending_tasks += 1;
                        self.mcd.find_path2(ox, oy, sx, sy, id);
                        return;
                    }
                }
            }
            if let Some(p) = list.iter().position(|&e| e == i) {
                list.remove(p);
            }
        }
        let cell = &mut self.worker.cells[c];
        cell.last_repop = hours;
        cell.repop_target = -1;
    }

    /// `RepopulateTask::finished(status, x, y)`.
    fn repop_finished(&mut self, task: u32, status: i32, x: i32, y: i32) {
        let Some(t) = self.repop_tasks.remove(&task) else { return };
        self.worker.cells[t.cell].pending_tasks -= 1;
        let hours = self.gs.world_age_hours;
        if status != wall_follower::REACHED {
            return;
        }
        let Some(c) = self.cell_from_world(x, y) else { return };
        if !self.worker.cells[t.cell].loaded {
            return;
        }
        let size = self.ff_repop.size;
        let half = (if size < 0 { size + 15 } else { size }) >> 4;
        let tx = (if x < 0 { x + 7 } else { x }) as u32 >> 3;
        let ty = (if y < 0 { y + 7 } else { y }) as u32 >> 3;
        let cell_x = self.worker.cells[c].x as i32 * 256;
        let cell_y = self.worker.cells[c].y as i32 * 256;
        let mut ox = (tx as i32).wrapping_sub(half).wrapping_mul(8).max(cell_x);
        let mut oy = (ty as i32).wrapping_sub(half).wrapping_mul(8).max(cell_y);
        ox = ox.min(cell_x - size + 256);
        oy = oy.min(cell_y - size + 256);
        let mut ff = std::mem::replace(&mut self.ff_repop, FloodFill::new(0));
        let areas = self.worker.loaded_areas.areas.clone();
        ff.calculate(
            self,
            x,
            y,
            ox,
            oy,
            |f, _| {
                let (fox, foy) = f.origin;
                let n = f.size;
                for a in &areas {
                    let x0 = (a[0] * 8).max(fox);
                    let x1 = ((a[2] + a[0]) * 8).min(fox + n);
                    let y0 = (a[1] * 8).max(foy);
                    let y1 = ((a[3] + a[1]) * 8).min(foy + n);
                    if y0 < y1 && x0 < x1 {
                        for yy in y0..y1 {
                            for xx in x0..x1 {
                                f.set_visited(xx, yy);
                            }
                        }
                    }
                }
            },
            |f, n: &mut Native, x1, y1, x2, y2| f.in_window_unvisited(x2, y2) && !n.blocked(x1, y1, x2, y2),
        );
        let visited = ff.visited.len() as i32;
        if visited == 0 {
            self.ff_repop = ff;
            return;
        }
        let fh = hours as f32;
        let ocx = (if ox < 0 { ox + 7 } else { ox }) >> 3;
        let ocy = (if oy < 0 { oy + 7 } else { oy }) >> 3;
        let span = (if size < 0 { size + 7 } else { size }) >> 3;
        if size > 7 {
            let (ccx, ccy) = (self.worker.cells[c].x as i32, self.worker.cells[c].y as i32);
            for wy in ocy..ocy + span {
                for wx in ocx..ocx + span {
                    let i = (wx - ccx * 32) + (wy - ccy * 32) * 32;
                    self.worker.cells[c].chunks[i as usize].last_repop = fh;
                }
            }
        }
        self.worker.cells[c].dirty = true;
        if self.gs.server {
            self.cur_ptm().repop_events.push((ocx, ocy, fh as i32));
        }
        let count = t.count.min(visited);
        for _ in 0..count {
            let k = self.rand.next(visited) as usize;
            let (vx, vy) = ff.visited[k];
            let sx = ox + vx as i32;
            let sy = oy + vy as i32;
            let ci = self.worker.cells[c].chunk_index_world(sx, sy);
            let z = self.alloc_zombie();
            self.zombie_init(z, sx as f32 + 0.5, sy as f32 + 0.5, 0.0, NO_PATH, NO_PATH);
            if let Some(i) = ci {
                self.worker.cells[c].chunks[i].zombies.push(z);
            }
            self.worker.cells[c].dirty = true;
        }
        let cell = &mut self.worker.cells[c];
        cell.stored = cell.stored.wrapping_add(count as i16);
        let done = cell.repop_done + count;
        cell.repop_done = done;
        if cell.repop_target <= done {
            cell.last_repop = fh;
        }
        self.ff_repop = ff;
    }

    // ---- hordes ------------------------------------------------------------------

    fn horde_spawn_at(&mut self, h: &mut Horde, x: f32, y: f32) -> bool {
        let z = self.alloc_zombie();
        self.zombie_init(z, x, y, 0.0, ffloor(h.tx), ffloor(h.ty));
        let v = self.alloc_vz();
        self.vz_init(v, z, None);
        self.worker.vzs.push(v);
        self.worker.vz_dirty = true;
        let c = h.count;
        h.count -= 1;
        c < 2
    }

    fn horde_spawn4(&mut self, h: &mut Horde, x: i32, y: i32) -> bool {
        if !self.ff_horde.is_visited(x, y) {
            return false;
        }
        let fx = x as f32 + 0.25;
        let fy = y as f32 + 0.25;
        if self.horde_spawn_at(h, fx, fy) {
            return true;
        }
        let gx = x as f32 + 0.75;
        if self.horde_spawn_at(h, gx, fy) {
            return true;
        }
        let gy = y as f32 + 0.75;
        if self.horde_spawn_at(h, fx, gy) {
            return true;
        }
        self.horde_spawn_at(h, gx, gy)
    }

    /// `Horde::spawn`.
    fn horde_spawn(&mut self, mut h: Horde) {
        if h.w != 0 || h.h != 0 {
            self.horde_spawn_in_area(&mut h);
            return;
        }
        if !self.can_spawn_at(h.x, h.y) {
            return;
        }
        let n = self.ff_horde.size;
        let cx = ((if h.x < 0 { h.x + 7 } else { h.x }) as u32 >> 3) as i32;
        let cy = ((if h.y < 0 { h.y + 7 } else { h.y }) as u32 >> 3) as i32;
        let half = (if n < 0 { n + 15 } else { n }) >> 4;
        let mut ox = cx.wrapping_sub(half).wrapping_mul(8);
        let mut oy = cy.wrapping_sub(half).wrapping_mul(8);
        let w = &self.worker;
        ox = ox.max(w.min_x.wrapping_mul(256));
        oy = oy.max(w.min_y.wrapping_mul(256));
        ox = ox.min((w.width + w.min_x).wrapping_mul(256) - n);
        oy = oy.min((w.height + w.min_y).wrapping_mul(256) - n);
        let mut ff = std::mem::replace(&mut self.ff_horde, FloodFill::new(0));
        ff.calculate(self, h.x, h.y, ox, oy, |_, _| {}, |f, _: &mut Native, _, _, x2, y2| f.in_window_unvisited(x2, y2));
        self.ff_horde = ff;
        let size = n;
        let (hx, hy) = (h.x, h.y);
        if self.horde_spawn4(&mut h, hx, hy) || self.ff_horde.size < 2 {
            return;
        }
        let xend = ox + size;
        let yend = oy + size;
        let mut r = 1;
        while r < self.ff_horde.size {
            let x0 = h.x - r;
            let x1 = h.x + r;
            let y0 = h.y - r;
            let y1 = h.y + r;
            if y0 >= oy {
                for xi in x0.max(ox)..xend.min(x1 + 1) {
                    if self.horde_spawn4(&mut h, xi, y0) {
                        return;
                    }
                }
            }
            if y1 < yend {
                for xi in x0.max(ox)..xend.min(x1 + 1) {
                    if self.horde_spawn4(&mut h, xi, y1) {
                        return;
                    }
                }
            }
            if x0 >= ox {
                for yi in (y0 + 1).max(oy)..yend.min(y1) {
                    if self.horde_spawn4(&mut h, x0, yi) {
                        return;
                    }
                }
            }
            if x1 < xend {
                for yi in (y0 + 1).max(oy)..yend.min(y1) {
                    if self.horde_spawn4(&mut h, x1, yi) {
                        return;
                    }
                }
            }
            r += 1;
        }
    }

    /// `Horde::spawnInArea`.
    fn horde_spawn_in_area(&mut self, h: &mut Horde) {
        if h.w < 1 || h.h < 1 || h.count < 1 {
            return;
        }
        let mut i = 0;
        while i < h.count {
            for _ in 0..100 {
                let x = self.rand.next(h.w) + h.x;
                let y = self.rand.next(h.h) + h.y;
                if self.can_spawn_at(x, y) {
                    let z = self.alloc_zombie();
                    self.zombie_init(z, x as f32 + 0.5, y as f32 + 0.5, 0.0, ffloor(h.tx), ffloor(h.ty));
                    let v = self.alloc_vz();
                    self.vz_init(v, z, None);
                    self.worker.vzs.push(v);
                    self.worker.vz_dirty = true;
                    break;
                }
            }
            i += 1;
        }
    }

    // ---- world sounds -------------------------------------------------------------

    /// Zombies on the ground floor of a chunk that may start walking.
    fn movable_zombies(&mut self, c: CellIdx, i: usize) -> Vec<ZId> {
        let zs = self.worker.cells[c].chunks[i].zombies.clone();
        let mut out = Vec::new();
        for z in zs {
            let r = self.zombies[z as usize];
            if ffloor(r.z) != 0 || r.state & 8 != 0 {
                continue;
            }
            if self.bits(ffloor(r.x), ffloor(r.y)) & BLOCK_MASK != 0 {
                continue;
            }
            out.push(z);
        }
        out
    }

    /// `applyWorldSound`: groups of zombies near a sound start walking to it.
    fn apply_world_sound(&mut self, sid: SoundId) {
        let s = self.sounds[&sid];
        let q = |v: i32| (if v < 0 { v + 7 } else { v }) >> 3;
        let x0 = q(s.x.wrapping_sub(s.radius));
        let y0 = q(s.y.wrapping_sub(s.radius));
        let x1 = q(s.radius.wrapping_add(s.x));
        let y1 = q(s.y.wrapping_add(s.radius));
        if !(y0 <= y1 && x0 <= x1) {
            return;
        }
        let r2 = s.radius.wrapping_mul(s.radius) as u32 as f32;
        for wy in y0..=y1 {
            let cyf = (wy * 8) as f32 + 4.0;
            for wx in x0..=x1 {
                let cxf = (wx * 8) as f32 + 4.0;
                let dx = (s.x as f32 + 0.5) - cxf;
                let dy = (s.y as f32 + 0.5) - cyf;
                let d = dx.mul_add(dx, dy * dy);
                if !(d <= r2 && d >= 100.0) {
                    continue;
                }
                let Some(c) = self.cell_from_chunk(wx, wy) else { continue };
                self.load_if_needed(c);
                let Some(ci) = self.worker.cells[c].chunk_index_chunk(wx, wy) else { continue };
                if self.worker.cells[c].chunks[ci].zombies.is_empty() {
                    continue;
                }
                let mut cand = self.movable_zombies(c, ci);
                let mut li = 0usize;
                while li < cand.len() {
                    let sx = s.x as f32;
                    let dist = ((sx + 0.5) - cxf).abs() + ((s.y as f32 + 0.5) - cyf).abs();
                    let r1 = self.rand.next_float(-dist, dist);
                    let r2r = self.rand.next_float(-dist, dist);
                    let lead = cand[li];
                    let v = self.alloc_vz();
                    let px = ffloor(r1 * 0.25 + sx);
                    let py = ffloor(r2r * 0.25 + s.y as f32);
                    self.vz_init(v, lead, Some((px, py)));
                    {
                        let vz = &mut self.vzs[v as usize];
                        vz.sound = Some(sid);
                        vz.sound_steps = 0;
                        vz.group.clear();
                        vz.group.push(lead);
                    }
                    let (lx, ly) = (self.zombies[lead as usize].x, self.zombies[lead as usize].y);
                    let (fx, fy) = (ffloor(lx), ffloor(ly));
                    let ox = (if fx < 0 { fx + 7 } else { fx }) & !7;
                    let oy = (if fy < 0 { fy + 7 } else { fy }) & !7;
                    let mut ff = std::mem::replace(&mut self.ff_sound, FloodFill::new(0));
                    self.flood_one_chunk(&mut ff, fx, fy, ox, oy);
                    li += 1;
                    let mut m = li;
                    while m < cand.len() {
                        let r = self.zombies[cand[m] as usize];
                        if ff.is_visited(ffloor(r.x), ffloor(r.y)) {
                            self.vzs[v as usize].group.push(cand[m]);
                            cand.remove(m);
                            if self.vzs[v as usize].group.len() > 25 {
                                break;
                            }
                        } else {
                            m += 1;
                        }
                    }
                    self.ff_sound = ff;
                    self.vz_update_current_cell(v);
                    self.worker.vzs.push(v);
                    self.worker.vz_dirty = true;
                    let g = self.vzs[v as usize].group.clone();
                    let cell = &mut self.worker.cells[c];
                    cell.stored = cell.stored.wrapping_sub(g.len().max(1) as i16);
                    remove_zombies(&mut cell.chunks[ci].zombies, &g);
                    cell.dirty = true;
                }
            }
        }
    }

    // ---- redistribution --------------------------------------------------------------

    /// `Redistribute::updateCell`.
    fn redistribute_cell(&mut self, c: CellIdx) {
        let mut empty = Vec::new();
        for i in 0..1024 {
            let (wx, wy, n) = {
                let ch = &self.worker.cells[c].chunks[i];
                (ch.wx as i32, ch.wy as i32, ch.zombies.len())
            };
            if n == 0 && !self.mcd.is_chunk_blocked(wx, wy, BLOCK_MASK, &self.gs, &self.meta) {
                empty.push(i);
            }
        }
        if empty.is_empty() {
            return;
        }
        for i in 0..1024 {
            self.redistribute_chunk(c, i, &empty);
        }
    }

    /// `Redistribute::updateChunk`: splits crowded chunks into groups
    /// walking to empty chunks.
    fn redistribute_chunk(&mut self, c: CellIdx, i: usize, empty: &[usize]) {
        if self.worker.cells[c].chunks[i].zombies.len() < 6 {
            return;
        }
        let mut cand = self.movable_zombies(c, i);
        if cand.len() < 6 {
            return;
        }
        while !cand.is_empty() {
            let lead = cand[0];
            let k = self.rand.next(empty.len() as i32) as usize;
            let dest = &self.worker.cells[c].chunks[empty[k]];
            let (dx, dy) = ((dest.wx as i32) << 3 | 4, (dest.wy as i32) << 3 | 4);
            let v = self.alloc_vz();
            self.vz_init(v, lead, Some((dx, dy)));
            self.vzs[v as usize].group.clear();
            self.vzs[v as usize].group.push(lead);
            let (lx, ly) = (self.zombies[lead as usize].x, self.zombies[lead as usize].y);
            let (fx, fy) = (ffloor(lx), ffloor(ly));
            let ox = (if fx < 0 { fx + 7 } else { fx }) & !7;
            let oy = (if fy < 0 { fy + 7 } else { fy }) & !7;
            let mut ff = std::mem::replace(&mut self.ff_redistribute, FloodFill::new(0));
            self.flood_one_chunk(&mut ff, fx, fy, ox, oy);
            if cand.len() >= 2 {
                let mut m = 1usize;
                while m < cand.len() {
                    let r = self.zombies[cand[m] as usize];
                    if ff.is_visited(ffloor(r.x), ffloor(r.y)) {
                        self.vzs[v as usize].group.push(cand[m]);
                        cand.remove(m);
                        if self.vzs[v as usize].group.len() > 4 {
                            break;
                        }
                    } else {
                        m += 1;
                    }
                }
                cand.remove(0);
            } else {
                cand.clear();
            }
            self.ff_redistribute = ff;
            self.vz_update_current_cell(v);
            self.worker.vzs.push(v);
            self.worker.vz_dirty = true;
            let g = self.vzs[v as usize].group.clone();
            let cell = &mut self.worker.cells[c];
            cell.stored = cell.stored.wrapping_sub(g.len().max(1) as i16);
            remove_zombies(&mut cell.chunks[i].zombies, &g);
            cell.dirty = true;
        }
    }

    // ---- debug commands --------------------------------------------------------------

    fn debug_cell(&self, cx: i32, cy: i32) -> Option<CellIdx> {
        self.cell_from_cell_pos(cx, cy).filter(|&c| self.worker.cells[c].loaded)
    }

    fn exec_debug(&mut self, cmd: DebugCmd) {
        match cmd {
            DebugCmd::SpawnTimeToZero(x, y) => {
                if let Some(c) = self.debug_cell(x, y) {
                    let cell = &mut self.worker.cells[c];
                    for ch in &mut cell.chunks {
                        ch.last_seen = 0.0;
                        ch.last_repop = 0.0;
                    }
                    cell.last_repop = 0.0;
                }
            }
            DebugCmd::ClearZombies(x, y) => {
                if let Some(c) = self.debug_cell(x, y) {
                    let mut all = Vec::new();
                    for ch in &mut self.worker.cells[c].chunks {
                        all.append(&mut ch.zombies);
                    }
                    for z in all {
                        self.release_zombie(z);
                    }
                    let cell = &mut self.worker.cells[c];
                    cell.dirty = true;
                    cell.stored = 0;
                }
            }
            DebugCmd::SpawnNow(x, y) => {
                if let Some(c) = self.debug_cell(x, y) {
                    let h = self.gs.world_age_hours as f32;
                    let cfg = self.worker.config;
                    let a = h - cfg.respawn_unseen_hours;
                    let b = h - cfg.respawn_hours;
                    let cell = &mut self.worker.cells[c];
                    for ch in &mut cell.chunks {
                        ch.last_seen = a;
                        ch.last_repop = b;
                    }
                    cell.last_repop = h - cfg.respawn_hours;
                }
            }
        }
    }

    // ---- the worker loop --------------------------------------------------------------

    /// `ManagerWorker::updateThread`.
    pub fn worker_update_thread(&mut self) {
        let mut b = PassToWorker::default();
        while let Some(p) = self.q_to_worker.pop_front() {
            let mut src = self.ptws[p].take().unwrap();
            b.combine(&mut src);
            let mut rec = std::mem::take(&mut src.recycled);
            self.worker.ptm_free.append(&mut rec);
            src.clear();
            self.ptws[p] = Some(src);
            self.cur_ptm().recycled.push(p);
        }
        if b.time_changed {
            self.gs.multiplier = b.multiplier;
            self.gs.world_age_hours = b.hours;
        }
        let hours_d = self.gs.world_age_hours;
        let now = self.gs.now();
        if b.config_changed {
            self.on_config_reloaded(&b.config_float, &b.config_int);
        }
        if b.loaded_areas.changed {
            self.worker.loaded_areas.copy_from(&b.loaded_areas);
        }
        if self.gs.server && b.server_cells.changed {
            self.worker.server_cells.copy_from(&b.server_cells);
        }
        for &t in &b.aggro {
            self.set_aggro_target(t);
        }
        let hours = hours_d as f32;
        let mut touched: Vec<CellIdx> = Vec::new();
        let areas = self.worker.loaded_areas.areas.clone();
        for a in &areas {
            let x0 = a[0];
            let y0 = a[1];
            let x1 = x0 + a[2] - 1;
            let y1 = y0 + a[3] - 1;
            let q = |v: i32| (if v < 0 { v + 31 } else { v }) >> 5;
            let cx0 = q(x0);
            let cx1 = q(x1);
            let cy0 = q(y0);
            let cy1 = q(y1);
            if cy0 > cy1 {
                continue;
            }
            let ycell = cy0 * 32;
            let cy_lo = y0.max(ycell);
            let cy_hi = (ycell + 32).min(y1);
            let xcell = cx0 * 32;
            let cx_lo = x0.max(xcell);
            let cx_hi = (xcell + 32).min(x1);
            for cy in cy0..=cy1 {
                if cx0 > cx1 {
                    break;
                }
                for cx in cx0..=cx1 {
                    let Some(c) = self.cell_from_cell_pos(cx, cy) else { continue };
                    self.worker.cells[c].last_seen_ms = self.gs.now();
                    if !self.worker.cells[c].loaded {
                        self.cell_load(c);
                        self.cell_init_population(c);
                        self.cell_calc_population(c);
                        self.worker.loaded_cells.push(c);
                    }
                    touched.push(c);
                    if cy_lo < cy_hi && cx_lo < cx_hi {
                        for wy in cy_lo..cy_hi {
                            for wx in cx_lo..cx_hi {
                                if let Some(i) = self.worker.cells[c].chunk_index_chunk(wx, wy) {
                                    if self.worker.cells[c].chunks[i].last_seen != hours {
                                        self.worker.cells[c].chunks[i].last_seen = hours;
                                        self.worker.cells[c].dirty = true;
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        for &(wx, wy, added) in &b.chunks {
            self.chunk_pos_apply(wx, wy, added);
        }
        for &z in &b.add {
            self.worker_add_zombie(z);
        }
        for &z in &b.register {
            self.worker_add_real_zombie(z);
        }
        for &id in &b.release {
            self.worker_release_real_zombie(id);
        }
        self.apply_real_zombie_updates(&b.real);
        if !self.worker.loaded_cells.is_empty() {
            let mut i = 0usize;
            while i < self.worker.loaded_cells.len() {
                let c = self.worker.loaded_cells[i];
                if touched.contains(&c) {
                    i += 1;
                    continue;
                }
                let cell = &self.worker.cells[c];
                if cell.pending_tasks != 0 || !cell.vzs.is_empty() || now <= cell.last_seen_ms + 2000 {
                    i += 1;
                    continue;
                }
                self.cell_unload(c);
                self.worker.loaded_cells.remove(i);
            }
            let mut i = 0usize;
            while i < self.worker.loaded_cells.len() {
                let c = self.worker.loaded_cells[i];
                self.try_repop_chunk(c);
                i += 1;
            }
        }
        for &h in &b.hordes {
            self.horde_spawn(h);
        }
        for &sid in &b.sounds {
            let s = self.sounds[&sid];
            let existing = self.worker.sounds.iter().copied().find(|e| {
                let e = &self.sounds[e];
                e.x == s.x && e.y == s.y && e.radius == s.radius && e.volume == s.volume
            });
            match existing {
                Some(e) => {
                    let e = self.sounds.get_mut(&e).unwrap();
                    e.created_ms = now;
                    e.hours = hours as i32;
                    self.free_sound(sid);
                }
                None => {
                    let s = self.sounds.get_mut(&sid).unwrap();
                    s.created_ms = now;
                    s.hours = hours as i32;
                    self.worker.sounds.push(sid);
                }
            }
        }
        let mut i = 0usize;
        while i < self.worker.sounds.len() {
            let sid = self.worker.sounds[i];
            let s = self.sounds[&sid];
            if s.created_ms + 1000 < now {
                self.free_sound(sid);
                self.worker.sounds.remove(i);
                continue;
            }
            if s.applied_ms + 5000 < now {
                self.apply_world_sound(sid);
                self.sounds.get_mut(&sid).unwrap().applied_ms = now;
            }
            i += 1;
        }
        if self.worker.aggro_targets.is_empty() {
            if !self.worker.sounds.is_empty() {
                let vzs = self.worker.vzs.clone();
                for v in vzs {
                    if self.worker.sounds.is_empty() {
                        continue;
                    }
                    self.assign_closest_sound(v);
                }
            }
        } else {
            let vzs = self.worker.vzs.clone();
            for v in vzs {
                let (x, y) = self.leader_pos(v);
                if let Some(k) = self.closest_aggro_target(ffloor(x), ffloor(y)) {
                    let (_, tx, ty) = self.worker.aggro_targets[k];
                    let lead = self.leader(v);
                    self.zombies[lead as usize].path_x = tx;
                    self.zombies[lead as usize].path_y = ty;
                }
            }
        }
        if self.worker.paused {
            for &d in &b.debug {
                self.exec_debug(d);
            }
            return;
        }
        let mut i = 0i32;
        while (i as usize) < self.worker.vzs.len() {
            let v = self.worker.vzs[i as usize];
            i = self.vz_update_thread(v, i) + 1;
        }
        for &d in &b.debug {
            self.exec_debug(d);
        }
        let rh = self.worker.config.redistribute_hours;
        if rh > 0.0 && !self.worker.loaded_cells.is_empty() {
            let h = self.gs.world_age_hours as f32;
            let mut k = 0usize;
            while k < self.worker.loaded_cells.len() {
                let c = self.worker.loaded_cells[k];
                let old = self.worker.cells[c].last_redistribute;
                let lr = if old <= h { old } else { h };
                self.worker.cells[c].last_redistribute = lr;
                if lr + rh < h {
                    self.redistribute_cell(c);
                    self.worker.cells[c].last_redistribute = h;
                    self.worker.cells[c].dirty = true;
                }
                k += 1;
            }
        }
        if self.worker.next_save_ms < now {
            self.worker.next_save_ms = now + 300_000;
            self.worker_save();
        }
        if b.radar_request {
            let mut pts = Vec::new();
            if self.worker.width >= 1 && self.worker.height >= 1 && !self.worker.cells.is_empty() && self.worker.cells[0].loaded {
                for ch in &self.worker.cells[0].chunks {
                    for &z in &ch.zombies {
                        let r = &self.zombies[z as usize];
                        pts.push((r.x, r.y));
                    }
                }
            }
            for &v in &self.worker.vzs {
                pts.push(self.leader_pos(v));
            }
            let p = self.cur_ptm();
            p.radar = pts;
            p.has_radar = true;
        }
        if b.debug_request {
            let mut info = MpDebugInfo::default();
            for &c in &self.worker.loaded_cells {
                let cell = &self.worker.cells[c];
                let e = self.cell_effective_population(c);
                let t = self.cell_target_population(cell.desired as i32);
                info.cells.push((cell.x, cell.y, e as i16, t as i16, cell.last_repop));
            }
            info.loaded_areas = self.worker.loaded_areas.clone();
            info.server_cells = self.worker.server_cells.clone();
            info.has_data = true;
            self.cur_ptm().debug = info;
        }
    }

    fn assign_closest_sound(&mut self, v: VzId) {
        let lead = self.leader(v);
        let z = self.zombies[lead as usize];
        let cx = ffloor(z.x) as f32 + 0.5;
        let cy = ffloor(z.y) as f32 + 0.5;
        let mut best = i32::MAX;
        let mut found = None;
        for &sid in &self.worker.sounds {
            let s = &self.sounds[&sid];
            let dx = (s.x as f32 + 0.5) - cx;
            let dy = (s.y as f32 + 0.5) - cy;
            let d = dx.mul_add(dx, dy * dy) as i32;
            if best > d && s.radius.wrapping_mul(s.radius) > d {
                best = d;
                found = Some(sid);
            }
        }
        let Some(sid) = found else { return };
        if self.vzs[v as usize].sound == Some(sid) {
            return;
        }
        let s = self.sounds[&sid];
        let sxf = s.x as f32;
        let dist = ((sxf + 0.5) - z.x).abs() + ((s.y as f32 + 0.5) - z.y).abs();
        let r1 = self.rand.next_float(-dist, dist);
        let px = ffloor(r1 * 0.25 + sxf);
        self.zombies[lead as usize].path_x = px;
        let r2 = self.rand.next_float(-dist, dist);
        let py = ffloor(r2 * 0.25 + s.y as f32);
        self.zombies[lead as usize].path_y = py;
        let vz = &mut self.vzs[v as usize];
        vz.sound = Some(sid);
        vz.sound_steps = 0;
    }

    /// `ManagerWorker::save`.
    pub fn worker_save(&mut self) {
        // Zombies on their way to the main thread, and those queued for the
        // worker, are saved where they are.
        let mut pending: Vec<ZId> = self.ptms[self.worker.cur_ptm].as_ref().unwrap().add.clone();
        for &p in &self.q_to_main {
            pending.extend_from_slice(&self.ptms[p].as_ref().unwrap().add);
        }
        let mut register: Vec<ZId> = Vec::new();
        let mut released: HashSet<u32> = HashSet::new();
        let queued: Vec<PtwId> = self.q_to_worker.iter().copied().chain(std::iter::once(self.main.cur_ptw)).collect();
        for p in queued {
            let (add, reg, rel, real) = {
                let w = self.ptws[p].as_ref().unwrap();
                (w.add.clone(), w.register.clone(), w.release.clone(), w.real.clone())
            };
            pending.extend_from_slice(&add);
            register.extend_from_slice(&reg);
            released.extend(rel.iter().copied());
            self.apply_real_zombie_updates(&real);
        }
        let mut rel: Vec<u32> = released.iter().copied().collect();
        rel.sort_unstable();
        for id in rel {
            self.worker_release_real_zombie(id);
        }
        for z in register {
            if !released.contains(&self.zombies[z as usize].id) {
                pending.push(z);
            }
        }
        for z in pending {
            let r = self.zombies[z as usize];
            if self.worker.real_zombies.contains_key(&r.id) {
                continue;
            }
            if let Some(c) = self.cell_from_world(ffloor(r.x), ffloor(r.y)) {
                self.load_if_needed(c);
                self.worker.cells[c].save_pending.push(z);
                self.worker.cells[c].dirty = true;
            }
        }
        let reals: Vec<VzId> = self.worker.real_zombies.values().copied().collect();
        for v in reals {
            if let Some(c) = self.vzs[v as usize].cell {
                self.load_if_needed(c);
                self.worker.cells[c].dirty = true;
            }
        }
        let ok = if self.id_file_save_if_advanced() { self.virtual_file_save() } else { false };
        let loaded = self.worker.loaded_cells.clone();
        for &c in &loaded {
            if self.worker.cells[c].dirty {
                self.worker.cells[c].dirty = false;
                self.cell_save(c, ok);
            }
        }
        for &c in &loaded {
            self.worker.cells[c].save_pending.clear();
        }
    }

    /// `ManagerWorker::saveCell(cx, cy)`.
    pub fn worker_save_cell(&mut self, cx: i32, cy: i32) {
        if let Some(c) = self.cell_from_cell_pos(cx, cy) {
            if self.worker.cells[c].loaded && self.worker.cells[c].dirty {
                self.worker.cells[c].dirty = false;
                self.cell_save(c, false);
            }
        }
    }

    /// `ManagerWorker::shouldWait`.
    pub fn worker_should_wait(&mut self) -> bool {
        let cfg = self.worker.config;
        if !self.gs.zombies_disabled && cfg.respawn_hours > 0.0 && !self.worker.loaded_cells.is_empty() {
            let hours = self.gs.world_age_hours as i32 as f32;
            let loaded = self.worker.loaded_cells.clone();
            for c in loaded {
                let t = self.cell_target_population(self.worker.cells[c].desired as i32);
                let e = self.cell_effective_population(c);
                let a = self.adjacent_overpopulation(c);
                if t.wrapping_sub(e).max(0).wrapping_sub(a) > 9 && self.worker.cells[c].last_repop + cfg.respawn_hours < hours {
                    return false;
                }
            }
        }
        if !self.worker.vzs.is_empty() {
            return false;
        }
        self.q_to_worker.is_empty()
    }

    /// `MapCollisionData::update` (the worker's collision half): applies
    /// queued updates, advances path tasks and unloads idle cells.
    pub fn mcd_update(&mut self) {
        while let Some(b) = self.mcd.queue.pop_front() {
            self.mcd.apply_batch(b, &self.gs, &self.meta);
        }
        if self.mcd.current.is_none() {
            if let Some(t) = self.mcd.path_tasks.pop() {
                self.mcd.wall_follower.init_path(t.sx, t.sy, t.ex, t.ey);
                self.mcd.current = Some(t);
            }
        }
        if let Some(t) = self.mcd.current {
            let start = self.gs.now();
            let mut wf = std::mem::take(&mut self.mcd.wall_follower);
            let mut r = wf.update(&mut self.mcd, &self.gs, &self.meta);
            let mut n = 0;
            while r == wall_follower::CONTINUE {
                n += 1;
                if n == 100 {
                    if self.gs.now() - start > 50 {
                        self.mcd.wall_follower = wf;
                        return;
                    }
                    n = 0;
                }
                r = wf.update(&mut self.mcd, &self.gs, &self.meta);
            }
            let (cx, cy) = wf.cur;
            self.mcd.wall_follower = wf;
            self.mcd.current = None;
            self.repop_finished(t.task, r, cx, cy);
        }
        self.mcd.unload_idle(&self.gs);
    }
}

fn remove_zombies(list: &mut Vec<ZId>, gone: &[ZId]) {
    for g in gone {
        if let Some(i) = list.iter().position(|z| z == g) {
            list.remove(i);
        }
    }
}
