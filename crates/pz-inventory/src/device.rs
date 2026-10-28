//! `DeviceData` and `DevicePresets` (radios and televisions, as items and
//! as world objects).

use crate::io::{Reader, SaveResult, Writer};
use std::cell::RefCell;
use std::rc::Rc;

/// `PresetEntry`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PresetEntry {
    pub name: String,
    pub frequency: i32,
}

/// The actual PresetEntry allocation, independent of its list position.
pub type PresetEntryRef = Rc<RefCell<PresetEntry>>;
/// The actual list allocation; removing a slot does not destroy an entry
/// still held by another slot or caller.
pub type PresetListRef = Rc<RefCell<Vec<PresetEntryRef>>>;

/// `DevicePresets`' physical fields.
#[derive(Debug, PartialEq, Eq)]
pub struct DevicePresets {
    pub max_presets: i32,
    pub presets: PresetListRef,
}

impl Default for DevicePresets {
    fn default() -> Self {
        Self {
            max_presets: 10,
            presets: Rc::default(),
        }
    }
}

/// An owning reference to the same DevicePresets allocation. Clone shares
/// that owner; `deep_copy` is the native copy-constructor operation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DevicePresetsRef(Rc<RefCell<DevicePresets>>);

impl Default for DevicePresetsRef {
    fn default() -> Self {
        Self(Rc::new(RefCell::new(DevicePresets::default())))
    }
}

impl DevicePresetsRef {
    pub fn owner(&self) -> &Rc<RefCell<DevicePresets>> {
        &self.0
    }
    pub fn list(&self) -> PresetListRef {
        self.0.borrow().presets.clone()
    }
    pub fn max_presets(&self) -> i32 {
        self.0.borrow().max_presets
    }
    pub fn set_max_presets(&self, n: i32) {
        self.0.borrow_mut().max_presets = n;
    }
    pub fn len(&self) -> usize {
        self.list().borrow().len()
    }
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
    pub fn entry(&self, i: usize) -> Option<PresetEntryRef> {
        self.list().borrow().get(i).cloned()
    }
    pub fn clear_presets(&self) {
        self.list().borrow_mut().clear();
    }
    pub fn remove_preset(&self, id: i32) {
        let list = self.list();
        let mut list = list.borrow_mut();
        if id >= 0 && (id as usize) < list.len() {
            list.remove(id as usize);
        }
    }
    pub fn preset_name(&self, id: i32) -> String {
        usize::try_from(id)
            .ok()
            .and_then(|i| self.entry(i))
            .map(|p| p.borrow().name.clone())
            .unwrap_or_default()
    }
    pub fn preset_freq(&self, id: i32) -> i32 {
        usize::try_from(id)
            .ok()
            .and_then(|i| self.entry(i))
            .map_or(-1, |p| p.borrow().frequency)
    }
    pub fn set_preset_name(&self, id: i32, name: Option<&str>) {
        if let Some(p) = usize::try_from(id).ok().and_then(|i| self.entry(i)) {
            p.borrow_mut().name = name.unwrap_or("name-is-null").to_owned();
        }
    }
    pub fn set_preset_freq(&self, id: i32, frequency: i32) {
        if let Some(p) = usize::try_from(id).ok().and_then(|i| self.entry(i)) {
            p.borrow_mut().frequency = frequency;
        }
    }
    pub fn set_preset(&self, id: i32, name: Option<&str>, frequency: i32) {
        if let Some(p) = usize::try_from(id).ok().and_then(|i| self.entry(i)) {
            let mut p = p.borrow_mut();
            p.name = name.unwrap_or("name-is-null").to_owned();
            p.frequency = frequency;
        }
    }
    pub fn add_preset(&self, name: &str, frequency: i32) {
        let list = self.list();
        let mut list = list.borrow_mut();
        if list.iter().any(|p| {
            let p = p.borrow();
            p.frequency == frequency || p.name == name
        }) {
            return;
        }
        if (list.len() as i32) < self.max_presets() {
            list.push(Rc::new(RefCell::new(PresetEntry {
                name: name.to_owned(),
                frequency,
            })));
        }
    }
    pub fn deep_copy(&self) -> Self {
        let copy = Self::default();
        copy.set_max_presets(self.max_presets());
        let list = self.list();
        let entries = list
            .borrow()
            .iter()
            .map(|p| Rc::new(RefCell::new(p.borrow().clone())))
            .collect();
        *copy.list().borrow_mut() = entries;
        copy
    }
}

/// `DeviceData`: the saved state fields plus the runtime state of the
/// device simulation (`crate::device_sim`).
#[derive(Debug, PartialEq)]
pub struct DeviceData {
    pub device_name: String,
    pub two_way: bool,
    pub transmit_range: i32,
    pub mic_range: i32,
    pub mic_is_muted: bool,
    pub base_volume_range: f32,
    pub device_volume: f32,
    pub is_portable: bool,
    pub is_television: bool,
    pub is_high_tier: bool,
    pub is_turned_on: bool,
    pub channel: i32,
    pub min_channel_range: i32,
    pub max_channel_range: i32,
    pub presets: DevicePresetsRef,
    pub is_battery_powered: bool,
    pub has_battery: bool,
    pub power_delta: f32,
    pub use_delta: f32,
    pub headphone_type: i32,
    pub media_index: i16,
    pub media_type: i8,
    pub no_transmit: bool,
    pub last_minute_stamp: i64,
    pub media_item: Option<String>,
    /// The fields that are not saved (counters, media playback, the
    /// emitter); see [`crate::device_sim`].
    pub runtime: crate::device_sim::DeviceRuntime,
}

impl Clone for DeviceData {
    fn clone(&self) -> Self {
        Self {
            device_name: self.device_name.clone(),
            two_way: self.two_way.clone(),
            transmit_range: self.transmit_range.clone(),
            mic_range: self.mic_range.clone(),
            mic_is_muted: self.mic_is_muted.clone(),
            base_volume_range: self.base_volume_range.clone(),
            device_volume: self.device_volume.clone(),
            is_portable: self.is_portable.clone(),
            is_television: self.is_television.clone(),
            is_high_tier: self.is_high_tier.clone(),
            is_turned_on: self.is_turned_on.clone(),
            channel: self.channel.clone(),
            min_channel_range: self.min_channel_range.clone(),
            max_channel_range: self.max_channel_range.clone(),
            presets: self.presets.deep_copy(),
            is_battery_powered: self.is_battery_powered.clone(),
            has_battery: self.has_battery.clone(),
            power_delta: self.power_delta.clone(),
            use_delta: self.use_delta.clone(),
            headphone_type: self.headphone_type.clone(),
            media_index: self.media_index.clone(),
            media_type: self.media_type.clone(),
            no_transmit: self.no_transmit.clone(),
            last_minute_stamp: self.last_minute_stamp.clone(),
            media_item: self.media_item.clone(),
            runtime: self.runtime.clone(),
        }
    }
}

impl Default for DeviceData {
    /// `new DeviceData()`.
    fn default() -> Self {
        DeviceData {
            device_name: "WaveSignalDevice".to_string(),
            two_way: false,
            transmit_range: 1000,
            mic_range: 5,
            mic_is_muted: false,
            base_volume_range: 15.0,
            device_volume: 1.0,
            is_portable: false,
            is_television: false,
            is_high_tier: false,
            is_turned_on: false,
            channel: 88000,
            min_channel_range: 200,
            max_channel_range: 1000000,
            presets: DevicePresetsRef::default(),
            is_battery_powered: true,
            has_battery: true,
            power_delta: 1.0,
            use_delta: 0.001,
            headphone_type: -1,
            media_index: -1,
            media_type: -1,
            no_transmit: false,
            last_minute_stamp: -1,
            media_item: None,
            runtime: Default::default(),
        }
    }
}

impl DeviceData {
    /// `DeviceData.load(input, worldVersion, false)`.
    pub fn load(&mut self, r: &mut Reader<'_>) -> SaveResult<()> {
        let d = self;
        d.device_name = r.string()?;
        d.two_way = r.bool()?;
        d.transmit_range = r.i32()?;
        d.mic_range = r.i32()?;
        d.mic_is_muted = r.bool()?;
        d.base_volume_range = r.f32()?;
        d.device_volume = r.f32()?;
        d.is_portable = r.bool()?;
        d.is_television = r.bool()?;
        d.is_high_tier = r.bool()?;
        d.is_turned_on = r.bool()?;
        d.channel = r.i32()?;
        d.min_channel_range = r.i32()?;
        d.max_channel_range = r.i32()?;
        d.is_battery_powered = r.bool()?;
        d.has_battery = r.bool()?;
        d.power_delta = r.f32()?;
        d.use_delta = r.f32()?;
        d.headphone_type = r.i32()?;
        if r.bool()? {
            d.presets.clear_presets();
            d.presets.set_max_presets(r.i32()?);
            let n = r.i32()?;
            for _ in 0..n.max(0) {
                let name = r.string()?;
                let frequency = r.i32()?;
                if (d.presets.len() as i32) < d.presets.max_presets() {
                    d.presets
                        .list()
                        .borrow_mut()
                        .push(Rc::new(RefCell::new(PresetEntry { name, frequency })));
                }
            }
        }
        d.media_index = r.i16()?;
        d.media_type = r.i8()?;
        if r.bool()? {
            d.media_item = Some(r.string()?);
        }
        d.no_transmit = r.bool()?;
        Ok(())
    }

    /// `DeviceData.save(output, false)` (presets are never null).
    pub fn save(&self, w: &mut Writer) {
        let d = self;
        w.string(&d.device_name);
        w.bool(d.two_way);
        w.i32(d.transmit_range);
        w.i32(d.mic_range);
        w.bool(d.mic_is_muted);
        w.f32(d.base_volume_range);
        w.f32(d.device_volume);
        w.bool(d.is_portable);
        w.bool(d.is_television);
        w.bool(d.is_high_tier);
        w.bool(d.is_turned_on);
        w.i32(d.channel);
        w.i32(d.min_channel_range);
        w.i32(d.max_channel_range);
        w.bool(d.is_battery_powered);
        w.bool(d.has_battery);
        w.f32(d.power_delta);
        w.f32(d.use_delta);
        w.i32(d.headphone_type);
        w.i8(1);
        w.i32(d.presets.max_presets());
        w.i32(d.presets.len() as i32);
        for p in d.presets.list().borrow().iter() {
            let p = p.borrow();
            w.string(&p.name);
            w.i32(p.frequency);
        }
        w.i16(d.media_index);
        w.i8(d.media_type);
        match &d.media_item {
            Some(m) => {
                w.i8(1);
                w.string(m);
            }
            None => w.i8(0),
        }
        w.bool(d.no_transmit);
    }
}

impl DeviceData {
    /// `setDeviceVolume` (without the network transmit).
    pub fn set_device_volume(&mut self, f: f32) {
        self.device_volume = f.clamp(0.0, 1.0);
    }

    /// `setTurnedOnRaw`.
    pub fn set_turned_on_raw(&mut self, b: bool) {
        self.is_turned_on = b;
        if !b {
            self.last_minute_stamp = -1;
        }
    }

    /// `setPower`.
    pub fn set_power(&mut self, p: f32) {
        self.power_delta = p.clamp(0.0, 1.0);
    }

    /// `setInitialPower` with `GameTime.getMinutesStamp()`.
    pub fn set_initial_power(&mut self, minutes_stamp: i64) {
        self.last_minute_stamp = minutes_stamp;
        self.set_power(self.power_delta - self.use_delta * minutes_stamp as f32);
    }
}

/// `ZomboidRadio.GetChannelList(category)`: the channels of a category in
/// the map's iteration order, `None` when the category is unknown (or the
/// radio data is not loaded). The channel lists come from the game's radio
/// data and are runtime input here; `pz-radio`'s `ZomboidRadio` (and its
/// `ChannelNames`) implement this from the loaded radio scripts.
pub trait ChannelLists {
    fn channel_list(&self, category: &str) -> Option<Vec<(i32, String)>>;
}

/// No radio data (`GetChannelList` answers `null` for every category).
pub struct NoChannels;
impl ChannelLists for NoChannels {
    fn channel_list(&self, _category: &str) -> Option<Vec<(i32, String)>> {
        None
    }
}

/// Channel lists by category, each in iteration order.
impl ChannelLists for std::collections::HashMap<String, Vec<(i32, String)>> {
    fn channel_list(&self, category: &str) -> Option<Vec<(i32, String)>> {
        self.get(category).cloned()
    }
}

/// `Rand.Next(min, max)` through `Rand.Next(bound)`.
fn next_range(next: &mut dyn FnMut(i32) -> i32, min: i32, max: i32) -> i32 {
    if max == min {
        return min;
    }
    let (min, max) = if min > max { (max, min) } else { (min, max) };
    next(max.wrapping_sub(min)).wrapping_add(min)
}

impl DeviceData {
    /// `setTransmitRange`.
    pub fn set_transmit_range(&mut self, range: i32) {
        self.transmit_range = range.max(0);
    }

    /// `setMinChannelRange`.
    pub fn set_min_channel_range(&mut self, i: i32) {
        self.min_channel_range = if (200..=1_000_000).contains(&i) {
            i
        } else {
            200
        };
    }

    /// `setMaxChannelRange`.
    pub fn set_max_channel_range(&mut self, i: i32) {
        self.max_channel_range = if (200..=1_000_000).contains(&i) {
            i
        } else {
            1_000_000
        };
    }

    /// `setUseDelta` (the script value is per hour).
    pub fn set_use_delta(&mut self, f: f32) {
        self.use_delta = f / 60.0;
    }

    /// `generatePresets()`; `next` is `Rand.Next(bound)`.
    pub fn generate_presets(&mut self, lists: &dyn ChannelLists, next: &mut dyn FnMut(i32) -> i32) {
        self.presets.clear_presets();
        let in_range =
            |d: &DeviceData, k: i32| k >= d.min_channel_range && k <= d.max_channel_range;
        if self.is_television {
            if let Some(list) = lists.channel_list("Television") {
                for (k, v) in list {
                    if in_range(self, k) {
                        self.presets.add_preset(&v, k);
                    }
                }
            }
            return;
        }
        let mut add = |d: &mut DeviceData, cat: &str, chance: i32| {
            if let Some(list) = lists.channel_list(cat) {
                for (k, v) in list {
                    if !in_range(d, k) || next(1000) >= chance {
                        continue;
                    }
                    d.presets.add_preset(&v, k);
                }
            }
        };
        let mut chance = if self.two_way { 100 } else { 300 };
        if self.is_high_tier {
            chance = 800;
        }
        add(self, "Emergency", chance);
        let chance = if self.two_way { 100 } else { 800 };
        add(self, "Radio", chance);
        if self.two_way {
            add(self, "Amateur", chance);
        }
        if self.is_high_tier {
            add(self, "Military", 10);
        }
    }

    /// `setRandomChannel()`; `next` is `Rand.Next(bound)`.
    pub fn set_random_channel(&mut self, next: &mut dyn FnMut(i32) -> i32) {
        if !self.presets.is_empty() {
            let r = next_range(next, 0, self.presets.len() as i32);
            self.channel = self.presets.entry(r as usize).unwrap().borrow().frequency;
        } else {
            self.channel = next_range(next, self.min_channel_range, self.max_channel_range);
            self.channel -= self.channel % 200;
        }
    }

    /// The `DeviceData` part of `Item.InstanceItem` for a `Radio`: the
    /// script values, then `generatePresets()` and `setRandomChannel()`.
    pub fn instance_from_script(
        &mut self,
        display_name: Option<&str>,
        s: &crate::template::RadioScript,
        lists: &dyn ChannelLists,
        next: &mut dyn FnMut(i32) -> i32,
    ) {
        if let Some(n) = display_name {
            self.device_name = n.to_string();
        }
        self.two_way = s.two_way;
        self.set_transmit_range(s.transmit_range);
        self.mic_range = s.mic_range;
        self.base_volume_range = s.base_volume_range;
        self.is_portable = s.is_portable;
        self.is_television = s.is_television;
        self.set_min_channel_range(s.min_channel);
        self.set_max_channel_range(s.max_channel);
        self.is_battery_powered = s.uses_battery;
        self.is_high_tier = s.is_high_tier;
        self.set_use_delta(s.use_delta);
        self.media_type = s.accept_media_type;
        self.no_transmit = s.no_transmit;
        self.generate_presets(lists, next);
        self.set_random_channel(next);
    }
}
