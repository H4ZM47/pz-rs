//! Placed items (`IsoWorldInventoryObject.render`): the model choice and
//! transform `WorldItemAtlas.ItemParams.init` makes from an item, for
//! [`pz_iso_render::item_atlas`]; and the 2D sprite drawn when the item
//! has no model.
//!
//! Ported: the `getWorldStaticItem()` path (with the `Cooked`/`Burnt`/
//! `Rotten` food variants and the rotten tint) and the `HandWeapon` path
//! (weapon fix rotation). Not yet: weapon-part models, the clothing-item
//! path for head clothing without a world model, fluid-container
//! variants and the smart textures (blood, fluid, tint masks).

use pz_inventory::{InventoryItem, ItemKind};
use pz_iso_render::item_atlas::{FoodState, ItemModel, ItemParams};
use pz_iso_render::joml::Matrix4f;
use pz_scripts::objects::models::ModelAttachment;

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

/// `ItemParams.init(item, bHighRes)`: `None` when the item has no model
/// the atlas draws (or it is not loaded).
pub fn item_params(
    item: &InventoryItem,
    scripts: &dyn ModelScripts,
    res: &dyn ModelResources,
    max_zoom_is_one: bool,
) -> Option<ItemParams> {
    let blank = |s: &Option<String>| s.as_deref().is_none_or(|s| s.trim().is_empty());
    let world_static = item.get_world_static_model();
    let mut p = if !blank(&world_static) {
        let base = world_static.unwrap();
        let mut ms = scripts.model_script(&base)?;
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
        p
    } else if let ItemKind::Weapon(w) = &item.kind {
        let name = w.weapon_sprite.clone().filter(|s| !s.trim().is_empty())?;
        let ms = scripts.model_script(&name)?;
        let mut p = params_for(&ms, item, res, true)?;
        p.tint = [item.color_red, item.color_green, item.color_blue];
        p
    } else {
        return None;
    };
    p.max_zoom_is_one = max_zoom_is_one;
    Some(p)
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
