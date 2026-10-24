//! `ModelManager.ModelSlot` / `ModelInstance` and the character's model
//! tree: `ModelManager.Add`/`Reset` → `PopTemplateManager.populateCharacterModelSlot`
//! (body, hair, beard, clothing, body visuals) and `DoCharacterModelEquipped`
//! (attached items, hand items, weapon parts and hand replacements as
//! static models; the item getters are in [`crate::items`]).

use std::cell::RefCell;
use std::rc::Rc;

use pz_scripts::objects::items::ItemReplacement;
use pz_scripts::registry::{RegistryKey, ResourceLocation};

use crate::data::{
    attachment_by_id, BeardStyles, BodyLocationGroup, BodyLocationKey, ClothingItem,
    ClothingItemExt, HairStyles, HumanModel, ModelAttachment, ModelScript, ModelScriptRef,
    ResolvedItemVisual, Rgb,
};
use crate::items::{EquippedModels, HeldItem, ItemTextureInit};

/// The primary hand item's bone.
pub const PRIMARY_BONE: &str = "Bip01_Prop1";
/// The secondary hand item's bone.
pub const SECONDARY_BONE: &str = "Bip01_Prop2";
use crate::mask::CharacterMask;

/// Where an instance's texture comes from.
#[derive(Clone, Debug, PartialEq)]
pub enum TextureSource {
    /// No texture yet (`tex == null`): the instance is skipped until the
    /// texture creator gives it one.
    None,
    /// `media/textures/<name>.png`.
    File(String),
    /// The result of a [`crate::texture_creator::SmartTextureJob`].
    Smart(usize),
}

/// What part of the character an instance is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InstanceKind {
    Body,
    Hair,
    Beard,
    Clothing,
    Attached,
    /// A `ModelWeaponPart` on a weapon model.
    WeaponPart,
    PrimaryHand,
    SecondaryHand,
}

/// `ModelInstance`.
#[derive(Clone, Debug)]
pub struct ModelInstance {
    pub kind: InstanceKind,
    /// The model's mesh name (`skinned/malebody`, `skinned/hair/m_hair_crewcut`, ...).
    pub mesh_name: String,
    /// The model's own texture (`Model.tex`), `None` for the body.
    pub model_texture: Option<String>,
    pub is_static: bool,
    pub shader: Option<String>,
    /// `tex`: starts as the model's texture.
    pub tex: TextureSource,
    pub tint: [f32; 3],
    pub hue: f32,
    pub depth_bias: f32,
    pub parent: Option<usize>,
    pub parent_bone: i32,
    pub parent_bone_name: Option<String>,
    pub attachment_name_self: Option<String>,
    pub attachment_name_parent: Option<String>,
    pub scale: f32,
    pub model_script: Option<ModelScriptRef>,
    /// The item visual this instance shows (clothing).
    pub item_visual: Option<ResolvedItemVisual>,
    /// `maskVariableValue` (a masking hand model).
    pub mask_variable_value: Option<String>,
    /// `setTextureInitializer`: the item values its equipped texture is
    /// built from ([`crate::texture_creator::equipped_texture_job`]).
    pub texture_init: Option<ItemTextureInit>,
    pub sub: Vec<usize>,
}

impl ModelInstance {
    fn new(
        kind: InstanceKind,
        mesh: &str,
        texture: Option<&str>,
        is_static: bool,
        shader: Option<&str>,
    ) -> Self {
        ModelInstance {
            kind,
            mesh_name: mesh.to_string(),
            model_texture: texture.map(str::to_string),
            is_static,
            shader: shader.map(str::to_string),
            tex: texture
                .map(|t| TextureSource::File(t.to_string()))
                .unwrap_or(TextureSource::None),
            tint: [1.0; 3],
            hue: 0.0,
            depth_bias: 0.0,
            parent: None,
            parent_bone: 0,
            parent_bone_name: None,
            attachment_name_self: None,
            attachment_name_parent: None,
            scale: 1.0,
            model_script: None,
            item_visual: None,
            mask_variable_value: None,
            texture_init: None,
            sub: Vec::new(),
        }
    }

    /// `getAttachmentById`.
    pub fn attachment(&self, id: Option<&str>) -> Option<Rc<RefCell<ModelAttachment>>> {
        attachment_by_id(&self.model_script.as_ref()?.borrow(), id?)
    }
}

/// Looks up model scripts (`ScriptManager.getModelScript`).
pub trait ModelScripts {
    fn model_script(&self, name: &str) -> Option<ModelScriptRef>;
}

impl ModelScripts for std::collections::HashMap<String, ModelScriptRef> {
    fn model_script(&self, name: &str) -> Option<ModelScriptRef> {
        self.get(name).cloned()
    }
}

impl ModelScripts for pz_scripts::ScriptManager {
    fn model_script(&self, name: &str) -> Option<ModelScriptRef> {
        self.get::<ModelScript>(pz_scripts::ScriptType::Model, name)
    }
}

/// What a character model is built from.
pub struct CharacterModelInput<'a> {
    pub human: &'a HumanModel<'a>,
    /// `getItemVisuals` (worn items) in the character's order.
    pub item_visuals: &'a [ResolvedItemVisual],
    pub equipped: &'a EquippedModels<'a>,
    /// The `Human` body-location group.
    pub body_locations: &'a BodyLocationGroup,
    /// `HairStyles.instance` (`Population::hair_styles`).
    pub hair_styles: Option<&'a HairStyles>,
    /// `BeardStyles.instance` (`Population::beard_styles`).
    pub beard_styles: Option<&'a BeardStyles>,
    pub scripts: &'a dyn ModelScripts,
    /// The body's model script (`getVisual().getModelScript()`), if any.
    pub body_model_script: Option<ModelScriptRef>,
    /// `animPlayer.getSkinningBoneIndex(name, -1)`.
    pub bone_index: &'a dyn Fn(&str) -> i32,
}

/// `ModelManager.ModelSlot`.
#[derive(Clone, Debug, Default)]
pub struct ModelSlot {
    pub instances: Vec<ModelInstance>,
    /// `model`: the body (index 0 once built).
    pub model: usize,
    /// `sub`: the slot's instances in render order.
    pub sub: Vec<usize>,
    pub attached_models: Vec<usize>,
    /// `chr.primaryHandModel`.
    pub primary_hand: Option<usize>,
    /// `chr.secondaryHandModel`.
    pub secondary_hand: Option<usize>,
    /// `chr.setVariable(name, value)` calls the hand replacements made
    /// (`RightHandMask` / `LeftHandMask`), in order.
    pub mask_variables: Vec<(String, String)>,
}

impl ModelSlot {
    pub fn instance(&self, i: usize) -> &ModelInstance {
        &self.instances[i]
    }

    fn push(&mut self, inst: ModelInstance) -> usize {
        self.instances.push(inst);
        self.instances.len() - 1
    }

    /// `ModelManager.Add` / `Reset` for a human character: body model, then
    /// `DoCharacterModelParts`.
    pub fn build(input: &CharacterModelInput) -> ModelSlot {
        let mut slot = ModelSlot::default();
        let h = input.human;
        let body_mesh = if h.zombie && h.skeleton {
            if h.female {
                "skinned/Female_Skeleton"
            } else {
                "skinned/Male_Skeleton"
            }
        } else if h.female {
            "skinned/femalebody"
        } else {
            "skinned/malebody"
        };
        let mut body = ModelInstance::new(InstanceKind::Body, body_mesh, None, false, None);
        body.model_script = input.body_model_script.clone();
        slot.model = slot.push(body);
        slot.populate(input);
        slot.do_equipped(input);
        slot
    }

    /// `PopTemplateManager.populateCharacterModelSlot`.
    fn populate(&mut self, input: &CharacterModelInput) {
        let h = input.human;
        let visuals = input.item_visuals;
        let mut mask = CharacterMask::default();
        for v in visuals.iter().rev() {
            if let Some(c) = &v.clothing_item {
                c.combined_mask(&mut mask);
            }
        }
        if mask.is_part_visible(crate::mask::Part::Head) {
            let hat = visuals
                .iter()
                .find(|v| v.clothing_item.as_ref().is_some_and(|c| c.is_hat()));
            let face = visuals
                .iter()
                .find(|v| v.clothing_item.as_ref().is_some_and(|c| c.is_mask()));
            self.add_head_hair(input, hat, false);
            self.add_head_hair(input, face, true);
        }
        for (i, v) in visuals.iter().enumerate().rev() {
            let Some(c) = &v.clothing_item else { continue };
            if is_item_model_hidden(input.body_locations, visuals, i) {
                continue;
            }
            let alt = is_item_model_alt(input.body_locations, visuals, i);
            self.add_clothing_item(input, v, c, alt);
        }
        for v in h.body_visuals.iter().rev() {
            let Some(c) = &v.clothing_item else { continue };
            self.add_clothing_item(input, v, c, false);
        }
    }

    fn add_head_hair(
        &mut self,
        input: &CharacterModelInput,
        hat: Option<&ResolvedItemVisual>,
        beard_only: bool,
    ) {
        let h = input.human;
        let hat_category = hat
            .and_then(|v| v.clothing_item.as_ref())
            .and_then(|c| c.hat_category.clone());
        let hat_item = hat.and_then(|v| v.clothing_item.as_ref());
        let (model, texture, color, kind) = if !beard_only {
            let Some(styles) = input.hair_styles else {
                return;
            };
            let found = styles.find_style(h.female, Some(h.hair_model()));
            let style = match (found, hat_item) {
                (Some(s), Some(_)) => {
                    styles.get_alternate_for_hat(s, hat_category.as_deref().unwrap_or(""))
                }
                (s, _) => s,
            };
            let Some(style) = style.filter(|s| s.is_valid()) else {
                return;
            };
            (
                &style.model,
                &style.texture,
                h.hair_color(),
                InstanceKind::Hair,
            )
        } else {
            if h.female {
                return;
            }
            let Some(style) = input
                .beard_styles
                .and_then(|b| b.find_style(Some(h.beard_model())))
            else {
                return;
            };
            if !style.is_valid() {
                return;
            }
            if hat_category
                .as_deref()
                .is_some_and(|c| !c.is_empty() && c.contains("nobeard"))
            {
                return;
            }
            (
                &style.model,
                &style.texture,
                h.beard_color(),
                InstanceKind::Beard,
            )
        };
        let model = process_model_file_name(model.as_deref().unwrap_or(""));
        let inst = ModelInstance::new(kind, &model, Some(texture), false, None);
        self.post_process_new_item_instance(inst, color);
    }

    /// `PopTemplateManager.addClothingItem`.
    fn add_clothing_item(
        &mut self,
        input: &CharacterModelInput,
        v: &ResolvedItemVisual,
        c: &Rc<ClothingItem>,
        alt: bool,
    ) -> Option<usize> {
        let female = input.human.female;
        let mut model = c.model(female).map(str::to_string);
        if alt {
            if let Some(a) = c.alt_model(female) {
                model = Some(a.to_string());
            }
        }
        let model = model.filter(|m| !m.trim().is_empty())?;
        let model = process_model_file_name(&model);
        let texture = v.texture_choice(c);
        let tint = v.tint(c);
        let shader = c.shader.as_deref();
        let mut inst = match c.attach_bone.as_deref().filter(|b| !b.is_empty()) {
            Some(bone) => {
                let mut i =
                    ModelInstance::new(InstanceKind::Clothing, &model, texture, true, shader);
                let b = (input.bone_index)(bone);
                if b != -1 {
                    i.parent_bone = b;
                }
                i.parent_bone_name = Some(bone.to_string());
                i
            }
            None => ModelInstance::new(InstanceKind::Clothing, &model, texture, false, shader),
        };
        inst.item_visual = Some(v.clone());
        Some(self.post_process_new_item_instance(inst, tint))
    }

    /// `postProcessNewItemInstance`: parented to the body, inserted first.
    fn post_process_new_item_instance(&mut self, mut inst: ModelInstance, tint: Rgb) -> usize {
        inst.depth_bias = 0.0;
        inst.tint = tint;
        inst.parent = Some(self.model);
        let i = self.push(inst);
        self.instances[self.model].sub.insert(0, i);
        self.sub.insert(0, i);
        i
    }

    /// `addStaticForcedTex(slot, staticModel, boneName, forcedTex)` (and
    /// `addStatic(slot, staticModel, boneName)`): on a bone of the body,
    /// in the slot's and the body's `sub`. A model script's texture wins
    /// over `forced_tex`. (`ModelMetaData`, which only Lua's
    /// `setModelMetaData` fills, is not consulted.)
    fn add_static_on_bone(
        &mut self,
        input: &CharacterModelInput,
        static_model: &str,
        bone: &str,
        kind: InstanceKind,
        forced_tex: Option<&str>,
    ) -> Option<usize> {
        let item_name = get_item_name(static_model);
        let script = input.scripts.model_script(static_model);
        let (mesh, tex, shader) = match &script {
            Some(s) => {
                let s = s.borrow();
                (
                    s.mesh_name.clone().unwrap_or_default(),
                    s.texture_name.clone(),
                    s.shader_name.clone(),
                )
            }
            None => {
                let tex = forced_tex.filter(|t| !t.is_empty()).unwrap_or(item_name);
                (item_name.to_string(), Some(tex.to_string()), None)
            }
        };
        let mut inst = ModelInstance::new(kind, &mesh, tex.as_deref(), true, shader.as_deref());
        inst.parent = Some(self.model);
        let b = (input.bone_index)(bone);
        if b != -1 {
            inst.parent_bone = b;
        }
        inst.parent_bone_name = Some(bone.to_string());
        if let Some(s) = script {
            inst.scale = s.borrow().scale;
            inst.model_script = Some(s);
        }
        let i = self.push(inst);
        self.sub.push(i);
        self.instances[self.model].sub.push(i);
        Some(i)
    }

    /// `addStaticForcedTex(parentInst, modelName, attachNameSelf,
    /// attachNameParent, forcedTex)` (and `addStatic(parentInst, ...)`):
    /// in the parent's `sub` only. `forced_tex` wins over a model script's
    /// texture.
    fn add_static_on_attachment(
        &mut self,
        input: &CharacterModelInput,
        parent: usize,
        model_name: &str,
        attach_self: Option<&str>,
        attach_parent: Option<&str>,
        kind: InstanceKind,
        forced_tex: Option<&str>,
    ) -> Option<usize> {
        let item_name = get_item_name(model_name);
        let script = input.scripts.model_script(model_name);
        let (mesh, mut tex, shader) = match &script {
            Some(s) => {
                let s = s.borrow();
                (
                    s.mesh_name.clone().unwrap_or_default(),
                    s.texture_name.clone(),
                    s.shader_name.clone(),
                )
            }
            None => (item_name.to_string(), Some(item_name.to_string()), None),
        };
        if let Some(t) = forced_tex.filter(|t| !t.is_empty()) {
            tex = Some(t.to_string());
        }
        let mut inst = ModelInstance::new(kind, &mesh, tex.as_deref(), true, shader.as_deref());
        inst.parent = Some(parent);
        if let Some(s) = script {
            inst.scale = s.borrow().scale;
            inst.model_script = Some(s);
        }
        inst.attachment_name_self = attach_self.map(str::to_string);
        inst.attachment_name_parent = attach_parent.map(str::to_string);
        let i = self.push(inst);
        self.instances[parent].sub.push(i);
        Some(i)
    }

    /// `addWeaponPartModels`.
    fn add_weapon_part_models(
        &mut self,
        input: &CharacterModelInput,
        weapon: &HeldItem,
        parent: usize,
    ) {
        for m in weapon.weapon_part_models() {
            self.add_static_on_attachment(
                input,
                parent,
                &m.model_name,
                m.attachment_name_self.as_deref(),
                m.attachment_parent.as_deref(),
                InstanceKind::WeaponPart,
                None,
            );
        }
    }

    /// `DoCharacterModelEquipped` on a fresh slot: attached items, then the
    /// hand items (action and override models first).
    fn do_equipped(&mut self, input: &CharacterModelInput) {
        let eq = input.equipped;
        for a in &eq.attached {
            let Some(model) = a
                .item
                .static_model_exception(input.scripts)
                .filter(|m| !m.trim().is_empty())
            else {
                continue;
            };
            let name = a.attachment_name.as_deref();
            // `ItemBodyLocation.get(ResourceLocation.of(attachmentName))`.
            let loc = name
                .and_then(|n| ResourceLocation::of(n).ok())
                .map(|location| {
                    Rc::new(RegistryKey {
                        location,
                        name: name.unwrap_or_default().to_string(),
                        item_key: None,
                    })
                });
            if loc.is_some_and(|l| should_hide_model(input.body_locations, input.item_visuals, &l))
            {
                continue;
            }
            let Some(i) = self.add_static_on_attachment(
                input,
                self.model,
                &model,
                name,
                name,
                InstanceKind::Attached,
                None,
            ) else {
                continue;
            };
            self.sub.push(i);
            if a.item.is_weapon() {
                self.add_weapon_part_models(input, &a.item, i);
                if !eq.simple_weapon_textures {
                    self.instances[i].texture_init = Some(a.item.texture_init());
                }
            } else if a.item.requires_smart_texture() {
                self.instances[i].texture_init = Some(a.item.texture_init());
            }
            self.attached_models.push(i);
        }

        let mut primary = eq.primary.as_ref();
        let mut secondary = eq.secondary.as_ref();
        if eq.hide_weapon_model {
            primary = None;
            secondary = None;
        }
        if eq.hide_equipped_hand_l {
            secondary = None;
        }
        if eq.hide_equipped_hand_r {
            primary = None;
        }
        if eq.force_null_override {
            primary = None;
            secondary = None;
        }
        let mut override_hands = false;
        if let Some(action) = &eq.action {
            override_hands = true;
            primary = action.primary_item.as_ref();
            if primary.is_none() {
                if let Some(m) = &action.primary_model {
                    self.primary_hand = self.add_static_on_bone(
                        input,
                        m,
                        PRIMARY_BONE,
                        InstanceKind::PrimaryHand,
                        None,
                    );
                }
            }
            secondary = action.secondary_item.as_ref();
            if secondary.is_none() {
                if let Some(m) = &action.secondary_model {
                    self.secondary_hand = self.add_static_on_bone(
                        input,
                        m,
                        SECONDARY_BONE,
                        InstanceKind::SecondaryHand,
                        None,
                    );
                }
            }
        }
        if let Some(m) = eq
            .override_primary_hand_model
            .as_deref()
            .filter(|m| !m.is_empty())
        {
            override_hands = true;
            self.primary_hand =
                self.add_static_on_bone(input, m, PRIMARY_BONE, InstanceKind::PrimaryHand, None);
        }
        if let Some(m) = eq
            .override_secondary_hand_model
            .as_deref()
            .filter(|m| !m.is_empty())
        {
            override_hands = true;
            self.secondary_hand = self.add_static_on_bone(
                input,
                m,
                SECONDARY_BONE,
                InstanceKind::SecondaryHand,
                None,
            );
        }
        if let Some(p) = primary {
            self.primary_hand = self.add_equipped_model_instance(
                input,
                p,
                PRIMARY_BONE,
                p.replacement_primary_hand(),
                p.replacement_primary_clothing.as_ref(),
                override_hands,
                InstanceKind::PrimaryHand,
            );
        }
        if let Some(s) = secondary {
            // `primaryItem != secondaryItem` (identity).
            if !primary.is_some_and(|p| std::ptr::eq(p.item, s.item)) {
                self.secondary_hand = self.add_equipped_model_instance(
                    input,
                    s,
                    SECONDARY_BONE,
                    s.replacement_second_hand(),
                    s.replacement_second_clothing.as_ref(),
                    override_hands,
                    InstanceKind::SecondaryHand,
                );
            }
        }
    }

    /// `addEquippedModelInstance`.
    fn add_equipped_model_instance(
        &mut self,
        input: &CharacterModelInput,
        item: &HeldItem,
        bone: &str,
        replacement: Option<ItemReplacement>,
        replacement_clothing: Option<&Rc<ClothingItem>>,
        override_hands: bool,
        kind: InstanceKind,
    ) -> Option<usize> {
        let scripts = input.scripts;
        if item.clothing_item.is_none() && item.is_weapon() {
            // A weapon without a sprite: the original's `addStatic(null)`
            // fails.
            let sprite = item.static_model(scripts)?;
            let i = self.add_static_on_bone(input, &sprite, bone, kind, None)?;
            self.add_weapon_part_models(input, item, i);
            if !input.equipped.simple_weapon_textures {
                self.instances[i].texture_init = Some(item.texture_init());
            }
            return Some(i);
        }
        if let Some(r) = &replacement {
            let has_value = r
                .mask_variable_value
                .as_deref()
                .is_some_and(|v| !v.is_empty());
            if has_value && !(replacement_clothing.is_none() && !item.has_static_model(scripts)) {
                let i = self.add_masking_model(input, item, r, replacement_clothing, bone, kind);
                if let Some(i) = i {
                    if item.requires_smart_texture() {
                        self.instances[i].texture_init = Some(item.texture_init());
                    }
                }
                return i;
            }
        }
        if override_hands && item.has_static_model(scripts) {
            let model = item
                .fluid_model(scripts)
                .or_else(|| item.static_model(scripts))?;
            let i = self.add_static_on_bone(input, &model, bone, kind, None)?;
            if item.requires_smart_texture() {
                self.instances[i].texture_init = Some(item.texture_init());
            }
            return Some(i);
        }
        None
    }

    /// `addMaskingModel`: the replacement clothing item, or the item's
    /// static model with its texture choice (on the replacement's
    /// attachment, else on the hand bone); then the mask variable.
    fn add_masking_model(
        &mut self,
        input: &CharacterModelInput,
        item: &HeldItem,
        r: &ItemReplacement,
        replacement_clothing: Option<&Rc<ClothingItem>>,
        bone: &str,
        kind: InstanceKind,
    ) -> Option<usize> {
        let value = r.mask_variable_value.clone().unwrap_or_default();
        let result = match (replacement_clothing, &item.visual) {
            (Some(c), Some(v)) => {
                let rv = ResolvedItemVisual {
                    visual: v.clone(),
                    clothing_item: Some(c.clone()),
                    body_location: None,
                };
                let i = self.add_clothing_item(input, &rv, c, false);
                if let Some(i) = i {
                    self.instances[i].kind = kind;
                }
                i
            }
            _ => {
                let model = item
                    .static_model(input.scripts)
                    .filter(|m| !m.trim().is_empty())?;
                let forced = match (&item.visual, &item.clothing_item) {
                    (Some(v), Some(c)) => usize::try_from(v.texture_choice)
                        .ok()
                        .and_then(|t| c.texture_choices.get(t))
                        .cloned(),
                    _ => None,
                };
                let i = match r.attachment.as_deref().filter(|a| !a.is_empty()) {
                    Some(att) => self.add_static_on_attachment(
                        input,
                        self.model,
                        &model,
                        Some(att),
                        Some(att),
                        kind,
                        forced.as_deref(),
                    ),
                    None => self.add_static_on_bone(input, &model, bone, kind, forced.as_deref()),
                };
                if let Some(i) = i {
                    let inst = &mut self.instances[i];
                    inst.mask_variable_value = Some(value.clone());
                    if let Some(t) = item.visual.as_ref().and_then(|v| v.tint) {
                        inst.tint = t;
                    }
                }
                i
            }
        };
        if !value.is_empty() {
            if let Some(name) = &r.mask_variable_name {
                self.mask_variables.push((name.clone(), value));
            }
        }
        result
    }
}

/// `ScriptManager.getItemName`: drops the module prefix.
pub fn get_item_name(name: &str) -> &str {
    name.find('.').map(|p| &name[p + 1..]).unwrap_or(name)
}

/// `processModelFileName`: forward slashes, lower case.
pub fn process_model_file_name(name: &str) -> String {
    name.replace('\\', "/").to_lowercase()
}

fn location_of(group: &BodyLocationGroup, k: Option<&BodyLocationKey>) -> Option<BodyLocationKey> {
    let k = k?;
    group.location(Some(k)).ok().flatten().map(|_| k.clone())
}

fn hides(group: &BodyLocationGroup, a: &BodyLocationKey, b: &BodyLocationKey) -> bool {
    group.is_hide_model(Some(a), Some(b)).unwrap_or(false)
}

/// `isItemModelHidden(group, visuals, visual)`.
pub fn is_item_model_hidden(
    group: &BodyLocationGroup,
    visuals: &[ResolvedItemVisual],
    i: usize,
) -> bool {
    let Some(loc1) = location_of(group, visuals[i].body_location.as_ref()) else {
        return false;
    };
    visuals.iter().enumerate().any(|(j, v)| {
        j != i
            && location_of(group, v.body_location.as_ref())
                .is_some_and(|loc2| hides(group, &loc2, &loc1))
    })
}

/// `isItemModelAlt(group, visuals, visual)`.
pub fn is_item_model_alt(
    group: &BodyLocationGroup,
    visuals: &[ResolvedItemVisual],
    i: usize,
) -> bool {
    let Some(loc1) = location_of(group, visuals[i].body_location.as_ref()) else {
        return false;
    };
    visuals.iter().enumerate().any(|(j, v)| {
        j != i
            && location_of(group, v.body_location.as_ref()).is_some_and(|loc2| {
                group
                    .is_alt_model(Some(&loc2), Some(&loc1))
                    .unwrap_or(false)
            })
    })
}

/// `shouldHideModel(itemVisuals, location)`.
pub fn should_hide_model(
    group: &BodyLocationGroup,
    visuals: &[ResolvedItemVisual],
    loc: &BodyLocationKey,
) -> bool {
    let Some(loc) = location_of(group, Some(loc)) else {
        return false;
    };
    visuals.iter().any(|v| {
        location_of(group, v.body_location.as_ref()).is_some_and(|l2| hides(group, &l2, &loc))
    })
}
