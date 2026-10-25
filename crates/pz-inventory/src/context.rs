//! What the gameplay methods need from outside the item: the game state
//! ([`ItemContext`]: time, climate, `ZomboidGlobals`, scripts) and the
//! character or object holding the item ([`ItemOwner`]).
//!
//! The original reads these through `item.getContainer().getParent()`,
//! `getOutermostContainer()`, `GameTime`, `ClimateManager` and
//! `ScriptManager`; a [`crate::ItemContainer`] here has no parent, so the
//! caller describes the holder instead.

use std::rc::Rc;

use crate::assets::ItemAssets;
use crate::item::InventoryItem;
use crate::template::ItemTemplate;

/// Game state the item methods read.
pub trait ItemContext {
    /// `GameTime.getWorldAgeHours()`.
    fn world_age_hours(&self) -> f64;
    /// `GameTime.getMultiplier()`.
    fn multiplier(&self) -> f32;
    /// `GameTime.getMinutes()`.
    fn minutes(&self) -> i32 {
        0
    }
    /// `ClimateManager.getRainIntensity()`.
    fn rain_intensity(&self) -> f32;
    /// `ClimateManager.isRaining()`.
    fn is_raining(&self) -> bool;
    /// `ZomboidGlobals.equippedOrWornEncumbranceMultiplier`.
    fn equipped_or_worn_encumbrance_multiplier(&self) -> f64;
    /// `ZomboidGlobals.basicMaskDefense`.
    fn basic_mask_defense(&self) -> f32;
    /// `ZomboidGlobals.improvisedGasMaskDefense`.
    fn improvised_gas_mask_defense(&self) -> f32;
    /// `ZomboidGlobals.standardGasMaskDefense`.
    fn standard_gas_mask_defense(&self) -> f32;
    /// `ScriptManager.FindItem(name)` / `getItem(name)` as a template
    /// (`actualWeight`, `useDelta`, ...). `name` may lack the module.
    fn script(&self, name: &str) -> Option<Rc<ItemTemplate>>;
    /// `InventoryItemFactory.CreateItem(fullType)` (for `AddItem` in
    /// `Use`): `None` when the item cannot be created.
    fn create_item(&self, _full_type: &str) -> Option<InventoryItem> {
        None
    }
    /// `WorldDictionary.getItemTypeFromID(id)` (for `extraItems`).
    fn item_type_from_id(&self, _id: i16) -> Option<String> {
        None
    }
    /// `getFluidContainer().getAmount()` of the item, `None` without a
    /// fluid container. The default decodes the item's `FluidContainer`
    /// component (the amount needs no fluid registry); override it to
    /// decode with the game's entity tables.
    fn fluid_amount(&self, item: &InventoryItem) -> Option<f32> {
        crate::gameplay::fluid_container_amount(item, &pz_entity::BasicContext::default(), 0)
    }
    /// The `ClothingItem` assets and `BloodClothingType` table; without
    /// them no item has a visual (as when the assets are not loaded).
    fn assets(&self) -> Option<&ItemAssets> {
        None
    }
    /// `GameClient.client` (`setBroken` only calls `onBreak` off clients).
    fn is_client(&self) -> bool {
        false
    }
}

/// Where an item is, for `Clothing.getWetDryState()`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ItemPlace {
    /// No container and no world item.
    Nowhere,
    /// An `IsoWorldInventoryObject`; `None` without a square, else its
    /// square's `isInARoom()`.
    World { in_room: Option<bool> },
    /// In an `IsoDeadBody`'s container; `None` without a square.
    DeadBody { in_room: Option<bool> },
    /// In a character's container.
    Character {
        /// `None` without a current square, else `isInARoom() || haveRoof`.
        sheltered_square: Option<bool>,
        /// Asleep or resting in a tent or shelter bed.
        in_tent: bool,
        /// In a vehicle seat with a roof and an intact windshield window.
        in_roofed_vehicle: bool,
    },
    /// A container without a parent object.
    NoParent,
    /// An activated clothing dryer, washer or combination washer-dryer.
    ActiveDryerOrWasher,
    /// Any other container parent.
    OtherParent,
}

/// `Clothing.WetDryState`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WetDryState {
    Invalid,
    Dryer,
    Wetter,
}

/// The character (or object) holding an item, as the original reads it
/// through the item's container. Items are named by their id.
///
/// The defaults describe an item in a container without a parent.
pub trait ItemOwner {
    /// `isEquipped()`: the container's parent is a character and
    /// `chr.isEquipped(item)` (or a dead body's equivalent).
    fn is_equipped(&self, _item: &InventoryItem) -> bool {
        false
    }
    /// `chr.getPrimaryHandItem() == item`.
    fn is_primary_hand_item(&self, _item: &InventoryItem) -> bool {
        false
    }
    /// `chr.getSecondaryHandItem() == item`.
    fn is_secondary_hand_item(&self, _item: &InventoryItem) -> bool {
        false
    }
    /// `Clothing.isWorn()`: in a character's `getWornItems()`.
    fn is_worn(&self, _item: &InventoryItem) -> bool {
        false
    }
    /// `isInPlayerInventory()`: the outermost container's parent is an
    /// `IsoPlayer`.
    fn is_in_player_inventory(&self, _item: &InventoryItem) -> bool {
        false
    }
    /// `getOutermostContainer() == getContainer()` (the item is not inside
    /// a bag).
    fn is_in_outermost_container(&self, _item: &InventoryItem) -> bool {
        true
    }
    /// `getContainer() != null`.
    fn has_container(&self, _item: &InventoryItem) -> bool {
        true
    }
    /// Where the item is, for wetness.
    fn place(&self, _item: &InventoryItem) -> ItemPlace {
        ItemPlace::NoParent
    }
    /// `getWorldItem() != null && getWorldItem().hasComponent(FluidContainer)`:
    /// the world item's fluid amount.
    fn world_item_fluid_amount(&self, _item: &InventoryItem) -> Option<f32> {
        None
    }
    /// The container's `getTemperature()` (for `AddItem` of food).
    fn container_temperature(&self) -> f32 {
        1.0
    }
}

/// An item in a container without a parent object.
pub struct NoOwner;
impl ItemOwner for NoOwner {}

/// An item outside any container.
pub struct Loose;
impl ItemOwner for Loose {
    fn has_container(&self, _item: &InventoryItem) -> bool {
        false
    }
    fn place(&self, _item: &InventoryItem) -> ItemPlace {
        ItemPlace::Nowhere
    }
}

/// `Clothing.getWetDryState()` for an item at `place`.
pub fn wet_dry_state(place: ItemPlace, raining: bool, equipped: bool) -> WetDryState {
    let outdoor = |in_room: Option<bool>| match in_room {
        None => WetDryState::Invalid,
        Some(true) => WetDryState::Dryer,
        Some(false) if raining => WetDryState::Wetter,
        Some(false) => WetDryState::Dryer,
    };
    match place {
        ItemPlace::Nowhere => WetDryState::Invalid,
        ItemPlace::World { in_room } | ItemPlace::DeadBody { in_room } => outdoor(in_room),
        ItemPlace::Character {
            sheltered_square,
            in_tent,
            in_roofed_vehicle,
        } => match sheltered_square {
            None => WetDryState::Invalid,
            Some(true) => WetDryState::Dryer,
            Some(false) if !raining => WetDryState::Dryer,
            Some(false) if !equipped || in_tent || in_roofed_vehicle => WetDryState::Dryer,
            Some(false) => WetDryState::Wetter,
        },
        ItemPlace::NoParent | ItemPlace::OtherParent => WetDryState::Dryer,
        ItemPlace::ActiveDryerOrWasher => WetDryState::Invalid,
    }
}
