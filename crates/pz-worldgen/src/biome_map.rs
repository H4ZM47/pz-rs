//! `zombie.iso.worldgen.maps.BiomeMap` / `BiomeRaster`: the per-cell
//! `biomemap_X_Y.png` images of a map, whose red channel names the biome of
//! each square (through `biome_map_config`) and whose green channel names
//! a zone.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::Path;

use pz_fs::GameFs;

use crate::defs::BiomeMapEntry;

/// `BiomeMap.Type`: the image band read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BiomeBand {
    Biome = 0,
    Zone = 1,
}

/// `BiomeRaster`: the first two channels of each pixel.
#[derive(Clone, Debug)]
pub struct BiomeRaster {
    pub width: usize,
    pub height: usize,
    /// `x * 2 + band + y * width * 2`.
    pixels: Vec<u8>,
}

impl BiomeRaster {
    /// Decodes a PNG the way `NativeImage` + `ImageData` see it: 8-bit
    /// RGBA, of which the red and green channels are kept.
    pub fn from_png(path: &Path) -> Result<Self, String> {
        let f = std::fs::File::open(path).map_err(|e| e.to_string())?;
        let mut dec = png::Decoder::new(std::io::BufReader::new(f));
        dec.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
        let mut reader = dec.read_info().map_err(|e| e.to_string())?;
        let mut buf = vec![0; reader.output_buffer_size()];
        let info = reader.next_frame(&mut buf).map_err(|e| e.to_string())?;
        let (w, h) = (info.width as usize, info.height as usize);
        let ch = match info.color_type {
            png::ColorType::Grayscale => 1,
            png::ColorType::GrayscaleAlpha => 2,
            png::ColorType::Rgb => 3,
            png::ColorType::Rgba => 4,
            png::ColorType::Indexed => return Err("palette not expanded".into()),
        };
        let mut pixels = vec![0u8; w * h * 2];
        for y in 0..h {
            for x in 0..w {
                let p = &buf[y * info.line_size + x * ch..];
                let (r, g) = if ch < 3 { (p[0], p[0]) } else { (p[0], p[1]) };
                pixels[x * 2 + y * w * 2] = r;
                pixels[x * 2 + 1 + y * w * 2] = g;
            }
        }
        Ok(BiomeRaster {
            width: w,
            height: h,
            pixels,
        })
    }

    /// `getSamples(x, y, w, h, band)`; `None` where the original throws.
    pub fn samples(
        &self,
        x: usize,
        y: usize,
        w: usize,
        h: usize,
        band: BiomeBand,
    ) -> Option<Vec<i32>> {
        if x + w > self.width || y + h > self.height {
            return None;
        }
        let mut out = Vec::with_capacity(w * h);
        for i in y..y + h {
            for j in x..x + w {
                out.push(self.pixels[j * 2 + band as usize + i * self.width * 2] as i32);
            }
        }
        Some(out)
    }
}

type RasterCache = HashMap<(i32, i32), Option<std::rc::Rc<BiomeRaster>>>;

/// `BiomeMap`: the pixel-value table and a cache of cell rasters.
pub struct BiomeMap {
    pub entries: HashMap<i32, BiomeMapEntry>,
    maps: Vec<String>,
    fs: std::rc::Rc<GameFs>,
    cache: RefCell<RasterCache>,
}

impl BiomeMap {
    /// `new BiomeMap()` for the maps of `IsoWorld.getMap()` (split on
    /// `;`), searched in order.
    pub fn new(
        fs: std::rc::Rc<GameFs>,
        maps: &[&str],
        entries: HashMap<i32, BiomeMapEntry>,
    ) -> Self {
        BiomeMap {
            entries,
            maps: maps.iter().map(|s| s.to_string()).collect(),
            fs,
            cache: RefCell::new(HashMap::new()),
        }
    }

    /// `getRaster(cell)`: the first map's `biomemap_X_Y.png`, or `None`.
    pub fn raster(&self, cx: i32, cy: i32) -> Option<std::rc::Rc<BiomeRaster>> {
        if let Some(r) = self.cache.borrow().get(&(cx, cy)) {
            return r.clone();
        }
        let mut r = None;
        for m in &self.maps {
            let rel = format!("media/maps/{m}/maps/biomemap_{cx}_{cy}.png");
            if let Some(p) = self.fs.resolve(&rel).filter(|p| p.exists()) {
                r = BiomeRaster::from_png(&p).ok().map(std::rc::Rc::new);
                break;
            }
        }
        self.cache.borrow_mut().insert((cx, cy), r.clone());
        r
    }

    /// `getZones(chunkX, chunkY, type)`: the chunk's 64 pixel values
    /// (`[y * 8 + x]`), or `None`.
    pub fn zones(&self, chunk_x: i32, chunk_y: i32, band: BiomeBand) -> Option<Vec<i32>> {
        let cell_x = chunk_x / 32;
        let cell_y = chunk_y / 32;
        let (dx, dy) = (chunk_x - cell_x * 32, chunk_y - cell_y * 32);
        if dx < 0 || dy < 0 {
            return None;
        }
        let r = self.raster(cell_x, cell_y)?;
        r.samples(dx as usize * 8, dy as usize * 8, 8, 8, band)
    }

    /// `getEntry(pixel)`.
    pub fn entry(&self, pixel: i32) -> Option<&BiomeMapEntry> {
        self.entries.get(&pixel)
    }
}
