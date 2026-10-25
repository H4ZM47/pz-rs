//! The save thumbnail, `thumb.png`: `SavefileThumbnail.create` (run by
//! `GameWindow.save` on the game thread, after the `OnSave` event) renders
//! the first local player's view at zoom 1 and reads a 256x256 square from
//! the middle of it (`Core.TakeScreenshot`); `NativeImage.writePng` stores
//! it with `stbi_write_png` flipped vertically (OpenGL rows run bottom-up).
//!
//! The pixels come from the renderer. This module holds the save-side
//! logic: which player, the rectangle read back, and the PNG encoder, a
//! port of `stb_image_write`'s `stbi_write_png_to_mem` (filter chosen per
//! row by least absolute sum, its own fixed-Huffman deflate with
//! compression level 8, the stored-block fallback, Adler-32 and the chunk
//! CRCs), so the file has the original's bytes.

use std::io;
use std::path::Path;

/// `SavefileThumbnail.WIDTH` / `HEIGHT`.
pub const THUMB_SIZE: i32 = 256;
/// The file name in the save folder.
pub const THUMB_FILE: &str = "thumb.png";

/// `SavefileThumbnail.create()`: the first player slot with a player
/// (`IsoPlayer.players[n] != null` for `n < numPlayers`); `None` makes no
/// thumbnail.
pub fn thumbnail_player(players: &[bool]) -> Option<usize> {
    players.iter().position(|&p| p)
}

/// A screen rectangle `glReadPixels` reads (origin bottom-left, as GL).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ReadRect {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

/// The read-back of `TakeScreenShotDrawer.render` when the game has no
/// offscreen buffer: `Core.TakeScreenshot(256, 256, GL_BACK)`, the
/// middle of player 0's screen area (`IsoCamera.getScreenLeft/Top/Width/
/// Height(0)`), at most the area's size. With an offscreen buffer (the
/// usual case) the renderer draws the 256x256 square centred on the
/// player's area (`createWithRenderShader`, `sx = left + width / 2 - 128`)
/// into its own target and reads all of it: [`ReadRect`] `{0, 0, 256, 256}`.
pub fn screen_read_rect(left: i32, top: i32, screen_width: i32, screen_height: i32) -> ReadRect {
    let width = THUMB_SIZE.min(screen_width);
    let height = THUMB_SIZE.min(screen_height);
    ReadRect {
        x: left + screen_width / 2 - width / 2,
        y: top + screen_height / 2 - height / 2,
        width,
        height,
    }
}

/// The pixels the renderer read back: `width * height` RGBA texels, rows
/// bottom-up as `glReadPixels(.., GL_RGBA, GL_UNSIGNED_BYTE)` returns them.
#[derive(Clone, Debug, PartialEq)]
pub struct Thumbnail {
    pub width: i32,
    pub height: i32,
    pub rgba: Vec<u8>,
}

/// `NativeImage.writePng(save_dir/thumb.png, true)`. The renderer then
/// reloads the texture (`Texture.reload`).
pub fn save_thumbnail(save_dir: &Path, t: &Thumbnail) -> io::Result<()> {
    if t.width <= 0 || t.height <= 0 || t.rgba.len() != (t.width * t.height * 4) as usize {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Failed to save image: bad pixel buffer",
        ));
    }
    let png = stbi_write_png(&t.rgba, t.width as usize, t.height as usize, 4, true);
    std::fs::write(save_dir.join(THUMB_FILE), png)
}

/// `stbi_write_png_to_mem(pixels, width * n, width, height, n)` with
/// `stbi_flip_vertically_on_write(flip)`, the default compression level
/// (8) and no forced filter.
pub fn stbi_write_png(pixels: &[u8], width: usize, height: usize, n: usize, flip: bool) -> Vec<u8> {
    const CTYPE: [u8; 5] = [0, 0, 4, 2, 6];
    let stride = width * n;
    let row = stride + 1;
    let mut filt = vec![0u8; row * height];
    let mut line = vec![0u8; stride];
    for j in 0..height {
        let mut best_filter = 0;
        let mut best_val = i32::MAX;
        for filter in 0..5 {
            encode_png_line(pixels, stride, width, height, j, n, filter, flip, &mut line);
            let est: i32 = line.iter().map(|&b| (b as i8 as i32).abs()).sum();
            if est < best_val {
                best_val = est;
                best_filter = filter;
            }
        }
        encode_png_line(
            pixels,
            stride,
            width,
            height,
            j,
            n,
            best_filter,
            flip,
            &mut line,
        );
        filt[j * row] = best_filter as u8;
        filt[j * row + 1..(j + 1) * row].copy_from_slice(&line);
    }
    let zlib = zlib_compress(&filt, 8);
    let mut out = Vec::with_capacity(8 + 25 + 12 + zlib.len() + 12);
    out.extend_from_slice(&[137, 80, 78, 71, 13, 10, 26, 10]);
    let mut chunk = |tag: &[u8; 4], data: &[u8]| {
        out.extend_from_slice(&(data.len() as u32).to_be_bytes());
        let start = out.len();
        out.extend_from_slice(tag);
        out.extend_from_slice(data);
        let crc = crc32(&out[start..]);
        out.extend_from_slice(&crc.to_be_bytes());
    };
    let mut ihdr = Vec::with_capacity(13);
    ihdr.extend_from_slice(&(width as u32).to_be_bytes());
    ihdr.extend_from_slice(&(height as u32).to_be_bytes());
    ihdr.extend_from_slice(&[8, CTYPE[n], 0, 0, 0]);
    chunk(b"IHDR", &ihdr);
    chunk(b"IDAT", &zlib);
    chunk(b"IEND", &[]);
    out
}

/// `stbiw__paeth`.
fn paeth(a: i32, b: i32, c: i32) -> u8 {
    let p = a + b - c;
    let (pa, pb, pc) = ((p - a).abs(), (p - b).abs(), (p - c).abs());
    if pa <= pb && pa <= pc {
        a as u8
    } else if pb <= pc {
        b as u8
    } else {
        c as u8
    }
}

/// `stbiw__encode_png_line`: row `y` of the output with filter `filter`
/// (the first row uses the variants that read no previous row).
#[allow(clippy::too_many_arguments)]
fn encode_png_line(
    pixels: &[u8],
    stride: usize,
    width: usize,
    height: usize,
    y: usize,
    n: usize,
    filter: usize,
    flip: bool,
    line: &mut [u8],
) {
    const MAPPING: [usize; 5] = [0, 1, 2, 3, 4];
    const FIRSTMAP: [usize; 5] = [0, 1, 0, 5, 6];
    let t = if y != 0 {
        MAPPING[filter]
    } else {
        FIRSTMAP[filter]
    };
    let src_row = if flip { height - 1 - y } else { y };
    let z = &pixels[stride * src_row..stride * src_row + stride];
    // The previous output row (only read when y > 0).
    let prev_row = if y == 0 {
        src_row
    } else if flip {
        src_row + 1
    } else {
        src_row - 1
    };
    let p = &pixels[stride * prev_row..stride * prev_row + stride];
    let w = width * n;
    if t == 0 {
        line[..w].copy_from_slice(&z[..w]);
        return;
    }
    for i in 0..n {
        line[i] = match t {
            1 | 5 | 6 => z[i],
            2 => z[i].wrapping_sub(p[i]),
            3 => z[i].wrapping_sub(p[i] >> 1),
            4 => z[i].wrapping_sub(paeth(0, p[i] as i32, 0)),
            _ => unreachable!(),
        };
    }
    for i in n..w {
        line[i] = match t {
            1 => z[i].wrapping_sub(z[i - n]),
            2 => z[i].wrapping_sub(p[i]),
            3 => z[i].wrapping_sub(((z[i - n] as u32 + p[i] as u32) >> 1) as u8),
            4 => z[i].wrapping_sub(paeth(z[i - n] as i32, p[i] as i32, p[i - n] as i32)),
            5 => z[i].wrapping_sub(z[i - n] >> 1),
            6 => z[i].wrapping_sub(paeth(z[i - n] as i32, 0, 0)),
            _ => unreachable!(),
        };
    }
}

/// `stbiw__zhash`.
fn zhash(d: &[u8]) -> u32 {
    let mut h: u32 = d[0] as u32 + ((d[1] as u32) << 8) + ((d[2] as u32) << 16);
    h ^= h << 3;
    h = h.wrapping_add(h >> 5);
    h ^= h << 4;
    h = h.wrapping_add(h >> 17);
    h ^= h << 25;
    h = h.wrapping_add(h >> 6);
    h
}

/// `stbiw__zlib_countm`.
fn countm(data: &[u8], a: usize, b: usize, limit: usize) -> usize {
    let mut i = 0;
    while i < limit && i < 258 {
        if data[a + i] != data[b + i] {
            break;
        }
        i += 1;
    }
    i
}

/// `stbiw__zlib_bitrev`.
fn bitrev(mut code: u32, mut bits: u32) -> u32 {
    let mut res = 0;
    while bits > 0 {
        res = (res << 1) | (code & 1);
        code >>= 1;
        bits -= 1;
    }
    res
}

struct Bits {
    out: Vec<u8>,
    buf: u32,
    count: u32,
}

impl Bits {
    /// `stbiw__zlib_add` + flush.
    fn add(&mut self, code: u32, bits: u32) {
        self.buf |= code << self.count;
        self.count += bits;
        while self.count >= 8 {
            self.out.push(self.buf as u8);
            self.buf >>= 8;
            self.count -= 8;
        }
    }
    /// `stbiw__zlib_huff`: a fixed-Huffman literal/length code.
    fn huff(&mut self, n: u32) {
        if n <= 143 {
            self.add(bitrev(0x30 + n, 8), 8);
        } else if n <= 255 {
            self.add(bitrev(0x190 + n - 144, 9), 9);
        } else if n <= 279 {
            self.add(bitrev(n - 256, 7), 7);
        } else {
            self.add(bitrev(0xc0 + n - 280, 8), 8);
        }
    }
}

/// `stbi_zlib_compress(data, len, &out_len, quality)`.
pub fn zlib_compress(data: &[u8], quality: usize) -> Vec<u8> {
    const LENGTHC: [u32; 30] = [
        3, 4, 5, 6, 7, 8, 9, 10, 11, 13, 15, 17, 19, 23, 27, 31, 35, 43, 51, 59, 67, 83, 99, 115,
        131, 163, 195, 227, 258, 259,
    ];
    const LENGTHEB: [u32; 29] = [
        0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4, 5, 5, 5, 5, 0,
    ];
    const DISTC: [u32; 31] = [
        1, 2, 3, 4, 5, 7, 9, 13, 17, 25, 33, 49, 65, 97, 129, 193, 257, 385, 513, 769, 1025, 1537,
        2049, 3073, 4097, 6145, 8193, 12289, 16385, 24577, 32768,
    ];
    const DISTEB: [u32; 30] = [
        0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 8, 9, 9, 10, 10, 11, 11, 12, 12,
        13, 13,
    ];
    const ZHASH: usize = 16384;
    let quality = quality.max(5);
    let len = data.len() as i64;
    let mut table: Vec<Vec<usize>> = vec![Vec::new(); ZHASH];
    let mut b = Bits {
        out: vec![0x78, 0x5e],
        buf: 0,
        count: 0,
    };
    b.add(1, 1);
    b.add(1, 2);
    let mut i: i64 = 0;
    while i < len - 3 {
        let iu = i as usize;
        let h = zhash(&data[iu..]) as usize & (ZHASH - 1);
        let mut best = 3usize;
        let mut bestloc: Option<usize> = None;
        for &loc in &table[h] {
            if loc as i64 > i - 32768 {
                let d = countm(data, loc, iu, (len - i) as usize);
                if d >= best {
                    best = d;
                    bestloc = Some(loc);
                }
            }
        }
        if table[h].len() == 2 * quality {
            table[h].drain(..quality);
        }
        table[h].push(iu);
        if bestloc.is_some() {
            // Lazy matching: a longer match at the next byte makes this one
            // a literal.
            let h = zhash(&data[iu + 1..]) as usize & (ZHASH - 1);
            for &loc in &table[h] {
                if loc as i64 > i - 32767 {
                    let e = countm(data, loc, iu + 1, (len - i - 1) as usize);
                    if e > best {
                        bestloc = None;
                        break;
                    }
                }
            }
        }
        if let Some(loc) = bestloc {
            let d = (iu - loc) as u32;
            let best_u = best as u32;
            let mut j = 0;
            while best_u > LENGTHC[j + 1] - 1 {
                j += 1;
            }
            b.huff(j as u32 + 257);
            if LENGTHEB[j] != 0 {
                b.add(best_u - LENGTHC[j], LENGTHEB[j]);
            }
            let mut j = 0;
            while d > DISTC[j + 1] - 1 {
                j += 1;
            }
            b.add(bitrev(j as u32, 5), 5);
            if DISTEB[j] != 0 {
                b.add(d - DISTC[j], DISTEB[j]);
            }
            i += best as i64;
        } else {
            b.huff(data[iu] as u32);
            i += 1;
        }
    }
    while i < len {
        b.huff(data[i as usize] as u32);
        i += 1;
    }
    b.huff(256);
    while b.count != 0 {
        b.add(0, 1);
    }
    let mut out = b.out;
    // Stored blocks when compressing did not pay.
    let n = data.len();
    if out.len() > n + 2 + n.div_ceil(32767) * 5 {
        out.truncate(2);
        let mut j = 0;
        while j < n {
            let blocklen = (n - j).min(32767);
            out.push(u8::from(n - j == blocklen));
            out.push(blocklen as u8);
            out.push((blocklen >> 8) as u8);
            out.push(!blocklen as u8);
            out.push((!blocklen >> 8) as u8);
            out.extend_from_slice(&data[j..j + blocklen]);
            j += blocklen;
        }
    }
    let (mut s1, mut s2) = (1u32, 0u32);
    let mut blocklen = n % 5552;
    let mut j = 0;
    while j < n {
        for &v in &data[j..j + blocklen] {
            s1 += v as u32;
            s2 += s1;
        }
        s1 %= 65521;
        s2 %= 65521;
        j += blocklen;
        blocklen = 5552;
    }
    out.extend_from_slice(&[(s2 >> 8) as u8, s2 as u8, (s1 >> 8) as u8, s1 as u8]);
    out
}

/// `stbiw__crc32` (the PNG CRC).
fn crc32(data: &[u8]) -> u32 {
    let mut crc = !0u32;
    for &b in data {
        crc ^= b as u32;
        for _ in 0..8 {
            crc = if crc & 1 != 0 {
                (crc >> 1) ^ 0xedb8_8320
            } else {
                crc >> 1
            };
        }
    }
    !crc
}

#[cfg(test)]
mod tests;
