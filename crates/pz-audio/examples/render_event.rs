//! Render one event offline to a 16-bit stereo WAV with `studio::Studio`.
//!
//! `cargo run -p pz-audio --release --example render_event -- <game dir> <event path> <seconds> <out.wav> [x,y,z]`

use pz_audio::studio::{Studio, SAMPLE_RATE};
use pz_audio::vorbis_setup::SetupHeaders;
use pz_audio::BankFile;
use std::io::Write;
use std::path::PathBuf;

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
    let ev = strings.lookup(&a[1]).expect("unknown event");
    let id = s.create_instance(ev).unwrap();
    if let Some(p) = a.get(4) {
        let v: Vec<f32> = p.split(',').map(|x| x.parse().unwrap()).collect();
        s.set_position(id, [v[0], v[1], v[2]]);
    }
    s.start(id);
    let frames = (a[2].parse::<f32>().unwrap() * SAMPLE_RATE as f32) as usize;
    let mut out = vec![0f32; frames * 2];
    for (k, chunk) in out.chunks_mut(1024 * 2).enumerate() {
        s.mix(chunk);
        if std::env::var_os("PZ_AUDIO_DEBUG").is_some() && k < 3 {
            eprintln!("{}", s.debug_instance(id));
        }
    }
    let mut f = std::io::BufWriter::new(std::fs::File::create(&a[3]).unwrap());
    let n = (out.len() * 2) as u32;
    f.write_all(b"RIFF").unwrap();
    f.write_all(&(36 + n).to_le_bytes()).unwrap();
    f.write_all(b"WAVEfmt ").unwrap();
    f.write_all(&16u32.to_le_bytes()).unwrap();
    for v in [1u16, 2] {
        f.write_all(&v.to_le_bytes()).unwrap();
    }
    f.write_all(&SAMPLE_RATE.to_le_bytes()).unwrap();
    f.write_all(&(SAMPLE_RATE * 4).to_le_bytes()).unwrap();
    for v in [4u16, 16] {
        f.write_all(&v.to_le_bytes()).unwrap();
    }
    f.write_all(b"data").unwrap();
    f.write_all(&n.to_le_bytes()).unwrap();
    for x in out {
        f.write_all(&((x.clamp(-1.0, 1.0) * 32767.0) as i16).to_le_bytes())
            .unwrap();
    }
}
