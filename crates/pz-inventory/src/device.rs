//! `DeviceData` and `DevicePresets` (radios and televisions, as items and
//! as world objects).

use crate::io::{Reader, SaveResult, Writer};

/// `PresetEntry`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PresetEntry {
    pub name: String,
    pub frequency: i32,
}

/// `DevicePresets`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DevicePresets {
    pub max_presets: i32,
    pub presets: Vec<PresetEntry>,
}

impl Default for DevicePresets {
    fn default() -> Self {
        DevicePresets {
            max_presets: 10,
            presets: Vec::new(),
        }
    }
}

impl DevicePresets {
    /// `addPreset`: skipped when the frequency or the name is present, or
    /// when full.
    pub fn add_preset(&mut self, name: &str, frequency: i32) {
        if self
            .presets
            .iter()
            .any(|p| p.frequency == frequency || p.name == name)
        {
            return;
        }
        if (self.presets.len() as i32) < self.max_presets {
            self.presets.push(PresetEntry {
                name: name.to_string(),
                frequency,
            });
        }
    }
}

/// `DeviceData` (state fields; sound and media playback are not ported).
#[derive(Clone, Debug, PartialEq)]
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
    pub presets: DevicePresets,
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
            presets: DevicePresets::default(),
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
            d.presets.presets.clear();
            d.presets.max_presets = r.i32()?;
            let n = r.i32()?;
            for _ in 0..n.max(0) {
                let name = r.string()?;
                let frequency = r.i32()?;
                if (d.presets.presets.len() as i32) < d.presets.max_presets {
                    d.presets.presets.push(PresetEntry { name, frequency });
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
        w.i32(d.presets.max_presets);
        w.i32(d.presets.presets.len() as i32);
        for p in &d.presets.presets {
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
/// data and are runtime input here: no `ZomboidRadio` port exists yet.
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
        self.presets.presets.clear();
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
        if !self.presets.presets.is_empty() {
            let r = next_range(next, 0, self.presets.presets.len() as i32);
            self.channel = self.presets.presets[r as usize].frequency;
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
