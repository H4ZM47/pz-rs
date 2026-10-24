//! `zombie.core.textures.PNGDecoder`, decoding to RGBA as `ImageData` asks.
//!
//! This is the decoder every shared texture goes through (loose `.png` files
//! and `.pack` pages), so its rules are kept rather than replaced by a
//! general PNG library:
//! * only 8-bit grey/RGB/grey-alpha/RGBA and 1/2/4/8-bit paletted images,
//!   non-interlaced, are accepted;
//! * `tRNS` keys compare one byte of each 16-bit sample (the low byte for
//!   RGB, the high byte for grey);
//! * the CRC is checked only for chunks read to the end;
//! * the hit mask (`CREATE_MASK`) is filled only for RGBA images (alpha != 0)
//!   and, with the original's indexing slip, for RGB images with a `tRNS` key.
//!
//! APNG frames are not decoded yet; animated files decode their default
//! image. The game's `media/` has no APNGs.

use std::io::{self, Read};

use miniz_oxide::inflate::stream::{inflate, InflateState};
use miniz_oxide::{DataFormat, MZError, MZFlush, MZStatus};

use crate::image::BooleanGrid;

const SIGNATURE: [u8; 8] = [137, 80, 78, 71, 13, 10, 26, 10];
const IHDR: i32 = 0x4948_4452;
const PLTE: i32 = 0x504C_5445;
const TRNS: i32 = 0x7452_4E53;
const IDAT: i32 = 0x4944_4154;
const ACTL: i32 = 0x6163_544C;
const FCTL: i32 = 0x6663_544C;
const FDAT: i32 = 0x6664_4154;

fn err(msg: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, msg.into())
}

/// `java.util.zip.CRC32`.
struct Crc32(u32);

impl Crc32 {
    fn table() -> &'static [u32; 256] {
        static TABLE: std::sync::OnceLock<[u32; 256]> = std::sync::OnceLock::new();
        TABLE.get_or_init(|| {
            let mut t = [0u32; 256];
            for (n, slot) in t.iter_mut().enumerate() {
                let mut c = n as u32;
                for _ in 0..8 {
                    c = if c & 1 != 0 { 0xEDB8_8320 ^ (c >> 1) } else { c >> 1 };
                }
                *slot = c;
            }
            t
        })
    }

    fn update(&mut self, data: &[u8]) {
        let t = Self::table();
        let mut c = !self.0;
        for &b in data {
            c = t[((c ^ u32::from(b)) & 0xFF) as usize] ^ (c >> 8);
        }
        self.0 = !c;
    }
}

pub struct PngDecoder<R: Read> {
    input: R,
    crc: Crc32,
    chunk_length: i32,
    chunk_type: i32,
    chunk_remaining: i32,
    pub width: i32,
    pub height: i32,
    bitdepth: u8,
    color_type: u8,
    bytes_per_pixel: usize,
    palette: Option<Vec<u8>>,
    palette_a: Option<Vec<u8>>,
    trans_pixel: Option<Vec<u8>>,
    pub num_frames: i32,
    do_mask: bool,
    mask_m: i32,
    mask_id: i32,
    pub mask: Option<BooleanGrid>,
}

fn be_int(b: &[u8]) -> i32 {
    i32::from_be_bytes([b[0], b[1], b[2], b[3]])
}

impl<R: Read> PngDecoder<R> {
    /// `new PNGDecoder(input, doMask)`: reads the header chunks up to the
    /// first `IDAT`.
    pub fn new(input: R, do_mask: bool) -> io::Result<Self> {
        let mut d = PngDecoder {
            input,
            crc: Crc32(0),
            chunk_length: 0,
            chunk_type: 0,
            chunk_remaining: 0,
            width: 0,
            height: 0,
            bitdepth: 0,
            color_type: 0,
            bytes_per_pixel: 0,
            palette: None,
            palette_a: None,
            trans_pixel: None,
            num_frames: 0,
            do_mask,
            mask_m: 0,
            mask_id: 0,
            mask: None,
        };
        let mut sig = [0u8; 8];
        d.read_fully(&mut sig)?;
        if sig != SIGNATURE {
            return Err(err("Not a valid PNG file"));
        }
        d.open_chunk_expect(IHDR)?;
        d.read_ihdr()?;
        d.close_chunk()?;
        loop {
            d.open_chunk()?;
            match d.chunk_type {
                IDAT => break,
                PLTE => d.read_plte()?,
                TRNS => d.read_trns()?,
                ACTL => d.read_actl()?,
                FCTL => {
                    let mut b = [0u8; 26];
                    d.check_chunk_length(26)?;
                    d.read_chunk(&mut b)?;
                }
                FDAT => {
                    if d.chunk_remaining < 4 {
                        return Err(err("Chunk has wrong size"));
                    }
                    let mut b = [0u8; 4];
                    d.read_chunk(&mut b)?;
                }
                _ => {}
            }
            d.close_chunk()?;
        }
        if d.color_type == 3 && d.palette.is_none() {
            return Err(err("Missing PLTE chunk"));
        }
        if do_mask {
            d.mask = Some(BooleanGrid::new(d.width, d.height));
        }
        Ok(d)
    }

    pub fn is_animated(&self) -> bool {
        self.num_frames > 0
    }

    fn read_fully(&mut self, buf: &mut [u8]) -> io::Result<()> {
        self.input.read_exact(buf).map_err(|e| if e.kind() == io::ErrorKind::UnexpectedEof { io::Error::from(io::ErrorKind::UnexpectedEof) } else { e })
    }

    fn open_chunk(&mut self) -> io::Result<()> {
        let mut h = [0u8; 8];
        self.read_fully(&mut h)?;
        self.chunk_length = be_int(&h[0..4]);
        self.chunk_type = be_int(&h[4..8]);
        self.chunk_remaining = self.chunk_length;
        self.crc = Crc32(0);
        self.crc.update(&h[4..8]);
        Ok(())
    }

    fn open_chunk_expect(&mut self, expected: i32) -> io::Result<()> {
        self.open_chunk()?;
        if self.chunk_type != expected {
            return Err(err(format!("Expected chunk: {expected:x}")));
        }
        Ok(())
    }

    /// `closeChunk`: skips an unread remainder without checking its CRC.
    fn close_chunk(&mut self) -> io::Result<()> {
        if self.chunk_remaining > 0 {
            let n = self.chunk_remaining as u64 + 4;
            let skipped = io::copy(&mut (&mut self.input).take(n), &mut io::sink())?;
            if skipped < n {
                return Err(io::ErrorKind::UnexpectedEof.into());
            }
        } else {
            let mut c = [0u8; 4];
            self.read_fully(&mut c)?;
            if be_int(&c) as u32 != self.crc.0 {
                return Err(err("Invalid CRC"));
            }
        }
        self.chunk_remaining = 0;
        self.chunk_length = 0;
        self.chunk_type = 0;
        Ok(())
    }

    fn check_chunk_length(&self, expected: i32) -> io::Result<()> {
        if self.chunk_length != expected {
            return Err(err("Chunk has wrong size"));
        }
        Ok(())
    }

    /// `readChunk`: reads up to the chunk's remaining bytes.
    fn read_chunk(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let len = buf.len().min(self.chunk_remaining.max(0) as usize);
        self.read_fully(&mut buf[..len])?;
        self.crc.update(&buf[..len]);
        self.chunk_remaining -= len as i32;
        Ok(len)
    }

    fn read_ihdr(&mut self) -> io::Result<()> {
        self.check_chunk_length(13)?;
        let mut b = [0u8; 13];
        self.read_chunk(&mut b)?;
        self.width = be_int(&b[0..4]);
        self.height = be_int(&b[4..8]);
        self.bitdepth = b[8];
        self.color_type = b[9];
        let bad_depth = || err(format!("Unsupported bit depth: {}", b[8]));
        self.bytes_per_pixel = match self.color_type {
            0 | 4 | 2 | 6 if self.bitdepth != 8 => return Err(bad_depth()),
            0 => 1,
            4 => 2,
            2 => 3,
            6 => 4,
            3 if matches!(self.bitdepth, 1 | 2 | 4 | 8) => 1,
            3 => return Err(bad_depth()),
            c => return Err(err(format!("unsupported color format: {c}"))),
        };
        if b[10] != 0 {
            return Err(err("unsupported compression method"));
        }
        if b[11] != 0 {
            return Err(err("unsupported filtering method"));
        }
        if b[12] != 0 {
            return Err(err("unsupported interlace method"));
        }
        Ok(())
    }

    fn read_plte(&mut self) -> io::Result<()> {
        let entries = self.chunk_length / 3;
        if !(1..=256).contains(&entries) || self.chunk_length % 3 != 0 {
            return Err(err("PLTE chunk has wrong length"));
        }
        let mut p = vec![0u8; (entries * 3) as usize];
        self.read_chunk(&mut p)?;
        self.palette = Some(p);
        Ok(())
    }

    fn read_trns(&mut self) -> io::Result<()> {
        match self.color_type {
            0 => {
                self.check_chunk_length(2)?;
                let mut t = vec![0u8; 2];
                self.read_chunk(&mut t)?;
                self.trans_pixel = Some(t);
            }
            2 => {
                self.check_chunk_length(6)?;
                let mut t = vec![0u8; 6];
                self.read_chunk(&mut t)?;
                self.trans_pixel = Some(t);
            }
            3 => {
                let n = self.palette.as_ref().ok_or_else(|| err("tRNS chunk without PLTE chunk"))?.len() / 3;
                let mut a = vec![0xFFu8; n];
                self.read_chunk(&mut a)?;
                self.palette_a = Some(a);
            }
            _ => {}
        }
        Ok(())
    }

    fn read_actl(&mut self) -> io::Result<()> {
        self.check_chunk_length(8)?;
        let mut b = [0u8; 8];
        self.read_chunk(&mut b)?;
        self.num_frames = be_int(&b[0..4]);
        Ok(())
    }

    /// `decode(buffer, stride, height, Format.RGBA, IDAT)`: writes `height`
    /// rows of RGBA pixels, row `y` at `y * stride`.
    pub fn decode_rgba(&mut self, out: &mut [u8], stride: usize) -> io::Result<()> {
        let width = self.width.max(0) as usize;
        let line_size = (width * self.bitdepth as usize).div_ceil(8) * self.bytes_per_pixel;
        let mut cur = vec![0u8; line_size + 1];
        let mut prev = vec![0u8; line_size + 1];
        let mut pal_line = if self.bitdepth < 8 { vec![0u8; width + 1] } else { Vec::new() };
        self.mask_m = 0;
        let mut inflater = Inflater { state: InflateState::new_boxed(DataFormat::Zlib), input: Vec::new(), consumed: 0 };
        for y in 0..self.height.max(0) as usize {
            self.read_chunk_unzip(&mut inflater, &mut cur)?;
            self.unfilter(&mut cur, &prev)?;
            let row = out.get_mut(y * stride..).ok_or_else(|| err("buffer overflow"))?;
            match self.color_type {
                2 => self.copy_rgb_to_rgba(row, &cur)?,
                6 => self.copy(row, &cur)?,
                0 => self.copy_grey_to_rgba(row, &cur)?,
                4 => copy_grey_alpha_to_rgba(row, &cur)?,
                3 => {
                    let line = match self.bitdepth {
                        8 => &cur,
                        4 => {
                            expand4(&cur, &mut pal_line);
                            &pal_line
                        }
                        2 => {
                            expand2(&cur, &mut pal_line);
                            &pal_line
                        }
                        1 => {
                            expand1(&cur, &mut pal_line);
                            &pal_line
                        }
                        _ => return Err(err("Unsupported bitdepth for this image")),
                    };
                    self.copy_pal_to_rgba(row, line)?;
                }
                _ => return Err(err("Not yet implemented")),
            }
            std::mem::swap(&mut cur, &mut prev);
        }
        self.close_chunk()
    }

    fn refill(&mut self, inf: &mut Inflater) -> io::Result<()> {
        while self.chunk_remaining == 0 {
            self.close_chunk()?;
            self.open_chunk_expect(IDAT)?;
        }
        inf.input.resize(4096, 0);
        let n = self.read_chunk(&mut inf.input)?;
        inf.input.truncate(n);
        inf.consumed = 0;
        Ok(())
    }

    /// `readChunkUnzip`: inflates exactly `buf.len()` bytes, pulling more
    /// `IDAT` data as the inflater runs dry.
    fn read_chunk_unzip(&mut self, inf: &mut Inflater, buf: &mut [u8]) -> io::Result<()> {
        let mut offset = 0;
        while offset < buf.len() {
            let res = inflate(&mut inf.state, &inf.input[inf.consumed..], &mut buf[offset..], MZFlush::None);
            inf.consumed += res.bytes_consumed;
            offset += res.bytes_written;
            match res.status {
                Ok(MZStatus::StreamEnd) if res.bytes_written == 0 => return Err(io::ErrorKind::UnexpectedEof.into()),
                Ok(_) | Err(MZError::Buf) => {
                    if res.bytes_written == 0 {
                        if inf.consumed >= inf.input.len() {
                            self.refill(inf)?;
                        } else if res.bytes_consumed == 0 {
                            return Err(err(format!("Can't inflate {} bytes", buf.len() - offset)));
                        }
                    }
                }
                Err(_) => return Err(err("inflate error")),
            }
        }
        Ok(())
    }

    fn unfilter(&self, cur: &mut [u8], prev: &[u8]) -> io::Result<()> {
        let bpp = self.bytes_per_pixel;
        let n = cur.len();
        match cur[0] {
            0 => {}
            1 => {
                for i in bpp + 1..n {
                    cur[i] = cur[i].wrapping_add(cur[i - bpp]);
                }
            }
            2 => {
                for i in 1..n {
                    cur[i] = cur[i].wrapping_add(prev[i]);
                }
            }
            3 => {
                for i in 1..=bpp.min(n - 1) {
                    cur[i] = cur[i].wrapping_add(prev[i] >> 1);
                }
                for i in bpp + 1..n {
                    cur[i] = cur[i].wrapping_add(((u32::from(prev[i]) + u32::from(cur[i - bpp])) >> 1) as u8);
                }
            }
            4 => {
                for i in 1..=bpp.min(n - 1) {
                    cur[i] = cur[i].wrapping_add(prev[i]);
                }
                for i in bpp + 1..n {
                    let a = i32::from(cur[i - bpp]);
                    let b = i32::from(prev[i]);
                    let c = i32::from(prev[i - bpp]);
                    let p = a + b - c;
                    let (pa, pb, pc) = ((p - a).abs(), (p - b).abs(), (p - c).abs());
                    let pred = if pa <= pb && pa <= pc { a } else if pb <= pc { b } else { c };
                    cur[i] = cur[i].wrapping_add(pred as u8);
                }
            }
            f => return Err(err(format!("invalide filter type in scanline: {}", f as i8))),
        }
        Ok(())
    }

    /// `copy`: RGBA rows are stored as is; the mask marks alpha != 0.
    fn copy(&mut self, out: &mut [u8], cur: &[u8]) -> io::Result<()> {
        if self.do_mask {
            let w = self.width;
            let mask = self.mask.as_mut().unwrap();
            for i in (1..cur.len()).step_by(4) {
                if cur.get(i + 3).is_some_and(|&a| i32::from(a as i8) % 255 != 0) {
                    mask.set_value(self.mask_m % w, self.mask_m / w, true);
                }
                self.mask_m += 1;
            }
        }
        put(out, 0, &cur[1..])
    }

    fn copy_rgb_to_rgba(&mut self, out: &mut [u8], cur: &[u8]) -> io::Result<()> {
        let mut o = 0;
        let trans = self.trans_pixel.as_ref().map(|t| (t[1], t[3], t[5]));
        for px in cur[1..].as_chunks::<3>().0 {
            let a = match trans {
                Some((tr, tg, tb)) if px[0] == tr && px[1] == tg && px[2] == tb => 0,
                _ => 0xFF,
            };
            if trans.is_some() && self.do_mask && a == 0 {
                // The original indexes by the count of transparent pixels.
                let w = self.width;
                self.mask.as_mut().unwrap().set_value(self.mask_id % w, self.mask_id / w, true);
                self.mask_id += 1;
            }
            put(out, o, &[px[0], px[1], px[2], a])?;
            o += 4;
        }
        Ok(())
    }

    fn copy_grey_to_rgba(&self, out: &mut [u8], cur: &[u8]) -> io::Result<()> {
        let key = self.trans_pixel.as_ref().map(|t| t[0]);
        for (i, &g) in cur[1..].iter().enumerate() {
            let a = if key == Some(g) { 0 } else { 0xFF };
            put(out, i * 4, &[g, g, g, a])?;
        }
        Ok(())
    }

    fn copy_pal_to_rgba(&self, out: &mut [u8], line: &[u8]) -> io::Result<()> {
        let pal = self.palette.as_ref().unwrap();
        let oob = || err("palette index out of bounds");
        for (i, &idx) in line[1..].iter().enumerate() {
            let k = usize::from(idx);
            let rgb = pal.get(k * 3..k * 3 + 3).ok_or_else(oob)?;
            let a = match &self.palette_a {
                Some(pa) => *pa.get(k).ok_or_else(oob)?,
                None => 0xFF,
            };
            put(out, i * 4, &[rgb[0], rgb[1], rgb[2], a])?;
        }
        Ok(())
    }
}

struct Inflater {
    state: Box<InflateState>,
    input: Vec<u8>,
    consumed: usize,
}

/// `ByteBuffer.put`: fails rather than writing past the end.
fn put(out: &mut [u8], at: usize, bytes: &[u8]) -> io::Result<()> {
    out.get_mut(at..at + bytes.len()).ok_or_else(|| err("buffer overflow"))?.copy_from_slice(bytes);
    Ok(())
}

fn copy_grey_alpha_to_rgba(out: &mut [u8], cur: &[u8]) -> io::Result<()> {
    for (i, ga) in cur[1..].as_chunks::<2>().0.iter().enumerate() {
        put(out, i * 4, &[ga[0], ga[0], ga[0], ga[1]])?;
    }
    Ok(())
}

/// `expand4/2/1`: unpacks sub-byte palette indices into `dst[1..]`.
fn expand_bits(src: &[u8], dst: &mut [u8], bits: usize) {
    let per_byte = 8 / bits;
    let mask = (1u8 << bits) - 1;
    for (k, d) in dst.iter_mut().enumerate().skip(1).map(|(i, d)| (i - 1, d)) {
        let val = src[1 + k / per_byte];
        let shift = 8 - bits * (k % per_byte + 1);
        *d = (val >> shift) & mask;
    }
}

fn expand4(src: &[u8], dst: &mut [u8]) {
    expand_bits(src, dst, 4);
}

fn expand2(src: &[u8], dst: &mut [u8]) {
    expand_bits(src, dst, 2);
}

fn expand1(src: &[u8], dst: &mut [u8]) {
    expand_bits(src, dst, 1);
}
