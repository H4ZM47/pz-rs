//! The twelve `zombie.ai.states.animals` states, as stand-ins until step 3
//! of the animal port ports their bodies (`enter`/`execute`/`exit`/
//! `animEvent` and `BaseAnimalBehavior`). Each stand-in has the state's
//! class id and sync flags (all four false, as every animal state's
//! constructor passes) and does nothing, so an animal's state machine can
//! register, enter and leave them like the original's.

use crate::character::state::{OpaqueState, StateRef, SyncFlags};

use super::ids;

const NONE: SyncFlags = SyncFlags::new(false, false, false, false);

static STATES: [OpaqueState; 12] = [
    OpaqueState::new(ids::ANIMAL_ALERTED.0, NONE),
    OpaqueState::new(ids::ANIMAL_ATTACK.0, NONE),
    OpaqueState::new(ids::ANIMAL_CLIMB_OVER_FENCE.0, NONE),
    OpaqueState::new(ids::ANIMAL_EAT.0, NONE),
    OpaqueState::new(ids::ANIMAL_FALLDOWN.0, NONE),
    OpaqueState::new(ids::ANIMAL_FOLLOW_WALL.0, NONE),
    OpaqueState::new(ids::ANIMAL_HIT_REACTION.0, NONE),
    OpaqueState::new(ids::ANIMAL_IDLE.0, NONE),
    OpaqueState::new(ids::ANIMAL_ON_GROUND.0, NONE),
    OpaqueState::new(ids::ANIMAL_PATH_FIND.0, NONE),
    OpaqueState::new(ids::ANIMAL_WALK.0, NONE),
    OpaqueState::new(ids::ANIMAL_ZONE.0, NONE),
];

/// The stand-ins, for the registry.
pub fn all<I: 'static>() -> Vec<StateRef<I>> {
    STATES.iter().map(|s| s as StateRef<I>).collect()
}
