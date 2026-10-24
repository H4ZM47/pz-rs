//! `seamFix2` (`TileSeamManager`, `TileSeamModifier`): narrow strips
//! drawn along the south and east edges of a chunk's floors and walls so
//! adjacent chunk textures meet without gaps.

use pz_render::Texture;

use crate::color;
use crate::texd::TexDraw;

/// `TileSeamManager.Tiles`, in ordinal order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SeamTile {
    FloorSouth,
    FloorEast,
    WallSouth,
    WallEast,
    FloorSouthOneThird,
    FloorEastOneThird,
    FloorSouthTwoThirds,
    FloorEastTwoThirds,
}

impl SeamTile {
    pub const ALL: [SeamTile; 8] = [
        SeamTile::FloorSouth,
        SeamTile::FloorEast,
        SeamTile::WallSouth,
        SeamTile::WallEast,
        SeamTile::FloorSouthOneThird,
        SeamTile::FloorEastOneThird,
        SeamTile::FloorSouthTwoThirds,
        SeamTile::FloorEastTwoThirds,
    ];

    pub fn index(self) -> usize {
        self as usize
    }

    /// `TileSeamManager.getVertices`: the strip's corners in the tile's
    /// frame, for the floor seams.
    pub fn vertices(self) -> Option<[f32; 8]> {
        Some(match self {
            SeamTile::FloorSouth => [5.0, 221.0, 69.0, 253.0, 63.0, 256.0, -1.0, 224.0],
            SeamTile::FloorEast => [57.0, 253.0, 121.0, 221.0, 127.0, 224.0, 63.0, 256.0],
            SeamTile::FloorSouthOneThird => [5.0, 157.0, 69.0, 189.0, 63.0, 192.0, -1.0, 160.0],
            SeamTile::FloorEastOneThird => [57.0, 189.0, 121.0, 157.0, 127.0, 160.0, 63.0, 192.0],
            SeamTile::FloorSouthTwoThirds => [5.0, 93.0, 69.0, 125.0, 63.0, 128.0, -1.0, 96.0],
            SeamTile::FloorEastTwoThirds => [57.0, 125.0, 121.0, 93.0, 127.0, 96.0, 63.0, 128.0],
            SeamTile::WallSouth | SeamTile::WallEast => return None,
        })
    }

    pub fn is_floor(self) -> bool {
        !matches!(self, SeamTile::WallSouth | SeamTile::WallEast)
    }
}

/// `TileSeamManager.init`: the eight cells of `media/depthmaps/SEAMS_01.png`
/// (8 columns of 128 x 256 at tile scale 2).
pub fn seam_masks(sheet: &Texture) -> Vec<Texture> {
    SeamTile::ALL
        .iter()
        .map(|t| {
            let i = t.index() as i32;
            let (w, h) = (64 * pz_render::iso::TILE_SCALE, 128 * pz_render::iso::TILE_SCALE);
            let mut tex = sheet.clone();
            tex.name = format!("SEAMS_01_{i}").into();
            tex.offset_x = 0.0;
            tex.offset_y = 0.0;
            tex.set_region((i % 8) * w, (i / 8) * h, w, h);
            tex.width_orig = w;
            tex.height_orig = h;
            tex
        })
        .collect()
}

/// `TileSeamModifier` set up for one seam draw.
#[derive(Clone, Debug)]
pub struct SeamModifier {
    pub tile: SeamTile,
    pub depth: Texture,
    pub mask: Texture,
    /// `col` (walls) or `colFloor` (floors) after `setupFloorDepth`.
    pub col: [u32; 4],
}

impl SeamModifier {
    /// `setupFloorDepth`: the floor's corner colours rotated onto the
    /// strip for the south and east seams.
    pub fn floor(tile: SeamTile, depth: Texture, mask: Texture, col: [u32; 4]) -> Self {
        let mut c = col;
        match tile {
            SeamTile::FloorSouth | SeamTile::FloorSouthOneThird | SeamTile::FloorSouthTwoThirds => {
                c[0] = col[3];
                c[1] = col[2];
            }
            SeamTile::FloorEast | SeamTile::FloorEastOneThird | SeamTile::FloorEastTwoThirds => {
                c[0] = col[2];
                c[3] = col[2];
                c[2] = col[1];
            }
            _ => {}
        }
        SeamModifier { tile, depth, mask, col: c }
    }

    /// `TileSeamModifier.accept`.
    pub fn accept(&self, d: &mut TexDraw) {
        let Some(tex0) = d.sprite.texture.clone() else { return };
        let (t1, t2) = (&self.depth, &self.mask);
        let left = d.sprite.x[0] - tex0.offset_x;
        let top = d.sprite.y[0] - tex0.offset_y;
        let (xs, ys) = match self.tile.vertices() {
            Some(v) => ([v[0], v[2], v[4], v[6]], [v[1], v[3], v[5], v[7]]),
            None => {
                let max3 = |a: f32, b: f32, c: f32| a.max(b).max(c);
                let min3 = |a: f32, b: f32, c: f32| a.min(b).min(c);
                let x0 = max3(tex0.offset_x, t1.offset_x, t2.offset_x);
                let x1 = min3(tex0.offset_x + tex0.width as f32, t1.offset_x + t1.width as f32, t2.offset_x + t2.width as f32);
                let y0 = max3(tex0.offset_y, t1.offset_y, t2.offset_y);
                let y2 = min3(tex0.offset_y + tex0.height as f32, t1.offset_y + t1.height as f32, t2.offset_y + t2.height as f32);
                ([x0, x1, x1, x0], [y0, y0, y2, y2])
            }
        };
        let u = |t: &Texture, x: f32| (t.x_start * t.width_hw as f32 + (x - t.offset_x)) / t.width_hw as f32;
        let v = |t: &Texture, y: f32| (t.y_start * t.height_hw as f32 + (y - t.offset_y)) / t.height_hw as f32;
        for i in 0..4 {
            d.sprite.x[i] = left + xs[i];
            d.sprite.y[i] = top + ys[i];
            d.sprite.u[i] = u(&tex0, xs[i]);
            d.sprite.v[i] = v(&tex0, ys[i]);
            d.uv1[i] = [u(t1, xs[i]), v(t1, ys[i])];
            d.uv2[i] = [u(t2, xs[i]), v(t2, ys[i])];
            d.set_col(i, color::blend_bgr(d.col(i), self.col[i]));
        }
        d.tex1 = Some(t1.clone());
        d.tex2 = Some(t2.clone());
    }
}

/// The depth maps a frame draws with: the tile depth textures plus the
/// seam masks.
pub struct WorldDepthMaps<'a> {
    pub tiles: &'a pz_render::tile_depth::TileDepthTextureManager,
    /// [`seam_masks`] of `SEAMS_01.png`.
    pub seams: Vec<Texture>,
    /// The cutaway stencils, as [`crate::world::DepthMaps::cutaway_stencil`]
    /// numbers them.
    pub cutaways: Vec<Texture>,
}

impl crate::world::DepthMaps for WorldDepthMaps<'_> {
    fn preset(&self, preset: crate::world::DepthPreset) -> Option<Texture> {
        crate::world::DepthMaps::preset(self.tiles, preset)
    }

    fn default_texture(&self) -> Option<Texture> {
        crate::world::DepthMaps::default_texture(self.tiles)
    }

    fn seam_mask(&self, tile: SeamTile) -> Option<Texture> {
        self.seams.get(tile.index()).cloned()
    }

    fn cutaway_stencil(&self, index: usize) -> Option<Texture> {
        self.cutaways.get(index).cloned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tex(w: u32, h: u32, ox: f32, oy: f32) -> Texture {
        let mut t = Texture::empty("t");
        t.width = w;
        t.height = h;
        t.width_hw = w.next_power_of_two() as i32;
        t.height_hw = h.next_power_of_two() as i32;
        t.offset_x = ox;
        t.offset_y = oy;
        t.x_end = 1.0;
        t.y_end = 1.0;
        t.ready = true;
        t
    }

    #[test]
    fn floor_seam_places_the_strip() {
        let floor = tex(128, 64, 0.0, 192.0);
        let full = tex(128, 256, 0.0, 0.0);
        let mut d = TexDraw::new(&floor, 100.0, 392.0, 128.0, 64.0, pz_render::Rgba(1.0, 1.0, 1.0, 1.0)).unwrap();
        let cols = [0xFF00_0001, 0xFF00_0002, 0xFF00_0003, 0xFF00_0004];
        let m = SeamModifier::floor(SeamTile::FloorSouth, full.clone(), full, cols);
        m.accept(&mut d);
        // left = 100, top = 200: the strip's corners from the vertex table.
        assert_eq!(d.sprite.x, [105.0, 169.0, 163.0, 99.0]);
        assert_eq!(d.sprite.y, [421.0, 453.0, 456.0, 424.0]);
        assert_eq!(d.sprite.v[0], (221.0 - 192.0) / 64.0);
        assert_eq!(d.uv1[2], [63.0 / 128.0, 1.0]);
        // The south seam takes the floor's bottom corner colours.
        assert_eq!(d.sprite.color[0], 0xFF00_0004);
        assert_eq!(d.sprite.color[1], 0xFF00_0003);
    }
}
