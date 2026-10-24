//! `zombie.erosion.ErosionConfig`: the `erosion.ini` settings (noise seeds,
//! time, season climate) with the original reader, writer and network
//! format. Field defaults come from the game's constructors
//! ([`ErosionTables::config_default`]).

use std::path::Path;

use crate::java::{float_to_string, parse_float, parse_int, trim};
use crate::tables::ErosionTables;

/// `ErosionConfig.Seeds`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Seeds {
    pub seed_main_0: i32,
    pub seed_main_1: i32,
    pub seed_main_2: i32,
    pub seed_moisture_0: i32,
    pub seed_moisture_1: i32,
    pub seed_moisture_2: i32,
    pub seed_minerals_0: i32,
    pub seed_minerals_1: i32,
    pub seed_minerals_2: i32,
    pub seed_kudzu_0: i32,
    pub seed_kudzu_1: i32,
    pub seed_kudzu_2: i32,
}

/// `ErosionConfig.Time`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Time {
    pub tickunit: i32,
    pub ticks: i32,
    pub eticks: i32,
    pub epoch: i32,
}

/// `ErosionConfig.Debug`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Debug {
    pub enabled: bool,
    pub startday: i32,
    pub startmonth: i32,
}

/// `ErosionConfig.Season`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Season {
    pub lat: i32,
    pub temp_max: i32,
    pub temp_min: i32,
    pub temp_diff: i32,
    pub season_lag: i32,
    pub noon: f32,
    pub seed_a: i32,
    pub seed_b: i32,
    pub seed_c: i32,
    /// `jan` .. `dec`.
    pub rain: [f32; 12],
}

/// `ErosionConfig`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ErosionConfig {
    pub seeds: Seeds,
    pub time: Time,
    pub debug: Debug,
    pub season: Season,
}

const MONTHS: [&str; 12] = ["jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec"];

/// The `erosion.ini` key (as the reader and writer spell it) of each
/// integer field, with its accessor.
type IntField = (&'static str, &'static str, fn(&mut ErosionConfig) -> &mut i32);

fn int_fields() -> Vec<IntField> {
    vec![
        ("seeds.seedMain_0", "seeds.seedMain0", |c| &mut c.seeds.seed_main_0),
        ("seeds.seedMain_1", "seeds.seedMain1", |c| &mut c.seeds.seed_main_1),
        ("seeds.seedMain_2", "seeds.seedMain2", |c| &mut c.seeds.seed_main_2),
        ("seeds.seedMoisture_0", "seeds.seedMoisture0", |c| &mut c.seeds.seed_moisture_0),
        ("seeds.seedMoisture_1", "seeds.seedMoisture1", |c| &mut c.seeds.seed_moisture_1),
        ("seeds.seedMoisture_2", "seeds.seedMoisture2", |c| &mut c.seeds.seed_moisture_2),
        ("seeds.seedMinerals_0", "seeds.seedMinerals0", |c| &mut c.seeds.seed_minerals_0),
        ("seeds.seedMinerals_1", "seeds.seedMinerals1", |c| &mut c.seeds.seed_minerals_1),
        ("seeds.seedMinerals_2", "seeds.seedMinerals2", |c| &mut c.seeds.seed_minerals_2),
        ("seeds.seedKudzu_0", "seeds.seedKudzu0", |c| &mut c.seeds.seed_kudzu_0),
        ("seeds.seedKudzu_1", "seeds.seedKudzu1", |c| &mut c.seeds.seed_kudzu_1),
        ("seeds.seedKudzu_2", "seeds.seedKudzu2", |c| &mut c.seeds.seed_kudzu_2),
        ("time.tickunit", "time.tickunit", |c| &mut c.time.tickunit),
        ("time.ticks", "time.ticks", |c| &mut c.time.ticks),
        ("time.eticks", "time.eticks", |c| &mut c.time.eticks),
        ("time.epoch", "time.epoch", |c| &mut c.time.epoch),
        ("season.lat", "season.lat", |c| &mut c.season.lat),
        ("season.tempMax", "season.tempMax", |c| &mut c.season.temp_max),
        ("season.tempMin", "season.tempMin", |c| &mut c.season.temp_min),
        ("season.tempDiff", "season.tempDiff", |c| &mut c.season.temp_diff),
        ("season.seasonLag", "season.seasonLag", |c| &mut c.season.season_lag),
        ("season.seedA", "season.seedA", |c| &mut c.season.seed_a),
        ("season.seedB", "season.seedB", |c| &mut c.season.seed_b),
        ("season.seedC", "season.seedC", |c| &mut c.season.seed_c),
        ("debug.startday", "debug.startday", |c| &mut c.debug.startday),
        ("debug.startmonth", "debug.startmonth", |c| &mut c.debug.startmonth),
    ]
}

impl ErosionConfig {
    /// `new ErosionConfig()`: the constructors' defaults.
    pub fn new(tables: &ErosionTables) -> Self {
        let mut c = ErosionConfig::default();
        for (_, field, get) in int_fields() {
            if let Some(v) = tables.config_default(field).and_then(|v| v.as_int()) {
                *get(&mut c) = v as i32;
            }
        }
        if let Some(v) = tables.config_default("debug.enabled").and_then(|v| v.as_int()) {
            c.debug.enabled = v != 0;
        }
        if let Some(v) = tables.config_default("season.noon").and_then(|v| v.as_float()) {
            c.season.noon = v as f32;
        }
        for (i, m) in MONTHS.iter().enumerate() {
            if let Some(v) = tables.config_default(&format!("season.{m}")).and_then(|v| v.as_float()) {
                c.season.rain[i] = v as f32;
            }
        }
        c
    }

    /// `readFile(fileName)`: `false` if the file does not exist or a value
    /// does not parse (the original's caught exception). Lines it does not
    /// understand are logged and skipped.
    pub fn read_file(&mut self, path: &Path) -> bool {
        let Ok(bytes) = std::fs::read(path) else {
            return false;
        };
        self.read_str(&String::from_utf8_lossy(&bytes))
    }

    /// `readFile` over the file's text.
    pub fn read_str(&mut self, text: &str) -> bool {
        let unknown = |line: &str| eprintln!("ErosionConfig: unknown \"{line}\"");
        let ints = int_fields();
        // `BufferedReader.readLine` line terminators.
        let text = text.replace("\r\n", "\n").replace('\r', "\n");
        let mut lines: Vec<&str> = text.split('\n').collect();
        if lines.last() == Some(&"") {
            lines.pop();
        }
        for line in lines {
            if trim(line).starts_with("--") {
                continue;
            }
            if !line.contains('=') {
                if !trim(line).is_empty() {
                    unknown(line);
                }
                continue;
            }
            // `String.split("=")`: trailing empty strings are dropped.
            let mut ss: Vec<&str> = line.split('=').collect();
            while ss.len() > 1 && ss.last() == Some(&"") {
                ss.pop();
            }
            if ss.len() != 2 {
                unknown(line);
                continue;
            }
            let k = trim(ss[0]);
            let v = trim(ss[1]);
            let section = ["seeds.", "time.", "season.", "debug."].into_iter().find(|p| k.starts_with(p));
            let Some(section) = section else {
                unknown(line);
                continue;
            };
            if let Some((_, _, get)) = ints.iter().find(|(key, _, _)| *key == k) {
                let Some(n) = parse_int(v) else {
                    eprintln!("NumberFormatException: For input string: \"{v}\"");
                    return false;
                };
                *get(self) = n;
                continue;
            }
            let float_slot = if k == "season.noon" {
                Some(&mut self.season.noon)
            } else {
                MONTHS.iter().position(|m| k.strip_prefix("season.") == Some(m)).map(|i| &mut self.season.rain[i])
            };
            if let Some(slot) = float_slot {
                let Some(f) = parse_float(v) else {
                    eprintln!("NumberFormatException: For input string: \"{v}\"");
                    return false;
                };
                *slot = f;
                continue;
            }
            if k == "debug.enabled" {
                self.debug.enabled = pz_world::java::parse_boolean(v);
                continue;
            }
            // Unknown `debug.*` keys are ignored silently upstream.
            if section != "debug." {
                unknown(line);
            }
        }
        true
    }

    /// The text `writeFile` writes.
    pub fn to_file_string(&self) -> String {
        let mut c = self.clone();
        let mut s = String::new();
        for (key, _, get) in int_fields().into_iter().take(12) {
            s += &format!("{key} = {}\n", get(&mut c));
        }
        s += "\n";
        for (key, _, get) in int_fields().into_iter().skip(12).take(4) {
            s += &format!("{key} = {}\n", get(&mut c));
        }
        s += "\n";
        let se = &self.season;
        s += &format!("season.lat = {}\n", se.lat);
        s += &format!("season.tempMax = {}\n", se.temp_max);
        s += &format!("season.tempMin = {}\n", se.temp_min);
        s += &format!("season.tempDiff = {}\n", se.temp_diff);
        s += &format!("season.seasonLag = {}\n", se.season_lag);
        s += &format!("season.noon = {}\n", float_to_string(se.noon));
        s += &format!("season.seedA = {}\n", se.seed_a);
        s += &format!("season.seedB = {}\n", se.seed_b);
        s += &format!("season.seedC = {}\n", se.seed_c);
        for (m, v) in MONTHS.iter().zip(se.rain) {
            s += &format!("season.{m} = {}\n", float_to_string(v));
        }
        s += "\n";
        s += &format!("debug.enabled = {}\n", self.debug.enabled);
        s += &format!("debug.startday = {}\n", self.debug.startday);
        s += &format!("debug.startmonth = {}\n", self.debug.startmonth);
        s
    }

    /// `writeFile(fileName)` (skipped when `no_save`, like
    /// `Core.isNoSave()`).
    pub fn write_file(&self, path: &Path, no_save: bool) {
        if no_save {
            return;
        }
        if let Err(e) = std::fs::write(path, self.to_file_string()) {
            eprintln!("ErosionConfig: {e}");
        }
    }

    /// `save(ByteBufferWriter)` (the network format).
    pub fn save(&self, out: &mut Vec<u8>) {
        let mut c = self.clone();
        let ints = int_fields();
        let int = |out: &mut Vec<u8>, v: i32| out.extend_from_slice(&v.to_be_bytes());
        let float = |out: &mut Vec<u8>, v: f32| out.extend_from_slice(&v.to_bits().to_be_bytes());
        for (_, _, get) in &ints[..21] {
            int(out, *get(&mut c));
        }
        float(out, c.season.noon);
        for (_, _, get) in &ints[21..24] {
            int(out, *get(&mut c));
        }
        for v in c.season.rain {
            float(out, v);
        }
    }

    /// `load(ByteBufferReader)`.
    /// Reads the 37 big-endian values `save` writes from `data`.
    pub fn load(&mut self, data: &[u8]) -> Result<(), String> {
        if data.len() < 37 * 4 {
            return Err("BufferUnderflowException".into());
        }
        let word = |i: usize| u32::from_be_bytes(data[i * 4..i * 4 + 4].try_into().unwrap());
        let ints = int_fields();
        for (i, (_, _, get)) in ints[..21].iter().enumerate() {
            *get(self) = word(i) as i32;
        }
        self.season.noon = f32::from_bits(word(21));
        for (i, (_, _, get)) in ints[21..24].iter().enumerate() {
            *get(self) = word(22 + i) as i32;
        }
        for i in 0..12 {
            self.season.rain[i] = f32::from_bits(word(25 + i));
        }
        Ok(())
    }

    pub fn get_debug(&self) -> &Debug {
        &self.debug
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> ErosionConfig {
        let mut c = ErosionConfig::default();
        c.seeds.seed_main_0 = 3;
        c.time.tickunit = 10;
        c.season.lat = 40;
        c.season.noon = 11.5;
        c.season.seed_c = 9;
        c.season.rain[11] = 0.25;
        c.debug.startday = 4;
        c
    }

    #[test]
    fn file_round_trip() {
        let c = sample();
        let text = c.to_file_string();
        assert!(text.starts_with("seeds.seedMain_0 = 3\n"));
        assert!(text.contains("season.noon = 11.5\n"));
        assert!(text.contains("season.jan = 0.0\n"));
        assert!(text.ends_with("debug.startmonth = 0\n"));
        let mut back = ErosionConfig::default();
        assert!(back.read_str(&text));
        assert_eq!(back, c);
    }

    #[test]
    fn reader_rules() {
        let mut c = ErosionConfig::default();
        assert!(c.read_str("-- comment\n\nbogus\nseeds.seedMain_1 = 7\nseeds.x = 1\na=b=c\ndebug.enabled = TRUE\r\n"));
        assert_eq!(c.seeds.seed_main_1, 7);
        assert!(c.debug.enabled);
        assert!(!c.read_str("time.ticks = x\n"));
        assert!(!c.read_file(Path::new("/nonexistent/erosion.ini")));
    }

    #[test]
    fn network_round_trip() {
        let c = sample();
        let mut out = Vec::new();
        c.save(&mut out);
        assert_eq!(out.len(), 37 * 4);
        let mut back = ErosionConfig { debug: c.debug.clone(), ..Default::default() };
        back.load(&out).unwrap();
        assert_eq!(back, c);
    }
}
