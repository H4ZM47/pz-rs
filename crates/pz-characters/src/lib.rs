//! Character data the main menu and character creation use:
//! `zombie.characters.traits` (`CharacterTraits`, `ObservationFactory`),
//! `zombie.characters.SurvivorFactory` and `SurvivorDesc`, with the parts of
//! `HumanVisual` and `zombie.characters.WornItems` a descriptor holds, and
//! `zombie.characters.skills.PerkFactory`.
//!
//! `CharacterTraitDefinition` and `CharacterProfessionDefinition` live in
//! `pz-scripts` (their scripts fill them); the trait and profession keys are
//! that crate's registries.
//!
//! The original's static fields are kept in [`Characters`].

pub mod observation;
pub mod skills;
pub mod survivor;
pub mod traits;
pub mod visual;
pub mod worn_items;

pub use observation::{Observation, ObservationFactory, ObservationRef};
pub use skills::{Perk, PerkFactory, PerkRef};
pub use survivor::{CharacterGender, SurvivorDesc, SurvivorDescRef, SurvivorDescStatics, SurvivorFactory, SurvivorType};
pub use traits::CharacterTraits;
pub use visual::{Color, HumanVisual, ImmutableColor};
pub use worn_items::{BodyLocation, BodyLocationGroup, BodyLocationGroupRef, BodyLocationRef, BodyLocations, ItemIdentity, WornItem, WornItems};

use pz_scripts::ScriptError;

/// The static state of the classes in this crate.
#[derive(Default)]
pub struct Characters {
    /// `SurvivorFactory`'s name lists.
    pub survivor_factory: SurvivorFactory,
    /// `ObservationFactory.observationMap`.
    pub observations: ObservationFactory,
    /// `SurvivorDesc`'s statics.
    pub survivor_desc: SurvivorDescStatics,
    /// `BodyLocations.groups`.
    pub body_locations: BodyLocations,
    /// `PerkFactory`'s statics and the `PerkFactory.Perks` constants.
    pub perks: PerkFactory,
}

pub(crate) fn npe() -> ScriptError {
    ScriptError::new("java.lang.NullPointerException")
}

pub(crate) fn index_out_of_bounds(index: i64, len: usize) -> ScriptError {
    ScriptError::new(format!(
        "java.lang.IndexOutOfBoundsException: Index {index} out of bounds for length {len}"
    ))
}
