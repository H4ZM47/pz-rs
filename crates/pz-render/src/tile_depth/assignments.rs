//! `TileDepthTextureAssignments` / `TileDepthTextureAssignmentManager`:
//! `media/tileDepthTextureAssignments.txt`, which lets a tile without its own
//! depth map borrow another tile's.

use std::collections::HashMap;
use std::io;
use std::path::{Path, PathBuf};

use pz_fs::java;
use pz_script_parser as script_parser;
use pz_fs::GameFs;

/// `TileDepthTextureAssignments.VERSION_LATEST`.
pub const VERSION_LATEST: i32 = 1;
/// The file name under a media directory.
pub const FILE_NAME: &str = "tileDepthTextureAssignments.txt";

/// `TileDepthTextureAssignments`: tile name -> tile whose depth it uses.
#[derive(Clone, Debug, Default)]
pub struct TileDepthTextureAssignments {
    assignments: HashMap<String, String>,
    pub version: i32,
}

fn invalid(msg: String) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, msg)
}

impl TileDepthTextureAssignments {
    pub fn new() -> Self {
        TileDepthTextureAssignments { assignments: HashMap::new(), version: VERSION_LATEST }
    }

    /// `assignTileName(assignTo, otherTile)`: a blank `other_tile` removes
    /// the assignment; a tile is never assigned to itself.
    pub fn assign_tile_name(&mut self, assign_to: &str, other_tile: &str) {
        if assign_to.is_empty() {
            return;
        }
        if java::is_null_or_whitespace(other_tile) {
            self.assignments.remove(assign_to);
            return;
        }
        if assign_to == other_tile {
            return;
        }
        self.assignments.insert(assign_to.to_string(), other_tile.to_string());
    }

    /// `clearAssignedTileName(assignTo)`.
    pub fn clear_assigned_tile_name(&mut self, assign_to: &str) {
        self.assignments.remove(assign_to);
    }

    /// `getAssignedTileName(tileName)`.
    pub fn assigned_tile_name(&self, tile_name: &str) -> Option<&str> {
        self.assignments.get(tile_name).map(String::as_str)
    }

    pub fn len(&self) -> usize {
        self.assignments.len()
    }

    pub fn is_empty(&self) -> bool {
        self.assignments.is_empty()
    }

    /// `load()`: clears, then reads `<media>/tileDepthTextureAssignments.txt`.
    /// A missing file is not an error; on a parse error the assignments
    /// read so far are kept (the original logs it).
    pub fn load(&mut self, media_dir: &Path) -> io::Result<()> {
        self.assignments.clear();
        let bytes = match std::fs::read(media_dir.join(FILE_NAME)) {
            Ok(b) => b,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(()),
            Err(e) => return Err(e),
        };
        self.parse(&java::decode_utf8(&bytes))
    }

    /// `parseFile(text)`: the first block's `VERSION` must be 1; every
    /// `key = value` of that block (`VERSION` included) is an assignment.
    pub fn parse(&mut self, text: &str) -> io::Result<()> {
        let root = script_parser::parse(text);
        let block = root.children.first().ok_or_else(|| invalid("no block in tileDepthTextureAssignments.txt".into()))?;
        let value = block.get_value("VERSION").ok_or_else(|| invalid("missing VERSION in tileGeometry.txt".into()))?;
        let v = java::trim(value.get_value());
        self.version = java::try_parse_int(v, -1);
        if self.version < 1 || self.version > VERSION_LATEST {
            return Err(invalid(format!("unknown tileGeometry.txt VERSION \"{v}\"")));
        }
        for value in &block.values {
            let assign_to = java::trim(value.get_key());
            let other_tile = java::trim(value.get_value());
            if java::is_null_or_whitespace(assign_to) || java::is_null_or_whitespace(other_tile) {
                continue;
            }
            self.assign_tile_name(assign_to, other_tile);
        }
        Ok(())
    }
}

/// `TileDepthTextureAssignmentManager.ModData`.
#[derive(Debug)]
pub struct AssignmentModData {
    pub mod_id: String,
    pub media_dir: PathBuf,
    pub assignments: TileDepthTextureAssignments,
}

/// `TileDepthTextureAssignmentManager`: the assignments of the mods that
/// have the file, in load order, then the game's.
#[derive(Debug, Default)]
pub struct TileDepthTextureAssignmentManager {
    pub mod_data: Vec<AssignmentModData>,
    /// Files that failed to parse.
    pub errors: Vec<String>,
}

impl TileDepthTextureAssignmentManager {
    /// `init()`.
    pub fn init(fs: &mut GameFs) -> Self {
        let mods = super::manager::mod_media_dirs(fs, FILE_NAME);
        Self::init_from_dirs(&mods, Path::new(fs.media_root()))
    }

    /// `init()` over explicit (mod id, media directory) pairs.
    pub fn init_from_dirs(mods: &[(String, PathBuf)], game_media: &Path) -> Self {
        let mut m = TileDepthTextureAssignmentManager::default();
        let game = ("game".to_string(), game_media.to_path_buf());
        for (id, media) in mods.iter().chain(std::iter::once(&game)) {
            let mut assignments = TileDepthTextureAssignments::new();
            if let Err(e) = assignments.load(media) {
                m.errors.push(format!("{}: {e}", media.join(FILE_NAME).display()));
            }
            m.mod_data.push(AssignmentModData { mod_id: id.clone(), media_dir: media.clone(), assignments });
        }
        m
    }

    /// `getModData(modID)`.
    pub fn mod_data(&self, mod_id: &str) -> Option<&AssignmentModData> {
        self.mod_data.iter().find(|d| d.mod_id == mod_id)
    }

    /// `getAssignedTileName(tileName)`: the first mod data that assigns it.
    pub fn assigned_tile_name(&self, tile_name: &str) -> Option<&str> {
        self.mod_data.iter().find_map(|d| d.assignments.assigned_tile_name(tile_name))
    }
}
