//! `RadioChannel` (and `DynamicRadioChannel`, a channel whose airing
//! broadcast is not restored from the save).

use std::collections::HashMap;

use crate::java::{rand_range, Rng};
use crate::script::{BroadcastId, Broadcasts, RadioScript};
use crate::ChannelCategory;

/// The `RadioData` file a channel came from, as far as the channel needs it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RadioDataRef {
    pub guid: String,
    pub vanilla: bool,
}

/// A line a channel airs on its frequency (`RadioChannel.update` hands it
/// to `ZomboidRadio.SendTransmission`).
#[derive(Clone, Debug, PartialEq)]
pub struct AiredLine {
    pub frequency: i32,
    pub is_tv: bool,
    pub text: String,
    pub codes: String,
    pub r: f32,
    pub g: f32,
    pub b: f32,
    /// The channel is one of the Louisville channels that are scrambled
    /// once `ZomboidRadio.louisvilleObfuscation` is set.
    pub louisville_obfuscate: bool,
}

/// `RadioChannel`.
#[derive(Clone, Debug)]
pub struct RadioChannel {
    guid: String,
    radio_data: Option<RadioDataRef>,
    is_time_synced: bool,
    scripts: HashMap<String, RadioScript>,
    frequency: i32,
    name: String,
    is_tv: bool,
    category: ChannelCategory,
    player_is_listening: bool,
    current_script: Option<String>,
    current_script_loop: i32,
    current_script_max_loops: i32,
    airing_broadcast: Option<BroadcastId>,
    air_counter: f32,
    last_aired_line: String,
    last_broadcast_id: Option<String>,
    air_counter_multiplier: f32,
    louisville_obfuscate: bool,
    pub minmod: f32,
    pub maxmod: f32,
    dynamic: bool,
}

impl RadioChannel {
    /// `new RadioChannel(name, freq, category, guid)`. The original's
    /// constructor without a guid uses `UUID.randomUUID()`; callers pass
    /// their own here.
    pub fn new(name: &str, frequency: i32, category: ChannelCategory, guid: &str) -> Self {
        RadioChannel {
            guid: guid.to_string(),
            radio_data: None,
            is_time_synced: false,
            scripts: HashMap::new(),
            frequency,
            name: name.to_string(),
            is_tv: category == ChannelCategory::Television,
            category,
            player_is_listening: false,
            current_script: None,
            current_script_loop: 1,
            current_script_max_loops: 1,
            airing_broadcast: None,
            air_counter: 0.0,
            last_aired_line: String::new(),
            last_broadcast_id: None,
            air_counter_multiplier: 1.0,
            louisville_obfuscate: false,
            minmod: 1.5,
            maxmod: 5.0,
            dynamic: false,
        }
    }

    /// `new DynamicRadioChannel(name, freq, category, guid)`.
    pub fn new_dynamic(name: &str, frequency: i32, category: ChannelCategory, guid: &str) -> Self {
        let mut c = Self::new(name, frequency, category, guid);
        c.dynamic = true;
        c
    }

    pub fn is_dynamic(&self) -> bool {
        self.dynamic
    }

    /// `getGUID`.
    pub fn guid(&self) -> &str {
        &self.guid
    }

    /// `GetFrequency`.
    pub fn frequency(&self) -> i32 {
        self.frequency
    }

    /// `GetName`.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// `IsTv`.
    pub fn is_tv(&self) -> bool {
        self.is_tv
    }

    /// `GetCategory`.
    pub fn category(&self) -> ChannelCategory {
        self.category
    }

    /// The name of the current script (`getCurrentScript().GetName()`).
    pub fn current_script_name(&self) -> Option<&str> {
        self.current_script.as_deref()
    }

    /// `getCurrentScript`.
    pub fn current_script(&self) -> Option<&RadioScript> {
        self.current_script
            .as_ref()
            .and_then(|n| self.scripts.get(n))
    }

    pub fn current_script_mut(&mut self) -> Option<&mut RadioScript> {
        let n = self.current_script.as_ref()?;
        self.scripts.get_mut(n)
    }

    /// `getAiringBroadcast`.
    pub fn airing_broadcast(&self) -> Option<BroadcastId> {
        self.airing_broadcast
    }

    /// `setAiringBroadcast`.
    pub fn set_airing_broadcast(&mut self, b: Option<BroadcastId>) {
        self.airing_broadcast = b;
    }

    /// `getLastAiredLine`.
    pub fn last_aired_line(&self) -> &str {
        &self.last_aired_line
    }

    pub fn current_script_loop(&self) -> i32 {
        self.current_script_loop
    }

    pub fn current_script_max_loops(&self) -> i32 {
        self.current_script_max_loops
    }

    /// `getLastBroadcastID`.
    pub fn last_broadcast_id(&self) -> Option<&str> {
        self.last_broadcast_id.as_deref()
    }

    pub fn radio_data(&self) -> Option<&RadioDataRef> {
        self.radio_data.as_ref()
    }

    pub fn set_radio_data(&mut self, r: Option<RadioDataRef>) {
        self.radio_data = r;
    }

    pub fn is_time_synced(&self) -> bool {
        self.is_time_synced
    }

    pub fn set_time_synced(&mut self, v: bool) {
        self.is_time_synced = v;
    }

    /// `isVanilla`: true without radio data.
    pub fn is_vanilla(&self) -> bool {
        self.radio_data.as_ref().is_none_or(|r| r.vanilla)
    }

    pub fn louisville_obfuscate(&self) -> bool {
        self.louisville_obfuscate
    }

    pub fn set_louisville_obfuscate(&mut self, b: bool) {
        self.louisville_obfuscate = b;
    }

    /// `GetPlayerIsListening`.
    pub fn player_is_listening(&self) -> bool {
        self.player_is_listening
    }

    pub fn air_counter(&self) -> f32 {
        self.air_counter
    }

    pub fn scripts(&self) -> &HashMap<String, RadioScript> {
        &self.scripts
    }

    /// `AddRadioScript`: refused (`false`) when the name is taken.
    pub fn add_radio_script(&mut self, script: RadioScript) -> bool {
        if self.scripts.contains_key(script.name()) {
            return false;
        }
        self.scripts.insert(script.name().to_string(), script);
        true
    }

    /// `getRadioScript`.
    pub fn radio_script(&self, name: &str) -> Option<&RadioScript> {
        self.scripts.get(name)
    }

    pub fn radio_script_mut(&mut self, name: &str) -> Option<&mut RadioScript> {
        self.scripts.get_mut(name)
    }

    /// `LoadAiringBroadcast(guid, line)`: restores the broadcast that was
    /// airing when the game was saved (a negative line means it had
    /// finished). Does nothing on a dynamic channel.
    pub fn load_airing_broadcast(&mut self, arena: &mut Broadcasts, guid: &str, line: i32) {
        if self.dynamic {
            return;
        }
        let Some(script) = self.current_script_mut() else {
            return;
        };
        let mut airing = script.broadcast_with_id(arena, guid);
        if line < 0 {
            self.last_broadcast_id = Some(guid.to_string());
            airing = None;
        }
        self.airing_broadcast = airing;
        if let Some(b) = airing {
            arena.reset_line_counter(b, true);
            arena.get_mut(b).set_current_line_number(line);
            self.air_counter = 120.0;
            self.player_is_listening = true;
        }
    }

    /// `SetPlayerIsListening`: a listener starts the script's broadcast if
    /// one is due and nothing is airing.
    pub fn set_player_is_listening(&mut self, arena: &mut Broadcasts, listening: bool) {
        self.player_is_listening = listening;
        if self.player_is_listening && self.airing_broadcast.is_none() {
            let Some(script) = self
                .current_script
                .as_ref()
                .and_then(|n| self.scripts.get_mut(n))
            else {
                return;
            };
            self.airing_broadcast = script.valid_air_broadcast(arena);
            if let Some(b) = self.airing_broadcast {
                arena.reset_line_counter(b, true);
            }
            self.air_counter = 0.0;
        }
    }

    /// `setActiveScriptNull`.
    pub fn set_active_script_null(&mut self) {
        self.current_script = None;
        self.airing_broadcast = None;
    }

    /// `setActiveScript(name, day, loop, maxloops)`; `max_loops == -1` rolls
    /// the loop count between the script's bounds.
    pub fn set_active_script(
        &mut self,
        name: &str,
        day: i32,
        loop_: i32,
        max_loops: i32,
        rng: &mut Rng<'_>,
    ) {
        let Some(script) = self.scripts.get_mut(name) else {
            return;
        };
        self.current_script = Some(name.to_string());
        script.reset();
        script.set_start_day_stamp(day);
        self.current_script_loop = loop_;
        let mut max_loops = max_loops;
        if max_loops == -1 {
            let (min, max) = (script.loop_min(), script.loop_max());
            max_loops = if min >= max {
                min
            } else {
                rand_range(rng, min, max)
            };
        }
        self.current_script_max_loops = max_loops;
    }

    fn next_script(&mut self, day: i32, rng: &mut Rng<'_>) {
        let Some(name) = self.current_script.clone() else {
            return;
        };
        if self.current_script_loop < self.current_script_max_loops {
            self.current_script_loop += 1;
            let script = self.scripts.get_mut(&name).unwrap();
            script.reset();
            script.set_start_day_stamp(day);
        } else {
            let exit = self.scripts[&name].next_script(rng);
            self.current_script = None;
            if let Some(e) = exit {
                self.set_active_script(&e.script_name, day + e.start_delay, 1, -1, rng);
            }
        }
    }

    /// `UpdateScripts(timestamp, day)`: moves to the next script once the
    /// current one has run out of broadcasts.
    pub fn update_scripts(
        &mut self,
        arena: &Broadcasts,
        timestamp: i32,
        day: i32,
        rng: &mut Rng<'_>,
    ) {
        self.player_is_listening = false;
        let ended = match self.current_script_mut() {
            Some(s) => !s.update_script(arena, timestamp),
            None => false,
        };
        if ended {
            self.next_script(day + 1, rng);
        }
    }

    /// `update()`: counts the airing broadcast down by `1.25 * multiplier`
    /// and airs its next line through `air` when the counter runs out.
    pub fn update(
        &mut self,
        arena: &mut Broadcasts,
        multiplier: f32,
        air: &mut dyn FnMut(AiredLine),
    ) {
        let Some(b) = self.airing_broadcast else {
            return;
        };
        self.air_counter -= 1.25 * multiplier;
        if self.air_counter >= 0.0 {
            return;
        }
        let Some(line) = arena.next_line(b, true) else {
            self.last_broadcast_id = Some(arena.get(b).id().to_string());
            self.airing_broadcast = None;
            self.player_is_listening = false;
            return;
        };
        self.last_aired_line = line.text().to_string();
        air(AiredLine {
            frequency: self.frequency,
            is_tv: self.is_tv,
            text: line.text().to_string(),
            codes: line.effects().to_string(),
            r: line.r(),
            g: line.g(),
            b: line.b(),
            louisville_obfuscate: self.louisville_obfuscate,
        });
        if line.is_custom_air_time() {
            self.air_counter = line.air_time() * 60.0;
        } else {
            self.air_counter = line.text().encode_utf16().count() as f32 / 10.0 * 60.0;
            if self.air_counter < 60.0 * self.minmod {
                self.air_counter = 60.0 * self.minmod;
            } else if self.air_counter > 60.0 * self.maxmod {
                self.air_counter = 60.0 * self.maxmod;
            }
            self.air_counter *= self.air_counter_multiplier;
        }
    }

    pub fn air_counter_multiplier(&self) -> f32 {
        self.air_counter_multiplier
    }

    /// `setAirCounterMultiplier`, clamped to `[0.1, 10]`.
    pub fn set_air_counter_multiplier(&mut self, m: f32) {
        self.air_counter_multiplier = m.clamp(0.1, 10.0);
    }
}
