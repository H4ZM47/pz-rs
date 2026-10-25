//! What `DeadBodyAtlas` and `WorldItemAtlas` share: atlas textures cut into
//! equal entries (`Atlas.addEntry`, `assignEntryToAtlas`), the entry keys
//! (`Checksummer`, an MD5 written as decimal bytes), and the in-world draw
//! of an entry with the `DeadBodyAtlas` shader (`BodyTextureDepthDrawer`,
//! `ItemTextureDepthDrawer`), which takes each texel's depth from the
//! atlas depth texture, blended between a near and a far square depth.

use pz_render::iso::{self, fastfloor, TILE_SCALE};
use pz_render::{gl, DrawCmd};

use crate::Camera;

/// One entry of an atlas (`AtlasEntry`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AtlasEntry {
    /// The atlas texture's id (`DrawCmd::AtlasBegin`).
    pub atlas: u64,
    /// The atlas size (square).
    pub atlas_size: i32,
    /// Top-left corner, counted from the atlas' top row as `toBodyAtlas`
    /// and the scissor boxes count it.
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
    pub offset_x: f32,
    pub offset_y: f32,
}

impl AtlasEntry {
    /// `atlas.tex.split(key, x, size - (y + h), w, h)`: the entry's
    /// texture coordinates (x start, y start, x end, y end), bottom-up.
    pub fn uv(&self) -> [f32; 4] {
        let s = self.atlas_size as f32;
        let ty = self.atlas_size - (self.y + self.h);
        [
            self.x as f32 / s,
            ty as f32 / s,
            (self.x + self.w) as f32 / s,
            (ty + self.h) as f32 / s,
        ]
    }

    /// The entry's scissor box in GL window coordinates (bottom-left).
    pub fn scissor(&self) -> [i32; 4] {
        [self.x, self.atlas_size - (self.y + self.h), self.w, self.h]
    }
}

/// `Atlas`: a square texture of `entry_w` x `entry_h` cells filled in row
/// order.
#[derive(Clone, Debug)]
pub struct Atlas {
    pub id: u64,
    pub size: i32,
    pub entry_w: i32,
    pub entry_h: i32,
    pub entries: i32,
    /// Cleared before its first use.
    pub clear: bool,
}

impl Atlas {
    pub fn is_full(&self) -> bool {
        self.entries >= (self.size / self.entry_w) * (self.size / self.entry_h)
    }

    /// `addEntry`: the next cell.
    fn add_entry(&mut self) -> AtlasEntry {
        let columns = self.size / self.entry_w;
        let (col, row) = (self.entries % columns, self.entries / columns);
        self.entries += 1;
        AtlasEntry {
            atlas: self.id,
            atlas_size: self.size,
            x: col * self.entry_w,
            y: row * self.entry_h,
            w: self.entry_w,
            h: self.entry_h,
            offset_x: 0.0,
            offset_y: 0.0,
        }
    }
}

/// An atlas list (`atlasList`) with the ids its textures get.
#[derive(Clone, Debug)]
pub struct Atlases {
    pub size: i32,
    pub list: Vec<Atlas>,
    next_id: u64,
}

impl Atlases {
    /// Atlases of `size` pixels with texture ids from `first_id` up.
    pub fn new(size: i32, first_id: u64) -> Self {
        Atlases {
            size,
            list: Vec::new(),
            next_id: first_id,
        }
    }

    /// `assignEntryToAtlas`: a cell in the first atlas of this entry size
    /// with room, or in a new one.
    pub fn assign(&mut self, entry_w: i32, entry_h: i32) -> (usize, AtlasEntry) {
        if let Some(i) = self
            .list
            .iter()
            .position(|a| !a.is_full() && a.entry_w == entry_w && a.entry_h == entry_h)
        {
            return (i, self.list[i].add_entry());
        }
        let id = self.next_id;
        self.next_id += 1;
        self.list.push(Atlas {
            id,
            size: self.size,
            entry_w,
            entry_h,
            entries: 0,
            clear: true,
        });
        let i = self.list.len() - 1;
        (i, self.list[i].add_entry())
    }
}

/// `Checksummer`: MD5 over the fields, written as the digest's unsigned
/// bytes in decimal.
#[derive(Default)]
pub struct Checksummer {
    data: Vec<u8>,
}

impl Checksummer {
    pub fn byte(&mut self, b: u8) {
        self.data.push(b);
    }

    pub fn bool(&mut self, b: bool) {
        self.data.push(u8::from(b));
    }

    /// Little-endian, as `update(int)` writes it.
    pub fn int(&mut self, i: i32) {
        self.data.extend_from_slice(&i.to_le_bytes());
    }

    /// Nothing for null or empty strings.
    pub fn str(&mut self, s: Option<&str>) {
        if let Some(s) = s {
            self.data.extend_from_slice(s.as_bytes());
        }
    }

    /// `(byte)(f * 255.0f)`.
    pub fn unit(&mut self, f: f32) {
        self.byte(java_byte(f * 255.0));
    }

    pub fn finish(&self) -> String {
        md5(&self.data).iter().map(|b| b.to_string()).collect()
    }
}

/// Java's `(byte)` of a float: through `int`, keeping the low 8 bits.
pub fn java_byte(f: f32) -> u8 {
    (f as i32) as u8
}

/// MD5 (RFC 1321).
pub fn md5(input: &[u8]) -> [u8; 16] {
    const S: [u32; 64] = [
        7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 5, 9, 14, 20, 5, 9, 14, 20, 5,
        9, 14, 20, 5, 9, 14, 20, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 6, 10,
        15, 21, 6, 10, 15, 21, 6, 10, 15, 21, 6, 10, 15, 21,
    ];
    let k: [u32; 64] =
        std::array::from_fn(|i| ((i as f64 + 1.0).sin().abs() * 4_294_967_296.0) as u32);
    let mut msg = input.to_vec();
    let bits = (input.len() as u64).wrapping_mul(8);
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&bits.to_le_bytes());
    let mut h = [0x6745_2301u32, 0xefcd_ab89, 0x98ba_dcfe, 0x1032_5476];
    for chunk in msg.chunks(64) {
        let m: [u32; 16] = std::array::from_fn(|i| {
            u32::from_le_bytes(chunk[i * 4..i * 4 + 4].try_into().unwrap())
        });
        let [mut a, mut b, mut c, mut d] = h;
        for i in 0..64 {
            let (f, g) = match i / 16 {
                0 => ((b & c) | (!b & d), i),
                1 => ((d & b) | (!d & c), (5 * i + 1) % 16),
                2 => (b ^ c ^ d, (3 * i + 5) % 16),
                _ => (c ^ (b | !d), (7 * i) % 16),
            };
            let f = f.wrapping_add(a).wrapping_add(k[i]).wrapping_add(m[g]);
            a = d;
            d = c;
            c = b;
            b = b.wrapping_add(f.rotate_left(S[i]));
        }
        h = [
            h[0].wrapping_add(a),
            h[1].wrapping_add(b),
            h[2].wrapping_add(c),
            h[3].wrapping_add(d),
        ];
    }
    let mut out = [0u8; 16];
    for (i, v) in h.iter().enumerate() {
        out[i * 4..i * 4 + 4].copy_from_slice(&v.to_le_bytes());
    }
    out
}

/// The `addQuad(x1, y1, xStart, yStart, x2, y2, xEnd, yEnd, ...)` of an
/// entry drawn at `(x1, y1)` size `w` x `h`.
fn entry_quad(
    e: &AtlasEntry,
    x1: f32,
    y1: f32,
    w: f32,
    h: f32,
    rgba: [f32; 4],
    depth: [f32; 2],
) -> DrawCmd {
    let [u0, v0, u1, v1] = e.uv();
    let (x2, y2) = (x1 + w, y1 + h);
    DrawCmd::AtlasQuad {
        id: e.atlas,
        pos: [[x1, y1], [x2, y1], [x2, y2], [x1, y2]],
        uv: [[u0, v0], [u1, v0], [u1, v1], [u0, v1]],
        color: crate::color::to_abgr(rgba[0], rgba[1], rgba[2], rgba[3]),
        depth,
    }
}

/// The square depth `getSquareDepthData(camCharacter, x, y, z).depthStart`.
fn square_depth(cam: &Camera, x: f32, y: f32, z: f32) -> f32 {
    iso::square_depth(fastfloor(cam.x), fastfloor(cam.y), x, y, z)
}

/// `DeadBodyAtlas.BodyTextureDepthDrawer`: a corpse's entry drawn at
/// screen `(x, y)` for a body at world `origin`, tinted by 1.25 x the
/// light; depth runs from 2.2 squares in front of the body (a level up)
/// to 2.2 behind it.
pub fn body_depth_draw(
    e: &AtlasEntry,
    cam: &Camera,
    origin: [f32; 3],
    x: f32,
    y: f32,
    rgba: [f32; 4],
) -> Vec<DrawCmd> {
    let c = |v: f32| (v * 1.25).clamp(0.0, 1.0);
    let x1 = x - e.w as f32 / 2.0 - e.offset_x;
    let y1 = y - e.h as f32 / 2.0 - e.offset_y;
    let (jx, jy) = (cam.fix_jiggly_square_x, cam.fix_jiggly_square_y);
    let ts = TILE_SCALE as f32;
    let wx = -(e.offset_x + 2.0 * e.offset_y) / (64.0 * ts);
    let wy = -(e.offset_x - 2.0 * e.offset_y) / (-64.0 * ts);
    let [ox, oy, oz] = origin;
    let near = square_depth(cam, ox + jx + wx + 2.2, oy + jy + wy + 2.2, oz + 1.0) + 0.0014;
    let far = square_depth(cam, ox + jx + wx - 2.2, oy + jy + wy - 2.2, oz) + 0.0014;
    vec![
        DrawCmd::GlEnable(gl::DEPTH_TEST),
        DrawCmd::DepthMask(true),
        DrawCmd::DepthFunc(gl::LEQUAL),
        entry_quad(
            e,
            x1,
            y1,
            e.w as f32,
            e.h as f32,
            [c(rgba[0]), c(rgba[1]), c(rgba[2]), rgba[3]],
            [near, far],
        ),
        // VBORenderer's run ends with the depth test off again.
        DrawCmd::GlDisable(gl::DEPTH_TEST),
    ]
}

/// `ItemTextureDepthDrawer.Z_OFFSET`.
pub const ITEM_Z_OFFSET: f32 = 0.015;
/// `ItemTextureDepthDrawer.DEPTH_OFFSET`.
pub const ITEM_DEPTH_OFFSET: f32 = 0.00145;
/// `ItemTextureDepthDrawer.DEPTH_BOX_LENGTH`.
pub const ITEM_DEPTH_BOX_LENGTH: f32 = 137.0;

/// `WorldItemAtlas.ItemTextureDepthDrawer`: an item's entry drawn at
/// screen `(x, y)` for an item at world `origin`; entries are rendered at
/// 2.5x (`maxZoom`) unless `max_zoom_is_one`.
pub fn item_depth_draw(
    e: &AtlasEntry,
    max_zoom_is_one: bool,
    cam: &Camera,
    origin: [f32; 3],
    x: f32,
    y: f32,
    rgba: [f32; 4],
) -> Vec<DrawCmd> {
    let max_zoom = if max_zoom_is_one { 1.0 } else { 2.5 };
    let x1 = x - (e.w as f32 / 2.0 - e.offset_x) / max_zoom;
    let y1 = y - (e.h as f32 / 2.0 - e.offset_y) / max_zoom;
    let (w, h) = (e.w as f32 / max_zoom, e.h as f32 / max_zoom);
    let (jx, jy) = (cam.fix_jiggly_square_x, cam.fix_jiggly_square_y);
    let mut f = ITEM_DEPTH_BOX_LENGTH;
    if max_zoom_is_one {
        f *= 2.5;
    }
    let [ox, oy, oz] = origin;
    let oz = oz + ITEM_Z_OFFSET;
    let near =
        square_depth(cam, ox + jx + 0.5 * f, oy + jy + 0.5 * f, oz + 1.0) + ITEM_DEPTH_OFFSET;
    let far = square_depth(cam, ox + jx - 0.5 * f, oy + jy - 0.5 * f, oz) + ITEM_DEPTH_OFFSET;
    vec![
        DrawCmd::GlEnable(gl::DEPTH_TEST),
        DrawCmd::DepthMask(true),
        DrawCmd::DepthFunc(gl::LEQUAL),
        DrawCmd::GlEnable(gl::BLEND),
        DrawCmd::BlendFunc(gl::SRC_ALPHA, gl::ONE_MINUS_SRC_ALPHA),
        entry_quad(e, x1, y1, w, h, rgba, [near, far]),
        DrawCmd::GlDisable(gl::DEPTH_TEST),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn md5_known_values() {
        let hex = |d: [u8; 16]| d.iter().map(|b| format!("{b:02x}")).collect::<String>();
        assert_eq!(hex(md5(b"")), "d41d8cd98f00b204e9800998ecf8427e");
        assert_eq!(
            hex(md5(b"The quick brown fox jumps over the lazy dog")),
            "9e107d9d372bb6826bd81d3542a419d6"
        );
        let mut c = Checksummer::default();
        c.str(Some(""));
        assert_eq!(
            c.finish(),
            md5(b"").iter().map(|b| b.to_string()).collect::<String>()
        );
    }

    #[test]
    fn entries_fill_rows_then_new_atlases() {
        let mut a = Atlases::new(1024, 100);
        let (i, e) = a.assign(256, 384);
        assert_eq!((i, e.x, e.y, e.atlas), (0, 0, 0, 100));
        for _ in 0..3 {
            a.assign(256, 384);
        }
        let (_, e) = a.assign(256, 384);
        assert_eq!((e.x, e.y), (0, 384));
        // Another size gets its own atlas.
        let (i, e) = a.assign(128, 128);
        assert_eq!((i, e.atlas), (1, 101));
        // 4 x 2 cells of 256 x 384 fill the first.
        for _ in 0..3 {
            a.assign(256, 384);
        }
        assert!(a.list[0].is_full());
        assert_eq!(a.assign(256, 384).0, 2);
        let e = AtlasEntry {
            atlas: 0,
            atlas_size: 1024,
            x: 256,
            y: 384,
            w: 256,
            h: 384,
            offset_x: 0.0,
            offset_y: 0.0,
        };
        assert_eq!(e.uv(), [0.25, 256.0 / 1024.0, 0.5, 640.0 / 1024.0]);
        assert_eq!(e.scissor(), [256, 256, 256, 384]);
    }
}
