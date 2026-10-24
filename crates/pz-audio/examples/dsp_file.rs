//! Run one effect over a float WAV (32-bit, interleaved) for comparison with
//! FMOD's own DSP: `dsp_file <effect> <in.wav> <out.wav> <param>...`.

use pz_audio::dsp::{compressor::Compressor, Dsp};

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let data = std::fs::read(&a[1]).unwrap();
    let at = data.windows(4).position(|w| w == b"data").unwrap();
    let channels = u16::from_le_bytes([data[22], data[23]]) as usize;
    let mut pcm: Vec<f32> = data[at + 8..]
        .chunks_exact(4)
        .map(|b| f32::from_le_bytes(b.try_into().unwrap()))
        .collect();
    let params: Vec<f32> = a[3..].iter().map(|p| p.parse().unwrap()).collect();
    let mut dsp: Box<dyn Dsp> = match a[0].as_str() {
        "compressor" => {
            let mut c = Compressor::new(48000);
            for (i, p) in params.iter().enumerate() {
                c.set_float(i, *p);
            }
            Box::new(c)
        }
        other => panic!("unknown effect {other}"),
    };
    for block in pcm.chunks_mut(256 * channels) {
        dsp.process(block, channels);
    }
    let mut out = data[..at + 8].to_vec();
    for s in pcm {
        out.extend_from_slice(&s.to_le_bytes());
    }
    std::fs::write(&a[2], out).unwrap();
}
