//! `IsoChunk.updatePhysicsForLevel(level)` and `calcPhysics`: the
//! `ToBullet` buffer of a chunk level's static collision shapes
//! (`Bullet.beginUpdateChunk` / `updateChunk` / `endUpdateChunk`), built
//! from pz-world's squares.
//!
//! The buffer is `8, wx, wy, minLevel, maxLevel, level` (shorts, little
//! endian), then for each square `x, y, n` and `n` shape bytes
//! (`PhysicsShapes` ordinal + 1), then `-1, -1`; a level with no shapes
//! sends nothing.

use pz_world::objects::ObjectKind;
use pz_world::{IsoChunk, IsoGridSquare, IsoSpriteManager};

/// `IsoChunk.PhysicsShapes`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum PhysicsShape {
    Solid,
    WallN,
    WallW,
    WallS,
    WallE,
    Tree,
    Floor,
    StairsMiddleNorth,
    StairsMiddleWest,
    SolidStairs,
    /// `FIRST_MESH`: physics meshes follow, by `physicsShapeNameToIndex`.
    FirstMesh,
}

/// `addPhysicsShape(sq, shapes, count, shape)`: up to four distinct
/// shapes per square.
fn add(shapes: &mut [i32; 4], count: usize, shape: i32) -> usize {
    if shapes[..count].contains(&shape) {
        return count;
    }
    if count == shapes.len() {
        return count;
    }
    shapes[count] = shape;
    count + 1
}

/// `calcPhysics(x, y, z, shapes)` for one square. `mesh_index` is
/// `Bullet.physicsShapeNameToIndex` (full `Module.Name` keys).
pub fn calc_physics(
    sq: Option<&IsoGridSquare>,
    z: i32,
    m: &IsoSpriteManager,
    mesh_index: &dyn Fn(&str) -> Option<i32>,
) -> [i32; 4] {
    use PhysicsShape as S;
    let mut shapes = [-1i32; 4];
    let Some(sq) = sq else { return shapes };
    let f = &m.tables.flag;
    let t = &m.tables.ty;
    let flag = |name: &str| m.tables.flag_from_string(name);
    let props = &sq.properties;
    let has_flag = |name: &str| flag(name).is_some_and(|fl| props.has(fl));
    let has_str = |name: &str| props.has_name(&m.alias, name);
    let get_str = |name: &str| props.get(&m.alias, name);
    let mut n = 0usize;
    if z == 0 {
        let mut column = false;
        for o in &sq.objects {
            let spr = o.sprite.map(|i| &m.sprites[i]);
            let has_shape = spr.is_some_and(|s| m.has_prop(s, "PhysicsShape"));
            let Some(s) = spr else { continue };
            if has_shape {
                continue;
            }
            // `o.sprite.name == null` counts as not a lamp.
            let Some(name) = s.name.as_deref() else {
                continue;
            };
            let lamp = name.contains("lighting_outdoor_")
                || name == "recreational_sports_01_21"
                || name == "recreational_sports_01_19"
                || name == "recreational_sports_01_32";
            if !lamp {
                continue;
            }
            if m.has_prop(s, "MoveType") && m.prop(s, "MoveType") == Some("WallObject") {
                continue;
            }
            column = true;
            break;
        }
        if column {
            n = add(&mut shapes, n, S::Tree as i32);
        }
    }
    let solid_thumpable = sq.special_objects.iter().any(|id| {
        sq.objects
            .iter()
            .find(|o| o.id == *id)
            .is_some_and(|o| matches!(&o.kind, ObjectKind::Thumpable(t) if t.block_all_the_square))
    });
    let stairs = sq.has_stairs(m);
    if stairs {
        if sq.has_type(t.stairsMN) {
            n = add(&mut shapes, n, S::StairsMiddleNorth as i32);
        }
        if sq.has_type(t.stairsMW) {
            n = add(&mut shapes, n, S::StairsMiddleWest as i32);
        }
    }
    if sq.has_type(t.isMoveAbleObject) {
        n = add(&mut shapes, n, S::Tree as i32);
    }
    if sq.has_type(t.tree) {
        let tree = get_str("tree");
        let wind = get_str("WindType");
        if tree.is_none() {
            n = add(&mut shapes, n, S::Tree as i32);
        }
        if let Some(tr) = tree {
            let small = tr == "1" || (wind == Some("2") && (tr == "2" || tr == "1"));
            if !small {
                n = add(&mut shapes, n, S::Tree as i32);
            }
        }
    } else if props.has(f.solid)
        || props.has(f.solidtrans)
        || props.has(f.blocksight)
        || stairs
        || solid_thumpable
    {
        n = add(
            &mut shapes,
            n,
            if stairs { S::SolidStairs } else { S::Solid } as i32,
        );
    }
    if props.has(f.solidfloor) {
        n = add(&mut shapes, n, S::Floor as i32);
    }
    if has_str("CarSlowFactor") {
        return shapes;
    }
    // A closed door in a door wall; `checkForActiveRagdoll` (no ragdolls).
    if has_flag("DoorWallW") && props.has(f.doorW) && !props.has(f.open) {
        n = add(&mut shapes, n, S::WallW as i32);
    }
    if props.has(f.collideW) || props.has(f.windowW) {
        n = add(&mut shapes, n, S::WallW as i32);
    }
    if has_flag("DoorWallN") && props.has(f.doorN) && !props.has(f.open) {
        n = add(&mut shapes, n, S::WallN as i32);
    }
    if props.has(f.collideN) || props.has(f.windowN) {
        n = add(&mut shapes, n, S::WallN as i32);
    }
    if has_str("PhysicsShape") {
        let shape = match get_str("PhysicsShape") {
            Some("Solid") => Some(S::Solid),
            Some("WallN") => Some(S::WallN),
            Some("WallW") => Some(S::WallW),
            Some("WallS") => Some(S::WallS),
            Some("WallE") => Some(S::WallE),
            Some("Tree") => Some(S::Tree),
            Some("Floor") => Some(S::Floor),
            _ => None,
        };
        if let Some(s) = shape {
            n = add(&mut shapes, n, s as i32);
        }
    }
    if has_str("PhysicsMesh") {
        let mut name = get_str("PhysicsMesh").unwrap_or("").to_string();
        if !name.contains('.') {
            name = format!("Base.{name}");
        }
        if let Some(i) = mesh_index(&name) {
            add(&mut shapes, n, S::FirstMesh as i32 + i);
        }
    }
    shapes
}

/// `updatePhysicsForLevel(level)`: the `ToBullet` buffer, `None` when the
/// level has no shapes.
pub fn update_physics_for_level(
    chunk: &IsoChunk,
    level: i32,
    m: &IsoSpriteManager,
    mesh_index: &dyn Fn(&str) -> Option<i32>,
) -> Option<Vec<u8>> {
    let (min, max) = (chunk.min_level, chunk.max_level);
    build(
        chunk.wx,
        chunk.wy,
        min,
        max,
        level,
        mesh_index,
        m,
        &|x, y| {
            if (min..=max).contains(&level) {
                chunk.get_grid_square(x, y, level)
            } else {
                None
            }
        },
    )
}

#[allow(clippy::too_many_arguments)]
fn build<'a>(
    wx: i32,
    wy: i32,
    min_level: i32,
    max_level: i32,
    level: i32,
    mesh_index: &dyn Fn(&str) -> Option<i32>,
    m: &IsoSpriteManager,
    square: &dyn Fn(i32, i32) -> Option<&'a IsoGridSquare>,
) -> Option<Vec<u8>> {
    let mut buf = vec![8u8];
    for v in [wx, wy, min_level, max_level, level] {
        buf.extend_from_slice(&(v as i16).to_le_bytes());
    }
    for y in 0..8 {
        for x in 0..8 {
            let shapes = calc_physics(square(x, y), level, m, mesh_index);
            let present: Vec<u8> = shapes
                .iter()
                .filter(|&&s| s != -1)
                .map(|&s| (s + 1) as u8)
                .collect();
            buf.push(x as u8);
            buf.push(y as u8);
            buf.push(present.len() as u8);
            buf.extend_from_slice(&present);
        }
    }
    if buf.len() == 11 {
        return None;
    }
    buf.push(0xff);
    buf.push(0xff);
    Some(buf)
}

/// [`update_physics_for_level`] for a chunk whose level range is
/// `min_level..=max_level` (levels the squares do not cover are empty).
pub fn update_physics_for_level_in(
    chunk: &IsoChunk,
    level: i32,
    min_level: i32,
    max_level: i32,
    m: &IsoSpriteManager,
    mesh_index: &dyn Fn(&str) -> Option<i32>,
) -> Option<Vec<u8>> {
    let (cmin, cmax) = (chunk.min_level, chunk.max_level);
    build(
        chunk.wx,
        chunk.wy,
        min_level,
        max_level,
        level,
        mesh_index,
        m,
        &|x, y| {
            if (cmin..=cmax).contains(&level) {
                chunk.get_grid_square(x, y, level)
            } else {
                None
            }
        },
    )
}

/// `IsoChunkLevel.physicsCheck` flags and `Bullet.physicsShapeNameToIndex`,
/// with `Bullet.updatePhysicsForLevelIfNeeded` on top (pz-world keeps no
/// physics flags of its own).
#[derive(Clone, Debug, Default)]
pub struct ChunkPhysics {
    pending: std::collections::HashSet<(i32, i32, i32)>,
    /// `Bullet.physicsShapeNameToIndex` (`Module.Name` to mesh index).
    pub mesh_index: std::collections::HashMap<String, i32>,
}

impl ChunkPhysics {
    /// `IsoChunk.checkPhysicsLater(level)`.
    pub fn check_physics_later(&mut self, wx: i32, wy: i32, level: i32) {
        self.pending.insert((wx, wy, level));
    }

    /// `checkPhysicsLater` for every level of a chunk (as chunk loading
    /// does).
    pub fn check_chunk_later(&mut self, chunk: &IsoChunk) {
        for z in chunk.min_level..=chunk.max_level {
            self.check_physics_later(chunk.wx, chunk.wy, z);
        }
    }

    /// `Bullet.updatePhysicsForLevelIfNeeded(wx, wy, level)`: `valid_chunk`
    /// is `metaGrid.isValidChunk(wx, wy)`; the buffer goes to `to_bullet`.
    #[allow(clippy::too_many_arguments)]
    pub fn update_if_needed(
        &mut self,
        cell: &pz_world::IsoCell,
        valid_chunk: bool,
        m: &IsoSpriteManager,
        wx: i32,
        wy: i32,
        level: i32,
        to_bullet: &mut Vec<Vec<u8>>,
    ) -> bool {
        if !valid_chunk {
            return false;
        }
        let Some(chunk) = cell.chunks.get(&(wx, wy)) else {
            return false;
        };
        if !(chunk.min_level..=chunk.max_level).contains(&level) {
            return false;
        }
        if !self.pending.remove(&(wx, wy, level)) {
            return false;
        }
        let index = &self.mesh_index;
        if let Some(b) = update_physics_for_level(chunk, level, m, &|n| index.get(n).copied()) {
            to_bullet.push(b);
        }
        true
    }
}
