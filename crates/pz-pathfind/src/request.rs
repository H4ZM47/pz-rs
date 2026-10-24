//! `PathFindRequest`, `PMMover` and the request byte format of
//! `PathfindNative.findPath`.

use crate::bytes::{ByteReader, PathfindError};
use crate::high_level::TransId;
use crate::path::Path;
use crate::world::LevelRef;

/// Size of the request buffer written by `PathfindNative.findPath`.
pub const REQUEST_SIZE: usize = 47;

/// `MoverType` as encoded in the request (`PMMover::isAnimal` etc.).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum MoverKind {
    #[default]
    Animal,
    Player,
    Zombie,
}

impl MoverKind {
    pub fn from_i32(v: i32) -> MoverKind {
        match v {
            0 => MoverKind::Animal,
            1 => MoverKind::Player,
            2 => MoverKind::Zombie,
            // The native code only compares against 0, 1 and 2.
            _ => MoverKind::Animal,
        }
    }

    fn code(self) -> i32 {
        match self {
            MoverKind::Animal => 0,
            MoverKind::Player => 1,
            MoverKind::Zombie => 2,
        }
    }
}

/// `PMMover`: what the moving character can do.
#[derive(Clone, Debug, Default)]
pub struct PmMover {
    pub kind: MoverKind,
    pub is_npc: bool,
    pub can_crawl: bool,
    pub crawling: bool,
    pub ignore_crawl_cost: bool,
    pub can_thump: bool,
    pub can_climb_fences: bool,
    pub can_climb_tall_fences: bool,
    pub has_target: bool,
    pub can_bend: bool,
    pub avoid_farming_plants: bool,
    pub min_level: i32,
    pub max_level: i32,
    pub(crate) allowed_chunk_levels: Vec<LevelRef>,
    pub(crate) allowed_level_transitions: Vec<TransId>,
}

impl PmMover {
    pub fn is_animal(&self) -> bool {
        self.kind == MoverKind::Animal
    }
    pub fn is_player(&self) -> bool {
        self.kind == MoverKind::Player
    }
    pub fn is_zombie(&self) -> bool {
        self.kind == MoverKind::Zombie
    }
}

/// A typed pathfinding request (the decoded 47-byte request buffer).
///
/// Levels are given the way the game does it (ground floor 0); the +32
/// offset used on the wire and inside the native code is applied by
/// [`PathFindRequest::to_bytes`] / [`PathFindRequest::from_bytes`].
#[derive(Clone, Debug, PartialEq)]
pub struct PathFindRequest {
    pub start_x: f32,
    pub start_y: f32,
    pub start_z: f32,
    pub target_x: f32,
    pub target_y: f32,
    pub target_z: f32,
    pub mover: MoverKind,
    pub is_npc: bool,
    pub can_crawl: bool,
    pub crawling: bool,
    pub ignore_crawl_cost: bool,
    pub can_thump: bool,
    pub can_climb_fences: bool,
    pub has_target: bool,
    pub can_bend: bool,
    /// Lowest level the path may use, with the +32 offset (the Java side
    /// sends 0 by default).
    pub min_level: i32,
    /// Highest level, with the +32 offset (the Java side sends 63).
    pub max_level: i32,
    /// Debug rendering flag; has no effect on the result.
    pub render: bool,
    pub can_climb_tall_fences: bool,
    pub avoid_farming_plants: bool,
}

impl PathFindRequest {
    /// A request with the defaults `PathFindRequest.init` uses for the
    /// given mover type.
    pub fn new(mover: MoverKind, start: (f32, f32, f32), target: (f32, f32, f32)) -> Self {
        PathFindRequest {
            start_x: start.0,
            start_y: start.1,
            start_z: start.2,
            target_x: target.0,
            target_y: target.1,
            target_z: target.2,
            mover,
            is_npc: false,
            can_crawl: false,
            crawling: false,
            ignore_crawl_cost: false,
            can_thump: mover == MoverKind::Zombie,
            can_climb_fences: false,
            has_target: false,
            can_bend: false,
            min_level: 0,
            max_level: 63,
            render: false,
            can_climb_tall_fences: false,
            avoid_farming_plants: mover == MoverKind::Player,
        }
    }

    /// Encode as the big-endian request buffer of
    /// `PathfindNative.findPath`.
    pub fn to_bytes(&self) -> [u8; REQUEST_SIZE] {
        let mut out = [0u8; REQUEST_SIZE];
        let mut p = 0usize;
        let mut put = |b: &[u8]| {
            out[p..p + b.len()].copy_from_slice(b);
            p += b.len();
        };
        put(&self.start_x.to_be_bytes());
        put(&self.start_y.to_be_bytes());
        put(&(self.start_z + 32.0).to_be_bytes());
        put(&self.target_x.to_be_bytes());
        put(&self.target_y.to_be_bytes());
        put(&(self.target_z + 32.0).to_be_bytes());
        put(&self.mover.code().to_be_bytes());
        for b in [
            self.is_npc,
            self.can_crawl,
            self.crawling,
            self.ignore_crawl_cost,
            self.can_thump,
            self.can_climb_fences,
            self.has_target,
            self.can_bend,
        ] {
            put(&[b as u8]);
        }
        put(&self.min_level.to_be_bytes());
        put(&self.max_level.to_be_bytes());
        put(&[self.render as u8]);
        put(&[self.can_climb_tall_fences as u8]);
        put(&[self.avoid_farming_plants as u8]);
        out
    }

    /// Decode a request buffer (big-endian, as the Java side writes it).
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, PathfindError> {
        let mut bb = ByteReader::big_endian(bytes);
        let start_x = bb.get_f32()?;
        let start_y = bb.get_f32()?;
        let start_z = bb.get_f32()? - 32.0;
        let target_x = bb.get_f32()?;
        let target_y = bb.get_f32()?;
        let target_z = bb.get_f32()? - 32.0;
        let mover = MoverKind::from_i32(bb.get_i32()?);
        Ok(PathFindRequest {
            start_x,
            start_y,
            start_z,
            target_x,
            target_y,
            target_z,
            mover,
            is_npc: bb.get_bool()?,
            can_crawl: bb.get_bool()?,
            crawling: bb.get_bool()?,
            ignore_crawl_cost: bb.get_bool()?,
            can_thump: bb.get_bool()?,
            can_climb_fences: bb.get_bool()?,
            has_target: bb.get_bool()?,
            can_bend: bb.get_bool()?,
            min_level: bb.get_i32()?,
            max_level: bb.get_i32()?,
            render: bb.get_bool()?,
            can_climb_tall_fences: bb.get_bool()?,
            avoid_farming_plants: bb.get_bool()?,
        })
    }
}

/// The native in-flight request: the mover plus coordinates with the +32
/// level offset applied, and the path being built.
pub(crate) struct NativeRequest {
    pub start_x: f32,
    pub start_y: f32,
    pub start_z: f32,
    pub target_x: f32,
    pub target_y: f32,
    pub target_z: f32,
    pub path: Path,
}

impl NativeRequest {
    /// Decode the raw request buffer exactly like the JNI `findPath` (the
    /// level fields keep their +32 offset). Returns the mover separately.
    pub fn decode(bytes: &[u8]) -> Result<(NativeRequest, PmMover, bool), PathfindError> {
        let mut bb = ByteReader::big_endian(bytes);
        let start_x = bb.get_f32()?;
        let start_y = bb.get_f32()?;
        let start_z = bb.get_f32()?;
        let target_x = bb.get_f32()?;
        let target_y = bb.get_f32()?;
        let target_z = bb.get_f32()?;
        let kind = MoverKind::from_i32(bb.get_i32()?);
        let mut m = PmMover {
            kind,
            ..PmMover::default()
        };
        m.is_npc = bb.get_bool()?;
        m.can_crawl = bb.get_bool()?;
        m.crawling = bb.get_bool()?;
        m.ignore_crawl_cost = bb.get_bool()?;
        m.can_thump = bb.get_bool()?;
        m.can_climb_fences = bb.get_bool()?;
        m.has_target = bb.get_bool()?;
        m.can_bend = bb.get_bool()?;
        m.min_level = bb.get_i32()?;
        m.max_level = bb.get_i32()?;
        let render = bb.get_bool()?;
        m.can_climb_tall_fences = bb.get_bool()?;
        m.avoid_farming_plants = bb.get_bool()?;
        Ok((
            NativeRequest {
                start_x,
                start_y,
                start_z,
                target_x,
                target_y,
                target_z,
                path: Path::default(),
            },
            m,
            render,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_roundtrip() {
        let mut r = PathFindRequest::new(MoverKind::Zombie, (10.5, 11.5, 0.0), (20.25, 3.0, 1.0));
        r.has_target = true;
        let b = r.to_bytes();
        assert_eq!(b.len(), 47);
        assert_eq!(PathFindRequest::from_bytes(&b).unwrap(), r);
        let (nr, m, _) = NativeRequest::decode(&b).unwrap();
        assert_eq!(nr.start_z, 32.0);
        assert_eq!(nr.target_z, 33.0);
        assert!(m.is_zombie() && m.has_target && m.can_thump);
    }
}
