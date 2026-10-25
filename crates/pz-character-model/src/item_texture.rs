//! Item textures: `ItemSmartTexture` (an item model's texture with its
//! tint mask, weapon blood and fluid overlays) and the three places that
//! build one:
//!
//! - held and attached items: `ModelInstanceTextureInitializer` →
//!   `EquippedTextureCreator.updateTexture`, rendered once and again only
//!   when the initializer is set dirty ([`EquippedTextureState`],
//!   [`render_equipped_textures`]);
//! - placed items in the `WorldItemAtlas`: `RenderJob.checkSmartTexture`,
//!   for each atlas render ([`atlas_job`]); the atlas key carries the blood
//!   and fluid values, so a change renders a new entry;
//! - placed items drawn directly (`ItemModelRenderer.checkSmartTexture`,
//!   [`item_renderer_job`]).
//!
//! The jobs run through the texture combiner like the clothing jobs
//! ([`crate::combine::run_jobs`]).

use pz_render::combiner::Combiner;
use pz_render::{Texture, TextureManager};

use crate::data::{Rgb, WHITE};
use crate::items::ItemTextureInit;
use crate::model_tree::{ModelSlot, TextureSource};
use crate::texture_creator::{SmartTexture, SmartTextureJob, TexRef};

/// `media/textures/FullAlpha.png`, the tint and fluid masks' mask.
pub const FULL_ALPHA: &str = "media/textures/FullAlpha.png";
/// The weapon blood overlay and its mask.
pub const BLOOD_OVERLAY_WEAPON: &str = "media/textures/BloodTextures/BloodOverlayWeapon.png";
pub const BLOOD_OVERLAY_WEAPON_MASK: &str =
    "media/textures/BloodTextures/BloodOverlayWeaponMask.png";

/// `initTextureName(textureName)` (`ItemModelRenderer`, `ItemParams`).
pub fn item_texture_name(name: &str) -> String {
    if name.contains("media/") || name.contains("media\\") {
        name.to_string()
    } else {
        format!("media/textures/{name}.png")
    }
}

/// `initTextureName(textureName, suffix)`: the `TINT`/`FLUIDTINT` mask
/// beside a texture.
pub fn mask_texture_name(name: &str, suffix: &str) -> String {
    crate::texture_creator::equipped_texture_name(name, suffix)
}

/// `ItemSmartTexture`.
#[derive(Clone, Debug, Default)]
pub struct ItemSmartTexture {
    st: SmartTexture,
    /// `texName`.
    pub tex_name: Option<String>,
    /// `SmartTexture.dirty`.
    pub dirty: bool,
}

fn intensity_param(st: &mut SmartTexture, category: i32, intensity: f32) -> Option<bool> {
    let com = st.first_from_category(category)?;
    let mut changed = false;
    for p in &mut com.params {
        if p.name != "intensity" || p.min == intensity && p.max == intensity {
            continue;
        }
        p.min = intensity;
        p.max = intensity;
        changed = true;
    }
    Some(changed)
}

impl ItemSmartTexture {
    /// `new ItemSmartTexture(tex)`.
    pub fn new(tex: Option<&str>) -> Self {
        let mut s = ItemSmartTexture::default();
        if let Some(t) = tex {
            s.add(t);
            s.tex_name = Some(t.to_string());
        }
        s
    }

    /// `clear()`.
    pub fn clear(&mut self) {
        self.st = SmartTexture::default();
        self.dirty = false;
    }

    /// `add(tex)`.
    pub fn add(&mut self, tex: &str) {
        self.st.add(TexRef::File(tex.to_string()));
        self.dirty = true;
    }

    /// `addTint(tex, category, r, g, b)`.
    pub fn add_tint(&mut self, tex: &str, category: i32, c: Rgb) {
        self.st.add_tint(TexRef::File(tex.to_string()), category, c);
        self.dirty = true;
    }

    /// `setTintMask(tex, mask, category, tint)`.
    pub fn set_tint_mask(&mut self, tex: &str, mask: &str, category: i32, tint: Rgb) {
        self.st.add_tinted_overlay(tex, mask, 1.0, category, tint);
        self.dirty = true;
    }

    /// `setBlood(tex, mask, intensity, category)`: the category's first
    /// command takes the intensity, or a new blood overlay is added.
    pub fn set_blood(&mut self, tex: &str, mask: &str, intensity: f32, category: i32) {
        let intensity = 0.0f32.max(1.0f32.min(intensity));
        match intensity_param(&mut self.st, category, intensity) {
            Some(changed) => self.dirty |= changed,
            None if intensity > 0.0 => {
                self.st.add_overlay(tex, mask, intensity, category);
                self.dirty = true;
            }
            None => {}
        }
    }

    /// `setFluid(tex, mask, intensity, category, tint)`: as `setBlood`,
    /// adding a tinted overlay.
    pub fn set_fluid(&mut self, tex: &str, mask: &str, intensity: f32, category: i32, tint: Rgb) {
        let intensity = intensity.clamp(0.0, 1.0);
        match intensity_param(&mut self.st, category, intensity) {
            Some(changed) => self.dirty |= changed,
            None if intensity > 0.0 => {
                self.st
                    .add_tinted_overlay(tex, mask, intensity, category, tint);
                self.dirty = true;
            }
            None => {}
        }
    }

    /// The command list `calculate()` combines.
    pub fn job(&self) -> SmartTextureJob {
        SmartTextureJob {
            commands: self.st.commands.clone(),
        }
    }
}

/// `EquippedTextureCreator.init(modelInstance, item)` + `updateTexture`:
/// the job for an instance whose texture is `texture` (the file name,
/// `media/textures/...png`). `smart` says the instance already shows an
/// `ItemSmartTexture` (an earlier render made one); a white, bloodless
/// item keeps its plain texture otherwise, fluid included. `exists` stands
/// for the texture manager's `getSharedTexture` lookups.
pub fn equipped_job(
    texture: &str,
    init: &ItemTextureInit,
    smart: bool,
    exists: &dyn Fn(&str) -> bool,
) -> Option<SmartTextureJob> {
    let tint_mask = Some(mask_texture_name(texture, "TINT")).filter(|m| exists(m));
    let fluid_mask = Some(mask_texture_name(texture, "FLUIDTINT"))
        .filter(|m| init.fluid_level > 0.0 && exists(m));
    if init.tint == WHITE && init.blood_level <= 0.0 && !smart {
        return None;
    }
    let mut st = ItemSmartTexture::new(Some(texture));
    st.clear();
    st.add(texture);
    if init.tint != WHITE {
        match &tint_mask {
            Some(m) => st.set_tint_mask(m, FULL_ALPHA, 300, init.tint),
            None => st.add_tint(texture, 300, init.tint),
        }
    }
    if init.blood_level > 0.0 {
        st.set_blood(
            BLOOD_OVERLAY_WEAPON,
            BLOOD_OVERLAY_WEAPON_MASK,
            init.blood_level,
            301,
        );
    }
    if let Some(m) = &fluid_mask {
        st.set_fluid(
            m,
            FULL_ALPHA,
            init.fluid_level,
            302,
            init.fluid_tint.unwrap_or(WHITE),
        );
    }
    Some(st.job())
}

/// What `checkSmartTexture` reads off the item's render parameters.
#[derive(Clone, Debug, PartialEq)]
pub struct ItemTextureParams {
    /// `modelTextureName` (`media/textures/...png`).
    pub model_texture: Option<String>,
    /// The item tint (`tintR/G/B`).
    pub tint: Rgb,
    /// `tintMask`: the `TINT` mask, when it exists.
    pub tint_mask: Option<String>,
    /// `HandWeapon.getBloodLevel()` (0 for other items).
    pub blood_level: f32,
    /// `FluidContainer.getFilledRatio()`.
    pub fluid_level: f32,
    /// `fluidTextureMask`: the `FLUIDTINT` mask, when it exists and the
    /// container is not empty.
    pub fluid_mask: Option<String>,
    /// `FluidContainer.getColor()`.
    pub fluid_tint: Rgb,
}

fn check_smart_texture(
    p: &ItemTextureParams,
    blood_category: i32,
    fluid_exists: &dyn Fn(&str) -> bool,
) -> Option<SmartTextureJob> {
    let texture = p.model_texture.as_deref()?;
    let fluid = p.fluid_mask.as_deref().filter(|_| p.fluid_level > 0.0);
    if !(p.blood_level > 0.0 || fluid.is_some() || p.tint_mask.is_some()) {
        return None;
    }
    let mut st = ItemSmartTexture::new(None);
    st.clear();
    st.add(texture);
    if let Some(m) = &p.tint_mask {
        st.set_tint_mask(m, FULL_ALPHA, 300, p.tint);
    }
    if p.blood_level > 0.0 {
        st.set_blood(
            BLOOD_OVERLAY_WEAPON,
            BLOOD_OVERLAY_WEAPON_MASK,
            p.blood_level,
            blood_category,
        );
    }
    if let Some(m) = fluid.filter(|m| fluid_exists(m)) {
        st.set_fluid(m, FULL_ALPHA, p.fluid_level, 302, p.fluid_tint);
    }
    Some(st.job())
}

/// `WorldItemAtlas.RenderJob.checkSmartTexture`: `None` when the model's
/// own texture is drawn. The blood goes in category 300 like the tint
/// mask, so with a tint mask the blood level becomes the mask's
/// intensity instead of a blood overlay (as in the original); the fluid
/// overlay needs the mask loaded (`Texture.getTexture`).
pub fn atlas_job(p: &ItemTextureParams, loaded: &dyn Fn(&str) -> bool) -> Option<SmartTextureJob> {
    check_smart_texture(p, 300, loaded)
}

/// `ItemModelRenderer.checkSmartTexture` (items drawn without the atlas):
/// blood in category 301.
pub fn item_renderer_job(p: &ItemTextureParams) -> Option<SmartTextureJob> {
    check_smart_texture(p, 301, &|_| true)
}

/// `ModelInstanceTextureInitializer` for one instance: whether its
/// texture is up to date, whether it shows an `ItemSmartTexture`, and the
/// combined texture. The original keeps a copy per render state; drawn on
/// one thread, the change numbers reduce to the dirty flag.
#[derive(Clone, Debug)]
pub struct EquippedTextureState {
    /// `!rendered`: `init`/`setDirty` since the last render.
    pub dirty: bool,
    /// `modelInstance.tex instanceof ItemSmartTexture`.
    pub smart: bool,
    /// The texture the last render combined.
    pub texture: Option<Texture>,
}

impl Default for EquippedTextureState {
    /// `init` sets the initializer dirty.
    fn default() -> Self {
        EquippedTextureState {
            dirty: true,
            smart: false,
            texture: None,
        }
    }
}

/// `ModelInstanceTextureInitializer.renderMain` + `render` for every
/// instance of `slot` with an initializer that is dirty: builds the
/// equipped job from the instance's texture and current item values and
/// runs it (`run`, e.g. [`crate::combine::run_jobs`]); the instance then
/// shows the result. The texture a re-render replaces goes to `release`.
pub fn render_equipped_textures(
    slot: &mut ModelSlot,
    exists: &dyn Fn(&str) -> bool,
    run: &mut dyn FnMut(&SmartTextureJob) -> Option<Texture>,
    release: &mut dyn FnMut(Texture),
) {
    for inst in &mut slot.instances {
        let Some(init) = &inst.texture_init else {
            continue;
        };
        if !inst.equipped_texture.dirty {
            continue;
        }
        inst.equipped_texture.dirty = false;
        let Some(name) = inst.model_texture.as_deref().map(item_texture_name) else {
            continue;
        };
        let Some(job) = equipped_job(&name, init, inst.equipped_texture.smart, exists) else {
            continue;
        };
        let Some(tex) = run(&job) else { continue };
        if let Some(old) = inst.equipped_texture.texture.replace(tex.clone()) {
            release(old);
        }
        inst.equipped_texture.smart = true;
        inst.tex = TextureSource::Combined(tex);
    }
}

/// [`render_equipped_textures`] through a texture combiner, with files
/// loaded by `file` (`Texture.getSharedTexture(name,
/// ModelManager.getTextureFlags())`).
pub fn render_equipped_textures_with(
    slot: &mut ModelSlot,
    exists: &dyn Fn(&str) -> bool,
    file: &mut dyn FnMut(&mut TextureManager, &str) -> Option<Texture>,
    combiner: &mut dyn Combiner,
    tm: &mut TextureManager,
) {
    let mut released = Vec::new();
    {
        let mut run = |job: &SmartTextureJob| {
            crate::combine::run_jobs(std::slice::from_ref(job), file, &mut *combiner, tm)
                .pop()
                .flatten()
        };
        render_equipped_textures(slot, exists, &mut run, &mut |t| released.push(t));
    }
    for t in released {
        combiner.release(t);
    }
}

impl crate::model_tree::ModelInstance {
    /// `setDirty` with the item's current values
    /// (`IsoGameCharacter.updateEquippedTextures` → the next `renderMain`
    /// re-reads the item): the texture is rebuilt on the next render.
    pub fn refresh_texture_init(&mut self, init: ItemTextureInit) {
        self.texture_init = Some(init);
        self.equipped_texture.dirty = true;
    }
}
