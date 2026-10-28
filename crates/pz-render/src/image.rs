//! CPU-side texture data: `zombie.core.textures.ImageData` (power-of-two
//! RGBA buffers, mip maps, premultiplied alpha), `zombie.core.utils.BooleanGrid`
//! and `zombie.core.textures.Mask` (per-pixel hit masks).

use std::io::{self, Read};
use std::rc::Rc;

use crate::png::PngDecoder;

/// `PZMath.smallestEncompassingPowerOfTwo`.
pub fn pow2(n: i32) -> i32 {
    if n <= 1 {
        1
    } else {
        1i32.wrapping_shl(32 - (n - 1).leading_zeros())
    }
}

/// `zombie.core.utils.BooleanGrid`: a bit grid; out-of-range reads are false
/// and out-of-range writes are ignored.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BooleanGrid {
    bit_width: i32,
    width: i32,
    height: i32,
    value: Vec<u32>,
}

impl BooleanGrid {
    pub fn new(width: i32, height: i32) -> Self {
        let words = width / 32 + i32::from(width % 32 != 0);
        let len = (words.max(0) as usize) * (height.max(0) as usize);
        BooleanGrid { bit_width: width, width: words, height, value: vec![0; len] }
    }

    fn index(&self, x: i32, y: i32) -> Option<usize> {
        if x >= self.bit_width || x < 0 || y >= self.height || y < 0 {
            return None;
        }
        let xx = x / 32;
        if xx >= self.width {
            return None;
        }
        Some((xx + y * self.width) as usize)
    }

    pub fn value(&self, x: i32, y: i32) -> bool {
        self.index(x, y).is_some_and(|i| self.value[i] & (1 << (x & 31)) != 0)
    }

    pub fn set_value(&mut self, x: i32, y: i32, v: bool) {
        if let Some(i) = self.index(x, y) {
            if v {
                self.value[i] |= 1 << (x & 31);
            } else {
                self.value[i] &= !(1 << (x & 31));
            }
        }
    }

    pub fn fill(&mut self) {
        self.value.fill(u32::MAX);
    }
}

/// `zombie.core.textures.Mask`. Pack sub-textures share their page's grid
/// through a sub-mask view.
#[derive(Clone, Debug)]
pub struct Mask {
    pub width: i32,
    pub height: i32,
    pub full: bool,
    grid: Rc<BooleanGrid>,
    sub: Option<(i32, i32)>,
}

impl Mask {
    /// `new Mask(ITexture, BooleanGrid)`: copies the top-left
    /// `width` x `height` of `grid`.
    pub fn copy_of(grid: &BooleanGrid, width: i32, height: i32) -> Self {
        let mut g = BooleanGrid::new(width, height);
        for y in 0..height {
            for x in 0..width {
                g.set_value(x, y, grid.value(x, y));
            }
        }
        Mask { width, height, full: false, grid: Rc::new(g), sub: None }
    }

    /// `new Mask(BooleanGrid, x, y, w, h)`: a view into a shared grid.
    pub fn sub_mask(grid: Rc<BooleanGrid>, x: i32, y: i32, width: i32, height: i32) -> Self {
        Mask { width, height, full: false, grid, sub: Some((x, y)) }
    }

    /// `Mask.get(x, y)`.
    pub fn get(&self, x: i32, y: i32) -> bool {
        if self.full {
            return true;
        }
        match self.sub {
            Some((ox, oy)) => {
                if x < 0 || x >= self.width || y < 0 || y >= self.height {
                    return false;
                }
                self.grid.value(ox + x, oy + y)
            }
            None => self.grid.value(x, y),
        }
    }
}

/// One mip level: `width * height` RGBA pixels.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MipLevel {
    pub width: i32,
    pub height: i32,
    pub pixels: Vec<u8>,
}

impl MipLevel {
    fn new(width: i32, height: i32) -> Self {
        MipLevel { width, height, pixels: vec![0; (width.max(0) * height.max(0) * 4) as usize] }
    }

    fn pixel_clamped(&self, x: i32, y: i32) -> [i32; 4] {
        let x = x.clamp(0, self.width - 1);
        let y = y.clamp(0, self.height - 1);
        let i = ((x + y * self.width) * 4) as usize;
        let p = &self.pixels[i..i + 4];
        [i32::from(p[0]), i32::from(p[1]), i32::from(p[2]), i32::from(p[3])]
    }

    fn set_pixel(&mut self, x: i32, y: i32, rgba: [i32; 4]) {
        let i = ((x + y * self.width) * 4) as usize;
        for (dst, v) in self.pixels[i..i + 4].iter_mut().zip(rgba) {
            *dst = (v & 0xFF) as u8;
        }
    }

    fn in_bounds(&self, x: i32, y: i32) -> bool {
        x >= 0 && x < self.width && y >= 0 && y < self.height
    }
}

/// `ImageData`: the image padded to power-of-two dimensions.
#[derive(Clone, Debug)]
pub struct ImageData {
    pub width: i32,
    pub height: i32,
    pub width_hw: i32,
    pub height_hw: i32,
    pub solid: bool,
    pub data: MipLevel,
    pub mask: Option<BooleanGrid>,
    /// `preserveTransparentColor`: keep RGB of transparent pixels in mips.
    pub preserve_transparent_color: bool,
}

impl ImageData {
    /// `new ImageData(width, height)`: a blank (zeroed) image.
    pub fn new(width: i32, height: i32) -> Self {
        let (width_hw, height_hw) = (pow2(width), pow2(height));
        ImageData { width, height, width_hw, height_hw, solid: true, data: MipLevel::new(width_hw, height_hw), mask: None, preserve_transparent_color: false }
    }

    /// `new ImageData(InputStream, doMask)`: decodes a PNG with the
    /// original's decoder into the top-left of a power-of-two buffer.
    pub fn from_png(input: impl Read, do_mask: bool) -> io::Result<Self> {
        let mut png = PngDecoder::new(input, do_mask)?;
        if png.width < 0 || png.height < 0 || png.width > 16384 || png.height > 16384 {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "image too large"));
        }
        let mut img = ImageData::new(png.width, png.height);
        let stride = 4 * img.width_hw as usize;
        png.decode_rgba(&mut img.data.pixels, stride)?;
        if do_mask {
            img.mask = png.mask.take();
        }
        Ok(img)
    }

    /// RGBA pixels already laid out `width_hw` wide.
    pub fn from_pixels(width: i32, height: i32, pixels: Vec<u8>) -> Self {
        let mut img = ImageData::new(width, height);
        assert_eq!(pixels.len(), img.data.pixels.len());
        img.data.pixels = pixels;
        img
    }

    /// `calculateNumMips(widthHW, heightHW)`.
    pub fn mip_count(&self) -> i32 {
        let n = |d: i32| 32 - (d.max(0) as u32).leading_zeros() as i32;
        n(self.width_hw).max(n(self.height_hw))
    }

    /// What `TextureID.generateHwId` uploads for a `MIPMAPS` texture: every
    /// level, premultiplied by alpha. Levels 1 and 2 average only the
    /// non-transparent texels; deeper levels average all four.
    pub fn mip_maps(&self) -> Vec<MipLevel> {
        let count = self.mip_count();
        let mut levels = vec![self.data.clone()];
        let next = |d: i32| if d > 1 { d >> 1 } else { d };
        let (mut w, mut h) = (next(self.width_hw), next(self.height_hw));
        for i in 0..count - 1 {
            let parent = levels.last().unwrap();
            let mut sub = MipLevel::new(w, h);
            for y in 0..h {
                for x in 0..w {
                    let p = if i < 2 { self.sample_max_alpha(parent, x, y) } else { sample_average(parent, x, y) };
                    sub.set_pixel(x, y, p);
                }
            }
            levels.push(sub);
            w = next(w);
            h = next(h);
        }
        for level in &mut levels {
            premultiply(level);
        }
        levels
    }

    /// `scaleMipLevelMaxAlpha` for one texel.
    fn sample_max_alpha(&self, parent: &MipLevel, x: i32, y: i32) -> [i32; 4] {
        let orig = parent.pixel_clamped(x * 2, y * 2);
        let (mut r, mut n) = if self.preserve_transparent_color || orig[3] > 0 { (orig, 1) } else { ([0; 4], 0) };
        for (dx, dy) in [(1, 0), (0, 1), (1, 1)] {
            let (sx, sy) = (x * 2 + dx, y * 2 + dy);
            if !parent.in_bounds(sx, sy) {
                continue;
            }
            let p = parent.pixel_clamped(sx, sy);
            if p[3] > 0 {
                for k in 0..4 {
                    r[k] += p[k];
                }
                n += 1;
            }
        }
        if n > 0 {
            for v in &mut r {
                *v /= n;
            }
        }
        r
    }
}

/// `scaleMipLevelAverage` for one texel.
fn sample_average(parent: &MipLevel, x: i32, y: i32) -> [i32; 4] {
    let mut r = parent.pixel_clamped(x * 2, y * 2);
    let mut n = 1;
    for (dx, dy) in [(1, 0), (0, 1), (1, 1)] {
        let (sx, sy) = (x * 2 + dx, y * 2 + dy);
        if parent.in_bounds(sx, sy) {
            let p = parent.pixel_clamped(sx, sy);
            for k in 0..4 {
                r[k] += p[k];
            }
            n += 1;
        }
    }
    for v in &mut r {
        *v /= n;
    }
    r
}

/// `performPreMultipliedAlpha`: `(int)((float)(c * a) / 255f)`.
fn premultiply(level: &mut MipLevel) {
    for px in level.pixels.as_chunks_mut::<4>().0 {
        let a = i32::from(px[3]);
        for c in &mut px[..3] {
            *c = ((i32::from(*c) * a) as f32 / 255.0) as i32 as u8;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pow2_matches_pzmath() {
        assert_eq!([pow2(0), pow2(1), pow2(2), pow2(3), pow2(64), pow2(65)], [1, 1, 2, 4, 64, 128]);
    }

    #[test]
    fn boolean_grid_bounds() {
        let mut g = BooleanGrid::new(40, 2);
        g.set_value(39, 1, true);
        g.set_value(40, 1, true);
        assert!(g.value(39, 1));
        assert!(!g.value(40, 1));
        assert!(!g.value(-1, 0));
        let m = Mask::sub_mask(Rc::new(g), 30, 0, 10, 2);
        assert!(m.get(9, 1));
        assert!(!m.get(10, 1));
    }

    #[test]
    fn mips_skip_transparent_texels_then_average() {
        let mut img = ImageData::new(4, 4);
        // Top-left 2x2: one opaque red texel, three transparent white ones.
        img.data.set_pixel(0, 0, [255, 0, 0, 255]);
        for (x, y) in [(1, 0), (0, 1), (1, 1)] {
            img.data.set_pixel(x, y, [255, 255, 255, 0]);
        }
        let mips = img.mip_maps();
        assert_eq!(mips.len(), 3);
        assert_eq!(&mips[1].pixels[0..4], &[255, 0, 0, 255]);
        assert_eq!(&mips[0].pixels[4..8], &[0, 0, 0, 0]);
    }
}
