//! The character state machine: `zombie.ai.State`, `zombie.ai.StateMachine`,
//! `State.Param`, `zombie.characters.component.StateMachineComponent` and
//! the action system that drives it (`zombie.characters.action`), with the
//! character's animation variables (`IAnimationVariableSource`) and the
//! routing of animation events to the states.
//!
//! # How the pieces fit
//!
//! * A state is a `'static` singleton implementing [`State`] (the
//!   original's `State` subclasses with their static `instance()`). States
//!   are compared by [`StateId`] (the Java class name), which is what the
//!   original's `==` on the singletons amounts to.
//! * [`StateMachine`] holds the current, previous and sub states; its
//!   operations are methods of [`GameCharacter`] (`change_state`,
//!   `state_machine_update`, ...) because a state's `enter`/`execute`/`exit`
//!   receive the owner mutably.
//! * Per-character state data lives in [`StateParams`], keyed by
//!   [`Param`]s (`State.Param` and `IsoGameCharacter.getStateMachineParams`).
//! * [`action::ActionContext`] evaluates the `media/actiongroups` transitions
//!   against the character's variables and the reported animation events and,
//!   when the action states change, maps them through the registered AI
//!   states (`registerAIState`) to `changeState` with sub states
//!   ([`GameCharacter::update_action_context`]).
//! * The character's variables ([`vars`]) are the original's game-variable
//!   map plus the callback slots that read and write character fields;
//!   [`GameCharacter`] implements `pz_anim`'s [`AnimationVariableSource`] over
//!   them, with the action states' own variables in front.
//! * Fired animation events go through
//!   [`GameCharacter::on_anim_event`] (`IsoGameCharacter.OnAnimEvent`): the
//!   character's own listeners, `ActionContext.reportEvent`, then
//!   `StateMachine.stateAnimEvent`.
//!
//! Everything a state reads from outside the character goes through
//! [`StateCx`]: the script/character statics, the [`SimContext`] singletons
//! and the [`StateWorld`] (other characters, squares, path finding).
//!
//! [`AnimationVariableSource`]: pz_anim::AnimationVariableSource

pub mod action;
pub mod cx;
pub mod events;
pub mod machine;
pub mod params;
pub mod vars;

pub use cx::{
    BehaviorResult, CorpseInfo, EatTarget, NullWorld, SquareInfo, StateCx, StateWorld, TestResults, ThumpInfo,
    ThumpKind, ThumpTarget,
};
pub use action::ActionGroups;
pub use machine::{StateMachine, StateMachineComponent, SubstateSlot};
pub use params::{Param, StateParams};
pub use vars::{CharVar, CharacterVars};

use pz_anim::AnimEventRecord;

use super::GameCharacter;
#[allow(unused_imports)]
use super::SimContext;

/// A state's identity: the Java class name (`zombie.ai.states.IdleState`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct StateId(pub &'static str);

impl StateId {
    /// `getClass().getSimpleName()`.
    pub fn simple_name(self) -> &'static str {
        match self.0.rfind(['.', '$']) {
            Some(i) => &self.0[i + 1..],
            None => self.0,
        }
    }
}

/// `State.Stage`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stage {
    Enter,
    Execute,
    Exit,
}

/// `IStateFlagsSource`'s network flags, fixed per state by its constructor
/// (`State(isSyncOnEnter, isSyncOnExit, isSyncOnSquare, isSyncInIdle)`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SyncFlags {
    pub on_enter: bool,
    pub on_exit: bool,
    pub on_square: bool,
    pub in_idle: bool,
}

impl SyncFlags {
    pub const fn new(on_enter: bool, on_exit: bool, on_square: bool, in_idle: bool) -> Self {
        SyncFlags { on_enter, on_exit, on_square, in_idle }
    }
}

/// `MoveDeltaModifiers`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MoveDeltaModifiers {
    pub turn_delta: f32,
    pub move_delta: f32,
    pub twist_delta: f32,
}

impl Default for MoveDeltaModifiers {
    fn default() -> Self {
        MoveDeltaModifiers { turn_delta: -1.0, move_delta: -1.0, twist_delta: -1.0 }
    }
}

impl MoveDeltaModifiers {
    /// `setMaxTurnDelta`.
    pub fn set_max_turn_delta(&mut self, d: f32) {
        self.turn_delta = pz_max(self.turn_delta, d);
    }
    /// `setMaxMoveDelta`.
    pub fn set_max_move_delta(&mut self, d: f32) {
        self.move_delta = pz_max(self.move_delta, d);
    }
    /// `setMaxTwistDelta`.
    pub fn set_max_twist_delta(&mut self, d: f32) {
        self.twist_delta = pz_max(self.twist_delta, d);
    }
}

/// `PZMath.max(float, float)`.
fn pz_max(a: f32, b: f32) -> f32 {
    if a > b {
        a
    } else {
        b
    }
}

/// `zombie.ai.State` (with `IStateFlagsSource`'s queries). Every method
/// has the base class's behaviour; a state overrides what its Java class
/// overrides. `animEvent` is the state's `AnimEventBroadcaster` dispatch:
/// states match on the event name instead of registering listeners.
pub trait State<I: 'static>: 'static {
    /// The class.
    fn id(&self) -> StateId;

    /// `getName()`: the simple class name.
    fn name(&self) -> &'static str {
        self.id().simple_name()
    }

    /// The constructor's network flags.
    fn sync_flags(&self) -> SyncFlags {
        SyncFlags::default()
    }

    /// `enter(owner)`.
    fn enter(&self, _owner: &mut GameCharacter<I>, _cx: &mut StateCx<'_, I>) {}

    /// `execute(owner)`.
    fn execute(&self, _owner: &mut GameCharacter<I>, _cx: &mut StateCx<'_, I>) {}

    /// `exit(owner)`.
    fn exit(&self, _owner: &mut GameCharacter<I>, _cx: &mut StateCx<'_, I>) {}

    /// `animEvent(owner, layer, track, event)`.
    fn anim_event(&self, _owner: &mut GameCharacter<I>, _cx: &mut StateCx<'_, I>, _event: &AnimEventRecord) {}

    /// `getDeltaModifiers(owner, modifiers)`.
    fn get_delta_modifiers(&self, _owner: &GameCharacter<I>, _modifiers: &mut MoveDeltaModifiers) {}

    /// `isIgnoreCollide(owner, fromX, fromY, fromZ, toX, toY, toZ)`.
    fn is_ignore_collide(&self, _owner: &GameCharacter<I>, _from: (i32, i32, i32), _to: (i32, i32, i32)) -> bool {
        false
    }

    /// `isAttacking(owner)`.
    fn is_attacking(&self, _owner: &GameCharacter<I>) -> bool {
        false
    }

    /// `isMoving(owner)`.
    fn is_moving(&self, _owner: &GameCharacter<I>) -> bool {
        false
    }

    /// `isDoingActionThatCanBeCancelled()`.
    fn is_doing_action_that_can_be_cancelled(&self) -> bool {
        false
    }

    /// `canBeHitByVehicle(owner, vehicle)`.
    fn can_be_hit_by_vehicle(&self, _owner: &GameCharacter<I>) -> bool {
        true
    }

    /// `causesDamageToVehicleWhenHit(owner, vehicle)`.
    fn causes_damage_to_vehicle_when_hit(&self, _owner: &GameCharacter<I>) -> bool {
        true
    }

    /// `canSlowDownVehicleWhenHit(owner, vehicle)`.
    fn can_slow_down_vehicle_when_hit(&self, _owner: &GameCharacter<I>) -> bool {
        true
    }

    /// `canRagdoll(owner)`.
    fn can_ragdoll(&self, _owner: &GameCharacter<I>) -> bool {
        true
    }

    /// `awayCheckDistance(owner)`.
    fn away_check_distance(&self, _owner: &GameCharacter<I>) -> f32 {
        0.2
    }

    /// `isProcessedOnEnter()` (network replication).
    fn is_processed_on_enter(&self) -> bool {
        false
    }

    /// `isProcessedOnExit()` (network replication).
    fn is_processed_on_exit(&self) -> bool {
        false
    }
}

/// A state singleton.
pub type StateRef<I> = &'static dyn State<I>;

/// Identity comparison of two optional states (`==` on the singletons).
pub fn same_state<I: 'static>(a: Option<StateRef<I>>, b: Option<StateRef<I>>) -> bool {
    match (a, b) {
        (Some(a), Some(b)) => a.id() == b.id(),
        (None, None) => true,
        _ => false,
    }
}

/// A state whose behaviour is the base class's: the stand-in for a state
/// class whose behaviour is not ported, so that it can still be registered,
/// entered and compared (`OpaqueState::leak("zombie.ai.states.FishingState")`).
pub struct OpaqueState {
    id: StateId,
    flags: SyncFlags,
}

impl OpaqueState {
    pub const fn new(class: &'static str, flags: SyncFlags) -> Self {
        OpaqueState { id: StateId(class), flags }
    }
}

impl<I: 'static> State<I> for OpaqueState {
    fn id(&self) -> StateId {
        self.id
    }
    fn sync_flags(&self) -> SyncFlags {
        self.flags
    }
}

/// A lookup of the state singletons by class name (the jar's state
/// classes): ported states first, then stand-ins.
pub struct StateRegistry<I: 'static> {
    states: std::collections::HashMap<&'static str, StateRef<I>>,
}

impl<I: 'static> Default for StateRegistry<I> {
    fn default() -> Self {
        StateRegistry { states: std::collections::HashMap::new() }
    }
}

impl<I: 'static> StateRegistry<I> {
    /// Every ported state.
    pub fn with_ported() -> Self {
        let mut r = Self::default();
        for s in crate::character::states::all::<I>() {
            r.add(s);
        }
        r
    }

    pub fn add(&mut self, s: StateRef<I>) {
        self.states.insert(s.id().0, s);
    }

    /// The singleton of `class` (`a.b.C` or `a/b/C`); classes without a
    /// ported state get a leaked [`OpaqueState`] once.
    pub fn get_or_opaque(&mut self, class: &str) -> StateRef<I> {
        let class = class.replace('/', ".");
        if let Some(s) = self.states.get(class.as_str()) {
            return *s;
        }
        let name: &'static str = Box::leak(class.into_boxed_str());
        let s: StateRef<I> = Box::leak(Box::new(OpaqueState::new(name, SyncFlags::default())));
        self.states.insert(name, s);
        s
    }

    pub fn get(&self, class: &str) -> Option<StateRef<I>> {
        self.states.get(class).copied()
    }
}

#[cfg(test)]
mod tests;
