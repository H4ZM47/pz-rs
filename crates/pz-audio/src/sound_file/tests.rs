use super::decode;

fn chunk(id: &[u8; 4], data: &[u8]) -> Vec<u8> {
    let mut bytes = id.to_vec();
    bytes.extend((data.len() as u32).to_le_bytes());
    bytes.extend(data);
    if data.len() & 1 != 0 {
        bytes.push(0);
    }
    bytes
}
fn riff(chunks: &[u8]) -> Vec<u8> {
    let mut bytes = b"RIFF".to_vec();
    bytes.extend((4 + chunks.len() as u32).to_le_bytes());
    bytes.extend(b"WAVE");
    bytes.extend(chunks);
    bytes
}
fn format(tag: u16, channels: u16, rate: u32, bits: u16) -> Vec<u8> {
    let mut bytes = tag.to_le_bytes().to_vec();
    bytes.extend(channels.to_le_bytes());
    bytes.extend(rate.to_le_bytes());
    bytes.extend((rate * channels as u32 * u32::from(bits / 8)).to_le_bytes());
    bytes.extend((channels * (bits / 8)).to_le_bytes());
    bytes.extend(bits.to_le_bytes());
    bytes
}
fn wave(tag: u16, channels: u16, rate: u32, bits: u16, samples: &[u8]) -> Vec<u8> {
    let mut chunks = chunk(b"fmt ", &format(tag, channels, rate, bits));
    chunks.extend(chunk(b"data", samples));
    riff(&chunks)
}

#[test]
fn truncated_fmt_claiming_sixteen_bytes_returns_error() {
    let mut chunks = b"fmt ".to_vec();
    chunks.extend(16u32.to_le_bytes());
    assert!(decode(&riff(&chunks)).is_err());
}
#[test]
fn truncated_extensible_fmt_claiming_twenty_six_bytes_returns_error() {
    let mut chunks = b"fmt ".to_vec();
    chunks.extend(26u32.to_le_bytes());
    chunks.extend(0xfffeu16.to_le_bytes());
    assert!(decode(&riff(&chunks)).is_err());
}
#[test]
fn every_truncated_prefix_of_a_complete_wave_returns_error() {
    let bytes = wave(1, 2, 44100, 16, &[0, 64, 0, 192]);
    for len in 0..bytes.len() {
        assert!(
            decode(&bytes[..len]).is_err(),
            "accepted truncated prefix {len}"
        );
    }
    let file = decode(&bytes).unwrap();
    assert_eq!((file.rate, file.pcm.channels), (44100, 2));
    assert_eq!(file.pcm.data, [0.5, -0.5]);
}
#[test]
fn oversized_unknown_chunk_and_partial_chunk_header_return_error() {
    let mut chunks = b"JUNK".to_vec();
    chunks.extend(u32::MAX.to_le_bytes());
    assert!(decode(&riff(&chunks)).is_err());
    assert!(decode(&riff(b"fmt")).is_err());
}
#[test]
fn short_format_and_missing_extensible_subtype_return_error() {
    for (tag, size) in [(1, 14), (0xfffe, 16), (0xfffe, 24), (0xfffe, 25)] {
        let mut fmt = format(tag, 1, 48000, 16);
        fmt.resize(size, 0);
        let mut chunks = chunk(b"fmt ", &fmt);
        chunks.extend(chunk(b"data", &[0, 64]));
        assert!(decode(&riff(&chunks)).is_err(), "accepted {tag:#x}/{size}");
    }
}
#[test]
fn zero_channels_and_sample_rate_are_rejected() {
    assert!(decode(&wave(1, 0, 48000, 16, &[0, 64])).is_err());
    assert!(decode(&wave(1, 1, 0, 16, &[0, 64])).is_err());
}
#[test]
fn odd_unknown_chunk_padding_is_consumed_before_format() {
    let mut chunks = chunk(b"JUNK", &[42]);
    chunks.extend(chunk(b"fmt ", &format(1, 1, 48000, 16)));
    chunks.extend(chunk(b"data", &[0, 64]));
    let file = decode(&riff(&chunks)).unwrap();
    assert_eq!((file.rate, file.pcm.channels), (48000, 1));
    assert_eq!(file.pcm.data, [0.5]);
}
#[test]
fn existing_pcm_widths_and_float_values_are_preserved() {
    let cases: Vec<(u16, u16, Vec<u8>)> = vec![
        (1, 8, vec![0, 128, 192]),
        (
            1,
            16,
            [-32768i16, 0, 16384]
                .into_iter()
                .flat_map(i16::to_le_bytes)
                .collect(),
        ),
        (1, 24, vec![0, 0, 128, 0, 0, 0, 0, 0, 64]),
        (
            1,
            32,
            [i32::MIN, 0, 1073741824]
                .into_iter()
                .flat_map(i32::to_le_bytes)
                .collect(),
        ),
        (
            3,
            32,
            [-1.0f32, 0.0, 0.5]
                .into_iter()
                .flat_map(f32::to_le_bytes)
                .collect(),
        ),
    ];
    for (tag, bits, data) in cases {
        let file = decode(&wave(tag, 1, 48000, bits, &data)).unwrap();
        assert_eq!((file.rate, file.pcm.channels), (48000, 1));
        assert_eq!(file.pcm.data, [-1.0, 0.0, 0.5], "{tag}/{bits}");
    }
}
