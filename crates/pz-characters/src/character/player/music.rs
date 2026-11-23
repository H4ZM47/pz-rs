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
