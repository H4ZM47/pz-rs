//! Rendering: textures (`zombie.core.textures.Texture`), texture packs and
//! the `SpriteRenderer` draw list.
//!
//! Game code records [`DrawCmd`]s; a backend (macroquad in `pz-client`)
//! executes them. This keeps simulation and UI code testable headless.
//! Texture pixels are decoded here ([`TextureManager`]); the backend uploads
//! them from [`TextureManager::data`] by [`Texture::id`].

pub mod combiner;
pub mod gl;
pub mod image;
pub mod iso;
pub mod lua;
pub mod manager;
pub mod model;
pub mod png;
pub mod tile_depth;

use std::rc::Rc;

pub use image::{ImageData, Mask, MipLevel};
pub use lua::{get_shared_texture, install_shared_texture_loader, LuaTexture};
pub use manager::{SharedTextures, TextureData, TextureManager};

/// `TextureFlags`.
pub mod flags {
    pub const FILTER_MIN_NEAREST: i32 = 1;
    pub const FILTER_MAG_NEAREST: i32 = 2;
    pub const COMPRESS: i32 = 4;
    pub const CREATE_MASK: i32 = 8;
    pub const FBO: i32 = 16;
    pub const CLAMP_TO_EDGE: i32 = 32;
    pub const MIPMAPS: i32 = 64;
    pub const LIMIT_TEXTURE_SIZE_1: i32 = 128;
    pub const LIMIT_TEXTURE_SIZE_2: i32 = 256;
    pub const FBO_DEPTH: i32 = 512;
}

/// Shared texture handle (`Texture.getSharedTexture`).
#[derive(Clone, Debug)]
pub struct Texture {
    pub name: Rc<str>,
    /// Index of the backing image (`TextureID`) in [`TextureManager::data`];
    /// pack sub-textures share their page's image.
    pub id: Option<u32>,
    pub width: u32,
    pub height: u32,
    /// Sub-rectangle inside the backing image, in pixels: x, y, w, h.
    pub uv: [f32; 4],
    /// Texture coordinates of the sub-rectangle (`xStart`..`yEnd`), relative
    /// to the power-of-two image.
    pub x_start: f32,
    pub y_start: f32,
    pub x_end: f32,
    pub y_end: f32,
    /// Where the trimmed image sits inside its original frame.
    pub offset_x: f32,
    pub offset_y: f32,
    /// Original frame size; 0 means "same as width/height".
    pub width_orig: i32,
    pub height_orig: i32,
    /// Power-of-two size of the backing image.
    pub width_hw: i32,
    pub height_hw: i32,
    pub solid: bool,
    pub flip: bool,
    pub mask: Option<Rc<Mask>>,
    /// `isReady()`; false once loading failed (`isFailure()`).
    pub ready: bool,
}

impl Texture {
    /// A texture with no image yet (`new Texture()`).
    pub fn empty(name: &str) -> Self {
        Texture {
            name: Rc::from(name),
            id: None,
            width: 0,
            height: 0,
            uv: [0.0; 4],
            x_start: 0.0,
            y_start: 0.0,
            x_end: 1.0,
            y_end: 1.0,
            offset_x: 0.0,
            offset_y: 0.0,
            width_orig: 0,
            height_orig: 0,
            width_hw: 0,
            height_hw: 0,
            solid: false,
            flip: false,
            mask: None,
            ready: false,
        }
    }

    /// `getWidthOrig()`.
    pub fn width_orig(&self) -> i32 {
        if self.width_orig == 0 { self.width as i32 } else { self.width_orig }
    }

    /// `getHeightOrig()`.
    pub fn height_orig(&self) -> i32 {
        if self.height_orig == 0 { self.height as i32 } else { self.height_orig }
    }

    /// `isMaskSet(x, y)`: true without a mask.
    pub fn is_mask_set(&self, x: i32, y: i32) -> bool {
        self.mask.as_ref().is_none_or(|m| m.get(x - self.offset_x as i32, y - self.offset_y as i32))
    }

    /// `setRegion(x, y, w, h)`, including its height clamp that ignores `y`.
    pub fn set_region(&mut self, x: i32, y: i32, mut width: i32, mut height: i32) {
        let (whw, hhw) = (self.width_hw, self.height_hw);
        if x < 0 || x > whw || y < 0 || y > hhw || width <= 0 || height <= 0 {
            return;
        }
        if width + x > whw {
            width = whw - x;
        }
        if height > hhw {
            height = hhw - y;
        }
        self.x_start = x as f32 / whw as f32;
        self.y_start = y as f32 / hhw as f32;
        self.x_end = (x + width) as f32 / whw as f32;
        self.y_end = (y + height) as f32 / hhw as f32;
        self.width = width.max(0) as u32;
        self.height = height.max(0) as u32;
        self.uv = [x as f32, y as f32, width as f32, height as f32];
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rgba(pub f32, pub f32, pub f32, pub f32);

/// `Color.colorToABGR`: channels clamped to 0..1 and truncated to bytes.
pub fn color_to_abgr(r: f32, g: f32, b: f32, a: f32) -> u32 {
    let byte = |v: f32| (v.clamp(0.0, 1.0) * 255.0) as i32 as u32 & 0xFF;
    byte(a) << 24 | byte(b) << 16 | byte(g) << 8 | byte(r)
}

/// Blend modes the original sets through `IndieGL.glBlendFunc`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Blend {
    /// (GL_SRC_ALPHA, GL_ONE_MINUS_SRC_ALPHA)
    Alpha,
    /// (GL_SRC_ALPHA, GL_ONE)
    Additive,
}

/// `TextureDraw` for a `glDraw`: one quad, corners clockwise from the top
/// left, with per-corner ABGR colours and texture coordinates.
#[derive(Clone, Debug, PartialEq)]
pub struct Sprite {
    pub texture: Option<Texture>,
    pub x: [f32; 4],
    pub y: [f32; 4],
    pub u: [f32; 4],
    pub v: [f32; 4],
    pub color: [u32; 4],
    pub flipped: bool,
}

impl PartialEq for Texture {
    fn eq(&self, other: &Self) -> bool {
        self.name == other.name && self.id == other.id && self.uv == other.uv
    }
}

#[derive(Clone, Debug)]
// Commands are recorded by the thousand per frame; boxing the large world
// variants would cost an allocation each.
#[allow(clippy::large_enum_variant)]
pub enum DrawCmd {
    /// A plain rectangle in the scaffold's form; `texture: None` draws a
    /// solid rectangle.
    Quad { texture: Option<Texture>, x: f32, y: f32, w: f32, h: f32, color: Rgba },
    /// A quad as `SpriteRenderer` records it.
    Sprite(Sprite),
    SetBlend(Blend),
    /// `glBlendFunc(src, dst)` with GL enum values.
    BlendFunc(u32, u32),
    /// `glBlendFuncSeparate(srcRGB, dstRGB, srcA, dstA)`.
    BlendFuncSeparate(u32, u32, u32, u32),
    GlEnable(u32),
    GlDisable(u32),
    StencilMask(u32),
    StencilFunc(u32, i32, u32),
    StencilOp(u32, u32, u32),
    ColorMask(bool, bool, bool, bool),
    AlphaFunc(u32, f32),
    Clear(u32),
    /// Scissor rectangle (`UIElement` stencil/clip), `None` clears it.
    Clip(Option<[i32; 4]>),
    /// `glDepthFunc` (GL comparison enum).
    DepthFunc(u32),
    /// `glDepthMask`.
    DepthMask(bool),
    /// A world sprite drawn with one of the world shaders.
    WorldSprite(Sprite, WorldShader),
    /// A new projection and viewport, as the `Core.DoStartFrameStuff`
    /// variants set: `ortho2D(left, right, bottom, top)` over a viewport of
    /// `width` x `height`.
    Ortho { left: f32, right: f32, bottom: f32, top: f32, width: u32, height: u32 },
    /// `glViewport(x, y, width, height)` in GL window coordinates (origin
    /// at the bottom-left of the bound framebuffer): a split-screen
    /// player's quarter (`DoStartFrameStuff` with a player index,
    /// `ModelOutlines.endFrame`'s `getScreenLeft`/`getScreenTop`).
    /// [`DrawCmd::Ortho`] keeps the origin and sets only the size; texture
    /// targets (`ChunkBegin`, `AtlasBegin`) start at the origin and the
    /// previous origin comes back when they end.
    Viewport { x: u32, y: u32, width: u32, height: u32 },
    /// `FBORenderChunk.beginRenderThread`: following draws go to chunk
    /// texture `id` (colour plus 16-bit depth), created at this size if
    /// needed; `clear` clears colour to 0 and depth to 1.
    ChunkBegin { id: u64, width: u32, height: u32, clear: bool },
    /// `FBORenderChunk.endRenderThread`: back to the target bound before
    /// `ChunkBegin`.
    ChunkEnd,
    /// `FBORenderChunk.renderInWorldMainThread`: chunk texture `id` drawn
    /// with `chunkShader` (colour times `color`, depth = `chunk_depth` +
    /// stored depth). `mag_linear` is the `GL_LINEAR` magnification used at
    /// zoom 0.75.
    ChunkComposite { id: u64, x: f32, y: f32, w: f32, h: f32, color: u32, chunk_depth: f32, mag_linear: bool },
    /// Releases chunk texture `id` (`FBORenderChunk.destroy`).
    ChunkFree(u64),
    /// `Core.StartFrame`'s `glBuffer(1)`: following draws go to the
    /// offscreen world buffer (`MultiTextureFBO2`), a texture of this size
    /// with depth and stencil, created if needed.
    OffscreenBegin { width: u32, height: u32 },
    /// Back to the main target.
    OffscreenEnd,
    /// `MultiTextureFBO2.render`: the offscreen buffer drawn to the screen
    /// quad `x, y, w, h` from its texel rectangle `tex` (x, y, w, h) with
    /// the `screen` shader.
    ScreenPass { x: f32, y: f32, w: f32, h: f32, tex: [f32; 4], params: ScreenParams },
    /// A `VBORenderer` quad already projected: corners in GL normalised
    /// device coordinates with window depth (`vboRenderer_PositionColorUV`
    /// after its `userDepth` adjustment), texture coordinates and colour.
    /// Texels with alpha below 0.01 are discarded; `nearest` forces
    /// `GL_NEAREST` sampling (`setMinMagFilters`).
    ProjectedQuad { texture: Texture, pos: [[f32; 3]; 4], uv: [[f32; 2]; 4], color: u32, nearest: bool },
    /// A `VBORenderer` quad with per-vertex depth
    /// (`formatPositionColorUvDepth`, `vboRenderer_PositionColorUVDepth`):
    /// corners in normalised device coordinates with each corner's window
    /// depth written as the fragment depth (interpolated), colour times
    /// texel with no discard. `FBORenderShadows` draws drop shadows so.
    ProjectedDepthQuad { texture: Texture, pos: [[f32; 3]; 4], uv: [[f32; 2]; 4], color: u32 },
    /// [`DrawCmd::ProjectedQuad`] with a float colour, as `VBORenderer`'s
    /// `formatPositionColorUv` carries it (`WeatherParticleDrawer`): faint
    /// colours are not rounded to 8 bits before the texel is multiplied.
    ProjectedQuadF32 { texture: Texture, pos: [[f32; 3]; 4], uv: [[f32; 2]; 4], color: [f32; 4], nearest: bool },
    /// `glPushAttrib(GL_ALL_ATTRIB_BITS)`: the fixed-function state
    /// (blending, alpha, depth, stencil, colour mask, scissor) saved.
    PushAttrib,
    /// `glPopAttrib()`: the state saved by the matching
    /// [`DrawCmd::PushAttrib`] restored.
    PopAttrib,
    /// `TextureFBO.startDrawing` on an atlas texture (`DeadBodyAtlas`,
    /// `WorldItemAtlas`, `ModelManager.bitmap`): following draws go to
    /// atlas `id` (colour sampled `GL_LINEAR` without mips, 16-bit depth
    /// sampled `GL_NEAREST`), created at this size if needed; `clear`
    /// clears colour to 0 and depth to 1. The projection, viewport and GL
    /// state are saved (`glPushAttrib`) and the viewport set to the
    /// atlas. Atlas textures are addressed as GL's bottom-up framebuffers
    /// (a scissor box's y counts from the bottom row).
    AtlasBegin { id: u64, width: u32, height: u32, clear: bool },
    /// `endDrawing`: back to the previous target, restoring what
    /// `AtlasBegin` saved.
    AtlasEnd,
    /// A quad drawn with the `DeadBodyAtlas` shader from atlas `id`:
    /// corners (top-left, top-right, bottom-right, bottom-left) in the
    /// current projection, texture coordinates, colour times texel, and
    /// fragment depth `depth[0] + (depth[1] - depth[0]) * d` from the
    /// atlas depth `d`; texels with `alpha * d == 0` are discarded.
    AtlasQuad { id: u64, pos: [[f32; 2]; 4], uv: [[f32; 2]; 4], color: u32, depth: [f32; 2] },
    /// A 3D model draw (`basicEffect`), depth-tested against the world.
    Model(Box<model::ModelDraw>),
    /// A vehicle mesh (`Model.DrawVehicle` with a vehicle shader).
    VehicleModel(Box<model::VehicleDraw>),
    /// `ModelOutlines.fboA.startDrawing(clear, true)` (after
    /// `checkFBOs`): following draws go to the outline mask, a
    /// `width` x `height` texture (`Core.width`/`height`, stored at the
    /// next powers of two) created if needed; `clear` clears it to
    /// transparent. The projection and viewport stay as they are.
    OutlineBegin { width: u32, height: u32, clear: bool },
    /// `fboA.endDrawing()`: back to the target bound before.
    OutlineEnd,
    /// A mesh drawn into the outline mask with `aim_outline_solid`
    /// (`aim_outline_solid_static` for static meshes): the draw's
    /// matrices, palette and `FinalScale` (static meshes take no
    /// `FinalScale`, neither takes `targetDepth`), `u_color` =
    /// (`color`, 1) where the texture's alpha is at least 0.5 and
    /// transparent elsewhere. No depth writes (`depthMask` false).
    OutlineModel { draw: Box<model::ModelDraw>, color: [f32; 3] },
    /// `FBORenderObjectOutline.Drawer2`: a sprite (an object's sprite or
    /// overlay texture, or an atlas entry when `atlas` is set) drawn into
    /// the outline mask with `vboRenderer_SpriteOutline` under the current
    /// projection: (`color.rgb`, `color.a` x step(0.5, texel alpha)),
    /// blended (SRC_ALPHA, ONE_MINUS_SRC_ALPHA). Corners top-left,
    /// top-right, bottom-right, bottom-left.
    OutlineSprite { texture: Option<Texture>, atlas: Option<u64>, pos: [[f32; 2]; 4], uv: [[f32; 2]; 4], color: [f32; 4] },
    /// `ModelOutlines.endFrame`'s thickening: the mask's alpha blurred
    /// horizontally (`aim_outline_h`) into the second texture and that
    /// vertically (`aim_outline_v`) into the third, each cleared first and
    /// drawn with (SRC_ALPHA, ONE_MINUS_SRC_ALPHA) over the current
    /// viewport ([`DrawCmd::Viewport`]: the player's part of the screen). `u_resolution` = `resolution` (the mask's
    /// unpadded size), `u_radius` = `radius`, `u_color` = `color`.
    OutlineThicken { color: [f32; 4], radius: f32, resolution: [f32; 2] },
    /// `endFrame`'s blit (`aim_outline_blit`): the thickened outline drawn
    /// over the current target's viewport with its alpha times one minus
    /// the mask's alpha, blended (SRC_ALPHA, ONE_MINUS_SRC_ALPHA), no
    /// depth test.
    OutlineBlit,
    /// An `IsoWater`/`IsoPuddles` vertex-buffer draw (`ModelManager.RenderWater`,
    /// `RenderPuddles`, `IsoPuddles.RenderToChunkTexture`) with the water or
    /// puddles shader.
    Surface(Box<SurfaceDraw>),
    /// `ModelManager.RenderSkyBox`: the sky drawn into one of the two
    /// 512x512 reflection textures ([`skybox_texture`]) with the `skybox` or
    /// `skybox_hires` shader.
    SkyBox(Box<SkyBoxDraw>),
    /// `glBlendEquation` (`gl::FUNC_ADD`, `gl::FUNC_REVERSE_SUBTRACT`, ...).
    BlendEquation(u32),
    /// `WeatherFxMask`'s `glBuffer(4)`/`glBuffer(6)`: following draws go to
    /// weather texture `index` (0 `fboMask`, 1 `fboParticles`; see
    /// [`weather_fx_texture`]), screen-sized, created at this size if
    /// needed and cleared to transparent black (`startDrawing(true, true)`).
    WeatherFxBegin { index: usize, width: u32, height: u32 },
    /// `glBuffer(5)`/`glBuffer(7)`: back to the target bound before
    /// `WeatherFxBegin`.
    WeatherFxEnd,
    /// A quad drawn with the `fogCircle` shader (`IsoWeatherFX.renderFogCircle`):
    /// texture times colour, positions through its own `u_mvp`,
    /// `ortho(0, width, height, 0)`.
    FogCircle { sprite: Sprite, width: f32, height: f32 },
    /// `ModelManager.RenderParticles`' instanced draw with the `fire` or
    /// `smoke` shader (`ParticlesFire`).
    Particles(Box<ParticlesDraw>),
    /// `ImprovedFogDrawer.render` (`FBORenderCell.renderFog`): each
    /// rectangle a `formatPositionColorUvDepth` quad in the current
    /// projection drawn with the `fog` shader over `ImprovedFog`'s noise
    /// texture, blended (SRC_ALPHA, ONE_MINUS_SRC_ALPHA), depth-tested
    /// `GL_LESS` without depth writes, each corner's window depth written
    /// as the fragment depth. The GL state is restored afterwards.
    ImprovedFog(Box<FogDraw>),
}

/// The uniforms and rectangles of one `ImprovedFogDrawer` frame.
#[derive(Clone, Debug, PartialEq)]
pub struct FogDraw {
    /// `ImprovedFog.getNoiseTexture()` (`NoiseTexture`, unit 0).
    pub noise: Option<Texture>,
    /// `screenInfo1..3`: the offscreen width, height and zoom (the fourth
    /// component is each rectangle's layer alpha).
    pub screen: [f32; 3],
    /// `textureInfo`: debug colours, octaves, base alpha, tile scale.
    pub texture_info: [f32; 4],
    /// `worldOffset`: camera offset and right-click offset.
    pub world_offset: [f32; 4],
    /// `scalingInfo1, 2` (wind scroll) and `scalingInfo4`; the third
    /// component is each rectangle's level.
    pub scaling: [f32; 3],
    /// `colorInfo`.
    pub color_info: [f32; 4],
    /// `paramInfo`: top and bottom alpha heights, alpha-circle alpha and
    /// radius.
    pub param_info: [f32; 4],
    /// `cameraInfo1..3`: offscreen left, top, zoom (the fourth component
    /// is each rectangle's row offset).
    pub camera: [f32; 3],
    /// The quads' vertex alpha (`alpha`; white otherwise).
    pub alpha: f32,
    pub rects: Vec<FogRect>,
}

/// One rectangle of an [`FogDraw`] (`addRectangle`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FogRect {
    /// `sx, sy, ex, ey` in the current projection.
    pub pos: [f32; 4],
    /// `u0, v0, u1, v1`.
    pub uv: [f32; 4],
    /// Window depth of the corners (top-left, top-right, bottom-right,
    /// bottom-left).
    pub depth: [f32; 4],
    /// The row's noise offset (`cameraInfo.w`).
    pub offset: f32,
    /// `screenInfo.w`.
    pub layer_alpha: f32,
    /// `scalingInfo.z`.
    pub z_layer: i32,
}

/// The `fog` shader's uniforms for one rectangle.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FogUniforms {
    pub screen_info: [f32; 4],
    pub texture_info: [f32; 4],
    pub rectangle_info: [f32; 4],
    pub scaling_info: [f32; 4],
    pub color_info: [f32; 4],
    pub world_offset: [f32; 4],
    pub param_info: [f32; 4],
    pub camera_info: [f32; 4],
}

impl FogDraw {
    /// The uniforms rectangle `i` is drawn with: `setScreenInfo2`,
    /// `setRectangleInfo2` (integer-truncated), `setCameraInfo2` and
    /// `setScalingInfo2` (whose level holds over from the previous
    /// rectangle until it changes, the same value either way).
    pub fn uniforms(&self, i: usize) -> FogUniforms {
        let r = &self.rects[i];
        let [sx, sy, ex, ey] = r.pos;
        let t = |v: f32| v as i32 as f32;
        FogUniforms {
            screen_info: [self.screen[0], self.screen[1], self.screen[2], r.layer_alpha],
            texture_info: self.texture_info,
            rectangle_info: [t(sx), t(sy), t(ex - sx), t(ey - sy)],
            scaling_info: [self.scaling[0], self.scaling[1], r.z_layer as f32, self.scaling[2]],
            color_info: self.color_info,
            world_offset: self.world_offset,
            param_info: self.param_info,
            camera_info: [self.camera[0], self.camera[1], self.camera[2], r.offset],
        }
    }
}

/// The shader of a [`ParticlesDraw`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ParticleProgram {
    /// `fire` (`FireShader`): flames from `FireFlame.png`'s 8x4 frames.
    Fire,
    /// `smoke` (`SmokeShader`): puffs from `FireSmokes.png`'s 2x2 cells.
    Smoke,
}

/// One `glDrawArraysInstanced` of the particle quad (`Particles.initBuffers`'
/// strip from -1 to 1) over `particles`. The draw pushes and pops the GL
/// attributes: blending with `blend_func`, no depth test, viewport
/// `viewport`.
#[derive(Clone, Debug, PartialEq)]
pub struct ParticlesDraw {
    pub program: ParticleProgram,
    /// `mvpMatrix`, column major (`m[col][row]`).
    pub mvp: [[f32; 4]; 4],
    /// `FireTime`.
    pub time: f32,
    /// `FireParam[col][row]`.
    pub param: [[f32; 3]; 3],
    /// `FireTexture` (unit 0).
    pub texture: Option<Texture>,
    /// Attribute 1 (`Particule`) per instance: x, y, id, slot fraction.
    pub particles: Vec<[f32; 4]>,
    /// `glBlendFunc(src, dst)`.
    pub blend_func: [u32; 2],
    pub viewport: (u32, u32),
}

/// The ids of `WeatherFxMask`'s two screen-sized render textures
/// (`fboMask`, `fboParticles`), reserved like [`SKYBOX_TEXTURE_IDS`] so
/// draws can sample them.
pub const WEATHER_FX_TEXTURE_IDS: [u32; 2] = [0xFFFF_FE10, 0xFFFF_FE11];

/// A [`Texture`] naming weather texture `index` at `width` x `height`
/// (`new Texture(w, h, 16)`: its power-of-two size is its own).
pub fn weather_fx_texture(index: usize, width: u32, height: u32) -> Texture {
    let mut t = Texture::empty(if index == 0 { "WeatherFxMask.fboMask" } else { "WeatherFxMask.fboParticles" });
    t.id = Some(WEATHER_FX_TEXTURE_IDS[index.min(1)]);
    t.width = width;
    t.height = height;
    t.width_hw = width as i32;
    t.height_hw = height as i32;
    t.uv = [0.0, 0.0, width as f32, height as f32];
    t.ready = true;
    t
}

/// The ids of `SkyBox`'s two render textures (`textureFboA`/`B`), reserved
/// above any [`TextureManager`] id so surface draws can sample them like
/// any texture.
pub const SKYBOX_TEXTURE_IDS: [u32; 2] = [0xFFFF_FE00, 0xFFFF_FE01];

/// The size of the sky textures.
pub const SKYBOX_TEXTURE_SIZE: u32 = 512;

/// A [`Texture`] naming sky texture `index` (0 = A, 1 = B).
pub fn skybox_texture(index: usize) -> Texture {
    let mut t = Texture::empty(if index == 0 { "SkyBox.textureFboA" } else { "SkyBox.textureFboB" });
    t.id = Some(SKYBOX_TEXTURE_IDS[index]);
    t.width = SKYBOX_TEXTURE_SIZE;
    t.height = SKYBOX_TEXTURE_SIZE;
    t.width_hw = SKYBOX_TEXTURE_SIZE as i32;
    t.height_hw = SKYBOX_TEXTURE_SIZE as i32;
    t.uv = [0.0, 0.0, SKYBOX_TEXTURE_SIZE as f32, SKYBOX_TEXTURE_SIZE as f32];
    t.ready = true;
    t
}

/// `SkyBoxShader`'s uniforms.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SkyBoxUniforms {
    /// `SBTime`.
    pub time: f32,
    pub cloud_count: f32,
    pub cloud_size: f32,
    pub sun_light: [f32; 3],
    pub sun_colour: [f32; 3],
    pub sky_h_colour: [f32; 3],
    pub sky_l_colour: [f32; 3],
    pub cloud_light: f32,
    pub stars: f32,
    pub fog: f32,
    pub wind: [f32; 3],
}

/// One sky render.
#[derive(Clone, Debug, PartialEq)]
pub struct SkyBoxDraw {
    /// Which of [`SKYBOX_TEXTURE_IDS`] to draw into.
    pub target: usize,
    /// `skybox_hires` (`Core.getPerfSkybox() == 0`).
    pub hires: bool,
    pub uniforms: SkyBoxUniforms,
}

/// The shader of a [`SurfaceDraw`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SurfaceProgram {
    /// `water` (water quality 1).
    Water,
    /// `water_hq` (water quality 0).
    WaterHq,
    /// `puddles_hq` (puddles quality 0).
    PuddlesHq,
    /// `puddles_mq` (puddles quality 1).
    PuddlesMq,
    /// `puddles_lq` (puddles quality 2, drawn into chunk textures).
    PuddlesLq,
}

impl SurfaceProgram {
    pub fn is_water(self) -> bool {
        matches!(self, SurfaceProgram::Water | SurfaceProgram::WaterHq)
    }
}

/// One vertex of an `IsoWater`/`IsoPuddles` quad as `RenderData.addSquare`
/// packs it: four shader attributes (water: depth, flow, speed, external;
/// puddles: dirNE, dirNW, dirAll, dirNone), the position, the ABGR vertex
/// light and the fragment depth (`aFragDepth`, window depth 0..1).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SurfaceVertex {
    pub attr: [f32; 4],
    pub pos: [f32; 2],
    pub color: u32,
    pub depth: f32,
}

/// The uniforms `WaterShader.updateWaterParams` / `PuddlesShader.updatePuddlesParams`
/// set.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SurfaceUniforms {
    /// `WTime`.
    pub time: f32,
    /// `WOffset` (already shifted by -90000, -640000).
    pub offset: [f32; 4],
    /// `WViewport`.
    pub viewport: [f32; 4],
    /// `WReflectionParam` (`SkyBox.getTextureShift`).
    pub reflection_param: f32,
    /// Water: `[WParamWind.x, WParamWind.y, WParamWindSpeed,
    /// WParamRainIntensity]` and zeros. Puddles: columns 0 and 1 of
    /// `PuddlesParams` (`[windX, windY, windSpeed, z]`, `[muddy, wetGround,
    /// size, rainIntensity]`).
    pub params: [[f32; 4]; 2],
}

/// A batch of water or puddle quads. The draw sets its own GL state for its
/// duration (`glPushAttrib`/`glPopAttrib` in the original): depth test on
/// with `depth_func`, depth writes off, blending with `blend_func`.
#[derive(Clone, Debug, PartialEq)]
pub struct SurfaceDraw {
    pub program: SurfaceProgram,
    pub quads: Vec<[SurfaceVertex; 4]>,
    pub uniforms: SurfaceUniforms,
    /// Texture unit 0: `WaterGroundTex` (`river_bottom.png`) for water,
    /// `PuddlesHM` (`puddles_hm.png`) for puddles.
    pub ground: Option<Texture>,
    /// `WaterTextureReflectionA`/`B` (`SkyBox.getTextureCurrent/Prev`).
    pub reflection: [Option<Texture>; 2],
    pub depth_func: u32,
    pub blend_func: [u32; 4],
}

/// The `WeatherShader` uniforms the screen pass reads.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScreenParams {
    /// `DesaturationVal` (`PlayerRenderSettings.getDesaturation`).
    pub desaturation: f32,
    /// `timer`: the frame counter that animates the grain.
    pub timer: f32,
}

/// The shader a [`DrawCmd::WorldSprite`] is drawn with.
#[derive(Clone, Debug, PartialEq)]
pub enum WorldShader {
    /// `default`: texture times colour, transparent texels discarded, depth
    /// `chunkDepth + zDepth` in GL clip space (-1..1).
    Default { z: f32 },
    /// `tileWithDepth` (or `opaqueWithDepth`, which also drops texels with
    /// alpha <= 0.8): depth from the greyscale depth texture, blended from
    /// `front` (texel 0) to `far` (texel 1); texels with depth 0 are
    /// discarded. `uv1` holds the depth texture coordinates of the four
    /// corners.
    TileDepth { depth: Texture, uv1: [[f32; 2]; 4], front: f32, far: f32, opaque: bool },
    /// `seamFix2` (`TileSeamShader`): as `TileDepth`, also discarding where
    /// the seam mask (texture coordinates `uv2`) has alpha 0.
    Seam { depth: Texture, uv1: [[f32; 2]; 4], mask: Texture, uv2: [[f32; 2]; 4], front: f32, far: f32 },
    /// `CircleStencil` (`DoCutawayShader`): a wall cut down by the stencil
    /// texture (`wallcutaways.png`): texels where the stencil's red channel
    /// is zero are discarded when `discard_value` > 0 (kept otherwise, for
    /// window-frame outlines), the stencil's channels shape alpha and add
    /// the edge highlight, and depth comes from `depth`.
    Cutaway { depth: Texture, uv1: [[f32; 2]; 4], stencil: Texture, uv2: [[f32; 2]; 4], front: f32, far: f32, discard_value: f32 },
    /// `CutawayNoDepth` (`DoCutawayShaderSprite`, the weather mask's
    /// walls): texels where the stencil (texture coordinates `uv2`) has
    /// green or blue are discarded, the rest multiplied by the vertex
    /// colour's rgb and the stencil's `rrra`, with the same edge terms as
    /// `CircleStencil`; no depth is written.
    CutawayNoDepth { stencil: Texture, uv2: [[f32; 2]; 4] },
}

#[derive(Default)]
pub struct DrawList {
    pub cmds: Vec<DrawCmd>,
}

impl DrawList {
    pub fn quad(&mut self, texture: Option<&Texture>, x: f32, y: f32, w: f32, h: f32, color: Rgba) {
        self.cmds.push(DrawCmd::Quad { texture: texture.cloned(), x, y, w, h, color });
    }

    /// `TextureDraw.Create(texd, tex, x0..y3, c0..c3)` with the texture's own
    /// coordinates.
    fn push_corners(&mut self, tex: Option<&Texture>, x: [f32; 4], y: [f32; 4], color: [u32; 4]) {
        let (u, v) = match tex {
            Some(t) => ([t.x_start, t.x_end, t.x_end, t.x_start], [t.y_start, t.y_start, t.y_end, t.y_end]),
            None => ([0.0, 1.0, 1.0, 0.0], [0.0, 0.0, 1.0, 1.0]),
        };
        self.push_uv(tex, x, y, u, v, color);
    }

    fn push_uv(&mut self, tex: Option<&Texture>, x: [f32; 4], y: [f32; 4], u: [f32; 4], v: [f32; 4], color: [u32; 4]) {
        let flipped = tex.is_some_and(|t| t.flip);
        self.cmds.push(DrawCmd::Sprite(Sprite { texture: tex.cloned(), x, y, u, v, color, flipped }));
    }

    /// `Texture.render(x, y, w, h, r, g, b, a)`: offsets by the texture's
    /// trim offset, then `SpriteRenderer.render`.
    pub fn render_texture(&mut self, tex: &Texture, x: f32, y: f32, w: f32, h: f32, c: Rgba) {
        self.render(Some(tex), x + tex.offset_x, y + tex.offset_y, w, h, c);
    }

    /// `SpriteRenderer.render(tex, x, y, w, h, r, g, b, a)` with
    /// `fboRenderChunk` on (the B42 default): no pixel snapping. Skipped
    /// when the texture is not ready or alpha is 0.
    pub fn render(&mut self, tex: Option<&Texture>, x: f32, y: f32, w: f32, h: f32, c: Rgba) {
        if tex.is_some_and(|t| !t.ready) || c.3 == 0.0 {
            return;
        }
        let col = color_to_abgr(c.0, c.1, c.2, c.3);
        let (x2, y2) = (x + w, y + h);
        self.push_corners(tex, [x, x2, x2, x], [y, y, y2, y2], [col; 4]);
    }

    /// `SpriteRenderer.renderi`.
    pub fn renderi(&mut self, tex: Option<&Texture>, x: i32, y: i32, w: i32, h: i32, c: Rgba) {
        self.render(tex, x as f32, y as f32, w as f32, h as f32, c);
    }

    /// `SpriteRenderer.render(tex, x, y, w, h, r, g, b, a, u1..v4)`.
    #[allow(clippy::too_many_arguments)]
    pub fn render_uv(&mut self, tex: Option<&Texture>, x: f32, y: f32, w: f32, h: f32, c: Rgba, u: [f32; 4], v: [f32; 4]) {
        if c.3 == 0.0 {
            return;
        }
        let col = color_to_abgr(c.0, c.1, c.2, c.3);
        let (x2, y2) = (x + w, y + h);
        self.push_uv(tex, [x, x2, x2, x], [y, y, y2, y2], u, v, [col; 4]);
    }

    /// `SpriteRenderer.renderClamped`: clips the quad (and its texture
    /// coordinates) to a rectangle.
    #[allow(clippy::too_many_arguments)]
    pub fn render_clamped(&mut self, tex: Option<&Texture>, x: i32, y: i32, w: i32, h: i32, clamp: [i32; 4], c: Rgba) {
        let [cx, cy, cw, ch] = clamp;
        let x1 = x.clamp(cx, cx + cw);
        let y1 = y.clamp(cy, cy + ch);
        let x2 = (x + w).clamp(cx, cx + cw);
        let y2 = (y + h).clamp(cy, cy + ch);
        if x1 == x2 || y1 == y2 {
            return;
        }
        let (dx1, dx2, dy1, dy2) = (x1 - x, x + w - x2, y1 - y, y + h - y2);
        if dx1 == 0 && dx2 == 0 && dy1 == 0 && dy2 == 0 {
            self.renderi(tex, x, y, w, h, c);
            return;
        }
        let (mut u0, mut v0, mut u1, mut v1) = (0.0, 0.0, 1.0, 1.0);
        if let Some(t) = tex {
            u0 = dx1 as f32 / w as f32;
            v0 = dy1 as f32 / h as f32;
            u1 = (w - dx2) as f32 / w as f32;
            v1 = (h - dy2) as f32 / h as f32;
            let (us, vs) = (t.x_end - t.x_start, t.y_end - t.y_start);
            u0 = t.x_start + u0 * us;
            u1 = t.x_start + u1 * us;
            v0 = t.y_start + v0 * vs;
            v1 = t.y_start + v1 * vs;
        }
        self.render_uv(tex, x1 as f32, y1 as f32, (x2 - x1) as f32, (y2 - y1) as f32, c, [u0, u1, u1, u0], [v0, v0, v1, v1]);
    }

    /// `SpriteRenderer.renderRect`: a solid rectangle.
    pub fn render_rect(&mut self, x: i32, y: i32, w: i32, h: i32, c: Rgba) {
        if c.3 == 0.0 {
            return;
        }
        self.render(None, x as f32, y as f32, w as f32, h as f32, c);
    }

    /// `SpriteRenderer.renderPoly(tex, x1..y4, r, g, b, a)`.
    #[allow(clippy::too_many_arguments)]
    pub fn render_poly(&mut self, tex: Option<&Texture>, x: [f32; 4], y: [f32; 4], c: Rgba) {
        if tex.is_some_and(|t| !t.ready) {
            return;
        }
        let col = color_to_abgr(c.0, c.1, c.2, c.3);
        self.push_corners(tex, x, y, [col; 4]);
    }

    /// `SpriteRenderer.render(tex, x1, y1, ..., x4, y4, r, g, b, a)`: four
    /// free corners, drawn even when the texture is not ready.
    pub fn render4(&mut self, tex: Option<&Texture>, x: [f32; 4], y: [f32; 4], c: Rgba) {
        let col = color_to_abgr(c.0, c.1, c.2, c.3);
        self.push_corners(tex, x, y, [col; 4]);
    }

    /// `SpriteRenderer.renderline(tex, x1, y1, x2, y2, r, g, b, a, thickness)`.
    #[allow(clippy::too_many_arguments)]
    pub fn render_line(&mut self, tex: Option<&Texture>, x1: f32, y1: f32, x2: f32, y2: f32, c: Rgba, t: f32) {
        let (xs, ys) = if x1 <= x2 && y1 <= y2 {
            ([x1 + t, x2 + t, x2 - t, x1 - t], [y1 - t, y2 - t, y2 + t, y1 + t])
        } else if x1 >= x2 && y1 >= y2 {
            ([x1 + t, x1 - t, x2 - t, x2 + t], [y1 - t, y1 + t, y2 + t, y2 - t])
        } else if x1 >= x2 && y1 <= y2 {
            ([x2 - t, x1 - t, x1 + t, x2 + t], [y2 - t, y1 - t, y1 + t, y2 + t])
        } else {
            ([x1 - t, x1 + t, x2 + t, x2 - t], [y1 - t, y1 + t, y2 + t, y2 - t])
        };
        let col = color_to_abgr(c.0, c.1, c.2, c.3);
        self.push_corners(tex, xs, ys, [col; 4]);
    }

    /// `SpriteRenderer.renderline(tex, int x1, y1, x2, y2, r, g, b, a)`:
    /// one pixel thick.
    pub fn render_line_i(&mut self, tex: Option<&Texture>, x1: i32, y1: i32, x2: i32, y2: i32, c: Rgba) {
        self.render_line(tex, x1 as f32, y1 as f32, x2 as f32, y2 as f32, c, 1.0);
    }

    pub fn blend_func(&mut self, src: u32, dst: u32) {
        self.cmds.push(DrawCmd::BlendFunc(src, dst));
    }

    pub fn gl_enable(&mut self, cap: u32) {
        self.cmds.push(DrawCmd::GlEnable(cap));
    }

    pub fn gl_disable(&mut self, cap: u32) {
        self.cmds.push(DrawCmd::GlDisable(cap));
    }
}

/// Texture loading service, implemented by the client backend.
pub trait TextureLoader {
    /// `Texture.getSharedTexture(path)`: loose image file or pack sub-texture.
    fn shared_texture(&mut self, path: &str) -> Option<Texture>;

    /// Width and height of the backing image (`TextureID`) `id`, when the
    /// loader knows it.
    fn image_size(&self, _id: u32) -> Option<(i32, i32)> {
        None
    }
}

#[cfg(test)]
mod tests;
