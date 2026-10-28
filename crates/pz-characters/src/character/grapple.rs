//! The stable owner behind a character's BaseGrappleable queries. Pending
//! corpse conversion, gameplay targeting and callback-bearing grapple methods
//! are deliberately separate from these pure owner operations.

use super::{CharacterKind, GameCharacter};
use pz_kahlua::gc::{traceable, TracePayload, TraceVisitor};
use std::any::Any;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

/// Value-like zombie state with an explicit shared view for the actual
/// grapple endpoint. Cloning ZombieFields must copy its value, not alias it.
#[derive(Debug, Default)]
pub(crate) struct ZombieFlag(Rc<Cell<bool>>);
impl Clone for ZombieFlag {
    fn clone(&self) -> Self {
        Self(Rc::new(Cell::new(self.get())))
    }
}
impl PartialEq for ZombieFlag {
    fn eq(&self, other: &Self) -> bool {
        self.get() == other.get()
    }
}
impl ZombieFlag {
    pub(crate) fn get(&self) -> bool {
        self.0.get()
    }
    pub(crate) fn set(&self, value: bool) {
        self.0.set(value);
    }
    pub(super) fn shared(&self) -> Rc<Cell<bool>> {
        self.0.clone()
    }
}

type GrappleRef = Rc<RefCell<GrappleState>>;

pub(super) struct GrappleState {
    kind: CharacterKind,
    zombie_reanimated: Rc<Cell<bool>>,
    do_grapple: bool,
    picking_up: bool,
    putting_down: bool,
    continuing: bool,
    being_grappled: bool,
    grappling: bool,
    /// Original reset retains this strong slot; the role masks its getter.
    grappled_by: Option<GrappleRef>,
    target: Option<GrappleRef>,
    shared_type: Option<String>,
    anim_node: String,
    time: f32,
    fraction: f32,
    result: Option<String>,
    offset_forward: f32,
    offset_behaviour: String,
    deferred_offset: (f32, f32, f32),
}

impl TracePayload for GrappleState {
    fn trace(&self, visitor: &mut TraceVisitor) {
        if let Some(target) = &self.target {
            visitor.owner(target);
        }
        // Count the raw strong reference even while its getter is masked.
        if let Some(grappler) = &self.grappled_by {
            visitor.owner(grappler);
        }
    }
    fn detach(&mut self) -> Option<Box<dyn Any>> {
        Some(Box::new((self.target.take(), self.grappled_by.take())))
    }
}

impl GrappleState {
    pub(super) fn new(kind: CharacterKind, zombie_reanimated: Rc<Cell<bool>>) -> GrappleRef {
        traceable(Self {
            kind,
            zombie_reanimated,
            do_grapple: false,
            picking_up: false,
            putting_down: false,
            continuing: false,
            being_grappled: false,
            grappling: false,
            grappled_by: None,
            target: None,
            shared_type: Some(String::new()),
            anim_node: String::new(),
            time: 0.0,
            fraction: 0.0,
            result: Some(String::new()),
            offset_forward: 0.0,
            offset_behaviour: "NONE".into(),
            deferred_offset: (0.0, 0.0, 0.0),
        })
    }

    fn reset(&mut self, result: Option<String>) -> Option<GrappleRef> {
        self.do_grapple = false;
        self.picking_up = false;
        self.putting_down = false;
        self.continuing = false;
        self.grappling = false;
        self.being_grappled = false;
        let old = self.target.take();
        self.result = result;
        self.shared_type = Some(String::new());
        self.anim_node.clear();
        self.time = 0.0;
        self.fraction = 0.0;
        self.offset_forward = 0.0;
        self.offset_behaviour = "NONE".into();
        self.deferred_offset = (0.0, 0.0, 0.0);
        old
    }

    fn set_type(owner: &GrappleRef, value: Option<&str>) {
        let (target, grappler) = {
            let mut state = owner.borrow_mut();
            if state.shared_type.as_deref() == value {
                return;
            }
            state.shared_type = value.map(str::to_string);
            (
                state.grappling.then(|| state.target.clone()).flatten(),
                state
                    .being_grappled
                    .then(|| state.grappled_by.clone())
                    .flatten(),
            )
        };
        if let Some(target) = target {
            Self::set_type(&target, value);
        }
        if let Some(grappler) = grappler {
            Self::set_type(&grappler, value);
        }
        owner.borrow_mut().picking_up = false;
    }
}

impl<I: 'static> GameCharacter<I> {
    /// Exact original query: actual grappler role, real zombie endpoint,
    /// and that target's current canonical reanimated-only scalar.
    pub fn is_dragging_corpse(&self) -> bool {
        let target = {
            let state = self.grapple.borrow();
            if !state.grappling {
                return false;
            }
            state.target.clone()
        };
        target.is_some_and(|target| {
            let target = target.borrow();
            target.kind == CharacterKind::Zombie && target.zombie_reanimated.get()
        })
    }
    pub fn is_grappling(&self) -> bool {
        self.grapple.borrow().grappling
    }
    pub fn is_being_grappled(&self) -> bool {
        self.grapple.borrow().being_grappled
    }
    pub fn is_do_grapple(&self) -> bool {
        self.grapple.borrow().do_grapple
    }
    pub fn set_do_grapple(&self, value: bool) {
        self.grapple.borrow_mut().do_grapple = value;
    }
    pub fn is_do_continue_grapple(&self) -> bool {
        self.grapple.borrow().continuing
    }
    pub fn set_do_continue_grapple(&self, value: bool) {
        let mut state = self.grapple.borrow_mut();
        let old = state.continuing;
        state.continuing = value;
        if state.grappling && old && !value {
            state.putting_down = true;
        }
    }
    pub fn is_picking_up_body(&self) -> bool {
        self.grapple.borrow().picking_up
    }
    pub fn is_putting_down_body(&self) -> bool {
        self.grapple.borrow().putting_down
    }
    pub fn shared_grapple_type(&self) -> Option<String> {
        self.grapple.borrow().shared_type.clone()
    }
    pub fn set_shared_grapple_type(&self, value: Option<&str>) {
        GrappleState::set_type(&self.grapple, value);
    }
    pub fn shared_grapple_anim_node(&self) -> String {
        self.grapple.borrow().anim_node.clone()
    }
    pub fn shared_grapple_time(&self) -> f32 {
        self.grapple.borrow().time
    }
    pub fn shared_grapple_fraction(&self) -> f32 {
        self.grapple.borrow().fraction
    }
    pub fn grapple_result(&self) -> Option<String> {
        self.grapple.borrow().result.clone()
    }
    pub fn grapple_offset_forward(&self) -> f32 {
        self.grapple.borrow().offset_forward
    }
    pub fn grapple_offset_behaviour(&self) -> String {
        self.grapple.borrow().offset_behaviour.clone()
    }

    /// BaseGrappleable.setGrapplingTarget is intentionally one-sided and
    /// accepts null; it does not run AcceptGrapple or participant callbacks.
    pub fn set_grappling_target(&self, target: Option<&Self>, shared_type: Option<&str>) {
        let target = target.map(|target| target.grapple.clone());
        let old = {
            let mut state = self.grapple.borrow_mut();
            let old = state.reset(Some(String::new()));
            state.grappling = true;
            state.continuing = true;
            state.target = target;
            state.shared_type = shared_type.map(str::to_string);
            old
        };
        drop(old);
    }
    pub fn is_grappling_target(&self, target: Option<&Self>) -> bool {
        let state = self.grapple.borrow();
        let current = state.grappling.then_some(state.target.as_ref()).flatten();
        match (current, target) {
            (Some(current), Some(target)) => Rc::ptr_eq(current, &target.grapple),
            (None, None) => true,
            _ => false,
        }
    }
    pub fn reset_grapple_state_to_default(&self, result: Option<&str>) {
        let old = self.grapple.borrow_mut().reset(result.map(str::to_string));
        drop(old);
    }
}

#[cfg(test)]
mod tests;
