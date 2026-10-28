//! Play one event live on the default output device.
//!
//! `cargo run -p pz-audio --release --example play_event -- <game dir> <event path> [seconds]`

use pz_audio::output::Output;
use pz_audio::studio::Studio;
use pz_audio::vorbis_setup::SetupHeaders;
use pz_audio::BankFile;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let game = PathBuf::from(&a[0]);
    let banks = game.join("media/sound/banks/Desktop");
    let strings = BankFile::open(&banks.join("ZomboidSound.strings.bank"))
        .unwrap()
        .strings()
        .unwrap();
    let mut s = Studio::new(SetupHeaders::from_game_dir(&game).unwrap());
    for b in ["ZomboidSound", "ZomboidMusic", "ZomboidSoundMP"] {
        s.load_bank(&banks.join(format!("{b}.bank"))).unwrap();
    }
    let id = s
        .create_instance(strings.lookup(&a[1]).expect("unknown event"))
        .unwrap();
    s.start(id);
    let studio = Arc::new(Mutex::new(s));
    let out = Output::start(studio).expect("audio output");
    eprintln!(
        "playing at {} Hz, {} channels",
        out.device_rate, out.device_channels
    );
    let secs = a.get(2).map_or(5.0, |x| x.parse().unwrap());
    std::thread::sleep(std::time::Duration::from_secs_f32(secs));
}
