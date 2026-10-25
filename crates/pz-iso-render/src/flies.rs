//! `FBORenderCell.renderFlies` and `CorpseFlies`: the animated swarm drawn
//! over each square with a rotting corpse (`IsoGridSquare.hasFlies`),
//! every frame after the fireplace flames, depth-tested against the
//! world.

use pz_render::iso::{self, fastfloor, TILE_SCALE};
use pz_render::{gl, DrawCmd, Sprite, Texture, WorldShader};

use crate::cell::ChunkRenderer;
use crate::chunk::{FLOOR_HEIGHT, FLOOR_WIDTH, JUMBO_XXL_HEIGHT, JUMBO_XXL_WIDTH};
use crate::frame::ChunkLevels;
use crate::world::{ObjectKind, RenderSquare};

const FRAME_SIZE: i32 = 128;
const COLUMNS: i32 = 8;
const NUM_FRAMES: i32 = 56;

/// `CorpseFlies`' state: the shared animation frame and
/// `media/textures/CorpseFlies.png` (8 x 7 frames of 128 pixels).
#[derive(Clone, Debug, Default)]
pub struct CorpseFlies {
    pub texture: Option<Texture>,
    /// `counter`, in milliseconds.
    pub counter: f32,
    /// `frame`, 0..56.
    pub frame: i32,
}

impl CorpseFlies {
    /// `CorpseFlies.update()` (`IsoWorld.update`, not on a server):
    /// 20 frames a second of real time.
    pub fn update(&mut self, realworld_seconds_since_last_update: f32) {
        self.counter += realworld_seconds_since_last_update * 1000.0;
        if self.counter > 50.0 {
            self.counter %= 50.0;
            self.frame = (self.frame + 1) % NUM_FRAMES;
        }
    }

    /// `CorpseFlies.render(x, y, z)`: one swarm over square `(x, y, z)`.
    pub fn render(&self, r: &ChunkRenderer<'_>, x: i32, y: i32, z: i32, out: &mut Vec<DrawCmd>) {
        let Some(tex) = self.texture.as_ref().filter(|t| t.ready) else {
            return;
        };
        let frame = (self.frame + x + y).rem_euclid(NUM_FRAMES);
        let (column, row) = (frame % COLUMNS, frame / COLUMNS);
        let (tw, th) = (tex.width as f32, tex.height as f32);
        let u1 = (column * FRAME_SIZE) as f32 / tw;
        let v1 = (row * FRAME_SIZE) as f32 / th;
        let u2 = ((column + 1) * FRAME_SIZE) as f32 / tw;
        let v2 = ((row + 1) * FRAME_SIZE) as f32 / th;
        let cam = &r.cam;
        let (fx, fy, fz) = (x as f32, y as f32, z as f32);
        let mut sx = iso::x_to_screen(fx + 0.5, fy + 0.5) - cam.off_x as f32;
        let mut sy = iso::y_to_screen(fx + 0.5, fy + 0.5, fz, 0) - cam.off_y as f32;
        let w = 64 * TILE_SCALE;
        sx -= (w / 2) as f32;
        sy -= (w + 16 * TILE_SCALE) as f32;
        let w = w as f32;
        // StartShader(0); nextZ from the square depth a little inside the
        // square (nextChunkDepth stays 0).
        let depth = iso::square_depth(
            fastfloor(cam.x),
            fastfloor(cam.y),
            fx + 0.75,
            fy + 0.75,
            fz + 0.25,
        ) * 2.0
            - 1.0;
        let c = pz_render::color_to_abgr(1.0, 1.0, 1.0, 1.0);
        let sprite = Sprite {
            texture: Some(tex.clone()),
            x: [sx, sx + w, sx + w, sx],
            y: [sy, sy, sy + w, sy + w],
            u: [u1, u2, u2, u1],
            v: [v1, v1, v2, v2],
            color: [c; 4],
            flipped: false,
        };
        out.push(DrawCmd::WorldSprite(
            sprite,
            WorldShader::Default { z: depth },
        ));
    }
}

/// `IsoGridSquare.IsOnScreen(false)` against the offscreen buffer of the
/// frame (`view_size` times zoom).
pub fn square_on_screen(r: &ChunkRenderer<'_>, sq: &RenderSquare) -> bool {
    let sx = iso::x_to_screen(sq.x as f32, sq.y as f32) - r.cam.off_x as f32;
    let sy = iso::y_to_screen(sq.x as f32, sq.y as f32, sq.z as f32, 0) - r.cam.off_y as f32;
    let ow = r.view_size.0 as f32 * r.cam.zoom;
    let oh = r.view_size.1 as f32 * r.cam.zoom;
    if sq.objects.iter().any(|o| o.kind == ObjectKind::Tree) {
        let off_x = ((JUMBO_XXL_WIDTH - FLOOR_WIDTH) / 2) as f32;
        let off_y = (JUMBO_XXL_HEIGHT - FLOOR_HEIGHT) as f32;
        return !(sx + off_x <= 0.0
            || sy + FLOOR_HEIGHT as f32 <= 0.0
            || sx - off_x >= ow
            || sy - off_y >= oh);
    }
    !(sx + FLOOR_WIDTH as f32 / 2.0 <= 0.0
        || sy + FLOOR_HEIGHT as f32 <= 0.0
        || sx - FLOOR_WIDTH as f32 / 2.0 >= ow
        || sy - iso::PIXELS_PER_LEVEL as f32 >= oh)
}

/// `renderFlies(playerIndex)`: per on-screen chunk holding squares with
/// flies (`chunksWithFlies`, in on-screen order), its levels ascending,
/// each visible on-screen square with flies, depth-tested `LEQUAL`
/// without writes, blended (SRC_ALPHA, ONE_MINUS_SRC_ALPHA).
pub fn render_flies(
    r: &ChunkRenderer<'_>,
    f: &CorpseFlies,
    chunks: &[ChunkLevels<'_>],
    out: &mut Vec<DrawCmd>,
) {
    let cv = r.cutaways.as_ref();
    let mut done: Vec<(i32, i32)> = Vec::new();
    for c in chunks {
        let k = (c.texture.wx, c.texture.wy);
        if done.contains(&k) {
            continue;
        }
        done.push(k);
        let mut squares: Vec<&RenderSquare> = chunks
            .iter()
            .filter(|o| (o.texture.wx, o.texture.wy) == k)
            .flat_map(|o| {
                o.levels
                    .iter()
                    .flat_map(|(_, l)| l.iter().flatten().copied())
            })
            .filter(|s| s.flies)
            .collect();
        if squares.is_empty() {
            continue;
        }
        squares.sort_by_key(|s| s.z);
        // renderOneChunk_Flies.
        out.extend([
            DrawCmd::GlEnable(gl::DEPTH_TEST),
            DrawCmd::DepthFunc(gl::LEQUAL),
            DrawCmd::DepthMask(false),
            DrawCmd::GlEnable(gl::BLEND),
            DrawCmd::BlendFunc(gl::SRC_ALPHA, gl::ONE_MINUS_SRC_ALPHA),
        ]);
        for sq in squares {
            if cv.is_some_and(|cv| !cv.is_visible(sq)) || !square_on_screen(r, sq) {
                continue;
            }
            f.render(r, sq.x, sq.y, sq.z, out);
        }
        out.push(DrawCmd::DepthMask(false));
        out.push(DrawCmd::DepthFunc(gl::ALWAYS));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::{DepthMaps, DepthPreset};
    use crate::Camera;

    struct NoMaps;
    impl DepthMaps for NoMaps {
        fn preset(&self, _: DepthPreset) -> Option<Texture> {
            None
        }
        fn default_texture(&self) -> Option<Texture> {
            None
        }
    }

    fn tex() -> Texture {
        let mut t = Texture::empty("media/textures/CorpseFlies.png");
        t.id = Some(7);
        t.width = 1024;
        t.height = 1024;
        t.ready = true;
        t
    }

    #[test]
    fn update_steps_at_20_fps_and_wraps() {
        let mut f = CorpseFlies::default();
        f.update(0.04);
        assert_eq!(f.frame, 0);
        f.update(0.02);
        assert_eq!(f.frame, 1);
        assert!((f.counter - 10.0).abs() < 1e-3);
        f.frame = 55;
        f.update(0.06);
        assert_eq!(f.frame, 0);
    }

    /// The quad sits centred on the square, 32 pixels above its top
    /// corner's 128-pixel box, with the frame picked by x + y.
    #[test]
    fn quad_and_frame() {
        let cam = Camera::centred(10.0, 10.0, 0.0, 1280, 720, 1.0);
        let r = ChunkRenderer::new(cam, &NoMaps, (1280, 720));
        let f = CorpseFlies {
            texture: Some(tex()),
            counter: 0.0,
            frame: 50,
        };
        let mut out = Vec::new();
        f.render(&r, 3, 4, 0, &mut out);
        let DrawCmd::WorldSprite(s, WorldShader::Default { z }) = &out[0] else {
            panic!()
        };
        // frame (50 + 7) % 56 = 1: column 1, row 0.
        assert_eq!(s.u, [0.125, 0.25, 0.25, 0.125]);
        assert_eq!(s.v, [0.0, 0.0, 0.125, 0.125]);
        let sx = iso::x_to_screen(3.5, 4.5) - cam.off_x as f32 - 64.0;
        let sy = iso::y_to_screen(3.5, 4.5, 0.0, 0) - cam.off_y as f32 - 160.0;
        assert_eq!(
            (s.x[0], s.y[0], s.x[2], s.y[2]),
            (sx, sy, sx + 128.0, sy + 128.0)
        );
        assert_eq!(*z, iso::square_depth(10, 10, 3.75, 4.75, 0.25) * 2.0 - 1.0);
    }
}
