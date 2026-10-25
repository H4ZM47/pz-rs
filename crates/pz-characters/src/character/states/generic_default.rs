//! `GenericDefaultState`: the base class's behaviour.

use crate::character::state::{State, StateId, StateRef, SyncFlags};

pub const ID: StateId = super::ids::GENERIC_DEFAULT;

pub struct GenericDefaultState;

static INSTANCE: GenericDefaultState = GenericDefaultState;

/// `GenericDefaultState.instance()`.
pub fn instance<I: 'static>() -> StateRef<I> {
    &INSTANCE
}

impl<I: 'static> State<I> for GenericDefaultState {
    fn id(&self) -> StateId {
        ID
    }
    fn sync_flags(&self) -> SyncFlags {
        SyncFlags::new(false, false, false, false)
    }
}
