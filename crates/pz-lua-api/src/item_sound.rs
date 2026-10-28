//! `ItemSoundManager`'s real item/emitter ownership and deferred retirement.
//!
//! Items and emitters are paired by object identity. This ports the add,
//! remove and delayed-stop phases; item-specific `updateSound`, validity
//! checks, sound limiters and positioned updates remain unported. No sound
//! is invented when an item enters the manager.

use pz_inventory::shared::ItemRef;
use pz_sound::system::SoundSystem;
use pz_sound::world_emitters::EmitterRef;
use std::cell::RefCell;
use std::rc::Rc;

struct ItemEmitter {
    item: ItemRef,
    emitter: EmitterRef,
}

#[derive(Default)]
pub struct ItemSoundManager {
    active: Vec<ItemEmitter>,
    to_add: Vec<ItemRef>,
    to_remove: Vec<ItemRef>,
    to_stop: Vec<ItemEmitter>,
}

/// Owning slots retired by an update. Drop these after releasing the
/// enclosing SoundSystem borrow, since item host owners may reenter audio.
pub struct RetiredItemSounds {
    _delayed: Vec<ItemEmitter>,
    _removals: Vec<ItemRef>,
}

impl ItemSoundManager {
    pub fn add_item(&mut self, item: Option<ItemRef>, server: bool) {
        let Some(item) = item else { return };
        if server || self.active.iter().any(|p| Rc::ptr_eq(&p.item, &item)) {
            // Unlike the cell queue, re-adding an active item does not
            // cancel an already queued sound removal.
            return;
        }
        if let Some(n) = self.to_remove.iter().position(|i| Rc::ptr_eq(i, &item)) {
            self.to_remove.remove(n);
        }
        if let Some(n) = self.to_stop.iter().position(|p| Rc::ptr_eq(&p.item, &item)) {
            self.active.push(self.to_stop.remove(n));
            return;
        }
        if !self.to_add.iter().any(|i| Rc::ptr_eq(i, &item)) {
            self.to_add.push(item);
        }
    }

    pub fn remove_item(&mut self, item: Option<ItemRef>, server: bool) {
        if server {
            return;
        }
        let Some(item) = item else { return };
        if let Some(n) = self.to_add.iter().position(|i| Rc::ptr_eq(i, &item)) {
            self.to_add.remove(n);
        }
        if self.active.iter().any(|p| Rc::ptr_eq(&p.item, &item))
            && !self.to_remove.iter().any(|i| Rc::ptr_eq(i, &item))
        {
            self.to_remove.push(item);
        }
    }

    pub fn emitter_for(&self, item: &ItemRef) -> Option<EmitterRef> {
        self.active
            .iter()
            .find(|p| Rc::ptr_eq(&p.item, item))
            .map(|p| p.emitter.clone())
    }
}

thread_local! {
    static MANAGER: Rc<RefCell<ItemSoundManager>> = Rc::default();
}

pub fn shared_manager() -> Rc<RefCell<ItemSoundManager>> {
    MANAGER.with(Clone::clone)
}

pub fn add_item(item: Option<ItemRef>) {
    let server = crate::bind_core::shared_core().borrow().is_server;
    shared_manager().borrow_mut().add_item(item, server);
}

pub fn remove_items(items: impl IntoIterator<Item = ItemRef>) {
    let server = crate::bind_core::shared_core().borrow().is_server;
    let manager = shared_manager();
    for item in items {
        manager.borrow_mut().remove_item(Some(item), server);
    }
}

/// The default InventoryItem body is empty. Both alarm classes override
/// this with stopRinging. Apply their real alarm fields and removal here;
/// the absent playerOwner/ringSound audio path is not fabricated.
fn stop_sound_on_player(
    manager: &Rc<RefCell<ItemSoundManager>>,
    item: &ItemRef,
    minutes: i32,
    server: bool,
) {
    let alarm = {
        let mut item = item.borrow_mut();
        if let Some(alarm) = pz_inventory::alarm::alarm_mut(&mut item) {
            alarm.stop_ringing(minutes);
            true
        } else {
            false
        }
    };
    if alarm {
        manager.borrow_mut().remove_item(Some(item.clone()), server);
    }
}

/// The delayed-stop, add and removal phases run in that order. No manager
/// RefCell borrow spans an emitter/backend call or a detached payload drop.
pub fn update_manager(
    manager: &Rc<RefCell<ItemSoundManager>>,
    sound: &mut SoundSystem,
    minutes: i32,
    server: bool,
) -> RetiredItemSounds {
    let delayed = std::mem::take(&mut manager.borrow_mut().to_stop);
    for pair in &delayed {
        stop_sound_on_player(manager, &pair.item, minutes, server);
        sound.with_world_emitters(|pool, ctx| {
            pair.emitter.borrow_mut().stop_all(ctx);
            pool.return_ownership(pair.emitter.clone());
        });
    }
    let additions = std::mem::take(&mut manager.borrow_mut().to_add);
    for item in additions {
        let emitter = sound.with_world_emitters(|pool, _| {
            let emitter = pool.get_free();
            pool.take_ownership(&emitter);
            emitter
        });
        manager
            .borrow_mut()
            .active
            .push(ItemEmitter { item, emitter });
    }
    let removals = std::mem::take(&mut manager.borrow_mut().to_remove);
    for item in &removals {
        let mut manager = manager.borrow_mut();
        if let Some(n) = manager
            .active
            .iter()
            .position(|p| Rc::ptr_eq(&p.item, &item))
        {
            let pair = manager.active.remove(n);
            manager.to_stop.push(pair);
        }
    }
    // TODO(port): existing-container/world-item validity, per-item
    // updateSound, limiter frame/groups and positional emitter parameters.
    // Emitters already owned by actual callers still tick their real voices.
    let emitters: Vec<_> = manager
        .borrow()
        .active
        .iter()
        .map(|p| p.emitter.clone())
        .collect();
    for emitter in emitters {
        sound.with_world_emitters(|_, ctx| emitter.borrow_mut().tick(ctx));
    }
    RetiredItemSounds {
        _delayed: delayed,
        _removals: removals,
    }
}

/// `IngameState.UpdateStuff`'s unpaused ItemSoundManager phase.
pub fn update() {
    let minutes = crate::bind_climate::shared_game_time().borrow().minutes();
    let server = crate::bind_core::shared_core().borrow().is_server;
    let sound = crate::bind_audio::sound_system();
    let retired = {
        let mut sound = sound.borrow_mut();
        update_manager(&shared_manager(), &mut sound, minutes, server)
    };
    drop(retired);
}

/// `ItemSoundManager.Reset`: clear the owning lists, after KillCell.
pub fn reset() {
    let old = std::mem::take(&mut *shared_manager().borrow_mut());
    drop(old);
}

#[cfg(test)]
mod tests;
