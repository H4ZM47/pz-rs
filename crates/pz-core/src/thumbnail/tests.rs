use super::*;

/// Inflate a zlib stream with fixed-Huffman and stored blocks only (all
/// [`zlib_compress`] writes), to check the encoder round-trips.
fn inflate(z: &[u8]) -> Vec<u8> {
    struct R<'a> {
        d: &'a [u8],
        pos: usize,
        bit: u32,
    }
    impl R<'_> {
        fn bit(&mut self) -> u32 {
            let b = (self.d[self.pos] >> self.bit) & 1;
            self.bit += 1;
            if self.bit == 8 {
                self.bit = 0;
                self.pos += 1;
            }
            b as u32
        }
        fn bits(&mut self, n: u32) -> u32 {
            (0..n).fold(0, |v, i| v | (self.bit() << i))
        }
        fn rev(&mut self, n: u32) -> u32 {
            (0..n).fold(0, |v, _| (v << 1) | self.bit())
        }
    }
    let mut r = R {
        d: &z[2..],
        pos: 0,
        bit: 0,
    };
    let mut out = Vec::new();
    loop {
        let last = r.bits(1);
        let ty = r.bits(2);
        if ty == 0 {
            if r.bit != 0 {
                r.bit = 0;
                r.pos += 1;
            }
            let n = u16::from_le_bytes([r.d[r.pos], r.d[r.pos + 1]]) as usize;
            r.pos += 4;
            out.extend_from_slice(&r.d[r.pos..r.pos + n]);
            r.pos += n;
        } else {
            assert_eq!(ty, 1);
            loop {
                let mut c = r.rev(7);
                let sym = if c <= 0x17 {
                    c + 256
                } else {
                    c = (c << 1) | r.bit();
                    if (0x30..=0xbf).contains(&c) {
                        c - 0x30
                    } else if (0xc0..=0xc7).contains(&c) {
                        c - 0xc0 + 280
                    } else {
                        c = (c << 1) | r.bit();
                        c - 0x190 + 144
                    }
                };
                if sym < 256 {
                    out.push(sym as u8);
                    continue;
                }
                if sym == 256 {
                    break;
                }
                const LB: [u32; 29] = [
                    3, 4, 5, 6, 7, 8, 9, 10, 11, 13, 15, 17, 19, 23, 27, 31, 35, 43, 51, 59, 67,
                    83, 99, 115, 131, 163, 195, 227, 258,
                ];
                const LE: [u32; 29] = [
                    0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4, 5, 5,
                    5, 5, 0,
                ];
                const DB: [u32; 30] = [
                    1, 2, 3, 4, 5, 7, 9, 13, 17, 25, 33, 49, 65, 97, 129, 193, 257, 385, 513, 769,
                    1025, 1537, 2049, 3073, 4097, 6145, 8193, 12289, 16385, 24577,
                ];
                let k = (sym - 257) as usize;
                let l = LB[k] + r.bits(LE[k]);
                let dk = r.rev(5) as usize;
                let de = if dk < 2 { 0 } else { (dk as u32 - 2) / 2 };
                let d = (DB[dk] + r.bits(de)) as usize;
                for _ in 0..l {
                    out.push(out[out.len() - d]);
                }
            }
        }
        if last == 1 {
            break;
        }
    }
    out
}

#[test]
fn deflate_round_trips() {
    let mut data = Vec::new();
    for i in 0..20000u32 {
        data.push((i % 7 + (i / 300) % 5) as u8);
    }
    let z = zlib_compress(&data, 8);
    assert_eq!(&z[..2], [0x78, 0x5e]);
    assert!(z.len() < data.len() / 4);
    assert_eq!(inflate(&z), data);
    // Noise does not compress: stored blocks.
    let mut x = 12345u32;
    let noise: Vec<u8> = (0..70000)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            x as u8
        })
        .collect();
    let z = zlib_compress(&noise, 8);
    assert_eq!(z.len(), 2 + noise.len() + 3 * 5 + 4);
    assert_eq!(inflate(&z), noise);
}

#[test]
fn png_layout() {
    let rgba: Vec<u8> = (0..4 * 4 * 4).map(|i| (i * 13) as u8).collect();
    let png = stbi_write_png(&rgba, 4, 4, 4, true);
    assert_eq!(&png[..8], [137, 80, 78, 71, 13, 10, 26, 10]);
    assert_eq!(&png[12..16], b"IHDR");
    assert_eq!(&png[16..29], [0, 0, 0, 4, 0, 0, 0, 4, 8, 6, 0, 0, 0]);
    // The CRC of IHDR covers the tag and data.
    assert_eq!(&png[29..33], crc32(&png[12..29]).to_be_bytes());
    assert_eq!(
        &png[png.len() - 12..],
        [0, 0, 0, 0, b'I', b'E', b'N', b'D', 0xae, 0x42, 0x60, 0x82]
    );
    // The first PNG row is the last GL row.
    let idat_len = u32::from_be_bytes(png[33..37].try_into().unwrap()) as usize;
    let raw = inflate(&png[41..41 + idat_len]);
    assert_eq!(raw.len(), 4 * 17);
    let f = raw[0];
    assert!(f <= 4);
    if f == 0 {
        assert_eq!(&raw[1..17], &rgba[48..64]);
    }
}

#[test]
fn read_rect_is_centred() {
    assert_eq!(
        screen_read_rect(0, 0, 1920, 1080),
        ReadRect {
            x: 832,
            y: 412,
            width: 256,
            height: 256
        }
    );
    assert_eq!(screen_read_rect(10, 0, 200, 100).width, 200);
    assert_eq!(thumbnail_player(&[false, true, true]), Some(1));
    assert_eq!(thumbnail_player(&[false]), None);
}
