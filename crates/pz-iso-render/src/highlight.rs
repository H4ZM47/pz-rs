//! `FBORenderObjectHighlight`: highlighted objects (context-menu targets,
//! build and fishing UIs, ...) drawn over the composited chunk textures in
//! their highlight colour, after `renderAnimatedAttachments` and the
//! corpse flies.
//!
//! An object is highlighted for the viewing player when its
//! [`RenderObject::highlight`] is set. Objects recorded into a chunk
//! texture are drawn there plainly; this pass draws each such object
//! again, mixed towards its highlight colour (`IsoObject.prepareToRender`)
//! with the shapers tinted, just in front of its chunk copy (depth less
//! `5e-5`), together with the objects stacked above it on its square so
//! those still cover it. Objects drawn every frame (the translucent
//! layers) take their highlight in their own pass.

use pz_render::DrawCmd;

use crate::alpha;
use crate::cell::{self, ChunkRenderer, Layer};
use crate::sprite::SpriteCtx;
use crate::world::{ObjectKind, RenderObject, RenderSquare};

/// One frame's highlight pass. The highlights themselves are on the
/// objects; `SavefileThumbnail.isCreatingThumbnail()` skips the pass.
#[derive(Clone, Debug, Default)]
pub struct HighlightFrame {
    pub creating_thumbnail: bool,
}

/// `isRenderedEveryFrame`: the objects drawn in the per-frame translucent
/// passes (and trees, with the default wind sprite effects).
fn rendered_every_frame(layer: Layer, obj: &RenderObject) -> bool {
    matches!(
        layer,
        Layer::Translucent | Layer::TranslucentFloor | Layer::TranslucentSE
    ) || obj.kind == ObjectKind::Tree
}

/// `FBORenderObjectHighlight.render(playerIndex)` over the frame's
/// on-screen `squares` (the original walks its registered-object set; here
/// the squares' objects in order).
pub fn render(
    r: &ChunkRenderer<'_>,
    h: &HighlightFrame,
    squares: &[&RenderSquare],
    out: &mut Vec<DrawCmd>,
) {
    if h.creating_thumbnail {
        return;
    }
    let cv = r.cutaways.as_ref();
    let mut ctx = SpriteCtx {
        cam: &r.cam,
        chunk: None,
        depth_maps: r.depth_maps,
        out,
        highlight_pass: true,
    };
    for sq in squares {
        for (index, obj) in sq.objects.iter().enumerate() {
            if obj.highlight.is_none() || obj.kind == ObjectKind::WorldInventory {
                continue;
            }
            // renderObject.
            let layer = alpha::render_layer(cv, sq, obj);
            if layer == Layer::None || alpha::target_alpha(cv, sq, obj) <= 0.0 {
                continue;
            }
            for (i, o) in sq.objects.iter().enumerate().skip(index) {
                let l = alpha::render_layer(cv, sq, o);
                if rendered_every_frame(l, o)
                    || (i != index && o.highlight.is_some())
                    || o.kind == ObjectKind::WorldInventory
                {
                    continue;
                }
                if l == Layer::None || alpha::target_alpha(cv, sq, o) <= 0.0 {
                    continue;
                }
                if l == Layer::Floor {
                    cell::render_floor(&mut ctx, sq, i);
                } else {
                    cell::render_translucent(&mut ctx, sq, o, cv);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::color::ColorInfo;
    use crate::world::{DepthMaps, DepthPreset, ObjectHighlight, SpriteFlags, SpriteInfo};
    use crate::Camera;
    use pz_render::{Texture, WorldShader};
    use std::rc::Rc;

    struct Maps;
    impl DepthMaps for Maps {
        fn preset(&self, _: DepthPreset) -> Option<Texture> {
            let mut t = Texture::empty("depth");
            t.id = Some(9);
            t.width = 128;
            t.height = 256;
            t.ready = true;
            Some(t)
        }
        fn default_texture(&self) -> Option<Texture> {
            self.preset(DepthPreset::Floor)
        }
    }

    fn sprite(name: &str, id: u32, flags: SpriteFlags) -> Rc<SpriteInfo> {
        let mut t = Texture::empty(name);
        t.id = Some(id);
        t.width = 128;
        t.height = 256;
        t.width_hw = 128;
        t.height_hw = 256;
        t.ready = true;
        Rc::new(SpriteInfo {
            name: name.into(),
            texture: Some(t),
            flags,
            tint_mod: [1.0; 3],
            ..Default::default()
        })
    }

    /// A highlighted crate on a floor: the pass draws the crate (and not
    /// the floor below it) mixed towards the highlight colour, in front of
    /// its chunk copy; the chunk recording draws it plainly.
    #[test]
    fn highlighted_object_redrawn_in_its_colour() {
        let mut sq = RenderSquare::new(10, 10, 0);
        sq.light_info = ColorInfo::new(0.5, 0.5, 0.5, 1.0);
        sq.objects.push(RenderObject::new(
            ObjectKind::Plain,
            Some(sprite(
                "floor",
                1,
                SpriteFlags {
                    solidfloor: true,
                    ..Default::default()
                },
            )),
        ));
        let mut crate_ = RenderObject::new(
            ObjectKind::Plain,
            Some(sprite("crate", 2, SpriteFlags::default())),
        );
        crate_.highlight = Some(ObjectHighlight {
            color: ColorInfo::new(1.0, 0.0, 0.0, 0.5),
            alpha_scale: 1.0,
        });
        sq.objects.push(crate_);
        let cam = Camera::centred(10.0, 10.0, 0.0, 800, 600, 1.0);
        let r = ChunkRenderer::new(cam, &Maps, (800, 600));
        let mut out = Vec::new();
        render(&r, &HighlightFrame::default(), &[&sq], &mut out);
        let sprites: Vec<_> = out
            .iter()
            .filter_map(|c| match c {
                DrawCmd::WorldSprite(s, sh) => Some((s.clone(), sh.clone())),
                _ => None,
            })
            .collect();
        assert_eq!(sprites.len(), 1);
        let (s, sh) = &sprites[0];
        assert_eq!(s.texture.as_ref().unwrap().id, Some(2));
        // 0.5 * (1 - 0.5) + 1 * 0.5 = 0.75 red, 0.25 green and blue.
        assert_eq!(s.color[0], crate::color::to_abgr(0.75, 0.25, 0.25, 1.0));
        let WorldShader::TileDepth { front, far, .. } = sh else {
            panic!("{sh:?}")
        };
        let (cx, cy) = (10, 10);
        let far0 = pz_render::iso::square_depth(cx, cy, 10.0, 10.0, 0.0);
        let front0 = pz_render::iso::square_depth(cx, cy, 11.0, 11.0, 1.0);
        assert_eq!((*front, *far), (front0 - 5.0e-5, far0 - 5.0e-5));
        // Without a highlight nothing is drawn.
        sq.objects[1].highlight = None;
        let mut out = Vec::new();
        render(&r, &HighlightFrame::default(), &[&sq], &mut out);
        assert!(out.is_empty());
    }
}
