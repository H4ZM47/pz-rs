//! `Path` and `PathNode`: the result of a search.

use crate::bytes::{ByteWriter, PathfindError};
use crate::util::dist3;

/// `PathNode` flag: the node is a crawl node (under a vehicle).
pub const NODE_FLAG_CRAWL: i32 = 1;
/// `PathNode` flag: the node is inside a vehicle's crawl area.
pub const NODE_FLAG_CRAWL_INTERIOR: i32 = 2;
/// `PathNode` flag: keep this node; the path goes through a door/gap edge.
pub const NODE_FLAG_KEEP: i32 = 0x10000;

/// `PathNode`. Coordinates are world squares; `z` carries the +32 level
/// offset in the native representation (see [`PathNode::level`]).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PathNode {
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub flags: i32,
}

impl PathNode {
    /// The level in game terms (`z - 32`) of a node of a **raw native**
    /// path, as `World::find_path*` returns it (the native side stores
    /// levels offset by 32).
    ///
    /// Do not call it on paths delivered by `PathfindService` or
    /// `PathFindBehavior2`: the service already converts their nodes to
    /// game levels (as `PathfindNativeThread` does), so `z` is the level
    /// there and this would be 32 too low.
    pub fn level(&self) -> f32 {
        self.z - 32.0
    }

    pub fn has_flag(&self, flag: i32) -> bool {
        self.flags & flag != 0
    }

    /// `isApproximatelyEqual(x, y, z)` (tolerance `0.01f`).
    pub fn is_approximately_equal(&self, x: f32, y: f32, z: f32) -> bool {
        (self.x - x).abs() < 0.01f32 && (self.y - y).abs() < 0.01f32 && (self.z - z).abs() < 0.01f32
    }
}

/// `Path`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Path {
    pub nodes: Vec<PathNode>,
}

impl Path {
    pub fn add_node(&mut self, x: f32, y: f32, z: f32, flags: i32) {
        self.nodes.push(PathNode { x, y, z, flags });
    }

    pub fn clear(&mut self) {
        self.nodes.clear();
    }

    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    /// `Path::length()`: sum of segment lengths, with level differences
    /// weighted by 2.5.
    pub fn length(&self) -> f32 {
        let mut total = 0.0f32;
        for w in self.nodes.windows(2) {
            let (a, b) = (w[0], w[1]);
            total += dist3(b.x - a.x, b.y - a.y, (b.z - a.z) * 2.5);
        }
        total
    }

    /// Serialise as the JNI `findPath` writes the result buffer: `i16`
    /// node count, then per node `f32 x, f32 y, f32 z+32, i32 flags`
    /// (big-endian, the order `PathfindNative.init` gives the buffer).
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut w = ByteWriter::with_limit(usize::MAX);
        let _ = self.write_to(&mut w);
        w.data
    }

    pub(crate) fn write_to(&self, w: &mut ByteWriter) -> Result<(), PathfindError> {
        w.put_i16(self.nodes.len() as i16)?;
        for n in &self.nodes {
            w.put_f32(n.x)?;
            w.put_f32(n.y)?;
            w.put_f32(n.z)?;
            w.put_i32(n.flags)?;
        }
        Ok(())
    }
}
