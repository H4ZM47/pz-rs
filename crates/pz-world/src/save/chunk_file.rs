//! The chunk file: `IsoChunk.Save(ByteBuffer, CRC32, boolean)`,
//! `IsoChunk.LoadFromDiskOrBufferInternal`, `IsoGridSquare.save`/`load`,
//! the `map/<wx>/<wy>.bin` naming (`ChunkMapFilenames.getFilename`) and the
//! erosion step of `IsoChunk.doLoadGridsquare`.
//!
//! Layout: `debug (byte)`, `worldVersion (int)`, `length (int)`,
//! `CRC32 (long)` of bytes `17..length`, then the chunk fields, the
//! squares (`x` outer, `y`, then levels, each `(x, y)` column preceded by a
//! long whose bit `z + 32` marks a saved level), erosion, generators, an
//! empty vehicle list (vehicles live in `vehicles.db`), the loot respawn
//! hour and the spawned rooms. There is no compression.

use std::path::{Path, PathBuf};

use super::io::{crc32, err, has, BitHeaderWrite, Reader, SaveResult, Writer};
use super::lua_table::LuaTable;
use super::object_io::{load_object, save_object};
use super::visual::IsoDeadBody;
use super::{SaveCtx, WORLD_VERSION};
use crate::chunk::{FloorBloodSplat, IsoChunk, IsoGridSquare};
use crate::erosion_data::{ErosionChunkData, ErosionSquareData};
use crate::java::fastfloor;
use crate::objects::ObjectKind;

/// `ChunkMapFilenames.getFilename(wx, wy)` under the save directory.
pub fn chunk_path(save_dir: &Path, wx: i32, wy: i32) -> PathBuf {
    save_dir
        .join("map")
        .join(wx.to_string())
        .join(format!("{wy}.bin"))
}

/// `IsoChunk.validateByteBufferHeader`.
pub fn validate_header(bb: &[u8]) -> bool {
    if bb.len() < 17 {
        return false;
    }
    let len = i32::from_be_bytes(bb[5..9].try_into().unwrap());
    if len as usize != bb.len() {
        return false;
    }
    let crc = i64::from_be_bytes(bb[9..17].try_into().unwrap());
    crc == crc32(&bb[17..]) as i64
}

/// Result of loading a chunk file.
pub struct LoadedChunk {
    pub chunk: IsoChunk,
    pub world_version: i32,
    /// The saved CRC matched (`sanityCheck.checkCRC` only logs a mismatch).
    pub crc_ok: bool,
}

fn read_flags(r: &mut Reader<'_>, out: &mut [bool]) -> SaveResult<()> {
    let v = r.i8()? as i32;
    for (i, f) in out.iter_mut().enumerate() {
        let pow = 1 << i;
        *f = v & pow == pow;
    }
    Ok(())
}

fn write_flags(w: &mut Writer, flags: &[bool]) {
    let mut b = 0i32;
    for (i, &f) in flags.iter().enumerate() {
        b += (f as i32) << i;
    }
    w.i8(b as i8);
}

/// `IsoFloorBloodSplat.load`.
fn load_splat(r: &mut Reader<'_>) -> SaveResult<FloorBloodSplat> {
    let q = |b: i8| fastfloor((b as u8) as f32) as f32 / 255.0 * 8.0;
    Ok(FloorBloodSplat {
        x: q(r.i8()?),
        y: q(r.i8()?),
        z: q(r.i8()?),
        kind: r.i8()?,
        world_age: r.f32()?,
        index: r.i8()?,
    })
}

/// `IsoFloorBloodSplat.save`.
fn save_splat(w: &mut Writer, s: &FloorBloodSplat) {
    for v in [s.x, s.y, s.z] {
        let q = fastfloor(v / 8.0 * 255.0).clamp(0, 255);
        w.i8(q as u8 as i8);
    }
    w.i8(s.kind);
    w.f32(s.world_age);
    w.i8(s.index);
}

/// `IsoChunk.LoadFromDiskOrBuffer` for the chunk at `(wx, wy)`.
pub fn load_chunk(bb: &[u8], wx: i32, wy: i32, ctx: &SaveCtx<'_>) -> SaveResult<LoadedChunk> {
    let mut r = Reader::new(bb);
    let debug = r.bool()?;
    if debug {
        return err("debug chunk saves are not supported");
    }
    let wv = r.i32()?;
    if wv > WORLD_VERSION {
        return err(format!(
            "unknown world version {wv} while reading chunk {wx},{wy}"
        ));
    }
    let _len = r.i32()?;
    let crc = r.i64()?;
    let crc_ok = bb.len() >= 17 && crc == crc32(&bb[17..]) as i64;
    let mut ch = IsoChunk::new(wx, wy);
    // The lot header is known when its file exists; the caller decides by
    // passing a meta grid.
    if ctx.meta.is_some() {
        ch.lotheader = Some(crate::map_files::header_name(
            (wx * 8).div_euclid(256),
            (wy * 8).div_euclid(256),
        ));
    }
    let s = &mut ch.saved;
    if wv >= 209 {
        s.blending_done_full = r.bool()?;
    }
    if wv >= 210 {
        read_flags(&mut r, &mut s.blending_modified)?;
        s.blending_done_partial = r.bool()?;
        if s.blending_modified != [true; 4] && s.blending_done_partial {
            for d in s.blending_depth.iter_mut() {
                *d = r.i8()?;
            }
        }
    }
    if wv >= 214 {
        s.attachments_done_full = r.bool()?;
        read_flags(&mut r, &mut s.attachments_state)?;
    }
    if wv >= 221 {
        let n = r.i16()?;
        if n == 0 {
            s.attachments_partial = None;
        } else {
            let mut v = Vec::new();
            for _ in 0..n.max(0) {
                v.push((r.i32()?, r.i32()?, r.i32()?));
            }
            s.attachments_partial = Some(v);
        }
    }
    let (max_level, min_level) = if wv >= 206 {
        (r.i32()?, r.i32()?)
    } else {
        (7, 0)
    };
    if !(crate::chunk::MIN_LEVEL..=crate::chunk::MAX_LEVEL).contains(&min_level)
        || !(min_level..=crate::chunk::MAX_LEVEL).contains(&max_level)
    {
        return err(format!("bad chunk levels {min_level}..{max_level}"));
    }
    let lifespan = ctx.settings.blood_splat_lifespan_days;
    let wah = ctx.settings.world_age_hours as f32;
    let n = r.i32()?;
    for _ in 0..n.max(0) {
        let mut sp = load_splat(&mut r)?;
        if sp.world_age >= wah {
            sp.world_age = wah;
        }
        if lifespan > 0 && wah - sp.world_age >= (lifespan * 24) as f32 {
            continue;
        }
        let s = &mut ch.saved;
        if sp.kind < 8 {
            s.next_splat_index = sp.index as i32 % 10;
        }
        if s.floor_blood_splats.len() == 1000 {
            s.floor_blood_splats.remove(0);
        }
        s.floor_blood_splats.push(sp);
    }
    ch.set_min_max_level(min_level, max_level);
    for x in 0..8 {
        for y in 0..8 {
            let flags = if wv >= 206 {
                r.i64()?
            } else {
                (r.i8()? as i64) << 32
            };
            for z in min_level..=max_level {
                if flags & (1i64 << (z + 32)) == 0 {
                    continue;
                }
                let mut sq = IsoGridSquare::new(x + wx * 8, y + wy * 8, z);
                if let Some(meta) = ctx.meta {
                    sq.room_id = meta.get_room_at(sq.x, sq.y, sq.z).map_or(-1, |d| d.id);
                    sq.roof_hide_building = meta
                        .get_empty_outside_at(sq.x, sq.y, sq.z)
                        .and_then(|d| d.building);
                }
                sq.reset_iso_world_region();
                load_square(&mut r, ctx, wv, &mut sq)?;
                sq.fix_stackable_objects(ctx.sprites);
                ch.set_square(x, y, z, Some(sq));
            }
        }
    }
    ch.erosion = Some(ErosionChunkData::load(&mut r)?);
    let n = r.i16()?;
    if n > 0 && ch.saved.generators.is_none() {
        ch.saved.generators = Some(Vec::new());
    }
    for _ in 0..n.max(0) {
        let g = (r.i32()?, r.i32()?, r.i8()?);
        ch.saved.generators.as_mut().unwrap().push(g);
    }
    let vehicles = r.i16()?;
    if vehicles > 0 {
        return err("chunk files with vehicles are not supported");
    }
    ch.saved.loot_respawn_hour = r.i32()?;
    if wv >= 206 {
        let n = r.i16()?;
        for _ in 0..n.max(0) {
            let id = r.i64()?;
            if !ch.saved.spawned_rooms.contains(&id) {
                ch.saved.spawned_rooms.push(id);
            }
        }
    } else {
        return err("spawned rooms from world version < 206 need RoomID.makeID");
    }
    if ch.get_grid_square(0, 0, 0).is_none() && ch.get_grid_square(7, 7, 0).is_none() {
        return err(format!("black chunk {wx},{wy}"));
    }
    Ok(LoadedChunk {
        chunk: ch,
        world_version: wv,
        crc_ok,
    })
}

/// Whether `IsoGridSquare.load` keeps a loaded world item (the removal
/// check, with obsolete and unknown items dropped).
fn keep_world_item(ctx: &SaveCtx<'_>, wi: &crate::objects::IsoWorldInventoryObject) -> bool {
    let reg = wi.item.registry_id();
    if !ctx.items.exists(reg) {
        return false;
    }
    let Some(ty) = ctx.items.full_type(reg) else {
        return true;
    };
    if ctx.items.obsolete(ty) {
        return false;
    }
    let st = ctx.settings;
    let bl = st.item_removal_list_blacklist;
    let head = ty.split('_').next().unwrap_or(ty);
    let listed = (wi.drop_time > -1.0
        && st.hours_for_world_item_removal > 0.0
        && (!bl && st.removal_list_contains(ty) || bl && !st.removal_list_contains(ty)))
        || !bl && st.removal_list_contains(head)
        || bl && !st.removal_list_contains(head);
    !(listed
        && !wi.ignore_remove_sandbox
        && ctx.settings.world_age_hours > wi.drop_time + st.hours_for_world_item_removal)
}

/// `IsoGridSquare.load`.
pub fn load_square(
    r: &mut Reader<'_>,
    ctx: &SaveCtx<'_>,
    wv: i32,
    sq: &mut IsoGridSquare,
) -> SaveResult<()> {
    sq.erosion = Some(ErosionSquareData::load(r)?);
    let header = r.i8()? as i64 as u64;
    if header != 0 {
        if has(header, 1) {
            let objs = if has(header, 2) {
                2
            } else if has(header, 4) {
                3
            } else if has(header, 8) {
                r.i16()? as i32
            } else {
                1
            };
            for _ in 0..objs.max(0) {
                let flags = r.i8()?;
                let special = flags & 2 != 0;
                let world = flags & 4 != 0;
                let id = sq.new_object_id();
                let Some(obj) = load_object(r, ctx, wv, id)? else {
                    continue;
                };
                if let ObjectKind::WorldInventory(wi) = &obj.kind {
                    if !keep_world_item(ctx, wi) {
                        continue;
                    }
                }
                if let ObjectKind::Window(_) = obj.kind {
                    let name = obj
                        .sprite
                        .and_then(|s| ctx.sprites.sprites[s].name.as_deref());
                    if matches!(name, Some("walls_special_01_8" | "walls_special_01_9")) {
                        continue;
                    }
                }
                if world && !matches!(obj.kind, ObjectKind::WorldInventory(_)) {
                    return err("ClassCastException: world object flag on a non-item");
                }
                sq.objects.push(obj);
                if special {
                    sq.special_objects.push(id);
                }
                if world {
                    sq.world_objects.push(id);
                }
            }
        }
        sq.overlay_done = has(header, 16);
        sq.have_roof = has(header, 32);
        if has(header, 64) {
            let bits = super::io::read_header(r, 1)?;
            if has(bits, 1) {
                let n = r.i16()?;
                for _ in 0..n.max(0) {
                    if !r.bool()? {
                        continue;
                    }
                    let class = r.i8()?;
                    if ctx.tables.class_name(class) != Some("DeadBody") {
                        return err(format!(
                            "static moving object of class {:?} is not supported",
                            ctx.tables.class_name(class)
                        ));
                    }
                    // IsoObject field initialisers: offsetX = 32 * tileScale,
                    // offsetY = 96 * tileScale.
                    let ts = ctx.settings.tile_scale as f32;
                    sq.static_moving.push(IsoDeadBody::load(
                        r,
                        wv,
                        ctx.tables,
                        (32.0 * ts, 96.0 * ts),
                        ctx.items,
                    )?);
                }
            }
            if has(bits, 2) {
                sq.mod_data = Some(LuaTable::load(r, wv)?);
            }
            sq.burnt_out = has(bits, 4);
            if has(bits, 8) {
                // Read into locals and dropped, as upstream.
                let _ = (r.i32()?, r.i32()?, r.i32()?);
            }
            sq.have_sheet_rope = has(bits, 16);
        }
    }
    let vis = r.i8()?;
    for (i, s) in sq.seen.iter_mut().enumerate() {
        *s = vis & (1 << i) != 0;
    }
    // IsoLightSwitch.load: a room light takes the room's state.
    if let Some(meta) = ctx.meta {
        let room = sq.room_def(meta);
        for o in &mut sq.objects {
            if let (ObjectKind::LightSwitch(s), Some(room)) = (&mut o.kind, room) {
                if s.light_room {
                    s.activated = room.lights_active.get();
                }
            }
        }
    }
    Ok(())
}

/// `IsoGridSquare.shouldSave()`.
pub fn should_save(sq: &IsoGridSquare) -> bool {
    !sq.objects.is_empty() || sq.z == 0
}

/// `IsoGridSquare.save`.
pub fn save_square(w: &mut Writer, ctx: &SaveCtx<'_>, sq: &IsoGridSquare) -> SaveResult<()> {
    save_erosion_square(w, sq.erosion.as_ref());
    let mut header = BitHeaderWrite::alloc(w, 1);
    let n = sq.objects.len();
    if n > 0 {
        header.add(1);
        match n {
            2 => header.add(2),
            3 => header.add(4),
            1 => {}
            _ => header.add(8),
        }
        if n >= 4 {
            w.i16(n as i16);
        }
        for o in &sq.objects {
            let mut f = 0i8;
            if sq.special_objects.contains(&o.id) {
                f |= 2;
            }
            if sq.world_objects.contains(&o.id) {
                f |= 4;
            }
            w.i8(f);
            save_object(w, ctx, o)?;
        }
    }
    if sq.overlay_done {
        header.add(16);
    }
    if sq.have_roof {
        header.add(32);
    }
    let mut bits = BitHeaderWrite::alloc(w, 1);
    if !sq.static_moving.is_empty() {
        bits.add(1);
        w.i16(sq.static_moving.len() as i16);
        let class = ctx.class_id("DeadBody")?;
        for b in &sq.static_moving {
            b.save(w, class)?;
        }
    }
    if let Some(t) = sq.mod_data.as_ref().filter(|t| !t.is_empty()) {
        bits.add(2);
        t.save(w);
    }
    if sq.burnt_out {
        bits.add(4);
    }
    if let Some((x, y, z)) = sq.trap_position.filter(|p| p.0 > 0) {
        bits.add(8);
        w.i32(x);
        w.i32(y);
        w.i32(z);
    }
    if sq.have_sheet_rope {
        bits.add(16);
    }
    if bits.flags != 0 {
        header.add(64);
        bits.write(w);
    } else {
        bits.discard(w);
    }
    let mut vis = 0i8;
    for (i, &s) in sq.seen.iter().enumerate() {
        if s {
            vis |= 1 << i;
        }
    }
    w.i8(vis);
    header.write(w);
    Ok(())
}

/// `IsoChunk.Save(bb, crc, bHotSave)`: the whole file.
pub fn save_chunk(ch: &IsoChunk, ctx: &SaveCtx<'_>) -> SaveResult<Vec<u8>> {
    let mut w = Writer::new();
    let s = &ch.saved;
    w.i8(0);
    w.i32(WORLD_VERSION);
    w.i32(0);
    w.i64(-1);
    w.bool(s.blending_done_full);
    write_flags(&mut w, &s.blending_modified);
    w.bool(s.blending_done_partial);
    if s.blending_modified != [true; 4] && s.blending_done_partial {
        for &d in &s.blending_depth {
            w.i8(d);
        }
    }
    w.bool(s.attachments_done_full);
    write_flags(&mut w, &s.attachments_state);
    match &s.attachments_partial {
        None => w.i16(0),
        Some(v) => {
            w.i16(v.len() as i16);
            for &(x, y, z) in v {
                w.i32(x);
                w.i32(y);
                w.i32(z);
            }
        }
    }
    let count = s.floor_blood_splats.len().min(1000);
    let start = s.floor_blood_splats.len() - count;
    let pos_levels = w.pos();
    w.i32(ch.max_level);
    w.i32(ch.min_level);
    w.i32(count as i32);
    for sp in &s.floor_blood_splats[start..] {
        save_splat(&mut w, sp);
    }
    let (mut min1, mut max1) = (i32::MAX, i32::MIN);
    for x in 0..8 {
        for y in 0..8 {
            let flag_pos = w.pos();
            w.i64(0);
            let mut flags = 0i64;
            for z in ch.min_level..=ch.max_level {
                let Some(gs) = ch.get_grid_square(x, y, z) else {
                    continue;
                };
                if !should_save(gs) {
                    continue;
                }
                flags |= 1i64 << (z + 32);
                min1 = min1.min(z);
                max1 = max1.max(z);
                save_square(&mut w, ctx, gs)?;
            }
            w.patch_i64(flag_pos, flags);
        }
    }
    if min1 <= max1 {
        w.patch_i32(pos_levels, max1);
        w.patch_i32(pos_levels + 4, min1);
    }
    match &ch.erosion {
        Some(e) => e.save(&mut w),
        None => w.i8(0),
    }
    match &s.generators {
        None => w.i16(0),
        Some(g) => {
            w.i16(g.len() as i16);
            for &(x, y, z) in g {
                w.i32(x);
                w.i32(y);
                w.i8(z);
            }
        }
    }
    w.i16(0);
    w.i32(s.loot_respawn_hour);
    let rooms = s.spawned_rooms.len().min(i16::MAX as usize);
    w.i16(rooms as i16);
    for &id in &s.spawned_rooms {
        w.i64(id);
    }
    let len = w.pos();
    let crc = crc32(&w.buf[17..len]);
    w.patch_i32(5, len as i32);
    w.patch_i64(9, crc as i64);
    Ok(w.buf)
}

fn save_erosion_square(w: &mut Writer, e: Option<&ErosionSquareData>) {
    match e {
        Some(e) => e.save(w),
        None => w.i8(0),
    }
}

/// What a [`ChunkLoadHook`] may use besides the chunk itself.
pub struct HookCtx<'a> {
    /// `IsoSpriteManager.instance`: hooks may add sprites the way
    /// `getSprite`/`AddSprite` does.
    pub sprites: &'a mut crate::sprite::IsoSpriteManager,
    /// The rest of the cell (every loaded chunk except the one being
    /// loaded), for neighbour lookups across the chunk edge.
    pub neighbours: &'a dyn crate::square::GetSquare,
}

/// Calls the original makes into the erosion system once a chunk is loaded
/// (`IsoChunk.doLoadGridsquare`, after `LoadFromDisk`). Implemented by
/// `pz-erosion`.
pub trait ChunkLoadHook {
    /// `ErosionMain.LoadGridsquare(square)` for the square at chunk-local
    /// `(x, y)` on level `z`. The original calls it for every square that
    /// has objects, levels `minLevel..=maxLevel` outer, then `x`, then `y`
    /// (not on a soft reset). The square's `erosion` holds the loaded
    /// `ErosionData.Square`.
    fn load_grid_square(
        &mut self,
        chunk: &mut IsoChunk,
        x: i32,
        y: i32,
        z: i32,
        ctx: &mut HookCtx<'_>,
    );
    /// `ErosionMain.ChunkLoaded(chunk)`, once after all squares. The
    /// chunk's `erosion` holds the loaded `ErosionData.Chunk`.
    fn chunk_loaded(&mut self, chunk: &mut IsoChunk, ctx: &mut HookCtx<'_>);
    /// Called when chunk `(wx, wy)` leaves the cell. The original has no
    /// such call: per-square erosion state lives on `IsoGridSquare` objects
    /// and goes away with them. Hooks that keep that state on the side drop
    /// it here.
    fn chunk_unloaded(&mut self, _wx: i32, _wy: i32) {}
}

/// The steps after a chunk read from disk is placed in `cell`, in the
/// original's order: `loadInWorldStreamerThread`/`loadInMainThread`
/// (missing squares, `RecalcProperties`, roofs, neighbour matrices) and then
/// the erosion calls of `doLoadGridsquare`, so hooks see recalculated flags.
/// While the hook runs the chunk is taken out of `cell`, which is passed to
/// the hook as the neighbour view, and put back afterwards.
pub fn finish_chunk_load(
    cell: &mut crate::chunk::IsoCell,
    wx: i32,
    wy: i32,
    meta: &crate::meta_grid::IsoMetaGrid,
    sprites: &mut crate::sprite::IsoSpriteManager,
    hook: Option<&mut dyn ChunkLoadHook>,
) {
    cell.recalc_chunk_squares(wx, wy, meta, sprites);
    let Some(mut ch) = cell.chunks.remove(&(wx, wy)) else {
        return;
    };
    let mut ctx = HookCtx {
        sprites,
        neighbours: &*cell,
    };
    do_load_gridsquare(&mut ch, hook, &mut ctx);
    cell.chunks.insert((wx, wy), ch);
}

/// The erosion part of `IsoChunk.doLoadGridsquare`: runs `hook` where the
/// original calls `ErosionMain`. The other per-square work there
/// (`addToWorld`, `MapObjects`, Lua events) is not ported.
pub fn do_load_gridsquare(
    ch: &mut IsoChunk,
    hook: Option<&mut dyn ChunkLoadHook>,
    ctx: &mut HookCtx<'_>,
) {
    let Some(hook) = hook else {
        return;
    };
    for z in ch.min_level..=ch.max_level {
        for x in 0..8 {
            for y in 0..8 {
                if ch
                    .get_grid_square(x, y, z)
                    .is_some_and(|sq| !sq.objects.is_empty())
                {
                    hook.load_grid_square(ch, x, y, z, ctx);
                }
            }
        }
    }
    hook.chunk_loaded(ch, ctx);
}
