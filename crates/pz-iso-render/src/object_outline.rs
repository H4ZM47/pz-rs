//! `FBORenderObjectOutline`: outlines around highlighted objects that are
//! not characters (world items, corpses, mannequins, furniture and other
//! sprite objects). After the world's translucent passes and fog, each
//! highlighted object's textures (its sprite, overlay and attached sprites
//! when the highlight covers them, or its `DeadBodyAtlas` /
//! `WorldItemAtlas` entry) are drawn into `ModelOutlines`' mask with
//! `vboRenderer_SpriteOutline` (`Drawer2`), first the objects behind the
//! camera character, then those in front, sharing a mask while the colour
//! and side stay the same. `ModelOutlines.endFrame` thickens the mask and
//! draws the ring as it does for characters.

use pz_render::iso::{x_to_screen, y_to_screen, TILE_SCALE};
use pz_render::{DrawCmd, Texture};

use crate::atlas::AtlasEntry;
use crate::outlines::{abgr_to_color, ModelOutlines};
use crate::Camera;

/// One texture `Drawer2` draws into the mask: its quad in the world
/// projection and what it samples.
#[derive(Clone, Debug, PartialEq)]
pub struct OutlineTexture {
    pub texture: Option<Texture>,
    /// An atlas entry's texture (`DeadBodyAtlas`, `WorldItemAtlas`).
    pub atlas: Option<u64>,
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    /// `xStart, yStart, xEnd, yEnd`.
    pub uv: [f32; 4],
}

/// A highlighted object as `Drawer2.init` sets it up.
#[derive(Clone, Debug, PartialEq)]
pub struct ObjectOutline {
    /// `outlineColor.setABGR(getOutlineHighlightCol(playerIndex))`, alpha
    /// times `Core.blinkAlpha` when blinking.
    pub color: [f32; 4],
    /// `isBehindPlayer`.
    pub behind_player: bool,
    pub textures: Vec<OutlineTexture>,
}

/// `FBORenderObjectOutline.isBehindPlayer`: the object's square centre (a
/// world item's own position) above the camera character on screen.
pub fn is_behind_player(x: f32, y: f32, cam: &Camera) -> bool {
    x + y < cam.x + cam.y
}

/// The outline colour of a highlight (`setABGR` of the ABGR colour), with
/// the blink alpha applied.
pub fn outline_color(abgr: u32, blink_alpha: Option<f32>) -> [f32; 4] {
    let mut c = abgr_to_color(abgr);
    if let Some(b) = blink_alpha {
        c[3] *= b;
    }
    c
}

/// `Drawer2.init`'s screen position of an object at `(x, y, z)`: the
/// square-jiggle fix added to the world position, then the camera offset
/// (`IsoSprite.globalOffsetX/Y`).
pub fn object_screen(cam: &Camera, x: f32, y: f32, z: f32) -> (f32, f32) {
    let (jx, jy) = (cam.fix_jiggly_square_x, cam.fix_jiggly_square_y);
    (
        x_to_screen(x + jx, y + jy) - cam.off_x as f32,
        y_to_screen(x + jx, y + jy, z, 0) - cam.off_y as f32,
    )
}

fn entry_texture(e: &AtlasEntry, x: f32, y: f32, w: f32, h: f32) -> OutlineTexture {
    OutlineTexture {
        texture: None,
        atlas: Some(e.atlas),
        x,
        y,
        w,
        h,
        uv: e.uv(),
    }
}

impl ObjectOutline {
    /// An `IsoDeadBody` with its `DeadBodyAtlas` entry
    /// (`BodyTexture.getRenderX/Y/Width/Height`).
    pub fn corpse(e: &AtlasEntry, cam: &Camera, pos: [f32; 3], color: [f32; 4]) -> Self {
        let (sx, sy) = object_screen(cam, pos[0], pos[1], pos[2]);
        let behind = is_behind_player(pos[0].floor() + 0.5, pos[1].floor() + 0.5, cam);
        let (w, h) = (e.w as f32, e.h as f32);
        ObjectOutline {
            color,
            behind_player: behind,
            textures: vec![entry_texture(
                e,
                sx - w / 2.0 - e.offset_x,
                sy - h / 2.0 - e.offset_y,
                w,
                h,
            )],
        }
    }

    /// An `IsoMannequin` with its `DeadBodyAtlas` entry: lifted two pixels
    /// when it stands on a table surface or has a render y offset.
    /// `render_y_offset` is `getRenderYOffset()`, `surface` the summed
    /// `getSurfaceOffset() * tileScale` of the table surfaces below it on
    /// its square.
    pub fn mannequin(
        e: &AtlasEntry,
        cam: &Camera,
        pos: [f32; 3],
        render_y_offset: f32,
        surface: f32,
        color: [f32; 4],
    ) -> Self {
        let (sx, sy) = object_screen(cam, pos[0], pos[1], pos[2]);
        let mut sy = sy - render_y_offset * TILE_SCALE as f32;
        if render_y_offset + surface > 0.0 {
            sy -= 2.0;
        }
        let behind = is_behind_player(pos[0].floor() + 0.5, pos[1].floor() + 0.5, cam);
        let (w, h) = (e.w as f32, e.h as f32);
        ObjectOutline {
            color,
            behind_player: behind,
            textures: vec![entry_texture(
                e,
                sx - w / 2.0 - e.offset_x,
                sy - h / 2.0 - e.offset_y,
                w,
                h,
            )],
        }
    }

    /// An `IsoWorldInventoryObject` drawn from the `WorldItemAtlas`
    /// (`ItemTexture.getRenderX/Y/Width/Height`, entries rendered at 2.5x
    /// unless `max_zoom_is_one`). `pos` is the item's world position
    /// (square plus `xoff/yoff/zoff`).
    pub fn world_item_atlas(
        e: &AtlasEntry,
        max_zoom_is_one: bool,
        cam: &Camera,
        pos: [f32; 3],
        color: [f32; 4],
    ) -> Self {
        let (sx, sy) = object_screen(cam, pos[0], pos[1], pos[2]);
        let max_zoom = if max_zoom_is_one { 1.0 } else { 2.5 };
        let (w, h) = (e.w as f32 / max_zoom, e.h as f32 / max_zoom);
        let x = sx - (e.w as f32 / 2.0 - e.offset_x) / max_zoom;
        let y = sy - (e.h as f32 / 2.0 - e.offset_y) / max_zoom;
        ObjectOutline {
            color,
            behind_player: is_behind_player(pos[0], pos[1], cam),
            textures: vec![entry_texture(e, x, y, w, h)],
        }
    }

    /// An `IsoWorldInventoryObject` without an atlas texture
    /// (`initWorldInventoryObject`'s fallback): its world sprite, centred
    /// and lifted by three quarters of its scaled original height, drawn
    /// at the texture's own size. `scale` is the sprite definition's scale.
    pub fn world_item_sprite(
        tex: &Texture,
        scale: [f32; 2],
        cam: &Camera,
        pos: [f32; 3],
        color: [f32; 4],
    ) -> Self {
        let (sx, sy) = object_screen(cam, pos[0], pos[1], pos[2]);
        let orig = |o: i32, w: u32| if o > 0 { o as f32 } else { w as f32 };
        let dx = orig(tex.width_orig, tex.width) * scale[0] / 2.0;
        let dy = orig(tex.height_orig, tex.height) * scale[1] * 3.0 / 4.0;
        ObjectOutline {
            color,
            behind_player: is_behind_player(pos[0], pos[1], cam),
            textures: vec![sprite_texture(tex, sx - dx, sy - dy)],
        }
    }

    /// Any other object: its sprite's texture for the current frame, then,
    /// when the highlight covers attached sprites (`isOutlineHlAttached`),
    /// the overlay and attached sprites' textures (`textures` in that
    /// order). `offset` is the object's `offsetX/offsetY`,
    /// `render_y_offset` `getRenderYOffset()`.
    pub fn sprites(
        textures: &[Texture],
        cam: &Camera,
        pos: [f32; 3],
        offset: [f32; 2],
        render_y_offset: f32,
        color: [f32; 4],
    ) -> Self {
        let (sx, sy) = object_screen(cam, pos[0], pos[1], pos[2]);
        let x = sx - offset[0];
        let y = sy - (offset[1] + render_y_offset * TILE_SCALE as f32);
        ObjectOutline {
            color,
            behind_player: is_behind_player(pos[0].floor() + 0.5, pos[1].floor() + 0.5, cam),
            textures: textures.iter().map(|t| sprite_texture(t, x, y)).collect(),
        }
    }

    /// `Drawer2.render`: `beginRenderOutline` (flushing a pending outline
    /// of another colour or side), then each texture into the mask at
    /// `renderX + offsetX`, `renderY + offsetY`.
    pub fn render(&self, outlines: &mut ModelOutlines, out: &mut Vec<DrawCmd>) {
        let clear = outlines.begin_render_outline(self.color, self.behind_player, false, out);
        out.push(DrawCmd::OutlineBegin {
            width: outlines.width,
            height: outlines.height,
            clear,
        });
        for t in &self.textures {
            let (x1, y1) = (t.x, t.y);
            let (x2, y2) = (x1 + t.w, y1 + t.h);
            let [u0, v0, u1, v1] = t.uv;
            out.push(DrawCmd::OutlineSprite {
                texture: t.texture.clone(),
                atlas: t.atlas,
                pos: [[x1, y1], [x2, y1], [x2, y2], [x1, y2]],
                uv: [[u0, v0], [u1, v0], [u1, v1], [u0, v1]],
                // addQuad(..., 1, 1, 1, 1): the mask is opaque white; the
                // outline colour comes in with the thickening.
                color: [1.0; 4],
            });
        }
        out.push(DrawCmd::OutlineEnd);
    }
}

/// A texture at `renderX/Y` plus its trim offset, at its own size.
fn sprite_texture(t: &Texture, x: f32, y: f32) -> OutlineTexture {
    OutlineTexture {
        texture: Some(t.clone()),
        atlas: None,
        x: x + t.offset_x,
        y: y + t.offset_y,
        w: t.width as f32,
        h: t.height as f32,
        uv: [t.x_start, t.y_start, t.x_end, t.y_end],
    }
}

/// `FBORenderObjectOutline.render(playerIndex)`: the objects behind the
/// camera character, then those in front, in list order.
pub fn render_object_outlines(
    objects: &[ObjectOutline],
    outlines: &mut ModelOutlines,
    out: &mut Vec<DrawCmd>,
) {
    for behind in [true, false] {
        for o in objects
            .iter()
            .filter(|o| o.behind_player == behind && !o.textures.is_empty())
        {
            o.render(outlines, out);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry() -> AtlasEntry {
        AtlasEntry {
            atlas: 5,
            atlas_size: 1024,
            x: 0,
            y: 0,
            w: 192,
            h: 128,
            offset_x: 10.0,
            offset_y: -4.0,
        }
    }

    #[test]
    fn corpse_outline_covers_its_atlas_draw() {
        let cam = Camera::player(10.5, 10.5, 0.0, 640, 480, 1.0);
        let o = ObjectOutline::corpse(&entry(), &cam, [11.9, 10.2, 0.0], [1.0, 0.0, 0.0, 1.0]);
        // BodyTexture.render draws at x - w/2 - offsetX of the same screen
        // position (without the jiggle fix, which is zero here).
        let draw = crate::body_atlas::body_world_draw(&entry(), &cam, [11.9, 10.2, 0.0], [1.0; 4]);
        let pz_render::DrawCmd::AtlasQuad { pos, uv, .. } = &draw[3] else {
            panic!("{:?}", draw[3])
        };
        let t = &o.textures[0];
        assert!(
            (t.x - pos[0][0]).abs() < 1e-3 && (t.y - pos[0][1]).abs() < 1e-3,
            "{t:?} vs {pos:?}"
        );
        assert_eq!([t.uv[0], t.uv[1]], uv[0]);
        assert!(!o.behind_player, "south-east of the camera character");
    }

    #[test]
    fn behind_objects_draw_first_and_colours_share_a_mask() {
        let cam = Camera::player(10.5, 10.5, 0.0, 640, 480, 1.0);
        let red = [1.0, 0.0, 0.0, 1.0];
        let front = ObjectOutline::corpse(&entry(), &cam, [12.0, 12.0, 0.0], red);
        let back = ObjectOutline::corpse(&entry(), &cam, [8.0, 8.0, 0.0], red);
        let mut outlines = ModelOutlines::new(640, 480, 1.0);
        outlines.start_frame();
        let mut out = Vec::new();
        render_object_outlines(&[front, back.clone(), back], &mut outlines, &mut out);
        outlines.end_frame(&mut out);
        let kinds: Vec<&str> = out
            .iter()
            .filter_map(|c| match c {
                DrawCmd::OutlineBegin { clear: true, .. } => Some("begin+clear"),
                DrawCmd::OutlineBegin { clear: false, .. } => Some("begin"),
                DrawCmd::OutlineSprite { .. } => Some("sprite"),
                DrawCmd::OutlineThicken { .. } => Some("thicken"),
                DrawCmd::OutlineBlit => Some("blit"),
                _ => None,
            })
            .collect();
        // Two behind (one mask), then the side changes: flush, new mask.
        assert_eq!(
            kinds,
            [
                "begin+clear",
                "sprite",
                "begin",
                "sprite",
                "thicken",
                "blit",
                "begin+clear",
                "sprite",
                "thicken",
                "blit"
            ]
        );
    }

    #[test]
    fn mannequins_on_surfaces_are_lifted() {
        let cam = Camera::player(10.5, 10.5, 0.0, 640, 480, 1.0);
        let c = [1.0; 4];
        let flat = ObjectOutline::mannequin(&entry(), &cam, [9.5, 9.5, 0.0], 0.0, 0.0, c);
        let table = ObjectOutline::mannequin(&entry(), &cam, [9.5, 9.5, 0.0], 0.0, 30.0, c);
        assert_eq!(table.textures[0].y, flat.textures[0].y - 2.0);
    }
}
