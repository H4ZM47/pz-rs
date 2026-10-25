//! `zombie.ai.states.player.PlayerMovementState` (the `movement`, `run`
//! and `sprint` action states): a local owner stores whether it runs or
//! sprints in the state's params on enter and exit.

use super::ids;
use crate::character::state::{Param, State, StateCx, StateId, StateRef, SyncFlags};
use crate::character::GameCharacter;

pub const ID: StateId = ids::PLAYER_MOVEMENT;

/// `PlayerMovementState.RUN`, `SPRINT`.
pub const RUN: Param<bool> = Param::new(ID, "run", || false);
pub const SPRINT: Param<bool> = Param::new(ID, "sprint", || false);

pub struct PlayerMovementState;

static INSTANCE: PlayerMovementState = PlayerMovementState;

/// `PlayerMovementState.instance()`.
pub fn instance<I: 'static>() -> StateRef<I> {
    &INSTANCE
}

/// `setParams(owner, stage)` for a local owner.
fn set_params<I: 'static>(owner: &mut GameCharacter<I>) {
    let (run, sprint) = (owner.running, owner.environment.sprinting);
    RUN.set(&mut owner.state.params, run);
    SPRINT.set(&mut owner.state.params, sprint);
}

impl<I: 'static> State<I> for PlayerMovementState {
    fn id(&self) -> StateId {
        ID
    }
    fn sync_flags(&self) -> SyncFlags {
        SyncFlags::new(true, true, false, false)
    }
    fn enter(&self, owner: &mut GameCharacter<I>, _cx: &mut StateCx<'_, I>) {
        set_params(owner);
    }
    fn exit(&self, owner: &mut GameCharacter<I>, _cx: &mut StateCx<'_, I>) {
        set_params(owner);
    }
}
