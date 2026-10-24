use std::path::{Path, PathBuf};

use super::*;
use crate::image::ImageData;

fn encode(w: u32, h: u32, color: ::png::ColorType, depth: ::png::BitDepth, data: &[u8], setup: impl FnOnce(&mut ::png::Encoder<&mut Vec<u8>>)) -> Vec<u8> {
    let mut out = Vec::new();
    {
        let mut e = ::png::Encoder::new(&mut out, w, h);
        e.set_color(color);
        e.set_depth(depth);
        setup(&mut e);
        let mut wr = e.write_header().unwrap();
        wr.write_image_data(data).unwrap();
    }
    out
}

fn pixel(img: &ImageData, x: i32, y: i32) -> [u8; 4] {
    let i = ((y * img.width_hw + x) * 4) as usize;
    img.data.pixels[i..i + 4].try_into().unwrap()
}

#[test]
fn rgba_with_mask_and_padding() {
    // 3x2 RGBA, filtered rows exercise every unfilter path via the encoder's
    // adaptive filtering.
    let mut data = Vec::new();
    for i in 0..6u8 {
        data.extend_from_slice(&[i * 40, 255 - i * 30, i * 7, if i % 2 == 0 { 0 } else { 200 }]);
    }
    let bytes = encode(3, 2, ::png::ColorType::Rgba, ::png::BitDepth::Eight, &data, |e| e.set_filter(::png::FilterType::Paeth));
    let img = ImageData::from_png(&bytes[..], true).unwrap();
    assert_eq!((img.width, img.height, img.width_hw, img.height_hw), (3, 2, 4, 2));
    assert_eq!(pixel(&img, 1, 0), [40, 225, 7, 200]);
    assert_eq!(pixel(&img, 3, 1), [0, 0, 0, 0]);
    let mask = img.mask.as_ref().unwrap();
    assert!(!mask.value(0, 0));
    assert!(mask.value(1, 0));
    assert!(mask.value(0, 1));
}

#[test]
fn paletted_and_grey_transparency_keys() {
    // 4-bit palette, entry 1 transparent.
    let bytes = encode(3, 1, ::png::ColorType::Indexed, ::png::BitDepth::Four, &[0x01, 0x20], |e| {
        e.set_palette(vec![10, 20, 30, 40, 50, 60, 70, 80, 90]);
        e.set_trns(vec![255, 0]);
    });
    let img = ImageData::from_png(&bytes[..], false).unwrap();
    assert_eq!(pixel(&img, 0, 0), [10, 20, 30, 255]);
    assert_eq!(pixel(&img, 1, 0), [40, 50, 60, 0]);
    assert_eq!(pixel(&img, 2, 0), [70, 80, 90, 255]);

    // Grey keys compare the high byte of the 16-bit tRNS sample.
    let bytes = encode(2, 1, ::png::ColorType::Grayscale, ::png::BitDepth::Eight, &[0, 5], |e| e.set_trns(vec![0, 5]));
    let img = ImageData::from_png(&bytes[..], false).unwrap();
    assert_eq!(pixel(&img, 0, 0), [0, 0, 0, 0]);
    assert_eq!(pixel(&img, 1, 0), [5, 5, 5, 255]);
}

#[test]
fn rgb_key_mask_uses_transparent_count() {
    // Pixels 0 and 2 match the key; the original marks mask bits 0 and 1.
    let data = [9, 9, 9, 1, 1, 1, 9, 9, 9];
    let bytes = encode(3, 1, ::png::ColorType::Rgb, ::png::BitDepth::Eight, &data, |e| e.set_trns(vec![0, 9, 0, 9, 0, 9]));
    let img = ImageData::from_png(&bytes[..], true).unwrap();
    assert_eq!(pixel(&img, 0, 0), [9, 9, 9, 0]);
    let m = img.mask.as_ref().unwrap();
    assert_eq!([m.value(0, 0), m.value(1, 0), m.value(2, 0)], [true, true, false]);
}

#[test]
fn rejects_what_the_original_rejects() {
    let bytes = encode(1, 1, ::png::ColorType::Rgba, ::png::BitDepth::Sixteen, &[0; 8], |_| {});
    assert!(ImageData::from_png(&bytes[..], false).is_err());
    let mut bad = encode(1, 1, ::png::ColorType::Rgba, ::png::BitDepth::Eight, &[0; 4], |_| {});
    bad[29] ^= 1; // IHDR CRC
    assert!(ImageData::from_png(&bad[..], false).is_err());
    assert!(ImageData::from_png(&b"not a png"[..], false).is_err());
}

#[test]
fn decodes_like_a_reference_decoder() {
    let (w, h) = (37u32, 23u32);
    let data: Vec<u8> = (0..w * h * 4).map(|i| (i * 7 % 251) as u8).collect();
    for f in [::png::FilterType::NoFilter, ::png::FilterType::Sub, ::png::FilterType::Up, ::png::FilterType::Avg, ::png::FilterType::Paeth] {
        let bytes = encode(w, h, ::png::ColorType::Rgba, ::png::BitDepth::Eight, &data, |e| e.set_filter(f));
        let img = ImageData::from_png(&bytes[..], false).unwrap();
        for y in 0..h as i32 {
            for x in 0..w as i32 {
                let i = ((y as u32 * w + x as u32) * 4) as usize;
                assert_eq!(pixel(&img, x, y), data[i..i + 4], "{f:?} at {x},{y}");
            }
        }
    }
}

// ---- texture manager ------------------------------------------------------

struct Scratch(PathBuf);

impl Scratch {
    fn new(tag: &str) -> Self {
        let p = std::env::temp_dir().join(format!("pz-render-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).unwrap();
        Scratch(p)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn write(p: &Path, bytes: &[u8]) {
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, bytes).unwrap();
}

/// A version-1 pack with one 8x4 page: "Left" (opaque) and "Right"
/// (transparent, offset 1,2 in a 6x6 frame).
fn pack_bytes() -> Vec<u8> {
    let mut px = Vec::new();
    for _y in 0..4 {
        for x in 0..8 {
            px.extend_from_slice(if x < 4 { &[255, 0, 0, 255] } else { &[0, 0, 255, 0] });
        }
    }
    let page = encode(8, 4, ::png::ColorType::Rgba, ::png::BitDepth::Eight, &px, |_| {});
    let mut b = b"PZPK".to_vec();
    let int = |v: i32, b: &mut Vec<u8>| b.extend_from_slice(&v.to_le_bytes());
    int(1, &mut b);
    int(1, &mut b);
    int(4, &mut b);
    b.extend_from_slice(b"page");
    int(2, &mut b);
    int(1, &mut b);
    for (name, vals) in [("Left", [0, 0, 4, 4, 0, 0, 4, 4]), ("Right", [4, 0, 4, 4, 1, 2, 6, 6])] {
        int(name.len() as i32, &mut b);
        b.extend_from_slice(name.as_bytes());
        for v in vals {
            int(v, &mut b);
        }
    }
    int(page.len() as i32, &mut b);
    b.extend_from_slice(&page);
    b
}

#[test]
fn shared_textures_from_packs_and_files() {
    let s = Scratch::new("shared");
    let game = s.0.join("game");
    write(&game.join("media/texturepacks/UI.pack"), &pack_bytes());
    write(&game.join("media/ui/Loose.png"), &encode(5, 3, ::png::ColorType::Rgb, ::png::BitDepth::Eight, &[128; 45], |_| {}));
    write(&game.join("media/ui/Broken.png"), b"junk");
    let mut fs = pz_fs::GameFs::new(&game);
    fs.set_cache_dir(s.0.join("user"));
    let mut tm = TextureManager::new();
    tm.load_texture_pack(&fs, "UI", flags::FILTER_MAG_NEAREST, None);
    tm.set_texture_pack_lookup(&[]);

    let left = tm.shared_texture(&fs, "Left").unwrap();
    assert!(left.ready);
    assert_eq!((left.width, left.height, left.x_start, left.x_end, left.y_end), (4, 4, 0.0, 0.5, 1.0));
    // Any folder or ".png" suffix finds the pack entry by its base name.
    let right = tm.shared_texture(&fs, "media/ui/Right.png").unwrap();
    assert_eq!(right.id, left.id);
    assert_eq!((right.x_start, right.offset_x, right.width_orig()), (0.5, 1.0, 6));
    assert!(!right.is_mask_set(2, 3));
    assert!(left.is_mask_set(0, 0));
    let d = tm.data(left.id.unwrap()).unwrap();
    assert_eq!(d.flags, flags::FILTER_MAG_NEAREST | flags::CREATE_MASK);
    assert_eq!(d.upload_levels().len(), 1);

    let loose = tm.shared_texture(&fs, "media/ui/loose.png").unwrap();
    assert!(loose.ready);
    assert_eq!((loose.width, loose.height, loose.width_hw, loose.x_end), (5, 3, 8, 0.625));
    assert_eq!(tm.data(loose.id.unwrap()).unwrap().flags, flags::COMPRESS);

    let broken = tm.shared_texture(&fs, "media/ui/Broken.png").unwrap();
    assert!(!broken.ready);
    assert!(tm.shared_texture(&fs, "media/ui/Missing.png").is_none());
    assert!(tm.shared_texture(&fs, "NoSuchPackEntry").is_none());

    let white = tm.white();
    assert_eq!(tm.shared_texture(&fs, "media/ui/white.png").unwrap().id, white.id);
    let err = tm.error_texture();
    let px = &tm.data(err.id.unwrap()).unwrap().image.as_ref().unwrap().data.pixels;
    assert_eq!(&px[0..4], &[255, 0, 0, 255]);
    assert_eq!(&px[16..20], &[255, 255, 255, 255]);
    assert_eq!(&px[4 * 32 * 4..4 * 32 * 4 + 4], &[255, 255, 255, 255]);
}

#[test]
fn draw_list_matches_sprite_renderer() {
    let mut t = Texture::empty("t");
    t.ready = true;
    t.width_hw = 64;
    t.height_hw = 64;
    t.offset_x = 2.0;
    t.set_region(16, 0, 16, 32);
    let mut dl = DrawList::default();
    dl.render_texture(&t, 10.0, 20.0, 16.0, 32.0, Rgba(1.0, 0.5, 0.0, 1.0));
    dl.render(Some(&t), 0.0, 0.0, 1.0, 1.0, Rgba(1.0, 1.0, 1.0, 0.0));
    dl.render_clamped(Some(&t), 0, 0, 16, 32, [0, 8, 16, 16], Rgba(1.0, 1.0, 1.0, 1.0));
    assert_eq!(dl.cmds.len(), 2);
    let DrawCmd::Sprite(s) = &dl.cmds[0] else { panic!() };
    assert_eq!(s.x, [12.0, 28.0, 28.0, 12.0]);
    assert_eq!(s.u, [0.25, 0.5, 0.5, 0.25]);
    assert_eq!(s.color[0], 0xFF00_7FFF);
    let DrawCmd::Sprite(c) = &dl.cmds[1] else { panic!() };
    assert_eq!(c.y, [8.0, 8.0, 24.0, 24.0]);
    assert_eq!(c.v, [0.125, 0.125, 0.375, 0.375]);
}

/// Decodes every page of every pack in an installed game and compares the
/// pixels with an independent PNG decoder:
/// `PZ_GAME_DIR=... cargo test -p pz-render --release -- --ignored`.
#[test]
#[ignore]
fn installed_game_packs() {
    let Some(dir) = std::env::var_os("PZ_GAME_DIR") else { return };
    let fs = pz_fs::GameFs::new(PathBuf::from(dir));
    let packs = std::fs::read_dir(fs.media_file("texturepacks")).unwrap();
    let (mut pages, mut subs, mut failed) = (0, 0, Vec::new());
    for e in packs.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        let Some(pack) = name.strip_suffix(".pack") else { continue };
        let dev = match pz_fs::pack::TexturePackDevice::open(pack, &e.path().to_string_lossy(), 0) {
            Ok(d) => d,
            Err(err) => {
                failed.push(format!("{pack}: {err}"));
                continue;
            }
        };
        subs += dev.sub_textures().len();
        for page in &dev.pages {
            pages += 1;
            let img = match ImageData::from_png(dev.open_page(&page.name).unwrap(), page.has_alpha) {
                Ok(i) => i,
                Err(err) => {
                    failed.push(format!("{pack}/{}: {err}", page.name));
                    continue;
                }
            };
            let mut dec = ::png::Decoder::new(dev.open_page(&page.name).unwrap());
            dec.set_transformations(::png::Transformations::EXPAND);
            let mut r = dec.read_info().unwrap();
            let mut buf = vec![0; r.output_buffer_size()];
            let info = r.next_frame(&mut buf).unwrap();
            assert_eq!(info.color_type, ::png::ColorType::Rgba, "{pack}/{}", page.name);
            for y in 0..img.height as usize {
                let a = &img.data.pixels[y * img.width_hw as usize * 4..][..img.width as usize * 4];
                let b = &buf[y * info.line_size..][..img.width as usize * 4];
                assert!(a == b, "{pack}/{} row {y} differs", page.name);
            }
        }
    }
    eprintln!("pages {pages}, sub-textures {subs}, failed {failed:?}");
    assert!(failed.is_empty());
}

/// Decodes every loose `.png` under `media/` and reports those the
/// original's decoder rejects too (16-bit, interlaced, ...).
#[test]
#[ignore]
fn installed_game_loose_pngs() {
    let Some(dir) = std::env::var_os("PZ_GAME_DIR") else { return };
    let fs = pz_fs::GameFs::new(PathBuf::from(dir));
    let (mut ok, mut rejected) = (0, std::collections::BTreeMap::<String, usize>::new());
    for abs in fs.active_file_map.values().filter(|p| p.to_lowercase().ends_with(".png")) {
        match ImageData::from_png(std::io::BufReader::new(std::fs::File::open(abs).unwrap()), false) {
            Ok(_) => ok += 1,
            Err(e) => *rejected.entry(e.to_string()).or_default() += 1,
        }
    }
    eprintln!("decoded {ok}; rejected {rejected:?}");
}
