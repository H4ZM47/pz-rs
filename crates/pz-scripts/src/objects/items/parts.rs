//! Small value classes `Item` stores: `ItemReplacement`,
//! `VehiclePartModel` and `ModelWeaponPart`.

/// `zombie.scripting.objects.ItemReplacement`: a clothing item or anim
/// mask that replaces a hand while the item is held.
/// TODO(ClothingItem): the resolved `clothingItem` asset.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ItemReplacement {
    pub clothing_item_name: Option<String>,
    pub mask_variable_name: Option<String>,
    pub mask_variable_value: Option<String>,
    pub attachment: Option<String>,
}

/// `zombie.scripting.objects.VehiclePartModel`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct VehiclePartModel {
    pub part_id: String,
    pub part_model_id: String,
    pub model_id: String,
}

/// `zombie.scripting.objects.ModelWeaponPart`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ModelWeaponPart {
    pub part_type: String,
    pub model_name: String,
    pub attachment_name_self: Option<String>,
    pub attachment_parent: Option<String>,
}
