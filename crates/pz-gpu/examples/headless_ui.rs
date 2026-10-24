//! Renders a few shared textures from an installed game headless and saves
//! a PNG: `headless_ui <game dir containing media/> <out.png> [names...]`.

use pz_render::{DrawList, Rgba, TextureManager};

fn main() {
    let mut args = std::env::args().skip(1);
    let game = args.next().expect("game directory");
    let out = args.next().expect("output png");
    let names: Vec<String> = args.collect();
    let fs = pz_fs::GameFs::new(game);
    let mut tm = TextureManager::new();
    tm.load_ui_packs(&fs);
    let (w, h) = (800u32, 600u32);
    let mut dl = DrawList::default();
    dl.render_rect(0, 0, w as i32, h as i32, Rgba(0.2, 0.2, 0.25, 1.0));
    let (mut x, mut y, mut row) = (10.0f32, 10.0f32, 0.0f32);
    for n in &names {
        let Some(t) = tm.shared_texture(&fs, n) else {
            eprintln!("{n}: not found");
            continue;
        };
        if x + t.width as f32 > w as f32 {
            x = 10.0;
            y += row + 10.0;
            row = 0.0;
        }
        dl.render_texture(&t, x, y, t.width as f32, t.height as f32, Rgba(1.0, 1.0, 1.0, 1.0));
        x += t.width_orig() as f32 + 10.0;
        row = row.max(t.height_orig() as f32);
    }
    let mut gpu = pz_gpu::Gpu::new_headless(w, h).expect("gpu");
    gpu.render_frame(&[(pz_gpu::FrameParams { clear: Some([0.0, 0.0, 0.0, 1.0]), ..pz_gpu::FrameParams::ui(w, h) }, &dl)], &mut tm);
    let px = gpu.read_pixels();
    let f = std::fs::File::create(&out).unwrap();
    let mut e = png::Encoder::new(std::io::BufWriter::new(f), w, h);
    e.set_color(png::ColorType::Rgba);
    e.set_depth(png::BitDepth::Eight);
    e.write_header().unwrap().write_image_data(&px).unwrap();
}
