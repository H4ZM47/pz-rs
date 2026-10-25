//! `RadioScriptManager` and the channel name registry `ZomboidRadio`
//! keeps beside it (`channelNames`, `categorizedChannels`,
//! `knownFrequencies`).

use indexmap::IndexMap;

use crate::channel::{AiredLine, RadioChannel};
use crate::java::{
    is_blank, line_separator, parse_int, rand_range, split_char, trim, JavaHashMap, Rng,
};
use crate::script::Broadcasts;

/// The channel names `ZomboidRadio` keeps for the device presets and the
/// UI, by frequency and by category name.
#[derive(Clone, Debug, Default)]
pub struct ChannelNames {
    channel_names: JavaHashMap<i32, String>,
    categorized: JavaHashMap<String, JavaHashMap<i32, String>>,
    known_frequencies: Vec<i32>,
}

impl ChannelNames {
    /// `addChannelName(name, frequency, category, overwrite)`.
    pub fn add(&mut self, name: &str, frequency: i32, category: &str, overwrite: bool) {
        if overwrite || !self.channel_names.contains_key(&frequency) {
            if !self.categorized.contains_key(category) {
                self.categorized
                    .insert(category.to_string(), JavaHashMap::new());
            }
            self.categorized
                .get_mut(category)
                .unwrap()
                .insert(frequency, name.to_string());
            self.channel_names.insert(frequency, name.to_string());
            self.known_frequencies.push(frequency);
        }
    }

    /// `removeChannelName(frequency)` (the frequency stays "known").
    pub fn remove(&mut self, frequency: i32) {
        if self.channel_names.remove(&frequency).is_some() {
            for m in self.categorized.values_mut() {
                m.remove(&frequency);
            }
        }
    }

    /// `GetChannelList(category)`: `(frequency, name)` in the original's
    /// iteration order, `None` for an unknown category.
    pub fn channel_list(&self, category: &str) -> Option<Vec<(i32, String)>> {
        self.categorized
            .get(category)
            .map(|m| m.iter().into_iter().map(|(f, n)| (*f, n.clone())).collect())
    }

    /// `getFullChannelList`: every category in iteration order.
    pub fn full_channel_list(&self) -> Vec<(String, Vec<(i32, String)>)> {
        self.categorized
            .iter()
            .into_iter()
            .map(|(c, m)| {
                (
                    c.clone(),
                    m.iter().into_iter().map(|(f, n)| (*f, n.clone())).collect(),
                )
            })
            .collect()
    }

    /// `getChannelName(frequency)`.
    pub fn channel_name(&self, frequency: i32) -> Option<&str> {
        self.channel_names.get(&frequency).map(String::as_str)
    }

    pub fn known_frequencies(&self) -> &[i32] {
        &self.known_frequencies
    }

    /// `getRandomFrequency(min, max)`: a multiple of 200 no channel uses.
    pub fn random_frequency(&self, min: i32, max: i32, rng: &mut Rng<'_>) -> i32 {
        loop {
            let f = rand_range(rng, min, max) / 200 * 200;
            if !self.known_frequencies.contains(&f) {
                return f;
            }
        }
    }
}

/// A malformed radio save line (`NumberFormatException` in the original).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LoadError(pub String);

/// `RadioScriptManager`.
#[derive(Clone, Debug, Default)]
pub struct RadioScriptManager {
    channels: IndexMap<i32, RadioChannel>,
    current_time_stamp: i32,
    /// Every broadcast of every channel and advert.
    pub broadcasts: Broadcasts,
}

impl RadioScriptManager {
    pub fn new() -> Self {
        Self::default()
    }

    /// `getChannels`: by frequency, in insertion order.
    pub fn channels(&self) -> &IndexMap<i32, RadioChannel> {
        &self.channels
    }

    pub fn channel_mut(&mut self, frequency: i32) -> Option<&mut RadioChannel> {
        self.channels.get_mut(&frequency)
    }

    /// Splits the manager into its channels and the broadcast arena.
    pub fn parts_mut(&mut self) -> (&mut IndexMap<i32, RadioChannel>, &mut Broadcasts) {
        (&mut self.channels, &mut self.broadcasts)
    }

    /// `getRadioChannel(uuid)`.
    pub fn radio_channel(&self, guid: &str) -> Option<&RadioChannel> {
        self.channels.values().find(|c| c.guid() == guid)
    }

    pub fn current_time_stamp(&self) -> i32 {
        self.current_time_stamp
    }

    /// `simulateScriptsUntil(days, force)`.
    pub fn simulate_scripts_until(&mut self, days: i32, force: bool, rng: &mut Rng<'_>) {
        let freqs: Vec<i32> = self.channels.values().map(|c| c.frequency()).collect();
        for f in freqs {
            self.simulate_channel_until(f, days, force, rng);
        }
    }

    /// `simulateChannelUntil(frequency, days, force)`. As in the original,
    /// each simulated day calls `UpdateScripts(currentTimeStamp, i * 1440)`:
    /// the stamp stays the manager's current one and the day argument is in
    /// minutes.
    pub fn simulate_channel_until(
        &mut self,
        frequency: i32,
        days: i32,
        force: bool,
        rng: &mut Rng<'_>,
    ) {
        let ts = self.current_time_stamp;
        let Some(chan) = self.channels.get_mut(&frequency) else {
            return;
        };
        if chan.is_time_synced() && !force {
            return;
        }
        for i in 0..days {
            chan.update_scripts(&self.broadcasts, ts, i.wrapping_mul(24 * 60), rng);
        }
        chan.set_time_synced(true);
    }

    /// `PlayerListensChannel(frequency, mode, sourceIsTV)`.
    pub fn player_listens_channel(&mut self, frequency: i32, mode: bool, source_is_tv: bool) {
        if let Some(c) = self.channels.get_mut(&frequency) {
            if c.is_tv() == source_is_tv {
                c.set_player_is_listening(&mut self.broadcasts, mode);
            }
        }
    }

    /// `AddChannel(channel, overwrite)`: `false` (logged by the original)
    /// when the frequency is taken and `overwrite` is not set.
    pub fn add_channel(
        &mut self,
        channel: RadioChannel,
        overwrite: bool,
        names: &mut ChannelNames,
    ) -> bool {
        if !overwrite && self.channels.contains_key(&channel.frequency()) {
            return false;
        }
        names.add(
            channel.name(),
            channel.frequency(),
            channel.category().name(),
            overwrite,
        );
        self.channels.insert(channel.frequency(), channel);
        true
    }

    /// `RemoveChannel(frequency)`.
    pub fn remove_channel(&mut self, frequency: i32, names: &mut ChannelNames) {
        if self.channels.shift_remove(&frequency).is_some() {
            names.remove(frequency);
        }
    }

    /// `UpdateScripts(day, hour, mins)`.
    pub fn update_scripts(&mut self, day: i32, hour: i32, mins: i32, rng: &mut Rng<'_>) {
        self.current_time_stamp = day
            .wrapping_mul(24 * 60)
            .wrapping_add(hour * 60)
            .wrapping_add(mins);
        let ts = self.current_time_stamp;
        for c in self.channels.values_mut() {
            c.update_scripts(&self.broadcasts, ts, day, rng);
        }
    }

    /// `update()`: every channel, in order; aired lines go to `air` as they
    /// are aired.
    pub fn update(&mut self, multiplier: f32, air: &mut dyn FnMut(AiredLine)) {
        for c in self.channels.values_mut() {
            c.update(&mut self.broadcasts, multiplier, air);
        }
    }

    /// `Save(writer)`: one line per channel,
    /// `freq,loop,maxloops,script,startday,broadcast,line`.
    pub fn save(&self, out: &mut String) {
        for (freq, c) in &self.channels {
            let script = c.current_script();
            let airing = c.airing_broadcast().map(|b| self.broadcasts.get(b));
            let mut bcid = airing.map_or("none", |b| b.id());
            if airing.is_none() {
                if let Some(last) = c.last_broadcast_id() {
                    bcid = last;
                }
            }
            let fields = [
                freq.to_string(),
                c.current_script_loop().to_string(),
                c.current_script_max_loops().to_string(),
                script.map_or("none".to_string(), |s| s.name().to_string()),
                script.map_or("-1".to_string(), |s| s.start_day().to_string()),
                bcid.to_string(),
                airing.map_or("-1".to_string(), |b| b.current_line_number().to_string()),
            ];
            out.push_str(&fields.join(","));
            out.push_str(line_separator());
        }
    }

    /// `Load(channelLines)`. Stops at the first malformed number, keeping
    /// what the earlier lines restored.
    pub fn load(&mut self, lines: &[String], rng: &mut Rng<'_>) -> Result<(), LoadError> {
        let num =
            |s: &str| parse_int(s).ok_or_else(|| LoadError(format!("For input string: \"{s}\"")));
        for line in lines {
            if is_blank(line) {
                continue;
            }
            let line = trim(line);
            let s = split_char(line, ',');
            let first = s
                .first()
                .ok_or_else(|| LoadError("Index 0 out of bounds for length 0".into()))?;
            let freq = num(first)?;
            let Some(chan) = self.channels.get_mut(&freq) else {
                continue;
            };
            chan.set_time_synced(true);
            if s.len() != 7 {
                continue;
            }
            if s[3] != "none" {
                let (day, loop_, max) = (num(s[4])?, num(s[1])?, num(s[2])?);
                chan.set_active_script(s[3], day, loop_, max, rng);
            }
            if s[5] == "none" {
                continue;
            }
            let line_no = num(s[6])?;
            chan.load_airing_broadcast(&mut self.broadcasts, s[5], line_no);
        }
        Ok(())
    }

    /// `getChannelsList`.
    pub fn channels_list(&self) -> Vec<&RadioChannel> {
        self.channels.values().collect()
    }
}
