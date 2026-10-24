//! The data the character model is assembled from.
//!
//! Owned elsewhere and re-exported here:
//! - [`ClothingItem`] and [`ItemVisual`]: pz-inventory (`assets`,
//!   `visual`); [`ResolvedItemVisual`] pairs an `ItemVisual` with what the
//!   original reads through it (`getClothingItem`, the script item's body
//!   location).
//! - [`HumanVisual`]: pz-characters `visual` (moving to pz-inventory next
//!   to `ItemVisual`; only the `human_visual` import below names the
//!   owner). [`HumanModel`] adds what `IHumanVisual` answers.
//! - [`BodyLocationGroup`]: pz-characters `worn_items` (keys are
//!   `ItemBodyLocation` [`RegistryKey`]s).
//! - [`ModelScript`] / [`ModelAttachment`]: pz-scripts.
//!
//! Local until an owner ports them: [`ClothingDecal`] (`ClothingDecals`),
//! [`HairStyles`] (`HairStyles`/`BeardStyles`, listed as not ported in
//! pz-characters `visual`), and [`EquippedItems`] (the static models
//! `DoCharacterModelEquipped` reads off `InventoryItem`/`AttachedItems`).
//!
//! The random choices the original makes lazily through `OutfitRNG`
//! (texture choice, tint, hue, decal) are expected already resolved on the
//! visuals (`ItemVisual::pick_uninitialized_values`).

use std::cell::RefCell;
use std::path::Path;
use std::rc::Rc;

use pz_anim::xml::{self, Element};
use pz_scripts::objects::models;

use crate::mask::{CharacterMask, BLOOD_PARTS};

pub use pz_characters::worn_items::BodyLocationGroup;
pub use pz_inventory::assets::ClothingItem;
pub use pz_inventory::visual::{ItemVisual, Rgb};
pub use pz_scripts::objects::models::ModelAttachment;
pub use pz_scripts::registry::RegistryKey;

pub use human_visual::HumanVisual;
/// The `HumanVisual` owner (switch here when it moves to pz-inventory).
use pz_characters::visual as human_visual;

/// `zombie.scripting.objects.ModelScript` as the script manager shares it.
pub type ModelScript = models::ModelScript;
pub type ModelScriptRef = Rc<RefCell<ModelScript>>;
/// A body location (`ItemBodyLocation` registry key).
pub type BodyLocationKey = Rc<RegistryKey>;

/// `ImmutableColor.white`.
pub const WHITE: Rgb = [1.0, 1.0, 1.0];

/// `ClothingItemXML`'s default `m_MasksFolder` / `m_UnderlayMasksFolder`.
pub const DEFAULT_MASKS_FOLDER: &str = "media/textures/Body/Masks";

fn text(e: &Element) -> String {
    e.text_content()
}

fn trimmed(e: &Element) -> String {
    pz_anim::vars::java_trim(&e.text_content()).to_string()
}

/// The `ClothingItem` methods the model code calls that pz-inventory does
/// not (yet) have.
pub trait ClothingItemExt {
    /// `getModel(female)`.
    fn model(&self, female: bool) -> Option<&str>;
    /// `getAltModel(female)`.
    fn alt_model(&self, female: bool) -> Option<&str>;
    /// `getCombinedMask`: hides this item's mask parts.
    fn combined_mask(&self, mask: &mut CharacterMask);
    /// `m_MasksFolder`.
    fn masks_folder(&self) -> &str;
    /// `m_UnderlayMasksFolder`.
    fn underlay_masks_folder(&self) -> &str;
}

impl ClothingItemExt for ClothingItem {
    fn model(&self, female: bool) -> Option<&str> {
        if female {
            self.female_model.as_deref()
        } else {
            self.male_model.as_deref()
        }
    }

    fn alt_model(&self, female: bool) -> Option<&str> {
        if female {
            self.alt_female_model.as_deref()
        } else {
            self.alt_male_model.as_deref()
        }
    }

    fn combined_mask(&self, mask: &mut CharacterMask) {
        mask.set_parts_visible(&self.masks, false);
    }

    fn masks_folder(&self) -> &str {
        self.masks_folder.as_deref().unwrap_or(DEFAULT_MASKS_FOLDER)
    }

    fn underlay_masks_folder(&self) -> &str {
        self.underlay_masks_folder
            .as_deref()
            .unwrap_or(DEFAULT_MASKS_FOLDER)
    }
}

/// An [`ItemVisual`] with what the model code reads through it:
/// `getClothingItem()` and `getScriptItem().getBodyLocation()`.
#[derive(Clone, Debug, PartialEq)]
pub struct ResolvedItemVisual {
    pub visual: ItemVisual,
    pub clothing_item: Option<Rc<ClothingItem>>,
    pub body_location: Option<BodyLocationKey>,
}

impl ResolvedItemVisual {
    /// A visual of `full_type` (`new ItemVisual()` + `setItemType`).
    pub fn new(full_type: &str, clothing_item: Option<Rc<ClothingItem>>) -> Self {
        let mut visual = ItemVisual::new(full_type);
        if let Some(c) = &clothing_item {
            visual.clothing_item_name = c.name.clone();
        }
        ResolvedItemVisual {
            visual,
            clothing_item,
            body_location: None,
        }
    }

    /// `InventoryItem.getVisual()` (creating and resolving the visual as
    /// the original does) with its clothing item; `None` without a ready
    /// clothing item. The body location is the caller's (the script
    /// item's, as an `ItemBodyLocation` key).
    pub fn from_item(
        item: &mut pz_inventory::InventoryItem,
        assets: &pz_inventory::ItemAssets,
        owner: &dyn pz_inventory::ItemOwner,
        rng: &dyn pz_inventory::visual::VisualRng,
        body_location: Option<BodyLocationKey>,
    ) -> Option<Self> {
        let c = Rc::new(item.get_clothing_item(Some(assets), owner)?.clone());
        let visual = item.get_visual(Some(assets), owner, rng)?.clone();
        Some(ResolvedItemVisual {
            visual,
            clothing_item: Some(c),
            body_location,
        })
    }

    fn byte(&self, arr: usize, part: usize) -> f32 {
        // ItemVisual's getters, by array.
        match arr {
            0 => self.visual.get_blood(part),
            1 => self.visual.get_dirt(part),
            2 => self.visual.get_hole(part),
            3 => self.visual.get_basic_patch(part),
            4 => self.visual.get_denim_patch(part),
            _ => self.visual.get_leather_patch(part),
        }
    }

    pub fn blood(&self, i: usize) -> f32 {
        self.byte(0, i)
    }
    pub fn dirt(&self, i: usize) -> f32 {
        self.byte(1, i)
    }
    pub fn hole(&self, i: usize) -> f32 {
        self.byte(2, i)
    }
    pub fn basic_patch(&self, i: usize) -> f32 {
        self.byte(3, i)
    }
    pub fn denim_patch(&self, i: usize) -> f32 {
        self.byte(4, i)
    }
    pub fn leather_patch(&self, i: usize) -> f32 {
        self.byte(5, i)
    }

    fn index(i: i8, len: usize) -> Option<usize> {
        (len > 0).then(|| (i.max(0) as usize).min(len - 1))
    }

    /// `getTextureChoice` (an unresolved choice reads the first).
    pub fn texture_choice<'a>(&self, c: &'a ClothingItem) -> Option<&'a str> {
        Self::index(self.visual.texture_choice, c.texture_choices.len())
            .map(|i| c.texture_choices[i].as_str())
    }

    /// `getBaseTexture` (an unresolved choice reads the first).
    pub fn base_texture<'a>(&self, c: &'a ClothingItem) -> Option<&'a str> {
        Self::index(self.visual.base_texture, c.base_textures.len())
            .map(|i| c.base_textures[i].as_str())
    }

    /// `getTint`: the resolved (or custom) colour, white when unset.
    pub fn tint(&self, _c: &ClothingItem) -> Rgb {
        self.visual.tint.unwrap_or(WHITE)
    }

    /// `getHue`: 0 unless the item allows a random hue (unresolved reads 0).
    pub fn hue(&self, c: &ClothingItem) -> f32 {
        if c.allow_random_hue && self.visual.hue.is_finite() {
            self.visual.hue
        } else {
            0.0
        }
    }

    /// `getDecal`.
    pub fn decal(&self, c: &ClothingItem) -> Option<&str> {
        if c.decal_group.as_deref().is_none_or(|g| g.trim().is_empty()) {
            return None;
        }
        self.visual.decal.as_deref()
    }

    /// Sets a byte per `BloodBodyPartType` (`setBlood`, ... with 0..255).
    pub fn set_blood(&mut self, part: usize, amount: f32) {
        self.visual.set_blood(part, amount, BLOOD_PARTS);
    }
}

/// A [`HumanVisual`] with what its owner (`IHumanVisual`) answers and its
/// body visuals resolved.
pub struct HumanModel<'a> {
    pub visual: &'a HumanVisual,
    /// `isFemale()`.
    pub female: bool,
    /// `isZombie()`.
    pub zombie: bool,
    /// `isSkeleton()`.
    pub skeleton: bool,
    /// `bodyVisuals`, resolved. pz-characters keeps only the item type and
    /// clothing item name of each; this goes once `HumanVisual` holds full
    /// `ItemVisual`s (pz-inventory).
    pub body_visuals: &'a [ResolvedItemVisual],
}

impl HumanModel<'_> {
    /// `getSkinTexture()`: the stored name. TODO(PopTemplateManager): the
    /// name from the `skinTexture` index when none is stored.
    pub fn skin_texture(&self) -> &str {
        self.visual.skin_texture_name.as_deref().unwrap_or("")
    }

    pub fn hair_model(&self) -> &str {
        self.visual.hair_model.as_deref().unwrap_or("")
    }

    pub fn beard_model(&self) -> &str {
        self.visual.beard_model.as_deref().unwrap_or("")
    }

    fn rgb(c: Option<human_visual::ImmutableColor>) -> Rgb {
        c.map_or(WHITE, |c| [c.r, c.g, c.b])
    }

    /// `getHairColor()` (white when unset).
    pub fn hair_color(&self) -> Rgb {
        Self::rgb(self.visual.hair_color)
    }

    /// `getBeardColor()` (white when unset).
    pub fn beard_color(&self) -> Rgb {
        Self::rgb(self.visual.beard_color)
    }

    /// `getBlood(part)`.
    pub fn blood(&self, i: usize) -> f32 {
        self.visual.blood.get(i).map_or(0.0, |&b| b as f32 / 255.0)
    }

    /// `getDirt(part)`.
    pub fn dirt(&self, i: usize) -> f32 {
        self.visual.dirt.get(i).map_or(0.0, |&b| b as f32 / 255.0)
    }
}

/// `ModelScript.getAttachmentById` (blank ids find nothing).
pub fn attachment_by_id(s: &ModelScript, id: &str) -> Option<Rc<RefCell<ModelAttachment>>> {
    if id.trim().is_empty() {
        return None;
    }
    s.attachment_by_id(id)
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
        let mut d = ClothingDecal {
            name: name.into(),
            texture: String::new(),
            x: 0,
            y: 0,
            width: 0,
            height: 0,
        };
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
        !self.texture.trim().is_empty()
            && self.x >= 0
            && self.y >= 0
            && self.width > 0
            && self.height > 0
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
        self.model.as_deref().is_some_and(|m| !m.trim().is_empty())
            && self
                .texture
                .as_deref()
                .is_some_and(|t| !t.trim().is_empty())
    }

    /// `getAlternate(category)`.
    pub fn alternate_for(&self, category: &str) -> &str {
        self.alternate
            .iter()
            .find(|(c, _)| c.eq_ignore_ascii_case(category))
            .map(|(_, s)| s.as_str())
            .unwrap_or(&self.name)
    }

    fn from_element(e: &Element) -> HairStyle {
        let mut s = HairStyle {
            name: String::new(),
            model: None,
            texture: Some("F_Hair_White".into()),
            alternate: Vec::new(),
            level: 0,
        };
        for f in e.elements() {
            match f.tag.as_str() {
                "name" => s.name = text(f),
                "model" => s.model = Some(text(f)),
                "texture" => s.texture = Some(text(f)),
                "level" => s.level = trimmed(f).parse().unwrap_or(0),
                "alternate" => s.alternate.push((
                    f.attr_or_empty("category").to_string(),
                    f.attr_or_empty("style").to_string(),
                )),
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
        self.beards
            .iter()
            .find(|s| s.name.eq_ignore_ascii_case(name))
    }

    /// `getAlternateForHat`.
    pub fn alternate_for_hat<'a>(
        &'a self,
        style: &'a HairStyle,
        category: Option<&str>,
        female: bool,
    ) -> Option<&'a HairStyle> {
        let category = category.unwrap_or("");
        if category.eq_ignore_ascii_case("nohair") || category.eq_ignore_ascii_case("nohairnobeard")
        {
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

/// A static model the character carries: an attached item (`AttachedItem`,
/// on its location's attachment point) or a hand item.
#[derive(Clone, Debug, PartialEq)]
pub struct StaticItemModel {
    /// The model script name (`getStaticModel` / `getStaticModelException`).
    pub model: String,
    /// The attachment point name (attached items) or bone (hand items).
    pub attachment: String,
    /// The item's body location for `shouldHideModel` (attached items).
    pub body_location: Option<BodyLocationKey>,
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
