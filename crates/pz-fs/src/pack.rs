//! `.pack` texture archives: `zombie.fileSystem.TexturePackDevice` and the
//! integer/string readers of `zombie.core.textures.TexturePackPage`.
//!
//! Layout (little-endian): optional `"PZPK"` + version (1), then a page
//! count. Each page has a name, an entry count, a has-alpha flag, its
//! sub-texture entries, then a PNG. Version 0 PNGs end at the marker
//! `0xDEADBEEF`; version 1 PNGs are prefixed with their byte length.

use std::collections::HashMap;
use std::fs::File;
use std::io::{self, BufReader, Read, Seek, SeekFrom};

/// `TexturePackPage.SubTextureInfo`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SubTextureInfo {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
    /// Offset of the trimmed image inside the original frame.
    pub ox: i32,
    pub oy: i32,
    /// Size of the original (untrimmed) frame.
    pub fx: i32,
    pub fy: i32,
    pub name: String,
}

/// `TexturePackDevice.Page`.
#[derive(Clone, Debug)]
pub struct Page {
    pub name: String,
    pub has_alpha: bool,
    /// File offset of the PNG (for version 1, of its length prefix).
    pub png_start: u64,
    pub sub: Vec<SubTextureInfo>,
}

/// `FileSystem.SubTexture`: where a named sub-texture lives.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SubTexture {
    pub pack_name: String,
    pub page_name: String,
    pub info: SubTextureInfo,
}

/// `TexturePackDevice`.
#[derive(Debug)]
pub struct TexturePackDevice {
    pub name: String,
    pub filename: String,
    pub version: i32,
    pub pages: Vec<Page>,
    page_map: HashMap<String, usize>,
    /// Sub-texture name -> (page index, entry index); later pages win.
    sub_map: HashMap<String, (usize, usize)>,
    pub texture_flags: i32,
}

/// Reader state for `TexturePackPage.readInt`/`readIntByte`: the last four
/// bytes read (`chl1..chl4`), shared between calls as in the original.
struct PackReader<R: Read> {
    input: R,
    pos: u64,
    chl: [i32; 4],
}

impl<R: Read> PackReader<R> {
    /// `InputStream.read()`: a byte, or -1 at end of file.
    fn read_byte(&mut self) -> io::Result<i32> {
        let mut b = [0u8; 1];
        loop {
            match self.input.read(&mut b) {
                Ok(0) => return Ok(-1),
                Ok(_) => {
                    self.pos += 1;
                    return Ok(i32::from(b[0]));
                }
                Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                Err(e) => return Err(e),
            }
        }
    }

    /// `TexturePackPage.readInt(InputStream)`.
    fn read_int(&mut self) -> io::Result<i32> {
        let c = [self.read_byte()?, self.read_byte()?, self.read_byte()?, self.read_byte()?];
        self.chl = c;
        if (c[0] | c[1] | c[2] | c[3]) < 0 {
            return Err(io::ErrorKind::UnexpectedEof.into());
        }
        Ok(c[0].wrapping_add(c[1] << 8).wrapping_add(c[2] << 16).wrapping_add(c[3] << 24))
    }

    /// `TexturePackPage.readIntByte`: slides the four-byte window by one.
    fn read_int_byte(&mut self) -> io::Result<i32> {
        let c = [self.chl[1], self.chl[2], self.chl[3], self.read_byte()?];
        self.chl = c;
        if (c[0] | c[1] | c[2] | c[3]) < 0 {
            return Err(io::ErrorKind::UnexpectedEof.into());
        }
        Ok(c[0].wrapping_add(c[1] << 8).wrapping_add(c[2] << 16).wrapping_add(c[3] << 24))
    }

    /// `TexturePackPage.ReadString`: a length, then one byte per char
    /// (Latin-1); a byte read past the end becomes U+FFFF.
    fn read_string(&mut self) -> io::Result<String> {
        let size = self.read_int()?;
        let mut s = String::new();
        for _ in 0..size.max(0) {
            let b = self.read_byte()?;
            s.push(if b < 0 { '\u{FFFF}' } else { char::from(b as u8) });
        }
        Ok(s)
    }

    fn skip(&mut self, n: u64) -> io::Result<()> {
        let copied = io::copy(&mut (&mut self.input).take(n), &mut io::sink())?;
        self.pos += copied;
        Ok(())
    }
}

const DEADBEEF: i32 = 0xDEADBEEFu32 as i32;

impl TexturePackDevice {
    /// `new TexturePackDevice(name, flags)` + `initMetaData()`. `filename`
    /// is `getString("media/texturepacks/<name>.pack")`.
    pub fn open(name: &str, filename: &str, flags: i32) -> io::Result<Self> {
        let mut dev = TexturePackDevice {
            name: name.to_string(),
            filename: filename.to_string(),
            version: -1,
            pages: Vec::new(),
            page_map: HashMap::new(),
            sub_map: HashMap::new(),
            texture_flags: flags,
        };
        let file = File::open(filename)?;
        let mut r = PackReader { input: BufReader::new(file), pos: 0, chl: [0; 4] };
        let mut magic = [0u8; 4];
        let n = read_up_to(&mut r.input, &mut magic)?;
        if n == 4 && &magic == b"PZPK" {
            r.pos = 4;
            dev.version = r.read_int()?;
            if dev.version != 1 {
                return Err(io::Error::new(io::ErrorKind::InvalidData, format!("invalid .pack file version {}", dev.version)));
            }
        } else {
            r.input.seek(SeekFrom::Start(0))?;
            dev.version = 0;
        }
        let count = r.read_int()?;
        for _ in 0..count.max(0) {
            let page = dev.read_page(&mut r)?;
            let idx = dev.pages.len();
            dev.page_map.insert(page.name.clone(), idx);
            for (j, sub) in page.sub.iter().enumerate() {
                dev.sub_map.insert(sub.name.clone(), (idx, j));
            }
            dev.pages.push(page);
        }
        Ok(dev)
    }

    fn read_page<R: Read>(&self, r: &mut PackReader<R>) -> io::Result<Page> {
        let name = r.read_string()?;
        let entries = r.read_int()?;
        let has_alpha = r.read_int()? != 0;
        let mut sub = Vec::new();
        for _ in 0..entries.max(0) {
            let entry = r.read_string()?;
            let v = [r.read_int()?, r.read_int()?, r.read_int()?, r.read_int()?, r.read_int()?, r.read_int()?, r.read_int()?, r.read_int()?];
            sub.push(SubTextureInfo { x: v[0], y: v[1], w: v[2], h: v[3], ox: v[4], oy: v[5], fx: v[6], fy: v[7], name: entry });
        }
        let png_start = r.pos;
        if self.version == 0 {
            while r.read_int_byte()? != DEADBEEF {}
        } else {
            let len = r.read_int()?;
            r.skip(u64::try_from(len).unwrap_or(0))?;
        }
        Ok(Page { name, has_alpha, png_start, sub })
    }

    /// `getSubTextureInfo(result)`: every sub-texture of the pack by name.
    pub fn sub_textures(&self) -> HashMap<String, SubTexture> {
        self.sub_map
            .values()
            .map(|&(p, s)| {
                let page = &self.pages[p];
                let info = page.sub[s].clone();
                (info.name.clone(), SubTexture { pack_name: self.name.clone(), page_name: page.name.clone(), info })
            })
            .collect()
    }

    pub fn page(&self, name: &str) -> Option<&Page> {
        self.page_map.get(name).map(|&i| &self.pages[i])
    }

    /// `isAlpha(page)`: whether the page's sub-textures get hit masks.
    pub fn is_alpha(&self, page: &str) -> bool {
        self.page(page).is_some_and(|p| p.has_alpha)
    }

    /// `createStream(page)`: the file positioned at the page's PNG. The
    /// decoder reads only as much as the PNG needs.
    pub fn open_page(&self, page: &str) -> io::Result<BufReader<File>> {
        let p = self.page(page).ok_or(io::ErrorKind::NotFound)?;
        let mut f = File::open(&self.filename)?;
        f.seek(SeekFrom::Start(p.png_start))?;
        let mut r = BufReader::new(f);
        if self.version >= 1 {
            let mut len = [0u8; 4];
            r.read_exact(&mut len)?;
        }
        Ok(r)
    }
}

fn read_up_to(r: &mut impl Read, buf: &mut [u8]) -> io::Result<usize> {
    let mut n = 0;
    while n < buf.len() {
        match r.read(&mut buf[n..]) {
            Ok(0) => break,
            Ok(k) => n += k,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e),
        }
    }
    Ok(n)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn int(v: i32, out: &mut Vec<u8>) {
        out.extend_from_slice(&v.to_le_bytes());
    }

    fn string(s: &str, out: &mut Vec<u8>) {
        int(s.len() as i32, out);
        out.extend_from_slice(s.as_bytes());
    }

    /// Builds a pack with one page holding two entries and a fake image.
    fn build(version1: bool) -> Vec<u8> {
        let mut b = Vec::new();
        if version1 {
            b.extend_from_slice(b"PZPK");
            int(1, &mut b);
        }
        int(1, &mut b);
        string("page0", &mut b);
        int(2, &mut b);
        int(1, &mut b);
        for (name, x) in [("a", 0), ("b", 16)] {
            string(name, &mut b);
            for v in [x, 0, 16, 8, 1, 2, 32, 32] {
                int(v, &mut b);
            }
        }
        let image = b"IMAGEDATA";
        if version1 {
            int(image.len() as i32, &mut b);
            b.extend_from_slice(image);
        } else {
            b.extend_from_slice(image);
            int(DEADBEEF, &mut b);
        }
        b
    }

    #[test]
    fn reads_both_versions() {
        for v1 in [false, true] {
            let dir = std::env::temp_dir().join(format!("pz-fs-pack-{}-{v1}", std::process::id()));
            std::fs::create_dir_all(&dir).unwrap();
            let path = dir.join("T.pack");
            std::fs::write(&path, build(v1)).unwrap();
            let dev = TexturePackDevice::open("T", &path.to_string_lossy(), 2).unwrap();
            assert_eq!(dev.version, i32::from(v1));
            assert!(dev.is_alpha("page0"));
            let subs = dev.sub_textures();
            assert_eq!(subs["b"].info, SubTextureInfo { x: 16, y: 0, w: 16, h: 8, ox: 1, oy: 2, fx: 32, fy: 32, name: "b".into() });
            assert_eq!(subs["a"].page_name, "page0");
            let mut img = [0u8; 9];
            dev.open_page("page0").unwrap().read_exact(&mut img).unwrap();
            assert_eq!(&img, b"IMAGEDATA");
            std::fs::remove_dir_all(&dir).unwrap();
        }
    }
}
