//! The small files `GameWindow.save` writes after `map_sand.bin`, and
//! their loads in `IsoWorld.init`:
//!
//! | class | file |
//! |---|---|
//! | `InstanceTracker` | `iTrack.bin` (and `ItemTracker.log` in the cache folder) |
//! | `MetaTracker` | `metadata.bin` (`IsoWorld.helicopter`, `AmbientStreamManager`'s alarms) |
//! | `StatisticsManager` | `statistics.bin` |
//!
//! (`WorldGenParams.save`, `map_worldgen.bin`, is `pz_worldgen::params`.)
//!
//! The original keeps the counters in `java.util.HashMap`s and Trove
//! `TObjectIntHashMap`s and writes them in their iteration order; both are
//! reproduced ([`JavaHashOrder`], [`TroveObjectIntMap`]).

use std::io;
use std::path::Path;

use pz_kahlua::java_io::{JavaReader, JavaWrite};

/// `IsoWorld.getWorldVersion()` in this build.
pub const WORLD_VERSION: i32 = 249;

/// `String.hashCode()`.
pub fn string_hash(s: &str) -> i32 {
    s.encode_utf16()
        .fold(0i32, |h, c| h.wrapping_mul(31).wrapping_add(c as i32))
}

fn bad(msg: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, msg.into())
}

/// The key order of a `java.util.HashMap<String, V>` (default capacity and
/// load factor; `clear()` keeps the table): keys in insertion order (a
/// `put` of a present key keeps its place) and the most keys the map has
/// held. Treeified bins are not modelled.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct JavaHashOrder {
    keys: Vec<String>,
    peak: usize,
}

impl JavaHashOrder {
    /// Whether `key` is present.
    pub fn contains(&self, key: &str) -> bool {
        self.keys.iter().any(|k| k == key)
    }
    /// `put(key, ..)`: a new key goes last.
    pub fn insert(&mut self, key: &str) {
        if !self.contains(key) {
            self.keys.push(key.to_string());
            self.peak = self.peak.max(self.keys.len());
        }
    }
    /// `clear()`.
    pub fn clear(&mut self) {
        self.keys.clear();
    }
    pub fn len(&self) -> usize {
        self.keys.len()
    }
    pub fn is_empty(&self) -> bool {
        self.keys.is_empty()
    }
    /// The keys in the map's iteration order.
    pub fn iter_order(&self) -> Vec<&str> {
        let mut cap: usize = 16;
        while self.peak as f64 > cap as f64 * 0.75 {
            cap *= 2;
        }
        let mut idx: Vec<(usize, usize)> = self
            .keys
            .iter()
            .enumerate()
            .map(|(i, k)| {
                let h = string_hash(k);
                let spread = h ^ ((h as u32) >> 16) as i32;
                ((spread as u32 as usize) & (cap - 1), i)
            })
            .collect();
        idx.sort();
        idx.into_iter()
            .map(|(_, i)| self.keys[i].as_str())
            .collect()
    }
}

/// `gnu.trove.map.hash.TObjectIntHashMap<String>` with the default
/// capacity (10) and load factor (0.5), never removed from: open
/// addressing with double hashing, grown to the next prime above twice the
/// capacity, iterated from the last slot to the first.
#[derive(Clone, Debug, PartialEq)]
pub struct TroveObjectIntMap {
    slots: Vec<Option<(String, i32)>>,
    size: usize,
    max_size: usize,
}

impl Default for TroveObjectIntMap {
    fn default() -> Self {
        // `setUp(fastCeil(10 / 0.5f))`: `PrimeFinder.nextPrime(20)`.
        Self::with_capacity(23)
    }
}

fn is_prime(n: usize) -> bool {
    if n < 2 {
        return false;
    }
    let mut i = 2;
    while i * i <= n {
        if n.is_multiple_of(i) {
            return false;
        }
        i += 1;
    }
    true
}

impl TroveObjectIntMap {
    fn with_capacity(capacity: usize) -> Self {
        TroveObjectIntMap {
            slots: vec![None; capacity],
            size: 0,
            // `computeMaxSize`: `min(capacity - 1, (int)(capacity * 0.5f))`.
            max_size: (capacity - 1).min((capacity as f32 * 0.5) as usize),
        }
    }

    pub fn len(&self) -> usize {
        self.size
    }
    pub fn is_empty(&self) -> bool {
        self.size == 0
    }

    /// `insertKey`: the slot of `key`, and whether it was free.
    fn slot(&self, key: &str) -> (usize, bool) {
        let len = self.slots.len();
        let hash = (string_hash(key) & i32::MAX) as usize;
        let mut index = hash % len;
        match &self.slots[index] {
            None => return (index, true),
            Some((k, _)) if k == key => return (index, false),
            _ => {}
        }
        let probe = 1 + hash % (len - 2);
        loop {
            index = (index + len - probe % len) % len;
            match &self.slots[index] {
                None => return (index, true),
                Some((k, _)) if k == key => return (index, false),
                _ => {}
            }
        }
    }

    /// `get(key)` (`0`, the no-entry value, when absent).
    pub fn get(&self, key: &str) -> i32 {
        match self.slot(key) {
            (i, false) => self.slots[i].as_ref().map_or(0, |e| e.1),
            _ => 0,
        }
    }

    /// `put(key, value)`.
    pub fn put(&mut self, key: &str, value: i32) {
        let (i, free) = self.slot(key);
        if !free {
            self.slots[i].as_mut().expect("present").1 = value;
            return;
        }
        self.slots[i] = Some((key.to_string(), value));
        self.size += 1;
        // `postInsertHook`: without removals `_free` reaches 0 only after
        // `_size` passed `_maxSize`.
        if self.size > self.max_size {
            let mut cap = self.slots.len() << 1;
            while !is_prime(cap) {
                cap += 1;
            }
            self.rehash(cap);
        }
    }

    fn rehash(&mut self, capacity: usize) {
        let old = std::mem::replace(self, Self::with_capacity(capacity));
        self.size = old.size;
        for e in old.slots.into_iter().rev().flatten() {
            let (i, _) = self.slot(&e.0);
            self.slots[i] = Some(e);
        }
    }

    /// `forEachEntry` order: from the last slot to the first.
    pub fn entries(&self) -> impl Iterator<Item = (&str, i32)> {
        self.slots
            .iter()
            .rev()
            .flatten()
            .map(|(k, v)| (k.as_str(), *v))
    }
}

/// `InstanceTracker`: counters by group and key.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct InstanceTracker {
    order: JavaHashOrder,
    groups: Vec<(String, TroveObjectIntMap)>,
}

/// `InstanceTracker.ITEMS`.
pub const ITEMS: &str = "Item Spawns";
/// `InstanceTracker.CONTAINERS`.
pub const CONTAINERS: &str = "Container Rolls";
/// `InstanceTracker.STATS`.
pub const STATS: &str = "Stats";

impl InstanceTracker {
    fn group(&self, group: &str) -> Option<&TroveObjectIntMap> {
        self.groups.iter().find(|(g, _)| g == group).map(|(_, m)| m)
    }

    fn group_or_new(&mut self, group: &str) -> &mut TroveObjectIntMap {
        if self.group(group).is_none() {
            self.order.insert(group);
            self.groups
                .push((group.to_string(), TroveObjectIntMap::default()));
        }
        &mut self
            .groups
            .iter_mut()
            .find(|(g, _)| g == group)
            .expect("just added")
            .1
    }

    /// `get(group, key)`.
    pub fn get(&self, group: &str, key: &str) -> i32 {
        self.group(group).map_or(0, |m| m.get(key))
    }

    /// `set(group, key, value)`: a blank key is ignored.
    pub fn set(&mut self, group: &str, key: &str, value: i32) {
        if key.trim_matches(char::is_whitespace).is_empty() {
            return;
        }
        self.group_or_new(group).put(key, value);
    }

    /// `adj(group, key, value)` (the sum wraps before the no-op clamp).
    pub fn adj(&mut self, group: &str, key: &str, value: i32) {
        let m = self.group_or_new(group);
        let v = m.get(key).wrapping_add(value);
        m.put(key, v);
    }

    /// `inc(group, key)`.
    pub fn inc(&mut self, group: &str, key: &str) {
        self.adj(group, key, 1);
    }

    /// `dec(group, key)`.
    pub fn dec(&mut self, group: &str, key: &str) {
        self.adj(group, key, -1);
    }

    /// The groups in the `HashMap`'s order.
    pub fn groups(&self) -> Vec<(&str, &TroveObjectIntMap)> {
        self.order
            .iter_order()
            .into_iter()
            .map(|g| (g, self.group(g).expect("ordered key")))
            .collect()
    }

    /// The bytes `save()` writes to `iTrack.bin`: the world version, the
    /// group count, then each group's name, entry count and entries.
    pub fn save_bytes(&self) -> Vec<u8> {
        let mut out = Vec::new();
        out.put_i32(WORLD_VERSION);
        out.put_i32(self.groups.len() as i32);
        for (group, m) in self.groups() {
            out.write_string_utf(Some(group));
            out.put_i32(m.len() as i32);
            for (k, v) in m.entries() {
                out.write_string_utf(Some(k));
                out.put_i32(v);
            }
        }
        out
    }

    /// `load()` from the bytes of `iTrack.bin`: the groups are cleared
    /// first (the `HashMap` keeps its table); each group read replaces any
    /// group of that name with a new map. A failure is returned after the
    /// groups read so far were kept (the original logs it).
    pub fn load_bytes(&mut self, data: &[u8]) -> io::Result<()> {
        self.order.clear();
        self.groups.clear();
        let mut r = JavaReader::new(data);
        let _world_version = r.get_i32()?;
        let n = r.get_i32()?;
        for _ in 0..n.max(0) {
            let group = r.read_string_utf()?;
            let size = r.get_i32()?;
            match self.groups.iter_mut().find(|(g, _)| *g == group) {
                Some(e) => e.1 = TroveObjectIntMap::default(),
                None => {
                    self.order.insert(&group);
                    self.groups
                        .push((group.clone(), TroveObjectIntMap::default()));
                }
            }
            for _ in 0..size.max(0) {
                let key = r.read_string_utf()?;
                let count = r.get_i32()?;
                self.set(&group, &key, count);
            }
        }
        Ok(())
    }

    /// `InstanceTracker.save()`: nothing with saving off.
    pub fn save_file(&self, save_dir: &Path, no_save: bool) -> io::Result<()> {
        if no_save {
            return Ok(());
        }
        std::fs::write(save_dir.join("iTrack.bin"), self.save_bytes())
    }

    /// `InstanceTracker.load()`: the groups are cleared; nothing more with
    /// saving off or no file.
    pub fn load_file(&mut self, save_dir: &Path, no_save: bool) -> io::Result<()> {
        self.order.clear();
        self.groups.clear();
        if no_save {
            return Ok(());
        }
        match std::fs::read(save_dir.join("iTrack.bin")) {
            Ok(d) => self.load_bytes(&d),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e),
        }
    }

    /// `exportGroup(group, Format.TEXT, Sort.KEY)`: empty for a missing
    /// group.
    pub fn export_group_text(&self, group: &str) -> String {
        let Some(m) = self.group(group) else {
            return String::new();
        };
        let mut s = format!("[{group}] ; {} entries\n", m.len());
        let mut keys: Vec<&str> = m.entries().map(|(k, _)| k).collect();
        // `String.compareTo`: UTF-16 code units.
        keys.sort_by(|a, b| a.encode_utf16().cmp(b.encode_utf16()));
        for k in keys {
            s.push_str(&format!("{k} = {}\n", m.get(k)));
        }
        s.push('\n');
        s
    }

    /// The `ItemTracker.log` that `save()` writes to the cache folder
    /// (`exportFile(List.of(ITEMS, CONTAINERS), "ItemTracker.log", TEXT,
    /// KEY)`).
    pub fn item_tracker_log(&self) -> String {
        let mut s = self.export_group_text(ITEMS);
        s.push_str(&self.export_group_text(CONTAINERS));
        s
    }
}

/// `Helicopter.State`.
pub const HELICOPTER_STATES: usize = 4;

/// The saved fields of `IsoWorld.helicopter`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct HelicopterSave {
    pub active: bool,
    /// `state.ordinal()` (`Arriving`, `Hovering`, `Searching`, `Leaving`;
    /// `null` is saved as 0).
    pub state: i32,
    pub x: f32,
    pub y: f32,
}

/// An `AmbientStreamManager` house alarm (`Alarm.save`).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct AlarmSave {
    pub x: i32,
    pub y: i32,
    pub end_game_time: f32,
}

/// `metadata.bin`: the world version, `helicopter.save`, then
/// `AmbientStreamManager.save` (a short count and the alarms).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MetaTracker {
    pub helicopter: HelicopterSave,
    pub alarms: Vec<AlarmSave>,
}

impl MetaTracker {
    /// The bytes `MetaTracker.save()` writes.
    pub fn save_bytes(&self) -> Vec<u8> {
        let mut out = Vec::new();
        out.put_i32(WORLD_VERSION);
        out.put_bool(self.helicopter.active);
        out.put_i32(self.helicopter.state);
        out.put_f32(self.helicopter.x);
        out.put_f32(self.helicopter.y);
        out.put_i16(self.alarms.len() as i16);
        for a in &self.alarms {
            out.put_i32(a.x);
            out.put_i32(a.y);
            out.put_f32(a.end_game_time);
        }
        out
    }

    /// `MetaTracker.load()` from the bytes: the helicopter's fields (an
    /// unknown state is `ArrayIndexOutOfBoundsException`), then the alarms
    /// are appended to those there are. `Helicopter.load` aims an active
    /// helicopter at `IsoPlayer.players[0]` (the runtime's target and
    /// movement); without that player (`player0` false) it becomes
    /// inactive.
    pub fn load_bytes(&mut self, data: &[u8], player0: bool) -> io::Result<()> {
        let mut r = JavaReader::new(data);
        let _world_version = r.get_i32()?;
        self.helicopter.active = r.get_bool()?;
        let state = r.get_i32()?;
        if state < 0 || state as usize >= HELICOPTER_STATES {
            return Err(bad(format!(
                "java.lang.ArrayIndexOutOfBoundsException: Index {state} out of bounds for length {HELICOPTER_STATES}"
            )));
        }
        self.helicopter.state = state;
        self.helicopter.x = r.get_f32()?;
        self.helicopter.y = r.get_f32()?;
        if self.helicopter.active && !player0 {
            self.helicopter.active = false;
        }
        let n = r.get_i16()?;
        for _ in 0..n.max(0) {
            self.alarms.push(AlarmSave {
                x: r.get_i32()?,
                y: r.get_i32()?,
                end_game_time: r.get_f32()?,
            });
        }
        Ok(())
    }

    /// `MetaTracker.save()`: nothing with saving off.
    pub fn save_file(&self, save_dir: &Path, no_save: bool) -> io::Result<()> {
        if no_save {
            return Ok(());
        }
        std::fs::write(save_dir.join("metadata.bin"), self.save_bytes())
    }

    /// `MetaTracker.load()`: nothing with saving off or no file. `player0`:
    /// see [`MetaTracker::load_bytes`].
    pub fn load_file(&mut self, save_dir: &Path, no_save: bool, player0: bool) -> io::Result<()> {
        if no_save {
            return Ok(());
        }
        match std::fs::read(save_dir.join("metadata.bin")) {
            Ok(d) => self.load_bytes(&d, player0),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e),
        }
    }
}

/// `StatisticType` constant names, in declaration order.
pub const STATISTIC_TYPES: [&str; 5] = ["Player", "Zombie", "World", "Server", "Undefined"];
/// `StatisticCategory` constant names, in declaration order.
pub const STATISTIC_CATEGORIES: [&str; 15] = [
    "General",
    "Combat",
    "Exploration",
    "Health",
    "Wellness",
    "Skill",
    "Perks",
    "Traits",
    "Travel",
    "Items",
    "Survival",
    "Vehicle",
    "PVP",
    "Events",
    "Undefined",
];

/// A `Statistic`: its type and category by constant index.
#[derive(Clone, Debug, PartialEq)]
pub struct Statistic {
    pub name: String,
    pub statistic_type: usize,
    pub category: usize,
    pub value: f32,
}

/// `StatisticsManager.statisticHashMap`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Statistics {
    order: JavaHashOrder,
    stats: Vec<Statistic>,
}

/// `DataOutputStream.writeUTF`: a 16-bit length and modified UTF-8.
fn write_modified_utf(out: &mut Vec<u8>, s: &str) -> io::Result<()> {
    let mut b = Vec::new();
    for c in s.encode_utf16() {
        match c {
            0x0001..=0x007f => b.push(c as u8),
            0x0000 | 0x0080..=0x07ff => {
                b.push(0xc0 | (c >> 6) as u8);
                b.push(0x80 | (c & 0x3f) as u8);
            }
            _ => {
                b.push(0xe0 | (c >> 12) as u8);
                b.push(0x80 | ((c >> 6) & 0x3f) as u8);
                b.push(0x80 | (c & 0x3f) as u8);
            }
        }
    }
    if b.len() > 65535 {
        return Err(bad(
            "java.io.UTFDataFormatException: encoded string too long",
        ));
    }
    out.put_i16(b.len() as u16 as i16);
    out.extend_from_slice(&b);
    Ok(())
}

/// `DataInputStream.readUTF`.
fn read_modified_utf(r: &mut JavaReader<'_>) -> io::Result<String> {
    let n = r.get_u16()? as usize;
    let b = r.take(n)?;
    let mut units = Vec::with_capacity(n);
    let mut i = 0;
    let malformed = || bad("java.io.UTFDataFormatException: malformed input");
    while i < n {
        let c = b[i] as u16;
        if c < 0x80 {
            units.push(c);
            i += 1;
        } else if c >> 5 == 0b110 {
            let c2 = *b.get(i + 1).ok_or_else(malformed)? as u16;
            if c2 & 0xc0 != 0x80 {
                return Err(malformed());
            }
            units.push(((c & 0x1f) << 6) | (c2 & 0x3f));
            i += 2;
        } else if c >> 4 == 0b1110 {
            let c2 = *b.get(i + 1).ok_or_else(malformed)? as u16;
            let c3 = *b.get(i + 2).ok_or_else(malformed)? as u16;
            if c2 & 0xc0 != 0x80 || c3 & 0xc0 != 0x80 {
                return Err(malformed());
            }
            units.push(((c & 0x0f) << 12) | ((c2 & 0x3f) << 6) | (c3 & 0x3f));
            i += 3;
        } else {
            return Err(malformed());
        }
    }
    Ok(String::from_utf16_lossy(&units))
}

impl Statistics {
    /// The statistics in the `HashMap`'s order.
    pub fn values(&self) -> Vec<&Statistic> {
        self.order
            .iter_order()
            .into_iter()
            .map(|k| {
                self.stats
                    .iter()
                    .find(|s| s.name == k)
                    .expect("ordered key")
            })
            .collect()
    }

    fn entry(&mut self, statistic_type: usize, category: usize, key: &str) -> &mut Statistic {
        if !self.order.contains(key) {
            self.order.insert(key);
            self.stats.push(Statistic {
                name: key.to_string(),
                statistic_type,
                category,
                value: 0.0,
            });
        }
        self.stats
            .iter_mut()
            .find(|s| s.name == key)
            .expect("present")
    }

    /// `incrementStatistic(type, category, key, amount)` (the achievement
    /// check is the caller's).
    pub fn increment(&mut self, statistic_type: usize, category: usize, key: &str, amount: f32) {
        self.entry(statistic_type, category, key).value += amount;
    }

    /// `setStatistic(type, category, key, amount)`.
    pub fn set(&mut self, statistic_type: usize, category: usize, key: &str, amount: f32) {
        self.entry(statistic_type, category, key).value = amount;
    }

    /// `getStatistic(key)`.
    pub fn get(&self, key: &str) -> Option<f32> {
        self.stats.iter().find(|s| s.name == key).map(|s| s.value)
    }

    /// The bytes `save()` writes to `statistics.bin`: the count, then each
    /// statistic's name, type name, category name (`writeUTF`) and value.
    pub fn save_bytes(&self) -> io::Result<Vec<u8>> {
        let mut out = Vec::new();
        out.put_i32(self.stats.len() as i32);
        for s in self.values() {
            write_modified_utf(&mut out, &s.name)?;
            write_modified_utf(&mut out, STATISTIC_TYPES[s.statistic_type])?;
            write_modified_utf(&mut out, STATISTIC_CATEGORIES[s.category])?;
            out.put_f32(s.value);
        }
        Ok(out)
    }

    /// `load()` from the bytes (after the map was cleared): a name that is
    /// not a constant is `IllegalArgumentException` (`valueOf`), which ends
    /// the load keeping what was read. Each statistic is `put` (a repeated
    /// name replaces the earlier one in place). The achievement checks are
    /// the caller's.
    pub fn load_bytes(&mut self, data: &[u8]) -> io::Result<()> {
        let mut r = JavaReader::new(data);
        let n = r.get_i32()?;
        for _ in 0..n.max(0) {
            let name = read_modified_utf(&mut r)?;
            let t = read_modified_utf(&mut r)?;
            let c = read_modified_utf(&mut r)?;
            let value_of = |names: &[&str], s: &str, class: &str| {
                names.iter().position(|n| *n == s).ok_or_else(|| {
                    bad(format!(
                        "java.lang.IllegalArgumentException: No enum constant zombie.statistics.{class}.{s}"
                    ))
                })
            };
            let statistic_type = value_of(&STATISTIC_TYPES, &t, "StatisticType")?;
            let category = value_of(&STATISTIC_CATEGORIES, &c, "StatisticCategory")?;
            let value = r.get_f32()?;
            let s = self.entry(statistic_type, category, &name);
            s.statistic_type = statistic_type;
            s.category = category;
            s.value = value;
        }
        Ok(())
    }

    /// `StatisticsManager.save()`: nothing with saving off.
    pub fn save_file(&self, save_dir: &Path, no_save: bool) -> io::Result<()> {
        if no_save {
            return Ok(());
        }
        std::fs::write(save_dir.join("statistics.bin"), self.save_bytes()?)
    }

    /// `StatisticsManager.load()`: the map is cleared; nothing more with
    /// saving off or no file.
    pub fn load_file(&mut self, save_dir: &Path, no_save: bool) -> io::Result<()> {
        self.order.clear();
        self.stats.clear();
        if no_save {
            return Ok(());
        }
        match std::fs::read(save_dir.join("statistics.bin")) {
            Ok(d) => self.load_bytes(&d),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e),
        }
    }
}

#[cfg(test)]
mod tests;
