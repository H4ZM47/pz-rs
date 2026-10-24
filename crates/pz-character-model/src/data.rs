//! The data the character model is assembled from.
//!
//! Loaded here from game files: `ClothingItem` (`media/clothing/clothingItems`),
//! `ClothingDecal` (`media/clothing/clothingDecals`), hair and beard styles
//! (`media/hairStyles`).
//!
//! Defined as minimal input structs, to be filled by the crates that own
//! them once they merge (pz-inventory `ItemVisual`/`InventoryItem`,
//! pz-scripts `ModelScript`/`ModelAttachment` and the body-location groups
//! from `BodyLocations.lua`): [`ItemVisual`], [`HumanVisual`],
//! [`ModelScript`], [`BodyLocationGroup`], [`EquippedItems`]. The random
//! choices the original makes lazily through `OutfitRNG` (texture choice,
//! tint, hue, decal, skin) are expected already resolved on these inputs.

use std::collections::HashMap;
use std::path::Path;
use std::rc::Rc;

use pz_anim::xml::{self, Element};

use crate::mask::{CharacterMask, BLOOD_PARTS};

/// `ImmutableColor` (rgb; alpha is not used by the model code).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ImmutableColor {
    pub r: f32,
    pub g: f32,
    pub b: f32,
}

impl ImmutableColor {
    pub const WHITE: ImmutableColor = ImmutableColor { r: 1.0, g: 1.0, b: 1.0 };

    pub fn new(r: f32, g: f32, b: f32) -> Self {
        ImmutableColor { r, g, b }
    }
}

fn text(e: &Element) -> String {
    e.text_content()
}

fn trimmed(e: &Element) -> String {
    pz_anim::vars::java_trim(&e.text_content()).to_string()
}

fn fix_path(s: String) -> String {
    s.replace('\\', "/")
}

/// `ClothingItem` as loaded through `ClothingItemXML`.
#[derive(Clone, Debug, PartialEq)]
pub struct ClothingItem {
    /// The file name without extension (`m_Name`/`mame`).
    pub name: String,
    pub guid: String,
    pub male_model: Option<String>,
    pub female_model: Option<String>,
    pub alt_male_model: Option<String>,
    pub alt_female_model: Option<String>,
    pub is_static: bool,
    pub base_textures: Vec<String>,
    pub attach_bone: Option<String>,
    pub masks: Vec<i32>,
    pub masks_folder: String,
    pub underlay_masks_folder: String,
    pub texture_choices: Vec<String>,
    pub allow_random_hue: bool,
    pub allow_random_tint: bool,
    pub decal_group: Option<String>,
    pub shader: Option<String>,
    pub hat_category: Option<String>,
    pub spawn_with: Vec<String>,
}

pub const DEFAULT_MASKS_FOLDER: &str = "media/textures/Body/Masks";

impl ClothingItem {
    pub fn from_element(name: &str, root: &Element) -> ClothingItem {
        let mut c = ClothingItem {
            name: name.to_string(),
            guid: String::new(),
            male_model: None,
            female_model: None,
            alt_male_model: None,
            alt_female_model: None,
            is_static: false,
            base_textures: Vec::new(),
            attach_bone: None,
            masks: Vec::new(),
            masks_folder: DEFAULT_MASKS_FOLDER.into(),
            underlay_masks_folder: DEFAULT_MASKS_FOLDER.into(),
            texture_choices: Vec::new(),
            allow_random_hue: false,
            allow_random_tint: false,
            decal_group: None,
            shader: None,
            hat_category: None,
            spawn_with: Vec::new(),
        };
        let b = |e: &Element| matches!(trimmed(e).as_str(), "true" | "1");
        for f in root.elements() {
            match f.tag.as_str() {
                "m_GUID" => c.guid = text(f),
                "m_MaleModel" => c.male_model = Some(fix_path(text(f))),
                "m_FemaleModel" => c.female_model = Some(fix_path(text(f))),
                "m_AltMaleModel" => c.alt_male_model = Some(fix_path(text(f))),
                "m_AltFemaleModel" => c.alt_female_model = Some(fix_path(text(f))),
                "m_Static" => c.is_static = b(f),
                "m_BaseTextures" => c.base_textures.push(fix_path(text(f))),
                "m_AttachBone" => c.attach_bone = Some(text(f)),
                "m_Masks" => {
                    if let Ok(v) = trimmed(f).parse() {
                        c.masks.push(v)
                    }
                }
                "m_MasksFolder" => c.masks_folder = fix_path(text(f)),
                "m_UnderlayMasksFolder" => c.underlay_masks_folder = fix_path(text(f)),
                "textureChoices" => c.texture_choices.push(fix_path(text(f))),
                "m_AllowRandomHue" => c.allow_random_hue = b(f),
                "m_AllowRandomTint" => c.allow_random_tint = b(f),
                "m_DecalGroup" => c.decal_group = Some(text(f)),
                "m_Shader" => c.shader = Some(text(f)),
                "m_HatCategory" => c.hat_category = Some(text(f)),
                "m_SpawnWith" => c.spawn_with.push(fix_path(text(f))),
                _ => {}
            }
        }
        c
    }

    /// Loads `<dir>/<name>.xml`.
    pub fn load(dir: &Path, name: &str) -> Option<ClothingItem> {
        let root = xml::parse_xml_file(&dir.join(format!("{name}.xml"))).ok()?;
        Some(ClothingItem::from_element(name, &root))
    }

    pub fn model(&self, female: bool) -> Option<&str> {
        if female { self.female_model.as_deref() } else { self.male_model.as_deref() }
    }

    pub fn alt_model(&self, female: bool) -> Option<&str> {
        if female { self.alt_female_model.as_deref() } else { self.alt_male_model.as_deref() }
    }

    fn hat(&self) -> Option<&str> {
        self.hat_category.as_deref().filter(|s| !s.trim().is_empty())
    }

    /// `isHat`.
    pub fn is_hat(&self) -> bool {
        self.hat().is_some_and(|h| h != "nobeard")
    }

    /// `isMask`.
    pub fn is_mask(&self) -> bool {
        self.hat().is_some_and(|h| h == "nohairnobeard" || !h.contains("hair"))
    }

    /// `getCombinedMask`: hides this item's mask parts.
    pub fn combined_mask(&self, mask: &mut CharacterMask) {
        mask.set_parts_visible(&self.masks, false);
    }
}

/// `ClothingDecal`.
#[derive(Clone, Debug, PartialEq)]
pub struct ClothingDecal {
    pub name: String,
    pub texture: String,
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

impl ClothingDecal {
    pub fn load(dir: &Path, name: &str) -> Option<ClothingDecal> {
        let root = xml::parse_xml_file(&dir.join(format!("{name}.xml"))).ok()?;
        let mut d = ClothingDecal { name: name.into(), texture: String::new(), x: 0, y: 0, width: 0, height: 0 };
        for f in root.elements() {
            let i = || trimmed(f).parse().unwrap_or(0);
            match f.tag.as_str() {
                "texture" => d.texture = text(f),
                "x" => d.x = i(),
                "y" => d.y = i(),
                "width" => d.width = i(),
                "height" => d.height = i(),
                _ => {}
            }
        }
        Some(d)
    }

    /// `isValid`.
    pub fn is_valid(&self) -> bool {
        !self.texture.trim().is_empty() && self.x >= 0 && self.y >= 0 && self.width > 0 && self.height > 0
    }
}

/// `HairStyle` / `BeardStyle`.
#[derive(Clone, Debug, PartialEq)]
pub struct HairStyle {
    pub name: String,
    pub model: Option<String>,
    pub texture: Option<String>,
    /// (category, style)
    pub alternate: Vec<(String, String)>,
    pub level: i32,
}

impl HairStyle {
    /// `isValid`.
    pub fn is_valid(&self) -> bool {
        self.model.as_deref().is_some_and(|m| !m.trim().is_empty()) && self.texture.as_deref().is_some_and(|t| !t.trim().is_empty())
    }

    /// `getAlternate(category)`.
    pub fn alternate_for(&self, category: &str) -> &str {
        self.alternate.iter().find(|(c, _)| c.eq_ignore_ascii_case(category)).map(|(_, s)| s.as_str()).unwrap_or(&self.name)
    }

    fn from_element(e: &Element) -> HairStyle {
        let mut s = HairStyle { name: String::new(), model: None, texture: Some("F_Hair_White".into()), alternate: Vec::new(), level: 0 };
        for f in e.elements() {
            match f.tag.as_str() {
                "name" => s.name = text(f),
                "model" => s.model = Some(text(f)),
                "texture" => s.texture = Some(text(f)),
                "level" => s.level = trimmed(f).parse().unwrap_or(0),
                "alternate" => s.alternate.push((f.attr_or_empty("category").to_string(), f.attr_or_empty("style").to_string())),
                _ => {}
            }
        }
        s
    }
}

/// `HairStyles` and `BeardStyles`.
#[derive(Clone, Debug, Default)]
pub struct HairStyles {
    pub male: Vec<HairStyle>,
    pub female: Vec<HairStyle>,
    pub beards: Vec<HairStyle>,
}

impl HairStyles {
    /// `hairStyles.xml` and `beardStyles.xml` from `media/hairStyles`.
    pub fn load(dir: &Path) -> HairStyles {
        let mut h = HairStyles::default();
        if let Ok(root) = xml::parse_xml_file(&dir.join("hairStyles.xml")) {
            for e in root.elements() {
                match e.tag.as_str() {
                    "male" => h.male.push(HairStyle::from_element(e)),
                    "female" => h.female.push(HairStyle::from_element(e)),
                    _ => {}
                }
            }
        }
        if let Ok(root) = xml::parse_xml_file(&dir.join("beardStyles.xml")) {
            for e in root.elements().filter(|e| e.tag == "style") {
                h.beards.push(HairStyle::from_element(e));
            }
        }
        h
    }

    fn find(list: &[HairStyle], name: &str) -> Option<usize> {
        for (i, s) in list.iter().enumerate() {
            if s.name.eq_ignore_ascii_case(name) {
                return Some(i);
            }
            if name.is_empty() && s.name.eq_ignore_ascii_case("bald") {
                return Some(i);
            }
        }
        None
    }

    pub fn find_male(&self, name: &str) -> Option<&HairStyle> {
        Self::find(&self.male, name).map(|i| &self.male[i])
    }

    pub fn find_female(&self, name: &str) -> Option<&HairStyle> {
        Self::find(&self.female, name).map(|i| &self.female[i])
    }

    pub fn find_beard(&self, name: &str) -> Option<&HairStyle> {
        self.beards.iter().find(|s| s.name.eq_ignore_ascii_case(name))
    }

    /// `getAlternateForHat`.
    pub fn alternate_for_hat<'a>(&'a self, style: &'a HairStyle, category: Option<&str>, female: bool) -> Option<&'a HairStyle> {
        let category = category.unwrap_or("");
        if category.eq_ignore_ascii_case("nohair") || category.eq_ignore_ascii_case("nohairnobeard") {
            return None;
        }
        let alt = style.alternate_for(category);
        if female && self.female.iter().any(|s| s == style) {
            return self.find_female(alt);
        }
        if !female && self.male.iter().any(|s| s == style) {
            return self.find_male(alt);
        }
        Some(style)
    }
}

/// An `ItemVisual` with its script item's facts and its lazily chosen
/// randomness already resolved.
#[derive(Clone, Debug, PartialEq)]
pub struct ItemVisual {
    pub item_type: String,
    pub clothing_item: Option<Rc<ClothingItem>>,
    /// The script item's body location (`Item.getBodyLocation`).
    pub body_location: Option<String>,
    /// Index into `textureChoices` (`getTextureChoice`).
    pub texture_choice: usize,
    /// Index into `baseTextures` (`getBaseTexture`).
    pub base_texture: usize,
    /// `getTint`: the custom colour, the random tint, or white.
    pub tint: ImmutableColor,
    /// `getHue`.
    pub hue: f32,
    /// `getDecal`: the chosen decal of the item's decal group.
    pub decal: Option<String>,
    /// 0..255 per `BloodBodyPartType` like the original's byte arrays.
    pub blood: [u8; BLOOD_PARTS],
    pub dirt: [u8; BLOOD_PARTS],
    pub holes: [u8; BLOOD_PARTS],
    pub basic_patches: [u8; BLOOD_PARTS],
    pub denim_patches: [u8; BLOOD_PARTS],
    pub leather_patches: [u8; BLOOD_PARTS],
}

impl ItemVisual {
    pub fn new(item_type: &str, clothing_item: Option<Rc<ClothingItem>>) -> Self {
        ItemVisual {
            item_type: item_type.into(),
            clothing_item,
            body_location: None,
            texture_choice: 0,
            base_texture: 0,
            tint: ImmutableColor::WHITE,
            hue: 0.0,
            decal: None,
            blood: [0; BLOOD_PARTS],
            dirt: [0; BLOOD_PARTS],
            holes: [0; BLOOD_PARTS],
            basic_patches: [0; BLOOD_PARTS],
            denim_patches: [0; BLOOD_PARTS],
            leather_patches: [0; BLOOD_PARTS],
        }
    }

    fn byte(v: &[u8; BLOOD_PARTS], i: usize) -> f32 {
        v[i] as f32 / 255.0
    }

    pub fn blood(&self, i: usize) -> f32 {
        Self::byte(&self.blood, i)
    }
    pub fn dirt(&self, i: usize) -> f32 {
        Self::byte(&self.dirt, i)
    }
    pub fn hole(&self, i: usize) -> f32 {
        Self::byte(&self.holes, i)
    }
    pub fn basic_patch(&self, i: usize) -> f32 {
        Self::byte(&self.basic_patches, i)
    }
    pub fn denim_patch(&self, i: usize) -> f32 {
        Self::byte(&self.denim_patches, i)
    }
    pub fn leather_patch(&self, i: usize) -> f32 {
        Self::byte(&self.leather_patches, i)
    }

    /// `getTextureChoice`.
    pub fn texture_choice<'a>(&self, c: &'a ClothingItem) -> Option<&'a str> {
        c.texture_choices.get(self.texture_choice.min(c.texture_choices.len().saturating_sub(1))).map(String::as_str)
    }

    /// `getBaseTexture`.
    pub fn base_texture<'a>(&self, c: &'a ClothingItem) -> Option<&'a str> {
        c.base_textures.get(self.base_texture.min(c.base_textures.len().saturating_sub(1))).map(String::as_str)
    }

    /// `getTint`: white unless the item allows a random tint (or the input
    /// carries a custom colour).
    pub fn tint(&self, c: &ClothingItem) -> ImmutableColor {
        if c.allow_random_tint || self.tint != ImmutableColor::WHITE {
            self.tint
        } else {
            ImmutableColor::WHITE
        }
    }

    /// `getHue`.
    pub fn hue(&self, c: &ClothingItem) -> f32 {
        if c.allow_random_hue {
            self.hue
        } else {
            0.0
        }
    }

    /// `getDecal`.
    pub fn decal(&self, c: &ClothingItem) -> Option<&str> {
        if c.decal_group.as_deref().is_none_or(|g| g.trim().is_empty()) {
            return None;
        }
        self.decal.as_deref()
    }
}

/// `HumanVisual` with its random choices resolved.
#[derive(Clone, Debug, PartialEq)]
pub struct HumanVisual {
    pub female: bool,
    pub zombie: bool,
    pub skeleton: bool,
    /// `getSkinTexture()` (e.g. `MaleBody01a`).
    pub skin_texture: String,
    pub hair_model: String,
    pub hair_color: ImmutableColor,
    pub beard_model: String,
    pub beard_color: ImmutableColor,
    pub body_visuals: Vec<ItemVisual>,
    /// 0..1 per `BloodBodyPartType`.
    pub blood: [f32; BLOOD_PARTS],
    pub dirt: [f32; BLOOD_PARTS],
}

/// `ModelAttachment`.
#[derive(Clone, Debug, PartialEq)]
pub struct ModelAttachment {
    pub id: String,
    pub bone: Option<String>,
    pub offset: [f32; 3],
    /// Degrees.
    pub rotate: [f32; 3],
    pub scale: f32,
}

/// `ModelScript`, the parts the character model reads.
#[derive(Clone, Debug, PartialEq)]
pub struct ModelScript {
    pub name: String,
    pub mesh_name: String,
    pub texture_name: Option<String>,
    pub shader_name: Option<String>,
    pub is_static: bool,
    pub scale: f32,
    pub attachments: Vec<ModelAttachment>,
    /// `cullFace` (-1 default, 0 none, else a GL face).
    pub cull_face: i32,
}

impl ModelScript {
    /// `ModelScript.Load`: parses one `model <name> { ... }` block of a
    /// script file (`mesh`, `texture`, `shader`, `static`, `scale`,
    /// `cullFace`, `undoCoreScale`, `attachment <id> { offset, rotate,
    /// bone, scale }`).
    pub fn parse(name: &str, block: &str) -> ModelScript {
        let mut s = ModelScript {
            name: name.into(),
            mesh_name: String::new(),
            texture_name: None,
            shader_name: None,
            is_static: true,
            scale: 1.0,
            attachments: Vec::new(),
            cull_face: -1,
        };
        let body = match (block.find('{'), block.rfind('}')) {
            (Some(a), Some(b)) if b > a => &block[a + 1..b],
            _ => block,
        };
        let mut undo_core_scale = false;
        let mut rest = body;
        let vec3 = |v: &str| {
            let p: Vec<f32> = v.split(' ').filter(|x| !x.is_empty()).filter_map(pz_anim::vars::java_parse_float).collect();
            [p.first().copied().unwrap_or(0.0), p.get(1).copied().unwrap_or(0.0), p.get(2).copied().unwrap_or(0.0)]
        };
        loop {
            let t = rest.trim_start();
            if t.is_empty() {
                break;
            }
            let brace = t.find('{');
            let comma = t.find(',');
            match (brace, comma) {
                (Some(b), c) if c.is_none_or(|c| b < c) => {
                    let head = t[..b].trim();
                    let end = t[b..].find('}').map(|e| b + e).unwrap_or(t.len() - 1);
                    let inner = &t[b + 1..end];
                    if let Some(id) = head.strip_prefix("attachment") {
                        let mut a = ModelAttachment { id: id.trim().to_string(), bone: None, offset: [0.0; 3], rotate: [0.0; 3], scale: 1.0 };
                        for kv in inner.split(',') {
                            let Some((k, v)) = kv.split_once('=') else { continue };
                            let (k, v) = (k.trim(), v.trim());
                            match k {
                                "bone" => a.bone = Some(v.to_string()),
                                "offset" => a.offset = vec3(v),
                                "rotate" => a.rotate = vec3(v),
                                "scale" => a.scale = pz_anim::vars::try_parse_float(v, 1.0),
                                _ => {}
                            }
                        }
                        match s.attachments.iter_mut().find(|x| x.id == a.id) {
                            Some(x) => *x = a,
                            None => s.attachments.push(a),
                        }
                    }
                    rest = &t[(end + 1).min(t.len())..];
                }
                (_, Some(c)) => {
                    if let Some((k, v)) = t[..c].split_once('=') {
                        let (k, v) = (k.trim(), v.trim());
                        match k.to_ascii_lowercase().as_str() {
                            "mesh" => s.mesh_name = v.to_string(),
                            "scale" => s.scale = pz_anim::vars::java_parse_float(v).unwrap_or(1.0),
                            "shader" => s.shader_name = Some(v.to_string()),
                            "static" => s.is_static = v.eq_ignore_ascii_case("true"),
                            "texture" => s.texture_name = Some(v.to_string()),
                            "cullface" => {
                                s.cull_face = match v.to_ascii_lowercase().as_str() {
                                    "back" => 1029,
                                    "front" => 1028,
                                    "none" => 0,
                                    _ => s.cull_face,
                                }
                            }
                            "undocorescale" => undo_core_scale = v.eq_ignore_ascii_case("true"),
                            _ => {}
                        }
                    }
                    rest = &t[c + 1..];
                }
                _ => break,
            }
        }
        if undo_core_scale {
            s.scale *= 0.6666667;
        }
        s
    }

    /// `getAttachmentById`.
    pub fn attachment(&self, id: &str) -> Option<&ModelAttachment> {
        if id.trim().is_empty() {
            return None;
        }
        self.attachments.iter().find(|a| a.id == id)
    }
}

/// `BodyLocationGroup`, the rules the model code reads: which locations
/// hide or switch to the alternate model of which others.
#[derive(Clone, Debug, Default)]
pub struct BodyLocationGroup {
    /// location id → (hide-model locations, alt-model locations)
    pub locations: HashMap<String, (Vec<String>, Vec<String>)>,
}

impl BodyLocationGroup {
    pub fn has(&self, loc: &str) -> bool {
        self.locations.contains_key(loc)
    }

    /// `isHideModel(a, b)`: does location `a` hide `b`'s model.
    pub fn is_hide_model(&self, a: &str, b: &str) -> bool {
        self.locations.get(a).is_some_and(|(h, _)| h.iter().any(|x| x == b))
    }

    /// `isAltModel(a, b)`.
    pub fn is_alt_model(&self, a: &str, b: &str) -> bool {
        self.locations.get(a).is_some_and(|(_, alt)| alt.iter().any(|x| x == b))
    }
}

/// A static model the character carries: an attached item (`AttachedItem`,
/// on its location's attachment point) or a hand item.
#[derive(Clone, Debug, PartialEq)]
pub struct StaticItemModel {
    /// The model script name (`getStaticModel` / `getStaticModelException`).
    pub model: String,
    /// The attachment point name (attached items) or bone (hand items).
    pub attachment: String,
    /// The item's body location for `shouldHideModel` (attached items).
    pub body_location: Option<String>,
}

/// What the character holds and carries (`DoCharacterModelEquipped`):
/// static weapon / item models only.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct EquippedItems {
    pub attached: Vec<StaticItemModel>,
    /// Model on `Bip01_Prop1`.
    pub primary: Option<String>,
    /// Model on `Bip01_Prop2` (skipped when the same item as the primary).
    pub secondary: Option<String>,
}
