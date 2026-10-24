//! `IdleState`.

use pz_anim::AnimEventRecord;

use crate::character::state::{State, StateCx, StateId, StateRef, SyncFlags};
use crate::character::{GameCharacter, SimEvent};

pub const ID: StateId = super::ids::IDLE;

pub struct IdleState;

static INSTANCE: IdleState = IdleState;

/// `IdleState.instance()`.
pub fn instance<I: 'static>() -> StateRef<I> {
    &INSTANCE
}

impl<I: 'static> State<I> for IdleState {
    fn id(&self) -> StateId {
        ID
    }
    fn sync_flags(&self) -> SyncFlags {
        SyncFlags::new(true, false, false, false)
    }

    /// `PlaySound` plays on the square; `PlayerVoiceSound` only under the
    /// `dbgForceAnim` variable.
    fn anim_event(&self, owner: &mut GameCharacter<I>, cx: &mut StateCx<'_, I>, event: &AnimEventRecord) {
        let e = &event.event;
        let param = e.parameter_value.as_deref().unwrap_or("");
        if e.event_name.eq_ignore_ascii_case("PlaySound") && !param.is_empty() {
            cx.ctx.event(SimEvent::PlaySound(param.to_string()));
        }
        if e.event_name.eq_ignore_ascii_case("PlayerVoiceSound") && owner.is_player() && owner.variable_boolean("dbgForceAnim") {
            cx.ctx.event(SimEvent::PlayerVoiceSoundNamed(param.to_string()));
        }
    }

    /// `setParams(Enter)` clears `isAiming`; then off the floor.
    fn enter(&self, owner: &mut GameCharacter<I>, _cx: &mut StateCx<'_, I>) {
        owner.vars.is_aiming = false;
        owner.vars.on_floor = false;
    }
}
