//! Boots the Lua UI headless (as `pz-client --headless-frames` does), lowers
//! the main menu's draw list and renders it with pz-gpu into a PNG:
//! `main_menu <game dir> <out.png> [frames]`.

use std::cell::RefCell;
use std::rc::Rc;

use pz_lua_api::LuaManager;
use pz_ui::UIManager;

fn main() {
    let mut args = std::env::args().skip(1);
    let game = args.next().expect("game directory");
    let out = args.next().expect("output png");
    let frames: u32 = args.next().and_then(|n| n.parse().ok()).unwrap_or(5);
    let core = pz_lua_api::bind_core::shared_core();
    let (w, h) = {
        let c = core.borrow();
        (c.get_screen_width().max(1) as u32, c.get_screen_height().max(1) as u32)
    };
    let fs = Rc::new(RefCell::new(pz_fs::GameFs::new(&game)));
    pz_lua_api::bind_render::textures().borrow_mut().load_ui_packs(&fs.borrow());
    let mut mgr = LuaManager::init(fs).expect("init");
    mgr.thread.log = Box::new(|_| {});
    mgr.thread.out = Box::new(|_| {});
    UIManager::init();
    mgr.load_all_client();
    mgr.trigger_event("OnGameBoot", &[]);
    mgr.trigger_event("OnMainMenuEnter", &[]);
    for _ in 0..frames {
        mgr.trigger_event("OnFETick", &[0.0.into()]);
        UIManager::update(&mut mgr.thread);
        UIManager::render(&mut mgr.thread);
    }
    let mut dl = pz_render::DrawList::default();
    pz_ui::with_ui(|m| m.sprite.lower_into(&mut dl));
    eprintln!("{w}x{h}, {} draw commands", dl.cmds.len());
    let mut gpu = pz_gpu::Gpu::new_headless(w, h).expect("gpu");
    let tm = pz_lua_api::bind_render::textures();
    gpu.render_frame(&[(pz_gpu::FrameParams { clear: Some([0.0, 0.0, 0.0, 1.0]), ..pz_gpu::FrameParams::ui(w, h) }, &dl)], &mut tm.borrow_mut());
    let px = gpu.read_pixels();
    let f = std::fs::File::create(&out).unwrap();
    let mut e = png::Encoder::new(std::io::BufWriter::new(f), w, h);
    e.set_color(png::ColorType::Rgba);
    e.set_depth(png::BitDepth::Eight);
    e.write_header().unwrap().write_image_data(&px).unwrap();
}
