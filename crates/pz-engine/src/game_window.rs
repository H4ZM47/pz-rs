//! `zombie.GameWindow`: window, event loop and the per-frame
//! logic/render sequence, rendering through `pz-gpu`.

use crate::game_loading::LoadingQueueState;
use crate::game_state::{state, GameStateMachine, StateCx};
use crate::iso_world::{IsoWorld, NewGameSetup};
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
    pub start: Start,
}

/// Where the state machine starts.
#[derive(Clone, Debug, Default)]
pub enum Start {
    /// The title screen (`MainScreenState`).
    #[default]
    MainMenu,
    /// Development entry: what the main menu's Lua does before it calls
    /// `forceChangeState(LoadingQueueState.new())` (`setGameMode`,
    /// `createWorld`, `initWorld`'s player setup), without the menu.
    Game(DevGame),
}

/// A game the development entry starts.
#[derive(Clone, Debug)]
pub struct DevGame {
    /// `Core.gameMode` (`getWorld():setGameMode`).
    pub game_mode: String,
    /// The save folder's name (`Core.gameSaveWorld`).
    pub world: String,
    /// `Some` for a new game: `createWorld(world)` and the player setup;
    /// `None` loads the existing save.
    pub new_game: Option<NewGameSetup>,
    /// `Core.gameMap` for a new game (`getWorld():setMap`).
    pub map: String,
}

impl Start {
    /// The player setup `IsoWorld.init` uses for a new game.
    pub fn new_game_setup(&self) -> Option<NewGameSetup> {
        match self {
            Start::Game(g) => g.new_game.clone(),
            Start::MainMenu => None,
        }
    }

    /// Whether `GameLoadingState` takes the "click to start" by itself
    /// (the development entry runs without anyone to click).
    pub fn click_to_skip(&self) -> bool {
        matches!(self, Start::Game(_))
    }
}

/// `LuaManager.GlobalObject.createWorld(worldName)`: the save folder (unless
/// saving is off) with its subdirectories, and `Core.gameSaveWorld`.
pub fn create_world(cache_dir: &std::path::Path, world_name: &str) -> Result<(), String> {
    let name = if world_name.is_empty() {
        "blah"
    } else {
        world_name
    };
    let name = pz_lua_api::savefile::sanitize_world_name(name);
    let core = pz_lua_api::bind_core::shared_core();
    let (mode, no_save) = {
        let c = core.borrow();
        (c.game_mode.clone(), c.is_no_save())
    };
    let dir = cache_dir.join("Saves").join(mode).join(&name);
    if !no_save {
        std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        pz_lua_api::world_save::ensure_subdirectories_exist(&dir)
            .map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    core.borrow_mut().game_save_world = name;
    Ok(())
}

struct App {
    launch: Launch,
    window: Option<Arc<Window>>,
    gpu: Option<pz_gpu::Gpu>,
    lua: Option<LuaManager>,
    core: Rc<RefCell<Core>>,
    /// `GameWindow.states`.
    states: GameStateMachine,
    /// `IsoWorld.instance` once a game loads.
    world: Option<IsoWorld>,
    error: Option<crate::failure::EngineError>,
    retired_models: Vec<crate::world_models::WorldModels>,
    last_frame: Instant,
    /// `Controllers` backend (gilrs); `None` when unavailable.
    gamepads: Option<pz_input::gamepads::Gamepads>,
    /// The audio device stream (windowed runs only).
    audio: Option<pz_audio::output::Output>,
    /// `UI3DModel` avatars (installed at boot).
    ui_models: Option<Rc<RefCell<crate::ui_models::UiModels>>>,
}

/// Boot (`GameWindow.init`) and run the main menu until the window closes.
pub fn run(launch: Launch) -> crate::failure::EngineResult<()> {
    let _retirement_boundary = HostRetirementBoundary;
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
        states: GameStateMachine::default(),
        world: None,
        error: None,
        retired_models: Vec::new(),
        last_frame: Instant::now(),
        gamepads: None,
        audio: None,
        ui_models: None,
    };
    event_loop.run_app(&mut app).map_err(|e| e.to_string())?;
    app.error.take().map_or(Ok(()), Err)
}

impl App {
    /// A native window close saves an active session through the same
    /// quit path as the menu, then stops its owners before the GPU goes.
    fn close(&mut self) {
        let collection_deferral = pz_kahlua::gc::defer_collection();
        let current = self.states.current.take();
        let in_game = current
            .as_ref()
            .is_some_and(|s| s.borrow().class_name() == "zombie.gameStates.IngameState");
        let (screen_w, screen_h) = self.screen_size();
        let Some(lua) = self.lua.as_mut() else {
            crate::floor_world::retire_installed_floor_world();
            drop(collection_deferral);
            drain_retired_host_owners();
            return;
        };
        if in_game {
            if let Some(world) = self.world.as_mut() {
                if let Err(error) = world.save_on_quit(lua) {
                    crate::failure::record_first(&mut self.error, (format!("PlayerDB save host failure: {error}")).into());
                }
            }
        }
        if let Some(current) = current {
            current.borrow_mut().exit(&mut StateCx {
                lua,
                launch: &self.launch,
                screen_w,
                screen_h,
                multiplier: 0.0,
                time_delta: 0.0,
                world: &mut self.world,
                shutdown: true,
                error: &mut self.error,
                retired_models: &mut self.retired_models,
            });
        }
        // A partially loaded world may have no current IngameState yet.
        if let Some(world) = self.world.as_mut() {
            if let Some(models) = world.models.take() {
                self.retired_models.push(models);
            }
            world.stop();
        }
        self.world = None;
        crate::floor_world::retire_installed_floor_world();
        // A close during loading does not pass through IngameState.KillCell.
        // Retire the in-process host's current cell after its owners stop.
        pz_lua_api::bind_world::set_cell(None);
        for mut models in self.retired_models.drain(..) {
            models.clear();
            if let Some(gpu) = self.gpu.as_mut() {
                models.prepare(gpu);
                gpu.clear_character_mesh_cache();
            }
        }
        // Error-only and partially booted sessions may never reach a frame
        // tail. Retire their pins after world teardown has ended every guard.
        drop(collection_deferral);
        drain_retired_host_owners();
    }

    fn boot(&mut self) -> crate::failure::EngineResult<()> {
        let _collection_deferral = pz_kahlua::gc::defer_collection();
        let fs = Rc::new(RefCell::new(pz_fs::GameFs::new(&self.launch.game_dir)));
        fs.borrow_mut().set_cache_dir(&self.launch.cache_dir);
        pz_lua_api::bind_render::textures()
            .borrow_mut()
            .load_ui_packs(&fs.borrow());
        crate::boot::load_scripts(&self.launch.game_dir, &fs)?;
        // SoundManager.init / FMODManager.init (windowed runs; headless
        // runs keep the silent system).
        if self.window.is_some() {
            match crate::sound::install(
                &self.launch.game_dir,
                &self.launch.cache_dir,
                fs.clone(),
                &self.core.borrow(),
            ) {
                Ok(o) => self.audio = Some(o),
                Err(e) => eprintln!("sound: {e}"),
            }
        }
        self.ui_models = Some(crate::ui_models::install(&self.launch.game_dir, fs.clone()));
        // Controllers.create (GameWindow.init, before LuaManager.init).
        pz_lua_api::bind_joypad::set_cache_dir(&self.launch.cache_dir);
        self.gamepads = pz_input::with_controllers(|c| {
            pz_input::gamepads::Gamepads::new(&self.launch.game_dir, &self.launch.cache_dir, c)
        });
        let mut lua = LuaManager::init(fs).map_err(|error| crate::failure::EngineError::lua("LuaManager.init", error))?;
        // GameWindow.init: Translator.loadFiles and TextManager.Init.
        pz_lua_api::bind_text::boot(&lua.fs.borrow(), &self.core.borrow())
            .map_err(|e| format!("text init: {e}"))?;
        pz_lua_api::hud::init_ui(&mut lua.thread);
        lua.load_all_client();
        lua.trigger_event("OnGameBoot", &[]);
        self.lua = Some(lua);
        self.init_states()
    }

    /// The end of `GameWindow.init`: the listed states (entered by the
    /// first `states.update()`), or the development entry's game.
    fn init_states(&mut self) -> crate::failure::EngineResult<()> {
        match self.launch.start.clone() {
            Start::MainMenu => {
                // TODO(port): `TISLogoState` and `TermsOfServiceState` come
                // first (`loopToState = 1` outside debug, the terms screen).
                self.states.states.push(state(MainScreenState::default()));
            }
            Start::Game(g) => {
                // The main menu normally registers the shipped/default
                // and saved key bindings through MainOptions.loadKeys.
                // Development game entry bypasses that screen.
                crate::boot::load_key_bindings(self.lua.as_mut().ok_or("no Lua")?)?;
                {
                    let mut c = self.core.borrow_mut();
                    c.set_game_mode(&g.game_mode);
                    c.game_save_world = g.world.clone();
                    if g.new_game.is_some() {
                        c.game_map = g.map.clone();
                    }
                }
                if g.new_game.is_some() {
                    create_world(&self.launch.cache_dir, &g.world)?;
                }
                // `GameWindow.doRenderEvent(false)`;
                // `forceChangeState(LoadingQueueState.new())`.
                self.states
                    .force_next_state(state(LoadingQueueState::default()));
            }
        }
        Ok(())
    }

    /// The current state's class name (`GameWindow.states.current`).
    pub fn current_state(&self) -> Option<&'static str> {
        self.states.current_name()
    }

    fn screen_size(&self) -> (i32, i32) {
        let c = self.core.borrow();
        (c.get_screen_width().max(1), c.get_screen_height().max(1))
    }

    fn frame(&mut self) {
        if self.error.is_some() {
            drain_retired_host_owners();
            return;
        }
        let Some(lua) = self.lua.as_mut() else { return };
        let collection_deferral = pz_kahlua::gc::defer_collection();
        let now = Instant::now();
        let dt = now.duration_since(self.last_frame).as_secs_f32();
        self.last_frame = now;
        // GameTime multiplier at 30 FPS reference, clamped like the original.
        let multiplier = (dt * 30.0).clamp(0.0, 5.0);
        // FPSTracking.frameStep: GameTime.fpsMultiplier = 60 / fps, at most 5.
        pz_lua_api::bind_climate::shared_game_time()
            .borrow_mut()
            .fps_multiplier = (dt * 60.0).min(5.0);
        // GameWindow.logic pumps the single-player server then client after
        // frame timing, before input/UI and pause capture, even while paused.
        pz_lua_api::global_objects::update_server(&mut lua.thread);
        pz_lua_api::global_objects::update_client(&mut lua.thread);
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
        if let Err(error) = UIManager::update(&mut lua.thread) {
            crate::failure::record_first(&mut self.error, crate::failure::EngineError::lua("UIManager.update", error));
            drop(collection_deferral);
            drain_retired_host_owners();
            return;
        }
        pz_lua_api::bind_input::after_ui_update(lua);
        if let Some(w) = self.window.as_ref() {
            apply_window_requests(w, pz_lua_api::bind_input::take_window_requests());
        }
        // MapCollisionData.instance.updateGameState().
        // GameWindow.logic snapshots this after UI/input callbacks. Paused
        // frames still update UI/audio/rendering, but never the game states.
        let game_paused = pz_lua_api::bind_climate::is_game_paused();
        if let Some(world) = self.world.as_mut() {
            let no_save = self.core.borrow().is_no_save();
            world.update_game_state(no_save, game_paused);
        }
        if game_paused {
            // TODO(port): IsoCamera.updateAll(). The paused event carries
            // the original constant zero and runs only in the ingame state.
            if self.states.current_name() == Some("zombie.gameStates.IngameState") {
                lua.trigger_event("OnTickEvenPaused", &[pz_kahlua::Value::Num(0.0)]);
            }
        } else {
            let mut cx = StateCx {
                lua: &mut *lua,
                launch: &self.launch,
                screen_w: w,
                screen_h: h,
                multiplier,
                time_delta: dt,
                world: &mut self.world,
                shutdown: false,
                error: &mut self.error,
                retired_models: &mut self.retired_models,
            };
            self.states.update(&mut cx);
        }
        // Direct game entry is a startup request. Once a session returns
        // to the menu, future loads use the choices made by the menu.
        if self.world.is_none()
            && self.states.current_name() == Some("zombie.gameStates.MainScreenState")
        {
            self.launch.start = Start::MainMenu;
        }
        for mut models in self.retired_models.drain(..) {
            models.clear();
            if let Some(gpu) = self.gpu.as_mut() {
                models.prepare(gpu);
                gpu.clear_character_mesh_cache();
            }
        }
        if self.error.is_some() {
            drop(collection_deferral);
            drain_retired_host_owners();
            return;
        }
        // SoundManager.instance.Update() (GameWindow.logic, "Sound").
        let toggle_music = pz_lua_api::bind_input::is_key_down_named("Toggle Music")
            && !pz_lua_api::bind_input::was_key_down_named("Toggle Music");
        pz_lua_api::bind_audio::sound_system()
            .borrow_mut()
            .with(|m, ctx| m.update(toggle_music, ctx));
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
        if let (Some(world), Some(gpu)) = (self.world.as_mut(), self.gpu.as_mut()) {
            if let Some(models) = world.models.as_mut() {
                models.prepare(gpu);
            }
        }
        let mut dl = pz_render::DrawList::default();
        {
            let mut cx = StateCx {
                lua: &mut *lua,
                launch: &self.launch,
                screen_w: w,
                screen_h: h,
                multiplier,
                time_delta: dt,
                world: &mut self.world,
                shutdown: false,
                error: &mut self.error,
                retired_models: &mut self.retired_models,
            };
            self.states.render(&mut cx, &mut dl);
        }
        if self.error.is_some() {
            drop(collection_deferral);
            drain_retired_host_owners();
            return;
        }
        if let Some(gpu) = self.gpu.as_mut() {
            let tm = pz_lua_api::bind_render::textures();
            let params = pz_gpu::FrameParams {
                clear: Some([0.0, 0.0, 0.0, 1.0]),
                ..pz_gpu::FrameParams::ui(w as u32, h as u32)
            };
            gpu.render_frame(&[(params, &dl)], &mut tm.borrow_mut());
        }
        // `if (doRenderEvent) onRender()`: OnRenderTick after the frame.
        if pz_lua_api::bind_game_window::render_event_enabled() {
            lua.trigger_event("OnRenderTick", &[]);
        }
        // Core.DoFrameReady -> Core.updateKeyboard: focused text entry.
        pz_lua_api::bind_uitextbox2::update_keyboard(&mut lua.thread);
        // Interpreter call boundaries can occur inside WORLD/sprite scopes.
        // Only this outer boundary permits final host destructors to reenter.
        drop(collection_deferral);
        drain_retired_host_owners();
    }
}

/// Outermost frame/error/close boundary, after all world, sprite and entity
/// guards have ended. Retired payloads can reenter host registration on drop.
fn drain_retired_host_owners() {
    crate::iso_world::drain_retired_world_objects();
    pz_inventory::component_host::drain_retired_components();
    drop(pz_scripts::objects::entity::components::attributes::take_retired_attribute_templates());
    drop(pz_scripts::texture_capture::take_retired_textures());
    drop(pz_render::take_retired_texture_loaders());
    pz_kahlua::gc::drain_retired_table_values();
    pz_lua_api::component_gc::retry_pending();
    for fault in pz_lua_api::component_gc::take_registration_faults() {
        eprintln!("component registration: {fault}");
    }
    for fault in pz_lua_api::bind_world_object::take_native_class_faults() {
        eprintln!("world native class registration: {fault}");
    }
    for fault in pz_lua_api::bind_designation::take_native_class_faults() {
        eprintln!("designation native class registration: {fault}");
    }
    let _ = pz_kahlua::gc::collect_pending();
}

/// Declared before App so unwinding and boot failures release host pins only
/// after the application's fields and all scoped guards have been dropped.
struct HostRetirementBoundary;
impl Drop for HostRetirementBoundary {
    fn drop(&mut self) {
        drain_retired_host_owners();
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
        self.core
            .borrow_mut()
            .set_screen_size(size.width as i32, size.height as i32);
        match pz_gpu::Gpu::new(window.clone(), size.width, size.height) {
            Ok(mut g) => {
                g.set_vsync(self.core.borrow().get_option_v_sync());
                self.gpu = Some(g);
            }
            Err(e) => eprintln!("graphics: {e}"),
        }
        self.window = Some(window);
        if let Err(e) = self.boot() {
            crate::failure::record_first(&mut self.error, e);
            el.exit();
        }
    }

    fn window_event(&mut self, el: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        // GLFW's key, char, cursor, button and scroll callbacks.
        pz_input::with_input(|i| pz_input::winit::handle(i, &event));
        match event {
            WindowEvent::CloseRequested => {
                self.close();
                el.exit();
            }
            WindowEvent::Resized(size) => {
                if let Some(g) = self.gpu.as_mut() {
                    g.resize(size.width.max(1), size.height.max(1));
                }
                self.core
                    .borrow_mut()
                    .set_screen_size(size.width.max(1) as i32, size.height.max(1) as i32);
            }
            WindowEvent::RedrawRequested => {
                self.frame();
                if self.error.is_some() {
                    el.exit();
                    return;
                }
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
                let mode = if g {
                    winit::window::CursorGrabMode::Confined
                } else {
                    winit::window::CursorGrabMode::None
                };
                if w.set_cursor_grab(mode).is_err() && g {
                    let _ = w.set_cursor_grab(winit::window::CursorGrabMode::Locked);
                }
            }
            SetCursorVisible(v) => w.set_cursor_visible(v),
        }
    }
}

/// Boot headless, run `frames` frames and return the last frame as RGBA
/// pixels (width, height, data).
pub fn render_headless(launch: Launch, frames: u32) -> crate::failure::EngineResult<(u32, u32, Vec<u8>)> {
    let (w, h, px) = headless(launch, frames, true)?;
    Ok((w, h, px.unwrap_or_default()))
}

/// Boot headless without a GPU and run `frames` frames (logic and the
/// draw lists, nothing presented); returns the state the machine is in.
pub fn run_headless(launch: Launch, frames: u32) -> crate::failure::EngineResult<Option<&'static str>> {
    let _retirement_boundary = HostRetirementBoundary;
    let mut app = headless_app(launch)?;
    app.boot()?;
    run_headless_lua(&mut app)?;
    run_frames(&mut app, frames);
    if let Some(error) = app.error.take() {
        return Err(error);
    }
    Ok(app.current_state())
}

fn headless_app(launch: Launch) -> crate::failure::EngineResult<App> {
    let core = pz_lua_api::bind_core::shared_core();
    {
        let mut c = core.borrow_mut();
        c.set_cache_dir(&launch.cache_dir);
        let _ = c.load_options();
    }
    Ok(App {
        launch,
        window: None,
        gpu: None,
        lua: None,
        core,
        states: GameStateMachine::default(),
        world: None,
        error: None,
        retired_models: Vec::new(),
        last_frame: Instant::now(),
        gamepads: None,
        audio: None,
        ui_models: None,
    })
}

/// Development harness: `PZ_HEADLESS_LUA=<file>` runs a Lua file after
/// boot (e.g. to open a screen before the frames).
fn run_headless_lua(app: &mut App) -> crate::failure::EngineResult<()> {
    let _collection_deferral = pz_kahlua::gc::defer_collection();
    if let Some(path) = std::env::var_os("PZ_HEADLESS_LUA") {
        let src = std::fs::read_to_string(&path).map_err(|e| format!("PZ_HEADLESS_LUA: {e}"))?;
        let lua = app.lua.as_mut().ok_or("no Lua")?;
        run_headless_source(lua, &src)?;
    }
    Ok(())
}

fn run_headless_source(lua: &mut LuaManager, source: &str) -> crate::failure::EngineResult<()> {
    let f = pz_kahlua::stdlib::load_string(source, Some("headless"), lua.thread.globals().clone())
        .map_err(|error| crate::failure::EngineError::lua("PZ_HEADLESS_LUA compile", error))?;
    lua.thread.pcall_ex(&f, &[])
        .map_err(|(_, _, error)| crate::failure::EngineError::lua("PZ_HEADLESS_LUA run", error))?;
    Ok(())
}

fn run_frames(app: &mut App, frames: u32) {
    for _ in 0..frames {
        if app.error.is_some() {
            break;
        }
        app.last_frame = Instant::now() - Duration::from_millis(33);
        app.frame();
    }
}

fn headless(launch: Launch, frames: u32, gpu: bool) -> crate::failure::EngineResult<(u32, u32, Option<Vec<u8>>)> {
    let _retirement_boundary = HostRetirementBoundary;
    let mut app = headless_app(launch)?;
    let (w, h) = app.screen_size();
    if gpu {
        app.gpu = Some(pz_gpu::Gpu::new_headless(w as u32, h as u32).map_err(|e| e.to_string())?);
    }
    app.boot()?;
    run_headless_lua(&mut app)?;
    run_frames(&mut app, frames);
    if let Some(error) = app.error.take() {
        return Err(error);
    }
    let px = app.gpu.as_ref().map(|g| g.read_pixels());
    Ok((w as u32, h as u32, px))
}

#[cfg(test)]
mod retirement_tests;
#[cfg(test)]
mod session_tests;

#[cfg(test)]
mod error_boundary_tests;
