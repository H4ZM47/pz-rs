//! The seam to types other workstreams own. Each owner's type comes in
//! through exactly one `pub use` here; the rest of the crate names only
//! these paths.

// owner: pz-inventory (items thread). `HumanVisual` and the style/owner
// traits its lazy getters use.
pub use pz_inventory::visual::human::{HumanStyles, HumanVisual, HumanVisualOwner, SkinList};

// owner: pz-characters `population` (characters thread): `ClothingDecals`,
// `HairStyles`, `BeardStyles`, `PopTemplateManager` skin lists, and the
// `HumanStyles` over them (`Population::styles`).
pub use pz_characters::population::{
    BeardStyle, BeardStyles, ClothingDecal, ClothingDecals, HairStyle, HairStyleAlternate,
    HairStyles, PopTemplateManager, Population, PopulationStyles,
};
