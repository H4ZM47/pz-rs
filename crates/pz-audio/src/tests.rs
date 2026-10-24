use super::*;

#[test]
fn crc32_matches_ieee() {
    assert_eq!(vorbis_setup::crc32(b"123456789"), 0xcbf4_3926);
}

/// Build a tiny FSB5 by hand: two mono samples, names, no codec data.
fn synthetic_fsb() -> Vec<u8> {
    let mut shdr = Vec::new();
    // Sample 0: 48 kHz (index 9), mono, offset 0, 100 frames, one loop chunk.
    let w0: u64 = 1 | (9 << 1) | (100u64 << 34);
    shdr.extend_from_slice(&w0.to_le_bytes());
    let c: u32 = (8 << 1) | ((fsb5::chunk::LOOP as u32) << 25);
    shdr.extend_from_slice(&c.to_le_bytes());
    shdr.extend_from_slice(&5u32.to_le_bytes());
    shdr.extend_from_slice(&90u32.to_le_bytes());
    // Sample 1: 44.1 kHz stereo at data offset 64, 7 frames.
    let w1: u64 = (8 << 1) | (1 << 5) | (2 << 7) | (7u64 << 34);
    shdr.extend_from_slice(&w1.to_le_bytes());
    let mut names = Vec::new();
    names.extend_from_slice(&8u32.to_le_bytes());
    names.extend_from_slice(&12u32.to_le_bytes());
    names.extend_from_slice(b"one\0two\0");
    let data = vec![0xaau8; 96];
    let mut b = Vec::new();
    b.extend_from_slice(b"FSB5");
    for v in [
        1u32,
        2,
        shdr.len() as u32,
        names.len() as u32,
        data.len() as u32,
        15,
    ] {
        b.extend_from_slice(&v.to_le_bytes());
    }
    b.resize(0x3c, 0);
    b.extend_from_slice(&shdr);
    b.extend_from_slice(&names);
    b.extend_from_slice(&data);
    b
}

#[test]
fn parses_fsb5_headers() {
    let b = synthetic_fsb();
    let f = fsb5::Fsb5::parse(&b).unwrap();
    assert_eq!(f.codec, fsb5::Codec::Vorbis);
    let [a, c] = &f.samples[..] else { panic!() };
    assert_eq!((a.frequency, a.channels, a.frames), (48000, 1, 100));
    assert_eq!(a.loop_points, Some((5, 90)));
    assert_eq!(a.name.as_deref(), Some("one"));
    assert_eq!((c.frequency, c.channels, c.frames), (44100, 2, 7));
    assert_eq!(c.name.as_deref(), Some("two"));
    assert_eq!(a.data.end, c.data.start);
    assert_eq!(c.data.end - c.data.start, 32);
}

#[test]
fn splits_vorbis_packets() {
    let d = [2, 0, 1, 2, 1, 0, 9, 0, 0];
    let v: Vec<&[u8]> = fsb5::vorbis_packets(&d).collect();
    assert_eq!(v, vec![&[1u8, 2][..], &[9][..]]);
}

#[test]
fn setup_table_with_patch() {
    // A fake library image: a base packet, a patch, and two table entries.
    let mut lib = vec![0u8; 0x100];
    let base: Vec<u8> = b"\x05vorbis"
        .iter()
        .copied()
        .chain((0..120u8).map(|i| i ^ 0x5a))
        .collect();
    let base_at = lib.len();
    lib.extend_from_slice(&base);
    let patch = [1u8, 2, 3, 4];
    let patch_at = lib.len();
    lib.extend_from_slice(&patch);
    while !lib.len().is_multiple_of(8) {
        lib.push(0);
    }
    let mut patched = base[..100].to_vec();
    patched[50..54].copy_from_slice(&patch);
    // Pretend the image is loaded at 0x1000 to exercise delta discovery.
    let va = |o: usize| (o + 0x1000) as u64;
    let mut entry = |data: u64, size: u32, crc: u32, base: u64, off: u32, len: u32| {
        lib.extend_from_slice(&data.to_le_bytes());
        lib.extend_from_slice(&size.to_le_bytes());
        lib.extend_from_slice(&crc.to_le_bytes());
        lib.extend_from_slice(&base.to_le_bytes());
        lib.extend_from_slice(&off.to_le_bytes());
        lib.extend_from_slice(&len.to_le_bytes());
    };
    entry(
        va(base_at),
        base.len() as u32,
        vorbis_setup::crc32(&base),
        0,
        0,
        0,
    );
    entry(
        va(patch_at),
        100,
        vorbis_setup::crc32(&patched),
        va(base_at),
        50,
        4,
    );
    let s = vorbis_setup::SetupHeaders::from_fmod_library(&lib);
    assert_eq!(s.len(), 2);
    assert_eq!(s.get(vorbis_setup::crc32(&patched)), Some(&patched[..]));
}

#[test]
fn guid_round_trip() {
    let s = "{12345678-9abc-4def-8123-456789abcdef}";
    let g = reader::Guid::parse(s).unwrap();
    assert_eq!(g.0[0], 0x78);
    assert_eq!(g.to_string(), s);
}

#[test]
fn counts_and_struct_arrays() {
    let d = [
        0x05, 0x00, 0x02, 0x00, 1, 2, 3, 4, 0xbe, 0x8a, 0x01, 0x00, 0x01, 0x00,
    ];
    let mut r = reader::Reader::new(&d);
    let v = r.read_structs(|r| r.u8()).unwrap();
    assert_eq!(v, vec![1, 3]);
    assert_eq!(r.count().unwrap(), 35518);
    assert!(r.read_structs(|r| r.u8()).unwrap().is_empty());
    // Unsized form: each element has its own size.
    let d = [0x04, 0x00, 0x01, 0x00, 7, 0x02, 0x00, 8, 9];
    let mut r = reader::Reader::new(&d);
    assert_eq!(r.read_structs(|r| r.u8()).unwrap(), vec![7, 8]);
}

/// A string table built by hand: "bus:/", "bus:/Reverb" and "event:/A".
#[test]
fn parses_string_table() {
    let labels = b"bus:/\0reverb\0event:/A\0";
    let mut d = 1u32.to_le_bytes().to_vec();
    let node = |d: &mut Vec<u8>, label: u32, c: u8, child: u32, n: u8| {
        d.extend_from_slice(&label.to_le_bytes()[..3]);
        d.push(c);
        d.extend_from_slice(&child.to_le_bytes()[..3]);
        d.push(n);
    };
    d.extend_from_slice(&((5u16 << 1) | 1).to_le_bytes());
    d.extend_from_slice(&8u16.to_le_bytes());
    node(&mut d, 0xff_ffff, 0, 1, 2); // 0 root
    node(&mut d, 0, b'b', 3, 2); // 1 "bus:/"
    node(&mut d, 13, b'e', 2, 0); // 2 "event:/A" -> GUID 2
    node(&mut d, 0xff_ffff, 0, 0, 0); // 3 end of "bus:/" -> GUID 0
    node(&mut d, 6, b'r', 1, 0); // 4 "reverb" -> GUID 1
    d.extend_from_slice(&((3u16 << 1) | 1).to_le_bytes());
    d.extend_from_slice(&16u16.to_le_bytes());
    for i in 0..3u8 {
        d.extend_from_slice(&[i + 1; 16]);
    }
    d.extend_from_slice(&(labels.len() as u16).to_le_bytes());
    d.extend_from_slice(labels);
    d.extend_from_slice(&3u16.to_le_bytes());
    for leaf in [3u32, 4, 2] {
        d.extend_from_slice(&leaf.to_le_bytes()[..3]);
    }
    d.extend_from_slice(&5u16.to_le_bytes());
    for parent in [0xff_ffffu32, 0, 0, 1, 1] {
        d.extend_from_slice(&parent.to_le_bytes()[..3]);
    }
    let t = strings::StringTable::parse(&d).unwrap();
    assert_eq!(t.path(&reader::Guid([1; 16])), Some("bus:/"));
    assert_eq!(t.path(&reader::Guid([2; 16])), Some("bus:/reverb"));
    assert_eq!(t.lookup("EVENT:/a"), Some(reader::Guid([3; 16])));
}

/// Parity checks against an installed game:
/// `PZ_GAME_DIR=... cargo test -p pz-audio --release -- --ignored`.
#[test]
#[ignore]
fn decodes_every_game_bank() {
    let Some(dir) = std::env::var_os("PZ_GAME_DIR") else {
        return;
    };
    let dir = std::path::PathBuf::from(dir);
    let setups = vorbis_setup::SetupHeaders::from_game_dir(&dir).unwrap();
    assert!(
        !setups.is_empty(),
        "no Vorbis setup headers found in the FMOD library"
    );
    let mut banks = Vec::new();
    for sub in ["media/sound/banks/Desktop", "media/sound/export/Desktop"] {
        for e in std::fs::read_dir(dir.join(sub)).unwrap().flatten() {
            let p = e.path();
            if p.extension().is_some_and(|x| x == "bank") {
                banks.push(p);
            }
        }
    }
    let mut total = 0;
    for p in banks {
        let bank = BankFile::open(&p).unwrap();
        let Some(fsb) = &bank.fsb else { continue };
        for (i, s) in fsb.samples.iter().enumerate() {
            let crc = vorbis::setup_crc(bank.fsb_bytes(), s).unwrap();
            assert!(
                setups.get(crc).is_some(),
                "{}: sample {i} needs setup {crc:08x}",
                p.display()
            );
        }
        // Decoding all ~19k samples takes long; decode a spread of them fully.
        let step = (fsb.samples.len() / 40).max(1);
        for i in (0..fsb.samples.len()).step_by(step) {
            let s = &fsb.samples[i];
            let pcm = bank.decoder(i, &setups).unwrap().decode_all().unwrap();
            assert_eq!(
                pcm.len(),
                s.frames as usize * s.channels as usize,
                "{} #{i}",
                p.display()
            );
            assert!(pcm.iter().all(|x| x.is_finite() && x.abs() <= 1.5));
            total += 1;
        }
    }
    assert!(total > 0);
}

/// Every object in the game's banks parses and every reference an event
/// makes resolves: `PZ_GAME_DIR=... cargo test -p pz-audio --release -- --ignored`.
#[test]
#[ignore]
fn parses_every_game_bank_model() {
    let Some(dir) = std::env::var_os("PZ_GAME_DIR") else {
        return;
    };
    let banks = std::path::PathBuf::from(dir).join("media/sound/banks/Desktop");
    let mut events = 0;
    for name in ["ZomboidSound", "ZomboidMusic", "ZomboidSoundMP"] {
        let bank = BankFile::open(&banks.join(format!("{name}.bank"))).unwrap();
        let m = bank.model().unwrap();
        for e in m.events.values() {
            assert!(
                m.timelines.contains_key(&e.timeline),
                "{name}: {} timeline",
                e.id
            );
            assert_eq!(e.parameter_sheets.len(), e.parameter_ids.len());
            assert_eq!(
                m.event_parameters(e, &[]).len(),
                e.parameter_sheets.len(),
                "{name}: {}",
                e.id
            );
            events += 1;
        }
        for i in m.instruments.values() {
            if let bank::InstrumentKind::Single { waveform } = &i.kind {
                let w = m.waveforms.get(waveform).expect("waveform");
                assert!((w.subsound as usize) < bank.fsb.as_ref().unwrap().samples.len());
            }
        }
    }
    assert!(events > 0);
}

#[test]
fn compressor_below_threshold_applies_makeup_only() {
    use dsp::{compressor::Compressor, Dsp};
    let mut c = Compressor::new(48000);
    for (i, v) in [-20.0, 3.0, 10.0, 100.0, 6.0].into_iter().enumerate() {
        c.set_float(i, v);
    }
    let mut buf = vec![0.01f32; 2 * 4800];
    c.process(&mut buf, 2);
    let g = buf[buf.len() - 1] / 0.01;
    assert!((g - 10f32.powf(6.0 / 20.0)).abs() < 1e-4, "{g}");
    // A loud signal settles to (1 - 1/ratio) of the overshoot in power.
    let mut buf = vec![1.0f32; 2 * 48000];
    c.process(&mut buf, 2);
    let out_db = 20.0 * buf[buf.len() - 1].log10();
    // Linked detection sums both channels' power: +3 dB over threshold+3.
    let over = 10.0 * 2f32.log10() + 20.0;
    let expect = -(1.0 - 1.0 / 3.0) * over + 6.0;
    assert!((out_db - expect).abs() < 0.05, "{out_db} vs {expect}");
}
