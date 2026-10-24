//! `MapFiles`, `LotHeader` and the `map.info` subset of `ChooseGameInfo.Map`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use pz_fs::GameFs;

use crate::java::{fastfloor, parse_boolean};

pub const LOTHEADER_MAGIC: [u8; 4] = *b"LOTH";
pub const LOTPACK_MAGIC: [u8; 4] = *b"LOTP";

/// `ChooseGameInfo.Map`: the fields map loading uses.
#[derive(Clone, Debug, Default)]
pub struct MapInfo {
    pub dir: PathBuf,
    pub title: String,
    pub lots_dir: Vec<String>,
    pub desc: Option<String>,
    pub fixed2x: bool,
    pub zoom_x: f32,
    pub zoom_y: f32,
    pub zoom_s: f32,
    pub demo_video: Option<String>,
}

impl MapInfo {
    /// `ChooseGameInfo.getMapDetails(dir)` minus thumbnails and translations.
    pub fn load(fs: &GameFs, dir: &str) -> Option<MapInfo> {
        let file = fs.resolve(&format!("media/maps/{dir}/map.info"))?;
        if !file.is_file() {
            return None;
        }
        let text = std::fs::read(&file).ok()?;
        Some(Self::parse(
            dir,
            file.parent().unwrap_or(Path::new("")),
            &String::from_utf8_lossy(&text),
        ))
    }

    pub fn parse(dir: &str, map_dir: &Path, text: &str) -> MapInfo {
        let mut map = MapInfo {
            dir: map_dir.to_path_buf(),
            title: dir.to_string(),
            ..Default::default()
        };
        // BufferedReader.readLine splits on \n, \r or \r\n.
        for line in text.split('\n').flat_map(|l| l.split('\r')) {
            let line = crate::java::java_trim(line);
            if line.starts_with("title=") {
                map.title = line.replace("title=", "");
            } else if line.starts_with("lots=") {
                map.lots_dir
                    .push(crate::java::java_trim(&line.replace("lots=", "")).to_string());
            } else if line.starts_with("description=") {
                let d = map.desc.get_or_insert_with(String::new);
                d.push_str(&line.replace("description=", ""));
            } else if line.starts_with("fixed2x=") {
                map.fixed2x = parse_boolean(crate::java::java_trim(&line.replace("fixed2x=", "")));
            } else if line.starts_with("zoomX=") {
                map.zoom_x = parse_float(&line.replace("zoomX=", "")).unwrap_or(map.zoom_x);
            } else if line.starts_with("zoomY=") {
                map.zoom_y = parse_float(&line.replace("zoomY=", "")).unwrap_or(map.zoom_y);
            } else if line.starts_with("zoomS=") {
                map.zoom_s = parse_float(&line.replace("zoomS=", "")).unwrap_or(map.zoom_s);
            } else if line.starts_with("demoVideo=") {
                map.demo_video = Some(line.replace("demoVideo=", ""));
            }
        }
        map
    }
}

fn parse_float(s: &str) -> Option<f32> {
    crate::java::java_trim(s).parse().ok()
}

/// `MapFiles.getLotDirectories(Core.gameMap)`: a `;`-separated list is taken
/// as-is, otherwise the map and its `lots=` dependencies, depth first.
pub fn lot_directories(fs: &GameFs, game_map: &str) -> Vec<String> {
    let mut result = Vec::new();
    if game_map.contains(';') {
        for s in game_map.split(';') {
            let lot_dir = crate::java::java_trim(s);
            if lot_dir.is_empty() || result.iter().any(|r| r == lot_dir) {
                continue;
            }
            result.push(lot_dir.to_string());
        }
    } else {
        collect_lot_directories(fs, game_map, &mut result);
    }
    result
}

fn collect_lot_directories(fs: &GameFs, map_name: &str, result: &mut Vec<String>) {
    if result.iter().any(|r| r == map_name) {
        return;
    }
    let Some(info) = MapInfo::load(fs, map_name) else {
        return;
    };
    result.push(map_name.to_string());
    for lot_dir in &info.lots_dir {
        collect_lot_directories(fs, lot_dir, result);
    }
}

/// `LotHeader`.
#[derive(Clone, Debug)]
pub struct LotHeader {
    pub cell_x: i32,
    pub cell_y: i32,
    pub width: i32,
    pub height: i32,
    pub min_level: i32,
    pub max_level: i32,
    pub version: i32,
    pub fixed2x: bool,
    pub tiles_used: Vec<String>,
    zombie_intensity: Box<[u8; 1024]>,
    /// Index of the owning [`MapFiles`] (its priority).
    pub map_files: usize,
    pub file_name: String,
    pub absolute_file_path: PathBuf,
    /// Indexed by `IsoDirections` ordinal.
    pub adjacent_cells: [bool; 8],
}

impl LotHeader {
    pub fn new(cell_x: i32, cell_y: i32) -> Self {
        LotHeader {
            cell_x,
            cell_y,
            width: 0,
            height: 0,
            min_level: 0,
            max_level: 0,
            version: 0,
            fixed2x: false,
            tiles_used: Vec::new(),
            zombie_intensity: Box::new([0; 1024]),
            map_files: 0,
            file_name: String::new(),
            absolute_file_path: PathBuf::new(),
            adjacent_cells: [false; 8],
        }
    }

    pub fn get_num_levels(&self) -> i32 {
        self.max_level - self.min_level + 1
    }

    pub fn get_zombie_intensity(&self, i: usize) -> u8 {
        self.zombie_intensity[i]
    }

    pub fn set_zombie_intensity(&mut self, i: usize, v: u8) {
        self.zombie_intensity[i] = v;
    }
}

/// `MapFiles`: one map directory's lot files.
#[derive(Debug)]
pub struct MapFiles {
    pub map_directory_name: String,
    pub map_directory_absolute_path: PathBuf,
    /// Position in the lot directory list, counting folders that were skipped
    /// because they do not exist. Used as an index into `IsoLot.MapFiles`.
    pub priority: usize,
    /// Position in `IsoLot.MapFiles` (stands in for the object reference).
    pub index: usize,
    pub min_x: i32,
    pub min_y: i32,
    pub max_x: i32,
    pub max_y: i32,
    pub info_headers: HashMap<String, LotHeader>,
    /// Header file names in directory-listing order (`infoHeaderNames`).
    pub info_header_names: Vec<String>,
    pub info_file_names: HashMap<String, PathBuf>,
    pub info_file_modded: HashMap<String, bool>,
    bg_has_cell: Vec<bool>,
    pub min_cell300_x: i32,
    pub min_cell300_y: i32,
    pub max_cell300_x: i32,
    pub max_cell300_y: i32,
    bg_has_cell300: Vec<bool>,
}

impl MapFiles {
    pub fn new(name: &str, absolute_path: PathBuf, priority: usize, index: usize) -> Self {
        MapFiles {
            map_directory_name: name.to_string(),
            map_directory_absolute_path: absolute_path,
            priority,
            index,
            min_x: i32::MAX,
            min_y: i32::MAX,
            max_x: i32::MIN,
            max_y: i32::MIN,
            info_headers: HashMap::new(),
            info_header_names: Vec::new(),
            info_file_names: HashMap::new(),
            info_file_modded: HashMap::new(),
            bg_has_cell: Vec::new(),
            min_cell300_x: 0,
            min_cell300_y: 0,
            max_cell300_x: 0,
            max_cell300_y: 0,
            bg_has_cell300: Vec::new(),
        }
    }

    pub fn get_width_in_cells(&self) -> i32 {
        self.max_x - self.min_x + 1
    }

    pub fn get_height_in_cells(&self) -> i32 {
        self.max_y - self.min_y + 1
    }

    fn create_lot_header(&mut self, fixed2x: bool, file_name: &str) -> Option<LotHeader> {
        // split("_"), then parseInt(trim) of the first two parts.
        let mut split = file_name.split('_');
        let x: i32 = crate::java::java_trim(split.next()?).parse().ok()?;
        let y: i32 = crate::java::java_trim(&split.next()?.replace(".lotheader", ""))
            .parse()
            .ok()?;
        self.min_x = self.min_x.min(x);
        self.min_y = self.min_y.min(y);
        self.max_x = self.max_x.max(x);
        self.max_y = self.max_y.max(y);
        let mut h = LotHeader::new(x, y);
        h.fixed2x = fixed2x;
        h.map_files = self.index;
        h.file_name = file_name.to_string();
        h.absolute_file_path = self.map_directory_absolute_path.join(file_name);
        Some(h)
    }

    /// `load()`: index the directory's `.lotheader`, `.lotpack` and
    /// `chunkdata_*` files. `fixed2x` comes from the map's `map.info`.
    pub fn load(&mut self, fixed2x: bool, modded: bool) -> bool {
        let Ok(rd) = std::fs::read_dir(&self.map_directory_absolute_path) else {
            return false;
        };
        // `File.list()` order (directory order) is kept.
        let names: Vec<String> = rd
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        for file_name in names {
            if file_name.ends_with(".lotheader") {
                let Some(info) = self.create_lot_header(fixed2x, &file_name) else {
                    continue;
                };
                self.info_file_names
                    .insert(file_name.clone(), info.absolute_file_path.clone());
                self.info_file_modded.insert(file_name.clone(), modded);
                self.info_headers.insert(file_name.clone(), info);
                self.info_header_names.push(file_name);
            } else if file_name.ends_with(".lotpack") || file_name.starts_with("chunkdata_") {
                self.info_file_names.insert(
                    file_name.clone(),
                    self.map_directory_absolute_path.join(&file_name),
                );
                self.info_file_modded.insert(file_name, modded);
            }
        }
        true
    }

    pub fn post_load(&mut self) {
        if self.min_x > self.max_x {
            self.max_x = 0;
            self.min_x = 0;
            self.max_y = 0;
            self.min_y = 0;
        }
        let (w, h) = (self.get_width_in_cells(), self.get_height_in_cells());
        self.bg_has_cell = vec![false; (w * h) as usize];
        for y in 0..h {
            for x in 0..w {
                let key = format!("{}_{}.lotheader", self.min_x + x, self.min_y + y);
                self.bg_has_cell[(x + y * w) as usize] = self.info_headers.contains_key(&key);
            }
        }
        self.min_cell300_x = (self.min_x as f32 * 256.0 / 300.0).floor() as i32;
        self.min_cell300_y = (self.min_y as f32 * 256.0 / 300.0).floor() as i32;
        self.max_cell300_x = ((self.max_x + 1) as f32 * 256.0 / 300.0).floor() as i32;
        self.max_cell300_y = ((self.max_y + 1) as f32 * 256.0 / 300.0).floor() as i32;
        let w300 = self.max_cell300_x - self.min_cell300_x + 1;
        let h300 = self.max_cell300_y - self.min_cell300_y + 1;
        self.bg_has_cell300 = vec![false; (w300 * h300) as usize];
        for cy in self.min_cell300_y..=self.max_cell300_y {
            for cx in self.min_cell300_x..=self.max_cell300_x {
                // `Math.floor((float)a * 300.0f / 256.0f)`: float multiply, then double floor.
                let c256x = (cx as f32 * 300.0 / 256.0).floor() as i32;
                let c256y = (cy as f32 * 300.0 / 256.0).floor() as i32;
                if !self.has_cell(c256x, c256y) || !self.has_cell(c256x + 1, c256y + 1) {
                    continue;
                }
                self.bg_has_cell300
                    [((cx - self.min_cell300_x) + (cy - self.min_cell300_y) * w300) as usize] =
                    true;
            }
        }
    }

    pub fn is_valid_cell_pos(&self, cell_x: i32, cell_y: i32) -> bool {
        cell_x >= self.min_x && cell_y >= self.min_y && cell_x <= self.max_x && cell_y <= self.max_y
    }

    pub fn has_cell(&self, cell_x: i32, cell_y: i32) -> bool {
        if !self.is_valid_cell_pos(cell_x, cell_y) {
            return false;
        }
        let w = self.get_width_in_cells();
        self.bg_has_cell
            .get(((cell_x - self.min_x) + (cell_y - self.min_y) * w) as usize)
            .copied()
            .unwrap_or(false)
    }

    pub fn has_cell300(&self, cx: i32, cy: i32) -> bool {
        let w = self.max_cell300_x - self.min_cell300_x + 1;
        let h = self.max_cell300_y - self.min_cell300_y + 1;
        let (x, y) = (cx - self.min_cell300_x, cy - self.min_cell300_y);
        if x < 0 || y < 0 || x >= w || y >= h {
            return false;
        }
        self.bg_has_cell300[(x + y * w) as usize]
    }

    pub fn get_lot_header(&self, cell_x: i32, cell_y: i32) -> Option<&LotHeader> {
        if !self.has_cell(cell_x, cell_y) {
            return None;
        }
        self.info_headers.get(&header_name(cell_x, cell_y))
    }

    pub fn get_lot_header_mut(&mut self, cell_x: i32, cell_y: i32) -> Option<&mut LotHeader> {
        if !self.has_cell(cell_x, cell_y) {
            return None;
        }
        self.info_headers.get_mut(&header_name(cell_x, cell_y))
    }
}

/// `ChunkMapFilenames.getHeader`.
pub fn header_name(cell_x: i32, cell_y: i32) -> String {
    format!("{cell_x}_{cell_y}.lotheader")
}

pub fn lotpack_name(cell_x: i32, cell_y: i32) -> String {
    format!("world_{cell_x}_{cell_y}.lotpack")
}

/// `LotHeader.getZombieIntensityForChunk`.
pub fn zombie_intensity_for_chunk(
    map_files: &[MapFiles],
    header: Option<&LotHeader>,
    chunk_x: i32,
    chunk_y: i32,
) -> i32 {
    if !(0..32).contains(&chunk_x) || !(0..32).contains(&chunk_y) {
        return -1;
    }
    let Some(header) = header else { return -1 };
    let start = map_files[header.map_files].priority;
    for mf in map_files.iter().skip(start) {
        let c300x = fastfloor((header.cell_x * 256 + chunk_x * 8) as f32 / 300.0);
        let c300y = fastfloor((header.cell_y * 256 + chunk_y * 8) as f32 / 300.0);
        if !mf.has_cell300(c300x, c300y) {
            continue;
        }
        // NPE upstream if the cell is missing from this map; treat as absent.
        let Some(h2) = mf.get_lot_header(header.cell_x, header.cell_y) else {
            return -1;
        };
        return h2.get_zombie_intensity((chunk_x + chunk_y * 32) as usize) as i32;
    }
    -1
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn map_info_fields() {
        let m = MapInfo::parse(
            "X",
            Path::new("/m"),
            "title=Town\r\nlots=Base \nfixed2x=TRUE\ndescription=a\ndescription=b\n",
        );
        assert_eq!(m.title, "Town");
        assert_eq!(m.lots_dir, vec!["Base"]);
        assert!(m.fixed2x);
        assert_eq!(m.desc.as_deref(), Some("ab"));
    }
}
