//! `map_t.bin`: `GameTime.save(DataOutputStream)` / `load(DataInputStream)`
//! with `ClimateManager.save`/`load` (and its `WeatherPeriod` and
//! `ThunderStorm`), plus the header-less `ByteBuffer` form the original uses
//! for network packets (`GameTime.save(ByteBuffer)` / `load(ByteBufferReader)`).
//!
//! `map_t.bin` layout: `"GMTM"`, world version, the clock fields, three
//! unused fields, an optional mod-data table, the poisonous berry and
//! mushroom (from `Core`), the helicopter day and hours, then the climate.
//! Strings and tables use the `DataOutputStream` encodings.
//!
//! The dedicated-server branch (`GameServer.server`), which appends the
//! admin overrides of fifteen `ClimateFloat`s/`ClimateColor`s/`ClimateBool`s,
//! is not ported; `client` selects the multiplayer-client branch, which
//! writes the climate and weather as absent.

use crate::core::Core;
use crate::game_time::{AirFront, ClimateManager, GameTime, StrLerpVal, ThunderCloud, WeatherPeriod, WeatherStage};
use crate::weather::{ColorRef, ThunderStorm};
use pz_kahlua::java_io::{JavaReader, JavaWrite};
use pz_kahlua::table_save::{load_table, save_table, Flavor};
use pz_kahlua::TableRef;
use std::io;
use std::path::Path;

/// `IsoWorld.getWorldVersion()` in this build.
pub const WORLD_VERSION: i32 = 249;
const MAGIC: [u8; 4] = *b"GMTM";

/// `GameTime.save(DataOutputStream)`: the whole of `map_t.bin`.
pub fn save_map_t(gt: &GameTime, core: &Core, client: bool) -> io::Result<Vec<u8>> {
    let mut out = Vec::new();
    out.extend_from_slice(&MAGIC);
    out.put_i32(WORLD_VERSION);
    save_clock(gt, &mut out, Flavor::DataStream)?;
    out.write_string_chars(core.get_poisonous_berry().as_deref());
    out.write_string_chars(core.get_poisonous_mushroom().as_deref());
    out.put_i32(gt.helicopter_day1);
    out.put_i32(gt.helicopter_time1_start);
    out.put_i32(gt.helicopter_time1_end);
    save_climate(&gt.climate, &mut out, client)?;
    Ok(out)
}

/// `GameTime.load(DataInputStream)`. A file without the `"GMTM"` header is
/// read from its start with `saved_world_version` (`IsoWorld.savedWorldVersion`;
/// `-1` means the current version). On a multiplayer client the berry and
/// mushroom are read but not applied to `Core`.
///
/// Fields are assigned as they are read, so an error leaves the earlier
/// ones loaded (the original logs the exception and carries on).
pub fn load_map_t(
    gt: &mut GameTime,
    core: &mut Core,
    data: &[u8],
    saved_world_version: i32,
    client: bool,
) -> io::Result<()> {
    let mut world_version = if saved_world_version == -1 {
        WORLD_VERSION
    } else {
        saved_world_version
    };
    let mut r = JavaReader::new(data);
    if r.take(4)? == MAGIC {
        world_version = r.get_i32()?;
    } else {
        r.set_position(0)?;
    }
    load_clock(gt, &mut r, world_version, Flavor::DataStream)?;
    let berry = r.read_string_chars()?;
    let mushroom = r.read_string_chars()?;
    if !client {
        core.set_poisonous_berry(Some(&berry));
        core.set_poisonous_mushroom(Some(&mushroom));
    }
    gt.helicopter_day1 = r.get_i32()?;
    gt.helicopter_time1_start = r.get_i32()?;
    gt.helicopter_time1_end = r.get_i32()?;
    load_climate(&mut gt.climate, &mut r, world_version)?;
    Ok(())
}

/// `GameTime.save()`: writes `map_t.bin` in `save_dir`.
pub fn save_map_t_file(gt: &GameTime, core: &Core, save_dir: &Path) -> io::Result<()> {
    std::fs::write(save_dir.join("map_t.bin"), save_map_t(gt, core, false)?)
}

/// `GameTime.load()`: a missing `map_t.bin` is not an error and changes
/// nothing.
pub fn load_map_t_file(
    gt: &mut GameTime,
    core: &mut Core,
    save_dir: &Path,
    saved_world_version: i32,
) -> io::Result<()> {
    match std::fs::read(save_dir.join("map_t.bin")) {
        Ok(data) => load_map_t(gt, core, &data, saved_world_version, false),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
    }
}

/// `GameTime.save(ByteBuffer)` (network form: clock and mod data only).
pub fn save_packet(gt: &GameTime, out: &mut Vec<u8>) -> io::Result<()> {
    save_clock(gt, out, Flavor::ByteBuffer)
}

/// `GameTime.load(ByteBufferReader)`.
pub fn load_packet(gt: &mut GameTime, r: &mut JavaReader<'_>) -> io::Result<()> {
    load_clock(gt, r, WORLD_VERSION, Flavor::ByteBuffer)
}

fn save_clock(gt: &GameTime, out: &mut Vec<u8>, flavor: Flavor) -> io::Result<()> {
    out.put_f32(gt.multiplier);
    out.put_i32(gt.nights_survived);
    out.put_i32(gt.target_zombies);
    out.put_f32(gt.last_time_of_day);
    out.put_f32(gt.time_of_day);
    out.put_i32(gt.day);
    out.put_i32(gt.month);
    out.put_i32(gt.year);
    out.put_f32(0.0);
    out.put_f32(0.0);
    out.put_i32(0);
    save_optional_table(gt.table.as_ref(), out, flavor)
}

fn load_clock(
    gt: &mut GameTime,
    r: &mut JavaReader<'_>,
    world_version: i32,
    flavor: Flavor,
) -> io::Result<()> {
    gt.multiplier = r.get_f32()?;
    gt.nights_survived = r.get_i32()?;
    gt.target_zombies = r.get_i32()?;
    gt.last_time_of_day = r.get_f32()?;
    gt.time_of_day = r.get_f32()?;
    gt.day = r.get_i32()?;
    gt.month = r.get_i32()?;
    gt.year = r.get_i32()?;
    r.get_f32()?;
    r.get_f32()?;
    r.get_i32()?;
    // `readByte() == 1` / `getBoolean()` (`!= 0`).
    let present = r.get_i8()?;
    let present = match flavor {
        Flavor::DataStream => present == 1,
        Flavor::ByteBuffer => present != 0,
    };
    if present {
        let t = gt.mod_data();
        load_table(&t, r, world_version, flavor)?;
    }
    Ok(())
}

fn save_optional_table(t: Option<&TableRef>, out: &mut Vec<u8>, flavor: Flavor) -> io::Result<()> {
    match t {
        Some(t) => {
            out.put_i8(1);
            save_table(t, out, flavor)
        }
        None => {
            out.put_i8(0);
            Ok(())
        }
    }
}

fn save_front(f: &AirFront, out: &mut Vec<u8>) {
    out.put_f32(f.days);
    out.put_f32(f.max_noise);
    out.put_f32(f.total_noise);
    out.put_i32(f.front_type);
    out.put_f32(f.strength);
    out.put_f32(f.front_wind_angle_degrees);
    out.put_i32(f.noise_cache.len() as i32);
    for v in f.noise_cache {
        out.put_f32(v);
    }
}

fn load_front(f: &mut AirFront, r: &mut JavaReader<'_>) -> io::Result<()> {
    f.days = r.get_f32()?;
    f.max_noise = r.get_f32()?;
    f.total_noise = r.get_f32()?;
    f.front_type = r.get_i32()?;
    f.strength = r.get_f32()?;
    f.front_wind_angle_degrees = r.get_f32()?;
    let len = r.get_i32()?;
    let cache = f.noise_cache.len() as i32;
    for i in 0..len.max(cache) {
        if i < len {
            let v = r.get_f32()?;
            if i < cache {
                f.noise_cache[i as usize] = v;
            }
        } else if i < cache {
            f.noise_cache[i as usize] = -1.0;
        }
    }
    Ok(())
}

fn save_color(c: &ColorRef, out: &mut Vec<u8>) {
    let c = c.borrow();
    for v in c.interior.iter().chain(c.exterior.iter()) {
        out.put_f32(*v);
    }
}

fn load_color(c: &ColorRef, r: &mut JavaReader<'_>) -> io::Result<()> {
    let mut guard = c.borrow_mut();
    let c = &mut *guard;
    for v in c.interior.iter_mut().chain(c.exterior.iter_mut()) {
        *v = r.get_f32()?;
    }
    Ok(())
}

/// `ClimateManager.save(DataOutputStream)` (non-server).
pub fn save_climate(c: &ClimateManager, out: &mut Vec<u8>, client: bool) -> io::Result<()> {
    if !client {
        out.put_i8(1);
        out.put_f64(c.simplex_offset_a);
        out.put_f64(c.simplex_offset_b);
        out.put_f64(c.simplex_offset_c);
        out.put_f64(c.simplex_offset_d);
        save_front(&c.current_front, out);
        out.put_f32(c.snow_frac_now);
        out.put_f32(c.snow_strength);
        out.put_bool(c.can_do_winter_sprites);
        out.put_bool(c.day_do_fog);
        out.put_f32(c.day_fog_strength);
    } else {
        out.put_i8(0);
    }
    save_weather_period(&c.weather_period, out, client);
    save_thunder_storm(&c.thunder_storm.clouds, out, client);
    save_optional_table(c.mod_data_table.as_ref(), out, Flavor::DataStream)
}

/// `ClimateManager.load(DataInputStream, worldVersion)` (non-server).
pub fn load_climate(
    c: &mut ClimateManager,
    r: &mut JavaReader<'_>,
    world_version: i32,
) -> io::Result<()> {
    if r.get_i8()? == 1 {
        c.simplex_offset_a = r.get_f64()?;
        c.simplex_offset_b = r.get_f64()?;
        c.simplex_offset_c = r.get_f64()?;
        c.simplex_offset_d = r.get_f64()?;
        load_front(&mut c.current_front, r)?;
        c.snow_frac_now = r.get_f32()?;
        c.snow_strength = r.get_f32()?;
        c.can_do_winter_sprites = r.get_bool()?;
        c.day_do_fog = r.get_bool()?;
        c.day_fog_strength = r.get_f32()?;
    }
    load_weather_period(&mut c.weather_period, r)?;
    load_thunder_storm(&mut c.thunder_storm, r)?;
    if r.get_i8()? == 1 {
        let t = c.mod_data_table.get_or_insert_with(TableRef::new).clone();
        load_table(&t, r, world_version, Flavor::DataStream)?;
    }
    c.after_load();
    Ok(())
}

fn save_weather_period(w: &WeatherPeriod, out: &mut Vec<u8>, client: bool) {
    if client {
        out.put_i8(0);
        return;
    }
    out.put_i8(1);
    out.put_bool(w.is_running);
    if !w.is_running {
        return;
    }
    out.put_i32(w.weather_stage_index);
    out.put_f32(w.current_strength);
    out.put_f32(w.rain_threshold);
    out.put_bool(w.is_thunder_storm);
    out.put_bool(w.is_tropical_storm);
    out.put_bool(w.is_blizzard);
    save_front(&w.front_cache, out);
    out.put_i32(w.weather_stages.len() as i32);
    for ws in &w.weather_stages {
        out.put_i32(ws.stage_id);
        out.put_f64(ws.stage_duration);
        out.put_f64(ws.stage_start);
        out.put_f32(ws.entry_strength);
        out.put_f32(ws.exit_strength);
        out.put_f32(ws.target_strength);
        out.put_i32(ws.lerp_mid_val.value());
        out.put_i32(ws.lerp_end_val.value());
        out.put_bool(ws.has_started_cloud);
        out.put_i8(ws.mod_id.is_some() as i8);
        if let Some(m) = &ws.mod_id {
            out.write_string_chars(Some(m));
        }
        out.put_f32(ws.fog_strength);
    }
    save_color(&w.cloud_color, out);
}

fn load_weather_period(w: &mut WeatherPeriod, r: &mut JavaReader<'_>) -> io::Result<()> {
    if r.get_i8()? != 1 {
        return Ok(());
    }
    w.is_running = r.get_bool()?;
    if !w.is_running {
        return Ok(());
    }
    w.weather_stage_index = r.get_i32()?;
    w.current_strength = r.get_f32()?;
    w.rain_threshold = r.get_f32()?;
    w.is_thunder_storm = r.get_bool()?;
    w.is_tropical_storm = r.get_bool()?;
    w.is_blizzard = r.get_bool()?;
    load_front(&mut w.front_cache, r)?;
    // `clearCurrentWeatherStages()`: pooled stages come back reset.
    w.weather_stages.clear();
    let count = r.get_i32()?;
    for _ in 0..count.max(0) {
        let mut ws = WeatherStage {
            stage_id: r.get_i32()?,
            ..WeatherStage::default()
        };
        ws.set_stage_duration(r.get_f64()?);
        ws.stage_start = r.get_f64()?;
        ws.entry_strength = r.get_f32()?;
        ws.exit_strength = r.get_f32()?;
        ws.target_strength = r.get_f32()?;
        ws.lerp_mid_val = StrLerpVal::from_value(r.get_i32()?);
        ws.lerp_end_val = StrLerpVal::from_value(r.get_i32()?);
        ws.has_started_cloud = r.get_bool()?;
        if r.get_i8()? == 1 {
            ws.mod_id = Some(r.read_string_chars()?);
        }
        ws.fog_strength = r.get_f32()?;
        w.weather_stages.push(ws);
    }
    load_color(&w.cloud_color, r)?;
    Ok(())
}

fn save_thunder_storm(clouds: &[ThunderCloud; 3], out: &mut Vec<u8>, client: bool) {
    if client {
        out.put_i8(0);
        return;
    }
    out.put_i8(clouds.len() as i8);
    for c in clouds {
        out.put_bool(c.is_running);
        if !c.is_running {
            continue;
        }
        out.put_i32(c.start_x);
        out.put_i32(c.start_y);
        out.put_i32(c.end_x);
        out.put_i32(c.end_y);
        out.put_f32(c.radius);
        out.put_f32(c.angle);
        out.put_f32(c.strength);
        out.put_f32(c.thunder_ratio);
        out.put_f64(c.start_time);
        out.put_f64(c.end_time);
        out.put_f64(c.duration);
        out.put_f32(c.percentage_offset);
    }
}

fn load_thunder_storm(ts: &mut ThunderStorm, r: &mut JavaReader<'_>) -> io::Result<()> {
    let len = r.get_i8()? as i32;
    // Clouds past the third go into a scratch cloud (`dummyCloud`).
    if len > ts.clouds.len() as i32 && ts.dummy_cloud.is_none() {
        ts.dummy_cloud = Some(ThunderCloud::default());
    }
    for i in 0..len.max(0) as usize {
        let running = r.get_bool()?;
        let c = if i < ts.clouds.len() {
            &mut ts.clouds[i]
        } else {
            ts.dummy_cloud.get_or_insert_with(ThunderCloud::default)
        };
        c.is_running = running;
        if !running {
            continue;
        }
        c.start_x = r.get_i32()?;
        c.start_y = r.get_i32()?;
        c.end_x = r.get_i32()?;
        c.end_y = r.get_i32()?;
        c.radius = r.get_f32()?;
        c.angle = r.get_f32()?;
        c.strength = r.get_f32()?;
        c.thunder_ratio = r.get_f32()?;
        c.start_time = r.get_f64()?;
        c.end_time = r.get_f64()?;
        c.duration = r.get_f64()?;
        c.percentage_offset = r.get_f32()?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use pz_kahlua::Value;

    fn sample() -> (GameTime, Core) {
        let mut gt = GameTime {
            nights_survived: 3,
            time_of_day: 13.25,
            day: 5,
            helicopter_day1: 9,
            ..GameTime::default()
        };
        gt.mod_data().set_str("camping", Value::str("x"));
        let c = &mut gt.climate;
        c.simplex_offset_a = 1234.0;
        c.current_front.noise_cache = [0.5, -1.0];
        c.weather_period.is_running = true;
        c.weather_period.weather_stage_index = 1;
        for id in [2, 4] {
            let mut ws = WeatherStage {
                stage_id: id,
                lerp_end_val: StrLerpVal::Target,
                ..WeatherStage::default()
            };
            ws.set_stage_duration(3.5);
            c.weather_period.weather_stages.push(ws);
        }
        c.weather_period.weather_stages[0].mod_id = Some("mod".into());
        c.thunder_storm.clouds[1].is_running = true;
        c.thunder_storm.clouds[1].duration = 2.0;
        let mut core = Core::new();
        core.set_poisonous_berry(Some("Base.BerryGeneric5"));
        core.set_poisonous_mushroom(Some("Base.MushroomGeneric1"));
        (gt, core)
    }

    #[test]
    fn round_trip() {
        let (gt, core) = sample();
        let bytes = save_map_t(&gt, &core, false).unwrap();
        assert_eq!(&bytes[..8], b"GMTM\0\0\0\xf9");
        let (mut gt2, mut core2) = (GameTime::default(), Core::new());
        load_map_t(&mut gt2, &mut core2, &bytes, -1, false).unwrap();
        assert_eq!(gt2.day, 5);
        let ids = |g: &GameTime| {
            g.climate.weather_period.weather_stages.iter().map(|s| (s.stage_id, s.stage_duration)).collect::<Vec<_>>()
        };
        assert_eq!(ids(&gt2), ids(&gt));
        // `load` links the stages and starts the current one.
        assert_eq!(gt2.climate.weather_period.weather_stages[0].next_stage, Some(1));
        assert_eq!(gt2.climate.weather_period.current_stage, Some(1));
        assert_eq!(
            core2.get_poisonous_berry().as_deref(),
            Some("Base.BerryGeneric5")
        );
        assert_eq!(save_map_t(&gt2, &core2, false).unwrap(), bytes);
    }

    #[test]
    fn headerless_file_uses_saved_world_version() {
        let (gt, core) = sample();
        let bytes = save_map_t(&gt, &core, false).unwrap();
        let (mut gt2, mut core2) = (GameTime::default(), Core::new());
        load_map_t(&mut gt2, &mut core2, &bytes[8..], 249, false).unwrap();
        assert_eq!(save_map_t(&gt2, &core2, false).unwrap(), bytes);
    }

    #[test]
    fn load_normalises_like_the_original() {
        let (mut gt, core) = sample();
        let wp = &mut gt.climate.weather_period;
        wp.weather_stages[0].stage_duration = 0.25;
        let bytes = save_map_t(&gt, &core, false).unwrap();
        let (mut gt2, mut core2) = (GameTime::default(), Core::new());
        load_map_t(&mut gt2, &mut core2, &bytes, -1, false).unwrap();
        assert_eq!(
            gt2.climate.weather_period.weather_stages[0].stage_duration,
            1.0
        );
        assert_eq!(StrLerpVal::from_value(9), StrLerpVal::None);

        // A stage index out of range stops the period.
        let (mut gt, core) = sample();
        gt.climate.weather_period.weather_stage_index = 2;
        let bytes = save_map_t(&gt, &core, false).unwrap();
        let (mut gt2, mut core2) = (GameTime::default(), Core::new());
        load_map_t(&mut gt2, &mut core2, &bytes, -1, false).unwrap();
        assert!(!gt2.climate.weather_period.is_running);
        assert!(save_map_t(&gt2, &core2, false).unwrap().len() < bytes.len());
    }

    #[test]
    fn client_branch() {
        let (gt, core) = sample();
        let bytes = save_map_t(&gt, &core, true).unwrap();
        let (mut gt2, mut core2) = (GameTime::default(), Core::new());
        load_map_t(&mut gt2, &mut core2, &bytes, -1, true).unwrap();
        assert_eq!(core2.get_poisonous_berry(), None);
        assert_eq!(gt2.climate.simplex_offset_a, 0.0);
        assert_eq!(gt2.helicopter_day1, 9);
    }

    #[test]
    fn packet_form() {
        let (gt, _) = sample();
        let mut out = Vec::new();
        save_packet(&gt, &mut out).unwrap();
        let mut gt2 = GameTime::default();
        load_packet(&mut gt2, &mut JavaReader::new(&out)).unwrap();
        assert_eq!(gt2.time_of_day, 13.25);
        assert!(matches!(gt2.mod_data().get_str("camping"), Value::Str(_)));
    }
}
