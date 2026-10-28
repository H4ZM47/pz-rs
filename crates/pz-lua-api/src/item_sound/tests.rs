use super::*;
use pz_inventory::{InventoryItem, ItemClass, ItemTemplate};
use pz_sound::backend::*;

fn item(class: ItemClass, id: i32) -> ItemRef {
    let mut item = InventoryItem::create(Rc::new(ItemTemplate::new(class, "Base", "QueueItem")), 1);
    item.id = id;
    let item = pz_inventory::shared::new_item(item);
    pz_inventory::component_host::drain_retired_components();
    item
}

struct NativeOwnerMetadata {
    item: Option<pz_inventory::shared::ItemObserver>,
    container: Option<pz_inventory::shared::ContainerObserver>,
}
impl NativeOwnerMetadata {
    fn new() -> Self {
        Self {
            item: pz_inventory::shared::install_item_observer(Some(crate::component_gc::observe_item)),
            container: pz_inventory::shared::install_container_observer(Some(crate::component_gc::observe_container)),
        }
    }
}
impl Drop for NativeOwnerMetadata {
    fn drop(&mut self) {
        pz_inventory::shared::install_item_observer(self.item);
        pz_inventory::shared::install_container_observer(self.container);
        pz_inventory::component_host::drain_retired_components();
        crate::component_gc::retry_pending();
        let _ = pz_kahlua::gc::collect_tables();
    }
}

#[test]
fn item_sound_identity_cancel_and_delayed_readd_rules() {
    let manager = Rc::new(RefCell::new(ItemSoundManager::default()));
    let mut sound = SoundSystem::silent();
    let a = item(ItemClass::Normal, 8);
    let b = item(ItemClass::Normal, 8);
    {
        let mut m = manager.borrow_mut();
        m.add_item(None, false);
        m.add_item(Some(a.clone()), true);
        assert!(m.to_add.is_empty());
        m.add_item(Some(a.clone()), false);
        m.add_item(Some(a.clone()), false);
        m.remove_item(Some(a.clone()), false);
        assert!(m.to_add.is_empty(), "removal cancels not-yet-added item");
        m.add_item(Some(a.clone()), false);
        m.add_item(Some(b.clone()), false);
    }
    update_manager(&manager, &mut sound, 17, false);
    assert_eq!(
        manager.borrow().active.len(),
        2,
        "IDs are not object identity"
    );
    let emitter = manager.borrow().emitter_for(&a).unwrap();
    {
        let mut m = manager.borrow_mut();
        m.remove_item(Some(a.clone()), false);
        m.remove_item(Some(a.clone()), false);
        m.add_item(Some(a.clone()), false);
        assert_eq!(
            m.to_remove.len(),
            1,
            "active sound re-add does not cancel removal"
        );
    }
    update_manager(&manager, &mut sound, 17, false);
    assert!(manager.borrow().emitter_for(&a).is_none());
    assert_eq!(manager.borrow().to_stop.len(), 1);
    manager.borrow_mut().add_item(Some(a.clone()), false);
    assert!(manager.borrow().to_stop.is_empty());
    assert!(Rc::ptr_eq(
        &emitter,
        &manager.borrow().emitter_for(&a).unwrap()
    ));
    update_manager(&manager, &mut sound, 17, false);
    assert_eq!(sound.world_emitters.free_count(), 0);
}

struct RecordingBackend {
    calls: Rc<RefCell<Vec<String>>>,
    next: i64,
    manager: std::rc::Weak<RefCell<ItemSoundManager>>,
}

impl AudioBackend for RecordingBackend {
    fn event_description(&mut self, path: &str) -> Option<Rc<EventDescription>> {
        Some(Rc::new(EventDescription {
            address: 1,
            path: path.into(),
            length_ms: 5000,
            is_3d: false,
            is_oneshot: false,
            has_sustain_points: false,
            max_distance: 20.0,
            parameters: vec![],
        }))
    }
    fn create_event_instance(&mut self, _: &EventDescription) -> i64 {
        self.next += 1;
        self.calls
            .borrow_mut()
            .push(format!("create {}", self.next));
        self.next
    }
    fn instance_set_volume(&mut self, _: i64, _: f32) {}
    fn instance_set_parameter_by_name(&mut self, _: i64, _: &str, _: f32) {}
    fn instance_start(&mut self, n: i64) {
        self.calls.borrow_mut().push(format!("start {n}"));
    }
    fn instance_stop(&mut self, n: i64, _: bool) {
        assert!(
            self.manager.upgrade().unwrap().try_borrow_mut().is_ok(),
            "backend called with manager borrow held"
        );
        self.calls.borrow_mut().push(format!("stop {n}"));
    }
    fn instance_release(&mut self, n: i64) {
        self.calls.borrow_mut().push(format!("release {n}"));
    }
    fn instance_playback_state(&mut self, _: i64) -> PlaybackState {
        PlaybackState::Playing
    }
    fn listener_3d(&mut self, _: i32, _: [f32; 3], _: [f32; 3], _: [f32; 3], _: [f32; 3]) {}
    fn load_sound(&mut self, _: &str, _: bool) -> u64 {
        0
    }
    fn play_sound(&mut self, _: u64, _: bool) -> u64 {
        0
    }
    fn channel_set_volume(&mut self, _: u64, _: f32) {}
    fn channel_set_pitch(&mut self, _: u64, _: f32) {}
    fn channel_set_mode(&mut self, _: u64, _: u64) {}
    fn channel_set_paused(&mut self, _: u64, _: bool) {}
    fn channel_is_playing(&mut self, _: u64) -> bool {
        false
    }
    fn channel_stop(&mut self, _: u64) {}
    fn master_channel_group_set_volume(&mut self, _: f32) {}
}

#[test]
fn item_sound_real_backend_stops_next_frame_and_reuses_after_release() {
    let manager = Rc::new(RefCell::new(ItemSoundManager::default()));
    let calls = Rc::new(RefCell::new(Vec::new()));
    let mut sound = SoundSystem::new(
        Box::new(RecordingBackend {
            calls: calls.clone(),
            next: 40,
            manager: Rc::downgrade(&manager),
        }),
        Default::default(),
        0,
    );
    let a = item(ItemClass::Normal, 1);
    let b = item(ItemClass::Normal, 2);
    manager.borrow_mut().add_item(Some(a.clone()), false);
    update_manager(&manager, &mut sound, 17, false);
    let emitter = manager.borrow().emitter_for(&a).unwrap();
    let reference =
        sound.with_world_emitters(|_, ctx| emitter.borrow_mut().play_sound("QueueLoop", ctx));
    assert_eq!(reference, 41);
    update_manager(&manager, &mut sound, 17, false);
    assert!(calls.borrow().contains(&"start 41".to_string()));
    manager.borrow_mut().remove_item(Some(a.clone()), false);
    update_manager(&manager, &mut sound, 17, false);
    assert!(!calls.borrow().iter().any(|c| c.starts_with("stop")));
    manager.borrow_mut().add_item(Some(a.clone()), false);
    update_manager(&manager, &mut sound, 17, false);
    assert!(
        !calls.borrow().iter().any(|c| c.starts_with("stop")),
        "re-add rescues delayed emitter"
    );
    manager.borrow_mut().remove_item(Some(a), false);
    update_manager(&manager, &mut sound, 17, false);
    manager.borrow_mut().add_item(Some(b.clone()), false);
    update_manager(&manager, &mut sound, 17, false);
    assert_eq!(
        &*calls.borrow(),
        &["create 41", "start 41", "stop 41", "release 41"]
    );
    let reused = manager.borrow().emitter_for(&b).unwrap();
    assert!(
        Rc::ptr_eq(&emitter, &reused),
        "stop/return precedes pending additions"
    );
    assert!(
        sound.world_emitters.current().is_empty(),
        "manager owns active emitter"
    );
    assert_eq!(sound.world_emitters.free_count(), 0);
    sound.with_world_emitters(|_, ctx| {
        assert_eq!(reused.borrow_mut().play_sound("QueueLoop", ctx), 42)
    });
    assert_eq!(calls.borrow().last().unwrap(), "create 42");
}

#[test]
fn item_sound_alarm_retirement_applies_real_stop_ringing_fields() {
    let manager = Rc::new(RefCell::new(ItemSoundManager::default()));
    let mut sound = SoundSystem::silent();
    let clocks: Vec<_> = [ItemClass::AlarmClock, ItemClass::AlarmClockClothing]
        .into_iter()
        .map(|class| {
            let clock = item(class, 9);
            pz_inventory::alarm::alarm_mut(&mut clock.borrow_mut())
                .unwrap()
                .ring_since = 3.0;
            manager.borrow_mut().add_item(Some(clock.clone()), false);
            clock
        })
        .collect();
    update_manager(&manager, &mut sound, 37, false);
    for clock in &clocks {
        manager.borrow_mut().remove_item(Some(clock.clone()), false);
    }
    update_manager(&manager, &mut sound, 37, false);
    assert!(clocks
        .iter()
        .all(|c| pz_inventory::alarm::alarm(&c.borrow())
            .unwrap()
            .is_ringing()));
    update_manager(&manager, &mut sound, 37, false);
    for clock in clocks {
        let clock = clock.borrow();
        let alarm = pz_inventory::alarm::alarm(&clock).unwrap();
        assert!(!alarm.is_ringing());
        assert_eq!(alarm.force_dont_ring, 30);
    }
    assert_eq!(sound.world_emitters.free_count(), 2);
}

#[test]
fn item_sound_delayed_final_owner_drop_can_reenter_sound_and_manager() {
    struct ReenterAudio {
        sound: std::rc::Weak<RefCell<SoundSystem>>,
        manager: std::rc::Weak<RefCell<ItemSoundManager>>,
        dropped: Rc<std::cell::Cell<bool>>,
    }
    impl Drop for ReenterAudio {
        fn drop(&mut self) {
            assert!(
                self.sound.upgrade().unwrap().try_borrow_mut().is_ok(),
                "SoundSystem still borrowed during item host destruction"
            );
            assert!(
                self.manager.upgrade().unwrap().try_borrow_mut().is_ok(),
                "ItemSoundManager still borrowed during item host destruction"
            );
            self.dropped.set(true);
        }
    }
    let old_sound = crate::bind_audio::sound_system();
    let old_core = crate::bind_core::shared_core();
    crate::bind_core::install_core(Rc::new(RefCell::new(pz_core::core::Core::default())));
    let sound = Rc::new(RefCell::new(SoundSystem::silent()));
    crate::bind_audio::install_sound_system(sound.clone());
    reset();
    let manager = shared_manager();
    let _metadata = NativeOwnerMetadata::new();
    let dropped = Rc::new(std::cell::Cell::new(false));
    let bag = item(ItemClass::Container, 5);
    let weak = Rc::downgrade(&bag);
    pz_inventory::shared::inventory_of(&bag.borrow())
        .unwrap()
        .borrow_mut()
        .parent = Some(Rc::new(ReenterAudio {
        sound: Rc::downgrade(&sound),
        manager: Rc::downgrade(&manager),
        dropped: dropped.clone(),
    }));
    add_item(Some(bag.clone()));
    update();
    remove_items([bag.clone()]);
    drop(bag);
    update();
    assert!(
        weak.upgrade().is_some(),
        "first phase only queues delayed retirement"
    );
    assert!(!dropped.get());
    assert!(manager.borrow().to_stop.iter().any(|pair| {
        Rc::ptr_eq(&pair.item, &weak.upgrade().unwrap())
    }), "first update must still own the actual delayed item");
    update();
    assert!(manager.borrow().to_stop.is_empty());
    pz_inventory::component_host::drain_retired_components();
    assert!(weak.upgrade().is_some(), "actual bag owner cycle remains after manager release");
    crate::component_gc::retry_pending();
    assert!(matches!(pz_kahlua::gc::collect_tables(), pz_kahlua::gc::CollectionResult::Collected(_)));
    assert!(weak.upgrade().is_none());
    assert!(dropped.get());
    reset();
    crate::bind_audio::install_sound_system(old_sound);
    crate::bind_core::install_core(old_core);
}
