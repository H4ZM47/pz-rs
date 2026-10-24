//! Static models for the items a character holds or has attached: what
//! `ModelManager` decides from an item (hand replacements, the weapon-part
//! models of `addWeaponPartModels`, the `_Fluid` model variant), and what
//! `ModelInstanceTextureInitializer` / `EquippedTextureCreator.init` take
//! from the item. `ModelManager.DoCharacterModelEquipped` itself is in
//! [`crate::model_tree`].
//!
//! The item getters themselves (`getStaticModel`, `getWorldStaticItem`,
//! `getStaticModelException`, `getModelWeaponPart`, `Food.isRotten` and
//! `getFluidContainer`) are pz-inventory's ([`InventoryItem`], in
//! `pz_inventory::model`). The script item ([`Item`]) is only read for
//! `replacePrimaryHand`/`replaceSecondHand` (the template keeps just the
//! clothing name). `getClothingItem()` / `getVisual()` and the
//! replacement's clothing asset are resolved by the caller through
//! pz-inventory (see [`HeldItem::with_visual`]).

use std::cell::RefCell;
use std::rc::Rc;

use pz_entity::{EntityContext, FluidContainer};
use pz_inventory::{InventoryItem, ItemKind};
use pz_scripts::objects::items::{Item, ItemReplacement};

use crate::data::{ClothingItem, ItemVisual, Rgb, WHITE};
use crate::model_tree::ModelScripts;

pub use pz_inventory::model::TAG_USE_WORLD_STATIC_MODEL;

/// A script item as the script manager shares it.
pub type ItemScriptRef = Rc<RefCell<Item>>;

/// An item the character holds or carries, with its script and what the
/// caller resolved for it.
#[derive(Clone)]
pub struct HeldItem<'a> {
    pub item: &'a InventoryItem,
    /// `getScriptItem()`.
    pub script: Option<ItemScriptRef>,
    /// `getFluidContainer()`, if the item has one.
    pub fluid: Option<FluidContainer>,
    /// What the fluid colours resolve against (`FluidContainer.getColor`);
    /// without it only custom fluid colours resolve (others are white).
    pub entity_ctx: Option<&'a dyn EntityContext>,
    /// `getClothingItem()` (with the hand replacement it selects while
    /// held).
    pub clothing_item: Option<Rc<ClothingItem>>,
    /// `getVisual()`.
    pub visual: Option<ItemVisual>,
    /// `getItemReplacementPrimaryHand().clothingItem`.
    pub replacement_primary_clothing: Option<Rc<ClothingItem>>,
    /// `getItemReplacementSecondHand().clothingItem`.
    pub replacement_second_clothing: Option<Rc<ClothingItem>>,
}

/// A context without registered fluids: only custom colours resolve.
struct NoFluids;

impl EntityContext for NoFluids {
    fn attribute_kind(&self, _id: i16) -> Option<pz_entity::AttributeKind> {
        None
    }
}

fn blank(s: Option<&str>) -> bool {
    s.is_none_or(|s| s.trim().is_empty())
}

/// `ScriptManager.getModelScript(name).getName()`, as the pz-inventory
/// getters take it.
pub fn model_script_name(scripts: &dyn ModelScripts, name: &str) -> Option<String> {
    scripts
        .model_script(name)
        .map(|m| m.borrow().name.clone().unwrap_or_default())
}

impl<'a> HeldItem<'a> {
    pub fn new(item: &'a InventoryItem, script: Option<ItemScriptRef>) -> Self {
        HeldItem {
            item,
            script,
            fluid: None,
            entity_ctx: None,
            clothing_item: None,
            visual: None,
            replacement_primary_clothing: None,
            replacement_second_clothing: None,
        }
    }

    /// The item and its script from the script manager.
    pub fn from_scripts(item: &'a InventoryItem, sm: &pz_scripts::ScriptManager) -> Self {
        HeldItem::new(
            item,
            pz_scripts::objects::items::get_item(sm, &item.get_full_type()),
        )
    }

    /// Sets `getClothingItem()`/`getVisual()` (from
    /// `InventoryItem::get_clothing_item` / `get_visual`).
    pub fn with_visual(
        mut self,
        clothing_item: Option<Rc<ClothingItem>>,
        visual: Option<ItemVisual>,
    ) -> Self {
        self.clothing_item = clothing_item;
        self.visual = visual;
        self
    }

    /// Reads the item's `FluidContainer` component
    /// (`InventoryItem::get_fluid_container`).
    pub fn with_fluid(mut self, ctx: &'a dyn EntityContext) -> Self {
        self.fluid = self.item.get_fluid_container(ctx);
        self.entity_ctx = Some(ctx);
        self
    }

    /// Resolves the hand replacements' clothing assets by name.
    pub fn with_replacement_assets(mut self, assets: &pz_inventory::ItemAssets) -> Self {
        let get = |r: Option<ItemReplacement>| {
            r.and_then(|r| r.clothing_item_name)
                .and_then(|n| assets.clothing_items.get(&n).cloned())
                .map(Rc::new)
        };
        self.replacement_primary_clothing = get(self.replacement_primary_hand());
        self.replacement_second_clothing = get(self.replacement_second_hand());
        self
    }

    fn script_field<T>(&self, f: impl FnOnce(&Item) -> T) -> Option<T> {
        self.script.as_ref().map(|s| f(&s.borrow()))
    }

    pub fn is_weapon(&self) -> bool {
        matches!(self.item.kind, ItemKind::Weapon(_))
    }

    /// `getStaticModel()` ([`InventoryItem::get_static_model`]) with the
    /// model scripts.
    pub fn static_model(&self, scripts: &dyn ModelScripts) -> Option<String> {
        self.item
            .get_static_model(&|n| model_script_name(scripts, n))
    }

    /// `getStaticModelException()`
    /// ([`InventoryItem::get_static_model_exception`]) with the model
    /// scripts.
    pub fn static_model_exception(&self, scripts: &dyn ModelScripts) -> Option<String> {
        self.item
            .get_static_model_exception(&|n| model_script_name(scripts, n))
    }

    /// `getItemReplacementPrimaryHand()`.
    pub fn replacement_primary_hand(&self) -> Option<ItemReplacement> {
        self.script_field(|s| s.replace_primary_hand.clone())
            .flatten()
    }

    /// `getItemReplacementSecondHand()`.
    pub fn replacement_second_hand(&self) -> Option<ItemReplacement> {
        self.script_field(|s| s.replace_second_hand.clone())
            .flatten()
    }

    /// `ModelManager.requiresSmartTexture`.
    pub fn requires_smart_texture(&self) -> bool {
        let i = self.item;
        i.color_red != 1.0
            || i.color_green != 1.0
            || i.color_blue != 1.0
            || self.fluid.as_ref().is_some_and(|f| !f.is_empty())
    }

    /// `ModelInstanceTextureInitializer.init(inst, item)` →
    /// `EquippedTextureCreator.init(inst, item)`.
    pub fn texture_init(&self) -> ItemTextureInit {
        let i = self.item;
        let blood_level = match &i.kind {
            ItemKind::Weapon(w) => w.blood_level,
            _ => 0.0,
        };
        let tint = if i.color_red * i.color_green * i.color_blue != 1.0 {
            [i.color_red, i.color_green, i.color_blue]
        } else {
            WHITE
        };
        ItemTextureInit {
            blood_level,
            tint,
            fluid_level: self.fluid.as_ref().map_or(0.0, |f| f.get_filled_ratio()),
            fluid_tint: self
                .fluid
                .as_ref()
                .map(|f| f.get_color(self.entity_ctx.unwrap_or(&NoFluids))),
        }
    }

    /// `addWeaponPartModels`: for each attached part (in attach order),
    /// every `getModelWeaponPart()` entry of its full type, in script order.
    pub fn weapon_part_models(&self) -> Vec<WeaponPartModel> {
        let ItemKind::Weapon(w) = &self.item.kind else {
            return Vec::new();
        };
        let Some(mwps) = self.item.get_model_weapon_part() else {
            return Vec::new();
        };
        let mut out = Vec::new();
        for part in &w.attachments {
            let ft = part.get_full_type();
            for m in mwps.iter().filter(|m| m.part_type == ft) {
                out.push(WeaponPartModel {
                    part_type: m.part_type.clone(),
                    model_name: m.model_name.clone(),
                    attachment_name_self: m.attachment_name_self.clone(),
                    attachment_parent: m.attachment_parent.clone(),
                });
            }
        }
        out
    }

    /// `item.hasComponent(FluidContainer) && getFilledRatio() > 0.5` with a
    /// `<worldStaticItem>_Fluid` model script: the model name to use.
    pub(crate) fn fluid_model(&self, scripts: &dyn ModelScripts) -> Option<String> {
        let f = self.fluid.as_ref()?;
        if f.get_filled_ratio() <= 0.5 {
            return None;
        }
        let name = format!(
            "{}_Fluid",
            self.item
                .get_world_static_model()
                .as_deref()
                .unwrap_or("null")
        );
        scripts.model_script(&name).map(|_| name)
    }

    /// `StringUtils.isNullOrWhitespace(item.getStaticModel())`.
    pub(crate) fn has_static_model(&self, scripts: &dyn ModelScripts) -> bool {
        !blank(self.static_model(scripts).as_deref())
    }
}

/// One `ModelWeaponPart` that applies to a weapon's attached part.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WeaponPartModel {
    pub part_type: String,
    pub model_name: String,
    pub attachment_name_self: Option<String>,
    pub attachment_parent: Option<String>,
}

/// What `EquippedTextureCreator` takes from an item (the texture
/// initializer a weapon or tinted/filled item model gets).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ItemTextureInit {
    /// `HandWeapon.getBloodLevel()` (0 otherwise).
    pub blood_level: f32,
    /// The item colour when not white.
    pub tint: Rgb,
    /// `FluidContainer.getFilledRatio()`.
    pub fluid_level: f32,
    pub fluid_tint: Option<Rgb>,
}

/// `BaseAction` with `overrideHandModels` set: what it puts in the hands.
#[derive(Clone, Default)]
pub struct ActionHandModels<'a> {
    /// `getPrimaryHandItem()`.
    pub primary_item: Option<HeldItem<'a>>,
    /// `getPrimaryHandMdl()` (used without a primary item).
    pub primary_model: Option<String>,
    pub secondary_item: Option<HeldItem<'a>>,
    pub secondary_model: Option<String>,
}

/// An `AttachedItem` with its location's attachment name
/// (`getGroup().getLocation(location).getAttachmentName()`).
#[derive(Clone)]
pub struct AttachedModel<'a> {
    pub attachment_name: Option<String>,
    pub item: HeldItem<'a>,
}

/// What `DoCharacterModelEquipped` reads off the character.
#[derive(Clone, Default)]
pub struct EquippedModels<'a> {
    /// `getAttachedItems()` in order.
    pub attached: Vec<AttachedModel<'a>>,
    /// `getPrimaryHandItem()`.
    pub primary: Option<HeldItem<'a>>,
    /// `getSecondaryHandItem()` (the same `InventoryItem` as the primary
    /// for a two-handed hold).
    pub secondary: Option<HeldItem<'a>>,
    /// `isHideWeaponModel()`.
    pub hide_weapon_model: bool,
    /// `isHideEquippedHandL()`.
    pub hide_equipped_hand_l: bool,
    /// `isHideEquippedHandR()`.
    pub hide_equipped_hand_r: bool,
    /// An `IsoPlayer` with `forceNullOverride`.
    pub force_null_override: bool,
    /// The first character action, when it has `overrideHandModels`.
    pub action: Option<ActionHandModels<'a>>,
    /// `overridePrimaryHandModel`.
    pub override_primary_hand_model: Option<String>,
    /// `overrideSecondaryHandModel`.
    pub override_secondary_hand_model: Option<String>,
    /// `Core.getOptionSimpleWeaponTextures()`.
    pub simple_weapon_textures: bool,
}

impl<'a> EquippedModels<'a> {
    /// The attached items of a pz-characters `AttachedItems`, with each
    /// item's [`HeldItem`] from `held`.
    pub fn attached_from<I: pz_characters::worn_items::ItemIdentity>(
        items: &'a pz_characters::attached_items::AttachedItems<I>,
        held: impl Fn(&'a I) -> HeldItem<'a>,
    ) -> Vec<AttachedModel<'a>> {
        let group = items.group().borrow();
        items
            .items()
            .iter()
            .map(|a| AttachedModel {
                attachment_name: group
                    .location(Some(a.location()))
                    .and_then(|l| l.borrow().attachment_name().map(str::to_string)),
                item: held(a.item()),
            })
            .collect()
    }
}
