//! Music intensity events retained by each IsoPlayer and exposed to Lua.
use pz_kahlua::value::LuaObject;
use std::{
    any::Any,
    cell::{Cell, RefCell},
    rc::Rc,
};

pub const INTENSITY_EVENT_CLASS: &str = "zombie.audio.MusicIntensityEvent";
pub const INTENSITY_EVENTS_CLASS: &str = "zombie.audio.MusicIntensityEvents";

/// Java `String.equalsIgnoreCase` (`StringUtils.equalsIgnoreCase` is
/// null-safe; both null compare equal).
pub fn eq_ignore_case(a: Option<&str>, b: Option<&str>) -> bool {
    match (a, b) {
        (Some(a), Some(b)) if a.is_ascii() && b.is_ascii() => a.eq_ignore_ascii_case(b),
        (Some(a), Some(b)) => {
            a.chars().count() == b.chars().count() && a.to_lowercase() == b.to_lowercase()
        }
        (None, None) => true,
        _ => false,
    }
}

#[derive(Debug, PartialEq)]
pub struct MusicIntensityEvent {
    pub id: Option<Rc<str>>,
    pub intensity: f32,
    pub duration_ms: i64,
    pub elapsed_ms: Cell<i64>,
}

impl LuaObject for MusicIntensityEvent {
    fn class_name(&self) -> &str {
        INTENSITY_EVENT_CLASS
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

#[derive(Debug, PartialEq)]
pub struct MusicIntensityEvents {
    events: RefCell<Vec<Rc<MusicIntensityEvent>>>,
    update_time_ms: Cell<i64>,
    intensity: Cell<f32>,
}

impl MusicIntensityEvents {
    pub fn new() -> Self {
        Self {
            events: RefCell::default(),
            update_time_ms: Cell::new(-1),
            intensity: Cell::default(),
        }
    }

    pub fn add_event(
        &self,
        id: Option<Rc<str>>,
        intensity: f32,
        duration_ms: i64,
        multiple: bool,
    ) -> Rc<MusicIntensityEvent> {
        if !multiple {
            if let Some(e) = self.find_event_by_id(id.as_deref()) {
                e.elapsed_ms.set(0);
                return e;
            }
        }
        let e = Rc::new(MusicIntensityEvent {
            id,
            intensity,
            duration_ms,
            elapsed_ms: Cell::new(0),
        });
        self.events.borrow_mut().push(e.clone());
        e
    }

    pub fn find_event_by_id(&self, id: Option<&str>) -> Option<Rc<MusicIntensityEvent>> {
        self.events
            .borrow()
            .iter()
            .find(|e| eq_ignore_case(e.id.as_deref(), id))
            .cloned()
    }

    pub fn clear(&self) {
        self.events.borrow_mut().clear();
    }

    pub fn count(&self) -> usize {
        self.events.borrow().len()
    }

    pub fn event(&self, index: usize) -> Option<Rc<MusicIntensityEvent>> {
        self.events.borrow().get(index).cloned()
    }

    pub fn intensity(&self) -> f32 {
        self.intensity.get()
    }

    fn calculate_intensity(&self) -> f32 {
        let mut i = 50.0f32;
        for e in self.events.borrow().iter() {
            i += e.intensity;
        }
        i.clamp(0.0, 100.0)
    }

    /// `update()` with `System.currentTimeMillis()` passed in.
    pub fn update_at(&self, now_ms: i64) {
        let elapsed = now_ms.wrapping_sub(self.update_time_ms.get());
        self.update_time_ms.set(now_ms);
        self.events.borrow_mut().retain(|e| {
            if e.duration_ms <= 0 && e.elapsed_ms.get() > 0 {
                return false;
            }
            e.elapsed_ms.set(e.elapsed_ms.get().wrapping_add(elapsed));
            !(e.duration_ms > 0 && e.elapsed_ms.get() >= e.duration_ms)
        });
        self.intensity.set(self.calculate_intensity());
    }
}

impl LuaObject for MusicIntensityEvents {
    fn class_name(&self) -> &str {
        INTENSITY_EVENTS_CLASS
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

impl Default for MusicIntensityEvents {
    fn default() -> Self {
        Self::new()
    }
}

pub const THREAT_STATUS_CLASS: &str = "zombie.audio.MusicThreatStatus";

#[derive(Debug, PartialEq)]
pub struct MusicThreatStatus {
    pub id: Option<Rc<str>>,
    pub intensity: Cell<f32>,
}

impl LuaObject for MusicThreatStatus {
    fn class_name(&self) -> &str {
        THREAT_STATUS_CLASS
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

#[derive(Debug, Default, PartialEq)]
pub struct MusicThreatLevels {
    statuses: RefCell<Vec<Rc<MusicThreatStatus>>>,
    intensity: Cell<f32>,
}

impl MusicThreatLevels {
    pub fn set_status(&self, id: Option<Rc<str>>, intensity: f32) -> Rc<MusicThreatStatus> {
        if let Some(s) = self.find_status_by_id(id.as_deref()) {
            s.intensity.set(intensity);
            return s;
        }
        let s = Rc::new(MusicThreatStatus {
            id,
            intensity: Cell::new(intensity),
        });
        self.statuses.borrow_mut().push(s.clone());
        s
    }

    pub fn find_status_by_id(&self, id: Option<&str>) -> Option<Rc<MusicThreatStatus>> {
        self.statuses
            .borrow()
            .iter()
            .find(|s| eq_ignore_case(s.id.as_deref(), id))
            .cloned()
    }

    pub fn clear(&self) {
        self.statuses.borrow_mut().clear();
    }

    pub fn count(&self) -> usize {
        self.statuses.borrow().len()
    }

    pub fn status(&self, index: usize) -> Option<Rc<MusicThreatStatus>> {
        self.statuses.borrow().get(index).cloned()
    }

    pub fn intensity(&self) -> f32 {
        self.intensity.get()
    }

    pub fn calculate_intensity(&self, contribution: impl Fn(&MusicThreatStatus) -> f32) {
        self.intensity
            .set(self.statuses.borrow().iter().map(|s| contribution(s)).sum());
    }

    fn set_player_status(&self, id: &str, value: f32) {
        if let Some(status) = self.find_status_by_id(Some(id)) {
            status.intensity.set(value);
        } else {
            self.set_status(Some(id.into()), value);
        }
    }

    pub fn update_from_player<I>(&self, player: &crate::character::GameCharacter<I>, panic: i32) {
        let stats = &player.stats;
        self.set_player_status("MoodlePanic", panic as f32 / 4.0);
        self.set_player_status("PlayerHealth", 1.0 - player.health);
        for (id, count) in [
            ("ZombiesVisible", stats.music_zombies_visible),
            (
                "ZombiesTargeting.DistantNotMoving",
                stats.music_zombies_targeting_distant_not_moving,
            ),
            (
                "ZombiesTargeting.NearbyNotMoving",
                stats.music_zombies_targeting_nearby_not_moving,
            ),
            (
                "ZombiesTargeting.DistantMoving",
                stats.music_zombies_targeting_distant_moving,
            ),
            (
                "ZombiesTargeting.NearbyMoving",
                stats.music_zombies_targeting_nearby_moving,
            ),
        ] {
            self.set_player_status(id, (count as f32 / 100.0).clamp(0.0, 1.0));
        }
    }
}
