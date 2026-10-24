//! The string table of `*.strings.bank` files (`STDT` chunk): a radix tree
//! mapping lower-cased paths such as `event:/Zombies/Male 1` to GUIDs.
//!
//! ```text
//! u32 version (1)
//! struct array of nodes, 8 bytes each:
//!     u24 label offset (0xffffff = empty)  u8 first label byte, lower-cased
//!     u24 first child or GUID index        u8 child count (0 = leaf)
//! struct array of GUIDs
//! count + label bytes (NUL-terminated labels)
//! count + u24 leaf node per GUID
//! count + u24 parent per node (0xffffff for the root)
//! ```

use crate::reader::{Guid, Reader, Result};
use std::collections::HashMap;

const NONE: u32 = 0xff_ffff;

#[derive(Default, Clone, Debug)]
pub struct StringTable {
    pub by_guid: HashMap<Guid, String>,
    by_path: HashMap<String, Guid>,
}

impl StringTable {
    pub fn parse(data: &[u8]) -> Result<StringTable> {
        let mut r = Reader::new(data);
        let _version = r.u32()?;
        let nodes = r.read_structs(|r| Ok((r.u24()?, r.u8()?, r.u24()?, r.u8()?)))?;
        let guids = r.read_structs(|r| r.guid())?;
        let n = r.count()?;
        let labels = r.bytes(n)?;
        let n = r.count()?;
        let leaves = (0..n).map(|_| r.u24()).collect::<Result<Vec<_>>>()?;
        let n = r.count()?;
        let parents = (0..n).map(|_| r.u24()).collect::<Result<Vec<_>>>()?;

        let label = |i: usize| -> &[u8] {
            let off = nodes.get(i).map_or(NONE, |n| n.0);
            if off == NONE {
                return &[];
            }
            let s = labels.get(off as usize..).unwrap_or(&[]);
            &s[..s.iter().position(|&c| c == 0).unwrap_or(s.len())]
        };
        let mut t = StringTable::default();
        for (g, &leaf) in guids.iter().zip(&leaves) {
            let mut parts = Vec::new();
            let mut node = leaf;
            // Bounded walk in case of a malformed parent chain.
            for _ in 0..parents.len() {
                if node == NONE {
                    break;
                }
                parts.push(label(node as usize));
                node = parents.get(node as usize).copied().unwrap_or(NONE);
            }
            let path: Vec<u8> = parts.iter().rev().flat_map(|p| p.iter().copied()).collect();
            let path = String::from_utf8_lossy(&path).into_owned();
            t.by_path.insert(path.to_lowercase(), *g);
            t.by_guid.insert(*g, path);
        }
        Ok(t)
    }

    pub fn len(&self) -> usize {
        self.by_guid.len()
    }

    pub fn is_empty(&self) -> bool {
        self.by_guid.is_empty()
    }

    pub fn path(&self, g: &Guid) -> Option<&str> {
        self.by_guid.get(g).map(String::as_str)
    }

    /// `Studio::System::lookupID`: paths match case-insensitively.
    pub fn lookup(&self, path: &str) -> Option<Guid> {
        self.by_path.get(&path.to_lowercase()).copied()
    }

    pub fn merge(&mut self, other: StringTable) {
        self.by_path.extend(other.by_path);
        self.by_guid.extend(other.by_guid);
    }
}
