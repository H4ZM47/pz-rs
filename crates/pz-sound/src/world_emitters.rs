//! `IsoWorld`'s free/current emitter ownership, independent of the UI emitter.

use crate::emitter::FmodSoundEmitter;
use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;

pub type EmitterRef = Rc<RefCell<FmodSoundEmitter>>;

pub struct WorldEmitters {
    free: VecDeque<EmitterRef>,
    current: Vec<EmitterRef>,
    parameter_count: usize,
    last_update_ms: i64,
}

impl WorldEmitters {
    pub fn new(parameter_count: usize) -> Self {
        Self {
            free: VecDeque::new(),
            current: Vec::new(),
            parameter_count,
            last_update_ms: 0,
        }
    }

    pub fn new_emitter(&self) -> EmitterRef {
        Rc::new(RefCell::new(FmodSoundEmitter::new(self.parameter_count)))
    }

    /// `getFreeEmitter()`: return an emitter under world ownership.
    pub fn get_free(&mut self) -> EmitterRef {
        let emitter = self
            .free
            .pop_front()
            .unwrap_or_else(|| self.new_emitter());
        emitter.borrow_mut().set_play_remote_events(false);
        self.current.push(emitter.clone());
        emitter
    }

    /// An item/other owner becomes responsible for ticking the emitter.
    pub fn take_ownership(&mut self, emitter: &EmitterRef) {
        if let Some(n) = self.current.iter().position(|e| Rc::ptr_eq(e, emitter)) {
            self.current.remove(n);
        }
    }

    /// `returnOwnershipOfEmitter`: empty emitters return to the FIFO free
    /// pool after clearing their parameters; nonempty ones remain current.
    pub fn return_ownership(&mut self, emitter: EmitterRef) {
        if self
            .current
            .iter()
            .chain(self.free.iter())
            .any(|e| Rc::ptr_eq(e, &emitter))
        {
            return;
        }
        if emitter.borrow().is_empty() {
            emitter.borrow_mut().clear_parameters();
            self.free.push_back(emitter);
        } else {
            self.current.push(emitter);
        }
    }

    /// `IsoWorld.updateInternal` starts newly queued sounds immediately
    /// and advances ongoing ones every 30 ms. Empty emitters are reusable.
    pub fn update(&mut self, now_ms: i64, ctx: &mut crate::emitter::Ctx) -> bool {
        let due = now_ms.saturating_sub(self.last_update_ms) >= 30;
        if due { self.last_update_ms = now_ms; }
        self.current.retain(|owner| {
            let mut emitter = owner.borrow_mut();
            if due || emitter.has_sounds_to_start() { emitter.tick(ctx); }
            if !emitter.is_empty() { return true; }
            emitter.clear_parameters();
            self.free.push_front(owner.clone());
            false
        });
        due
    }

    pub fn current(&self) -> &[EmitterRef] {
        &self.current
    }
    pub fn free_count(&self) -> usize {
        self.free.len()
    }

    /// Stop queued and playing backend instances before discarding this world.
    pub fn stop_and_reset(&mut self, ctx: &mut crate::emitter::Ctx) -> Vec<EmitterRef> {
        for emitter in &self.current {
            emitter.borrow_mut().stop_all(ctx);
        }
        self.reset()
    }

    /// `KillCell` replaces IsoWorld with a new instance. Return the old
    /// owning slots so their payloads drop after the caller's system borrow.
    pub fn reset(&mut self) -> Vec<EmitterRef> {
        self.last_update_ms = 0;
        let mut retired: Vec<_> = self.free.drain(..).collect();
        retired.append(&mut self.current);
        retired
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn world_emitter_ownership_is_distinct_fifo_and_deduplicated() {
        let mut world = WorldEmitters::new(3);
        let a = world.get_free();
        let b = world.get_free();
        assert!(!Rc::ptr_eq(&a, &b));
        assert_eq!(world.current().len(), 2);
        world.take_ownership(&a);
        world.take_ownership(&b);
        assert!(world.current().is_empty());
        world.return_ownership(a.clone());
        world.return_ownership(a.clone());
        world.return_ownership(b.clone());
        assert_eq!(world.free_count(), 2);
        assert!(Rc::ptr_eq(&a, &world.get_free()));
        assert!(Rc::ptr_eq(&b, &world.get_free()));
        assert_eq!(world.free_count(), 0);
    }
}
