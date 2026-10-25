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
fn pan_matches_measured_fmod_vbap_and_rolloff() {
    use dsp::pan::{spread, vbap, Rolloff};
    let close = |a: f32, b: f32| (a - b).abs() < 2e-3;
    let g = vbap(45.0);
    assert!(close(g[1], 0.962) && close(g[5], 0.275), "{g:?}");
    let g = vbap(90.0);
    assert!(close(g[1], 0.367) && close(g[5], 0.930), "{g:?}");
    let g = vbap(180.0);
    assert!(close(g[4], 0.707) && close(g[5], 0.707), "{g:?}");
    let g = spread(0.0, 60.0);
    assert!(close(g[2], 0.816) && close(g[0], 0.408), "{g:?}");
    assert!(close(Rolloff::Linear.gain(5.0, 2.0, 20.0), 0.8333));
    assert!(close(Rolloff::LinearSquared.gain(5.0, 2.0, 20.0), 0.6944));
    assert!(close(Rolloff::Inverse.gain(25.0, 2.0, 20.0), 0.1));
    assert!(close(Rolloff::InverseTapered.gain(15.0, 2.0, 20.0), 0.0772));
}

/// A 16-bit PCM WAVE file of `samples` (interleaved).
fn wav16(channels: u16, rate: u32, samples: &[i16]) -> Vec<u8> {
    let data: Vec<u8> = samples.iter().flat_map(|s| s.to_le_bytes()).collect();
    let mut b = b"RIFF".to_vec();
    b.extend((36 + data.len() as u32).to_le_bytes());
    b.extend(b"WAVEfmt ");
    b.extend(16u32.to_le_bytes());
    b.extend(1u16.to_le_bytes());
    b.extend(channels.to_le_bytes());
    b.extend(rate.to_le_bytes());
    b.extend((rate * channels as u32 * 2).to_le_bytes());
    b.extend((channels * 2).to_le_bytes());
    b.extend(16u16.to_le_bytes());
    b.extend(b"data");
    b.extend((data.len() as u32).to_le_bytes());
    b.extend(data);
    b
}

#[test]
fn decodes_wave_files() {
    let f = sound_file::decode(&wav16(2, 44100, &[16384, -16384, 0, 32767])).unwrap();
    assert_eq!((f.rate, f.pcm.channels, f.pcm.frames()), (44100, 2, 2));
    assert_eq!(f.pcm.data[..2], [0.5, -0.5]);
    assert!(sound_file::decode(b"nonsense").is_err());
}

#[test]
fn core_channels_play_loop_pitch_and_stop() {
    use channels::Channels;
    let one = |n: usize| sound_file::decode(&wav16(1, 48000, &vec![16384; n])).unwrap();
    let mut c = Channels::default();
    let s = c.add_sound(one(100), false);
    let ch = c.play(s, true).unwrap();
    let mut out = vec![0.0; 2 * 64];
    c.mix(&mut out, |p| p);
    assert!(out.iter().all(|&x| x == 0.0), "paused");
    c.set_volume(ch, 0.5);
    c.set_paused(ch, false);
    c.mix(&mut out, |p| p);
    assert_eq!((out[0], out[1]), (0.25, 0.25));
    // 100 frames: done during the second block.
    let mut out = vec![0.0; 2 * 64];
    c.mix(&mut out, |p| p);
    assert_eq!(out[2 * 35], 0.25);
    assert_eq!(out[2 * 36], 0.0);
    c.reap();
    assert!(!c.is_playing(ch));
    // A reused slot does not answer to the old id.
    let ch2 = c.play(s, false).unwrap();
    assert_ne!(ch, ch2);
    assert!(c.is_playing(ch2) && !c.is_playing(ch));
    c.set_looped(ch2, true);
    c.set_pitch(ch2, 2.0);
    for _ in 0..10 {
        c.mix(&mut vec![0.0; 128], |p| p);
    }
    assert!(c.is_playing(ch2), "looping");
    c.stop(ch2);
    assert!(!c.is_playing(ch2));
}

#[test]
fn core_channels_attenuate_in_3d() {
    use channels::Channels;
    let mut c = Channels::default();
    let s = c.add_sound(sound_file::decode(&wav16(1, 48000, &[16384; 64])).unwrap(), false);
    let level = |c: &mut Channels, pos: [f32; 3]| {
        let ch = c.play(s, false).unwrap();
        c.set_3d(ch, true);
        c.set_min_max_distance(ch, 1.0, 100.0);
        c.set_position(ch, pos);
        let mut out = vec![0.0; 8];
        c.mix(&mut out, |p| p);
        c.stop(ch);
        (out[0], out[1])
    };
    let (l, r) = level(&mut c, [0.0, 0.0, 4.0]);
    // Inverse rolloff: 1/4 at four times the minimum distance.
    assert!((l - r).abs() < 1e-6);
    let (l1, _) = level(&mut c, [0.0, 0.0, 1.0]);
    assert!((l / l1 - 0.25).abs() < 1e-4, "{l} {l1}");
    let (l, r) = level(&mut c, [3.0, 0.0, 0.0]);
    assert!(r > l, "right of the listener");
}
