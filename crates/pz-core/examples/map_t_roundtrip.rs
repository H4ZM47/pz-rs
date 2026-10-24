//! Loads each `map_t.bin` given and saves it again, reporting whether the
//! bytes are identical. With `--mutate <out-dir>`, also writes a copy with
//! changed clock, weather and mod data (named after its input's parent
//! folder) for loading in the original game.
//!
//! `map_t_roundtrip [--mutate <out-dir>] <map_t.bin>...`

use pz_core::core::Core;
use pz_core::game_time::{GameTime, StrLerpVal, WeatherStage};
use pz_core::game_time_save::{load_map_t, save_map_t};
use pz_kahlua::{TableRef, Value};
use std::path::Path;

fn main() {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let mut mutate = None;
    if args.first().map(String::as_str) == Some("--mutate") {
        mutate = Some(args[1].clone());
        args.drain(..2);
    }
    let mut failed = false;
    for f in &args {
        let data = std::fs::read(f).expect("read");
        let (mut gt, mut core) = (GameTime::default(), Core::new());
        if let Err(e) = load_map_t(&mut gt, &mut core, &data, -1, false) {
            println!("{f}: load error {e}");
            failed = true;
            continue;
        }
        let out = save_map_t(&gt, &core, false).expect("save");
        let same = out == data;
        failed |= !same;
        println!(
            "{f}: {} ({} bytes, day {}/{}/{}, {:.3}h, weather running {}, {} stages)",
            if same { "identical" } else { "DIFFERENT" },
            data.len(),
            gt.day,
            gt.month,
            gt.year,
            gt.time_of_day,
            gt.climate.weather_period.is_running,
            gt.climate.weather_period.weather_stages.len()
        );
        if let Some(dir) = &mutate {
            gt.day += 1;
            gt.time_of_day = 23.5;
            gt.nights_survived += 7;
            let md = gt.mod_data();
            md.set_str("rustAdded", Value::str("caf\u{e9} \u{1F600}"));
            let sub = TableRef::new();
            sub.set_int(1, true);
            sub.set_int(2, -0.0);
            md.set_str("rustTable", Value::Table(sub));
            let wp = &mut gt.climate.weather_period;
            wp.is_running = true;
            wp.weather_stage_index = 0;
            wp.weather_stages.push(WeatherStage {
                stage_id: 3,
                stage_duration: 4.25,
                lerp_mid_val: StrLerpVal::NextTarget,
                mod_id: Some("rust".into()),
                ..WeatherStage::default()
            });
            gt.climate.thunder_clouds[2].is_running = true;
            gt.climate.thunder_clouds[2].end_x = 12345;
            core.set_poisonous_berry(Some("Base.BerryGeneric2"));
            let name = Path::new(f)
                .parent()
                .and_then(|p| p.file_name())
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| "out".into());
            let path = Path::new(dir).join(format!("{name}.map_t.bin"));
            std::fs::write(&path, save_map_t(&gt, &core, false).expect("save")).expect("write");
        }
    }
    std::process::exit(failed as i32);
}
