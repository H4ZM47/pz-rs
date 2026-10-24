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
