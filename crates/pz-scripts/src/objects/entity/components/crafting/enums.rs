//! Enums the crafting scripts store: `zombie.entity.components.crafting`'s
//! `StartMode`, `ItemApplyMode`, `FluidMatchMode`, `InputFlag`, `OutputFlag`,
//! and `zombie.scripting.objects`' `CraftRecipeTag`, `CraftRecipeGroup`,
//! `WallCoveringType` and `SignType`.

use crate::base::ScriptResult;
use crate::java;
use crate::objects::entity::illegal_argument;

entity_enum!(
    /// `StartMode`.
    StartMode,
    "zombie.entity.components.crafting.StartMode",
    [Manual, Automatic, Passive]
);

impl StartMode {
    /// `getByteId`.
    pub fn byte_id(self) -> i8 {
        self as i8 + 1
    }

    /// `getBits`.
    pub fn bits(self) -> i32 {
        1 << (self as i32)
    }
}

entity_enum!(
    /// `ItemApplyMode`.
    ItemApplyMode,
    "zombie.entity.components.crafting.ItemApplyMode",
    [Normal, Keep, Destroy]
);

entity_enum!(
    /// `FluidMatchMode`.
    FluidMatchMode,
    "zombie.entity.components.crafting.FluidMatchMode",
    [Exact, Primary, Mixture, Anything]
);

entity_enum!(
    /// `InputFlag`.
    InputFlag,
    "zombie.entity.components.crafting.InputFlag",
    [
        HandcraftOnly, AutomationOnly, IsFull, NotFull, ItemIsUses, ItemIsFluid, ItemIsEnergy, IsEmpty, NotEmpty,
        Prop1, Prop2, ToolLeft, ToolRight, IsDamaged, IsUndamaged, IsWholeFoodItem, IsEmptyContainer,
        IsUncookedFoodItem, IsCookedFoodItem, IsNotDull, IsHeadPart, IsSharpenable, DontPutBack, InheritColor,
        InheritCondition, InheritEquipped, InheritSharpness, InheritHeadCondition, MayDegrade, MayDegradeLight,
        MayDegradeVeryLight, MayDegradeHeavy, SharpnessCheck, InheritUses, InheritUsesAndEmpty, InheritFood,
        InheritFoodAge, InheritCooked, InheritModelVariation, InheritWeight, InheritName, InheritFreezingTime,
        DontInheritCondition, AllowFrozenItem, AllowRottenItem, NoBrokenItems, AllowDestroyedItem, IsWorn, IsNotWorn,
        InheritAmmunition, CopyClothing, AllowFavorite, InheritFavorite, FakeOutput, DontReplace, CanBeDoneFromFloor,
        ItemCount, IsExclusive, RecordInput, DontRecordInput, ResearchInput, IsBlunt, HasOneUse, HasNoUses, IsSealed,
        IsNotSealed, Unseal, EquipSecondary, SetActivated,
    ]
);

entity_enum!(
    /// `OutputFlag`.
    OutputFlag,
    "zombie.entity.components.crafting.OutputFlag",
    [
        HandcraftOnly, AutomationOnly, IsEmpty, ForceEmpty, AlwaysFill, RespectCapacity, IsBlunt, HasOneUse,
        HasNoUses, DontInheritCondition, EquipSecondary, SetActivated,
    ]
);

/// The `CraftRecipeTag` ids the scripts test for (`toString()` values).
pub mod craft_recipe_tag {
    pub const ANY_SURFACE_CRAFT: &str = "AnySurfaceCraft";
    pub const AUTO_ROTATE: &str = "AutoRotate";
    pub const CANNOT_BE_RESEARCHED: &str = "CannotBeResearched";
    pub const CAN_ALWAYS_BE_RESEARCHED: &str = "CanAlwaysBeResearched";
    pub const CAN_BE_DONE_FROM_FLOOR: &str = "CanBeDoneFromFloor";
    pub const CAN_BE_DONE_IN_DARK: &str = "CanBeDoneInDark";
    pub const ENTITY_RECIPE: &str = "EntityRecipe";
    pub const IN_HAND_CRAFT: &str = "InHandCraft";
    pub const OUTDOORS: &str = "Outdoors";
}

/// `CraftRecipeGroup`.
#[allow(non_camel_case_types)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CraftRecipeGroup {
    OPEN_BOX,
}

impl CraftRecipeGroup {
    /// `toString`: the group id.
    pub fn id(self) -> &'static str {
        match self {
            CraftRecipeGroup::OPEN_BOX => "OpenBox",
        }
    }

    /// The icon texture path. TODO(Texture): `getIconTexture`.
    pub fn icon_path(self) -> &'static str {
        match self {
            CraftRecipeGroup::OPEN_BOX => "media/textures/Build_CrateWood.png",
        }
    }

    /// `fromString`: exact id match, `None` otherwise.
    pub fn from_string(id: &str) -> Option<CraftRecipeGroup> {
        [CraftRecipeGroup::OPEN_BOX].into_iter().find(|g| g.id() == id)
    }

    /// `getTranslationName`'s key. TODO(Translator): `getRecipeGroupName`.
    pub fn translation_key(self) -> String {
        format!("RecipeGroup_{}", self.id())
    }
}

/// `WallCoveringType`.
#[allow(non_camel_case_types)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum WallCoveringType {
    PAINT_THUMP,
    PAINT_SIGN,
    PLASTER,
    WALLPAPER,
}

impl WallCoveringType {
    const ALL: [WallCoveringType; 4] = [
        WallCoveringType::PAINT_THUMP,
        WallCoveringType::PAINT_SIGN,
        WallCoveringType::PLASTER,
        WallCoveringType::WALLPAPER,
    ];

    /// `toString`.
    pub fn type_string(self) -> &'static str {
        match self {
            WallCoveringType::PAINT_THUMP => "paintThump",
            WallCoveringType::PAINT_SIGN => "paintSign",
            WallCoveringType::PLASTER => "plaster",
            WallCoveringType::WALLPAPER => "wallpaper",
        }
    }

    /// `typeOf`: case-insensitive.
    pub fn type_of(s: &str) -> ScriptResult<WallCoveringType> {
        Self::ALL
            .into_iter()
            .find(|t| java::eq_ignore_case(t.type_string(), s))
            .ok_or_else(|| illegal_argument(&format!("No enum constant WallCoveringType with value {s}")))
    }
}

/// `SignType`.
#[allow(non_camel_case_types)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SignType {
    SIGN_SKULL,
    SIGN_RIGHTARROW,
    SIGN_LEFTARROW,
    SIGN_DOWNARROW,
    SIGN_UPARROW,
}

impl SignType {
    const ALL: [SignType; 5] = [
        SignType::SIGN_SKULL,
        SignType::SIGN_RIGHTARROW,
        SignType::SIGN_LEFTARROW,
        SignType::SIGN_DOWNARROW,
        SignType::SIGN_UPARROW,
    ];

    /// `getSpriteIndex`.
    pub fn sprite_index(self) -> i32 {
        match self {
            SignType::SIGN_SKULL => 36,
            SignType::SIGN_RIGHTARROW => 32,
            SignType::SIGN_LEFTARROW => 33,
            SignType::SIGN_DOWNARROW => 34,
            SignType::SIGN_UPARROW => 35,
        }
    }

    /// `getSpriteName`.
    pub fn sprite_name(self) -> String {
        format!("constructedobjects_signs_01_{}", self.sprite_index())
    }

    /// `typeOf`.
    pub fn type_of(index: i32) -> ScriptResult<SignType> {
        Self::ALL
            .into_iter()
            .find(|t| t.sprite_index() == index)
            .ok_or_else(|| illegal_argument(&format!("No enum constant SignType with value {index}")))
    }
}
