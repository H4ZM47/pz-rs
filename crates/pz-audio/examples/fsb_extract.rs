//! List or extract samples from a game bank, for listening and for checking
//! the decoder against an independent one.
//!
//! ```text
//! cargo run -p pz-audio --release --example fsb_extract -- <game dir> <bank> [index out.wav [out.ogg]]
//! ```
//!
//! The optional `.ogg` holds the same Vorbis packets in a standard Ogg
//! stream (rebuilt identification/setup headers), so any Vorbis decoder can
//! be compared with ours. Output stays local; never commit it.

use pz_audio::vorbis_setup::{crc32, SetupHeaders};
use pz_audio::{fsb5, vorbis, BankFile};
use std::io::Write;
use std::path::PathBuf;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let game = PathBuf::from(&args[0]);
    let bank = BankFile::open(&PathBuf::from(&args[1])).expect("open bank");
    let fsb = bank.fsb.as_ref().expect("bank has no samples");
    let Some(index) = args.get(2).map(|s| s.parse::<usize>().unwrap()) else {
        for (i, s) in fsb.samples.iter().enumerate() {
            println!(
                "{i}\t{}\t{}ch\t{}Hz\t{}",
                s.frames,
                s.channels,
                s.frequency,
                s.name.as_deref().unwrap_or("")
            );
        }
        return;
    };
    let setups = SetupHeaders::from_game_dir(&game).expect("read FMOD library");
    let s = &fsb.samples[index];
    let pcm = bank
        .decoder(index, &setups)
        .expect("decoder")
        .decode_all()
        .expect("decode");
    write_wav(&args[3], s.channels as u16, s.frequency, &pcm);
    if let Some(ogg) = args.get(4) {
        let crc = vorbis::setup_crc(bank.fsb_bytes(), s).unwrap();
        write_ogg(ogg, s, bank.fsb_bytes(), setups.get(crc).unwrap());
    }
}

fn write_wav(path: &str, ch: u16, rate: u32, pcm: &[f32]) {
    let mut f = std::io::BufWriter::new(std::fs::File::create(path).unwrap());
    let data_len = (pcm.len() * 4) as u32;
    f.write_all(b"RIFF").unwrap();
    f.write_all(&(36 + data_len).to_le_bytes()).unwrap();
    f.write_all(b"WAVEfmt ").unwrap();
    f.write_all(&16u32.to_le_bytes()).unwrap();
    f.write_all(&3u16.to_le_bytes()).unwrap(); // IEEE float
    f.write_all(&ch.to_le_bytes()).unwrap();
    f.write_all(&rate.to_le_bytes()).unwrap();
    f.write_all(&(rate * ch as u32 * 4).to_le_bytes()).unwrap();
    f.write_all(&(ch * 4).to_le_bytes()).unwrap();
    f.write_all(&32u16.to_le_bytes()).unwrap();
    f.write_all(b"data").unwrap();
    f.write_all(&data_len.to_le_bytes()).unwrap();
    for x in pcm {
        f.write_all(&x.to_le_bytes()).unwrap();
    }
}

fn ogg_crc(data: &[u8]) -> u32 {
    let mut crc = 0u32;
    for &b in data {
        crc ^= (b as u32) << 24;
        for _ in 0..8 {
            crc = if crc & 0x8000_0000 != 0 {
                (crc << 1) ^ 0x04c1_1db7
            } else {
                crc << 1
            };
        }
    }
    crc
}

fn ogg_page(out: &mut Vec<u8>, flags: u8, granule: i64, seq: u32, packets: &[&[u8]]) {
    let mut lacing = Vec::new();
    for p in packets {
        lacing.extend(std::iter::repeat_n(255u8, p.len() / 255));
        lacing.push((p.len() % 255) as u8);
    }
    let start = out.len();
    out.extend_from_slice(b"OggS\0");
    out.push(flags);
    out.extend_from_slice(&granule.to_le_bytes());
    out.extend_from_slice(&0x5a5au32.to_le_bytes());
    out.extend_from_slice(&seq.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.push(lacing.len() as u8);
    out.extend_from_slice(&lacing);
    for p in packets {
        out.extend_from_slice(p);
    }
    let c = ogg_crc(&out[start..]);
    out[start + 22..start + 26].copy_from_slice(&c.to_le_bytes());
}

fn write_ogg(path: &str, s: &fsb5::Sample, fsb: &[u8], setup: &[u8]) {
    assert_eq!(crc32(setup), vorbis::setup_crc(fsb, s).unwrap());
    let mut ident = b"\x01vorbis".to_vec();
    ident.extend_from_slice(&0u32.to_le_bytes());
    ident.push(s.channels as u8);
    ident.extend_from_slice(&s.frequency.to_le_bytes());
    ident.extend_from_slice(&[0; 12]);
    ident.extend_from_slice(&[0xb8, 1]);
    let mut comment = b"\x03vorbis".to_vec();
    comment.extend_from_slice(&0u32.to_le_bytes());
    comment.extend_from_slice(&0u32.to_le_bytes());
    comment.push(1);
    let mut out = Vec::new();
    ogg_page(&mut out, 2, 0, 0, &[&ident]);
    ogg_page(&mut out, 0, 0, 1, &[&comment, setup]);
    let packets: Vec<&[u8]> = fsb5::vorbis_packets(&fsb[s.data.clone()]).collect();
    for (seq, (i, p)) in (2..).zip(packets.iter().enumerate()) {
        let last = i + 1 == packets.len();
        let granule = if last { s.frames as i64 } else { -1 };
        ogg_page(&mut out, if last { 4 } else { 0 }, granule, seq, &[p]);
    }
    std::fs::write(path, out).unwrap();
}
