//! `BinkCopyToBuffer` with surface type 6 (32-bit RGBA): the decoded
//! YCrCb 4:2:0 planes to RGBA bytes.
//!
//! The arithmetic was fitted black-box against the game's Bink library
//! (planes from `BinkGetFrameBuffersInfo` against its RGBA output) and
//! matches it on every pixel of the frames compared (see
//! `oracle_frames_match`):
//! - chroma is upsampled bilinearly from centre-sited samples with weights
//!   9/3/3/1, the neighbour clamped at the plane edges;
//! - a 2x2 ordered dither (0, 128, 192, 64 in 1/256 units) is added to the
//!   scaled luma;
//! - the matrix is BT.601 studio range in fixed point.

/// One 8-bit plane.
#[derive(Clone, Copy, Debug)]
pub struct Plane<'a> {
    pub data: &'a [u8],
    pub pitch: usize,
}

impl Plane<'_> {
    #[inline]
    fn at(&self, x: usize, y: usize) -> i32 {
        i32::from(self.data[y * self.pitch + x])
    }
}

/// A decoded frame: luma `width` x `height`, chroma `(width + 1) / 2` x
/// `(height + 1) / 2`, and the alpha plane at luma size when the file has
/// one.
#[derive(Clone, Copy, Debug)]
pub struct FramePlanes<'a> {
    pub width: usize,
    pub height: usize,
    pub y: Plane<'a>,
    pub cr: Plane<'a>,
    pub cb: Plane<'a>,
    pub a: Option<Plane<'a>>,
}

const DITHER: [i32; 4] = [0, 128, 192, 64];

#[inline]
fn clamp8(v: i32) -> u8 {
    v.clamp(0, 255) as u8
}

/// Converts `p` into `dst` (`pitch` bytes per row, at least `4 * width`).
pub fn copy_rgba(p: &FramePlanes<'_>, dst: &mut [u8], pitch: usize) {
    let (w, h) = (p.width, p.height);
    if w == 0 || h == 0 {
        return;
    }
    let cw = w.div_ceil(2);
    let ch = h.div_ceil(2);
    for y in 0..h {
        let cy = y >> 1;
        let ny = if y & 1 == 1 { (cy + 1).min(ch - 1) } else { cy.saturating_sub(1) };
        let row = &mut dst[y * pitch..y * pitch + 4 * w];
        for x in 0..w {
            let cx = x >> 1;
            let nx = if x & 1 == 1 { (cx + 1).min(cw - 1) } else { cx.saturating_sub(1) };
            let up = |pl: &Plane<'_>| 9 * pl.at(cx, cy) + 3 * pl.at(nx, cy) + 3 * pl.at(cx, ny) + pl.at(nx, ny) - 2048;
            let sr = up(&p.cr);
            let sb = up(&p.cb);
            let yy = 298 * p.y.at(x, y) + DITHER[(x & 1) | ((y & 1) << 1)];
            let r = (yy + 32 * ((6536 * sr) >> 13) - 4992) >> 8;
            let g = (yy + 32 * ((-3332 * sr) >> 13) + 32 * ((-1603 * sb + 2048) >> 13) - 4640) >> 8;
            let b = (yy + 32 * ((8265 * sb + 2048) >> 13) - 5056) >> 8;
            let a = p.a.as_ref().map_or(255, |a| a.at(x, y) as u8);
            row[4 * x..4 * x + 4].copy_from_slice(&[clamp8(r), clamp8(g), clamp8(b), a]);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flat(w: usize, h: usize, y: u8, cr: u8, cb: u8) -> Vec<u8> {
        let (cw, ch) = (w.div_ceil(2), h.div_ceil(2));
        let yp = vec![y; w * h];
        let crp = vec![cr; cw * ch];
        let cbp = vec![cb; cw * ch];
        let p = FramePlanes {
            width: w,
            height: h,
            y: Plane { data: &yp, pitch: w },
            cr: Plane { data: &crp, pitch: cw },
            cb: Plane { data: &cbp, pitch: cw },
            a: None,
        };
        let mut out = vec![0; 4 * w * h];
        copy_rgba(&p, &mut out, 4 * w);
        out
    }

    #[test]
    fn grey_levels_and_range() {
        // Studio black and white with neutral chroma, all four dither
        // positions.
        let black = flat(2, 2, 16, 128, 128);
        assert!(black.chunks(4).all(|px| px[3] == 255 && px[..3].iter().all(|&c| c <= 1)), "{black:?}");
        let white = flat(2, 2, 235, 128, 128);
        assert!(white.chunks(4).all(|px| px[..3].iter().all(|&c| c >= 253)), "{white:?}");
        let mid = flat(2, 2, 126, 128, 128);
        assert!(mid.chunks(4).all(|px| (127..=129).contains(&px[0])));
    }

    #[test]
    fn saturated_chroma_clamps() {
        let red = flat(2, 2, 81, 240, 90);
        assert!(red.chunks(4).all(|px| px[0] > 250 && px[1] < 5 && px[2] < 5), "{red:?}");
        let blue = flat(2, 2, 41, 110, 240);
        assert!(blue.chunks(4).all(|px| px[2] > 250 && px[0] < 5), "{blue:?}");
    }

    #[test]
    fn odd_sizes_clamp_chroma_edges() {
        // 3x3 luma has 2x2 chroma; the last column/row reuse the edge.
        let out = flat(3, 3, 100, 128, 128);
        assert_eq!(out.len(), 36);
    }

    /// Parity with the game's Bink library: `PZ_BINK_ORACLE` lists
    /// `dir:width` pairs (comma separated) of dumps holding
    /// `frame_NNNN.{y,cr,cb,rgba}` (tightly packed planes and the
    /// library's RGBA for the same frame).
    #[test]
    fn oracle_frames_match() {
        let Ok(list) = std::env::var("PZ_BINK_ORACLE") else { return };
        let mut checked = 0;
        for item in list.split(',') {
            let (dir, w) = item.rsplit_once(':').expect("dir:width");
            let w: usize = w.parse().unwrap();
            let mut names: Vec<_> = std::fs::read_dir(dir)
                .unwrap()
                .filter_map(|e| e.ok()?.file_name().into_string().ok())
                .filter(|n| n.ends_with(".rgba"))
                .collect();
            names.sort();
            for n in names {
                let stem = n.trim_end_matches(".rgba");
                let read = |ext: &str| std::fs::read(format!("{dir}/{stem}.{ext}")).unwrap();
                let (yp, crp, cbp, rgba) = (read("y"), read("cr"), read("cb"), read("rgba"));
                let h = yp.len() / w;
                let cw = w.div_ceil(2);
                let p = FramePlanes {
                    width: w,
                    height: h,
                    y: Plane { data: &yp, pitch: w },
                    cr: Plane { data: &crp, pitch: cw },
                    cb: Plane { data: &cbp, pitch: cw },
                    a: None,
                };
                let mut out = vec![0; 4 * w * h];
                copy_rgba(&p, &mut out, 4 * w);
                let bad = out.iter().zip(&rgba).filter(|(a, b)| a != b).count();
                assert_eq!(bad, 0, "{dir}/{stem}: {bad} bytes differ");
                checked += 1;
            }
        }
        assert!(checked > 0);
    }
}
