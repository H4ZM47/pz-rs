//! `StaggerBackState` (shared by players and zombies).

use pz_anim::AnimEventRecord;

use crate::character::state::{State, StateCx, StateId, StateRef, SyncFlags};
use crate::character::GameCharacter;

pub const ID: StateId = super::ids::STAGGER_BACK;

pub struct StaggerBackState;

static INSTANCE: StaggerBackState = StaggerBackState;

/// `StaggerBackState.instance()`.
pub fn instance<I: 'static>() -> StateRef<I> {
    &INSTANCE
}

/// `getMaxStaggerTime(owner)`: `35 * hitForce * staggerTimeMod` in
/// `[20, 30]`.
pub fn max_stagger_time<I: 'static>(owner: &GameCharacter<I>) -> f32 {
    (35.0 * owner.vars.hit_force * owner.vars.stagger_time_mod).clamp(20.0, 30.0)
}

impl<I: 'static> State<I> for StaggerBackState {
    fn id(&self) -> StateId {
        ID
    }
    fn sync_flags(&self) -> SyncFlags {
        SyncFlags::new(false, false, false, false)
    }
    fn enter(&self, owner: &mut GameCharacter<I>, _cx: &mut StateCx<'_, I>) {
        owner.vars.state_event_delay_timer = max_stagger_time(owner);
    }
    /// `execute`: the animation player's target angle (TODO(anim)).
    fn execute(&self, _owner: &mut GameCharacter<I>, _cx: &mut StateCx<'_, I>) {}
    fn exit(&self, owner: &mut GameCharacter<I>, _cx: &mut StateCx<'_, I>) {
        if owner.is_zombie() {
            owner.vars.stagger_back = false;
        }
        owner.vars.shootable = true;
        owner.vars.deferred_movement_enabled = true;
    }
    fn anim_event(&self, owner: &mut GameCharacter<I>, _cx: &mut StateCx<'_, I>, event: &AnimEventRecord) {
        let e = &event.event;
        if e.event_name.eq_ignore_ascii_case("FallOnFront") {
            owner.environment.fall_on_front = parse_boolean(e.parameter_value.as_deref());
        }
        // SetState: the zombie's sound parameter (TODO(sounds)).
    }
}

/// `Boolean.parseBoolean`.
pub fn parse_boolean(s: Option<&str>) -> bool {
    s.is_some_and(|s| s.eq_ignore_ascii_case("true"))
}
