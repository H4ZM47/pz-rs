//! Rendering: textures (`zombie.core.textures.Texture`), texture packs and
//! the `SpriteRenderer` draw list.
//!
//! Game code records [`DrawCmd`]s; a backend (macroquad in `pz-client`)
//! executes them. This keeps simulation and UI code testable headless.

use std::rc::Rc;

/// Shared texture handle (`Texture.getSharedTexture`).
#[derive(Clone, Debug)]
pub struct Texture {
    pub name: Rc<str>,
    /// Backend image id; `None` until uploaded.
    pub id: Option<u32>,
    pub width: u32,
    pub height: u32,
    /// Sub-rectangle inside a pack page, in pixels.
    pub uv: [f32; 4],
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Rgba(pub f32, pub f32, pub f32, pub f32);

/// Blend modes the original sets through `IndieGL.glBlendFunc`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Blend {
    /// (GL_SRC_ALPHA, GL_ONE_MINUS_SRC_ALPHA)
    Alpha,
    /// (GL_SRC_ALPHA, GL_ONE)
    Additive,
}

#[derive(Clone, Debug)]
pub enum DrawCmd {
    /// `SpriteRenderer.renderi`; `texture: None` draws a solid rectangle.
    Quad { texture: Option<Texture>, x: f32, y: f32, w: f32, h: f32, color: Rgba },
    SetBlend(Blend),
    /// Scissor rectangle (`UIElement` stencil/clip), `None` clears it.
    Clip(Option<[i32; 4]>),
}

#[derive(Default)]
pub struct DrawList {
    pub cmds: Vec<DrawCmd>,
}

impl DrawList {
    pub fn quad(&mut self, texture: Option<&Texture>, x: f32, y: f32, w: f32, h: f32, color: Rgba) {
        self.cmds.push(DrawCmd::Quad { texture: texture.cloned(), x, y, w, h, color });
    }
}

/// Texture loading service, implemented by the client backend.
pub trait TextureLoader {
    /// `Texture.getSharedTexture(path)`: loose image file or pack sub-texture.
    fn shared_texture(&mut self, path: &str) -> Option<Texture>;
}
