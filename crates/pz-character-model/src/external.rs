//! The seam to types other workstreams own. Each owner's type comes in
//! through exactly one `pub use` here; the rest of the crate names only
//! these paths, so swapping a local stand-in for the owner's port is a
//! one-line change.

// owner: pz-inventory (items thread). `HumanVisual` and the style/owner
// traits its lazy getters use.
pub use pz_inventory::visual::human::{HumanStyles, HumanVisual, HumanVisualOwner, SkinList};

// owner: pz-characters (characters thread); swap when ported
pub use crate::local::ClothingDecal;

// owner: pz-characters (characters thread); swap when ported
pub use crate::local::{HairStyle, HairStyles};

// owner: pz-characters (characters thread); swap when ported
// (`PopTemplateManager` skin lists, and the `HumanStyles` over them and the
// hair/beard styles that `HumanVisual.getSkinTexture`/`getHairModel` use)
pub use crate::local::{CharacterStyles, PopTemplateSkins};
