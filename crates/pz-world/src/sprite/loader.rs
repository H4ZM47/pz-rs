//! Tile definition loading from `IsoWorld`: `LoadTileDefinitionsPropertyStrings`,
//! `SetCustomPropertyValues`, `GenerateTilePropertyLookupTables`,
//! `LoadTileDefinitions` (per-tile property application, `setOpenDoorProperties`,
//! movable groups and sprite grids), `registerFakeJumboTree` and the load
//! order in `IsoWorld.init`.
//!
//! Like the original, the files are read as a stream and any error (I/O,
//! `NullPointerException` on a missing string, `NumberFormatException`)
//! abandons the rest of that file while keeping what was already applied.

use std::collections::HashMap;
use std::io::{self, BufRead, BufReader};
use std::path::Path;

use pz_core::debug::{self, JulLevel, StackElement};

use super::manager::{GridIdx, IsoSpriteGrid, IsoSpriteManager, SpriteIdx};
use super::properties::{PropertyValueMap, TilePropertyAliasMap};
use super::types::SpriteTables;
use super::{is_null_or_whitespace, java_split, parse_int, try_parse_int};
use crate::java::{hashmap_order, io_throwable, java_trim};
use crate::tiledef::{read_int, read_string, sprite_id, TDEF_FILE_MAGIC, TILE_DEFINITION_FILES};

const FACE_NAMES: [&str; 4] = ["N", "E", "S", "W"];

/// `readString(...).trim()`: a missing string is a `NullPointerException`.
fn read_trimmed<R: BufRead>(r: &mut R) -> io::Result<String> {
    match read_string(r)? {
        Some(s) => Ok(java_trim(&s).to_string()),
        None => Err(io::Error::other("NullPointerException: readString")),
    }
}

/// Header of a `.tiles` file; returns `(numTilesheets, maxTilesets, maxTiles)`.
fn read_header<R: BufRead>(
    r: &mut R,
    filename: &str,
    file_number: i32,
) -> io::Result<(i32, i32, i32)> {
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
    let num = read_int(r)?;
    if num < 0 || num > max_tilesets {
        return Err(io::Error::other(format!(
            "{filename}: invalid number of tilesets {num}, must be from 0 to {max_tilesets}"
        )));
    }
    Ok((num, max_tilesets, max_tiles))
}

/// Tileset header after the name; returns `(tilesetNumber, nTiles)`.
fn read_tileset_header<R: BufRead>(
    r: &mut R,
    filename: &str,
    name: &str,
    max_tilesets: i32,
    max_tiles: i32,
) -> io::Result<(i32, i32)> {
    let _image_name = read_string(r)?;
    let _w_tiles = read_int(r)?;
    let _h_tiles = read_int(r)?;
    let tileset_number = read_int(r)?;
    if tileset_number < 1 || tileset_number > max_tilesets {
        return Err(io::Error::other(format!(
            "{filename}: invalid tileset number {tileset_number}, must be from 1 to {max_tilesets} in {name}"
        )));
    }
    let n_tiles = read_int(r)?;
    if n_tiles < 0 || n_tiles > max_tiles {
        return Err(io::Error::other(format!(
            "{filename}: invalid number of tiles {n_tiles}, must be from 0 to {max_tiles} in {name}"
        )));
    }
    Ok((tileset_number, n_tiles))
}

/// `IsoWorld.LoadTileDefinitionsPropertyStrings`: collects every property
/// name and its distinct values.
pub fn load_property_strings<R: BufRead>(
    map: &mut PropertyValueMap,
    r: &mut R,
    filename: &str,
    file_number: i32,
) -> io::Result<()> {
    let (num, max_tilesets, max_tiles) = read_header(r, filename, file_number)?;
    for _ in 0..num {
        let name = read_trimmed(r)?;
        let (_, n_tiles) = read_tileset_header(r, filename, &name, max_tilesets, max_tiles)?;
        for _ in 0..n_tiles {
            let n_props = read_int(r)?;
            for _ in 0..n_props {
                let prop = read_trimmed(r)?;
                let val = read_trimmed(r)?;
                let values = map.entry(&prop);
                if !values.contains(&val) {
                    values.push(val);
                }
            }
        }
    }
    Ok(())
}

/// `IsoWorld.SetCustomPropertyValues`. A property it extends that no file
/// defined is a `NullPointerException` upstream (fatal to `IsoWorld.init`).
pub fn set_custom_property_values(map: &mut PropertyValueMap) -> Result<(), String> {
    fn get<'a>(map: &'a mut PropertyValueMap, k: &str) -> Result<&'a mut Vec<String>, String> {
        map.get_mut(k)
            .ok_or_else(|| format!("NullPointerException: PropertyValueMap has no {k}"))
    }
    for k in ["WindowN", "WindowW", "DoorWallN", "DoorWallW", "WallSE"] {
        get(map, k)?.push(k.to_string());
    }
    let offsets: Vec<String> = (-96..=96).map(|i: i32| i.to_string()).collect();
    for k in ["Noffset", "Soffset", "Woffset", "Eoffset"] {
        map.put(k, offsets.clone());
    }
    for v in ["5", "6", "7", "8"] {
        get(map, "tree")?.push(v.to_string());
    }
    for k in ["lightR", "lightG", "lightB"] {
        get(map, k)?.push("0".to_string());
    }
    for i in 0..=96 {
        let value = i.to_string();
        for k in ["ItemHeight", "Surface"] {
            let values = get(map, k)?;
            if !values.contains(&value) {
                values.push(value.clone());
            }
        }
    }
    Ok(())
}

/// `SpriteGridParseData.Level`.
#[derive(Default)]
struct GridLevel {
    width: i32,
    height: i32,
    z: i32,
    keys: Vec<String>,
    xy_to_sprite: HashMap<String, SpriteIdx>,
}

impl IsoSpriteManager {
    /// The full load in `IsoWorld.init` order for the base game files in
    /// `media_dir` (no mods). Per-file errors are logged and skipped as
    /// upstream; only a failing `SetCustomPropertyValues` is fatal.
    ///
    /// `entity_script_names` are the `GameEntityScript` names registered as
    /// `EntityScriptName` values (empty when scripts are not loaded).
    pub fn load_tile_definitions_from(
        tables: SpriteTables,
        media_dir: &Path,
        entity_script_names: &[String],
    ) -> Result<IsoSpriteManager, String> {
        let mut map = PropertyValueMap::default();
        for (name, number) in TILE_DEFINITION_FILES {
            let path = media_dir.join(name);
            let res = std::fs::File::open(&path).and_then(|f| {
                load_property_strings(
                    &mut map,
                    &mut BufReader::new(f),
                    &path.to_string_lossy(),
                    number,
                )
            });
            if let Err(e) = res {
                // `Logger.getLogger(GameWindow.class.getName()).log(Level.SEVERE, null, ex)`.
                debug::game_window_logger(
                    JulLevel::Severe,
                    Some(&StackElement::new(
                        "zombie.iso.IsoWorld",
                        "LoadTileDefinitionsPropertyStrings",
                    )),
                    None,
                    Some(&io_throwable(&e)),
                );
            }
        }
        let mut mgr = IsoSpriteManager::new(tables);
        mgr.generate_alias_map(map, entity_script_names)?;
        for (name, number) in TILE_DEFINITION_FILES {
            let path = media_dir.join(name);
            let res = std::fs::File::open(&path).and_then(|f| {
                mgr.load_tile_definitions(&mut BufReader::new(f), &path.to_string_lossy(), number)
            });
            if let Err(e) = res {
                // `ExceptionLogger.logException(ex)`.
                debug::log_exception(
                    &io_throwable(&e),
                    Some(&StackElement::new(
                        "zombie.iso.IsoWorld",
                        "LoadTileDefinitions",
                    )),
                );
            }
        }
        mgr.register_fake_jumbo_tree(7);
        mgr.add_sprite_named("media/ui/missing-tile.png");
        Ok(mgr)
    }

    /// `SetCustomPropertyValues` + `GenerateTilePropertyLookupTables`.
    pub fn generate_alias_map(
        &mut self,
        mut map: PropertyValueMap,
        entity_script_names: &[String],
    ) -> Result<(), String> {
        set_custom_property_values(&mut map)?;
        self.alias = TilePropertyAliasMap::generate(&map, entity_script_names);
        Ok(())
    }

    /// `IsoWorld.registerFakeJumboTree`.
    pub fn register_fake_jumbo_tree(&mut self, file_number: i32) {
        let s = self.add_sprite("jumbo_tree_01_0", file_number * 512 * 512 + 6144);
        let (t, a) = (&self.tables, &self.alias);
        let spr = &mut self.sprites[s];
        spr.name = Some("jumbo_tree_01_0".to_string());
        spr.tile_type = t.ty.tree;
        spr.properties.set(t, a, "tree", "4", true);
        spr.properties.unset_flag(t.flag.solid);
        spr.properties.set_flag(t.flag.blocksight);
    }

    /// `IsoWorld.LoadTileDefinitions(sprMan, filename, fileNumber)`. A
    /// filename ending in `.patch.tiles` changes existing sprites only.
    pub fn load_tile_definitions<R: BufRead>(
        &mut self,
        r: &mut R,
        filename: &str,
        file_number: i32,
    ) -> io::Result<()> {
        let patch = filename.ends_with(".patch.tiles");
        let (num, max_tilesets, max_tiles) = read_header(r, filename, file_number)?;
        let mut buffer: Vec<SpriteIdx> = Vec::new();
        for _ in 0..num {
            let name = read_trimmed(r)?;
            let (tileset_number, n_tiles) =
                read_tileset_header(r, filename, &name, max_tilesets, max_tiles)?;
            for m in 0..n_tiles {
                let string = format!("{name}_{m}");
                let spr = if patch {
                    match self.named_map.get(&string) {
                        Some(&s) => s,
                        None => {
                            let n_props = read_int(r)?;
                            for _ in 0..n_props {
                                read_string(r)?;
                                read_string(r)?;
                            }
                            continue;
                        }
                    }
                } else {
                    self.add_sprite(&string, sprite_id(file_number, tileset_number, m))
                };
                buffer.push(spr);
                {
                    let (t, a) = (&self.tables, &self.alias);
                    let s = &mut self.sprites[spr];
                    if !patch {
                        s.name = Some(string.clone());
                        s.tileset_name = Some(name.clone());
                        s.tile_sheet_index = m;
                    }
                    let sname = s.name().to_string();
                    if sname.contains("damaged") || sname.contains("trash_") {
                        s.attached_floor = true;
                        s.properties.set(t, a, "attachedFloor", "true", true);
                    }
                    if sname.starts_with("f_bushes") && m <= 31 {
                        s.is_bush = true;
                        s.attached_floor = true;
                    }
                }
                let n_props = read_int(r)?;
                for _ in 0..n_props {
                    let prop = read_trimmed(r)?;
                    let val = read_trimmed(r)?;
                    self.apply_property(spr, &name, &prop, val)?;
                }
                let (t, a) = (&self.tables, &self.alias);
                let p = &mut self.sprites[spr].properties;
                let lights = ["lightR", "lightG", "lightB"];
                if lights.iter().any(|l| p.has_name(a, l)) {
                    for l in lights {
                        if !p.has_name(a, l) {
                            p.set(t, a, l, "0", true);
                        }
                    }
                }
            }
            self.set_open_door_properties(&name, &buffer);
            self.apply_movables(&buffer)?;
            buffer.clear();
        }
        Ok(())
    }

    /// One `(prop, val)` pair of a tile in `LoadTileDefinitions`.
    fn apply_property(
        &mut self,
        spr: SpriteIdx,
        tileset_name: &str,
        prop: &str,
        mut val: String,
    ) -> io::Result<()> {
        let (t, a) = (&self.tables, &self.alias);
        let s = &mut self.sprites[spr];
        let (f, ty) = (t.flag, t.ty);
        macro_rules! set {
            ($k:expr, $v:expr) => {
                s.properties.set(t, a, $k, $v, true)
            };
        }
        macro_rules! setr {
            ($k:expr, $v:expr) => {
                s.properties.set_raw(a, $k, $v)
            };
        }
        macro_rules! flags {
            ($($x:ident),*) => {{ $( s.properties.set_flag(f.$x); )* }};
        }

        let typ = t.object_type_from_string(prop);
        if typ != t.object_type_max() {
            if (s.tile_type != ty.doorW && s.tile_type != ty.doorN) || typ != ty.wall {
                s.tile_type = typ;
            }
            if typ == ty.doorW {
                flags!(doorW);
            } else if typ == ty.doorN {
                flags!(doorN);
            }
        } else {
            match prop {
                "firerequirement" | "fireRequirement" => s.firerequirement = parse_int(&val)?,
                "BurntTile" => s.burnt_tile = Some(val.clone()),
                "ForceAmbient" => {
                    s.force_ambient = true;
                    set!(prop, &val);
                }
                "solidfloor" => {
                    s.solidfloor = true;
                    set!(prop, &val);
                }
                "canBeRemoved" => {
                    s.can_be_removed = true;
                    set!(prop, &val);
                }
                "attachedFloor" => {
                    s.attached_floor = true;
                    set!(prop, &val);
                }
                "cutW" => {
                    s.cut_w = true;
                    set!(prop, &val);
                }
                "cutN" => {
                    s.cut_n = true;
                    set!(prop, &val);
                }
                "solid" => {
                    s.solid = true;
                    set!(prop, &val);
                }
                "solidtrans" => {
                    s.solid_trans = true;
                    set!(prop, &val);
                }
                "invisible" => {
                    s.invisible = true;
                    set!(prop, &val);
                }
                "alwaysDraw" => {
                    s.always_draw = true;
                    set!(prop, &val);
                }
                "FasciaEdge" => {
                    s.fascia_edge = t.fascia_edge_from_string(&val);
                    set!(prop, &val);
                }
                "forceRender" => {
                    s.force_render = true;
                    set!(prop, &val);
                }
                "FloorHeight" => {
                    if val == "OneThird" {
                        flags!(FloorHeightOneThird);
                    } else if val == "TwoThirds" {
                        flags!(FloorHeightTwoThirds);
                    }
                }
                "MoveWithWind" => {
                    s.move_with_wind = true;
                    set!(prop, &val);
                }
                "WindType" => {
                    s.wind_type = parse_int(&val)?;
                    set!(prop, &val);
                }
                "RenderLayer" => {
                    set!(prop, &val);
                    if val == "Default" {
                        s.render_layer = 0;
                    } else if val == "Floor" {
                        s.render_layer = 1;
                    }
                }
                "TreatAsWallOrder" => {
                    s.treat_as_wall_order = true;
                    set!(prop, &val);
                }
                _ => {
                    set!(prop, &val);
                    if prop == "WindowN" || prop == "WindowW" {
                        setr!(prop, &val);
                    }
                }
            }
        }

        if typ == ty.tree {
            if s.name() == "e_riverbirch_1_1" {
                val = "1".to_string();
            }
            set!("tree", &val);
            flags!(blocksight);
            let mut size = parse_int(&val)?;
            if tileset_name.starts_with("vegetation_trees") {
                size = 4;
            }
            size = size.clamp(1, 4);
            if size == 1 || size == 2 {
                s.properties.unset_flag(f.blocksight);
            }
            set!("MaterialType", "Wood_Solid");
        }
        if prop == "interior" && val == "false" {
            flags!(exterior);
        }
        if prop == "HoppableN" {
            flags!(collideN, canPathN, transparentN);
        }
        if prop == "HoppableW" {
            flags!(collideW, canPathW, transparentW);
        }
        if prop == "WallN" {
            flags!(collideN, cutN);
            s.tile_type = ty.wall;
            s.cut_n = true;
            setr!("WallN", "");
        }
        if prop == "CantClimb" {
            flags!(CantClimb);
        } else if "container".eq_ignore_ascii_case(prop) {
            // `IsoPropertyType.CONTAINER.isProperty(prop)`.
            setr!(prop, &val);
        } else {
            match prop {
                "WallNTrans" => {
                    flags!(collideN, cutN, transparentN);
                    s.tile_type = ty.wall;
                    s.cut_n = true;
                    setr!("WallNTrans", "");
                }
                "WallW" => {
                    flags!(collideW, cutW);
                    s.tile_type = ty.wall;
                    s.cut_w = true;
                    setr!("WallW", "");
                }
                "windowN" => {
                    set!("WindowN", "WindowN");
                    flags!(transparentN);
                    setr!("WindowN", "WindowN");
                }
                "windowW" => {
                    set!("WindowW", "WindowW");
                    flags!(transparentW);
                    setr!("WindowW", "WindowW");
                }
                "cutW" => {
                    flags!(cutW);
                    s.cut_w = true;
                }
                "cutN" => {
                    flags!(cutN);
                    s.cut_n = true;
                }
                "WallWTrans" => {
                    flags!(collideW, transparentW, cutW);
                    s.tile_type = ty.wall;
                    s.cut_w = true;
                    setr!("WallWTrans", "");
                }
                "DoorWallN" => {
                    flags!(cutN);
                    s.cut_n = true;
                    setr!("DoorWallN", "");
                }
                "DoorWallNTrans" => {
                    flags!(cutN, transparentN);
                    s.cut_n = true;
                    setr!("DoorWallNTrans", "");
                }
                "DoorWallW" => {
                    flags!(cutW);
                    s.cut_w = true;
                    setr!("DoorWallW", "");
                }
                "DoorWallWTrans" => {
                    flags!(cutW, transparentW);
                    s.cut_w = true;
                    setr!("DoorWallWTrans", "");
                }
                "WallNW" => {
                    flags!(collideN, cutN, collideW, cutW);
                    s.tile_type = ty.wall;
                    s.cut_w = true;
                    s.cut_n = true;
                    setr!("WallNW", "");
                }
                "WallNWTrans" => {
                    flags!(collideN, cutN, collideW, transparentN, transparentW, cutW);
                    s.tile_type = ty.wall;
                    s.cut_w = true;
                    s.cut_n = true;
                    setr!("WallNWTrans", "");
                }
                "WallSE" => {
                    flags!(cutW, WallSE);
                    set!("WallSE", "WallSE");
                    s.cut_w = true;
                }
                "WindowW" => {
                    flags!(canPathW, collideW, cutW, transparentW);
                    s.tile_type = ty.windowFW;
                    if s.properties.has(f.HoppableW) {
                        s.properties.unset_flag(f.HoppableW);
                    }
                    s.cut_w = true;
                }
                "WindowN" => {
                    flags!(canPathN, collideN, cutN, transparentN);
                    s.tile_type = ty.windowFN;
                    if s.properties.has(f.HoppableN) {
                        s.properties.unset_flag(f.HoppableN);
                    }
                    s.cut_n = true;
                }
                "UnbreakableWindowW" => {
                    flags!(canPathW, collideW, cutW, transparentW, collideW);
                    s.tile_type = ty.wall;
                    s.cut_w = true;
                }
                "UnbreakableWindowN" => {
                    flags!(canPathN, collideN, cutN, transparentN, collideN);
                    s.tile_type = ty.wall;
                    s.cut_n = true;
                }
                "UnbreakableWindowNW" => {
                    flags!(cutN, transparentN, collideN, cutN, collideW, cutW);
                    s.tile_type = ty.wall;
                    s.cut_w = true;
                    s.cut_n = true;
                }
                "NoWallLighting" => flags!(NoWallLighting),
                "ForceAmbient" => flags!(ForceAmbient),
                _ => {}
            }
        }
        if prop == "name" {
            s.parent_object_name = Some(val);
        }
        Ok(())
    }

    /// `IsoWorld.setOpenDoorProperties`.
    fn set_open_door_properties(&mut self, tilesheet_name: &str, sprites: &[SpriteIdx]) {
        let (t, a) = (&self.tables, &self.alias);
        for &si in sprites {
            let s = &self.sprites[si];
            let tt = s.tile_type;
            if (tt != t.ty.doorN && tt != t.ty.doorW) || s.properties.has(t.flag.open) {
                continue;
            }
            if let Some(dd) = s.properties.get(a, "DoubleDoor") {
                if try_parse_int(dd, -1) >= 5 {
                    self.sprites[si].properties.set_flag(t.flag.open);
                }
                continue;
            }
            if let Some(gd) = s.properties.get(a, "GarageDoor") {
                if try_parse_int(gd, -1) >= 4 {
                    self.sprites[si].properties.set_flag(t.flag.open);
                }
                continue;
            }
            let open_name = format!("{tilesheet_name}_{}", s.tile_sheet_index + 2);
            let Some(&o) = self.named_map.get(&open_name) else {
                continue;
            };
            let o = &mut self.sprites[o];
            o.tile_type = tt;
            o.properties.set_flag(if tt == t.ty.doorN {
                t.flag.doorN
            } else {
                t.flag.doorW
            });
            o.properties.set_flag(t.flag.open);
        }
    }

    /// The movable/sprite-grid pass over one tileset's sprites in
    /// `LoadTileDefinitions`. Groups are independent of each other, so the
    /// `groupMap` iteration order does not matter.
    fn apply_movables(&mut self, buffer: &[SpriteIdx]) -> io::Result<()> {
        let mut groups: Vec<(String, Vec<SpriteIdx>)> = Vec::new();
        let mut add = |g: String, s: SpriteIdx| match groups.iter_mut().find(|(k, _)| *k == g) {
            Some((_, v)) => v.push(s),
            None => groups.push((g, vec![s])),
        };
        for &si in buffer {
            let (t, a) = (&self.tables, &self.alias);
            let s = &mut self.sprites[si];
            if s.properties.has_name(a, "StopCar") && s.tile_type != t.ty.tree {
                s.tile_type = t.ty.isMoveAbleObject;
            }
            let p = &s.properties;
            let custom = p.get(a, "CustomName");
            if p.has_name(a, "IsMoveAble") {
                let Some(custom) = custom.filter(|c| !c.is_empty()) else {
                    continue;
                };
                if let Some(group) = p.get(a, "GroupName") {
                    add(format!("{group} {custom}"), si);
                }
                continue;
            }
            if !p.has_name(a, "SpriteGridPos") {
                continue;
            }
            if is_null_or_whitespace(custom) {
                continue;
            }
            if let Some(group) = p.get(a, "GroupName") {
                add(format!("{group} {}", custom.unwrap_or_default()), si);
            }
        }
        for (_, members) in groups {
            self.apply_group(buffer, &members)?;
        }
        Ok(())
    }

    fn apply_group(&mut self, buffer: &[SpriteIdx], members: &[SpriteIdx]) -> io::Result<()> {
        let a = &self.alias;
        let is_grid = |si: SpriteIdx| {
            self.sprites[si]
                .properties
                .get(a, "SpriteGridPos")
                .is_some_and(|v| v != "None")
        };
        let mut faces: [Vec<SpriteIdx>; 4] = Default::default();
        let first_grid = is_grid(members[0]);
        let mut valid = true;
        for &cur in members {
            if is_grid(cur) != first_grid {
                valid = false;
                break;
            }
            match self.sprites[cur].properties.get(a, "Facing") {
                Some("N") => faces[0].push(cur),
                Some("E") => faces[1].push(cur),
                Some("S") => faces[2].push(cur),
                Some("W") => faces[3].push(cur),
                _ => valid = false,
            }
            if !valid {
                break;
            }
        }
        if !valid {
            return Ok(());
        }
        if !first_grid {
            if members.len() > 4 || faces.iter().any(|f| f.len() > 1) {
                return Ok(());
            }
            let index_of =
                |s: SpriteIdx| buffer.iter().position(|&b| b == s).map_or(-1, |i| i as i32);
            for &cur in members {
                for (fi, face) in faces.iter().enumerate() {
                    if face.is_empty() || face[0] == cur {
                        continue;
                    }
                    let off = (index_of(face[0]) - index_of(cur)).to_string();
                    let key = format!("{}offset", FACE_NAMES[fi]);
                    self.sprites[cur]
                        .properties
                        .set(&self.tables, &self.alias, &key, &off, true);
                }
            }
            return Ok(());
        }

        let mut spr_count = 0usize;
        let mut grids: [Option<IsoSpriteGrid>; 4] = Default::default();
        for i in 0..4 {
            let dir = &faces[i];
            if dir.is_empty() {
                continue;
            }
            if spr_count == 0 {
                spr_count = dir.len();
            }
            if spr_count != dir.len() {
                valid = false;
                break;
            }
            let mut levels: Vec<GridLevel> = Vec::new();
            let (mut width, mut height) = (0i32, 0i32);
            for &mem in dir {
                let p = &self.sprites[mem].properties;
                let pos = p.get(a, "SpriteGridPos").unwrap_or_default();
                let parts = java_split(pos, ',');
                if parts.len() < 2 || parts.len() > 3 {
                    valid = false;
                    break;
                }
                let x = parse_int(parts[0])?;
                let y = parse_int(parts[1])?;
                let mut z = 0;
                if parts.len() == 3 {
                    z = parse_int(parts[2])?;
                }
                if let Some(level) = p.get(a, "SpriteGridLevel") {
                    z = parse_int(level)?;
                    if z < 0 {
                        valid = false;
                        break;
                    }
                }
                // `getOrCreateLevel(z)`.
                while levels.len() as i64 <= z as i64 {
                    let z1 = levels.len() as i32;
                    levels.push(GridLevel {
                        z: z1,
                        ..Default::default()
                    });
                }
                let level = levels
                    .get_mut(usize::try_from(z).map_err(|_| {
                        io::Error::other(format!("IndexOutOfBoundsException: level {z}"))
                    })?)
                    .expect("level");
                if level.xy_to_sprite.contains_key(pos) {
                    valid = false;
                    break;
                }
                level.keys.push(pos.to_string());
                level.xy_to_sprite.insert(pos.to_string(), mem);
                level.width = level.width.max(x.wrapping_add(1));
                level.height = level.height.max(y.wrapping_add(1));
                width = width.max(level.width);
                height = height.max(level.height);
            }
            if !valid {
                break;
            }
            // `SpriteGridParseData.isValid()`.
            if width <= 0
                || height <= 0
                || levels.is_empty()
                || levels
                    .iter()
                    .any(|l| l.width <= 0 || l.height <= 0 || l.xy_to_sprite.is_empty())
            {
                valid = false;
                break;
            }
            let cells = (width as i64) * (height as i64) * (levels.len() as i64);
            if cells > i32::MAX as i64 {
                return Err(io::Error::other("sprite grid too large"));
            }
            let mut grid = IsoSpriteGrid::new(width, height, levels.len() as i32);
            for level in &levels {
                let keys: Vec<&str> = level.keys.iter().map(|k| k.as_str()).collect();
                for key in hashmap_order(&keys) {
                    let parts = java_split(key, ',');
                    let x = parse_int(parts[0])?;
                    let y = parse_int(parts[1])?;
                    grid.set_sprite(x, y, level.z, level.xy_to_sprite[key]);
                }
            }
            if !grid.validate() {
                valid = false;
                break;
            }
            grids[i] = Some(grid);
        }
        if !valid || spr_count == 0 {
            return Ok(());
        }
        let anchors: [Option<i32>; 4] = std::array::from_fn(|j| {
            grids[j]
                .as_ref()
                .map(|g| self.sprites[g.anchor_sprite().expect("validated grid")].tile_sheet_index)
        });
        let mut ids: [Option<GridIdx>; 4] = [None; 4];
        for (i, g) in grids.into_iter().enumerate() {
            if let Some(g) = g {
                self.grids.push(g);
                ids[i] = Some(self.grids.len() - 1);
            }
        }
        for (i, gi) in ids.iter().enumerate() {
            let Some(gi) = *gi else { continue };
            let members: Vec<SpriteIdx> =
                self.grids[gi].sprites.iter().flatten().copied().collect();
            for member in members {
                let s = &mut self.sprites[member];
                s.sprite_grid = Some(gi);
                for (j, anchor) in anchors.iter().enumerate() {
                    let Some(anchor) = anchor.filter(|_| j != i) else {
                        continue;
                    };
                    let off = (anchor - s.tile_sheet_index).to_string();
                    let key = format!("{}offset", FACE_NAMES[j]);
                    s.properties
                        .set(&self.tables, &self.alias, &key, &off, true);
                }
            }
        }
        Ok(())
    }
}
