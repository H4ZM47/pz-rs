//! `RadioLine`, `RadioBroadCast` and `RadioScript`.
//!
//! Broadcasts live in a [`Broadcasts`] arena and are referred to by
//! [`BroadcastId`]: an advert broadcast is shared by every broadcast that
//! uses it as a pre or post segment, and its line counter with it, as in
//! the original.

use crate::java::{rand_next, Rng};

/// `RadioLine`.
#[derive(Clone, Debug, PartialEq)]
pub struct RadioLine {
    r: f32,
    g: f32,
    b: f32,
    text: String,
    effects: String,
    air_time: f32,
}

impl RadioLine {
    /// `new RadioLine(txt, r, g, b, fx)`: a `null` text keeps the
    /// placeholder, a `null` effects string is empty.
    pub fn new(text: Option<&str>, r: f32, g: f32, b: f32, effects: Option<&str>) -> Self {
        RadioLine {
            r,
            g,
            b,
            text: text.unwrap_or("<!text missing!>").to_string(),
            effects: effects.unwrap_or("").to_string(),
            air_time: -1.0,
        }
    }

    pub fn r(&self) -> f32 {
        self.r
    }

    pub fn g(&self) -> f32 {
        self.g
    }

    pub fn b(&self) -> f32 {
        self.b
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn set_text(&mut self, text: String) {
        self.text = text;
    }

    /// `getEffectsString`.
    pub fn effects(&self) -> &str {
        &self.effects
    }

    pub fn is_custom_air_time(&self) -> bool {
        self.air_time > 0.0
    }

    pub fn air_time(&self) -> f32 {
        self.air_time
    }

    pub fn set_air_time(&mut self, air_time: f32) {
        self.air_time = air_time;
    }

    /// `RadioBroadCast.pauseLine`, the grey `~` aired around segments.
    pub fn pause() -> Self {
        RadioLine::new(Some("~"), 0.5, 0.5, 0.5, None)
    }
}

/// Index of a broadcast in [`Broadcasts`].
pub type BroadcastId = usize;

/// `RadioBroadCast`.
#[derive(Clone, Debug)]
pub struct RadioBroadCast {
    id: String,
    start_stamp: i32,
    end_stamp: i32,
    lines: Vec<RadioLine>,
    line_count: i32,
    pre_segment: Option<BroadcastId>,
    post_segment: Option<BroadcastId>,
    has_done_pre_segment: bool,
    has_done_post_pause: bool,
}

impl RadioBroadCast {
    pub fn new(id: &str, start_stamp: i32, end_stamp: i32) -> Self {
        RadioBroadCast {
            id: id.to_string(),
            start_stamp,
            end_stamp,
            lines: Vec::new(),
            line_count: 0,
            pre_segment: None,
            post_segment: None,
            has_done_pre_segment: false,
            has_done_post_pause: false,
        }
    }

    /// `getID`.
    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn start_stamp(&self) -> i32 {
        self.start_stamp
    }

    pub fn end_stamp(&self) -> i32 {
        self.end_stamp
    }

    pub fn lines(&self) -> &[RadioLine] {
        &self.lines
    }

    pub fn pre_segment(&self) -> Option<BroadcastId> {
        self.pre_segment
    }

    pub fn post_segment(&self) -> Option<BroadcastId> {
        self.post_segment
    }

    pub fn set_pre_segment(&mut self, b: Option<BroadcastId>) {
        self.pre_segment = b;
    }

    pub fn set_post_segment(&mut self, b: Option<BroadcastId>) {
        self.post_segment = b;
    }

    /// `AddRadioLine`.
    pub fn add_line(&mut self, line: RadioLine) {
        self.lines.push(line);
    }

    /// `getCurrentLineNumber`.
    pub fn current_line_number(&self) -> i32 {
        self.line_count
    }

    /// `setCurrentLineNumber` (negative clamps to 0).
    pub fn set_current_line_number(&mut self, n: i32) {
        self.line_count = n.max(0);
    }

    /// `getCurrentLine`.
    pub fn current_line(&self) -> Option<&RadioLine> {
        usize::try_from(self.line_count)
            .ok()
            .and_then(|i| self.lines.get(i))
    }

    /// `PeekNextLineText`.
    pub fn peek_next_line_text(&self) -> &str {
        match self.current_line() {
            Some(l) => l.text(),
            None => "None",
        }
    }
}

/// Every broadcast loaded, adverts included.
#[derive(Clone, Debug, Default)]
pub struct Broadcasts {
    list: Vec<RadioBroadCast>,
}

impl Broadcasts {
    pub fn add(&mut self, b: RadioBroadCast) -> BroadcastId {
        self.list.push(b);
        self.list.len() - 1
    }

    pub fn get(&self, id: BroadcastId) -> &RadioBroadCast {
        &self.list[id]
    }

    pub fn get_mut(&mut self, id: BroadcastId) -> &mut RadioBroadCast {
        &mut self.list[id]
    }

    pub fn len(&self) -> usize {
        self.list.len()
    }

    pub fn is_empty(&self) -> bool {
        self.list.is_empty()
    }

    /// `resetLineCounter(doChildren)`. The segment flags are left alone,
    /// as in the original.
    pub fn reset_line_counter(&mut self, id: BroadcastId, do_children: bool) {
        let b = &mut self.list[id];
        b.line_count = 0;
        if do_children {
            let (pre, post) = (b.pre_segment, b.post_segment);
            if let Some(p) = pre {
                self.reset_line_counter(p, false);
            }
            if let Some(p) = post {
                self.reset_line_counter(p, false);
            }
        }
    }

    /// `getNextLine(doChildren)`: the pre segment's lines and a pause, the
    /// broadcast's own lines, then a pause and the post segment's lines.
    pub fn next_line(&mut self, id: BroadcastId, do_children: bool) -> Option<RadioLine> {
        let b = &self.list[id];
        if do_children && !b.has_done_pre_segment && b.line_count == 0 {
            if let Some(pre) = b.pre_segment {
                if let Some(r) = self.next_line(pre, true) {
                    return Some(r);
                }
                self.list[id].has_done_pre_segment = true;
                return Some(RadioLine::pause());
            }
        }
        let b = &self.list[id];
        let r = b.current_line().cloned();
        if do_children && r.is_none() {
            if let Some(post) = b.post_segment {
                if !b.has_done_post_pause {
                    self.list[id].has_done_post_pause = true;
                    return Some(RadioLine::pause());
                }
                return self.next_line(post, true);
            }
        }
        self.list[id].line_count += 1;
        r
    }
}

/// `RadioScript.ExitOption`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExitOption {
    pub script_name: String,
    pub chance: i32,
    pub start_delay: i32,
}

/// Why `AddBroadcast` refused a broadcast (the original logs it).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AddBroadcastError {
    Timestamps,
}

/// `RadioScript`.
#[derive(Clone, Debug)]
pub struct RadioScript {
    broadcasts: Vec<BroadcastId>,
    exit_options: Vec<ExitOption>,
    guid: String,
    name: String,
    start_day: i32,
    start_day_stamp: i32,
    loop_min: i32,
    loop_max: i32,
    internal_stamp: i32,
    current_broadcast: Option<BroadcastId>,
    current_has_aired: bool,
}

impl RadioScript {
    pub fn new(name: &str, loop_min: i32, loop_max: i32, guid: &str) -> Self {
        RadioScript {
            broadcasts: Vec::new(),
            exit_options: Vec::new(),
            guid: guid.to_string(),
            name: name.to_string(),
            start_day: 0,
            start_day_stamp: 0,
            loop_min,
            loop_max,
            internal_stamp: 0,
            current_broadcast: None,
            current_has_aired: false,
        }
    }

    /// `GetGUID`.
    pub fn guid(&self) -> &str {
        &self.guid
    }

    /// `GetName`.
    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn start_day_stamp(&self) -> i32 {
        self.start_day_stamp
    }

    pub fn start_day(&self) -> i32 {
        self.start_day
    }

    pub fn loop_min(&self) -> i32 {
        self.loop_min
    }

    pub fn loop_max(&self) -> i32 {
        self.loop_max
    }

    pub fn current_broadcast(&self) -> Option<BroadcastId> {
        self.current_broadcast
    }

    /// `getBroadcastList`.
    pub fn broadcasts(&self) -> &[BroadcastId] {
        &self.broadcasts
    }

    pub fn exit_options(&self) -> &[ExitOption] {
        &self.exit_options
    }

    pub fn clear_exit_options(&mut self) {
        self.exit_options.clear();
    }

    /// `setStartDayStamp(day)`.
    pub fn set_start_day_stamp(&mut self, day: i32) {
        self.start_day = day;
        self.start_day_stamp = day.wrapping_mul(24 * 60);
    }

    /// `getValidAirBroadcast`: the current broadcast, once, while it is on.
    pub fn valid_air_broadcast(&mut self, arena: &Broadcasts) -> Option<BroadcastId> {
        if let Some(c) = self.current_broadcast {
            let b = arena.get(c);
            if !self.current_has_aired
                && self.internal_stamp >= b.start_stamp
                && self.internal_stamp < b.end_stamp
            {
                self.current_has_aired = true;
                return Some(c);
            }
        }
        None
    }

    /// `Reset`.
    pub fn reset(&mut self) {
        self.current_broadcast = None;
        self.current_has_aired = false;
    }

    fn next_broadcast(&mut self, arena: &Broadcasts) -> Option<BroadcastId> {
        match self.current_broadcast {
            Some(c) if arena.get(c).end_stamp > self.internal_stamp => Some(c),
            _ => {
                for &id in &self.broadcasts {
                    if arena.get(id).end_stamp > self.internal_stamp {
                        self.current_has_aired = false;
                        return Some(id);
                    }
                }
                None
            }
        }
    }

    /// `getBroadcastWithID`: also makes it the current, already aired,
    /// broadcast.
    pub fn broadcast_with_id(&mut self, arena: &Broadcasts, guid: &str) -> Option<BroadcastId> {
        let found = self
            .broadcasts
            .iter()
            .copied()
            .find(|&id| arena.get(id).id == guid)?;
        self.current_broadcast = Some(found);
        self.current_has_aired = true;
        Some(found)
    }

    /// `UpdateScript(timeStamp)`: `false` once every broadcast has ended.
    pub fn update_script(&mut self, arena: &Broadcasts, time_stamp: i32) -> bool {
        self.internal_stamp = time_stamp.wrapping_sub(self.start_day_stamp);
        self.current_broadcast = self.next_broadcast(arena);
        self.current_broadcast.is_some()
    }

    /// `getNextScript`: rolls `Rand.Next(100)` against the exit chances.
    pub fn next_script(&self, rng: &mut Rng<'_>) -> Option<ExitOption> {
        let mut chance_start = 0;
        let roll = rand_next(rng, 100);
        for e in &self.exit_options {
            if roll >= chance_start && roll < chance_start + e.chance {
                return Some(e.clone());
            }
            chance_start += e.chance;
        }
        None
    }

    /// `AddBroadcast(broadcast, ignoreTimestamps)`: unless timestamps are
    /// ignored, a broadcast must start at or after 0, end after it starts
    /// and not overlap the previous one.
    pub fn add_broadcast(
        &mut self,
        arena: &Broadcasts,
        id: BroadcastId,
        ignore_timestamps: bool,
    ) -> Result<(), AddBroadcastError> {
        if ignore_timestamps {
            self.broadcasts.push(id);
            return Ok(());
        }
        let b = arena.get(id);
        if b.start_stamp >= 0 && b.end_stamp > b.start_stamp {
            let fits = match self.broadcasts.last() {
                None => true,
                Some(&last) => arena.get(last).end_stamp <= b.start_stamp,
            };
            if fits {
                self.broadcasts.push(id);
                return Ok(());
            }
        }
        Err(AddBroadcastError::Timestamps)
    }

    /// `AddExitOption`: refused (`false`) when the chances would pass 100.
    pub fn add_exit_option(&mut self, script_name: &str, chance: i32, start_delay: i32) -> bool {
        let total = chance + self.exit_options.iter().map(|e| e.chance).sum::<i32>();
        if total <= 100 {
            self.exit_options.push(ExitOption {
                script_name: script_name.to_string(),
                chance,
                start_delay,
            });
            true
        } else {
            false
        }
    }

    /// `getValidAirBroadcastDebug`.
    pub fn valid_air_broadcast_debug(&self, arena: &Broadcasts) -> Option<BroadcastId> {
        match self.current_broadcast {
            Some(c) if arena.get(c).end_stamp > self.internal_stamp => Some(c),
            _ => self
                .broadcasts
                .iter()
                .copied()
                .find(|&id| arena.get(id).end_stamp > self.internal_stamp),
        }
    }
}
