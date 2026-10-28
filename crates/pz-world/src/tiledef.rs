//! `.tiles` tile definition files, as read by `IsoWorld.LoadTileDefinitions`.
//!
//! This is the file layer: tilesets, tiles, sprite ids and each tile's raw
//! property pairs in file order. Applying the properties to `IsoSprite`
//! (tile types, flags, movable groups, sprite grids) builds on this.

use std::io::{self, BufRead, BufReader, Read};
use std::path::Path;

pub const TDEF_FILE_MAGIC: [u8; 4] = *b"tdef";

/// The `fileNumber` passed for each file in `IsoWorld.LoadTileDefinitions`
/// order. `-1` is a patch file applied to sprites that already exist.
pub const TILE_DEFINITION_FILES: [(&str, i32); 7] = [
    ("newtiledefinitions.tiles", 1),
    ("tiledefinitions_erosion.tiles", 2),
    ("tiledefinitions_overlays.tiles", 4),
    ("tiledefinitions_b42chunkcaching.tiles", 5),
    ("tiledefinitions_noiseworks.patch.tiles", -1),
    ("jumbo_trees_big.tiles", 6),
    ("jumbo_trees.tiles", 8),
];

#[derive(Clone, Debug)]
pub struct TileDef {
    /// `tilesetName + "_" + index`.
    pub name: String,
    pub index: i32,
    /// Raw `(property, value)` pairs, each `trim()`med, in file order.
    pub props: Vec<(String, String)>,
}

#[derive(Clone, Debug)]
pub struct TileSet {
    pub name: String,
    pub image_name: String,
    pub w_tiles: i32,
    pub h_tiles: i32,
    pub tileset_number: i32,
    pub tiles: Vec<TileDef>,
}

#[derive(Clone, Debug)]
pub struct TileDefFile {
    pub file_number: i32,
    pub patch: bool,
    pub tilesets: Vec<TileSet>,
}

/// `IsoWorld.getSpriteID(fileNumber, tilesetNumber, tileIndex)`.
pub fn sprite_id(file_number: i32, tileset_number: i32, tile_index: i32) -> i32 {
    if file_number == 1 {
        return (tileset_number - 1) * 1024 + tile_index;
    }
    0x100000 + (file_number - 2) * 262144 + (tileset_number - 1) * 512 + tile_index
}

pub(crate) fn read_int<R: Read>(r: &mut R) -> io::Result<i32> {
    let mut b = [0u8; 4];
    r.read_exact(&mut b)?;
    Ok(i32::from_le_bytes(b))
}

/// `IsoWorld.readString(InputStream, StringBuilder)`: bytes up to `\n` or
/// end of stream as Latin-1; a `\r` is an error.
pub(crate) fn read_string<R: BufRead>(r: &mut R) -> io::Result<Option<String>> {
    let mut out = String::new();
    let mut c: i32;
    loop {
        let buf = r.fill_buf()?;
        if buf.is_empty() {
            c = -1;
            break;
        }
        let b = buf[0];
        r.consume(1);
        c = b as i32;
        match b {
            b'\n' => break,
            b'\r' => return Err(io::Error::other("\r\n unsupported")),
            _ => out.push(b as char),
        }
    }
    if c == -1 && out.is_empty() {
        return Ok(None);
    }
    Ok(Some(out))
}

fn trimmed(s: Option<String>) -> String {
    // A null string NPEs on `trim()` upstream; treat it as empty.
    crate::java::java_trim(&s.unwrap_or_default()).to_string()
}

impl TileDefFile {
    pub fn read(path: &Path, file_number: i32) -> io::Result<TileDefFile> {
        let patch = path.to_string_lossy().ends_with(".patch.tiles");
        let mut r = BufReader::new(std::fs::File::open(path)?);
        Self::parse(&mut r, &path.display().to_string(), file_number, patch)
    }

    pub fn parse<R: BufRead>(
        r: &mut R,
        filename: &str,
        file_number: i32,
        patch: bool,
    ) -> io::Result<TileDefFile> {
        let max_tilesets = if file_number == 1 { 1024 } else { 512 };
        let max_tiles = if file_number == 1 { 1024 } else { 512 };
        let mut magic = [0u8; 4];
        let n = r.read(&mut magic)?;
        if n != 4 || magic != TDEF_FILE_MAGIC {
            return Err(io::Error::other("not magic"));
        }
        let version = read_int(r)?;
        if version != 1 {
            return Err(io::Error::other(format!(
                "{filename}: invalid version {version}, must be from 1 to 1"
            )));
        }
        let num_tilesheets = read_int(r)?;
        if num_tilesheets < 0 || num_tilesheets > max_tilesets {
            return Err(io::Error::other(format!(
                "{filename}: invalid number of tilesets {num_tilesheets}, must be from 0 to {max_tilesets}"
            )));
        }
        let mut tilesets = Vec::with_capacity(num_tilesheets as usize);
        for _ in 0..num_tilesheets {
            let name = trimmed(read_string(r)?);
            let image_name = read_string(r)?.unwrap_or_default();
            let w_tiles = read_int(r)?;
            let h_tiles = read_int(r)?;
            let tileset_number = read_int(r)?;
            if tileset_number < 1 || tileset_number > max_tilesets {
                return Err(io::Error::other(format!(
                    "{filename}: invalid tileset number {tileset_number}, must be from 1 to {max_tilesets} in {name}"
                )));
            }
            let n_tiles = read_int(r)?;
            if n_tiles < 0 || n_tiles > max_tiles {
                return Err(io::Error::other(format!("{filename}: invalid number of tiles {n_tiles}, must be from 0 to {max_tiles} in {name}")));
            }
            let mut tiles = Vec::with_capacity(n_tiles as usize);
            for m in 0..n_tiles {
                let n_props = read_int(r)?;
                let mut props = Vec::with_capacity(n_props.max(0) as usize);
                for _ in 0..n_props {
                    let prop = trimmed(read_string(r)?);
                    let val = trimmed(read_string(r)?);
                    props.push((prop, val));
                }
                tiles.push(TileDef {
                    name: format!("{name}_{m}"),
                    index: m,
                    props,
                });
            }
            tilesets.push(TileSet {
                name,
                image_name,
                w_tiles,
                h_tiles,
                tileset_number,
                tiles,
            });
        }
        Ok(TileDefFile {
            file_number,
            patch,
            tilesets,
        })
    }

    pub fn sprite_id(&self, tileset: &TileSet, tile: &TileDef) -> i32 {
        sprite_id(self.file_number, tileset.tileset_number, tile.index)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn int(v: &mut Vec<u8>, i: i32) {
        v.extend(i.to_le_bytes());
    }

    #[test]
    fn parses_tileset() {
        let mut v = b"tdef".to_vec();
        int(&mut v, 1);
        int(&mut v, 1);
        v.extend(b" floors_01 \nfloors_01.png\n");
        int(&mut v, 8);
        int(&mut v, 16);
        int(&mut v, 3);
        int(&mut v, 2);
        int(&mut v, 1);
        v.extend(b"solidfloor\n\n");
        int(&mut v, 0);
        let f = TileDefFile::parse(&mut &v[..], "t", 2, false).unwrap();
        let ts = &f.tilesets[0];
        assert_eq!(ts.name, "floors_01");
        assert_eq!(ts.tiles[0].name, "floors_01_0");
        assert_eq!(
            ts.tiles[0].props,
            vec![("solidfloor".to_string(), String::new())]
        );
        assert_eq!(f.sprite_id(ts, &ts.tiles[1]), 0x100000 + 2 * 512 + 1);
    }

    #[test]
    fn rejects_cr() {
        let mut v = b"tdef".to_vec();
        int(&mut v, 1);
        int(&mut v, 1);
        v.extend(b"a\r\n");
        assert!(TileDefFile::parse(&mut &v[..], "t", 1, false).is_err());
    }
}
