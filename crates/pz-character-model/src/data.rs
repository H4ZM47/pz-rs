//! The data the character model is assembled from.
//!
//! Owned elsewhere and re-exported here:
//! - [`ClothingItem`] and [`ItemVisual`]: pz-inventory (`assets`,
//!   `visual`); [`ResolvedItemVisual`] pairs an `ItemVisual` with what the
//!   original reads through it (`getClothingItem`, the script item's body
//!   location).
//! - [`HumanVisual`]: pz-inventory `visual::human` (through
//!   [`crate::external`]). [`HumanModel`] resolves its lazy getters and adds
//!   what `IHumanVisual` answers.
//! - [`BodyLocationGroup`]: pz-characters `worn_items` (keys are
//!   `ItemBodyLocation` [`RegistryKey`]s).
//! - [`ModelScript`] / [`ModelAttachment`]: pz-scripts.
//! - Held and attached items: pz-inventory `InventoryItem` with its
//!   pz-scripts `Item` (see [`crate::items`]).
//!
//! Local until the characters thread ports them, behind
//! [`crate::external`]: [`ClothingDecal`], [`HairStyles`] and the
//! `PopTemplateManager` skin lists ([`PopTemplateSkins`],
//! [`CharacterStyles`]).
//!
//! The random choices the original makes lazily through `OutfitRNG`
//! (texture choice, tint, hue, decal) are expected already resolved on the
//! visuals (`ItemVisual::pick_uninitialized_values`).

use std::cell::RefCell;
use std::rc::Rc;

use pz_scripts::objects::models;

use crate::mask::{CharacterMask, BLOOD_PARTS};

pub use pz_characters::worn_items::BodyLocationGroup;
pub use pz_inventory::assets::ClothingItem;
pub use pz_inventory::visual::{ItemVisual, Rgb};
pub use pz_scripts::objects::models::ModelAttachment;
pub use pz_scripts::registry::RegistryKey;

use crate::external::HumanVisualOwner;
pub use crate::external::{
    CharacterStyles, ClothingDecal, HairStyle, HairStyles, HumanStyles, HumanVisual,
    PopTemplateSkins,
};

/// `zombie.scripting.objects.ModelScript` as the script manager shares it.
pub type ModelScript = models::ModelScript;
pub type ModelScriptRef = Rc<RefCell<ModelScript>>;
/// A body location (`ItemBodyLocation` registry key).
pub type BodyLocationKey = Rc<RegistryKey>;

/// `ImmutableColor.white`.
pub const WHITE: Rgb = [1.0, 1.0, 1.0];

/// `ClothingItemXML`'s default `m_MasksFolder` / `m_UnderlayMasksFolder`.
pub const DEFAULT_MASKS_FOLDER: &str = "media/textures/Body/Masks";

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

/// A [`HumanVisual`] with what its owner (`IHumanVisual`) answers, its
/// lazy getters resolved (`getSkinTexture`, `getHairModel`,
/// `getBeardModel`, `getHairColor`, `getBeardColor`) and its body visuals
/// resolved.
pub struct HumanModel<'a> {
    pub visual: &'a HumanVisual,
    /// `isFemale()`.
    pub female: bool,
    /// `isZombie()`.
    pub zombie: bool,
    /// `isSkeleton()`.
    pub skeleton: bool,
    /// `bodyVisuals` with their clothing items (the caller resolves
    /// `HumanVisual::body_visuals` against the clothing assets).
    pub body_visuals: &'a [ResolvedItemVisual],
    skin_texture: String,
    hair_model: String,
    beard_model: String,
    hair_color: Rgb,
    beard_color: Rgb,
}

struct Owner {
    female: bool,
    zombie: bool,
    skeleton: bool,
}

impl HumanVisualOwner for Owner {
    fn is_female(&self) -> bool {
        self.female
    }
    fn is_zombie(&self) -> bool {
        self.zombie
    }
    fn is_skeleton(&self) -> bool {
        self.skeleton
    }
}

impl<'a> HumanModel<'a> {
    /// Runs the getters `populateCharacterModelSlot` and the texture
    /// creator call (which fill unset values from `styles`, as the
    /// original does), then keeps the visual read-only.
    pub fn resolve(
        visual: &'a mut HumanVisual,
        female: bool,
        zombie: bool,
        skeleton: bool,
        styles: &dyn HumanStyles,
        body_visuals: &'a [ResolvedItemVisual],
    ) -> Self {
        let owner = Owner {
            female,
            zombie,
            skeleton,
        };
        let hair_model = visual.get_hair_model(&owner, styles).unwrap_or_default();
        let hair_color = visual.get_hair_color(styles).unwrap_or(WHITE);
        let beard_model = visual.get_beard_model(&owner, styles).unwrap_or_default();
        let beard_color = visual.get_beard_color(styles).unwrap_or(WHITE);
        let skin_texture = visual.get_skin_texture(&owner, styles).unwrap_or_default();
        HumanModel {
            visual,
            female,
            zombie,
            skeleton,
            body_visuals,
            skin_texture,
            hair_model,
            beard_model,
            hair_color,
            beard_color,
        }
    }

    /// `getSkinTexture()`.
    pub fn skin_texture(&self) -> &str {
        &self.skin_texture
    }

    /// `getHairModel()`.
    pub fn hair_model(&self) -> &str {
        &self.hair_model
    }

    /// `getBeardModel()`.
    pub fn beard_model(&self) -> &str {
        &self.beard_model
    }

    /// `getHairColor()` (white when unset).
    pub fn hair_color(&self) -> Rgb {
        self.hair_color
    }

    /// `getBeardColor()` (white when unset).
    pub fn beard_color(&self) -> Rgb {
        self.beard_color
    }

    /// `getBlood(part)`.
    pub fn blood(&self, i: usize) -> f32 {
        if i < self.visual.blood.len() {
            self.visual.get_blood(i)
        } else {
            0.0
        }
    }

    /// `getDirt(part)`.
    pub fn dirt(&self, i: usize) -> f32 {
        if i < self.visual.dirt.len() {
            self.visual.get_dirt(i)
        } else {
            0.0
        }
    }
}

/// `ModelScript.getAttachmentById` (blank ids find nothing).
pub fn attachment_by_id(s: &ModelScript, id: &str) -> Option<Rc<RefCell<ModelAttachment>>> {
    if id.trim().is_empty() {
        return None;
    }
    s.attachment_by_id(id)
}
