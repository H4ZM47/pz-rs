//! `TextureCombiner`: the command list a `SmartTexture` resolves on the
//! render thread, and the backend-independent parts of
//! `TextureCombiner.combine` (the result size and each command's quad).
//!
//! A backend ([`Combiner`]) draws the commands in order into a cleared
//! RGBA8 texture taken from a pool of `TextureFBO`s keyed by size, each
//! command a quad drawn with one of the `SmartShader` programs and its
//! `glBlendFuncSeparate`, and returns the texture.

use crate::{Texture, TextureManager};

/// `TextureCombiner.coordinateSpaceMax`: `addRect` rectangles are given in
/// a 256 x 256 space scaled to the result.
pub const COORDINATE_SPACE_MAX: f32 = 256.0;

/// The size of a combined texture with no input (`getResultingWidth` of an
/// empty list).
pub const EMPTY_SIZE: u32 = 32;

/// The `SmartShader` programs `SmartTexture` creates
/// (`media/shaders/<name>.vert/.frag`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CombinerShader {
    Blit,
    /// `hueChange`: `tint` and `hue` are two programs of this shader.
    HueChange,
    OverlayMask,
    DirtMask,
    BodyMask,
    BodyMaskHue,
    BodyMaskTint,
    AddHole,
    RemoveHole,
}

impl CombinerShader {
    pub const ALL: [CombinerShader; 9] = [
        CombinerShader::Blit,
        CombinerShader::HueChange,
        CombinerShader::OverlayMask,
        CombinerShader::DirtMask,
        CombinerShader::BodyMask,
        CombinerShader::BodyMaskHue,
        CombinerShader::BodyMaskTint,
        CombinerShader::AddHole,
        CombinerShader::RemoveHole,
    ];

    /// The shader file name.
    pub fn name(self) -> &'static str {
        match self {
            CombinerShader::Blit => "blit",
            CombinerShader::HueChange => "hueChange",
            CombinerShader::OverlayMask => "overlayMask",
            CombinerShader::DirtMask => "dirtMask",
            CombinerShader::BodyMask => "bodyMask",
            CombinerShader::BodyMaskHue => "bodyMaskHue",
            CombinerShader::BodyMaskTint => "bodyMaskTint",
            CombinerShader::AddHole => "addHole",
            CombinerShader::RemoveHole => "removeHole",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|s| s.name() == name)
    }

    /// Whether the fragment shader samples `MASK`.
    pub fn uses_mask(self) -> bool {
        !matches!(self, CombinerShader::Blit | CombinerShader::HueChange)
    }

    /// The float uniforms the shader declares, with their initial values
    /// (GLSL initialisers; 0 for uninitialised uniforms).
    pub fn uniforms(self) -> &'static [(&'static str, f32)] {
        match self {
            CombinerShader::Blit => &[],
            CombinerShader::HueChange => &[("HueChange", 0.0), ("R", 1.0), ("G", 1.0), ("B", 1.0)],
            CombinerShader::OverlayMask => &[("intensity", 1.0), ("bloodDark", 0.73)],
            CombinerShader::DirtMask => &[("intensity", 1.0)],
            CombinerShader::BodyMask => &[("maskPaddingRadius", 0.0)],
            CombinerShader::BodyMaskHue => &[("HueChange", 0.0), ("maskPaddingRadius", 0.0)],
            CombinerShader::BodyMaskTint => &[
                ("R", 1.0),
                ("G", 1.0),
                ("B", 1.0),
                ("maskPaddingRadius", 0.0),
            ],
            CombinerShader::AddHole => &[("cutoffMin", 0.20), ("cutoffMax", 0.55)],
            CombinerShader::RemoveHole => &[
                ("cutoffMin", 0.0),
                ("cutoffMax", 0.00001),
                ("bloodMinAlpha", 0.25),
                ("bloodMaxAlpha", 0.975),
                ("maskPaddingRadius", 1.0 / 64.0),
            ],
        }
    }
}

/// `TextureCombinerCommand` with its textures resolved (`requireNonNull`
/// has already replaced missing ones with the error texture) and its
/// `TextureCombinerShaderParam`s evaluated (`Rand.Next(min, max)`, which is
/// `min` when the two are equal, as every `SmartTexture` parameter is).
#[derive(Clone, Debug)]
pub struct CombinerCommand {
    pub tex: Texture,
    pub mask: Option<Texture>,
    pub shader: CombinerShader,
    /// `shader.setValue(name, value)` before the draw, in order.
    pub params: Vec<(String, f32)>,
    /// `glBlendFuncSeparate(src, dst, srcA, dstA)` (GL enums).
    pub blend: [u32; 4],
    /// `x, y, w, h` in the 256 x 256 coordinate space (`addRect`), or the
    /// whole texture.
    pub rect: Option<[i32; 4]>,
}

impl CombinerCommand {
    /// The uniform values for the draw: the program's values with this
    /// command's parameters applied. Each `SmartShader` is its own GL
    /// program whose uniforms persist between draws, but every command a
    /// program draws sets the same names, so this equals the program's
    /// state at the draw.
    pub fn uniform(&self, name: &str) -> Option<f32> {
        let init = self
            .shader
            .uniforms()
            .iter()
            .find(|(n, _)| *n == name)
            .map(|(_, v)| *v)?;
        Some(
            self.params
                .iter()
                .rev()
                .find(|(n, _)| n == name)
                .map_or(init, |(_, v)| *v),
        )
    }
}

/// `PZMath.smallestEncompassingPowerOfTwo`.
fn pow2(n: u32) -> u32 {
    n.max(1).next_power_of_two()
}

/// `TextureCombiner.getResultingWidth`/`getResultingHeight`: the widest
/// (tallest) input's size rounded up to a power of two; the first command
/// wins ties.
pub fn resulting_size(cmds: &[CombinerCommand]) -> (u32, u32) {
    let dominant = |key: fn(&Texture) -> u32| {
        let mut best: Option<&Texture> = None;
        for c in cmds {
            if best.is_none_or(|b| key(&c.tex) > key(b)) {
                best = Some(&c.tex);
            }
        }
        best.map_or(EMPTY_SIZE, |t| pow2(key(t)))
    };
    (dominant(|t| t.width), dominant(|t| t.height))
}

/// One corner of a command's quad: position in the combiner's
/// `ortho2D(0, w, h, 0)` space and texture coordinates.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CombinerVertex {
    pub pos: [f32; 2],
    pub uv: [f32; 2],
}

/// The quad `combine` adds for a command (`VBORenderer.addQuad`, in its
/// corner order) into a `width` x `height` result. Position y = 0 is the
/// framebuffer's top (GL's last row), where texture v = 1 is sampled, so
/// texture rows land on the same rows of the result.
pub fn command_quad(cmd: &CombinerCommand, width: u32, height: u32) -> [CombinerVertex; 4] {
    let (w, h) = (width as f32, height as f32);
    let (x0, y0, x1, y1) = match cmd.rect {
        Some([x, y, rw, rh]) => {
            let (sx, sy) = (w / COORDINATE_SPACE_MAX, h / COORDINATE_SPACE_MAX);
            (
                x as f32 * sx,
                y as f32 * sy,
                (x + rw) as f32 * sx,
                (y + rh) as f32 * sy,
            )
        }
        None => (0.0, 0.0, w, h),
    };
    let v = |x, y, u, v| CombinerVertex {
        pos: [x, y],
        uv: [u, v],
    };
    [
        v(x0, y0, 0.0, 1.0),
        v(x0, y1, 0.0, 0.0),
        v(x1, y1, 1.0, 0.0),
        v(x1, y0, 1.0, 1.0),
    ]
}

/// A `TextureCombiner` backend.
pub trait Combiner {
    /// `TextureCombiner.combine(cmdList)`: draws the commands into a
    /// texture of [`resulting_size`] and returns it (registered in `tm`, so
    /// it can be drawn with like any texture).
    fn combine(&mut self, cmds: &[CombinerCommand], tm: &mut TextureManager) -> Texture;

    /// `TextureCombiner.releaseTexture`: returns a result to the pool.
    fn release(&mut self, tex: Texture);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tex(w: u32, h: u32) -> Texture {
        let mut t = Texture::empty("t");
        t.width = w;
        t.height = h;
        t
    }

    fn cmd(t: Texture) -> CombinerCommand {
        CombinerCommand {
            tex: t,
            mask: None,
            shader: CombinerShader::Blit,
            params: vec![],
            blend: [770, 771, 1, 771],
            rect: None,
        }
    }

    #[test]
    fn result_is_the_largest_input_rounded_up() {
        assert_eq!(resulting_size(&[]), (32, 32));
        assert_eq!(
            resulting_size(&[cmd(tex(300, 100)), cmd(tex(256, 512))]),
            (512, 512)
        );
        assert_eq!(resulting_size(&[cmd(tex(128, 64))]), (128, 64));
    }

    #[test]
    fn rect_quads_scale_from_the_256_space() {
        let mut c = cmd(tex(64, 64));
        c.rect = Some([102, 118, 52, 52]);
        let q = command_quad(&c, 512, 1024);
        assert_eq!(
            q[0],
            CombinerVertex {
                pos: [204.0, 472.0],
                uv: [0.0, 1.0]
            }
        );
        assert_eq!(
            q[2],
            CombinerVertex {
                pos: [308.0, 680.0],
                uv: [1.0, 0.0]
            }
        );
        let full = command_quad(&cmd(tex(64, 64)), 64, 32);
        assert_eq!(
            full.map(|v| v.pos),
            [[0.0, 0.0], [0.0, 32.0], [64.0, 32.0], [64.0, 0.0]]
        );
    }

    #[test]
    fn uniforms_start_at_the_glsl_initialisers() {
        let mut c = cmd(tex(1, 1));
        c.shader = CombinerShader::OverlayMask;
        c.params = vec![("intensity".into(), 0.4)];
        assert_eq!(c.uniform("intensity"), Some(0.4));
        assert_eq!(c.uniform("bloodDark"), Some(0.73));
        assert_eq!(c.uniform("R"), None);
        assert_eq!(
            CombinerShader::from_name("removeHole"),
            Some(CombinerShader::RemoveHole)
        );
    }
}
