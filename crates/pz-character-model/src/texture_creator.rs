//! `ModelInstanceTextureCreator`: the composited body and clothing
//! textures (skin + texture-only clothing + blood/dirt/holes/patches/decals).
//!
//! The original renders these on the GPU: each `SmartTexture` is a list of
//! `TextureCombinerCommand`s that `TextureCombiner.combine` draws into an
//! FBO with the `hueChange`/`overlayMask`/`dirtMask`/`bodyMask*`/
//! `addHole`/`removeHole`/`blit` shaders. This port produces the same
//! command lists as [`SmartTextureJob`]s for the GPU backend to run in
//! order; a job's output is referenced by later jobs as
//! [`TexRef::Job`]. Each job's result size is the largest input texture's
//! size rounded up to a power of two (`TextureCombiner.getResultingWidth`).

use std::collections::HashMap;
use std::rc::Rc;

use crate::data::{
    BodyLocationGroup, ClothingDecal, ClothingItem, ClothingItemExt, HumanModel,
    ResolvedItemVisual, Rgb, DEFAULT_MASKS_FOLDER, WHITE,
};
use crate::mask::{patch_mask_file, BloodBodyPartType, CharacterMask, BLOOD_PARTS, MASK_FILES};
use crate::model_tree::{ModelSlot, TextureSource};

/// GL blend factors.
pub const GL_ZERO: u32 = 0;
pub const GL_ONE: u32 = 1;
pub const GL_SRC_ALPHA: u32 = 770;
pub const GL_ONE_MINUS_SRC_ALPHA: u32 = 771;
pub const GL_DST_ALPHA: u32 = 772;
pub const GL_DST_COLOR: u32 = 774;

/// A texture a combiner command reads.
#[derive(Clone, Debug, PartialEq)]
pub enum TexRef {
    /// A file under the game directory (`media/textures/...png`).
    File(String),
    /// The result of an earlier job.
    Job(usize),
}

/// `SmartShader` programs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SmartShader {
    Blit,
    /// `hueChange` (used for both tint and hue).
    HueChange,
    OverlayMask,
    DirtMask,
    BodyMask,
    BodyMaskHue,
    BodyMaskTint,
    AddHole,
    RemoveHole,
}

impl SmartShader {
    pub fn program(self) -> &'static str {
        match self {
            SmartShader::Blit => "blit",
            SmartShader::HueChange => "hueChange",
            SmartShader::OverlayMask => "overlayMask",
            SmartShader::DirtMask => "dirtMask",
            SmartShader::BodyMask => "bodyMask",
            SmartShader::BodyMaskHue => "bodyMaskHue",
            SmartShader::BodyMaskTint => "bodyMaskTint",
            SmartShader::AddHole => "addHole",
            SmartShader::RemoveHole => "removeHole",
        }
    }
}

/// `TextureCombinerShaderParam` (a float, or a min/max pair).
#[derive(Clone, Debug, PartialEq)]
pub struct ShaderParam {
    pub name: String,
    pub min: f32,
    pub max: f32,
}

impl ShaderParam {
    fn f(name: &str, v: f32) -> Self {
        ShaderParam {
            name: name.into(),
            min: v,
            max: v,
        }
    }
}

/// `TextureCombinerCommand`.
#[derive(Clone, Debug, PartialEq)]
pub struct CombinerCommand {
    pub tex: TexRef,
    pub mask: Option<TexRef>,
    pub shader: SmartShader,
    pub params: Vec<ShaderParam>,
    /// `blendSrc, blendDest, blendSrcA, blendDestA`.
    pub blend: [u32; 4],
    /// `x, y, w, h` for `addRect` (decals).
    pub rect: Option<[i32; 4]>,
}

/// One `SmartTexture.calculate()`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SmartTextureJob {
    pub commands: Vec<CombinerCommand>,
}

/// `SmartTexture` while it is being built.
#[derive(Clone, Debug, Default)]
pub(crate) struct SmartTexture {
    pub(crate) commands: Vec<CombinerCommand>,
    category: HashMap<i32, Vec<usize>>,
}

fn tex(path: &str) -> TexRef {
    TexRef::File(path.to_string())
}

fn mask_file(dir: &str, file: &str) -> String {
    format!("media/textures/{dir}/{file}.png")
}

impl SmartTexture {
    fn add_to_cat(&mut self, cat: i32) {
        let n = self.commands.len();
        self.category.entry(cat).or_default().push(n);
    }

    pub(crate) fn first_from_category(&mut self, cat: i32) -> Option<&mut CombinerCommand> {
        let i = *self.category.get(&cat)?.first()?;
        self.commands.get_mut(i)
    }

    fn cmd(
        &mut self,
        tex: TexRef,
        shader: SmartShader,
        params: Vec<ShaderParam>,
        mask: Option<TexRef>,
        blend: [u32; 4],
    ) {
        self.commands.push(CombinerCommand {
            tex,
            mask,
            shader,
            params,
            blend,
            rect: None,
        });
    }

    /// `add(tex)`.
    pub(crate) fn add(&mut self, t: TexRef) {
        self.cmd(
            t,
            SmartShader::Blit,
            vec![],
            None,
            [
                GL_SRC_ALPHA,
                GL_ONE_MINUS_SRC_ALPHA,
                GL_ONE,
                GL_ONE_MINUS_SRC_ALPHA,
            ],
        );
    }

    pub(crate) fn add_tint(&mut self, t: TexRef, cat: i32, c: Rgb) {
        self.add_to_cat(cat);
        self.cmd(
            t,
            SmartShader::HueChange,
            vec![
                ShaderParam::f("R", c[0]),
                ShaderParam::f("G", c[1]),
                ShaderParam::f("B", c[2]),
            ],
            None,
            [
                GL_SRC_ALPHA,
                GL_ONE_MINUS_SRC_ALPHA,
                GL_ONE,
                GL_ONE_MINUS_SRC_ALPHA,
            ],
        );
    }

    fn add_hue(&mut self, t: TexRef, cat: i32, h: f32) {
        self.add_to_cat(cat);
        self.cmd(
            t,
            SmartShader::HueChange,
            vec![ShaderParam::f("HueChange", h)],
            None,
            [
                GL_SRC_ALPHA,
                GL_ONE_MINUS_SRC_ALPHA,
                GL_ONE,
                GL_ONE_MINUS_SRC_ALPHA,
            ],
        );
    }

    /// `addTexture(base, category, tint, hue)`.
    fn add_texture(&mut self, base: TexRef, cat: i32, tint: Rgb, hue: f32) {
        if tint != WHITE {
            self.add_tint(base, cat, tint);
        } else if !(-1.0e-4..=1.0e-4).contains(&hue) {
            self.add_hue(base, cat, hue);
        } else {
            self.add(base);
        }
    }

    /// `addOverlay(tex, mask, intensity, category)` (blood).
    pub(crate) fn add_overlay(&mut self, t: &str, mask: &str, intensity: f32, cat: i32) {
        self.add_to_cat(cat);
        self.cmd(
            tex(t),
            SmartShader::OverlayMask,
            vec![
                ShaderParam::f("intensity", intensity),
                ShaderParam {
                    name: "bloodDark".into(),
                    min: 0.5,
                    max: 0.5,
                },
            ],
            Some(tex(mask)),
            [
                GL_DST_COLOR,
                GL_ONE_MINUS_SRC_ALPHA,
                GL_DST_ALPHA,
                GL_ONE_MINUS_SRC_ALPHA,
            ],
        );
    }

    /// `addDirtOverlay`.
    fn add_dirt_overlay(&mut self, t: &str, mask: &str, intensity: f32, cat: i32) {
        self.add_to_cat(cat);
        self.cmd(
            tex(t),
            SmartShader::DirtMask,
            vec![ShaderParam::f("intensity", intensity)],
            Some(tex(mask)),
            [
                GL_DST_COLOR,
                GL_ONE_MINUS_SRC_ALPHA,
                GL_DST_ALPHA,
                GL_ONE_MINUS_SRC_ALPHA,
            ],
        );
    }

    /// `addOverlayPatches`.
    fn add_overlay_patches(&mut self, t: &str, mask: &str, cat: i32) {
        self.add_to_cat(cat);
        self.cmd(
            tex(t),
            SmartShader::Blit,
            vec![],
            Some(tex(mask)),
            [
                GL_SRC_ALPHA,
                GL_ONE_MINUS_SRC_ALPHA,
                GL_ONE,
                GL_ONE_MINUS_SRC_ALPHA,
            ],
        );
    }

    /// The `addBlood`/`addDirt` merge: add to the first command of the
    /// category, or start one.
    fn add_intensity(&mut self, cat: i32, intensity: f32, start: impl FnOnce(&mut Self)) {
        if let Some(c) = self.first_from_category(cat) {
            if let Some(p) = c.params.iter_mut().find(|p| p.name == "intensity") {
                let v = (p.min + intensity).min(1.0);
                p.min = v;
                p.max = v;
                return;
            }
            start(self);
            return;
        }
        start(self);
    }

    fn mask(
        &mut self,
        t: TexRef,
        mask: TexRef,
        cat: i32,
        shader: SmartShader,
        params: Vec<ShaderParam>,
    ) {
        self.add_to_cat(cat);
        self.cmd(
            t,
            shader,
            params,
            Some(mask),
            [
                GL_SRC_ALPHA,
                GL_ONE_MINUS_SRC_ALPHA,
                GL_ONE,
                GL_ONE_MINUS_SRC_ALPHA,
            ],
        );
    }

    /// `addMaskedTexture`.
    fn add_masked_texture(
        &mut self,
        mask: &CharacterMask,
        folder: &str,
        base: TexRef,
        cat: i32,
        tint: Rgb,
        hue: f32,
    ) {
        if mask.is_nothing_visible() {
            return;
        }
        if mask.is_all_visible() {
            self.add_texture(base, cat, tint, hue);
            return;
        }
        for part in mask.visible_leaves() {
            let m = tex(&format!("{folder}/{}.png", part.name()));
            if tint != WHITE {
                self.mask(
                    base.clone(),
                    m,
                    cat,
                    SmartShader::BodyMaskTint,
                    vec![
                        ShaderParam::f("R", tint[0]),
                        ShaderParam::f("G", tint[1]),
                        ShaderParam::f("B", tint[2]),
                    ],
                );
            } else if !(-1.0e-4..=1.0e-4).contains(&hue) {
                self.mask(
                    base.clone(),
                    m,
                    cat,
                    SmartShader::BodyMaskHue,
                    vec![ShaderParam::f("HueChange", hue)],
                );
            } else {
                self.mask(base.clone(), m, cat, SmartShader::BodyMask, vec![]);
            }
        }
    }

    /// `addTintedOverlay`.
    pub(crate) fn add_tinted_overlay(&mut self, t: &str, mask: &str, intensity: f32, cat: i32, c: Rgb) {
        self.add_overlay(t, mask, intensity, cat);
        self.add_tint(tex(t), cat, c);
    }

    /// `removeHole(bodyTex, part)`.
    fn remove_hole(&mut self, body: TexRef, part: BloodBodyPartType) {
        self.add_to_cat(3);
        let m = tex(&mask_file("HoleTextures", MASK_FILES[part.index()]));
        self.cmd(
            body,
            SmartShader::RemoveHole,
            vec![],
            Some(m),
            [
                GL_SRC_ALPHA,
                GL_ONE_MINUS_SRC_ALPHA,
                GL_ONE,
                GL_ONE_MINUS_SRC_ALPHA,
            ],
        );
    }
}

/// The jobs and the textures they give the model instances.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TextureCreatorOutput {
    /// In execution order.
    pub jobs: Vec<SmartTextureJob>,
}

impl TextureCreatorOutput {
    fn finish(&mut self, st: SmartTexture) -> usize {
        self.jobs.push(SmartTextureJob {
            commands: st.commands,
        });
        self.jobs.len() - 1
    }
}

/// `ModelInstanceTextureCreator.ItemData`: a clothing item's texture
/// inputs, gathered by `init`.
#[derive(Clone, Debug)]
pub struct ItemTextureData {
    /// The item's model instance in the slot; `None` for texture-only
    /// clothing drawn on the body.
    pub instance: Option<usize>,
    pub mask: CharacterMask,
    pub mask_folder: String,
    /// `media/textures/<name>.png`.
    pub base_texture: String,
    pub tint: Rgb,
    pub hue: f32,
    pub decal: Option<Rc<ClothingDecal>>,
    pub blood: [f32; BLOOD_PARTS],
    pub dirt: [f32; BLOOD_PARTS],
    pub basic: [f32; BLOOD_PARTS],
    pub denim: [f32; BLOOD_PARTS],
    pub leather: [f32; BLOOD_PARTS],
    /// > 0: a hole; -1: a hole in a garment above shows this one.
    pub hole: [f32; BLOOD_PARTS],
}

/// `ModelInstanceTextureCreator.CharacterData` and its `holeMask`.
#[derive(Clone, Debug)]
pub struct CharacterTextureData {
    pub mask: CharacterMask,
    pub mask_folder: String,
    /// `media/textures/Body/<skin>.png`.
    pub base_texture: String,
    pub blood: [f32; BLOOD_PARTS],
    pub dirt: [f32; BLOOD_PARTS],
    pub hole_mask: [bool; BLOOD_PARTS],
}

/// What `ModelInstanceTextureCreator.render` gives the model instances.
#[derive(Clone, Debug, Default)]
pub struct RenderedTextures {
    pub out: TextureCreatorOutput,
    /// The body's job.
    pub body: usize,
    /// Clothing instances and their textures, in assignment order.
    pub instances: Vec<(usize, TextureSource)>,
    /// Instances whose tint is reset to white (`createFullItemTexture`).
    pub white_tint: Vec<usize>,
}

/// Inputs beyond the model slot.
pub struct TextureCreatorInput<'a> {
    pub human: &'a HumanModel<'a>,
    pub item_visuals: &'a [ResolvedItemVisual],
    pub body_locations: &'a BodyLocationGroup,
    /// `ClothingDecals.getDecal` (e.g. `|n| decals.get_decal(fs, n)`).
    pub decals: &'a dyn Fn(&str) -> Option<Rc<ClothingDecal>>,
    /// `Core.isOptionSimpleClothingTextures(zombie)`.
    pub simple_clothing_textures: bool,
}

/// `ModelInstanceTextureCreator.init` + `render`: builds the jobs and
/// points the body and clothing instances of `slot` at their textures.
pub fn create_textures(slot: &mut ModelSlot, input: &TextureCreatorInput) -> TextureCreatorOutput {
    let h = input.human;
    let mut overlayer = CharacterMask::default();
    let mut overlay_folder = DEFAULT_MASKS_FOLDER.to_string();
    let mut hole_mask = [false; BLOOD_PARTS];
    let mut items: Vec<ItemTextureData> = Vec::new();
    let body_sub = slot.instances[slot.model].sub.clone();
    let find_instance = |c: &Rc<ClothingItem>| {
        body_sub.iter().copied().find(|&i| {
            slot.instances[i]
                .item_visual
                .as_ref()
                .and_then(|v| v.clothing_item.as_ref())
                .is_some_and(|ic| Rc::ptr_eq(ic, c))
        })
    };
    let visuals = input.item_visuals;
    for (vi, v) in visuals.iter().enumerate().rev() {
        let Some(c) = &v.clothing_item else { continue };
        if crate::model_tree::is_item_model_hidden(input.body_locations, visuals, vi) {
            continue;
        }
        let inst = find_instance(c);
        if inst.is_none() && c.model(h.female).is_some_and(|m| !m.trim().is_empty()) {
            continue;
        }
        items.push(item_data(
            inst,
            v,
            c,
            &overlayer,
            &overlay_folder,
            &mut hole_mask,
            input,
        ));
        for (j, b) in BloodBodyPartType::ALL.iter().enumerate() {
            if v.hole(j) > 0.0 && overlayer.is_blood_body_part_visible(*b) {
                hole_mask[j] = true;
            }
        }
        for &m in &c.masks {
            if let Some(part) = crate::mask::Part::from_int(m) {
                for b in part.blood_body_part_types() {
                    if v.hole(b.index()) <= 0.0 {
                        hole_mask[b.index()] = false;
                    }
                }
            }
        }
        c.combined_mask(&mut overlayer);
        if !c
            .underlay_masks_folder()
            .eq_ignore_ascii_case(DEFAULT_MASKS_FOLDER)
        {
            overlay_folder = c.underlay_masks_folder().to_string();
        }
    }
    let chr_mask = overlayer.clone();
    let chr_folder = overlay_folder.clone();
    overlayer.set_all_visible(true);
    let overlay_folder = DEFAULT_MASKS_FOLDER.to_string();
    for v in h.body_visuals.iter().rev() {
        let Some(c) = &v.clothing_item else { continue };
        let inst = find_instance(c);
        if inst.is_none() && c.model(h.female).is_some_and(|m| !m.trim().is_empty()) {
            continue;
        }
        items.push(item_data(
            inst,
            v,
            c,
            &overlayer,
            &overlay_folder,
            &mut hole_mask,
            input,
        ));
    }

    let chr = CharacterTextureData {
        mask: chr_mask,
        mask_folder: chr_folder,
        base_texture: format!("media/textures/Body/{}.png", h.skin_texture()),
        blood: std::array::from_fn(|i| h.blood(i)),
        dirt: std::array::from_fn(|i| h.dirt(i)),
        hole_mask,
    };
    let r = render_textures(&chr, &items);
    for &i in &r.white_tint {
        slot.instances[i].tint = [1.0; 3];
    }
    for (i, t) in r.instances {
        slot.instances[i].tex = t;
    }
    slot.instances[slot.model].tex = TextureSource::Smart(r.body);
    r.out
}

/// `ModelInstanceTextureCreator.render`: the jobs for the character and
/// clothing data `init` gathered (`items` in `itemData` order).
pub fn render_textures(chr: &CharacterTextureData, items: &[ItemTextureData]) -> RenderedTextures {
    let mut r = RenderedTextures::default();
    let out = &mut r.out;
    let mut full = SmartTexture::default();
    full.add_texture(tex(&chr.base_texture), 0, WHITE, 0.0);
    for (i, _) in BloodBodyPartType::ALL.iter().enumerate() {
        let m = mask_file("BloodTextures", MASK_FILES[i]);
        if chr.dirt[i] > 0.0 {
            let d = chr.dirt[i];
            full.add_intensity(400 + i as i32, d, |s| {
                s.add_dirt_overlay(
                    "media/textures/BloodTextures/GrimeOverlay.png",
                    &m,
                    d,
                    400 + i as i32,
                )
            });
        }
        if chr.blood[i] > 0.0 {
            let b = chr.blood[i];
            full.add_intensity(300 + i as i32, b, |s| {
                s.add_overlay(
                    "media/textures/BloodTextures/BloodOverlay.png",
                    &m,
                    b,
                    300 + i as i32,
                )
            });
        }
    }
    let full_id = out.finish(full);
    let mut body = SmartTexture::default();
    body.add_masked_texture(
        &chr.mask,
        &chr.mask_folder,
        TexRef::Job(full_id),
        0,
        WHITE,
        0.0,
    );
    for (i, b) in BloodBodyPartType::ALL.iter().enumerate() {
        if chr.hole_mask[i] {
            body.remove_hole(TexRef::Job(full_id), *b);
        }
    }
    for d in items.iter().rev() {
        let item_tex: TexRef;
        if is_simple_texture(d) {
            item_tex = tex(&d.base_texture);
            if !is_item_smart_texture_required(d) {
                r.instances.push((
                    d.instance.unwrap(),
                    TextureSource::File(d.base_texture.clone()),
                ));
                continue;
            }
        } else {
            if let Some(i) = d.instance {
                r.white_tint.push(i);
            }
            item_tex = create_full_item_texture(d, out);
        }
        match d.instance {
            None => apply_item_texture(d, item_tex, &mut body),
            Some(i) => {
                let mut st = SmartTexture::default();
                apply_item_texture(d, item_tex, &mut st);
                let id = out.finish(st);
                r.instances.push((i, TextureSource::Smart(id)));
            }
        }
    }
    r.body = out.finish(body);
    r
}

fn item_data(
    inst: Option<usize>,
    v: &ResolvedItemVisual,
    c: &ClothingItem,
    overlayer: &CharacterMask,
    overlay_folder: &str,
    hole_mask: &mut [bool; BLOOD_PARTS],
    input: &TextureCreatorInput,
) -> ItemTextureData {
    let tex_name = if inst.is_none() {
        v.base_texture(c)
    } else {
        v.texture_choice(c)
    };
    let mut d = ItemTextureData {
        instance: inst,
        mask: overlayer.clone(),
        mask_folder: c.masks_folder().to_string(),
        base_texture: format!("media/textures/{}.png", tex_name.unwrap_or("null")),
        tint: v.tint(c),
        hue: v.hue(c),
        decal: None,
        blood: [0.0; BLOOD_PARTS],
        dirt: [0.0; BLOOD_PARTS],
        basic: [0.0; BLOOD_PARTS],
        denim: [0.0; BLOOD_PARTS],
        leather: [0.0; BLOOD_PARTS],
        hole: [0.0; BLOOD_PARTS],
    };
    if d.mask_folder.eq_ignore_ascii_case(DEFAULT_MASKS_FOLDER) {
        d.mask_folder = overlay_folder.to_string();
    }
    if d.mask_folder.eq_ignore_ascii_case("none") {
        d.mask.set_all_visible(true);
    }
    if d.mask_folder.contains("Clothes/Hat/Masks") {
        d.mask.set_all_visible(true);
    }
    if input.simple_clothing_textures {
        return d;
    }
    if let Some(name) = v.decal(c) {
        if let Some(decal) = (input.decals)(name).filter(|d| d.is_valid()) {
            d.decal = Some(decal);
        }
    }
    for i in 0..BLOOD_PARTS {
        d.blood[i] = v.blood(i);
        d.dirt[i] = v.dirt(i);
        d.basic[i] = v.basic_patch(i);
        d.denim[i] = v.denim_patch(i);
        d.leather[i] = v.leather_patch(i);
        d.hole[i] = v.hole(i);
        if d.hole[i] == 0.0 && hole_mask[i] {
            d.hole[i] = -1.0;
        }
    }
    d
}

fn is_simple_texture(d: &ItemTextureData) -> bool {
    if d.hue != 0.0 {
        return false;
    }
    let tint = if d.instance.is_some() { WHITE } else { d.tint };
    if tint != WHITE || d.decal.is_some() {
        return false;
    }
    (0..BLOOD_PARTS).all(|i| {
        !(d.blood[i] > 0.0
            || d.dirt[i] > 0.0
            || d.hole[i] > 0.0
            || d.basic[i] > 0.0
            || d.denim[i] > 0.0
            || d.leather[i] > 0.0)
    })
}

fn is_item_smart_texture_required(d: &ItemTextureData) -> bool {
    if d.instance.is_none() {
        return true;
    }
    if d.hole.iter().any(|&h| h < 0.0) {
        return true;
    }
    !d.mask.is_all_visible()
}

/// `createFullItemTexture`; holes split it into chained jobs.
fn create_full_item_texture(d: &ItemTextureData, out: &mut TextureCreatorOutput) -> TexRef {
    let mut st = SmartTexture::default();
    st.add_texture(tex(&d.base_texture), 3, d.tint, d.hue);
    if let Some(dec) = &d.decal {
        st.commands.push(CombinerCommand {
            tex: tex(&format!(
                "media/textures/{}.png",
                dec.texture.as_deref().unwrap_or("")
            )),
            mask: None,
            shader: SmartShader::Blit,
            params: vec![],
            blend: [
                GL_SRC_ALPHA,
                GL_ONE_MINUS_SRC_ALPHA,
                GL_ONE,
                GL_ONE_MINUS_SRC_ALPHA,
            ],
            rect: Some([dec.x, dec.y, dec.width, dec.height]),
        });
    }
    for (i, b) in BloodBodyPartType::ALL.iter().enumerate() {
        let m = mask_file("BloodTextures", MASK_FILES[i]);
        if d.blood[i] > 0.0 {
            let v = d.blood[i];
            st.add_intensity(300 + i as i32, v, |s| {
                s.add_overlay(
                    "media/textures/BloodTextures/BloodOverlay.png",
                    &m,
                    v,
                    300 + i as i32,
                )
            });
        }
        if d.dirt[i] > 0.0 {
            let v = d.dirt[i];
            let cat = 400 + i as i32;
            // ItemSmartTexture.addDirt starts a dirt overlay, but adds a
            // (blood) overlay when the category exists without an
            // intensity parameter.
            if st.first_from_category(cat).is_none() {
                st.add_dirt_overlay("media/textures/BloodTextures/GrimeOverlay.png", &m, v, cat);
            } else {
                st.add_intensity(cat, v, |s| {
                    s.add_overlay("media/textures/BloodTextures/GrimeOverlay.png", &m, v, cat)
                });
            }
        }
        for (amount, suffix) in [
            (d.basic[i], "sheet"),
            (d.denim[i], "denim"),
            (d.leather[i], "leather"),
        ] {
            if amount > 0.0 {
                if let Some(f) = patch_mask_file(*b, suffix) {
                    st.add_overlay_patches(
                        &format!("media/textures/patches/{f}.png"),
                        "media/textures/patches/patchesmask.png",
                        300 + i as i32,
                    );
                }
            }
        }
    }
    for (i, b) in BloodBodyPartType::ALL.iter().enumerate() {
        if d.hole[i] > 0.0 {
            // addHole: resolve what is there, then cut the hole out of it.
            st.add_to_cat(3);
            let prev = out.finish(std::mem::take(&mut st));
            let m = tex(&mask_file("HoleTextures", MASK_FILES[b.index()]));
            st.cmd(
                TexRef::Job(prev),
                SmartShader::AddHole,
                vec![],
                Some(m),
                [GL_SRC_ALPHA, GL_ZERO, GL_ONE, GL_ONE_MINUS_SRC_ALPHA],
            );
        }
    }
    TexRef::Job(out.finish(st))
}

/// `applyItemTexture`.
fn apply_item_texture(d: &ItemTextureData, full: TexRef, dest: &mut SmartTexture) {
    dest.add_masked_texture(&d.mask, &d.mask_folder, full.clone(), 3, WHITE, 0.0);
    for (i, b) in BloodBodyPartType::ALL.iter().enumerate() {
        if d.hole[i] < 0.0 {
            dest.remove_hole(full.clone(), *b);
        }
    }
}

/// `EquippedTextureCreator.initTextureName`.
pub fn equipped_texture_name(name: &str, suffix: &str) -> String {
    let name = name.strip_suffix(".png").unwrap_or(name);
    if name.contains("media/") || name.contains("media\\") {
        format!("{name}{suffix}.png")
    } else {
        format!("media/textures/{name}{suffix}.png")
    }
}

/// The `ItemSmartTexture` an `EquippedTextureCreator` builds for a model
/// instance's first render (`init(inst, item)` + `updateTexture`):
/// `texture` is the instance texture's name, `texture_exists` stands for
/// the texture manager's lookups. `None` when the plain texture stays (a
/// white, bloodless item: its fluid overlay is then skipped too, as in the
/// original).
pub fn equipped_texture_job(
    texture: &str,
    init: &crate::items::ItemTextureInit,
    texture_exists: &dyn Fn(&str) -> bool,
) -> Option<SmartTextureJob> {
    crate::item_texture::equipped_job(texture, init, false, texture_exists)
}
