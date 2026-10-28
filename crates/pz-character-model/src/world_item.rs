//! Placed items (`IsoWorldInventoryObject.render`): the model choice and
//! transform `WorldItemAtlas.ItemParams.init` makes from an item, for
//! [`pz_iso_render::item_atlas`]; and the 2D sprite drawn when the item
//! has no model.
//!
//! Ported: the `getWorldStaticItem()` path (with the `_Fluid` model of a
//! container more than half full, the `Cooked`/`Burnt`/`Rotten` food
//! variants and the rotten tint), the `HandWeapon` path (weapon fix
//! rotation, blood level) and the texture parameters `checkSmartTexture`
//! reads (tint mask, blood, fluid level, colour and mask; see
//! [`atlas_smart_texture`]). Not yet: weapon-part models and the
//! clothing-item path for head clothing without a world model.

use pz_entity::{EntityContext, FluidContainer};
use pz_inventory::{InventoryItem, ItemKind, ModDataResult};
use pz_iso_render::item_atlas::{FoodState, ItemModel, ItemParams};
use pz_iso_render::joml::Matrix4f;
use pz_scripts::objects::models::ModelAttachment;

use pz_render::combiner::Combiner;
use pz_render::{Texture, TextureManager};

use crate::combine::run_jobs;
use crate::item_texture::{atlas_job, item_texture_name, mask_texture_name, ItemTextureParams};
use crate::model_tree::{ModelScripts, TextureSource};
use crate::render::ModelResources;

/// `ModelInstanceRenderData.makeAttachmentTransform` in JOML.
pub fn attachment_matrix(a: &ModelAttachment) -> Matrix4f {
    let d = std::f32::consts::PI / 180.0;
    let (o, r) = (a.offset(), a.rotate());
    Matrix4f::translation(o.x, o.y, o.z)
        .rotate_xyz(r.x * d, r.y * d, r.z * d)
        .scale1(a.scale())
}

/// `WorldItemAtlas.ROTTEN_FOOD_COLOR`.
const ROTTEN: [f32; 3] = [0.5, 0.5, 0.5];

/// `ItemParams.init(item, bHighRes)` for an item without a fluid
/// container: `None` when the item has no model the atlas draws (or it is
/// not loaded).
pub fn try_item_params(
    item: &InventoryItem,
    scripts: &dyn ModelScripts,
    res: &dyn ModelResources,
    max_zoom_is_one: bool,
) -> ModDataResult<Option<ItemParams>> {
    try_item_params_with_fluid(item, None, scripts, res, max_zoom_is_one)
}

pub fn item_params(
    item: &InventoryItem,
    scripts: &dyn ModelScripts,
    res: &dyn ModelResources,
    max_zoom_is_one: bool,
) -> Option<ItemParams> {
    item_params_with_fluid(item, None, scripts, res, max_zoom_is_one)
}

/// `initFluidContainer`: filled (more than half), the filled ratio and
/// the colour.
struct FluidState {
    filled: bool,
    level: f32,
    color: [f32; 3],
}

/// `ItemParams.init(item, bHighRes)` with the item's `FluidContainer`
/// (`getFluidContainer()`, else its world item's) and what its fluid
/// colours resolve against.
pub fn try_item_params_with_fluid(
    item: &InventoryItem,
    fluid: Option<(&FluidContainer, &dyn EntityContext)>,
    scripts: &dyn ModelScripts,
    res: &dyn ModelResources,
    max_zoom_is_one: bool,
) -> ModDataResult<Option<ItemParams>> {
    let blank = |s: &Option<String>| s.as_deref().is_none_or(|s| s.trim().is_empty());
    let world_static = item.try_get_world_static_model()?;
    Ok((|| {
        let fluid = fluid.map(|(f, ctx)| {
            let level = f.get_filled_ratio();
            FluidState {
                filled: level > 0.5,
                level,
                color: f.get_color(ctx),
            }
        });
        let fluid_model = |fluid: &Option<FluidState>| {
            let base = world_static.clone().unwrap_or_else(|| "null".into());
            fluid
                .as_ref()
                .filter(|f| f.filled)
                .and_then(|_| scripts.model_script(&format!("{base}_Fluid")))
        };
        let exists = |name: &str| {
            res.texture(&TextureSource::File(name.to_string()))
                .is_some()
        };
        let mut p = if !blank(&world_static) {
            let base = world_static.clone().unwrap();
            let mut ms = scripts.model_script(&base)?;
            if let Some(m) = fluid_model(&fluid) {
                ms = m;
            }
            let mut food_state = FoodState::Normal;
            let mut tint = [item.color_red, item.color_green, item.color_blue];
            if let ItemKind::Food(f) = &item.kind {
                food_state = if item.is_rotten() {
                    FoodState::Rotten
                } else if f.burnt {
                    FoodState::Burnt
                } else if f.cooked {
                    FoodState::Cooked
                } else {
                    FoodState::Normal
                };
                if f.cooked {
                    if let Some(m) = scripts.model_script(&format!("{base}Cooked")) {
                        ms = m;
                    }
                }
                if f.burnt {
                    if let Some(m) = scripts.model_script(&format!("{base}Burnt")) {
                        ms = m;
                    }
                }
                if item.is_rotten() {
                    match scripts.model_script(&format!("{base}Rotten")) {
                        Some(m) => ms = m,
                        None => tint = ROTTEN,
                    }
                }
            }
            let mut p = params_for(&ms, item, res, false)?;
            p.food_state = food_state;
            p.tint = tint;
            if let Some(tex) = ms.borrow().texture_name() {
                p.model_texture_name = Some(item_texture_name(tex));
                p.tint_mask = Some(mask_texture_name(tex, "TINT")).filter(|m| exists(m));
                if fluid.as_ref().is_some_and(|f| f.level > 0.0) && p.model.texture.is_some() {
                    p.fluid_texture_mask =
                        Some(mask_texture_name(tex, "FLUIDTINT")).filter(|m| exists(m));
                }
            }
            p
        } else if let ItemKind::Weapon(w) = &item.kind {
            let name = w.weapon_sprite.clone().filter(|s| !s.trim().is_empty())?;
            let mut ms = scripts.model_script(&name)?;
            let model_texture = ms.borrow().texture_name().map(item_texture_name);
            if let Some(m) = fluid_model(&fluid) {
                ms = m;
            }
            let mut p = params_for(&ms, item, res, true)?;
            p.tint = [item.color_red, item.color_green, item.color_blue];
            p.model_texture_name = model_texture;
            p.blood_level = w.blood_level;
            p
        } else {
            return None;
        };
        if let Some(f) = &fluid {
            // Color.getRedByte etc.: (int)(c * 255), written red, blue, green.
            let byte = |c: f32| (c * 255.0) as i32 as u8;
            p.fluid_filled = f.filled;
            p.fluid_level = f.level;
            p.fluid_color = f.color;
            p.fluid_tint = [byte(f.color[0]), byte(f.color[2]), byte(f.color[1])];
        }
        p.max_zoom_is_one = max_zoom_is_one;
        Some(p)
    })())
}

pub fn item_params_with_fluid(
    item: &InventoryItem,
    fluid: Option<(&FluidContainer, &dyn EntityContext)>,
    scripts: &dyn ModelScripts,
    res: &dyn ModelResources,
    max_zoom_is_one: bool,
) -> Option<ItemParams> {
    try_item_params_with_fluid(item, fluid, scripts, res, max_zoom_is_one)
        .expect("unborrowed world item ModData")
}

/// What `RenderJob.checkSmartTexture` reads off the params.
pub fn atlas_texture_params(p: &ItemParams) -> ItemTextureParams {
    ItemTextureParams {
        model_texture: p.model_texture_name.clone(),
        tint: p.tint,
        tint_mask: p.tint_mask.clone(),
        blood_level: p.blood_level,
        fluid_level: p.fluid_level,
        fluid_mask: p.fluid_texture_mask.clone(),
        fluid_tint: p.fluid_color,
    }
}

/// `checkSmartTexture` + `calculate` for an atlas render job: the item
/// model's combined texture, or `None` for its own texture. `loaded` is
/// `Texture.getTexture` (the fluid mask is loaded); `file` loads texture
/// files for the combiner. Hand the result to the combiner's `release`
/// once the frame that draws the atlas is drawn.
pub fn atlas_smart_texture(
    p: &ItemParams,
    loaded: &dyn Fn(&str) -> bool,
    file: &mut dyn FnMut(&mut TextureManager, &str) -> Option<Texture>,
    combiner: &mut dyn Combiner,
    tm: &mut TextureManager,
) -> Option<Texture> {
    let job = atlas_job(&atlas_texture_params(p), loaded)?;
    run_jobs(std::slice::from_ref(&job), file, combiner, tm)
        .pop()
        .flatten()
}

/// The script path of `init`: the model, the `world` attachment's inverse,
/// the mesh transform, the scale and the item's rotation.
fn params_for(
    ms: &crate::data::ModelScriptRef,
    item: &InventoryItem,
    res: &dyn ModelResources,
    weapon_fix: bool,
) -> Option<ItemParams> {
    let s = ms.borrow();
    let mesh_name = s.mesh_name()?.to_string();
    let tex_name = s.texture_name().map(str::to_string);
    if !s.is_static() {
        return None;
    }
    let mesh = res.mesh(&mesh_name, true)?;
    let texture = tex_name
        .as_ref()
        .and_then(|t| res.texture(&TextureSource::File(t.clone())));
    let mut t = Matrix4f::identity();
    if weapon_fix {
        t = t.rotate_xyz(0.0, std::f32::consts::PI, 1.570_796_4);
    }
    if let Some(a) = s.attachment_by_id("world") {
        t = t.mul(&attachment_matrix(&a.borrow()).invert_affine());
    }
    if let Some(mt) = &mesh.transform {
        t = t.mul(&Matrix4f::from_cols(*mt));
    }
    if item.world_scale != 1.0 {
        t = t.scale1(s.scale * item.world_scale);
    } else if s.scale != 1.0 {
        t = t.scale1(s.scale);
    }
    let model = ItemModel {
        name: format!("{}_{}", mesh_name, tex_name.as_deref().unwrap_or("")),
        mesh: mesh.mesh.clone(),
        min: mesh.min_xyz,
        max: mesh.max_xyz,
        texture,
    };
    let mut p = ItemParams::new(model, t);
    p.world_scale = item.world_scale;
    p.model_script_scale = s.scale;
    p.angle = [
        item.world_x_rotation,
        item.world_z_rotation,
        item.world_y_rotation,
    ];
    Some(p)
}
