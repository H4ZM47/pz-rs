//! `zombie.GameWindow`: window, event loop and the per-frame
//! logic/render sequence, rendering through `pz-gpu`.

use crate::main_screen::MainScreenState;
use pz_core::core::Core;
use pz_lua_api::LuaManager;
use pz_ui::UIManager;
use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};
use winit::application::ApplicationHandler;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::window::{Window, WindowId};

pub struct Launch {
    pub game_dir: PathBuf,
    /// Cache directory (the original's `~/Zomboid`).
    pub cache_dir: PathBuf,
}

struct App {
    launch: Launch,
    window: Option<Arc<Window>>,
    gpu: Option<pz_gpu::Gpu>,
    lua: Option<LuaManager>,
    core: Rc<RefCell<Core>>,
    state: MainScreenState,
    last_frame: Instant,
    /// `Controllers` backend (gilrs); `None` when unavailable.
    gamepads: Option<pz_input::gamepads::Gamepads>,
    /// `UI3DModel` avatars (installed at boot).
    ui_models: Option<Rc<RefCell<crate::ui_models::UiModels>>>,
}

/// Boot (`GameWindow.init`) and run the main menu until the window closes.
pub fn run(launch: Launch) -> Result<(), String> {
    let event_loop = EventLoop::new().map_err(|e| e.to_string())?;
    event_loop.set_control_flow(ControlFlow::Poll);
    let core = pz_lua_api::bind_core::shared_core();
    {
        let mut c = core.borrow_mut();
        c.set_cache_dir(&launch.cache_dir);
        let _ = c.load_options();
    }
    let mut app = App {
        launch,
        window: None,
        gpu: None,
        lua: None,
        core,
        state: MainScreenState::default(),
        last_frame: Instant::now(),
        gamepads: None,
        ui_models: None,
    };
    event_loop.run_app(&mut app).map_err(|e| e.to_string())
}

impl App {
    fn boot(&mut self) -> Result<(), String> {
        let (w, h) = self.screen_size();
        let fs = Rc::new(RefCell::new(pz_fs::GameFs::new(&self.launch.game_dir)));
        fs.borrow_mut().set_cache_dir(&self.launch.cache_dir);
        pz_lua_api::bind_render::textures().borrow_mut().load_ui_packs(&fs.borrow());
        crate::boot::load_scripts(&self.launch.game_dir, &mut fs.borrow_mut())?;
        self.ui_models = Some(crate::ui_models::install(&self.launch.game_dir, fs.clone()));
        // Controllers.create (GameWindow.init, before LuaManager.init).
        pz_lua_api::bind_joypad::set_cache_dir(&self.launch.cache_dir);
        self.gamepads = pz_input::with_controllers(|c| pz_input::gamepads::Gamepads::new(&self.launch.game_dir, &self.launch.cache_dir, c));
        let mut lua = LuaManager::init(fs).map_err(|e| format!("LuaManager.init: {e}"))?;
        // GameWindow.init: Translator.loadFiles and TextManager.Init.
        pz_lua_api::bind_text::boot(&lua.fs.borrow(), &self.core.borrow()).map_err(|e| format!("text init: {e}"))?;
        pz_lua_api::hud::init_ui(&mut lua.thread);
        lua.load_all_client();
        lua.trigger_event("OnGameBoot", &[]);
        self.state.enter(&mut lua, w, h);
        self.lua = Some(lua);
        Ok(())
    }

    fn screen_size(&self) -> (i32, i32) {
        let c = self.core.borrow();
        (c.get_screen_width().max(1), c.get_screen_height().max(1))
    }

    fn frame(&mut self) {
        let Some(lua) = self.lua.as_mut() else { return };
        let now = Instant::now();
        let dt = now.duration_since(self.last_frame).as_secs_f32();
        self.last_frame = now;
        // GameTime multiplier at 30 FPS reference, clamped like the original.
        let multiplier = (dt * 30.0).clamp(0.0, 5.0);
        let (w, h) = {
            let c = self.core.borrow();
            (c.get_screen_width().max(1), c.get_screen_height().max(1))
        };
        // GameWindow.logic: Controllers poll, Mouse/GameKeyboard/GameInput
        // updates, UIManager.update (with UITransition.UpdateAll), the
        // blocked-button and joypad pass after it, then states.update.
        if let Some(gp) = self.gamepads.as_mut() {
            pz_input::with_controllers(|c| gp.poll(c));
        }
        pz_lua_api::bind_input::update(lua, dt);
        pz_lua_api::bind_misc::ui_transition_update_all();
        UIManager::update(&mut lua.thread);
        pz_lua_api::bind_input::after_ui_update(lua);
        if let Some(w) = self.window.as_ref() {
            apply_window_requests(w, pz_lua_api::bind_input::take_window_requests());
        }
        self.state.update(lua, multiplier);
        pz_ui::with_ui(|m| m.mouse.end_frame());
        // The avatars' texture jobs (ModelInstanceTextureCreator) before
        // the UI draws them.
        if let Some(u) = self.ui_models.as_ref() {
            // GameTime.getTimeDelta(): the 30 FPS multiplier over 30.
            u.borrow_mut().set_time_delta(multiplier / 30.0);
            if let Some(gpu) = self.gpu.as_mut() {
                u.borrow_mut().prepare(gpu);
            }
        }
        // GameWindow.render: states.render.
        let mut dl = pz_render::DrawList::default();
        self.state.render(lua, &mut dl, w, h, multiplier);
        if let Some(gpu) = self.gpu.as_mut() {
            let tm = pz_lua_api::bind_render::textures();
            let params = pz_gpu::FrameParams { clear: Some([0.0, 0.0, 0.0, 1.0]), ..pz_gpu::FrameParams::ui(w as u32, h as u32) };
            gpu.render_frame(&[(params, &dl)], &mut tm.borrow_mut());
        }
        // Core.DoFrameReady -> Core.updateKeyboard: focused text entry.
        pz_lua_api::bind_uitextbox2::update_keyboard(&mut lua.thread);
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, el: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        let (w, h) = self.screen_size();
        let attrs = Window::default_attributes()
            .with_title("Project Zomboid")
            .with_inner_size(winit::dpi::PhysicalSize::new(w as u32, h as u32));
        let window = match el.create_window(attrs) {
            Ok(w) => Arc::new(w),
            Err(e) => {
                eprintln!("window: {e}");
                el.exit();
                return;
            }
        };
        let size = window.inner_size();
        self.core.borrow_mut().set_screen_size(size.width as i32, size.height as i32);
        match pz_gpu::Gpu::new(window.clone(), size.width, size.height) {
            Ok(mut g) => {
                g.set_vsync(self.core.borrow().get_option_v_sync());
                self.gpu = Some(g);
            }
            Err(e) => eprintln!("graphics: {e}"),
        }
        self.window = Some(window);
        if let Err(e) = self.boot() {
            eprintln!("{e}");
            el.exit();
        }
    }

    fn window_event(&mut self, el: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        // GLFW's key, char, cursor, button and scroll callbacks.
        pz_input::with_input(|i| pz_input::winit::handle(i, &event));
        match event {
            WindowEvent::CloseRequested => el.exit(),
            WindowEvent::Resized(size) => {
                if let Some(g) = self.gpu.as_mut() {
                    g.resize(size.width.max(1), size.height.max(1));
                }
                self.core.borrow_mut().set_screen_size(size.width.max(1) as i32, size.height.max(1) as i32);
            }
            WindowEvent::RedrawRequested => {
                self.frame();
                if let Some(w) = &self.window {
                    w.request_redraw();
                }
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, _el: &ActiveEventLoop) {
        if let Some(w) = &self.window {
            w.request_redraw();
        }
        // PerformanceSettings.lockFPS pacing.
        let fps = 60.0;
        let target = Duration::from_secs_f64(1.0 / fps);
        let elapsed = self.last_frame.elapsed();
        if elapsed < target {
            std::thread::sleep(target - elapsed);
        }
    }
}

/// `Mouse.setXY`, `Mouse.setCursorVisible` and grabbing, as GLFW does
/// them on the window.
fn apply_window_requests(w: &Window, requests: Vec<pz_input::WindowRequest>) {
    use pz_input::WindowRequest::*;
    for r in requests {
        match r {
            SetCursorPosition(x, y) => {
                let _ = w.set_cursor_position(winit::dpi::PhysicalPosition::new(x, y));
            }
            SetGrabbed(g) => {
                let mode = if g { winit::window::CursorGrabMode::Confined } else { winit::window::CursorGrabMode::None };
                if w.set_cursor_grab(mode).is_err() && g {
                    let _ = w.set_cursor_grab(winit::window::CursorGrabMode::Locked);
                }
            }
            SetCursorVisible(v) => w.set_cursor_visible(v),
        }
    }
}

/// Boot headless, run `frames` main-menu frames and return the last frame
/// as RGBA pixels (width, height, data).
pub fn render_headless(launch: Launch, frames: u32) -> Result<(u32, u32, Vec<u8>), String> {
    let core = pz_lua_api::bind_core::shared_core();
    {
        let mut c = core.borrow_mut();
        c.set_cache_dir(&launch.cache_dir);
        let _ = c.load_options();
    }
    let mut app = App {
        launch,
        window: None,
        gpu: None,
        lua: None,
        core,
        state: MainScreenState::default(),
        last_frame: Instant::now(),
        gamepads: None,
        ui_models: None,
    };
    let (w, h) = app.screen_size();
    app.gpu = Some(pz_gpu::Gpu::new_headless(w as u32, h as u32).map_err(|e| e.to_string())?);
    app.boot()?;
    // Development harness: `PZ_HEADLESS_LUA=<file>` runs a Lua file after
    // boot (e.g. to open a screen before the frames).
    if let Some(path) = std::env::var_os("PZ_HEADLESS_LUA") {
        let src = std::fs::read_to_string(&path).map_err(|e| format!("PZ_HEADLESS_LUA: {e}"))?;
        let lua = app.lua.as_mut().ok_or("no Lua")?;
        let f = pz_kahlua::stdlib::load_string(&src, Some("headless"), lua.thread.globals().clone())
            .map_err(|e| format!("PZ_HEADLESS_LUA: {e:?}"))?;
        if let Err((e, tr)) = lua.thread.pcall(&f, &[]) {
            eprintln!("PZ_HEADLESS_LUA: {e:?}\n{tr}");
        }
    }
    for _ in 0..frames {
        app.last_frame = Instant::now() - Duration::from_millis(33);
        app.frame();
    }
    let px = app.gpu.as_ref().unwrap().read_pixels();
    Ok((w as u32, h as u32, px))
}
