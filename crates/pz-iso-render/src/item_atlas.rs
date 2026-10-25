//! `WorldItemAtlas` and `WorldItemModelDrawer.renderAtlasTexture`: placed
//! items' 3D models rendered once into 512x512 atlas textures (at 2.5x when
//! the view is zoomed out, unless `maxZoomIsOne`), then drawn each frame as
//! an atlas quad with per-texel depth.
//!
//! The item's model choice and transform (`ItemParams.init`) come from the
//! item code ([`ItemParams`] is what it fills); this module has the render
//! job (`calcMatrices`, `calcModelBounds`, `calcModelOffset`,
//! `calcEntrySize`, `renderModel`), the cache by key and the in-world draw.

// The original writes the angles as float literals.
#![allow(clippy::approx_constant)]

use std::collections::HashMap;
use std::rc::Rc;

use pz_render::iso::{self, x_to_screen, y_to_screen, TILE_SCALE};
use pz_render::model::{MeshData, ModelDraw, ModelLight, ModelPass};
use pz_render::{gl, DrawCmd, Texture};

use crate::atlas::{item_depth_draw, AtlasEntry, Atlases, Checksummer};
use crate::joml::{rotate_y_vec, Matrix4f};
use crate::models::{core_scale, encode_light};
use crate::Camera;

/// `WorldItemAtlas.ATLAS_SIZE`.
pub const ATLAS_SIZE: i32 = 512;
/// `WorldItemAtlas.MATRIX_SIZE`: the viewport the models are drawn with.
pub const MATRIX_SIZE: i32 = 1024;
/// `WorldItemModelDrawer.CARPET_DELTA_Z`.
pub const CARPET_DELTA_Z: f32 = 0.003;

/// A loaded static model (`Model` with its `ModelMesh`).
#[derive(Clone, Debug)]
pub struct ItemModel {
    /// `model.name` (the model manager key) for the entry key.
    pub name: String,
    pub mesh: Rc<MeshData>,
    /// `mesh.minXyz` / `maxXyz`.
    pub min: [f32; 3],
    pub max: [f32; 3],
    /// `model.tex` (the error texture stands in when missing).
    pub texture: Option<Texture>,
}

/// `WeaponPartParams`.
#[derive(Clone, Debug)]
pub struct WeaponPartParams {
    pub model: ItemModel,
    pub transform: Matrix4f,
}

/// `ItemParams.FoodState`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum FoodState {
    #[default]
    Normal,
    Cooked,
    Burnt,
    Rotten,
}

/// `WorldItemAtlas.ItemParams` after `init`.
#[derive(Clone, Debug)]
pub struct ItemParams {
    pub world_scale: f32,
    pub model: ItemModel,
    pub model_script_scale: f32,
    pub weapon_parts: Vec<WeaponPartParams>,
    pub hue: f32,
    pub tint: [f32; 3],
    /// In degrees: x, y (`worldZRotation`), z (`worldYRotation`).
    pub angle: [f32; 3],
    pub transform: Matrix4f,
    pub ambient: [f32; 3],
    pub food_state: FoodState,
    pub max_zoom_is_one: bool,
    pub fluid_filled: bool,
    pub blood_level: f32,
    pub fluid_level: f32,
    /// `fluidTint` red, blue, green bytes as the key writes them.
    pub fluid_tint: [u8; 3],
    /// A `TINT` mask texture exists: the tint goes into the smart texture,
    /// the shader tint is white.
    pub tint_mask: bool,
}

impl ItemParams {
    /// `ItemParams.init(item, model, modelScript, hue, tint, ...)`'s
    /// defaults for a model: no rotation, the given transform.
    pub fn new(model: ItemModel, transform: Matrix4f) -> Self {
        ItemParams {
            world_scale: 1.0,
            model,
            model_script_scale: 1.0,
            weapon_parts: Vec::new(),
            hue: 1.0,
            tint: [1.0; 3],
            angle: [0.0; 3],
            transform,
            ambient: [1.0; 3],
            food_state: FoodState::Normal,
            max_zoom_is_one: false,
            fluid_filled: false,
            blood_level: 0.0,
            fluid_level: 0.0,
            fluid_tint: [255; 3],
            tint_mask: false,
        }
    }

    /// `getItemKey`.
    pub fn key(&self) -> String {
        let mut c = Checksummer::default();
        c.str(Some(&self.model.name));
        for p in &self.weapon_parts {
            c.str(Some(&p.model.name));
        }
        c.int((self.world_scale * self.model_script_scale * 1000.0) as i32);
        c.unit(self.tint[0]);
        c.unit(self.tint[1]);
        c.unit(self.tint[2]);
        c.int((self.angle[0] * 1000.0) as i32);
        c.int((self.angle[1] * 1000.0) as i32);
        c.int((self.angle[2] * 1000.0) as i32);
        c.byte(self.food_state as u8);
        c.bool(self.max_zoom_is_one);
        c.bool(self.fluid_filled);
        c.int((self.blood_level * 1000.0) as i32);
        c.int((self.fluid_level * 100.0) as i32);
        c.byte(self.fluid_tint[0]);
        c.byte(self.fluid_tint[1]);
        c.byte(self.fluid_tint[2]);
        c.finish()
    }

    fn max_zoom(&self) -> f32 {
        if self.max_zoom_is_one {
            1.0
        } else {
            2.5
        }
    }

    /// `RenderJob.calcMatrices(projection, modelView, offsetX, offsetY)`.
    pub fn matrices(&self, offset_x: f32, offset_y: f32) -> (Matrix4f, Matrix4f) {
        let p = Matrix4f::set_ortho(
            -0.266_666_68,
            0.266_666_68,
            0.266_666_68,
            -0.266_666_68,
            -10.0,
            10.0,
        );
        let mz = self.max_zoom();
        let d = std::f32::consts::PI / 180.0;
        let mv = Matrix4f::identity()
            .scale1(core_scale() * TILE_SCALE as f32 / 2.0)
            .rotate(0.523_598_8, 1.0, 0.0, 0.0)
            .rotate(2.356_194_5, 0.0, 1.0, 0.0)
            .scale(-1.5 * mz, 1.5 * mz, 1.5 * mz)
            .rotate_xyz(self.angle[0] * d, self.angle[1] * d, self.angle[2] * d)
            .translate(offset_x, 0.0, offset_y)
            .mul(&self.transform);
        (p, mv)
    }

    /// `calcModelBounds`: the meshes' bounding boxes projected into a
    /// 512-pixel viewport, doubled (min x, min y, max x, max y).
    pub fn bounds(&self) -> [f32; 4] {
        let (p, mv) = self.matrices(0.0, 0.0);
        let mut b = [f32::MAX, f32::MAX, -f32::MAX, -f32::MAX];
        model_bounds(&p, &mv, &self.model, &mut b);
        for part in &self.weapon_parts {
            model_bounds(&p, &mv.mul(&part.transform), &part.model, &mut b);
        }
        b.map(|v| v * 2.0)
    }
}

fn model_bounds(p: &Matrix4f, mv: &Matrix4f, m: &ItemModel, b: &mut [f32; 4]) {
    let pm = p.mul(mv);
    let (mn, mx) = (m.min, m.max);
    let xs = [mn[0], mn[0], mx[0], mx[0]];
    let ys = [mn[1], mx[1], mx[1], mn[1]];
    for i in 0..4 {
        for z in [mn[2], mx[2]] {
            let s = pm.project(xs[i], ys[i], z, [0, 0, ATLAS_SIZE, ATLAS_SIZE]);
            b[0] = b[0].min(s[0]);
            b[2] = b[2].max(s[0]);
            b[1] = b[1].min(s[1]);
            b[3] = b[3].max(s[1]);
        }
    }
}

/// `calcModelOffset`: the bounds' centre relative to the 1024 viewport's.
pub fn entry_offset(b: [f32; 4]) -> (f32, f32) {
    (
        b[0] + (b[2] - b[0]) / 2.0 - 512.0,
        b[1] + (b[3] - b[1]) / 2.0 - 512.0,
    )
}

/// `calcEntrySize`: the bounds padded by 2 and snapped out to 16 pixels.
pub fn entry_size(b: [f32; 4]) -> (i32, i32) {
    let snap_lo = |v: f32| ((v - 2.0) / 16.0).floor() * 16.0;
    let snap_hi = |v: f32| ((v + 2.0) / 16.0).ceil() * 16.0;
    (
        (snap_hi(b[2]) - snap_lo(b[0])) as i32,
        (snap_hi(b[3]) - snap_lo(b[1])) as i32,
    )
}

/// The GL viewport `(x, y, 1024, 1024)` folded into the projection of a
/// draw into the whole 512x512 atlas.
fn viewport_fold(x: i32, y: i32) -> Matrix4f {
    let s = (MATRIX_SIZE / ATLAS_SIZE) as f32;
    let mut v = Matrix4f::identity();
    v.m[0][0] = s;
    v.m[1][1] = s;
    v.m[3][0] = (2 * x + MATRIX_SIZE - ATLAS_SIZE) as f32 / ATLAS_SIZE as f32;
    v.m[3][1] = (2 * y + MATRIX_SIZE - ATLAS_SIZE) as f32 / ATLAS_SIZE as f32;
    v
}

/// `RenderJob.renderModel`: one mesh into the atlas.
fn model_draw(
    params: &ItemParams,
    model: &ItemModel,
    part: Option<&Matrix4f>,
    p: &Matrix4f,
    mv: &Matrix4f,
    fold: &Matrix4f,
) -> ModelDraw {
    let inv = params.transform.invert_affine();
    let mvp = fold.mul(p).mul(&mv.mul(&inv));
    let transform = match part {
        None => params.transform,
        Some(t) => params.transform.mul(t),
    };
    let none = ModelLight {
        direction: [0.0, 1.0, 0.0],
        colour: [0.0; 3],
    };
    let mut lights = [none; 5];
    let xyz = rotate_y_vec(
        [0.0, 5.0, -2.0],
        params.angle[1] * (std::f32::consts::PI / 180.0),
    );
    let a = params.ambient;
    lights[4] = encode_light(
        [xyz[0], xyz[2], xyz[1]],
        [a[0] / 4.0 * 1.5, a[1] / 4.0 * 1.5, a[2] / 4.0 * 1.5],
        5000.0,
        [0.0; 3],
        None,
    );
    ModelDraw {
        mesh: model.mesh.clone(),
        texture: model.texture.clone(),
        mvp: mvp.m,
        transform: transform.m,
        palette: Rc::new(Vec::new()),
        target_depth: 0.5,
        final_scale: 1.0,
        uv_scale: [1.0, 1.0],
        lights,
        ambient: [a[0] * 0.4, a[1] * 0.4, a[2] * 0.4],
        tint: if params.tint_mask {
            [1.0; 3]
        } else {
            params.tint
        },
        hue: params.hue,
        alpha: 1.0,
        // glBlendFunc(SRC_ALPHA, ONE_MINUS_SRC_ALPHA), GL_LESS; culling is
        // left off by the model passes before it.
        pass: ModelPass::Character { cull: false },
    }
}

/// `WorldItemAtlas.ItemTexture` + its `AtlasEntry`.
#[derive(Clone, Debug)]
pub struct ItemTexture {
    pub max_zoom_is_one: bool,
    pub entry: Option<AtlasEntry>,
    /// `entry.ready`: the render job has drawn it.
    pub ready: bool,
    pub too_big: bool,
}

/// `WorldItemAtlas.instance`: textures by key, the atlases and the render
/// jobs waiting for `render`.
pub struct WorldItemAtlas {
    pub atlases: Atlases,
    pub textures: HashMap<String, ItemTexture>,
    jobs: Vec<(String, ItemParams)>,
}

impl WorldItemAtlas {
    /// Atlas textures get ids from `first_id` up.
    pub fn new(first_id: u64) -> Self {
        WorldItemAtlas {
            atlases: Atlases::new(ATLAS_SIZE, first_id),
            textures: HashMap::new(),
            jobs: Vec::new(),
        }
    }

    /// `getItemTexture(params)`: the cached texture, or a new one with a
    /// render job queued.
    pub fn item_texture(&mut self, params: &ItemParams) -> String {
        let key = params.key();
        if !self.textures.contains_key(&key) {
            self.textures.insert(
                key.clone(),
                ItemTexture {
                    max_zoom_is_one: params.max_zoom_is_one,
                    entry: None,
                    ready: false,
                    too_big: false,
                },
            );
            self.jobs.push((key.clone(), params.clone()));
        }
        key
    }

    /// `render()`: runs the queued jobs (`RenderJob.render`), drawing each
    /// item into its atlas entry.
    pub fn render(&mut self, out: &mut Vec<DrawCmd>) {
        for (key, params) in std::mem::take(&mut self.jobs) {
            let b = params.bounds();
            let (ox, oy) = entry_offset(b);
            let (w, h) = entry_size(b);
            let tex = self.textures.get_mut(&key).expect("job texture");
            if w <= 0 || h <= 0 {
                continue;
            }
            if w > ATLAS_SIZE || h > ATLAS_SIZE {
                tex.too_big = true;
                continue;
            }
            let (ai, mut e) = self.atlases.assign(w, h);
            e.offset_x = ox;
            e.offset_y = oy;
            let atlas = &mut self.atlases.list[ai];
            out.push(DrawCmd::AtlasBegin {
                id: e.atlas,
                width: ATLAS_SIZE as u32,
                height: ATLAS_SIZE as u32,
                clear: atlas.clear,
            });
            atlas.clear = false;
            // clearColorAndDepth: the entry's cell only.
            let [sx, sy, sw, sh] = e.scissor();
            out.push(DrawCmd::Clip(Some([sx, sy, sw, sh])));
            out.push(DrawCmd::ColorMask(true, true, true, true));
            out.push(DrawCmd::DepthMask(true));
            out.push(DrawCmd::Clear(gl::COLOR_BUFFER_BIT | gl::DEPTH_BUFFER_BIT));
            out.push(DrawCmd::Clip(None));
            let vx = e.x - ox as i32 - (MATRIX_SIZE - e.w) / 2;
            let vy = -(oy as i32) - (MATRIX_SIZE - e.h) / 2 + ATLAS_SIZE - (e.y + e.h);
            let fold = viewport_fold(vx, vy);
            let (p, mv) = params.matrices(0.0, 0.0);
            out.push(DrawCmd::Model(Box::new(model_draw(
                &params,
                &params.model,
                None,
                &p,
                &mv,
                &fold,
            ))));
            for part in &params.weapon_parts {
                out.push(DrawCmd::Model(Box::new(model_draw(
                    &params,
                    &part.model,
                    Some(&part.transform),
                    &p,
                    &mv,
                    &fold,
                ))));
            }
            out.push(DrawCmd::AtlasEnd);
            tex.entry = Some(e);
            tex.ready = true;
        }
    }
}

/// Where and how a placed item draws this frame
/// (`renderAtlasTexture`'s inputs).
#[derive(Clone, Copy, Debug)]
pub struct ItemPlacement {
    /// World position with the item's offsets (`getX() + xoff`, ...).
    pub x: f32,
    pub y: f32,
    pub z: f32,
    /// The square's `FootstepMaterial` is `Carpet`.
    pub carpet: bool,
    /// The square's light at the item (`interpolateLight(x % 1, y % 1)`),
    /// or the highlight colour.
    pub rgb: [f32; 3],
    /// `getSurfaceAlpha(square, z - (int)z)`.
    pub alpha: f32,
}

/// `renderAtlasTexture`'s draw of a ready entry: screen position from the
/// camera, then `ItemTextureDepthDrawer`.
pub fn item_world_draw(tex: &ItemTexture, cam: &Camera, at: &ItemPlacement) -> Vec<DrawCmd> {
    let Some(e) = tex.entry.filter(|_| tex.ready) else {
        return Vec::new();
    };
    if at.alpha <= 0.0 {
        return Vec::new();
    }
    let z = at.z + if at.carpet { CARPET_DELTA_Z } else { 0.0 };
    let sx = x_to_screen(at.x, at.y) - cam.off_x as f32 + cam.fix_jiggly_x * cam.zoom;
    let sy = y_to_screen(at.x, at.y, z, 0) - cam.off_y as f32 + cam.fix_jiggly_y * cam.zoom;
    item_depth_draw(
        &e,
        tex.max_zoom_is_one,
        cam,
        [at.x, at.y, z],
        sx,
        sy,
        [at.rgb[0], at.rgb[1], at.rgb[2], at.alpha],
    )
}

/// `bMaxZoomIsOne`: `!Core.getOptionHighResPlacedItems() || zoom >= 0.75`.
pub fn max_zoom_is_one(high_res_placed_items: bool, zoom: f32) -> bool {
    !high_res_placed_items || zoom >= 0.75
}

/// `IsoWorldInventoryObject.render`'s 2D fallback for an item without a
/// model: the item's world sprite (`updateSprite`: its icon or world
/// texture scaled by `ScaleWorldIcon`), lifted by three quarters of its
/// height and centred, drawn in the translucent pass with the `default`
/// shader at the item's depth.
#[allow(clippy::too_many_arguments)]
pub fn item_sprite_draw(
    cam: &Camera,
    tex: &Texture,
    scale: f32,
    x: f32,
    y: f32,
    z: f32,
    rgb: [f32; 3],
    tint_mod: [f32; 3],
    alpha: f32,
) -> Vec<DrawCmd> {
    use pz_render::iso::{chunk_depth_data, fastfloor};
    let dx = tex.width_orig() as f32 * scale / 2.0;
    let dy = tex.height_orig() as f32 * scale * 3.0 / 4.0;
    // IsoSprite.render: prepareToRenderSprite outside a chunk texture.
    let mut sx = x_to_screen(x, y) - dx - cam.off_x as f32 + cam.fix_jiggly_x * cam.zoom;
    let mut sy = y_to_screen(x, y, z, 0) - dy - cam.off_y as f32 + cam.fix_jiggly_y * cam.zoom;
    let (mut w, mut h) = (tex.width as f32, tex.height as f32);
    if scale != 1.0 {
        sx += tex.offset_x * (scale - 1.0);
        w *= scale;
        sy += tex.offset_y * (scale - 1.0);
        h *= scale;
    }
    let c = [
        rgb[0] * tint_mod[0],
        rgb[1] * tint_mod[1],
        rgb[2] * tint_mod[2],
    ];
    let Some(draw) = crate::texd::TexDraw::new(
        tex,
        sx + tex.offset_x,
        sy + tex.offset_y,
        w,
        h,
        pz_render::Rgba(c[0], c[1], c[2], alpha),
    ) else {
        return Vec::new();
    };
    let sz = iso::calculate_depth(x, y, z);
    let mut chunk_depth = chunk_depth_data(
        cam.chunk_x(),
        cam.chunk_y(),
        fastfloor(x / 8.0),
        fastfloor(y / 8.0),
        fastfloor(z),
    )
    .depth_start;
    chunk_depth += 0.5;
    chunk_depth = chunk_depth * 2.0 - 1.0;
    vec![
        DrawCmd::GlEnable(gl::DEPTH_TEST),
        DrawCmd::DepthFunc(gl::LEQUAL),
        DrawCmd::DepthMask(false),
        draw.into_cmd(pz_render::WorldShader::Default {
            z: chunk_depth + (sz * 2.0 - 1.0),
        }),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use pz_render::model::MeshKind;

    fn cube() -> ItemModel {
        ItemModel {
            name: "cube".into(),
            mesh: Rc::new(MeshData {
                id: 1,
                kind: MeshKind::Static,
                vertices: vec![],
                indices: vec![],
            }),
            min: [-0.1, -0.05, -0.1],
            max: [0.1, 0.05, 0.1],
            texture: None,
        }
    }

    #[test]
    fn entry_size_snaps_to_16() {
        assert_eq!(entry_size([100.5, 200.0, 140.0, 230.0]), (48, 48));
        assert_eq!(entry_size([100.5, 200.0, 143.0, 230.0]), (64, 48));
        assert_eq!(entry_offset([100.0, 200.0, 140.0, 230.0]), (-392.0, -297.0));
    }

    #[test]
    fn small_item_gets_a_cell_and_draws() {
        let mut a = WorldItemAtlas::new(1000);
        let mut p = ItemParams::new(cube(), Matrix4f::identity());
        p.max_zoom_is_one = true;
        let key = a.item_texture(&p);
        assert_eq!(key, a.item_texture(&p));
        let mut out = Vec::new();
        a.render(&mut out);
        assert!(matches!(
            out[0],
            DrawCmd::AtlasBegin {
                id: 1000,
                clear: true,
                ..
            }
        ));
        assert_eq!(
            out.iter()
                .filter(|c| matches!(c, DrawCmd::Model(_)))
                .count(),
            1
        );
        let t = &a.textures[&key];
        let e = t.entry.unwrap();
        // A 0.2-unit box is a few dozen pixels across at 1x.
        assert!(e.w > 16 && e.w < 128 && e.h > 16 && e.h < 128, "{e:?}");
        // The model's centre lands in the centre of its cell: the bounds'
        // centre projected through the folded viewport.
        let DrawCmd::Model(d) = &out[6] else { panic!() };
        let mvp = Matrix4f::from_cols(d.mvp);
        let c = mvp.transform([0.0, 0.0, 0.0, 1.0]);
        let (px, py) = ((c[0] / c[3] + 1.0) * 256.0, (c[1] / c[3] + 1.0) * 256.0);
        let cell_cx = e.x as f32 + e.w as f32 / 2.0;
        let cell_cy = (ATLAS_SIZE - e.y) as f32 - e.h as f32 / 2.0;
        assert!(
            (px - cell_cx).abs() < 2.0 && (py - cell_cy).abs() < 2.0,
            "{px} {py} vs {cell_cx} {cell_cy}"
        );
        let cam = Camera::player(10.5, 10.5, 0.0, 640, 480, 1.0);
        let draws = item_world_draw(
            t,
            &cam,
            &ItemPlacement {
                x: 11.0,
                y: 10.5,
                z: 0.0,
                carpet: false,
                rgb: [1.0; 3],
                alpha: 1.0,
            },
        );
        assert!(draws
            .iter()
            .any(|c| matches!(c, DrawCmd::AtlasQuad { id: 1000, .. })));
    }
}
