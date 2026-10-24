//! `zombie.ai.StateMachine` and `StateMachineComponent`, with the
//! `IsoGameCharacter` methods built on them (`changeState`,
//! `getCurrentState`, `isCurrentState`, `setDefaultState`, the
//! action-state-changed callback, `getStateMachineParams`).

use std::collections::HashMap;

use pz_anim::AnimEventRecord;

use super::action::{ActionContext, ActionGroup};
use super::{same_state, StateCx, StateId, StateParams, StateRef};
use crate::character::{GameCharacter, SimEvent};

/// `StateMachine.SubstateSlot`.
pub struct SubstateSlot<I: 'static> {
    pub state: Option<StateRef<I>>,
    pub should_be_active: bool,
}

impl<I: 'static> SubstateSlot<I> {
    pub fn is_empty(&self) -> bool {
        self.state.is_none()
    }
}

/// `StateMachine`.
pub struct StateMachine<I: 'static> {
    /// `isLocked`.
    pub locked: bool,
    /// `activeStateChanged`.
    pub active_state_changed: i32,
    current: Option<StateRef<I>>,
    previous: Option<StateRef<I>>,
    sub_states: Vec<SubstateSlot<I>>,
}

impl<I: 'static> Default for StateMachine<I> {
    fn default() -> Self {
        StateMachine { locked: false, active_state_changed: 0, current: None, previous: None, sub_states: Vec::new() }
    }
}

impl<I: 'static> StateMachine<I> {
    /// `getCurrent()`.
    pub fn current(&self) -> Option<StateRef<I>> {
        self.current
    }
    /// `getPrevious()`.
    pub fn previous(&self) -> Option<StateRef<I>> {
        self.previous
    }
    /// `getSubStateCount()`.
    pub fn sub_state_count(&self) -> usize {
        self.sub_states.len()
    }
    /// `getSubStateAt(idx)`.
    pub fn sub_state_at(&self, idx: usize) -> Option<StateRef<I>> {
        self.sub_states.get(idx).and_then(|s| s.state)
    }
    /// `isSubstate(state)`.
    pub fn is_substate(&self, s: StateRef<I>) -> bool {
        self.sub_states.iter().any(|slot| same_state(slot.state, Some(s)))
    }
    fn is_substate_id(&self, id: StateId) -> bool {
        self.sub_states.iter().any(|slot| slot.state.is_some_and(|s| s.id() == id))
    }
    fn existing_slot(&self, s: StateRef<I>) -> Option<usize> {
        self.sub_states.iter().position(|slot| same_state(slot.state, Some(s)))
    }
}

/// The original's `AdvancedAnimator.setState` call when the action states
/// change; installed by the owner of the character's animation.
pub type AnimStateHook<I> = fn(&mut GameCharacter<I>, &str, &[String]);

/// `owner.actionStateChanged(context)` (`IsoZombie` overrides it).
pub type ActionStateChangedHook<I> = fn(&mut GameCharacter<I>, &mut StateCx<'_, I>);

/// `StateMachineComponent`.
pub struct StateMachineComponent<I: 'static> {
    pub machine: StateMachine<I>,
    /// `defaultState` (`GenericDefaultState` from the constructor).
    pub default_state: Option<StateRef<I>>,
    /// `aiStateMap` (lowercase action state name to AI state).
    pub ai_state_map: HashMap<String, StateRef<I>>,
    /// `stateMachineParams`.
    pub params: StateParams,
    /// `actionContext`.
    pub action_context: ActionContext,
    /// `advancedAnimator.setState(name, subStateNames)`.
    pub anim_state_hook: Option<AnimStateHook<I>>,
    /// The subclass's `actionStateChanged` override.
    pub action_state_changed_hook: Option<ActionStateChangedHook<I>>,
}

impl<I: 'static> Default for StateMachineComponent<I> {
    fn default() -> Self {
        StateMachineComponent {
            machine: StateMachine::default(),
            default_state: Some(crate::character::states::generic_default::instance()),
            ai_state_map: HashMap::new(),
            params: StateParams::default(),
            action_context: ActionContext::new(),
            anim_state_hook: None,
            action_state_changed_hook: None,
        }
    }
}

impl<I: 'static> GameCharacter<I> {
    /// `getStateMachine()`.
    pub fn state_machine(&self) -> &StateMachine<I> {
        &self.state.machine
    }

    /// `changeState(state, subStates, restart)`.
    pub fn change_state_with(
        &mut self,
        new_state: Option<StateRef<I>>,
        sub_states: &[Option<StateRef<I>>],
        restart: bool,
        cx: &mut StateCx<'_, I>,
    ) {
        if self.state.machine.locked {
            // DebugType.Action.warn("StateMachine is locked...")
            return;
        }
        self.change_root_state(new_state, restart, cx);
        for slot in &mut self.state.machine.sub_states {
            slot.should_be_active = false;
        }
        for s in sub_states.iter().flatten() {
            self.ensure_substate_active(*s, cx);
        }
        let mut i = 0;
        while i < self.state.machine.sub_states.len() {
            let slot = &self.state.machine.sub_states[i];
            if !slot.should_be_active && !slot.is_empty() {
                self.remove_substate_at(i, cx);
            }
            i += 1;
        }
    }

    /// `IsoGameCharacter.changeState(state)`.
    pub fn change_state(&mut self, state: Option<StateRef<I>>, cx: &mut StateCx<'_, I>) {
        self.change_state_with(state, &[], false, cx);
    }

    fn change_root_state(&mut self, new_state: Option<StateRef<I>>, restart: bool, cx: &mut StateCx<'_, I>) {
        let cur = self.state.machine.current;
        if same_state(cur, new_state) {
            if restart {
                if let Some(c) = cur {
                    self.state_enter(c, cx);
                }
            }
            return;
        }
        if let Some(p) = cur {
            self.state_exit(p, cx);
        }
        self.state.machine.previous = cur;
        self.state.machine.current = new_state;
        if let Some(n) = new_state {
            self.state_enter(n, cx);
        }
        cx.ctx.event(SimEvent::AiStateChange {
            current: self.state.machine.current.map(|s| s.id()),
            previous: self.state.machine.previous.map(|s| s.id()),
        });
    }

    fn ensure_substate_active(&mut self, s: StateRef<I>, cx: &mut StateCx<'_, I>) {
        if same_state(Some(s), self.state.machine.current) {
            return;
        }
        if let Some(i) = self.state.machine.existing_slot(s) {
            self.state.machine.sub_states[i].should_be_active = true;
            return;
        }
        let m = &mut self.state.machine;
        match m.sub_states.iter_mut().find(|slot| slot.is_empty()) {
            Some(slot) => {
                slot.state = Some(s);
                slot.should_be_active = true;
            }
            None => m.sub_states.push(SubstateSlot { state: Some(s), should_be_active: true }),
        }
        self.state_enter(s, cx);
    }

    fn remove_substate_at(&mut self, i: usize, cx: &mut StateCx<'_, I>) {
        let Some(s) = self.state.machine.sub_states[i].state.take() else { return };
        self.state_exit(s, cx);
    }

    /// `revertToPreviousState(sender)`.
    pub fn revert_to_previous_state(&mut self, sender: StateRef<I>, cx: &mut StateCx<'_, I>) {
        if let Some(i) = self.state.machine.existing_slot(sender) {
            self.remove_substate_at(i, cx);
            return;
        }
        if !same_state(self.state.machine.current, Some(sender)) {
            // DebugType.ActionSystem.warn("The sender ... is not an active state")
            return;
        }
        let prev = self.state.machine.previous;
        self.change_root_state(prev, false, cx);
    }

    /// `StateMachine.update()`: the current state's `execute`, then each
    /// sub state's (slots added meanwhile included).
    pub fn state_machine_update(&mut self, cx: &mut StateCx<'_, I>) {
        if let Some(c) = self.state.machine.current {
            c.execute(self, cx);
        }
        let mut i = 0;
        while i < self.state.machine.sub_states.len() {
            if let Some(s) = self.state.machine.sub_states[i].state {
                s.execute(self, cx);
            }
            i += 1;
        }
    }

    fn state_enter(&mut self, s: StateRef<I>, cx: &mut StateCx<'_, I>) {
        s.enter(self, cx);
    }

    fn state_exit(&mut self, s: StateRef<I>, cx: &mut StateCx<'_, I>) {
        s.exit(self, cx);
    }

    /// `StateMachine.stateAnimEvent(stateLayer, layer, track, event)`.
    pub fn state_anim_event(&mut self, state_layer: i32, event: &AnimEventRecord, cx: &mut StateCx<'_, I>) {
        if state_layer <= 0 {
            if let Some(c) = self.state.machine.current {
                c.anim_event(self, cx, event);
            }
            if state_layer == 0 {
                return;
            }
        }
        let mut i = 0;
        while i < self.state.machine.sub_states.len() {
            if let Some(s) = self.state.machine.sub_states[i].state {
                s.anim_event(self, cx, event);
            }
            i += 1;
        }
    }

    /// `getCurrentState()`.
    pub fn current_state(&self) -> Option<StateRef<I>> {
        self.state.machine.current
    }

    /// The current state's id.
    pub fn current_state_id(&self) -> Option<StateId> {
        self.state.machine.current.map(|s| s.id())
    }

    /// `isCurrentState(state)`: the root or a sub state.
    pub fn is_current_state(&self, s: StateRef<I>) -> bool {
        self.is_current_state_id(s.id())
    }

    /// `isCurrentState` by class.
    pub fn is_current_state_id(&self, id: StateId) -> bool {
        self.state.machine.is_substate_id(id) || self.current_state_id() == Some(id)
    }

    /// `getCurrentStateName()`.
    pub fn current_state_name(&self) -> Option<&'static str> {
        self.state.machine.current.map(|s| s.name())
    }

    /// `getPreviousStateName()`.
    pub fn previous_state_name(&self) -> Option<&'static str> {
        self.state.machine.previous.map(|s| s.name())
    }

    /// `getDefaultState()`.
    pub fn default_state(&self) -> Option<StateRef<I>> {
        self.state.default_state
    }

    /// `setDefaultState(state)`.
    pub fn set_default_state_to(&mut self, s: Option<StateRef<I>>) {
        self.state.default_state = s;
    }

    /// `setDefaultState()`: `changeState(defaultState, null, false)`.
    pub fn set_default_state(&mut self, cx: &mut StateCx<'_, I>) {
        let d = self.state.default_state;
        self.change_state_with(d, &[], false, cx);
    }

    /// `setStateMachineLocked(locked)`.
    pub fn set_state_machine_locked(&mut self, locked: bool) {
        self.state.machine.locked = locked;
    }

    /// `registerAIState(name, state)`.
    pub fn register_ai_state(&mut self, name: &str, s: StateRef<I>) {
        self.state.ai_state_map.insert(name.to_lowercase(), s);
    }

    /// `clearAIStateMap()`.
    pub fn clear_ai_state_map(&mut self) {
        self.state.ai_state_map.clear();
    }

    /// `tryGetAIState(name)`.
    pub fn try_get_ai_state(&self, name: &str) -> Option<StateRef<I>> {
        self.state.ai_state_map.get(&name.to_lowercase()).copied()
    }

    /// `IsoGameCharacter.clear(stateClass)`.
    pub fn clear_state_params(&mut self, id: StateId) {
        self.state.params.clear(id);
    }

    /// `getActionContext().setGroup(ActionGroup.getActionGroup(name))`.
    pub fn set_action_group(&mut self, group: std::rc::Rc<ActionGroup>) {
        self.state.action_context.set_group(group);
    }

    /// `reportEvent(name)`: `actionContext.reportEvent(null, name)`.
    pub fn report_event(&mut self, name: &str) {
        self.state.action_context.report_event(None, name);
    }

    /// `getActionStateName()`.
    pub fn action_state_name(&self) -> String {
        self.state.action_context.current_state_name()
    }

    /// `getCurrentActionContextStateName()`: "" without a root.
    pub fn current_action_context_state_name(&self) -> String {
        if self.state.action_context.current_state().is_none() {
            return String::new();
        }
        self.state.action_context.current_state_name()
    }

    /// `getPreviousActionContextStateName()`.
    pub fn previous_action_context_state_name(&self) -> String {
        self.state.action_context.peek_previous_state_name()
    }

    /// `ActionContext.update()` from `postUpdateAnimating`: evaluate the
    /// transitions against this character's variables, then (when the
    /// action states changed) `StateMachineComponent.actionStateChanged`
    /// and the subclass hook.
    pub fn update_action_context(&mut self, cx: &mut StateCx<'_, I>) {
        let next = self.state.action_context.evaluate_next(&*self);
        self.state.action_context.transfer_action_state(next);
        if self.state.action_context.post_update() {
            self.action_state_changed(cx);
            if let Some(h) = self.state.action_state_changed_hook {
                h(self, cx);
            }
        }
    }

    /// `StateMachineComponent.actionStateChanged(sender)`: the animator's
    /// state, then `changeState` to the registered AI states (the default
    /// state for an unregistered root; unregistered children are skipped).
    pub fn action_state_changed(&mut self, cx: &mut StateCx<'_, I>) {
        let ctx = &self.state.action_context;
        let root = ctx.current_state_name();
        let children: Vec<String> = ctx.child_states().iter().map(|c| c.name().to_string()).collect();
        if let Some(h) = self.state.anim_state_hook {
            h(self, &root, &children);
        }
        self.state.machine.active_state_changed += 1;
        let ai = self.try_get_ai_state(&root).or(self.state.default_state);
        let subs: Vec<Option<StateRef<I>>> = children.iter().map(|c| self.try_get_ai_state(c)).collect();
        self.change_state_with(ai, &subs, false, cx);
        self.state.machine.active_state_changed -= 1;
    }
}
